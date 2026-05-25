---
name: mika-spawn
description: Spawn a fresh claude tenant in Hyprland workspace 4 with an intent threaded as the first prompt
argument-hint: "[<intent>] [--bare] [--tactical]"
---

Run the spawn launcher and surface its confirmation:

```bash
"$(git rev-parse --show-toplevel)/scripts/mika-platform-spawn" $ARGUMENTS
```

If the script exits non-zero, surface its stderr to the operator. On success, surface the one-line stdout confirmation.

## Argument shape

- `/mika-spawn <intent>` → spawns `claude "/mika <intent>"` in workspace 4. Examples: `/mika-spawn mika#984`, `/mika-spawn fix release label`.
- `/mika-spawn /<command> [args]` → spawns `claude "/<command> [args]"` in workspace 4 (direct slash command, no `/mika` prefix). Examples: `/mika-spawn /mika-handsoff`, `/mika-spawn /mika-onboarding /exit`.
- `/mika-spawn` (no args) → spawns `claude "/mika-onboarding"` in workspace 4 (handsoff → onboarding bridge).
- `/mika-spawn --tactical <intent>` → spawns `claude "<intent>"` in workspace 4 (no `/mika` prefix; for tactical work on existing worktrees — CI fixes, rebase resolution, prompt compression, hotfixes).
- `/mika-spawn --bare` → spawns `claude` in workspace 4 with no first-prompt injection.

## Related

- `scripts/mika-platform-spawn` — the actual launcher.
- `.claude/commands/mika-onboarding.md` — the no-args default's target (handsoff → onboarding bridge).
- `docs/operator/hyprland-keybind-mika-spawn.md` — keybind setup snippet.
- `docs/brainstorms/2026-05-06-session-continuity-orthogonality-brainstorm.md` — origin design.
- `MIKA_SPAWN_ID` env var — correlation handle exported to spawned tenant for orchestrator inbox protocol (mika-platform#100). Non-bare spawns only.
- `.claude/commands/mika-handsoff.md` — Phase 6 writes inbox entry when `MIKA_SPAWN_ID` is set; Phase 0 reads inbox when running as orchestrator.
