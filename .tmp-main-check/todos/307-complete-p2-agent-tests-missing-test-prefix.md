---
status: complete
priority: p2
issue_id: "307"
tags: [code-review, quality, testing]
dependencies: []
---

# Agent tests missing `test_` prefix naming convention

## Problem Statement

All 13 unit tests added to `crates/mika-agent/src/agent.rs` omit the `test_` prefix that is used consistently by every other test module in the codebase (~330+ tests across 40+ modules). This breaks naming convention consistency.

**Why it matters:** Inconsistent naming makes it harder to grep for tests, breaks developer expectations, and could cause confusion about which functions are tests vs helpers when scanning code.

## Findings

**Pattern Recognition Specialist** identified that every other test module in `mika-agent` prefixes test functions with `test_`:
- `db.rs`: `test_migration_creates_tables`, `test_conversation_roundtrip`
- `search_memory.rs`: `test_search_finds_person`, `test_search_no_results`
- `cancel_reminder.rs`: `test_cancel_reminder_success`, `test_cancel_reminder_not_found`
- `skills/index.rs`: `test_scan_valid_skills`, `test_disabled_skill`
- `skills/manifest.rs`: `test_parse_new_format`, `test_parse_always_on`
- `skills/matcher.rs`: `test_always_on_included_regardless`, `test_keyword_match`

Current agent.rs test names (missing `test_` prefix):
- `loop_mode_conversation_properties` → should be `test_loop_mode_conversation_properties`
- `check_onboarding_true_when_no_core_memory` → should be `test_check_onboarding_true_when_no_core_memory`
- `build_skill_tool_map_collects_all_tools` → should be `test_build_skill_tool_map_collects_all_tools`
- (and 10 more)

## Proposed Solutions

### Option A: Rename all 13 test functions (Recommended)
Add `test_` prefix to all 13 test function names.

**Pros:** Restores full consistency with codebase convention
**Cons:** Trivial churn
**Effort:** Small (5 min) | **Risk:** None

## Acceptance Criteria
- [ ] All 13 test functions in agent.rs have `test_` prefix
- [ ] All tests still pass

## Work Log
### 2026-02-27 - Discovery
**By:** Claude Code (pattern-recognition-specialist review agent)
**Actions:** Compared test naming across 40+ test modules in mika-agent crate
