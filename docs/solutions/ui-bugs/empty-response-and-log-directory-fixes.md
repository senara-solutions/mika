---
title: "Fix conversation stops on empty response and misplaced log directory"
date: 2026-02-25
category: ui-bugs
tags: [tui, logging, agent-loop, initialization, type-safety]
severity: medium
modules: [mika-cli, mika-agent]
symptoms:
  - "Conversation stops mid-flow with no feedback"
  - "No log files in ~/.mika/agents/main/logs/"
root_cause:
  - "Empty text from tool-only Claude responses silently dropped"
  - "Log directory computed before agent name resolved"
---

# Fix: Conversation Stops on Empty Response and Missing Log Directory

## Problem Symptom

Two related bugs reported during a conversation with Mika:

1. **Conversation stops**: Mid-conversation, the TUI would go idle with no visible response after the agent processed tool calls. The user saw the input become unresponsive with no error or feedback.

2. **Missing logs**: The `~/.mika/agents/main/logs/` directory contained no log files, making it impossible to debug the first issue.

## Investigation Steps Tried

1. **Examined `run_agent_inner` in `agent.rs`**: Found that `response.text()` returns an empty string when Claude responds with only `ToolUse` content blocks (no text blocks). The empty string was returned to callers without any signal.

2. **Traced TUI flow in `app.rs` and `chat.rs`**: The TUI received the empty string and silently went idle — no system message, no error, no indication to the user.

3. **Checked `main.rs` initialization order**: Found that `init_pretty()` (which sets up the file-based log appender) was called with a log directory derived from the global home (`~/.mika/logs/`), not the agent-specific home (`~/.mika/agents/main/logs/`). The agent name was resolved *after* logging was initialized.

4. **Examined `parse_log_level`**: Found a hand-rolled TOML line scanner that had edge cases — `starts_with("log_level")` could match `log_level_override`, and it didn't handle inline TOML comments.

## Root Cause Analysis

### Bug 1: Conversation Stops

The Claude Messages API returns `stop_reason: "end_turn"` with content blocks that may contain only `ToolUse` blocks and no `Text` blocks. When this happens, `response.text()` returns `""`. The agent loop returned this empty string to:

- **TUI (`app.rs`)**: Received empty string, had a guard `if response.content.is_empty()` but it set status to `Idle` without any user feedback.
- **CLI (`ask.rs`)**: Printed a blank line to stdout.
- **Server (`handlers.rs`)**: Sent an empty response body.

The type signature `Result<String>` made the empty case implicit — callers had to independently remember to check for empty strings.

### Bug 2: Missing Logs

In `main.rs`, the initialization order was:

1. Parse CLI args
2. Compute log directory from global home → `~/.mika/logs/`
3. Initialize tracing with that directory (one-time init, cannot be changed)
4. *Then* resolve agent name
5. Compute agent home from agent name

Since `tracing_subscriber` can only be initialized once per process, logs were written to `~/.mika/logs/` instead of `~/.mika/agents/main/logs/`. Additionally, the log level was read from the global config, not the agent-specific config.

## Working Solution

### Fix 1: Empty Response Handling (4 layers)

**Agent layer — `crates/mika-agent/src/agent.rs`:**

Changed return type from `Result<String>` to `Result<Option<String>>`:

```rust
// Before
pub async fn run_agent(...) -> Result<String> { ... }
// After
pub async fn run_agent(...) -> Result<Option<String>> { ... }
```

Added explicit `tool_use_occurred` tracking:

```rust
let mut tool_use_occurred = false;
for step in 0..MAX_TOOL_STEPS {
    let response = claude.send_message(&request).await?;
    match response.stop_reason {
        StopReason::EndTurn | StopReason::MaxTokens => {
            let text = response.text();
            if !text.is_empty() {
                db.save_message("assistant", &text, channel_type).await?;
            } else if tool_use_occurred {
                warn!(step, stop_reason = ?response.stop_reason,
                    "agent returned empty text after tool use");
            }
            return Ok(if text.is_empty() { None } else { Some(text) });
        }
        StopReason::ToolUse => {
            tool_use_occurred = true;
            process_tool_calls(response.content, tools, &tool_ctx, &mut request).await;
        }
        StopReason::StopSequence => {
            // Same empty-text handling as EndTurn
        }
    }
}
```

**TUI layer — `crates/mika-cli/src/tui/app.rs`:**

```rust
} else if response.content.is_empty() {
    self.messages.push(ChatMessage {
        role: ChatRole::System,
        content: "Agent processed your request.".to_string(),
        rendered: None,
    });
    self.status = AgentStatus::Idle;
}
```

**CLI layer — `crates/mika-cli/src/commands/ask.rs`:**

