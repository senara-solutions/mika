# Plan — feat(server): bounded webhook queue with backpressure + coalescing (mika#1870)

**Ticket:** mika#1870 — replace the `POST /message` 429-reject-on-busy pattern with a per-agent
bounded queue that applies backpressure and coalesces redundant webhook bursts.
**Type:** feat · **Priority:** p1-important · **Component:** agent-core (`crates/mika-agent/src/server/`)
**Blast radius:** medium-high — replaces a load-bearing dispatch pattern for ALL agents. Kill-switch (AC9)
provides instant rollback. **Follow-up to** mika#1869 (tactical `check_suite.completed` dedup, already
merged as `check_suite_dedup.rs`). This ticket is the durable class-fix that generalizes it.

---

## 1. Problem statement

`POST /message` (`crates/mika-agent/src/server/handlers.rs:280`) serializes agent processing with a
non-blocking `agent_lock.try_lock_owned()`. On contention it emits a throttled `rate_limit_trip` audit
event and returns **429 "agent busy"** (`handlers.rs:280–336`). Under burst load — N concurrent webhooks
for the same logical event — the gateway sees N 429s and either drops them or spins. Observed
2026-07-28: mika-qa took **41 `rate_limit_trip` events in one hour**, verdicts stalled 5h+. Root cause was
a webhook storm (up to 8 workflows × 4 PRs = 32 `check_suite.completed` events, plus op-proxy un-drafts
re-triggering CI).

