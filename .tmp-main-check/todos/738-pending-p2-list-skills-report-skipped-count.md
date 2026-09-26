---
status: complete
priority: p2
issue_id: 738
tags: [code-review, skills, agent-native]
---

# list_skills tool should report skipped_count

## Problem Statement

The `list_skills` agent tool only reports loaded skills but does not surface `registry.skipped_count()`. An agent calling `list_skills` cannot detect that skills were skipped due to errors (e.g., oversized prompts on always_on skills). This means an agent cannot self-diagnose a degraded skill registry.

## Findings

- Found during #331 review by agent-native-reviewer
- `list_skills` tool at `crates/mika-agent/src/tools/list_skills.rs` calls `SkillRegistry::from_dir()` but only reports loaded entries
- The `skipped_count` is available via `registry.skipped_count()` but never exposed in tool output
- Pre-existing gap, not introduced by #331

## Proposed Solutions

### Option A: Add footer line to list_skills output

When `skipped_count > 0`, append: `"\nWarning: N skill(s) skipped due to errors. Run 'mika skills validate' for details."`

- **Pros:** Minimal change, actionable for agents
- **Cons:** Only visible when agent calls list_skills
- **Effort:** Small

### Option B: Create validate_skills agent tool

Expose `validate_skill()` as an agent tool for server-mode agents.

- **Pros:** Full diagnostic access for agents
- **Cons:** Larger scope, new tool registration
- **Effort:** Medium

## Technical Details

- **Affected files:** `crates/mika-agent/src/tools/list_skills.rs`
- **Components:** Skills system, agent tools

## Acceptance Criteria

- [x] `list_skills` output includes skipped_count when > 0
- [x] Agent can detect degraded skill loading at runtime
