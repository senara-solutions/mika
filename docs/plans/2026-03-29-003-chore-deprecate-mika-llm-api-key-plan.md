---
title: "chore: deprecate MIKA_LLM_API_KEY and fix OAuth setup env var mismatch"
type: fix
status: completed
date: 2026-03-29
issue: 317
---

# Deprecate MIKA_LLM_API_KEY and Fix OAuth Setup Env Var Mismatch

## Overview

`MIKA_LLM_API_KEY` is a dead env var — it has no corresponding field in the `Settings` struct, so config-rs never reads it. Yet `setup.rs`, `doctor.rs`, `claude.rs`, and `mika-server.rs` all reference it in user-facing code. This creates a split-brain where:

- **Setup** writes `MIKA_LLM_API_KEY` to `~/.mika/.env`
- **Config system** reads `MIKA_ANTHROPIC_API_KEY` from env
- **Result:** Every user who ran `mika setup` has a broken `.env` file

The OAuth flow is the most painful case: `setup --mode oauth` persists the subscription token hash against `MIKA_LLM_API_KEY`, but `ClaudeClient` receives the key from `MIKA_ANTHROPIC_API_KEY` via `Settings.anthropic_api_key` — hash mismatch, auth failure.

## Proposed Solution

Replace all references to `MIKA_LLM_API_KEY` with `MIKA_ANTHROPIC_API_KEY` across user-facing code, error messages, setup wizards, health checks, tests, docs, and shell handlers. Add a startup deprecation warning when the old var is detected. No silent fallback — warn and tell the user exactly what to rename.

## Changes

### 1. `crates/mika-cli/src/commands/setup.rs`

| Line(s) | Current | Change |
|---------|---------|--------|
| 45 | `write_dotenv_key(home_dir, "MIKA_LLM_API_KEY", &api_key)` | `MIKA_ANTHROPIC_API_KEY` |
| 66 | `secret_is_set("MIKA_LLM_API_KEY")` | `MIKA_ANTHROPIC_API_KEY` |
| 126-127 | `secret_is_set("MIKA_LLM_API_KEY")` + prompt label "LLM API key" | `MIKA_ANTHROPIC_API_KEY` + label "Anthropic API key (skip if using another provider)" |
| 271 | `std::env::var("MIKA_LLM_API_KEY")` (OAuth read) | `MIKA_ANTHROPIC_API_KEY` |
| 276, 279-280 | References in OAuth flow | `MIKA_ANTHROPIC_API_KEY` |
| 293 | `write_dotenv_key(home_dir, "MIKA_LLM_API_KEY", ...)` (OAuth write) | `MIKA_ANTHROPIC_API_KEY` |
| 450 | `MIKA_LLM_API_KEY={api_key}` in compose env | `MIKA_ANTHROPIC_API_KEY` + update prompt label |

### 2. `crates/mika-cli/src/commands/doctor.rs`

| Line(s) | Current | Change |
|---------|---------|--------|
| 243 | `std::env::var("MIKA_LLM_API_KEY")` | `MIKA_ANTHROPIC_API_KEY` |
| 255 | `.env` scan for `MIKA_LLM_API_KEY` | `MIKA_ANTHROPIC_API_KEY` |

Doctor should check the active provider's key. For now, checking `MIKA_ANTHROPIC_API_KEY` (the default provider) is sufficient and matches the config system.

### 3. `crates/mika-common/src/claude.rs`

| Line(s) | Current | Change |
|---------|---------|--------|
| 344 | `"MIKA_LLM_API_KEY is required but not set..."` | `"MIKA_ANTHROPIC_API_KEY is required but not set. Set it to your Anthropic API key (sk-ant-api03-...) or OAuth token (sk-ant-oat01-...)."` |
| 504 | `"Check that MIKA_LLM_API_KEY is set to a valid Anthropic API key."` | `"Check that MIKA_ANTHROPIC_API_KEY is set to a valid Anthropic API key."` |

### 4. `crates/mika-agent/src/bin/mika-server.rs`

| Line | Current | Change |
|------|---------|--------|
| 13 | `"Set MIKA_LLM_API_KEY (API key or OAuth token) and MIKA_INTERNAL_TOKEN."` | `"Set MIKA_ANTHROPIC_API_KEY (or your provider's key) and MIKA_INTERNAL_TOKEN."` |

### 5. Comments — `executor.rs`, `mcp/mod.rs`

| File | Line | Change |
|------|------|--------|
| `crates/mika-agent/src/skills/executor.rs` | 27 | Update comment: `MIKA_LLM_API_KEY` → `MIKA_ANTHROPIC_API_KEY` (as example) |
| `crates/mika-agent/src/mcp/mod.rs` | 231 | Update comment: `MIKA_LLM_API_KEY` → `MIKA_ANTHROPIC_API_KEY` |

### 6. Shell handler — `shell-exec/handlers/run.sh`

Line 13 currently unsets:
```bash
unset MIKA_LLM_API_KEY MIKA_INTERNAL_TOKEN MIKA_OPENAI_API_KEY MIKA_BRAVE_API_KEY MIKA_GITHUB_TOKEN MIKA_INVESTIGATE_GITHUB_TOKEN
```

