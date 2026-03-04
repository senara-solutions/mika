---
title: "feat: Observability — OpenTelemetry/Langfuse + TUI Team Dashboard"
type: feat
status: active
date: 2026-03-04
---

# Observability — OpenTelemetry/Langfuse + TUI Team Dashboard

## Overview

Add structured observability to Mika across all execution modes: single-agent conversations, team runs, server endpoints, and background tasks. Two complementary systems:

1. **OpenTelemetry + Langfuse** — structured trace export for LLM calls, tool execution, team phases, and HTTP requests. Feature-flagged with zero cost when disabled.
2. **TUI Team Dashboard** — live split-pane showing agent status grid, phase indicator, and activity log during team runs.

## Problem Statement

Current observability is limited to `tracing` log macros with only 2 manual spans in the entire codebase. There is no timing for LLM calls or tool execution, no trace export, no metrics, and no structured visibility into team run progress. The TUI renders team events as flat text with no dashboard view.

**Gaps identified:**
- Zero spans on `run_agent()`, `run_loop()`, `claude.send_message()`, `execute_tool()`
- No `#[instrument]` proc-macro usage anywhere
- `tower-http` `trace` feature enabled but `TraceLayer` never applied to Axum router
- No OpenTelemetry integration (no OTLP, no Jaeger, no Langfuse)
- No correlation IDs threading through team engine (only server handler has `request_id`)
- Team engine `JoinSet::spawn` sites lack `.instrument()` — trace context lost across spawn boundaries
- No observability for silent mode (heartbeat, reminders, reflection) or compaction LLM calls

## Proposed Solution

### Architecture: "Always Instrument, Optionally Export"

Use `tracing` spans unconditionally (zero-cost when no subscriber consumes them). Put only the OTel exporter and Langfuse SDK behind `#[cfg(feature = "telemetry")]`. This avoids conditional compilation of instrumentation code while keeping the dependency tree lean.

```
┌─────────────────────────────────────────────────┐
│                tracing spans                     │
│  (always compiled, zero-cost without subscriber) │
├─────────────────────────────────────────────────┤
│   fmt layer (existing)  │  OTel layer (optional) │
│   JSON/pretty output    │  #[cfg(feature)]       │
│                         │  Langfuse exporter     │
└─────────────────────────┴────────────────────────┘
```

## Technical Approach

### Phase 1: Span Instrumentation (No New Dependencies)

Add `tracing` spans to all critical code paths. These are zero-cost when no OTel subscriber is active and immediately improve structured log output.

#### 1.1 Agent Loop Spans

**File:** `crates/mika-agent/src/agent.rs`

```rust
// Top-level conversation entry
#[tracing::instrument(skip_all, fields(
    session_id = %session_id,
    agent = %agent_name,
    mode = "conversation"
))]
pub async fn run_agent(params: &AgentParams<'_>) -> Result<AgentOutput> { ... }

// Shared tool-step loop
#[tracing::instrument(skip_all, fields(mode = %mode_label, max_steps))]
async fn run_loop(...) -> Result<LoopResult> {
    for step in 0..max_steps {
        let _step_span = tracing::info_span!("agent_step", step).entered();
        // ... claude.send_message + process_tool_calls
    }
}

// Tool dispatch
#[tracing::instrument(skip_all, fields(tool = %name))]
async fn execute_tool(...) -> ToolOutput { ... }
```

#### 1.2 Claude API Client Spans

**File:** `crates/mika-common/src/claude.rs`

```rust
#[tracing::instrument(skip_all, fields(
    model = %request.model,
    max_tokens = request.max_tokens,
    // Populated after response:
    input_tokens = tracing::field::Empty,
    output_tokens = tracing::field::Empty,
    stop_reason = tracing::field::Empty,
))]
pub async fn send_message(&self, request: &Request) -> Result<Response, ClaudeApiError> {
    // After successful response:
    Span::current().record("input_tokens", usage.input_tokens);
    Span::current().record("output_tokens", usage.output_tokens);
    Span::current().record("stop_reason", &stop_reason);
}
```

#### 1.3 Team Engine Spans

**File:** `crates/mika-agent/src/teams/engine.rs`

