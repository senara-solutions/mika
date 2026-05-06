---
ticket: mika#988
type: fix
title: Auto-skip closed-issue autodispatch instead of pausing for confirmation
date: 2026-05-06
seq: 002
---

# Plan: auto-skip closed-issue autodispatch (mika#988)

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

## Verified state

- Source of the rejection: `mika/skills/bundled/_shared/dispatch-lib.sh:185-194` (lines may shift; the symbol is `if [ "$ISSUE_STATE" = "CLOSED" ]`).
- Exit-trap behaviour: `_dispatch_lib_exit_trap` in same file, lines ~21-105. On non-zero exit, it constructs `HANDLER CRASH (exit code N)` from stderr tail and delivers via `mika ask --task-id X --task-complete`.
- Structured-error precedent: `_validate_inputs` (lines ~125-160) emits `DISPATCH_VALIDATION_ERROR: {"error":"...","field":"..."}` to stderr before exit 1, also wrapped as HANDLER CRASH but with parseable JSON tail. mika#955 introduced this pattern. We reuse it.
- Symptom session for evidence: mika-dev session `callback-476caa1d-ef6d-4bac-a60c-a3c78f9a342d` (the failure callback turn).
- Drift session: mika-dev session `40a52d43-f186-4175-9c86-b998aafcf4bb` (07:19Z deliberation that I had to nudge to resume at 07:59Z).
- Acceptance fix shipped recently for a sibling pattern: `webhook_ready_label_dispatch` (mika#847), `webhook_no_unauthorized_dispatch` (mika#910). Both engine-side guards. They are precedent for moving classification out of LLM prompts and into engine-side rules.

## Scope

**In scope:**
- Change handler exit semantics for the closed-issue branch from "exit 1 / HANDLER CRASH" to "exit 0 / structured AUTO_SKIP_CLOSED result" (Phase 1).
- Make the auto-skip result legible to mika-dev's callback turn so the LLM does not interpret it as a failure that needs operator escalation (Phase 2).
- Add a unit/integration test that exercises the closed-issue path end-to-end (Phase 3).
- Document the behaviour in `_shared/dispatch-lib.sh` and in the self-dev skill prompt where the dispatch contract is described (Phase 4).

**Out of scope (explicitly):**
- Generalising "post-callback auto-advance" across all callback outcomes. That is the larger Pathology B — see Phase 5 follow-up note. This plan ships the closed-issue fix and files the broader pattern as a separate ticket so the fix bar stays narrow.
- Changing mika-engine's task scheduler to fire the next queued task after a callback. That is also Pathology B territory and a Rust change of meaningful surface; not justified by this single failure mode alone.

## Phase 1 — Reclassify closed-issue rejection from crash to structured skip

**Files touched:** `mika/skills/bundled/_shared/dispatch-lib.sh` (single file).

**Change shape:**

Replace the closed-issue branch (currently around line 191-194):
```bash
if [ "$ISSUE_STATE" = "CLOSED" ]; then
    echo "Error: Issue #${ISSUE_NUM} in senara-solutions/${REPO} is closed. Reopen first." >&2
    exit 1
fi
```

With:
```bash
if [ "$ISSUE_STATE" = "CLOSED" ]; then
    # Auto-skip: PR merge auto-closed the issue between webhook enqueue and handler fire.
    # This is an expected race, not a handler bug — deliver a structured skip result so
    # mika-dev's callback turn can advance the queue without operator confirmation.
    # See mika#988 for the failure mode this guards against.
    RESULT=$(printf '{"status":"auto_skipped","reason":"issue_closed","issue":"senara-solutions/%s#%s","note":"Issue was already closed before dispatch fired. Presumed shipped via earlier PR merge."}' "$REPO" "$ISSUE_NUM")
    if [ -n "$AGENT" ]; then
        mika ask --task-id "$TASK_ID" --task-complete --agent "$AGENT" -- "$RESULT"
    else
        mika ask --task-id "$TASK_ID" --task-complete -- "$RESULT"
    fi
    CALLBACK_SENT=1
    exit 0
fi
```

Why exit 0 (and why not let the EXIT trap handle delivery):
- Exit 0 with explicit `CALLBACK_SENT=1` mirrors the success path's contract. The EXIT trap's first line guards on `CALLBACK_SENT` and skips redelivery, so we won't double-fire.
- Inlining the `mika ask --task-complete` call keeps the auto-skip path symmetric with how the success path delivers — same call, same flags, just a structured JSON payload instead of free-form text.
- A pure exit 0 with no callback would leave the task wedged in_progress until a watchdog reaped it. Worse than the current bug.

**Result-payload shape:** structured JSON, single line. mika-dev's callback turn already calls `jq` on results that look like JSON (per the existing `DISPATCH_VALIDATION_ERROR` pattern from mika#955). Keying on `status: "auto_skipped"` lets future tooling (the audit dashboard, the autonomous-loop replay) filter these out from real failures.

## Phase 2 — Make the auto-skip legible at the LLM layer

**Why this phase exists:** Phase 1 alone is insufficient because mika-dev's session prompt is what decides whether to advance the queue autonomously after a callback. Even with a clean structured result, the prompt may still cause the LLM to ask "want me to proceed?" — that's Pathology B.

**Constraint:** per `feedback_prompt_enforcement_fragile.md`, prompts are not the right place for hard structural rules. So Phase 2's prompt change is **narrow and recognition-only**: tell the LLM to recognize `status: "auto_skipped"` as a non-event that does not warrant a status message in the session, not to enforce queue-advancement behaviour.

**Files touched:**
- `mika/skills/bundled/self-dev/system_prompt.md` (or wherever the post-callback turn rules live — verify at implementation; the planned location may have moved since this plan was written).
- Possibly `mika/skills/bundled/self-dev-webhook-ci/system_prompt.md` and sibling webhook callback handlers if they share the post-callback behaviour.

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
- Cross-link from `mika/docs/solutions/best-practices/` if a relevant compound entry exists for "race between webhook enqueue and PR merge"; otherwise file a small compound entry as part of this PR (single page, ~30 lines).

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

- **Risk: the `gh` shim approach for Test 1 may not match how the existing test harness in `mika-skills/` is structured.** Mitigation: verify at Phase 3 implementation. If the harness uses a different stubbing pattern, follow that pattern. The test must be runnable in CI, not just locally.
- **Unknown: the exact location of mika-dev's post-callback prompt rules.** It may be in `self-dev/system_prompt.md`, in `mika-dev`'s own agent system prompt (lives in agents-teams or wherever the dev agent is configured), or split across both. Resolve at Phase 2 implementation by grepping for the existing post-callback guidance and adding alongside it.
- **Unknown: whether the prompt change in Phase 2 actually changes behaviour given known prompt-adherence drift on similar models.** Mitigation: Phase 1's handler change is sufficient on its own to make the result *legible* (no longer HANDLER CRASH). If Phase 2's prompt rule does not stick on the current mika-dev model (kimi-k2.5 per `project_mika_dev_model_switch.md`), the LLM may still occasionally narrate the auto-skip — but this is degraded behaviour, not the original failure mode (the queue still advances on its own normal cadence). Phase 5 is the structural fix if this surfaces.

## Compound learning to write at PR-close

A short compound at `mika/docs/solutions/best-practices/` covering: "Handler exit semantics — distinguishing expected races from real crashes." Pattern: when a handler can fail for a foreseeable racy reason (issue closed mid-flight, target branch deleted, etc.), the right shape is exit 0 + structured JSON skip result, not exit 1 + HANDLER CRASH. Sibling: mika#955's `DISPATCH_VALIDATION_ERROR` pattern.
