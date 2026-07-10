# Session-messages ordered stream — verification pass (mika#1736)

**Sub-ticket F** of mika#1727. Verification against the sub-A landing (mika#1731 / PR#1756) that the A2A `message/stream` SSE shape already covers the TUI thin-client's per-turn assistant-message rendering need — closing this ticket as **no-op** per AC3.

## Scope of this verification

TUI's chat pane wants a session-scoped SSE that delivers each new assistant/user message as it lands, so the pane can render in real time without polling the `GET /dashboard/sessions/{id}` snapshot endpoint.

Sub-A (mika#1731) shipped the `message/stream` frame catalog + type stubs for `ToolCallStart` / `ToolCallResult` variants. This ticket verifies whether the pre-existing `message/stream` shape — plus sub-A's catalog documentation — already carries what the TUI needs, or whether a follow-up augmentation is required (AC2 gate).

## Finding — Coverage sufficient

**Verdict**: **YES**, the current `message/stream` shape covers the TUI's per-turn assistant-message rendering need for Phase 1. This ticket closes as no-op.

### Evidence

Two orthogonal claims to verify:

1. **The wire delivers each assistant turn's final text.**
2. **The fields TUI needs — role, content, timestamp, and a per-turn correlator — are all present.**

Both hold on the current `main` shape (verified against commit `71bf5ee7`, augmented by mika#1731's catalog).

#### Claim 1 — assistant text delivery

Every operator message → mika-spirit A2A cycle creates one **task** on the server. The task's lifecycle:

| Emit site | State | `is_final` | `status.message` payload |
|---|---|---|---|
| `a2a.rs:382` | `Working` | `false` | `None` |
| `a2a.rs:427` | `Completed` | `true` | `Some(<assistant response Message>)` |
| `a2a.rs:446` | `Failed` | `true` | `None` |

The `Completed` frame's `status.message` field is set to a fully-constructed A2A `Message` carrying the terminal assistant text as `Part::Text`. This is emitted verbatim on the SSE stream via `StreamEvent::StatusUpdate(TaskStatusUpdateEvent { status: TaskStatus { message: Some(...), ... }, ... })`. Source of truth for the emit contract: mika#1731's frame catalog (`crates/mika-agent/docs/a2a-stream-frame-catalog-2026-07-10.md`), § 1 "`StatusUpdate` — `"kind": "status-update"`".

**Coverage claim**: TUI's chat pane, subscribed to `message/stream` for a task it just launched via `message/send` or `message/stream`, receives the final assistant text exactly once, on the terminal `StatusUpdate(Completed)` frame. This is the per-turn assistant-message rendering need in Phase 1's scope.

#### Claim 2 — fields present

The ticket enumerates: `role`, `content`, `timestamp`, `turn_id` (or equivalent).

| Requested field | Present in `TaskStatusUpdateEvent`? | Location |
|---|---|---|
| `role` | Yes | `status.message.role` (`Role::Assistant` on the `Completed` frame) |
| `content` | Yes | `status.message.parts` — a `Vec<Part>` with the assistant text as `Part::Text` |
| `timestamp` | Yes | `status.timestamp` — RFC 3339 UTC |
| `turn_id` (or equivalent) | Yes | `task_id` at the frame root — each A2A task = one operator ↔ agent turn |

All four map to existing fields on the shipped shape; no schema change required.

### Ticket-defined gates

- **AC1** (verification note) — this document.
- **AC2** (augmentation if needed) — not needed; coverage sufficient. Skipped.
- **AC3** (close as no-op) — this ticket closes as "no-op — covered by A".

## Nuances and explicit non-goals

- **User-message echo across consumers.** If TUI A sends a message via `message/stream` and TUI B (a second client) needs to see the user message on their pane in real time, `message/stream` does NOT cover this — each task's broadcast channel is opened by the requesting client. This is **out of scope for Phase 1** per the ticket "Not in scope" list; multi-consumer cross-session multiplexing is deferred.
- **Historical replay on reconnect.** The `GET /dashboard/sessions/{id}` snapshot endpoint remains the historical read surface. `message/stream` is for the live tail of a single task.
- **Intermediate per-tool-step progress.** Covered by sub-A's `ToolCallStart` / `ToolCallResult` variants (types shipped in mika#1731; emission plumbing tracked as sub-A's own follow-up per that ticket's scope reduction). Not this ticket's territory.

## Dependency posture

This verification landed **before** mika#1731 (PR#1756) merged to `main`. The verification is against the state of `main` (commit `71bf5ee7`) plus mika#1731's shipped catalog (frame-catalog doc + type additions on the sub-A branch). If PR#1756 changes the `StatusUpdate.message` shape or the emit sites between now and merge, the finding above must be re-verified before closing mika#1736.

## Cross-links

- **Parent ticket**: `senara-solutions/mika#1727`.
- **Sibling protocol references**:
  - `crates/mika-agent/docs/a2a-stream-frame-catalog-2026-07-10.md` (sub-A, mika#1731) — authoritative frame catalog.
  - `crates/mika-agent/docs/permission-decision-protocol-2026-07-06.md` (sub-C, mika#1733) — sibling SSE surface on `/dashboard/permissions/*`.
  - `crates/mika-agent/docs/ask-user-question-bridge-2026-07-10.md` (sub-D, mika#1734) — sibling SSE variant on the permissions channel.
- **Ratification chain**: this doc's finding is authoritative for closing mika#1736. Any implementer or reviewer who believes the coverage claim is wrong should surface the specific missing field / missing emit site rather than re-litigate the design.
