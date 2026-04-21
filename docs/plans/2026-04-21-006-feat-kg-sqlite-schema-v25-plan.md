---
title: "feat: KG SQLite schema v25 — entities, relationships, chunks, subject tables"
type: feat
status: active
date: 2026-04-21
---

# feat: KG SQLite schema v25 — entities, relationships, chunks, subject tables

## Overview

Add the SQLite schema foundation for Mika's Knowledge Graph (milestone #14). Schema v24 → v25 migration with 10 new tables across three KG layers, plus a companion Rust module with column constants and type enums. Also fix a pre-existing CI flake in `toggle_skill.rs` caused by a shared-static race condition.

## Problem Frame

The current memory model has three layers (core memory, structured facts, hybrid search) but no structured knowledge representation. Issue #722 lays the relational foundation — typed entities, directed relationships, text chunks, subject extraction, and provenance tracking — that future issues (#687–#692) will populate and query.

A CI flake in `test_already_disabled` (toggle_skill.rs:274) blocks merging any PR. The root cause is a process-wide `static AtomicBool` shared across concurrent tests.

## Requirements Trace

- R1. Migrate schema from v24 to v25 (incremental path)
- R2. Update clean-slate `create_tables_v1()` to include all KG tables
- R3. Domain layer: `kg_entities` (global, typed nodes with CHECK-enforced `entity_key = type:name`) and `kg_relationships` (directed edges, FK cascade)
- R4. Lexical layer: `kg_chunks` (per-agent, composes with `search_content` via `source_type='kg_chunk'`)
- R5. Subject layer: `kg_subject_entities` (confidence-scored), `kg_subject_resolutions` (subject→domain), `kg_subject_relationships` (fact triples)
- R6. Provenance: `kg_chunk_subjects`, `kg_chunk_subject_relationships`
- R7. Tracking: `kg_extractions`, `kg_resolutions_log`
- R8. `db/kg_schema.rs` module with column constants, type enums, `format_entity_key` helper
- R9. `docs/architecture/kg-id-convention.md` documenting the typed-prefix ID scheme
- R10. Update ADR-003 with KG composition notes
- R11. 12+ forward tests including convergence test for clean-slate vs incremental migration
- R12. Fix `test_already_disabled` CI flake in `toggle_skill.rs`

## Scope Boundaries

- No population logic, no query tools — those are #687–#692
- No KG-specific agent tools — schema only
- No changes to the existing `search_content` pipeline beyond documenting the composition point

### Deferred to Separate Tasks

