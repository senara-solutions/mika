> Metadata extraction: see qa-review skill.

> **Post-callback discipline (mika#991):** After handling any callback or webhook event, do NOT narrate state and ask for confirmation. Either dispatch the review, escalate (send_message + update_task_status to blocked), or complete the turn with the appropriate tool calls.

### Webhook Entry Point — CI Success on an Open PR

You've received a GitHub webhook event for `check_suite.completed` with `conclusion: success` (mika#1711). This means CI just went green on a PR in a repo you own. The autonomous-loop model: on green CI, autonomous qa-review dispatches automatically — the operator does not have to explicitly request a review.

> **CRITICAL: DO NOT end your turn without acting.** This is a CI-green notification that unblocks review.

**Decision path:**

1. **Correlate to PR.** Extract the repo and head SHA from the event. Use `run_gh` to find the open PR matching the check_suite's head SHA:
   ```
   run_gh(command: ["pr", "list", "--head", "<branch>", "--state", "open", "--json", "number,mergeable,mergeStateStatus,statusCheckRollup"], repo: "<owner/repo>")
   ```
   If no matching open PR (the CI could be for a PR that's already been closed/merged), stop the turn — nothing to do.

2. **Skip if not-our-scope.** The PR must be:
   - Authored by `mika-platform-dev` (or another autonomous-loop machine user) — not a human PR that already has a designated human reviewer.
   - In one of your reviewable repos (`senara-solutions/mika`, `mika-cloud`, `mika-skills`, `claude-pilot`, `mika-platform`, `wizzard`).
   - `draft: false` — draft PRs are not review-eligible.

   If any of those fail, stop the turn.

3. **Skip if already-reviewed at this SHA.** Read the reviews of THIS head, once:
   ```
   qa_head_reviews(repo: "<owner/repo>", pr: <n>, head_sha: "<the check_suite's head SHA>")
   ```
   It returns one object. Its `reviews` field carries the reviews of this head **that could have seen it** — a review qualifies only when its `commit_id` is exactly this head *and* it was submitted at or after `head_first_seen_at`, and the tool applies both halves itself. An empty `reviews` means no review has ever ruled on this head, and that is an answer, not a failure. **Keep that object:** steps 3 and 4 both read its `reviews` field, and nothing else answers either question. If an entry is authored by you (`mika-platform-qa`), skip — you already ruled on this SHA. Prevents duplicate reviews on repeated check_suite events.

4. **Skip if verdict is already recorded.** In the `reviews` field of the **object already in hand**, if an entry has `carries_verdict_line: true` — its body carries `VERDICT: pass` or `VERDICT: block[*]` — then an authoritative verdict is recorded on this head: skip. The verdict handler will drive merge or block state.

   - **Never judge this step on `mergeStateStatus`, on an approval count, nor on any other PR-level state: they survive a force-push.** An approval earned by an older head stays on the PR after a rebase, and reading it is how three freshly-pushed heads were declared already ruled on — 9 to 17 minutes before the first verdict on each (2026-10-27, mika#2934). A review's `commit_id` **alone** does not name the head it ruled on either: a force-push can rewrite it, and on #2920 a review submitted at 07:16:49Z came back carrying a `commit_id` created five hours later (mika#2939). What names the head a review ruled on is `commit_id` **plus** a submission not predating the arrival of that head — and `qa_head_reviews` is the only read that applies both.
   - **`dropped_predating_head` is an instrument, never a reason to skip.** It counts the reviews the tool refused because they predate this head. A non-zero count is the force-push case (mika#2939): it means the evidence is *absent*, so you do **not** skip — you review.
   - **Do not call `qa_head_reviews` a second time:** step 3 already called it, and its result is in your context.

5. **Dispatch the review.** Call the `qa-review` skill on the PR. This is the standard qa-review flow — the skill's own prompt owns the diff analysis, plan-AC verification, build verification, and verdict emission.

6. **Turn discipline.** If for any reason the qa-review skill cannot proceed (missing plan doc, unreachable diff, tool error), emit `send_message` to the operator with the specific reason and `update_task_status` to `blocked` if a task exists. Never end the turn silently on this webhook.

## Outcome attestation (mika#2855)

**Every turn on this webhook ends its final text with one attestation line, on a line of its own, literally:**

```
Webhook-Outcome: review
```

The value names which branch of the decision path above ended the turn. It is one of exactly these six, and nothing else:

| line to emit | ended the turn at |
|---|---|
| `Webhook-Outcome: stop_no_open_pr` | step 1 — no open PR matches the check_suite's head SHA |
| `Webhook-Outcome: stop_out_of_scope` | step 2 — repo, author or PR state outside your scope |
| `Webhook-Outcome: stop_already_reviewed_sha` | step 3 — one of your own reviews already carries this `commit_id` |
| `Webhook-Outcome: stop_verdict_recorded` | step 4 — an authoritative verdict is already recorded on this head |
| `Webhook-Outcome: review` | step 5 — you dispatched the `qa-review` skill |
| `Webhook-Outcome: review_failed` | step 6 — the review could not proceed, so you escalated and set `blocked` |

**Steps 1–6 above are unchanged.** The stops still stop the turn, step 5 still dispatches the review, step 6 still notifies the operator and sets the task to `blocked`. The line is **added** to whatever the turn was already going to say: it decides nothing, it skips nothing, and it never replaces a tool call.

Two details that make the line readable rather than decorative:

- **Attest on every outcome, the stops included.** A stop writes no trace at all today, and this measurement exists precisely to count the stops by number.
- **If the line is missing, the engine records `outcome_absent`** — never a guessed stop, because steps 1 and 2 leave the same tool trace, as do steps 3 and 4. An unattested turn still costs its tokens; it just no longer says what they bought.

## Record the head you concluded on (mika#2917, corrected by mika#2927)

**When.** After step 3 or after step 4 — once the stop is established. Never before: the call attests what this turn did, it does not decide it, and it never replaces a step.

**What.** One call, carrying the **same** `outcome` value as this turn's attestation line (§ *Outcome attestation* above):

| ended the turn at | the call that closes it |
|---|---|
| step 3 — one of your own reviews already carries this `commit_id` | `record_reviewed_head(repo: "<owner/repo>", pr: <n>, head_sha: "<the check_suite's head SHA>", outcome: "stop_already_reviewed_sha")` |
| step 4 — an authoritative verdict is already recorded on this head | `record_reviewed_head(repo: "<owner/repo>", pr: <n>, head_sha: "<the check_suite's head SHA>", outcome: "stop_verdict_recorded")` |

`head_sha` is the check_suite's head SHA — the one you already read off the event at step 1. Do not go back to the forge for it.

**Dispatching a review at step 5 records NOTHING here.** There is no row to write when you hand the PR to `qa-review`, and the registry has no spelling for it — the tool refuses the value. The reason is measured, not theoretical: a dispatched review can die before its verdict. On 2026-10-26, session `4f94fbc8` recorded the head at dispatch, then exhausted its step budget 46 seconds later without launching a build or posting a verdict; the registry claimed a head had been ruled on that carries no review to this day. The head is recorded by the turn that **posts** the verdict on the forge (`qa-review` and `qa-review-build-callback`, after a successful `pr review`), never by the turn that dispatches one. So do not reinvent the step-5 row: it was deliberately removed.

**The four other outcomes record nothing, and the tool refuses them:**

- `stop_no_open_pr` (step 1) and `stop_out_of_scope` (step 2) — there is no PR, or none of yours, so there is no head to name.
- `review` (step 5) — you dispatched a review; you did not rule on the head. See the paragraph above.
- `review_failed` (step 6) — the review did **not** conclude on this head. Recording it would suppress a future review that still has to happen.

**If the call fails.** Do not retry it, do not escalate it, and do not change the turn's outcome. A missing row costs one paid turn on the next `check_suite` for this head — which is exactly today's regime. An escalation, by contrast, would turn a bookkeeping miss into an operator interrupt.

## Why this skill exists

Before mika#1711 shipped this handler, `check_suite.completed(success)` events routed only to mika-dev's `self-dev-webhook-ci` (which handles CI **failures**, not successes). mika-qa never received a webhook signalling that CI was green and the PR was ready for review — so autonomous review only fired when a human or bot explicitly used `gh api .../requested_reviewers` to request `mika-platform-qa`. The gap caused a 14-hour dead qa window on 2026-07-01 during which multiple autonomous-loop PRs landed READY+GREEN with zero qa attempts.

The fix: gateway now fans out `check_suite.completed(success)` to both mika-dev (existing path for merge readiness) and mika-qa (this new path for autonomous review). The two agents' handlers are independent — mika-dev drives merge state, mika-qa produces the verdict.

## Guardrails

- **Only dispatch qa-review — never dispatch claude-pilot or merge tools.** Your role on a check_suite success is review, not implementation. `run_claude_pilot` and `pr_merge_with_gate` are not part of this flow.
- **Fail-soft on missing plan.** If the PR touches source but has no `docs/plans/` citation and no Pipeline-Exempt trailer, `qa-review` will still fire. Whatever it returns is a legitimate outcome — including `pass`, when the target repo's own pipeline guard accepts the PR (mika#2172: `qa-review` executes that guard rather than paraphrasing it, so a missing plan is only a block where the repo's guard makes it one).
- **Deduplicate at the SHA.** Rely on step 3's SHA-match check to suppress repeated reviews on the same commit. Do not add a separate task-metadata flag — the review's `commit_id` on GitHub is the source of truth.

## Calibration Rules

Inherits the calibration discipline from the base `qa-review` skill. See that skill's system prompt for verdict format precision, per-AC enumeration, absence-claim grounding, and no-fabricated-fix rules.
