---
ticket: mika#1749
branch: feat/1749/gateway-telegram-account-unlink-re-pair
type: feat
scope: crates/mika-gateway
grooming: /mika-groom-ticket
---

# Plan — mika#1749 Telegram `/unlink` self-service + admin unlink endpoint

## Problem

Live incident 2026-07-09: a customer whose Telegram account was paired to a stale customer row hit `"This Telegram account is already linked to another account."` (Postgres 23505 on `customers.telegram_chat_id UNIQUE`) with **no product-side release valve**. Manual SQL (`UPDATE customers SET telegram_chat_id = NULL`) was the only escape.

The ticket asks for three things:
1. Bot `/unlink` command letting a paired user release their own binding, with confirmation.
2. Admin endpoint (same auth class as `POST /admin/customers`) to unlink a `chat_id` server-side.
3. Cross-customer re-pair flow when incoming chat_id is already bound elsewhere.

Per samidarko-claude's dispatch for this pass: **v1 ships (1) and (2) — SELF-unlink only. (3) is deferred to a gated follow-up ticket** because cross-customer moves touch consent semantics and the invite-chain identity model — a design conversation for Prime, not code to ship this weekend.

## Root cause

The 23505 branch at `crates/mika-gateway/src/routes.rs:1258-1276` composes a dead-end string and returns. No affordance. No downstream verbs the paired user can send. No admin surface to unbind server-side.

The gateway's Telegram command parser (`telegram.rs:149-217`) only recognizes `/start`. Any other slash-command (including `/unlink`) falls through to `ParsedMessage::Text` and gets forwarded to the agent container as free-text — the gateway never sees it as a command.

The admin surface (`POST /admin/customers`, `routes.rs:245`) has no lifecycle mutation verbs beyond register. No `PATCH`, no `DELETE`, no `/unlink`.

## Scope

### In scope for v1 (this PR)

**AC1 — Bot `/unlink` command with two-step confirmation.**

- Extend `ParsedMessage` (`crates/mika-gateway/src/telegram.rs:88-125`) with two new variants: `Unlink` and `UnlinkConfirm`.
- Extend `parse_update()` (`telegram.rs:149-217`) to recognize:
  - `text == "/unlink"` → `Unlink`
  - `text == "/unlink confirm"` → `UnlinkConfirm`
  - `text` starts with `/unlink ` but is neither of the above → `Unlink` variant (with a warning that the user should send exactly `/unlink confirm`) — keeps typos from silently no-oping.
- Extend `dispatch_parsed_message()` (`routes.rs:496-589`) to route both to new handlers.

Handler shapes:

- `handle_unlink(state, tg, chat_id)`:
  - Verify chat_id is currently paired (`SELECT id, status FROM customers WHERE telegram_chat_id = $1`).
  - If not paired: reply `"Your Telegram is not linked to any Mika account."` and return.
  - If paired: reply with a clear warning message:
    ```
    ⚠️ Unlinking will release your Telegram from this Mika account.
    You will need a new invite link from your admin to re-pair.
    This cannot be undone.

    To confirm, send:  /unlink confirm
    ```

