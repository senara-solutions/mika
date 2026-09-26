---
title: "feat: Add --skill-always-on CLI flag to mika ask; default self-dev to always_on=false"
type: feat
status: active
date: 2026-04-20
issue: 670
---

# feat: Add --skill-always-on CLI flag to mika ask; default self-dev to always_on=false

## Overview

Decouple the `self-dev` skill from always-on activation and add a `--skill-always-on` CLI flag to `mika ask` that forces named skills into always-on mode for that invocation. This lets interactive sessions be conversational by default while autonomous dispatches lock in the exact skills they need.

## Problem Frame

`self-dev` is `always_on = true` in its manifest, meaning every message to mika-dev activates self-dev — even for simple questions or non-implementation conversations. This makes interactive sessions frustrating. The autonomous dev loop (`claude-pilot`) always knows it wants `self-dev` and can explicitly request it, so the manifest default should favor the interactive case.

## Requirements Trace

- R1. `self-dev` skill defaults to `always_on = false` in its manifest
- R2. `mika ask` accepts `--skill-always-on <name>` flag, repeatable for multiple skills
- R3. The flag forces the named skill(s) to `always_on = true` for that invocation only (not persisted)
- R4. The transient override applies after DB overrides, stacking on top of both manifest defaults and DB state
- R5. If a requested skill is disabled (evicted) or not found, emit a warning to stderr and continue
- R6. The flag should not conflict with `--team` mode (team mode builds its own skill registry, but the flag can coexist for future extensibility)

## Scope Boundaries

- CLI flag on `mika ask` only — not on `mika chat` (TUI has its own lifecycle, out of scope)
- No HTTP API changes — the `MessageRequest` struct is unchanged; server always_on is managed via DB overrides
- No schema migration — this is a runtime-only overlay, not a new DB column
- No changes to silent mode skill selection (`safe_always_on_skills`, `callback_safe_skills`)

### Deferred to Separate Tasks

- HTTP `MessageRequest` skill override field: separate PR when the server needs per-request always_on
- TUI `/skill-always-on` command or chat-mode equivalent: separate PR if needed

## Context & Research

### Relevant Code and Patterns

- `crates/mika-cli/src/cli.rs` — `AskArgs` struct (line 155), clap `#[arg]` attributes
- `crates/mika-cli/src/main.rs` — `Commands::Ask` dispatch (line 245)
- `crates/mika-cli/src/commands/ask.rs` — `run()` function, skill registry construction (lines 237-257)
- `crates/mika-agent/src/skills/mod.rs` — `SkillRegistry`, `apply_overrides()` (line 349), `always_on_skills()` (line 334)
- `crates/mika-agent/src/skills/matcher.rs` — `match_skills()` checks `entry.manifest.skill.always_on` (line 51)
- `skills/bundled/self-dev/skill.toml` — current `always_on = true` (line 5)

### Institutional Learnings

- **`--model` one-shot override pattern** (`docs/solutions/architecture-patterns/cli-model-override-one-shot.md`): Add field to `AskArgs`, extract in `main.rs`, apply in `ask.rs` before `Arc::new()`. Same pattern applies here.
- **CLI flag scoping** (`docs/solutions/architecture-patterns/cli-flag-subcommand-scoping.md`): Do not use `global = true`. Add to `AskArgs` directly.
- **Skill override precedence** (`docs/solutions/architecture-patterns/skill-enabled-state-db-eviction.md`): `enabled=false` always wins over `always_on=true`. Disabled skills are evicted before `apply_overrides()` runs. The CLI flag cannot resurrect evicted skills — warn instead.
- **Oversized prompt handling** (`docs/solutions/integration-issues/always-on-skill-oversized-prompt-loud-failure.md`): Skills exceeding `max_prompt_size` are hard-skipped at scan time regardless of `always_on`. The CLI flag cannot override this — the skill is already excluded from the registry.

## Key Technical Decisions

- **Transient overlay, not DB mutation**: The flag creates ephemeral `SkillOverride` entries applied alongside (after) DB overrides. Nothing is persisted. This follows the `--model` override pattern.
- **Apply after `apply_overrides()` as a second pass**: Rather than injecting synthetic entries into the DB overrides vector (which would mix transient and persistent state), apply CLI overrides as a separate method call on `SkillRegistry`. This keeps the concerns clean.
- **New method `apply_transient_always_on()`**: A dedicated method on `SkillRegistry` that takes `&[String]` skill names and sets `always_on = true` on matching entries. Returns a list of names that were not found (for warning output). This is simpler than creating full `SkillOverride` structs and re-running `apply_overrides()`.
- **Repeatable flag via `Vec<String>`**: Use clap's built-in support for repeatable flags: `--skill-always-on foo --skill-always-on bar`. Default to empty vec (no-op when not provided).

