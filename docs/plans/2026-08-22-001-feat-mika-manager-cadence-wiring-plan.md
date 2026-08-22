# Plan — feat(agent): cadence wiring mika-manager Phase 1 — tokio spawn on env

**Status:** DRAFT
**Date:** 2026-08-22
**Owner:** mika-orchestrator (Vincent + Claude Code, co-creators)
**Follow-up to:** PR#1932 (mika#1931 — mika-manager Phase 1 baptism)
**Class:** LECTURE-seule agent-milestone enhancement (Phase 1 pur — pas de dispatch)

## Why

PR#1932 shipped the mika-manager Phase 1 composers (Reader → Assessor → Reporter → `run_manager_cycle_with`) as a library surface reachable only via the CLI (`mika milestone {read,assess,report}`). The cadence loop — the daemon-side scheduler that fires the cycle every heartbeat interval **and** on state change — is the last piece needed to make Phase 1 observable in production.

Vincent's directive (sami morning, 2026-08-22 05:08Z, verbatim): « **Cadence wiring mika-manager** : GO pour la follow-up PR courte (tokio spawn + env) — c'est du Phase 1 pur (lecture seule), pas du dispatch. »

The wiring is intentionally scoped tight: env-gate, config assembly, tokio spawn, graceful shutdown. **Zero new authority.** The LECTURE-seule structural gate (`no_dispatch_test.rs`) continues to enforce that the module tree carries no `run_claude_pilot`, no `gh api PATCH/POST/DELETE`, no PR merge.

## What

Three additions to `crates/mika-agent/src/milestone_manager/`:

1. **`spawn.rs` (new)** — Config assembly + spawn function.
   - `manager_config_from_env() -> Result<Option<ManagerConfig>, String>` — reads `MIKA_MANAGER_*` env vars, returns `Ok(None)` when `MIKA_MANAGER_TARGET_MILESTONE` is unset (feature default-off), returns `Ok(Some(cfg))` when the target is set and parses cleanly. Three-tier fallback on numeric env vars (absent → default; unparseable → `warn!` + default; valid → use value) matching the pattern already used elsewhere in the crate.
   - `spawn_manager_cycle_task(cfg: ManagerConfig, cancel: CancellationToken) -> JoinHandle<()>` — spawns a tokio task that runs `run_manager_cycle()` on an `event-driven-with-heartbeat` cadence. Structurally mirrors `kg::resolver_tick::spawn_resolver_tick_task`: `tokio::time::interval` + `tokio::select!` on `interval.tick()` vs `cancel.cancelled()`. Cycle errors log via `tracing::warn!` and continue — cycle failures do not crash the spawn (fail-open).

2. **`server::mod.rs` wiring** — After the existing per-agent spawns (KG tick, wedge watchdog, drain workers) but before the `AppState` build, add:
   - A dedicated `manager_shutdown_token = CancellationToken::new()` sibling to `kg_shutdown_token` and `webhook_queue_shutdown`.
   - A call to `manager_config_from_env()`: on `Ok(None)`, log `info!("mika-manager cadence disabled — MIKA_MANAGER_TARGET_MILESTONE unset")` and skip. On `Ok(Some(cfg))`, log `info!("mika-manager cadence starting — target=…, delivery_url=…, escalation_url=…, heartbeat=…s")` and spawn.
   - Cancellation wired at the same `.with_graceful_shutdown` site that already cancels `kg_shutdown_token` / `webhook_queue_shutdown`.
   - JoinHandle pushed onto `tick_handles` for belt-and-suspenders `.abort()` at shutdown.

3. **Tests** (unit + integration + injection-verified).
   - `spawn::tests::env_unset_returns_none` — `MIKA_MANAGER_TARGET_MILESTONE` unset → `manager_config_from_env()` returns `Ok(None)`. No spawn attempted.
   - `spawn::tests::env_set_returns_some_with_defaults` — target set, all other vars unset → `Ok(Some(cfg))` with default heartbeat (6h) and default silence threshold (3 days).
   - `spawn::tests::env_set_parses_full_config` — all env vars set → each field on `ManagerConfig` matches the env source.
   - `spawn::tests::env_invalid_heartbeat_falls_back_with_warn` — `MIKA_MANAGER_HEARTBEAT_INTERVAL_SECS=notanumber` → default heartbeat used (log capture verifies WARN).
   - `spawn::tests::env_invalid_silence_falls_back_with_warn` — same pattern for `MIKA_MANAGER_SILENCE_THRESHOLD_DAYS`.
   - `spawn::tests::env_invalid_target_returns_error` — `MIKA_MANAGER_TARGET_MILESTONE=malformed` → `Err(...)` (invalid target is loud, not silent-default).
   - `spawn::tests::spawn_cycle_fires_on_short_interval` — spawn with `heartbeat_interval = 100ms` + recording deliverer + injected `MilestoneState` fixture → within 500ms the deliverer records ≥1 call. Requires a testable variant of the spawn that accepts an injected `dyn ReportDeliverer` + an injectable state source (or uses `run_manager_cycle_with` under the hood via a wrapper).
   - `spawn::tests::spawn_respects_cancel_token` — spawn task → `cancel.cancel()` → task returns within 1s (bounded by tick check cadence).
   - **Injection-verified (per `feedback_verify_pipeline_passes_without_the_fix`):** documented in `todos/mika-manager-cadence-wiring-injection-verification.md` — for each of (env-gate, cancel-response, cycle-fire), verify the test fails when the guard is inverted, then restore and verify green.

