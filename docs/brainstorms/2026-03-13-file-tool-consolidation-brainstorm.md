# Brainstorm: File Tool Consolidation

**Date:** 2026-03-13
**Issue:** [#127](https://github.com/senara-solutions/mika/issues/127)
**Status:** Ready for planning

## What We're Building

Rename the three builtin agent-home file tools to use an `_agent_` naming convention, making the ownership model self-documenting and eliminating confusion with the skill-based `read_file` tool.

```
read_home_file   →  read_agent_file
list_home_files  →  list_agent_files
write_file       →  write_agent_file
```

Also delete the orphaned `read_file.rs` (dead code from a prior rename refactor).

## Why This Approach

### The two-tool architecture is correct by design

The issue initially asked whether `read_home_file` and `read_file` should be consolidated. After analysis, keeping both is the right call. The distinction isn't just path scope — it's **execution context**:

| | Builtin `*_agent_*` tools | Skill-based `read_file` |
|---|---|---|
| **Handler** | Compiled Rust, in-process | Exec subprocess (shell script) |
| **Scope** | Agent home dir only (`~/.mika/agents/{name}/`) | Arbitrary filesystem |
| **Image support** | No (text only) | Yes (`__mika_v1` envelope) |
| **Silent mode** | Available | Filtered out by `safe_always_on_skills()` |
| **Path validation** | `validate_and_resolve_path` (symlink, traversal, containment) | Shell-level only |
| **Purpose** | Mika reading/writing her own data | Mika reading user's files |

The critical constraint: **`safe_always_on_skills()` filters out all exec/http handler skills from silent/heartbeat mode.** Consolidating into a single exec handler would break Mika's ability to read her own config and data during background tasks. That's a real regression.

### The problem is naming, not architecture

`read_home_file` sounds like "read from `~/`" (user's home), not "read from the agent's home directory." `write_file` has no `_home_` signal at all despite being sandboxed to agent home with an overwrite confirmation flow. The `_agent_` infix makes the ownership model explicit:

- `*_agent_*` = sandboxed to Mika's home dir, path-validated, safe in silent mode
- `read_file` = exec skill, arbitrary filesystem, image-capable, excluded from silent mode

## Key Decisions

1. **Keep both tools** — The execution context boundary (builtin vs exec handler) serves a real architectural purpose (silent mode availability).

2. **Rename all three builtin file tools** — Consistency matters. Renaming only `read_home_file` while leaving `list_home_files` and `write_file` just moves the confusion.

3. **No image support in builtin** — The exec skill's image pipeline (`file --mime-type`, magic-byte validation, `__mika_v1` envelope, base64, 5MB/5-image cap) is already built and tested. Duplicating it in the Rust builtin creates two maintenance paths without a compelling use case. Silent-mode image analysis isn't a real scenario — if a background task needs image reading, it should be a delegated agent turn, not a heartbeat.

4. **Delete orphaned `read_file.rs`** — Dead code from the original rename (commit fec82a2). Not imported, not registered, wouldn't compile.

## Scope

### In scope
- Rename tool names in structs, `default_tools()`, and tool description strings
- Rename source files: `read_home_file.rs` → `read_agent_file.rs`, `list_home_files.rs` → `list_agent_files.rs`, `write_file.rs` → `write_agent_file.rs`
- Update `mod` declarations in `tools/mod.rs`
- Update all system prompt references in `prompt.rs`
- Update tests asserting tool names
- Update documentation (`docs/architecture.md`, `CLAUDE.md`, solution docs)
- Delete orphaned `read_file.rs`

### Out of scope
- Adding image support to the builtin (deliberately excluded; revisit if silent-mode image analysis becomes necessary)
- Changing the file-reader skill's `read_file` tool name
- Any behavior changes — this is a pure rename

## Open Questions

None — all decisions resolved during brainstorming.

## Notes

- The `docs/solutions/logic-errors/builtin-skill-tool-name-shadowing.md` solution doc explains the original rename history and the `_home_` naming convention recommendation. It should be updated to reflect the new `_agent_` convention.
- Migration cost is bounded: tool name rename in `default_tools()`, tool structs, system prompt, tests. No schema changes, no behavior changes.
