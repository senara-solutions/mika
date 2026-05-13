---
module: skills/bundled/resolve-pr-conflicts
tags: [worktree, path-derivation, tool-schema, LLM-input-sanitization]
problem_type: logic-error
category: logic-errors
issue: 783
date: 2026-05-13
---

# resolve_pr_conflicts: LLM-constructed worktree paths fail silently

## Problem

The `resolve_pr_conflicts` tool required `worktree_path` as a required input — an absolute path the LLM had to construct by applying the canonical branch-to-directory sanitization rule (`/` → `-`). LLMs routinely got this wrong, passing slash-separated paths (`feat/286/...`) instead of dash-separated (`feat-286-...`), causing the handler to `cd` into a nonexistent directory and exit 1.

Observed in mika-dev session on mika#286 / PR #782: two consecutive retries with incorrect paths, both producing `HANDLER CRASH (exit code 1)`.

## Root Cause

LLMs should not construct slugged/sanitized filesystem paths. The sanitization rule lives in `mika-platform/scripts/derive-worktree-path`, not in the tool's schema or prompt. Even after persisting the slash→dash rule as a durable fact, any future agent without that fact would repeat the bug.

## Fix

Two-tier worktree path resolution in the handler:

1. **`pr_url` (preferred):** New optional field. Handler parses repo + PR number from the URL, calls `gh pr view` for `headRefName`, then delegates to `derive-worktree-path` for canonical path computation.
2. **`worktree_path` (deprecated fallback):** Kept for backward compatibility. If both provided, handler validates they match (WARN on mismatch, uses derived path).

Schema change: `required` narrowed from `["worktree_path", "task_id"]` to `["task_id"]`.

### Key design decisions

- **No DB tier:** No existing handler reads `mika.db` directly. Adding sqlite3 as a handler dependency would cross the handler/storage boundary without precedent. Two-tier (pr_url → worktree_path) is sufficient.
- **Mismatch = WARN, not error:** When both inputs are provided and disagree, the derived path wins with a warning. Hard rejection would break backward-compatible callers that pass both.
- **`derive-worktree-path` is the single source of truth:** The handler delegates to the canonical script rather than reimplementing the `tr '/' '-'` sanitization. This ensures the handler stays correct if the sanitization rule ever changes.

## Files Changed

- `skills/bundled/resolve-pr-conflicts/tools.json` — schema: `pr_url` added, `worktree_path` deprecated, `required` narrowed
- `skills/bundled/resolve-pr-conflicts/handlers/run.sh` — `derive_worktree_from_pr()` function + two-tier resolution logic
- `skills/bundled/resolve-pr-conflicts/system_prompt.md` — updated inputs table, example, behavior list

## Lesson

**Never require LLMs to construct derived/sanitized values.** If a value can be computed from other available inputs (task_id → branch → sanitized path), derive it in the handler. Prompt enforcement of sanitization rules is fragile — structural derivation eliminates the failure class entirely. This aligns with `feedback_prompt_enforcement_fragile.md`: structural > prompt.
