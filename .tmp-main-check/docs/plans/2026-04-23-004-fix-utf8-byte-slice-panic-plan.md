---
title: "fix: Prevent UTF-8 byte-slice panics in KG resolver/extractor and 7 other sites"
type: fix
status: active
date: 2026-04-23
---

# fix: Prevent UTF-8 byte-slice panics in KG resolver/extractor and 7 other sites

## Overview

Fix 10 unsafe `&str` byte-slicing patterns that panic on multi-byte UTF-8 characters. Add a shared `safe_truncate` helper in `mika-common`, wrap KG background spawns with JoinHandle panic containment, and add a CI lint to prevent regression.

## Problem Frame

KG resolution is fully broken in production after the #757 deploy. `entity_resolver.rs` panics at byte boundaries inside multi-byte chars (em-dashes `U+2014`, arrows `U+2192`, box-drawing `U+2501`). 27 panic events today across all agents. The panics are invisible to tracing because `tokio::spawn` JoinHandles are discarded. The pattern `&s[..s.len().min(N)]` and `&s[..N]` exists in 10 sites across 7 files. Schema v26 (#757) triggered mass re-extraction, amplifying the latent bug. See GitHub issue #764.

## Requirements Trace

- R1. Single shared `safe_truncate` helper — no duplicate truncation logic
- R2. All 10 unsafe byte-slice sites replaced with safe alternative
- R3. KG spawn-site panic containment — panics logged, not silently swallowed
- R4. Unit tests for helper and panic observability
- R5. CI grep gate to prevent regression

## Scope Boundaries

- Global tracing-aware panic hook for mika-spirit — separate ticket
- Semantic LLM prompt truncation (sentence/paragraph boundary, token-aware) — separate ticket
- Schema-migration-as-integration-event retrospective — separate ticket

## Context & Research

### Relevant Code and Patterns

- `db::truncate_chars(s, max_chars)` — existing char-count-based truncation in `crates/mika-agent/src/db.rs:7597`. Returns `String`, appends "...". Uses `chars().take()`. **Not suitable** for the byte-budget use case: the 10 sites use byte limits for log line widths and prompt size budgets, not char counts.
- `rewind::truncate_content(s, max_len)` — private helper in `rewind.rs:728`. Identical to `truncate_chars` but private. Also char-count-based.
- `str::floor_char_boundary(n)` — stable since Rust 1.80. Returns the largest byte index `<= n` that is a char boundary. Exactly what we need for byte-budget truncation.

### Institutional Learnings

- `docs/solutions/runtime-errors/utf8-byte-slicing-panic-in-dashboard-dto.md` — Prior incident with the same bug class. Recommends `floor_char_boundary`. The dashboard DTO was fixed but the broader audit was not completed.
- `docs/solutions/best-practices/first-boot-cost-spike-after-tracking-table-migration-2026-04-23.md` — Documents that KG spawns have no stored JoinHandles. Panic in a spawned task loses the `kg_resolutions_log` marker, causing repeated LLM spending.

## Key Technical Decisions

- **Helper location: `mika-common`** — One of the 10 sites is in `crates/mika-common/src/embedding.rs`. Placing the helper in `mika-common` avoids a circular dependency and lets both crates share it. Location: new module `crates/mika-common/src/text.rs`.
- **Byte-budget semantics preserved** — The issue specifies these constants were chosen for byte-level constraints (log widths, prompt budgets). Using `safe_truncate(s, N)` with `floor_char_boundary` preserves byte semantics (returns `&str` up to N bytes, rounding down to char boundary). No conversion to char counts.
- **Returns `&str`, not `String`** — Unlike `truncate_chars`/`truncate_content`, the new helper returns a `&str` slice. Callers that need "..." appended do so at the call site. This avoids allocations in the common case (logging, format strings).
- **Spot-check Pattern B sites** — All 3 Pattern B sites (`dashboard.rs:385`, `get_session_messages.rs:102`, `embedding.rs:177`) use the truncation for log/error output with byte-level budgets. `safe_truncate` with byte semantics is correct for all 3.
- **Panic containment via outer spawn wrapper** — Wrap the existing `tokio::spawn` with an outer spawn that `.await`s the JoinHandle and logs panics. This is the pattern from the issue's acceptance criteria. The inner task logic stays unchanged.

## Implementation Units

- [x] **Unit 1: Add `safe_truncate` helper in mika-common**

**Goal:** Provide a single shared UTF-8-safe byte-budget truncation function.

**Requirements:** R1

**Dependencies:** None

