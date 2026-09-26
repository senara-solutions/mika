---
title: "GH_TOKEN identity collision from .env leak"
category: security-issues
date: 2026-04-02
severity: high
tags: [env-vars, identity-separation, GH_TOKEN, dotenvy, scrub, defense-in-depth]
issue: "#380"
modules: [dotenv, executor, builtin_handlers, shell-exec]
---

# GH_TOKEN identity collision from .env leak

## Problem

`GH_TOKEN` set in `~/.mika/.env` caused `dotenvy` to inject it into the process environment at startup. Since `scrub_mika_env_vars()` only removes `MIKA_*`-prefixed vars, `GH_TOKEN` survived scrubbing and leaked into ALL child processes — exec handlers, long-running skills (claude-pilot via self-dev), and git operations.

This collapsed the developer/platform identity separation: claude-pilot (which should use the host's `gh auth` identity) ended up using `MIKA_GITHUB_TOKEN`'s value instead, causing GitHub to block self-approval on PRs (same identity creating and reviewing).

**Symptom:** Self-approval block in the autonomous dev loop — PR author and reviewer resolve to the same GitHub identity.

## Root Cause

Three compounding issues:

1. **dotenvy loads ALL vars** — `load_dotenv()` injects every key from `~/.mika/.env` into the process env, not just `MIKA_*` vars. A user-placed `GH_TOKEN` entry becomes process-wide.
2. **`scrub_mika_env_vars()` missed non-MIKA vars** — Only `MIKA_*` prefix was removed from child processes. `GH_TOKEN` passed through untouched.
3. **`run_gh` ordering fragility** — `GH_TOKEN` was set BEFORE `scrub_mika_env_vars()`. While this worked when only `MIKA_*` was scrubbed, it would break if the scrub was broadened.

## Solution

Three-layer defense-in-depth fix:

### Layer 1: Startup detection and active removal (`dotenv.rs`)

```rust
pub fn check_env_warnings(home_dir: &Path) {
    // Parse .env file as text (not process env) to avoid false positives
    // when GH_TOKEN is legitimately set in the host shell
    if env_file_contains_key(home_dir, "GH_TOKEN") {
        warn!("GH_TOKEN found in {}/.env — removed from process environment...");
        unsafe { std::env::remove_var("GH_TOKEN") };
    }
}
```

Key design decisions:
- **File-based detection** (not `std::env::var()`) — avoids false positives when `GH_TOKEN` is legitimately in the host shell
- **Active removal, not just a warning** — in autonomous mode (claude-pilot), nobody reads startup logs
- **`unsafe` is justified** — called at startup before tokio spawns threads; single-threaded context

### Layer 2: Exec handler scrub (`executor.rs`)

```rust
const EXTRA_SCRUB_VARS: &[&str] = &["GH_TOKEN"];

pub(crate) fn scrub_mika_env_vars(cmd: &mut tokio::process::Command) {
    for (key, _) in std::env::vars() {
        if key.starts_with("MIKA_") { cmd.env_remove(&key); }
    }
    for key in EXTRA_SCRUB_VARS { cmd.env_remove(key); }
}
```

Defense-in-depth: even if Layer 1 fails or is bypassed, child processes won't inherit `GH_TOKEN`.

### Layer 3: run_gh reordering (`builtin_handlers.rs`)

```rust
// BEFORE: GH_TOKEN set, then scrub (scrub didn't touch it)
// AFTER: scrub first (removes GH_TOKEN), then re-add correct platform token
super::executor::scrub_mika_env_vars(&mut cmd);
if let Some(token) = ctx.github_token {
    cmd.env("GH_TOKEN", token);  // correct platform token survives
}
```

### Layer 3b: Shell handler parity (`shell-exec/handlers/run.sh`)

```sh
# Existing wildcard scrub
for _mika_var in $(env | grep '^MIKA_' | cut -d= -f1); do unset "$_mika_var"; done
# NEW: explicit GH_TOKEN scrub (#380)
unset GH_TOKEN 2>/dev/null
```

## Prevention

1. **Never add non-`MIKA_*` secrets to `~/.mika/.env`** — dotenvy loads ALL vars, not just `MIKA_*`. The `MIKA_` prefix convention exists for a reason.
2. **`env_file_contains_key()` handles `export` prefix** — dotenvy supports `export GH_TOKEN=...` syntax, so the detector strips it.
3. **Follow the env scrubbing tiers** when adding new child process spawn paths:
   - MCP: `env_clear()` + allowlist (strictest)
   - Exec handlers: `scrub_mika_env_vars()` removes `MIKA_*` + `EXTRA_SCRUB_VARS`
   - Shell handlers: wildcard `unset` + explicit `unset GH_TOKEN`
4. **Never silently fall back between token scopes** — lesson from #359 (dedicated-github-token-agent-operations.md).

## Related

- [run_gh missing MIKA_GITHUB_TOKEN injection](../integration-issues/run-gh-github-token-injection.md) — predecessor fix (#346)
- [Dedicated GitHub token for agent operations](../architecture-patterns/dedicated-github-token-agent-operations.md) — two-token architecture (#289, #359)
- [Env var leakage through exec handler child processes](env-var-leakage-exec-handler-child-processes.md) — three-tier scrubbing model
- [MIKA_LLM_API_KEY deprecation](../integration-issues/mika-llm-api-key-deprecation-env-var-mismatch.md) — shell handler wildcard scrub pattern
