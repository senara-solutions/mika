---
title: "feat(tui): TUI as thin HTTP client of mika-spirit — Phase 1 audit + sub-issue fan-out"
issue: mika#1727
type: feat
status: done
authored: 2026-07-06
finalized: 2026-07-10
plan_seq: 2026-07-06-001
---

# feat(tui): TUI as thin HTTP client of mika-spirit

Implementation plan for `senara-solutions/mika#1727`.

## Scope framing (read first)

mika#1727 is an **audit ticket**, not a code-refactor ticket. Its five acceptance
criteria (AC1–AC5) are *documents and decisions*, plus a **sub-issue fan-out** for the
spirit-side API gaps the audit surfaces. The actual TUI refactor — deleting standalone
mode, flipping crate visibility so `AgentLoop`/`ToolExecutor`/`SessionManager` cannot be
constructed in `mika-cli` — lands in a **separate closing PR** *after* the fan-out
sub-tickets (A–G) clear. This plan therefore delivers:

1. A landed Phase 1 audit document (AC1–AC5).
2. A filed set of Phase 2 sub-issues (the gap-fill work), each a sub-issue of mika#1727.
3. The explicit decisions the ticket demands (standalone disposition; structural-enforcement shape).

**What this plan does NOT do:** touch any source file under `crates/mika-cli/src/` or
`crates/mika-agent/src/`. The refactor is deferred by design — Prime's discipline is that
"missing spirit endpoints from the audit become individual follow-up sub-issues, NOT
bundled into this refactor."

## Current state (grounded, 2026-07-06)

An audit doc is **already committed on this branch** at
`crates/mika-cli/docs/2026-07-06-tui-thin-client-phase-1-audit-and-plan.md`
(commit `b3f4b4fe`, 213 lines). It establishes findings F1–F4, the AC1 duplication
inventory, the AC2 gap catalog, the AC3 wrapper-doctrine subsection, the AC4 standalone
disposition (recommend delete), and the AC5 structural-enforcement sketch. This plan
**finalizes** that doc and drives the sub-issue fan-out — it is the idempotent re-groom
continuation, not a from-scratch author.

Structural claims re-verified during this grooming pass (all hold except one correction):

- **F1 holds** — `mika-spirit` is a binary of `mika-agent` (`crates/mika-agent/src/bin/mika-spirit.rs`), not a separate crate. The refactor is a crate-visibility flip, not a new-crate creation.
- **F2 holds** — `crates/mika-cli/Cargo.toml` declares `mika-agent.workspace = true` alongside `mika-common` and `mika-a2a`. That in-process dependency IS the standalone surface.
- **F3 holds** — `crates/mika-cli/src/remote_ask.rs` exists; the A2A thin-client precedent is real.
- **F4 holds** — `crates/mika-a2a/src/client.rs` exists; spirit's HTTP+A2A surface is rich. `commands/chat.rs` has 11 `mika_agent::` use-sites, `commands/ask.rs` has 9.
- **CORRECTION to the audit's `/healthz` gap** — spirit **already exposes `/health`** (`crates/mika-agent/src/server/mod.rs:317`, deliberately *outside* the auth layer for probes). The audit doc's sub-ticket E ("add `/healthz`") is therefore either a no-op or a thin alias/rename, not net-new endpoint work. **The finalized audit doc must correct this** so E is not filed as substantive gap-fill work.

## Requirements

### R1 — Finalize the Phase 1 audit document (AC1–AC5)

The committed audit doc is marked `status: audit-in-progress`. Finalize it:

- **R1.1** — Correct the `/health` finding (see CORRECTION above). Sub-ticket E is downgraded to "verify `/health` covers TUI liveness needs; add `/healthz` alias only if a probe contract requires the exact path" — a verification, not an endpoint build.
- **R1.2** — Flip `status: audit-in-progress` → `status: audit-complete` (or `landed`) once R1.1 and the sub-issue fan-out (R2) are done. The status field is the AC1/AC2 "landed" signal.
- **R1.3** — The doc already contains AC3 (wrapper-doctrine subsection), AC4 (standalone disposition = delete, with rationale), and the AC5 structural-enforcement sketch (Option A crate-visibility flip vs Option B `mika-agent-core` extraction). No new authoring needed there — confirm they read cleanly against the corrected F4.

### R2 — File the Phase 2 sub-issues (the fan-out)

