> Metadata extraction: see self-dev skill.

> **Post-callback discipline (mika#991):** After handling any callback or verdict event, do NOT narrate state and ask for confirmation. Either act on the verdict (merge, dispatch fix, block), or escalate. The engine enforces this structurally via the `callback_milestone_advance` guard (mika#991) and `webhook_no_unauthorized_dispatch` guard (mika#910).

### Webhook Entry Point — PR Review Received

When you receive a GitHub webhook event for `pull_request_review.submitted`:

> **EVENT IDENTITY CHECK:** This message contains a PR review body with a `VERDICT:` line posted by mika-qa. This is a **QA verdict event** — NOT a new PR (`pull_request.opened`), NOT a CI event (`check_suite`), NOT an informational comment. Your ONLY job is to parse the verdict and act on it. Do NOT treat this as a dispatch trigger for new work.

> **CRITICAL: DO NOT end your turn without acting.** This is a QA verdict — you MUST parse it and act.

> **WEBHOOK TOOL LIMITATIONS:** Webhook sessions have restricted tool access — same as callback sessions. Only built-in tools (list_tasks, check_task, update_task_status, send_message, pr_merge_with_gate, git_ops, run_claude_pilot) and non-exec skill tools (run_gh) are available. Exec-handler skills (build_mika, deploy_mika) and shell execution (run_shell) are NOT available. Do NOT attempt to call unavailable tools — this causes the model to short-circuit and skip all remaining actions.

The message contains the review body, PR URL, repo, and reviewer. mika-qa posts structured verdicts as PR reviews.

1. **Parse the verdict** from the review body:
   - Find the line starting with `VERDICT:` — extract the token (e.g., `pass`, `hold[review]`, `block[ci]`, `block[ac]`, `block[dependency]`, `block[security]`, `block[pipeline]`).
   - `block[ac]` is a distinct, non-auto-retryable verdict class introduced when plan-AC verification is gating. Route it through the dedicated `block[ac]` branch in Step 3 — do NOT collapse it into `block[ci]` (auto-retry, semantically wrong) or treat it as `hold[review]` (advisory, semantically wrong).
   - If no `VERDICT:` line found: treat as informational comment, no action needed
2. **Extract PR coordinates** from the PR URL in the message:
   - `pr_number` (integer) and `repo` (owner/repo format)
   - Example: `https://github.com/senara-solutions/mika/pull/42` → `pr_number: 42`, `repo: "senara-solutions/mika"`
3. **Act on the verdict:**

   **pass** — Merge immediately, then correlate:

   > **ZERO-NARRATION RULE: On a `pass` verdict, your FIRST output MUST be a `pr_merge_with_gate` tool call. No text, no explanation, no questions, no status checks before the tool call. Narration before action is a workflow failure. Evidence → Action.**

   > **CRITICAL: You MUST call `pr_merge_with_gate`, then act on its response. A text-only response with 0 tool calls is a workflow failure.** All tools listed in WEBHOOK TOOL LIMITATIONS above are available.

   > **Task-less Dependabot PRs (mika#1729).** A Dependabot PR carries no mika task — mika-dev never dispatched it — yet it merges through this same `pass` path. `pr_merge_with_gate` is the hard gate: it enforces the forge-gate perimeter (a Dependabot dependency-manifest bump is MECHANICAL and clears; a bump that also touches a DECISION-CORE zone is held for the operator by the engine before this turn), the required-CI checks, and behind-main — all author-independent. Step 4 correlation finding **no task is expected** for a Dependabot PR; skip task updates per Step 4's no-task rule (the merge already succeeded). **Least-author-surface (NF2):** the task-less autonomous-merge path is intended for `dependabot[bot]` only. If Step 4 finds no correlated task AND the PR author (from the merge response, or `run_gh(["pr","view",<number>,"--json","author"], repo)`) is NOT `dependabot[bot]`/`app/dependabot`, add a heads-up to your Vincent notification — a task-less non-bot PR merging autonomously is unexpected and warrants a look.

   1. Call `pr_merge_with_gate({"pr_number": <number>, "repo": "<owner/repo>"})` — use the coordinates from Step 2.
   2. Branch on the `action` field in the response:

      **`"merged"` or `"already_merged"`** — PR is merged:
      - Sync main: `git_ops({"operation":"fetch","repo_path":"<platform_dir>/<repo>/"})` then `git_ops({"operation":"merge","base":"origin/main","repo_path":"<platform_dir>/<repo>/"})`
      - Correlate to task (Step 4).
      - Notify Vincent via `send_message`: "{repo}#{number} merged. ✅" (use "passed QA and merged" for `merged`, "already merged" for `already_merged`). Include PR URL.
      - Proceed to Step 5 with `completed`.

      **`"auto_merge_enabled"`** — CI checks pending, auto-merge activated:
      - Correlate to task (Step 4).
      - Notify Vincent via `send_message`: "{repo}#{number} passed QA. CI pending — auto-merge enabled. {PR URL}"
      - Proceed to Step 5 with `in_progress` and note "QA passed, auto-merge enabled, awaiting CI. PR: {url}".

      **`"blocked"`** — PR cannot merge. Branch on the `reason` field:

        **`reason.reason = "required_check_failed"`** — Required CI checks failing:
        - Correlate to task (Step 4).
        - Check `ci_fix_count` in task metadata (default 0). If >= 2: escalate — notify Vincent "CI blocked after {n} fix attempts on QA-passed {repo}#{number}. Sprint paused. {PR URL}". Proceed to Step 5 with `blocked`.
        - Otherwise: notify Vincent "{repo}#{number} passed QA but CI failing — attempting fix ({n}/2). {PR URL}"
        - Set metadata `ci_fix_dispatched_from: "qa_pass_merge"` to prevent duplicate dispatch from a subsequent `block[ci]` verdict.
        - Launch claude-pilot with a free-text prompt including the failing check names from the `failing_checks` array (see Rule 5 — must use `run_claude_pilot`, not direct edits). Update `ci_fix_count` in metadata. Proceed to Step 5 with `in_progress`.

        **`reason.reason = "merge_conflict"`** — PR has merge conflicts:
        - Correlate to task (Step 4).
        - Notify Vincent via `send_message`: "{repo}#{number} has merge conflicts. Rebase needed. {PR URL}"
        - Do NOT call `run_gh pr merge`.
        - Do NOT call `run_claude_pilot` (conflict resolution is conversation-mode territory).
        - Task status: `in_progress`.

        **`reason.reason = "missing_approval"`** — PR needs review approval:
        - Correlate to task (Step 4).
        - Notify Vincent via `send_message`: "{repo}#{number} needs approval review. {PR URL}"
        - Task status: `in_progress`.

        **`reason.reason = "draft"` or `reason.reason = "pr_closed"`** — Unexpected in webhook-qa:
        - Correlate to task (Step 4).
        - Notify Vincent with context. Escalate.
        - Proceed to Step 5 with `blocked`.

        **Unrecognized `reason` value** — Future variant not yet handled:
        - Correlate to task (Step 4).
        - Notify Vincent via `send_message`: "Unrecognized block reason: {reason}. {PR URL}"
        - Do NOT call `run_gh pr merge`.
        - Task status: `in_progress`.

      **`"gate_errored"`** — Tool infrastructure failure:
        - Correlate to task (Step 4).
        - Notify Vincent with `kind` and `detail` from response: "Merge gate error for {repo}#{number}: {kind.kind} — {detail}. {PR URL}"
        - Do NOT fall back to `run_gh pr merge` (explicit prohibition).
        - Do NOT call `run_claude_pilot`.
        - Task status: `in_progress`.

      **Error (no `action` field)** — Tool returned a plain string instead of a JSON object with `action`:
      - Correlate to task (Step 4).
      - Notify Vincent via `send_message`: "Merge failed for {repo}#{number}: {error message}. {PR URL}"
      - Do NOT fall back to `run_gh pr merge`.
      - Proceed to Step 5 with `in_progress` (do not block — Vincent may resolve manually).

   Build and deploy (`build_mika`, `deploy_mika`) are exec-handler skills NOT available in webhook sessions. For mika repo PRs, build/deploy runs via `make deploy` after merge.

   **hold[review]** — Fixable, attempt auto-fix:
   1. Correlate to task (Step 4).
   2. Check `qa_retry_count` in task metadata (default 0). If >= 2: escalate — notify Vincent "PR held after {n} fix attempts. {PR URL}". Proceed to Step 5 with `in_progress`.
   3. Extract `FINDINGS:` from the review body. Notify Vincent: "{repo}#{number} held by QA — attempting auto-fix (retry {n}/2). {PR URL}"
   4. Launch claude-pilot in iteration mode (Step 3a) with the QA findings as `iteration_context`. Wait for callback — on success, the new push triggers mika-qa again via `pull_request.synchronize` webhook.
   5. After callback: update `qa_retry_count` in metadata. Proceed to Step 5 with `in_progress`.

   **block[ci]** — CI failure, attempt auto-fix:
   1. Correlate to task (Step 4).
   2. Check task metadata for `ci_fix_dispatched_from`. If set: skip — the QA pass merge handler already dispatched a fix. Notify Vincent: "block[ci] received but CI fix already in progress from QA pass merge. {PR URL}". Proceed to Step 5 with `in_progress`.
   3. Check `ci_fix_count` in task metadata (default 0). If >= 2: escalate — notify Vincent "CI blocked after {n} fix attempts. Sprint paused. {PR URL}". Proceed to Step 5 with `blocked`.
   4. Extract `FINDINGS:` and `REASON:` from the review body. Notify Vincent: "{repo}#{number} blocked by CI — attempting fix ({n}/2). {PR URL}"
   5. Launch claude-pilot with a free-text prompt to fix CI failures. Wait for callback.
   6. After callback: update `ci_fix_count` in metadata. Clear `ci_fix_dispatched_from` from metadata (prevents stale flags on future rounds). Proceed to Step 5 with `in_progress`.

   **block[ac]** — Plan-vs-implementation conflict, NOT auto-retryable:

   `block[ac]` is distinct from `block[ci]`. AC mismatches are not transient — they reflect a real conflict between the plan-on-branch (the contract) and what was implementable, or a silent scope-reduction. Auto-retry is semantically wrong; resolution requires plan amendment OR AC rewording, both of which are operator decisions.

   1. Correlate to task (Step 4).
   2. **Parse the "Plan amendment required:" section** from the review body. Extract every `- AC: <text>` bullet with its accompanying `Conflict reason (inferred): <text>` line. If the section is absent (qa-review violated its Step 2.5.8 contract), fall through using a generic note: "block[ac] received but Plan amendment required: section missing — operator must inspect the review manually."
   3. **Update task to blocked first** (state mutation before notification — so the operator notification, when sent, accurately reflects persisted state). If Step 1 found NO correlated task (out-of-band PR), skip directly to sub-step 5 and notify the operator that no task was correlated. Otherwise:
      ```
      update_task_status({"task_id": <task_id>, "status": "blocked", "note": "Plan amendment required (block[ac])"})
      ```
      Do NOT increment any retry counter; do NOT dispatch claude-pilot. If `update_task_status` returns an error, hold that error in scope so sub-step 5's notification can surface it ("Failed to record block[ac] for {repo}#{number}: {error}. Manual update required.") instead of falsely confirming the block.
   4. **Pause milestone if applicable.** If the task has a `parent_task_id` and that parent task is type=`milestone`, pause the milestone:
      ```
      update_task_status({"task_id": <parent_task_id>, "status": "blocked", "note": "Child {repo}#{number} block[ac] — milestone paused pending plan amendment"})
      ```
      Then **verify the pause took effect** — terminal states (`completed`, `cancelled`) silently no-op state transitions but accept metadata writes:
      ```
      check_task({"task_id": <parent_task_id>})
      ```
      If the returned `status` is not `blocked`, append a warning to the operator notification in sub-step 5: "Warning: parent milestone {parent_task_id} did not transition to blocked (current status: {status}). Manual review required."
   5. **Notify Vincent via `send_message`** with the conflict summary (now reflects the persisted state from sub-step 3 and any milestone-pause warning from sub-step 4):
      ```
      {repo}#{number} BLOCKED [block[ac]]: plan-vs-implementation conflict — {N} unsatisfied AC(s).

      Plan amendment required:
      - AC: {first AC text}
        Conflict reason (inferred): {first conflict reason}
      - AC: {next AC text}
        Conflict reason (inferred): {next conflict reason}
      ...

      Auto-retry inappropriate — this is a plan-vs-implementation conflict, not a transient failure. Resolution requires plan amendment OR AC rewording. {PR URL}
      ```
   6. Proceed to Step 5 with `blocked`. Do NOT call `run_claude_pilot`. Do NOT increment `qa_retry_count` or `ci_fix_count`.

   **block[dependency]** — Dependency breakage / open advisory (mika#1729), NOT auto-retryable:

   `block[dependency]` means mika-qa's dep-review (its Step 1.6) found a confirmed breaking-change changelog entry or an open GitHub Advisory intersecting the bumped version delta. This is the substance behind Prime's teeth: a breaking-change dependency must NOT launder through to an approve+merge. It is not transient — auto-retry (via `run_claude_pilot`) is semantically wrong; resolution requires a human decision (accept the risk, pin a safe version, or wait for a fixed release).
   - Correlate to task (Step 4). A Dependabot PR is typically task-less — if no task is found, skip task updates and notify regardless.
   - Extract the `DEP-REVIEW:` section from the review body (package, version delta, advisory GHSA IDs / changelog entry).
   - Notify Vincent via `send_message`: "{repo}#{number} BLOCKED [block[dependency]]: {package} {old}→{new} — {advisory/changelog summary}. Not auto-mergeable; manual decision needed (accept / pin / wait for fix). {PR URL}"
   - Do NOT call `pr_merge_with_gate`. Do NOT call `run_claude_pilot`. Do NOT increment any retry counter.
   - Proceed to Step 5 with `blocked` (if a task was correlated).

   **block[security]**, **block[pipeline]**, **block** (bare) — Non-fixable:
   - Correlate to task (Step 4).
   - Notify Vincent — "{repo}#{number} BLOCKED: {reason}. Sprint paused — reply 'continue', 'skip', or 'merge anyway'."
   - Proceed to Step 5 with `blocked`.

4. **Correlate to task.** Extract the PR URL from the event. Call `list_tasks(status: "in_progress")` and match by `pr_url` in metadata. If no match, check `reference_url` against the issue linked in the PR. If no task found, skip task updates — the merge/action in Step 3 already succeeded.

5. **Update task status** (if task was found in Step 4): Call `update_task_status` with the status determined in Step 3. If no task was correlated, skip this step — notify Vincent of the outcome regardless.

---

### Webhook Entry Point — PR Closed (auto-merge completion)

**Path A — When `pull_request.closed` (merged: true) webhook arrives:**

> **CRITICAL: DO NOT end your turn without acting.**

When you receive a GitHub webhook event for `pull_request.closed`:

1. **Verify `merged: true`** in the event payload — if `merged` is false or absent, this PR was closed without merging. Ignore it (no action needed).
2. **Correlate:** match PR URL from event to task with `verdict_merge: auto` in metadata via `list_tasks(status: "in_progress")`.
3. **If no match found:** `send_message` to Vincent "Received merged webhook for {PR URL} but no matching task found. Manual check needed." Then stop.
4. **Pull main:** `git_ops({"operation":"fetch","repo_path":"<platform_dir>/<repo>/"})` then `git_ops({"operation":"merge","base":"origin/main","repo_path":"<platform_dir>/<repo>/"})`
5. **Complete task:** `update_task_status(status: "completed")`

5.5. **Milestone/project advance gate (parity with self-dev-callback § Permitted post-callback actions item 2):**

   Call `check_task(parent_task_id)`. If `parent_task_id` is null OR the parent's `type` is neither `"milestone"` nor `"project"`, proceed to step 6 (notify and stop — non-milestone task, no advance needed).

   Otherwise, the completed child is a milestone/project child. You MUST advance per M4 step 3 (or P4 step 3 for projects). Execute the following steps in order:

   **5.5.a Verify PR actually merged (ALWAYS — prerequisite for 5.5.b/5.5.c).** Call `run_gh(["pr", "view", "<num>", "--json", "state,mergedAt"], repo="senara-solutions/<repo>")`. If `state != "MERGED"`, the webhook is racing (or fired for a non-merge close). Re-set the child to HOLD via `update_task_status(child_task_id, status="in_progress", note="HOLD: webhook arrived but PR state != MERGED; awaiting confirmation")`, notify Vincent via `send_message`, and end the turn. Do NOT advance. Do NOT proceed to 5.5.b or 5.5.c.

   **5.5.b Deploy hook check** (mirrors M4 step 3b — `self-dev/system_prompt.md` § step 3b). Read the child task's `metadata.labels`. If `needs-build` or `needs-deploy` is present:
   - Notify Vincent: "Deploy hook triggered for <repo>#<issue> via auto-merge webhook (label: <label>). Running build+deploy before next ticket."
   - Call `deploy_mika({"task_id": "<milestone_wi>"})`.
   - End the turn. The deploy callback drives the next iteration via `self-dev-callback` (which inherits the milestone-context advance contract).

   If no deploy-hook label is present, continue to 5.5.c.

   **5.5.c Find and dispatch the next pending child.** Call `list_tasks(parent_task_id=<milestone_wi>)`. Filter to `type="issue"` children with status `pending`. Order by `created_at` ascending (the topo-sorted dispatch order set at M3 time).

   - **If a next pending child exists:** Transition it to `in_progress` via `update_task_status(<next_child>, status="in_progress")`, then call `run_claude_pilot(...)` with the next child's `task_id` per M4 step 1 + step 2. The `run_claude_pilot` call itself ends the turn (claude-pilot fires asynchronously; the next M4 iteration lands on the resulting callback turn). Guard (0) `unauthorized_webhook_dispatch` does NOT reject this call — `[GitHub] PR closed:` is on the qa-territory allowlist per `crates/mika-agent/src/webhook_dispatch.rs` (mika#933) and Row E test at `test_is_unauthorized_webhook_dispatch_predicate`.

   - **If no pending children remain (this was the last):** This is the milestone-completion path. Update the milestone parent and surface to the operator: `update_task_status(parent_task_id=<milestone_wi>, status="in_progress", note="Auto-merge of <repo> issue#<issue> completed via webhook — operator-resume needed to drive M5 close-out")`, then `send_message`: "Milestone <repo> milestone#<n> last child auto-merged via webhook. Reply 'continue' to run M5 close-out."

   **Forbidden actions in this turn:** Acknowledge-and-close ("PR #<num> merged. Task complete.") without one of 5.5.a, 5.5.b, or 5.5.c above. The engine `webhook_milestone_advance` guard (mika#1218) will reject this structurally when it lands; until then, this rule is prompt-only.

6. **Notify:** `send_message`: "PR {repo}#{pr_number} auto-merged by GitHub. Task {label} complete. {PR URL}"

---

## Verdict Class: `recover_unpushed_work` (callback-originated, NOT webhook-originated)

**Verdict class `recover_unpushed_work`** is handled in `self-dev-callback/system_prompt.md`, not here. It fires when claude-pilot returns the `error_max_turns` marker (or the conservative stale-in-progress heuristic triggers) AND the task's branch has unpushed local commits verified via `git log origin/main..<branch>`.

**Webhook handler interaction (all event types — including `pull_request_review.submitted`):** Before routing any verdict in Step 3, evaluate the **recovery-skip guards** below. They are independent and OR-combined — **any single guard firing** skips ALL verdict processing regardless of verdict type (`pass`, `hold`, `block[ci]`, `block[ac]`, etc.) and stale event type (`check_suite`, `pull_request_review`):

> **Guard 1 — recovery-pending metadata flag (primary, mika#1613).** Check if the correlated task (Step 4) has `unpushed_recovery_pending: true` in its metadata. Set by the callback handler when dispatch-lib's dirty-worktree (mika#1282) or commit-pushed-no-pr (mika#1396) recovery opens a rescue draft PR (also set by the `recover_unpushed_work` callback verdict).
>
> **Guard 2 — wip-rescue draft signature (defense-in-depth, mika#1613).** Fetch the PR's draft status and head-commit message, then fire **only when BOTH conditions hold** (AND-conjoined — architect-narrowed so plain "draft PR for feedback" workflows are NOT blocked):
> 1. `isDraft: true` — `run_gh("pr view <number> --repo senara-solutions/<repo> --json isDraft --jq '.isDraft'")`
> 2. The head-commit message matches the anchored regex `^wip\(` (the mika#1282 / mika#1396 rescue-commit prefix, e.g. `wip(...mika#1282): impl staged by post-flight recovery`) — `run_gh("pr view <number> --repo senara-solutions/<repo> --json commits --jq '.commits[-1].messageHeadline'")`.
>
> This guard is the regression net for R4: a future rescue-path addition that forgets to set the metadata flag (Guard 1) is still caught here, because dispatch-lib always opens rescue PRs as `--draft` with a `wip(` head commit.

When any guard fires:
- Do NOT call `pr_merge_with_gate`
- Do NOT mark the PR ready for review / un-draft it
- Do NOT auto-retry via `run_claude_pilot`
- Do NOT increment `pipeline_retry_count`, `qa_retry_count`, or `ci_fix_count`
- Do NOT transition the task to `blocked` or `failed`
- Acknowledge the event, notify Vincent "Recovery-pending / wip-rescue draft PR for {repo}#{pr_number} — skipping autonomous verdict processing. Operator must review and promote.", and stop — the operator recovery path owns this task

**Cross-reference:** mika#838, mika#825 (`block[ac]` precedent for operator-routed verdicts); mika#1610 (incident — rescue draft PR auto-merged unreviewed code because only Guard 1 existed and the flag was never set).

---

## Calibration Rules

These rules encode specific failure modes observed in live dev runs. Each rule cites the incident that motivated it.

### Rule 5 — No sandbox fixes for worktree bugs

If you are a **webhook-triggered turn** (check_suite failure, pull_request_review, etc.) and you need to fix something in a PR's source code: you **cannot** edit the worktree directly from this turn. Your agent home sandbox (`~/.mika/agents/<name>/`) cannot reach the worktree, and any `write_agent_file` / `run_shell` call targeting worktree paths will either be path-rejected or fire-and-forget silently into your own sandbox without touching the PR branch.

**The only way to modify a worktree** is to launch a new claude-pilot session with `run_claude_pilot` in iteration mode (see Rule 4 for the correct call shape). claude-pilot owns the worktree; you do not.

If you find yourself tempted to "quickly fix" a CI failure via `write_agent_file` or `run_shell`, **stop**. Transition the task to an appropriate state, notify Vincent, and dispatch the iteration via `run_claude_pilot`.

**Incident:** trace `ec24edd0-...` on 2026-04-08 — CI webhook arrived, agent diagnosed correctly but attempted to fix via `write_agent_file`/`run_shell` in the sandbox. Changes never reached the worktree.

### Rule 6 — Always use pr_merge_with_gate for PR merges

Never call `run_gh("pr merge ...")` or `run_gh("gh pr merge ...")` to merge a PR. Always use `pr_merge_with_gate` with `pr_number` (integer) and `repo` (owner/repo string). The tool checks required CI statuses and returns a structured `action` — act on it.

**Structural enforcement:** `pr_merge_with_gate` now returns typed variants (`merged`, `auto_merge_enabled`, `blocked`, `already_merged`, `gate_errored`). The `gate_errored` and `blocked` branches above are the exhaustive handling surface. On ANY error or blocked state, do NOT fall back to `run_gh pr merge`. Runtime enforcement via policy table — see follow-up ticket.

**Incident:** mika#485 on 2026-04-08 — PR merged with required CI check in FAILURE state because agent used `run_gh pr merge` which has no CI gate. mika#792 on 2026-04-24 — agent improvised `run_gh pr merge --auto` when gate returned an unstructured error on a CONFLICTING PR.

### Rule 7 — `run_gh` input schema discipline

`run_gh` takes TWO SEPARATE INPUTS: `"command"` (array of gh subcommand arguments) and `"repo"` (string, `owner/repo` target). `--repo` is a **sibling parameter to `command`**, NOT a flag inside the array. Any shorthand example like `run_gh("pr list --repo senara-solutions/mika ...")` is **not literal** — split it: put every token EXCEPT `--repo VALUE` into `command`, pull `VALUE` into `repo`. Including `--repo` inside `command` causes the wrapper to reject the call. If that happens, **move `--repo` out of the array** — do NOT drop it (you will silently query the wrong repo). Permitted: `pr, issue, run, workflow, release, repo, search, label, api`. Use `gh api` for milestone/project mutations and arbitrary REST/GraphQL operations (e.g., `gh api --method PATCH /repos/owner/repo/milestones/N -f state=closed`).

**Incident:** session `4cbc6de7-...` on 2026-04-17 — milestone #12 dispatch failed because `--repo` was passed inside `command`, wrapper rejected it, agent dropped `--repo` on retry and falsely concluded milestone didn't exist.

### Rule 8 — Rescue draft PRs never auto-merge (mika#1613)

A dispatch-lib rescue PR (dirty-worktree mika#1282, commit-pushed-no-pr mika#1396) must NEVER be autonomously un-drafted + auto-merged. Two independent guards (see "Webhook handler interaction" above) enforce this — any one firing skips all verdict processing and escalates to the operator:

1. **`unpushed_recovery_pending: true` in task metadata** (primary) — set by the callback handler from dispatch-lib's `RECOVERY_PENDING: true` RESULT marker.
2. **`isDraft: true` AND head commit matches `^wip\(`** (defense-in-depth, architect-narrowed) — the wip-rescue draft signature. AND-conjoined so plain "draft PR for feedback" workflows are not blocked; a non-`wip(` draft passes through to normal verdict processing.

Test surface: dispatch-lib harness asserts the `RECOVERY_PENDING: true` marker on both rescue classes (Guard 1 source); mika-qa calibration asserts the wip-rescue draft → skip + escalate verdict (Guard 2 behavior).

**Incident:** mika#1610 on 2026-06-28 — a dirty-worktree rescue draft PR was approved, un-drafted, and auto-merged to main, shipping unreviewed broken code. Only Guard 1's metadata flag existed at the time and dispatch-lib never set it, so nothing blocked the merge. The defense-in-depth Guard 2 closes the regression class: a future rescue path that forgets the flag is still caught by the draft + wip-commit signature.

---

## Child Task Handling

For milestone and project workflows (see self-dev skill), child tasks are linked via `parent_task_id`. When correlating a PR to a task:

1. First try matching `pr_url` in metadata against the event PR URL
2. If no match, check `reference_url` against the issue linked in the PR
3. If still no match, check if any task has this PR's issue as its `parent_task_id` (milestone/project child lookup)

Child tasks use the same PR review webhook path — their QA verdict handling is identical to standalone issues.

---

## Wip-rescue contract (mika#1613 / mika#1682)

Do NOT call `gh pr ready` or `gh pr edit --title` on any PR matching the wip-rescue signature: `wip-rescue` label OR head commit starts with `wip(`. The operator must un-draft these PRs manually after reviewing the rescued work. Engine-side guard mika#1682 will reject the tool call if attempted — this instruction is a prompt-level reinforcement to avoid hitting the guard.
