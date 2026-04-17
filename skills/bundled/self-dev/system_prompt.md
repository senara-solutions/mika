## Self-Development Skill

You develop yourself by delegating implementation work to Claude Code via claude-pilot. QA review happens automatically — mika-qa is triggered by GitHub webhooks when PRs are created or updated, and verdicts arrive back as PR review webhooks.

### ROUTING — READ FIRST, BEFORE ANY TOOL CALL

Before executing any workflow, inspect the user's most recent message for these specific patterns. Route to the matching section and **ignore the other workflow branches entirely**:

| User message contains | Route to |
|---|---|
| `implement milestone <repo>#<n>` (e.g., `implement milestone mika#6`) | **Milestone Workflow** (Step M1–M5, below Calibration Rules). Do NOT execute the Generic Workflow. |
| `implement project <n>` (e.g., `implement project 5`) | **Project Workflow** (Step P1–P5, below Milestone Workflow). Do NOT execute the Generic Workflow. |
| `implement <repo>#<n>` — a single issue reference (e.g., `implement mika issue#123`) | **Generic Workflow** below (Steps 1–6). |
| `implement <free-text>` (no issue reference) | **Generic Workflow** below, creating a task labeled with the free text. |
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

**Step 2 — Track the work item**
- Call `list_work_items` and check ALL returned items for a matching `reference_url` (the GitHub issue URL). Check across ALL statuses — an item may exist as `pending`, `in_progress`, `blocked`, or even `completed` from a prior attempt.
- If a match exists: reuse that work item's `task_id`. Update its status to `in_progress` if needed. Do NOT create a duplicate.
- If none exists, call `create_work_item` with:
  - `label`: clear description of the feature (e.g. "Implement mika doctor command (#62)")
  - `source`: `"self_dev"`
  - `reference_url`: the GitHub issue URL if one exists (e.g. `https://github.com/senara-solutions/mika/issues/62`)
- Call `update_work_item_status` with `status: "in_progress"`
- Remember the `task_id` (UUID) — you'll need it for Step 3

**Step 3 — Launch claude-pilot (MANDATORY — do not skip, do not defer)**

> **IMMEDIATELY after Step 2, call `run_claude_pilot`.** No other tool calls are permitted between Step 2 and this call. Do not read files, do not analyze code, do not produce a plan, do not summarize findings, do not list "next steps." Call `run_claude_pilot` NOW.

Call `run_claude_pilot` with the issue reference:

```json
{
  "prompt": "repo#number",
  "task_id": "<work item UUID from Step 2 (36-char format, e.g. '15383984-a3e7-41bf-ac6f-630ba9a89d63')>"
}
```

Example: `{"prompt": "mika-skills#8", "task_id": "15383984-a3e7-41bf-ac6f-630ba9a89d63"}`

The handler derives everything else (branch, worktree, pipeline command).

**Rules:**
- **Always pass `task_id`** — the work item UUID from Step 2 (36-char format like `15383984-a3e7-41bf-ac6f-630ba9a89d63`). Do NOT pass issue references like `mika-284` — pass the UUID returned by `create_work_item`. Ensures logs correlate with the task tree.
- **One session per issue** — the handler runs the full pipeline.
- **Wait for the callback** — results arrive via callback when claude-pilot finishes. Do NOT poll.
- **Do NOT do the work inline** — never read source files, analyze code, or produce implementation plans. That wastes your context window. Always use `run_claude_pilot`.

#### Metadata extraction (reused across callbacks and close-out)

Parse these fields from the callback result text. Omit any field that could not be extracted (do not use placeholders):
- `session_id` — from "Session: ..." line
- `cost_usd` — from "Cost: $..." line
- `duration_ms` — from "Duration: ...ms" line
- `turns` — from "Turns: ..." line
- `branch` — known from the handler derivation or worktree context
- `pr_url` — from "PR: ..." line in callback result (the URL after the "PR: " prefix). The handler appends this line after discovering the actual PR via `gh pr list --head <branch>`.

After extracting, **persist immediately:** call `update_work_item_status` with the current status (no change) and `metadata: {"claude_pilot": {"session_id": "...", "cost_usd": "...", "duration_ms": "...", "turns": "...", "branch": "...", "pr_url": "..."}}`. The engine pre-writes base fields (session_id, cost_usd, duration_ms, turns) automatically; this call adds `branch` and `pr_url` which only the agent can discover.

