---
title: Fix skill dependency resolution and unsolicited action guard
type: fix
status: active
date: 2026-03-13
---

# Fix skill dependency resolution and unsolicited action guard

## Overview

Two bugs: (1) always_on skills that reference tools from other skills fail when the user message doesn't keyword-match the dependency skill, (2) agent takes unsolicited multi-step actions when the user asks an informational question.

## Problem Statement

**Bug 1:** The self-dev skill (always_on) instructs the agent to use tmux tools, but when the user says "yes please" (no tmux keywords), `match_skills()` doesn't load the tmux skill. The dependency resolution infrastructure already exists (`dependencies: Vec<String>` on `SkillInfo`, two-pass matcher), but the self-dev skill doesn't declare `dependencies = ["tmux"]` in its `skill.toml`.

**Bug 2:** When asked "can you list my tmux sessions?", the agent lists sessions then immediately sends a `/mika` command to start a pipeline — never asking for confirmation. No system prompt guardrail exists for this.

## Proposed Solution

### Bug 1 — Declare missing skill dependencies

- [x] ~~Add `dependencies = ["tmux"]` to the self-dev skill's `skill.toml`~~ N/A — self-dev is a user custom skill, not bundled. Infrastructure supports any skill declaring dependencies.
- [x] Add `validate_dependencies()` method to `SkillRegistry` that logs warnings at startup for any declared dependency that doesn't match an installed skill name
- [x] Call `validate_dependencies()` after `apply_overrides()` in the registry initialization path
- [x] Add test for `validate_dependencies()` — warns on missing, silent on valid

### Bug 2 — Add unsolicited action guardrail to system prompt

- [x] Add a "Confirmation before action" rule to the instructions section in `prompt.rs`:
  - When the user asks an informational question (e.g., "can you list...", "what are...", "show me..."), answer the question directly and stop
  - Do not interpret questions as implicit requests to start multi-step workflows
  - If follow-up action may be useful, suggest it and wait for confirmation
- [x] Add test verifying the guardrail text appears in the assembled system prompt

## Key Files

- `crates/mika-agent/src/skills/manifest.rs` — `SkillInfo` (dependencies field already exists)
- `crates/mika-agent/src/skills/matcher.rs` — `match_skills()` (two-pass resolution already exists)
- `crates/mika-agent/src/skills/mod.rs` — `SkillRegistry` (add `validate_dependencies()`)
- `crates/mika-agent/src/prompt.rs` — system prompt (add guardrail)

## Acceptance Criteria

- [x] ~~self-dev skill's `skill.toml` declares `dependencies = ["tmux"]`~~ N/A — user custom skill
- [x] `validate_dependencies()` logs warning for nonexistent dependency targets
- [x] System prompt contains unsolicited action guardrail
- [ ] All existing tests pass
- [ ] New tests for `validate_dependencies()`

## Context

- Related issue: #134
- The dependency field and two-pass matcher already exist — this was discovered during research
- The learnings doc `callback-task-loop-prevention.md` confirms silent/callback agents use `safe_always_on_skills()` which is a separate safety layer
- The learnings doc `adding-prompt-only-bundled-skill.md` notes that always_on skills need specific keywords to prevent false positives
