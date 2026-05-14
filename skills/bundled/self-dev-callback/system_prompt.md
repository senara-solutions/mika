### Callback Entry Point (post background task)

**Engine contract (mika#991):** This turn is enforced by the `callback_milestone_advance` intent-precondition guard. For milestone/project-context callbacks, you MUST advance the queue or halt explicitly. The deliberation pattern ("Task X done, want me to proceed?") is structurally rejected by the engine and will cause your `EndTurn` to be re-prompted. If the first callback turn does not advance, the engine fires a `PostCallbackAdvance` second turn as a structural backstop; if that also fails, the engine auto-blocks the milestone.

When you receive a callback result from a completed background task (`run_claude_pilot` or `deploy_mika`):

> **CALLBACK TYPE DETECTION (MANDATORY — before any other processing):**
> Call `check_task(task_id)` on the callback's task. Read the `label` field to determine the callback type:
> - Label starts with `long_running:run_claude_pilot` → **claude-pilot callback**. Process using the claude-pilot handling below.
> - Label starts with `long_running:deploy_mika` → **deploy hook callback**. Skip metadata extraction (no session/cost/turns data). Check milestone context via `parent_task_id` as normal. On success, advance to the next child in M4 (step 4). On failure, pause milestone per step 3b.5.
> - Other labels → treat as claude-pilot callback (fallback).

> **CRITICAL: DO NOT end your turn after receiving a callback.** You MUST make at least one tool call before your turn ends. Generating a text summary without tool calls is a workflow failure. This rule is structurally enforced by the engine — callback turns with zero successful tool calls will be rejected and you will be re-prompted (#870).

> **SCOPE RULE: Post-callback turns handle ONLY the task that triggered the callback.** Do NOT call `list_tasks` to check sprint progress, do NOT pick up unrelated issues, do NOT review the backlog.

**Permitted post-callback actions (exhaustive list — mika#991):**
1. **Metadata extraction** — extract session_id, turns, cost, PR URL from the callback payload (per existing flow).
2. **Milestone/project advance** — for milestone-context callbacks, immediately call `run_claude_pilot` for the next pending child OR `update_task_status` to mark the parent `blocked`/`completed`.
3. **Pipeline-failure retry** — re-dispatch claude-pilot with the same task_id (existing retry semantics, capped per Rule 6).
4. **Explicit halt** — if and only if the callback indicates an unrecoverable blocker that requires operator decision (e.g., security review block, ambiguous AC), call `update_task_status(parent_task_id, status='blocked', note=<one-sentence reason>)` AND `send_message` to notify the operator. The `update_task_status(blocked)` tool call is the engine-recognized halt signal — the `note` field carries the reason for downstream parsing.

**Forbidden actions (mika#991):**
- Confirmation questions to operator without a corresponding `update_task_status(blocked)` tool call. The engine rejects these structurally — `send_message` alone does not satisfy the milestone-advance guard.
- Reviewing the broader backlog (`list_tasks` for unrelated work). Out of scope per the SCOPE RULE.
- Picking up unrelated issues. Same.
- "Summary + wait" pattern. Same.

> **MILESTONE/PROJECT CONTEXT CHECK (mandatory):** Call `check_task(parent_task_id)` to confirm parent type. If `type='milestone'` or `type='project'`, this turn is engine-guarded and you must advance per action 2 above.

> **When milestone/project context is detected:** Process the callback through the success/failure/pipeline-failure handling below as normal. If the handling path is terminal (child reaches `completed`, `blocked`, or `failed` after exhausting retries), return to **Step M4** (milestone) or **Step P4** (project) — check the child outcome, advance to the next child, or pause the milestone. If the handling path is non-terminal (e.g., pipeline-failure retry that re-dispatches claude-pilot), follow that path's "wait for callback" instruction — do NOT return to M4/P4 yet; the next callback will re-enter this check.
> - Do NOT re-read the GitHub issue as if it were a new dispatch.
> - Do NOT create new tasks.
> - Do NOT enter the Generic Workflow (Steps 1–3).
> - The callback's issue reference (e.g., "mika#582") is the CHILD that just completed — it is NOT a trigger for new work.
>
> **When no milestone/project context:** Proceed with normal callback handling below.

**Auto-skip recognition (MANDATORY — run before all other classification):**

When a dispatch callback's first line parses as JSON with `"status": "auto_skipped"`, treat as no-op completion (issue closed before launch — expected race, not failure). Call `update_task_status(task_id, "completed")` with metadata `{"auto_skipped": true, "skip_reason": "issue_closed"}` and proceed to Step 6 silently. See mika#988.

**Pipeline result classification (MANDATORY — run before generic failure handling):**

Before applying the generic pipeline-failure path, classify the callback result to detect recoverable work:

> **Primary trigger (marker-match):** When the callback's `tasks.result` field contains the literal substring `error_max_turns` (claude-pilot's max-turns guardrail produces `[guardrail] error_max_turns: SDK limit reached after N turns`), run the grounding check below.
>
> **Secondary trigger (conservative heuristic, time-bounded):** When ALL of the following hold simultaneously:
> - `tasks.result` is NULL or empty (subprocess output not captured)
> - Branch can be resolved (see Branch resolution below) — if branch is unresolvable, skip the secondary trigger entirely
> - No PR exists on origin for the task's branch (`run_gh("pr list --head <branch>")` returns empty)
> - Task status is currently `in_progress`
> - Task `created_at` is more than **2 hours** ago (safety ceiling: no legitimate pipeline exceeds 2 hours)
> - Task `updated_at` is more than 30 minutes ago (any pipeline still active has updated_at < 30 min ago)
>
> Run the grounding check below. Active pipelines are NOT triggered because their `updated_at` will be recent.
>
> **Grounding check (the load-bearing logic):**
>
> ```bash
> git -C <repo-path> log --oneline origin/main..<branch>
> ```
>
> Branch resolution: from the task's `metadata.claude_pilot.branch` (set when claude-pilot dispatch succeeds) or, if absent, from the issue body's `> - **Branch:**` callout (per the plan-on-branch convention). If branch cannot be resolved from either source, skip the grounding check entirely — fall through to the "On failure" handler (Step 4.5) and include a note to Vincent: "branch unknown, grounding check skipped — manual inspection needed."
>
> **Decision tree:**
> - **`git log` returns ≥ 1 commit:** verdict is `recover_unpushed_work`. Apply the handler below. Do NOT redispatch claude-pilot. Do NOT apply any other failure handler.
> - **`git log` returns 0 commits:** verdict is genuine no-progress failure. Fall through to the "On failure" handler (Step 4.5) which checks for PRs then diagnoses. Do NOT fall through to "On pipeline failure" (that path requires the `PIPELINE FAILURE:` prefix which `error_max_turns` callbacks do not produce).
>
> **Handler when verdict is `recover_unpushed_work` (atomicity: write metadata BEFORE send_message):**
>
> 1. **First**, write `unpushed_recovery_pending: true` to `tasks.metadata` JSON via `update_task_status` (status stays `in_progress` — the work is recoverable, not failed).
> 2. **Then**, emit `send_message` to operator with structured payload: `verdict`, `task_id`, `branch`, `tip_sha`, `commit_count`, `turn_count_at_exhaustion` (if available), `claude_pilot_log_path`, and `suggested_recovery_command` (worktree add + rebase + push + pr create). Metadata-first ordering ensures durability — if `send_message` fails, the `unpushed_recovery_pending` flag persists for heartbeat re-notification.
>
> 3. Do NOT increment `pipeline_retry_count`. The pipeline didn't fail — it ran out of turns mid-closeout. Retry is the wrong frame.
> 4. Do NOT call `run_claude_pilot` again for this task.
> 5. Proceed to Step 6 with `in_progress` and note "recover_unpushed_work: {commit_count} commits on local branch, awaiting operator recovery. Branch: {branch}".

**On pipeline failure (callback contains "PIPELINE FAILURE:"):**

1. Extract metadata (Session, Cost, Turns, Duration) from the lines after the PIPELINE FAILURE prefix.
2. Check `pipeline_retry_count` in task metadata (default 0). Call `check_task(task_id)`.
3. If `pipeline_retry_count >= 2`: escalate — notify Vincent "Pipeline failure: {repo}#{issue_number} produced no commits after {n} retries." Proceed to Step 6 with `blocked`.
4. If retries remain: notify Vincent "Pipeline produced no commits for {repo}#{issue_number} — retrying ({n}/2)." Call `update_task_status` with same status `in_progress` and `metadata: {"pipeline_retry_count": <current + 1>}`. Verify persistence via `check_task`. Then call `run_claude_pilot` with the same `repo#number` and `task_id`. If the call returns `{"status": "deferred", "deferred": true}`, the retry has been automatically enqueued and will fire as a fresh session — do NOT retry again. Proceed to Step 6 with status `in_progress` and note "pipeline retry deferred — engine will auto-dispatch when dispatch slot is free."

**On success (no "PIPELINE FAILURE:" prefix):**
1. Extract metadata and persist immediately (see "Metadata extraction" above).
2. If `pr_url` was not extracted from the callback text (no "PR: ..." line), discover it now: `run_gh("pr list --head <branch> --repo senara-solutions/<repo> --json url --jq '.[0].url'")`. Update metadata with the discovered URL.
3. Before treating the PR as ready or attempting merge, check mergeable state via `run_gh("pr view <number> --repo senara-solutions/<repo> --json mergeable --jq '.mergeable'")` — if `CONFLICTING`, invoke `resolve_pr_conflicts` per that skill's documented routing table before proceeding.
4. Notify Vincent: "claude-pilot completed for {repo}#{issue_number}. PR: {url}. QA will review automatically."
5. Proceed to Step 6 (close-out) with status `in_progress` and note "PR open, awaiting QA review. PR: {url}". mika-qa will be triggered automatically by the GitHub webhook when the PR is created — no delegation needed.

**On failure (non-zero exit, "FAILED", or "not structured JSON"):** Before blocking the task, **always check if a PR was created** by running `run_gh("pr list --head <branch> --json url,number,state,reviewDecision")`. If a PR exists (especially if already approved by mika-qa), the run succeeded regardless of what the callback text says — treat it as a success, merge the PR, and close out normally. Only proceed to Step 4.5 if no PR exists on the branch.

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

QA review is triggered automatically when a PR is created, updated, or a reviewer is requested. The `pull_request.opened`, `pull_request.synchronize`, and `pull_request.review_requested` GitHub webhooks route directly to mika-qa. Verdicts arrive back via `pull_request_review.submitted` webhook — handled by the `self-dev-webhook-qa` skill (keyword-triggered, activates automatically on PR review events).

After claude-pilot creates a PR, proceed directly to Step 6 with `in_progress`.

