use anyhow::{Context, Result, anyhow};
use chrono::Timelike;
use mika_common::config::Settings;
use mika_common::embedding::EmbeddingClient;
use mika_common::llm::LlmProvider;
use regex::Regex;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::LazyLock;
use std::sync::atomic::AtomicBool;
use tracing::{debug, info, warn};

/// Typed dispatch errors so callers can match on specific failure modes
/// (e.g. "agent busy") without fragile string matching.
#[derive(Debug, thiserror::Error)]
pub enum DispatchError {
    #[error("agent busy, defer task {0}")]
    AgentBusy(String),
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

use crate::agent::{SilentAgentParams, SilentTrigger, run_silent_agent};
use crate::async_db::AsyncDatabase;
use crate::db::Task;
use crate::messaging::MessageSender;
use crate::skills::SkillRegistry;
use crate::skills::executor::{MAX_STUCK_REARMS, RearmOutcome};
use crate::skills::manifest::ToolHandler;
use crate::tools::ToolRegistry;

use super::types::action_type;

/// Les deux scans périodiques qui résolvent leur token GitHub via
/// [`resolve_periodic_scan_token`] (mika#2205).
///
/// Chaque variante ne porte qu'une chose : le nom d'événement du WARN émis
/// quand aucun token n'est résolu. Les deux scans partagent tout le reste.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PeriodicScan {
    AutoPull,
    WipRescue,
}

impl PeriodicScan {
    /// Nom d'événement greppable dans `$MIKA_SPIRIT_LOG_FILE`.
    fn no_token_event(self) -> &'static str {
        match self {
            Self::AutoPull => "auto_pull_no_token",
            Self::WipRescue => "wip_rescue_no_token",
        }
    }

    /// Ce que l'opérateur perd tant que le token manque.
    fn idle_consequence(self) -> &'static str {
        match self {
            Self::AutoPull => "aucune sélection de ticket groomé ne s'exécute",
            Self::WipRescue => "aucun brouillon wip-rescue n'est repris",
        }
    }
}

/// Résout le token GitHub des deux scans périodiques du dispatcher (mika#2205).
///
/// PAT d'abord, puis repli sur un token d'installation GitHub App — c'est-à-dire
/// exactement [`Settings::resolve_github_token`], le convertisseur canonique que
/// mika#2013 a déjà posé sur le cycle mika-manager
/// (`milestone_manager/spawn.rs`). Avant ce correctif les deux scans lisaient
/// `self.github_token` (le PAT seul, via `Settings::agent_github_token`) : le
/// 2026-09-05 le PAT a disparu de l'environnement du spirit à ~16:20 et les deux
/// scans sont morts à la même seconde, alors que le chemin App était sain
/// (`manager_token_refreshed` jusqu'à 23:17Z, zéro `gh_app_token_exchange_failed`).
///
/// # Identité (AC4, ADR-008)
///
/// ADR-008 exige l'identité PAT machine **là où GitHub lit l'auteur ou le
/// reviewer** de l'action — revue et merge de PR, où `mika-qa` approuvant une PR
/// `mika-dev` sous l'identité App partagée est refusé par GitHub même
/// (`Review Can not approve your own pull request`). Aucune opération de ces
/// deux scans n'est de cette forme :
///
/// - `auto_pull` — bascule du label `ready` (`gh issue edit`), lectures `gh`.
/// - `wip_rescue` — rebase, push sur une branche de brouillon, `gh pr ready`,
///   commentaire de PR.
///
/// L'identité bot de l'App est donc acceptable en repli ici. Les chemins qui
/// **exigent** l'identité machine (revue/merge de PR) ne passent pas par cette
/// fonction et ne sont pas touchés.
///
/// # Fail-safe (AC3)
///
/// Ni PAT ni App résolus → `None`, le scan ne fait rien ce tick. C'est le
/// comportement d'avant ; ce qui change est qu'il le **dit** — WARN au lieu de
/// DEBUG, parce qu'un scan silencieusement inactif se lit comme un scan qui n'a
/// rien trouvé à faire.
async fn resolve_periodic_scan_token(
    settings: &Settings,
    github_app: Option<&mika_common::github_app::GitHubApp>,
    task_id: &str,
    scan: PeriodicScan,
) -> Option<String> {
    let resolved = settings.resolve_github_token(github_app).await;
    if resolved.is_none() {
        warn!(
            task_id = %task_id,
            event = scan.no_token_event(),
            "scan inactif : aucun github_token résolu (PAT absent ET App indisponible) ; {}",
            scan.idle_consequence()
        );
    }
    resolved
}

/// `metadata.$.delivery_attempts` — consecutive failed delivery attempts on a
/// callback (mika#2179). Reset by nothing: a delivery that succeeds ends the
/// row's life as an undelivered callback, so there is no state to clear.
const DELIVERY_ATTEMPTS_KEY: &str = "delivery_attempts";
/// `metadata.$.delivery_first_failed_at` — when the run of failures began.
const DELIVERY_FIRST_FAILED_AT_KEY: &str = "delivery_first_failed_at";
/// `metadata.$.delivery_last_error_class` — class of the most recent failure.
const DELIVERY_LAST_ERROR_CLASS_KEY: &str = "delivery_last_error_class";
/// `metadata.$.delivery_quarantined_at` — when the row crossed the attempt
/// threshold. Its presence is the visible half of AC3's "mise à l'écart
/// visible"; the row's `status` deliberately does not move.
const DELIVERY_QUARANTINED_AT_KEY: &str = "delivery_quarantined_at";

/// Ceiling on the exponent of the callback-delivery backoff (mika#2179).
///
/// `1u64 << 32` is ~4.3e9; multiplied by any sane base that already saturates
/// far past any configurable `max`, so the clamp costs nothing in practice and
/// keeps the shift provably in range. 63 would also be in range for the shift
/// itself, but `base << 62` silently evaluates to zero for a small base — the
/// one arithmetic result that would quietly reinstate the unbounded retry.
const BACKOFF_MAX_SHIFT: u32 = 32;

/// Cap on the error text carried in a `callback_delivery_failed` audit event.
/// The class is the queryable field; this is context for a human reading one
/// row, not a payload to grep.
const DELIVERY_ERROR_REASONING_MAX_CHARS: usize = 300;

/// Backoff for a failed callback delivery (mika#2179, AC3).
///
/// `base * 2^(attempts-1)` capped at `max` while the row is still inside its
/// attempt budget, and **`max` outright from the quarantine crossing onward**.
///
/// That last clause is not cosmetic. With the shipped defaults the crossing is
/// at `attempts = 3`, where the doubling has only reached 240 s; letting it
/// keep doubling would mean four more `resume_agent` turns — each able to hold
/// the agent lock for `AGENT_TOTAL_TIMEOUT_SECS` (300 s) — in the hour after we
/// announced the row was quarantined. The operator doc and the
/// `callback_delivery_quarantined` log line both say a quarantined callback
/// retries at the ceiling; this is what makes that sentence true rather than
/// approximately true. Announcing a bound and then not applying it for another
/// four attempts is the shape of instrument that gets distrusted once and
/// ignored thereafter.
///
/// A pure function so the far tail is testable. `attempts` is unbounded by
/// construction — a row that keeps failing keeps counting — and the naive
/// `base << (attempts - 1)` is wrong there in a way no ordinary run surfaces:
/// `60u64 << 62` drops every set bit and evaluates to **0**, writing a
/// `next_fire_at` in the past and restoring the once-a-minute retry loop this
/// whole function exists to end. `checked_shl` does not catch it either — it
/// refuses only shifts >= 64 and wraps for everything below.
fn delivery_backoff_secs(base: u64, max: u64, attempts: u32, quarantine_at: u32) -> u64 {
    if attempts >= quarantine_at {
        return max;
    }
    let shift = attempts.saturating_sub(1).min(BACKOFF_MAX_SHIFT);
    base.saturating_mul(1u64 << shift).min(max)
}

/// Classify a failed callback delivery by the `LlmError` variant underneath it
/// (mika#2179, AC1).
///
/// **Reads the variant, not the message.** `anyhow::Error::downcast_ref` walks
/// the whole cause chain, so an intermediate `.context()` does not break this;
/// a substring match on the rendered message would break the day a provider
/// rewords its errors. The one place a string is consulted is *inside* the
/// `Transport` variant, to separate a timeout from a connection refusal —
/// `LlmError` does not model that distinction, and the timeout is the shape the
/// founding incident was made of (19 of them in four hours).
///
/// Returns [`Cow::Borrowed`] for every fixed class and [`Cow::Owned`] only for
/// `http_<status>`. The plan wrote this signature as `-> &'static str`, which
/// cannot carry a status code; the classification set it specified — including
/// `http_<status>` — is what is implemented here. Keeping `429` distinguishable
/// from `500` is the difference between "we are being rate-limited" and "the
/// provider is down", and triage needs both.
fn classify_delivery_error(err: &anyhow::Error) -> std::borrow::Cow<'static, str> {
    use mika_common::llm::error::LlmError;
    use std::borrow::Cow;

    let Some(llm_err) = err.downcast_ref::<LlmError>() else {
        return Cow::Borrowed("other");
    };

    match llm_err {
        LlmError::Transport(msg) if msg.to_lowercase().contains("timed out") => {
            Cow::Borrowed("transport_timeout")
        }
        LlmError::Transport(_) => Cow::Borrowed("transport"),
        LlmError::HttpError { status, .. } => Cow::Owned(format!("http_{status}")),
        LlmError::ParseError(_) => Cow::Borrowed("parse"),
        LlmError::ProviderError(_) => Cow::Borrowed("provider"),
        LlmError::UnsupportedFeature(_) => Cow::Borrowed("unsupported"),
    }
}

/// Parent statuses from which no dispatch can ever be produced (mika#2169).
///
/// `run_claude_pilot` refuses a non-active task and `failed` is terminal, so a
/// re-armament aimed at any of these is structurally impossible — not merely
/// unlikely. `absent` is the synthetic value for a parent row that has
/// disappeared, which means the same thing more strongly.
///
/// `completed` and `delivered` belong here for a different reason than the
/// three failure states: work that finished does not need a second dispatch,
/// and re-arming into it would create one.
const TERMINAL_PARENT_STATUSES: &[&str] = &[
    "failed",
    "cancelled",
    "expired",
    "completed",
    "delivered",
    "absent",
];

/// Executes a task's action by matching on `action_type`.
///
/// `send_message` and `inject_context` are fully implemented.
/// `run_skill` is implemented for "heartbeat" and "reflection" triggers.
pub struct TaskDispatcher {
    pub db: AsyncDatabase,
    /// Agent tier, threaded from `AgentState.tier` at construction (mika#1962).
    /// Silent turns read this instead of `AgentTier::from_env()` so a
    /// mid-runtime env change cannot flip the tier of a running dispatcher.
    pub tier: mika_common::home::AgentTier,
    pub llm: Arc<dyn LlmProvider>,
    pub tools: Arc<ToolRegistry>,
    pub skills: Arc<SkillRegistry>,
    pub message_sender: Option<Arc<dyn MessageSender>>,
    pub home_dir: PathBuf,
    pub embedding_client: Option<EmbeddingClient>,
    pub brave_api_key: Option<String>,
    pub github_token: Option<String>,
    /// Gateway base URL for builtins that call substrate endpoints on
    /// the gateway (mika#1969 — `fetch_url` delegates to
    /// `POST /internal/fetch`). Populated from `AppState.gateway_url`
    /// at construction; `None` in tests.
    pub gateway_url: Option<String>,
    /// Shared bearer token for internal substrate endpoints
    /// (mika#1969). Populated from `AppState.internal_token`.
    pub internal_token: Option<String>,
    /// GitHub App authentication manager (optional).
    pub github_app: Option<Arc<mika_common::github_app::GitHubApp>>,
    pub skills_dirty: Arc<AtomicBool>,
    /// Per-agent lock used when running a silent agent turn.
    /// When `Some`, `dispatch_run_skill` uses `try_lock` and defers if busy.
    pub agent_lock: Option<Arc<tokio::sync::Mutex<()>>>,
    /// When true, the engine skips `dispatch_undelivered_callbacks()`.
    /// CLI/TUI mode handles callbacks via `poll_callback_tasks()` instead,
    /// preventing a race where the engine steals callbacks from the TUI.
    pub cli_mode: bool,
    /// Per-agent settings for constructing per-skill LLM overrides in silent mode.
    /// Passed as `settings: Some(&self.settings)` to all `SilentAgentParams` constructions.
    /// See #323.
    pub settings: Settings,
    /// Session-scoped PR review dedup map (#821). Shared with `AppState`.
    /// Entries evicted at each `end_session()` callsite.
    pub pr_reviews_posted: Option<Arc<dashmap::DashMap<String, std::collections::HashSet<String>>>>,
}

impl TaskDispatcher {
    /// Write `execution_trace_id` back to a task after a silent agent run completes.
    /// Best-effort: logs a warning on failure but does not propagate the error.
    async fn write_execution_trace(&self, task_id: &str, trace_id: &str) {
        if let Err(e) = self
            .db
            .update_task_execution_trace_id(task_id, trace_id)
            .await
        {
            warn!(task_id = %task_id, error = %e, "failed to write execution_trace_id");
        }
    }

    /// Check if all sibling tasks of the given task are done, and if so,
    /// dispatch the parent task.
    ///
    /// This is a convenience wrapper around `try_complete_parent_on_sibling_done`
    /// that handles logging and error cases. Call sites only need a single line.
    pub async fn check_and_dispatch_parent(&self, task_id: &str) {
        match self.db.try_complete_parent_on_sibling_done(task_id).await {
            Ok(Some(parent_id)) => {
                info!(
                    task_id = task_id,
                    parent_id = %parent_id,
                    "all siblings done, dispatching parent task"
                );
                if let Err(e) = self.dispatch(&parent_id).await {
                    warn!(parent_id = %parent_id, error = %e, "failed to dispatch parent task");
                }
            }
            Ok(None) => {} // not all siblings done, or no parent
            Err(e) => {
                warn!(task_id = task_id, error = %e, "failed sibling completion check");
            }
        }
    }

    /// Dispatch a task by its ID: load from DB, match on `action_type`, execute.
    pub async fn dispatch(&self, task_id: &str) -> Result<(), DispatchError> {
        let task = self
            .db
            .get_task(task_id)
            .await?
            .ok_or_else(|| anyhow!("task not found: {}", task_id))?;

        let config: serde_json::Value =
            serde_json::from_str(&task.action_config).unwrap_or(serde_json::Value::Null);

        match task.action_type.as_str() {
            action_type::SEND_MESSAGE => Ok(self.dispatch_send_message(&task, &config).await?),
            action_type::INJECT_CONTEXT => Ok(self.dispatch_inject_context(&task, &config).await?),
            action_type::RUN_SKILL => self.dispatch_run_skill(&task, &config).await,
            action_type::RESUME_AGENT => self.dispatch_resume_agent(&task).await,
            action_type::INVOKE_ORCHESTRATOR => {
                Ok(self.dispatch_invoke_orchestrator(&task, &config).await?)
            }
            other => Err(anyhow!("unknown action_type: {}", other).into()),
        }
    }

    /// Send a message to the user via the configured `MessageSender`.
    ///
    /// Expects `action_config`: `{"text": "<message>"}`
    async fn dispatch_send_message(&self, task: &Task, config: &serde_json::Value) -> Result<()> {
        let text = config["text"].as_str().ok_or_else(|| {
            anyhow!(
                "send_message task {} missing 'text' in action_config",
                task.id
            )
        })?;

        const MAX_MESSAGE_LEN: usize = 50_000;
        if text.len() > MAX_MESSAGE_LEN {
            return Err(anyhow!(
                "send_message task {} text exceeds {} chars (got {})",
                task.id,
                MAX_MESSAGE_LEN,
                text.len()
            ));
        }

        if let Some(sender) = &self.message_sender {
            match sender.send(text).await? {
                crate::messaging::SendOutcome::Delivered => {}
                // Task-engine sends are fire-and-forget; delivery failure is logged
                // but does not fail the dispatch (unlike the send_message tool path
                // which surfaces Failed as ToolOutput::error for LLM awareness).
                crate::messaging::SendOutcome::Failed { reason } => {
                    warn!(task_id = %task.id, reason = %reason, "send_message task: delivery failed");
                }
                crate::messaging::SendOutcome::NoChannel => {
                    warn!(task_id = %task.id, "send_message task: no reply channel (chat_id=0)");
                }
            }
        } else {
            debug!(task_id = %task.id, "send_message: no sender configured, dropping message");
        }
        Ok(())
    }

    /// Inject-context tasks have a two-phase lifecycle.
    ///
    /// Phase 1 (this function): the context payload is already stored in `task.action_config`
    /// at creation time. Nothing to do here — we just return `Ok(())`.
    /// The `fire_task` caller will NOT mark this task completed since `inject_context` is
    /// special-cased; the task stays `in_progress`.
    ///
    /// Phase 2 (agent loop, at prompt-build time): `agent.rs` queries for `in_progress`
    /// `inject_context` tasks, injects them into the system prompt, then marks them completed.
    async fn dispatch_inject_context(
        &self,
        _task: &Task,
        _config: &serde_json::Value,
    ) -> Result<()> {
        Ok(())
    }

    /// Run a background silent agent for proactive tasks.
    ///
    /// Supports two modes via `action_config`:
    /// - Built-in triggers: `{"trigger": "heartbeat" | "reflection"}` — pre-filtered
    /// - Arbitrary skills: `{"skill_name": "...", "args": {...}}` — runs the named skill
    async fn dispatch_run_skill(
        &self,
        task: &Task,
        config: &serde_json::Value,
    ) -> Result<(), DispatchError> {
        // Check for arbitrary skill_name first
        if let Some(skill_name) = config["skill_name"].as_str() {
            return self.dispatch_skill_by_name(task, skill_name, config).await;
        }

        // Fall back to built-in trigger names
        let trigger_name = config["trigger"].as_str().ok_or_else(|| {
            anyhow!(
                "run_skill task {} missing 'trigger' or 'skill_name' in action_config",
                task.id
            )
        })?;

        match trigger_name {
            "heartbeat" => Ok(self.dispatch_heartbeat(task).await?),
            "reflection" => Ok(self.dispatch_reflection(task).await?),
            "auto_pull_groomed" => Ok(self.dispatch_auto_pull_groomed(task).await?),
            "wip_rescue" => Ok(self.dispatch_wip_rescue(task).await?),
            "curator_review" => Ok(self.dispatch_curator_review(task).await?),
            other => Err(anyhow!("unknown run_skill trigger: {}", other).into()),
        }
    }