## Definition of Done

- [ ] `crates/mika-agent/src/milestone_manager/spawn.rs` exists with `manager_config_from_env()` + `spawn_manager_cycle_task()` public API.
- [ ] `crates/mika-agent/src/milestone_manager/mod.rs` re-exports the new public API.
- [ ] `crates/mika-agent/src/server/mod.rs` invokes `manager_config_from_env()` at startup, env-gates the spawn, and wires cancellation at the graceful-shutdown site.
- [ ] All new tests pass under `cargo test -p mika-agent --lib milestone_manager::spawn`.
- [ ] `cargo test -p mika-agent --lib milestone_manager::no_dispatch_test` continues to pass — the LECTURE-seule structural gate is not weakened.
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` clean.
- [ ] `cargo fmt --all --check` clean.
- [ ] `bash scripts/verify-pipeline.sh` passes (docs + source buckets both present).
- [ ] Docs updated: `crates/mika-agent/CLAUDE.md` § Milestone Manager (Phase 1) gains one paragraph describing the spawn wiring + `MIKA_MANAGER_TARGET_MILESTONE` env-gate.

## Acceptance criteria

- [ ] **AC1.** `MIKA_MANAGER_TARGET_MILESTONE` unset → no `spawn_manager_cycle_task` invocation. Startup log carries `mika-manager cadence disabled` (INFO). No `tick_handles` entry added for manager cadence.
- [ ] **AC2.** `MIKA_MANAGER_TARGET_MILESTONE=senara-solutions/mika#30` set → `manager_config_from_env()` returns `Ok(Some(cfg))` with `cfg.target` matching the parsed value. Startup log carries `mika-manager cadence starting` (INFO) with the resolved delivery/escalation URLs and heartbeat interval.
- [ ] **AC3.** All numeric env vars (`MIKA_MANAGER_HEARTBEAT_INTERVAL_SECS`, `MIKA_MANAGER_SILENCE_THRESHOLD_DAYS`) implement three-tier fallback: absent → default (21600s / 3 days); unparseable → default + WARN log; valid → use value.
- [ ] **AC4.** Malformed `MIKA_MANAGER_TARGET_MILESTONE` (e.g., missing `#`, non-numeric number, empty repo) → `manager_config_from_env()` returns `Err(...)`. Server-side treatment: log `error!("mika-manager cadence config invalid: {e}")` and skip spawn (do not crash startup).
- [ ] **AC5.** Cycle loop fires on the heartbeat interval. Verified by test using a short interval (`100ms`) + recording deliverer.
- [ ] **AC6.** Cycle errors do NOT crash the spawn — `run_manager_cycle` returning `Err(_)` triggers a `warn!` log and the next tick still fires.
- [ ] **AC7.** SIGTERM (via `cancel.cancel()`) stops the cycle loop within one tick interval. The graceful-shutdown path in `server::mod.rs::run_server` cancels the manager token at the same `.with_graceful_shutdown` site that cancels sibling tokens.
- [ ] **AC8.** LECTURE-seule invariant preserved: `no_dispatch_test.rs` passes without changes to `FORBIDDEN_TOKENS`. The spawn wiring composes existing surfaces (`run_manager_cycle`, `HttpReportDeliverer`) and adds no `run_claude_pilot`, `pr_merge_with_gate`, or `gh api PATCH/POST/DELETE` callsite.
- [ ] **AC9.** Injection-verified: for each of `env_unset_returns_none`, `spawn_respects_cancel_token`, `spawn_cycle_fires_on_short_interval`, the accompanying `todos/mika-manager-cadence-wiring-injection-verification.md` documents a sed-inject bug (e.g., remove the env-gate; invert the `if cfg.is_none()` branch; remove the tick call) that makes the test fail, followed by a restore + re-green.
- [ ] **AC10.** All existing 3711+ workspace tests remain green.

## Verification contract

- **Config path (highest confidence):** env vars → `ManagerConfig` fields verified by unit tests reading env via `std::env::set_var` under `#[serial_test::serial]` (avoids parallel test env races).
- **Spawn behavior:** the tokio spawn is verified by (a) a functional test with `heartbeat_interval = 100ms` + recording deliverer proving the cycle fires, (b) a cancellation test proving the loop terminates on cancel, (c) an error-tolerance test proving cycle errors do not stop the loop.
- **Wiring path (server integration):** verified by manual read of the diff in `server::mod.rs::run_server`. The env-gate + spawn call must sit alongside the sibling KG/webhook-queue spawns (same block, same shutdown wiring). No new integration test in this PR — the server integration is validated by the existing eval suite plus the deliberate structural constraint that we mirror the KG resolver pattern verbatim.
- **LECTURE-seule invariant:** `no_dispatch_test.rs` continues to pass. If the test starts failing, the fix is to update the FORBIDDEN_TOKENS list AND the module docstring atomically (per the existing convention).

