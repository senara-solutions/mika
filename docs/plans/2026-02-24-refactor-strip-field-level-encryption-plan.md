---
title: "Strip Field-Level Encryption"
type: refactor
status: completed
date: 2026-02-24
---

# Strip Field-Level Encryption

## Overview

Remove all field-level AES-256-GCM encryption, HMAC-SHA256 hash lookups, and the `EncryptionKey` infrastructure from Mika. Replace encrypted BLOB columns with plaintext TEXT columns. Rely on Kubernetes encrypted volumes for data-at-rest protection.

## Problem Statement

The current field-level encryption adds significant complexity (27 encrypt/decrypt call sites, ~150 lines of decrypt-or-skip patterns, 3 crypto dependencies) for no additional security benefit given Mika's per-customer container isolation architecture. It also blocks features: preference search is broken (todo #038) because HMAC is exact-match only, and FTS5 full-text search (Layer 3) is impossible on encrypted columns.

## Proposed Solution

Fresh schema v4 that drops all tables and recreates with plaintext TEXT columns. Remove `EncryptionKey`, `CryptoError`, HMAC functions, and all encrypt/decrypt call sites. Remove `ring`, `zeroize`, `hex` dependencies.

## Implementation Phases

### Phase 1: Delete crypto module and update config

Remove the encryption infrastructure that everything else depends on.

**Files:**

- [x] `crates/mika-common/src/crypto.rs` — Delete entire file (~200 lines)
- [x] `crates/mika-common/src/lib.rs` — Remove `pub mod crypto;`
- [x] `crates/mika-common/src/config.rs` — Remove `encryption_key: String` field from `Settings` struct. Remove manual `Debug` impl (lines 99-113), replace with `#[derive(Debug)]`. Remove `MIKA_ENCRYPTION_KEY` from test setup.
- [x] `crates/mika-common/Cargo.toml` — Remove `ring`, `zeroize`, `hex` dependencies
- [x] `Cargo.toml` (workspace root) — Remove version pins for `ring`, `zeroize`, `hex` if present
- [x] `.env.example` — Remove `MIKA_ENCRYPTION_KEY` line
- [x] `config/default.toml` — Remove `encryption_key` default if present

### Phase 2: Update Database struct and migration

Remove EncryptionKey from Database, add migration v4.

**Files:**

- [x] `crates/mika-agent/src/db.rs` — Database struct changes:
  - Remove `key: EncryptionKey` field from `Database` struct
  - Change `pub fn open(path: &str, key: EncryptionKey)` → `pub fn open(path: &str)`
  - Change `pub fn open_in_memory(key: EncryptionKey)` → `pub fn open_in_memory()`
  - Remove `fn hmac_hash(&self, input: &str)` helper method
  - Remove `pub fn check_encryption_key(&self)` method
  - Remove `use mika_common::crypto::EncryptionKey;` import

- [x] `crates/mika-agent/src/db.rs` — Add `migrate_v4()`:
  - Bump `CURRENT_SCHEMA_VERSION` from 3 to 4
  - Add `migrate_v4(conn)` function: DROP all tables, CREATE with plaintext TEXT columns
  - Schema changes:
    | Table | Old Column | New Column |
    |-------|-----------|------------|
    | conversations | `content_encrypted BLOB` | `content TEXT NOT NULL` |
    | core_memory | `value_encrypted BLOB` | `value TEXT NOT NULL` |
    | people | `canonical_name_encrypted BLOB` + `canonical_name_hash TEXT UNIQUE` | `canonical_name TEXT NOT NULL UNIQUE COLLATE NOCASE` |
    | people | `relationship_encrypted BLOB` | `relationship TEXT` |
    | people | `notes_encrypted BLOB` | `notes TEXT` |
    | commitments | `description_encrypted BLOB` + `description_hash TEXT UNIQUE` | `description TEXT NOT NULL UNIQUE COLLATE NOCASE` |
    | preferences | `category_encrypted BLOB` + `category_hash TEXT UNIQUE` | `category TEXT NOT NULL UNIQUE COLLATE NOCASE` |
    | preferences | `value_encrypted BLOB` | `value TEXT NOT NULL` |
    | events | `description_encrypted BLOB` | `description TEXT NOT NULL` |
  - Add `v4` case to `migrate()` match

### Phase 3: Simplify all database methods

Remove all encrypt/decrypt calls and filter_map patterns. This is the bulk of the work.

**File: `crates/mika-agent/src/db.rs`**

