---
title: Agent CLI self-knowledge and file-reader skill trigger fixes
date: 2026-02-26
category: logic-errors
components:
  - mika-agent/prompt.rs
  - mika-agent/bundled_skills.rs
  - mika-cli/main.rs
  - templates/skills/file-reader
symptoms:
  - Agent invents non-existent CLI commands (e.g., "mika send --message hi")
  - Agent claims "I don't have filesystem access" despite having file-reader skill
  - File-reader skill triggers on unrelated messages containing "cat"
  - Agent denies real but unlisted CLI commands exist
root_cause: System prompt lacked CLI reference; file-reader skill had narrow keywords, no system_prompt.md, and a false-positive-prone 3-char keyword
severity: medium
resolution_time: ~3 hours
tags:
  - prompt-engineering
  - skill-activation
  - keyword-matching
  - cli-reference
  - agent-hallucination
  - drift-detection
---

# Agent CLI Self-Knowledge and File-Reader Skill Trigger Fixes

## Problem Summary

The Mika agent exhibited two categories of self-knowledge failures:

1. **CLI command hallucination:** The system prompt contained zero information about the CLI interface, so the agent freely invented commands like `mika send --message hi`, `mika list-all-commands`, etc. When users asked "How do I send you a message from the command line?", the agent would fabricate plausible-sounding but non-existent commands.

2. **File-reader skill activation failure:** Asking "What's the content of ~/.bashrc?" would not trigger the file-reader skill because (a) the keyword list was too narrow (`["read file", "show file", "cat", "file contents"]`), (b) the skill lacked a `system_prompt.md` to guide the agent on when to use `read_file`, and (c) the `"cat"` keyword caused false positives on any message containing "cat" as a substring (e.g., "my cat is sick", "category", "concatenate").

## Root Cause Analysis

**Why the agent hallucinated CLI commands:**

The system prompt is built via `write_*_section()` functions in `prompt.rs`. There was no `write_cli_section()` function, so the agent in CLI mode had no reference point for which commands exist. Without constraints, the LLM freely generated plausible command names.

**Why the file-reader skill didn't trigger:**

The skill matcher in `skills/matcher.rs` uses case-insensitive substring matching: `message_lower.contains(&kw)`. The original 4 keywords were too specific to match natural language variations like "What's the content of..." or "Can you open this file?". The missing `system_prompt.md` meant even when keywords did match, the agent lacked guidance on when and how to use the `read_file` tool.

**Why "cat" caused false positives:**

The 3-character keyword `"cat"` matches any message containing that substring. Substring matching has no word-boundary awareness, so "category", "concatenate", "education", and "my cat is sick" all triggered the file-reader skill, polluting the tool context with irrelevant tools.

**Why "Available commands" was misleading:**

The initial fix used "Available commands:" with the instruction "Refer only to the commands listed above." This created a closed-world assumption: the agent would deny that real but unlisted commands (e.g., `mika agents clone`, `mika skills test`) exist, because the prompt told it the list was exhaustive when it was actually curated to save tokens.

## Solution

### Change 1: Add CLI Reference section (channel-gated)

Added `write_cli_section()` to `crates/mika-agent/src/prompt.rs`, called from `build_system_prompt()` only when `channel_type == Some("cli")`:

