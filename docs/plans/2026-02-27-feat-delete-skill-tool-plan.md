---
title: "feat: Add delete_skill tool to complete CRUD"
type: feat
status: completed
date: 2026-02-27
---

# feat: Add delete_skill tool to complete CRUD

## Overview

Add a `delete_skill` tool to complete the skill management CRUD operations. The existing tools are `create_skill`, `update_skill`, `list_skills`, and `toggle_skill`. The missing `delete_skill` tool allows users to permanently remove custom skills from the filesystem.

## Problem Statement / Motivation

The skills system currently supports Create (create_skill), Read (list_skills), Update (update_skill), and a soft-disable (toggle_skill), but has no way to permanently remove a custom skill. Users who create experimental or one-off skills have no way to clean them up without manual filesystem access.

## Proposed Solution

Add a `delete_skill` tool that removes a custom skill's entire directory from `{home_dir}/skills/{name}/`. Built-in (bundled) skills are protected from deletion. The tool follows the exact same validation, security, and error-handling patterns established by the existing four skill tools.

## Technical Approach

### Execution Order

Following the pattern established by `toggle_skill` and `update_skill` (which require `canonicalize()` on existing paths):

```
1. Extract and trim `name` from input
2. validate_skill_name(name) → ToolOutput::error on failure
3. Construct skills_dir = ctx.home_dir.join("skills")
4. Construct skill_dir = skills_dir.join(name)
5. Check skill_dir.exists() → ToolOutput::error("Skill '{name}' not found.") if false
6. is_bundled_skill(name) → ToolOutput::error with toggle guidance if true
7. verify_skill_path(&skills_dir, &skill_dir) → ToolOutput::error on failure
8. std::fs::remove_dir_all(&skill_dir) → ToolOutput::error on failure
9. Return ToolOutput::success with restart reminder
```

### Files to Create

#### `crates/mika-agent/src/tools/delete_skill.rs` (new)

```rust
use super::create_skill::{validate_skill_name, verify_skill_path};
use super::{Tool, ToolContext, ToolOutput};
use crate::bundled_skills::is_bundled_skill;
use async_trait::async_trait;
use mika_common::claude::ToolDefinition;
use serde_json::Value;

pub struct DeleteSkillTool;

#[async_trait]
impl Tool for DeleteSkillTool {
    fn name(&self) -> &str { "delete_skill" }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "delete_skill".to_string(),
            description: "Permanently delete a custom skill. Removes the skill directory and all its files. Built-in skills cannot be deleted — use toggle_skill to disable them instead. Changes take effect after restarting the conversation.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "name": {
                        "type": "string",
                        "description": "Name of the skill to delete"
                    }
                },
                "required": ["name"]
            }),
        }
    }

    async fn execute(&self, input: Value, ctx: &ToolContext<'_>) -> anyhow::Result<ToolOutput> {
        let name = input["name"].as_str().unwrap_or("").trim();

        // 1. Validate skill name
        if let Err(e) = validate_skill_name(name) {
            return Ok(ToolOutput::error(e));
        }

        // 2. Check skill exists
        let skills_dir = ctx.home_dir.join("skills");
        let skill_dir = skills_dir.join(name);

        if !skill_dir.exists() {
            return Ok(ToolOutput::error(format!("Skill '{name}' not found.")));
        }

        // 3. Prevent deletion of built-in skills
        if is_bundled_skill(name) {
            return Ok(ToolOutput::error(format!(
                "Cannot delete built-in skill '{name}'. Use toggle_skill to disable it instead."
            )));
        }

        // 4. Verify path safety (symlink escape prevention)
        if let Err(e) = verify_skill_path(&skills_dir, &skill_dir) {
            return Ok(ToolOutput::error(e));
        }

        // 5. Delete the skill directory
        if let Err(e) = std::fs::remove_dir_all(&skill_dir) {
            return Ok(ToolOutput::error(format!("Failed to delete skill: {e}")));
        }

        Ok(ToolOutput::success(format!(
            "Deleted skill '{name}'.\nChanges take effect after restarting the conversation."
        )))
    }
}
```

