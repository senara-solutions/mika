---
module: crates/mika-common/src/config.rs
tags: [config, config-rs, env-vars, rename, mika-spirit]
problem_type: best-practice
category: best-practices
date: 2026-06-15
ticket: mika#1535 (rename)
applies_when:
  - Renaming an `MIKA_<FIELD>` env var
  - Adding a new env var that maps to a `Settings` struct field
  - Debugging "env var has no effect at runtime" / "default value used despite env override"
resolution_type: discipline
---

# config-rs binds env vars to struct fields BY NAME — rename them together

## TL;DR

When renaming an `MIKA_FOO_BAR` env var, also rename the `Settings` struct field `foo_bar` in the same change. Otherwise the env var is silently dropped and the default value is used. config-rs's `Environment::with_prefix("MIKA")` source maps env vars to fields lexically — there is no separate `#[serde(rename = …)]` indirection in the way settings are loaded.

## Founding incident (mika#1535, 2026-06-14)

The mika-server → mika-spirit hard cutover renamed `MIKA_SERVER_PORT` → `MIKA_SPIRIT_PORT` and `MIKA_SERVER_LOG_FILE` → `MIKA_SPIRIT_LOG_FILE` in the deployed `~/.mika/.env`. Smoke tests failed with `Address already in use` because the running mika-server was on 8081 and the new mika-spirit process was also trying to bind 8081 — the test expected the new process to use a different port via `MIKA_SPIRIT_PORT=<free>`, but config-rs ignored it because the `Settings` struct still had `server_port`, not `spirit_port`.

The fix: rename the struct fields (`server_port` → `spirit_port`, `server_log_file` → `spirit_log_file`) in the same PR + update all 16 callsites. With both renames done, `MIKA_SPIRIT_PORT` bound correctly.

## Why this happens

`config-rs` `Environment::with_prefix("MIKA")` builds a key path by:

1. Stripping the prefix `MIKA_`
2. Lowercasing the remainder
3. Splitting on configured separators (default: `.` — but you can configure `_` as separator for nested maps; mika does not)

So `MIKA_SPIRIT_PORT` becomes the flat key `spirit_port`. The deserializer then looks up that key in the `Settings` struct **by field name**. No match → field stays at its default. No error, no warning — the source silently contributes nothing.

If you want the deserializer to map `MIKA_SPIRIT_PORT` to a field named `server_port`, you must add `#[serde(rename = "spirit_port")]` (or alias). But Mika's convention is **identity binding** — env-var name = struct field name (uppercased + prefixed) — because the alternative requires every binding to be inspected for an aliased mapping. Identity binding is the default discipline; renames touch both sides together.

## What the bug looks like at runtime

- `~/.mika/.env` has the new env var set.
- The process starts without errors.
- `Settings::default()` value is used in place of the configured value.
- For port-like fields: another process holding the default port → bind failure.
- For path-like fields: log/data file is at the default location instead of the configured one.
- For boolean flags: behavior matches `false` (the default) even when env is `true`.

No log line says "env var dropped." The miss is silent. The diagnostic signal is the **observed behavior diverging from the configured env**.

## Checklist when renaming an env var

1. Update the env-var name everywhere (`.env`, deploy scripts, docs, CI workflows, Helm charts).
2. Update the corresponding `Settings` struct field name in `crates/mika-common/src/config.rs`.
3. Update **every callsite** of the renamed field — `cargo build` will catch these as errors. (mika#1535 had ~16 callsites.)
4. Update `Settings::test_defaults()` in the same file.
5. Update the `Debug` impl in the same file (it explicitly lists field names).
6. Update any documentation listing the struct field names (docs/configuration.md table — both `crates/mika-agent/docs/configuration.md` mirror and the canonical `docs/configuration.md`).
7. Verify with `grep -rn "MIKA_OLDNAME\|old_field_name"` that no stragglers remain.

## Related

- `feedback_verify_pipeline_passes_without_the_fix` — silent-fail diagnostic principle; this is the env-var/field analog.
- `docs/solutions/architecture-patterns/cli-flag-subcommand-scoping.md` — adjacent: where surface-layer naming matters for binding.
- mika#1535 — the rename ticket.
- mika#1537 — the follow-up that swept extensionless files (OpenRC init.d, systemd units, debian postinst/prerm) that `sed --include` missed in the original sweep.

## Out of scope

- Adding `#[serde(alias)]` to provide backwards-compat for the old env var name. Mika is pre-1.0 — breaking renames are shipped without compat shims (see `mika/CLAUDE.md` § Versioning). If you ARE adding compat for a >1.0 binding, alias is the mechanism.
