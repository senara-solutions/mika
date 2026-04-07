---
title: "feat: Per-agent skill LLM override via DB + full linked skill review/refinement"
type: feat
status: active
date: 2026-04-07
issue: senara-solutions/mika#474
---

# feat: Per-agent skill LLM override via DB + full linked skill review/refinement

## Context

Two orthogonal frictions in skill operation today, fixed in one cohesive change:

1. **LLM override friction.** The only way to retune a skill's provider/model
   (e.g. switch `qa-review` from `deepseek` to `anthropic/claude-sonnet-4-6`) is
   editing `[llm]` in the skill's `skill.toml` and committing it. The manifest
   captures *author intent* and is the right place for marketplace defaults; it
   is the wrong place for a per-operator runtime decision. Add an operator
   override layer in the existing `skill_overrides` DB table — same pattern as
   `always_on` (schema v7).

2. **Linked skills blocked from review/refinement.** A skill installed with
   `--link` is a symlink to a live source directory. Today
   `review_skill` and `write_skill_variant` (`crates/mika-agent/src/skills/builtin_handlers.rs:991-996,1152-1156`)
   hard-refuse linked skills citing a "read-only invariant" — but the entire
   point of `--link` is mutable source. Replace the block with a structured
   warning so review/refinement flows through normally and writes land in the
   source tree.

This implements the design in
`/home/samidarko/Downloads/2026-04-07-feat-skill-llm-override-and-linked-review-plan.md`,
which has already been validated against the current codebase. Tracking issue:
senara-solutions/mika#474.

## Design Decisions

Adopted as-is from the source plan. Summarized for reviewer convenience:

- **D1.** Keep `[llm]` in `skill.toml` (author default); add DB as override layer.
  Resolution: **DB override > manifest `[llm]` > agent default**. Deprecating
  `[llm]` would lose author intent, install-time validation, and zero-config
  defaults for marketplace skills.
- **D2.** Routing for write operations is `linked + is_bundled`, not origin
  alone. Bundled/snapshot → DB UPSERT (file is overwritten by seed on restart).
  Linked marketplace + custom → write to source `skill.toml`.
- **D3.** Default-equals-delete: if effective override matches manifest `[llm]`
  (or both absent), delete the row. Prevents stale overrides blocking future
  skill updates. Mirrors `always_on` semantics.
- **D4.** `apply_overrides()` extends in place. `scan_skills_dir()` signature
  unchanged — callers already pass `&[SkillOverride]`.
- **D5.** Linked skills: warn, never block. Single structured `tracing::warn!`
  + tool-output annotation. The user explicitly opted into mutable-source
  semantics with `--link`.
- **D6.** `SkillEntry.has_override` already flips on any override row — naturally
  covers the new columns, no badge changes.
- **D7.** No new external crates.

## Schema Change (v19 → v20)

```sql
-- migration v20
ALTER TABLE skill_overrides ADD COLUMN llm_provider TEXT;  -- NULL = use manifest
ALTER TABLE skill_overrides ADD COLUMN llm_model    TEXT;  -- NULL = use manifest
```

Both columns nullable; `NULL` means "no override". `default-equals-delete`
already prevents fully-`NULL` orphan rows in practice.

`CURRENT_SCHEMA_VERSION` (`crates/mika-agent/src/db.rs:25`) bumps `19 → 20`.
Migration follows the same shape as the existing v17/v18/v19 `ALTER TABLE`
migrations.

## Resolution Chain

```
resolve_skill_llm(entry, db_override) -> Option<LlmOverride>
  1. DB row has (llm_provider, llm_model) → merge into entry.manifest.llm
  2. manifest.llm is Some(_)              → use manifest values
  3. otherwise                            → None (agent default applies)
```

`resolve_skill_llm_override()` (`crates/mika-agent/src/agent.rs:2427`) already
reads from `entry.manifest.llm`. Once `apply_overrides()` merges DB values into
`entry.manifest.llm`, **this function does not change**. This is the load-bearing
property that keeps the diff small.

## CLI Surface

Symmetric with `mika skills toggle`, in `crates/mika-cli/src/commands/skills.rs`:

```
mika skills llm <name> set <provider>/<model>
mika skills llm <name> reset
mika skills llm <name> show          # effective value + source (db/manifest/agent-default)
```

- `set` validates `provider/model` via `ModelSpec::parse()` (reuse existing
  factory) — fails loud on unknown provider before any DB write.
- `reset` clears the columns; if the resulting row is fully `NULL`, deletes it.
- `show` prints effective value with source annotation:
  `qa-review  llm: anthropic/claude-sonnet-4-6  [db-override]`.

## Phases

### Phase 1: DB schema migration

**File:** `crates/mika-agent/src/db.rs`

