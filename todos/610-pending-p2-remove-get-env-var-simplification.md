---
status: complete
priority: p2
issue_id: 610
tags: [code-review, simplification, quality]
dependencies: []
---

# Remove get_env_var — redundant after load_dotenv

## Problem Statement

`dotenv::get_env_var()` exists solely so `secret_is_set()` in setup.rs can check `.env` without loading vars into the process. But `load_dotenv()` is already called before `mika setup` runs (in `main.rs:74-76`), so all `.env` values are already in the process environment. `std::env::var(key)` covers all cases.

The only edge case — a previous prompt in the same `run()` call wrote a new key to `.env` — does not apply because `secret_is_set` is only called once per key, always before writing that key.

Removing `get_env_var` also eliminates the parser divergence concern (todo 600) and simplifies the `secret_is_set` signature by dropping the `home_dir` parameter.

## Findings

- **Source:** code-simplicity-reviewer (primary), architecture-strategist, pattern-recognition
- **Location:** `crates/mika-common/src/dotenv.rs:23-31` (function), `crates/mika-cli/src/commands/setup.rs:338-341` (sole caller)
- **Evidence:** After `load_dotenv` at `main.rs:75`, `std::env::var` returns the same values. The simplicity reviewer traced all code paths and confirmed the timing is correct.
- **Impact:** ~40 lines removed (function + 2 tests), simpler API surface, eliminates parser divergence class of bugs

## Proposed Solutions

### Option A: Remove get_env_var entirely (Recommended)
1. Delete `get_env_var` from `dotenv.rs` (lines 23-31)
2. Delete its 2 tests (lines 185-213)
3. Simplify `secret_is_set` in `setup.rs`:
```rust
fn secret_is_set(key: &str) -> bool {
    std::env::var(key).ok().filter(|v| !v.is_empty()).is_some()
}
```
4. Update all `secret_is_set` call sites to drop `home_dir` argument
- Effort: Small
- Risk: Low — `load_dotenv` is called before setup in all code paths
- Pro: ~40 lines removed, eliminates parser divergence, cleaner API

### Option B: Keep but mark as cfg(test) only
If any future use case emerges, keep the function but restrict visibility.
- Effort: Small
- Risk: Low
- Con: YAGNI — reintroduce if needed later

## Acceptance Criteria

- [ ] `get_env_var` removed from public API
- [ ] `secret_is_set` simplified to env-var-only check
- [ ] All setup tests still pass
- [ ] `mika setup` on already-initialized system still detects existing secrets
