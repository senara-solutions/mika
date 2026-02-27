---
title: "Complete Skill Management CRUD with delete_skill Tool"
date: 2026-02-27
severity: Low
module: "mika-agent crate — tools subsystem"
symptoms:
  - "No way to permanently remove custom skills from the system"
  - "Users forced to use manual filesystem access for skill deletion"
  - "Incomplete CRUD operations on skills (Create, Update, List, Toggle existed; Delete was missing)"
root_cause: "The skill management toolset was incomplete. While create_skill, update_skill, list_skills, and toggle_skill existed, the delete operation was never implemented, leaving users without a safe way to remove skills they no longer needed."
fix_commit_shas:
  - "502b2fb"
  - "5f7e987"
related_issues: []
tags:
  - "skills-system"
  - "CRUD-completeness"
  - "feature-addition"
  - "tool-development"
  - "security"
investigation_time: "1 hour"
resolution_time: "2 hours"
---

# Complete Skill Management CRUD with delete_skill Tool

## Root Cause

The Mika agent's skills system had tools for creating (`create_skill`), updating (`update_skill`), listing (`list_skills`), and toggling (`toggle_skill`) skills, but lacked a mechanism for permanently deleting custom skills. Users could disable skills via `toggle_skill`, but couldn't remove them entirely from the filesystem, leaving orphaned skill directories.

## Solution

Implemented a `delete_skill` tool that safely removes custom skills from the filesystem while protecting built-in skills and preventing directory traversal attacks.

### Execution Order

```
1. Extract and trim `name` from input
2. validate_skill_name(name) — reused from create_skill.rs
3. Check skill_dir.exists() — opaque "not found" error
4. is_bundled_skill(name) — protect built-ins, suggest toggle_skill
5. verify_skill_path() — symlink escape prevention
6. std::fs::remove_dir_all() — recursive deletion
7. Return success with restart reminder
```

### Code Changes

| File | Change |
|------|--------|
| `crates/mika-agent/src/tools/delete_skill.rs` | New tool: `DeleteSkillTool` struct implementing `Tool` trait (267 lines: 75 code + 192 tests) |
| `crates/mika-agent/src/tools/mod.rs` | Added `mod delete_skill;` and registered in `default_tools()` |
| `crates/mika-agent/src/prompt.rs` | Added agent discoverability hint + test assertion |
| `CLAUDE.md` | Updated test count (~512 to ~538) |

## Key Design Decisions

1. **Shared validator reuse**: Leveraged existing `validate_skill_name()` and `verify_skill_path()` from `create_skill.rs` — no code duplication.
2. **Built-in skill protection**: `is_bundled_skill()` check prevents deletion of bundled skills (tmux, shell-exec, web-search, file-reader, calendar, self-knowledge). Error message guides users to `toggle_skill` instead.
3. **Opaque error messages**: Errors never expose filesystem paths (e.g., `"Skill 'foo' not found."` not `"Skill not found at /home/user/.mika/skills/foo"`).
4. **No confirmation parameter**: Consistent with existing skill tools. The agent can ask the user conversationally before calling.
5. **Disabled skills are deletable**: A custom skill with `.disabled` marker can still be permanently removed.
6. **No database cleanup needed**: Skills are purely filesystem-based; no DB state to clean up.

## Test Coverage

Nine comprehensive tests:

| Test | What it verifies |
|------|-----------------|
| `test_delete_skill_success` | Custom skill directory removed, success message includes restart reminder |
| `test_delete_skill_not_found` | Opaque error, no path leakage (asserts `"skills/"` absent from error) |
| `test_delete_skill_bundled_rejected` | Built-in skill protected, error includes `toggle_skill` guidance |
| `test_delete_skill_invalid_name_empty` | Empty name rejected |
| `test_delete_skill_invalid_name_path_traversal` | `../evil` rejected |
| `test_delete_skill_invalid_name_special_chars` | `sk!ll` rejected |
| `test_delete_skill_disabled_skill` | Disabled custom skill can be deleted |
| `test_delete_skill_with_subdirectories` | Skills with `handlers/` dir fully removed |
| `test_delete_skill_symlink_escape` | Unix-only: symlink to external dir detected, external dir preserved |

## What Went Right

The implementation proactively consulted institutional learnings from the `update_skill` review (which found 8 issues). By applying those patterns from the start:

- Prompt integration + test assertion added immediately (not discovered in review)
- All shared validators reused (no duplication to find in review)
- Bundled skill protection added (unique to delete since deletion is irreversible)
- Security guards (symlink, path opacity) applied uniformly
- All 5 review agents found zero P1 or P2 issues

## Prevention Strategies

### Checklist for Adding New Skill-Mutating Tools

1. **Consult institutional memory**: Read `docs/solutions/` for related tool reviews
2. **Prompt integration**: Add tool hint to `prompt.rs` Tool Usage section + test assertion
3. **Validation parity**: Reuse shared validators from `create_skill.rs`, never duplicate
4. **Security guards**: Call `verify_skill_path()` before any filesystem operation; never leak paths in errors
5. **Bundled protection**: Check `is_bundled_skill()` for destructive operations
6. **Module registration**: Add `mod` and `register()` in alphabetical order in `mod.rs`
7. **Test helpers**: Use `TestHarness::ctx_with_home()` — do not duplicate
8. **Post-mutation invariants**: Validate skill still has trigger mechanism (keywords or always_on)
9. **Cross-tool review**: Verify all related tools follow identical patterns

### Key Invariants

| Invariant | Guard | Applied In |
|-----------|-------|-----------|
| Path stays inside skills root | `verify_skill_path()` | create, update, toggle, delete |
| Skill names are safe | `validate_skill_name()` | create, update, toggle, delete |
| Error messages are opaque | No `path.display()` in errors | All filesystem-touching tools |
| Agent discoverability | Tool name in prompt + test assertion | All tools |
| Built-in skills protected | `is_bundled_skill()` | create (overwrite check), delete |

## Related Documentation

- [update-skill-tool-discoverability-and-parity-gaps.md](../logic-errors/update-skill-tool-discoverability-and-parity-gaps.md) — 8 critical issues found in update_skill review; all patterns applied here
- [agent-skill-hallucination-tui-scroll-telegram-awareness.md](../logic-errors/agent-skill-hallucination-tui-scroll-telegram-awareness.md) — Original create_skill/list_skills/toggle_skill implementation
- [filesystem-skill-registry-implementation.md](../architecture-decisions/filesystem-skill-registry-implementation.md) — Skill system architecture and security model
- [agent-api-self-knowledge-and-skill-origin-awareness.md](../logic-errors/agent-api-self-knowledge-and-skill-origin-awareness.md) — Agent discoverability requirements
