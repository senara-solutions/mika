# Plan: fix(mika-arch): always_on=true + remove per-skill LLM overrides

**Ticket:** mika issue#949
**Type:** bug fix
**Scope:** `crates/mika-agent/src/well_known_agents.rs`, `skills/bundled/mika-arch-*/skill.toml`

## Problem

mika-arch's skill configuration is in three contradictory states:

1. Agent default model is Opus 4.7 (operator-set in `config.toml`).
2. Per-skill LLM overrides in `well_known_agents.rs` route all three skills to Sonnet 4.6 — demoting them below the agent default.
3. All three skills are `always_on = false` (keyword-triggered), creating fragility where the architect may skip the `Disposition:` suffix when keyword-trigger fires late or partially.

## Solution

Remove the per-skill LLM overrides so skills inherit the agent default (Opus 4.7). Set `always_on = true` on all three skills since mika-arch is single-purpose (every invocation is review work). Add a cleanup path for stale DB rows.

## Changes

### 1. Remove LLM overrides from `MIKA_ARCH` static (`well_known_agents.rs`)

**File:** `crates/mika-agent/src/well_known_agents.rs`

Change `MIKA_ARCH.llm_overrides` from the current 3-element array to `&[]` (empty).

```rust
// Before:
llm_overrides: &[
    LlmOverrideSpec { skill_name: "mika-arch-groom-ticket", provider: "anthropic", model: "claude-sonnet-4-6" },
    LlmOverrideSpec { skill_name: "mika-arch-groom-milestone", provider: "anthropic", model: "claude-sonnet-4-6" },
    LlmOverrideSpec { skill_name: "mika-arch-second-review", provider: "anthropic", model: "claude-sonnet-4-6" },
],

// After:
llm_overrides: &[],
```

Update the docstring on `MIKA_ARCH` to remove the mention of per-skill LLM overrides routing to Opus/Sonnet. The new doc should state that skills inherit the agent default model.

### 2. Add stale-override cleanup to `seed_well_known_skill_overrides` (`well_known_agents.rs`)

**Problem:** The current `seed_well_known_skill_overrides` function has a fast-path exit when both `disabled_skills` and `llm_overrides` are empty (line 839). After this change, mika-arch hits that fast path — but stale DB rows from the previous 3-override spec remain in `skill_overrides`. The reconciliation loop (lines 888–933) only updates drifted values; it never deletes rows that no longer appear in the spec.

**Solution:** Add a cleanup step that runs when `spec.llm_overrides` is empty but the DB still has LLM override rows for this agent. This must run BEFORE the fast-path exit.

In `seed_well_known_skill_overrides`, after the `find_well_known_agent` lookup and before the fast-path exit:

1. Query `db.get_skill_overrides(agent_name)`.
2. If `spec.llm_overrides.is_empty()` and existing overrides contain rows with non-None `llm_provider` or `llm_model`, delete those rows via `db.delete_skill_override(agent_name, skill_name)`.
3. Log each deletion at INFO level for operator visibility.

This is idempotent: on subsequent runs, no LLM override rows exist, so the cleanup is a no-op.

**Important:** Only delete rows whose sole purpose was the LLM override (i.e., `enabled` is `None` — the default). If a row has `enabled = Some(false)`, that was an operator-set disable and must not be deleted. If a row has both an LLM override and an `enabled` flag, clear the LLM fields but keep the row for the `enabled` state.

The cleanup should be structured as a new helper function `cleanup_stale_llm_overrides(db, agent_name, spec)` called from `seed_well_known_skill_overrides` to keep the main function's control flow clean.

### 3. Set `always_on = true` in skill manifests

**Files:**
- `skills/bundled/mika-arch-groom-ticket/skill.toml`
- `skills/bundled/mika-arch-groom-milestone/skill.toml`
- `skills/bundled/mika-arch-second-review/skill.toml`

Change `always_on = false` to `always_on = true` in all three files.

**Note on `[triggers] keywords`:** Keep the keywords section. While `always_on = true` means the skill is always matched, the keywords section is still used by `collect_required_tools()` which only fires on keyword-matched skills (#463). However, since `always_on` skills' `required_suffix_lines` and `required_finding_list_prefixes` are collected from both `Keyword` and `AlwaysOn` match reasons (per `collect_required_suffix_lines` and `collect_required_finding_list_prefixes` in the crate CLAUDE.md), the output contracts continue to be enforced. The keywords section is retained for documentation/grep-ability.

### 4. Update tests (`well_known_agents.rs`)

**a. `test_mika_arch_has_llm_overrides` (line 1860):**
Replace with a test asserting `MIKA_ARCH.llm_overrides.is_empty()`. Rename to `test_mika_arch_has_no_llm_overrides`.

**b. `test_seed_skill_overrides_mika_arch` (line 1948):**
Update to assert 0 override rows after seeding (since both `disabled_skills` and `llm_overrides` are now empty, the fast-path exit fires).

**c. `test_seed_skill_overrides_reconciles_drifted_llm_override` (line 1981):**
Repurpose as `test_seed_skill_overrides_cleans_stale_llm_overrides`. Pre-seed 3 LLM override rows, call `seed_well_known_skill_overrides`, assert 0 rows remain.

**d. `test_seed_skill_overrides_reconciliation_is_idempotent` (line 2035):**
Update to verify that calling seed twice on a clean DB is a no-op (0 rows both times).

**e. Add new test: `test_cleanup_preserves_operator_disabled_rows`:**
Pre-seed an LLM override row that also has `enabled = Some(false)`. Call `seed_well_known_skill_overrides`. Assert the row still exists with `enabled = Some(false)` and the LLM fields cleared.

## Verification

1. `cargo test -p mika-agent -- well_known_agents` — all tests pass.
2. `cargo clippy -p mika-agent` — no warnings.
3. After deploy + restart:
   - `sqlite3 ~/.mika/data/mika.db "SELECT * FROM skill_overrides WHERE agent_id='mika-arch'"` returns 0 rows.
   - `mika skills --agent mika-arch list` shows all three skills with `always_on=true` and no `[llm: ...]` annotation.
   - LLM call telemetry for mika-arch review work shows the agent default model (Opus 4.7).

## Risks

- **Low:** Removing overrides means mika-arch review quality depends on the agent default model (Opus 4.7, which is higher-quality than the Sonnet 4.6 overrides being removed). This is the operator's intent.
- **Low:** The `always_on` change means all three skills are always matched on every mika-arch turn. Since mika-arch is single-purpose (only does review), this is correct — every turn should have the review skill active.
- **Medium:** The stale-override cleanup in `seed_well_known_skill_overrides` must not delete operator-set `enabled=false` rows. The plan addresses this with an explicit `enabled` field check.
