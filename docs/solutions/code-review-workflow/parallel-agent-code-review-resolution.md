---
title: "Parallel Agent Resolution of 13 Code Review Findings"
date: 2026-02-23
category: code-review-workflow
tags:
  - code-review
  - parallel-agents
  - security
  - encryption
  - performance
  - rust
  - refactoring
  - multi-agent
  - aes-256-gcm
  - hmac
  - zeroize
  - yagni
severity: informational
component: full-stack
status: resolved
problem_type: code-review-workflow
root_cause: "13 code review findings from 7-agent Rust v2 audit required systematic resolution"
symptoms:
  - "SQL injection vector in update_commitment_status"
  - "Encryption key material lingered in memory after use"
  - "Decryption failures silently swallowed by filter_map"
  - "PII stored in plaintext SQLite columns"
  - "Agent loop rebuilt request struct on every iteration"
  - "Unused crate skeleton and dead code accumulated"
findings_summary:
  total_resolved: 12
  deferred: 1
  critical_p1: 4
  important_p2: 4
  nice_to_have_p3: 4
  parallel_rounds: 3
  agents_spawned: 9
---

# Parallel Agent Resolution of 13 Code Review Findings

## Problem Statement

A 7-agent code review of the Mika v2 Rust rewrite (commit `2fb541e`) produced 13 findings (todos #025-#037) spanning security vulnerabilities, encryption weaknesses, performance issues, and YAGNI violations. Resolving them sequentially would be slow and error-prone due to file conflicts between findings. A systematic parallel resolution strategy was needed.

## Strategy: File-Conflict-Aware Parallel Execution

### The Core Insight

Not all findings can run in parallel — agents editing the same file will conflict. The solution was to build a **file-conflict matrix** mapping each finding to the files it touches, then group findings into non-conflicting rounds.

### File-Conflict Matrix

| Finding | Files Touched |
|---------|--------------|
| #025 SQL injection | db.rs |
| #026 Secrets in memory | crypto.rs, config.rs, Cargo.tomls |
| #028 search_memory ref | prompt.rs |
| #029 Silent decryption | db.rs |
| #030 Typed API errors | claude.rs |
| #031 Agent loop perf | agent.rs |
| #032 AES key cache | crypto.rs, Cargo.tomls |
| #033 Plaintext PII | db.rs |
| #034 YAGNI cleanup | many files |
| #035 Tool input validation | tools/*.rs |
| #036 Gitignore + env | .gitignore, .env.example |
| #037 Crypto improvements | crypto.rs, Cargo.tomls |

### Conflict Groups

- **db.rs**: #025, #029, #033 (cannot run together)
- **crypto.rs + Cargo.tomls**: #026, #032, #037 (cannot run together)
- **All files**: #034 YAGNI cleanup (must run last)

### Resolution: 3 Rounds + 1 Deferral

```
Round 1 (6 agents in parallel — no file conflicts):
  #025 (db.rs)  #028 (prompt.rs)  #030 (claude.rs)
  #031 (agent.rs)  #035 (tools/*.rs)  #036 (.gitignore)

Round 2 (2 combined agents in parallel — different file groups):
  #026+#032+#037 combined → crypto.rs, config.rs, Cargo.tomls
  #029+#033 combined     → db.rs

Round 3 (1 agent — touches many files):
  #034 YAGNI cleanup

Deferred:
  #027 async SQLite — not needed until Phase 2 HTTP server
```

## Results

### Round 1: 6 Parallel Agents

All completed successfully, 27 tests pass, clippy clean.

**#025 — SQL Injection Fix** (`db.rs`)
- `update_commitment_status` now uses a status allowlist (`["pending", "done", "cancelled"]`) validated before a static SQL query
- Eliminated string interpolation in SQL entirely

**#028 — Remove search_memory Reference** (`prompt.rs`)
- Deleted one line referencing a tool that was never implemented in Rust v2

**#030 — Typed Claude API Errors** (`claude.rs`)
- Added `ClaudeApiError` enum: `HttpError { status: u16 }`, `Transport(reqwest::Error)`, `ParseError(reqwest::Error)`
- `send_once()` logs error body at WARN but returns only status code (no secrets in error chain)
- `is_retryable()` matches on enum variant (429/500/529 status, transport timeouts)
- API key validation moved before retry loop with opaque error message

**#031 — Agent Loop Performance** (`agent.rs`)
- `MessagesRequest` built once before loop, cloned only when calling `send_message`
- Added 5-minute total timeout via `tokio::time::timeout` (`AGENT_TOTAL_TIMEOUT_SECS = 300`)
- Refactored into `run_agent` (public, timeout wrapper) + `run_agent_inner` (private, loop)

**#035 — Tool Input Validation** (`tools/*.rs`)
- Added `pub const MAX_INPUT_LEN: usize = 10_000` in `tools/mod.rs`
- All 4 tools validate all string inputs against 10K character limit before processing

**#036 — Gitignore + .env.example**
- `.gitignore`: Added `config/local.*` to prevent local config from being committed
- `.env.example`: Created with `MIKA_ANTHROPIC_API_KEY`, `MIKA_ENCRYPTION_KEY`, optional overrides

### Round 2: 2 Combined Agents

All completed, 32 tests pass (5 new), clippy clean.

**#026+#032+#037 — Crypto Overhaul** (`crypto.rs`, `config.rs`, `Cargo.tomls`)
- `EncryptionKey`: Added `cached_key: LessSafeKey` field for AES key caching (no re-derivation per operation)
- Added `#[derive(ZeroizeOnDrop)]`, removed `Clone` to prevent key duplication
- Added `key_bytes()` accessor for HMAC operations
- Replaced hand-rolled `mod hex` with `hex` crate
- Added `pub fn hmac_sha256_hex(key_bytes: &[u8; 32], input: &str) -> String`
- `config.rs`: Removed `Debug` derive, added manual `impl Debug` that redacts secrets
- Removed `openai_api_key` field (unused)
- New tests: `test_hmac_sha256_hex`, `test_key_bytes_accessor`

**#029+#033 — Decryption Logging + PII Encryption** (`db.rs`)
- All `filter_map(|r| r.ok())` replaced with `warn!` logging on decryption failures
- Added `check_encryption_key()` startup validation
- `people.canonical_name` → `canonical_name_encrypted` + `canonical_name_hash` (HMAC-SHA256)
- Same pattern for `relationship` field and preferences `category`
- Added `hmac_hash()` helper method
- New tests: `test_people_encrypted_at_rest`, `test_preferences_encrypted_at_rest`, `test_check_encryption_key`

### Round 3: 1 Agent

Completed, 32 tests pass, clippy clean.

**#034 — YAGNI Cleanup**
- Deleted `crates/mika-routing/` (empty crate skeleton with 12 unused dependencies)
- Deleted `crates/mika-common/src/types.rs` (unused `InboundMessage`, `OutboundMessage`, `TypingRequest`)
- Simplified `ToolContext` to just `pub db: &'a Database` (removed `customer_id`, `routing_url`)
- Removed `schedules` table from migration
- Cleaned workspace `Cargo.toml`: removed axum, tower-http, sqlx, mika-routing

### Deferred: #027 — Async SQLite

Correctly deferred. The CLI is single-threaded; sync `rusqlite` only blocks when an async HTTP server (Phase 2) is added. Premature wrapping adds complexity without benefit.

## Test Growth

| Phase | Tests |
|-------|-------|
| Before resolution | 14 |
| After Round 1 | 27 |
| After Round 2 | 32 |
| After Round 3 | 32 |

5 new tests added:
1. `test_check_encryption_key` — validates key check on startup
2. `test_people_encrypted_at_rest` — verifies PII columns are encrypted in SQLite
3. `test_preferences_encrypted_at_rest` — verifies preference category is encrypted
4. `test_hmac_sha256_hex` — HMAC function correctness
5. `test_key_bytes_accessor` — key bytes accessor returns correct value

## Prevention Strategies

### Coding Standards

1. **No string interpolation in SQL** — Use parameterized queries exclusively. If dynamic column selection is needed, use allowlists with static SQL patterns.
2. **All PII columns encrypted** — Use the `encrypt`/`decrypt` + `hmac_hash` pattern for any column containing personal data. Plaintext columns are only for non-sensitive metadata.
3. **Secret types derive `ZeroizeOnDrop`** — Any struct holding key material must zeroize on drop and must not implement `Clone`.
4. **Typed errors at API boundaries** — Return enum variants with status codes, not string messages. Log details at WARN, return opaque errors to callers.
5. **Tool inputs validated** — All string inputs to agent tools must be length-checked before processing.

### Review Checklist for Future PRs

- [ ] No `format!()` or string concatenation in SQL queries
- [ ] New PII columns use encrypted + HMAC hash pattern
- [ ] Secret-holding types derive ZeroizeOnDrop, don't derive Clone
- [ ] Error types use enums, not string matching
- [ ] Tool inputs length-validated against MAX_INPUT_LEN
- [ ] No unused dependencies in Cargo.toml
- [ ] Manual Debug impl for any struct containing secrets

### Parallel Resolution Decision Matrix

When resolving multiple code review findings, use this matrix to decide the execution strategy:

| Condition | Strategy |
|-----------|----------|
| Findings touch different files | Run in parallel |
| Findings touch same file but different sections | Combine into one agent |
| Finding touches many files (cleanup/refactor) | Run last, alone |
| Finding requires architectural change not yet needed | Defer with documented rationale |

## Related Documentation

- **Original Code Review**: `docs/solutions/code-review/multi-agent-mvp-code-review.md` (Python v1 — 24 findings)
- **Rust Rewrite Plan**: `docs/plans/2026-02-22-mika-v2-rust-rewrite-plan.md`
- **Architecture Brainstorm**: `docs/brainstorms/2026-02-22-mika-v2-rust-rewrite-brainstorm.md`
- **Learnings from Rewrite**: `docs/learnings-for-rust-rewrite.md`
- **Todo Files**: `todos/025-complete-p1-*.md` through `todos/037-complete-p3-*.md` (027 still pending)

## Lessons Learned

1. **File-conflict analysis is essential for parallel resolution.** Without it, agents would produce conflicting edits requiring manual merge. The 5-minute matrix analysis saved hours of conflict resolution.

2. **Combining related findings into single agents is more effective than splitting.** The crypto overhaul (#026+#032+#037) produced a coherent result because one agent could reason about the full encryption key lifecycle — creation, caching, zeroization, and HMAC derivation — as a unified design.

3. **YAGNI cleanup should always run last.** It touches many files and can conflict with any other change. Running it after all targeted fixes ensures it cleans up whatever remains.

4. **Deferral is a valid resolution.** #027 (async SQLite) was correctly deferred because the architectural change it requires (wrapping sync rusqlite in `spawn_blocking`) has no benefit until the HTTP server is added. Premature async wrapping would add complexity to a sync CLI.

5. **Test count is a useful progress metric.** Growing from 14 to 32 tests across 3 rounds confirmed that each resolution added verification, not just code changes.
