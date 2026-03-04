---
title: "Observability: OpenTelemetry Integration + TUI Team Dashboard"
category: architecture
tags:
  - observability
  - opentelemetry
  - tracing
  - tui
  - team-engine
  - telemetry
components:
  - mika-common/telemetry
  - mika-common/logging
  - mika-agent/agent
  - mika-agent/server
  - mika-agent/teams/engine
  - mika-cli/tui
date: 2026-03-04
severity: enhancement
resolution: implemented
---

# Observability: OpenTelemetry Integration + TUI Team Dashboard

## Problem Statement

Mika lacked structured observability infrastructure. While `tracing` was used for log output, there were no distributed trace spans for the agent loop, Claude API calls, tool execution, team orchestration, or server request handling. Additionally, team runs in the TUI provided no real-time visibility into agent progress — users had no way to see which agents were active, what phase the orchestration was in, or individual agent status.

### Symptoms

- No way to trace request latency through the agent loop pipeline
- Claude API calls had no span instrumentation for timing or error attribution
- Team orchestration was opaque — no visibility into phase transitions or per-agent progress
- TUI showed only final deliverables, not live agent activity
- No ability to export traces to external observability platforms (Langfuse, Jaeger, etc.)

## Investigation

### Approach Selection

The "always instrument, optionally export" pattern was chosen:

1. **`tracing` spans compiled unconditionally** — zero-cost when no subscriber collects them
2. **OpenTelemetry export behind `#[cfg(feature = "telemetry")]`** — no runtime cost or dependency bloat when disabled
3. **Langfuse as primary target** — OTLP-compatible, purpose-built for LLM observability

### Key Technical Decisions

- **Generic OTel layer parameter** on logging functions avoids type-level composition issues with tracing-subscriber's layered types
- **`NoopLayer` re-export** (`tracing_subscriber::layer::Identity`) allows call sites without `tracing-subscriber` dep to pass `None::<NoopLayer>`
- **`TelemetryGuard` pattern** wraps `SdkTracerProvider` with `shutdown()` on Drop for clean OTLP flush
- **`tracing::Instrument` trait** (not `EnteredSpan`) for propagating span context into `tokio::spawn` and `JoinSet::spawn` since `EnteredSpan` is `!Send`

## Root Cause

Not a bug — this was a greenfield observability implementation. The codebase had logging but no structured tracing spans or external trace export capability.

## Solution

### Phase 1: Tracing Spans (Zero-Cost Instrumentation)

Added `#[instrument]` annotations and manual spans across four layers:

**Agent loop** (`crates/mika-agent/src/agent.rs`):
```rust
#[tracing::instrument(skip_all, fields(session_id, agent_name))]
pub async fn run_agent_loop(...) -> Result<String> {
    // ...
    let tool_span = tracing::info_span!("tool_execution", tool_name = name);
    let result = tool.execute(input, &tool_ctx)
        .instrument(tool_span)
        .await;
}
```

**Claude API** (`crates/mika-common/src/claude.rs`):
```rust
#[tracing::instrument(skip_all, fields(model, max_tokens))]
pub async fn send_message(&self, request: &MessageRequest) -> Result<...> { ... }
```

**Team engine** (`crates/mika-agent/src/teams/engine.rs`):
```rust
#[tracing::instrument(skip_all, fields(team_name, goal))]
pub async fn run_team(...) -> Result<String> {
    let phase_span = tracing::info_span!("team_phase", phase = %phase);
    // Per-agent spans within JoinSet::spawn
}
```

**Server handlers** (`crates/mika-agent/src/server/handlers.rs`):
```rust
#[tracing::instrument(skip_all, fields(session_id))]
pub async fn handle_message(...) -> impl IntoResponse { ... }
```

### Phase 2: Team Engine Events + TUI Dashboard

**New `TeamPhase` enum** (`crates/mika-agent/src/teams/types.rs`):
```rust
pub enum TeamPhase {
    Decompose,
    Execute,
    Review,
    Deliver,
    ReDecompose,
}
```

**New `TeamEvent` variants**:
- `PhaseChanged { phase, description }` — orchestration phase transitions
- `AgentStarted { agent_name, task_summary }` — per-agent work begins
- `AgentCompleted { agent_name, task_summary }` — per-agent work ends
- `AllAgentsCompleted { phase }` — all agents in a phase finished

**TUI split-pane dashboard** (`crates/mika-cli/src/tui/ui.rs`):
When terminal width >= 80 columns, renders a 70/30 split layout showing conversation on the left and a live dashboard on the right with current phase, agent status (spinner/checkmark), and timing.

### Phase 3: Feature-Flagged OpenTelemetry Export

