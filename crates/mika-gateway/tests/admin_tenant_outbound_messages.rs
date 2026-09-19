//! DB-backed integration test for the SQL half of
//! `GET /admin/tenants/{customer_id}/outbound-messages` (mika#2387).
//!
//! Same disposition as its four neighbours (`admin_customers_read.rs`,
//! `admin_customers.rs`, `unlink.rs`, `pairing_rejection.rs`): `#[ignore]` by
//! design — CI does not provision Postgres for the gateway crate. Run by hand
//! against a throwaway database:
//!
//! ```bash
//! MIKA_DATABASE_URL=postgres://mika:mika@localhost/mika \
//!   cargo test -p mika-gateway --test admin_tenant_outbound_messages -- --ignored --nocapture
//! ```
//!
//! **This file holds what CI structurally cannot.** The twelve in-crate tests
//! of `routes::tests::mika2387` cover the response shape, the auth, the refusal
//! of malformed input, the fail-closed short-circuit and the audited route —
//! everything that is a pure function or a router decision. Real SQL semantics
//! is not among them, which means the two tests that actually prove the absence
//! of a cross-tenant leak are here, and they do not run in CI.
//!
//! That limit is inherited from the crate's disposition, not introduced by
//! mika#2387; closing it (provisioning Postgres for this crate) touches all
//! five existing test files and the workflow, and has its own follow-up ticket.
//! Saying it here is what keeps AC3 from looking like it is held by a test that
//! never executes.
//!
//! If the SELECT/WHERE/ORDER BY shape in
//! `crates/mika-gateway/src/routes.rs::handle_admin_tenant_outbound_messages`
//! changes, update this copy to keep the regression honest.

use sqlx::postgres::PgPoolOptions;
use sqlx::{Column, Executor, Row};
use uuid::Uuid;

/// The production query, verbatim. The projection is an allowlist — never
/// `SELECT *` — and the ORDER BY is **total**: the PK is
/// `(telegram_message_id, chat_id)` and a burst of sends shares `created_at` to
/// the millisecond, so without the tiebreak OFFSET pagination can duplicate or
/// skip a row. A burst of near-simultaneous sends is exactly what mika#2358 is
/// counting, so an unstable sort would have the instrument falsify the count.
const OUTBOUND_PAGE_SQL: &str = "SELECT telegram_message_id, chat_id, agent_name, created_at \
     FROM outbound_messages \
     WHERE chat_id = $1 \
       AND created_at >= $2 \
       AND created_at < COALESCE($3::timestamptz, 'infinity'::timestamptz) \
     ORDER BY created_at DESC, telegram_message_id DESC \
     LIMIT $4 OFFSET $5";

/// The tenant resolution that runs *before* it, and whose `None` short-circuits
/// the query entirely (`routes::outbound_scope`).
const TENANT_CHAT_SQL: &str = "SELECT telegram_chat_id FROM customers WHERE id = $1";

#[tokio::test]
#[ignore = "requires a live Postgres at MIKA_DATABASE_URL / DATABASE_URL"]
async fn admin_tenant_outbound_messages_sql_contract() {
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

    // Mirror migrations 001 (the columns this endpoint reads) and 002 verbatim.
    pool.execute(
        r#"CREATE TABLE customers (
            id UUID PRIMARY KEY,
            name TEXT NOT NULL,
            plan TEXT NOT NULL DEFAULT 'standard',
            status TEXT NOT NULL DEFAULT 'provisioned',
            telegram_chat_id BIGINT UNIQUE,
            created_at TIMESTAMPTZ NOT NULL DEFAULT now()
        )"#,
    )
    .await
    .expect("create customers table");
    pool.execute(
        r#"CREATE TABLE outbound_messages (
            telegram_message_id BIGINT NOT NULL,
            chat_id BIGINT NOT NULL,
            agent_name TEXT NOT NULL,
            created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
            PRIMARY KEY (telegram_message_id, chat_id)
        )"#,
    )
    .await
    .expect("create outbound_messages table");
    pool.execute(
        "CREATE TABLE audit_events (
            id BIGSERIAL PRIMARY KEY,
            tool_name TEXT NOT NULL,
            target_key TEXT NOT NULL,
            metadata JSONB,
            created_at TIMESTAMPTZ NOT NULL DEFAULT now()
        )",
    )
    .await
    .expect("create audit_events table");

    let result = run_scenarios(&pool).await;

    let _ = pool
        .execute(format!("DROP SCHEMA {schema} CASCADE").as_str())
        .await;
    result.expect("outbound-messages SQL assertions");
}

