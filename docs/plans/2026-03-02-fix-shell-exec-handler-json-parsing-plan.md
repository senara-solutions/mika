---
title: "fix: Shell-exec handler breaks on commands with quotes/newlines"
type: fix
status: completed
date: 2026-03-02
---

# fix: Shell-exec handler breaks on commands with quotes/newlines

## Overview

The `run_shell` skill handler uses naive `grep`-based JSON parsing that silently truncates commands containing double quotes, newlines, or backslashes. This makes it impossible for Mika to write shell scripts, use heredocs, or execute any command containing `"` characters through the `run_shell` tool.

## Problem Statement

### Root Cause

`crates/mika-agent/templates/skills/shell-exec/handlers/run.sh` extracts the `command` field from JSON input using:

```sh
COMMAND=$(echo "$INPUT" | grep -o '"command":"[^"]*"' | head -1 | cut -d'"' -f4)
```

The regex `[^"]*` **cannot match any content containing double quotes**. When the Rust executor serializes a command like `cat > file << 'EOF'\nSTATE="$HOME/..."` to JSON, the embedded `"` characters become `\"` in the JSON string. The grep pattern stops at the first escaped quote, truncating the command.

### Observed Failure

In conversation #2178, Mika attempted to write `~/.local/bin/claude-relay-toggle` via `run_shell`:

1. **Heredoc attempt** (`cat > file << 'EOF'...`): Command contained `STATE="$HOME/..."` with double quotes — grep truncated the command, producing a malformed heredoc that failed
2. **Python fallback** (`python3 -c "..."`) — also contains double quotes — same failure
3. **printf fallback** (`printf '%s\n' '...'`) — failed with exit code 2
4. **tmux_send_command workaround** — worked because tmux bypasses JSON parsing, but consumed 10 tool steps and still didn't finish all tasks

### Scope of the Bug

- **`shell-exec/handlers/run.sh`** — uses the broken grep pattern as its **only** parsing method (no jq, no fallback)
- **`tmux/handlers/*.sh`** — use the broken grep pattern as a **fallback** when `jq` is unavailable (less severe but still buggy)
- **`github/handlers/run.sh`** and **`file-reader/handlers/read.sh`** — already use `jq` correctly

### Impact

Any shell command containing double quotes fails silently. This is a very common pattern:
- Variable assignments: `VAR="value"`
- Heredocs with quoted content
- `jq` expressions: `jq '.field'`
- `sed` expressions: `sed 's/"old"/"new"/'`
- Any string containing paths, messages, or structured data

## Proposed Solution

Replace the naive grep-based JSON parsing in `run.sh` with `jq`, following the pattern already established by `github/handlers/run.sh` and `file-reader/handlers/read.sh`. Add a graceful grep fallback for environments without `jq` (matching the `tmux/handlers/create_session.sh` pattern).

## Acceptance Criteria

- [x] `run.sh` uses `jq` for JSON field extraction when available
- [x] `run.sh` falls back to grep when `jq` is unavailable (with a warning)
- [x] Commands containing double quotes execute correctly (e.g., `echo "hello world"`)
- [x] Commands containing heredocs with quoted content execute correctly
- [x] Commands containing newlines (via JSON `\n`) execute correctly
- [x] The `working_dir` field is also extracted via `jq`
- [x] Existing tests pass
- [x] New test covers commands with embedded quotes
- [x] The deployed copy at `~/.mika/agents/main/skills/shell-exec/handlers/run.sh` is updated on next Mika startup (via bundled skill sync)

## MVP

### `crates/mika-agent/templates/skills/shell-exec/handlers/run.sh`

```sh
#!/bin/sh
# Shell execution handler.
# Input: JSON on stdin with "command" and optional "working_dir" fields
# Output: command output on stdout, errors on stderr
#
# SECURITY: This handler executes arbitrary commands. Use responsibly.

INPUT=$(cat)

# Parse JSON fields (jq preferred, grep/cut fallback)
if command -v jq >/dev/null 2>&1; then
    COMMAND=$(printf '%s\n' "$INPUT" | jq -r '.command // empty')
    WORKDIR=$(printf '%s\n' "$INPUT" | jq -r '.working_dir // empty')
else
    # Fallback: grep-based extraction (cannot handle embedded quotes)
    COMMAND=$(printf '%s\n' "$INPUT" | grep -o '"command":"[^"]*"' | head -1 | cut -d'"' -f4)
    WORKDIR=$(printf '%s\n' "$INPUT" | grep -o '"working_dir":"[^"]*"' | head -1 | cut -d'"' -f4)
fi

if [ -z "$COMMAND" ]; then
    echo "Error: no command provided" >&2
    exit 1
fi

if [ -n "$WORKDIR" ] && [ -d "$WORKDIR" ]; then
    cd "$WORKDIR" || exit 1
fi

eval "$COMMAND" 2>&1
```

### `crates/mika-agent/src/skills/executor.rs` — new test

```rust
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_exec_handler_command_with_quotes() {
    let tmp = tempfile::tempdir().unwrap();
    write_script(
        &tmp.path().join("handler.sh"),
        "#!/bin/sh\nINPUT=$(cat)\nCOMMAND=$(printf '%s\\n' \"$INPUT\" | jq -r '.command // empty')\neval \"$COMMAND\" 2>&1",
    );

    let tool = make_exec_tool(tmp.path(), "handler.sh");
    let input = serde_json::json!({"command": "echo \"hello world\""});
    let output = execute_skill_tool(&tool, input, 30).await;
    assert!(!output.is_error, "unexpected error: {}", output.content);
    assert!(output.content.contains("hello world"));
}
```

## References

- Broken handler: `crates/mika-agent/templates/skills/shell-exec/handlers/run.sh:10`
- Correct pattern (github): `crates/mika-agent/templates/skills/github/handlers/run.sh:18`
- Correct pattern (file-reader): `crates/mika-agent/templates/skills/file-reader/handlers/read.sh:7`
- Correct pattern with fallback (tmux): `crates/mika-agent/templates/skills/tmux/handlers/create_session.sh:14-22`
- Executor (pipes JSON to stdin): `crates/mika-agent/src/skills/executor.rs:258-260`
- Bundled skill sync: `crates/mika-agent/src/bundled_skills.rs:136`
- Conversation showing the failure: SQLite row IDs 2178-2185 in `~/.mika/agents/main/data/mika.db`
