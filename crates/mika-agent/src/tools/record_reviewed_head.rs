//! Outil `record_reviewed_head` (mika#2917, vol 2).
//!
//! L'écrivain **unique** du registre des têtes revues. Le skill
//! `qa-review-webhook-success` l'appelle à la fin d'un tour qui a conclu sur une
//! tête : il inscrit `(repo, PR, head_sha, issue)` dans `audit_events` sous
//! [`QA_REVIEWED_HEAD_TOOL`], là où le pré-filtre du vol 3 pourra le lire sans
//! `gh`.
//!
//! Ce que cet outil ne fait pas, et ce sont des refus, pas des manques :
//!
//! - **Aucun `gh`, aucun réseau, aucun sous-processus.** Il ne vérifie pas que la
//!   PR existe, ni que le `head_sha` est bien sa tête : le tour qui l'appelle
//!   vient de le lire dans le payload du webhook, et aller le redemander à la
//!   forge serait payer deux fois une donnée déjà en main (mika#2260).
//! - **Il ne décide rien.** L'issue du tour est décidée par le skill et passée en
//!   paramètre ; l'outil la valide contre le vocabulaire fermé et l'écrit telle
//!   que [`reviewed_head_outcome`] la canonise. Dupliquer la décision dans le
//!   Rust ferait deux sites de vérité, ce que le bearing de mika#2680 refuse.
//! - **Il ne passe jamais d'`agent_id`.** `AsyncDatabase::log_audit_event`
//!   l'injecte lui-même depuis la connexion : le cloisonnement par agent est une
//!   propriété de construction, pas une discipline d'appelant.
//!
//! Un refus de validation n'écrit **rien** et nomme le champ fautif dans son
//! `hint`. Quatre champs, quatre refus : un `repo` vide, un `pr` absent ou nul,
//! un `head_sha` vide, une `outcome` hors du vocabulaire. Le modèle doit pouvoir
//! se corriger au tour suivant sans deviner lequel des quatre l'a arrêté.

use anyhow::Result;
use async_trait::async_trait;
use mika_common::claude::ToolDefinition;
use serde_json::Value;

use super::{Tool, ToolContext, ToolOutput};
use crate::qa_reviewed_head::{
    QA_REVIEWED_HEAD_TOOL, REVIEWED_HEAD_OUTCOMES, head_key, reviewed_head_outcome,
};

pub struct RecordReviewedHeadTool;

/// Refus de validation : aucune écriture, et le `hint` nomme le champ fautif.
fn field_error(field: &str, hint: &str) -> ToolOutput {
    ToolOutput::error(
        serde_json::json!({
            "error": "invalid_field",
            "field": field,
            "hint": hint,
        })
        .to_string(),
    )
}

