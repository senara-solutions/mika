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

The autonomous dev loop requires `mika-dev` and `mika-qa` agents with specific configurations: identity files (soul.md), skill assignments, and per-agent env vars. Previously these had to be created manually. Missing agents caused silent degradation. Wrong skill assignments caused active interference (qa-review's run_gh allowlist blocked mika-dev's `gh pr create`).

The provisioning system follows a two-phase design to work within the existing startup sequence where filesystem operations happen before the DB is available.

## Guidance

### Two-phase provisioning

**Phase 1 (filesystem):** `provision_well_known_agents()` runs after `migrate_to_multi_agent()` but before `list_agents()` in server startup. For each well-known agent spec, it checks `agent_exists()`, calls `bootstrap_agent()` to create the directory structure, then overwrites `identity.toml` and `soul.md` with agent-specific content.

**Phase 2 (DB overrides):** `seed_well_known_skill_overrides()` runs inside `init_agent()` after the DB is opened and `seed_bundled_skills_if_needed()` has written all skill files. It writes `set_skill_enabled(false)` for skills the agent should not have. Both phases are gated on `settings.dev_mode`.

### Key design decisions

1. **DB overrides, not file filtering:** All bundled skills are still written to all agents by `seed_bundled_skills()` (preserving security update propagation). Per-agent filtering uses the existing `skill_overrides` DB table.

2. **First-creation-only overrides:** Skill overrides are written only when no `skill_overrides` rows exist for the agent. This preserves user customizations via `mika skills enable/disable` across restarts.

3. **`disable_agent_provisioning` env var:** Follows the `MIKA_DISABLE_BUNDLED_SKILLS` pattern. When true, prevents file creation/overwrite even when `dev_mode = true`. Allows manual edits to soul.md and identity.toml to persist across deploys.

4. **Gate `seed_well_known_skill_overrides` on `dev_mode`:** The skill override seeding must be gated on `dev_mode` at the call site, not just inside the function. Without this gate, a user who manually names an agent "mika-dev" outside dev mode would have skills silently disabled.

### Adding a new well-known agent

1. Add a `WellKnownAgent` static spec in `crates/mika-agent/src/well_known_agents.rs` with name, display_name, emoji, soul content, and `disabled_skills` list
2. Add a reference to `WELL_KNOWN_AGENTS` static slice
3. The agent is automatically provisioned on next startup with `dev_mode = true`

### Agent spec structure

```rust
pub static MIKA_DEV: WellKnownAgent = WellKnownAgent {
    name: "mika-dev",
    display_name: "Dev",
    emoji: "...",
    soul: MIKA_DEV_SOUL,
    disabled_skills: &["qa-review", "qa-review-build-callback", "skill-review"],
};
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
    seed_well_known_skill_overrides -> disables unwanted skills via DB
    apply_overrides()               -> evicts disabled skills from registry
```

**Key files:**
- `crates/mika-agent/src/well_known_agents.rs` -- agent specs + provisioning logic
- `crates/mika-common/src/config.rs` -- `dev_mode` and `disable_agent_provisioning` fields
- `crates/mika-agent/src/server/mod.rs` -- server startup integration
- `crates/mika-cli/src/init.rs` -- CLI startup integration

## Related

- Issue #254 -- original feature request
- Issue #602 -- skill interference incident that motivated per-agent skill filtering
- `docs/solutions/architecture-patterns/skill-enabled-state-db-eviction.md` -- how skill overrides work
- `docs/solutions/architecture-patterns/per-agent-dotenv-config-injection.md` -- per-agent config loading
