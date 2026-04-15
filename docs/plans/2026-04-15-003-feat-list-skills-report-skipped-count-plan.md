---
title: "feat: Report skipped skill count in list_skills tool output"
type: feat
status: active
date: 2026-04-15
---

# feat: Report skipped skill count in list_skills tool output

## Overview

Add a warning footer to the `list_skills` agent tool output when skills were skipped during registry loading. This enables agents to self-diagnose a degraded skill registry without changing output for the normal (zero-skipped) case.

## Problem Frame

The `list_skills` tool calls `SkillRegistry::from_dir()` which tracks skipped skills (via `skipped_count()` / `skipped()`), but the tool only reports loaded entries. An agent cannot detect that skills were silently skipped due to errors (oversized prompts, missing handlers, broken manifests). The TUI `/skills` command already surfaces skipped skills — the agent tool should too.

Related: #334, #331, #333.

## Requirements Trace

- R1. When `skipped_count > 0`, append a warning footer to `list_skills` output
- R2. No output change when `skipped_count == 0`
- R3. Test coverage for both cases

## Scope Boundaries

- Only the `list_skills` agent tool output is changed
- The TUI `/skills` command already handles this — no changes needed there
- No new tools, no schema changes, no new dependencies

## Context & Research

### Relevant Code and Patterns

- `crates/mika-agent/src/tools/list_skills.rs` — tool implementation, line 92 is the return point
- `crates/mika-agent/src/skills/mod.rs` — `SkillRegistry::skipped_count()` (line 75), `with_skipped()` (line 66)
- `crates/mika-agent/src/skills/index.rs` — `SkippedSkill { name, reason }` struct (line 281)
- `crates/mika-cli/src/tui/commands/handlers.rs` — TUI precedent: shows `"Loaded skills ({}, {} skipped):\n"` header + `SKIPPED` section

### Institutional Learnings

- `docs/solutions/integration-issues/custom-skill-silent-loading-failure.md` — documents the silent-skip problem; `scan_skills_dir()` emits `warn!` but users/agents never see it
- `docs/solutions/integration-issues/always-on-skill-oversized-prompt-loud-failure.md` — introduced `skipped_count` tracking; data already exists, just not exposed via tool
- `docs/solutions/architecture-patterns/startup-skill-validation-structural-enforcement.md` — `validate_loaded()` pattern classifies failures; the tool is a natural channel to surface skipped skills

## Key Technical Decisions

- **Footer, not header:** Append a warning footer after the skills list rather than modifying the header. This preserves backward compatibility for agents that parse the header line, and matches the issue's proposed solution.
- **Use `skipped_count` only, not individual reasons:** The footer directs to `mika skills validate` for details, keeping tool output concise. Individual `SkippedSkill` reasons are available but would clutter the output.
- **Test via invalid skill directory:** Create a skill directory with a malformed `skill.toml` to trigger a skip, rather than refactoring to inject a registry. This keeps the test realistic and matches existing test patterns.

## Implementation Units

- [x] **Unit 1: Add skipped count warning footer to list_skills output**

**Goal:** When `registry.skipped_count() > 0`, append a warning line after the skills listing.

**Requirements:** R1, R2

**Dependencies:** None

**Files:**
- Modify: `crates/mika-agent/src/tools/list_skills.rs`
- Test: `crates/mika-agent/src/tools/list_skills.rs` (inline `#[cfg(test)]` module)

**Approach:**
- After line 90 (end of the entries loop), before line 92 (return), check `registry.skipped_count()`
- If > 0, push `"\nWarning: N skill(s) skipped due to errors. Run 'mika skills validate' for details.\n"` to `output`
- The `registry` variable is already in scope (line 34)

**Patterns to follow:**
- TUI handler at `crates/mika-cli/src/tui/commands/handlers.rs` — already surfaces skipped count
- Existing test pattern: `setup()` creates `TempDir` + `TestHarness`, writes `skill.toml` files

**Test scenarios:**
- Happy path: list skills with no skipped entries — output does NOT contain "Warning" or "skipped"
- Happy path: list skills with skipped entries — output contains `"Warning: 1 skill(s) skipped due to errors"` and `"mika skills validate"`
- Edge case: all skills skipped (only invalid skill dirs) — output is "No skills installed." with the warning appended
- Edge case: mix of valid and invalid skills — output shows valid skills AND the warning footer

**Verification:**
- `cargo test -p mika-agent -- list_skills` passes with both existing and new tests
- `cargo clippy -p mika-agent` clean

## System-Wide Impact

- **API surface parity:** The TUI `/skills` command already reports skipped count — this brings the agent tool to parity
- **Unchanged invariants:** `list_skills` JSON schema, tool name, and input format are unchanged. Output is text, not structured — the footer is additive

## Risks & Dependencies

| Risk | Mitigation |
|------|------------|
| Agents parsing the output string may break | Footer is appended after existing content; header format unchanged. Low risk — output is free-text, not structured |

## Sources & References

- Related issue: #334
- Related PRs: #333, #331
- Code: `crates/mika-agent/src/tools/list_skills.rs`, `crates/mika-agent/src/skills/mod.rs`
