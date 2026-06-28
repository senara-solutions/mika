# Plan: Fix POST /admin/customers P1 bugs + P2/P3 nits

**Issue:** mika#1612
**Type:** Bug fix (P1 x2, P2 x1, P3 x2)
**Component:** mika-gateway (`crates/mika-gateway/src/routes.rs`, `crates/mika-gateway/src/telegram.rs`)

## Problem

PR #1610 (mika#1609) shipped `POST /admin/customers` with two P1 defects that make the endpoint non-functional and one silent-outage edge case:

1. **P1 #1:** `make_interval(hours => $9)` bound as `f64` — Postgres `make_interval` requires `int4`; the type mismatch causes every upsert to 500.
2. **P1 #2:** Unconditional `webhook_secret` rotation on re-register + fail-open `setWebhook` — if Telegram `setWebhook` fails for an active customer, the DB now holds a new secret that Telegram doesn't know, causing 401 on every inbound message.
3. **P2:** Case-sensitive `getMe` username comparison — Telegram usernames are case-insensitive.
4. **P3:** Fabricated `pairing_token` in response for active customers — `effective_token` falls back to a never-persisted token.
5. **P3:** Potential `bot_token` leak in `set_webhook_impl` error path — reqwest `Error` Display may include the URL containing the token.

## Solution

All fixes in `crates/mika-gateway/src/routes.rs` with one ancillary fix in `telegram.rs`. Plus a Postgres-backed integration test for the upsert path.

## Implementation steps

### Step 1: Fix `make_interval` type bind (P1 #1)

**File:** `crates/mika-gateway/src/routes.rs:1032`

Change:
```rust
.bind(ttl_hours as f64)
```
To:
```rust
.bind(ttl_hours as i32)
```

`make_interval(hours => $9)` requires `int4`. The `i64 → i32` narrowing is safe because TTL hours is bounded to reasonable values (default 48, typical range 1–8760). Add an early clamp/validation at the top of the handler to reject values outside `1..=8760` with a `400`:

```rust
let ttl_hours = payload.pairing_token_ttl_hours.unwrap_or(48);
if !(1..=8760).contains(&ttl_hours) {
    return (StatusCode::BAD_REQUEST, Json(json!({"error": "pairing_token_ttl_hours must be 1-8760"}))).into_response();
}
```

Then bind as `i32`:
```rust
.bind(ttl_hours as i32)
```

### Step 2: Conditional webhook_secret rotation (P1 #2)

**File:** `crates/mika-gateway/src/routes.rs` — upsert SQL (lines 1013-1022) and surrounding logic (lines 1048-1073)

The fix has two parts:

**Part A — Guard `webhook_secret` in the upsert SQL (like `pairing_token`):**

Change the `ON CONFLICT` clause from:
```sql
webhook_secret = EXCLUDED.webhook_secret,
```
To:
```sql
webhook_secret = CASE WHEN customers.status = 'provisioned' THEN EXCLUDED.webhook_secret ELSE customers.webhook_secret END,
```

This preserves the working `webhook_secret` for `active` customers. For `provisioned` customers (not yet paired), rotating the secret is safe because no webhook is actively delivering.

**Part B — On successful `setWebhook`, update `webhook_secret` for active customers:**

After the `setWebhook` call succeeds (line 1062), run a targeted UPDATE to atomically rotate the secret:

```rust
if webhook_registered && !row.was_inserted && row.status != "provisioned" {
    // setWebhook succeeded — now safe to persist the new secret
    let _ = sqlx::query("UPDATE customers SET webhook_secret = $1 WHERE id = $2")
        .bind(&webhook_secret)
        .bind(payload.customer_id)
        .execute(&state.pool)
        .await
        .map_err(|e| {
            error!(error = %e, customer_id = %payload.customer_id,
                "failed to update webhook_secret after successful setWebhook");
        });
}
```

This ensures the DB secret and Telegram's secret are always in sync:
- **New customer (INSERT):** `webhook_secret` is set in the INSERT. `setWebhook` is called. If it fails, the customer is `provisioned` (no inbound traffic yet), so the mismatch is harmless — they'll re-register.
- **Re-register `provisioned`:** Secret is rotated in the upsert (safe, no active inbound). `setWebhook` called after.
- **Re-register `active`:** Secret is preserved in upsert. If `setWebhook` succeeds with the new secret, the UPDATE atomically writes it. If `setWebhook` fails, the old secret remains — inbound keeps working.

To support the post-`setWebhook` UPDATE, we need the `status` field from `UpsertCustomerRow` (already returned by the RETURNING clause, just currently `#[allow(dead_code)]`). Remove the `#[allow(dead_code)]` on `status`.

### Step 3: Case-insensitive username comparison (P2)

**File:** `crates/mika-gateway/src/routes.rs:976`

Change:
```rust
if actual_username != payload.bot_username {
```
To:
```rust
if !actual_username.eq_ignore_ascii_case(&payload.bot_username) {
```

