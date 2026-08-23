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

        // mika#1968 AC5 (change 5b) — boot-time GitHub auth sanity call.
        // Runs before the cycle loop starts so any 401 is surfaced with a
        // loud, actionable message at startup rather than showing up as
        // cycle_error spam every poll_interval. Best-effort: log-and-continue
        // on failure (substrate-reliability class — cadence is best-effort,
        // never panic).
        let runner = ProcessGhRunner::new(cfg.github_token.clone());
        match verify_gh_auth(&runner).await {
            Ok(rate_limit_remaining) => {
                info!(
                    target: "mika::milestone_manager",
                    event = "manager_gh_auth_check_ok",
                    milestone = %cfg.target.as_display(),
                    rate_limit_remaining,
                    "mika-manager GitHub auth verified"
                );
            }
            Err(GhAuthError {
                auth_class,
                stderr_head,
                exit_code,
            }) => {
                error!(
                    target: "mika::milestone_manager",
                    event = "manager_gh_auth_check_failed",
                    milestone = %cfg.target.as_display(),
                    auth_class = auth_class.as_str(),
                    exit_code,
                    stderr_head = %stderr_head,
                    hint = "check `tr '\\0' '\\n' < /proc/$(pidof mika-spirit)/environ | grep MIKA_GITHUB_TOKEN` — token likely missing/invalid in daemon env",
                    "mika-manager GitHub auth check failed — cycles will 401 until fixed"
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
            Self::Network => "network",
            Self::Other => "other",
        }
    }
}

/// Classify a cycle error string emitted by the `Reader` path into one of
/// four operator-relevant buckets. String-based because the underlying
/// `gh` subprocess surface is a string.
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

/// Failure detail from `verify_gh_auth` — mirrors the discriminator shape
/// of `classify_cycle_error` so the boot-time and cycle-time surfaces stay
/// in lockstep.
#[derive(Debug, Clone)]
pub struct GhAuthError {
    pub auth_class: AuthClass,
    pub stderr_head: String,
    pub exit_code: i32,
}

