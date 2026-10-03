use anyhow::Result;
use async_trait::async_trait;
use mika_common::claude::ToolDefinition;
use serde_json::Value;

use super::{Tool, ToolContext, ToolOutput};
use crate::live_pilot::{
    LivePilotVerdict, UNREADABLE_DB_ERROR, UNREADABLE_NO_START_TIME, live_pilot_for_task,
};

// ───────────── Télémétrie de la garde (mika#2653 § 6, AC7) ─────────────

/// Le `tool_name` sous lequel chaque décision de la garde est auditée.
///
/// **Un seul nom**, la décision dans `after_value` — motif `ready_label_outcome`
/// (mika#2323) : les quatre issues appartiennent au **même site** et à la
/// **même population**, donc un `GROUP BY after_value` les sépare et les rend
/// soustractibles. Délibérément **pas** le motif à deux noms de
/// `phantom_aged_out` / `phantom_sweep_spared` (mika#2156), qui s'applique
/// quand chaque nom porte sa propre cause.
///
/// **SOLE WRITER**, épinglé par
/// `canonical_tokens::tests::mika2653_le_nom_daudit_a_un_seul_ecrivain` — c'est
/// cette propriété qui rend le compte exact plutôt qu'un nombre sur lequel deux
/// sites peuvent diverger.
pub(crate) const CANCEL_PILOT_GUARD_AUDIT_TOOL: &str = "cancel_task_pilot_guard";

/// Un pilote **vif** travaillait sous cette tâche : l'annulation est refusée.
pub(crate) const AUDIT_BLOCKED_LIVE_PILOT: &str = "blocked_live_pilot";
/// La base n'a pas répondu : la question n'a pas pu être posée.
pub(crate) const AUDIT_BLOCKED_DB_UNREADABLE: &str = "blocked_db_unreadable";
/// Un pgid est là mais son instance n'est pas prouvable (population
/// `unusable_child_count`, mika#2335).
pub(crate) const AUDIT_BLOCKED_START_TIME_UNREADABLE: &str = "blocked_start_time_unreadable";
/// **Le contrôle positif** : tour webhook PR, aucun pilote, l'annulation passe.
pub(crate) const AUDIT_ALLOWED_NO_PILOT: &str = "allowed_no_pilot";

/// Toutes les valeurs que `after_value` peut prendre, pour l'épinglage du
/// format de fil (détecteur 3).
///
/// Reste en production plutôt que derrière `#[cfg(test)]`, et c'est la raison
/// qu'`ALL_MARKER_CLASSES` a déjà dû écrire : le registre d'un format de fil est
/// ce qu'un opérateur lit pour savoir ce qu'un `GROUP BY` peut rendre, et un
/// registre qui n'existe que sous `cfg(test)` est un registre qu'un lecteur de
/// ce fichier ne trouve pas.
#[allow(dead_code)]
pub(crate) const ALL_CANCEL_PILOT_GUARD_VERDICTS: &[&str] = &[
    AUDIT_BLOCKED_LIVE_PILOT,
    AUDIT_BLOCKED_DB_UNREADABLE,
    AUDIT_BLOCKED_START_TIME_UNREADABLE,
    AUDIT_ALLOWED_NO_PILOT,
];

/// Motif d'erreur rendu au modèle quand un pilote vif porte le travail.
///
/// Les valeurs du champ `error` sont un **format de fil** elles aussi : elles
/// atterrissent dans `tool_calls.output`, où un opérateur les groupe (voir la
/// requête SQL du § 11.2 du plan). Deux orthographes d'un même motif
/// couperaient une population en deux sans le dire.
pub(crate) const ERROR_CANCEL_LIVE_PILOT: &str = "cancel_refused_live_pilot";
/// Motif d'erreur rendu au modèle quand la vivacité n'a pas pu être établie.
pub(crate) const ERROR_CANCEL_PILOT_UNREADABLE: &str = "cancel_refused_pilot_unreadable";

/// Tous les motifs du champ `error`, pour l'épinglage du format de fil.
#[allow(dead_code)]
pub(crate) const ALL_CANCEL_PILOT_GUARD_ERRORS: &[&str] =
    &[ERROR_CANCEL_LIVE_PILOT, ERROR_CANCEL_PILOT_UNREADABLE];

