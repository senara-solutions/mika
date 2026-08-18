//! DB-backed integration test for the `GET /admin/customers/{id}` and
//! `GET /admin/customers` SQL shapes (mika#1820, AC1/AC2/AC4).
//!
//! Same disposition as `admin_customers.rs` and `unlink.rs`: `#[ignore]` by
//! design — CI does not provision Postgres for the gateway crate. Exercised
//! against a throwaway Postgres by hand:
//!
//! ```bash
//! MIKA_DATABASE_URL=postgres://mika:mika@localhost/mika \
//!   cargo test -p mika-gateway --test admin_customers_read -- --ignored --nocapture
//! ```
//!
//! Guards two invariants of the mika#1820 security contract:
//!
//! 1. The projected `SELECT` list on the read endpoint never surfaces
//!    `bot_token`, `pairing_token` value, or `webhook_secret` (only presence
//!    for the pairing token).
//! 2. The orphan filter (`status='provisioned' AND paired_at IS NULL AND age >
//!    N min`) correctly picks up unpaired-and-stale rows and skips paired,
//!    non-provisioned, or fresh rows.
//!
//! If the SELECT/WHERE shapes in `crates/mika-gateway/src/routes.rs::handle_get_customer`
//! or `handle_list_customers` change, update this copy to keep the regression honest.

use sqlx::postgres::PgPoolOptions;
use sqlx::{Column, Executor, Row};
use uuid::Uuid;

/// The safe-column projection from `handle_get_customer`. NEVER change this
/// without extending the security-review checklist in the PR — the constant
/// name in production is `CUSTOMER_SAFE_COLUMNS`.
const SAFE_COLUMNS_SELECT: &str = "SELECT id, name, bot_username, status, paired_at, telegram_chat_id, plan, \
            (pairing_token IS NOT NULL) AS pairing_token_present, \
            pairing_expires_at, created_at, updated_at \
     FROM customers WHERE id = $1";

/// Orphan-list SQL matching the shape in `handle_list_customers` with all
/// three filters applied. Uses the same EXTRACT(EPOCH …) arithmetic the
/// handler builds dynamically.
const ORPHAN_LIST_SQL: &str = "SELECT id, bot_username, status, paired_at, plan, created_at, \
                                      CAST(FLOOR(EXTRACT(EPOCH FROM (now() - created_at)) / 60) AS BIGINT) AS age_minutes \
                               FROM customers \
                               WHERE status = $1 \
                                 AND paired_at IS NULL \
                                 AND (EXTRACT(EPOCH FROM (now() - created_at)) / 60) > $2 \
                               ORDER BY created_at ASC \
                               LIMIT 500";

#[tokio::test]
#[ignore = "requires a live Postgres at MIKA_DATABASE_URL / DATABASE_URL"]
async fn admin_customers_read_and_orphan_list() {
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

    let schema = format!("t_{}", Uuid::new_v4().simple());
    pool.execute(format!("CREATE SCHEMA {schema}").as_str())
        .await
        .expect("create schema");
    pool.execute(format!("SET search_path TO {schema}").as_str())
        .await
        .expect("set search_path");
    // Full customers shape — mirror migrations 001 + 008 so the projection can
    // exercise every safe column. Match production types so the FromRow shape
    // in production compiles cleanly against the same table.
    pool.execute(
        r#"CREATE TABLE customers (
            id UUID PRIMARY KEY,
            name TEXT NOT NULL,
            plan TEXT NOT NULL DEFAULT 'standard',
            status TEXT NOT NULL DEFAULT 'provisioned',
            telegram_chat_id BIGINT UNIQUE,
            timezone TEXT NOT NULL DEFAULT 'UTC',
            pairing_token TEXT UNIQUE,
            pairing_expires_at TIMESTAMPTZ,
            paired_at TIMESTAMPTZ,
            bot_token TEXT,
            bot_username TEXT,
            webhook_secret TEXT,
            created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
            updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
        )"#,
    )
    .await
    .expect("create customers table");

    let result = run_read_scenarios(&pool).await;

    let _ = pool
        .execute(format!("DROP SCHEMA {schema} CASCADE").as_str())
        .await;
    result.expect("read/orphan-list assertions");
}

