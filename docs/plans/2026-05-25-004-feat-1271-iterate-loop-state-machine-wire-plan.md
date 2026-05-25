# Iterate-loop state machine: wire `_iterate_groom_loop` behind feature flag (mika#1271)

**Ticket:** mika#1271 — Contract refactor: pilot owns content; dispatch-lib owns git workflow + iterate loop.
**Architect verdict:** `flip` on session `0583a902-cd7a-45ab-89be-59e13c8b09ec`; v1 scope `yes`.
**Sub-PR sequence:** Third sub-PR of mika#1271.
  - PR#1273 (merged `f2bef21`): `_post_flight_push` → `_push_branch` rename.
  - PR#1274 (merged `1eb5a03`): Phase A/B/C primitives (`_arch_ask`, `_parse_disposition`, `_parse_verdict`, `_trail_append`/`_trail_read`).
  - **This PR**: Phase D state machine (`_iterate_groom_loop`) + Phase F feature-flag wiring.

## Goal

Add the `_iterate_groom_loop` state machine and wire it conditionally into `dispatch_claude_pilot` behind `MIKA_DISPATCH_USE_ITERATE_LOOP=1`. v1 scope: READY → second-pass → GROOMED finalize ONLY. ITERATE and ESCALATE flows are out-of-v1 (WARN + return non-zero, fall through to existing pilot-owns-architect path).

The state machine validates the architect-call-from-dispatch-lib pattern works against real `mika ask` responses. On a real exercise with `MIKA_DISPATCH_USE_ITERATE_LOOP=1` set, the loop invokes mika-arch first-pass + second-pass directly, captures the verdict trail, and on GROOMED returns 0. The existing Class D body-callout shim still writes the canonical callout downstream — the state machine's role in v1 is the convergence-validation pass, not the callout writer.

## Inline corrections to the prior sub-PR

**`_arch_ask` flag-name fix** — PR#1274 shipped `_arch_ask` using `--skill <name>`. The actual `mika ask` flag is `--enable-skill <name>` (verified via `mika ask --help`). Bug never observed in production because no call site exercised `_arch_ask`. This PR fixes the flag and adds an argv-capture test (`mika()` stub function captures argv into a `|`-joined string, asserted against). The fix lands together with the first real call site in `_iterate_groom_loop`.

Per `feedback_smoke_before_claiming_done`: I should have validated the flag at write-time in PR#1274. Calibration recorded.

## Three load-bearing flows (the contract, restated)

1. **READY → second-pass → finalize.** First-pass returns `Disposition: READY`. State machine invokes `mika-arch-second-review` against the same plan on the continuing architect session. On `Verdict: GROOMED`, return 0; the Class D shim writes the canonical callout downstream.
2. **ITERATE → revise-payload → second-pass.** First-pass returns `Disposition: ITERATE`. **Out-of-v1.** v1 emits a WARN naming this as a follow-up and returns 1 so dispatch falls through to the existing pilot-owns-architect path. Follow-up sub-PR adds the pilot relaunch with revise-payload.
3. **ESCALATE → fail-loudly per mika#1033.** First-pass returns `Disposition: ESCALATE` OR second-pass returns `Verdict: ESCALATE`. **Out-of-v1.** v1 emits a WARN and returns 1 (fall-through). Follow-up sub-PR writes the structured PIPELINE_FAILURE marker.

## Implementation

### Changes in this PR

**`skills/bundled/_shared/dispatch-lib.sh`:**
- Fix `_arch_ask`: `--skill` → `--enable-skill` (one-character correction inside the args array).
- New function `_iterate_groom_loop()` between `_trail_read` and `_deliver_callback`.
- Wiring point in `dispatch_claude_pilot`: between `_run_claude_pilot "$ENTRY_COMMAND"` and `_push_branch`, a conditional block that invokes `_iterate_groom_loop` when `SKILL == "dev-groom" && MIKA_DISPATCH_USE_ITERATE_LOOP == "1"`. The loop's return code is logged but does not alter dispatch flow — if the loop fails, the existing path continues unchanged.

**`skills/bundled/_shared/test-dispatch-lib.sh`:**
- 24 new test assertions covering: `_arch_ask` argv construction (argv-capture stub), `_iterate_groom_loop` guard rejections (no WORKTREE_DIR / no ISSUE_NUM / no REPO / no plan file), code-shape inspection (READY/ITERATE/ESCALATE branches present, GROOMED check present, both architect skills invoked, session_id threading), and `dispatch_claude_pilot` wiring inspection (feature flag read, dev-groom guard, ordering between `_run_claude_pilot` and `_push_branch`).

### Behavior unchanged when feature flag is off

