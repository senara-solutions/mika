---
title: Required tools gate filters unavailable tools at enforcement time
category: prompt-engineering
date: 2026-04-11
tags: [agent-loop, required-tools, enforcement, resilience, llm-cost]
refs: ["#516", "#517", "#270"]
---

# Required Tools Availability Filter

**Issues:** #516, #517
**Category:** prompt-engineering, agent-core
**Date:** 2026-04-11

## Problem

The required_tools enforcement gate (see `required-tools-enforcement-gate.md`) retried when a
skill's `[constraints] required_tools` referenced tools not in the current tool registry. The
agent can't call tools that don't exist, so the retry is always wasteful — consuming an extra
LLM API call per enforcement failure with no possibility of success.

**Observed in production:** mika-qa's `qa-review` skill declared `run_shell` in `required_tools`,
but `run_shell` is not registered as a tool in the agent's tool set. The gate retried once
(by design, to prevent infinite loops), but the retry was guaranteed to fail. Cost: 5 extra
LLM calls (9 total vs expected 4) on the mika-skills#124 PR review.

### Root cause

The root cause was **skill config** — `run_shell` should not have been in `required_tools`.
However, the engine should be resilient to stale or incorrect config rather than blindly
retrying for impossible tools.

### Why the existing validation didn't catch it

`validate_skill()` already warns at scan time when `required_tools` references names not in the
skill's own `tools.json`. But it intentionally allows unknown names because builtins and MCP
tools aren't in `tools.json` — so the warning is advisory. The full tool universe (builtins +
all skill tools + MCP) is only known inside `run_loop()`.

## Solution

### `filter_available_required_tools()`

Added a runtime filter between `collect_required_tools()` (which gathers from skill manifests)
and the enforcement gate (which rejects + re-prompts). The filter checks each required tool
name against the three tool sources available at runtime:

1. **ToolRegistry** — builtin tools (`tools.get(name)`)
2. **skill_tool_map** — skill-defined tools (`HashMap::contains_key`)
3. **McpManager** — MCP tools (`is_mcp_tool(name)`)

Tools not found in any source are excluded with a `warn!` log so operators can fix the config.

```rust
fn filter_available_required_tools(
    required: &HashSet<String>,
    tools: &ToolRegistry,
    skill_tool_map: &HashMap<String, &ResolvedSkillTool>,
    mcp_manager: Option<&McpManager>,
) -> HashSet<String> {
    required
        .iter()
        .filter(|t| {
            let available = tools.get(t).is_some()
                || skill_tool_map.contains_key(t.as_str())
                || mcp_manager.is_some_and(|m| m.is_mcp_tool(t));
            if !available {
                warn!(tool = %t, "required_tools references unavailable tool");
            }
            available
        })
        .cloned()
        .collect()
}
```

Called once at the top of `run_loop()`, producing `effective_required_tools` used by the gate.

### Design decisions

1. **Filter at runtime, not scan time.** The full tool set is only known in `run_loop()`.
   `validate_skill()` can't do this job because it runs per-skill and doesn't see builtins/MCP.

2. **Fail-open, not fail-closed.** Unavailable tools are silently excluded from enforcement
   (with a warning). The alternative — failing the entire turn — would be too aggressive for
   a config mistake.

3. **Mirrors the dispatch chain.** The three-source check matches exactly the routing order
   in `execute_tool()`: builtins → skill tools → MCP. This ensures the filter is authoritative.

4. **One-time cost per turn.** Runs once at loop entry, not per step. Performance is
   negligible (~sub-microsecond for typical 0-3 required tools).

## Prevention

- **Skill authors:** Only list tools in `required_tools` that the skill actually needs and
  that exist in the agent's tool set. Use `mika skills validate` to catch mismatches early.
- **Engine:** The runtime filter makes the gate resilient to config drift. No operational
  action needed beyond fixing the warning logs.

## Related

- `required-tools-enforcement-gate.md` — Original gate design (#270)
- `docs/adr/008-github-identity-separation.md` — Related identity issue (#515, same umbrella)
