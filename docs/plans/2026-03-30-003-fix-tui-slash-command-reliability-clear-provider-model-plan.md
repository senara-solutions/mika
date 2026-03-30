---
title: "fix: TUI slash command reliability — /clear, /provider, /model"
type: fix
status: completed
date: 2026-03-30
issues: ["#342", "#343", "#344"]
---

# fix: TUI slash command reliability — /clear, /provider, /model

## Overview

Three related TUI slash command bugs share the same root cause: insufficient state management and error propagation. `/clear` doesn't reset the session ID (#342), and `/provider`/`/model` commands silently diverge between display state and agent worker state when the provider rebuild fails (#343). This plan fixes all three in a single pass, adding test infrastructure to prevent regressions.

## Problem Statement

1. **`/clear` is display-only** — clears `app.messages` Vec but does not create a new session, end the old one, or notify the agent worker. The header continues showing the old session ID. The agent retains full conversation context.

2. **`/provider` and `/model` use optimistic UI updates** — `app.model` and `app.provider` are updated immediately, then `AgentRequest::SetModel` is fire-and-forget via `let _ = app.agent_tx.send(...)`. If `make_llm_provider()` fails on the worker side (e.g., missing API key), the display shows the new model but the worker uses the old one. The user sends messages believing they're using GPT-4o while the worker is still on Claude.

3. **Config persistence errors are invisible** — `write_config_toml()` failures are logged via `tracing::warn` but the user sees "Switched to X" regardless. The change reverts on restart.

4. **No async test infrastructure** — `App::new()` requires live channels, `AsyncDatabase`, paths, and `SkillRegistry`. The existing tests only cover pure sync helper functions.

## Proposed Solution

### Phase 1: Test Infrastructure (`crates/mika-cli/src/tui/commands/test_helpers.rs`)

Create a `TestApp` builder that provides:
- In-memory `AsyncDatabase` (real SQLite, like `EvalHarness`)
- Captured `mpsc::UnboundedReceiver<AgentRequest>` for verifying messages sent to worker
- Temp directory for `home_dir` and `global_home`
- Minimal `SkillRegistry` (empty)
- Helper methods to assert state (`session_id`, `model`, `provider`, `messages`)

```rust
// crates/mika-cli/src/tui/commands/test_helpers.rs
pub struct TestApp {
    pub app: App<'static>,
    pub agent_rx: mpsc::UnboundedReceiver<AgentRequest>,
    pub _temp_dir: TempDir,
}

impl TestApp {
    pub async fn new() -> Self { ... }
    pub fn drain_requests(&mut self) -> Vec<AgentRequest> { ... }
}
```

### Phase 2: Fix `/clear` — Session Reset (#342)

**Approach:** Add `AgentRequest::NewSession { session_id: String }` variant. This is lighter than restarting the worker (which resets MCP connections, settings, etc.).

**`/clear` handler changes:**
1. End the current session: `app.db.end_session(&app.session_id).await`
2. Generate new session ID: `Uuid::new_v4().to_string()`
3. Create new session: `app.db.create_session(agent_id, &new_session_id, "cli").await`
4. Send `AgentRequest::NewSession { session_id: new_session_id.clone() }` to worker
5. Update `app.session_id = new_session_id`
6. Clear `app.messages`, reset `app.scroll_offset = 0`
7. Reset `app.last_seen_msg_id = 0`
8. Reset `app.context_tokens = None`
9. Set `app.needs_redraw = true`

**Worker-side handling** (in `chat.rs` agent worker loop):
```rust
AgentRequest::NewSession { session_id } => {
    worker_session = session_id;
}
```

**Callback fate:** Pending callbacks from the old session are NOT re-mapped. They are delivered to the old (now ended) session. This matches the behavior of `/switch` (agent switch), which also ends the old session without migrating callbacks. The risk is low — long-running tasks that complete after `/clear` will have their results available via `search_tool_history` but won't be injected into the new chat.

**`--all` flag:** Out of scope. Remove the `[--all]` hint from the command definition to avoid confusion. Can be added later.

### Phase 3: Fix `/provider` and `/model` — Eliminate State Divergence (#343)

**Approach:** Pre-validate on the handler side before updating the UI. Both handlers are `async`, so they can load settings and test `make_llm_provider()`.

**For `/provider <name>` (provider switch):**
1. Load current settings from `app.home_dir` config
2. Clone settings, set `llm_provider` to new provider
3. Call `settings.make_llm_provider()` — if it fails, return error message to user (e.g., "Cannot switch to openai: MIKA_OPENAI_API_KEY is not set")
4. Only on success: update `app.provider`, `app.model`, send `SetModel`, persist to config
5. If config persistence fails, append warning: "Switched to openai (warning: failed to save — change won't survive restart)"

**For `/model <alias>` (model switch):**
1. Resolve alias via `resolve_model_name()` (existing)
2. Load settings, clone, set model
3. Call `settings.make_llm_provider()` to validate
4. Only on success: update `app.model`, send `SetModel`, persist
5. Same persistence warning pattern

