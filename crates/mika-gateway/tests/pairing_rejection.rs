//! DB-backed integration test for the `one-telegram-one-mika` refusal verdict
//! (mika-cloud#208).
//!
//! Same disposition as `unlink.rs` and `admin_customers_read.rs`: `#[ignore]`
//! by design — CI does not provision Postgres for the gateway crate. Run it
//! against a throwaway Postgres by hand:
//!
//! ```bash
//! MIKA_DATABASE_URL=postgres://mika:mika@localhost/mika \
//!   cargo test -p mika-gateway --test pairing_rejection -- --ignored --nocapture
//! ```
//!
//! What it pins, and why. The guard is a `UNIQUE` constraint on
//! `telegram_chat_id`: when a second customer tries to bind an already-bound
//! Telegram account, the pairing UPDATE fails with SQLSTATE 23505 and the
//! refused row is left untouched — `status = 'provisioned'`, `paired_at NULL`,
//! `pairing_token` intact. That is byte-identical to a customer who has not
//! started pairing, which is why the onboarding wizard could announce
//! completion on a pairing that never happened. These cases exercise, in order:
//!
//! 1. The guard still refuses (23505) — the invariant is untouched.
//! 2. The refusal verdict lands on the row bearing the presented token, and on
//!    no other row.
//! 3. A later successful pairing clears the verdict in the same atomic UPDATE.
//! 4. The CHECK constraints reject an unknown reason and a half-set pair.
//!
//! If the SQL in `crates/mika-gateway/src/routes.rs::handle_pairing` or
//! `record_pairing_rejection` changes, update this copy to keep the regression
//! honest.

use sqlx::postgres::PgPoolOptions;
use sqlx::{Executor, Row};
use uuid::Uuid;

/// The successful-pairing UPDATE from `handle_pairing`, verdict-clearing
/// clause included (mika-cloud#208).
const PAIR_SQL: &str = "UPDATE customers \
                        SET telegram_chat_id = $1, paired_at = now(), status = 'active', \
                            pairing_token = NULL, pairing_expires_at = NULL, \
                            pairing_rejected_at = NULL, pairing_rejection_reason = NULL \
                        WHERE pairing_token = $2 \
                          AND telegram_chat_id IS NULL \
                          AND status = 'provisioned' \
                          AND pairing_expires_at > now() \
                        RETURNING id";

/// `record_pairing_rejection`'s statement.
const RECORD_REJECTION_SQL: &str = "UPDATE customers \
                                    SET pairing_rejected_at = now(), \
                                        pairing_rejection_reason = $1 \
                                    WHERE pairing_token = $2";

const REASON_ALREADY_LINKED: &str = "telegram_already_linked";

/// The `provisioned` branch of the customer upsert in `handle_create_customer`,
/// reduced to the columns this test cares about: a new token, and the verdict
/// clearing that must ride with it.
const REISSUE_TOKEN_SQL: &str = "UPDATE customers \
                                 SET pairing_token = $1, \
                                     pairing_expires_at = now() + interval '48 hours', \
                                     pairing_rejected_at = NULL, \
                                     pairing_rejection_reason = NULL \
                                 WHERE id = $2 AND status = 'provisioned'";

#[tokio::test]
#[ignore = "requires a live Postgres at MIKA_DATABASE_URL / DATABASE_URL"]
async fn pairing_rejection_verdict_is_recorded_and_cleared() {
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

    // Production-shaped subset: the UNIQUE guard from 001_customers.sql plus
    // the verdict columns and CHECK constraints from
    // 010_customers_pairing_rejection.sql.
    pool.execute(
        "CREATE TABLE customers ( \
             id UUID PRIMARY KEY, \
             status TEXT NOT NULL DEFAULT 'provisioned', \
             telegram_chat_id BIGINT UNIQUE, \
             pairing_token TEXT UNIQUE, \
             pairing_expires_at TIMESTAMPTZ, \
             paired_at TIMESTAMPTZ, \
             pairing_rejected_at TIMESTAMPTZ, \
             pairing_rejection_reason TEXT, \
             CONSTRAINT customers_pairing_rejection_reason_check CHECK ( \
                 pairing_rejection_reason IS NULL \
                 OR pairing_rejection_reason IN ('telegram_already_linked') \
             ), \
             CONSTRAINT customers_pairing_rejection_coherent_check CHECK ( \
                 (pairing_rejected_at IS NULL AND pairing_rejection_reason IS NULL) \
                 OR (pairing_rejected_at IS NOT NULL AND pairing_rejection_reason IS NOT NULL) \
             ) \
         )",
    )
    .await
    .expect("create customers table");

    let result = run_rejection_flows(&pool).await;

    let _ = pool
        .execute(format!("DROP SCHEMA {schema} CASCADE").as_str())
        .await;
    result.expect("pairing rejection flows");
}

