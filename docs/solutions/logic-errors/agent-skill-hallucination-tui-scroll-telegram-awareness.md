---
title: "Agent skill hallucination, TUI scroll cutoff, and Telegram unawareness"
date: 2026-02-26
type: fix
severity: high
modules: [mika-agent, mika-cli]
components: [agent-tools, tui-rendering, prompt-context]
tags: [skill-system, tui, ratatui, scroll, telegram, prompt-injection, toml-injection, agent-native]
related_issues: [278, 279, 280, 281]
---

# Agent Skill Hallucination, TUI Scroll Cutoff, and Telegram Unawareness

## Problem Symptom

When creating a new agent (`claude-asked-relay`) and asking it to create a skill:

1. **Agent said "Skill saved"** but no files existed in `.mika/agents/claude-asked-relay/skills/`
2. **TUI didn't scroll** to show the last message — the conversation was cut off at the bottom, and terminal resize didn't help
3. **Agent denied Telegram existed** — said "I don't have Telegram configured" even though the Telegram integration was working

## Investigation Steps

1. **Checked the agent's SQLite database** — conversations table showed the agent first hallucinating success, then admitting it had no skill storage system
2. **Audited `default_tools()` registry** — confirmed no `create_skill` tool existed; only memory, reminder, and messaging tools
3. **Examined TUI scroll calculation** in `ui.rs` — found `lines.len()` counting logical lines, not wrapped visual rows
4. **Traced `channel_type` flow** — confirmed it was passed through `AgentParams` but only used for DB tagging, never injected into the system prompt
5. **Checked `PromptContext`** — no `channel_type` or `telegram_configured` fields existed

## Root Cause Analysis

### Bug 1: No create_skill tool existed

The agent had no tool to create skill files. When asked, Claude hallucinated success because nothing in the system prompt clarified available capabilities. The CLI had `mika skills create` but no equivalent agent tool.

**Pattern**: Capability gap between CLI and agent (write-only view of the skills subsystem).

### Bug 2: Scroll counted logical lines, not visual rows

`Paragraph::scroll()` with `Wrap { trim: false }` operates on *wrapped visual rows*, but the calculation used `lines.len()` which counts unwrapped logical lines. A single long line wrapping to 3 visual rows was counted as 1, causing `max_scroll` to be underestimated.

**Pattern**: Rendering abstraction mismatch — measuring at the wrong level of the stack.

### Bug 3: Agent prompt had no channel awareness

`channel_type` flowed through `AgentParams` from the gateway but was only used for DB tagging in `save_message()`. It was never injected into the system prompt. `customer_config.chat_id` indicated Telegram was configured but was never surfaced to the agent.

**Pattern**: Runtime context not propagated to the LLM's context window.

## Working Solution

### Bug 1: Added create_skill, list_skills, toggle_skill tools

```rust
// crates/mika-agent/src/tools/create_skill.rs
// Uses toml::to_string_pretty() instead of format!() to prevent TOML injection
let manifest = SkillManifest {
    skill: SkillInfo {
        name: name.to_string(),
        description: description.to_string(),
        version: "0.1.0".to_string(),
        always_on,
        timeout_secs: 30,
    },
    triggers: Triggers { keywords },
};
let skill_toml = toml::to_string_pretty(&manifest)?;
```

Key security features:
- Name validation: alphanumeric + hyphens/underscores only, max 50 chars, no path traversal
- TOML serialization via `Serialize` trait (never string interpolation)
- `create_dir()` not `create_dir_all()` (defense-in-depth)
- Symlink guard: canonicalize after creation, verify containment
- No executable handlers (security boundary)
- Keyword limits: max 50 keywords, max 100 chars each

### Bug 2: Calculate wrapped visual row count

```rust
// crates/mika-cli/src/tui/ui.rs
let viewport_width = inner.width as usize;
let total_lines: usize = if viewport_width == 0 {
    lines.len()
} else {
    lines.iter().map(|line| {
        let w = line.width();
        if w == 0 { 1 } else { (w.saturating_sub(1) / viewport_width) + 1 }
    }).sum()
};
```

