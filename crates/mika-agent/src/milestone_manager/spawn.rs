//! Cadence spawn wiring — env-var config assembly + tokio spawn.
//!
//! **LECTURE seule.** This module composes `run_manager_cycle` on an interval
//! and cancellation token. It adds no dispatch authority. Every outbound side
//! effect flows through the existing composers in `cadence.rs` (HTTP POST to
//! the delivery/escalation URL, or an offline sink write when unset).
//!
//! ## Env-gate contract
//!
//! `MIKA_MANAGER_TARGET_MILESTONE` is the single feature-gate:
//! - Unset → `manager_config_from_env()` returns `Ok(None)`; the server should
//!   log an INFO line and skip the spawn.
//! - Set to a valid `<owner/repo>#<number>` → `Ok(Some(cfg))`.
//! - Set to a malformed value → `Err(...)` (loud); the server should log an
//!   ERROR and skip the spawn (do not crash startup).
//!
//! All other `MIKA_MANAGER_*` env vars are optional refinements with
//! three-tier fallback: absent → default; unparseable → default + WARN;
//! valid → use value. The exact defaults are documented in the field
//! comments below and echoed in the root `CLAUDE.md`.
//!
//! ## Sibling pattern
//!
//! Structurally mirrors `kg::resolver_tick::spawn_resolver_tick_task`:
//! `tokio::time::interval` + `tokio::select!` on `interval.tick()` vs
//! `cancel.cancelled()`. Cycle errors log via `tracing::warn!` and continue
//! (fail-open). Graceful shutdown responds to SIGTERM within one poll
//! interval.

use super::cadence::{ManagerConfig, run_manager_cycle};
use super::reader::{GhRunner, ProcessGhRunner};
use super::types::MilestoneRef;
use mika_common::config::Settings;
use mika_common::github_app::GitHubApp;
use std::env;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

/// mika#1968 AC6 — process-scoped guard against double-spawn of the cadence
/// task. `Mutex<bool>` (not `OnceLock`) is deliberate: the extra ~40ns cost is
/// irrelevant for a boot-time call, and a `Mutex` allows the test suite to
/// reset the guard between tests via `reset_spawn_guard_for_test()`. See §6a
/// of the plan for the design rationale.
///
/// This guard collapses single-process double-init (root-cause candidate #2:
/// `run_server()` re-entering, or a bin-side loop firing spawn twice). It does
/// NOT prevent two separate mika-spirit PROCESSES from each spawning once
/// (root-cause candidate #1: OpenRC supervise-daemon race, or a stale
/// process not reaped before the new one started). Discrimination is done via
/// the `manager_cadence_spawn_attempt` log — see `spawn_manager_cycle_task`.
///
/// If `manager_cadence_spawn_duplicate_rejected` never fires post-deploy but
/// double-log persists (two `manager_cadence_spawn_attempt` lines with DIFFERENT
/// `pid` values), root cause is two processes — investigate the supervise-daemon
/// restart discipline / stale-PID reap. That fix is out-of-scope for this
/// module (belongs on an OpenRC init-script ticket per plan §6d).
static MANAGER_SPAWN_GUARD: Mutex<bool> = Mutex::new(false);

#[cfg(test)]
fn reset_spawn_guard_for_test() {
    *MANAGER_SPAWN_GUARD.lock().unwrap() = false;
}

/// Env-var name for the Phase 1 single-target milestone. Setting this
/// enables the cadence loop; unset → cadence disabled.
pub const ENV_TARGET_MILESTONE: &str = "MIKA_MANAGER_TARGET_MILESTONE";
pub const ENV_HEARTBEAT_INTERVAL_SECS: &str = "MIKA_MANAGER_HEARTBEAT_INTERVAL_SECS";
pub const ENV_POLL_INTERVAL_SECS: &str = "MIKA_MANAGER_POLL_INTERVAL_SECS";
pub const ENV_SILENCE_THRESHOLD_DAYS: &str = "MIKA_MANAGER_SILENCE_THRESHOLD_DAYS";
pub const ENV_DELIVERY_URL: &str = "MIKA_MANAGER_DELIVERY_URL";
pub const ENV_DELIVERY_TOKEN: &str = "MIKA_MANAGER_DELIVERY_TOKEN";
pub const ENV_ESCALATION_URL: &str = "MIKA_MANAGER_ESCALATION_URL";
pub const ENV_HEALTH_URL: &str = "MIKA_MANAGER_HEALTH_URL";
pub const ENV_CHECKPOINT_DIR: &str = "MIKA_MANAGER_CHECKPOINT_DIR";
pub const ENV_OFFLINE_SINK_DIR: &str = "MIKA_MANAGER_OFFLINE_SINK_DIR";

/// Default heartbeat interval in seconds (6 hours per brief § verdict 2).
pub const DEFAULT_HEARTBEAT_INTERVAL_SECS: i64 = 21_600;
/// Default silence threshold in days (per brief § verdict 5).
pub const DEFAULT_SILENCE_THRESHOLD_DAYS: u32 = 3;
/// Default poll interval in seconds. Cap the cycle latency at 5 min so
/// state-change events are surfaced quickly; the internal cycle is a no-op
/// when neither `state_changed` nor `heartbeat_fired` is true, so a shorter
/// poll costs one `gh` invocation and a digest comparison — cheap.
pub const DEFAULT_POLL_INTERVAL_SECS: u64 = 300;
/// Fallback root for checkpoint/offline-sink dirs when the env vars and
/// `HOME` are all unset (last-resort — should not be reached in production).
const FALLBACK_STATE_ROOT: &str = "/tmp/mika-manager";

