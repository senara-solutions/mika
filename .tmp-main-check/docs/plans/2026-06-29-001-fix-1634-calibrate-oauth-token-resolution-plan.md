# Plan: fix(calibrate): OAuth-token resolution fails in calibrate binary

**Issue:** mika#1634
**Type:** fix
**Branch:** `fix/1634/calibrate-oauth-token-resolution-fails`

## Problem

The `calibrate` binary fails when `MIKA_ANTHROPIC_API_KEY` is a subscription token (`sk-ant-oat*`), even when `~/.mika/oauth.json` contains valid tokens that mika-spirit uses successfully on the same host.

Two observed failure modes:
1. Subscription token in env → OAuth resolution succeeds → API call returns generic "unexpected error" (HTTP error, non-200 status)
2. Raw access token passed as API key → hash mismatch with subscription token → "OAuth token resolution failed"

## Root Cause

The `calibrate` binary (`crates/mika-agent/src/bin/calibrate.rs`) skips the standard initialization sequence that mika-spirit performs:

```
mika-spirit:                          calibrate:
─────────────                         ──────────
1. resolve_home_dir()                 1. (skipped)
2. load_dotenv(&home_dir)             2. (skipped — user manually sources .env)
3. check_env_warnings(&home_dir)      3. (skipped)
4. Settings::load(&home_dir)          4. (skipped)
5. create provider via Settings       5. create_provider_from_spec() — reads env directly
```

The `create_provider_from_spec()` at `providers.rs:80` reads `MIKA_ANTHROPIC_API_KEY` from env, passes it to `create_provider()`, which creates `AnthropicProvider::new()`, which creates `ClaudeClient::new()`. Inside `ClaudeClient::new()` (claude.rs:372-374), the OAuth prefix `sk-ant-oat` is detected, `resolve_home_dir()` is called, and an `OAuthTokenManager` is constructed with the resolved home dir.

The home dir resolution works. The token loading works. The issue is that the API call itself fails — the `check_health()` preflight at `calibrate.rs:98` sends a real API request via the same `ClaudeClient::send_message()` used by mika-spirit, but Anthropic returns a non-success HTTP status.

The likely cause of the API-level failure is missing dotenv loading: when `load_dotenv()` is not called, secondary env vars that `Settings` would normally populate may be absent or have different values. Even though the user manually `source`d the `.env` file, subtle differences may exist (e.g., the dotenv loader handles quoting and multiline values differently than shell `source`). Additionally, without `Settings::load()`, the calibrate binary may be using stale or missing provider configuration that mika-spirit picks up from `config.toml`.

## Requirements

1. The calibrate binary must perform the same initialization sequence as mika-spirit before creating a provider
2. OAuth token resolution must work identically in both binaries
3. The `check_health()` preflight must succeed when mika-spirit can call the same model
4. The fix must not change the calibrate binary's CLI interface or output format

## Changes

### File 1: `crates/mika-agent/src/bin/calibrate.rs`

**What:** Add proper initialization before provider creation, matching mika-spirit's startup sequence.

**Before (line 52-78):**
```rust
#[tokio::main]
async fn main() {
    let args = Args::parse();

    // Validate role
    let scenarios = match args.role.as_str() { ... };

    // Create provider
    let provider = match create_provider_from_spec(&args.model) { ... };
```

**After:**
```rust
#[tokio::main]
async fn main() {
    let args = Args::parse();

    // Initialize environment — same sequence as mika-spirit
    let home_dir = match mika_common::home::resolve_home_dir() {
        Ok(h) => h,
        Err(e) => {
            eprintln!("Error: could not resolve Mika home directory: {e}");
            std::process::exit(2);
        }
    };
    mika_common::dotenv::load_dotenv(&home_dir);

    // Validate role
    let scenarios = match args.role.as_str() { ... };

    // Create provider
    let provider = match create_provider_from_spec(&args.model) { ... };
```

