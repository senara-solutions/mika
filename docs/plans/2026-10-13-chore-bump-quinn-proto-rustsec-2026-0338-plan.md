---
date: 2026-10-13
type: chore
---

# Plan: bump quinn-proto to 0.11.19 (RUSTSEC-2026-0338, -0340, -0343)

## Problem

The Security job fails on every open PR: `cargo audit` flags `quinn-proto 0.11.17` with three advisories. The crate sits only in `Cargo.lock` (optional transitive dependency, never activated), but `cargo audit` reads the lockfile.

Once `cargo audit` passes, `cargo deny check` runs and fails on `unsafe-libyaml` 0.2.11 (RUSTSEC-2024-0449, unmaintained, no safe upgrade), pulled by the deprecated `serde_yaml` 0.9 in `mika-agent` and `mika-cli`.

## Fix

- `Cargo.lock`: `cargo update -p quinn-proto --precise 0.11.19`, keeping only the quinn-proto hunk.
- `deny.toml`: add `[[advisories.ignore]]` for RUSTSEC-2024-0449 with its reason, following the existing entries. The real remedy, replacing `serde_yaml`, is tracked in mika#2686.

## Acceptance criteria

- AC1: `Cargo.lock` diff is the quinn-proto version and checksum only; `cargo metadata --locked` succeeds.
- AC2: `cargo audit` passes with no quinn-proto advisory.
- AC3: `cargo deny check` passes; the RUSTSEC-2024-0449 entry names mika#2686.
- AC4: The Security job is green on the PR.
