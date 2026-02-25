---
title: "fix: Conversation stops silently + logs written to wrong directory"
type: fix
status: completed
date: 2026-02-25
---

# fix: Conversation stops silently + logs written to wrong directory

## Overview

Two bugs affecting daily use of the Mika TUI CLI:

1. **Conversation stops mid-conversation** — When Claude responds with only tool-use blocks (no text), the TUI silently transitions to idle with zero feedback. The user sees nothing and thinks Mika froze.
2. **No logs in `~/.mika/agents/main/logs/`** — Log files are written to `~/.mika/logs/` instead of the agent-specific directory, and the log level is hardcoded to `warn` ignoring the configured `log_level`.

## Problem Statement

### Bug 1: Empty response silently drops conversation

In `crates/mika-agent/src/agent.rs:189-195`, when Claude responds with `EndTurn` or `MaxTokens` but no text blocks (only `ToolUse` blocks), `response.text()` returns `""`. This empty string propagates to the TUI via `AgentResponse { content: "", is_error: false }`.

In `crates/mika-cli/src/tui/app.rs:220-222`, the TUI receives the empty content and silently transitions to `AgentStatus::Idle`:

```rust
} else if response.content.is_empty() {
    // Agent responded with tool-use only (no text) — skip display
    self.status = AgentStatus::Idle;
}
```

The same gap exists for `StopReason::StopSequence` at `agent.rs:200-206`.

The user sees nothing — no message, no error, no indication anything happened. The conversation appears to have frozen.

### Bug 2: Logs written to wrong directory

In `crates/mika-cli/src/main.rs:18`, the log directory is computed as:

```rust
let log_dir = home::resolve_home_dir().ok().map(|h| h.join("logs"));
```

This resolves to `~/.mika/logs/` — but in the multi-agent layout, each agent's logs should be at `~/.mika/agents/{name}/logs/`. The comment on line 16 even says the correct path but the code doesn't match.

Additionally, line 24 hardcodes `"warn"`:

```rust
let _log_guard = mika_common::logging::init_pretty("warn", log_dir.as_deref());
```

This ignores `settings.log_level` (which defaults to `"info"`), meaning most log events are suppressed.

## Proposed Solution

### Bug 1: Show feedback on empty agent response

**Agent layer** (`crates/mika-agent/src/agent.rs`):
- Add a `warn!` log when `response.text()` returns empty on a terminal stop reason
- No change to the return value — the agent correctly returns `""` (the empty text *is* the response)

**TUI layer** (`crates/mika-cli/src/tui/app.rs`):
- When `response.content.is_empty()` and `!response.is_error`, push a `ChatRole::System` message: `"Agent processed your request."`
- This gives the user visible feedback that the agent did something (tool calls) but had nothing to say

### Bug 2: Fix log directory and level

**`crates/mika-cli/src/main.rs`**:
- Move log directory resolution after agent name resolution (line 27-34)
- Compute log dir as `resolve_agent_home(global_home, agent_name).join("logs")`
- Read `log_level` from the agent's config file (load settings before `init_pretty`)
- Fallback: if settings load fails, use `"warn"` + stderr-only (no file logging)

**Trade-off**: There is a narrow window between process start and `init_pretty` where tracing events are dropped. This is acceptable because:
- Agent name resolution and config loading are fast, deterministic operations
- Errors in this window surface via `anyhow::Result` to stderr
- The current code already has this gap (logging is initialized before any agent work)

## Acceptance Criteria

- [x] When Claude responds with tool-only content (no text), the TUI shows "Agent processed your request." as a system message
- [x] The warning is logged at `warn` level in the agent layer when empty text is returned after tool execution
- [x] Log files are written to `~/.mika/agents/{agent_name}/logs/mika.log.YYYY-MM-DD`
- [x] `log_level` from agent config is respected (default `"info"`)
- [x] `RUST_LOG` env var still takes precedence over config (existing behavior preserved)
- [x] Legacy layout (no `agents/` dir) continues to work — logs go to `~/.mika/logs/`
- [x] Existing tests pass, new tests added for both fixes

## MVP

### Bug 1 — `crates/mika-agent/src/agent.rs`

Add warning log when returning empty text on terminal stop reasons:

