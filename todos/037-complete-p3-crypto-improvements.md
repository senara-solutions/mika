---
status: complete
priority: p3
issue_id: "037"
tags: [code-review, security, rust-v2]
dependencies: []
---

# Minor Crypto Improvements: hex Crate, HMAC Dedup, ring Consolidation

## Problem Statement

Three minor crypto-related improvements:
1. Hand-rolled hex decoder in `crypto.rs` could be replaced with the audited `hex` crate
2. Commitment dedup uses unsalted SHA-256 — common descriptions can be rainbow-tabled
3. `ring` is a dependency of both `mika-common` and `mika-agent` — the `sha256_hex` function in db.rs should live in `mika-common::crypto`

**Why it matters:** Defense-in-depth improvements and dependency hygiene.

## Findings

- **Source:** Security Sentinel (M1, L3), Architecture Strategist (L)
- **Locations:**
  - `crates/mika-common/src/crypto.rs:101-111` — custom hex module
  - `crates/mika-agent/src/db.rs:657-663` — sha256_hex using ring directly
  - `crates/mika-agent/Cargo.toml:18` — ring dependency

## Proposed Solutions

### Option A: hex crate + HMAC + consolidate ring (Recommended)
- Add `hex` crate to workspace deps, replace custom module
- Move `sha256_hex` to `mika-common::crypto`, change to `hmac_sha256_hex` using encryption key
- Remove `ring` from `mika-agent/Cargo.toml`
- **Effort:** Small
- **Risk:** Low

## Acceptance Criteria

- [ ] No hand-rolled hex decoder
- [ ] Commitment dedup hash uses HMAC (not plain SHA-256)
- [ ] `ring` only appears in `mika-common/Cargo.toml`