```rust
// Root span for team run
#[tracing::instrument(skip_all, fields(
    team = %self.run.team_name,
    run_id = %self.run.id,
    goal = tracing::field::Empty
))]
pub async fn execute(&mut self) -> Result<String> { ... }

// Phase spans
async fn decompose(&mut self, ...) {
    let _span = tracing::info_span!("team_phase", phase = "decompose", iteration).entered();
    // ...
}

async fn execute_tasks(&mut self, ...) {
    let _span = tracing::info_span!("team_phase", phase = "execute", task_count).entered();
    // Each JoinSet spawn MUST carry span context:
    let agent_span = tracing::info_span!("team_agent_task", agent = %name, role = %role);
    join_set.spawn(async move { ... }.instrument(agent_span));
}

async fn review(&mut self, ...) {
    let _span = tracing::info_span!("team_phase", phase = "review", iteration).entered();
}

async fn deliver(&mut self, ...) {
    let _span = tracing::info_span!("team_phase", phase = "deliver").entered();
}
```

#### 1.4 All `tokio::spawn` Sites

Add `.instrument()` to every spawn that currently lacks it:

| File | Line | Spawn | Span to add |
|------|------|-------|-------------|
| `server/handlers.rs` | ~177 | `handle_message` agent loop | Already has `process_message` span |
| `teams/engine.rs` | ~504 | `JoinSet::spawn` in `execute_tasks` | `team_agent_task` (see 1.3) |
| `server/handlers.rs` | ~169 | `flush_failed_sends` | `flush_failed_sends` |
| `server/handlers.rs` | ~252 | compaction spawn | `compaction` |
| `scheduler.rs` | poller spawns | reminder firing | `reminder_fire` |

#### 1.5 Tower-HTTP TraceLayer

**File:** `crates/mika-agent/src/server/mod.rs`

```rust
use tower_http::trace::TraceLayer;

fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/message", post(handlers::handle_message))
        .route("/heartbeat", post(handlers::handle_heartbeat))
        .layer(TraceLayer::new_for_http())  // <-- add this
        .layer(RequestBodyLimitLayer::new(10 * 1024 * 1024))
        // ...
}
```

### Phase 2: New TeamEvent Variants + TUI Dashboard

#### 2.1 `TeamPhase` Enum and New Events

**File:** `crates/mika-agent/src/teams/types.rs`

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TeamPhase {
    Decompose,
    Execute,
    Review,
    Deliver,
    ReDecompose,
}

impl fmt::Display for TeamPhase {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Decompose => write!(f, "decomposing"),
            Self::Execute => write!(f, "executing"),
            Self::Review => write!(f, "reviewing"),
            Self::Deliver => write!(f, "delivering"),
            Self::ReDecompose => write!(f, "re-decomposing"),
        }
    }
}

// Add to TeamEvent enum:
PhaseChanged { phase: TeamPhase, iteration: u32 },
AgentStarted { agent: String, role: String },
```

These supplement (not replace) existing `Progress` events. `PhaseChanged` is emitted once per phase transition; `Progress` continues for freeform status messages.

#### 2.2 Emit Events in Engine

**File:** `crates/mika-agent/src/teams/engine.rs`

- `PhaseChanged { phase: Decompose, iteration }` before `decompose()`
- `AgentStarted { agent, role }` before each `JoinSet::spawn` in `execute_tasks()`
- `PhaseChanged { phase: Execute, iteration }` before `execute_tasks()`
- `PhaseChanged { phase: Review, iteration }` before `review()`
- `PhaseChanged { phase: Deliver, iteration }` before `deliver()`
- `PhaseChanged { phase: ReDecompose, iteration }` before re-decompose with feedback

#### 2.3 `TeamDashboardState`

**File:** `crates/mika-cli/src/tui/app.rs`

```rust
pub struct TeamDashboardState {
    pub phase: TeamPhase,
    pub iteration: u32,
    pub agents: Vec<AgentEntry>,
    pub activity_log: Vec<ActivityEntry>,
    pub run_started: Instant,
}

pub struct AgentEntry {
    pub name: String,
    pub role: String,
    pub status: AgentStatus,  // Pending, Running, Completed, Failed
    pub started_at: Option<Instant>,
}

