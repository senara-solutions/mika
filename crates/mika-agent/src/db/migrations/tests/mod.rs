//! Tests de l'échelle de migrations (mika#2321).
//!
//! Ils vivent ici, et non dans `db::tests`, parce que les `migrate_vN_to_vM`
//! sont **privées** à `db::migrations` : un enfant voit les items privés de son
//! parent, deux frères ne se voient pas. C'est la règle D3 du plan — *un module
//! de test voyage avec le code qu'il teste* — et c'est elle qui remplace une
//! trentaine d'élargissements de signature par aucun.
//!
//! La règle a une portée plus large que les tests de `db` : elle a aussi
//! rapatrié ici `migration_v38_to_v39_idempotent`, qui vivait dans le `mod
//! tests` de `db/operational.rs` — un module *frère*, donc aveugle aux mêmes
//! privés.
//!
//! Les helpers descendus avec eux (`db_at_v46`, le harnais de snapshot de
//! schéma KG) n'avaient aucun autre lecteur : les laisser en amont aurait
//! demandé de les rendre `pub(crate)`, c'est-à-dire de payer en visibilité ce
//! que le placement donne gratuitement.

use super::*;
use crate::db::tests::db;

mod kg_schema;

/// Rapatrié de `db::operational::tests` (mika#2321 D3) : `migrate_v38_to_v39`
/// est privée à `db::migrations`, et `db::operational` en est un frère.
#[test]
fn migration_v38_to_v39_idempotent() {
    // This is tested by the convergence test, but let's also verify
    // the idempotency of the migration function directly.
    let mut db = db();
    // DB is already at v39, calling migrate again should be a no-op
    db.migrate_v38_to_v39().unwrap();
}

// ===== KG Schema Migration Tests (v24 → v25 forward-test harness) =====

