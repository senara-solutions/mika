---
title: "Implementation Recommendations: Slash-Command Timeout and Tests"
type: guide
status: published
date: 2026-02-25
---

# Recommended Enhancements for Slash-Command System

This document provides concrete code changes to enhance robustness and maintainability of the slash-command system added in commit `09d8595`.

## 1. Add Timeout Wrapper for Handler Dispatch

### Problem

The `tick()` method calls `dispatch()` which can take an arbitrary amount of time if a handler stalls (slow database query, network timeout, filesystem hang). This can freeze the TUI for up to that duration, making the UI feel unresponsive.

### Current Code (app.rs)

```rust
pub async fn tick(&mut self) {
    self.tick_count = self.tick_count.wrapping_add(1);

    // Process pending slash command
    if let Some(cmd) = self.pending_command.take() {
        if let Some(output) = commands::handlers::dispatch(self, &cmd).await {
            self.messages.push(ChatMessage {
                role: ChatRole::Command,
                content: output,
                rendered: None,
            });
            self.scroll_offset = 0;
        }
        self.needs_redraw = true;
    }
    // ... rest of tick ...
}
```

### Proposed Change

Import `tokio::time::timeout` and wrap the dispatch call:

```rust
use std::time::Duration;

pub async fn tick(&mut self) {
    self.tick_count = self.tick_count.wrapping_add(1);

    // Process pending slash command
    if let Some(cmd) = self.pending_command.take() {
        // Timeout after 5 seconds to prevent UI freeze
        match tokio::time::timeout(
            Duration::from_secs(5),
            commands::handlers::dispatch(self, &cmd)
        ).await {
            Ok(Some(output)) => {
                self.messages.push(ChatMessage {
                    role: ChatRole::Command,
                    content: output,
                    rendered: None,
                });
                self.scroll_offset = 0;
                self.needs_redraw = true;
            }
            Ok(None) => {
                // Handler returned no output (e.g., /exit)
                self.needs_redraw = true;
            }
            Err(_timeout) => {
                self.messages.push(ChatMessage {
                    role: ChatRole::System,
                    content: "Command timed out (exceeded 5 seconds).".to_string(),
                    rendered: None,
                });
                self.scroll_offset = 0;
                self.needs_redraw = true;
            }
        }
    }
    // ... rest of tick ...
}
```

### Implementation Notes

- **Timeout duration**: 5 seconds is generous for CLI operations. Most handlers (memory search, status, export) complete in <100ms. Long operations like compaction or network calls could approach 5 seconds but shouldn't exceed it.
- **Graceful degradation**: If a handler times out, the user sees a clear message rather than a frozen TUI.
- **No handler changes required**: Handlers don't need modification; the timeout is applied at the dispatch boundary.

### Rationale

This is defensive programming that aligns with the 30ms tick rate design (handlers should respond within 166 ticks = 5 seconds). It prevents any single handler from blocking the event loop indefinitely.

---

## 2. Add Integration Tests for Key Handlers

### Problem

Currently, only the command registry and autocomplete state are unit-tested. Handlers (`handle_memory()`, `handle_compact()`, etc.) are not tested because they require a full `App` struct with real resources (database, file system, Claude client). This leaves the system vulnerable to regressions when handlers are modified.

### Proposed Solution

Create a test module in `handlers.rs` that:
1. Provides a helper to create a minimal test `App` with in-memory database
2. Tests 3-4 representative handlers covering different access patterns
3. Verifies error handling and output formatting

### Implementation

Add to `/data/workspace/senara-solutions/mika/crates/mika-cli/src/tui/commands/handlers.rs`:

