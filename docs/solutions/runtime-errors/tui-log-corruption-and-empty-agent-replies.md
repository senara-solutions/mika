---
title: Fix TUI Log Corruption and Empty Agent Replies
date: 2026-02-25
category: runtime-errors
tags:
  - agent-loop
  - tracing
  - tui
  - claude-api
  - logging
  - ratatui
severity: high
components:
  - crates/mika-agent
  - crates/mika-cli
  - crates/mika-common
symptoms:
  - Logs appearing in TUI causing visual chaos
  - Agent stops replying after a few turns
  - Empty response strings from agent when no text content generated
---

# Fix TUI Log Corruption and Empty Agent Replies

## Problem Symptom

Two critical bugs made the TUI "completely unusable":
1. **Log output in TUI**: tracing's stderr pretty layer was visible in the ratatui alternate screen, causing visual corruption
2. **Agent stops replying**: After tool-use-only turns (e.g., storing a fact), the agent returned empty text and the conversation appeared frozen

## Root Cause Analysis

### Bug 1: TUI Log Corruption

Ratatui's `EnterAlternateScreen` only covers **stdout**. The tracing subscriber's pretty stderr layer continued writing to stderr, which overlays on top of the TUI display without being managed by ratatui's screen buffer. This corrupts the visual display.

The `init_pretty` function had no way to suppress stderr output — it always added a pretty stderr layer regardless of whether the caller was a TUI or a CLI command.

### Bug 2: Agent Stops Replying

The Claude Messages API can respond with `stop_reason: end_turn` but only `ToolUse` content blocks (zero `Text` blocks). In this case, `response.text()` returns `""`. The original `run_agent` returned `Result<String>`, making empty responses indistinguishable from real text responses.

The propagation path:
- **Agent layer**: Returns `Ok("")` — looks like success
- **TUI layer**: Checks `is_empty()`, sets status to Idle with zero user feedback
- **CLI ask**: Prints blank line
- **Server**: Sends empty HTTP 200 body

The user sees the conversation appear to hang — tool execution completes silently with no acknowledgment.

## Solution

### Fix 1: LogOutput Enum for stderr Suppression

Added a `LogOutput` enum to `crates/mika-common/src/logging.rs`:

```rust
pub enum LogOutput {
    /// Pretty stderr + file (non-TUI CLI commands)
    PrettyAndFile,
    /// File only, no stderr (TUI mode)
    FileOnly,
}
```

The `init_pretty` function now matches on `(log_dir, output)` with four arms. The arms look duplicative but **cannot be deduplicated** — tracing_subscriber's type-level layer composition creates distinct monomorphic types for each `.with()` chain, preventing extraction of shared setup code.

Call site in `main.rs`:
```rust
let is_tui = matches!(cli.command, None | Some(Commands::Chat));
let log_output = if is_tui { LogOutput::FileOnly } else { LogOutput::PrettyAndFile };
let _log_guard = init_pretty(&log_level, log_dir.as_deref(), log_output);
```

### Fix 2: Option<String> Return Type + Follow-Up Injection

Changed `run_agent` return type from `Result<String>` to `Result<Option<String>>`:
- `Some(text)` — agent produced text
- `None` — tool-use-only turn (valid, no text blocks)

Added follow-up injection: when Claude ends a turn with only tool calls (no text), the agent injects a synthetic user message `"[Briefly confirm what you just did.]"` to get an acknowledgment. This is attempted once; if the follow-up also produces no text, `None` is returned.

```rust
if tool_use_occurred && !follow_up_attempted {
    follow_up_attempted = true;
    request.messages.push(Message {
        role: "assistant".to_string(),
        content: MessageContent::Blocks(response.content),
    });
    request.messages.push(Message {
        role: "user".to_string(),
        content: MessageContent::Text("[Briefly confirm what you just did.]".to_string()),
    });
    continue;
}
```

### Additional Fixes (Code Review)

- **Agent-native parity**: Server handler sends `"Done."` fallback on `None` response so Telegram users always get a reply
- **Unified fallback constant**: `EMPTY_RESPONSE_FALLBACK` shared across TUI, CLI ask, and server
- **MIKA_LOG_LEVEL env var**: CLI now checks env var before config files (consistent with server mode)
- **Log level allowlist**: `parse_log_level` rejects filter directives, only accepting `trace|debug|info|warn|error|off`
- **Team agent follow-up**: `run_team_agent_inner` now has the same follow-up injection pattern

## Key Files Changed

| File | Change |
|------|--------|
| `crates/mika-common/src/logging.rs` | `LogOutput` enum, `init_pretty` signature change |
| `crates/mika-agent/src/agent.rs` | `Option<String>` return, follow-up injection, `EMPTY_RESPONSE_FALLBACK` |
| `crates/mika-cli/src/main.rs` | `is_tui` flag, `LogOutput` usage, `MIKA_LOG_LEVEL` env var |
| `crates/mika-agent/src/server/handlers.rs` | Fallback response on `None` |
| `crates/mika-cli/src/tui/app.rs` | System message on empty response |
| `crates/mika-cli/src/commands/ask.rs` | Handle `None` with fallback message |

## Prevention Strategies

1. **TUI + stderr rule**: Any TUI application must suppress stderr logging. Use `LogOutput::FileOnly` enum variant to make this explicit in the type system.

2. **Option<T> over empty T**: When a value may be legitimately absent, use `Option<T>` not empty `String`. The compiler forces exhaustive pattern matching, preventing missed code paths.

3. **tracing_subscriber deduplication gotcha**: `.with()` chaining creates distinct generic types at compile time. Do not attempt to extract shared layer setup into helper functions — it won't compile.

4. **WorkerGuard lifetime**: The `_log_guard` returned by `init_pretty` MUST live until program exit. Dropping it stops file logging.

5. **Validate config input**: Treat config file content as untrusted. Use allowlists for log levels to prevent filter directive injection.

6. **Initialization order**: Resolve agent name -> compute paths -> load config -> initialize tracing. `tracing_subscriber::init()` can only run once.

## Related Documentation

- [empty-response-and-log-directory-fixes.md](../ui-bugs/empty-response-and-log-directory-fixes.md) — Earlier fix for the same class of issues
- [claude-api-error-message-formatting.md](../runtime-errors/claude-api-error-message-formatting.md) — Error handling patterns for Claude API
- [mika-cli-21-findings-parallel-resolution.md](../code-review-workflow/mika-cli-21-findings-parallel-resolution.md) — Previous TUI rendering fixes

## Branch & Commits

- **Branch**: `fix/conversation-stops-and-missing-logs`
- **Key commits**: `a089feb` (main fix), `1ed2802` (review findings), `911ef2d` (P3 todos resolved)