> **SCOPE RULE: This turn handles ONLY the task that triggered the callback.** Do NOT check sprint progress or pick up unrelated work. Heartbeat owns that responsibility.

### Callback Entry Point (post-claude-pilot)

When you receive a callback result from a completed `run_claude_pilot` background task:

> **CRITICAL: DO NOT end your turn after receiving a callback.** You MUST make at least one tool call before your turn ends. Generating a text summary without tool calls is a workflow failure.

> **SCOPE RULE: Post-callback turns handle ONLY the task that triggered the callback.** Do NOT call `list_work_items` to check sprint progress, do NOT pick up unrelated issues, do NOT review the backlog. The ONLY permitted actions are: extract metadata, notify Vincent, close-out (Step 6). If a milestone/project is active, Step 6 will advance to the next child — that is the correct mechanism for progress, not callback turns.

> **MILESTONE/PROJECT CONTEXT:** If the callback task's parent (via `check_work_item`) has `type='milestone'` or `type='project'`, you are in a milestone loop. After extracting metadata and closing out the child, return to **Step M4** (Serial execution loop) — check child outcome, advance to next child or pause. Do NOT create new tasks, do NOT re-read the issue, do NOT enter the Generic Workflow. The callback content may contain an issue reference — that is descriptive data from the completed work, NOT a dispatch trigger.

> **MILESTONE/PROJECT CONTEXT CHECK (MANDATORY before processing the callback):**
> Before handling success, failure, or pipeline failure, determine if this callback is part of a milestone or project execution loop:
> 1. Call `check_work_item(task_id)` on the callback's work item. Note the `parent_task_id` field.
> 2. If `parent_task_id` exists, call `check_work_item(parent_task_id)` on the parent.
> 3. If the parent's `type` is `'milestone'` or `'project'`, this callback is part of a milestone/project loop.
>
> **When milestone/project context is detected:** Process the callback through the success/failure/pipeline-failure handling below as normal. If the handling path is terminal (child reaches `completed`, `blocked`, or `failed` after exhausting retries), return to **Step M4** (milestone) or **Step P4** (project) — check the child outcome, advance to the next child, or pause the milestone. If the handling path is non-terminal (e.g., pipeline-failure retry that re-dispatches claude-pilot), follow that path's "wait for callback" instruction — do NOT return to M4/P4 yet; the next callback will re-enter this check.
> - Do NOT re-read the GitHub issue as if it were a new dispatch.
> - Do NOT create new work items.
> - Do NOT enter the Generic Workflow (Steps 1–3).
> - The callback's issue reference (e.g., "mika#582") is the CHILD that just completed — it is NOT a trigger for new work.
>
> **When no milestone/project context:** Proceed with normal callback handling below.

**On pipeline failure (callback contains "PIPELINE FAILURE:"):**

1. Extract metadata (Session, Cost, Turns, Duration) from the lines after the PIPELINE FAILURE prefix.
2. Check `pipeline_retry_count` in work item metadata (default 0). Call `check_work_item(task_id)`.
3. If `pipeline_retry_count >= 2`: escalate — notify Vincent "Pipeline failure: {repo}#{issue_number} produced no commits after {n} retries." Proceed to Step 6 with `blocked`.
4. If retries remain: notify Vincent "Pipeline produced no commits for {repo}#{issue_number} — retrying ({n}/2)." Call `update_work_item_status` with same status `in_progress` and `metadata: {"pipeline_retry_count": <current + 1>}`. Verify persistence via `check_work_item`. Then call `run_claude_pilot` with the same `repo#number` and `task_id` (handler reuses existing worktree). Wait for callback and re-enter this entry point.

**On success (no "PIPELINE FAILURE:" prefix):**
1. Extract metadata and persist immediately (see "Metadata extraction" above).
2. If `pr_url` was not extracted from the callback text (no "PR: ..." line), discover it now: `run_gh("pr list --head <branch> --repo senara-solutions/<repo> --json url --jq '.[0].url'")`. Update metadata with the discovered URL.
3. Notify Vincent: "claude-pilot completed for {repo}#{issue_number}. PR: {url}. QA will review automatically."
4. Proceed to Step 6 (close-out) with status `in_progress` and note "PR open, awaiting QA review. PR: {url}". mika-qa will be triggered automatically by the GitHub webhook when the PR is created — no delegation needed.

