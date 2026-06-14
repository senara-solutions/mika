---
title: "feat: Rename --skill-always-on to --enable-skill / --disable-skill and revert self-dev to always_on=true"
type: feat
status: active
date: 2026-04-20
---

# Rename --skill-always-on to --enable-skill / --disable-skill

## Overview

Replace the `--skill-always-on` CLI flag with two flags: `--enable-skill` (transient force-on) and `--disable-skill` (transient force-off). Revert self-dev to `always_on = true` so it loads automatically on webhook/callback paths. Update `claude-pilot.json` to remove `--skill-always-on self-dev` (no longer needed).

## Problem Frame

PR #675 changed self-dev to `always_on = false` and added `--skill-always-on` to force it on during claude-pilot sessions. This broke webhook-triggered turns: the gateway has no mechanism to pass `--skill-always-on`, so self-dev doesn't load for GitHub event callbacks. The better design is the inverse: keep self-dev always-on by default and let interactive users disable it transiently with `--disable-skill self-dev`.

## Requirements Trace

- R1. `--skill-always-on` is renamed to `--enable-skill` (same semantics: transient force-on)
- R2. New `--disable-skill` flag added (transient force-off via eviction from registry)
- R3. Both flags are repeatable (`Vec<String>`) and mutually exclusive per skill name
- R4. Same skill in both flags produces a hard error before any registry operations
- R5. self-dev reverted to `always_on = true` in `skills/bundled/self-dev/skill.toml`
- R6. `claude-pilot.json` updated: remove `--skill-always-on self-dev` (no longer needed)
- R7. Warning messages reference the new flag names
- R8. Both flags scoped to `AskArgs` only, both with `conflicts_with = "team"`

## Scope Boundaries

- No rename of the `always_on` field in `SkillInfo`, DB schema, or internal APIs — this is a CLI-surface rename only
- No changes to `mika chat` (TUI) — transient overrides remain `mika ask`-only
- No deprecated alias for `--skill-always-on` — all callers are within the workspace

### Deferred to Separate Tasks

- Update `claude-pilot.json` in other repos (`mika-skills`, `mika-cloud`, `claude-pilot-py`, `mika-platform`): separate PRs in those repos after this merges

## Context & Research

### Relevant Code and Patterns

- `crates/mika-cli/src/cli.rs:193-198` — `AskArgs.skill_always_on: Vec<String>` with `conflicts_with = "team"`
- `crates/mika-cli/src/main.rs:255` — passes `&args.skill_always_on` to `ask::run()`
- `crates/mika-cli/src/commands/ask.rs:36,248-259` — calls `apply_transient_always_on()`, emits warnings
- `crates/mika-agent/src/skills/mod.rs:463-490` — `apply_transient_always_on()` method
- `crates/mika-agent/src/skills/mod.rs:130-146` — `TransientOverrideResult` struct
- `crates/mika-agent/src/skills/mod.rs:378-449` — `apply_overrides()` Phase 0 eviction pattern (model for transient disable)
- `crates/mika-agent/src/skills/mod.rs:2213-2347` — existing transient always_on tests
- `.claude/claude-pilot.json` — autonomous dispatch config

### Institutional Learnings

- `docs/solutions/architecture-patterns/cli-skill-always-on-transient-override.md` — documents the current pattern; must be updated
- `docs/solutions/architecture-patterns/skill-enabled-state-db-eviction.md` — eviction precedence: `enabled=false` wins over `always_on=true`; eviction before mutation
- `docs/solutions/architecture-patterns/cli-flag-id-suffix-convention.md` — `--{noun}` for human-readable names, `--{noun}-id` for opaque IDs
- `docs/solutions/architecture-patterns/cli-flag-subcommand-scoping.md` — keep flags scoped to `AskArgs`

## Key Technical Decisions

- **Transient disable uses full eviction (not flag toggle):** Setting `always_on = false` would still allow keyword matching. Full eviction from `self.skills` → `self.disabled` prevents all activation paths, consistent with DB `enabled=false` eviction in `apply_overrides()` Phase 0.
- **Ordering: disable first, enable second:** Matches `apply_overrides()` pattern (Phase 0 eviction, Phase 1 mutation). Prevents logical conflicts.
- **Case-insensitive conflict detection:** `eq_ignore_ascii_case` for checking same-skill-in-both-flags, consistent with all other skill name comparisons.
- **No dependency-chain warnings for transient disable:** Existing `validate_loaded()` doesn't check dependencies. Adding it for transient disable adds complexity for an edge case. Users choosing `--disable-skill` are expected to understand the consequence. Keep it simple — no new dependency validation.
- **CLI field rename only:** The internal `always_on` field name in `SkillInfo`, `SkillOverride`, and DB schema stays unchanged. The issue asks to rename the CLI flag, not the internal representation.

