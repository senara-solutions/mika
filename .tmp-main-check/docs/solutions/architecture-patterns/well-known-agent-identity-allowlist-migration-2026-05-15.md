---
title: "Well-known agent identity-driven allowlist migration (D2 cross-cutting)"
date: 2026-05-15
category: architecture-patterns
module: well_known_agents, skills, startup
problem_type: best_practice
component: development_workflow
severity: medium
applies_when:
  - Adding new bundled skills and deciding which agents should have them
  - Debugging why a well-known agent has or lacks a specific skill
  - Understanding the provisioning lifecycle for well-known agents
  - Planning changes to agent skill assignments
tags:
  - identity-allowlist
  - well-known-agents
  - skill-provisioning
  - d2-migration
  - separation-of-concerns
---

# Well-known agent identity-driven allowlist migration (D2 cross-cutting)

## Context

Prior to #815, well-known agents (mika-dev, mika-qa, mika-relay) defined their skill set via a `disabled_skills` denylist constant in `well_known_agents.rs`, which was translated into `skill_overrides` DB rows (`enabled=0`) at startup by `seed_well_known_skill_overrides()`. This split "what an agent is" between compile-time Rust constants and runtime DB writes — a separation-of-concerns violation identified in the mika-arch v1 plan (§ D2).

mika-arch was the first agent to migrate to identity-driven `[skills].allowlist` in #813. This ticket (#815) completed the migration for the remaining three agents.

## Guidance

All four well-known agents now declare `[skills].allowlist` in their identity.toml (static consts for mika-dev/qa/relay, computed at provision time for mika-arch). The `disabled_skills` field on all agent specs is `&[]` (empty). New bundled skills are **denied by default** — they must be explicitly added to each agent's allowlist.

### Allowlist sizes

| Agent | Skills | Identity const |
|-------|--------|---------------|
| mika-dev | 25 | `MIKA_DEV_IDENTITY` (static) |
| mika-qa | 17 | `MIKA_QA_IDENTITY` (static) |
| mika-relay | 1 | `MIKA_RELAY_IDENTITY` (static) |
| mika-arch | 3 | `build_mika_arch_identity()` (computed — needs `MIKA_KG_DOCS_ROOTS`) |

### Migration mechanics

A one-time migration (`migrate_well_known_to_identity_allowlist`) runs inside `init_agent()` when `dev_mode = true`. It:

1. Checks `schema_meta` for marker `well_known_d2_migration_v1`
2. If absent, runs a single transaction that:
   - DELETEs `skill_overrides` rows with `agent_id IN ('mika-dev', 'mika-qa', 'mika-relay') AND enabled = 0`
   - INSERTs the idempotency marker
3. Operator-set LLM overrides (`enabled IS NULL`, `llm_provider`/`llm_model` non-NULL) are preserved
4. User-defined agents and mika-arch rows are untouched

### Fast-path exit

`seed_well_known_skill_overrides()` now checks `spec.disabled_skills.is_empty() && spec.llm_overrides.is_empty()` and returns immediately for agents that have nothing to seed. Post-migration, mika-dev/qa/relay take this fast path. mika-arch still enters for LLM override reconciliation.

## Why This Matters

**Deny-by-default is safer.** With the denylist pattern, new bundled skills were silently inherited by all agents unless someone remembered to add them to every agent's denylist. With the allowlist, new skills must be consciously assigned — the safe default is no access.

**Identity owns the agent's complete definition.** The agent's name, emoji, KG config, and now skill set all live in one place (the identity const/template). No more cross-referencing `disabled_skills` constants against `skill_overrides` DB rows to understand what an agent can do.

**Eliminated dead-code maintenance.** The `disabled_skills` constants were 7-31 entries that had to stay in sync with the growing bundled skills set. Every new bundled skill required updating up to 3 denylist arrays. The allowlist inverts this: you add the skill only to agents that need it.

## When to Apply

- **Adding a new bundled skill:** Add the skill name to each agent's identity const that should have access. See root `CLAUDE.md` § "Adding a New Bundled Skill" for the full checklist.
- **Debugging skill visibility:** Check the agent's identity const for the `[skills].allowlist` section. The allowlist is the single source of truth for which skills are active (subject to `apply_overrides()` at Phase 0 for operator disables).
- **Understanding skill filtering order:** Phase -1 (`apply_identity_allowlist`) → Phase 0 (`apply_overrides` — DB-backed enabled state) → Phase 1 (transient overrides). The allowlist runs first and evicts non-listed skills; DB overrides can further disable allowlisted skills.

## Examples

**Before (denylist pattern):**
```rust
pub static MIKA_RELAY: WellKnownAgent = WellKnownAgent {
    disabled_skills: &[
        "self-dev", "qa-review", "skill-review", // ... 31 entries
    ],
    identity_source: None,
    // ...
};
// + seed_well_known_skill_overrides() writes 31 DB rows at startup
```

**After (allowlist pattern):**
```rust
pub static MIKA_RELAY: WellKnownAgent = WellKnownAgent {
    disabled_skills: &[],  // empty
    identity_source: Some(IdentitySource::Static(MIKA_RELAY_IDENTITY)),
    // ...
};

const MIKA_RELAY_IDENTITY: &str = "\
name = \"Relay\"\n\
emoji = \"🔑\"\n\
\n\
[skills]\n\
allowlist = [\n\
  \"permission-policy\",\n\
]\n";
// Zero DB rows needed — identity owns the skill set
```

## Related

- Issue #815 — this migration
- Issue #813 — introduced the identity-allowlist mechanism for mika-arch
- `docs/solutions/architecture-patterns/well-known-agent-provisioning-dev-mode.md` — overall provisioning architecture (updated to reflect this change)
- `docs/solutions/architecture-patterns/structural-readonly-agent-binds-at-every-layer-2026-04-25.md` — mika-arch's read-only constraints (tools denylist, skill allowlist)
- `docs/solutions/runtime-errors/well-known-agent-disabled-skills-seeding-drift-2026-05-09.md` — the #1041 drift bug that this migration subsumes for mika-dev/qa/relay
