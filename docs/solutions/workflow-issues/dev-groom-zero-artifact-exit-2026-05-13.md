---
module: skills/bundled/dev-groom, claude-pilot-py
tags: [autonomous-loop, dev-groom, claude-pilot, early-exit, guardrail]
problem_type: bug
category: workflow-issues
date: 2026-05-13
ticket: mika#1097
---

# dev-groom zero-artifact exit — 2026-05-13

## Problem

During the 2026-05-13 mass-dispatch window (07:28-07:31Z), 6 of 8 autonomous grooming sessions exited `status="success"` at ~12 turns / ~$0.40 / ~59s **without calling the architect and without committing any plan file**. The dispatch-lib post-flight checks (HEAD-unchanged, plan-file ≥500 bytes) caught the empty artifacts and rewrote results to `PIPELINE FAILURE`, but ~$2.80 was burned for zero output.

## Root cause analysis

### Layer correction

The issue body originally attributed the failure to kimi-k2.5 (mika-dev's base model). This is incorrect — the failing LLM runs inside the claude-pilot child process via `SubprocessCLITransport`, calling the Claude Code CLI against `MIKA_ANTHROPIC_API_KEY` (Sonnet). mika-dev's kimi is the upstream caller, not the session runner.

### Evidence shape

All 6 failing sessions shared:
- 12 turns, ~$0.40, ~59s
- Zero `[tool:request]` lines in the log between `[prompt]` and `[done]`
- `[init] Session , model unknown` (empty session_id, unknown model)
- Zero rows in `tool_calls` table for the session window

### Phase 0 diagnostic instrumentation (deployed)

Three diagnostic capabilities were added:

1. **Persistent stderr** (`dispatch-lib.sh`): stderr is now copied to `/var/log/claude-pilot/<task_id>.stderr` before processing, surviving callback delivery cleanup.

2. **`--trace` flag** (`claude-pilot`): Logs every `AssistantMessage` content block (text, thinking, tool_use, tool_result) to the file sink. Also logs `repr(SystemMessage)` for init events to diagnose the empty session_id / unknown model signal.

3. **Environment wire**: `CLAUDE_PILOT_TRACE=1` in dispatch-lib enables trace for specific skills.

### Phase 0 outcome

**Pending live reproduction.** Deploy the instrumented build and single-dispatch `mika ask --agent mika-dev "groom mika issue#1057"` with trace enabled. One of four outcomes will be named:

- **Outcome 0**: mika#1081's ROLE CONSTRAINT block is the proximate cause (Step 0-pre)
- **Outcome 1**: Model produces thinking blocks only and exits end_turn
- **Outcome 2**: Tool calls are attempted but denied/fail silently
- **Outcome 3**: Single-dispatch always works; mass-dispatch is the variable

## Fix layers (deployed)

### Layer A — Skill prompt hardening

1. Moved the ROLE CONSTRAINT block from a leading prefix (lines 1-4, pre-heading) to a bolded inline paragraph within the skill description section. This prevents the constraint from being parsed as a session-level system override that might truncate the workflow.

2. Added a **COMPLETION CONSTRAINT** clause explicitly warning about early-exit cost (~$0.40/session) and requiring all phases to complete.

3. Made Phase 1's `gh issue view` call **MANDATORY FIRST ACTION** with explicit instruction to execute before any reasoning.

### Layer B — Structural early-exit guard (claude-pilot)

Added a tool-call counter and re-prompt guard to claude-pilot:

1. `ToolCallCounter` in `permissions.py` increments on every allowed tool call (Tier 1 auto-approve and relay-allow).

2. On `ResultMessage` with `status="success"` and observed tool calls < `CLAUDE_PILOT_MIN_TOOL_CALLS`, the agent is re-prompted once with a spec-anchored corrective message.

3. On second early-exit after re-prompt, claude-pilot emits `status="terminated"` with `subtype="early_exit_zero_action"` — a named structural failure that dispatch-lib can parse.

4. Gated per-skill: `CLAUDE_PILOT_MIN_TOOL_CALLS=3` is set by dispatch-lib only for `dev-groom`. Dev-pilot threshold is a follow-up ticket after live calibration data accumulates.

### Layer D — Mass-dispatch rate-limit

Out of scope per issue body. To be filed as a sibling ticket if Phase 0 outcome 3 lands.

## Reproduction protocol

```bash
# Single-dispatch (always single, never mass):
mika ask --agent mika-dev "groom mika issue#1057"

# After completion, verify:
# 1. Check /var/log/claude-pilot/<task_id>.log for [tool:request] lines
# 2. Check /var/log/claude-pilot/<task_id>.stderr for full stderr
# 3. Query tool_calls: SELECT * FROM tool_calls WHERE session_id = ? ORDER BY id
# 4. Check parent task metadata for session details
```

## Related

- mika#1033 — dev-groom drift INTO executor mode (predecessor fix)
- mika#1058 — callback-safe deferred dispatch (upstream fix)
- mika#940 — dev-groom early-exit family root
- mika#864 — required-suffix-line guard (pattern for this guard)