- Bump `CURRENT_SCHEMA_VERSION` to `20` (line 25).
- Add `migrate_v19_to_v20()` following the v17→v18 pattern; runs the two
  `ALTER TABLE` statements.
- Extend `SkillOverride` struct (line 398) with `llm_provider: Option<String>`,
  `llm_model: Option<String>`.
- Extend `get_skill_overrides()` (line 2509) `SELECT` to include the two new
  columns.
- Add `set_skill_llm_override(agent_id, skill_name, provider, model)` —
  UPSERT preserving `always_on` via `COALESCE`.
- Add `delete_skill_llm_override(agent_id, skill_name)` — sets both columns
  to `NULL`; if the resulting row has all-`NULL` overrides, deletes the row
  (D3).
- The existing `delete_skill_override()` (full row delete, line 2541) is
  unchanged — `delete_skill` and `mika skills uninstall` continue to work.
- Tests: extend the existing case-insensitive / per-agent / round-trip tests
  in the db test module (around line 8399) to cover LLM columns.

### Phase 2: `apply_overrides()` extension

**File:** `crates/mika-agent/src/skills/mod.rs:113`

Extend the existing loop:

```rust
if let Some(v) = ov.always_on { /* existing */ }
if ov.llm_provider.is_some() || ov.llm_model.is_some() {
    let existing = entry.manifest.llm.get_or_insert_default();
    if let Some(p) = &ov.llm_provider { existing.provider = Some(p.clone()); }
    if let Some(m) = &ov.llm_model    { existing.model    = Some(m.clone()); }
    entry.has_override = true;
}
```

