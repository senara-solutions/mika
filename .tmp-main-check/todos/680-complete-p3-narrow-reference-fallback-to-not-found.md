---
status: complete
priority: p3
issue_id: "680"
tags: [code-review, security, quality]
dependencies: []
---

# Reference workspace fallback should only trigger on "not found"

## Problem Statement

The `read_workspace` fallback logic triggers on **any** error from the current workspace (symlink rejection, file too large, etc.), not just "file not found". This could mask security violations by silently serving content from the reference workspace instead.

## Findings

- **Security Sentinel**: Low severity. No privilege escalation since reference dir has same security checks, but an agent could receive stale data without knowing the current workspace had a security issue.
- **Code Simplicity**: Notes the `!output.is_error` proxy is imprecise.

**Affected file:** `crates/mika-agent/src/tools/read_workspace.rs` (lines ~118-137)

## Proposed Solutions

Check for "not found" or "does not exist" in the error message before falling back, or introduce an error enum to distinguish not-found from security-violation errors.

- **Effort:** Small
- **Risk:** Low

## Acceptance Criteria

- [ ] Fallback only triggers on genuine "not found" errors
- [ ] Security errors (symlink, traversal) are returned without fallback

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-03-15 | Created from code review | Security sentinel + code simplicity flagged |
