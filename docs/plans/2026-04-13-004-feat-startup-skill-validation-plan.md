---
title: "feat: Validate skills at agent startup, not just on explicit validate command"
type: feat
status: active
date: 2026-04-13
---

# feat: Validate skills at agent startup, not just on explicit validate command

## Overview

Wire `validate_skill()` into the agent startup path so that semantic validation errors (name-in-keywords, deprecated `[llm]` section, missing handler scripts, invalid markdown, placeholder mismatches) are surfaced automatically — not only when an operator remembers to run `mika skills validate`. Skills with fatal validation errors that prevent functioning are removed from the registry; others are loaded with warnings.

## Problem Frame

`validate_skill()` catches 12+ semantic issues that `scan_skills_dir()` does not. But it only runs when explicitly invoked via `mika skills validate`. The qa-review `[llm]` orphan incident proved this gap: a skill was functionally invalid for 12 hours with zero system notification. The existing `SkippedSkill` pattern handles structural failures (missing manifest, bad TOML, broken symlinks) but not semantic ones (deprecated sections, missing handlers, placeholder mismatches).

This follows the established "code guards over prompt instructions" principle — telling operators to run a CLI command is fragile enforcement (see `harden-write-skill-variant-no-path-input.md`).

## Requirements Trace

- R1. `validate_skill()` runs on every loaded skill at agent startup (after `apply_overrides()`)
- R2. Validation errors emit `WARN` log entries with structured fields: `skill_name`, `error_message`, `error_kind`
- R3. Decision matrix determines per-error-type action (skip vs load-with-warning)
- R4. TUI startup injects `ChatRole::System` warning for skills with validation errors (max 5 inline)
- R5. `mika ask` surfaces validation warnings on stderr
- R6. Server mode surfaces validation warnings via `tracing::warn`
- R7. New unit tests cover each decision-matrix branch
- R8. Documentation updated in `crates/mika-agent/CLAUDE.md` and `docs/skills.md`

## Scope Boundaries