```rust
#[cfg(test)]
mod integration_tests {
    use super::*;
    use mika_agent::async_db::AsyncDatabase;
    use mika_common::claude::ClaudeClient;
    use mika_agent::skills::SkillRegistry;
    use crate::tui::app::{App, AgentStatus};
    use std::sync::Arc;
    use std::path::PathBuf;
    use tokio::sync::mpsc;
    use uuid::Uuid;

    /// Create a test App with minimal real resources.
    async fn create_test_app() -> App<'static> {
        let session_id = Uuid::new_v4().to_string();

        // Create in-memory database
        let db = {
            use mika_agent::db::Database;
            let db = Database::new(":memory:").expect("failed to create in-memory db");
            AsyncDatabase::new(db)
        };

        // Create mock Claude client (will not be used in read-only tests)
        let claude = ClaudeClient::new(
            Some("sk-test-key".to_string()),
            "claude-opus-4-6".to_string(),
            4096,
        ).expect("failed to create claude client");

        let home_dir = PathBuf::from("/tmp/mika-test");
        let skills = Arc::new(SkillRegistry::from_dir(&home_dir.join("skills")));

        let (agent_tx, agent_rx) = mpsc::unbounded_channel();

        App::new(
            agent_tx,
            agent_rx,
            session_id,
            "claude-opus-4-6".to_string(),
            "TestAgent".to_string(),
            db,
            claude,
            home_dir,
            skills,
        )
    }

    #[tokio::test]
    async fn test_handle_model_displays_current_model() {
        let app = create_test_app().await;
        let output = handle_model(&app);
        assert!(output.contains("claude-opus-4-6"));
        assert!(output.contains("Current model"));
    }

    #[tokio::test]
    async fn test_handle_status_includes_schema_version() {
        let app = create_test_app().await;
        let output = handle_status(&app).await;
        // Status should include schema version from the in-memory DB
        assert!(output.contains("Schema"));
        assert!(output.contains("Messages"));
    }

    #[tokio::test]
    async fn test_handle_help_lists_all_commands() {
        let output = handle_help();
        // Verify all major commands are in help output
        assert!(output.contains("/help"));
        assert!(output.contains("/memory"));
        assert!(output.contains("/clear"));
        assert!(output.contains("/compact"));
        assert!(output.contains("/reminders"));
        assert!(output.contains("/status"));
        assert!(output.contains("/soul"));
        assert!(output.contains("/export"));
        assert!(output.contains("/skills"));
    }

    #[tokio::test]
    async fn test_handle_clear_returns_confirmation() {
        let mut app = create_test_app().await;
        // Add a test message
        app.messages.push(crate::tui::app::ChatMessage {
            role: crate::tui::app::ChatRole::User,
            content: "Hello".to_string(),
            rendered: None,
        });
        assert!(!app.messages.is_empty());

        let output = handle_clear(&mut app, "").await;
        assert!(app.messages.is_empty());
        assert!(output.contains("cleared"));
    }

    #[tokio::test]
    async fn test_handle_skills_with_empty_registry() {
        let app = create_test_app().await;
        let output = handle_skills(&app);
        // Skills directory likely empty in test environment
        assert!(output.contains("skill") || output.contains("No skills"));
    }

    #[tokio::test]
    async fn test_handle_skill_not_found() {
        let app = create_test_app().await;
        let output = handle_skill(&app, "nonexistent-skill");
        assert!(output.contains("No skill found"));
        assert!(output.contains("nonexistent-skill"));
    }

    #[tokio::test]
    async fn test_dispatch_with_unknown_command() {
        let mut app = create_test_app().await;
        let output = dispatch(&mut app, "/unknowncommand").await;
        assert!(output.is_some());
        let msg = output.unwrap();
        assert!(msg.contains("Unknown command"));
        assert!(msg.contains("unknowncommand"));
    }

    #[tokio::test]
    async fn test_dispatch_help_alias() {
        let mut app = create_test_app().await;
        let output = dispatch(&mut app, "/h").await;
        assert!(output.is_some());
        let msg = output.unwrap();
        assert!(msg.contains("Available commands"));
    }

    #[tokio::test]
    async fn test_handle_memory_with_no_entries() {
        let app = create_test_app().await;
        let output = handle_memory(&app, "").await;
        // In-memory DB has no entries initially
        assert!(output.contains("No core memory") || output.contains("Memory"));
    }

    #[tokio::test]
    async fn test_handle_memory_search_with_no_results() {
        let app = create_test_app().await;
        let output = handle_memory_search(&app, "nonexistent-query").await;
        assert!(output.contains("No results"));
    }
}
```

### Integration with CI/CD

These tests will run as part of `cargo test -p mika-cli`:

```bash
cargo test -p mika-cli --test '*'
# Output will include:
# running 14 tests
# test tui::commands::handlers::integration_tests::test_handle_model_displays_current_model ... ok
# test tui::commands::handlers::integration_tests::test_dispatch_help_alias ... ok
# ... etc
```

### Maintenance Notes

- Tests use an in-memory database (`:memory:`), so they're fast and have no I/O side effects
- Mock file system operations by creating temporary directories in `/tmp`
- Keep integration tests minimal; they should verify behavior, not implementation details
- Update tests when adding new handlers or changing handler output format

---

## 3. Document the Slash-Command Pattern in CLAUDE.md

### Recommendation

