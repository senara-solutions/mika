---
title: "feat: Migrate all API key fields to SecretString"
type: feat
status: active
date: 2026-04-20
---

# feat: Migrate all API key fields to SecretString

## Overview

Migrate 14 API key and token fields in `Settings` from `Option<String>` to `Option<SecretString>` (secrecy crate). This eliminates accidental clone/log exposure paths at compile time and provides zeroize-on-drop for all secret material. The gateway crate already uses `SecretString` for all its secrets — this aligns the agent crate with that pattern.

## Problem Frame

The `Settings` struct has inconsistent secret handling: 4 fields use `SecretString` (`internal_token`, `dashboard_token`, `github_app_private_key`, `otlp_auth_header`) while 14 others hold secrets as plain `String`. Plain `String` fields can be accidentally cloned into logs, are not zeroized on drop, and receive none of the compile-time safety that `SecretString` provides. The manual `Debug` impl mitigates display-level leakage but cannot prevent accidental `.clone()` propagation.

## Requirements Trace

- R1. All secret fields in `Settings` use `SecretString` for compile-time exposure safety
- R2. No user-facing behavior change — config loading, CLI commands, and LLM provider construction work identically
- R3. `get_effective_value()` returns a presence marker (`"[SET]"`) for all secret fields, not raw values
- R4. The secret exposure boundary is at `Settings` accessor methods — downstream types (`ActiveLlmConfig`, `ModelSpec`, `ClaudeClient`, `ToolContext`) continue using `String`/`&str`
- R5. Existing tests pass without modification (all test fixtures use `None` for API key fields)

## Scope Boundaries

- This PR covers Layer 1 (type consistency) from issue #97 only
- Layer 2 (~/.mika/.env support) is already fully implemented — `dotenv.rs` module handles loading, atomic writes, 0600 permissions, per-agent isolation
- Layers 3-6 (SecretRef, keychain, secure input, shell sandboxing) are separate future work
- No changes to downstream types (`ActiveLlmConfig`, `ModelSpec`, `ClaudeClient`, `EmbeddingClient`, `ToolContext`, `InvestigateConfig`)
- No changes to the `dotenv.rs` module or `.env` file handling
- No changes to the `scrub_mika_env_vars` or `EXTRA_SCRUB_VARS` logic

## Context & Research

### Relevant Code and Patterns

- `crates/mika-gateway/src/settings.rs` — uses `SecretString` for ALL secrets (target pattern)
- `crates/mika-common/src/config.rs` — `Settings` struct, `provider_fields()`, `get_effective_value()`, manual `Debug` impl
- Existing `SecretString` fields: `internal_token` (line 605), `dashboard_token` (line 609), `github_app_private_key` (line 652), `otlp_auth_header` (line 693)
- `secrecy = { version = "0.10", features = ["serde"] }` already in workspace `Cargo.toml` — serde support enabled
- `provider_fields()` returns `(Option<&str>, Option<&str>, Option<&str>)` via `.as_deref()` — needs migration to `.as_ref().map(|s| s.expose_secret().as_str())`
- `agent_github_token()` and `resolve_github_token()` use `.as_deref()` — same pattern

### Institutional Learnings

- `docs/solutions/architecture-patterns/config-key-rename-across-layers.md` — 9-layer checklist for config changes
- `docs/solutions/security-issues/debug-log-secret-leakage-and-file-permissions.md` — "new path reflex" for secret handling
- `docs/solutions/security-issues/exec-handler-gh-token-injection.md` — scrub-then-inject pattern (unaffected by this change)
- `docs/solutions/architecture-patterns/per-agent-dotenv-config-injection.md` — `dotenv_to_toml()` produces inline TOML; config-rs deserializes into `SecretString` via serde feature (proven by existing fields)
- `todos/646-pending-p2-investigate-github-token-secretstring.md` — existing TODO for this subset of work

## Key Technical Decisions

