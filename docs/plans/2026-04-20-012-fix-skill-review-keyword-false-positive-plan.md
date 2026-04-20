---
title: "fix: Remove keyword triggers from skill-review to prevent false positive activation"
type: fix
status: active
date: 2026-04-20
---

# fix: Remove keyword triggers from skill-review to prevent false positive activation

## Overview

Remove all keyword triggers from the `skill-review` skill manifest, making it dependency-only and explicit-invocation-only. This prevents false positive activation when PR titles or bodies contain phrases like "review skill" in meta-discussion rather than as a user intent to invoke the skill.

## Problem Frame

`skill-review` has keywords `["review skill", "adapt skill", "generate variant", "tune prompt", "skill variant"]` in its `skill.toml`. The `match_skills()` function in `matcher.rs` uses pure substring matching (`message_lower.contains(kw)`). When a GitHub webhook delivers a PR event whose body discusses the skill-review feature, phrases like "review skill" trigger keyword matching, loading skill-review's full system prompt and enforcing `required_tools = ["review_skill"]` — even though no one asked for a skill review.

PR #574's `review_filter` only prevents *other* skills from co-activating when skill-review is already matched. It does not prevent skill-review itself from falsely triggering. This is a known class of bug (see `project_keyword_substring_false_positives.md`).

## Requirements Trace

- R1. A PR whose body contains "skill-review" must NOT cause `skill-review` to load during mika-qa's review turn
- R2. Explicit invocation via dependency loading from another skill must still work
- R3. Direct user invocation (`review_skill` tool call) must still work — the tool remains registered
- R4. Unit test covers the false-positive case

## Scope Boundaries

- Only modifying the `skill-review` manifest — no changes to the keyword matching engine itself
- Not adding skill-review as a dependency to any downstream skill (no current skills need it as a dependency — verified via grep across both `skills/bundled/` and `mika-skills/`)
- Not addressing the general substring matching problem (tracked separately in memory)

## Context & Research

### Relevant Code and Patterns

- `skills/bundled/skill-review/skill.toml` — manifest with keyword triggers to remove
- `crates/mika-agent/src/skills/matcher.rs` — `match_skills()` with substring matching, existing test suite
- `crates/mika-agent/src/skills/review_filter.rs` — post-match filter (not affected by this change)
- `crates/mika-agent/src/skills/mod.rs` — `SkillRegistry::match_message()` entry point

### Institutional Learnings

- `project_keyword_substring_false_positives.md` — documents this exact class of bug with substring matching
- `docs/solutions/architecture-patterns/review-target-exclusion-filter.md` — documents the existing filter that only handles the inverse problem
- `docs/solutions/architecture-patterns/conditional-required-tools-enforcement-via-match-reason.md` — confirms constraints (required_tools) only apply to `MatchReason::Keyword` matches, so removing keywords eliminates false constraint enforcement

## Key Technical Decisions

- **Remove keywords entirely (Option 1):** Cleanest fix. skill-review's job is to rewrite other skills' prompts — keyword matching on a meta-skill that operates on skills themselves is inherently dangerous. Any keyword containing "skill" or "review" will false-match on meta-discussion. Dependency-based and direct tool invocation remain available.
- **No downstream dependency additions needed:** No existing skill declares skill-review as a dependency, and none currently need to. If a future skill needs variant generation, it can add `dependencies = ["skill-review"]` to its own manifest.

## Implementation Units

- [ ] **Unit 1: Remove keywords from skill-review manifest**

**Goal:** Eliminate false positive keyword activation by clearing the keywords list.

**Requirements:** R1, R2, R3

**Dependencies:** None

**Files:**
- Modify: `skills/bundled/skill-review/skill.toml`

**Approach:**
- Set `keywords = []` in the `[triggers]` section
- Keep `always_on = false` — the skill loads only when pulled in as a dependency or when the user directly calls `review_skill`
- Keep `required_tools = ["review_skill"]` — this constraint only fires for `MatchReason::Keyword`, so with no keywords it never fires, which is correct

**Patterns to follow:**
- Other dependency-only skills that have `keywords = []` or an empty triggers section

**Test expectation:** none — manifest-only change; behavioral coverage in Unit 2

**Verification:**
- `skill.toml` has `keywords = []`
- `cargo build` succeeds (build.rs re-discovers the manifest)

- [ ] **Unit 2: Add unit test for false-positive prevention**

**Goal:** Verify that a message containing "review skill" as meta-discussion does not keyword-match skill-review.

**Requirements:** R1, R4

**Dependencies:** Unit 1

**Files:**
- Modify: `crates/mika-agent/src/skills/matcher.rs` (test module)

**Approach:**
- Add a test that creates a skill-review entry with `keywords = []` and verifies it does NOT match a message like `"[GitHub] PR opened: skill-review fires on PRs that discuss skill-review"` or `"the review skill feature is broken"`
- Also verify it does NOT match via always_on (since `always_on = false`)
- The test confirms the manifest change achieves the desired behavior at the matcher level

**Patterns to follow:**
- Existing `test_no_match` test in `matcher.rs` for structure
- `make_entry()` helper for creating test skill entries

**Test scenarios:**
- Happy path: skill-review with empty keywords does not match a message containing "review skill" or "skill-review"
- Happy path: skill-review with empty keywords IS included when pulled in as a dependency of another matched skill
- Edge case: message that would have matched old keywords ("adapt skill for claude") does not trigger skill-review

**Verification:**
- `cargo test -p mika-agent -- matcher::tests` passes with new tests

## System-Wide Impact

- **Interaction graph:** No downstream skills depend on skill-review, so removing keywords has no cascade effect. The `review_filter` in `review_filter.rs` only activates when skill-review is keyword-matched — with no keywords, it becomes dormant (correct behavior, no dead code concern since the filter is a safety net).
- **Error propagation:** No change — `required_tools` enforcement for skill-review becomes unreachable (only fires on `MatchReason::Keyword`), which is the desired outcome.
- **Unchanged invariants:** The `review_skill` tool itself remains registered and callable. The skill's system prompt and templates remain intact for dependency-based loading.

## Risks & Dependencies

| Risk | Mitigation |
|------|------------|
| Future skills may need skill-review via keyword | Any skill needing variant generation can declare `dependencies = ["skill-review"]` — this is the correct activation path for a meta-skill |
| Direct user invocation harder without keywords | Users can still call `review_skill` tool directly; the tool is always registered. If needed, a future PR can add a CLI command or explicit invocation path |

## Sources & References

- Related issue: #576
- Related PRs: #573, #574 (review_filter, does not fix this)
- Memory: `project_keyword_substring_false_positives.md`
- Solution doc: `docs/solutions/architecture-patterns/review-target-exclusion-filter.md`