- `handle_unlink_confirm(state, tg, chat_id)`:
  - Perform the atomic UPDATE:
    ```sql
    UPDATE customers SET telegram_chat_id = NULL
       WHERE telegram_chat_id = $1
     RETURNING id
    ```
  - If a row updates: reply `"✅ Unlinked. Your invite link (or a new one) will pair a fresh session when you're ready."` Log `info!(customer_id, chat_id, "customer self-unlinked telegram binding")`.
  - If no row updates (chat_id wasn't paired — user typed `/unlink confirm` cold or after already unlinking): reply `"Nothing to unlink. Send /unlink first if you meant to release a Telegram binding."`

**Stateless confirmation rationale.** The gateway does not currently have inline-keyboard / callback_query support (`SendMessagePayload` is plain-text only — `telegram.rs:288-292`). Adding that surface would be a larger change (extend `SendMessagePayload` with `reply_markup`, extend `TelegramUpdate` / `parse_update` for `update.callback_query`, add `answerCallbackQuery` API call). Two-step text confirmation (`/unlink` → warning → `/unlink confirm` → action) achieves the "with confirmation" requirement without any client-surface expansion, and the safety comes from the second command being explicit (nobody types `/unlink confirm` by accident). No new schema, no in-memory state, no cross-request session.

**AC2 — Admin unlink endpoint.**

Route: `POST /admin/customers/{customer_id}/unlink` — register alongside the existing `POST /admin/customers` at `routes.rs:245-254`.

- Path param: `customer_id: Uuid`.
- Body: empty (no payload needed for v1). Reject non-empty bodies with 400 — keeps future evolution (adding `reason: Option<String>` for audit) an additive change without ambiguity now.
- Auth: `require_bearer_token` middleware (`routes.rs:1282-1299`) — same class as `POST /admin/customers`. **No new secret, no new auth class.**
- Handler `handle_admin_unlink(State(state), Path(customer_id)) -> impl IntoResponse`:
  - SQL: `UPDATE customers SET telegram_chat_id = NULL WHERE id = $1 RETURNING telegram_chat_id AS previous_chat_id`.
  - Row found + previous_chat_id was Some: 200 `{"customer_id": "...", "previous_chat_id": <i64>, "unlinked_at": "<ISO>"}`. Log `info!(customer_id, previous_chat_id, "admin unlinked telegram binding")`.
  - Row found + previous_chat_id was None (already unbound): 200 `{"customer_id": "...", "previous_chat_id": null, "unlinked_at": "<ISO>"}`. Idempotent; no error.
  - Row not found (bad customer_id): 404 `{"error": "customer not found"}`.
- **No `deleteWebhook` call** in v1 — the ticket says "unlink," not "deprovision." The bot token stays configured; only the Telegram user binding drops. If the whole customer teardown is wanted, that's a separate `DELETE /admin/customers/{id}` or `POST /.../deprovision` endpoint (not scoped here).

**AC3 — Better user-facing copy on the 23505 dead-end.**

Update `routes.rs:1266`:

```rust
// Before
"This Telegram account is already linked to another account."

// After
"This Telegram account is already linked to another Mika account. \
 If it's an account you control, send /unlink from that account first, \
 then click your invite link again. Otherwise, contact support."
```

Adds one sentence of actionable follow-up. Same 23505 branch, same handler, no new code paths.

**AC4 — Tests.**

- **Parser tests** (`telegram.rs` `tests` mod, mirroring the existing `/start` tests at lines 795-861):
  - `/unlink` → `ParsedMessage::Unlink`.
  - `/unlink confirm` → `ParsedMessage::UnlinkConfirm`.
  - `/unlink foo` (unknown suffix) → `ParsedMessage::Unlink` (falls back to warning path).
  - `/unlink   confirm` (extra whitespace) → `ParsedMessage::UnlinkConfirm` (canonicalize by trimming/collapsing whitespace before matching).
  - `/unlinkxxx` → `ParsedMessage::Text` (not our command; forwarded to agent as-is).

- **Handler tests** in `crates/mika-gateway/tests/` — `#[ignore]`-gated DB-backed test (matches the existing pattern at `admin_customers.rs`, since `#[sqlx::test]` is unavailable per the gateway crate's CI shape). New file `crates/mika-gateway/tests/unlink.rs`:
  - Pair a customer via the same SQL path used by the existing test → verify `telegram_chat_id` is set.
  - Call the bot-side handler function directly (constructing an `AppState` with a live pool) with a fake `CustomerTelegramClient` mock that captures sent messages: assert `/unlink` sends the warning; assert `/unlink confirm` unlinks + sends the success reply.
  - Call the admin endpoint via `axum::Router::oneshot`: assert 200 + previous_chat_id + row unlinked in DB; second call idempotent with previous_chat_id=null; 404 on missing customer.

  **Fallback if `CustomerTelegramClient` mocking is prohibitive:** factor the "should we unlink now?" decision into a pure helper `classify_unlink_input(text: &str) -> UnlinkAction` and test the classifier at parser level (cheap, no gateway state). The DB-touching handler test then just asserts the SQL side-effect and skips the reply-capture assertion.

**AC5 — Docs.**

- `crates/mika-gateway/CLAUDE.md` — add row for the new admin endpoint in the endpoint table.
- No architecture-doc change needed.

**AC6 — Build & lint clean.**

- `cargo build --release -p mika-gateway`
- `cargo test -p mika-gateway` (non-ignored)
- `cargo clippy -p mika-gateway --all-targets -- -D warnings`

### Out of scope for v1 (deferred)

- **Cross-customer re-pair** (ticket ask #3 — "move my Telegram to this new account"). Requires:
  - Consent semantics: how does the previous customer's admin authorize the move?
  - Identity ledger: no history table exists (`crates/mika-gateway/migrations/*.sql` searched — no `previous_customer_id`, no `paired_to`, no `invite_chain`). Cross-customer moves are destructive today; a new `customer_telegram_bindings` history table would be needed.
  - Invite-chain identity model touch — flagged by ticket for Prime consultation.
  - **File as follow-up ticket alongside this plan, gated "do not activate until Prime signs off on consent shape."**
- **Inline-keyboard / callback_query confirmation UI.** Larger client-surface expansion; text-only two-step confirmation is sufficient for v1.
- **Audit log of unlinks.** No `unlink_events` table; adding one is orthogonal. If admin unlinks need attribution ("who ran the admin unlink"), that's an audit-events broader concern (see `event_log` / `audit_events` per D5 decision in the meta-repo).
- **Bot teardown** (`deleteWebhook`, bot token rotation, deprovisioning). Full customer teardown is a separate feature.
- **`/link` re-pair command** from within Telegram after unlink. The existing invite-link flow already handles re-pair — the user's admin (or self-service via console) reissues an invite. No new command needed.

## Implementation guardrails

### File and function targets

| Change | File | Location |
|---|---|---|
| Add `Unlink` and `UnlinkConfirm` variants to `ParsedMessage` | `crates/mika-gateway/src/telegram.rs` | Enum at ~line 88 |
| Extend `parse_update()` to recognize `/unlink` and `/unlink confirm` | `crates/mika-gateway/src/telegram.rs` | Function at line 149-217, after the `/start` arm |
| Add `handle_unlink()` and `handle_unlink_confirm()` | `crates/mika-gateway/src/routes.rs` | Near `handle_pairing()` ~line 1180 |
| Route new variants in `dispatch_parsed_message()` | `crates/mika-gateway/src/routes.rs` | Match at line 496-589 |
| Add `POST /admin/customers/{id}/unlink` route | `crates/mika-gateway/src/routes.rs` | Router build at line 245-254 |
| Add `handle_admin_unlink()` | `crates/mika-gateway/src/routes.rs` | After `handle_register_customer()` at line 1166 |
| Extend 23505 copy | `crates/mika-gateway/src/routes.rs` | Line 1266 (the literal string) |
| Endpoint table update | `crates/mika-gateway/CLAUDE.md` | Endpoint reference table |
| Parser unit tests | `crates/mika-gateway/src/telegram.rs` (tests mod) | Line 711+ |
| DB-backed handler tests | `crates/mika-gateway/tests/unlink.rs` (new) | New file, mirroring `admin_customers.rs` |

### `ParsedMessage` extension shape

```rust
pub enum ParsedMessage {
    // ... existing variants
    Unlink { chat_id: i64 },
    UnlinkConfirm { chat_id: i64 },
}
```

Both carry only `chat_id` — the handler queries `customers` for the rest.

### Parser canonicalization

Trim leading/trailing whitespace; collapse interior whitespace to single spaces; then match:

```rust
let canonical: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
match canonical.as_str() {
    "/unlink" => ParsedMessage::Unlink { chat_id },
    "/unlink confirm" => ParsedMessage::UnlinkConfirm { chat_id },
    s if s.starts_with("/unlink ") => ParsedMessage::Unlink { chat_id }, // typo → warning path
    _ => /* fall through to Text or existing variants */
}
```

### Backwards compatibility

- Schema: no change (no migration).
- Existing routes: unchanged behavior.
- Existing bot commands (`/start`): unchanged parsing precedence.
- Log format: additive `info!` lines only.
- Callers: none affected.

### Auth & security

- **Bot side:** authorization comes from Telegram's own account guarantee — only the paired user's Telegram account can send messages to that binding's chat_id. No new auth check needed.
- **Admin side:** existing `require_bearer_token` middleware. Same class as `POST /admin/customers`. Reject non-empty bodies with 400 to keep the API contract clean.
- **No PII in logs:** log `chat_id` (already an ID, not a message body) and `customer_id`. Do not log message text.

## Acceptance criteria

**AC1.** A paired user sending `/unlink` receives the warning message (containing the exact phrase `send: /unlink confirm`); their `telegram_chat_id` binding is NOT modified.

**AC2.** After receiving the warning, the same user sending `/unlink confirm` releases their binding (`telegram_chat_id` NULLed atomically) and receives the success message. A user sending `/unlink confirm` without a paired binding receives the "nothing to unlink" message.

**AC3.** `POST /admin/customers/{customer_id}/unlink` with a valid bearer token:
- Returns 200 with `{"customer_id", "previous_chat_id", "unlinked_at"}` on success.
- Returns 200 with `previous_chat_id: null` idempotently if already unlinked.
- Returns 404 for a non-existent customer.
- Returns 401 without/with invalid bearer token.

**AC4.** The 23505 dead-end string now includes actionable follow-up (mentions `/unlink`).

**AC5.** Parser unit tests cover `/unlink`, `/unlink confirm`, whitespace-canonicalized variants, and the "unknown suffix falls to warning" case.

**AC6.** `#[ignore]`-gated DB-backed handler and admin-endpoint tests cover the pair → unlink → verify-NULL flow, admin idempotence, and the 404 case.

**AC7.** `crates/mika-gateway/CLAUDE.md` endpoint table includes the new admin route.

**AC8.** `cargo build --release -p mika-gateway`, `cargo test -p mika-gateway`, `cargo clippy -p mika-gateway --all-targets -- -D warnings` all pass.

## Verification steps (post-implementation)

1. `cargo test -p mika-gateway parse_unlink` (parser tests, no DB) — green.
2. `MIKA_DATABASE_URL=... cargo test -p mika-gateway --test unlink -- --ignored` (DB-backed) — green.
3. `cargo clippy -p mika-gateway --all-targets -- -D warnings` — clean.
4. Manual (documented in PR body, not a CI gate):
   a. Local pair via existing invite flow.
   b. Send `/unlink` in Telegram → warning received.
   c. Send `/unlink confirm` → success reply; verify DB row has `telegram_chat_id = NULL`.
   d. `curl -X POST -H "Authorization: Bearer $MIKA_INTERNAL_TOKEN" http://localhost:PORT/admin/customers/<uuid>/unlink` → 200 with response body. Second call returns `previous_chat_id: null`. Bad UUID returns 404.
   e. Attempt to re-pair via a fresh invite link → succeeds (no 23505).

## Rollout

- Merge to `main` → next `make deploy` picks it up (no cluster ops).
- No customer-facing breaking change; `/unlink` is a new command, admin endpoint is additive.
- Watch: first 24h post-deploy, grep for `customer self-unlinked` and `admin unlinked` log lines to confirm both paths fire in real usage.

## Risks and mitigations

| Risk | Mitigation |
|---|---|
| A user sends `/unlink confirm` accidentally without prior `/unlink`. | The warning message on `/unlink` is the standard flow. `/unlink confirm` cold succeeds but nobody types the two-word command by accident — same shape as destructive CLI verbs (`git push --force-with-lease`). If evidence shows accidental cold-confirms in practice, add a per-`chat_id` in-memory TTL flag ("saw /unlink within 60s"). Deferred until measured. |
| Admin endpoint accidentally called on an active-in-use customer. | 200 idempotent behavior + `info!` log with `customer_id`, `previous_chat_id`. The follow-up path (re-pair via a fresh invite link) is already supported. No PII leaked. |
| Cross-customer re-pair isn't handled — a user with a stale-pair situation still hits 23505 via the invite-link path if they haven't `/unlink`ed the old one. | Copy update in AC3 makes the follow-up path explicit: "send /unlink from that account first." If the user no longer has access to the old Telegram account (lost phone), they contact support — an admin uses the endpoint. Full cross-customer consent flow is deferred (follow-up ticket). |
| A partially-typed command like `/unlinkoops` accidentally releases the binding. | Parser canonicalization is prefix-strict: `/unlink` (exact after trim) or `/unlink confirm` (exact after canonical whitespace) only. `/unlinkoops` does not match — falls through to Text and is forwarded to the agent unchanged. Test asserts this. |
| Admin endpoint auth relies on `MIKA_INTERNAL_TOKEN` — same secret as `/send`. Compromising it exposes both. | Documented risk of the existing shared internal-token model. Not new; not in-scope to redesign. If per-endpoint tokens are wanted, that's a separate auth-hardening ticket. |
| The invite-chain identity model may care about the fact that a Telegram was ever linked to a prior customer. | No history is kept today. If we later add one, the migration will backfill NULLs for pre-history rows. Additive change, forwards-compatible. |

## Files changed (expected)

- `crates/mika-gateway/src/telegram.rs` — enum variants + parser branches + parser tests. ~40 lines added.
- `crates/mika-gateway/src/routes.rs` — two bot handlers + one admin handler + route registration + string copy update. ~150 lines added.
- `crates/mika-gateway/CLAUDE.md` — one endpoint-table row.
- `crates/mika-gateway/tests/unlink.rs` — new integration file. ~200 lines.

Estimated diff: ~400 net lines added (majority is tests).

## Grooming history

- 2026-07-10 — `/ce:plan` draft
- 2026-07-10 — `mika-arch` first-pass review (session `ed46fdca-c74d-4067-8e77-29ead8daaab2`): **Disposition: ITERATE**. Single finding F1 — the pass-1 brief summarized the plan using scoped-bullet numbering rather than transcribing the `## Acceptance criteria` section, so the architect flagged the section as absent. All three uncertainties confirmed architecturally sound (stateless confirmation, POST verb, skip deleteWebhook). No plan revisions applied — communication fix, not a plan defect.
- 2026-07-10 — `mika-arch` second-pass review (same session): **Verdict: GROOMED**. AC section transcribed verbatim into the brief; architect confirmed "non-empty, concrete, and testable (AC1–AC8)."