/// A structural fingerprint of a SQLite table, including columns, indexes,
/// and foreign keys. Used for migration convergence testing.
#[derive(Debug, Clone, PartialEq, Eq)]
struct TableSnapshot {
    name: String,
    columns: Vec<ColumnInfo>,
    indexes: Vec<IndexInfo>,
    foreign_keys: Vec<ForeignKeyInfo>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct ColumnInfo {
    name: String,
    col_type: String,
    not_null: bool,
    default_value: Option<String>,
    pk: i32,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct IndexInfo {
    name: String,
    unique: bool,
    columns: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct ForeignKeyInfo {
    from_col: String,
    to_table: String,
    to_col: String,
    on_delete: String,
}

/// Snapshot the full structural schema of a database for comparison.
fn snapshot_schema(conn: &rusqlite::Connection) -> Vec<TableSnapshot> {
    let mut stmt = conn
        .prepare("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name")
        .unwrap();
    let table_names: Vec<String> = stmt
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(|r| r.unwrap())
        .collect();

    let mut snapshots = Vec::new();
    for name in table_names {
        // Skip virtual tables (fts_search, vec_search) — they don't support PRAGMA introspection
        // Skip v26 backup tables left by migrate_v26_to_v27 (pending #787 coalesce)
        if name == "fts_search"
            || name == "vec_search"
            || name.starts_with("fts_search_")
            || name.starts_with("vec_search_")
            || name.ends_with("_v26_backup")
        {
            continue;
        }

        let mut columns: Vec<ColumnInfo> = {
            let mut s = conn
                .prepare(&format!("PRAGMA table_info('{name}')"))
                .unwrap();
            s.query_map([], |r| {
                Ok(ColumnInfo {
                    name: r.get(1)?,
                    col_type: r.get(2)?,
                    not_null: r.get::<_, bool>(3)?,
                    default_value: r.get(4)?,
                    pk: r.get(5)?,
                })
            })
            .unwrap()
            .map(|r| r.unwrap())
            .collect()
        };
        columns.sort();

        let mut indexes: Vec<IndexInfo> = {
            let mut s = conn
                .prepare(&format!("PRAGMA index_list('{name}')"))
                .unwrap();
            let raw: Vec<(String, bool)> = s
                .query_map([], |r| Ok((r.get::<_, String>(1)?, r.get::<_, bool>(2)?)))
                .unwrap()
                .map(|r| r.unwrap())
                .collect();
            raw.into_iter()
                .map(|(idx_name, unique)| {
                    let mut si = conn
                        .prepare(&format!("PRAGMA index_info('{idx_name}')"))
                        .unwrap();
                    let cols: Vec<String> = si
                        .query_map([], |r| r.get(2))
                        .unwrap()
                        .map(|r| r.unwrap())
                        .collect();
                    IndexInfo {
                        name: idx_name,
                        unique,
                        columns: cols,
                    }
                })
                .collect()
        };
        indexes.sort();

        let mut foreign_keys: Vec<ForeignKeyInfo> = {
            let mut s = conn
                .prepare(&format!("PRAGMA foreign_key_list('{name}')"))
                .unwrap();
            s.query_map([], |r| {
                Ok(ForeignKeyInfo {
                    from_col: r.get(3)?,
                    to_table: r.get(2)?,
                    to_col: r.get(4)?,
                    on_delete: r.get(6)?,
                })
            })
            .unwrap()
            .map(|r| r.unwrap())
            .collect()
        };
        foreign_keys.sort();

        snapshots.push(TableSnapshot {
            name,
            columns,
            indexes,
            foreign_keys,
        });
    }
    snapshots.sort_by(|a, b| a.name.cmp(&b.name));
    snapshots
}

/// All ten KG tables added in v25.
const KG_TABLES: &[&str] = &[
    "kg_entities",
    "kg_relationships",
    "kg_chunks",
    "kg_subject_entities",
    "kg_subject_resolutions",
    "kg_subject_relationships",
    "kg_chunk_subjects",
    "kg_chunk_subject_relationships",
    "kg_extractions",
    "kg_resolutions_log",
];

fn table_exists(conn: &rusqlite::Connection, table: &str) -> bool {
    conn.query_row(
        "SELECT count(*) FROM sqlite_master WHERE type='table' AND name=?1",
        [table],
        |r| r.get::<_, i64>(0),
    )
    .unwrap()
        > 0
}

fn index_exists(conn: &rusqlite::Connection, index: &str) -> bool {
    conn.query_row(
        "SELECT count(*) FROM sqlite_master WHERE type='index' AND name=?1",
        [index],
        |r| r.get::<_, i64>(0),
    )
    .unwrap()
        > 0
}

// ------------------------------------------------------------------
// mika#1676 — v46→v47: team_runs delegation-visibility (Unit B + Unit A
// terminal state). The migration is a CHECK-expansion table-rebuild
// following the v34→v35 kg_resolutions_log precedent, plus three
// additive columns for observability. Tests below assert the migration
// (a) applies cleanly on a v46 DB seeded with rows, (b) preserves
// row-count and per-row values, (c) accepts inserts with the new
// `failed_no_delegation` status, (d) accepts the pre-existing status
// set unchanged, and (e) round-trips the three new columns.
//
// A sixth test (`test_migrate_v46_to_v47_is_idempotent`) proves the
// early-return guard so a crash-recovery re-run of the migration chain
// does not corrupt the rebuilt table.
//
// Re-slot note (post-rebase 2026-08-22): originally landed as v45→v46
// (mika#1676 pre-conflict). After main merged mika#1867 (`served_content`
// ledger) into the v46 slot first, this migration was moved to v47 to
// preserve the linear history. Migration semantics unchanged.
// ------------------------------------------------------------------

/// Build a fresh in-memory DB, rewind schema_version to 46, and
/// clean-slate-DDL-drop the delegation columns so `migrate_v46_to_v47`
/// exercises the real ALTER path. `db()` runs the full migration chain
/// (up to `CURRENT_SCHEMA_VERSION` = 48 after the mika#1712 v47→v48
/// behavioral-marker re-slot), so we then rebuild `team_runs` with the
/// pre-v47 shape (without the three new columns and without the expanded
/// CHECK constraint) to reproduce a v46 database exactly.
fn db_at_v46() -> Database {
    let db = db();
    db.conn
        .execute_batch(
            "PRAGMA foreign_keys = OFF;
                 DROP TABLE IF EXISTS team_runs;
                 CREATE TABLE team_runs (
                     id TEXT PRIMARY KEY,
                     team_id TEXT NOT NULL REFERENCES teams(id),
                     goal TEXT NOT NULL,
                     status TEXT NOT NULL DEFAULT 'running'
                         CHECK (status IN ('running','completed','failed','cancelled','suspended')),
                     failure_reason TEXT,
                     iteration INTEGER NOT NULL DEFAULT 1,
                     max_iterations INTEGER NOT NULL DEFAULT 3,
                     deliverable TEXT,
                     checkpoint TEXT,
                     trace_id TEXT,
                     started_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                     ended_at TEXT
                 );
                 CREATE INDEX idx_team_runs_team ON team_runs(team_id, started_at DESC);
                 DELETE FROM schema_version WHERE version > 46;
                 PRAGMA foreign_keys = ON;",
        )
        .unwrap();
    db
}

#[test]
fn test_migrate_v50_to_v51_is_idempotent() {
    let mut db = db();
    // Already at CURRENT (>=51) — must no-op rather than double-apply.
    db.migrate_v50_to_v51().unwrap();
    db.migrate_v50_to_v51().unwrap();
    let version: i64 = db
        .conn
        .query_row("SELECT MAX(version) FROM schema_version", [], |r| r.get(0))
        .unwrap();
    assert_eq!(version, CURRENT_SCHEMA_VERSION);
}

#[test]
fn test_migrate_v51_to_v52_is_idempotent() {
    let mut db = db();
    // Already at CURRENT (>=52) — must no-op rather than double-apply.
    db.migrate_v51_to_v52().unwrap();
    db.migrate_v51_to_v52().unwrap();
    let version: i64 = db
        .conn
        .query_row("SELECT MAX(version) FROM schema_version", [], |r| r.get(0))
        .unwrap();
    assert_eq!(version, CURRENT_SCHEMA_VERSION);
}

#[test]
fn test_migrate_v52_to_v53_is_idempotent() {
    let mut db = db();
    // Already at CURRENT (>=53) — must no-op rather than double-apply.
    db.migrate_v52_to_v53().unwrap();
    db.migrate_v52_to_v53().unwrap();
    let version: i64 = db
        .conn
        .query_row("SELECT MAX(version) FROM schema_version", [], |r| r.get(0))
        .unwrap();
    assert_eq!(version, CURRENT_SCHEMA_VERSION);
}

/// The ALTER path itself, exercised from a v52-shaped `llm_calls`.
///
/// The idempotence test above runs against an already-migrated database, so
/// it proves the guard and never the migration. This drops the column and
/// rewinds the version so the `ALTER TABLE` actually executes — otherwise a
/// broken statement would ship green.
#[test]
fn migrate_v52_to_v53_adds_the_column_and_preserves_rows() {
    let mut db = db();
    db.conn
        .execute("ALTER TABLE llm_calls DROP COLUMN request_bytes", [])
        .unwrap();
    db.conn
        .execute(
            "INSERT INTO llm_calls (id, agent_id, session_id, provider, model)
                 VALUES ('pre-v53', 'a', 's', 'openrouter', 'kimi')",
            [],
        )
        .unwrap();
    db.conn.execute("DELETE FROM schema_version", []).unwrap();
    db.conn
        .execute("INSERT INTO schema_version (version) VALUES (52)", [])
        .unwrap();

    db.migrate_v52_to_v53().unwrap();

    assert!(db.column_exists("llm_calls", "request_bytes").unwrap());
    let carried: Option<i64> = db
        .conn
        .query_row(
            "SELECT request_bytes FROM llm_calls WHERE id = 'pre-v53'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        carried, None,
        "a pre-v53 row must read NULL — the column is not retroactive, and a \
             default of 0 would be indistinguishable from a genuinely empty request"
    );
}

#[test]
fn test_migrate_v51_to_v52_bails_on_unexpected_baseline() {
    let mut db = db();
    db.conn.execute("DELETE FROM schema_version", []).unwrap();
    db.conn
        .execute("INSERT INTO schema_version (version) VALUES (49)", [])
        .unwrap();
    let err = db.migrate_v51_to_v52().unwrap_err();
    assert!(
        err.to_string().contains("unexpected baseline version 49"),
        "migration must name the baseline it refused — got {err}"
    );
}

#[test]
fn test_migrate_v50_to_v51_bails_on_unexpected_baseline() {
    let mut db = db();
    // Forge a baseline the migration must refuse: below 50, so the
    // idempotency guard does not return, but not the expected 50 either.
    db.conn.execute("DELETE FROM schema_version", []).unwrap();
    db.conn
        .execute("INSERT INTO schema_version (version) VALUES (48)", [])
        .unwrap();
    let err = db.migrate_v50_to_v51().unwrap_err();
    assert!(
        err.to_string().contains("unexpected baseline version 48"),
        "bail must name the offending baseline — got: {err}"
    );
}

#[test]
fn test_migrate_v49_to_v50_alters_a_real_v49_table_and_preserves_rows() {
    // The idempotence test below runs against a fresh DB, which the v1
    // inline schema already builds at v50 — so it exercises the no-op
    // branch, not the ALTER. This one builds the actual v49 shape, puts a
    // row in it, and proves the upgrade path keeps existing circuit-breaker
    // state while defaulting the new counters.
    let mut db = db();
    db.conn
        .execute_batch(
            "DROP TABLE auto_pull_stats;
                 CREATE TABLE auto_pull_stats (
                     repo_full_name TEXT NOT NULL,
                     issue_number INTEGER NOT NULL,
                     failure_count INTEGER NOT NULL DEFAULT 0,
                     last_auto_pull_at TEXT,
                     last_failure_at TEXT,
                     PRIMARY KEY (repo_full_name, issue_number)
                 );
                 INSERT INTO auto_pull_stats
                     (repo_full_name, issue_number, failure_count, last_auto_pull_at)
                 VALUES ('senara-solutions/mika', 1901, 2, '2026-08-29T00:00:00Z');
                 DELETE FROM schema_version;
                 INSERT INTO schema_version (version) VALUES (49);",
        )
        .unwrap();
    assert_eq!(db.schema_version().unwrap(), 49);
    assert!(
        !db.column_exists("auto_pull_stats", "redrive_count")
            .unwrap(),
        "precondition: the v49 shape has no re-drive columns"
    );

    db.migrate_v49_to_v50().unwrap();

    assert_eq!(db.schema_version().unwrap(), 50);
    assert_eq!(
        db.get_auto_pull_failure_count("senara-solutions/mika", 1901)
            .unwrap(),
        2,
        "existing circuit-breaker state survives the upgrade"
    );
    assert_eq!(
        db.get_auto_pull_redrive_state("senara-solutions/mika", 1901)
            .unwrap(),
        (0, false),
        "a pre-existing row starts with a clean re-drive budget"
    );

    // Second call must recognise v50 and no-op rather than re-ALTER.
    db.migrate_v49_to_v50().unwrap();
    assert_eq!(db.schema_version().unwrap(), 50);
}

#[test]
fn test_migrate_v49_to_v50_is_idempotent() {
    let mut db = db();
    // A fresh DB is already at v50 via the v1 inline schema; the migration
    // must recognise that and no-op rather than re-ALTER.
    db.migrate_v49_to_v50().unwrap();
    db.migrate_v49_to_v50().unwrap();
    assert_eq!(db.schema_version().unwrap(), CURRENT_SCHEMA_VERSION);
    assert!(
        db.column_exists("auto_pull_stats", "redrive_count")
            .unwrap()
    );
    assert!(
        db.column_exists("auto_pull_stats", "last_redrive_at")
            .unwrap()
    );
    assert!(
        db.column_exists("auto_pull_stats", "redrive_abandoned_at")
            .unwrap()
    );
}

#[test]
fn test_migrate_v28_to_v29_scrubs_existing_rows() {
    let mut db = db();

    // Temporarily set schema to 28 so migration will run
    db.conn
        .execute("UPDATE schema_version SET version = 28", [])
        .unwrap();

    // Insert a row with a secret directly (bypassing the scrubber via raw SQL)
    db.conn
        .execute(
            "INSERT INTO tool_calls (id, agent_id, session_id, step, tool_name, tool_source, input, output, success, non_zero_exit, latency_ms)
                 VALUES ('old-1', 'mika', 'test-session', 0, 'read_agent_file', 'builtin', '{\"path\":\".env\"}', 'MIKA_GITHUB_TOKEN=github_pat_11CBQ5ABC1234567890abcdef', 1, 0, 100)",
            [],
        )
        .unwrap();

    // Insert a clean row that should not be modified
    db.conn
        .execute(
            "INSERT INTO tool_calls (id, agent_id, session_id, step, tool_name, tool_source, input, output, success, non_zero_exit, latency_ms)
                 VALUES ('old-2', 'mika', 'test-session', 1, 'search_memory', 'builtin', '{\"query\":\"test\"}', 'No results found', 1, 0, 50)",
            [],
        )
        .unwrap();

    // Run migration
    db.migrate_v28_to_v29().unwrap();

    // Verify version bumped
    assert_eq!(db.schema_version().unwrap(), 29);

    // Verify secret row was scrubbed
    let output: String = db
        .conn
        .query_row(
            "SELECT output FROM tool_calls WHERE id = 'old-1'",
            [],
            |row: &rusqlite::Row| row.get(0),
        )
        .unwrap();
    assert!(
        !output.contains("github_pat_11CBQ5"),
        "migration should have scrubbed secret: {output}"
    );
    assert!(
        output.contains("<REDACTED>"),
        "migration should have redacted: {output}"
    );

    // Verify clean row was unchanged
    let clean_output: String = db
        .conn
        .query_row(
            "SELECT output FROM tool_calls WHERE id = 'old-2'",
            [],
            |row: &rusqlite::Row| row.get(0),
        )
        .unwrap();
    assert_eq!(clean_output, "No results found");
}

#[test]
fn test_migrate_v28_to_v29_idempotent() {
    let mut db = db();
    // DB is already at CURRENT_SCHEMA_VERSION (db() creates at latest)
    // Running migration again should be a no-op
    db.migrate_v28_to_v29().unwrap();
    assert_eq!(db.schema_version().unwrap(), CURRENT_SCHEMA_VERSION);
}

#[test]
fn test_migrate_v29_to_v30_expands_check_constraint() {
    let mut db = db();

    // Seed subject entities so FK constraints are satisfied.
    // The db() helper creates fresh schema with kg_subject_entities table.
    db.conn
        .execute_batch(
            "INSERT INTO kg_subject_entities (id, docs_root_hash, docs_root, entity_key, type, name, confidence, trace_id)
                 VALUES (1, 'abcd1234', '/docs', 'skill:test1', 'skill', 'test1', 0.9, 'trace-1');
                 INSERT INTO kg_subject_entities (id, docs_root_hash, docs_root, entity_key, type, name, confidence, trace_id)
                 VALUES (2, 'abcd1234', '/docs', 'skill:test2', 'skill', 'test2', 0.9, 'trace-2');
                 INSERT INTO kg_subject_entities (id, docs_root_hash, docs_root, entity_key, type, name, confidence, trace_id)
                 VALUES (3, 'abcd1234', '/docs', 'skill:test3', 'skill', 'test3', 0.9, 'trace-3');",
        )
        .unwrap();

    // Temporarily set schema to 29 so migration will run.
    // We need to rebuild the table with the old CHECK constraint first.
    db.conn
        .execute_batch(
            "PRAGMA foreign_keys = OFF;
                 DROP INDEX IF EXISTS idx_kg_res_log_pending;
                 ALTER TABLE kg_resolutions_log RENAME TO kg_resolutions_log_old;
                 CREATE TABLE kg_resolutions_log (
                     id INTEGER PRIMARY KEY AUTOINCREMENT,
                     agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                     subject_entity_id INTEGER NOT NULL REFERENCES kg_subject_entities(id) ON DELETE CASCADE,
                     outcome TEXT NOT NULL CHECK (outcome IN (
                         'matched_exact', 'matched_llm', 'no_match', 'skipped_discovered_type', 'skipped_no_llm', 'error'
                     )),
                     resolution_trace_id TEXT NOT NULL,
                     source_extraction_trace_id TEXT,
                     model TEXT,
                     duration_ms INTEGER,
                     resolved_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                     UNIQUE (agent_id, subject_entity_id)
                 );
                 INSERT INTO kg_resolutions_log SELECT * FROM kg_resolutions_log_old;
                 DROP TABLE kg_resolutions_log_old;
                 CREATE INDEX idx_kg_res_log_pending ON kg_resolutions_log(agent_id, outcome);
                 PRAGMA foreign_keys = ON;
                 UPDATE schema_version SET version = 29;",
        )
        .unwrap();

    // Verify matched_llm_db_fallback is REJECTED before migration.
    let insert_result = db.conn.execute(
        "INSERT INTO kg_resolutions_log (agent_id, subject_entity_id, outcome, resolution_trace_id) \
             VALUES ('mika', 1, 'matched_llm_db_fallback', 'trace-pre')",
        [],
    );
    assert!(
        insert_result.is_err(),
        "v29 schema should reject matched_llm_db_fallback"
    );

    // Seed a row with an existing outcome to verify preservation.
    db.conn
        .execute(
            "INSERT INTO kg_resolutions_log (agent_id, subject_entity_id, outcome, resolution_trace_id) \
                 VALUES ('mika', 1, 'matched_exact', 'trace-seed')",
            [],
        )
        .unwrap();

    let count_before: i64 = db
        .conn
        .query_row("SELECT COUNT(*) FROM kg_resolutions_log", [], |r| r.get(0))
        .unwrap();

    // Run migration.
    db.migrate_v29_to_v30().unwrap();

    // Verify version bumped.
    assert_eq!(db.schema_version().unwrap(), 30);

    // Verify row count preserved.
    let count_after: i64 = db
        .conn
        .query_row("SELECT COUNT(*) FROM kg_resolutions_log", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count_before, count_after, "row count should be preserved");

    // Verify matched_llm_db_fallback is ACCEPTED after migration.
    // Use a different subject_entity_id to avoid UNIQUE violation.
    let insert_result = db.conn.execute(
        "INSERT INTO kg_resolutions_log (agent_id, subject_entity_id, outcome, resolution_trace_id) \
             VALUES ('mika', 2, 'matched_llm_db_fallback', 'trace-post')",
        [],
    );
    assert!(
        insert_result.is_ok(),
        "v30 schema should accept matched_llm_db_fallback: {:?}",
        insert_result.err()
    );

    // Verify invalid values still rejected.
    let invalid_result = db.conn.execute(
        "INSERT INTO kg_resolutions_log (agent_id, subject_entity_id, outcome, resolution_trace_id) \
             VALUES ('mika', 3, 'invalid_value', 'trace-invalid')",
        [],
    );
    assert!(
        invalid_result.is_err(),
        "invalid outcome should still be rejected"
    );
}

#[test]
fn test_migrate_v29_to_v30_idempotent() {
    let mut db = db();
    // DB is already at CURRENT_SCHEMA_VERSION (db() creates at latest)
    // Running migration again should be a no-op
    db.migrate_v29_to_v30().unwrap();
    assert_eq!(db.schema_version().unwrap(), CURRENT_SCHEMA_VERSION);
}

/// AC2 / test-coverage-mandatory line 1: v46→v47 migration applies cleanly
/// on a seeded v46 database, bumps the schema version, and lands the
/// three new columns with their DEFAULT values on pre-existing rows.
#[test]
fn test_migrate_v46_to_v47_applies_cleanly() {
    let mut db = db_at_v46();
    db.register_agent("mika", "Mika", "").unwrap();
    db.conn
        .execute(
            "INSERT INTO teams (id, name, config_path) VALUES ('t-1', 'alpha', '')",
            [],
        )
        .unwrap();
    db.conn
        .execute(
            "INSERT INTO team_runs (id, team_id, goal, status, iteration, max_iterations, started_at) \
                 VALUES ('r-1', 't-1', 'g', 'completed', 1, 3, '2026-01-01T00:00:00Z')",
            [],
        )
        .unwrap();

    db.migrate_v46_to_v47().unwrap();

    let version: i64 = db
        .conn
        .query_row("SELECT MAX(version) FROM schema_version", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        version, 47,
        "schema_version must reach 47 after migrate_v46_to_v47"
    );

    let (delegation_count, solo_absorption, failure_context): (i64, i64, Option<String>) = db
        .conn
        .query_row(
            "SELECT delegation_count, solo_absorption, failure_context FROM team_runs WHERE id = 'r-1'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!(
        delegation_count, 0,
        "delegation_count DEFAULT must backfill to 0"
    );
    assert_eq!(
        solo_absorption, 0,
        "solo_absorption DEFAULT must backfill to 0"
    );
    assert!(
        failure_context.is_none(),
        "failure_context must backfill to NULL"
    );
}

/// AC2 / test-coverage-mandatory line 2: the v46→v47 rebuild preserves
/// row count and per-row values (id/team_id/goal/status/iteration/…) so
/// no team-run history is lost across the migration.
#[test]
fn test_migrate_v46_to_v47_preserves_row_data() {
    let mut db = db_at_v46();
    db.register_agent("mika", "Mika", "").unwrap();
    db.conn
        .execute(
            "INSERT INTO teams (id, name, config_path) VALUES ('t-1', 'alpha', '')",
            [],
        )
        .unwrap();
    db.conn
        .execute_batch(
            "INSERT INTO team_runs (id, team_id, goal, status, failure_reason, \
                     iteration, max_iterations, deliverable, checkpoint, trace_id, \
                     started_at, ended_at) \
                 VALUES ('r-1', 't-1', 'goal A', 'completed', NULL, 2, 3, 'D', NULL, 'tr-1', \
                     '2026-01-01T00:00:00Z', '2026-01-01T00:01:00Z'),
                        ('r-2', 't-1', 'goal B', 'failed', 'boom', 3, 3, NULL, NULL, 'tr-2', \
                     '2026-01-02T00:00:00Z', '2026-01-02T00:02:00Z'),
                        ('r-3', 't-1', 'goal C', 'cancelled', NULL, 1, 3, NULL, NULL, NULL, \
                     '2026-01-03T00:00:00Z', NULL);",
        )
        .unwrap();

    let count_before: i64 = db
        .conn
        .query_row("SELECT COUNT(*) FROM team_runs", [], |r| r.get(0))
        .unwrap();

    db.migrate_v46_to_v47().unwrap();

    let count_after: i64 = db
        .conn
        .query_row("SELECT COUNT(*) FROM team_runs", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        count_before, count_after,
        "row count MUST be preserved through the table-rebuild migration"
    );

    // Spot-check every column on the middle row — the migration must
    // preserve NULLs (failure_reason on r-1) and non-NULL values alike.
    let (goal, status, failure_reason, iteration, deliverable, trace_id): (
        String,
        String,
        Option<String>,
        i64,
        Option<String>,
        Option<String>,
    ) = db
        .conn
        .query_row(
            "SELECT goal, status, failure_reason, iteration, deliverable, trace_id \
                 FROM team_runs WHERE id = 'r-2'",
            [],
            |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                ))
            },
        )
        .unwrap();
    assert_eq!(goal, "goal B");
    assert_eq!(status, "failed");
    assert_eq!(failure_reason.as_deref(), Some("boom"));
    assert_eq!(iteration, 3);
    assert!(deliverable.is_none());
    assert_eq!(trace_id.as_deref(), Some("tr-2"));