## Open Questions

### Resolved During Planning

- **Should the flag conflict with `--team`?** No — team mode builds its own skill registry in the team engine, so the flag has no effect there today. But adding a conflict would prevent future extensibility. Instead, the flag is silently ignored in team mode (the team path in `ask.rs` doesn't use the flag's value).
- **Should the flag accept comma-separated values?** No — clap's repeatable flag pattern (`--skill-always-on a --skill-always-on b`) is the established CLI convention in this codebase. Comma parsing adds complexity and ambiguity (skill names could theoretically contain commas).

### Deferred to Implementation

- Exact warning message wording for unfound/disabled skills

## Implementation Units

- [x] **Unit 1: Flip self-dev to always_on=false**

**Goal:** Change the self-dev skill manifest default so it no longer activates on every message.

**Requirements:** R1

**Dependencies:** None

**Files:**
- Modify: `skills/bundled/self-dev/skill.toml`

**Approach:**
- Change `always_on = true` to `always_on = false` on line 5
- The skill already has robust keyword triggers that will activate it for implementation-related messages

**Patterns to follow:**
- Other bundled skills that use keyword-only activation (e.g., `build-mika`, `deploy-mika`)

**Test scenarios:**
- Test expectation: none — manifest-only change, no behavioral code modified. Covered by integration tests in Unit 3.

**Verification:**
- `grep always_on skills/bundled/self-dev/skill.toml` shows `always_on = false`
- `cargo build` succeeds (build.rs re-discovers the manifest)

---

- [x] **Unit 2: Add --skill-always-on flag to AskArgs and plumb through dispatch**

**Goal:** Add the CLI flag to `mika ask` and pass it through to the `ask::run()` function.

**Requirements:** R2

**Dependencies:** None (parallel with Unit 1)

**Files:**
- Modify: `crates/mika-cli/src/cli.rs`
- Modify: `crates/mika-cli/src/main.rs`
- Modify: `crates/mika-cli/src/commands/ask.rs`

**Approach:**
- Add `skill_always_on: Vec<String>` field to `AskArgs` with `#[arg(long, num_args = 1)]` for repeatability
- Add `skill_always_on: &[String]` parameter to `ask::run()` signature
- Extract and pass from the `Commands::Ask` match arm in `main.rs`
- In `ask.rs`, after `skill_registry.apply_overrides(&overrides)` and before `Arc::new()`, call the new `apply_transient_always_on()` method (from Unit 3)
- Emit stderr warnings for any unresolved skill names returned by the method

**Patterns to follow:**
- `--model` flag: field in `AskArgs`, extracted in `main.rs` dispatch, applied in `ask.rs` before agent loop
- `--task-id` validation pattern in `ask.rs` for input validation

