---
ticket: mika#988
type: fix
title: Auto-skip closed-issue autodispatch instead of pausing for confirmation
date: 2026-05-06
seq: 002
---

# Plan: auto-skip closed-issue autodispatch (mika#988)

## Verified state (post-architect-pass-1)

- **F1 (Phase 0 Pin) addressed** — explicit line-pinned source locations now appear in the new `## Phase 0 — Pin` section: closed-issue check at lines 191-194, `_deliver_callback()` helper at lines 374-390, EXIT trap re-delivery guard at lines 19-23, init/trap-install at lines 419-425, all anchored to `dispatch-lib.sh` HEAD `48e52c83`.
- **F2 (duplicate-callback risk) addressed structurally** — Phase 1 now reuses the existing `_deliver_callback()` helper instead of inlining a second callback-send site. There is no second site; the EXIT trap's `CALLBACK_SENT == 1` guard at line 22 is what makes single-delivery safe; the helper sets `CALLBACK_SENT=1` at line 382. Pre-implementation verification step added so a future drift in line numbers surfaces before the edit lands.
- **F3 (compound-doc principle citation) addressed** — Phase 4 now names the principle ("Handler exit semantics for foreseeable races vs real crashes") and the file path (`mika/docs/solutions/best-practices/handler-exit-semantics-foreseeable-races-2026-05-06.md`). Sibling pattern citations included: review-guide § Orthogonality, mika#955's contrapositive `DISPATCH_VALIDATION_ERROR` shape.
- **F4 (silent-skip-for-intentional-closes) addressed** — explicit position in `## Scope` section: PR-closes and human-closes are treated identically as "presumed handled," with four-point reasoning. Audit trail is preserved via the structured-JSON callback to the `messages` table; close-reason enrichment is deferred to Phase 5 if operational practice surfaces a need.

## Why

On 2026-05-06 the autonomous loop stalled for ~7 hours. Concrete sequence:

1. mika#985 PR #986 merged at 2026-05-05 23:44Z, auto-closing the issue.
2. mika-dev's queued autodispatch task for mika#985 (enqueued before the merge) fired at 00:34:35Z.
3. The dispatch handler reached `_shared/dispatch-lib.sh:192` (`if [ "$ISSUE_STATE" = "CLOSED" ]; then echo "Error: ... closed. Reopen first." >&2; exit 1`).
4. Exit 1 was caught by the handler's EXIT trap which wraps stderr into `RESULT="HANDLER CRASH (exit code 1). Script failed before building result. Stderr (last 10KB): ..."` and delivers via mika callback.
5. mika-dev's callback turn read the HANDLER CRASH result, marked the task completed, then **posted a confirmation question to its session** ("Task marked completed. Next in queue is mika#984 dispatch. Want me to proceed?") and idled.
6. No further automatic progress until manual nudge at 07:59Z.

Two pathological behaviours collided:
- **Pathology A — Handler classifies "expected race" as "crash."** Closed-issue dispatch is a foreseeable race (PR merge auto-closes the issue between webhook enqueue and handler fire), but the handler delivers the same `HANDLER CRASH` envelope as a real bash failure. The callback consumer can't distinguish "harmless skip" from "real bug."
- **Pathology B — mika-dev's post-callback turn is conversational, not autonomous.** After any callback completes, mika-dev tends to summarize state and ask the operator what to do next, rather than firing the next pending task autonomously. This was already known; mika#988 raises its visibility but is not solely caused by it.

The fix bar is: when an autodispatch fires for a closed issue, the next queued task fires without operator confirmation, and the audit log records the auto-skip with the closed-issue ID.

## Phase 0 — Pin (verified state, source-anchored)

All line numbers verified against `mika/skills/bundled/_shared/dispatch-lib.sh` at HEAD `48e52c83` (main).

**The closed-issue rejection — what we are replacing (lines 191-194):**
```bash
if [ "$ISSUE_STATE" = "CLOSED" ]; then
    echo "Error: Issue #${ISSUE_NUM} in senara-solutions/${REPO} is closed. Reopen first." >&2
    exit 1
fi
```

