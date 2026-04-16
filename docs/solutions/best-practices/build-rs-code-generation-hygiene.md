---
title: "build.rs code generation hygiene — escape with {:?}, avoid name-derived identifiers, test the real merge"
category: best-practices
problem_type: best_practice
tags: [build.rs, code-generation, include_str, cargo, rust, testing]
date: 2026-04-16
severity: medium
component: mika-agent
related_issues: ["#598"]
---

# build.rs Code Generation Hygiene

## Context

mika#598 added a `build.rs`-driven directory walk that generates a `static ENTRIES: &[BundledSkill]` table from `skills/bundled/*/skill.toml`. The generated code embeds files via `include_str!` with absolute paths. Code review surfaced three recurring anti-patterns that show up anytime Rust generates Rust source from arbitrary filesystem input — they're not specific to bundled skills, so capturing them as reusable guidance.

## Guidance

Three rules for any `build.rs` that writes Rust source into `OUT_DIR`.

### 1. Use `{:?}` to emit string literals — never hand-roll escape chains

**Wrong:**

```rust
out.push_str(&format!(
    "    SkillFile {{ path: \"{rel}\", content: include_str!(\"{abs}\"), executable: {exe} }},\n",
    rel = file.rel_path,
    abs = abs.replace('\\', "\\\\").replace('"', "\\\""),
    exe = file.executable,
));
```

The manual `.replace()` chain covers backslash and double-quote but silently omits other characters that produce invalid Rust (e.g., a skill directory named `my"skill` or a path fragment containing `\n`). Worse, the escaping is asymmetric across fields — `abs` is escaped, `rel` isn't. A reviewer has to compare every emitted field to verify consistency, and contributors will miss new fields.

**Right:**

```rust
out.push_str(&format!(
    "    SkillFile {{ path: {rel:?}, content: include_str!({abs:?}), executable: {exe} }},\n",
    rel = file.rel_path,
    abs = abs,
    exe = file.executable,
));
```

Rust's `{:?}` (Debug) formatter for `&str` and `Path` produces a correctly quoted, fully escaped string literal — it handles backslashes, double quotes, non-ASCII control characters, and newlines uniformly. Every string field gets the same treatment automatically, and future additions inherit the escaping without the author needing to remember. Confidence in the generated code goes up because the escaping surface collapses to "are we using `{:?}`?".

### 2. Never derive Rust identifiers from arbitrary filesystem names

**Wrong:**

```rust
let upper = entry.name.to_ascii_uppercase().replace('-', "_");
out.push_str(&format!("static {upper}_ENTRY_FILES: &[SkillFile] = &[\n"));
```

This looks readable but has three failure modes on a case-sensitive filesystem:

- `skills/bundled/foo/` and `skills/bundled/FOO/` both map to `FOO_ENTRY_FILES` → duplicate static → `error[E0428]`.
- `my-skill/` and `my_skill/` both map to `MY_SKILL_ENTRY_FILES` → same error.
- Any non-alphanumeric character in the name (period, space, `#`) produces an invalid Rust identifier → a parse error in `$OUT_DIR/generated.rs` that points at the generated file, not the offending directory.

The developer sees the error on a file they've never edited and has to work backward to realize their directory name is the cause.

**Right:**

```rust
for (idx, entry) in entries.iter().enumerate() {
    out.push_str(&format!(
        "static SKILL_{idx}_ENTRY_FILES: &[SkillFile] = &[\n"
    ));
    // ...
    out.push_str(&format!(
        "    BundledSkill {{ name: {name:?}, files: SKILL_{idx}_ENTRY_FILES }},\n",
        name = entry.name,
    ));
}
```

Index-based identifiers can't collide. The human-readable name travels in the `name:` field (a string literal — escaped correctly by `{:?}` per rule #1), preserving discoverability without letting filesystem input determine Rust identifier validity. Apply this pattern to any generator that derives a symbol from user-controlled or filesystem-sourced text.

### 3. Extract merge/transformation logic so tests exercise the real function

**Wrong:**

```rust
fn all_bundled_skills() -> Vec<&'static BundledSkill> {
    let mut merged: Vec<&'static BundledSkill> = BUNDLED_SKILLS.to_vec();
    for entry in ENTRIES {
        if let Some(slot) = merged.iter_mut().find(|existing| existing.name.eq_ignore_ascii_case(entry.name)) {
            *slot = entry;
        } else {
            merged.push(entry);
        }
    }
    merged
}

#[test]
fn test_merge_prefers_entries_on_collision() {
    // Local closure re-implementing the merge algorithm
    fn merge(legacy: &[&BundledSkill], entries: &[BundledSkill]) -> Vec<&BundledSkill> {
        // ... same logic as all_bundled_skills ...
    }
    let merged = merge(legacy, entries);
    // assertions
}
```

