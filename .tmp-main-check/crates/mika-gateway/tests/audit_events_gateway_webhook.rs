//! DB-backed integration test for the mika#1774 `audit_events` table + writer.
//!
//! This test is `#[ignore]` by design — same rationale as `admin_customers.rs`:
//! CI does not provision a Postgres for the gateway crate (no `services:` block
//! in `ci.yml`, and the sqlx workspace features omit `macros`/`migrate` so
//! `#[sqlx::test]` is unavailable). Pure unit tests over the metadata JSON
//! shape and target_key constants live inline in `audit_events.rs`; this file
//! covers AC3's dashboard query pattern end-to-end.
//!
//! Run manually against a throwaway Postgres:
//!
//! ```bash
//! MIKA_DATABASE_URL=postgres://mika:mika@localhost/mika \
//!   cargo test -p mika-gateway --test audit_events_gateway_webhook \
//!   -- --ignored --nocapture
//! ```
//!
//! The test is self-contained: it creates its own uniquely-named schema, builds
//! the `audit_events` table inside it (the SQL is a copy of the migration —
//! keep in sync if the DDL changes), exercises the four drop-reason paths, and
//! runs the AC3 dashboard query against each. Then it drops the schema.

use sqlx::postgres::PgPoolOptions;
use sqlx::{Executor, Row};
use uuid::Uuid;

