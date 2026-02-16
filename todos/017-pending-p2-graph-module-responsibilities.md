---
status: pending
priority: p2
issue_id: "017"
tags: [code-review, architecture]
dependencies: []
---

# graph.py Has Too Many Responsibilities

## Problem Statement

`app/agent/graph.py` handles LangGraph graph construction, tool definitions, prompt templates, state management, and orchestration — all in one file. This makes it difficult to test, modify, and understand.

## Findings

- **Source:** Architecture Strategist, Pattern Recognition
- **Location:** `app/agent/graph.py`

## Proposed Solutions

### Option A: Extract into focused modules (Recommended)
- `app/agent/tools.py` — tool definitions
- `app/agent/prompts.py` — prompt templates
- `app/agent/state.py` — state schema
- `app/agent/graph.py` — just graph construction/wiring
- **Effort:** Medium | **Risk:** Low

## Acceptance Criteria

- [ ] Each module has a single responsibility
- [ ] Imports are clean between modules
- [ ] All agent tests still pass

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-02-16 | Created from code review | |
