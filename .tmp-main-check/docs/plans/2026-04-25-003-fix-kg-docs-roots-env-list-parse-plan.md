---
title: "fix: MIKA_KG_DOCS_ROOTS env var fails to parse as Vec<PathBuf>"
type: fix
status: active
date: 2026-04-25
---

# fix: MIKA_KG_DOCS_ROOTS env var fails to parse as Vec<PathBuf>

## Overview

Setting `MIKA_KG_DOCS_ROOTS=/path/a:/path/b` causes a startup deserialization error because config-rs delivers the raw colon-separated string to serde, which expects a sequence (`Vec<PathBuf>`). The fix adds a custom serde deserializer that accepts both a string (splits on `:`) and a native array.

## Problem Frame

`kg_docs_roots` is an `Option<Vec<PathBuf>>` field in `Settings`. Three config sources can provide it:

1. **TOML config file** (`config.toml`) — native TOML array syntax works correctly.
2. **Agent `.env` file** — `dotenv_to_toml()` emits it as a TOML string (`kg_docs_roots = "/a:/b"`), which fails deserialization.
3. **Process environment** — config-rs `Environment` source delivers the raw string, which fails deserialization.

Sources 2 and 3 both fail because serde sees a string where it expects a sequence. mika-arch depends on this field for multi-corpus KG — without it, the agent is skipped during provisioning.

## Requirements Trace

- R1. `MIKA_KG_DOCS_ROOTS=/a:/b:/c` parses as a 3-element `Vec<PathBuf>` without errors
- R2. Removing `kg_docs_roots` from config.toml and setting only the env var still provisions mika-arch
- R3. Existing TOML-array path in config.toml continues to work
- R4. CLAUDE.md description "Colon-separated list of absolute paths" remains accurate

## Scope Boundaries

- No changes to `dotenv_to_toml()` — the custom deserializer handles its string output
- No `try_parsing(true)` on the config-rs Environment source — that changes global parsing behavior for all env vars, risking type coercion issues on string fields
- No new public types or API changes — the field type stays `Option<Vec<PathBuf>>`

## Context & Research

### Relevant Code and Patterns

- `crates/mika-common/src/config.rs:817` — field declaration: `pub kg_docs_roots: Option<Vec<PathBuf>>`
- `crates/mika-common/src/config.rs:1196-1203` — config-rs builder with `Environment::with_prefix("MIKA")`, no list parsing configured
- `crates/mika-common/src/config.rs:548` — `get_effective_value` arm for `kg_docs_roots` (formats Vec for display, no change needed)
- `crates/mika-common/src/dotenv.rs:53-68` — `dotenv_to_toml` emits all values as TOML strings (line 64: `"{escaped}"`)
- `crates/mika-common/src/config.rs:1438-1470` — existing config tests with `clean_env()` helper and `#[serial]`

### Institutional Learnings

- `docs/solutions/architecture-patterns/simplified-config-4-source-model.md` — config-rs env source delivers all values as flat strings; list types need explicit handling
- `docs/solutions/architecture-patterns/kg-docs-root-config-driven-resolution-2026-04-24.md` — the singular `kg_docs_root` follows the same pattern; this fix applies only to the plural form

## Key Technical Decisions

- **Custom serde deserializer over config-rs `try_parsing`:** config-rs 0.15 requires `.try_parsing(true)` as a prerequisite for `.list_separator()` + `.with_list_parse_key()`. Enabling `try_parsing` globally auto-parses ALL env vars as bool/i64/f64, risking unintended type coercion for string-typed fields (e.g., model names, log levels, API keys). A field-level custom deserializer is zero-risk and handles all three config sources in one place.
- **Colon as separator, not configurable:** Matches the documented contract. Windows is explicitly unsupported for this field (doc comment says "Linux/macOS only").
- **Empty segments filtered:** `/a::/b` produces 2 paths, not 3. Trailing/leading colons are tolerated.

## Open Questions

### Resolved During Planning

- **Does config-rs 0.15 support `with_list_parse_key`?** Yes, but only with `try_parsing(true)`, which has global side effects. Rejected in favor of custom deserializer.
- **Does `dotenv_to_toml` need a parallel fix?** No — the custom deserializer handles the string it emits.

### Deferred to Implementation

- None — the fix is fully scoped.

## Implementation Units

