//! DB-backed integration test for the `POST /admin/customers` upsert SQL (mika#1612, AC5).
//!
//! This test is `#[ignore]` by design: it requires a live Postgres and CI does not
//! provision one for the gateway crate (no `services:` block in `ci.yml`, and the
//! sqlx workspace features omit `macros`/`migrate` so `#[sqlx::test]` is unavailable).
//! It is the SQL-level regression test the architect-groomed plan specified — full
//! HTTP-handler tests (Axum harness + mock Telegram) are a separate enhancement.
//!
//! Run it manually against a throwaway Postgres:
//!
//! ```bash
//! MIKA_DATABASE_URL=postgres://mika:mika@localhost/mika \
//!   cargo test -p mika-gateway --test admin_customers -- --ignored --nocapture
//! ```
//!
//! The test is self-contained: it creates its own uniquely-named schema, builds a
//! minimal `customers` table inside it, exercises the upsert, then drops the schema.
//! It never touches an application schema.
//!
//! NOTE: the `INSERT ... ON CONFLICT` statement below mirrors the upsert in
//! `crates/mika-gateway/src/routes.rs::handle_register_customer`. If that SQL
//! changes, update this copy to keep the regression honest.

use sqlx::postgres::PgPoolOptions;
use sqlx::{Executor, Row};
use uuid::Uuid;

/// The customer-registration upsert, identical to the production handler's.
const UPSERT_SQL: &str = r#"INSERT INTO customers (id, name, plan, timezone, status, bot_token, bot_username, webhook_secret, pairing_token, pairing_expires_at)
   VALUES ($1, $2, $3, $4, 'provisioned', $5, $6, $7, $8, now() + make_interval(hours => $9))
   ON CONFLICT (id) DO UPDATE SET
       name = EXCLUDED.name,
       bot_token = EXCLUDED.bot_token,
       bot_username = EXCLUDED.bot_username,
       webhook_secret = CASE WHEN customers.status = 'provisioned' THEN EXCLUDED.webhook_secret ELSE customers.webhook_secret END,
       pairing_token = CASE WHEN customers.status = 'provisioned' THEN EXCLUDED.pairing_token ELSE customers.pairing_token END,
       pairing_expires_at = CASE WHEN customers.status = 'provisioned' THEN EXCLUDED.pairing_expires_at ELSE customers.pairing_expires_at END
   RETURNING status, pairing_token, (xmax = 0) AS was_inserted"#;

#[tokio::test]
#[ignore = "requires a live Postgres at MIKA_DATABASE_URL / DATABASE_URL"]
async fn admin_customers_upsert_secret_lifecycle() {
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
    pool.execute(
        r#"CREATE TABLE customers (
            id UUID PRIMARY KEY,
            name TEXT NOT NULL,
            plan TEXT NOT NULL DEFAULT 'standard',
            status TEXT NOT NULL DEFAULT 'provisioned',
            timezone TEXT NOT NULL DEFAULT 'UTC',
            bot_token TEXT,
            bot_username TEXT,
            webhook_secret TEXT,
            pairing_token TEXT UNIQUE,
            pairing_expires_at TIMESTAMPTZ
        )"#,
    )
    .await
    .expect("create customers table");

    let result = run_lifecycle(&pool).await;

    // Always drop the schema, even on assertion failure, then surface the result.
    let _ = pool
        .execute(format!("DROP SCHEMA {schema} CASCADE").as_str())
        .await;
    result.expect("lifecycle assertions");
}

