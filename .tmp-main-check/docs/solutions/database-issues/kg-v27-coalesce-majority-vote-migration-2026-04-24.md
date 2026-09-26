---
module: database
date: 2026-04-24
problem_type: database_issue
component: database
severity: high
symptoms:
  - "11 agents each hold independent copies of KG extraction rows for the same corpus"
  - "v27 schema changes primary key scope from agent_id to docs_root_hash — requires data coalescing"
  - "Arbitrary row selection (MIN(id)) would systematically bias the graph toward whichever agent ingested first"
root_cause: logic_error
resolution_type: migration
tags: [kg, migration, v27, coalesce, majority-vote, sqlite, dedup, docs-root-hash]
---

# KG v27 Coalesce: Majority-Vote Migration for Per-Agent Row Deduplication

## Problem

Schema v27 flips the primary key of shared-corpus KG tables from `agent_id` to `docs_root_hash`. Current v26 databases have N agents each holding independent copies of chunks, subject entities, and relationships for the same corpus. The migration must coalesce N×M rows into M rows, selecting the "best" representative when agents disagree on entity names, types, or confidence values.

The empirical observation that drove the design: 26–43 distinct entity names across 11 agents on the same document (~65% spread). Simple `MIN(id)` selection would systematically bias the graph toward whichever agent was first to ingest.

## Solution

### Majority-vote with three-level tiebreak

The coalesce uses a majority-vote strategy with this tiebreak hierarchy:

1. **Agent count** — `COUNT(DISTINCT agent_id)` descending: entities extracted by more agents are more likely correct
2. **Mean confidence** — `AVG(confidence)` descending: higher-confidence extractions preferred
3. **Deterministic fallback** — `MIN(id)` ascending: stable, reproducible selection when all else ties

### Key SQL pattern (entity winner selection)

```sql
INSERT INTO kg_subject_entities (docs_root_hash, docs_root, entity_key, type, name, confidence, ...)
SELECT '{docs_root_hash}', '{docs_root}', entity_key, type, name, confidence, ...
FROM (
    SELECT b.*, ROW_NUMBER() OVER (
        PARTITION BY LOWER(TRIM(b.entity_key))
        ORDER BY vote_count DESC, avg_conf DESC, b.id ASC
    ) AS rn
    FROM kg_subject_entities_v26_backup b
    JOIN (
        SELECT LOWER(TRIM(entity_key)) AS norm_key,
               COUNT(DISTINCT agent_id) AS vote_count,
               AVG(confidence) AS avg_conf
        FROM kg_subject_entities_v26_backup
        GROUP BY LOWER(TRIM(entity_key))
    ) agg ON LOWER(TRIM(b.entity_key)) = agg.norm_key
)
WHERE rn = 1;
```

### FK rewiring via temp lookup tables

After selecting winners, every foreign key pointing at old row IDs must be rewired to the new IDs:

```sql
CREATE TEMP TABLE subject_entity_id_map (old_id INTEGER PRIMARY KEY, new_id INTEGER NOT NULL);

INSERT INTO subject_entity_id_map (old_id, new_id)
SELECT b.id, e.id FROM kg_subject_entities_v26_backup b
JOIN kg_subject_entities e ON LOWER(TRIM(e.entity_key)) = LOWER(TRIM(b.entity_key))
    AND e.docs_root_hash = '{docs_root_hash}';
```

Three lookup tables map every old ID to its winning new ID: `chunk_id_map`, `subject_entity_id_map`, `subject_relationship_id_map`. Downstream tables (`kg_chunk_subjects`, `kg_subject_resolutions`, etc.) JOIN against these maps in their INSERT statements.

### Junction table dedup with INSERT OR IGNORE

After rewiring, multiple v26 rows may collapse to the same (chunk_id, subject_entity_id) pair. `INSERT OR IGNORE` handles this gracefully — the first insertion wins, duplicates are silently dropped.

### Single-transaction composed write

The entire coalesce (DDL + data migration + FK rewiring + backup drop + marker write) runs inside one `execute_batch` call with `BEGIN IMMEDIATE...COMMIT`. Any failure rolls back atomically. The `v27_coalesce_complete` marker in `schema_meta` is the LAST statement before COMMIT.

## Why This Works

The majority-vote strategy reflects a key insight: when multiple independent LLM extraction runs disagree on an entity's existence, the entity that appears in more agents' extractions is more likely to be a genuine entity rather than extraction noise. The mean-confidence tiebreak further disambiguates among equally-popular entities.

The `LOWER(TRIM(entity_key))` normalization collapses case/whitespace variants (e.g., `skill:Self-Dev` vs `skill:self-dev`) into a single group while preserving the winning row's original casing in the stored value.

## Prevention

### TEMP tables survive transaction rollback in SQLite

SQLite TEMP tables are **session-scoped**, not transaction-scoped. If a migration fails after `CREATE TEMP TABLE` but before `DROP TABLE`, the TEMP tables persist for the connection lifetime. Always prefix with `DROP TABLE IF EXISTS` to make the SQL retryable:

```sql
DROP TABLE IF EXISTS chunk_id_map;
CREATE TEMP TABLE chunk_id_map (...);
```

### Never use SELECT * in migration copy steps

Always enumerate columns explicitly in `INSERT INTO ... SELECT` statements. This prevents silent column mismatches when schema versions drift between the backup source and the v27 target (lesson from the ISO 8601 timestamp migration).

### Test with realistic drift

The test fixture uses `DriftProfile::ObservedDrift` which simulates the observed 26–43 entity spread: each agent draws 8–12 from a "true set" of 30 entities, adds 2–4 agent-unique entities, and 1–2 case variants. This catches majority-vote bugs that would be invisible with identical data across agents.

### Insertion-order independence

The migration must produce identical results regardless of which agent's rows appear first in the backup table. The three-level tiebreak (`agent_count DESC, mean_confidence DESC, MIN(id) ASC`) ensures this. Invariant test #8 verifies it by running the coalesce on two identically-seeded DBs and comparing entity sets.