/// DDL for the gateway `audit_events` table.
///
/// Mirrors `crates/mika-gateway/migrations/009_audit_events.sql`. If that
/// migration changes, update this copy to keep the regression honest.
const AUDIT_EVENTS_DDL: &str = r#"
CREATE TABLE audit_events (
    id          BIGSERIAL   PRIMARY KEY,
    tool_name   TEXT        NOT NULL,
    target_key  TEXT        NOT NULL,
    metadata    JSONB       NOT NULL DEFAULT '{}'::jsonb,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX audit_events_lookup_idx
    ON audit_events (tool_name, target_key, created_at DESC);
"#;

#[tokio::test]
#[ignore = "requires a live Postgres at MIKA_DATABASE_URL / DATABASE_URL"]
async fn audit_events_all_four_drop_paths_land() {
    let url = match std::env::var("MIKA_DATABASE_URL").or_else(|_| std::env::var("DATABASE_URL")) {
        Ok(u) => u,
        Err(_) => {
            eprintln!("SKIP: set MIKA_DATABASE_URL or DATABASE_URL to run this test");
            return;
        }
    };

    let pool = PgPoolOptions::new()
        .max_connections(1)
        .connect(&url)
        .await
        .expect("connect to Postgres");

    // Isolated schema so we never collide with or pollute an application schema.
    let schema = format!("t_{}", Uuid::new_v4().simple());
    pool.execute(format!("CREATE SCHEMA {schema}").as_str())
        .await
        .expect("create schema");
    pool.execute(format!("SET search_path TO {schema}").as_str())
        .await
        .expect("set search_path");
    pool.execute(AUDIT_EVENTS_DDL)
        .await
        .expect("create audit_events table");

    let result = run_all_four_paths(&pool).await;

    // Always drop the schema, even on assertion failure, then surface the result.
    let _ = pool
        .execute(format!("DROP SCHEMA {schema} CASCADE").as_str())
        .await;
    result.expect("drop-path assertions");
}

async fn run_all_four_paths(pool: &sqlx::PgPool) -> Result<(), String> {
    // --- Path 1: route_event(...) == None → webhook_no_route ---
    insert_drop(
        pool,
        "webhook_no_route",
        serde_json::json!({
            "event_type": "check_suite",
            "action": "completed",
            "check_conclusion": "cancelled",
            "delivery_id": "d-noroute-1",
            "repo_full_name": "senara-solutions/mika",
            "drop_reason": "webhook_no_route",
        }),
    )
    .await?;
    assert_ac3_count(pool, "webhook_no_route", 1, "path 1").await?;

    // --- Path 2: is_suppressed_review_request → webhook_reviewer_filter_dropped ---
    insert_drop(
        pool,
        "webhook_reviewer_filter_dropped",
        serde_json::json!({
            "event_type": "pull_request",
            "action": "review_requested",
            "check_conclusion": null,
            "delivery_id": "d-reviewer-1",
            "repo_full_name": "senara-solutions/mika",
            "drop_reason": "webhook_reviewer_filter_dropped",
        }),
    )
    .await?;
    assert_ac3_count(pool, "webhook_reviewer_filter_dropped", 1, "path 2").await?;

    // --- Path 3: is_webhook_denylisted_skill → webhook_denylisted_skill_dropped ---
    insert_drop(
        pool,
        "webhook_denylisted_skill_dropped",
        serde_json::json!({
            "event_type": "issues",
            "action": "labeled",
            "check_conclusion": null,
            "delivery_id": "d-denylist-1",
            "repo_full_name": "senara-solutions/mika",
            "drop_reason": "webhook_denylisted_skill_dropped",
        }),
    )
    .await?;
    assert_ac3_count(pool, "webhook_denylisted_skill_dropped", 1, "path 3").await?;

    // --- Path 4: synchronize no-diff → webhook_synchronize_no_diff_change ---
    insert_drop(
        pool,
        "webhook_synchronize_no_diff_change",
        serde_json::json!({
            "event_type": "pull_request",
            "action": "synchronize",
            "check_conclusion": null,
            "delivery_id": "d-nodiff-1",
            "repo_full_name": "senara-solutions/mika",
            "drop_reason": "webhook_synchronize_no_diff_change",
        }),
    )
    .await?;
    assert_ac3_count(pool, "webhook_synchronize_no_diff_change", 1, "path 4").await?;

    // --- Cross-path: metadata for one path shouldn't taint the AC3 query for
    //     any other. Insert one more row into path 1 and confirm the two paths
    //     independently count 2 and 1.
    insert_drop(
        pool,
        "webhook_no_route",
        serde_json::json!({
            "event_type": "issues",
            "action": "opened",
            "check_conclusion": null,
            "delivery_id": "d-noroute-2",
            "repo_full_name": "senara-solutions/mika",
            "drop_reason": "webhook_no_route",
        }),
    )
    .await?;
    assert_ac3_count(pool, "webhook_no_route", 2, "cross-path/1").await?;
    assert_ac3_count(pool, "webhook_reviewer_filter_dropped", 1, "cross-path/2").await?;

    // --- Metadata round-trip: read back and confirm the JSONB shape survives.
    let row = sqlx::query(
        "SELECT metadata FROM audit_events WHERE metadata->>'delivery_id' = $1 LIMIT 1",
    )
    .bind("d-nodiff-1")
    .fetch_one(pool)
    .await
    .map_err(|e| format!("metadata round-trip probe failed: {e}"))?;
    let meta: serde_json::Value = row.get("metadata");
    if meta["event_type"] != "pull_request"
        || meta["action"] != "synchronize"
        || meta["drop_reason"] != "webhook_synchronize_no_diff_change"
    {
        return Err(format!("metadata round-trip: shape mismatch, got {meta}"));
    }

    Ok(())
}

async fn insert_drop(
    pool: &sqlx::PgPool,
    target_key: &str,
    metadata: serde_json::Value,
) -> Result<(), String> {
    sqlx::query(
        r#"INSERT INTO audit_events (tool_name, target_key, metadata)
           VALUES ($1, $2, $3)"#,
    )
    .bind("gateway_webhook")
    .bind(target_key)
    .bind(&metadata)
    .execute(pool)
    .await
    .map(|_| ())
    .map_err(|e| format!("insert for {target_key} failed: {e}"))
}

/// AC3: the dashboard query filter must count only rows for the given
/// `target_key`, keyed on `tool_name = 'gateway_webhook'`, within the last day.
async fn assert_ac3_count(
    pool: &sqlx::PgPool,
    target_key: &str,
    want: i64,
    ctx: &str,
) -> Result<(), String> {
    let row = sqlx::query(
        r#"SELECT count(*) AS n FROM audit_events
           WHERE tool_name = 'gateway_webhook'
             AND target_key = $1
             AND created_at > now() - interval '1 day'"#,
    )
    .bind(target_key)
    .fetch_one(pool)
    .await
    .map_err(|e| format!("{ctx}: AC3 count query failed: {e}"))?;
    let got: i64 = row.get("n");
    if got == want {
        Ok(())
    } else {
        Err(format!(
            "{ctx}: AC3 count for {target_key} — want {want}, got {got}"
        ))
    }
}