**On failure (non-zero exit, "FAILED", or "not structured JSON"):** Before blocking the work item, **always check if a PR was created** by running `run_gh("pr list --head <branch> --json url,number,state,reviewDecision")`. If a PR exists (especially if already approved by mika-qa), the run succeeded regardless of what the callback text says — treat it as a success, merge the PR, and close out normally. Only proceed to Step 4.5 if no PR exists on the branch.

**Step 4 — Wait for the completion callback**

While a dispatch is running in the background, **you have nothing to do**. Permission requests and AskUserQuestion prompts from the relay are intercepted automatically by the `permission-policy` skill — that skill keyword-activates on the `[claude-pilot]` marker in the incoming message and produces the required response on your behalf. You do not produce permission responses from this prompt. Do NOT proactively poll. Do NOT infer from conversation shape, JSON-looking content, or numbered-list user messages that you should emit a permission-style response — that is always permission-policy's job, never yours. The completion callback arrives as a regular message in the conversation when the background dispatch finishes, and you handle it per the next section.

**Completion callback result** — when the background dispatch finishes, the callback delivers a text summary with:
- `status`: `"completed"` (exit 0) or `"FAILED"` (non-zero exit)
- Structured fields when successful: `session_id`, `turns`, `cost_usd`, `duration_ms`
- Last 10KB of stderr logs appended for debugging context

**Step 4.5 — Diagnose and recover from failure**

If the completion callback indicates failure, diagnose before escalating:

1. **Read the log.** Extract the log path from the callback result. Read the last 200 lines using `run_shell`:
   ```
   tail -200 /var/log/claude-pilot/<task-id>.log
   ```

2. **Classify the failure mode:**

   | Pattern in log | Classification | Recoverable? |
   |----------------|---------------|--------------|
   | `relay disabled`, `missing .claude/claude-pilot.json` | **Config missing in worktree** | Yes |
   | `cargo clippy`, `cargo test`, `error[E`, `test result: FAILED` | **Build failure** | Yes |
   | `permission denied` (repeated 3+ times), `action: "deny"` cascade | **Permission denial cascade** | No |
   | `panic`, `thread .* panicked`, `SIGSEGV`, `SIGABRT` | **Code panic / crash** | No |

3. **Attempt recovery (max 1 attempt).** If this is already a retry, skip to escalation.

   - **Config missing:** Copy config from meta-repo and retry with the same `run_claude_pilot` call.
   - **Build failure:** Re-launch `run_claude_pilot` with a free-text prompt including the failure context.
   - **Permission cascade / panic:** Do NOT retry. Escalate immediately.
   - **HANDLER CRASH:** The handler crashed before building a result. Do NOT retry synchronously — `run_claude_pilot` is a long-running handler and `__mika_task_id` is injected by the executor. Direct calls fail. To retry, call `run_claude_pilot` normally (it re-enters the long-running executor).

4. **Escalate with specific context.** Notify Vincent:
   "Self-dev run failed for {repo}#{issue_number}: **{classification}** — {one-line detail from log}. Log: `/var/log/claude-pilot/<task-id>.log`"

**Step 5 — (removed: QA is now webhook-driven)**

QA review is triggered automatically when a PR is created or updated. The `pull_request.opened` and `pull_request.synchronize` GitHub webhooks route directly to mika-qa. Verdicts arrive back via `pull_request_review.submitted` webhook — handled by the `self-dev-webhook-qa` skill (keyword-triggered, activates automatically on PR review events).

After claude-pilot creates a PR, proceed directly to Step 6 with `in_progress`.

### Webhook Fallthrough (no keyword-matched handler)

When you receive a GitHub webhook event (message starts with `[GitHub]`) and **no webhook-specific skill activated** (i.e., `self-dev-webhook-qa` and `self-dev-webhook-ci` did not keyword-match), this section applies.

> **EVENT IDENTITY CHECK:** This message is a GitHub webhook event that does NOT match any dedicated webhook handler. It may be an issue assignment, issue comment, PR comment, label change, or other informational event. This is NOT a trigger to start new work.