## Open Questions

### Resolved During Planning

- **Should `--enable-skill` warn for already-always-on skills?** No — idempotent behavior is correct (current behavior preserved).
- **Should `--disable-skill` have `conflicts_with = "team"`?** Yes — team mode builds its own registry, transient flags have no effect there.
- **Should `claude-pilot.json` get `--enable-skill` or be stripped entirely?** Stripped entirely — self-dev reverts to `always_on = true`, so no flag is needed.

### Deferred to Implementation

- Exact warning message wording for `--disable-skill` edge cases — will mirror the existing pattern

## Implementation Units

- [ ] **Unit 1: Revert self-dev to always_on=true**

**Goal:** Change self-dev manifest so it loads automatically on all paths (webhook, callback, autonomous, interactive).

**Requirements:** R5

**Dependencies:** None

**Files:**
- Modify: `skills/bundled/self-dev/skill.toml`

**Approach:**
- Change `always_on = false` to `always_on = true`

**Patterns to follow:**
- `skills/bundled/qa-review/skill.toml` (only other always_on=true skill)

**Test expectation:** none — manifest-only change, verified by existing skill loading tests

**Verification:**
- `cargo build` succeeds (build.rs re-discovers the skill)

- [ ] **Unit 2: Rename CLI flag and add --disable-skill**

**Goal:** Replace `--skill-always-on` with `--enable-skill` and add `--disable-skill` on `AskArgs`. Wire both through `main.rs` to `ask::run()`.

**Requirements:** R1, R2, R3, R8

**Dependencies:** None

**Files:**
- Modify: `crates/mika-cli/src/cli.rs`
- Modify: `crates/mika-cli/src/main.rs`
- Modify: `crates/mika-cli/src/commands/ask.rs`