- **Expose at Settings boundary:** `provider_fields()`, `agent_github_token()`, `make_embedding_client()`, and `resolve_github_token()` call `.expose_secret()` and return `&str`/`String`. Downstream types remain unchanged. This minimizes blast radius while still providing zeroize-on-drop and compile-time safety at the storage layer.
- **`get_effective_value()` returns `"[SET]"` for all secrets:** Currently inconsistent — `github_app_private_key` returns `"[SET]"` but `internal_token` exposes the raw value. After migration, all secret-flagged fields return `Some("[SET]".to_string())`. The display path already uses `ConfigKeyInfo.secret` to redact, so no programmatic caller needs the raw value through this function.
- **Keep manual `Debug` impl:** `SecretString`'s `Debug` prints `SecretString([REDACTED])` which is noisier than the current `Some("[REDACTED]")`. The manual impl stays for consistent formatting, but the `.as_ref().map(|_| "[REDACTED]")` pattern works identically on `Option<SecretString>` as on `Option<String>`.

## Open Questions

### Resolved During Planning

- **Should downstream types carry `SecretString`?** No — expose at Settings boundary. The secret is intentionally exposed when it leaves Settings into provider constructors and tool contexts. This is consistent with how `internal_token` is already used.
- **Should `InvestigateConfig` use `SecretString`?** No — it's a short-lived struct consumed within the same handler. The exposure-at-boundary strategy already applies.
- **Will config-rs serde work with `SecretString`?** Yes — proven by existing fields (`internal_token`, `dashboard_token`, `github_app_private_key`, `otlp_auth_header`) that already deserialize through config-rs with `secrecy`'s serde feature.

### Deferred to Implementation

- **Exact `kimi_api_key`/`qwen_api_key`/`minimax_api_key` field locations in Debug impl:** These newer provider fields may or may not be in the manual Debug impl — will verify during implementation.

## Implementation Units

- [x] **Unit 1: Migrate Settings field types**

**Goal:** Change all 14 remaining secret fields from `Option<String>` to `Option<SecretString>`

**Requirements:** R1

**Dependencies:** None

**Files:**
- Modify: `crates/mika-common/src/config.rs`

**Approach:**
- Change field type declarations for: `anthropic_api_key`, `openai_api_key`, `openrouter_api_key`, `groq_api_key`, `ollama_api_key`, `mistral_api_key`, `google_api_key`, `deepseek_api_key`, `minimax_api_key`, `kimi_api_key`, `qwen_api_key`, `brave_api_key`, `github_token`, `investigate_github_token`
- No serde attribute changes needed — `SecretString` with `serde` feature deserializes from string values transparently
- Compiler errors from this change drive all subsequent units

**Patterns to follow:**
- Existing `internal_token: Option<SecretString>` declaration at line 605

**Test scenarios:**
- Happy path: Settings deserializes correctly with `SecretString` fields from TOML config source
- Happy path: Settings deserializes correctly with `None` values for all secret fields
- Edge case: Empty string API key deserializes into `SecretString` (not `None`) — verify existing trim/filter logic handles this

**Verification:**
- All 14 fields declared as `Option<SecretString>` in the struct definition

- [x] **Unit 2: Update Settings accessor methods**

**Goal:** Update `provider_fields()`, `agent_github_token()`, `resolve_github_token()`, and `make_embedding_client()` to expose secrets at the boundary

**Requirements:** R2, R4

**Dependencies:** Unit 1

**Files:**
- Modify: `crates/mika-common/src/config.rs`