/// mika#1968 AC5 (change 5b) — call `gh api /rate_limit` via the provided
/// `GhRunner` and return `Ok(rate_limit_remaining)` on success, or an
/// error carrying the auth-class discriminator on failure. Exercises the
/// exact same code path (`ProcessGhRunner`) the cycle loop uses so a
/// success here means the loop's `gh` calls will also authenticate.
///
/// The primary_rate object shape follows the GitHub REST API's
/// `/rate_limit` response (`resources.core.remaining`).
///
/// **A2 P1 fix** — Malformed / empty / schema-drift response bodies are
/// treated as failure (`AuthClass::Other`), NOT as `Ok(0)`. A successful
/// HTTP response with unparseable body is a fidelity gap for a verifier
/// whose job is to catch bad auth loudly at boot: `Ok(0)` would log
/// `manager_gh_auth_check_ok rate_limit_remaining=0` which is impossible
/// on healthy authenticated accounts (5000/hr baseline) and would mask a
/// silent degradation. We fail-loud with a distinct `parse_failure:` prefix
/// on `stderr_head` so operators can grep for the schema-drift class
/// without confusing it with a genuine 401/403/network failure.
///
/// **Fidelity boundary (A4 deferred):** `/rate_limit` validates the token
/// authenticates against GitHub but does NOT confirm the token has
/// scope/reachability for the specific target milestone repo. A
/// wrong-user PAT or a valid-but-wrong-org App installation token would
/// pass this check and then 403 on every cycle. Target-repo probe is a
/// follow-up (see plan §5c non-goals + `docs/solutions/best-practices/
/// app-owned-env-vs-init-owned-env-2026-08-23.md`).
pub async fn verify_gh_auth<R: GhRunner>(runner: &R) -> Result<u64, GhAuthError> {
    match runner.run(&["api", "/rate_limit"]).await {
        Ok(body) => {
            // A2 P1 fix — parse-or-fail. A successful HTTP response without a
            // usable `resources.core.remaining` is a fidelity gap the verifier
            // must surface, not paper over with `unwrap_or(0)`.
            let remaining = serde_json::from_str::<serde_json::Value>(&body)
                .ok()
                .and_then(|v| {
                    v.get("resources")
                        .and_then(|r| r.get("core"))
                        .and_then(|c| c.get("remaining"))
                        .and_then(|n| n.as_u64())
                });
            match remaining {
                Some(n) => Ok(n),
                None => {
                    let snippet: String = body.chars().take(160).collect();
                    Err(GhAuthError {
                        auth_class: AuthClass::Other,
                        stderr_head: format!(
                            "parse_failure: body missing resources.core.remaining — snippet={snippet:?}"
                        ),
                        exit_code: -1,
                    })
                }
            }
        }
        Err(e) => {
            let raw = format!("{e}");
            let auth_class = classify_cycle_error(&raw);
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

        // Wait for the task to exit — should be well under 1s given the 50ms poll.
        let join = tokio::time::timeout(Duration::from_secs(2), handle).await;
        assert!(
            join.is_ok(),
            "spawn task must exit within 2s of cancel — got timeout"
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

    /// mika#1968 AC5 test — `verify_gh_auth` returns `Err` with `Unauthorized`
    /// discriminator when the runner emits a `gh` failure containing 401 /
    /// "Unauthorized". This locks the classifier's grep contract that the
    /// operator log message depends on.
    #[tokio::test]
    async fn verify_gh_auth_401_returns_err_unauthorized() {
        let runner = MockGhRunner {
            result: Err(anyhow::anyhow!(
                "gh api /rate_limit failed: HTTP 401: Bad credentials"
            )),
        };
        let err = verify_gh_auth(&runner)
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

    /// mika#1968 AC5 test — success path returns the parsed
    /// `resources.core.remaining` value.
    #[tokio::test]
    async fn verify_gh_auth_success_returns_ok_with_remaining() {
        let runner = MockGhRunner {
            result: Ok(
                r#"{"resources":{"core":{"limit":5000,"remaining":4321,"reset":123}}}"#.to_string(),
            ),
        };
        let remaining = verify_gh_auth(&runner)
            .await
            .expect("valid rate_limit body must return Ok");
        assert_eq!(remaining, 4321);
    }

    /// mika#1968 T4 P2 (A2 P1 companion) — malformed / empty / schema-drift
    /// success bodies MUST be treated as failure, not `Ok(0)`. Locks the
    /// A2 P1 fix: a HTTP 200 whose body lacks `resources.core.remaining`
    /// is a fidelity gap the verifier must surface loudly (else `check_ok
    /// remaining=0` would print on healthy accounts that always have 5000
    /// baseline — an operator would misread it as OK when auth is
    /// silently degraded).
    ///
    /// Also protects against a regression to `unwrap_or(0)` — that shape
    /// would return `Ok(0)` here and fail the assertion.
    #[tokio::test]
    async fn verify_gh_auth_malformed_body_returns_err_other() {
        // Case 1: empty JSON object — valid JSON but missing schema.
        let runner = MockGhRunner {
            result: Ok("{}".to_string()),
        };
        let err = verify_gh_auth(&runner)
            .await
            .expect_err("empty body must not silently pass as Ok(0)");
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
        let err = verify_gh_auth(&runner)
            .await
            .expect_err("empty string body must not silently pass as Ok(0)");
        assert_eq!(err.auth_class, AuthClass::Other);

        // Case 3: schema-drift — resources present but no core.remaining.
        let runner = MockGhRunner {
            result: Ok(r#"{"resources":{"other":{}}}"#.to_string()),
        };
        let err = verify_gh_auth(&runner)
            .await
            .expect_err("schema-drift body must not silently pass as Ok(0)");
        assert_eq!(err.auth_class, AuthClass::Other);
    }

    /// mika#1968 AC5 test — 403 and network-class errors classify separately
    /// from Unauthorized so `manager_cycle_error auth_class=…` grep isolates
    /// distinct failure classes.
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
