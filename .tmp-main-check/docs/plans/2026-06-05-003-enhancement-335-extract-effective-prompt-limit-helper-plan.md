# Plan: Extract effective_prompt_limit helper to reduce duplication

**Ticket:** mika issue#335
**Type:** enhancement (refactor)
**Risk:** low — pure mechanical extraction, no behavioral change

## Problem

The effective prompt limit calculation is duplicated 6 times across `crates/mika-agent/src/skills/index.rs` (5 sites) and `crates/mika-agent/src/skills/variants.rs` (1 site):

```rust
.map(|v| v.min(MAX_PROMPT_SIZE_CEILING))
.unwrap_or(MAX_PROMPT_SNIPPET_SIZE)
```

Additionally, the integration test at `crates/mika-agent/tests/bundled_skills_load.rs` reimplements the same logic inline.

## Solution

Extract a `pub(super)` free function in `index.rs` that encapsulates the calculation, then replace all 6 internal call sites + the test's inline reimplementation.

## Implementation Steps

### Step 1 — Add the helper function in `index.rs`

Add near the constant definitions (around line 28, after `MAX_PROMPT_SIZE_CEILING`):

```rust
/// Effective prompt-size limit for a skill.
///
/// Returns the minimum of the manifest's `max_prompt_size` and the hard
/// ceiling ([`MAX_PROMPT_SIZE_CEILING`]), falling back to the default
/// snippet size ([`MAX_PROMPT_SNIPPET_SIZE`]) when no override is declared.
pub fn effective_prompt_limit(max_prompt_size: Option<u64>) -> u64 {
    max_prompt_size
        .map(|v| v.min(MAX_PROMPT_SIZE_CEILING))
        .unwrap_or(MAX_PROMPT_SNIPPET_SIZE)
}
```

Visibility: `pub` (not `pub(super)`) — the integration test in `tests/bundled_skills_load.rs` needs access, and both constants are already `pub`.

### Step 2 — Replace 5 call sites in `index.rs`

Replace each occurrence of the duplicated pattern with a call to `effective_prompt_limit(entry.manifest.skill.max_prompt_size)` (or the equivalent local binding). The 5 sites are:

1. **`load_skill_entry()`** (~line 576) — prompt snippet truncation
2. **`validate_skill()`** (~line 1129) — base prompt size diagnostic
3. **`validate_skill()`** (~line 1250) — variant override size diagnostic
4. **`scan_variants_by_provider()`** (~line 1595) — hand-authored variant truncation
5. **`scan_generated_variants()`** (~line 1777) — auto-generated variant truncation

### Step 3 — Replace 1 call site in `variants.rs`

`validate_variant()` (~line 358) imports `MAX_PROMPT_SIZE_CEILING` and `MAX_PROMPT_SNIPPET_SIZE` from `super::index`. Replace the inline calculation with `super::index::effective_prompt_limit(...)`.

### Step 4 — Update the integration test

In `crates/mika-agent/tests/bundled_skills_load.rs` (~line 138), replace the inline reimplementation:

```rust
let effective_cap = manifest_max
    .unwrap_or(MAX_PROMPT_SNIPPET_SIZE)
    .min(MAX_PROMPT_SIZE_CEILING);
```

with:

```rust
let effective_cap = effective_prompt_limit(manifest_max);
```

Add `effective_prompt_limit` to the existing `use mika_agent::skills::index::{...}` import.

### Step 5 — Verify

- `cargo test -p mika-agent` — all existing tests pass
- `cargo test -p mika-agent --test bundled_skills_load` — integration test passes
- `cargo clippy -p mika-agent` — no warnings

## Files Changed

| File | Change |
|------|--------|
| `crates/mika-agent/src/skills/index.rs` | Add `effective_prompt_limit()`, replace 5 inline calculations |
| `crates/mika-agent/src/skills/variants.rs` | Replace 1 inline calculation with helper call |
| `crates/mika-agent/tests/bundled_skills_load.rs` | Replace inline reimplementation with helper call |

## Out of Scope

- No new tests needed — existing tests already exercise all code paths; the refactor is behavior-preserving.
- No changes to `mod.rs` — `apply_overrides()` does not contain the duplicated pattern (ticket description was slightly outdated; the duplication was fully within `index.rs` and `variants.rs`).
