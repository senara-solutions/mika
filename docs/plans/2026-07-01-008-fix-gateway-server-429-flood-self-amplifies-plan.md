# Plan: Gateway↔server 429 flood self-amplifies via retry loop (mika#1710)

**Ticket:** mika issue#1710 — `fix(gateway,server): 429 flood self-amplifies via retry loop — 1h+ silent-loop outage 2026-07-01`
**Labels:** `bug`, `p1-important`, `ready`
**Type:** issue (bug)
**Priority tier:** Tier 1 — *Breaks the loop* (gateway routing + dispatch reliability; a 1h+ silent-loop outage that required a manual restart).

---

## Problem

On 2026-07-01 (~16:00Z–17:15Z) the gateway → mika-spirit webhook delivery path entered a self-amplifying HTTP 429 flood that silently killed the autonomous loop for 1h+ until Vincent manually restarted both services. Hard evidence from the incident:

- Gateway logged **23,543** retry-flood 429s (`grep -c '"status":429' gateway.log`), sustained ~2×/sec, all `target=mika-dev`, all `attempt=5` (max).
- mika-spirit logged **53,810** rate-limit rejections (`grep -c '"status":429' server.log`), each with 82–102µs latency — instant rejection, no real work.
- `audit_events` silent for 1h11m (`MAX(created_at)` = 16:01:25Z until restart) — the loop was dead, not merely slow.
- Fresh restart of both services cleared it instantly (12,899 audit events in the following 5 min).

The ticket scopes **the amplification**, explicitly *not* the specific event-flood trigger (out of scope: "Fixing the specific event flood cause").

## Grounded root-cause analysis (what the code actually does)

Investigation of the current code (not the ticket's hypotheses) reveals the true mechanism, which differs from the ticket's "req/sec rate limiter" framing in one important way:

1. **The server-side "rate limiter" is a per-agent concurrency-of-1 lock, not a token bucket.**
   `crates/mika-agent/src/server/handlers.rs:221-234` — `handle_message` does `agent_state.agent_lock.clone().try_lock_owned()`; on `Err` it returns `429 TOO_MANY_REQUESTS {"error":"agent busy"}`. The lock (`tokio::sync::Mutex<()>`) is held for the *entire* agent turn (up to the 5-min deadline + 120s transport = ~420s worst case). **There is no configurable req/sec limit** — `crates/mika-gateway/src/settings.rs` and agent `Settings` have zero rate-limit tunables. So while an agent is busy on one message, *every* other inbound `/message` (webhook, redelivery, DLQ replay) gets an instant 429. This is by-design single-conversation-per-agent serialization — but it is invisible and un-throttled from the gateway's side.

2. **The gateway retry loop amplifies but does not coordinate across events.**
   `crates/mika-gateway/src/github.rs:1102-1282` — each inbound webhook is an independent `tokio::spawn` (`github.rs:881`) running `deliver_with_retry`. On 429 (classified Retryable at `github.rs:1035`) it retries on the fixed schedule `RETRY_DELAYS = [2s,5s,15s,60s,300s]` (`github.rs:589-595`) with ±25% jitter (`apply_jitter`, `github.rs:599-609`) → **6 HTTP attempts per event**, then persists to the DLQ. Backoff and jitter *already exist*, but **there is no shared per-target state**: N concurrent events each independently hammer the same busy agent, blind to the fact that the other N-1 are also getting 429s. Nothing tells event #2 "the target just 429'd event #1 fifty times — back off globally."

3. **The DLQ + GitHub redelivery keep re-injecting into the saturated lock.**
   `crates/mika-gateway/src/dlq.rs` — the Postgres-backed DLQ worker wakes every 30s (`WORKER_INTERVAL`), retries pending rows past `30s*2^attempts` (capped 1h), max 10 attempts → `dead`. Combined with fresh GitHub events and in-flight retry chains, the aggregate 429 rate stays above the drain rate. **The system cannot self-heal** because retries re-inject old events into an already-saturated lock, and the *legitimate* dispatch webhook is one 429 among tens of thousands — it never gets through. Hence the manual-restart requirement.

**Synthesis:** amplification = (independent per-event 6× retry fan-out) × (no cross-event awareness of target saturation) × (DLQ + redelivery re-injection) against a serialized concurrency-1 lock. The fix is a **gateway-side, per-target-agent circuit breaker** with shared state, plus **observability** (audit event on trip) and a **post-deploy smoke test** to catch regressions. Raising a numeric limit is *not* the fix — there is no numeric limit; the correct lever is coalescing/circuit-breaking on the gateway side.

## Requirements

The central new mechanism is **shared, concurrency-safe per-target-agent health state** on the gateway `AppState`, tracking consecutive-429 counts and a circuit-open-until instant per target agent. All six ACs hang off this spine (AC3 excepted — it is server-side).

### R1 — Per-target circuit breaker in the gateway (AC1 + AC5 unified)
Add `target_health: Arc<DashMap<String, TargetHealth>>` to `AppState` (`crates/mika-gateway/src/routes.rs:86`). `TargetHealth` tracks `consecutive_429: u32` and `open_until: Option<Instant>`. A single breaker with **two escalating thresholds** (avoids two competing breakers):
- **Soft trip (AC1):** on the **3rd** consecutive 429 for a target, open the circuit for **30s**. While open, new deliveries to that target short-circuit straight to the DLQ (no HTTP attempt) instead of hammering. `CB_SOFT_THRESHOLD = 3`, `CB_SOFT_OPEN = 30s`.
- **Hard trip / pause (AC5):** if consecutive-429 observations (probe failures across open/half-open cycles) reach **100**, open for **60s** and emit a distinct `gateway_target_paused` WARN log. `CB_HARD_THRESHOLD = 100`, `CB_HARD_OPEN = 60s`.
- **Half-open probe:** when `open_until` elapses, allow exactly one delivery through as a probe. Success → close + reset `consecutive_429 = 0`. 429 → re-open (escalate duration if hard threshold reached) and increment.
- **Reset on success:** any 200/202 from a target resets `consecutive_429 = 0` and clears `open_until`.
- All thresholds/durations are named `const`s (code-edit-tunable), consistent with `RETRY_DELAYS` and DLQ constants.

> Existing backoff+jitter (`RETRY_DELAYS`) is retained — AC1's "jitter + exponential backoff starting at 2s" is already satisfied by the current schedule; R1 adds the missing per-target circuit-break dimension. The 300s tail of `RETRY_DELAYS` is left intact (it is stronger backoff than AC1's suggested 60s max and already jittered); changing it is out of scope.

