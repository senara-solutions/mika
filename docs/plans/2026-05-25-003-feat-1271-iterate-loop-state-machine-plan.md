# Iterate-loop state machine in dispatch-lib (mika#1271)

**Ticket:** mika#1271 — Contract refactor: pilot owns content; dispatch-lib owns git workflow + iterate loop.
**Architect verdict:** `flip` (session `0583a902-cd7a-45ab-89be-59e13c8b09ec`); v1 scope `yes`.
**Parent context:** Second sub-deliverable of mika#1271, following the `_post_flight_push` → `_push_branch` rename (PR #1273, merged 2026-05-25 14:12:07Z, commit `f2bef21`).

## Goal

Move architect-verdict-reading and iterate-loop decisioning out of claude-pilot's slash-command logic (`/mika-groom-ticket`) and into `dispatch-lib.sh`. Under the new contract, the pilot's contract is content-only (write or revise the plan); dispatch-lib owns the verdict-driven workflow.

## Three load-bearing flows (the contract this PR commits to)

### 1. `READY → second-pass → finalize`

`mika-arch-groom-ticket` returns `Disposition: READY`. dispatch-lib:
1. Invokes `mika-arch-second-review` against the same plan-on-branch (continuing the architect session).
2. On `Verdict: GROOMED`, writes the canonical body callout (Branch / Plan / Grooming history with full verdict trail), runs `_push_branch`, delivers callback.
3. On `Verdict: ESCALATE` from second-pass, follow the ESCALATE flow below.

### 2. `ITERATE → revise-payload → second-pass`

`mika-arch-groom-ticket` returns `Disposition: ITERATE` with findings. dispatch-lib:
1. Re-launches the pilot with explicit content-only payload `revise the plan to address: <findings>` (continuing the architect session so verdict trail is preserved).
2. After pilot revises and exits, re-invokes `mika-arch-groom-ticket` for re-evaluation (NOT directly to second-pass — the revised plan needs a fresh first-pass).
3. If new disposition is READY → flow 1. If still ITERATE → loop (bounded by `MAX_ITERATE_ROUNDS`, surfaced as a named constant; default 3). If ESCALATE → flow 3.
4. On exhaustion (`N` ITERATE rounds without convergence), follow ESCALATE flow.

### 3. `ESCALATE → fail-loudly per mika#1033`

Any `Disposition: ESCALATE` from first-pass OR `Verdict: ESCALATE` from second-pass OR ITERATE-bound exhaustion. dispatch-lib:
1. Writes a structured `PIPELINE FAILURE: groom escalated — <reason>` marker to RESULT.
2. Marks the task escalated.
3. Surfaces architect findings in the callback to the operator. **No retry.** Mirrors mika#1033's "detect-and-fail-loudly" precedent for the dev-groom drift class.

## Implementation shape

### Phase A — architect-call helper

New `_arch_ask()` function in `dispatch-lib.sh`:

```
_arch_ask() {
  local skill="$1"       # mika-arch-groom-ticket | mika-arch-second-review
  local plan_path="$2"   # absolute path to plan file on disk
  local session_id="$3"  # optional — continues existing arch session

  local args=( --agent mika-arch --format json --verbose )
  [ -n "$session_id" ] && args+=( --session-id "$session_id" )
  args+=( "@${plan_path}" )

  # Returns JSON to stdout; caller parses .content and .metadata.session_id.
  mika ask "${args[@]}"
}
```

### Phase B — verdict parser

New `_parse_disposition()` and `_parse_verdict()` helpers:

```
_parse_disposition() {
  # Reads architect response from stdin, emits READY|ITERATE|ESCALATE on stdout.
  # Tolerates literal Disposition: lines only in v1.
  # Paraphrased dispositions handling is out-of-v1 (mika#1272, sub-issue).
  grep -oE 'Disposition:[[:space:]]*(READY|ITERATE|ESCALATE)' \
    | grep -oE '(READY|ITERATE|ESCALATE)' \
    | head -1
}

_parse_verdict() {
  # Same shape but for second-pass: GROOMED|ESCALATE.
  grep -oE 'Verdict:[[:space:]]*(GROOMED|ESCALATE)' \
    | grep -oE '(GROOMED|ESCALATE)' \
    | head -1
}
```