**Approach:**
- `provider_fields()`: Replace `.as_deref()` on api_key fields with `.as_ref().map(|s| s.expose_secret().as_str())`. Model and base_url fields are not secrets — they stay as `.as_deref()`.
- `agent_github_token()`: Same pattern — `.as_ref().map(|s| s.expose_secret().as_str())`
- `resolve_github_token()`: Update the `.as_deref()` chain
- `make_embedding_client()`: Change from `.filter(|k| !k.trim().is_empty()).and_then(|key| ... key.clone())` to `.map(|s| s.expose_secret()).filter(|k| !k.trim().is_empty()).and_then(|key| ... key.to_string())`
- `active_llm_config()` calls `provider_fields()` and converts to `String` — no change needed since `provider_fields()` still returns `Option<&str>`

**Patterns to follow:**
- `github_app.rs:79` — `github_app_private_key` exposed with `.expose_secret()`
- `telemetry.rs:60` — `otlp_auth_header` exposed with `.expose_secret()`

**Test scenarios:**
- Happy path: `provider_fields(ProviderKind::Anthropic)` returns the API key string when set
- Happy path: `active_llm_config()` produces correct `ActiveLlmConfig` with API key
- Happy path: `make_embedding_client()` returns `Some` when OpenAI key is set and non-empty
- Edge case: `make_embedding_client()` returns `None` when OpenAI key is whitespace-only
- Happy path: `agent_github_token()` returns exposed secret as `&str`

**Verification:**
- `cargo build` succeeds with no type errors in config.rs
- LLM provider construction path unchanged from caller perspective

- [x] **Unit 3: Update `get_effective_value()` for consistency**

**Goal:** Return `"[SET]"` for all secret-flagged fields instead of exposing raw values

**Requirements:** R3

**Dependencies:** Unit 1

**Files:**
- Modify: `crates/mika-common/src/config.rs`

**Approach:**
- For all 14 migrated fields plus the existing `internal_token` and `dashboard_token`: change to `.as_ref().map(|_| "[SET]".to_string())`
- This aligns with the existing `github_app_private_key` pattern and fixes the `internal_token` inconsistency (currently exposes raw value via `.expose_secret()`)
- `otlp_auth_header` should also use `"[SET]"` if it currently exposes (verify during implementation)

**Patterns to follow:**
- `github_app_private_key` at line 475-478: `.as_ref().map(|_| "[SET]".to_string())`

**Test scenarios:**
- Happy path: `get_effective_value("anthropic_api_key", &settings)` returns `Some("[SET]")` when key is configured
- Happy path: `get_effective_value("anthropic_api_key", &settings)` returns `None` when key is not configured
- Happy path: `get_effective_value("internal_token", &settings)` returns `Some("[SET]")` (behavior change from current raw exposure)
- Integration: `mika config list` displays `[SET]` for all secret fields (visual confirmation)

**Verification:**
- All secret-flagged fields in `get_effective_value()` return `"[SET]"` not raw values
- `ConfigKeyInfo.secret == true` entries match the fields using the `"[SET]"` pattern

- [x] **Unit 4: Update manual Debug impl and downstream consumers**

**Goal:** Ensure the manual Debug impl and any remaining consumers compile and behave correctly

**Requirements:** R2, R5

**Dependencies:** Units 1, 2, 3

**Files:**
- Modify: `crates/mika-common/src/config.rs` (Debug impl)
- Modify: `crates/mika-agent/src/server/investigate.rs` (InvestigateConfig construction)
- Modify: any other files with compiler errors from Unit 1 type change

**Approach:**
- Debug impl: The `.as_ref().map(|_| "[REDACTED]")` pattern works on both `Option<String>` and `Option<SecretString>` — no change needed unless kimi/qwen/minimax fields are missing from the impl
- `InvestigateConfig` construction: Change from `.clone()` to `.as_ref().map(|s| s.expose_secret().to_string())` for `investigate_github_token`
- Follow compiler errors to find any other access sites that need `.expose_secret()`
- Check `ToolContext` construction in mika-agent — `brave_api_key` and `github_token` are passed as `Option<&str>`, need `.expose_secret()` at the extraction point

**Patterns to follow:**
- Existing `.expose_secret()` call sites in `github_app.rs`, `telemetry.rs`

