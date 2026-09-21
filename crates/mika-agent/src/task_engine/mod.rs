pub mod cron;
pub mod dispatcher;
pub mod engine;
pub mod liveness;
pub mod pilot_transcript;
pub mod process_kill;
pub mod process_liveness;
pub mod queue;
pub mod types;
pub mod worktree_activity;

pub use dispatcher::{DispatchError, TaskDispatcher};
pub use engine::{TaskEngine, promoted_wrapper_liveness_secs, stuck_pending_reaper_grace_secs};
pub use queue::QueuedTask;
pub use types::{action_type, task_status, trigger_type};

use crate::async_db::AsyncDatabase;
use crate::db::NewTask;
use chrono::Timelike;
use std::collections::HashSet;
use std::path::Path;
use std::sync::{Mutex, OnceLock};
use tracing::{debug, info, warn};

/// Prune completed/failed/cancelled/expired tasks older than 30 days at startup
/// to prevent unbounded DB growth.
pub async fn prune_old_tasks(db: &AsyncDatabase) {
    // 30 days in seconds
    const THIRTY_DAYS_SECS: i64 = 30 * 24 * 60 * 60;
    if let Err(e) = db.prune_completed_tasks(THIRTY_DAYS_SECS).await {
        warn!("Failed to prune completed tasks: {}", e);
    }
}

/// Structured log event naming the resolved state of the agent-level activity
/// gate and its provenance (mika#2456).
///
/// Answers *"is this agent disabled, and through which door?"* without reading
/// the disk. Same shape and same reason as `llm_budget_resolved` (mika#2293) and
/// `tenant_language_resolved` (mika#2247).
pub const AGENT_GATE_RESOLVED_EVENT: &str = "agent_recurring_gate_resolved";

/// Structured log event for a registration the gate refused (mika#2456).
///
/// **Expected regime: non-empty on the first startup after the knob is posted**
/// (one line per recurrence cancelled), then a handful per startup. Without this
/// line a gate that bites reads exactly like an inert one — the class mika#2205
/// had to close.
pub const REGISTRATION_REFUSED_EVENT: &str = "recurring_registration_refused_agent_disabled";

