---
title: "sqlx migration mismatch: missing embedded migration due to Cargo incremental compilation"
category: build-errors
date: 2026-03-15
tags: [sqlx, migrations, cargo, incremental-compilation, build-rs, postgres, axum]
component: mika-gateway
severity: high
rust_crates: [sqlx, axum, cargo]
---

## Problem

The mika-gateway binary failed to start with the error:

```
migration 2 was previously applied but is missing in the resolved migrations
```

This occurred after migration 002 (creating the `outbound_messages` table) was added to `crates/mika-gateway/migrations/`.

## Root Cause

`sqlx::migrate!("./migrations")` is a compile-time proc macro that embeds migration SQL directly into the binary at build time. Cargo's incremental compilation tracks changes to existing files, but does NOT track new files added to a directory. When `migrations/002_outbound_messages.sql` was added, Cargo saw no modifications to tracked inputs and reused the cached binary — which only had migration 001 embedded.

At runtime, SQLx found a record for migration version 2 in the `_sqlx_migrations` table but could not match it to any migration embedded in the current binary. This mismatch is treated as a fatal error because SQLx cannot verify the integrity of the applied migration.

The failure mode is silent at build time: the binary compiles cleanly, passes all type checks, and links without error. The missing migration is simply absent from the embedded set. The failure only surfaces at runtime.

## Solution

Add a `build.rs` file to `crates/mika-gateway/` that explicitly declares the `migrations/` directory as a build input:

**`crates/mika-gateway/build.rs`:**

```rust
fn main() {
    println!("cargo::rerun-if-changed=migrations");
}
```

This registers the `migrations/` directory with Cargo's dependency tracking. Any future addition, removal, or modification of files under `migrations/` will invalidate the build cache and force recompilation, ensuring the embedded migration set always matches the files on disk.

## Recovery Steps

If the database is already in a corrupted state (migration 2 recorded in `_sqlx_migrations` but not present in the binary):

```bash
# Step 1: Force a full rebuild of mika-gateway with the new build.rs in place
cargo clean -p mika-gateway && cargo build --release --bin mika-gateway

# Step 2: Clean up the inconsistent DB state
psql -d mika_gateway -c "DROP TABLE IF EXISTS outbound_messages; DELETE FROM _sqlx_migrations WHERE version = 2;"

# Step 3: Restart mika-gateway — SQLx will re-apply migration 002 cleanly
```

**Note:** If the `_sqlx_migrations` row is present and `success = true`, SQLx will not re-apply the migration unless the row is removed. Removing the row and dropping the table together ensures a clean slate for re-application.

## Verification

Confirm the migration SQL is embedded in the binary:

```bash
strings target/release/mika-gateway | grep -i outbound_messages
```

Confirm the database reflects all migrations applied successfully:

```bash
psql -d mika_gateway -c "SELECT version, description, success FROM _sqlx_migrations ORDER BY version;"
```

Expected output should show both version 1 and version 2 with `success = true`.

## Prevention

### Every crate using `sqlx::migrate!()` must have a `build.rs`

This is not optional hygiene — it is a correctness requirement. Without it, adding a migration file will not trigger recompilation, and the binary silently ships without the new migration.

Emit the directive at the directory level, not the file level. `rerun-if-changed` on a directory watches for additions, deletions, and modifications within it.

### Checklist: Adding a New SQLx Migration

```
BEFORE writing the migration file:
  [ ] Confirm the crate has build.rs with cargo::rerun-if-changed=migrations
  [ ] If build.rs is absent, add it before proceeding

AFTER writing the migration file:
  [ ] Run cargo build and confirm the affected crate recompiles (check build output)
  [ ] Run the binary against a fresh database and confirm migration applies cleanly
  [ ] Run the binary against a database at the previous schema version

FOR mika-gateway specifically:
  [ ] Start the db compose profile (docker compose --profile db up -d)
  [ ] Run the gateway binary against the live Postgres instance
  [ ] Confirm the new migration appears in _sqlx_migrations
```

### CI/CD Considerations

- Docker image builds are inherently clean (fresh layer per build), so production images always embed all migrations correctly — even if a developer's local incremental build was stale.
- CI pipelines that cache `target/` between runs are subject to the same blind spot. Consider a separate CI job with a clean build for migration-bearing crates.
- Do not rely solely on `cargo test` with a cached target directory to validate migration correctness.

## Cross-References

- **`crates/mika-agent/build.rs`** — Sibling pattern using `cargo:rerun-if-changed` (single-colon, older syntax) for doc sync. Same mechanism, different use case.
- **[gateway-monorepo-migration.md](../integration-issues/gateway-monorepo-migration.md)** — Gateway monorepo migration; section 2 covers `include_str!` compile-time issues (same class of problem).
- **[docker-buildkit-cache-mounts-compose.md](../architecture-patterns/docker-buildkit-cache-mounts-compose.md)** — Cargo incremental compilation and cache semantics in Docker builds.
- **[rust-workspace-release-plz-github-actions.md](../ci-cd/rust-workspace-release-plz-github-actions.md)** — References `CARGO_INCREMENTAL: 0` in CI context.
- **[docs/runtime-structure.md](../../runtime-structure.md)** — Schema version history and migration file listing.
