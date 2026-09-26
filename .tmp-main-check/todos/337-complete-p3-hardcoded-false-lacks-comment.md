---
status: complete
priority: p3
issue_id: "337"
tags: [code-review, quality]
dependencies: []
---

# Hardcoded `false` in agents create Lacks Explanatory Comment

## Problem Statement

In `agents create`, `seed_bundled_skills_if_needed(&agent_home, false)` hardcodes `false` instead of reading from `Settings`. This is intentionally correct — explicit agent creation should always seed skills. However, the "why" is not documented at the call site, which could confuse future maintainers.

## Findings

- Flagged by: pattern-recognition-specialist
- Location: `crates/mika-cli/src/commands/agents.rs:57`
- The existing comment on line 55 says "Seed bundled skills into the new agent's skills directory" but doesn't explain why the config flag is bypassed

## Proposed Solutions

### Option A: Add inline comment
```rust
// Always seed on explicit creation, regardless of disable_bundled_skills config
mika_agent::startup::seed_bundled_skills_if_needed(&agent_home, false);
```
- **Effort:** Small
- **Risk:** None

## Acceptance Criteria

- [ ] Comment explains why `false` is hardcoded instead of reading from settings
