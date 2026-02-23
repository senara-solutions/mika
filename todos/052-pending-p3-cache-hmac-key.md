---
status: pending
priority: p3
issue_id: "052"
tags: [code-review, performance, crypto, rust-v2]
dependencies: []
---

# HMAC Key Reconstructed on Every Call

## Problem Statement
`hmac_sha256_hex()` in crypto.rs reconstructs an `hmac::Key` from raw bytes on every call. During `search_memory`, this is called dozens of times. The `ring::hmac::Key::new` performs key expansion internally.

**Location:** `crates/mika-common/src/crypto.rs:112-117`

**Reported by:** performance-oracle

## Proposed Solutions
Cache the `hmac::Key` in the `EncryptionKey` struct alongside `cached_key`.
- **Effort:** Small

## Work Log
| Date | Action | Learnings |
|------|--------|-----------|
| 2026-02-24 | Created from multi-agent code review | Individually ~100ns per call, adds up in search |
