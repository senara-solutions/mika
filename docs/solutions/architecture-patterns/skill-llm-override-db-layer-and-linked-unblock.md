---
title: "Per-skill LLM override as operator DB layer; linked skills are mutable by design"
category: architecture-patterns
date: 2026-04-07
tags: [skills, llm, overrides, migrations, symlinks, operator-ux]
pr: senara-solutions/mika#475
issue: senara-solutions/mika#474
---

# Per-skill LLM override as operator DB layer; linked skills are mutable by design

## Problem

Two orthogonal operator frictions in skill configuration:

1. **LLM override required editing tracked files.** The only way to retune a
   skill's provider/model (e.g. swap `qa-review` from DeepSeek to Claude
   Sonnet) was editing `[llm]` in that skill's `skill.toml` and committing.
   The manifest captures *author intent* (what the skill was designed for);
   there was no operator-facing runtime layer.
2. **Linked skills were refused by review/refinement.** Skills installed with
   `--link` are symlinks to live source. `review_skill` and
   `write_skill_variant` hard-refused them with a "read-only invariant"
   error, contradicting the entire point of `--link`.

## Root cause

Conceptual conflation. `[llm]` in `skill.toml` was answering two different
questions with the same mechanism: "what did the author validate?" (author
intent, marketplace default) *and* "what does this operator want right now?"
(runtime tuning). Two questions, two layers — the always_on override pattern
(schema v7) already solved exactly this shape, just for a different field.

The linked-skill refusal was symmetric confusion: treating `--link` as
"read-only source" when it is explicitly "mutable source via symlink."

## Solution

**Pattern: three-tier resolution with DB as the operator layer.**

```
DB override (skill_overrides.llm_provider/llm_model)  -- schema v20
  > Manifest [llm] in skill.toml                      -- author default
    > Agent default                                   -- fallback
```

Mirror the `always_on` override pattern exactly:

- Add two nullable columns (`llm_provider TEXT`, `llm_model TEXT`) to
  `skill_overrides`. NULL = "no override at this layer."
- `SkillRegistry::apply_overrides()` merges DB columns into
  `entry.manifest.llm` before the loop reads it. Downstream code
  (`resolve_skill_llm_override()` in `agent.rs:2427`) needs **zero changes** —
  the DB layer is invisible to the runtime.
- New CLI surface: `mika skills llm <name> {set|reset|show}` with
  **default-equals-delete**: setting an override that matches the manifest
  default deletes the row instead of storing it. Prevents stale rows
  blocking future skill updates.
- `show` annotates the source of the effective value: `[db-override]`,
  `[manifest]`, or `[agent-default]`.

**Linked skills: warn, don't refuse.** Replace both refusal sites in
`builtin_handlers.rs` with a single `warn_linked_skill_write()` helper that
emits a structured `tracing::warn!` and threads `linked: true` + a `warning`
string into tool output JSON. Writes land in the source tree by symlink
transparency. Flipped tests verify the variant file actually appears in
the source directory.

### Key code anchors

- `crates/mika-agent/src/db.rs` — migration `migrate_v19_to_v20`,
  `SkillOverride` struct, `set_skill_llm_override` /
  `delete_skill_llm_override` (transactional prune)
- `crates/mika-agent/src/skills/mod.rs:113` — `apply_overrides()` merges
  DB columns onto `entry.manifest.llm`
- `crates/mika-cli/src/commands/skills.rs` — `run_skill_llm()` with
  `ProviderKind` validation and default-equals-delete
- `crates/mika-agent/src/skills/builtin_handlers.rs` —
  `warn_linked_skill_write()` helper, both refusal blocks replaced

## Gotchas hit during implementation

- `migrate_v1` is clean-slate and directly inserts the *latest* version
  number. Adding a new v19→v20 migration required updating `migrate_v1` to
  insert `(20)` and to include the new columns in its inline
  `CREATE TABLE skill_overrides`, or tests that open fresh DBs fail with
  "no such column." This is a trap for every future migration author.
- The default-equals-delete pattern requires comparing effective override
  against manifest. Do this *before* the DB write, not after, so the pruned
  row never touches disk.
- When adding nullable columns, the multi-statement delete-then-prune flow
  must be wrapped in a transaction. A crash between UPDATE and prune DELETE
  would leave a half-cleared row.

## Prevention

- **Any time a marketplace/author concern conflicts with an operator-runtime
  concern, separate them into layers.** The `always_on` override pattern
  (schema v7) is the canonical precedent — copy its shape for any future
  per-skill runtime-tunable field.
- **Never refuse an operation on a `--link`ed entity.** `--link` is a
  user-signed consent to mutable-source semantics. Warn, don't block.
- **When adding a DB column, update both `migrate_v1` clean-slate CREATE
  and the incremental migration.** Test this with a fresh in-memory DB to
  catch the trap above.