async fn run_scenarios(pool: &sqlx::PgPool) -> Result<(), String> {
    // ---- Seed: two paired tenants and one unpaired ----
    let tenant_a = Uuid::new_v4();
    let tenant_b = Uuid::new_v4();
    let unpaired = Uuid::new_v4();
    let chat_a: i64 = 111_111_111;
    let chat_b: i64 = 222_222_222;

    for (id, name, chat) in [
        (tenant_a, "TenantA", Some(chat_a)),
        (tenant_b, "TenantB", Some(chat_b)),
        (unpaired, "Unpaired", None),
    ] {
        sqlx::query("INSERT INTO customers (id, name, telegram_chat_id) VALUES ($1, $2, $3)")
            .bind(id)
            .bind(name)
            .bind(chat)
            .execute(pool)
            .await
            .map_err(|e| format!("seed {name} failed: {e}"))?;
    }

    // Tenant A: three sends spread over time, plus a burst of four sharing one
    // `created_at` to the microsecond (the mika#2358 shape).
    for (msg_id, offset) in [(1_i64, "3 days"), (2, "2 days"), (3, "1 day")] {
        sqlx::query(
            "INSERT INTO outbound_messages (telegram_message_id, chat_id, agent_name, created_at) \
             VALUES ($1, $2, 'mika', now() - $3::interval)",
        )
        .bind(msg_id)
        .bind(chat_a)
        .bind(offset)
        .execute(pool)
        .await
        .map_err(|e| format!("seed A msg {msg_id} failed: {e}"))?;
    }
    for msg_id in 100_i64..=103 {
        sqlx::query(
            "INSERT INTO outbound_messages (telegram_message_id, chat_id, agent_name, created_at) \
             VALUES ($1, $2, 'mika', now() - interval '6 hours')",
        )
        .bind(msg_id)
        .bind(chat_a)
        .execute(pool)
        .await
        .map_err(|e| format!("seed A burst {msg_id} failed: {e}"))?;
    }

    // Tenant B: rows that must never appear in A's answer.
    for msg_id in 900_i64..=902 {
        sqlx::query(
            "INSERT INTO outbound_messages (telegram_message_id, chat_id, agent_name, created_at) \
             VALUES ($1, $2, 'mika', now() - interval '12 hours')",
        )
        .bind(msg_id)
        .bind(chat_b)
        .execute(pool)
        .await
        .map_err(|e| format!("seed B msg {msg_id} failed: {e}"))?;
    }

    let week_ago = chrono::Utc::now() - chrono::Duration::days(7);
    let no_upper: Option<chrono::DateTime<chrono::Utc>> = None;

    // ---- Case 1: outbound_messages_of_another_tenant_are_never_returned ----
    let rows = sqlx::query(OUTBOUND_PAGE_SQL)
        .bind(chat_a)
        .bind(week_ago)
        .bind(no_upper)
        .bind(1000_i64)
        .bind(0_i64)
        .fetch_all(pool)
        .await
        .map_err(|e| format!("case 1: query failed: {e}"))?;
    let ids: Vec<i64> = rows
        .iter()
        .map(|r| r.get::<i64, _>("telegram_message_id"))
        .collect();
    if ids.len() != 7 {
        return Err(format!("case 1: expected A's 7 rows, got {ids:?}"));
    }
    let chats: Vec<i64> = rows.iter().map(|r| r.get::<i64, _>("chat_id")).collect();
    if chats.iter().any(|c| *c != chat_a) {
        return Err(format!(
            "case 1 SECURITY: another tenant's chat_id in A's answer: {chats:?}"
        ));
    }
    if ids.iter().any(|i| (900..=902).contains(i)) {
        return Err(format!(
            "case 1 SECURITY: tenant B's rows leaked into A: {ids:?}"
        ));
    }
    // The projection is the allowlist — nothing else is even selectable.
    let columns: Vec<&str> = rows[0].columns().iter().map(|c| c.name()).collect();
    let mut sorted = columns.clone();
    sorted.sort_unstable();
    if sorted != ["agent_name", "chat_id", "created_at", "telegram_message_id"] {
        return Err(format!("case 1: unexpected projection: {columns:?}"));
    }

    // ---- Case 2: tenant_with_null_chat_id_returns_empty_not_everything ----
    // The handler resolves the chat first and short-circuits on NULL. This
    // asserts the resolution really yields NULL, then that the naive
    // "optional filter" an implementer might have reached for — the shape
    // production deliberately does not contain — would have returned the whole
    // table. That second assertion is what makes this test about the leak
    // rather than about an empty result.
    let chat: Option<i64> = sqlx::query_scalar(TENANT_CHAT_SQL)
        .bind(unpaired)
        .fetch_one(pool)
        .await
        .map_err(|e| format!("case 2: tenant lookup failed: {e}"))?;
    if chat.is_some() {
        return Err(format!(
            "case 2: expected NULL telegram_chat_id, got {chat:?}"
        ));
    }
    let leaky: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM outbound_messages WHERE ($1::bigint IS NULL OR chat_id = $1)",
    )
    .bind(chat)
    .fetch_one(pool)
    .await
    .map_err(|e| format!("case 2: control query failed: {e}"))?;
    if leaky != 10 {
        return Err(format!(
            "case 2: the optional-filter shape was expected to return all 10 rows (the leak \
             production short-circuits before), got {leaky}"
        ));
    }

    // ---- Case 3: since_and_until_bound_the_window ----
    let since = chrono::Utc::now() - chrono::Duration::days(2) - chrono::Duration::hours(1);
    let until = chrono::Utc::now() - chrono::Duration::hours(12);
    let rows = sqlx::query(OUTBOUND_PAGE_SQL)
        .bind(chat_a)
        .bind(since)
        .bind(Some(until))
        .bind(1000_i64)
        .bind(0_i64)
        .fetch_all(pool)
        .await
        .map_err(|e| format!("case 3: query failed: {e}"))?;
    let ids: Vec<i64> = rows
        .iter()
        .map(|r| r.get::<i64, _>("telegram_message_id"))
        .collect();
    // In-window: msg 2 (-2d) and msg 3 (-1d). Out: msg 1 (-3d, before `since`)
    // and the -6h burst (after `until`).
    if ids != vec![3, 2] {
        return Err(format!(
            "case 3: expected [3, 2] within the window, got {ids:?}"
        ));
    }

    // ---- Case 4: pagination_neither_duplicates_nor_skips_across_pages ----
    // Paged two at a time across the four-row burst that shares one timestamp.
    let mut seen: Vec<i64> = Vec::new();
    for page in 0..4_i64 {
        let rows = sqlx::query(OUTBOUND_PAGE_SQL)
            .bind(chat_a)
            .bind(week_ago)
            .bind(no_upper)
            .bind(2_i64)
            .bind(page * 2)
            .fetch_all(pool)
            .await
            .map_err(|e| format!("case 4: page {page} failed: {e}"))?;
        seen.extend(rows.iter().map(|r| r.get::<i64, _>("telegram_message_id")));
    }
    let mut unique = seen.clone();
    unique.sort_unstable();
    unique.dedup();
    if unique.len() != seen.len() {
        return Err(format!(
            "case 4: pagination duplicated a row across pages: {seen:?}"
        ));
    }
    let mut expected = vec![1_i64, 2, 3, 100, 101, 102, 103];
    expected.sort_unstable();
    if unique != expected {
        return Err(format!(
            "case 4: the union of the pages is not the whole set — got {unique:?}, want {expected:?}"
        ));
    }

    // ---- Case 5: a_served_read_writes_one_audit_row_naming_this_route ----
    // The production writer is `audit_events::log_admin_read`, which is
    // `pub(crate)` and not reachable from an integration test; this exercises
    // the row it emits against the real table and constraints.
    let target_key = format!("tenant:{tenant_a}");
    sqlx::query("INSERT INTO audit_events (tool_name, target_key, metadata) VALUES ($1, $2, $3)")
        .bind("gateway_admin_read")
        .bind(&target_key)
        .bind(serde_json::json!({
            "route": "GET /admin/tenants/{customer_id}/outbound-messages",
            "customer_id": tenant_a,
        }))
        .execute(pool)
        .await
        .map_err(|e| format!("case 5: audit insert failed: {e}"))?;

    let row = sqlx::query(
        "SELECT metadata->>'route' AS route FROM audit_events \
         WHERE tool_name = 'gateway_admin_read' AND target_key = $1",
    )
    .bind(&target_key)
    .fetch_one(pool)
    .await
    .map_err(|e| format!("case 5: audit read-back failed: {e}"))?;
    let route: String = row.get("route");
    if !route.ends_with("outbound-messages") {
        return Err(format!("case 5: audit row names the wrong route: {route}"));
    }
    if route.ends_with("recurring-tasks") {
        return Err("case 5: this read was recorded as a registry read".into());
    }

    Ok(())
}
