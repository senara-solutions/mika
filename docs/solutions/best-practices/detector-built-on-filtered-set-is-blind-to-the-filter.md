---
module: skills
tags: [detector, structural-gate, discovery, false-negative, verify-bundled-skills, builtin-tools]
problem_type: bug-class-prevention
category: best-practices
---

# A detector built on a filtered set is blind to what the filter drops

## Context

mika#1575 added `make verify-bundled-skills` — a pre-merge structural gate that asserts
invariants on the engine-coupled skills under `skills/bundled/` (the structural counterpart
to mika#1326 AC2). Its job is to catch *incomplete skill-adds*: a skill bundle that is
missing a required file, has an unresolvable `required_tools` token, etc.

The first implementation enumerated its subjects by calling the shared walker
`discover_bundled_skills()` and building a `SkillView` per result. Code review (cross-
corroborated by two independent reviewers) caught that **the walker only returns a
directory once it already contains a real `skill.toml`** (`build_support/bundled_skills_discover.rs`
skips any dir whose `skill.toml` is absent or a symlink). So a skill-add that *forgot
`skill.toml` entirely* produces zero `SkillView`s — and the gate passes **green** on the
exact failure class it exists to catch. The `if !rel_files.contains("skill.toml")` branch
was dead code against the real tree; only the synthetic unit test ever exercised it.

## Guidance

When a detector's input set is produced by a filter that **presupposes the invariant you
are checking**, the detector structurally cannot see the invariant's violation. Before
trusting "iterate the discovered set and check each one," ask: *what does the discovery
step silently drop, and is any dropped item a violation I am responsible for flagging?*

The fix is to check the **raw, unfiltered surface** for the existence class, separately from
the per-item checks on the discovered set:

```rust
// WRONG: every SkillView already has skill.toml (the walker guaranteed it),
// so this branch can never fire against the real tree.
for s in &skill_views {
    if !s.rel_files.contains("skill.toml") { flag(...); }
}

// RIGHT: scan the raw directory listing for skill-shaped dirs the walker dropped.
fn check1_missing_manifest(base: &Path) -> Vec<Failure> {
    for entry in std::fs::read_dir(base)?.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if !discover::is_bundled_skill_dir(&name) { continue; } // skip _shared, dotfiles
        let toml = entry.path().join("skill.toml");
        let is_real = std::fs::symlink_metadata(&toml)
            .map(|m| m.file_type().is_file()).unwrap_or(false);
        if !is_real { flag(name, "missing required file skill.toml"); }
    }
}
```

Test the gate against the **pipeline** (discovery → check), not just synthetic fixtures —
a synthetic `SkillView` with a missing file passes the unit test while masking that the real
pipeline never produces such a view. mika#1575's `real_bundled_tree_passes_green` test plus
a tempdir test that creates a manifest-less dir cover both directions.

### Companion fact — two distinct builtin-name registries in mika-agent

Resolving "is this tool name real?" in the skills layer requires knowing which registry to
consult — they are not interchangeable:

- **`skills::builtin_handlers::KNOWN_BUILTINS`** (7 entries) — the skill-handler *dispatch*
  functions. Use this to validate a `tools.json` declaration whose `handler.type = "builtin"`
  (its `function` must be one of these 7). A handler function not in this set is unroutable.
- **`tools::BUILTIN_TOOL_NAMES`** (~55 entries, kept complete by the mika#1217 F4 sync test) —
  every engine-registered builtin *tool*. `KNOWN_BUILTINS` is a strict subset. Use this to
  resolve a `[constraints] required_tools` token (e.g. `write_agent_file`, `update_task_status`).

Conflating them causes false-positives: validating a `required_tools` token against the
7-entry `KNOWN_BUILTINS` rejects legitimate core builtins. Prior art for the correct choice:
`skills/index.rs` already resolves `required_tools` against `BUILTIN_TOOL_NAMES`.

## Why This Matters

A detector that silently passes on the failure it was built to catch is worse than no
detector — it manufactures false confidence. The operator stops hand-checking ("the gate
covers it") precisely where the gate is blind. The cost of the gap is paid later, in
production, by whoever debugs the skill that never loaded.

This is a general shape, not specific to skills: any lint/gate/audit whose candidate set
comes from a parser, loader, or walker that *requires the thing being validated* inherits the
blind spot. Schema validators that only see rows the ORM could hydrate, link-checkers that
only see pages the router could resolve, and dependency auditors that only see packages the
resolver could install all have the same failure mode.

## When to Apply

- Building any structural gate, lint, or audit that iterates a "discovered" / "loaded" /
  "parsed" set and asserts properties per item.
- Reviewing a detector PR: explicitly ask what the discovery step drops and whether any
  dropped item is in the detector's mandate. Demand a test that exercises the full
  discovery→check pipeline, not only synthetic per-item fixtures.
- Resolving tool/handler names anywhere in the mika skills layer: pick `KNOWN_BUILTINS`
  (handler dispatch) vs `BUILTIN_TOOL_NAMES` (full builtin tool surface) deliberately.