    // The idx_team_runs_team index MUST be recreated (dropped as a
    // side effect of the RENAME/DROP) — otherwise queries against
    // team_runs by (team_id, started_at) fall back to a table scan.
    let idx_exists: i64 = db
        .conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master \
                 WHERE type = 'index' AND name = 'idx_team_runs_team'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        idx_exists, 1,
        "idx_team_runs_team MUST be recreated after the table-rebuild"
    );

    // FK integrity — the load-bearing invariant that motivated the
    // CREATE-INSERT-DROP-RENAME sequence in the migration (see the
    // rationale comment on migrate_v46_to_v47 for why the naïve
    // RENAME-first sequence would silently retarget tasks.team_run_id
    // to a nonexistent table). First assert the FK still names
    // `team_runs` (schema-level check), then prove writes actually
    // work end-to-end.
    db.register_agent("mika", "Mika", "").unwrap();
    let fk_target: String = db
        .conn
        .query_row(
            "SELECT \"table\" FROM pragma_foreign_key_list('tasks') \
                 WHERE \"from\" = 'team_run_id'",
            [],
            |r| r.get(0),
        )
        .expect("tasks.team_run_id FK must exist");
    assert_eq!(
        fk_target, "team_runs",
        "tasks.team_run_id must still target team_runs after rebuild — \
             a `team_runs_new` or backup target here means the migration \
             regressed to the RENAME-first sequence"
    );
    db.conn
        .execute(
            "INSERT INTO tasks (id, agent_id, team_run_id, label, trigger_type, action_type, status) \
                 VALUES ('task-1', 'mika', 'r-1', 'noop', 'manual', 'none', 'pending')",
            [],
        )
        .expect("INSERT INTO tasks with valid team_run_id FK must succeed post-rebuild");
}

