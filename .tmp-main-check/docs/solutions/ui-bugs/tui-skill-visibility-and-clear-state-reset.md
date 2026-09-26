---
title: "TUI skill visibility at startup and /skills, /clear state reset cleanup"
category: ui-bugs
date: 2026-04-03
tags: [tui, skills, slash-commands, clear, state-reset, startup-warning]
issues: ["#409", "#391", "#390", "#347"]
---

# TUI Skill Visibility and /clear State Reset

## Problem

Three related TUI gaps:

1. **Silent skill failures (#391, #390):** `scan_skills_dir()` logs skipped skills via `tracing::warn!` but the TUI user sees nothing. `ScanResult` only tracked `skipped_count: usize` — no names, no reasons. Users had to read log files or run `mika skills validate` to discover broken skills.

2. **Incomplete `/clear` (#347):** `/clear` reset session/messages/scroll but missed `pending_response`, `reveal_index`, `status`, `pending_images`, `pending_command`, `has_new_message`, `selection_state`, `pending_task_count`. If `/clear` runs while the agent is thinking, the in-flight response could create a ghost message in the new session.

## Root Cause

- `ScanResult` had no data structure to capture skip details — only an integer count.
- `/clear` was implemented incrementally and never got a comprehensive state audit.
- The agent response channel (`agent_rx`) was not drained on `/clear`, allowing stale responses through.

## Solution

### Part 1: `SkippedSkill` struct

Added `SkippedSkill { name: String, reason: String }` to `skills/index.rs`. Extended `ScanResult` with `skipped: Vec<SkippedSkill>`. Removed the redundant `skipped_count` field (derived from `skipped.len()` instead).

All 7 skip paths in `scan_skills_dir()` now push to the vector: broken symlink, oversized toml, unreadable manifest, legacy format, invalid TOML, oversized always_on prompt, unreadable always_on prompt. `apply_overrides()` also appends when removing post-override skills.

Key design decisions:
- `String` for reason (not enum) — display-only, 7+ variants with embedded data would add complexity without enabling programmatic dispatch.
- `dir_name` extracted once per loop iteration (not duplicated in each skip branch).
- `with_skipped()` constructor on `SkillRegistry` for cross-crate test construction (fields are private).

### Part 2: Startup warning

In `chat.rs`, after `App::new()` and `load_thinking_level()`, inject a `ChatRole::System` message when `skill_registry.skipped()` is non-empty. Shows up to 5 skipped skills inline, then "... and N more. Run `mika skills validate` for details." Only at startup — hot-reload changes are visible via `/skills`.

### Part 3: `/skills` SKIPPED section

Updated `handle_skills()` to show a "SKIPPED" section after ALWAYS ON and ON DEMAND, with `✗` badge and reason. Header shows `"Loaded skills (N, M skipped):"` when skipped > 0. Column alignment uses `max_name_width` across all sections (including skipped names).

### Part 4: `/clear` state reset

Added resets for: `pending_response`, `reveal_index`, `status` (→ Idle), `pending_images`, `pending_command`, `has_new_message`, `selection_state`, `pending_task_count`. Added `while app.agent_rx.try_recv().is_ok() {}` to drain stale responses.

Intentionally preserved: `thinking_level`, `model`, `provider`, `skills` (user preferences, not session state).

## Prevention

- **State audit pattern:** When modifying `/clear` or adding new `App` fields, consider whether the field is session-scoped (reset on `/clear`) or preference-scoped (preserved). The test `test_clear_resets_all_state_fields` and `test_clear_preserves_preferences` serve as a regression gate.
- **Data structure completeness:** When tracking counts of things, prefer `Vec<Detail>` over `usize` — the count is always derivable from `.len()`, and the details are useful for display.
- **Cross-crate test constructors:** When struct fields are private, provide test constructors (like `with_skipped()`) rather than making fields public.

## Related

- `docs/solutions/ui-bugs/tui-slash-command-reliability-clear-provider-model.md` — Prior `/clear` fix establishing the session reset pattern
- `docs/solutions/integration-issues/always-on-skill-oversized-prompt-loud-failure.md` — `SnippetLoadResult` and always_on skill loading invariants
- `docs/solutions/integration-issues/custom-skill-silent-loading-failure.md` — The "silent skip" anti-pattern in skill loading
