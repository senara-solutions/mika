---
title: "fix: Remove stale templates and add escape character regression tests"
type: fix
status: completed
date: 2026-03-02
---

# fix: Remove stale templates and add escape character regression tests

## Overview

Mika's shell-exec handler failed to update CSS files containing `#` characters (selectors like `#custom-claude-relay` and hex colors like `#a6e3a1`), escaped quotes, and multi-line heredoc commands. PR #47 fixed the root cause (grep-based JSON parsing → jq), but left behind a stale copy of ALL template files at the repo root `templates/` that still contains the broken code. Additionally, the regression test coverage only covers escaped quotes — not CSS content, heredocs, or multi-line commands.

## Problem Statement

**Observed failure** (conversation row IDs 2228-2230, main agent):
1. `sed -i 's/#claude-relay\\.enabled/#custom-claude-relay.enabled/'` — exit code 2, **emptied the CSS file**
2. `cat >> style.css << 'EOF'` with CSS content — exit code 1 (heredoc broken by grep parser)
3. `printf '\\n#custom-claude-relay...'` — wrote literal `\n` instead of newlines
4. Multiple python workaround attempts — all failed

**Root cause** (fixed by PR #47): `grep -o '"command":"[^"]*"'` regex stops at `\"` and cannot parse multi-line JSON values. Replaced with `jq -r '.command // empty'`.

**Remaining issues:**
1. **Stale root `templates/` directory** — 33 tracked files, 7 divergent from canonical `crates/mika-agent/templates/`. Not referenced by any build code but creates confusion.
2. **Insufficient test coverage** — only `echo "hello world"` tested; no CSS `#`, heredoc, or multi-line tests.
3. **Missing `jq` guard** in `file-reader/handlers/read.sh` — inconsistent with the pattern established by PR #47.

## Proposed Solution

Three commits in one PR:

### Commit 1: Remove stale root `templates/` directory

Delete the entire root `templates/skills/` directory. Verification:
- No `*.rs`, `*.toml`, `Dockerfile*`, `*.yml`, `*.sh` files reference this path
- `bundled_skills.rs` uses relative `"../templates/skills/..."` paths that resolve to `crates/mika-agent/templates/skills/`
- `CLAUDE.md` does not reference the root `templates/` path

### Commit 2: Add `jq` guard to `file-reader/handlers/read.sh`

Add the same `command -v jq` guard pattern used in all other handlers:

**File:** `crates/mika-agent/templates/skills/file-reader/handlers/read.sh`

```sh
#!/bin/sh
# ...existing comments...

command -v jq >/dev/null 2>&1 || { echo "Error: jq is required but not installed" >&2; exit 1; }

INPUT=$(cat)
# ...rest unchanged...
```

### Commit 3: Add comprehensive escape character regression tests

**File:** `crates/mika-agent/src/skills/executor.rs` (test module)

Add tests using `include_str!` to test the ACTUAL deployed handler:

1. **`test_exec_handler_css_hash_chars`** — Command: `echo '#custom-relay { color: #a6e3a1; }'`
   - Verifies `#` in CSS selectors and hex colors survives JSON → jq → eval pipeline

2. **`test_exec_handler_heredoc_multiline`** — Command with embedded newlines simulating a heredoc:
   ```
   cat << 'EOF'
   #selector { color: #fff; }
   EOF
   ```
   - Verifies `\n` in JSON is decoded by jq to real newlines, and `eval` executes the heredoc

3. **`test_exec_handler_sed_with_hash`** — Command: create a temp file, then `sed -i 's/#old/#new/' file`
   - Verifies sed patterns with `#` work through the pipeline

4. **`test_exec_handler_backslash_in_printf`** — Command: `printf 'line1\nline2\n'`
   - Verifies backslash sequences survive the JSON → jq → eval → printf chain

## Acceptance Criteria

- [x] Root `templates/` directory removed from git tracking
- [x] `file-reader/handlers/read.sh` has `jq` availability guard
- [x] All 4 new regression tests pass locally and in CI
- [x] Existing `test_exec_handler_command_with_quotes` test still passes
- [x] `cargo clippy` clean
- [x] `cargo fmt` clean

## Technical Considerations

- **CI environment**: ubuntu-22.04 has `jq` pre-installed; existing `test_exec_handler_command_with_quotes` already uses `include_str!` with the real handler, confirming CI compatibility
- **No architecture changes**: The actual fix (grep → jq) was already shipped in PR #47. This PR is cleanup and test hardening.
- **`eval` behavior is intentional**: Characters like `#`, `$`, `{}` have shell meaning when unquoted in `eval`. This is expected — the LLM must generate properly quoted shell commands. Tests verify the JSON→jq→eval pipeline works correctly when commands are properly quoted.

## References

- PR #47: `fix/shell-exec-jq-json-parsing` (merged) — Core grep→jq fix
- PR #48: `feat/shell-exec-config-editing-quality` (merged) — System prompt improvements
- Solution doc: `docs/solutions/integration-issues/shell-exec-jq-json-parsing.md`
- Executor code: `crates/mika-agent/src/skills/executor.rs:229-329`
- Handler template: `crates/mika-agent/templates/skills/shell-exec/handlers/run.sh`
- Bundled skills: `crates/mika-agent/src/bundled_skills.rs:64-69`
