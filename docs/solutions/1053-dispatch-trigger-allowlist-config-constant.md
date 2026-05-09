---
module: crates/mika-agent/src/well_known_agents.rs
tags: [dispatch, security, config, allowlist]
problem_type: config-surface-selection
category: architecture-patterns
---

# Dispatch trigger allowlist as config constant

## Problem

No allowlist existed for dispatch-triggering label actions. Anyone with write access could set the `ready` label and trigger autonomous work via mika-dev.

## Solution

Added `DISPATCH_TRIGGER_ALLOWLIST: &[&str]` constant in `well_known_agents.rs`, colocated with the `MIKA_DEV` agent specification. Initial allowlist: `samidarko` + `mika-platform-dev`.

## Why Rust constant (not core memory or skill config)

1. **Proximity to consumer.** Rec 3's gate logic (separate ticket) will read this constant from either `agent.rs` (engine-side) or the self-dev prompt (prompt-side).
2. **Rebuild-required is fine.** Allowlist churn is rare. Deploy-at-quiescent-boundary is the operational model.
3. **Not core memory.** Adds surface area (accidental overwrite, compaction survival, provision-time seeding) without reducing operational cost at zero churn.
4. **Not skill config.** Would couple storage to one specific consumer.

## Escalation path

If churn rate rises, promote the constant's value to core memory seeding in `provision_well_known_agents()`. The constant stays as the authoritative default.

## Scope boundary

This ticket is storage only. Rec 3 (gate logic that consumes the allowlist) is a separate ticket.
