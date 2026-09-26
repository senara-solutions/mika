# Iterate-loop ESCALATE flow: structured PIPELINE FAILURE marker (mika#1271)

**Ticket:** mika#1271 — Contract refactor: pilot owns content; dispatch-lib owns git workflow + iterate loop.
**Architect verdict:** `flip` on session `0583a902-cd7a-45ab-89be-59e13c8b09ec`; v1 scope `yes`.
**Sub-PR sequence:** **Fifth sub-PR of mika#1271.**
  - PR#1273 (`f2bef21`), PR#1274 (`1eb5a03`), PR#1275 (`30917d7`), PR#1276 (`96fd2281`), companion `mika-platform e9c7060`.
  - **This PR**: ESCALATE flow — three call sites converted from WARN+fall-through to structured PIPELINE FAILURE per mika#1033 precedent.

## Goal

Replace the three remaining WARN+return-1 ESCALATE call sites in `_iterate_groom_loop` with a structured failure path that (1) preserves architect findings to `$WORKTREE_DIR/.iterate/escalate-<stage>.md` for operator forensic access, and (2) appends a structured `PIPELINE FAILURE: groom escalated by mika-arch <stage>` marker to `RESULT` so the callback delivers an actionable error instead of a generic "no PR" message.

This honors the mika#1033 precedent: detect-and-fail-loudly for ESCALATE. Per the architect-prompt contract (verified in sub-PR 4):
- First-pass `Disposition: ESCALATE` → human decision required, no iteration.
- Second-pass `Verdict: ESCALATE` → terminal, no third pass.
- The state machine MUST surface both as structured failures, not as silent fall-throughs.

## Three ESCALATE call sites

In current `_iterate_groom_loop` (post-PR#1276), three branches reach ESCALATE state and currently WARN+return-1:

1. **First-pass `ESCALATE)`** — architect returned `Disposition: ESCALATE` on the initial review.
2. **READY → second-pass `*)` non-GROOMED** — first-pass said READY, but second-pass returned non-GROOMED (i.e., `Verdict: ESCALATE`).
3. **ITERATE → second-pass `*)` non-GROOMED** — first-pass said ITERATE, pilot revised, but second-pass on the revision returned non-GROOMED.

All three are now wired to `_escalate_groom <stage> <content> <session_id>`. Stage labels: `first-pass`, `second-pass-after-ready`, `second-pass-after-iterate`.

## Implementation

### New helper: `_escalate_groom <stage> <content> <session_id>`

Defined just before `_iterate_groom_loop` in `dispatch-lib.sh`. Does two things:

1. **Write architect rationale to disk.** `$WORKTREE_DIR/.iterate/escalate-<stage>.md` captures the architect's content verbatim. Best-effort; silent on filesystem failures (the RESULT marker is the mandatory product).
2. **Append structured marker to RESULT.** Four lines:
   ```
   PIPELINE FAILURE: groom escalated by mika-arch <stage>.
   Verdict: ESCALATE — human review required.
   Session: <session_id>
   Architect findings preserved at: <findings_file>
   ```
   These follow the existing line-based RESULT convention (`STATUS=` / `PIPELINE FAILURE:` / `Push:` lines elsewhere in dispatch-lib).

### Findings preservation, NOT cleanup

Per friend-peer sharpening in sub-PR 4: **sweep on GROOMED, preserve on ESCALATE.** This PR preserves findings on all three ESCALATE paths. `_cleanup_iterate_findings` is called exclusively on the two GROOMED success paths (verified by `grep -c` in the test suite — must be exactly 2). Worktree TTL handles eventual cleanup; the findings file is the operator's primary forensic artifact at exactly the moment of ESCALATE.

### Behavior unchanged when feature flag is off

`MIKA_DISPATCH_USE_ITERATE_LOOP` defaults to unset. ESCALATE handling under the existing pilot-owns-architect path (Class D body-callout shim + downstream) is untouched. This PR only affects flow under the feature flag.

## Acceptance criteria

- [ ] **AC1:** `_escalate_groom <stage> <content> <session_id>` is defined in `skills/bundled/_shared/dispatch-lib.sh`. Writes `$WORKTREE_DIR/.iterate/escalate-<stage>.md` and appends a structured PIPELINE FAILURE block to `RESULT`.
- [ ] **AC2:** Three ESCALATE call sites in `_iterate_groom_loop` use `_escalate_groom`: first-pass ESCALATE → `_escalate_groom "first-pass"`; READY-then-second-pass-non-GROOMED → `_escalate_groom "second-pass-after-ready"`; ITERATE-then-second-pass-non-GROOMED → `_escalate_groom "second-pass-after-iterate"`.
- [ ] **AC3:** Preservation invariant — `_cleanup_iterate_findings` is called exactly twice in `_iterate_groom_loop` (the two GROOMED success paths only). No cleanup call adjacent to or downstream of any `_escalate_groom` call.
- [ ] **AC4:** RESULT marker structure verified — includes `PIPELINE FAILURE: groom escalated by mika-arch <stage>`, `Verdict: ESCALATE — human review required`, `Session: <session_id>`, and `Architect findings preserved at: <path>` lines.
- [ ] **AC5:** `_escalate_groom` is defensive on missing `WORKTREE_DIR` — findings-file write is best-effort, RESULT marker still populates. (Robustness for callers that hit ESCALATE in a state where `WORKTREE_DIR` is somehow unset.)
- [ ] **AC6:** `bash -n` syntax check passes on `dispatch-lib.sh` and `test-dispatch-lib.sh`.
- [ ] **AC7:** 13 new test assertions pass. Pre-existing failure count (6 on main) unchanged.

## Verified invariants from prior sub-PRs (still hold)

- No plan-on-origin coupling between architect passes (verified via grep in sub-PR 4).
- `.iterate/` directory ownership exclusive to dispatch-lib (verified in sub-PR 4).
- session-id symmetry across READY and ITERATE branches (test counts `mika-arch-second-review` invocations = exactly 2).
- Cleanup-on-GROOMED-only (test counts `_cleanup_iterate_findings` calls = exactly 2).

## What does NOT ship in this sub-PR

- **Canonical body-callout writer** (`_write_canonical_callout`) — Class D shim still writes downstream on GROOMED success. Next sub-PR.
- **mika#1272** (paraphrased dispositions) — the `*)` default branch in first-pass case-switch still WARN+return-1 for unparsed disposition. Separate ticket; the structured PIPELINE FAILURE path for paraphrased dispositions ships when mika#1272 lands.
- **Class D body-callout shim retire** — final sub-PR.
- **Feature flag removal** — terminal sub-PR.

## Test plan

All AC items verified by tests in `test-dispatch-lib.sh`. No integration test invokes real `mika ask`. Operator-driven exercise on a ticket that's known to trigger architect ESCALATE (rare in practice; manual stage-via-fixture is the realistic validation path).

## Provenance

- mika#1271 parent ticket, milestone#26.
- Sequence: PRs #1273 → #1274 → #1275 → #1276 → **this**.
- Architect contract: session `0583a902-cd7a-45ab-89be-59e13c8b09ec` (`flip` / `(i) Retire` / `yes`).
- mika#1033 — detect-and-fail-loudly precedent for dev-groom drift class; the structured PIPELINE FAILURE shape used here mirrors mika#1033's existing markers.
- Friend-peer sharpenings (sub-PR 4): preserve-on-ESCALATE invariant, session-id symmetry. Both honored unchanged in this PR.
