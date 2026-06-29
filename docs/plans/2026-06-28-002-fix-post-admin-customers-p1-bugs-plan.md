# Plan: Fix POST /admin/customers P1 bugs (mika#1612)

## Summary

PR #1610 (mika#1609) shipped `POST /admin/customers` with two P1 defects (endpoint 500s on every call; silent inbound outage on active-customer re-register) plus P2/P3 nits. This plan fixes all six items from the ticket's acceptance criteria.

## Scope

**In scope:** AC1–AC7 from mika#1612.
**Out of scope:** `bot_token`/`bot_username` uniqueness constraint (separate ticket per issue body).

---

## Step 1 — P1 #1: Fix `make_interval` type mismatch (AC1)

**File:** `crates/mika-gateway/src/routes.rs:1032`

**Change:** `ttl_hours as f64` → `ttl_hours as i32`

Postgres `make_interval(hours => $N)` expects `int4`. sqlx binds `f64` as `float8` (OID 701), which is an assignment cast — not implicit — so function resolution fails. `i64 as i32` truncates values > 2^31, but TTL hours are capped at practical values (default 48); no additional clamp needed.

**Evidence:** Ticket includes a throwaway Postgres 16 repro confirming `float8` fails and `int4` works.

---

## Step 2 — P1 #2: Make webhook_secret rotation atomic with successful setWebhook (AC2)

**File:** `crates/mika-gateway/src/routes.rs:1006–1098`

**Problem:** The upsert unconditionally overwrites `webhook_secret` (line 1019), then calls `setWebhook` fail-open. If `setWebhook` fails for an active customer, the DB holds the new secret but Telegram still sends the old one → constant_time_eq mismatch → 401 on every inbound.

**Fix strategy — two-phase upsert:**

1. **Initial upsert** (existing SQL, modified): On the `ON CONFLICT` path, change `webhook_secret = EXCLUDED.webhook_secret` to preserve the existing secret:
   ```sql
   webhook_secret = CASE WHEN customers.status = 'provisioned' THEN EXCLUDED.webhook_secret ELSE customers.webhook_secret END
   ```
   Also add `RETURNING webhook_secret` so we know which secret the DB holds.

2. **After successful `setWebhook`:** If the customer was NOT newly inserted AND `setWebhook` succeeded, run a targeted UPDATE to rotate the secret:
   ```sql
   UPDATE customers SET webhook_secret = $1 WHERE id = $2
   ```
   This ensures the DB secret only changes when Telegram actually holds the new one.

3. **Webhook registration call:** Always call `setWebhook` with the **new** secret (so a new registration or a re-register-of-provisioned works on first try). For active customers where setWebhook fails, the old secret stays in the DB — inbound continues to work.

4. **Response `webhook_registered` field:** Already returns `false` on failure — no change needed. The semantic now correctly means "webhook was registered AND secret was rotated" for active customers.

**Key invariant:** After this change, `customers.webhook_secret` always matches what Telegram holds (or is NULL for customers never successfully registered). The DB is never ahead of Telegram.

---

## Step 3 — P2: Case-insensitive bot_username comparison (AC3)

**File:** `crates/mika-gateway/src/routes.rs:976`

**Change:** `actual_username != payload.bot_username` → `!actual_username.eq_ignore_ascii_case(&payload.bot_username)`

Telegram usernames are case-insensitive; `getMe` returns canonical casing. A caller passing different case should not get a 400.

---

## Step 4 — P3: Don't fabricate pairing_token for active customers (AC4)

**File:** `crates/mika-gateway/src/routes.rs:1075–1080`

**Problem:** When `row.pairing_token` is `None` (active customer — nulled by `handle_pairing`), `effective_token` falls back to the freshly-generated-but-never-persisted token. The response advertises a `pairing_url` that cannot pair.

**Fix:** Return `pairing_token` and `pairing_url` as `Option<String>` in `RegisterCustomerResponse`. When `row.pairing_token` is `None`, set both to `None` in the JSON response. This requires making those fields optional in the response struct.

If changing the response shape is too disruptive (no other consumers yet — this endpoint just shipped), an alternative is to return empty strings. But `Option` is the correct semantic — the endpoint just shipped and there are no downstream consumers to break.

