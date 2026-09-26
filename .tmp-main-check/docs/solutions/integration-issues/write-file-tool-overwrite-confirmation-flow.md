---
title: "Write File Tool with Overwrite Confirmation Flow"
problem_type: integration_issue
component: mika-agent/tools
date: 2026-03-03
tags: [tools, file-operations, security, confirmation-flow, agent-safety]
severity: medium
resolution_time: ~2 hours
---

# Write File Tool with Overwrite Confirmation Flow

## Problem Symptom

The agent had a `write_workspace` tool for shared workspace files, but no builtin tool for writing files to its own home directory. Meanwhile, `read_file` was a skill-based exec handler (filtered out in silent/heartbeat mode), creating an asymmetry: the agent could potentially lose access to file operations in certain contexts. Additionally, arbitrary shell commands (`cat >`, `tee`, `sed -i`) could overwrite files without the agent first reviewing existing content.

## Investigation Steps

1. **Audited existing tool patterns:** Examined `write_workspace.rs`, `read_workspace.rs`, and `store_fact.rs` for path validation, error handling, and test patterns.
2. **Reviewed `ToolContext` fields:** Confirmed `home_dir` is available on every tool context, providing the sandboxed directory.
3. **Checked `default_tools()` registration:** Found the pattern in `tools/mod.rs` for registering builtin tools.
4. **Reviewed security model:** Path traversal prevention via component inspection, symlink checks, and canonicalize containment were already established in `write_workspace.rs`.

## Root Cause Analysis

No builtin `write_agent_file` tool existed. The gap meant:
- No enforced read-before-overwrite for home directory files
- Shell-based file writes had no guardrails
- Agent couldn't safely manage its own files in all operating modes

## Working Solution

### 1. Created `write_agent_file.rs` Tool

**File:** `crates/mika-agent/src/tools/write_agent_file.rs`

Key design decisions:
- **Confirmation flow:** If file exists and `confirm` is not `true`, return existing content as an error. Agent must explicitly acknowledge what it's replacing.
- **New files write immediately:** No confirmation needed for new files.
- **Path security:** Uses shared `validate_and_resolve_path()` helper (empty check, length limit, absolute path rejection, component traversal inspection, parent dir creation, symlink checks, canonicalize containment).
- **Target-file symlink check:** Defense-in-depth check on the target file itself using `symlink_metadata()`, preventing writes through symlinks that could escape the home directory.

```rust
// Confirmation flow core logic
if file_exists && !confirm {
    match tokio::fs::read_to_string(&full_path).await {
        Ok(existing) => {
            return Ok(ToolOutput::error(format!(
                "File already exists at '{path}' ({size} bytes). \
                 Current content is shown below. \
                 Call again with confirm: true to overwrite.\n\n{existing}"
            )));
        }
        // ... error handling
    }
}
```

### 2. Extracted Shared Path Validation Helper

**File:** `crates/mika-agent/src/tools/mod.rs`

The `validate_and_resolve_path()` function consolidates ~60 lines of duplicated path security logic from `write_agent_file`, `write_workspace`, and `read_workspace`:

```rust
pub(crate) async fn validate_and_resolve_path(
    path: &str,
    base_dir: &Path,
) -> std::result::Result<PathBuf, ToolOutput> {
    // 1. Empty path check
    // 2. Length check (MAX_INPUT_LEN)
    // 3. Absolute path rejection
    // 4. Component traversal inspection (rejects "..")
    // 5. Parent directory creation
    // 6. Parent symlink check via symlink_metadata
    // 7. Canonicalize containment verification
}
```

### 3. Added Shell-Exec Soft Guardrail

**File:** `crates/mika-agent/templates/skills/shell-exec/system_prompt.md`

Added a "File writing" section as a soul-level instruction:

```markdown
## File writing
Before writing to any file via shell (cat >, tee, sed -i, echo >, heredoc, etc.):
- ALWAYS read the file first if it exists, to understand what you're replacing.
- Prefer the `write_agent_file` tool over shell writes when the content fits.
- When shell writes are necessary, use `cat <file>` or `head <file>` first.
```

### 4. Registered in `default_tools()`

Added `write_agent_file::WriteAgentFileTool` to the builtin tool registry, making it available to all agents in all modes (including silent/heartbeat).

## Prevention Strategies

### Security Checklist for New File Tools
1. Always use `validate_and_resolve_path()` for any tool accepting file paths
2. Check symlinks on both parent directories AND target files
3. Reject absolute paths and `..` components
4. Verify canonicalized path remains within the base directory
5. Cap file sizes for read-back operations (100KB for confirmation previews)

### Confirmation Flow Pattern
When building tools that modify existing state:
1. Detect existing state before modification
2. Return existing state to the agent as context
3. Require explicit confirmation parameter on the second call
4. This pattern forces the agent to "see" what it's replacing

### Testing Patterns
- Test happy path (new file, overwrite with confirm)
- Test rejection (overwrite without confirm, explicit `confirm: false`)
- Test path security (traversal, absolute paths, symlinks on parent and target)
- Test edge cases (empty path, empty content, double dots in filenames)

## Cross-References

- **ADR-006:** Skills marketplace architecture (`docs/adr/006-skills-marketplace.md`)
- **Related solution:** [Shared path validation extraction](../code-review-patterns/) — DRY principle applied across file tools
- **Todo #425:** Future work to add `read_file` and `list_files` builtins for full home directory parity
- **Plan:** `docs/plans/2026-03-03-feat-write-file-tool-with-overwrite-confirmation-plan.md`

## Lessons Learned

1. **Defense-in-depth matters:** The initial implementation checked symlinks on parent directories but missed the target file. A symlink at the leaf node could escape the sandbox. Always check both.
2. **Shared helpers reduce bugs:** Extracting `validate_and_resolve_path()` from 3 tools eliminated duplicated security logic and ensures future file tools inherit the same protections.
3. **Soft guardrails complement hard enforcement:** The shell-exec system prompt instruction can't prevent shell-based overwrites at the tool level, but it sets behavioral expectations. Combined with the hard-enforced `write_agent_file` confirmation flow, this creates layered protection.
4. **Prompt documentation is agent-native parity:** Adding tool guidance to `prompt.rs` ensures the agent knows about `write_agent_file` and its confirmation semantics. Tools without prompt documentation are effectively invisible to the agent.
