---
title: "feat: Generalize intent-precondition guard (#696 expansion) + add resume keywords"
type: feat
status: active
date: 2026-04-21
issue: "#702"
---

# feat: Generalize intent-precondition guard (#696 expansion) + add resume keywords

## Overview

Generalize the webhook zero-tools guard (#696) into a registry-driven intent-precondition system in `agent.rs`, add resume/continue keywords to `self-dev/skill.toml`, and condense the `Resume Semantics` section in `system_prompt.md` to a pointer referencing the structural guard.

## Problem Frame

mika-dev failed to resume milestone#8 when instructed with `mika ask --agent mika-dev "resume mika milestone#8"`. Root cause is two gaps:

1. **Missing keywords:** "resume"/"continue" aren't in `self-dev/skill.toml` keywords, so the skill matches via `AlwaysOn` not `Keyword`, which means `required_tools = ["run_claude_pilot"]` is never enforced (#265 conditioning).
2. **No structural guard for resume intent:** The webhook zero-tools guard (#696) catches webhook fabrication but nothing catches resume-intent turns where the agent makes zero qualifying reconciliation calls. The Resume Semantics section in `system_prompt.md` describes the algorithm but has no structural enforcement.

These are structurally identical to #696 — user intent implies required action, agent makes zero qualifying tool calls, no guard fires. Rather than adding a second point guard, generalize #696 into a registry.

## Requirements Trace

- R1. `self-dev/skill.toml` keywords include "resume" and "continue"
- R2. `agent.rs` contains an intent-precondition registry; existing #696 webhook guard refactored to use it (no behavior change on webhook path)
- R3. Resume-intent entry: trigger matches resume/continue + process ref (`milestone#`, `project#`), requires `check_task` or `list_tasks` with success before EndTurn accept
- R4. Single-retry rejection pattern matches #648/#696 precedent (one retry flag per intent, inject correction once, accept on second attempt with warning)
- R5. Eval harness tests for the resume-intent guard
- R6. `Resume Semantics` section condensed to a pointer referencing the guard

## Scope Boundaries

- No required-sequence semantics (accept-on-Nth-retry) — single-retry only, matching all 7 existing guards
- No engine-side `mika ask --resume-milestone N` CLI flag — separate ticket if needed
- No KG integration — guard is substrate-independent
- No changes to the `collect_required_tools()` / `MatchReason` conditioning (#265) — that system stays as-is
- The persistence evaluation guard (#648, guard #7) is NOT part of the registry — it's a nudge, not a rejection, and has different semantics

### Deferred to Separate Tasks

- Required-sequence guard semantics (reject-forever vs accept-on-Nth-retry) — revisit after this single-retry version is observed in production

## Context & Research

### Relevant Code and Patterns

- `crates/mika-agent/src/agent.rs:595-617` — 7 retry flag declarations, all following `let mut <name>_retry_done = false` pattern
- `crates/mika-agent/src/agent.rs:1013-1049` — webhook zero-tools guard (#696): checks `user_input_text.starts_with("[GitHub]")` + `!all_tool_summaries.iter().any(|s| s.success)`, single retry with correction message
- `crates/mika-agent/src/agent.rs:828-888` — required-tools gate: checks `effective_required_tools` against `tools_called`, terminal failure bypass
- `crates/mika-agent/src/agent.rs:3243-3404` — detection functions for other guards (`detect_completion_claim`, `detect_fabricated_action_claim`, etc.)
- `crates/mika-agent/tests/eval/test_webhook_zero_tools_guard.rs` — 5 eval tests covering: fires on zero tools, skips when tools succeed, skips on regular messages, single-retry semantics, fires on other webhook types
- `skills/bundled/self-dev/skill.toml` — current keywords (7 entries, missing resume/continue), `required_tools = ["run_claude_pilot"]`
- `skills/bundled/self-dev/system_prompt.md:527-567` — Resume Semantics section with algorithmic steps

### Institutional Learnings

- `docs/solutions/architecture-patterns/engine-guards-vs-prompt-rules-for-agent-behavior-2026-04-19.md` — "When a behavioral rule fights a model's trained gradient, structural engine-level guards bind deterministically. Prompt-level rules are unreliable."
- `docs/solutions/prompt-enforcement-structural-guards.md` — The "As Above, So Below" pattern: engine holds invariant via structural guards; prompt mirrors during transition window, then simplifies.
- `docs/solutions/architecture-patterns/webhook-zero-tools-guard-fabrication-prevention-2026-04-20.md` — #696 guard design rationale and implementation pattern.

## Key Technical Decisions

- **Registry as const array, not trait objects:** Use `&[IntentPrecondition]` with function pointers, not `Box<dyn Guard>`. Matches the codebase's preference for static dispatch and keeps the guard chain zero-allocation. The existing guards all use inline code with simple boolean flags — the registry is a lightweight extraction, not a framework.
- **Registry replaces only guards #6 (webhook) and new resume-intent:** Guards #1-5 and #7 have heterogeneous logic (regex detection, tool registry lookups, nudge-not-reject) that don't fit the "trigger fn + required tool signature" pattern cleanly. The registry is for the specific class of "user intent implies required action, check tool summaries" guards.
- **Resume intent detection via regex:** `detect_resume_intent()` matches `(?i)\b(resume|continue)\b` combined with `(?i)\b(milestone#|project#|milestone\s*#|project\s*#)` in the user message. This is tighter than bare "resume" to avoid false positives on general conversation.
- **Required tool check uses `any` not `all`:** The resume guard requires that at least one of `check_task` or `list_tasks` succeeded (reconciliation happened). This mirrors the webhook guard's `any(|s| s.success)` pattern rather than requiring all tools.
- **Single retry flag per registry entry, not one shared flag:** Each `IntentPrecondition` gets its own retry-done tracking. This preserves the existing per-guard isolation — if webhook fires and resume also applies in the same turn (unlikely but possible), each gets its own retry.

## Open Questions

### Resolved During Planning

- **Should the registry cover all 7 guards?** No — only guards with the "trigger + required tool signature" pattern. Guards #1-5 and #7 have heterogeneous logic. Start with #6 + resume-intent; expand later if pattern proves useful.
- **Should "resume" alone (without process ref) trigger the guard?** No — "resume" in general conversation would cause false positives. Require both a resume verb AND a process reference (`milestone#`, `project#`).

### Deferred to Implementation

- Exact correction message text for resume-intent — will follow the style of the webhook guard's correction message but reference the Resume Semantics section.

## High-Level Technical Design

> *This illustrates the intended approach and is directional guidance for review, not implementation specification. The implementing agent should treat it as context, not code to reproduce.*

```
struct IntentPrecondition {
    label: &'static str,
    trigger: fn(&str) -> bool,                    // user_input_text -> matches?
    satisfied: fn(&[ToolCallSummary]) -> bool,     // tool summaries -> precondition met?
    correction_message: &'static str,
}

const INTENT_GUARDS: &[IntentPrecondition] = &[
    // Existing #696 behavior, extracted
    IntentPrecondition { label: "webhook_zero_tools", ... },
    // New resume-intent guard
    IntentPrecondition { label: "resume_reconcile", ... },
];

// In the guard chain (replacing current guard #6 location):
// For each entry in INTENT_GUARDS:
//   if trigger matches AND NOT satisfied AND NOT retry_done[label]:
//     set retry_done, inject correction, continue
```

Retry tracking uses a `HashSet<&'static str>` keyed by label (or a small fixed-size array indexed by position) instead of individual boolean variables. This scales with registry entries without adding new variable declarations.

## Implementation Units

- [ ] **Unit 1: Add resume/continue keywords to self-dev skill.toml**

**Goal:** Enable keyword-based matching for resume/continue messages so `required_tools` enforcement activates via the existing #265 conditioning.

**Requirements:** R1

**Dependencies:** None

**Files:**
- Modify: `skills/bundled/self-dev/skill.toml`

**Approach:**
- Add `"resume"` and `"continue"` to the `[triggers] keywords` array
- This is the minimal fix that would have caught the original failure on its own

**Patterns to follow:**
- Existing keyword entries in the same file

**Test scenarios:**
- Test expectation: none — keyword matching is already covered by existing `match_skills` unit tests in `skills/matcher.rs`. The new keywords are plain strings that go through the same substring-match path.

**Verification:**
- `skill.toml` parses correctly (cargo build succeeds)
- Keywords array contains both new entries

---

- [ ] **Unit 2: Implement intent-precondition registry in agent.rs**

**Goal:** Extract the webhook zero-tools guard (#696) into a registry-driven pattern and add the resume-intent guard as the second entry. No behavior change on the webhook path.

**Requirements:** R2, R3, R4

**Dependencies:** Unit 1

**Files:**
- Modify: `crates/mika-agent/src/agent.rs`
- Test: `crates/mika-agent/tests/eval/test_intent_precondition_guard.rs`

**Approach:**
- Define `IntentPrecondition` struct near the existing guard helper functions (around line 3000+)
- Define `INTENT_GUARDS` const array with two entries: `webhook_zero_tools` (existing behavior) and `resume_reconcile` (new)
- Add `detect_resume_intent(msg: &str) -> bool` function using regex (same pattern as `detect_completion_claim`, `detect_fabricated_action_claim`)
- Add `resume_reconcile_satisfied(summaries: &[ToolCallSummary]) -> bool` — checks for successful `check_task` or `list_tasks` call
- Replace the inline webhook guard code (lines 1013-1049) with a loop over `INTENT_GUARDS`
- Replace `webhook_zero_tools_retry_done` boolean with a `HashSet<&'static str>` for registry-based retry tracking
- Keep `skip_remaining_guards` check (PR review early-accept still skips these guards)
- The correction message for resume-intent should reference the Resume Semantics section

**Patterns to follow:**
- Existing guard pattern: check `matches!(response.stop_reason, LlmStopReason::EndTurn)`, retry flag, push assistant + correction messages, `continue`
- Detection functions: `detect_completion_claim()`, `detect_fabricated_action_claim()` in the same file
- `ToolCallSummary` fields: `name`, `success` for checking tool outcomes

**Test scenarios:**
- Happy path: Resume-intent message + agent calls `list_tasks` successfully -> guard does not fire, turn completes normally
- Happy path: Resume-intent message + agent calls `check_task` successfully -> guard does not fire
- Edge case: Resume-intent message + zero tool calls -> guard fires, re-prompt injected, agent retries with tool call -> succeeds
- Edge case: Resume-intent message + `list_tasks` called but failed -> guard fires (requires success)
- Edge case: Resume-intent message + unrelated tool called successfully but not `check_task`/`list_tasks` -> guard fires
- Edge case: Single-retry semantics — guard fires once, if agent still doesn't call reconciliation tools on retry, turn ends normally (no infinite loop)
- Edge case: Regular message without resume intent -> guard does not fire
- Edge case: Message with "resume" but no process ref (no "milestone#" or "project#") -> guard does not fire
- Integration: Webhook guard still works identically — `[GitHub]` prefix + zero tools -> fires; `[GitHub]` + successful tool -> skips
- Integration: Both guards could theoretically match in same turn (unlikely) — each gets independent retry tracking

**Verification:**
- `cargo test -p mika-agent --test eval` passes
- Webhook guard behavior unchanged (existing tests still pass)
- New resume-intent tests cover fire/skip/single-retry paths

---

- [ ] **Unit 3: Condense Resume Semantics section in system_prompt.md**

**Goal:** Replace the algorithmic content of the Resume Semantics section with a pointer referencing the structural guard, following the "As Above, So Below" pattern.

**Requirements:** R6

**Dependencies:** Unit 2

**Files:**
- Modify: `skills/bundled/self-dev/system_prompt.md`

**Approach:**
- Keep the `## Resume Semantics` heading and subheadings (`### Milestone/Project Resume`, `### Manual Commands`)
- Condense the algorithmic steps (lines 527-567) to a brief pointer: reference the intent-precondition guard in `agent.rs` as the structural enforcement, keep the `list_tasks` → match by `reference_url` → resume execution algorithm as a concise reference (the LLM still needs to know WHAT to do, just not bear the enforcement burden)
- Keep the Manual Commands section (`continue`, `skip`, `stop`) — these are command semantics, not enforcement
- Do NOT rewrite the entire section — minimal change, per issue guidance

**Patterns to follow:**
- Existing pattern in `system_prompt.md` Callback Entry Point section (line 95-117): references #696 guard inline — "This rule is structurally enforced by the engine — webhook turns with zero successful tool calls will be rejected (#696)"

**Test scenarios:**
- Test expectation: none — prose change, no behavioral change. The structural guard (Unit 2) is the enforcement mechanism.

**Verification:**
- `system_prompt.md` still contains Resume Semantics section
- Section references the structural guard (#702)
- Manual Commands section preserved intact

## System-Wide Impact

- **Interaction graph:** The intent-precondition registry sits in the same guard chain position as the current webhook guard (#6). It interacts with: `skip_remaining_guards` (PR review early-accept still skips), `all_tool_summaries` (read-only), `user_input_text` (read-only). No new callbacks or middleware.
- **Error propagation:** Guard rejection follows existing pattern — push correction message, `continue` to re-prompt. No new error types.
- **State lifecycle risks:** Retry tracking changes from individual booleans to a `HashSet`. The set is local to the agent loop iteration — no persistence, no cross-turn state. Risk is minimal.
- **API surface parity:** No API changes. The guard is internal to the agent loop.
- **Integration coverage:** Eval harness tests verify the guard chain end-to-end via `MockLlmProvider` sequences.
- **Unchanged invariants:** Guards #1-5 and #7 are untouched. The `collect_required_tools()` / `MatchReason` conditioning (#265) is untouched. The `required_tools` gate (guard #3) still operates independently — Layer A (keywords) makes it fire on resume messages, and the intent registry (Layer B) provides a second safety net for the case where keyword matching alone isn't sufficient.

## Risks & Dependencies

| Risk | Mitigation |
|------|------------|
| Resume intent regex matches too broadly (false positives) | Require both resume verb AND process ref (`milestone#`/`project#`). Eval tests cover negative cases. |
| Refactoring webhook guard into registry introduces regression | Eval tests in `test_webhook_zero_tools_guard.rs` must continue passing unchanged. Registry entry produces identical behavior. |
| `HashSet` retry tracking has different semantics than individual booleans | Functionally identical — `contains`/`insert` maps 1:1 to `bool` check/set. HashSet is allocated once per agent loop. |

## Sources & References

- Related issues: #696 (webhook zero-tools guard), #648 (persistence eval guard), #265 (match-reason conditioning), #693, #695
- Related code: `crates/mika-agent/src/agent.rs` (guard chain), `crates/mika-agent/src/skills/matcher.rs` (MatchReason), `skills/bundled/self-dev/skill.toml`
- Related tests: `crates/mika-agent/tests/eval/test_webhook_zero_tools_guard.rs`
- Related docs: `docs/solutions/architecture-patterns/engine-guards-vs-prompt-rules-for-agent-behavior-2026-04-19.md`, `docs/solutions/prompt-enforcement-structural-guards.md`
