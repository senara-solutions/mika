---
status: pending
priority: p2
issue_id: "485"
tags: [code-review, agent-native, tools, security]
dependencies: []
---

# Builtin read_file Silently Shadows exec-handler file-reader Skill

## Problem Statement

Both the new builtin `read_file` tool and the bundled `file-reader` exec-handler skill register
under the tool name `"read_file"`. The dispatch chain is builtins → skills → MCP, so the
builtin silently shadows the skill. The exec-handler skill accepts any filesystem path (absolute
or relative, including paths outside home dir) and returns binary image files via the
`__mika_v1` envelope. The builtin is sandboxed to the agent home dir and calls `read_to_string`
— it fails with "Absolute paths are not allowed" for paths outside home dir. The prompt at
`prompt.rs:307` instructs the agent to "use read_file on that path to view the image contents"
for images returned by tools — this instruction now silently fails for non-home-dir paths.

## Findings

- **Source**: agent-native-reviewer review
- **Location**: `tools/read_file.rs:16–17` (builtin name "read_file"), `templates/skills/file-reader/tools.json` (skill name "read_file")
- The `system_prompt.md` for file-reader skill references the `read_file` tool name
- Agents expecting to view screenshots or files from `/tmp/` or other paths outside home dir
  cannot do so through the now-shadowed skill

## Proposed Solutions

### Option A: Rename builtins to home_dir-scoped names (Recommended)
- `read_file` → `read_home_file`
- `list_files` → `list_home_files`
- `write_file` → `write_home_file` (for consistency)
Update prompt docs and tool descriptions. The skill retains `read_file` for broader access.
- **Pros**: Eliminates collision, accurately describes scope, restores image-read capability
- **Cons**: Breaking rename for write_file (already in use)
- **Effort**: Small | **Risk**: Low (no external API, internal tool names)

### Option B: Rename the skill's tool
Rename the file-reader skill's tool from `read_file` to `read_any_file` or `read_path`.
- **Pros**: Smaller change (only skill)
- **Cons**: Existing agent conversations referencing the skill tool name break
- **Effort**: Tiny | **Risk**: Low

### Option C: Restrict builtin, restore skill dispatch
Make the builtin not register when the file-reader skill is present (conditional registration).
- **Pros**: No rename needed
- **Cons**: Complex conditional registration logic
- **Effort**: Medium | **Risk**: Medium

## Acceptance Criteria

- [ ] Builtin home-dir file operations and exec-handler arbitrary file operations do not share a name
- [ ] Agent can view images via the exec-handler skill after this fix
- [ ] Prompt documentation accurately reflects which tool serves which scope
- [ ] `file-reader` skill's `__mika_v1` image return path still works for agents

## Work Log

- 2026-03-06: Identified by agent-native-reviewer of feat/unified-task-engine
