> Metadata extraction: see self-dev skill.

### Webhook Entry Point — PR Review Received

When you receive a GitHub webhook event for `pull_request_review.submitted`:

> **EVENT IDENTITY CHECK:** This message contains a PR review body with a `VERDICT:` line posted by mika-qa. This is a **QA verdict event** — NOT a new PR (`pull_request.opened`), NOT a CI event (`check_suite`), NOT an informational comment. Your ONLY job is to parse the verdict and act on it. Do NOT treat this as a dispatch trigger for new work.

> **CRITICAL: DO NOT end your turn without acting.** This is a QA verdict — you MUST parse it and act.

> **WEBHOOK TOOL LIMITATIONS:** Webhook sessions have restricted tool access — same as callback sessions. Only built-in tools (list_work_items, check_work_item, update_work_item_status, send_message, pr_merge_with_gate, git_ops, run_claude_pilot) and non-exec skill tools (run_gh) are available. Exec-handler skills (build_mika, deploy_mika) and shell execution (run_shell) are NOT available. Do NOT attempt to call unavailable tools — this causes the model to short-circuit and skip all remaining actions.

The message contains the review body, PR URL, repo, and reviewer. mika-qa posts structured verdicts as PR reviews.

1. **Parse the verdict** from the review body:
   - Find the line starting with `VERDICT:` — extract the token (e.g., `pass`, `hold[review]`, `block[ci]`)
   - If no `VERDICT:` line found: treat as informational comment, no action needed
2. **Extract PR coordinates** from the PR URL in the message:
   - `pr_number` (integer) and `repo` (owner/repo format)
   - Example: `https://github.com/senara-solutions/mika/pull/42` → `pr_number: 42`, `repo: "senara-solutions/mika"`
3. **Act on the verdict:**

   **pass** — Merge immediately, then correlate:

   > **ZERO-NARRATION RULE: On a `pass` verdict, your FIRST output MUST be a `pr_merge_with_gate` tool call. No text, no explanation, no questions, no status checks before the tool call. Narration before action is a workflow failure. Evidence → Action.**

   > **CRITICAL: You MUST call `pr_merge_with_gate`, then act on its response. A text-only response with 0 tool calls is a workflow failure.** All tools listed in WEBHOOK TOOL LIMITATIONS above are available.

   1. Call `pr_merge_with_gate({"pr_number": <number>, "repo": "<owner/repo>"})` — use the coordinates from Step 2.
   2. Branch on the `action` field in the response:

      **`"merged"` or `"already_merged"`** — PR is merged:
      - Sync main: `git_ops({"operation":"fetch","repo_path":"<platform_dir>/<repo>/"})` then `git_ops({"operation":"merge","base":"origin/main","repo_path":"<platform_dir>/<repo>/"})`
      - Correlate to work item (Step 4).
      - Notify Vincent via `send_message`: "{repo}#{number} merged. ✅" (use "passed QA and merged" for `merged`, "already merged" for `already_merged`). Include PR URL.
      - Proceed to Step 5 with `completed`.

      **`"auto_merge_enabled"`** — CI checks pending, auto-merge activated:
      - Correlate to work item (Step 4).
      - Notify Vincent via `send_message`: "{repo}#{number} passed QA. CI pending — auto-merge enabled. {PR URL}"
      - Proceed to Step 5 with `in_progress` and note "QA passed, auto-merge enabled, awaiting CI. PR: {url}".

      **`"blocked"`** — Required CI checks failing:
      - Correlate to work item (Step 4).
      - Check `ci_fix_count` in work item metadata (default 0). If >= 2: escalate — notify Vincent "CI blocked after {n} fix attempts on QA-passed {repo}#{number}. Sprint paused. {PR URL}". Proceed to Step 5 with `blocked`.
      - Otherwise: notify Vincent "{repo}#{number} passed QA but CI failing — attempting fix ({n}/2). {PR URL}"
      - Set metadata `ci_fix_dispatched_from: "qa_pass_merge"` to prevent duplicate dispatch from a subsequent `block[ci]` verdict.
      - Launch claude-pilot with a free-text prompt including the failing check names from the `pr_merge_with_gate` response (see Rule 5 — must use `run_claude_pilot`, not direct edits). Update `ci_fix_count` in metadata. Proceed to Step 5 with `in_progress`.

      **Error (no `action` field)** — Tool returned a plain string instead of a JSON object with `action`:
      - Correlate to work item (Step 4).
      - Notify Vincent via `send_message`: "Merge failed for {repo}#{number}: {error message}. {PR URL}"
      - Proceed to Step 5 with `in_progress` (do not block — Vincent may resolve manually).

   Build and deploy (`build_mika`, `deploy_mika`) are exec-handler skills NOT available in webhook sessions. For mika repo PRs, build/deploy runs via `make deploy` after merge.

   **hold[review]** — Fixable, attempt auto-fix:
   1. Correlate to work item (Step 4).
   2. Check `qa_retry_count` in work item metadata (default 0). If >= 2: escalate — notify Vincent "PR held after {n} fix attempts. {PR URL}". Proceed to Step 5 with `in_progress`.
   3. Extract `FINDINGS:` from the review body. Notify Vincent: "{repo}#{number} held by QA — attempting auto-fix (retry {n}/2). {PR URL}"
   4. Launch claude-pilot in iteration mode (Step 3a) with the QA findings as `iteration_context`. Wait for callback — on success, the new push triggers mika-qa again via `pull_request.synchronize` webhook.
   5. After callback: update `qa_retry_count` in metadata. Proceed to Step 5 with `in_progress`.

   **block[ci]** — CI failure, attempt auto-fix:
   1. Correlate to work item (Step 4).
   2. Check work item metadata for `ci_fix_dispatched_from`. If set: skip — the QA pass merge handler already dispatched a fix. Notify Vincent: "block[ci] received but CI fix already in progress from QA pass merge. {PR URL}". Proceed to Step 5 with `in_progress`.
   3. Check `ci_fix_count` in work item metadata (default 0). If >= 2: escalate — notify Vincent "CI blocked after {n} fix attempts. Sprint paused. {PR URL}". Proceed to Step 5 with `blocked`.
   4. Extract `FINDINGS:` and `REASON:` from the review body. Notify Vincent: "{repo}#{number} blocked by CI — attempting fix ({n}/2). {PR URL}"
   5. Launch claude-pilot with a free-text prompt to fix CI failures. Wait for callback.
   6. After callback: update `ci_fix_count` in metadata. Clear `ci_fix_dispatched_from` from metadata (prevents stale flags on future rounds). Proceed to Step 5 with `in_progress`.

   **block[security]**, **block[pipeline]**, **block** (bare) — Non-fixable:
   - Correlate to work item (Step 4).
   - Notify Vincent — "{repo}#{number} BLOCKED: {reason}. Sprint paused — reply 'continue', 'skip', or 'merge anyway'."
   - Proceed to Step 5 with `blocked`.