`MIKA_DISPATCH_USE_ITERATE_LOOP` defaults to unset. With the default, `_iterate_groom_loop` is never called and the pilot-owns-architect path runs as before. Autonomous dispatch (mika-dev's webhook → `run_claude_pilot_groom`) does not set the flag; production behavior is preserved.

### Feature flag rollout strategy

Phase 1 (this PR): flag-off by default. Operator-driven exercise via `MIKA_DISPATCH_USE_ITERATE_LOOP=1 mika ask --agent mika-dev "groom mika#X"` for tickets known to be READY-on-first-pass.

Phase 2 (follow-up PRs): flip default-on for operator-driven flows once the READY path is exercised reliably; autonomous flows opt in next.

Phase 3 (final sub-PR of mika#1271): remove the flag, retire the existing pilot-owns-architect path, retire Class D body-callout shim, ship the canonical callout writer.

## Acceptance criteria

- [ ] **AC1:** `_arch_ask` invokes `mika ask` with `--enable-skill <skill>` (not `--skill`). Verified by argv-capture test using a stubbed `mika()` shell function that joins argv with `|`.
- [ ] **AC2:** `_iterate_groom_loop` is defined in `skills/bundled/_shared/dispatch-lib.sh` with all three disposition branches (READY/ITERATE/ESCALATE) and a GROOMED check on the second-pass path. Code-shape tests verify presence of each branch.
- [ ] **AC3:** `_iterate_groom_loop` guards on missing `WORKTREE_DIR` / `ISSUE_NUM` / `REPO` and on no-plan-file-present; returns 1 in all four cases with a WARN log line on stderr.
- [ ] **AC4:** `dispatch_claude_pilot` reads `MIKA_DISPATCH_USE_ITERATE_LOOP` and invokes `_iterate_groom_loop` only when the flag is `1` AND `SKILL == "dev-groom"`. Default behavior unchanged.
- [ ] **AC5:** Wiring ordering: `_iterate_groom_loop` is called AFTER `_run_claude_pilot` returns and BEFORE `_push_branch`. Verified by a line-position test against `declare -f dispatch_claude_pilot`.
- [ ] **AC6:** `bash -n` syntax check passes on both `dispatch-lib.sh` and `test-dispatch-lib.sh`.
- [ ] **AC7:** 24 new test assertions pass. Pre-existing failure count (6 on main) unchanged — no regressions.

## Risks

- **`_arch_ask` exit code on `mika` failure.** `_arch_ask` returns whatever `mika ask` returns. If mika exits non-zero (network failure, auth, etc.), `_iterate_groom_loop` catches it via `|| { ...; return 1; }` and falls through. The existing path then runs.
- **JSON parse failures.** If `mika ask --format json --verbose` returns malformed JSON, `jq -r '.content // empty'` returns empty string. The loop checks for non-empty `.content` and `.metadata.session_id` and bails with WARN. No silent failure.
- **Architect-session continuity.** The state machine threads `session_id` from first-pass response into second-pass invocation. If the architect's CLI changes the JSON envelope shape, the threading breaks silently — the second-pass would start a fresh session, the verdict still produces a GROOMED/ESCALATE answer but without continuity. Acceptable for v1; full continuity validation lands when the ITERATE flow exercises multi-call session reuse.
- **Plan file lookup race.** `find docs/plans -name "*-${ISSUE_NUM}-*-plan.md"` returns most-recent first. If the pilot wrote multiple plans for the same issue, we use the most recent. Same heuristic as Class D — known acceptable.

## Out of v1 (sub-PR follow-ups)

- **ITERATE flow with pilot relaunch + revise-payload** — next sub-PR.
- **ESCALATE flow with structured PIPELINE_FAILURE marker** — next sub-PR.
- **Canonical body-callout writer** (`_write_canonical_callout`) — after ITERATE/ESCALATE flows; depends on verdict-trail reader logic.
- **mika#1272** (paraphrased dispositions) — separate ticket; queued after the state machine is exercising.
- **Class D body-callout shim retire** — depends on the canonical writer landing.
- **Feature flag removal** — final sub-PR of mika#1271.

## Test plan

Same as AC1–AC7. All structural; no integration test invokes real `mika ask` against architect (would cost real API calls). Operator-driven exercise with `MIKA_DISPATCH_USE_ITERATE_LOOP=1` is the live validation.

## Provenance

- mika#1271 parent ticket, milestone#26.
- Architect contract verdict: session `0583a902-cd7a-45ab-89be-59e13c8b09ec` (rounds: `flip` / Class D `(i) Retire` / v1 scope `yes`).
- Previous sub-PRs: PR#1273 (`f2bef21`), PR#1274 (`1eb5a03`).
- Empirical contract-violation evidence (the underlying motivation for the refactor): pilot session logs `9fb5c2bd` (mika#1263 groom), `b9c8f517` (mika#1269 impl), `1a45de67` (mika#1268 impl), `5a1d583d` (mika#1267 trajectory probe) — all showing zero `git commit` invocations.
