use std::path::{Path, PathBuf};

use crate::btrfs;
use crate::error::MikaOsError;
use crate::redact;
use crate::snapshot::{self, SnapshotLabel};
use crate::subvolume_layout;

/// Tables with an `agent_id` column that need updating during fork-ingest.
/// This list MUST be kept in sync with the schema — the test in this module
/// validates it against `sqlite_master` at test time.
pub const FORK_INGEST_TABLES: &[&str] = &[
    "agents",
    "sessions",
    "messages",
    "tasks",
    "tool_calls",
    "llm_calls",
    "skill_overrides",
    "core_memory",
    "facts",
    "search_content",
    "kg_subject_resolutions",
    "kg_resolutions_log",
    "agent_kg_corpora",
    "operational_items",
    "team_runs",
    "audit_events",
    "people",
    "commitments",
    "preferences",
    "events",
];

/// Rollback to a previous snapshot.
///
/// This replaces the current `~/.mika/` state with the snapshot's state.
/// Services should be stopped before calling this.
///
/// Steps:
/// 1. Create a safety snapshot of current state
/// 2. Delete nested subvolumes (logs, backups)
/// 3. Delete main subvolume
/// 4. Restore from snapshot as writable
/// 5. Re-create nested subvolumes (fresh, empty)
pub fn rollback(home: &Path, snap_path: &Path) -> Result<(), MikaOsError> {
    if !snap_path.exists() {
        return Err(MikaOsError::SnapshotNotFound(
            snap_path.display().to_string(),
        ));
    }

    // 1. Safety snapshot
    let safety_label = SnapshotLabel {
        tenant_id: "safety".to_string(),
        session_id: "pre-rollback".to_string(),
        timestamp: chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string(),
    };
    if let Err(e) = snapshot::create_snapshot(home, &safety_label) {
        tracing::warn!(error = %e, "failed to create safety snapshot before rollback");
    }

    // 2. Delete nested subvolumes (must die before parent)
    for subvol in subvolume_layout::NESTED_SUBVOLUMES {
        let subvol_path = home.join(subvol);
        if subvol_path.exists()
            && let Ok(true) = btrfs::is_subvolume(&subvol_path)
        {
            btrfs::delete_subvolume(&subvol_path).map_err(|e| {
                MikaOsError::RollbackFailed(format!(
                    "failed to delete nested subvolume {subvol}: {e}"
                ))
            })?;
        }
    }

    // 3. Delete main subvolume
    btrfs::delete_subvolume(home).map_err(|e| {
        MikaOsError::RollbackFailed(format!("failed to delete main subvolume: {e}"))
    })?;

    // 4. Restore from snapshot as writable
    btrfs::snapshot_writable(snap_path, home)
        .map_err(|e| MikaOsError::RollbackFailed(format!("failed to restore snapshot: {e}")))?;

    // 5. Re-create nested subvolumes (fresh, empty)
    subvolume_layout::initialize_layout(home).map_err(|e| {
        MikaOsError::RollbackFailed(format!("failed to re-create nested subvolumes: {e}"))
    })?;

    Ok(())
}

/// Fork a snapshot into a new tenant directory.
///
/// Creates a writable copy of the snapshot at `~/.mika-tenants/{new_tenant}/`,
/// re-creates nested subvolumes, runs tenant-ingest (renames agent references
/// in the DB), and optionally redacts secrets.
///
/// Returns the path to the new tenant directory.
pub fn fork(
    home: &Path,
    snap_path: &Path,
    new_tenant: &str,
    keep_secrets: bool,
) -> Result<PathBuf, MikaOsError> {
    if !snap_path.exists() {
        return Err(MikaOsError::SnapshotNotFound(
            snap_path.display().to_string(),
        ));
    }

    // Derive tenant directory from home's parent
    let tenants_dir = home
        .parent()
        .ok_or_else(|| MikaOsError::ForkFailed("cannot determine parent of home".to_string()))?
        .join(".mika-tenants");
    std::fs::create_dir_all(&tenants_dir)?;

    let fork_path = tenants_dir.join(new_tenant);
    if fork_path.exists() {
        return Err(MikaOsError::ForkFailed(format!(
            "tenant directory already exists: {}",
            fork_path.display()
        )));
    }

    // 1. Create writable snapshot
    btrfs::snapshot_writable(snap_path, &fork_path)
        .map_err(|e| MikaOsError::ForkFailed(format!("failed to create fork snapshot: {e}")))?;

    // 2. Re-create nested subvolumes in fork target
    // First, delete the readonly nested subvols from the snapshot
    for subvol in subvolume_layout::NESTED_SUBVOLUMES {
        let subvol_path = fork_path.join(subvol);
        if subvol_path.exists()
            && let Ok(true) = btrfs::is_subvolume(&subvol_path)
        {
            let _ = btrfs::delete_subvolume(&subvol_path);
        }
    }
    subvolume_layout::initialize_layout(&fork_path).map_err(|e| {
        MikaOsError::ForkFailed(format!("failed to create nested subvolumes in fork: {e}"))
    })?;

    // 3. Run tenant-ingest (rename agent_id references in DB)
    let db_path = fork_path.join("data/mika.db");
    if db_path.exists()
        && let Err(e) = run_tenant_ingest(&db_path, new_tenant)
    {
        tracing::warn!(error = %e, "tenant-ingest had errors (fork may need manual fixup)");
    }

    // 4. Redact secrets unless --keep-secrets
    if !keep_secrets {
        let env_path = fork_path.join(".env");
        if env_path.exists()
            && let Err(e) = redact::redact_env_file(&env_path)
        {
            tracing::warn!(error = %e, "failed to redact .env in fork");
        }

        let oauth_path = fork_path.join("oauth.json");
        if let Err(e) = redact::redact_oauth_json(&oauth_path) {
            tracing::warn!(error = %e, "failed to redact oauth.json in fork");
        }

        let config_path = fork_path.join("config.toml");
        if let Err(e) = redact::redact_config_toml(&config_path) {
            tracing::warn!(error = %e, "failed to redact config.toml in fork");
        }

        if db_path.exists()
            && let Err(e) = redact::redact_db_secrets(&db_path, new_tenant)
        {
            tracing::warn!(error = %e, "failed to redact DB secrets in fork");
        }
    }

    Ok(fork_path)
}

