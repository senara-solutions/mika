# Plan: Bump rand to 0.9.3+ to clear RUSTSEC-2026-0097

**Issue:** #539
**Type:** chore (dependency maintenance)
**Priority:** p1-important (sprint blocker)

## Context

Advisory RUSTSEC-2026-0097 affects `rand >= 0.7, < 0.9.3` and `0.10.0`. Our lockfile has `rand 0.9.2` (affected), used by `mika-cli`, `mika-common`, `mika-agent`, and `mika-gateway`. This blocks the Sprint 2026-04-11 CI security job.

## Changes

### 1. Bump rand to 0.9.3+

- Run `cargo update -p rand@0.9.2 --precise 0.9.3` (or latest 0.9.x if available)
- Verify the lockfile shows `rand 0.9.3+`

### 2. Remove stale deny.toml exemptions

Delete two resolved advisory ignores from `deny.toml`:
- **RUSTSEC-2023-0071** (line 9) — rsa Marvin Attack exemption
- **RUSTSEC-2026-0002** (line 17) — lru IterMut unsoundness exemption

### 3. Verify

- `cargo build` — ensure compilation succeeds
- `cargo test` — ensure no regressions
- `cargo deny check advisories` — confirm no advisory warnings remain (except RUSTSEC-2024-0436 paste, which is retained)

## Risk

Minimal — patch-level semver bump within 0.9.x. No API changes expected.