/// Ce que la garde a décidé pour cet appel à `cancel_task`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CancelPilotDecision {
    /// Hors tour webhook PR — **aucune ligne, aucun audit**. C'est l'essentiel
    /// du trafic, soit très exactement le churn que la doctrine mika#2131
    /// borne, et c'est la garantie de non-régression d'AC3.
    Allowed,
    /// Tour webhook PR, aucun pilote : l'annulation passe, et on le dit.
    /// **Le contrôle positif** de la garde.
    AllowedInPopulation,
    /// Tour webhook PR, pilote vif prouvé : refusé.
    RefusedLivePilot { child_task_id: String, pid: u32 },
    /// Tour webhook PR, vivacité non établie : refusé (fail-closed, § 3).
    RefusedUnreadable { reason: &'static str },
}

impl CancelPilotDecision {
    /// La valeur de `after_value`, en `match` exhaustif **sans bras `_ =>`** :
    /// une cinquième issue doit être tranchée ici par le compilateur, pas
    /// devinée. `Allowed` n'écrit rien, donc n'a pas de valeur d'audit.
    pub(crate) fn audit_value(&self) -> Option<&'static str> {
        match self {
            CancelPilotDecision::Allowed => None,
            CancelPilotDecision::AllowedInPopulation => Some(AUDIT_ALLOWED_NO_PILOT),
            CancelPilotDecision::RefusedLivePilot { .. } => Some(AUDIT_BLOCKED_LIVE_PILOT),
            CancelPilotDecision::RefusedUnreadable { reason } => {
                Some(unreadable_audit_value(reason))
            }
        }
    }

    /// True quand la décision arrête l'annulation.
    pub(crate) fn refuses(&self) -> bool {
        matches!(
            self,
            CancelPilotDecision::RefusedLivePilot { .. }
                | CancelPilotDecision::RefusedUnreadable { .. }
        )
    }
}

/// La cause de l'illisibilité, traduite en valeur d'audit.
///
/// Les deux causes ont des **remèdes opposés** — « la base ne répond pas »
/// contre « l'instance n'est pas prouvable » — donc elles sont comptées
/// séparément, et `live_pilot` en définit exactement deux
/// ([`UNREADABLE_DB_ERROR`], [`UNREADABLE_NO_START_TIME`]).
///
/// **Un troisième motif est une décision à prendre ici, pas un repli.** C'est ce
/// que le `debug_assert_eq!` dit : en build de test il **échoue** plutôt que de
/// compter la cause inconnue avec la cause base en silence, parce que le motif
/// est une valeur d'audit sur laquelle un opérateur groupe et qu'un fourre-tout
/// rendrait deux populations indiscernables. En release il retombe sur la cause
/// base — un verdict illisible reste un verdict illisible, et la ligne de
/// journal porte de toute façon le `reason` **brut**, qui est ce qui permet de
/// les séparer à la lecture.
///
/// La dispatch sur un `&'static str` est l'héritage de
/// `LivePilotVerdict::Unreadable { reason }`, dont la fermeture en enum de
/// causes est un **suivi** : elle rendrait ce `match` exhaustif et retirerait à
/// la fois l'assertion et le repli, mais elle touche les deux appelants
/// existants de mika#2279.
fn unreadable_audit_value(reason: &str) -> &'static str {
    if reason == UNREADABLE_NO_START_TIME {
        AUDIT_BLOCKED_START_TIME_UNREADABLE
    } else {
        debug_assert_eq!(
            reason, UNREADABLE_DB_ERROR,
            "mika#2653 — un troisième motif d'illisibilité est apparu dans \
             `live_pilot` et n'a pas de valeur d'audit : en décider une ici, \
             l'ajouter à ALL_CANCEL_PILOT_GUARD_VERDICTS, et ne pas la laisser \
             se compter avec la cause base"
        );
        AUDIT_BLOCKED_DB_UNREADABLE
    }
}

