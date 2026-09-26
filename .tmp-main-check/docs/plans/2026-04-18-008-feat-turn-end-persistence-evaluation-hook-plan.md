---
title: "feat: Engine turn-end persistence evaluation hook"
type: feat
status: active
date: 2026-04-18
---

# Engine Turn-End Persistence Evaluation Hook

## Overview

Add a 5th post-condition guard to the EndTurn chain in the agent loop. When the agent ends a turn without calling any persistence tool, and the turn's content suggests institutional knowledge was produced (diagnostic conclusions, user corrections, FYI acknowledgments, verdict-shaped output), the engine nudges the model to consider calling `store_fact` before finalizing. The model can accept the nudge and persist, or decline and end the turn normally.

This is the behavioral analog of the core_memory path guard (#645) and completion-claim guard (#483) — structural engine enforcement replacing unreliable prompt-layer rules.

## Problem Frame

mika-dev consistently fails to call `store_fact` after substantive turns. Diagnostic conclusions, design validations, user corrections, and institutional knowledge are produced as text and lost when the turn ends. Multiple prompt-level fixes have been tried across a single 12-hour session (soul.md hotpatch, active self_model update, fabrication risk block) — none stick reliably. The pattern is clear: behavioral invariants against model gradients need engine-level enforcement. See #648 iteration log.

## Requirements Trace

- R1. EndTurn evaluates the persistence heuristic on every conversation-mode turn
- R2. When the heuristic fires, the model receives a synthetic nudge message: "Persistence check: [reason]. Proceed with EndTurn or call store_fact first?"
- R3. The model can either persist (new tool call, turn continues) or confirm EndTurn (turn ends)
- R4. The nudge fires at most once per turn (single-retry flag pattern)
- R5. The guard only fires when no write-persistence tool was called during the turn
- R6. The guard only fires in conversation mode (not silent/team)
- R7. Unit tests cover: write-tool fired -> no flag; diagnostic user input + no write -> flag; simple ack -> no flag; fires-once-only
- R8. After shipping, compliance rate is measurable via existing observability (step count, tool call logs)

## Scope Boundaries

- The guard is a **nudge**, not a rejection — softer language than existing guards
- Content detection uses keyword/pattern matching, not semantic classification
- No new database schema or tables
- No changes to the tool registry or tool implementations
- Silent mode and team mode are excluded

### Deferred to Separate Tasks

- Pre-tool context check (#2 sibling concern) — requires semantic redundancy detection, ships later
- Compliance rate dashboard visualization — future iteration
- Tuning detection patterns based on observed false positive/negative rates — future iteration

## Context & Research

### Relevant Code and Patterns

- `crates/mika-agent/src/agent.rs` lines 580-592: retry flag initialization pattern
- `crates/mika-agent/src/agent.rs` lines 858-889: fabricated action-claim guard (closest analog — fires on EndTurn, checks `tools_called`, single retry)
- `crates/mika-agent/src/agent.rs` lines 2880-2933: detection function patterns (`detect_completion_claim`, `detect_fabricated_action_claim`) — fast-path substring check + regex
- `crates/mika-agent/src/agent.rs` line 582: `tools_called: HashSet<String>` — the tool call ledger
- `crates/mika-agent/tests/eval/test_completion_claim_guard.rs`: test structure to follow
- `crates/mika-agent/src/tools/mod.rs` lines 612-615: `store_fact`, `update_fact`, `update_core_memory` in `default_tools()`

### Institutional Learnings

- **Core memory path guard (#645):** Three-layer defense-in-depth (engine primary, prompt secondary, tool description tertiary). "When correctness depends on an invariant, enforce it structurally in the engine."
- **Completion-claim guard (#483):** Template for EndTurn guards: detect condition -> check `tools_called` -> single retry with correction message -> flag to prevent infinite loop. Gate on tool availability.
- **Fabricated action-claim guard (#308):** Guard family member #4, fires on zero tool calls + action verb + GitHub URL pattern.
- **Engine-level callback metadata extraction (#376):** Moving persistence from prompt-driven to engine-level when tool budget exhaustion causes persistence steps to be dropped.
- **Delegation task guard:** "The LLM can ignore or forget prompt instructions, especially after conversation compaction." Code-level guards required when non-compliance means data loss.

## Key Technical Decisions

- **Nudge, not rejection:** Unlike existing guards that say "Your response was rejected," this guard uses softer language ("Before ending this turn, consider whether..."). The model can legitimately decide nothing is worth persisting — the guard removes the "I didn't think to persist" failure mode, not the decision itself.
- **Conversation-mode only:** Silent/team modes are background tasks where persistence evaluation adds no value. Gate on `mode.is_conversation()`.
- **Conservative write-tool set:** Check `tools_called` for `store_fact`, `update_fact`, and `update_core_memory` only. These are the knowledge-persistence tools. Excluding `create_task`, `update_task_status`, etc. because those are workflow tools, not knowledge persistence — calling them doesn't mean the agent persisted learnings.
- **No tool-availability gate:** Unlike the completion-claim guard which checks `tools.get("update_task_status").is_some()`, all three persistence tools are in `default_tools()` and always available. No gate needed.
- **Guard position: 5th in chain:** After fabricated-action-claim guard (line 889), before `saves_to_db` block (line 891). This is the least critical guard (nudge vs. rejection), so it runs last.
- **Detection approach: keyword patterns on user message + assistant response:** Check the user's input for informational signals (FYI, diagnostic, maintenance) AND/OR the assistant's response for verdict-shaped output (validates, confirmed, verified, root cause, conclusion). Use fast-path substring checks + regex, matching existing detection function patterns.

## Open Questions

### Resolved During Planning

- **Should the guard check user input, assistant output, or both?** Both. The issue specifies two independent signals: (2) user input labeled informational/diagnostic, (3) assistant output containing verdict-shaped content. Either signal alone is sufficient to fire the guard (when combined with no write tools).
- **Should the guard fire on MaxTokens/ContentFilter?** No. Following existing guard convention — only `EndTurn`. MaxTokens/ContentFilter are unrecoverable contexts.
- **What constitutes a "write tool" for this guard?** `store_fact`, `update_fact`, `update_core_memory`. These are the knowledge-persistence tools. Other write tools (`create_task`, `write_agent_file`, etc.) don't indicate the agent persisted learnings from the conversation.

### Deferred to Implementation

- Exact regex patterns for detection — will be tuned during implementation based on the issue's examples and observed patterns
- Whether the fast-path substring list needs additional entries beyond the initial set

## High-Level Technical Design

> *This illustrates the intended approach and is directional guidance for review, not implementation specification. The implementing agent should treat it as context, not code to reproduce.*

```
EndTurn path (after guard 4, before saves_to_db):

  if stop_reason == EndTurn
     AND mode.is_conversation()
     AND NOT persistence_eval_retry_done
     AND NOT any of PERSISTENCE_WRITE_TOOLS in tools_called
     AND (detect_informational_input(user_message) OR detect_persistable_output(assistant_text)):
       
       persistence_eval_retry_done = true
       push assistant response to messages
       push nudge message: "[Persistence check: ...]"
       continue  // re-enter loop for one more LLM call
```

## Implementation Units

- [x] **Unit 1: Detection functions and constants**

**Goal:** Add the `PERSISTENCE_WRITE_TOOLS` constant and two detection functions (`detect_informational_input`, `detect_persistable_output`) to `agent.rs`.

**Requirements:** R5, R1

**Dependencies:** None

**Files:**
- Modify: `crates/mika-agent/src/agent.rs`
- Test: `crates/mika-agent/src/agent.rs` (inline `#[cfg(test)] mod tests`)

**Approach:**
- Define `PERSISTENCE_WRITE_TOOLS: &[&str] = &["store_fact", "update_fact", "update_core_memory"]` as a module-level constant
- `detect_informational_input(text: &str) -> Option<&str>`: fast-path substring check + regex for informational signals in the user's message. Patterns: FYI, diagnostic, maintenance check, heads up, just letting you know, for your information, self-assessment, status update, incident report, correction, actually/correction-shaped phrases
- `detect_persistable_output(text: &str) -> Option<&str>`: fast-path substring check + regex for verdict-shaped patterns in assistant output. Patterns: this validates, this confirms, root cause, conclusion, verified, diagnosed, determined, the issue was, lesson learned, key takeaway, institutional knowledge markers
- Both functions return `Option<&str>` with the matched pattern for logging (consistent with `detect_completion_claim`)
- Use `LazyLock<regex::Regex>` for compiled patterns (existing convention)

**Patterns to follow:**
- `detect_completion_claim()` at line 2888 — fast-path + regex pattern
- `detect_fabricated_action_claim()` at line 2925 — fast-path + regex, returns matched text for logging

**Test scenarios:**
- Happy path: `detect_informational_input("FYI the deploy succeeded")` returns `Some("FYI")`
- Happy path: `detect_persistable_output("This confirms the diagnosis — root cause was the timeout")` returns match
- Edge case: `detect_informational_input("Can you fix the FYI endpoint?")` — FYI as part of a technical term, should still match (conservative is fine — the guard is a nudge, not a rejection)
- Edge case: `detect_persistable_output("I'll verify the fix")` — future-tense "verify" should NOT match (only past-tense/present-conclusion patterns)
- Edge case: `detect_informational_input("")` returns `None`
- Edge case: `detect_persistable_output("Here's the code change")` returns `None` — normal response without verdict language

**Verification:**
- Detection functions compile and pass inline unit tests
- Pattern coverage matches the examples from issue #648

- [x] **Unit 2: Persistence evaluation guard in the EndTurn chain**

**Goal:** Wire the 5th post-condition guard into the agent loop, using the detection functions from Unit 1.

**Requirements:** R1, R2, R3, R4, R5, R6

**Dependencies:** Unit 1

**Files:**
- Modify: `crates/mika-agent/src/agent.rs`

**Approach:**
- Add `persistence_eval_retry_done: bool = false` flag at line ~592 (alongside other retry flags)
- Capture the user's input message text before the loop or at guard evaluation time — needed for `detect_informational_input`. The user message is already available in `request.messages` (the last user message before the loop starts)
- Insert guard 5 after line 889 (after fabricated-action-claim guard), before line 891 (`saves_to_db` block)
- Guard condition: `EndTurn` AND `mode.is_conversation()` AND `!persistence_eval_retry_done` AND no persistence write tool in `tools_called` AND (informational input OR persistable output detected)
- Nudge message text: softer than existing guards — `"[Before ending this turn, consider: {reason}. If any new information, conclusions, or corrections from this conversation should be remembered for future sessions, call store_fact now. If nothing warrants persistence, you may proceed with your response.]"`
- Where `{reason}` is derived from which detection matched (e.g., "this turn contains informational input (matched: 'FYI')" or "your response contains conclusions that may be worth persisting (matched: 'root cause')")
- Log at `info!` level (not `warn!` — this is a nudge, not an error condition)

**Patterns to follow:**
- Lines 858-889: fabricated action-claim guard structure (condition, flag set, warn, push assistant, push user, continue)
- Lines 791-851: completion-claim guard (lazy condition evaluation, contextual nudge message)

**Test scenarios:**
- Integration: guard fires when user says "FYI" and agent responds without calling store_fact -> 2 LLM steps
- Integration: guard does NOT fire when agent called store_fact during the turn -> normal step count
- Integration: guard does NOT fire when assistant text has no verdict/informational patterns -> 1 step
- Integration: guard fires at most once (second EndTurn without store_fact passes through) -> exactly 2 steps even if second response also has patterns
- Integration: guard does NOT fire in silent mode
- Error path: empty assistant text -> guard skips (no text to analyze)

**Verification:**
- Guard integrates into the EndTurn chain without breaking existing guards
- Nudge message appears in conversation history when fired
- Guard fires only once per turn

- [x] **Unit 3: Integration tests**

**Goal:** Create comprehensive eval harness tests for the persistence evaluation guard.

**Requirements:** R7

**Dependencies:** Unit 2

**Files:**
- Create: `crates/mika-agent/tests/eval/test_persistence_eval_guard.rs`
- Modify: `crates/mika-agent/tests/eval/mod.rs` (add module declaration)

**Approach:**
- Follow `test_completion_claim_guard.rs` structure exactly
- No stub tools needed — `store_fact`, `update_fact`, `update_core_memory` are in `default_tools()`
- Test both user-input detection and assistant-output detection paths independently
- Test the "fires once only" invariant

**Patterns to follow:**
- `crates/mika-agent/tests/eval/test_completion_claim_guard.rs` — test structure, harness usage, assertion patterns

**Test scenarios:**
- Happy path: `guard_fires_on_informational_input` — user sends "FYI the deploy completed without errors", agent responds with text only -> 2 steps (guard fires)
- Happy path: `guard_fires_on_persistable_output` — user sends normal message, agent responds with "Root cause was the connection timeout" -> 2 steps
- Happy path: `guard_skips_when_store_fact_called` — agent calls `store_fact` tool then responds with verdict text -> 2 steps (tool + response, no extra guard step)
- Happy path: `guard_skips_when_update_core_memory_called` — agent calls `update_core_memory` -> no guard fire
- Edge case: `guard_skips_on_normal_response` — user asks "what time is it?", agent responds with no verdict language -> 1 step
- Edge case: `guard_fires_once_only` — provide 3 mock responses: first triggers guard, second also has patterns but guard should not re-fire, verify exactly 2 steps
- Edge case: `guard_skips_on_empty_text` — agent responds with empty text -> no guard fire

**Verification:**
- All tests pass with `cargo test -p mika-agent --test eval`
- Test coverage matches acceptance criteria from issue #648

- [x] **Unit 4: Documentation**

**Goal:** Update CLAUDE.md with guard 5 documentation and add a solution doc.

**Requirements:** R8 (measurement documentation)

**Dependencies:** Unit 2

**Files:**
- Modify: `crates/mika-agent/CLAUDE.md`
- Create: `docs/solutions/architecture-patterns/persistence-evaluation-guard.md`

**Approach:**
- Add guard 5 entry to the "Post-Conditions (EndTurn Chain)" section in `crates/mika-agent/CLAUDE.md`, following the pattern of guards 1-4
- Create solution doc in `docs/solutions/architecture-patterns/` with YAML frontmatter (module: mika-agent, tags: [guard, persistence, endturn, store_fact], problem_type: behavioral-enforcement)
- Solution doc should reference the iteration log from #648 as motivation
- Document measurement approach: guard fires are observable via step count (2 steps = guard fired) and `info!` log lines; compliance rate = (turns where model persists after nudge) / (total nudge fires)

**Patterns to follow:**
- `docs/solutions/architecture-patterns/completion-claim-guard-work-item-state-enforcement.md` — solution doc structure
- `crates/mika-agent/CLAUDE.md` guard documentation entries 1-4

**Test expectation:** none -- documentation only

**Verification:**
- CLAUDE.md guard list updated to 5 entries
- Solution doc exists with proper frontmatter

## System-Wide Impact

- **Interaction graph:** The guard sits in the EndTurn chain between guard 4 (fabricated action-claim) and the `saves_to_db` block. It can cause one additional LLM call per turn. No interaction with tool dispatch, skill matching, or other subsystems.
- **Error propagation:** No new error paths — the guard only pushes messages and continues the loop. If detection functions panic (they won't — pure string matching), the existing loop error handling catches it.
- **State lifecycle risks:** None. The guard adds messages to the in-flight request but doesn't touch DB state, session state, or task state.
- **API surface parity:** No API changes. The guard is internal to the agent loop.
- **Integration coverage:** The eval harness tests exercise the full `run_agent()` path, including the guard interaction with other guards in the chain.
- **Unchanged invariants:** Existing guards 1-4 are untouched. The guard fires after all existing guards pass. Tool dispatch, skill matching, and message persistence are unaffected.

## Risks & Dependencies

| Risk | Mitigation |
|------|------------|
| False positives (guard fires on turns that don't need persistence) | The guard is a nudge, not a rejection — the model can decline. Low cost of false positives. Pattern tuning is deferred to future iteration based on observed rates. |
| False negatives (guard misses turns that need persistence) | Conservative initial pattern set is acceptable — any improvement over 0% prompt compliance is a win. Patterns can be expanded. |
| Performance: extra LLM call on nudge fires | Single retry pattern caps at +1 call. Same cost model as existing guards. |
| Detection patterns too broad in non-English contexts | Current deployment is English-only. Revisit if multi-language support is added. |

## Sources & References

- Related issue: #648 (this implementation)
- Related issue: #645 (core_memory path guard — sibling structural fix)
- Related issue: #483 (completion-claim guard — template pattern)
- Related issue: #308 (fabricated action-claim guard)
- Related code: `crates/mika-agent/src/agent.rs` (agent loop, detection functions)
- Related docs: `docs/solutions/architecture-patterns/completion-claim-guard-work-item-state-enforcement.md`
- Related docs: `docs/solutions/architecture-patterns/core-memory-path-guard-read-agent-file.md`
