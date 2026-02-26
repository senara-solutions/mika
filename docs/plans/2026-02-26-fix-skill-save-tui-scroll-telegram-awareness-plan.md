---
title: "fix: Skill save hallucination, TUI scroll cutoff, and agent Telegram awareness"
type: fix
status: completed
date: 2026-02-26
---

# fix: Skill save hallucination, TUI scroll cutoff, and agent Telegram awareness

## Overview

Three bugs discovered when creating a new agent (`claude-asked-relay`) and asking it to create a skill:

1. **Agent hallucinates skill creation** -- says "Skill saved" but no files are written (no tool exists)
2. **TUI doesn't auto-scroll** -- last message cut off at bottom, resize doesn't help
3. **Agent denies Telegram exists** -- says "I don't have Telegram configured" even when it works

All three share a root cause pattern: the agent's runtime capabilities don't match what it believes it can do, and the TUI has a rendering bug that hides the evidence.

## Bug 1: Agent Cannot Create Skills

### Problem Statement

The agent has no tool to create skill files. When asked to create a skill, Claude hallucinates success because nothing in the system prompt clarifies what tools are actually available for skill management. The registered tools (`default_tools()` in `crates/mika-agent/src/tools/mod.rs:181-191`) are: `update_core_memory`, `store_fact`, `search_memory`, `update_fact`, `create_reminder`, `list_reminders`, `cancel_reminder`, `send_message`. None write to the filesystem.

The CLI has `mika skills create <name>` (`crates/mika-cli/src/commands/skills.rs:125`) but no equivalent agent tool.

DB evidence (conversations table):
```
6|assistant|Skill saved. Here's a summary of what I've stored and how I'll behave...
8|assistant|I didn't actually create it anywhere — I have no "skill storage" system...
```

### Proposed Solution

Add a `create_skill` builtin tool that creates a minimal skill scaffold (skill.toml + system_prompt.md). **No tools.json or executable handlers** -- those require manual setup for security reasons.

### Implementation

#### Phase 1: Add `create_skill` tool

**`crates/mika-agent/src/tools/create_skill.rs`** (new file)

```rust
// Tool: create_skill
// Input: { name: string, description: string, keywords: string[], system_prompt: string }
// Creates: {home_dir}/skills/{name}/skill.toml + system_prompt.md
// Validates: name (alphanumeric + hyphens, no path traversal), no overwrite
// Returns: "Created skill '{name}' at {path}. Available after restart."
```

Key design decisions:
- **No executable handlers** -- only `skill.toml` + `system_prompt.md` (security boundary)
- **No hot-reload** -- `SkillRegistry` is `Arc<SkillRegistry>`, built once at startup. Tool returns "available after restart/next session"
- **Name validation**: alphanumeric + hyphens only, max 50 chars, reject path traversal (`..`, `/`)
- **No overwrite**: return error if skill directory already exists
- **Input validation**: all string inputs checked for empty + `MAX_INPUT_LEN` (10,000 chars)

**`crates/mika-agent/src/tools/mod.rs`**

Register the new tool in `default_tools()`:
```rust
registry.register(Box::new(create_skill::CreateSkillTool));
```

#### Phase 2: Tests

- Unit: valid name creates correct directory structure (skill.toml parses, system_prompt.md exists)
- Unit: duplicate name returns error
- Unit: invalid names rejected (empty, `../etc`, special chars, too long)
- Unit: created skill is loadable by `scan_skills_dir()`
- Unit: input validation (empty description, oversized content)

### Files to modify

- `crates/mika-agent/src/tools/create_skill.rs` -- **NEW** tool implementation
- `crates/mika-agent/src/tools/mod.rs` -- register tool, add `mod create_skill`

---

## Bug 2: TUI Doesn't Auto-Scroll to Latest Message

### Problem Statement

At `crates/mika-cli/src/tui/ui.rs:190`, `total_lines = lines.len()` counts **unwrapped** `Line` items. But `Paragraph::new(lines).scroll((scroll_u16, 0)).wrap(Wrap { trim: false })` at lines 198-200 wraps long lines at render time. A single `Line` wider than the viewport wraps into multiple visual rows, but `total_lines` still counts it as 1. Result: `max_scroll` is underestimated, `effective_scroll` is too small, and the bottom of the conversation is invisible.

Resize doesn't fix it because `AppEvent::Resize` only sets `needs_redraw = true` (line 234 of `chat.rs`) without recalculating anything -- the same wrong `total_lines` is used.

### Proposed Solution

Replace `lines.len()` with a wrapped-line-count calculation that accounts for line wrapping at the viewport width. Use `Line::width()` from ratatui (available in 0.29) to measure each line's character width, then compute how many visual rows it occupies.

### Implementation

**`crates/mika-cli/src/tui/ui.rs`** -- `draw_messages` function, lines 188-196:

Replace:
```rust
let total_lines = lines.len();
```

