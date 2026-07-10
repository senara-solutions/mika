---
ticket: mika#1731
branch: feat/1731/verify-augment-a2a-message-stream-sse-to
type: feat
scope: crates/mika-a2a, crates/mika-agent
grooming: /mika-groom-ticket
parent: mika#1727 (Phase 1 audit sub-ticket A)
---

# Plan — mika#1731 augment a2a `message/stream` SSE with tool-call events

## Problem

Sub-issue of mika#1727 (TUI as thin HTTP client of mika-spirit). The TUI's chat surface needs fine-grained tool-call visibility ("running Bash: git status", "finished with output …") — not just the terminal assistant-turn text. The Phase 1 audit (`crates/mika-cli/docs/2026-07-06-tui-thin-client-phase-1-audit-and-plan.md` §AC2) flagged `message/stream` as "present but likely insufficient" and forked sub-ticket A (this ticket) to verify + augment.

## Verification result

**Verified — augmentation is required.** The current `message/stream` SSE emits only three lifecycle frames — all `StatusUpdate`, states `Working` → `Completed | Failed` — from `crates/mika-agent/src/server/a2a.rs::handle_message_stream` (lines 382, 427, 446). There is:

- **Zero tool-call plumbing end-to-end.** `ToolCallSummary` exists (`crates/mika-agent/src/tool_execution/types.rs:7`) but is post-facto only, written to SQLite and to `messages.metadata`. No callback trait, no channel, no `tx.send(...)` between `process_tool_calls` and the `broadcast::Sender<StreamEvent>` captured by the spawned SSE task.
- **Zero client-side SSE parse** in the production consumer path. `crates/mika-cli/src/remote_ask.rs` calls `message/send` (non-streaming). The dead-code `A2aClient::send_message_streaming` exists but is unused.
- **No `#[serde(other)]` catch-all** on `StreamEvent` — a new variant would parse-fail on any strict consumer that shipped between now and the client refactor.
- **No integration test** for `message/stream`. Existing tests are enum round-trip in `crates/mika-a2a/src/streaming.rs:51-212` only.
- **Sibling SSE surface exists** — `PermissionStreamFrame` shipped in mika#1741 (`crates/mika-agent/src/server/permissions_stream.rs`) with its own tagged enum on a different route. The audit correctly recommends NOT unifying the two enums (three axes of deliberate divergence: discriminator key `kind` vs `event`, per-task vs per-process broadcast, JSON-RPC-inline vs dedicated GET route).

## Scope

### In scope for v1 (this PR)

**AC1 — Verification note.** Land `crates/mika-agent/docs/a2a-stream-frame-catalog-2026-07-10.md` enumerating every `StreamEvent` variant emitted by `message/stream` today. Include variant name, `kind` tag value, field names + types, and exact emit-site (file:line). Cover the three currently-emitted frames (Working, Completed, Failed) plus dead-emit paths noted for reader completeness (`Task`, `Message`, `ArtifactUpdate` are defined but never broadcast by `message/stream`).

**AC2 — New tool-call variants.** Add two variants to `mika_a2a::streaming::StreamEvent` (`crates/mika-a2a/src/streaming.rs:8`):

```rust
StreamEvent::ToolCallStart {
    task_id: String,
    context_id: Option<String>,
    step: u32,
    tool_name: String,
    args_summary: String,      // truncated JSON preview, cap 500 chars
    timestamp: chrono::DateTime<chrono::Utc>,
}
StreamEvent::ToolCallResult {
    task_id: String,
    context_id: Option<String>,
    step: u32,
    tool_name: String,
    success: bool,
    non_zero_exit: bool,
    output_summary: String,    // truncated preview, cap 500 chars
    duration_ms: u64,
    timestamp: chrono::DateTime<chrono::Utc>,
}
```

Serde tags: `kind = "tool-call-start"` and `kind = "tool-call-result"` (kebab-case, matches existing `"status-update"` / `"artifact-update"` convention already established in `StreamEvent`).