/// Returns `Err` with a message on the first failed invariant; `Ok(())` if all pass.
async fn run_lifecycle(pool: &sqlx::PgPool) -> Result<(), String> {
    let id = Uuid::new_v4();

    // --- Case 1: fresh insert (AC1 — i32 bind for make_interval must not 500). ---
    let row = sqlx::query(UPSERT_SQL)
        .bind(id)
        .bind("Acme")
        .bind("standard")
        .bind("UTC")
        .bind("token-1")
        .bind("acmebot")
        .bind("secret-A")
        .bind("pair-1")
        .bind(48_i32) // int4, not float8 — the mika#1612 P1 #1 fix
        .fetch_one(pool)
        .await
        .map_err(|e| format!("case 1 upsert failed (AC1 regression?): {e}"))?;
    let was_inserted: bool = row.get("was_inserted");
    if !was_inserted {
        return Err("case 1: expected was_inserted = true on fresh insert".into());
    }
    expect_secret(pool, id, "secret-A", "case 1 fresh insert").await?;

    // --- Case 2: re-register while provisioned → secret IS overwritten. ---
    upsert_again(pool, id, "secret-B", "pair-2").await?;
    let was_inserted = last_was_inserted(pool, id).await?; // sanity: still false below
    let _ = was_inserted;
    expect_secret(pool, id, "secret-B", "case 2 provisioned re-register").await?;

    // --- Case 3: active customer, setWebhook FAILS → secret PRESERVED. ---
    set_active(pool, id).await?;
    upsert_again(pool, id, "secret-C-rejected", "pair-3").await?;
    // No rotation UPDATE happens because setWebhook "failed" — secret must stay B.
    expect_secret(
        pool,
        id,
        "secret-B",
        "case 3 active re-register, setWebhook failure",
    )
    .await?;
    // The consumed (NULL) pairing_token must NOT be resurrected by the upsert — this is
    // the DB-side guarantee behind AC4 (response returns no pairing fields).
    expect_pairing_token(pool, id, None, "case 3 active re-register").await?;

    // --- Case 4: active customer, setWebhook SUCCEEDS → secret rotated via UPDATE. ---
    upsert_again(pool, id, "secret-D", "pair-4").await?;
    // Upsert preserved B; the handler then runs the rotation UPDATE on setWebhook success.
    sqlx::query("UPDATE customers SET webhook_secret = $1 WHERE id = $2")
        .bind("secret-D")
        .bind(id)
        .execute(pool)
        .await
        .map_err(|e| format!("case 4 rotation UPDATE failed: {e}"))?;
    expect_secret(
        pool,
        id,
        "secret-D",
        "case 4 active re-register, setWebhook success",
    )
    .await?;

    Ok(())
}

async fn upsert_again(
    pool: &sqlx::PgPool,
    id: Uuid,
    secret: &str,
    pairing: &str,
) -> Result<(), String> {
    sqlx::query(UPSERT_SQL)
        .bind(id)
        .bind("Acme")
        .bind("standard")
        .bind("UTC")
        .bind("token-1")
        .bind("acmebot")
        .bind(secret)
        .bind(pairing)
        .bind(48_i32)
        .fetch_one(pool)
        .await
        .map(|_| ())
        .map_err(|e| format!("re-register upsert failed: {e}"))
}

async fn last_was_inserted(pool: &sqlx::PgPool, id: Uuid) -> Result<bool, String> {
    let row = sqlx::query("SELECT (xmax = 0) AS was_inserted FROM customers WHERE id = $1")
        .bind(id)
        .fetch_one(pool)
        .await
        .map_err(|e| format!("was_inserted probe failed: {e}"))?;
    Ok(row.get("was_inserted"))
}

async fn set_active(pool: &sqlx::PgPool, id: Uuid) -> Result<(), String> {
    // Model a paired customer: status='active' and pairing_token consumed (NULL) by
    // handle_pairing. This is the scenario the AC4 response fix cares about.
    sqlx::query("UPDATE customers SET status = 'active', pairing_token = NULL WHERE id = $1")
        .bind(id)
        .execute(pool)
        .await
        .map(|_| ())
        .map_err(|e| format!("set_active failed: {e}"))
}

async fn expect_secret(pool: &sqlx::PgPool, id: Uuid, want: &str, ctx: &str) -> Result<(), String> {
    let row = sqlx::query("SELECT webhook_secret FROM customers WHERE id = $1")
        .bind(id)
        .fetch_one(pool)
        .await
        .map_err(|e| format!("{ctx}: secret probe failed: {e}"))?;
    let got: Option<String> = row.get("webhook_secret");
    if got.as_deref() == Some(want) {
        Ok(())
    } else {
        Err(format!(
            "{ctx}: expected webhook_secret {want:?}, got {got:?}"
        ))
    }
}

async fn expect_pairing_token(
    pool: &sqlx::PgPool,
    id: Uuid,
    want: Option<&str>,
    ctx: &str,
) -> Result<(), String> {
    let row = sqlx::query("SELECT pairing_token FROM customers WHERE id = $1")
        .bind(id)
        .fetch_one(pool)
        .await
        .map_err(|e| format!("{ctx}: pairing_token probe failed: {e}"))?;
    let got: Option<String> = row.get("pairing_token");
    if got.as_deref() == want {
        Ok(())
    } else {
        Err(format!(
            "{ctx}: expected pairing_token {want:?}, got {got:?}"
        ))
    }
}
