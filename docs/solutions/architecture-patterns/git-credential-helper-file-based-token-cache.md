---
title: "Git credential helper with file-based token cache for short-lived CLI processes"
category: architecture-patterns
date: 2026-04-02
tags: [github-app, credential-helper, git, token-cache, cli, security]
module: mika-common, mika-cli
issue: "#383"
---

# Git Credential Helper with File-Based Token Cache

## Problem

The `GitHubApp` module uses an in-memory `tokio::sync::RwLock` cache for installation tokens. This works for long-running processes (mika-spirit, TUI) but is useless for short-lived CLI processes like `mika credential-helper`, which git spawns on every HTTPS push. Each invocation would make a full JWT + HTTP round-trip (~1-2 seconds) without cross-process caching.

Additionally, the credential helper runs as a child of git, which itself may be spawned by `run_git` with `scrub_mika_env_vars()` — meaning `MIKA_*` env vars are unavailable. The helper must read config from `~/.mika/.env` directly via `load_dotenv()`.

## Root Cause

In-memory caches don't persist across process boundaries. `GitHubApp::installation_token()` generates a JWT and exchanges it for an installation token on every call when the in-memory cache is cold (i.e., every new process invocation).

## Solution

### 1. File-based token cache (`installation_token_with_file_cache`)

Added to `github_app.rs` alongside the existing in-memory cache:

```rust
pub async fn installation_token_with_file_cache(&self, cache_path: &Path) -> Result<String> {
    // 1. Read file cache → if valid, return immediately
    if let Some(token) = Self::read_file_cache(cache_path) {
        return Ok(token);
    }
    // 2. Fall through to in-memory cache / JWT exchange
    let token = self.installation_token().await?;
    // 3. Read actual expiry from in-memory cache, write to file
    let expires_at = self.cache.read().await.as_ref().map(|c| c.expires_at);
    if let Some(expires_at) = expires_at {
        Self::write_file_cache(cache_path, &token, expires_at);
    }
    Ok(token)
}
```

Key design decisions:
- **Actual expiry propagated**: Reads `expires_at` from the in-memory `CachedToken` after `installation_token()` populates it — no fabricated estimates.
- **Same 5-minute `EXPIRY_BUFFER`** as in-memory cache for consistency.
- **Atomic write with restrictive permissions**: Uses `OpenOptionsExt::mode(0o600)` at creation time (no TOCTOU race between write and chmod), writes to `.tmp` then renames.
- **Non-fatal**: File cache read/write failures silently fall through to the JWT exchange path.
- **CLI-only**: Agent runtime continues using the in-memory cache via `installation_token()`.

### 2. Early-exit CLI pattern

Both `mika token github` and `mika credential-helper` bypass the full CLI initialization (agent resolution, tracing, telemetry, CLI reference generation) for fast startup:

```rust
// main.rs — early exit before agent resolution
match &cli.command {
    Some(Commands::Token(args)) => {
        let home_dir = home::resolve_home_dir()?;
        mika_common::dotenv::load_dotenv(&home_dir);
        return commands::token::run(&args.command, &home_dir).await;
    }
    Some(Commands::CredentialHelper(args)) => { /* same pattern */ }
    _ => {}
}
```

### 3. Credential helper security

- **Host filter**: Only responds for `protocol=https` AND `host=github.com` — rejects all other hosts (prevents token leakage).
- **Silent failure**: Returns `Ok(())` (exit 0) with no output on any error, letting git fall through to other credential sources.
- **Accepts `impl BufRead`**: `parse_credential_request()` is injectable for testing.

## Prevention / Best Practices

1. **When adding file-based caches for short-lived tokens**: Use `OpenOptionsExt::mode()` at file creation time, not a separate `set_permissions()` call. The latter creates a TOCTOU window.
2. **Thread actual expiry**: Don't fabricate expiry estimates when the real value is available in the in-memory cache. Read it after the cache is populated.
3. **Early-exit for lightweight CLI commands**: Commands that don't need the agent, DB, or logging should exit before the full initialization block in `main.rs`. Follow the pattern established by the team-mode early branch.
4. **Credential helpers must be silent on failure**: Git aborts on non-zero exit from credential helpers. Always return exit 0 with empty output on failure to let git try other sources.

## Files

- `crates/mika-common/src/github_app.rs` — `installation_token_with_file_cache()`, `read_file_cache()`, `write_file_cache()`, `FileCachedToken`
- `crates/mika-cli/src/commands/token.rs` — `mika token github` subcommand
- `crates/mika-cli/src/commands/credential_helper.rs` — `mika credential-helper` subcommand
- `crates/mika-cli/src/main.rs` — Early-exit dispatch for lightweight commands

## Related

- `docs/solutions/architecture-patterns/github-app-jwt-authentication-module.md` — Existing in-memory cache design (this feature extends it with a file layer)
- `docs/solutions/security-issues/gh-token-identity-collision-dotenv-leak.md` — Three-identity model (host gh auth, MIKA_GITHUB_TOKEN PAT, GitHub App installation token)
- `docs/solutions/integration-issues/run-gh-github-token-injection.md` — Scrub-then-inject pattern for subprocess tokens
- Issue #383, parent senara-solutions/mika-platform#3 (Phase 3)
