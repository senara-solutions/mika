---
title: "Migrate mika-gateway from private repo to public monorepo"
date: "2026-03-01"
category: "integration-issues"
tags:
  - workspace-integration
  - dependency-management
  - open-core-model
  - dockerfile
  - rust-edition-2024
modules:
  - mika-gateway
  - Cargo.toml
  - Dockerfile.gateway
severity: medium
---

# Migrate mika-gateway from Private Repo to Public Monorepo

## Problem

The mika-gateway Rust crate lived in a separate private repository (mika-cloud)
with a git SSH dependency on mika-common. For the open-core model, it needed to
move into the public mika monorepo alongside mika-agent and mika-cli.

## Challenges

1. **Dependency alignment** — sqlx (Postgres) wasn't in the mika workspace deps
2. **include_str! at compile time** — OpenAPI test referenced a file that didn't exist
3. **Clippy warnings** — 5 collapsible `if let` warnings (edition 2024 let-chains)
4. **Dockerfile adaptation** — needed a new Dockerfile.gateway for monorepo paths
5. **Dev-dependency misclassification** — rand/hex were production deps but test-only
6. **Stale documentation** — 3+ docs still referenced gateway in mika-cloud

## Solution

### 1. Copy and workspace auto-discovery

```bash
cp -r mika-cloud/mika-gateway/ mika/crates/mika-gateway/
```

The root `Cargo.toml` uses `members = ["crates/*"]` — no manual member addition
needed. The gateway's `Cargo.toml` already used `workspace = true` for all deps.

### 2. Add missing workspace dependency

```toml
# Root Cargo.toml
[workspace.dependencies]
sqlx = { version = "0.8", features = ["runtime-tokio", "postgres", "uuid", "chrono"] }
```

### 3. Generate OpenAPI spec for include_str!

The test `test_committed_spec_is_current` uses `include_str!()` which resolves
at compile time. The file must exist before compilation:

```bash
mkdir -p docs/openapi
touch docs/openapi/gateway.yaml  # placeholder
cargo test -p mika-gateway openapi::tests::write_gateway_openapi_yaml -- --ignored
# Now the real spec exists and subsequent builds work
```

### 4. Fix edition 2024 clippy (collapsible if → let-chains)

```rust
// Before (nested if let)
if let Some(photos) = &message.photo {
    if let Some(largest) = photos.last() {
        return ParsedMessage::Photo { /* ... */ };
    }
}

// After (let-chain)
if let Some(photos) = &message.photo
    && let Some(largest) = photos.last()
{
    return ParsedMessage::Photo { /* ... */ };
}
```

### 5. Dockerfile.gateway for monorepo

Key differences from Dockerfile.agent:
- No gcc/libc-dev (no bundled SQLite)
- Must copy mika-common manifest for workspace resolution
- Leaner runtime (ca-certificates + wget only)
- No home directory (stateless service)

```dockerfile
COPY Cargo.toml Cargo.lock ./
COPY crates/mika-common/Cargo.toml crates/mika-common/Cargo.toml
COPY crates/mika-gateway/Cargo.toml crates/mika-gateway/Cargo.toml
```

### 6. Move rand/hex to dev-dependencies

Both were only used inside `#[cfg(test)]` blocks for `generate_pairing_token`.

## Prevention

- When adding crates to a workspace with `members = ["crates/*"]`, ensure all
  crate-specific deps are declared at the workspace level
- Run `cargo clippy` after copying code between repos — edition settings may differ
- If a test uses `include_str!()`, ensure the referenced file exists or is
  generated before the test suite runs
- Audit `[dependencies]` vs `[dev-dependencies]` — test-only crates should be dev

## Related

- [MCP HTTP TLS missing](mcp-http-tls-missing-rmcp.md) — similar workspace dep issue
- [ADR-001: Axum HTTP server architecture](../adr/001-axum-http-server-architecture.md)
- `docs/deployment.md` — Docker build patterns
- `docs/configuration.md` — gateway environment variables
