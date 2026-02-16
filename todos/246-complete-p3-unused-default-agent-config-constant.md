---
status: complete
priority: p3
issue_id: "246"
tags: [code-review, dead-code]
dependencies: []
---

# `DEFAULT_AGENT_CONFIG` constant defined but never used

## Problem Statement

`DEFAULT_AGENT_CONFIG` in `home.rs` is defined but never referenced. `bootstrap_agent` calls `bootstrap()` which writes `DEFAULT_CONFIG`. New agents get the full legacy config template instead of the lightweight per-agent template.

## Findings

- **Source:** Pattern Recognition Specialist
- **File:** `crates/mika-common/src/home.rs:141-145`

## Proposed Solutions

Either use `DEFAULT_AGENT_CONFIG` in `bootstrap_agent`, or remove the constant.

## Acceptance Criteria

- [ ] Constant is either used or removed

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-02-25 | Created from PR #12 code review | |