**The `_deliver_callback()` helper — what we will reuse (lines 374-390):**
```bash
_deliver_callback() {
    set +e
    if [ -n "$AGENT" ]; then
        mika ask --task-id "$TASK_ID" --task-complete --agent "$AGENT" -- "$RESULT"
    else
        mika ask --task-id "$TASK_ID" --task-complete -- "$RESULT"
    fi
    CALLBACK_EXIT=$?
    CALLBACK_SENT=1
    rm -f "$TRACE_FILE"
    set -e

    if [ "$CALLBACK_EXIT" -ne 0 ]; then
        echo "ERROR: callback delivery failed (exit $CALLBACK_EXIT) for task $TASK_ID" >&2
    fi
}
```

This helper is the canonical callback-delivery site. It owns the `set +e / mika ask / CALLBACK_SENT=1 / set -e` ordering. The success path uses it. The auto-skip branch in Phase 1 will use it too — single delivery site, single point of ordering, no inline duplication. **This directly resolves architect F2** (the duplicate-callback risk), because there is no second callback-send site introduced by the auto-skip path.

**The EXIT trap's re-delivery guard (lines 19-23):**
```bash
_dispatch_lib_exit_trap() {
    _EXIT_CODE=$?
    # Guard: skip if already delivered or no task ID
    [ "$CALLBACK_SENT" -eq 1 ] && { [ -n "$STDOUT_FILE" ] && rm -f "$STDOUT_FILE"; [ -n "$STDERR_FILE" ] && rm -f "$STDERR_FILE"; rm -f "$TRACE_FILE"; return; }
```

The `CALLBACK_SENT == 1` guard at line 22 is what makes single-delivery safe. Once `_deliver_callback` sets `CALLBACK_SENT=1` (line 382), the trap's first conditional bails out — even if the script later exits non-zero from any cause.

**Initialization and trap install (lines 419-425, inside `dispatch_claude_pilot()`):**
```bash
# Line 420
CALLBACK_SENT=0

# Line 425
trap '_dispatch_lib_exit_trap' EXIT
```

`CALLBACK_SENT=0` is set BEFORE `_parse_input_json` (line 422), and the trap is installed at line 425. The closed-issue check at line 191 is in a region of the file numerically earlier than 425 but executes AFTER `dispatch_claude_pilot()` runs lines 419-425 — i.e., when the function body reaches the closed-issue branch in execution order, `CALLBACK_SENT=0` is initialized and the trap is armed. This is verified by tracing the function body's control flow: `dispatch_claude_pilot()` defines its body inline, lines 399-onward, with the trap install at 425 happening before the input-parsing branch that leads to the issue-state check.

**Symptom evidence:**
- mika-dev session `callback-476caa1d-ef6d-4bac-a60c-a3c78f9a342d` — the failure callback turn (00:36:04Z, 2026-05-06).
- mika-dev session `40a52d43-f186-4175-9c86-b998aafcf4bb` — the drift turn (07:19:08Z, 7 hours later, manual nudge at 07:59Z restored progress).

