---
title: "GitHub skill missing label operation documentation and keyword triggers"
date: 2026-03-13
module: "crates/mika-agent/skills/github"
severity: minor
tags:
  - skills
  - github
  - system-prompt
  - skill-discovery
  - labels
symptoms:
  - "Agent guesses gh label syntax instead of using correct flags"
  - "Agent falls back to run_shell for label operations instead of using run_gh"
  - "Label-related user messages do not trigger the github skill"
root_cause: >
  The run_gh builtin handler already had label in its subcommand allowlist,
  but skill.toml keywords lacked label-related terms (preventing skill activation)
  and system_prompt.md had no label operation documentation (preventing correct usage).
  Pure documentation/config gap — zero Rust code changes needed.
---

# GitHub Skill Missing Label Operation Documentation

## Problem

The `run_gh` builtin handler's subcommand allowlist included `label`, meaning `gh label list/create/edit/delete` would execute without error. However, two gaps prevented the agent from using label operations:

1. **Skill activation gap**: `skill.toml` keywords didn't include "label" or related terms. Label-only messages wouldn't trigger the github skill, so the agent had no access to `run_gh`.

2. **Prompt guidance gap**: `system_prompt.md` documented PR, issue, CI/CD, and repo operations but had no Labels section. Without examples, the agent guessed syntax incorrectly or fell back to `run_shell`.

**Observed behavior** (session `0a872dd9`): Agent tried `issue edit --add-label p2` on 4 issues, all failed because the label didn't exist. Created it via `run_shell` + raw `gh`, then retried — a blind retry loop mixing two tools.

## Investigation

1. Checked `builtin_handlers.rs:225-236` — confirmed `label` was in `GH_ALLOWED_SUBCOMMANDS`
2. Checked `system_prompt.md` — no Labels section, no label examples
3. Checked `skill.toml` keywords — no label-related terms
4. Confirmed fix was purely prompt/config level

## Solution

**3 files changed, 0 Rust changes.**

### 1. Added label keywords to `skill.toml`

```toml
keywords = [
  # ... existing keywords ...
  "label", "labels", "add label", "create label",
  "edit label", "delete label", "remove label"
]
```

### 2. Added Labels section to `system_prompt.md`

```markdown
### Labels
- List all labels: `["label", "list"]`
- List labels (structured): `["label", "list", "--json", "name,color,description"]`
- Create a label: `["label", "create", "bug-triage", "--color", "d73a4a", "--description", "Needs triage"]`
- Edit a label: `["label", "edit", "old-name", "--name", "new-name", "--color", "0075ca"]`
- Delete a label: `["label", "delete", "label-name", "--yes"]`
```

Key guidance added:
- **Check before apply**: Verify label exists via `label list` before using `issue edit --add-label`
- **Color format**: 6-char hex WITHOUT `#` prefix (`d73a4a` not `#d73a4a`)
- **Batch labels**: Comma-separated `--add-label "bug,p1-important"`
- **Idempotency**: `label create` on existing label returns error — skip and continue
- **Destructive ops**: `label delete` and `label edit (rename)` added to confirmation list

### 3. Updated `docs/skills.md`

Added label keywords to the github skill row in the bundled skills table.

## Key Decisions

- **No Rust handler changes** — Handler is intentionally subcommand-agnostic. Intelligence belongs in the prompt.
- **No tools.json changes** — The `run_gh` tool definition is generic and doesn't need label-specific schema.
- **Label taxonomy not hard-coded** — Project-specific label sets belong in agent memory (via `store_fact`), not in the skill template.

## Prevention

This is a **documentation drift** pattern: backend capability exists but LLM-facing guidance is missing.

1. **Allowlist-to-prompt cross-check**: When adding subcommands to the handler allowlist, always update the system prompt and keywords in the same commit.
2. **Three-way consistency review**: For any skill change, check handler allowlist, system prompt docs, and skill.toml keywords together.
3. **Potential CI test**: Parse `GH_ALLOWED_SUBCOMMANDS` and assert each entry appears in `system_prompt.md`. Static analysis, no runtime deps.

## Related

- GitHub Issue: [#131](https://github.com/senara-solutions/mika/issues/131)
- [Adding a prompt-only bundled skill](../integration-issues/adding-prompt-only-bundled-skill.md)
- [Skills doc-code drift and validation](../integration-issues/skills-doc-code-drift-and-validation-infrastructure.md)
- [Skill dependency resolution](../architecture-patterns/skill-dependency-resolution-and-action-guard.md)
- Handler implementation: `crates/mika-agent/src/skills/builtin_handlers.rs:224-399`