### Phase C — verdict trail capture

Append-only file at `$WORKTREE_DIR/.claude/groom-verdict-trail.log` capturing each architect call's session_id, skill, and disposition/verdict. Used as input for the Grooming history callout field. Discarded after callout is written.

### Phase D — the state machine itself

New `_iterate_groom_loop()` function. Pseudocode:

```
_iterate_groom_loop() {
  local plan_path="$1"
  local arch_session_id=""
  local round=0

  while [ "$round" -lt "${MAX_ITERATE_ROUNDS:-3}" ]; do
    # First-pass
    local resp; resp=$(_arch_ask mika-arch-groom-ticket "$plan_path" "$arch_session_id")
    arch_session_id=$(echo "$resp" | jq -r '.metadata.session_id')
    local content; content=$(echo "$resp" | jq -r '.content')
    local disposition; disposition=$(echo "$content" | _parse_disposition)
    _trail_append "groom-ticket" "$arch_session_id" "$disposition"

    case "$disposition" in
      READY)
        # Second-pass
        local resp2; resp2=$(_arch_ask mika-arch-second-review "$plan_path" "$arch_session_id")
        local content2; content2=$(echo "$resp2" | jq -r '.content')
        local verdict; verdict=$(echo "$content2" | _parse_verdict)
        _trail_append "second-review" "$arch_session_id" "$verdict"

        case "$verdict" in
          GROOMED)
            _write_canonical_callout "$plan_path" "$arch_session_id"
            return 0
            ;;
          ESCALATE|*)
            _escalate "second-pass returned $verdict"
            return 1
            ;;
        esac
        ;;
      ITERATE)
        # Pilot revises with findings as payload
        _launch_revise_pilot "$plan_path" "$content"
        round=$((round + 1))
        continue
        ;;
      ESCALATE|*)
        _escalate "first-pass returned $disposition"
        return 1
        ;;
    esac
  done

  _escalate "$MAX_ITERATE_ROUNDS ITERATE rounds without convergence"
  return 1
}
```

### Phase E — wiring

In `dispatch_claude_pilot()`, after `_run_claude_pilot "$ENTRY_COMMAND"` returns:
- If `SKILL == "dev-groom"` AND a plan file exists on disk: call `_iterate_groom_loop`.
- Otherwise: existing path (dev-pilot impl; no architect loop).

`_push_branch` is called after `_iterate_groom_loop` returns (success or fail), so any committed plan revisions land on origin regardless of verdict.

### Phase F — feature flag

`MIKA_DISPATCH_USE_ITERATE_LOOP=1` gates the new flow. Default unset (uses current pilot-owns-architect path) until the state machine is exercising correctly in operator runs. When proven stable, the flag flips to default-on, then is removed.

## v1 PR scope (this PR)

**In scope (this PR — minimal shippable slice of the contract refactor):**

- Phase A (`_arch_ask`), Phase B (`_parse_disposition`, `_parse_verdict`), Phase C (`_trail_append`, `_trail_read`), Phase D skeleton of `_iterate_groom_loop` with **READY flow only** (no ITERATE re-launch, no ESCALATE handling beyond a stub that fails through), Phase F (feature flag gate).
- Wiring point in `dispatch_claude_pilot` reads `MIKA_DISPATCH_USE_ITERATE_LOOP` and dispatches to either the new loop or the existing path. v1: only READY → GROOMED finalize is actually wired; ITERATE/ESCALATE fall through to the existing pilot-owns-architect path with a WARN log line.

