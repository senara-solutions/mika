---
status: complete
priority: p2
issue_id: "453"
tags: [code-review, architecture, telemetry]
dependencies: []
---

# Duplicated OTel Wiring Boilerplate at 3 Call Sites

## Problem Statement

The 10-line `#[cfg(feature)]` / `#[cfg(not(feature))]` block for building the OTel layer and guard appears 3 times: `mika-server.rs`, `main.rs` (team mode), and `main.rs` (agent mode). The `NoopLayer` re-export exists solely to serve these call sites.

## Findings

- **Source**: Code simplicity reviewer + architecture strategist
- **Locations**: `crates/mika-cli/src/main.rs:35-47`, `crates/mika-cli/src/main.rs:112-126`, `crates/mika-agent/src/bin/mika-server.rs:15-26`
- **Evidence**: ~40 lines of identical cfg-gated boilerplate

## Proposed Solutions

### Option A: Internalize OTel layer in logging functions (Recommended)
Pass `Option<&Settings>` to `init`/`init_pretty` and let them call `build_otel_layer` internally. Return a combined guard type. Eliminates the generic `<OL>` parameter, `NoopLayer` re-export, and all call-site cfg blocks.
- **Pros**: ~40 lines removed, simpler API, one cfg location
- **Cons**: Logging functions gain telemetry awareness
- **Effort**: Medium

### Option B: Extract helper function
Create `maybe_otel_layer(settings)` returning `(Option<impl Layer>, Option<TelemetryGuard>)` with the cfg logic centralized.
- **Pros**: Less invasive, keeps logging/telemetry separate
- **Cons**: Still needs NoopLayer for type inference
- **Effort**: Small

## Acceptance Criteria

- [ ] OTel cfg blocks exist in exactly one location
- [ ] `NoopLayer` re-export removed (if Option A)
- [ ] Both `cargo build` and `cargo build --features telemetry` pass
