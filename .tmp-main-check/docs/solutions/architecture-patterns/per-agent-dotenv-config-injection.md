---
title: Per-Agent .env Config Injection Without Process Env Mutation
date: 2026-04-04
status: documented
category: architecture-patterns
tags: [configuration, dotenv, multi-agent, server-mode, config-rs, per-agent]
modules:
  - mika-common (dotenv.rs, config.rs)
  - mika-cli (main.rs)
  - mika-agent (server/mod.rs)
severity: high
symptoms:
  - Per-agent GitHub App credentials ignored in CLI chat/ask and server mode
  - Agent operations (PR reviews, context injection) use wrong identity
  - mika-qa actions appear as samidarko instead of mika-qa-bot[bot]
---

# Per-Agent .env Config Injection Without Process Env Mutation

## Problem

Per-agent `.env` files (`~/.mika/agents/<name>/.env`) were never loaded in CLI agent
mode or server mode. Only `token` and `credential-helper` subcommands loaded them
(fixed in PR #428). Since PR #426 moved GitHub App credentials to per-agent `.env`,
all tools using `Settings::agent_github_token()` fell back to the host's `gh auth`.

This caused mika-qa's PR review on #429 to appear as `samidarko` instead of
`mika-qa-bot[bot]`.

## Root Cause

Two startup paths skipped per-agent `.env`:

1. **CLI agent mode** (`main.rs`): Loaded global `.env` at line 152, then resolved
   `agent_home` later at line 158. Per-agent `.env` was never loaded.
2. **Server mode** (`mika-spirit.rs`): Loaded global `.env` once at process start.
   `init_agent()` called `Settings::load_for_agent()` which reads process env vars,
   but per-agent `.env` was never in the process environment.

The fundamental challenge: process env vars are global. In server mode with multiple
agents, `dotenvy::from_path()` (which sets process env) can't work — the first agent's
values would win and all subsequent agents would get the wrong credentials.

## Solution

Two distinct fixes for two execution models:

### CLI mode (single agent per process)

Move `agent_home` resolution before dotenv loading. Load per-agent `.env` first
(dotenvy won't override existing vars), then global:

```rust
let agent_home = global_home.as_ref()
    .map(|h| home::resolve_agent_home(h, &agent_name));
if let Some(ref ah) = agent_home {
    mika_common::dotenv::load_dotenv(ah);  // per-agent first
}
if let Some(ref h) = global_home {
    mika_common::dotenv::load_dotenv(h);   // global as fallback
}
```

### Server mode (multiple agents per process)

Parse per-agent `.env` without mutating process env using `dotenvy::from_path_iter()`,
convert to inline TOML, and inject as a config-rs `File` source between config files
and the `Environment` source (process env always wins):

```rust
// In Settings::load_for_agent():
if global_home != agent_home {
    let dotenv_vars = crate::dotenv::parse_dotenv(agent_home);
    if !dotenv_vars.is_empty() {
        let toml = crate::dotenv::dotenv_to_toml(&dotenv_vars);
        builder = builder.add_source(File::from_str(&toml, FileFormat::Toml));
    }
}
// Then add Environment source (highest priority)
builder = builder.add_source(Environment::with_prefix("MIKA")...);
```

Key helpers added to `dotenv.rs`:
- `parse_dotenv(home_dir)` — reads `.env` into `HashMap<String, String>` without
  setting process env vars
- `dotenv_to_toml(vars)` — strips `MIKA_` prefix, lowercases keys, escapes special
  chars (backslash, quotes, newlines, tabs) for valid TOML

Also removed `github_token` parameter from `init_agent()` — each agent now derives
its own token from `agent_settings.agent_github_token()`.

## Prevention

- When adding per-agent config, always consider both CLI (single agent) and server
  (multi-agent) execution models
- Process env vars are global — never use `load_dotenv()` per-agent in multi-agent
  server mode
- Use `dotenvy::from_path_iter()` to parse `.env` files without side effects
- Config-rs source ordering determines priority: add per-agent sources between file
  sources and `Environment` source so shell always wins

## Related

- Issue: #430
- Prior fix (token/credential-helper): PR #428, #427
- Per-agent `.env` introduced: PR #426
- Config cascade: `docs/solutions/architecture-patterns/simplified-config-4-source-model.md`
- GH_TOKEN defense: `docs/solutions/security-issues/gh-token-identity-collision-dotenv-leak.md`
- GitHub App identity: `docs/solutions/architecture/github-app-identity-and-agent-infrastructure.md`
