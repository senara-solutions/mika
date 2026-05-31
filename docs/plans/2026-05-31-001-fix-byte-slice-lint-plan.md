# Plan: Fix unsafe byte-slice lint on main (mika#1356)

## Problem

PR #1354 (mika#1183) introduced an unsafe byte-slice pattern at `crates/mika-agent/tests/eval/grounding_regressions/milestone_close.rs:285`:

```rust
&correction_text[..correction_text.len().min(500)]
```

This triggers CI's `byte-slice-lint` job (`scripts/check-byte-slices.sh` Pattern A), which detects `[..var.len().min(N)]` patterns on `&str` that can panic on multi-byte UTF-8 characters.

## Fix

Replace the unsafe byte-slice with `mika_common::text::safe_truncate()`, which uses `floor_char_boundary` for UTF-8-safe truncation.

### Step 1: Replace the byte-slice pattern

**File:** `crates/mika-agent/tests/eval/grounding_regressions/milestone_close.rs:285`

**Before:**
```rust
&correction_text[..correction_text.len().min(500)]
```

**After:**
```rust
mika_common::text::safe_truncate(&correction_text, 500)
```

### Step 2: Add the import

Add `use mika_common::text::safe_truncate;` to the file's imports section (or use the fully-qualified path inline).

### Step 3: Verify

- Run `scripts/check-byte-slices.sh` locally to confirm no violations.
- Run `cargo test -p mika-agent --test eval -- milestone_close` to confirm the test still compiles and passes.

## Scope

Single-file, single-line fix. No behavioral change — `safe_truncate` produces the same truncation for ASCII text and safe truncation for multi-byte text.

## Acceptance Criteria

- [x] `scripts/check-byte-slices.sh` passes locally
- [x] CI green on main after merge