pub struct ActivityEntry {
    pub timestamp: Instant,
    pub message: String,
}
```

- Created when `PhaseChanged` is first received
- Updated by `tick_team_mode()` handling new event variants
- Cleared on `Deliverable`/`RunFailed`/channel disconnect

#### 2.4 Update `tick_team_mode()`

**File:** `crates/mika-cli/src/tui/app.rs`

Add exhaustive match arms for `PhaseChanged` and `AgentStarted`:

```rust
TeamEvent::PhaseChanged { phase, iteration } => {
    if let Some(ref mut dash) = self.team_dashboard {
        dash.phase = phase;
        dash.iteration = iteration;
        // Reset agent statuses for new phase
    }
}
TeamEvent::AgentStarted { agent, role } => {
    if let Some(ref mut dash) = self.team_dashboard {
        dash.agents.push(AgentEntry {
            name: agent.clone(),
            role: role.clone(),
            status: AgentStatus::Running,
            started_at: Some(Instant::now()),
        });
    }
}
```

#### 2.5 Split-Pane TUI Layout

**File:** `crates/mika-cli/src/tui/ui.rs`

When `app.team_dashboard.is_some()` AND terminal width >= 120 cols:

```
┌──────────────────────────────┬───────────────┐
│                              │ Phase: Execute │
│    Messages (70%)            │ Iter: 2/3     │
│                              │               │
│    [system] Decomposing...   │ Agents:       │
│    [system] 3 tasks assigned │ ✓ researcher  │
│    [system] Running writer   │ ... writer    │
│                              │ ○ critic      │
│                              │               │
│                              │ Elapsed: 45s  │
├──────────────────────────────┴───────────────┤
│ agents: 1/3 | elapsed: 45s | phase: execute │
│ > [input area]                               │
└──────────────────────────────────────────────┘
```

- Below 120 cols: dashboard panel hidden, state preserved in memory
- Terminal resize: re-check width on each `draw()` call (ratatui already provides current frame size)
- Message scroll offset adjusted when width changes (use new `available_width` for `visual_line_rows()`)
- Dashboard has fixed height sections, activity log scrolls if overflow
- `available_width` for input wrapping must use message area width (not full terminal)
- Autocomplete popup renders above dashboard (existing z-order is fine since it draws last)

#### 2.6 Footer Enhancement

During team runs, replace the standard footer with:

```
agents: 2/4 | elapsed: 45s | phase: executing (iter 2)
```

### Phase 3: OpenTelemetry + Langfuse Export (Feature-Flagged)

#### 3.1 Feature Flag

**Files:** workspace `Cargo.toml`, `crates/mika-common/Cargo.toml`, `crates/mika-agent/Cargo.toml`, `crates/mika-cli/Cargo.toml`

```toml
# Workspace Cargo.toml [workspace.dependencies]
opentelemetry = { version = "0.31", optional = true }
opentelemetry_sdk = { version = "0.31", optional = true, features = ["rt-tokio"] }
opentelemetry-otlp = { version = "0.31", optional = true }
tracing-opentelemetry = { version = "0.31", optional = true }

# mika-common Cargo.toml
[features]
telemetry = ["dep:opentelemetry", "dep:opentelemetry_sdk", "dep:opentelemetry-otlp", "dep:tracing-opentelemetry"]

# mika-agent Cargo.toml
[features]
telemetry = ["mika-common/telemetry"]

# mika-cli Cargo.toml
[features]
telemetry = ["mika-common/telemetry"]
```

**Note on Langfuse:** Use OTLP export to Langfuse's OTLP endpoint rather than the `opentelemetry-langfuse` crate. This keeps the dependency surface smaller and works with any OTLP-compatible backend (Jaeger, Grafana Tempo, etc.).

#### 3.2 Configuration

**File:** `crates/mika-common/src/config.rs`

```rust
// New fields on Settings (all optional)
pub telemetry_enabled: bool,          // MIKA_TELEMETRY_ENABLED (default: false)
pub otlp_endpoint: Option<String>,    // MIKA_OTLP_ENDPOINT (e.g. "https://cloud.langfuse.com/api/public/otel")
pub otlp_auth_header: Option<String>, // MIKA_OTLP_AUTH_HEADER (Base64 encoded public:secret for Langfuse)
```

**Secret handling:** `otlp_auth_header` must be added to the manual `Debug` impl redaction list alongside `anthropic_api_key` and `internal_token`.

**Validation:** When `telemetry_enabled = true` but `otlp_endpoint` is missing, log a warning and continue without OTel export (graceful degradation).

**Config cascade:** Per-agent config can override (agent-level `config.toml` takes precedence). Env vars always win: `MIKA_TELEMETRY_ENABLED=true`.

#### 3.3 Telemetry Init Module

**New file:** `crates/mika-common/src/telemetry.rs`

```rust
#[cfg(feature = "telemetry")]
pub fn build_otel_layer(settings: &Settings) -> Option<(impl Layer<S>, TelemetryGuard)>
where S: Subscriber + for<'a> LookupSpan<'a> {
    if !settings.telemetry_enabled { return None; }
    let endpoint = settings.otlp_endpoint.as_ref()?;

    let exporter = opentelemetry_otlp::SpanExporter::builder()
        .with_http()
        .with_endpoint(endpoint)
        .with_headers(/* auth header if configured */)
        .build()?;

    let tracer = opentelemetry_sdk::trace::SdkTracerProvider::builder()
        .with_batch_exporter(exporter)
        .with_config(Config::default()
            .with_resource(Resource::new(vec![
                KeyValue::new("service.name", "mika-agent"),
            ])))
        .build();

    let layer = tracing_opentelemetry::layer().with_tracer(tracer.tracer("mika"));

    Some((layer, TelemetryGuard(tracer)))
}

