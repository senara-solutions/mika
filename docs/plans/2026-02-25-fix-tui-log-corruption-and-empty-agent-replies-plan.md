---
title: "Fix TUI log corruption and empty agent replies"
type: fix
status: active
date: 2026-02-25
---

# Fix TUI Log Corruption and Empty Agent Replies

## Overview

Two bugs make the TUI unusable: (1) tracing log output written to stderr corrupts the ratatui alternate screen, and (2) after tool-only turns Claude returns no text, leaving the user with a cryptic "Agent processed your request." instead of a natural-language acknowledgment.

## Problem Statement

### Bug 1: Logs corrupt TUI display

`init_pretty` in `crates/mika-common/src/logging.rs:33` registers a `.pretty()` tracing layer that writes to `std::io::stderr`. Ratatui's `EnterAlternateScreen` (at `crates/mika-cli/src/commands/chat.rs:186`) only redirects **stdout** — stderr remains on the physical terminal. Every `warn!`, `info!`, `debug!` call during the agent loop, Claude API retries, tool execution, and scheduler recovery renders as raw text on top of the TUI, causing visual chaos.

The comment at `crates/mika-cli/src/main.rs:51` ("TUI commands write to stderr which ratatui's alternate screen handles") is **incorrect**.

### Bug 2: Agent stops replying after tool use

When Claude responds to tool results with `stop_reason: EndTurn` but produces only `ToolUse` blocks (no `Text` blocks), `response.text()` returns `""`. The agent returns `Ok(None)` and the TUI shows the generic system message "Agent processed your request." (see `crates/mika-cli/src/tui/app.rs:220-227`). The user gets no natural-language confirmation of what happened.

Evidence from `mika.log.2026-02-25` line 6:
```json
{"level":"WARN","message":"agent returned empty text after tool use","step":2,"stop_reason":"EndTurn"}
```

The conversation DB confirms: user said "CET" (timezone), got no assistant response (message id 25 is the last, a user message).

## Proposed Solution

### Fix 1: Suppress stderr layer in TUI mode

Add a `suppress_stderr: bool` parameter to `init_pretty`. When `true`, only register the file appender layer (no stderr output). The caller in `main.rs` determines TUI mode from `cli.command`.

**Files:**
- `crates/mika-common/src/logging.rs` — Add `suppress_stderr` parameter
- `crates/mika-cli/src/main.rs` — Compute `is_tui` flag, pass to `init_pretty`, fix incorrect comment

### Fix 2: Re-prompt Claude after tool-only turns

When `tool_use_occurred` is true and the final response has empty text, instead of returning `Ok(None)`, inject a follow-up turn asking Claude to briefly acknowledge what it did. This produces a natural response like "Got it, I've saved your timezone as CET."

**Files:**
- `crates/mika-agent/src/agent.rs` — Add follow-up injection logic in `run_agent_inner`

## Technical Approach

### Fix 1: `init_pretty` stderr suppression

```rust
// crates/mika-common/src/logging.rs
pub fn init_pretty(
    default_level: &str,
    log_dir: Option<&Path>,
    suppress_stderr: bool,
) -> Option<WorkerGuard> {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(default_level));

    match (log_dir, suppress_stderr) {
        (Some(dir), false) => {
            // Both stderr (pretty) + file (JSON) — non-TUI commands
            // ... existing dual-layer setup ...
        }
        (Some(dir), true) => {
            // File only — TUI mode, no stderr corruption
            let _ = std::fs::create_dir_all(dir);
            let file_appender = tracing_appender::rolling::daily(dir, "mika.log");
            let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);
            tracing_subscriber::registry()
                .with(filter)
                .with(fmt::layer().json().flatten_event(true).with_writer(non_blocking))
                .init();
            Some(guard)
        }
        (None, false) => {
            // Stderr only — no log dir available, non-TUI
            // ... existing pretty-only setup ...
        }
        (None, true) => {
            // TUI mode but no log dir — drop all events silently.
            // This is acceptable: if home dir is missing, init_for_agent
            // will fail before TUI starts.
            tracing_subscriber::registry().with(filter).init();
            None
        }
    }
}
```

Caller update in `main.rs`:

```rust
let is_tui = matches!(cli.command, None | Some(Commands::Chat));
// Initialize tracing: suppress stderr in TUI mode to avoid corrupting ratatui.
let _log_guard = mika_common::logging::init_pretty(&log_level, log_dir.as_deref(), is_tui);
```