    /// Run an arbitrary skill as a silent background agent turn.
    ///
    /// The skill name is looked up in the skill registry. If not found, returns an error.
    /// Returns `DispatchError::AgentBusy` if the agent is busy so the caller can re-queue.
    async fn dispatch_skill_by_name(
        &self,
        task: &Task,
        skill_name: &str,
        _config: &serde_json::Value,
    ) -> Result<(), DispatchError> {
        // Acquire agent lock — return error if busy so caller can re-queue
        let _guard = if let Some(ref lock) = self.agent_lock {
            match lock.try_lock() {
                Ok(guard) => Some(guard),
                Err(_) => {
                    debug!(task_id = %task.id, skill = skill_name, "agent busy, deferring skill run");
                    return Err(DispatchError::AgentBusy(task.id.clone()));
                }
            }
        } else {
            None
        };

        let session_id = format!("skill-{}-{}", skill_name, uuid::Uuid::new_v4());
        let trace_id = mika_common::trace::generate_trace_id();
        info!(task_id = %task.id, skill = skill_name, session_id = %session_id, trace_id = %trace_id, "running skill task");

        if let Err(e) = self
            .db
            .create_session_with_parent(
                &session_id,
                &task.agent_id,
                "system",
                Some(r#"{"trigger": "skill_run"}"#),
                task.created_by_session.as_deref(),
                Some(&task.id),
            )
            .await
        {
            warn!(session_id = %session_id, error = %e, "failed to create session for skill run");
        }

        let params = SilentAgentParams {
            db: &self.db,
            tier: self.tier,
            llm: self.llm.as_ref(),
            tools: &self.tools,
            skills: &self.skills,
            trigger: SilentTrigger::SkillRun {
                skill_name: skill_name.to_string(),
            },
            home_dir: &self.home_dir,
            session_id: &session_id,
            message_sender: self.message_sender.clone(),
            embedding_client: self.embedding_client.as_ref(),
            brave_api_key: self.brave_api_key.as_deref(),
            gateway_url: self.gateway_url.as_deref(),
            internal_token: self.internal_token.as_deref(),
            github_token: self.github_token.as_deref(),
            github_app: self.github_app.as_deref(),
            skills_dirty: &self.skills_dirty,
            settings: Some(&self.settings),
            trace_id: Some(trace_id.clone()),
        };

        if let Err(e) = run_silent_agent(&params).await {
            warn!(task_id = %task.id, skill = skill_name, error = %e, "skill task agent run failed");
        }

        if let Err(e) = self.db.end_session(&session_id).await {
            warn!(session_id = %session_id, error = %e, "failed to end skill session");
        }
        if let Some(ref map) = self.pr_reviews_posted {
            map.remove(&session_id);
        }

        self.write_execution_trace(&task.id, &trace_id).await;

        Ok(())
    }

    /// Resume the agent after a callback task completes/fails, or when a reminder fires.
    ///
    /// Two entry paths with different lifecycles:
    /// - **Callback** (`trigger_type = "callback"`): reads `task.result`, marks delivered
    ///   after dispatch, uses `SilentTrigger::Callback` with untrusted framing.
    /// - **Reminder** (`trigger_type = "time"` or `"recurring"`): reads `action_config.text`,
    ///   does NOT mark delivered (caller `fire_task` handles completion), uses
    ///   `SilentTrigger::Reminder` with trusted framing. See #363.
    ///
    /// Returns `Err` with a specific message when the agent is busy so the caller
    /// can re-queue the task instead of losing the callback result.
    pub(crate) async fn dispatch_resume_agent(&self, task: &Task) -> Result<(), DispatchError> {
        let is_callback = task.trigger_type == "callback";

        // Determine context and trigger based on entry path
        let is_deferred_dispatch =
            is_callback && task.label == crate::agent::DEFERRED_DISPATCH_LABEL;

        let (trigger, session_prefix, session_trigger_meta) = if is_deferred_dispatch {
            // mika#1011 — Deferred-dispatch retry path: the dispatch slot is free,
            // the agent's only job is to re-invoke run_claude_pilot.
            let parent_task_id = task
                .parent_task_id
                .clone()
                .unwrap_or_else(|| task.id.clone());
            (
                SilentTrigger::DeferredDispatch {
                    task_id: task.id.clone(),
                    parent_task_id,
                    action_config: task.action_config.clone(),
                },
                "deferred-dispatch",
                r#"{"trigger": "deferred_dispatch"}"#,
            )
        } else if is_callback {
            // Callback path: read from task.result
            let is_failed = task.status == "failed";
            let result = match task.result.clone() {
                Some(r) if !r.is_empty() => r,
                _ if is_failed => crate::agent::FAILED_TASK_FALLBACK.to_string(),
                _ => {
                    return Err(anyhow!(
                        "resume_agent task {} has no result — callback may not have completed yet",
                        task.id
                    )
                    .into());
                }
            };
            (
                SilentTrigger::Callback {
                    task_id: task.id.clone(),
                    label: task.label.clone(),
                    result,
                    failed: is_failed,
                    parent_task_id: task.parent_task_id.clone(),
                },
                "callback",
                r#"{"trigger": "callback"}"#,
            )
        } else {
            // Reminder path: read from action_config.text
            let config: serde_json::Value =
                serde_json::from_str(&task.action_config).unwrap_or(serde_json::Value::Null);
            let message = match config["text"].as_str() {
                Some(text) => text.to_string(),
                None => {
                    warn!(task_id = %task.id, "reminder task missing text in action_config, falling back to label");
                    task.label.clone()
                }
            };
            (
                SilentTrigger::Reminder {
                    task_id: task.id.clone(),
                    message,
                },
                "reminder",
                r#"{"trigger": "reminder"}"#,
            )
        };

        // Acquire agent lock — return error if busy so caller can re-queue
        let _guard = if let Some(ref lock) = self.agent_lock {
            match lock.try_lock() {
                Ok(guard) => Some(guard),
                Err(_) => {
                    debug!(task_id = %task.id, "agent busy, deferring resume_agent");
                    return Err(DispatchError::AgentBusy(task.id.clone()));
                }
            }
        } else {
            None
        };

        let session_id = format!("{session_prefix}-{}", uuid::Uuid::new_v4());
        let trace_id = mika_common::trace::generate_trace_id();
        info!(task_id = %task.id, session_id = %session_id, trace_id = %trace_id, label = %task.label, path = session_prefix, "resuming agent for {} task", session_prefix);

        if let Err(e) = self
            .db
            .create_session_with_parent(
                &session_id,
                &task.agent_id,
                "system",
                Some(session_trigger_meta),
                task.created_by_session.as_deref(),
                Some(&task.id),
            )
            .await
        {
            warn!(session_id = %session_id, error = %e, "failed to create session for {}", session_prefix);
        }

        // Best-effort: extract structured metadata from the callback result text
        // and persist it to the parent task BEFORE the silent agent runs.
        // This guarantees at minimum session_id, cost_usd, duration_ms, and turns
        // are captured even if the agent exhausts its step budget.
        if is_callback {
            try_extract_callback_metadata(&self.db, task).await;
            // mika#965: Write a human-readable callback summary to task_messages
            // so the dispatch session's next rebuild_context() includes it.
            try_write_callback_summary(&self.db, task).await;
            // #958: If the callback carries a pr_url (success indicator) and the
            // parent was reaped to `failed`, promote it back to `completed`.
            try_promote_parent_on_retry_success(&self.db, task).await;
            // mika#1162: success-side structural backstop. If the callback
            // carries a pr_url and the parent is still `in_progress` (silent
            // turn hasn't run yet, or hasn't called update_task_status), mark
            // the parent `completed` so the dispatch slot frees without
            // depending on the LLM to fire the transition. Coupled pair with
            // `reap_orphaned_parent_tasks` (failure path).
            try_complete_parent_on_callback_success(&self.db, task).await;
            // mika#2158 AC8: refusal-side sibling of the two above. A
            // `dispatch-lib` auto-skip (canonically `already_groomed`) reached
            // the engine and produced nothing — the pre-created tracking row
            // aged ~60 min into `phantom_aged_out` while counting as `in_flight`
            // and zeroing the auto-pull re-drive budget on every tick. Resolve
            // it here, at the moment the refusal arrives.
            try_resolve_parent_on_dispatch_refusal(&self.db, task).await;
            // mika#1289 / mika#1614: structural counterpart to the prompt-level
            // groom-success handler in self-dev-callback (PR #1291). When a groom
            // callback delivers with `Outcome: PLAN_GROOMED`, the engine spawns the
            // implement-class dev-pilot dispatch DIRECTLY (mirroring the
            // ready_label_handler engine-side path, mika#1572) — no `gh` label
            // round-trip, no LLM-mediated turn, so it fires every time. The
            // prompt-level path in self-dev-callback remains as defense-in-depth.
            try_dispatch_pilot_after_groom_success(
                &self.db,
                task,
                self.github_token.as_deref(),
                &self.skills,
            )
            .await;
        }

        // Suppress user-facing notifications for team-child callbacks.
        // The consolidated team-run notification (from run_team tool or
        // dispatch_invoke_orchestrator) handles user delivery — per-child
        // silent turns should not independently call send_message on the
        // user channel. The silent turn still runs for internal state updates.
        let message_sender = if is_team_child_callback(task) {
            info!(
                task_id = %task.id,
                team_run_id = ?task.team_run_id,
                parent_task_id = ?task.parent_task_id,
                agent_id = %task.agent_id,
                "team_child_callback_notification_suppressed"
            );
            Some(Arc::new(crate::messaging::NoopSender) as Arc<dyn crate::messaging::MessageSender>)
        } else {
            self.message_sender.clone()
        };

        let params = SilentAgentParams {
            db: &self.db,
            tier: self.tier,
            llm: self.llm.as_ref(),
            tools: &self.tools,
            skills: &self.skills,
            trigger,
            home_dir: &self.home_dir,
            session_id: &session_id,
            message_sender,
            embedding_client: self.embedding_client.as_ref(),
            brave_api_key: self.brave_api_key.as_deref(),
            gateway_url: self.gateway_url.as_deref(),
            internal_token: self.internal_token.as_deref(),
            github_token: self.github_token.as_deref(),
            github_app: self.github_app.as_deref(),
            skills_dirty: &self.skills_dirty,
            settings: Some(&self.settings),
            trace_id: Some(trace_id.clone()),
        };

        if let Err(e) = run_silent_agent(&params).await {
            warn!(task_id = %task.id, error = %e, "resume_agent run failed");

            // mika#2179 — record the failure and back the row off BEFORE the
            // re-arm below. Order matters twice over: the re-arm creates a new
            // `pending` row and returns early on some paths, and this row is
            // the one that keeps being re-selected every 60s scan. Until this
            // call existed, the branch wrote a `warn!` and nothing else — no
            // counter, no audit event, no `next_fire_at` — so callback
            // `800d739f` re-took the agent lock once a minute for five hours
            // while its parent died of age waiting for the return.
            if is_callback {
                self.record_callback_delivery_failure(task, &e, &session_id, &trace_id)
                    .await;
            }

            // mika#2045 — a deferred wrapper whose turn errored is consumed all
            // the same: promotion already set it `completed`, so it has left the
            // pending queue for good. This branch used to do nothing at all — no
            // mark_delivered, no R9 detection, no event — which is why the
            // 2026-08-29 09:10-09:51Z occurrence stranded four tasks without a
            // single `deferred_dispatch_noop_completion` in the log. Re-arm here
            // too, or the loudest failure mode stays the silent one.
            if is_callback && task.label == crate::agent::DEFERRED_DISPATCH_LABEL {
                self.rearm_consumed_deferred_wrapper(task, "silent_turn_error")
                    .await;
            }
        } else if is_callback {
            // Mark delivered so TUI polling doesn't re-process this callback.
            // Only for callbacks — reminder lifecycle is managed by fire_task().
            match self.db.mark_task_delivered(&task.id).await {
                // mika#2179 — the latency this delivery just ended is written
                // where it can be queried, gated on the transition actually
                // having happened. `mark_task_delivered` returns false when
                // another path won the race, and a measurement attributed to a
                // delivery we did not make is worse than none.
                Ok(true) => {
                    self.record_callback_delivery_success(task, &session_id, &trace_id)
                        .await;
                }
                Ok(false) => {
                    debug!(
                        task_id = %task.id,
                        "callback was already delivered — skipping latency measurement"
                    );
                }
                Err(e) => {
                    warn!(task_id = %task.id, error = %e, "failed to mark callback task as delivered");
                }
            }

            // #991 — Post-callback advance backstop. After a milestone/project-context
            // callback turn completes, check whether the queue was advanced. If not,
            // fire a PostCallbackAdvance trigger to give the agent one more turn with
            // explicit advance instructions.
            self.maybe_fire_post_callback_advance(task).await;

            // mika#1011 — Promote next pending deferred-dispatch callback (FIFO).
            // The blocking dispatch just completed, so the slot is free. Promote
            // the oldest pending deferred callback to 'completed' status. The
            // engine's next periodic scan (~60s) dispatches it as a
            // DeferredDispatch silent turn. Must run AFTER mark_task_delivered.
            //
            // mika#1070 — Removed anti-cascade guard. Rationale at the time was
            // "promotion is just a DB write, no call-stack cascade."
            //
            // mika#1124 — Re-added the anti-cascade guard with refined rationale.
            // Empirically (today, 2026-05-15 drain of 4-ticket queue), when a
            // DeferredDispatch turn's silent run completes WITHOUT actually calling
            // `run_claude_pilot` (e.g., LLM emits no tool calls, hits the stall
            // detector, or has any other no-op shape), chain-promoting the next
            // deferred callback creates an inline loop: N deferred wrappers each
            // get promoted → silent turn fires → no pilot dispatched → marked
            // delivered → chain-promotes the next → repeat, until the queue is
            // empty. Reaper then kills the parent (no PR url, grace expired).
            //
            // Observed: parent `19c8bbbe` (mika#861 today) accumulated 10 deferred
            // callbacks all delivered with result "deferred dispatch slot freed",
            // none ran the impl pilot. Same shape on `14f9f32d` (mika#1124's own
            // groom dispatch — 6 deferred wrappers, none ran).
            //
            // Fix: skip inline chain-promotion when the just-completed task is
            // itself a deferred wrapper. Deferred wrappers don't represent real
            // dispatch completion — their completion shouldn't trigger queue
            // progression. Real callbacks (impl-class run_claude_pilot completions)
            // still chain immediately. The periodic backstop
            // (`promote_pending_deferred_if_idle`, ~60s cadence, slot-checked)
            // handles re-promotion when the dispatch slot becomes truly idle.
            if task.label != crate::agent::DEFERRED_DISPATCH_LABEL {
                self.dispatch_next_deferred_callback().await;
            } else {
                debug!(
                    task_id = %task.id,
                    "deferred wrapper completed — skipping inline chain-promotion (mika#1124); periodic backstop will handle re-promotion if slot is idle"
                );

                // R9 (#1172): detect no-op wrapper completion. If the parent has
                // no active non-deferred callback child after this wrapper completes,
                // the wrapper's silent turn did NOT spawn a real dispatch.
                if let Some(ref parent_id) = task.parent_task_id {
                    match self
                        .db
                        .has_non_deferred_active_callback_child(parent_id)
                        .await
                    {
                        Ok(false) => {
                            warn!(
                                event = "deferred_dispatch_noop_completion",
                                task_id = %task.id,
                                parent_task_id = %parent_id,
                                "deferred wrapper completed without spawning a real callback — no-op cascade risk (mika#1124)"
                            );
                            // W4: audit event for no-op completion
                            if let Err(e) = self
                                .db
                                .log_audit_event(
                                    &session_id,
                                    "deferred_dispatch_noop_completion",
                                    &format!("task:{}", task.id),
                                    None,
                                    Some("noop_completion"),
                                    Some(&format!(
                                        "parent:{parent_id} — wrapper completed without real dispatch"
                                    )),
                                    Some(&trace_id),
                                )
                                .await
                            {
                                warn!(error = %e, "failed to write deferred_dispatch_noop_completion audit event");
                            }

                            // mika#2045 — R9 detected the no-op but stopped at
                            // the warning. Detection without repair is why the
                            // 792 occurrences of this event never healed
                            // anything. Re-arm so the parent stays dispatchable.
                            self.rearm_consumed_deferred_wrapper(task, "noop_completion")
                                .await;
                        }
                        Ok(true) => {
                            debug!(
                                task_id = %task.id,
                                parent_task_id = %parent_id,
                                "deferred wrapper completed — parent has active non-deferred child (healthy path)"
                            );
                        }
                        Err(e) => {
                            warn!(
                                task_id = %task.id,
                                error = %e,
                                "failed to check for non-deferred callback children — R9 detection skipped"
                            );
                        }
                    }
                }
            }
        }

        if let Err(e) = self.db.end_session(&session_id).await {
            warn!(session_id = %session_id, error = %e, "failed to end {} session", session_prefix);
        }
        if let Some(ref map) = self.pr_reviews_posted {
            map.remove(&session_id);
        }

        self.write_execution_trace(&task.id, &trace_id).await;

        Ok(())
    }

    /// Resume a team run after all child tasks (agent delegations) have completed.
    ///
    /// Loads child task results, deserializes the checkpoint from `input_context`,
    /// and calls `resume_team_run` to continue the team pipeline from the specified phase.
    ///
    /// **Race guard:** Only proceeds if the team run is in `suspended` status. When agents
    /// complete synchronously, `execute_tasks()` cancels this parent task and continues
    /// the pipeline directly. If the task fires anyway (race window), this check prevents
    /// a duplicate review/deliver phase.
    async fn dispatch_invoke_orchestrator(
        &self,
        task: &Task,
        config: &serde_json::Value,
    ) -> Result<()> {
        let team_run_id = task
            .team_run_id
            .as_deref()
            .ok_or_else(|| anyhow!("invoke_orchestrator task {} has no team_run_id", task.id))?;
        let team_name = config["team_name"]
            .as_str()
            .ok_or_else(|| anyhow!("missing team_name in action_config"))?;
        let next_phase = config["next_phase"]
            .as_str()
            .ok_or_else(|| anyhow!("missing next_phase in action_config"))?;

        // Race guard: only resume if the team run is actually suspended.
        // When agents complete synchronously, execute_tasks() cancels this task and
        // continues the pipeline directly. If we still fire (race window), bail out.
        match self.db.load_team_run_by_id(team_run_id).await? {
            Some(run) if run.status == "suspended" => {
                // Expected path for async resumption — proceed.
            }
            Some(run) => {
                warn!(
                    task_id = %task.id,
                    team_run_id,
                    status = %run.status,
                    "invoke_orchestrator fired but team run is not suspended, skipping"
                );
                // Mark as completed so it doesn't re-fire.
                if let Err(e) = self.db.update_task_status(&task.id, "completed").await {
                    warn!(task_id = %task.id, error = %e, "failed to mark stale orchestrator task as completed");
                }
                return Ok(());
            }
            None => {
                warn!(
                    task_id = %task.id,
                    team_run_id,
                    "invoke_orchestrator fired but team run not found, skipping"
                );
                if let Err(e) = self.db.update_task_status(&task.id, "completed").await {
                    warn!(task_id = %task.id, error = %e, "failed to mark orphaned orchestrator task as completed");
                }
                return Ok(());
            }
        }

        // Load child task results (each child = one agent's output)
        let children = self.db.get_child_tasks(&task.id).await?;
        let child_results: Vec<_> = children
            .iter()
            .map(|c| {
                serde_json::json!({
                    "agent": c.label.strip_prefix("team-agent-").unwrap_or(&c.label),
                    "status": c.status,
                    "result": c.result.as_deref().unwrap_or("")
                })
            })
            .collect();

        let team_state = task.input_context.as_deref().ok_or_else(|| {
            anyhow!(
                "invoke_orchestrator task {} has no input_context (checkpoint)",
                task.id
            )
        })?;

        info!(
            task_id = %task.id,
            team_run_id,
            team_name,
            next_phase,
            children = children.len(),
            "resuming team run from checkpoint"
        );

        crate::teams::resume_team_run(
            team_run_id,
            team_name,
            next_phase,
            team_state,
            &serde_json::to_string(&child_results)?,
            &self.home_dir,
            &self.db,
            self.github_app.clone(),
            self.pr_reviews_posted.clone(),
            self.tier, // mika#1962 — cached at agent init, never re-read here
        )
        .await
        .with_context(|| format!("resuming team_run_id={team_run_id}"))?;

        // Consolidated team-run notification (async path).
        // Paired with run_team tool (sync path); see
        // docs/plans/2026-04-24-003-fix-team-callback-consolidation-plan.md
        // for why this lives here and not in TeamEngine::finalize_and_shutdown
        // (keeps the team engine free of a user_message_sender field).
        if let Some(run) = self.db.load_team_run_by_id(team_run_id).await? {
            if let Some(msg) =
                crate::teams::notification::build_run_completion_message_from_row(&run)
            {
                if let Some(ref sender) = self.message_sender {
                    match sender.send(&msg.text).await {
                        Ok(crate::messaging::SendOutcome::Delivered) => {
                            info!(
                                team_run_id = %run.id,
                                team_name = %run.team_name,
                                status = %run.status,
                                notification_kind = %msg.notification_kind,
                                deliverable_chars = msg.deliverable_chars,
                                truncated = msg.truncated,
                                path = "async",
                                "team_run_notified"
                            );
                        }
                        Ok(crate::messaging::SendOutcome::NoChannel) => {
                            // NoChannel is permanent — silent per dispatcher policy.
                        }
                        Ok(crate::messaging::SendOutcome::Failed { reason }) => {
                            warn!(
                                team_run_id = %run.id,
                                error = %reason,
                                "team_run_notification_delivery_failed"
                            );
                        }
                        Err(e) => {
                            warn!(
                                team_run_id = %run.id,
                                error = %e,
                                "team_run_notification_send_error"
                            );
                        }
                    }
                }
                // Log warning for completed-without-deliverable
                if msg.notification_kind == "fallback" {
                    warn!(
                        team_run_id = %run.id,
                        "team run completed without a deliverable"
                    );
                }
            } else {
                // Non-terminal status (running/suspended) after resume — team run
                // re-suspended for another delegation cycle. Notification deferred
                // to the next resume that reaches a terminal state.
                debug!(
                    team_run_id = %run.id,
                    team_name = %run.team_name,
                    status = %run.status,
                    "team run non-terminal after resume, deferring notification"
                );
            }
        }

        Ok(())
    }

    /// Run the heartbeat silent agent with all pre-filter checks.
    ///
    /// Pre-filters (same logic as the removed `/heartbeat` HTTP endpoint):
    /// 1. Active hours (08:00–21:00 in customer's local timezone)
    /// 2. Rate limit: max 1 heartbeat per hour
    /// 3. Rate limit: max 3 heartbeats per day
    /// 4. Skip if user messaged within 2 hours
    /// 5. Skip if agent is busy (try_lock)
    async fn dispatch_heartbeat(&self, task: &Task) -> Result<()> {
        if !self.heartbeat_should_run().await {
            debug!(task_id = %task.id, "heartbeat skipped by pre-filter");
            return Ok(());
        }

        // Acquire agent lock (skip if busy — heartbeat is always skippable)
        let _guard = if let Some(ref lock) = self.agent_lock {
            match lock.try_lock() {
                Ok(guard) => Some(guard),
                Err(_) => {
                    debug!(task_id = %task.id, "agent busy, deferring heartbeat");
                    return Ok(());
                }
            }
        } else {
            None
        };

        let session_id = format!("heartbeat-{}", uuid::Uuid::new_v4());
        let trace_id = mika_common::trace::generate_trace_id();
        info!(task_id = %task.id, session_id = %session_id, trace_id = %trace_id, "running heartbeat");

        // Heartbeat sessions are autonomous (not triggered by a user conversation),
        // so they intentionally use create_session_with_metadata (no parent_session_id).
        if let Err(e) = self
            .db
            .create_session_with_metadata(
                &session_id,
                &task.agent_id,
                "system",
                Some(r#"{"trigger": "heartbeat"}"#),
                Some(&task.id),
            )
            .await
        {
            warn!(session_id = %session_id, error = %e, "failed to create session for heartbeat");
        }

        let params = SilentAgentParams {
            db: &self.db,
            tier: self.tier,
            llm: self.llm.as_ref(),
            tools: &self.tools,
            skills: &self.skills,
            trigger: SilentTrigger::Heartbeat,
            home_dir: &self.home_dir,
            session_id: &session_id,
            message_sender: self.message_sender.clone(),
            embedding_client: self.embedding_client.as_ref(),
            brave_api_key: self.brave_api_key.as_deref(),
            gateway_url: self.gateway_url.as_deref(),
            internal_token: self.internal_token.as_deref(),
            github_token: self.github_token.as_deref(),
            github_app: self.github_app.as_deref(),
            skills_dirty: &self.skills_dirty,
            settings: Some(&self.settings),
            trace_id: Some(trace_id.clone()),
        };

        if let Err(e) = run_silent_agent(&params).await {
            warn!(task_id = %task.id, error = %e, "heartbeat agent run failed");
        }

        if let Err(e) = self.db.end_session(&session_id).await {
            warn!(session_id = %session_id, error = %e, "failed to end heartbeat session");
        }
        if let Some(ref map) = self.pr_reviews_posted {
            map.remove(&session_id);
        }

        self.write_execution_trace(&task.id, &trace_id).await;

        // Record send for rate-limit tracking
        if let Err(e) = self.db.record_heartbeat_send().await {
            warn!(task_id = %task.id, error = %e, "failed to record heartbeat send");
        }

        Ok(())
    }

    /// Run the auto-pull groomed ticket logic (mika#1363).
    ///
    /// This does NOT run a silent agent turn — it directly executes the
    /// auto-pull selection logic which calls `gh` CLI and applies the `ready`
    /// label on the selected ticket. The webhook-driven dispatch flow then
    /// picks up the labelled ticket.
    async fn dispatch_auto_pull_groomed(&self, task: &Task) -> Result<()> {
        // mika#2205 — PAT d'abord, App en repli. Voir `resolve_periodic_scan_token`
        // pour le raisonnement d'identité ADR-008 : la bascule du label `ready` ne
        // lit pas l'auteur, donc l'identité bot de l'App convient ici.
        let resolved = resolve_periodic_scan_token(
            &self.settings,
            self.github_app.as_deref(),
            &task.id,
            PeriodicScan::AutoPull,
        )
        .await;
        let github_token = match resolved.as_deref() {
            Some(t) => t,
            None => return Ok(()),
        };

        let trace_id = mika_common::trace::generate_trace_id();
        let session_id = format!("auto-pull-{}", uuid::Uuid::new_v4());

        info!(
            task_id = %task.id,
            trace_id = %trace_id,
            "auto_pull: running groomed ticket selection"
        );

        let result = crate::auto_pull::auto_pull_groomed_ticket(
            &self.db,
            github_token,
            &trace_id,
            &session_id,
        )
        .await;

        match result {
            Some(issue_number) => {
                info!(
                    task_id = %task.id,
                    issue = issue_number,
                    trace_id = %trace_id,
                    "auto_pull: selected and labelled #{issue_number} ready"
                );
            }
            None => {
                debug!(
                    task_id = %task.id,
                    trace_id = %trace_id,
                    "auto_pull: no action taken"
                );
            }
        }

        Ok(())
    }

    /// Run the auto-resume wip-rescue drafts scan (mika#1852).
    ///
    /// Cron-driven, low-priority (fond-de-file, AC7): like `auto_pull` this does
    /// NOT run a silent agent turn — it directly executes the scan which drives
    /// at most one eligible `wip-rescue` draft PR back toward review (rebase →
    /// clippy → perimeter gate → un-draft), or bails it to a human. The
    /// concurrency cap of 1 (AC6) is intrinsic: the scan handles one draft per
    /// tick.
    async fn dispatch_wip_rescue(&self, task: &Task) -> Result<()> {
        // mika#2205 — PAT d'abord, App en repli. Voir `resolve_periodic_scan_token` :
        // rebase, push de brouillon, `gh pr ready` et commentaire ne sont pas des
        // opérations dont GitHub lit l'auteur au sens d'ADR-008, donc l'identité bot
        // de l'App est acceptable en repli.
        let resolved = resolve_periodic_scan_token(
            &self.settings,
            self.github_app.as_deref(),
            &task.id,
            PeriodicScan::WipRescue,
        )
        .await;
        let github_token = match resolved.as_deref() {
            Some(t) => t,
            None => return Ok(()),
        };

        let trace_id = mika_common::trace::generate_trace_id();
        let session_id = format!("wip-rescue-{}", uuid::Uuid::new_v4());

        info!(
            task_id = %task.id,
            trace_id = %trace_id,
            "wip_rescue: running auto-resume scan"
        );

        let result = crate::wip_rescue::auto_resume_wip_rescue_drafts(
            &self.db,
            github_token,
            &trace_id,
            &session_id,
        )
        .await;

        match result {
            Some(count) => {
                info!(
                    task_id = %task.id,
                    resumed = count,
                    trace_id = %trace_id,
                    "wip_rescue: scan complete"
                );
            }
            None => {
                debug!(
                    task_id = %task.id,
                    trace_id = %trace_id,
                    "wip_rescue: no action taken"
                );
            }
        }

        Ok(())
    }

    /// Run the curator review (mika#1584).
    ///
    /// This is a deterministic query+notification task — no LLM call.
    /// Queries for idle agent-authored skills and emits proposals.
    async fn dispatch_curator_review(&self, task: &Task) -> Result<()> {
        let identity = crate::prompt::load_identity_async(&self.home_dir).await;
        let max_idle_days = identity
            .curator
            .as_ref()
            .and_then(|c| c.max_idle_days)
            .unwrap_or(30);

        let candidates = self
            .db
            .get_archival_candidates(&task.agent_id, max_idle_days)
            .await?;

        if candidates.is_empty() {
            debug!(
                agent = %task.agent_id,
                "curator review: no archival candidates"
            );
            return Ok(());
        }

        let proposals = crate::skills::curator::build_proposals(&candidates, max_idle_days);

        crate::skills::curator::emit_curator_proposal(&self.db, &task.agent_id, &proposals).await?;

        // Notify operator if message_sender is available
        if let Some(ref sender) = self.message_sender {
            let summary = format!(
                "[Curator] {} skill(s) idle >{}d for agent {}. Run `mika skills curator status --agent {}` for details.",
                candidates.len(),
                max_idle_days,
                task.agent_id,
                task.agent_id,
            );
            let _ = sender.send(&summary).await;
        }

        Ok(())
    }

    /// Run the nightly reflection silent agent with pre-filter checks.
    ///
    /// Pre-filters:
    /// 1. Reflection enabled in identity.toml
    /// 2. Not already run today
    /// 3. User not active within 30 minutes
    /// 4. Conversations exist today
    /// 5. Skip if agent is busy (try_lock)
    async fn dispatch_reflection(&self, task: &Task) -> Result<()> {
        let identity = crate::prompt::load_identity_async(&self.home_dir).await;
        let config = match identity.reflection.as_ref().filter(|c| c.enabled) {
            Some(c) => c,
            None => {
                debug!(task_id = %task.id, "reflection disabled in identity.toml, skipping");
                return Ok(());
            }
        };
        let _ = config; // validated above

        let tz_str = self
            .db
            .get_customer_config("timezone")
            .await
            .ok()
            .flatten()
            .unwrap_or_else(|| "UTC".to_string());

        // Check if already ran today
        match self.db.last_reflection_run_today(&tz_str).await {
            Ok(true) => {
                debug!(task_id = %task.id, "reflection already ran today, skipping");
                return Ok(());
            }
            Err(e) => {
                warn!(task_id = %task.id, error = %e, "failed to check last reflection run");
                return Ok(());
            }
            _ => {}
        }

        // Skip if user active within 30 minutes
        if let Ok(Some(last_ts)) = self.db.last_user_message_time().await {
            let elapsed = if let Ok(last_dt) = crate::timestamp::parse(&last_ts) {
                chrono::Utc::now()
                    .signed_duration_since(last_dt)
                    .num_seconds()
            } else {
                warn!(timestamp = %last_ts, "failed to parse last_user_message_time, treating as stale");
                i64::MAX
            };
            if elapsed < 30 * 60 {
                debug!(task_id = %task.id, "user active within 30 min, deferring reflection");
                return Ok(());
            }
        }

        // Skip if no conversations today
        let midnight_str = crate::timestamp::format(&crate::db::today_midnight_utc(&tz_str));
        let conversations = self
            .db
            .get_messages_since(&midnight_str)
            .await
            .unwrap_or_default();
        if conversations.is_empty() {
            debug!(task_id = %task.id, "no conversations today, skipping reflection");
            return Ok(());
        }

        // Acquire agent lock (skip if busy)
        let _guard = if let Some(ref lock) = self.agent_lock {
            match lock.try_lock() {
                Ok(guard) => Some(guard),
                Err(_) => {
                    debug!(task_id = %task.id, "agent busy, deferring reflection");
                    return Ok(());
                }
            }
        } else {
            None
        };

        let tz: chrono_tz::Tz = tz_str.parse().unwrap_or(chrono_tz::UTC);
        let today_str = chrono::Utc::now()
            .with_timezone(&tz)
            .format("%Y-%m-%d")
            .to_string();
        let session_id = format!("reflection-{today_str}");
        let trace_id = mika_common::trace::generate_trace_id();
        info!(task_id = %task.id, session_id = %session_id, trace_id = %trace_id, "running daily reflection");

        // Reflection sessions are autonomous (not triggered by a user conversation),
        // so they intentionally use create_session_with_metadata (no parent_session_id).
        if let Err(e) = self
            .db
            .create_session_with_metadata(
                &session_id,
                &task.agent_id,
                "system",
                Some(r#"{"trigger": "reflection"}"#),
                Some(&task.id),
            )
            .await
        {
            warn!(session_id = %session_id, error = %e, "failed to create session for reflection");
        }

        let params = SilentAgentParams {
            db: &self.db,
            tier: self.tier,
            llm: self.llm.as_ref(),
            tools: &self.tools,
            skills: &self.skills,
            trigger: SilentTrigger::Reflection,
            home_dir: &self.home_dir,
            session_id: &session_id,
            message_sender: self.message_sender.clone(),
            embedding_client: self.embedding_client.as_ref(),
            brave_api_key: self.brave_api_key.as_deref(),
            gateway_url: self.gateway_url.as_deref(),
            internal_token: self.internal_token.as_deref(),
            github_token: self.github_token.as_deref(),
            github_app: self.github_app.as_deref(),
            skills_dirty: &self.skills_dirty,
            settings: Some(&self.settings),
            trace_id: Some(trace_id.clone()),
        };

        match run_silent_agent(&params).await {
            Ok(()) => {
                if let Err(e) = self.db.record_reflection_run("completed", 0, None).await {
                    warn!(task_id = %task.id, error = %e, "failed to record reflection run");
                }
            }
            Err(e) => {
                warn!(task_id = %task.id, error = %e, "reflection run failed");
                if let Err(db_err) = self
                    .db
                    .record_reflection_run("failed", 0, Some(&e.to_string()))
                    .await
                {
                    warn!(task_id = %task.id, error = %db_err, "failed to record reflection run failure in DB");
                }
            }
        }

        if let Err(e) = self.db.end_session(&session_id).await {
            warn!(session_id = %session_id, error = %e, "failed to end reflection session");
        }
        if let Some(ref map) = self.pr_reviews_posted {
            map.remove(&session_id);
        }

        self.write_execution_trace(&task.id, &trace_id).await;

        Ok(())
    }

    /// Heartbeat pre-filter: checks active hours, rate limits, and recent user activity.
    async fn heartbeat_should_run(&self) -> bool {
        let tz_str = self
            .db
            .get_customer_config("timezone")
            .await
            .ok()
            .flatten()
            .unwrap_or_else(|| "UTC".to_string());

        let now_utc = chrono::Utc::now();
        let tz: chrono_tz::Tz = tz_str.parse().unwrap_or(chrono_tz::UTC);
        let now_local = now_utc.with_timezone(&tz);
        let hour = now_local.hour();

        // 1. Active hours (08:00–21:00 local)
        if !(8..21).contains(&hour) {
            return false;
        }

        // 2. Rate limit: max 1 per hour
        if self.db.count_heartbeat_sends_last_hour().await.unwrap_or(0) >= 1 {
            return false;
        }

        // 3. Rate limit: max 3 per day
        if self
            .db
            .count_heartbeat_sends_today(&tz_str)
            .await
            .unwrap_or(0)
            >= 3
        {
            return false;
        }

        // 4. Skip if user messaged within 2 hours
        if let Ok(Some(last_ts)) = self.db.last_user_message_time().await {
            let elapsed = if let Ok(last_dt) = crate::timestamp::parse(&last_ts) {
                now_utc.signed_duration_since(last_dt).num_seconds()
            } else {
                warn!(timestamp = %last_ts, "failed to parse last_user_message_time, treating as stale");
                i64::MAX
            };
            if elapsed < 2 * 3600 {
                return false;
            }
        }

        true
    }

    /// mika#2179 — Record a failed callback delivery, then bound its retry.
    ///
    /// Called on the `Err` arm of `run_silent_agent` for callbacks only. Three
    /// things happen here, in this order, and the order is what makes the row
    /// stop consuming the agent lock:
    ///
    /// 1. the attempt counter and the last error class are written to
    ///    `metadata` (readable with `mika tasks get`, no log needed);
    /// 2. a `callback_delivery_failed` audit event is written (AC1);
    /// 3. `next_fire_at` is pushed forward by an exponential backoff, which
    ///    the engine's existing guard in `dispatch_undelivered_callbacks`
    ///    already reads — no scan-side change, no new column (AC3).
    ///
    /// Nothing here is fatal: every step warns and continues. A row that fails
    /// to record its failure must still reach the re-arm path below it, and a
    /// delivery bounded imperfectly is better than a delivery not attempted.
    async fn record_callback_delivery_failure(
        &self,
        task: &Task,
        err: &anyhow::Error,
        session_id: &str,
        trace_id: &str,
    ) {
        let class = classify_delivery_error(err);

        let previous_attempts = self
            .db
            .get_task_metadata_field(&task.id, DELIVERY_ATTEMPTS_KEY)
            .await
            .unwrap_or_default()
            .and_then(|v| v.parse::<u32>().ok())
            .unwrap_or(0);
        let attempts = previous_attempts.saturating_add(1);

        // Metadata first: it is the surface an operator reads on the row.
        //
        // The counter's write outcome is kept, because the entire escalation
        // ladder is derived from re-reading it. `set_task_metadata_field` runs
        // `json_set(COALESCE(metadata,'{}'), …)`, and SQLite raises a hard
        // "malformed JSON" error — not NULL — on a row whose `metadata` is not
        // valid JSON. Left merely warned-about, that row would read
        // `previous_attempts = 0` on *every* failure: `attempts` pinned at 1,
        // the backoff pinned at one scan interval, and `attempts >=
        // max_attempts` never true, so it is never quarantined. The starvation
        // this ticket closes would be silently restored, and the only trace
        // would be a stream of `callback_delivery_failed` rows all reading
        // `attempt:1` — which reads like a first failure, not a broken counter.
        let counter_written = self
            .set_delivery_metadata(&task.id, DELIVERY_ATTEMPTS_KEY, &attempts.to_string())
            .await;
        self.set_delivery_metadata(&task.id, DELIVERY_LAST_ERROR_CLASS_KEY, class.as_ref())
            .await;
        if previous_attempts == 0 {
            self.set_delivery_metadata(
                &task.id,
                DELIVERY_FIRST_FAILED_AT_KEY,
                &crate::timestamp::now(),
            )
            .await;
        }

        // AC1 — the named event. Modelled on `deferred_dispatch_noop_completion`
        // above: same `task:<id>` target shape, same fire-and-forget policy.
        let reasoning = format!(
            "attempt:{attempts} label:{} err:{}",
            task.label,
            crate::db::truncate_chars(&err.to_string(), DELIVERY_ERROR_REASONING_MAX_CHARS)
        );
        if let Err(e) = self
            .db
            .log_audit_event(
                session_id,
                "callback_delivery_failed",
                &format!("task:{}", task.id),
                None,
                Some(class.as_ref()),
                Some(&reasoning),
                Some(trace_id),
            )
            .await
        {
            warn!(error = %e, task_id = %task.id, "failed to write callback_delivery_failed audit event");
        }

        // AC3 — the backoff. See `delivery_backoff_secs` for why the shift is
        // clamped and the multiply saturates: `attempts` is unbounded by
        // construction, and the naive shift silently evaluates to zero at the
        // far end of a long outage.
        let max_attempts = self.settings.effective_callback_delivery_max_attempts();
        let max_backoff = self.settings.effective_callback_delivery_backoff_max_secs();
        let backoff_secs = if counter_written {
            delivery_backoff_secs(
                self.settings
                    .effective_callback_delivery_backoff_base_secs(),
                max_backoff,
                attempts,
                max_attempts,
            )
        } else {
            // The counter could not be persisted, so `attempts` is not a count
            // — it is 1, and it will be 1 again next scan. Escalating from an
            // unreliable number is not possible; the safe reading of "this row
            // just failed and I cannot tell how many times" is the ceiling, not
            // the base. Failing open here would pin the row at a 60s retry
            // forever, which is the starvation, not a mitigation of it.
            warn!(
                event = "callback_delivery_counter_unwritable",
                task_id = %task.id,
                label = %task.label,
                error_class = %class,
                "cannot persist the delivery-attempt counter (malformed task metadata?) — \
                 backing off at the ceiling instead of escalating from an unreliable count"
            );
            max_backoff
        };
        let fire_at = crate::timestamp::now_plus(chrono::Duration::seconds(backoff_secs as i64));
        if let Err(e) = self.db.update_task_next_fire_at(&task.id, &fire_at).await {
            warn!(error = %e, task_id = %task.id, "failed to write callback delivery backoff");
        }

        warn!(
            event = "callback_delivery_failed",
            task_id = %task.id,
            label = %task.label,
            error_class = %class,
            attempt = attempts,
            backoff_secs,
            next_fire_at = %fire_at,
            "callback delivery failed — backing off"
        );

        // AC3 — the quarantine, announced once at the threshold crossing.
        // Once, not per attempt: an event that repeats every minute is the same
        // silence as no event, just louder. Gated on `counter_written` for the
        // same reason the backoff is: a pinned counter would either never reach
        // the threshold, or re-announce the crossing on every single failure.
        if counter_written && attempts >= max_attempts && previous_attempts < max_attempts {
            self.set_delivery_metadata(
                &task.id,
                DELIVERY_QUARANTINED_AT_KEY,
                &crate::timestamp::now(),
            )
            .await;
            if let Err(e) = self
                .db
                .log_audit_event(
                    session_id,
                    "callback_delivery_quarantined",
                    &format!("task:{}", task.id),
                    None,
                    Some(class.as_ref()),
                    Some(&format!("attempts={attempts} backoff_secs={backoff_secs}")),
                    Some(trace_id),
                )
                .await
            {
                warn!(error = %e, task_id = %task.id, "failed to write callback_delivery_quarantined audit event");
            }
            warn!(
                event = "callback_delivery_quarantined",
                task_id = %task.id,
                label = %task.label,
                error_class = %class,
                attempts,
                "callback delivery quarantined — retrying at the backoff ceiling, result preserved"
            );
        }
    }

    /// mika#2179 — Record the latency of a delivery that just succeeded (AC2).
    ///
    /// The measurement is unconditional; only the `warn!` is gated on the
    /// configured threshold. That asymmetry is the point of the AC: before
    /// this, the only way to know how long a callback waited was to subtract
    /// two columns and hope nothing else had written to the row since.
    async fn record_callback_delivery_success(
        &self,
        task: &Task,
        session_id: &str,
        trace_id: &str,
    ) {
        // `completed_at` is NULL for a `failed` callback that never completed.
        // That is a real shape (the stale-failed path in the engine), so it is
        // reported as `unknown` rather than unwrapped or skipped.
        let wait_secs = task
            .completed_at
            .as_deref()
            .and_then(|ts| crate::timestamp::parse(ts).ok())
            .map(|completed| (chrono::Utc::now() - completed).num_seconds().max(0));

        let attempts = self
            .db
            .get_task_metadata_field(&task.id, DELIVERY_ATTEMPTS_KEY)
            .await
            .unwrap_or_default()
            .and_then(|v| v.parse::<u32>().ok())
            .unwrap_or(0);

        let after_value = match wait_secs {
            Some(secs) => secs.to_string(),
            None => "unknown".to_string(),
        };
        let reasoning = match wait_secs {
            Some(secs) => format!("wait_secs={secs} attempts={attempts} label={}", task.label),
            None => format!(
                "wait_secs=unknown (completed_at is NULL) attempts={attempts} label={}",
                task.label
            ),
        };

        if let Err(e) = self
            .db
            .log_audit_event(
                session_id,
                "callback_delivered",
                &format!("task:{}", task.id),
                None,
                Some(&after_value),
                Some(&reasoning),
                Some(trace_id),
            )
            .await
        {
            warn!(error = %e, task_id = %task.id, "failed to write callback_delivered audit event");
        }

        let threshold = self
            .settings
            .effective_callback_delivery_slow_threshold_secs();
        if let Some(secs) = wait_secs
            && secs as u64 > threshold
        {
            warn!(
                event = "callback_delivery_slow",
                task_id = %task.id,
                label = %task.label,
                wait_secs = secs,
                threshold_secs = threshold,
                attempts,
                "callback delivered far later than it completed"
            );
        }
    }

    /// Write one `metadata.$.<key>` field, warning on failure. Returns whether
    /// the write landed.
    ///
    /// Factored out because the failure path writes four of these and each one
    /// is individually non-fatal — inlining the `if let Err` four times buried
    /// the sequence it belongs to. The return value matters for exactly one of
    /// the four: the attempt counter, which the escalation ladder re-reads. The
    /// other three are pure operator surface and their loss costs a warn.
    async fn set_delivery_metadata(&self, task_id: &str, key: &str, value: &str) -> bool {
        match self.db.set_task_metadata_field(task_id, key, value).await {
            Ok(()) => true,
            Err(e) => {
                warn!(error = %e, task_id = %task_id, key, "failed to write callback delivery metadata");
                false
            }
        }
    }

    /// mika#2045 — Replace a deferred wrapper that was consumed without
    /// producing a real dispatch, so its parent stays dispatchable.
    ///
    /// `rearm_deferred_callback` owns the "did this turn actually dispatch?"
    /// guard, so both this call site and the reaper's inherit it.
    /// `pub(crate)` so the mika#2169 replay tests can drive this function
    /// directly. They must: the defect is what this function *writes*, and a
    /// test that reached it through the full silent-turn path would be
    /// measuring the turn, not the record.
    pub(crate) async fn rearm_consumed_deferred_wrapper(&self, task: &Task, cause: &str) {
        let Some(parent_id) = task.parent_task_id.as_deref() else {
            return;
        };

        let dispatch_class = task.dispatch_class.as_deref().unwrap_or("implement");
        let system_session = format!("system-{}", self.db.agent_id());

        // L2a (mika#2169) — refuse to re-arm into a terminal parent.
        //
        // `run_claude_pilot` rejects a non-active task ("Task is not an active
        // task"), and `failed` is terminal ("Cannot transition from 'failed' to
        // 'in_progress'"), so a re-armament aimed at a dead parent is
        // structurally impossible before it is attempted. Measured twice on two
        // distinct parents and two distinct causes of death: `14465667`
        // (`stuck_pending_no_deferred_wrapper`) and `620ae345`
        // (`phantom_aged_out`, terminal at 02:04:01Z while its three sterile
        // turns ran at 03:35Z / 03:59Z / 04:01Z). Both burned repair budget
        // against a corpse and said nothing.
        //
        // A parent we cannot read is NOT treated as terminal: the exhaustion of
        // the budget is a decision, and taking it on a failed read would spend
        // it on a database hiccup. We fall through to the normal path, which
        // has its own fail-closed guards.
        let parent_status = match self.db.get_task(parent_id).await {
            Ok(Some(parent)) => Some(parent.status),
            // The row is gone: nothing can dispatch for it ever again.
            Ok(None) => Some("absent".to_string()),
            Err(e) => {
                warn!(
                    task_id = %task.id,
                    parent_task_id = %parent_id,
                    error = %e,
                    "failed to read parent before re-arming — proceeding on the normal path"
                );
                None
            }
        };

        if let Some(status) = parent_status.as_deref()
            && TERMINAL_PARENT_STATUSES.contains(&status)
        {
            // Word order is load-bearing: the accented tail
            // `terminal — re-armement impossible` is the exact substring the
            // mika#2169 replay asserts on, so the status is carried BEFORE it
            // rather than wedged inside it. Both halves survive: the reader
            // gets which terminal state, the test gets a stable landmark.
            let reason = format!(
                "aucun dispatch produit ; parent {parent_id} ({status}) \
                 terminal — re-armement impossible"
            );
            self.record_wrapper_noop(&task.id, &reason).await;

            warn!(
                event = "deferred_wrapper_orphaned_by_terminal_parent",
                task_id = %task.id,
                parent_task_id = %parent_id,
                parent_status = %status,
                cause,
                dispatch_class,
                "deferred wrapper consumed against a terminal parent — not re-armed, budget preserved"
            );
            if let Err(e) = self
                .db
                .log_audit_event(
                    &system_session,
                    "deferred_wrapper_orphaned_by_terminal_parent",
                    &format!("task:{}", task.id),
                    None,
                    Some("expired"),
                    Some(&format!(
                        "parent:{parent_id} parent_status:{status} cause:{cause}"
                    )),
                    None,
                )
                .await
            {
                warn!(error = %e, "failed to write deferred_wrapper_orphaned_by_terminal_parent audit event");
            }
            return;
        }

        let outcome = crate::skills::executor::rearm_deferred_callback(
            &self.db,
            parent_id,
            &task.action_config,
            dispatch_class,
            cause,
        )
        .await;

        // L3a (mika#2169) — consume the outcome instead of dropping it.
        //
        // `RearmOutcome` exists precisely to keep `NotNow` and `Unrepairable`
        // apart; this call site used to end in `.await;` and threw both away.
        // On `Unrepairable`, `rearm_deferred_callback` logs "leaving the task
        // for the reaper to expire" — but the reaper it names,
        // `find_orphaned_pending_issue_tasks`, carries `parent.status =
        // 'pending'`. A `blocked` parent falls between the two and nothing ever
        // writes anything about it. So we write it here, where the cause is
        // known, rather than delegating to a sweep that will not come.
        match outcome {
            RearmOutcome::Rearmed => {
                info!(
                    event = "deferred_wrapper_rearmed",
                    task_id = %task.id,
                    parent_task_id = %parent_id,
                    cause,
                    dispatch_class,
                    "deferred wrapper consumed without dispatching — parent re-armed"
                );
                self.record_wrapper_noop(
                    &task.id,
                    &format!("noop: aucun dispatch produit (cause={cause}) — parent ré-armé"),
                )
                .await;
            }
            // Transient by definition — destroy nothing, retry next tick.
            RearmOutcome::NotNow => {
                debug!(
                    task_id = %task.id,
                    parent_task_id = %parent_id,
                    cause,
                    "re-arm refused for a transient reason — retrying next tick"
                );
            }
            RearmOutcome::Unrepairable => {
                let reason = format!(
                    "re-armement différé épuisé après {MAX_STUCK_REARMS} tentatives \
                     (cause={cause}) — aucun dispatch produit"
                );
                self.record_wrapper_noop(
                    &task.id,
                    &format!("noop: aucun dispatch produit (cause={cause}) — budget épuisé"),
                )
                .await;

                match self.db.update_task_failed(parent_id, &reason).await {
                    Ok(true) => {
                        warn!(
                            event = "deferred_dispatch_unrepairable_parent_failed",
                            task_id = %task.id,
                            parent_task_id = %parent_id,
                            cause,
                            dispatch_class,
                            budget = MAX_STUCK_REARMS,
                            "repair budget exhausted — parent failed with a reason instead of staying silent"
                        );
                        if let Err(e) = self
                            .db
                            .log_audit_event(
                                &system_session,
                                "deferred_dispatch_unrepairable_parent_failed",
                                &format!("task:{parent_id}"),
                                None,
                                Some("failed"),
                                Some(&format!("cause:{cause} wrapper:{}", task.id)),
                                None,
                            )
                            .await
                        {
                            warn!(error = %e, "failed to write deferred_dispatch_unrepairable_parent_failed audit event");
                        }
                    }
                    Ok(false) => {
                        debug!(
                            parent_task_id = %parent_id,
                            "parent left its non-terminal state before the budget write — no-op"
                        );
                    }
                    Err(e) => {
                        warn!(
                            parent_task_id = %parent_id,
                            error = %e,
                            "failed to write the exhausted-budget failure on the parent"
                        );
                    }
                }
            }
        }
    }

    /// L1 (mika#2169) — write the wrapper's honest terminal record.
    ///
    /// Fire-and-forget: the record is the point, but failing to write it must
    /// not abort the repair path that produced it.
    async fn record_wrapper_noop(&self, wrapper_id: &str, reason: &str) {
        match self.db.mark_deferred_wrapper_noop(wrapper_id, reason).await {
            Ok(true) => {}
            Ok(false) => {
                debug!(
                    task_id = %wrapper_id,
                    "wrapper was not in a markable state — terminal record skipped"
                );
            }
            Err(e) => {
                warn!(
                    task_id = %wrapper_id,
                    error = %e,
                    "failed to write the wrapper's terminal noop record"
                );
            }
        }
    }

    /// mika#1011 — Promote the next pending deferred-dispatch callback to `completed`.
    ///
    /// Called after a blocking callback completes (mark_task_delivered succeeded),
    /// and by the engine-level periodic backstop (mika#1070).
    /// Promotes the oldest pending deferred callback to `completed` (FIFO).
    /// The task engine's periodic scan picks up `completed` resume_agent tasks
    /// and dispatches them via `dispatch_resume_agent`, which constructs a
    /// `SilentTrigger::DeferredDispatch` turn for tasks with the deferred label.
    pub(crate) async fn dispatch_next_deferred_callback(&self) {
        match self.db.promote_next_deferred_callback().await {
            Ok(Some(promoted_task_id)) => {
                info!(
                    event = "deferred_dispatch_promoted",
                    promoted_task_id = %promoted_task_id,
                    "promoted oldest pending deferred wrapper for engine dispatch"
                );
                // W4: audit event for promotion (#1172)
                if let Err(e) = self
                    .db
                    .log_audit_event(
                        "system",
                        "deferred_dispatch_promoted",
                        &format!("task:{promoted_task_id}"),
                        Some("pending"),
                        Some("completed"),
                        Some("inline promotion after dispatch completion"),
                        None,
                    )
                    .await
                {
                    warn!(error = %e, "failed to write deferred_dispatch_promoted audit event");
                }
            }
            Ok(None) => {} // No pending deferred callbacks
            Err(e) => {
                warn!(error = %e, "failed to promote deferred callback — will retry on next tick");
            }
        }
    }

    /// mika#1175 — Class-scoped sibling of `dispatch_next_deferred_callback`.
    /// Promotes the oldest pending deferred wrapper matching the given
    /// `dispatch_class`. Used by the periodic backstop's per-class iteration.
    pub(crate) async fn dispatch_next_deferred_callback_for_class(&self, dispatch_class: &str) {
        match self
            .db
            .promote_next_deferred_callback_for_class(dispatch_class)
            .await
        {
            Ok(Some(promoted_task_id)) => {
                info!(
                    event = "deferred_dispatch_promoted",
                    promoted_task_id = %promoted_task_id,
                    dispatch_class, "promoted oldest pending deferred wrapper for engine dispatch"
                );
                // W4: audit event for class-scoped promotion (#1172)
                if let Err(e) = self
                    .db
                    .log_audit_event(
                        "system",
                        "deferred_dispatch_promoted",
                        &format!("task:{promoted_task_id}"),
                        Some("pending"),
                        Some("completed"),
                        Some(&format!(
                            "periodic backstop promotion (class: {dispatch_class})"
                        )),
                        None,
                    )
                    .await
                {
                    warn!(error = %e, "failed to write deferred_dispatch_promoted audit event");
                }
            }
            Ok(None) => {} // No pending deferred callbacks in this class
            Err(e) => {
                warn!(
                    error = %e,
                    dispatch_class,
                    "failed to promote deferred callback — will retry on next tick"
                );
            }
        }
    }

    /// #991 — Post-callback advance backstop. After a milestone/project-context
    /// callback turn completes, checks whether the queue was advanced. If not,
    /// fires a `PostCallbackAdvance` trigger to give the agent one more explicit
    /// advance turn. If that also fails, marks the milestone `blocked` automatically.
    ///
    /// Detection logic (DB-state based, not tool-summary based):
    /// - Queue advanced: parent has a new `in_progress`/`pending` callback child,
    ///   or parent is now `blocked`/`completed`/`cancelled`.
    /// - Queue NOT advanced: parent is still `in_progress` with no new active child.
    async fn maybe_fire_post_callback_advance(&self, callback_task: &Task) {
        // Only applies to callbacks with a parent task.
        let parent_id = match &callback_task.parent_task_id {
            Some(id) => id.clone(),
            None => return,
        };

        // Look up parent task — must be manual and milestone/project type.
        let parent = match self.db.get_task_unscoped(&parent_id).await {
            Ok(Some(t)) if t.trigger_type == "manual" => t,
            _ => return,
        };

        if parent.r#type != "milestone" && parent.r#type != "project" {
            return;
        }

        // If parent is already terminal or blocked, the queue was handled.
        if parent.status == "completed"
            || parent.status == "cancelled"
            || parent.status == "blocked"
        {
            return;
        }

        // Check if the callback turn created a new active callback child
        // (indicating advancement to the next issue).
        let has_active_child = match self.db.get_child_tasks(&parent_id).await {
            Ok(children) => children.iter().any(|c| {
                c.trigger_type == "callback"
                    && (c.status == "pending" || c.status == "in_progress")
                    && c.id != callback_task.id
            }),
            Err(e) => {
                warn!(
                    parent_task_id = %parent_id,
                    error = %e,
                    "post_callback_advance: failed to query child tasks, skipping"
                );
                return;
            }
        };

        if has_active_child {
            // Queue was advanced — no backstop needed.
            return;
        }

        // Queue was NOT advanced. Fire PostCallbackAdvance.
        let child_outcome = if callback_task.status == "failed" {
            "failed"
        } else {
            "completed"
        };

        info!(
            parent_task_id = %parent_id,
            callback_task_id = %callback_task.id,
            parent_kind = %parent.r#type,
            child_outcome,
            "post_callback_advance: milestone queue not advanced, firing backstop trigger"
        );

        let advance_trigger = SilentTrigger::PostCallbackAdvance {
            parent_task_id: parent_id.clone(),
            parent_kind: parent.r#type.clone(),
            last_child_outcome: child_outcome.to_string(),
        };

        let trace_id = mika_common::trace::generate_trace_id();
        let session_id = format!("advance-{}", &trace_id[..8]);

        if let Err(e) = self
            .db
            .create_session_with_parent(
                &session_id,
                &callback_task.agent_id,
                "system",
                Some(r#"{"trigger": "post_callback_advance"}"#),
                None,
                None,
            )
            .await
        {
            warn!(
                session_id = %session_id,
                error = %e,
                "post_callback_advance: failed to create session"
            );
        }

        let params = SilentAgentParams {
            db: &self.db,
            tier: self.tier,
            llm: self.llm.as_ref(),
            tools: &self.tools,
            skills: &self.skills,
            trigger: advance_trigger,
            home_dir: &self.home_dir,
            session_id: &session_id,
            message_sender: self.message_sender.clone(),
            embedding_client: self.embedding_client.as_ref(),
            brave_api_key: self.brave_api_key.as_deref(),
            gateway_url: self.gateway_url.as_deref(),
            internal_token: self.internal_token.as_deref(),
            github_token: self.github_token.as_deref(),
            github_app: self.github_app.as_deref(),
            skills_dirty: &self.skills_dirty,
            settings: Some(&self.settings),
            trace_id: Some(trace_id.clone()),
        };

        if let Err(e) = run_silent_agent(&params).await {
            warn!(
                parent_task_id = %parent_id,
                error = %e,
                "post_callback_advance: advance turn failed"
            );

            // Last resort: if the advance turn also failed, mark the milestone
            // blocked automatically so it doesn't sit idle indefinitely.
            let note = format!(
                "auto-blocked: mika-dev failed to advance after callback + advance turn (mika#991). \
                 Original callback: {} (outcome: {})",
                callback_task.id, child_outcome
            );
            if let Err(e) = self.db.update_task_failed(&parent_id, &note).await {
                warn!(
                    parent_task_id = %parent_id,
                    error = %e,
                    "post_callback_advance: failed to auto-block milestone"
                );
            }
        } else {
            // Check again after the advance turn — if still not advanced,
            // auto-block the milestone.
            let still_stuck = match self.db.get_task_unscoped(&parent_id).await {
                Ok(Some(t)) => {
                    t.status == "in_progress"
                        && !matches!(
                            self.db.get_child_tasks(&parent_id).await,
                            Ok(ref children) if children.iter().any(|c| {
                                c.trigger_type == "callback"
                                    && (c.status == "pending" || c.status == "in_progress")
                                    && c.id != callback_task.id
                            })
                        )
                }
                _ => false,
            };

            if still_stuck {
                let note = format!(
                    "auto-blocked: mika-dev failed to advance after callback + advance turn (mika#991). \
                     Original callback: {} (outcome: {})",
                    callback_task.id, child_outcome
                );
                warn!(
                    parent_task_id = %parent_id,
                    "post_callback_advance: advance turn completed but milestone still not advanced, auto-blocking"
                );
                if let Err(e) = self.db.update_task_failed(&parent_id, &note).await {
                    warn!(
                        parent_task_id = %parent_id,
                        error = %e,
                        "post_callback_advance: failed to auto-block milestone"
                    );
                }
            }
        }

        if let Err(e) = self.db.end_session(&session_id).await {
            warn!(
                session_id = %session_id,
                error = %e,
                "post_callback_advance: failed to end session"
            );
        }
    }
}

/// Extract metadata from callback result text and persist to parent task.
///
/// This is a best-effort, fire-and-forget operation. Failures are logged
/// but do not block the callback dispatch. Runs BEFORE the silent agent
/// to guarantee base metadata is captured even if the agent exhausts its
/// step budget.
async fn try_extract_callback_metadata(db: &AsyncDatabase, task: &Task) {
    // 1. Check parent_task_id exists
    let parent_id = match &task.parent_task_id {
        Some(id) => id.clone(),
        None => return,
    };

    // 2. Verify parent is a manual task
    let parent = match db.get_task_unscoped(&parent_id).await {
        Ok(Some(t)) if t.trigger_type == "manual" => t,
        _ => return,
    };

    // 3. Parse result text
    let result = match &task.result {
        Some(r) if !r.is_empty() => r,
        _ => return,
    };

    let extracted = extract_callback_fields(result);
    if extracted.is_null() {
        return;
    }

    // 4. Two-level shallow merge with existing metadata (see issue #489).
    //    Shared helper guarantees identical semantics with the agent-facing
    //    update_task_status tool.
    let merged = match &parent.metadata {
        Some(existing) => {
            if let Ok(mut base) = serde_json::from_str::<serde_json::Value>(existing) {
                crate::task_state::merge_metadata(&mut base, &extracted);
                base
            } else {
                extracted
            }
        }
        None => extracted,
    };

    // 5. Persist
    match db
        .update_task_metadata(&parent_id, &merged.to_string())
        .await
    {
        Ok(true) => info!(
            parent_task_id = %parent_id,
            callback_task_id = %task.id,
            "engine: persisted callback metadata to task"
        ),
        Ok(false) => warn!(
            parent_task_id = %parent_id,
            "engine: parent task not found for metadata write"
        ),
        Err(e) => warn!(
            parent_task_id = %parent_id,
            error = %e,
            "engine: failed to persist callback metadata"
        ),
    }
}

/// Write a human-readable callback summary to `task_messages` keyed to the
/// scope-root task_id (mika#965).
///
/// This closes the narrative gap where `rebuild_context()` merges `task_messages`
/// but no callback outcome was ever written there. The dispatch session's next
/// turn sees the callback result in its rebuilt context.
///
/// Best-effort, fire-and-forget — same pattern as `try_extract_callback_metadata`.
async fn try_write_callback_summary(db: &AsyncDatabase, task: &Task) {
    // 1. Need a parent to resolve scope root from.
    let parent_id = match &task.parent_task_id {
        Some(id) => id.clone(),
        None => return,
    };

    // 2. Parse result text — if empty or no extractable fields, nothing to write.
    let result = match &task.result {
        Some(r) if !r.is_empty() => r,
        _ => return,
    };

    let extracted = extract_callback_fields(result);
    if extracted.is_null() {
        return;
    }

    // 3. Resolve scope root — the task_messages row is keyed to the scope root
    //    so the dispatching session's rebuild_context() picks it up.
    let scope_root_id = match db.resolve_scope_root_task_id(&parent_id).await {
        Ok(Some(id)) => id,
        Ok(None) => {
            // No scope root — parent is not part of a scoped hierarchy.
            // Fall back to parent_id itself so the summary still lands
            // somewhere the dispatch session can read.
            parent_id.clone()
        }
        Err(e) => {
            warn!(
                parent_task_id = %parent_id,
                callback_task_id = %task.id,
                error = %e,
                "callback_summary: failed to resolve scope root"
            );
            return;
        }
    };

    // 4. Build human-readable summary from extracted fields.
    let pilot = &extracted["claude_pilot"];
    let mut parts = Vec::new();

    if let Some(s) = pilot.get("session_id").and_then(|v| v.as_str()) {
        parts.push(format!("session={s}"));
    }
    if let Some(n) = pilot.get("turns").and_then(|v| v.as_u64()) {
        parts.push(format!("turns={n}"));
    }
    if let Some(c) = pilot.get("cost_usd").and_then(|v| v.as_f64()) {
        parts.push(format!("cost=${c:.2}"));
    }
    if let Some(d) = pilot.get("duration_ms").and_then(|v| v.as_u64()) {
        parts.push(format!("duration={d}ms"));
    }
    if let Some(url) = pilot.get("pr_url").and_then(|v| v.as_str()) {
        parts.push(format!("PR: {url}"));
    }

    if parts.is_empty() {
        return;
    }

    let summary = format!("Callback completed: {}", parts.join(", "));

    // 5. Use the callback's session as the session_id for the task_message row.
    let session_id = task.created_by_session.as_deref().unwrap_or("callback");
    let trace_id = task.execution_trace_id.as_deref();
    let metadata_json = extracted.to_string();

    match db
        .insert_task_message(
            &scope_root_id,
            &task.agent_id,
            session_id,
            "system",
            &summary,
            Some(&metadata_json),
            trace_id,
        )
        .await
    {
        Ok(_) => info!(
            scope_root_id = %scope_root_id,
            callback_task_id = %task.id,
            "callback_summary: wrote summary to task_messages"
        ),
        Err(e) => warn!(
            scope_root_id = %scope_root_id,
            callback_task_id = %task.id,
            error = %e,
            "callback_summary: failed to write summary to task_messages"
        ),
    }
}

/// Promote the parent task from `failed` → `completed` when a retry callback
/// succeeds (#958).
///
/// A retry child may deliver a successful result (with `pr_url`) after the
/// orphaned-parent reaper already marked the parent `failed`. This function
/// detects that case and promotes the parent status, keeping the engine's
/// state consistent with the actual outcome.
///
/// Best-effort, fire-and-forget — same pattern as `try_extract_callback_metadata`.
async fn try_promote_parent_on_retry_success(db: &AsyncDatabase, task: &Task) {
    // 1. Need a parent to promote.
    let parent_id = match &task.parent_task_id {
        Some(id) => id.clone(),
        None => return,
    };

    // 2. Read the parent — only promote self_dev manual tasks in `failed` state.
    //    The source='self_dev' guard mirrors the reaper's scope (#958 review).
    let parent = match db.get_task_unscoped(&parent_id).await {
        Ok(Some(t))
            if t.trigger_type == "manual"
                && t.status == "failed"
                && t.source.as_deref() == Some("self_dev") =>
        {
            t
        }
        _ => return,
    };

    // 3. Check whether the callback result contains a pr_url (success indicator).
    let result = match &task.result {
        Some(r) if !r.is_empty() => r,
        _ => return,
    };

    let extracted = extract_callback_fields(result);
    let pr_url = extracted
        .get("claude_pilot")
        .and_then(|cp| cp.get("pr_url"))
        .and_then(|v| v.as_str());

    let pr_url = match pr_url {
        Some(url) if !url.is_empty() => url.to_string(),
        _ => return,
    };

    // 4. Promote: failed → completed.
    let reason = format!("retry_success (pr_url: {pr_url})");
    match db.promote_task_completed(&parent_id, &reason).await {
        Ok(true) => {
            // Emit audit event for traceability.
            let system_session = format!("system-{}", parent.agent_id);
            let trace_id = mika_common::trace::generate_trace_id();
            if let Err(e) = db
                .log_audit_event(
                    &system_session,
                    "task_engine_retry_promoter",
                    &parent_id,
                    Some("failed"),
                    Some("completed"),
                    Some(&reason),
                    Some(&trace_id),
                )
                .await
            {
                warn!(
                    parent_task_id = %parent_id,
                    error = %e,
                    "engine: failed to write retry-promoter audit event"
                );
            }
            info!(
                parent_task_id = %parent_id,
                callback_task_id = %task.id,
                pr_url = %pr_url,
                "engine: promoted parent task from failed to completed (retry success)"
            );
        }
        Ok(false) => {
            // Parent already left `failed` state (concurrent action) — skip.
            debug!(
                parent_task_id = %parent_id,
                "engine: parent no longer in failed state, skipping retry promotion"
            );
        }
        Err(e) => {
            warn!(
                parent_task_id = %parent_id,
                error = %e,
                "engine: failed to promote parent task on retry success"
            );
        }
    }
}

/// Auto-complete the parent task when a callback delivers with a `pr_url`
/// success indicator (mika#1162).
///
/// Structural backstop sibling to `try_promote_parent_on_retry_success`
/// (mika#958): that function handles `failed → completed` AFTER the reaper
/// has fired; this one handles `in_progress → completed` for the direct
/// success case where the silent agent turn fails to call `update_task_status`
/// (timeout, max-steps continuation, transport error, etc.).
///
/// Together with the reaper (`in_progress → failed` when no pr_url), the
/// engine covers every (callback outcome × parent state) combination.
///
/// Best-effort, fire-and-forget — same shape as the sibling helpers.
async fn try_complete_parent_on_callback_success(db: &AsyncDatabase, task: &Task) {
    // 1. Need a parent to complete.
    let parent_id = match &task.parent_task_id {
        Some(id) => id.clone(),
        None => return,
    };

    // 2. Read the parent — only complete self_dev manual tasks currently in
    //    `in_progress`. Mirror the reaper's scope; the retry-promoter owns
    //    the `failed` precondition.
    let parent = match db.get_task_unscoped(&parent_id).await {
        Ok(Some(t))
            if t.trigger_type == "manual"
                && t.status == "in_progress"
                && t.source.as_deref() == Some("self_dev") =>
        {
            t
        }
        _ => return,
    };

    // 3. Defense-in-depth: only auto-complete implement-class dispatches.
    //    Groom-class callbacks don't emit `PR:` lines, so step 4 would
    //    short-circuit anyway, but the filter mirrors mika#1118's reaper
    //    invariant for symmetric drift-protection.
    if task.dispatch_class.as_deref().unwrap_or("implement") != "implement" {
        return;
    }

    // 4. Check whether the callback result contains a pr_url (success indicator).
    let result = match &task.result {
        Some(r) if !r.is_empty() => r,
        _ => return,
    };

    let extracted = extract_callback_fields(result);
    let pr_url = extracted
        .get("claude_pilot")
        .and_then(|cp| cp.get("pr_url"))
        .and_then(|v| v.as_str());

    let pr_url = match pr_url {
        Some(url) if !url.is_empty() => url.to_string(),
        _ => return,
    };

    // 5. Transition: in_progress → completed via the guarded
    //    `update_task_completed` (WHERE status IN ('pending', 'in_progress')).
    //    Concurrent operator cancels or agent updates lose the race cleanly.
    let reason = format!("parent_completed_from_callback (pr_url: {pr_url})");
    match db.update_task_completed(&parent_id, Some(&reason)).await {
        Ok(true) => {
            // Emit audit event for traceability — same tool_name as the
            // periodic backstop in engine.rs so consumers can grep both
            // call sites with a single query.
            let system_session = format!("system-{}", parent.agent_id);
            let trace_id = mika_common::trace::generate_trace_id();
            if let Err(e) = db
                .log_audit_event(
                    &system_session,
                    "task_engine_parent_completer",
                    &parent_id,
                    Some("in_progress"),
                    Some("completed"),
                    Some(&reason),
                    Some(&trace_id),
                )
                .await
            {
                warn!(
                    parent_task_id = %parent_id,
                    error = %e,
                    "engine: failed to write parent-completer audit event"
                );
            }
            info!(
                parent_task_id = %parent_id,
                callback_task_id = %task.id,
                pr_url = %pr_url,
                "engine: auto-completed parent task from callback success (mika#1162)"
            );
        }
        Ok(false) => {
            // Parent already left `in_progress` (concurrent operator cancel,
            // agent update, or sibling completer) — skip.
            debug!(
                parent_task_id = %parent_id,
                "engine: parent no longer in_progress, skipping auto-completion"
            );
        }
        Err(e) => {
            warn!(
                parent_task_id = %parent_id,
                error = %e,
                "engine: failed to auto-complete parent task on callback success"
            );
        }
    }
}

/// The refusal reason carried by a `dispatch-lib` auto-skip callback, when the
/// callback result is one (mika#2158 M6a).
///
/// `dispatch-lib` refuses a foreseeable condition with `_deliver_callback` + `exit 0`,
/// shipping a JSON body of the shape `{"status":"auto_skipped","reason":"…", …}` as the
/// callback text (`mika#988` exit semantics). Tolerant of surrounding prose, because the
/// callback transport is a message body and nothing structurally forbids a prologue:
/// strict parse first, then the shared brace-matching extractor (mika#876), which is
/// string-literal- and escape-aware — a naive first-`{`..last-`}` slice would be defeated
/// by a brace in the refusal's own `note` field.
fn parse_auto_skip_reason(result: &str) -> Option<String> {
    fn reason_of(v: &serde_json::Value) -> Option<String> {
        if v.get("status")?.as_str()? != "auto_skipped" {
            return None;
        }
        Some(
            v.get("reason")
                .and_then(|r| r.as_str())
                .unwrap_or("unspecified")
                .to_string(),
        )
    }

    let trimmed = result.trim();
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(trimmed)
        && let Some(r) = reason_of(&v)
    {
        return Some(r);
    }
    let candidate = crate::kg::subject_extractor::extract_first_json_object(trimmed)?;
    let v = serde_json::from_str::<serde_json::Value>(candidate).ok()?;
    reason_of(&v)
}

/// A dispatch refusal must produce an effect (mika#2158 AC8).
///
/// # Le défaut que ceci ferme
///
/// Quand `dispatch-lib` refuse un dispatch — canoniquement `already_groomed` : le plan du
/// ticket est déjà committé sur la branche, le re-groomer bouclerait — le refus arrivait au
/// moteur et n'y produisait rien. La ligne `tasks` pré-créée par `ready_label_handler`
/// restait `in_progress` jusqu'à ce que le balayage phantom (mika#1712) la passe `failed`
/// une heure plus tard. Mesuré sur `~/.mika/data/mika.db` le 2026-09-03 : 31 tâches
/// `failed` sur #1772, 8 sur #2127, 7 sur #2108, toutes en `phantom_aged_out`.
///
/// Le coût n'était pas seulement une ligne morte. Pendant les ~60 min du fantôme, le ticket
/// comptait comme `in_flight`, et le reconciliateur Phase 2 de l'auto-pull **remettait son
/// budget de re-drive à zéro** à chaque tick. Le garde-fou censé borner la boucle était
/// effacé par la boucle : après 31 re-drives sur #1772, `redrive_count` disait 1. C'est la
/// moitié M6b (`classify_stuck_ready`) qui ferme cette seconde moitié.
///
/// # Pourquoi `completed` et non `failed`
///
/// Le refus est un **résultat correct** : le ticket EST groomé. Le marquer `failed`
/// dirait que quelque chose s'est mal passé et brouillerait le compte des vraies pannes.
///
/// # Pourquoi tous les `auto_skipped`, et pas seulement `already_groomed`
///
/// L'AC8 nomme `already_groomed` parce que c'est la forme mesurée. Le mécanisme du fantôme
/// est identique pour `issue_closed` (mika#988) et pour toute refus futur : la raison
/// exacte est reportée verbatim dans le `result` et dans l'événement d'audit, donc les
/// populations restent séparables. Filtrer sur la seule raison mesurée laisserait le même
/// défaut ouvert sous un autre nom.
///
/// # Attribution (M6a)
///
/// L'événement d'audit porte un `tool_name` qui n'est écrit qu'ici. Avant, une ligne
/// `phantom_aged_out` ne disait pas si le pilote n'avait jamais tourné ou si `dispatch-lib`
/// avait refusé sans que le refus atteigne le moteur ; les deux hypothèses étaient
/// compatibles avec ce qu'on pouvait lire. Désormais la seconde laisse une trace nommée, et
/// son absence sous un fantôme est une information.
///
/// Best-effort, fire-and-forget — même forme que les helpers frères.
async fn try_resolve_parent_on_dispatch_refusal(db: &AsyncDatabase, task: &Task) {
    let Some(parent_id) = task.parent_task_id.clone() else {
        return;
    };

    let Some(result) = task.result.as_deref().filter(|r| !r.is_empty()) else {
        return;
    };
    let Some(reason) = parse_auto_skip_reason(result) else {
        return;
    };

    // Portée identique aux frères : une tâche de suivi self_dev encore en vol. Pas de
    // filtre sur `dispatch_class` — la classe est déductible de la raison, et un second
    // prédicat plus faible pour le même fait est exactement ce que mika#2158 ferme.
    let parent = match db.get_task_unscoped(&parent_id).await {
        Ok(Some(t))
            if t.trigger_type == "manual"
                && t.status == "in_progress"
                && t.source.as_deref() == Some("self_dev") =>
        {
            t
        }
        _ => return,
    };

    let detail = format!("parent_resolved_from_dispatch_refusal (reason: {reason})");
    match db.update_task_completed(&parent_id, Some(&detail)).await {
        Ok(true) => {
            let system_session = format!("system-{}", parent.agent_id);
            let trace_id = mika_common::trace::generate_trace_id();
            if let Err(e) = db
                .log_audit_event(
                    &system_session,
                    "dispatch_refusal_resolver",
                    &parent_id,
                    Some("in_progress"),
                    Some("completed"),
                    Some(&detail),
                    Some(&trace_id),
                )
                .await
            {
                warn!(
                    parent_task_id = %parent_id,
                    error = %e,
                    "engine: failed to write dispatch-refusal-resolver audit event"
                );
            }
            info!(
                event = "dispatch_refusal_resolved",
                parent_task_id = %parent_id,
                callback_task_id = %task.id,
                reason = %reason,
                "engine: dispatch refusal resolved its own tracking row (mika#2158)"
            );
        }
        Ok(false) => {
            debug!(
                parent_task_id = %parent_id,
                reason = %reason,
                "engine: parent no longer in_progress, skipping dispatch-refusal resolution"
            );
        }
        Err(e) => {
            warn!(
                parent_task_id = %parent_id,
                error = %e,
                "engine: failed to resolve parent task on dispatch refusal"
            );
        }
    }
}

/// Returns `true` if the task is a team-child callback (per-delegation
/// `resume_agent` created by the team engine). Both `team_run_id` and
/// `parent_task_id` are set together at child-creation time in
/// `engine.rs:874-912`.
fn is_team_child_callback(task: &Task) -> bool {
    task.team_run_id.is_some() && task.parent_task_id.is_some()
}

/// mika#1289 — When a dev-groom callback delivers with `Outcome: PLAN_GROOMED`
/// in its result text, re-add the `ready` label on the GitHub issue so the
/// ready-label webhook handler dispatches dev-pilot. This is the structural
/// counterpart to the LLM-mediated prompt-level path in `self-dev-callback`
/// (mika#996 / PR #1291) which has a documented drift rate: the LLM
/// sometimes sets `update_task_status(completed)` but skips the
/// `gh issue edit --add-label ready` step, leaving the queue wedged.
///
/// Engine-side dispatch is fire-and-forget: failure to re-add the label is
/// logged at WARN and does not affect the callback delivery. The prompt-
/// level handler remains as defense-in-depth (idempotent — re-adding an
/// already-present label is a no-op on GitHub).
///
/// Trigger conditions (all must hold):
///   - `task.dispatch_class == Some("groom")` — groom-class callbacks only
///   - `task.result` contains the literal `"Outcome: PLAN_GROOMED"` —
///     the structural success marker emitted by `_write_canonical_callout`'s
///     parent `_iterate_groom_loop` (does not depend on body-marker write
///     having succeeded; the callback-result text is canonical for this signal)
///   - parent task has a parseable `reference_url` of shape
///     `https://github.com/<owner>/<repo>/issues/<n>`
///
/// Dispatch is **engine-side and direct** (mika#1614): the function resolves the
/// `run_claude_pilot` tool from the `SkillRegistry`, **reuses the groom parent
/// task** by flipping its `dispatch_class` groom→implement (mika#996 task-reuse
/// pattern — NOT a new parent, which would collide with the groom parent on the
/// `reference_url` unique index while it is still `in_progress`), validates
/// dispatch readiness, and spawns the claude-pilot subprocess via
/// `spawn_long_running_exec` — mirroring `ready_label_handler`'s steps 9a–9i
/// (mika#1572). This replaces the prior `gh issue edit --add-label ready` →
/// webhook round-trip → LLM-mediated turn chain, which was vulnerable to LLM
/// fabrication (an "auto-fired dispatch" log line with no actual subprocess
/// spawn — observed on mika#1609, 2026-06-28).
///
/// Fire-and-forget: on ANY precondition failure (tool not in registry, readiness
/// rejection, handler missing) the function logs a WARN and returns. The
/// prompt-level path in `self-dev-callback` remains as defense-in-depth.
///
/// Audit event written under `tool_name='task_engine_groom_pilot_dispatcher'`
/// with `after_value='implement_dispatched'` for traceability.
async fn try_dispatch_pilot_after_groom_success(
    db: &AsyncDatabase,
    task: &Task,
    github_token: Option<&str>,
    skills: &SkillRegistry,
) {
    // 1. Groom-class callbacks only.
    if task.dispatch_class.as_deref() != Some("groom") {
        return;
    }

    // 2. Canonical success marker in callback result text.
    match &task.result {
        Some(r) if r.contains("Outcome: PLAN_GROOMED") => {}
        _ => return,
    };

    // 3. Parent task with parseable issue URL.
    let parent_id = match &task.parent_task_id {
        Some(id) => id.clone(),
        None => return,
    };
    let parent = match db.get_task_unscoped(&parent_id).await {
        Ok(Some(t)) => t,
        _ => return,
    };
    let reference_url = match parent.reference_url.as_deref() {
        Some(url) => url.to_string(),
        None => return,
    };
    let (repo, issue_num) = match parse_repo_issue_from_url(&reference_url) {
        Some(parsed) => parsed,
        None => return,
    };

    // 4. GitHub token required (the grooming-marker readiness check fetches the
    //    issue body; without a token it fails-open, but the dispatch is
    //    meaningless without GitHub access — keep the original precondition).
    if github_token.is_none_or(str::is_empty) {
        warn!(
            groom_parent_task_id = %parent_id,
            callback_task_id = %task.id,
            "engine: groom-pilot auto-fire skipped — no GitHub token configured"
        );
        return;
    }

    // 5. Engine-side direct dispatch (mika#1614, mirrors ready_label_handler
    //    steps 9a–9i / mika#1572). Grooming just succeeded, so the follow-up is
    //    always the implement-class dev-pilot run.
    //
    //    CRITICAL — task REUSE, not a new parent (mika#996 pattern). The groom
    //    parent is still `in_progress` here (groom callbacks carry no `pr_url`,
    //    so the earlier `try_complete_parent_on_callback_success` is a no-op for
    //    them). Creating a *separate* implement parent with the same
    //    `reference_url` would collide with the partial-unique index
    //    `idx_tasks_manual_active_ref_url` (active manual tasks, per agent+url) —
    //    `create_task` would error and the dispatch would silently never fire,
    //    re-introducing the very bug this fixes. Instead we flip the groom
    //    parent's `dispatch_class` groom→implement and reuse it as the implement
    //    parent (one task identity per issue, no leak — it completes normally
    //    when the pilot delivers a `pr_url`).
    let system_session = format!("system-{}", parent.agent_id);
    let trace_id = mika_common::trace::generate_trace_id();

    // 5a. Resolve the dev-pilot dispatch tool from the agent's SkillRegistry.
    let skill_tool = match skills.resolve_tool_by_name("run_claude_pilot") {
        Some(t) => t,
        None => {
            warn!(
                parent_task_id = %parent_id,
                callback_task_id = %task.id,
                "engine: groom-pilot auto-fire skipped — run_claude_pilot not in \
                 SkillRegistry (prompt-level path remains as fallback)"
            );
            return;
        }
    };

    // 5b. Resolve the long-running exec command + estimated duration from the
    //     tool handler (binds the callback timeout to the same value the LLM
    //     tool-call path uses; dev-pilot declares estimated_duration_secs).
    let (command, estimated_duration_secs) = match &skill_tool.handler {
        ToolHandler::Exec {
            command,
            long_running: true,
            estimated_duration_secs,
        } => (command.clone(), *estimated_duration_secs),
        _ => {
            warn!(
                parent_task_id = %parent_id,
                callback_task_id = %task.id,
                "engine: groom-pilot auto-fire skipped — run_claude_pilot is not a \
                 long-running exec handler"
            );
            return;
        }
    };

    // 5c. Flip the groom parent's dispatch_class groom→implement BEFORE the
    //     readiness check, so the per-class slot guard (#1001) scopes to the
    //     `implement` slot (not the now-vacated `groom` slot). On failure, abort
    //     before any dispatch state is created.
    match db.update_task_dispatch_class(&parent_id, "implement").await {
        Ok(true) => {}
        Ok(false) => {
            warn!(
                parent_task_id = %parent_id,
                "engine: groom-pilot auto-fire skipped — groom parent not found for \
                 dispatch_class flip"
            );
            return;
        }
        Err(e) => {
            warn!(
                parent_task_id = %parent_id,
                error = %e,
                "engine: groom-pilot auto-fire failed — could not flip dispatch_class"
            );
            return;
        }
    }

    // 5d. The dispatch input the LLM would have provided. `prompt` is the bare
    //     `<repo>#<num>` reference (dispatch-lib's worktree-setup parser requires
    //     the bare form, mika#1593). `task_id` is the reused (now implement)
    //     parent.
    let dispatch_input = serde_json::json!({
        "skill": "dev-pilot",
        "prompt": format!("{repo}#{issue_num}"),
        "task_id": parent_id,
    });

    // 5e. Re-use the dispatch-readiness gate (slot availability, grooming
    //     markers, blockedBy). `originating_message = None` — this is an engine
    //     callback, not a webhook fallthrough event, so guard (0) is a no-op. On
    //     rejection (e.g. slot occupied), the rejection JSON is written to the
    //     task's `result` by `validate_dispatch_readiness` itself (operator-
    //     visible); the prompt-level path remains as defense-in-depth.
    if let Err(rejection) = crate::skills::executor::validate_dispatch_readiness(
        db,
        &parent_id,
        github_token,
        Some(&dispatch_input),
        None,
    )
    .await
    {
        warn!(
            parent_task_id = %parent_id,
            rejection = %rejection,
            "engine: groom-pilot auto-fire skipped — dispatch readiness check failed"
        );
        return;
    }

    // 5f. Create the callback child task (same shape + timeout as the LLM
    //     tool-call path via the shared `build_callback_task` helper).
    let timeout_secs = (estimated_duration_secs.unwrap_or(3600) * 3).clamp(600, 7_776_000);
    let callback_task = crate::skills::executor::build_callback_task(
        parent.agent_id.clone(),
        Some(parent_id.clone()),
        "run_claude_pilot",
        &dispatch_input,
        timeout_secs,
        &system_session,
        &trace_id,
    );
    let callback_task_id = match db.create_task(callback_task).await {
        Ok(id) => id,
        Err(e) => {
            warn!(
                parent_task_id = %parent_id,
                error = %e,
                "engine: groom-pilot auto-fire failed — could not create callback child"
            );
            return;
        }
    };

    // 5g. Verify the handler script exists before committing to the spawn. If
    //     missing, mark the callback child failed so it does not dangle.
    let cmd_path = skill_tool.skill_dir.join(&command);
    if !cmd_path.exists() {
        warn!(
            parent_task_id = %parent_id,
            callback_task_id = %callback_task_id,
            cmd_path = %cmd_path.display(),
            "engine: groom-pilot auto-fire failed — dispatch handler script not found"
        );
        let _ = db
            .update_task_failed(
                &callback_task_id,
                &format!("handler not found: {}", cmd_path.display()),
            )
            .await;
        return;
    }

    // 5h. The reused parent is already `in_progress` (it was the groom parent),
    //     so no status transition is needed — execute_long_running's #525
    //     pending→in_progress transition does not apply to task reuse.

    // 5i. Inject subprocess metadata, mirroring execute_long_running:
    //     `__mika_task_id` is the callback child (delivery target), `__mika_agent`
    //     is this agent.
    let mut enriched_input = dispatch_input.clone();
    if let serde_json::Value::Object(ref mut map) = enriched_input {
        map.insert(
            "__mika_task_id".to_string(),
            serde_json::Value::String(callback_task_id.clone()),
        );
        map.insert(
            "__mika_agent".to_string(),
            serde_json::Value::String(parent.agent_id.clone()),
        );
    }

    // 5j. Spawn the detached subprocess. No webhook round-trip, no LLM turn.
    crate::skills::executor::spawn_long_running_exec(
        cmd_path,
        skill_tool.skill_dir.clone(),
        enriched_input,
        callback_task_id.clone(),
        db.clone(),
        github_token.map(|s| s.to_string()),
    );

    info!(
        parent_task_id = %parent_id,
        callback_task_id = %callback_task_id,
        repo = %repo,
        issue = issue_num,
        "engine: auto-fired dev-pilot dispatch after groom success \
         (mika#1614 — engine-side direct spawn, task reuse, no webhook round-trip)"
    );

    let reason = format!("groom_pilot_dispatch_fired (issue: {repo}#{issue_num})");
    if let Err(e) = db
        .log_audit_event(
            &system_session,
            "task_engine_groom_pilot_dispatcher",
            &parent_id,
            Some("groom_delivered"),
            Some("implement_dispatched"),
            Some(&reason),
            Some(&trace_id),
        )
        .await
    {
        warn!(
            parent_task_id = %parent_id,
            error = %e,
            "engine: failed to write groom-pilot-dispatcher audit event"
        );
    }
}

/// Parse `senara-solutions/<repo>/issues/<n>` from a reference URL.
/// Returns `(repo, issue_number)` on match, `None` otherwise.
fn parse_repo_issue_from_url(url: &str) -> Option<(String, u64)> {
    let after_org = url.split("senara-solutions/").nth(1)?;
    let mut parts = after_org.split('/');
    let repo = parts.next()?.to_string();
    let kind = parts.next()?;
    if kind != "issues" {
        return None;
    }
    let n_str = parts.next()?;
    // Strip any query string from the number.
    let n_str = n_str.split('?').next().unwrap_or(n_str);
    let n: u64 = n_str.parse().ok()?;
    Some((repo, n))
}

/// Parse structured fields from callback result text.
///
/// Expected format (lines from claude-pilot `run.sh`):
/// ```text
/// claude-pilot completed (status: done).
/// Session: <session_id>
/// Turns: <N>
/// Cost: $<amount>
/// Duration: <N>ms
/// ```
///
/// Fields default to `"unknown"` in the handler when not available;
/// this parser skips `"unknown"` values to avoid polluting metadata.
fn extract_callback_fields(result: &str) -> serde_json::Value {
    static RE_SESSION: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"Session:\s*(\S+)").unwrap());
    static RE_TURNS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"Turns:\s*(\d+)").unwrap());
    static RE_COST: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"Cost:\s*\$([0-9]+(?:\.[0-9]+)?)").unwrap());
    static RE_DURATION: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"Duration:\s*(\d+)ms").unwrap());
    // PR URL emitted by dev-pilot/handlers/run.sh:398 — `PR: <url>`.
    // Anchored at line start (multiline) so free-text mentions don't match.
    // See mika#871 R4 for the integration contract.
    static RE_PR_URL: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?m)^PR:\s+(https?://github\.com/\S+)").unwrap());

    let mut map = serde_json::Map::new();

    if let Some(cap) = RE_SESSION.captures(result) {
        let val = &cap[1];
        if val != "unknown" {
            map.insert("session_id".into(), serde_json::Value::String(val.into()));
        }
    }
    if let Some(cap) = RE_TURNS.captures(result)
        && let Ok(n) = cap[1].parse::<u64>()
    {
        map.insert("turns".into(), serde_json::Value::Number(n.into()));
    }
    if let Some(cap) = RE_COST.captures(result) {
        // Parse as f64 and store as JSON number (consistent with turns/duration_ms).
        // from_f64 returns None for NaN/Infinity — impossible from the regex but handled defensively.
        if let Some(n) = cap[1]
            .parse::<f64>()
            .ok()
            .and_then(serde_json::Number::from_f64)
        {
            map.insert("cost_usd".into(), serde_json::Value::Number(n));
        }
    }
    if let Some(cap) = RE_DURATION.captures(result)
        && let Ok(n) = cap[1].parse::<u64>()
    {
        map.insert("duration_ms".into(), serde_json::Value::Number(n.into()));
    }
    if let Some(cap) = RE_PR_URL.captures(result) {
        map.insert("pr_url".into(), serde_json::Value::String(cap[1].into()));
    }

    if map.is_empty() {
        serde_json::Value::Null
    } else {
        // Nest under "claude_pilot" key to match self-dev prompt schema
        serde_json::json!({ "claude_pilot": map })
    }
}

