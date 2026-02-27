---
title: "Standardize agent.rs test naming and add collision/onboarding edge case tests"
date: 2026-02-27
status: documented
category: code-review-workflow
severity: medium
module: crates/mika-agent/src/agent.rs
tags:
  - test-coverage
  - naming-convention
  - code-review
  - tool-collision-handling
  - onboarding-logic
symptoms:
  - Test functions not discoverable via `cargo test test_` prefix filtering
  - HashMap last-writer-wins behavior in build_skill_tool_map undocumented by tests
  - check_onboarding user_summary-only design lacked edge case coverage
related_issues:
  - "TODO #307 (P2): test naming convention alignment"
  - "TODO #308 (P3): skill collision edge case"
  - "TODO #309 (P3): onboarding customization edge case"
commit: f5a62ae
---

# Agent Test Quality Improvements: Naming and Coverage

## Problem Statement

Multi-agent code review (pattern-recognition-specialist, architecture-strategist) identified 3 test quality gaps in `crates/mika-agent/src/agent.rs`:

1. **Naming convention drift (P2):** All 29 test functions in agent.rs omitted the `test_` prefix used consistently across 40+ other test modules (~500+ tests). This broke grep-ability and developer expectations when filtering tests.

2. **Undocumented collision behavior (P3):** `build_skill_tool_map` (line 866) uses HashMap `collect()` which silently applies last-writer-wins when two skills define tools with the same name. No test documented this behavior, meaning future refactors changing iteration order could silently regress.

3. **Missing boundary test (P3):** `check_onboarding` (line 247) only checks the `user_summary` core memory section. Three tests covered the happy paths, but no test verified that customizing other sections (persona, key_people) does NOT flip onboarding off.

## Root Cause Analysis

Tests were added across multiple commits (f5a50e0 for initial 13, then 573596b added 16 more for tool call metadata) without following the established `test_` prefix convention. The collision and negative case tests were coverage gaps not caught during original development — they required cross-module pattern analysis to identify.

## Working Solution

### TODO #307 -- Rename 29 test functions

Mechanical rename of all 29 test functions across 6 groups:

```rust
// Before
fn loop_mode_conversation_properties() { ... }
fn build_skill_tool_map_collects_all_tools() { ... }
fn truncate_summary_no_op_for_short_strings() { ... }

// After
fn test_loop_mode_conversation_properties() { ... }
fn test_build_skill_tool_map_collects_all_tools() { ... }
fn test_truncate_summary_no_op_for_short_strings() { ... }
```

Groups renamed: LoopMode (3), check_onboarding (3), skill helpers (7), truncate_summary (5), tool_calls_metadata (3), format_tool_summary_block (5), DB metadata (3).

### TODO #308 -- Add collision test

```rust
#[test]
fn test_build_skill_tool_map_last_skill_wins_on_collision() {
    let s1 = make_skill_entry("alpha", 10, &["shared_tool"]);
    let s2 = make_skill_entry("beta", 20, &["shared_tool"]);
    let matched: Vec<&SkillEntry> = vec![&s1, &s2];
    let map = build_skill_tool_map(&matched);
    assert_eq!(map.len(), 1);
    assert_eq!(map["shared_tool"].skill_dir, PathBuf::from("/skills/beta"));
}
```

Asserts both the map size (1 key, not 2) and which skill won (beta, the last in iteration order).

### TODO #309 -- Add onboarding negative case

```rust
#[tokio::test]
async fn test_check_onboarding_true_even_when_other_sections_customized() {
    let db = test_async_db();
    db.seed_core_memory(None).await.unwrap();
    db.set_core_memory("persona", "Custom persona")
        .await
        .unwrap();
    assert!(check_onboarding(&db).await);
}
```

Documents that only `user_summary` customization indicates non-onboarding state.

### TODO file updates

Renamed files from `*-pending-*` to `*-complete-*` and updated YAML frontmatter `status: pending` to `status: complete`.

## Verification

- `cargo test -p mika-agent -- agent::tests` -- 31 tests pass (29 renamed + 2 new)
- `cargo clippy -p mika-agent` -- zero new warnings introduced
- 4-agent code review (pattern-recognition, code-simplicity, agent-native, learnings-researcher) found zero issues

## Prevention Strategies

### 1. Test naming convention enforcement

A CI check or pre-commit hook can catch `#[test]`/`#[tokio::test]` functions missing the `test_` prefix. Simplest approach: grep-based check in CI that fails on violations.

### 2. Implicit behavior documentation

When a function relies on collection semantics (HashMap ordering, Vec deduplication), add a test that explicitly documents the behavior. Name the test to describe the assumption: `test_*_last_skill_wins_on_collision` makes the contract self-documenting.

### 3. Boundary case enumeration

For boolean-returning functions, systematically test all return paths plus at least one "suspicious boundary" -- the case where a related input changes but the output should not. This catches the `check_onboarding` class of gaps.

## Cross-References

- **TODO files:** `todos/307-complete-p2-*.md`, `todos/308-complete-p3-*.md`, `todos/309-complete-p3-*.md`
- **Agent loop architecture:** `docs/solutions/refactoring/agent-loop-variant-extraction-and-deduplication.md` (LoopMode enum, test structure)
- **Skill system architecture:** `docs/solutions/architecture-decisions/filesystem-skill-registry-implementation.md` (build_skill_tool_map design)
- **Related code review:** `docs/solutions/logic-errors/update-skill-tool-discoverability-and-parity-gaps.md` (test helper patterns, validation parity)
- **Review methodology:** `docs/solutions/code-review-workflow/parallel-agent-code-review-methodology.md` (multi-agent review that discovered these issues)
