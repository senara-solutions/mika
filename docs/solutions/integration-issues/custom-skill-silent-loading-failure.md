---
title: Custom Skill Silently Skipped Due to Manifest and Handler Format Issues
date: 2026-03-11
module: skills system
tags:
  - skill-loading
  - manifest-validation
  - silent-failure
  - exec-handlers
  - long-running
  - callback-delivery
severity: medium
symptoms:
  - Skill directory exists but skill is not loaded at agent startup
  - Tools from skill are unavailable despite correct placement in skills directory
  - No error message in TUI; only warn! in tracing logs indicates parsing failure
  - skill.toml uses flat format instead of [skill] section wrapper
  - tools.json missing required handler field on tool definitions
  - Exec handler script does not consume stdin or implement callback delivery
---

# Custom Skill Silently Skipped Due to Manifest and Handler Format Issues

## Problem

A custom Mika agent skill at `~/.mika/agents/{agent}/skills/{skill}/` was completely broken and silently ignored at startup. The agent never saw the tool, and the user received no error — only `warn!` logs during `scan_skills_dir()`.

### Symptoms

- Skill directory exists with all expected files (`skill.toml`, `tools.json`, `build.sh`, `system_prompt.md`)
- `mika skills list` does not show the skill (or shows it without tools)
- Agent cannot call the tool — it doesn't appear in the tool list
- No user-visible error; must check tracing logs for `warn!` entries

## Root Cause

Four format violations that each independently prevent the skill from loading:

### 1. skill.toml — Missing `[skill]` Section

The `SkillManifest` struct (`crates/mika-agent/src/skills/manifest.rs`) requires a top-level `[skill]` section. The parser has no default for this section — it fails with `missing field 'skill'`.

**Broken:**
```toml
name = "build-mika"
description = "Build the mika project"

[handler]
type = "exec"
command = "build.sh"

[options]
always_on = false
```

**Fixed:**
```toml
[skill]
name = "build-mika"
description = "Build the mika project"
version = "0.1.0"
always_on = false
timeout_secs = 300

[triggers]
keywords = ["build mika", "cargo build"]
```

Note: `is_legacy_format()` in `index.rs` only detects `[handler] type="builtin"`. Legacy skills with `type="exec"` fall through to a generic parse error — no helpful deprecation message.

### 2. tools.json — Missing `handler` Field

The `SkillToolDef` struct requires a `handler` field. Without it, `load_tools_json()` fails to deserialize, logs a warning, and returns an empty tool list. The skill loads but with zero tools.

**Broken:**
```json
[{
  "name": "build_mika",
  "description": "Build the project",
  "input_schema": {"type": "object", "properties": {}, "required": []}
}]
```

**Fixed:**
```json
[{
  "name": "build_mika",
  "description": "Build the project",
  "input_schema": {"type": "object", "properties": {}, "required": []},
  "handler": {
    "type": "exec",
    "command": "build.sh",
    "long_running": true,
    "estimated_duration_secs": 120
  }
}]
```

### 3. build.sh — Missing Long-Running Handler Protocol

For `long_running: true` exec handlers, the executor spawns the subprocess with `stdout(Stdio::null())`. The script must:
- Read JSON from stdin (contains `__mika_task_id` and `__mika_agent` injected by executor)
- Capture work output to a variable (stdout is discarded)
- Deliver results via `mika ask --task-id <id> --agent <name> -- <result>`

**Broken:**
```sh
#!/bin/sh
cd ~/workspace/project || exit 1
cargo build --release 2>&1  # Output goes to /dev/null — lost!
```

**Fixed:**
```sh
#!/bin/sh
command -v jq >/dev/null 2>&1 || { echo "Error: jq required" >&2; exit 1; }
command -v mika >/dev/null 2>&1 || { echo "Error: mika CLI required" >&2; exit 1; }

INPUT=$(cat)
unset MIKA_ANTHROPIC_API_KEY MIKA_INTERNAL_TOKEN MIKA_OPENAI_API_KEY MIKA_BRAVE_API_KEY

TASK_ID=$(printf '%s\n' "$INPUT" | jq -r '.__mika_task_id // empty')
AGENT=$(printf '%s\n' "$INPUT" | jq -r '.__mika_agent // empty')

[ -z "$TASK_ID" ] && { echo "Error: no __mika_task_id" >&2; exit 1; }

cd ~/workspace/project || exit 1
BUILD_OUTPUT=$(cargo build --release 2>&1)
BUILD_EXIT=$?

if [ "$BUILD_EXIT" -eq 0 ]; then
    RESULT="Build succeeded.
${BUILD_OUTPUT}"
else
    RESULT="Build FAILED (exit ${BUILD_EXIT}).
${BUILD_OUTPUT}"
fi

RESULT=$(printf '%s' "$RESULT" | head -c 92000)

if [ -n "$AGENT" ]; then
    mika ask --task-id "$TASK_ID" --agent "$AGENT" -- "$RESULT"
else
    mika ask --task-id "$TASK_ID" -- "$RESULT"
fi
```