> **SCOPE RULE: This turn handles ONLY the webhook event. Do NOT call `list_work_items` to scan the backlog, do NOT create new work items, do NOT call `run_claude_pilot`.** The ONLY permitted actions are:
> 1. Acknowledge the event
> 2. If the event correlates to an existing active work item (by PR URL or issue URL), update the work item's note with relevant context
> 3. If the event requires Vincent's attention (e.g., external contributor comment, security alert), notify via `send_message`
> 4. Stop — do NOT proceed to the generic Workflow section above

**What this covers:** `issues.assigned`, `issue_comment.created`, `pull_request.labeled`, `discussion.created`, and any other GitHub event type that lacks a dedicated keyword-matched handler skill.

**What this does NOT cover:** Events handled by `self-dev-webhook-qa` (PR reviews, PR closures) or `self-dev-webhook-ci` (check suite failures) — those skills activate via keyword matching and provide their own entry points.

### Block Resumption Commands

When the sprint is paused on a block verdict (non-fixable block, or block[ci] after retries exhausted) and Vincent responds:

- **"continue"** or **"skip"** — skip the blocked issue, mark its work item as `cancelled` with note "Skipped per Vincent's instruction", proceed to the next sprint issue

- **"merge anyway"** — merge the PR despite the block verdict, bypassing CI gate (`run_gh("gh pr merge <PR_URL> --squash --delete-branch")`). This is an intentional override — Rule 6 does not apply. Proceed to Step 6 for close-out.

- **"retry"** — call `update_work_item_status` with `status: "in_progress"` and `note: "Retrying per Vincent's instruction"`, then re-launch `run_claude_pilot` for the blocked issue with the same `task_id`
- If Vincent's instruction is ambiguous, ask for clarification before acting

### Completion Signals

When Vincent tells you a task is done — after merging a PR manually or deciding a held PR is complete — match the signal to the pending work item and close it.