/// Assemble a `ManagerConfig` from the current process env, or return
/// `Ok(None)` when the feature-gate env var is unset.
///
/// Errors are limited to structural parse failures on `MIKA_MANAGER_TARGET_MILESTONE`
/// (missing `#`, non-numeric number, empty repo) — the operator asked for a target
/// but named a broken one. Numeric refinements (heartbeat, poll, silence) fall
/// back to their defaults with a WARN log; they never fail loud.
///
/// **mika#1968 AC5 — Settings-routed GitHub token.** Token acquisition goes
/// through `settings.resolve_github_token(github_app)` (PAT first, App
/// installation-token fallback) rather than a raw env read. This closes the
/// bypass path where `spawn.rs` previously called `read_string_env("MIKA_GITHUB_TOKEN")`
/// directly — a bypass that meant the manager cadence could not benefit from
/// the GitHub App fallback the rest of the engine uses. `None` remains a
/// valid outcome; the boot-time `verify_gh_auth` sanity call (see
/// `spawn_manager_cycle_task`) surfaces the failure loudly at startup so an
/// operator sees the auth gap before cycle-error spam accumulates.
pub async fn manager_config_from_env(
    settings: &Settings,
    github_app: Option<&GitHubApp>,
) -> Result<Option<ManagerConfig>, String> {
    let target_raw = match env::var(ENV_TARGET_MILESTONE) {
        Ok(v) if !v.trim().is_empty() => v,
        _ => return Ok(None),
    };
    let target = MilestoneRef::parse(target_raw.trim())
        .map_err(|e| format!("{ENV_TARGET_MILESTONE}: {e}"))?;

    let heartbeat_secs = read_u64_env(
        ENV_HEARTBEAT_INTERVAL_SECS,
        DEFAULT_HEARTBEAT_INTERVAL_SECS as u64,
    );
    let poll_secs = read_u64_env(ENV_POLL_INTERVAL_SECS, DEFAULT_POLL_INTERVAL_SECS);
    let silence_days = read_u32_env(ENV_SILENCE_THRESHOLD_DAYS, DEFAULT_SILENCE_THRESHOLD_DAYS);

    // Poll interval floors at 1s and never exceeds heartbeat — a poll_interval
    // larger than the heartbeat would delay heartbeat delivery indefinitely.
    let poll_secs_effective = poll_secs.max(1).min(heartbeat_secs.max(1));

    // Optional string env vars — treat empty string as unset per convention.
    let delivery_url = read_string_env(ENV_DELIVERY_URL);
    let delivery_token = read_string_env(ENV_DELIVERY_TOKEN);
    let escalation_url = read_string_env(ENV_ESCALATION_URL);
    let health_url = read_string_env(ENV_HEALTH_URL);

    // mika#1968 AC5 (change 5a) — route through Settings::resolve_github_token
    // (PAT first, App installation-token fallback) instead of raw env read.
    // The `agent_github_token()` accessor is the same MIKA_GITHUB_TOKEN the rest
    // of the crate uses; if the PAT is unset, the App fallback resolves via
    // the injected GitHubApp handle.
    //
    // **A3 P1 note — App token lifetime hazard (deferred per plan §5c).**
    // When this path returns an App installation token (PAT is unset,
    // GitHubApp resolved), the token has a ~1h TTL. `ManagerConfig.github_token`
    // is populated ONCE at spawn time and forwarded verbatim to `gh` on every
    // cycle. After 1h the manager cycles silently 401 until the process
    // restarts, even though `verify_gh_auth` passed at boot. This is the same
    // shape as the founding incident from the operator's log-grep perspective
    // (401s buried in cycle_error spam), scoped to App-path deployments only.
    // The `auth_class=401` field on `manager_cycle_error` (change 5c) is the
    // primary diagnostic surface — sustained hits >1h post-boot on an
    // App-auth deployment signal this class. Follow-up ticket needed to
    // periodically refresh via `resolve_github_token` inside the cycle loop.
    let github_token = settings.resolve_github_token(github_app).await;

    let checkpoint_dir = read_path_env(ENV_CHECKPOINT_DIR)
        .unwrap_or_else(|| default_state_root().join("checkpoints"));
    let offline_sink_dir =
        read_path_env(ENV_OFFLINE_SINK_DIR).unwrap_or_else(|| default_state_root().join("sink"));

    Ok(Some(ManagerConfig {
        target,
        github_token,
        heartbeat_interval: chrono::Duration::seconds(heartbeat_secs as i64),
        poll_interval: chrono::Duration::seconds(poll_secs_effective as i64),
        silence_threshold_days: silence_days,
        delivery_url,
        delivery_token,
        escalation_url,
        health_url,
        checkpoint_dir,
        offline_sink_dir,
    }))
}

/// Spawn the manager cadence loop as a background tokio task.
///
/// The loop polls at `cfg.poll_interval` and invokes `run_manager_cycle` on
/// each tick. The first tick is skipped (mirrors the KG resolver pattern) so
/// the process does not fire a cycle before startup work has settled.
///
/// Cycle failures are logged at WARN and do not stop the loop. Graceful
/// shutdown responds to `cancel.cancelled()` within one poll interval.
///
/// **mika#1968 AC6 — Idempotent spawn.** Returns `Some(handle)` on the first
/// call within a process; returns `None` on any subsequent call, emitting
/// `manager_cadence_spawn_duplicate_rejected` (WARN). The sole caller
/// (`server::run_server`) already treats the spawn as best-effort; `None` means
/// "another spawn already running, skip." The guard collapses single-process
/// double-entry (root-cause candidate #2 from the plan). Two separate
/// mika-spirit processes would each pass their own guard — for that class,
/// the `manager_cadence_spawn_attempt` log carries the PID so operators can
/// discriminate between the two root-cause candidates.
pub fn spawn_manager_cycle_task(
    cfg: ManagerConfig,
    cancel: CancellationToken,
) -> Option<JoinHandle<()>> {
    // mika#1968 AC6 (change 6b) — diagnostic PID log emitted BEFORE the guard
    // check so operators can distinguish "two mika-spirit processes" (two
    // spawn_attempt lines with DIFFERENT pids) from "one process re-entering"
    // (two spawn_attempt lines with the SAME pid). The guard check that
    // follows collapses the second case; only the first requires an operator
    // fix outside this module.
    info!(
        target: "mika::milestone_manager",
        event = "manager_cadence_spawn_attempt",
        pid = std::process::id(),
        milestone = %cfg.target.as_display(),
        "spawn_manager_cycle_task entered"
    );

    // mika#1968 AC6 (change 6a) — process-scoped guard. See MANAGER_SPAWN_GUARD
    // docstring for the failure-mode discrimination this covers.
    {
        let mut guard = MANAGER_SPAWN_GUARD.lock().unwrap();
        if *guard {
            warn!(
                target: "mika::milestone_manager",
                event = "manager_cadence_spawn_duplicate_rejected",
                pid = std::process::id(),
                milestone = %cfg.target.as_display(),
                "spawn_manager_cycle_task called twice within same process — second call rejected"
            );
            return None;
        }
        *guard = true;
    }

    Some(tokio::spawn(async move {
        info!(
            target: "mika::milestone_manager",
            event = "manager_cadence_start",
            milestone = %cfg.target.as_display(),
            heartbeat_secs = cfg.heartbeat_interval.num_seconds(),
            poll_secs = cfg.poll_interval.num_seconds(),
            delivery_url_set = cfg.delivery_url.is_some(),
            escalation_url_set = cfg.escalation_url.is_some(),
            "mika-manager cadence started"
        );

        // mika#1968 AC5 + mika#1974 — boot-time GitHub auth sanity call.
        // Runs before the cycle loop starts so any auth/scope failure is
        // surfaced with a loud, actionable message at startup rather than
        // showing up as cycle_error spam every poll_interval.
        //
        // Probes the target milestone endpoint (mika#1974) rather than
        // `/rate_limit` (the pre-#1974 shape) — success proves both token
        // validity AND scope for the target milestone, catching the
        // wrong-user-PAT class that `/rate_limit` silently accepted.
        //
        // Best-effort: log-and-continue on failure (substrate-reliability
        // class — cadence is best-effort, never panic).
        let runner = ProcessGhRunner::new(cfg.github_token.clone());
        match verify_gh_auth(&runner, &cfg.target).await {
            Ok(()) => {
                info!(
                    target: "mika::milestone_manager",
                    event = "manager_gh_auth_check_ok",
                    milestone = %cfg.target.as_display(),
                    "mika-manager GitHub auth verified (milestone-scoped probe)"
                );
            }
            Err(GhAuthError {
                auth_class,
                stderr_head,
                exit_code,
            }) => {
                // mika#1974 AC2 — per-class operator hint. Named class
                // discrimination via structured `auth_class` field remains
                // the primary greppable signal; hints add the human touch.
                let hint = match auth_class {
                    AuthClass::Unauthorized => {
                        "token missing/invalid/expired — check `tr '\\0' '\\n' < /proc/$(pidof mika-spirit)/environ | grep MIKA_GITHUB_TOKEN`"
                    }
                    AuthClass::Forbidden => {
                        "token authenticated but lacks scope for target repo — check GitHub App installation on the org or PAT org access"
                    }
                    AuthClass::MilestoneNotFound => {
                        "target milestone not found — check `MIKA_MANAGER_TARGET_MILESTONE` value or milestone existence on GitHub"
                    }
                    AuthClass::Network => {
                        "gh cannot reach GitHub — check network/DNS/TLS from daemon host"
                    }
                    AuthClass::Other => "unexpected failure — see stderr_head for details",
                };
                error!(
                    target: "mika::milestone_manager",
                    event = "manager_gh_auth_check_failed",
                    milestone = %cfg.target.as_display(),
                    auth_class = auth_class.as_str(),
                    exit_code,
                    stderr_head = %stderr_head,
                    hint,
                    "mika-manager GitHub auth check failed — cycles will fail until fixed"
                );
            }
        }

        let poll = duration_from_chrono(cfg.poll_interval);
        let mut interval = tokio::time::interval(poll);
        // Skip the first immediate fire so we don't cycle before the server
        // finishes initializing sibling subsystems (KG ticks, checkpoint,
        // wedge watchdog).
        interval.tick().await;

        loop {
            tokio::select! {
                _ = interval.tick() => {}
                _ = cancel.cancelled() => {
                    info!(
                        target: "mika::milestone_manager",
                        event = "manager_cadence_cancelled",
                        milestone = %cfg.target.as_display(),
                        "mika-manager cadence stopped (graceful shutdown)"
                    );
                    return;
                }
            }
            // Double-check before starting a potentially long cycle.
            if cancel.is_cancelled() {
                return;
            }

            match run_manager_cycle(&cfg).await {
                Ok(outcome) => {
                    if outcome.delivered {
                        info!(
                            target: "mika::milestone_manager",
                            event = "manager_cycle_delivered",
                            milestone = %cfg.target.as_display(),
                            severity = ?outcome.severity,
                            state_changed = outcome.state_changed,
                            heartbeat_fired = outcome.heartbeat_fired,
                            escalated = outcome.escalated,
                        );
                    } else {
                        // No-op cycle — expected when state unchanged and
                        // heartbeat hasn't elapsed. Log at TRACE-equivalent
                        // via structured info at low verbosity by omitting;
                        // silence keeps the log cheap.
                    }
                }
                Err(e) => {
                    // mika#1968 AC5 (change 5c) — structured `auth_class`
                    // field on cycle errors lets operators grep specifically
                    // for `manager_cycle_error auth_class=401` to separate
                    // auth failures from transient network failures without
                    // regex-parsing the free-text error body.
                    let auth_class = classify_cycle_error(&format!("{e}"));
                    warn!(
                        target: "mika::milestone_manager",
                        event = "manager_cycle_error",
                        milestone = %cfg.target.as_display(),
                        auth_class = auth_class.as_str(),
                        error = %e,
                        "manager cycle failed — continuing loop"
                    );
                }
            }
        }
    }))
}

