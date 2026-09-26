---
title: "fix(skills): address PR #478 review findings (must-fix #1-3)"
type: fix
status: active
date: 2026-04-07
---

# fix(skills): address PR #478 review findings (must-fix #1-3)

Surgical iteration on PR #478. The merge of `write_skill_variant` into `review_skill` (parent plan: `docs/plans/2026-04-07-004-fix-merge-skill-variant-into-review-plan.md`) shipped three review issues that must be fixed before merge. This plan addresses **only** findings #1, #2, #3 from [PR #478 review 4069963804](https://github.com/senara-solutions/mika/pull/478#pullrequestreview-4069963804). Findings #4–#8 are explicit follow-ups, out of scope.

All changes are in `crates/mika-agent/src/skills/builtin_handlers.rs`, function `review_skill_single` (lines 1022–1194). Tests in `crates/mika-agent/src/skills/builtin_handlers.rs::tests` (lines 2072+) must stay green.

## Acceptance Criteria

- [ ] Inspect and persist branches return the **same core fields** (`provider`, `model`, `target_path`) — no shape branching by mode (#1)
- [ ] `warn_linked_skill_write` fires **only when a write actually happens through a symlink**, not on every linked-skill inspect call (#2)
- [ ] Warning string tense matches branch reality: future for inspect ("will be persisted"), past or absent for persist (#3)
- [ ] All 16 `test_review_skill_*` tests pass after assertion updates
- [ ] `cargo clippy --all-targets -- -D warnings` clean
- [ ] `cargo fmt` clean

## Changes

### Finding #1 — Unify response shape

**Current state** (`builtin_handlers.rs`):
- Persist branch (lines 1157–1174) emits: `runtime_provider`, `runtime_model`, `provider`, `model`, `existing_variant`, `dry_run`, `skipped`, `linked`, `warning`, `written`, `written_path`, `content_bytes`, `source_bytes`
- Inspect branch (lines 1179–1191) emits: `runtime_provider`, `runtime_model`, `existing_variant`, `dry_run`, `skipped`, `linked`, `warning`, `written` — **missing** `provider`, `model`, and any path field

**Change:**
- Add `provider`, `model`, `target_path` to inspect-branch JSON
- Rename `written_path` → `target_path` in persist-branch JSON (single canonical name across branches; `written: bool` already disambiguates whether the file exists yet)
- `target_path` value = `variant_path.display().to_string()` in both branches (already computed at line 1085)
- Keep `content_bytes`/`source_bytes` only in persist (write-specific metrics)
- Keep `runtime_provider`/`runtime_model` in both for now — they're a separate duplication finding tracked elsewhere, not part of #1

**Resulting unified core (both branches):** `skill_name`, `root_prompt`, `tools_json`, `runtime_provider`, `runtime_model`, `provider`, `model`, `target_path`, `existing_variant`, `dry_run`, `skipped`, `linked`, `warning`, `written`

### Finding #2 — Move `warn_linked_skill_write` into the actual-write path

**Current state** (`builtin_handlers.rs:1054-1056`):
```rust
if linked {
    warn_linked_skill_write(skill_name, &skill_dir);
}
```
This fires unconditionally for **any** review of a linked skill — including pure inspect calls and dry-runs. The reviewer's note that "the warn! is gone" is technically incorrect (the call exists), but the *intent* — log only when a real write happens through a symlink — is the correct semantic. Currently the warning is noise.

**Change:**
- Delete the unconditional call at lines 1054–1056
- Inside the persist branch, after the `if !dry_run` `fs::write` block succeeds (line 1148, before `skills_dirty.store(...)`), call `warn_linked_skill_write(skill_name, &skill_dir)` only when `linked && written`
- Result: log fires exactly once per actual symlink write, never on inspect or dry-run

### Finding #3 — Branch-specific warning tense

**Current state** (`builtin_handlers.rs:1104-1112`):
```rust
let warning = if linked {
    Some(format!(
        "linked skill: any variant written by review_skill will be persisted \
         through the symlink to the source directory at {}",
        skill_dir.display()
    ))
} else {
    None
};
```
Single string, future tense ("will be persisted"). Reused in both branches' JSON. On the persist+`!dry_run` path the write already happened — tense is wrong.

**Change:**
- Move `warning` construction into each branch
- Persist branch (`!dry_run` case after write succeeds): warning = `Some(format!("linked skill: variant persisted through symlink to source directory at {}", skill_dir.display()))`
- Persist branch (`dry_run` case): warning = `Some(format!("linked skill: variant would be persisted through symlink to source directory at {}", skill_dir.display()))`
- Inspect branch: keep the original future-tense message (no write planned, but the agent is being warned in case it follows up with content)
- Non-linked: `None` in all cases (unchanged)

## Test updates

Tests that assert the persist-branch response shape (line numbers from current worktree HEAD `a8ca63b`):

- `test_review_skill_inspect_then_persist_round_trip` (:2551) — already asserts `content_bytes`; add assertions for `target_path` symmetry between inspect and persist
- `test_review_skill_persist_dry_run_does_not_touch_disk` (:2590) — currently reads `parsed["written_path"]`; rename to `target_path`
- `test_review_skill_persist_rejects_batch_mode` (:2622) — no field rename needed; verify
- Tests at :2354, :2385 reading `parsed["written_path"]` — rename to `target_path`
- `test_review_skill_linked_skill_warns_and_succeeds` (:2143) — currently expects warn from inspect; update to call persist mode with content (and `force=true` if needed) so the warn fires per the new semantics. Add a sibling test that asserts a pure inspect of a linked skill does **not** emit the warn (use `tracing-test` if needed, or a counter-style fake — match existing test patterns in the file)

## Out of scope (track as follow-ups)

Per the reviewer, **do not address**:
- #4 nested-symlink escape (pre-existing gap, not a regression)
- #5 `#[allow(clippy::too_many_arguments)]` refactor
- #6 absolute path leak in JSON response
- #7 `MIN_VARIANT_RATIO` truncation guard comment
- #8 stale prose in `templates/skills/skill-review/system_prompt.md`

If any of #4–#8 become trivial during the fixes for #1–#3, file a follow-up issue rather than scope-creeping this PR.

## Verification

```bash
cargo test -p mika-agent skills::builtin_handlers::tests::test_review_skill 2>&1 | tail -30
cargo clippy -p mika-agent --all-targets -- -D warnings
cargo fmt --check
```

Manual smoke (after build):
```bash
# Inspect (no content) — must show provider/model/target_path, no warn! log
mika ask "review web-search skill"

# Persist (with content) on linked skill — must show warn! log exactly once
# (use a linked test skill if available, otherwise scope to unit tests)
```

## Sources

- Parent PR plan: [docs/plans/2026-04-07-004-fix-merge-skill-variant-into-review-plan.md](2026-04-07-004-fix-merge-skill-variant-into-review-plan.md)
- Compound learning origin: `docs/solutions/merge-two-step-llm-tool-contracts.md` (the asymmetry in #1 violates this learning's central thesis)
- PR review: https://github.com/senara-solutions/mika/pull/478#pullrequestreview-4069963804
- Code under change: `crates/mika-agent/src/skills/builtin_handlers.rs:1022-1208`
- Tests: `crates/mika-agent/src/skills/builtin_handlers.rs:2072-2700`
