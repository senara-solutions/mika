---
status: pending
priority: p3
issue_id: "592"
tags: [code-review, security]
dependencies: []
---

# Dashboard .gitignore Missing .env

## Problem Statement
The dashboard `.gitignore` only covers `node_modules`, `dist`, and `*.local`. A plain `.env` file with `VITE_MIKA_TOKEN=...` would be tracked by git.

## Findings
- **Source:** Security Sentinel
- **Location:** `dashboard/.gitignore`

## Proposed Solutions
Add `.env` and `.env.*` (excluding `.env.example`) to dashboard `.gitignore`.

## Acceptance Criteria
- [ ] `.env` files are gitignored in dashboard/

## Work Log
| Date | Action | Learnings |
|------|--------|-----------|
| 2026-03-08 | Created from code review | Security Sentinel flagged missing .env ignore |

## Resources
- PR #89