- No changes to `validate_skill()` diagnostic levels themselves — the existing Ok/Warn/Fail classification stays the same
- No changes to `scan_skills_dir()` — its structural validation is unchanged
- No deduplication of I/O between scan and validate (tracked in existing TODO #634) — this is additive wiring, not a refactor
- No telemetry counters (issue mentions `mika_skill_validation_failed_total` but telemetry is feature-gated and optional; structured `tracing::warn` fields are sufficient for now)

### Deferred to Separate Tasks

- Hot-reload validation: The four hot-reload sites (`chat.rs:214`, `chat.rs:270`, `server/handlers.rs:194`, `server/a2a.rs:120`) rebuild the registry from scratch. Adding validation there follows the same pattern but is a separate concern — hot-reload is already re-running `scan_skills_dir()` + `apply_overrides()`, so the new `validate_loaded()` call will naturally slot in. Defer to a follow-up PR to keep this one focused.
- Deduplicating filesystem I/O between `scan_skills_dir()` and `validate_skill()` (TODO #634)

## Context & Research

### Relevant Code and Patterns

- `crates/mika-agent/src/skills/index.rs` — `validate_skill()` (line 588), `scan_skills_dir()` (line 301), `SkillDiagnostic`, `DiagnosticLevel`, `SkippedSkill`
- `crates/mika-agent/src/skills/mod.rs` — `SkillRegistry` struct (line 18), `from_dir()` (line 25), `apply_overrides()` (line 113), `skipped()` accessor (line 73)
- `crates/mika-cli/src/commands/chat.rs` — Startup skill loading (line 98), `SkippedSkill` TUI warning (line 510-533)
- `crates/mika-cli/src/commands/ask.rs` — `mika ask` startup skill loading (line 225)
- `crates/mika-agent/src/server/mod.rs` — Server startup skill loading (line 337)
- `crates/mika-cli/src/commands/skills.rs` — CLI `validate_skills()` command (line 1221)

### Institutional Learnings

- `always-on-skill-oversized-prompt-loud-failure.md` — "Validate after all mutation phases." Startup validation must run AFTER `apply_overrides()` since DB overrides can flip `always_on` state.
- `harden-write-skill-variant-no-path-input.md` — "Prompt-level enforcement of load-bearing constraints is fragile." Telling users to run a CLI command isn't enforcement.
- `dispatch-readiness-guard-long-running-status-validation.md` — "If the agent ignoring an instruction would cause real harm, enforce it in the harness."
- `is-legacy-format-false-positive-on-valid-skills.md` — The qa-review skill was functionally dead for 12 hours with no user-visible error.
- `custom-skill-silent-loading-failure.md` — Documents the `scan_skills_dir()` silent-skip pattern.
- `validate-agents-teams-commands.md` — Already notes "consider adding automatic validation to startup flows."

## Key Technical Decisions

- **Add `validate_loaded()` method on `SkillRegistry`** rather than calling `validate_skill()` from each startup site: Centralizes the logic in one place. The method calls `validate_skill()` on each `SkillEntry.dir`, applies the decision matrix, removes fatal skills (moving them to `skipped`), and stores diagnostics for TUI/logging consumption. All three startup sites (TUI, ask, server) call this single method after `apply_overrides()`.

- **Store validation diagnostics in a new `validated_warnings` field on `SkillRegistry`**: Distinct from `skipped` (which is for scan-time structural failures). Allows the TUI and `mika ask` to access post-validation warnings without re-running validation. Type: `Vec<SkillValidationWarning>` where each entry is `{ skill_name: String, diagnostics: Vec<SkillDiagnostic> }`.

- **Decision matrix — which Fail diagnostics skip vs warn at startup**: `validate_skill()` emits `Fail` for many conditions, but most fatal ones (missing manifest, bad TOML, legacy format) are already caught by `scan_skills_dir()` and never reach `validate_loaded()`. The Fail diagnostics that can appear on already-loaded skills and their classification:

  | Diagnostic | Skip or Warn | Rationale |
  |-----------|-------------|-----------|
  | Missing handler script | Skip | Skill cannot function without its handler |
  | Handler not executable | Skip | Will fail at runtime |
  | Oversized tools.json | Skip | Tools won't load |
  | Invalid tools.json / cannot read tools.json | Skip | Tools won't load |
  | `skill.toml not found` / `cannot read skill.toml` | Skip | Symlink race — target disappeared between scan and validate |
  | `[llm]` section present | Warn | Runtime ignores it via `#[serde(skip)]` |
  | Name in keywords | Warn | Cosmetic, doesn't affect functionality |
  | Invalid context type | Warn | Context injection fails gracefully |
  | Placeholder mismatch | Warn | Context may not render correctly but skill still loads |
  | Invalid markdown in system_prompt.md | Warn | Prompt still loads, just may have formatting issues |

  Rule of thumb: if the failure means the skill **cannot execute any tool calls** or has no readable manifest, skip it. Otherwise, warn. As a catch-all: if `validate_skill()` returns only Fail diagnostics with zero Ok diagnostics, treat as skip-worthy.

- **TUI warning injection follows existing `SkippedSkill` pattern**: Append a second `ChatRole::System` message after the existing skipped-skills block at `chat.rs:510`. Same max-5-inline, same "run `mika skills validate`" overflow message.

- **`mika ask` surfaces warnings on stderr**: Consistent with existing pending-task notice pattern.

## Open Questions

### Resolved During Planning

- **Should validation run before or after `apply_overrides()`?** After — `apply_overrides()` can flip `always_on` state and modify LLM overrides, which affects validation context (per learning from `always-on-skill-oversized-prompt-loud-failure.md`).
- **Should bundled skills that fail validation be treated differently?** No — bundled skills go through the same code path. A failing bundled skill indicates a template bug (compile-time embedded content), which should be visible, not hidden.
- **How to distinguish "skip" vs "warn" Fail diagnostics?** By diagnostic message pattern matching on the specific check that produced it. A cleaner approach would be adding a `kind` enum to `SkillDiagnostic`, but that's a larger refactor. For now, classify at the `validate_loaded()` call site based on which checks produce actionable failures.

### Deferred to Implementation

- Exact `SkillDiagnostic` message string patterns used for skip-vs-warn classification — implementer should inspect `validate_skill()` output for each check and classify
- Whether `validate_loaded()` should also re-run `warn_missing_llm_api_keys()` or leave that as-is (currently separate)

## High-Level Technical Design

> *This illustrates the intended approach and is directional guidance for review, not implementation specification. The implementing agent should treat it as context, not code to reproduce.*

```
Startup flow (all 3 sites):
  seed_bundled_skills_if_needed()
  SkillRegistry::from_dir(skills_dir)          // scan_skills_dir() — structural
  skill_registry.apply_overrides(&overrides)    // DB overrides
  skill_registry.validate_loaded()              // NEW — semantic validation
  Arc::new(skill_registry)

validate_loaded() internals:
  for each entry in self.skills:
    diags = validate_skill(&entry.dir)
    fail_diags = diags where level == Fail
    warn_diags = diags where level == Warn

    classify each fail_diag:
      if handler-missing or handler-not-executable or tools-json-broken:
        mark skill for removal → SkippedSkill
      else:
        downgrade to warning

    if skill marked for removal:
      move to self.skipped
      log WARN with structured fields
    else if any warnings:
      store in self.validated_warnings
      log WARN with structured fields

TUI (chat.rs): after existing skipped-skills block, check validated_warnings
mika ask (ask.rs): print validated_warnings to stderr
server (mod.rs): tracing::warn already emitted in validate_loaded()
```

## Implementation Units

- [ ] **Unit 1: Add `SkillValidationWarning` type and `validate_loaded()` method**

  **Goal:** Add the core validation-at-startup infrastructure to `SkillRegistry`.

  **Requirements:** R1, R2, R3

  **Dependencies:** None

  **Files:**
  - Modify: `crates/mika-agent/src/skills/mod.rs`
  - Modify: `crates/mika-agent/src/skills/index.rs` (add `SkillValidationWarning` near `SkippedSkill`)
  - Test: `crates/mika-agent/src/skills/mod.rs` (inline `#[cfg(test)] mod tests`)

  **Approach:**
  - Define `SkillValidationWarning { skill_name: String, diagnostics: Vec<SkillDiagnostic> }` in `index.rs` alongside `SkippedSkill`
  - Add `validated_warnings: Vec<SkillValidationWarning>` field to `SkillRegistry`
  - Add `pub fn validated_warnings(&self) -> &[SkillValidationWarning]` accessor
  - Implement `pub fn validate_loaded(&mut self)` on `SkillRegistry` using a **two-phase pattern** (required by borrow checker — cannot mutate `validated_warnings` inside `retain()` closure):
    - **Phase 1 (collect):** Iterate `self.skills` (by index or name), call `validate_skill(&entry.dir)` for each, filter to Warn and Fail diagnostics, classify Fail diagnostics via `is_skip_worthy_failure()`, collect results into local `Vec`s: `to_skip: Vec<(String, String)>` (name, reason) and `to_warn: Vec<SkillValidationWarning>`
    - **Phase 2 (apply):** Call `self.skills.retain(|e| !skip_set.contains(&e.manifest.skill.name))`, then `self.skipped.extend(...)` and `self.validated_warnings = to_warn`
    - Emit `tracing::warn!` for each skill with issues, with structured fields `skill = %name`, `error_kind = "..."`, `message = "..."`
  - The skip-vs-warn classification should use a helper function `is_skip_worthy_failure(diag: &SkillDiagnostic) -> bool` that matches on known message prefixes from `validate_skill()`. Skip-worthy prefixes: "tool '" (handler not found/not executable), "invalid tools.json", "cannot read tools.json", "tools.json exceeds", "skill.toml not found", "cannot read skill.toml". Catch-all: if all diagnostics are Fail with zero Ok entries, treat as skip-worthy (handles symlink races where target disappeared between scan and validate). The function should include a comment listing the `validate_skill()` source lines it depends on, so future edits to diagnostic messages trigger classifier updates

  **Patterns to follow:**
  - `apply_overrides()` post-override removal pattern (line 156-195 in `mod.rs`) — same `retain()` + move-to-skipped approach
  - `warn_missing_llm_api_keys()` structured logging pattern (line 568 in `index.rs`)

  **Test scenarios:**
  - Happy path: skill with no validation issues → not in `validated_warnings`, not in `skipped`
  - Happy path: skill with only `DiagnosticLevel::Ok` entries → no warnings emitted
  - Edge case: skill with `[llm]` section (Fail) → loaded with warning, not skipped
  - Edge case: skill with name-in-keywords (Fail) → loaded with warning, not skipped
  - Error path: skill with missing handler script (Fail) → removed from skills, added to skipped
  - Error path: skill with non-executable handler (Fail) → removed from skills, added to skipped
  - Error path: skill with invalid tools.json (Fail) → removed from skills, added to skipped
  - Edge case: skill with both skip-worthy Fail AND warn-only diagnostics → skipped (skip takes precedence)
  - Edge case: skill with only Warn diagnostics (e.g., markdown issues) → loaded with warning
  - Edge case: empty registry → `validate_loaded()` is a no-op, no panics
  - Integration: `validate_loaded()` after `apply_overrides()` that flipped `always_on` → validation runs against post-override state

  **Verification:**
  - `cargo test -p mika-agent` passes with new tests covering each decision-matrix branch
  - `validate_loaded()` correctly partitions skills into loaded-with-warnings vs skipped

- [ ] **Unit 2: Wire `validate_loaded()` into all three startup sites**

  **Goal:** Call `validate_loaded()` after `apply_overrides()` in TUI chat, `mika ask`, and server startup.

  **Requirements:** R1, R6

  **Dependencies:** Unit 1

  **Files:**
  - Modify: `crates/mika-cli/src/commands/chat.rs`
  - Modify: `crates/mika-cli/src/commands/ask.rs`
  - Modify: `crates/mika-agent/src/server/mod.rs`

  **Approach:**
  - In each file, after the `skill_registry.apply_overrides(&overrides)` call and before `Arc::new(skill_registry)`, add `skill_registry.validate_loaded()`
  - This is a one-line addition at each site
  - Server mode already gets `tracing::warn` from `validate_loaded()` — no additional code needed there

  **Patterns to follow:**
  - Existing `apply_overrides()` call pattern at each site

  **Test scenarios:**
  - Happy path: startup with all-valid skills → no warnings emitted, no skills removed
  - Integration: startup with a skill that has a missing handler → skill removed, `tracing::warn` emitted (verify via test log capture)

  **Verification:**
  - `cargo build` succeeds
  - `cargo test` passes
  - Manual smoke test: agent starts, runs `validate_loaded()` (visible in debug logs)

- [ ] **Unit 3: TUI startup warning for validation errors**

  **Goal:** Inject a `ChatRole::System` warning message in the TUI when loaded skills have validation errors.

  **Requirements:** R4

  **Dependencies:** Unit 1, Unit 2

  **Files:**
  - Modify: `crates/mika-cli/src/commands/chat.rs`

  **Approach:**
  - After the existing skipped-skills warning block (line 510-533), add a parallel block that checks `app.skills.validated_warnings()`
  - Same display pattern: max 5 inline entries, overflow message pointing to `mika skills validate`
  - Each entry shows skill name + first diagnostic message (keep it concise)
  - Use a distinct warning prefix to differentiate from skipped skills, e.g., `⚠ N skill(s) loaded with validation warnings:`

  **Patterns to follow:**
  - Existing `SkippedSkill` warning injection at `chat.rs:510-533` — exact same `ChatMessage { role: ChatRole::System, ... }` pattern

  **Test scenarios:**
  - Happy path: no validation warnings → no system message injected
  - Happy path: 2 skills with warnings → system message lists both
  - Edge case: 7 skills with warnings → first 5 shown, overflow message for remaining 2
  - Edge case: both skipped skills AND validation warnings → two separate system messages displayed

  **Verification:**
  - TUI shows validation warnings at startup when skills have semantic issues
  - Warning message is visually distinct from skipped-skills message

- [ ] **Unit 4: `mika ask` stderr warning for validation errors**

  **Goal:** Surface validation warnings on stderr for non-interactive `mika ask` mode.

  **Requirements:** R5

  **Dependencies:** Unit 1, Unit 2

  **Files:**
  - Modify: `crates/mika-cli/src/commands/ask.rs`

  **Approach:**
  - After `validate_loaded()` call, check `skill_registry.validated_warnings()`
  - Print a summary to stderr using `eprintln!`, similar to the pending-task notice pattern
  - Format: `[mika] N skill(s) loaded with validation warnings. Run 'mika skills validate' for details.`

  **Patterns to follow:**
  - Existing pending-task stderr notice in `ask.rs`

  **Test scenarios:**
  - Happy path: no validation warnings → nothing printed to stderr
  - Happy path: skills with warnings → stderr message printed with count

  **Verification:**
  - `mika ask "test"` with an invalid skill prints validation notice to stderr

- [ ] **Unit 5: Unit tests for decision matrix branches**

  **Goal:** Comprehensive test coverage for the skip-vs-warn classification logic.

  **Requirements:** R7

  **Dependencies:** Unit 1

  **Files:**
  - Modify: `crates/mika-agent/src/skills/mod.rs` (inline tests) or create test fixtures
  - Create: test skill directories under a temp dir with specific validation failures

  **Approach:**
  - Use `tempdir` to create skill directories with intentionally invalid configurations
  - Test each decision-matrix branch:
    - `[llm]` section in TOML → loaded, warning
    - Name appears in keywords → loaded, warning
    - Missing handler script in tools.json → skipped
    - Handler script not executable → skipped
    - Invalid tools.json → skipped
    - Invalid markdown in system_prompt.md → loaded, warning
    - Placeholder mismatch → loaded, warning
    - Invalid context type → loaded, warning (downgraded from Fail)
  - Verify both the `skipped` and `validated_warnings` outputs

  **Patterns to follow:**
  - Existing tests in `crates/mika-agent/src/skills/` that use temp directories
  - `validate_skill()` test patterns in `skills.rs` CLI command

  **Test scenarios:**
  - Each row of the decision matrix as an individual test case
  - Combination: skill with both skip-worthy and warn-only failures → correctly classified as skip
  - Combination: multiple skills, some valid, some warn-only, some skip-worthy → correct partitioning

  **Verification:**
  - `cargo test -p mika-agent` passes with all new tests green
  - Each decision-matrix branch has explicit coverage

- [ ] **Unit 6: Documentation update**

  **Goal:** Document the new startup validation behavior.

  **Requirements:** R8

  **Dependencies:** Units 1-5

  **Files:**
  - Modify: `crates/mika-agent/CLAUDE.md`
  - Modify: `docs/skills.md`

  **Approach:**
  - In `crates/mika-agent/CLAUDE.md` Skills System section: add a note about startup validation — `validate_loaded()` runs after `apply_overrides()`, decision matrix reference, link to issue #530
  - In `docs/skills.md`: add a "Startup Validation" section explaining the behavior, the decision matrix table, and how warnings surface in each mode (TUI, ask, server)

  **Test expectation: none — documentation-only change**

  **Verification:**
  - Documentation accurately describes the implemented behavior
  - Decision matrix table matches the code

## System-Wide Impact

- **Interaction graph:** `validate_loaded()` sits between `apply_overrides()` and `Arc::new(skill_registry)` in the startup sequence. It modifies the registry in place (removing skills, adding warnings). Downstream consumers (`TaskDispatcher`, `agent_loop`, matcher) see the post-validation registry — they do not need changes.
- **Error propagation:** Validation errors are captured and stored, never propagated as panics. Fatal validation → `SkippedSkill` (same as scan-time failures). Non-fatal → `SkillValidationWarning` (new, advisory only).
- **State lifecycle risks:** Skills removed by validation will no longer be in the registry for the agent session. If a skill is incorrectly classified as skip-worthy, the operator sees the warning and can fix the underlying issue. No data loss — skills remain on disk.
- **API surface parity:** Server mode gets `tracing::warn` (same as existing skipped-skill logging). No new HTTP endpoints needed.
- **Integration coverage:** The key cross-layer scenario is: bundled skill seeded → scan passes → validation catches semantic issue → skill correctly classified (skip vs warn) → TUI/ask/server surfaces the diagnostic. This end-to-end path should be verified manually.
- **Unchanged invariants:** `scan_skills_dir()` behavior is completely unchanged. `validate_skill()` diagnostic output is unchanged. `apply_overrides()` behavior is unchanged. The change is purely additive — a new step inserted between existing steps.

## Risks & Dependencies

| Risk | Mitigation |
|------|------------|
| `validate_skill()` adds startup latency (filesystem I/O per skill) | Acceptable for typical 10-20 skills. Dedup with scan is tracked in TODO #634 for future optimization. |
| Message-prefix matching for skip classification is brittle | Use `is_skip_worthy_failure()` helper with clear comments. A `DiagnosticKind` enum is the clean solution but deferred to avoid scope creep. |
| Removing a loaded skill post-override could break dependency warnings | `apply_overrides()` dependency check runs before `validate_loaded()`, so warnings may reference soon-to-be-removed skills. Acceptable — the removed skill's own warning is more actionable. |

## Documentation / Operational Notes

- Operators will see new WARN log lines at startup for skills with validation issues — this is intentional and expected
- The existing recommendation "run `mika skills validate`" remains valid for full diagnostic output; startup validation surfaces only the most critical findings

## Sources & References

- Related issue: #530
- Related PRs: #522 (added validator checks that exposed the gap)
- Related issues: mika-skills#127 (the orphan), mika-skills#128 (the fix)
- Existing pattern: `SkippedSkill` warning at TUI startup (`chat.rs:510-533`)
- Learnings: `custom-skill-silent-loading-failure.md`, `harden-write-skill-variant-no-path-input.md`, `dispatch-readiness-guard-long-running-status-validation.md`