**Test scenarios:**
- Happy path: `format!("{:?}", settings)` shows `[REDACTED]` for all API keys (same as before)
- Happy path: Investigation panel creates GitHub issues when `investigate_github_token` is set
- Integration: `ToolContext` receives `brave_api_key` and `github_token` as `Option<&str>` — web search and GitHub operations work

**Verification:**
- `cargo build` succeeds across all crates (mika-common, mika-agent, mika-cli, mika-gateway)
- `cargo test` passes — all existing tests work without modification
- `cargo clippy` clean

- [x] **Unit 5: Add focused test coverage**

**Goal:** Add tests that verify SecretString serialization/deserialization through config-rs and the exposure boundary

**Requirements:** R1, R2, R5

**Dependencies:** Units 1-4

**Files:**
- Modify: `crates/mika-common/src/config.rs` (test module)
- Test: `crates/mika-common/src/config.rs` (inline `#[cfg(test)] mod tests`)

**Approach:**
- Add a test that constructs Settings via config-rs `Config::builder()` with a TOML source containing API keys, verifies the fields are `Some(SecretString)` and that `.expose_secret()` returns the expected value
- Add a test that verifies `get_effective_value()` returns `"[SET]"` for all secret-flagged keys
- Add a test that verifies the Debug output does not contain any raw API key values

**Patterns to follow:**
- Existing tests in the `config.rs` test module (use `clean_env()`, `serial_test`)

**Test scenarios:**
- Happy path: Config-rs deserializes TOML string with API key into `Option<SecretString>` and `.expose_secret()` returns the original value
- Happy path: `get_effective_value()` returns `Some("[SET]")` for every `ConfigKeyInfo` where `secret == true`
- Happy path: Debug formatting of Settings does not contain test API key values
- Edge case: Config-rs deserializes empty env var into `SecretString` containing empty string (not `None`)

**Verification:**
- All new tests pass with `cargo test -p mika-common`

## System-Wide Impact

- **Interaction graph:** Settings -> `provider_fields()` -> `active_llm_config()` -> LLM providers. Settings -> `ToolContext` -> tools. Settings -> `InvestigateConfig` -> investigation panel. Settings -> `make_embedding_client()` -> vector search. All exposure happens at Settings accessor methods.
- **Error propagation:** No change — the migration is a type-level refactor. Runtime behavior is identical.
- **State lifecycle risks:** None — `SecretString` is zeroized on drop, which is strictly safer than `String`.
- **API surface parity:** The `mika config get/list` CLI output changes for `internal_token` (was raw value, now `[SET]`). This is a security improvement, not a regression.
- **Unchanged invariants:** `ActiveLlmConfig`, `ModelSpec`, `ClaudeClient`, `EmbeddingClient`, `ToolContext`, `InvestigateConfig` — all continue to use `String`/`&str` for API keys. The scrub-then-inject pattern in `executor.rs` is unaffected (operates on process env, not Settings fields). `dotenv.rs` module unchanged.

## Risks & Dependencies

| Risk | Mitigation |
|------|------------|
| Config-rs serde compatibility | Already proven by 4 existing `SecretString` fields; add focused test in Unit 5 |
| Missed access site causing compile error | Compiler-driven — `Option<SecretString>` is not `Option<String>`, all mismatches are compile errors |
| `get_effective_value` behavior change for `internal_token` | Intentional security improvement; no known programmatic consumer of raw values through this function |

## Sources & References

- Related issue: #97 (Design: Secret Management & Shell Sandboxing)
- Related TODO: `todos/646-pending-p2-investigate-github-token-secretstring.md`
- Gateway pattern: `crates/mika-gateway/src/settings.rs` (all secrets as SecretString)
- Institutional: `docs/solutions/architecture-patterns/config-key-rename-across-layers.md`
- Institutional: `docs/solutions/security-issues/debug-log-secret-leakage-and-file-permissions.md`
