---
title: SecretString expose-at-boundary pattern for config secrets
date: 2026-04-20
category: best-practices
module: mika-common/config.rs
problem_type: best_practice
component: tooling
severity: medium
applies_when:
  - Adding a new API key or token field to Settings
  - Migrating an existing Option<String> secret field to SecretString
  - Accessing secret fields from Settings in downstream code
tags: [secretstring, secrecy, api-key, config, expose-at-boundary, zeroize]
---

# SecretString expose-at-boundary pattern for config secrets

## Context

The `Settings` struct stores all configuration including API keys and tokens. Originally, most secret fields used `Option<String>` while only a few (`internal_token`, `dashboard_token`, `github_app_private_key`, `otlp_auth_header`) used `secrecy::SecretString`. This inconsistency meant plain `String` API keys could be accidentally cloned into logs, were not zeroized on drop, and received none of the compile-time safety that `SecretString` provides.

The gateway crate (`mika-gateway`) already used `SecretString` for all its secrets, providing the target pattern.

## Guidance

All secret fields in `Settings` must use `Option<SecretString>` from the `secrecy` crate (with the `serde` feature enabled for config-rs deserialization). Secrets are exposed at the `Settings` accessor boundary — downstream types (`ActiveLlmConfig`, `ModelSpec`, `ClaudeClient`, `EmbeddingClient`, `ToolContext`, `InvestigateConfig`) continue using plain `String`/`&str`.

**Field declaration:**
```rust
// In Settings struct
pub anthropic_api_key: Option<SecretString>,
```

**Accessor methods (the boundary):**
```rust
// provider_fields() — expose at boundary, return &str
self.anthropic_api_key.as_ref().map(|s| s.expose_secret())

// make_embedding_client() — expose, filter, then convert to String
self.openai_api_key
    .as_ref()
    .map(|s| s.expose_secret())
    .filter(|k| !k.trim().is_empty())
    .and_then(|key| EmbeddingClient::new(key.to_string(), ...))
```

**`get_effective_value()` — never expose raw values:**
```rust
"anthropic_api_key" => settings.anthropic_api_key.as_ref().map(|_| "[SET]".to_string()),
```

**Downstream consumers — expose at the extraction point:**
```rust
// When passing to a struct that takes Option<String>
brave_api_key: settings.brave_api_key.as_ref().map(|s| s.expose_secret().to_string()),

// When passing to a struct that takes Option<&str>
brave_api_key: settings.brave_api_key.as_ref().map(|s| s.expose_secret()),
```

**Import requirement:** Add `use secrecy::ExposeSecret;` in any file that calls `.expose_secret()`.

## Why This Matters

- **Compile-time safety:** `SecretString` does not implement `Deref<Target=str>` — accessing the secret value requires an explicit `.expose_secret()` call. Accidental `.clone()` propagation into logs is prevented at compile time.
- **Zeroize-on-drop:** When a `SecretString` is dropped, its memory is overwritten with zeros, reducing the window for memory-scraping attacks.
- **Consistent redaction:** The manual `Debug` impl uses `.as_ref().map(|_| "[REDACTED]")` uniformly for all secret fields. `get_effective_value()` returns `"[SET]"` for all secret-flagged fields.
- **Compiler-driven migration:** Changing a field from `Option<String>` to `Option<SecretString>` causes compile errors at every access site, ensuring no site is missed.

## When to Apply

- When adding any new API key, token, or credential field to `Settings`
- When any `Option<String>` field in `Settings` holds secret material
- The expose-at-boundary strategy means downstream types do NOT need to change — only the `Settings` accessors and the call sites that extract from `Settings`

## Examples

**Adding a new provider API key:**

1. Add field: `pub new_provider_api_key: Option<SecretString>`
2. Add to `provider_fields()` match arm: `.as_ref().map(|s| s.expose_secret())`
3. Add to `get_effective_value()`: `.as_ref().map(|_| "[SET]".to_string())`
4. Add to manual `Debug` impl: `.field("new_provider_api_key", &self.new_provider_api_key.as_ref().map(|_| "[REDACTED]"))`
5. Add to `ConfigKeyInfo` registry with `secret: true`
6. Follow compiler errors for any downstream access sites

**9-layer config checklist** (from `docs/solutions/architecture-patterns/config-key-rename-across-layers.md`):
Settings struct, ConfigKeyInfo registry, get_effective_value(), manual Debug impl, direct `std::env::var()` calls, test fixtures, handler script unset lists, `.env.example`/docs, CI workflows.

## Related

- `docs/solutions/security-issues/debug-log-secret-leakage-and-file-permissions.md` — logging as an output boundary
- `docs/solutions/security-issues/setup-wizard-secret-handling.md` — atomic writes, 0600 permissions
- `docs/solutions/architecture-patterns/config-key-rename-across-layers.md` — 9-layer checklist
- `docs/solutions/architecture-patterns/per-agent-dotenv-config-injection.md` — per-agent secrets without process env mutation
- `crates/mika-gateway/src/settings.rs` — gateway uses SecretString for all secrets (reference pattern)
- Issue #97 — Design: Secret Management & Shell Sandboxing