- [ ] **Unit 1: Add custom deserializer and wire it to the field**

  **Goal:** Make `kg_docs_roots` accept both a colon-separated string and a native TOML/JSON array during deserialization.

  **Requirements:** R1, R2, R3

  **Dependencies:** None

  **Files:**
  - Modify: `crates/mika-common/src/config.rs`

  **Approach:**
  - Add a `deserialize_colon_paths` function implementing a serde `Visitor` that handles `visit_str` (split on `:`), `visit_seq` (collect elements), `visit_none`/`visit_unit` (return `None`)
  - Annotate the `kg_docs_roots` field with `#[serde(default, deserialize_with = "deserialize_colon_paths")]`
  - Place the deserializer near the field declaration or in a private `serde_helpers` block within the same file

  **Patterns to follow:**
  - Standard serde `Visitor` pattern with `deserialize_any` to accept multiple input types
  - Existing field-level serde attributes in Settings (e.g., `#[serde(default)]`, `#[serde(default = "default_true")]`)

  **Test scenarios:**
  - Happy path: env var `MIKA_KG_DOCS_ROOTS=/a:/b:/c` → `Some(vec!["/a", "/b", "/c"])` as `Vec<PathBuf>`
  - Happy path: TOML array `kg_docs_roots = ["/a", "/b"]` → `Some(vec!["/a", "/b"])`
  - Happy path: single path `MIKA_KG_DOCS_ROOTS=/a` → `Some(vec!["/a"])`
  - Edge case: empty string `MIKA_KG_DOCS_ROOTS=` → `None`
  - Edge case: consecutive colons `MIKA_KG_DOCS_ROOTS=/a::/b` → `Some(vec!["/a", "/b"])` (empty segments filtered)
  - Edge case: trailing colon `MIKA_KG_DOCS_ROOTS=/a:/b:` → `Some(vec!["/a", "/b"])`
  - Edge case: not set → `None` (serde default)

  **Verification:**
  - `cargo test -p mika-common` passes with all new tests green
  - `cargo clippy -p mika-common` clean

- [ ] **Unit 2: Add integration tests for env-var and TOML paths**

  **Goal:** Verify the full `Settings::load` path handles `kg_docs_roots` from both env vars and config.toml.

  **Requirements:** R1, R2, R3

  **Dependencies:** Unit 1

  **Files:**
  - Modify: `crates/mika-common/src/config.rs` (in `#[cfg(test)] mod tests`)

  **Approach:**
  - Add tests in the existing `mod tests` block using the `clean_env()` + `#[serial]` pattern
  - Test env var path: set `MIKA_KG_DOCS_ROOTS`, call `Settings::load`, assert `kg_docs_roots` field
  - Test TOML path: write `config.toml` with `kg_docs_roots = ["/a", "/b"]`, call `Settings::load`, assert field
  - Test precedence: set both env var and TOML, verify env wins (config-rs source ordering)

  **Patterns to follow:**
  - `test_defaults()` and `test_home_config_loaded()` in the same test module
  - `clean_env()` helper clears `MIKA_KG_DOCS_ROOTS` (already present at line 1451)

  **Test scenarios:**
  - Integration: env var `/a:/b:/c:/d` → Settings has 4-element Vec via `Settings::load`
  - Integration: TOML array in config.toml → Settings has correct Vec via `Settings::load`
  - Integration: env var overrides TOML array (source precedence)
  - Integration: neither set → `kg_docs_roots` is `None`

  **Verification:**
  - `cargo test -p mika-common -- kg_docs_roots` passes

## System-Wide Impact

- **Interaction graph:** `Settings::load` → `kg_docs_roots` field → `crates/mika-agent/src/kg/config.rs` (reads field for multi-corpus) → `provision_well_known_agents` (mika-arch). No callbacks or middleware involved.
- **Error propagation:** Deserialization errors previously caused `Settings::load` to fail entirely (`anyhow::bail`). With the custom deserializer, malformed values are handled gracefully.
- **Unchanged invariants:** `get_effective_value("kg_docs_roots")` format unchanged. `dotenv_to_toml` unchanged. Config-rs source ordering unchanged.
- **Parallel code path (intentional):** `crates/mika-agent/src/kg/config.rs` Tier 3 reads `MIKA_KG_DOCS_ROOTS` directly via `std::env::var` and splits on `:` independently. This is by design — the 6-tier resolution chain resolves per-agent docs roots at runtime, while `Settings.kg_docs_roots` feeds `build_mika_arch_identity()` at provision time. Both splitting implementations use `.split(':').filter(|p| !p.is_empty())` — the custom deserializer must use the same filtering logic to stay aligned.

## Risks & Dependencies

| Risk | Mitigation |
|------|------------|
| Custom deserializer silently accepts unexpected input types | `deserialize_any` visitor only implements `visit_str`, `visit_seq`, `visit_none`, `visit_unit` — other types produce a clear serde error |
| Future list-type fields need the same treatment | Document the pattern; consider extracting a generic `colon_separated_list` deserializer if a second field appears |

## Documentation / Operational Notes

- CLAUDE.md `MIKA_KG_DOCS_ROOTS` description is already accurate ("Colon-separated list of absolute paths") — no doc change needed (R4)
- The config.toml workaround (TOML array syntax) continues to work; operators can remove it once the fix is deployed

## Sources & References

- Related issue: #814
- Related PRs: #813 (mika-arch v1 Units 2-6 — surfaced the gap)
- Related milestone: senara-solutions/mika-platform#51 (mika-arch v1)
- config-rs 0.15 source: `~/.cargo/registry/src/*/config-0.15.22/src/env.rs` lines 302-332 (try_parsing gate)
