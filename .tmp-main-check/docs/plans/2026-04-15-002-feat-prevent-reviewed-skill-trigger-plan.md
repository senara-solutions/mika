---
title: "feat: Prevent reviewed skill from being triggered during its own review"
type: feat
status: active
date: 2026-04-15
---

# feat: Prevent reviewed skill from being triggered during its own review

## Overview

When the `skill-review` skill is keyword-triggered (e.g., "review skill self-dev"), the skill being reviewed (e.g., `self-dev`) must be excluded from the matched skill set before prompt injection. This prevents circular activation where the reviewed skill's prompt contaminates the review context.

## Problem Frame

During a `review_skill` turn, the user message (e.g., "review skill self-dev") may contain keywords that match the target skill. If `self-dev` has keywords like "dev" or "self-dev" that appear in the message, it gets keyword-matched alongside `skill-review`. Both prompts are then injected into the system prompt, meaning the reviewer is contaminated by the reviewed skill's instructions — the reviewer becomes the reviewed.

This is a structural safety issue. The fix must be code-level enforcement, not prompt-level guards (per institutional learning: prompt enforcement fails when the LLM controls the critical path).

## Requirements Trace

- R1. When `skill-review` is keyword-matched, exclude the target skill from `match_skills()` results before prompt injection
- R2. Applies to all skills — not just trust-critical ones
- R3. Exclusion is turn-scoped (lasts for the single agent turn, not the entire session)
- R4. The exclusion must work in both conversation mode and team mode paths

## Scope Boundaries

