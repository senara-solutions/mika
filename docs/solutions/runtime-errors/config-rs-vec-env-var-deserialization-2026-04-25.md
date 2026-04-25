---
title: "config-rs Vec<PathBuf> env var fails to deserialize from colon-separated string"
date: 2026-04-25
category: runtime-errors
module: mika-common (config.rs)
problem_type: runtime_error
component: tooling
symptoms:
  - "invalid type: string \"/path/a:/path/b\", expected a sequence for key `kg_docs_roots`"
  - "mika-arch agent not provisioned on startup despite MIKA_KG_DOCS_ROOTS being set"
root_cause: config_error
resolution_type: code_fix
severity: high
tags:
  - config-rs
  - serde
  - env-var
  - vec-pathbuf
  - kg-docs-roots
  - custom-deserializer
---

# config-rs Vec<PathBuf> env var fails to deserialize from colon-separated string

## Problem

Setting `MIKA_KG_DOCS_ROOTS=/path/a:/path/b:/path/c` in `~/.mika/.env` or as a process environment variable caused a startup deserialization error. The `kg_docs_roots` field (typed as `Option<Vec<PathBuf>>`) could not be populated from any env-var source, which silently prevented mika-arch from provisioning.

## Symptoms

- Server log shows: `invalid type: string "/path/a:/path/b:/path/c:/path/d", expected a sequence for key 'kg_docs_roots'`
- `mika agents list` does not include mika-arch
- `SELECT COUNT(*) FROM agent_kg_corpora WHERE agent_id='mika-arch'` returns 0

## What Didn't Work

- **Option A from the issue: config-rs `list_separator` + `with_list_parse_key`** — Investigated the config-rs 0.15 Environment source API. The `list_separator(":")` and `with_list_parse_key("kg_docs_roots")` methods exist but are gated behind `try_parsing(true)` (confirmed in `~/.cargo/registry/src/*/config-0.15.22/src/env.rs` line 302). Enabling `try_parsing` globally auto-parses ALL env vars as bool/i64/f64 before falling through to string, risking unintended type coercion for string-typed fields like model names, API keys, and log levels. Rejected due to blast radius.

## Solution

Added a custom serde deserializer `deserialize_colon_paths` that accepts both a colon-separated string and a native TOML/JSON array, wired via `#[serde(default, deserialize_with = "deserialize_colon_paths")]`.

The `ColonPathsVisitor` implements:
- `visit_str` — splits on `:`, filters empty segments, returns `Some(vec![...])` or `None`
- `visit_seq` — collects elements from TOML/JSON arrays, filters empty strings
- `visit_none` / `visit_unit` — returns `None`

```rust
fn deserialize_colon_paths<'de, D>(deserializer: D) -> Result<Option<Vec<PathBuf>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    struct ColonPathsVisitor;

    impl<'de> de::Visitor<'de> for ColonPathsVisitor {
        type Value = Option<Vec<PathBuf>>;
        // visit_str: split on ':', filter empty, map to PathBuf
        // visit_seq: collect non-empty elements as PathBuf
        // visit_none/visit_unit: return None
    }

    deserializer.deserialize_any(ColonPathsVisitor)
}
```

This handles all three config sources:
1. **Process env var** — config-rs delivers raw string → `visit_str` splits
2. **Agent `.env` file** — `dotenv_to_toml()` emits as TOML string → `visit_str` splits
3. **TOML config file** — native array → `visit_seq` collects

## Why This Works

config-rs 0.15 without `try_parsing(true)` delivers all environment variables as `ValueKind::String`. Serde then tries to deserialize a string into `Option<Vec<PathBuf>>`, which fails because serde expects a sequence. The custom deserializer intercepts the deserialization via `deserialize_any`, which calls the appropriate `Visitor` method based on the actual value type — `visit_str` for env-var strings, `visit_seq` for TOML arrays. This is a field-level fix with zero risk to other Settings fields.

## Prevention

- **Pattern for future list-type env vars:** If a second `Vec<>` field is added to Settings with env-var support, extract `deserialize_colon_paths` into a generic `deserialize_colon_separated_list<T: From<String>>` helper. One field doesn't justify the abstraction yet.
- **Alignment with parallel code paths:** `crates/mika-agent/src/kg/config.rs` Tier 3 reads `MIKA_KG_DOCS_ROOTS` directly via `std::env::var` and splits independently using the same `.split(':').filter(|p| !p.is_empty())` logic. If the splitting rule changes, both sites must be updated. The deserializer doc comment references this alignment explicitly.

## Related Issues

- [#814](https://github.com/senara-solutions/mika/issues/814) — This issue
- [#813](https://github.com/senara-solutions/mika/pull/813) — mika-arch v1 Units 2-6 (surfaced the gap during smoke test)
- [#798](https://github.com/senara-solutions/mika/issues/798) — Multi-corpus per-agent KG (introduced `kg_docs_roots`)
- `docs/solutions/architecture-patterns/kg-docs-root-config-driven-resolution-2026-04-24.md` — Singular `kg_docs_root` resolution chain (same config-rs cascade model)
- `docs/solutions/architecture-patterns/simplified-config-4-source-model.md` — config-rs 4-source cascade (documents the env-as-string limitation)
