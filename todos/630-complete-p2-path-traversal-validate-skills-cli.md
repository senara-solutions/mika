---
status: complete
priority: p2
issue_id: "630"
tags: [code-review, security]
dependencies: []
---

# Path Traversal in validate_skills() CLI

## Problem Statement

`validate_skills()` in `crates/mika-cli/src/commands/skills.rs:544` joins user-supplied `--name` argument directly into a path via `skills_dir.join(name)` without calling `validate_skill_name()`. A user could pass `../../etc/passwd` as a skill name.

## Findings

- Identified by: security-sentinel (MEDIUM)
- This is a local CLI tool (not network-exposed), so the blast radius is limited to the local user
- `validate_skill_name()` already exists and is used in `create_skill` — just not called here

## Proposed Solutions

### Option A: Call validate_skill_name() before join (Recommended)
- Pros: Reuses existing validation, consistent with create_skill
- Cons: None
- Effort: Small (1 line)
- Risk: None

## Technical Details

- **Affected file:** `crates/mika-cli/src/commands/skills.rs` — `validate_skills()` function

## Acceptance Criteria

- [ ] `mika skills validate --name ../../etc/passwd` returns an error, not a path traversal

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-03-11 | Created from code review | — |