**Files:**
- Create: `crates/mika-common/src/text.rs`
- Modify: `crates/mika-common/src/lib.rs` (add `pub mod text;`)
- Test: `crates/mika-common/src/text.rs` (inline `#[cfg(test)] mod tests`)

**Approach:**
- Add `pub fn safe_truncate(s: &str, max_bytes: usize) -> &str` using `s.floor_char_boundary(s.len().min(max_bytes))`
- Single function, returns `&str`, never panics

**Patterns to follow:**
- `crates/mika-agent/src/timestamp.rs` — small utility module with inline tests

**Test scenarios:**
- Happy path: ASCII string shorter than limit returns unchanged
- Happy path: ASCII string longer than limit truncated at exact byte
- Edge case: 3-byte em-dash `\u{2014}` at byte boundary — `safe_truncate("abc\u{2014}def", 5)` returns `"abc"` (byte 4 is inside the em-dash, rounds down to 3)
- Edge case: `safe_truncate("abc\u{2014}def", 6)` returns `"abc\u{2014}"` (byte 6 is the end of the em-dash, valid boundary)
- Edge case: empty string returns empty
- Edge case: max_bytes = 0 returns empty
- Edge case: max_bytes exceeds string length returns full string
- Edge case: string of only multi-byte chars (e.g., `"\u{2014}\u{2192}\u{2501}"`) with limit inside each char
- Edge case: mixed ASCII and 4-byte emoji at boundary

**Verification:**
- `cargo test -p mika-common` passes
- No panics on any multi-byte input

- [x] **Unit 2: Fix all unsafe byte-slice sites (10 sites from issue + 4 additional)**

**Goal:** Replace all `[..s.len().min(N)]` and `[..N]` patterns on `&str` with `safe_truncate(s, N)`. Also fix the unsafe `truncate()` helper in `list_audit_events.rs` and eval test patterns.

**Requirements:** R2

**Dependencies:** Unit 1

**Files:**
- Modify: `crates/mika-agent/src/kg/entity_resolver.rs` (3 sites: lines ~1029, ~1156, ~1194)
- Modify: `crates/mika-agent/src/kg/subject_extractor.rs` (2 sites: lines ~912, ~965)
- Modify: `crates/mika-agent/src/rewind.rs` (1 site: line ~713)
- Modify: `crates/mika-agent/src/skills/context.rs` (1 site: line ~265)
- Modify: `crates/mika-agent/src/server/dashboard.rs` (1 site: line ~385: `&content[..1000]`)
- Modify: `crates/mika-agent/src/tools/get_session_messages.rs` (1 site: line ~102: `&msg.content[..500]`)
- Modify: `crates/mika-agent/src/tools/list_audit_events.rs` (1 site: line ~107: `&s[..max_len]` in unsafe `truncate()` helper)
- Modify: `crates/mika-common/src/embedding.rs` (1 site: line ~177: `&body[..500]`)
- Modify: `crates/mika-agent/tests/eval/assertions.rs` (1 site: line ~246)
- Modify: `crates/mika-agent/tests/eval/test_task_not_found_retry.rs` (2 sites: lines ~104, ~170)

**Approach:**
- Add `use mika_common::text::safe_truncate;` to each mika-agent file
- For `embedding.rs` (in mika-common): `use crate::text::safe_truncate;`
- Replace `&s[..s.len().min(N)]` with `safe_truncate(s, N)` preserving the byte limit constant
- Replace `&s[..N]` (after length check) with `safe_truncate(s, N)`
- Fix the `list_audit_events::truncate()` helper to use `safe_truncate` internally
- The return type `&str` works directly in `format!` and `push_str` contexts
- For eval test sites: fix for consistency even though test data is often ASCII

**Patterns to follow:**
- Existing `use mika_common::*` imports in each file
- Existing `floor_char_boundary` usage in `get_team_status.rs`, `teams/engine.rs`

**Test scenarios:**
- Test expectation: none for this unit — the helper is tested in Unit 1, and the call sites are log/error messages

**Verification:**
- `cargo build` (workspace) compiles
- `cargo clippy` passes
- `grep` for the old patterns in fixed files returns zero matches

- [x] **Unit 4: Spawn-site panic containment for KG background tasks**

**Goal:** Wrap KG extraction and resolution `tokio::spawn` calls so panics are logged via tracing, not silently swallowed.

**Requirements:** R3

**Dependencies:** None (independent of Units 1-3)

**Files:**
- Modify: `crates/mika-agent/src/server/mod.rs` (extraction spawn ~line 849, resolution spawn ~line 934)

