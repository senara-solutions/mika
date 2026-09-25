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
// mika#2515 U2a — les clés de `metadata` que `classify_undelivered_verdict` lit.
// Ré-exportées plutôt que recopiées : l'écrivain possède le nom, le lecteur
// l'importe. Deux orthographes d'une même clé feraient répondre le classificateur
// et le compteur sur deux champs différents, en silence — la classe que
// `grooming_marker` a dû refermer une fois (mika#2158).
pub(crate) use dispatcher::{
    DELIVERY_ATTEMPTS_KEY, DELIVERY_QUARANTINED_AT_KEY, VERDICT_DELIVERY_DEFERRALS_KEY,
};
pub use engine::{TaskEngine, promoted_wrapper_liveness_secs, stuck_pending_reaper_grace_secs};
pub use queue::QueuedTask;
pub use types::{action_type, task_status, trigger_type};

use crate::async_db::AsyncDatabase;
use crate::db::NewTask;
use chrono::Timelike;
use std::path::Path;
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
pub async fn ensure_recurring_task(
    db: &AsyncDatabase,
    label: &str,
    cron_expr: &str,
    action_config: &str,
) {
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

/// Pourquoi `mika tasks rearm` a refusé (mika#2446 R-4/R-5).
///
/// Chaque refus nomme ce qui le fonde : un geste opérateur refusé sans
/// raison lisible est un geste qu'on contourne au jugé — en éditant la base,
/// précisément ce que la commande existe pour rendre inutile.
#[derive(Debug, thiserror::Error)]
pub enum RearmError {
    /// Aucune ligne récurrente morte pour ce label. Jamais de création ex
    /// nihilo : créer une récurrente depuis un label inconnu serait le seul
    /// geste capable d'introduire un trigger non routable.
    #[error(
        "no dead recurring task labelled `{label}` for agent `{agent_id}` — \
         nothing to re-arm (a rearm never creates a recurrence ex nihilo)"
    )]
    NoDeadRow { agent_id: String, label: String },
    /// Une ligne active existe déjà : il n'y a rien à ressusciter.
    #[error("recurring task `{label}` is already armed — nothing to re-arm")]
    AlreadyArmed { label: String },
    /// `ensure_recurring_task` n'enregistre que des `run_skill` ; ré-armer
    /// autre chose passerait par un chemin qui réécrirait l'action.
    #[error(
        "recurring task `{label}` (task {task_id}) has action_type `{action_type}`; \
         `mika tasks rearm` only re-registers run_skill recurrences"
    )]
    NotRunSkill {
        label: String,
        task_id: String,
        action_type: String,
    },
    /// L'`action_config` de la ligne morte ne porte pas de `trigger` lisible :
    /// sans trigger, la routabilité ne peut pas être établie — refus.
    #[error(
        "recurring task `{label}` (task {task_id}) carries no readable `trigger` in its \
         action_config — routability cannot be established, refusing"
    )]
    NoTrigger { label: String, task_id: String },
    /// R-5 / AC7 — le binaire courant ne sait pas router ce trigger.
    /// Ré-armer ne produirait qu'une mort de plus par trigger inconnu.
    #[error(
        "refusing to re-arm `{label}`: trigger `{trigger}` is not routable by this binary \
         (mika {version}, git {git_hash}); routable triggers: {routable_triggers}. \
         Re-arming would only produce another unknown-trigger death — deploy a binary \
         that carries the arm first."
    )]
    NotRoutable {
        label: String,
        trigger: String,
        version: String,
        git_hash: String,
        routable_triggers: String,
    },
    /// La ligne morte n'a pas de `cron_expr` : l'opérateur ne doit pas en
    /// retaper un, donc la commande refuse plutôt que d'en inventer un.
    #[error("recurring task `{label}` (task {task_id}) has no cron_expr to re-register with")]
    NoCron { label: String, task_id: String },
    /// Le marqueur est posé et tracé, mais la ré-inscription n'a pas pris —
    /// une autre garde a refusé ; le journal porte son WARN.
    #[error(
        "re-registration of `{label}` did not take after the operator lift — \
         read the mika#1742 warning in the log"
    )]
    NotRegistered { label: String },
    #[error(transparent)]
    Db(#[from] anyhow::Error),
}

