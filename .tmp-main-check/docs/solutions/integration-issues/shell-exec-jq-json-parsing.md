---
title: "Shell-exec handler silently truncates commands with double quotes"
category: integration-issues
severity: high
module: crates/mika-agent/src/skills
components:
  - skills/executor
  - templates/skills/shell-exec
  - templates/skills/tmux
tags:
  - json-parsing
  - shell-handler
  - jq
  - skill-system
date_resolved: 2026-03-02
pr: "#47"
branch: fix/shell-exec-jq-json-parsing
---

# Shell-exec handler silently truncates commands with double quotes

## Problem Symptom

Mika's `run_shell` tool failed to execute desktop automation commands containing
double quotes. The user asked Mika to create a Hyprland shortcut toggle and Waybar
indicator, but every command with quoted strings was silently truncated — the tool
returned empty or partial output with no error, causing Mika to retry the same
failing command in a loop until hitting the 10-step tool limit.

**Example failing command:**

```bash
echo "hello world"
```

**What the handler actually executed:** `echo ` (truncated at the first `"`)

## Investigation Steps

1. **Read conversation history** from SQLite (`conversations` table, row 2178) — confirmed
   repeated `run_shell` failures with identical commands containing double quotes.

2. **Traced the executor pipeline** in `crates/mika-agent/src/skills/executor.rs`:
   - `execute_exec()` serializes tool input via `serde_json::to_vec(&input)` and pipes
     JSON to the handler's stdin.
   - The handler script is responsible for parsing the JSON.

3. **Identified root cause** in `crates/mika-agent/templates/skills/shell-exec/handlers/run.sh`:
   - Line used `grep -o '"command":"[^"]*"' | cut -d'"' -f4` to extract the command field.
   - The regex `[^"]*` cannot match escaped `\"` inside JSON string values.
   - For input `{"command":"echo \"hello world\""}`, grep matched only up to the first
     escaped quote, producing `echo ` instead of `echo "hello world"`.

4. **Verified** that all tmux handler scripts had the same pattern — `grep -o` with `[^"]*`
   as a fallback when jq was unavailable.

## Root Cause

**Grep-based JSON parsing cannot handle escaped characters in string values.**

The shell handler used `grep -o '"command":"[^"]*"'` which is fundamentally broken for
any JSON value containing:
- Double quotes (escaped as `\"` in JSON)
- Backslashes (escaped as `\\`)
- Unicode escapes (`\uXXXX`)
- Newlines (escaped as `\n`)

This is not a bug that can be patched in grep — regex-based JSON parsing is inherently
unsafe. The correct tool is a proper JSON parser like `jq`.

## Working Solution

### 1. Replace grep with jq in shell-exec handler

**File:** `crates/mika-agent/templates/skills/shell-exec/handlers/run.sh`

```sh
#!/bin/sh
# Shell execution handler.
# Input: JSON on stdin with "command" and optional "working_dir" fields
# Output: command output on stdout, errors on stderr
#
# SECURITY: This handler executes arbitrary commands. Use responsibly.

command -v jq >/dev/null 2>&1 || { echo "Error: jq is required but not installed" >&2; exit 1; }

INPUT=$(cat)

# Scrub sensitive env vars so subprocesses cannot leak them
unset MIKA_LLM_API_KEY MIKA_INTERNAL_TOKEN MIKA_OPENAI_API_KEY MIKA_BRAVE_API_KEY

# Parse JSON fields
COMMAND=$(printf '%s\n' "$INPUT" | jq -r '.command // empty')
WORKDIR=$(printf '%s\n' "$INPUT" | jq -r '.working_dir // empty')

if [ -z "$COMMAND" ]; then
    echo "Error: no command provided" >&2
    exit 1
fi

if [ -n "$WORKDIR" ] && [ -d "$WORKDIR" ]; then
    cd "$WORKDIR" || exit 1
fi

eval "$COMMAND" 2>&1
```

### 2. Standardize all tmux handlers to jq-only

Removed grep fallbacks from all 5 tmux handler scripts (`create_session.sh`,
`send_command.sh`, `kill_session.sh`, `read_output.sh`, `wait_for_text.sh`).
Each now has a `jq` guard clause at the top and uses `jq -r` for all field extraction.

### 3. Add regression test

**File:** `crates/mika-agent/src/skills/executor.rs`

```rust
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_exec_handler_command_with_quotes() {
    let tmp = tempfile::tempdir().unwrap();
    let handler_dir = tmp.path().join("handlers");
    fs::create_dir_all(&handler_dir).unwrap();
    write_script(
        &handler_dir.join("run.sh"),
        include_str!("../../templates/skills/shell-exec/handlers/run.sh"),
    );

    let tool = make_exec_tool(tmp.path(), "handlers/run.sh");
    let input = serde_json::json!({"command": "echo \"hello world\""});
    let output = execute_skill_tool(&tool, input, 30).await;
    assert!(!output.is_error, "unexpected error: {}", output.content);
    assert!(
        output.content.contains("hello world"),
        "expected 'hello world' in output, got: {}",
        output.content
    );
}
```

### 4. Add environment variable scrubbing

All exec-handler scripts that run user-provided commands now `unset` sensitive
`MIKA_*` environment variables before executing, preventing accidental leakage
of API keys or tokens to subprocesses.

## Prevention Strategies

### Immediate

- **jq is now a hard dependency:** All bundled handlers fail fast with a clear error
  if jq is not installed, rather than falling back to broken grep parsing.
- **Regression test with `include_str!`:** Tests the actual bundled template, not a
  copy — so template changes are automatically covered.
- **Documentation updated:** `docs/getting-started.md` lists jq as a prerequisite;
  `docs/skills.md` documents the jq requirement for exec-handler skills.

### Future Considerations

- **Pre-parse JSON in Rust executor:** Instead of piping raw JSON to stdin, the
  executor could deserialize the input and pass individual fields as environment
  variables (`MIKA_TOOL_COMMAND`, `MIKA_TOOL_PATH`, etc.). This would eliminate
  JSON parsing bugs from shell scripts entirely.
- **Handler validation tool:** A static analysis pass that scans handler scripts
  for known-broken patterns (grep-based JSON extraction, unquoted variables,
  missing jq checks) and fails CI if found.
- **Test vector library:** A standard set of "dangerous inputs" (embedded quotes,
  backslashes, unicode, newlines) that every handler must pass.

## Cross-References

- **PR:** [#47](https://github.com/senara-solutions/mika/pull/47)
- **ADR:** [002 - Filesystem Skill Registry](../adr/002-filesystem-skill-registry.md)
- **Skills docs:** [docs/skills.md](../skills.md) (exec-handler security section)
- **Executor source:** `crates/mika-agent/src/skills/executor.rs` (execute_exec at ~line 229)
- **Bundled skills sync:** `crates/mika-agent/src/bundled_skills.rs` (seed_bundled_skills)
