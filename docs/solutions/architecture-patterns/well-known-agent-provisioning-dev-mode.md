---
title: Well-known agent provisioning via dev_mode flag
date: 2026-04-18
category: architecture-patterns
module: startup, config, skills
problem_type: best_practice
component: development_workflow
severity: medium
applies_when:
  - Adding new well-known agents to the platform
  - Extending agent provisioning with new skill assignments
  - Debugging why a well-known agent has wrong skills enabled
tags:
  - agent-provisioning
  - dev-mode
  - skill-overrides
  - well-known-agents
  - startup-sequence
---

# Well-known agent provisioning via dev_mode flag

## Context

The autonomous dev loop requires `mika-dev`, `mika-qa`, and `mika-relay` agents with specific configurations: identity files (soul.md), skill assignments, model overrides, and per-agent env vars. Previously these had to be created manually. Missing agents caused silent degradation. Wrong skill assignments caused active interference (qa-review's run_gh allowlist blocked mika-dev's `gh pr create`).

The provisioning system follows a two-phase design to work within the existing startup sequence where filesystem operations happen before the DB is available.

## Guidance

### Two-phase provisioning

**Phase 1 (filesystem):** `provision_well_known_agents()` runs after `migrate_to_multi_agent()` but before `list_agents()` in server startup. For each well-known agent spec, it checks `agent_exists()`, calls `bootstrap_agent()` to create the directory structure, then overwrites `identity.toml` and `soul.md` with agent-specific content. If the spec provides `config_toml`, the default `config.toml` is also overwritten with agent-specific LLM settings (e.g., mika-relay uses haiku for cheap permission classification).

**Phase 2 (identity-driven allowlist, #815):** All four well-known agents declare `[skills].allowlist` in their identity.toml (static const for mika-dev/qa/relay, computed at provision time for mika-arch). `SkillRegistry::apply_identity_allowlist()` runs as Phase -1 before `apply_overrides()`, evicting all skills NOT in the allowlist. New bundled skills are denied by default unless explicitly added to an agent's allowlist. `seed_well_known_skill_overrides()` still runs inside `init_agent()` for agents with `llm_overrides` (mika-arch); agents with empty `disabled_skills` AND empty `llm_overrides` take a fast-path exit. Both phases are gated on `settings.dev_mode`. A one-time migration (`migrate_well_known_to_identity_allowlist`) deletes stale denylist `skill_overrides` rows for mika-dev/qa/relay, guarded by `schema_meta` marker `well_known_d2_migration_v1`.

### Key design decisions

1. **Identity allowlist, not DB denylist (#815):** All bundled skills are still written to all agents by `seed_bundled_skills()` (preserving security update propagation). Per-agent filtering uses `[skills].allowlist` in identity.toml — the agent's identity owns its skill set rather than splitting it between Rust compile-time constants and runtime DB rows. The `skill_overrides` table remains for operator-level per-skill LLM overrides and for user-defined agents.

2. **Deny by default:** With the allowlist pattern, new bundled skills are automatically denied unless explicitly added to an agent's allowlist. This is the correct default for well-known agents — new skills should be consciously assigned, not silently inherited.

3. **`disable_agent_provisioning` env var:** Follows the `MIKA_DISABLE_BUNDLED_SKILLS` pattern. When true, prevents file creation/overwrite even when `dev_mode = true`. Allows manual edits to soul.md and identity.toml to persist across deploys.

4. **Gate `seed_well_known_skill_overrides` on `dev_mode`:** The skill override seeding must be gated on `dev_mode` at the call site, not just inside the function. Without this gate, a user who manually names an agent "mika-dev" outside dev mode would have skills silently disabled.

### Adding a new well-known agent

1. Add a `WellKnownAgent` static spec in `crates/mika-agent/src/well_known_agents.rs` with name, display_name, emoji, soul content, `disabled_skills: &[]`, optional `config_toml`, and `identity_source: Some(IdentitySource::Static(IDENTITY_CONST))`
2. Add an identity const with `[skills].allowlist` listing the skills this agent should have
3. Add a reference to `WELL_KNOWN_AGENTS` static slice
4. The agent is automatically provisioned on next startup with `dev_mode = true`

### Agent spec structure

```rust
pub static MIKA_DEV: WellKnownAgent = WellKnownAgent {
    name: "mika-dev",
    display_name: "Dev",
    emoji: "...",
    soul: MIKA_DEV_SOUL,
    disabled_skills: &[],  // empty — uses identity allowlist
    config_toml: None,     // uses default config
    identity_source: Some(IdentitySource::Static(MIKA_DEV_IDENTITY)),
    llm_overrides: &[],
};
```

The identity const declares the skill allowlist:

```rust
const MIKA_DEV_IDENTITY: &str = "\
name = \"Dev\"\n\
emoji = \"🛠\"\n\
\n\
[skills]\n\
allowlist = [\n\
  \"self-dev\",\n\
  // ... 25 skills total\n\
]\n";
```

## Why This Matters

Without automated provisioning, every fresh install or new developer requires manual agent creation with exact skill assignments. Getting it wrong causes silent degradation (missing agents) or active interference (wrong skills enabled). The two-phase design respects the startup sequence constraints: filesystem provisioning must happen before `list_agents()` discovers agents, but DB operations can only happen after `init_agent()` opens the database.

The `dev_mode` gate is important: provisioning is opt-in and only affects development environments. Production deployments without `dev_mode` are unaffected.

## When to Apply

- When adding new well-known agents to the platform
- When modifying skill assignments for existing well-known agents
- When debugging startup sequence issues related to agent provisioning
- When understanding why `seed_bundled_skills()` writes ALL skills to ALL agents (by design)

## Examples

**Startup sequence with `dev_mode = true`:**
```
Settings::load()                    -> reads dev_mode
migrate_to_multi_agent()
provision_well_known_agents()       -> creates agent dirs + identity/soul files
list_agents()                       -> discovers all agents including new ones
for each agent:
  init_agent()
    seed_bundled_skills()           -> writes ALL skills to ALL agents
    migrate_well_known_to_identity_allowlist -> one-time: delete stale denylist rows
    seed_well_known_skill_overrides -> seeds LLM overrides (mika-arch); fast-path exit for others
    apply_identity_allowlist()      -> Phase -1: evicts skills not in allowlist
    apply_overrides()               -> Phase 0: applies DB overrides (LLM, enabled state)
```

**Key files:**
- `crates/mika-agent/src/well_known_agents.rs` -- agent specs + provisioning logic
- `crates/mika-common/src/config.rs` -- `dev_mode` and `disable_agent_provisioning` fields
- `crates/mika-agent/src/server/mod.rs` -- server startup integration
- `crates/mika-cli/src/init.rs` -- CLI startup integration

## Related

- Issue #254 -- original feature request
- Issue #602 -- skill interference incident that motivated per-agent skill filtering
- Issue #721 -- mika-relay agent for permission relay (first agent with `config_toml` override)
- Issue #813 -- identity-driven allowlist mechanism (mika-arch first)
- Issue #815 -- D2 cross-cutting: migrate mika-dev/qa/relay to identity allowlist
- `docs/solutions/architecture-patterns/skill-enabled-state-db-eviction.md` -- how skill overrides work
- `docs/solutions/architecture-patterns/per-agent-dotenv-config-injection.md` -- per-agent config loading
