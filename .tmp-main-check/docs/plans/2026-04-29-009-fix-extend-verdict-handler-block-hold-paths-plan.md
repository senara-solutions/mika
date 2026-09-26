---
title: "fix(mika-agent): extend structural verdict handler from mika#524 to cover block[ac] / block[*] / hold[*] dispatch paths"
type: fix
status: draft
date: 2026-04-29
issue: 889
---

# fix(mika-agent): extend structural verdict handler — block[ac] / block[*] / hold[*]

## Phase 0 — pre-implementation verification (gate before commit)

Before writing any handler code, the implementer MUST verify the following pinned facts. If any disagree with the plan, halt and surface to operator before proceeding.

### Pin 1 — `Verdict` enum shape (verified 2026-04-29 in `crates/mika-agent/src/server/verdict.rs`)

Current shape (line 26):
```rust
pub(crate) enum Verdict {
    Pass,
    Block(String),    // carries the reason string ("ac", "ci", "security", "pipeline")
    Hold(String),     // carries the reason string ("review")
    Missing { truncated: bool },
}
```

`parse_verdict()` (verified) extracts the verdict via `VERDICT_RE` regex. For `Block(_)`, the inner `BLOCK_RE` extracts the bracketed reason. Same for `Hold(_)`. **Plan dispatch must use nested match on the reason string, NOT flat-variant assumptions.** This invalidated the original draft of this plan; architect F1 forced a re-derivation.

### Pin 2 — pre-digest pattern (verified in `verdict_handler.rs`)

Existing helpers (lines 397, 433, 451):
- `format_success_pre_digest(event, action_desc, &task_id)` → "merge initiated for PR <url>: ..."
- `format_already_merged_pre_digest(event, &task_id)`
- `format_error_pre_digest(event, error)`

New helpers MUST match this naming + signature pattern. Architect F8 mandates parallel structure across verdict paths so the LLM's context shape is consistent.

### Pin 3 — `HandlerResult::Handled { pre_digest: String }` is the success-acted return shape

Current shape (line 31): `Handled { pre_digest: String }`. New handlers return the same `HandlerResult::Handled` variant with the appropriate pre-digest string.

### Pin 4 — task metadata is JSON; no schema change needed

Per architect F2 fix path (a): retry counter lives in `tasks.metadata` JSON as `verdict_block_ac.count`. No new column, no migration. Existing pattern from `handle_pass_verdict`'s `update_verdict_metadata()` call.

### Pin 5 — AC-parser regex must be derived from qa-review skill prompt before coding

Read `skills/bundled/qa-review/system_prompt.md`. Derive the AC-failure line shape (the format mika-qa uses to signal unsatisfied ACs in its review body). Pin the resulting regex BEFORE implementing `extract_unsatisfied_acs()` in Change 5. The fallback contract (2000-char ceiling + `verdict_ac_extraction_fallback` log event) means parser failure causes no correctness loss; this Pin is about quality-of-prompt-content for the auto-dispatched claude-pilot session, not correctness.

---

## Why

mika#524 (CLOSED 2026-04-13) shipped the structural verdict handler for `VERDICT: pass` (deterministic merge dispatch via `pr_merge_with_gate`). The handler explicitly leaves `block[*]` and `hold[*]` verdicts to LLM judgment per `mika/docs/solutions/architecture-patterns/structural-verdict-handler-pr-review-auto-merge.md` § Guidance step 3 carve-out: *"block[*] / hold[*] → pass through to LLM (LLM still drives retry logic)."*

That carve-out failed in production today: PR mika#888 received `state=COMMENTED` + `VERDICT: block[ac]` from mika-qa with two real unsatisfied ACs (Unit 1 eval test, Unit 3 mika-platform companion command). mika-dev's LLM gated on GH `review.state` ("COMMENTED" ≠ "CHANGES_REQUESTED") and silently dismissed the verdict as "comment, not formal verdict." PR sat with unfixed ACs until manual operator unblock. Same antipattern family as mika#522's pre-#524 misclassification.