**For `let _ = app.agent_tx.send(...)`:**
Replace with:
```rust
if app.agent_tx.send(request).is_err() {
    return "Error: Agent worker is not responding. Try /exit and restart.".to_string();
}
```

### Phase 4: Tests

Tests in `crates/mika-cli/src/tui/commands/handlers.rs` `#[cfg(test)] mod tests`:

**`/clear` tests:**
- `test_clear_creates_new_session` — session_id changes, messages empty, scroll reset
- `test_clear_sends_new_session_request` — verify `AgentRequest::NewSession` sent via captured rx
- `test_clear_ends_old_session` — old session marked ended in DB

**`/provider` tests:**
- `test_provider_switch_updates_state` — app.provider and app.model change
- `test_provider_switch_sends_set_model` — verify `AgentRequest::SetModel` content
- `test_provider_switch_missing_api_key_does_not_update` — state unchanged on validation failure
- `test_provider_no_args_shows_list` — displays current provider and available providers
- `test_provider_set_model_persists` — config.toml updated
- `test_provider_set_api_key_writes_env` — .env updated

**`/model` tests:**
- `test_model_alias_resolves` — "opus" → correct full model ID
- `test_model_unknown_returns_error` — unknown alias returns error, state unchanged
- `test_model_no_args_shows_current` — displays current model and aliases
- `test_model_updates_state_and_sends_request` — app.model changes, SetModel sent

### Phase 5: Documentation

**`docs/slash-commands.md`** — Add `/provider` section:
- `/provider` — show current provider and available providers
- `/provider <name>` — switch to a different LLM provider
- `/provider set model <name>` — set model for current provider
- `/provider set api_key <value>` — set API key (requires restart)
- `/provider set base_url <url>` — set custom base URL

## Technical Considerations

- **`AgentRequest` enum change:** Adding `NewSession` is backward-compatible. The enum is internal to `mika-cli` (not serialized).
- **Settings loading in handlers:** `Settings::new()` reads `config.toml` + env vars. This is cheap (file read + env scan). Handlers already have `app.home_dir` and `app.global_home`.
- **`make_llm_provider()` in handler context:** This constructs the provider (HTTP client, API key validation). It does NOT make a network call, so it's safe to call in the UI thread.
- **Test isolation:** Each test creates its own `TestApp` with isolated temp dirs and in-memory DB. No shared state between tests.

## System-Wide Impact

- **Interaction graph:** `/clear` now: end_session DB write → NewSession channel send → worker updates session → UI clears. `/provider`/`/model` now: Settings load → make_llm_provider validation → UI update → SetModel channel send → config persist.
- **Error propagation:** Validation errors surface to user as command output. Config persistence errors append warnings. Channel send failures produce actionable error messages.
- **State lifecycle risks:** `/clear` session transition is atomic from the user's perspective (all UI state updated in one handler call). The worker receives `NewSession` asynchronously but this is safe — messages between the clear and the session update go to the old session, which is ended but still valid for writes.
- **API surface parity:** No API changes. These are TUI-only fixes.

## Acceptance Criteria

- [x] `/clear` creates a new session (new UUID in header)
- [x] `/clear` ends the old session in the DB
- [x] `/clear` sends `AgentRequest::NewSession` to the worker
- [x] `/clear` resets context_tokens and last_seen_msg_id
- [x] `/provider <name>` with missing API key shows error and does NOT update UI
- [x] `/provider <name>` with valid config updates UI, worker, and persists
- [x] `/model <alias>` resolves alias and validates before updating UI
- [x] Config persistence failures produce user-visible warnings
- [x] Channel send failures produce user-visible errors
- [x] All new behaviors covered by async handler tests
- [x] `/provider` documented in `docs/slash-commands.md`
- [x] `[--all]` hint removed from `/clear` command definition
- [x] All existing tests pass (`cargo test -p mika-cli`)

## Key Files

| File | Changes |
|------|---------|
| `crates/mika-cli/src/tui/commands/handlers.rs` | Fix `/clear`, `/provider`, `/model` handlers; add tests |
| `crates/mika-cli/src/tui/commands/test_helpers.rs` | New: `TestApp` builder for async handler tests |
| `crates/mika-cli/src/tui/commands/mod.rs` | Remove `[--all]` from `/clear` definition; add `mod test_helpers` |
| `crates/mika-cli/src/tui/app.rs` | Add `AgentRequest::NewSession` variant |
| `crates/mika-cli/src/commands/chat.rs` | Handle `NewSession` in worker loop |
| `docs/slash-commands.md` | Add `/provider` documentation |

## Sources & References

- Related issues: #342, #343, #344
- Institutional learning: `docs/solutions/ui-bugs/tui-dashboard-slash-command-removal-footer-dispatch.md` — three-file update pattern for slash commands
- Institutional learning: `docs/solutions/architecture-patterns/cli-model-override-one-shot.md` — `override_model()` two-step invariant
- Agent worker loop: `crates/mika-cli/src/commands/chat.rs` lines 395-417
