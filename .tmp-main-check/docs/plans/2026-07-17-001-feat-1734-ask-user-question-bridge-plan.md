---
issue: 1734
type: feat
scope: mika-agent (permissions_stream), mika-a2a (streaming)
title: AskUserQuestion callback bridge — TUI ↔ spirit wire protocol for structured questions
---

# Plan — mika#1734 sub-D: AskUserQuestion callback bridge

## Problem

Sub-D of mika#1727 (TUI thin-client refactor). The agent's `AskUserQuestion` tool needs an operator-facing structured-question surface that pairs with sub-C's permission-decision channel (mika#1733). Without it, `AskUserQuestion` is a placeholder variant on the sub-C SSE stream carrying loose `serde_json::Value` — no answer route, no timeout semantics, no per-question rejection matrix.

TUI can't render structured operator questions until the wire shape is tightened + the sibling POST endpoint (`/answer`) + hold-timeout landed.

## Committed position

**Wire protocol, discriminated-union reuse of the sub-C channel, sibling POST for answers, hold-timeout at the classifier boundary.**

- Tighten the `AskUserQuestion` variant on `PermissionsChannel` from `serde_json::Value` → structured `Vec<AskQuestion { question, options, multiSelect }>`. `deny_unknown_fields` at the answer body.
- Add sibling POST endpoint `/dashboard/permissions/{request_id}/answer` with server-side hold-timeout that materializes `AnswerResult::Timeout { reason: "operator-timeout" }` — not `Deny` (AskUserQuestion has no Approve/Deny semantics).
- Reuse sub-C's SSE stream + auth (bearer via `MIKA_INTERNAL_TOKEN` / `MIKA_DASHBOARD_TOKEN`). No new stream, no fork.
- Sibling storage: `PermissionsChannel` gains a `pending_asks` map alongside the existing `pending` map, because the resolution shape differs (`AnswerResult` vs `OperatorDecision`). Sharing at SSE + auth layer, separate at resolution layer — the F1-style "no wrapper, no seam" discipline from mika#1733.

**Design contract** (verbatim, in-repo): `crates/mika-agent/docs/ask-user-question-bridge-2026-07-10.md`.

### Key design choices (locked)

- **Answer key convention**: question index (`"0"`, `"1"`, …) — stable against text edits inside a single request lifetime, unambiguous when questions duplicate. Rationale in design doc §AC2 tradeoff table.
- **Reuse the permission-decision channel, don't fork** — reduces connection count + auth surface per the ticket contract.
- **Timeout materializes `AnswerResult::Timeout`, not `Deny`** — `AnswerResult` is the correct type for structured-question outcomes; the agent loop that awaits the oneshot interprets it per the claude-pilot cpp#20 joint-2 discipline.

## Scope

### In scope for v1 (this PR)

