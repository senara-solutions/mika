//! DB-backed integration test for the `/unlink` self-service SQL and the admin
//! unlink endpoint's SQL shape (mika#1749).
//!
//! Like `admin_customers.rs`, this test is `#[ignore]` by design: it requires a
//! live Postgres and CI does not provision one for the gateway crate. It
//! exercises the two atomic UPDATEs used by the production handlers:
//!
//! - `handle_unlink_confirm` (bot side):
//!   `UPDATE customers SET telegram_chat_id = NULL WHERE telegram_chat_id = $1 RETURNING id`
//! - `handle_admin_unlink` (admin side):
//!   `UPDATE customers c SET telegram_chat_id = NULL FROM (SELECT id, telegram_chat_id FROM customers WHERE id = $1) AS old WHERE c.id = old.id RETURNING old.telegram_chat_id`
//!
//! If the SQL in `crates/mika-gateway/src/routes.rs::handle_unlink_confirm` or
//! `handle_admin_unlink` changes, update this copy to keep the regression honest.
//!
//! Run manually against a throwaway Postgres:
//!
//! ```bash
//! MIKA_DATABASE_URL=postgres://mika:mika@localhost/mika \
//!   cargo test -p mika-gateway --test unlink -- --ignored --nocapture
//! ```

use sqlx::postgres::PgPoolOptions;
use sqlx::{Executor, Row};
use uuid::Uuid;

/// Bot-side self-unlink: matches on `telegram_chat_id`, returns the customer id.
const SELF_UNLINK_SQL: &str = "UPDATE customers SET telegram_chat_id = NULL \
                               WHERE telegram_chat_id = $1 RETURNING id";

/// Admin-side unlink: matches on customer id, returns the pre-update chat_id
/// via a self-join subquery snapshot.
const ADMIN_UNLINK_SQL: &str = "UPDATE customers c \
                                SET telegram_chat_id = NULL \
                                FROM (SELECT id, telegram_chat_id FROM customers WHERE id = $1) AS old \
                                WHERE c.id = old.id \
                                RETURNING old.telegram_chat_id";

#[tokio::test]
#[ignore = "requires a live Postgres at MIKA_DATABASE_URL / DATABASE_URL"]
async fn unlink_self_and_admin_flows() {
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
    // Minimal customers shape — UNIQUE on telegram_chat_id matches production
    // (`crates/mika-gateway/migrations/001_customers.sql:8`), so we exercise the
    // same constraint interaction the unlink policy has to reason about.
    pool.execute(
        "CREATE TABLE customers ( \
             id UUID PRIMARY KEY, \
             telegram_chat_id BIGINT UNIQUE \
         )",
    )
    .await
    .expect("create customers table");

    let result = run_unlink_flows(&pool).await;

    // Always drop the schema, even on failure.
    let _ = pool
        .execute(format!("DROP SCHEMA {schema} CASCADE").as_str())
        .await;
    result.expect("unlink flows");
}