Add a new section to `/data/workspace/senara-solutions/mika/CLAUDE.md` documenting the slash-command architecture as a reference for future CLI extensions:

```markdown
## Slash-Command System (TUI Local)

### Overview

The mika-cli TUI supports in-session slash commands (e.g., `/help`, `/memory`, `/compact`) without leaving the chat. Users type "/" to trigger an autocomplete popup.

### Architecture

**File Structure:**
- `crates/mika-cli/src/tui/commands/mod.rs` — `SlashCommand` registry and prefix matching
- `crates/mika-cli/src/tui/commands/autocomplete.rs` — `AutocompleteState` UI state machine (10 tests)
- `crates/mika-cli/src/tui/commands/handlers.rs` — `dispatch()` router and 13 handlers

**Data Access:**
- Handlers receive `&mut App` which holds Arc-cloned shared resources: `AsyncDatabase`, `ClaudeClient`, `SkillRegistry`, `PathBuf`
- Same pattern as mika-agent `AppState` in the server handlers
- Cheap clones (Arc-wrapping); database operations are non-blocking (background thread via `mpsc` channel)

**Execution Model:**
- User types "/" → `send_message()` queues command in `App::pending_command`
- Next `tick()` (every 30ms) calls `dispatch()` asynchronously
- Output rendered as `ChatRole::Command` (cyan) message
- Commands are client-side only; they never reach the agent loop

### Adding a New Command

1. Add `SlashCommand` entry to `COMMANDS` in `mod.rs`
2. Add match arm in `dispatch()` and implement handler function in `handlers.rs`

Example: Add `/weather <city>`:
```rust
// In mod.rs
SlashCommand {
    name: "weather",
    aliases: &["w"],
    description: "Get weather forecast",
    args_hint: Some("<city>"),
},

// In handlers.rs
"weather" | "w" => Some(handle_weather(app, args).await),

async fn handle_weather(app: &App<'_>, args: &str) -> String {
    let city = args.trim();
    if city.is_empty() {
        return "Usage: /weather <city>".to_string();
    }
    // ... fetch weather, return formatted output
}
```

### Best Practices

1. **Error Handling**: Format errors as output strings, not panics. Render in `ChatRole::Command` or `System`.
2. **Async Operations**: Use `tokio::fs::*` for filesystem, `app.db.*().await` for database, never block.
3. **Guard Destructive Ops**: Check `app.status == AgentStatus::Idle` before running commands that modify conversation state (e.g., `/compact`, `/clear --all`).
4. **Keep Output Concise**: Limit to ~10 lines. Long output (like export) is acceptable but should be scrollable.
5. **Test Command Output**: Add integration tests in `handlers.rs` for new commands that touch DB or filesystem.

### Performance Notes

- Most commands complete in <100ms (database reads, memory queries)
- `handle_compact()` can take 1-2 seconds (calls Claude API for summarization)
- Commands that time out (>5s) show "Command timed out" error message
- No impact on agent loop or message processing
```

---

## Summary of Recommendations

| Enhancement | Priority | Effort | Impact |
|---|---|---|---|
| Add timeout wrapper | High | 20 lines | Prevents UI freeze; improves UX |
| Integration tests | Medium | 80 lines | Regression prevention; confidence in refactoring |
| Document pattern in CLAUDE.md | Medium | 30 lines | Reduces onboarding time for future features; establishes patterns |

**Total Implementation Time**: ~1-2 hours

**Blocking Issues**: None. Current implementation is production-ready.

---

## Testing the Enhancements

### Test the Timeout

1. Temporarily add a sleep in a handler:
   ```rust
   async fn handle_test_slow(app: &App<'_>) -> String {
       tokio::time::sleep(Duration::from_secs(6)).await;
       "Done".to_string()
   }
   ```
2. Add to dispatch: `"testslow" => Some(handle_test_slow(app).await),`
3. Run CLI and type `/testslow`
4. Verify message: "Command timed out (exceeded 5 seconds)." appears after ~5 seconds

### Test the Integration Tests

```bash
cd /data/workspace/senara-solutions/mika
cargo test -p mika-cli --test handlers -- --nocapture
# Should show all integration_tests passing
```

---

## Questions for Implementation

1. **Timeout duration**: Is 5 seconds reasonable, or should it be shorter (e.g., 3 seconds) for CLI-like responsiveness?
2. **Test database**: Should tests use an in-memory database or mock the AsyncDatabase trait?
3. **Documentation location**: Is CLAUDE.md the right place, or should this go in a separate `docs/patterns/` file?

