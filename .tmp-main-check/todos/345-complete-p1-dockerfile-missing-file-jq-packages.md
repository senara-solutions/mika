---
status: complete
priority: p1
issue_id: 345
tags: [code-review, deployment, docker]
dependencies: []
---

# Dockerfile.agent Missing `file` and `jq` Packages

## Problem Statement

The `read.sh` handler for the file-reader skill uses `file -b --mime-type` (for MIME detection) and `jq` (for safe JSON construction of the `__mika_v1` envelope). These commands work in development but are NOT installed in the `Dockerfile.agent` runtime stage, which only installs `ca-certificates` and `wget`.

This means the multimodal image pipeline will silently fail in production Docker containers.

## Findings

- **Source:** agent-native-reviewer
- **Location:** `Dockerfile.agent:39-40` (runtime apt-get install)
- **Evidence:** `read.sh` lines 20 and 24 use `file` and `jq`

## Proposed Solutions

### Option A: Add packages to runtime stage (Recommended)
Add `file` and `jq` to the existing apt-get install line.
- Pros: Simple, direct fix. ~2MB image size increase.
- Cons: Adds dependencies to production image.
- Effort: Small
- Risk: Low

## Acceptance Criteria

- [x] `Dockerfile.agent` runtime stage installs `file` and `jq`
- [ ] Docker image builds successfully
- [ ] `read.sh` MIME detection works inside container

## Work Log

| Date | Action | Result |
|------|--------|--------|
| 2026-02-28 | Identified during code review | Pending |
| 2026-02-28 | Added `file` and `jq` to Dockerfile.agent runtime stage | Complete |