```rust
// agent.rs:189-196 — EndTurn | MaxTokens arm
StopReason::EndTurn | StopReason::MaxTokens => {
    let text = response.text();
    if !text.is_empty() {
        db.save_message("assistant", &text, channel_type).await?;
    } else {
        warn!(step, stop_reason = ?response.stop_reason, "agent returned empty text (tool-only response)");
    }
    info!(step, stop_reason = ?response.stop_reason, "agent done");
    return Ok(text);
}

// agent.rs:200-206 — StopSequence arm (same fix)
StopReason::StopSequence => {
    let text = response.text();
    if !text.is_empty() {
        db.save_message("assistant", &text, channel_type).await?;
    } else {
        warn!(step, "agent returned empty text on StopSequence");
    }
    return Ok(text);
}
```

### Bug 1 — `crates/mika-cli/src/tui/app.rs`

Show system message instead of silent idle:

```rust
// app.rs:220-222
} else if response.content.is_empty() {
    // Agent responded with tool-use only (no text) — show feedback
    self.messages.push(ChatMessage {
        role: ChatRole::System,
        content: "Agent processed your request.".to_string(),
        rendered: None,
    });
    self.status = AgentStatus::Idle;
}
```

### Bug 2 — `crates/mika-cli/src/main.rs`

Reorder initialization: resolve agent name first, load settings, then init logging:

```rust
#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // Resolve agent name first — needed for correct log directory.
    let agent_name = match cli.agent {
        Some(name) => {
            let name = agent::normalize_agent_name(&name);
            agent::validate_agent_name(&name)?;
            name
        }
        None => init::resolve_active_agent()?,
    };

    // Resolve log directory: ~/.mika/agents/{name}/logs/
    // Uses agent-specific home so logs land in the correct agent directory.
    let global_home = home::resolve_home_dir().ok();
    let log_dir = global_home.as_ref().map(|h| {
        home::resolve_agent_home(h, &agent_name).join("logs")
    });

    // Read log_level from config, falling back to "warn" if config load fails.
    let log_level = global_home
        .as_ref()
        .and_then(|h| {
            let agent_home = home::resolve_agent_home(h, &agent_name);
            // Quick read of just the log_level from config.toml
            let config_path = agent_home.join("config.toml");
            std::fs::read_to_string(&config_path).ok()
        })
        .and_then(|content| {
            content.lines()
                .find(|l| l.trim().starts_with("log_level"))
                .and_then(|l| l.split('=').nth(1))
                .map(|v| v.trim().trim_matches('"').to_string())
        })
        .unwrap_or_else(|| "warn".to_string());

    // Initialize tracing with correct directory and level.
    let _log_guard = mika_common::logging::init_pretty(&log_level, log_dir.as_deref());

    match cli.command {
        // ... (unchanged)
    }
}
```

**Note on log_level loading**: We intentionally do a lightweight TOML parse (just find the `log_level` line) rather than loading full `Settings`, because `Settings::load` may require the database and other infrastructure that isn't available yet. This avoids pulling in config-rs and its dependency chain before logging is ready.

## Testing

### Bug 1 tests

- **Unit test in `app.rs`**: Deliver `AgentResponse { content: "", is_error: false }` via channel, call `tick()`, assert `ChatRole::System` message with "Agent processed your request." is pushed, and `status == Idle`
- **Unit test in `app.rs`**: Deliver `AgentResponse { content: "hello", is_error: false }`, assert progressive reveal starts (no system message)

### Bug 2 tests

- **Unit test in `main.rs` or integration test**: For multi-agent layout, assert computed log path is `~/.mika/agents/{name}/logs/`
- **Unit test**: For legacy layout (no `agents/` dir), assert log path falls back to `~/.mika/logs/`
- **Verify manually**: Run `mika`, check that `~/.mika/agents/main/logs/mika.log.*` files appear with `info`-level entries

## References

- Agent loop: `crates/mika-agent/src/agent.rs:179-215`
- TUI response handling: `crates/mika-cli/src/tui/app.rs:210-243`
- CLI main: `crates/mika-cli/src/main.rs:13-64`
- Logging init: `crates/mika-common/src/logging.rs:18-53`
- Home directory: `crates/mika-common/src/home.rs:6-13, 61-67`
- Chat worker: `crates/mika-cli/src/commands/chat.rs:75-113`
- Prior art: `docs/solutions/code-review-workflow/mika-cli-21-findings-parallel-resolution.md` (P1: missing tracing)
- Prior art: `docs/solutions/runtime-errors/claude-api-error-message-formatting.md` (error patterns)
