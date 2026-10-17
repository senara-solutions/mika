use anyhow::Result;
use async_trait::async_trait;
use mika_common::claude::ToolDefinition;
use serde_json::Value;

use super::cancel_task::{
    AUDIT_ALLOWED_NO_PILOT, AUDIT_BLOCKED_LIVE_PILOT, unreadable_audit_value,
};
use super::{Tool, ToolContext, ToolOutput};
use crate::live_pilot::{LivePilotVerdict, live_pilot_for_task};
use crate::task_engine::trigger_type;

const MAX_RESULT_LEN: usize = 100_000;

// ───────────── Télémétrie de la garde (mika#2739) ─────────────

/// Le `tool_name` sous lequel chaque décision de la garde est auditée.
///
/// **Un seul nom, la décision dans `after_value`** — motif `ready_label_outcome`
/// (mika#2323) : les trois issues appartiennent au même site et à la même
/// population, donc un `GROUP BY after_value` les sépare et les rend
/// soustractibles.
///
/// Distinct de `cancel_task_pilot_guard` alors que les **valeurs** sont
/// partagées : c'est le `tool_name` qui sépare les deux populations, et c'est ce
/// qui permet à un opérateur de lire soit une porte, soit la vivacité toutes
/// portes confondues (voir `ALL_CANCEL_PILOT_GUARD_VERDICTS`).
///
/// **SOLE WRITER**, épinglé par
/// `canonical_tokens::tests::mika2739_le_nom_daudit_a_un_seul_ecrivain` — c'est
/// cette propriété qui rend le compte exact plutôt qu'un nombre sur lequel deux
/// sites peuvent diverger.
pub(crate) const COMPLETE_TASK_PILOT_GUARD_AUDIT_TOOL: &str = "complete_task_pilot_guard";

/// Motif d'erreur rendu au modèle quand un pilote vif porte le travail.
///
/// Les valeurs du champ `error` sont un **format de fil** : elles atterrissent
/// dans `tool_calls.output`, où un opérateur les groupe. Deux orthographes d'un
/// même motif couperaient une population en deux sans le dire.
pub(crate) const ERROR_COMPLETE_LIVE_PILOT: &str = "complete_refused_live_pilot";
/// Motif d'erreur rendu au modèle quand la vivacité n'a pas pu être établie.
pub(crate) const ERROR_COMPLETE_PILOT_UNREADABLE: &str = "complete_refused_pilot_unreadable";

/// Tous les motifs du champ `error`, pour l'épinglage du format de fil.
///
/// Reste en production plutôt que derrière `#[cfg(test)]`, pour la raison
/// qu'`ALL_CANCEL_PILOT_GUARD_VERDICTS` a déjà écrite : le registre d'un format
/// de fil est ce qu'un opérateur lit pour savoir ce qu'un `GROUP BY` peut
/// rendre.
#[allow(dead_code)]
pub(crate) const ALL_COMPLETE_TASK_PILOT_GUARD_ERRORS: &[&str] =
    &[ERROR_COMPLETE_LIVE_PILOT, ERROR_COMPLETE_PILOT_UNREADABLE];

/// Ce que la garde a décidé pour cet appel à `complete_task`.
///
/// **Trois issues et non quatre** : contrairement à mika#2653, cette garde n'a
/// pas de terme de classe de tour (D4 du plan), donc il n'existe pas d'issue
/// « hors population ». Toute ligne `callback` en statut complétable est dans la
/// population.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CompletePilotDecision {
    /// Aucun pilote sous cette tâche : la complétion passe, et on le dit.
    /// **Le contrôle positif** de la garde.
    AllowedNoPilot,
    /// Pilote vif prouvé : refusé. C'est le défaut mesuré de mika#2739.
    RefusedLivePilot { owner_task_id: String, pid: u32 },
    /// Vivacité non établie : refusé (fail-closed, D3).
    RefusedUnreadable { reason: &'static str },
}

