---
title: "feat(skills): consolidate run_gh skill with full label support + seed label taxonomy in memory"
type: feat
status: completed
date: 2026-03-13
---

# feat(skills): consolidate run_gh skill with full label support

## Overview

The `run_gh` builtin handler already allows the `label` subcommand (it's in `GH_ALLOWED_SUBCOMMANDS`), but the github skill's system prompt has zero documentation for label operations. This causes the agent to either guess syntax, fail, or fall back to `run_shell` for label management. Additionally, `skill.toml` keywords don't include "label" so label-only messages may not trigger the skill at all.

## Problem Statement

1. **No prompt guidance for labels** — The system prompt documents PR, issue, CI/CD, and repo operations but omits labels entirely. The agent doesn't know the `gh label` CLI syntax.
2. **Missing skill trigger keywords** — "label", "labels" etc. are not in `skill.toml` keywords, so messages about labels may not activate the github skill.
3. **Label taxonomy not in structured memory** — The project's canonical labels are prose notes in core memory, not searchable structured facts.

## Proposed Solution

Three file changes, zero Rust code changes:

### 1. Update `skill.toml` keywords

**File:** `crates/mika-agent/templates/skills/github/skill.toml`

Add label-related trigger keywords to the keyword list:
- `"label"`, `"labels"`, `"add label"`, `"create label"`, `"remove label"`

### 2. Add Labels section to `system_prompt.md`

**File:** `crates/mika-agent/templates/skills/github/system_prompt.md`

Add a new `### Labels` section under Common Operations with:
- `label list` — list all labels (plain and `--json` variants)
- `label create` — create a label (with explicit "6-char hex, no # prefix" guidance)
- `label edit` — rename or change color/description
- `label delete` — delete a label (destructive — confirm with user)
- Pre-apply check guidance: "Before applying a label with `issue edit --add-label`, verify the label exists by running `label list` first (once per conversation is sufficient). If it doesn't exist, create it first."
- Note that `--add-label` accepts comma-separated values for batch application
- Note that label names are case-insensitive on GitHub
- Note that `label create` on an existing label returns an error (non-fatal, skip and continue)

Add `label delete` to the destructive operations confirmation list in Guidelines.

### 3. Seed label taxonomy in agent memory (documentation approach)

**NOT hard-coded in the skill template** — the label taxonomy is project-specific data. Instead:

- Add a "Label Discovery" guidance note in the system prompt: "When working with a repository's labels for the first time, run `label list` to discover the available labels. Store important label taxonomies as preferences using `store_fact` so you can reference them in future sessions."
- For the Mika project specifically, the user (or a setup step) will tell the agent to memorize the current taxonomy. This is a one-time runtime action, not a code change.

The current Mika repo labels (for reference during seeding):

**Priority labels:**
| Name | Color | Description |
|------|-------|-------------|
| p0-critical | #b60205 | Blocks usage or causes data loss |
| p1-important | #d93f0b | Significant impact, needs prompt fix |
| p2-normal | #fbca04 | Standard priority |
| p3-nice-to-have | #0e8a16 | Low priority improvement |
| deferred | #CCCCCC | Parked — not actively being worked on |

**Component labels:**
| Name | Color | Description |
|------|-------|-------------|
| agent-core | #5319e7 | Agent loop, tools, memory |
| tui | #1d76db | CLI/TUI interface |
| gateway | #f9d0c4 | Telegram gateway |
| skill | #bfd4f2 | Skills system and marketplace |
| team-engine | #c5def5 | Multi-agent team orchestration |
| infrastructure | #d4c5f9 | Docker, CI/CD, deployment |

**Type labels:** bug, enhancement, documentation, release

**Note:** There is a duplicate `p2` label (color #0075CA, "Priority 2 — normal priority") that overlaps with `p2-normal`. This should be cleaned up.

## Technical Considerations

- **No Rust handler changes** — The handler is intentionally subcommand-agnostic. Pre-flight checks belong in the prompt, not the handler.
- **No tools.json changes** — The `run_gh` tool definition is generic and doesn't need label-specific schema.
- **store_fact category** — Label taxonomy stored as `preference` facts (e.g., `key="github_labels:senara-solutions/mika"`, `value="<structured list>"`). The `preference` category is the closest fit.

## Acceptance Criteria

- [x] `skill.toml` keywords include label-related terms so the skill activates on label messages
- [x] `system_prompt.md` has a Labels section with `label list/create/edit/delete` examples
- [x] System prompt includes "check before apply" guidance
- [x] System prompt includes color format guidance (6-char hex, no `#` prefix)
- [x] `label delete` is called out as destructive (requires user confirmation)
- [x] No Rust code changes (handler already supports labels)
- [x] All existing tests pass (`cargo test`)

## Files to Modify

1. `crates/mika-agent/templates/skills/github/skill.toml` — Add keywords
2. `crates/mika-agent/templates/skills/github/system_prompt.md` — Add Labels section + label discovery guidance

## Sources

- GitHub Issue: #131
- Handler implementation: `crates/mika-agent/src/skills/builtin_handlers.rs:224-399`
- Allowlist already includes `"label"`: `builtin_handlers.rs:233`
