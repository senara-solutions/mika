---
title: "refactor: Hard-skip oversized skills, split startup log, delete match-time filter"
type: refactor
status: active
date: 2026-04-18
---

# Hard-skip oversized skills + split startup log + delete match-time filter

## Overview

Four follow-up cleanups after #629 (skill enabled state moved to DB):
1. Hard-skip non-`always_on` skills with oversized prompts instead of silently loading them with empty prompts
2. Split the startup skills log into three-state counts (loaded/disabled/skipped)
3. Delete the now-dead match-time disabled filter in `matcher.rs`
4. Update `/skills` CLI output to show three-state counts

## Problem Frame

After #629, disabled skills are evicted from the registry by `apply_overrides()`. But two cleanup items remain:

- **Oversized non-always_on footgun:** A skill with an oversized prompt gets its prompt silently emptied but still loads into the registry with its tools. If triggered, the LLM has tools but no system prompt context — producing garbage. The `always_on` path correctly skips the skill entirely; the non-`always_on` path should be symmetric.
- **Conflated startup log:** The `skills loaded count=25` line fires before `apply_overrides()` and lumps all states together. After #629 introduced the disabled/evicted concept, the log should reflect three distinct states.
- **Dead code:** The `if !entry.enabled { continue; }` guard in `matcher.rs:44-46` is now belt-and-suspenders since `apply_overrides()` evicts disabled skills before matching ever runs. Safe to remove after #629 has soaked.
- **CLI count mismatch:** `Skills (25):` lumps everything; should match the three-state log structure.

## Requirements Trace

- R1. Oversized non-`always_on` skills are skipped (pushed to `ScanResult.skipped`), not loaded with empty prompt
- R2. Startup log shows three separate counts: `loaded=N disabled=N skipped=N`
- R3. Per-skip WARN line with skill name and reason
- R4. `matcher.rs` disabled filter (`if !entry.enabled { continue; }`) is deleted
- R5. `/skills` CLI text output shows split counts in header
- R6. Tool-only skills (no `system_prompt.md` at all) still load correctly (regression guard)
- R7. Post-override `always_on` oversized validation in `apply_overrides()` is now dead code — remove it

## Scope Boundaries

- No changes to the `SnippetLoadResult` enum itself
- No changes to `ReadError` handling for non-`always_on` — that path is a separate concern (prompt file exists but unreadable is different from prompt exceeds limit)
- No changes to bundled skills discovery or `seed_bundled_skills()`

## Context & Research

### Relevant Code and Patterns

- `crates/mika-agent/src/skills/index.rs:499-528` — `SnippetLoadResult::Oversized` handling (the divergent paths)
- `crates/mika-agent/src/skills/mod.rs:140-168` — `SkillRegistry::from_dir()` constructor and startup log
- `crates/mika-agent/src/skills/mod.rs:341-457` — `apply_overrides()` including post-override oversized validation
- `crates/mika-agent/src/skills/matcher.rs:44-46` — dead `!entry.enabled` filter
- `crates/mika-cli/src/commands/skills.rs:326-388` — CLI text output with `Skills ({count}):` header
- `crates/mika-agent/src/skills/index.rs:2232-2253` — test `test_scan_non_always_on_with_oversized_prompt_still_loads` (must be updated)

### Institutional Learnings

- `docs/solutions/integration-issues/always-on-skill-oversized-prompt-loud-failure.md` — documents the asymmetry and explicitly references #630 as needing hard-skip symmetry
- `docs/solutions/architecture-patterns/skill-enabled-state-db-eviction.md` — documents the #629 eviction pattern
- `docs/solutions/dev-loop/cli-log-noise-stderr-pollutes-piped-commands-2026-04-17.md` — suggests demoting `skills loaded` INFO to DEBUG; this plan does not change log level (separate concern) but does restructure the log content

## Key Technical Decisions