impl CompletePilotDecision {
    /// La valeur de `after_value`, en `match` exhaustif **sans bras `_ =>`** :
    /// une quatrième issue doit être tranchée ici par le compilateur, pas
    /// devinée.
    ///
    /// Les quatre valeurs possibles sont celles de `cancel_task`, **importées**
    /// plutôt que réécrites : les deux gardes posent la même question à
    /// [`live_pilot_for_task`] et obtiennent les mêmes issues.
    ///
    /// **Le passage écrit lui aussi.** Sans ce contrôle positif, « zéro refus »
    /// et « la garde ne tourne pas » rendent les mêmes octets (classe
    /// mika#2205).
    pub(crate) fn audit_value(&self) -> &'static str {
        match self {
            CompletePilotDecision::AllowedNoPilot => AUDIT_ALLOWED_NO_PILOT,
            CompletePilotDecision::RefusedLivePilot { .. } => AUDIT_BLOCKED_LIVE_PILOT,
            CompletePilotDecision::RefusedUnreadable { reason } => unreadable_audit_value(reason),
        }
    }

    /// True quand la décision arrête la complétion.
    pub(crate) fn refuses(&self) -> bool {
        matches!(
            self,
            CompletePilotDecision::RefusedLivePilot { .. }
                | CompletePilotDecision::RefusedUnreadable { .. }
        )
    }
}

/// Une complétion peut-elle partir sur cette tâche ? (mika#2739)
///
/// # Le défaut que ça ferme
///
/// Le 2026-10-16 à 13:19Z, un tour mika-dev de type « deferred-dispatch » a
/// appelé `complete_task` sur la tâche `long_running:run_claude_pilot` du vol
/// d'implémentation de mika#2700, au motif que *« Parent task … already
/// completed. Stale callback cleared to free dispatch slot. »* Le pilote
/// tournait depuis 52 min. La place `implement` a été libérée pendant le vol —
/// trois pilotes sous un cap de 2 — et le callback du pilote tombera sur une
/// ligne déjà `completed`, donc le traitement de son issue n'aura pas lieu.
///
/// **La prémisse était fausse par construction** : la tâche ready-label parente
/// est `completed` *normalement* après un groom, puisque le moteur relance le
/// pilote en réutilisant la tâche (mika#1614).
///
/// # La garde est inconditionnelle
///
/// Pas de terme de classe de tour, et c'est la différence avec mika#2653 (dont
/// la garde est bornée au tour webhook PR). Aucun des sites de référence de
/// `complete_task` n'est un chemin opérateur, et il n'existe pas de
/// `mika tasks complete` : le chemin opérateur est `POST /tasks/{id}/complete`,
/// qui écrit par `update_task_completed` sans traverser l'outil. AC1 est donc
/// littéral, sans qualificatif de tour.
///
/// Le classificateur reste malgré tout une **fonction pure** prenant le verdict
/// en paramètre, pour que ses issues soient testables sans base.
///
/// # L'arbitrage de fail-safe, tranché : fail-CLOSED
///
/// `Unreadable` **refuse**, dans ses deux causes — l'inverse de la politique
/// propre de `live_pilot` (mika#2279), et l'inversion est raisonnée par
/// l'asymétrie de coût, site par site :
///
/// | | faux refus | faux passage |
/// |---|---|---|
/// | coût | **un appel d'outil refusé** | un vol détruit : place libérée, callback perdu (mesuré) |
/// | visibilité | `tool_calls.output` + WARN + ligne d'audit | aucune ligne |
/// | rattrapage | le callback arrive de lui-même ; `timeout_at` (6 h) borne le reste | aucun |
///
/// # Coût du fail-closed, nommé plutôt que découvert
///
/// Une ligne `callback` portant un `process_id` **sans** `process_start_time`
/// lisible — la population que mika#2335 compte sous `unusable_child_count` — ne
/// pourra plus être complétée par l'outil LLM. Borné par `timeout_at`
/// (panic-fallback 6 h), compté sous `blocked_start_time_unreadable`, et les
/// deux chemins opérateur (HTTP, CLI d'annulation) restent entiers.
pub(crate) fn classify_complete_on_live_pilot(verdict: &LivePilotVerdict) -> CompletePilotDecision {
    match verdict {
        LivePilotVerdict::Alive {
            child_task_id, pid, ..
        } => CompletePilotDecision::RefusedLivePilot {
            owner_task_id: child_task_id.clone(),
            pid: *pid,
        },
        LivePilotVerdict::None => CompletePilotDecision::AllowedNoPilot,
        LivePilotVerdict::Unreadable { reason } => {
            CompletePilotDecision::RefusedUnreadable { reason }
        }
    }
}