// ---- mika#1968 AC5 GitHub auth verification ------------------------------

/// Classification of a `manager_cycle_error` for operator observability.
///
/// The tag is derived post-hoc from the error string produced by `gh` (via
/// the `Reader` path) — it does NOT change error handling, only surfaces a
/// structured discriminator so operators can grep for specific failure
/// classes without regex-parsing the free-text body.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthClass {
    /// HTTP 401 — token missing/invalid/expired (the founding-incident class).
    Unauthorized,
    /// HTTP 403 — token present but insufficient scope, or rate-limit exhaustion.
    Forbidden,
    /// HTTP 404 on the milestone probe — target milestone gone / wrong repo /
    /// wrong milestone number (mika#1974). Distinct from generic `Other`
    /// server failures because the operator remediation is different: check
    /// `MIKA_MANAGER_TARGET_MILESTONE` value / milestone existence, not the
    /// token itself. Only fires from `classify_milestone_probe_error`; the
    /// cycle-body `classify_cycle_error` leaves 404 as `Other` because a 404
    /// mid-cycle (e.g., an issue that got deleted) is not a milestone-scope
    /// signal.
    MilestoneNotFound,
    /// Network-layer failure (connection refused, DNS, TLS, transport reset).
    Network,
    /// Anything else — genuine server-side or parsing failures.
    Other,
}

impl AuthClass {
    fn as_str(self) -> &'static str {
        match self {
            Self::Unauthorized => "401",
            Self::Forbidden => "403",
            Self::MilestoneNotFound => "404_milestone_not_found",
            Self::Network => "network",
            Self::Other => "other",
        }
    }
}

/// Classify a cycle error string emitted by the `Reader` path into one of
/// four operator-relevant buckets. String-based because the underlying
/// `gh` subprocess surface is a string.
///
/// **404 handling:** intentionally not surfaced here — a 404 mid-cycle (an
/// issue that got deleted, an artifact URL that went stale) is orthogonal to
/// milestone-scope auth. Only the boot-time milestone probe uses
/// `classify_milestone_probe_error` which adds 404 discrimination on top of
/// this classifier's four-bucket base.
fn classify_cycle_error(err_text: &str) -> AuthClass {
    let lower = err_text.to_ascii_lowercase();
    if lower.contains("401") || lower.contains("unauthorized") || lower.contains("bad credentials")
    {
        AuthClass::Unauthorized
    } else if lower.contains("403") || lower.contains("forbidden") || lower.contains("rate limit") {
        AuthClass::Forbidden
    } else if lower.contains("connection refused")
        || lower.contains("connection reset")
        || lower.contains("dns")
        || lower.contains("tls")
        || lower.contains("timed out")
        || lower.contains("network is unreachable")
    {
        AuthClass::Network
    } else {
        AuthClass::Other
    }
}

/// Classify a `verify_gh_auth` milestone-probe error string. Adds 404
/// discrimination on top of `classify_cycle_error` (mika#1974 AC2). Kept
/// separate from the cycle classifier so the two contexts stay decoupled —
/// 404 during the boot-time milestone probe means "target milestone gone /
/// wrong repo" (a distinct operator remediation from a mid-cycle 404).
fn classify_milestone_probe_error(err_text: &str) -> AuthClass {
    let lower = err_text.to_ascii_lowercase();
    // Check 404 BEFORE delegating so `MilestoneNotFound` wins over `Other`.
    // The `not found` phrasing catches gh's default 404 stderr shape as well
    // as the raw HTTP status token.
    if lower.contains("404") || lower.contains("not found") {
        AuthClass::MilestoneNotFound
    } else {
        classify_cycle_error(err_text)
    }
}

/// Failure detail from `verify_gh_auth` — mirrors the discriminator shape
/// of `classify_cycle_error` so the boot-time and cycle-time surfaces stay
/// in lockstep.
#[derive(Debug, Clone)]
pub struct GhAuthError {
    pub auth_class: AuthClass,
    pub stderr_head: String,
    pub exit_code: i32,
}

