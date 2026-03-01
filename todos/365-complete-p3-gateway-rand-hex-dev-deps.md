---
status: pending
priority: p3
issue_id: "365"
tags: [code-review, dependencies, gateway]
dependencies: []
---

# Move rand and hex to dev-dependencies in mika-gateway

## Problem Statement

`rand` and `hex` are listed as production dependencies in `crates/mika-gateway/Cargo.toml` but are only used inside `#[cfg(test)]` blocks (`generate_pairing_token` test helper). They are unnecessarily linked into the production binary.

## Proposed Solutions

### Option A: Move to dev-dependencies (Recommended)
- Move `rand` and `hex` from `[dependencies]` to `[dev-dependencies]`
- Effort: Small
- Risk: None

## Technical Details

**Affected files:**
- `crates/mika-gateway/Cargo.toml`

## Acceptance Criteria

- [ ] `rand` and `hex` are under `[dev-dependencies]`
- [ ] `cargo test -p mika-gateway` still passes
- [ ] `cargo build -p mika-gateway` still compiles