/// Une annulation peut-elle partir, dans ce tour, sur cette tâche ? (mika#2653)
///
/// # Le défaut que ça ferme
///
/// Trace `8ec6364c-be71-11f1-908b-e931f18d2c16`, 2026-10-02 : un tour mika-dev
/// ouvert par un **webhook de revue QA sur la PR #2644** a appelé `cancel_task`
/// sur le pilote **Fix-CI en vol** (`8a3b2082`) — **71 tours jetés**. Second cas
/// mesuré de la famille mika#2649 (*un tour webhook PR agit sur le plan de
/// dispatch au-delà de ce que l'événement justifie*), et le plus coûteux : son
/// remède n'est pas un terme de **cible** mais un terme de **vivacité**.
///
/// # Le terme de classe de tour est DANS le prédicat
///
/// Forme mika#2649 : « hors tour webhook PR, rien ne change » devient ainsi une
/// propriété de cette fonction pure, avec son propre test, plutôt qu'un `if` à
/// lire chez l'appelant. C'est **AC3**, et c'est ce que le contrôle négatif de
/// V1 atteste.
///
/// # L'arbitrage de fail-safe, tranché : fail-CLOSED
///
/// `Unreadable` **refuse**, dans ses deux causes. C'est l'**inverse** de la
/// politique propre de `live_pilot` (mika#2279, qui a délibérément dérogé à la
/// doctrine mika#2277 pour ses deux appelants), et l'inversion est raisonnée —
/// l'asymétrie de coût est inverse, site par site :
///
/// | | faux `Alive` (refus à tort) | faux `None` (passage à tort) |
/// |---|---|---|
/// | coût | **un appel d'outil refusé** dans un tour webhook | **71 tours jetés** (mesuré) |
/// | visibilité | `tool_calls.output` + un WARN + une ligne d'audit | aucune ligne, le pilote meurt |
/// | boucle ? | **non** — un refus d'outil ne se rejoue pas tout seul, et aucune garde `required_tools` ne réclame `cancel_task` | oui — le défaut se rejoue à chaque webhook |
/// | rattrapage | le modèle rend la main, le callback arrive de lui-même | aucun : le travail est détruit |
///
/// # Et le geste de reprise de l'opérateur n'est PAS sur ce chemin
///
/// La rectification R5 du plan, et c'est elle qui renverse la raison par
/// laquelle le ticket se sortait du périmètre de mika#2649. Le ticket écrit
/// *« `cancel_task` est le geste de reprise le plus court de l'opérateur ; un
/// faux refus le lui retire »* — **c'est faux pour cette garde**.
/// `cancel_task_and_kill` a quatre appelants et **un seul traverse un
/// `ToolContext`** : `mika tasks cancel <id>` (CLI), `POST /tasks/{id}/cancel`
/// (HTTP) et les tests eval mika#2335 n'y arrivent pas. Et sur ce quatrième
/// chemin, le booléen ne vaut `true` que si `originating_message` **commence
/// par** `[GitHub] PR ` ou `[GitHub] Check suite ` : un opérateur qui écrit
/// « annule la tâche X » par `mika ask` ouvre un tour dont le message est son
/// texte, donc **hors population, aucune garde**. Ce qu'un faux refus retire
/// est un appel d'outil à un modèle, rendu visible dans `tool_calls.output` et
/// dans `audit_events`.
///
/// # Coût du fail-closed, nommé plutôt que découvert
///
/// Une ligne portant un `process_id` **sans** `process_start_time` lisible ne
/// pourra pas être annulée depuis un tour webhook PR. C'est la population que
/// mika#2335 compte sous `unusable_child_count`, celle que le faucheur
/// mika#2249 **décline** et que le watchdog #959 ne peut pas juger — donc le
/// refus peut durer jusqu'à ce que `timeout_at` (panic-fallback 6 h) rende la
/// ligne terminale. Borné, mesuré par sa propre valeur d'audit, et l'opérateur
/// garde ses trois chemins. **L'arbitrage est local et ne se transporte pas.**
pub(crate) fn classify_cancel_on_live_pilot(
    is_webhook_pr_event_turn: bool,
    verdict: &LivePilotVerdict,
) -> CancelPilotDecision {
    if !is_webhook_pr_event_turn {
        return CancelPilotDecision::Allowed;
    }
    match verdict {
        LivePilotVerdict::Alive {
            child_task_id, pid, ..
        } => CancelPilotDecision::RefusedLivePilot {
            child_task_id: child_task_id.clone(),
            pid: *pid,
        },
        LivePilotVerdict::None => CancelPilotDecision::AllowedInPopulation,
        LivePilotVerdict::Unreadable { reason } => {
            CancelPilotDecision::RefusedUnreadable { reason }
        }
    }
}

