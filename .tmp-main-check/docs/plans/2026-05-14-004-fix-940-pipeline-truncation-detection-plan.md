# Plan: Pipeline-Truncation Detection for claude-pilot /mika Sessions

- **Ticket:** mika issue#940
- **Type:** fix
- **Branch:** `fix/940/self-dev-claude-pilot-mika-pipeline`
- **Date:** 2026-05-14

## Phase 0 — File Pinning

Three change sites across two repos. Pinned at mika `8731102d` and claude-pilot-py `463d8573`.

### Pin 1: `mika/skills/bundled/_shared/dispatch-lib.sh` — lines 387-394 + 523-535

**Insertion point A** (post-flight PR check, after line 394 — new `PIPELINE FAILURE` block):
```bash
        # Issue #138: Discover actual PR URL from the branch
        if [ -n "$REPO" ] && [ -n "$BRANCH" ]; then
            PR_URL=$(gh pr list --repo "senara-solutions/$REPO" --head "$BRANCH" --json url --jq '.[0].url' 2>/dev/null || true)
            if [ -n "$PR_URL" ]; then
                RESULT="${RESULT}
PR: ${PR_URL}"
            fi
        fi
```
Insert the new `PIPELINE FAILURE` block immediately after line 394 (closing `fi` of the PR-discovery block).

**Insertion point B** (env var export, line 525 — `dev-pilot)` case branch):
```bash
    case "$SKILL" in
      dev-pilot)  ENTRY_COMMAND="/mika" ;;
      dev-groom)
```
Change line 525 to add `export CLAUDE_PILOT_REQUIRE_PR=1` after the `ENTRY_COMMAND` assignment.

### Pin 2: `claude-pilot-py/src/claude_pilot/agent.py` — lines 244-271

**Insertion point** (after `status == "success"` check at line 260, before `_emit_result`):
```python
                # Normal result handling
                result = ResultJson(
                    status=status,
                    subtype=subtype,
                    ...
                )
                _emit_result(result)

                if status == "success":
                    log_done(...)
```
The `pipeline_incomplete` check inserts between the `result = ResultJson(...)` construction and `_emit_result(result)` at lines 245-258. When `require_pr=True` and `tool_call_counter.pr_created is False`, override `status` and `subtype` before constructing `ResultJson`.

### Pin 3: `claude-pilot-py/src/claude_pilot/permissions.py` — lines 53-69

**Modification point** (`ToolCallCounter` class):
```python
class ToolCallCounter:
    """Tracks the number of tool calls observed during a session."""

    def __init__(self) -> None:
        self._count = 0

    def increment(self) -> None:
        self._count += 1

    @property
    def count(self) -> int:
        return self._count
```
Add `pr_created: bool = False` field and a `record_pr_creation(self, tool_name: str, tool_input: dict)` method that sets `self.pr_created = True` when `tool_name == "Bash"` and `"gh pr create"` is in `tool_input.get("command", "")`.

### Cross-ticket sequencing (mika#1032)

mika#1032 also modifies `dispatch-lib.sh` (dev-groom drift detection). These two tickets touch adjacent but non-overlapping sections of post-flight logic (lines 363-394). **Merge ordering:** mika#940 (this ticket) should merge after mika#1032 if both are in flight simultaneously, since #940 adds new code after the existing post-flight block while #1032 modifies the existing dev-groom plan-validation check. If #1032 is not yet in flight, #940 can proceed independently — the changes don't conflict at the line level. The implementer should check `gh pr list --repo senara-solutions/mika --search "1032"` before starting.

