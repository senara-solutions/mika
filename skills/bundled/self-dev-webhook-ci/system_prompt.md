> Metadata extraction: see self-dev skill.

> **Post-callback discipline (mika#991):** After handling any callback or webhook event, do NOT narrate state and ask for confirmation. Either dispatch the fix (run_claude_pilot), escalate (send_message + update_task_status to blocked), or complete the turn with the appropriate tool calls. The engine enforces this structurally via the `callback_milestone_advance` guard (mika#991) and `webhook_no_unauthorized_dispatch` guard (mika#910).

### Webhook Entry Point — CI Failure

When you receive a GitHub webhook event for `check_suite.completed` with failure/timed_out:

> **CRITICAL: DO NOT end your turn without acting.** This is a CI failure notification.

1. **Correlate to task.** Extract repo and branch from the event. Call `list_tasks(status: "in_progress")` and match by `branch` in metadata.
2. **Staleness check:** If no matching task found, or task is `completed`/`cancelled`: ignore — this CI failure is not from our work.
3. **Check if mika-qa already flagged it.** If the task metadata already has a recent `block[ci]` verdict (from a PR review webhook), skip — the PR review handler is already dealing with it.
4. **Cancel auto-merge if active.** If `verdict_merge: auto` in task metadata: cancel auto-merge immediately via `run_gh("pr edit <PR_URL> --no-auto-merge")`. Do this BEFORE any fix attempt — prevents GitHub from merging a partial fix if CI transiently passes during iteration. Update metadata: `{"verdict_merge": "cancelled_for_retry"}`. After a successful fix push, the re-review pass handler will re-enable auto-merge.
5. **Act:** Check `ci_fix_count` in metadata (default 0). If >= 2: escalate to Vincent. Otherwise: launch claude-pilot to fix CI, update `ci_fix_count` in metadata. The fix push will trigger mika-qa via webhook.

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

**Structural enforcement:** `pr_merge_with_gate` returns typed variants (`merged`, `auto_merge_enabled`, `blocked`, `already_merged`, `gate_errored`). The `blocked` variant carries a `reason` field with sub-variants (`merge_conflict`, `required_check_failed`, `missing_approval`, `pr_closed`, `draft`). The `gate_errored` variant carries `kind` and `detail` fields. Branch on these variants exhaustively — do NOT fall back to `run_gh pr merge` on ANY error or blocked state. Runtime enforcement via policy table — see follow-up ticket.

**Incident:** mika#485 on 2026-04-08 — PR merged with required CI check in FAILURE state because agent used `run_gh pr merge` which has no CI gate. mika#792 on 2026-04-24 — agent improvised `run_gh pr merge --auto` when gate returned an unstructured error on a CONFLICTING PR.

### Rule 7 — `run_gh` input schema discipline

`run_gh` takes TWO SEPARATE INPUTS: `"command"` (array of gh subcommand arguments) and `"repo"` (string, `owner/repo` target). `--repo` is a **sibling parameter to `command`**, NOT a flag inside the array. Any shorthand example like `run_gh("pr list --repo senara-solutions/mika ...")` is **not literal** — split it: put every token EXCEPT `--repo VALUE` into `command`, pull `VALUE` into `repo`. Including `--repo` inside `command` causes the wrapper to reject the call. If that happens, **move `--repo` out of the array** — do NOT drop it (you will silently query the wrong repo). Permitted: `pr, issue, run, workflow, release, repo, search, label, api`. Use `gh api` for milestone/project mutations and arbitrary REST/GraphQL operations (e.g., `gh api --method PATCH /repos/owner/repo/milestones/N -f state=closed`).

**Incident:** session `4cbc6de7-...` on 2026-04-17 — milestone #12 dispatch failed because `--repo` was passed inside `command`, wrapper rejected it, agent dropped `--repo` on retry and falsely concluded milestone didn't exist.
