---
title: "CLI-to-Telegram Messaging and Bundled Skill Seeding"
category: integration-issues
severity: high
resolution_type: code-fix
date_resolved: "2026-02-26"
components:
  - mika-cli
  - mika-agent
  - mika-gateway
symptoms:
  - "CLI send_message reports 'Message delivered (CLI)' but message never reaches Telegram"
  - "Mika responds 'I don't have access to your system' when asked about tmux"
  - "Skills directory exists but is empty after agent bootstrap"
root_cause:
  - "GatewayMessageSender never constructed in CLI chat path"
  - "Bundled skill templates never copied from compile-time assets to agent skills directory"
tags:
  - multi-channel
  - messaging
  - skills
  - cli
  - telegram
  - bundled-skills
  - gateway
related_files:
  - crates/mika-cli/src/init.rs
  - crates/mika-cli/src/commands/chat.rs
  - crates/mika-cli/src/commands/ask.rs
  - crates/mika-agent/src/bundled_skills.rs
  - crates/mika-agent/src/startup.rs
  - crates/mika-agent/src/server/mod.rs
  - crates/mika-agent/src/tools/send_message.rs
  - crates/mika-cli/src/tui/app.rs
---

# CLI-to-Telegram Messaging and Bundled Skill Seeding

## Problem

Two independent issues prevented Mika from working correctly as a multi-channel assistant:

### Issue 1: CLI cannot send messages to Telegram

When a user asks Mika via CLI to "say hi on telegram", the `send_message` tool executes and reports success with "Message delivered (CLI)" — but the message never reaches Telegram. The reverse path (Telegram-to-CLI via cross-channel DB polling) works correctly.

**Observed behavior:**
- `send_message` tool logs success
- No HTTP request to gateway
- Message never appears in Telegram
- No error or warning in logs

### Issue 2: Bundled skills not available

Mika responds "I don't have access to your system" when asked to list tmux sessions or perform other skill-based tasks. The `~/.mika/agents/{name}/skills/` directory exists but is empty after agent bootstrap.

**Observed behavior:**
- `SkillRegistry::from_dir()` returns empty
- Claude has no skill tools in its tool list
- Skill template files exist in the binary's source tree but never reach the agent's runtime directory

## Investigation

### Issue 1: Tracing the message sender path

1. Checked `send_message.rs` — tool correctly checks for `Option<Arc<dyn MessageSender>>` in `ToolContext`, uses it if `Some`, falls back to local logging if `None`.
2. Checked `chat.rs:spawn_agent_worker` — `message_sender` was hardcoded to `None` in `AgentParams`.
3. Checked `Settings` — `routing_url` and `internal_token` fields exist and are populated from environment variables.
4. Checked `GatewayMessageSender` — fully functional, used correctly in the server path (`mika-server`).

**Conclusion:** The CLI path never constructed a `GatewayMessageSender` even when the required config was present.

### Issue 2: Tracing the skill loading path

1. Checked `home::bootstrap()` — creates `skills/` as an empty directory.
2. Checked `templates/skills/` — all 5 skill templates exist with complete files.
3. Checked `SkillRegistry::from_dir()` — correctly scans directory, returns empty when no skills found.
4. No code path exists to copy templates into the agent's skills directory.

**Conclusion:** The bootstrap creates the directory structure but never populates it with bundled skills.

## Root Cause

### Issue 1: Missing GatewayMessageSender construction

In `crates/mika-cli/src/commands/chat.rs`, the `spawn_agent_worker` function passed `message_sender: None` to `AgentParams`. The `Settings` struct had the required `routing_url` and `internal_token` fields, but no code in the CLI path used them to construct a `GatewayMessageSender`.

The server path (`mika-server`) correctly constructed the sender, so this was a CLI-only gap.

### Issue 2: No skill seeding on startup

`home::bootstrap()` in `mika-common` creates the directory layout (`~/.mika/agents/{name}/skills/`) but has no awareness of skill templates. The templates existed in the source tree under `templates/skills/` but were never embedded in the binary or copied to the runtime directory.

## Solution

### Fix 1: Wire GatewayMessageSender into CLI

**Created `make_message_sender` in `crates/mika-cli/src/init.rs`:**

```rust
pub fn make_message_sender(
    settings: &Settings,
    db: &AsyncDatabase,
    http_client: &reqwest::Client,
) -> Option<Arc<dyn MessageSender>> {
    let url = settings.routing_url.as_deref()?;
    let token = settings.internal_token.clone()?;

    let parsed = match reqwest::Url::parse(url) {
        Ok(parsed) => parsed,
        Err(e) => {
            tracing::warn!(error = %e, "invalid routing_url, skipping gateway message sender");
            return None;
        }
    };

    if !matches!(parsed.scheme(), "http" | "https") {
        tracing::warn!(
            scheme = parsed.scheme(),
            "routing_url must use http or https scheme"
        );
        return None;
    }

    let sender = GatewayMessageSender::new(
        url.to_string(),
        token,
        db.clone(),
        http_client.clone(),
        None,
    );
    Some(Arc::new(sender))
}
```

**Wired into all CLI paths:**
- `chat.rs:spawn_agent_worker` — passes sender to both `ReminderScheduler` and `AgentParams`
- `ask.rs:run` — passes sender to `AgentParams`

**Key design decisions:**
- Function lives in `init.rs` (shared initialization) rather than `chat.rs` to avoid duplication across `chat` and `ask` subcommands
- Returns `Option` — `None` when config is missing (preserves CLI-only behavior)
- Accepts `&reqwest::Client` to enable connection pool reuse across agent switches
- URL scheme validation provides defense-in-depth against misconfiguration

