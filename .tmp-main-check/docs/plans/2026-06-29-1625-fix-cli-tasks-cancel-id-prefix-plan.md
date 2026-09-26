# Plan: fix(cli) — `mika tasks cancel <id>` rejects truncated id from `tasks list`

**Issue:** senara-solutions/mika#1625
**Type:** fix (bug)
**Scope:** mika-cli, mika-agent (db layer)

## Problem

`mika tasks list` truncates task UUIDs to 12 characters for display (`&t.id[..12]`), but `mika tasks cancel <id>` and `mika tasks get <id>` use exact-match SQL (`WHERE id = ?1`). The operator copy-pastes the truncated ID from `list` output and hits "Task not found" — a complete UX dead-end with no workaround from the CLI surface.

Hard evidence: two orphan `in_progress` tasks held mika-dev dispatch slots for ~11 hours (2026-06-29) because the operator couldn't cancel them via the displayed IDs.

## Approach: Prefix Expansion in the CLI Layer

Add a prefix-match resolution step at the CLI layer. When the user provides a task ID that doesn't exact-match, expand it as a prefix against the agent's tasks table. This is the ticket's preferred approach (option 1) — operator-ergonomic, minimal SQL surface change.

The resolution lives in the DB layer as a new query method, invoked from the CLI handlers. The existing `cancel_task_and_kill` and `get_task` paths continue to use exact-match IDs — the prefix expansion happens before those calls.

## Requirements

1. `mika tasks cancel <truncated-id>` resolves the truncated ID to the unique full UUID and cancels the task
2. `mika tasks get <truncated-id>` resolves the truncated ID to the unique full UUID and shows full details
3. Ambiguous prefix (multiple matches) produces a clear error listing the matching task IDs
4. No matches produces the existing "not found" message (unchanged)
5. Full UUIDs still work (exact match takes priority, no regression)
6. Minimum prefix length: 4 characters (prevents overly broad matches)

## Implementation Steps

### Step 1: Add `resolve_task_id_by_prefix` to `Database` (db.rs)

New method on `Database`:

```rust
pub fn resolve_task_id_by_prefix(&self, prefix: &str, agent_id: &str) -> Result<Vec<String>> {
    let mut stmt = self.conn.prepare(
        "SELECT id FROM tasks WHERE id LIKE ?1 || '%' AND agent_id = ?2 ORDER BY id LIMIT 10"
    )?;
    let ids: Vec<String> = stmt
        .query_map(params![prefix, agent_id], |row| row.get(0))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(ids)
}
```

Key decisions:
- `LIMIT 10`: cap results for the ambiguity message; no point listing hundreds
- Scoped to `agent_id`: same security boundary as all other task queries
- Returns `Vec<String>` of full IDs: caller decides how to handle 0/1/N matches
- No status filter: the caller (`cancel` vs `get`) has different status expectations — `cancel` already filters via `cancel_task`'s `status NOT IN (...)` clause; `get` returns any status

### Step 2: Add `resolve_task_id_by_prefix` to `AsyncDatabase` (async_db.rs)

Async wrapper following the existing pattern:

```rust
pub async fn resolve_task_id_by_prefix(&self, prefix: &str) -> Result<Vec<String>> {
    let p = prefix.to_owned();
    let a = self.agent_id.clone();
    self.with_db(move |db| db.resolve_task_id_by_prefix(&p, &a)).await
}
```

### Step 3: Add `resolve_task_id` helper to CLI tasks.rs

A shared resolution function used by both `cancel` and `get` handlers:

```rust
async fn resolve_task_id(db: &AsyncDatabase, input: &str) -> Result<Option<String>> {
    // Try exact match first (fast path, no regression)
    if db.get_task(input).await?.is_some() {
        return Ok(Some(input.to_string()));
    }

    // Minimum prefix length guard
    if input.len() < 4 {
        return Ok(None); // fall through to "not found"
    }

    // Prefix expansion
    let matches = db.resolve_task_id_by_prefix(input).await?;
    match matches.len() {
        0 => Ok(None),
        1 => Ok(Some(matches.into_iter().next().unwrap())),
        _ => {
            eprintln!("\n  Ambiguous task ID prefix '{input}'. Matches:");
            for id in &matches {
                eprintln!("    {id}");
            }
            eprintln!();
            std::process::exit(1);
        }
    }
}
```

