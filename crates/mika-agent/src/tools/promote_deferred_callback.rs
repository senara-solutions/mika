use anyhow::Result;
use async_trait::async_trait;
use mika_common::claude::ToolDefinition;
use serde_json::Value;

use super::{Tool, ToolContext, ToolOutput};
use crate::db::ForcePromoteResult;

/// Known dispatch classes (keep in sync with `DISPATCH_CLASSES` in engine.rs
/// and `derive_dispatch_class` in skills/executor.rs).
const VALID_DISPATCH_CLASSES: &[&str] = &["implement", "groom"];

// ───────── Télémétrie et format de fil de la garde (mika#2654 § 4) ─────────

/// Le `tool_name` sous lequel chaque refus de la garde est audité.
///
/// **Quatrième nom de la famille `deferred_dispatch_force_promote_*`**, dont
/// mika#1453 a posé les trois premiers (`_succeeded`, `_rejected_slot_busy`,
/// `_override`). Un nom propre plutôt qu'une quatrième valeur de `after_value`
/// sous un nom partagé : les trois noms existants répondent à « la promotion
/// a-t-elle eu lieu, et sinon pourquoi » — une question sur le **créneau** —
/// alors que celui-ci répond à « qui a demandé », une question sur la **classe
/// de tour**. Ils ne se soustraient pas, donc ils ne se groupent pas.
///
/// **SOLE WRITER**, épinglé par
/// `canonical_tokens::tests::mika2654_le_nom_daudit_a_un_seul_ecrivain` — c'est
/// cette propriété qui rend la sonde S1 du § 11 un compte exact plutôt qu'un
/// nombre sur lequel deux sites peuvent diverger.
///
/// # Le needle du scan est ce nom ENTIER, et c'est mesuré
///
/// Les trois noms frères partagent le préfixe `deferred_dispatch_force_promote`,
/// donc un scan sur ce préfixe trouverait trois écrivains et serait **rouge à la
/// naissance** — et un lint rouge à la naissance se fait désarmer. Le needle est
/// donc le nom entier, et la comparaison est **exacte** plutôt qu'en
/// sous-chaîne : c'est elle qui empêche un nom *étendant* celui-ci — la forme
/// que prennent les résidus de cette famille, `cancel_task_pilot_guard` →
/// `cancel_task_pilot_guard_audit_failed` chez la sœur — de compter comme un
/// second écrivain de la ligne d'audit. C'est la correction que mika#2649, puis
/// mika#2634, puis mika#2653 ont dû écrire chacun sur son scan jumeau ;
/// `audit_name_literal_present` compare `lit.trim() == needle`, et c'est la
/// bonne forme.
///
/// Le résidu de **cette** garde, lui, s'appelle
/// `promote_deferred_guard_audit_failed` : il ne partage pas ce nom, donc il ne
/// pèse sur rien ici — ce qui est une propriété des noms choisis, pas du scan.
pub(crate) const FORCE_PROMOTE_TURN_GUARD_AUDIT_TOOL: &str =
    "deferred_dispatch_force_promote_refused_webhook_pr_turn";

/// La seule valeur que `after_value` prend sous ce nom d'audit.
pub(crate) const AUDIT_REFUSED_WEBHOOK_PR_TURN: &str = "refused_webhook_pr_turn";

/// Toutes les valeurs que `after_value` peut prendre, pour l'épinglage du
/// format de fil (V6).
///
/// **Une seule, et le registre existe quand même.** Il reste en production
/// plutôt que derrière `#[cfg(test)]` pour la raison qu'`ALL_MARKER_CLASSES`
/// puis `ALL_CANCEL_PILOT_GUARD_VERDICTS` ont déjà dû écrire : le registre d'un
/// format de fil est ce qu'un opérateur lit pour savoir ce qu'un `GROUP BY` peut
/// rendre, et un registre qui n'existe que sous `cfg(test)` est un registre
/// qu'un lecteur de ce fichier ne trouve pas. Une seconde valeur — une seconde
/// cause de refus — se déclare ici et devient immédiatement une colonne que la
/// sonde S1 sait séparer.
#[allow(dead_code)]
pub(crate) const ALL_FORCE_PROMOTE_TURN_GUARD_VERDICTS: &[&str] = &[AUDIT_REFUSED_WEBHOOK_PR_TURN];

/// Préfixe que le corps du refus ouvre, tel que le modèle le reçoit.
///
/// **Un format de fil, avec un seul site de définition** — motif
/// [`crate::tools::TESTIMONY_ACCESS_REFUSAL_PREFIX`] (mika#2627 D5) et
/// `REFUSED (cwd-guard, mika#2536)`. Un `ToolOutput::error` atterrit dans
/// `tool_calls.output` (persisté, scrubbé), donc ce préfixe est ce qui rend
///
/// ```sql
/// SELECT count(*) FROM tool_calls
///  WHERE tool_name = 'promote_deferred_callback'
///    AND output LIKE 'REFUS (webhook-pr-turn, mika#2654)%';
/// ```
///
/// exact — la contrepartie côté outil de la sonde S1 côté `audit_events`, qui
/// reste la surface comptable de référence (§ 11).
pub(crate) const PROMOTE_REFUSAL_PREFIX: &str = "REFUS (webhook-pr-turn, mika#2654)";

