---
title: "Strip Field-Level Encryption in Favor of Kubernetes Volume-Level Protection"
date: 2026-02-24
category: refactoring
tags:
  - rust
  - sqlite
  - encryption
  - refactoring
  - kubernetes
  - database-schema
  - security
  - migration
  - code-review
modules:
  - crates/mika-common/src/crypto.rs
  - crates/mika-common/src/config.rs
  - crates/mika-agent/src/db.rs
  - crates/mika-agent/src/cli.rs
  - crates/mika-agent/src/tools/store_fact.rs
  - crates/mika-agent/src/tools/search_memory.rs
  - crates/mika-agent/src/tools/update_core_memory.rs
severity: high
resolution_type: refactor
commits:
  - eb03ea7
  - ca20ab5
branch: refactor/strip-field-level-encryption
---

# Strip Field-Level Encryption Refactor

## Problem Symptom

Mika's SQLite database layer used AES-256-GCM field-level encryption with HMAC-SHA256 hash lookups on every PII column. This created 27 encrypt/decrypt call sites, ~150 lines of duplicated decrypt-or-skip `filter_map` patterns, and 3 cryptographic crate dependencies (`ring`, `zeroize`, `hex`) — all providing zero additional security benefit given Mika's per-customer Kubernetes container isolation architecture.

