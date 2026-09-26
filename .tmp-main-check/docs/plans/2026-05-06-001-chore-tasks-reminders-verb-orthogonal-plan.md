---
title: "chore(cli): Make tasks and reminders verb-orthogonal with sibling surfaces"
type: refactor
status: active
date: 2026-05-06
---

# chore(cli): Make tasks and reminders verb-orthogonal with sibling surfaces

## Overview

Add explicit `list` and `get` subcommands to `mika tasks` and `mika reminders`, matching the verb surface of `mika agents` and `mika skills`. Bare invocation remains as a backward-compatible alias for `list`.

## Problem Frame

The CLI is inconsistent: `agents` and `skills` use explicit verbs (`list`, `info`), but `tasks` and `reminders` only have implicit bare-invocation listing and `cancel`. This breaks agent discoverability (autonomous agents parse `--help` to discover verbs) and human muscle memory (`mika tasks list` errors out despite `--help` promising it).

## Requirements Trace

- R1. `mika tasks list` runs without error, returns same data as bare `mika tasks`
- R2. `mika tasks get <id>` returns a single task's full record
- R3. `mika reminders list` runs without error, returns same data as bare `mika reminders`
- R4. `mika reminders get <id>` returns a single reminder's full record
- R5. All new verbs documented in `--help` output
- R6. Bare `mika tasks` and `mika reminders` keep working (backward compat)
- R7. `--format text|json` on list and get verbs (per CLI conventions)

## Scope Boundaries

- No TUI changes
- No new verbs on other surfaces (e.g., `mika sessions`)
- No renaming `cancel` to `delete`
- No prefix-matching for IDs (exact match only — matches existing `get_task` DB method)

## Context & Research

### Relevant Code and Patterns

- `crates/mika-cli/src/cli.rs` — `TaskArgs`/`TaskCommand` at L584-597, `ReminderArgs`/`ReminderCommand` at L569-582
- `crates/mika-cli/src/commands/tasks.rs` — current bare-list + cancel implementation
- `crates/mika-cli/src/commands/reminders.rs` — current bare-list + cancel implementation
- `crates/mika-cli/src/commands/skills.rs` — canonical pattern: `None` → list, `Some(List)` → list with format, `Some(Info)` → detail view
- `crates/mika-agent/src/db.rs` — `Task` struct (L135-169), `get_task(id, agent_id)` (L4189)
- `crates/mika-agent/src/async_db.rs` — `get_task(&self, id)` (L233), `get_tasks_by_status()`, `get_user_visible_tasks()`

### Institutional Learnings

- **CLI output format convention:** All list/show commands use `--format text|json` via `OutputFormat` enum
- **AgentFlag pattern:** Flatten into subcommand structs, never use `global = true`
- **ID suffix convention:** Opaque IDs use `--task-id` naming, but positional args like `get <id>` are fine
- **Reminders are tasks:** `trigger_type IN ('time', 'recurring')` with `action_type IN ('send_message', 'resume_agent')`

## Key Technical Decisions

- **`get` not `info`:** The issue specifies `get`, and the pattern maps better to the data-retrieval semantics (tasks are looked up by ID, not by name like skills)
- **Exact ID match only:** No prefix matching — keeps it simple, uses existing `get_task` DB method. Users copy IDs from `list` output
- **Bare invocation stays as alias:** `Option<TaskCommand>` pattern preserved — `None` delegates to list logic, matching the existing skills pattern
- **`--format` on list and get:** Both support `text|json`. JSON output uses serde on the Task struct
- **Full ID in list output:** Keep showing short IDs in text mode for readability, but include full IDs in JSON output

## Implementation Units

- [ ] **Unit 1: Add `List` and `Get` variants to `TaskCommand` and `ReminderCommand` enums**

**Goal:** Extend the clap enums with new subcommand variants

**Requirements:** R1, R2, R3, R4, R5, R6

**Dependencies:** None

**Files:**
- Modify: `crates/mika-cli/src/cli.rs`

**Approach:**
- Add `List` variant with `format: OutputFormat` arg to both `TaskCommand` and `ReminderCommand`
- Add `Get` variant with positional `id: String` and `format: OutputFormat` arg to both
- Keep `Cancel` variant unchanged
- Add doc comments for `--help` output

**Patterns to follow:**
- `SkillsCommand::List { format }` and `SkillsCommand::Info { name }` in same file

**Test scenarios:**
- Test expectation: none — pure struct/enum definition, tested indirectly via Unit 2/3

**Verification:**
- `cargo build` succeeds with new enum variants

- [ ] **Unit 2: Implement `tasks list` and `tasks get` handlers**