- KG entity extraction (#687)
- KG query tools (#688–#692)
- KG population from existing facts

## Context & Research

### Relevant Code and Patterns

- `crates/mika-agent/src/db.rs` — single ~9000-line file with all schema, migrations, and DB operations. `CURRENT_SCHEMA_VERSION = 24`. Migration chain in `migrate()`. `create_tables_v1()` for fresh DBs.
- `search_content` table schema: `(id INTEGER PK, agent_id TEXT, source_type TEXT, source_id INTEGER, content TEXT, embedding_json TEXT, timestamps)`. Source types: `person`, `preference`, `commitment`, `event`. Indexed on `(agent_id, source_type)`. KG chunks will add `source_type='kg_chunk'`.
- `crates/mika-agent/src/tools/toggle_skill.rs` — test uses `static SKILLS_DIRTY` from `test_utils.rs`, races with other tests.
- `crates/mika-agent/src/tools/send_message.rs` — demonstrates safe pattern: local `let skills_dirty = AtomicBool::new(false)` per test.

### Institutional Learnings

- All timestamps must be `TEXT` in ISO 8601 format using `crate::timestamp` helpers (docs/solutions: iso8601-timestamp-migration)
- All text PKs and unique columns use `COLLATE NOCASE` (docs/solutions: skill-override-persistence)
- Every table must include `agent_id TEXT NOT NULL` for per-agent scoping (docs/solutions: consolidate-per-agent-dbs) — **exception: domain-layer tables are global (D1)**
- Simple new table creation uses `CREATE TABLE IF NOT EXISTS` in migration (no ALTER/rebuild needed)
- CHECK constraints on enum columns follow the `tasks.type` pattern

## Key Technical Decisions

- **Agent scoping per-layer (D1):** Domain tables (`kg_entities`, `kg_relationships`) are global (no `agent_id`). Subject and lexical tables are per-agent. This follows the issue's design: entities are shared knowledge; subject interpretations are per-agent.
- **INTEGER PK for joins, TEXT entity_key for identity (D3):** Integer PKs for performant joins. UNIQUE `entity_key` (format `type:name`) for external identity and deduplication. CHECK constraint enforces the format.
- **Composed indexing (D2):** KG chunks reuse the existing `search_content` pipeline via `source_type='kg_chunk'` — no parallel FTS5/vec tables needed.
- **No direct chunk→entity FK (D9):** Linkage goes through the subject→resolution pipeline. This keeps the lexical and domain layers loosely coupled.
- **Content-change idempotency (D10):** `source_doc_hash NOT NULL` on chunks enables skip-if-unchanged extraction.
- **db/ sub-module for KG schema:** Extract KG-specific constants and helpers into `crates/mika-agent/src/db/kg_schema.rs` as a new sub-module rather than adding to the already-large `db.rs`. The actual SQL DDL and migration code stays in `db.rs` (consistent with all other migrations), but the typed constants, enums, and helper functions live in the new module.
- **toggle_skill fix:** Replace shared static `SKILLS_DIRTY` with local `AtomicBool` in the two tests that assert on `skills_dirty`, following the `send_message.rs` pattern.

## Open Questions

### Resolved During Planning

- **Should domain tables have agent_id?** No — D1 specifies global scope for entities and relationships.
- **Where does DDL go vs constants?** DDL/migration in `db.rs` (consistent with existing pattern). Constants, enums, and helpers in `db/kg_schema.rs`.

### Deferred to Implementation

- Exact `entity_key` CHECK constraint regex vs simpler substring check — depends on SQLite regex support
- Whether `kg_resolutions_log` needs additional indexes beyond PK — depends on query patterns in #688

## Output Structure

```
crates/mika-agent/src/db/
├── mod.rs                  # re-exports from existing db.rs + kg_schema
└── kg_schema.rs            # KG column constants, type enums, format_entity_key

docs/architecture/
└── kg-id-convention.md     # typed-prefix ID scheme documentation
```

Note: The db.rs file remains at `crates/mika-agent/src/db.rs` — the new `db/` directory is an addition. If Rust module resolution requires restructuring (db.rs → db/mod.rs), that will be handled in Unit 1.

## Implementation Units

- [x] **Unit 0: Fix toggle_skill.rs CI flake**

**Goal:** Eliminate the shared-static race condition in `test_already_disabled` and `test_disable_sets_skills_dirty`.

**Requirements:** R12

**Dependencies:** None

**Files:**
- Modify: `crates/mika-agent/src/tools/toggle_skill.rs`

**Approach:**
- In `test_already_disabled` and `test_disable_sets_skills_dirty`, replace `harness.ctx_with_home()` with a manually-constructed `ToolContext` that uses a local `let skills_dirty = AtomicBool::new(false)` and `let pr_review_posted = AtomicBool::new(false)`, following the pattern in `send_message.rs` tests.
- Keep `ctx_with_home()` for the other toggle_skill tests that don't assert on `skills_dirty`.

**Patterns to follow:**
- `crates/mika-agent/src/tools/send_message.rs` tests — local `AtomicBool` per test

**Test scenarios:**
- Happy path: `test_already_disabled` passes reliably (no race with concurrent tests)
- Happy path: `test_disable_sets_skills_dirty` passes reliably
- Happy path: All other toggle_skill tests remain unchanged and pass

**Verification:**
- `cargo test -p mika-agent toggle_skill` passes consistently (run 5+ times)

- [x] **Unit 1: KG schema module (`db/kg_schema.rs`)**

**Goal:** Create the Rust module with KG column constants, entity type enums, and the `format_entity_key` helper.

**Requirements:** R8

**Dependencies:** None (can be developed in parallel with Unit 0)

**Files:**
- Create: `crates/mika-agent/src/db/kg_schema.rs`
- Create: `crates/mika-agent/src/db/mod.rs` (or restructure if needed)
- Modify: `crates/mika-agent/src/db.rs` (add `pub mod kg_schema` or restructure)

**Approach:**
- Define entity type constants (e.g., `ENTITY_TYPE_PERSON`, `ENTITY_TYPE_ORG`)
- Define relationship type constants
- Define `format_entity_key(entity_type, name) -> String` that produces `type:name` format
- Add write contract documentation as doc comments
- Keep module lightweight — no DB access, pure types and helpers

**Patterns to follow:**
- `TASK_TYPE_ISSUE`/`TASK_TYPE_MILESTONE`/`TASK_TYPE_PROJECT`/`VALID_TASK_TYPES` constants pattern in `db.rs`

**Test scenarios:**
- Happy path: `format_entity_key("person", "Alice")` → `"person:Alice"`
- Edge case: empty name → should still format (or error)
- Edge case: name containing `:` → should handle (colon is a separator)

**Verification:**
- `cargo test -p mika-agent kg_schema` passes
- Module compiles cleanly with `cargo clippy`

- [x] **Unit 2: Schema v25 migration — domain layer tables**

**Goal:** Add `kg_entities` and `kg_relationships` tables via v24→v25 incremental migration, and update `create_tables_v1()`.

**Requirements:** R1, R2, R3

**Dependencies:** Unit 1 (for constant references)

**Files:**
- Modify: `crates/mika-agent/src/db.rs`

**Approach:**
- Bump `CURRENT_SCHEMA_VERSION` to 25
- Add `migrate_v24_to_v25()` that creates all 10 KG tables in a single `BEGIN IMMEDIATE` transaction
- Update `create_tables_v1()` to include all KG tables
- Update the migration chain `if` guard ranges
- `kg_entities`: INTEGER PK, `entity_key TEXT NOT NULL UNIQUE COLLATE NOCASE`, `entity_type TEXT NOT NULL`, `name TEXT NOT NULL`, `description TEXT`, timestamps. CHECK: `entity_key = entity_type || ':' || name`
- `kg_relationships`: INTEGER PK, `source_entity_id` FK → kg_entities(id) ON DELETE CASCADE, `target_entity_id` FK, `relationship_type TEXT NOT NULL`, `weight REAL`, timestamps
- Indexes: `(entity_type)`, `(source_entity_id, relationship_type)`, `(target_entity_id)`
- Auto-backup before migration (existing pattern)

**Patterns to follow:**
- `migrate_v23_to_v24()` pattern with `BEGIN IMMEDIATE` and `INSERT INTO schema_version`
- `COLLATE NOCASE` on unique text columns
- `timestamp::now()` for default timestamp values

**Test scenarios:**
- Happy path: Fresh in-memory DB creates all KG tables at v25
- Happy path: `CURRENT_SCHEMA_VERSION` equals 25
- Edge case: `entity_key` CHECK constraint rejects mismatched `type:name`
- Edge case: FK cascade — deleting an entity cascades to relationships
- Integration: Column existence checks for key columns

**Verification:**
- `cargo test -p mika-agent` passes with new schema tests

- [x] **Unit 3: Schema v25 migration — lexical layer table**

**Goal:** Add `kg_chunks` table to the v25 migration and `create_tables_v1()`.

**Requirements:** R1, R2, R4

**Dependencies:** Unit 2 (migration transaction)

**Files:**
- Modify: `crates/mika-agent/src/db.rs`

**Approach:**
- `kg_chunks`: INTEGER PK, `agent_id TEXT NOT NULL`, `source_doc_hash TEXT NOT NULL`, `source_url TEXT`, `chunk_index INTEGER NOT NULL`, `content TEXT NOT NULL`, `token_count INTEGER`, timestamps
- UNIQUE constraint on `(agent_id, source_doc_hash, chunk_index)` for dedup
- Index on `agent_id`
- Designed to feed `search_content` pipeline via `source_type='kg_chunk'` (actual insertion into search_content is #687)

**Note:** In practice, Units 2–5 are all part of the same migration function. They are separated here for review clarity but will be implemented as a single `migrate_v24_to_v25()`.

**Test scenarios:**
- Happy path: `kg_chunks` table exists after migration
- Edge case: UNIQUE constraint on `(agent_id, source_doc_hash, chunk_index)` rejects duplicates
- Edge case: `source_doc_hash NOT NULL` rejects null hashes

**Verification:**
- Schema convergence test passes

- [x] **Unit 4: Schema v25 migration — subject layer tables**

**Goal:** Add subject layer tables: `kg_subject_entities`, `kg_subject_resolutions`, `kg_subject_relationships`.

**Requirements:** R1, R2, R5

**Dependencies:** Unit 2 (migration transaction)

**Files:**
- Modify: `crates/mika-agent/src/db.rs`

**Approach:**
- `kg_subject_entities`: INTEGER PK, `agent_id TEXT NOT NULL`, `chunk_id` FK → kg_chunks(id), `mention_text TEXT NOT NULL`, `entity_type TEXT NOT NULL`, `confidence REAL NOT NULL CHECK(confidence >= 0.0 AND confidence <= 1.0)`, timestamps
- `kg_subject_resolutions`: INTEGER PK, `subject_entity_id` FK → kg_subject_entities(id) ON DELETE CASCADE, `resolved_entity_id` FK → kg_entities(id), `resolution_type TEXT NOT NULL CHECK(type IN ('exact', 'alias', 'coreference'))`, `confidence REAL NOT NULL`, timestamps
- `kg_subject_relationships`: INTEGER PK, `agent_id TEXT NOT NULL`, `subject_id` FK → kg_subject_entities(id), `predicate TEXT NOT NULL`, `object_id` FK → kg_subject_entities(id), `confidence REAL NOT NULL`, `source_chunk_id` FK → kg_chunks(id), timestamps

**Test scenarios:**
- Happy path: All three tables exist after migration
- Edge case: confidence CHECK constraint rejects values outside [0.0, 1.0]
- Edge case: FK cascade — deleting a subject entity cascades to resolutions
- Edge case: resolution_type CHECK constraint rejects invalid values

**Verification:**
- Schema tests pass for all subject layer tables

- [x] **Unit 5: Schema v25 migration — provenance and tracking tables**

**Goal:** Add provenance (`kg_chunk_subjects`, `kg_chunk_subject_relationships`) and tracking (`kg_extractions`, `kg_resolutions_log`) tables.

**Requirements:** R1, R2, R6, R7

**Dependencies:** Units 3, 4 (FK references)

**Files:**
- Modify: `crates/mika-agent/src/db.rs`

**Approach:**
- `kg_chunk_subjects`: junction table linking chunks to subject entities
- `kg_chunk_subject_relationships`: junction table linking chunks to subject relationships
- `kg_extractions`: tracks extraction runs (agent_id, source_url, chunk_count, status, timestamps)
- `kg_resolutions_log`: tracks resolution attempts (agent_id, subject_entity_id, outcome, timestamps)
- All per-agent tables include `agent_id TEXT NOT NULL`

**Test scenarios:**
- Happy path: All four tables exist after migration
- Happy path: Extraction tracking records can be inserted
- Edge case: Junction table unique constraints prevent duplicates

**Verification:**
- Full table count in `sqlite_master` matches expected count (existing tables + 10 new)

- [x] **Unit 6: Convergence test and comprehensive schema tests**

**Goal:** Add the convergence test that verifies clean-slate and incremental migration produce identical schemas, plus remaining test scenarios.

**Requirements:** R11

**Dependencies:** Units 2–5

**Files:**
- Modify: `crates/mika-agent/src/db.rs` (test module)

**Approach:**
- Convergence test: create two in-memory DBs — one via `create_tables_v1()` (clean-slate) and one via sequential migration from v24. Compare `sqlite_master` table/index definitions.
- Additional tests: CHECK constraints for entity_key format, confidence bounds, resolution outcome enum; FK cascade tests; UNIQUE constraint tests; column existence tests.
- Target: 12+ new tests total across all KG schema features.

**Patterns to follow:**
- Existing `test_v3_tables_exist` pattern for table counting
- Existing `test_tasks_type_defaults_to_issue` for CHECK constraint testing
- Existing `column_exists()` helper for column presence

**Test scenarios:**
- Integration: Clean-slate DB and incrementally-migrated DB have identical `sqlite_master` entries for KG tables
- Happy path: All 10 KG tables appear in `sqlite_master`
- Edge case: entity_key CHECK constraint (valid format accepted, invalid rejected)
- Edge case: confidence bounds CHECK (0.0 and 1.0 accepted, -0.1 and 1.1 rejected)
- Edge case: FK cascade from kg_entities deletion
- Edge case: UNIQUE constraint on kg_chunks dedup key
- Edge case: resolution_type CHECK constraint

**Verification:**
- `cargo test -p mika-agent` passes with 12+ new KG tests
- Convergence test specifically passes

- [x] **Unit 7: Documentation — KG ID convention and ADR-003 update**

**Goal:** Create `docs/architecture/kg-id-convention.md` and update ADR-003 with KG composition notes.

**Requirements:** R9, R10

**Dependencies:** Units 1–5 (schema must be finalized)

**Files:**
- Create: `docs/architecture/kg-id-convention.md`
- Modify: `docs/adr/003-layer3-hybrid-vector-search.md`

**Approach:**
- KG ID convention doc: describe the typed-prefix `entity_key = type:name` scheme, explain why INTEGER PKs for joins and TEXT keys for identity, document the `format_entity_key` helper.
- ADR-003 update: add a section on KG composition — how `kg_chunks` feed into `search_content` via `source_type='kg_chunk'`, maintaining the existing FTS5+vec pipeline without duplication.

**Test expectation:** none — documentation only

**Verification:**
- Files exist and are well-formed markdown
- ADR-003 update is additive (does not remove existing content)

## System-Wide Impact

- **Interaction graph:** New tables are schema-only — no callbacks, middleware, or tool registrations. Future issues (#687–#692) will add the tool and pipeline interactions.
- **Error propagation:** Migration errors follow existing pattern — fail before agent startup, auto-backup preserves data.
- **State lifecycle risks:** None — no write paths are added to the KG tables in this PR.
- **API surface parity:** No API changes.
- **Integration coverage:** Convergence test is the critical cross-layer test — ensures clean-slate and incremental paths agree.
- **Unchanged invariants:** Existing `search_content` pipeline, all v24 tables, and the FTS5/vec search system are untouched. The KG tables are additive.

## Risks & Dependencies

| Risk | Mitigation |
|------|------------|
| Large `db.rs` file grows further (~200 lines of DDL) | KG constants/helpers extracted to `db/kg_schema.rs`; DDL stays in `db.rs` for consistency |
| `entity_key` CHECK constraint may not support `=` with concatenation in all SQLite versions | Use `entity_key = entity_type || ':' || name` which is standard SQL; test in CI |
| Toggle_skill fix could break other tests if `ToolContext` fields change | Minimal change — only two tests affected, following established `send_message.rs` pattern |

## Sources & References

- Related issues: #722, #687, #688, #689, #690, #691, #692, milestone #14
- ADR: `docs/adr/003-layer3-hybrid-vector-search.md`
- Learnings: `docs/solutions/database-issues/iso8601-timestamp-migration.md`, `docs/solutions/database-issues/skill-override-persistence-via-db-layer.md`
