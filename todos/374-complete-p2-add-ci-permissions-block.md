---
status: complete
priority: p2
issue_id: 374
tags: [code-review, security, ci-cd]
dependencies: []
---

# Add explicit permissions block to CI workflow

## Problem Statement

The CI workflow (`.github/workflows/ci.yml`) lacks an explicit `permissions` block. Without one, the workflow inherits the repository's default permissions, which may be overly broad. Explicit least-privilege permissions reduce blast radius if the workflow is compromised.

## Findings

- **Source:** Security Sentinel review agent
- **Severity:** MEDIUM — principle of least privilege violation
- **Note:** The release-plz and release workflows already have explicit permissions blocks

## Proposed Solutions

### Option 1: Add read-only contents permission (Recommended)
- Add `permissions: { contents: read }` at the workflow level
- CI only needs to read code — no write access needed
- **Effort:** Small
- **Risk:** Low

## Technical Details

- **Affected files:** `.github/workflows/ci.yml`

## Acceptance Criteria

- [ ] CI workflow has explicit `permissions: contents: read` block
- [ ] CI workflow still runs successfully with restricted permissions
