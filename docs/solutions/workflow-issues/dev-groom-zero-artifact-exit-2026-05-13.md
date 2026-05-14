---
module: skills/bundled/dev-groom, claude-pilot-py
tags: [autonomous-loop, dev-groom, claude-pilot, early-exit, guardrail, dispatch-lib]
problem_type: bug
category: workflow-issues
date: 2026-05-13
ticket: mika#1097
resolution_type: structural_guard
---

# dev-groom zero-artifact exit — 2026-05-13

## Problem

During the 2026-05-13 mass-dispatch window (07:28-07:31Z), 6 of 8 autonomous grooming sessions exited `status="success"` at ~12 turns / ~$0.40 / ~59s **without calling the architect and without committing any plan file**. The dispatch-lib post-flight checks (HEAD-unchanged, plan-file ≥500 bytes) caught the empty artifacts and rewrote results to `PIPELINE FAILURE`, but ~$2.80 was burned for zero output.

## Symptoms

- claude-pilot log: `[done] Success | 12 turns | $0.40 | 59s` with zero `[tool:request]` lines between `[prompt]` and `[done]`
- `[init] Session , model unknown` — empty session_id, unknown model in the init event
- Zero rows in `tool_calls` table for the session window
- Parent task marked `failed` with `callback_delivered_without_pr_url`
- All 6 failures occurred during a 3-minute mass-dispatch window (8 concurrent grooming tasks)

## What didn't work

