---
title: "fix: Block review_skill from reviewing built-in skills"
type: fix
status: completed
date: 2026-04-07
issue: 480
---

# fix: Block review_skill from reviewing built-in skills

## Overview

The `review_skill` tool handler accepts built-in skill names (skill-review, self-dev, self-knowledge, etc.) and will inspect or persist generated variants for them. This is a trust-boundary violation — built-in skills are platform-managed and define the agent's operating contract. An agent should not be able to rewrite its own `skill-review` prompt (meta self-modification).

Observed: mika-qa (gemini-2.5-flash) reviewed `skill-review` itself, producing a real variant at `~/.mika/agents/mika-qa/skills/skill-review/generated/google/gemini-2.5-flash/system_prompt.md`.

## Problem Statement

The `review_skill` tool validates path traversal, linked-skill status, and filesystem existence — but never checks whether the target skill is a platform-managed built-in. Other tool handlers (`delete_skill`, `update_skill`, skill installation) already guard against built-in modifications using the existing `is_bundled_skill()` function. `review_skill` is the only tool that lacks this guard.

## Proposed Solution

Add `is_bundled_skill(skill_name)` guards to both the single-skill and batch-skill paths in `review_skill`. Block both inspect and persist modes — the tool's purpose is variant generation, which is meaningless for built-in skills that are overwritten by `seed_bundled_skills()` on every restart. Update the skill-review system prompt to mention the restriction so agents avoid wasting tool calls.

## Implementation

### 1. Guard in `review_skill()` single-skill path

**File:** `crates/mika-agent/src/skills/builtin_handlers.rs`

After input validation (path traversal check at ~line 952) and **before** the batch/single dispatch (~line 993), add:

```rust
// Block built-in skills early (before filesystem existence check)
// so case-mismatched names get "built-in" error, not "not found"
if !is_batch && is_bundled_skill(&skill_name) {
    return Ok(format!(
        "Cannot review built-in skill '{}'. Built-in skills are platform-managed and updated automatically.",
        skill_name
    ));
}
```

Place this before the filesystem existence check so `review_skill(skill_name="SKILL-REVIEW")` returns "built-in" (via `eq_ignore_ascii_case`) rather than "not found" on case-sensitive filesystems.

### 2. Guard in `review_skill_batch()` loop

**File:** `crates/mika-agent/src/skills/builtin_handlers.rs`

Inside the directory iteration loop (~line 1266), add a skip alongside the existing linked-skill skip:

```rust
if is_bundled_skill(&name_str) {
    skipped.push(format!("{} (built-in)", name_str));
    continue;
}
```

### 3. Update skill-review system prompt

**File:** `crates/mika-agent/templates/skills/skill-review/system_prompt.md`

Add a "Restrictions" section noting that built-in skills cannot be reviewed:

```markdown
## Restrictions

Built-in skills are platform-managed and cannot be reviewed or adapted. The `review_skill`
tool will reject them with an error. Built-in skills include: tmux, shell-exec, web-search,
file-reader, skill-review, self-knowledge, git-ops, google-workspace, github, mcp,
browser-control, agents-teams. Batch mode (`skill_name: "*"`) automatically skips them.
```

### 4. Tests

**File:** `crates/mika-agent/src/skills/builtin_handlers.rs` (test module)

Add tests:

| Test | Description |
|------|-------------|
| `test_review_skill_rejects_builtin` | Single skill inspect of a built-in returns error message |
| `test_review_skill_rejects_builtin_persist` | Single skill persist of a built-in returns error message |
| `test_review_skill_batch_skips_builtins` | Batch mode includes built-ins in skipped list with "built-in" reason |
| `test_review_skill_rejects_builtin_case_insensitive` | `"SKILL-REVIEW"` is rejected (case-insensitive via `eq_ignore_ascii_case`) |

## Acceptance Criteria

- [x] `review_skill(skill_name="skill-review")` returns clear error without touching filesystem
- [x] `review_skill(skill_name="skill-review", content="...")` returns same error (persist blocked)
- [x] `review_skill(skill_name="*")` skips all 12 built-in skills with "built-in" reason in skipped list
- [x] Case-insensitive matching: `"SHELL-EXEC"`, `"Self-Knowledge"` all blocked
- [x] Error message follows existing convention: "Cannot review built-in skill '{}'. Built-in skills are platform-managed and updated automatically."
- [x] skill-review system prompt mentions the restriction
- [x] All new tests pass
- [x] Existing `review_skill` tests continue to pass
- [x] `cargo clippy` clean, `cargo test` green

## Out of Scope

- **Stale variant cleanup:** Existing generated variants under built-in skill directories persist. `seed_bundled_skills()` doesn't purge `generated/` subdirs. File as follow-up if needed.
- **`write_agent_file` bypass:** An agent could theoretically use `write_agent_file` to write directly into `skills/<built-in>/generated/`. This is a pre-existing concern, not introduced by this change.
- **Manifest `builtin` field:** The issue suggests adding `builtin = true` to `skill.toml`. This is unnecessary — `is_bundled_skill()` is already data-driven via the `BUNDLED_SKILLS` array and is used by all other guard paths. A manifest field would be less secure (a malicious skill could set `builtin = false`).

## Sources

- Related issue: #480
- Existing guard pattern: `is_bundled_skill()` in `bundled_skills.rs:150` — used by `delete_skill`, `update_skill`, `install`
- Architecture doc: `docs/solutions/architecture-patterns/merge-two-step-llm-tool-contracts.md`
- Architecture doc: `docs/solutions/architecture-patterns/harden-write-skill-variant-no-path-input.md`
- Linked skill unblock: `docs/solutions/architecture-patterns/skill-llm-override-db-layer-and-linked-unblock.md` (PR #475)
