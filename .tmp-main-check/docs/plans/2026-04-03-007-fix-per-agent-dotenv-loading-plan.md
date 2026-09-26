---
title: "fix: per-agent .env loading for token and credential-helper"
type: fix
status: completed
date: 2026-04-03
---

# fix: per-agent .env loading for token and credential-helper

## Overview

`mika token github` and `mika credential-helper` only load `~/.mika/.env` (global). When `--agent` is passed, the per-agent `~/.mika/agents/<name>/.env` is never loaded into the process environment. Since PR #426 moved GitHub App credentials to per-agent `.env`, these commands fail.

## Root Cause

In `main.rs` lines 23-30 and 32-39: `load_dotenv(&global_home)` is called, but `load_dotenv(&agent_home)` is never called. The `agent_home` is correctly resolved and passed to `token.rs`/`credential_helper.rs`, but `Settings::load_for_agent()` reads from process env vars — which were never populated from the agent's `.env` file.

## Fix

Load per-agent `.env` BEFORE global `.env` when `--agent` is specified. dotenvy does not override existing vars, so the first load wins. Per-agent credentials take precedence.

### `crates/mika-cli/src/main.rs`

For both `Token` and `CredentialHelper` branches:

```rust
// Before (broken):
let global_home = home::resolve_home_dir()?;
mika_common::dotenv::load_dotenv(&global_home);
let agent_home = cli.agent.as_deref()
    .map(|name| home::resolve_agent_home(&global_home, name));

// After (fixed):
let global_home = home::resolve_home_dir()?;
let agent_home = cli.agent.as_deref()
    .map(|name| home::resolve_agent_home(&global_home, name));
// Load per-agent .env first (wins on conflict), then global as fallback
if let Some(ref ah) = agent_home {
    mika_common::dotenv::load_dotenv(ah);
}
mika_common::dotenv::load_dotenv(&global_home);
```

## Acceptance Criteria

- [ ] `mika --agent mika-dev token github` loads per-agent `.env` and prints a token
- [ ] `mika --agent mika-qa token github` loads per-agent `.env` and prints a token
- [ ] `mika token github` (no agent) still works with global `.env`
- [ ] `mika --agent mika-dev credential-helper get` uses per-agent App credentials
- [ ] Per-agent vars take precedence over global vars
- [ ] `cargo test` passes

## Sources

- Issue: #427
- Per-agent `.env` introduced in: PR #426
- dotenv module: `crates/mika-common/src/dotenv.rs`
- CLI entry point: `crates/mika-cli/src/main.rs:22-39`