Key decisions:
- Exact match first: full UUIDs hit the fast path with zero regression risk
- 4-char minimum prefix: prevents accidentally matching dozens of tasks with very short prefixes. The `tasks list` display uses 12 chars, so operators will always provide at least 12
- `process::exit(1)` on ambiguity: same pattern as `PromoteDeferred` error handling in the same file

### Step 4: Wire `resolve_task_id` into `Cancel` handler (tasks.rs)

Replace the direct `cancel_task_and_kill(db, &id)` call:

```rust
Some(TaskCommand::Cancel { id }) => {
    let resolved_id = match resolve_task_id(db, &id).await? {
        Some(id) => id,
        None => {
            println!("\n  Task {id} not found or already completed.\n");
            return Ok(());
        }
    };
    match mika_agent::task_engine::process_kill::cancel_task_and_kill(db, &resolved_id).await? {
        // ... existing match arms, using resolved_id in messages
    }
}
```

### Step 5: Wire `resolve_task_id` into `Get` handler (tasks.rs)

Replace the direct `db.get_task(&id)` call:

```rust
Some(TaskCommand::Get { id, format }) => {
    let resolved_id = match resolve_task_id(db, &id).await? {
        Some(id) => id,
        None => {
            println!("\n  Task {id} not found.\n");
            return Ok(());
        }
    };
    let task = db.get_task(&resolved_id).await?;
    // ... existing match arms
}
```

### Step 6: Add unit tests

In `db.rs` tests:
- `test_resolve_task_id_by_prefix_unique_match` — single match returns the full ID
- `test_resolve_task_id_by_prefix_ambiguous` — multiple matches returns all
- `test_resolve_task_id_by_prefix_no_match` — empty result
- `test_resolve_task_id_by_prefix_exact_match` — full UUID still works
- `test_resolve_task_id_by_prefix_scoped_to_agent` — different agent's tasks don't match

In `tasks.rs` tests (existing test module):
- `test_resolve_task_id` tests are integration-shaped (need AsyncDatabase); if impractical, cover via the DB-level tests above

## Files Changed

| File | Change |
|------|--------|
| `crates/mika-agent/src/db.rs` | Add `resolve_task_id_by_prefix()` method |
| `crates/mika-agent/src/async_db.rs` | Add async wrapper |
| `crates/mika-cli/src/commands/tasks.rs` | Add `resolve_task_id()` helper; wire into `Cancel` and `Get` handlers |

## Verification Contract

1. **Truncated cancel works:** `mika tasks cancel <12-char-prefix>` succeeds when the prefix uniquely identifies a task
2. **Truncated get works:** `mika tasks get <12-char-prefix>` shows full task details
3. **Full UUID still works:** No regression for exact-match IDs
4. **Ambiguity reported:** Multiple matches produce a clear list and non-zero exit
5. **Short prefix rejected:** Prefixes < 4 chars fall through to "not found"
6. **Agent scoping preserved:** Prefix resolution is scoped to the active agent
7. **`cargo test` passes:** All existing + new tests pass
8. **`cargo clippy` clean:** No new warnings

## Definition of Done

- [ ] `resolve_task_id_by_prefix` added to `Database` and `AsyncDatabase`
- [ ] CLI `cancel` and `get` handlers use prefix resolution
- [ ] Unit tests for prefix resolution (unique, ambiguous, no match, agent scoping)
- [ ] `cargo test` passes
- [ ] `cargo clippy` clean

## Acceptance criteria

1. `mika tasks cancel <12-char-id-from-list>` cancels the task (the primary UX dead-end fix)
2. `mika tasks get <12-char-id-from-list>` shows full task details
3. Ambiguous prefixes (multiple tasks sharing a prefix) produce a clear error listing all matches
4. Full UUIDs continue to work unchanged (no regression)
5. Prefix resolution is agent-scoped (one agent's prefix cannot resolve to another agent's task)

## Out of Scope

- Automatic orphan task reaping (separate concern per ticket)
- Changing `tasks list` display format (the prefix expansion approach means truncated display is fine)
- Prefix resolution for `promote-deferred` (uses dispatch class, not task ID)
- HTTP API prefix resolution (server endpoints are programmatic, not operator-facing)
