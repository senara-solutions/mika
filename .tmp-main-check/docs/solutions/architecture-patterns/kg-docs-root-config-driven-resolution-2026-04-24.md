---
title: "KG Docs Root — Config-Driven Path Resolution with Source Disambiguation"
date: 2026-04-24
status: documented
category: architecture-patterns
tags: [configuration, knowledge-graph, lexical-ingestion, path-resolution, config-cascade]
modules:
  - mika-common (config.rs — Settings field, CONFIG_KEYS entry, get_effective_value arm)
  - mika-agent (kg/config.rs — resolve_kg_docs_root + PathSource, server/mod.rs — empty-path guard)
problem_type: silent_failure
severity: high
symptoms:
  - Lexical ingestor logs "docs/solutions not found — skipping lexical ingestion" on every restart
  - KG lexical, subject, and resolution layers remain permanently empty
  - Only affects non-container hosts where CWD != repo root (e.g., OpenRC supervise-daemon with CWD=/)
---

# KG Docs Root — Config-Driven Path Resolution with Source Disambiguation

## Problem

`LexicalIngestor` resolved its docs root via `std::env::current_dir().join("docs/solutions")`. Inside Docker containers this works because the Dockerfile COPYs `docs/` into the workdir. On non-container hosts where an init system (OpenRC `supervise-daemon`, systemd) launches `mika-spirit` with CWD=`/`, the path resolves to `/docs/solutions` which does not exist. The ingestor silently skips, and the entire lexical/subject/resolution KG pipeline never populates.

This was a hard block for KG milestone #14 on the Gentoo OpenRC host — only the in-memory domain graph survived across restarts.

## Root Cause

Implicit CWD dependency in the path resolution. The code assumed the process working directory would always be the repo root, which is only true for container deploys and manual `cargo run` invocations.

## Solution

### Resolution chain in `kg::config::resolve_kg_docs_root`

New module `crates/mika-agent/src/kg/config.rs` with a pure resolver function:

```rust
pub fn resolve_kg_docs_root(settings: &Settings) -> (PathBuf, PathSource)
```

Resolution order (first hit wins):

1. `MIKA_KG_DOCS_ROOT` environment variable — direct `std::env::var` read.
2. `settings.kg_docs_root` field — populated by config-rs from `config.toml`.
3. `<CWD>/docs/solutions` — backward-compatible container default.

### Why re-inspect the env var

Config-rs merges `MIKA_KG_DOCS_ROOT` into `settings.kg_docs_root`, so a `Some(...)` value could originate from either the env var or the config file. The resolver checks `std::env::var("MIKA_KG_DOCS_ROOT")` directly to distinguish the two sources via `PathSource`.

### `PathSource` enum for downstream classification

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathSource {
    EnvVar,
    ConfigFile,
    CwdDefault,
}
```

Returned alongside the path so consumers can apply source-appropriate policies. The current consumer (`server/mod.rs`) uses warn-and-skip for all sources. Future ticket #778 (per-agent `docs_root`) will use `PathSource` to distinguish "operator explicitly configured this path — hard-error if missing" from "fell through to container default — warn-and-skip if missing".

### Why config-rs cascade instead of hand-rolled 3-step

The existing 4-source config model (documented in `simplified-config-4-source-model.md`) already handles env > config file precedence via `config-rs`'s `Environment::with_prefix("MIKA")` source. Adding `kg_docs_root: Option<PathBuf>` to `Settings` with `#[serde(default)]` gets env > config.toml precedence for free. The only hand-written fallback is the CWD default in step 3, which config-rs cannot express (it is not a config source, it is a runtime computation). This avoids reimplementing precedence logic that config-rs already provides and tests.

### Empty-path guard

`server/mod.rs` checks `docs_root.as_os_str().is_empty()` before the existence check and emits a distinct warning naming both possible config sources:

```
kg_docs_root is set to empty string — check MIKA_KG_DOCS_ROOT env var or kg_docs_root in config.toml; skipping lexical ingestion
```

