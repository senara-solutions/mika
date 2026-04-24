---
title: "Feat: Add individual team member add/remove tools"
type: feat
status: active
date: 2026-04-24
---

# Feat: Add individual team member add/remove tools

## Overview

Add two new management tools — `add_team_member` and `remove_team_member` — for incremental team-composition changes. Today, any modification requires calling `update_team` with the entire replacement `agents` array, forcing the LLM caller to reconstruct the full roster for single-member changes. Each new tool is a thin wrapper that loads the existing `TeamDefinition`, applies one mutation, validates, and writes back — mirroring the `update_team` pattern at a smaller surface area.

No schema change, no new types, no new shared helpers. Two new tool files, two registration entries, inline tests.

## Problem Frame

`update_team` (`crates/mika-agent/src/tools/update_team.rs`, 708 lines) requires the caller to supply the full `agents: [...]` array for any composition change. For the common cases — "add Alice to the team", "remove Bob" — the LLM has to first recall or fetch the current roster, construct the complete new array, and call `update_team` with all members listed. This is more verbose than individual-verb tool calls would be, and arguably more error-prone (agent names must match exactly).

**Evidence status:** This is a **preemptive ergonomic improvement**, not a fix for an observed failure. The ticket is labeled `p3-nice-to-have`. No telemetry, no cited failing run, no user complaint references `update_team` mis-calls for single-member operations. The justification is a plausible hypothesis that individual mutation verbs (`add`, `remove`) are more discoverable for LLMs than "use update_team with a modified array" — true in general for tool-use ergonomics, but not demonstrated to be an active pain point here.

The zero-cost alternative would be one sentence in `update_team`'s tool description: *"For single-member changes, include the unchanged members plus the addition/removal — no other fields need to change."* That's worth trying first for anyone wanting to avoid adding two new tools without evidence they're needed. This plan proceeds with the dedicated-tools approach because that was the direction chosen during grooming, but the choice is a judgment call, not a forced one.

## Requirements Trace

- **R1.** `add_team_member(team_name, agent: {name, role, mandate})` adds a new member if: team exists, agent exists globally, agent is not already on the team.
- **R2.** `remove_team_member(team_name, agent_name)` removes a member if: team exists, agent is on the team, removal keeps the team at or above the 2-member minimum, the agent is not the orchestrator.
- **R3.** Both tools preserve the existing validation contract of `mika_common::team::validate_team` (orchestrator listed in agents, all agents exist globally, team name valid).
- **R4.** Both tools are registered under the same conditional guard as `update_team` (`agents.len() > 1 || !teams.is_empty()`).
- **R5.** No regression to existing team tools (`create_team`, `update_team`, `delete_team`, `run_team`).

## Scope Boundaries

### In scope

- `crates/mika-agent/src/tools/add_team_member.rs` (new)
- `crates/mika-agent/src/tools/remove_team_member.rs` (new)
- `crates/mika-agent/src/tools/mod.rs` — two `mod` declarations and two `tools.push(...)` registration lines inside the conditional block (next to `update_team`)
- Inline `#[cfg(test)] mod tests` in each new file

### Deferred to Separate Tasks

- **Compound "swap orchestrator" operation:** removing the orchestrator is rejected by `remove_team_member`. The caller does `update_team` (to change orchestrator), then `remove_team_member` (to remove the old one) — two steps. If this friction shows up in practice, a follow-up could add a `replace_orchestrator` tool. Out of scope.
- **Batch operations (`add_team_members`, `remove_team_members`):** YAGNI — the two single-item tools cover every observed need; batched variants add input-schema complexity for a rare case.
- **Updating an existing member's `role` or `mandate` in-place:** not a remove-and-re-add case; out of scope. Covered today by `update_team` with the full agents array (same capability that motivates this ticket, but less painful for a single-field edit than for composition changes).

## Context & Research

### Relevant Code and Patterns

- `crates/mika-agent/src/tools/update_team.rs:1-140` — canonical pattern for team-modifying tools: input validation, name normalization, `team_exists` check, `load_team`, validation, `toml::to_string_pretty` + `std::fs::write` (lines 262-273)
- `crates/mika-common/src/team.rs:56-154` — `validate_team_name`, `normalize_team_name`, `team_exists`, `load_team`, `validate_team`. All the validation primitives this plan needs are already exported.
- `crates/mika-common/src/agent.rs` — `agent_exists(home_dir, agent_name)` for the "agent exists globally" check
- `crates/mika-agent/src/tools/mod.rs:720-736` — registration block, conditional on `agents.len() > 1 || !teams.is_empty()`. This is where both new tools get registered next to `update_team`.
- `crates/mika-agent/CLAUDE.md` → "Management Tools" section: "Only default agent or team-listed orchestrators can delegate/run teams" — inherited via the existing orchestrator guard at the tool-registration level.

