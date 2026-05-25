---
title: "dev-groom ITERATE → pass-2 autonomous progression"
ticket: mika#1269
type: fix
date: 2026-05-25
---

# Plan: dev-groom ITERATE → pass-2 autonomous progression

## Problem

When the autonomous dev-groom pilot (`/mika-groom-ticket` via claude-pilot) receives `Disposition: ITERATE` from mika-arch first-pass, the inner Claude Code session exits without completing Phase 4 (revisions + second-pass review). Post-flight recovery stamps a placeholder callout (`"architect verdict not verified, operator dispatch required"`) that the `dispatch_no_grooming_marker` gate correctly rejects. Every non-trivial ticket that receives findings on first pass wedges the loop.

**Reproduction (mika#1268):** ready-label → dev-groom dispatch → plan committed → first-pass ITERATE with 1 blocking + 2 non-blocking findings → pilot exits → body gets post-flight recovery callout → re-dispatch fails grooming gate. This is the dominant manual-rescue pattern.

## Root cause (pinned at `0c3c0f15`)

**Hypothesis validation (per issue body):**

- **H1 ("dev-groom skill prompt doesn't instruct ITERATE handling"):** CONFIRMED as a framing error in the issue. Two layers are involved:
  - **Outer layer (mika-dev):** `skills/bundled/dev-groom/system_prompt.md` (lines 1-23 at `0c3c0f15`) is a dispatcher-only prompt. It says "Use `run_claude_pilot_groom` to dispatch...". It does NOT instruct ITERATE handling because it never runs the grooming itself — it delegates to claude-pilot.
  - **Inner layer (claude-pilot session):** The inner session runs `/mika-groom-ticket` (`.claude/commands/mika-groom-ticket.md` in mika-platform, copied to worktree by `dispatch-lib.sh` line 451-453). Phase 3 step 10 (line 183 of the command spec) explicitly handles ITERATE: `"Disposition: ITERATE — apply the architect's specific concerns to the staged plan, then commit...Continue to Phase 4."` Phase 4 (lines 189-206) specifies the full revise → second-pass → GROOMED flow.
  - **Root cause:** The command spec IS correct. The LLM in the headless session reads the ITERATE disposition and treats it as terminal — it considers the grooming "done" (verdict received) and ends its turn without entering Phase 4. This is a premature-EndTurn, not a missing instruction.

- **H2 ("pilot exhausts turn budget"):** RULED OUT by reproduction evidence. mika#1268 pilot completed in 7 minutes with ~18 turns. Default `maxTurns=200` (claude-pilot-py `types.py` line 42). Budget was not the constraint.

- **H3 ("session_id not threaded"):** NOT THE PRIMARY CAUSE. session_id threading is a Phase 4 concern (line 198-200 of the command spec: `--session-id <captured-from-step-9>`). The pilot never reaches Phase 4, so session_id threading is moot. The re-launch approach makes this irrelevant — the second session manages its own architect sessions.

**Operative root cause:** Premature-EndTurn family — same class as dev-pilot mika#931/#938/#939 where the model exits after a milestone completion signal without reaching the next required step. The spec is correct; the model doesn't follow it autonomously under headless execution.

**No claude-pilot structural guard exists for dev-groom.** Dev-pilot has `CLAUDE_PILOT_REQUIRE_PR=1` (dispatch-lib.sh line 846, consumed at agent.py line 157-168) which flips status to `pipeline_incomplete` when the session exits without `gh pr create`. Dev-groom has `CLAUDE_PILOT_MIN_TOOL_CALLS=3` (dispatch-lib.sh line 856) which guards against zero-action exits, but that's too low a bar — the pilot makes 15+ tool calls during Phase 1-3 before exiting on ITERATE.

## Fix strategy: post-flight ITERATE recovery loop

**Design principle:** structural recovery in `dispatch-lib.sh`, not prompt-level hope. Same pattern as `CLAUDE_PILOT_REQUIRE_PR` — when the model doesn't follow through, the infrastructure detects and re-enters.

After the first `_run_claude_pilot` completes, dispatch-lib.sh checks whether the grooming reached GROOMED verdict. If not, and the session log shows the architect returned ITERATE (not ESCALATE or READY), it re-launches claude-pilot with the same entry command and the same prompt. The re-launched session picks up the existing worktree and plan via the idempotency clause in `/mika-groom-ticket` Phase 2 step 5 ("if the worktree exists, the existing plan is reused"). The second pass runs the full `/mika-groom-ticket` flow, which re-reads the issue, finds the committed plan, runs architect review again — this time on an already-iterated plan that should earn READY or GROOMED.

**Why re-launch, not inject iteration context?** The `/mika-groom-ticket` idempotency clause already handles this case. A fresh session with the same prompt sees the worktree + committed plan and picks up where it left off. Injecting a complex iteration-context prompt adds fragile coupling to architect response format. The re-launch is clean: same entry command, same prompt, new session, existing state on disk.

**Bounded:** max 1 re-launch (2 total sessions). If the second session also exits without GROOMED, deliver callback with a `GROOM_ITERATE_EXHAUSTED` marker and let mika-dev's callback handler escalate.

## Units of work

### Unit 1: dispatch-lib.sh — `_iterate_recovery()` function

**File:** `skills/bundled/_shared/dispatch-lib.sh`

Add `_iterate_recovery()` helper, called from `dispatch_claude_pilot()` between `_run_claude_pilot` and `_deliver_callback`.

**Logic:**

```
_iterate_recovery(ENTRY_COMMAND):
  # Guard: only for dev-groom
  if SKILL != "dev-groom": return
  if WORKTREE_DIR empty or not a directory: return

  # Guard: check if GROOMED already reached
  BODY = gh issue view $ISSUE_NUM --repo senara-solutions/$REPO --json body -q .body
  if BODY contains "second-pass (GROOMED)" or "second-pass (READY, paraphrased GROOMED": return

  # Guard: check session log for ITERATE disposition (not ESCALATE, not READY)
  SESSION_LOG = /var/log/claude-pilot/${LOG_ID}.log
  if log not readable: return  # fail-open
  if log does NOT contain "Disposition: ITERATE" (case-insensitive): return
  if log contains "Disposition: ESCALATE": return  # don't retry an ESCALATE

  # Guard: bounded retry
  if GROOM_ITERATE_RETRY >= 1: 
    # Exhausted — prepend structured marker to RESULT for callback handler classification.
    # Include diagnostic references (F4: operator loses fidelity without these).
    RESULT="GROOM_ITERATE_EXHAUSTED: Session received ITERATE on first-pass but
did not reach GROOMED after 2 sessions. Manual intervention required.

First session log: /var/log/claude-pilot/${PREV_LOG_ID:-$LOG_ID}.log
Retry session log: /var/log/claude-pilot/${LOG_ID}.log
Branch: $BRANCH
Worktree: $WORKTREE_DIR
Issue: senara-solutions/$REPO#$ISSUE_NUM

Operator recovery: read the architect findings in the first session log,
revise the plan manually, then re-run /mika-groom-ticket or apply the
ready label after manual verification.

$RESULT"
    return

  GROOM_ITERATE_RETRY=1

  # ── Body callout cleanup (F3 resolution) ──
  # The first _run_claude_pilot's post-flight `_verify_and_write_body_callout`
  # (dispatch-lib.sh lines 162-311) may have prepended a recovery placeholder:
  #   > - **Grooming history:** body callout recovered by post-flight (mika#1123)
  #     — architect verdict not verified, operator dispatch required
  # If the second session writes the canonical callout (Phase 5 step 19, which
  # says "prepend or merge into existing callouts"), it would collide with the
  # placeholder — producing two callout blocks. `check_grooming_markers`
  # (executor.rs line 800) does substring matching, so the placeholder's
  # "architect verdict not verified" line would coexist with the canonical
  # "second-pass (GROOMED)" line, but the body would be visually confusing
  # and the placeholder is misleading once GROOMED is reached.
  #
  # Fix: strip the recovery placeholder from the body BEFORE re-launching.
  # This gives the second session a clean body to write the canonical callout.
  _CURRENT_BODY=$(gh issue view "$ISSUE_NUM" --repo "senara-solutions/$REPO" \
      --json body -q '.body' 2>/dev/null || true)
  if printf '%s' "$_CURRENT_BODY" | grep -qF "body callout recovered by post-flight"; then
      _CLEANED_BODY=$(printf '%s' "$_CURRENT_BODY" \
          | sed '/^> - \*\*Branch:\*\*/d' \
          | sed '/^> - \*\*Plan:\*\*/d' \
          | sed '/^> - \*\*Grooming history:\*\* body callout recovered/d' \
          | sed '/^$/{ N; /^\n$/d; }')  # collapse double blank lines
      _TMPFILE=$(mktemp /tmp/iterate-recovery-body-XXXXXX.md)
      printf '%s' "$_CLEANED_BODY" > "$_TMPFILE"
      gh issue edit "$ISSUE_NUM" --repo "senara-solutions/$REPO" \
          --body-file "$_TMPFILE" 2>/dev/null || true
      rm -f "$_TMPFILE"
      echo "iterate_recovery: stripped recovery placeholder from issue body" >&2
  fi

  # Re-launch: same entry command, same prompt, new LOG_ID for separate log file
  PREV_LOG_ID=$LOG_ID
  LOG_ID="${TASK_ID}-retry-$(date +%s)"
  # Preserve PRE_RUN_HEAD from the first run (plan commits are already there)
  PRE_RUN_HEAD=$(git -C "$WORKTREE_DIR" rev-parse HEAD 2>/dev/null || true)
  
  echo "iterate_recovery: re-launching claude-pilot for pass-2 (prev session: $PREV_LOG_ID)" >&2
  _run_claude_pilot "$ENTRY_COMMAND"
```

**Call site in `dispatch_claude_pilot()`:**

```bash
_run_claude_pilot "$ENTRY_COMMAND"
_iterate_recovery "$ENTRY_COMMAND"
_deliver_callback
```

**Variables introduced:**
- `GROOM_ITERATE_RETRY` — initialized to `0` at the top of `dispatch_claude_pilot()`, guards max 1 retry.
- `PREV_LOG_ID` — preserves the first session's log ID for diagnostic tracing.

### Unit 2: dispatch-lib.sh — outcome classification update

**File:** `skills/bundled/_shared/dispatch-lib.sh`

In the existing outcome classification block (the `if echo "$RESULT" | grep -qF "PIPELINE FAILURE:"` cascade around lines 651-673), add a case for `GROOM_ITERATE_EXHAUSTED`:

```bash
elif echo "$RESULT" | grep -qF "GROOM_ITERATE_EXHAUSTED:"; then
    RESULT="${RESULT}

Outcome: GROOM_ITERATE_EXHAUSTED — iterate recovery exhausted, operator intervention needed."
```

This ensures the callback handler (self-dev-callback) can classify the outcome without log inspection.

### Unit 3: self-dev-callback handler — ITERATE exhaustion recognition

**File:** `skills/bundled/self-dev-callback/system_prompt.md`

Add a recognition pattern for `GROOM_ITERATE_EXHAUSTED:` in the callback classification section, alongside existing `PIPELINE FAILURE:` and `auto_skipped` patterns:

- On `GROOM_ITERATE_EXHAUSTED:` → classify as failure (not pipeline failure), escalate to operator. The ticket has a plan committed and first-pass findings but could not reach GROOMED autonomously.

### Unit 4: test-dispatch-lib.sh — ITERATE recovery test

**File:** `skills/bundled/_shared/test-dispatch-lib.sh`

Add test cases that verify:
1. `_iterate_recovery` is a no-op when skill is not dev-groom
2. `_iterate_recovery` is a no-op when body already contains `second-pass (GROOMED)`
3. `_iterate_recovery` detects ITERATE in session log and prepends `GROOM_ITERATE_EXHAUSTED` when retry limit is reached (structural test — mock the `gh` and log check)
4. `_iterate_recovery` does NOT fire when log shows ESCALATE

These are structural/logic tests, not end-to-end (same pattern as existing `test-dispatch-lib.sh` which tests closed-issue auto-skip logic in isolation).

### Unit 5: Solution doc update

**File:** `docs/solutions/2026-05-21-groom-post-flight-recovery-without-architect-verdict.md`

Update `resolution_type` from `investigation_needed` to `resolved`. Add a "Resolution" section referencing this fix. Mark the followup candidates as addressed:
- Candidate 1 (dev-groom pass-2 retry-or-escalate) → addressed by Unit 1
- Candidate 2 (post-flight recovery awareness in dispatch gate) → addressed by Unit 2
- Candidate 3 (pilot turn budget for grooming) → out of scope (not the root cause per reproduction evidence: 7-min session, well within budget)
- Candidate 4 (compound metric) → out of scope (observability, separate ticket)

## Scope

### In scope
- dispatch-lib.sh ITERATE recovery loop (max 1 re-launch)
- Outcome classification for exhausted retries
- self-dev-callback recognition of new outcome class
- Structural tests for the recovery logic
- Solution doc update

### Out of scope
- **claude-pilot GROOMED verdict tracking** (`CLAUDE_PILOT_REQUIRE_GROOMED` env var, analogous to `REQUIRE_PR`). Nice-to-have detection layer but not needed — dispatch-lib.sh recovery is the structural fix. Can add later if the re-launch approach doesn't converge.
- **`/mika-groom-ticket` command spec reinforcement.** The spec already handles ITERATE correctly (Phase 3 step 10 → Phase 4). The LLM doesn't follow it. Prompt reinforcement is unreliable; structural recovery is the fix. If future LLM versions follow through on ITERATE, the recovery loop becomes a no-op (detected GROOMED in body → early return).
- **Architect-side improvements** (better first-pass quality, fewer ITERATE verdicts). Separate dimension.
- **Multi-session architect context sharing** (passing session_id from first session to second). The second session runs the full `/mika-groom-ticket` flow which manages its own architect sessions. Cross-session architect context is a nice-to-have optimization.
- **Cross-repo changes** (`/mika-groom-ticket` spec in mika-platform, claude-pilot-py changes). This fix is mika-repo only.

## Risks

1. **Second session re-runs pass-1 on the same plan.** The re-launched session follows `/mika-groom-ticket` from Phase 1, re-reads the issue, finds the existing plan via idempotency. It runs pass-1 again, which costs one architect API call. If the first pass already improved the plan (or if the plan was always close to READY), the second pass-1 may return READY directly — cheaper than the alternative of injecting fragile iteration context. Acceptable cost: one extra architect call per ITERATE recovery.

2. **Session log parsing fragility.** `_iterate_recovery` greps the session log for `Disposition: ITERATE`. If mika-arch changes its output format, the grep fails silently and recovery doesn't fire (fail-open). This is the correct failure mode — false negatives (no retry) are preferable to false positives (retrying ESCALATE or READY).

3. **gh issue view race.** Between the first session updating the body and `_iterate_recovery` checking it, GitHub's API may be stale. The check is `body contains GROOMED` — a false negative means we retry unnecessarily (the second session finds GROOMED already written and exits quickly). Acceptable.

4. **Two sessions' cost.** Worst case: 2× the grooming cost ($1-4 total for a non-trivial ticket). Compared to the current cost of operator manual intervention on every ITERATE ticket, this is a clear win. The `max 1 retry` bound prevents runaway spend.

5. **Body callout collision between sessions (F3 resolution).** The first `_run_claude_pilot` call includes post-flight `_verify_and_write_body_callout` (dispatch-lib.sh lines 162-311) which may prepend a recovery placeholder callout before `_iterate_recovery` fires. If the second session then writes the canonical callout via Phase 5 step 19 ("prepend or merge into existing callouts at the top" — mika-platform `.claude/commands/mika-groom-ticket.md` line 220), the body would have two callout blocks. **Mitigation:** `_iterate_recovery` strips the recovery placeholder from the body before re-launching (see Unit 1 body-callout cleanup step). The sed pattern targets the three specific recovery callout lines (`> - **Branch:**`, `> - **Plan:**`, `> - **Grooming history:** body callout recovered by post-flight`). The `check_grooming_markers` gate (executor.rs line 800) does substring matching, so even if cleanup fails partially, the canonical callout from the second session would pass the gate. Defense in depth: cleanup is best-effort (`|| true`), not a hard gate.

## Acceptance criteria tie-back

- **AC1 (ITERATE → GROOMED autonomously):** Unit 1 re-launches the pilot, which runs the full `/mika-groom-ticket` flow including pass-1 → ITERATE handling → pass-2 → GROOMED.
- **AC1 (ESCALATE callout):** `_iterate_recovery` explicitly skips when the log shows ESCALATE — the first session's result is delivered as-is, and post-flight recovery writes the partial callout.
- **AC1 (bounded retry-exhaustion callout):** Unit 2 adds `GROOM_ITERATE_EXHAUSTED` outcome classification.
- **AC2 (regression test):** Unit 4 adds structural tests.
- **AC3 (solution doc):** Unit 5 updates the existing doc.

## Implementation sequence

1. Unit 1 (dispatch-lib.sh `_iterate_recovery`) — the primary fix
2. Unit 2 (outcome classification) — extends existing classification cascade
3. Unit 3 (self-dev-callback) — one-line pattern addition
4. Unit 4 (tests) — structural tests exercising the new logic
5. Unit 5 (solution doc) — doc update
