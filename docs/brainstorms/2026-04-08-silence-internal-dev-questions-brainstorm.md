# Brainstorm: Silence Internal Dev Questions in the TUI

**Date:** 2026-04-08
**Status:** Captured — ready for planning

## What We're Building

A way to keep agent-to-agent "dev" questions out of the TUI's main view while preserving full threading in the audit log. The TUI becomes an **escalation-only inbox** for the human user: only questions actually addressed to a person show up. Internal chatter (mika-dev ↔ mika-qa, planners consulting reviewers, etc.) still happens, still threads correctly, but lives in the audit stream — not in the human's field of view.

## Why

The core problem is **wrong audience**, not volume. Today the TUI shows every question regardless of who it's really for. When mika-dev asks mika-qa something, the human sees it and has to mentally filter it out. That breaks the illusion of Mika as a coworker delivering outcomes and turns the TUI into a debug console.

The desired perspective is **escalation-only**: the TUI should behave like an inbox, quiet until something actually needs the human. Internal deliberation should be observable on demand, not broadcast by default.

## Approach: A + C (tag at source, filter at view)

**A — `internal: true` flag on questions (data model).**
Questions carry an opt-in boolean marking them as agent-to-agent. The asking agent decides intent at the moment of asking. Any consumer (TUI, audit log, future dashboards) can filter on this field without heuristics.

**C — TUI inbox mode (view layer).**
The TUI filters out `internal: true` questions by default, rendering only human-addressed ones. A mode toggle lets the user drop into audit/observe mode to peek at internal chatter when curious — no separate tab, no collapse UI, no routing changes.

### Why this combination

- **A alone** fixes semantics at the source but still leaves the TUI rendering everything by default.
- **C alone** would need heuristics to guess which questions are internal — fragile and leak-prone.
- **A + C** is the clean separation: data model reflects intent, view layer consumes it. KISS + orthogonality.
- Rejected **B (explicit `audience: human|agent`)**: redundant once A exists — `internal: true` already implies audience. Bigger migration for no extra signal.
- Rejected **separate dev view/tab**: YAGNI. A toggle on the existing view is simpler.
- Rejected **collapsed-but-threaded**: still visually present, still noise.

## Key Decisions

1. **Silencing is opt-in at ask-time**, not inferred. Agents tag questions as `internal` when the intended audience is another agent.
2. **Internal questions still thread normally** in the audit log — nothing is dropped or rerouted, only hidden from the default TUI view.
3. **TUI default is inbox mode** (escalation-only). Audit/observe mode is available via toggle.
4. **No schema for `audience`** — a single boolean is enough for the first cut.
5. **Miscalibration is acceptable risk.** Agents will occasionally mis-tag; we'll tune via audits rather than policing at the schema level.

## Open Questions

- **Tag default:** Should `internal` default to `false` (safer — nothing hidden unless explicitly marked) or `true` for questions originating from specific agents like mika-dev/mika-qa (less noise out of the gate)?
- **Escalation path:** If an internal question times out or the responder agent is unavailable, should it auto-promote to visible so the human can step in?
- **Audit mode affordance:** How does the user know there are hidden internal questions? Silent counter in the status bar? Periodic summary? Nothing at all?
- **Retroactive tagging:** Do existing in-flight questions get migrated, or does the flag only apply to new ones?

## Next

Resolve the open questions (particularly default behavior and escalation path), then `/ce:plan` to design the schema change, TUI filter, and mode toggle.