Today saw 4 reproductions of LLM-improvises-on-state-machine-transitions: PR #888 (this), mika#522 (#524 fixed), grooming-comment-as-dispatch-trigger (mika#890 dispatch), and the mika#889 body's third-instance documentation.

## Goal

Every parseable `VERDICT:` token in a `pull_request_review.submitted` webhook body deterministically maps to an engine-level action — no LLM judgment on the dispatch decision.

| Verdict | Action |
|---|---|
| `pass` | (existing) merge via `pr_merge_with_gate` |
| `block[ac]` | dispatch claude-pilot with AC-fix prompt; bounded retry counter |
| `block[ci]` | dispatch claude-pilot with CI-fix prompt; bounded retry counter |
| `block[security]`, `block[pipeline]` | mark task `blocked`; notify operator; NO auto-dispatch |
| `hold[review]` | leave task `in_progress`; notify operator |
| missing/unparseable | `hold[review]` semantics + `verdict_classification_failed` log event |

The LLM stops being the verdict classifier. The body's `VERDICT:` token is authoritative regardless of GH `review.state`.

## Existing code surface

| File | Role | Relevant symbols |
|---|---|---|
| `crates/mika-agent/src/server/verdict_handler.rs` | Main handler (~280 LOC) | `try_handle_pr_review_verdict()`, `handle_pass_verdict()`, `update_verdict_metadata()`, `format_*_pre_digest()` |
| `crates/mika-agent/src/server/verdict.rs` | Verdict parsing + types | `Verdict` enum, `parse_pr_review_event()`, `parse_verdict()`, `VERDICT_RE`, `BLOCK_RE`, `HOLD_RE` |
| `crates/mika-agent/src/server/handlers.rs` | Webhook dispatch entry | (caller of `try_handle_pr_review_verdict`) |
| `crates/mika-agent/src/server/ci_failure_handler.rs` | Existing CI-fix dispatch | reference pattern for `block[ci]` |
| `crates/mika-agent/src/server/webhook_queue.rs` | Webhook event types | `PrReviewEvent` |
| `crates/mika-gateway/src/github.rs` | Gateway-side webhook routing | `route_event()` for `pull_request_review.submitted` (NO changes — this fix is consumer-side per mika#487 contract) |

## Approach

Extend the `match verdict { ... }` block in `try_handle_pr_review_verdict()` (currently at `verdict_handler.rs:66`) to dispatch each variant deterministically. **Use the actual `Verdict::Block(String) / Hold(String) / Missing` enum shape (Pin 1), NOT flat variants.**

### Change 1 — Match block extension (verdict_handler.rs)

Current:
```rust
match verdict {
    Verdict::Pass => handle_pass_verdict(...),
    _ => /* "Let the LLM handle block/hold verdicts" — TO BE REMOVED */
}
```

Replace with nested-match dispatch on `Verdict::Block(String) / Hold(String) / Missing`:
```rust
match verdict {
    Verdict::Pass => handle_pass_verdict(event, state).await,
    Verdict::Block(reason) => match reason.as_str() {
        "ac" => handle_block_ac(event, state, &event.body).await,
        "ci" => handle_block_ci(event, state, &event.body).await,
        "security" | "pipeline" => handle_escalate(event, state, &reason, &event.body).await,
        unrecognized => {
            warn!(reason = %unrecognized, "Unrecognized block[*] verdict subtype — passing through to LLM");
            HandlerResult::PassThrough
        }
    },
    Verdict::Hold(reason) => match reason.as_str() {
        "review" => handle_hold_review(event, state, &event.body).await,
        unrecognized => {
            warn!(reason = %unrecognized, "Unrecognized hold[*] verdict subtype — passing through to LLM");
            HandlerResult::PassThrough
        }
    },
    Verdict::Missing { truncated } => handle_missing_verdict(event, state, truncated).await,
}
```

Unrecognized block/hold subtypes pass through (not silent dismiss — LLM still has a chance). Logged warn so operator can see the contract violation.

### Change 2 — `handle_block_ac()` with bounded retry counter (architect F2 BLOCKER)

```rust
async fn handle_block_ac(
    event: &PrReviewEvent,
    state: &AppState,
    body: &str,
) -> HandlerResult {
    // 1. Look up task by metadata.claude_pilot.pr_url (same lookup as handle_pass_verdict)
    // 2. If no task: pass through to LLM with verdict_missing-style enrichment, log warn
    // 3. Read task.metadata.verdict_block_ac.count (default 0)
    // 4. If count >= BLOCK_AC_MAX_RETRIES (const 3): fall through to handle_escalate semantics
    //    a. Mark task: blocked
    //    b. Send_message operator: "PR <url>: block[ac] retry limit (3) reached. Last verdict body excerpt: ... Operator escalation required."
    //    c. Log audit_event: verdict_escalated_block_ac_loop_limit
    //    d. Return HandlerResult::Handled { pre_digest: format_block_ac_limit_pre_digest(event, &task_id) }
    // 5. Else (count < limit):
    //    a. Extract AC list from body (see Change 5)
    //    b. Increment task.metadata.verdict_block_ac.count
    //    c. Construct AC-fix prompt: "<verdict body><AC list summary>"
    //    d. Dispatch run_claude_pilot tool call with iteration_context = AC-fix prompt
    //    e. Update task.metadata.verdict_block_ac = { received_at, review_id, count: count+1, last_verdict_body_excerpt }
    //    f. Log audit_event: verdict_handled (action="ac_fix_dispatched", count=N)
    //    g. Return HandlerResult::Handled { pre_digest: format_block_ac_pre_digest(event, ac_summary, &task_id, count+1) }
}

const BLOCK_AC_MAX_RETRIES: u32 = 3;
const BLOCK_CI_MAX_RETRIES: u32 = 3;
```

**Per architect F9 sharpening:** declare these as separate consts, NOT a shared `MAX_RETRIES`. Future calibration may diverge between AC and CI thresholds (e.g., CI is auto-fixable so could allow more retries; AC depends on plan-quality so may need stricter cap). Separate symbols keep that adjustability.

**Why bounded:** without a counter, a `block[ac]` verdict that fixes some ACs but the next mika-qa review surfaces NEW ACs creates an unbounded loop (architect F2). The mika#524 `pass → merge` loop terminates by design (`pass` is terminal); `block[ac]` does NOT terminate by design.

The cap value (3) is calibration-bait: too low blocks legitimate iterative fixes; too high allows runaway loops. 3 is the rebound limit — gives one retry to fix the obvious cause, one to react to the AC's rebound, and one final attempt before escalation.

### Change 3 — `handle_block_ci()`

Same structure as `handle_block_ac()` but uses `ci_failure_handler.rs`'s existing CI-fix dispatch pattern. Same bounded retry counter (`verdict_block_ci.count`, max `BLOCK_CI_MAX_RETRIES = 3`).

### Change 4 — `handle_escalate()` for block[security] / block[pipeline]

```rust
async fn handle_escalate(
    event: &PrReviewEvent,
    state: &AppState,
    reason: &str,  // "security" or "pipeline"
    body: &str,
) -> HandlerResult {
    // 1. Look up task; if no task, pass through with warn
    // 2. Mark task status: blocked
    // 3. Update task.metadata.verdict_escalated = { received_at, review_id, reason, body_excerpt }
    // 4. Send_message operator (Vincent) with structured escalation: "PR <url> block[<reason>]. Body excerpt: ... Task marked blocked. Operator review required."
    // 5. Log audit_event: verdict_escalated (reason=<reason>)
    // 6. Return HandlerResult::Handled { pre_digest: format_escalate_pre_digest(event, reason, &task_id) }
}
```

**Why escalate vs. retry (architect F4):** `block[pipeline]` failures are gate-configuration or artifact violations requiring operator review, not auto-remediable transients. `block[security]` is operator-attention-by-design. Contrast with `block[ci]` where failures are transient and auto-fixable via re-run or code patch.

### Change 5 — AC extraction with fallback contract (architect F3 BLOCKER)

Best-effort extraction from mika-qa's structured verdict body shape (per `skills/bundled/qa-review/system_prompt.md`). Parse lines matching `^\s*-?\s*\[❌\] unsatisfied (Unit \d+|R\d+):` (or equivalent emoji+ticker pattern).

**Fallback contract (architect F3):**
- If parser yields zero ACs OR parser fails: use first 2000 chars of verdict body as the fallback content, with a `[ac-extraction-fallback: true]` marker prepended.
- If verdict body > 2000 chars, truncate at 2000 with `[truncated]` suffix.
- Log structured event `verdict_ac_extraction_fallback` (mirroring `verdict_classification_failed`) with PR URL, review id, body excerpt, fallback reason ("zero matches" | "parser error: <reason>").

```rust
fn extract_ac_list_or_fallback(body: &str) -> AcExtraction {
    if let Some(acs) = parse_structured_acs(body) {
        if !acs.is_empty() {
            return AcExtraction::Structured(acs);
        }
    }
    // Fallback path
    let excerpt = if body.len() > 2000 { format!("{}[truncated]", &body[..2000]) } else { body.to_string() };
    log_verdict_ac_extraction_fallback(...);
    AcExtraction::Fallback(excerpt)
}
```

The pre-digest message must indicate fallback fired (so operator notices when the extraction broke and the prompt is using the full body).

### Change 6 — `handle_hold_review()`

```rust
async fn handle_hold_review(
    event: &PrReviewEvent,
    state: &AppState,
    body: &str,
) -> HandlerResult {
    // 1. Look up task
    // 2. Leave task status: in_progress (no transition)
    // 3. Update task.metadata.verdict_hold_review = { received_at, review_id, body_excerpt }
    // 4. Send_message operator: "Hold[review] verdict on PR <url>; awaiting operator decision. Body: ..."
    // 5. Log audit_event: verdict_held (reason="review")
    // 6. Return HandlerResult::Handled { pre_digest: format_hold_review_pre_digest(event, &task_id) }
}
```

`hold[review]` is operator-mediated review iteration. Surface it; operator decides next step.

### Change 7 — `handle_missing_verdict()` (architect F3 sharpening + safe-default)

```rust
async fn handle_missing_verdict(
    event: &PrReviewEvent,
    state: &AppState,
    truncated: bool,
) -> HandlerResult {
    // Log structured event: verdict_classification_failed
    //   fields: { delivery_id, pr_url, review_id, body_truncated: truncated, body_excerpt: first_200_chars }
    // Apply hold[review] semantics: notify operator, leave task in_progress
    // Pre-digest: "[verdict_classification_failed] No parseable VERDICT: line in review body. Operator notified."
}
```

Currently (line 75-90 of existing handler) verdict-missing fires only on approved reviews. Generalize: ANY missing/unparseable verdict on `pull_request_review.submitted` → safe-default `hold[review]`. Do NOT silently dismiss.

### Change 8 — New pre-digest helpers (architect F8 sharpening)

Following the `format_*_pre_digest(event, ...)` naming + signature pattern (Pin 2):

- `format_block_ac_pre_digest(event, ac_summary, task_id, retry_count)` → "ac-fix dispatched for PR <url>: <count> ACs (<summary>); retry <N>/3."
- `format_block_ac_limit_pre_digest(event, task_id)` → "ac-fix retry limit reached for PR <url>; task escalated."
- `format_block_ci_pre_digest(event, ci_summary, task_id, retry_count)` → "ci-fix dispatched for PR <url>: <failing checks>; retry <N>/3."
- `format_escalate_pre_digest(event, reason, task_id)` → "escalation initiated for PR <url> block[<reason>]; task marked blocked."
- `format_hold_review_pre_digest(event, task_id)` → "hold[review] on PR <url>; operator notified; task remains in_progress."
- `format_verdict_classification_failed_pre_digest(event)` → "[verdict_classification_failed] no parseable VERDICT in review body; operator notified."

All match `format_success_pre_digest`'s prefix pattern ("merge initiated for PR <url>: ..."). Pre-digest verbs: `merge initiated`, `ac-fix dispatched`, `ci-fix dispatched`, `escalation initiated`, `hold[review]`. Parallel structure.

## Critical files

| Purpose | Path |
|---|---|
| Main handler extension | `crates/mika-agent/src/server/verdict_handler.rs` (existing ~280 LOC, expand by ~200-300 LOC) |
| Verdict enum (read-only verification) | `crates/mika-agent/src/server/verdict.rs` |
| Test fixtures (inline) | `crates/mika-agent/src/server/verdict_handler.rs::tests` |
| CI-fix reference shape | `crates/mika-agent/src/server/ci_failure_handler.rs` |
| qa-review verdict body shape | `skills/bundled/qa-review/system_prompt.md` (read for AC extraction parser) |

## Out of Scope

- **Producer-side changes** to qa-review skill. Body-as-truth contract is intentional; this fix consumes existing verdict tokens.
- **`issue_comment.created` events** (third reproduction in mika#889 body — auto-dispatch from grooming-summary comment text). Different event type, separate gateway arm. **File separately if it remains an issue post-this-fix** (architect F7 confirms scope restriction).
- **New verdict tokens** beyond the 6 covered. Extend then.
- **Schema migration** — all new metadata lives in `tasks.metadata` JSON.
- **Table-driven dispatch refactor** — match-block extension is correct for v1 (architect F6); table-driven is a follow-up if dispatch grows.
- **Retry-counter tuning** — fixed at 3 for `block[ac]` and `block[ci]`. If empirical loops surface, calibrate in a follow-up.

## Acceptance Criteria

- [x] R0 (Phase 0 gate): All 4 pinned facts (Pin 1-4 above) verified before commit. Implementer halts and surfaces to operator on any disagreement.
- [x] R1: Match block in `try_handle_pr_review_verdict()` dispatches `Verdict::Block(reason)` and `Verdict::Hold(reason)` via nested match on `reason.as_str()`. No `_ => { /* LLM passes */ }` for known reason strings ("ac", "ci", "security", "pipeline", "review"). Unrecognized reasons log warn and pass through.
- [x] R2: `handle_block_ac()` dispatches claude-pilot with AC-fix prompt; tracks `task.metadata.verdict_block_ac.count`; on count >= 3, escalates instead of dispatching.
- [x] R3: `handle_block_ci()` dispatches claude-pilot with CI-fix prompt; tracks `task.metadata.verdict_block_ci.count`; on count >= 3, escalates.
- [x] R4: `handle_escalate()` for block[security]/block[pipeline] marks task `blocked`, sends operator notification, does NOT auto-dispatch claude-pilot.
- [x] R5: `handle_hold_review()` keeps task `in_progress`, sends operator notification.
- [x] R6: `handle_missing_verdict()` applies hold[review] semantics + logs structured `verdict_classification_failed` event with diagnostic fields.
- [x] R7: AC extraction has explicit fallback contract: parser failure or zero matches → 2000-char body excerpt + `verdict_ac_extraction_fallback` log event with diagnostic fields.
- [x] R8: All verdict classes pre-digest the action result for the LLM via new helpers matching the `format_*_pre_digest` naming + prefix pattern. No raw webhook text reaches the agent context.
- [x] R9: Existing `pass → merge` path is unchanged. Regression test for `pass` verdict still passes (covers mika#524's original AC).
- [x] R10: Test fixtures cover all paths: `state=COMMENTED` + each of `block[ac]`, `block[ci]`, `block[security]`, `block[pipeline]`, `hold[review]`, missing. Plus regression: `state=APPROVED` + `VERDICT: pass` (mika#524). Plus architect F5 case: `state=CHANGES_REQUESTED` + `VERDICT: block[ac]` → assert single dispatch (no double-fire from any legacy CHANGES_REQUESTED path).
- [x] R11: `audit_events` table records the appropriate row for each new dispatch class (`verdict_handled` for ac/ci dispatches; `verdict_escalated` for security/pipeline; `verdict_held` for review; `verdict_classification_failed` for missing).
- [x] R12: Retry counter test: synthesize three sequential `block[ac]` webhooks for the same PR; verify first two trigger `handle_block_ac` dispatches, third triggers escalation.

## Verification

1. **Unit tests:** `cargo test -p mika-agent verdict_handler` — all paths pass.
2. **Reproduction test (PR #888 reference):** synthetic `state=COMMENTED` + `VERDICT: block[ac]` webhook → assert claude-pilot dispatch fires with AC-fix prompt; task metadata gets `verdict_block_ac.count = 1`.
3. **Regression test (mika#524):** existing `pass → merge` test still green.
4. **Retry-limit test (R12):** synthesize 3 sequential block[ac] events; assert first two dispatch, third escalates.
5. **Fallback test:** synthetic webhook with malformed body (no parseable AC list) → assert `verdict_ac_extraction_fallback` log event fires + handler completes with fallback prompt content.
6. **End-to-end smoke** (post-deploy): synthesize a PR with mika-qa block[ac] verdict; verify webhook → engine handler → claude-pilot dispatch → mika-qa re-review cycle works without manual unblock.

## Cross-references

- mika#524 (CLOSED 2026-04-13) — original `pass → merge` structural handler
- mika#553 (CLOSED 2026-04-12) — webhook entry-point tightening for `pass` verdict
- mika#487 (CLOSED 2026-04-09) — gateway PR review observability + body-as-truth verdict contract
- mika#864 (MERGED 2026-04-29) — `required_suffix_lines` enforcement at emission
- `mika/docs/solutions/architecture-patterns/structural-verdict-handler-pr-review-auto-merge.md` — design doc for #524; § Guidance step 3 carve-out is what this fix closes
- PR mika#888 — canonical reproduction (review id 4196247084)
- mika#695 / mika#821 / mika#822 — within-session duplicate-review prevention (related but distinct)

## Sequencing & Risk

- **Risk: regression on existing `pass → merge` path.** Mitigated by R9 + R10 regression test.
- **Risk: AC extraction parser brittleness.** Mitigated by F3 fallback contract (2000-char ceiling + structured log event).
- **Risk: retry counter race conditions.** Counter increment is read-modify-write on `task.metadata`. If two webhook events for the same PR fire concurrently, counter could be off-by-one. Mitigated by `update_verdict_metadata()` using existing transaction discipline (same pattern as mika#524's `pass → merge`).
- **Sequencing:** Independent of mika#887 (skills handler) and mika#893 (factorization). Engine-side change in `crates/mika-agent/`. Can dispatch in any order in the sprint.

## Grooming history

- /ce:plan (operator-drafted, well-specified ticket body) → mika-arch first-pass review (session `dbbfcd56-7989-4ff6-99a5-52dbf5b24c21`):
  - F1 BLOCKER: enum shape unverified. Resolved via Pin 1 — read `verdict.rs::Verdict` and dispatch shape rewritten as nested match on `Block(String) / Hold(String)`.
  - F2 BLOCKER: unbounded `block[ac]` retry. Resolved by adding `task.metadata.verdict_block_ac.count` + `BLOCK_AC_MAX_RETRIES = 3` cap with escalation fallthrough.
  - F3 BLOCKER: AC fallback contract ambiguous. Resolved by specifying 2000-char ceiling + `verdict_ac_extraction_fallback` structured log event.
  - F4 sharpening: block[pipeline] vs block[ci] rationale. Applied inline in Change 4.
  - F5 sharpening: CHANGES_REQUESTED test fixture. Added to R10.
  - F6 sharpening: match-block extension correct for v1. Confirmed in Out of Scope.
  - F7 sharpening: issue_comment scope restriction. Confirmed in Out of Scope.
  - F8 sharpening: pre-digest format match `format_*_pre_digest`. Resolved via Pin 2 + Change 8 helpers list.
- Disposition: ITERATE → revisions applied → second-pass pending GROOMED.
