PR: fix(#2299): the merge gate rejects a self-merge
Size: +71 -4, 2 files
State: OPEN

**Step 2.5 — AC Classification & Verification**

Plan: `docs/plans/2026-09-09-001-merge-gate-self-merge-plan.md`

| AC | Text | Classification | Rationale |
|---|---|---|---|
| AC1 | `pr_merge_with_gate` refuses a merge when the merging actor is the actor that posted the approving review | **Structural** | Verifiable by reading source — the diff shows `evaluate()` comparing `req.actor` against `review.submitted_by` and returning `Err(MergeGateError::ReviewerIsMerger)`. No CLI command or runtime output to invoke. |
| AC2 | The refusal carries a distinct reason token so the caller can branch on it | **Structural** | Greppable — `ReviewerIsMerger` variant added to `MergeGateError` enum. |
| AC3 | No test regressions — `cargo test` passes | **CI-deferred** | Explicitly defers to CI. Diff summary reports 214 passed, 0 failed. |

**Implicit structural AC:** Changed files are `crates/mika-agent/src/server/merge_gate.rs` and `crates/mika-agent/tests/merge_gate_test.rs` — no new files under `docs/plans/`. No parallel plan files. ✓

**Step 3b — Diff Review**

Checklist scan against the supplied diff:
- Hardcoded credentials/secrets/tokens: none
- `unsafe` blocks: none
- `eval`/`exec` dynamic execution: none
- SQL injection vectors: none
- Obvious logic errors: none — the comparison `req.actor` vs `review.submitted_by` returning `Err(ReviewerIsMerger)` is straightforward
- Dead code: none — `ReviewerIsMerger` is constructed and returned in `evaluate()`
- Missing error handling: none — error is returned via `Err(MergeGateError::ReviewerIsMerger)`
- TODO file status mismatch: no `todos/` files in diff
- Behavioral refactor (logic removal + delegation): none detected

Diff review passes.

**Notable observation (not a Step 3b block):** All three added tests assert the positive (allow) path. No test constructs a request where `actor == review.submitted_by` to verify `Err(MergeGateError::ReviewerIsMerger)` is returned. The core behavior the PR introduces — rejecting a self-merge — is untested. This is a test-coverage gap, not a logic error, so it does not trigger a Step 3b hold.

---

VERDICT: pass
DEPTH: code-level
REASON: Pipeline green, diff review clean, all plan ACs satisfied. Noting missing test coverage for the self-merge rejection path as a finding.

DIFF ANALYSIS:
Files reviewed: 2
Key changes:
- Added `ReviewerIsMerger` variant to `MergeGateError` enum in `crates/mika-agent/src/server/merge_gate.rs`; `evaluate()` now compares `req.actor` against `review.submitted_by` before the behind-main check and returns `Err(MergeGateError::ReviewerIsMerger)` on match
- Three tests added in `crates/mika-agent/tests/merge_gate_test.rs` — `gate_allows_merge_when_actor_differs_from_reviewer`, `gate_allows_merge_for_dependabot`, `gate_returns_ok_on_clean_ci` — all assert the positive (allow) path via `assert!(evaluate(&req, &review).is_ok())`
- No test exercises the rejection path (actor == submitted_by); the core behavior introduced by this PR is untested

PIPELINE: pass (mika)
- scripts/verify-pipeline.sh → exit 0
  Pipeline verification passed. Plan: docs/plans/2026-09-09-001-merge-gate-self-merge-plan.md Compound: <none>

PLAN-AC VERIFICATION:
Plan: docs/plans/2026-09-09-001-merge-gate-self-merge-plan.md
ACs evaluated: 3
- [✅] satisfied: `pr_merge_with_gate` refuses a merge when the merging actor is the actor that posted the approving review: `evaluate()` compares `req.actor` against `review.submitted_by` and returns `Err(MergeGateError::ReviewerIsMerger)` on match — confirmed in merge_gate.rs diff
- [✅] satisfied: The refusal carries a distinct reason token so the caller can branch on it: `ReviewerIsMerger` variant added to `MergeGateError` enum — confirmed in merge_gate.rs diff
- [⏭️] CI-deferred: No test regressions — `cargo test` passes
- [✅] implicit structural: no parallel plan files in docs/plans/

BUILD VERIFICATION: skipped (no Behavioral ACs in plan)

FINDINGS:
- The self-merge rejection path — the core behavior this PR introduces — has no test coverage. All three added tests assert the positive (allow) path. No test constructs a request where `actor == review.submitted_by` to verify `Err(MergeGateError::ReviewerIsMerger)` is returned. Recommend adding a test such as `gate_rejects_merge_when_actor_equals_reviewer` that asserts `evaluate()` returns `Err(MergeGateError::ReviewerIsMerger)` when actor and reviewer match. This is a test-coverage recommendation, not a gating finding.

VERDICT: pass
DEPTH: code-level
REASON: Pipeline green, diff review clean, all plan ACs satisfied. Noting missing test coverage for the self-merge rejection path as a finding.