**Sibling precedents:**
- `webhook_ready_label_dispatch` (mika#847) — engine-side guard for label-triggered dispatch.
- `webhook_no_unauthorized_dispatch` (mika#910) — engine-side guard for cross-tenant dispatch.
- `DISPATCH_VALIDATION_ERROR` pattern (mika#955) — structured JSON in stderr, also wrapped as HANDLER CRASH but with parseable tail. **Different shape from this fix:** mika#955 is for *handler bugs* (missing required field) where exit 1 is the right verdict; this fix is for *expected races* where exit 0 is the right verdict.

**Out-of-pin (deliberately not addressed by this plan):**
- Pathology B (mika-dev's post-callback conversational turn) lives in mika-dev's system prompt + the post-callback turn's prompt assembly path. Not pinned here because Phase 5 files it separately.

## Scope

**In scope:**
- Change handler exit semantics for the closed-issue branch from "exit 1 / HANDLER CRASH" to "exit 0 / structured AUTO_SKIP_CLOSED result" (Phase 1).
- Make the auto-skip result legible to mika-dev's callback turn so the LLM does not interpret it as a failure that needs operator escalation (Phase 2).
- Add a unit/integration test that exercises the closed-issue path end-to-end (Phase 3).
- Document the behaviour in `_shared/dispatch-lib.sh` and in the self-dev skill prompt where the dispatch contract is described (Phase 4).

**Out of scope (explicitly):**
- Generalising "post-callback auto-advance" across all callback outcomes. That is the larger Pathology B — see Phase 5 follow-up note. This plan ships the closed-issue fix and files the broader pattern as a separate ticket so the fix bar stays narrow.
- Changing mika-engine's task scheduler to fire the next queued task after a callback. That is also Pathology B territory and a Rust change of meaningful surface; not justified by this single failure mode alone.

**Position on intentional human-closes (architect F4):**

The auto-skip branch fires whenever `ISSUE_STATE = "CLOSED"` regardless of whether the close came from a PR merge or from a human pressing "Close issue." Both close-reasons are treated identically as "presumed handled." Reasoning:

1. **Audit trail is preserved either way.** The structured JSON callback (`status: auto_skipped, reason: issue_closed, issue: senara-solutions/REPO#N`) is delivered via `mika ask --task-complete`, which writes a row to the `messages` table with `task_id` and `created_at`. The dashboard's task-history view can filter on `status: auto_skipped` to show every skipped dispatch with its issue ID. Operators see the skip; they just don't see it as a HANDLER CRASH that required intervention.
2. **Distinguishing close-reason at handler-time is over-engineering.** Determining whether a close was "PR-driven" vs "human-driven" requires querying the issue's `closed_by` and the events log via additional `gh` calls. Each additional call is a new failure mode (rate limit, transient 5xx, auth scope mismatch). The marginal value — slightly more legible audit trail — does not justify this surface.
3. **The HANDLER CRASH framing wasn't actually surfacing intent.** The 2026-05-06 stall happened on a PR-driven close (mika#985 closed by PR #986 merge). The HANDLER CRASH envelope did not help; it stalled the loop. There is no evidence that "loud HANDLER CRASH" is currently load-bearing for human-close detection.
4. **Symmetric with how mika-dev's autonomous loop already treats issue state.** Mika-dev's webhook handlers already drop closed-issue webhooks at the gateway level (no task is created if the issue is already closed at webhook time). The auto-skip branch handles only the race window between webhook arrival and handler fire — a small interval where the close happened mid-flight. Treating both close-reasons identically in this narrow window is consistent with how the rest of the system treats them.

If the audit trail proves insufficient in practice (operators want to know "was this a real intent-to-not-ship close vs an automatic close?"), Phase 5's follow-up ticket is the right surface for adding `closed_reason` enrichment, not this PR.

## Phase 1 — Reclassify closed-issue rejection from crash to structured skip

**Files touched:** `mika/skills/bundled/_shared/dispatch-lib.sh` (single file).

**Change shape:**

Replace the closed-issue branch (currently lines 191-194):
```bash
if [ "$ISSUE_STATE" = "CLOSED" ]; then
    echo "Error: Issue #${ISSUE_NUM} in senara-solutions/${REPO} is closed. Reopen first." >&2
    exit 1
fi
```

With:
```bash
if [ "$ISSUE_STATE" = "CLOSED" ]; then
    # Auto-skip: PR merge (or any other close) raced ahead of the webhook-triggered
    # dispatch enqueue. This is an expected race, not a handler bug — deliver a
    # structured skip result via the canonical _deliver_callback() helper so
    # mika-dev's callback turn can recognise it as a no-op and the audit dashboard
    # can filter on status: "auto_skipped". See mika#988 for the failure mode.
    # Position on human-closes vs PR-closes: treated identically — see plan §Scope.
    RESULT=$(printf '{"status":"auto_skipped","reason":"issue_closed","issue":"senara-solutions/%s#%s","note":"Issue was already closed before dispatch fired. Presumed handled."}' "$REPO" "$ISSUE_NUM")
    _deliver_callback
    exit 0
fi
```

Why this exact shape:

- **Reuse `_deliver_callback`, do not inline.** The success path delivers via `_deliver_callback()` (lines 374-390 in the pin). Calling the same helper gives us identical `set +e / mika ask / CALLBACK_SENT=1 / set -e` ordering with zero duplication. **F2 is structurally resolved** — no second callback-send site is introduced.
- **Exit 0 after the helper.** The helper sets `CALLBACK_SENT=1` (line 382). The EXIT trap's guard at line 22 returns early when `CALLBACK_SENT=1`, so even though `exit 0` triggers the trap, the trap is a no-op and no duplicate callback fires. Exit 0 (not exit 1, not exit 99, not "fall through") is the right verdict because the handler did exactly what was asked: dispatch was attempted and the outcome was a clean structured skip.
- **No fall-through to the rest of `dispatch_claude_pilot()`.** The `exit 0` ensures the function does not proceed to worktree setup, claude-pilot invocation, or the success-path callback that follows much later in the function body. The auto-skip branch is terminal.
- **Result-payload shape:** structured JSON, single line. mika-dev's callback turn already handles JSON-looking results (per the existing `DISPATCH_VALIDATION_ERROR` pattern from mika#955). Keying on `status: "auto_skipped"` lets future tooling filter these out from real failures.

**Pre-implementation verification step:**

Before editing the file, run a smoke check to confirm the line numbers match the pin (the file may have shifted since this plan was written):

```bash
grep -n "if \[ \"\$ISSUE_STATE\" = \"CLOSED\" \]" mika/skills/bundled/_shared/dispatch-lib.sh
grep -n "^_deliver_callback() {" mika/skills/bundled/_shared/dispatch-lib.sh
```

Expected output:
- Closed-issue check at or near line 191.
- `_deliver_callback()` definition at or near line 374.

If the line numbers have drifted by more than ~10 lines in either direction, treat that as a signal that other dispatch-related changes have landed since this plan was written and re-read the function body to confirm the auto-skip branch will compose correctly with the surrounding control flow before applying the edit.

## Phase 2 — Make the auto-skip legible at the LLM layer

**Why this phase exists:** Phase 1 alone is insufficient because mika-dev's session prompt is what decides whether to advance the queue autonomously after a callback. Even with a clean structured result, the prompt may still cause the LLM to ask "want me to proceed?" — that's Pathology B.

**Constraint:** per `feedback_prompt_enforcement_fragile.md`, prompts are not the right place for hard structural rules. So Phase 2's prompt change is **narrow and recognition-only**: tell the LLM to recognize `status: "auto_skipped"` as a non-event that does not warrant a status message in the session, not to enforce queue-advancement behaviour.

**Files touched:**
- `mika/skills/bundled/self-dev/system_prompt.md` (or wherever the post-callback turn rules live — verify at implementation; the planned location may have moved since this plan was written).
- Possibly `mika/skills/bundled/self-dev-webhook-ci/system_prompt.md` and sibling webhook callback handlers if they share the post-callback behaviour.

**Pre-flight gate (architect U1, second-pass tightening):**

Before writing the Phase 2 prompt change, the implementer MUST run the following greps and identify the exact insertion site. This is a gate, not an in-progress check — Phase 2 does not begin until the insertion site is named:

```bash
grep -rn "callback\|task.complete\|dispatch.*result\|HANDLER CRASH" \
    mika/skills/bundled/self-dev/system_prompt.md \
    mika/skills/bundled/self-dev-webhook-ci/system_prompt.md \
    mika/skills/bundled/self-dev-webhook-qa/system_prompt.md
grep -rn "post-callback\|callback turn\|auto.skip" \
    mika/skills/bundled/
```

The implementer's first deliverable for Phase 2 is a one-line answer to the question "where does the post-callback turn rule live?" If the answer is "no existing rule, this would be a new stanza," that is acceptable but must be a **deliberate** choice — confirmed by reading the surrounding prompt structure, not by defaulting to "well, there's no obvious place, so I'll make one." The intent is to prevent implementation-discovery-by-default; the rule lives next to whatever existing post-callback guidance the prompts already have, or in a new clearly-labelled stanza if none exists.

**Change shape (prompt addition):**

Add a single rule to the post-callback handling section:

> When a dispatch callback delivers a result whose first line parses as JSON with `"status": "auto_skipped"`, treat the task as a no-op completion. Do not post a status message to the session. Do not ask the operator whether to proceed. Continue normal idle/listen behaviour — the engine schedules the next pending task on its own normal cadence.

**What we are explicitly NOT doing in this phase:**
- We are NOT adding "after every callback, autonomously fire the next pending task" guidance. That is Pathology B and a separate ticket (see Phase 5 follow-up).
- We are NOT giving the LLM any role in deciding whether to skip. The handler already decided in Phase 1; the LLM only learns to be quiet about the decision.

## Phase 3 — Test coverage

**Test 1 — handler unit test for closed-issue path.**

Location: `mika/skills/bundled/_shared/test-dispatch-lib.sh` if it exists; otherwise a new file alongside.

Approach: feed a synthetic input JSON to `dispatch_claude_pilot` with a stubbed `gh issue view` (e.g., a PATH-prepended `gh` shim) that returns `{"state":"CLOSED",...}`. Assert:
- Exit code is 0.
- `mika ask --task-complete` was called with a result containing `"status":"auto_skipped"` and `"reason":"issue_closed"`.
- The result is exactly one line of JSON (no trailing log lines, no banner contamination).
- No `HANDLER CRASH` substring appears in the delivered result.

If the existing skills test harness does not support stubbing `gh`, the test can be written as a small bash function that sets `gh()` as a function before sourcing dispatch-lib. mika-skills already uses this pattern in `qa-review`'s tests (verify at implementation).

**Test 2 — integration smoke for the prompt change.**

Approach: a transcript-replay test where we feed mika-dev a synthetic callback event with `status: "auto_skipped"` and assert that no `assistant`-role message is emitted to the session. If the existing `mika test --replay` harness supports this, use it; otherwise this can be a manual smoke step on the first deploy.

**No test for "queue advances autonomously"** — that is Pathology B and out of scope for this ticket. Phase 3's tests cover the surfaces this ticket actually changes.

## Phase 4 — Documentation

- Add a `# Auto-skip rationale` comment block to `dispatch-lib.sh` near the closed-issue branch, citing mika#988 and the symptom session IDs. Future readers who try to "tighten" this back to exit 1 should see why the looser shape is correct.
- Update `mika/CLAUDE.md`'s autonomous-loop section (if it documents callback contracts) with a one-line note: "Closed-issue autodispatch returns `status: auto_skipped`; not a failure."
- File a new compound entry at `mika/docs/solutions/best-practices/handler-exit-semantics-foreseeable-races-2026-05-06.md`. Title: **"Handler exit semantics for foreseeable races vs real crashes."** Single principle stated up front:

  > When a handler can fail for a *foreseeable racy reason* (the target became invalid between enqueue and fire — issue closed, branch deleted, repo archived, etc.), the right shape is `exit 0` + structured-JSON skip result delivered via the canonical callback helper. Reserve `exit 1` + HANDLER CRASH envelope for actual handler bugs (logic errors, missing required fields, unexpected provider responses) where the consumer cannot recover cleanly. The exit code is the load-bearing distinction; downstream consumers (mika-dev's callback turn, the audit dashboard, watchdogs) make decisions based on it.

  Citations from existing best practices the principle aligns with:
  - `mika/docs/architecture/review-guide.md` § Orthogonality — keeping recovery-class outcomes in the response shape rather than letting them bleed into the exit-code channel.
  - `mika/docs/solutions/cross-repo-patterns/security-hardening-playbook.md` — analogous shape for fail-closed-vs-fail-open guards (reject-with-structured-error vs. crash).
  - mika#955's `DISPATCH_VALIDATION_ERROR` pattern is the *contrapositive* of this principle: real handler bugs (missing required field) get exit 1 + structured-JSON-in-stderr. mika#988 is for *foreseeable races* and gets exit 0 + structured-JSON-in-callback. The two patterns share "structured JSON" but differ on exit code by design — that is the load-bearing distinction.

  The compound entry is ~30-60 lines (frontmatter, principle statement, why-this-shape, sibling patterns, anti-patterns to avoid). Keeping it scoped to one principle so future readers can cite it cleanly without absorbing tangentially related material.

## Phase 5 — Follow-up filed, not shipped

After this PR merges, file a separate ticket on mika for **Pathology B** (mika-dev's post-callback turn is conversational rather than autonomously advancing the queue). The follow-up's surface is meaningfully larger:
- Audit how `self-dev`, `self-dev-webhook-ci`, `self-dev-webhook-qa`, `qa-review-build-callback` handle their callback turns.
- Decide whether the engine should auto-fire the next pending task on callback completion (Rust change in mika-engine's scheduler) or whether prompt-level changes are sufficient.
- Add tests at the team-engine level for "queue advancement after each terminal callback."

That follow-up is filed as a separate ticket with this plan's failure mode cited as one of three documented incidents (closed-issue, prior milestone#13 wedge cancellation per the 2026-05-06 handsoff, and the heartbeat-doesn't-resume-milestones gap from `project_heartbeat_milestone_phantom.md`).

## Acceptance criteria (from the ticket)

- [x] When an autodispatch fires for a closed issue, the next queued ticket fires without operator confirmation. **Phase 1 + Phase 2.**
- [x] A trace/log entry records the auto-skip with the closed-issue ID, so the operator can audit retroactively. **Phase 1 — the structured JSON result is the audit record, persisted via the normal `mika ask --task-complete` path that writes to the messages table.**
- [x] The fix does not silently suppress dispatch failures for *open* issues — only the closed-issue case. **Phase 1 — the new branch is gated on `ISSUE_STATE = "CLOSED"`, sibling failure modes (issue not found, missing fields, etc.) are unchanged.**
- [x] Test coverage: a unit/integration test that simulates the closed-issue dispatch path. **Phase 3.**

## Sequencing

Phase 1 → Phase 2 → Phase 3 → Phase 4 in one PR. Phase 5 is a separate ticket filed at PR-merge time.

## Risks and known unknowns

- **Risk: the `gh` shim approach for Test 1 may not match how the existing test harness in `mika-skills/` is structured.** Mitigation: at Phase 3 implementation, grep for existing dispatch-lib tests first (`grep -rn "dispatch-lib\|dispatch_claude_pilot" mika/skills/`). Whatever pattern those use is the pattern to follow. If no harness exists, the test belongs in `mika/skills/bundled/_shared/test-dispatch-lib.sh` as a new file, runnable from CI via the existing `make test` or skills test runner.
- **Unknown: the exact location of mika-dev's post-callback prompt rules.** It may be in `self-dev/system_prompt.md`, in `mika-dev`'s own agent system prompt (lives in agent identity files via `MIKA_DEV_MODE` provisioning per `mika/CLAUDE.md`), or split across both. Resolve at Phase 2 implementation by grepping for the existing post-callback guidance (`grep -rn "callback\|task.complete\|dispatch.*result" mika/skills/bundled/self-dev/system_prompt.md`) and adding alongside it. If the rule lives in agent identity rather than skill prompt, the appropriate change site is `crates/mika-agent/src/dev_mode/` (where well-known agents are provisioned) — verify before editing.
- **Unknown: whether the prompt change in Phase 2 actually changes behaviour given known prompt-adherence drift on similar models.** Mitigation: Phase 1's handler change is sufficient on its own to make the result *legible* (no longer HANDLER CRASH). If Phase 2's prompt rule does not stick on the current mika-dev model (kimi-k2.5 per `project_mika_dev_model_switch.md`), the LLM may still occasionally narrate the auto-skip — but this is degraded behaviour, not the original failure mode. The queue still advances on its own normal cadence (driven by webhook events and milestone-parent advancement, not by mika-dev's narration). Phase 5 is the structural fix if recurring narration surfaces in audits.
- **Risk that F2 reintroduces.** Phase 0 pin's reuse of `_deliver_callback()` is the structural answer. The implementation must call the helper as-is; any temptation to "inline a quick callback for the auto-skip case" must be rejected — that path is exactly what F2 warned against. The pre-implementation verification step in Phase 1 ensures the helper exists at expected line numbers; if it has been refactored away, the plan needs revisiting before applying the edit.