### Files to Modify

#### `crates/mika-agent/src/tools/mod.rs`

1. Add `mod delete_skill;` in alphabetical order among the skill tool modules
2. Register `delete_skill::DeleteSkillTool` in `default_tools()` alongside other skill tools

#### `crates/mika-agent/src/prompt.rs`

Add `delete_skill` mention in the Tool Usage section for agent discoverability, following the pattern established by `update_skill` (commit f87d38b finding).

## Acceptance Criteria

### Functional Requirements

- [x] `delete_skill` tool removes custom skill directories completely (`skill.toml`, `system_prompt.md`, `tools.json`, `handlers/`, `.disabled`)
- [x] Built-in/bundled skills are protected from deletion with helpful error message guiding to `toggle_skill`
- [x] Non-existent skill names return clean error without path leakage
- [x] Invalid skill names (empty, path traversal, special chars) are rejected via `validate_skill_name()`
- [x] Symlink attacks are caught by `verify_skill_path()`
- [x] Success message includes restart reminder consistent with other skill tools
- [x] Tool is registered in `default_tools()` and available to the agent
- [x] Tool is mentioned in `prompt.rs` Tool Usage section with test assertion

### Non-Functional Requirements

- [x] No `display()` calls on paths in user-facing error messages (opaque errors)
- [x] Shared validators imported from `create_skill`, not duplicated
- [x] Follows the exact same struct/trait/test patterns as existing skill tools

### Test Coverage

- [x] `test_delete_skill_success` — custom skill directory is removed
- [x] `test_delete_skill_not_found` — returns error, no path leakage
- [x] `test_delete_skill_bundled_rejected` — built-in skill protected, error includes toggle guidance
- [x] `test_delete_skill_invalid_name` — empty, path traversal, special chars rejected
- [x] `test_delete_skill_disabled_skill` — disabled custom skill can still be deleted
- [x] `test_delete_skill_with_subdirectories` — skills with handlers/ dir are fully removed
- [x] `test_delete_skill_symlink_escape` — verify_skill_path catches symlink attacks
- [x] `test_delete_skill_prompt_mention` — prompt.rs contains delete_skill reference

## Design Decisions

1. **No confirmation parameter**: Consistent with existing skill tools (none have confirmation). The agent can ask the user in conversation before calling.
2. **No `dry_run` parameter**: Keep interface minimal. Agent can use `list_skills` first.
3. **`is_bundled_skill()` check before `verify_skill_path()`**: More efficient (avoids canonicalization) and provides clearer error message for bundled skills.
4. **Disabled skills are deletable**: A custom skill with `.disabled` marker can still be permanently removed — `remove_dir_all` handles this naturally.
5. **No database cleanup needed**: Skills are purely filesystem-based. Tool call summaries in conversation history are historical records.

## Dependencies & Risks

- **Low risk**: This is a straightforward addition following well-established patterns
- **Irreversible deletion**: Custom skills are permanently removed (unlike bundled skills which re-seed). Agent judgment + conversation context serve as the safety net.
- **In-memory registry**: Deleted skill remains active in the current session's `SkillRegistry` until restart. Success message communicates this.

## References & Research

### Internal References

- Pattern source: `crates/mika-agent/src/tools/create_skill.rs` (shared validators)
- Pattern source: `crates/mika-agent/src/tools/update_skill.rs` (update pattern)
- Pattern source: `crates/mika-agent/src/tools/toggle_skill.rs` (toggle pattern)
- Bundled check: `crates/mika-agent/src/bundled_skills.rs:109` (`is_bundled_skill()`)
- Tool trait: `crates/mika-agent/src/tools/mod.rs:49-59`
- Registration: `crates/mika-agent/src/tools/mod.rs:187` (`default_tools()`)

### Institutional Learnings

- `docs/solutions/logic-errors/update-skill-tool-discoverability-and-parity-gaps.md` — 8 critical issues found in update_skill's initial review. All patterns must be replicated.
- Key invariants: prompt integration + test assertion, verify_skill_path before filesystem ops, opaque error messages, shared validators