/// Parse an anchored `NO_PR: <reason>` line from a callback result (mika#2121 U2).
///
/// Mirror of `RE_PR_URL` above: `(?m)^NO_PR:` is line-anchored so a free-text
/// mention never matches, and the reason is constrained to the lowercase-token
/// charset the three `dispatch-lib.sh` emission sites produce (`no_pr_on_branch`,
/// `gh_query_failed`, `branch_unset`, `repo_unset`, `rescue_pr_create_failed` —
/// KTD2). Returns `None` when no `NO_PR:` line is present, which is the AC-G2
/// contract: a callback carrying neither `PR:` nor `NO_PR:` (a producer that
/// predates U1) leaves the generic `callback_delivered_without_pr_url` motif
/// intact. The reaper (`engine::reap_orphaned_parent_tasks`) calls this to
/// decide between the generic motif and `callback_no_pr_<reason>`.
pub(crate) fn parse_no_pr_reason(result: &str) -> Option<String> {
    static RE_NO_PR: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?m)^NO_PR:\s+([a-z_]+)").unwrap());
    RE_NO_PR.captures(result).map(|cap| cap[1].to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::async_db::AsyncDatabase;
    use crate::db::{Database, NewTask};
    use crate::messaging::{MessageSender, SendOutcome};
    use std::path::PathBuf;
    use std::sync::atomic::AtomicBool;

    struct NoopSender;
    #[async_trait::async_trait]
    impl MessageSender for NoopSender {
        async fn send(&self, _text: &str) -> anyhow::Result<SendOutcome> {
            Ok(SendOutcome::Delivered)
        }
    }

    // ---------------------------------------------------------------------
    // mika#2205 — les deux scans périodiques résolvent PAT-first / App-fallback
    // ---------------------------------------------------------------------

    /// `Settings` sans PAT — la forme exacte du 2026-09-05 après ~16:20.
    fn settings_without_pat() -> Settings {
        let mut s = Settings::test_defaults();
        s.github_token = None;
        s
    }

    /// `Settings` avec un PAT — l'identité machine d'ADR-008.
    fn settings_with_pat(pat: &str) -> Settings {
        let mut s = Settings::test_defaults();
        s.github_token = Some(secrecy::SecretString::from(pat.to_string()));
        s
    }

    /// AC5 — le cœur du correctif : PAT absent, App saine ⇒ le scan obtient un
    /// token et tourne. Avant mika#2205 cette résolution rendait `None` et les
    /// deux scans mouraient à la même seconde que la disparition du PAT.
    #[tokio::test]
    async fn mika2205_periodic_scans_fall_back_to_github_app_without_pat() {
        let app = mika_common::github_app::GitHubApp::new_with_test_token("ghs_app_token").await;
        let settings = settings_without_pat();

        for scan in [PeriodicScan::AutoPull, PeriodicScan::WipRescue] {
            let token =
                resolve_periodic_scan_token(&settings, Some(app.as_ref()), "task-2205", scan).await;
            assert_eq!(
                token.as_deref(),
                Some("ghs_app_token"),
                "{scan:?} doit emprunter le repli App quand le PAT est absent"
            );
        }
    }

    /// ADR-008 préservé : quand le PAT existe, c'est lui — l'App reste un repli,
    /// jamais une substitution. Un renversement ici casserait l'identité machine
    /// distincte sur laquelle reposent revue et merge de PR.
    #[tokio::test]
    async fn mika2205_pat_still_wins_over_the_app_fallback() {
        let app = mika_common::github_app::GitHubApp::new_with_test_token("ghs_app_token").await;
        let settings = settings_with_pat("ghp_machine_user");

        for scan in [PeriodicScan::AutoPull, PeriodicScan::WipRescue] {
            let token =
                resolve_periodic_scan_token(&settings, Some(app.as_ref()), "task-2205", scan).await;
            assert_eq!(
                token.as_deref(),
                Some("ghp_machine_user"),
                "{scan:?} doit préférer le PAT — l'App n'est qu'un repli (ADR-008)"
            );
        }
    }

    /// AC3 — fail-safe conservé : ni PAT ni App ⇒ pas de token, le scan saute.
    /// Ce qui change par rapport à avant mika#2205 n'est pas l'issue mais le
    /// niveau de log (DEBUG → WARN), non observable depuis un test unitaire.
    #[tokio::test]
    async fn mika2205_no_pat_and_no_app_still_skips() {
        let settings = settings_without_pat();

        for scan in [PeriodicScan::AutoPull, PeriodicScan::WipRescue] {
            let token = resolve_periodic_scan_token(&settings, None, "task-2205", scan).await;
            assert!(
                token.is_none(),
                "{scan:?} doit sauter quand ni PAT ni App ne résolvent"
            );
        }
    }

    /// Les deux WARN portent des noms d'événement distincts : un opérateur qui
    /// grep `wip_rescue_no_token` ne doit pas récolter les ticks d'`auto_pull`.
    #[test]
    fn mika2205_scan_warn_events_are_distinct() {
        assert_ne!(
            PeriodicScan::AutoPull.no_token_event(),
            PeriodicScan::WipRescue.no_token_event()
        );
    }

    /// AC1 + AC2, garde structurelle : ni `dispatch_auto_pull_groomed` ni
    /// `dispatch_wip_rescue` ne doivent relire `self.github_token`.
    ///
    /// Les quatre tests ci-dessus prouvent que le **résolveur** fait le bon
    /// choix ; aucun ne prouve que les deux scans l'appellent. Le défaut de
    /// mika#2205 n'était pas une mauvaise résolution, c'était un appelant qui ne
    /// résolvait pas — exactement ce qu'un test du résolveur seul ne voit pas.
    /// D'où cette garde sur le corps des deux fonctions, dans la forme déjà
    /// employée par `grooming_marker::tests::no_grooming_regex_outside_this_module`.
    ///
    /// Hors périmètre volontaire : le troisième site PAT-seul du fichier, l'appel
    /// à `try_dispatch_pilot_after_groom_success` (auto-fire après grooming), qui
    /// passe encore `self.github_token.as_deref()`. Ce chemin mène à une création
    /// de PR, où l'identité lue par GitHub compte au sens d'ADR-008 ; l'y basculer
    /// est une décision d'identité distincte, pas un corollaire de ce ticket.
    #[test]
    fn mika2205_periodic_scans_do_not_read_the_pat_field_directly() {
        let src = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/task_engine/dispatcher.rs"),
        )
        .expect("la garde doit pouvoir lire dispatcher.rs");

        for fn_name in ["dispatch_auto_pull_groomed", "dispatch_wip_rescue"] {
            let sig = format!("async fn {fn_name}(");
            let start = src
                .find(&sig)
                .unwrap_or_else(|| panic!("{fn_name} doit exister dans dispatcher.rs"));
            // Le corps court jusqu'à la prochaine méthode de l'impl (indentation 4).
            let rest = &src[start + sig.len()..];
            let end = rest.find("\n    async fn ").unwrap_or(rest.len());
            let body = &rest[..end];

            assert!(
                !body.contains("self.github_token"),
                "{fn_name} lit encore self.github_token (PAT seul) — mika#2205 exige \
                 resolve_periodic_scan_token, sinon un PAT absent tue le scan alors \
                 que l'App est saine"
            );
            assert!(
                body.contains("resolve_periodic_scan_token"),
                "{fn_name} doit résoudre son token via resolve_periodic_scan_token"
            );
        }
    }

    fn test_db() -> AsyncDatabase {
        let db = Database::open_in_memory().unwrap();
        AsyncDatabase::new_with_agent(db, "mika")
    }

    fn test_dispatcher(db: AsyncDatabase) -> TaskDispatcher {
        let tmp = tempfile::tempdir().unwrap();
        let settings = Settings::load(tmp.path()).unwrap();
        TaskDispatcher {
            db,
            tier: mika_common::home::AgentTier::Default,
            llm: mika_common::llm::dummy_provider(),
            tools: Arc::new(crate::tools::default_tools()),
            skills: Arc::new(crate::skills::SkillRegistry::empty()),
            message_sender: Some(Arc::new(NoopSender)),
            home_dir: PathBuf::from("/tmp"),
            embedding_client: None,
            brave_api_key: None,
            github_token: None,
            gateway_url: None,
            internal_token: None,
            github_app: None,
            skills_dirty: Arc::new(AtomicBool::new(false)),
            agent_lock: None,
            cli_mode: false,
            settings,
            pr_reviews_posted: None,
        }
    }

    #[tokio::test]
    async fn test_dispatch_send_message_missing_text_returns_error() {
        let db = test_db();
        let dispatcher = test_dispatcher(db.clone());

        let task = NewTask {
            agent_id: "mika".to_string(),
            team_run_id: None,
            parent_task_id: None,
            depth: 0,
            label: "test".to_string(),
            trigger_type: "time".to_string(),
            cron_expr: None,
            event_source: None,
            event_offset_secs: None,
            condition_expr: None,
            next_fire_at: Some(crate::timestamp::now()),
            timeout_at: None,
            action_type: "send_message".to_string(),
            action_config: "{}".to_string(), // missing "text" key
            input_context: None,
            created_by_session: None,
            created_trace_id: None,
            reference_url: None,
            source: None,
            metadata: None,
            r#type: None,
            dispatch_class: None,
        };
        let id = db.create_task(task).await.unwrap();
        let result = dispatcher.dispatch(&id).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("missing 'text'"));
    }

    #[tokio::test]
    async fn test_dispatch_send_message_succeeds() {
        let db = test_db();
        let dispatcher = test_dispatcher(db.clone());

        let task = NewTask {
            agent_id: "mika".to_string(),
            team_run_id: None,
            parent_task_id: None,
            depth: 0,
            label: "test".to_string(),
            trigger_type: "time".to_string(),
            cron_expr: None,
            event_source: None,
            event_offset_secs: None,
            condition_expr: None,
            next_fire_at: Some(crate::timestamp::now()),
            timeout_at: None,
            action_type: "send_message".to_string(),
            action_config: r#"{"text": "hello"}"#.to_string(),
            input_context: None,
            created_by_session: None,
            created_trace_id: None,
            reference_url: None,
            source: None,
            metadata: None,
            r#type: None,
            dispatch_class: None,
        };
        let id = db.create_task(task).await.unwrap();
        let result = dispatcher.dispatch(&id).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_dispatch_inject_context_is_noop() {
        let db = test_db();
        let dispatcher = test_dispatcher(db.clone());

        let task = NewTask {
            agent_id: "mika".to_string(),
            team_run_id: None,
            parent_task_id: None,
            depth: 0,
            label: "test".to_string(),
            trigger_type: "time".to_string(),
            cron_expr: None,
            event_source: None,
            event_offset_secs: None,
            condition_expr: None,
            next_fire_at: Some(crate::timestamp::now()),
            timeout_at: None,
            action_type: "inject_context".to_string(),
            action_config: r#"{"context": "some context"}"#.to_string(),
            input_context: None,
            created_by_session: None,
            created_trace_id: None,
            reference_url: None,
            source: None,
            metadata: None,
            r#type: None,
            dispatch_class: None,
        };
        let id = db.create_task(task).await.unwrap();
        let result = dispatcher.dispatch(&id).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_dispatch_resume_agent_callback_no_result_returns_error() {
        let db = test_db();
        let dispatcher = test_dispatcher(db.clone());

        let task = NewTask {
            agent_id: "mika".to_string(),
            team_run_id: None,
            parent_task_id: None,
            depth: 0,
            label: "callback task".to_string(),
            trigger_type: "callback".to_string(),
            cron_expr: None,
            event_source: None,
            event_offset_secs: None,
            condition_expr: None,
            next_fire_at: Some(crate::timestamp::now()),
            timeout_at: None,
            action_type: "resume_agent".to_string(),
            action_config: r#"{"text": "some context"}"#.to_string(),
            input_context: None,
            created_by_session: None,
            created_trace_id: None,
            reference_url: None,
            source: None,
            metadata: None,
            r#type: None,
            dispatch_class: None,
        };
        let id = db.create_task(task).await.unwrap();
        // Callback with no result should error
        let result = dispatcher.dispatch(&id).await;
        assert!(result.is_err());
        assert!(
            result.unwrap_err().to_string().contains("no result"),
            "should error about missing result for callback"
        );
    }

    #[tokio::test]
    async fn test_dispatch_resume_agent_reminder_reads_action_config() {
        // Reminder-path resume_agent tasks read from action_config.text,
        // not task.result. This test verifies the dispatcher accepts
        // reminder tasks without a result field (the run_silent_agent
        // call will fail in test because there's no real LLM, but
        // the dispatch itself should not error).
        let db = test_db();
        let dispatcher = test_dispatcher(db.clone());

        let task = NewTask {
            agent_id: "mika".to_string(),
            team_run_id: None,
            parent_task_id: None,
            depth: 0,
            label: "Check CI status".to_string(),
            trigger_type: "time".to_string(),
            cron_expr: None,
            event_source: None,
            event_offset_secs: None,
            condition_expr: None,
            next_fire_at: Some(crate::timestamp::now()),
            timeout_at: None,
            action_type: "resume_agent".to_string(),
            action_config: r#"{"text": "Check CI status and merge PR"}"#.to_string(),
            input_context: None,
            created_by_session: None,
            created_trace_id: None,
            reference_url: None,
            source: None,
            metadata: None,
            r#type: None,
            dispatch_class: None,
        };
        let id = db.create_task(task).await.unwrap();
        // Should not error — reminder path reads from action_config
        let result = dispatcher.dispatch(&id).await;
        // The dispatch itself succeeds (run_silent_agent may fail internally
        // but dispatch_resume_agent catches that with warn! and returns Ok)
        assert!(
            result.is_ok(),
            "reminder resume_agent should not error: {:?}",
            result
        );
    }

    // ── delivery_backoff_secs tests (mika#2179) ──

    /// The doubling, up to the quarantine crossing. With a quarantine budget
    /// large enough not to interfere, the sequence is the plain exponential
    /// capped at the ceiling.
    #[test]
    fn delivery_backoff_doubles_then_holds_at_the_ceiling() {
        let seq: Vec<u64> = (1..=8)
            .map(|n| delivery_backoff_secs(60, 3600, n, u32::MAX))
            .collect();
        assert_eq!(seq, vec![60, 120, 240, 480, 960, 1920, 3600, 3600]);
    }

    /// The crossing jumps straight to the ceiling, and this is what makes the
    /// `callback_delivery_quarantined` log line and the operator doc true
    /// rather than approximately true. With the shipped budget of 3, the
    /// doubling has only reached 240 s at the crossing — four more attempts
    /// away from the hour both surfaces promise. Each of those attempts can
    /// hold the agent lock for `AGENT_TOTAL_TIMEOUT_SECS`, so "approximately"
    /// is worth about 20 minutes of lock on a bad night.
    #[test]
    fn delivery_backoff_jumps_to_the_ceiling_at_the_quarantine_crossing() {
        // Below the budget: ordinary doubling.
        assert_eq!(delivery_backoff_secs(60, 3600, 1, 3), 60);
        assert_eq!(delivery_backoff_secs(60, 3600, 2, 3), 120);
        // At and past it: the ceiling, not 240.
        assert_eq!(
            delivery_backoff_secs(60, 3600, 3, 3),
            3600,
            "the announced bound must be the applied bound"
        );
        assert_eq!(delivery_backoff_secs(60, 3600, 4, 3), 3600);
    }

    /// The far tail, which no ordinary run reaches and which the naive shift
    /// gets catastrophically wrong: `60u64 << 62` is 0, i.e. a `next_fire_at`
    /// in the past — the unbounded retry loop, back, after roughly 57 h of
    /// unbroken failure. Asserts the floor, not merely the absence of a panic:
    /// zero does not panic, so a "does not panic" test would have passed on the
    /// broken version. Quarantine is disabled here so the arithmetic itself is
    /// what is under test, not the crossing short-circuit in front of it.
    #[test]
    fn delivery_backoff_never_collapses_to_zero_on_a_long_outage() {
        for attempts in [40u32, 62, 63, 64, 65, 1000, u32::MAX] {
            let backoff = delivery_backoff_secs(60, 3600, attempts, u32::MAX);
            assert_eq!(
                backoff, 3600,
                "attempt {attempts} must stay at the ceiling, got {backoff}"
            );
        }
    }

    /// A first attempt gets exactly one base interval — one engine scan — so a
    /// single transient blip behaves as it did before this fix.
    #[test]
    fn delivery_backoff_first_attempt_is_one_base_interval() {
        assert_eq!(delivery_backoff_secs(60, 3600, 1, 3), 60);
        assert_eq!(
            delivery_backoff_secs(60, 3600, 0, 3),
            60,
            "attempts=0 is unreachable from the caller but must not underflow"
        );
    }

    // ── classify_delivery_error tests (mika#2179) ──

    /// The founding incident's exact error text, verbatim from
    /// `/var/log/mika/server.log` on the night of 2026-09-03/04.
    const INCIDENT_TRANSPORT_TIMEOUT: &str = "failed to read response body: error decoding response body: \
         request or response body error: operation timed out";

    #[test]
    fn classify_delivery_error_names_the_incident_class() {
        let err = anyhow::Error::new(mika_common::llm::error::LlmError::Transport(
            INCIDENT_TRANSPORT_TIMEOUT.to_string(),
        ));
        assert_eq!(classify_delivery_error(&err), "transport_timeout");
    }

    #[test]
    fn classify_delivery_error_separates_timeout_from_other_transport() {
        let err = anyhow::Error::new(mika_common::llm::error::LlmError::Transport(
            "connection refused".to_string(),
        ));
        assert_eq!(
            classify_delivery_error(&err),
            "transport",
            "a refused connection is a transport failure but not the starvation shape"
        );
    }

    #[test]
    fn classify_delivery_error_keeps_the_http_status() {
        for (status, expected) in [(429u16, "http_429"), (500, "http_500")] {
            let err = anyhow::Error::new(mika_common::llm::error::LlmError::HttpError {
                status,
                message: "boom".into(),
                retryable: true,
            });
            assert_eq!(
                classify_delivery_error(&err),
                expected,
                "rate-limited and provider-down must stay distinguishable"
            );
        }
    }

    #[test]
    fn classify_delivery_error_covers_the_remaining_variants() {
        use mika_common::llm::error::LlmError;
        let cases = [
            (LlmError::ParseError("bad json".into()), "parse"),
            (LlmError::ProviderError("upstream".into()), "provider"),
            (LlmError::UnsupportedFeature("vision".into()), "unsupported"),
        ];
        for (err, expected) in cases {
            assert_eq!(classify_delivery_error(&anyhow::Error::new(err)), expected);
        }
    }

    /// The load-bearing property: classification survives `.context()` layers,
    /// because `downcast_ref` walks the cause chain. `run_loop` propagates the
    /// `LlmError` with a bare `?` today, but nothing stops a future caller from
    /// adding context — and a classifier that broke silently on that would send
    /// every future starvation into the `other` bucket.
    #[test]
    fn classify_delivery_error_sees_through_context_layers() {
        let err = anyhow::Error::new(mika_common::llm::error::LlmError::Transport(
            INCIDENT_TRANSPORT_TIMEOUT.to_string(),
        ))
        .context("silent agent turn failed")
        .context("resume_agent");
        assert_eq!(classify_delivery_error(&err), "transport_timeout");
    }

    /// A non-LLM failure must not be laundered into an LLM class. The `Err`
    /// arm this classifier serves catches everything `run_silent_agent` can
    /// return, and most of that is not a provider problem.
    #[test]
    fn classify_delivery_error_reports_non_llm_errors_as_other() {
        let err = anyhow::anyhow!("database is locked");
        assert_eq!(classify_delivery_error(&err), "other");
    }

    // ── extract_callback_fields tests ──

    #[test]
    fn test_extract_success_format() {
        let result = "claude-pilot completed (status: done).\n\
                       Session: abc-123-def\n\
                       Turns: 91\n\
                       Cost: $7.07\n\
                       Duration: 996000ms";
        let extracted = extract_callback_fields(result);
        let cp = &extracted["claude_pilot"];
        assert_eq!(cp["session_id"], "abc-123-def");
        assert_eq!(cp["turns"], 91);
        assert_eq!(cp["cost_usd"], 7.07);
        assert_eq!(cp["duration_ms"], 996000);
    }

    #[test]
    fn test_extract_pipeline_failure_format() {
        let result = "PIPELINE FAILURE: zero commits produced.\n\
                       Session: sess-456\n\
                       Turns: 45\n\
                       Cost: $3.50\n\
                       Duration: 500000ms";
        let extracted = extract_callback_fields(result);
        let cp = &extracted["claude_pilot"];
        assert_eq!(cp["session_id"], "sess-456");
        assert_eq!(cp["turns"], 45);
        assert_eq!(cp["cost_usd"], 3.5);
        assert_eq!(cp["duration_ms"], 500000);
    }

    #[test]
    fn test_extract_partial_fields() {
        let result = "claude-pilot completed (status: done).\n\
                       Session: my-session\n\
                       Cost: $1.23";
        let extracted = extract_callback_fields(result);
        let cp = &extracted["claude_pilot"];
        assert_eq!(cp["session_id"], "my-session");
        assert_eq!(cp["cost_usd"], 1.23);
        assert!(cp.get("turns").is_none());
        assert!(cp.get("duration_ms").is_none());
    }

    #[test]
    fn test_extract_unknown_values_skipped() {
        let result = "claude-pilot completed (status: done).\n\
                       Session: unknown\n\
                       Turns: 10\n\
                       Cost: $unknown\n\
                       Duration: 5000ms";
        let extracted = extract_callback_fields(result);
        let cp = &extracted["claude_pilot"];
        // "unknown" session and cost should be skipped
        assert!(cp.get("session_id").is_none());
        assert_eq!(cp["turns"], 10);
        // Cost regex won't match "$unknown" since it expects digits
        assert!(cp.get("cost_usd").is_none());
        assert_eq!(cp["duration_ms"], 5000);
    }

    #[test]
    fn test_extract_garbage_returns_null() {
        let result = "some random text with no structured fields";
        let extracted = extract_callback_fields(result);
        assert!(extracted.is_null());
    }

    #[test]
    fn test_extract_empty_returns_null() {
        assert!(extract_callback_fields("").is_null());
    }

    // --- mika#2121 U2: NO_PR reason parser (mirror of the ^PR: parser) ---

    #[test]
    fn test_parse_no_pr_reason_known_reasons() {
        // All five reasons U1 emits across the three dispatch-lib sites.
        for reason in [
            "no_pr_on_branch",
            "gh_query_failed",
            "branch_unset",
            "repo_unset",
            "rescue_pr_create_failed",
        ] {
            let result = format!("Outcome: PIPELINE_INCOMPLETE\nNO_PR: {reason}");
            assert_eq!(parse_no_pr_reason(&result).as_deref(), Some(reason));
        }
    }

    #[test]
    fn test_parse_no_pr_reason_anchored_not_free_text() {
        // A free-text mention that is not line-anchored must not match.
        let result = "the run produced NO_PR: no_pr_on_branch inline in prose";
        assert_eq!(parse_no_pr_reason(result), None);
    }

    #[test]
    fn test_parse_no_pr_reason_absent_is_none() {
        // AC-G2 negative control: a callback carrying neither PR: nor NO_PR:
        // (a producer that predates U1) yields None so the reaper keeps the
        // generic motif.
        assert_eq!(
            parse_no_pr_reason("claude-pilot completed.\nTurns: 3"),
            None
        );
        assert_eq!(parse_no_pr_reason(""), None);
    }

    #[test]
    fn test_parse_no_pr_reason_ignores_pr_line() {
        // A PR: line is not a NO_PR: line — the reaper would not even fire in
        // this case (parent has a pr_url), but the parser must not confuse them.
        let result = "PR: https://github.com/senara-solutions/mika/pull/42";
        assert_eq!(parse_no_pr_reason(result), None);
    }

    #[tokio::test]
    async fn test_try_extract_callback_metadata_writes_to_parent() {
        let db = test_db();

        // Create a manual task (parent)
        let parent = NewTask {
            agent_id: "mika".to_string(),
            team_run_id: None,
            parent_task_id: None,
            depth: 0,
            label: "Implement feature #123".to_string(),
            trigger_type: "manual".to_string(),
            cron_expr: None,
            event_source: None,
            event_offset_secs: None,
            condition_expr: None,
            next_fire_at: None,
            timeout_at: None,
            action_type: "none".to_string(),
            action_config: "{}".to_string(),
            input_context: None,
            created_by_session: None,
            created_trace_id: None,
            reference_url: None,
            source: Some("self_dev".to_string()),
            metadata: None,
            r#type: None,
            dispatch_class: None,
        };
        let parent_id = db.create_task(parent).await.unwrap();

        // Create a callback task (child) with result text
        let callback = NewTask {
            agent_id: "mika".to_string(),
            team_run_id: None,
            parent_task_id: Some(parent_id.clone()),
            depth: 1,
            label: "run_claude_pilot".to_string(),
            trigger_type: "callback".to_string(),
            cron_expr: None,
            event_source: None,
            event_offset_secs: None,
            condition_expr: None,
            next_fire_at: None,
            timeout_at: None,
            action_type: "resume_agent".to_string(),
            action_config: "{}".to_string(),
            input_context: None,
            created_by_session: None,
            created_trace_id: None,
            reference_url: None,
            source: None,
            metadata: None,
            r#type: None,
            dispatch_class: None,
        };
        let callback_id = db.create_task(callback).await.unwrap();

        // Simulate callback completion with result
        db.update_task_completed(
            &callback_id,
            Some(
                "claude-pilot completed (status: done).\n\
                 Session: test-session-id\n\
                 Turns: 91\n\
                 Cost: $7.07\n\
                 Duration: 996000ms",
            ),
        )
        .await
        .unwrap();

        // Load the callback task and run extraction
        let task = db.get_task_unscoped(&callback_id).await.unwrap().unwrap();
        try_extract_callback_metadata(&db, &task).await;

        // Verify metadata was written to parent
        let parent_task = db.get_task_unscoped(&parent_id).await.unwrap().unwrap();
        let metadata: serde_json::Value =
            serde_json::from_str(parent_task.metadata.as_ref().unwrap()).unwrap();
        let cp = &metadata["claude_pilot"];
        assert_eq!(cp["session_id"], "test-session-id");
        assert_eq!(cp["turns"], 91);
        assert_eq!(cp["cost_usd"], 7.07);
        assert_eq!(cp["duration_ms"], 996000);
    }

    #[tokio::test]
    async fn test_try_extract_callback_metadata_merges_with_existing() {
        let db = test_db();

        // Create a manual task with existing metadata
        let parent = NewTask {
            agent_id: "mika".to_string(),
            team_run_id: None,
            parent_task_id: None,
            depth: 0,
            label: "Implement feature #456".to_string(),
            trigger_type: "manual".to_string(),
            cron_expr: None,
            event_source: None,
            event_offset_secs: None,
            condition_expr: None,
            next_fire_at: None,
            timeout_at: None,
            action_type: "none".to_string(),
            action_config: "{}".to_string(),
            input_context: None,
            created_by_session: None,
            created_trace_id: None,
            reference_url: None,
            source: Some("self_dev".to_string()),
            metadata: Some(r#"{"pipeline_retry_count": 1}"#.to_string()),
            r#type: None,
            dispatch_class: None,
        };
        let parent_id = db.create_task(parent).await.unwrap();

        // Create callback task
        let callback = NewTask {
            agent_id: "mika".to_string(),
            team_run_id: None,
            parent_task_id: Some(parent_id.clone()),
            depth: 1,
            label: "run_claude_pilot".to_string(),
            trigger_type: "callback".to_string(),
            cron_expr: None,
            event_source: None,
            event_offset_secs: None,
            condition_expr: None,
            next_fire_at: None,
            timeout_at: None,
            action_type: "resume_agent".to_string(),
            action_config: "{}".to_string(),
            input_context: None,
            created_by_session: None,
            created_trace_id: None,
            reference_url: None,
            source: None,
            metadata: None,
            r#type: None,
            dispatch_class: None,
        };
        let callback_id = db.create_task(callback).await.unwrap();

        db.update_task_completed(
            &callback_id,
            Some(
                "claude-pilot completed (status: done).\n\
                 Session: sess-789\n\
                 Turns: 50\n\
                 Cost: $4.00\n\
                 Duration: 300000ms",
            ),
        )
        .await
        .unwrap();

        let task = db.get_task_unscoped(&callback_id).await.unwrap().unwrap();
        try_extract_callback_metadata(&db, &task).await;

        // Verify metadata was merged (existing key preserved, new key added)
        let parent_task = db.get_task_unscoped(&parent_id).await.unwrap().unwrap();
        let metadata: serde_json::Value =
            serde_json::from_str(parent_task.metadata.as_ref().unwrap()).unwrap();
        assert_eq!(metadata["pipeline_retry_count"], 1);
        assert_eq!(metadata["claude_pilot"]["session_id"], "sess-789");
        assert_eq!(metadata["claude_pilot"]["turns"], 50);
    }

    #[tokio::test]
    async fn test_try_extract_callback_metadata_noop_no_parent() {
        let db = test_db();

        // Callback task with no parent_task_id — should be a no-op
        let callback = NewTask {
            agent_id: "mika".to_string(),
            team_run_id: None,
            parent_task_id: None,
            depth: 0,
            label: "run_claude_pilot".to_string(),
            trigger_type: "callback".to_string(),
            cron_expr: None,
            event_source: None,
            event_offset_secs: None,
            condition_expr: None,
            next_fire_at: None,
            timeout_at: None,
            action_type: "resume_agent".to_string(),
            action_config: "{}".to_string(),
            input_context: None,
            created_by_session: None,
            created_trace_id: None,
            reference_url: None,
            source: None,
            metadata: None,
            r#type: None,
            dispatch_class: None,
        };
        let callback_id = db.create_task(callback).await.unwrap();
        db.update_task_completed(&callback_id, Some("Session: abc\nTurns: 10"))
            .await
            .unwrap();

        let task = db.get_task_unscoped(&callback_id).await.unwrap().unwrap();
        // Should not panic or error — just returns early
        try_extract_callback_metadata(&db, &task).await;
    }

    // ===== try_write_callback_summary tests (mika#965) =====

    #[tokio::test]
    async fn test_callback_summary_writes_to_scope_root() {
        let db = test_db();

        // Create a manual parent task (acts as scope root)
        let parent = NewTask {
            agent_id: "mika".to_string(),
            team_run_id: None,
            parent_task_id: None,
            depth: 0,
            label: "Implement feature #965".to_string(),
            trigger_type: "manual".to_string(),
            cron_expr: None,
            event_source: None,
            event_offset_secs: None,
            condition_expr: None,
            next_fire_at: None,
            timeout_at: None,
            action_type: "none".to_string(),
            action_config: "{}".to_string(),
            input_context: None,
            created_by_session: None,
            created_trace_id: None,
            reference_url: None,
            source: Some("self_dev".to_string()),
            metadata: None,
            r#type: None,
            dispatch_class: None,
        };
        let parent_id = db.create_task(parent).await.unwrap();

        // Create a callback child with result text including pr_url
        let callback = NewTask {
            agent_id: "mika".to_string(),
            team_run_id: None,
            parent_task_id: Some(parent_id.clone()),
            depth: 1,
            label: "run_claude_pilot".to_string(),
            trigger_type: "callback".to_string(),
            cron_expr: None,
            event_source: None,
            event_offset_secs: None,
            condition_expr: None,
            next_fire_at: None,
            timeout_at: None,
            action_type: "resume_agent".to_string(),
            action_config: "{}".to_string(),
            input_context: None,
            created_by_session: Some("session-cb-1".to_string()),
            created_trace_id: None,
            reference_url: None,
            source: None,
            metadata: None,
            r#type: None,
            dispatch_class: None,
        };
        let callback_id = db.create_task(callback).await.unwrap();
        db.update_task_completed(
            &callback_id,
            Some(
                "claude-pilot completed (status: done).\n\
                 Session: test-session-abc\n\
                 Turns: 42\n\
                 Cost: $3.50\n\
                 Duration: 120000ms\n\
                 PR: https://github.com/senara-solutions/mika/pull/999",
            ),
        )
        .await
        .unwrap();

        let task = db.get_task_unscoped(&callback_id).await.unwrap().unwrap();
        try_write_callback_summary(&db, &task).await;

        // Verify task_messages row was written keyed to the parent (scope root)
        let msgs = db.load_task_messages(&parent_id).await.unwrap();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].role, "system");
        assert!(msgs[0].content.contains("session=test-session-abc"));
        assert!(msgs[0].content.contains("turns=42"));
        assert!(msgs[0].content.contains("cost=$3.50"));
        assert!(msgs[0].content.contains("duration=120000ms"));
        assert!(
            msgs[0]
                .content
                .contains("PR: https://github.com/senara-solutions/mika/pull/999")
        );
        // Metadata should contain the extracted JSON
        let meta: serde_json::Value =
            serde_json::from_str(msgs[0].metadata.as_ref().unwrap()).unwrap();
        assert_eq!(meta["claude_pilot"]["session_id"], "test-session-abc");
        assert_eq!(meta["claude_pilot"]["turns"], 42);
    }

    #[tokio::test]
    async fn test_callback_summary_noop_no_parent() {
        let db = test_db();

        // Callback with no parent — should silently return
        let callback = NewTask {
            agent_id: "mika".to_string(),
            team_run_id: None,
            parent_task_id: None,
            depth: 0,
            label: "run_claude_pilot".to_string(),
            trigger_type: "callback".to_string(),
            cron_expr: None,
            event_source: None,
            event_offset_secs: None,
            condition_expr: None,
            next_fire_at: None,
            timeout_at: None,
            action_type: "resume_agent".to_string(),
            action_config: "{}".to_string(),
            input_context: None,
            created_by_session: None,
            created_trace_id: None,
            reference_url: None,
            source: None,
            metadata: None,
            r#type: None,
            dispatch_class: None,
        };
        let callback_id = db.create_task(callback).await.unwrap();
        db.update_task_completed(&callback_id, Some("Session: abc\nTurns: 10"))
            .await
            .unwrap();

        let task = db.get_task_unscoped(&callback_id).await.unwrap().unwrap();
        // Should not panic or error — just returns early
        try_write_callback_summary(&db, &task).await;
    }

    #[tokio::test]
    async fn test_callback_summary_noop_empty_result() {
        let db = test_db();

        let parent = NewTask {
            agent_id: "mika".to_string(),
            team_run_id: None,
            parent_task_id: None,
            depth: 0,
            label: "parent".to_string(),
            trigger_type: "manual".to_string(),
            cron_expr: None,
            event_source: None,
            event_offset_secs: None,
            condition_expr: None,
            next_fire_at: None,
            timeout_at: None,
            action_type: "none".to_string(),
            action_config: "{}".to_string(),
            input_context: None,
            created_by_session: None,
            created_trace_id: None,
            reference_url: None,
            source: None,
            metadata: None,
            r#type: None,
            dispatch_class: None,
        };
        let parent_id = db.create_task(parent).await.unwrap();

        let callback = NewTask {
            agent_id: "mika".to_string(),
            team_run_id: None,
            parent_task_id: Some(parent_id.clone()),
            depth: 1,
            label: "run_claude_pilot".to_string(),
            trigger_type: "callback".to_string(),
            cron_expr: None,
            event_source: None,
            event_offset_secs: None,
            condition_expr: None,
            next_fire_at: None,
            timeout_at: None,
            action_type: "resume_agent".to_string(),
            action_config: "{}".to_string(),
            input_context: None,
            created_by_session: None,
            created_trace_id: None,
            reference_url: None,
            source: None,
            metadata: None,
            r#type: None,
            dispatch_class: None,
        };
        let callback_id = db.create_task(callback).await.unwrap();
        // Complete with empty result — no fields extractable
        db.update_task_completed(&callback_id, Some(""))
            .await
            .unwrap();

        let task = db.get_task_unscoped(&callback_id).await.unwrap().unwrap();
        try_write_callback_summary(&db, &task).await;

        // No task_messages should be written
        let msgs = db.load_task_messages(&parent_id).await.unwrap();
        assert!(msgs.is_empty());
    }

    #[tokio::test]
    async fn test_callback_summary_handles_missing_optional_fields() {
        let db = test_db();

        let parent = NewTask {
            agent_id: "mika".to_string(),
            team_run_id: None,
            parent_task_id: None,
            depth: 0,
            label: "groom task".to_string(),
            trigger_type: "manual".to_string(),
            cron_expr: None,
            event_source: None,
            event_offset_secs: None,
            condition_expr: None,
            next_fire_at: None,
            timeout_at: None,
            action_type: "none".to_string(),
            action_config: "{}".to_string(),
            input_context: None,
            created_by_session: None,
            created_trace_id: None,
            reference_url: None,
            source: None,
            metadata: None,
            r#type: None,
            dispatch_class: None,
        };
        let parent_id = db.create_task(parent).await.unwrap();

        let callback = NewTask {
            agent_id: "mika".to_string(),
            team_run_id: None,
            parent_task_id: Some(parent_id.clone()),
            depth: 1,
            label: "run_claude_pilot_groom".to_string(),
            trigger_type: "callback".to_string(),
            cron_expr: None,
            event_source: None,
            event_offset_secs: None,
            condition_expr: None,
            next_fire_at: None,
            timeout_at: None,
            action_type: "resume_agent".to_string(),
            action_config: "{}".to_string(),
            input_context: None,
            created_by_session: Some("session-groom".to_string()),
            created_trace_id: None,
            reference_url: None,
            source: None,
            metadata: None,
            r#type: None,
            dispatch_class: None,
        };
        let callback_id = db.create_task(callback).await.unwrap();
        // Groom callback — no pr_url, just session and turns
        db.update_task_completed(&callback_id, Some("Session: groom-123\nTurns: 15"))
            .await
            .unwrap();

        let task = db.get_task_unscoped(&callback_id).await.unwrap().unwrap();
        try_write_callback_summary(&db, &task).await;

        let msgs = db.load_task_messages(&parent_id).await.unwrap();
        assert_eq!(msgs.len(), 1);
        assert!(msgs[0].content.contains("session=groom-123"));
        assert!(msgs[0].content.contains("turns=15"));
        // Should NOT contain pr_url since it's not in the result
        assert!(!msgs[0].content.contains("PR:"));
    }

    // ===== is_team_child_callback predicate tests =====

    fn make_task_with_team_fields(team_run_id: Option<&str>, parent_task_id: Option<&str>) -> Task {
        Task {
            id: "test-task-1".to_string(),
            agent_id: "mika".to_string(),
            team_run_id: team_run_id.map(String::from),
            parent_task_id: parent_task_id.map(String::from),
            depth: 0,
            label: "test".to_string(),
            trigger_type: "callback".to_string(),
            cron_expr: None,
            event_source: None,
            event_offset_secs: None,
            condition_expr: None,
            next_fire_at: None,
            timeout_at: None,
            action_type: "resume_agent".to_string(),
            action_config: "{}".to_string(),
            status: "completed".to_string(),
            process_id: None,
            input_context: None,
            result: None,
            created_by_session: None,
            created_trace_id: None,
            execution_trace_id: None,
            created_at: "2026-04-24T12:00:00Z".to_string(),
            updated_at: "2026-04-24T12:00:00Z".to_string(),
            fired_at: None,
            completed_at: None,
            reference_url: None,
            source: None,
            metadata: None,
            r#type: "issue".to_string(),
            dispatch_class: None,
            dispatcher_source: None,
        }
    }

    #[test]
    fn test_is_team_child_callback_both_set() {
        let task = make_task_with_team_fields(Some("run-1"), Some("parent-1"));
        assert!(is_team_child_callback(&task));
    }

    #[test]
    fn test_is_team_child_callback_no_team_run_id() {
        // Regular skill callback with a parent but no team_run_id
        let task = make_task_with_team_fields(None, Some("parent-1"));
        assert!(!is_team_child_callback(&task));
    }

    #[test]
    fn test_is_team_child_callback_no_parent() {
        // team_run_id set but no parent — team root task (not a child callback)
        let task = make_task_with_team_fields(Some("run-1"), None);
        assert!(!is_team_child_callback(&task));
    }

    #[test]
    fn test_is_team_child_callback_neither_set() {
        let task = make_task_with_team_fields(None, None);
        assert!(!is_team_child_callback(&task));
    }

    // -- extract_callback_fields pr_url tests (#871 R4) --

    #[test]
    fn test_extract_callback_fields_parses_pr_url() {
        let input = "claude-pilot completed (status: done).\n\
                      Session: abc123\n\
                      Turns: 42\n\
                      Cost: $1.23\n\
                      Duration: 180000ms\n\
                      PR: https://github.com/senara-solutions/mika/pull/871";

        let val = extract_callback_fields(input);
        let pr_url = val
            .get("claude_pilot")
            .and_then(|cp| cp.get("pr_url"))
            .and_then(|v| v.as_str());
        assert_eq!(
            pr_url,
            Some("https://github.com/senara-solutions/mika/pull/871")
        );
    }

    #[test]
    fn test_extract_callback_fields_no_pr_url() {
        let input = "claude-pilot completed (status: done).\n\
                      Session: abc123\n\
                      Turns: 10\n\
                      Cost: $0.50\n\
                      Duration: 60000ms";

        let val = extract_callback_fields(input);
        let pr_url = val.get("claude_pilot").and_then(|cp| cp.get("pr_url"));
        assert!(
            pr_url.is_none(),
            "should not have pr_url when line is absent"
        );
    }

    #[test]
    fn test_extract_callback_fields_pr_url_not_matched_in_prose() {
        // PR: in the middle of a line should not match (regex anchored to ^)
        let input = "Session: abc123\nSome text PR: https://github.com/x/y/pull/1 more text";
        let val = extract_callback_fields(input);
        let pr_url = val.get("claude_pilot").and_then(|cp| cp.get("pr_url"));
        assert!(pr_url.is_none(), "mid-line PR: should not match");
    }

    // ── maybe_fire_post_callback_advance unit tests (#991) ──

    /// Helper: create a parent milestone task and a callback child task.
    /// Returns `(parent_id, child_id)`.
    async fn create_milestone_with_callback_child(
        db: &AsyncDatabase,
        parent_status: &str,
        child_status: &str,
    ) -> (String, String) {
        let parent_id = db
            .create_task(NewTask {
                agent_id: "mika".to_string(),
                team_run_id: None,
                parent_task_id: None,
                depth: 0,
                label: "Milestone #19".to_string(),
                trigger_type: "manual".to_string(),
                cron_expr: None,
                event_source: None,
                event_offset_secs: None,
                condition_expr: None,
                next_fire_at: None,
                timeout_at: None,
                action_type: "none".to_string(),
                action_config: "{}".to_string(),
                input_context: None,
                created_by_session: None,
                created_trace_id: None,
                reference_url: None,
                source: Some("self_dev".to_string()),
                metadata: None,
                r#type: Some("milestone".to_string()),
                dispatch_class: None,
            })
            .await
            .unwrap();

        // Transition parent to the desired status
        if parent_status != "pending" {
            db.update_task_status(&parent_id, "in_progress")
                .await
                .unwrap();
            if parent_status == "blocked" {
                db.update_task_status(&parent_id, "blocked").await.unwrap();
            } else if parent_status == "completed" {
                db.update_task_status(&parent_id, "completed")
                    .await
                    .unwrap();
            }
        }

        let child_id = db
            .create_task(NewTask {
                agent_id: "mika".to_string(),
                team_run_id: None,
                parent_task_id: Some(parent_id.clone()),
                depth: 1,
                label: "Child issue #200".to_string(),
                trigger_type: "callback".to_string(),
                cron_expr: None,
                event_source: None,
                event_offset_secs: None,
                condition_expr: None,
                next_fire_at: Some(crate::timestamp::now()),
                timeout_at: None,
                action_type: "resume_agent".to_string(),
                action_config: r#"{"text": "callback result"}"#.to_string(),
                input_context: None,
                created_by_session: None,
                created_trace_id: None,
                reference_url: None,
                source: None,
                metadata: None,
                r#type: Some("issue".to_string()),
                dispatch_class: None,
            })
            .await
            .unwrap();

        // Transition child to desired status
        if child_status != "pending" {
            db.update_task_status(&child_id, "in_progress")
                .await
                .unwrap();
            if child_status == "completed" {
                db.update_task_status(&child_id, "completed").await.unwrap();
            } else if child_status == "delivered" {
                db.update_task_status(&child_id, "completed").await.unwrap();
                db.mark_task_delivered(&child_id).await.unwrap();
            } else if child_status == "failed" {
                db.update_task_failed(&child_id, "subprocess crashed")
                    .await
                    .unwrap();
            }
        }

        (parent_id, child_id)
    }

    /// #991: maybe_fire_post_callback_advance skips when callback has no parent.
    #[tokio::test]
    async fn test_post_callback_advance_skips_no_parent() {
        let db = test_db();
        let dispatcher = test_dispatcher(db.clone());

        // Create a standalone callback task (no parent)
        let child_id = db
            .create_task(NewTask {
                agent_id: "mika".to_string(),
                team_run_id: None,
                parent_task_id: None,
                depth: 0,
                label: "Standalone callback".to_string(),
                trigger_type: "callback".to_string(),
                cron_expr: None,
                event_source: None,
                event_offset_secs: None,
                condition_expr: None,
                next_fire_at: Some(crate::timestamp::now()),
                timeout_at: None,
                action_type: "resume_agent".to_string(),
                action_config: r#"{"text": "result"}"#.to_string(),
                input_context: None,
                created_by_session: None,
                created_trace_id: None,
                reference_url: None,
                source: None,
                metadata: None,
                r#type: None,
                dispatch_class: None,
            })
            .await
            .unwrap();

        let task = db.get_task_unscoped(&child_id).await.unwrap().unwrap();

        // Should return early — no parent means no milestone advance needed.
        // The function returns () so we just verify it doesn't panic.
        dispatcher.maybe_fire_post_callback_advance(&task).await;
    }

    /// #991: maybe_fire_post_callback_advance skips when parent is not a milestone/project.
    #[tokio::test]
    async fn test_post_callback_advance_skips_non_milestone_parent() {
        let db = test_db();
        let dispatcher = test_dispatcher(db.clone());

        // Create a regular issue parent (not milestone/project)
        let parent_id = db
            .create_task(NewTask {
                agent_id: "mika".to_string(),
                team_run_id: None,
                parent_task_id: None,
                depth: 0,
                label: "Regular issue".to_string(),
                trigger_type: "manual".to_string(),
                cron_expr: None,
                event_source: None,
                event_offset_secs: None,
                condition_expr: None,
                next_fire_at: None,
                timeout_at: None,
                action_type: "none".to_string(),
                action_config: "{}".to_string(),
                input_context: None,
                created_by_session: None,
                created_trace_id: None,
                reference_url: None,
                source: None,
                metadata: None,
                r#type: Some("issue".to_string()),
                dispatch_class: None,
            })
            .await
            .unwrap();

        db.update_task_status(&parent_id, "in_progress")
            .await
            .unwrap();

        let child_id = db
            .create_task(NewTask {
                agent_id: "mika".to_string(),
                team_run_id: None,
                parent_task_id: Some(parent_id.clone()),
                depth: 1,
                label: "Callback child".to_string(),
                trigger_type: "callback".to_string(),
                cron_expr: None,
                event_source: None,
                event_offset_secs: None,
                condition_expr: None,
                next_fire_at: Some(crate::timestamp::now()),
                timeout_at: None,
                action_type: "resume_agent".to_string(),
                action_config: r#"{"text": "result"}"#.to_string(),
                input_context: None,
                created_by_session: None,
                created_trace_id: None,
                reference_url: None,
                source: None,
                metadata: None,
                r#type: Some("issue".to_string()),
                dispatch_class: None,
            })
            .await
            .unwrap();

        let task = db.get_task_unscoped(&child_id).await.unwrap().unwrap();

        // Should skip — parent is type=issue, not milestone/project
        dispatcher.maybe_fire_post_callback_advance(&task).await;

        // Parent should still be in_progress (not auto-blocked)
        let parent = db.get_task_unscoped(&parent_id).await.unwrap().unwrap();
        assert_eq!(parent.status, "in_progress");
    }

    /// #991: maybe_fire_post_callback_advance skips when parent is already terminal.
    #[tokio::test]
    async fn test_post_callback_advance_skips_terminal_parent() {
        let db = test_db();
        let dispatcher = test_dispatcher(db.clone());

        let (parent_id, child_id) = create_milestone_with_callback_child(
            &db,
            "completed", // Parent already completed
            "completed",
        )
        .await;

        let task = db.get_task_unscoped(&child_id).await.unwrap().unwrap();

        // Should skip — parent is already terminal
        dispatcher.maybe_fire_post_callback_advance(&task).await;

        // Parent should remain completed
        let parent = db.get_task_unscoped(&parent_id).await.unwrap().unwrap();
        assert_eq!(parent.status, "completed");
    }

    /// #991: maybe_fire_post_callback_advance skips when an active callback child exists
    /// (queue was already advanced).
    #[tokio::test]
    async fn test_post_callback_advance_skips_when_active_child_exists() {
        let db = test_db();
        let dispatcher = test_dispatcher(db.clone());

        let (parent_id, child_id) =
            create_milestone_with_callback_child(&db, "in_progress", "completed").await;

        // Create a SECOND active callback child — simulates that the agent
        // already dispatched the next child via run_claude_pilot.
        let _next_child_id = db
            .create_task(NewTask {
                agent_id: "mika".to_string(),
                team_run_id: None,
                parent_task_id: Some(parent_id.clone()),
                depth: 1,
                label: "Next child issue #201".to_string(),
                trigger_type: "callback".to_string(),
                cron_expr: None,
                event_source: None,
                event_offset_secs: None,
                condition_expr: None,
                next_fire_at: Some(crate::timestamp::now()),
                timeout_at: None,
                action_type: "resume_agent".to_string(),
                action_config: r#"{"text": "pending"}"#.to_string(),
                input_context: None,
                created_by_session: None,
                created_trace_id: None,
                reference_url: None,
                source: None,
                metadata: None,
                r#type: Some("issue".to_string()),
                dispatch_class: None,
            })
            .await
            .unwrap();

        let task = db.get_task_unscoped(&child_id).await.unwrap().unwrap();

        // Should skip — the next child is already pending (queue was advanced)
        dispatcher.maybe_fire_post_callback_advance(&task).await;

        // Parent should still be in_progress (not auto-blocked)
        let parent = db.get_task_unscoped(&parent_id).await.unwrap().unwrap();
        assert_eq!(parent.status, "in_progress");
    }

    /// mika#1124: drift guard — the DEFERRED_DISPATCH_LABEL constant is the
    /// load-bearing string the anti-cascade guard at the chain-promotion
    /// callsite matches against. If either side drifts, the wedge returns
    /// silently.
    #[test]
    fn test_deferred_dispatch_label_string_drift_guard() {
        assert_eq!(
            crate::agent::DEFERRED_DISPATCH_LABEL,
            "long_running:run_claude_pilot:deferred",
            "DEFERRED_DISPATCH_LABEL changed — verify the anti-cascade guard \
             in dispatch_resume_agent (mika#1124) still matches the wrapper task label."
        );
    }

    /// mika#1124: the legitimate FIFO promotion path still works. Verifies that
    /// `dispatch_next_deferred_callback` promotes the oldest pending deferred
    /// callback when invoked. The anti-cascade guard short-circuits this call
    /// only when the just-completed task is itself a deferred wrapper —
    /// non-wrapper callbacks and the periodic backstop still drive the queue.
    #[tokio::test]
    async fn test_dispatch_next_deferred_callback_promotes_pending() {
        let db = test_db();
        let dispatcher = test_dispatcher(db.clone());

        let parent_id = db
            .create_task(NewTask {
                agent_id: "mika".to_string(),
                team_run_id: None,
                parent_task_id: None,
                depth: 0,
                label: "manual self_dev parent".to_string(),
                trigger_type: "manual".to_string(),
                cron_expr: None,
                event_source: None,
                event_offset_secs: None,
                condition_expr: None,
                next_fire_at: None,
                timeout_at: None,
                action_type: "none".to_string(),
                action_config: "{}".to_string(),
                input_context: None,
                created_by_session: None,
                created_trace_id: None,
                reference_url: None,
                source: Some("self_dev".to_string()),
                metadata: None,
                r#type: None,
                dispatch_class: None,
            })
            .await
            .unwrap();

        let deferred_id = db
            .create_task(NewTask {
                agent_id: "mika".to_string(),
                team_run_id: None,
                parent_task_id: Some(parent_id),
                depth: 1,
                label: crate::agent::DEFERRED_DISPATCH_LABEL.to_string(),
                trigger_type: "callback".to_string(),
                cron_expr: None,
                event_source: None,
                event_offset_secs: None,
                condition_expr: None,
                next_fire_at: Some(crate::timestamp::now()),
                timeout_at: None,
                action_type: "resume_agent".to_string(),
                action_config: r#"{"text": "deferred"}"#.to_string(),
                input_context: None,
                created_by_session: None,
                created_trace_id: None,
                reference_url: None,
                source: None,
                metadata: None,
                r#type: None,
                dispatch_class: Some("implement".to_string()),
            })
            .await
            .unwrap();

        // Pre-condition: deferred wrapper is `pending`.
        let before = db.get_task_unscoped(&deferred_id).await.unwrap().unwrap();
        assert_eq!(before.status, "pending");

        // Drive the legitimate promotion path directly.
        dispatcher.dispatch_next_deferred_callback().await;

        // Post-condition: the wrapper is `completed` with the synthetic result
        // the engine recognizes for DeferredDispatch silent-turn dispatch.
        let after = db.get_task_unscoped(&deferred_id).await.unwrap().unwrap();
        assert_eq!(
            after.status, "completed",
            "deferred wrapper should be promoted"
        );
        assert!(
            after
                .result
                .as_deref()
                .unwrap_or("")
                .contains("deferred dispatch slot freed"),
            "promoted wrapper carries the engine's synthetic result string"
        );
    }

    /// #991: maybe_fire_post_callback_advance fires and auto-blocks when no
    /// active child exists and the dummy LLM fails (simulating advance failure).
    #[tokio::test]
    async fn test_post_callback_advance_auto_blocks_on_failure() {
        let db = test_db();
        let dispatcher = test_dispatcher(db.clone());

        let (parent_id, child_id) =
            create_milestone_with_callback_child(&db, "in_progress", "completed").await;

        // Create the required session for the callback task
        db.create_session_with_parent("test-session", "mika", "system", None, None, None)
            .await
            .unwrap();

        let task = db.get_task_unscoped(&child_id).await.unwrap().unwrap();

        // The dummy LLM provider will fail, causing the advance turn to error.
        // The function should then auto-block the milestone.
        dispatcher.maybe_fire_post_callback_advance(&task).await;

        // Parent should be auto-blocked (failed status) because the dummy
        // LLM can't actually run the advance turn.
        let parent = db.get_task_unscoped(&parent_id).await.unwrap().unwrap();
        assert_eq!(
            parent.status, "failed",
            "parent milestone should be auto-blocked when advance turn fails"
        );
    }

    // -- try_resolve_parent_on_dispatch_refusal tests (mika#2158 AC8) --

    /// The literal refusal body `dispatch-lib` ships for `already_groomed`, trimmed of the
    /// long `note` field. Shape, not paraphrase: `status` + `reason` are what the resolver
    /// keys on.
    const ALREADY_GROOMED_RESULT: &str = r#"{"status":"auto_skipped","reason":"already_groomed","issue":"senara-solutions/mika#2108","branch":"fix/2108/x","plan":"docs/plans/p.md","provenance":"committed on branch","note":"Dispatch dev-pilot to implement."}"#;

    /// Helper: a self_dev parent in `in_progress` with a groom callback child whose result
    /// is `body`.
    async fn create_refusal_callback_pair(db: &AsyncDatabase, body: &str) -> (String, String) {
        let parent = NewTask {
            agent_id: "mika".to_string(),
            team_run_id: None,
            parent_task_id: None,
            depth: 0,
            label: "ready-label: senara-solutions/mika#2108".to_string(),
            trigger_type: "manual".to_string(),
            cron_expr: None,
            event_source: None,
            event_offset_secs: None,
            condition_expr: None,
            next_fire_at: None,
            timeout_at: None,
            action_type: "none".to_string(),
            action_config: "{}".to_string(),
            input_context: None,
            created_by_session: None,
            created_trace_id: None,
            reference_url: Some("https://github.com/senara-solutions/mika/issues/2108".to_string()),
            source: Some("self_dev".to_string()),
            metadata: None,
            r#type: Some("issue".to_string()),
            dispatch_class: Some("groom".to_string()),
        };
        let parent_id = db.create_task(parent).await.unwrap();
        db.update_task_status(&parent_id, "in_progress")
            .await
            .unwrap();

        let callback = NewTask {
            agent_id: "mika".to_string(),
            team_run_id: None,
            parent_task_id: Some(parent_id.clone()),
            depth: 1,
            label: "long_running:run_claude_pilot_groom".to_string(),
            trigger_type: "callback".to_string(),
            cron_expr: None,
            event_source: None,
            event_offset_secs: None,
            condition_expr: None,
            next_fire_at: None,
            timeout_at: None,
            action_type: "resume_agent".to_string(),
            action_config: "{}".to_string(),
            input_context: None,
            created_by_session: None,
            created_trace_id: None,
            reference_url: None,
            source: None,
            metadata: None,
            r#type: None,
            dispatch_class: Some("groom".to_string()),
        };
        let callback_id = db.create_task(callback).await.unwrap();
        db.update_task_completed(&callback_id, Some(body))
            .await
            .unwrap();
        (parent_id, callback_id)
    }

    #[test]
    fn test_parse_auto_skip_reason_shapes() {
        assert_eq!(
            parse_auto_skip_reason(ALREADY_GROOMED_RESULT).as_deref(),
            Some("already_groomed")
        );
        // mika#988's sibling refusal — same mechanism, different name.
        assert_eq!(
            parse_auto_skip_reason(r#"{"status":"auto_skipped","reason":"issue_closed"}"#)
                .as_deref(),
            Some("issue_closed")
        );
        // Tolerant of a prologue/epilogue around the JSON body.
        assert_eq!(
            parse_auto_skip_reason(
                "dispatch note\n{\"status\":\"auto_skipped\",\"reason\":\"already_groomed\"}\ntrailer"
            )
            .as_deref(),
            Some("already_groomed")
        );
        // …and of a brace inside the refusal's own prose fields, which a naive
        // first-`{`..last-`}` slice would be defeated by.
        assert_eq!(
            parse_auto_skip_reason(
                "prologue\n{\"status\":\"auto_skipped\",\"reason\":\"already_groomed\",\"note\":\"use ${BRANCH} or {plan}\"}\nepilogue }"
            )
            .as_deref(),
            Some("already_groomed")
        );
        // A refusal with no named reason is still a refusal.
        assert_eq!(
            parse_auto_skip_reason(r#"{"status":"auto_skipped"}"#).as_deref(),
            Some("unspecified")
        );
        // Non-refusals must not match.
        assert_eq!(parse_auto_skip_reason(""), None);
        assert_eq!(
            parse_auto_skip_reason("claude-pilot completed (done)."),
            None
        );
        assert_eq!(
            parse_auto_skip_reason(r#"{"status":"completed","reason":"already_groomed"}"#),
            None
        );
    }

    #[tokio::test]
    async fn test_refusal_resolves_its_own_tracking_row() {
        let db = test_db();
        let (parent_id, callback_id) =
            create_refusal_callback_pair(&db, ALREADY_GROOMED_RESULT).await;
        let callback = db.get_task_unscoped(&callback_id).await.unwrap().unwrap();

        try_resolve_parent_on_dispatch_refusal(&db, &callback).await;

        let parent = db.get_task_unscoped(&parent_id).await.unwrap().unwrap();
        assert_eq!(
            parent.status, "completed",
            "the refusal must resolve the row it left behind, not let it age into a phantom"
        );
        assert!(
            parent
                .result
                .as_deref()
                .unwrap_or_default()
                .contains("already_groomed"),
            "the reason must survive into the row so the two refusal populations stay \
             separable, got: {:?}",
            parent.result
        );
    }

    #[tokio::test]
    async fn test_non_refusal_callback_leaves_the_parent_alone() {
        let db = test_db();
        let (parent_id, callback_id) =
            create_refusal_callback_pair(&db, "claude-pilot completed (status: done).").await;
        let callback = db.get_task_unscoped(&callback_id).await.unwrap().unwrap();

        try_resolve_parent_on_dispatch_refusal(&db, &callback).await;

        let parent = db.get_task_unscoped(&parent_id).await.unwrap().unwrap();
        assert_eq!(
            parent.status, "in_progress",
            "only a refusal payload may resolve the parent"
        );
    }

    #[tokio::test]
    async fn test_refusal_does_not_reopen_a_terminal_parent() {
        let db = test_db();
        let (parent_id, callback_id) =
            create_refusal_callback_pair(&db, ALREADY_GROOMED_RESULT).await;
        db.update_task_failed(&parent_id, "reaped earlier")
            .await
            .unwrap();
        let callback = db.get_task_unscoped(&callback_id).await.unwrap().unwrap();

        try_resolve_parent_on_dispatch_refusal(&db, &callback).await;

        let parent = db.get_task_unscoped(&parent_id).await.unwrap().unwrap();
        assert_eq!(
            parent.status, "failed",
            "a parent that already reached a terminal state is not this resolver's business"
        );
    }

    // -- try_complete_parent_on_callback_success tests (mika#1162) --

    /// Helper: create a self_dev parent in `in_progress` with an `implement`
    /// callback child whose result carries the supplied `PR:` line.
    async fn create_success_callback_pair(
        db: &AsyncDatabase,
        pr_line: Option<&str>,
    ) -> (String, String) {
        let parent = NewTask {
            agent_id: "mika".to_string(),
            team_run_id: None,
            parent_task_id: None,
            depth: 0,
            label: "Implement mika#1162".to_string(),
            trigger_type: "manual".to_string(),
            cron_expr: None,
            event_source: None,
            event_offset_secs: None,
            condition_expr: None,
            next_fire_at: None,
            timeout_at: None,
            action_type: "none".to_string(),
            action_config: "{}".to_string(),
            input_context: None,
            created_by_session: None,
            created_trace_id: None,
            reference_url: None,
            source: Some("self_dev".to_string()),
            metadata: None,
            r#type: None,
            dispatch_class: Some("implement".to_string()),
        };
        let parent_id = db.create_task(parent).await.unwrap();
        // Move parent to in_progress
        db.update_task_status(&parent_id, "in_progress")
            .await
            .unwrap();

        let callback = NewTask {
            agent_id: "mika".to_string(),
            team_run_id: None,
            parent_task_id: Some(parent_id.clone()),
            depth: 1,
            label: "long_running:run_claude_pilot".to_string(),
            trigger_type: "callback".to_string(),
            cron_expr: None,
            event_source: None,
            event_offset_secs: None,
            condition_expr: None,
            next_fire_at: None,
            timeout_at: None,
            action_type: "resume_agent".to_string(),
            action_config: "{}".to_string(),
            input_context: None,
            created_by_session: None,
            created_trace_id: None,
            reference_url: None,
            source: None,
            metadata: None,
            r#type: None,
            dispatch_class: Some("implement".to_string()),
        };
        let callback_id = db.create_task(callback).await.unwrap();
        let mut body = String::from(
            "claude-pilot completed (status: done).\n\
             Session: sess-1162\n\
             Turns: 181\n\
             Cost: $20.82\n\
             Duration: 2391500ms",
        );
        if let Some(pr) = pr_line {
            body.push_str(&format!("\nPR: {pr}"));
        }
        db.update_task_completed(&callback_id, Some(&body))
            .await
            .unwrap();
        (parent_id, callback_id)
    }

    #[tokio::test]
    async fn test_try_complete_parent_on_callback_success_happy_path() {
        let db = test_db();
        let pr_url = "https://github.com/senara-solutions/mika/pull/1160";
        let (parent_id, callback_id) = create_success_callback_pair(&db, Some(pr_url)).await;

        let task = db.get_task_unscoped(&callback_id).await.unwrap().unwrap();
        try_complete_parent_on_callback_success(&db, &task).await;

        let parent = db.get_task_unscoped(&parent_id).await.unwrap().unwrap();
        assert_eq!(parent.status, "completed");
        let result = parent.result.unwrap();
        assert!(
            result.contains("parent_completed_from_callback"),
            "result should carry the audit-grep marker, got: {result}"
        );
        assert!(
            result.contains(pr_url),
            "result should embed the pr_url, got: {result}"
        );
        assert!(parent.completed_at.is_some());

        // R3 — audit event must be written for observability.
        let pid = parent_id.clone();
        let events = db
            .with_db(move |inner| {
                inner.list_audit_events_paginated(
                    "mika",
                    Some("task_engine_parent_completer"),
                    Some(&pid),
                    10,
                    0,
                )
            })
            .await
            .unwrap();
        assert_eq!(events.len(), 1, "exactly one audit event must be written");
        let event = &events[0];
        assert_eq!(event.before_value.as_deref(), Some("in_progress"));
        assert_eq!(event.after_value.as_deref(), Some("completed"));
        let reasoning = event.reasoning.as_deref().unwrap();
        assert!(reasoning.contains("parent_completed_from_callback"));
        assert!(reasoning.contains(pr_url));
        assert!(event.trace_id.is_some(), "audit event must carry trace_id");
    }

    #[tokio::test]
    async fn test_try_complete_parent_on_callback_success_no_pr_url_noop() {
        let db = test_db();
        let (parent_id, callback_id) = create_success_callback_pair(&db, None).await;

        let task = db.get_task_unscoped(&callback_id).await.unwrap().unwrap();
        try_complete_parent_on_callback_success(&db, &task).await;

        let parent = db.get_task_unscoped(&parent_id).await.unwrap().unwrap();
        assert_eq!(
            parent.status, "in_progress",
            "no pr_url means the reaper owns this case, not the completer"
        );
    }

    #[tokio::test]
    async fn test_try_complete_parent_on_callback_success_parent_failed_noop() {
        let db = test_db();
        let pr_url = "https://github.com/x/y/pull/1";
        let (parent_id, callback_id) = create_success_callback_pair(&db, Some(pr_url)).await;

        // Simulate the reaper having marked the parent failed already.
        db.update_task_failed(&parent_id, "callback_delivered_without_pr_url")
            .await
            .unwrap();

        let task = db.get_task_unscoped(&callback_id).await.unwrap().unwrap();
        try_complete_parent_on_callback_success(&db, &task).await;

        let parent = db.get_task_unscoped(&parent_id).await.unwrap().unwrap();
        assert_eq!(
            parent.status, "failed",
            "the retry-promoter (mika#958) owns the failed → completed transition, not this fn"
        );
    }

    #[tokio::test]
    async fn test_try_complete_parent_on_callback_success_parent_already_completed_noop() {
        let db = test_db();
        let pr_url = "https://github.com/x/y/pull/1";
        let (parent_id, callback_id) = create_success_callback_pair(&db, Some(pr_url)).await;

        // Agent's silent turn already completed the parent.
        db.update_task_completed(&parent_id, Some("agent_self_completed"))
            .await
            .unwrap();

        let task = db.get_task_unscoped(&callback_id).await.unwrap().unwrap();
        try_complete_parent_on_callback_success(&db, &task).await;

        let parent = db.get_task_unscoped(&parent_id).await.unwrap().unwrap();
        assert_eq!(parent.status, "completed");
        // Must NOT overwrite the agent's result with the structural-backstop reason.
        assert_eq!(parent.result.as_deref(), Some("agent_self_completed"));
    }

    #[tokio::test]
    async fn test_try_complete_parent_on_callback_success_no_parent_noop() {
        // Callback with no parent_task_id — should be a no-op (no panic).
        let db = test_db();
        let callback = NewTask {
            agent_id: "mika".to_string(),
            team_run_id: None,
            parent_task_id: None,
            depth: 0,
            label: "long_running:run_claude_pilot".to_string(),
            trigger_type: "callback".to_string(),
            cron_expr: None,
            event_source: None,
            event_offset_secs: None,
            condition_expr: None,
            next_fire_at: None,
            timeout_at: None,
            action_type: "resume_agent".to_string(),
            action_config: "{}".to_string(),
            input_context: None,
            created_by_session: None,
            created_trace_id: None,
            reference_url: None,
            source: None,
            metadata: None,
            r#type: None,
            dispatch_class: Some("implement".to_string()),
        };
        let callback_id = db.create_task(callback).await.unwrap();
        db.update_task_completed(&callback_id, Some("PR: https://x/y/pull/1"))
            .await
            .unwrap();

        let task = db.get_task_unscoped(&callback_id).await.unwrap().unwrap();
        // No panic, no DB side effect — fire-and-forget contract.
        try_complete_parent_on_callback_success(&db, &task).await;
    }

    #[tokio::test]
    async fn test_try_complete_parent_on_callback_success_groom_class_noop() {
        let db = test_db();
        let pr_url = "https://github.com/x/y/pull/1";
        let (parent_id, callback_id) = create_success_callback_pair(&db, Some(pr_url)).await;

        // Demote the child to groom-class.
        db.update_task_dispatch_class(&callback_id, "groom")
            .await
            .unwrap();

        let task = db.get_task_unscoped(&callback_id).await.unwrap().unwrap();
        try_complete_parent_on_callback_success(&db, &task).await;

        let parent = db.get_task_unscoped(&parent_id).await.unwrap().unwrap();
        assert_eq!(
            parent.status, "in_progress",
            "groom-class callbacks must not auto-complete their parent"
        );
    }

    #[tokio::test]
    async fn test_try_complete_parent_on_callback_success_parent_cancelled_noop() {
        // mika#1162 plan Unit 2 scenario 5 — cancelled parent must not be
        // resurrected by the auto-completer. The early-return guard
        // (`status == "in_progress"`) catches this before `update_task_completed`
        // would (the DB-level guard only allows `pending` and `in_progress`
        // through). Test both layers of defense by ensuring the result string
        // is not overwritten with the auto-complete reason.
        let db = test_db();
        let pr_url = "https://github.com/x/y/pull/1";
        let (parent_id, callback_id) = create_success_callback_pair(&db, Some(pr_url)).await;

        // Cancel the parent via direct status flip — the cancel_task tool path
        // would do the same thing.
        let pid = parent_id.clone();
        db.with_db(move |inner| {
            inner.conn.execute(
                "UPDATE tasks SET status = 'cancelled', result = 'operator_cancel' WHERE id = ?1",
                rusqlite::params![pid],
            )?;
            Ok(())
        })
        .await
        .unwrap();

        let task = db.get_task_unscoped(&callback_id).await.unwrap().unwrap();
        try_complete_parent_on_callback_success(&db, &task).await;

        let parent = db.get_task_unscoped(&parent_id).await.unwrap().unwrap();
        assert_eq!(parent.status, "cancelled");
        assert_eq!(
            parent.result.as_deref(),
            Some("operator_cancel"),
            "operator's cancel reason must not be overwritten"
        );
    }

    #[tokio::test]
    async fn test_try_complete_parent_on_callback_success_non_self_dev_noop() {
        // Mirror the reaper's source guard: only self_dev parents are in scope.
        let db = test_db();
        let parent = NewTask {
            agent_id: "mika".to_string(),
            team_run_id: None,
            parent_task_id: None,
            depth: 0,
            label: "Non-self_dev parent".to_string(),
            trigger_type: "manual".to_string(),
            cron_expr: None,
            event_source: None,
            event_offset_secs: None,
            condition_expr: None,
            next_fire_at: None,
            timeout_at: None,
            action_type: "none".to_string(),
            action_config: "{}".to_string(),
            input_context: None,
            created_by_session: None,
            created_trace_id: None,
            reference_url: None,
            source: None, // <- not self_dev
            metadata: None,
            r#type: None,
            dispatch_class: Some("implement".to_string()),
        };
        let parent_id = db.create_task(parent).await.unwrap();
        db.update_task_status(&parent_id, "in_progress")
            .await
            .unwrap();

        let callback = NewTask {
            agent_id: "mika".to_string(),
            team_run_id: None,
            parent_task_id: Some(parent_id.clone()),
            depth: 1,
            label: "long_running:run_claude_pilot".to_string(),
            trigger_type: "callback".to_string(),
            cron_expr: None,
            event_source: None,
            event_offset_secs: None,
            condition_expr: None,
            next_fire_at: None,
            timeout_at: None,
            action_type: "resume_agent".to_string(),
            action_config: "{}".to_string(),
            input_context: None,
            created_by_session: None,
            created_trace_id: None,
            reference_url: None,
            source: None,
            metadata: None,
            r#type: None,
            dispatch_class: Some("implement".to_string()),
        };
        let callback_id = db.create_task(callback).await.unwrap();
        db.update_task_completed(&callback_id, Some("PR: https://github.com/x/y/pull/1"))
            .await
            .unwrap();

        let task = db.get_task_unscoped(&callback_id).await.unwrap().unwrap();
        try_complete_parent_on_callback_success(&db, &task).await;

        let parent = db.get_task_unscoped(&parent_id).await.unwrap().unwrap();
        assert_eq!(
            parent.status, "in_progress",
            "non-self_dev parent must not be auto-completed"
        );
    }

    // ---- mika#1614: engine-side auto-fire-after-groom ----

    /// Create a groom-class parent + groom-class callback child. The callback's
    /// result text controls whether `Outcome: PLAN_GROOMED` is present; the
    /// parent carries the GitHub issue reference_url.
    async fn create_groom_callback_pair(
        db: &AsyncDatabase,
        plan_groomed: bool,
        reference_url: Option<&str>,
    ) -> (String, String) {
        let parent = NewTask {
            agent_id: "mika".to_string(),
            team_run_id: None,
            parent_task_id: None,
            depth: 0,
            label: "ready-label: senara-solutions/mika#1614".to_string(),
            trigger_type: "manual".to_string(),
            cron_expr: None,
            event_source: None,
            event_offset_secs: None,
            condition_expr: None,
            next_fire_at: None,
            timeout_at: None,
            action_type: "none".to_string(),
            action_config: "{}".to_string(),
            input_context: None,
            created_by_session: None,
            created_trace_id: None,
            reference_url: reference_url.map(|s| s.to_string()),
            source: Some("self_dev".to_string()),
            metadata: None,
            r#type: Some("issue".to_string()),
            dispatch_class: Some("groom".to_string()),
        };
        let parent_id = db.create_task(parent).await.unwrap();
        db.update_task_status(&parent_id, "in_progress")
            .await
            .unwrap();

        let callback = NewTask {
            agent_id: "mika".to_string(),
            team_run_id: None,
            parent_task_id: Some(parent_id.clone()),
            depth: 1,
            label: "long_running:run_claude_pilot_groom".to_string(),
            trigger_type: "callback".to_string(),
            cron_expr: None,
            event_source: None,
            event_offset_secs: None,
            condition_expr: None,
            next_fire_at: None,
            timeout_at: None,
            action_type: "resume_agent".to_string(),
            action_config: "{}".to_string(),
            input_context: None,
            created_by_session: None,
            created_trace_id: None,
            reference_url: None,
            source: None,
            metadata: None,
            r#type: None,
            dispatch_class: Some("groom".to_string()),
        };
        let callback_id = db.create_task(callback).await.unwrap();
        let body = if plan_groomed {
            "claude-pilot completed (status: done).\nOutcome: PLAN_GROOMED\nSession: sess-1614"
        } else {
            "claude-pilot completed (status: done).\nOutcome: PLAN_ITERATE\nSession: sess-1614"
        };
        db.update_task_completed(&callback_id, Some(body))
            .await
            .unwrap();
        (parent_id, callback_id)
    }

    /// Read the current dispatch_class of a task (None → "<none>").
    async fn dispatch_class_of(db: &AsyncDatabase, task_id: &str) -> String {
        db.get_task_unscoped(task_id)
            .await
            .unwrap()
            .unwrap()
            .dispatch_class
            .unwrap_or_else(|| "<none>".to_string())
    }

    /// Count active (non-terminal) manual tasks with the given reference_url —
    /// the population the `idx_tasks_manual_active_ref_url` unique index covers.
    async fn count_active_manual_with_ref(db: &AsyncDatabase, reference_url: &str) -> usize {
        db.list_manual_tasks(None, None, true)
            .await
            .unwrap()
            .into_iter()
            .filter(|(t, _)| {
                t.reference_url.as_deref() == Some(reference_url)
                    && !matches!(
                        t.status.as_str(),
                        "completed" | "cancelled" | "failed" | "delivered"
                    )
            })
            .count()
    }

    const TEST_ISSUE_URL: &str = "https://github.com/senara-solutions/mika/issues/1614";

    #[tokio::test]
    async fn test_auto_fire_skips_non_groom_class() {
        let db = test_db();
        let (parent_id, callback_id) =
            create_groom_callback_pair(&db, true, Some(TEST_ISSUE_URL)).await;
        // Flip the callback to implement-class → precondition 1 returns early.
        db.update_task_dispatch_class(&callback_id, "implement")
            .await
            .unwrap();
        let task = db.get_task_unscoped(&callback_id).await.unwrap().unwrap();

        try_dispatch_pilot_after_groom_success(
            &db,
            &task,
            Some("ghp_token"),
            &crate::skills::SkillRegistry::empty(),
        )
        .await;

        assert_eq!(
            dispatch_class_of(&db, &parent_id).await,
            "groom",
            "non-groom callback must not flip the parent's dispatch_class"
        );
    }

    #[tokio::test]
    async fn test_auto_fire_skips_without_plan_groomed_marker() {
        let db = test_db();
        let (parent_id, callback_id) =
            create_groom_callback_pair(&db, false, Some(TEST_ISSUE_URL)).await;
        let task = db.get_task_unscoped(&callback_id).await.unwrap().unwrap();

        try_dispatch_pilot_after_groom_success(
            &db,
            &task,
            Some("ghp_token"),
            &crate::skills::SkillRegistry::empty(),
        )
        .await;

        assert_eq!(
            dispatch_class_of(&db, &parent_id).await,
            "groom",
            "groom callback without PLAN_GROOMED must not dispatch"
        );
    }

    /// Périmètre, explicité par mika#2205 : ce test porte sur le **paramètre**
    /// `github_token` de `try_dispatch_pilot_after_groom_success`, pas sur les
    /// deux scans périodiques. Il ne verrouille donc pas le comportement que
    /// mika#2205 corrige, et il reste vrai après ce correctif — les deux scans
    /// sautent toujours quand *aucun* token ne résout, ils le disent simplement
    /// plus fort (AC3).
    ///
    /// Ce que la lecture initiale du plan mika#2205 avait manqué : l'appelant de
    /// cette fonction (l'auto-fire après grooming) passe encore
    /// `self.github_token.as_deref()` et reste donc PAT-seul. C'est un troisième
    /// site de la même classe, laissé hors périmètre à dessein — il mène à une
    /// création de PR, où l'identité lue par GitHub compte au sens d'ADR-008.
    /// Voir `mika2205_periodic_scans_do_not_read_the_pat_field_directly`.
    #[tokio::test]
    async fn test_auto_fire_skips_when_no_token_is_passed() {
        let db = test_db();
        let (parent_id, callback_id) =
            create_groom_callback_pair(&db, true, Some(TEST_ISSUE_URL)).await;
        let task = db.get_task_unscoped(&callback_id).await.unwrap().unwrap();

        // No token → precondition 4 returns before any dispatch state changes.
        try_dispatch_pilot_after_groom_success(
            &db,
            &task,
            None,
            &crate::skills::SkillRegistry::empty(),
        )
        .await;

        assert_eq!(
            dispatch_class_of(&db, &parent_id).await,
            "groom",
            "no github token must short-circuit before the dispatch_class flip"
        );
    }

    #[tokio::test]
    async fn test_auto_fire_skips_when_tool_not_in_registry() {
        let db = test_db();
        let (parent_id, callback_id) =
            create_groom_callback_pair(&db, true, Some(TEST_ISSUE_URL)).await;
        let task = db.get_task_unscoped(&callback_id).await.unwrap().unwrap();

        // Token + PLAN_GROOMED present, but empty registry has no run_claude_pilot
        // tool → step 5a returns BEFORE the dispatch_class flip (5c).
        try_dispatch_pilot_after_groom_success(
            &db,
            &task,
            Some("ghp_token"),
            &crate::skills::SkillRegistry::empty(),
        )
        .await;

        assert_eq!(
            dispatch_class_of(&db, &parent_id).await,
            "groom",
            "missing run_claude_pilot tool must not flip the parent before bailing"
        );
    }

    /// Regression guard for the mika#1614 design decision: the engine MUST reuse
    /// the groom parent (flip its dispatch_class), NOT create a new implement
    /// parent. A new parent would reuse the groom parent's reference_url while the
    /// groom parent is still `in_progress`, colliding with the partial-unique
    /// index `idx_tasks_manual_active_ref_url` — `create_task` errors and the
    /// dispatch silently never fires. This test pins that collision so a future
    /// refactor back to "new parent" fails loudly here.
    #[tokio::test]
    async fn test_new_parent_would_collide_on_active_groom_ref_url() {
        let db = test_db();
        let (_groom_parent, _cb) =
            create_groom_callback_pair(&db, true, Some(TEST_ISSUE_URL)).await;

        // Simulate the rejected "new parent" approach: a second active manual
        // task with the SAME reference_url while the groom parent is in_progress.
        let dup = NewTask {
            agent_id: "mika".to_string(),
            team_run_id: None,
            parent_task_id: None,
            depth: 0,
            label: "auto-fire: mika#1614".to_string(),
            trigger_type: "manual".to_string(),
            cron_expr: None,
            event_source: None,
            event_offset_secs: None,
            condition_expr: None,
            next_fire_at: None,
            timeout_at: None,
            action_type: "none".to_string(),
            action_config: "{}".to_string(),
            input_context: None,
            created_by_session: None,
            created_trace_id: None,
            reference_url: Some(TEST_ISSUE_URL.to_string()),
            source: Some("self_dev".to_string()),
            metadata: None,
            r#type: Some("issue".to_string()),
            dispatch_class: Some("implement".to_string()),
        };
        let res = db.create_task(dup).await;
        assert!(
            res.is_err(),
            "EXPECTED COLLISION on idx_tasks_manual_active_ref_url — a new implement \
             parent cannot coexist with the active groom parent; the engine must REUSE \
             the groom parent instead. got Ok({res:?})"
        );
    }

    /// The chosen fix mechanism: flipping the groom parent's dispatch_class
    /// groom→implement succeeds in-place, keeps a single active task on the issue
    /// URL (no collision, no leak), and the reused parent accepts an
    /// implement-class callback child.
    #[tokio::test]
    async fn test_reuse_flips_groom_parent_in_place_without_collision() {
        let db = test_db();
        let (parent_id, _cb) = create_groom_callback_pair(&db, true, Some(TEST_ISSUE_URL)).await;

        // Exactly one active manual task on the issue URL before the flip.
        assert_eq!(count_active_manual_with_ref(&db, TEST_ISSUE_URL).await, 1);

        let flipped = db
            .update_task_dispatch_class(&parent_id, "implement")
            .await
            .unwrap();
        assert!(flipped, "flip must report the row was updated");
        assert_eq!(
            dispatch_class_of(&db, &parent_id).await,
            "implement",
            "groom parent is reused as the implement parent"
        );
        // Still exactly one active manual task — reuse, not duplication.
        assert_eq!(
            count_active_manual_with_ref(&db, TEST_ISSUE_URL).await,
            1,
            "reuse must not create a second active task on the same reference_url"
        );

        // The reused parent accepts an implement callback child (the dispatch
        // slot the engine then validates + spawns against).
        let dispatch_input = serde_json::json!({
            "skill": "dev-pilot",
            "prompt": "mika#1614",
            "task_id": parent_id,
        });
        let callback = crate::skills::executor::build_callback_task(
            "mika".to_string(),
            Some(parent_id.clone()),
            "run_claude_pilot",
            &dispatch_input,
            7200,
            "system-mika",
            "trace-xyz",
        );
        let cb_id = db
            .create_task(callback)
            .await
            .expect("implement callback child must be creatable on the reused parent");
        let cb = db.get_task_unscoped(&cb_id).await.unwrap().unwrap();
        assert_eq!(cb.dispatch_class.as_deref(), Some("implement"));
        assert_eq!(cb.parent_task_id.as_deref(), Some(parent_id.as_str()));
    }
}
