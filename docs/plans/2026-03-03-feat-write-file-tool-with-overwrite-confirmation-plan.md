---
title: "feat: Add write_file tool with overwrite confirmation flow"
type: feat
status: completed
date: 2026-03-03
---

# feat: Add write_file tool with overwrite confirmation flow

## Overview

Add a built-in `write_file` tool that writes content to files within the agent's home directory, with a hard-enforced confirmation flow for overwrites. If the target file already exists, the tool returns the current content and requires a second call with `confirm: true` to proceed. Additionally, update the shell-exec skill's system prompt with a soft guardrail instructing the agent to read files before writing via shell commands.

## Problem Statement / Motivation

The agent currently relies on the `run_shell` skill (shell-exec) for all file writes. Shell commands like `cat >`, `tee`, `sed -i` are powerful but dangerous — they can silently overwrite files without the agent seeing what was there before. There's no structural enforcement that the agent reads before it writes.

A dedicated `write_file` tool provides:
1. **Hard enforcement:** The agent *must* see existing content before overwriting — the tool returns the content and refuses to write without `confirm: true`
2. **Security:** Path traversal protection, symlink rejection, containment checks — all enforced at the Rust level
3. **Auditability:** Structured tool calls are easier to trace than arbitrary shell commands
4. **Safety net for shell writes:** A soul-level soft guardrail in shell-exec's system prompt reminds the agent to read-before-write even when using shell commands

## Proposed Solution

### Part 1: `write_file` built-in tool

New file: `crates/mika-agent/src/tools/write_file.rs`

**Tool schema:**
```json
{
  "name": "write_file",
  "description": "Write content to a file in the agent's home directory. If the file already exists, the current content is returned and you MUST call again with confirm: true to overwrite.",
  "input_schema": {
    "type": "object",
    "properties": {
      "path": {
        "type": "string",
        "description": "Relative path within the agent's home directory (e.g., 'notes/todo.md')"
      },
      "content": {
        "type": "string",
        "description": "Content to write to the file"
      },
      "confirm": {
        "type": "boolean",
        "description": "Set to true to confirm overwriting an existing file. Required when the file already exists."
      }
    },
    "required": ["path", "content"]
  }
}
```

**Execution flow:**
1. Validate `path`: non-empty, within MAX_INPUT_LEN, no absolute paths, no traversal components (`..`, root, prefix)
2. Validate `content`: non-empty, within MAX_INPUT_LEN
3. Resolve `full_path = home_dir.join(path)`
4. Create parent directories if needed
5. Verify containment: canonical resolved path must be within canonical `home_dir` (same pattern as `write_workspace`)
6. Reject symlinks in the parent chain
7. **If file exists and `confirm != true`:** Read the existing content (cap at 100KB), return it as a `ToolOutput::error` with message: `"File already exists at '{path}' ({size} bytes). Current content is shown below. Call again with confirm: true to overwrite.\n\n{existing_content}"`
8. **If file exists and `confirm == true`:** Proceed to write
9. **If file does not exist:** Write immediately (no confirmation needed)
10. Return `ToolOutput::success("Wrote {bytes} bytes to '{path}'.")`

**Security measures** (matching `write_workspace` patterns):
- Reject absolute paths
- Reject path traversal via component inspection
- Reject symlinks via `symlink_metadata` check on parent chain
- Verify containment via `canonicalize` against `home_dir`
- Content size limit: `MAX_INPUT_LEN` (10,000 chars)
- Existing file preview cap: 100KB (same as `read_workspace`'s `MAX_READ_SIZE`)

### Part 2: Shell-exec soft guardrail

Update: `crates/mika-agent/templates/skills/shell-exec/system_prompt.md`

Add a new section after the existing "Config file editing" section:

```markdown
## File writing

Before writing to any file via shell (cat >, tee, sed -i, echo >, heredoc, etc.):
- ALWAYS read the file first if it exists, to understand what you're replacing.
- Prefer the `write_file` tool over shell writes when the content fits — it enforces read-before-overwrite automatically.
- When shell writes are necessary (e.g., binary data, piping, large files), use `cat <file>` or `head <file>` first.
```

### Part 3: Register in `default_tools()`

Update: `crates/mika-agent/src/tools/mod.rs`

- Add `mod write_file;` to the module declarations (line ~24)
- Add `registry.register(Box::new(write_file::WriteFileTool));` to `default_tools()` (line ~261)

## Technical Considerations

- **Architecture impact:** Minimal. Unit struct tool (no state needed — `home_dir` comes from `ToolContext`). Follows exact same pattern as `StoreFactTool`.
- **Performance:** Synchronous file I/O via `tokio::fs` (non-blocking). Reading existing content for the confirmation flow adds one extra filesystem read, but only when the file exists and `confirm` is false.
- **Security:** Strict path validation prevents escape from `home_dir`. The tool cannot write outside the agent's home directory. Symlink checks prevent TOCTOU attacks.
- **Content size:** `MAX_INPUT_LEN` (10,000 chars) is consistent with other tools. For larger writes, the agent uses `run_shell`.

## Acceptance Criteria

- [x] `write_file` tool creates new files immediately without confirmation
- [x] `write_file` returns existing content and requires `confirm: true` for overwrites
- [x] `write_file` rejects absolute paths, path traversal, and symlinks
- [x] `write_file` creates parent directories automatically
- [x] `write_file` is registered in `default_tools()` and available to all agents
- [x] Shell-exec `system_prompt.md` includes read-before-write guidance
- [x] Comprehensive tests covering: new file, overwrite flow (reject then confirm), path traversal, absolute path, empty inputs, symlink rejection, large existing file truncation
- [x] `cargo test` passes, `cargo clippy` clean

## MVP

### crates/mika-agent/src/tools/write_file.rs

```rust
pub struct WriteFileTool;

#[async_trait]
impl Tool for WriteFileTool {
    fn name(&self) -> &str { "write_file" }
    fn definition(&self) -> ToolDefinition { /* schema above */ }
    async fn execute(&self, input: Value, ctx: &ToolContext<'_>) -> Result<ToolOutput> {
        // 1. Validate path + content
        // 2. Resolve full_path = ctx.home_dir.join(path)
        // 3. Security checks (traversal, symlink, containment)
        // 4. If exists && !confirm → return existing content
        // 5. Create parent dirs + write
    }
}
```

### crates/mika-agent/src/tools/mod.rs (changes)

```rust
mod write_file;  // add to module declarations

// In default_tools():
registry.register(Box::new(write_file::WriteFileTool));
```

### crates/mika-agent/templates/skills/shell-exec/system_prompt.md (append)

```markdown
## File writing

Before writing to any file via shell (cat >, tee, sed -i, echo >, heredoc, etc.):
- ALWAYS read the file first if it exists, to understand what you're replacing.
- Prefer the `write_file` tool over shell writes when the content fits — it enforces read-before-overwrite automatically.
- When shell writes are necessary (e.g., binary data, piping, large files), use `cat <file>` or `head <file>` first.
```

## Sources

- Similar implementations: `crates/mika-agent/src/tools/write_workspace.rs` (path validation, containment), `crates/mika-agent/src/tools/read_workspace.rs` (file reading with size limits)
- Tool trait: `crates/mika-agent/src/tools/mod.rs:62-78`
- Tool registration: `crates/mika-agent/src/tools/mod.rs:245-263`
- Shell-exec prompt: `crates/mika-agent/templates/skills/shell-exec/system_prompt.md`
