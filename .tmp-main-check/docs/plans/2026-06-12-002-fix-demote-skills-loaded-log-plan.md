---
title: "fix: Demote 'skills loaded' log from INFO to DEBUG"
date: 2026-06-12
type: fix
origin: "GitHub issue #619"
status: draft
---

## Summary

Demote the `skills loaded` log line in `SkillRegistry::log_summary()` from `tracing::info!` to `tracing::debug!` so it no longer pollutes stderr on every CLI subcommand invocation under the default `info` log level.

## Problem Frame

Running any CLI subcommand (e.g., `mika skills list | grep qa`) prints a noisy `INFO skills loaded count=N names=[...]` line to stderr. While stderr correctly bypasses pipes, it clutters the terminal. The root cause is `log_summary()` emitting at INFO unconditionally — including short-lived read-only CLI commands where `log_level = "info"` is the default.

## Requirements

- R1. `tracing::info!` → `tracing::debug!` at the "skills loaded" summary line
- R2. Per-skip `tracing::warn!` lines remain unchanged
- R3. `crates/mika-agent/CLAUDE.md` updated to reflect the new level
- R4. `mika skills list` no longer prints "skills loaded" at default log level
- R5. `RUST_LOG=debug` or `log_level=debug` still shows the line

## Key Technical Decisions

- **KTD-1: debug, not trace.** The information is useful for diagnosing skill loading issues but not needed in normal operation. `debug` is the correct level — visible when requested, silent by default.

## Implementation Units

### U1. Demote log level and update doc

**Goal:** Change one `tracing::info!` macro call and update the corresponding CLAUDE.md reference.

**Requirements:** R1, R2, R3, R4, R5

**Dependencies:** None

**Files:**
- `crates/mika-agent/src/skills/mod.rs` (modify)
- `crates/mika-agent/CLAUDE.md` (modify)

**Approach:** Change `tracing::info!` to `tracing::debug!` at `log_summary()`. Update the doc comment above the method and the CLAUDE.md "Startup logging" paragraph from "INFO" to "DEBUG".

**Patterns to follow:** Existing `tracing::debug!` usage throughout the crate.

**Test scenarios:**
- Verify `tracing::warn!` per-skip lines are unchanged (code inspection)
- Verify the doc comment and CLAUDE.md reference say DEBUG, not INFO

**Verification:** `cargo clippy` and `cargo test -p mika-agent` pass. Grep confirms no remaining `info!` at the "skills loaded" callsite.
