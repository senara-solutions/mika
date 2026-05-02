---
title: Sibling skill tool-name collision makes dispatch unreachable
date: 2026-05-02
category: logic-errors
module: skills
problem_type: logic_error
component: tooling
symptoms:
  - "Skill 'dev-groom' is not a valid skill. Valid values: [\"dev-pilot\"]"
  - "Handler crash exit code 1 on run_claude_pilot(skill=\"dev-groom\")"
  - "Task blocked after claude-pilot dispatch failure with invalid skill error"
root_cause: config_error
resolution_type: config_change
severity: high
tags:
  - skills
  - tool-registration
  - dispatch
  - dev-pilot
  - dev-groom
  - tool-name-collision
  - run-claude-pilot
---

# Sibling skill tool-name collision makes dispatch unreachable

## Problem

Two bundled skills (dev-pilot and dev-groom) registered the same tool name `run_claude_pilot` with conflicting `skill` enum schemas (`["dev-pilot"]` vs `["dev-groom"]`). The engine's tool-registration dedup (`seen.insert()` in `inject_skills_and_resolve_tools()`) collapsed the duplicate — only dev-pilot's schema survived. When mika-dev called `run_claude_pilot(skill="dev-groom")`, the surviving schema rejected the argument, the handler exited 1, and the dispatch task was marked `blocked`. dev-groom was functionally unreachable despite correct keyword activation.

## Symptoms

- `Skill 'dev-groom' is not a valid skill. Valid values: ["dev-pilot"]` in handler stderr
- Handler crash exit code 1 on any `run_claude_pilot(skill="dev-groom", ...)` call
- Task transitions to `blocked` with note: `claude-pilot dispatch failed: invalid skill 'dev-groom'`
- `groom <repo>#<n>` prompts to mika-dev trigger dev-groom's keywords correctly but the tool call always fails at schema validation

## What Didn't Work

- The pre-#932 architecture assumed that since dev-pilot and dev-groom are never keyword-matched simultaneously, only one `run_claude_pilot` registration would be active per turn. This was incorrect — the engine registers all tools from all loaded skills, not just the keyword-activated one. The dedup layer collapses duplicates globally, not per-activation.

## Solution

Consolidated `run_claude_pilot` into a single host skill (dev-pilot) with a union-enum `skill` parameter. Three coordinated changes:

1. **dev-pilot/tools.json** — widened `skill.enum` from `["dev-pilot"]` to `["dev-pilot", "dev-groom"]`. Updated tool and parameter descriptions to reflect dual-purpose dispatch.

2. **_shared/dispatch-lib.sh** — added a `case` switch that derives the entry command from `$SKILL`:
   ```sh
   case "$SKILL" in
     dev-pilot)  ENTRY_COMMAND="/mika" ;;
     dev-groom)  ENTRY_COMMAND="/mika-groom-ticket" ;;
     *) echo "Unknown skill: $SKILL" >&2; exit 1 ;;
   esac
   ```
   The `dispatch_claude_pilot` function no longer takes positional arguments — entry command is derived, not passed.

3. **dev-groom became prompt-only** — deleted `tools.json` and `handlers/run.sh`. dev-groom retains `skill.toml` (keywords intact for activation) and `system_prompt.md` (grooming instructions for the dispatched session). No tool registration, no handler.

4. **self-dev/system_prompt.md** — added a Grooming Dispatch section (Steps G1–G4) teaching mika-dev when to call `run_claude_pilot(skill="dev-groom", ...)` for grooming work. Also added `skill: "dev-pilot"` explicitly to the existing Generic Workflow and Ready-Label Dispatch JSON examples.

## Why This Works

The root cause was two independent JSON Schema definitions claiming the same tool name with mutually exclusive enum values. The engine's first-wins dedup silently dropped the second registration. By consolidating to a single registration with a union enum, both skill values are valid in a single schema. The lib-side case switch replaces the per-skill handler pattern — no duplicate registration, no collision.

The host/sibling pattern generalizes: only one skill (the host) owns the tool registration. Siblings are prompt-only — they provide context and keywords but no tools. The mapping between skill values and entry commands lives in the shared lib, not in per-skill handlers.

## Prevention

- **Never register the same tool name from multiple skills.** The engine's dedup is first-wins with no warning. Two skills with the same tool name will silently collide.
- **Use the host/sibling pattern** for skills that share a dispatch mechanism. One host skill owns the tool with a union enum; siblings are prompt-only.
- **Sentinel comment in dispatch-lib.sh** documents the three-step contract for adding a sibling: (1) add a case arm, (2) widen the host's enum, (3) update self-dev's system prompt. Threshold: if N>5 siblings, escalate to skill-scoped tool registries (Option C from mika#932).
- **Post-deploy cleanup required until mika#923 ships:** `rm -rf ~/.mika/agents/<agent>/skills/dev-groom/{tools.json,handlers}` — stale files from prior installs can re-introduce the collision.

## Related Issues

- [mika#932](https://github.com/senara-solutions/mika/issues/932) — this fix
- [mika#893](https://github.com/senara-solutions/mika/issues/893) — original consolidation that introduced `_shared/dispatch-lib.sh`
- [mika#923](https://github.com/senara-solutions/mika/issues/923) — `mika skills update` doesn't propagate `_shared/` or clean up stale files
- `docs/solutions/logic-errors/builtin-skill-tool-name-shadowing.md` — related: builtin-vs-skill variant of the same dedup collision class
- `docs/solutions/best-practices/shared-dispatch-library-for-claude-pilot-skills-2026-04-29.md` — updated to reflect the host/sibling pattern
