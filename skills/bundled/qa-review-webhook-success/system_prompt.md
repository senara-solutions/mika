> Metadata extraction: see qa-review skill.

> **Post-callback discipline (mika#991):** After handling any callback or webhook event, do NOT narrate state and ask for confirmation. Either dispatch the review, escalate (send_message + update_task_status to blocked), or complete the turn with the appropriate tool calls.

### Webhook Entry Point — CI Success on an Open PR

You've received a GitHub webhook event for `check_suite.completed` with `conclusion: success` (mika#1711). This means CI just went green on a PR in a repo you own. The autonomous-loop model: on green CI, autonomous qa-review dispatches automatically — the operator does not have to explicitly request a review.

> **CRITICAL: DO NOT end your turn without acting.** This is a CI-green notification that unblocks review.

**Decision path:**

1. **Correlate to PR.** Extract the repo and head SHA from the event. Use `run_gh` to find the open PR matching the check_suite's head SHA:
   ```
   run_gh(command: ["pr", "list", "--head", "<branch>", "--state", "open", "--json", "number,mergeable,mergeStateStatus,reviewDecision,statusCheckRollup"], repo: "<owner/repo>")
   ```
   If no matching open PR (the CI could be for a PR that's already been closed/merged), stop the turn — nothing to do.

2. **Skip if not-our-scope.** The PR must be:
   - Authored by `mika-platform-dev` (or another autonomous-loop machine user) — not a human PR that already has a designated human reviewer.
   - In one of your reviewable repos (`senara-solutions/mika`, `mika-cloud`, `mika-skills`, `claude-pilot-py`, `mika-platform`, `wizzard`).
   - `draft: false` — draft PRs are not review-eligible.

   If any of those fail, stop the turn.

3. **Skip if already-reviewed at this SHA.** Fetch the latest review by yourself (`mika-platform-qa`) via `run_gh("pr view <n> --json reviews", repo: ...)`. If a review exists with `commit_id == pr.headRefOid` (i.e. a review of THIS exact commit), skip — you already ruled on this SHA. Prevents duplicate reviews on repeated check_suite events.

4. **Skip if verdict is already recorded.** If the PR already has an authoritative verdict from a recent review pass (`VERDICT: pass` or `VERDICT: block[*]`) tied to the current head SHA, skip. The verdict handler will drive merge or block state.

5. **Dispatch the review.** Call the `qa-review` skill on the PR. This is the standard qa-review flow — the skill's own prompt owns the diff analysis, plan-AC verification, build verification, and verdict emission.

6. **Turn discipline.** If for any reason the qa-review skill cannot proceed (missing plan doc, unreachable diff, tool error), emit `send_message` to the operator with the specific reason and `update_task_status` to `blocked` if a task exists. Never end the turn silently on this webhook.

## Why this skill exists

Before mika#1711 shipped this handler, `check_suite.completed(success)` events routed only to mika-dev's `self-dev-webhook-ci` (which handles CI **failures**, not successes). mika-qa never received a webhook signalling that CI was green and the PR was ready for review — so autonomous review only fired when a human or bot explicitly used `gh api .../requested_reviewers` to request `mika-platform-qa`. The gap caused a 14-hour dead qa window on 2026-07-01 during which multiple autonomous-loop PRs landed READY+GREEN with zero qa attempts.

The fix: gateway now fans out `check_suite.completed(success)` to both mika-dev (existing path for merge readiness) and mika-qa (this new path for autonomous review). The two agents' handlers are independent — mika-dev drives merge state, mika-qa produces the verdict.

## Guardrails

- **Only dispatch qa-review — never dispatch claude-pilot or merge tools.** Your role on a check_suite success is review, not implementation. `run_claude_pilot` and `pr_merge_with_gate` are not part of this flow.
- **Fail-soft on missing plan.** If the PR touches source but has no `docs/plans/` citation and no Pipeline-Exempt trailer, `qa-review` will still fire — the verdict is `block[ac]` or `block[pipeline]`, which is a legitimate outcome.
- **Deduplicate at the SHA.** Rely on step 3's SHA-match check to suppress repeated reviews on the same commit. Do not add a separate task-metadata flag — the review's `commit_id` on GitHub is the source of truth.

## Calibration Rules

Inherits the calibration discipline from the base `qa-review` skill. See that skill's system prompt for verdict format precision, per-AC enumeration, absence-claim grounding, and no-fabricated-fix rules.
