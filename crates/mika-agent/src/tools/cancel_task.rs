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
/// **Le contrôle positif** : aucun pilote sous la tâche, l'annulation passe.
pub(crate) const AUDIT_ALLOWED_NO_PILOT: &str = "allowed_no_pilot";

/// Toutes les valeurs que `after_value` peut prendre, pour l'épinglage du
/// format de fil (détecteur 3).
///
/// Reste en production plutôt que derrière `#[cfg(test)]`, et c'est la raison
/// qu'`ALL_MARKER_CLASSES` a déjà dû écrire : le registre d'un format de fil est
/// ce qu'un opérateur lit pour savoir ce qu'un `GROUP BY` peut rendre, et un
/// registre qui n'existe que sous `cfg(test)` est un registre qu'un lecteur de
/// ce fichier ne trouve pas.
///
/// # Un vocabulaire **partagé**, et ce qui sépare les populations (mika#2739)
///
/// Ces quatre valeurs sont aussi celles qu'écrit la garde de `complete_task`
/// (mika#2739), qui les **importe** d'ici plutôt que de les redéclarer : les
/// deux gardes posent la même question à [`live_pilot_for_task`] et obtiennent
/// les mêmes quatre issues, donc deux orthographes d'un même verdict
/// couperaient une population en deux sans le dire.
///
/// Ce qui sépare les deux populations n'est pas la valeur mais le **`tool_name`**
/// sous lequel la ligne est écrite — `cancel_task_pilot_guard` ici,
/// `complete_task_pilot_guard` là-bas. Un opérateur qui veut l'un des deux
/// filtre sur `tool_name` ; un opérateur qui veut la vivacité toutes portes
/// confondues groupe sur `after_value` sans filtrer. **Les deux lectures sont
/// voulues**, et c'est précisément ce que le partage du vocabulaire achète.
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
///
/// **Trois issues depuis mika#2763, et plus quatre.** Les deux anciennes issues
/// passantes — `Allowed` (hors tour webhook PR, aucune ligne) et
/// `AllowedInPopulation` (dans la population, aucun pilote) — étaient le même
/// fait *du point de vue du travail* : aucun pilote ne meurt. Ce qui les
/// séparait n'était pas la vivacité mais la **classe du tour**, et c'est
/// exactement le terme que mika#2763 a retiré du prédicat. Les fusionner en
/// [`CancelPilotDecision::AllowedNoPilot`] rend `audit_value()` **totale** : il
/// n'existe plus d'issue muette, donc plus de population dans laquelle « la
/// garde a laissé passer » et « la garde n'a pas tourné » rendent les mêmes
/// octets (classe mika#2205). Même forme que la sœur mika#2739 sur
/// `complete_task`, qui n'a jamais eu de quatrième issue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CancelPilotDecision {
    /// Aucun pilote sous la tâche : l'annulation passe, et on le dit.
    /// **Le contrôle positif** de la garde.
    AllowedNoPilot,
    /// Pilote vif prouvé : refusé.
    RefusedLivePilot { child_task_id: String, pid: u32 },
    /// Vivacité non établie : refusé (fail-closed, § 3).
    RefusedUnreadable { reason: &'static str },
}