### Fix 2: Compile-time embedded skill seeding

**Created `crates/mika-agent/src/bundled_skills.rs`:**

Uses `include_str!` to embed all 5 skill templates at compile time, with a declarative macro for concise definitions:

```rust
macro_rules! skill {
    ($name:expr, [ $( $entry:tt ),+ $(,)? ]) => {
        BundledSkill {
            name: $name,
            files: &[ $( skill!(@file $entry), )+ ],
        }
    };
    (@file ($path:expr => $template:expr, +x)) => {
        SkillFile { path: $path, content: include_str!($template), executable: true }
    };
    (@file ($path:expr => $template:expr)) => {
        SkillFile { path: $path, content: include_str!($template), executable: false }
    };
}
```

**Seeding logic (`seed_bundled_skills`):**
- For each bundled skill, checks if directory already exists — skips if so (never overwrites user customizations)
- Creates directory, writes all files, sets `+x` (mode `0o700`) on handler scripts
- On partial failure: warns, removes partial directory, continues to next skill
- Idempotent — safe to call on every startup

**Integrated into all startup paths:**
- `crates/mika-cli/src/init.rs:init_base_for_agent` — after `seed_core_memory_if_empty`
- `crates/mika-agent/src/server/mod.rs` — after `seed_core_memory_if_empty`
- `crates/mika-cli/src/commands/agents.rs` — after `bootstrap_agent` in the `create` command

**Skills embedded:**
- `tmux` — 9 files (skill.toml, system_prompt.md, tools.json, 6 handler scripts)
- `shell-exec` — 4 files
- `web-search` — 4 files
- `file-reader` — 3 files
- `calendar` — 3 files

### Additional fixes from code review

8 code review findings were resolved alongside the two main fixes:

| Finding | Fix |
|---------|-----|
| Hardcoded "telegram" strings in polling | Centralized into `POLLED_CHANNELS` constant in `tui/app.rs` |
| `reqwest::Client` created per agent switch | Shared client via parameter, reusing connection pool |
| `mika ask` missing message_sender/embedding | Wired both into `ask.rs` via `init::make_message_sender` |
| Misleading "Message delivered (CLI)" text | Changed to "Message logged locally (no outbound sender configured)" |
| No URL scheme validation | Added `http`/`https` check in both CLI and server paths |
| Raw URL in log output (credential leak risk) | Removed URL from warning messages |
| Handler permissions too broad (0o755) | Tightened to 0o700 (owner-only) |
| 170 lines of repetitive skill definitions | Replaced with declarative macro (~50 lines) |

## Prevention Strategies

### 1. Feature parity checklist for multi-path systems

When a feature works in one path (e.g., server), verify it works in all paths (CLI, ask subcommand). Create a checklist:
- [ ] Server path (`mika-server`)
- [ ] CLI interactive path (`mika chat`)
- [ ] CLI one-shot path (`mika ask`)
- [ ] Agent creation path (`mika agents create`)

### 2. Centralize shared initialization

Keep initialization logic in a single module (`init.rs`) rather than duplicating across command modules. The `make_message_sender` pattern — a shared function that returns `Option` based on config — prevents divergence.

### 3. Startup seeding pattern

For resources that must exist at runtime (skills, core memory, etc.), use a consistent pattern:
1. Check if resource exists
2. Skip if already present (never overwrite)
3. Seed from compiled-in defaults
4. Call from all startup paths

### 4. Honest error messages

Never report success when an operation was a no-op. "Message delivered (CLI)" when no message was sent is actively misleading. Use messages like "Message logged locally (no outbound sender configured)" that accurately describe what happened.

### 5. Defense-in-depth for configuration

Validate configuration values beyond just presence:
- URL scheme validation (http/https only)
- Credential redaction in logs
- File permission hardening (0o700 for executable scripts)

### 6. Code review patterns to watch for

- `None` passed where a real value is expected in production
- Hardcoded strings that should be constants
- Resources created per-operation that should be shared (HTTP clients, DB connections)
- Missing feature parity across similar code paths

## Verification

### CLI-to-Telegram
1. Set `MIKA_ROUTING_URL` and `MIKA_INTERNAL_TOKEN` in environment
2. Ensure `chat_id` exists in `customer_config` DB table
3. Run `cargo run --bin mika`, ask "say hi on telegram"
4. Verify message appears in Telegram (or gateway logs show POST to `/send`)
5. Without env vars, verify graceful degradation (no crash, honest "logged locally" message)

### Bundled skills
1. Remove `~/.mika/agents/main/skills/tmux/` if it exists
2. Run `cargo run --bin mika`
3. Ask "list my tmux sessions" — should invoke `tmux_list_sessions` tool
4. Verify all 5 skills appear in `~/.mika/agents/main/skills/`
5. Verify existing custom skills are not overwritten on restart

### Tests
All 432 tests pass:
- 292 mika-agent (includes 4 new bundled_skills tests)
- 45 mika-cli
- 79 mika-common
- 16 mika-gateway

## Related Documentation

- [Multi-Channel TUI Visibility and Polling](../ui-bugs/multi-channel-tui-visibility-and-polling.md) — Cross-channel DB polling for Telegram messages in CLI TUI
- [Telegram Gateway Architecture](../architecture-decisions/telegram-gateway-architecture.md) — Gateway design for routing messages between channels
- [Filesystem Skill Registry](../architecture/filesystem-skill-registry.md) — How skills are loaded from disk
- [Agent Skill Hallucination](../runtime-errors/agent-skill-hallucination-fix.md) — Prior fix for Claude hallucinating skill names
- [Phase 2 Axum Server](../architecture/phase-2-axum-http-server.md) — Server-side message sender wiring (worked correctly)