The key additions:
- `resolve_home_dir()` — ensures `MIKA_HOME` / `~/.mika` is resolved before anything else
- `load_dotenv(&home_dir)` — loads `~/.mika/.env` into the process environment, ensuring all API keys and config values are available to `create_provider_from_spec()` which reads from `std::env::var()`

We deliberately do NOT call `Settings::load()` because:
- The calibrate binary intentionally takes its model spec from the CLI (`--model` flag), not from Settings
- `Settings::load()` would pull in the full config stack, which is unnecessary for calibration
- The provider construction path (`create_provider_from_spec → create_provider → AnthropicProvider::new → ClaudeClient::new`) only needs the API key from env, which `load_dotenv()` ensures is loaded

We also do NOT call `check_env_warnings()` because:
- That function actively removes `GH_TOKEN` from the environment (defense-in-depth for agent sessions) — not relevant for calibration
- Its other warnings are server-specific (port conflicts, log file paths)

### File 2: `crates/mika-agent/src/bin/calibrate.rs` — doc comment

**What:** Document the OAuth auth contract in the module-level doc comment.

Add to the module doc comment (lines 1-8):
```rust
//! ## Authentication
//!
//! The binary performs the same dotenv initialization as mika-spirit (`resolve_home_dir`
//! + `load_dotenv`) before creating the LLM provider. For Anthropic OAuth tokens
//! (`sk-ant-oat*`), this ensures the `OAuthTokenManager` can resolve the token cache
//! at `~/.mika/oauth.json`. The `check_health()` preflight (below) verifies auth
//! before running scenarios — if mika-spirit can call Anthropic on this host, so can
//! `calibrate`.
```

This satisfies AC4 from the issue.

## Verification Contract

- **AC1 — Reproduce:** Confirmed by issue author. The bug is present on `main` today.
- **AC2 — Post-fix:** `make calibrate-mika-arch MODEL=anthropic/claude-sonnet-4-6` must succeed on a host where mika-spirit calls Anthropic successfully. The `check_health()` preflight passes, scenarios run.
- **AC3 — Operator can calibrate Anthropic models:** Follows from AC2.
- **AC4 — Doc comment:** The OAuth auth contract is documented in the calibrate binary's module-level doc comment.
- **Build:** `cargo build --bin calibrate` succeeds.
- **Tests:** `cargo test -p mika-agent -- calibrate` passes. Existing `test_oauth_token_creates_provider` test at `providers.rs:189` verifies provider construction with OAuth-prefix keys.
- **Clippy:** `cargo clippy --bin calibrate` clean.

## Definition of Done

- [ ] `resolve_home_dir()` + `load_dotenv()` called before provider creation in `calibrate.rs`
- [ ] Module doc comment documents the OAuth auth contract
- [ ] `cargo build --bin calibrate` succeeds
- [ ] `cargo clippy --bin calibrate` clean
- [ ] Existing tests pass (`cargo test -p mika-agent`)

## Acceptance criteria

- AC1 — Reproduce: `make calibrate-mika-arch MODEL=anthropic/claude-sonnet-4-6` fails on a host where mika-spirit calls Anthropic successfully for the same model (current state).
- AC2 — Post-fix: same command succeeds (auth passes, scenarios run, pass/fail per scenario, summary report written).
- AC3 — Operator can swap any agent role to a candidate Anthropic model with calibration evidence — no longer blocked on OAuth resolution.
- AC4 — Document the calibrate binary's OAuth path in `crates/mika-agent/src/bin/calibrate.rs` doc comment so future operators understand the dual-binary (mika-spirit vs calibrate) auth contract.

## Risks

- **Low:** The fix adds two function calls (`resolve_home_dir`, `load_dotenv`) that are well-tested and used by every other binary in the project (mika-spirit, mika CLI). No new dependencies or architectural changes.
- **Edge case:** If `resolve_home_dir()` fails (no `HOME` env var, no `MIKA_HOME`), the binary exits with a clear error message before any provider construction. This is the correct behavior — calibration requires a functioning Mika installation.
