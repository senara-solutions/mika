---
status: complete
priority: p1
issue_id: "554"
tags: [code-review, security, dead-code]
dependencies: []
---

# callback_context Prompt Guard Is Dead Code

## Problem Statement

The `PromptContext.callback_context` field is added and the prompt builder has logic to render a "Callback Result Turn" section, but `run_agent_inner()` always sets it to `None`. The `AgentParams.is_callback_turn` controls the code guard (blocking `LongRunningContext`) but is never wired through to the prompt guard. The brainstorm explicitly designed dual-layer defense-in-depth (code + prompt), but only the code layer works.

Without the prompt guard, the agent may attempt to call skills that fail at the executor level, producing confusing errors. A crafted callback result could also instruct the agent to call non-long-running tools (write_file, update_core_memory, etc.) since the prompt doesn't tell it to restrict behavior.

## Findings

- **Found by:** Architecture Strategist, Security Sentinel, Pattern Recognition, Agent-Native Reviewer, Code Simplicity Reviewer (5/8 agents)
- **Location:** `crates/mika-agent/src/agent.rs:664` — always `callback_context: None`
- **Design doc:** Brainstorm explicitly calls for both code guard AND prompt guard

## Proposed Solutions

### Option A: Wire it up (Recommended)
- In `run_agent_inner()`, set `callback_context: if params.is_callback_turn { Some("Processing callback results.") } else { None }`
- The prompt builder already handles `Some(...)` correctly
- **Pros:** Completes the defense-in-depth design, guides agent behavior
- **Cons:** None — the infrastructure is already built
- **Effort:** Small (1 line change)
- **Risk:** Low

### Option B: Remove it entirely
- Delete `callback_context` from `PromptContext`, remove prompt builder logic, remove from all 25 test sites
- Rely solely on the code guard (`lr_ctx = None`)
- **Pros:** Removes ~37 LOC of dead code, simpler
- **Cons:** Loses defense-in-depth, agent may make confusing tool calls
- **Effort:** Small
- **Risk:** Low (code guard still works)

## Recommended Action

Option A — wire it up. The infrastructure exists; it just needs one line connected.

## Technical Details

- **Affected files:** `crates/mika-agent/src/agent.rs` (line 664)
- **Components:** Agent loop, prompt builder

## Acceptance Criteria

- [ ] When `is_callback_turn: true`, system prompt contains "Callback Result Turn" section
- [ ] When `is_callback_turn: false`, system prompt does NOT contain that section
- [ ] Test: `build_system_prompt` with `callback_context: Some(...)` produces expected output

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-03-07 | Created from code review | 5/8 agents flagged independently |
| 2026-03-07 | Approved during triage | Wire it up — 1 line fix in run_agent_inner |

## Resources

- Brainstorm: `docs/brainstorms/2026-03-07-callback-tui-delivery-brainstorm.md`
