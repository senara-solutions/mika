---
name: mika-onboarding
description: Receiver-side session-start ceremony — read latest handsoff log, run status, surface decisions-in-flight, emit ready block
argument-hint: ""
---

This command is the receiver-side dual to `/mika-handsoff`. It loads narrative continuity (latest handsoff log + workspace status + decisions-in-flight) when an operator boots a new orchestrator-Claude session. Standalone-callable in any meta-repo Claude session, and the no-args default that `/mika-spawn` (mika-platform#86) injects.

This command reads documents and memory only — it does not query `mika tasks --agent ... list` or `gh issue list --label ready`. No agent-internal probing.

## Phase 1 — Workspace-detection guard

Before any file operations, verify `mika-platform/docs/logs/` is reachable. Determine the meta-repo root:
- If cwd is inside the mika-platform repo (check `git -C . rev-parse --show-toplevel` and verify basename is `mika-platform`), use that as the root.
- Otherwise, check `../mika-platform/`, `../../mika-platform/`, or the workspace root from `CLAUDE_PROJECT_ROOT`.

If `docs/logs/` cannot be located, **halt immediately** with:

> Cannot locate mika-platform workspace from current directory. Invoke from inside the mika-platform workspace.

Do not proceed. No further phases execute.

### Canonical-path resolution

Once the meta-repo root is identified, resolve it to its canonical path:

```bash
META_REPO_ROOT=$(realpath "$META_REPO_ROOT" 2>/dev/null || readlink -f "$META_REPO_ROOT")
```

Bare `$PWD` is incorrect under symlink setups — `~/workspace/mika-platform` may be symlinked to `/data/workspace/mika-platform`, and Claude Code's auto-memory namespace slug is derived from the canonical path, not the symlink path. Save `META_REPO_ROOT` for Phase 4's memory-path derivation.

> **Sync warning:** This Phase mirrors `.claude/commands/mika-handsoff.md` Phase 1 § Workspace-detection guard. Update both together when the heuristic changes.

## Phase 2 — Locate latest handsoff log

Enumerate `docs/logs/*.md` from the meta-repo root (exclude `HANDSOFF-CONTRACT.md` and any file whose leading filename prefix does not match `YYYY-MM-DD`).

Track the count of skipped non-conforming filenames silently — no halt, no error, no warning at this phase. Surface the count in Phase 6's ready block if non-zero.

```
LET TODAY = current date in YYYY-MM-DD form.
```

**Case 1 — most-recent-date == TODAY AND count(files for TODAY) > 1:**
HALT — HANDSOFF-CONTRACT hard rule #3 violation. Surface the file list to the operator and stop. Do not consolidate. Do not pick a winner.

**Case 2 — most-recent-date == TODAY AND count(files for TODAY) == 1:**
READ that file.

**Case 3 — most-recent-date < TODAY AND count(files for that date) > 1:**
PICK the file with the newest mtime (the `-v2` continuation case per HANDSOFF-CONTRACT hard rule #2). READ that file.

**Case 4 — most-recent-date < TODAY AND count(files for that date) == 1:**
READ that file.

**Case 5 — no files at all matching the YYYY-MM-DD prefix:**
Continue to subsequent phases. Phase 5's no-log-at-all threshold will handle this case after Phases 3 and 4 have run.

**Why the today-vs-prior-day asymmetry:** HANDSOFF-CONTRACT hard rule #3 makes multiple files for *today* a contract bug — implementations should never produce that state, so encountering it signals something is wrong. Multiple files for a *prior day* is the documented `-v2` escape-hatch — permitted, and the mtime-newest pick is the contract-aligned reading.

After reading the chosen file, summarize the TL;DR and carry-forward state for the operator in chat. Surface the `## Blocked / carry-forward` and `## What to do next session` sections prominently.

## Phase 3 — Run mika-platform-status

From the meta-repo root, invoke:

```bash
scripts/mika-platform-status
```

Surface notable items from the output: dirty repos, unexpected branches, open PRs, stale worktrees marked "prunable." If the output is clean, note that briefly and move on.

Same conventions as `.claude/commands/mika-platform-status.md`.

## Phase 4 — Read decisions-in-flight memory

Derive the auto-memory namespace slug from the canonical `META_REPO_ROOT` captured in Phase 1:

1. Replace every `/` with `-` in the canonical path. The leading `/` naturally becomes a leading `-`.
   Example: `/data/workspace/mika-platform` → `-data-workspace-mika-platform`.
2. Full path: `~/.claude/projects/<slug>/memory/project_decisions_in_flight.md`.

```bash
SLUG=$(echo "$META_REPO_ROOT" | sed 's|/|-|g')
DECISIONS_FILE="$HOME/.claude/projects/$SLUG/memory/project_decisions_in_flight.md"
```

- **If the file exists**, read it and surface unresolved decision entries in chat.
- **If the file is missing**, skip silently. No warning, no halt — fresh operators or alternate-path checkouts hit this branch.

## Phase 4.5 — Strategic surface read

Beyond today's handsoff, the operator's strategic direction lives in two more surfaces. Reading them at session start prevents the orchestrator from answering strategy/direction questions from incomplete grounding — a pattern documented in memory `feedback_strategic_grounding_three_surfaces` after the 2026-05-13 incident where the orchestrator dismissed a planned milestone as a defer-it concept.

### Phase 4.5a — Open milestones across the workspace

```bash
for REPO in mika mika-platform mika-skills mika-cloud; do
  gh api repos/senara-solutions/$REPO/milestones --jq ".[] | select(.state == \"open\") | \"$REPO#\(.number) \(.title) — \(.open_issues) open / \(.closed_issues) closed\"" 2>/dev/null
done
```

Surface every open milestone in chat with its open/closed count. These are the multi-week-to-quarter strategic vectors. Closing one is a goal-state shift; opening one is a new strategic direction. The operator expects the orchestrator to know them when asked about strategy, direction, long-term goals, or "what's next at the milestone level."

If a milestone has a `description` worth seeing (>50 chars), include the truncated description (`description[:200]`) — strategic vectors often encode rationale that issue titles don't.

### Phase 4.5b — Rolling 7-day handsoff window awareness

Phase 2 already enumerated `docs/logs/*.md`. Count how many handsoff logs match the `YYYY-MM-DD` prefix with date >= today-7. Surface the count and the date list in chat.

**Do NOT auto-read the full rolling window** — context cost is too high; only the latest is auto-read in Phase 2. But knowing the count exists encodes the contract: when the operator asks about strategic direction / long-term goals / "what's next" / "are we still on plan," the orchestrator MUST deep-read the rolling window before answering, not project from the latest log alone.

- If `rolling_count == 1` (only today's log), no strategic context predates today — deep-read unnecessary.
- If `rolling_count > 1`, deep-read required on any strategic question.

The ceremony does not enforce the deep-read at session start (overload risk); it enforces awareness so the orchestrator does the read on demand.

## Phase 5 — Two-threshold staleness handling

Compute N (days since most-recent log) **from filename-encoded date**, not mtime:

1. Enumerate `docs/logs/*.md`, parse the leading `YYYY-MM-DD` from each filename.
2. Take the max date as `LATEST_LOG_DATE`.
3. `N = today - LATEST_LOG_DATE` (in days).

**Threshold 1 — No log at all** (no file matches the `YYYY-MM-DD` prefix pattern):
Halt with diagnostic:

> No handsoff log found. Either this is a fresh workspace or HANDSOFF-CONTRACT is being skipped. Want me to proceed with status + decisions-in-flight only?

Wait for operator confirmation. If the operator agrees, skip the handsoff summary in Phase 6 and emit the ready block with status + decisions only. If the operator declines, stop.

**Threshold 2 — N > 2:**
Emit a soft warning:

> ⚠ Last handsoff is N days old (LATEST_LOG_DATE). Context may be stale.

Continue without halting. All subsequent phases run normally.

The 2-day soft threshold matches the autonomous-loop's normal daily cadence.

## Phase 6 — Emit terminal ready block

Print a single summary block:

```
=== Onboarding ===
Log:        <filename read in Phase 2, or "(none)" if no-log path>
Stale:      <N days old, or "current" if today, or "n/a" if no log>
Carry-fwd:  <count of items from Blocked/carry-forward section>
Decisions:  <count of unresolved decisions-in-flight, or "n/a" if file missing>
Milestones: <count> open across workspace (deep list surfaced above in Phase 4.5a)
Rolling:    <N> handsoff logs in last 7d (deep-read on strategic Qs per Phase 4.5b)
Status:     <count of notable items from Phase 3, or "clean">
Skipped:    <count of malformed-filename files from Phase 2, or omit line if 0>
Ready.
```

The operator can scroll past this block in <5 seconds. The full details were surfaced inline during each phase — this block is a compressed index, not restated content.

---

## Discipline

- **Synthesize from session context, don't quiz the operator.** This command takes no arguments and asks no questions (except R7's no-log-at-all confirmation).
- **No agent-internal probing.** No `mika tasks ... list`, no `gh issue list --label ready`. Document-driven only.
- **Workspace-detection guard halts the entire flow on failure.** No partial execution from outside the workspace.
- **Staleness halts only on no-log-at-all.** The >2-day-old case continues with a soft warning — it is not a contract violation, just a freshness signal.
- **Read-only ceremony.** No file writes, no commits, no network mutations (except `gh pr list` inside `mika-platform-status` and `gh api repos/.../milestones` in Phase 4.5a, both read-only).
- **Strategic grounding is ceremony, not memory.** Phase 4.5 enforces that orchestrator-Claude has access to milestones + rolling-window awareness at session start, so any subsequent strategic-direction question is answered from the canonical surfaces (memory + 7-day logs + milestones), not projection. Per memory `feedback_strategic_grounding_three_surfaces` and 2026-05-13 incident.

## Related

- `.claude/commands/mika-handsoff.md` — sender-side ceremony; Phase 1's workspace-detection prose is the source-of-truth shape inline-duplicated here.
- `docs/logs/HANDSOFF-CONTRACT.md` — canonical contract this command consumes.
- `mika-platform#86` — companion launcher (`/mika-spawn`) that injects `/mika-onboarding` as its no-args default.
- `mika-platform#85` — this command's implementation ticket.
