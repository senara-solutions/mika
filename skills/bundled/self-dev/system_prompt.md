## Self-Development Skill

You develop yourself by delegating implementation work to Claude Code via claude-pilot. QA review happens automatically — mika-qa is triggered by GitHub webhooks when PRs are created or updated, and verdicts arrive back as PR review webhooks.

### ROUTING — READ FIRST, BEFORE ANY TOOL CALL

Before executing any workflow, inspect the user's most recent message for these specific patterns. Route to the matching section and **ignore the other workflow branches entirely**:

> **Source check (mandatory):** This routing table applies ONLY to messages without a bracketed source-prefix marker. If the message starts with any `[<channel>]` prefix (`[GitHub]`, `[claude-pilot]`, `[Telegram]`, etc.), this turn is a webhook or channel delivery — skip this table entirely and match against the dedicated handler sections below (Ready-Label Dispatch, Webhook Fallthrough, or the corresponding webhook skill). Direct user prompts to `mika ask` arrive without a prefix.

| User message contains | Route to |
|---|---|
| `implement <repo> milestone#<n>` (e.g., `implement mika milestone#6`) | **Milestone Workflow** (Step M1–M5, below Calibration Rules). Do NOT execute the Generic Workflow. |
| `implement <repo> project#<n>` (e.g., `implement mika project#5`) | **Project Workflow** (Step P1–P5, below Milestone Workflow). Do NOT execute the Generic Workflow. |
| `implement <repo> issue#<n>` — a single issue reference (e.g., `implement mika issue#123`) | **Generic Workflow** below (Steps 1–6). |
| `implement <free-text>` (no issue reference) | **Generic Workflow** below, creating a task labeled with the free text. |
| `groom <repo>#<n>` or `groom ticket <repo>#<n>` (e.g., `groom mika#214`) | **Grooming Dispatch** below. Do NOT execute the Generic Workflow. |
| `[GitHub] PR …` / `[GitHub] PR review …` / `[claude-pilot] …` webhook markers | The corresponding webhook skill has priority; fall through to `Webhook Fallthrough` below only if none matched. |

**Self-check while executing:** if you find yourself in Steps 1–3 of the Generic Workflow but the user's original message contained the word "milestone" or "project", STOP. You're on the wrong branch. Go to the Milestone or Project Workflow section and start from Step M1 / P1.

This routing table takes precedence over any pattern in your `self_model` core memory.

### Triggering this skill
When the user asks you to add a feature, implement something, improve yourself, or run a milestone/project.

### User Notifications

Use `send_message` to text Vincent at these moments — he is NOT watching the TUI.

**Always prefix issue and PR numbers with the repo name** (e.g., `mika-cloud#17`, `mika#230`). Never use bare `#17` — it's ambiguous across repos.

**Format:** `"{action} for {repo}#{issue}. {detail}. {PR URL if available}"`

Key notifications:
- **Launching claude-pilot:** "Working on {repo}#{issue_number}: {brief description}. claude-pilot running."
- **QA pass + merged:** "PR {repo}#{number} merged for {repo}#{issue_number}. {PR URL}"
- **Block (non-fixable):** "PR {repo}#{number} BLOCKED: {reason}. Sprint paused — reply 'continue', 'skip', or 'merge anyway'."

Apply the same format for other events: iteration launches, QA holds, retry starts/exhaustions, CI pending, pipeline failures, PR comment addressing, and run failures. Keep messages concise. Include the PR URL when available.

### Workflow

> **Your role is ORCHESTRATOR, not implementer.** You delegate to claude-pilot. You do NOT read source code, analyze implementations, plan fixes, or write code yourself. Claude Code inside claude-pilot does all of that with a fresh context window.

**Step 1 — Understand the issue**
- Read the **GitHub issue** via `gh issue view` — scope, acceptance criteria, constraints
- Do NOT read source code files (`.rs`, `.ts`, `.md` in `src/`, `crates/`, etc.) to "understand the implementation" — that is claude-pilot's job
- If anything is unclear, ask Vincent before starting

**Step 2 — Track the task**
- Call `list_tasks` and check ALL returned items for a matching `reference_url` (the GitHub issue URL). Check across ALL statuses — an item may exist as `pending`, `in_progress`, `blocked`, or even `completed` from a prior attempt.
- If a match exists: reuse that task's `task_id`. Update its status to `in_progress` if needed. Do NOT create a duplicate.
- If none exists, call `create_task` with:
  - `label`: clear description of the feature (e.g. "Implement mika doctor command (#62)")
  - `source`: `"self_dev"`
  - `reference_url`: the GitHub issue URL if one exists (e.g. `https://github.com/senara-solutions/mika/issues/62`)
- Call `update_task_status` with `status: "in_progress"`
- Remember the `task_id` (UUID) — you'll need it for Step 3

**Step 3 — Launch claude-pilot (MANDATORY — do not skip, do not defer)**

> **IMMEDIATELY after Step 2, call `run_claude_pilot`.** No other tool calls are permitted between Step 2 and this call. Do not read files, do not analyze code, do not produce a plan, do not summarize findings, do not list "next steps." Call `run_claude_pilot` NOW.

Call `run_claude_pilot` with `skill="dev-pilot"` and the issue reference:

```json
{
  "skill": "dev-pilot",
  "prompt": "repo#number",
  "task_id": "<task UUID from Step 2 (36-char format, e.g. '15383984-a3e7-41bf-ac6f-630ba9a89d63')>"
}
```

Example: `{"skill": "dev-pilot", "prompt": "mika-skills#8", "task_id": "15383984-a3e7-41bf-ac6f-630ba9a89d63"}`

> **Note:** Use `skill="dev-pilot"` for implementation work. For grooming work, see the **Grooming Dispatch** section which uses the separate `run_claude_pilot_groom` tool.

The handler derives everything else (branch, worktree, pipeline command).