**Approach:**
- In `AskArgs`: rename `skill_always_on` field to `enable_skill`, add `disable_skill: Vec<String>` — both with `#[arg(long, conflicts_with = "team")]`
- In `main.rs`: pass both `&args.enable_skill` and `&args.disable_skill` to `ask::run()`
- In `ask.rs`: update `run()` signature to accept both lists. Add early conflict check (case-insensitive intersection → `anyhow::bail!`). Call `apply_transient_always_on()` for enable list (method name unchanged — it's internal). Call new `apply_transient_disable()` for disable list. Order: disable first, then enable.
- Update warning messages to reference `--enable-skill` and `--disable-skill`

**Patterns to follow:**
- Existing `--skill-always-on` pattern in `cli.rs` and `ask.rs`
- `apply_overrides()` Phase 0 eviction for the disable flow

**Test scenarios:**
- Happy path: `--enable-skill a` sets always_on=true on skill a (existing tests, update field name)
- Happy path: `--disable-skill b` with b loaded — b is evicted from registry
- Edge case: `--enable-skill a --disable-skill a` — hard error before any registry ops
- Edge case: case-insensitive conflict detection (`--enable-skill Self-Dev --disable-skill self-dev`)
- Edge case: `--disable-skill x` where x is not loaded — returns not_found
- Edge case: `--disable-skill x` where x is already DB-disabled — no-op (already evicted)
- Edge case: `--enable-skill x` where x is DB-disabled — returns disabled warning

**Verification:**
- `cargo build` succeeds
- `cargo test -p mika-cli` passes
- `mika ask --help` shows `--enable-skill` and `--disable-skill`, no `--skill-always-on`

- [ ] **Unit 3: Add apply_transient_disable() to SkillRegistry**

**Goal:** Implement the transient disable method that evicts named skills from the registry for a single invocation.

**Requirements:** R2, R3

**Dependencies:** Unit 2 (caller exists)

**Files:**
- Modify: `crates/mika-agent/src/skills/mod.rs`
- Test: `crates/mika-agent/src/skills/mod.rs` (inline `#[cfg(test)] mod tests`)

**Approach:**
- Add `TransientDisableResult` struct with `not_found: Vec<String>` field
- Add `apply_transient_disable(&mut self, skill_names: &[String]) -> TransientDisableResult` method
- Implementation: for each name, find matching entry in `self.skills` (case-insensitive). If found, remove from `self.skills`, push to `self.disabled` with reason. If not found (and not in `self.disabled` already), add to `not_found`.
- Follow `apply_overrides()` Phase 0 pattern: collect names → `retain()` on `self.skills` → push evicted to `self.disabled`

**Patterns to follow:**
- `apply_overrides()` lines 379-406 — Phase 0 eviction pattern
- `apply_transient_always_on()` — method shape and result type pattern
- `DisabledSkill` struct for evicted entries

**Test scenarios:**
- Happy path: disable a loaded skill — removed from `skills`, present in `disabled`
- Happy path: disable multiple skills — all evicted
- Edge case: disable nonexistent skill — returns in `not_found`
- Edge case: disable already-DB-disabled skill — no-op (already in `self.disabled`)
- Edge case: case-insensitive matching (`Self-Dev` disables `self-dev`)
- Edge case: empty input — no-op
- Integration: disabled skill no longer appears in `match_skills()` results
- Integration: disabled skill no longer in `always_on_skills()` even if it had `always_on=true`

**Verification:**
- `cargo test -p mika-agent -- transient_disable` passes
- All existing skill tests still pass

- [ ] **Unit 4: Update claude-pilot.json**

**Goal:** Remove `--skill-always-on self-dev` from the mika repo's claude-pilot config since self-dev is now always_on=true.

**Requirements:** R6

**Dependencies:** Unit 1 (self-dev is always_on)

**Files:**
- Modify: `.claude/claude-pilot.json`

**Approach:**
- Remove `"--skill-always-on", "self-dev"` from the args array, keeping just `["--agent", "mika-dev", "ask"]`

**Test expectation:** none — JSON config file, no behavioral test

**Verification:**
- JSON is valid
- Args no longer reference `--skill-always-on`

- [ ] **Unit 5: Update solution doc and existing plan reference**

**Goal:** Update the compound solution doc to reflect the renamed flag and new `--disable-skill` capability.

**Requirements:** R7

**Dependencies:** Units 1-4

**Files:**
- Modify: `docs/solutions/architecture-patterns/cli-skill-always-on-transient-override.md`

**Approach:**
- Rename references from `--skill-always-on` to `--enable-skill`
- Add `--disable-skill` documentation
- Update the call sequence to show disable-first, enable-second ordering
- Update the example usage section

**Test expectation:** none — documentation only

**Verification:**
- No references to `--skill-always-on` remain in the doc
- Both `--enable-skill` and `--disable-skill` are documented

## System-Wide Impact

- **Interaction graph:** `ask::run()` → `apply_transient_disable()` → `SkillRegistry.disabled` eviction. No callbacks, middleware, or observers affected.
- **Error propagation:** Conflict check in `ask.rs` uses `anyhow::bail!` (exits before agent loop). Warnings go to stderr (non-fatal).
- **State lifecycle risks:** None — transient overrides are per-invocation, no persistence.
- **API surface parity:** Server path (`mika-spirit`) does not use transient overrides and is unaffected. `mika chat` (TUI) is unaffected.
- **Unchanged invariants:** `apply_overrides()` DB-based enable/disable is unchanged. `SkillInfo.always_on` field name is unchanged. DB schema is unchanged. `toggle_skill`, `update_skill`, `list_skills` agent tools are unchanged. The `always_on_skills()`, `safe_always_on_skills()`, `callback_safe_skills()` methods are unchanged.

## Risks & Dependencies

| Risk | Mitigation |
|------|------------|
| Other repos' `claude-pilot.json` still references `--skill-always-on` | Deferred to separate PRs. Those repos invoke `mika` binary — until they update, autonomous sessions would fail. Coordinate update after merge. |
| Removing `--skill-always-on` breaks any scripts or aliases | Only known callers are `claude-pilot.json` files, all within workspace. No external callers. |

## Sources & References

- Related issue: #682
- Related PR: #675 (introduced `--skill-always-on`)
- Solution doc: `docs/solutions/architecture-patterns/cli-skill-always-on-transient-override.md`
- Solution doc: `docs/solutions/architecture-patterns/skill-enabled-state-db-eviction.md`