/// Le corps du refus rendu au modèle.
///
/// **Il nomme la levée sans donner de gabarit** (doctrine mika#2520 /
/// mika#2292) : il porte le fait (un pilote travaille sous cette tâche, et quel
/// row porte le pgid), les **deux sorties correctes pour le modèle** (attendre
/// le callback, ou rendre la main en signalant la situation) et **à qui**
/// appartient la levée (un opérateur, sur l'hôte). Et il **ne nomme aucune
/// commande** : `MIKA_DEV_IDENTITY` porte `shell-exec`, donc écrire la commande
/// d'annulation ici donnerait au modèle le gabarit du contournement — *un refus
/// qui donne le gabarit est une fuite avec une étape de plus*. Le résidu
/// `run_shell` est nommé comme suivi (R6), pas comme sortie.
fn refusal_body(decision: &CancelPilotDecision, task_id: &str) -> String {
    match decision {
        CancelPilotDecision::RefusedLivePilot { child_task_id, pid } => serde_json::json!({
            "error": ERROR_CANCEL_LIVE_PILOT,
            "doctrine": "mika#2653",
            "task": task_id,
            "pilot_row": child_task_id,
            "pilot_pid": pid,
            "reason":
                "A pilot is alive and working under this task. This turn was opened by a \
                 GitHub pull-request or check-suite event, which authorizes reviewing and \
                 reporting on that pull request — not ending work in flight. Cancelling \
                 here throws away every turn the pilot has produced.",
            "remedy":
                "Two exits, and they are the only two. (1) Let the pilot finish: its \
                 callback arrives on its own, and the work is then visible. (2) If this \
                 dispatch genuinely looks lost, say so and stop — report what you observed \
                 and let a human decide. Lifting this refusal belongs to an operator, on \
                 the host; it is not lifted by retrying under a different shape.",
        })
        .to_string(),
        CancelPilotDecision::RefusedUnreadable { reason } => serde_json::json!({
            "error": ERROR_CANCEL_PILOT_UNREADABLE,
            "doctrine": "mika#2653",
            "task": task_id,
            "signal": reason,
            "reason":
                "Whether a pilot is alive under this task could not be established, and a \
                 signal that cannot be read is never a satisfied term. This turn was \
                 opened by a GitHub pull-request or check-suite event, where the cost of \
                 cancelling work that is in fact running is a whole dispatch thrown away.",
            "remedy":
                "Two exits, and they are the only two. (1) Proceed with what the event \
                 actually asks of you and leave the task alone. (2) Report that liveness \
                 could not be established and stop. Lifting this refusal belongs to an \
                 operator, on the host.",
        })
        .to_string(),
        // `Allowed` / `AllowedInPopulation` ne composent pas de corps — le
        // `match` reste exhaustif pour qu'une cinquième issue soit tranchée ici.
        CancelPilotDecision::Allowed | CancelPilotDecision::AllowedInPopulation => String::new(),
    }
}