1. **mika#1033's drift detection** — that fix caught sessions drifting INTO executor mode (running ticket commands instead of planning). It could not prevent sessions drifting OUT of all work (LLM emitting `end_turn` without any tool calls).
2. **The ROLE CONSTRAINT block (mika#1081)** — placed as a bare prefix (lines 1-4 before any heading) in `dev-groom/system_prompt.md`. Hypothesis: the model may have interpreted this as a session-level constraint that inhibited tool use entirely, rather than as a planning-mode directive. The block's position before the heading made it structurally ambiguous.
3. **Existing guardrails (stall/empty/idle)** — the stall detector triggers on consecutive text-only turns, but the failing sessions had zero content blocks at all (not even text), so the stall counter was never incremented.

## Root cause analysis

### Layer correction

The issue body originally attributed the failure to kimi-k2.5 (mika-dev's base model). This is incorrect — the failing LLM runs inside the claude-pilot child process via `SubprocessCLITransport`, calling the Claude Code CLI against `MIKA_ANTHROPIC_API_KEY` (Sonnet). mika-dev's kimi is the upstream caller, not the session runner.

### Evidence shape

All 6 failing sessions shared the same signature: 12 turns of SDK-level "turns" (likely thinking blocks or empty content) followed by `stop_reason=end_turn`. Without `--trace` logging, the actual content of those 12 turns was invisible — neither `log_text` (requires text blocks) nor `log_tool_request` (requires permission callbacks) fired.

### Phase 0 diagnostic instrumentation (deployed)

Three diagnostic capabilities were added to close the visibility gap:

1. **Persistent stderr** (`dispatch-lib.sh`): stderr is now copied to `/var/log/claude-pilot/<task_id>.stderr` before processing, surviving callback delivery cleanup.

2. **`--trace` flag** (`claude-pilot`): Logs every `AssistantMessage` content block (text, thinking, tool_use, tool_result) to the file sink. Also logs `repr(SystemMessage)` for init events to diagnose the empty session_id / unknown model signal.

3. **Environment wire**: `CLAUDE_PILOT_TRACE=1` in dispatch-lib enables trace for specific skills.

### Phase 0 outcome

**Outcome 0 — Skill-context vocabulary mismatch (mechanism (a): required_suffix_lines guard didn't fire on the architect response).**

The grooming for mika#1097 completed structurally — mika-arch reviewed the plan in two passes (session `166ff701-7ff7-4f90-8b16-b1c0f27c382d`) — but the second-pass verdict used the wrong vocabulary, blocking downstream dispatch.

**Evidence chain (from `~/.mika/data/mika.db`):**

1. **Both passes ran under the first-pass skill context.** All `llm_calls` rows for session `166ff701` have `prompt_variant = {"mika-arch-groom-ticket":"base"}` — including the second-pass response (row `153e6316`). The `mika-arch-second-review` skill was never activated.

2. **The architect emitted `Disposition: READY` instead of `Verdict: GROOMED`.** Message `33773` (second-pass response): *"Stored. The mika#1097 GROOMED verdict and the key diagnostic decisions are now persisted… Disposition: READY"*. The model believed it was emitting a GROOMED verdict (note the body text saying "GROOMED verdict") but used the first-pass keyword (`Disposition:`) and value (`READY`) instead of the second-pass vocabulary (`Verdict: GROOMED`).

3. **The wrong guard accepted the response.** `mika-arch-groom-ticket/skill.toml` has `required_suffix_lines = ["Disposition: READY", "Disposition: ITERATE", "Disposition: ESCALATE"]`. Since the skill context stayed first-pass, `Disposition: READY` matched the guard and was accepted. The second-pass guard (`mika-arch-second-review/skill.toml`: `required_suffix_lines = ["Verdict: GROOMED", "Verdict: ESCALATE"]`) never ran.

4. **The groomer faithfully transcribed "READY" into the issue body callout.** mika-dev sessions `a17e82c2` and `a77e701e` show the engine gate `dispatch_no_grooming_marker` rejecting the dispatch because the issue body contained `second-pass (READY)` instead of the required `second-pass (GROOMED)` verbatim. Session `018cf783` shows dispatch eventually succeeded after manual intervention.

**Root cause:** The two-pass grooming ran in a single mika-arch session (`166ff701`), and the skill context was bound to `mika-arch-groom-ticket` at session creation and never switched to `mika-arch-second-review` for the second pass. The `required_suffix_lines` guard is per-skill, so the first-pass guard's vocabulary (`Disposition: READY/ITERATE/ESCALATE`) was applied on both passes. The vocabulary divergence between the two skills (`Disposition:` vs `Verdict:`, `READY` vs `GROOMED`) meant the second-pass response passed validation under the wrong guard but was semantically wrong for the downstream dispatch gate.

**Disposition of original four outcomes:**

- **Outcome 0** (mika#1081 ROLE CONSTRAINT): Not directly tested in this reproduction, but the fix (Layer A) rewrites the prompt as defense-in-depth.
- **Outcome 1** (thinking-only exit): Not observed — the architect session had 10 tool calls and substantial content.
- **Outcome 2** (tools denied/fail): Not observed — all tool calls succeeded.
- **Outcome 3** (mass-dispatch variable): Partially confirmed — the original 6/8 failures occurred during mass-dispatch; the single-dispatch grooming of #1097 itself worked (architect reviewed and approved).

## Solution

### Layer A — Skill prompt hardening (`dev-groom/system_prompt.md`)

```markdown
# Before (mika#1081 — pre-heading bare prefix)
ROLE CONSTRAINT: You are a PLANNER, not an implementer...
## dev-groom — Two-Pass Grooming Skill

# After (mika#1097 — inline within skill description)
## dev-groom — Two-Pass Grooming Skill
...skill description...
**ROLE CONSTRAINT:** You are a PLANNER, not an implementer...
**COMPLETION CONSTRAINT (mika#1097):** You MUST complete all phases...
### Phase 1 — Read the ticket and pick the branch (MANDATORY FIRST ACTION)
1. **IMMEDIATELY** fetch the issue — this must be your FIRST tool call...
```

Three changes: (1) moved ROLE CONSTRAINT inline after the heading, (2) added COMPLETION CONSTRAINT with cost warning, (3) made Phase 1's `gh issue view` a mandatory first tool call.

### Layer B — Structural early-exit guard (`claude-pilot`)

Added `ToolCallCounter` to `permissions.py` — increments on every allowed tool call (Tier 1 auto-approve and relay-allow). In `agent.py`, when a `ResultMessage` arrives with `status="success"` and observed tool calls < `CLAUDE_PILOT_MIN_TOOL_CALLS`:

1. **First early-exit** → re-prompt once with spec-anchored corrective (no executor-directive verbs to avoid mika#1033 family regression)
2. **Second early-exit** → emit `status="terminated"`, `subtype="early_exit_zero_action"` (exit code 1)
3. **Re-prompt failure** → emit the same terminated result with the exception details

Gated per-skill via `CLAUDE_PILOT_MIN_TOOL_CALLS` env var. dispatch-lib sets it to 3 for `dev-groom` only (calibrated from incident: failures had 0 tool calls, successful sessions have 15-40+).

## Why this works

The guard addresses the failure at two layers:

- **Layer A** (prompt) reduces the probability of early exit by making the first tool call structurally required and warning about cost consequences. This is fragile (prompt-only) but cheap.
- **Layer B** (code) catches early exits that slip past the prompt and either recovers them (re-prompt) or names them (early_exit_zero_action subtype). This is structural and reliable.

Together, the layers provide defense-in-depth: Layer A prevents, Layer B detects and recovers.

## Prevention

1. **New dispatch skills should set `CLAUDE_PILOT_MIN_TOOL_CALLS`** in the dispatch-lib case switch. The threshold should be calibrated from production data (check successful session tool-call counts).

2. **Prompt-first actions should be tool calls, not reasoning.** When a skill's first phase requires fetching external data (issue metadata, branch state), make the fetch the mandatory first action with explicit "before any reasoning" language.

3. **Use `--trace` for debugging zero-artifact sessions.** Set `CLAUDE_PILOT_TRACE=1` before dispatch to get full content-block logs. The trace file lives alongside the regular log at `/var/log/claude-pilot/<task_id>.log`.

4. **Never mass-dispatch grooming tasks** until the queue-depth variable is resolved. Single-dispatch one at a time. The 2026-05-13 incident had 8 concurrent grooms in 3 minutes; 6 of 8 failed.

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
- mika#1081 — ROLE CONSTRAINT block addition (possible regression suspect)
