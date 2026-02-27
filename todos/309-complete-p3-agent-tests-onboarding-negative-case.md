---
status: complete
priority: p3
issue_id: "309"
tags: [code-review, quality, testing]
dependencies: []
---

# Missing `check_onboarding` negative case test

## Problem Statement

`check_onboarding` only checks the `user_summary` core memory section. If a user modifies other sections (`persona`, `key_people`, etc.) but not `user_summary`, the function still returns `true` (onboarding). No test documents this intentional behavior.

**Why it matters:** Without a test, a future developer might assume `check_onboarding` checks any section for customization and introduce a regression.

## Findings

**Architecture Strategist** identified that the three existing `check_onboarding` tests cover: no memory, default value, and customized `user_summary`. Missing: customized non-`user_summary` section still returns `true`.

Location: `crates/mika-agent/src/agent.rs:247-259` (`check_onboarding` function)

## Proposed Solutions

### Option A: Add a negative case test (Recommended)
```rust
#[tokio::test]
async fn test_check_onboarding_true_even_when_other_sections_customized() {
    let db = test_async_db();
    db.seed_core_memory(None).await.unwrap();
    db.set_core_memory("persona", "Custom persona").await.unwrap();
    assert!(check_onboarding(&db).await);
}
```

**Pros:** Documents the user_summary-only design, prevents misunderstanding
**Cons:** None
**Effort:** Small (5 min) | **Risk:** None

## Acceptance Criteria
- [ ] Test documents that `check_onboarding` only checks `user_summary`

## Work Log
### 2026-02-27 - Discovery
**By:** Claude Code (architecture-strategist review agent)
**Actions:** Identified missing boundary test for onboarding detection
