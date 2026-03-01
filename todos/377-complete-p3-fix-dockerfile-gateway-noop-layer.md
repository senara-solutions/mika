---
status: complete
priority: p3
issue_id: 377
tags: [code-review, docker, simplicity]
dependencies: []
---

# Replace Dockerfile.gateway no-op RUN layer with comment

## Problem Statement

`Dockerfile.gateway` has `RUN echo "No build dependencies required"` which creates an unnecessary Docker layer that does nothing. This was introduced when OpenSSL deps were removed. It should be a comment instead.

## Findings

- **Source:** Code Simplicity Reviewer + Architecture Strategist review agents
- **Severity:** LOW — harmless but adds unnecessary layer to image

## Proposed Solutions

### Option 1: Replace with comment (Recommended)
- Change `RUN echo "No build dependencies required"` to `# No native deps needed: uses rustls (no OpenSSL), no rusqlite`
- **Effort:** Small
- **Risk:** Low

## Technical Details

- **Affected files:** `Dockerfile.gateway`

## Acceptance Criteria

- [ ] No-op RUN layer replaced with a comment
- [ ] Docker image builds successfully
