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
use crate::tools::ToolRegistry;

use super::types::action_type;

/// Executes a task's action by matching on `action_type`.
///
/// `send_message` and `inject_context` are fully implemented.
/// `run_skill` is implemented for "heartbeat" and "reflection" triggers.
pub struct TaskDispatcher {
    pub db: AsyncDatabase,
    pub llm: Arc<dyn LlmProvider>,
    pub tools: Arc<ToolRegistry>,
    pub skills: Arc<SkillRegistry>,
    pub message_sender: Option<Arc<dyn MessageSender>>,
    pub home_dir: PathBuf,
    pub embedding_client: Option<EmbeddingClient>,
    pub brave_api_key: Option<String>,
    pub github_token: Option<String>,
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
            // mika#1289: structural counterpart to the prompt-level groom-
            // success handler in self-dev-callback (PR #1291). When a groom
            // callback delivers with `Outcome: PLAN_GROOMED`, re-add the
            // `ready` label so the ready-label webhook handler dispatches
            // dev-pilot — engine-side, not LLM-mediated, so it fires every
            // time. Prompt-level path remains as defense-in-depth
            // (idempotent — re-adding an already-present label is a no-op).
            try_dispatch_pilot_after_groom_success(&self.db, task, self.github_token.as_deref())
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
            llm: self.llm.as_ref(),
            tools: &self.tools,
            skills: &self.skills,
            trigger,
            home_dir: &self.home_dir,
            session_id: &session_id,
            message_sender,
            embedding_client: self.embedding_client.as_ref(),
            brave_api_key: self.brave_api_key.as_deref(),
            github_token: self.github_token.as_deref(),
            github_app: self.github_app.as_deref(),
            skills_dirty: &self.skills_dirty,
            settings: Some(&self.settings),
            trace_id: Some(trace_id.clone()),
        };

        if let Err(e) = run_silent_agent(&params).await {
            warn!(task_id = %task.id, error = %e, "resume_agent run failed");
        } else if is_callback {
            // Mark delivered so TUI polling doesn't re-process this callback.
            // Only for callbacks — reminder lifecycle is managed by fire_task().
            if let Err(e) = self.db.mark_task_delivered(&task.id).await {
                warn!(task_id = %task.id, error = %e, "failed to mark callback task as delivered");
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
            llm: self.llm.as_ref(),
            tools: &self.tools,
            skills: &self.skills,
            trigger: SilentTrigger::Heartbeat,
            home_dir: &self.home_dir,
            session_id: &session_id,
            message_sender: self.message_sender.clone(),
            embedding_client: self.embedding_client.as_ref(),
            brave_api_key: self.brave_api_key.as_deref(),
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
        let github_token = match self.github_token.as_deref() {
            Some(t) => t,
            None => {
                debug!(task_id = %task.id, "auto_pull: no github_token configured, skipping");
                return Ok(());
            }
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
            llm: self.llm.as_ref(),
            tools: &self.tools,
            skills: &self.skills,
            trigger: SilentTrigger::Reflection,
            home_dir: &self.home_dir,
            session_id: &session_id,
            message_sender: self.message_sender.clone(),
            embedding_client: self.embedding_client.as_ref(),
            brave_api_key: self.brave_api_key.as_deref(),
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
            llm: self.llm.as_ref(),
            tools: &self.tools,
            skills: &self.skills,
            trigger: advance_trigger,
            home_dir: &self.home_dir,
            session_id: &session_id,
            message_sender: self.message_sender.clone(),
            embedding_client: self.embedding_client.as_ref(),
            brave_api_key: self.brave_api_key.as_deref(),
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
/// `gh` subprocess uses the resolved GitHub token from `github_token` (already
/// scrubbed-and-re-injected pattern from `run_gh`). Audit event written under
/// `tool_name='task_engine_groom_pilot_dispatcher'` for traceability.
async fn try_dispatch_pilot_after_groom_success(
    db: &AsyncDatabase,
    task: &Task,
    github_token: Option<&str>,
) {
    // 1. Groom-class callbacks only.
    if task.dispatch_class.as_deref() != Some("groom") {
        return;
    }

    // 2. Canonical success marker in callback result text.
    let result = match &task.result {
        Some(r) if r.contains("Outcome: PLAN_GROOMED") => r,
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
        Some(url) => url,
        None => return,
    };
    let (repo, issue_num) = match parse_repo_issue_from_url(reference_url) {
        Some(parsed) => parsed,
        None => return,
    };

    // 4. GitHub token required.
    let token = match github_token {
        Some(t) if !t.is_empty() => t,
        _ => {
            warn!(
                parent_task_id = %parent_id,
                callback_task_id = %task.id,
                "engine: groom-pilot auto-fire skipped — no GitHub token configured"
            );
            return;
        }
    };

    // 5. Spawn `gh issue edit <n> --repo senara-solutions/<repo> --add-label ready`.
    //    Idempotent: re-adding an already-present label is a no-op on GitHub.
    let mut cmd = tokio::process::Command::new("gh");
    cmd.args([
        "issue",
        "edit",
        &issue_num.to_string(),
        "--repo",
        &format!("senara-solutions/{repo}"),
        "--add-label",
        "ready",
    ]);
    cmd.env("GH_TOKEN", token);
    cmd.env("GH_PROMPT_DISABLED", "1");

    match cmd.output().await {
        Ok(out) if out.status.success() => {
            info!(
                parent_task_id = %parent_id,
                callback_task_id = %task.id,
                repo = %repo,
                issue = issue_num,
                _result_len = result.len(),
                "engine: auto-fired dev-pilot dispatch after groom success (mika#1289 — re-added ready label)"
            );
            let system_session = format!("system-{}", parent.agent_id);
            let trace_id = mika_common::trace::generate_trace_id();
            let reason = format!("groom_pilot_dispatch_fired (issue: {repo}#{issue_num})");
            if let Err(e) = db
                .log_audit_event(
                    &system_session,
                    "task_engine_groom_pilot_dispatcher",
                    &parent_id,
                    Some("groom_delivered"),
                    Some("ready_label_re_added"),
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
        Ok(out) => {
            let stderr = String::from_utf8_lossy(&out.stderr);
            warn!(
                parent_task_id = %parent_id,
                callback_task_id = %task.id,
                repo = %repo,
                issue = issue_num,
                exit_code = ?out.status.code(),
                stderr = %stderr,
                "engine: groom-pilot auto-fire failed — gh issue edit returned non-zero"
            );
        }
        Err(e) => {
            warn!(
                parent_task_id = %parent_id,
                callback_task_id = %task.id,
                error = %e,
                "engine: groom-pilot auto-fire failed — could not spawn gh subprocess"
            );
        }
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

    fn test_db() -> AsyncDatabase {
        let db = Database::open_in_memory().unwrap();
        AsyncDatabase::new_with_agent(db, "mika")
    }

    fn test_dispatcher(db: AsyncDatabase) -> TaskDispatcher {
        let tmp = tempfile::tempdir().unwrap();
        let settings = Settings::load(tmp.path()).unwrap();
        TaskDispatcher {
            db,
            llm: mika_common::llm::dummy_provider(),
            tools: Arc::new(crate::tools::default_tools()),
            skills: Arc::new(crate::skills::SkillRegistry::empty()),
            message_sender: Some(Arc::new(NoopSender)),
            home_dir: PathBuf::from("/tmp"),
            embedding_client: None,
            brave_api_key: None,
            github_token: None,
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
}
