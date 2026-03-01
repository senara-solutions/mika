---
status: pending
priority: p2
issue_id: "363"
tags: [code-review, documentation, gateway]
dependencies: []
---

# Stale Gateway Documentation

## Problem Statement

Three documentation files still describe mika-gateway as living in the private `mika-cloud` repo. Now that the gateway source has moved to `crates/mika-gateway/` in this repo, these references are incorrect and will mislead developers.

## Findings

- `docs/architecture.md` — references gateway in mika-cloud
- `docs/deployment.md` — references gateway in mika-cloud
- `CLAUDE.md` — references gateway in mika-cloud (mika-gateway section under Reference Repositories)

## Proposed Solutions

### Option A: Update all references (Recommended)
- Update all three files to reflect the new gateway location
- Effort: Small
- Risk: None

## Technical Details

**Affected files:**
- `docs/architecture.md`
- `docs/deployment.md`
- `CLAUDE.md`

## Acceptance Criteria

- [ ] All references to gateway in mika-cloud are updated to crates/mika-gateway/
- [ ] Helm charts and provisioning scripts correctly noted as remaining in mika-cloud
