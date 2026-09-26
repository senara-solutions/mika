---
title: "fix: Use builtin file tools instead of run_shell for config changes"
type: fix
status: completed
date: 2026-03-26
issue: 284
---

# fix: Use builtin file tools instead of run_shell for config changes

## Overview

Agent bypasses `read_agent_file`/`write_agent_file` builtins by using `run_shell cat` and `run_shell cat >` heredocs for file I/O on agent config files. This loses audit logging, overwrite confirmation flow, and path validation. Observed in trace `55be39bc`: 7 Opus LLM calls, 178K input tokens, 0% cache efficiency for a task that should take 2-3 steps.

Two root causes: (1) prompt instructions say "prefer" builtins but don't prohibit shell file I/O, and (2) file tools are scoped to the current agent's home directory — orchestrators can't access other agents' files through builtins, forcing `run_shell` for cross-agent config changes.

## Proposed Solution

### Part A: Add cross-agent file access to builtin file tools

Add an optional `agent` parameter to `read_agent_file`, `write_agent_file`, and `list_agent_files`. When provided, resolve the target agent's home directory and use it as the base path. Guarded by orchestrator-only check.

### Part B: Strengthen prompt instructions

Change "prefer builtins" to "NEVER use run_shell for agent home directory files" with BAD/GOOD examples following the four-part anti-hallucination formula.

## Technical Approach

### Step 1: Add `global_home_dir` to `ToolContext`

**File:** `crates/mika-agent/src/tools/mod.rs`

Add `global_home_dir: Option<&'a Path>` to the `ToolContext` struct (after `home_dir`).

**File:** `crates/mika-agent/src/agent.rs`

Thread `global_home_dir` through the three `ToolContext` construction sites:

| Mode | Line | Value |
|------|------|-------|
| Conversation (`run_agent`) | ~1094 | `params.global_home_dir` |
| Silent (`run_silent_agent`) | ~1837 | `Some(params.home_dir)` if available, or derive from `SilentAgentParams` |
| Team (`run_team_agent`) | ~2074 | `None` (team agents must NOT get cross-agent access) |

For delegate agents (in `delegate_task.rs` ~line 143), set `global_home_dir: None` on the delegate's `ToolContext`.

**File:** `crates/mika-agent/src/test_utils.rs`

Add `ctx_with_home_and_global(&self, home: &Path, global: &Path) -> ToolContext` helper to `TestHarness`. Update existing `ctx()` and `ctx_with_home()` to set `global_home_dir: None`.

### Step 2: Create shared `resolve_agent_home` helper

**File:** `crates/mika-agent/src/tools/mod.rs`

Add a helper function used by all three file tools:

```rust
/// Resolve the base directory for file operations, optionally targeting another agent.
/// Returns Ok(base_dir) or Err(ToolOutput) with a descriptive error.
pub(crate) async fn resolve_agent_home<'a>(
    agent_param: Option<&str>,
    ctx: &'a ToolContext<'a>,
) -> Result<&'a Path, ToolOutput> {
    let agent_name = match agent_param {
        None | Some("") => return Ok(ctx.home_dir),
        Some(name) => name.trim(),
    };

    // Require global_home_dir for cross-agent access
    let global_home = match ctx.global_home_dir {
        Some(home) => home,
        None => return Err(ToolOutput::error(
            "Cross-agent file access is not available in this context."
        )),
    };

    // Check if targeting self (short-circuit)
    let current_agent = ctx.db.agent_id().await;
    if agent_name == current_agent {
        return Ok(ctx.home_dir);
    }

    // Orchestrator guard
    if !is_orchestrator(global_home, &current_agent) {
        return Err(ToolOutput::error(
            "Only orchestrator agents can access other agents' files."
        ));
    }

    // Validate agent exists (doubles as path traversal protection)
    if !agent::agent_exists(global_home, agent_name) {
        let agents = agent::list_agents(global_home);
        return Err(ToolOutput::error(format!(
            "Agent '{}' not found. Available agents: {}",
            agent_name,
            agents.join(", ")
        )));
    }

    // Return owned path — caller must handle lifetime
    // Actually, return the resolved PathBuf
    Ok(/* agent::agent_dir(global_home, agent_name) */)
}
```

Note: Since `agent_dir` returns a `PathBuf` (owned), the helper should return `Result<PathBuf, ToolOutput>` with `ctx.home_dir.to_path_buf()` for the self-path case. This avoids lifetime issues.

### Step 3: Update `read_agent_file`

**File:** `crates/mika-agent/src/tools/read_agent_file.rs`

1. Add `agent` to `input_schema` properties:
   ```json
   "agent": {
     "type": "string",
     "description": "Target agent name (orchestrator-only). Omit to use your own home directory."
   }
   ```
2. In `execute()`, extract `agent` param and call `resolve_agent_home()` to get the base dir
3. Pass the resolved base dir to `validate_and_resolve_path()` instead of `ctx.home_dir`

### Step 4: Update `write_agent_file`

**File:** `crates/mika-agent/src/tools/write_agent_file.rs`

1. Add `agent` to `input_schema` properties (same description as read)
2. In `execute()`, resolve base dir via `resolve_agent_home()`
3. Pass resolved base dir to `validate_and_resolve_path()`
4. **Critical:** Update confirmation message to include agent name when doing cross-agent writes:
   ```
   "Call again with confirm: true and agent: \"{name}\" to overwrite."
   ```
   This prevents the agent from accidentally writing to its own home on the retry call.