pub struct TelemetryGuard(SdkTracerProvider);
impl Drop for TelemetryGuard {
    fn drop(&mut self) {
        // Flush remaining spans with 5s timeout
        let _ = self.0.shutdown();
    }
}

#[cfg(not(feature = "telemetry"))]
pub fn build_otel_layer(_settings: &Settings) -> Option<()> { None }
```

#### 3.4 Logging Integration

**File:** `crates/mika-common/src/logging.rs`

Modify `init()` and `init_pretty()` to accept an optional OTel layer:

```rust
pub fn init(
    default_level: &str,
    log_file: Option<&Path>,
    #[cfg(feature = "telemetry")] otel_layer: Option<impl Layer<S>>,
) -> Option<WorkerGuard> {
    let subscriber = tracing_subscriber::registry()
        .with(env_filter)
        .with(fmt_layer);

    #[cfg(feature = "telemetry")]
    let subscriber = subscriber.with(otel_layer);

    subscriber.init();
    // ...
}
```

**TUI mode constraint:** The OTel exporter must NOT write to stderr (would corrupt ratatui). The OTLP HTTP exporter uses reqwest internally and does not write to stderr — safe to use with `LogOutput::FileOnly`.

#### 3.5 Init Entry Points

- **CLI** (`crates/mika-cli/src/main.rs`): Build OTel layer after settings load, pass to `init_pretty()`, store `TelemetryGuard` in main scope
- **Server** (`crates/mika-agent/src/bin/mika-server.rs`): Build OTel layer, pass to `init()`, store guard. Flush on `shutdown_signal()`

#### 3.6 Exporter Configuration (Bounded)

Hardcoded sensible defaults for the batch exporter:
- Batch size: 512
- Queue size: 2048
- Export timeout: 5s
- On queue overflow: drop oldest spans
- On startup failure (bad endpoint/auth): log warning, continue without OTel

This ensures Langfuse unavailability does not prevent agent operation and does not cause OOM in the 256MB container target.

## Target Span Hierarchy

### Single-Agent Conversation (CLI/Server)

```
agent_turn (session_id, agent, mode=conversation)
├── agent_step[0]
│   └── llm_call (model, input_tokens, output_tokens, stop_reason)
├── agent_step[1]
│   ├── llm_call (model, ...)
│   └── tool_execution (tool=web_search)
├── agent_step[2]
│   ├── llm_call (model, ...)
│   └── tool_execution (tool=store_fact)
└── agent_step[3]
    └── llm_call (model, ..., stop_reason=end_turn)
```

### Team Run

```
team_run (team, run_id, goal)
├── team_phase (phase=decompose, iteration=1)
│   └── team_agent_task (agent=orchestrator)
│       └── llm_call
├── team_phase (phase=execute, iteration=1)
│   ├── team_agent_task (agent=researcher, role=analyst)
│   │   ├── llm_call
│   │   ├── tool_execution (tool=web_search)
│   │   └── llm_call
│   └── team_agent_task (agent=writer, role=content)
│       ├── llm_call
│       └── tool_execution (tool=write_file)
├── team_phase (phase=review, iteration=1)
│   └── team_agent_task (agent=critic)
│       └── llm_call
└── team_phase (phase=deliver, iteration=1)
    └── team_agent_task (agent=writer)
        └── llm_call