/// Le corps du refus rendu au modèle.
///
/// **Il nomme la levée sans donner de gabarit** (doctrine mika#2520 /
/// mika#2292) : il porte le fait (un pilote travaille sous cette tâche, quel row
/// porte le pgid, quel PID), les **deux** sorties correctes pour le modèle
/// (attendre le callback, ou rendre la main en signalant la situation) et **à
/// qui** appartient la levée (un opérateur, sur l'hôte). Il **ne nomme aucune
/// commande** : `MIKA_DEV_IDENTITY` porte `shell-exec`, donc écrire ici la
/// commande qui termine la tâche donnerait au modèle le gabarit du
/// contournement — *un refus qui donne le gabarit est une fuite avec une étape
/// de plus*.
///
/// Il nomme aussi la **prémisse réfutée**, puisque c'est elle qui a produit
/// l'incident : un parent `completed` est l'état normal après un groom
/// (mika#1614), et n'est pas la preuve d'un callback orphelin.
fn refusal_body(decision: &CompletePilotDecision, task_id: &str, task_label: &str) -> String {
    match decision {
        CompletePilotDecision::RefusedLivePilot { owner_task_id, pid } => serde_json::json!({
            "error": ERROR_COMPLETE_LIVE_PILOT,
            "doctrine": "mika#2739",
            "task": task_id,
            "task_label": task_label,
            "pilot_row": owner_task_id,
            "pilot_pid": pid,
            "reason":
                "A pilot is alive and working under this callback task, so the work it \
                 reports is not finished. Completing the row here frees the dispatch slot \
                 while the flight continues, and the pilot's own callback will later land \
                 on a row already in a terminal state, where its result is dropped. Note \
                 the premise that usually leads here: a parent task in status 'completed' \
                 is the normal state after a groom, because the engine reuses that task to \
                 relaunch the pilot (mika#1614). It is not evidence of an orphaned callback.",
            "remedy":
                "Two exits, and they are the only two. (1) Let the pilot finish: its \
                 callback arrives on its own and completes this row with the real result. \
                 (2) If this dispatch genuinely looks lost, say so and stop — report what \
                 you observed, including the row and the pid above, and let a human decide. \
                 Lifting this refusal belongs to an operator, on the host; it is not lifted \
                 by retrying under a different shape.",
        })
        .to_string(),
        CompletePilotDecision::RefusedUnreadable { reason } => serde_json::json!({
            "error": ERROR_COMPLETE_PILOT_UNREADABLE,
            "doctrine": "mika#2739",
            "task": task_id,
            "task_label": task_label,
            "signal": reason,
            "reason":
                "Whether a pilot is alive under this callback task could not be \
                 established, and a signal that cannot be read is never a satisfied term. \
                 The cost of completing a row whose pilot is in fact running is a whole \
                 dispatch thrown away: the slot is freed mid-flight and the pilot's \
                 callback is dropped on a terminal row.",
            "remedy":
                "Two exits, and they are the only two. (1) Leave the task alone and \
                 proceed with what you were actually asked to do. (2) Report that liveness \
                 could not be established, naming this task, and stop. Lifting this refusal \
                 belongs to an operator, on the host.",
        })
        .to_string(),
        // Le passage ne compose pas de corps — le `match` reste exhaustif pour
        // qu'une quatrième issue soit tranchée ici.
        CompletePilotDecision::AllowedNoPilot => String::new(),
    }
}