- New `AskQuestion` struct + `AnswerResult` enum on the shared A2A streaming surface (`crates/mika-a2a/src/streaming.rs`).
- Tightened `AskUserQuestion` variant of `PermissionsChannel`'s outbound-event enum with structured payload + camelCase-on-the-wire.
- New `POST /dashboard/permissions/{request_id}/answer` endpoint (auth: same middleware chain as `/decide`).
- Server-side hold-timeout via `register_pending_ask` spawning `tokio::time::sleep` watcher — race-safe against `resolve_answer`.
- `deny_unknown_fields` on the answer body + runtime coverage/label/extra-key validation. Failed validation preserves the pending entry for retry.
- `Settings.permission_hold_timeout_secs` reused from mika#1733 (no new env var).
- `crates/mika-cli/examples/ask_user_question_stub.rs` — subscribes + canned first-option answers; compile-time shape-check on the local wire mirror.
- 24 `permissions_stream` unit tests (13 pre-existing from mika#1733 + 11 new for #1734).

### Deferred / out of scope

- **TUI rendering** of the wire — separate sub-ticket in the mika#1727 milestone; this PR ships the *bridge*, not the *client*.
- **Multi-operator scoping** of answers — the current design assumes single operator per session; multi-operator answering (voting, first-wins, quorum) is a follow-up.
- **Persistence of unanswered questions across restarts** — questions expire on server restart today; durable persistence is a follow-up if the operator-away use case demands it.

## Acceptance criteria

- [x] **AC1** — Structured `Vec<AskQuestion { question, options, multiSelect }>` on the wire, camelCase (`multiSelect`), discriminated with `PermissionRequest` via `serde(tag = "event")`. Serde round-trip test: `AskUserQuestion` with camelCase `multiSelect`.
- [x] **AC2** — `POST /dashboard/permissions/{request_id}/answer` endpoint with `deny_unknown_fields` + runtime coverage/label/extra-key validation. Failed validation preserves the pending entry for retry (does NOT consume it). Rejection matrix tests: missing answer, invalid option label, extra key, unknown request_id.
- [x] **AC3** — `register_pending_ask` spawns a `tokio::time::sleep` watcher; on expiry fires `AnswerResult::Timeout { reason: "operator-timeout" }` — race-safe against `resolve_answer`. Hold-timeout unit test with a 50 ms watcher confirms the shape.
- [x] **AC4** — Same middleware chain as `/decide`: bearer via `MIKA_INTERNAL_TOKEN` or `MIKA_DASHBOARD_TOKEN`. No new auth surface.
- [x] **AC5** — `crates/mika-cli/examples/ask_user_question_stub.rs` compiles under `cargo build --example ask_user_question_stub -p mika-cli` — subscribes + posts the first-option answer for every incoming question. Compile-time shape-check on the local wire mirror (types stay in lockstep).
- [x] **AC6** — 24 `permissions_stream` unit tests pass (13 pre-existing + 11 new): serde round-trip, `PermissionRequest` + `AskUserQuestion` co-existence on the same broadcast, happy-path answer routing to classifier oneshot, rejection matrix, hold-timeout materializing, `peek_pending_ask` returning cloned snapshot without consuming.

## Definition of Done

- All acceptance criteria satisfied where marked in-scope for this PR (AC1-AC6).
- `cargo build --workspace` clean, `cargo clippy --workspace --all-targets -- -D warnings` clean, `cargo test -p mika-agent --lib permissions_stream` 24/24 pass.
- Design contract doc `crates/mika-agent/docs/ask-user-question-bridge-2026-07-10.md` committed in-repo (in-tree; not tracked separately from this PR).
- `cargo build --example ask_user_question_stub -p mika-cli` succeeds — the CLI example builds against the shared A2A types.
- Full `cargo test --workspace` runs in CI (may be intermittent due to substrate disk pressure that affected mika#1760 — the AC-relevant tests directly cover the shipped behavior).

## Files touched

- `crates/mika-a2a/src/streaming.rs` — `AskQuestion` + `AnswerResult` types (shared surface)
- `crates/mika-agent/src/server/permissions_stream.rs` — `pending_asks` map, `register_pending_ask`, hold-timeout, tightened variant
- `crates/mika-agent/src/server/handlers.rs` — `/answer` handler
- `crates/mika-agent/src/server/a2a.rs`, `crates/mika-agent/src/server/mod.rs` — wiring
- `crates/mika-cli/examples/ask_user_question_stub.rs` — CLI-side wire mirror + example
- `crates/mika-agent/docs/ask-user-question-bridge-2026-07-10.md` — design contract
- `crates/mika-agent/docs/a2a-stream-frame-catalog-2026-07-10.md` — updated frame catalog
- `crates/mika-agent/docs/architecture.md`, `docs/runtime-structure.md` — cross-refs updated
- Various in-workspace docs updates (session-messages-stream-verification, etc.) — hitchhikers from the sibling substrate work

## Verification

```bash
cargo build --workspace                                                      # clean
cargo clippy --workspace --all-targets -- -D warnings                        # clean
cargo test -p mika-agent --lib permissions_stream                            # 24/24
cargo build --example ask_user_question_stub -p mika-cli                     # clean
```

## References

- **Parent milestone**: mika#1727 (TUI thin-client refactor)
- **Sibling sub-C (base)**: mika#1733 / PR#1760 (permission-decision channel — merged; provides the shared SSE stream + auth surface + `Settings.permission_hold_timeout_secs`)
- **Related sub-A**: mika#1731 (`message/stream` catalog + type stubs)
- **claude-pilot joint-2 discipline** (cpp#20) — the timeout materialization pattern this PR mirrors on the answer side
- **Cascade recovery note**: this PR was originally #1762 (base branch `feat/1733/...` since #1760 was stacked on it). When #1760 merged 2026-07-13, GitHub auto-closed #1762. Rebased onto main; original commit `31058d4a` replayed as `d7577e3c` and reopened as #1777.

Plan: docs/plans/2026-07-17-001-feat-1734-ask-user-question-bridge-plan.md