async fn run_unlink_flows(pool: &sqlx::PgPool) -> Result<(), String> {
    let alice = Uuid::new_v4();
    let bob = Uuid::new_v4();
    let alice_chat: i64 = 111_000;
    let bob_chat: i64 = 222_000;

    // Seed two paired customers and one unpaired customer.
    for (id, chat_id) in [(alice, Some(alice_chat)), (bob, Some(bob_chat))] {
        sqlx::query("INSERT INTO customers (id, telegram_chat_id) VALUES ($1, $2)")
            .bind(id)
            .bind(chat_id)
            .execute(pool)
            .await
            .map_err(|e| format!("seed insert failed for {id}: {e}"))?;
    }
    let carol = Uuid::new_v4();
    sqlx::query("INSERT INTO customers (id, telegram_chat_id) VALUES ($1, NULL)")
        .bind(carol)
        .execute(pool)
        .await
        .map_err(|e| format!("seed carol (unpaired) failed: {e}"))?;

    // --- Case 1: Alice runs /unlink confirm → releases her binding. ---
    let released_id: Option<Uuid> = sqlx::query_scalar(SELF_UNLINK_SQL)
        .bind(alice_chat)
        .fetch_optional(pool)
        .await
        .map_err(|e| format!("case 1: self-unlink query failed: {e}"))?;
    if released_id != Some(alice) {
        return Err(format!(
            "case 1: expected Some({alice}) released, got {released_id:?}"
        ));
    }
    // Verify Alice's row now has telegram_chat_id = NULL.
    let alice_chat_after: Option<i64> =
        sqlx::query_scalar("SELECT telegram_chat_id FROM customers WHERE id = $1")
            .bind(alice)
            .fetch_one(pool)
            .await
            .map_err(|e| format!("case 1: alice re-read failed: {e}"))?;
    if alice_chat_after.is_some() {
        return Err(format!(
            "case 1: expected alice.telegram_chat_id NULL, got {alice_chat_after:?}"
        ));
    }

    // --- Case 2: cold /unlink confirm (nothing to unlink) is idempotent no-op. ---
    let cold: Option<Uuid> = sqlx::query_scalar(SELF_UNLINK_SQL)
        .bind(alice_chat) // already released
        .fetch_optional(pool)
        .await
        .map_err(|e| format!("case 2: cold self-unlink failed: {e}"))?;
    if cold.is_some() {
        return Err(format!(
            "case 2: expected None from cold self-unlink, got {cold:?}"
        ));
    }

    // --- Case 3: Admin unlinks Bob → returns Bob's pre-update chat_id. ---
    let bob_prev: Option<Option<i64>> = sqlx::query_scalar(ADMIN_UNLINK_SQL)
        .bind(bob)
        .fetch_optional(pool)
        .await
        .map_err(|e| format!("case 3: admin-unlink bob failed: {e}"))?;
    match bob_prev {
        Some(Some(x)) if x == bob_chat => {}
        other => {
            return Err(format!(
                "case 3: expected Some(Some({bob_chat})) for bob previous_chat_id, got {other:?}"
            ));
        }
    }
    // Bob's row now has telegram_chat_id = NULL.
    let bob_after: Option<i64> =
        sqlx::query_scalar("SELECT telegram_chat_id FROM customers WHERE id = $1")
            .bind(bob)
            .fetch_one(pool)
            .await
            .map_err(|e| format!("case 3: bob re-read failed: {e}"))?;
    if bob_after.is_some() {
        return Err(format!(
            "case 3: expected bob.telegram_chat_id NULL, got {bob_after:?}"
        ));
    }

    // --- Case 4: Admin unlinks Carol (already unbound) → row match, prev = None. ---
    let carol_prev: Option<Option<i64>> = sqlx::query_scalar(ADMIN_UNLINK_SQL)
        .bind(carol)
        .fetch_optional(pool)
        .await
        .map_err(|e| format!("case 4: admin-unlink carol failed: {e}"))?;
    if !matches!(carol_prev, Some(None)) {
        return Err(format!(
            "case 4: expected Some(None) idempotent for carol, got {carol_prev:?}"
        ));
    }

    // --- Case 5: Admin unlinks a nonexistent customer → 0 rows, None. ---
    let ghost = Uuid::new_v4();
    let ghost_prev: Option<Option<i64>> = sqlx::query_scalar(ADMIN_UNLINK_SQL)
        .bind(ghost)
        .fetch_optional(pool)
        .await
        .map_err(|e| format!("case 5: admin-unlink ghost failed: {e}"))?;
    if ghost_prev.is_some() {
        return Err(format!(
            "case 5: expected None for nonexistent customer, got {ghost_prev:?}"
        ));
    }

    // --- Case 6: After all unlinks, a fresh chat_id can be paired (regression
    //     guard: the UNIQUE constraint is not stuck holding a phantom binding). ---
    let fresh: i64 = 333_000;
    let paired: Uuid =
        sqlx::query_scalar("UPDATE customers SET telegram_chat_id = $1 WHERE id = $2 RETURNING id")
            .bind(fresh)
            .bind(alice)
            .fetch_one(pool)
            .await
            .map_err(|e| format!("case 6: fresh pair failed (unique still holding?): {e}"))?;
    if paired != alice {
        return Err(format!("case 6: expected alice re-paired, got {paired}"));
    }

    // Sanity: verify our schema matches what production expects.
    let schema_check: Vec<(String, String)> = sqlx::query(
        "SELECT column_name, data_type FROM information_schema.columns \
         WHERE table_name = 'customers' ORDER BY ordinal_position",
    )
    .fetch_all(pool)
    .await
    .map_err(|e| format!("schema check failed: {e}"))?
    .into_iter()
    .map(|r| (r.get::<String, _>(0), r.get::<String, _>(1)))
    .collect();
    if !schema_check.iter().any(|(n, _)| n == "telegram_chat_id") {
        return Err(format!(
            "schema check: missing telegram_chat_id column: {schema_check:?}"
        ));
    }

    Ok(())
}