## Implementation guidance

- **File layout.** New file `crates/mika-agent/src/milestone_manager/spawn.rs`. Module publishes two public symbols: `manager_config_from_env` and `spawn_manager_cycle_task`. Re-exported from `mod.rs`.
- **Env-gate discipline.** `MIKA_MANAGER_TARGET_MILESTONE` is the single feature-gate. All other env vars are optional refinements. When the target is set but any other var is invalid, log + default (never crash). When the target itself is invalid, error (loud) — do not proceed with a bogus target.
- **Cycle-error tolerance.** The cycle loop must survive individual cycle failures. Pattern:
  ```rust
  loop {
      tokio::select! {
          _ = interval.tick() => {}
          _ = cancel.cancelled() => return,
      }
      if cancel.is_cancelled() { return; }
      match run_manager_cycle(&cfg).await {
          Ok(outcome) => tracing::info!(target: "mika::milestone_manager", event = "manager_cycle_ok", …),
          Err(e) => tracing::warn!(target: "mika::milestone_manager", event = "manager_cycle_err", error = %e, …),
      }
  }
  ```
- **Sibling pattern.** The spawn matches `kg::resolver_tick::spawn_resolver_tick_task` structurally: same `interval + select! + cancel` shape, same fail-open discipline, same `info!` on start + `info!` on cancel + `warn!` on per-iteration error.
- **Shutdown wiring.** In `server::mod.rs::run_server`:
  - Add `let manager_shutdown_token = CancellationToken::new();` alongside `kg_shutdown_token` and `webhook_queue_shutdown`.
  - After the per-agent loop but before `AppState` build, call `manager_config_from_env()` once. On `Ok(Some(cfg))`, spawn with `manager_shutdown_token.child_token()`; push the handle to `tick_handles`.
  - In `.with_graceful_shutdown`, add `manager_shutdown_token.cancel();` alongside the sibling `.cancel()` calls.
- **CLAUDE.md update.** In `crates/mika-agent/CLAUDE.md` § Milestone Manager (Phase 1), add one paragraph: "**Cadence spawn.** When `MIKA_MANAGER_TARGET_MILESTONE` is set at process startup, `spawn_manager_cycle_task` runs `run_manager_cycle` on a `MIKA_MANAGER_HEARTBEAT_INTERVAL_SECS`-cadenced tokio task (default 6h). Cycle errors log via `tracing::warn!` and do not crash the spawn. Graceful shutdown responds to SIGTERM within one tick interval via a shared cancellation token wired at `.with_graceful_shutdown`."

## Non-goals

- **Not** wiring Phase 2 dispatch (still gated behind the three portes — forge-gate loop-résistance, contention exec, INTERNAL_TOKEN alignment).
- **Not** adding multi-milestone support (Phase 1.5 per the founding brief).
- **Not** wiring the executor liveness check to any decision — `probe_executor_health` remains an optional field on `MilestoneState.executor_healthy` that Reporters may surface but Assessors do not gate on.
- **Not** changing the LECTURE-seule structural gate (`no_dispatch_test.rs` FORBIDDEN_TOKENS unchanged).

## Test coverage

| Test | File | Purpose |
|---|---|---|
| `env_unset_returns_none` | `spawn.rs` | AC1 — feature default-off invariant |
| `env_set_returns_some_with_defaults` | `spawn.rs` | AC2, AC3 — target set + defaults populate |
| `env_set_parses_full_config` | `spawn.rs` | AC2 — every env var round-trips |
| `env_invalid_heartbeat_falls_back_with_warn` | `spawn.rs` | AC3 — three-tier fallback (numeric) |
| `env_invalid_silence_falls_back_with_warn` | `spawn.rs` | AC3 — three-tier fallback (numeric) |
| `env_invalid_target_returns_error` | `spawn.rs` | AC4 — malformed target is loud |
| `spawn_cycle_fires_on_short_interval` | `spawn.rs` | AC5 — cycle actually fires |
| `spawn_survives_cycle_error` | `spawn.rs` | AC6 — cycle errors do not crash spawn |
| `spawn_respects_cancel_token` | `spawn.rs` | AC7 — graceful shutdown works |
| Existing `no_dispatch_scaffolding_in_milestone_manager` | `no_dispatch_test.rs` | AC8 — LECTURE-seule preserved |

## Rollout

Ship as short-scope follow-up PR to #1932 mika-manager Phase 1 baptism. Class = LECTURE-seule agent-milestone enhancement. sami merges au fil de l'eau. On production deploy, cadence remains disabled until an operator sets `MIKA_MANAGER_TARGET_MILESTONE` in the mika-spirit env (e.g., `senara-solutions/mika#1799` Luminescent Core reconciliation, per the parent PR's recommendation).