### 4. Security Convention: `--` Before Positional Args

Build output is untrusted external data that may start with `--`. Without the `--` separator, clap could misinterpret it as a flag. Always use `mika ask --task-id "$ID" -- "$RESULT"`.

## Investigation Steps

1. Checked `mika skills list` — skill not shown
2. Examined bundled skill examples (`templates/skills/shell-exec/`) for correct format
3. Read `SkillManifest` and `SkillToolDef` structs in `manifest.rs` — found `[skill]` section and `handler` field are mandatory
4. Read `scan_skills_dir()` and `load_tools_json()` in `index.rs` — confirmed silent skip with `warn!`
5. Read `execute_long_running()` in `executor.rs` — confirmed stdout goes to `/dev/null`, `__mika_task_id`/`__mika_agent` injected into stdin JSON
6. Checked `is_legacy_format()` — only catches `type="builtin"`, not `type="exec"`

## Solution

Fixed all four files to match the conventions established by bundled skills:
- `skill.toml`: Added `[skill]` section, proper field placement
- `tools.json`: Added `handler` object with exec type, long_running, estimated_duration_secs
- `build.sh`: Full long-running handler protocol (stdin, jq, env scrub, output capture, callback)
- `system_prompt.md`: Documented callback behavior for the agent

## Custom Skill Authoring Checklist

When creating a custom skill with a long-running exec handler:

- [ ] `skill.toml` has `[skill]` section (not flat top-level fields)
- [ ] `skill.toml` has no `[handler]` or `[options]` sections (those are invalid)
- [ ] `tools.json` includes `handler` object on each tool: `{type, command, long_running, estimated_duration_secs}`
- [ ] Handler script checks `jq` and `mika` are in PATH
- [ ] Handler reads stdin: `INPUT=$(cat)`
- [ ] Handler parses `__mika_task_id` and `__mika_agent` from stdin JSON via `jq`
- [ ] Handler scrubs `MIKA_*` env vars: `unset MIKA_ANTHROPIC_API_KEY ...`
- [ ] Handler captures work output to variable (stdout is /dev/null in long-running mode)
- [ ] Handler delivers results via `mika ask --task-id <id> --agent <name> -- <result>`
- [ ] Handler uses `--` before positional args containing untrusted data
- [ ] Handler script is executable (`chmod +x`)
- [ ] Output truncated to stay within 100KB limit

## Prevention

1. **Consult bundled skill examples** before authoring custom skills — `crates/mika-agent/templates/skills/shell-exec/` is the canonical exec handler pattern
2. **Check tracing logs** if a skill doesn't appear — `scan_skills_dir()` logs `warn!` on failures
3. **Consider a `mika skills validate` command** (not yet implemented) to catch format errors before startup
4. **Broaden `is_legacy_format()` detection** to catch `[handler]` with any `type` value, not just `"builtin"`

## Related Documentation

- `docs/skills.md` — Complete skill system reference with custom skill walkthrough
- `docs/solutions/integration-issues/shell-exec-jq-json-parsing.md` — Why jq is mandatory for exec handlers
- `docs/solutions/security-issues/env-var-leakage-exec-handler-child-processes.md` — MIKA_* env scrubbing
- `docs/solutions/architecture/callback-resume-agent-lifecycle.md` — Full callback task lifecycle
- `docs/adr/002-filesystem-skill-registry.md` — Design rationale for skill system

## Key Files

- `crates/mika-agent/src/skills/manifest.rs` — SkillManifest, SkillToolDef, ToolHandler structs
- `crates/mika-agent/src/skills/index.rs` — scan_skills_dir(), load_tools_json(), is_legacy_format()
- `crates/mika-agent/src/skills/executor.rs` — execute_long_running(), spawn_long_running_exec()
- `crates/mika-agent/templates/skills/shell-exec/` — Canonical exec handler example