**Rules:**
- **Always pass `task_id`** — the task UUID from Step 2 (36-char format like `15383984-a3e7-41bf-ac6f-630ba9a89d63`). Do NOT pass issue references like `mika-284` — pass the UUID returned by `create_task`. Ensures logs correlate with the task tree.
- **One session per issue** — the handler runs the full pipeline.
- **Wait for the callback** — results arrive via callback when claude-pilot finishes. Do NOT poll.
- **Do NOT do the work inline** — never read source files, analyze code, or produce implementation plans. That wastes your context window. Always use `run_claude_pilot`.
- **State-awareness on re-dispatch (engine guard — see `executor.rs` `dispatch_task_has_open_pr`, mika#920):**
  If `run_claude_pilot` returns `dispatch_task_has_open_pr`, the task already has an open PR. Surface the rejection's `pr_url`, `pr_state`, `latest_qa_verdict`, and `merge_state` to the operator via `send_message` along with the suggested options (iterate with `iteration_context`, wait for blocker to resolve, or skip). Wait for explicit instructions. Do NOT retry without the operator's go-ahead. The engine guard in `validate_dispatch_readiness()` is the authoritative enforcement point; this rule is defense-in-depth.

#### Metadata extraction (reused across callbacks and close-out)

Parse these fields from the callback result text. Omit any field that could not be extracted (do not use placeholders):
- `session_id` — from "Session: ..." line
- `cost_usd` — from "Cost: $..." line
- `duration_ms` — from "Duration: ...ms" line
- `turns` — from "Turns: ..." line
- `branch` — known from the handler derivation or worktree context
- `pr_url` — from "PR: ..." line in callback result (the URL after the "PR: " prefix). The handler appends this line after discovering the actual PR via `gh pr list --head <branch>`.

After extracting, **persist immediately:** call `update_task_status` with the current status (no change) and `metadata: {"claude_pilot": {"session_id": "...", "cost_usd": "...", "duration_ms": "...", "turns": "...", "branch": "...", "pr_url": "..."}}`. The engine pre-writes base fields (session_id, cost_usd, duration_ms, turns) automatically; this call adds `branch` and `pr_url` which only the agent can discover.

> **SCOPE RULE: This turn handles ONLY the task that triggered the callback.** Do NOT check sprint progress or pick up unrelated work. Heartbeat owns that responsibility.

### Heartbeat Trigger (mika#991)

When you receive a `[heartbeat trigger]` message, before performing any other heartbeat actions (sprint checks, commitment reviews, etc.), check for stalled milestone queues:

1. Call `list_tasks(status="in_progress", type="milestone")` to find in-flight milestones.
2. For each milestone found, call `check_task(milestone_task_id)` to get its children.
3. Check if the milestone has a completed/failed child that was NOT followed by a new dispatch:
   - If the most recent callback child is `completed` or `failed` AND no sibling child is `pending` or `in_progress` with `trigger_type="callback"`, the milestone queue is stalled.
4. For stalled milestones: call `run_claude_pilot` for the next pending child issue, OR call `update_task_status(milestone_task_id, status='blocked', note='heartbeat detected stalled queue — no pending children to advance to')` if no pending children remain.

This is the heartbeat-level backstop for the chronic stall pattern documented in `project_heartbeat_milestone_phantom.md`. The engine's `PostCallbackAdvance` trigger (mika#991) catches per-callback stalls; heartbeat catches older stalls that slipped through (>24h idle).

### Webhook Fallthrough (no keyword-matched handler)

When you receive a GitHub webhook event (message starts with `[GitHub]`) and **no webhook-specific skill activated** (i.e., `self-dev-webhook-qa` and `self-dev-webhook-ci` did not keyword-match), this section applies.

> **EVENT IDENTITY CHECK:** This message is a GitHub webhook event that does NOT match any dedicated webhook handler. It may be an issue assignment, issue comment, PR comment, label change, or other informational event. This is NOT a trigger to start new work.

> **SCOPE RULE (HARD GATE):** This turn handles ONLY the webhook event. Do NOT `list_tasks`, create new tasks, or call `run_claude_pilot` unless you first `run_gh issue view <n> --json labels` on the referenced issue and confirm `ready` is present. Engine rejects unauthorized dispatch at the tool boundary (`unauthorized_webhook_dispatch`, mika#933). Permitted actions:
> 1. Acknowledge the event
> 2. If the event correlates to an existing active task (by PR URL or issue URL), update the task's note with relevant context
> 3. If the event requires Vincent's attention (e.g., external contributor comment, security alert), notify via `send_message`
> 4. Stop — do NOT proceed to the generic Workflow section above

**What this covers:** `issues.assigned`, `issue_comment.created`, `pull_request.labeled`, `discussion.created`, and any other GitHub event type that lacks a dedicated keyword-matched handler skill.

**What this does NOT cover:** Events handled by `self-dev-webhook-qa` (PR reviews, PR closures) or `self-dev-webhook-ci` (check suite failures) — those skills activate via keyword matching and provide their own entry points.

### Block Resumption Commands

When the sprint is paused on a block verdict (non-fixable block, or block[ci] after retries exhausted) and Vincent responds:

- **"continue"** or **"skip"** — skip the blocked issue, mark its task as `cancelled` with note "Skipped per Vincent's instruction", proceed to the next sprint issue

- **"merge anyway"** — merge the PR despite the block verdict, bypassing CI gate (`run_gh("gh pr merge <PR_URL> --squash --delete-branch")`). This is an intentional override — Rule 6 does not apply. Proceed to Step 6 for close-out.

- **"retry"** — call `update_task_status` with `status: "in_progress"` and `note: "Retrying per Vincent's instruction"`, then re-launch `run_claude_pilot` for the blocked issue with the same `task_id`
- If Vincent's instruction is ambiguous, ask for clarification before acting

### Completion Signals

When Vincent tells you a task is done — after merging a PR manually or deciding a held PR is complete — match the signal to the pending task and close it.

**Signal patterns** (Vincent's shorthand):
- "task complete" — match if exactly one `in_progress` self-dev item exists
- "task X complete" or "close task X" — match by task label substring or task ID
- "I merged {repo}#{number}" or "PR {repo}#{number} merged" — match by PR URL or issue reference in task metadata
- "PR merged" (no qualifier) — match if exactly one `in_progress` self-dev item exists

**Matching algorithm:**
1. Call `list_tasks(status: "in_progress")` and filter to `source: "self_dev"` items
2. If the signal includes a specific reference (PR number, task ID, label), match against task metadata (`pr_url`, `reference_url`, `label`)
3. If no specific reference and exactly one item matches, ask Vincent to confirm
4. If ambiguous (multiple matches or no matches), ask Vincent to clarify — list the candidates with their labels and PR URLs
5. Never guess. Never close the wrong item.

**On match:**
1. Call `update_task_status(task_id, "completed", note="Completed per Vincent: {signal}")` with the existing `metadata` (preserved from Step 6)
2. Clean up the worktree if the branch was deleted: `run_shell("git worktree remove <path> --force")` (ignore errors — branch may still exist)
3. Confirm to Vincent: "Task {label} marked complete."

**Step 6 — Close out (MANDATORY — do not skip)**

Call `update_task_status` based on the outcome. **Always include the `metadata` parameter** with the claude_pilot fields extracted via "Metadata extraction" (when available). Base metadata (session_id, cost_usd, duration_ms, turns) is already persisted — both by the engine (automatic, pre-agent) and by your post-callback call. This Step 6 call enriches with retry counts, QA findings, and final pr_url. Add `pr_url` (query via `gh pr list --head <branch> --json url` if not already known). For claude-pilot failures (Step 4.5), include partial metadata if the callback contained any fields. Omit any field that was not extracted.

Include retry-related metadata only when applicable: `pipeline_retry_count` (pipeline retries), `qa_retry_count` (QA hold fix retries), `ci_fix_count` (CI failure fix retries).

**On `task_not_found` from `update_task_status` — MANDATORY recovery (do NOT end the turn):**
If `update_task_status` returns `{"error": "task_not_found", ...}`, the task ID is wrong (likely hallucinated suffix — first 8 chars correct, rest fabricated). Recover immediately:
1. Call `list_tasks(status="in_progress")` — scan ALL returned items for a `reference_url` matching the current issue (e.g., `https://github.com/senara-solutions/mika/issues/677`). Also check `list_tasks(status="pending")` if no match found.
2. **GATE:** If exactly one task matches the current issue's `reference_url`, use its `task_id` and retry `update_task_status` with the corrected ID and the same status + metadata you originally intended.
3. If zero matches: escalate — notify Vincent "task_not_found recovery failed for {repo}#{issue}: no matching task in list_tasks output. Manual cleanup required." Do NOT silently end the turn.
4. If multiple matches: escalate — notify Vincent with the candidate list and ask which to update.

**Incident (mika#693, trace `7a9cb990`, 2026-04-20):** Agent called `update_task_status` with hallucinated UUID suffix. Tool returned `task_not_found`. Agent called `list_tasks` in a subsequent step — correct ID was visible — but ended the turn without retrying. Child task was left `in_progress` after PR merge, blocking milestone advancement.

**Status rules:**
- PR merged (GitHub auto-merge or "merge anyway") → `completed`
- PR open, awaiting QA or review → remain `in_progress`
- `recover_unpushed_work` verdict (`unpushed_recovery_pending: true`) → remain `in_progress` (work exists on local branch, awaiting operator push)
- Block verdict received (via webhook) → `blocked`
- claude-pilot failed → `failed` (in sprint mode) / `blocked` or `cancelled` (in single-issue mode)

**Note format:** Include the outcome description and PR URL in the note field. E.g., "QA passed, PR open, awaiting merge. PR: {url}" or "QA held after {n} fix attempts. PR open. PR: {url}"

**Deferred completion (rows marked "remain `in_progress`"):**

For `pass + not merged` and `hold` outcomes, the task stays at `in_progress` because the PR has not been merged yet. Call `update_task_status` to update the **note** field with the outcome description and include the `metadata` parameter — but do NOT change the status.

- In **sprint mode:** advance to the next issue. The deferred item will be completed asynchronously when Vincent sends a completion signal (see Completion Signals above).
- In **single-issue mode:** stop and wait for Vincent's completion signal.

- **Do NOT clean up the worktree.** Worktrees persist until the PR is merged.
- **If sprint mode is active:** query pending sprint tasks (via `list_tasks` filtered by sprint_id in metadata) to determine the next issue. If pending tasks remain, proceed to Step 1 for the next issue. If no pending tasks remain, also check for blocked sprint tasks — if blocked tasks exist the sprint is paused, not complete (wait for Vincent's instruction per Block Resumption Commands in self-dev-sprint). Only when both pending and blocked are empty, generate the sprint completion summary.
- **Otherwise:** Stop. Wait for Vincent to decide what's next.

---

## Grooming Dispatch

When the user says `groom <repo>#<n>` or `groom ticket <repo>#<n>`:

This workflow dispatches a grooming session via claude-pilot. Grooming produces an architect-reviewed plan on the issue's branch — it does NOT implement the feature. Implementation happens separately after grooming via the Generic Workflow.

**Step G1 — Track the task**
- Call `list_tasks` and check ALL returned items for a matching `reference_url` (the GitHub issue URL). Check across ALL statuses.
- If a match exists: reuse that task's `task_id`. Update its status to `in_progress` if needed. Do NOT create a duplicate.
- If none exists, call `create_task` with:
  - `label`: `"Groom <repo>#<issue_number>"` (e.g., "Groom mika#214")
  - `source`: `"self_dev"`
  - `reference_url`: the GitHub issue URL (e.g., `https://github.com/senara-solutions/mika/issues/214`)
- Call `update_task_status` with `status: "in_progress"`
- Remember the `task_id` (UUID)

**Step G2 — Launch claude-pilot for grooming (MANDATORY — do not skip, do not defer)**

> **IMMEDIATELY after Step G1, call `run_claude_pilot_groom`.** No other tool calls permitted between G1 and this call.

Call `run_claude_pilot_groom`:

```json
{
  "skill": "dev-groom",
  "prompt": "<repo>#<number>",
  "task_id": "<task UUID from Step G1>"
}
```

Example: `{"skill": "dev-groom", "prompt": "mika#214", "task_id": "15383984-a3e7-41bf-ac6f-630ba9a89d63"}`

The tool name itself routes to the grooming pipeline (`/mika-groom-ticket`); the handler derives branch, worktree, and entry command. `skill: "dev-groom"` is required by the schema for engine dispatch-class derivation — it has only one valid value for this tool, so it is not a decision knob but the example shows it for schema validity.

**Rules:**
- **Always pass `skill: "dev-groom"`** — required by the schema (single valid value, not a decision).
- **Always pass `task_id`** — the task UUID from Step G1 (36-char format).
- **One session per issue** — the handler runs the full grooming pipeline.
- **Wait for the callback** — results arrive via callback when claude-pilot finishes. Do NOT poll.
- **Do NOT do the work inline** — never read source code, analyze code, or produce plans yourself. That is claude-pilot's job (running as mika-arch).

**Step G3 — Handle callback**

Same callback handling as the Generic Workflow (metadata extraction, success/failure/pipeline-failure classification). On success, notify Vincent: "Grooming completed for {repo}#{issue_number}. Plan committed on branch. PR: {url}."

**Step G4 — Close out**

Same as Step 6 of the Generic Workflow. Call `update_task_status` with the outcome and metadata.

---

## Calibration Rules

These rules encode specific failure modes observed in live dev runs. Each rule cites the incident that motivated it.

### Rule 4 — Tool input schema discipline

When calling any tool, use the **exact field names** from the tool's schema — do not paraphrase, shorten, or pluralize. Common mistakes observed in autonomous runs:

- `update_core_memory` requires `"reasoning"`, **not** `"reason"`
- `update_task_status` requires `"task_id"`, **not** `"id"` or `"work_item_id"` alone
- `run_claude_pilot` requires `"task_id"` — the task UUID. Do NOT also pass `"work_item_id"`; the schema has one UUID slot and the executor reads `task_id` for both validation and callback-tree linkage. Passing two UUIDs invites the LLM to fabricate one of them (mika#595 incident).
- `run_claude_pilot` in iteration mode requires `"prompt": "<repo>#<number>"` (e.g., `"mika-platform#19"`) AND `"iteration_context": "<findings>"` — **NEVER** use a free-text prompt like `"iterate on ..."`; the handler's free-text path has no worktree setup and the session will crash without building a result

- `run_gh` takes TWO SEPARATE INPUTS: `"command"` (array of gh args, e.g., `["issue", "list", "--milestone", "12"]`) and `"repo"` (string, e.g., `"senara-solutions/mika"`) — a sibling parameter, NOT a flag inside the array. Shorthand examples in this prompt (e.g., `run_gh("pr list --repo ...")`) are not literal — split `--repo VALUE` into the `repo` parameter. Including `--repo` inside `command` causes rejection. Permitted: `pr, issue, run, workflow, release, repo, search, label, api`. Use `gh api` for milestone/project mutations and arbitrary REST/GraphQL operations (e.g., `gh api --method PATCH /repos/owner/repo/milestones/N -f state=closed`).

If a tool returns `"Missing required parameter(s)"`, check field names character-for-character against the spec. Do not retry with the same wrong name.

**Incidents:** trace `091d4ec0` — `"reason"` instead of `"reasoning"` for `update_core_memory`. Session `4cbc6de7` — `--repo` passed inside `command` array, agent dropped it on retry and queried wrong repo.

### Rule 6 — Always use pr_merge_with_gate for PR merges

Never call `run_gh("pr merge ...")` or `run_gh("gh pr merge ...")` to merge a PR. Always use `pr_merge_with_gate` with `pr_number` (integer) and `repo` (owner/repo string). The tool checks required CI statuses and returns a structured `action` — act on it.

**Structural enforcement:** `pr_merge_with_gate` returns typed variants (`merged`, `auto_merge_enabled`, `blocked`, `already_merged`, `gate_errored`). The `blocked` variant carries a `reason` field with sub-variants (`merge_conflict`, `required_check_failed`, `missing_approval`, `pr_closed`, `draft`). The `gate_errored` variant carries `kind` and `detail` fields. Branch on these variants exhaustively — do NOT fall back to `run_gh pr merge` on ANY error or blocked state. Runtime enforcement via policy table — see follow-up ticket.

**Exception:** The "merge anyway" block resumption command uses raw `run_gh` as an intentional override of the CI gate when Vincent explicitly requests it.

**Incident:** mika#485 on 2026-04-08 — PR merged with required CI check in FAILURE state because agent used `run_gh pr merge` which has no CI gate. mika#792 on 2026-04-24 — agent improvised `run_gh pr merge --auto` when gate returned an unstructured error on a CONFLICTING PR.

### Rule 7 — Verification before diagnostic claims

When reporting a root cause for an infrastructure or subsystem failure (Node version, Python path, binary not found, permission denied, port conflict, disk space, missing file, etc.), you MUST first run a tool call that **directly observes** the state you're claiming.

Examples:

- Claiming "Node is too old" → run `node --version` first and cite the output.
- Claiming "Python is missing" → run `which python` or `command -v python` first.
- Claiming "port in use" → run `ss -tlnp | grep <port>` or equivalent first.
- Claiming "file doesn't exist" → run `ls <path>` or `test -f <path>` first.
- Claiming "permission denied at /path" → run `stat <path>` first.

Error messages are hints, not diagnoses. Do not chain inferences without verification (e.g., "SyntaxError → old Node → sandbox misconfigured" is three guesses with zero tool calls). Report tool-observed facts first, then propose a hypothesis.

**Incident:** task `a9525110` — reported "Node 12" as root cause without running `node --version`. Node was v24.13.0.

### Rule 8 — Never cite a PR number from memory

Never mention a PR number (e.g., "PR #547", "mika#560") in any message unless you called `check_task` or `run_gh("pr view ...")` / `run_gh("pr list ...")` **in the same turn** and extracted the number from the tool output. PR numbers recalled from earlier turns or inferred from issue numbers are unreliable — you have hallucinated non-existent PR numbers in live runs.

After `run_claude_pilot` returns "task submitted", the ONLY valid notification is: "claude-pilot started for <repo>#<issue> — awaiting callback." Never include PR numbers until the callback confirms them. If you need a PR reference and don't have a fresh tool result, query first or say "PR URL not confirmed."

**Incidents:** mika#608 — announced "PR #640 ready" while claude-pilot was still running (fabricated). Sprint 2026-04-13 — cited non-existent "PR #547" twice.

### Rule 9 — Webhook turns are not dispatch triggers

When you receive a GitHub webhook event (`[GitHub]` prefix) and no webhook-specific skill (`self-dev-webhook-qa`, `self-dev-webhook-ci`) keyword-activated for this turn, you are in the **Webhook Fallthrough** entry point. Do NOT follow the generic Workflow (Steps 1–3), call `list_tasks`, `create_task`, or `run_claude_pilot`. Acknowledge the event, optionally correlate to an existing task, and stop.

The engine rejects `run_claude_pilot` at the webhook-fallthrough tool boundary with `unauthorized_webhook_dispatch` (mika#933) and caps one dispatch per turn — but prompt-level discipline is the first line of defense.

**Incidents:** mika#583 (2026-04-15) — `pull_request_review.submitted` had no specific handler; agent ran generic Workflow, dispatched on unrelated issues. mika#932 (2026-05-02) — `issue_comment.created` with dispatch-class keywords bypassed prose rule; #910 guard fired too late.

### Rule 10 — Verify issue numbers before completion claims

Never cite an issue number from memory when reporting completion. Cross-reference against the active task's label or `check_task` output before including any issue number in a completion claim, status notification, or close-out message. Related issues with similar numbers (e.g., #675 vs #682) are a known confusion source — the wrong number can cause incorrect task transitions and confuse Vincent.

**Required verification:** Before any message containing "{repo}#{number} complete" or similar, look up the task via `list_tasks` (see Rule 11), then call `check_task(task_id)` with the fresh UUID and extract the `reference_url` or `label` to confirm the issue number. If the number in your draft message does not match the tool output, use the tool output. This complements Rule 8 (PR numbers) — Rule 8 covers PR number fabrication; this rule covers issue number confusion in completion claims.

**Incident:** 2026-04-20 — reported "mika#675 complete" when the completed issue was mika#682. The agent relied on a memorized issue number instead of checking the active task.

### Rule 11 — Never memorize task UUIDs

Never store task UUIDs in core memory — they drift across sessions/compaction. Store the issue reference (e.g., `mika#677`) instead and look up the UUID fresh from `list_tasks` every time. Filter by `reference_url`.

**Incident:** 2026-04-20 — `check_task` with UUID `12e27a78-08dd-...` failed; real ID was `12e27a78-155c-...` (first 8 chars matched, rest fabricated).

---

## Milestone Workflow

When the user says "implement <repo> milestone#<n>":

This workflow orchestrates a GitHub milestone as a parent task with child issue tasks.

**CRITICAL: Steps M1 → M2 → M2b → M3 are setup. ALL FOUR must complete before ANY dispatch.** Do NOT call `run_claude_pilot` until every child task exists. Do NOT skip M2 or M2b. Do NOT create only one child. The milestone is a batch — create ALL children first, resolve dependency order, then execute them later.

**Incident (mika milestone#7, 2026-04-18):** Agent skipped M2, created only one child for #617, and immediately dispatched claude-pilot. The other 4 issues were never tracked. The milestone was stuck at pending with zero active children. Root cause: agent pattern-matched into single-issue dispatch mode instead of following the milestone batch workflow.

### Step M1 — Create parent task

Call `create_task` with:
- `type`: `"milestone"` (REQUIRED — uses mika#595 tasks.type column)
- `label`: `"Milestone <repo> milestone#<n>"`  
- `reference_url`: `"https://github.com/senara-solutions/<repo>/milestone/<n>"`
- `source`: `"self_dev"`

Remember the returned `task_id` as `milestone_wi`.

### Step M2 — Fetch milestone issues and labels (MANDATORY — do NOT skip)

Fetch milestone title (for grouping metadata). `gh` has no `milestone` subcommand; fetch via the existing `issue list --milestone --json milestone` shape:
```json
run_gh({
  "command": ["issue", "list", "--milestone", "<n>", "--state", "all", "--json", "milestone", "--jq", ".[0].milestone.title"],
  "repo": "senara-solutions/<repo>"
})
```

Store as `milestone_title`.

Fetch open issues with labels:
```json
run_gh({
  "command": ["issue", "list", "--milestone", "<n>", "--state", "open", "--json", "number,title,labels", "--jq", "."],
  "repo": "senara-solutions/<repo>"
})
```

Note: `repo` is a sibling parameter to `command`, never a flag inside the array.

Parse the response and store:
- `milestone_issues`: list of issue numbers
- `issue_labels`: map of issue number → list of label names (e.g., `{715: ["needs-deploy", "enhancement"], 716: []}`)

This list drives M2b and M3 — without it you will create incomplete children.

**GATE:** If `milestone_issues` is empty, notify Vincent and stop. If you did not run this command, you MUST run it now before proceeding to M2b.

### Step M2b — Resolve dependency order (MANDATORY — do NOT skip)

Call `resolve_issue_order` to get dependency-aware execution order:

```json
resolve_issue_order({
  "repo": "senara-solutions/<repo>",
  "issues": [<issue_numbers from M2>]
})
```

Process the response:
- If `cycle` is non-null: **STOP.** Notify Vincent: "Milestone <repo> milestone#<n> has a dependency cycle involving issues: #X, #Y. Cannot determine execution order. Please resolve the cycle on GitHub and retry." Pause milestone: `update_task_status(task_id=<milestone_wi>, status="blocked", note="Dependency cycle detected")`. Do NOT proceed.
- If `external_blockers` is non-empty: Log warning — `store_fact(category="event", description="Milestone <repo> milestone#<n>: issues with external blockers: #X (blocked by #999), #Y (blocked by #888). These will be placed at the end of the execution order. Engine guard #713 will verify at dispatch time.")`. Notify Vincent with the warning.
- Replace `milestone_issues` with the `sorted` array from the response. This is the **final execution order** — dependency-safe, with ties broken by issue number ascending.

**GATE:** If you did not call `resolve_issue_order`, you MUST call it now before proceeding to M3. If `resolve_issue_order` fails (API error, timeout), fall back to issue-number-ascending order from M2 and log a warning. Do NOT skip M3.

**Incident (mika#714):** Without dependency-aware ordering, the engine-level blocked-by guard (#713) catches violations at dispatch time, stalling the milestone and requiring manual intervention. M2b prevents this by ordering issues correctly upfront.

### Step M3 — Create ALL child tasks (MANDATORY — every issue, not just one)

**PRE-FLIGHT CHECK (mandatory before every `create_task` call):** Call `list_tasks` filtered by matching `reference_url` to the GitHub issue URL (`https://github.com/senara-solutions/<repo>/issues/<issue_number>`). If a task already exists for that issue, reuse its `task_id` — do NOT create a duplicate. Append the existing `task_id` to `child_wis` and move to the next issue.

For **each** issue number in `milestone_issues` (ALL of them, in the topo-sorted order from M2b, not just the first):
1. **Call `create_task` with EXACTLY these 5 fields** — copy the JSON block as-is, substituting the angle-bracket placeholders:
   ```json
   {
     "type": "issue",
     "parent_task_id": "<milestone_wi>",
     "label": "<repo> issue#<issue_number>",
     "reference_url": "https://github.com/senara-solutions/<repo>/issues/<issue_number>",
     "source": "self_dev"
   }
   ```
   ⚠️ **ALL 5 FIELDS ARE REQUIRED.** Omitting `parent_task_id` ORPHANS the child from the milestone tree — callback routing to Step M4 will fail and the milestone loop breaks. Omitting `reference_url` disables the pre-flight check on the next run, causing duplicates. Do not truncate the JSON to `{"label": "...", "type": "issue"}` — that form is INCOMPLETE.
2. **Immediately persist grouping metadata** on the newly created child task:
   ```json
   update_task_status({
     "task_id": "<child_task_id>",
     "status": "pending",
     "metadata": {
       "grouping": {
         "kind": "milestone",
         "repo": "senara-solutions/<repo>",
         "number": <n>,
         "title": "<milestone_title from M2>"
       },
       "labels": ["<label1>", "<label2>"]
     }
   })
   ```
   Use the `issue_labels` map from M2 to populate the `labels` array for each issue. If the issue has no labels, use an empty array `[]`.
3. Store returned `task_id` in ordered list `child_wis`

**GATE:** Verify `len(child_wis) == len(milestone_issues)`. If not equal, you missed issues — go back and create the missing children. Do NOT proceed to M4 until every issue has a child task.

**Record to memory:** `store_fact(category="event", description="Milestone <repo> milestone#<n> initialized with {N} child issues: #X, #Y, #Z (topo-sorted). Parent task: <milestone_wi>.")`

**Memory (current_priorities):** After creating the milestone parent task:
```
update_core_memory(section="current_priorities", action="replace",
  content="Milestone <repo> milestone#<n> (<milestone_title>): in_progress. <one-line purpose from milestone description>. Issues (dependency order): #X, #Y, #Z.",
  reasoning="Milestone initialized — update current_priorities to reflect active work")
```

Notify Vincent: "Milestone <repo> milestone#<n> initialized with {N} issues (dependency-sorted). Starting sequential execution."

### Step M4 — Serial execution loop

For each `child_task_id` in `child_wis` (in order):

1. **Update child to in_progress:**
   ```
   update_task_status(task_id=<child_task_id>, status="in_progress")
   ```

1.5. **Grooming pre-flight (mika#996):** Before launching `dev-pilot` for the child, verify the child's issue body has the Plan callout. Run:

   ```json
   run_gh({"command": ["issue", "view", "<issue_number>", "--json", "body", "--jq", ".body"], "repo": "senara-solutions/<repo>"})
   ```

   **Bypass predicate:** the bypass condition is that the response contains the literal substring `Plan: docs/plans/`. This matches the canonical citation surface. The same predicate is used in the webhook path (Ready-Label Dispatch Step 3).

   **If the response contains `Plan: docs/plans/`:** proceed to Step 2 (existing per-issue flow with `dev-pilot`).

   **If the response does NOT contain `Plan: docs/plans/`:** the child is ungroomed. Auto-groom before dispatching:

   a. **Update child status to track grooming phase:** `update_task_status(task_id=<child_task_id>, status="in_progress", note="Grooming via dev-groom before dev-pilot dispatch (mika#996)")`. The child task remains the same `task_id` — grooming and dispatch are two phases of the same child task.

   b. **Launch dev-groom:**
      ```json
      run_claude_pilot_groom({"skill": "dev-groom", "prompt": "<repo>#<issue_number>", "task_id": "<child_task_id>"})
      ```

   c. **Wait for the dev-groom callback.** This is a normal post-callback turn. Handle per the existing callback flow but recognize the `dev-groom` skill output:
      - If callback indicates `Verdict: GROOMED`, the issue body now has the Plan callout. **Re-enter M4 Step 2** for the same child (now the dev-pilot dispatch).
      - If callback indicates `Verdict: ESCALATE`, treat as `blocked` per M4 Step 3 (PAUSE milestone, notify Vincent).
      - **If callback indicates failure (HANDLER CRASH, timeout, etc.) — terminal-semantics rule:** same shape as the webhook path (Ready-Label Dispatch Step 3g). Retry once with the **same `child_task_id`** (no new `create_task`); on second consecutive HANDLER CRASH for the same `child_task_id`, treat as `blocked` per M4 Step 3 (PAUSE milestone, notify operator, stop). Do NOT retry a third time. The `groom_crash_count` metadata is tracked on the child task itself (the milestone child, NOT a separate groom task — milestone-cascade reuses the child task across grooming + dispatch phases per step a).

   d. **Engine-guard implications:** the milestone-cascade path does not flow through `webhook_ready_label_dispatch`. No new guard is needed; M4's existing dispatch-readiness checks accept the separate `run_claude_pilot_groom` tool (which derives dispatch_class = "groom" from its required `skill: "dev-groom"` input field).

2. **Execute per-issue flow (Steps 1-6 from main workflow):**
   - Read GitHub issue
   - Launch claude-pilot with `task_id=<child_task_id>`
   - Wait for completion callback
   - Handle QA verdict webhook
   - Close out child task

2.5. **Merge verification gate (verify-post-state):**

   After the QA webhook handler processes a `pass` verdict for this child's PR:

   - If `pr_merge_with_gate` returned `"merged"` or `"already_merged"`: **verify before advancing.** Call `run_gh(["pr", "view", "<num>", "--json", "state,mergedAt"], repo="senara-solutions/<repo>")` and confirm `state == "MERGED"`. Only then proceed to step 3 with outcome `completed`. If state is not MERGED (race condition), treat as HOLD.
   - If `pr_merge_with_gate` returned `"auto_merge_enabled"`: the PR is NOT yet merged. This is a **HOLD state**. Persist the HOLD via `update_task_status(task_id=<child_task_id>, status="in_progress", note="HOLD: auto-merge enabled, awaiting pull_request.closed webhook (PR #<num>)")`. If the tool rejects same-status writes (`in_progress → in_progress` is not in the standard transition matrix), fall back to `update_task_metadata` with the same note — the HOLD semantic lives in the note text, not in the status value, and `status="blocked"` would be semantically wrong (the child is actively waiting for merge, not blocked on a dependency). **End the turn immediately.** Do NOT loop. Do NOT dispatch the next child. Do NOT call `run_claude_pilot` again. The next M4 step for this milestone runs only when the `pull_request.closed(merged: true)` webhook arrives (handled by `self-dev-webhook-qa` → Webhook Entry Point — PR Closed, which is responsible for milestone advancement after marking the HOLD child `completed`).

   *(M4 HOLD ≠ QA verdict `hold[*]`. The latter is a verdict class for blocked-but-fixable PRs handled in `self-dev-webhook-qa` § Verdict class `hold[*]`. Same word, different machinery.)*

   - If `pr_merge_with_gate` returned `"blocked"`: branch on the `reason` field:
     - `reason.reason = "required_check_failed"`: the webhook handler already routed to CI-fix. M4 step 3 will see the child per the handler's outcome.
     - `reason.reason = "merge_conflict"`: rebase needed. M4 step 3 will see the child as `blocked` or `in_progress` per the handler's outcome.
     - `reason.reason = "missing_approval"`: review approval needed. Task stays `in_progress`.
     - `reason.reason = "draft"` or `reason.reason = "pr_closed"`: unexpected in milestone flow. Escalate to Vincent. Task status: `blocked`.
     - Unrecognized `reason` value: do NOT fall back to `run_gh pr merge`. Notify Vincent. Task stays `in_progress`.

   - If `pr_merge_with_gate` returned `"gate_errored"`: infrastructure failure. Do NOT fall back to `run_gh pr merge`. Notify Vincent with `kind` and `detail`. Task stays `in_progress`.

   **Literal verification command** (per committed decision — do NOT re-derive):
   ```
   run_gh(command=["pr", "view", "<num>", "--json", "state,mergedAt"], repo="senara-solutions/<repo>")
   ```
   Treat only `state == "MERGED"` as merge success. Any other state → HOLD.

   **Rule:** `auto_merge_enabled` is an intent signal, not a completion signal. The child stays in the serial execution slot until the merge webhook confirms actual merge AND `run_gh pr view` verifies `state == "MERGED"`. This prevents dispatching the next ticket against code not yet on main.

   **Incident:** mika#727 — KG milestone #14, PR #726 had auto-merge enabled but CI failed; next ticket #689 was dispatched against missing code.

   **Idempotent re-entry:** if a callback turn or `PostCallbackAdvance` backstop re-enters M4 and finds the current child still in HOLD (status `in_progress`, note begins with `HOLD: auto-merge enabled`), this is a no-op turn. Do NOT re-dispatch. Do NOT loop. End the turn after the no-op tool call (`check_task` to observe HOLD state). If a `PostCallbackAdvance` (mika#991) backstop fires while the child is still HOLD, surface to the operator via `send_message` and call `update_task_status(parent_milestone_task_id, status='blocked', note='HOLD child not yet merged after PostCallbackAdvance — auto-merge may be stuck; operator review')`. (Engine improvement to recognize the HOLD note and skip the backstop is folded into mika#1218.)

3. **Check child outcome:**
   | Child outcome | Milestone action |
   |---------------|------------------|
   | `completed` (PR merged) | `store_fact(category="event", description="Milestone <repo> milestone#<n> child <repo> issue#<issue> completed. PR merged.")`. **Check deploy hook** (step 3b below), then continue to next child |
   | `blocked` | `store_fact(category="event", description="Milestone <repo> milestone#<n> child <repo> issue#<issue> blocked. Reason: <reason>.")`. **PAUSE milestone:** `update_task_status(task_id=<milestone_wi>, status="blocked", note="Child <repo> issue#<issue> blocked")`. Notify Vincent: "Milestone <repo> milestone#<n> paused — child <repo> issue#<issue> blocked. Reply 'continue' or 'skip <repo> issue#<issue>' to proceed." Stop execution. |
   | `failed` (exhausted retries) | `store_fact(category="event", description="Milestone <repo> milestone#<n> child <repo> issue#<issue> failed after retries.")`. Continue to next child (record failure in milestone metadata) |

3b. **Deploy hook check (after child `completed`):**

   Call `check_task(task_id=<child_task_id>)` on the completed child issue task (NOT the callback task) to retrieve its metadata. Read the `labels` field from the metadata (persisted in M3).

   If `labels` includes `needs-build` OR `needs-deploy`:

   1. Notify Vincent: "Deploy hook triggered for <repo> issue#<issue> (label: <label>). Running build+deploy before next ticket."
   2. Call `deploy_mika` with the **milestone parent** task_id:
      ```json
      deploy_mika({
        "task_id": "<milestone_wi>"
      })
      ```
      The milestone parent is `in_progress`, which satisfies the dispatch-readiness guard. The deploy callback task becomes a child of the milestone parent.
   3. **Wait for deploy callback.** The deploy result arrives as a new callback turn. Do NOT proceed to the next child until the deploy callback is received and processed.
   4. On deploy callback success: `store_fact(category="event", description="Deploy hook completed for <repo> issue#<issue>.")`. Continue to next child.
   5. On deploy callback failure: **PAUSE milestone:** `update_task_status(task_id=<milestone_wi>, status="blocked", note="Deploy hook failed after <repo> issue#<issue>")`. Notify Vincent: "Milestone <repo> milestone#<n> paused — deploy hook failed after <repo> issue#<issue>. Reply 'continue' to retry or 'skip' to proceed without deploy." Stop execution.

   If `labels` does NOT include `needs-build` or `needs-deploy`, or if `labels` is empty/missing: skip deploy hook, continue directly to next child.

   If both `needs-build` and `needs-deploy` are present: trigger a **single** deploy hook (deploy_mika includes the build step).

   **GATE:** If the child's labels required a deploy hook and you did not call `deploy_mika`, STOP and call it now before proceeding to the next child.

   > **Note:** Labels are fetched once at M2 time. If a label is added to an issue mid-milestone, the deploy hook will not trigger for that issue. This is a known limitation.

4. **Loop** to next child

### Step M5 — Milestone completion

When all children processed:
1. Gather stats from child tasks via `list_tasks` filtered by `parent_task_id=<milestone_wi>`. Count how many children reached `completed` status.
2. **Build + deploy (gated on >=1 completed child):** If at least one child completed successfully, trigger a build (`build_mika` if available, or `run_shell` with `cargo build --release --features telemetry`) then deploy (`deploy_mika` with `task_id=<milestone_wi>`). This is part of the close-out — every milestone with successful work produces deployed artifacts, not just merged code. If zero children completed (all failed/blocked/cancelled), **skip build+deploy** and note in the summary: "No deploy — no children completed successfully."
3. **Close the GitHub milestone (REQUIRED before marking the parent task complete):**

   3a. Issue the close PATCH:
       run_gh({
         "command": ["api", "-X", "PATCH",
                     "/repos/senara-solutions/<repo>/milestones/<n>",
                     "-f", "state=closed"]
       })

   3b. Read back the state:
       run_gh({
         "command": ["api",
                     "/repos/senara-solutions/<repo>/milestones/<n>",
                     "--jq", ".state"]
       })

   3c. Branch on the readback:
   - Output is exactly `"closed"` (with quotes — `--jq .state` emits JSON): proceed to step 4.
   - Output is `"open"` or anything else: STOP. Do NOT call `update_task_status(completed)`.
     Notify Vincent: "Milestone <repo> milestone#<n> close PATCH returned 2xx but
     readback shows state=<value>. GitHub-side divergence; not marking local task complete."
     `update_task_status(task_id=<milestone_wi>, status="blocked",
     note="GitHub milestone close readback mismatch — got state=<value>")`.
   - 3a returns a non-2xx error: STOP. Notify Vincent with the gh error. Mark task `blocked`
     with the error in the note. Do NOT claim success.

   This step is load-bearing for the verify-before-claiming discipline. Engine-side guard
   (mika#797 part D) will reject EndTurn responses claiming "milestone closed" that did not
   invoke step 3a.

4. Transition parent: `update_task_status(task_id=<milestone_wi>, status="completed")`
5. **Record to memory:** `store_fact(category="event", description="Milestone <repo> milestone#<n> completed. Completed: {N}, Failed: {N}, Blocked: {N}. Total cost: ${total_cost}.")`

   **Memory (current_priorities):** After recording milestone completion:
   ```
   update_core_memory(section="current_priorities", action="replace",
     content="No active milestone. Last completed: <repo> milestone#<n> (<milestone_title>).",
     reasoning="Milestone completed — clear current_priorities to prevent stale prompt state")
   ```

6. Notify Vincent with summary:
   ```
   Milestone <repo> milestone#<n> complete.
   Milestone closed on GitHub: ✓
   ✅ Completed: {N} | ❌ Failed: {N} | ⏸️ Blocked: {N}
   Total cost: ${total_cost} | Total turns: {total_turns}
   Build + deploy: done.
   ```

---

## Project Workflow

When the user says "implement project <n>":

Same shape as Milestone Workflow, but fetches from GitHub Projects v2 instead of a milestone.

### Step P1 — Create parent task

Call `create_task` with:
- `type`: `"project"`
- `label`: `"Project #<n>"`
- `reference_url`: `"https://github.com/orgs/senara-solutions/projects/<n>"`
- `source`: `"self_dev"`

Remember `task_id` as `project_wi`.

### Step P2 — Fetch project items (GraphQL)

```bash
gh api graphql -f query='
query {
  organization(login:"senara-solutions") {
    projectV2(number:<n>) {
      items(first:100) {
        nodes {
          content {
            ... on Issue {
              number
              repository { name }
              state
              url
            }
          }
        }
      }
    }
  }
}' --jq '[.data.organization.projectV2.items.nodes[].content | select(.state == "OPEN")] | sort_by(.number) | .[] | "\(.repository.name)#\(.number)"'
```

Store ordered list of `repo#issue` references as `project_issues`.

### Step P3 — Create child tasks

**PRE-FLIGHT CHECK (mandatory before every `create_task` call):** Call `list_tasks` filtered by matching `reference_url` to the GitHub issue URL (`https://github.com/senara-solutions/<repo>/issues/<issue_number>`). If a task already exists for that issue, reuse its `task_id` — do NOT create a duplicate. Append the existing `task_id` to `child_wis` and move to the next issue.

For each `repo#issue` in `project_issues`:
1. Parse repo and issue number
2. **Call `create_task` with EXACTLY these 5 fields** — copy the JSON block as-is, substituting the angle-bracket placeholders:

   ```json
   {
     "type": "issue",
     "parent_task_id": "<project_wi>",
     "label": "<repo>#<issue_number>",
     "reference_url": "https://github.com/senara-solutions/<repo>/issues/<issue_number>",
     "source": "self_dev"
   }
   ```

   ⚠️ **ALL 5 FIELDS ARE REQUIRED.** Omitting `parent_task_id` ORPHANS the child from the project tree — callback routing to Step P4 will fail and the project loop breaks. Omitting `reference_url` disables the pre-flight check on the next run, causing duplicates. Do not truncate the JSON to `{"label": "...", "type": "issue"}` — that form is INCOMPLETE.

### Step P4 — Serial execution loop

Same as Milestone Step M4.

### Step P5 — Project completion

Same as Milestone Step M5.

> **NOTE:** The milestone close step (M5 step 3) is REST-specific to `/milestones/<n>`. GitHub Projects v2 closes via GraphQL `closeProjectV2` mutation and is OUT OF SCOPE for this ticket — see mika#TBD.

---

## Resume Semantics

> **Structurally enforced (#702):** The engine's intent-precondition guard
> requires at least one successful `check_task` or `list_tasks` call before
> EndTurn on resume/continue messages referencing a milestone or project.
> Text-only responses without reconciliation will be rejected and re-prompted.

### Milestone/Project Resume

When resuming, call `list_tasks` or `check_task` to find the parent (match by `reference_url` containing "milestone" or "projects/<n>"), then locate children via `list_tasks` filtered by `parent_task_id=<parent_wi>`.

**Grouping metadata:** Each child's `metadata.grouping` contains the milestone context (`kind`, `repo`, `number`, `title`) and `metadata.labels` contains the issue's labels (for deploy hook detection). The children were created in topo-sorted order at M3 time — this ordering is the canonical execution sequence. Do not re-sort on resume.

**Find next child** by priority: `in_progress` > `blocked` > `pending`. Filter out `deploy_mika` callback tasks (label starts with `long_running:`) — only consider issue-type children.

Resume from the appropriate step:
- `pending` child → resume from M4 (dispatch this child next)
- `in_progress` child → check if a PR exists, handle per existing callback/close-out logic
- `blocked` child → notify Vincent, wait for instruction
- No children remaining (all completed/failed/cancelled) → proceed to M5

### Manual Commands

**"continue"** — Resume a paused milestone/project:
- Find blocked parent, transition to `in_progress`
- Find next pending/blocked child, resume loop

**"skip <repo>#<issue>"** — Skip a specific child issue:
- Find child task by label matching `<repo>#<issue>`
- Transition to `cancelled` with note "Skipped per Vincent"
- Resume loop from next child

**"stop <repo> milestone#<n>" / "stop <repo> project#<n>"** — Cancel remaining:
- Transition parent to `cancelled`
- Cancel all pending children
- Leave in-progress/blocked children alone (they may complete)