There is **no bounded queue with backpressure** between webhook ingestion and the agent mailbox. The
existing `webhook_queue` module (`webhook_queue.rs`, mika#528) is a *different* mechanism — it defers
`pull_request_review.submitted` events while a target task has an in-flight `run_claude_pilot` callback,
to prevent a metadata race. It is **not** a general-purpose backpressure queue and stays untouched by
this ticket.

`check_suite_dedup.rs` (mika#1869) coalesces one event type (`check_suite.completed(success)`) at the
`ci_success_handler` layer, downstream of `POST /message`. This ticket lifts that idea to a general
per-agent queue at the ingestion boundary, keyed by an exhaustive-match coalescing function.

---

## 2. Current architecture (grounding)

Message flow through `handle_message` (`handlers.rs:139`):

1. **Deferral check** (existing mika#528 path, `handlers.rs:~200–277`) — if the webhook correlates to a
   task with an in-flight callback, push onto `AgentState.webhook_queue` (`Vec<DeferredWebhook>`), spawn a
   60s timeout replay task (`handlers.rs:254`), return `202 {status:"deferred"}`. **Unchanged.**
2. **Lock acquire** (`handlers.rs:280`) — `agent_lock.clone().try_lock_owned()`. On `Err` → throttled
   `rate_limit_trip` audit + **429**. On `Ok(guard)` → spawn `run_agent_for_message(&s,&a,req,imgs,lock)`
   (`handlers.rs:378`), returning `202` immediately; the spawned task holds the owned lock for the whole
   agent loop (`run_agent_for_message`, `handlers.rs:810`, `let _lock = lock;` at `:817`).
3. **Replay path** (`replay_deferred_webhooks`, `handlers.rs:780`) already models the exact primitive the
   drain worker needs: `let lock = agent_state.agent_lock.clone().lock_owned().await;` (blocking) then
   `run_agent_for_message(state, agent_state, deferred.request, imgs, lock).await;` (`:797`).

Key facts the design leans on:

- **The client contract is already async 202.** `POST /message` returns `202 Accepted` today and
  processes in a spawned background task. Routing through a queue does **not** change what the gateway
  observes for the accepted case.
- **Event kind is parsed from `MessageRequest.text`**, not HTTP headers. The gateway pre-formats each
  event into text: `[GitHub] Check suite {conclusion} on {repo} (branch: {branch})`
  (`webhook_queue.rs` `CHECK_SUITE_RE`), `[GitHub] PR review (...) on {repo}#{n}`
  (`verdict.rs::parse_pr_review_event`), `[GitHub] Issue labeled ready on {repo}#{n}`, `[GitHub] PR
  closed: {repo}#{n}`, etc. `MessageRequest` fields (`types.rs:12`): `text`, `chat_id`, `channel`,
  `request_id`, `agent`, `images`. There is **no** structured event-kind/repo/sha field — those are
  regex-parsed. This is the single most important realism constraint for AC2.
- `AgentState` (`state.rs:26–53`): `agent_lock: Arc<tokio::sync::Mutex<()>>`,
  `webhook_queue: Arc<tokio::sync::Mutex<Vec<DeferredWebhook>>>`, `db: AsyncDatabase`, `settings`. New
  field will live here.
- Per-agent background workers are spawned in `run_server()` iterating `state.agents` with a parent
  `CancellationToken` (KG resolver tick, `mod.rs:893,1302`; dashboard checkpoint, `mod.rs:834`). Lazy
  agents resolved post-boot (`resolve_agent`, mika#1399) must also get a worker.
- `AsyncDatabase::log_audit_event(session_id, tool_name, target_key, before, after, reasoning, trace_id)`
  (`async_db.rs:1459`). Audit throttle helper `should_emit_rate_limit_audit(map, label, now, interval)`
  (`handlers.rs:55`) with `AppState.rate_limit_audit_last: Arc<DashMap<String, Instant>>`.
- Config pattern: `Settings` field `foo: Option<u64>` (`#[serde(default)]`) + `DEFAULT_FOO` const +
  `effective_foo() -> u64 { self.foo.unwrap_or(DEFAULT_FOO) }` (`config.rs:911–913,964,1317`). config-rs
  auto-maps `MIKA_FOO` → `foo`.
- **No gauge-metric infrastructure exists.** All observability is audit-event + structured-log based
  (`tracing`). The ticket's "Metrics (dashboard hook)" section is not covered by any hard AC — it is
  scoped as periodic structured-log gauges here, dashboard wiring deferred (§8).

---

## 3. Design

### 3.1 Uniform-queue model (primary decision)

**All inbound `POST /message` requests enqueue into a per-agent bounded queue; a single per-agent drain
worker is the sole consumer that acquires `agent_lock` and calls `run_agent_for_message`.** The existing
deferral check (§2 step 1) runs *before* the enqueue and is unchanged.

Rationale for uniform-queue over a contention-triggered fast-path:

- The client contract is **already** async-202 (§2), so uniform enqueue changes nothing the gateway sees
  for accepted events — only the busy case changes from 429 to "queued".
- A single consumer removes the dual-processor race between an inline fast-path and a drain worker both
  contending on `agent_lock` — that race would make coalescing non-deterministic.
- Coalescing fires exactly when it must: the first event is being processed (worker holds the lock) while
  the burst siblings accumulate in the queue and collapse. The uncontended latency cost is one
  `Notify` wakeup hop (microseconds).

Kill-switch (AC9) restores the legacy path: when `MIKA_WEBHOOK_QUEUE_ENABLED=false`, `handle_message`
skips the enqueue and uses the current `try_lock_owned()` → 429 logic verbatim. Both paths coexist behind
the flag; no redeploy needed to roll back.

**Single priority class in v1.** The ticket marks priority classes "v2 optional" and AC7's priority test
"(if v2)". v1 ships one FIFO class — it covers the founding incident (a `check_suite` burst). Priority
classes are deferred (§8) to bound blast radius.

### 3.2 New module `crates/mika-agent/src/server/webhook_queue_v2.rs`

```rust
/// Classified webhook event kind, parsed from the gateway-formatted MessageRequest.text.
/// Exhaustive so coalescing_key() is compiler-checked (AC2).
enum WebhookEventKind {
    CheckSuite { repo: String, branch: String },        // sha not in gateway text → see note
    PullRequestSync { repo: String, pr: u64 },
    Push { repo: String, branch: String },
    PrReview,                                            // NEVER coalesce (user input)
    IssueLabeled { repo: String, issue: u64, label: String },
    ReadyLabel,                                          // dispatch trigger — never coalesce
    Other,                                               // Telegram + unclassified GitHub — never coalesce
}

struct QueuedWebhook {
    request: MessageRequest,
    kind: WebhookEventKind,
    coalescing_key: Option<String>,
    event_id: u64,          // monotonic (AtomicU64) — replaces_event_id audit + drop-oldest ident
    enqueued_at: Instant,
}

pub struct WebhookQueue {
    inner: tokio::sync::Mutex<VecDeque<QueuedWebhook>>,
    notify: Arc<Notify>,
    max_depth: usize,
    block_timeout: Duration,
    next_event_id: AtomicU64,
}

pub enum EnqueueResult { Enqueued { depth: usize }, Coalesced { replaced_event_id: u64 }, Dropped }

impl WebhookQueue {
    pub fn new(max_depth: usize, block_timeout: Duration) -> Self;
    pub async fn enqueue(&self, req: MessageRequest) -> EnqueueResult;
    pub async fn dequeue(&self) -> Option<QueuedWebhook>;   // awaits Notify when empty
    pub async fn depth(&self) -> usize;
}

/// AC2 — pure, exhaustive-match classifier + key extractor.
pub fn classify_event(text: &str) -> WebhookEventKind;      // reuses CHECK_SUITE_RE / parse_pr_review_event / markers
pub fn coalescing_key(kind: &WebhookEventKind) -> Option<String>;
```

**Coalescing-key table (AC2)** — `coalescing_key` is an exhaustive match; `None` = never coalesce:

| Kind | Key | Rationale |
|---|---|---|
| `CheckSuite{repo,branch}` | `Some("check_suite:{repo}:{branch}")` | Generalizes mika#1869. N workflows on one branch collapse to 1. |
| `PullRequestSync{repo,pr}` | `Some("pr_sync:{repo}:{pr}")` | Sync is idempotent-by-state; newest wins. |
| `Push{repo,branch}` | `Some("push:{repo}:{branch}")` | Burst-collapse rapid pushes. |
| `IssueLabeled{repo,issue,label}` | `Some("labeled:{repo}:{issue}:{label}")` | Re-apply of same label while pending. |
| `PrReview` | `None` | Each review is user input — must not merge. |
| `ReadyLabel` | `None` | Dispatch trigger — must not merge. |
| `Other` | `None` | Telegram / unclassified — preserve order, never coalesce. |

**`check_suite` SHA caveat (must be surfaced, correctness-critical).** mika#1869 keys on
`(repo, branch, head_sha)` because a genuine second push advances `head_sha` and must NOT be conflated
with the first. **The gateway text carries `repo` and `branch` but NOT `head_sha`** (`CHECK_SUITE_RE`
captures only conclusion/repo/branch). So a `{repo}:{branch}`-only coalescing key would collapse two
distinct pushes to the same branch into one — a **missed-event** hazard, the ticket's stated primary
risk. Mitigations, in preference order:

1. **Time-bounded coalescing (chosen for v1).** Coalescing only replaces a queued sibling that is still
   *in the queue* (i.e., enqueued within the current burst window while the worker is busy). Two pushes
   seconds apart to the same branch each get processed as long as the first has already drained before
   the second arrives — which is the overwhelmingly common case because the queue drains continuously.
   The residual risk (two pushes both queued simultaneously) is bounded and no worse than mika#1869's
   own 60s window behavior at the handler layer. The downstream `ci_success_handler` re-aggregates all
   required checks on every invocation, so a coalesced-away duplicate does not lose a state transition —
   it only skips a redundant walk. Document this explicitly in the module header.
2. **(Deferred, §8) Thread `head_sha` end-to-end.** Add `head_sha` to the gateway's check_suite text
   format (companion `mika-gateway` change) and to `CheckSuite`'s key. This is the fully-correct form but
   is a cross-repo change out of scope for v1; tracked as follow-up.

The reviewer must weigh mitigation 1's residual risk; it is called out as an open question (§9).

### 3.3 Enqueue algorithm (HYBRID: coalesce → block → drop-oldest)

`enqueue(req)`:
1. `kind = classify_event(&req.text)`; `key = coalescing_key(&kind)`.
2. If `key.is_some()` and a queued entry shares that key → **remove the old entry, push the new to the
   back** (newest-wins, preserves FIFO on freshest); return `Coalesced{replaced_event_id}`. One audit
   event.
3. Else if `depth < max_depth` → push back; return `Enqueued{depth}`. One audit event.
4. Else (full) → **block up to `block_timeout` (default 100ms)** on `Notify` waiting for a drain slot;
   on wakeup retry step 3 once.
5. Still full after timeout → **drop the oldest** (`pop_front`), push the new; return `Dropped`. Audit
   event = dead-letter surface.

`dequeue()`: `pop_front`; if empty, `notify.notified().await` then retry. On any successful pop, call
`notify.notify_waiters()` so a blocked enqueuer (step 4) can proceed.

### 3.4 Drain worker (AC4)

One `tokio::spawn` loop per agent, modeled on `replay_deferred_webhooks` (`handlers.rs:780–797`) and the
KG-tick cancellation shape (mika#802):

```
loop {
    select! {
        _ = shutdown.cancelled() => break,
        item = queue.dequeue() => {
            let Some(item) = item else { continue };
            let lock = agent_state.agent_lock.clone().lock_owned().await;   // blocking, serialises
            let wait_ms = item.enqueued_at.elapsed().as_millis();
            // audit webhook_queue_dequeued
            run_agent_for_message(&state, &agent_state, item.request, imgs, lock).await;  // holds lock
        }
        _ = sleep(HEARTBEAT_5S) => { emit gauge logs (§3.6); }   // also un-wedges depth logging
    }
}
```

- The worker owns the lock acquisition; `run_agent_for_message` already holds `_lock` for the loop
  duration and drops it on return, so the next dequeue proceeds. **Never crashes the loop** — a panic in
  processing is caught/logged (`tokio::spawn` + per-iteration error handling), matching the fail-open
  discipline of the KG ticks.
- The worker needs both `AppState` and `Arc<AgentState>` (run_agent_for_message signature,
  `handlers.rs:810`). Spawn therefore happens **after** `AppState` is built.

### 3.5 Wiring the spawn site (AC4)

- **Boot:** in `run_server()`, after `state.agents` is populated and alongside the existing per-agent
  worker loop (`mod.rs:1384–1392` starts task engines; add the drain-worker spawn in the same iteration),
  create a parent `CancellationToken` (sibling to `kg_shutdown_token`, `mod.rs:893`), spawn one worker per
  agent with a child token.
- **Lazy agents (mika#1399):** in `resolve_agent()` (`state.rs:125`), after a successful lazy insert,
  spawn a drain worker for the new agent with a child of the same parent token. Store the parent token on
  `AppState` so `resolve_agent` can reach it. Emit the existing `agent_resolved_lazily` INFO plus a new
  `webhook_queue_worker_spawned` line.
- `AgentState` gains `pub webhook_queue_v2: Arc<WebhookQueue>`, constructed in `init_agent`
  (`mod.rs:544–559`) from `agent_settings.effective_webhook_queue_max_depth()` and
  `effective_webhook_queue_block_timeout_ms()`.

### 3.6 Audit instrumentation (AC5) + gauges

Five audit `tool_name`s written via `db.log_audit_event("system", tool_name, "agent:{name}", None,
Some(after), Some(reason), None)`, throttled at **max 1/sec per (agent, action)** by reusing the
`should_emit_rate_limit_audit` shape against a new `AppState.webhook_queue_audit_last: DashMap<String,
Instant>` keyed `"{agent}:{action}"`:

| tool_name | after / reasoning |
|---|---|
| `webhook_queue_enqueued` | `event_kind={k} coalescing_key={key} depth={n}` |
| `webhook_queue_coalesced` | `event_kind={k} coalescing_key={key} replaces_event_id={old}` |
| `webhook_queue_drop_oldest` | `event_kind={dropped} reason=queue_full depth={n}` |
| `webhook_queue_dequeued` | `event_kind={k} wait_ms={ms} processed_ok={bool}` |
| `webhook_queue_processing_error` | `event_kind={k} error={msg} retry_scheduled={bool}` |

**Gauges (best-effort, structured-log only, NOT a hard AC):** the worker's 5s heartbeat emits an INFO
`webhook_queue.gauge` log line with `agent_id`, `depth`, `coalesce_rate_per_min`, `drop_rate_per_min`,
`p95_wait_ms` computed from a small in-worker ring buffer. No `metrics` crate, no new dependency.
Dashboard visualization is deferred (§8). This honors the ticket's telemetry intent without inventing
gauge infrastructure the codebase lacks.

### 3.7 Config (AC6) — `crates/mika-common/src/config.rs`

Three additive `Option`-typed `Settings` fields + `DEFAULT_*` consts + `effective_*()` helpers, following
the `callback_watchdog_grace_period_secs` pattern (`config.rs:911–913,964,1317`):

| Field | Env | Default | Effective helper |
|---|---|---|---|
| `webhook_queue_max_depth: Option<usize>` | `MIKA_WEBHOOK_QUEUE_MAX_DEPTH` | `64` | `effective_webhook_queue_max_depth()` |
| `webhook_queue_block_timeout_ms: Option<u64>` | `MIKA_WEBHOOK_QUEUE_BLOCK_TIMEOUT_MS` | `100` | `effective_webhook_queue_block_timeout_ms()` |
| `webhook_queue_enabled: Option<bool>` | `MIKA_WEBHOOK_QUEUE_ENABLED` | `true` | `effective_webhook_queue_enabled()` |

Add the three to `Settings::test_defaults()` (`mika-common`, compile-forcing per convention) and to the
`test_defaults` construction in `config.rs:1529`. Invalid/≤0 numeric values fall back to default with a
`warn!` (mirrors the reaper-grace parsing discipline).

---

## 4. Implementation steps

1. **Config (mika-common).** Add the three `Settings` fields, `DEFAULT_WEBHOOK_QUEUE_*` consts, three
   `effective_*` helpers, `test_defaults()` entries. Unit tests: default + env-override for each
   (mirror `callback_watchdog_grace_period_*` tests at `config.rs:2394–2425`).
2. **New module `webhook_queue_v2.rs`.** `WebhookEventKind`, `classify_event`, `coalescing_key`,
   `QueuedWebhook`, `WebhookQueue`, `EnqueueResult`. Reuse `CHECK_SUITE_RE` (or a sibling regex) and
   `verdict::parse_pr_review_event` for classification; add regexes/markers for push, pr-sync,
   issue-labeled, ready-label. Module header documents the SHA caveat (§3.2) and the single-class scope.
   Register `mod webhook_queue_v2;` in `server/mod.rs`.
3. **AgentState field.** Add `webhook_queue_v2: Arc<WebhookQueue>` (`state.rs:26`), construct in
   `init_agent` (`mod.rs:557`-adjacent) from effective config.
4. **AppState fields.** Add `webhook_queue_audit_last: Arc<DashMap<String, Instant>>` and a parent
   `webhook_queue_shutdown: CancellationToken` (for lazy-agent worker spawn).
5. **Drain worker.** `spawn_webhook_drain_worker(state, agent_state, shutdown_child)` in
   `webhook_queue_v2.rs` (or `handlers.rs` next to `replay_deferred_webhooks`). Implements §3.4.
6. **Handler integration (AC3).** In `handle_message` (`handlers.rs:139`), after the unchanged deferral
   check: if `effective_webhook_queue_enabled()` → `match agent_state.webhook_queue_v2.enqueue(req)`:
   `Enqueued`/`Coalesced` → audit + `202 Accepted`; `Dropped` → audit + `429` (queue overflow, dead-letter
   recorded). Else (flag off) → **verbatim legacy** `try_lock_owned()` → 429 path (AC9). Remove the inline
   spawn of `run_agent_for_message` from the enabled path — the drain worker owns it now.
7. **Spawn wiring.** Boot loop in `run_server()` (`mod.rs:~1384`) + lazy path in `resolve_agent()`
   (`state.rs:125`). Parent token cancelled on shutdown alongside `kg_shutdown_token`.
8. **Tests (AC7).** See §5.
9. **Docs.** Update `crates/mika-agent/CLAUDE.md` (§ "Webhook Deferral Queue" gains a sibling
   "Bounded Webhook Queue (v2)" subsection) and root `mika/CLAUDE.md` env-var table with the three new
   vars + kill-switch + `webhook_queue_*` audit/grep signals. Run `scripts/sync-agent-docs.sh` if
   `docs/` source-of-truth files change (CI `docs-sync` gate).

---

## 5. Verification contract

**Unit / integration tests (`#[cfg(test)]` in `webhook_queue_v2.rs` + config tests):**

- **AC7 burst:** fire 100 `classify_event`-identical `check_suite.completed(repo=X,branch=Y)` into a fresh
  queue while no dequeue runs → assert depth stays 1 (99 coalesced). Assert 99 `Coalesced` + 1 `Enqueued`.
- **AC7 overflow:** enqueue 200 distinct (non-coalescing, e.g. distinct `PrReview`-shaped or `Other`)
  events into a depth-64 queue with no drain → assert 64 retained, 136 `Dropped`, and 136
  `webhook_queue_drop_oldest` audit intents recorded (no data lost to the dead-letter surface).
- **AC7 coalescing correctness:** enqueue 3 `PullRequestSync{repo,pr}` for the same PR → assert depth 1
  and the retained entry is the newest (`event_id` monotonic check).
- **AC7 drain-worker resume:** with a mock `agent_lock`, enqueue while worker parked (cancelled/not
  spawned), assert accumulation; spawn worker, assert queue drains to 0 and items processed in FIFO order.
- **Never-coalesce invariants:** `coalescing_key(PrReview) == None`, `coalescing_key(ReadyLabel) == None`,
  `coalescing_key(Other) == None`; two `PrReview` events both retained.
- **`classify_event` table test:** each gateway text format → expected `WebhookEventKind` (including a
  Telegram-shaped text → `Other`).
- **Config:** default (unset env) and env-override for all three vars; invalid value → default + warn.
- **Kill-switch:** `effective_webhook_queue_enabled()==false` → `handle_message` takes the legacy 429 path
  (assert via a handler-level test that a busy lock yields 429 and no enqueue).

**Build/lint gates (must pass):** `cargo build`, `cargo test -p mika-agent`, `cargo test -p mika-common`,
`cargo clippy`, `cargo fmt --check`. Exhaustive `match` in `coalescing_key` (no wildcard arm) so a future
`WebhookEventKind` variant fails to compile until a key decision is made (AC2 compiler-enforcement).

**Manual smoke (local):** run `mika-spirit`, POST two rapid identical `check_suite` texts to `/message`
while the agent is busy, grep `$MIKA_SPIRIT_LOG_FILE` for `webhook_queue_coalesced` and
`webhook_queue.gauge`.

---

## 6. Definition of Done

- `webhook_queue_v2.rs` implements `WebhookQueue` (`enqueue`/`dequeue`/`depth`), `classify_event`,
  `coalescing_key` (exhaustive), `EnqueueResult`.
- `POST /message` enqueues (202 on Enqueued/Coalesced, 429 only on Dropped) when the queue is enabled;
  legacy 429-reject path preserved verbatim behind `MIKA_WEBHOOK_QUEUE_ENABLED=false`.
- Per-agent drain worker spawned at boot and on lazy agent resolution; respects `agent_lock`; cancelled
  cleanly on shutdown; never crashes its loop.
- Five `webhook_queue_*` audit events emitted (throttled ≤1/sec/action/agent); 5s structured-log gauge.
- Three config vars with defaults 64 / 100ms / true, effective helpers, and tests.
- All AC7 tests pass; `cargo build/test/clippy/fmt` green; docs updated + agent-docs synced.

## Acceptance criteria

Transcribed from mika#1870 (single-class v1 scope noted where the ticket marks "v2 optional"):

- **AC1 — Bounded queue module.** `webhook_queue_v2.rs` with `WebhookQueue` (per-agent, thread-safe via
  `tokio::sync::Mutex<VecDeque<QueuedWebhook>>` + `Notify`), `enqueue -> EnqueueResult`
  (`Enqueued`/`Coalesced`/`Dropped`), async `dequeue -> Option<..>`, `depth -> usize`.
- **AC2 — Coalescing key extraction.** `coalescing_key(&WebhookEventKind) -> Option<String>`, exhaustive
  match, initial set per the §3.2 table (compiler-enforced exhaustiveness).
- **AC3 — Replace 429-reject in POST /message.** `handlers.rs` replaces the `try_lock_owned()` reject
  path with an enqueue call; `202 Accepted` on enqueue, `429` only when `Dropped` (behind the enabled
  flag; legacy path when disabled).
- **AC4 — Drain worker.** Per-agent worker spawned at `init_agent`/boot (and lazy-resolve);
  `AgentState.webhook_queue_v2` field; worker acquires `agent_lock` with a long blocking acquire.
- **AC5 — Audit-events instrumentation.** Every enqueue/coalesce/drop/dequeue/error → `audit_events` row;
  throttled max 1/sec per action per agent.
- **AC6 — Config.** `MIKA_WEBHOOK_QUEUE_MAX_DEPTH` (64), `MIKA_WEBHOOK_QUEUE_BLOCK_TIMEOUT_MS` (100),
  `MIKA_WEBHOOK_QUEUE_ENABLED` (true; false = kill-switch to 429-reject).
- **AC7 — Tests.** Burst (100 identical → depth 1), overflow (200 → 64 kept + 136 dropped w/ audit),
  drain-worker crash-recovery, coalescing correctness (3 syncs → latest only). *Priority test deferred
  with the priority-class feature (v2, §8).*
- **AC8 — Post-deploy verify (48h).** `rate_limit_trip` → near-zero (baseline 41/h → <1/h);
  `webhook_queue_coalesced` reflects real coalescing volume; `webhook_queue_drop_oldest` near-zero;
  verdicts post continuously; queue-depth gauge trends near-zero. *(Operational, post-merge.)*
- **AC9 — Kill-switch.** `MIKA_WEBHOOK_QUEUE_ENABLED=false` reverts to legacy 429-reject without redeploy.

## 8. Deferred / follow-ups

- **Priority classes** (critical/normal/defer sub-limits + AC7 priority test) — ticket-marked "v2
  optional". File as a follow-up sub-issue.
- **`head_sha` in the `check_suite` coalescing key** — requires a companion `mika-gateway` change to emit
  `head_sha` in the check_suite text format, then key `CheckSuite` on `(repo,branch,sha)` for full parity
  with mika#1869. Cross-repo; out of v1 scope (see §9 risk).
- **Dashboard gauge wiring** — a `GET /api/v1/agents/{id}/webhook-queue` endpoint + dashboard panel for
  `depth`/`coalesce_rate`/`drop_rate`/`p95_wait_ms`. v1 emits structured-log gauges only.
- **Deprecate `check_suite_dedup.rs` (mika#1869 layer 1)** — once v1 is enabled for mika-qa and AC8
  confirms coalescing volume, the handler-layer dedup becomes redundant for `check_suite`; collapse it to
  a single coalescing-key entry (the ticket's stated end-state).
- **Staged rollout** (per ticket migration path): dark-launch `ENABLED=false` → enable mika-relay →
  mika-qa → mika-dev/arch, verifying AC8 at each step. Operational, tracked outside the plan.

## 9. Risks & open questions

- **[Primary risk] Coalescing miss on same-branch double-push.** Without `head_sha` in the gateway text
  (§3.2), the time-bounded mitigation (only in-queue siblings coalesce) leaves a bounded residual: two
  pushes queued simultaneously to the same branch collapse to one. Downstream `ci_success_handler`
  re-aggregation means no *state transition* is lost, only a redundant walk — but the architect should
  confirm this is acceptable for v1 vs. blocking on the cross-repo `head_sha` thread. **Open question for
  review.**
- **Uniform-queue vs. fast-path.** §3.1 chose uniform enqueue for determinism. If the reviewer prefers to
  minimize the change to only the contended case, the alternative is a contention-triggered enqueue —
  but that reintroduces the dual-processor race. Flagged for architect disposition.
- **Lock-ownership handoff.** The drain worker must hold `agent_lock` across the whole
  `run_agent_for_message` call (as the current spawned path does). Getting this wrong would either
  serialize nothing (races) or deadlock (double-acquire). Mitigated by modeling exactly on
  `replay_deferred_webhooks` (`handlers.rs:780–797`).
- **Blast radius.** Every agent's ingestion path changes. AC9 kill-switch + staged rollout bound it.