/// Journaliser et auditer une décision de la garde (motif
/// `report_event_target_binding`, mika#2649).
///
/// Trois noms d'événement pour deux conduites de refus et un contrôle positif —
/// chacun au **niveau de son régime attendu**, un WARN sur une population
/// nominale étant un WARN qu'on finit par museler.
///
/// **Rien n'est écrit hors population** : `audit_value()` rend `None` sur
/// `Allowed`, et cette fonction ne fait alors rien. Ce serait l'essentiel du
/// trafic, soit très exactement le churn que la doctrine mika#2131 borne — et
/// c'est aussi ce qui rend la non-régression d'AC3 observable (aucune ligne
/// d'audit sur un tour d'opérateur).
///
/// **Écriture d'audit fire-and-forget** : un échec d'audit ne doit jamais
/// pouvoir changer un verdict d'annulation. Le résidu
/// `cancel_task_pilot_guard_audit_failed` est ce qui dit qu'à partir de là le
/// `GROUP BY` sous-compte.
async fn report_cancel_pilot_decision(
    decision: &CancelPilotDecision,
    task_id: &str,
    ctx: &ToolContext<'_>,
) {
    let Some(audit_value) = decision.audit_value() else {
        return;
    };

    match decision {
        CancelPilotDecision::RefusedLivePilot { child_task_id, pid } => {
            tracing::warn!(
                event = "cancel_task_live_pilot_blocked",
                agent_id = %ctx.db.agent_id(),
                session_id = %ctx.session_id,
                trace_id = %ctx.trace_id,
                task_id = %task_id,
                owner_task_id = %child_task_id,
                pid = *pid,
                "refused to cancel a task whose pilot is alive, on a turn opened by a \
                 GitHub PR / check-suite event (mika#2653)"
            );
        }
        CancelPilotDecision::RefusedUnreadable { reason } => {
            tracing::warn!(
                event = "cancel_task_pilot_unreadable",
                agent_id = %ctx.db.agent_id(),
                session_id = %ctx.session_id,
                trace_id = %ctx.trace_id,
                task_id = %task_id,
                reason = %reason,
                "refused to cancel: pilot liveness could not be established on a turn \
                 opened by a GitHub PR / check-suite event (mika#2653)"
            );
        }
        CancelPilotDecision::AllowedInPopulation => {
            tracing::info!(
                event = "cancel_task_webhook_turn_allowed",
                agent_id = %ctx.db.agent_id(),
                session_id = %ctx.session_id,
                trace_id = %ctx.trace_id,
                task_id = %task_id,
                "no pilot under this task — the guard ran and let the cancellation \
                 through (mika#2653 positive control)"
            );
        }
        // Hors population : `audit_value()` a rendu `None` et la fonction a
        // déjà rendu la main. Le `match` reste exhaustif, sans bras `_ =>`,
        // pour qu'une cinquième issue soit tranchée ici par le compilateur.
        CancelPilotDecision::Allowed => {}
    }

    if let Err(e) = ctx
        .db
        .log_audit_event(
            ctx.session_id,
            CANCEL_PILOT_GUARD_AUDIT_TOOL,
            &format!("task:{task_id}"),
            None,
            Some(audit_value),
            None,
            Some(ctx.trace_id),
        )
        .await
    {
        tracing::warn!(
            event = "cancel_task_pilot_guard_audit_failed",
            error = %e,
            task_id = %task_id,
            verdict = %audit_value,
            "the log line landed but its audit row did not; the GROUP BY undercounts \
             from here"
        );
    }
}

pub struct CancelTaskTool;