```rust
// Keep command list in sync with crates/mika-cli/src/cli.rs Commands enum.
fn write_cli_section(prompt: &mut String, channel_type: Option<&str>) {
    if channel_type != Some("cli") {
        return;
    }
    prompt.push_str(
        "## CLI Reference\n\
         The user interacts with you through the `mika` CLI. Common commands:\n\
         - `mika` — interactive chat (default)\n\
         - `mika ask \"message\"` — send a single message, print response\n\
         - `mika status` — health info\n\
         - `mika memory search \"query\"` — search stored facts\n\
         ...\n\
         Never invent CLI commands. These are the most common; other subcommands may exist.\n\n",
    );
}
```

Key design decisions:
- **Channel-gated to "cli" only** — Telegram and API users have no CLI access; showing them CLI commands would be confusing and wasteful.
- **"Common commands" not "Available commands"** — Accurately conveys the list is curated, not exhaustive.
- **Softened restriction** — "These are the most common; other subcommands may exist" allows the agent to acknowledge uncertainty rather than denying real commands.

### Change 2: Fix file-reader skill triggers

**Expanded keywords** in `templates/skills/file-reader/skill.toml`:
```toml
keywords = ["read file", "show file", "file contents", "content of", "what's in", "open file", "view file", "print file"]
```

Key change: **Removed `"cat"`** (false-positive magnet). Added natural language phrases like `"content of"` and `"what's in"` that directly match user phrasing.

**Created `system_prompt.md`** for the file-reader skill:
```markdown
- Use `read_file` when the user asks to see, read, or examine the contents of a file.
- The `path` parameter accepts absolute or relative paths. Expand `~` to the user's home directory.
- For binary files or very large files, warn the user that output may be truncated or unreadable.
```

**Registered in `bundled_skills.rs`** so the file gets seeded on startup alongside the existing skill files.

### Change 3: Add drift-detection test

Added `test_cli_prompt_mentions_top_level_commands` in `crates/mika-cli/src/main.rs` that validates the system prompt contains all top-level CLI command names:

```rust
#[test]
fn test_cli_prompt_mentions_top_level_commands() {
    let prompt = build_system_prompt(&ctx); // with channel_type: Some("cli")
    for cmd in ["ask", "status", "memory", "reminders", "skills", "config", "agents", "teams"] {
        assert!(prompt.contains(cmd), "CLI Reference missing command: {cmd}");
    }
}
```

This acts as a tripwire: adding a new top-level CLI command without updating the prompt will fail the test.

## Investigation Steps

Six code review agents examined the changes in parallel:

1. **Security-Sentinel** — Verified channel-gating prevents prompt injection from untrusted gateway sources. The `VALID_CHANNELS` allowlist at `prompt.rs:113` prevents arbitrary strings from reaching the prompt. No vulnerabilities found.

2. **Architecture-Strategist** — Cross-referenced the prompt against the actual `Commands` enum in `cli.rs`. Found intentional omissions (setup, agents clone/delete, etc.) saving ~200 tokens. Recommended the drift-detection test.

3. **Code-Simplicity-Reviewer** — Flagged `"cat"` as a false-positive magnet and "Available commands" as misleading for a curated list. Confirmed all 3 tests are non-overlapping and necessary.

4. **Pattern-Recognition-Specialist** — Confirmed `write_cli_section` follows the exact same pattern as all other `write_*_section()` functions. File-reader skill now matches the structure of all other bundled skills.

5. **Agent-Native-Reviewer** — Confirmed CLI Reference is correctly channel-specific. Identified pre-existing agent-native gaps (status, config, agent management have no tool equivalents) but these are not regressions.

6. **Learnings-Researcher** — Found relevant prior solutions: Todo #089 (prompt injection delimiters), Todo #215 (skill prompt sanitization), and the bundled skills seeding pattern from the CLI-Telegram messaging fix.

## Prevention Strategies

### 1. Keywords must be multi-word phrases

Substring matching on short keywords causes false positives. **Minimum 2 words per keyword**, unless the single word is unambiguous in context (e.g., "tmux", "calendar").

- BAD: `["cat", "file", "read"]`
- GOOD: `["read file", "show file", "content of", "what's in"]`

### 2. Every keyword-triggered skill must have system_prompt.md

Without it, the agent has no guidance on when to use the skill's tools. The file should explain: when to use it, what parameters to expect, and edge cases.

### 3. Prompt wording must match list completeness

- Exhaustive list: "Available commands:" + "Refer only to these"
- Curated list: "Common commands:" + "Other subcommands may exist"

Mixing exhaustive wording with a curated list creates false negatives (agent denies real commands).

### 4. Drift-detection tests for hardcoded prompt content

Any hardcoded list in the system prompt (commands, tool names, etc.) should have a corresponding test that validates it against the source of truth. The test in `mika-cli/src/main.rs` catches CLI command additions that aren't reflected in the prompt.

### 5. Code review checklist for skill changes

- [ ] Keywords are multi-word and specific (no single common words)
- [ ] `system_prompt.md` exists and accurately describes the tool
- [ ] `bundled_skills.rs` includes all skill files
- [ ] Manual test: skill triggers on natural language, doesn't trigger on unrelated messages

## Related Documentation

- **[Agent Skill Hallucination Fix](agent-skill-hallucination-tui-scroll-telegram-awareness.md)** — Prior fix for agent hallucinating skill capabilities. Added `create_skill`, `list_skills`, `toggle_skill` tools and channel awareness.
- **[Filesystem Skill Registry](../architecture-decisions/filesystem-skill-registry-implementation.md)** — Architecture for skill discovery, keyword matching, and handler execution.
- **[CLI-Telegram Messaging and Skill Seeding](../integration-issues/cli-telegram-messaging-and-skill-seeding.md)** — Bundled skills seeding implementation with `include_str!` macro.
- **Todos:** #297 (cat keyword), #298 (CLI wording), #299 (drift test) — all resolved in commit `bef7b3f`.

## Migration Note

Existing users must delete `~/.mika/agents/main/skills/file-reader/` and restart to pick up the new keywords and `system_prompt.md`. Bundled skill seeding skips directories that already exist (by design, to preserve user customizations).
