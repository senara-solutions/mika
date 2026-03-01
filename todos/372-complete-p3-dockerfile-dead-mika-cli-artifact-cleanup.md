---
status: complete
priority: p3
issue_id: 372
tags: [code-review, docker, cleanup]
dependencies: []
---

# Dockerfile.agent has dead mika-cli artifact cleanup line

## Problem Statement

`Dockerfile.agent` line 26 runs `rm -f target/release/mika-cli` but the binary is defined as `name = "mika"` in `[[bin]]`. The artifact `target/release/mika-cli` has never existed — this is a no-op cleanup line that is misleading.

## Proposed Solutions

### Option 1: Fix the artifact name
- Change `mika-cli` to `mika` in the rm line (if the mika binary should be excluded from the agent image)
- Or remove the line entirely if it's not needed
- **Effort:** Small
- **Risk:** Low

## Technical Details

- **Affected files:** `Dockerfile.agent:26`

## Acceptance Criteria

- [ ] Dockerfile cleanup line references correct binary artifact names

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-03-01 | Created from code review of commit 2eca502 | Pre-existing issue flagged by pattern and architecture reviewers |
