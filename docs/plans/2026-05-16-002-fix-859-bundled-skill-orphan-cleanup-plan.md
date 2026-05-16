---
title: Prune orphaned bundled-skill directories via a one-shot known-removed list
ticket: mika#859
type: fix
status: draft
created: 2026-05-16
revision: 2 (post mika-arch first-pass ITERATE — F1/F2/F3)
---

# Plan: Prune orphaned bundled-skill directories (mika#859)

> **Frame correction (load-bearing).** mika#859 attributes the persistent
> `skill:claude-pilot` row to a missing GC pass in
> `crates/mika-agent/src/kg/domain_builder.rs`. That attribution is empirically
> incorrect — the GC pass already exists (P0.1), is covered by a passing test
> (P0.7), and is documented in `crates/mika-agent/CLAUDE.md` § Knowledge Graph —
> Domain Graph Builder. The reason `skill:claude-pilot` persists is that the
> bundled-skill seeder (`crates/mika-agent/src/bundled_skills.rs:267`) is
> additive-only and leaves orphaned per-agent directories on disk after a
> rename or removal. The skill registry re-enumerates them every boot, the KG
> correctly mirrors the registry, and the existing GC correctly preserves them
> (they're in the desired-key set).
>
> Fix therefore belongs at the seeder, not at the KG layer. After the prune
> lands, the existing rebuild self-heals on the next server boot — exercised
> by the existing `rebuild_deletes_removed_entities` test (P0.7).

## TL;DR

`seed_bundled_skills()` writes every current bundled skill but never walks
`skills/` to find directories that used to be bundled and aren't anymore.
mika#853 renamed `claude-pilot` → `dev-pilot` + `dev-groom` in the bundle
source, but 12 stale `~/.mika/agents/<name>/skills/claude-pilot/` directories
survive on the live host (mtime 2026-04-28, the mika#853 deploy date,
verified 2026-05-16). Each agent's `SkillRegistry::scan()` keeps reporting
them; the KG correctly mirrors them; the existing GC correctly preserves them.

Minimal fix: a single `KNOWN_REMOVED_BUNDLED_SKILLS: &[&str] = &["claude-pilot"]`
const + a `prune_known_removed_bundled_skills(skills_dir)` helper called at
the top of `seed_bundled_skills()`. Each future PR that removes or renames a
bundled skill adds the OLD name to that list in the same commit. The
directories with a name matching the list (case-insensitive, non-symlink,
non-`_`-prefixed) are `remove_dir_all`'d before the seed loop runs. The KG
self-heals on the next server boot via the existing rebuild path — no KG
code changes, no schema migration, no marker file.

A more general "marker file at every bundled-skill seed" design was drafted
in revision 1 and rejected by mika-arch first-pass as YAGNI for a p2-normal
incident given the operator's `feedback_keep_simple.md` preference. That
shape will be filed as a follow-up ticket and revisited the second time this
class of orphan appears.

## Phase 0 — Pin (load-bearing source slices)

**Base commit:** `31c1b0a5c12db57e6898c341fdeb3d7adad2c4d9` (worktree
`fix-859-kg-domain-builder-leaves-orphan-kg/mika`, 2026-05-16).

### P0.1 — Existing KG GC pass

`crates/mika-agent/src/kg/domain_builder.rs:386–426`:

```rust
                // 2c. DELETE entities no longer in sources.
                // The type filter enforces the sole-writer contract at the SQL level.
                let type_placeholders: String = KG_DOMAIN_ENTITY_TYPES
                    .iter()
                    .map(|_| "?")
                    .collect::<Vec<_>>()
                    .join(", ");

                // Build the NOT IN clause for desired keys
                let key_placeholders: String = if desired.entity_keys.is_empty() {
                    // No desired keys — delete everything in domain types
                    String::new()
                } else {
                    let placeholders: String = desired
                        .entity_keys
                        .iter()
                        .enumerate()
                        .map(|(i, _)| format!("?{}", i + KG_DOMAIN_ENTITY_TYPES.len() + 1))
                        .collect::<Vec<_>>()
                        .join(", ");
                    format!(" AND entity_key NOT IN ({placeholders})")
                };

                let delete_sql = format!(
                    "DELETE FROM kg_entities WHERE type IN ({type_placeholders}){key_placeholders}"
                );

                let removed = {
                    let mut stmt = tx.prepare(&delete_sql)?;
                    let mut param_idx = 1;
                    for t in KG_DOMAIN_ENTITY_TYPES {
                        stmt.raw_bind_parameter(param_idx, *t)?;
                        param_idx += 1;
                    }
                    for key in &desired.entity_keys {
                        stmt.raw_bind_parameter(param_idx, key.as_str())?;
                        param_idx += 1;
                    }
                    stmt.raw_execute()?
                };
```

### P0.2 — KG domain types include `"skill"`

`crates/mika-agent/src/db/kg_schema.rs:186`:

```rust
pub const KG_DOMAIN_ENTITY_TYPES: &[&str] = &["skill", "tool", "agent", "problem_type", "concept"];
```

### P0.3 — Bundled-skill seeder body (additive-only)

`crates/mika-agent/src/bundled_skills.rs:267–289`:

```rust
pub fn seed_bundled_skills(skills_dir: &Path) {
    // Support directories are also seeded unconditionally from startup.rs
    // (before the disabled guard). This call ensures seed_bundled_skills()
    // is self-contained when called directly (e.g., by create_agent tool
    // or tests). The second write on the normal startup path is idempotent.
    seed_support_dirs(skills_dir);

    for skill in all_bundled_skills() {
        let skill_dir = skills_dir.join(skill.name);
        let is_update = skill_dir.exists();

        if let Err(e) = write_skill(&skill_dir, skill) {
            warn!(skill = skill.name, error = %e, "failed to seed bundled skill");
            if !is_update {
                let _ = std::fs::remove_dir_all(&skill_dir);
            }
        } else if is_update {
            debug!(skill = skill.name, "updated bundled skill");
        } else {
            info!(skill = skill.name, "seeded bundled skill");
        }
    }
}
```

### P0.4 — `is_bundled_skill()` reflects current bundle (post-rename)

`crates/mika-agent/src/bundled_skills.rs:205–212`:

```rust
pub fn is_bundled_skill(name: &str) -> bool {
    BUNDLED_SKILLS
        .iter()
        .any(|s| s.name.eq_ignore_ascii_case(name))
        || ENTRIES.iter().any(|s| s.name.eq_ignore_ascii_case(name))
}
```

`claude-pilot` is in neither `BUNDLED_SKILLS` nor `ENTRIES` post-mika#853.

### P0.5 — `seed_bundled_skills` production call sites (rebuild coupling)

```
$ grep -rn "seed_bundled_skills_if_needed\|seed_bundled_skills(" crates --include="*.rs" \
    | grep -v "_test\|tests" | sort -u
```

Production callers (non-test) of `seed_bundled_skills_if_needed`:

1. `crates/mika-agent/src/server/mod.rs:383` — server boot, per-agent init.
   Followed by `DomainGraphBuilder::rebuild()` at line 780 (after all agents
   initialise). **KG self-heals on this path.**
2. `crates/mika-agent/src/tools/create_agent.rs:119` — `create_agent` tool
   call during a running agent turn. **No `rebuild()` follows in the same
   process**, but the new agent's `skills/` directory is freshly created
   here — it cannot contain any orphan directories from a previous
   deploy. The KNOWN_REMOVED prune is a no-op on this path, and there is
   no consistency window to defend.
3. `crates/mika-cli/src/init.rs:68` — `mika init` CLI (one-time agent home
   creation). **No `rebuild()` follows** (CLI doesn't touch the KG). On
   the next server boot, both prune and rebuild run; consistency is
   restored at server boot.
4. `crates/mika-cli/src/commands/agents.rs:114` — `mika agents` CLI
   subcommand. Same posture as (3): CLI doesn't touch the KG; server boot
   reconciles.

**F3 finding (paths 3 + 4):** the only path where a prune fires without a
KG row delete is the CLI. The window of inconsistency is "CLI prunes
directory" → "operator restarts server" → "rebuild deletes the row." This
matches the existing `make deploy` flow (build → install → restart) and is
operator-visible only via `mika kg status` queries between the two events.
Acceptable; not gating the fix.

`server/mod.rs` rebuild call site, `crates/mika-agent/src/server/mod.rs:773–795`:

```rust
        let builder = crate::kg::domain_builder::DomainGraphBuilder::new(
            &dashboard_db,
            &skill_reg,
            &tool_registry,
            default_state.mcp_manager.as_ref(),
            &agent_infos,
        );
        match builder.rebuild().await {
            Ok(stats) => info!(
                event = "domain_rebuild_complete",
                added = stats.entities_added,
                updated = stats.entities_updated,
                removed = stats.entities_removed,
                depends_on = stats.edges_depends_on,
                provides = stats.edges_provides,
                duration_ms = stats.duration_ms,
                "domain graph ready"
            ),
            …
        }
```

### P0.6 — Marketplace install path does NOT call `write_skill`

`crates/mika-agent/src/skills/install.rs:282–331` shows three public entry
points (`install_skill`, `install_skill_linked`, `install_skill_inner`)
that copy directly via `std::fs::copy` (line 635) and never invoke
`write_skill`. Therefore the bundled-skill prune predicate (membership in
`KNOWN_REMOVED_BUNDLED_SKILLS`) is the only signal — marketplace skills
can never accidentally appear in that const, so the prune cannot wipe user
state by construction. (The marker-file branch from revision 1 also held
this guarantee, but the const-only design holds it more obviously: the
discriminator is a literal allowlist of names we know we removed.)

### P0.7 — Existing test exercises the KG self-heal

`crates/mika-agent/src/kg/domain_builder.rs:1114–1175`:

```rust
async fn rebuild_deletes_removed_entities() {
    let db = make_async_db();

    // First rebuild with skill X that provides tool_x
    let skill_x = make_skill("skill-x", "X", vec![], vec!["tool_x"]);
    let registry1 = SkillRegistry::from_test_entries(vec![skill_x]);
    let tool_registry = make_tool_registry(&[]);

    let builder1 = DomainGraphBuilder::new(&db, &registry1, &tool_registry, None, &[]);
    builder1.rebuild().await.expect("first rebuild");

    // Verify edge exists after first rebuild
    let edge_count_before: usize = …;
    assert_eq!(edge_count_before, 1);

    // Second rebuild WITHOUT skill X
    let registry2 = SkillRegistry::empty();
    let builder2 = DomainGraphBuilder::new(&db, &registry2, &tool_registry, None, &[]);
    let stats2 = builder2.rebuild().await.expect("second rebuild");

    // skill-x entity should be removed
    assert!(stats2.entities_removed > 0);

    // Verify entity is gone from DB
    let remaining = …;
    assert!(remaining.is_empty());

    // Verify CASCADE removed edges touching deleted entity
    let edge_count_after: usize = …;
    assert_eq!(edge_count_after, 0);
}
```

### P0.8 — Empirical evidence (live host, 2026-05-16)

```
$ sqlite3 ~/.mika/data/mika.db \
    "SELECT entity_key, datetime(updated_at), datetime(created_at)
     FROM kg_entities WHERE entity_key = 'skill:claude-pilot';"
skill:claude-pilot|2026-05-16 15:50:55|2026-04-22 10:29:36
```

Fresh `updated_at` proves the entity is being UPSERTed every boot — the
registry still reports `claude-pilot` as a skill.

```
$ find ~/.mika/agents -maxdepth 3 -type d -name claude-pilot
/home/samidarko/.mika/agents/steve-jobs/skills/claude-pilot
/home/samidarko/.mika/agents/mika-dev/skills/claude-pilot
/home/samidarko/.mika/agents/mika/skills/claude-pilot
/home/samidarko/.mika/agents/chase-hughes/skills/claude-pilot
/home/samidarko/.mika/agents/elon-musk/skills/claude-pilot
/home/samidarko/.mika/agents/odds-engine-ceo/skills/claude-pilot
/home/samidarko/.mika/agents/odds-engine-cto/skills/claude-pilot
/home/samidarko/.mika/agents/odds-engine-quant/skills/claude-pilot
/home/samidarko/.mika/agents/mika-test/skills/claude-pilot
/home/samidarko/.mika/agents/mika-arch/skills/claude-pilot
/home/samidarko/.mika/agents/mika-qa/skills/claude-pilot
/home/samidarko/.mika/agents/mika-relay/skills/claude-pilot
```

12 surviving stale directories — one per agent. File mtimes inside one:

```
$ ls -la ~/.mika/agents/mika-dev/skills/claude-pilot/
drwxr-xr-x  handlers/
.rw-r--r--  skill.toml         (mtime 2026-04-28T12:27:06 — mika#853 deploy)
.rw-r--r--  system_prompt.md
.rw-r--r--  tools.json
```

## Phase 1 — The change

### 1.1 Add `KNOWN_REMOVED_BUNDLED_SKILLS` const

In `crates/mika-agent/src/bundled_skills.rs`, near the existing
`TRUST_CRITICAL_SKILLS` const:

```rust
/// Skill names that were once bundled but have been renamed or removed,
/// listed here so the prune pass can clean up their per-agent directories
/// on hosts that were deployed before the rename or removal.
///
/// **Update procedure:** any future PR that renames or removes a bundled
/// skill MUST add the OLD name here in the same commit. Stale entries are
/// harmless (the directories no longer exist on previously-cleaned hosts).
/// Entries can be removed in a later cleanup PR once every production host
/// is confirmed to have rebooted since the rename/removal landed.
///
/// **Marketplace-safety invariant:** entries in this list will be
/// `remove_dir_all`'d from every agent's `skills/` dir. Names added here
/// must be names that were definitively bundled in a prior release.
/// Adding the name of a marketplace skill here would wipe user state.
pub(crate) const KNOWN_REMOVED_BUNDLED_SKILLS: &[&str] = &[
    // claude-pilot was renamed to dev-pilot + dev-groom in mika#853
    // (deployed 2026-04-28). 12 stale per-agent directories survived
    // the rename — see mika#859 § Phase 0 P0.8 for verification output.
    "claude-pilot",
];
```

### 1.2 Add `prune_known_removed_bundled_skills(skills_dir)` helper

```rust
/// Remove per-agent skill directories whose name appears in
/// [`KNOWN_REMOVED_BUNDLED_SKILLS`]. Called once at the top of
/// [`seed_bundled_skills`].
///
/// Defense-in-depth (matches `write_skill`):
/// - Symlinks are never followed and never removed.
/// - `_`-prefixed support directories (e.g. `_shared/`) are skipped.
/// - `.`-prefixed entries (e.g. `.mika-bundled` markers in a future
///   revision) are skipped.
/// - I/O errors are logged and skipped — a single bad directory must not
///   block the seed.
///
/// Returns the number of directories actually removed.
pub(crate) fn prune_known_removed_bundled_skills(skills_dir: &Path) -> usize {
    let entries = match std::fs::read_dir(skills_dir) {
        Ok(e) => e,
        Err(e) => {
            warn!(error = %e, "failed to read skills_dir for known-removed prune");
            return 0;
        }
    };

    let mut pruned = 0;
    for entry in entries.flatten() {
        let path = entry.path();

        let meta = match entry.file_type() {
            Ok(m) => m,
            Err(_) => continue,
        };
        if meta.is_symlink() || !meta.is_dir() {
            continue;
        }

        let name = match path.file_name().and_then(|s| s.to_str()) {
            Some(n) => n,
            None => continue,
        };

        if name.starts_with('_') || name.starts_with('.') {
            continue;
        }

        let is_known_removed = KNOWN_REMOVED_BUNDLED_SKILLS
            .iter()
            .any(|s| s.eq_ignore_ascii_case(name));
        if !is_known_removed {
            continue;
        }

        match std::fs::remove_dir_all(&path) {
            Ok(()) => {
                info!(
                    skill = name,
                    "pruned orphaned bundled-skill directory (known-removed list)"
                );
                pruned += 1;
            }
            Err(e) => {
                warn!(
                    skill = name,
                    error = %e,
                    "failed to prune orphaned bundled-skill directory"
                );
            }
        }
    }
    pruned
}
```

### 1.3 Wire the prune into `seed_bundled_skills`

```rust
pub fn seed_bundled_skills(skills_dir: &Path) {
    seed_support_dirs(skills_dir);

    // Prune known-removed orphans BEFORE the write loop. Order matters:
    // if a future PR ever reuses a removed name, we want the prune to
    // happen first so the subsequent write_skill is a clean create rather
    // than an update over stale content.
    let pruned = prune_known_removed_bundled_skills(skills_dir);
    if pruned > 0 {
        info!(count = pruned, "pruned known-removed bundled-skill directories");
    }

    for skill in all_bundled_skills() {
        // … existing write loop, unchanged
    }
}
```

### 1.4 KG side — NO change

The existing `domain_builder::rebuild()` (P0.1, P0.2) covers `skill:*`
entities under `KG_DOMAIN_ENTITY_TYPES`. The existing test
`rebuild_deletes_removed_entities` (P0.7) covers the contract:
registry-removes-skill → next-rebuild-deletes-entity. Once Phase 1 lands
and the server next boots, that test's flow runs in production and
deletes `skill:claude-pilot` from `kg_entities`.

No new KG code, no new KG test.

## Phase 2 — Tests

All in `crates/mika-agent/src/bundled_skills.rs mod tests` using the
existing `tempfile::tempdir` pattern.

### T1 — `prune_removes_directory_in_known_removed_list`

Setup: tempdir with `skills/claude-pilot/skill.toml`. Call
`prune_known_removed_bundled_skills`. Assert: directory removed, return
value = 1.

### T2 — `prune_is_case_insensitive`

Setup: tempdir with `skills/Claude-Pilot/skill.toml`. Call prune. Assert:
directory removed, return value = 1. Mirrors the `eq_ignore_ascii_case`
predicate.

### T3 — `prune_preserves_directory_not_in_known_removed_list`

Setup: tempdir with `skills/my-custom-skill/skill.toml`. Call prune.
Assert: directory preserved, return value = 0. **This is the
marketplace-safety regression guard.**

### T4 — `prune_preserves_symlinked_skill_dir`

Setup: tempdir with `skills/claude-pilot -> /tmp/real-target/` (real
target is named so it would match by name). Call prune. Assert: symlink
preserved, real-target preserved. Mirrors `write_skill`'s existing
symlink defense.

### T5 — `prune_preserves_support_directories`

Setup: tempdir with `skills/_shared/dispatch-lib.sh` (somehow this dir
got a name that happens to match a known-removed; defense-in-depth even
though `_`-prefix → not a skill). Call prune. Assert: `_shared/`
preserved.

### T6 — `prune_preserves_current_bundled_skill_collision`

Setup: tempdir with `skills/dev-pilot/skill.toml` (currently bundled,
NOT in `KNOWN_REMOVED_BUNDLED_SKILLS`). Call prune. Assert: directory
preserved, return value = 0. Guards against accidental list overlap with
the current bundle.

### T7 — `prune_is_idempotent`

Call `prune_known_removed_bundled_skills` twice on the same tempdir.
Second call returns 0 and is a no-op.

### T8 — `seed_bundled_skills_prunes_before_seeding`

End-to-end: tempdir with `skills/claude-pilot/skill.toml`. Call
`seed_bundled_skills`. Assert: `skills/claude-pilot/` is gone,
`skills/dev-pilot/` exists, `skills/dev-groom/` exists. Verifies the
prune-before-seed ordering and the full incident fix end-to-end.

## Phase 3 — Acceptance

Unit:

- `cargo test -p mika-agent bundled_skills` — T1–T8 pass.
- `cargo test -p mika-agent kg::domain_builder::tests::rebuild_deletes_removed_entities`
  continues to pass unchanged (P0.7).
- `cargo clippy -p mika-agent --all-targets -- -D warnings` clean.
- `cargo fmt --check`.

Manual (operator-side, post-deploy of this PR):

1. Server restarts cleanly (`make deploy`).
2. `find ~/.mika/agents -maxdepth 3 -type d -name claude-pilot` returns
   zero results.
3. After server boot completes:
   `sqlite3 ~/.mika/data/mika.db "SELECT entity_key FROM kg_entities WHERE entity_key = 'skill:claude-pilot'"`
   returns zero rows.
4. `mika kg status --agent mika-arch` does not list `claude-pilot`.
5. Sanity sweep — list any other skill entities that are not in the
   current bundle:
   `sqlite3 ~/.mika/data/mika.db "SELECT entity_key FROM kg_entities WHERE entity_key LIKE 'skill:%' ORDER BY entity_key"`
   — diff against current bundle; expected to be either currently
   bundled, currently marketplace-installed, or `claude-pilot` (which
   should be absent after this PR's deploy). If any other orphan
   surfaces, add it to `KNOWN_REMOVED_BUNDLED_SKILLS` in a follow-up.

## Out of scope (deferred follow-ups)

- **Marker-file design (revision 1's Part A).** A `.mika-bundled` marker
  written by `write_skill` and a more general `prune_orphaned_bundled_skills`
  helper that uses the marker as the discriminator. Rejected at first-pass
  architect review as YAGNI for a p2-normal incident. File as a separate
  ticket the next time this orphan class appears (incident #2) or proactively
  if the operator decides the recurrence cost outweighs the simplicity. The
  revision 1 plan body in the git history of this branch serves as the
  design brief.
- **Marketplace-skill GC.** A user uninstalling a marketplace skill runs
  through `mika skills uninstall` and `marketplace.lock` — distinct flow.
- **Subject-graph cascade verification.** Existing FK cascade in the v25
  KG schema already covers `kg_subject_resolutions` rows pointing at
  domain entities deleted by `rebuild()`. The existing
  `rebuild_deletes_removed_entities` test exercises the cascade ("Verify
  CASCADE removed edges touching deleted entity") and no behaviour change
  is in scope here.
- **`KNOWN_REMOVED_BUNDLED_SKILLS` cleanup.** Entries can be removed in a
  later PR once every production host has rebooted since the entry's
  associated rename. Tracking that confidence is not in scope for this PR.

## Risk and reversibility

- **Pruning happens at the very top of `seed_bundled_skills`, before any
  writes.** A bug that mis-identifies a marketplace skill would delete
  user state. The marketplace-safety guarantee rests on `KNOWN_REMOVED_BUNDLED_SKILLS`
  being an explicit allowlist of names we know we removed — adding a
  marketplace name there is an obvious developer error that code review
  catches. T3 is the regression test.
- **Typo in `KNOWN_REMOVED_BUNDLED_SKILLS`.** A typo (e.g.
  `"clauude-pilot"`) is silently inert — it just fails to prune the real
  orphan. The inverse direction (typo deletes a user skill) requires the
  typo to match an actual on-disk marketplace directory name, which is
  extremely unlikely.
- **`MIKA_DISABLE_BUNDLED_SKILLS=true` interaction.** That flag skips
  `seed_bundled_skills()` entirely (`startup.rs:48–68`). The prune is
  inside `seed_bundled_skills()`, so the disable flag also skips the
  prune — same posture as the rest of the seeder. Hosts running with the
  flag will retain orphan directories until they re-enable. Acceptable
  per the flag's documented purpose (hot-patch during dev only).
- **Rollback:** `git revert` this PR. The deleted orphan directories
  don't re-appear, but no user state is lost (we only deleted
  marketplace-safe known-removed names). Worst case is operator re-runs
  `make deploy` on a previous tag.
- **Test cost:** 8 tests, all using `tempfile::tempdir`. No new fixtures
  or test infrastructure. Estimated +~140 lines under `mod tests`.

## Architect-decision provenance

This plan's revision 1 (marker-file + KNOWN_REMOVED) was reviewed by
mika-arch first-pass on 2026-05-16, session `e344299b-e79c-46df-8c24-9ee0718237e3`.
Disposition: ITERATE. Three BLOCKING findings:

- **F1** — Phase 0 Pin lacked base commit SHA and complete verbatim for
  load-bearing sites. Addressed in revision 2 Phase 0 (base SHA at top,
  8 numbered slices, every load-bearing claim has verbatim source).
- **F2** — Part A (marker file) was a YAGNI violation at p2-normal given
  `feedback_keep_simple.md`. Addressed in revision 2 by dropping Part A
  entirely; marker design moved to the "Out of scope (deferred
  follow-ups)" section.
- **F3** — Rebuild-after-seed coupling was asserted but unverified.
  Addressed in revision 2 P0.5 — all four production `seed_bundled_skills_if_needed`
  call sites enumerated; only one (`server/mod.rs:383`) is followed by a
  `rebuild()` in the same process. The other three are CLI/tool paths
  where either no orphans can exist (`create_agent.rs:119` — fresh dir)
  or the next server boot reconciles (`init.rs:68`, `agents.rs:114`).
  No production correctness gap.

Remaining uncertainties for second pass:

- U1: With Part A removed, the only orphan-class we defend against is
  "name was bundled, now isn't." If a future rename also happens to be a
  *rename* (claude-pilot → dev-pilot), the OLD name must be added to the
  const list in the same PR. There is no engine-side guard that catches
  "the rename PR forgot to update KNOWN_REMOVED." A test that ensures
  `KNOWN_REMOVED_BUNDLED_SKILLS` entries are not in the current bundle
  could be added; the inverse ("every rename PR added its OLD name to
  this const") is unenforceable without external tooling. T6 partially
  covers the first half. Worth adding a `const_known_removed_disjoint_from_bundle`
  invariant test, or out of scope?
- U2: T8 ("seed_bundled_skills_prunes_before_seeding") asserts ordering
  by checking the end state. The seeder is not currently ordering-tested
  in any other way. Is the end-state assertion sufficient, or should the
  test thread some kind of sentinel that records observed call order?
  My lean: end-state is fine for this scope — adding a call-order
  sentinel would require infrastructure not present elsewhere in the
  module.
