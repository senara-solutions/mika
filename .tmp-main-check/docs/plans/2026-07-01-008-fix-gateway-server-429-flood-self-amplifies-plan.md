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
Add `target_health: Arc<DashMap<String, TargetHealth>>` to `AppState` (`crates/mika-gateway/src/routes.rs:86`). `TargetHealth` tracks `consecutive_429: u32`, a rolling record of recent 429 timestamps (for the hard-pause count), `open_until: Option<Instant>`, and `current_open: Duration` (the escalating open duration). **One breaker, one counter, two signals** (soft trip + hard pause) driven off the same state — avoids two competing breakers:
- **Soft trip (AC1):** on the **3rd** consecutive 429 for a target, open the circuit. While open, new deliveries to that target short-circuit straight to the DLQ (no HTTP attempt) instead of hammering. `CB_SOFT_THRESHOLD = 3`; open duration starts at `CB_SOFT_OPEN = 30s`.
- **Adaptive open-window escalation (F3 fix — addresses probe-burn against the ~420s lock hold):** each time the half-open probe fails (429), *escalate* the next open duration exponentially: `current_open = min(current_open * 2, CB_MAX_OPEN)`, i.e. 30s→60s→120s→240s→`CB_MAX_OPEN = 480s`. `CB_MAX_OPEN` is deliberately **> the worst-case ~420s per-agent lock hold** (5-min deadline + 120s transport, per the root-cause analysis §1), so after a few failed probes the open window *exceeds* the lock-hold timescale and the next probe finally lands on a *free* lock — instead of a 30s probe uselessly re-failing every cycle against a turn that is still running. This is F3 option (a) done adaptively: the breaker converges its probe interval toward the real recovery window rather than burning a delivery every 30s. **Framing (F3): the breaker is primarily a backpressure valve, not a precise recovery detector** — the widening probe interval stops the hammering and re-tests cheaply; it is not expected to pinpoint the exact recovery instant. This is stated explicitly so future readers do not mistake the half-open probe for a health check.
- **Hard pause / self-heal (AC5), rolling-window (F1 fix):** the hard threshold counts **≥ `CB_HARD_THRESHOLD = 100` 429 observations for a target within a rolling `CB_HARD_WINDOW = 5min`** — a *rolling-window* count, **not** a purely-consecutive count that the soft short-circuit would starve. Because every event's 429s to the same busy target land in the same window (the incident logged 23,543 for `target=mika-dev`), 100 is genuinely reachable under a sustained flood, while a single slow event never approaches it. On crossing the threshold the breaker holds the circuit open for `max(CB_HARD_OPEN = 60s, current_open)` (always ≥ AC5's 60s floor) and emits a distinct `gateway_target_paused` WARN. **This is defense-in-depth and expected to be rare** (F1 option c): under normal load the soft trip + adaptive escalation already shed the flood; the hard pause is the loud, explicit AC5 self-heal signal reserved for the sustained-flood case.
- **Half-open probe:** when `open_until` elapses, allow exactly one delivery through as a probe. Success → close, reset `consecutive_429 = 0`, reset `current_open = CB_SOFT_OPEN`. 429 → re-open with the escalated `current_open` and record the 429 into the rolling window.
- **Reset on success:** any 200/202 from a target resets `consecutive_429 = 0`, resets `current_open`, prunes the rolling window, and clears `open_until`.
- All thresholds/durations/windows are named `const`s (code-edit-tunable), consistent with `RETRY_DELAYS` and DLQ constants.

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
- **Preconditioned idle state (F4 — the test must not trip the very breaker it validates):** if the target agent is mid-turn when the flood fires, the first webhooks 429, the soft breaker trips at 3, and the rest short-circuit to the DLQ — the test would then see missing 200s (or read the breaker as a false-positive) rather than a clean pass. Two-part guard: **(a)** fire against a dedicated, guaranteed-idle **`smoke-test` target agent** — never `mika-dev`, which may be mid-dispatch or draining queued DLQ events from a prior deploy; **(b)** before firing, poll readiness (`GET /health`, or a lock-free check) and wait up to a short bounded timeout for the agent lock to be free. If the agent does not reach idle within the timeout, **skip with a clear message** (fail-open, same posture as the missing-token skip) rather than emit a false regression signal. Document this precondition in the script header and in the CLAUDE.md deploy note. This satisfies the review-guide § Test Reliability requirement that success-asserting tests ensure a pre-conditioned idle state.

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
- `test_circuit_breaker_half_open_probe` — after `open_until` elapses (inject clock / short const), exactly one probe allowed; 429 re-opens, success closes + resets `current_open` to `CB_SOFT_OPEN`.
- `test_circuit_breaker_open_window_escalates_on_probe_failure` (F3) — repeated probe failures escalate the open duration 30s→60s→120s→240s→`CB_MAX_OPEN`; assert the cap (`CB_MAX_OPEN = 480s`) **exceeds the ~420s worst-case lock hold** and the duration does not grow unbounded; a success mid-escalation resets `current_open` to `CB_SOFT_OPEN`.
- `test_circuit_breaker_hard_pause_rolling_window` (F1/D2) — ≥100 429 observations for a target within `CB_HARD_WINDOW` → open ≥ `CB_HARD_OPEN` (60s) + `gateway_target_paused` observed; a 429 count that never reaches 100 *within the window* (older 429s pruned) does **not** hard-pause — proving the rolling-window (not purely-consecutive) semantics.
- `test_deliver_short_circuits_to_dlq_when_open` — with breaker open, `deliver_with_retry_inner` persists to DLQ without HTTP attempt (extend existing injected-schedule harness at `github.rs:1131`).
- `test_inflight_bound_sheds_to_dlq` (R4) — at `MAX_INFLIGHT_DELIVERIES`, new webhook lands in DLQ + `delivery_buffer_full`, not an unbounded spawn.
- Retry-semantics tests (F2/D1): `test_deliver_retry_on_429_then_success` still passes (one 429 then success — below the soft threshold, breaker never trips). `test_deliver_retry_budget_exhausted_after_six_attempts` is **reframed** — with the shared breaker active against a persistently-busy target, the lone event DLQs at the soft trip (~3 attempts), the ratified D1 behavior; the test asserts the *new* correct outcome. A companion `test_deliver_retry_budget_six_attempts_breaker_below_threshold` preserves the pure 6-attempt path for the case where the breaker does not trip (e.g., the target's other traffic keeps `consecutive_429` reset, or retryable non-429 errors that do not increment the 429 counter).

**Unit (agent, `cargo test -p mika-agent`):**
- `test_rate_limit_trip_emits_audit_event` — busy lock → 429 path writes one `rate_limit_trip` audit row with `target_key == "agent:<name>"`.
- `test_rate_limit_trip_audit_throttled` — N rapid 429s within the interval → at most one audit row per target per interval.
- Existing `test_message_returns_429_when_busy` (`server/mod.rs:1733`) still passes.

**Manual / integration:**
- Run `scripts/smoke-webhook-flood.sh` against a locally-running gateway+agent → all 200/202, zero 429.
- `cargo clippy --all-targets` clean; `cargo fmt` clean.
- `make build` succeeds; `docker build -f Dockerfile.gateway` succeeds (CI `docker-build` gate).

## Decisions (resolved from architect first-pass ITERATE) and remaining open question

**D1 — ratified retry-semantics change (resolves former OQ1 / architect F2, BLOCKING): the soft breaker counts *attempts*, and the reduced per-event retry budget under target stress is the *desired* behavior.** A single event's own 6-attempt retry chain produces consecutive 429s that count toward its target's `consecutive_429`; at threshold 3 the event trips the breaker mid-chain and is persisted to the DLQ after ~3 in-chain attempts instead of all 6. **We explicitly ratify this as the intended amplification-control semantics** (review-guide.md § Behavioral Contracts / "changes to retry semantics require explicit operator ratification"): under target stress, amplification control outweighs per-event in-chain persistence. **Durability is not lost** — the event lands in the DLQ and receives its remaining delivery budget on the DLQ's own spaced schedule (`30s·2^attempts`, cap 1h, max 10 attempts). Net effect: *fewer immediate hammering retries, same at-least-once guarantee via a slower, target-friendly channel*. We reject F2 option (b) distinct-event-ID tracking (unnecessary complexity for no durability gain) and reject F2 option (c) raising the soft threshold to 6 (it would preserve the very in-chain hammering we are suppressing). **Test impact:** `test_deliver_retry_budget_exhausted_after_six_attempts` is reframed — with the shared breaker active against a persistently-busy target, a lone event DLQs at the soft trip (~3 attempts), which is the ratified behavior; a companion variant preserves the pure 6-attempt path for the breaker-not-tripped case (see Verification contract).

**D2 — rolling-window hard pause (resolves former OQ2 / architect F1): the AC5 hard pause uses a rolling-window 429 count, making the 100 threshold reachable; it is defense-in-depth and expected to be rare.** See R1. The hard threshold counts 429 observations for a target within `CB_HARD_WINDOW = 5min` (across events and across open/close cycles), **not** a purely-consecutive count that the soft short-circuit would starve (the concern F1 raised). Under a genuine sustained flood (incident: 23,543 `target=mika-dev` 429s) 100 is reached quickly; under normal operation the soft trip + adaptive open-window escalation shed load first and the hard pause rarely fires. This honors AC5's literal "100 … pause 60s + log" while giving it a reachable activation path (review-guide.md § YAGNI — "complexity budget requires reachable activation paths"). We reject F1 option (a) removing the hard trip: AC5 mandates the 100/60s pause, so removal would weaken an AC rather than address the finding.

**OQ3 (still open — minor, not gated by any finding): Audit `session_id` for the server-side trip (R3).** `webhook_deferred` uses `"system"`. `rate_limit_trip` should follow the same actor bucket (recommended — it is a system-level event, not session-scoped). Left as an implementer detail; no architect finding depends on it.

## Definition of Done

- [ ] `AppState` carries per-target circuit-breaker state; all construction sites updated; builds clean.
- [ ] Gateway circuit breaker: soft trip (3→30s), adaptive open-window escalation (30s→…→`CB_MAX_OPEN` 480s > ~420s lock hold, F3), rolling-window hard pause (100 in 5min → ≥60s + `gateway_target_paused`, F1/D2), half-open probe, reset-on-success — unit-tested.
- [ ] Open-circuit deliveries short-circuit to DLQ (no HTTP hammering) — unit-tested.
- [ ] In-flight delivery buffer explicitly bounded with DLQ overflow (drop-oldest/shed) — unit-tested.
- [ ] Server emits throttled `rate_limit_trip` audit events on the 429 busy-lock path — unit-tested.
- [ ] `scripts/smoke-webhook-flood.sh` added and wired into `make deploy` after `check-ngrok` (non-fatal, loud on failure); fires against a guaranteed-idle `smoke-test` agent with a readiness precondition, skips fail-open if not idle (F4).
- [ ] gateway + agent CLAUDE.md updated (circuit breaker, honest 429-is-a-lock note, audit event); `sync-agent-docs.sh` run if agent `docs/` touched.
- [ ] `cargo test` (both crates), `cargo clippy --all-targets`, `cargo fmt --check` all pass.
- [ ] `docker build -f Dockerfile.gateway` passes (CI `docker-build`).
- [ ] PR body documents the concurrency-1-lock reframing of AC2, ratifies the D1 retry-semantics change (fewer in-chain retries under target stress, durability preserved via the DLQ), and links the incident evidence.

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
  *→ R1 hard pause (100 429s within a rolling 5-min window → ≥60s open) + `gateway_target_paused` log. Rolling-window counting (D2) keeps the 100 threshold reachable across the soft-trip open/close cycles that F1 flagged; the adaptive open-window escalation (up to 480s > the ~420s lock hold) is the F3 fix so the pause outlasts the busy turn.*
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

## Revision history

- rev 2 (2026-07-01): addressed architect first-pass ITERATE findings F1–F4.
  - **F2 (BLOCKING)** — ratified the retry-semantics change as decision **D1**: the soft breaker counts *attempts*, a lone event under a persistently-busy target DLQs at the soft trip (~3 attempts) instead of 6, and this reduced in-chain retry budget is explicitly the desired amplification control; durability is preserved via the DLQ's spaced schedule. Rejected distinct-event tracking (F2b) and a 6+ soft threshold (F2c) with reasons. Reframed the affected retry test + added a breaker-below-threshold companion.
  - **F3** — added **adaptive open-window escalation** to R1: the open duration doubles on each probe failure up to `CB_MAX_OPEN = 480s`, deliberately chosen to exceed the ~420s worst-case per-agent lock hold so probes stop burning against an in-flight turn. Explicitly framed the breaker as a backpressure valve, not a recovery detector (F3 option a, done adaptively + option c framing).
  - **F1** — changed the hard pause to a **rolling-window** 429 count (100 within `CB_HARD_WINDOW = 5min`, decision **D2**) so the AC5 threshold is reachable across soft-trip open/close cycles, and framed it as rare defense-in-depth. Did not remove the hard trip (F1a) because AC5 mandates it.
  - **F4** — added a preconditioned-idle-state guard to the R5 smoke test: fire against a dedicated guaranteed-idle `smoke-test` agent + poll readiness before firing, skip fail-open if not idle.
  - Converted Open Questions 1 & 2 into ratified decisions D1/D2; kept OQ3 (audit `session_id`) open as a non-finding-gated implementer detail. Updated Verification contract, Definition of Done, and the AC5 note to match.