/// mika#1974 (mika#1968 A4 P1 deferred) — call
/// `gh api /repos/{owner}/{repo}/milestones/{number}` via the provided
/// `GhRunner` and return `Ok(())` on success, or an error carrying the
/// auth-class discriminator on failure. Probes the exact target milestone
/// so success proves BOTH token validity AND scope/reachability for the
/// specific milestone the cadence loop reads on every cycle.
///
/// **Superset check (replaces mika#1968 `/rate_limit` probe):** the previous
/// probe called `/rate_limit`, which passes on any authenticated PAT even
/// when the token belongs to a wrong user with no access to the target
/// milestone repo. The verifier's job — surface auth gaps loudly at boot
/// before cycle-error spam accumulates — is exactly the failure mode
/// `/rate_limit` could not catch. Every cycle then 403s on the actual
/// milestone read (the silent-degradation class the boot check exists to
/// prevent). Replacing with the milestone endpoint is a strict fidelity
/// upgrade: success proves the concrete access the cycle needs; failure
/// gives four-way discrimination (401/403/404/network) instead of two-way
/// (auth/network).
///
/// **Replace, not compose:** two-probe composition (rate_limit + milestone)
/// would add one API call for zero fidelity gain — a healthy milestone probe
/// implies a healthy `/rate_limit`. Cost matters less than the noise-floor
/// of a second failure surface interleaving with the first.
///
/// **A2 P1 discipline preserved** — Malformed / empty / schema-drift or
/// number-mismatched response bodies are treated as failure
/// (`AuthClass::Other`) with a `parse_failure:` prefix on `stderr_head`,
/// NOT as silent success. A successful HTTP 200 whose body lacks a
/// `number` field or returns a number different from the requested one
/// is a fidelity gap (proxy interposition, schema drift, silently-followed
/// redirect) the verifier must surface loudly.
pub async fn verify_gh_auth<R: GhRunner>(
    runner: &R,
    target: &MilestoneRef,
) -> Result<(), GhAuthError> {
    let path = format!("/repos/{}/milestones/{}", target.repo, target.number);
    match runner.run(&["api", &path]).await {
        Ok(body) => {
            // Parse-or-fail: a successful HTTP response must contain the
            // milestone with the EXPECTED number. Mismatched number implies
            // the URL was rewritten (proxy, redirect) — a fidelity gap the
            // verifier surfaces rather than silently trusting.
            let number = serde_json::from_str::<serde_json::Value>(&body)
                .ok()
                .and_then(|v| v.get("number").and_then(|n| n.as_u64()));
            match number {
                Some(n) if n == target.number => Ok(()),
                Some(other) => {
                    let snippet: String = body.chars().take(160).collect();
                    Err(GhAuthError {
                        auth_class: AuthClass::Other,
                        stderr_head: format!(
                            "parse_failure: milestone endpoint returned number={other} (expected {}) — snippet={snippet:?}",
                            target.number
                        ),
                        exit_code: -1,
                    })
                }
                None => {
                    let snippet: String = body.chars().take(160).collect();
                    Err(GhAuthError {
                        auth_class: AuthClass::Other,
                        stderr_head: format!(
                            "parse_failure: milestone endpoint response missing 'number' field — snippet={snippet:?}"
                        ),
                        exit_code: -1,
                    })
                }
            }
        }
        Err(e) => {
            let raw = format!("{e}");
            let auth_class = classify_milestone_probe_error(&raw);
            // The `ProcessGhRunner` error format is `gh <args> failed: <stderr>`.
            // Take the first ~200 chars for the log line.
            let stderr_head = raw.chars().take(200).collect();
            // `gh` exit code is not exposed by our runner API today — mark as -1
            // to signal "captured via stderr, not process exit".
            Err(GhAuthError {
                auth_class,
                stderr_head,
                exit_code: -1,
            })
        }
    }
}

// ---- env-var helpers ------------------------------------------------------

/// Read a `u64` env var with three-tier fallback: absent/empty → default;
/// unparseable/zero → default + WARN; valid → use value. Zero is treated as
/// invalid because a zero interval would spin.
fn read_u64_env(name: &str, default: u64) -> u64 {
    match env::var(name) {
        Ok(raw) if !raw.trim().is_empty() => match raw.trim().parse::<u64>() {
            Ok(n) if n > 0 => n,
            Ok(_) => {
                warn!(
                    target: "mika::milestone_manager",
                    event = "manager_env_invalid",
                    var = name,
                    raw = %raw,
                    reason = "zero_or_negative",
                    default,
                    "invalid env value — using default"
                );
                default
            }
            Err(e) => {
                warn!(
                    target: "mika::milestone_manager",
                    event = "manager_env_invalid",
                    var = name,
                    raw = %raw,
                    reason = "parse_error",
                    error = %e,
                    default,
                    "invalid env value — using default"
                );
                default
            }
        },
        _ => default,
    }
}

/// Same three-tier fallback for `u32`. Zero is accepted for `u32` because
/// `silence_threshold_days == 0` is a legitimate "always alert on silence"
/// tuning; the Assessor consumes it verbatim.
fn read_u32_env(name: &str, default: u32) -> u32 {
    match env::var(name) {
        Ok(raw) if !raw.trim().is_empty() => match raw.trim().parse::<u32>() {
            Ok(n) => n,
            Err(e) => {
                warn!(
                    target: "mika::milestone_manager",
                    event = "manager_env_invalid",
                    var = name,
                    raw = %raw,
                    reason = "parse_error",
                    error = %e,
                    default,
                    "invalid env value — using default"
                );
                default
            }
        },
        _ => default,
    }
}

/// Read an optional string env var. Empty/whitespace-only → `None`.
fn read_string_env(name: &str) -> Option<String> {
    match env::var(name) {
        Ok(v) if !v.trim().is_empty() => Some(v),
        _ => None,
    }
}

/// Read an optional path env var. Empty/whitespace-only → `None`.
fn read_path_env(name: &str) -> Option<PathBuf> {
    read_string_env(name).map(PathBuf::from)
}

/// Resolve the fallback root for state directories (checkpoint + offline sink)
/// when the explicit env vars are unset. Prefers `$HOME/.mika/manager`; falls
/// back to a well-known /tmp path (last-resort, should not be hit in
/// production but keeps the spawn from panicking if HOME is unset).
fn default_state_root() -> PathBuf {
    if let Ok(home) = env::var("HOME")
        && !home.trim().is_empty()
    {
        return PathBuf::from(home).join(".mika").join("manager");
    }
    PathBuf::from(FALLBACK_STATE_ROOT)
}