**Telemetry module** (`crates/mika-common/src/telemetry.rs`):
```rust
#[cfg(feature = "telemetry")]
pub fn build_otel_layer<S>(settings: &Settings) -> Option<(OpenTelemetryLayer<S, SdkTracer>, TelemetryGuard)>
where
    S: Subscriber + for<'span> LookupSpan<'span>,
{
    // Checks MIKA_TELEMETRY_ENABLED, builds OTLP exporter with auth header
    // Returns None gracefully if not configured
}
```

**Wired into logging** (`crates/mika-common/src/logging.rs`):
```rust
pub fn init<OL>(default_level: &str, log_file: Option<&Path>, otel_layer: Option<OL>) -> Option<WorkerGuard>
where
    OL: Layer<Registry> + Send + Sync + 'static,
{
    // OTel layer composed BEFORE filter: registry().with(otel_layer).with(filter).with(fmt)
}
```

**Server entry point** (`crates/mika-agent/src/bin/mika-server.rs`):
```rust
#[cfg(feature = "telemetry")]
let (otel_layer, _telemetry_guard) = match mika_common::telemetry::build_otel_layer(&settings) {
    Some((layer, guard)) => (Some(layer), Some(guard)),
    None => (None, None),
};
#[cfg(not(feature = "telemetry"))]
let otel_layer = None::<mika_common::logging::NoopLayer>;
```

### Configuration

Three new env vars / config fields:
- `MIKA_TELEMETRY_ENABLED` — master switch (default: false)
- `MIKA_OTLP_ENDPOINT` — OTLP HTTP endpoint with `/v1/traces` path (e.g., `https://cloud.langfuse.com/api/public/otel/v1/traces` or `http://localhost:4318/v1/traces`)
- `MIKA_OTLP_AUTH_HEADER` — Base64-encoded auth header (redacted in Debug output)

Build with: `cargo build --features telemetry`

## Prevention & Best Practices

### Adding New Spans Checklist

- [ ] Use `#[instrument(skip_all, fields(...))]` for top-level async functions
- [ ] Add meaningful field names (session_id, agent_name, tool_name, model)
- [ ] For `tokio::spawn` / `JoinSet::spawn`: use `.instrument(span)` — NOT `span.enter()` (which is `!Send`)
- [ ] Record dynamic values with `tracing::Span::current().record("key", value)`
- [ ] Keep span names stable (they become metric labels in OTLP)

### Async Span Gotchas

- `EnteredSpan` is `!Send` — cannot cross `.await` points or be held across `tokio::spawn`
- Use `tracing::Instrument` trait: `future.instrument(span).await`
- For `JoinSet::spawn`, create the span outside, `.instrument()` inside the closure
- Never clone spans into multiple concurrent tasks (creates confusing parent-child relationships)

### Feature Flag Maintenance

- `telemetry` feature propagates: `mika-cli -> mika-common/telemetry`, `mika-agent -> mika-common/telemetry`
- Workspace deps are NOT optional — optionality is at crate feature level
- All `#[cfg(feature = "telemetry")]` blocks must have a matching `#[cfg(not(...))]` fallback
- Test both `cargo test` and `cargo test --features telemetry`

### Testing Guidance

- Spans are zero-cost without subscriber — no test changes needed for Phase 1
- `TeamEvent` variants require exhaustive match — compiler catches missing arms
- TUI dashboard testable via `TeamDashboardState` struct (pure data, no rendering)
- OTel integration tests: use `opentelemetry_sdk::testing::InMemorySpanExporter`

## Cross-References

- **Plan document**: `docs/plans/2026-03-04-feat-observability-otel-tui-dashboard-plan.md`
- **ADR**: `docs/adr/` — consider adding ADR for "always instrument, optionally export" pattern
- **Configuration docs**: `docs/configuration.md` (telemetry settings section)
- **GitHub Issues**: #30 (structured logging), #48 (Langfuse integration), #53 (team observability)

## Files Changed

| File | Change |
|------|--------|
| `crates/mika-common/src/telemetry.rs` | New — OTel layer builder with TelemetryGuard |
| `crates/mika-common/src/logging.rs` | Generic OTel layer parameter, NoopLayer re-export |
| `crates/mika-common/src/config.rs` | Telemetry settings fields |
| `crates/mika-common/Cargo.toml` | Feature-gated OTel dependencies |
| `crates/mika-agent/src/agent.rs` | Agent loop + tool execution spans |
| `crates/mika-agent/src/server/handlers.rs` | Request handler spans |
| `crates/mika-agent/src/teams/engine.rs` | Phase/agent spans + TeamEvent emissions |
| `crates/mika-agent/src/teams/types.rs` | TeamPhase enum, new TeamEvent variants |
| `crates/mika-cli/src/tui/app.rs` | TeamDashboardState, event handler |
| `crates/mika-cli/src/tui/ui.rs` | Split-pane dashboard renderer |
| `crates/mika-agent/src/bin/mika-server.rs` | Wired OTel into subscriber |
| `crates/mika-gateway/src/main.rs` | NoopLayer parameter |
| `.env.example` | Telemetry env vars |
