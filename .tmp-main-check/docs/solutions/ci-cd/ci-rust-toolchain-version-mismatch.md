---
title: "CI Rust toolchain version mismatch with rust-toolchain.toml"
category: ci-cd
date: 2026-03-28
tags:
  - ci
  - rust
  - rustfmt
  - toolchain
  - cross-repo
severity: high
affected_components:
  - .github/workflows/ci.yml
  - rust-toolchain.toml
related_issues:
  - mika-cloud#61
---

# CI Rust Toolchain Version Mismatch

## Problem

PR #61 on mika-cloud failed CI's `cargo fmt --all -- --check` step even though the code was formatted locally. Different Rust versions produce different rustfmt output.

**Local:** `rust-toolchain.toml` pins to 1.93 → rustfmt 1.8.0
**CI:** `dtolnay/rust-toolchain@stable` installs latest stable (1.94.1) → different rustfmt

## Root Cause

Both mika and mika-cloud CI workflows used `dtolnay/rust-toolchain@631a55b...  # stable` which always installs the latest stable Rust, ignoring the `rust-toolchain.toml` file. This created a version split:

| Component | Version |
|-----------|---------|
| `rust-toolchain.toml` | 1.93 |
| Dockerfiles | rust:1.93 |
| CI workflow | latest stable (1.94.1) |
| Local dev | 1.93 (from rust-toolchain.toml) |

## Solution

Replaced the `dtolnay/rust-toolchain` action with `rustup show` which reads `rust-toolchain.toml` and installs the pinned toolchain. This makes `rust-toolchain.toml` the single source of truth for all environments.

```yaml
# Before
- uses: dtolnay/rust-toolchain@631a55b...  # stable

# After
- name: Install Rust toolchain
  run: rustup show  # reads rust-toolchain.toml
```

Applied to both mika and mika-cloud CI workflows.

## Prevention

When upgrading Rust version, update in one place: `rust-toolchain.toml`. CI, local dev, and lefthook all derive from it automatically. Dockerfiles still need manual updates (`FROM rust:X.YZ`).

Never use `dtolnay/rust-toolchain@stable` in repos that have `rust-toolchain.toml` — they will diverge over time.
