---
title: "write_file tool reported success with wrong path due to missing home directory context"
date: 2026-03-03
category: logic-errors
tags:
  - path-resolution
  - user-feedback
  - agent-context
  - tools
  - write_file
  - prompt-engineering
severity: high
components:
  - crates/mika-agent/src/tools/write_file.rs
  - crates/mika-agent/src/tools/write_workspace.rs
  - crates/mika-agent/src/tools/read_workspace.rs
  - crates/mika-agent/src/prompt.rs
  - crates/mika-agent/src/agent.rs
related_prs:
  - "PR #65 (feat/write-file-tool)"
---

# write_file Tool Reported Success with Wrong Path Due to Missing Home Directory Context

## Problem Symptom

The agent called `write_file` to update `identity.toml` and received a success message, but the actual target file was unchanged. The agent had no way to detect the mistake.

**Observed in production (2026-03-03, conversation row 3025):**

```
Agent wanted:     ~/.mika/agents/main/identity.toml
Agent's home_dir: ~/.mika/agents/main/
Agent called:     write_file(path=".mika/agents/main/identity.toml")
Tool resolved:    ~/.mika/agents/main/.mika/agents/main/identity.toml  ← WRONG
Tool reported:    "Wrote 86 bytes to '.mika/agents/main/identity.toml'"  ← MISLEADING
```

**Proof on disk:**
- `~/.mika/agents/main/identity.toml` — old content (unchanged)
- `~/.mika/agents/main/.mika/agents/main/identity.toml` — wrongly-placed file with intended content

The overwrite confirmation flow was bypassed because no file existed at the wrong nested path.

## Root Cause

Two compounding issues:

1. **Success messages echoed relative paths, not resolved absolute paths.** The agent saw `"Wrote 86 bytes to '.mika/agents/main/identity.toml'"` and had no way to verify the actual write location. The tool used `{path}` (user input) instead of `full_path.display()` (resolved path) in format strings.

2. **The agent didn't know its home directory.** The system prompt said `"Paths are relative to your home"` but never told the agent what its home directory IS. The agent guessed `.mika/agents/main/` as a prefix — not knowing it was already inside that directory.

Neither issue alone would be sufficient — together they created a silent failure where the agent's mental model diverged from reality with no feedback mechanism to correct it.

## Investigation Steps

1. **User report:** Agent said it updated identity.toml but the file content was unchanged.
2. **Database inspection:** Queried `mika.db` conversation rows 3024-3027. Row 3025 metadata showed `write_file` called with path `.mika/agents/main/identity.toml`, reporting "Wrote 86 bytes".
3. **Log inspection:** `mika.log.2026-03-03` confirmed the tool execution and the relative path in the success message.
4. **Disk inspection:** Found the wrongly-placed file at `~/.mika/agents/main/.mika/agents/main/identity.toml` with the intended content.
5. **Code audit:** Traced `write_file.rs` — success message at line 120-121 used `{path}` (user input). All 5 message format strings had the same issue.
6. **Prompt audit:** `prompt.rs` line 309-311 — instruction said "relative to your home" but `PromptContext` had no `home_dir` field.

## Solution

### Change 1: Absolute path reporting in write_file.rs

Changed 5 message format strings from `{path}` (user-provided relative) to `full_path.display()` (resolved absolute):

**Before:**
```rust
Ok(ToolOutput::success(format!(
    "Wrote {bytes_written} bytes to '{path}'."
)))
```

**After:**
```rust
Ok(ToolOutput::success(format!(
    "Wrote {bytes_written} bytes to '{}'.",
    full_path.display()
)))
```

Applied to: success message, write failure error, and 3 confirmation/error messages for the overwrite flow.

### Change 2: Home directory in system prompt (prompt.rs + agent.rs)

Added `home_dir: Option<&'a Path>` to `PromptContext`. The system prompt now conditionally includes the absolute path:

```rust
if let Some(home) = ctx.home_dir {
    writeln!(
        prompt,
        "- You can write files to your home directory with write_file. \
         Your home directory is {} — all paths are relative to this directory. \
         For example, to write identity.toml at the root of your home, use path 'identity.toml'. \
         If the file exists, you must review the current content and call again with confirm: true to overwrite.",
        home.display()
    ).unwrap();
}
```

Updated both `PromptContext` construction sites in `agent.rs` to pass `home_dir: Some(params.home_dir)`.

### Change 3: Consistency in write_workspace.rs + read_workspace.rs

Applied the same absolute-path reporting pattern to all workspace file tools for consistency.

## Verification

5 new tests added:

| Test | File | Verifies |
|------|------|----------|
| `test_success_message_contains_absolute_path` | write_file.rs | Success message includes resolved absolute path |
| `test_confirmation_message_contains_absolute_path` | write_file.rs | Overwrite confirmation shows absolute path |
| `test_prompt_includes_home_dir_in_write_file_instruction` | prompt.rs | System prompt contains home directory when set |
| `test_prompt_fallback_when_home_dir_none` | prompt.rs | Falls back to generic instruction when home_dir is None |
| `test_write_success_message_contains_absolute_path` | write_workspace.rs | Workspace tool also reports absolute paths |

All 826 tests pass.

## Prevention Strategies

### For new file-system tools

- **Always use resolved paths in messages.** After calling `validate_and_resolve_path()`, use `full_path.display()` in all success/error messages — never echo the user's input path.
- **Test message content, not just I/O.** Verify that output messages contain the absolute path, not just that the file was created.
- **Surface implicit context in prompts.** Any tool that operates relative to a base directory must have that directory explicitly stated in the system prompt.

### Checklist for new tools

- [ ] Calls `validate_and_resolve_path()` for path security
- [ ] All messages use `full_path.display()` (resolved absolute path)
- [ ] System prompt discloses the base directory
- [ ] Tests verify message content includes absolute paths
- [ ] Tests verify both success and error paths

### Design principle

> Tools must report ground truth (resolved paths), AND the agent must understand ground truth (explicit directory in prompt). Neither alone is sufficient — together they create alignment between agent reasoning and actual system behavior.

## Related Documentation

- **Plan:** `docs/plans/2026-03-03-fix-write-file-silent-miswrite-plan.md` — detailed plan for this fix
- **Prior solution:** `docs/solutions/integration-issues/write-file-tool-overwrite-confirmation-flow.md` — original write_file implementation and overwrite flow
- **Prior plan:** `docs/plans/2026-03-03-feat-write-file-tool-with-overwrite-confirmation-plan.md` — original feature plan
- **Shared helper:** `crates/mika-agent/src/tools/mod.rs:196-277` — `validate_and_resolve_path()` implementation
