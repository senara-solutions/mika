//! Periodic WAL checkpoint for the dashboard database connection.
//!
//! Defense-in-depth for mika#636: even if a future code path accidentally
//! leaks a transaction (pinning the WAL snapshot), the periodic checkpoint
//! forces the connection to advance past any held snapshot.
//!
//! Uses `PRAGMA wal_checkpoint(PASSIVE)` which is non-blocking — it only
//! checkpoints pages not currently being read by another connection. If the
//! WAL file continues growing despite 60-second PASSIVE checkpoints (e.g.,
//! under continuous read load that prevents any pages from being
//! checkpointed), escalate to `RESTART` mode which briefly blocks writers
//! but guarantees a full checkpoint.
//!
//! The interval is hard-coded to 60 seconds. If operator demand surfaces
//! for tunability, add `MIKA_DASHBOARD_CHECKPOINT_INTERVAL_SECS` env var.

use crate::async_db::AsyncDatabase;
use std::time::Duration;
use tracing::{info, warn};

/// Checkpoint interval in seconds. Hard-coded for v1 (YAGNI per plan Finding 5).
/// Future tunable: `MIKA_DASHBOARD_CHECKPOINT_INTERVAL_SECS`.
const CHECKPOINT_INTERVAL_SECS: u64 = 60;

/// Spawns a background tokio task that runs `PRAGMA wal_checkpoint(PASSIVE)`
/// on the dashboard database connection every [`CHECKPOINT_INTERVAL_SECS`].
///
/// The task runs until the `AsyncDatabase` is dropped (at which point
/// `with_db` returns an error and the loop exits).
pub fn spawn_dashboard_checkpoint_task(db: AsyncDatabase) {
    tokio::spawn(async move {
        let interval = Duration::from_secs(CHECKPOINT_INTERVAL_SECS);
        loop {
            tokio::time::sleep(interval).await;

            info!(target: "mika::checkpoint", event = "checkpoint.start");

            match db
                .with_db(|db| {
                    let row = db
                        .conn
                        .query_row("PRAGMA wal_checkpoint(PASSIVE)", [], |row| {
                            let busy: i64 = row.get(0)?;
                            let log: i64 = row.get(1)?;
                            let checkpointed: i64 = row.get(2)?;
                            Ok((busy, log, checkpointed))
                        })?;
                    Ok(row)
                })
                .await
            {
                Ok((busy, log, checkpointed)) => {
                    info!(
                        target: "mika::checkpoint",
                        event = "checkpoint.complete",
                        busy_pages = busy,
                        log_pages = log,
                        checkpointed_pages = checkpointed,
                    );
                }
                Err(e) => {
                    warn!(
                        target: "mika::checkpoint",
                        event = "checkpoint.error",
                        error = %e,
                    );
                    // If the database is shut down, exit the loop.
                    // Covers all three AsyncDatabase error variants:
                    // "database has been shut down", "database thread has stopped",
                    // "database thread dropped reply" (closure panic via catch_unwind).
                    if e.to_string().contains("has been shut down")
                        || e.to_string().contains("has stopped")
                        || e.to_string().contains("dropped reply")
                    {
                        info!(
                            target: "mika::checkpoint",
                            event = "checkpoint.stopped",
                            reason = "database shut down",
                        );
                        break;
                    }
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use rusqlite::Connection;
    use tempfile::NamedTempFile;

    /// Test that `PRAGMA wal_checkpoint(PASSIVE)` returns page counts and
    /// checkpoints pending WAL frames.
    #[test]
    fn test_checkpoint_pragma_returns_pages() {
        let tmp = NamedTempFile::new().unwrap();
        let path = tmp.path().to_str().unwrap();

        // Open a connection and enable WAL mode.
        let conn = Connection::open(path).unwrap();
        conn.execute_batch("PRAGMA journal_mode=WAL;").unwrap();
        conn.execute_batch("CREATE TABLE t (id INTEGER PRIMARY KEY, val TEXT);")
            .unwrap();

        // Write some data to generate WAL pages.
        for i in 0..100 {
            conn.execute("INSERT INTO t (val) VALUES (?1)", [format!("row-{i}")])
                .unwrap();
        }

        // Run PASSIVE checkpoint.
        let (busy, log, checkpointed): (i64, i64, i64) = conn
            .query_row("PRAGMA wal_checkpoint(PASSIVE)", [], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            })
            .unwrap();

        // With a single connection (no concurrent readers blocking), all pages
        // should be checkpointed.
        assert_eq!(busy, 0, "no pages should be busy with a single connection");
        assert!(log > 0, "WAL should have pages to checkpoint");
        assert_eq!(
            log, checkpointed,
            "all WAL pages should be checkpointed with no concurrent readers"
        );
    }

    /// Test that `PRAGMA wal_checkpoint(PASSIVE)` completes without error
    /// when another connection holds a read transaction. The checkpoint may
    /// report non-zero busy count (expected — those pages are skipped), but
    /// the checkpoint itself succeeds and the reader's query still works.
    ///
    /// Note: This tests concurrent-runner sanity, NOT staleness-fix.
    /// Test `test_dashboard_sees_writes_from_separate_connection` in db.rs
    /// is the staleness regression check.
    #[test]
    fn test_checkpoint_passive_does_not_error_under_concurrent_reader() {
        let tmp = NamedTempFile::new().unwrap();
        let path = tmp.path().to_str().unwrap();

        // Connection A: writer + checkpointer
        let conn_a = Connection::open(path).unwrap();
        conn_a.execute_batch("PRAGMA journal_mode=WAL;").unwrap();
        conn_a
            .execute_batch("CREATE TABLE t (id INTEGER PRIMARY KEY, val TEXT);")
            .unwrap();

        for i in 0..50 {
            conn_a
                .execute("INSERT INTO t (val) VALUES (?1)", [format!("row-{i}")])
                .unwrap();
        }

        // Connection B: reader holding a transaction (simulates dashboard query)
        let conn_b = Connection::open(path).unwrap();
        conn_b.execute_batch("BEGIN;").unwrap();
        let count_before: i64 = conn_b
            .query_row("SELECT COUNT(*) FROM t", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count_before, 50);

        // Write more data after reader started its transaction.
        for i in 50..100 {
            conn_a
                .execute("INSERT INTO t (val) VALUES (?1)", [format!("row-{i}")])
                .unwrap();
        }

        // PASSIVE checkpoint should succeed (some pages may be busy due to reader).
        let (busy, log, checkpointed): (i64, i64, i64) = conn_a
            .query_row("PRAGMA wal_checkpoint(PASSIVE)", [], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            })
            .unwrap();

        // The checkpoint completed — that's the key assertion. busy may be non-zero.
        assert!(log >= 0, "log count should be valid");
        assert!(checkpointed >= 0, "checkpointed count should be valid");
        // With a concurrent reader, some pages may be busy (can't checkpoint past
        // the reader's snapshot boundary).
        let _ = busy; // Acknowledged — not asserting exact value.

        // Reader's query should still work after the checkpoint.
        let count_during: i64 = conn_b
            .query_row("SELECT COUNT(*) FROM t", [], |row| row.get(0))
            .unwrap();
        // Reader sees the snapshot from when its transaction started (50 rows).
        assert_eq!(count_during, 50);

        conn_b.execute_batch("COMMIT;").unwrap();
    }
}