```

### Server HTTP Request

```
HTTP request (tower-http TraceLayer: method, uri, status, latency)
└── process_message (request_id)
    └── agent_turn (session_id, agent, mode=conversation)
        └── ... (same as single-agent hierarchy)
```

## System-Wide Impact

### Interaction Graph

Adding spans touches: `agent.rs` (run_agent, run_loop, execute_tool, process_tool_calls), `claude.rs` (send_message), `engine.rs` (execute, decompose, execute_tasks, review, deliver), `server/mod.rs` (TraceLayer), `server/handlers.rs` (spawn sites), `logging.rs` (subscriber composition). The `tracing` crate is already a universal dependency — no new imports needed for Phase 1.

### Error Propagation

OTel export errors are fire-and-forget (batch exporter). They do not propagate to the agent loop or affect user-facing behavior. The `TelemetryGuard` flush on drop uses a 5s timeout — if it times out, spans are lost silently (acceptable for shutdown).

### State Lifecycle Risks

- `TeamDashboardState` is created on first `PhaseChanged` event and cleared on terminal events (Deliverable/RunFailed/disconnect). No orphan risk.
- `TelemetryGuard` is held in the binary's `main()` scope — dropped on process exit. No orphan risk.

### API Surface Parity

Adding `PhaseChanged` and `AgentStarted` to `TeamEvent` is a breaking change for exhaustive matches. All match sites must be updated:
- `crates/mika-cli/src/tui/app.rs` (`tick_team_mode`)
- `crates/mika-cli/src/commands/teams.rs` (CLI team runner callback)
- `crates/mika-agent/src/teams/engine.rs` (`emit_event` logging)

### Integration Test Scenarios

1. **Team run with OTel enabled:** Run team, verify Langfuse receives traces with correct parent-child hierarchy
2. **Server request with TraceLayer:** POST to /message, verify HTTP span wraps agent_turn span
3. **Feature flag off:** `cargo build` without `telemetry` feature — no OTel deps in binary
4. **Terminal resize during team run:** Dashboard appears/disappears as width crosses 120 cols
5. **Langfuse unreachable:** Agent continues functioning, warning logged, spans dropped

## Implementation Order

| Phase | What | Dependencies | Estimated scope |
|-------|------|-------------|-----------------|
| 1 | Span instrumentation (agent loop, Claude client, team engine, spawn sites, TraceLayer) | None — pure `tracing` additions | ~15 files, ~100 lines |
| 2 | New TeamEvent variants + TUI dashboard | Phase 1 spans provide richer data | ~6 files, ~300 lines |
| 3 | OTel/Langfuse export (feature-flagged) | Phase 1 spans to export | ~8 files, ~200 lines (new code) |

Phases 1 and 2 can be done on the same branch. Phase 3 can be a separate branch/PR.

## Acceptance Criteria

### Phase 1: Span Instrumentation
- [ ] `run_agent` wrapped in `agent_turn` span with session_id, agent, mode fields
- [ ] `run_loop` has per-step `agent_step` spans
- [ ] `execute_tool` has `tool_execution` span with tool name
- [ ] `claude.send_message` has `llm_call` span with model, tokens, stop_reason
- [ ] `TeamEngine::execute` has `team_run` root span
- [ ] Phase methods have `team_phase` spans (decompose, execute, review, deliver)
- [ ] All `JoinSet::spawn` sites use `.instrument()` for context propagation
- [ ] `TraceLayer::new_for_http()` added to Axum router
- [ ] `cargo test` passes (~837 tests)
- [ ] `cargo clippy` clean

### Phase 2: TUI Dashboard
- [ ] `TeamPhase` enum defined with Display impl
- [ ] `PhaseChanged` and `AgentStarted` variants added to `TeamEvent`
- [ ] Events emitted at correct points in engine
- [ ] `TeamDashboardState` struct in app.rs
- [ ] `tick_team_mode()` handles all new variants (exhaustive match)
- [ ] Split-pane layout renders at >= 120 cols
- [ ] Dashboard hides gracefully below 120 cols (state preserved)
- [ ] Footer shows agent count, elapsed, phase during team runs
- [ ] Input wrapping uses correct `available_width`
- [ ] CLI team runner (`teams.rs`) handles new variants

### Phase 3: OTel/Langfuse
- [ ] `telemetry` feature flag in workspace, mika-common, mika-agent, mika-cli
- [ ] Config fields: `telemetry_enabled`, `otlp_endpoint`, `otlp_auth_header`
- [ ] `otlp_auth_header` redacted in `Settings` Debug impl
- [ ] `telemetry.rs` module with `build_otel_layer()` + `TelemetryGuard`
- [ ] `logging.rs` composes OTel layer when feature enabled
- [ ] CLI and server init OTel at startup, store guard
- [ ] `cargo build` (no features) compiles cleanly — no OTel deps
- [ ] `cargo build --features telemetry` compiles and exports to OTLP endpoint
- [ ] Graceful degradation: missing endpoint logs warning, agent continues
- [ ] Bounded exporter: queue=2048, batch=512, timeout=5s

## Critical Files

| File | Changes |
|------|---------|
| `crates/mika-agent/src/agent.rs` | Spans on run_agent, run_loop, execute_tool, process_tool_calls |
| `crates/mika-common/src/claude.rs` | Span on send_message with token/model fields |
| `crates/mika-agent/src/teams/engine.rs` | Phase spans, JoinSet .instrument(), emit new events |
| `crates/mika-agent/src/teams/types.rs` | TeamPhase enum, PhaseChanged + AgentStarted variants |
| `crates/mika-cli/src/tui/app.rs` | TeamDashboardState, tick_team_mode new variants |
| `crates/mika-cli/src/tui/ui.rs` | Split-pane layout, draw_team_dashboard, footer |
| `crates/mika-cli/src/commands/chat.rs` | Callback handles new events |
| `crates/mika-cli/src/commands/teams.rs` | CLI team runner handles new events |
| `crates/mika-agent/src/server/mod.rs` | TraceLayer on router |
| `crates/mika-agent/src/server/handlers.rs` | .instrument() on spawn sites |
| `crates/mika-common/src/config.rs` | Telemetry config fields + redaction |
| `crates/mika-common/src/telemetry.rs` | **New:** OTel layer builder + guard |
| `crates/mika-common/src/logging.rs` | Compose OTel layer into subscriber |
| `crates/mika-common/src/lib.rs` | Expose telemetry module |
| Workspace + crate Cargo.toml files | Feature flags + optional deps |
| `config/default.toml` | Default telemetry config values |

## Deferred (Future Work)

- **Metrics** (counters, histograms, gauges) — separate plan after tracing is stable
- **Gateway instrumentation** — mika-gateway OTel integration for end-to-end Telegram traces
- **Silent mode / scheduler spans** — heartbeat, reminder, reflection instrumentation (low priority, can be added incrementally after Phase 1 patterns are established)
- **MCP tool span attributes** — distinguish MCP tool calls from builtin tools in traces

## Verification

1. **Phase 1:** Run `RUST_LOG=debug cargo run --bin mika`, send a message, verify structured spans in log output (agent_turn, agent_step, llm_call, tool_execution)
2. **Phase 2:** `cargo run --bin mika -- --team dev-team`, send a goal, verify split-pane dashboard with agent status grid and phase transitions
3. **Phase 3:** `MIKA_TELEMETRY_ENABLED=true MIKA_OTLP_ENDPOINT=https://cloud.langfuse.com/api/public/otel cargo run --bin mika --features telemetry`, verify traces appear in Langfuse with correct parent-child hierarchy
4. **Feature flag isolation:** `cargo build` (no features) — no OTel deps, clean compile
5. **Resize:** During team run, resize terminal below/above 120 cols — dashboard toggles without state loss

## Sources & References

### Internal References
- Original sketch: `~/.claude/plans/parallel-wobbling-sketch.md`
- Team event types: `crates/mika-agent/src/teams/types.rs`
- Team engine: `crates/mika-agent/src/teams/engine.rs`
- Agent loop: `crates/mika-agent/src/agent.rs`
- Claude client: `crates/mika-common/src/claude.rs`
- Logging: `crates/mika-common/src/logging.rs`
- Server router: `crates/mika-agent/src/server/mod.rs`
- Finding #434 (async span fix): `todos/434-complete-p2-span-enter-in-async-function.md`
- Team persistence solution: `docs/solutions/database-issues/team-graph-persistence-replacing-toml-history.md`
- Background agent checklist: `docs/solutions/code-review-patterns/background-agent-mode-design-checklist.md`

### GitHub Issues (No Correlation)
- Issues #60-64 are onboarding/setup focused — no overlap with observability work