impl CancelPilotDecision {
    /// La valeur de `after_value`, en `match` exhaustif **sans bras `_ =>`** :
    /// une quatrième issue doit être tranchée ici par le compilateur, pas
    /// devinée.
    ///
    /// **Totale depuis mika#2763** (`-> &'static str`, plus d'`Option`) : toute
    /// décision de cette garde s'audite, parce que toute décision est désormais
    /// prise dans la même population — celle de tous les appels à `cancel_task`
    /// qui traversent un `ToolContext`.
    pub(crate) fn audit_value(&self) -> &'static str {
        match self {
            CancelPilotDecision::AllowedNoPilot => AUDIT_ALLOWED_NO_PILOT,
            CancelPilotDecision::RefusedLivePilot { .. } => AUDIT_BLOCKED_LIVE_PILOT,
            CancelPilotDecision::RefusedUnreadable { reason } => unreadable_audit_value(reason),
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
///
/// `pub(crate)` depuis mika#2739 : la garde de `complete_task` traduit les mêmes
/// deux causes vers les mêmes deux valeurs, et une seconde copie de ce `match`
/// serait un second endroit où un troisième motif pourrait se compter en
/// silence avec la cause base.
pub(crate) fn unreadable_audit_value(reason: &str) -> &'static str {
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

/// Une annulation peut-elle partir sur cette tâche ? (mika#2653, mika#2763)
///
/// # Les deux défauts que ça ferme
///
/// **mika#2653** — trace `8ec6364c-be71-11f1-908b-e931f18d2c16`, 2026-10-02 :
/// un tour mika-dev ouvert par un **webhook de revue QA sur la PR #2644** a
/// appelé `cancel_task` sur le pilote **Fix-CI en vol** (`8a3b2082`) — **71
/// tours jetés**. Second cas mesuré de la famille mika#2649 (*un tour webhook PR
/// agit sur le plan de dispatch au-delà de ce que l'événement justifie*), et le
/// plus coûteux : son remède n'est pas un terme de **cible** mais un terme de
/// **vivacité**.
///
/// **mika#2763** — 2026-10-17 07:59:38Z : un tour **silencieux**
/// (`trigger=deferred_dispatch`), ouvert pour lancer le regroom de mika#2752, a
/// trouvé la place groom tenue par le groom **vivant** de mika#2757 (lancé à
/// 07:55:25Z, 4 min), l'a déclaré « stale (>1h, no output, no PR, no Plan
/// callout) » et l'a annulé. Le « >1h » était l'âge du **parent** (06:41:38Z),
/// seule date que l'agent pouvait lire : `check_task` refuse une tâche non
/// manuelle. mika#2757 est resté `in_progress` sans enfant actif.
///
/// # La classe du tour n'entre PLUS dans le prédicat (mika#2763)
///
/// mika#2653 avait posé la vivacité **sous** un terme de classe de tour, forme
/// mika#2649. Le second incident est passé par la seule porte que ce terme
/// laissait ouverte : `is_webhook_pr_event_turn` n'est dérivé qu'**à un seul
/// site** (`agent_loop/mod.rs`, tour de conversation) et les ~vingt autres
/// constructions de `ToolContext` — dont celle des tours silencieux — le posent
/// à `false` en dur. La garde ne tournait donc pas du tout sur le chemin de
/// l'incident.
///
/// Un pilote vif est un pilote vif dans **toute** classe de tour : le coût d'un
/// faux passage est un vol entier, quel que soit l'événement qui a ouvert le
/// tour. La classe du tour n'était jamais un terme de la question *« du travail
/// est-il en train de tourner sous cette tâche ? »* ; elle était un terme de la
/// question *« ai-je le droit d'agir sur cette PR ? »*, qui est une autre garde.
/// Cette fonction ne prend donc **aucun booléen** : elle lit un verdict de
/// vivacité, et rien d'autre.
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
/// | coût (mika#2653) | **un appel d'outil refusé** | **71 tours jetés** (mesuré) |
/// | coût (mika#2763) | **un appel d'outil refusé** | **un groom entier perdu, et le ticket victime bloqué `in_progress` sans enfant** (mesuré) |
/// | visibilité | `tool_calls.output` + un WARN + une ligne d'audit | aucune ligne, le pilote meurt |
/// | boucle ? | **non** — un refus d'outil ne se rejoue pas tout seul, et aucune garde `required_tools` ne réclame `cancel_task` | oui — le défaut se rejoue à chaque dispatch refusé |
/// | rattrapage | le modèle rend la main, le callback arrive de lui-même | aucun : le travail est détruit |
///
/// L'asymétrie ne tient pas à la classe du tour : elle tient à ce qu'un refus
/// est réversible et qu'un kill ne l'est pas. C'est pourquoi retirer le terme de
/// classe ne la déplace pas.
///
/// # Et le geste de reprise de l'opérateur n'est PAS sur ce chemin
///
/// Cette propriété était la rectification R5 de mika#2653, et c'est elle qui
/// rend l'élargissement de mika#2763 sans coût pour l'opérateur. Le ticket
/// mika#2653 écrivait *« `cancel_task` est le geste de reprise le plus court de
/// l'opérateur ; un faux refus le lui retire »* — **c'est faux pour cette
/// garde**, et ça ne dépendait déjà pas du terme de classe.
/// `cancel_task_and_kill` a quatre appelants et **un seul traverse un
/// `ToolContext`** : `mika tasks cancel <id>` (CLI, `mika-cli/src/commands/
/// tasks.rs`), `POST /tasks/{id}/cancel` (HTTP, `server/handlers.rs`) et les
/// tests eval mika#2335 n'y arrivent pas. Ce qu'un faux refus retire est un
/// appel d'outil à un **modèle**, rendu visible dans `tool_calls.output` et dans
/// `audit_events` — jamais une commande d'opérateur.
///
/// # Coût du fail-closed, nommé plutôt que découvert
///
/// Une ligne portant un `process_id` **sans** `process_start_time` lisible ne
/// pourra pas être annulée **par un modèle**, dans aucune classe de tour
/// désormais (c'est l'élargissement, et il est nommé ici plutôt que découvert
/// en production). C'est la population que mika#2335 compte sous
/// `unusable_child_count`, celle que le faucheur mika#2249 **décline** et que le
/// watchdog #959 ne peut pas juger — donc le refus peut durer jusqu'à ce que
/// `timeout_at` (panic-fallback 6 h) rende la ligne terminale. Borné, mesuré par
/// sa propre valeur d'audit, et les deux chemins opérateur ci-dessus restent
/// entiers. **L'arbitrage est local et ne se transporte pas.**
pub(crate) fn classify_cancel_on_live_pilot(verdict: &LivePilotVerdict) -> CancelPilotDecision {
    match verdict {
        LivePilotVerdict::Alive {
            child_task_id, pid, ..
        } => CancelPilotDecision::RefusedLivePilot {
            child_task_id: child_task_id.clone(),
            pid: *pid,
        },
        LivePilotVerdict::None => CancelPilotDecision::AllowedNoPilot,
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
///
/// # Ce que mika#2763 a changé dans le texte
///
/// La justification *« ce tour a été ouvert par un événement de pull-request »*
/// est **retirée des deux bras** : elle était fausse dès que la garde a cessé de
/// consulter la classe du tour, et un refus dont le motif est faux est un refus
/// qu'un modèle a raison de contourner.
///
/// À sa place, le bras `RefusedLivePilot` **nomme la prémisse réfutée** du
/// second incident : *« aucune sortie, aucune PR, aucun plan » est l'état normal
/// d'un dispatch en vol, pas une preuve de péremption*. C'est littéralement le
/// raisonnement que l'agent du 2026-10-17 a tenu — et il l'a tenu sur l'âge du
/// **parent**, parce que `check_task` refuse de lire un callback. Dire au modèle
/// que ce signal ne prouve rien est ce qui empêche le refus d'être lu comme un
/// obstacle à franchir.
fn refusal_body(decision: &CancelPilotDecision, task_id: &str) -> String {
    match decision {
        CancelPilotDecision::RefusedLivePilot { child_task_id, pid } => serde_json::json!({
            "error": ERROR_CANCEL_LIVE_PILOT,
            "doctrine": "mika#2653, mika#2763",
            "task": task_id,
            "pilot_row": child_task_id,
            "pilot_pid": pid,
            "reason":
                "A pilot is alive and working under this task: the process is running right \
                 now. Cancelling here throws away every turn it has produced. Note what is \
                 NOT evidence of staleness: no output, no pull request and no Plan callout \
                 are the normal state of a dispatch in flight, and the age you can read on \
                 a parent task is not the age of this one.",
            "remedy":
                "Two exits, and they are the only two. (1) Let the pilot finish: its \
                 callback arrives on its own, and the work is then visible. (2) If this \
                 dispatch genuinely looks lost, say so and stop — report what you observed \
                 and let a human decide. Freeing an occupied dispatch slot is never your \
                 task: a deferred launcher already waits for it to be released. Lifting \
                 this refusal belongs to an operator, on the host; it is not lifted by \
                 retrying under a different shape.",
        })
        .to_string(),
        CancelPilotDecision::RefusedUnreadable { reason } => serde_json::json!({
            "error": ERROR_CANCEL_PILOT_UNREADABLE,
            "doctrine": "mika#2653, mika#2763",
            "task": task_id,
            "signal": reason,
            "reason":
                "Whether a pilot is alive under this task could not be established, and a \
                 signal that cannot be read is never a satisfied term. The cost of \
                 cancelling work that is in fact running is a whole dispatch thrown away, \
                 and it is not recoverable.",
            "remedy":
                "Two exits, and they are the only two. (1) Proceed with what this turn \
                 actually asks of you and leave the task alone. (2) Report that liveness \
                 could not be established and stop. Lifting this refusal belongs to an \
                 operator, on the host.",
        })
        .to_string(),
        // `AllowedNoPilot` ne compose pas de corps — le `match` reste exhaustif
        // pour qu'une quatrième issue soit tranchée ici.
        CancelPilotDecision::AllowedNoPilot => String::new(),
    }
}

/// Journaliser et auditer une décision de la garde (motif
/// `report_event_target_binding`, mika#2649).
///
/// Trois noms d'événement pour deux conduites de refus et un contrôle positif —
/// chacun au **niveau de son régime attendu**, un WARN sur une population
/// nominale étant un WARN qu'on finit par museler.
///
/// **Toute décision s'écrit, depuis mika#2763.** `audit_value()` est totale, il
/// n'y a plus de bras muet : le `GROUP BY after_value` du § 11.2 rend donc
/// désormais le compte de **tous** les appels à `cancel_task` passés par un
/// outil, pas seulement ceux d'un tour webhook PR. C'est ce qui rend le défaut
/// de mika#2763 détectable à la lecture — un passage sans ligne était
/// auparavant indiscernable d'une garde qui ne tournait pas (classe mika#2205).
///
/// Le churn est borné par le trafic réel de l'outil : `cancel_task` est un geste
/// rare (quelques appels par jour au plus), à comparer aux populations que la
/// doctrine mika#2131 borne, qui sont des boucles par tour.
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
    let audit_value = decision.audit_value();

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
                "refused to cancel a task whose pilot is alive (mika#2653, mika#2763 — \
                 every turn class)"
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
                "refused to cancel: pilot liveness could not be established (mika#2653, \
                 mika#2763 — every turn class)"
            );
        }
        // Renommé par mika#2763 : l'ancien `cancel_task_webhook_turn_allowed`
        // nommait la classe du tour, qui n'est plus un terme. Migration de nom
        // pour l'opérateur dans le corps de PR — avant / après, pas de double
        // écriture.
        CancelPilotDecision::AllowedNoPilot => {
            tracing::info!(
                event = "cancel_task_pilot_absent",
                agent_id = %ctx.db.agent_id(),
                session_id = %ctx.session_id,
                trace_id = %ctx.trace_id,
                task_id = %task_id,
                "no pilot under this task — the guard ran and let the cancellation \
                 through (mika#2653 positive control)"
            );
        }
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

        // ── Garde mika#2653, inconditionnelle depuis mika#2763 ──
        //
        // Placement : **après** la validation d'existence et de portée (la tâche
        // doit exister et appartenir à l'agent avant qu'on parle de son pilote),
        // **avant** `cancel_task_and_kill` — donc avant toute écriture de statut
        // et avant tout signal.
        //
        // **Dans l'outil, jamais dans `cancel_task_and_kill`** : c'est ce qui
        // préserve les deux chemins opérateur (CLI, HTTP) **par construction**
        // plutôt que par prédicat, et c'est la différence avec une garde posée
        // dans l'infrastructure de kill.
        //
        // **`cancel_reminder` est couvert gratuitement** : `CancelReminderTool`
        // délègue littéralement à `CancelTaskTool.execute`. À nommer plutôt
        // qu'à découvrir — un futur éditeur qui dédoublerait `cancel_reminder`
        // perdrait la garde en silence.
        //
        // **Aucun `if` de classe de tour ici, et c'est le correctif.** Le
        // `if ctx.is_webhook_pr_event_turn` qui enveloppait ce bloc est ce par
        // quoi l'incident du 2026-10-17 est passé : le booléen n'est dérivé
        // qu'au site conversationnel, les tours silencieux le posent à `false`
        // en dur, donc la garde ne tournait pas du tout sur le chemin
        // `deferred_dispatch`. Le verdict de vivacité est maintenant résolu à
        // **chaque** appel — une lecture, sur un geste rare, contre un vol
        // entier.
        let verdict = live_pilot_for_task(ctx.db, id).await;
        let decision = classify_cancel_on_live_pilot(&verdict);
        report_cancel_pilot_decision(&decision, id, ctx).await;

        if decision.refuses() {
            return Ok(ToolOutput::error(refusal_body(&decision, id)));
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

    // ──────────── mika#2653 / mika#2763 — la garde, trois issues ────────────

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

    /// **V1** — les trois issues du prédicat pur.
    ///
    /// Chaque terme est exercé seul : une conjonction de termes fail-safe ne se
    /// prouve pas en les neutralisant tous ensemble (leçon mika#2277).
    ///
    /// Trois et non quatre depuis mika#2763 : la quatrième issue était
    /// `Allowed`, l'issue muette hors tour webhook PR, et elle disparaît avec le
    /// terme de classe de tour qui la produisait.
    #[test]
    fn mika2763_les_trois_issues_du_predicat() {
        assert_eq!(
            classify_cancel_on_live_pilot(&alive()),
            CancelPilotDecision::RefusedLivePilot {
                child_task_id: "8a3b2082-0000-0000-0000-000000000000".to_string(),
                pid: 4242,
            },
            "pilote vif ⇒ refus : c'est le défaut mesuré, deux fois"
        );
        assert_eq!(
            classify_cancel_on_live_pilot(&LivePilotVerdict::None),
            CancelPilotDecision::AllowedNoPilot,
            "aucun pilote ⇒ passe, et le dit (contrôle positif)"
        );
        for reason in [UNREADABLE_DB_ERROR, UNREADABLE_NO_START_TIME] {
            assert_eq!(
                classify_cancel_on_live_pilot(&LivePilotVerdict::Unreadable { reason }),
                CancelPilotDecision::RefusedUnreadable { reason },
                "fail-CLOSED sur `{reason}` : l'arbitrage du § 3 est inverse de celui \
                 de `live_pilot`, et l'asymétrie de coût le décide"
            );
        }
    }

    /// **Remplace** `mika2653_hors_tour_webhook_pr_rien_ne_change`, et porte sa
    /// raison plutôt que son assertion.
    ///
    /// Ce test-là affirmait qu'un verdict `Alive` hors tour webhook PR rendait
    /// `Allowed` — c'est-à-dire exactement le passage par lequel l'incident du
    /// 2026-10-17 a tué un pilote vif de 4 minutes. L'assertion est donc
    /// **renversée, pas désactivée** : la laisser sous `#[ignore]` ou la
    /// supprimer en silence laisserait croire que la propriété tient encore.
    ///
    /// Ce qui est épinglé maintenant : la signature de la fonction ne comporte
    /// **aucun terme de classe de tour**, donc la propriété « hors tour webhook
    /// PR, rien ne change » n'est plus exprimable — et ce test l'atteste en
    /// montrant que le verdict de vivacité décide **seul**, sur les quatre
    /// verdicts possibles. Un futur éditeur qui rajouterait un booléen de classe
    /// devrait réécrire ce corps, ce qui est le signal voulu.
    ///
    /// Le contrôle de **production** correspondant est V6c du fichier eval
    /// (`mika2653_un_tour_webhook_pr_sans_pilote_annule_et_le_dit`) : c'est lui,
    /// désormais seul, qui sépare « la garde décide » de « la garde bloque toute
    /// annulation ».
    #[test]
    fn mika2763_la_classe_du_tour_nentre_plus_dans_le_predicat() {
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
            let decision = classify_cancel_on_live_pilot(&verdict);
            let expected_refusal = !matches!(verdict, LivePilotVerdict::None);
            assert_eq!(
                decision.refuses(),
                expected_refusal,
                "INVARIANT VIOLÉ (mika#2763) : sur {verdict:?} la décision ne suit pas la \
                 vivacité seule — si une classe de tour est revenue dans le prédicat, \
                 c'est la porte du 2026-10-17 qui se réouvre"
            );
            // Et toute décision s'audite : plus d'issue muette, donc plus de
            // population où « passé » et « pas tourné » rendent les mêmes
            // octets (classe mika#2205).
            assert!(
                ALL_CANCEL_PILOT_GUARD_VERDICTS.contains(&decision.audit_value()),
                "{decision:?} produit une valeur d'audit non déclarée"
            );
        }
    }

    /// **Détecteur 3** — les quatre valeurs d'audit sont un format de fil : tout
    /// ce que la garde produit est déclaré, et tout ce qui est déclaré est
    /// atteignable.
    ///
    /// Les deux directions. Sans la seconde, une valeur pourrait être déclarée,
    /// groupée par un opérateur, et produite par rien — une colonne de zéros qui
    /// se lit comme une population saine.
    ///
    /// **Quatre valeurs pour trois issues, et ce n'est pas une incohérence**
    /// (mika#2763) : `RefusedUnreadable` en produit deux, parce que les deux
    /// causes d'illisibilité ont des remèdes opposés. Ce que mika#2763 a retiré
    /// est l'issue **muette**, pas une valeur d'audit — les quatre littéraux de
    /// [`ALL_CANCEL_PILOT_GUARD_VERDICTS`] sont inchangés, et le `GROUP BY` d'un
    /// opérateur ne bouge pas. Le `filter_map` d'avant est devenu un `map` :
    /// `audit_value()` est totale.
    #[test]
    fn mika2653_les_valeurs_daudit_sont_un_format_de_fil() {
        let produced: Vec<&'static str> = [
            classify_cancel_on_live_pilot(&alive()),
            classify_cancel_on_live_pilot(&LivePilotVerdict::None),
            classify_cancel_on_live_pilot(&LivePilotVerdict::Unreadable {
                reason: UNREADABLE_DB_ERROR,
            }),
            classify_cancel_on_live_pilot(&LivePilotVerdict::Unreadable {
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
        let live = refusal_body(&classify_cancel_on_live_pilot(&alive()), "task-0000-0000");
        let unreadable = refusal_body(
            &classify_cancel_on_live_pilot(&LivePilotVerdict::Unreadable {
                reason: UNREADABLE_NO_START_TIME,
            }),
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

        // L'issue autorisante ne compose rien.
        assert!(
            refusal_body(&CancelPilotDecision::AllowedNoPilot, "task-0000-0000").is_empty(),
            "une décision qui laisse passer n'a pas de corps de refus à rendre"
        );
    }

    /// **AC8** — le refus nomme sa levée et **aucun contournement**.
    ///
    /// `MIKA_DEV_IDENTITY` porte `shell-exec`, donc nommer la commande
    /// d'annulation dans le corps du refus donnerait au modèle le gabarit du
    /// contournement — *un refus qui donne le gabarit est une fuite avec une
    /// étape de plus* (doctrine mika#2520).
    ///
    /// **Étendu par mika#2763** : le corps ne doit pas non plus justifier le
    /// refus par la classe du tour, qui n'est plus un terme — un refus dont le
    /// motif est faux est un refus qu'un modèle a raison de contourner.
    #[test]
    fn mika2653_le_refus_ne_nomme_aucun_contournement() {
        for decision in [
            classify_cancel_on_live_pilot(&alive()),
            classify_cancel_on_live_pilot(&LivePilotVerdict::Unreadable {
                reason: UNREADABLE_DB_ERROR,
            }),
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
            // mika#2763 — plus aucune justification par la classe du tour.
            for stale_premise in [
                "this turn was opened by a github",
                "check-suite event",
                "pull-request event",
            ] {
                assert!(
                    !lowered.contains(stale_premise),
                    "le refus justifie encore par la classe du tour (`{stale_premise}`), \
                     qui n'est plus un terme depuis mika#2763 : {body}"
                );
            }
        }

        // Et le bras `Alive` nomme la prémisse que l'agent du 2026-10-17 a
        // prise pour une preuve de péremption.
        let live = refusal_body(&classify_cancel_on_live_pilot(&alive()), "task-0000-0000");
        let lowered = live.to_ascii_lowercase();
        for premise in ["no output", "no pull request", "no plan callout", "parent"] {
            assert!(
                lowered.contains(premise),
                "le refus doit réfuter « {premise} » : c'est le raisonnement exact du \
                 tour qui a tué le groom de mika#2757, et un refus qui ne le nomme pas \
                 laisse le modèle le retenir : {live}"
            );
        }
    }

    // ───── mika#2763 — la garde ne consulte plus la classe de tour ─────

    /// Un pilote **réellement vif**, et c'est un processus enfant — jamais le
    /// nôtre — nettoyé par RAII.
    ///
    /// # Pourquoi pas `std::process::id()`
    ///
    /// La sœur `complete_task` l'emploie sans danger : sa garde n'atteint aucun
    /// chemin de kill. Ici U1 **traverse** `cancel_task_and_kill` dès que la
    /// garde ne mord pas — et c'est très exactement l'état d'avant mika#2763,
    /// donc l'état dans lequel ce test a été vu rouge. Or ce chemin envoie un
    /// SIGTERM au **groupe de processus** du pgid enregistré : avec notre propre
    /// pid, le test rouge tuerait la campagne qui l'exécute, de façon dépendante
    /// de l'environnement.
    ///
    /// `process_group(0)` fait de l'enfant son **propre** chef de groupe, donc
    /// le pgid enregistré en base en est un véritable. Le `Drop` est ce qu'un
    /// nettoyage en dernière ligne n'offre pas : U1 et U2 sont des refus, donc
    /// rien d'autre ne signale l'enfant, et une assertion qui panique y
    /// abandonnerait un `sleep 120`. Il `waitpid` aussi, faute de quoi un enfant
    /// tué reste un zombie pour la vie du binaire de test.
    ///
    /// Même idiome que `tests/eval/test_cancel_task_live_pilot_2653.rs`. Les
    /// deux copies vivent de part et d'autre de la frontière
    /// crate / test d'intégration, qu'aucun helper ne traverse.
    #[cfg(target_os = "linux")]
    struct LivePilotChild {
        child: std::process::Child,
        pid: i64,
        start_time: u64,
    }

    #[cfg(target_os = "linux")]
    impl LivePilotChild {
        fn spawn() -> Self {
            use std::os::unix::process::CommandExt;
            let child = std::process::Command::new("sleep")
                .arg("120")
                .process_group(0)
                .spawn()
                .expect("spawn sleep child");
            let pid = child.id();
            let start_time = crate::task_engine::process_liveness::read_process_start_time(pid)
                .expect("read child start time");
            Self {
                child,
                pid: i64::from(pid),
                start_time,
            }
        }

        fn is_alive(&self) -> bool {
            crate::task_engine::process_liveness::is_same_process_alive(
                u32::try_from(self.pid).expect("pid fits in u32"),
                self.start_time,
            )
        }
    }

    #[cfg(target_os = "linux")]
    impl Drop for LivePilotChild {
        fn drop(&mut self) {
            // Le groupe d'abord (l'enfant en est le chef), puis le processus
            // seul — même ordre et même raison que `kill_process_gracefully`.
            for target in [format!("-{}", self.pid), self.pid.to_string()] {
                let _ = std::process::Command::new("kill")
                    .arg("-KILL")
                    .arg(target)
                    .output();
            }
            let _ = self.child.wait();
        }
    }

    /// Une ligne callback de dispatch portant le pgid d'un pilote, telle que
    /// l'exécuteur l'écrit (la topologie à deux lignes de `live_pilot`).
    #[cfg(target_os = "linux")]
    async fn add_groom_callback_with_pilot(
        harness: &TestHarness,
        pid: i64,
        start_time: Option<u64>,
    ) -> String {
        let id = add_callback_task(harness, "long_running:run_claude_pilot_groom").await;
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

    /// **U1 (AC1)** — l'incident de mika#2763, reproduit puis refermé : un
    /// pilote groom vif, et `cancel_task` appelé **hors** tour webhook PR.
    ///
    /// **Vu rouge avant le correctif** : posé sur le `cancel_task.rs` d'avant,
    /// le `if ctx.is_webhook_pr_event_turn` du site d'appel écarte la garde,
    /// l'annulation part, `cancel_task_and_kill` SIGTERM le groupe et l'enfant
    /// meurt — c'est l'incident du 2026-10-17 07:59:38Z rejoué. Un test qu'on
    /// n'a jamais vu rouge n'atteste rien (motif `grooming_marker`, mika#2158).
    ///
    /// `harness.ctx()` pose `is_webhook_pr_event_turn: false`, comme les vingt
    /// sites de construction qui ne le calculent pas — dont le site silencieux,
    /// celui du tour `deferred_dispatch` de l'incident.
    #[tokio::test]
    #[cfg(target_os = "linux")]
    async fn mika2763_un_pilote_vif_est_refuse_hors_tour_webhook_pr() {
        let harness = TestHarness::new();
        let pilot = LivePilotChild::spawn();
        let task_id =
            add_groom_callback_with_pilot(&harness, pilot.pid, Some(pilot.start_time)).await;

        let ctx = harness.ctx();
        assert!(
            !ctx.is_webhook_pr_event_turn,
            "la fixture doit être HORS de la population de mika#2653 — c'est tout \
             ce que ce test mesure"
        );

        let output = CancelTaskTool
            .execute(serde_json::json!({"id": task_id}), &ctx)
            .await
            .unwrap();

        assert!(
            output.is_error,
            "un pilote vif doit interdire l'annulation dans TOUTE classe de tour : \
             {}",
            output.content
        );
        assert!(
            output.content.contains(ERROR_CANCEL_LIVE_PILOT),
            "le refus doit porter son motif de fil : {}",
            output.content
        );
        assert!(
            output.content.contains(&pilot.pid.to_string()),
            "le refus doit nommer le PID, qui est ce qu'un opérateur corrèle : {}",
            output.content
        );

        let task = harness.db.get_task(&task_id).await.unwrap().unwrap();
        assert_eq!(
            task.status, "pending",
            "INVARIANT VIOLÉ : la garde a laissé écrire un statut — elle tourne \
             AVANT `cancel_task_and_kill`, donc avant toute écriture et avant tout \
             signal"
        );

        // En apparence trivial, et c'est pourtant le seul contrôle qui dise que
        // le travail survit, puisque c'est le travail que le défaut détruisait.
        assert!(
            pilot.is_alive(),
            "le pilote a été signalé : la garde a laissé passer le kill"
        );
    }

    /// **U2 (AC2)** — le fail-CLOSED sur l'illisible, lui aussi hors tour
    /// webhook PR : un pgid présent dont le `process_start_time` manque.
    ///
    /// La paire qui identifie une *instance* est incomplète, donc la vivacité
    /// n'est pas prouvable, donc l'annulation est refusée — et mika#2763 ne
    /// touche pas cet arbitrage, il en élargit seulement la population.
    #[tokio::test]
    #[cfg(target_os = "linux")]
    async fn mika2763_une_vivacite_illisible_refuse_hors_tour_webhook_pr() {
        let harness = TestHarness::new();
        let pilot = LivePilotChild::spawn();
        let task_id = add_groom_callback_with_pilot(&harness, pilot.pid, None).await;

        let ctx = harness.ctx();
        let output = CancelTaskTool
            .execute(serde_json::json!({"id": task_id}), &ctx)
            .await
            .unwrap();

        assert!(
            output.is_error && output.content.contains(ERROR_CANCEL_PILOT_UNREADABLE),
            "un signal qu'on ne peut pas lire n'est jamais un terme satisfait : {}",
            output.content
        );

        let task = harness.db.get_task(&task_id).await.unwrap().unwrap();
        assert_eq!(task.status, "pending");
        assert!(pilot.is_alive());
    }
}