**Other callers of `init_pretty`**: Check if any exist outside `mika-cli`. The server uses `init()` (JSON to stdout), so it is unaffected. Any other callers pass `false`.

### Fix 2: Follow-up injection after empty tool response

In `run_agent_inner`, when `tool_use_occurred && text.is_empty()` and no follow-up has been attempted yet:

1. Push Claude's response as an assistant message (preserves alternating roles)
2. Push a synthetic user message asking for a summary
3. Continue the loop for one more iteration

```rust
// crates/mika-agent/src/agent.rs — in run_agent_inner
let mut tool_use_occurred = false;
let mut follow_up_attempted = false;

for step in 0..MAX_TOOL_STEPS {
    let response = claude.send_message(&request).await?;

    match response.stop_reason {
        StopReason::EndTurn | StopReason::MaxTokens => {
            let text = response.text();
            if !text.is_empty() {
                db.save_message("assistant", &text, channel_type).await?;
                info!(step, stop_reason = ?response.stop_reason, "agent done");
                return Ok(Some(text));
            }

            // Tool-only turn with no text: re-prompt once for acknowledgment
            if tool_use_occurred && !follow_up_attempted {
                follow_up_attempted = true;
                debug!(step, "injecting follow-up after empty tool response");

                // Push the assistant's (empty-text) response to maintain alternating roles
                request.messages.push(Message {
                    role: "assistant".to_string(),
                    content: MessageContent::Blocks(response.content),
                });
                // Ask Claude to acknowledge what it did
                request.messages.push(Message {
                    role: "user".to_string(),
                    content: MessageContent::Text(
                        "[Briefly confirm what you just did.]".to_string()
                    ),
                });
                continue; // Re-enter loop for one more API call
            }

            if tool_use_occurred {
                warn!(step, stop_reason = ?response.stop_reason,
                    "agent returned empty text after tool use and follow-up");
            }
            info!(step, stop_reason = ?response.stop_reason, "agent done");
            return Ok(None);
        }
        // ... rest unchanged ...
    }
}
```

**Key constraints:**
- `follow_up_attempted` prevents infinite re-injection
- The synthetic user message is NOT saved to DB — it only exists in the in-memory request
- Claude's summary response IS saved (existing code at line 195 handles this)
- The assistant response content is pushed even if it has no text blocks, to maintain the alternating user/assistant role constraint
- The `StopSequence` arm gets the same treatment

**Edge case: `response.content` is empty `[]`**: The Claude API accepts assistant messages with empty content blocks. If it rejects them, fall back to a synthetic `ContentBlock::Text { text: String::new() }`.

## Acceptance Criteria

- [x] No tracing output appears on the terminal while the TUI is active (`mika` and `mika chat`)
- [x] Non-TUI commands (`mika ask`, `mika status`, etc.) still show stderr logs normally
- [x] Log files continue to be written to `~/.mika/agents/{name}/logs/` in all modes
- [x] After tool-only turns (e.g., saving timezone, updating core memory), the agent produces a natural-language acknowledgment instead of "Agent processed your request."
- [x] If the follow-up also returns empty text, the TUI falls back to "Agent processed your request."
- [x] The injected follow-up message is NOT persisted to the conversations table
- [x] `cargo test` passes
- [x] `cargo clippy` is clean
- [x] Fix the incorrect comment at `main.rs:51`

## Files Changed

| File | Change |
|------|--------|
| `crates/mika-common/src/logging.rs` | Add `suppress_stderr` parameter, handle 4 cases |
| `crates/mika-cli/src/main.rs` | Compute `is_tui`, pass to `init_pretty`, fix comment |
| `crates/mika-agent/src/agent.rs` | Add `follow_up_attempted` flag, inject follow-up turn |

## Out of Scope

- Team agent (`run_team_agent_inner`) empty-text handling — different semantics, address separately
- Silent agent — deliberately does not deliver text, no change needed
- Redirecting non-tracing stderr writes (e.g., from dependencies) — partial fix is acceptable
- Runtime log level changes — existing limitation, not introduced by this fix

## References

- Prior fix: `docs/solutions/ui-bugs/empty-response-and-log-directory-fixes.md` — the `Option<String>` return type and log directory fix
- Prior plan: `docs/plans/2026-02-25-fix-conversation-stops-and-missing-logs-plan.md`
- PR #16: TUI bugs (empty response display, history loading, log panel)
- PR #17: Conversation stops and missing logs (current branch)
