---
artifact_contract: ce-unified-plan/v1
artifact_readiness: implementation-ready
execution: code
product_contract_source: ce-plan-bootstrap
origin: senara-solutions/mika#1573
plan_depth: lightweight
created: 2026-06-27
---

# fix: Relax `test_bundled_skills_no_cross_skill_tool_name_collision` to AC2's spec

> **Target repo:** mika · **Issue:** senara-solutions/mika#1573 · **Branch:** `fix/1573/skills-relax-test-bundled-skills-no`

## Summary

`mika#1569` (AC2 of `mika#1326`) added `test_bundled_skills_no_cross_skill_tool_name_collision` — a build-time invariant test guarding against cross-skill tool-name collisions in bundled-skill `tools.json` declarations. The dispatched author wrote it **stricter than AC2 specified**: it flags ALL same-name declarations regardless of handler type, where AC2 named only the "different handler types" class (the production-risk class — divergent handlers silently shadowing each other via `HashMap` last-write-wins, e.g. the 2026-05-28 `run_gh` Builtin-vs-Exec incident).

That over-strict framing false-positives on four bundled skills (`gh-read-only`, `mika-arch-groom-ticket`, `mika-arch-groom-milestone`, `mika-arch-second-review`) that each legitimately declare `gh_read` with an **identical** Builtin handler — the skill-scoped tool surface model working as designed (same name + same handler = no last-write-wins risk). `mika#1569` papered over this with a `KNOWN_PRE_EXISTING_COLLISIONS` allowlist. This plan removes that allowlist and relaxes the test to AC2's actual spec.

This is a **test-only** change in `crates/mika-agent/` — no production behavior changes.

## Problem Frame

- **What's wrong:** The collision test flags benign same-handler-type declarations, requiring an allowlist to stay green. Allowlists rot; the underlying test contract is wrong relative to AC2.
- **Why it matters:** AC2 was scoped to catch the divergent-handler class (real last-write-wins shadowing). Flagging same-handler redundancy is coverage AC2 never required and produces false-positives on the intended skill-scoped tool surface model.
- **Scope boundary:** Relax the test to flag only divergent-handler-type collisions; remove the allowlist; prove the original `mika#1326` incident class still trips. No skill-loader changes (Path A is explicitly out of scope per the issue body). No AC3–AC5 of `mika#1326`.

## Requirements

- **R1** — Remove the `KNOWN_PRE_EXISTING_COLLISIONS` allowlist (and its self-cleaning + unallowlisted-filter logic). The four `gh_read` declarations are no longer flagged. *(issue AC1)*
- **R2** — `test_bundled_skills_no_cross_skill_tool_name_collision` flags **only** same-name declarations with **different handler types** (`handler.type`). Same-name + same-handler-type is not reported. *(issue AC2)*
- **R3** — A fixture-based regression test asserts the relaxed detector still catches the `mika#1326` incident class: two skills declaring `run_gh` with Builtin vs Exec handlers trips the detector. *(issue AC3)*
- **R4** — No dangling reference to the deleted `KNOWN_PRE_EXISTING_COLLISIONS` symbol remains in the codebase (doc-comment in `verify_bundled_skills.rs`).

## Key Technical Decisions

- **KTD-1 — Factor a pure, testable helper.** Extract the collision logic into `detect_divergent_handler_collisions(skills: &[&BundledSkill]) -> Vec<String>` so both the real invariant test (over `all_bundled_skills()`) and the AC3 fixture test exercise the **same** detection code path. This is what makes AC3 a genuine regression guard rather than a parallel re-implementation. Helper lives inside `#[cfg(test)] mod tests` (the scope-reservation discipline from `mika#1326` keeps collision logic out of the production loader).
- **KTD-2 — Group by `(tool_name → set of handler types)`; report when the set size > 1.** Handler type is read from `tool_def["handler"]["type"]`. A missing/unparseable `handler.type` maps to a stable sentinel string so two declarations that both lack a handler type are treated as same (not a divergence) and a missing-vs-present pairing is flagged conservatively. Malformed `tools.json` is skipped (other tests own that failure).
- **KTD-3 — No allowlist in the relaxed test.** Divergent-handler-type collisions are always bugs (never benign), so the relaxed test needs no exception mechanism. Removing `KNOWN_PRE_EXISTING_COLLISIONS` is a net deletion, not a replacement.

## Implementation Units

### U1. Relax the collision test + factor the detection helper

- **Goal:** Replace the strict same-name detection with divergent-handler-type detection, behind a reusable helper. Satisfies R1 + R2.
- **Requirements:** R1, R2
- **Dependencies:** none
- **Files:**
  - `crates/mika-agent/src/bundled_skills.rs` (modify — the `#[cfg(test)] mod tests` block, ~lines 1592–1731)
- **Approach:**
  - Delete the `KNOWN_PRE_EXISTING_COLLISIONS` const and the two blocks that consume it (the self-cleaning per-entry assertion loop and the `unallowlisted` filter).
  - Add `fn detect_divergent_handler_collisions(skills: &[&BundledSkill]) -> Vec<String>`: for each skill, find the `tools.json` file, parse it (skip on parse error), and for each tool record `tool_name → handler_type` into a `BTreeMap<String, BTreeMap<String, BTreeSet<String>>>` (tool → handler_type → declaring skills). Emit one formatted line per tool whose handler-type map has > 1 entry, naming the tool and the divergent `handler_type [skills...]` groups. Use `BTree*` for deterministic output ordering.
  - Rewrite the test body to call the helper over `all_bundled_skills()` and assert the result is empty, with a message that names the divergent collisions and explains the last-write-wins risk + the relaxation provenance (`mika#1326` / `mika#1573`).
  - Update the test's leading doc comment to state the relaxed contract (different-handler-types only) and why same-handler-type is benign.
