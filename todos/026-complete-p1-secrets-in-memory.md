---
status: complete
priority: p1
issue_id: "026"
tags: [code-review, security, rust-v2]
dependencies: []
---

# Secrets Exposed in Memory and Debug Output

## Problem Statement

Three related issues expose sensitive key material:

1. `EncryptionKey` stores raw `[u8; 32]` with `#[derive(Clone)]`, never zeroized on drop
2. `Settings` derives `Debug`, dumping `anthropic_api_key`, `encryption_key`, `openai_api_key` to logs/stderr
3. `ClaudeClient` API key could leak via `InvalidHeaderValue` debug output in error chains

**Why it matters:** Memory dumps, core dumps, log files, or panic output can expose the master encryption key and API credentials.

## Findings

- **Source:** Security Sentinel (C2, C3, H1), Architecture Strategist (B)
- **Locations:**
  - `crates/mika-common/src/crypto.rs:24-27` — `EncryptionKey` Clone without zeroize
  - `crates/mika-common/src/config.rs:4` — `#[derive(Debug)]` on Settings
  - `crates/mika-common/src/claude.rs:187` — API key in HeaderValue error chain

## Proposed Solutions

### Option A: zeroize + manual Debug impls (Recommended)
- Add `zeroize` crate, derive `ZeroizeOnDrop` on `EncryptionKey`
- Remove `Clone` from `EncryptionKey`, wrap in `Arc` if sharing needed
- Implement `Debug` manually on `Settings` to redact secrets
- Map `HeaderValue::from_str` error to opaque message
- **Pros:** Defense-in-depth, prevents all three leak vectors
- **Cons:** Minor API changes to EncryptionKey (no Clone)
- **Effort:** Small
- **Risk:** Low

## Acceptance Criteria

- [ ] `EncryptionKey` key bytes zeroed on drop (test with `zeroize`)
- [ ] `Settings` Debug output shows `[REDACTED]` for all secret fields
- [ ] API key error message says "invalid characters" without revealing the key
- [ ] No `println!("{:?}", settings)` anywhere in codebase
