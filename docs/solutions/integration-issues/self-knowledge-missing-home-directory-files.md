---
title: Agent answers self-knowledge questions from incomplete knowledge, missing home directory files
date: 2026-03-10
severity: medium
modules:
  - crates/mika-agent/src/prompt.rs
  - crates/mika-agent/templates/skills/self-knowledge/system_prompt.md
tags:
  - self-knowledge
  - prompt-engineering
  - home-directory
  - agent-introspection
  - hallucination-prevention
symptoms:
  - Agent answered questions about its own internals using only LLM training knowledge
  - Agent said personality adjustments happen "only via core memory" without checking soul.md
  - Self-knowledge skill only covered get_documentation topics, not home directory files
related:
  - docs/solutions/integration-issues/mcp-self-knowledge-command-hallucination.md
  - docs/solutions/logic-errors/agent-creates-duplicates-after-compaction.md
  - docs/solutions/logic-errors/agent-skips-multi-action-and-reasking-answered-questions.md
issue: https://github.com/senara-solutions/mika/issues/95
---

# Agent answers self-knowledge questions from incomplete knowledge

## Problem

When asked "what gets adjusted when I change your personality?", the agent answered "only core memory" without checking `soul.md` or `identity.toml`. The agent has tools to inspect its own state (`list_home_files`, `read_home_file`, `get_documentation`) but nothing told it to use them for questions about its configuration files.

The `self-knowledge` skill (`always_on = true`) only instructed the agent to use `get_documentation` for system documentation topics (architecture, CLI, API, etc.). It said nothing about checking home directory files. The base system prompt had "Never fabricate information" but no specific self-knowledge file-checking instruction.

## Root Cause

Gap in prompt coverage. The self-knowledge skill covered system documentation but not agent-specific configuration files. The agent's home directory contains files that define how it works (`soul.md`, `identity.toml`, `mcp.json`, `skills/`), but no prompt instruction told the agent to inspect them before answering questions about itself.

## Solution

Two-layer fix following the "layered reinforcement" pattern established by the MCP hallucination fix.

### Layer 1: Base system prompt (broad safety net)

Added a single instruction to `write_instructions_section()` in `prompt.rs`:

```
When asked about your own configuration, setup files, or how specific parts of you work,
check your own files (list_home_files, read_home_file) and documentation (get_documentation)
before answering. Never guess about your own internals.
```

### Layer 2: Self-knowledge skill prompt (detailed guidance)

Expanded `system_prompt.md` with:
- Key files list: `soul.md`, `identity.toml`, `mcp.json`, `skills/`
- 4 rules for handling home directory questions
- Rule 3 carve-out: do NOT re-read `soul.md` for personality questions (already in system prompt via `write_soul_section`)
- Bad/good examples using CRITICAL/NEVER pattern

## Key Design Decisions

1. **Layered reinforcement over single-point instruction.** The base prompt (~30 tokens) provides a broad catch-all. The skill prompt (~200 tokens) provides detailed behavioral guidance with examples. This ensures coverage even if the skill is disabled.

2. **Scoped instruction with explicit exception.** Rule 3 prevents unnecessary tool calls by telling the agent that `soul.md` content is already in its system prompt. Only file-content questions ("what does your soul.md say?") need `read_home_file`.

3. **Strong language pattern.** CRITICAL/NEVER markers with concrete bad/good examples, proven effective from the MCP hallucination fix where polite "never invent" instructions were treated as soft suggestions.

## Prevention

- When adding new agent configuration files to the home directory, update the self-knowledge skill prompt's key files list.
- Follow the "proactive state checking" convention: any new tool that reads agent state should be referenced in the appropriate prompt instruction.
- Use the strong instruction pattern (CRITICAL/NEVER + bad/good examples) for any behavior the LLM must not skip — passive suggestions get ignored under complex reasoning.

## Related

- [MCP self-knowledge command hallucination](./mcp-self-knowledge-command-hallucination.md) — established the strong instruction pattern
- [Agent creates duplicates after compaction](../logic-errors/agent-creates-duplicates-after-compaction.md) — established "proactive state checking" convention
- [Agent skips multi-action and re-asks answered questions](../logic-errors/agent-skips-multi-action-and-reasking-answered-questions.md) — active mandatory language > passive suggestions