---

## Step 5 — P3: Confirm and fix bot_token leak in setWebhook error log (AC6)

**File:** `crates/mika-gateway/src/routes.rs:1065` (the `warn!` call) and `crates/mika-gateway/src/telegram.rs:596`

**Analysis:** `set_webhook_impl` wraps reqwest errors in `anyhow::anyhow!("setWebhook request failed: {e}")`. reqwest 0.12's `Error::fmt` includes the URL (which contains `bot<TOKEN>`). The `warn!(error = %e, ...)` at routes.rs:1065 then logs this.

**Fix:** In `set_webhook_impl` (telegram.rs:596), replace `{e}` with a static error message that doesn't include the URL:
```rust
.map_err(|_| anyhow::anyhow!("setWebhook network request failed"))?;
```
This matches the `get_me` error path which already uses a safe static string (`"network error"`).

The Telegram API response parse error at line 601 is safe (response body, not URL).

---

## Step 6 — Integration test for upsert + active-customer re-register (AC5)

**File:** New test in `crates/mika-gateway/src/routes.rs` `#[cfg(test)] mod tests` (or a new integration test file if sqlx test harness is needed).

The ticket requires a DB-backed integration test. The gateway uses sqlx with Postgres; `#[sqlx::test]` with a test database is the standard approach.

**Test cases:**
1. **Fresh insert:** Upsert a new customer → verify `was_inserted = true`, status = `provisioned`, `webhook_secret` is set.
2. **Re-register provisioned customer:** Upsert again → verify `was_inserted = false`, pairing_token is regenerated.
3. **Re-register active customer, setWebhook success:** Verify secret IS rotated, pairing_token is NOT regenerated.
4. **Re-register active customer, setWebhook failure:** Verify secret is NOT rotated (old secret preserved), inbound validation still passes.

**Implementation note:** The sqlx test harness requires `DATABASE_URL` and runs migrations automatically. These tests exercise the SQL directly against Postgres — they don't mock. Mark with `#[sqlx::test]` and gate behind `#[cfg(test)]`. For the setWebhook success/failure paths, test the SQL logic (upsert + conditional update) rather than the full handler (which requires Telegram API mocking).

---

## Step 7 — Verify (AC7)

```bash
cargo clippy -p mika-gateway
cargo test -p mika-gateway
```

---

## File change summary

| File | Changes |
|------|---------|
| `crates/mika-gateway/src/routes.rs` | P1#1 bind fix; P1#2 two-phase upsert + conditional secret update; P2 case-insensitive comparison; P3 optional pairing_token in response; integration tests |
| `crates/mika-gateway/src/telegram.rs` | P3 redact URL from setWebhook error message |

## Risk assessment

- **P1 #1** is a one-character fix (`f64` → `i32`). Zero risk.
- **P1 #2** adds a second SQL statement on the success path. The window between upsert and UPDATE is short; a crash there leaves the old secret (safe — inbound works). The only risk is the UPDATE failing silently, which we mitigate by logging the error.
- **P2, P3, P3-log** are minor, isolated changes.
- **Integration tests** require a Postgres test database (`sqlx::test` handles this automatically via `DATABASE_URL`).

## Acceptance criteria

- **AC1** — `make_interval(hours => $9)` bind: change `ttl_hours as f64` → `ttl_hours as i32` at `crates/mika-gateway/src/routes.rs:1032`. Endpoint returns 2xx for provisioning paths instead of 500.
- **AC2** — `webhook_secret` rotation atomic with successful `setWebhook`: two-phase upsert preserves existing secret on conflict when status is not `provisioned`; rotation UPDATE runs only after `setWebhook` returns 2xx.
- **AC3** — Case-insensitive `bot_username` comparison in the active-customer guard.
- **AC4** — Pairing-token fabrication suppressed for active customers.
- **AC5** — Integration test exercises upsert + active-customer re-register (make_interval bind, secret rotation, idempotent re-register).
- **AC6** — `setWebhook` error log redacts the bot-token URL.
- **AC7** — `cargo build -p mika-gateway` and `cargo test -p mika-gateway` pass.