- **Patterns to follow:** existing `serde_json::from_str::<Vec<serde_json::Value>>(tools_json.content)` parse pattern already in the test; `tool_def.get("handler").and_then(|h| h.get("type")).and_then(|t| t.as_str())` extraction (mirrors `verify_bundled_skills.rs::parse_tools`).
- **Test scenarios:**
  - `test_bundled_skills_no_cross_skill_tool_name_collision` runs over the real bundle and passes green (the four `gh_read` Builtin declarations are not flagged — verified there are zero divergent-handler collisions in the tree today).
  - *Covers R2:* the helper does not report a tool name shared across skills when every declaration has the same `handler.type`.
- **Verification:** `cargo test -p mika-agent test_bundled_skills_no_cross_skill_tool_name_collision` passes; no reference to `KNOWN_PRE_EXISTING_COLLISIONS` remains in `bundled_skills.rs`.

### U2. Add the AC3 fixture regression test

- **Goal:** Prove the relaxed detector still catches the original `mika#1326` incident class. Satisfies R3.
- **Requirements:** R3
- **Dependencies:** U1 (uses `detect_divergent_handler_collisions`)
- **Files:**
  - `crates/mika-agent/src/bundled_skills.rs` (modify — add a `#[test]` in the same tests module)
- **Approach:** Build `static` `BundledSkill` fixtures with inline `tools.json` content (mirroring the existing `MERGE_TEST_*` static-fixture pattern, since `BundledSkill` fields are `&'static`). Positive case: two skills declaring `run_gh` with `{"type":"builtin",...}` vs `{"type":"exec",...}` → assert exactly one collision reported, naming `run_gh`. Negative companion: two skills both declaring `gh_read` with identical `{"type":"builtin",...}` → assert empty (the false-positive that motivated `mika#1573`).
- **Patterns to follow:** `MERGE_TEST_LEGACY_FILES` / `MERGE_TEST_LEGACY_ALPHA` static-fixture construction already in the tests module; `r#"..."#` raw-string JSON for `tools.json` content.
- **Test scenarios:**
  - *Covers R3 (positive):* `run_gh` Builtin-vs-Exec across two fixture skills → detector returns one collision containing `"run_gh"`.
  - *Covers R2 (negative):* `gh_read` Builtin-vs-Builtin across two fixture skills → detector returns empty.
- **Verification:** the new test passes; deliberately divergent fixture trips the detector (proven by the positive assertion).

### U3. De-reference the deleted symbol in the verify-gate doc comment

- **Goal:** Keep the `make verify-bundled-skills` binary's doc comment accurate after `KNOWN_PRE_EXISTING_COLLISIONS` is deleted. Satisfies R4.
- **Requirements:** R4
- **Dependencies:** U1
- **Files:**
  - `crates/mika-agent/src/bin/verify_bundled_skills.rs` (modify — doc comment at ~line 47)
- **Approach:** The comment currently says the binary's `KNOWN_EXCEPTIONS` "Mirrors `KNOWN_PRE_EXISTING_COLLISIONS` in `bundled_skills.rs`". Rewrite to describe the self-cleaning-allowlist *pattern* it mirrors without naming the now-deleted constant (e.g., reference the self-cleaning exception discipline and note the AC2 collision test was relaxed to handler-type divergence per `mika#1573`). `verify_bundled_skills.rs` performs no collision detection itself — this is a documentation-only touch; its five structural checks are unchanged.
- **Test scenarios:** `Test expectation: none -- doc-comment-only change, no behavior.`
- **Verification:** `grep -rn KNOWN_PRE_EXISTING_COLLISIONS crates/` returns no matches; `cargo build -p mika-agent --bin verify-bundled-skills` succeeds; `make verify-bundled-skills` still passes green.

## Verification Contract

- `cargo test -p mika-agent bundled_skills` — both the relaxed invariant test and the new AC3 fixture test pass.
- `cargo build -p mika-agent` and `cargo clippy -p mika-agent` — clean.
- `grep -rn KNOWN_PRE_EXISTING_COLLISIONS crates/` — zero matches (R1 + R4 complete).
- `make verify-bundled-skills` — green (unaffected; confirms no collateral).

## Definition of Done

All three issue ACs satisfied: allowlist removed (AC1/R1), test flags only divergent handler types (AC2/R2), fixture regression test guards the `run_gh` Builtin-vs-Exec class (AC3/R3); no dangling symbol reference (R4); crate builds + lints clean; PR opened with `Closes #1573`.

## Scope Boundaries

**In scope:** Relaxing the collision test, removing the allowlist, the AC3 fixture test, the verify-gate doc-comment touch.

### Deferred / Out of scope (per issue body)

- **Path A** — consolidating the `gh_read` declarations behind a shared-declaration loader mechanism. Rejected in grooming as architectural complexity to "fix" a non-bug.
- **AC3–AC5 of `mika#1326`** — registry-drift detection, mtime-based reload trigger, recovery-without-restart. Reserved in `mika#1326`'s scope.