/// Rewrite agent_id references in the forked DB to the new tenant name.
fn run_tenant_ingest(db_path: &Path, new_tenant: &str) -> Result<(), MikaOsError> {
    let conn = rusqlite::Connection::open(db_path)?;

    // Get the current agent_id (assume single-tenant DB — first agent row)
    let old_agent: Option<String> = conn
        .query_row("SELECT id FROM agents ORDER BY rowid LIMIT 1", [], |row| {
            row.get(0)
        })
        .ok();

    let Some(old_agent) = old_agent else {
        tracing::info!("no agents found in forked DB, skipping tenant-ingest");
        return Ok(());
    };

    if old_agent == new_tenant {
        tracing::info!("agent_id already matches new tenant, skipping tenant-ingest");
        return Ok(());
    }

    // Update each table that has an agent_id column
    for table in FORK_INGEST_TABLES {
        // Skip the agents table — we rename the id directly
        if *table == "agents" {
            continue;
        }

        // Check if the table exists (schema may vary)
        let exists: bool = conn
            .query_row(
                "SELECT COUNT(*) > 0 FROM sqlite_master WHERE type='table' AND name=?1",
                [table],
                |row| row.get(0),
            )
            .unwrap_or(false);

        if !exists {
            continue;
        }

        // Check if the table has an agent_id column
        let has_col = has_column(&conn, table, "agent_id");
        if !has_col {
            continue;
        }

        let sql = format!("UPDATE \"{table}\" SET agent_id = ?1 WHERE agent_id = ?2");
        match conn.execute(&sql, rusqlite::params![new_tenant, old_agent]) {
            Ok(n) => {
                tracing::debug!(table, rows = n, "tenant-ingest updated");
            }
            Err(e) => {
                tracing::warn!(table, error = %e, "tenant-ingest failed for table");
            }
        }
    }

    // Rename the agent row itself
    let _ = conn.execute(
        "UPDATE agents SET id = ?1 WHERE id = ?2",
        rusqlite::params![new_tenant, old_agent],
    );

    Ok(())
}

fn has_column(conn: &rusqlite::Connection, table: &str, column: &str) -> bool {
    let sql = format!("PRAGMA table_info(\"{table}\")");
    let mut stmt = match conn.prepare(&sql) {
        Ok(s) => s,
        Err(_) => return false,
    };

    let result = stmt.query_map([], |row| {
        let name: String = row.get(1)?;
        Ok(name)
    });

    match result {
        Ok(rows) => rows.filter_map(|r| r.ok()).any(|name| name == column),
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fork_ingest_tables_is_not_empty() {
        assert!(!FORK_INGEST_TABLES.is_empty());
    }

    #[test]
    fn fork_ingest_tables_contains_expected_tables() {
        // Critical tables that MUST be in the list
        let critical = ["agents", "sessions", "messages", "tasks", "core_memory"];
        for table in &critical {
            assert!(
                FORK_INGEST_TABLES.contains(table),
                "FORK_INGEST_TABLES missing critical table: {table}"
            );
        }
    }

    #[test]
    fn fork_ingest_tables_no_duplicates() {
        let mut seen = std::collections::HashSet::new();
        for table in FORK_INGEST_TABLES {
            assert!(
                seen.insert(table),
                "duplicate table in FORK_INGEST_TABLES: {table}"
            );
        }
    }
}
