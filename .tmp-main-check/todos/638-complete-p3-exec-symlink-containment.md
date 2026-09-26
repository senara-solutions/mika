---
status: complete
priority: p3
issue_id: "638"
tags: [code-review, security]
dependencies: []
---

# Exec Command Symlink Follows Without Containment Check

## Problem Statement

`validate_skill()` checks exec command existence and permissions but follows symlinks without verifying the target stays within the skill directory.

## Findings

- Identified by: security-sentinel (LOW)
- Local CLI context — low blast radius

## Proposed Solutions

### Option A: Add canonicalize + containment check
- Effort: Small
- Risk: None

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-03-11 | Created from code review | — |