### R2 — Rate-limiter config sanity, documented honestly (AC2)
The "rate limiter" is a concurrency-1 lock, not a req/sec limit. Resolve AC2 by:
- Documenting in `crates/mika-gateway/CLAUDE.md` (Inbound delivery retry section) and `crates/mika-agent/CLAUDE.md` (HTTP Server section) that the 429 is the per-agent single-turn lock (`agent busy`), that the correct amplification control is the gateway circuit breaker (R1) + coalescing, **not** a numeric limit.
- No new numeric config field is introduced (there is nothing to size). If the implementer finds a concrete need to bound concurrent *distinct-event* fan-out per target beyond the circuit breaker, that belongs in R4, not a req/sec knob.

### R3 — Audit event on rate-limit trip, server-side (AC3)
In `handle_message` (`handlers.rs`), immediately before returning `StatusCode::TOO_MANY_REQUESTS` (the `Err(_)` arm of `try_lock_owned`), emit an audit event via the existing helper `agent_state.db.log_audit_event(session_id, tool_name, target_key, before, after, reasoning, trace_id)` (signature at `crates/mika-agent/src/async_db.rs:1439`, precedent `webhook_deferred` at `handlers.rs:156`):
- `tool_name = "rate_limit_trip"`, `target_key = format!("agent:{}", req.agent)`, `reasoning = "agent busy — message rejected with 429"`, plus request_id in the after/reasoning field.
- **Volume guard:** a naive emit-on-every-429 would itself write 53k audit rows during a flood. Rate-limit the audit emission to **at most one row per target per N seconds** (`RATE_LIMIT_TRIP_AUDIT_INTERVAL`, e.g. 10s) using an in-memory `last_emitted: DashMap<agent, Instant>` on `AppState` (agent side). This keeps the signal visible to the orchestrator without re-creating a write flood. Fire-and-forget on DB error (warn-log), matching `webhook_deferred`.

### R4 — Bounded retry buffer with drop-oldest (AC4)
Today the in-flight retry "buffer" is unbounded `tokio::spawn`ed tasks; only *concurrent HTTP* is bounded (30-permit `webhook_semaphore`) — tasks sleeping between retries hold no permit and can accumulate without limit under flood. Add an explicit bound:
- A dedicated in-flight-delivery counter (an `Arc<AtomicUsize>` or a second semaphore `delivery_slots` on `AppState`) capping the number of concurrently-spawned delivery tasks per gateway (`MAX_INFLIGHT_DELIVERIES`, e.g. 500).
- **Drop-oldest / shed policy:** when at capacity, a new inbound webhook is persisted directly to the DLQ (durable, drop-nothing) rather than spawning an unbounded task — the DLQ *is* the bounded overflow store (Postgres, `dead`-transition bounded). Emit a `delivery_buffer_full` WARN. This makes the in-memory footprint bounded and deterministic while preserving at-least-once delivery via the DLQ.
- With the circuit breaker (R1) short-circuiting to DLQ during open windows, in-flight task accumulation is already sharply reduced; R4 is the hard ceiling backstop.

