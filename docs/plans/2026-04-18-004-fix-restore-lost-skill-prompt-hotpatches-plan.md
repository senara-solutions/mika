---
title: "Restore lost skill prompt hot-patches: run_gh schema discipline + M2/M3 pre-flight + P3 treatment"
type: fix
status: completed
date: 2026-04-18
---

# Restore Lost Skill Prompt Hot-Patches

## Overview

Six skill-prompt fixes hot-patched on 2026-04-17 were lost during the overnight reconcile cycle (PR #639). The bundle and deployed agents now regress to the pre-hot-patch state. This issue restores the patches: `run_gh` tool-call input shape discipline across 6 skill files, plus M2/M3/P3 structural fixes in self-dev.

## Problem Statement

PR #639 shipped CRITICAL gates, `store_fact` memory recording, and Rule 8 expansion — but silently overwrote 6 hot-patched prompt changes that addressed different failure modes:

1. **`run_gh` input shape bug** — session `4cbc6de7-...` put `--repo` inside the `command` array, causing the wrapper to reject the call. Agent dropped `--repo` on retry and falsely concluded the milestone didn't exist.
2. **Work item duplication** — session `749345f9-...` created `mika#629` work item twice before engine idempotency caught it.
3. **Missing fields** — same session created children with only `{label, type}`, dropping `parent_task_id`, `reference_url`, `source`. Children were orphaned from the milestone tree.

These are **tool-call input shape discipline** bugs — a different failure dimension from #639's workflow enforcement.

## Proposed Solution

Restore the 6 lost patches across 6 skill prompt files. All changes are to markdown prompt files — no Rust code changes.

## Acceptance Criteria

- [x] Step M2 in `self-dev` shows JSON tool-input form (`run_gh({...})`), not CLI shorthand
- [x] Step M3 has pre-flight idempotency check (`list_work_items` by `reference_url` before `create_work_item`)
- [x] Step M3 says "EXACTLY these 5 fields" with copy-as-is instruction + ⚠️ warning
- [x] Step P3 mirrors M3 fixes (pre-flight check + "EXACTLY 5 fields" + ⚠️)
- [x] Rule 4 enumerates the `run_gh` two-input schema, relocation rule, allowed subcommands, and incident `4cbc6de7-...`
- [x] Each of 5 additional skill files has a `run_gh` discipline rule/constraint with `4cbc6de7` incident reference
- [x] `wc -c skills/bundled/self-dev/system_prompt.md` stays under 49,152 bytes (final: 39,134; cap: 49,152)

## Technical Considerations

- **Prompt size budget:** self-dev is at 36,351 bytes with a 49,152 cap (~12,800 bytes headroom). The 6 patches add ~2-3KB total — well within budget.
- **Merge into existing steps:** Per learnings from `docs/solutions/prompt-engineering/2026-04-10-harden-skill-review-prompt-enforcement.md`, enforcement language must be merged into existing numbered steps, not added as parallel "MANDATORY" overlay blocks.
- **JSON examples over prose:** Per `docs/solutions/logic-errors/milestone-callback-misrouted-to-generic-workflow.md`, LLMs treat JSON code blocks as the strongest signal for structured output compliance.
- **No Rust code changes:** All changes are to `system_prompt.md` files. Build-time discovery via `build.rs` picks them up automatically.

## Implementation Plan

### File 1: `skills/bundled/self-dev/system_prompt.md`

**Change 1 — Step M2 (line ~339): JSON tool-input form**

Replace the CLI shorthand code block:
```bash
run_gh issue list --milestone <n> --repo senara-solutions/<repo> --state open --json number,title --jq 'sort_by(.number) | .[].number'
```

With JSON tool-input form:
```json
run_gh({
  "command": ["issue", "list", "--milestone", "<n>", "--state", "open", "--json", "number,title", "--jq", "sort_by(.number) | .[].number"],
  "repo": "senara-solutions/<repo>"
})
```

Add note: `repo` is a sibling parameter to `command`, never a flag inside the array.

**Change 2 — Step M3 (line ~351): Pre-flight idempotency check**

Add before the per-issue `create_work_item` loop:

> **PRE-FLIGHT CHECK (mandatory before every `create_work_item` call):** Call `list_work_items` filtered by matching `reference_url` to the GitHub issue URL. If a work item already exists for that issue, reuse its `task_id` — do NOT create a duplicate. Append the existing `task_id` to `child_wis` and move to the next issue.

**Change 3 — Step M3: "EXACTLY 5 fields" wording**

Replace `Call create_work_item with **all** of these fields (do NOT omit parent_task_id)` with:

> **Call `create_work_item` with EXACTLY these 5 fields** — copy the JSON block as-is, substituting the angle-bracket placeholders

Add ⚠️ after the JSON block:

> ⚠️ **ALL 5 FIELDS ARE REQUIRED.** Omitting `parent_task_id` ORPHANS the child from the milestone tree — callback routing to Step M4 will fail and the milestone loop breaks. Omitting `reference_url` disables the pre-flight check on the next run, causing duplicates. Do not truncate the JSON to `{"label": "...", "type": "issue"}` — that form is INCOMPLETE.

**Change 4 — Step P3 (line ~457): Mirror M3 fixes**

Apply the same three changes (pre-flight check, "EXACTLY 5 fields", ⚠️ warning) to Step P3.

**Change 5 — Rule 4 (line ~257): `run_gh` schema discipline**

Add to Rule 4's bullet list:

> - `run_gh` takes TWO SEPARATE INPUTS, not a single shell string. The schema is:
>   - `"command"`: array of gh subcommand arguments (e.g., `["issue", "list", "--milestone", "12", "--state", "open"]`)
>   - `"repo"`: a string, the `owner/repo` target (e.g., `"senara-solutions/mika"`) — **a sibling parameter to `command`, NOT a flag inside the array**.
>   Any example in this prompt written as shorthand — `run_gh("pr list --head <branch> --repo senara-solutions/<repo> ...")` — is **not literal**. When you execute it, you MUST split it: put every token EXCEPT `--repo VALUE` into `command`, and pull `VALUE` into the `repo` parameter. Including `--repo` inside `command` causes the wrapper to reject the call. If you see that error, **move `--repo` out of the array into the `repo` parameter** — do NOT drop `--repo` and retry without it (you will silently query the wrong repo and be lied to by the "not found" response). Also: `gh api` is **not an allowed subcommand**. Permitted: `pr, issue, run, workflow, release, repo, search, label, milestone, project`.

Add incident ref: `session 4cbc6de7-7e02-4552-a93f-524557cbe1eb on 2026-04-17 — milestone #12 dispatch failed because --repo was passed inside command, wrapper rejected it, agent dropped --repo on retry and falsely concluded milestone didn't exist.`

### Files 2-6: `run_gh` discipline rule

Each file gets the same `run_gh` schema discipline block (adapted to the file's existing format):

**The rule content** (adapted per file's style):

> `run_gh` takes TWO SEPARATE INPUTS: `"command"` (array of gh subcommand arguments) and `"repo"` (string, `owner/repo` target). `--repo` is a **sibling parameter to `command`**, NOT a flag inside the array. Any shorthand example like `run_gh("pr list --repo senara-solutions/mika ...")` is **not literal** — split it: put every token EXCEPT `--repo VALUE` into `command`, pull `VALUE` into `repo`. Including `--repo` inside `command` causes the wrapper to reject the call. If that happens, **move `--repo` out of the array** — do NOT drop it (you will silently query the wrong repo). `gh api` is not an allowed subcommand. Permitted: `pr, issue, run, workflow, release, repo, search, label, milestone, project`. (Incident: session `4cbc6de7-...` on 2026-04-17.)

**Insertion points:**

| File | Section | Position |
|------|---------|----------|
| `skills/bundled/qa-review/system_prompt.md` | `### Constraints` | New bullet at end (line ~356) |
| `skills/bundled/qa-review-build-callback/system_prompt.md` | `### Constraints` | New bullet at end (line ~157) |
| `skills/bundled/self-dev-webhook-ci/system_prompt.md` | Calibration Rules | Rule 7 after Rule 6 (line ~35) |
| `skills/bundled/self-dev-webhook-qa/system_prompt.md` | Calibration Rules | Rule 7 after Rule 6 (line ~118) |
| `skills/bundled/self-dev-iterate/system_prompt.md` | Calibration Rules | Rule 2 after Rule 1 (line ~64) |

## Dependencies & Risks

- **No code dependencies.** All changes are prompt-only; no Rust compilation needed for correctness (though build.rs will re-bundle).
- **Risk: prompt size.** Mitigated — 12,800 bytes headroom, changes add ~2-3KB.
- **Risk: anchor point drift.** Repo research confirmed all section headers and rule numbers match current HEAD.

## Sources

- Issue: mika#640
- Incident sessions: `4cbc6de7-7e02-4552-a93f-524557cbe1eb`, `749345f9-3a22-441a-b780-07bd1c82efd3`
- Prior fix PR: mika#639 (complementary, non-overlapping)
- Learnings: `docs/solutions/logic-errors/milestone-callback-misrouted-to-generic-workflow.md`
- Learnings: `docs/solutions/integration-issues/run-gh-string-to-array-coercion.md`
- Learnings: `docs/solutions/prompt-engineering/2026-04-17-always-on-skill-prompt-size-headroom.md`
