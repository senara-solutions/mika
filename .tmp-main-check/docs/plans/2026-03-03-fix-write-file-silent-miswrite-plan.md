---
title: "fix: write_agent_file silently writes to wrong path when agent uses incorrect relative path"
type: fix
status: completed
date: 2026-03-03
---

# fix: write_agent_file silently writes to wrong path when agent uses incorrect relative path

## Overview

The `write_agent_file` tool silently writes files to wrong locations when the agent guesses an incorrect relative path. The tool reports success with the user-provided relative path, making it impossible for the agent to detect the mistake. The agent has no way to know its own home directory absolute path.

## Problem Statement

**Observed in production** (conversation row 3025, 2026-03-03):

1. Agent wanted to write `~/.mika/agents/main/identity.toml`
2. Agent's `home_dir` (base for `write_agent_file`) is `~/.mika/agents/main/`
3. Agent called `write_agent_file` with path `.mika/agents/main/identity.toml` — thinking its home was `~/` or `~/.mika/`
4. Tool resolved: `~/.mika/agents/main/` + `.mika/agents/main/identity.toml` = `~/.mika/agents/main/.mika/agents/main/identity.toml`
5. That path didn't exist → **overwrite confirmation was bypassed** (no existing file at wrong path)
6. Tool wrote 86 bytes and reported: `"Wrote 86 bytes to '.mika/agents/main/identity.toml'."`
7. Agent believed the write succeeded. **The actual file was unchanged.**

**Proof:** Both files still exist on disk:
- `~/.mika/agents/main/identity.toml` — old content (no reflection config)
- `~/.mika/agents/main/.mika/agents/main/identity.toml` — the wrongly-placed file with the intended content

**Two root causes:**

1. **Success message echoes the relative path, not the resolved absolute path.** The agent sees `"Wrote 86 bytes to '.mika/agents/main/identity.toml'"` and has no way to verify the actual write location. (`write_agent_file.rs:120-121`)

2. **The agent doesn't know its home directory.** The system prompt says `"Paths are relative to your home"` (`prompt.rs:310`) but never tells the agent what its home directory IS. The agent has to guess — and in this case guessed wrong.

## Proposed Solution

Two focused changes, both minimal and directly addressing the root causes:

### Change 1: Report resolved absolute path in write_agent_file messages

In `write_agent_file.rs`, change all user-facing messages to show the resolved absolute path instead of (or in addition to) the relative path. This gives the agent immediate feedback to verify correctness.

**Files:** `crates/mika-agent/src/tools/write_agent_file.rs`

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

Apply the same pattern to ALL messages in write_agent_file that reference the path:
- Success: line 120-121
- Error (write failure): line 123
- Confirmation prompt (file exists): lines 88, 96, 103

### Change 2: Add home_dir to PromptContext and surface it in the system prompt

Add the per-agent `home_dir` to `PromptContext` and include it in the Tool Usage section of the system prompt, so the agent knows its base directory.

**Files:** `crates/mika-agent/src/prompt.rs`, `crates/mika-agent/src/agent.rs`

**prompt.rs — PromptContext struct:** Add `home_dir: Option<&'a Path>` field.

**prompt.rs — write_agent_file instruction (line 309-311):** Change from:
```
"- You can write files to your home directory with write_agent_file. Paths are relative to your home. ..."
```
To (when home_dir is Some):
```
"- You can write files to your home directory with write_agent_file. Your home directory is /home/user/.mika/agents/main/ — all paths are relative to this directory. For example, to write identity.toml at the root of your home, use path 'identity.toml'. If the file exists, you must review the current content and call again with confirm: true to overwrite."
```

**agent.rs — PromptContext construction:** Pass `home_dir: Some(params.home_dir)` at both construction sites (conversation mode ~line 577, team agent ~line 1350).

### Change 3 (consistency): Apply same absolute-path reporting to write_workspace

For consistency and to prevent the same class of bug, update `write_workspace.rs` success/error messages to also include the resolved absolute path.

**Files:** `crates/mika-agent/src/tools/write_workspace.rs`

## Acceptance Criteria

- [x] `write_agent_file` success message shows the resolved absolute path (e.g., `Wrote 86 bytes to '/home/user/.mika/agents/main/identity.toml'.`)
- [x] `write_agent_file` confirmation/error messages show the resolved absolute path
- [x] System prompt includes the agent's home directory absolute path in the write_agent_file instruction
- [x] `write_workspace` success/error messages show the resolved absolute path
- [x] All existing tests pass (`cargo test`)
- [x] New test: verify write_agent_file success message contains the absolute home path, not just the relative input path
- [x] New test: verify system prompt contains the home_dir path when `home_dir` is `Some`

## Context

- PR #65 introduced the `write_agent_file` tool (merged 2026-03-03)
- The overwrite confirmation flow works correctly for files that DO exist at the resolved path
- The bug is specifically about the agent using wrong relative paths — the tool doesn't help the agent detect this
- Similar file: `crates/mika-agent/src/tools/write_workspace.rs:63` — same echo-relative-path pattern
- Similar file: `crates/mika-agent/src/tools/read_workspace.rs` — should be checked for consistency

## Sources

- Conversation evidence: `~/.mika/agents/main/data/mika.db`, rows 3024-3027
- Log evidence: `~/.mika/agents/main/logs/mika.log.2026-03-03`, line 1369
- Wrong file on disk: `~/.mika/agents/main/.mika/agents/main/identity.toml`
- `write_agent_file.rs:120-121` — success message format
- `prompt.rs:309-311` — write_agent_file instruction in system prompt
- `agent.rs:577-587` — PromptContext construction