### R5 — Post-deploy verification smoke test (AC6)
Add a smoke test invoked by `make deploy` after `restart` (alongside `check-ngrok` at `Makefile:45-54`):
- New script `scripts/smoke-webhook-flood.sh` (or a `make smoke-webhooks` target) that fires ~10 mock `/message` POSTs (or `/webhook/github` events with valid HMAC) at the running gateway/agent and asserts **all return 200/202, zero 429s**.
- Wire it into the `deploy` target after `check-ngrok`. **Non-fatal by default** (warn on failure, like `check-ngrok`) to avoid blocking deploys on a transient cold-start 429 — but print a loud, actionable warning. Rationale: `make deploy` must not hard-fail on a benign single busy-lock during warmup; the test's job is regression *visibility*, per AC6 ("Catch regression class").
- The script must not require secrets it can't obtain locally; if `MIKA_INTERNAL_TOKEN` is unavailable it skips with a clear message (fail-open, same posture as `check-ngrok`).

### R6 — Test coverage for the cascade (verification contract)
The existing `deliver_with_retry_inner` already accepts an injected `retry_delays` for deterministic timing tests (`github.rs:1131`). Extend the harness to inject/observe the circuit-breaker state so the cascade is regression-gated (see Verification Contract).

## Non-goals / Out of scope

- **NG1 — Fixing the specific event-flood trigger** (deploy-triggered / user-triggered burst). Per the ticket, this fix is about amplification, which happens regardless of cause.
- **NG2 — Replacing the concurrency-1 agent lock with a real queue / multi-turn concurrency.** The single-turn-per-agent serialization is a deliberate engine invariant; changing it is a much larger design change, not this bug fix.
- **NG3 — Changing `RETRY_DELAYS` values or the DLQ backoff formula.** Backoff already exists and is adequate; R1 adds the missing cross-event coordination layer, not a re-tune.
- **NG4 — Telegram retry parity.** `telegram.rs` has no autonomous retry loop (429s returned to caller, no fixed-schedule retry); it is not part of the observed flood. The shared circuit breaker keying by target agent naturally applies if Telegram delivery is later routed through the same path, but no Telegram-specific work is in scope.
- **NG5 — A req/sec numeric rate-limit config.** There is no numeric limit to tune (R2); inventing one would be a false fix.

## Implementation approach

**Repo:** `mika` (single repo, two crates: `mika-gateway` + `mika-agent`). No cross-repo companion.

1. **`AppState` extension** (`crates/mika-gateway/src/routes.rs:86`): add `target_health: Arc<DashMap<String, TargetHealth>>` and `delivery_slots` (AtomicUsize or Semaphore). Update **all ~5 construction sites**: `main.rs:173`, `orchestrator_inbox.rs:541`, and test builders at `github.rs:2051/2166/2593`. Add `dashmap` to `mika-gateway/Cargo.toml` if not already a dependency (it is used elsewhere in the workspace).
2. **Circuit breaker module** (new `crates/mika-gateway/src/circuit_breaker.rs` or inline in `github.rs`): `TargetHealth` struct + `check_and_record` API (`is_open(target) -> bool`, `record_429(target)`, `record_success(target)`), with the two-threshold escalation and half-open probe logic. Pure, unit-testable (no I/O). Consts co-located.
3. **Wire into `deliver_with_retry_inner`** (`github.rs:1131`): before each HTTP attempt, if `target_health.is_open(target)` → short-circuit to DLQ (skip attempt). On `ForwardResult::Retryable{429}` → `record_429`. On `Success` → `record_success`. On hard-trip → `gateway_target_paused` WARN.
4. **In-flight bound** (R4): at `handle_github_webhook` spawn site (`github.rs:881`), acquire a `delivery_slots` permit / check the in-flight counter before spawning; on capacity → `dlq::insert_delivery` + `delivery_buffer_full` WARN instead of spawn.
5. **Server-side audit** (R3): in `handlers.rs` 429 arm (`:225`), throttled `log_audit_event("system"/session, "rate_limit_trip", "agent:{name}", ...)`; add the `last_emitted` DashMap to the agent server `AppState`.
6. **Smoke test** (R5): `scripts/smoke-webhook-flood.sh` + `Makefile` wiring after `check-ngrok`.
7. **Docs** (R2 + component CLAUDE.md updates): gateway CLAUDE.md "Inbound delivery retry" + a new "Target circuit breaker" subsection; agent CLAUDE.md HTTP Server section (audit event + honest 429-is-a-lock note). Run `scripts/sync-agent-docs.sh` if `docs/` under agent crate changed (CI `docs-sync` gate).

