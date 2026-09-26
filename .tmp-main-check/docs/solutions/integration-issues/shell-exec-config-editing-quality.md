---
title: "Shell-Exec Config File Editing Quality and Shellcheck Integration"
date: 2026-03-02
category: integration-issues
tags:
  - shell-exec
  - config-files
  - prompt-engineering
  - shellcheck
  - best-practices
severity: low
component: mika-agent
related_files:
  - crates/mika-agent/templates/skills/shell-exec/system_prompt.md
  - crates/mika-agent/src/prompt.rs
---

# Shell-Exec Config File Editing Quality and Shellcheck Integration

## Problem Statement

When Mika's shell-exec skill was asked to add a Hyprland keybinding, it executed the commands correctly but produced poor-quality output. Instead of inserting the new binding into a properly formatted section with comments and matching the file's existing conventions, Mika appended a bare `bind` line at the very end of `hyprland.conf` with no section header or documentation. This demonstrated that the shell-exec skill lacks **guidance** on **how** to edit config files well.

### Symptoms

- Config file edits placed at EOF instead of in logical sections
- Missing section headers and comments
- Hardcoded absolute paths instead of using `~` shortcuts where appropriate
- No validation of written shell scripts with `shellcheck`
- Poor preservation of file formatting conventions

### Example: Poor vs. Good Config Edit

**What Mika produced (line 376 of hyprland.conf):**
```
bind = $mainMod SHIFT, R, exec, /home/samidarko/.local/bin/claude-relay-toggle
```

**What it should have produced:**
```
##############################
### CLAUDE RELAY ###
##############################

# Toggle claude-asked relay on/off (Waybar indicator updates via RTMIN+8)
bind = $mainMod SHIFT, R, exec, ~/.local/bin/claude-relay-toggle
```

## Investigation Steps

1. **Log analysis**: Reviewed `~/.mika/agents/main/logs/mika.log.2026-03-02` showing tool execution trace
2. **File inspection**: Examined `hyprland.conf` — line 376 vs. lines 362-375 (OpenClaw's properly formatted section)
3. **Prompt review**: Checked `crates/mika-agent/templates/skills/shell-exec/system_prompt.md` — contained only 5 lines of security warnings
4. **Comparative analysis**: Studied OpenClaw's system prompt to understand how it achieved better results
5. **Script validation gap**: Identified that shell scripts written by Mika were never validated with `shellcheck`

## Root Cause

The shell-exec skill's `system_prompt.md` provided **zero guidance on config file editing quality**. It contained only security warnings (no destructive commands, no secret leaks, etc.) but no instructions to:

- Read the file first before editing
- Find the logically correct section for new content
- Add section headers and comments matching the file's existing style
- Make edits idempotent (check if already present)
- Use appropriate path notation (`~` vs. absolute paths)

Similarly, there was no guidance on validating shell scripts with `shellcheck` after writing them to disk.

### Why This Happened

Mika's shell-exec skill uses `run_shell` with heredocs and echo commands to append content to files. Without explicit guidance in the system prompt, the LLM has no knowledge of file structure and simply executes the command as requested, resulting in EOF appends.

OpenClaw achieves better results through a different architecture: its `edit` tool is separate from shell commands, and it reads the entire file before editing, giving the model context about the file structure.

## Solution

Enhanced the shell-exec skill's `system_prompt.md` with two new sections: one for config file editing best practices and one for shell script writing validation.

### File Changed

**Path**: `crates/mika-agent/templates/skills/shell-exec/system_prompt.md`

**Changes**: Added 18 lines (from 5 to 23 lines total):

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

### Technical Implementation Details

1. **Prompt Injection**: The shell-exec system_prompt is injected via `<context type="skill">` blocks during prompt assembly in `crates/mika-agent/src/prompt.rs`

2. **Bundled Skill Sync**: The `system_prompt.md` is compiled into the binary via `include_str!` in `bundled_skills.rs`. On startup, unless `MIKA_DISABLE_BUNDLED_SKILLS=true` is set, the file is synced from the binary to disk, ensuring users always have the latest guidance

3. **No Rust Code Changes**: This is purely a prompt/template change — no changes to the skill handler or agent loop logic

4. **Prompt Budget**: The addition of ~18 lines is well within the token budget for system prompt context

5. **Backward Compatible**: This is a pure enhancement with no API or schema changes

### How It Works

When a user asks Mika to add a config entry:

1. The shell-exec skill is invoked
2. The enhanced `system_prompt.md` is injected into the context
3. The LLM reads the guidance: "ALWAYS read the file first to understand its structure"
4. The LLM generates a command that reads the file first: `cat hyprland.conf | head -100` or similar
5. The LLM sees the existing section structure (e.g., `### KEYBINDS ###`)
6. The LLM generates a command that inserts the new binding in the correct section with comments, matching the existing style
7. If the LLM writes a shell script, it now has explicit guidance to validate it with `shellcheck`

## What Didn't Work

### Pure Prompt Engineering Without Examples

Initial consideration: Could we add this guidance without showing examples? No — the LLM needs concrete patterns to follow. By showing the section header format from the problem statement in the guidance, we make it clear what "match the file's existing header style" means.

### Attempting to Enforce via Regex Validation

Another approach considered: Add validation in the agent loop to reject shell commands that append without reading first. This was rejected because:
- False positives: `echo "setting=value" >> config.ini` might be correct in some cases
- Adds runtime overhead with no user visibility
- Better to guide the LLM to good behavior via prompting

### Relying on File-Edit Tool Instead

A deeper solution might be to add a separate `edit_config_file` tool like OpenClaw has, which would:
- Parse the file structure
- Find logical sections
- Validate insertion points
- Rewrite the file safely

However, this would require significant Rust code changes and is outside the scope of this fix. The prompt-only approach achieves 80% of the benefit with minimal code change.

## Prevention Strategies

1. **Skill system prompts are governance tools**: Use them to document not just *what* a skill does, but *how* to use it well
2. **Provide examples in negative space**: Show users what NOT to do (bare appends, hardcoded paths, unquoted vars)
3. **Link to external validation**: When a tool like `shellcheck` exists and is available, guide the LLM to use it
4. **Document file-specific conventions**: Mention actual config files users are likely to edit (hyprland.conf, waybar, bashrc)
5. **Idempotency is key**: For destructive or state-changing operations, always include guidance on checking state first
6. **Re-sync bundled skills regularly**: The bundled skill system ensures users always get the latest guidance on startup

## Testing

- Existing test suite passes: `cargo test -p mika-agent`
- No new tests required (prompt-only change)
- Manual verification: Future user requests to edit config files should produce better-formatted output

## Related References

- **Commit**: `25d67a1` — "feat: add config editing and shellcheck guidance to shell-exec skill"
- **PR**: #48 on branch `feat/shell-exec-config-editing-quality`
- **Plan**: `docs/plans/2026-03-02-feat-improve-shell-exec-config-editing-quality-plan.md`
- **Related solution**: `docs/solutions/integration-issues/shell-exec-jq-json-parsing.md` (PR #47 — shell-exec JSON parsing improvements)
- **Reference implementations**:
  - OpenClaw `src/agents/system-prompt.ts` — no explicit config editing guidance (relies on separate edit tool)
  - Mika shell-exec skill: `crates/mika-agent/src/skills/shell_exec/mod.rs`
- **Shellcheck**: Available at `/usr/bin/shellcheck`, man page at `man shellcheck`
