---
title: "System prompt should instruct agent to check own files/docs before answering self-knowledge questions"
type: feat
status: completed
date: 2026-03-10
---

# feat: Instruct agent to check own files/docs before answering self-knowledge questions

## Overview

When asked "how do I work?" style questions about its own internals, the agent answers from incomplete knowledge instead of checking its own files first. The agent has tools to inspect its state (`list_agent_files`, `read_agent_file`, `get_documentation`) but nothing tells it to use them for self-knowledge questions beyond the narrow `get_documentation` topic list.

The fix is to expand the self-knowledge skill's `system_prompt.md` to also cover home directory files and add a targeted instruction in the base system prompt.

## Problem Statement

The agent incorrectly told a user that personality adjustments only happen via core memory, missing the existence of `soul.md`. Only discovered `soul.md` after being prompted to check its own files. The `self-knowledge` skill is `always_on` but only instructs the agent to use `get_documentation` for system documentation topics — it says nothing about checking home directory files (`soul.md`, `identity.toml`, `mcp.json`, skill configs).

## Proposed Solution

Two complementary changes, following the institutional learning that **layered reinforcement** across base prompt + skill prompt is more effective than either alone (see: `docs/solutions/integration-issues/mcp-self-knowledge-command-hallucination.md`).

### Change 1: Expand self-knowledge skill prompt

**File:** `crates/mika-agent/templates/skills/self-knowledge/system_prompt.md`

Add a new section after the existing `get_documentation` rules covering home directory file inspection. This is the primary change since the self-knowledge skill is `always_on` and already establishes the "check before answering" pattern.

Add guidance for:
- **Configuration questions** (identity, MCP servers, installed skills): Use `list_agent_files` and `read_agent_file`
- **Personality/behavior questions** where the user asks about file contents: Use `read_agent_file("soul.md")`
- **Distinguish** between "what's your personality" (answerable from system prompt context — soul.md is already injected) and "what does your soul.md say" (needs `read_agent_file` for literal content)
- **Key files to mention by name**: `soul.md` (personality), `identity.toml` (name/emoji), `mcp.json` (MCP server config), `skills/` directory (installed skills)
- **Fallback**: If the answer isn't in any file or documentation, say "I don't know" rather than guessing

Follow the strong instruction pattern from the MCP hallucination fix:
1. Explicit prohibition with CRITICAL/NEVER
2. Concrete bad example (guessing about soul.md without checking)
3. Concrete good example (calling read_agent_file first)

### Change 2: Add targeted base prompt instruction

**File:** `crates/mika-agent/src/prompt.rs` (in `write_instructions_section`, around line 337)

Add a single bullet point after the existing "check before creating" instruction:

> When asked about your own configuration, setup files, or how specific parts of you work, check your own files (`list_agent_files`, `read_agent_file`) and documentation (`get_documentation`) before answering. Never guess about your own internals.

This is a lightweight reinforcement — the detailed guidance lives in the skill prompt. The base prompt instruction is broad enough to catch self-knowledge questions even if the skill matcher somehow fails.

**Important scoping:** The instruction should NOT tell the agent to re-read `soul.md` for personality questions — that content is already in the system prompt via `write_soul_section()`. The instruction targets configuration files and setup details that are NOT in the prompt context.

## Technical Considerations

- **Token budget:** The self-knowledge skill prompt addition is ~200 tokens. The base prompt addition is ~30 tokens. Both are modest given they fire on every turn (skill is `always_on`).
- **Tool step budget:** In the worst case, a self-knowledge answer costs 2 tool steps (`list_agent_files` + `read_agent_file`). This is acceptable within the 10-step limit.
- **No code changes beyond prompt text:** Both changes are pure prompt content — no Rust code, no schema changes, no new tools.
- **Silent/team mode:** Silent prompt is built separately (`build_silent_prompt`). Self-knowledge skill prompt is injected via the skill matcher which runs per-user-message. Neither silent mode nor team agents are affected in a harmful way.

## Acceptance Criteria

- [x] Self-knowledge skill `system_prompt.md` includes instructions about `list_agent_files` / `read_agent_file` for configuration questions
- [x] Self-knowledge skill prompt enumerates key home directory files (`soul.md`, `identity.toml`, `mcp.json`, `skills/`)
- [x] Self-knowledge skill prompt includes bad/good examples for the home directory file pattern
- [x] Base system prompt (`prompt.rs`) includes a "check own files before answering about yourself" instruction
- [x] Instruction scoping: does NOT tell agent to re-read soul.md for personality questions (content already in prompt)
- [x] Instruction includes fallback guidance: say "I don't know" if not in files
- [x] Existing tests pass (`cargo test`)
- [x] `prompt_snapshot` test updated if it exists (system prompt snapshot tests) — N/A, no snapshot tests found

## MVP

### `crates/mika-agent/templates/skills/self-knowledge/system_prompt.md`

Add after the existing content (line 19):

```markdown

**Your own files and configuration:**

Beyond system documentation, you also have files in your home directory that define your configuration and identity. When asked about your own setup, configuration, or how specific parts of you work, check these files before answering.

Key files:
- `soul.md` — your personality, communication style, and behavioral boundaries
- `identity.toml` — your name and emoji
- `mcp.json` — MCP server connections and configuration
- `skills/` — installed skill directories (each with `skill.toml` and optional `system_prompt.md`)

**Rules for home directory questions:**
1. When asked about your configuration files, MCP servers, installed skills, or identity settings, use `list_agent_files` and `read_agent_file` to check BEFORE answering.
2. When asked "what does your soul.md say?" or similar file-content questions, use `read_agent_file` — do NOT paraphrase from memory.
3. You do NOT need to re-read `soul.md` for general personality questions — its content is already in your system prompt.
4. If you cannot find the answer in your files or documentation, say so. Never guess about your own internals.

**Bad example (NEVER do this):** Being asked "what gets adjusted when I change your personality?" and answering "only core memory" without checking `soul.md` and `identity.toml`.
**Good example:** Call `list_agent_files` to see what config files exist, then `read_agent_file("soul.md")` to check what personality settings are stored there, and answer accurately.
```

### `crates/mika-agent/src/prompt.rs`

Add after the "Before creating or storing anything" instruction (after line 337):

```rust
prompt.push_str(
    "- When asked about your own configuration, setup files, or how specific parts of you work, \
     check your own files (list_agent_files, read_agent_file) and documentation (get_documentation) \
     before answering. Never guess about your own internals.\n",
);
```

## Sources

- Issue: #95
- Self-knowledge skill prompt: `crates/mika-agent/templates/skills/self-knowledge/system_prompt.md`
- Base system prompt builder: `crates/mika-agent/src/prompt.rs:232-406`
- MCP hallucination fix learning: `docs/solutions/integration-issues/mcp-self-knowledge-command-hallucination.md`
- Prompt-only skill learning: `docs/solutions/integration-issues/adding-prompt-only-bundled-skill.md`