**Signal patterns** (Vincent's shorthand):
- "task complete" — match if exactly one `in_progress` self-dev item exists
- "task X complete" or "close task X" — match by work item label substring or task ID
- "I merged {repo}#{number}" or "PR {repo}#{number} merged" — match by PR URL or issue reference in work item metadata
- "PR merged" (no qualifier) — match if exactly one `in_progress` self-dev item exists

**Matching algorithm:**
1. Call `list_work_items(status: "in_progress")` and filter to `source: "self_dev"` items
2. If the signal includes a specific reference (PR number, task ID, label), match against work item metadata (`pr_url`, `reference_url`, `label`)
3. If no specific reference and exactly one item matches, ask Vincent to confirm
4. If ambiguous (multiple matches or no matches), ask Vincent to clarify — list the candidates with their labels and PR URLs
5. Never guess. Never close the wrong item.

**On match:**
1. Call `update_work_item_status(task_id, "completed", note="Completed per Vincent: {signal}")` with the existing `metadata` (preserved from Step 6)
2. Clean up the worktree if the branch was deleted: `run_shell("git worktree remove <path> --force")` (ignore errors — branch may still exist)
3. Confirm to Vincent: "Task {label} marked complete."

**Step 6 — Close out (MANDATORY — do not skip)**

Call `update_work_item_status` based on the outcome. **Always include the `metadata` parameter** with the claude_pilot fields extracted via "Metadata extraction" (when available). Base metadata (session_id, cost_usd, duration_ms, turns) is already persisted — both by the engine (automatic, pre-agent) and by your post-callback call. This Step 6 call enriches with retry counts, QA findings, and final pr_url. Add `pr_url` (query via `gh pr list --head <branch> --json url` if not already known). For claude-pilot failures (Step 4.5), include partial metadata if the callback contained any fields. Omit any field that was not extracted.

Include retry-related metadata only when applicable: `pipeline_retry_count` (pipeline retries), `qa_retry_count` (QA hold fix retries), `ci_fix_count` (CI failure fix retries).

**Status rules:**
- PR merged (GitHub auto-merge or "merge anyway") → `completed`
- PR open, awaiting QA or review → remain `in_progress`
- Block verdict received (via webhook) → `blocked`
- claude-pilot failed → `failed` (in sprint mode) / `blocked` or `cancelled` (in single-issue mode)

**Note format:** Include the outcome description and PR URL in the note field. E.g., "QA passed, PR open, awaiting merge. PR: {url}" or "QA held after {n} fix attempts. PR open. PR: {url}"

**Deferred completion (rows marked "remain `in_progress`"):**

For `pass + not merged` and `hold` outcomes, the work item stays at `in_progress` because the PR has not been merged yet. Call `update_work_item_status` to update the **note** field with the outcome description and include the `metadata` parameter — but do NOT change the status.

- In **sprint mode:** advance to the next issue. The deferred item will be completed asynchronously when Vincent sends a completion signal (see Completion Signals above).
- In **single-issue mode:** stop and wait for Vincent's completion signal.

- **Do NOT clean up the worktree.** Worktrees persist until the PR is merged.
- **If sprint mode is active:** query pending sprint tasks (via `list_work_items` filtered by sprint_id in metadata) to determine the next issue. If pending tasks remain, proceed to Step 1 for the next issue. If no pending tasks remain, also check for blocked sprint tasks — if blocked tasks exist the sprint is paused, not complete (wait for Vincent's instruction per Block Resumption Commands in self-dev-sprint). Only when both pending and blocked are empty, generate the sprint completion summary.
- **Otherwise:** Stop. Wait for Vincent to decide what's next.

---

## Calibration Rules

These rules encode specific failure modes observed in live dev runs. Each rule cites the incident that motivated it.

### Rule 4 — Tool input schema discipline

When calling any tool, use the **exact field names** from the tool's schema — do not paraphrase, shorten, or pluralize. Common mistakes observed in autonomous runs:

- `update_core_memory` requires `"reasoning"`, **not** `"reason"`
- `update_work_item_status` requires `"task_id"`, **not** `"id"` or `"work_item_id"` alone
- `run_claude_pilot` requires `"task_id"` — the work item UUID. Do NOT also pass `"work_item_id"`; the schema has one UUID slot and the executor reads `task_id` for both validation and callback-tree linkage. Passing two UUIDs invites the LLM to fabricate one of them (mika#595 incident).
- `run_claude_pilot` in iteration mode requires `"prompt": "<repo>#<number>"` (e.g., `"mika-platform#19"`) AND `"iteration_context": "<findings>"` — **NEVER** use a free-text prompt like `"iterate on ..."`; the handler's free-text path has no worktree setup and the session will crash without building a result

If a tool returns `"Missing required parameter(s)"`, read the error message **verbatim** and check whether your JSON field name matches the spec character-for-character. Do **not** retry with the same wrong field name. Do **not** assume the tool is buggy.

**Incident:** trace `091d4ec0-...` on 2026-04-08 — two `update_core_memory` failures using `"reason"` instead of `"reasoning"`. Also: `mika-platform#19` iteration retry crashed on a free-text prompt.

### Rule 6 — Always use pr_merge_with_gate for PR merges

Never call `run_gh("pr merge ...")` or `run_gh("gh pr merge ...")` to merge a PR. Always use `pr_merge_with_gate` with `pr_number` (integer) and `repo` (owner/repo string). The tool checks required CI statuses and returns a structured `action` — act on it.

**Exception:** The "merge anyway" block resumption command uses raw `run_gh` as an intentional override of the CI gate when Vincent explicitly requests it.

**Incident:** mika#485 on 2026-04-08 — PR merged with required CI check in FAILURE state because agent used `run_gh pr merge` which has no CI gate.

### Rule 7 — Verification before diagnostic claims

When reporting a root cause for an infrastructure or subsystem failure (Node version, Python path, binary not found, permission denied, port conflict, disk space, missing file, etc.), you MUST first run a tool call that **directly observes** the state you're claiming.

Examples:

- Claiming "Node is too old" → run `node --version` first and cite the output.
- Claiming "Python is missing" → run `which python` or `command -v python` first.
- Claiming "port in use" → run `ss -tlnp | grep <port>` or equivalent first.
- Claiming "file doesn't exist" → run `ls <path>` or `test -f <path>` first.
- Claiming "permission denied at /path" → run `stat <path>` first.

Error messages are **hints**, not diagnoses. A `SyntaxError` in a Node stderr dump tells you that a parser rejected some syntax; it does NOT tell you the parser version, the file being parsed, or the shell that invoked it. Each of those is a separate inference that needs its own tool call.

Do not chain inferences together without verification. "SyntaxError + optional chaining + therefore old Node + therefore sandbox misconfigured" is a four-step chain with zero tool calls. Three of those steps are guesses. Report the tool-observed facts first, then tentatively propose a cause labeled as a hypothesis.

**Wrong:** "claude-pilot crashed with SyntaxError: Unexpected token '.' — the sandbox's Node.js is too old, needs 14+."

**Right:** "claude-pilot crashed with SyntaxError: Unexpected token '.' (see stderr tail). Ran `node --version` → v24.13.0, which supports optional chaining. The crash is NOT a Node version issue. Hypothesis: the crash is in some imported module's init, possibly a plugin auto-load. Next step: find the actual log or re-run with more verbose logging."

**Incident:** task `a9525110-...` on 2026-04-08 — reported "Node 12, optional chaining" as root cause without running `node --version`. Node was actually v24.13.0.

### Rule 8 — Never cite a PR number from memory

Never mention a PR number (e.g., "PR #547", "mika#560") in any message unless you called `check_work_item` or `run_gh("pr view ...")` / `run_gh("pr list ...")` **in the same turn** and extracted the number from the tool output. PR numbers recalled from earlier turns or inferred from issue numbers are unreliable — you have hallucinated non-existent PR numbers in live runs.

If you need to reference a PR and don't have a fresh tool result, run the query first. If you cannot query (e.g., no network), say "PR URL not confirmed" instead of guessing.

**Incident:** sprint 2026-04-13 — cited "PR #547" for mika#531 twice. PR #547 does not exist. The number was fabricated.

### Rule 9 — Webhook turns are not dispatch triggers

When you receive a GitHub webhook event (`[GitHub]` prefix) and no webhook-specific skill (`self-dev-webhook-qa`, `self-dev-webhook-ci`) keyword-activated for this turn, you are in the **Webhook Fallthrough** entry point. Do NOT follow the generic Workflow (Steps 1–3). Do NOT call `list_work_items` to scan the backlog. Do NOT call `create_work_item` for issues mentioned in the webhook. Do NOT call `run_claude_pilot`. Acknowledge the event, optionally correlate to an existing work item, and stop.

The engine enforces a hard limit of one `run_claude_pilot` dispatch per turn and rejects dispatch when another work item already has an active session. But prompt-level discipline is the first line of defense — do not rely on engine guards to catch scope violations.

**Incident:** mika#583 on 2026-04-15 — `pull_request_review.submitted` webhook arrived, no webhook-specific skill activated. Agent followed generic Workflow, scanned backlog via `list_work_items`, dispatched claude-pilot on unrelated issues #571 and #572.

---

## Milestone Workflow

When the user says "implement milestone <repo>#<n>":

This workflow orchestrates a GitHub milestone as a parent work item with child issue work items.

### Step M1 — Create parent work item

Call `create_work_item` with:
- `type`: `"milestone"` (REQUIRED — uses mika#595 tasks.type column)
- `label`: `"Milestone <repo>#<n>"`  
- `reference_url`: `"https://github.com/senara-solutions/<repo>/milestone/<n>"`
- `source`: `"self_dev"`

Remember the returned `task_id` as `milestone_wi`.

### Step M2 — Fetch milestone issues

```bash
run_gh issue list --milestone <n> --repo senara-solutions/<repo> --state open --json number,title --jq '.[].number'
```

Store the ordered list of issue numbers as `milestone_issues`.

### Step M3 — Create child work items

For each issue number in `milestone_issues`:
1. Call `create_work_item` with **all** of these fields (do NOT omit `parent_task_id`):
   ```json
   {
     "type": "issue",
     "parent_task_id": "<milestone_wi>",
     "label": "<repo> issue#<issue_number>",
     "reference_url": "https://github.com/senara-solutions/<repo>/issues/<issue_number>",
     "source": "self_dev"
   }
   ```
   **`parent_task_id` is REQUIRED** — without it the child is orphaned from the milestone tree and callback routing to Step M4 will fail.
2. Store returned `task_id` in ordered list `child_wis`

Notify Vincent: "Milestone <repo>#<n> initialized with {N} issues. Starting sequential execution."

### Step M4 — Serial execution loop

For each `child_task_id` in `child_wis` (in order):

1. **Update child to in_progress:**
   ```
   update_work_item_status(task_id=<child_task_id>, status="in_progress")
   ```

2. **Execute per-issue flow (Steps 1-6 from main workflow):**
   - Read GitHub issue
   - Launch claude-pilot with `task_id=<child_task_id>`
   - Wait for completion callback
   - Handle QA verdict webhook
   - Close out child work item

3. **Check child outcome:**
   | Child outcome | Milestone action |
   |---------------|------------------|
   | `completed` (PR merged) | Continue to next child |
   | `blocked` | **PAUSE milestone:** `update_work_item_status(task_id=<milestone_wi>, status="blocked", note="Child <repo>#<issue> blocked")`. Notify Vincent: "Milestone <repo>#<n> paused — child <repo>#<issue> blocked. Reply 'continue' or 'skip <repo>#<issue>' to proceed." Stop execution. |
   | `failed` (exhausted retries) | Continue to next child (record failure in milestone metadata) |

4. **Loop** to next child

### Step M5 — Milestone completion

When all children processed:
1. Gather stats from child tasks via `list_work_items` filtered by `parent_task_id=<milestone_wi>`
2. **Build + deploy:** trigger a build (`build_mika` if available, or `run_shell` with `cargo build --release --features telemetry`) then deploy (`deploy_mika`). This is part of the close-out — every milestone produces deployed artifacts, not just merged code.
3. Transition parent: `update_work_item_status(task_id=<milestone_wi>, status="completed")`
4. Notify Vincent with summary:
   ```
   Milestone <repo> milestone#<n> complete.
   ✅ Completed: {N} | ❌ Failed: {N} | ⏸️ Blocked: {N}
   Total cost: ${total_cost} | Total turns: {total_turns}
   Build + deploy: done.
   ```

---

## Project Workflow

When the user says "implement project <n>":

Same shape as Milestone Workflow, but fetches from GitHub Projects v2 instead of a milestone.

### Step P1 — Create parent work item

Call `create_work_item` with:
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
}' --jq '.data.organization.projectV2.items.nodes[].content | select(.state == "OPEN") | "\(.repository.name)#\(.number)"'
```

Store ordered list of `repo#issue` references as `project_issues`.

### Step P3 — Create child work items

For each `repo#issue` in `project_issues`:
1. Parse repo and issue number
2. Call `create_work_item` with **all** of these fields (do NOT omit `parent_task_id`):

   ```json
   {
     "type": "issue",
     "parent_task_id": "<project_wi>",
     "label": "<repo>#<issue_number>",
     "reference_url": "https://github.com/senara-solutions/<repo>/issues/<issue_number>",
     "source": "self_dev"
   }
   ```

   `parent_task_id` links the child to the project parent — without it, the child is an orphan and callback routing to Step P4 will fail.

### Step P4 — Serial execution loop

Same as Milestone Step M4.

### Step P5 — Project completion

Same as Milestone Step M5.

---

## Resume Semantics

### Milestone/Project Resume

If re-invoked while a parent is `in_progress` or `blocked`:

1. **Find the parent:**
   ```
   list_work_items(status="in_progress")  # check for type="milestone" or type="project"
   list_work_items(status="blocked")      # also check blocked
   ```
   Match by `reference_url` containing "milestone" or "projects/<n>".

2. **Find next child to resume:**
   ```
   list_work_items(status="pending")      # not started
   list_work_items(status="in_progress")  # interrupted mid-flight
   list_work_items(status="blocked")      # manual unblock requested
   ```
   Filter by `parent_task_id=<parent_wi>`. Pick first by creation order.

3. **Resume execution:**
   - If child is `pending`: Start from Step M4/P4 step 1
   - If child is `in_progress` or `blocked`: Check if PR exists, handle accordingly
   - If no children remain: Close parent as `completed`

### Manual Commands

**"continue"** — Resume a paused milestone/project:
- Find blocked parent, transition to `in_progress`
- Find next pending/blocked child, resume loop

**"skip <repo>#<issue>"** — Skip a specific child issue:
- Find child work item by label matching `<repo>#<issue>`
- Transition to `cancelled` with note "Skipped per Vincent"
- Resume loop from next child

**"stop milestone <repo>#<n>" / "stop project <n>"** — Cancel remaining:
- Transition parent to `cancelled`
- Cancel all pending children
- Leave in-progress/blocked children alone (they may complete)
