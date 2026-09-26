---
status: complete
priority: p2
issue_id: "632"
tags: [code-review, architecture]
dependencies: []
---

# Server/Team/Delegate Paths Missing skipped_count Warning

## Problem Statement

The `skipped_count` startup warning was added to `ask.rs` and `chat.rs` but not to:
- Server startup (`server/mod.rs`)
- Team engine (`teams/engine.rs`)
- Delegate task (`delegate_task.rs`)
- Chat reload/hot-reload paths

## Findings

- Identified by: architecture-strategist, pattern-recognition-specialist, agent-native-reviewer
- Consider moving the warning into `SkillRegistry::from_dir()` to cover all call sites automatically

## Proposed Solutions

### Option A: Move warning to SkillRegistry::from_dir() (Recommended)
- Pros: Single location, covers all current and future call sites
- Cons: Couples logging to the registry constructor
- Effort: Small
- Risk: Low

### Option B: Add warning to each missing call site
- Pros: Explicit per-site control
- Cons: Easy to miss new call sites
- Effort: Small
- Risk: Low

## Acceptance Criteria

- [ ] Server startup logs a warning when skills are skipped
- [ ] All paths that load skills emit the warning

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-03-11 | Created from code review | — |
