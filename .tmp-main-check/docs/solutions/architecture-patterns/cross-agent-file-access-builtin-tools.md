---
title: Cross-agent file access via builtin file tools
category: architecture-patterns
date: 2026-03-26
severity: high
tags: [tools, file-access, security, prompt-engineering, orchestrator]
related_issues: [284]
---

# Cross-agent file access via builtin file tools

## Problem

The orchestrator agent used `run_shell cat` and `run_shell cat >` heredocs to read/write other agents' config files instead of builtin file tools. This bypassed audit logging, overwrite confirmation, and path validation. Observed in trace `55be39bc`: 7 LLM calls, 178K input tokens, 0% cache efficiency for a task that should take 2-3 steps.

Two root causes:
1. **Prompt weakness:** Shell-exec prompt said "prefer" builtins but didn't prohibit shell file I/O
2. **Tool limitation:** `read_agent_file`/`write_agent_file`/`list_agent_files` were scoped to the current agent's home directory via `validate_and_resolve_path`. Orchestrators at `~/.mika/agents/mika/` couldn't access sibling agent directories (e.g., `~/.mika/agents/chase-hughes/`) because path traversal was blocked.

## Root Cause

The file tools used `ctx.home_dir` (per-agent home) as the only base directory. Cross-agent access required path traversal (`../other-agent/`) which was correctly blocked by security validation. No mechanism existed for legitimate cross-agent file access through builtins, forcing the agent to fall back to `run_shell`.

## Solution

### Part A: Cross-agent file access

Added `global_home_dir: Option<&'a Path>` to `ToolContext` (set from `AgentParams.global_home_dir` in conversation mode, `None` for silent/team/delegate modes).

Created `resolve_agent_home(agent_param, ctx)` shared helper in `tools/mod.rs`:
1. If `agent` param is omitted/empty, returns `ctx.home_dir` (no behavioral change)
2. If self-reference (agent name matches current agent), short-circuits to `ctx.home_dir`
3. Requires `global_home_dir` is `Some` (blocks team/delegate agents)
4. `is_orchestrator()` guard (only orchestrators can access other agents' files)
5. `agent::agent_exists()` validation (also serves as path traversal protection)
6. Returns `agent::agent_dir(global_home_dir, agent_name)` as the resolved base

Added optional `agent` parameter to all three file tools' `input_schema` and `execute()`.

For `write_agent_file`, confirmation messages include the agent name for cross-agent writes: `"Call again with confirm: true and agent: \"work\" to overwrite."` This prevents misdirected retries.

### Part B: Prompt hardening

Used the four-part anti-hallucination formula (NEVER prohibition + BAD example + GOOD example + tool suggestion):

- **`prompt.rs`**: Added explicit prohibition in Tool Usage section
- **`shell-exec/system_prompt.md`**: Changed "prefer builtins" to "NEVER use shell commands for agent home directory files" with concrete examples
- **`self-knowledge/system_prompt.md`**: Added cross-agent config change rules (use `agent` param, only read what you need, never use `run_shell`)

## Key Design Decisions

1. **`ToolContext` extension vs tool struct state**: Chose `ToolContext` because all three file tools are unit structs and the pattern is consistent with how `home_dir` is already carried.
2. **`None` as access control**: `global_home_dir: None` blocks cross-agent access for team agents, delegates, and silent mode — simple, explicit, no boolean flags needed.
3. **`agent_exists()` as path traversal protection**: The function checks for `config.toml` in the resolved directory, which means path traversal attempts (e.g., `../../etc`) return false. This is defense-in-depth on top of the allowlist pattern.
4. **`is_orchestrator()` called with `global_home_dir`**: The function needs the global mika home (not per-agent home) to find teams. Using `ctx.home_dir` would have silently failed.

## Prevention

- When adding new tools that need cross-scope access, follow the `resolve_agent_home` pattern: optional parameter with guard checks, not path traversal
- Use the four-part prompt formula (NEVER/BAD/GOOD/tool) for critical behavioral guidance — "prefer" is too weak for LLM instruction following
- When tools hint at alternatives (like `write_agent_file` suggesting `run_shell`), scope the hint precisely to avoid reinforcing bad patterns

## Related

- `docs/solutions/integration-issues/shell-exec-config-editing-quality.md` — Prior prompt-only fix for config editing quality
- `docs/solutions/integration-issues/write-file-tool-overwrite-confirmation-flow.md` — Overwrite confirmation pattern
- `docs/solutions/logic-errors/tool-path-reporting-misbehavior.md` — Always report resolved absolute paths
- `docs/solutions/prompt-engineering/grounding-rule-downstream-state-hallucination.md` — Four-part prompt formula
