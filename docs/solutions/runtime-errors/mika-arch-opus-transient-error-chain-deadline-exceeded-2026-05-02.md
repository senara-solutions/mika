---
title: "mika-arch Opus 4.7 transient-error chain exhausts agent deadline"
date: 2026-05-02
category: runtime-errors
module: llm-transport
problem_type: runtime_error
component: assistant
symptoms:
  - "Three consecutive 'transient Claude API error' warnings over 6 minutes"
  - "'agent deadline exceeded — exiting loop gracefully' after 8 minutes wall time"
  - "User-visible fallback: 'I'm sorry, that took too long. Let me try a simpler approach next time.'"
  - "Per-skill LLM override routes mika-arch-groom-ticket to Opus 4.7 despite agent default being Sonnet 4.6"
root_cause: config_error
resolution_type: code_fix
severity: high
tags:
  - mika-arch
  - opus-4-7
  - transient-error
  - retry-chain
  - deadline-exceeded
  - skill-llm-override
  - deadline-aware-retry
---

# mika-arch Opus 4.7 transient-error chain exhausts agent deadline

## Problem

mika-arch's `mika-arch-groom-ticket` skill had a per-skill LLM override routing it to Opus 4.7. During grooming on 2026-05-02, Opus 4.7 returned three consecutive transient errors. Each retry waited for the 120s reqwest timeout to expire before the short exponential backoff (500ms, 1s, 2s) fired. Total: 4 attempts x ~2 minutes = 8 minutes, exceeding the 5-minute agent deadline and producing an uninformative user-visible fallback message.

## Symptoms

- `transient Claude API error` WARN logged three times at 2-minute intervals
- `retrying Claude API call` WARN logged three times with sub-second backoff delays
- `agent deadline exceeded — exiting loop gracefully` after ~480 seconds
- LLM calls telemetry: step 0 latency = 479,619ms (nearly 8 minutes on a single "call" that included all retries)
- Post-deadline Sonnet 4.6 turns produced "No new facts warrant persistence" hallucination (separate issue)

## What Didn't Work

- Disabling only `mika-arch-groom-ticket` and leaving `mika-arch-second-review` active produced a Sonnet 4.6 hallucination on the second pass ("No new facts warrant persistence" — a persistence-meta free-association pattern unrelated to this fix, tracked separately)
- The existing retry loop (`MAX_RETRIES = 3`) had no awareness of the outer agent deadline — it would happily consume all retry attempts even when the deadline had already passed

## Solution

Two orthogonal fixes shipped atomically:

**1. Move groom skills from Opus 4.7 to Sonnet 4.6** (`crates/mika-agent/src/well_known_agents.rs`):

```rust
// Before:
LlmOverrideSpec { skill_name: "mika-arch-groom-ticket",    provider: "anthropic", model: "claude-opus-4-7" },
LlmOverrideSpec { skill_name: "mika-arch-groom-milestone", provider: "anthropic", model: "claude-opus-4-7" },

// After:
LlmOverrideSpec { skill_name: "mika-arch-groom-ticket",    provider: "anthropic", model: "claude-sonnet-4-6" },
LlmOverrideSpec { skill_name: "mika-arch-groom-milestone", provider: "anthropic", model: "claude-sonnet-4-6" },
```

Added DB drift reconciliation to `seed_well_known_skill_overrides()` so existing deployments with stale DB rows auto-correct on next startup (previously the function returned early when overrides already existed, causing source-vs-DB drift).

**2. Add deadline-aware retry abort** (`crates/mika-common/src/llm/mod.rs`, `claude.rs`, `openai.rs`):

```rust
// New trait method with default no-op implementation:
async fn send_message_with_deadline(
    &self,
    request: &LlmRequest,
    deadline: Option<Instant>,
) -> Result<LlmResponse, LlmError> {
    let _ = deadline;
    self.send_message(request).await
}

// In both provider retry loops, before each retry:
if let Some(dl) = deadline {
    let remaining = dl.saturating_duration_since(Instant::now());
    if remaining < Duration::from_secs(TYPICAL_CALL_DURATION_SECS + RETRY_BUFFER_SECS) {
        warn!(attempt, remaining_ms = remaining.as_millis() as u64,
            "aborting retry chain — remaining deadline insufficient for another attempt");
        break;
    }
}
```

Shared constants: `TYPICAL_CALL_DURATION_SECS = 90`, `RETRY_BUFFER_SECS = 30` (abort when remaining < 120s).

## Why This Works

The root cause was two-fold: (1) the Opus 4.7 override was an inconsistent calibration choice — Sonnet 4.6 handled the same prompts cleanly, matching `mika-arch-second-review` which was already on Sonnet; (2) the retry loop had no deadline awareness, so 3 retries x 120s reqwest timeout consumed 480s regardless of the 300s agent deadline. The fix addresses both: eliminates the Opus dependency and adds a transport-level abort so any future transient-error chain fails fast before the deadline expires.

## Prevention

- **Deadline awareness at every retry boundary:** When adding retry loops for external calls, always check the remaining time budget before starting a new attempt. The `send_message_with_deadline` pattern (optional deadline parameter with no-op default) preserves backward compatibility while enabling callers with deadline visibility to opt in.
- **Source-of-truth reconciliation for well-known agent configs:** `seed_well_known_skill_overrides` now reconciles drifted DB rows on every startup, not just on first creation. This prevents the class of bug where source code changes to LLM overrides are silently ignored by existing deployments.
- **Consistent model routing across related skills:** All three mika-arch skills (groom-ticket, groom-milestone, second-review) now use the same model (Sonnet 4.6), eliminating the mixed-model failure surface.

## Related Issues

- [mika#939](https://github.com/senara-solutions/mika/issues/939) — this ticket
- [mika#848](https://github.com/senara-solutions/mika/issues/848) — original deadline enforcement via Instant-based checks at iteration-top
- `docs/solutions/runtime-errors/agent-deadline-graceful-exit-2026-04-27.md` — the #848 compound doc establishing the deadline-check-at-boundaries pattern this fix extends to the retry layer
- `docs/solutions/best-practices/mika-arch-first-dogfood-2026-04-25.md` — first dogfood revealing skills running on wrong model
- `docs/solutions/architecture-patterns/well-known-agent-provisioning-dev-mode.md` — the first-creation-only seeding rule that caused the DB drift
