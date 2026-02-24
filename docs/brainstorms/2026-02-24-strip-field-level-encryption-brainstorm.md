# Strip Field-Level Encryption

**Date:** 2026-02-24
**Status:** Ready for planning

## What We're Building

Remove all field-level AES-256-GCM encryption, HMAC-SHA256 hash lookups, and the `EncryptionKey` infrastructure from Mika. Replace with plaintext SQLite columns, relying on Kubernetes encrypted volumes for data-at-rest protection.

## Why This Approach

The current field-level encryption was designed for a threat model that doesn't match Mika's architecture. Each customer gets their own container with their own volume on Kubernetes. The threats (disk theft, cross-tenant access) are already handled by K8s encrypted volumes at the infrastructure layer.

Field-level encryption adds significant complexity for no additional security benefit:
- 27 encrypt/decrypt call sites across db.rs
- 10 encrypted BLOB columns requiring deserialization
- ~150 lines of duplicated decrypt-or-skip filter_map patterns
- HMAC-SHA256 hashes for uniqueness lookups (can't use normal SQL)
- 3 crypto crate dependencies (ring, zeroize, hex)
- Broken preference search (todo #038) because HMAC is exact-match only
- FTS5 full-text search (Layer 3) impossible on encrypted columns

If an attacker has access to the running container's memory, field-level encryption is already defeated. If they have access to the volume at rest, K8s encryption handles it.

## Key Decisions

1. **Strip entirely** — No SQLCipher, no field-level encryption. Plaintext SQLite on encrypted K8s volumes.
2. **No startup integrity check** — SQLite's own integrity is sufficient.
3. **Case-insensitive uniqueness** — `COLLATE NOCASE` on unique TEXT columns (people names, preference categories). More forgiving for the LLM agent which may vary casing.
4. **Fresh schema (v4)** — Drop all tables and recreate with plaintext TEXT columns. No backward compatibility needed (pre-production, no real users).

## What Gets Removed

### Code
- `EncryptionKey` struct and all methods (`encrypt`, `decrypt`, `encrypt_string`, `decrypt_string`)
- `hmac_sha256_hex()` function
- `CryptoError` enum variants for encryption/decryption
- `check_encryption_key()` startup validation
- All `self.key.encrypt_string()` / `self.key.decrypt_string()` calls in db.rs (27 call sites)
- All `self.hmac_hash()` calls in db.rs
- All decrypt-or-skip `filter_map` patterns (~150 lines)
- `MIKA_ENCRYPTION_KEY` config field and env var
- Manual `Debug` impl on `Settings` (was for redacting encryption key)

### Dependencies
- `ring` — AES-256-GCM and HMAC (may keep if used elsewhere, but currently crypto-only)
- `zeroize` — ZeroizeOnDrop on EncryptionKey
- `hex` — Hex encoding for HMAC output

### Schema Changes
- All `*_encrypted` BLOB columns become plaintext TEXT columns
- All `*_hash` columns removed (no longer needed for lookups)
- UNIQUE constraints move to the plaintext TEXT columns with `COLLATE NOCASE`

### Config
- Remove `encryption_key` from `Settings` struct
- Remove `MIKA_ENCRYPTION_KEY` from `.env.example`
- Remove encryption key validation from CLI startup

## What Gets Simpler

- **Preference search (todo #038):** HMAC exact-match becomes SQL `LIKE` or `INSTR()`
- **FTS5 search (Layer 3):** Directly indexable plaintext columns
- **Database reads:** No decrypt step, no filter_map, no silent data loss on wrong key
- **Database writes:** No encrypt step, no nonce generation, no 28-byte overhead per field
- **Testing:** No `test_key()` helper needed, no encryption roundtrip assertions
- **Config:** One fewer required secret to manage per deployment

## Todos Resolved or Simplified

| Todo | Impact |
|------|--------|
| #038 Broken preference search | **Resolved** — plaintext enables LIKE/substring |
| #041 Plaintext metadata in memory_events | **Resolved** — all columns plaintext by design |
| #052 HMAC key reconstructed per call | **Resolved** — no HMAC |
| #054 Decrypt-or-skip duplication | **Resolved** — no decryption |
| #045 Test helper duplication | **Simplified** — test_key() no longer needed |

## Open Questions

None — all questions resolved during brainstorming.