The test defines its own copy of the merge algorithm because `all_bundled_skills()` closes over module-level statics and can't accept injected inputs. The assertions pass as long as the test's local copy is correct — the production function could be bypassed or broken and the test wouldn't notice.

**Right:** Extract the logic as a pure function that takes explicit inputs:

```rust
fn merge_skill_lists(
    legacy: &[&'static BundledSkill],
    entries: &'static [BundledSkill],
) -> Vec<&'static BundledSkill> {
    // single authoritative implementation
}

fn all_bundled_skills() -> Vec<&'static BundledSkill> {
    merge_skill_lists(BUNDLED_SKILLS, ENTRIES)
}

#[test]
fn test_merge_prefers_entries_on_collision() {
    let merged = merge_skill_lists(LEGACY_FIXTURE, ENTRIES_FIXTURE);
    // assertions
}
```

Now there's one merge function, one test surface, and any change to the production semantics fails the test. The extraction is small (five lines moved) but eliminates the "test passes against a stale copy" failure class entirely.

**Companion rule:** when two functions conceptually do the same lookup (e.g., `is_bundled_skill()` and `all_bundled_skills()` both enumerate sources), add a coupling guard test: `for skill in all_bundled_skills() { assert!(is_bundled_skill(skill.name)); }`. Cheap to write, catches silent drift when a future contributor adds a third source to one function without updating the other.

### Sharing a helper between build.rs and integration tests

When a generator's discovery logic is worth testing in isolation, `build.rs` can't `use` anything from `src/` (compiles before the crate) and a new workspace crate is over-engineering for ~130 lines. The pragmatic pattern is `#[path]` mod attributes from both sides:

```
crates/<crate>/build_support/discovery.rs   # shared helper, outside src/
crates/<crate>/build.rs:
    #[path = "build_support/discovery.rs"]
    mod discovery;
crates/<crate>/tests/foo_integration.rs:
    #[path = "../build_support/discovery.rs"]
    mod discovery;
```

Both sides compile the same file. Integration tests can run the discovery logic against fixtures under `tests/fixtures/` while `build.rs` runs it against the production source tree. Add `#[allow(dead_code)]` to fields that only one consumer reads — when two unrelated compilation units share a file, per-consumer dead-code warnings fire even though every field is used in aggregate.

## Why This Matters

All three rules trade "looks fine on the happy path" for "fails safely on the adversarial path." The happy path for this PR was 12 legacy skills and an empty directory. The adversarial path is six months from now when a migration ticket populates `skills/bundled/` with real skills and a contributor names one `my.skill` or adds `foo/` alongside an existing `FOO/`. Rule #1 prevents syntax errors from filesystem content. Rule #2 prevents identifier collisions. Rule #3 prevents silent drift between tests and production.

The cost of applying these upfront is roughly zero — `{:?}` is shorter than manual escaping, index-based identifiers are shorter than name-derived ones, and extracting a pure helper is a five-line refactor. The cost of skipping them surfaces weeks or months later as opaque error messages pointing at generated files.

## When to Apply

- Any `build.rs` that emits Rust source code from filesystem input or external data
- Any macro-free code-generation pass that composes string literals into Rust syntax
- Any test that claims to verify a merge, transformation, or routing algorithm that lives in a production function
- Any helper that needs to be shared between `build.rs` and tests (use `#[path]` mod attributes)

## Examples

See the PR for #598:

- `crates/mika-agent/build.rs` — uses `{:?}` for all emitted string literals; uses `SKILL_{idx}_ENTRY_FILES` instead of name-derived identifiers
- `crates/mika-agent/src/bundled_skills.rs` — `merge_skill_lists()` extracted as a pure helper; `test_merge_prefers_entries_on_collision` and sibling tests drive it directly; `test_is_bundled_skill_agrees_with_all_bundled_skills` is the coupling guard
- `crates/mika-agent/build_support/bundled_skills_discover.rs` — shared discovery helper included from both `build.rs` and `tests/bundled_skills_directory_source.rs` via `#[path]`

## Reference

- PR: Bundle engine-coupled skills — build-time discovery refactor (#598)
- Related: `crates/mika-agent/build.rs` for the canonical pattern going forward
