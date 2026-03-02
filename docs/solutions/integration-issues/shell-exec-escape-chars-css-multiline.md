---
title: Shell-exec handler escape character handling — CSS and multiline commands
date: 2026-03-02
severity: high
module: mika-agent
component:
  - skills/executor
  - bundled handler scripts (shell-exec, file-reader, tmux, github)
tags:
  - shell-exec
  - json-parsing
  - handler-scripts
  - css-files
  - special-characters
  - regression-tests
  - stale-templates
related_prs:
  - 47
  - 48
  - 50
related_docs:
  - docs/solutions/integration-issues/shell-exec-jq-json-parsing.md
  - docs/solutions/integration-issues/shell-exec-config-editing-quality.md
---

# Shell-exec handler escape character handling — CSS and multiline commands

## Problem Statement

After PR #47 fixed the core grep-to-jq parsing issue, three residual problems remained:

1. **Stale root `templates/` directory** — 33 tracked files at the repo root duplicated the canonical templates at `crates/mika-agent/templates/`. Seven files had diverged after PR #47 (still using broken grep-based parsing), creating confusion about which templates were authoritative.

2. **Insufficient test coverage** — PR #47's regression test only covered escaped quotes (`echo "hello world"`). The actual failure scenario (CSS files with `#` selectors, hex colors, heredoc multiline commands, and printf backslash sequences) was not tested.

3. **Missing `jq` guard in file-reader handler** — All other handlers (shell-exec, tmux, github) had a `command -v jq` availability check, but `file-reader/handlers/read.sh` lacked it despite using jq on two lines.

## Background: The Original Failure

(Full root cause documented in [shell-exec-jq-json-parsing.md](./shell-exec-jq-json-parsing.md))

Mika attempted to edit `~/.config/waybar/style.css` to fix CSS selectors. The conversation (rows 2227-2230, main agent) showed cascading failures:

1. **sed with `#` patterns** — `sed -i 's/#claude-relay\\.enabled/#custom-claude-relay.enabled/'` returned exit code 2 and **emptied the CSS file entirely**
2. **Heredoc append** — `cat >> style.css << 'EOF'` with CSS content returned exit code 1 (heredoc broken by grep parser's inability to handle `\n` in JSON)
3. **printf with escape sequences** — `printf '\\n#custom-claude-relay...'` wrote literal `\n` characters instead of newlines
4. **Python workarounds** — All 5 python-based attempts failed due to the same underlying JSON parsing issue

Root cause: `grep -o '"command":"[^"]*"'` regex cannot handle JSON escape sequences — stops at escaped quotes `\"`, ignores `\n` newlines, and silently truncates multi-line values.

## Investigation

1. **Queried conversation data** from SQLite (rows 2227-2230) to see exact tool calls and failures
2. **Compared two template directories** — found root `templates/` diverged from canonical `crates/mika-agent/templates/` on 7 files
3. **Verified no build code references root `templates/`** — `bundled_skills.rs` uses relative paths from `crates/mika-agent/src/` that resolve to the crate-local templates
4. **Checked all handler scripts** for jq guard — file-reader was the only one missing it
5. **Confirmed CI has jq** — ubuntu-22.04 runners include jq; existing `test_exec_handler_command_with_quotes` already passes

## Solution (PR #50)

### 1. Removed stale root `templates/` directory

```bash
git rm -r templates/
```

33 files deleted. No code referenced this path — the canonical templates at `crates/mika-agent/templates/skills/` are used by `bundled_skills.rs` via `include_str!` relative paths.

### 2. Added jq guard to file-reader handler

```sh
# crates/mika-agent/templates/skills/file-reader/handlers/read.sh
command -v jq >/dev/null 2>&1 || { echo "Error: jq is required but not installed" >&2; exit 1; }
```

Consistent with the pattern in all other handlers.

### 3. Added 4 regression tests

All tests use `include_str!` to test the ACTUAL bundled handler script, not a test copy:

```rust
// CSS hash characters: selectors and hex colors
let input = serde_json::json!({"command": "echo '#custom-relay { color: #a6e3a1; }'"});

// Heredoc multiline: \n in JSON → real newlines → eval executes heredoc
let input = serde_json::json!({"command": "cat << 'EOF'\n#selector { color: #fff; }\nEOF"});

// sed with # in pattern: sed substitution on CSS selectors
let cmd = format!("sed 's/#old-selector/#new-selector/' '{}'", css_file.display());

// printf with backslash sequences: \n survives JSON → jq → eval → printf
let input = serde_json::json!({"command": "printf 'line1\\nline2\\n'"});
```

Also extracted `setup_shell_exec_handler()` helper to reduce duplication across tests.

## Prevention Strategies

1. **Regression tests** — 5 tests now cover the full JSON → jq → eval pipeline with various special characters. Future handler changes will be caught.

2. **Single source of truth** — Removing the stale root `templates/` eliminates confusion about which templates are canonical. All templates live in `crates/mika-agent/templates/skills/`.

3. **jq as hard dependency** — All handlers now have the `command -v jq` guard. Missing jq produces a clear error instead of cryptic grep failures.

4. **`include_str!` test pattern** — Tests embed the actual handler script via `include_str!`, so any template changes are automatically tested against all escape character scenarios.

5. **Potential CI check** — A grep guard in CI could prevent reintroduction:
   ```yaml
   - name: Check for grep-based JSON parsing in handlers
     run: |
       if grep -r 'grep -o.*"command"' crates/mika-agent/templates/; then
         echo "ERROR: Found grep-based JSON parsing"
         exit 1
       fi
   ```

## Key Insight

The `#` character in CSS is not special in JSON — it serializes verbatim. The actual problem was never about `#` specifically, but about the grep regex `[^"]*` failing on escaped quotes and multi-line values in the JSON payload. Once jq properly extracts the command string, `eval "$COMMAND"` handles `#` correctly (it's only a comment when unquoted at word start, which doesn't happen inside properly quoted shell commands).

## References

- PR #47: Core grep→jq fix — [shell-exec-jq-json-parsing.md](./shell-exec-jq-json-parsing.md)
- PR #48: Config editing quality — [shell-exec-config-editing-quality.md](./shell-exec-config-editing-quality.md)
- PR #50: This cleanup — stale templates, regression tests, jq guard
- Executor code: `crates/mika-agent/src/skills/executor.rs:229-329`
- Handler template: `crates/mika-agent/templates/skills/shell-exec/handlers/run.sh`
- Bundled skills: `crates/mika-agent/src/bundled_skills.rs:64-69`