/// Journaliser et auditer une décision de la garde (motif
/// `report_event_target_binding`, mika#2649).
///
/// Trois noms d'événement pour deux conduites de refus et un contrôle positif —
/// chacun au **niveau de son régime attendu**, un WARN sur une population
/// nominale étant un WARN qu'on finit par museler.
///
/// Les champs du WARN de refus sont **AC3** : la tâche, le PID et l'agent, plus
/// la session, la trace, le libellé et la ligne qui porte le pgid.
///
/// **Écriture d'audit fire-and-forget** (motif `AuthBoundaryLedger`) : un échec
/// d'audit ne doit jamais pouvoir changer un verdict. Le résidu
/// `complete_task_pilot_guard_audit_failed` est ce qui dit qu'à partir de là le
/// `GROUP BY` sous-compte.
async fn report_complete_pilot_decision(
    decision: &CompletePilotDecision,
    task_id: &str,
    task_label: &str,
    ctx: &ToolContext<'_>,
) {
    match decision {
        CompletePilotDecision::RefusedLivePilot { owner_task_id, pid } => {
            tracing::warn!(
                event = "complete_task_live_pilot_blocked",
                agent_id = %ctx.db.agent_id(),
                session_id = %ctx.session_id,
                trace_id = %ctx.trace_id,
                task_id = %task_id,
                task_label = %task_label,
                owner_task_id = %owner_task_id,
                pid = *pid,
                "refused to complete a callback task whose pilot is alive (mika#2739)"
            );
        }
        CompletePilotDecision::RefusedUnreadable { reason } => {
            tracing::warn!(
                event = "complete_task_pilot_unreadable",
                agent_id = %ctx.db.agent_id(),
                session_id = %ctx.session_id,
                trace_id = %ctx.trace_id,
                task_id = %task_id,
                task_label = %task_label,
                reason = %reason,
                "refused to complete: pilot liveness could not be established (mika#2739)"
            );
        }
        CompletePilotDecision::AllowedNoPilot => {
            tracing::info!(
                event = "complete_task_pilot_absent",
                agent_id = %ctx.db.agent_id(),
                session_id = %ctx.session_id,
                trace_id = %ctx.trace_id,
                task_id = %task_id,
                task_label = %task_label,
                "no pilot under this callback task — the guard ran and let the completion \
                 through (mika#2739 positive control)"
            );
        }
    }

    let audit_value = decision.audit_value();
    if let Err(e) = ctx
        .db
        .log_audit_event(
            ctx.session_id,
            COMPLETE_TASK_PILOT_GUARD_AUDIT_TOOL,
            &format!("task:{task_id}"),
            None,
            Some(audit_value),
            None,
            Some(ctx.trace_id),
        )
        .await
    {
        tracing::warn!(
            event = "complete_task_pilot_guard_audit_failed",
            error = %e,
            task_id = %task_id,
            verdict = %audit_value,
            "the log line landed but its audit row did not; the GROUP BY undercounts \
             from here"
        );
    }
}

pub struct CompleteTaskTool;

