# Brainstorm: Out-of-Sandbox File Writes & Task Completeness

**Date:** 2026-03-11
**Status:** Ready for planning
**Triggered by:** Mika failed to adapt `~/.local/bin/quickclaw` — wrote to wrong path, didn't update Hyprland keybinding config

---

## What We're Building

Two prompt-level improvements (zero Rust code changes) that fix a class of agent failure where Mika:

1. **Cannot write files outside `~/.mika/`** — `write_agent_file` is sandboxed by design, but Mika defaults to it instead of using `run_shell` for user filesystem operations.
2. **Stops short of completing multi-step tasks** — treats "adapt this script" as "draft a replacement" rather than "make the system work end-to-end."

## Why This Approach

**Prompt-only fix, no new tools.** The reasoning:

- `run_shell` already solves out-of-sandbox writes — it just needs prompt guidance steering Mika toward it for the right cases.
- A new `write_user_file` Rust tool adds maintenance surface for a problem the shell already handles.
- Expanding `write_agent_file` to accept absolute paths silently erodes the sandbox invariant — future features that should be restricted may accidentally inherit expanded permissions.
- The confirmation step (not the filesystem restriction) is the real security boundary. This matches the industry pattern: Cursor, Claude Code, and Aider all use shell for out-of-sandbox writes with per-operation confirmation.

## Key Decisions

### Decision 1: Prompt-only, no new Rust code

Keep `write_agent_file` sandboxed to `~/.mika/`. Add system prompt guidance that explicitly directs Mika to use `run_shell` for file operations outside her home directory. The shell-exec skill's existing `system_prompt.md` is the right place for the technical guidance.

### Decision 2: Two-layer guidance (soul + skill)

The task completeness failure has two layers that map to different parts of the prompt system:

| Layer | Where | What |
|-------|-------|------|
| **Disposition** ("want to finish") | `soul.md` | Core principle: "When asked to adapt or replace something, you own the outcome, not just the artifact. The job isn't done until the system works." |
| **Technique** ("know how to finish") | `shell-exec/system_prompt.md` | Concrete pattern: grep for references, locate dependent configs (keybindings, aliases, systemd units), update them, verify the change landed. |

Soul alone is too abstract to catch the Hyprland case. Shell-exec alone can't install the disposition — it only fires when Mika already suspects there's a config to update.

### Decision 3: Context-dependent confirmation

Not all out-of-sandbox writes carry the same risk. The heuristic:

**Confirm before executing when:**
- The target path already exists (overwrite risk)
- The operation is destructive or irreversible (`rm`, `mv`, `chmod` on system files)
- The write touches a config controlling a running system (dotfiles, systemd units, WM configs)

**Narrate and execute immediately when:**
- Creating a net-new file at a path that doesn't exist
- Appending to a log or history file
- The operation is trivially reversible

**Rationale:** Uniform confirmation causes approval fatigue — rubber-stamping defeats the purpose. The two-line encoding: *"For out-of-sandbox writes, check if the target exists. If it does, confirm first and show the exact command. If it doesn't, narrate what you're creating and proceed."*

## Changes Required

### 1. `soul.md` (default soul / personality)

Location: `crates/mika-common/src/home.rs` → `DEFAULT_SOUL` constant

Add a principle about task ownership. Something like:
> When you adapt, replace, or rename something, you own the full outcome — not just the artifact. Trace all references (configs, keybindings, aliases, imports) and update them. The job isn't done until the system works end-to-end. Don't leave manual steps for the user.

### 2. `shell-exec/system_prompt.md`

Location: `crates/mika-agent/templates/skills/shell-exec/system_prompt.md`

Two additions:

**a) Out-of-sandbox write guidance:**
> For file operations outside `~/.mika/` (the user's filesystem), use `run_shell`. The `write_agent_file` tool is sandboxed to your home directory — it cannot reach paths like `~/.local/bin/`, `~/.config/`, etc.
>
> For out-of-sandbox writes, check if the target exists first. If it does, show the exact command and confirm before executing. If it doesn't, narrate what you're creating and proceed.

**b) Reference-tracing technique:**
> When adapting, replacing, or renaming a script/config/binary, trace all references to the original:
> - `grep -rl "original_name" ~/.config/` to find config files that reference it
> - Check keybinding configs, shell aliases, systemd units, cron entries
> - Update each reference and verify with a follow-up grep

### 3. Existing shell-exec guidance update

The current line *"Prefer the `write_agent_file` tool over shell writes when the content fits"* needs to be scoped: it should only apply to writes within `~/.mika/`. For user filesystem paths, `run_shell` is the correct tool.

## What Success Looks Like

Replay the quickclaw scenario:
1. User: "Adapt `~/.local/bin/quickclaw` to work with mika ask"
2. Mika reads the script, adapts the content
3. Mika writes to `~/.local/bin/quickmika` via `run_shell` (new file, narrate-and-execute)
4. Mika runs `chmod +x` immediately
5. Mika greps `~/.config/hypr/` for `quickclaw`, finds the keybinding
6. Mika shows the config change (existing file, confirm before executing)
7. Mika verifies with `ls -la` and `grep`
8. Zero manual steps for the user

## Resolved Questions

- **Q: Should we add a new Rust tool?** No. `run_shell` already handles this; adding tools creates maintenance surface for a solved problem.
- **Q: Where does the "finish the job" principle go?** Split: soul.md for disposition, shell-exec for technique.
- **Q: Confirm every out-of-sandbox write?** No — context-dependent. New files proceed with narration; existing files require confirmation. Avoids approval fatigue.

## Open Questions

None — all decisions resolved during brainstorming.
