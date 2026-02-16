---
status: pending
priority: p3
issue_id: "019"
tags: [code-review, quality]
dependencies: []
---

# ~259 LOC Dead Code Removable

## Problem Statement

Multiple unused functions, classes, and imports across the codebase add cognitive load and maintenance burden.

## Findings

- **Source:** Code Simplicity Reviewer
- **Items to remove:**
  - `get_opus()` in `app/agent/llm.py` (unused model getter)
  - `sync_session_factory` in `app/common/db.py` (unused sync engine)
  - `RelationType` enum in `app/memory/models.py` (unused)
  - `MERGE_KEYS` dict in `app/memory/models.py` (unused)
  - `AuditLog` model in `app/models/audit.py` (never populated)
  - Unused repository functions
  - `send_template()` in WhatsApp adapter (never called)

## Proposed Solutions

### Option A: Remove all dead code (Recommended)
- Delete unused functions, classes, imports
- **Effort:** Small | **Risk:** Low

## Acceptance Criteria

- [ ] All identified dead code removed
- [ ] All tests still pass
- [ ] No broken imports

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-02-16 | Created from code review | |
