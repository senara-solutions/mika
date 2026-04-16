> Metadata extraction: see self-dev skill.

### Sprint/Batch Mode (Task-Based)

Sprint mode uses the **tasks engine** for durable state. Every sprint item is a task — queryable, recoverable across restarts, and auditable.

#### Sprint Initiation

When the user's prompt contains a **numbered list of issues** or says "sprint mode" / "batch mode":

1. **Parse issue references** from the user's prompt. Each reference should be in `repo#number` format (e.g., `mika#214`, `mika-skills#50`).

2. **Validate count** — maximum 5 issues per sprint. If the user provides more than 5, reject the entire request:
   "Sprint limited to 5 issues. You provided {N}. Please split into smaller sprints."
   Do NOT partially accept — either all issues are valid or none are.

3. **Generate a sprint_id** — use the format `sprint-YYYY-MM-DDTHH:MM:SS` (e.g., `sprint-2026-04-13T14:30:00`). This ID tags all tasks in this sprint for filtering and cleanup.

4. **Check for existing sprint tasks** — call `list_work_items(status: "pending")` and filter for items with `source: "self_dev"` and a `sprint_id` in metadata. Also check `list_work_items(status: "in_progress")` and `list_work_items(status: "blocked")` with the same filter — a prior sprint may have been interrupted mid-execution or paused on a block. If any sprint tasks exist from a prior sprint, ask Vincent:
   "Found {N} tasks from a previous sprint (sprint_id: {id}) — {pending} pending, {in_progress} in-progress, {blocked} blocked. Cancel them and start fresh, or resume the existing sprint?"
   Wait for instruction before proceeding.

5. **Create all sprint tasks upfront** — for each issue reference, call `create_work_item` with:
   - `label`: description derived from the issue (e.g., "Implement health endpoint (mika#214)")
   - `source`: `"self_dev"`
   - `reference_url`: the GitHub issue URL (e.g., `https://github.com/senara-solutions/mika/issues/214`)
   - `metadata`: `{"sprint_id": "<generated sprint_id>"}`

   All tasks are created in `pending` state.

6. **Confirm to Vincent** — send a summary:
   "Sprint started (sprint_id: {id}). Created {N} tasks:
   1. {label_1} — pending
   2. {label_2} — pending
   ...
   Starting with the first issue now."

#### Serial Execution Loop

After sprint initiation (or when resuming a sprint), execute tasks one at a time:

**Resuming a sprint:** If you are resuming after a restart or context loss and do not have the `sprint_id` in memory, recover it by calling `list_work_items(status: "pending")` and filtering for items with `source: "self_dev"` that have a `sprint_id` in metadata. Extract the `sprint_id` from the first matching task's metadata. If no pending tasks exist, also check `list_work_items(status: "in_progress")` with the same filter — a sprint may have a task mid-execution. If neither pending nor in_progress tasks are found, also check `list_work_items(status: "blocked")` — the sprint may be paused with all remaining tasks blocked. Use the recovered `sprint_id` for all subsequent queries.

1. **Pick the next task** — call `list_work_items(status: "pending")` and filter for items with `source: "self_dev"` and matching `sprint_id` in metadata. Pick the first one.

2. **If no pending tasks remain** — before declaring completion, also check `list_work_items(status: "blocked")` filtered by `source: "self_dev"` and `sprint_id`. If blocked tasks exist, the sprint is **paused**, not complete — notify Vincent and wait for block resumption commands. Only if both pending and blocked are empty, jump to **Sprint Completion Summary** below.

3. **Execute the task** — follow the self-dev workflow Steps 1–6:
   - Step 1: Read the GitHub issue (`gh issue view`)
   - Step 2: The work item already exists (created during initiation). Call `update_work_item_status` with `status: "in_progress"`.
   - Step 3: Call `run_claude_pilot` with the issue reference and `task_id`
   - Step 4: Wait for the completion callback
   - Step 5: (skipped — QA is webhook-driven)
   - Step 6: Close out based on outcome