### Institutional Learnings

- No prior solution docs or brainstorms touch this specific surface. The pattern is well-established via `update_team` and the surrounding team-management tools.

## Key Technical Decisions

- **Two discrete tools, not a single `update_team` patch extension.** Adding `add_agents`/`remove_agents` params to `update_team` would grow its already-large input schema and blur its "replace the full array" semantics. Individual verbs match LLM tool-use ergonomics and are easier to audit in tool-call history. **Rationale:** LLMs are good at matching verbs; `update_team` already handles full replacements cleanly; don't overload it.
- **Orchestrator-removal is rejected.** `remove_team_member` returns an error when `agent_name == def.team.orchestrator`. Message points the caller at `update_team` to reassign the orchestrator first. **Rationale:** supporting atomic "remove + reappoint" would require a `new_orchestrator` parameter — that's a compound operation smell. Two-step flow (reassign via `update_team`, then remove old member) is acceptable friction for a rare case. If the rare case becomes common, we add `replace_orchestrator` later (deferred above).
- **Minimum team size enforced on removal.** `remove_team_member` rejects if `def.agents.len() - 1 < 2`. Matches the existing `update_team` contract at `update_team.rs:127-129`.
- **Validation always runs `validate_team` on both add and remove.** After each mutation, the tool calls `mika_common::team::validate_team(&home_dir, &def)` before serializing. This is the same downstream guard `update_team` relies on, so persistence semantics are identical across all three tools — no validation asymmetry, no orthogonality violation. If `validate_team` fails on the remove path because of a pre-existing orphan elsewhere in the roster (an agent listed in `team.toml` whose global definition has since been deleted), the tool returns an actionable error pointing at `update_team`: *"Team 'X' has an invalid roster — agent 'Y' no longer exists globally. Fix this first with `update_team`, then retry."* **Rationale:** silently tolerating orphans on one persistence path would be preserving a bug by making it removable one member at a time. Surfacing the orphan with a clear error preserves the invariant across all team-modifying tools.
- **What `validate_team` catches in the context of these tools** (per `mika-common/src/team.rs:129-153`): (1) the orchestrator still exists as a global agent, (2) every team member still exists as a global agent, (3) the orchestrator is still listed in the agents array. The relevant failure modes for this plan: on `add` — fails if `agent.name` was deleted globally between the step-4 `agent_exists` check and the save (tiny race window, but possible); on `remove` — fails if a pre-existing orphan is in the roster. Both are legitimate signals; surfacing them is correct.
- **Persistence mirrors `update_team`.** `toml::to_string_pretty(&def)` then `std::fs::write(team_dir.join("team.toml"), ...)` — same pattern as `update_team.rs:262-273`. No shared helper extracted; the pattern is ~4 lines and duplicating it avoids premature abstraction.
- **Output shape: text summary of the change.** Success: `"Added agent 'alice' to team 'inner-circle' (role: researcher)."` / `"Removed agent 'alice' from team 'inner-circle' (2 members remaining)."`. Mirrors `update_team`'s changelog-style output.
- **Concurrency: same guarantees as `update_team` today.** No file locking. Two concurrent calls to these tools (or to `update_team`) on the same `team.toml` can race; that's a pre-existing behavior not introduced by this plan.

## Open Questions

### Resolved During Planning

- **Split into two tools vs extend `update_team`?** Two tools — decided above.
- **Orchestrator removal: require new orchestrator, or reject?** Reject — decided above.
- **Minimum team size on removal?** 2 (matches existing contract).
- **Shared helper for load-mutate-validate-save?** No. Four calls per tool, barely any duplication, extracting would obscure the per-tool logic. Rule of three — if a third tool appears later with the same pattern, extract then.
- **Return the updated `TeamDefinition` or a text summary?** Text summary, matching `update_team`.

### Deferred to Implementation

- **Exact wording of the error messages.** Keep them short and actionable (point at `update_team` for orchestrator-replacement and for pre-existing orphan errors; reference the current roster for "already a member" / "not a member" cases). Low risk.

## Implementation Units

Both units are independent — they share no code and touch no common files beyond the two-line addition to `tools/mod.rs`. Land them in whatever order the implementer prefers.

- [ ] **Unit 1: `add_team_member` tool**

**Goal:** Append a single agent to an existing team's roster and persist.

**Requirements:** R1, R3, R4

**Dependencies:** None

**Files:**
- Create: `crates/mika-agent/src/tools/add_team_member.rs` — `AddTeamMemberTool` struct + `Tool` impl + inline `#[cfg(test)] mod tests`
- Modify: `crates/mika-agent/src/tools/mod.rs` — `mod add_team_member;` at the top with the other `mod` declarations; `tools.push(Box::new(add_team_member::AddTeamMemberTool { home_dir: home_dir.to_path_buf() }));` inside the conditional block next to `update_team`