/// Convert a `chrono::Duration` to `std::time::Duration`, clamping negatives
/// to 1s so the tokio interval never receives a zero/negative period.
fn duration_from_chrono(d: chrono::Duration) -> Duration {
    let secs = d.num_seconds();
    if secs <= 0 {
        Duration::from_secs(1)
    } else {
        Duration::from_secs(secs as u64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::milestone_manager::cadence::{
        DeliveryBody, ReportDeliverer, run_manager_cycle_with,
    };
    use crate::milestone_manager::types::{
        CiState, IssueState, MilestoneState, ProgressCounts, RecentActivity, SubIssue,
    };
    use anyhow::Result;
    use chrono::Utc;
    use serial_test::serial;
    use std::sync::{Arc, Mutex};

    /// Unset every `MIKA_MANAGER_*` env var this module reads. Called at the
    /// top of every serial env test to give a clean slate.
    fn clear_manager_env() {
        // SAFETY: tests are serialized via `#[serial_test::serial]`.
        unsafe {
            for name in [
                ENV_TARGET_MILESTONE,
                ENV_HEARTBEAT_INTERVAL_SECS,
                ENV_POLL_INTERVAL_SECS,
                ENV_SILENCE_THRESHOLD_DAYS,
                ENV_DELIVERY_URL,
                ENV_DELIVERY_TOKEN,
                ENV_ESCALATION_URL,
                ENV_HEALTH_URL,
                ENV_CHECKPOINT_DIR,
                ENV_OFFLINE_SINK_DIR,
            ] {
                env::remove_var(name);
            }
        }
    }

    fn set_env(name: &str, value: &str) {
        // SAFETY: tests are serialized via `#[serial_test::serial]`.
        unsafe { env::set_var(name, value) };
    }

    /// mika#1968 AC5 — canonical minimal `Settings` for tests that only need
    /// to exercise the config-assembly path (no GitHub App, no PAT — the
    /// resolver returns `None` cleanly). Delegates to the workspace-canonical
    /// `Settings::test_defaults()` (test-utils feature-gated in mika-common).
    fn test_settings() -> Settings {
        Settings::test_defaults()
    }

    #[tokio::test]
    #[serial]
    async fn env_unset_returns_none() {
        clear_manager_env();
        let result = manager_config_from_env(&test_settings(), None)
            .await
            .expect("no error when target unset");
        assert!(
            result.is_none(),
            "cadence must be disabled when target unset"
        );
    }

    #[tokio::test]
    #[serial]
    async fn env_target_empty_string_returns_none() {
        clear_manager_env();
        set_env(ENV_TARGET_MILESTONE, "   ");
        let result = manager_config_from_env(&test_settings(), None)
            .await
            .expect("whitespace target treated as unset");
        assert!(
            result.is_none(),
            "whitespace-only target must be treated as unset"
        );
    }

    #[tokio::test]
    #[serial]
    async fn env_set_returns_some_with_defaults() {
        clear_manager_env();
        set_env(ENV_TARGET_MILESTONE, "senara-solutions/mika#30");
        let cfg = manager_config_from_env(&test_settings(), None)
            .await
            .expect("no parse error")
            .expect("Some when target set");
        assert_eq!(cfg.target.repo, "senara-solutions/mika");
        assert_eq!(cfg.target.number, 30);
        assert_eq!(
            cfg.heartbeat_interval.num_seconds(),
            DEFAULT_HEARTBEAT_INTERVAL_SECS
        );
        assert_eq!(cfg.silence_threshold_days, DEFAULT_SILENCE_THRESHOLD_DAYS);
        assert_eq!(
            cfg.poll_interval.num_seconds() as u64,
            DEFAULT_POLL_INTERVAL_SECS
        );
        assert!(cfg.delivery_url.is_none());
        assert!(cfg.escalation_url.is_none());
    }

    #[tokio::test]
    #[serial]
    async fn env_set_parses_full_config() {
        clear_manager_env();
        set_env(ENV_TARGET_MILESTONE, "senara-solutions/mika#42");
        set_env(ENV_HEARTBEAT_INTERVAL_SECS, "3600");
        set_env(ENV_POLL_INTERVAL_SECS, "120");
        set_env(ENV_SILENCE_THRESHOLD_DAYS, "7");
        set_env(ENV_DELIVERY_URL, "https://cm.example.com/manager/deliver");
        set_env(ENV_DELIVERY_TOKEN, "tok-123");
        set_env(ENV_ESCALATION_URL, "https://cm.example.com/vincent/direct");
        set_env(ENV_HEALTH_URL, "https://cm.example.com/health/mika-dev");
        set_env(ENV_CHECKPOINT_DIR, "/var/lib/mika-manager/checkpoints");
        set_env(ENV_OFFLINE_SINK_DIR, "/var/lib/mika-manager/sink");

        let cfg = manager_config_from_env(&test_settings(), None)
            .await
            .expect("valid")
            .expect("Some");
        assert_eq!(cfg.target.number, 42);
        assert_eq!(cfg.heartbeat_interval.num_seconds(), 3600);
        assert_eq!(cfg.poll_interval.num_seconds(), 120);
        assert_eq!(cfg.silence_threshold_days, 7);
        assert_eq!(
            cfg.delivery_url.as_deref(),
            Some("https://cm.example.com/manager/deliver")
        );
        assert_eq!(cfg.delivery_token.as_deref(), Some("tok-123"));
        assert_eq!(
            cfg.escalation_url.as_deref(),
            Some("https://cm.example.com/vincent/direct")
        );
        assert_eq!(
            cfg.health_url.as_deref(),
            Some("https://cm.example.com/health/mika-dev")
        );
        assert_eq!(
            cfg.checkpoint_dir.to_string_lossy(),
            "/var/lib/mika-manager/checkpoints"
        );
        assert_eq!(
            cfg.offline_sink_dir.to_string_lossy(),
            "/var/lib/mika-manager/sink"
        );
    }

    #[tokio::test]
    #[serial]
    async fn env_invalid_heartbeat_falls_back_to_default() {
        clear_manager_env();
        set_env(ENV_TARGET_MILESTONE, "senara-solutions/mika#1");
        set_env(ENV_HEARTBEAT_INTERVAL_SECS, "not-a-number");
        let cfg = manager_config_from_env(&test_settings(), None)
            .await
            .expect("target valid")
            .expect("Some");
        assert_eq!(
            cfg.heartbeat_interval.num_seconds(),
            DEFAULT_HEARTBEAT_INTERVAL_SECS
        );
    }

    #[tokio::test]
    #[serial]
    async fn env_invalid_poll_falls_back_to_default() {
        clear_manager_env();
        set_env(ENV_TARGET_MILESTONE, "senara-solutions/mika#1");
        set_env(ENV_POLL_INTERVAL_SECS, "junk");
        let cfg = manager_config_from_env(&test_settings(), None)
            .await
            .expect("target valid")
            .expect("Some");
        assert_eq!(
            cfg.poll_interval.num_seconds() as u64,
            DEFAULT_POLL_INTERVAL_SECS
        );
    }

    #[tokio::test]
    #[serial]
    async fn env_zero_heartbeat_falls_back_to_default() {
        clear_manager_env();
        set_env(ENV_TARGET_MILESTONE, "senara-solutions/mika#1");
        set_env(ENV_HEARTBEAT_INTERVAL_SECS, "0");
        let cfg = manager_config_from_env(&test_settings(), None)
            .await
            .expect("target valid")
            .expect("Some");
        assert_eq!(
            cfg.heartbeat_interval.num_seconds(),
            DEFAULT_HEARTBEAT_INTERVAL_SECS,
            "zero heartbeat must not spin — falls back to default"
        );
    }

    #[tokio::test]
    #[serial]
    async fn env_invalid_silence_falls_back_to_default() {
        clear_manager_env();
        set_env(ENV_TARGET_MILESTONE, "senara-solutions/mika#1");
        set_env(ENV_SILENCE_THRESHOLD_DAYS, "notanumber");
        let cfg = manager_config_from_env(&test_settings(), None)
            .await
            .expect("target valid")
            .expect("Some");
        assert_eq!(cfg.silence_threshold_days, DEFAULT_SILENCE_THRESHOLD_DAYS);
    }

    #[tokio::test]
    #[serial]
    async fn env_zero_silence_days_accepted() {
        clear_manager_env();
        set_env(ENV_TARGET_MILESTONE, "senara-solutions/mika#1");
        set_env(ENV_SILENCE_THRESHOLD_DAYS, "0");
        let cfg = manager_config_from_env(&test_settings(), None)
            .await
            .expect("target valid")
            .expect("Some");
        assert_eq!(
            cfg.silence_threshold_days, 0,
            "0 days is a legitimate 'always alert' tuning"
        );
    }

    #[tokio::test]
    #[serial]
    async fn env_invalid_target_returns_error() {
        clear_manager_env();
        set_env(ENV_TARGET_MILESTONE, "malformed-no-hash");
        let err = manager_config_from_env(&test_settings(), None)
            .await
            .expect_err("malformed target must error");
        assert!(
            err.contains(ENV_TARGET_MILESTONE),
            "error must name the offending env var: {err}"
        );
    }

    #[tokio::test]
    #[serial]
    async fn env_target_non_numeric_returns_error() {
        clear_manager_env();
        set_env(ENV_TARGET_MILESTONE, "senara-solutions/mika#not-a-num");
        let err = manager_config_from_env(&test_settings(), None)
            .await
            .expect_err("non-numeric number must error");
        assert!(err.contains(ENV_TARGET_MILESTONE));
    }

    #[tokio::test]
    #[serial]
    async fn env_poll_larger_than_heartbeat_is_clamped_to_heartbeat() {
        clear_manager_env();
        set_env(ENV_TARGET_MILESTONE, "senara-solutions/mika#1");
        set_env(ENV_HEARTBEAT_INTERVAL_SECS, "60");
        set_env(ENV_POLL_INTERVAL_SECS, "999999");
        let cfg = manager_config_from_env(&test_settings(), None)
            .await
            .expect("valid")
            .expect("Some");
        assert_eq!(
            cfg.poll_interval.num_seconds(),
            cfg.heartbeat_interval.num_seconds(),
            "poll_interval must not exceed heartbeat — a longer poll would delay heartbeat delivery"
        );
    }

    // ---- spawn behavior tests (do NOT touch env — use ManagerConfig directly)

    fn base_state() -> MilestoneState {
        MilestoneState {
            milestone_ref: MilestoneRef {
                repo: "senara-solutions/mika".into(),
                number: 30,
            },
            title: "test".into(),
            description: "".into(),
            state: IssueState::Open,
            created_at: "".into(),
            due_on: None,
            last_activity_at: Some("2026-08-22T00:00:00Z".into()),
            sub_issues: vec![SubIssue {
                number: 100,
                title: "T".into(),
                state: IssueState::Open,
                priority_rank: None,
                plan_present: true,
                branch_present: true,
                pr_number: Some(200),
                pr_state: Some("open".into()),
                ci_state: CiState::Success,
                blockers: vec![],
                updated_at: "2026-08-22T00:00:00Z".into(),
                labels: vec![],
            }],
            progress: ProgressCounts {
                in_flight: 1,
                total: 1,
                ..Default::default()
            },
            recent_activity: vec![RecentActivity {
                at: "2026-08-22T00:00:00Z".into(),
                kind: "sub_issue_closed".into(),
                subject: "#99".into(),
            }],
            executor_healthy: None,
        }
    }

    #[derive(Default, Clone)]
    struct CountingDeliverer {
        count: Arc<Mutex<usize>>,
    }

    #[async_trait::async_trait]
    impl ReportDeliverer for CountingDeliverer {
        async fn deliver(
            &self,
            _url: &str,
            _token: Option<&str>,
            _body: &DeliveryBody,
        ) -> Result<()> {
            *self.count.lock().unwrap() += 1;
            Ok(())
        }
    }

    fn mk_test_cfg(
        dir: &std::path::Path,
        heartbeat: chrono::Duration,
        poll: chrono::Duration,
    ) -> ManagerConfig {
        ManagerConfig {
            target: MilestoneRef {
                repo: "senara-solutions/mika".into(),
                number: 30,
            },
            github_token: None,
            heartbeat_interval: heartbeat,
            poll_interval: poll,
            silence_threshold_days: 3,
            delivery_url: Some("http://normal/deliver".into()),
            delivery_token: Some("t".into()),
            escalation_url: Some("http://vincent/direct".into()),
            health_url: None,
            checkpoint_dir: dir.join("checkpoints"),
            offline_sink_dir: dir.join("sink"),
        }
    }

    /// End-to-end sanity: exercise the same cycle body that
    /// `spawn_manager_cycle_task` calls under the hood, using the direct
    /// `run_manager_cycle_with` API with a controlled clock and recording
    /// deliverer. This proves the composition contract without spinning a
    /// real tokio interval or reaching for tokio's test-util `pause()`
    /// (which would fight the reqwest client the spawn constructs).
    #[tokio::test]
    async fn cycle_delivers_on_first_call_via_composed_path() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = mk_test_cfg(
            tmp.path(),
            chrono::Duration::hours(6),
            chrono::Duration::seconds(30),
        );
        let deliverer = CountingDeliverer::default();
        let outcome = run_manager_cycle_with(&cfg, base_state(), &deliverer, Utc::now())
            .await
            .expect("cycle ok");
        assert!(outcome.delivered);
        assert_eq!(*deliverer.count.lock().unwrap(), 1);
    }

    /// Spawn the real loop with a tiny poll interval and verify cancellation
    /// is honored within a bounded window. We don't assert on `run_manager_cycle`
    /// succeeding here (it will fail because there's no live `gh` for this
    /// synthetic milestone) — we assert only that the cancel path drops the
    /// task promptly. This is AC7.
    ///
    /// **Timeout slack (mika#1974):** the boot-time `verify_gh_auth` probe
    /// now hits the milestone endpoint (`/repos/{repo}/milestones/{n}`)
    /// rather than `/rate_limit` — a marginally slower call over the wire.
    /// The 10s bound accommodates parallel-test-load latency variance while
    /// still catching a truly wedged cancel path (real loop-cancel exits in
    /// tens of milliseconds, not seconds).
    #[tokio::test]
    #[serial]
    async fn spawn_respects_cancel_token() {
        reset_spawn_guard_for_test();
        let tmp = tempfile::tempdir().unwrap();
        // Use a target that will fail at the `gh` boundary — that's fine, the
        // cycle body logs a warn and continues. We're testing the loop's
        // cancel-response, not cycle success.
        let cfg = mk_test_cfg(
            tmp.path(),
            chrono::Duration::seconds(1),
            chrono::Duration::milliseconds(50),
        );
        let cancel = CancellationToken::new();
        let handle = spawn_manager_cycle_task(cfg, cancel.clone())
            .expect("first spawn returns Some(handle)");

        // Let the loop tick at least once.
        tokio::time::sleep(Duration::from_millis(200)).await;
        cancel.cancel();

        // Wait for the task to exit — cancel-response is sub-second on a healthy
        // loop, but the boot-time verify_gh_auth probe (milestone endpoint,
        // real network call) precedes loop-entry and takes 200-500ms
        // depending on load. 10s bound absorbs parallel-test contention.
        let join = tokio::time::timeout(Duration::from_secs(10), handle).await;
        assert!(
            join.is_ok(),
            "spawn task must exit within 10s of cancel — got timeout"
        );
        join.unwrap().expect("join should not panic");
    }

    /// Injection-verified: the env-gate is the load-bearing predicate for
    /// AC1. If `manager_config_from_env` ever silently returns `Some(_)` when
    /// the target is unset, the spawn wiring in `server::mod.rs` will start
    /// a rogue cadence loop against `MilestoneRef { repo: "", number: 0 }` or
    /// similar. This test locks the invariant: unset target ⇒ None, no
    /// side-effect fallback.
    #[tokio::test]
    #[serial]
    async fn env_gate_is_load_bearing_default_off() {
        clear_manager_env();
        for _ in 0..5 {
            // Repeat to guard against any stray singleton/state cache.
            assert!(
                manager_config_from_env(&test_settings(), None)
                    .await
                    .unwrap()
                    .is_none()
            );
        }
    }

    // ---- mika#1968 AC5 config-routing tests ----------------------------

    /// mika#1968 AC5 test — locks the load-bearing routing invariant: when
    /// `Settings.github_token` is set, the resulting `ManagerConfig.github_token`
    /// MUST reflect the Settings value, NOT the raw `MIKA_GITHUB_TOKEN` env
    /// var. A future refactor that reverts to `read_string_env("MIKA_GITHUB_TOKEN")`
    /// — the pre-fix bypass this ticket exists to close — would fail here.
    ///
    /// This is the T2 P1 gap the code-review flagged: `verify_gh_auth_*`
    /// mocks GhRunner and never exercises `manager_config_from_env`'s token
    /// wiring at all. Without this test, a re-introduction of the bypass
    /// class would ship green.
    ///
    /// Sets both env var and Settings.github_token to DIFFERENT values;
    /// asserts the resulting ManagerConfig carries the Settings value.
    #[tokio::test]
    #[serial]
    async fn manager_config_from_env_routes_through_settings_not_raw_env() {
        use mika_common::config::Settings;
        use secrecy::SecretString;

        clear_manager_env();
        set_env(ENV_TARGET_MILESTONE, "senara-solutions/mika#1968");

        // Env value the bypass path (raw read_string_env) would return.
        set_env("MIKA_GITHUB_TOKEN", "env_value_should_NOT_appear");

        // Settings value the Settings-routing path should return.
        let mut settings = Settings::test_defaults();
        settings.github_token = Some(SecretString::from("settings_value_MUST_win".to_string()));

        let cfg = manager_config_from_env(&settings, None)
            .await
            .expect("no parse error")
            .expect("Some when target set");

        assert_eq!(
            cfg.github_token.as_deref(),
            Some("settings_value_MUST_win"),
            "manager_config_from_env MUST route through Settings::resolve_github_token \
             — a regression to raw env read (the pre-fix bypass) would fail this test. \
             Founding incident: mika#1968 (bypass meant App-token fallback was unreachable)."
        );

        // Cleanup env leak.
        unsafe { env::remove_var("MIKA_GITHUB_TOKEN") };
    }

    // ---- mika#1968 AC5 GitHub auth tests -------------------------------

    /// Mock `GhRunner` that always returns a given result — used to exercise
    /// `verify_gh_auth` without spawning `gh` subprocesses.
    struct MockGhRunner {
        result: Result<String>,
    }

    #[async_trait::async_trait]
    impl GhRunner for MockGhRunner {
        async fn run(&self, _args: &[&str]) -> Result<String> {
            match &self.result {
                Ok(body) => Ok(body.clone()),
                Err(e) => Err(anyhow::anyhow!("{}", e)),
            }
        }
    }

    /// Canonical test target — matches the mock success body's `number`.
    fn test_target() -> MilestoneRef {
        MilestoneRef {
            repo: "senara-solutions/mika".into(),
            number: 30,
        }
    }

    /// Valid milestone JSON body with `number = 30` — success arm for
    /// `verify_gh_auth`.
    fn milestone_success_body() -> String {
        r#"{"number":30,"title":"test milestone","state":"open","open_issues":5,"closed_issues":25}"#.to_string()
    }

    /// mika#1974 AC1 companion — `verify_gh_auth` returns `Err` with
    /// `Unauthorized` discriminator when the runner emits a `gh` failure
    /// containing 401 / "Unauthorized". Locks the classifier's grep contract
    /// that the operator log message depends on.
    #[tokio::test]
    async fn verify_gh_auth_401_returns_err_unauthorized() {
        let runner = MockGhRunner {
            result: Err(anyhow::anyhow!(
                "gh api /repos/senara-solutions/mika/milestones/30 failed: HTTP 401: Bad credentials"
            )),
        };
        let err = verify_gh_auth(&runner, &test_target())
            .await
            .expect_err("401 must return Err");
        assert_eq!(
            err.auth_class,
            AuthClass::Unauthorized,
            "401 body must classify as Unauthorized"
        );
        assert!(
            err.stderr_head.contains("401") || err.stderr_head.contains("Bad credentials"),
            "stderr_head must preserve the 401 signal for grep: {}",
            err.stderr_head
        );
    }

    /// mika#1974 AC1 companion — success path returns `Ok(())` when the
    /// milestone endpoint returns a JSON body whose `number` matches the
    /// requested target.
    #[tokio::test]
    async fn verify_gh_auth_success_returns_ok() {
        let runner = MockGhRunner {
            result: Ok(milestone_success_body()),
        };
        verify_gh_auth(&runner, &test_target())
            .await
            .expect("valid milestone body with matching number must return Ok(())");
    }

    /// mika#1974 — malformed / empty / schema-drift / number-mismatched
    /// success bodies MUST be treated as failure. Locks the A2 P1 discipline
    /// carried forward from mika#1968: a HTTP 200 whose body lacks the
    /// expected `number` field (or returns a different number, implying URL
    /// rewrite by a proxy/redirect) is a fidelity gap the verifier surfaces
    /// loudly.
    #[tokio::test]
    async fn verify_gh_auth_malformed_body_returns_err_other() {
        // Case 1: empty JSON object — valid JSON but missing schema.
        let runner = MockGhRunner {
            result: Ok("{}".to_string()),
        };
        let err = verify_gh_auth(&runner, &test_target())
            .await
            .expect_err("empty body must not silently pass");
        assert_eq!(err.auth_class, AuthClass::Other);
        assert!(
            err.stderr_head.contains("parse_failure"),
            "stderr_head must carry parse_failure prefix: {}",
            err.stderr_head
        );

        // Case 2: garbage body (not JSON).
        let runner = MockGhRunner {
            result: Ok("".to_string()),
        };
        let err = verify_gh_auth(&runner, &test_target())
            .await
            .expect_err("empty string body must not silently pass");
        assert_eq!(err.auth_class, AuthClass::Other);

        // Case 3: schema-drift — no `number` field.
        let runner = MockGhRunner {
            result: Ok(r#"{"title":"drift","state":"open"}"#.to_string()),
        };
        let err = verify_gh_auth(&runner, &test_target())
            .await
            .expect_err("schema-drift body must not silently pass");
        assert_eq!(err.auth_class, AuthClass::Other);
        assert!(err.stderr_head.contains("missing 'number'"));

        // Case 4: mismatched number — proxy/redirect signal.
        let runner = MockGhRunner {
            result: Ok(r#"{"number":42,"title":"wrong"}"#.to_string()),
        };
        let err = verify_gh_auth(&runner, &test_target())
            .await
            .expect_err("mismatched number must not silently pass");
        assert_eq!(err.auth_class, AuthClass::Other);
        assert!(
            err.stderr_head.contains("number=42") && err.stderr_head.contains("expected 30"),
            "stderr_head must expose the mismatch: {}",
            err.stderr_head
        );
    }

    /// mika#1968 AC5 test — 403 and network-class errors classify separately
    /// from Unauthorized so `manager_cycle_error auth_class=…` grep isolates
    /// distinct failure classes. Cycle classifier unchanged in mika#1974 —
    /// 404 stays `Other` here (the milestone probe uses its own classifier).
    #[tokio::test]
    async fn classify_cycle_error_discriminates_403_and_network() {
        assert_eq!(
            classify_cycle_error("gh api foo failed: HTTP 403: Rate limit exceeded"),
            AuthClass::Forbidden
        );
        assert_eq!(
            classify_cycle_error(
                "gh api foo failed: Get https://api.github.com/rate_limit: dial tcp: dns lookup failed"
            ),
            AuthClass::Network
        );
        assert_eq!(
            classify_cycle_error("gh api foo failed: HTTP 500: Internal Server Error"),
            AuthClass::Other
        );
        // 404 mid-cycle stays Other (orthogonal to milestone-scope auth).
        assert_eq!(
            classify_cycle_error("gh api foo failed: HTTP 404: Not Found"),
            AuthClass::Other,
            "cycle-body 404s must remain Other — the milestone-probe classifier owns 404-discrimination"
        );
    }

    /// mika#1974 AC2 — `classify_milestone_probe_error` adds 404 discrimination
    /// on top of the four-bucket base. 404 → `MilestoneNotFound`; all other
    /// shapes delegate to `classify_cycle_error` unchanged.
    #[tokio::test]
    async fn classify_milestone_probe_error_discriminates_404() {
        // 404 → MilestoneNotFound (the new class).
        assert_eq!(
            classify_milestone_probe_error(
                "gh api /repos/senara-solutions/mika/milestones/999 failed: HTTP 404: Not Found"
            ),
            AuthClass::MilestoneNotFound
        );
        // Bare "not found" string (belt-and-braces phrasing).
        assert_eq!(
            classify_milestone_probe_error("gh api foo failed: milestone not found"),
            AuthClass::MilestoneNotFound
        );
        // 401 still classifies as Unauthorized (delegated).
        assert_eq!(
            classify_milestone_probe_error("gh api foo failed: HTTP 401: Bad credentials"),
            AuthClass::Unauthorized
        );
        // 403 still classifies as Forbidden (delegated).
        assert_eq!(
            classify_milestone_probe_error("gh api foo failed: HTTP 403: Forbidden"),
            AuthClass::Forbidden
        );
        // Network still classifies as Network (delegated).
        assert_eq!(
            classify_milestone_probe_error("gh api foo failed: dial tcp: dns lookup failed"),
            AuthClass::Network
        );
    }

    /// mika#1974 AC2 — 404 on the milestone endpoint surfaces as
    /// `MilestoneNotFound` so operators can distinguish target-milestone-gone
    /// from token-invalid remediation paths without regex-parsing stderr.
    #[tokio::test]
    async fn verify_gh_auth_404_returns_err_milestone_not_found() {
        let runner = MockGhRunner {
            result: Err(anyhow::anyhow!(
                "gh api /repos/senara-solutions/mika/milestones/30 failed: HTTP 404: Not Found"
            )),
        };
        let err = verify_gh_auth(&runner, &test_target())
            .await
            .expect_err("404 must return Err");
        assert_eq!(
            err.auth_class,
            AuthClass::MilestoneNotFound,
            "404 body must classify as MilestoneNotFound"
        );
        assert_eq!(
            err.auth_class.as_str(),
            "404_milestone_not_found",
            "as_str() must expose the discriminator suffix so grep can distinguish it"
        );
    }

    /// mika#1974 AC2 — 403 on the milestone endpoint surfaces as `Forbidden`
    /// (the class that catches the wrong-org App installation / PAT-org-access
    /// remediation path).
    #[tokio::test]
    async fn verify_gh_auth_403_returns_err_forbidden() {
        let runner = MockGhRunner {
            result: Err(anyhow::anyhow!(
                "gh api /repos/senara-solutions/mika/milestones/30 failed: HTTP 403: Resource not accessible by integration"
            )),
        };
        let err = verify_gh_auth(&runner, &test_target())
            .await
            .expect_err("403 must return Err");
        assert_eq!(err.auth_class, AuthClass::Forbidden);
    }

    /// `GhRunner` mock that inspects the argument list and returns different
    /// responses for `/rate_limit` vs milestone-scoped paths. Used to prove
    /// the AC3 regression scenario: a PAT that succeeds on `/rate_limit` but
    /// lacks scope for the target milestone.
    struct ArgAwareRunner {
        rate_limit_response: Result<String>,
        milestone_response: Result<String>,
    }

    #[async_trait::async_trait]
    impl GhRunner for ArgAwareRunner {
        async fn run(&self, args: &[&str]) -> Result<String> {
            // Path is always the last arg after `api`.
            let path = args.last().copied().unwrap_or("");
            if path == "/rate_limit" {
                match &self.rate_limit_response {
                    Ok(b) => Ok(b.clone()),
                    Err(e) => Err(anyhow::anyhow!("{}", e)),
                }
            } else if path.starts_with("/repos/") && path.contains("/milestones/") {
                match &self.milestone_response {
                    Ok(b) => Ok(b.clone()),
                    Err(e) => Err(anyhow::anyhow!("{}", e)),
                }
            } else {
                Err(anyhow::anyhow!(
                    "ArgAwareRunner: unexpected path {path:?} in args {args:?}"
                ))
            }
        }
    }

    /// mika#1974 AC3 — regression test for the wrong-user-PAT class. Locks
    /// the founding-incident shape: a PAT that would pass the pre-#1974
    /// `/rate_limit` probe (rate-limit endpoint is user-scoped, not
    /// milestone-scoped) but has no access to the target milestone repo
    /// MUST be caught by the new milestone-scoped probe.
    ///
    /// Any regression that reverts `verify_gh_auth` to call `/rate_limit`
    /// will fail this test — the mock returns success on `/rate_limit` and
    /// 403 on the milestone endpoint. A `/rate_limit`-only probe would
    /// return `Ok(_)`; the milestone-scoped probe correctly returns
    /// `Err(Forbidden)`.
    #[tokio::test]
    async fn verify_gh_auth_catches_wrong_user_pat_class() {
        let runner = ArgAwareRunner {
            // Pre-#1974 probe would succeed on this — user-scoped rate limit
            // ignores the target repo entirely.
            rate_limit_response: Ok(
                r#"{"resources":{"core":{"limit":5000,"remaining":4999,"reset":123}}}"#.to_string(),
            ),
            // Milestone-scoped probe correctly surfaces the scope gap.
            milestone_response: Err(anyhow::anyhow!(
                "gh api /repos/senara-solutions/mika/milestones/30 failed: HTTP 403: Resource not accessible by integration"
            )),
        };
        let err = verify_gh_auth(&runner, &test_target()).await.expect_err(
            "wrong-user PAT with valid /rate_limit but no milestone scope MUST return Err — \
                 a regression to /rate_limit-only probe would return Ok here",
        );
        assert_eq!(
            err.auth_class,
            AuthClass::Forbidden,
            "wrong-user PAT scope failure classifies as Forbidden (403)"
        );
    }

    /// `GhRunner` mock that records the args passed to each `run()` call.
    /// Used to prove AC1 structurally — the probe uses the milestone-scoped
    /// endpoint, not `/rate_limit`.
    struct RecordingArgRunner {
        calls: Arc<Mutex<Vec<Vec<String>>>>,
        response: String,
    }

    #[async_trait::async_trait]
    impl GhRunner for RecordingArgRunner {
        async fn run(&self, args: &[&str]) -> Result<String> {
            self.calls
                .lock()
                .unwrap()
                .push(args.iter().map(|s| s.to_string()).collect());
            Ok(self.response.clone())
        }
    }

    /// mika#1974 AC1 — structural assertion that `verify_gh_auth` probes
    /// the milestone-scoped endpoint (not `/rate_limit`). Any regression
    /// that reverts the probe path will fail this test.
    #[tokio::test]
    async fn verify_gh_auth_probes_milestone_scoped_endpoint() {
        let calls: Arc<Mutex<Vec<Vec<String>>>> = Arc::new(Mutex::new(Vec::new()));
        let runner = RecordingArgRunner {
            calls: calls.clone(),
            response: milestone_success_body(),
        };
        verify_gh_auth(&runner, &test_target())
            .await
            .expect("valid success body");

        let recorded = calls.lock().unwrap().clone();
        assert_eq!(recorded.len(), 1, "verifier must issue exactly one probe");
        let args = &recorded[0];
        assert_eq!(args[0], "api", "first arg must be `api`");
        assert_eq!(
            args[1], "/repos/senara-solutions/mika/milestones/30",
            "second arg must be the milestone-scoped path, NOT /rate_limit — \
             this test locks mika#1974 AC1 against regression"
        );
        assert!(
            !args.iter().any(|a| a == "/rate_limit"),
            "verifier must NOT probe /rate_limit (mika#1974 replaces it)"
        );
    }

    // ---- mika#1968 AC6 spawn-once guard tests --------------------------

    /// mika#1968 AC6 test — second call within the same process is rejected
    /// with `None` (defense-in-depth against single-process double-init).
    /// Guard reset via `reset_spawn_guard_for_test()` so this test stays
    /// hermetic; without the reset the guard would leak across tests since
    /// `Mutex<bool>` is process-scoped.
    #[tokio::test]
    #[serial]
    async fn spawn_manager_cycle_task_second_call_rejected() {
        reset_spawn_guard_for_test();
        let tmp = tempfile::tempdir().unwrap();
        let cfg1 = mk_test_cfg(
            tmp.path(),
            chrono::Duration::seconds(1),
            chrono::Duration::milliseconds(50),
        );
        let cfg2 = cfg1.clone();

        let cancel = CancellationToken::new();
        let first = spawn_manager_cycle_task(cfg1, cancel.clone());
        assert!(first.is_some(), "first spawn must return Some(handle)");

        // Second call MUST be rejected — regardless of whether the first
        // task is still running.
        let second = spawn_manager_cycle_task(cfg2, cancel.clone());
        assert!(
            second.is_none(),
            "second spawn within same process must be rejected"
        );

        // Cleanup: cancel and drain the first handle so this test does not
        // leak the background task into sibling tests.
        cancel.cancel();
        let handle = first.unwrap();
        let _ = tokio::time::timeout(Duration::from_secs(2), handle).await;
    }
}
