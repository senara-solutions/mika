---
title: "fix: Build global Telegram client on token presence — operator-agent /send route in multi-bot mode"
date: 2026-06-27
type: fix
artifact_contract: ce-unified-plan/v1
artifact_readiness: implementation-ready
product_contract_source: ce-plan-bootstrap
execution: code
issue: 1590
---

# fix: Build global Telegram client on token presence — operator-agent /send route in multi-bot mode

## Goal Capsule

Fix the half-migrated contract where operator agents (mika-dev, mika-arch, mika-qa) cannot deliver messages via the gateway `/send` endpoint in multi-bot mode because the global `TelegramClient` is only built when `MIKA_TELEGRAM_SINGLE_BOT_MODE` is enabled. Decouple outbound client construction from inbound webhook registration so a bot token alone is sufficient for outbound delivery. Add structured logging to the silent 400 arm.

---

## Problem Frame

Since the multi-bot migration (mika#1454), the gateway conditionally builds the global `TelegramClient` only when `MIKA_TELEGRAM_SINGLE_BOT_MODE` is enabled (`main.rs:90-117`). In the default per-customer mode, `state.telegram = None`. Operator agents send messages via `/send` with `{chat_id, text, agent_name}` but no `customer_id` — they're not customer-scoped. The `/send` handler's fallback branch (`routes.rs:1100-1106`) returns 400 when `state.telegram` is `None`, silently dropping all operator notifications.

Secondary defect: the 400 arm emits no structured log, making this failure class invisible gateway-side.

---

## Requirements

- R1. The gateway builds the global `TelegramClient` whenever `MIKA_TELEGRAM_BOT_TOKEN` is configured, regardless of `MIKA_TELEGRAM_SINGLE_BOT_MODE`.
- R2. `MIKA_TELEGRAM_SINGLE_BOT_MODE` exclusively controls inbound global webhook registration — it no longer gates outbound client construction.
- R3. When `MIKA_TELEGRAM_SINGLE_BOT_MODE` is off, `MIKA_TELEGRAM_BOT_TOKEN` alone is sufficient — `MIKA_TELEGRAM_WEBHOOK_SECRET` and `MIKA_TELEGRAM_WEBHOOK_URL` are not required.
- R4. When `MIKA_TELEGRAM_SINGLE_BOT_MODE` is on, all three (token + secret + URL) remain required for inbound webhook registration.
- R5. The 400 "no customer_id" arm emits a structured `warn!` with `agent_name`, `chat_id`, and `request_id`.
- R6. Per-customer bot lookup (with `customer_id` present) is unaffected.
- R7. Documentation reflects the semantic narrowing of `MIKA_TELEGRAM_SINGLE_BOT_MODE`.

---

## Key Technical Decisions

**KTD1. Two-phase client construction in `main.rs`.**
Build the global `TelegramClient` in an early phase (token presence only), then conditionally register the webhook in a second phase (single-bot mode). This keeps the existing `TelegramClient` struct unchanged and only restructures the conditional in `main.rs`. The `state.telegram` field becomes `Some` whenever a bot token exists, which is exactly what the `/send` fallback branch checks.

**KTD2. Validation split in `settings.rs`.**
The existing validation block (`settings.rs:124-152`) requires all three Telegram vars when single-bot mode is on. The fix leaves this block unchanged but does NOT add any new validation for the token-only case — if `MIKA_TELEGRAM_BOT_TOKEN` is set without single-bot mode, the token is simply used for outbound. No webhook secret/URL validation runs because those are inbound-only concerns.

**KTD3. `warn!` over `error!` for the 400 arm.**
The 400 arm in `/send` means "no global client AND no customer_id" — a misconfiguration or caller error, not an internal failure. `warn!` is the right severity. The response body already carries the error message for the caller.

---

## Scope Boundaries

### In Scope
- `main.rs` — restructure global client construction
- `settings.rs` — no code change needed (existing validation is already correct — it only fires when single-bot mode is on)
- `routes.rs` — add structured logging to the 400 arm
- `CLAUDE.md` — document the semantic narrowing
- Unit tests for settings validation and /send behavior

### Out of Scope
- Agent-side changes (the agent payload is already correct)
- Operator bot token provisioning (operator-host work, documented in the issue)
- Per-customer Telegram migration paths

### Deferred to Follow-Up Work
- None identified

---

## Implementation Units

### U1. Restructure global TelegramClient construction in main.rs

**Goal:** Build the global `TelegramClient` whenever `MIKA_TELEGRAM_BOT_TOKEN` is configured, independent of `MIKA_TELEGRAM_SINGLE_BOT_MODE`. The mode flag gates only the webhook registration call.

**Requirements:** R1, R2

**Dependencies:** None

**Files:**
- `crates/mika-gateway/src/main.rs`

**Approach:**
Replace the single `if single_bot_mode { ... } else { None }` block (lines 90-117) with two phases:
1. Build `TelegramClient` from `settings.telegram_bot_token` when present (no mode check). Store as `Option<TelegramClient>`.
2. If `single_bot_mode` is true AND the client was built, register the webhook using `tg.set_webhook(...)`. The webhook registration still requires `telegram_webhook_url` and `telegram_webhook_secret` (validated by settings when single-bot mode is on).

Update the `AppState` doc comment on the `telegram` field from "populated only in single-bot mode" to "populated when `MIKA_TELEGRAM_BOT_TOKEN` is configured".

**Patterns to follow:** The existing two-phase pattern used for `github_app` construction in the same file — build from credentials, then use conditionally.

**Test scenarios:**
- With `MIKA_TELEGRAM_BOT_TOKEN` set and `MIKA_TELEGRAM_SINGLE_BOT_MODE` unset: `state.telegram` is `Some` (global client built for outbound), no webhook registration occurs.
- With both token and single-bot mode enabled: `state.telegram` is `Some` AND webhook registration runs.
- With no bot token set: `state.telegram` is `None` regardless of mode flag.

**Verification:** `cargo build -p mika-gateway` succeeds. The info log changes from "per-customer Telegram bot mode — global webhook registration skipped" to a message reflecting that the global client is built but webhook registration is skipped.

---

### U2. Add structured logging to the /send 400 arm

**Goal:** Make the "no customer_id and no global client" failure visible in gateway logs.

**Requirements:** R5

**Dependencies:** None (can be done in parallel with U1)

**Files:**
- `crates/mika-gateway/src/routes.rs`

**Approach:**
In the `None` arm of `state.telegram.as_ref()` (around line 1100), add a `warn!` before returning the 400 response:
```
warn!(
    agent_name = ?payload.agent_name,
    chat_id = payload.chat_id,
    request_id = ?payload.request_id,
    "send failed: no customer_id provided and no global Telegram client configured"
);
```

**Patterns to follow:** The existing `error!` pattern at line 1093 (`customer lookup failed for /send`) which uses structured fields.

**Test scenarios:**
- `/send` with no `customer_id` and `state.telegram = None` → 400 response AND warn log emitted with `agent_name`, `chat_id`, `request_id` fields.

**Verification:** The log line appears in structured JSON output when the 400 path is hit.

---

### U3. Unit tests for settings validation contract

**Goal:** Verify that `MIKA_TELEGRAM_BOT_TOKEN` alone is sufficient when single-bot mode is off, and all three vars are required when single-bot mode is on.

**Requirements:** R3, R4

**Dependencies:** None

**Files:**
- `crates/mika-gateway/src/settings.rs`

**Approach:**
The existing `GatewaySettings::load()` validation already only fires when `telegram_single_bot_mode_is_enabled()` returns true. The token-only case needs no code change — it already works because no validation runs for non-single-bot-mode. Add tests that construct `GatewaySettings` directly and call the validation path to confirm:
1. Token-only + mode-off → success (no validation error).
2. Token-only + mode-on → validation error naming missing `MIKA_TELEGRAM_WEBHOOK_SECRET`.
3. All three + mode-on → success.

Since `GatewaySettings::load()` reads from env vars via `config-rs`, the cleanest approach is to test the validation logic extracted into a helper, or test via the existing `load()` with env var overrides. Follow the existing test pattern in settings.rs which tests individual helper functions.

**Patterns to follow:** Existing `test_single_bot_mode_*` and `test_orchestrator_inbox_*` test patterns in `settings.rs`.

**Test scenarios:**
- Load settings with `telegram_bot_token = Some(...)`, `telegram_single_bot_mode = None` → no validation error.
- Load settings with `telegram_bot_token = Some(...)`, `telegram_single_bot_mode = Some("1")`, `telegram_webhook_secret = None` → validation error containing "MIKA_TELEGRAM_WEBHOOK_SECRET".
- Load settings with all three Telegram vars set + `telegram_single_bot_mode = Some("1")` → success.

**Verification:** `cargo test -p mika-gateway` passes with new tests.

---

### U4. Update CLAUDE.md documentation

**Goal:** Document the semantic narrowing of `MIKA_TELEGRAM_SINGLE_BOT_MODE` and the global outbound client behavior.

**Requirements:** R7

**Dependencies:** U1

**Files:**
- `crates/mika-gateway/CLAUDE.md`

**Approach:**
Update the environment variables section:
1. `MIKA_TELEGRAM_BOT_TOKEN` — change from "Required only in single-bot mode" to document that it's used for the global outbound client whenever configured, independent of single-bot mode.
2. `MIKA_TELEGRAM_WEBHOOK_SECRET` — keep "Required only in single-bot mode" (unchanged, inbound-only).
3. `MIKA_TELEGRAM_WEBHOOK_URL` — keep "Required only in single-bot mode" (unchanged, inbound-only).
4. `MIKA_TELEGRAM_SINGLE_BOT_MODE` — update description to state it exclusively controls inbound global webhook registration. Document the semantic narrowing: pre-fix it gated both inbound registration and outbound client construction; post-fix it gates inbound only.

Also update the `AppState` doc comment reference if the CLAUDE.md mentions it.

**Patterns to follow:** Existing env var documentation style in the file.

**Test scenarios:**
- Test expectation: none — documentation-only change.

**Verification:** CLAUDE.md accurately reflects the new behavior. The three env vars have consistent documentation.

---

## Verification Contract

1. `cargo build -p mika-gateway` — compiles without warnings.
2. `cargo test -p mika-gateway` — all existing and new tests pass.
3. `cargo clippy -p mika-gateway` — no new warnings.
4. Manual verification scenario: with `MIKA_TELEGRAM_BOT_TOKEN` set and `MIKA_TELEGRAM_SINGLE_BOT_MODE` unset, the gateway startup log shows the global client was built but webhook registration was skipped.

---

## Definition of Done

- [ ] Global `TelegramClient` is built on token presence, not mode flag
- [ ] `MIKA_TELEGRAM_SINGLE_BOT_MODE` only gates inbound webhook registration
- [ ] 400 arm in `/send` emits structured `warn!` with agent_name/chat_id/request_id
- [ ] Settings validation: token-only + mode-off succeeds; token-only + mode-on fails
- [ ] Unit tests cover the settings validation contract
- [ ] CLAUDE.md documents the semantic narrowing
- [ ] `cargo build && cargo test && cargo clippy` all pass for mika-gateway

---

## Sources & Research

- Issue: senara-solutions/mika#1590
- Multi-bot migration: mika#1454 / PR #1455 (commit `ac53b73d`)
- `crates/mika-gateway/src/main.rs:87-117` — current conditional client construction
- `crates/mika-gateway/src/routes.rs:1061-1108` — /send handler fallback branch
- `crates/mika-gateway/src/settings.rs:124-152` — single-bot mode validation
- `crates/mika-gateway/src/telegram.rs:557-596` — TelegramClient struct