**Approach:**
- Input schema:
  ```json
  {
    "type": "object",
    "properties": {
      "team_name": { "type": "string", "description": "Team to modify" },
      "agent": {
        "type": "object",
        "properties": {
          "name":    { "type": "string", "description": "Existing agent name" },
          "role":    { "type": "string", "description": "Role in the team" },
          "mandate": { "type": "string", "description": "What this agent is responsible for" }
        },
        "required": ["name", "role", "mandate"]
      }
    },
    "required": ["team_name", "agent"]
  }
  ```
- Steps in `execute`:
  1. Validate + normalize `team_name` (reuse `validate_team_name`, `normalize_team_name`)
  2. Validate `agent.name`, `agent.role`, `agent.mandate` are non-empty and within `super::MAX_INPUT_LEN` (the same 10,000-char cap `update_team` uses — do not redeclare)
  3. Check `team::team_exists(&self.home_dir, &team_name)` → error if not
  4. Check `agent::agent_exists(&self.home_dir, &agent.name)` → error if not
  5. `let mut def = team::load_team(&self.home_dir, &team_name)?;`
  6. Check `def.agents.iter().any(|a| a.name == agent.name)` → error "'{name}' is already a member of '{team_name}'"
  7. Push new `TeamAgent { name, role, mandate }` into `def.agents`
  8. `team::validate_team(&self.home_dir, &def)` — load-bearing, not defensive. Catches: orchestrator-still-in-roster invariant, globally-deleted agents elsewhere in the team (pre-existing orphans), and a race where `agent.name` was deleted globally between step 4 and this step. On failure, return an error that mentions the specific invariant `validate_team` surfaced.
  9. `toml::to_string_pretty(&def)` → `std::fs::write(team_dir(&home_dir, &team_name).join("team.toml"), ...)`
  10. Return `ToolOutput::success(format!("Added agent '{}' to team '{}' (role: {}). Team now has {} members.", ...))`
- Orchestrator guard is inherited from the registration conditional in `tools/mod.rs` (same guard as `update_team`); no per-tool check needed.

**Patterns to follow:**
- `crates/mika-agent/src/tools/update_team.rs:73-140` for the validation-and-normalize sequence
- `update_team.rs:262-273` for the serialize + write pattern
- Existing `#[cfg(test)] mod tests` in `update_team.rs:280+` for fixture setup (test agent dir, team.toml scaffold)