## Verification contract

**Unit (gateway, `cargo test -p mika-gateway`):**
- `test_circuit_breaker_soft_trip_opens_after_3` — 3 consecutive `record_429` → `is_open` true; 4th delivery short-circuits (no HTTP call issued — assert via mock forward count).
- `test_circuit_breaker_resets_on_success` — after soft trip, `record_success` → `is_open` false, `consecutive_429 == 0`.
- `test_circuit_breaker_half_open_probe` — after `open_until` elapses (inject clock / short const), exactly one probe allowed; 429 re-opens, success closes.
- `test_circuit_breaker_hard_pause_at_100` — 100 consecutive 429 → 60s open + `gateway_target_paused` observed.
- `test_deliver_short_circuits_to_dlq_when_open` — with breaker open, `deliver_with_retry_inner` persists to DLQ without HTTP attempt (extend existing injected-schedule harness at `github.rs:1131`).
- `test_inflight_bound_sheds_to_dlq` (R4) — at `MAX_INFLIGHT_DELIVERIES`, new webhook lands in DLQ + `delivery_buffer_full`, not an unbounded spawn.
- Existing retry tests (`test_deliver_retry_on_429_then_success`, `test_deliver_retry_budget_exhausted_after_six_attempts`) still pass unchanged (breaker threshold=3 must not break the 6-attempt single-event path — a single event's own attempts count toward its target's consecutive-429, so verify interaction: **decision point — see Open Question 1**).

**Unit (agent, `cargo test -p mika-agent`):**
- `test_rate_limit_trip_emits_audit_event` — busy lock → 429 path writes one `rate_limit_trip` audit row with `target_key == "agent:<name>"`.
- `test_rate_limit_trip_audit_throttled` — N rapid 429s within the interval → at most one audit row per target per interval.
- Existing `test_message_returns_429_when_busy` (`server/mod.rs:1733`) still passes.

**Manual / integration:**
- Run `scripts/smoke-webhook-flood.sh` against a locally-running gateway+agent → all 200/202, zero 429.
- `cargo clippy --all-targets` clean; `cargo fmt` clean.
- `make build` succeeds; `docker build -f Dockerfile.gateway` succeeds (CI `docker-build` gate).

## Open questions (for the implementer / architect to resolve)

1. **Single-event self-interaction with the soft threshold.** A single event's own 6-attempt retry chain produces up to 6 consecutive 429s for its target — which would itself trip the soft breaker (threshold 3) mid-chain. Is that desired (good — one slow event stops hammering after 3 tries and DLQs) or should the breaker count *distinct events* rather than *attempts*? **Recommendation:** count attempts (simpler, and tripping after 3 in-chain 429s is exactly the amplification control we want — the event goes to DLQ and retries later on the DLQ schedule). Confirm this reframes `test_deliver_retry_budget_exhausted_after_six_attempts` (a lone event may now DLQ after 3, not 6, when the breaker is shared). The test may need to assert the *new* correct behavior.
2. **Reaching 100 consecutive 429s (AC5) given the soft breaker short-circuits at 3.** Because soft-trip short-circuits deliveries (no new agent 429s during open), the "100 consecutive" count accrues via half-open probe failures across many open/close cycles. Confirm the semantics: count probe-failures toward the hard threshold. If Vincent/architect prefer AC5 as an independent longer-window counter (e.g., 429s-per-rolling-60s), state that; the plan models it as breaker escalation for orthogonality.
3. **Audit `session_id` for the server-side trip (R3).** `webhook_deferred` uses `"system"`. Confirm `"system"` is the right actor bucket for `rate_limit_trip` (recommended — it is a system-level event, not session-scoped).

## Definition of Done

