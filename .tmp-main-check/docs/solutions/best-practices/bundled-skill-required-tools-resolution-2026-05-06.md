---
title: Resolve required_tools against deployed registry before writing skill.toml
module: skills
date: 2026-05-06
problem_type: best_practice
component: tooling
severity: medium
applies_when:
  - Adding a new bundled skill to skills/bundled/
  - Specifying required_tools in a skill manifest
  - Porting tool names from external contexts (Claude Code, IDEs) into mika skill manifests
tags:
  - bundled-skills
  - required-tools
  - skill-manifest
  - tool-registry
---

# Resolve required_tools Against Deployed Registry Before Writing skill.toml

## Context

When creating a new bundled skill, the `[constraints].required_tools` field in `skill.toml` is a pre-flight gate — the engine checks that all listed tools exist in the agent's available tool surface before activating the skill. Tool names in this field must be **mika-internal builtin or skill-provided tool names**, not tool names from other execution contexts.

A common friction point: design tickets and plans often reference tool names from the Claude Code environment (`bash`, `edit`, `write`, `read`) or from generic programming contexts. These are NOT valid entries for `required_tools` — they don't exist in mika's tool registry and will cause silent skill non-activation (the required-tools gate fails without error visibility to the end user).

## Guidance

**Always resolve `required_tools` against the deployed registry before writing `skill.toml`.** The resolution is a two-step pre-flight:

### Step 1: Verify the tool surface supports the skill's needs

```bash
# Check what builtin tools exist
ls crates/mika-agent/src/tools/

# Check what skill-provided tools other bundled skills declare
find skills/bundled/ -name "tools.json" -exec grep -l '"name"' {} \;
```

For a prompt-only skill that needs file write capability:
- `write_agent_file` — writes relative to agent home (`~/.mika/agents/<name>/`)
- `read_agent_file` — reads relative to agent home
- `list_agent_files` — lists agent home contents

These are builtins registered for ALL agents in `default_tools()`.

### Step 2: Grep existing manifests for canonical names

```bash
grep -h "^required_tools" skills/bundled/*/skill.toml | sort -u
```

As of 2026-05-06, the deployed surface shows:
- `gh_read` (builtin handler)
- `qa_pr_view`, `run_gh`, `run_shell`, `build_mika` (skill-provided via tools.json)
- `review_skill` (skill-provided)
- `run_claude_pilot` (skill-provided via dev-pilot)

**Note:** `run_shell` is skill-provided (from qa-review's `tools.json`), NOT a builtin. It only exists for agents that have qa-review loaded. Using it in `required_tools` gates activation on qa-review being present.

### Decision matrix

| Skill needs | Correct tool name | Why |
|-------------|-------------------|-----|
| Write files in agent home | `write_agent_file` | Builtin, all agents |
| Read files in agent home | `read_agent_file` | Builtin, all agents |
| Run shell commands | `run_shell` | Skill-provided (qa-review) — only if that skill is loaded |
| Dispatch Claude Code session | `run_claude_pilot` | Skill-provided (dev-pilot) |
| GitHub CLI operations | `run_gh` | Skill-provided (qa-review) |

## Why This Matters

The `required_tools` gate is enforcement-without-feedback. When a listed tool doesn't exist in the agent's registry, `filter_available_required_tools()` silently removes it. If ALL listed tools are filtered out, the constraint effectively becomes empty (passes trivially). If SOME tools remain but the missing one was the critical gate, the skill activates but the LLM can't call the tool it needs.

Either way, the failure mode is silent: no error log, no startup warning, just a skill that either never fires its gate or fires and then can't execute its instructions.

## Examples

**Wrong (Claude Code tool names):**
```toml
[constraints]
required_tools = ["bash", "edit", "write"]
```

**Right (mika-internal names):**
```toml
[constraints]
required_tools = ["write_agent_file"]
```

## When to Apply

- Every time a new `skill.toml` is created with a `[constraints]` section
- When copying tool references from design documents, tickets, or plans authored outside the mika engine context
- When porting skills between execution environments (community skills may use different tool name conventions)