- **Move the startup log after `apply_overrides()`:** Currently the log fires in `from_dir()` before overrides run, so `disabled` count is always 0. The log must move to after `apply_overrides()` to show accurate three-state counts. This means adding a dedicated `log_summary()` method on `SkillRegistry` that callers invoke after `apply_overrides()`, rather than logging inside `from_dir()`.
- **Delete post-override oversized validation:** The block at `mod.rs:414-456` (`apply_overrides()`) that catches `always_on` override + empty prompt + oversized file becomes dead code once non-`always_on` oversized skills are hard-skipped at scan time (they never enter `entries` at all). Remove it.
- **`ReadError` stays asymmetric for now:** Non-`always_on` + `ReadError` still loads with empty prompt. A missing/unreadable prompt file is a different failure class — the file might appear later, or the skill might be tool-only with an optional prompt. Hardening that path is out of scope.

## Implementation Units

- [x] **Unit 1: Hard-skip non-always_on oversized skills at scan time**

**Goal:** Make the `Oversized` branch in `scan_skills_dir()` always skip the skill, regardless of `always_on` status.

**Requirements:** R1, R7

**Dependencies:** None

**Files:**
- Modify: `crates/mika-agent/src/skills/index.rs`
- Modify: `crates/mika-agent/src/skills/mod.rs`
- Test: `crates/mika-agent/src/skills/index.rs` (inline tests)

**Approach:**
- In `index.rs`, collapse the `Oversized` match arm: remove the `if manifest.skill.always_on` branch and make all `Oversized` cases push to `skipped` and `continue`. The error log message should be unified (drop the "prompt will be empty" variant).
- In `mod.rs`, delete the post-override `always_on` oversized validation block in `apply_overrides()` (lines ~414-456) since the edge case it catches can no longer occur.
- Update `test_scan_non_always_on_with_oversized_prompt_still_loads` to assert the opposite: skill is skipped, not loaded.

**Patterns to follow:**
- Existing `always_on` + `Oversized` path at `index.rs:503-518` — same pattern of `skipped.push()` + `continue`

**Test scenarios:**
- Happy path: Oversized non-always_on skill is skipped with reason containing size and limit
- Happy path: Oversized always_on skill is still skipped (no regression)
- Edge case: Skill with no `system_prompt.md` at all (tool-only) still loads with empty prompt via `SnippetLoadResult::Empty` (R6 regression guard)
- Edge case: Skill with prompt under limit loads normally

**Verification:**
- `cargo test -p mika-agent -- scan` passes with updated assertions

- [x] **Unit 2: Move startup log after apply_overrides and split into three states**

**Goal:** Replace the single `skills loaded count=N` log with a three-state summary and per-skip WARN lines.

**Requirements:** R2, R3

**Dependencies:** Unit 1

**Files:**
- Modify: `crates/mika-agent/src/skills/mod.rs`
- Modify: `crates/mika-cli/src/commands/skills.rs` (caller site)
- Modify: `crates/mika-agent/src/tools/list_skills.rs` (caller site)
- Modify: `crates/mika-agent/src/server/handlers.rs` (caller site)
- Modify: `crates/mika-agent/src/server/a2a.rs` (caller site)
- Modify: `crates/mika-agent/src/teams/engine.rs` (caller site)
- Modify: `crates/mika-agent/src/tools/delegate_task.rs` (caller site)

**Approach:**
- Remove the `tracing::info!` and `tracing::warn!` from `from_dir()` (currently at lines 144-160).
- Add a `pub fn log_summary(&self)` method on `SkillRegistry` that emits:
  - `tracing::info!(loaded = self.skills.len(), disabled = self.disabled.len(), skipped = self.skipped.len(), "skills loaded")`
  - For each skipped skill: `tracing::warn!(name = %s.name, reason = %s.reason, "skill skipped")`
- Each caller site that calls `apply_overrides()` should call `registry.log_summary()` after it. Callers that don't call `apply_overrides()` (like `a2a_card.rs` tests) get no log — acceptable since those are test-only paths.
- The existing `tracing::warn!` about "run `mika skills validate`" can be folded into the per-skip WARN lines (the suggestion becomes part of each skip reason or a footer log).

