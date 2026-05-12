---
title: "Agent state reset CLI pattern — per-table transactional wipe with safety guards"
date: 2026-05-12
category: best-practices
module: mika-cli
problem_type: best_practice
component: tooling
severity: medium
applies_when:
  - Adding a CLI command that deletes data across many tables for a scoped entity
  - Implementing destructive operations that must preserve parent rows while clearing children
  - Building shared-resource safety checks (e.g., KG corpus shared across agents)
tags:
  - agent-reset
  - cli
  - sqlite
  - transactional-delete
  - fts5-rebuild
  - shared-corpus-safety
  - destructive-operations
---

# Agent state reset CLI pattern — per-table transactional wipe with safety guards

## Context

Mika agents accumulate state across 22+ SQLite tables (sessions, messages, memory, tasks, KG corpus, audit events, tool/LLM call history, skill overrides, etc.). Operators need a way to return an agent to a freshly-provisioned state without losing identity configuration (`identity.toml`, agent row, directory layout). The closest prior art was `mika kg purge --agent <name>` which only covered KG tables. Recreating via delete was unsafe — `ON DELETE CASCADE` on `agents(id)` removes the agent row, and re-provisioning replaces customized `identity.toml` with well-known defaults.

## Guidance

### Single-transaction delete with macro-based repetition

Use a macro to eliminate repetition when deleting from many tables by the same column:

```rust
macro_rules! delete_by_agent {
    ($table:expr, $field:ident) => {
        tx.execute(
            &format!("DELETE FROM {} WHERE agent_id = ?1", $table),
            params![agent_id],
        )?;
        counts.$field = tx.changes();
    };
}
```

This keeps the delete logic DRY across 22+ tables while capturing per-table counts via `tx.changes()`.

### FTS5 external-content rebuild must be post-transaction

When deleting from a table that backs an FTS5 external-content virtual table (e.g., `search_content` backing `fts_search`), the FTS5 rebuild command must run **after** the transaction commits:

```rust
tx.commit()?;
// If rebuild fails, data is already deleted — warn instead of propagating error.
// The FTS index will self-heal on next startup when search_content is re-indexed.
if let Err(e) = self.conn.execute("INSERT INTO fts_search(fts_search) VALUES('rebuild')", []) {
    tracing::warn!(error = %e, agent_id, "FTS5 rebuild failed — index may be stale");
}
```

The `'rebuild'` command re-scans the content table and reconstructs the entire FTS index. Running it inside the transaction can conflict with active transactions. Handle rebuild failure gracefully — the transaction already committed, so propagating the error would mislead callers into thinking the reset failed when all data was already deleted.

### Shared-resource safety checks

For tables shared across entities (e.g., `kg_chunks` shared via `docs_root_hash` across agents), query ownership **before** deleting the mapping table:

```rust
// Query corpora BEFORE deleting agent_kg_corpora rows
let corpora = tx.prepare("SELECT docs_root_hash FROM agent_kg_corpora WHERE agent_id = ?1")?;
// ... for each hash, check if other agents reference it
let other_refs: i64 = tx.query_row(
    "SELECT COUNT(*) FROM agent_kg_corpora WHERE docs_root_hash = ?1 AND agent_id != ?2",
    params![hash, agent_id], |r| r.get(0),
)?;
if other_refs == 0 {
    // Sole owner — safe to delete shared rows in FK-safe order
}
```

### Active-task guard pattern

Refuse destructive operations when the target entity has active work. Provide `--force` to override and `--dry-run` to preview:

```rust
let active = db.get_active_tasks_for_agent(&agent_id)?;
if !active.is_empty() && !force {
    bail!("agent_busy: {} active task(s)", active.len());
}
```

### Typed-name confirmation for destructive CLI operations

For operations that delete significant amounts of data, require the operator to type the entity name (not just `y/N`). This follows the `mika kg purge` precedent:

```rust
print!("  Type the agent name to confirm: ");
io::stdout().flush()?;
let mut input = String::new();
io::stdin().read_line(&mut input)?;
if input.trim() != name {
    return Ok(());  // Clean exit on user abort — don't use process::exit()
}
```

Non-TTY contexts must require `--yes` to prevent hangs in scripted/autonomous contexts.

## Why This Matters

Destructive multi-table operations are a common source of data integrity bugs. The patterns here — single transaction, macro-based repetition, shared-resource safety, active-task guards — compose into a reliable template. The FTS5 rebuild placement is a subtle SQLite requirement that, if wrong, causes silent stale-index bugs. The shared-corpus safety check prevents one agent's reset from destroying another agent's KG data.

## When to Apply

- Adding new CLI commands that delete data across multiple tables for a scoped entity
- Implementing "reset" or "purge" operations that must be selective (delete children, preserve parent)
- Working with FTS5 external-content tables after bulk deletes
- Building safety guards for shared resources in a multi-agent system

## Examples

The `mika agent reset <name>` command demonstrates all patterns:

```bash
# Preview what would be deleted
mika agents reset mika-test --dry-run

# Reset with confirmation
mika agents reset mika-test

# Reset non-interactively (for scripts)
mika agents reset mika-test --yes

# Force reset even with active tasks
mika agents reset mika-dev --force --yes
```

Post-reset state: zero rows in all 22+ child tables, agent row preserved, `identity.toml` untouched, `~/.mika/agents/<name>/` directory preserved. Bundled skills restore on next startup via `seed_bundled_skills()`.

## Related

- `crates/mika-agent/src/db.rs` — `reset_agent_state()`, `count_agent_state()`, `purge_kg_for_agent()` (KG-only precedent)
- `crates/mika-cli/src/commands/agents.rs` — CLI handler with guard/confirmation/execution
- `crates/mika-cli/src/commands/kg.rs` — `run_purge()` (confirmation pattern precedent)
- mika#964 — Feature ticket
