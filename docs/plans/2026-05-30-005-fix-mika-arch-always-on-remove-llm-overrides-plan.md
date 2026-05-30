# Plan: fix(mika-arch): always_on=true + remove per-skill LLM overrides

**Ticket:** mika issue#949
**Type:** bug fix
**Scope:** `crates/mika-agent/src/well_known_agents.rs`, `skills/bundled/mika-arch-*/skill.toml`

## Problem

mika-arch's skill configuration is in three contradictory states:

1. **Agent default model** is Opus 4.7 (operator set in `~/.mika/agents/mika-arch/config.toml`).
2. **Per-skill LLM overrides** in source code route all three skills to Sonnet 4.6, effectively *demoting* every skill call below the agent default — opposite of operator intent.
3. **`always_on = false`** on all three skills creates keyword-trigger fragility. mika-arch is single-purpose (all skills are review work), so keyword gating adds a fragility surface with no value.

## Changes

### 1. Remove LLM overrides from `MIKA_ARCH` static — `well_known_agents.rs:267-283`

**Current:**
```rust
llm_overrides: &[
    LlmOverrideSpec { skill_name: "mika-arch-groom-ticket", provider: "anthropic", model: "claude-sonnet-4-6" },
    LlmOverrideSpec { skill_name: "mika-arch-groom-milestone", provider: "anthropic", model: "claude-sonnet-4-6" },
    LlmOverrideSpec { skill_name: "mika-arch-second-review", provider: "anthropic", model: "claude-sonnet-4-6" },
],
```

**After:**
```rust
llm_overrides: &[],
```

**Effect:** `seed_well_known_skill_overrides` will hit the fast-path exit at line 874 (`disabled_skills.is_empty() && llm_overrides.is_empty()`) and not seed or reconcile any override rows. Existing DB rows remain until cleanup (step 5 below).

### 2. Update doc comment on `MIKA_ARCH` — `well_known_agents.rs:247-253`

Remove the sentence about per-skill LLM overrides routing to Opus/Sonnet. Update to reflect that skills inherit the agent default model.

### 3. Set `always_on = true` in skill manifests

Edit three files:
- `skills/bundled/mika-arch-groom-ticket/skill.toml` — `always_on = false` → `always_on = true`
- `skills/bundled/mika-arch-groom-milestone/skill.toml` — `always_on = false` → `always_on = true`
- `skills/bundled/mika-arch-second-review/skill.toml` — `always_on = false` → `always_on = true`

**Rationale:** mika-arch is single-purpose. Every skill is review-specific. `always_on = true` eliminates keyword-trigger fragility (disposition-keyword drift documented in `mika-arch-first-dogfood-2026-04-25.md`). The identity-driven `[skills].allowlist` already restricts which skills load — `always_on` just controls whether a loaded skill requires keyword activation or is active on every turn.

