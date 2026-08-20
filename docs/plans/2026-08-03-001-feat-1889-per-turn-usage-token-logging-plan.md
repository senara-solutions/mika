# Plan — feat(agent-core): per-turn usage/token logging (RT-005 brick 4/5)

**Ticket:** mika#1889 (RT-005 physics pilot, brick 4/5 — **CRITICAL**; primary outcome = planning-tokens)
**Type:** feat (agent-core, p2-normal) — pure instrumentation, zero behaviour change
**Branch:** `feat/1889/agent-core-per-turn-usage-token-logging`
**Target file:** `crates/mika-agent/src/agent_loop/mod.rs` (one new helper + three call sites, mirroring the mika#1217 `emit_system_prompt_assembled` precedent)

---

## Context

RT-005 (physics pilot, ratified 2026-07-28) needs to measure **planning-tokens** as its single
primary, non-verification outcome. Brick 4/5 (this ticket) is the instrumentation layer that emits
per-turn token `usage` from the agent loop; brick 5/5 is the offline analyzer that consumes it.

### What already exists (and why it is not enough)

The agent loop (`run_loop`, `crates/mika-agent/src/agent_loop/mod.rs:654`) iterates
`for step in 0..max_steps` — each `step` is exactly one LLM call. Every call already returns
`LlmResponse.usage` (`LlmUsage { input_tokens, output_tokens, cache_creation_input_tokens,
cache_read_input_tokens }`, `crates/mika-common/src/llm/types.rs:159`) and the loop **already
persists** it to the `llm_calls` SQLite table via `save_llm_call(...)` — with `step`, `session_id`,
`agent_id`, `trace_id`, `stop_reason`, `latency_ms`, and `system_prompt_bytes`
(`agent_loop/mod.rs:860`).

Two gaps make that persistence insufficient for the RT-005 offline analyzer (brick 5/5):

1. **Gated behind `MIKA_STORE_LLM_CALLS`.** DB persistence of `llm_calls` is skipped entirely when
   `store_llm_calls == false` (the `if store_llm_calls` guard at `agent_loop/mod.rs:845`). RT-005's
   measurement must not silently vanish because an operator (or a test tenant) disabled that flag.
2. **Not a parsable log stream.** The whole platform's offline-analysis convention is grep+jq over
   the JSON server log (`$MIKA_SPIRIT_LOG_FILE`) — see the Signal A–N operator patterns in
   `mika/CLAUDE.md`. Brick 5/5 is a log consumer, not a SQLite client. There is a per-turn structured
   log precedent — `emit_system_prompt_assembled` (mika#1217, `agent_loop/mod.rs:5191`) emits a
   `system_prompt_assembled` INFO event on `target: "mika::otel"` every turn — but it carries only
   *byte* counts, never token `usage` (it fires **before** the LLM call, when usage is unknown).

So the concrete gap is: **no per-turn structured log event carries the LLM token `usage`.** This plan
adds exactly that, mirroring the mika#1217 precedent one-for-one.

### The Prime guardrail this instrumentation must respect

Prime's hard condition #1: the primary outcome is **planning-tokens SEUL**. The other three
candidates (turns, handshakes, recalculs) are **descriptive covariables only**, never folded into the
primary estimand — otherwise the measurement device is "un thermomètre gradué en définition" (a
thermometer graduated in the very definition it is meant to measure).

The instrumentation-layer consequence is decisive and is the single most important design decision
below (D1): **the log event emits RAW per-turn usage plus RAW discriminating dimensions (turn number,
`stop_reason`, mode, whether tools were called in the turn). It does NOT bake a "planning" vs
"execution/verification" label into the log line.** The planning/execution boundary is *defined* by
the offline analyzer (brick 5/5), not pre-graded by the thermometer. What the log must guarantee is
that those raw dimensions are *sufficient* to separate planning tokens from execution/verification
tokens offline (AC bullet 3) — nothing more.

---

## Requirements

- **R1 — Per-turn `turn_usage` log event.** Emit one structured INFO event on `target: "mika::otel"`,
  event name `turn_usage`, per LLM call inside `run_loop`. Carries: `agent_id`, `session_id`,
  `trace_id`, `mode` (`conversation`/`silent agent`/`team agent` via `mode.label()`), `step`
  (0-indexed turn number; `u32::MAX` for a continuation turn), `stop_reason` (`Debug` of
  `LlmStopReason`, or `""`/`error` on the error arm), `input_tokens`, `output_tokens`,
  `cache_read_tokens`, `cache_write_tokens` (`Option<u64>` → `0`), `latency_ms`, and `status`
  (`success`/`error`). Field names mirror the `llm_calls` columns so the offline analyzer can join or
  cross-check the two surfaces.
- **R2 — Ungated by `MIKA_STORE_LLM_CALLS`.** The event fires on **every** LLM call in the loop
  regardless of `store_llm_calls`. The log stream is the RT-005 measurement channel; it must not be
  coupled to the DB-persistence flag (see Context gap 1). This is the deliberate divergence from the
  existing `if store_llm_calls` guard (D2).
- **R3 — Both success and error arms.** Emit on the success arm (real usage from `resp.usage`) **and**
  the error arm (zero tokens, `status = "error"`). An error turn still consumed a turn of wall-clock
  and belongs in the covariable "turns" count; dropping it would silently undercount. Mirrors the
  existing two-arm `save_llm_call` shape at `agent_loop/mod.rs:860`/`887`.
- **R4 — Continuation turn covered.** The max-steps continuation LLM call
  (`save_continuation_llm_call`, `agent_loop/mod.rs:480`) also emits a `turn_usage` event with
  `step = u32::MAX` (the same sentinel the DB path uses). A continuation call consumes real
  planning-class tokens (it summarizes); omitting it would undercount the primary outcome.
- **R5 — Raw dimensions only, no baked classification (Prime guardrail, D1).** The event MUST NOT
  contain a `phase`/`is_planning`/`role` field or any field that pre-classifies the turn as planning
  vs execution/verification. Only raw, mechanically-observed dimensions are emitted. The offline
  analyzer owns the estimand definition.
- **R6 — Zero behaviour change (AC bullet 3).** Instrumentation only: no change to control flow, tool
  execution, guards, deadline handling, DB writes, or return values. Adding a log call is
  side-effect-free with respect to the loop's observable behaviour.

---

## Design decisions

### D1 — Emit raw dimensions, never a baked planning/execution label (Prime hard condition #1)

The event carries `step`, `stop_reason`, `mode`, and (see D3) a `tool_use_in_turn` boolean — the raw
signals from which brick 5/5 *defines* the planning boundary. It carries **no** `phase` or
`is_planning` field. Rationale, verbatim from Prime: baking the classification into the thermometer is
"un thermomètre gradué en définition." The separation of planning tokens from execution/verification
tokens is an offline analytic decision; the instrumentation's only obligation is that the raw
dimensions are *sufficient* for that separation (AC bullet 3). This is the load-bearing correctness
decision in the plan; flagged explicitly for architect review.

Citation: `docs/architecture/review-guide.md` § Orthogonality — the measurement layer and the
estimand-definition layer are separate concerns; coupling them destroys the ability to revise the
definition without re-instrumenting.

### D2 — Ungated by `MIKA_STORE_LLM_CALLS`; decoupled from DB persistence

The existing per-step `save_llm_call` lives inside `if store_llm_calls`. The `turn_usage` event is
placed **outside** that guard so the RT-005 stream survives `MIKA_STORE_LLM_CALLS=false`. The log
event and the DB row are independent observability surfaces with independent lifecycles: the DB row is
for the dashboard's LLM-calls table; the log event is for offline token analysis. Coupling them to one
flag would make the pilot silently unmeasurable under a common ops configuration. Mirrors mika#1217,
where `emit_system_prompt_assembled` also fires independent of `store_llm_calls`.

### D3 — `tool_use_in_turn` boolean for offline planning/execution separation

To satisfy AC bullet 3 ("separate planning tokens from execution/verification tokens") without baking
the classification in (D1), the event carries one extra raw boolean: `tool_use_in_turn` = whether the
LLM response for this turn requested tool calls (`resp.stop_reason == ToolUse` or an EndTurn-with-tool-
calls, i.e. `resp.has_tool_calls()`). This is the mechanical, definition-free signal the analyzer most
naturally keys on (a turn that emits no tool call and ends the turn is a candidate "pure text /
verification" turn; a turn that emits tool calls is a candidate "acting" turn). It is a raw
observation, not a graded label — it states *what the model did*, not *what class of work it was*.
On the error arm and continuation-error arm, `tool_use_in_turn = false` (no parsed response).

### D4 — One shared helper, three call sites (DRY, testable seam)

Add a single private helper `emit_turn_usage(...)` next to `emit_system_prompt_assembled`
(`agent_loop/mod.rs:5191`), taking the raw fields and emitting the INFO event. Call it from:
1. `run_loop` success arm — after the LLM call resolves (`agent_loop/mod.rs:~845`, right after
   `resp` is available; outside the `if store_llm_calls` block per D2).
2. `run_loop` error arm — zero tokens, `status = "error"`.
3. `save_continuation_llm_call` — `step = u32::MAX`, usage from its `Option<&LlmUsage>` param.

The field-assembly (cache `Option<u64>` → `0`, sentinel step passthrough, `tool_use_in_turn` derivation)
is extracted into a pure `fn build_turn_usage_fields(step, usage, stop_reason, tool_use, status) ->
TurnUsageFields` so the mapping is unit-testable without a tracing subscriber (D5). `emit_turn_usage`
is a thin wrapper that calls the builder then `info!`s.

### D5 — Structural testability via a pure builder, not log-capture assertions

Tracing-event assertions are brittle (require a captured subscriber). Instead, the token/dimension
mapping is tested through the pure `build_turn_usage_fields` builder: token pass-through, `Option`
cache → `0`, `u32::MAX` sentinel preserved, `status`/`stop_reason`/`tool_use_in_turn` correctness on
both arms. The emission site itself is exercised (no-panic, no behaviour change) by the existing eval
harness (`cargo test -p mika-agent --test eval`), which runs the full `run_loop` with
`MockLlmProvider`. This mirrors the mika#1863 pure-predicate testing discipline.

---

## Implementation steps

1. **`TurnUsageFields` struct + `build_turn_usage_fields()` pure builder** (`agent_loop/mod.rs`, near
   `emit_system_prompt_assembled`): maps `(step: u32, usage: Option<&LlmUsage>, stop_reason: &str,
   tool_use_in_turn: bool, status: &str)` → a plain struct of the R1 fields, with cache `Option<u64>`
   → `0` and `input/output` → `0` when `usage` is `None`. No I/O, no logging.
2. **`emit_turn_usage(agent_id, session_id, trace_id, mode, fields: &TurnUsageFields)` helper**: single
   `info!(target: "mika::otel", event = "turn_usage", …)` call. Mirrors `emit_system_prompt_assembled`
   shape exactly.
3. **Wire success arm** (`run_loop`, after the LLM call resolves, **outside** `if store_llm_calls`):
   compute `tool_use_in_turn = matches!(resp.stop_reason, LlmStopReason::ToolUse) ||
   resp.has_tool_calls()`; call `emit_turn_usage(db.agent_id(), session_id, tool_ctx.trace_id,
   mode.label(), &build_turn_usage_fields(step as u32, Some(&resp.usage), &format!("{:?}",
   resp.stop_reason), tool_use_in_turn, "success"))`.
4. **Wire error arm**: `emit_turn_usage(…, &build_turn_usage_fields(step as u32, None, "error", false,
   "error"))`.
5. **Wire continuation** (`save_continuation_llm_call`): after the existing `save_llm_call`, emit with
   `step = u32::MAX`, usage from the `usage: Option<&LlmUsage>` param, `stop_reason` from its param,
   `tool_use_in_turn = false`, `status` from its param. `agent_id` via `db.agent_id()`.
6. **Unit tests** (inline `#[cfg(test)] mod tests`): `build_turn_usage_fields` — success-with-cache,
   success-no-cache (`None` → `0`), error-arm-zero, `u32::MAX` sentinel passthrough,
   `tool_use_in_turn` true/false.
7. **Docs**: add a one-line "Signal — RT-005 per-turn token accounting" bullet to `mika/CLAUDE.md`
   (Observability section) documenting the `turn_usage` event name, its fields, and the grep pattern
   `grep turn_usage $MIKA_SPIRIT_LOG_FILE | jq …` — consistent with the existing Signal A–N catalogue.

---

## Verification contract

- `cargo test -p mika-agent build_turn_usage` — new pure-builder unit tests green.
- `cargo test -p mika-agent --test eval` — full agent-loop eval harness unchanged and passing (proves
  R6 zero-behaviour-change: the loop still runs to completion across all modes).
- `cargo clippy -p mika-agent -- -D warnings`, `cargo fmt --check`.
- No schema change, no new env var, no new tool, no control-flow change, no `skills/bundled/*` change.
- Manual offline check (documented, not CI): after a local `mika ask`, `grep turn_usage
  $MIKA_SPIRIT_LOG_FILE | jq '{step, stop_reason, mode, input_tokens, output_tokens,
  tool_use_in_turn}'` shows one line per turn with populated token fields.

---

## Definition of Done

- `turn_usage` INFO event emitted per LLM call in `run_loop` (success + error arms) and for the
  continuation turn, on `target: "mika::otel"`, with the R1 field set (R1, R3, R4).
- Event fires independent of `MIKA_STORE_LLM_CALLS` (R2, D2).
- Event carries only raw dimensions — no `phase`/`is_planning`/`role` field (R5, D1); `tool_use_in_turn`
  present as a raw observation (D3).
- Zero behaviour change: control flow, guards, DB writes, deadline handling, and return values
  untouched (R6); eval harness unchanged and green.
- Pure `build_turn_usage_fields` builder unit-tested (D5); clippy/fmt clean.
- `mika/CLAUDE.md` documents the event name, fields, and grep pattern (impl step 7).

---

## Acceptance criteria

*(Transcribed from mika#1889 issue body — "Acceptance criteria" section.)*

- **AC1** — `usage` (input/output tokens) logged per turn, with `session_id` + agent + turn number.
- **AC2** — Offline-parsable format (the brick 5/5 analyzer will consume it).
- **AC3** — No regression on the existing loop (pure instrumentation, no behaviour change).

**Mapping to plan:** AC1 → R1 (fields include `session_id`, `agent_id`, `step`, `input_tokens`,
`output_tokens`). AC2 → R1/D2 (structured JSON INFO event on `target: "mika::otel"`, grep+jq-parsable
from `$MIKA_SPIRIT_LOG_FILE`, ungated). AC3 → R6/D5 (instrumentation-only; eval harness unchanged and
green).

**Non-negotiable guardrail (Prime hard condition #1), realised by D1/R5:** the log separates planning
from execution/verification tokens via *raw* dimensions (`step`, `stop_reason`, `tool_use_in_turn`,
`mode`) consumed by the offline analyzer — it does **not** bake a planning/execution classification
into the instrumentation. Planning-tokens remain the sole primary estimand; turns/handshakes/recalculs
stay descriptive covariables, defined offline.

---

## Out of scope

- **The offline analyzer / estimand computation** (brick 5/5, separate ticket). This plan emits the
  raw stream; it does not classify, aggregate, or compute planning-tokens.
- **KG extractor/resolver `save_llm_call` sites** (`kg/subject_extractor.rs`, `kg/entity_resolver.rs`)
  — those are background NER/resolution calls, not agent-loop turns. RT-005 measures the agent loop.
- **Any change to the `llm_calls` schema or the DB-persistence path.** The DB surface is untouched;
  the new event is a parallel log-only surface.
- **A `phase`/`is_planning` field** — deliberately excluded per D1 (Prime guardrail). Adding one later
  is an offline-analyzer concern, not an instrumentation one.
- **Dashboard UI for per-turn tokens** — the RT-005 consumer is the offline log analyzer, not the
  dashboard.

---

## Risks

- **R-undercount-if-arm-missed** — if the event is wired to only one arm, error/continuation turns
  drop from the covariable "turns" count. Mitigated by R3/R4 wiring all three call sites and D5 unit
  coverage of both arms.
- **R-baked-classification-creep** — a future reviewer may be tempted to add a convenient `phase`
  field. D1/R5 and the Out-of-scope note make the exclusion explicit and cite Prime's guardrail so the
  boundary is legible.
- **R-log-volume** — one extra INFO line per turn. Negligible: it is one line per LLM call, the same
  cardinality as the existing `system_prompt_assembled` and `agent done` events already emitted per
  turn. No new hot path.

---

## References

- mika#1217 (`emit_system_prompt_assembled` — the per-turn structured-log precedent this plan mirrors
  one-for-one; `agent_loop/mod.rs:5191`, v37→v38 companion).
- `crates/mika-common/src/llm/types.rs:159` (`LlmUsage` struct — the token source).
- `crates/mika-agent/src/agent_loop/mod.rs:654` (`run_loop`), `:480` (`save_continuation_llm_call`),
  `:845` (`if store_llm_calls` guard the event is deliberately placed outside of).
- RT-005 physics protocol: `~/.claude/plans/round-table-005-physics-protocol-2026-07-28.md`
  (ratified 2026-07-28). Sibling bricks: 1/5–3/5 (vague 1), 5/5 (offline analyzer, consumer of this
  stream).
- Prime hard condition #1 (planning-tokens SEUL; covariables descriptive only) — the load-bearing
  constraint behind D1/R5.