**Test scenarios:**
- Happy path: `mika ask --skill-always-on self-dev --agent mika-dev "implement X"` parses correctly, skill_always_on contains `["self-dev"]`
- Happy path: Multiple flags `--skill-always-on a --skill-always-on b` produces `["a", "b"]`
- Edge case: No `--skill-always-on` flags produces empty vec (no-op)
- Edge case: Flag used with `--team` — flag is accepted by clap but has no effect (team path doesn't use it)

**Verification:**
- `cargo build -p mika-cli` compiles
- `cargo run --bin mika -- ask --help` shows `--skill-always-on` in the help output

---

- [x] **Unit 3: Add apply_transient_always_on() to SkillRegistry**

**Goal:** Add a method to `SkillRegistry` that applies transient always_on overrides from CLI flags, respecting the precedence rules (cannot resurrect disabled/evicted skills).

**Requirements:** R3, R4, R5

**Dependencies:** None (parallel with Unit 2, integrated when both land)

**Files:**
- Modify: `crates/mika-agent/src/skills/mod.rs`
- Test: `crates/mika-agent/src/skills/mod.rs` (inline `#[cfg(test)] mod tests`)

**Approach:**
- Add `pub fn apply_transient_always_on(&mut self, skill_names: &[String]) -> Vec<String>` to `SkillRegistry`
- For each name in the input, find the matching `SkillEntry` by case-insensitive name comparison
- If found and enabled: set `entry.manifest.skill.always_on = true`, mark `entry.has_override = true`
- If not found (not in `self.skills`): check `self.disabled` — if found there, log a warning about the skill being disabled; either way, add to the unresolved names return vec
- Return the list of names that could not be resolved (for caller to emit warnings)

**Patterns to follow:**
- `apply_overrides()` Phase 1 loop: case-insensitive name matching with `eq_ignore_ascii_case`
- `always_on` mutation pattern: `entry.manifest.skill.always_on = always_on; entry.has_override = true;`

**Test scenarios:**
- Happy path: Applying `["skill-a"]` to a registry where `skill-a` has `always_on=false` sets it to `true`, returns empty unresolved vec
- Happy path: Applying `["skill-a", "skill-b"]` to a registry with both skills sets both to `always_on=true`
- Happy path: Applying to a skill that is already `always_on=true` is idempotent (no error, still returns empty unresolved)
- Edge case: Applying `["nonexistent"]` returns `["nonexistent"]` in unresolved vec
- Edge case: Applying `["SKILL-A"]` matches `skill-a` (case-insensitive)
- Edge case: Applying `["disabled-skill"]` where the skill was evicted returns it in unresolved vec (cannot resurrect)
- Edge case: Empty input `[]` is a no-op, returns empty vec
- Integration: After `apply_transient_always_on(["skill-a"])`, `match_skills()` includes `skill-a` with `MatchReason::AlwaysOn` even when the message has no keywords

**Verification:**
- `cargo test -p mika-agent` passes, including new inline tests
- The method correctly stacks on top of DB overrides (tested by applying DB overrides first, then transient)

---

- [x] **Unit 4: Integration wiring and CLI CLAUDE.md update**

**Goal:** Wire Units 2 and 3 together in `ask.rs` and update CLI documentation.

**Requirements:** R2, R3, R4, R5

**Dependencies:** Unit 2, Unit 3

**Files:**
- Modify: `crates/mika-cli/src/commands/ask.rs`
- Modify: `crates/mika-cli/CLAUDE.md`

**Approach:**
- In `ask.rs`, after `skill_registry.apply_overrides(&overrides)` (line 243) and before `skill_registry.validate_loaded()` (line 245), insert the transient override call:
  - Call `skill_registry.apply_transient_always_on(&skill_always_on_names)` 
  - For each unresolved name, `eprintln!` a warning
- Update the `mika ask` section in `crates/mika-cli/CLAUDE.md` to document the new `--skill-always-on` flag

**Patterns to follow:**
- Warning output pattern: `eprintln!("[mika] ...")` used elsewhere in `ask.rs` (line 252)

**Test scenarios:**
- Integration: `mika ask --skill-always-on self-dev --agent mika-dev "implement issue #123"` activates self-dev even though its manifest says `always_on = false`
- Error path: `mika ask --skill-always-on nonexistent-skill --agent test "hello"` prints a warning to stderr but still runs the agent
- Integration: DB override `always_on=false` + CLI `--skill-always-on` for same skill -> skill is `always_on=true` (CLI wins, applied after DB)

**Verification:**
- `cargo build` succeeds
- `cargo test` passes
- Manual test: `mika ask --skill-always-on self-dev --agent mika-dev "test"` works end-to-end

## System-Wide Impact

- **Interaction graph:** The flag only affects the `mika ask` CLI path. Server message handling, silent mode, team mode, and callback dispatch are unaffected. `match_skills()` and `match_message()` see the mutated `always_on` field transparently.
- **Error propagation:** Unresolved skill names produce stderr warnings, not errors. The agent loop runs regardless.
- **State lifecycle risks:** None — the transient override is applied to a `SkillRegistry` that is immediately wrapped in `Arc` and dropped after the agent loop. No persistence, no side effects.
- **API surface parity:** The HTTP `MessageRequest` does not gain this field. Server-side always_on management continues through DB overrides. This is intentional — the CLI flag serves the headless dispatch use case; the server serves webhook-driven use cases.
- **Unchanged invariants:** `apply_overrides()` behavior is unchanged. `safe_always_on_skills()` and `callback_safe_skills()` see the mutated field, but these are only called in silent mode which `mika ask` does not use.

## Risks & Dependencies

| Risk | Mitigation |
|------|------------|
| Flipping self-dev to `always_on=false` breaks autonomous dev loop | The dev loop dispatches via `mika ask --skill-always-on self-dev`, which explicitly activates it. Existing `skill_overrides` DB rows for mika-dev (if any) are unaffected. |
| Skill name typo silently does nothing | Warning emitted to stderr for each unresolved name. The user sees it immediately. |
| Future confusion about transient vs persistent overrides | Method named `apply_transient_always_on` (not `apply_overrides`) makes the distinction explicit. |

## Sources & References

- Related issue: #670
- `docs/solutions/architecture-patterns/cli-model-override-one-shot.md`
- `docs/solutions/architecture-patterns/cli-flag-subcommand-scoping.md`
- `docs/solutions/architecture-patterns/skill-enabled-state-db-eviction.md`
