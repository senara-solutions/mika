---
title: "fix: Pin CI Rust toolchain to rust-toolchain.toml"
type: fix
status: active
date: 2026-03-28
---

# fix: Pin CI Rust toolchain to rust-toolchain.toml

## Overview

CI uses `dtolnay/rust-toolchain@stable` which installs the latest stable Rust (currently 1.94.1), but `rust-toolchain.toml` pins to 1.93. Different rustfmt versions produce different formatting — code formatted locally with 1.93 fails CI's 1.94.1 `cargo fmt --check`. This affects both mika and mika-cloud repos.

## Acceptance Criteria

- [x] CI reads Rust version from `rust-toolchain.toml` (single source of truth)
- [x] No version duplication between CI workflow and `rust-toolchain.toml`
- [x] `cargo fmt`, `cargo clippy`, `cargo test` all use the pinned toolchain in CI

## Solution

Replace `dtolnay/rust-toolchain@...  # stable` with `rustup show` which reads `rust-toolchain.toml` and installs the specified toolchain. The `Swatinem/rust-cache` action continues to work since the toolchain is installed before it runs.

### Before
```yaml
- uses: dtolnay/rust-toolchain@631a55b12751854ce901bb631d5902ceb48146f7  # stable
```

### After
```yaml
- name: Install Rust toolchain
  run: rustup show  # reads rust-toolchain.toml
```

### Files to modify

1. `.github/workflows/ci.yml` — replace dtolnay action with `rustup show` (both Check and Security jobs)

Cross-repo: same change in mika-cloud.

## Sources

- mika-cloud PR #61 CI failure: cargo fmt mismatch between local 1.93 and CI 1.94.1
- `rust-toolchain.toml`: pins to `channel = "1.93"`
- Dockerfiles: already pin to `rust:1.93`
