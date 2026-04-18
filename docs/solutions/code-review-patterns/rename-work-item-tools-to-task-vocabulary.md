---
title: Rename work_item tools to task vocabulary (mika#608)
module: mika-agent
tags: [refactor, vocabulary, tool-rename, salvage]
problem_type: refactor
---

# Rename work_item tools to task vocabulary (mika#608)

## Problem

The agent saw tools named `create_work_item`, `update_work_item_status`, `list_work_items`, `check_work_item`, while the entire domain model around them used `task`: DB table `tasks`, engine identifiers `task_id`, CLI flag `--task-id`. This vocabulary split created room for the LLM to treat `work_item_id` and `task_id` as distinct entities, which contributed to the mika#595 UUID-fabrication incident.

## Resolution

Pre-1.0, so no backward-compat aliases. Single-pass rename of the entire work-item vocabulary to task:

- Tool files: `create_work_item.rs`, `update_work_item_status.rs`, `list_work_items.rs`, `check_work_item.rs` → `create_task.rs`, `update_task_status.rs`, `list_tasks.rs`, `check_task.rs`.
- Tool structs and registered names: `CreateWorkItemTool` → `CreateTaskTool`, etc., and the corresponding `"create_work_item"` strings → `"create_task"`.
- Scheduled-task tool disambiguation: the old `create_task` (generic scheduler) and `list_tasks` (generic scheduler) were renamed to `create_scheduled_task` / `list_scheduled_tasks` to make room for the tracking tools.
- DB layer: `find_active_work_item_by_{ref_url,pr_url,branch,label}` → `find_active_task_by_*`, `list_active_work_items` → `list_active_tasks`, `update_work_item_metadata` → `update_task_metadata`, `count_session_work_items` → `count_session_tasks`. `TaskHealthSummary.active_work_items` → `active_tasks`.
- Module rename: `work_item_metadata.rs` → `task_metadata.rs`.
- Skill prompts: typed-reference format propagated — `implement <repo> milestone#<n>`, `implement <repo> project#<n>`, `implement <repo> issue#<n>` (no bare `mika#N` for milestones/projects).
- Prose: "work item" / "Work item" → "task" / "Task" across crate source, tests, docs.

## Salvage Lessons

Two earlier autonomous attempts by mika-dev blew through 2+ hours and 650+ tool calls without ever running `cargo check`. The salvage rule that unstuck the rename: **run `cargo check` before editing anything, and re-run it every 5-10 edits**. `cargo check --all-targets` surfaces the dependency graph across lib + tests + other crates in one pass; relying on the lib-only build hides cascading failures in eval tests.

Bulk identifier renames benefit from a `grep -rl <old> | xargs sed -i -e 's/<old>/<new>/g'` sweep followed by a `cargo check` loop, not blind line-by-line editing. Distinguish identifier renames (word-bounded via full-prefix substitution — raw `\b` does not match before `_` in Rust snake_case) from prose renames (case-sensitive "work item" → "task") and run them in separate passes to avoid collateral damage.

Preserve legacy-format strings: `rewind.rs` parses historical audit rows encoded as `"work_item:<id>"` and must keep that exact prefix for backward-compat read. Similarly, incident callouts like `mika#485` in skill prompts refer to historical PRs and must not be reformatted as `mika issue#485`.

## Follow-ups

- Agent core memory (`core_memory` blocks on the live mika-dev agent) may still hold old tool names in the `self_model` block. Post-deploy audit required — not in PR scope.
- Community skills in the `mika-skills/` repo were not touched here; any residual `work_item` references there need a separate PR.

## References

- Plan: `docs/plans/2026-04-18-004-refactor-rename-work-item-tools-to-task-plan.md`
- Issue: senara-solutions/mika#608
- Related vocabulary incidents: mika#595 (UUID fabrication), mika#601 (task_id field standardization)