- [x] `save_message()` — Remove `self.key.encrypt_string(content)?`, insert content as TEXT directly
- [x] `load_recent_messages()` — Remove decrypt filter_map, read `content` as TEXT. Remove `RawConversationRow` struct.
- [x] `set_core_memory()` — Remove encrypt call, insert value as TEXT directly
- [x] `get_core_memory()` — Remove decrypt call, read value as TEXT
- [x] `get_all_core_memory()` — Remove decrypt filter_map, read values as TEXT. Remove `RawCoreMemoryRow` struct.
- [x] `upsert_person()` — Remove 3 encrypt calls (name, relationship, notes) and hmac_hash. Use plaintext INSERT with `canonical_name COLLATE NOCASE`.
- [x] `get_person()` — Remove hmac_hash lookup, query by `canonical_name` directly. Remove decrypt calls.
- [x] `list_people()` — Remove decrypt filter_map (~35 lines). Remove `RawPersonRow` struct. Read plaintext directly.
- [x] `add_commitment()` — Remove encrypt + hmac_hash. Use plaintext INSERT with `description COLLATE NOCASE`.
- [x] `list_commitments()` — Remove decrypt filter_map. Remove `RawCommitmentRow` struct.
- [x] `set_preference()` — Remove 2 encrypt calls + hmac_hash. Use plaintext INSERT with `category COLLATE NOCASE`.
- [x] `get_preference()` — Remove hmac_hash lookup + decrypt. Query by `category` directly.
- [x] `add_event()` — Remove encrypt call. Insert description as TEXT.
- [x] `get_memory_events()` — Remove decrypt if present. Read plaintext.
- [x] `log_memory_event()` — Remove encrypt if present.

### Phase 4: Update CLI and tools

Update callers of Database::open() and remove EncryptionKey references.

**Files:**

- [x] `crates/mika-agent/src/cli.rs`:
  - Remove `use mika_common::crypto::EncryptionKey;`
  - Remove `EncryptionKey::from_hex()` initialization (lines 41-42)
  - Change `Database::open(&db_path, key)` → `Database::open(&db_path)`
  - Update error message to remove mention of `MIKA_ENCRYPTION_KEY`

- [x] `crates/mika-agent/src/tools/store_fact.rs` — Remove `test_key()` helper, update `test_db()` to call `Database::open_in_memory()` without key
- [x] `crates/mika-agent/src/tools/search_memory.rs` — Same test helper cleanup
- [x] `crates/mika-agent/src/tools/update_core_memory.rs` — Same test helper cleanup
- [x] `crates/mika-agent/src/tools/mod.rs` — No changes expected (ToolContext only holds `&Database`)

### Phase 5: Update tests

Remove crypto tests, add case-insensitivity tests.

**File: `crates/mika-agent/src/db.rs` tests**

- [x] Remove `test_check_encryption_key`
- [x] Remove `test_people_encrypted_at_rest`
- [x] Remove `test_preferences_encrypted_at_rest`
- [x] Remove `test_data_encrypted_at_rest` (if exists)
- [x] Update `test_db()` helper — remove EncryptionKey parameter
- [x] Add `test_person_lookup_case_insensitive` — store "Sarah Chen", query "sarah chen", verify match
- [x] Add `test_preference_case_insensitive` — store category "Food", query "food", verify match
- [x] Add `test_commitment_dedup_case_insensitive` — add "Review Q4", add "review q4", verify single record

**File: `crates/mika-common/src/crypto.rs` tests**

- [x] Entire file deleted (removes `test_roundtrip`, `test_string_roundtrip`, `test_different_nonces`, `test_invalid_key_length`, `test_tampered_ciphertext`, `test_ciphertext_too_short`, `test_key_bytes_accessor`, `test_hmac_sha256_hex`)

### Phase 6: Update documentation

- [x] `CLAUDE.md` — Update encryption-related lines:
  - Line 14: Change "encrypted at field level" → remove encryption mention, add "encrypted at K8s volume level" or similar
  - Line 37-38: Update or remove encryption bullet
  - Remove `MIKA_ENCRYPTION_KEY` from Environment Variables section
  - Update tool description to remove "encrypted at rest" mentions where inaccurate

## Acceptance Criteria

- [x] `cargo build` compiles without warnings
- [x] `cargo test` passes (expect ~70 tests, down from 76 — minus crypto tests plus case-insensitive tests)
- [x] `cargo clippy` clean
- [x] `cargo fmt` clean
- [x] CLI starts without `MIKA_ENCRYPTION_KEY` env var
- [x] No references to `EncryptionKey`, `encrypt_string`, `decrypt_string`, `hmac_sha256_hex` in codebase
- [x] No `ring`, `zeroize`, `hex` in `Cargo.lock`
- [x] Case-insensitive person/preference/commitment lookups work

## Todos Resolved

| Todo | Status After |
|------|-------------|
| #038 Broken preference search | Resolved — plaintext enables LIKE/substring |
| #041 Plaintext metadata in memory_events | Resolved — all columns plaintext by design |
| #052 HMAC key reconstructed per call | Resolved — no HMAC |
| #054 Decrypt-or-skip duplication | Resolved — no decryption |
| #045 Test helper duplication | Simplified — test_key() removed, test_db() simpler |

## References

- Brainstorm: `docs/brainstorms/2026-02-24-strip-field-level-encryption-brainstorm.md`
- Prior crypto work: `docs/solutions/code-review-workflow/parallel-agent-code-review-resolution.md` (Rounds 2-3 added the encryption we're now removing)
- Encryption call sites: `crates/mika-agent/src/db.rs` (27 sites), `crates/mika-common/src/crypto.rs` (full module)