#[async_trait]
impl Tool for RecordReviewedHeadTool {
    fn name(&self) -> &str {
        "record_reviewed_head"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "record_reviewed_head".to_string(),
            description: "Enregistre qu'un tour de revue a conclu sur une tête de PR : \
                 (repo, pr, head_sha, outcome). À appeler une fois, à la fin du tour, \
                 avec la MÊME valeur que la ligne `Webhook-Outcome:` de ce tour. \
                 Trois issues seulement sont enregistrables : review, \
                 stop_already_reviewed_sha, stop_verdict_recorded. \
                 N'appelle pas la forge et ne vérifie pas que la PR existe."
                .to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "repo": {
                        "type": "string",
                        "description": "owner/name, exactement comme le webhook le nomme"
                    },
                    "pr": {
                        "type": "integer",
                        "description": "le numéro de la PR que le check_suite nomme"
                    },
                    "head_sha": {
                        "type": "string",
                        "description": "la tête sur laquelle ce tour a conclu"
                    },
                    "outcome": {
                        "type": "string",
                        "enum": ["review", "stop_already_reviewed_sha", "stop_verdict_recorded"],
                        "description": "l'issue du tour, identique à sa ligne `Webhook-Outcome:`"
                    }
                },
                "required": ["repo", "pr", "head_sha", "outcome"]
            }),
        }
    }

    async fn execute(&self, input: Value, ctx: &ToolContext<'_>) -> Result<ToolOutput> {
        let repo = input["repo"].as_str().unwrap_or("").trim();
        if repo.is_empty() {
            return Ok(field_error(
                "repo",
                "'repo' est requis : owner/name, exactement comme le webhook le nomme.",
            ));
        }

        let pr = match input.get("pr").and_then(|v| v.as_u64()) {
            Some(n) if n > 0 => n,
            _ => {
                return Ok(field_error(
                    "pr",
                    "'pr' est requis et doit être le numéro de PR (entier strictement positif) que le check_suite nomme.",
                ));
            }
        };

        let head_sha = input["head_sha"].as_str().unwrap_or("").trim();
        if head_sha.is_empty() {
            return Ok(field_error(
                "head_sha",
                "'head_sha' est requis : la tête sur laquelle ce tour a conclu.",
            ));
        }

        let raw_outcome = input["outcome"].as_str().unwrap_or("");
        let Some(outcome) = reviewed_head_outcome(raw_outcome) else {
            return Ok(field_error(
                "outcome",
                &format!(
                    "'outcome' doit être l'une de : {}. Les issues qui ne concluent sur aucune tête ne s'enregistrent pas.",
                    REVIEWED_HEAD_OUTCOMES.join(", ")
                ),
            ));
        };

        let key = head_key(repo, pr, head_sha);
        let reasoning = format!("qa-review-webhook-success a conclu en {outcome} sur cette tête");

        ctx.db
            .log_audit_event(
                ctx.session_id,
                QA_REVIEWED_HEAD_TOOL,
                &key,
                None,
                Some(outcome),
                Some(&reasoning),
                Some(ctx.trace_id),
            )
            .await?;

        Ok(ToolOutput::success(
            serde_json::json!({
                "recorded": true,
                "key": key,
                "outcome": outcome,
            })
            .to_string(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::test_helpers::TestHarness;

    /// Le vocabulaire des issues est celui du registre, pas une copie locale :
    /// l'`enum` du schéma doit nommer exactement les trois littéraux canoniques.
    #[test]
    fn mika2917_le_schema_nomme_exactement_les_trois_issues() {
        let def = RecordReviewedHeadTool.definition();
        let declared = def.input_schema["properties"]["outcome"]["enum"]
            .as_array()
            .expect("l'enum des issues est déclarée")
            .iter()
            .map(|v| v.as_str().expect("un littéral").to_string())
            .collect::<Vec<_>>();
        assert_eq!(declared, REVIEWED_HEAD_OUTCOMES.to_vec());

        let required = def.input_schema["required"]
            .as_array()
            .expect("les champs requis sont déclarés")
            .iter()
            .map(|v| v.as_str().expect("un littéral"))
            .collect::<Vec<_>>();
        assert_eq!(required, vec!["repo", "pr", "head_sha", "outcome"]);
    }

    /// Les quatre refus de validation, et la propriété qui les rend utiles :
    /// aucun n'écrit, et chacun nomme son champ.
    #[tokio::test]
    async fn mika2917_les_quatre_refus_nomment_leur_champ_et_necrivent_rien() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();

        let cases: Vec<(&str, Value)> = vec![
            (
                "repo",
                serde_json::json!({"repo": "  ", "pr": 2917, "head_sha": "deadbeef", "outcome": "review"}),
            ),
            (
                "pr",
                serde_json::json!({"repo": "senara-solutions/mika", "pr": 0, "head_sha": "deadbeef", "outcome": "review"}),
            ),
            (
                "pr",
                serde_json::json!({"repo": "senara-solutions/mika", "head_sha": "deadbeef", "outcome": "review"}),
            ),
            (
                "head_sha",
                serde_json::json!({"repo": "senara-solutions/mika", "pr": 2917, "head_sha": "", "outcome": "review"}),
            ),
            (
                "outcome",
                serde_json::json!({"repo": "senara-solutions/mika", "pr": 2917, "head_sha": "deadbeef", "outcome": "stop_no_open_pr"}),
            ),
        ];

        for (field, input) in cases {
            let out = RecordReviewedHeadTool
                .execute(input, &ctx)
                .await
                .expect("un refus de validation n'est pas une erreur de transport");
            assert!(out.is_error, "le champ {field} doit être refusé");
            assert!(
                out.content.contains(field),
                "le hint doit nommer {field} ; reçu : {}",
                out.content
            );
        }

        let rows = ctx
            .db
            .get_audit_events(ctx.session_id)
            .await
            .expect("lecture du registre");
        assert!(
            rows.iter().all(|r| r.tool_name != QA_REVIEWED_HEAD_TOOL),
            "un refus de validation n'écrit rien"
        );
    }

    /// Le chemin nominal : une ligne sous le `tool_name` du registre, à la clé
    /// que `head_key` dicte, portant l'issue **canonique**.
    #[tokio::test]
    async fn mika2917_un_tour_conclu_ecrit_une_ligne_a_la_cle_du_registre() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();

        let out = RecordReviewedHeadTool
            .execute(
                serde_json::json!({
                    "repo": "senara-solutions/mika",
                    "pr": 2917,
                    "head_sha": "deadbeefcafe",
                    // Graphie libre de l'appelant : c'est le littéral canonique
                    // qui doit atterrir dans `after_value`.
                    "outcome": "  Stop_Verdict_Recorded  ",
                }),
                &ctx,
            )
            .await
            .expect("écriture du registre");
        assert!(!out.is_error, "reçu : {}", out.content);

        let expected_key = head_key("senara-solutions/mika", 2917, "deadbeefcafe");
        assert!(out.content.contains(&expected_key));

        let rows = ctx
            .db
            .get_audit_events(ctx.session_id)
            .await
            .expect("lecture du registre");
        let written: Vec<_> = rows
            .iter()
            .filter(|r| r.tool_name == QA_REVIEWED_HEAD_TOOL)
            .collect();
        assert_eq!(written.len(), 1, "une ligne, exactement");
        assert_eq!(written[0].target_key, expected_key);
        assert_eq!(
            written[0].after_value.as_deref(),
            Some("stop_verdict_recorded"),
            "l'issue écrite est le littéral canonique, jamais la graphie de l'appelant"
        );
    }
}