4. **Correlate to work item.** Extract the PR URL from the event. Call `list_work_items(status: "in_progress")` and match by `pr_url` in metadata. If no match, check `reference_url` against the issue linked in the PR. If no work item found, skip work item updates — the merge/action in Step 3 already succeeded.

5. **Update work item status** (if work item was found in Step 4): Call `update_work_item_status` with the status determined in Step 3. If no work item was correlated, skip this step — notify Vincent of the outcome regardless.

---

### Webhook Entry Point — PR Closed (auto-merge completion)

**Path A — When `pull_request.closed` (merged: true) webhook arrives:**

> **CRITICAL: DO NOT end your turn without acting.**

When you receive a GitHub webhook event for `pull_request.closed`:

1. **Verify `merged: true`** in the event payload — if `merged` is false or absent, this PR was closed without merging. Ignore it (no action needed).
2. **Correlate:** match PR URL from event to work item with `verdict_merge: auto` in metadata via `list_work_items(status: "in_progress")`.
3. **If no match found:** `send_message` to Vincent "Received merged webhook for {PR URL} but no matching work item found. Manual check needed." Then stop.
4. **Pull main:** `git_ops({"operation":"fetch","repo_path":"<platform_dir>/<repo>/"})` then `git_ops({"operation":"merge","base":"origin/main","repo_path":"<platform_dir>/<repo>/"})`
5. **Complete work item:** `update_work_item_status(status: "completed")`
6. **Notify:** `send_message`: "PR {repo}#{pr_number} auto-merged by GitHub. Task {label} complete. {PR URL}"

---

## Calibration Rules

These rules encode specific failure modes observed in live dev runs. Each rule cites the incident that motivated it.

### Rule 5 — No sandbox fixes for worktree bugs

If you are a **webhook-triggered turn** (check_suite failure, pull_request_review, etc.) and you need to fix something in a PR's source code: you **cannot** edit the worktree directly from this turn. Your agent home sandbox (`~/.mika/agents/<name>/`) cannot reach the worktree, and any `write_agent_file` / `run_shell` call targeting worktree paths will either be path-rejected or fire-and-forget silently into your own sandbox without touching the PR branch.

**The only way to modify a worktree** is to launch a new claude-pilot session with `run_claude_pilot` in iteration mode (see Rule 4 for the correct call shape). claude-pilot owns the worktree; you do not.

If you find yourself tempted to "quickly fix" a CI failure via `write_agent_file` or `run_shell`, **stop**. Transition the work item to an appropriate state, notify Vincent, and dispatch the iteration via `run_claude_pilot`.

**Incident:** trace `ec24edd0-...` on 2026-04-08 — CI webhook arrived, agent diagnosed correctly but attempted to fix via `write_agent_file`/`run_shell` in the sandbox. Changes never reached the worktree.

### Rule 6 — Always use pr_merge_with_gate for PR merges

Never call `run_gh("pr merge ...")` or `run_gh("gh pr merge ...")` to merge a PR. Always use `pr_merge_with_gate` with `pr_number` (integer) and `repo` (owner/repo string). The tool checks required CI statuses and returns a structured `action` — act on it.

**Incident:** mika#485 on 2026-04-08 — PR merged with required CI check in FAILURE state because agent used `run_gh pr merge` which has no CI gate.
