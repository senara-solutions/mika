---
status: complete
priority: p3
issue_id: "308"
tags: [code-review, quality, testing]
dependencies: []
---

# Missing `build_skill_tool_map` collision test

## Problem Statement

When two matched skills define tools with the same name, `build_skill_tool_map` silently uses the last one (HashMap `collect()` last-writer-wins). This behavior is undocumented by any test, which means changes to iteration order or deduplication policy could silently regress.

**Why it matters:** If a future refactor changes skill iteration order or adds explicit deduplication, having no test means the behavior could change silently without anyone noticing.

## Findings

**Architecture Strategist** identified that while `inject_skills_deduplicates_tool_names` tests builtin-vs-skill deduplication, no test covers skill-vs-skill collision in `build_skill_tool_map`.

Location: `crates/mika-agent/src/agent.rs:866-872` (`build_skill_tool_map` function)

## Proposed Solutions

### Option A: Add a collision test (Recommended)
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

**Pros:** Documents existing behavior, prevents silent regression
**Cons:** Tests implementation detail (iteration order)
**Effort:** Small (5 min) | **Risk:** None

## Acceptance Criteria
- [ ] Test documents `build_skill_tool_map` collision behavior

## Work Log
### 2026-02-27 - Discovery
**By:** Claude Code (architecture-strategist review agent)
**Actions:** Identified coverage gap in skill tool map collision handling
