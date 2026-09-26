---
title: "fix: mika-arch always_on=true + remove per-skill LLM overrides"
type: fix
status: active
date: 2026-05-29
---

# fix: mika-arch always_on=true + remove per-skill LLM overrides

## Overview

Remove the three per-skill `LlmOverrideSpec` entries from `MIKA_ARCH` and flip all three mika-arch skills to `always_on = true`. After this change, mika-arch skills inherit the agent default model (Opus 4.7, operator-set in `~/.mika/agents/mika-arch/config.toml`) instead of being demoted to Sonnet 4.6. The `always_on = true` flag eliminates keyword-trigger fragility that caused disposition-keyword ghosting in prior dogfooding (documented in `docs/solutions/best-practices/mika-arch-first-dogfood-2026-04-25.md`).

## Problem Frame

Post-mika#939 merge, mika-arch is in a contradictory three-way state:

1. **Agent default**: Opus 4.7 (operator-set in `config.toml`)
2. **Per-skill LLM overrides** (source in `well_known_agents.rs`): all three skills route to Sonnet 4.6
3. **Skill `always_on`** (in `skill.toml`): all three are `false` (keyword-triggered)

Combined effect: the overrides DEMOTE every skill call from Opus to Sonnet (opposite of operator intent). The `always_on=false` creates a fragility surface where keyword-trigger sometimes fires late or partially, causing the architect to skip the `Disposition:` suffix line.

mika-arch is single-purpose — every skill is review-specific. Keyword-trigger gating exists for general-purpose agents that might do non-skill work; it is a fragility surface here, not a value-add.

## Requirements Trace

- R1. Per-skill LLM override entries removed from source — skills inherit agent default model
- R2. All three mika-arch skills set to `always_on = true` in their `skill.toml`
- R3. Stale `skill_overrides` DB rows cleaned up on deploy (auto-migration, not manual operator action)
- R4. Tests updated to reflect the new configuration
- R5. Post-deploy: `SELECT * FROM skill_overrides WHERE agent_id='mika-arch'` returns 0 rows

## Scope Boundaries