- [ ] `AppState` carries per-target circuit-breaker state; all construction sites updated; builds clean.
- [ ] Gateway circuit breaker: soft trip (3→30s), hard pause (100→60s), half-open probe, reset-on-success — unit-tested.
- [ ] Open-circuit deliveries short-circuit to DLQ (no HTTP hammering) — unit-tested.
- [ ] In-flight delivery buffer explicitly bounded with DLQ overflow (drop-oldest/shed) — unit-tested.
- [ ] Server emits throttled `rate_limit_trip` audit events on the 429 busy-lock path — unit-tested.
- [ ] `scripts/smoke-webhook-flood.sh` added and wired into `make deploy` after `check-ngrok` (non-fatal, loud on failure).
- [ ] gateway + agent CLAUDE.md updated (circuit breaker, honest 429-is-a-lock note, audit event); `sync-agent-docs.sh` run if agent `docs/` touched.
- [ ] `cargo test` (both crates), `cargo clippy --all-targets`, `cargo fmt --check` all pass.
- [ ] `docker build -f Dockerfile.gateway` passes (CI `docker-build`).
- [ ] PR body documents the concurrency-1-lock reframing of AC2 and links the incident evidence.

## Acceptance criteria

Transcribed verbatim from mika#1710's `## Acceptance criteria` section (per grooming step 5b). Implementation notes in italics reflect the grounded reframing above; the criteria themselves are unchanged.

- **AC1 — retry backoff on 429.** Gateway retry policy respects a 429 as "back off exponentially," not "retry immediately 5×." Recommend: on 429, jitter + exponential backoff starting at 2s, max 60s. Circuit-break after N=3 consecutive 429s for that target agent for M=30s.
  *→ Backoff+jitter already present (`RETRY_DELAYS`); R1 adds the missing N=3/30s per-target circuit breaker.*
- **AC2 — rate-limiter config sanity.** mika-spirit `/message` rate limit config verified appropriate for gateway's normal webhook rate. If limit is 100 req/sec and gateway sustains 200 req/sec on a normal burst → limit is wrong. If gateway generates 200 on a legitimate burst → gateway coalescing is wrong.
  *→ Grounded finding: the limit is not req/sec — it is a per-agent concurrency-1 lock. R2 documents this and establishes that gateway circuit-breaking/coalescing (not a numeric limit) is the correct control.*
- **AC3 — audit_event on rate-limit trip.** When rate limiter rejects, emit an audit_event kind=`rate_limit_trip` with target, source, endpoint. Currently invisible to orchestrator.
  *→ R3: throttled `log_audit_event("rate_limit_trip", "agent:{name}", …)` on the server 429 path.*
- **AC4 — bounded retry buffer.** Gateway's in-memory retry buffer must have a bounded size + drop-oldest policy. Currently appears unbounded (or very large) which is what allows self-amplification.
  *→ R4: explicit in-flight-delivery bound with DLQ overflow (durable drop-oldest).*
- **AC5 — self-heal mechanism.** After N=100 consecutive 429s from the same target, gateway pauses retries to that target for M=60s. Log the pause. Currently requires human restart.
  *→ R1 hard-trip threshold (100→60s) + `gateway_target_paused` log.*
- **AC6 — post-deploy verification test.** Add smoke-test to `make deploy` post-restart that fires ~10 mock webhooks and verifies all get 200s, no 429s. Catch regression class.
  *→ R5: `scripts/smoke-webhook-flood.sh` wired into `make deploy` after `check-ngrok`.*

## References

- `crates/mika-agent/src/server/handlers.rs:221-234` — the per-agent lock → 429 path (the "rate limiter").
- `crates/mika-agent/src/server/handlers.rs:142-218` — webhook deferral queue (#528) + `webhook_deferred` audit precedent (`:156`).
- `crates/mika-agent/src/async_db.rs:1439` — `log_audit_event` signature (AC3).
- `crates/mika-agent/src/server/mod.rs:1733` — `test_message_returns_429_when_busy`.
- `crates/mika-gateway/src/github.rs:1102-1282` — `deliver_with_retry(_inner)` retry loop.
- `crates/mika-gateway/src/github.rs:589-609` — `RETRY_DELAYS` + `apply_jitter`.
- `crates/mika-gateway/src/github.rs:1035` — 429/5xx Retryable classification.
- `crates/mika-gateway/src/github.rs:853-895` — webhook spawn + semaphore load-shed (503).
- `crates/mika-gateway/src/routes.rs:86-97` — `AppState` (breaker-state anchor).
- `crates/mika-gateway/src/dlq.rs:110-150` — DLQ worker interval/backoff/max-attempts.
- `crates/mika-gateway/src/settings.rs` — confirms no rate-limit config field (AC2).
- `Makefile:45-54` — `deploy` → `check-ngrok` (AC6 wiring point).
- Incident evidence: `gateway.log` + `server.log` 2026-07-01 16:00–17:15Z (23,543 / 53,810 429s; 1h11m audit silence).