Telegram usernames are ASCII-only (alphanumeric + underscore), so `eq_ignore_ascii_case` is correct.

### Step 4: Fix fabricated pairing_token for active customers (P3)

**File:** `crates/mika-gateway/src/routes.rs:1076`

The current code falls back to the locally-generated (never-persisted) token when `row.pairing_token` is `None`:
```rust
let effective_token = row.pairing_token.unwrap_or_else(|| pairing_token.clone());
```

For `active` customers, `pairing_token` is NULL in the DB (cleared at pairing). The response should reflect this:

```rust
let effective_token = row.pairing_token.clone();
let pairing_url = effective_token.as_ref().map(|t| {
    format!("https://t.me/{}?start={}", payload.bot_username, t)
});
```

Update `RegisterCustomerResponse` to use `Option<String>` for both `pairing_token` and `pairing_url`:
```rust
struct RegisterCustomerResponse {
    customer_id: Uuid,
    bot_username: String,
    pairing_token: Option<String>,
    pairing_url: Option<String>,
    webhook_registered: bool,
}
```

With `#[serde(skip_serializing_if = "Option::is_none")]` so the fields are omitted (not `null`) when the customer is active.

### Step 5: Fix bot_token leak in setWebhook error (P3)

**File:** `crates/mika-gateway/src/telegram.rs:596`

The error message includes `{e}` from reqwest, which may contain the request URL (including the bot token in the path). Replace with a sanitized message:

```rust
.map_err(|_| anyhow::anyhow!("setWebhook network request failed"))?;
```

This matches the `get_me` error pattern (line ~548) which already returns a static `"network error"` string. The Telegram API error response body (parsed from JSON) is still surfaced — only the reqwest transport error is sanitized.

Similarly, sanitize the JSON parse error path (line ~600):
```rust
.map_err(|_| anyhow::anyhow!("setWebhook response parse failed"))?;
```

### Step 6: Integration test for the upsert SQL

**File:** `crates/mika-gateway/tests/admin_customers.rs` (new file)

Add a Postgres-backed integration test using `sqlx::test` that:

1. Runs migrations 001+008 to set up the schema.
2. Executes the upsert SQL directly (not through the HTTP handler — the goal is to verify the SQL binds correctly against Postgres).
3. Tests three scenarios:
   - **Insert new customer:** `was_inserted = true`, `status = 'provisioned'`, `pairing_expires_at` is set.
   - **Re-register provisioned customer:** `was_inserted = false`, `pairing_token` and `webhook_secret` are rotated.
   - **Re-register active customer:** `was_inserted = false`, `pairing_token` and `webhook_secret` are preserved (the CASE guards work).

This covers the `make_interval` bind (would have caught P1 #1) and the conditional rotation logic (would have caught P1 #2).

The test uses `#[sqlx::test]` with the gateway's migration path. Add `sqlx` to dev-dependencies if not already present (it is — `sqlx` is a workspace dependency used in the main code; the test just needs access to the `#[sqlx::test]` macro via `features = ["runtime-tokio", "postgres"]`).

Guard the test with `#[cfg(feature = "integration")]` or `#[ignore]` + a clear comment explaining that it requires a Postgres connection. The `#[sqlx::test]` macro handles database provisioning automatically (creates an ephemeral test database).

### Step 7: Verify

- `cargo clippy -p mika-gateway` — clean.
- `cargo test -p mika-gateway` — unit tests pass.
- If a test Postgres is available: `cargo test -p mika-gateway --test admin_customers` — integration test passes.

## Files changed

| File | Change |
|------|--------|
| `crates/mika-gateway/src/routes.rs` | Steps 1-4: fix `i32` bind, conditional `webhook_secret`, case-insensitive username, `Option` pairing fields |
| `crates/mika-gateway/src/telegram.rs` | Step 5: sanitize `set_webhook_impl` error messages |
| `crates/mika-gateway/tests/admin_customers.rs` | Step 6: new integration test file |

## Risk assessment

- **Low risk:** Steps 1, 3, 5 are one-line fixes with clear before/after behavior.
- **Medium risk:** Step 2 changes the upsert SQL and adds a post-`setWebhook` UPDATE. The two-phase approach (preserve in upsert, update after success) is the simplest shape that guarantees secret consistency. The UPDATE is idempotent and targets a single row by PK.
- **Low risk:** Step 4 changes the response shape (`String` → `Option<String>`). This is a breaking API change, but the endpoint is new (shipped in #1610) and has no consumers yet.
- **Low risk:** Step 6 adds a test file with no production code changes.

## Out of scope

- `bot_token`/`bot_username` uniqueness constraint (per ticket: file separately).
- `DELETE`/`GET /admin/customers`.
- HTTP-level integration tests (Axum test harness with mock Telegram) — the SQL-level test catches the bind/logic bugs; full HTTP tests are a separate enhancement.