**Approach:**
- For each spawn site: wrap the inner task in a named `let handle = tokio::spawn(...)`, then add an outer `tokio::spawn` that awaits the handle and matches on `JoinError::is_panic()` / `is_cancelled()`
- Use `downcast_ref::<&str>` and `downcast_ref::<String>` to extract panic message, falling back to `"<non-string panic>"`
- Emit `error!` with `event = "resolution_panicked"` / `event = "extraction_panicked"` and the agent ID
- Emit `debug!` for cancellation, `warn!` for unknown join errors
- Preserve existing `Ok(Ok(...))` and `Ok(Err(...))` handling logic (move it into the outer spawn's match arms)

**Patterns to follow:**
- Existing tracing event naming convention: `event = "entity_resolution_ready"`, etc.

**Test scenarios:**
- Integration: a helper function that wraps a spawned future with panic containment, tested by spawning a task that panics with a string message → assert the error is captured (not silently lost). Use `tracing_test::traced_test` or a `tracing_subscriber::fmt::TestWriter` to verify the log event fires with the expected fields.
- Edge case: panic with non-string payload (`Box<dyn Any>`) → verify fallback message `"<non-string panic>"`
- Edge case: task returns `Err(anyhow)` → verify `warn!` fires (not the panic path)

**Verification:**
- `cargo test -p mika-agent` passes
- Panic in a spawned KG task produces a visible tracing event

- [x] **Unit 5: CI grep gate script**

**Goal:** Prevent unsafe byte-slice patterns from reappearing in `crates/`.

**Requirements:** R5

**Dependencies:** Units 2, 3 (patterns must be fixed before the gate passes)

**Files:**
- Create: `scripts/check-byte-slices.sh`
- Modify: `.github/workflows/ci.yml` (add new job)

**Approach:**
- Script greps `crates/` for two pattern families:
  - Pattern A: `\[\.\..*\.len\(\)\.min\(` on `&str` values
  - Pattern B: `\[\.\.[0-9]+\]` on known string-type variables (`&content`, `&body`, `.content`, `.body`, etc.)
- Lines containing `// safe-byte-slice:` are excluded
- Exit 1 with actionable error message referencing issue #764
- New CI job `byte-slice-lint` runs in the `check` job or as a separate lightweight job (no Rust toolchain needed, just `grep`)

**Patterns to follow:**
- `docs-sync` job structure in `ci.yml` — lightweight checkout + script run
- Existing script naming in `scripts/`

**Test scenarios:**
- Test expectation: none — the script is verified by running it and confirming zero matches after Units 2-3

**Verification:**
- `bash scripts/check-byte-slices.sh` exits 0 after all fixes applied
- CI workflow includes the new job

## System-Wide Impact

- **Interaction graph:** The `safe_truncate` helper is called from log/error/format paths only — no behavioral change to agent loop, HTTP responses, or DB writes.
- **Error propagation:** Spawn-site panic containment converts silent task poisoning into visible tracing events. No change to error propagation semantics — panics were already non-recoverable for the spawned task.
- **State lifecycle risks:** The primary risk was already manifesting: panics in resolver spawns lose `kg_resolutions_log` markers, causing repeated LLM spending on the same entity. This fix eliminates the panic source. The containment wrapper provides observability but does not recover state — that is the correct design (the entity will retry on next restart, but without panicking).
- **API surface parity:** No API changes. The helper is internal.
- **Unchanged invariants:** `truncate_chars` and `truncate_content` are not modified — they serve different use cases (char-count truncation with "..." suffix). The new `safe_truncate` serves byte-budget truncation without suffix.

## Risks & Dependencies

| Risk | Mitigation |
|------|------------|
| CI grep gate false positives on legitimate `[..N]` byte patterns (e.g., `&[u8]` slices) | Allowlist comment `// safe-byte-slice: <reason>`. Issue #764 already identified the safe `&[u8]` sites to exclude. |
| `floor_char_boundary` MSRV | Stable since Rust 1.80; workspace MSRV is 1.91. No risk. |
| Spawn containment adds overhead | Negligible — one extra `tokio::spawn` + `JoinHandle::await` per KG agent per startup. |

## Sources & References

- Related issue: #764
- Related PRs: #759 (schema v26 deploy that triggered the bug)
- Prior solution: `docs/solutions/runtime-errors/utf8-byte-slicing-panic-in-dashboard-dto.md`
- Prior solution: `docs/solutions/best-practices/first-boot-cost-spike-after-tracking-table-migration-2026-04-23.md`
