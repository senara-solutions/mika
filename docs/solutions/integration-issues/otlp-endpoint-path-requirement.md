---
title: "OpenTelemetry OTLP endpoint requires full /v1/traces path in opentelemetry-otlp 0.31"
date: "2026-03-04"
category: "integration-issues"
tags: [opentelemetry, otlp, langfuse, jaeger, rust, tracing, configuration]
severity: "medium"
component: "mika-common/telemetry"
affects: ["mika-cli", "mika-spirit", "mika-common"]
status: "resolved"
---

# OTLP Endpoint Requires Full `/v1/traces` Path

## Problem Symptom

Traces were not appearing in Jaeger or Langfuse despite telemetry being enabled and all environment variables set correctly (`MIKA_TELEMETRY_ENABLED=true`, `MIKA_OTLP_ENDPOINT`, `MIKA_OTLP_AUTH_HEADER`). The tracing infrastructure initialized without error, but no trace data reached the backend. Silent failure with no error messages.

## Investigation Steps

1. **Verified environment configuration** - Confirmed all three telemetry env vars were set to expected values
2. **Set up local Jaeger** - Deployed `jaeger-all-in-one` container, verified accessible at `http://localhost:16686`
3. **Ran agent with telemetry enabled** - Started `mika` CLI with `--features telemetry`
4. **Checked Jaeger UI** - Only `jaeger-all-in-one` service appeared; no Mika traces
5. **Researched opentelemetry-otlp 0.31** - Discovered `.with_endpoint()` uses the URL exactly as provided, does NOT auto-append `/v1/traces`
6. **Added `/v1/traces` to endpoint** - Traces immediately appeared in Jaeger
7. **Verified with Langfuse** - Same fix: `https://cloud.langfuse.com/api/public/otel/v1/traces`

## Root Cause

The `opentelemetry-otlp` 0.31 crate's `HttpExporterBuilder::with_endpoint()` treats the provided URL as the **complete** endpoint path. It does NOT auto-append `/v1/traces`.

Many OTLP backend docs show base URLs (e.g., `https://cloud.langfuse.com/api/public/otel`), creating the false impression that the exporter handles path completion. The Rust exporter does not - it sends HTTP POST requests to whatever URL you provide. When the path is wrong, the request silently fails or gets a non-200 response that the exporter swallows.

## Working Solution

### 1. Correct Endpoint URLs

Always include the full path:

```bash
# Jaeger (local)
MIKA_OTLP_ENDPOINT=http://localhost:4318/v1/traces

# Langfuse (cloud)
MIKA_OTLP_ENDPOINT=https://cloud.langfuse.com/api/public/otel/v1/traces
```

### 2. Code Refactoring

Extracted `try_init_otel()` helper with cfg-gated variants to reduce duplication across CLI and server binaries:

```rust
#[cfg(feature = "telemetry")]
pub fn try_init_otel(settings: &Settings) -> (Option<impl Layer<S>>, Option<TelemetryGuard>) {
    match build_otel_layer(settings) {
        Some((layer, guard)) => (Some(layer), Some(guard)),
        None => (None, None),
    }
}

#[cfg(not(feature = "telemetry"))]
pub fn try_init_otel(_settings: &Settings) -> (Option<tracing::subscriber::NoopLayer>, Option<()>) {
    (None, None)
}
```

### 3. CI Integration

Added telemetry-specific test step to catch regressions:

```yaml
- name: Test (with telemetry feature)
  run: cargo test --workspace --features telemetry
```

## What Didn't Work

- **Assuming the exporter auto-appends paths** - Many backend docs show base URLs, but `opentelemetry-otlp` 0.31 does not add any path segments
- **Relying on error messages** - The exporter silently swallows failures when hitting the wrong path
- **Using `eprintln!` diagnostics** - Added temporary stderr printing to debug; traces still didn't appear because the endpoint itself was wrong

## Prevention Strategies

1. **Always use full OTLP endpoint paths in examples and documentation** - Never assume the client library appends path segments
2. **Test with a local Jaeger instance** before configuring cloud backends - faster feedback loop
3. **Log the full endpoint URL at initialization** - Makes debugging trivial:
   ```rust
   tracing::info!(endpoint = %url, "OpenTelemetry OTLP export initialized");
   ```
4. **Pin and document tested opentelemetry-otlp versions** - Endpoint behavior can change between versions

## Related Documentation

- [Observability OTel + TUI Dashboard](../architecture/observability-otel-tui-dashboard.md) - Broader observability implementation
- [Configuration Reference](../../configuration.md) - Telemetry env vars section
- `.env.example` - Endpoint examples with full paths

## Key Takeaway

When integrating with OTLP backends, always verify the **complete HTTP endpoint path** required by your exporter library. Consult the exporter's source code or run a network trace if behavior seems inconsistent with documentation.