#[async_trait]
impl Tool for CancelTaskTool {
    fn name(&self) -> &str {
        "cancel_task"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "cancel_task".to_string(),
            description:
                "Cancel any pending or in-progress task by its full UUID (from list_tasks or create_task). \
                Works for any task type: reminders, callback tasks, recurring tasks, etc. \
                If the task has a running process (e.g., claude-pilot), the process is killed."
                    .to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "id": {
                        "type": "string",
                        "description": "The full UUID of the task to cancel"
                    }
                },
                "required": ["id"]
            }),
        }
    }

    async fn execute(&self, input: Value, ctx: &ToolContext<'_>) -> Result<ToolOutput> {
        let id = input["id"].as_str().unwrap_or("").trim();
        if id.is_empty() {
            return Ok(ToolOutput::error("'id' is required."));
        }
        // Format + existence pre-check — catches fabricated UUIDs before they reach
        // the cancel_task_and_kill infrastructure layer. The kill path does its own
        // get_task internally, so this is an intentional extra DB read for safety.
        let scoped = match super::AgentScopedTaskId::from_tool_context(ctx, id) {
            Ok(s) => s,
            Err(e) => return Ok(e),
        };
        if let Err(e) = super::validate_task_exists(ctx.db, "id", &scoped).await {
            return Ok(e);
        }

        // ── Garde mika#2653 ──
        //
        // Placement : **après** la validation d'existence et de portée (la tâche
        // doit exister et appartenir à l'agent avant qu'on parle de son pilote),
        // **avant** `cancel_task_and_kill` — donc avant toute écriture de statut
        // et avant tout signal.
        //
        // **Dans l'outil, jamais dans `cancel_task_and_kill`** : c'est ce qui
        // préserve les trois chemins opérateur (CLI, HTTP, conversation) **par
        // construction** plutôt que par prédicat, et c'est la différence avec
        // une garde posée dans l'infrastructure de kill.
        //
        // **`cancel_reminder` est couvert gratuitement** : `CancelReminderTool`
        // délègue littéralement à `CancelTaskTool.execute`. À nommer plutôt
        // qu'à découvrir — un futur éditeur qui dédoublerait `cancel_reminder`
        // perdrait la garde en silence.
        //
        // Le verdict de vivacité n'est résolu que **dans la population** : hors
        // tour webhook PR, aucune requête n'est payée. Le terme de classe de
        // tour reste malgré tout un **paramètre** de la fonction pure, pour que
        // « hors tour webhook PR, rien ne change » soit sa propriété et non une
        // branche d'ici.
        if ctx.is_webhook_pr_event_turn {
            let verdict = live_pilot_for_task(ctx.db, id).await;
            let decision = classify_cancel_on_live_pilot(ctx.is_webhook_pr_event_turn, &verdict);
            report_cancel_pilot_decision(&decision, id, ctx).await;

            if decision.refuses() {
                return Ok(ToolOutput::error(refusal_body(&decision, id)));
            }
        }

        let outcome = crate::task_engine::process_kill::cancel_task_and_kill(ctx.db, id).await?;

        let Some(outcome) = outcome else {
            return Ok(ToolOutput::error(format!(
                "Task {id} not found or not in cancellable status."
            )));
        };

        // Build kill status message
        let kill_msg = match (outcome.process_killed, outcome.pid) {
            (Some(true), Some(pid)) => format!(" Process (PID {pid}) terminated."),
            (Some(false), Some(pid)) => {
                format!(" Warning: process (PID {pid}) may still be running.")
            }
            _ => String::new(),
        };

        ctx.db
            .log_audit_event(
                ctx.session_id,
                "cancel_task",
                &format!("task:{id}"),
                None,
                Some("cancelled"),
                None,
                Some(ctx.trace_id),
            )
            .await?;

        Ok(ToolOutput::success(format!(
            "Task {id} (\"{}\") has been cancelled.{kill_msg}",
            outcome.label
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::NewTask;
    use crate::test_utils::test_helpers::TestHarness;

    async fn add_callback_task(harness: &TestHarness, label: &str) -> String {
        harness
            .db
            .create_task(NewTask {
                agent_id: harness.db.agent_id.clone(),
                team_run_id: None,
                parent_task_id: None,
                depth: 0,
                label: label.to_string(),
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
            })
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn test_cancel_task_success() {
        let harness = TestHarness::new();
        let id = add_callback_task(&harness, "Analyze codebase").await;

        let ctx = harness.ctx();
        let tool = CancelTaskTool;

        let result = tool
            .execute(serde_json::json!({"id": id}), &ctx)
            .await
            .unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("cancelled"));
        assert!(result.content.contains("Analyze codebase"));
    }

    #[tokio::test]
    async fn test_cancel_task_not_found() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = CancelTaskTool;

        let result = tool
            .execute(
                serde_json::json!({"id": "00000000-0000-0000-0000-000000000000"}),
                &ctx,
            )
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("task_not_found"));
    }

    #[tokio::test]
    async fn test_cancel_task_invalid_uuid() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = CancelTaskTool;

        let result = tool
            .execute(serde_json::json!({"id": "not-a-uuid"}), &ctx)
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("invalid_uuid"));
    }

    #[tokio::test]
    async fn test_cancel_task_missing_id() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = CancelTaskTool;

        let result = tool.execute(serde_json::json!({}), &ctx).await.unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("'id' is required"));
    }

    // ───────────── mika#2653 — la garde, et ses quatre issues ─────────────

    fn alive() -> LivePilotVerdict {
        LivePilotVerdict::Alive {
            child_task_id: "8a3b2082-0000-0000-0000-000000000000".to_string(),
            // `None` est la forme que ce ticket a rendue possible — une ligne
            // callback nommée directement n'a pas forcément de parent — et la
            // garde n'en lit rien, ce qui est précisément pourquoi le champ
            // n'avait pas à être fabriqué.
            parent_task_id: None,
            pid: 4242,
        }
    }

    /// **V1** — les quatre issues du prédicat pur.
    ///
    /// Chaque terme est exercé seul : une conjonction de termes fail-safe ne se
    /// prouve pas en les neutralisant tous ensemble (leçon mika#2277).
    #[test]
    fn mika2653_les_quatre_issues_du_predicat() {
        assert_eq!(
            classify_cancel_on_live_pilot(true, &alive()),
            CancelPilotDecision::RefusedLivePilot {
                child_task_id: "8a3b2082-0000-0000-0000-000000000000".to_string(),
                pid: 4242,
            },
            "tour webhook PR + pilote vif ⇒ refus : c'est le défaut mesuré"
        );
        assert_eq!(
            classify_cancel_on_live_pilot(true, &LivePilotVerdict::None),
            CancelPilotDecision::AllowedInPopulation,
            "tour webhook PR + aucun pilote ⇒ passe, et le dit (contrôle positif)"
        );
        for reason in [UNREADABLE_DB_ERROR, UNREADABLE_NO_START_TIME] {
            assert_eq!(
                classify_cancel_on_live_pilot(true, &LivePilotVerdict::Unreadable { reason }),
                CancelPilotDecision::RefusedUnreadable { reason },
                "fail-CLOSED sur `{reason}` : l'arbitrage du § 3 est inverse de celui \
                 de `live_pilot`, et l'asymétrie de coût le décide"
            );
        }
    }

    /// Le contrôle négatif du **prédicat pur** : il épingle le contrat de la
    /// fonction, à savoir que le terme de classe de tour décide seul.
    ///
    /// **Ce qu'il ne garantit pas, dit ici pour que personne ne le déduise.**
    /// Il n'atteste aucun des trois chemins opérateur : CLI et HTTP ne
    /// traversent aucun `ToolContext` (R5), donc aucun prédicat à ce niveau ne
    /// peut les casser ; et le chemin de conversation est protégé par le
    /// `if ctx.is_webhook_pr_event_turn` du site d'appel, que ce test n'exécute
    /// jamais. Le contrôle de **production** est V6b du fichier eval
    /// (`mika2653_un_tour_de_conversation_annule_comme_avant`) — c'est lui qui
    /// sépare « la garde décide » de « la garde bloque toute annulation ».
    ///
    /// Corollaire du même fait : l'appelant ayant déjà branché sur le drapeau,
    /// `classify_cancel_on_live_pilot` ne reçoit jamais `false` en production,
    /// donc la branche `Allowed` n'y est pas atteignable. C'est la conséquence
    /// assumée du choix de passer le terme en **paramètre** — ce qui rend
    /// « hors population, rien ne change » vérifiable sans base ni tour.
    #[test]
    fn mika2653_hors_tour_webhook_pr_rien_ne_change() {
        for verdict in [
            alive(),
            LivePilotVerdict::None,
            LivePilotVerdict::Unreadable {
                reason: UNREADABLE_DB_ERROR,
            },
            LivePilotVerdict::Unreadable {
                reason: UNREADABLE_NO_START_TIME,
            },
        ] {
            let decision = classify_cancel_on_live_pilot(false, &verdict);
            assert_eq!(
                decision,
                CancelPilotDecision::Allowed,
                "INVARIANT VIOLÉ (AC3) : la garde a mordu hors de sa population sur \
                 {verdict:?} — le geste d'annulation de l'opérateur n'a pas de \
                 contournement, et le lui retirer est pire que le défaut qu'on referme"
            );
            assert!(
                decision.audit_value().is_none(),
                "hors population : aucune ligne, aucun audit (doctrine mika#2131)"
            );
            assert!(!decision.refuses());
        }
    }

    /// **Détecteur 3** — les quatre valeurs d'audit sont un format de fil : tout
    /// ce que la garde produit est déclaré, et tout ce qui est déclaré est
    /// atteignable.
    ///
    /// Les deux directions. Sans la seconde, une valeur pourrait être déclarée,
    /// groupée par un opérateur, et produite par rien — une colonne de zéros qui
    /// se lit comme une population saine.
    #[test]
    fn mika2653_les_valeurs_daudit_sont_un_format_de_fil() {
        let produced: Vec<&'static str> = [
            classify_cancel_on_live_pilot(true, &alive()),
            classify_cancel_on_live_pilot(true, &LivePilotVerdict::None),
            classify_cancel_on_live_pilot(
                true,
                &LivePilotVerdict::Unreadable {
                    reason: UNREADABLE_DB_ERROR,
                },
            ),
            classify_cancel_on_live_pilot(
                true,
                &LivePilotVerdict::Unreadable {
                    reason: UNREADABLE_NO_START_TIME,
                },
            ),
        ]
        .iter()
        .filter_map(|d| d.audit_value())
        .collect();

        for value in &produced {
            assert!(
                ALL_CANCEL_PILOT_GUARD_VERDICTS.contains(value),
                "{value} est produit mais pas déclaré"
            );
        }
        for declared in ALL_CANCEL_PILOT_GUARD_VERDICTS {
            assert!(
                produced.contains(declared),
                "{declared} est déclaré mais inatteignable — un opérateur grouperait \
                 sur une valeur que rien n'écrit"
            );
        }
        // Les deux causes d'illisibilité ont des remèdes OPPOSÉS, donc elles ne
        // doivent jamais se compter ensemble.
        assert_ne!(
            unreadable_audit_value(UNREADABLE_DB_ERROR),
            unreadable_audit_value(UNREADABLE_NO_START_TIME),
            "« la base ne répond pas » et « l'instance n'est pas prouvable » appellent \
             deux gestes différents : les fondre rendrait la sonde illisible"
        );
    }

    /// **Détecteur 3 (second volet)** — les motifs du champ `error` sont un
    /// format de fil : ils atterrissent dans `tool_calls.output`, où un
    /// opérateur les groupe.
    #[test]
    fn mika2653_les_motifs_derreur_sont_un_format_de_fil() {
        let live = refusal_body(
            &classify_cancel_on_live_pilot(true, &alive()),
            "task-0000-0000",
        );
        let unreadable = refusal_body(
            &classify_cancel_on_live_pilot(
                true,
                &LivePilotVerdict::Unreadable {
                    reason: UNREADABLE_NO_START_TIME,
                },
            ),
            "task-0000-0000",
        );

        for (body, expected) in [
            (&live, ERROR_CANCEL_LIVE_PILOT),
            (&unreadable, ERROR_CANCEL_PILOT_UNREADABLE),
        ] {
            let parsed: serde_json::Value =
                serde_json::from_str(body).expect("le corps du refus est du JSON structuré");
            assert_eq!(
                parsed["error"].as_str(),
                Some(expected),
                "motif d'erreur du corps : {body}"
            );
            assert!(
                ALL_CANCEL_PILOT_GUARD_ERRORS.contains(&expected),
                "{expected} est produit mais pas déclaré"
            );
        }

        // Les deux corps autorisants ne composent rien.
        for decision in [
            CancelPilotDecision::Allowed,
            CancelPilotDecision::AllowedInPopulation,
        ] {
            assert!(refusal_body(&decision, "task-0000-0000").is_empty());
        }
    }

    /// **AC8** — le refus nomme sa levée et **aucun contournement**.
    ///
    /// `MIKA_DEV_IDENTITY` porte `shell-exec`, donc nommer la commande
    /// d'annulation dans le corps du refus donnerait au modèle le gabarit du
    /// contournement — *un refus qui donne le gabarit est une fuite avec une
    /// étape de plus* (doctrine mika#2520).
    #[test]
    fn mika2653_le_refus_ne_nomme_aucun_contournement() {
        for decision in [
            classify_cancel_on_live_pilot(true, &alive()),
            classify_cancel_on_live_pilot(
                true,
                &LivePilotVerdict::Unreadable {
                    reason: UNREADABLE_DB_ERROR,
                },
            ),
        ] {
            let body = refusal_body(&decision, "task-0000-0000");
            let lowered = body.to_ascii_lowercase();
            for forbidden in [
                "tasks cancel",
                "run_shell",
                "mika tasks",
                "update_task_status",
                "promote_deferred_callback",
                "kill ",
                "sigterm",
            ] {
                assert!(
                    !lowered.contains(forbidden),
                    "le corps du refus nomme un contournement (`{forbidden}`) : {body}"
                );
            }
            // Et il nomme bien la levée, et à qui elle appartient.
            assert!(
                lowered.contains("operator"),
                "le refus doit nommer à qui appartient la levée : {body}"
            );
        }
    }
}
