---
title: "Fix build-mika Custom Skill Format"
type: fix
status: completed
date: 2026-03-11
---

# Fix build-mika Custom Skill Format

## Overview

The custom `build-mika` skill at `~/.mika/agents/mika-dev/skills/build-mika/` is completely broken — Mika silently skips it at startup due to multiple format/convention violations. All four skill files need fixes.

## Problem Statement

The skill was authored using an incorrect format that doesn't match the skill system's expectations. Specifically:

1. **skill.toml** uses a flat/legacy format instead of the required `[skill]` section format
2. **tools.json** is missing the required `handler` field on each tool definition
3. **build.sh** doesn't follow exec handler conventions (stdin consumption, callback delivery, env scrubbing)
4. **system_prompt.md** is adequate but could be improved

The result: `scan_skills_dir()` either fails to parse the manifest or fails to deserialize tools.json, logging a `warn!` and silently skipping the skill. The agent never sees the `build_mika` tool.

## Root Cause Analysis

The skill was likely authored based on an older or imagined format rather than the actual conventions used by bundled skills. The key misunderstandings:

| File | What's Wrong | What's Expected |
|------|-------------|-----------------|
| `skill.toml` | Fields at root + `[handler]` + `[options]` sections | `[skill]` section with metadata + `[triggers]` section for keywords |
| `tools.json` | Tool defs have no `handler` field; `long_running` in wrong file | Each tool needs `handler: {type, command, long_running, estimated_duration_secs}` |
| `build.sh` | Doesn't read stdin, no callback, no env scrubbing | Must read JSON from stdin (has `__mika_task_id`), call `mika ask --task-id`, scrub `MIKA_*` vars |

## Proposed Solution

Fix all four files to match the conventions established by bundled skills (shell-exec, file-reader, github).

## Acceptance Criteria

- [x] `skill.toml` uses `[skill]` section format — `skill.toml`
- [x] `tools.json` includes `handler` object with `type: "exec"`, `command`, `long_running: true`, `estimated_duration_secs` — `tools.json`
- [x] `build.sh` reads JSON from stdin via `cat` + `jq` — `build.sh`
- [x] `build.sh` parses `__mika_task_id` and `__mika_agent` from stdin JSON — `build.sh`
- [x] `build.sh` scrubs `MIKA_*` env vars before running cargo — `build.sh`
- [x] `build.sh` checks `jq` and `mika` are in PATH — `build.sh`
- [x] `build.sh` captures build output and delivers via `mika ask --task-id <id> --agent <name>` — `build.sh`
- [x] `build.sh` delivers both success and failure results via callback (not just exit non-zero) — `build.sh`
- [x] `build.sh` exits non-zero only if `mika ask` itself fails — `build.sh`
- [x] `system_prompt.md` updated with callback behavior guidance — `system_prompt.md`
- [ ] Skill loads successfully at Mika startup (no warn! in logs)
- [ ] `build_mika` tool appears in agent's tool list
- [ ] End-to-end: user says "build mika" → agent calls tool → callback returns build result

## MVP

### skill.toml

```toml
[skill]
name = "build-mika"
description = "Build the mika project with cargo --release --features telemetry"
version = "0.1.0"
always_on = false
timeout_secs = 300

[triggers]
keywords = ["build mika", "build the project", "cargo build", "compile mika"]
```

Key changes:
- Added `[skill]` section (required by `SkillManifest` parser)
- Moved `always_on` into `[skill]` (was under `[options]`)
- Removed `[handler]` section (handler config belongs in `tools.json`)
- Removed `[options]` section (not part of the schema)
- Added `timeout_secs = 300` for release builds
- Added `version`

### tools.json

```json
[
  {
    "name": "build_mika",
    "description": "Build the mika project with `cargo build --release --features telemetry`. Long-running — returns a callback task ID immediately.",
    "input_schema": {
      "type": "object",
      "properties": {},
      "required": []
    },
    "handler": {
      "type": "exec",
      "command": "build.sh",
      "long_running": true,
      "estimated_duration_secs": 120
    }
  }
]
```

Key changes:
- Added `handler` object (required by `SkillToolDef` deserialization)
- `long_running` and `estimated_duration_secs` moved here from skill.toml
- `command` is `"build.sh"` (resolved relative to skill_dir by executor)

### build.sh

