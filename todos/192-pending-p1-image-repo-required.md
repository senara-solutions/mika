---
status: pending
priority: p1
issue_id: "192"
tags: [code-review, correctness, operational]
dependencies: []
---

# MIKA_IMAGE_REPO Should Be Required, Not Optional

## Problem Statement
`provision.sh` lists `MIKA_IMAGE_REPO` as optional and defaults it to empty: `--set image.repository="${MIKA_IMAGE_REPO:-}"`. An empty repository produces image reference `:latest` (no repo prefix), causing an ImagePullBackOff error. While `helm install --wait` will eventually fail and trigger rollback, the error message is about pod scheduling, not the actual root cause (missing repo).

## Findings
- **Architecture strategist**: Medium severity operational risk
- Location: `scripts/provision.sh` line 149 (`--set image.repository="${MIKA_IMAGE_REPO:-}"`)
- The usage text at line 26 lists it as "Optional" which is misleading

## Proposed Solutions

### Option 1: Make it required (Recommended)
Add `MIKA_IMAGE_REPO` to the required env var validation section.

```bash
: "${MIKA_IMAGE_REPO:?MIKA_IMAGE_REPO is required}"
```

Move it from "Optional" to "Required" in the usage text.

- **Pros**: Fails fast with clear error message
- **Cons**: None
- **Effort**: Small (5 minutes)
- **Risk**: Low

## Technical Details
- **Affected Files**: `scripts/provision.sh`

## Acceptance Criteria
- [ ] `MIKA_IMAGE_REPO` validated as required before any action
- [ ] Usage text lists it under "Required environment variables"
- [ ] Running without MIKA_IMAGE_REPO produces clear error message

## Work Log
### 2026-02-24 - Found during code review
**By:** Architecture strategist