/// When each `(agent, enabled, source)` triple was last announced by this
/// process. A repetition is silent; a **change** is re-emitted.
static GATE_RESOLVED_SEEN: OnceLock<Mutex<HashSet<(String, bool, &'static str)>>> = OnceLock::new();

/// Whether this agent may register and fire recurring automatic turns, plus the
/// provenance of that answer (mika#2456).
///
/// Sits next to [`heartbeat_enabled_for_agent`] because it is the same kind of
/// question one level up: that one gates a single feature, this one gates the
/// agent. The two **compose by conjunction** (§ 3.4) — `[heartbeat] enabled =
/// false` remains in force and is neither removed nor deprecated.
pub async fn agent_recurring_tasks_enabled(home_dir: &Path) -> (bool, crate::prompt::GateSource) {
    crate::prompt::load_identity_async(home_dir)
        .await
        .recurring_tasks_enabled()
}

/// Emit [`AGENT_GATE_RESOLVED_EVENT`] once per resolved state per agent.
///
/// Deduplicated rather than per-call: this runs nine times per startup per
/// agent, and nine identical lines would be the churn mika#2131 bounds. A
/// **change** of resolved state is re-emitted, which is what makes "the operator
/// posted the knob" readable on one line.
fn announce_gate_state(agent_id: &str, enabled: bool, source: crate::prompt::GateSource) {
    let key = (agent_id.to_string(), enabled, source.as_str());
    {
        let mut seen = GATE_RESOLVED_SEEN
            .get_or_init(|| Mutex::new(HashSet::new()))
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if !seen.insert(key) {
            return;
        }
    }
    info!(
        event = AGENT_GATE_RESOLVED_EVENT,
        agent_id,
        enabled,
        source = source.as_str(),
        "agent-level recurring-task gate resolved"
    );
}

/// Register a recurring task in the DB if one with the same label doesn't already exist.
/// If it already exists but the cron expression differs, update the cron and recompute
/// the next fire time.
///
/// Used at startup to ensure built-in tasks (heartbeat, reflection) are always registered.
///
/// **Calling this function *is* the config declaring the task must run** — the
/// callers are the boot paths that already evaluated the knob or the
/// `identity.toml` toggle. So a prior *config-driven* cancel of the same label
/// (the knob-off boot cancelled the row) must not survive as a veto: mika#2271
/// reverts it before re-registering. Terminal failures (`failed` / `expired`)
/// keep blocking through the mika#1742 refuse-to-zombie guard — only the
/// deliberate `cancelled` state is cleared here.
///
/// # The agent-level gate lives HERE, not at the callers (mika#2456)
///
/// The reflex is to wrap the nine registration sites in an `if agent_enabled`.
/// Two measurements refuse it.
///
/// **(a) This function resurrects what such a guard would have cancelled.** The
/// `revert_config_cancel_recurring_task` call below exists to lift the mika#1742
/// veto on a row a knob cancelled (mika#2271). So a single unguarded caller —
/// the CLI site in `chat.rs`, or any future one — **reopens** the row the boot
/// just cancelled. A guard spread over N sites, one of which lifts the others'
/// veto, is not a guard; it is a race.
///
/// **(b) Nine sites, and nothing reddens on the tenth.** A site added without
/// the guard makes no decision wrong: it makes the gate inert, with every test
/// green. That is the exact shape mika#2205 had to close.
///
/// So `home_dir` is a **parameter, never derived**: the nine callers already
/// hold one (`agent_state.home_dir`, `ctx.home_dir`), and a caller that forgets
/// it does not compile. The compiler, not a reviewer, is what forces a future
/// site to take the decision — the `dispatch_substrate_diagnostic` motif, and
/// `mika2334_every_scan_variant_is_covered`'s.
///
/// When the agent is disabled this function does **not** call the revert, does
/// **not** create the row, and **cancels** any existing one — which is what
/// makes the ticket's negative test (*"aucune tâche récurrente
/// `recurring_active`"*) true.
pub async fn ensure_recurring_task(
    db: &AsyncDatabase,
    home_dir: &Path,
    label: &str,
    cron_expr: &str,
    action_config: &str,
) {
    let (enabled, source) = agent_recurring_tasks_enabled(home_dir).await;
    announce_gate_state(&db.agent_id, enabled, source);
    if !enabled {
        // Deliberately BEFORE `revert_config_cancel_recurring_task`: lifting the
        // veto and then cancelling again would leave the row's history saying
        // the opposite of what happened, and would make a concurrent caller's
        // registration land in the window between the two writes.
        let cancelled_rows = match db.cancel_recurring_task_by_label(label).await {
            Ok(n) => n,
            Err(e) => {
                warn!(label, error = %e, "failed to cancel recurring task for disabled agent");
                0
            }
        };
        info!(
            event = REGISTRATION_REFUSED_EVENT,
            agent_id = %db.agent_id,
            label,
            cancelled_rows,
            "agent is disabled (identity.toml `enabled = false`) — recurring task \
             not registered"
        );
        return;
    }

    // mika#2271: knob-off cancelled this label; the caller now says it must run.
    // Clear the config-cancel veto so the mika#1742 guard doesn't refuse the
    // re-registration below.
    match db.revert_config_cancel_recurring_task(label).await {
        Ok(0) => {}
        Ok(n) => {
            info!(
                label,
                rows = n,
                "reverted config cancel on recurring task (mika#2271)"
            )
        }
        Err(e) => warn!(label, error = %e, "failed to revert config cancel on recurring task"),
    }

    let agent_id = db.agent_id.clone();
    let task = NewTask {
        agent_id,
        team_run_id: None,
        parent_task_id: None,
        depth: 0,
        label: label.to_string(),
        trigger_type: "recurring".to_string(),
        cron_expr: Some(cron_expr.to_string()),
        event_source: None,
        event_offset_secs: None,
        condition_expr: None,
        next_fire_at: None,
        timeout_at: None,
        action_type: action_type::RUN_SKILL.to_string(),
        action_config: action_config.to_string(),
        input_context: None,
        created_by_session: None,
        created_trace_id: None,
        reference_url: None,
        source: None,
        metadata: None,
        r#type: None,
        dispatch_class: None,
    };

    match db.create_recurring_task_if_absent(task).await {
        Ok(Some(id)) => info!(label, task_id = %id, cron = cron_expr, "registered recurring task"),
        Ok(None) => {
            // Task already exists — check if the cron expression changed.
            if let Ok(Some(existing_cron)) = db.get_recurring_task_cron(label).await {
                if existing_cron != cron_expr {
                    let now = crate::timestamp::now();
                    match cron::next_fire_from_cron(cron_expr, &now) {
                        Ok(next_fire) => {
                            match db
                                .update_recurring_task_cron(label, cron_expr, &next_fire)
                                .await
                            {
                                Ok(_) => {
                                    info!(label, old_cron = %existing_cron, new_cron = cron_expr, "updated recurring task cron")
                                }
                                Err(e) => {
                                    warn!(label, error = %e, "failed to update recurring task cron")
                                }
                            }
                        }
                        Err(e) => {
                            warn!(label, cron = cron_expr, error = %e, "failed to compute next fire time for updated cron")
                        }
                    }
                } else {
                    debug!(label, "recurring task already registered, skipping");
                }
            }
        }
        Err(e) => warn!(label, error = %e, "failed to register recurring task"),
    }
}

/// Check if heartbeat is enabled for the agent from identity.toml config.
/// Returns `true` (default) unless `[heartbeat] enabled = false`.
pub async fn heartbeat_enabled_for_agent(home_dir: &Path) -> bool {
    let identity = crate::prompt::load_identity_async(home_dir).await;
    identity
        .heartbeat
        .as_ref()
        .map(|c| c.enabled)
        .unwrap_or(true)
}

/// Build a UTC cron expression for reflection from identity.toml config + customer timezone.
/// Returns `None` if reflection is disabled or not configured.
pub async fn reflection_cron_for_agent(home_dir: &Path, db: &AsyncDatabase) -> Option<String> {
    let identity = crate::prompt::load_identity_async(home_dir).await;
    let config = identity.reflection.as_ref().filter(|c| c.enabled)?;
    let local_time = config.parse_time()?;

    let tz_str = if let Some(ref tz) = config.timezone {
        tz.clone()
    } else {
        db.get_customer_config("timezone")
            .await
            .ok()
            .flatten()
            .unwrap_or_else(|| "UTC".to_string())
    };
    let tz: chrono_tz::Tz = match tz_str.parse() {
        Ok(tz) => tz,
        Err(_) => {
            warn!(timezone = %tz_str, "invalid timezone in customer config, skipping reflection registration");
            return None;
        }
    };

    // Convert local time to UTC: pick today's date, attach the local time,
    // convert to UTC, extract hour/minute.
    // NOTE: DST drift — the UTC offset is computed from today's date. For timezones with
    // daylight saving time, the reflection may fire ~1 hour early or late after a DST
    // transition until the next restart. This is acceptable for daily reflections.
    let today = chrono::Utc::now().with_timezone(&tz).date_naive();
    let local_dt = today.and_time(local_time);
    let utc_dt = local_dt.and_local_timezone(tz).earliest()?;
    let utc_time = utc_dt.with_timezone(&chrono::Utc).time();

    Some(format!("0 {} {} * * *", utc_time.minute(), utc_time.hour()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;

    const FEEDER_LABEL: &str = "auto_pull_groomed";
    const FEEDER_CRON: &str = "0 */20 * * * *";
    const FEEDER_CONFIG: &str = r#"{"trigger":"auto_pull_groomed"}"#;

    fn test_async_db() -> AsyncDatabase {
        AsyncDatabase::new(Database::open_in_memory().unwrap())
    }

    /// An agent home whose `identity.toml` carries exactly `body`.
    ///
    /// The `TempDir` is returned so the caller keeps it alive: dropping it
    /// deletes the directory, and an absent `identity.toml` resolves through
    /// `fail_closed_identity()` — whose `enabled` is `None`, hence `true`. A
    /// test that let the dir drop would therefore silently measure the enabled
    /// path while believing it measured the disabled one.
    fn agent_home(body: &str) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("identity.toml"), body).unwrap();
        dir
    }

    /// An identity that says nothing about `enabled` (the shape every deployed
    /// agent carries today).
    fn enabled_home() -> tempfile::TempDir {
        agent_home("name = \"Mika\"\nemoji = \"✦\"\n")
    }

    /// An identity carrying the knob this ticket creates.
    fn disabled_home() -> tempfile::TempDir {
        agent_home("name = \"Mika\"\nemoji = \"✦\"\nenabled = false\n")
    }

    async fn statuses_for(db: &AsyncDatabase, label: &str) -> Vec<String> {
        db.get_tasks_by_status(vec![
            "recurring_active".to_string(),
            "pending".to_string(),
            "in_progress".to_string(),
            "cancelled".to_string(),
            "failed".to_string(),
            "expired".to_string(),
        ])
        .await
        .unwrap()
        .into_iter()
        .filter(|t| t.label == label)
        .map(|t| t.status)
        .collect()
    }

    /// **Porte mika#2271 — test négatif.** Invariant : *un cycle knob-off →
    /// knob-on ré-inscrit le feeder*. Le knob-off boot annule la task
    /// récurrente ; le knob-on boot rappelle `ensure_recurring_task`, ce qui
    /// **est** la config déclarant que la task doit tourner. La garde
    /// refuse-to-zombie (mika#1742) ne doit pas transformer ce cancel
    /// délibéré en veto permanent.
    ///
    /// Sans le fix, le second `ensure_recurring_task` est refusé par la garde
    /// et le seul statut restant est `cancelled` — la boucle n'est plus
    /// réalimentée (symptôme mesuré le 2026-09-09).
    #[tokio::test]
    async fn knob_off_then_on_reregisters_the_feeder() {
        let db = test_async_db();
        let home = enabled_home();

        // Boot 1 — knob absent : le feeder s'inscrit.
        ensure_recurring_task(&db, home.path(), FEEDER_LABEL, FEEDER_CRON, FEEDER_CONFIG).await;
        assert_eq!(
            statuses_for(&db, FEEDER_LABEL).await,
            vec!["recurring_active".to_string()],
            "boot initial : le feeder doit être inscrit"
        );

        // Boot 2 — MIKA_DEV_AUTO_PULL=0 : la branche knob-off annule la row.
        db.cancel_recurring_task_by_label(FEEDER_LABEL)
            .await
            .unwrap();
        assert_eq!(
            statuses_for(&db, FEEDER_LABEL).await,
            vec!["cancelled".to_string()],
            "knob-off : la row doit être annulée"
        );

        // Boot 3 — knob retiré : le feeder doit revenir.
        ensure_recurring_task(&db, home.path(), FEEDER_LABEL, FEEDER_CRON, FEEDER_CONFIG).await;

        let statuses = statuses_for(&db, FEEDER_LABEL).await;
        assert!(
            statuses.iter().any(|s| s == "recurring_active"),
            "knob-on : le feeder doit être RÉ-INSCRIT (recurring_active), \
             pas laissé cancelled — statuts observés : {statuses:?}"
        );
    }

    /// Contrôle positif de la garde : un `failed` récent bloque toujours la
    /// ré-inscription. L'exemption mika#2271 ne vise que le cancel de config —
    /// elle ne doit pas désarmer la protection anti-zombie de mika#1742.
    #[tokio::test]
    async fn recent_failed_still_blocks_reregistration() {
        let db = test_async_db();
        let home = enabled_home();
        ensure_recurring_task(&db, home.path(), FEEDER_LABEL, FEEDER_CRON, FEEDER_CONFIG).await;

        let label = FEEDER_LABEL.to_string();
        db.with_db(move |d| {
            d.conn.execute(
                "UPDATE tasks SET status = 'failed',
                 updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now', '-1 hour')
                 WHERE label = ?1",
                rusqlite::params![label],
            )?;
            Ok(())
        })
        .await
        .unwrap();

        ensure_recurring_task(&db, home.path(), FEEDER_LABEL, FEEDER_CRON, FEEDER_CONFIG).await;

        let statuses = statuses_for(&db, FEEDER_LABEL).await;
        assert_eq!(
            statuses,
            vec!["failed".to_string()],
            "un échec terminal récent doit toujours bloquer la ré-inscription \
             (mika#1742) — statuts observés : {statuses:?}"
        );
    }

    // ---------------------------------------------------------------------
    // mika#2456 — la porte `enabled` niveau agent
    // ---------------------------------------------------------------------

    /// **V1 — le test négatif littéral du ticket.** `enabled = false` :
    /// `ensure_recurring_task` ne crée rien, et la row préexistante devient
    /// `cancelled`.
    #[tokio::test]
    async fn mika2456_v1_disabled_agent_registers_nothing_and_cancels_the_existing_row() {
        let db = test_async_db();

        // Une row née avant que la clé soit posée.
        let enabled = enabled_home();
        ensure_recurring_task(&db, enabled.path(), "heartbeat", "0 0 * * * *", "{}").await;
        assert_eq!(
            statuses_for(&db, "heartbeat").await,
            vec!["recurring_active".to_string()]
        );

        // L'opérateur pose `enabled = false`, puis on redémarre.
        let disabled = disabled_home();
        ensure_recurring_task(&db, disabled.path(), "heartbeat", "0 0 * * * *", "{}").await;

        assert_eq!(
            statuses_for(&db, "heartbeat").await,
            vec!["cancelled".to_string()],
            "un agent désactivé ne doit porter AUCUNE récurrente active"
        );
    }

    /// **V2 — contrôle négatif, porteur.** `enabled = true` et clé **absente**
    /// créent la row normalement.
    ///
    /// Sans lui, V1 ne distingue pas « la garde mord » de « la fonction est
    /// cassée ».
    #[tokio::test]
    async fn mika2456_v2_enabled_and_absent_both_register_normally() {
        for (body, what) in [
            ("name = \"Mika\"\n", "clé absente"),
            ("name = \"Mika\"\nenabled = true\n", "enabled = true"),
        ] {
            let db = test_async_db();
            let home = agent_home(body);
            ensure_recurring_task(&db, home.path(), "heartbeat", "0 0 * * * *", "{}").await;
            assert_eq!(
                statuses_for(&db, "heartbeat").await,
                vec!["recurring_active".to_string()],
                "{what} : la récurrente doit être inscrite"
            );
        }
    }

    /// **V3 — anti-résurrection, et le cœur du § 2.1(a).**
    ///
    /// Un second appel sur un agent désactivé laisse la row `cancelled` :
    /// `revert_config_cancel_recurring_task` n'a PAS été appelé. C'est le
    /// défaut mika#2271 retourné — la fonction qui ressuscite est celle-là même
    /// qu'on garde, ce qui est la raison pour laquelle la garde y vit plutôt
    /// que chez ses neuf appelants.
    #[tokio::test]
    async fn mika2456_v3_a_disabled_agent_never_resurrects_a_cancelled_row() {
        let db = test_async_db();
        let disabled = disabled_home();

        ensure_recurring_task(&db, disabled.path(), "heartbeat", "0 0 * * * *", "{}").await;
        ensure_recurring_task(&db, disabled.path(), "heartbeat", "0 0 * * * *", "{}").await;

        let statuses = statuses_for(&db, "heartbeat").await;
        assert!(
            statuses.iter().all(|s| s == "cancelled") && !statuses.is_empty()
                || statuses.is_empty(),
            "un agent désactivé ne doit jamais ressusciter une row annulée — \
             statuts observés : {statuses:?}"
        );
        assert!(
            !statuses.iter().any(|s| s == "recurring_active"),
            "aucune row ne doit être `recurring_active` — {statuses:?}"
        );
    }

    /// **V6 — conjonction (§ 3.4).** `enabled = true` + `[heartbeat] enabled =
    /// false` : le heartbeat reste refusé par son knob par-feature, que ce
    /// ticket ne retire ni ne déprécie.
    #[tokio::test]
    async fn mika2456_v6_the_per_feature_knob_still_holds_under_an_enabled_agent() {
        let home = agent_home("name = \"Mika\"\nenabled = true\n\n[heartbeat]\nenabled = false\n");
        assert!(
            !heartbeat_enabled_for_agent(home.path()).await,
            "`[heartbeat] enabled = false` doit rester en vigueur"
        );
        let (agent_enabled, source) = agent_recurring_tasks_enabled(home.path()).await;
        assert!(agent_enabled, "l'agent lui-même est actif");
        assert_eq!(source, crate::prompt::GateSource::Identity);
    }

    /// **V7 — fail-closed (§ 3.3).** `identity.toml` absent ⇒ `enabled` résolu
    /// à `true`, et les récurrentes sont enregistrées.
    ///
    /// Élargir la sévérité du fail-closed dans un correctif de coût ferait
    /// qu'une erreur I/O transitoire couperait les messages proactifs d'un
    /// tenant famille. Question nommée, renvoyée à son ticket.
    #[tokio::test]
    async fn mika2456_v7_a_fail_closed_identity_keeps_the_agent_enabled() {
        let db = test_async_db();
        let empty = tempfile::tempdir().unwrap(); // pas d'identity.toml

        let (enabled, source) = agent_recurring_tasks_enabled(empty.path()).await;
        assert!(enabled, "le chemin fail-closed garde `enabled = true`");
        assert_eq!(
            source,
            crate::prompt::GateSource::Default,
            "aucun fichier n'a été lu : la provenance honnête est `default`"
        );

        ensure_recurring_task(&db, empty.path(), "heartbeat", "0 0 * * * *", "{}").await;
        assert_eq!(
            statuses_for(&db, "heartbeat").await,
            vec!["recurring_active".to_string()]
        );
    }

    /// La provenance sépare les deux remèdes de la sonde (AC6) : `identity`
    /// ⇒ la clé est en vigueur ; `default` ⇒ elle n'a pas atterri.
    #[tokio::test]
    async fn mika2456_provenance_distinguishes_a_posed_key_from_an_absent_one() {
        let posed = agent_home("name = \"Mika\"\nenabled = false\n");
        assert_eq!(
            agent_recurring_tasks_enabled(posed.path()).await,
            (false, crate::prompt::GateSource::Identity)
        );

        let absent = agent_home("name = \"Mika\"\n");
        assert_eq!(
            agent_recurring_tasks_enabled(absent.path()).await,
            (true, crate::prompt::GateSource::Default)
        );
    }
}
