# Plan: fix(dispatch) — mika-qa autonomous dispatch silently no-ops on green CI (mika#1711)

- **Ticket:** mika issue#1711
- **Type:** fix (bug) — p1-important
- **Branch:** `fix/1711/dispatch-mika-qa-autonomous-dispatch`
- **Date:** 2026-07-01

## Problem

mika-qa's autonomous qa-review never fires for PRs that reach green CI, leaving
READY+GREEN PRs stranded with **no QA verdict**. Confirmed 14-hour dead window on
2026-07-01 (last mika-qa activity 02:53Z until manual mika-spirit restart at
17:15Z). PRs #1706, #1709, #1700 all landed green with zero qa attempts and had to
be admin-merged under Vincent-consent. `audit_events` show 0 `dispatch` rows for
`mika-qa` in 24h; 100% of webhook events land on mika-dev.

## Root-cause diagnosis (AC1)

The trigger gap is a **missing green-CI → mika-qa re-trigger**, split across two
concrete layers. Both were confirmed by reading the code, not inferred.

### The designed trigger and where it breaks

Per `skills/bundled/self-dev/system_prompt.md:3`: *"QA review happens
automatically — mika-qa is triggered by GitHub webhooks when PRs are created or
updated, and verdicts arrive back as PR review webhooks."*

The intended path:

1. Pilot opens PR → `pull_request.opened` → `route_event` (`crates/mika-gateway/src/github.rs:321`) → **mika-qa** → qa-review fires.
2. qa-review runs **build verification** (`skills/bundled/qa-review/system_prompt.md:40,424`) and posts `VERDICT: pass|hold|block`.
3. `pull_request_review.submitted` → mika-dev (`github.rs:323`) → `verdict_handler` merges on pass.

The hole: **step 1 depends on CI being green at PR-open time**, but in the
autonomous loop a PR is opened *before* CI finishes. When qa-review runs on an
open-but-not-yet-green PR it cannot emit `VERDICT: pass` (build verification is
gated on a green pipeline; the max verdict without it is `hold[review]` —
`qa-review/system_prompt.md:40`). The PR then needs a **re-trigger once CI turns
green** — and that re-trigger does not exist:

- **Layer 1 — gateway routing (`crates/mika-gateway/src/github.rs:324-327`).**
  `check_suite.completed` routes **all** conclusions (`failure|timed_out|success`)
  to **mika-dev**. There is no route from a green-CI event to mika-qa.

- **Layer 2 — mika-dev's CI-success handler (`crates/mika-agent/src/server/ci_success_handler.rs:150-159`).**
  On `check_suite.completed(success)`, `try_handle_ci_success` looks for an
  existing `VERDICT: pass` review (`find_pass_verdict`). When none exists it
  returns **`VerdictAction::Passthrough { enrichment: None }`** — a **silent
  no-op**. This is the exact 14h-dead-window behavior: green CI, no verdict,
  nothing routes the signal to mika-qa, and nothing is logged.

**Net:** the green-CI signal lands on mika-dev, mika-dev only *merges*
already-approved PRs, and there is no code path that turns "CI green + no QA
verdict yet" into a mika-qa qa-review dispatch. mika-qa is architecturally
excluded from the one event that says "the PR is now verifiable."

### Confirmation step (executed first by the pilot, before coding)

The pilot must empirically confirm which sub-case dominates before writing the
fix, because it changes nothing about the fix but validates the diagnosis:

```sql
-- Did mika-qa ever receive PR-open events (primary trigger alive)?
SELECT agent_id, tool_name, COUNT(*), MIN(datetime(created_at)), MAX(datetime(created_at))
FROM audit_events
WHERE created_at > datetime('now','-48 hours')
GROUP BY agent_id, tool_name
ORDER BY agent_id, tool_name;

-- ci_success silent-no-op count (post-fix this becomes webhook_no_op rows)
-- grep the server log for the info! at ci_success_handler.rs:153
```