**AC3 — Forward-compat catch-all.** Add `#[serde(other)]` on `StreamEvent` as `Unknown` variant (or reserve a catch-all pattern by another means if the serde version doesn't allow `other` on tagged enums — see §Implementation guardrails for the exact shape). Existing consumers must deserialize an unknown-`kind` frame without erroring. This is defensive scaffolding: mika#1741 already ships a divergent SSE enum, and future sub-tickets (mika#1732 task-event, mika#1734 AskUserQuestion bridge) will add more variants; the catch-all keeps the wire additive without a client-refactor gate.

**AC4 — Plumbing: agent → SSE broadcast.** Introduce a new field on `AgentParams`:

```rust
pub struct AgentParams {
    // ... existing fields
    /// Optional broadcast sender for tool-call events. When Some(_), the agent
    /// loop emits StreamEvent::ToolCallStart / ToolCallResult before/after each
    /// tool call. None disables emission (default for non-A2A callers).
    pub stream_tx: Option<Arc<tokio::sync::broadcast::Sender<StreamEvent>>>,
}
```

`handle_message_stream` passes the same `Arc<broadcast::Sender>` it already creates (`a2a.rs:359`) via `AgentParams::stream_tx = Some(broadcaster.clone())`. All other callers (conversation, silent, team, delegate, CLI ask/chat) pass `None` — no behavioral change.

Emission points: in `process_tool_calls` (`crates/mika-agent/src/tool_execution/dispatch.rs:43`) — emit `ToolCallStart` before dispatching each tool, `ToolCallResult` after. Fields populated from the same data already threaded through `execute_tool()` and `ToolCallSummary`. `send()` on a `broadcast::Sender` is fire-and-forget: log at `debug!` on `Err(broadcast::SendError)` (no active subscribers) and continue — never fail the tool call because a subscriber dropped.

**AC5 — Truncation policy.** `args_summary` and `output_summary` capped at 500 chars. Truncation via UTF-8-safe boundary (existing helper `truncate_for_log` in `messaging.rs:250` already handles this — reuse). Rationale: dashboard `tool_calls` table caps input/output at 50 KB per field for persistence; the SSE preview is for real-time rendering, not audit — 500 chars is enough for "running Bash: git status" or "output: 47 lines". Reader can fetch the full call via the existing `GET /api/v1/traces/:trace_id/tool-calls` if needed.

**AC6 — Integration test.** New file `crates/mika-agent/tests/a2a_message_stream.rs`. Uses `EvalHarness` (`MockLlmProvider`) to spin up an agent turn that calls one tool (a builtin like `store_fact` — deterministic, no network), and asserts the SSE frame sequence includes at least: `StatusUpdate(Working)` → `ToolCallStart` → `ToolCallResult` → `StatusUpdate(Completed)`. Assertions on `kind` tags and field presence (not exact content — the mock output isn't the SUT). Covers the primary regression class ("did we forget to emit ToolCallResult on error?" or "did the ordering flip?").

**AC7 — Docs update.** `docs/architecture.md` §14 gains a "SSE frame catalog" subsection (or a dedicated §14.2) that lists all `StreamEvent` variants with `kind` tag, purpose, and cross-reference to the sibling `PermissionStreamFrame` (mika#1741). The verification note from AC1 lives in `crates/mika-agent/docs/` as the detailed reference; architecture.md carries the summary + cross-reference. Both are updated together and doc-synced via `scripts/sync-agent-docs.sh` so `crates/mika-agent/docs/architecture.md` matches. Rationale: the audit flagged MEDIUM-HIGH drift risk because §14 today only lists module names, not variants — every new variant (mika#1732, mika#1734, mika#1736, etc.) will silently pass any doc lint without this section.

**AC8 — Backwards compatibility.** Existing consumers (`Task`, `StatusUpdate`, `ArtifactUpdate`, `Message` variants) unchanged. Two new variants ship as additive; no existing field renamed or removed. `remote_ask.rs` (`message/send`-only) is unaffected. Dead-code `A2aClient::send_message_streaming` gains the two new variants automatically via `serde` derivation — no signature change.

**AC9 — Build + lint clean.**
- `cargo build -p mika-a2a -p mika-agent`
- `cargo test -p mika-a2a` (existing enum round-trip tests + new variant round-trip)
- `cargo test -p mika-agent --test a2a_message_stream` (new integration test)
- `cargo clippy -p mika-a2a -p mika-agent --all-targets -- -D warnings`

### Out of scope for v1 (deferred)

- **TUI rendering** — mika#1727 closing PR consumes these frames. This ticket ships the wire only.
- **`remote_ask.rs` migration to `message/stream`** — the current `message/send` call path is retained. Migration is mika#1727's job (client-side refactor).
- **Refactoring `PermissionStreamFrame` into a shared enum** — three axes of deliberate divergence (audit §6). Do not conflate.
- **`GET /api/v1/dashboard/sessions/{id}/messages` streaming** — the audit flagged this as a candidate for absorption into `message/stream` (see §Note-1 in verification), but it's a distinct SSE surface (dashboard route, not A2A). Deferred to mika#1736 (session-messages ordered stream).
- **Per-tool-call trace-id propagation** — the `trace_id` field is available inside `ToolContext` but not currently on `StreamEvent` frames. If the TUI later needs to hyperlink into the observability dashboard's tool-call detail, add `trace_id: Option<String>` in a future PR. Not v1 (YAGNI).
- **Rate limiting / backpressure on the broadcast channel** — the channel is bounded at 32 (a2a.rs:359). If a tool call storm exceeds 32 pending frames, the oldest are dropped by `tokio::broadcast` semantics. Acceptable for v1 (subscriber's fault to fall behind); revisit if the TUI reports gaps.

### Explicit non-goals for this PR

- No change to `agent_loop` / tool-execution internals — this ticket only shapes the OUTBOUND SSE and adds a fire-and-forget emit call.
- No cross-tenant streaming or authorization — same as parent ticket §Not in scope.

## Implementation guardrails

### File and function targets

| Change | File | Location |
|---|---|---|
| Add `ToolCallStart` + `ToolCallResult` variants | `crates/mika-a2a/src/streaming.rs` | Enum at line 8-18 |
| Add `#[serde(other)]` catch-all | `crates/mika-a2a/src/streaming.rs` | Same enum. Note: if serde's `tag = "kind"` + `other` combo is unsupported by the pinned serde version, use `#[serde(untagged)] Unknown(serde_json::Value)` as a final variant instead — see §serde compat below |
| Add unit tests for both new variants | `crates/mika-a2a/src/streaming.rs` (tests mod) | After existing round-trip tests |
| Add `stream_tx` field to `AgentParams` | `crates/mika-agent/src/agent.rs` (or wherever `AgentParams` is defined) | Struct definition |
| Populate `stream_tx = Some(broadcaster.clone())` on A2A path | `crates/mika-agent/src/server/a2a.rs` | `handle_message_stream` around line 395 (call to `run_a2a_agent`) |
| Emit `ToolCallStart` before tool dispatch | `crates/mika-agent/src/tool_execution/dispatch.rs` | Inside `process_tool_calls` at line 43+ |
| Emit `ToolCallResult` after tool dispatch | Same | After each `execute_tool()` returns |
| Verification note | `crates/mika-agent/docs/a2a-stream-frame-catalog-2026-07-10.md` | New file |
| Architecture doc §14.2 | `docs/architecture.md` + `crates/mika-agent/docs/architecture.md` (via sync script) | New subsection after §14 |
| Integration test | `crates/mika-agent/tests/a2a_message_stream.rs` | New file |

### serde compat for `#[serde(other)]`

The `#[serde(other)]` attribute is supported on internally-tagged enums in serde ≥1.0.130 (checked in `Cargo.toml`). If for any reason the pinned version doesn't accept it on `#[serde(tag = "kind")]`, the fallback is:

```rust
#[serde(untagged)]
Unknown(serde_json::Value),
```

as a final variant. The catch-all sits AFTER all named variants so it only fires on non-match — behaviorally equivalent for forward-compat purposes. Prefer `#[serde(other)]` for clarity; fall back only if compile fails.

### Broadcast emission discipline

- `stream_tx.send(event)` returns `Result<usize, SendError>`. `Ok(n)` where `n` is subscriber count; `Err` when zero subscribers. **Do not fail the tool call on either outcome.** Log at `debug!` on error, continue.
- Ordering: emit `ToolCallStart` immediately before calling `execute_tool()`, `ToolCallResult` immediately after `execute_tool()` returns (before any per-tool-call persistence). The `step` field increments per-tool-call within a turn (matches existing `ToolCallSummary.step`).
- Do NOT emit on tool calls the LLM emitted but the engine deduplicated via the #582 per-turn dedup guard (`process_tool_calls` uses a cached `ToolOutput`). The user's TUI shouldn't see two Starts and only one Result. Emit only on the physically-dispatched call; dedup replays get no frame.

### Truncation helper

Reuse `crate::messaging::truncate_for_log(text: &str, max: usize) -> String` (or promote to `crate::text::truncate_utf8_safe` if it's currently pub(super)). If moving is out of budget, inline a small helper in `dispatch.rs`. Do NOT reinvent UTF-8-safe truncation — the byte-slice-lint script (`scripts/check-byte-slices.sh`) will fail CI on naive slicing (mika#764).

### Backwards compatibility contract

- `StreamEvent` gains variants; no existing variant renamed, reordered, or removed.
- `AgentParams` gains an optional field; all existing struct-literal call sites must be updated to include `stream_tx: None`. Prefer `#[derive(Default)]` or a builder if there are many sites — grep before deciding.
- SSE frame body format unchanged: single JSON object per `data:` line, `kind` discriminator inside.
- No route change, no auth change, no HTTP-level behavior change.

### Test coverage

Beyond the AC6 integration test:

- Serde round-trip for `ToolCallStart` / `ToolCallResult` in `crates/mika-a2a/src/streaming.rs` tests mod — parallel to the existing `StatusUpdate` round-trip.
- Unknown-variant deserialization test asserting `#[serde(other)]` fallback consumes an arbitrary `kind` value without panicking.

## Acceptance criteria

**AC1.** `crates/mika-agent/docs/a2a-stream-frame-catalog-2026-07-10.md` exists, enumerates every currently-emitted `StreamEvent` variant with `kind` tag, fields, types, and file:line emit-site.

**AC2.** `mika_a2a::streaming::StreamEvent` gains `ToolCallStart` and `ToolCallResult` variants with the exact fields specified above. Serde tags are `"tool-call-start"` and `"tool-call-result"`. Serde round-trip unit tests pass.

**AC3.** A forward-compat catch-all (either `#[serde(other)]` or `#[serde(untagged)] Unknown(Value)` fallback per §serde compat) is present on `StreamEvent`. Deserialization of an unknown `kind` value succeeds without panic.

**AC4.** `AgentParams` gains `stream_tx: Option<Arc<broadcast::Sender<StreamEvent>>>`. `handle_message_stream` in `crates/mika-agent/src/server/a2a.rs` populates it with `Some(broadcaster.clone())`. All other callers pass `None`.

**AC5.** `process_tool_calls` in `crates/mika-agent/src/tool_execution/dispatch.rs` emits `ToolCallStart` before each physical tool dispatch and `ToolCallResult` after. Emission is fire-and-forget: broadcast errors are `debug!`-logged and do not fail the tool call. Per-turn dedup replays (`ToolContext` cached `ToolOutput`) do NOT emit frames.

**AC6.** `args_summary` and `output_summary` fields are UTF-8-safe truncated at 500 chars via the existing truncation helper (or an inlined equivalent — no naive byte slicing).

**AC7.** `crates/mika-agent/tests/a2a_message_stream.rs` is a new integration test using `EvalHarness` + `MockLlmProvider`. It asserts a Working → ToolCallStart → ToolCallResult → Completed frame sequence for a tool-invoking turn.

**AC8.** `docs/architecture.md` §14 gains a "SSE frame catalog" subsection (either §14.2 or an inline extension of §14) that enumerates all `StreamEvent` variants and cross-references `PermissionStreamFrame` (mika#1741). Doc sync via `scripts/sync-agent-docs.sh` propagates to `crates/mika-agent/docs/architecture.md`.

**AC9.** `cargo build`, `cargo test -p mika-a2a -p mika-agent`, and `cargo clippy -p mika-a2a -p mika-agent --all-targets -- -D warnings` all pass. Existing `remote_ask.rs` behavior is unchanged.

## Verification steps (post-implementation)

1. `cargo test -p mika-a2a streaming::tests` — new round-trip tests green.
2. `cargo test -p mika-a2a streaming::tests::unknown` (or equivalent) — catch-all test green.
3. `cargo test -p mika-agent --test a2a_message_stream` — integration test green.
4. `cargo clippy -p mika-a2a -p mika-agent --all-targets -- -D warnings` — clean.
5. Manual (documented in PR body): local mika-spirit + `curl` (or `nc`) hitting `POST /a2a/{customer_id}/{agent_name}` with `message/stream` params, observe the SSE frame stream includes `kind:"tool-call-start"` and `kind:"tool-call-result"` for any tool-invoking prompt.

## Rollout

- Merge to `main` → next `make deploy` picks it up (no cluster ops).
- No breaking change; TUI won't consume the new frames until mika#1727 lands, but the wire is in place.
- Watch: grep agent logs for `stream_broadcast_send_no_subscribers` (or similar `debug!` we add) — expected to be common (nobody's listening yet); non-error unless observability suggests otherwise.

## Risks and mitigations

| Risk | Mitigation |
|---|---|
| Broadcast channel (bounded 32) drops frames under a tool-storm turn. | Documented as acceptable v1 behavior (§Out of scope). TUI is not a real-time strict-ordering consumer; visible gaps are tolerable. Revisit with a bounded-lag policy if the TUI reports incorrect renders. |
| Adding `stream_tx: None` to every `AgentParams` call site is tedious. | If N > 10 sites, add `#[derive(Default)]` or a builder to keep the diff surgical. Grep first. |
| `#[serde(other)]` behavior differs across serde versions or with `tag = "kind"`. | Fallback path documented (`#[serde(untagged)] Unknown(Value)`). Compile-check catches the mismatch immediately. |
| New variants collide with a future architecture doc's naming choice. | The audit already established `kind` values as canonical (`"status-update"`, `"artifact-update"`). New kebab-case values (`"tool-call-start"`, `"tool-call-result"`) follow the pattern; no naming lock-in. |
| The integration test relies on `MockLlmProvider` scripted output to trigger a tool call. | Follow the existing eval-harness pattern (`crates/mika-agent/tests/eval.rs`); the mock produces a canned `tool_use` block for a deterministic builtin (`store_fact`, `list_tasks`, etc.). No network. |
| Doc drift on §14 subsection when future sub-tickets add variants. | The subsection ships with an explicit "Cross-reference: `PermissionStreamFrame` at `crates/mika-agent/src/server/permissions_stream.rs`" line. Future sub-ticket authors are prompted to update this subsection; the tests should also catch dropped variants if we assert the catalog matches emitted frames — but that's a doc-testing follow-up, not in v1. |

## Files changed (expected)

- `crates/mika-a2a/src/streaming.rs` — 2 new variants + catch-all + round-trip tests. ~120 lines added.
- `crates/mika-agent/src/agent.rs` (or wherever `AgentParams` lives) — 1 new field + threading. ~10 lines.
- `crates/mika-agent/src/server/a2a.rs` — 1 new line populating `stream_tx = Some(broadcaster.clone())`.
- `crates/mika-agent/src/tool_execution/dispatch.rs` — emission calls in `process_tool_calls`. ~30 lines.
- Update all struct-literal `AgentParams` construction sites to include `stream_tx: None`. Grep-driven.
- `crates/mika-agent/docs/a2a-stream-frame-catalog-2026-07-10.md` — new verification note. ~100 lines.
- `docs/architecture.md` + `crates/mika-agent/docs/architecture.md` — new §14 subsection. ~40 lines.
- `crates/mika-agent/tests/a2a_message_stream.rs` — new integration test. ~150 lines.

Estimated diff: ~500 net lines added.

## Grooming history

- 2026-07-10 — `/ce:plan` draft (with pre-groom verification pass — see §Verification result above).
- 2026-07-10 — `mika-arch` first-pass review (session `8430d9b7-1cd3-4eed-b19c-c0200722d2bf`): **Disposition: READY**. All three uncertainties confirmed: (1) concrete `stream_tx: Option<Arc<broadcast::Sender>>` on AgentParams is correct — YAGNI over `ToolEventSink` trait; refactor to trait only if a second distinct transport emerges. (2) 500-char truncation is right v1 default; TUI can fetch full via `GET /api/v1/traces/:trace_id/tool-calls`; policy constant, not schema constraint. (3) Deferring `remote_ask.rs` migration to mika#1727 is correct scope boundary; integration test validates wire shape without full client migration. No revisions applied — plan is dispatch-ready as-committed.