/// Ce que la garde a décidé pour cet appel à `promote_deferred_callback`.
///
/// Deux issues et pas trois : il n'y a **pas** de contrôle positif à l'intérieur
/// du type, contrairement à `CancelPilotDecision::AllowedInPopulation`. La
/// raison est que la sœur mika#2653 a un second terme (la vivacité d'un pilote),
/// donc « la garde a tourné et a laissé passer » y est un état distinct de
/// « la garde n'a pas tourné » et doit être compté séparément. Ici le seul terme
/// **est** la classe de tour : dans la population, la garde refuse toujours.
/// Un `AllowedInPopulation` serait une valeur d'audit que rien n'écrit — une
/// colonne de zéros qui se lit comme une population saine, soit exactement ce
/// que la seconde direction de V6 refuse. Le contrôle positif de cette garde est
/// donc **hors du type** : c'est AC9, qui montre un tour `[GitHub] Issue …` qui
/// passe.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ForcePromoteTurnDecision {
    /// Hors tour webhook PR — **aucun corps, aucune ligne de journal, aucun
    /// audit**. C'est l'essentiel du trafic (58 sessions mesurées en 30 j, dont
    /// 31 hors canal github), soit très exactement le churn que la doctrine
    /// mika#2131 borne, et c'est ce qui rend la non-régression d'AC3
    /// **observable** : zéro ligne d'audit sur un tour d'opérateur.
    Allowed,
    /// Tour ouvert par un événement PR / check-suite : refusé.
    RefusedWebhookPrTurn,
}

impl ForcePromoteTurnDecision {
    /// La valeur de `after_value`, en `match` exhaustif **sans bras `_ =>`** :
    /// une troisième issue doit être tranchée ici par le compilateur, pas
    /// devinée. `Allowed` n'écrit rien, donc n'a pas de valeur d'audit.
    pub(crate) fn audit_value(&self) -> Option<&'static str> {
        match self {
            ForcePromoteTurnDecision::Allowed => None,
            ForcePromoteTurnDecision::RefusedWebhookPrTurn => Some(AUDIT_REFUSED_WEBHOOK_PR_TURN),
        }
    }
}

/// La garde de mika#2654, **pure** : un tour ouvert par un événement PR /
/// check-suite ne force pas la promotion d'un wrapper différé.
///
/// # Pourquoi le terme est un PARAMÈTRE et non une branche de l'appelant (AC6)
///
/// Forme mika#2649, reprise par mika#2653 : passer la classe de tour en
/// paramètre fait de « hors tour webhook PR, rien ne change » une **propriété
/// testable de la fonction** (V2, V6) plutôt qu'une lecture du site d'appel.
/// L'appelant branche malgré tout sur le drapeau — ce qui veut dire qu'en
/// production cette fonction ne reçoit jamais `false` et que la branche
/// `Allowed` n'y est pas atteignable. C'est la conséquence assumée du choix, et
/// elle est nommée plutôt que découverte : ce qu'elle achète est de rendre AC3
/// vérifiable sans base ni tour.
///
/// # Ce que le refus retire, et ce qu'il ne retire pas
///
/// Il retire **une avance ≤ 60 s** (`DB_SCAN_INTERVAL_TICKS = 60` : le backstop
/// `promote_pending_deferred_if_idle` promeut de lui-même au prochain scan
/// quand la classe a de la place) et le **contournement de la priorité
/// opérateur** (`has_pending_operator_task_for_class`, mika#1948 AC3), que
/// `force_promote_deferred_for_class` ne consulte pas et que le backstop
/// consulte — R1/R2 du plan. Il ne retire **rien** à l'opérateur : le CLI
/// `mika tasks promote-deferred` ne traverse aucun `ToolContext` (R5), donc
/// aucun prédicat de ce niveau ne peut l'atteindre.
///
/// # Pourquoi un refus et pas une retenue
///
/// Trois raisons, et la première est la porteuse : **le refus EST la mesure.**
/// Retirer l'outil du catalogue ne laisserait aucune trace comptable d'une
/// tentative, et c'est précisément l'information dont la halte transverse du
/// § 11 a besoin — « la garde ne mord jamais » et « la garde ne tourne pas »
/// rendraient les mêmes octets (mika#2205). Ensuite, c'est la forme de la garde
/// sœur `cancel_task` sur le même booléen. Enfin, aucun `required_tools`
/// n'exige cet outil et aucun prompt ne le prescrit (R5), donc le refus ne peut
/// pas boucler.
pub(crate) fn classify_force_promote_turn(
    is_webhook_pr_event_turn: bool,
) -> ForcePromoteTurnDecision {
    if is_webhook_pr_event_turn {
        ForcePromoteTurnDecision::RefusedWebhookPrTurn
    } else {
        ForcePromoteTurnDecision::Allowed
    }
}