Per the audit's A–G list, file each as a sub-issue of mika#1727 on `senara-solutions/mika`.
Each carries its own ACs and `Refs: mika#1727` (parent link via GitHub sub-issue relation
where the tooling supports it, else `Parent: mika#1727` in the body). After the `/health`
correction, the fan-out is:

| ID | Title | Substance | Notes |
|----|-------|-----------|-------|
| A | Verify + augment A2A `message/stream` to carry `tool_call_start`/`tool_call_result` SSE frames | Substantive — TUI "running Bash: X" rendering depends on it | Verify-first; augment only if text-only today |
| B | New SSE task-event live stream for TUI status pane | Substantive — `/dashboard/tasks` is snapshot-only | New endpoint |
| C | Permission-decision request stream (spirit-defers-to-operator approval) | **Largest** — correlated request/response over SSE + POST, mirrors claude-pilot `canUseTool` | New wire protocol |
| D | AskUserQuestion callback bridge | Likely combined with C (discriminated event type on same channel) | May fold into C |
| E | ~~Add `/healthz`~~ → Verify `/health` covers TUI liveness | **Downgraded** — `/health` already exists | Verification, not build |
| F | Session-messages ordered SSE stream for TUI message pane | May be subsumed by A's augmentation of `message/stream` | Verify against A first |
| G | MCP server-management boundary decision (CLI edit-time vs spirit runtime) | Doctrine call + implementation | Depends on the boundary decision, not just code |

**Filing discipline (hard rule from CLAUDE.md):** file only with concrete scope. A, B, C
are the substantive net-new-API tickets. D likely folds into C. E is a verification. F is
a verify-then-maybe. G is a decision-first ticket. The plan's recommendation: **file A, B,
C, G as standalone sub-issues; fold D into C's body as a second event type; write E and F
as verification checklist items inside sub-issue A's body** rather than separate tickets,
to avoid the speculative-ticket noise CLAUDE.md's filing discipline forbids. (Implementer
may split if scope grows.)

### R3 — Record the two load-bearing decisions explicitly (already in the doc; confirm)

- **AC4 decision:** delete standalone mode. Rationale carried in the doc (doctrine + F3 precedent + deploy shape + AC5 dependency). No open question — confirm the doc states it as a *decision*, not a *recommendation*, for the closing-PR implementer to execute.
- **AC5 shape:** Option A (flip `agent_loop`/`tool_execution`/session-manager types to `pub(crate)` in `mika-agent`) first; migrate to Option B (`mika-agent-core` crate split) only if semver cleanliness demands. The closing PR owns the final call after the fan-out lands.

## Non-goals (explicit, to bound the closing PR)

- No deletion of `mika_agent::agent::run_agent` call sites in `chat.rs`/`ask.rs` — that is the closing PR.
- No crate-visibility changes to `mika-agent` — closing PR, after fan-out.
- No new SSE/HTTP endpoints in `mika-agent/src/server/` — those are the A/B/C sub-tickets.
- No `Cargo.toml` dependency surgery on `mika-cli` — closing PR.

## Verification Contract

Because this ticket lands **documents + issues**, not code, verification is document-and-tracker-shaped:

- **V1** — `crates/mika-cli/docs/2026-07-06-tui-thin-client-phase-1-audit-and-plan.md` exists on the branch, `status:` is no longer `audit-in-progress`, and the `/health` correction (R1.1) is present. Grep check: the doc must NOT claim spirit "lacks a `/healthz` endpoint" without the correcting note that `/health` exists.
- **V2** — Sub-issues A, B, C, G exist on `senara-solutions/mika`, each linked to mika#1727 as parent/sub-issue, each with at least one testable AC in its body. Verify via `gh issue list --repo senara-solutions/mika --search "parent:1727"` or the sub-issue tracker.
- **V3** — The audit doc's AC1 table and AC2 gap list are internally consistent with the filed sub-issues (every substantive gap maps to a filed ticket or an explicit "folded into X" / "verification item" note).
- **V4** — `cargo check` / `cargo build` is UNCHANGED (no source touched) — a green build is the proof the plan honored its non-goals. No test additions expected.
- **V5** — The AC5 structural-enforcement property is stated as a *future acceptance test* in the doc (the `error[E0603]: module 'agent_loop' is private` compile-error demonstration), not asserted as already-landed.

## Definition of Done

