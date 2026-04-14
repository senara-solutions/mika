---
title: "chore: Replace tmux-based stop/restart with OpenRC service management"
type: refactor
status: active
date: 2026-04-14
---

# chore: Replace tmux-based stop/restart with OpenRC service management

## Overview

Replace all tmux-based service stop/restart logic in the Makefile with OpenRC `rc-service` commands. The production Gentoo host uses `supervise-daemon` via OpenRC — tmux sessions are not used. The current Makefile `restart` target is completely non-functional without tmux, meaning `make deploy` builds and installs new binaries but leaves the old process running.

## Problem Frame

`make deploy` chains: `build-dashboard` -> `build` -> `stop` -> `install` -> `restart` -> `check-ngrok`. The `stop` target uses `tmux send-keys C-c` as its primary path (with a `pkill` fallback), and `restart` uses `tmux send-keys` with **no fallback at all**. On the Gentoo production host, which runs services via OpenRC/supervise-daemon, `restart` silently prints a warning and does nothing. This caused mika-qa to run with a stale skill registry for hours after a skills-only deploy.

## Requirements Trace

- R1. `make stop` stops services via `rc-service`, not tmux
- R2. `make restart` restarts services via `rc-service`, not tmux
- R3. `make deploy` results in the new binary actually running after completion
- R4. No tmux references remain in the deploy workflow (Makefile `stop`/`restart` targets and their help text)
- R5. Skills-only changes (no binary rebuild) still restart the server so skills are reloaded

## Scope Boundaries

- Only the Makefile `stop` and `restart` targets are changed
- The bundled `tmux` skill (`crates/mika-agent/templates/skills/tmux/`) is **unrelated** — it lets the agent manage tmux sessions as a tool. Not in scope.
- Historical tmux references in CHANGELOG.md, docs/plans/, docs/solutions/, todos/ are not modified
- No OpenRC init scripts or supervise-daemon configs are created in-repo — those are managed outside the repo on the host
- Sudoers entries already exist for the deploy user (`rc-service mika-server` and `rc-service mika-gateway`)
- `CLAUDE.md` line describing `make deploy` is tmux-agnostic ("dashboard+build+stop+install") and needs no change

## Context & Research

### Relevant Code and Patterns

- `Makefile` lines 20-55: current `stop` and `restart` targets with tmux logic
- `Makefile` line 69: `deploy` target chain — `build-dashboard build stop install restart check-ngrok`
- `CLAUDE.md` line 39: describes Makefile targets — currently tmux-agnostic wording
- The `BINARIES` variable (line 2) lists `mika mika-server mika-gateway`; stop/restart only operate on `mika-server mika-gateway`

### Institutional Learnings

- No prior learnings exist for service management patterns. This is a greenfield area in the knowledge base.

## Key Technical Decisions

- **Use `rc-service` directly, no fallback to pkill**: The issue explicitly specifies OpenRC as the sole mechanism. The tmux+pkill dual-path complexity was the root cause of the bug (silent no-op). A single, reliable path is better.
- **Use `stop` + `start` (not `restart`)**: The `stop` target runs before `install`, and `restart` runs after. This two-phase approach is correct — stop old binary, install new binary, start new binary. The Makefile `restart` target should use `rc-service start` (not `rc-service restart`) since the service was already stopped by the `stop` target. However, `rc-service restart` is idempotent and handles the already-stopped case, so either works. Use `restart` for robustness in case `stop` was skipped.
- **Keep the loop over both services**: Both `mika-server` and `mika-gateway` need stop/restart, preserving current behavior.

## Implementation Units

- [x] **Unit 1: Replace `stop` target with OpenRC commands**

  **Goal:** Replace the tmux-based stop logic with `sudo rc-service` stop commands.

  **Requirements:** R1, R3, R4

  **Dependencies:** None

  **Files:**
  - Modify: `Makefile`

  **Approach:**
  - Replace lines 20-45 (the entire `stop` target body) with a simple loop: `sudo rc-service $$bin stop` for each of `mika-server mika-gateway`
  - Update the help comment from `(via tmux C-c)` to `(via OpenRC)`
  - Use `|| true` after each stop command so Make doesn't fail if the service is already stopped

  **Patterns to follow:**
  - Keep the existing `@for bin in mika-server mika-gateway; do ... done` loop structure for consistency with the rest of the Makefile

  **Test expectation:** none — infrastructure target, no automated test. Verified manually via `make stop` on the host.

  **Verification:**
  - `make stop` calls `rc-service` for both services
  - No tmux references in the `stop` target

- [x] **Unit 2: Replace `restart` target with OpenRC commands**

  **Goal:** Replace the tmux-based restart logic with `sudo rc-service` restart commands.

  **Requirements:** R2, R3, R4, R5

  **Dependencies:** Unit 1

  **Files:**
  - Modify: `Makefile`

  **Approach:**
  - Replace lines 47-55 (the entire `restart` target body) with a simple loop: `sudo rc-service $$bin restart` for each of `mika-server mika-gateway`
  - Update the help comment from `in their tmux sessions` to `(via OpenRC)`
  - `rc-service restart` handles the already-stopped case (starts the service), so this works whether called after `stop` or independently

  **Patterns to follow:**
  - Same loop structure as the `stop` target

  **Test expectation:** none — infrastructure target, no automated test. Verified manually via `make restart` and `make deploy` on the host.

  **Verification:**
  - `make restart` calls `rc-service` for both services
  - No tmux references in the `restart` target
  - `make deploy` end-to-end results in the new binary running

## System-Wide Impact

- **Deploy chain:** The `deploy` target prerequisite list (`build-dashboard build stop install restart check-ngrok`) is unchanged — only the implementations of `stop` and `restart` change
- **Error propagation:** If `rc-service stop` fails (service not running), `|| true` prevents Make from aborting. If `rc-service restart` fails, it should propagate (deploy should fail visibly)
- **Unchanged invariants:** `build`, `build-dashboard`, `install`, `check-ngrok`, and all other Makefile targets are not modified. The bundled tmux skill is not affected.

## Risks & Dependencies

| Risk | Mitigation |
|------|------------|
| `sudo rc-service` requires sudoers entry | Already confirmed — deploy user has sudoers entries for both services |
| Service not registered in OpenRC | Out of scope — init scripts are managed on the host, not in-repo |

## Sources & References

- Related issue: #505
- Current Makefile: lines 20-55 (stop/restart targets)