```rust
match response {
    Some(text) => println!("{text}"),
    None => eprintln!("(Agent processed your request — no text response)"),
}
```

**Server layer — `crates/mika-agent/src/server/handlers.rs`:**

```rust
match result {
    Ok(Some(response)) => { /* send response */ }
    Ok(None) => { /* no-op, tool-only turn */ }
    Err(e) => { /* error handling */ }
}
```

### Fix 2: Log Directory Initialization Order

**`crates/mika-cli/src/main.rs`:**

Reordered initialization to resolve agent name first:

```rust
// 1. Resolve agent name FIRST
let agent_name = match cli.agent {
    Some(name) => { /* normalize + validate */ }
    None => init::resolve_active_agent()?,
};

// 2. Compute agent home directly (no .parent() hack)
let global_home = home::resolve_home_dir().ok();
let agent_home = global_home
    .as_ref()
    .map(|h| home::resolve_agent_home(h, &agent_name));
let log_dir = agent_home.as_ref().map(|h| h.join("logs"));

// 3. Read log level from agent config, fall back to global
let log_level = agent_home
    .as_ref()
    .and_then(|h| std::fs::read_to_string(h.join("config.toml")).ok())
    .and_then(|content| parse_log_level(&content))
    .or_else(|| { /* global config fallback */ })
    .unwrap_or_else(|| "warn".to_string());

// 4. Initialize logging with correct directory
let _log_guard = mika_common::logging::init_pretty(&log_level, log_dir.as_deref());
```

Replaced hand-rolled TOML parser with `toml::Table`:

```rust
fn parse_log_level(content: &str) -> Option<String> {
    let table: toml::Table = content.parse().ok()?;
    table.get("log_level")?.as_str().filter(|s| !s.is_empty()).map(str::to_string)
}
```

## Prevention Strategies

### Type System Enforcement

- Use `Option<T>` instead of empty sentinel values. The `Option<String>` return type forces all callers to handle the empty case at compile time, preventing silent drops.
- When adding new return paths, ask: "Can this value be meaningfully empty? If so, make it `Option`."

### Initialization Ordering

- Dependencies in `main()` should flow downward: resolve names → compute paths → initialize subsystems. Never derive a parent path from a child path (the `.parent()` anti-pattern).
- If a subsystem can only be initialized once (like `tracing_subscriber`), ensure all inputs are fully resolved before that call.

### Testing Recommendations

- **Unit tests for `parse_log_level`**: 6 tests covering valid values, missing keys, empty strings, comments, and malformed TOML.
- **Integration pattern**: When testing initialization order, verify that the log directory matches the expected agent-specific path.

### Code Review Checklist

- [ ] Are empty/sentinel values explicit in the type system (`Option`, enum)?
- [ ] Is initialization order correct — do all dependencies resolve before use?
- [ ] Does the log/config path respect the agent-specific directory?
- [ ] Are hand-rolled parsers replaced with proper crate-based parsing?

## Cross-References

- **PR**: [#17 — Fix conversation stops and missing logs](https://github.com/senara-solutions/mika/pull/17)
- **Related PR**: [#16 — Fix TUI bugs](https://github.com/senara-solutions/mika/pull/16) — fixed related TUI issues (empty response display, history loading, log panel, input wrapping)
- **Related solution**: [mika-cli-21-findings-parallel-resolution.md](../code-review-workflow/mika-cli-21-findings-parallel-resolution.md) — parallel resolution pattern used for code review findings
- **Related solution**: [claude-api-error-message-formatting.md](../runtime-errors/claude-api-error-message-formatting.md) — error message formatting patterns for Claude API responses
- **Related solution**: [async-database-wrapper-pattern.md](../architecture-decisions/async-database-wrapper-pattern.md) — AsyncDatabase pattern used in the agent layer
- **Related solution**: [fresh-install-agent-not-found-sentinel-mismatch.md](../logic-errors/fresh-install-agent-not-found-sentinel-mismatch.md) — agent home resolution edge cases

## Files Changed

| File | Change |
|------|--------|
| `crates/mika-agent/src/agent.rs` | `Option<String>` return type, `tool_use_occurred` flag, warn on empty |
| `crates/mika-cli/src/main.rs` | Reordered init, `agent_home` stored directly, `toml::Table` parser, 6 tests |
| `crates/mika-cli/src/tui/app.rs` | System message on empty response |
| `crates/mika-cli/src/commands/ask.rs` | Empty response handling via `Option` match |
| `crates/mika-cli/src/commands/chat.rs` | Updated `run_agent` result matching for `Option<String>` |
| `crates/mika-agent/src/server/handlers.rs` | Updated to handle `Ok(Some(...))` / `Ok(None)` |
| `crates/mika-agent/src/teams/engine.rs` | `.unwrap_or_default()` on `Option<String>` |