4. **After Step 6 close-out, determine next action based on outcome:**

   | Outcome | Task status | Sprint action |
   |---------|-------------|---------------|
   | PR created, awaiting QA/merge | Remain `in_progress` | Pick next pending task |
   | PR merged | `completed` | Pick next pending task |
   | claude-pilot failed (after retries exhausted) | `failed` | Pick next pending task |
   | Block verdict (non-fixable) | `blocked` | **Pause sprint** — notify Vincent, wait for block resumption command |
   | Pipeline failure (after retries exhausted) | `failed` | Pick next pending task |

5. **Loop** — after handling the outcome, return to step 1 (pick next pending task).

**Critical constraints:**
- **One session at a time** — never call `run_claude_pilot` while another session is running
- **Do NOT use `current_priorities` in core memory for sprint state** — the tasks engine is the single source of truth
- **Do NOT skip failed tasks silently** — always mark them `failed` with a note describing the failure, then continue

#### Stop Sprint

When Vincent says "stop sprint" or "cancel sprint":

1. **Let any active session finish** — if a `run_claude_pilot` session is currently running, let it complete naturally. Do NOT attempt to kill it.

2. **Cancel all remaining pending and blocked tasks** — call `list_work_items(status: "pending")` and `list_work_items(status: "blocked")`, filtering both for items with `source: "self_dev"` and matching `sprint_id` in metadata. For each, call `update_work_item_status` with `status: "cancelled"` and `note: "Sprint stopped per Vincent's instruction"`.

3. **Notify Vincent** — "Sprint stopped. Cancelled {N} remaining tasks ({pending} pending, {blocked} blocked). {M} tasks were already completed/failed/in-progress."

#### Sprint Scope Discipline

A sprint is the **specific set of tickets dispatched together** during Sprint Initiation. When asked "what happened in the last sprint?" or "sprint status," report ONLY the tasks tagged with that sprint's `sprint_id`. Do NOT include unrelated work that happened to complete during the same time period. The sprint is defined by its task list, not by a date range.

#### Sprint Completion Summary

When all tasks with the sprint's `sprint_id` have reached a terminal or deferred state (no more `pending` or `blocked` tasks — all are `completed`, `failed`, `in_progress` awaiting merge, or `cancelled`):

1. **Gather evidence (Rule 3 — three-source rule):**
   - `list_work_items(status: "completed")` — filter by sprint_id
   - `list_work_items(status: "pending")` — filter by sprint_id (should be empty)
   - `list_work_items(status: "blocked")` — filter by sprint_id
   - `list_work_items(status: "failed")` — filter by sprint_id
   - `gh issue view <n> --repo <repo>` on every ticket the sprint touched

2. **If any source disagrees, do NOT claim the sprint is complete.** Report the discrepancy and wait for instruction.

3. **Send Vincent one summary organized by status:**
   - **Completed** — items merged during the sprint
   - **Awaiting your action** — items with open PRs awaiting merge + completion signal (task status: `in_progress`). Include PR URLs.
   - **Blocked** — items paused by block verdict
   - **Failed** — items where claude-pilot failed

#### Sprint Retrospective (Step 7)

After the Sprint Completion Summary (Step 6), run a retrospective **before stopping**. This is mandatory — do not skip it even if the sprint was fully successful.

**Data gathering (single turn, no extra tool calls needed):**
The sprint summary already queried all work items by `sprint_id`. Use that data — do NOT re-query. Extract from each work item's metadata:
- `cost_usd`, `turns`, `duration_ms` — claude-pilot resource usage
- `pipeline_retry_count`, `qa_retry_count`, `ci_fix_count` — retry history
- `error` field — failure modes encountered
- PR review verdicts from the summary's `gh pr view` results

**Structured analysis (LLM reasoning, no tools):**
Answer these questions using only the data already gathered:

1. **Cost & efficiency:** Total sprint cost. Which ticket was most expensive and why? Any tickets with >100 turns or >$15 that suggest the pipeline was spinning?