**Test scenarios:**
- **Happy path:** add a valid new member → returns success, `team.toml` contains the new member, team size incremented.
- **Edge case (already a member):** call `add_team_member` with a name already in the team → error, `team.toml` unchanged.
- **Edge case (agent doesn't exist globally):** `agent.name` doesn't match any agent dir → error, `team.toml` unchanged.
- **Edge case (team doesn't exist):** `team_name` doesn't match any team → error.
- **Edge case (invalid team name):** team_name contains disallowed characters → error from `validate_team_name`.
- **Edge case (empty fields):** missing `agent.name` / `agent.role` / `agent.mandate` → error.

**Verification:**
- `cargo test -p mika-agent tools::add_team_member` passes.

---

- [ ] **Unit 2: `remove_team_member` tool**

**Goal:** Remove a single agent from an existing team's roster and persist.

**Requirements:** R2, R3, R4

**Dependencies:** None (independent of Unit 1; they share no code)

**Files:**
- Create: `crates/mika-agent/src/tools/remove_team_member.rs` — `RemoveTeamMemberTool` struct + `Tool` impl + inline `#[cfg(test)] mod tests`
- Modify: `crates/mika-agent/src/tools/mod.rs` — `mod remove_team_member;` + `tools.push(Box::new(remove_team_member::RemoveTeamMemberTool { home_dir: home_dir.to_path_buf() }));` in the same conditional block

**Approach:**
- Input schema:
  ```json
  {
    "type": "object",
    "properties": {
      "team_name":  { "type": "string", "description": "Team to modify" },
      "agent_name": { "type": "string", "description": "Agent to remove" }
    },
    "required": ["team_name", "agent_name"]
  }
  ```
- Steps in `execute`:
  1. Validate + normalize `team_name`
  2. Validate `agent_name` is non-empty and within `super::MAX_INPUT_LEN`
  3. `team_exists` → error if not
  4. `let mut def = team::load_team(...)?;`
  5. Check `def.agents.iter().any(|a| a.name == agent_name)` → error "'{name}' is not a member of '{team_name}'"
  6. Orchestrator guard: `if def.team.orchestrator == agent_name` → error "'{name}' is the orchestrator of '{team_name}'. Reassign the orchestrator via `update_team` before removing this agent."
  7. Min-size check: `if def.agents.len() - 1 < 2` → error "Removing '{name}' would leave '{team_name}' with fewer than 2 members."
  8. Retain filter: `def.agents.retain(|a| a.name != agent_name);`
  9. `team::validate_team(&self.home_dir, &def)` — same symmetric call as the add path. If it fails because of a pre-existing orphan elsewhere in the roster (a different team member whose global definition has since been deleted), return an actionable error: *"Team '{team_name}' has an invalid roster — agent 'Y' no longer exists globally. Fix this first with `update_team`, then retry the removal."* This preserves the invariant that all three team-modifying tools enforce the same validation contract; orphans are surfaced, not silently tolerated.
  10. `toml::to_string_pretty(&def)` → `std::fs::write(...)`
  11. Return `ToolOutput::success(format!("Removed agent '{}' from team '{}' ({} members remaining).", ...))`

**Patterns to follow:**
- Same patterns as Unit 1 (input validation, serialize, write)
- `update_team.rs:127-129` for the min-size check structure

**Test scenarios:**
- **Happy path:** remove a non-orchestrator member from a 3-member team → success, file updated, team size decremented.
- **Edge case (not a member):** `agent_name` not in roster → error, file unchanged.
- **Edge case (is orchestrator):** `agent_name == team.orchestrator` → error, file unchanged, message references `update_team`.
- **Edge case (would drop below minimum):** remove from a 2-member team → error, file unchanged.
- **Edge case (team doesn't exist):** `team_name` doesn't match → error.
- **Edge case (pre-existing orphan in roster):** remove a valid member from a team whose roster includes a globally-deleted agent (orphan) that isn't the target → **error**, file unchanged, message names the orphan and points at `update_team`. (Tests the validation-symmetry decision — orphans are surfaced, not silently tolerated.)

**Verification:**
- `cargo test -p mika-agent tools::remove_team_member` passes.

## System-Wide Impact

- **Interaction graph:** Two new tools in the team-management family; registered alongside `update_team`. No changes to other tools or engine paths.
- **Error propagation:** Standard `ToolOutput::error` for all failure modes; mirrors peer tools' behavior.
- **State lifecycle risks:** Same file-write race window as `update_team` today. Not worsened.
- **API surface parity:** No external API change. Tool registry gains two entries; tool schemas are new but additive.
- **Integration coverage:** Inline unit tests per tool using the same fixture scaffolding as `update_team.rs` tests. No new integration-harness work required.
- **Unchanged invariants:** `TeamDefinition` schema, `team.toml` format, all existing team tools, orchestrator-permission guard at the tool-registration level, minimum team size of 2.

## Risks & Dependencies

| Risk | Mitigation |
|------|------------|
| Adding two tools grows the tool registry the LLM sees; marginal context bloat per turn. | Each tool schema is small (~10 lines of JSON). Net growth is < 500 chars in the tool list. Acceptable for the UX improvement. |
| Tools ship without observed evidence they solve a real problem. | Called out explicitly in Problem Frame. Acceptable for a p3-nice-to-have enhancement; post-merge, watch `tool_calls` telemetry for (a) adoption of the new verbs vs continued use of `update_team` for single-member ops, (b) any increase in tool-call error rate in this family. If adoption is low, the hypothesis was wrong and the tools are carrying their weight only for auditability; that's information, not a regression. |

## Documentation / Operational Notes

- No deployment, migration, or rollout concerns. Purely additive.
- **Doc-update checklist (all in the same PR):**
  - `crates/mika-agent/CLAUDE.md` § Management Tools: currently lists "12 tools" with an enumerated list; update count to 14 and add `add_team_member` / `remove_team_member` to the list.
  - `docs/architecture.md:68`: currently says "27 builtin tools + 10 management tools (3 always-on + 7 conditional)" — the `10` and `7 conditional` counts drift here (line 235 says "12" and "remaining 9 tools"). This plan adds two more conditional tools, so the correct post-merge values need both lines updated consistently. Pick one source of truth (suggested: line 235's breakdown) and reconcile the line-68 summary to match.
  - `docs/architecture.md:235+` management-tools table: add two rows for the new tools.
  - Grep for any other `"12 tools"` / `"10 management tools"` callsites before merge to catch drift.

## Sources & References

- **Origin issue:** [senara-solutions/mika#283](https://github.com/senara-solutions/mika/issues/283)
- Related code: `crates/mika-agent/src/tools/update_team.rs`, `crates/mika-common/src/team.rs`, `crates/mika-agent/src/tools/mod.rs`
- Pattern source: `update_team.rs:73-273` — canonical team-modifying-tool structure
