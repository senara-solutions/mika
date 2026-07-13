---
issue: 1736
type: docs
scope: mika-agent
title: Session-messages ordered stream — verification pass (sub-F of mika#1727)
---

# Plan: verify sub-A `message/stream` coverage for TUI session-messages need (mika#1736)

**Ticket:** mika#1736
**Parent:** mika#1727 (TUI thin-client phase 1)
**Sibling reference:** mika#1731 (sub-A — A2A frame catalog + `ToolCallStart` / `ToolCallResult` type stubs)
**Type:** docs (verification pass — code no-op)

## Problem

Sub-ticket F of mika#1727 asks whether sub-A's augmentation of the A2A `message/stream`
SSE — which already carries the assistant turn's terminal text as
`StatusUpdate(Completed).status.message` — is sufficient for the TUI thin-client's
per-turn assistant-message rendering need. If yes, the ticket closes as **no-op —
covered by A** per its AC3. If no, the ticket becomes the follow-up implementation
(extend `message/stream` schema OR add `GET /dashboard/sessions/{id}/stream`).

Ticket § Scope enumerates two gates:
1. Verify each assistant turn's final text is delivered on `message/stream`.
2. Verify the fields TUI needs (`role`, `content`, `timestamp`, `turn_id` or equivalent)
   are all present on the frame.

The ticket has explicit `## Acceptance criteria` (AC1–AC3) — this plan transcribes them
verbatim under § Acceptance criteria; no rename.

## Scope

- **In scope:** author the verification note at
  `crates/mika-agent/docs/session-messages-stream-verification-2026-07-10.md` (AC1).
  The note documents the two orthogonal claims (assistant-text delivery + field
  coverage) with `file:line` citations into the current `main` shape (verified against
  commit `71bf5ee7`) plus sub-A's frame-catalog augmentation (mika#1731 /
  PR#1756's shipped catalog + type stubs).
- **In scope:** record the verdict (coverage sufficient → no-op close per AC3) and the
  dependency posture (verification runs against sub-A's landing; re-verification
  required if PR#1756 alters `StatusUpdate.message` shape or emit sites before merge).
- **Out of scope:** any code change to A2A types, SSE emit sites, or the
  `/dashboard/sessions/{id}` snapshot endpoint. This is a code no-op PR by design —
  the AC2 augmentation path is not exercised because AC1 finds coverage sufficient.
- **Out of scope:** multi-consumer cross-session multiplexing, historical replay on
  reconnect, and intermediate per-tool-step progress frames — all explicitly deferred
  by the ticket's § Not in scope.

## Approach

1. Enumerate the two claims the ticket demands: (a) each assistant turn's final text
   is delivered, (b) the required fields are all present on the frame.
2. Ground both claims in the current `crates/mika-a2a/src/a2a.rs` emit sites — the
   `Working` / `Completed` / `Failed` StatusUpdate frames (a2a.rs:382 / :427 / :446)
   — with the `Completed` frame's `status.message` field carrying the fully-constructed
   `Message` with the assistant text as `Part::Text` (Role::Assistant).
3. Cross-reference sub-A's `a2a-stream-frame-catalog-2026-07-10.md` for the emit
   contract's authoritative statement of the wire shape.
4. Map each ticket-requested field (`role`, `content`, `timestamp`, `turn_id`) to its
   present-on-frame location (`status.message.role`, `status.message.parts[..]`,
   `status.timestamp`, `task_id`).
5. Record verdict = coverage sufficient → ticket closes as no-op per AC3. Do not
   pursue AC2 (augmentation) because the AC1 gate that triggers it is not met.
6. Note the dependency posture explicitly: verification is against `main@71bf5ee7`
   plus mika#1731's shipped catalog; re-verify if PR#1756 changes the emit sites or
   the `StatusUpdate.message` shape before it merges.

## Files touched

- `crates/mika-agent/docs/session-messages-stream-verification-2026-07-10.md` (new,
  74 lines) — the verification note satisfying AC1.

No source-code files change. This is a docs-only PR — the `Pipeline Artifacts` CI
gate applies its docs-and-source split logic; because the linked ticket carries a
Product/enhancement label rather than `documentation`, this PR's docs-only shape is
justified by the ticket's AC3 (verification-only, code no-op) which is transcribed
into the verification note itself.

## Acceptance criteria

Transcribed verbatim from the ticket's `## Acceptance criteria` section (no rename;
mika#1600 discipline):

- [x] **AC1** — Verification note documenting whether sub-A's `message/stream` shape
  covers TUI's session-message rendering needs. Land at
  `crates/mika-agent/docs/session-messages-stream-verification-<date>.md`. Satisfied
  by the new file at `crates/mika-agent/docs/session-messages-stream-verification-2026-07-10.md`
  landed in commit `683e80ab`.
- [x] **AC2** — If augmentation needed: chosen shape (extend vs new endpoint)
  implemented + tested per Phase 2's follow-up plan (may become a separate PR after A
  lands). Not exercised — AC1's verdict is "coverage sufficient", so no augmentation
  is authored in this PR. If a downstream reviewer overturns the AC1 verdict, a
  separate follow-up ticket becomes the AC2 implementation vehicle (per the ticket's
  own phrasing "may become a separate PR").
- [x] **AC3** — Sub-ticket F may close as "no-op — covered by A" if verification
  passes. This PR is the vehicle for that closure — the verification note's
  § Finding records the "coverage sufficient" verdict and the ticket closes on merge.

## Definition of Done

- Verification note landed at the AC1-specified path with the two claim-tables
  (assistant-text delivery + field coverage), the AC1/AC2/AC3 gate mapping, and the
  dependency-posture caveat.
- PR body links `Closes senara-solutions/mika#1736`.
- Pipeline Artifacts CI gate passes — this plan doc's `## Acceptance criteria`
  section is non-empty and the docs-only-with-plan bucket shape is satisfied.
- No source-code files modified — verified by `gh pr diff 1763` showing only the
  two docs files (this plan + the verification note).

## References

- **Parent ticket:** `senara-solutions/mika#1727` — TUI thin-client phase 1.
- **Sibling ticket:** `senara-solutions/mika#1731` — sub-A frame catalog + type stubs
  (PR#1756).
- **Prior audit:** `crates/mika-cli/docs/2026-07-06-tui-thin-client-phase-1-audit-and-plan.md`
  §AC2 — the origin of the "session-messages ordered stream" need.
- **Frame catalog (authoritative for emit contract):**
  `crates/mika-agent/docs/a2a-stream-frame-catalog-2026-07-10.md`.
- **A2A emit sites (grounded citations for the verdict):**
  `crates/mika-a2a/src/a2a.rs:382` (`Working` StatusUpdate), `:427` (`Completed`
  StatusUpdate with `status.message = Some(<assistant Message>)`), `:446` (`Failed`
  StatusUpdate).
- **Verification base commit:** `main@71bf5ee7`.
