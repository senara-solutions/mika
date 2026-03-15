---
status: complete
priority: p2
issue_id: "674"
tags: [code-review, security, defense-in-depth]
dependencies: []
---

# run-id UUID format validation before filesystem path use

## Problem Statement

The `--run-id` CLI argument is used directly in `workspace_run_dir()` via `PathBuf::join(run_id)` without format validation. While the database lookup acts as an indirect check (all stored run_ids are UUIDs from `Uuid::new_v4()`), this relies on an indirect guarantee. If any future code path inserts a non-UUID run_id, this becomes a path traversal vector.

## Findings

- **Security Sentinel**: Medium severity. `workspace_run_dir()` joins `run_id` directly into the path. DB validation confirms existence but not format.
- **Learnings Researcher**: Past solution `tilde-home-expansion-file-tools.md` establishes the pattern of validating inputs before filesystem use.
- The `chat.rs` path passes `run_id` without any validation (no DB check like `ask.rs` has).

**Affected files:**
- `crates/mika-cli/src/commands/ask.rs` (~line 259)
- `crates/mika-cli/src/commands/chat.rs` (~line 634)
- `crates/mika-common/src/team.rs` (`workspace_run_dir`)

## Proposed Solutions

### Option A: UUID parse check at CLI entry points (Recommended)
Add `uuid::Uuid::parse_str(ref_id).is_err()` check in both `ask.rs` and `chat.rs` before passing to the engine.
- **Pros:** Simple one-liner, defense-in-depth, zero runtime cost on happy path
- **Cons:** Duplicated in two places
- **Effort:** Small
- **Risk:** None

### Option B: Validate in `workspace_run_dir()`
Add the UUID parse check inside `workspace_run_dir()` itself, returning `Result`.
- **Pros:** Single point of validation, cannot be bypassed
- **Cons:** Changes return type, more invasive
- **Effort:** Small-Medium
- **Risk:** Low

## Recommended Action

Option A for now — validate at CLI entry points.

## Acceptance Criteria

- [ ] `--run-id` with non-UUID value (e.g., `../../etc`) is rejected before filesystem use
- [ ] Both `ask.rs` and `chat.rs` paths validate format
- [ ] Test for invalid run-id format rejection

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-03-15 | Created from code review | Security sentinel + learnings researcher flagged |

## Resources

- Security finding #1 from security-sentinel review
- `docs/solutions/logic-errors/tilde-home-expansion-file-tools.md`