/// Le corps du refus rendu au modèle.
///
/// **Il nomme la levée sans donner de gabarit** (doctrine mika#2520 /
/// mika#2292) : il porte le fait (ce tour a été ouvert par un événement PR /
/// check-suite), ce que forcer la promotion achèterait réellement (une avance
/// ≤ 60 s et le contournement de la priorité opérateur), le fait que **le
/// moteur promeut le wrapper de lui-même** au prochain scan, et la sortie
/// correcte (terminer le tour sur ce que l'événement demande).
///
/// Et il **ne nomme aucune commande** — ni le geste CLI de promotion, ni son
/// drapeau de forçage, ni `run_shell`, ni `update_task_status` : un refus qui
/// donne le gabarit est une fuite avec une étape de plus (mika#2520 /
/// mika#2292), et `MIKA_DEV_IDENTITY` porte `shell-exec`. La liste interdite
/// est épinglée par V5.
///
/// **Ce que V5 tient exactement, et c'est plus étroit qu'une garantie de
/// confinement :** le refus ne **restitue** pas le gabarit. Il ne le retire pas
/// du tour, parce que deux surfaces préexistantes le portent déjà et qu'aucune
/// n'appartient à ce travail — la **description** de l'outil, servie au
/// catalogue à chaque tour puisque l'outil est *refusé* et non *retenu*, et le
/// corps d'erreur `slot_busy`, tous deux citant
/// `mika tasks promote-deferred <class> --override`. Les retirer serait une
/// décision de surface de prompt, pas un corollaire de cette garde : elle est
/// nommée ici plutôt que présumée, pour qu'un lecteur ne prenne pas V5 pour ce
/// qu'elle n'atteste pas.
///
/// `match` exhaustif **sans bras `_ =>`**, comme chez la sœur : `Allowed` ne
/// compose rien, et une troisième issue devra être tranchée ici.
fn refusal_body(decision: ForcePromoteTurnDecision, dispatch_class: &str) -> String {
    match decision {
        ForcePromoteTurnDecision::RefusedWebhookPrTurn => format!(
            "{PROMOTE_REFUSAL_PREFIX} — NOTHING WAS PROMOTED; the deferred wrapper for \
             class '{dispatch_class}' is untouched and still pending. This turn was opened \
             by a GitHub pull-request or check-suite event, which authorizes reviewing and \
             reporting on that pull request — not deciding which dispatch takes the \
             '{dispatch_class}' slot.\n\n\
             Forcing the promotion here buys exactly two things, and neither belongs to \
             this turn: a head start of at most one engine scan, and a bypass of the \
             operator-priority check that the engine itself honours. The engine promotes \
             this wrapper ON ITS OWN at its next scan, as soon as the class has room — so \
             not forcing it is the nominal path, not a delay you have to work around.\n\n\
             One exit, and it is the only one: finish this turn on what the event actually \
             asks of you — look at the pull request, report what you observed, and stop. \
             Lifting this refusal belongs to an operator, on the host; it is not lifted by \
             retrying under a different shape."
        ),
        // Hors population : aucun corps. Le `match` reste exhaustif pour
        // qu'une troisième issue soit tranchée ici par le compilateur.
        ForcePromoteTurnDecision::Allowed => String::new(),
    }
}

/// Rend le verdict de la garde **observable** : une ligne de journal, puis une
/// ligne d'audit sous [`FORCE_PROMOTE_TURN_GUARD_AUDIT_TOOL`].
///
/// # Tiré-oublié, et la signature est ce qui le garantit
///
/// Motif `report_cancel_pilot_decision` : le retour est `()`, donc **aucun
/// échec d'audit ne peut remonter jusqu'au verdict**. Un `?` ici ferait d'une
/// base saturée une promotion autorisée — la garde céderait précisément sous la
/// charge qui la rend utile. Ce que l'échec laisse derrière lui est nommé plutôt
/// que tu : le WARN résiduel `promote_deferred_guard_audit_failed` dit que la
/// ligne de journal a atterri et que la ligne d'audit non, donc que le `GROUP
/// BY` de la sonde S1 **sous-compte à partir de là**. On l'attend vide.
///
/// # « Rien hors de la population », porté ici et pas par l'appelant
///
/// L'`early return` sur `audit_value() == None` est ce qui tient la doctrine
/// mika#2131 : sur les 40 appels mesurés hors canal github, cette fonction
/// n'écrit **rien** — ni journal, ni audit. C'est aussi ce qui permet au site
/// d'appel de l'invoquer **inconditionnellement**, sans répéter le prédicat :
/// un second site qui reteste la classe de tour serait un second endroit où la
/// borne peut diverger.
async fn report_force_promote_turn_decision(
    decision: ForcePromoteTurnDecision,
    dispatch_class: &str,
    ctx: &ToolContext<'_>,
) {
    let Some(audit_value) = decision.audit_value() else {
        return;
    };

    match decision {
        ForcePromoteTurnDecision::RefusedWebhookPrTurn => {
            // INFO et non WARN : un modèle qui tente la promotion sur un tour
            // webhook fait une erreur de plan de dispatch, pas une panne. La
            // sœur mika#2653 journalise en WARN parce qu'un pilote vivant qu'on
            // tente de tuer est un incident ; ici le moteur promeut de lui-même
            // au scan suivant, donc le refus ne laisse aucun travail en
            // souffrance. Un WARN par refus aurait fait du bruit d'un défaut
            // refermé — et un WARN qu'on apprend à ignorer ne se lit plus.
            tracing::info!(
                event = "promote_deferred_refused_webhook_pr_turn",
                agent_id = %ctx.db.agent_id(),
                session_id = %ctx.session_id,
                trace_id = %ctx.trace_id,
                dispatch_class = %dispatch_class,
                "refused to force-promote a deferred wrapper on a turn opened by a \
                 GitHub PR / check-suite event; the engine promotes it on its own at \
                 its next scan (mika#2654)"
            );
        }
        // Hors population : `audit_value()` a rendu `None` et la fonction a
        // déjà rendu la main. Le `match` reste exhaustif, sans bras `_ =>`,
        // pour qu'une troisième issue soit tranchée ici par le compilateur.
        ForcePromoteTurnDecision::Allowed => {}
    }

    if let Err(e) = ctx
        .db
        .log_audit_event(
            ctx.session_id,
            FORCE_PROMOTE_TURN_GUARD_AUDIT_TOOL,
            &format!("dispatch_class:{dispatch_class}"),
            None,
            Some(audit_value),
            None,
            Some(ctx.trace_id),
        )
        .await
    {
        tracing::warn!(
            event = "promote_deferred_guard_audit_failed",
            error = %e,
            dispatch_class = %dispatch_class,
            verdict = %audit_value,
            "the log line landed but its audit row did not; the GROUP BY undercounts \
             from here"
        );
    }
}