Without this guard, an empty string would pass the emptiness check, fail the `docs_root.exists()` check, and produce the generic "not found" message — leaving operators chasing a filesystem problem when the real issue is a misconfigured value.

### No path validation in the resolver

The resolver returns the path without checking existence. Validation policy belongs to the consumer site. This keeps the resolver pure (testable without filesystem state) and allows #778 to apply a stricter policy (hard-error) downstream without forking the resolution logic.

## Files Changed

| File | Change |
|------|--------|
| `crates/mika-common/src/config.rs` | `kg_docs_root: Option<PathBuf>` field on `Settings`, `CONFIG_KEYS` entry, `get_effective_value` match arm, `test_defaults()` default, `Debug` impl field, `clean_env()` cleanup |
| `crates/mika-agent/src/kg/config.rs` | New module — `resolve_kg_docs_root()`, `PathSource` enum, 7 unit tests |
| `crates/mika-agent/src/kg/mod.rs` | `pub mod config;` re-export |
| `crates/mika-agent/src/server/mod.rs` | Replace inline CWD resolution with `resolve_kg_docs_root()` call + empty-path guard |
| `crates/mika-agent/tests/kg_docs_root_resolution.rs` | 3 integration tests verifying public API from outside the crate |
| `.env.example`, `docs/configuration.md`, `crates/mika-agent/CLAUDE.md`, root `CLAUDE.md` | Documentation of new env var and config field |

## Testing

**Unit tests** (`kg::config::tests`, 7 tests): env wins over config, config used when no env, CWD fallback when nothing set, empty env returns `EnvVar` source, empty config returns `ConfigFile` source, signature binding (compile-time drift detection for #778), `PathSource` exhaustiveness check (compile error on new variant).

**Config-layer tests** (`config.rs`, 5 tests): `kg_docs_root` defaults to `None`, env override populates `Some`, config file populates `Some`, env wins over config file through config-rs cascade, `get_effective_value` returns the display string.

**Integration tests** (`tests/kg_docs_root_resolution.rs`, 3 tests): nonexistent env path returned verbatim (no existence check), CWD default produces full path, public API types accessible from outside the crate.

All tests use `#[serial]` + env cleanup to avoid cross-test interference from `std::env::set_var`.

## Key Decisions

1. **Resolver lives in `mika-agent::kg::config`, not `mika-common`** — `mika-common` has no KG-specific concepts. Co-locating with the KG module matches existing layering and keeps the `pub fn` discoverable for #778.

2. **`PathSource` exists now, not deferred to #778** — adding the enum later would require changing the return type of a `pub fn` that #778 already depends on. Shipping it now means #778 can pattern-match on the source without a breaking signature change.

3. **Signature binding test** — `let _: fn(&Settings) -> (PathBuf, PathSource) = resolve_kg_docs_root;` catches accidental signature drift at compile time. #778 depends on this exact contract.

4. **Warn-and-skip, not hard-error** — consistent with the existing behavior where a missing `docs/solutions` directory is non-fatal. The server still boots; the KG layer stays empty. #778 will introduce hard-error semantics for per-agent misconfiguration using `PathSource` to distinguish the two policies.

## Operator Guidance

For non-container hosts where `mika-spirit` starts with CWD != repo root:

```bash
# Option A: env var (recommended for init scripts)
MIKA_KG_DOCS_ROOT=/path/to/mika-repo/docs/solutions

# Option B: config.toml
# In ~/.mika/config.toml:
kg_docs_root = "/path/to/mika-repo/docs/solutions"

# Option C: init-script --chdir (existing workaround, still works)
supervise-daemon mika-spirit --chdir /path/to/mika-repo ...
```

## Related

- **#738** — this ticket (CWD-dependent docs path fix)
- **#778** — per-agent `docs_root` using `PathSource` for policy classification
- **`simplified-config-4-source-model.md`** — the config cascade this solution builds on
- **`config-key-registry-cli-management.md`** — `CONFIG_KEYS` registry pattern followed for the new key
