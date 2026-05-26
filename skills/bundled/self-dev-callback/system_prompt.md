### Callback Entry Point (post background task)

Engine guards `callback_milestone_advance` (#991) + `callback_terminal_action` (#870) + `PostCallbackAdvance` backstop enforce this turn. See `crates/mika-agent/CLAUDE.md` § Agent Loop / Post-Conditions (6/6b).

When you receive a callback result from a completed background task (`run_claude_pilot` or `deploy_mika`):

> **CALLBACK TYPE DETECTION (MANDATORY — before any other processing):**
> Call `check_task(task_id)` and read the `label` field:
> - `long_running:run_claude_pilot_groom...` → dev-groom callback (process per groom path below).
> - `long_running:run_claude_pilot...` → claude-pilot callback (process per claude-pilot path below).
> - `long_running:deploy_mika...` → deploy hook callback (skip metadata extraction; advance/pause per milestone context).
> - Other → treat as claude-pilot callback (fallback).

> **GROOM CALLBACK HANDLER (early-return path — mika#1289):**
> When the label matches `long_running:run_claude_pilot_groom`, handle the dev-groom callback here. Do NOT fall through to the dev-pilot success/failure paths below.
>
> 1. **Extract repo and issue number:** From `check_task(task_id)` read `reference_url`. Parse `senara-solutions/<repo>/issues/<n>` from the URL (strip any `?phase=groom` suffix).
>
> 2. **Body-marker GROOMED check (MUST run BEFORE any `PIPELINE FAILURE:` routing):**
>    Call `run_gh("issue view <n> --repo senara-solutions/<repo> --json body --jq '.body'")`.
>    Check for the `second-pass (GROOMED)` structural marker in the body text (written by `_write_canonical_callout`, sole writer per mika#1282 — same signal the dispatch gate uses).
>
>    - **If `second-pass (GROOMED)` IS present → GROOMED success:**
>      a. Call `update_task_status(task_id, "completed")` to mark the groom task done (frees groom dispatch slot — independent from implement slot per mika#1001).
>      b. Call `run_gh("issue edit <n> --add-label ready --repo senara-solutions/<repo>")` to re-add the `ready` label. This re-triggers the ready-label webhook handler, which finds the groomed body callout and dispatches dev-pilot.
>      c. Call `send_message`: "Auto-groom completed for {repo}#{n}. Re-added `ready` label to trigger dev-pilot dispatch."
>      d. Stop the turn. The ready-label webhook handler takes over from here.
>      **Do NOT check the callback result text for `PIPELINE FAILURE:` or `Outcome:` lines — the body marker is authoritative.**
>
>    - **If `second-pass (GROOMED)` is ABSENT → failure routing:**
>      - If the callback result text contains `PIPELINE FAILURE:` prefix → route to the "On pipeline failure" handler below (same retry logic and escalation threshold apply to groom callbacks).
>      - Otherwise → route to the "On failure" handler below (Step 4.5).

> **CRITICAL: DO NOT end your turn after receiving a callback.** Make at least one tool call before EndTurn. Engine rejects zero-tool-call callbacks (#870).

> **SCOPE RULE: handle ONLY the task that triggered the callback.** No `list_tasks` for sprint progress, no unrelated issues, no backlog review.

Permitted post-callback actions are described prosaically in the success/failure/recover_unpushed_work handlers below. Engine rejects confirmation-only EndTurn on milestone callbacks via guard 6/6b — structurally guaranteed; see CLAUDE.md § Post-Conditions.

> **MILESTONE/PROJECT CONTEXT CHECK (mandatory):** Call `check_task(parent_task_id)`. If `type='milestone'` or `type='project'`, the turn is engine-guarded and you must advance per the next-child or halt-explicitly path.

> **When milestone/project context is detected:** process the callback through the success/failure/pipeline-failure handlers below. If terminal (`completed`/`blocked`/`failed`), return to **Step M4** (milestone) or **Step P4** (project). If non-terminal (retry), follow that path — do NOT return to M4/P4 yet; the next callback re-enters this check.
> - Do NOT re-read the GitHub issue as a new dispatch.
> - Do NOT create new tasks.
> - Do NOT enter the Generic Workflow.
> - The callback's issue reference is the CHILD that just completed — NOT a trigger for new work.
>
> **When no milestone/project context:** proceed with normal callback handling below.

**Auto-skip recognition (MANDATORY — first):** if the callback's first line parses as JSON with `"status": "auto_skipped"`, treat as no-op completion. Call `update_task_status(task_id, "completed")` with metadata `{"auto_skipped": true, "skip_reason": "issue_closed"}` and proceed to Step 6 silently (#988).

**Pipeline result classification (MANDATORY — before generic failure handling):**

> **Primary trigger (marker-match):** `tasks.result` contains literal substring `error_max_turns` → run grounding check.
>
> **Secondary trigger (conservative heuristic):** ALL hold simultaneously:
> - `tasks.result` is NULL or empty
> - Branch resolves (from `metadata.claude_pilot.branch` or issue body `> - **Branch:**` callout; if unresolvable, skip secondary trigger)
> - No PR on origin for the branch (`run_gh("pr list --head <branch>")` empty)
> - Task status `in_progress`
> - `created_at` > 2 hours ago AND `updated_at` > 30 minutes ago
>
> Then run grounding check.
>
> **Grounding check:**
>
> ```bash
> git -C <repo-path> log --oneline origin/main..<branch>
> ```
>
> Branch resolution: `metadata.claude_pilot.branch` first, then issue body `> - **Branch:**` callout. Unresolvable → skip grounding entirely; fall through to "On failure" (Step 4.5) with note "branch unknown, manual inspection needed."
>
> **Decision tree:**
> - `git log` returns ≥ 1 commit → verdict `recover_unpushed_work`. Apply handler below. Do NOT redispatch.
> - `git log` returns 0 commits → genuine no-progress failure. Fall through to "On failure" (Step 4.5). Do NOT fall through to "On pipeline failure" (that path requires `PIPELINE FAILURE:` prefix, which `error_max_turns` doesn't produce).
>
> **Handler — `recover_unpushed_work` (atomicity: metadata BEFORE send_message):**
>
> 1. Write `unpushed_recovery_pending: true` to `tasks.metadata` via `update_task_status` (status stays `in_progress`).
> 2. Emit `send_message` with structured payload: `verdict`, `task_id`, `branch`, `tip_sha`, `commit_count`, `turn_count_at_exhaustion` (if available), `claude_pilot_log_path`, `suggested_recovery_command`.
> 3. Do NOT increment `pipeline_retry_count`.
> 4. Do NOT call `run_claude_pilot` again for this task.
> 5. Proceed to Step 6 with `in_progress` and note "recover_unpushed_work: {commit_count} commits on local branch, awaiting operator recovery. Branch: {branch}".

**On pipeline failure (callback contains "PIPELINE FAILURE:"):**

1. Extract metadata (Session, Cost, Turns, Duration) from lines after the prefix.
2. Check `pipeline_retry_count` in metadata (default 0) via `check_task(task_id)`.
3. If `pipeline_retry_count >= 2`: escalate — notify Vincent "Pipeline failure: {repo}#{issue_number} produced no commits after {n} retries." Step 6 with `blocked`.
4. Retries remain: notify "Pipeline produced no commits for {repo}#{issue_number} — retrying ({n}/2)." `update_task_status` with same `in_progress` and `metadata: {"pipeline_retry_count": <current + 1>}`. Verify via `check_task`. Call `run_claude_pilot` with the same `repo#number` and `task_id`. If returns `{"status": "deferred", "deferred": true}`, retry is auto-enqueued — do NOT retry again. Step 6 with `in_progress`, note "pipeline retry deferred — engine will auto-dispatch when slot is free."

**On success (no "PIPELINE FAILURE:" prefix):**
1. Extract metadata and persist immediately.
2. If `pr_url` not in callback text, discover: `run_gh("pr list --head <branch> --repo senara-solutions/<repo> --json url --jq '.[0].url'")`. Update metadata with the URL.
3. Before treating PR as ready or attempting merge, check mergeable state: `run_gh("pr view <number> --repo senara-solutions/<repo> --json mergeable --jq '.mergeable'")`. If `CONFLICTING`, invoke `resolve_pr_conflicts`.
4. Notify Vincent: "claude-pilot completed for {repo}#{issue_number}. PR: {url}. QA will review automatically."
5. Step 6 with `in_progress`, note "PR open, awaiting QA review. PR: {url}". mika-qa is webhook-triggered — no delegation needed.

**On failure (non-zero exit, "FAILED", or "not structured JSON"):** Before blocking, **always check for a PR** via `run_gh("pr list --head <branch> --json url,number,state,reviewDecision")`. If a PR exists (especially if mika-qa-approved), the run succeeded — merge and close out normally. Only proceed to Step 4.5 if no PR exists.

**Completion callback result** — successful callback carries:
- `status`: `"completed"` (exit 0) or `"FAILED"` (non-zero exit)
- Structured fields on success: `session_id`, `turns`, `cost_usd`, `duration_ms`
- Last 10KB of stderr appended for debugging

**Step 4.5 — Diagnose and recover from failure**

1. **Read the log.** Extract log path from callback. `run_shell`:
   ```
   tail -200 /var/log/claude-pilot/<task-id>.log
   ```

2. **Classify:**

   | Pattern in log | Classification | Recoverable? |
   |----------------|---------------|--------------|
   | `relay disabled`, `missing .claude/claude-pilot.json` | Config missing in worktree | Yes |
   | `cargo clippy`, `cargo test`, `error[E`, `test result: FAILED` | Build failure | Yes |
   | `permission denied` (3+ times), `action: "deny"` cascade | Permission denial cascade | No |
   | `panic`, `thread .* panicked`, `SIGSEGV`, `SIGABRT` | Code panic / crash | No |

3. **Attempt recovery (max 1).** If already a retry, skip to escalation.
   - **Config missing:** copy config from meta-repo, retry same `run_claude_pilot` call.
   - **Build failure:** re-launch `run_claude_pilot` with free-text prompt including failure context.
   - **Permission cascade / panic:** do NOT retry. Escalate.
   - **HANDLER CRASH:** handler crashed before building result. Do NOT retry synchronously — direct calls fail without `__mika_task_id` injection. To retry, call `run_claude_pilot` normally (re-enters long-running executor).

4. **Escalate with specific context.** Notify Vincent:
   "Self-dev run failed for {repo}#{issue_number}: **{classification}** — {one-line detail from log}. Log: `/var/log/claude-pilot/<task-id>.log`"

**Step 5 — (removed: QA is webhook-driven)**

QA review triggers automatically on PR open/update/review_requested. Verdicts arrive via `pull_request_review.submitted` webhook, handled by the `self-dev-webhook-qa` skill. After claude-pilot creates a PR, proceed directly to Step 6 with `in_progress`.