- Only excludes skills that are identified as the review target — other keyword-matched skills remain
- Does not modify `match_skills()` internals — uses post-match filtering (existing pattern)
- Does not change `ToolContext` — the exclusion is applied before the LLM turn, not during tool execution
- `always_on` skills are never excluded by this mechanism (they don't inject via keyword match anyway, and removing them would break unrelated functionality)

## Context & Research

### Relevant Code and Patterns

- `crates/mika-agent/src/agent.rs:1269-1283` — Conversation mode: `match_message()` → context exclusion pattern
- `crates/mika-agent/src/agent.rs:2401-2414` — Team mode: identical pipeline
- `crates/mika-agent/src/skills/matcher.rs:37-88` — `match_skills()` function with `MatchReason` enum
- `crates/mika-agent/src/skills/mod.rs:180-182` — `SkillRegistry::match_message()`
- `crates/mika-agent/src/skills/builtin_handlers.rs:934-1032` — `review_skill` handler
- `crates/mika-agent/templates/skills/skill-review/skill.toml` — Keywords: `["review skill", "adapt skill", "generate variant", "tune prompt", "skill variant"]`

### Institutional Learnings

- **Deterministic skill context injection** (`docs/solutions/architecture-patterns/deterministic-skill-context-injection.md`): Pre-LLM pipeline ordering is `match_skills() → resolve_contexts() → exclude failed → resolve_skill_llm_override() → collect_required_tools() → inject_skills_and_resolve_tools()`. New filtering must slot in correctly.
- **Conditional required_tools enforcement** (`docs/solutions/architecture-patterns/conditional-required-tools-enforcement-via-match-reason.md`): Established pattern of filtering `MatchedSkill` by `MatchReason` at call sites.
- **Trust-critical skill tier** (`docs/solutions/architecture-patterns/trust-critical-skill-tier-and-template-sync.md`): `skill-review` is trust-critical; other skills are reviewable.
- **Prompt enforcement fails** (`docs/solutions/prompt-engineering/2026-04-10-harden-skill-review-prompt-enforcement.md`): Code-level guards required for reliable prevention.

## Key Technical Decisions

- **Post-match filtering (not matcher modification):** Follow the existing `context_exclude` pattern at lines 1280-1283. This keeps `match_skills()` pure (input → output, no side effects) and localizes the concern to the agent loop call site. The alternative of adding an `exclude` parameter to `match_skills()` would require threading review state through `SkillRegistry::match_message()`, which couples matching to review awareness unnecessarily.

- **Skill-name-in-message heuristic for target detection:** When `skill-review` is keyword-matched, scan the user message for names of other keyword-matched skills. Any skill whose name (case-insensitive) appears as a word in the user message is the review target and gets excluded. This is reliable because: (a) the user must name the skill to review it, (b) we only exclude from the *already keyword-matched* set (not all skills), and (c) we only check skills matched via `Keyword` reason (not `AlwaysOn` or `Dependency`).

- **Dedicated helper function:** Extract the filtering logic into a standalone `fn exclude_review_targets(matched, user_message) -> Vec<usize>` in `agent.rs` (or a new `skills/review_filter.rs`). This keeps the agent loop clean and makes the logic independently testable.

## Open Questions

### Resolved During Planning

- **Where to place the exclusion in the pipeline?** Right after `match_message()` and before `resolve_contexts()`. The excluded skill should not participate in context resolution, LLM override, or prompt injection. This is the earliest safe point.

- **Should `AlwaysOn` skills be excludable?** No. `AlwaysOn` skills don't inject via keyword context — they provide baseline capabilities. Excluding them would break unrelated functionality. Only `Keyword`-matched skills are candidates for exclusion.

- **What if the skill name is a substring of a common word?** The heuristic matches skill names against the user message. Skill names are typically multi-word or hyphenated (e.g., `self-dev`, `web-search`), making false positives unlikely. The `name-in-keywords` rejection (#510) already prevents skills from having names that are too generic. Additionally, we only exclude from skills that already keyword-matched — the intersection of "keyword-matched AND name-in-message" is a strong signal.

### Deferred to Implementation

- **Exact word-boundary matching strategy:** Whether to use simple `contains()` or a word-boundary-aware check. Implementation should test both and pick the simplest that passes all test scenarios.

## High-Level Technical Design

> *This illustrates the intended approach and is directional guidance for review, not implementation specification. The implementing agent should treat it as context, not code to reproduce.*

```
User message: "review skill self-dev"
                    │
                    ▼
        match_skills(skills, message)
                    │
                    ▼
    matched = [skill-review (Keyword), self-dev (Keyword)]
                    │
                    ▼
    detect skill-review in matched (Keyword reason)
         ├── YES → extract review targets from message
         │         exclude targets from matched
         │         matched = [skill-review (Keyword)]
         └── NO  → no filtering
                    │
                    ▼
    resolve_contexts() → ... → inject_skills_and_resolve_tools()
```

## Implementation Units

- [ ] **Unit 1: Add `exclude_review_targets` helper function**

**Goal:** Create a function that, given the matched skill set and user message, returns indices of skills that should be excluded because they are review targets.

**Requirements:** R1, R2, R3

**Dependencies:** None

**Files:**
- Create: `crates/mika-agent/src/skills/review_filter.rs`
- Modify: `crates/mika-agent/src/skills/mod.rs` (add `pub mod review_filter;`)
- Test: `crates/mika-agent/src/skills/review_filter.rs` (inline `#[cfg(test)] mod tests`)

**Approach:**
- The function accepts `&[MatchedSkill]` and `&str` (user message)
- First, check if any matched skill named `"skill-review"` has `MatchReason::Keyword`. If not, return empty vec (no exclusion needed)
- If skill-review is keyword-matched, iterate remaining keyword-matched skills. For each, check if the skill's name appears in the lowercased user message (case-insensitive substring match). Collect indices of matches.
- Return the indices in reverse order (for safe removal via `matched.remove(idx)`)

**Patterns to follow:**
- `context::resolve_contexts()` returning `context_exclude: Vec<usize>` indices
- `matcher.rs` test helper `make_entry()` for constructing test skill entries

**Test scenarios:**
- Happy path: message "review skill self-dev" with skill-review (Keyword) and self-dev (Keyword) matched → self-dev excluded
- Happy path: message "review skill web-search" with skill-review (Keyword) and web-search (Keyword) → web-search excluded
- Edge case: skill-review matched but no other keyword-matched skills → empty exclusion
- Edge case: skill-review matched via AlwaysOn (hypothetical) → no exclusion (only triggers on Keyword match)
- Edge case: target skill matched via AlwaysOn → not excluded (only Keyword-matched targets are candidates)
- Edge case: target skill matched via Dependency → not excluded
- Edge case: multiple skills keyword-matched alongside skill-review, only one name appears in message → only that one excluded
- Edge case: skill-review not in matched set → no exclusion
- Edge case: batch mode message "review skill *" → no skill name matches (asterisk is not a skill name)

**Verification:**
- All test scenarios pass
- `cargo clippy` clean

- [ ] **Unit 2: Integrate exclusion into conversation and team mode pipelines**

**Goal:** Wire `exclude_review_targets` into both agent loop paths (conversation mode and team mode), inserting the filtering step right after `match_message()` and before `resolve_contexts()`.

**Requirements:** R1, R3, R4

**Dependencies:** Unit 1

**Files:**
- Modify: `crates/mika-agent/src/agent.rs`
- Test: `crates/mika-agent/src/agent.rs` (inline tests) and `crates/mika-agent/tests/eval/` (integration test)

**Approach:**
- In conversation mode (after line 1269): call `exclude_review_targets(&matched, params.user_message)` and remove returned indices from `matched` before the context resolution step
- In team mode (after line 2401): identical pattern with `params.task_message`
- Add a `tracing::debug!` log when skills are excluded, naming the excluded skills
- The pattern mirrors the existing `context_exclude` removal loop at lines 1280-1283

**Patterns to follow:**
- Lines 1280-1283: `for &idx in context_exclude.iter().rev() { matched.remove(idx); }` — exact same pattern
- Lines 2411-2413: Team mode equivalent

**Test scenarios:**
- Integration: agent turn with "review skill self-dev" message where self-dev has a keyword matching → self-dev prompt NOT in system prompt, skill-review prompt IS in system prompt
- Integration: agent turn with normal message (no skill-review trigger) → all matching skills injected as usual (no regression)
- Happy path: conversation mode excludes review target
- Happy path: team mode excludes review target
- Edge case: review target has context requirements → context resolution not called for excluded skill

**Verification:**
- `cargo test` passes (no regressions)
- `cargo clippy` clean
- Manual or eval test confirms reviewed skill prompt is absent from system prompt during review turns

## System-Wide Impact

- **Interaction graph:** The filtering step runs between `match_message()` and `resolve_contexts()`. It does not affect silent mode (which uses `safe_always_on_skills()` / `callback_safe_skills()` — no keyword matching). It does not affect the `review_skill` tool handler itself.
- **Error propagation:** No new error paths. The function returns indices; if empty, nothing changes.
- **State lifecycle risks:** None. The exclusion is computed per-turn from immutable inputs (matched set + user message). No persistent state involved.
- **API surface parity:** Silent mode does not keyword-match, so no changes needed there.
- **Integration coverage:** The eval harness (`EvalHarness` + `MockLlmProvider`) can verify that the system prompt does not contain the excluded skill's prompt snippet.
- **Unchanged invariants:** `match_skills()` signature and behavior unchanged. `ToolContext` unchanged. `review_skill` handler unchanged. `SkillRegistry` API unchanged.

## Risks & Dependencies

| Risk | Mitigation |
|------|------------|
| Skill name substring false positives (e.g., skill "dev" matching in "review skill self-dev") | Unlikely in practice due to name-in-keywords rejection (#510) preventing generic names. Word-boundary matching can further reduce risk. |
| Batch mode "review skill *" accidentally excluding skills | Asterisk is not a valid skill name, so no skill name will match "*" in the message |
| Future paths that call `match_message()` without the filter | The filter is applied at the call site, not inside `match_message()`. New call sites must add it explicitly. A code comment at the filter call site documents this coupling. |

## Sources & References

- Related issues: #513, #512 (parent), #510 (name-in-keywords), #265 (match reason conditioning)
- Related code: `crates/mika-agent/src/skills/matcher.rs`, `crates/mika-agent/src/agent.rs`
- Institutional learnings: `docs/solutions/architecture-patterns/deterministic-skill-context-injection.md`, `docs/solutions/architecture-patterns/conditional-required-tools-enforcement-via-match-reason.md`
