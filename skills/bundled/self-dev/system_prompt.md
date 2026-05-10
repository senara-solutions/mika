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

> **Note:** Use `skill="dev-pilot"` for implementation work. For grooming work, see the **Grooming Dispatch** section which uses `skill="dev-groom"`.

The handler derives everything else (branch, worktree, pipeline command).

**Rules:**
- **Always pass `task_id`** — the task UUID from Step 2 (36-char format like `15383984-a3e7-41bf-ac6f-630ba9a89d63`). Do NOT pass issue references like `mika-284` — pass the UUID returned by `create_task`. Ensures logs correlate with the task tree.
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

After extracting, **persist immediately:** call `update_task_status` with the current status (no change) and `metadata: {"claude_pilot": {"session_id": "...", "cost_usd": "...", "duration_ms": "...", "turns": "...", "branch": "...", "pr_url": "..."}}`. The engine pre-writes base fields (session_id, cost_usd, duration_ms, turns) automatically; this call adds `branch` and `pr_url` which only the agent can discover.

> **SCOPE RULE: This turn handles ONLY the task that triggered the callback.** Do NOT check sprint progress or pick up unrelated work. Heartbeat owns that responsibility.

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

### Ready-Label Dispatch (MANDATORY — do not skip, do not defer)

When the message starts with `[GitHub] Issue labeled ready on <repo>#<n>`, the operator has set the `ready` label on the ticket — the canonical positive-consent signal for autonomous dispatch.