- Other agents' overrides or `always_on` flags are out of scope
- mika-arch's persistence-meta hallucination (mika#947) is orthogonal
- The `seed_well_known_skill_overrides` reconciliation loop's general architecture is not being refactored — only adding a clear-on-empty-source cleanup path
- The `AlwaysOn + DB-override carve-out` (mika#1011) is unaffected — it fires on `from_db_override = true` rows, which will no longer exist for mika-arch

## Context & Research

### Relevant Code and Patterns

- `crates/mika-agent/src/well_known_agents.rs:258-284` — `MIKA_ARCH` static with the three `LlmOverrideSpec` entries to remove
- `crates/mika-agent/src/well_known_agents.rs:866-1000` — `seed_well_known_skill_overrides()` — the seeding/reconciliation function. Currently takes a fast-path exit when both `disabled_skills` and `llm_overrides` are empty (line 874). Post-fix, mika-arch will hit this fast path, but pre-existing DB rows will persist unless cleaned up
- `crates/mika-agent/src/well_known_agents.rs:1155-1163` — `MIKA_ARCH_CONFIG` const with stale comment about per-skill overrides
- `skills/bundled/mika-arch-groom-ticket/skill.toml`, `skills/bundled/mika-arch-groom-milestone/skill.toml`, `skills/bundled/mika-arch-second-review/skill.toml` — the three skill manifests with `always_on = false`
- Post-#815 migration pattern: `migrate_well_known_to_identity_allowlist()` uses `schema_meta` marker `well_known_d2_migration_v1` for one-shot cleanup — same pattern applies here

### Institutional Learnings

- `docs/solutions/runtime-errors/well-known-agent-disabled-skills-seeding-drift-2026-05-09.md` — Documents the "seeding-once drift" failure class and one-directional reconciliation design. Key insight: removing an override from spec does NOT auto-delete the DB row. A cleanup migration is required.
- `docs/solutions/architecture-patterns/well-known-agent-identity-allowlist-migration-2026-05-15.md` — Confirms mika-arch is the only well-known agent still entering `seed_well_known_skill_overrides()` (for LLM override reconciliation). Post-fix, all four agents take the fast-path exit.

## Key Technical Decisions

- **Auto-cleanup via one-shot migration, not manual operator action**: The ticket suggests a post-deploy `DELETE FROM skill_overrides WHERE agent_id='mika-arch'`. Instead, add a one-shot cleanup inside `seed_well_known_skill_overrides()` that detects when the source spec has no overrides but the DB still has LLM override rows, and deletes them. This follows the existing `schema_meta` marker pattern from `migrate_well_known_to_identity_allowlist()` but is simpler — when the source is empty and DB rows exist, the rows are stale by definition. Guard with a `schema_meta` marker `mika_arch_llm_override_cleanup_v1` for idempotency.
- **`always_on = true` effect on post-condition guards**: With `always_on = true`, `collect_required_suffix_lines()` and `collect_required_finding_list_prefixes()` will union these skills' output constraints on every turn (not just keyword-matched turns). This is the intended fix — the `Disposition:` / `Verdict:` suffix line enforcement becomes unconditional.
- **`required_tools` constraint (`gh_read`) unchanged**: Per match-reason conditioning (#463), `required_tools` is only enforced on `Keyword` matches. With `always_on = true`, the `gh_read` constraint fires only when keywords also match. This is acceptable — the skill prompt instructs `gh_read` usage, and mika-arch messages always contain review-related keywords.

## Open Questions

### Resolved During Planning

- **Q: Does the fast-path exit in `seed_well_known_skill_overrides` need a new code path?** Yes — currently when both arrays are empty, the function returns immediately. Pre-existing DB rows from the old spec persist forever. Adding a cleanup path guarded by a `schema_meta` marker handles the transition cleanly without touching the general reconciliation architecture.
- **Q: Does `always_on = true` affect the `AlwaysOn + DB-override carve-out` (mika#1011)?** No. That carve-out fires on `from_db_override = true` rows (set by `apply_overrides()` when reading `skill_overrides` DB rows). After cleanup, mika-arch has zero `skill_overrides` rows, so no DB-override is applied, and the carve-out is never triggered.

### Deferred to Implementation

- **Q: Exact `schema_meta` marker key name** — will be finalized during implementation (proposed: `mika_arch_llm_override_cleanup_v1`).

## Implementation Units

- [ ] **Unit 1: Remove LLM overrides from MIKA_ARCH static and update config comment**

**Goal:** Remove the three `LlmOverrideSpec` entries and update all associated comments/doc strings.

**Requirements:** R1

**Dependencies:** None

**Files:**
- Modify: `crates/mika-agent/src/well_known_agents.rs`

**Approach:**
- Change `llm_overrides: &[LlmOverrideSpec { ... }, ...]` to `llm_overrides: &[]` on the `MIKA_ARCH` static (lines 267-283)
- Update the doc comment (lines 249-253) to state skills inherit the agent default model
- Update `MIKA_ARCH_CONFIG` comment (line 1157) to remove mention of per-skill LLM overrides

**Patterns to follow:**
- `MIKA_DEV`, `MIKA_QA`, `MIKA_RELAY` all use `llm_overrides: &[]` — mika-arch now matches

**Test scenarios:**
- Happy path: `MIKA_ARCH.llm_overrides` is empty (len == 0)
- Happy path: `MIKA_ARCH_CONFIG` parses as valid TOML with `llm_provider = "openrouter"` and `openrouter_model = "moonshotai/kimi-k2.5"` (base model unchanged)

**Verification:**
- `MIKA_ARCH.llm_overrides.is_empty()` asserts true
- Existing `test_mika_arch_config_toml_is_valid_toml` continues to pass

---

- [ ] **Unit 2: Flip always_on to true in all three mika-arch skill.toml files**

**Goal:** Change `always_on = false` to `always_on = true` in all three mika-arch skill manifests.

**Requirements:** R2

**Dependencies:** None (can be done in parallel with Unit 1)

**Files:**
- Modify: `skills/bundled/mika-arch-groom-ticket/skill.toml`
- Modify: `skills/bundled/mika-arch-groom-milestone/skill.toml`
- Modify: `skills/bundled/mika-arch-second-review/skill.toml`

**Approach:**
- Single-line change in each file: `always_on = false` -> `always_on = true`
- The `[triggers] keywords` section remains — keywords still contribute to `MatchReason::Keyword` for `required_tools` enforcement, but the skill loads unconditionally via `AlwaysOn`

**Patterns to follow:**
- Other always-on skills in the codebase (e.g., community bundled skills) for the manifest pattern

**Test scenarios:**
- Happy path: build-time discovery (`build.rs`) picks up the updated manifests and `BUNDLED_SKILL_MANIFESTS` reflects `always_on = true` for all three skills

**Verification:**
- `cargo build` succeeds (build.rs re-discovers skills)
- The three skills appear as `always_on = true` in the compiled `BUNDLED_SKILL_MANIFESTS`

---

- [ ] **Unit 3: Addone-shot DB cleanup for stale mika-arch LLM override rows**

**Goal:** Ensure pre-existing `skill_overrides` rows for mika-arch are cleaned up on the first deploy after this change, so R5 is met without manual operator intervention.

**Requirements:** R3, R5

**Dependencies:** Unit 1 (the source spec must be empty for the cleanup logic to be correct)

**Files:**
- Modify: `crates/mika-agent/src/well_known_agents.rs`

**Approach:**
- Add a one-shot cleanup function (e.g., `migrate_mika_arch_llm_override_cleanup()`) that:
  1. Checks `schema_meta` for marker `mika_arch_llm_override_cleanup_v1` — if present, return (idempotent)
  2. Deletes all `skill_overrides` rows where `agent_id = 'mika-arch'` and `llm_provider IS NOT NULL` (targets only LLM override rows, not hypothetical `enabled` rows)
  3. Writes the `schema_meta` marker
  4. Logs at INFO level with count of deleted rows
- Call this function from the same site that calls `seed_well_known_skill_overrides()` during agent init — either inside `seed_well_known_skill_overrides` itself (before the fast-path exit) or as a sibling call at the init callsite
- The cleanup is agent-scoped to `mika-arch` — no other agents' rows are touched

**Patterns to follow:**
- `migrate_well_known_to_identity_allowlist()` in the same file — uses `schema_meta` marker `well_known_d2_migration_v1`, one-shot execution, idempotent guard, INFO-level logging

**Test scenarios:**
- Happy path: DB has 3 mika-arch LLM override rows -> cleanup deletes all 3 -> `schema_meta` marker written -> subsequent calls are no-ops
- Edge case: DB has zero mika-arch rows -> cleanup writes marker, deletes nothing
- Edge case: DB has mika-arch rows with `enabled` column set (hypothetical operator disable) -> cleanup only deletes rows with `llm_provider IS NOT NULL`, preserving `enabled`-only rows
- Idempotency: calling cleanup twice produces the same result (marker prevents re-execution)

**Verification:**
- `SELECT * FROM skill_overrides WHERE agent_id='mika-arch'` returns 0 rows after cleanup
- `schema_meta` contains the marker key

---

- [ ] **Unit 4: Update tests to reflect the new configuration**

**Goal:** Update all mika-arch-related tests in `well_known_agents.rs` to match the new state (empty overrides, cleanup migration).

**Requirements:** R4

**Dependencies:** Units 1, 2, 3

**Files:**
- Modify: `crates/mika-agent/src/well_known_agents.rs` (test module)

**Approach:**

Tests to rewrite:
- `test_mika_arch_has_llm_overrides` → rename to `test_mika_arch_has_no_llm_overrides` — assert `MIKA_ARCH.llm_overrides.is_empty()`
- `test_seed_skill_overrides_mika_arch` → rewrite to assert that seeding produces 0 rows (mika-arch now takes the fast-path exit)
- `test_seed_skill_overrides_reconciles_drifted_llm_override` → remove entirely (no overrides to reconcile)
- `test_seed_skill_overrides_reconciliation_is_idempotent` → if mika-arch-specific assertions exist, remove them; if the test is mika-arch-only, remove entirely

Tests to update:
- `test_d2_migration_preserves_mika_arch_rows` → update to reflect that mika-arch no longer has LLM override rows post-cleanup. The test should verify that the D2 migration + the new cleanup migration together leave mika-arch with 0 rows

Tests to add:
- `test_mika_arch_llm_override_cleanup_migration` — verifies the one-shot cleanup (happy path, idempotency, no-op when no rows exist)

**Patterns to follow:**
- Existing test structure in the `#[cfg(test)] mod tests` block — `Database::open_in_memory()`, `register_agent()`, `set_skill_llm_override()`, `get_skill_overrides()`

**Test scenarios:**
- Happy path: new `test_mika_arch_has_no_llm_overrides` asserts empty overrides on the static
- Happy path: `test_seed_skill_overrides_mika_arch` asserts 0 rows after seeding (fast-path exit)
- Happy path: cleanup migration deletes pre-existing LLM override rows
- Idempotency: cleanup migration is a no-op on second call
- Edge case: cleanup with no pre-existing rows writes marker without error

**Verification:**
- `cargo test -p mika-agent -- well_known` passes with all tests green
- No test references the removed `LlmOverrideSpec` entries

## System-Wide Impact

- **Skill matching behavior**: With `always_on = true`, the three mika-arch skills are matched on every turn regardless of keyword presence. `required_suffix_lines` and `required_finding_list_prefixes` from `[output]` are now enforced unconditionally (they already union `AlwaysOn` matches). `required_tools` from `[constraints]` remains keyword-gated per #463 — no change.
- **LLM provider resolution**: Without DB override rows, `resolve_skill_llm_override()` returns `None` for all three skills. The agent-level provider (Kimi via OpenRouter, per `MIKA_ARCH_CONFIG`) is used for the orchestration shell; the operator's `config.toml` override (Opus 4.7) applies at runtime. Skills inherit whichever model the agent resolves to.
- **`seed_well_known_skill_overrides` fast path**: Post-fix, all four well-known agents take the fast-path exit (empty `disabled_skills` + empty `llm_overrides`). The function body past line 874 becomes dead code for well-known agents but remains correct for future agents that might use overrides.
- **Unchanged invariants**: mika-arch's identity allowlist (`[skills].allowlist`), tool denylist (`[tools].disabled`), KG configuration (`[kg]`), soul prompt, and config.toml base model are all unchanged. The fail-closed identity behavior on parse failure is unaffected.

## Risks & Dependencies

| Risk | Mitigation |
|------|------------|
| Stale DB rows persist if cleanup migration doesn't run | `schema_meta` marker pattern is proven (D2 migration). Cleanup runs in the same init path as `seed_well_known_skill_overrides`. Acceptance criterion R5 verifies. |
| `always_on = true` changes post-condition guard behavior unexpectedly | Only `required_suffix_lines` and `required_finding_list_prefixes` change from conditional to unconditional — both are the intended fix. `required_tools` stays keyword-gated. |
| Operator had manually set LLM overrides via `mika skills llm set` | The cleanup targets `llm_provider IS NOT NULL` rows — if an operator manually set a different override, it would also be deleted. Acceptable: the ticket's direction is to remove ALL per-skill overrides for mika-arch, and the operator can re-set if needed. |

## Sources & References

- Related issue: mika#949
- Predecessor: mika#939 / PR #941 — established the Sonnet 4.6 routing now being reverted
- Source: `crates/mika-agent/src/well_known_agents.rs` (MIKA_ARCH static, seed function, tests)
- Source: `skills/bundled/mika-arch-{groom-ticket,groom-milestone,second-review}/skill.toml`
- Dogfood doc: `docs/solutions/best-practices/mika-arch-first-dogfood-2026-04-25.md`
- Learning: `docs/solutions/runtime-errors/well-known-agent-disabled-skills-seeding-drift-2026-05-09.md`
- Learning: `docs/solutions/architecture-patterns/well-known-agent-identity-allowlist-migration-2026-05-15.md`