async fn run_read_scenarios(pool: &sqlx::PgPool) -> Result<(), String> {
    // ---- Seed data ----
    // stale-orphan: provisioned + unpaired + created 60 min ago (older than 30-min stale window)
    let stale_orphan = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO customers (id, name, plan, status, paired_at, bot_token, bot_username, webhook_secret, pairing_token, pairing_expires_at, created_at) \
         VALUES ($1, 'StaleOrphan', 'standard', 'provisioned', NULL, 'BOT-TOKEN-SECRET-STALE', 'stale_bot', 'WH-SECRET-STALE', 'PAIRING-TOKEN-STALE-64hex', now() + interval '1 day', now() - interval '60 minutes')",
    )
    .bind(stale_orphan)
    .execute(pool)
    .await
    .map_err(|e| format!("seed stale_orphan failed: {e}"))?;

    // fresh-orphan: provisioned + unpaired + created 5 min ago (inside the 30-min stale window)
    let fresh_orphan = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO customers (id, name, plan, status, paired_at, bot_token, bot_username, webhook_secret, pairing_token, pairing_expires_at, created_at) \
         VALUES ($1, 'FreshOrphan', 'standard', 'provisioned', NULL, 'BOT-TOKEN-SECRET-FRESH', 'fresh_bot', 'WH-SECRET-FRESH', 'PAIRING-TOKEN-FRESH', now() + interval '1 day', now() - interval '5 minutes')",
    )
    .bind(fresh_orphan)
    .execute(pool)
    .await
    .map_err(|e| format!("seed fresh_orphan failed: {e}"))?;

    // paired-active: paired + status='active' + created 60 min ago (should not match orphan filter)
    let active = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO customers (id, name, plan, status, telegram_chat_id, paired_at, bot_token, bot_username, webhook_secret, created_at) \
         VALUES ($1, 'PairedActive', 'standard', 'active', 42424242, now() - interval '30 minutes', 'BOT-TOKEN-SECRET-ACTIVE', 'active_bot', 'WH-SECRET-ACTIVE', now() - interval '60 minutes')",
    )
    .bind(active)
    .execute(pool)
    .await
    .map_err(|e| format!("seed active failed: {e}"))?;

    // ---- Case 1: read returns safe fields for provisioned customer ----
    let row = sqlx::query(SAFE_COLUMNS_SELECT)
        .bind(stale_orphan)
        .fetch_one(pool)
        .await
        .map_err(|e| format!("case 1: SELECT for stale_orphan failed: {e}"))?;
    let name: String = row.get("name");
    let bot_username: Option<String> = row.get("bot_username");
    let status: String = row.get("status");
    let paired_at: Option<chrono::DateTime<chrono::Utc>> = row.get("paired_at");
    let pairing_token_present: bool = row.get("pairing_token_present");
    let pairing_expires_at: Option<chrono::DateTime<chrono::Utc>> = row.get("pairing_expires_at");
    if name != "StaleOrphan" {
        return Err(format!("case 1: expected name StaleOrphan, got {name}"));
    }
    if bot_username.as_deref() != Some("stale_bot") {
        return Err(format!(
            "case 1: expected bot_username stale_bot, got {bot_username:?}"
        ));
    }
    if status != "provisioned" {
        return Err(format!("case 1: expected status provisioned, got {status}"));
    }
    if paired_at.is_some() {
        return Err(format!(
            "case 1: expected paired_at NULL, got {paired_at:?}"
        ));
    }
    if !pairing_token_present {
        return Err("case 1: expected pairing_token_present=true".into());
    }
    if pairing_expires_at.is_none() {
        return Err("case 1: expected pairing_expires_at SOME".into());
    }
    // SECURITY: attempt to read secret columns from the row — must fail.
    // rusqlite/sqlx don't expose "column not selected" as a typed error, but
    // trying to `.get()` a name not in the projection panics. We instead
    // introspect the row's column set.
    let column_names: Vec<&str> = row.columns().iter().map(|c| c.name()).collect();
    assert!(
        !column_names.contains(&"bot_token"),
        "case 1 SECURITY: projection leaks bot_token: {column_names:?}"
    );
    assert!(
        !column_names.contains(&"webhook_secret"),
        "case 1 SECURITY: projection leaks webhook_secret: {column_names:?}"
    );
    assert!(
        !column_names.contains(&"pairing_token"),
        "case 1 SECURITY: projection leaks pairing_token value: {column_names:?}"
    );
    assert!(
        column_names.contains(&"pairing_token_present"),
        "case 1: expected pairing_token_present in projection: {column_names:?}"
    );

    // ---- Case 2: orphan-list picks up stale_orphan, skips fresh_orphan and active ----
    let rows = sqlx::query(ORPHAN_LIST_SQL)
        .bind("provisioned")
        .bind(30_i64)
        .fetch_all(pool)
        .await
        .map_err(|e| format!("case 2: orphan-list query failed: {e}"))?;
    let ids: Vec<Uuid> = rows.iter().map(|r| r.get::<Uuid, _>("id")).collect();
    if !ids.contains(&stale_orphan) {
        return Err(format!(
            "case 2: expected stale_orphan in orphan list, got {ids:?}"
        ));
    }
    if ids.contains(&fresh_orphan) {
        return Err(format!(
            "case 2: fresh_orphan (5 min old) must NOT match 30-min stale filter, got {ids:?}"
        ));
    }
    if ids.contains(&active) {
        return Err(format!(
            "case 2: paired-active must NOT match orphan filter (paired_at IS NOT NULL), got {ids:?}"
        ));
    }
    // age_minutes projection is >= 60 for stale_orphan.
    let stale_row = rows
        .iter()
        .find(|r| r.get::<Uuid, _>("id") == stale_orphan)
        .expect("stale_orphan present");
    let age_min: i64 = stale_row.get("age_minutes");
    if age_min < 60 {
        return Err(format!(
            "case 2: expected age_minutes >= 60 for stale_orphan, got {age_min}"
        ));
    }

    Ok(())
}