/// Ce qu'un `mika tasks rearm` réussi a fait (mika#2446 R-4).
#[derive(Debug, Clone)]
pub struct RearmOutcome {
    /// Le label tel que stocké (la recherche est `COLLATE NOCASE`).
    pub label: String,
    pub trigger: String,
    pub cron_expr: String,
    /// La ligne morte la plus récente — celle dont la mort est absoute.
    pub dead_task_id: String,
    pub dead_status: String,
    /// Nombre de lignes mortes marquées par cet acte (toutes celles du label).
    pub rows_marked: usize,
}

/// Ré-arme une récurrente morte sans attendre la fenêtre de grâce
/// mika#1742 et sans édition manuelle de la base (mika#2446 R-4/R-5).
///
/// Séquence, et l'ordre est porteur :
/// 1. résoudre la ligne morte la plus récente du label — absente → refus,
///    jamais de création ex nihilo ;
/// 2. lire son `trigger` et **refuser s'il n'est pas routable par ce
///    binaire** (AC7), en nommant le trigger, la version, l'empreinte git et
///    l'inventaire ;
/// 3. **tracer l'acte AVANT de le poser** (`audit_events`,
///    `tool_name = 'recurring_operator_rearm'`) : un acte non tracé est ce que
///    ce geste refuse d'être, donc un audit illisible annule tout ;
/// 4. poser le marqueur sur les lignes mortes du label ;
/// 5. ré-enregistrer via [`ensure_recurring_task`] avec le `cron_expr` et
///    l'`action_config` lus **sur la ligne morte**, puis vérifier qu'une ligne
///    active existe.
///
/// **Refusé : une exemption automatique au second décès.** Ce serait désarmer
/// mika#1742 pour toute la classe. Le ré-armement est un acte explicite,
/// imputable, et chaque décès postérieur retrouve un veto armé.
pub async fn rearm_recurring_task(
    db: &AsyncDatabase,
    label: &str,
) -> Result<RearmOutcome, RearmError> {
    if db.get_recurring_task_cron(label).await?.is_some() {
        return Err(RearmError::AlreadyArmed {
            label: label.to_string(),
        });
    }

    let target = db
        .find_recurring_rearm_target(label)
        .await?
        .ok_or_else(|| RearmError::NoDeadRow {
            agent_id: db.agent_id.clone(),
            label: label.to_string(),
        })?;

    // La recherche est insensible à la casse ; la vérification d'activité
    // ci-dessus lit le label tapé. On la refait sur l'orthographe stockée.
    if target.label != label && db.get_recurring_task_cron(&target.label).await?.is_some() {
        return Err(RearmError::AlreadyArmed {
            label: target.label.clone(),
        });
    }

    if target.action_type != action_type::RUN_SKILL {
        return Err(RearmError::NotRunSkill {
            label: target.label.clone(),
            task_id: target.task_id.clone(),
            action_type: target.action_type.clone(),
        });
    }

    let trigger = serde_json::from_str::<serde_json::Value>(&target.action_config)
        .ok()
        .and_then(|v| v.get("trigger").and_then(|t| t.as_str()).map(str::to_owned))
        .filter(|t| !t.trim().is_empty())
        .ok_or_else(|| RearmError::NoTrigger {
            label: target.label.clone(),
            task_id: target.task_id.clone(),
        })?;

    // AC7 — le lecteur unique de l'inventaire pour les prédicats.
    if !dispatcher::is_routable_trigger(&trigger) {
        let attribution = dispatcher::binary_attribution();
        warn!(
            event = "recurring_operator_rearm_refused",
            label = %target.label,
            trigger = %trigger,
            binary_version = %attribution.version,
            binary_git_hash = %attribution.git_hash,
            routable_triggers = %attribution.routable_triggers,
            "mika#2446: rearm refused — trigger not routable by this binary"
        );
        return Err(RearmError::NotRoutable {
            label: target.label.clone(),
            trigger,
            version: attribution.version.to_string(),
            git_hash: attribution.git_hash.to_string(),
            routable_triggers: attribution.routable_triggers,
        });
    }

    let cron_expr = target
        .cron_expr
        .clone()
        .filter(|c| !c.trim().is_empty())
        .ok_or_else(|| RearmError::NoCron {
            label: target.label.clone(),
            task_id: target.task_id.clone(),
        })?;

    // Tracer AVANT de poser : si la trace ne s'écrit pas, rien n'est changé.
    let attribution = dispatcher::binary_attribution();
    db.log_audit_event(
        &format!("system-{}", db.agent_id()),
        "recurring_operator_rearm",
        &format!("label:{}", target.label),
        Some(&target.status),
        Some("rearmed"),
        Some(&format!(
            "dead_task:{} trigger:{} cron:{} dead_updated_at:{} \
             binary_version:{} binary_git_hash:{}",
            target.task_id,
            trigger,
            cron_expr,
            target.updated_at,
            attribution.version,
            attribution.git_hash,
        )),
        None,
    )
    .await?;

    let rows_marked = db.mark_recurring_operator_rearm(&target.label).await?;

    ensure_recurring_task(db, &target.label, &cron_expr, &target.action_config).await;

    if db.get_recurring_task_cron(&target.label).await?.is_none() {
        return Err(RearmError::NotRegistered {
            label: target.label.clone(),
        });
    }

    info!(
        event = "recurring_operator_rearm",
        label = %target.label,
        trigger = %trigger,
        cron = %cron_expr,
        dead_task_id = %target.task_id,
        dead_status = %target.status,
        rows_marked,
        "mika#2446: recurring task re-armed by operator"
    );

    Ok(RearmOutcome {
        label: target.label,
        trigger,
        cron_expr,
        dead_task_id: target.task_id,
        dead_status: target.status,
        rows_marked,
    })
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

        // Boot 1 — knob absent : le feeder s'inscrit.
        ensure_recurring_task(&db, FEEDER_LABEL, FEEDER_CRON, FEEDER_CONFIG).await;
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
        ensure_recurring_task(&db, FEEDER_LABEL, FEEDER_CRON, FEEDER_CONFIG).await;

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
        ensure_recurring_task(&db, FEEDER_LABEL, FEEDER_CRON, FEEDER_CONFIG).await;

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

        ensure_recurring_task(&db, FEEDER_LABEL, FEEDER_CRON, FEEDER_CONFIG).await;

        let statuses = statuses_for(&db, FEEDER_LABEL).await;
        assert_eq!(
            statuses,
            vec!["failed".to_string()],
            "un échec terminal récent doit toujours bloquer la ré-inscription \
             (mika#1742) — statuts observés : {statuses:?}"
        );
    }

    // ── mika#2446 — `mika tasks rearm <label>` (AC6 / AC7) ──────────────

    const REAP_LABEL: &str = "worktree_reap";
    const REAP_CRON: &str = "0 */10 * * * *";
    const REAP_CONFIG: &str = r#"{"trigger":"worktree_reap"}"#;

    /// Inscrit `label` puis le fait mourir `failed` dans la fenêtre de grâce,
    /// de cause quelconque (non marquée) — l'état qui arme le veto mika#1742.
    async fn kill_in_window(db: &AsyncDatabase, label: &str, cron: &str, config: &str) {
        ensure_recurring_task(db, label, cron, config).await;
        let label = label.to_string();
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
    }

    /// AC6 — le ré-armement lève le veto sans attendre 24 h ni éditer la
    /// base, ré-inscrit avec le cron lu sur la ligne morte, et trace l'acte.
    #[tokio::test]
    async fn mika2446_rearm_revives_a_dead_recurrence_and_traces_the_act() {
        let db = test_async_db();
        kill_in_window(&db, REAP_LABEL, REAP_CRON, REAP_CONFIG).await;

        // Précondition : le chemin nominal (redémarrage) est refusé par le veto.
        ensure_recurring_task(&db, REAP_LABEL, REAP_CRON, REAP_CONFIG).await;
        assert_eq!(
            statuses_for(&db, REAP_LABEL).await,
            vec!["failed".to_string()]
        );

        let outcome = rearm_recurring_task(&db, REAP_LABEL)
            .await
            .expect("un trigger routable doit être ré-armé");
        assert_eq!(outcome.label, REAP_LABEL);
        assert_eq!(outcome.trigger, "worktree_reap");
        assert_eq!(outcome.cron_expr, REAP_CRON);
        assert_eq!(outcome.dead_status, "failed");
        assert_eq!(outcome.rows_marked, 1);

        let statuses = statuses_for(&db, REAP_LABEL).await;
        assert!(
            statuses.iter().any(|s| s == "recurring_active"),
            "la récurrente doit être ré-inscrite — statuts : {statuses:?}"
        );
        assert_eq!(
            db.get_recurring_task_cron(REAP_LABEL)
                .await
                .unwrap()
                .as_deref(),
            Some(REAP_CRON),
            "le cron est lu sur la ligne morte, jamais retapé"
        );
        assert_eq!(
            db.count_audit_events_by_tool_name("recurring_operator_rearm")
                .await
                .unwrap(),
            1,
            "l'acte doit être tracé dans audit_events"
        );
    }

    /// AC7 — un trigger que ce binaire ne sait pas router est refusé, en
    /// nommant le trigger et l'inventaire ; rien n'est tracé, marqué ni
    /// ré-inscrit (ré-armer ne produirait qu'une mort de plus).
    #[tokio::test]
    async fn mika2446_rearm_refuses_a_trigger_this_binary_cannot_route() {
        let db = test_async_db();
        kill_in_window(&db, "zorglub_scan", REAP_CRON, r#"{"trigger":"zorglub"}"#).await;

        let err = rearm_recurring_task(&db, "zorglub_scan")
            .await
            .expect_err("un trigger non routable doit être refusé");
        match &err {
            RearmError::NotRoutable {
                trigger,
                routable_triggers,
                ..
            } => {
                assert_eq!(trigger, "zorglub");
                assert_eq!(routable_triggers, &dispatcher::routable_triggers_csv());
            }
            other => panic!("attendu NotRoutable, obtenu {other:?}"),
        }
        let rendered = err.to_string();
        assert!(rendered.contains("zorglub"), "le refus nomme le trigger");
        assert!(
            rendered.contains("worktree_reap"),
            "le refus nomme l'inventaire routable : {rendered}"
        );

        assert_eq!(
            statuses_for(&db, "zorglub_scan").await,
            vec!["failed".to_string()]
        );
        assert_eq!(
            db.count_audit_events_by_tool_name("recurring_operator_rearm")
                .await
                .unwrap(),
            0,
            "un refus ne trace pas d'acte"
        );
        // Aucun marqueur posé : le chemin nominal reste refusé par le veto.
        ensure_recurring_task(&db, "zorglub_scan", REAP_CRON, r#"{"trigger":"zorglub"}"#).await;
        assert_eq!(
            statuses_for(&db, "zorglub_scan").await,
            vec!["failed".to_string()]
        );
    }

    /// Jamais de création ex nihilo, et rien à ré-armer sur une ligne vivante.
    #[tokio::test]
    async fn mika2446_rearm_refuses_unknown_and_already_armed_labels() {
        let db = test_async_db();
        assert!(matches!(
            rearm_recurring_task(&db, REAP_LABEL).await,
            Err(RearmError::NoDeadRow { .. })
        ));
        assert!(statuses_for(&db, REAP_LABEL).await.is_empty());

        ensure_recurring_task(&db, REAP_LABEL, REAP_CRON, REAP_CONFIG).await;
        assert!(matches!(
            rearm_recurring_task(&db, REAP_LABEL).await,
            Err(RearmError::AlreadyArmed { .. })
        ));
    }
}