2. **Failure patterns:** What failed during this sprint? Group by failure type (handler crash, CI failure, merge conflict, QA block, pipeline retry). For each: was the failure mode covered by an existing calibration rule? Did you follow the rule?

3. **Rules that fired:** Which calibration rules were load-bearing this sprint? (e.g., Rule 7 prevented a false diagnosis, Rule 8 prevented a hallucinated PR number). Confirming what works is as important as finding what's broken.

4. **Uncovered gaps:** What situations did you encounter that no existing rule covers? Be specific — describe the scenario and what you did wrong or struggled with. These are calibration candidates.

5. **What you'd do differently:** For each failure, what was the correct action? Cite the specific workflow step or rule.

**Output — commit a retro doc:**
Write the retrospective to `docs/retrospectives/sprint-<sprint_id>.md` in the repo where the majority of work happened (or `mika-platform/docs/retrospectives/` for cross-repo sprints). Use `run_shell` with git commands — same pattern as plan/solution docs. Format:

```markdown
# Sprint Retrospective: <sprint_id>

## Tickets
| # | Repo | Issue | Status | PR | Cost | Turns | Retries |
...

## What went well
...

## Failure patterns
...

## Calibration candidates
...

## Rules that fired (confirmed load-bearing)
...
```

**Calibration issue creation:**
If the retro identified uncovered gaps (item 4), create a GitHub issue on `senara-solutions/mika-skills` for each calibration candidate:
```
gh issue create --repo senara-solutions/mika-skills \
  --title "calibration: <short description of the gap>" \
  --body "<scenario, what went wrong, proposed rule>" \
  --label "enhancement"
```

These issues feed directly into the next sprint's backlog — the system writes its own calibration rules from its own failure data.

**Then stop.** The retro is the final sprint action.

#### Block Resumption Commands

> Block resumption commands: see self-dev skill. After resuming (skip, merge, or retry), return to Serial Execution Loop step 1 to pick the next pending sprint task.

---

## Calibration Rules

These rules encode specific failure modes observed in live dev runs. Each rule cites the incident that motivated it.

### Rule 2 — Closure claims on umbrella tickets

Umbrella tickets contain multiple sub-items (sections A/B/C/D/...). **Closure claims apply per sub-item, not per umbrella.** You must NEVER mark a section of an umbrella ticket as done unless you can cite the **specific PR number** (or specific infra change with evidence, e.g. ruleset ID) that landed the actual code or config change for THAT section.

Examples from `mika-platform#17`:

- ✅ Section A: "done — Vincent direct infra, ruleset IDs 13380969/14822369/14822370/14822371" (cites specific artifacts)
- ❌ Section B: "done — mika#485 merged" (`mika#485` was unrelated; this is fabrication)
- ❌ Sprint: "fully closed" after receiving one webhook comment (umbrella progress ≠ sprint closure)

An unrelated merged PR is **NOT evidence of section closure**, no matter how plausible it sounds. If the diff doesn't touch the code the section describes, the section is not done.

**Incident:** trace `0e478ae0-...` on 2026-04-08 — generalized "Section A complete" webhook into "Sprint 2/3 fully closed" and wrote the fabrication into core memory.

### Rule 3 — Verifying closure claims (three-source rule)

Before emitting the words **"complete"**, **"closed"**, **"done"**, or **"finished"** about any sprint, umbrella, or multi-ticket workstream, you MUST gather evidence from **three sources in the same turn**:

1. `list_work_items status=completed` — what the engine believes you actually delivered
2. `list_work_items status=pending` — what is still queued
3. `gh issue view <n> --repo <repo>` on **every ticket the sprint touched** — authoritative GitHub state

If any of the three disagree, **do not claim closure.** Report the discrepancy in your response and stop. Wait for instruction.

A single webhook comment saying "section X complete" is **NOT sufficient evidence** for any closure claim. Webhook payloads tell you what changed; they do not tell you the total state of your work.

**Incident:** same trace as Rule 2 — all three sources would have shown the sprint incomplete (e.g. `mika#472` still OPEN).
