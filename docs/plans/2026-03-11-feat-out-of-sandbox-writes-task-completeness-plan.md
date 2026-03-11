---
title: "feat: Out-of-sandbox file writes and task completeness"
type: feat
status: active
date: 2026-03-11
origin: docs/brainstorms/2026-03-11-out-of-sandbox-writes-brainstorm.md
---

# feat: Out-of-sandbox file writes and task completeness

Prompt-level fix (+ one error message string change) for a class of agent failure where Mika writes files to the wrong location (sandbox instead of user filesystem) and stops short of completing multi-step tasks.

## Acceptance Criteria

- [ ] Soul.md contains a task ownership principle about owning outcomes end-to-end
- [ ] Shell-exec system prompt directs agent to `run_shell` for paths outside home/workspace
- [ ] Shell-exec system prompt includes context-dependent confirmation heuristic (confirm overwrites, narrate new files)
- [ ] Shell-exec system prompt includes reference-tracing technique (grep configs after rename/adapt)
- [ ] Existing "prefer write_file" line is scoped to home directory only
- [ ] `validate_and_resolve_path` error message hints at `run_shell` for out-of-sandbox paths
- [ ] Existing tests pass (`cargo test`)
- [ ] Build succeeds (shell-exec templates are `include_str!` compiled)

## Changes

### 1. `crates/mika-common/src/home.rs` — DEFAULT_SOUL

Add a task ownership principle to the Boundaries section (see brainstorm: Decision 2, Layer 1):

```
## Boundaries
...
- When you adapt, replace, or rename something, you own the full outcome — not just the artifact.
  Trace all references (configs, keybindings, aliases, imports) and update them.
  The job isn't done until the system works end-to-end. Don't leave manual steps for the user.
```

This is a dispositional directive — it tells the agent *why* to trace references. The *how* lives in the shell-exec skill.

### 2. `crates/mika-agent/templates/skills/shell-exec/system_prompt.md`

Three edits to the existing file:

**a) Scope the existing write_file preference (line 21):**

Change:
```
- Prefer the `write_file` tool over shell writes when the content fits — it enforces read-before-overwrite automatically.
```
To:
```
- For files inside your home directory, prefer `write_file` over shell writes — it enforces read-before-overwrite automatically.
```

**b) Add out-of-sandbox write guidance (new section after "File writing"):**

```
## Writing files outside your home directory

Your `write_file` tool is sandboxed to your home directory — it cannot reach paths like `~/.local/bin/`, `~/.config/`, etc. For file operations outside your home directory (and outside any team workspace), use `run_shell`.

Before writing via shell, check whether the target already exists:
- **Target exists:** Show the user the exact command you'll run and what it overwrites. Wait for explicit confirmation before executing. Example: "~/.config/hypr/hyprland.conf exists and contains a quickclaw keybinding. I'll replace it with quickmika. Here's the change: [show diff]. Shall I proceed?"
- **Target does not exist:** Tell the user what you're creating, then execute immediately. Example: "I'll create ~/.local/bin/quickmika with the adapted script." Then write the file and set permissions in the same turn.

After writing, verify success (`ls -la` for existence/permissions, `head` for content spot-check).
```

**c) Add reference-tracing technique (new section):**

```
## Tracing references after rename or adaptation

When you adapt, rename, or replace a script, config, or binary, trace references to the original:
- `grep -rl "original_name" ~/.config/` — find config files (keybindings, WM configs, autostart)
- Check shell configs (`~/.bashrc`, `~/.zshrc`, `~/.profile`) for aliases or PATH references
- Check systemd units, cron entries, desktop files if relevant
- Update each reference found and verify with a follow-up grep

If a location requires access you don't have, or the search returns no results and you're uncertain coverage was complete, list what you could not verify so the user can check manually.
```

### 3. `crates/mika-agent/src/tools/mod.rs` — error message hint

One-line string change in `validate_and_resolve_path` (line ~288). Change the absolute-path rejection message from:

```
"Absolute paths are not allowed. Use a relative path within the directory."
```

To:

```
"Absolute paths are not allowed. Use a relative path within your home directory, or use run_shell for paths outside it."
```

Last line of defense when prompt guidance isn't enough — prevents the agent from retrying with a wrong relative path instead of switching tools. String literal only, no logic change.

## SpecFlow Findings Incorporated

From the spec-flow analysis (see brainstorm for full context):

| Finding | Resolution |
|---|---|
| `write_workspace` interaction | Guidance says "outside your home directory and any team workspace" |
| "Narrate-and-execute" undefined | Added concrete examples in section 2b |
| Silent/team mode interaction | Soul principle is aspirational; agents do their best with available tools. Shell-exec excluded from silent mode already |
| Privilege escalation (`sudo`) | Not addressed in prompts — existing "confirm destructive commands" covers it. Agent should never `sudo` unprompted |
| Rewind can't undo filesystem changes | Acceptable limitation — out of scope for prompt fix |

## Sources

- **Origin brainstorm:** [docs/brainstorms/2026-03-11-out-of-sandbox-writes-brainstorm.md](docs/brainstorms/2026-03-11-out-of-sandbox-writes-brainstorm.md) — key decisions: prompt-only fix, two-layer guidance (soul + skill), context-dependent confirmation
- **Existing pattern:** `write_file` overwrite confirmation flow (`docs/solutions/integration-issues/write-file-tool-overwrite-confirmation-flow.md`)
- **Existing pattern:** Tool path reporting — resolved absolute paths (`docs/solutions/logic-errors/tool-path-reporting-misbehavior.md`)
- **Current files:** `crates/mika-common/src/home.rs:252-275` (DEFAULT_SOUL), `crates/mika-agent/templates/skills/shell-exec/system_prompt.md` (full file), `crates/mika-agent/src/prompt.rs:390-420` (home_dir in system prompt)
