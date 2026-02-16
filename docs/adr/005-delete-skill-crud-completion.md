# ADR-005: Complete Skill Management CRUD with delete_skill Tool

**Date:** 2026-02-27
**Status:** Accepted
**Component:** mika-agent (tools subsystem)

## Context

The skill management toolset had create, update, list, and toggle operations but
lacked delete. Users could disable skills via `toggle_skill` but couldn't permanently
remove custom skills, leaving orphaned directories.

## Decision

Implement a `delete_skill` tool that safely removes custom skills from the filesystem
while protecting built-in skills and preventing directory traversal attacks.

### Execution Flow

1. Validate skill name (`validate_skill_name()` — shared with create/update/toggle)
2. Check skill directory exists (opaque "not found" error — no path leakage)
3. Reject built-in skills (`is_bundled_skill()` — suggest `toggle_skill` instead)
4. Verify path containment (`verify_skill_path()` — symlink escape prevention)
5. Remove directory recursively (`std::fs::remove_dir_all()`)
6. Return success with restart reminder

### Key Design Choices

1. **Shared validator reuse** — no code duplication with create/update/toggle
2. **Built-in skill protection** — bundled skills cannot be deleted (irreversible),
   error guides users to `toggle_skill`
3. **Opaque error messages** — errors never expose filesystem paths
4. **No confirmation parameter** — agent asks the user conversationally before calling
5. **Disabled skills are deletable** — custom skills with `.disabled` marker can be removed
6. **No database cleanup** — skills are purely filesystem-based

## Consequences

- Complete CRUD for skill management (create, update, list, toggle, delete)
- Consistent security model across all skill-mutating tools
- Restart required after deletion for registry to update

### Invariants Across All Skill-Mutating Tools

| Invariant | Guard |
|-----------|-------|
| Path stays inside skills root | `verify_skill_path()` |
| Skill names are safe | `validate_skill_name()` |
| Error messages are opaque | No `path.display()` in errors |
| Agent discoverability | Tool name in prompt + test assertion |
| Built-in skills protected | `is_bundled_skill()` for destructive ops |
