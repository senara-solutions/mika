---
title: "KG Schema v27: docs_root_hash as shared-corpus primary key"
date: 2026-04-24
category: database-issues
module: mika-agent/db
problem_type: database_issue
component: database
symptoms:
  - "11 agents with the same docs_root each run extraction independently, producing N× duplicate rows"
  - "Entity-name counts range 26–43 across agents on the same doc (~65% spread from LLM non-determinism)"
  - "Extraction cost is 11× what it should be for identical corpora"
root_cause: scope_issue
resolution_type: migration
severity: medium
tags: [kg, schema, migration, sqlite, docs-root-hash, shared-corpus, v27, deduplication]
---

# KG Schema v27: docs_root_hash as shared-corpus primary key

## Problem

11 mika agents share a single hardcoded `docs_root` but each runs KG extraction independently. This produces N× duplicate rows across the shared-corpus layer (`kg_chunks`, `kg_subject_entities`, `kg_subject_relationships`, `kg_chunk_subjects`, `kg_chunk_subject_relationships`, `kg_extractions`). Empirically, entity-name counts range 26–43 across agents on the same doc — a ~65% spread caused by LLM non-determinism, not intentional divergence. The extraction pipeline is agent-agnostic (verified: `build_extraction_prompt` takes only text; system prompt interpolates only global `{approved_entity_types}`), so duplicate extraction is pure waste.

## Symptoms

- Same doc extracted 11 times (once per agent) at ~$0.05–$0.50 per restart
- Entity graphs drift across agents despite identical source documents
- `kg_extractions` table has N rows per doc instead of 1

## What Didn't Work

- **Single-agent consolidation (reduce agent count):** Would lose the per-agent resolution layer that bridges subject entities to each agent's distinct domain graph. Resolution is inherently per-agent.
- **Post-hoc dedup via SQL cleanup:** Would need to pick a "winner" among conflicting extractions without knowing which is most accurate. Treating the problem at the schema level prevents drift entirely.

## Solution

Schema v27 changes the primary-key scope of six shared-layer tables from `agent_id` to `docs_root_hash` — a 16-hex-char SHA-256 prefix of `fs::canonicalize(docs_root)`. Per-agent tables (`kg_subject_resolutions`, `kg_resolutions_log`) retain `agent_id`.

### Hash function

```rust
// crates/mika-agent/src/kg/config.rs
pub fn hash_docs_root(path: &Path) -> String {
    let canonical = std::fs::canonicalize(path)
        .unwrap_or_else(|_| path.to_path_buf());
    let mut hasher = Sha256::new();
    hasher.update(canonical.as_os_str().as_encoded_bytes());
    let digest = hasher.finalize();
    digest[..8].iter().map(|b| format!("{b:02x}")).collect::<String>()
}
```

Per-host stability only. Falls back to raw path bytes if canonicalization fails.

### Migration strategy (non-destructive)

`migrate_v26_to_v27()` uses the rename-preserve pattern:
1. Rename v26 tables to `*_v26_backup` (data preserved)
2. Create fresh v27 tables with `docs_root_hash` columns
3. **Do NOT coalesce data** — that's #787's scope
4. Add `schema_meta` table with `v27_coalesce_complete` marker

```sql
PRAGMA foreign_keys = OFF;
BEGIN IMMEDIATE;
CREATE TABLE IF NOT EXISTS schema_meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
ALTER TABLE kg_chunks RENAME TO kg_chunks_v26_backup;
CREATE TABLE kg_chunks (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    docs_root_hash TEXT NOT NULL,
    docs_root TEXT NOT NULL,
    seq_id INTEGER NOT NULL,
    source_doc_path TEXT NOT NULL,
    source_doc_hash TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    trace_id TEXT,
    UNIQUE (docs_root_hash, source_doc_path, seq_id)
);
-- ... repeat for 5 more shared-layer tables + rebuild per-agent tables to fix FK refs
INSERT INTO schema_version (version) VALUES (27);
COMMIT;
PRAGMA foreign_keys = ON;
```

### Startup guard

`Database::open()` refuses to return a handle when `schema_version == 27` and `schema_meta.v27_coalesce_complete` is absent. This prevents queries against empty v27 tables between #786 merge and #787 merge (covers unplanned restarts: package upgrade, kernel upgrade, OpenRC auto-restart).

### First-writer-wins on `kg_extractions`

```sql
-- v26: ON CONFLICT(agent_id, source_doc_path) DO UPDATE
-- v27: INSERT OR IGNORE against UNIQUE(docs_root_hash, source_doc_path)
INSERT OR IGNORE INTO kg_extractions
    (docs_root_hash, docs_root, source_doc_path, source_doc_hash, extraction_model, ...)
VALUES (?1, ?2, ?3, ?4, ?5, ...);
```

Second agent's INSERT returns 0 rows changed → skip downstream chunk/subject writes. Cost N× → 1×.

## Why This Works

The extraction pipeline is agent-agnostic — `build_extraction_prompt(annotated_text: &str)` takes only text, and the system prompt interpolates only `{approved_entity_types}` (global). A shared subject graph keyed by `docs_root_hash` eliminates drift by ensuring a single extraction per document per corpus. Resolution stays per-agent because it bridges subject entities to each agent's unique domain graph (skills, tools, problem types).

The three-layer safety model (startup guard + rename-preserve backup + non-deployment discipline) ensures the window between #786 and #787 is safe against any restart event.

## Prevention

- **Schema convergence test:** `tests/schema_v27_convergence.rs` — 10 scenarios verifying fresh DB (via `migrate_v1`) matches expected v27 structure. Catches drift between clean-slate and upgrade DDL paths.
- **Inline convergence test:** `test_v1_and_incremental_schemas_converge` in `db.rs` — compares full schema fingerprints between fresh and incrementally-migrated databases.
- **Migration immutability rule:** Historical migrations (`migrate_v24_to_v25`, `migrate_v25_to_v26`) are frozen. All v27 changes live in `migrate_v26_to_v27` alone. Clean-slate `migrate_v1` reflects the current target, not the historical path.
- **`PRAGMA foreign_keys = OFF` outside transactions:** SQLite's `foreign_keys` is connection-scoped and becomes a no-op inside open transactions. `defer_foreign_keys` is transaction-scoped but risky with multi-table renames. The pattern used: `PRAGMA foreign_keys = OFF; BEGIN IMMEDIATE; <DDL>; COMMIT; PRAGMA foreign_keys = ON;`.

## Related Issues

- [mika#786](https://github.com/senara-solutions/mika/issues/786) — This ticket (schema v27 DDL + Rust cutover)
- [mika#787](https://github.com/senara-solutions/mika/issues/787) — Data coalesce from v26 backup tables (T-B)
- [mika#778](https://github.com/senara-solutions/mika/issues/778) — Per-agent `docs_root` config
- [mika#779](https://github.com/senara-solutions/mika/issues/779) — KG CLI
- `docs/solutions/database-issues/kg-schema-three-layer-sqlite-design.md` — v25 design (D1 decision "agent_id on shared-layer tables" is amended by this v27 change)
- `docs/solutions/database-issues/iso8601-timestamp-migration.md` — Prior primary-key-rewrite precedent
- `docs/solutions/best-practices/first-boot-cost-spike-after-tracking-table-migration-2026-04-23.md` — Migration immutability rule
- `docs/solutions/best-practices/kg-lexical-ingestion-composed-write-2026-04-22.md` — Shared-write contract precedent