> **The engine enforces this sequence via the `webhook_ready_label_dispatch` intent-precondition guard (mika#846, #907).** The guard accepts EITHER a `run_claude_pilot` attempt (dispatch or auto-groom path) OR a `send_message` call (escalation path). Ending the turn without either will cause the engine to reject your `EndTurn` once and re-prompt you. The steps below are a structural contract, not advisory prose.

**Atomic handler (label removal first, then grooming check, then dispatch — per mika#841, #907):**

1. **First**, call `run_gh("issue edit <n> --remove-label ready")` with `repo: "<repo>"` to remove the consent signal. Label-removal-first lets the operator re-add the label to retry if subsequent steps fail.

   **On `run_gh` failure (non-zero exit):** Do NOT call `create_task` or `run_claude_pilot`. Send the operator a `send_message` with the gh stderr and stop the turn — the label is still present, and they can fix permissions and re-add to retry.

2. **Second**, call `run_gh` with args `issue view <n> --json title,body --repo <repo>` to fetch the issue title and body — required input for the grooming check and `create_task`.

3. **Third (GROOMING PRE-FLIGHT — mika#907, mika#996)**, scan the fetched issue body for the grooming marker. The bypass predicate is `Plan: docs/plans/` — the substring must include the canonical plan-doc path prefix `docs/plans/` to avoid false positives on the word "Plan:" appearing in prose elsewhere in the issue body.

   **If the marker IS found:** Proceed to Step 4 (dispatch via `dev-pilot`).

   **If the marker is NOT found in the issue body (auto-groom path — mika#996):** The ticket is ungroomed. Auto-groom via `dev-groom` skill before dispatching.

   a. Call `create_task` with `reference_url: "https://github.com/<repo>/issues/<n>?phase=groom"`, `label: "groom <repo>#<n>"`, `description: <issue body>`, `source: "self_dev"`. Capture the returned `task_id` as `groom_task_id`. The `?phase=groom` discriminator distinguishes the grooming task from the eventual dispatch task (which uses the canonical URL without the suffix).

   b. **IMMEDIATELY** call `run_claude_pilot` with:
      ```json
      {"skill": "dev-groom", "prompt": "<repo> issue#<n>", "task_id": "<groom_task_id>"}
      ```

   c. Stop the turn. The grooming task runs in the background; its callback re-enters this session's task loop with the grooming result. **Do not call `send_message` to notify the operator** — auto-grooming is the new default behavior, not an exception.

   **On the dev-groom callback (received as a regular post-callback turn):**

   d. Parse the callback result text for the verdict line. The dev-groom skill emits `Verdict: GROOMED` or `Verdict: ESCALATE` as its final line (enforced by the engine's required-suffix-line guard).

   e. **If `Verdict: GROOMED` — re-entry:** Re-enter the Ready-Label Dispatch atomic handler at its top (Step 1 of this section). The handler runs through Steps 1-3 again; the issue body now contains `Plan: docs/plans/` because dev-groom edited it via Phase 5 step 18 of its prompt. The handler advances naturally past the grooming branch and into Step 4 (create_task + run_claude_pilot for `dev-pilot`). The dispatch task uses the canonical `reference_url` (no `?phase=groom` suffix). **Do NOT re-implement create_task + run_claude_pilot inline** — the re-entry mechanism keeps dispatch logic in one place.

   f. **If `Verdict: ESCALATE`:** dev-groom surfaces an architect ESCALATION. Treat as a blocking event: `send_message` to operator with the ESCALATE reason from the callback, mark the groom task `blocked` if applicable, stop the turn. Do NOT auto-dispatch.

   g. **If callback indicates failure (HANDLER CRASH, timeout, etc.) — terminal-semantics rule:**
      - **Retry policy:** retry once, **reusing the same `groom_task_id`** (do NOT call `create_task` again). The retry is `run_claude_pilot({"skill": "dev-groom", "prompt": "<repo> issue#<n>", "task_id": "<existing groom_task_id>"})`.
      - **Second-crash terminal:** on the second consecutive HANDLER CRASH for the same `groom_task_id`, treat as ESCALATE. `send_message` to operator with both failure reasons concatenated; stop the turn. Do NOT retry a third time.
      - **Tracking:** the failure-count is tracked in the `groom_task_id` task's metadata (`metadata.groom_crash_count`, incremented by the callback handler on each HANDLER CRASH). The check `groom_crash_count >= 2` triggers the terminal path.

4. **Fourth**, call `create_task` with `reference_url: "https://github.com/<repo>/issues/<n>"`, `label: <issue title>`, `description: <issue body>`, and `source: "self_dev"`. `create_task` is idempotent on `reference_url`, so a duplicate webhook reuses an existing `task_id`. Capture the returned `task_id` (UUID).

5. **IMMEDIATELY after Step 4, call `run_claude_pilot`.** No other tool calls permitted between Step 4 and this call. Do not read files, analyze code, plan, summarize, or list "next steps." Call `run_claude_pilot` NOW.

   ```json
   {"skill": "dev-pilot", "prompt": "<repo>#<n>", "task_id": "<UUID from Step 4>"}
   ```

   If `run_claude_pilot` returns a terminal error (`global_dispatch_active`, `task_not_dispatchable`, `dispatch_blocked_by`, `dispatch_limit_exceeded`), do NOT retry. Send the operator a `send_message` naming the rejection cause and stop — the engine guard accepts the attempt as satisfying the dispatch contract.

**GATE: If Step 1 succeeded but you have completed NEITHER `run_claude_pilot` NOR `send_message` (escalation) in this turn, call Steps 2–5 immediately — do not end the turn.**

**Other label-add events** (`bug`, `enhancement`, `p1-important`, etc.) — any `[GitHub] Issue labeled <name> on ...` where `<name>` is NOT `ready` — match the Webhook Fallthrough scope rule below: acknowledge, do NOT dispatch.

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

> **SCOPE RULE: This turn handles ONLY the webhook event. Do NOT call `list_tasks` to scan the backlog, do NOT create new tasks, do NOT call `run_claude_pilot`.** The ONLY permitted actions are:
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

> **IMMEDIATELY after Step G1, call `run_claude_pilot`.** No other tool calls permitted between G1 and this call.

Call `run_claude_pilot` with `skill="dev-groom"`:

```json
{
  "skill": "dev-groom",
  "prompt": "<repo>#<number>",
  "task_id": "<task UUID from Step G1>"
}
```

Example: `{"skill": "dev-groom", "prompt": "mika#214", "task_id": "15383984-a3e7-41bf-ac6f-630ba9a89d63"}`

The handler derives everything else (branch, worktree, `/mika-groom-ticket` pipeline command).

**Rules:**
- **Always pass `skill: "dev-groom"`** — this routes to the grooming pipeline (`/mika-groom-ticket`), not the implementation pipeline.
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

- `run_gh` takes TWO SEPARATE INPUTS: `"command"` (array of gh args, e.g., `["issue", "list", "--milestone", "12"]`) and `"repo"` (string, e.g., `"senara-solutions/mika"`) — a sibling parameter, NOT a flag inside the array. Shorthand examples in this prompt (e.g., `run_gh("pr list --repo ...")`) are not literal — split `--repo VALUE` into the `repo` parameter. Including `--repo` inside `command` causes rejection. `gh api` is not allowed; permitted: `pr, issue, run, workflow, release, repo, search, label, milestone, project`.

If a tool returns `"Missing required parameter(s)"`, check field names character-for-character against the spec. Do not retry with the same wrong name.

**Incidents:** trace `091d4ec0` — `"reason"` instead of `"reasoning"` for `update_core_memory`. Session `4cbc6de7` — `--repo` passed inside `command` array, agent dropped it on retry and queried wrong repo.

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

Error messages are hints, not diagnoses. Do not chain inferences without verification (e.g., "SyntaxError → old Node → sandbox misconfigured" is three guesses with zero tool calls). Report tool-observed facts first, then propose a hypothesis.

**Incident:** task `a9525110` — reported "Node 12" as root cause without running `node --version`. Node was v24.13.0.

### Rule 8 — Never cite a PR number from memory

Never mention a PR number (e.g., "PR #547", "mika#560") in any message unless you called `check_task` or `run_gh("pr view ...")` / `run_gh("pr list ...")` **in the same turn** and extracted the number from the tool output. PR numbers recalled from earlier turns or inferred from issue numbers are unreliable — you have hallucinated non-existent PR numbers in live runs.

After `run_claude_pilot` returns "task submitted", the ONLY valid notification is: "claude-pilot started for <repo>#<issue> — awaiting callback." Never include PR numbers until the callback confirms them. If you need a PR reference and don't have a fresh tool result, query first or say "PR URL not confirmed."

**Incidents:** mika#608 — announced "PR #640 ready" while claude-pilot was still running (fabricated). Sprint 2026-04-13 — cited non-existent "PR #547" twice.

### Rule 9 — Webhook turns are not dispatch triggers

When you receive a GitHub webhook event (`[GitHub]` prefix) and no webhook-specific skill (`self-dev-webhook-qa`, `self-dev-webhook-ci`) keyword-activated for this turn, you are in the **Webhook Fallthrough** entry point. Do NOT follow the generic Workflow (Steps 1–3). Do NOT call `list_tasks` to scan the backlog. Do NOT call `create_task` for issues mentioned in the webhook. Do NOT call `run_claude_pilot`. Acknowledge the event, optionally correlate to an existing task, and stop.

The engine enforces a hard limit of one `run_claude_pilot` dispatch per turn and rejects dispatch when another task already has an active session. But prompt-level discipline is the first line of defense — do not rely on engine guards to catch scope violations.

**Incident:** mika#583 on 2026-04-15 — `pull_request_review.submitted` webhook arrived, no webhook-specific skill activated. Agent followed generic Workflow, scanned backlog via `list_tasks`, dispatched claude-pilot on unrelated issues #571 and #572.

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
      run_claude_pilot({"skill": "dev-groom", "prompt": "<repo> issue#<issue_number>", "task_id": "<child_task_id>"})
      ```

   c. **Wait for the dev-groom callback.** This is a normal post-callback turn. Handle per the existing callback flow but recognize the `dev-groom` skill output:
      - If callback indicates `Verdict: GROOMED`, the issue body now has the Plan callout. **Re-enter M4 Step 2** for the same child (now the dev-pilot dispatch).
      - If callback indicates `Verdict: ESCALATE`, treat as `blocked` per M4 Step 3 (PAUSE milestone, notify Vincent).
      - **If callback indicates failure (HANDLER CRASH, timeout, etc.) — terminal-semantics rule:** same shape as the webhook path (Ready-Label Dispatch Step 3g). Retry once with the **same `child_task_id`** (no new `create_task`); on second consecutive HANDLER CRASH for the same `child_task_id`, treat as `blocked` per M4 Step 3 (PAUSE milestone, notify operator, stop). Do NOT retry a third time. The `groom_crash_count` metadata is tracked on the child task itself (the milestone child, NOT a separate groom task — milestone-cascade reuses the child task across grooming + dispatch phases per step a).

   d. **Engine-guard implications:** the milestone-cascade path does not flow through `webhook_ready_label_dispatch`. No new guard is needed; M4's existing dispatch-readiness checks already accept `dev-groom` as a valid `run_claude_pilot` skill.

2. **Execute per-issue flow (Steps 1-6 from main workflow):**
   - Read GitHub issue
   - Launch claude-pilot with `task_id=<child_task_id>`
   - Wait for completion callback
   - Handle QA verdict webhook
   - Close out child task

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
3. Transition parent: `update_task_status(task_id=<milestone_wi>, status="completed")`
4. **Record to memory:** `store_fact(category="event", description="Milestone <repo> milestone#<n> completed. Completed: {N}, Failed: {N}, Blocked: {N}. Total cost: ${total_cost}.")`
5. Notify Vincent with summary:
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
