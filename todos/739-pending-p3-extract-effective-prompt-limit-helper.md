---
status: pending
priority: p3
issue_id: 739
tags: [code-review, skills, refactor]
---

# Extract effective_prompt_limit helper to reduce duplication

## Problem Statement

The effective prompt limit computation (`max_prompt_size.map(|v| v.min(CEILING)).unwrap_or(DEFAULT)`) appears 5+ times across `index.rs` and `mod.rs`. This is a DRY violation that increases the risk of drift if constants or logic change.

## Findings

- Found during #331 review by pattern-recognition-specialist and architecture-strategist
- Locations: scan_skills_dir, validate_skill (2x), scan_provider_variants, apply_overrides
- Pre-existing duplication, worsened by #331 adding the apply_overrides site (now uses named constants)

## Proposed Solutions

### Option A: Add method to SkillInfo

`SkillInfo::effective_prompt_limit(&self) -> u64` — single source of truth.

- **Pros:** Clean OOP pattern, eliminates all 5+ sites
- **Cons:** Couples SkillInfo to size constants
- **Effort:** Small

## Technical Details

- **Affected files:** `crates/mika-agent/src/skills/manifest.rs`, `crates/mika-agent/src/skills/index.rs`, `crates/mika-agent/src/skills/mod.rs`

## Acceptance Criteria

- [ ] Single function computes effective prompt limit
- [ ] All call sites use the shared function