The encryption actively blocked feature work:
- **Preference search broken** (todo #038): HMAC-SHA256 is exact-match only — substring, LIKE, and case-insensitive queries are impossible on hashed columns
- **FTS5 full-text search blocked** (Layer 3 roadmap): Cannot index encrypted BLOB columns
- **Silent data loss**: Failed decryptions were silently dropped from result sets via warn-and-skip `filter_map` patterns
- **Test coupling**: Every test required `test_key()` + `EncryptionKey` initialization just to open an in-memory database

## Root Cause Analysis

The field-level encryption was introduced during the v2 Rust rewrite (commit `8bbcf73`, Rounds 2-3 of the parallel code review) as a defense-in-depth measure. The threat model assumed generic deployment scenarios, but Mika's actual architecture made it redundant:

1. **Container isolation**: Each customer runs in an isolated K8s container with its own SQLite database
2. **Volume encryption**: Kubernetes encrypted volumes handle data-at-rest protection at the infrastructure layer
3. **Memory access defeats field-level crypto**: An attacker with container memory access already has the `EncryptionKey` in process memory, making field-level encryption moot
4. **No cross-tenant risk**: Per-customer containers mean no shared database to protect

The encryption layer was pure accidental complexity — it duplicated infrastructure-level protection while blocking application-level features.

## Investigation Steps

1. **Brainstorm** (`docs/brainstorms/2026-02-24-strip-field-level-encryption-brainstorm.md`): Enumerated all 27 call sites and 10 encrypted BLOB columns across 5 tables. Evaluated three options (full strip, SQLCipher, keep as-is). Chose full strip.

2. **Plan** (`docs/plans/2026-02-24-refactor-strip-field-level-encryption-plan.md`): Organized into 6 phases with per-file checklists. Identified 5 existing todos that would be resolved.

3. **Implementation**: Executed all 6 phases in a single commit (`eb03ea7`).

4. **6-agent parallel code review**: Found 11 findings (2 P1, 6 P2, 3 P3), documented in commit `ca20ab5`.

## Working Solution

### Phase 1: Delete crypto module

Deleted `crates/mika-common/src/crypto.rs` (198 lines) containing `EncryptionKey`, `CryptoError`, `encrypt()`/`decrypt()`, `hmac_sha256_hex()`, and 8 unit tests.

Removed from `mika-common/Cargo.toml`:
```toml
# Deleted:
ring.workspace = true
zeroize.workspace = true
hex.workspace = true
```

Removed `encryption_key: String` field from `Settings` struct in `config.rs`.

### Phase 2: Simplify Database struct

```rust
// Before:
pub struct Database {
    conn: Connection,
    key: EncryptionKey,
}
impl Database {
    pub fn open(path: &Path, key: EncryptionKey) -> Result<Self> { ... }
    pub fn open_in_memory(key: EncryptionKey) -> Result<Self> { ... }
}

// After:
pub struct Database {
    conn: Connection,
}
impl Database {
    pub fn open(path: &Path) -> Result<Self> { ... }
    pub fn open_in_memory() -> Result<Self> { ... }
}
```

### Phase 3: Schema migration v4 (DROP all + recreate)

Pre-production, no real user data. Fresh schema with plaintext TEXT columns and `COLLATE NOCASE` for case-insensitive unique constraints:

| Table | Old Columns | New Columns |
|-------|-------------|-------------|
| conversations | `content_encrypted BLOB` | `content TEXT NOT NULL` |
| people | `canonical_name_encrypted BLOB` + `canonical_name_hash TEXT UNIQUE` | `canonical_name TEXT NOT NULL UNIQUE COLLATE NOCASE` |
| people | `relationship_encrypted BLOB`, `notes_encrypted BLOB` | `relationship TEXT`, `notes TEXT` |
| commitments | `description_encrypted BLOB` + `description_hash TEXT UNIQUE` | `description TEXT NOT NULL UNIQUE COLLATE NOCASE` |
| preferences | `category_encrypted BLOB` + `category_hash TEXT UNIQUE`, `value_encrypted BLOB` | `category TEXT NOT NULL UNIQUE COLLATE NOCASE`, `value TEXT NOT NULL` |
| events | `description_encrypted BLOB` | `description TEXT NOT NULL` |

### Phase 4: Simplify all database methods

The core pattern change across 15+ methods:

```rust
// Before (encrypt + HMAC for writes):
pub fn upsert_person(&self, name: &str, ...) -> Result<i64> {
    let name_encrypted = self.key.encrypt_string(name)?;
    let name_hash = self.hmac_hash(name);
    self.conn.execute(
        "INSERT INTO people (canonical_name_encrypted, canonical_name_hash, ...)
         VALUES (?1, ?2, ...) ON CONFLICT(canonical_name_hash) DO UPDATE SET ...",
        rusqlite::params![name_encrypted, name_hash, ...],
    )?;
    // ...
}

// After (plaintext + COLLATE NOCASE):
pub fn upsert_person(&self, name: &str, ...) -> Result<i64> {
    self.conn.execute(
        "INSERT INTO people (canonical_name, ...) VALUES (?1, ...)
         ON CONFLICT(canonical_name) DO UPDATE SET ...",
        rusqlite::params![name, ...],
    )?;
    // ...
}
```

```rust
// Before (decrypt filter_map for reads):
.filter_map(|r| match r {
    Ok(raw) => match self.key.decrypt_string(&raw.content_encrypted) {
        Ok(content) => Some(ConversationMessage { content, ... }),
        Err(e) => { tracing::warn!("decryption failed"); None }
    },
    Err(e) => { tracing::warn!("row read failed"); None }
})

// After (direct TEXT read):
.filter_map(|r| match r {
    Ok(row) => Some(row),
    Err(e) => { tracing::warn!("row read failed"); None }
})
```

4 internal `Raw*Row` structs deleted (`RawConversationRow`, `RawCoreMemoryRow`, `RawPersonRow`, `RawCommitmentRow`).

### Phase 5: CLI and tool caller updates

```rust
// Before (cli.rs):
let key = EncryptionKey::from_hex(&settings.encryption_key)?;
let db = Database::open(&db_path, key)?;

// After:
let db = Database::open(&db_path)?;
```

Test helpers simplified across 3 tool modules: `test_key()` removed, `test_db()` reduced to `Database::open_in_memory().unwrap()`.

### Phase 6: Test updates

- Removed: 4 encryption-at-rest tests from `db.rs`, 8 crypto tests from `crypto.rs`
- Added: 3 case-insensitive tests (`test_person_lookup_case_insensitive`, `test_commitment_dedup_case_insensitive`, `test_preference_case_insensitive`)
- Net: 61 tests passing, all quality gates clean

## Verification

```bash
cargo build        # zero warnings
cargo test         # 61 tests passing
cargo clippy       # clean
cargo fmt --check  # clean

# No residual references:
grep -r "EncryptionKey\|encrypt_string\|decrypt_string\|hmac_sha256_hex" crates/  # empty
grep -E '^name = "(ring|zeroize|hex)"' Cargo.lock                                 # empty
```

## Code Review Findings

6-agent parallel review (security-sentinel, architecture-strategist, performance-oracle, code-simplicity-reviewer, agent-native-reviewer, learnings-researcher) produced:

**Resolved by the refactor (2 closed):**
- #041 — Plaintext metadata in memory_events (all columns plaintext by design)
- #052 — HMAC key reconstructed per call (HMAC deleted entirely)

**New findings (6 created):**
| # | Priority | Finding |
|---|----------|---------|
| #056 | P1 | Stale `MIKA_ENCRYPTION_KEY` in `home.rs` `DEFAULT_CONFIG` — file was not in the plan |
| #057 | P2 | `memory_events` table grows unboundedly — no retention policy |
| #058 | P2 | Stale encryption references in `README.md` — not in the plan |
| #059 | P2 | `Database::conn()` is `pub` but should be `pub(crate)` |
| #060 | P3 | Redundant `COLLATE NOCASE` in WHERE clauses |
| #061 | P3 | Unused `idx_conv_created` index |

**Existing todos updated (5):** #038, #045, #047, #050, #054

## Prevention Strategies

### 1. Refactor Completeness: Grep in Concentric Rings

Search beyond imports — the P1 miss was a string literal inside a `const`:

```bash
# Ring 1: Direct symbols
grep -r "EncryptionKey\|encrypt_field\|decrypt_field" --include="*.rs" .

# Ring 2: Config and env var references (caught the home.rs miss)
grep -r "MIKA_ENCRYPTION_KEY" --include="*.rs" --include="*.toml" --include="*.md" --include="*.example" .

# Ring 3: Schema residue
grep -r "_hash\|_encrypted\|filter_map.*decrypt" --include="*.rs" .

# Ring 4: String literals
grep -r '"encryption\|"ENCRYPTION\|64 hex\|32 bytes' --include="*.rs" .

# Ring 5: Documentation
grep -r "encrypt\|decrypt\|AES\|GCM\|HMAC" --include="*.md" .
```

### 2. Claim Verification: Test the Behavior, Not the Compilation

Before claiming "resolves todo #X":
1. Read the todo and write a falsifiable behavior statement
2. Write a failing test for that exact behavior
3. Verify the test passes after the fix
4. Grep for all callers of the fixed function
5. Close the todo with the test name as evidence

The preference search P1 (#038) was claimed resolved because plaintext enables the fix, but the code wasn't updated. A test would have caught this.

### 3. Multi-File Refactor Safety

Build an explicit dependency graph of the concern being removed before writing code. Every node in the graph becomes a checklist item. The `home.rs` `DEFAULT_CONFIG` constant would have appeared in the graph under "Settings → config string references."

### 4. Post-Refactor Verification Checklist

- [ ] Full-repo grep for every removed symbol returns zero results
- [ ] Every removed env var grepped across all file types (`.rs`, `.toml`, `.md`, `.example`)
- [ ] Every todo claimed as resolved has a named test proving the behavior
- [ ] All `filter_map` patterns re-evaluated if their original justification was removed
- [ ] All documentation files (README.md, CLAUDE.md, inline comments) updated
- [ ] Migration wrapped in explicit transaction
- [ ] `cargo build` + `cargo test` + `cargo clippy` + `cargo fmt --check` all clean

### 5. Migration Safety: Always Wrap in Transaction

```rust
fn run_migration(conn: &Connection) -> Result<()> {
    let tx = conn.transaction()?;
    tx.execute_batch("...")?;
    tx.commit()?;
    Ok(())
}
```

Without a transaction, a crash mid-migration leaves the schema inconsistent.

## Metrics

| Metric | Before | After |
|--------|--------|-------|
| Lines in db.rs | ~850 | ~670 (-180) |
| Net lines changed | — | -351 |
| Encrypt/decrypt call sites | 27 | 0 |
| Crypto dependencies | 3 (ring, zeroize, hex) | 0 |
| Raw*Row structs | 4 | 0 |
| Required env vars | 2 | 1 |
| Tests | 76 (with crypto) | 61 |
| filter_map decrypt stages | 5 | 0 (warn-and-skip shells remain) |
| Storage per people row | ~184 bytes (encrypted) | ~36 bytes (plaintext, ~5x reduction) |

## References

### Internal
- Brainstorm: `docs/brainstorms/2026-02-24-strip-field-level-encryption-brainstorm.md`
- Plan: `docs/plans/2026-02-24-refactor-strip-field-level-encryption-plan.md`
- Prior crypto work: `docs/solutions/code-review-workflow/parallel-agent-code-review-resolution.md` (Rounds 2-3 added the encryption this refactor removes)

### Related Todos
- #038 (P1) — Broken preference search (unblocked, fix pending)
- #041 (resolved) — Plaintext metadata in memory_events
- #045 (P2) — Test helper duplication (simplified, not fully resolved)
- #047 (P2) — Migration transaction wrapping
- #052 (resolved) — HMAC key caching
- #054 (P3) — Decrypt-or-skip duplication (decrypt removed, filter_map pattern remains)
- #056 (P1) — Stale MIKA_ENCRYPTION_KEY in home.rs

### Commits
- `eb03ea7` — refactor: strip field-level encryption, use plaintext SQLite on K8s encrypted volumes
- `ca20ab5` — docs: add 6 code review findings from encryption-strip review