**Goal:** Wire up the new task subcommands with list and detail logic

**Requirements:** R1, R2, R6, R7

**Dependencies:** Unit 1

**Files:**
- Modify: `crates/mika-cli/src/commands/tasks.rs`
- Test: `crates/mika-cli/src/commands/tasks.rs` (inline tests)

**Approach:**
- `None` and `Some(TaskCommand::List { format })` both call the same list function (extract current bare-list logic into a helper)
- `Some(TaskCommand::Get { id, format })` calls `db.get_task(&id)`, renders full detail view
- Text detail view: print all meaningful fields line-by-line (id, label, status, type, trigger_type, action_type, agent_id, created_at, updated_at, next_fire_at, fired_at, completed_at, parent_task_id, reference_url, source, process_id, cron_expr, result)
- JSON detail view: serialize the Task struct
- For list: text mode keeps current short-ID format; JSON mode serializes full task objects

**Patterns to follow:**
- `show_skill_detail()` in `crates/mika-cli/src/commands/skills.rs` for detail view formatting
- Current `None` arm in `tasks.rs` for list logic

**Test scenarios:**
- Happy path: `get` with valid ID returns full task detail
- Error path: `get` with non-existent ID prints "Task not found" message and exits gracefully
- Happy path: `list` with `--format json` produces valid JSON array
- Happy path: `list` with no tasks prints empty message

**Verification:**
- `mika tasks list` produces same output as bare `mika tasks`
- `mika tasks get <id>` shows full task detail
- `mika tasks list --format json` outputs JSON array

- [ ] **Unit 3: Implement `reminders list` and `reminders get` handlers**

**Goal:** Wire up the new reminder subcommands with list and detail logic

**Requirements:** R3, R4, R6, R7

**Dependencies:** Unit 1

**Files:**
- Modify: `crates/mika-cli/src/commands/reminders.rs`
- Test: `crates/mika-cli/src/commands/reminders.rs` (inline tests)

**Approach:**
- Same pattern as Unit 2 but using `get_user_visible_tasks()` for list and `get_task()` for detail
- Detail view surfaces reminder-specific fields prominently: label, fire_at, status, cron_expr (for recurring), action_config (the message text)
- For `get`: after fetching task, verify it's a reminder type (trigger_type is 'time' or 'recurring') — if not, print "Not a reminder" error

**Patterns to follow:**
- Unit 2's implementation
- Current `None` arm in `reminders.rs`

**Test scenarios:**
- Happy path: `get` with valid reminder ID returns full reminder detail
- Error path: `get` with non-existent ID prints "Reminder not found"
- Edge case: `get` with a task ID that's not a reminder (trigger_type = 'manual') prints appropriate error
- Happy path: `list --format json` produces valid JSON

**Verification:**
- `mika reminders list` produces same output as bare `mika reminders`
- `mika reminders get <id>` shows reminder detail

- [ ] **Unit 4: Update CLAUDE.md CLI documentation**

**Goal:** Document the new verbs in the CLI reference

**Requirements:** R5

**Dependencies:** Units 2, 3

**Files:**
- Modify: `crates/mika-cli/CLAUDE.md`

**Approach:**
- Add `mika tasks list`, `mika tasks get <id>` to the subcommands documentation
- Add `mika reminders list`, `mika reminders get <id>` similarly
- Note `--format text|json` support
- Add both to the "Other `--format text|json` Commands" list

**Test scenarios:**
- Test expectation: none — documentation only

**Verification:**
- CLAUDE.md accurately reflects the new CLI surface

## System-Wide Impact

- **API surface parity:** This change brings tasks/reminders in line with agents/skills. Future CLI noun-surfaces should follow the same verb pattern (list, get/info, create, delete/cancel).
- **Unchanged invariants:** The underlying DB methods (`get_task`, `get_tasks_by_status`, `get_user_visible_tasks`, `cancel_task`) are unchanged. The task engine, scheduler, and server API are not affected.
- **Agent readiness:** Autonomous agents parsing `--help` will now discover `list` and `get` verbs consistently across all surfaces.

## Risks & Dependencies

| Risk | Mitigation |
|------|------------|
| Breaking scripts that parse bare `mika tasks` output | Bare invocation preserved as alias — no output format change |
| ID confusion between tasks and reminders in `get` | Reminders validate trigger_type; tasks show any task regardless of type |

## Sources & References

- Related issue: #981
- Existing pattern: `crates/mika-cli/src/commands/skills.rs` (list/info pattern)
- Learnings: `docs/solutions/architecture-patterns/cli-output-format-list-commands.md`
- Learnings: `docs/solutions/cli-features/kg-cli-management-subcommands-2026-04-25.md`