With:
```rust
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

This correctly computes the number of visual rows after wrapping. `Line::width()` returns the sum of character widths across all spans. Empty lines count as 1 row. Lines wider than the viewport are divided by viewport width (ceiling division).

### Tests

- Unit: `total_lines` calculation with lines shorter than viewport (no wrapping)
- Unit: `total_lines` calculation with lines longer than viewport (wrapping to 2+ rows)
- Unit: empty lines count as 1 row
- Unit: zero viewport width doesn't panic (fallback to `lines.len()`)

### Files to modify

- `crates/mika-cli/src/tui/ui.rs` -- fix scroll calculation in `draw_messages`

---

## Bug 3: Agent Doesn't Know About Telegram

### Problem Statement

The system prompt (`crates/mika-agent/src/prompt.rs:124`, `build_system_prompt()`) has zero awareness of integrations, channels, or Telegram. The `PromptContext` struct (line 55) contains soul, identity, core_memory, time, onboarding, and global_home -- but not `channel_type`.

`channel_type` flows through the system:
1. Gateway sets `MessageRequest.channel = "telegram"`
2. Server handler passes it to `AgentParams.channel_type`
3. **Only used for DB tagging** (`save_message()`) -- never injected into prompt

`customer_config.chat_id` is used by `GatewayMessageSender` for outbound delivery but never mentioned in the prompt.

### Proposed Solution

Add `channel_type: Option<&str>` to `PromptContext` and inject a `## Communication Channel` section into the system prompt when present. Additionally, query `customer_config` for `chat_id` to determine active integrations.

### Implementation

**`crates/mika-agent/src/prompt.rs`**

1. Add field to `PromptContext`:
```rust
pub struct PromptContext<'a> {
    // ... existing fields ...
    /// The channel this message arrived on (e.g., "telegram", "cli").
    /// When `None`, the channel section is omitted (team agents, tests).
    pub channel_type: Option<&'a str>,
    /// Whether Telegram integration is configured (chat_id exists in customer_config).
    pub telegram_configured: bool,
}
```

2. Add `write_channel_section()`:
```rust
fn write_channel_section(prompt: &mut String, channel_type: Option<&str>, telegram_configured: bool) {
    if channel_type.is_none() && !telegram_configured {
        return;
    }
    prompt.push_str("## Communication Channel\n");
    if let Some(ch) = channel_type {
        writeln!(prompt, "This conversation is happening via: {ch}").unwrap();
    }
    if telegram_configured {
        prompt.push_str("Telegram integration is active. You can reach the user via Telegram using send_message.\n");
    }
    prompt.push('\n');
}
```

3. Call it in `build_system_prompt()` after the time section.

**`crates/mika-agent/src/agent.rs`** -- `run_agent_inner()`

Query `customer_config` for `chat_id` and pass to prompt context:
```rust
let chat_id = db.get_customer_config("chat_id").await?;
let prompt_ctx = prompt::PromptContext {
    // ... existing fields ...
    channel_type: Some(params.channel_type),
    telegram_configured: chat_id.is_some(),
};
```

**`crates/mika-agent/src/agent.rs`** -- `run_team_agent_inner()` and `run_silent_inner()`

Team agents: `channel_type: None, telegram_configured: false` (teams are internal).
Silent agent: also add `channel_type` and `telegram_configured` to `SilentPromptContext` so the agent knows delivery will happen via Telegram.

### Tests

- Unit: `build_system_prompt` with `channel_type = Some("telegram")` + `telegram_configured = true` includes channel section
- Unit: `build_system_prompt` with `channel_type = Some("cli")` + `telegram_configured = false` shows CLI context
- Unit: `build_system_prompt` with `channel_type = None` + `telegram_configured = false` omits section
- Unit: `build_silent_prompt` with Telegram configured mentions delivery channel

### Files to modify

- `crates/mika-agent/src/prompt.rs` -- add `channel_type` + `telegram_configured` to `PromptContext`, add `write_channel_section()`, update `build_system_prompt()`, update `SilentPromptContext` + `build_silent_prompt()`
- `crates/mika-agent/src/agent.rs` -- pass `channel_type` and `telegram_configured` in all `PromptContext` instantiation sites (3 call sites: `run_agent_inner`, `run_team_agent_inner`, `run_silent_inner`)

---

## Acceptance Criteria

### Bug 1: Skill Creation
- [x] Agent can create a skill via `create_skill` tool with name, description, keywords, and system_prompt
- [x] Created skill has valid `skill.toml` loadable by `scan_skills_dir()`
- [x] Tool returns clear message that skill is available after restart
- [x] Invalid names (path traversal, empty, special chars) return errors
- [x] Duplicate skill names return errors
- [x] No executable handler scripts created (security)

### Bug 2: TUI Scroll
- [x] Long wrapped messages are fully visible when auto-scrolled to bottom
- [x] Scroll calculation accounts for lines wider than viewport
- [x] Terminal resize correctly redraws with proper scroll position
- [x] Progressive reveal still scrolls correctly during animation
- [x] Manual scroll up/down still works correctly

### Bug 3: Telegram Awareness
- [x] Agent in Telegram mode knows "This conversation is happening via: telegram"
- [x] Agent in CLI mode knows "This conversation is happening via: cli"
- [x] Agent knows when Telegram integration is configured
- [x] Silent mode agent knows delivery channel
- [x] Team agents omit channel section (internal context)
- [x] All existing prompt tests still pass

## References

- `crates/mika-agent/src/tools/mod.rs:181-191` -- current tool registry
- `crates/mika-cli/src/commands/skills.rs:125-218` -- CLI `create_skill` (reference implementation)
- `crates/mika-cli/src/tui/ui.rs:188-201` -- scroll calculation bug
- `crates/mika-agent/src/prompt.rs:55-65` -- `PromptContext` struct
- `crates/mika-agent/src/prompt.rs:124-188` -- `build_system_prompt()`
- `crates/mika-agent/src/agent.rs:134-327` -- `run_agent_inner()` with `channel_type`
- `docs/solutions/architecture-decisions/filesystem-skill-registry-implementation.md` -- skill system architecture
- `todos/204-complete-p2-scroll-offset-u16-truncation.md` -- related scroll bug (u16 truncation, already addressed)
