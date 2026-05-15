# Plan: Migrate mika-dev/mika-qa/mika-relay to identity allowlist (D2 cross-cutting)

- **Issue:** senara-solutions/mika#815
- **Type:** feat
- **Branch:** `feat/well-known-agents-d2-migration-identity-allowlist`
- **Parent:** senara-solutions/mika-platform#51 (mika-arch v1 — final cleanup item)
- **Predecessor:** senara-solutions/mika#813 (introduced identity-allowlist for mika-arch)

## Problem

PR #813 introduced the `[skills].allowlist` identity-driven mechanism. mika-arch uses it cleanly — zero `skill_overrides` rows. The other three well-known agents (mika-dev, mika-qa, mika-relay) still use the `disabled_skills` denylist constant + `seed_well_known_skill_overrides()` DB rows pattern. This splits "what an agent is" between compile-time Rust constants and runtime DB writes — the SOC violation that #813 fixed for mika-arch.

## Approach

Mirror the mika-arch pattern: each agent gets an allowlist in its identity.toml, the `disabled_skills` constant becomes empty, and a one-time migration deletes the now-dead `skill_overrides` rows.

### Key design decisions

**D1: Static vs Computed identity.** mika-dev and mika-qa need `[kg].enabled = false` in their identity — both values are compile-time constants. mika-relay needs only `permission-policy` in its allowlist — also a constant. None of these agents require runtime config resolution (unlike mika-arch's `MIKA_KG_DOCS_ROOTS`). **Decision: use `IdentitySource::Static` for all three.** Each gets a const `&str` identity template with the `[skills].allowlist` section baked in.

**D2: Allowlist computation.** The allowlist for each agent is the complement of its current `disabled_skills` against the full set of bundled + community skills. Computed from the current source:

| Agent | Disabled count | Allowlist count | Allowlist |
|-------|---------------|-----------------|-----------|
| mika-dev | 7 | 25 | self-dev, self-dev-callback, self-dev-iterate, self-dev-webhook-qa, self-dev-webhook-ci, self-dev-webhook-ready-label, dev-pilot, build-mika, deploy-mika, permission-policy, agents-teams, address-pr-comments, resolve-pr-conflicts, self-check, dev-handsoff, tmux, shell-exec, web-search, file-reader, self-knowledge, git-ops, google-workspace, github, mcp, browser-control |
| mika-qa | 15 | 17 | qa-review, qa-review-build-callback, skill-review, build-mika, deploy-mika, self-check, dev-handsoff, tmux, shell-exec, web-search, file-reader, self-knowledge, git-ops, google-workspace, github, mcp, browser-control |
| mika-relay | 31 | 1 | permission-policy |

**D3: Denylist vs allowlist for future skill additions.** With the allowlist pattern, new bundled skills are automatically denied unless explicitly added to an agent's allowlist. This is the correct default for well-known agents — new skills should be consciously assigned, not silently inherited. For mika-relay (1-skill allowlist), this is essential. For mika-dev and mika-qa, it means new engine-coupled skills must be added to their identity templates as part of the skill's PR. Document this in the "Adding a New Bundled Skill" section of `CLAUDE.md`.

**D4: Migration scope.** Delete ALL `skill_overrides` rows for the three agents, not just `enabled=false` rows. These agents have zero `llm_overrides`, so no LLM override rows exist. If an operator has manually set overrides, the identity allowlist now governs — the operator can re-add manual overrides post-migration via `mika skills llm set`. The migration is scoped to `agent_id IN ('mika-dev', 'mika-qa', 'mika-relay')` — user-defined agents are untouched.

**D5: Migration location.** Run the migration inside `seed_well_known_skill_overrides()` itself — not as a schema version bump. The migration is behavioral (data cleanup), not structural (DDL). Use a `schema_meta` marker (`well_known_d2_migration_v1`) to guard idempotency.

**D6: `seed_well_known_skill_overrides()` changes.** After the migration, agents with empty `disabled_skills` AND empty `llm_overrides` take a fast-path exit — no rows to seed, no reconciliation needed. The function remains for agents that still use it (future well-known agents could use denylist if appropriate). Do NOT delete the function.

## Implementation units

### Unit 1: Add `[skills].allowlist` to identity templates

**File:** `crates/mika-agent/src/well_known_agents.rs`

1. Replace `MIKA_DEV_IDENTITY` const with an expanded template that includes `[skills].allowlist`:
   ```rust
   const MIKA_DEV_IDENTITY: &str = "\
   name = \"Dev\"\n\
   emoji = \"🛠\"\n\
   \n\
   [kg]\n\
   enabled = false\n\
   \n\
   [skills]\n\
   allowlist = [\n\
     \"self-dev\",\n\
     \"self-dev-callback\",\n\
     // ... (25 skills total)\n\
   ]\n";
   ```

2. Similarly expand `MIKA_QA_IDENTITY` with its 17-skill allowlist.

3. Add a new `MIKA_RELAY_IDENTITY` const with `[skills].allowlist = ["permission-policy"]` and update `MIKA_RELAY` to use `identity_source: Some(IdentitySource::Static(MIKA_RELAY_IDENTITY))`.

4. Set `disabled_skills: &[]` on all three agent specs.

### Unit 2: One-time migration to delete stale `skill_overrides` rows

**File:** `crates/mika-agent/src/well_known_agents.rs`

In `seed_well_known_skill_overrides()`, before the existing logic:

1. Check `schema_meta` for marker `well_known_d2_migration_v1`.
2. If absent:
   a. `DELETE FROM skill_overrides WHERE agent_id IN ('mika-dev', 'mika-qa', 'mika-relay')` — wrapping the three deletes in a transaction.
   b. Insert `well_known_d2_migration_v1 = '1'` into `schema_meta`.
   c. Log `info!` with the count of deleted rows.
3. If present: no-op (migration already ran).

The function already has `&mut Database` access, so the DB operations are straightforward.

**Fast-path exit:** After the migration check, if `spec.disabled_skills.is_empty() && spec.llm_overrides.is_empty()`, return early — nothing to seed.

### Unit 3: Update CLAUDE.md documentation

**File:** `CLAUDE.md` (root) — update the "Adding a New Bundled Skill" section to note that new skills must be explicitly added to well-known agent identity templates (allowlist pattern). Mention mika-relay's restrictive 1-skill allowlist as the exemplar.

**File:** `crates/mika-agent/CLAUDE.md` — update the Skills System section's identity-driven allowlist documentation to reflect that all four well-known agents now use the pattern (not just mika-arch).

### Unit 4: Tests

**File:** `crates/mika-agent/src/well_known_agents.rs` (inline `#[cfg(test)]`)

1. **Allowlist coverage test:** For each of the three migrated agents, verify that `disabled_skills` is empty and that `render_identity_content()` produces TOML with a `[skills].allowlist` entry. Parse the TOML and assert the allowlist length matches the expected count (25, 17, 1).

2. **Migration idempotency test:** Seed overrides for a test agent, run the migration, verify rows deleted. Run again, verify no error (idempotent via marker).

3. **No rows after migration test:** After migration, `db.get_skill_overrides("mika-dev")` returns empty (or only LLM override rows if any exist in the future).

4. **User-defined agents untouched test:** Create a user-defined agent with `skill_overrides` rows, run the migration, verify rows are preserved.

## Verification

After deploy + restart:

1. `SELECT agent_id, COUNT(*) FROM skill_overrides WHERE agent_id IN ('mika-dev','mika-qa','mika-relay','mika-arch') GROUP BY agent_id` — returns 0 rows for all four agents.
2. `SELECT key, value FROM schema_meta WHERE key = 'well_known_d2_migration_v1'` — returns `('well_known_d2_migration_v1', '1')`.
3. `mika skills list --agent mika-dev` — shows the same effective skill set as before.
4. `mika skills list --agent mika-relay` — shows only `permission-policy`.
5. Subsequent restarts do not re-run the deletion (idempotency via schema_meta marker).

## Risk assessment

**Low risk.** The identity allowlist mechanism is already proven on mika-arch (#813). The migration only deletes redundant state — the allowlist already governs skill visibility. `apply_identity_allowlist()` runs at Phase -1, before `apply_overrides()` at Phase 0, so even if the migration fails to delete rows, the allowlist takes precedence and the agent's effective skill set is unchanged.

**Rollback:** Revert the Rust changes and redeploy. The `seed_well_known_skill_overrides()` reconciliation path will re-seed the `enabled=false` rows on the next restart (it already handles this drift case via mika#1041). The `schema_meta` marker stays in the DB but is harmless (no code reads it after migration).

## Out of scope

- User-defined agents' `skill_overrides` rows (table stays, mechanism stays for non-well-known agents).
- Schema removal of `skill_overrides` table (premature — user-defined agents depend on it).
- `[tools].disabled` for mika-dev/mika-qa/mika-relay (separate concern; currently only mika-arch uses tool denylist).
