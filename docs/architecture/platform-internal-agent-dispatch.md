# Platform-Internal Agent Dispatch

**Origin:** [mika#935](https://github.com/senara-solutions/mika/issues/935)

## Category

`mika ask --agent <peer>` invocations between well-known platform agents (mika-arch, mika-dev, mika-qa) are **intra-platform dispatch** — a category distinct from operator-shell Bash. These calls are platform-prescribed peer communication, not arbitrary user commands.

## Why It Bypasses the Bash Classifier

The permission-policy LLM classifier (Haiku-tier) evaluates raw command strings. It cannot reliably distinguish `mika ask --agent mika-arch "<2KB brief>"` from arbitrary Bash — the trailing message payload defeats literal pattern matching. This was empirically proven across five canary rounds (mika#935 diagnostic).

The structural pre-classifier recognizes the dispatch shape deterministically (no LLM) and returns `{"action": "allow"}` before the classifier is consulted.

## Two-Layer Architecture

1. **claude-pilot tier1 fast-path** (`claude-pilot-py/src/claude_pilot/tier1.py`): `is_intra_platform_agent_dispatch()` recognizes the command and returns `True`. The relay subprocess is never spawned. This is the primary layer.

2. **mika-relay structural pre-classifier** (`crates/mika-agent/src/server/permission_pre_classifier.rs`): If a caller bypasses tier1 (e.g., a future skill handler that shells out directly), the relay's pre-classifier catches it at the Rust layer before LLM invocation. Defense-in-depth.

## Agent Registry

The platform peer list lives in `crates/mika-agent/src/well_known_agents.rs`:

```rust
pub const INTRA_PLATFORM_DISPATCH_PEERS: &[&str] = &["mika-arch", "mika-dev", "mika-qa"];
```

This is the sole source of truth for which agents the pre-classifier allows. Adding a new platform agent requires updating this list and the corresponding `INTRA_PLATFORM_AGENTS` constant in `claude-pilot-py/src/claude_pilot/tier1.py`.

## How to Add a New Platform Agent

1. Add the agent name to `INTRA_PLATFORM_DISPATCH_PEERS` in `crates/mika-agent/src/well_known_agents.rs`.
2. Add the agent name to `INTRA_PLATFORM_AGENTS` in `claude-pilot-py/src/claude_pilot/tier1.py`.
3. Deploy both repos (`make deploy` for mika, `uv tool install --force --editable ./claude-pilot-py` for claude-pilot).

No prompt edits. No LLM re-tuning. Two-line change.

## Safety Invariants

- **TIER 3 patterns always deny.** If the command contains `rm -rf`, `git push --force`, or any other TIER 3 pattern, the pre-classifier falls through to the existing classifier (which also denies).
- **Unknown peers fall through.** Only peers in `INTRA_PLATFORM_DISPATCH_PEERS` are structurally allowed. All others go to the LLM classifier.
- **Only Bash tool calls.** Non-Bash tool permissions (Read, Write, etc.) are not handled by this pre-classifier.
- **Only mika-relay.** The pre-classifier gates on `agent_id == "mika-relay"` — it has no effect on other agents.

## Cross-Repo Reference

- claude-pilot-py typed dispatch surface: `claude-pilot-py/docs/typed-dispatch-surface.md`
- Permission-policy prompt (defense-in-depth third layer): `skills/bundled/permission-policy/system_prompt.md`
