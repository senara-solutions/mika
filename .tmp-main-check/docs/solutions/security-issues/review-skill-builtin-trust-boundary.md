---
title: "review_skill: block reviewing built-in/Claude-authored skills"
category: security-issues
date: 2026-04-07
tags: [skill-review, built-in, trust-boundary, is_bundled_skill, variant-generation]
issue: 480
---

# review_skill: block reviewing built-in/Claude-authored skills

## Problem

The `review_skill` tool handler accepted built-in skill names and would inspect or persist generated prompt variants for them. This is a trust-boundary violation: built-in skills are platform-managed and define the agent's operating contract. An agent running on a weaker model could rewrite its own `skill-review` prompt (meta self-modification).

**Observed:** mika-qa (gemini-2.5-flash) reviewed `skill-review` itself, producing a real variant at `~/.mika/agents/mika-qa/skills/skill-review/generated/google/gemini-2.5-flash/system_prompt.md`.

## Root Cause

Other tool handlers (`delete_skill`, `update_skill`, skill installation) already guarded against built-in modifications using `is_bundled_skill()` from `bundled_skills.rs`. The `review_skill` tool was the only handler missing this guard, likely because it was added later (PR #475 unblocked linked skills for review, which may have inadvertently opened the door for built-ins too).

## Solution

Added `is_bundled_skill(skill_name)` guards to both code paths in `crates/mika-agent/src/skills/builtin_handlers.rs`:

1. **Single-skill path** (line ~993): Early return with `ToolOutput::success("Cannot review built-in skill '{}'. Built-in skills are platform-managed and updated automatically.")` before the filesystem existence check. This ensures case-mismatched names (e.g., `SKILL-REVIEW`) get the "built-in" error via `eq_ignore_ascii_case` rather than "not found" on case-sensitive filesystems.

2. **Batch path** (`review_skill_batch` loop): Skip with `"reason": "built-in"` in the skipped skills list, placed before the linked-skill check (a built-in that happens to be a symlink should be skipped for being built-in, not linked).

3. **System prompt**: Added a "Restrictions" section to `templates/skills/skill-review/system_prompt.md` listing all 12 built-in skills so agents avoid wasting tool calls.

### Key design decisions

- **`ToolOutput::success` not `ToolOutput::error`**: The guard is a policy rejection, not a tool malfunction. Within `review_skill`, `error` is reserved for malformed inputs (wrong parameter type, unsupported combination). This follows the exec-handler pattern documented in `docs/solutions/logic-errors/exec-handler-stdout-discarded-on-nonzero-exit.md`.

- **No manifest `builtin` field**: The issue suggested adding `builtin = true` to `skill.toml`, but `is_bundled_skill()` is already data-driven via the `BUNDLED_SKILLS` array and used by all other guard paths. A manifest field would be less secure (a malicious skill could set `builtin = false`).

- **Block both inspect and persist**: The tool's purpose is variant generation, which is meaningless for built-in skills that are overwritten by `seed_bundled_skills()` on every restart. Agents can still read built-in prompts via `read_agent_file` or `get_documentation`.

## Prevention

When adding new tool handlers that operate on skills by name, always check `is_bundled_skill()` early in the handler. The function is `pub` in `crate::bundled_skills` and handles case-insensitive matching internally.

**Pattern to follow:**
```rust
use crate::bundled_skills::is_bundled_skill;

// Early in the handler, before filesystem operations:
if is_bundled_skill(skill_name) {
    return ToolOutput::success(format!(
        "Cannot {verb} built-in skill '{skill_name}'. \
         Built-in skills are platform-managed and updated automatically.",
    ));
}
```

### Known residual risk

`write_agent_file` can write directly into `skills/<built-in>/generated/` since it only validates path containment within the agent home directory, not built-in skill status. This is a pre-existing concern documented in the plan's "Out of Scope" section.

## Related

- Issue: #480
- Guard pattern: `is_bundled_skill()` in `bundled_skills.rs:150`
- Prior art: `docs/solutions/architecture-patterns/adding-skill-review-builtin-handler.md`
- Linked skill unblock: `docs/solutions/architecture-patterns/skill-llm-override-db-layer-and-linked-unblock.md` (PR #475)
- Hardened variant writes: `docs/solutions/architecture-patterns/harden-write-skill-variant-no-path-input.md`
