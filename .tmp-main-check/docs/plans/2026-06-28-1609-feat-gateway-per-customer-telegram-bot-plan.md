# Plan: Gateway per-customer Telegram bot registration capability

**Issue:** mika#1609
**Type:** Feature (enhancement)
**Component:** mika-gateway

## Problem

Creating a per-customer Telegram bot registration in the gateway DB has **no code path** — it requires raw SQL `INSERT` + manual Telegram `setWebhook` + hand-issued pairing link. This blocks the console self-serve onboarding flow (companion milestone: senara-solutions/mika-cloud#6).

**Evidence from codebase:**
- No `INSERT INTO customers` exists in production code — only `UPDATE` on pairing (`routes.rs:900-907`).
- `generate_pairing_token()` exists only in `#[cfg(test)]` (`routes.rs:1393-1398`).
- Per-customer `setWebhook` has no production path — `main.rs:96-111` registers only the single-bot global webhook.
- The per-customer inbound + pairing handlers already work (`/webhook/telegram/{customer_id}` → `handle_customer_webhook`), but nothing creates the `provisioned` row they consume.
- Migration 008 already added `bot_token`/`bot_username`/`webhook_secret` to `customers`.

## Solution

Add `POST /admin/customers` — an internal-token-authed endpoint that:
1. Creates a `customers` row with `status='provisioned'`.
2. Stores `bot_token`, `bot_username`.
3. Generates `webhook_secret` (32 random bytes, hex-encoded → 64-char hex).
4. Generates `pairing_token` (32 random bytes, hex-encoded → 64-char hex) + sets `pairing_expires_at`.
5. Calls Telegram `setWebhook` for that bot, pointed at `/webhook/telegram/{customer_id}` with the per-customer secret.
6. Returns `{ customer_id, bot_username, pairing_token, pairing_url, webhook_registered }`.

Idempotent on `customer_id`: re-registering with the same UUID updates `bot_token`/`bot_username`, regenerates `webhook_secret` + re-registers webhook, and regenerates `pairing_token` only if the customer is still in `provisioned` status (not yet paired).

## Implementation steps

### Step 1: New setting — `gateway_external_url`

**File:** `crates/mika-gateway/src/settings.rs`

Add `gateway_external_url: Option<String>` to `GatewaySettings`. Maps to `MIKA_GATEWAY_EXTERNAL_URL`. This is the public HTTPS base URL of the gateway (e.g., `https://gateway.mika.example.com`). Required for per-customer webhook registration — the endpoint constructs `{gateway_external_url}/webhook/telegram/{customer_id}` as the Telegram webhook URL.

- Add to the struct (with `#[serde(default)]`, `Option<String>`).
- Add to the `Debug` impl.
- Add to test defaults (set a placeholder like `https://gateway.test.example.com`).
- No validation at load time — validated at use site (the endpoint returns an error if unset when called).

### Step 2: Promote `generate_pairing_token()` to production

**File:** `crates/mika-gateway/src/routes.rs`

Move the test-only `generate_pairing_token()` (line 1393-1398) out of `#[cfg(test)] mod tests` and into the module proper. Also add a `generate_webhook_secret()` function with identical implementation (32 random bytes → 64-char hex). Both use `rand::fill` + `hex::encode`.

```rust
/// Generate a cryptographic pairing token (32 random bytes, hex-encoded → 64 chars).
fn generate_pairing_token() -> String {
    let mut bytes = [0u8; 32];
    rand::fill(&mut bytes);
    hex::encode(bytes)
}

/// Generate a webhook secret (32 random bytes, hex-encoded → 64 chars).
/// Same format as pairing tokens — validated by `is_valid_pairing_token()` shape.
fn generate_webhook_secret() -> String {
    let mut bytes = [0u8; 32];
    rand::fill(&mut bytes);
    hex::encode(bytes)
}
```

The test code should continue to use the now-public `generate_pairing_token()` — remove the duplicate from the test module.

### Step 3: Add `gateway_external_url` to `AppState`

**File:** `crates/mika-gateway/src/routes.rs` (AppState struct) and `crates/mika-gateway/src/main.rs`

Add `gateway_external_url: Option<String>` to `AppState`. Plumb from `GatewaySettings` in `main.rs`.

### Step 4: Implement `POST /admin/customers` handler

**File:** `crates/mika-gateway/src/routes.rs`

#### Request payload

```rust
#[derive(Debug, Deserialize)]
struct RegisterCustomerPayload {
    /// Customer UUID (caller-provided, typically from console provisioning).
    customer_id: Uuid,
    /// Customer display name.
    name: String,
    /// Telegram bot token from @BotFather.
    bot_token: SecretString,
    /// Telegram bot username (without @).
    bot_username: String,
    /// Customer plan (optional, defaults to "standard").
    plan: Option<String>,
    /// Customer timezone (optional, defaults to "UTC").
    timezone: Option<String>,
    /// Pairing token TTL in hours (optional, defaults to 48).
    pairing_token_ttl_hours: Option<i64>,
}
```

#### Response payload

```rust
#[derive(Debug, Serialize)]
struct RegisterCustomerResponse {
    customer_id: Uuid,
    bot_username: String,
    pairing_token: String,
    pairing_url: String,
    webhook_registered: bool,
}
```

#### Handler logic

```rust
async fn handle_register_customer(
    State(state): State<AppState>,
    Json(payload): Json<RegisterCustomerPayload>,
) -> impl IntoResponse
```

1. **Validate `gateway_external_url`** is set in state — return `500 Internal Server Error` with `{"error": "gateway_external_url not configured"}` if missing. This is a deployment error, not a caller error.

2. **Validate `plan`** if provided — must be `"standard"` or `"premium"`. Return `400` if invalid.

3. **Validate `bot_username`** — alphanumeric + underscores, 1-32 chars, no leading `@`. Return `400` if invalid.

4. **Generate secrets:**
   - `webhook_secret = generate_webhook_secret()` (always, even on re-register)
   - `pairing_token = generate_pairing_token()`
   - `pairing_expires_at = now() + ttl` (default 48 hours)

5. **Upsert the customer row** (idempotent on `customer_id`):
   ```sql
   INSERT INTO customers (id, name, plan, timezone, status, bot_token, bot_username, webhook_secret, pairing_token, pairing_expires_at)
   VALUES ($1, $2, $3, $4, 'provisioned', $5, $6, $7, $8, $9)
   ON CONFLICT (id) DO UPDATE SET
       name = EXCLUDED.name,
       bot_token = EXCLUDED.bot_token,
       bot_username = EXCLUDED.bot_username,
       webhook_secret = EXCLUDED.webhook_secret,
       -- Only regenerate pairing if still provisioned (not yet paired)
       pairing_token = CASE WHEN customers.status = 'provisioned' THEN EXCLUDED.pairing_token ELSE customers.pairing_token END,
       pairing_expires_at = CASE WHEN customers.status = 'provisioned' THEN EXCLUDED.pairing_expires_at ELSE customers.pairing_expires_at END
   RETURNING status, pairing_token
   ```
   The `RETURNING` clause gives us the effective `pairing_token` (which may be the old one if the customer is already `active`).

6. **Call Telegram `setWebhook`** for the per-customer bot:
   - URL: `{gateway_external_url}/webhook/telegram/{customer_id}`
   - Secret: the generated `webhook_secret`
   - Use `set_webhook_impl` (already exists in `telegram.rs`) via a new `CustomerTelegramClient` constructed from the provided `bot_token`.
   - On success: `webhook_registered = true`.
   - On Telegram API failure: log the error, set `webhook_registered = false`, still return `201`/`200` with the customer data — the DB row is created, webhook can be re-registered later. This fail-open approach prevents Telegram downtime from blocking provisioning.

7. **Construct `pairing_url`:** `https://t.me/{bot_username}?start={pairing_token}` — this is the standard Telegram deep-link format that triggers the `/start {token}` command.

8. **Return response:**
   - `201 Created` for new customers.
   - `200 OK` for re-registered (conflict-updated) customers.
   - Distinguish via the SQL upsert (check if INSERT or UPDATE occurred — use `xmax` system column or track via the RETURNING `status`).

#### Error responses

| Scenario | Status | Body |
|----------|--------|------|
| Missing/invalid JSON | 400 | Axum default |
| Invalid `plan` | 400 | `{"error": "invalid plan: must be 'standard' or 'premium'"}` |
| Invalid `bot_username` | 400 | `{"error": "invalid bot_username: ..."}` |
| `gateway_external_url` not configured | 500 | `{"error": "gateway_external_url not configured"}` |
| DB error | 500 | `{"error": "internal error"}` (details logged) |

### Step 5: Wire the route into the router

**File:** `crates/mika-gateway/src/routes.rs` (`build_router`)

Add the route with `require_bearer_token` middleware (same auth as `/send`):

```rust
.route(
    "/admin/customers",
    post(handle_register_customer)
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            require_bearer_token,
        ))
        .layer(RequestBodyLimitLayer::new(16 * 1024)),  // 16KB — small JSON payload
)
```

Route path rationale: `/admin/customers` signals this is an internal administration endpoint, not a public API. The `admin` prefix leaves room for future admin endpoints (`GET /admin/customers`, `DELETE /admin/customers/{id}`, etc.) without polluting the top-level namespace.

### Step 6: Expose `set_webhook_impl` for per-customer use

**File:** `crates/mika-gateway/src/telegram.rs`

`set_webhook_impl` is currently `async fn set_webhook_impl(...)` (module-private). Either:
- (a) Add a `set_webhook` method to `CustomerTelegramClient` that delegates to `set_webhook_impl`, mirroring `TelegramClient::set_webhook`. **Preferred** — keeps the API symmetric.
- (b) Make `set_webhook_impl` `pub(crate)` and call it directly from the handler.

Option (a):
```rust
impl CustomerTelegramClient {
    pub async fn set_webhook(&self, webhook_url: &str, webhook_secret: &str) -> anyhow::Result<()> {
        set_webhook_impl(&self.client, self.bot_token.expose_secret(), webhook_url, webhook_secret).await
    }
}
```

### Step 7: Add `getMe` validation (defense-in-depth)

**File:** `crates/mika-gateway/src/telegram.rs`

Before registering the webhook, call Telegram's `getMe` endpoint to validate the `bot_token` is valid and the returned `username` matches `bot_username`. This catches typos and revoked tokens early.

```rust
/// Validate a bot token by calling Telegram's getMe endpoint.
/// Returns the bot's username on success.
pub(crate) async fn get_me(client: &reqwest::Client, bot_token: &str) -> Result<String, TelegramApiError> {
    let resp = client
        .get(api_url(bot_token, "getMe"))
        .send()
        .await?;
    // Parse response, extract result.username
}
```

Call sequence in the handler: `getMe` → validate username match → `setWebhook`. If `getMe` fails or username mismatches, return `400` with a descriptive error (not `500` — this is a caller-provided-bad-input case).

### Step 8: Tests

**File:** `crates/mika-gateway/src/routes.rs` (in `mod tests`)

1. **Unit tests for `generate_pairing_token` and `generate_webhook_secret`** — verify 64-char hex output, uniqueness.

2. **Unit test for `bot_username` validation** — valid/invalid patterns.

3. **Integration test skeleton** — the full handler requires a Postgres pool and Telegram API mocking, which may be out of scope for this PR. Document the manual test procedure:
   - Start gateway with `MIKA_GATEWAY_EXTERNAL_URL` set.
   - `curl -X POST -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" -d '{"customer_id":"...", "name":"Test", "bot_token":"...", "bot_username":"..."}' http://localhost:3001/admin/customers`
   - Verify DB row creation, Telegram webhook registration, pairing URL.

### Step 9: Update CLAUDE.md and OpenAPI spec

1. **`crates/mika-gateway/CLAUDE.md`** — add `POST /admin/customers` to the endpoints table, add `MIKA_GATEWAY_EXTERNAL_URL` to the env vars section.
2. **`docs/openapi/gateway.yaml`** — add the endpoint schema if the OpenAPI spec exists and is maintained.

## Acceptance criteria

1. `POST /admin/customers` with valid payload creates a `provisioned` customer row with `bot_token`, `bot_username`, `webhook_secret`, `pairing_token`, `pairing_expires_at`.
2. The endpoint calls Telegram `setWebhook` for the per-customer bot with the correct URL and secret.
3. The endpoint returns `pairing_url` in `https://t.me/{bot_username}?start={token}` format.
4. Re-calling with the same `customer_id` is idempotent: updates bot credentials, re-registers webhook, preserves pairing state for already-active customers.
5. Auth: rejects requests without valid `MIKA_INTERNAL_TOKEN` bearer (same middleware as `/send`).
6. Telegram API failure is fail-open: customer row is created, `webhook_registered: false` is returned, no 500.
7. `getMe` validation catches invalid bot tokens and username mismatches with a 400 response.
8. `generate_pairing_token()` and `generate_webhook_secret()` are production code (not test-only).
9. Gateway CLAUDE.md updated with new endpoint and env var.
10. `cargo clippy` and `cargo test -p mika-gateway` pass.

## Out of scope

- `DELETE /admin/customers/{id}` (deactivation/removal) — separate ticket.
- `GET /admin/customers` (list/query) — separate ticket.
- Console integration (mika-cloud#6 SUB-B calling this endpoint) — companion work.
- WhatsApp or other channel adapters — future work.
- Rate limiting on the admin endpoint — overkill for internal-token-authed endpoints.

## Risk assessment

- **Telegram API availability:** Mitigated by fail-open design — DB row is created regardless of Telegram response. Webhook can be re-registered by calling the endpoint again.
- **Bot token secrecy:** `bot_token` is stored as plaintext in Postgres (same as `webhook_secret`). This is consistent with the existing schema (migration 008). Encrypting at rest is a future concern, not this PR's scope.
- **Pairing token entropy:** 32 random bytes (256 bits) via `rand::fill` — cryptographically sufficient.
