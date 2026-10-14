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
/// naissance** — et un lint rouge à la naissance se fait désarmer. Dans l'autre
/// sens, le résidu `promote_deferred_guard_audit_failed` a ce nom pour
/// **préfixe**, donc une comparaison en sous-chaîne compterait une ligne de
/// journal comme un second écrivain de la ligne d'audit. C'est la correction que
/// mika#2649, puis mika#2634, puis mika#2653 ont dû écrire chacun sur son scan
/// jumeau ; `audit_name_literal_present` compare `lit.trim() == needle`, et
/// c'est la bonne forme.
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
                to cancel the blocker and force promotion."
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
             les 31 appels mesurés hors canal github (callbacks de pilote, \
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
