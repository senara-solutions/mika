---
title: "fix: Remove stale write_agent_file reference from skill-review tools.json"
type: fix
status: completed
date: 2026-04-08
issue: senara-solutions/mika-skills#99
pr: senara-solutions/mika#485
relocated_from: senara-solutions/mika-skills#102
---

# fix: Remove stale write_agent_file reference from skill-review tools.json

> **Relocation note (2026-04-08):** This plan originally landed on `mika-skills` main via PR #102 alongside a docs-only PR with no source changes — a cross-repo scope drift (see the companion solution doc). The actual implementation was shipped via `senara-solutions/mika#485`. This document has been relocated to the `mika` repository so the plan lives next to the code it describes.

## Overview

Issue mika-skills#99 requested switching `skill-review` from `write_agent_file` to `write_skill_variant` and deleting a bogus `gemini-2.5-flash` variant. Investigation during implementation revealed the scope had shifted:

1. **`write_skill_variant` was already merged into `review_skill`** via mika#477. The standalone builtin no longer exists as a separate tool.
2. **The `skill-review` skill lives in the `mika` repo**, not `mika-skills` — source at `crates/mika-agent/templates/skills/skill-review/`.
3. **The bogus variant file was never committed** to `mika-skills` — `qa-review/google/gemini-2.5-flash/system_prompt.md` does not exist on any branch.
4. **No references to `write_agent_file` or `write_skill_variant` existed in `mika-skills`** at all.

The real remaining work was a one-line fix in the `mika` repo: a stale `write_agent_file` string in the `review_skill` tool description.

## Problem Statement

The `review_skill` tool description in `tools.json` said "write it using `write_agent_file`" — a tool that no longer persists skill variants. The system prompt also carried a prohibition line referencing `write_agent_file`. Per the "remove capability, don't prohibit" learning (2026-04-08), negative instructions about removed tools should be deleted entirely rather than carried forward.

## Solution

**Target repo: `mika`** (the skill source of truth).

### Change 1 — Fix `tools.json` description

**File:** `crates/mika-agent/templates/skills/skill-review/tools.json`

Replace the stale line:

```
"After receiving the result, adapt the prompt for the target model and write it using write_agent_file."
```

With:

```
"After receiving the result, adapt the prompt for the target model and persist it by calling review_skill again with the content parameter."
```

### Change 2 — Remove `write_agent_file` prohibition from system prompt

**File:** `crates/mika-agent/templates/skills/skill-review/system_prompt.md`

Delete:

```
**Do not call `write_agent_file` to persist a variant.** The agent home directory sandbox will reject the path. `review_skill` is the only correct tool for writing skill variants.
```

The workflow section above this line already describes using `review_skill` with `content` for persistence. The prohibition is redundant and violates the "remove capability, don't prohibit" principle.

### Change 3 — Layer E: no-op

`qa-review/google/gemini-2.5-flash/system_prompt.md` does not exist in `mika-skills` (confirmed via `git log --all`). No deletion needed.

### Change 4 — Do NOT add `write_skill_variant` to `required_tools`

The original issue spec requested `required_tools = ["review_skill", "write_skill_variant"]`. Since `write_skill_variant` was merged into `review_skill` via mika#477 and no longer exists as a standalone builtin, adding it would either cause a startup warning from `validate_skill()` or silently break the required-tools gate. The correct value is `required_tools = ["review_skill"]`.

## Acceptance Criteria

- [x] `tools.json` description no longer references `write_agent_file`.
- [x] `system_prompt.md` no longer contains the `write_agent_file` prohibition line.
- [x] `skill.toml` constraints keep `required_tools = ["review_skill"]` (no stale tool names).
- [x] Layer E: `gemini-2.5-flash/system_prompt.md` confirmed absent (never committed) — no-op.
- [x] Changes shipped on a single PR in the correct repo: senara-solutions/mika#485.

## Sources

- Issue: `senara-solutions/mika-skills#99`
- Landed PR: `senara-solutions/mika#485`
- Original (wrong-repo) docs-only PR: `senara-solutions/mika-skills#102` (relocated here)
- Superseding merge: `senara-solutions/mika#477` — merged `write_skill_variant` into `review_skill`
- Learning: `docs/solutions/prompt-engineering/2026-04-08-cross-repo-issue-scope-drift-after-upstream-merge.md`
- Related postmortem: `senara-solutions/mika-platform#17`
