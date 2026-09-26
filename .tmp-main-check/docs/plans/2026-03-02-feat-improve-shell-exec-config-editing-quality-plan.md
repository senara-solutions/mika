---
title: "feat: Improve shell-exec config editing quality and add shellcheck"
type: feat
status: completed
date: 2026-03-02
---

# feat: Improve shell-exec config editing quality and add shellcheck

## Overview

Mika's shell-exec skill can execute commands but provides no LLM guidance on
**how** to edit config files well. When asked to add a Hyprland keybinding,
Mika appended a bare `bind` line at the very end of `hyprland.conf` with no
comment or section grouping — while OpenClaw naturally created a well-formatted
section header with comments in the same file. The fix is to enhance the
shell-exec skill's `system_prompt.md` with config file editing best practices,
and to integrate shellcheck validation for scripts Mika writes.

## Problem Statement / Motivation

**What happened:** User asked Mika to add a claude-relay toggle shortcut to
Hyprland. Mika created the scripts correctly but appended the keybinding at
line 376 (the very end of `hyprland.conf`) with no section header or comment:

```
bind = $mainMod SHIFT, R, exec, /home/samidarko/.local/bin/claude-relay-toggle
```

**What should have happened:** A properly organized addition matching the file's
existing conventions:

```
##############################
### CLAUDE RELAY ###
##############################

# Toggle claude-asked relay on/off (Waybar indicator updates via RTMIN+8)
bind = $mainMod SHIFT, R, exec, ~/.local/bin/claude-relay-toggle
```

**Root cause:** The shell-exec skill's `system_prompt.md` only contains security
warnings. It provides zero guidance on **quality** of config file edits — no
instructions to read files first, find logical sections, add comments, or
preserve formatting conventions.

**Why OpenClaw does it better:** Research shows OpenClaw also has no explicit
config editing instructions. The difference is likely that OpenClaw's agent
reads the entire file before editing (its `edit` tool is separate from shell
commands), while Mika uses `run_shell` with heredocs/echo to append content
without first understanding the file structure.

**Second improvement:** The handler scripts Mika writes (via shell commands)
should be validated with `shellcheck` when available. shellcheck is already
installed on the user's system (`/usr/bin/shellcheck`) and could catch common
shell scripting mistakes.

## Proposed Solution

Two changes to the shell-exec skill:

### 1. Enhance `system_prompt.md` with config file editing guidance

Add best practices to `crates/mika-agent/templates/skills/shell-exec/system_prompt.md`
that tell the LLM to:
- Read the entire config file before modifying it
- Insert new entries in the logically correct section (not just append)
- Add section headers and inline comments matching the file's existing style
- Use `tee -a` or heredocs positioned correctly, not blind `echo >>`
- Make changes idempotent (check if already present)

### 2. Add shellcheck guidance

Add a note in the system prompt telling the LLM to validate shell scripts
with `shellcheck` when writing multi-line scripts to disk. This catches
common mistakes like unquoted variables, useless `cat`, and POSIX
compatibility issues.

## Technical Considerations

- **No Rust code changes.** This is purely a prompt/template change.
- **Bundled skill sync:** The `system_prompt.md` is compiled into the binary
  via `include_str!` in `bundled_skills.rs`. It gets synced to disk on startup
  (unless `MIKA_DISABLE_BUNDLED_SKILLS=true`).
- **Prompt budget:** The shell-exec system_prompt.md is injected via
  `<context type="skill">` blocks. Current content is 5 lines. Adding
  ~15-20 lines of guidance is well within budget.
- **Backward compatible:** Prompt-only change, no API or schema changes.

## Acceptance Criteria

- [x] `system_prompt.md` includes config file editing best practices
  - [x] Read file first before modifying
  - [x] Find and use logical sections
  - [x] Add section headers and comments
  - [x] Make changes idempotent
  - [x] Match existing file style
- [x] `system_prompt.md` includes shellcheck guidance for script writing
- [x] Existing tests still pass (`cargo test -p mika-agent`)
- [x] No Rust code changes required (prompt-only)

## MVP

### `crates/mika-agent/templates/skills/shell-exec/system_prompt.md`

```markdown
- SECURITY WARNING: This skill executes arbitrary shell commands. Use with extreme caution.
- Never execute commands that could damage the system, delete data, or expose secrets.
- Always confirm destructive commands (rm, mv, chmod, etc.) with the user before executing.
- Prefer read-only commands (ls, cat, grep, find, etc.) when possible.
- Do not pipe secrets or credentials into commands.

## Config file editing

When modifying config files (hyprland.conf, waybar config, bashrc, etc.):
- ALWAYS read the file first to understand its structure, sections, and formatting conventions.
- Insert new entries in the logically correct section — do NOT blindly append to the end.
- Add a clear section header comment when adding a new group of related settings (match the file's existing header style).
- Add inline comments explaining what each new entry does.
- Check if the entry already exists before adding it (idempotent edits).
- Use `~` instead of hardcoded absolute home paths where the config format supports it.

## Writing shell scripts

When writing shell scripts to disk:
- Run `shellcheck <script>` after writing to catch common issues (if shellcheck is available).
- Use `#!/bin/sh` for POSIX scripts or `#!/bin/bash` only when bash features are needed.
- Quote all variable expansions: `"$VAR"` not `$VAR`.
- Prefer `printf '%s\n'` over `echo` for portability.
```

## References & Research

- **Mika log:** `~/.mika/agents/main/logs/mika.log.2026-03-02` — shows tool execution trace
- **Conversation row 2178:** User asked for claude-relay toggle, Mika appended binding at EOF
- **Current hyprland.conf:** Line 376 has bare `bind` with no comment; lines 362-375 show OpenClaw's properly formatted section
- **Shell-exec skill:** `crates/mika-agent/templates/skills/shell-exec/system_prompt.md`
- **Prompt assembly:** `crates/mika-agent/src/prompt.rs:157-274` and `src/agent.rs:1139-1161`
- **Related solution:** `docs/solutions/integration-issues/shell-exec-jq-json-parsing.md` (PR #47)
- **OpenClaw system prompt:** `/home/samidarko/workspace/senara-solutions/openclaw/src/agents/system-prompt.ts` — no explicit config editing guidance either
- **shellcheck:** Installed at `/usr/bin/shellcheck`, referenced once in codebase (github handler SC2086 disable)