If mika-qa shows PR-open activity but no green-CI follow-through → the green-CI
re-trigger gap is the sole cause (expected). If mika-qa shows **zero** PR-open
activity at all → there is an additional delivery/mode drop on the
`pull_request.opened → mika-qa` path to investigate before proceeding. The
confirmation gates which fix surface(s) are needed; the green-CI re-trigger is
required in either case.

## Requirements

- **R1.** When a PR reaches all-checks-green with no existing QA verdict, mika-qa's
  qa-review must be dispatched automatically (satisfies AC2).
- **R2.** Every previously-silent no-op branch in the CI-success path must emit an
  observable `audit_events` row with a machine-readable reason (satisfies AC3).
- **R3.** The successful qa dispatch must emit an `audit_events` row of kind
  `qa_dispatch_fired` per fire (satisfies AC2's audit clause).
- **R4.** The dispatch decision must be deterministic, pure, and unit-testable
  independent of GitHub I/O (enables AC4).
- **R5.** The mechanism must be idempotent — repeated `check_suite.completed`
  events for the same PR (one per workflow) must fire qa-review at most once,
  not on every workflow completion.
- **R6.** A dashboard signal must expose the mika-qa qa-fire rate and make a
  zero-fires-for-N-hours condition visible (satisfies AC5).
- **R7.** The existing merge path (green CI + existing pass verdict → merge) must
  be preserved unchanged.

## Design

### Chosen approach: re-trigger via the existing `review_requested → mika-qa` path

Extend `ci_success_handler` so that on **green CI + open PR + no pass verdict**,
instead of a silent `Passthrough`, it **requests a review from the QA bot**
(`mika-platform-qa`, the `QA_REVIEWER_LOGIN` const at `github.rs:239`). GitHub
then fires `pull_request.review_requested` with
`requested_reviewer.login == mika-platform-qa`, which:

- routes to **mika-qa** via `route_event` (`github.rs:321`), and
- passes the `is_suppressed_review_request` guard (`github.rs:304-306`, mika#1655)
  precisely because the reviewer *is* the QA bot,

triggering a fresh qa-review session against a now-green PR. This is the exact
operator-manual flow mika#1655 documented ("`gh api .../requested_reviewers` for
the QA bot triggers an autonomous qa-review"), now automated at the green-CI edge.

**Why this approach over the alternatives:**

- **vs. routing `check_suite.success` → mika-qa in the gateway
  (`github.rs:325`):** rejected. The green-CI event is *also* the merge trigger
  for already-approved PRs (`ci_success_handler`); re-pointing it to mika-qa would
  break R7, and the routing table has no dual-dispatch primitive. Requesting a
  reviewer keeps merge and re-review as two distinct, already-wired events.
- **vs. a new mika-qa CI-callback skill:** rejected as heavier — it duplicates
  qa-review's entry logic and requires a new skill + identity-allowlist change,
  when the `review_requested → qa-review` path already exists and is guarded.

The fix is surgical, reuses guarded paths, and puts the AC3 audit event exactly at
the site that was silently dropping.

### Decision extracted to a pure function (R4)

Mirror the existing `classify_checks` idiom. Add:

```rust
enum CiSuccessAction {
    Merge,                       // green CI + pass verdict + fresh SHA
    RequestQaReview,             // green CI + open PR + no verdict + not already requested/reviewed
    NoOp { reason: NoOpReason }, // everything else — now audited, not silent
}

enum NoOpReason {
    NoOpenPr, StaleVerdict, ChecksPending, ChecksFailing,
    BehindMain, QaAlreadyRequested, QaReviewAlreadyPresent, NoGithubToken,
}

fn decide_ci_success_action(
    has_open_pr: bool,
    pass_verdict: Option<&PassVerdictReview>,
    head_sha: &str,
    checks: CheckClassification,
    behind_main: bool,
    qa_review_present: bool,   // any mika-platform-qa review exists (pass or not)
    qa_already_requested: bool // mika-platform-qa in requested_reviewers
) -> CiSuccessAction
```

`try_handle_ci_success` becomes a thin I/O shell: gather PR/verdict/checks/
requested-reviewers state, call `decide_ci_success_action`, then execute
(merge / request-review / audit-noop). All branch logic is unit-tested on the
pure function; the shell keeps the existing 60s subprocess timeouts.

### Idempotency (R5)

Before requesting a review, the shell fetches the PR's existing reviews and
`requested_reviewers`. If a `mika-platform-qa` review already exists, or the QA
bot is already a requested reviewer, the action is `NoOp { QaAlreadyRequested |
QaReviewAlreadyPresent }` (audited, not fired). This bounds qa-review to at most
one fire per (PR, head SHA) across the multiple `check_suite` events a PR emits.

### Audit events (R2, R3)

- On `RequestQaReview` success → `log_audit_event(tool_name="dispatch",
  target_key="qa_dispatch_fired", ...)` with PR URL + head SHA in the detail.
- On every `NoOp { reason }` → `log_audit_event(tool_name="webhook",
  target_key="webhook_no_op", ...)` with the `NoOpReason` as the reason string.

Both keep the handler's existing `Passthrough`/`Handled` return contract for the
LLM turn; the audit write is fire-and-forget (warn-on-error), matching the
handler's existing audit-write style (`ci_success_handler.rs:326-341`).

### Dashboard signal (AC5, R6)

- **API:** add `GET /api/v1/qa-activity` to mika-spirit that aggregates
  `audit_events WHERE target_key IN ('qa_dispatch_fired','webhook_no_op')` into a
  time-bucketed count plus `last_fired_at` and `hours_since_last_fire`.
- **Dashboard:** add a "QA fire rate" card to the dashboard using the existing
  `@senara-solutions/ui` primitives (StatusBadge for the health state, no
  hand-rolled row/badge — per `packages/ui/CLAUDE.md`). Render `blocked`
  StatusBadge when `hours_since_last_fire > N` (N configurable, default 6h — the
  loop's observed dead-window sensitivity).

## Implementation steps

1. **AC1 confirmation.** Run the diagnosis SQL/log queries above; record the
   result in the PR body. Proceed with the green-CI re-trigger fix regardless;
   escalate only if the `pull_request.opened → mika-qa` path shows an *additional*
   drop.
2. **Extract `decide_ci_success_action` + enums** in `ci_success_handler.rs` (or a
   sibling `ci_success_decision.rs`), pure and `#[cfg(test)]`-covered.
3. **Add QA-state fetch helpers**: `find_qa_review(pr, repo, token)` (any review by
   `mika-platform-qa`) and `qa_in_requested_reviewers(pr, repo, token)` (GET
   `/repos/{repo}/pulls/{n}/requested_reviewers`). Reuse `run_gh_subprocess`
   (structural handler — not LLM-gated, so no `GH_API_ALLOW_MATRIX` change).
4. **Add `request_qa_review(pr, repo, token)`**: POST
   `/repos/{repo}/pulls/{n}/requested_reviewers` with `reviewers[]=mika-platform-qa`.
5. **Rewire `try_handle_ci_success`** to the gather → decide → execute shape;
   preserve the merge branch (R7) and 60s timeouts.
6. **Emit audit events** on `qa_dispatch_fired` and every `webhook_no_op` branch.
7. **AC4 tests** — see Verification Contract.
8. **AC5** — add the `/api/v1/qa-activity` endpoint + dashboard card.
9. **Docs** — update `crates/mika-agent/CLAUDE.md` "Structural CI Success Handler"
   and `crates/mika-gateway/CLAUDE.md` routing notes to describe the green-CI
   re-trigger; add a compound doc under `docs/solutions/` for the dead-window
   incident and the re-trigger pattern.

## Verification Contract

- **Unit (`decide_ci_success_action`)** — exhaustive matrix over
  `(has_open_pr, pass_verdict, sha match, checks classification, behind_main,
  qa_review_present, qa_already_requested)`:
  - green + pass + fresh SHA → `Merge`
  - green + no verdict + not requested/reviewed → `RequestQaReview`
  - green + no verdict + already requested → `NoOp{QaAlreadyRequested}`
  - green + no verdict + qa review already present → `NoOp{QaReviewAlreadyPresent}`
  - pending/failing checks → `NoOp{ChecksPending|ChecksFailing}`
  - stale verdict SHA → `NoOp{StaleVerdict}`
  - no open PR → `NoOp{NoOpenPr}`
- **Integration (`crates/mika-agent/tests/eval/test_ci_success_qa_retrigger.rs`,
  AC4)** — mirror `test_verdict_handler.rs` structure. Drive
  `parse_check_suite_success` + `decide_ci_success_action` with fixture PR state
  representing "green CI + open PR + no verdict"; assert the action is
  `RequestQaReview` and that the audit event `qa_dispatch_fired` is written. A
  companion fixture (`review_requested` marker text) asserts routing intent
  (`route_event("pull_request", Some("review_requested"), None) == Some("mika-qa")`
  and `is_suppressed_review_request(Some("review_requested"), Some("mika-platform-qa")) == false`).
  Runs in CI (no network — pure-function + parser level).
- **`cargo test -p mika-agent`**, `cargo clippy`, `cargo fmt --check` all green.
- **Dashboard** — `npm run build --prefix dashboard` succeeds; the QA-fire card
  renders via `@senara-solutions/ui` primitives.
- **Doc-sync** — `scripts/sync-agent-docs.sh` run if `docs/` changed (CI
  `docs-sync` gate).

## Definition of Done

- Green CI + open PR + no verdict deterministically dispatches a mika-qa qa-review
  via the guarded `review_requested → mika-qa` path, within one webhook cycle.
- Every previously-silent CI-success no-op emits an audited `webhook_no_op` row;
  each fire emits a `qa_dispatch_fired` row.
- The merge path for already-approved PRs is unchanged.
- Decision logic is pure and exhaustively unit-tested; an eval test proves the
  green-CI-no-verdict case dispatches; CI is green (test + clippy + fmt + docs-sync).
- The dashboard surfaces mika-qa qa-fire rate with a zero-fires-for-N-hours health
  state.
- `crates/mika-agent/CLAUDE.md`, `crates/mika-gateway/CLAUDE.md`, and a
  `docs/solutions/` compound entry are updated.

## Acceptance criteria

- **AC1 — diagnose the trigger gap.** Identify exactly which layer drops the webhook path to mika-qa. File-level location + specific missing wire.
- **AC2 — fire qa-review on green CI.** When a PR reaches all-checks-green + ready-for-review + no existing qa verdict, mika-qa's qa-review skill dispatches within 60s. Audit_event kind=`qa_dispatch_fired` emitted per fire.
- **AC3 — audit_event on silent-no-op.** When webhook lands but no skill fires, emit audit_event kind=`webhook_no_op` with reason. Currently invisible.
- **AC4 — integration test.** `crates/mika-agent/tests/eval/` covers: mocked review_requested webhook → mika-qa qa-review dispatch. Runs in CI. Passes.
- **AC5 — dashboard signal.** Add mika-qa qa-fire rate to observability dashboard. Zero-fires-for-N-hours triggers ops attention.

## Out of scope

- Fixing the qa-review skill's internal verdict logic — this ticket is
  dispatch-path only (per the ticket's own "Out of scope").
- Changing the merge semantics of `ci_success_handler` for already-approved PRs.
- Multi-tenant / customer-agent qa routing (the fix uses the well-known
  `mika-platform-qa` reviewer; per-customer QA-bot logins are a separate concern).

## Risks & mitigations

- **Self-request rejection:** GitHub forbids requesting the PR author as reviewer.
  The QA bot (`mika-platform-qa`) is a distinct machine user from the dev author
  (`mika-platform-dev`), so the request is valid — same precondition mika#1655
  already relies on. Mitigation: on a 422 from the request call, emit
  `webhook_no_op{reason=request_rejected}` rather than erroring the handler.
- **Duplicate fires across workflows:** mitigated by the idempotency guard (R5) —
  `qa_already_requested` / `qa_review_present` short-circuit to audited no-ops.
- **Fail-open on GitHub API errors:** consistent with the handler's existing
  fail-open posture (behind-main check at `ci_success_handler.rs:266-281`); API
  errors during QA-state fetch degrade to `NoOp` + audited reason, never a merge.