pub struct PromoteDeferredCallbackTool;

#[async_trait]
impl Tool for PromoteDeferredCallbackTool {
    fn name(&self) -> &str {
        "promote_deferred_callback"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "promote_deferred_callback".to_string(),
            description:
                "Force-promote the next pending deferred dispatch wrapper for a given dispatch class. \
                Fail-closed: if the per-class dispatch slot is already occupied by an active callback, \
                the promotion is refused. In that case, surface the rejection to the operator via \
                send_message — the operator can use `mika tasks promote-deferred <class> --override` \
                to cancel the blocker and force promotion. \
                Not available on a turn opened by a GitHub pull-request or check-suite event: \
                such a turn does not decide which dispatch takes the slot, and the engine \
                promotes the next deferred wrapper on its own at its next scan (mika#2654)."
                    .to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "dispatch_class": {
                        "type": "string",
                        "description": "The dispatch class to promote a deferred wrapper for: 'implement' or 'groom'",
                        "enum": ["implement", "groom"]
                    }
                },
                "required": ["dispatch_class"]
            }),
        }
    }

    async fn execute(&self, input: Value, ctx: &ToolContext<'_>) -> Result<ToolOutput> {
        let dispatch_class = input["dispatch_class"].as_str().unwrap_or("").trim();
        if dispatch_class.is_empty() {
            return Ok(ToolOutput::error("'dispatch_class' is required."));
        }
        if !VALID_DISPATCH_CLASSES.contains(&dispatch_class) {
            return Ok(ToolOutput::error(format!(
                "Invalid dispatch_class '{dispatch_class}'. Must be one of: {}",
                VALID_DISPATCH_CLASSES.join(", ")
            )));
        }

        // ── Garde mika#2654 ──
        //
        // Placement : **après** la validation de `dispatch_class` (une entrée
        // malformée lit la même erreur qu'ailleurs — le refus ne la masque
        // pas), et **avant** `force_promote_deferred_for_class` — donc avant
        // toute lecture et toute écriture en base. C'est l'ordre qu'AC2 exige :
        // un tour PR dont la classe est au plafond est refusé sous le motif
        // webhook, **jamais** sous `slot_busy`, parce que le second lirait
        // comme un problème de créneau et orienterait vers le forçage.
        //
        // **Dans l'outil, jamais dans `force_promote_deferred_for_class`** :
        // c'est ce qui préserve **par construction** le CLI
        // `mika tasks promote-deferred` et le backstop périodique, qui
        // partagent cette fonction de base et ne traversent aucun
        // `ToolContext` (R5).
        let decision = classify_force_promote_turn(ctx.is_webhook_pr_event_turn);
        // Appelé **inconditionnellement** : c'est la fonction elle-même qui
        // rend la main sans rien écrire hors de la population (mika#2131). Un
        // `if` ici répéterait le prédicat et ouvrirait un second endroit où la
        // borne peut diverger de celle du refus.
        report_force_promote_turn_decision(decision, dispatch_class, ctx).await;
        match decision {
            ForcePromoteTurnDecision::RefusedWebhookPrTurn => {
                return Ok(ToolOutput::error(refusal_body(decision, dispatch_class)));
            }
            // Hors population : rien n'a été composé, rien n'a été écrit, et
            // la suite est octet pour octet celle d'avant (AC3).
            ForcePromoteTurnDecision::Allowed => {}
        }

        let result = ctx
            .db
            .force_promote_deferred_for_class(
                dispatch_class,
                crate::skills::executor::max_concurrent_for_class(dispatch_class),
            )
            .await?;

        match result {
            ForcePromoteResult::Promoted { task_id } => {
                ctx.db
                    .log_audit_event(
                        ctx.session_id,
                        "deferred_dispatch_force_promote_succeeded",
                        &format!("dispatch_class:{dispatch_class}"),
                        None,
                        Some(&format!("promoted:{task_id}")),
                        None,
                        Some(ctx.trace_id),
                    )
                    .await?;

                Ok(ToolOutput::success(format!(
                    "Deferred wrapper promoted for class '{dispatch_class}'. Promoted task ID: {task_id}"
                )))
            }
            ForcePromoteResult::RejectedSlotBusy { blocking_label } => {
                ctx.db
                    .log_audit_event(
                        ctx.session_id,
                        "deferred_dispatch_force_promote_rejected_slot_busy",
                        &format!("dispatch_class:{dispatch_class}"),
                        None,
                        Some("rejected_slot_busy"),
                        Some(&format!("blocker: {blocking_label}")),
                        Some(ctx.trace_id),
                    )
                    .await?;

                Ok(ToolOutput::error(serde_json::to_string(
                    &serde_json::json!({
                        "error": "slot_busy",
                        "dispatch_class": dispatch_class,
                        "blocking_label": blocking_label,
                        "message": format!(
                            "Cannot promote: dispatch slot for class '{dispatch_class}' is occupied by '{blocking_label}'. \
                            Surface this to the operator via send_message — they can use \
                            `mika tasks promote-deferred {dispatch_class} --override` to cancel the blocker and force promotion."
                        )
                    }),
                )?))
            }
            ForcePromoteResult::NoPendingWrapper => Ok(ToolOutput::error(format!(
                "No pending deferred wrapper for class '{dispatch_class}'."
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::NewTask;
    use crate::test_utils::test_helpers::TestHarness;

    fn deferred_callback_task(agent_id: &str, dispatch_class: &str) -> NewTask {
        NewTask {
            agent_id: agent_id.to_string(),
            team_run_id: None,
            parent_task_id: None,
            depth: 0,
            label: "long_running:run_claude_pilot:deferred".to_string(),
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
            dispatch_class: Some(dispatch_class.to_string()),
        }
    }

    fn real_callback_task(agent_id: &str, dispatch_class: &str) -> NewTask {
        NewTask {
            agent_id: agent_id.to_string(),
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
            dispatch_class: Some(dispatch_class.to_string()),
        }
    }

    #[tokio::test]
    async fn test_promote_happy_path() {
        let harness = TestHarness::new();
        let agent_id = &harness.db.agent_id;

        // Create a pending deferred wrapper
        harness
            .db
            .create_task(deferred_callback_task(agent_id, "implement"))
            .await
            .unwrap();

        let ctx = harness.ctx();
        let tool = PromoteDeferredCallbackTool;

        let result = tool
            .execute(serde_json::json!({"dispatch_class": "implement"}), &ctx)
            .await
            .unwrap();
        assert!(
            !result.is_error,
            "Expected success but got error: {}",
            result.content
        );
        assert!(
            result.content.contains("promoted"),
            "Should contain 'promoted': {}",
            result.content
        );
    }

    #[tokio::test]
    async fn test_promote_slot_busy() {
        let harness = TestHarness::new();
        let agent_id = &harness.db.agent_id;

        // Create a pending deferred wrapper
        harness
            .db
            .create_task(deferred_callback_task(agent_id, "implement"))
            .await
            .unwrap();

        // Create a pending real callback (occupies the slot — pending non-deferred
        // callbacks count as slot-occupying per has_any_active_callback_for_class)
        harness
            .db
            .create_task(real_callback_task(agent_id, "implement"))
            .await
            .unwrap();

        let ctx = harness.ctx();
        let tool = PromoteDeferredCallbackTool;

        let result = tool
            .execute(serde_json::json!({"dispatch_class": "implement"}), &ctx)
            .await
            .unwrap();
        assert!(result.is_error, "Expected error but got success");
        assert!(
            result.content.contains("slot_busy"),
            "Should contain 'slot_busy': {}",
            result.content
        );
    }

    #[tokio::test]
    async fn test_promote_no_pending_wrapper() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = PromoteDeferredCallbackTool;

        let result = tool
            .execute(serde_json::json!({"dispatch_class": "implement"}), &ctx)
            .await
            .unwrap();
        assert!(result.is_error, "Expected error but got success");
        assert!(
            result.content.contains("No pending deferred wrapper"),
            "Should mention no pending wrapper: {}",
            result.content
        );
    }

    #[tokio::test]
    async fn test_promote_invalid_class() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = PromoteDeferredCallbackTool;

        let result = tool
            .execute(serde_json::json!({"dispatch_class": "invalid"}), &ctx)
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("Invalid dispatch_class"));
    }

    #[tokio::test]
    async fn test_promote_missing_class() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = PromoteDeferredCallbackTool;

        let result = tool.execute(serde_json::json!({}), &ctx).await.unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("'dispatch_class' is required"));
    }

    // ───────── mika#2654 — la garde, et ses deux issues ─────────

    /// Un `ToolContext` de test sur un tour ouvert par un événement PR /
    /// check-suite.
    ///
    /// Le booléen est posé **à la main** : `TestHarness::ctx()` le laisse à
    /// `false`, exactement comme les trois sites de production qui ne servent
    /// pas de webhook (mode silencieux et agents d'équipe dans
    /// `agent_loop/mod.rs`, `server/investigate.rs`) — le **seul** site qui le
    /// calcule est celui de conversation. Ce qui veut dire que les trois tests
    /// n'attestent **pas** le câblage — le champ pourrait n'être jamais calculé
    /// au site de conversation et ils resteraient verts. C'est AC9 qui l'atteste,
    /// et c'est pourquoi la mutation M4 ne fait rougir que lui.
    fn webhook_pr_ctx(harness: &TestHarness) -> ToolContext<'_> {
        let mut ctx = harness.ctx();
        ctx.is_webhook_pr_event_turn = true;
        ctx
    }

    /// **V1 (AC1)** — le défaut refermé : un tour webhook PR ne force pas la
    /// promotion, et le wrapper n'a pas bougé.
    #[tokio::test]
    async fn mika2654_un_tour_webhook_pr_ne_force_pas_la_promotion() {
        let harness = TestHarness::new();
        let agent_id = &harness.db.agent_id;
        let wrapper = harness
            .db
            .create_task(deferred_callback_task(agent_id, "implement"))
            .await
            .unwrap();

        let ctx = webhook_pr_ctx(&harness);
        let result = PromoteDeferredCallbackTool
            .execute(serde_json::json!({"dispatch_class": "implement"}), &ctx)
            .await
            .unwrap();

        assert!(
            result.is_error,
            "la promotion doit être refusée : obtenu un succès — {}",
            result.content
        );
        assert!(
            result.content.starts_with(PROMOTE_REFUSAL_PREFIX),
            "le refus doit ouvrir sur son préfixe de format de fil, sans quoi le \
             compte de la sonde côté `tool_calls.output` est faux ; obtenu : {}",
            result.content
        );

        // **L'assertion porteuse.** `promote_next_deferred_callback_for_class`
        // passe la ligne à `completed` (c'est ce que « promouvoir » veut dire
        // ici), donc `pending` dit littéralement qu'aucun créneau n'a été
        // décidé — et pas seulement que l'outil a rendu une erreur.
        let task = harness
            .db
            .get_task(&wrapper)
            .await
            .expect("task readable")
            .expect("task exists");
        assert_eq!(
            task.status, "pending",
            "INVARIANT VIOLÉ (AC1) : le wrapper a changé d'état — la garde \
             tourne AVANT `force_promote_deferred_for_class`, donc avant toute \
             écriture en base"
        );
    }

    /// **V2 — LE contrôle négatif** : hors tour webhook PR, la promotion passe
    /// comme avant.
    ///
    /// Sans lui, « la garde décide » ne se distingue pas de « la garde bloque
    /// tout », et la seconde serait un correctif qui casse le chemin de reprise
    /// (`DeferredDispatch`, callbacks de pilote : 40 des 73 appels mesurés, sur
    /// 31 des 58 sessions) en ayant l'air de fermer le ticket.
    #[tokio::test]
    async fn mika2654_hors_tour_webhook_pr_la_promotion_passe() {
        let harness = TestHarness::new();
        let agent_id = &harness.db.agent_id;
        let wrapper = harness
            .db
            .create_task(deferred_callback_task(agent_id, "implement"))
            .await
            .unwrap();

        // Même fixture que V1, à un booléen près : c'est l'axe, et rien d'autre.
        let ctx = harness.ctx();
        let result = PromoteDeferredCallbackTool
            .execute(serde_json::json!({"dispatch_class": "implement"}), &ctx)
            .await
            .unwrap();

        assert!(
            !result.is_error,
            "INVARIANT VIOLÉ (AC3) : la garde a mordu hors de sa population — {}",
            result.content
        );
        assert!(
            !result.content.contains(PROMOTE_REFUSAL_PREFIX),
            "aucun refus hors population : {}",
            result.content
        );

        let task = harness
            .db
            .get_task(&wrapper)
            .await
            .expect("task readable")
            .expect("task exists");
        assert_eq!(
            task.status, "completed",
            "la promotion doit aboutir exactement comme avant"
        );
    }

    /// **V3 (AC2)** — l'ordre : classe au plafond sur un tour webhook PR ⇒
    /// refus **webhook**, jamais `slot_busy`.
    ///
    /// Fixture de `test_promote_slot_busy`, à un booléen près. Le motif
    /// importe autant que le refus : `slot_busy` se lit comme un problème de
    /// **créneau** et sa remédiation oriente vers le forçage, alors que la
    /// cause ici est la **classe de tour**. Et c'est aussi ce qui atteste que la
    /// garde précède tout accès base : sous l'ordre inverse, la base aurait
    /// répondu la première.
    #[tokio::test]
    async fn mika2654_la_classe_au_plafond_est_refusee_sous_le_motif_webhook() {
        let harness = TestHarness::new();
        let agent_id = &harness.db.agent_id;
        let wrapper = harness
            .db
            .create_task(deferred_callback_task(agent_id, "implement"))
            .await
            .unwrap();
        harness
            .db
            .create_task(real_callback_task(agent_id, "implement"))
            .await
            .unwrap();

        let ctx = webhook_pr_ctx(&harness);
        let result = PromoteDeferredCallbackTool
            .execute(serde_json::json!({"dispatch_class": "implement"}), &ctx)
            .await
            .unwrap();

        assert!(result.is_error);
        assert!(
            result.content.starts_with(PROMOTE_REFUSAL_PREFIX),
            "obtenu : {}",
            result.content
        );
        assert!(
            !result.content.contains("slot_busy"),
            "INVARIANT VIOLÉ (AC2) : le refus est rendu sous le motif du \
             créneau, dont la remédiation oriente vers le forçage — la garde a \
             donc tourné APRÈS l'accès base ; obtenu : {}",
            result.content
        );

        let task = harness
            .db
            .get_task(&wrapper)
            .await
            .expect("task readable")
            .expect("task exists");
        assert_eq!(task.status, "pending");
    }

    /// **V4 (AC4)** — une ligne d'audit sous le nouveau nom, et **zéro** sous
    /// `deferred_dispatch_force_promote_succeeded`.
    ///
    /// Les deux directions comptent. La première fait du refus une grandeur
    /// comptable (sonde S1) ; la seconde est ce qui interdit à un refus de
    /// laisser derrière lui la trace d'une réussite — c'est-à-dire de rendre la
    /// sonde du § 11 inutilisable en mélangeant les deux populations sous le
    /// même `GROUP BY`.
    #[tokio::test]
    async fn mika2654_le_refus_ecrit_sa_ligne_daudit() {
        let harness = TestHarness::new();
        let agent_id = &harness.db.agent_id;
        harness
            .db
            .create_task(deferred_callback_task(agent_id, "groom"))
            .await
            .unwrap();

        let ctx = webhook_pr_ctx(&harness);
        let session_id = ctx.session_id.to_string();
        let result = PromoteDeferredCallbackTool
            .execute(serde_json::json!({"dispatch_class": "groom"}), &ctx)
            .await
            .unwrap();
        assert!(result.is_error);

        let rows = harness.db.get_audit_events(&session_id).await.unwrap();

        let guard: Vec<_> = rows
            .iter()
            .filter(|r| r.tool_name == FORCE_PROMOTE_TURN_GUARD_AUDIT_TOOL)
            .collect();
        assert_eq!(
            guard.len(),
            1,
            "exactement une ligne de garde attendue, obtenu {} — la sonde S1 \
             compte des refus, donc un doublon la fausse autant qu'une absence",
            guard.len()
        );
        assert_eq!(guard[0].target_key, "dispatch_class:groom");
        assert_eq!(
            guard[0].after_value.as_deref(),
            Some(AUDIT_REFUSED_WEBHOOK_PR_TURN)
        );
        // `before_value` est NULL : il n'y a pas d'état antérieur à nommer —
        // rien n'a changé, c'est tout le propos du refus.
        assert_eq!(guard[0].before_value, None);

        assert!(
            !rows
                .iter()
                .any(|r| r.tool_name == "deferred_dispatch_force_promote_succeeded"),
            "INVARIANT VIOLÉ : un refus a laissé une trace de réussite — les deux \
             populations se mélangeraient sous le même GROUP BY"
        );
    }

    /// **V5 (AC5)** — le refus ne donne aucun gabarit de contournement.
    ///
    /// *Un refus qui donne le gabarit est une fuite avec une étape de plus*
    /// (doctrine mika#2520 / mika#2292). `MIKA_DEV_IDENTITY` porte `shell-exec`,
    /// donc nommer le geste CLI ou son drapeau de forçage reviendrait à le
    /// prescrire. La contrepartie est obligatoire : le corps doit nommer la
    /// sortie **correcte**, sans quoi ce test serait satisfait par un refus muet.
    #[tokio::test]
    async fn mika2654_le_refus_ne_nomme_aucun_contournement() {
        let body = refusal_body(ForcePromoteTurnDecision::RefusedWebhookPrTurn, "implement");
        let lower = body.to_lowercase();

        for forbidden in [
            "promote-deferred",
            "--override",
            "run_shell",
            "update_task_status",
        ] {
            assert!(
                !lower.contains(forbidden),
                "le corps du refus nomme « {forbidden} » — c'est le gabarit du \
                 contournement, pas un refus : {body}"
            );
        }

        assert!(
            lower.contains("operator"),
            "le refus doit nommer à qui la levée appartient : {body}"
        );
        assert!(
            lower.contains("next scan"),
            "le refus doit dire que le moteur promeut de lui-même, sans quoi il se \
             lit comme une perte définitive et invite au contournement : {body}"
        );
    }

    /// **V6** — la table de vérité du classifieur, complète : deux entrées,
    /// deux issues.
    ///
    /// Elle est petite, et c'est ce qui la rend lisible plutôt que faible : le
    /// prédicat n'a **qu'un** terme (la classe de tour), contrairement à la
    /// conjonction de la garde sœur. Ce qu'elle épingle est AC6 — le terme est
    /// un paramètre, donc « hors tour webhook PR, rien ne change » est une
    /// propriété de la fonction et non une lecture du site d'appel.
    ///
    /// **Ce qu'elle ne garantit pas, dit ici pour que personne ne le déduise.**
    /// Elle n'atteste pas le câblage : le booléen est posé à **un seul** site de
    /// production (`agent_loop/mod.rs`) et `false` aux vingt autres, donc ce
    /// test reste vert si le champ n'est jamais calculé ni jamais lu — très
    /// exactement le mode de panne de cette famille, et la raison d'être d'AC9
    /// (mutation M4).
    #[test]
    fn mika2654_la_table_de_verite_du_classifieur() {
        assert_eq!(
            classify_force_promote_turn(true),
            ForcePromoteTurnDecision::RefusedWebhookPrTurn,
            "tour ouvert par un événement PR / check-suite ⇒ refus : c'est la \
             décision de la forme 2 (§ 3), et ce que le refus retire est une \
             avance ≤ 60 s plus le contournement de la priorité opérateur"
        );
        assert_eq!(
            classify_force_promote_turn(false),
            ForcePromoteTurnDecision::Allowed,
            "INVARIANT VIOLÉ (AC3) : la garde a mordu hors de sa population — \
             les 40 appels mesurés hors canal github (callbacks de pilote, \
             deferred-dispatch) sont le chemin de reprise, et le casser serait \
             pire que le défaut qu'on referme"
        );

        // Hors population : rien n'est écrit (doctrine mika#2131). C'est la
        // moitié de l'assertion que la non-régression d'AC3 rend observable.
        assert!(
            classify_force_promote_turn(false).audit_value().is_none(),
            "hors population : aucune ligne, aucun audit"
        );
    }

    /// **V6, second volet** — les constantes de fil sont épinglées, dans les
    /// **deux** directions.
    ///
    /// Sans la seconde, une valeur pourrait être déclarée, groupée par un
    /// opérateur, et produite par rien — une colonne de zéros qui se lit comme
    /// une population saine.
    #[test]
    fn mika2654_les_constantes_de_fil_sont_epinglees() {
        // Les littéraux, épinglés : ce sont les chaînes que les sondes du § 11
        // portent en dur, donc les renommer est un changement de format de fil
        // et pas un refactor.
        assert_eq!(
            FORCE_PROMOTE_TURN_GUARD_AUDIT_TOOL,
            "deferred_dispatch_force_promote_refused_webhook_pr_turn"
        );
        assert_eq!(AUDIT_REFUSED_WEBHOOK_PR_TURN, "refused_webhook_pr_turn");
        assert_eq!(PROMOTE_REFUSAL_PREFIX, "REFUS (webhook-pr-turn, mika#2654)");

        // Produit ⊆ déclaré, et déclaré ⊆ produit.
        let produced: Vec<&'static str> = [
            classify_force_promote_turn(true),
            classify_force_promote_turn(false),
        ]
        .iter()
        .filter_map(|d| d.audit_value())
        .collect();
        for value in &produced {
            assert!(
                ALL_FORCE_PROMOTE_TURN_GUARD_VERDICTS.contains(value),
                "{value} est produit mais pas déclaré"
            );
        }
        for declared in ALL_FORCE_PROMOTE_TURN_GUARD_VERDICTS {
            assert!(
                produced.contains(declared),
                "{declared} est déclaré mais inatteignable — un opérateur \
                 grouperait sur une valeur que rien n'écrit"
            );
        }

        // Le nom d'audit est le QUATRIÈME de sa famille, et il en est distinct.
        // C'est ce qui dit pourquoi le needle du scan S1 est le nom ENTIER :
        // les trois frères partagent le préfixe, donc un scan sur le préfixe
        // serait rouge à la naissance.
        let family = "deferred_dispatch_force_promote";
        assert!(
            FORCE_PROMOTE_TURN_GUARD_AUDIT_TOOL.starts_with(family),
            "le nom quitte sa famille : la sonde qui groupe sur \
             `{family}_%` ne le verrait plus"
        );
        for sibling in [
            "deferred_dispatch_force_promote_succeeded",
            "deferred_dispatch_force_promote_rejected_slot_busy",
            "deferred_dispatch_force_promote_override",
        ] {
            assert_ne!(
                FORCE_PROMOTE_TURN_GUARD_AUDIT_TOOL, sibling,
                "collision avec un nom de mika#1453 : les deux populations se \
                 fondraient, et elles ne se soustraient pas (l'une répond « le \
                 créneau », l'autre « la classe de tour »)"
            );
        }
    }
}
