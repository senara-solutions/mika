---
title: "TUI slash command reliability: /clear session reset, /provider and /model pre-validation"
category: ui-bugs
date: 2026-03-30
tags: [tui, slash-commands, session-management, provider-switching, state-divergence, testing]
issues: ["#342", "#343", "#344"]
---

# TUI Slash Command Reliability: /clear, /provider, /model

## Problem

Three related TUI slash command bugs caused by insufficient state management:

1. **`/clear` was display-only** — cleared `app.messages` Vec but did not create a new session, end the old one, or notify the agent worker. The header kept showing the old session ID. The agent retained full conversation context despite the user expecting a fresh start.

2. **`/provider` and `/model` used optimistic UI updates** — `app.model` and `app.provider` were updated immediately, then `AgentRequest::SetModel` was fire-and-forget via `let _ = app.agent_tx.send(...)`. If `make_llm_provider()` failed on the worker side (e.g., missing API key), the display showed the new model but the worker continued using the old one. Users could send messages believing they were using GPT-4o while the worker was still on Claude.

3. **Config persistence errors were invisible** — `write_config_toml()` failures were logged via `tracing::warn` but the user saw "Switched to X" regardless. Changes would silently revert on restart.

## Root Cause

- **`/clear`**: No `AgentRequest` variant existed to communicate session changes to the worker. The worker captured `session_id` as an immutable local variable.
- **`/provider`/`/model`**: No pre-validation before UI state mutation. The TUI handler and agent worker had no confirmation protocol — the handler assumed success.
- **Config errors**: Error results were discarded with `let _ =` or logged without user-facing feedback.

## Solution

### 1. `/clear` — Session Reset via `AgentRequest::NewSession`

Added `AgentRequest::NewSession { session_id: String }` variant. The `/clear` handler now:
- Ends the current session via `app.db.end_session()`
- Creates a new session with `Uuid::new_v4()`
- Sends `NewSession` to the worker (worker updates its `worker_session` binding)
- Resets `app.session_id`, `last_seen_msg_id`, `context_tokens`, `messages_layout`

Worker-side change: `let worker_session` → `let mut worker_session` + match arm for `NewSession`.

### 2. `/provider` and `/model` — Pre-Validation

Both handlers now call `validate_provider_switch_for()` BEFORE updating UI state:

```rust
fn validate_provider_switch_for(
    home_dir: &Path,
    global_home: &Path,
    provider: ProviderKind,
) -> Result<(), String> {
    let mut settings = Settings::load_for_agent(global_home, home_dir)?;
    settings.llm_provider = provider;
    settings.make_llm_provider()?; // No network call, just construction
    Ok(())
}
```

If validation fails, the user gets a clear error and UI state is unchanged.

### 3. Error Surfacing

- Config persistence failures append a warning: `"(warning: failed to save — change won't survive restart)"`
- Channel send failures return: `WORKER_NOT_RESPONDING` constant
- `/provider set api_key` now notes: `"(restart required for changes to take effect)"`

### 4. Test Infrastructure

Created `TestApp` builder in the test module enabling async handler tests:
- In-memory `AsyncDatabase` (real SQLite)
- Captured `mpsc::UnboundedReceiver<AgentRequest>` for verifying worker messages
- `NoopLlmProvider` satisfying `Arc<dyn LlmProvider>` type requirement
- 19 tests covering session reset, provider switching, model alias resolution, error handling

## Key Patterns

### Validate-First, Persist-Second

The `/model` handler initially wrote to `config.toml` before validating, leaving dirty config on disk if validation failed. The review caught this: always validate first, persist only on success. Same pattern as `/provider`.

### Three-File Update Rule for Slash Commands

From institutional learning `tui-dashboard-slash-command-removal-footer-dispatch.md`: slash command changes must touch `commands/mod.rs` (definition), `commands/handlers.rs` (handler), and `commands/completers.rs` (completer). This fix updated `mod.rs` and `handlers.rs`; completers were unchanged because no argument behavior changed.

### `WORKER_NOT_RESPONDING` Constant

The error message `"Error: Agent worker is not responding. Try /exit and restart."` appeared in 4 places. Extracted to a module-level constant for single source of truth.

## Prevention

- **New slash commands that mutate app state** should always validate before mutation and handle channel send errors explicitly (never `let _ = send(...)`).
- **Config-writing commands** should surface persistence failures to the user, not just log them.
- **Session-affecting commands** should use `AgentRequest::NewSession` rather than restarting the worker (preserves MCP connections, settings, etc.).
- **The `TestApp` builder** is now available for testing any TUI command handler — use it for new slash commands.

## Files Changed

- `crates/mika-cli/src/tui/app.rs` — `AgentRequest::NewSession` variant
- `crates/mika-cli/src/commands/chat.rs` — Worker-side `NewSession` handling
- `crates/mika-cli/src/tui/commands/handlers.rs` — Handler fixes + test infrastructure
- `crates/mika-cli/src/tui/commands/mod.rs` — Updated `/clear` description
- `docs/slash-commands.md` — Added `/provider` documentation, updated `/clear`
