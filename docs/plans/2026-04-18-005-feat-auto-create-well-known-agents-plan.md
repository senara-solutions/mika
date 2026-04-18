---
title: "feat: Auto-create well-known agents (mika-dev, mika-qa) at setup time"
type: feat
status: active
date: 2026-04-18
issue: 254
---

# feat: Auto-create well-known agents (mika-dev, mika-qa) at setup time

## Overview

When `dev_mode = true` in config, the startup sequence auto-provisions two well-known development agents (`mika-dev`, `mika-qa`) with role-specific identity, soul, and skill assignments. A companion `disable_agent_provisioning` flag prevents file overwrites on restart, allowing manual customization to persist.

## Problem Frame

The autonomous dev loop requires `mika-dev` and `mika-qa` agents with specific configurations: identity files (soul.md), skill assignments, env vars, core memory. Currently these must be manually created and configured. Missing agents cause silent degradation. Wrong skill assignments cause active interference (qa-review's run_gh allowlist blocked mika-dev's `gh pr create` in issue #602).

## Requirements Trace

- R1. `dev_mode = true` in config.toml triggers auto-provisioning of mika-dev and mika-qa on startup
- R2. `MIKA_DISABLE_AGENT_PROVISIONING=1` prevents agent file overwrites on restart/deploy
- R3. mika-dev gets self-dev family skills; qa-review family skills disabled
- R4. mika-qa gets qa-review family skills; self-dev family skills disabled
- R5. First-time setup with `dev_mode = true` produces a fully functional autonomous loop without manual agent creation
- R6. Existing manually-configured agents are never overwritten

## Scope Boundaries

- No changes to the skill seeding mechanism itself — `seed_bundled_skills()` continues to write ALL bundled skills to ALL agents. Role-specific filtering is done via `skill_overrides` DB table (disable unwanted skills per agent)
- No new manifest file format — uses existing DB-backed `skill_overrides` for skill assignment
- No agent deletion when `dev_mode` goes from true to false — agents persist, just stop being re-provisioned
- No core memory seeding beyond the default `user.md` mechanism — soul.md and identity are file-based
- Soul content for well-known agents is minimal/placeholder — operators customize via soul.md files

### Deferred to Separate Tasks

- Per-agent `.env` templates for GitHub App credentials: separate concern, documented in provisioning output
- `mika setup --mode dev` interactive wizard: future enhancement once provisioning is proven

## Context & Research

### Relevant Code and Patterns

- `crates/mika-common/src/home.rs` — `bootstrap_agent()`, `write_default_if_missing()`, `bootstrap()`
- `crates/mika-common/src/config.rs` — `Settings` struct, `disable_bundled_skills: bool` pattern
- `crates/mika-agent/src/startup.rs` — `seed_bundled_skills_if_needed()`, `seed_core_memory_if_empty()`
- `crates/mika-agent/src/server/mod.rs` — `run_server()` (line 518), `init_agent()` (line 307)
- `crates/mika-cli/src/init.rs` — `init_base_for_agent()`, `ensure_initialized_for_agent()`
- `crates/mika-agent/src/tools/create_agent.rs` — `CreateAgentTool::execute()` pattern for custom identity/soul after bootstrap
- `crates/mika-agent/src/db.rs` — `set_skill_enabled()`, `SkillOverride`, `get_skill_overrides()`

### Institutional Learnings

- **TOML injection (P1):** Config file writes MUST use typed structs + serde, never `format!()` (from `docs/solutions/ux-improvements/cli-agent-team-creation-wizard.md`). However, `create_agent.rs` already uses `format!()` for identity.toml — follow the existing tool pattern for consistency, then fix both in a follow-up
- **Skill overrides are the source of truth for per-agent skill config:** `seed_bundled_skills()` overwrites skill files on every startup (security updates). User preferences survive via `skill_overrides` DB table
- **soul.md is required:** Validation checks for its existence. Auto-created agents must include one
- **CLI vs server execution models:** Both paths need provisioning. Server iterates all agents; CLI targets a single named agent
- **Startup sequence order:** provision agents → seed bundled skills → scan skills dir → apply overrides → validate

## Key Technical Decisions

- **DB overrides, not a new manifest file:** Per-agent skill assignments use the existing `skill_overrides` table with `set_skill_enabled()`. No new file format or mechanism — keeps the system simple and consistent with how skill enable/disable already works
- **First-creation-only overrides:** Skill overrides are written to DB only when the agent is first created (not on every startup). This means user manual `mika skills enable/disable` changes persist across restarts. Trade-off: no "reset to defaults" without deleting the agent
- **Provisioning in `startup.rs`, not `home.rs`:** The provisioning function belongs in `mika-agent::startup` because it needs the `Database` to write skill overrides. `home.rs` (in `mika-common`) has no DB dependency. The file-system bootstrapping still delegates to `home::bootstrap_agent()`
- **`dev_mode` as a Settings field:** Follows the exact `disable_bundled_skills` pattern — settable via config.toml or `MIKA_DEV_MODE=true` env var
- **Provisioning runs before `list_agents()`:** In `run_server()`, provisioning must happen after `Settings::load()` (to read `dev_mode`) and before `list_agents()` (so newly created agents are discovered). In the CLI path, `ensure_initialized_for_agent()` is extended to auto-create well-known agents when requested
- **Soul content as embedded constants:** Well-known agent souls are defined as `const` strings in the provisioning module (same pattern as `DEFAULT_SOUL` in `home.rs`). Operators customize by editing the files after first creation

## Open Questions

### Resolved During Planning

- **Where to insert provisioning in server startup?** After `migrate_to_multi_agent()` (line 527) and before `list_agents()` (line 586) in `run_server()`. This ensures agents exist on disk before discovery
- **DB not available during provisioning?** Skill overrides are written inside `init_agent()` (which opens the DB), not during the file-system provisioning step. Two-phase: (1) create files, (2) set DB overrides during init
- **What if mika-dev exists but mika-qa doesn't?** Provisioning checks each agent independently via `agent_exists()`. Partial state is handled naturally

### Deferred to Implementation

- Exact soul.md prose for mika-dev and mika-qa — will be minimal placeholders that operators customize
- Whether `create_agent.rs` identity.toml format!() should be fixed in this PR or a follow-up

## Output Structure

```
crates/mika-agent/src/
  well_known_agents.rs          # NEW — agent specs + provisioning logic
  startup.rs                    # MODIFY — add provision_well_known_agents_if_needed()

crates/mika-common/src/
  config.rs                     # MODIFY — add dev_mode + disable_agent_provisioning fields

crates/mika-agent/src/server/
  mod.rs                        # MODIFY — call provisioning before list_agents()

crates/mika-cli/src/
  init.rs                       # MODIFY — auto-create well-known agents in CLI path
```

## High-Level Technical Design

> *This illustrates the intended approach and is directional guidance for review, not implementation specification. The implementing agent should treat it as context, not code to reproduce.*

```
Startup (server):
  Settings::load()           → reads dev_mode, disable_agent_provisioning
  migrate_to_multi_agent()
  provision_well_known_agents_if_needed(global_home, dev_mode, disabled)
    for each well-known agent spec:
      if agent_exists() → skip (log)
      bootstrap_agent()                    → creates dirs + default files
      overwrite identity.toml              → agent-specific name/emoji
      overwrite soul.md                    → agent-specific soul
  list_agents()              → discovers all agents including newly created
  for each agent:
    init_agent()
      seed_bundled_skills()  → writes ALL skills to disk (as before)
      seed_well_known_skill_overrides(db, agent_name)
        if agent is well-known AND no overrides exist yet:
          set_skill_enabled(disabled=true) for unwanted skills
      apply_overrides()      → evicts disabled skills from registry
```

## Implementation Units

- [x] **Unit 1: Add `dev_mode` and `disable_agent_provisioning` to Settings**

**Goal:** Add two new boolean config fields following the `disable_bundled_skills` pattern.

**Requirements:** R1, R2

**Dependencies:** None

**Files:**
- Modify: `crates/mika-common/src/config.rs`
- Test: `crates/mika-common/src/config.rs` (inline tests)

**Approach:**
- Add `dev_mode: bool` with `#[serde(default)]` (defaults to false)
- Add `disable_agent_provisioning: bool` with `#[serde(default)]` (defaults to false)
- Add both to the manual `Debug` impl (no redaction needed — not secrets)
- Both are settable via config.toml or `MIKA_DEV_MODE` / `MIKA_DISABLE_AGENT_PROVISIONING` env vars (automatic via config-rs `MIKA_` prefix)

**Patterns to follow:**
- `disable_bundled_skills: bool` field and its test `test_disable_bundled_skills_from_env`

**Test scenarios:**
- Happy path: `MIKA_DEV_MODE=true` env var → `settings.dev_mode == true`
- Happy path: `MIKA_DISABLE_AGENT_PROVISIONING=true` env var → `settings.disable_agent_provisioning == true`
- Happy path: Both default to false when unset
- Happy path: Config.toml `dev_mode = true` → field is true

**Verification:**
- `cargo test -p mika-common` passes with new fields and tests

---

- [x] **Unit 2: Create `well_known_agents` module with agent specs and provisioning**

**Goal:** Define the well-known agent specifications (name, display name, emoji, soul, skill assignments) and the filesystem provisioning function.

**Requirements:** R1, R2, R3, R4, R5, R6

**Dependencies:** Unit 1

**Files:**
- Create: `crates/mika-agent/src/well_known_agents.rs`
- Modify: `crates/mika-agent/src/lib.rs` (add `pub mod well_known_agents;`)
- Test: `crates/mika-agent/src/well_known_agents.rs` (inline tests)

**Approach:**
- Define a `WellKnownAgent` struct with: `name`, `display_name`, `emoji`, `soul` (all `&'static str`), and `disabled_skills: &'static [&'static str]` (skills to disable for this agent)
- Define two static specs: `MIKA_DEV` and `MIKA_QA`
- `MIKA_DEV.disabled_skills`: qa-review, qa-review-build-callback, skill-review
- `MIKA_QA.disabled_skills`: self-dev, self-dev-iterate, self-dev-webhook-qa, self-dev-webhook-ci, claude-pilot, permission-policy, agents-teams, address-pr-comments, resolve-pr-conflicts
- Shared skills (both get): build-mika, deploy-mika, self-check
- `WELL_KNOWN_AGENTS: &[WellKnownAgent]` static slice for iteration
- `provision_well_known_agents(home_dir, disabled)` function:
  - If `disabled`: log warning and return (same pattern as `seed_bundled_skills_if_needed`)
  - For each spec: check `agent_exists()`, skip if yes (log info), otherwise call `bootstrap_agent()`, then overwrite `identity.toml` and `soul.md` with agent-specific content
- `is_well_known_agent(name) -> Option<&WellKnownAgent>` helper for lookup
- `seed_well_known_skill_overrides(db, agent_name)` function:
  - If agent is not well-known, return early
  - If agent already has any `skill_overrides` rows, return early (user has customized)
  - Otherwise, write `set_skill_enabled(agent_name, skill, Some(false))` for each skill in `disabled_skills`

**Patterns to follow:**
- `seed_bundled_skills_if_needed()` in `startup.rs` for the guard pattern
- `CreateAgentTool::execute()` in `tools/create_agent.rs` for the bootstrap + overwrite pattern
- `DEFAULT_SOUL`, `DEFAULT_IDENTITY` constants in `home.rs` for content constants

**Test scenarios:**
- Happy path: `provision_well_known_agents()` creates both agents when they don't exist
- Happy path: Created agents have correct identity.toml content (name, emoji)
- Happy path: Created agents have correct soul.md content
- Edge case: Agent already exists → skipped, files not overwritten
- Edge case: One agent exists, other doesn't → only missing one created
- Edge case: `disabled = true` → no agents created, warning logged
- Happy path: `is_well_known_agent("mika-dev")` returns the spec
- Edge case: `is_well_known_agent("mika")` returns None
- Happy path: `seed_well_known_skill_overrides()` writes correct disabled skills for mika-dev
- Happy path: `seed_well_known_skill_overrides()` writes correct disabled skills for mika-qa
- Edge case: Agent already has skill overrides → no changes written
- Edge case: Non-well-known agent → no changes written

**Verification:**
- `cargo test -p mika-agent` passes with new module and tests

---

- [x] **Unit 3: Integrate provisioning into server startup**

**Goal:** Call `provision_well_known_agents()` in `run_server()` before agent discovery, and `seed_well_known_skill_overrides()` inside `init_agent()` after DB is available.

**Requirements:** R1, R5

**Dependencies:** Unit 2

**Files:**
- Modify: `crates/mika-agent/src/server/mod.rs`
- Test: `crates/mika-agent/src/server/mod.rs` (or integration test if needed)

**Approach:**
- In `run_server()`, after `migrate_to_multi_agent()` (line 527) and before `list_agents()` (line 586):
  ```
  if settings.dev_mode {
      well_known_agents::provision_well_known_agents(
          global_home,
          settings.disable_agent_provisioning,
      );
  }
  ```
- In `init_agent()`, after `seed_bundled_skills_if_needed()` (line 336) and before skill registry construction:
  - Open a sync DB handle (already available as `db` at that point)
  - Call `well_known_agents::seed_well_known_skill_overrides(&db, agent_name)`
- Log at info level when provisioning creates agents or sets overrides

**Patterns to follow:**
- The `disable_bundled_skills` guard in `init_agent()` (line 336)
- The `dashboard_enabled` check pattern (line 530)

**Test scenarios:**
- Integration: Server startup with `dev_mode = true` creates both agents and initializes them
- Integration: Server startup with `dev_mode = false` does not create well-known agents
- Integration: `disable_agent_provisioning = true` prevents file creation even when `dev_mode = true`

**Verification:**
- `cargo build` succeeds
- Server startup with `dev_mode = true` logs agent creation

---

- [x] **Unit 4: Integrate provisioning into CLI startup**

**Goal:** Auto-create well-known agents in the CLI path when `dev_mode` is enabled and the requested agent is a well-known name.

**Requirements:** R1, R5

**Dependencies:** Unit 2

**Files:**
- Modify: `crates/mika-cli/src/init.rs`

**Approach:**
- In `ensure_initialized_for_agent()`, before the "Agent not found" error:
  - Load global settings to check `dev_mode`
  - If `dev_mode == true` and `is_well_known_agent(agent_name).is_some()` and agent doesn't exist:
    - Call `provision_well_known_agents()` (provisions all well-known agents, idempotent)
    - Continue to the next check (agent should now exist)
- In `init_base_for_agent()`, after DB is opened and `seed_bundled_skills_if_needed()`:
  - Call `seed_well_known_skill_overrides(&db, agent_name)` (same as server path)
- The CLI error message remains for non-well-known agents that don't exist

**Patterns to follow:**
- `ensure_initialized_for_agent()` guard pattern
- `init_base_for_agent()` initialization sequence

**Test scenarios:**
- Happy path: `mika chat --agent mika-dev` with `dev_mode = true` auto-creates the agent
- Edge case: `mika chat --agent mika-dev` with `dev_mode = false` still errors ("Agent not found")
- Edge case: `mika chat --agent custom-agent` with `dev_mode = true` still errors for non-well-known agents

**Verification:**
- `cargo build` succeeds
- CLI startup with well-known agent name and `dev_mode = true` works without pre-creating the agent

---

- [x] **Unit 5: Update documentation and env var references**

**Goal:** Document the new config flags and provisioning behavior.

**Requirements:** R1, R2

**Dependencies:** Units 1-4

**Files:**
- Modify: `CLAUDE.md` (root) — add `MIKA_DEV_MODE` and `MIKA_DISABLE_AGENT_PROVISIONING` to env vars section
- Modify: `.env.example` — add the new env vars with comments

**Approach:**
- Add `MIKA_DEV_MODE` to the optional startup behavior section in CLAUDE.md
- Add `MIKA_DISABLE_AGENT_PROVISIONING` alongside `MIKA_DISABLE_BUNDLED_SKILLS`
- Document the behavior: what agents are created, what skills each gets, idempotency

**Patterns to follow:**
- `MIKA_DISABLE_BUNDLED_SKILLS` documentation pattern in CLAUDE.md

**Test expectation:** none — documentation-only changes

**Verification:**
- Documentation accurately describes the provisioning behavior and env vars

## System-Wide Impact

- **Interaction graph:** Provisioning runs before `list_agents()` in server, before `ensure_initialized_for_agent()` in CLI. Skill overrides are set inside `init_agent()` / `init_base_for_agent()` after DB open but before `SkillRegistry::from_dir()` + `apply_overrides()`
- **Error propagation:** Provisioning failures (filesystem errors) should log warnings but not crash the server — other agents should still initialize. Individual agent provisioning failures are non-fatal
- **State lifecycle risks:** Two-phase design (files first, DB overrides during init) means a crash between file creation and DB init would leave an agent with all skills enabled. Next startup would detect existing overrides and skip — but if the DB was never written to, overrides would be applied on that next init. This is safe because `seed_well_known_skill_overrides` checks for existing overrides, not for agent age
- **API surface parity:** No API changes. Dashboard discovers agents via `list_agents()` which will naturally include provisioned agents
- **Unchanged invariants:** `seed_bundled_skills()` continues to write ALL skills to ALL agents. The per-agent filtering happens entirely in the DB override layer. This preserves the security-update propagation guarantee

## Risks & Dependencies

| Risk | Mitigation |
|------|------------|
| Identity.toml uses `format!()` (TOML injection risk from learning) | Well-known agent names are hardcoded constants, not user input. No injection vector. Follow-up to fix both this and `create_agent.rs` |
| Skill overrides written only on first creation — no reset path | Document `mika skills enable/disable` for manual adjustment. Agent deletion + re-creation as escape hatch |
| Dev_mode flag checked on every startup but provisioning is idempotent | `agent_exists()` check is cheap (stat call). No performance concern |

## Sources & References

- Related issue: #254
- Related issues mentioned: #601 (bundling migration), #602 (skill interference), #620 (re-seed disabled skills)
- Existing pattern: `seed_bundled_skills_if_needed()` in `crates/mika-agent/src/startup.rs`
- Existing pattern: `CreateAgentTool::execute()` in `crates/mika-agent/src/tools/create_agent.rs`
- Learning: `docs/solutions/ux-improvements/cli-agent-team-creation-wizard.md` (TOML injection)
- Learning: `docs/solutions/architecture-patterns/skill-enabled-state-db-eviction.md` (override flow)
