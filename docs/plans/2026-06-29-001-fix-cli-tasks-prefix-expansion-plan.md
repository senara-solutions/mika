# Plan: fix(cli) — `mika tasks cancel/get <id>` prefix expansion (mika#1625)

## Problem

`mika tasks list` truncates task UUIDs to 12 characters for display (`&t.id[..12]`), but `mika tasks cancel <id>` and `mika tasks get <id>` use exact-match SQL (`WHERE id = ?1`). The operator copies the truncated ID from `list` output, pastes it into `cancel` or `get`, and gets "not found." There is no CLI path to discover the full UUID or cancel an orphan `in_progress` task.

This is a Tier 1 loop-breaker: orphan `in_progress` tasks held mika-dev dispatch slots for ~11 hours (2026-06-29) with no CLI escape hatch.

## Approach

**Prefix expansion in the CLI layer** (issue option 1). The CLI resolves a user-supplied ID prefix to the unique matching full UUID before calling the existing exact-match DB methods. No SQL schema changes. No new DB methods needed — we query the already-loaded task list or add a thin prefix-match query.

## Requirements

1. `mika tasks cancel <prefix>` resolves `<prefix>` to the unique task matching `id LIKE '<prefix>%'` (scoped to agent, cancellable statuses only), then calls `cancel_task_and_kill` with the full ID.
2. `mika tasks get <prefix>` resolves `<prefix>` to the unique task matching `id LIKE '<prefix>%'` (scoped to agent, all statuses), then calls `get_task` with the full ID.
3. **Exact match first:** If the user-supplied ID is an exact match, use it directly (backward compatible; also handles `--format json` scripted callers passing full UUIDs).
4. **Ambiguous prefix:** If the prefix matches multiple tasks, print all matching short IDs with their labels and exit with an error (no cancel/get performed).
5. **No match:** Existing "not found" message, unchanged.
6. **Minimum prefix length:** No minimum — even a 1-character prefix is valid if it uniquely resolves. The UUID space is sparse enough that short prefixes will typically be ambiguous, which is self-documenting.

## Design

### New DB method: `resolve_task_by_prefix`

Add to `Database` (`db.rs`):

```rust
pub fn resolve_task_by_prefix(&self, prefix: &str, agent_id: &str) -> Result<Vec<(String, String)>>
```

Returns `Vec<(id, label)>` for tasks where `id LIKE ?1 AND agent_id = ?2`. Limited to 10 rows (sufficient for ambiguity reporting; avoids unbounded result sets on very short prefixes).

Add async wrapper in `AsyncDatabase` (`async_db.rs`):

```rust
pub async fn resolve_task_by_prefix(&self, prefix: &str) -> Result<Vec<(String, String)>>
```

### CLI resolution helper

Add a private helper in `commands/tasks.rs`:

```rust
async fn resolve_task_id(db: &AsyncDatabase, id: &str) -> Result<String>
```

Logic:
1. Try `db.get_task(id)` — if `Some`, return `id` (exact match, fast path).
2. Call `db.resolve_task_by_prefix(id)`.
3. If exactly one result → return the full ID.
4. If zero results → bail with "Task {id} not found."
5. If multiple results → print the ambiguous matches and bail with "Ambiguous task ID prefix '{id}' — matches {N} tasks: ..." listing `short_id: "label"` for each.

### Integration points

- `TaskCommand::Cancel { id }`: Replace `cancel_task_and_kill(db, &id)` with `let full_id = resolve_task_id(db, &id).await?; cancel_task_and_kill(db, &full_id)`.
- `TaskCommand::Get { id, format }`: Replace `db.get_task(&id)` with `let full_id = resolve_task_id(db, &id).await?; db.get_task(&full_id)`.

### Not affected

- `TaskCommand::PromoteDeferred` — uses `find_active_callback_for_class`, not user-supplied task IDs.
- `reminders cancel` — uses a separate `ReminderCommand::Cancel` path. Same bug likely exists but is out of scope per the issue.
- Dashboard API / HTTP endpoints — use full UUIDs from the frontend; not affected.

## File changes

| File | Change |
|------|--------|
| `crates/mika-agent/src/db.rs` | Add `resolve_task_by_prefix()` method (~15 lines) |
| `crates/mika-agent/src/async_db.rs` | Add async `resolve_task_by_prefix()` wrapper (~5 lines) |
| `crates/mika-cli/src/commands/tasks.rs` | Add `resolve_task_id()` helper; wire into `Cancel` and `Get` arms (~30 lines) |

## Verification contract

1. **Unit test** (`db.rs`): `test_resolve_task_by_prefix` — create 3 tasks with known UUIDs, assert prefix match returns unique, assert ambiguous prefix returns multiple, assert no-match returns empty.
2. **Unit test** (`db.rs`): `test_resolve_task_by_prefix_exact_match` — full UUID returns single match.
3. **Integration test** (`commands/tasks.rs`): Existing tests remain passing (backward compatibility with full IDs).
4. **Manual smoke test**: `mika tasks list` → copy truncated ID → `mika tasks get <truncated>` succeeds → `mika tasks cancel <truncated>` succeeds.

## Definition of Done

- [ ] `resolve_task_by_prefix` added to `Database` and `AsyncDatabase`
- [ ] `resolve_task_id` helper in CLI wired into `Cancel` and `Get`
- [ ] Unit tests for prefix resolution (unique, ambiguous, no-match, exact-match)
- [ ] `cargo test -p mika-agent` passes
- [ ] `cargo test -p mika-cli` passes
- [ ] `cargo clippy` clean

## Acceptance criteria

1. `mika tasks cancel <truncated-12-char-id>` successfully cancels the task when the prefix uniquely identifies it.
2. `mika tasks get <truncated-12-char-id>` successfully displays the task detail when the prefix uniquely identifies it.
3. Ambiguous prefixes (matching multiple tasks) produce a clear error listing all matches.
4. Full UUIDs continue to work identically (backward compatibility).
5. No SQL schema changes required.
