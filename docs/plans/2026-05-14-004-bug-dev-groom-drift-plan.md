---
title: "fix: Harden dev-groom against executor-mode drift on action-verb tickets"
type: fix
status: active
date: 2026-05-14
---

# fix: Harden dev-groom against executor-mode drift on action-verb tickets

## Overview

The dev-groom skill drifts into executor mode when ticket bodies contain imperative verbs and numbered action steps. Instead of planning, the LLM reads the ticket's commands ("rebase", "force-push", "run cargo build") as instructions to execute. This is the N+3 recurrence of the prompt-rule-cheapness-bias pattern — prompt-level ROLE CONSTRAINTs don't bind under cognitive load (per `docs/solutions/best-practices/prompt-rule-cheapness-bias-toward-wrong-layer-2026-04-28.md`).

## Problem Frame

Single observed incident (mika#1031): autonomous dev-groom dispatch produced an empty plan file (0 bytes), never invoked `/ce:plan`, pivoted to `df -h` disk-space triage and worktree cleanup, then exited `Success` in 17 turns / $0.73. Same day, three feature-ticket grooms succeeded — all had prose-heavy "what to plan" content, not imperative-verb action steps. The existing structural defenses catch the failure but only after the session burns its full budget:

- Post-flight plan validation (mika#1033): detects `docs/plans/<today>-*-plan.md` absent or <500 bytes → PIPELINE FAILURE.
- `CLAUDE_PILOT_MIN_TOOL_CALLS=3`: catches zero-tool-call exits but not "called the wrong tools" drift.
- Required-suffix-line guard: prevents exiting without `Verdict: GROOMED/ESCALATE` but doesn't prevent mid-session drift.

**Root cause hypothesis (H4, most supported by evidence):** `/ce:plan` was never invoked. The Write tool wrote empty content either as a placeholder that never got filled, or as a side-effect of the LLM entering executor mode. The ROLE CONSTRAINT at line 5 of the prompt ("You are a PLANNER, not an implementer") was necessary but insufficient — the LLM rationalized past it when presented with action-verb-dense input.

## Requirements Trace

- R1. Prevent dev-groom from executing ticket-body commands instead of planning
- R2. Detect `/ce:plan` non-invocation structurally, not just via plan-file size
- R3. Reduce wasted session cost when drift occurs (currently burns full budget before detection)
- R4. Fix must be structural (engine or transport layer), not solely prompt-based (per `feedback_prompt_enforcement_fragile.md`)

## Scope Boundaries

- Prompt-only changes are defense-in-depth, not the primary fix
- This plan does not address H1-H3 (action-verb density, numbered AC, chore-label correlation) as independent failure modes — they are narrowing hypotheses that collapse if H4 is fixed
- Tool-availability restriction (removing Bash from dev-groom sessions) is architecturally desirable but out of scope — Claude Code needs Bash for `gh issue view`, `git add/commit/push`, etc.

### Deferred to Separate Tasks

- claude-pilot required-command enforcement (`claude-pilot-py` repo): add a `required_commands` config to `.claude/claude-pilot.json` that structurally prevents session completion without invoking specific slash commands. This is the proper structural fix for the "wrong tools called" failure class. Filed as a follow-up.
- Ticket-shape filtering for the grooming dispatch path: whether chore-class tickets should bypass grooming entirely (H3). Separate architectural question.

## Context & Research

### Relevant Code and Patterns

- `skills/bundled/dev-groom/system_prompt.md` — 109-line prompt with ROLE CONSTRAINT (line 5), COMPLETION CONSTRAINT (line 7), Phase 1-6 workflow
- `skills/bundled/dev-groom/skill.toml` — `required_suffix_lines` active, no `[constraints]` section
- `skills/bundled/_shared/dispatch-lib.sh` — post-flight plan validation (line 373-385), `CLAUDE_PILOT_MIN_TOOL_CALLS=3` (line 534)
- `crates/mika-agent/src/agent.rs` — 9 post-condition guards on EndTurn (required-tools gate #3 is the closest structural analogue)
- `crates/mika-agent/src/skills/manifest.rs` — `Constraints` struct with `required_tools` field

### Institutional Learnings

- `docs/solutions/best-practices/dev-groom-drift-detection-structural-validation-2026-05-11.md` — predecessor fix (mika#1033), post-flight plan-file validation
- `docs/solutions/workflow-issues/dev-groom-zero-artifact-exit-2026-05-13.md` — 6/8 mass-dispatch grooms exited with zero tool calls; ToolCallCounter fix
- `docs/solutions/best-practices/prompt-rule-cheapness-bias-toward-wrong-layer-2026-04-28.md` — N=9 documented recurrences; structural ticket after N+1
- `docs/solutions/architecture-patterns/engine-guards-vs-prompt-rules-for-agent-behavior-2026-04-19.md` — classify by gradient direction; "execute these steps" fights the LLM's trained tool-use reflex
- `docs/solutions/best-practices/required-tools-gate-evasion-patterns-2026-04-28.md` — Rule 3: agent read its own failure history and still repeated the failure

## Key Technical Decisions

- **Primary fix is log-based /ce:plan invocation check in dispatch-lib.sh** — the only structural layer available inside the mika repo that can catch "wrong tools called" (not just "no tools called" or "no plan file"). Post-flight, but cheaper than re-running the session. Rationale: the required-tools gate in the Mika engine operates on Mika agent tool calls, not on what happens inside the Claude Code subprocess.
- **Prompt quarantine is defense-in-depth, not primary** — ticket body presented inside an explicit `<planning-input>` XML block with framing text. Reduces probability of drift but cannot guarantee prevention.
- **Post-flight check fires before callback delivery** — if drift is detected, the session result is overwritten with PIPELINE FAILURE before the callback reaches the parent task engine. This prevents mika-dev from seeing a false "Success" signal.

## Open Questions

### Resolved During Planning

- **Can the Mika engine's required-tools gate enforce `/ce:plan` invocation?** No. The required-tools gate operates on Mika agent tool calls (mika-dev calling `run_claude_pilot`). The dev-groom drift happens inside the Claude Code subprocess, which is a separate process. The Mika engine has no visibility into Claude Code's tool calls.
- **Can we remove Bash from dev-groom sessions?** Not without breaking the workflow. Dev-groom needs Bash for `gh issue view`, `git add/commit/push`, and `mika ask --agent mika-arch`.

### Deferred to Implementation

- Exact grep pattern for detecting `/ce:plan` invocation in claude-pilot session logs — depends on log format inspection during implementation.

## Implementation Units

- [ ] **Unit 1: Post-flight `/ce:plan` invocation check in dispatch-lib.sh**

**Goal:** Detect when a dev-groom session completes without invoking `/ce:plan`, flagging it as PIPELINE FAILURE before the callback delivers.

**Requirements:** R2, R3, R4

**Dependencies:** None

**Files:**
- Modify: `skills/bundled/_shared/dispatch-lib.sh`
- Test: Manual verification via a dev-groom dispatch (the dispatch-lib is a shell script invoked by the skill executor; no unit test harness exists for it)

**Approach:**
- Add a new post-flight check after the existing plan-file validation (line 385) but before callback delivery
- Read the claude-pilot session log (path available via `CLAUDE_PILOT_LOG_DIR` or derived from session ID)
- Grep for evidence of `/ce:plan` invocation (the Skill tool call with `skill: "ce-plan"` or the `/ce:plan` command string)
- If not found AND the session exited successfully, overwrite RESULT with `PIPELINE FAILURE: dev-groom session completed without /ce:plan invocation. Session likely drifted into executor mode.`
- This check is additive — the existing plan-file-size check remains as a complementary guard

**Patterns to follow:**
- Existing plan-file validation block at lines 373-385 of dispatch-lib.sh
- Same `PIPELINE FAILURE` result-overwrite pattern

**Test scenarios:**
- Happy path: dev-groom session with `/ce:plan` invocation and valid plan file → passes both checks
- Error path: dev-groom session without `/ce:plan` invocation → PIPELINE FAILURE emitted with invocation-check reason
- Edge case: session log unavailable (e.g., log path misconfigured) → warn but don't block; fall through to plan-file check
- Edge case: `/ce:plan` invoked but plan file is empty → caught by existing plan-file-size check (complementary)

**Verification:**
- `grep -c "ce.plan" dispatch-lib.sh` returns ≥1 in the post-flight section
- A dev-groom dispatch that skips `/ce:plan` produces a PIPELINE FAILURE callback result

- [ ] **Unit 2: Ticket-body quarantine in dev-groom prompt**

**Goal:** Reduce the probability of executor-mode drift by presenting ticket content inside an explicit planning-input frame that the LLM is less likely to interpret as execution instructions.

**Requirements:** R1 (defense-in-depth for R4)

**Dependencies:** None (independent of Unit 1)

**Files:**
- Modify: `skills/bundled/dev-groom/system_prompt.md`

**Approach:**
- Add a structural directive before Phase 1 that instructs the LLM to wrap the ticket body in a `<planning-input>` block when processing it
- Reinforce that action verbs in the ticket body are WHAT to plan, not what to execute — the existing ROLE CONSTRAINT says this but it's a single sentence; expand with a negative-example pattern
- Add an explicit FORBIDDEN ACTIONS block listing the concrete commands from the incident (`git rebase`, `git push --force`, `cargo build`, `cargo test`, `df -h`) as examples of what the LLM must never execute from ticket content
- Keep the prompt change surgical — add 5-10 lines max, not a full rewrite

**Execution note:** This is prompt work, not code. The prompt change is defense-in-depth; its effectiveness is inherently probabilistic. The structural guarantee comes from Unit 1.

**Patterns to follow:**
- Existing ROLE CONSTRAINT block (line 5) and COMPLETION CONSTRAINT block (line 7) — same heading style
- The `<planning-input>` XML tag pattern used by other Mika skill prompts for input framing

**Test scenarios:**
- Test expectation: none — prompt changes cannot be structurally tested. Validation comes from subsequent dev-groom dispatches on chore-class tickets.

**Verification:**
- `grep -c 'FORBIDDEN' skills/bundled/dev-groom/system_prompt.md` returns ≥1
- The ROLE CONSTRAINT block is expanded, not replaced
- Total prompt length increase is ≤15 lines

## System-Wide Impact

- **Interaction graph:** dispatch-lib.sh → claude-pilot subprocess → callback delivery to task engine. The new check inserts between subprocess exit and callback.
- **Error propagation:** PIPELINE FAILURE result flows through the existing callback delivery path unchanged. The task engine already handles this result class.
- **State lifecycle risks:** None — the post-flight check is read-only (reads session log). The prompt change has no state implications.
- **API surface parity:** No other skill uses this `/ce:plan` invocation check. It's dev-groom-specific, matching the existing plan-file-size check.
- **Unchanged invariants:** The existing plan-file-size check (mika#1033), CLAUDE_PILOT_MIN_TOOL_CALLS, and required-suffix-line guard all remain active. The new check is additive.

## Risks & Dependencies

| Risk | Mitigation |
|------|------------|
| Session log format varies across claude-pilot versions | Grep pattern should be broad (`ce.plan` or `ce-plan` with flexible matching); fall through on log-unavailable |
| Prompt quarantine may reduce false positives but not eliminate them | Unit 1 provides the structural catch; prompt is defense-in-depth only |
| Post-flight check burns session cost before detecting drift | Deferred: claude-pilot required-command enforcement is the proper mid-session prevention (separate repo) |

## Sources & References

- Related issues: mika#1031 (incident), mika#1032 (this bug), mika#1033 (predecessor plan-file validation)
- Prior art: `docs/solutions/best-practices/dev-groom-drift-detection-structural-validation-2026-05-11.md`
- Prior art: `docs/solutions/workflow-issues/dev-groom-zero-artifact-exit-2026-05-13.md`
- Design principle: `docs/solutions/best-practices/prompt-rule-cheapness-bias-toward-wrong-layer-2026-04-28.md`
- Design principle: `docs/solutions/architecture-patterns/engine-guards-vs-prompt-rules-for-agent-behavior-2026-04-19.md`