async fn run_rejection_flows(pool: &sqlx::PgPool) -> Result<(), String> {
    let incumbent = Uuid::new_v4();
    let newcomer = Uuid::new_v4();
    let bystander = Uuid::new_v4();
    let chat_id: i64 = 7_875_349_528;
    let newcomer_token = "n".repeat(64);
    let bystander_token = "b".repeat(64);

    // The incumbent already holds Vincent's Telegram binding.
    sqlx::query(
        "INSERT INTO customers (id, status, telegram_chat_id, paired_at) \
         VALUES ($1, 'active', $2, now())",
    )
    .bind(incumbent)
    .bind(chat_id)
    .execute(pool)
    .await
    .map_err(|e| format!("seed incumbent failed: {e}"))?;

    // The newcomer is the freshly provisioned customer whose pairing will be
    // refused, and a bystander with its own unconsumed token.
    for (id, token) in [
        (newcomer, newcomer_token.as_str()),
        (bystander, bystander_token.as_str()),
    ] {
        sqlx::query(
            "INSERT INTO customers (id, status, pairing_token, pairing_expires_at) \
             VALUES ($1, 'provisioned', $2, now() + interval '1 hour')",
        )
        .bind(id)
        .bind(token)
        .execute(pool)
        .await
        .map_err(|e| format!("seed {id} failed: {e}"))?;
    }

    // --- Case 1: the guard refuses. This is the invariant, not the bug. ---
    let pair_attempt = sqlx::query(PAIR_SQL)
        .bind(chat_id)
        .bind(&newcomer_token)
        .fetch_optional(pool)
        .await;
    let db_err = match pair_attempt {
        Ok(_) => {
            return Err(
                "case 1: one-telegram-one-mika did NOT refuse — the guard is broken".to_string(),
            );
        }
        Err(e) => e,
    };
    let code = db_err
        .as_database_error()
        .and_then(|e| e.code().map(|c| c.to_string()));
    if code.as_deref() != Some("23505") {
        return Err(format!("case 1: expected SQLSTATE 23505, got {code:?}"));
    }

    // --- Case 2: the verdict lands on the refused row, and on it alone. ---
    let affected = sqlx::query(RECORD_REJECTION_SQL)
        .bind(REASON_ALREADY_LINKED)
        .bind(&newcomer_token)
        .execute(pool)
        .await
        .map_err(|e| format!("case 2: recording the verdict failed: {e}"))?
        .rows_affected();
    if affected != 1 {
        return Err(format!("case 2: expected 1 row updated, got {affected}"));
    }

    let row = sqlx::query(
        "SELECT status, paired_at, pairing_rejected_at, pairing_rejection_reason \
         FROM customers WHERE id = $1",
    )
    .bind(newcomer)
    .fetch_one(pool)
    .await
    .map_err(|e| format!("case 2: re-read newcomer failed: {e}"))?;
    let reason: Option<String> = row.get("pairing_rejection_reason");
    let rejected_at: Option<chrono::DateTime<chrono::Utc>> = row.get("pairing_rejected_at");
    let paired_at: Option<chrono::DateTime<chrono::Utc>> = row.get("paired_at");
    let status: String = row.get("status");
    if reason.as_deref() != Some(REASON_ALREADY_LINKED) || rejected_at.is_none() {
        return Err(format!(
            "case 2: expected a recorded verdict, got reason={reason:?} at={rejected_at:?}"
        ));
    }
    // Recording a verdict must not be a second path to pairing.
    if paired_at.is_some() || status != "provisioned" {
        return Err(format!(
            "case 2: the verdict write changed pairing state — status={status}, paired_at={paired_at:?}"
        ));
    }

    // The incumbent keeps its binding, and the bystander is untouched.
    let incumbent_chat: Option<i64> =
        sqlx::query_scalar("SELECT telegram_chat_id FROM customers WHERE id = $1")
            .bind(incumbent)
            .fetch_one(pool)
            .await
            .map_err(|e| format!("case 2: re-read incumbent failed: {e}"))?;
    if incumbent_chat != Some(chat_id) {
        return Err(format!(
            "case 2: the incumbent's binding moved — got {incumbent_chat:?}"
        ));
    }
    let bystander_reason: Option<String> =
        sqlx::query_scalar("SELECT pairing_rejection_reason FROM customers WHERE id = $1")
            .bind(bystander)
            .fetch_one(pool)
            .await
            .map_err(|e| format!("case 2: re-read bystander failed: {e}"))?;
    if bystander_reason.is_some() {
        return Err(format!(
            "case 2: the verdict leaked onto an unrelated row: {bystander_reason:?}"
        ));
    }

    // --- Case 3: after /unlink confirm, the retry pairs AND clears the verdict. ---
    sqlx::query("UPDATE customers SET telegram_chat_id = NULL WHERE telegram_chat_id = $1")
        .bind(chat_id)
        .execute(pool)
        .await
        .map_err(|e| format!("case 3: incumbent self-unlink failed: {e}"))?;

    let paired_id: Option<Uuid> = sqlx::query_scalar(PAIR_SQL)
        .bind(chat_id)
        .bind(&newcomer_token)
        .fetch_optional(pool)
        .await
        .map_err(|e| format!("case 3: retry pairing failed: {e}"))?;
    if paired_id != Some(newcomer) {
        return Err(format!(
            "case 3: expected the newcomer to pair, got {paired_id:?}"
        ));
    }

    let row = sqlx::query(
        "SELECT status, paired_at, pairing_rejected_at, pairing_rejection_reason \
         FROM customers WHERE id = $1",
    )
    .bind(newcomer)
    .fetch_one(pool)
    .await
    .map_err(|e| format!("case 3: re-read newcomer failed: {e}"))?;
    let reason: Option<String> = row.get("pairing_rejection_reason");
    let rejected_at: Option<chrono::DateTime<chrono::Utc>> = row.get("pairing_rejected_at");
    let paired_at: Option<chrono::DateTime<chrono::Utc>> = row.get("paired_at");
    if reason.is_some() || rejected_at.is_some() {
        return Err(format!(
            "case 3: a completed pairing still reads as refused — reason={reason:?} at={rejected_at:?}"
        ));
    }
    if paired_at.is_none() {
        return Err("case 3: paired_at was not set".to_string());
    }

    // --- Case 4: the CHECK constraints hold the vocabulary and the pair. ---
    let unknown_reason = sqlx::query(
        "UPDATE customers SET pairing_rejected_at = now(), pairing_rejection_reason = 'made_up' \
         WHERE id = $1",
    )
    .bind(bystander)
    .execute(pool)
    .await;
    if unknown_reason.is_ok() {
        return Err("case 4: an unknown rejection reason was accepted".to_string());
    }

    let half_set = sqlx::query("UPDATE customers SET pairing_rejected_at = now() WHERE id = $1")
        .bind(bystander)
        .execute(pool)
        .await;
    if half_set.is_ok() {
        return Err("case 4: a timestamp with no reason was accepted".to_string());
    }

    // --- Case 5: a fresh pairing token clears a spent verdict. ---
    // A refused customer stays `provisioned`, which is the branch that gets a
    // new token on re-provisioning. Leaving the old verdict there would show
    // the console a refusal the user has not made yet.
    sqlx::query(
        "UPDATE customers SET pairing_rejected_at = now(), \
             pairing_rejection_reason = $1 WHERE id = $2",
    )
    .bind(REASON_ALREADY_LINKED)
    .bind(bystander)
    .execute(pool)
    .await
    .map_err(|e| format!("case 5: seeding a verdict failed: {e}"))?;

    sqlx::query(REISSUE_TOKEN_SQL)
        .bind("c".repeat(64))
        .bind(bystander)
        .execute(pool)
        .await
        .map_err(|e| format!("case 5: token re-issue failed: {e}"))?;

    let reason: Option<String> =
        sqlx::query_scalar("SELECT pairing_rejection_reason FROM customers WHERE id = $1")
            .bind(bystander)
            .fetch_one(pool)
            .await
            .map_err(|e| format!("case 5: re-read bystander failed: {e}"))?;
    if reason.is_some() {
        return Err(format!(
            "case 5: a spent verdict survived a fresh token: {reason:?}"
        ));
    }

    Ok(())
}
