---
status: complete
priority: p2
issue_id: "657"
tags:
  - code-review
  - agent-native
  - skills
dependencies: []
---

# create_skill and update_skill tools lack dependencies parameter

## Problem Statement

The `create_skill` tool hardcodes `dependencies: vec![]` and the `update_skill` tool does not expose a `dependencies` property in its `input_schema`. Users can declare `dependencies = ["tmux"]` in `skill.toml` by hand, but the agent cannot do the same via its tools. This is an agent-native parity gap (Orphan Feature anti-pattern).

## Findings

- **Source**: agent-native-reviewer
- **Evidence**: `create_skill.rs:218` hardcodes `dependencies: vec![]`; `update_skill.rs` input_schema lists `description`, `keywords`, `system_prompt`, `always_on` but not `dependencies`
- **Impact**: Agent cannot create or modify skill dependencies programmatically

## Proposed Solutions

### Option A: Add dependencies param to both tools
- Add `"dependencies"` as optional array-of-strings to both `create_skill` and `update_skill` input schemas
- Validate each entry with existing `validate_skill_name()`, cap at 10-20
- **Pros**: Full agent-native parity
- **Cons**: Slightly more tool complexity
- **Effort**: Small
- **Risk**: Low

## Recommended Action

_To be filled during triage_

## Technical Details

- **Affected files**: `crates/mika-agent/src/tools/create_skill.rs`, `crates/mika-agent/src/tools/update_skill.rs`

## Acceptance Criteria

- [ ] `create_skill` accepts optional `dependencies` array parameter
- [ ] `update_skill` accepts optional `dependencies` array parameter
- [ ] Dependency names validated (non-empty, reasonable length, capped count)
- [ ] Tests cover creating/updating skills with dependencies

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-03-13 | Created from code review of PR #134 | Agent-native reviewer identified Orphan Feature pattern |

## Resources

- PR: #134
- Related issue: #134
