# Remove `MIKA_DISPATCH_USE_ITERATE_LOOP` feature flag (mika#1271)

**Ticket:** mika#1271 — Contract refactor: pilot owns content; dispatch-lib owns git workflow + iterate loop.
**Architect verdict:** `flip` on session `0583a902-cd7a-45ab-89be-59e13c8b09ec`; v1 scope `yes`.
**Sub-PR sequence:** **Seventh sub-PR of mika#1271 (sub-PR 7a — flag removal only).**
  - PRs #1273 (`f2bef21`), #1274 (`1eb5a03`), #1275 (`30917d7`), #1276 (`96fd2281`), #1277 (`9c20096e`), #1278 (`f58b5450`).
  - **This PR (7a)**: remove `MIKA_DISPATCH_USE_ITERATE_LOOP` flag → iterate loop runs unconditionally for `dev-groom`. Class D recovery shim STAYS as defense-in-depth.
  - **Sub-PR 7b (follow-up)**: retire Class D shim after 7a soaks in production.

## Scope-split rationale

Sub-PR 7 was originally scoped as "remove flag + retire Class D shim" in one PR. Splitting into 7a + 7b for safety:

- **The canonical writer has never been runtime-exercised in production.** The feature flag has been off by default since sub-PR 3. All evidence is from structural tests, not real `gh issue edit` round-trips against the architect.
- **7a flips the switch to enable production exercise** while keeping Class D as a safety net. If the iterate loop fails to write a callout for any reason (gh API error, idempotency check false-positive, etc.), Class D recovery still catches the drift and surfaces it to the operator.
- **7b retires Class D after 7a soaks** — once we have evidence the canonical writer covers every case the recovery shim caught, the redundancy can be removed.

Per memory `feedback_loop_stability_beats_loop_speed`: stability beats wall-clock; the 1-PR split is the safer call.

## What changes

### Removed: feature-flag gate

Line 1322-1329 of `dispatch-lib.sh` (call site of `_iterate_groom_loop`) was:

```bash
if [ "$SKILL" = "dev-groom" ] && [ "${MIKA_DISPATCH_USE_ITERATE_LOOP:-0}" = "1" ]; then
    _iterate_groom_loop || echo "info: iterate_groom_loop did not converge — falling through to existing path" >&2
fi
```

Becomes:

```bash
if [ "$SKILL" = "dev-groom" ]; then
    _iterate_groom_loop || echo "info: iterate_groom_loop did not converge — Class D recovery (mika#1123) downstream catches drift" >&2
fi
```

### Comment updates

- `_iterate_groom_loop` docstring updated to describe the five terminal states explicitly + the always-on contract.
- Wiring-point comment in `dispatch_claude_pilot` updated to reflect that the flag is gone and the Class D shim is still active.

### Tests

- Removed: `dispatch_claude_pilot reads MIKA_DISPATCH_USE_ITERATE_LOOP flag` (no longer applicable).
- Added: `dispatch_claude_pilot no longer references MIKA_DISPATCH_USE_ITERATE_LOOP flag` (via `assert_not_contains`).
- Added: `dispatch_claude_pilot still gates iterate-loop on dev-groom skill` (regression guard for the remaining gate).
- Added: `_run_claude_pilot still invokes Class D recovery shim (defense-in-depth)` (records 7a's defense-in-depth contract).

## Acceptance criteria

- [ ] **AC1:** `MIKA_DISPATCH_USE_ITERATE_LOOP` is gone from `dispatch-lib.sh` (`grep -c` returns 0).
- [ ] **AC2:** `_iterate_groom_loop` is invoked unconditionally for the `dev-groom` skill (no flag check, only `SKILL = "dev-groom"`).
- [ ] **AC3:** `_verify_and_write_body_callout` Class D recovery shim is STILL present and STILL invoked from `_run_claude_pilot` (sub-PR 7a defense-in-depth).
- [ ] **AC4:** All prior canonical-writer + escalate-groom invariants from sub-PRs 5 and 6 hold unchanged (call counts: 2 canonical-write, 3 escalate, 2 cleanup).
- [ ] **AC5:** `bash -n` passes on both `dispatch-lib.sh` and `test-dispatch-lib.sh`.
- [ ] **AC6:** Pre-existing test failure count (6) unchanged. Net new test count = +1.

## Behavioral contract under 7a

Production behavior after 7a lands:

1. `_run_claude_pilot` runs the dev-groom pilot. Pilot may write its organic callout block on success.
2. After the pilot exits, `_verify_and_write_body_callout` runs (Class D recovery). If pilot drifted (no callout written), recovery callout is prepended.
3. `_iterate_groom_loop` runs unconditionally. Invokes architect first-pass + (READY → second-pass) | (ITERATE → revise → second-pass). On GROOMED → `_write_canonical_callout` prepends the canonical block. On ESCALATE → structured PIPELINE FAILURE marker.
4. `_push_branch` + `_deliver_callback` finalize.

In the GROOMED case, the body may now carry up to three callout blocks (organic, recovery, canonical) — last-written-first. The dispatch gate's `has_verdict` regex matches the canonical block's `second-pass (GROOMED)` marker → gate passes. Operator-visible noise (multiple callout blocks) is the price of 7a's defense-in-depth posture; 7b cleans it up.

## What does NOT ship in this sub-PR (7b scope)

- **`_verify_and_write_body_callout` retirement.** Function definition + call site in `_run_claude_pilot` stay. Sub-PR 7b removes both.
- **dev-groom pilot prompt changes.** The pilot still invokes its own architect; the iterate loop's architect is redundant in production. Cost optimization, not correctness — addressed when (and only when) the pilot's contract is updated to "content-only."
- **Guard-failure shape changes.** The four `return 1` guards in `_iterate_groom_loop` (missing WORKTREE_DIR / ISSUE_NUM / REPO / plan file) still WARN+return-1 silently. Once Class D shim is gone (7b), these should produce PIPELINE FAILURE markers — but until then, Class D catches the missing-body-callout case, so the silent return is harmless.

## Test plan

Structural tests in `test-dispatch-lib.sh`:

- `MIKA_DISPATCH_USE_ITERATE_LOOP` absent from `dispatch_claude_pilot` source.
- `dev-groom` still gates the iterate-loop call.
- `_verify_and_write_body_callout` still invoked from `_run_claude_pilot` source.
- All previous canonical/escalate/cleanup call counts unchanged.

No runtime test invokes real `mika ask` or `gh issue edit` — production exercise is the operator's first observation surface for sub-PR 7a, by design.

## Provenance

- mika#1271 parent ticket, milestone#26.
- Sequence: PRs #1273 → #1274 → #1275 → #1276 → #1277 → #1278 → **this**.
- Architect contract: session `0583a902-cd7a-45ab-89be-59e13c8b09ec` (`flip` / `(i) Retire` / `yes`).
- mika#1123 — Class D recovery shim (defense-in-depth retained in 7a).
- Memory: `feedback_loop_stability_beats_loop_speed` — stability over speed; scope-split rationale.
