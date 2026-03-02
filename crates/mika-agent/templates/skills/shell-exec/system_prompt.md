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