- [x] Audit doc finalized: `/health` correction applied, `status` flipped off `audit-in-progress` (→ `audit-complete`), AC1–AC5 confirmed present and consistent (R1). Per-file line-by-line inventory added on 2026-07-10 (the "Phase 1 follow-up work" the original doc named).
- [x] Sub-issues A, B, C, D, F, G filed on `senara-solutions/mika` as sub-issues of mika#1727, each with testable ACs. Filed as separate sub-tickets rather than folding D into C — the discriminated-union pattern shipped in PR#1762 kept D structurally cheap. E filed as a small verification ticket rather than a verification item inside A's body (per-ticket tracking cleaner than nested checklist item).
- [x] AC4 (delete standalone) and AC5 (Option A first) recorded as explicit decisions in the audit doc §AC4 and §AC5 (R3).
- [x] No source files under `crates/*/src/` modified in the audit-and-plan PR. Refactor is Phase 2, gated on sub-ticket merges.
- [x] Sub-ticket landing status table added to the audit doc on 2026-07-10 with current PR#s; fan-out shape amendment (D not folded into C, E filed separately) reconciled there.

## Weekend loop-stall evidence

Recorded during the 2026-07-10 finalization pass, per Vincent's directive to "fold in the weekend loop-stall evidence — the drain proved live exactly why the daemon-agent architecture is needed." Full narrative lives in the audit doc's dedicated section; summary here: the mika#1741 → PR#1760 → PR#1762 → PR#1763 → PR#1764 drain landed only because orchestrator-CC ran the pipeline directly in-session via `/mika` inline execution or the direct-implement anti-stall fallback. The autonomous `ready`-label dispatch path stalled repeatedly across the weekend. The recovery-shape (interactive session as thin HTTP consumer of a spirit that already owns everything primitive) is exactly the closing PR's target shape — this is not aspirational; it is observed behavior of the substrate under load.

## Acceptance criteria

Transcribed from `senara-solutions/mika#1727`:

- **AC1 — Duplication inventory landed.** A document in `crates/mika-cli/docs/` (or `docs/architecture/`) enumerates every module in `mika-cli` that also lives in `mika-agent`, with file paths + a brief note on what each duplicates. Comprehensive, not spot-check.
- **AC2 — Spirit-side gap document landed.** A companion document (or a section of AC1's doc) enumerates the HTTP+SSE endpoints `mika-spirit` needs to expose for the TUI to become a thin client, cross-referenced against `crates/mika-agent`'s current Axum route table. Missing endpoints become individual follow-up tickets (sub-issues of mika#1727, NOT bundled into the refactor).
- **AC3 — Wrapper-doctrine test in refactor plan.** The refactor's plan document contains a first-class doctrine subsection: "TUI must not contain business logic mika-spirit already owns; every state read + every side effect goes through spirit's HTTP API." Applied to any proposed TUI feature going forward, same as D4's cm#66 wrapper-doctrine AC.
- **AC4 — Standalone-mode disposition decided.** The plan states, with rationale, either (a) keep standalone as a fallback/offline mode (at the cost of continued duplication — rejected by the doctrine), or (b) delete standalone mode entirely so the TUI requires a running `mika-spirit`. **Decision recorded: (b) delete.**
- **AC5 — Structural enforcement, not prompt-level.** After the refactor lands, the type system in `crates/mika-cli` must NOT be able to construct an `AgentLoop`, a `ToolExecutor`, or a `SessionManager` locally — those types only exist behind an `HttpSpiritClient` (or equivalent). If it compiles to a valid TUI without spirit, the AC fails. The doc records this as a future acceptance test (compile-error demonstration), scoped to the closing PR.

## Notes for the closing-PR implementer

The refactor that actually satisfies AC5's compile-error property is the sequel to this
ticket. It depends on sub-issues A, B, C (and G's boundary decision) landing first. When
it runs, its shape is already sketched in the audit doc §"What lands in the refactor PR
that closes mika#1727" (steps 1–7): delete `run_agent` consumption, rewire `chat.rs` onto
`mika_a2a::client::A2aClient`, wire permission-decision requests into the existing
approval-prompt UI, reduce the `mika-agent` dependency to wire-types-only, apply Option A
(`pub(crate)` visibility flip) to make standalone compilation impossible, delete
unreachable paths. Net expected: ~500–1500 lines deleted, ~200–400 added.