Formula: `ceil(line_width / viewport_width)` via `(w - 1) / viewport_width + 1`, with `saturating_sub` to prevent underflow on zero-width lines.

### Bug 3: Channel awareness with allowlist validation

```rust
// crates/mika-agent/src/prompt.rs
const VALID_CHANNELS: &[&str] = &["cli", "telegram", "whatsapp", "api"];

fn write_channel_section(prompt: &mut String, channel_type: Option<&str>, telegram_configured: bool) {
    let valid_channel = channel_type.filter(|ch| VALID_CHANNELS.contains(ch));
    if valid_channel.is_none() && !telegram_configured { return; }
    prompt.push_str("## Communication Channel\n");
    if let Some(ch) = valid_channel {
        writeln!(prompt, "This conversation is happening via: {ch}").unwrap();
    }
    if telegram_configured {
        prompt.push_str("Telegram integration is active. You can reach the user via Telegram using send_message.\n");
    }
    prompt.push('\n');
}
```

Security: allowlist prevents prompt injection from compromised gateway. `telegram_configured` derived from trusted DB, not HTTP request.

## Prevention Strategies

1. **Never interpolate user input into structured formats** (TOML, JSON, SQL). Always use typed serialization (serde, prepared statements).
2. **When using ratatui Wrap, measure visual width** — use `Line::width()` and viewport dimensions, not `lines.len()`.
3. **Propagate runtime context to the LLM** — if the system knows something (channel type, integrations), the prompt should reflect it.
4. **Validate external inputs with allowlists** — `channel_type` from gateway must match known values before prompt injection.
5. **Maintain agent-native parity** — if a CLI command exists, there should be a corresponding agent tool (or explicit documentation why not).
6. **Test tools in isolation** — the `create_skill` tests initially failed when run independently because they bypassed `init_sqlite_vec()`. Use established `TestHarness` patterns.

## Files Modified

- `crates/mika-agent/src/tools/create_skill.rs` — **NEW** create_skill tool
- `crates/mika-agent/src/tools/list_skills.rs` — **NEW** list_skills tool
- `crates/mika-agent/src/tools/toggle_skill.rs` — **NEW** toggle_skill tool
- `crates/mika-agent/src/tools/mod.rs` — register 3 new tools
- `crates/mika-agent/src/skills/manifest.rs` — add Serialize derives
- `crates/mika-agent/src/prompt.rs` — channel awareness + create_skill mention
- `crates/mika-agent/src/agent.rs` — pass channel_type and telegram_configured
- `crates/mika-cli/src/tui/ui.rs` — fix scroll calculation

## Cross-References

- [Filesystem Skill Registry Architecture](../architecture-decisions/filesystem-skill-registry-implementation.md) — skill system design and security findings
- [TUI Log Corruption and Empty Agent Replies](../runtime-errors/tui-log-corruption-and-empty-agent-replies.md) — related TUI rendering fixes
- [CLI 21 Findings Resolution](../code-review-workflow/mika-cli-21-findings-parallel-resolution.md) — scroll offset u16 truncation fix
- [Telegram Webhook Gateway Design](../integration-issues/telegram-webhook-gateway-design.md) — channel routing architecture
- [Code Review Security Findings (7aba1ec)](../security-issues/code-review-7aba1ec-shell-injection-memory-safety.md) — path traversal and memory safety patterns

## Test Coverage

- 8 tests for `create_skill` (success, duplicates, invalid names, empty inputs, always-on, TOML roundtrip with quotes, scanner compatibility)
- 3 tests for `list_skills` (empty, shows entries, shows disabled)
- 5 tests for `toggle_skill` (disable, enable, already-enabled, nonexistent, invalid name)
- 5 tests for channel awareness in prompt (telegram, cli, none, silent+telegram, silent-no-telegram)
- **Total: 419 tests pass across all crates**