**Shift note (post-#1032 merge):** If #1032 merges first, the line numbers for insertion point A may shift by a few lines. The anchor is the `# Issue #138: Discover actual PR URL from the branch` comment — insert after its closing `fi`.

## Problem

claude-pilot `/mika` pipeline exits `[done] Success` before reaching `git push` + `gh pr create`. Work is stranded in the worktree as uncommitted or unpushed edits. Three verified instances on 2026-05-02 (mika#931, mika#938, mika#939). The model voluntarily EndTurns after implementation/compounding phases — no guardrail tripped.

## Decision: Path 2(a) + 3(a) — Structural Detection at Two Layers

The ticket proposes three fix surfaces. Per the ticket's own lean analysis, prompt-engineering (Path 1) is fragile against training-conditioned "I'm done" patterns. The structural approach is:

1. **dispatch-lib.sh post-flight PR-existence check** (Path 3(a) shape, leveraging the existing PR-discovery code at line 387-394) — classify `dev-pilot` success-without-PR as `PIPELINE FAILURE`.
2. **claude-pilot `subtype` enrichment** (Path 2(a) shape) — add a new `pipeline_incomplete` subtype to `ResultJson` when the session prompt was a `/mika` dispatch and the session log shows no `gh pr create` Bash call.

**Why two layers:**
- Layer 1 (dispatch-lib) is zero-cost: the `gh pr list` call already runs at line 388; we just need to branch on empty result for `dev-pilot` skill.
- Layer 2 (claude-pilot) catches the failure at the source with a richer signal, enabling future re-prompt attempts (like the existing early-exit guard for `min_tool_calls`).

**What we're NOT doing:**
- Auto-push or auto-PR (per `feedback_orchestrate_not_implement.md` — operator authorization required).
- Third-pass re-prompt on pipeline-incomplete (keep it to detection + classification first; re-prompt can be added as a follow-up once we see the detection signal in production).

## Implementation Units

### Unit 1: dispatch-lib.sh — Post-Flight PR-Existence Failure Classification

**File:** `mika/skills/bundled/_shared/dispatch-lib.sh`

**Change:** After the existing PR-discovery block (lines 387-394), add a conditional `PIPELINE FAILURE` classification when:
- `$SKILL` is `dev-pilot` (grooming sessions don't create PRs)
- `$STATUS` is `success` (don't double-classify already-failed sessions)
- `$PR_URL` is empty (no PR found on the branch)
- `$POST_RUN_HEAD` ≠ `$PRE_RUN_HEAD` (there ARE commits — the session did work but didn't push/PR)

```bash
# Post-flight PR existence check (mika#940): detect dev-pilot success
# with commits but no PR — pipeline truncated before git push / gh pr create.
if [ "$SKILL" = "dev-pilot" ] && [ -z "$PR_URL" ] && [ -n "$POST_RUN_HEAD" ] && [ "$PRE_RUN_HEAD" != "$POST_RUN_HEAD" ]; then
    RESULT="PIPELINE FAILURE: claude-pilot produced commits (${PRE_RUN_HEAD}..${POST_RUN_HEAD}) but no PR was opened on branch '${BRANCH}'. Pipeline truncated before git push + gh pr create.

${RESULT}"
fi
```

**Why `dev-pilot` only:** `dev-groom` exits after committing a plan — no PR expected. The existing plan-validation check (lines 377-385) covers `dev-groom` drift. Only `dev-pilot` has the PR-creation contract.

**Why require HEAD change:** If both HEAD unchanged AND no PR, the zero-commit check (lines 364-371) already fires. We don't want both failure messages for the same incident.

**Env var for claude-pilot:** In the same `dev-pilot)` case branch (line 525), add:
```bash
dev-pilot)
    ENTRY_COMMAND="/mika"
    export CLAUDE_PILOT_REQUIRE_PR=1
    ;;
```
This enables Unit 2's `pipeline_incomplete` detection in claude-pilot for `dev-pilot` sessions only.

### Unit 2: claude-pilot — `pipeline_incomplete` Subtype

**Files:**
- `claude-pilot-py/src/claude_pilot/types.py` — Document the new subtype in `ResultJson` docstring.
- `claude-pilot-py/src/claude_pilot/agent.py` — Add post-session pipeline-completion check.
- `claude-pilot-py/src/claude_pilot/permissions.py` — Extend `ToolCallCounter` with PR-creation tracking.

**Session-type detection:** Use env var `CLAUDE_PILOT_REQUIRE_PR` (set by dispatch-lib per-skill, same pattern as `CLAUDE_PILOT_MIN_TOOL_CALLS`). This follows the established convention for per-skill session contracts. dispatch-lib sets `export CLAUDE_PILOT_REQUIRE_PR=1` in the `dev-pilot)` case branch. cli.py reads it and passes `require_pr: bool` to `run_agent()`.

**Why env var, not prompt-prefix:** The prompt prefix (`/mika`) is fragile — it couples claude-pilot's exit logic to the specific slash-command name, and changes if the entry command is renamed. The env var is set by dispatch-lib (the same component that knows which skill is running), consistent with `CLAUDE_PILOT_MIN_TOOL_CALLS`, and explicit about the contract.

**Tool-call tracking:** Extend `ToolCallCounter` in `permissions.py` with a `pr_created: bool` field (default `False`). On each `allow` callback for `Bash` tool, check if `tool_input.get("command", "")` contains `gh pr create`. If so, set `pr_created = True`. This is a minimal extension to an existing class already wired into the permissions callback — no new class needed.

**Exit logic (agent.py):** After `ResultMessage` with `status="success"`, if `require_pr` is True and `tool_call_counter.pr_created` is False:
- Emit `subtype="pipeline_incomplete"` instead of `"success"`.
- Set `termination_reason="Session completed without 'gh pr create' Bash call. CLAUDE_PILOT_REQUIRE_PR=1 was set. Work may be stranded in worktree."`.
- Set exit code to 1.

**No re-prompt on pipeline-incomplete.** Unlike the `min_tool_calls` early-exit guard, pipeline truncation happens after 20-30 turns of real work — re-prompt is unlikely to help because the model's "I'm done" signal is strong. Detection + classification first; re-prompt as a follow-up ticket if needed.

### Unit 3: dispatch-lib.sh — Enrich Callback Message with Outcome Classification

**File:** `mika/skills/bundled/_shared/dispatch-lib.sh`

**Change:** After all post-flight checks, add a summary classification line to `RESULT`:

```bash
# Outcome classification for operator/mika-dev consumption
if echo "$RESULT" | grep -qF "PIPELINE FAILURE:"; then
    RESULT="${RESULT}

Outcome: PIPELINE_INCOMPLETE — manual recovery needed."
elif [ -n "$PR_URL" ]; then
    RESULT="${RESULT}

Outcome: PR_OPENED — ${PR_URL}"
elif [ "$SKILL" = "dev-groom" ] && [ -n "$VALID_PLAN" ]; then
    RESULT="${RESULT}

Outcome: PLAN_GROOMED — ${VALID_PLAN}"
else
    RESULT="${RESULT}

Outcome: UNKNOWN — inspect worktree manually."
fi
```

This gives mika-dev a structured signal to parse instead of heuristic log inspection.

## Testing

### Unit 1 (dispatch-lib)
- Manually verify: create a worktree with commits but no PR, simulate the post-flight block with `SKILL=dev-pilot`, confirm `PIPELINE FAILURE` appears in output.
- Regression: existing `dev-groom` dispatches should NOT trigger the PR check (skill guard).

### Unit 2 (claude-pilot)
- Unit test: mock a session where `CLAUDE_PILOT_REQUIRE_PR=1`, `ResultMessage.subtype == "success"`, `ToolCallCounter.pr_created == False` → verify `ResultJson.subtype == "pipeline_incomplete"` and exit code 1.
- Unit test: same scenario but `pr_created == True` → verify normal `success` subtype.
- Unit test: `CLAUDE_PILOT_REQUIRE_PR` unset → verify no pipeline check runs, normal `success`.

### Unit 3 (dispatch-lib)
- Verify outcome classification appears in callback message for each scenario.

### Integration (Acceptance per ticket)
1. Re-fire dev-groom canary on a fresh ticket → claude-pilot reaches `gh pr create` OR reports `pipeline_incomplete`.
2. mika-dev close-out message shows structured outcome classification.
3. Across 5 dispatches: zero silent "Success but no PR" cases.

## Risks

1. **False positive on `gh pr create` substring match:** If the model runs `gh pr create --help` or `echo "gh pr create"`, the tracker would record a false positive. Acceptable — false positives are better than false negatives here, and the dispatch-lib layer provides a second check (actual PR existence on GitHub).
2. **Cross-repo coordination:** This touches both `mika/` (dispatch-lib.sh) and `claude-pilot-py/` (agent.py, permissions.py). Use the same branch name across repos per cross-repo convention. Sequential dispatch: mika first (dispatch-lib + env var), claude-pilot-py second (detection logic).
3. **mika#1032 merge ordering:** Both tickets modify dispatch-lib.sh post-flight logic. Non-overlapping sections, but implementer should verify #1032 state before starting. See Phase 0 for details.

## Sequence

1. Unit 2 first (claude-pilot) — the source-level detection.
2. Unit 1 + Unit 3 (dispatch-lib) — the consumption-side classification.
3. Both can be done in a single PR per repo (small, focused changes).

## References

- mika#938 — primary evidence (27 turns, $2.33, zero git/gh calls)
- mika#931 — secondary evidence (premature EndTurn at Phase 1)
- mika#1097 — early-exit guard pattern (prior art for `min_tool_calls`)
- dispatch-lib.sh lines 363-394 — existing post-flight checks
- `feedback_orchestrate_not_implement.md` — no auto-push/auto-PR