/// AC2 / test-coverage-mandatory line 3: the expanded CHECK constraint
/// accepts the new `'failed_no_delegation'` status, and the old status
/// set is still accepted verbatim (no CHECK regression). This is the
/// load-bearing assertion the plan (§ Migration pattern) demands:
/// widening the CHECK is the whole point of table-rebuild vs additive-
/// ALTER, and the invariant is only meaningful when both halves hold.
#[test]
fn test_migrate_v46_to_v47_check_expansion_accepts_new_and_old_statuses() {
    let mut db = db_at_v46();
    db.register_agent("mika", "Mika", "").unwrap();
    db.conn
        .execute(
            "INSERT INTO teams (id, name, config_path) VALUES ('t-1', 'alpha', '')",
            [],
        )
        .unwrap();

    db.migrate_v46_to_v47().unwrap();

    // New status: MUST be accepted after v47.
    db.conn
        .execute(
            "INSERT INTO team_runs (id, team_id, goal, status, iteration, max_iterations, started_at) \
                 VALUES ('r-new', 't-1', 'g', 'failed_no_delegation', 1, 3, '2026-01-01T00:00:00Z')",
            [],
        )
        .expect("v47 CHECK must accept 'failed_no_delegation'");

    // All pre-v47 statuses: MUST still be accepted (no regression).
    for status in &["running", "completed", "failed", "cancelled", "suspended"] {
        let id = format!("r-old-{status}");
        db.conn
            .execute(
                "INSERT INTO team_runs (id, team_id, goal, status, iteration, max_iterations, started_at) \
                     VALUES (?1, 't-1', 'g', ?2, 1, 3, '2026-01-01T00:00:00Z')",
                params![id, status],
            )
            .unwrap_or_else(|e| panic!("v47 CHECK must accept legacy status '{status}': {e}"));
    }

    // A genuinely-invalid status MUST still be rejected (defense-in-depth
    // — proves the CHECK is present and enforcing, not vacuously removed).
    let bogus = db.conn.execute(
        "INSERT INTO team_runs (id, team_id, goal, status, iteration, max_iterations, started_at) \
             VALUES ('r-bogus', 't-1', 'g', 'not_a_real_status', 1, 3, '2026-01-01T00:00:00Z')",
        [],
    );
    assert!(
        bogus.is_err(),
        "v47 CHECK must still reject arbitrary strings; got Ok — CHECK missing?"
    );
}

/// The migration MUST be idempotent so a crash-recovery re-run of the
/// full chain (see `migrate()`'s `if (3..=46).contains(&version)`
/// guard) does not corrupt the rebuilt table. Confirms the early-return
/// `if version >= 47 { return Ok(()); }` protects a second application.
#[test]
fn test_migrate_v46_to_v47_is_idempotent() {
    let mut db = db_at_v46();
    db.register_agent("mika", "Mika", "").unwrap();
    db.conn
        .execute(
            "INSERT INTO teams (id, name, config_path) VALUES ('t-1', 'alpha', '')",
            [],
        )
        .unwrap();
    db.conn
        .execute(
            "INSERT INTO team_runs (id, team_id, goal, status, iteration, max_iterations, started_at) \
                 VALUES ('r-1', 't-1', 'g', 'completed', 1, 3, '2026-01-01T00:00:00Z')",
            [],
        )
        .unwrap();

    db.migrate_v46_to_v47().unwrap();
    // Second application MUST be a no-op — the early-return guard fires.
    db.migrate_v46_to_v47().unwrap();

    let count: i64 = db
        .conn
        .query_row("SELECT COUNT(*) FROM team_runs", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        count, 1,
        "idempotent re-run must not lose or duplicate rows"
    );
}