#[async_trait]
impl Tool for CompleteTaskTool {
    fn name(&self) -> &str {
        "complete_task"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "complete_task".to_string(),
            description: "Mark a callback task as completed and provide its result. Only works for tasks with trigger_type=callback that are in pending or in_progress status.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "id": {
                        "type": "string",
                        "description": "The full UUID of the callback task to complete."
                    },
                    "result": {
                        "type": "string",
                        "description": "The result or output of the completed work (required, max 100KB)."
                    }
                },
                "required": ["id", "result"]
            }),
        }
    }

    async fn execute(&self, input: Value, ctx: &ToolContext<'_>) -> Result<ToolOutput> {
        let id = input["id"].as_str().unwrap_or("").trim();
        if id.is_empty() {
            return Ok(ToolOutput::error("'id' is required."));
        }

        let result = input["result"].as_str().unwrap_or("").trim();
        if result.is_empty() {
            return Ok(ToolOutput::error("'result' is required."));
        }
        if result.len() > MAX_RESULT_LEN {
            return Ok(ToolOutput::error(format!(
                "'result' exceeds maximum length of {} characters.",
                MAX_RESULT_LEN
            )));
        }

        // Format + existence + agent-scope validation in one call
        let scoped = match super::AgentScopedTaskId::from_tool_context(ctx, id) {
            Ok(s) => s,
            Err(e) => return Ok(e),
        };
        let task = match super::validate_task_exists(ctx.db, "id", &scoped).await {
            Ok(t) => t,
            Err(e) => return Ok(e),
        };

        if task.trigger_type != trigger_type::CALLBACK {
            return Ok(ToolOutput::error(format!(
                "Task '{}' has trigger_type '{}', not 'callback'. Only callback tasks can be completed with this tool.",
                id, task.trigger_type
            )));
        }

        if !matches!(task.status.as_str(), "pending" | "in_progress") {
            return Ok(ToolOutput::error(format!(
                "Task '{}' has status '{}' and cannot be completed.",
                id, task.status
            )));
        }

        // ── Garde mika#2739 ──
        //
        // Placement : **après** le refus de `trigger_type` et le refus de
        // statut (la ligne doit être une ligne callback complétable avant qu'on
        // parle de son pilote), **avant toute écriture** — donc avant
        // `update_task_completed`. C'est ce qui rend AC1 littéral : « ne change
        // pas le statut ».
        //
        // **Dans l'outil, et seulement dans l'outil.** Le chemin de livraison
        // nominal passe par `POST /tasks/{id}/complete`, qui écrit par
        // `update_task_completed` : le trap EXIT de `dispatch-lib` délivre le
        // callback **depuis l'intérieur** du wrapper encore vivant, donc le
        // processus *est* vivant et la complétion *est* légitime pendant cette
        // fenêtre. Une garde posée dans `update_task_completed` ou dans le
        // handler HTTP casserait cette fenêtre par construction.
        //
        // La vivacité est lue par le **lecteur unique** de la question
        // (`live_pilot_for_task`, mika#2653 / mika#2670) : ni la traversée
        // `find_dispatch_children_with_pid`, ni le filtre terminal, ni
        // `is_same_process_alive` n'apparaissent ici.
        let verdict = live_pilot_for_task(ctx.db, id).await;
        let decision = classify_complete_on_live_pilot(&verdict);
        report_complete_pilot_decision(&decision, id, &task.label, ctx).await;

        if decision.refuses() {
            return Ok(ToolOutput::error(refusal_body(&decision, id, &task.label)));
        }

        // Mark the task as completed (returns false if already in terminal state)
        if !ctx.db.update_task_completed(id, Some(result)).await? {
            return Ok(ToolOutput::error(format!(
                "Task '{}' could not be completed: already in a terminal state.",
                id
            )));
        }

        // Audit log
        ctx.db
            .log_audit_event(
                ctx.session_id,
                "complete_task",
                &format!("task:{id}"),
                None,
                Some(&*format!(
                    "Completed callback task: {} ({})",
                    task.label, id
                )),
                None,
                Some(ctx.trace_id),
            )
            .await?;

        Ok(ToolOutput::success(format!(
            "Task {} ('{}') marked as completed.",
            id, task.label
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::NewTask;
    use crate::live_pilot::{UNREADABLE_DB_ERROR, UNREADABLE_NO_START_TIME};
    use crate::test_utils::test_helpers::TestHarness;
    use crate::tools::cancel_task::ALL_CANCEL_PILOT_GUARD_VERDICTS;

    async fn add_callback_task(harness: &TestHarness) -> String {
        harness
            .db
            .create_task(NewTask {
                agent_id: harness.db.agent_id.clone(),
                team_run_id: None,
                parent_task_id: None,
                depth: 0,
                label: "test callback".to_string(),
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
    async fn test_complete_task_success() {
        let harness = TestHarness::new();
        let task_id = add_callback_task(&harness).await;
        let ctx = harness.ctx();
        let tool = CompleteTaskTool;
        let input = serde_json::json!({"id": task_id, "result": "analysis done"});
        let output = tool.execute(input, &ctx).await.unwrap();
        assert!(!output.is_error, "got error: {}", output.content);
        assert!(output.content.contains(&task_id));
    }

    // ───────────── mika#2739 — la garde, sur le chemin de production ─────────────

    /// Une ligne callback qui porte un pgid et, éventuellement, le
    /// `process_start_time` par lequel son instance se prouve.
    ///
    /// `NewTask` ne porte ni l'un ni l'autre : le pgid est estampillé après le
    /// spawn (`set_task_process_id`) et l'instant de départ vit dans
    /// `metadata` — l'idiome est celui des tests de `live_pilot`.
    async fn add_callback_task_with_pilot(
        harness: &TestHarness,
        pid: i64,
        start_time: Option<u64>,
    ) -> String {
        let id = add_callback_task(harness).await;
        harness
            .db
            .set_task_process_id(&id, Some(pid))
            .await
            .expect("record pgid");
        if let Some(st) = start_time {
            harness
                .db
                .set_task_metadata_field(&id, "process_start_time", &st.to_string())
                .await
                .expect("record start time");
        }
        id
    }

    /// Notre propre processus : vivant, avec un instant de départ qui concorde
    /// par construction.
    #[cfg(target_os = "linux")]
    fn self_pid_and_start() -> (i64, u64) {
        let pid = std::process::id();
        let st = crate::task_engine::process_liveness::read_process_start_time(pid)
            .expect("read own start time");
        (i64::from(pid), st)
    }

    /// **V1 (AC1)** — l'incident de mika#2739, reproduit puis refermé : un
    /// pilote vif sous la tâche callback, et `complete_task` qui la termine.
    ///
    /// Vu **rouge** avant la garde : l'appel passait et la ligne rendait
    /// `completed`. C'est le défaut mesuré le 2026-10-16 à 13:19Z.
    #[tokio::test]
    #[cfg(target_os = "linux")]
    async fn mika2739_un_pilote_vif_interdit_la_completion() {
        let harness = TestHarness::new();
        let (pid, st) = self_pid_and_start();
        let task_id = add_callback_task_with_pilot(&harness, pid, Some(st)).await;
        let ctx = harness.ctx();

        let output = CompleteTaskTool
            .execute(
                serde_json::json!({"id": task_id, "result": "stale callback cleared"}),
                &ctx,
            )
            .await
            .unwrap();

        assert!(
            output.is_error,
            "un pilote vif doit interdire la completion : {}",
            output.content
        );
        assert!(
            output.content.contains("complete_refused_live_pilot"),
            "le refus doit porter son motif de fil : {}",
            output.content
        );
        assert!(
            output.content.contains(&pid.to_string()),
            "le refus doit nommer le PID (AC1) : {}",
            output.content
        );

        let task = harness.db.get_task(&task_id).await.unwrap().unwrap();
        assert_eq!(
            task.status, "pending",
            "INVARIANT VIOLÉ (AC1) : le statut a bougé malgré le refus"
        );
    }

    /// **V2 (AC2)** — un pgid mort garde le comportement actuel.
    #[tokio::test]
    async fn mika2739_un_pgid_mort_passe_comme_avant() {
        let harness = TestHarness::new();
        let task_id = add_callback_task_with_pilot(&harness, 999_999_999, Some(1)).await;
        let ctx = harness.ctx();

        let output = CompleteTaskTool
            .execute(
                serde_json::json!({"id": task_id, "result": "pilot is gone"}),
                &ctx,
            )
            .await
            .unwrap();

        assert!(!output.is_error, "got error: {}", output.content);
        let task = harness.db.get_task(&task_id).await.unwrap().unwrap();
        assert_eq!(task.status, "completed");
    }

    /// **V4** — fail-CLOSED : un pgid vif dont l'instance n'est pas prouvable
    /// (`metadata` absent, population `unusable_child_count` de mika#2335) est
    /// un verdict illisible, et un signal qu'on ne peut pas lire n'est jamais un
    /// terme satisfait.
    #[tokio::test]
    #[cfg(target_os = "linux")]
    async fn mika2739_une_vivacite_illisible_refuse() {
        let harness = TestHarness::new();
        let (pid, _) = self_pid_and_start();
        let task_id = add_callback_task_with_pilot(&harness, pid, None).await;
        let ctx = harness.ctx();

        let output = CompleteTaskTool
            .execute(
                serde_json::json!({"id": task_id, "result": "cleared"}),
                &ctx,
            )
            .await
            .unwrap();

        assert!(output.is_error, "fail-closed : {}", output.content);
        assert!(
            output.content.contains("complete_refused_pilot_unreadable"),
            "le refus doit porter son motif de fil : {}",
            output.content
        );
        let task = harness.db.get_task(&task_id).await.unwrap().unwrap();
        assert_eq!(task.status, "pending");
    }

    #[tokio::test]
    async fn test_complete_task_not_found() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = CompleteTaskTool;
        let input =
            serde_json::json!({"id": "00000000-0000-0000-0000-000000000000", "result": "result"});
        let output = tool.execute(input, &ctx).await.unwrap();
        assert!(output.is_error);
        assert!(output.content.contains("task_not_found"));
    }

    #[tokio::test]
    async fn test_complete_task_wrong_trigger_type() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        // Create a time-triggered task via create_task tool
        let create_input = serde_json::json!({
            "label": "time reminder",
            "trigger_type": "time",
            "action_type": "send_message",
            "action_config": "{\"text\":\"hello\"}",
            "fire_at": "2099-01-01T00:00:00Z"
        });
        let create_output = crate::tools::create_scheduled_task::CreateScheduledTaskTool
            .execute(create_input, &ctx)
            .await
            .unwrap();
        assert!(
            !create_output.is_error,
            "setup failed: {}",
            create_output.content
        );
        // Extract UUID from "Task <uuid> created ..."
        let task_id = create_output
            .content
            .split_whitespace()
            .nth(1)
            .unwrap()
            .to_string();

        let tool = CompleteTaskTool;
        let input = serde_json::json!({"id": task_id, "result": "result"});
        let output = tool.execute(input, &ctx).await.unwrap();
        assert!(output.is_error);
        assert!(output.content.contains("callback"));
    }

    #[tokio::test]
    async fn test_complete_task_empty_id() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = CompleteTaskTool;
        let input = serde_json::json!({"id": "", "result": "result"});
        let output = tool.execute(input, &ctx).await.unwrap();
        assert!(output.is_error);
        assert!(output.content.contains("required"));
    }

    #[tokio::test]
    async fn test_complete_task_empty_result() {
        let harness = TestHarness::new();
        let task_id = add_callback_task(&harness).await;
        let ctx = harness.ctx();
        let tool = CompleteTaskTool;
        let input = serde_json::json!({"id": task_id, "result": ""});
        let output = tool.execute(input, &ctx).await.unwrap();
        assert!(output.is_error);
        assert!(output.content.contains("required"));
    }

    // ───────────── mika#2739 — le classificateur pur, et ses issues ─────────────

    fn alive() -> LivePilotVerdict {
        LivePilotVerdict::Alive {
            child_task_id: "a1b2c3d4-0000-0000-0000-000000000000".to_string(),
            // `None` est une forme réelle : une ligne callback nommée
            // directement n'a pas forcément de parent, et la garde n'en lit
            // rien — ce qui est précisément pourquoi le champ n'a pas à être
            // fabriqué.
            parent_task_id: None,
            pid: 4242,
        }
    }

    /// **Les quatre issues du prédicat pur.**
    ///
    /// Chaque terme est exercé seul : une conjonction de termes fail-safe ne se
    /// prouve pas en les neutralisant tous ensemble (leçon mika#2277).
    #[test]
    fn mika2739_les_issues_du_predicat() {
        assert_eq!(
            classify_complete_on_live_pilot(&alive()),
            CompletePilotDecision::RefusedLivePilot {
                owner_task_id: "a1b2c3d4-0000-0000-0000-000000000000".to_string(),
                pid: 4242,
            },
            "pilote vif ⇒ refus : c'est le défaut mesuré"
        );
        assert_eq!(
            classify_complete_on_live_pilot(&LivePilotVerdict::None),
            CompletePilotDecision::AllowedNoPilot,
            "aucun pilote ⇒ passe, et le dit (contrôle positif)"
        );
        for reason in [UNREADABLE_DB_ERROR, UNREADABLE_NO_START_TIME] {
            assert_eq!(
                classify_complete_on_live_pilot(&LivePilotVerdict::Unreadable { reason }),
                CompletePilotDecision::RefusedUnreadable { reason },
                "fail-CLOSED sur `{reason}` : l'asymétrie de coût décide, et elle est \
                 inverse de celle de `live_pilot`"
            );
        }
    }

    /// Les quatre valeurs d'audit sont un **format de fil** : tout ce que la
    /// garde produit est déclaré, et tout ce qui est déclaré est atteignable.
    ///
    /// Le registre est celui de `cancel_task`, **partagé** : c'est le `tool_name`
    /// qui sépare les deux populations, jamais la valeur.
    ///
    /// Les deux directions. Sans la seconde, une valeur pourrait être déclarée,
    /// groupée par un opérateur, et produite par rien — une colonne de zéros qui
    /// se lit comme une population saine.
    #[test]
    fn mika2739_les_valeurs_daudit_sont_un_format_de_fil() {
        let produced: Vec<&'static str> = [
            classify_complete_on_live_pilot(&alive()),
            classify_complete_on_live_pilot(&LivePilotVerdict::None),
            classify_complete_on_live_pilot(&LivePilotVerdict::Unreadable {
                reason: UNREADABLE_DB_ERROR,
            }),
            classify_complete_on_live_pilot(&LivePilotVerdict::Unreadable {
                reason: UNREADABLE_NO_START_TIME,
            }),
        ]
        .iter()
        .map(|d| d.audit_value())
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
                "{declared} est déclaré mais inatteignable depuis cette garde — un \
                 opérateur grouperait sur une valeur que rien n'écrit sous \
                 {COMPLETE_TASK_PILOT_GUARD_AUDIT_TOOL}"
            );
        }
        // Le contrôle positif écrit, lui aussi : sans lui, « zéro refus » et
        // « la garde ne tourne pas » rendent les mêmes octets (mika#2205).
        assert_eq!(
            CompletePilotDecision::AllowedNoPilot.audit_value(),
            AUDIT_ALLOWED_NO_PILOT
        );
        assert!(!CompletePilotDecision::AllowedNoPilot.refuses());
    }

    /// Les motifs du champ `error` sont un format de fil : ils atterrissent dans
    /// `tool_calls.output`, où un opérateur les groupe.
    #[test]
    fn mika2739_les_motifs_derreur_sont_un_format_de_fil() {
        let live = refusal_body(
            &classify_complete_on_live_pilot(&alive()),
            "task-0000-0000",
            "long_running:run_claude_pilot",
        );
        let unreadable = refusal_body(
            &classify_complete_on_live_pilot(&LivePilotVerdict::Unreadable {
                reason: UNREADABLE_NO_START_TIME,
            }),
            "task-0000-0000",
            "long_running:run_claude_pilot",
        );

        for (body, expected) in [
            (&live, ERROR_COMPLETE_LIVE_PILOT),
            (&unreadable, ERROR_COMPLETE_PILOT_UNREADABLE),
        ] {
            let parsed: serde_json::Value =
                serde_json::from_str(body).expect("le corps du refus est du JSON structuré");
            assert_eq!(
                parsed["error"].as_str(),
                Some(expected),
                "motif d'erreur du corps : {body}"
            );
            assert_eq!(
                parsed["task_label"].as_str(),
                Some("long_running:run_claude_pilot"),
                "AC1 : le refus nomme le libellé de la tâche"
            );
            assert!(
                ALL_COMPLETE_TASK_PILOT_GUARD_ERRORS.contains(&expected),
                "{expected} est produit mais pas déclaré"
            );
        }

        // Le refus pour pilote vif porte le PID et la ligne qui porte le pgid.
        let parsed: serde_json::Value = serde_json::from_str(&live).unwrap();
        assert_eq!(parsed["pilot_pid"].as_u64(), Some(4242));
        assert_eq!(
            parsed["pilot_row"].as_str(),
            Some("a1b2c3d4-0000-0000-0000-000000000000")
        );

        // Le passage ne compose rien.
        assert!(
            refusal_body(
                &CompletePilotDecision::AllowedNoPilot,
                "task-0000-0000",
                "whatever"
            )
            .is_empty()
        );
    }

    /// Le refus nomme sa levée, la **prémisse réfutée**, et **aucun
    /// contournement**.
    ///
    /// `MIKA_DEV_IDENTITY` porte `shell-exec`, donc nommer dans le corps du
    /// refus la commande ou l'outil qui termine la tâche donnerait au modèle le
    /// gabarit du contournement — *un refus qui donne le gabarit est une fuite
    /// avec une étape de plus* (doctrine mika#2520).
    #[test]
    fn mika2739_le_refus_ne_nomme_aucun_contournement() {
        for decision in [
            classify_complete_on_live_pilot(&alive()),
            classify_complete_on_live_pilot(&LivePilotVerdict::Unreadable {
                reason: UNREADABLE_DB_ERROR,
            }),
        ] {
            let body = refusal_body(&decision, "task-0000-0000", "long_running:run_claude_pilot");
            let lowered = body.to_ascii_lowercase();
            for forbidden in [
                "tasks complete",
                "tasks cancel",
                "run_shell",
                "mika tasks",
                "update_task_status",
                "promote_deferred_callback",
                "/tasks/",
                "kill ",
                "sigterm",
            ] {
                assert!(
                    !lowered.contains(forbidden),
                    "le corps du refus nomme un contournement (`{forbidden}`) : {body}"
                );
            }
            assert!(
                lowered.contains("operator"),
                "le refus doit nommer à qui appartient la levée : {body}"
            );
        }

        // Et le refus pour pilote vif nomme la prémisse qui a produit
        // l'incident : un parent `completed` est l'état normal après un groom.
        let live = refusal_body(
            &classify_complete_on_live_pilot(&alive()),
            "task-0000-0000",
            "long_running:run_claude_pilot",
        );
        assert!(
            live.contains("mika#1614"),
            "le refus doit nommer la prémisse réfutée (D6) : {live}"
        );
    }

    #[tokio::test]
    async fn test_complete_task_invalid_uuid() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = CompleteTaskTool;
        let input = serde_json::json!({"id": "not-a-valid-uuid", "result": "result"});
        let output = tool.execute(input, &ctx).await.unwrap();
        assert!(output.is_error);
        assert!(output.content.contains("invalid_uuid"));
    }
}