Replace `MIKA_LLM_API_KEY` with `MIKA_ANTHROPIC_API_KEY` and add all missing per-provider keys:
```bash
unset MIKA_ANTHROPIC_API_KEY MIKA_OPENAI_API_KEY MIKA_OPENROUTER_API_KEY MIKA_GROQ_API_KEY MIKA_OLLAMA_API_KEY MIKA_MISTRAL_API_KEY MIKA_GOOGLE_API_KEY MIKA_DEEPSEEK_API_KEY MIKA_INTERNAL_TOKEN MIKA_BRAVE_API_KEY MIKA_GITHUB_TOKEN MIKA_INVESTIGATE_GITHUB_TOKEN
```

This closes a pre-existing security gap where per-provider keys leaked to shell subprocesses.

### 7. Test — `smoke.rs`

| File | Line | Change |
|------|------|--------|
| `crates/mika-agent/tests/smoke.rs` | 19 | `.env("MIKA_LLM_API_KEY", ...)` → `.env("MIKA_ANTHROPIC_API_KEY", ...)` |

### 8. Backward compatibility — startup deprecation warning

Add a check after `load_dotenv()` in the CLI entry point and server startup:

```rust
// After dotenv is loaded, check for legacy env var
if std::env::var("MIKA_LLM_API_KEY").is_ok() {
    eprintln!(
        "WARNING: MIKA_LLM_API_KEY is deprecated. \
         Rename it to MIKA_ANTHROPIC_API_KEY in your ~/.mika/.env file. \
         MIKA_LLM_API_KEY is ignored by the config system."
    );
}
```

**No silent fallback.** The warning is explicit and tells the user exactly what to do. The config system does not read `MIKA_LLM_API_KEY` and we do not add a fallback — that would create two paths to maintain.

**Where:** The deprecation check should live in `mika-common` (e.g., a `check_deprecated_env_vars()` function called after `load_dotenv()`), so both CLI and server binaries get it.

### 9. Documentation updates

| File | What to change |
|------|---------------|
| `CLAUDE.md` | Remove any `MIKA_LLM_API_KEY` references in env var section |
| `docs/configuration.md` | Update migration table and all references |
| `docs/getting-started.md` | Replace `MIKA_LLM_API_KEY` in setup instructions |
| `docs/deployment.md` | Update env var references |
| `docs/architecture.md` | Update env var references |
| `README.md` | Update `export MIKA_LLM_API_KEY=...` example |
| `CONTRIBUTING.md` | Update env var reference |
| `.env.example` | Already correct (uses `MIKA_ANTHROPIC_API_KEY`) — verify no regression |
| `docs/solutions/architecture-patterns/unified-llm-api-key-consolidation.md` | Add supersession note — per-provider keys are now canonical |

After updating `docs/`, run `scripts/sync-agent-docs.sh` to sync `crates/mika-agent/docs/` copies.

## Acceptance Criteria

- [x] `mika setup` writes `MIKA_ANTHROPIC_API_KEY` to `.env` (not `MIKA_LLM_API_KEY`)
- [x] `mika setup --mode oauth` reads/writes `MIKA_ANTHROPIC_API_KEY` — OAuth token hash matches runtime
- [x] `mika setup --mode compose` generates `MIKA_ANTHROPIC_API_KEY` in `.env`
- [x] `mika doctor` checks `MIKA_ANTHROPIC_API_KEY` (not `MIKA_LLM_API_KEY`)
- [x] Error messages in `claude.rs` reference `MIKA_ANTHROPIC_API_KEY`
- [x] `mika-server` startup error references `MIKA_ANTHROPIC_API_KEY`
- [x] Smoke test uses `MIKA_ANTHROPIC_API_KEY`
- [x] Shell-exec handler unsets all per-provider API keys (security fix)
- [x] Startup deprecation warning when `MIKA_LLM_API_KEY` is detected in env
- [x] No code path reads `MIKA_LLM_API_KEY` — `grep -r "MIKA_LLM_API_KEY" crates/` returns only the deprecation check
- [x] All docs updated, `sync-agent-docs.sh` run
- [x] `cargo test` passes
- [x] `cargo clippy` clean

## Out of Scope

- Auto-migration of existing `.env` files (warn only — users rename manually)
- Provider-aware setup wizard (asking which provider, then prompting for that key)
- Adding missing provider keys to compose mode prompts
- `MIKA_LLM_API_KEY` does not exist in `config.rs` ConfigKeyInfo registry — no config-rs changes needed

## Sources

- Issue: #317
- Config system: `crates/mika-common/src/config.rs:58` — `MIKA_ANTHROPIC_API_KEY` registered as `anthropic_api_key`
- Provider fields: `crates/mika-common/src/config.rs:706` — `provider_fields(Anthropic)` returns `self.anthropic_api_key`
- `.env.example` already uses `MIKA_ANTHROPIC_API_KEY` (correct)
- CI already sets `MIKA_ANTHROPIC_API_KEY` (correct)
