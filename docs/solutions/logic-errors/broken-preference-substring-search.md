---
title: "Fix Broken Preference Substring Search in search_memory Tool"
date: 2026-02-24
category: logic-errors
tags:
  - rust
  - sqlite
  - tools
  - search
  - agent-native
modules:
  - crates/mika-agent/src/db.rs
  - crates/mika-agent/src/tools/search_memory.rs
  - crates/mika-common/src/home.rs
severity: high
resolution_type: fix
commits:
  - 9e8a800
branch: refactor/strip-field-level-encryption
related_todos:
  - "038"
  - "056"
---

# Fix Broken Preference Substring Search

## Problem Symptom

The `search_memory` tool claimed to support "case-insensitive substring matching" across all categories (people, commitments, preferences, events, core memory), but preference search only performed an exact-key lookup. If a user stored preference `Food: "No shellfish, prefers sushi"` and the agent searched for "shellfish", it found nothing. The agent could only find a preference by guessing the exact category key name.

Additionally, the `DEFAULT_CONFIG` constant in `home.rs` still referenced `MIKA_ENCRYPTION_KEY` — an environment variable that no longer existed after the encryption strip refactor. New users bootstrapping `~/.mika/` would see instructions to set a nonexistent key.

## Root Cause

**Preference search (#038):** The `search_memory` tool's preference branch called `ctx.db.get_preference(query)`, which treated the user's search string as an exact category key for a SQL `WHERE category = ?1` lookup. There was no `list_preferences()` method in the Database to enable a full scan with substring matching. A code comment at the call site acknowledged this: *"We need to scan all preferences — no list method exists yet."*

This was originally caused by the HMAC-SHA256 encryption model where full-scan with substring matching was impossible on hashed columns. When encryption was stripped (commit `eb03ea7`), the database was updated to plaintext but the tool code was not — the commit claimed todo #038 was resolved when it wasn't.

**Stale config (#056):** The `home.rs` file was not in the encryption-strip plan's file list. The `DEFAULT_CONFIG` constant contains a string literal referencing `MIKA_ENCRYPTION_KEY`, which standard Rust refactoring tools (IDE rename, `cargo fix`) do not detect because it's inside a raw string.

## Working Solution

### Fix 1: Add `list_preferences()` to Database

```rust
// crates/mika-agent/src/db.rs
pub fn list_preferences(&self) -> Result<Vec<(String, String)>> {
    let mut stmt = self
        .conn
        .prepare("SELECT category, value FROM preferences ORDER BY category")?;
    let prefs = stmt
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?
        .filter_map(|r| match r {
            Ok(row) => Some(row),
            Err(e) => {
                tracing::warn!(error = %e, "failed to read preference row");
                None
            }
        })
        .collect();
    Ok(prefs)
}
```

### Fix 2: Update search_memory to use substring matching

```rust
// crates/mika-agent/src/tools/search_memory.rs
// Before:
if let Some(value) = ctx.db.get_preference(query)? {
    results.push(format!("[preference] {query}: {value}"));
}

// After:
let prefs = ctx.db.list_preferences()?;
for (cat, val) in prefs {
    let searchable = format!("{cat} {val}");
    if searchable.to_lowercase().contains(&query_lower) {
        results.push(format!("[preference] {cat}: {val}"));
    }
}
```

### Fix 3: Remove stale env var reference

```rust
// crates/mika-common/src/home.rs DEFAULT_CONFIG
// Removed this line:
//   MIKA_ENCRYPTION_KEY    — 64 hex chars (32 bytes) for AES-256-GCM
```

## Verification

```bash
cargo test   # 62 tests passing (1 new preference search test)
cargo clippy # clean
cargo fmt    # clean
```

Test added: `test_search_finds_preference_by_value_substring` — verifies both value-substring search ("shellfish" finds Food preference) and partial-category search ("meeting" finds "Meeting time" preference).

## Prevention Strategies

### 1. Verify "resolves todo #X" claims with a behavioral test

The encryption-strip commit claimed #038 was resolved but no test was added to verify the specific broken behavior. Rule: before claiming a todo is resolved, write a test that would have failed before the fix and passes after.

### 2. Grep for env var names inside string literals

Standard refactoring tools miss references inside raw strings (`r#"..."#`). After removing any environment variable, run:

```bash
grep -r "MIKA_REMOVED_VAR" --include="*.rs" --include="*.toml" --include="*.md" .
```

This catches `DEFAULT_CONFIG` constants, README references, and documentation that embed env var names as prose.

### 3. Check tool description accuracy against implementation

When fixing a tool, re-read its `description` field and verify every claim is true. The `search_memory` description claimed substring matching across all categories — verifying this claim would have caught the broken preference path immediately.

## References

- Parent refactor: `docs/solutions/refactoring/strip-field-level-encryption-refactor.md`
- Original finding: `todos/038-complete-p1-broken-preference-search.md`
- Stale config finding: `todos/056-complete-p1-stale-encryption-key-home-rs.md`
- Follow-up: `todos/062-pending-p2-preference-struct-consistency.md` (return named struct instead of tuple)
