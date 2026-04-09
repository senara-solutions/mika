# Skill Provider/Model Lifecycle

**Date:** 2026-04-09
**Status:** Brainstorm
**Scope:** mika (core), mika-skills (data updates)

## What We're Building

A consistent provider/model lifecycle for skills — ensuring every skill always has explicit `[llm]` defaults, and that create/update flows handle provider/model transitions gracefully with variant management.

## Why This Approach

Skills currently allow empty `[llm]` sections, falling through to the agent's active model. This creates ambiguity: you can't tell if a skill was *designed* for a specific model or just happens to run on whatever's active. Explicit defaults make skills portable and predictable.

## Key Decisions

### 1. Provider/Model Required on All Skills

Every skill must have `[llm].provider` and `[llm].model` set. No exceptions.

- **Builtins:** All 12 get `anthropic/claude-sonnet-4-6` as their default
- **Marketplace:** Existing skills in senara-solutions marketplace must be updated with `[llm]` sections
- **Custom skills (created via CLI or agent tool):** Default to the agent's current provider/model at creation time
- **Validation:** `mika skills validate` rejects skills without `[llm].provider` AND `[llm].model`. CI enforces this for marketplace PRs.

### 2. Create Flow — Duplicate Detection (Both CLI + Agent Tool)

When creating a skill and a skill with that name already exists:

- **Current behavior:** Hard error ("already exists")
- **New behavior:** Inform the user the skill exists, ask if they want to **update** it instead
- Applies to both `mika skills create <name>` (CLI) and `create_skill` (agent tool)
- If user declines update, abort (no overwrite)

### 3. Update Flow — Behavioral Change via Agent

When the agent rewrites a skill's behavior (e.g., "update qa-review to not check CI status"):

1. Agent reads the skill's current `[llm]` section
2. If current provider/model differs from the agent's active provider/model:
   - **Inform** the user: "This skill's [llm] default will change from `{old_provider}/{old_model}` to `{new_provider}/{new_model}` — this is enforced by design"
   - This is not optional — the system auto-sets [llm] to the current provider/model on every behavioral update
3. Proceed with the prompt/behavior rewrite
4. After updating, move to variant handling (see below)

### 4. DB Override Cleanup on Update

When a behavioral update changes the `[llm]` default:

- If the user had a DB override (`mika skills llm set`) that **matches the old manifest default**: clear it (it was tracking the default, now stale)
- If the DB override is **different from the old default**: preserve it (user set it intentionally)
- Inform the user either way

### 5. Variant Handling on Update

After a behavioral update that changes provider/model:

1. **Check if the new default (provider/model) had an existing variant:**
   - If yes → delete that variant (it's now the root prompt, a variant would be redundant)
   - No confirmation needed for this — it's automatic cleanup

2. **For remaining variants:**
   - Ask the user: "This skill has variants for [list]. They should be regenerated for the updated behavior. Generate them now?"
   - User can accept (regenerate all) or decline (variants become stale but aren't deleted)
   - Regeneration uses the `review_skill` builtin flow

### 6. Install Flow — Validation Gate

`mika skills install` rejects skills without `[llm].provider` + `[llm].model`. The validation runs before installation, same rules as `mika skills validate`.

## Complete Path Map

```
                    ┌─────────────┐
                    │ User action │
                    └──────┬──────┘
                           │
              ┌────────────┼────────────┐
              │            │            │
         CREATE        UPDATE       INSTALL
              │            │            │
     ┌────────┴───┐        │            │
     │ exists?    │        │     ┌──────┴──────┐
     │            │        │     │ has [llm]?  │
   NO│          YES│       │   YES│           NO│
     │            │        │     │             │
  Create w/     Ask:       │   Install      REJECT
  current    "update?"     │                (validate
  provider/     │          │                 fails)
  model      ┌──┴──┐      │
           YES│   NO│      │
             │     │      │
          ┌──┴─────┘      │
          │               │
          ▼               ▼
    ┌─────────────────────────┐
    │ Read current [llm]      │
    │ Compare to active model │
    └───────────┬─────────────┘
                │
        ┌───────┴───────┐
      SAME           DIFFERENT
        │               │
     Rewrite         Inform user:
     prompt          "[llm] will change
        │            to {new} (by design)"
        │               │
        │          ┌────┴────┐
        │          │ DB override │
        │          │ cleanup     │
        │          └────┬────┘
        │               │
        └───────┬───────┘
                │
        ┌───────┴───────┐
        │ Has variants? │
        └───────┬───────┘
              YES│         NO│
                │           │
    ┌───────────┴──┐        │
    │ New default  │        │
    │ was variant? │      Done
    │              │
  YES│           NO│
    │              │
  Delete it    Ask: "regen
    │          variants?"
    │          ┌──┴──┐
    │        YES│   NO│
    │          │     │
    │       Regen  Keep
    │       all    stale
    └──────┬───────┘
           │
         Done
```

## Affected Code

| File | Change |
|------|--------|
| `mika/crates/mika-agent/src/tools/create_skill.rs` | Duplicate detection → offer update; set [llm] from context |
| `mika/crates/mika-cli/src/commands/skills.rs` | CLI create: duplicate detection + [llm] defaults |
| `mika/crates/mika-agent/src/skills/manifest.rs` | Make [llm] provider+model required in validation |
| `mika/crates/mika-agent/src/skills/install.rs` | Reject install if [llm] missing |
| `mika/crates/mika-agent/src/bundled_skills.rs` | Add [llm] sections to all 12 builtin templates |
| `mika-skills/*/skill.toml` | Add [llm] sections to all 9 marketplace skills |
| `mika/crates/mika-agent/src/skills/mod.rs` | DB override cleanup logic on update |

## Open Questions

None — all paths resolved during brainstorm.