Note: `LlmOverride` fields are already `Option<String>` (per #463), so use
`Some(p.clone())`. `get_or_insert_default()` requires `LlmOverride: Default`
— add `#[derive(Default)]` if not already present.

No callers change. Add unit tests under the existing `apply_overrides` test
suite (around line 428).

### Phase 3: `update_skill` tool routing

**File:** `crates/mika-agent/src/tools/update_skill.rs`

Add `llm_provider: Option<String>` and `llm_model: Option<String>` to the
input schema. Routing logic mirrors the existing `always_on` block:

```rust
if llm_provider.is_some() || llm_model.is_some() {
    let bundled = is_bundled_skill(&name);
    let linked  = is_linked_skill(&skill_dir);

    let manifest_llm = entry.manifest.llm.as_ref();
    let effective_p  = llm_provider.as_deref().or_else(|| manifest_llm.and_then(|l| l.provider.as_deref()));
    let effective_m  = llm_model.as_deref().or_else(|| manifest_llm.and_then(|l| l.model.as_deref()));

    if bundled || (!linked && is_marketplace_snapshot(&skill_dir)) {
        // D3: default-equals-delete
        let manifest_p = manifest_llm.and_then(|l| l.provider.as_deref());
        let manifest_m = manifest_llm.and_then(|l| l.model.as_deref());
        if effective_p == manifest_p && effective_m == manifest_m {
            ctx.db.delete_skill_llm_override(&ctx.agent_id, &name).await?;
        } else {
            ctx.db.set_skill_llm_override(&ctx.agent_id, &name,
                effective_p.unwrap_or(""), effective_m.unwrap_or("")).await?;
        }
    } else {
        // linked or custom → write to source skill.toml
        if linked { warn_linked_skill_write(&entry); }
        write_llm_to_skill_toml(&skill_dir, llm_provider.as_deref(), llm_model.as_deref())?;
    }
    ctx.skills_dirty.store(true, Ordering::Relaxed);
}
```

Helpers (private to this module unless reusable elsewhere is obvious):

- `is_linked_skill(dir)` — `std::fs::symlink_metadata(dir).map(|m| m.file_type().is_symlink())`.
- `is_marketplace_snapshot(dir)` — opposite of bundled+linked+custom; defer to
  marketplace lock if a helper already exists, otherwise inline.
- `write_llm_to_skill_toml(dir, p, m)` — read TOML, upsert `[llm]` table,
  write back. Reuse the existing TOML edit pattern from the `always_on`
  custom-skill path in the same file.
- `warn_linked_skill_write(entry)` — see Phase 6.

### Phase 4: `delete_skill` cleanup

**Files:** `crates/mika-agent/src/tools/delete_skill.rs`,
`crates/mika-cli/src/commands/skills.rs` (uninstall path).

**No code change.** `db.delete_skill_override()` already deletes the whole
row by PK; the new columns disappear with the row. Verified in plan source
section "Phase 4".

### Phase 5: CLI `mika skills llm` subcommand

**File:** `crates/mika-cli/src/commands/skills.rs`

Add a `Llm { name, action }` variant to `SkillsCommand` (parallel to `Toggle`)
and an inner `LlmAction { Set { model }, Reset, Show }` enum.

- `set`: parse via `ModelSpec::parse()`, open DB, call
  `set_skill_llm_override`, print confirmation including the previous value.
- `reset`: call `delete_skill_llm_override`, print effective post-reset value.
- `show`: load registry + overrides, print effective value with
  `[db-override]` / `[manifest]` / `[agent-default]` annotation.

`--format text|json` parity with the rest of the `skills` command family.

### Phase 6: Linked skill review/refinement unblock

**File:** `crates/mika-agent/src/skills/builtin_handlers.rs`

Two hard blocks today:

- `review_skill` linked refusal at lines `~991-996`.
- `write_skill_variant` linked refusal at lines `~1152-1156`.

Replace each with a single `warn_linked_skill_write()` call:

```rust
fn warn_linked_skill_write(skill_name: &str, path: &Path) {
    tracing::warn!(
        skill = %skill_name,
        path  = %path.display(),
        "[linked skill] Changes will be written to source directory"
    );
}
```

Plus a visible annotation in the tool output JSON so the agent (and user)
see it without scanning logs:

```json
{ "warning": "linked skill: changes written to <path>", ... }
```

No other changes — `tokio::fs::write` on a symlink path already writes through
to the source file (symlink transparency).

**Tests to update** (currently assert refusal, must be flipped to assert
success + warning):

- `test_review_skill_linked_skill_refused` (line 2177) →
  `test_review_skill_linked_skill_warns_and_succeeds`.
- `test_write_skill_variant_refuses_linked_skill` (line 2457) →
  `test_write_skill_variant_linked_skill_warns_and_writes_through`.

### Phase 7: Docs update

- `docs/skills.md` — document `[llm]`, override resolution chain, new CLI
  surface, linked skill write-back behavior.
- `docs/architecture.md` — extend `skill_overrides` ERD/description with the
  two new columns.
- `docs/adr/002-filesystem-based-skill-registry.md` — append decision: "LLM
  binding is a per-layer concern; manifest = author default, DB = operator
  runtime, agent settings = fallback."
- `docs/runtime-structure.md` — schema migration table: append v19 → v20.
- Run `scripts/sync-agent-docs.sh` after editing `docs/` so `crates/mika-agent/docs/`
  stays in sync (CI `docs-sync` job enforces this).

## Files to Change

| File | Change |
|---|---|
| `crates/mika-agent/src/db.rs` | Bump v20, migration, `SkillOverride` struct, 2 new CRUD methods, extend `get_skill_overrides` SELECT |
| `crates/mika-agent/src/skills/mod.rs` | `apply_overrides()` — merge LLM columns; add tests |
| `crates/mika-agent/src/skills/manifest.rs` | Ensure `LlmOverride: Default` (likely already true) |
| `crates/mika-agent/src/tools/update_skill.rs` | New `llm_provider`/`llm_model` params; routing helpers |
| `crates/mika-agent/src/skills/builtin_handlers.rs` | Replace 2 linked-skill blocks with warning; flip 2 tests |
| `crates/mika-cli/src/commands/skills.rs` | `Llm` subcommand + `LlmAction` enum |
| `crates/mika-cli/src/cli.rs` | Wire `Llm` variant into the `SkillsCommand` match |
| `docs/skills.md` | LLM override docs + CLI + linked write-back |
| `docs/architecture.md` | Extend skill_overrides ERD |
| `docs/adr/002-filesystem-based-skill-registry.md` | New decision entry |
| `docs/runtime-structure.md` | v19 → v20 migration row |

## System-Wide Impact

### Interaction Graph

```
mika skills llm qa-review set anthropic/claude-sonnet-4-6
  → ModelSpec::parse() validates
  → db.set_skill_llm_override(agent_id, "qa-review", "anthropic", "claude-sonnet-4-6")
  → next agent turn: SkillRegistry::from_dir() + apply_overrides()
  → entry.manifest.llm = Some(LlmOverride { provider: Some("anthropic"), model: Some("claude-sonnet-4-6") })
  → resolve_skill_llm_override() (agent.rs:2427) returns the override
  → run_loop() instantiates the per-skill provider via Settings::make_provider_for()
```

Tool-driven path (`update_skill` with `llm_provider`/`llm_model`) lands at the
same `set_skill_llm_override` call after origin routing.

### Error Propagation

- `ModelSpec::parse()` failure → CLI exits non-zero / tool returns error;
  no DB write attempted.
- `set_skill_llm_override` UPSERT is atomic.
- `write_llm_to_skill_toml()` failure on linked/custom → tool error; source
  file unchanged (write to a temp + rename, or wrapped in a single fs::write
  if existing pattern is direct).

### State Lifecycle Risks

- Orphaned rows on uninstall via path that bypasses DB: harmless (no matching
  `SkillEntry`), identical to existing `always_on` risk. Accepted.
- Migration v20 on a DB with empty `skill_overrides`: `ALTER TABLE` is a
  metadata-only no-op. Safe.
- `apply_overrides()` post-override removal of broken always_on skills
  (existing logic at lines 149-185) is unaffected — LLM columns alone do not
  trigger removal.

### Integration Test Scenarios

1. Set DB override → restart agent → override survives (not reset by
   `seed_bundled_skills()`).
2. Set DB override equal to manifest `[llm]` → row is deleted (D3).
3. Linked skill: `update_skill` with `llm_provider` writes to source
   `skill.toml`, no DB row.
4. Linked skill: `write_skill_variant` succeeds, emits warning, file appears
   in source tree.
5. `mika skills uninstall` removes the row (existing `delete_skill_override`).

## Acceptance Criteria

- [ ] Schema v20 migration adds `llm_provider` and `llm_model` columns to
      `skill_overrides`.
- [ ] `mika skills llm <name> set <provider>/<model>` writes DB override for
      bundled and snapshot-installed skills; writes to source `skill.toml`
      for linked/custom.
- [ ] `mika skills llm <name> reset` clears the override; if the row becomes
      fully `NULL`, the row is deleted.
- [ ] `mika skills llm <name> show` prints effective value with source
      annotation (`db-override` / `manifest` / `agent-default`).
- [ ] `update_skill` tool accepts `llm_provider`/`llm_model` and routes
      identically to the CLI.
- [ ] DB override survives agent restart.
- [ ] Setting override to manifest default deletes the override (D3).
- [ ] `list_skills` shows `[override]` (existing badge — no new badge needed).
- [ ] `write_skill_variant` on a linked skill succeeds, emits warn log + tool
      output annotation, writes to source.
- [ ] `review_skill` on a linked skill succeeds (no refusal), emits warn.
- [ ] `ModelSpec::parse()` called in CLI `set`; invalid provider/model returns
      a clear error before any DB write.
- [ ] `delete_skill` (tool) and `mika skills uninstall` clean up LLM override
      columns via the existing row-delete (no code change).
- [ ] Linked-skill refusal tests flipped to success+warning assertions.
- [ ] `cargo clippy` clean, `cargo test` green (all ~2045+ tests).
- [ ] `scripts/sync-agent-docs.sh` run after `docs/` edits; CI `docs-sync` job
      green.

## Out of Scope

- Per-variant LLM overrides (variant selection is provider-conditional already;
  agent-level + skill-level is sufficient — YAGNI).
- LLM badge in `mika skills list` text output (DB not available in that path
  today; same deferral as `always_on` badge in v7).
- Windows symlink support for `--link` (pre-existing limitation).

## Verification

Run from the worktree:

```
cargo build
cargo clippy --all-targets -- -D warnings
cargo test
cargo test -p mika-agent --test eval

# end-to-end smoke
cargo run --bin mika -- skills llm qa-review show
cargo run --bin mika -- skills llm qa-review set anthropic/claude-sonnet-4-6
cargo run --bin mika -- skills llm qa-review show       # → [db-override]
cargo run --bin mika -- skills llm qa-review reset
cargo run --bin mika -- skills llm qa-review show       # → [manifest] or [agent-default]

# linked skill round trip
mika skills install <local-path> --link
# trigger review_skill / write_skill_variant via agent — confirm warning + source write

bash scripts/verify-pipeline.sh
```

## Sources

- **Source plan:** `/home/samidarko/Downloads/2026-04-07-feat-skill-llm-override-and-linked-review-plan.md`
  (validated against current `mika/` source 2026-04-07)
- **Tracking issue:** senara-solutions/mika#474
- **Related prior work:**
  - #463 — `LlmOverride` fields made optional, `resolve_skill_llm_override`
    Keyword-only filter
  - #470 — `review_skill` / `write_skill_variant` hardening (this plan flips
    its linked-skill refusal)
  - schema v7 — `skill_overrides` table introduction
  - schema v17/v18/v19 — recent ALTER-TABLE migrations to mirror
- **Code anchors:**
  - `crates/mika-agent/src/db.rs:25` (`CURRENT_SCHEMA_VERSION`)
  - `crates/mika-agent/src/db.rs:398` (`SkillOverride`)
  - `crates/mika-agent/src/db.rs:2509` (`get_skill_overrides`)
  - `crates/mika-agent/src/db.rs:2525` (`set_skill_override`)
  - `crates/mika-agent/src/db.rs:2541` (`delete_skill_override`)
  - `crates/mika-agent/src/skills/mod.rs:113` (`apply_overrides`)
  - `crates/mika-agent/src/agent.rs:2427` (`resolve_skill_llm_override`)
  - `crates/mika-agent/src/skills/builtin_handlers.rs:991-996,1152-1156`
    (linked-skill refusal blocks)
  - `crates/mika-agent/src/skills/builtin_handlers.rs:2177,2457` (refusal tests)