```sh
#!/bin/sh
# Build handler for the build-mika skill.
# Input: JSON on stdin with __mika_task_id and __mika_agent (injected by executor for long-running)
# Output: Delivers result via `mika ask --task-id` callback
#
# This is a long-running handler: stdout goes to /dev/null, stderr is captured on failure.
# All meaningful output must be delivered via the callback mechanism.

# Dependency checks
command -v jq >/dev/null 2>&1 || { echo "Error: jq is required but not installed" >&2; exit 1; }
command -v mika >/dev/null 2>&1 || { echo "Error: mika CLI is required but not in PATH" >&2; exit 1; }

# Read input JSON from stdin
INPUT=$(cat)

# Scrub sensitive env vars so cargo build scripts cannot leak them
unset MIKA_ANTHROPIC_API_KEY MIKA_INTERNAL_TOKEN MIKA_OPENAI_API_KEY MIKA_BRAVE_API_KEY

# Parse callback fields from enriched input
TASK_ID=$(printf '%s\n' "$INPUT" | jq -r '.__mika_task_id // empty')
AGENT=$(printf '%s\n' "$INPUT" | jq -r '.__mika_agent // empty')

if [ -z "$TASK_ID" ]; then
    echo "Error: no __mika_task_id in input (not running as long-running handler?)" >&2
    exit 1
fi

# Run the build
cd ~/workspace/senara-solutions/mika || { echo "ERROR: could not cd to mika repo" >&2; exit 1; }
BUILD_OUTPUT=$(cargo build --release --features telemetry 2>&1)
BUILD_EXIT=$?

# Deliver result via callback
if [ "$BUILD_EXIT" -eq 0 ]; then
    RESULT="Build succeeded.

${BUILD_OUTPUT}"
else
    RESULT="Build FAILED (exit code ${BUILD_EXIT}).

${BUILD_OUTPUT}"
fi

# Truncate to ~90KB to stay within 100KB limit
RESULT=$(printf '%s' "$RESULT" | head -c 92000)

# Deliver via mika ask --task-id (marks task complete and exits)
AGENT_FLAG=""
if [ -n "$AGENT" ]; then
    AGENT_FLAG="--agent $AGENT"
fi

mika ask --task-id "$TASK_ID" $AGENT_FLAG "$RESULT"
```

Key changes:
- Added jq and mika dependency checks
- Reads JSON from stdin (executor pipes enriched input)
- Scrubs MIKA_* env vars (defense-in-depth — executor already scrubs at Rust level)
- Parses `__mika_task_id` and `__mika_agent` from stdin JSON
- Captures build output to variable (stdout is /dev/null in long-running mode)
- Delivers success/failure via `mika ask --task-id` callback
- Truncates output to stay within 100KB limit
- Only exits non-zero if something truly unexpected happens (pre-build failures)

### system_prompt.md

```markdown
## build-mika Skill

When the user asks to build mika, you MUST call the `build_mika` tool.
Do NOT use `run_shell` for this — it will time out on release builds.

The `build_mika` tool is long-running:
1. It returns a task ID immediately
2. The build runs in the background (~2 minutes for release)
3. When complete, the result is delivered via callback
4. You will receive the build output (success or failure) automatically

Inform the user the build has started and you'll report back when it finishes.
Do NOT call `build_mika` again while a build is already running.

Build command (handled by the skill): `cargo build --release --features telemetry`
```

## Technical Considerations

- **stdout suppression**: Long-running exec handlers spawn with `stdout(Stdio::null())`. All meaningful output must be captured to a variable and delivered via the callback. This is the most critical pattern to get right.
- **stderr on failure**: The monitor captures stderr on non-zero exit. But since we always deliver via `mika ask`, the monitor path is only hit if pre-build checks fail or `mika ask` itself fails.
- **Concurrent builds**: Cargo uses a file lock on the target directory, so concurrent builds serialize. The system prompt warns against calling twice.
- **Path hardcoding**: `build.sh` hardcodes `~/workspace/senara-solutions/mika`. This is acceptable for a user-specific custom skill.
- **No code changes to Mika itself**: This is purely a skill file fix — no Rust code changes needed.

## Sources

- Bundled skill examples: `crates/mika-agent/templates/skills/shell-exec/` (canonical exec handler pattern)
- Manifest parser: `crates/mika-agent/src/skills/manifest.rs` (SkillManifest, SkillToolDef, ToolHandler)
- Skill loading: `crates/mika-agent/src/skills/index.rs` (scan_skills_dir, load_tools_json)
- Executor: `crates/mika-agent/src/skills/executor.rs` (execute_long_running, spawn_long_running_exec)
- Learnings: `docs/solutions/integration-issues/shell-exec-jq-json-parsing.md` (jq mandatory for JSON parsing)
- Learnings: `docs/solutions/security-issues/env-var-leakage-exec-handler-child-processes.md` (MIKA_* scrubbing)
