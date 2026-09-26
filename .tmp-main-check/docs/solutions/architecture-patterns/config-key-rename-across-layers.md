---
problem_type: configuration-change
title: "Config Key Rename Across All Layers (MIKA_GITHUB_TOKEN → MIKA_INVESTIGATE_GITHUB_TOKEN)"
date: 2026-03-12
components:
  - crates/mika-common (config registry, Settings struct, Debug impl)
  - crates/mika-cli (setup wizard, doctor checks, config command)
  - crates/mika-agent (investigation panel, handler script env scrubbing)
  - docs (configuration.md, CLAUDE.md, .env.example)
symptoms:
  - Generic env var name MIKA_GITHUB_TOKEN suggests it powers the agent GitHub skill
  - Users may confuse dashboard investigation token with gh CLI auth
root_cause: >
  Env var introduced for a narrow purpose (investigation panel issue creation)
  but given a generic name that implied it was the primary GitHub credential.
keywords:
  - env var rename
  - config key rename
  - MIKA_GITHUB_TOKEN
  - MIKA_INVESTIGATE_GITHUB_TOKEN
  - breaking change
  - config registry
  - env scrubbing
---

# Config Key Rename Across All Layers

## Problem

The environment variable `MIKA_GITHUB_TOKEN` was introduced for the dashboard
investigation panel's GitHub issue creation feature. Despite the generic name,
it was never used by the agent's GitHub skill (which uses `gh` CLI's own auth
via `GH_TOKEN` / `gh auth login`). The generic name was misleading — users
might think it powers the agent's GitHub capabilities.

## Root Cause

Naming scope mismatch: a narrow-purpose credential was given a broad name. The
token's sole consumer is `CreateGithubIssueTool` inside the investigation panel.

## Solution

Renamed to `MIKA_INVESTIGATE_GITHUB_TOKEN` / `investigate_github_token` across
all layers. No backwards-compatible fallback — clean break (feature was recently
added, small blast radius).

### Layer-by-layer changes

**1. Config registry (`crates/mika-common/src/config.rs`):**

- `ConfigKeyInfo` entry: key, env_var, description
- `Settings` struct field name
- `get_effective_value()` match arm
- Manual `Debug` impl redaction field

**2. Server (`crates/mika-agent/src/server/investigate.rs`):**

- `InvestigationToolsConfig` struct field
- `build_investigation_tools()` destructure
- `handle_investigate()` — `has_github` check + OnceCell lazy init
- Code comments referencing the env var

**3. CLI (`crates/mika-cli/src/commands/`):**

- `setup.rs`: `secret_is_set()` check, prompt string, compose `.env` output
- `doctor.rs`: `check_optional_key()` call
- `config.rs`: refactored Env backend to branch on `info.secret` (needed
  because `github_repo` is non-secret but was funneled through Password prompt)

**4. Handler scripts (`crates/mika-agent/templates/skills/*/handlers/run.sh`):**

- Added `MIKA_INVESTIGATE_GITHUB_TOKEN` to `unset` lists (defense-in-depth)

**5. Test fixtures (`test_utils.rs`, `server/mod.rs`):**

- `Settings` struct literal field names behind `#[cfg(test)]`

**6. Documentation (`.env.example`, `CLAUDE.md`, `docs/configuration.md`):**

- All references updated

### Key decisions

- **No backwards-compat fallback:** Clean break. Feature was recently added.
- **`MIKA_GITHUB_REPO` NOT renamed:** Not ambiguous — only investigation uses it.
- **`CreateGithubIssueTool` internal field `github_token` NOT renamed:** Private
  implementation detail, not part of any config or public API surface.
- **Historical plan doc NOT updated:** Preserves historical accuracy.

## Verification

```bash
cargo build          # Struct field renames cause compile errors
cargo test           # Catches #[cfg(test)] fixture literals
cargo clippy         # No warnings
grep -r 'MIKA_GITHUB_TOKEN' --include='*.rs' --include='*.sh' --include='*.md' --include='*.example'
# Should only match historical plan doc
```

## Gotchas

1. **Test fixtures compile under `#[cfg(test)]` only.** `cargo build` won't
   catch stale struct field names in test helpers — always run `cargo test` too.

2. **Handler scripts are templates.** They live in `templates/skills/*/handlers/`
   and are copied to user directories at startup. Updating the template doesn't
   update already-deployed copies. Users must run `mika setup` or restart.

3. **config-rs mapping:** `MIKA_INVESTIGATE_GITHUB_TOKEN` maps to field
   `investigate_github_token` via the `MIKA_` prefix strip. Single underscores
   are literal; double underscores (`__`) denote nesting. No `#[serde(rename)]`
   needed.

4. **OnceCell lazy init:** The investigation tool registry is initialized once
   per process. Env var changes require a server restart.

## Prevention: Checklist for Future Env Var Renames

### Preparation

- [ ] Grep the entire repo with no file type filter: `rg 'OLD_VAR_NAME'`
- [ ] Categorize every hit: Rust source, test fixture, shell script, docs

### Rust changes

- [ ] `Settings` struct field (compiler enforces all usage sites)
- [ ] `ConfigKeyInfo` registry entry (key, env_var, description)
- [ ] `get_effective_value()` match arm
- [ ] Manual `Debug` impl redaction
- [ ] Any `std::env::var("OLD_NAME")` calls (bypass Settings struct)
- [ ] Test fixtures constructing `Settings` literals

### Shell scripts and docs

- [ ] Handler script `unset` lists
- [ ] `.env.example`
- [ ] `CLAUDE.md` Environment Variables section
- [ ] `docs/configuration.md` and living reference docs
- [ ] `docs/solutions/*.md` if they reference the env var as current guidance
- [ ] Docker/CI files if they set the var

### Validation

- [ ] `cargo build` + `cargo test` + `cargo clippy`
- [ ] `rg 'OLD_VAR_NAME'` returns zero results (excluding historical docs)
- [ ] End-to-end test with new env var set
- [ ] Graceful degradation test with new env var unset

## Related Documentation

- [Config 4-source model](../architecture-patterns/simplified-config-4-source-model.md)
- [Config key registry CLI management](../architecture-patterns/config-key-registry-cli-management.md)
- [Env var leakage in exec handlers](../security-issues/env-var-leakage-exec-handler-child-processes.md)
- [Conditional investigation tool registration](../architecture-patterns/conditional-investigation-tool-registration.md)
- [Investigation panel SSE agent loop](../architecture/investigation-panel-sse-agent-loop.md)