5. Update the absolute-path hint (line 66-71) to mention the `agent` parameter as an alternative:
   ```
   "For other agents' files, use the `agent` parameter. For paths outside any agent's home directory, use run_shell."
   ```

### Step 5: Update `list_agent_files`

**File:** `crates/mika-agent/src/tools/list_agent_files.rs`

1. Add `agent` to `input_schema` properties (same description)
2. In `execute()`, resolve base dir via `resolve_agent_home()`
3. Use resolved base dir instead of `ctx.home_dir`

### Step 6: Update prompt instructions

**File:** `crates/mika-agent/src/prompt.rs`

In the Tool Usage section (after the file tool instructions, ~line 464), add:

```
- NEVER use run_shell (cat, echo, heredoc, tee, sed) to read or write files inside
  your home directory or any agent's home directory. Always use read_agent_file,
  write_agent_file, and list_agent_files instead — they provide audit logging,
  overwrite confirmation, and path validation that shell commands bypass.
  For other agents' files, pass the agent parameter (e.g., read_agent_file with
  agent="chase-hughes"). Only use run_shell for paths outside agent home directories
  (e.g., ~/.local/bin/, ~/.config/).
```

When orchestrator mode is active (i.e., multiple agents exist), add to the file tool instructions:

```
- As an orchestrator, you can read, write, and list files in other agents' home
  directories by passing the `agent` parameter to file tools. For example:
  read_agent_file(path="config.toml", agent="chase-hughes") reads chase-hughes's config.
```

**File:** `crates/mika-agent/templates/skills/shell-exec/system_prompt.md`

Replace the current file writing section (lines 17-22) with:

```markdown
## File writing

NEVER use shell commands (cat >, tee, sed -i, echo >, heredoc) to read or write files inside any agent's home directory. Use the builtin file tools instead:

- `read_agent_file` — read files (with optional `agent` parameter for other agents)
- `write_agent_file` — write files with overwrite confirmation (with optional `agent` parameter)
- `list_agent_files` — list directory contents (with optional `agent` parameter)

**Bad example:** `run_shell("cat ~/.mika/agents/chase-hughes/config.toml")` — bypasses audit logging.
**Good example:** `read_agent_file(path="config.toml", agent="chase-hughes")` — audited, path-validated.

Shell writes are ONLY appropriate for paths outside agent home directories (e.g., `~/.local/bin/`, `~/.config/`). When shell writes are necessary for non-agent paths, use `cat <file>` or `head <file>` first.
```

**File:** `crates/mika-agent/templates/skills/self-knowledge/system_prompt.md`

Add after the "Rules for home directory questions" section:

```markdown
**Rules for config changes across agents:**
1. When asked to change configuration for multiple agents, use `read_agent_file` and `write_agent_file` with the `agent` parameter for each target agent.
2. Only read what you need — if changing config.toml, do NOT read identity.toml.
3. Do NOT use `run_shell` to read or write agent config files.
```

### Step 7: Tests

Add tests to each file tool module:

**`read_agent_file.rs` tests:**
- `test_cross_agent_read` — orchestrator reads another agent's file
- `test_cross_agent_read_blocked_non_orchestrator` — non-orchestrator gets error
- `test_cross_agent_read_agent_not_found` — invalid agent name with listing
- `test_cross_agent_read_global_home_none` — error when global_home_dir is None
- `test_cross_agent_read_self_reference` — passing own agent name works

**`write_agent_file.rs` tests:**
- `test_cross_agent_write_confirmation_includes_agent` — confirmation message includes agent name
- `test_cross_agent_write_with_confirm` — full overwrite flow works
- `test_cross_agent_write_blocked_non_orchestrator` — non-orchestrator gets error

**`list_agent_files.rs` tests:**
- `test_cross_agent_list` — orchestrator lists another agent's files
- `test_cross_agent_list_blocked_non_orchestrator` — non-orchestrator gets error

**`mod.rs` tests:**
- `test_resolve_agent_home_none_agent` — returns ctx.home_dir
- `test_resolve_agent_home_self_reference` — short-circuits to ctx.home_dir
- `test_resolve_agent_home_valid_agent` — returns target agent's home
- `test_resolve_agent_home_invalid_agent` — error with agent list

## Acceptance Criteria

- [x] `read_agent_file`, `write_agent_file`, `list_agent_files` accept optional `agent` parameter
- [x] Orchestrator agents can read/write/list files in other agents' home directories
- [x] Non-orchestrator agents get a clear error when using the `agent` parameter
- [x] Team agents and delegate agents cannot use cross-agent file access (`global_home_dir: None`)
- [x] `write_agent_file` confirmation message includes agent name for cross-agent writes
- [x] Path traversal in agent name is blocked (validated against `agent::agent_exists`)
- [x] Prompt prohibits `run_shell` for agent home directory files with BAD/GOOD examples
- [x] Shell-exec system prompt updated to NEVER allow shell file I/O for agent homes
- [x] Self-knowledge prompt updated with cross-agent config change guidance
- [x] All existing tests pass
- [x] New tests cover cross-agent access, guard rejections, and edge cases
- [x] `cargo clippy` passes
- [x] `cargo test` passes

## Sources

- Issue: senara-solutions/mika#284
- Related solution: `docs/solutions/integration-issues/shell-exec-config-editing-quality.md`
- Related solution: `docs/solutions/integration-issues/write-file-tool-overwrite-confirmation-flow.md`
- Related solution: `docs/solutions/logic-errors/tool-path-reporting-misbehavior.md`
- Related solution: `docs/solutions/prompt-engineering/grounding-rule-downstream-state-hallucination.md` (four-part prompt formula)
