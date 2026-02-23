---
status: complete
priority: p2
issue_id: "032"
tags: [code-review, security, performance, rust-v2]
dependencies: []
---

# AES-256-GCM: Cache LessSafeKey + Use Non-Empty AAD

## Problem Statement

Two crypto issues:
1. `EncryptionKey::make_key()` reconstructs `LessSafeKey` (AES key schedule expansion) on every encrypt/decrypt call — 25-30 times per agent turn
2. Empty Associated Authenticated Data (AAD) means encrypted blobs can be relocated between rows/tables without detection

**Why it matters:** Performance waste on hot path + ciphertext relocation attacks possible.

## Findings

- **Source:** Performance Oracle (CRITICAL-2), Security Sentinel (M5)
- **Locations:**
  - `crates/mika-common/src/crypto.rs:41-45` — make_key() called per operation
  - `crates/mika-common/src/crypto.rs:58,83` — `Aad::empty()`

## Proposed Solutions

### Option A: Cache key + table/row AAD (Recommended)
- Pre-compute `LessSafeKey` in `EncryptionKey::from_hex()`, store alongside raw bytes
- Add `encrypt_with_context(plaintext, table, row_id)` that binds AAD
- Keep backward-compat `encrypt()` for migration period
- **Pros:** Eliminates redundant key expansion, prevents ciphertext relocation
- **Cons:** Changes encrypt/decrypt API (add context parameter), `LessSafeKey` is not Clone
- **Effort:** Medium
- **Risk:** Medium (need to re-encrypt existing data or handle both formats)

### Option B: Cache key only (quick win)
- Pre-compute and store `LessSafeKey`, defer AAD to later
- **Pros:** Simple, immediate performance gain
- **Cons:** Leaves ciphertext relocation vulnerability open
- **Effort:** Small
- **Risk:** Low

## Acceptance Criteria

- [ ] AES key schedule expanded once at construction, not per operation
- [ ] Benchmark shows measurable improvement on bulk encrypt/decrypt
- [ ] (If AAD added) Encrypted blob from one table cannot be used in another
