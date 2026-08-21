---
module: milestone_manager
tags: [structural-invariant, lecture-seule, phase-gating, wrapper-only]
problem_type: silent-scope-drift
category: best-practices
---

# LECTURE seule structural gate for phase-gated modules

## Problem

When a module ships in a **read-only** phase (Phase 1) with a **write-authority** phase to follow (Phase 2 after gates clear), a prompt-level or docstring-level "no writes here yet" discipline erodes under review load. Someone adds a helper that shells out `gh issue edit` or `run_claude_pilot` "just for testing", the invariant silently drifts, and by the time it's noticed the module has become a de-facto Phase 2 without the gates being cleared.

The founding incident is the **mika-manager Phase 1** ratification (mika#1931, 2026-08-21) where Vincent + Prime tranched that Phase 2 (dispatch authority) requires three portes cleared first — but the module lives in the same crate as the dispatch machinery, so nothing structurally prevents a future edit from wiring `run_claude_pilot`.

## Pattern

Add a `no_dispatch_test.rs` sibling to the module that greps every `.rs` file under the module for a list of **forbidden write-authority tokens**:

```rust
const FORBIDDEN_TOKENS: &[&str] = &[
    "run_claude_pilot",
    "pr_merge_with_gate",
    "gh pr merge",
    "gh issue edit",
    "gh api -X PATCH",  // etc.
    "\"PATCH\"", "\"POST\"", "\"DELETE\"", "\"PUT\"",
];
```

The test walks the module directory, reads each `.rs` file, **strips line comments** (so docstrings can describe what's forbidden without tripping the guard), and panics with a named-token error when any forbidden token appears in executable code.

**Path resolution** must use `env!("CARGO_MANIFEST_DIR")` — `file!()` returns different values across CWD contexts (workspace root vs crate root) and breaks on IDE test runners.

**Exemption file** = the test file itself, since it contains the tokens as literals.

## Why this is stronger than prompt-level discipline

- **Compile-time-adjacent.** The test binary fails at `cargo test`, before the diff can be merged. There is no "reviewer forgot to look" failure mode.
- **Symmetric with the docstring.** Any legitimate promotion (Phase 2 gates cleared) requires updating BOTH the FORBIDDEN_TOKENS list AND the module docstring atomically. Missing one produces a compile error or a broken doctrine.
- **Grep-shape encodes intent.** Reading the FORBIDDEN_TOKENS list is itself a statement of what the module IS and IS NOT allowed to do. Downstream reviewers see the invariant.

## Companion: comment-strip discipline

The scanner MUST strip line comments before matching. Otherwise a docstring saying "no `gh pr merge` here" trips the guard. The stripper is naive (finds `//` outside string literals per line) — sufficient for the code shapes in a Phase 1 module.

## Companion: injection-verification anchor

Each composer emit protected by the LECTURE-seule invariant should ALSO have a per-emit injection-verified test (see `feedback_verify_pipeline_passes_without_the_fix`). The structural gate catches drift; the injection-verified tests catch silent-composer-removal. Both are needed.

## Reference implementation

`crates/mika-agent/src/milestone_manager/no_dispatch_test.rs` — 100 LOC, no dependencies beyond `std::fs`.

## When NOT to use this pattern

- If the module is intended to have write authority from day one, don't add the gate — it's a Phase 1 discipline, not a general one.
- If the "no writes" invariant is enforced at a higher layer (e.g., the module is a pure function crate with no I/O deps), the type system already does the work.
- If the forbidden tokens are pervasive elsewhere in the crate and cross the module boundary via `use` imports, the grep approach may false-positive. In that case, use a static-analysis-based gate (custom `clippy::disallowed_methods`).

## Escape hatch

Phase 2 promotion updates both the FORBIDDEN_TOKENS list and the module docstring in the same PR. A separate compound doc names the Phase 2 discipline that replaces the structural gate (typically a runtime permission classifier or a dispatch quorum).