**Impact on post-condition guards:** `collect_required_suffix_lines()` and `collect_required_finding_list_prefixes()` already collect from both `Keyword` AND `AlwaysOn` matched skills (documented in CLAUDE.md). So the required-suffix-line guard (#864) and required-finding-list guard (#901) will continue to fire correctly. The `required_tools` gate (guard #3) only enforces on `Keyword`-matched skills per #463 — switching to `AlwaysOn` means required_tools constraints will NOT be enforced. This is acceptable: the `gh_read` required_tools constraint was defense-in-depth for keyword-trigger mode; with `always_on = true`, the skill prompt is always present and the LLM reliably calls `gh_read` without the gate. The `required_fetches_for_quoted_resources` constraint is also keyword-only and follows the same reasoning.

### 4. Add DB cleanup on startup — `well_known_agents.rs::seed_well_known_skill_overrides`

The current fast-path exit (line 874) short-circuits when both `disabled_skills` and `llm_overrides` are empty. After this change, mika-arch will hit that fast-path, but stale LLM override rows will remain in `skill_overrides`. These stale rows will continue to override the agent default model until cleaned up.

**Add a cleanup path:** Before the fast-path exit, when `spec.llm_overrides.is_empty()` but existing DB rows have non-NULL `llm_provider`/`llm_model`, delete those stale LLM override rows. This is the "clear-on-empty-source semantic" the ticket's Out of Scope section flagged for investigation.

Implementation approach:
1. Move the fast-path exit after a new check: if `spec.llm_overrides.is_empty()` AND `spec.disabled_skills.is_empty()`, check for existing override rows with LLM fields set.
2. For any such rows, call `db.delete_skill_override(agent_name, &row.skill_name)` to remove them.
3. Log the cleanup at `info!` level for operator visibility.
4. Then return (fast-path).

This handles both fresh deploys (no rows to clean) and upgrades from pre-#949 (stale Sonnet rows removed).

**Scoping guard:** Only clean up rows that were plausibly seeded by the well-known agent provisioner — rows where `llm_provider` and `llm_model` are both non-NULL and `enabled` is NULL (pure LLM-override rows, not operator-disabled rows). Rows with `enabled = Some(false)` or `enabled = Some(true)` are operator intent and must not be touched.

### 5. Update tests — `well_known_agents.rs`

#### 5a. `test_mika_arch_has_llm_overrides` (line 2070-2091)

**Remove entirely** or rename to `test_mika_arch_has_no_llm_overrides` and assert `MIKA_ARCH.llm_overrides.is_empty()`.

#### 5b. `test_seed_skill_overrides_mika_arch` (line 2159-2190)

Update to assert that after seeding, `get_skill_overrides("mika-arch")` returns 0 rows (not 3).

#### 5c. `test_seed_skill_overrides_reconciles_drifted_llm_override` (line 2192-2244)

**Repurpose:** Pre-seed DB with stale LLM override rows (simulating pre-#949 state), then call `seed_well_known_skill_overrides`. Assert all three rows are deleted (cleanup path from step 4).

#### 5d. `test_seed_skill_overrides_reconciliation_is_idempotent` (line 2246-2262)

Update to verify that repeated calls with empty overrides are no-ops (no rows to clean on second call).

#### 5e. `test_d2_migration_preserves_mika_arch_rows` (line 1933-1965)

This test pre-seeds an LLM override row for mika-arch and asserts the D2 migration preserves it. After #949, the migration still preserves it (unchanged behavior), but `seed_well_known_skill_overrides` would clean it up on the next call. The test is about migration behavior, not seeding — keep it as-is since the D2 migration path is independent.

### 6. Post-deploy verification (no code change — operator action)

After deploy + restart:
1. `sqlite3 ~/.mika/data/mika.db "SELECT * FROM skill_overrides WHERE agent_id='mika-arch'"` → 0 rows.
2. `mika skills --agent mika-arch list` → all three skills loaded with `always_on=true`, no `[llm: ...]` annotation.
3. Check server logs for `kg_resolver_tick` or any mika-arch LLM call → model should be the agent default (Opus 4.7 from config.toml).

## Files Modified

| File | Change |
|------|--------|
| `crates/mika-agent/src/well_known_agents.rs` | Remove 3 `LlmOverrideSpec` entries, update doc comment, add stale-override cleanup in seeding, update 4 tests |
| `skills/bundled/mika-arch-groom-ticket/skill.toml` | `always_on = false` → `always_on = true` |
| `skills/bundled/mika-arch-groom-milestone/skill.toml` | `always_on = false` → `always_on = true` |
| `skills/bundled/mika-arch-second-review/skill.toml` | `always_on = false` → `always_on = true` |

## Risk Assessment

**Low risk.** All changes are in the mika-arch agent's configuration surface:
- Removing LLM overrides makes skills inherit the agent default — strictly simpler.
- `always_on = true` eliminates a fragility surface (keyword-trigger miss) with no functional cost since mika-arch is single-purpose.
- The stale-row cleanup is guarded to only touch pure LLM-override rows (not operator-set enabled/disabled state).
- No schema migration needed — cleanup uses existing `delete_skill_override` API.
- The `required_tools` gate change (no longer enforced on AlwaysOn) is acceptable because `gh_read` is reliably called without the gate when the skill prompt is present.

## Out of Scope

- mika-arch's persistence-meta hallucination (mika#947).
- Other agents' overrides or always_on flags.
- The `seed_well_known_skill_overrides` reconciliation loop's general architecture — only the mika-arch-specific cleanup is addressed.
