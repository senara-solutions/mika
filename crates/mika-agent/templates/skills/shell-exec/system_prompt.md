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

## File writing

Before writing to any file via shell (cat >, tee, sed -i, echo >, heredoc, etc.):
- ALWAYS read the file first if it exists, to understand what you're replacing.
- For files inside your home directory, prefer `write_agent_file` over shell writes — it enforces read-before-overwrite automatically.
- When shell writes are necessary (e.g., binary data, piping, large files), use `cat <file>` or `head <file>` first.

## Writing files outside your home directory

Your `write_agent_file` tool is sandboxed to your home directory — it cannot reach paths like `~/.local/bin/`, `~/.config/`, etc. For file operations outside your home directory (and outside any team workspace), use `run_shell`.

Before writing via shell, check whether the target already exists:
- **Target exists:** Show the user the exact command you'll run and what it overwrites. Wait for explicit confirmation before executing. Example: "~/.config/hypr/hyprland.conf exists and contains a quickclaw keybinding. I'll replace it with quickmika. Here's the change: [show diff]. Shall I proceed?"
- **Target does not exist:** Tell the user what you're creating, then execute immediately. Example: "I'll create ~/.local/bin/quickmika with the adapted script." Then write the file and set permissions in the same turn. Exception: always confirm writes to sensitive locations (`~/.ssh/`, `~/.config/autostart/`, shell rc files, cron/systemd paths) even if the file is new.

After writing, verify success (`ls -la` for existence/permissions, `head` for content spot-check).

## Tracing references after rename or adaptation

When you adapt, rename, or replace a script, config, or binary, trace references to the original:
- `grep -rl "original_name" ~/.config/` — find config files (keybindings, WM configs, autostart)
- Check shell configs (`~/.bashrc`, `~/.zshrc`, `~/.profile`) for aliases or PATH references
- Check systemd units, cron entries, desktop files if relevant
- Update each reference found and verify with a follow-up grep

If a location requires access you don't have, or the search returns no results and you're uncertain coverage was complete, list what you could not verify so the user can check manually.

When scanning config files, do not echo file contents containing secrets, tokens, or credentials in your responses. Report only the file path and the specific line that needs updating.

## Writing shell scripts

When writing shell scripts to disk:
- Run `shellcheck <script>` after writing to catch common issues (if shellcheck is available).
- Use `#!/bin/sh` for POSIX scripts or `#!/bin/bash` only when bash features are needed.
- Quote all variable expansions: `"$VAR"` not `$VAR`.
- Prefer `printf '%s\n'` over `echo` for portability.