**Out of v1 (follow-up PRs against mika#1271):**

- ITERATE flow with revise-payload pilot relaunch (Phase D's ITERATE branch).
- ESCALATE flow with structured failure marker (Phase D's ESCALATE branch).
- Canonical body callout writer (Phase D's `_write_canonical_callout`) — for v1, the existing Class D body-callout shim still writes the callout.
- mika#1272 (paraphrased dispositions) — separate ticket.
- Class D body-callout shim retire — separate sub-PR, depends on `_write_canonical_callout` landing first.

The v1 cut makes the state machine **demonstrably exercising correctly** on the simplest non-trivial path (READY first-pass + GROOMED second-pass) without requiring the full machinery to land in one commit. Day-14 readout reads this as "state-machine convergence visible in dispatch-lib."

## Acceptance criteria

- [ ] **AC1:** Helpers `_arch_ask`, `_parse_disposition`, `_parse_verdict`, `_trail_append`, `_trail_read` are defined in `skills/bundled/_shared/dispatch-lib.sh` with behavior matching the Phase A/B/C specifications above.
- [ ] **AC2:** `bash -n` syntax check passes on both `dispatch-lib.sh` and `test-dispatch-lib.sh`.
- [ ] **AC3:** 15 new test assertions in `test-dispatch-lib.sh` pass. Pre-existing failure count (6 on main) unchanged — no regressions.
- [ ] **AC4:** No call sites added in the live dispatch path (`dispatch_claude_pilot`, `_run_claude_pilot`). The primitives are unreachable from production flows until a follow-up PR wires them via the `MIKA_DISPATCH_USE_ITERATE_LOOP` feature flag.
- [ ] **AC5:** Plan doc at `docs/plans/2026-05-25-003-feat-1271-iterate-loop-state-machine-plan.md` documents the full contract (three flows, Phases A–F, v1 scope cut, follow-up sub-PR sequencing).

## Risks

- **Architect-session continuity across calls.** Phase A passes `--session-id` from the first call's response to the second call. Bash `jq -r` on `.metadata.session_id` must work against the live `mika ask --format json --verbose` envelope. Verified manually today (sessions `0583a902` and `eebb22d8` extracted cleanly).
- **Verdict-line position in architect response.** First-pass grooming prompt instructs literal `Disposition: <X>`. Today's empirical evidence: mika-arch occasionally paraphrases. v1 tolerates literal-only; paraphrased handling is mika#1272.
- **Plan-path detection on disk.** `_iterate_groom_loop` needs to find the plan file. Reuses existing `find $WORKTREE_DIR/docs/plans -name "*-${ISSUE_NUM}-*-plan.md"` pattern from `_verify_and_write_body_callout`.
- **Feature flag rollout.** v1 ships behind `MIKA_DISPATCH_USE_ITERATE_LOOP=1` so the existing autonomous loop continues to work. Operator-driven runs can set the flag to exercise the new path; failures don't regress production.

## Test plan

- v1: manual exercise on a real ticket with `MIKA_DISPATCH_USE_ITERATE_LOOP=1` against a plan that's already at READY-on-first-pass. Verify the new loop runs second-pass and produces GROOMED.
- bash `-n` syntax check (CI catches this via shellcheck on lint).
- `bash skills/bundled/_shared/test-dispatch-lib.sh` if existing test surface covers it.

## Provenance

- Architect contract verdict: session `0583a902-cd7a-45ab-89be-59e13c8b09ec`, three rounds (`flip` / Class D `(i) Retire` / v1 scope `yes`).
- Parent ticket: mika#1271 (filed 2026-05-25, milestone#26).
- First sub-PR: mika PR#1273 (rename, merged 2026-05-25 14:12:07Z, commit `f2bef21`).
- Sub-issue for paraphrased dispositions: mika#1272.
- Empirical grounding for the contract flip: pilot session logs `9fb5c2bd` (mika#1263 groom), `b9c8f517` (mika#1269 impl), `1a45de67` (mika#1268 impl), and most recently `5a1d583d` (mika#1267 trajectory probe) — all showing zero `git commit` invocations.
