# Brainstorm: Telegram Agent Prefix Attribution

**Date:** 2026-03-15
**Status:** Decided

## What We're Building

Fix missing `[agent_name]` prefix on Telegram messages sent via TUI/CLI, and ensure reply routing works for all outbound paths.

## Problem Statement

- From TUI, asking any agent (mika or mika-dev) to send a message on Telegram delivers it **without** the `[agent_name]` prefix.
- Messages originating from the server path (e.g., Telegram → gateway → server → agent → gateway → Telegram) correctly show the prefix.
- Without the prefix, `outbound_messages` isn't populated, breaking reply routing entirely for CLI-originated sends.

## Root Cause Analysis

The gateway's `/send` handler is the **single point** where `[agent_name]` prefixing happens (KISS, SRP — correct design). It only prepends the prefix when `agent_name` is present in the POST payload.

The `GatewayMessageSender` carries `agent_name: Option<String>` and includes it in every `/send` request. The bug is that the CLI's `make_message_sender` in `crates/mika-cli/src/init.rs:183` passes `None`:

```rust
None, // CLI doesn't send to Telegram gateway   ← FALSE, IT DOES
```

The comment is factually wrong — the CLI does send to the gateway when `MIKA_ROUTING_URL` is configured. This misleading comment likely caused repeated misdiagnosis.

### Path-by-path analysis

| Path | `agent_name` set? | Prefix? | Reply routing? |
|---|---|---|---|
| Telegram → server → agent → gateway | Yes (handlers.rs:218) | YES | YES |
| TUI → agent → `send_message` → gateway | **No** (init.rs:183) | **NO** | **NO** |
| Delegate → `send_message` → gateway | Yes (delegate_task.rs:204) | YES | YES |
| Delegate text response (no explicit send) | Was missing, now fixed | YES | YES |

## Why This Approach

The architecture is sound — single prefixing point in the gateway, `agent_name` carried in the sender, `outbound_messages` for reply routing. No structural change needed.

### Fix 1: CLI sender — pass agent_name

Add `agent_name` parameter to `make_message_sender` and pass the active agent's name. This is the only missing piece for TUI → Telegram to work correctly.

### Fix 2: Delegate text response auto-send (already applied)

When `delegate_task` gets `Ok(Some(text))` back from `run_team_agent`, send it via the delegate's sender before returning to the orchestrator. This ensures correct `[agent_name]` attribution for delegate responses that don't explicitly call `send_message`.

**Trade-off accepted:** This may cause double-sends if the orchestrator also relays the delegate's response. We accept this for now — correct attribution outweighs the cosmetic duplicate.

## Key Decisions

1. **CLI sender must carry agent_name** — pass it through `make_message_sender`
2. **Keep delegate auto-send** — accept potential double-send for correct attribution
3. **No architectural changes needed** — the gateway prefixing design is correct
4. **Fix the misleading comment** in init.rs that says "CLI doesn't send to Telegram gateway"

## Open Questions

None — root cause identified, fix is minimal.