**Patterns to follow:**
- Existing structured `tracing::info!` style used throughout `mod.rs`

**Test scenarios:**
- Integration: After `from_dir()` + `apply_overrides()` + `log_summary()`, the three counts reflect reality (e.g., 2 loaded, 1 disabled, 1 skipped)

**Verification:**
- `cargo test -p mika-agent -- skills` passes
- Manual: `mika skills list` startup shows the new log format

- [x] **Unit 3: Delete match-time disabled filter**

**Goal:** Remove the dead `if !entry.enabled { continue; }` guard from `match_skills()`.

**Requirements:** R4

**Dependencies:** Unit 1 (disabled skills are now evicted before matching)

**Files:**
- Modify: `crates/mika-agent/src/skills/matcher.rs`
- Test: `crates/mika-agent/src/skills/matcher.rs` (inline tests, if any test the disabled path)

**Approach:**
- Delete lines 44-46 in `matcher.rs`. The `enabled` field on `SkillEntry` is still present (it's used in JSON output) but is always `true` for skills in the registry since disabled skills are evicted.

**Patterns to follow:**
- Clean deletion — no replacement needed

**Test scenarios:**
- Happy path: Skills with `enabled: true` still match on keywords (no regression)
- Happy path: `always_on` matching unaffected

**Verification:**
- `cargo test -p mika-agent -- match` passes
- `cargo clippy -p mika-agent` reports no new warnings

- [x] **Unit 4: Update /skills CLI output with three-state counts**

**Goal:** Change the CLI header from `Skills (25):` to `Skills: 16 loaded, 9 disabled, 1 skipped` and add a skipped section.

**Requirements:** R5

**Dependencies:** Unit 1

**Files:**
- Modify: `crates/mika-cli/src/commands/skills.rs`

**Approach:**
- In the text output branch, change the header line to show three counts: loaded (from `registry.skills().len()`), disabled (from `registry.disabled().len()`), skipped (from `registry.skipped().len()`)
- Add a `Skipped (N):` section after `Disabled` showing each skipped skill's name and reason
- JSON output: add `disabled` and `skipped` arrays to the top-level output for completeness

**Patterns to follow:**
- Existing `Disabled ({count}):` section pattern at `skills.rs:382-387`

**Test scenarios:**
- Happy path: CLI shows `Skills: N loaded, M disabled, K skipped` header
- Edge case: All zeros except loaded — no disabled/skipped sections shown
- Edge case: Skipped section shows reason string

**Verification:**
- `cargo build --bin mika` succeeds
- Manual: `mika skills list` shows updated output format

## System-Wide Impact

- **Interaction graph:** The log change affects every call site that constructs a `SkillRegistry` and calls `apply_overrides()` — there are 6 such sites. Each needs `log_summary()` added.
- **Error propagation:** No change — skip behavior already existed for `always_on`; this extends it symmetrically.
- **State lifecycle risks:** None — this is purely about which skills enter the registry, not about runtime state.
- **API surface parity:** The JSON output of `mika skills list` should also reflect skipped skills for scripting consumers.
- **Unchanged invariants:** `SkillEntry.enabled` field remains on the struct but is always `true` for entries in the registry (disabled ones are evicted). The `enabled` field in JSON output stays for backward compatibility.

## Risks & Dependencies

| Risk | Mitigation |
|------|------------|
| Caller sites miss `log_summary()` call | Grep for all `apply_overrides()` call sites; compile-time there's no enforcement but the method is clearly documented |
| Removing post-override oversized validation misses an edge case | The only case it caught was non-`always_on` scanned with empty prompt + later DB override to `always_on`. After Unit 1, non-`always_on` oversized skills are skipped entirely, so they never enter `entries` and can't be overridden. The edge case is eliminated at the source. |

## Sources & References

- Related issue: #630
- Depends on: #629 (skill enabled state DB eviction — must be merged first)
- Origin learnings: `docs/solutions/integration-issues/always-on-skill-oversized-prompt-loud-failure.md`
