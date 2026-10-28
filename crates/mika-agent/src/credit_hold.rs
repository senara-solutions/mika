//! Une ligne de callback retenue parce que le crédit du fournisseur est épuisé
//! (mika#2944).
//!
//! # Le défaut que ce module ferme
//!
//! Mesuré le 2026-10-28 sur la tâche 26b8d5c7 (build de mika#2920) : la reprise
//! du callback échoue sur un HTTP 402 d'OpenRouter (« This request would exceed
//! your available credits »). Le dispatcher range l'échec en `http_402` et le
//! réessaie comme une panne passagère — 60 s, puis 120 s, puis la quarantaine au
//! plafond horaire. **Chaque tentative rejoue la revue depuis le début**, donc
//! consomme le crédit qui reste, puis retombe sur le même 402 : quatre
//! tentatives, 60 tours de revue, aucune ne pouvait aboutir, solde à 0,16 $ à
//! 08:07Z. Le verdict n'est parti qu'à 09:05:44Z, après une recharge à la main
//! vers 08:51Z (`callback_delivery_slow`, `wait_secs=7743`).
//!
//! Un backoff borne la **fréquence** d'un réessai. Il ne peut rien contre un
//! réessai dont on sait qu'il ne peut pas aboutir : le remède doit porter sur la
//! condition, pas sur la cadence.
//!
//! # Ce que « retenu » veut dire, et ce qu'il ne veut pas dire
//!
//! Retenu = *la prochaine reprise ne rejoue pas la revue ; elle envoie d'abord
//! une sonde d'un message.* Ce n'est **pas** une suspension : la ligne garde son
//! `status='completed'`, son résultat de build intact, et son `next_fire_at`
//! continue d'être posé — c'est lui qui ramène la sonde (AC2). Rien n'est perdu,
//! rien n'attend un geste humain.
//!
//! # La représentation : deux horodatages, pas un drapeau
//!
//! `crates/mika-agent/src/async_db.rs` n'expose que `set_task_metadata_field` et
//! `get_task_metadata_field` — **aucun helper de suppression**. Un drapeau
//! booléen ne pourrait donc jamais être retiré : il faudrait écrire `"false"` et
//! tout lecteur deviendrait responsable de distinguer « absent » de « posé puis
//! levé ». D'où deux marques monotones :
//!
//! * [`DELIVERY_CREDIT_HELD_AT_KEY`] — posée à chaque entrée en retenue ;
//! * [`DELIVERY_CREDIT_RELEASED_AT_KEY`] — posée au retour du crédit.
//!
//! **Retenu ⟺ `held_at > released_at`.** La comparaison est lexicographique et
//! c'est de la chronologie : `crate::timestamp` écrit un ISO 8601 UTC de largeur
//! fixe, donc l'ordre des chaînes *est* l'ordre du temps. Un `released_at`
//! absent avec un `held_at` présent est une retenue (première retenue, jamais
//! relâchée).
//!
//! [`is_held`] est le **seul** lecteur de cette représentation : le dispatcher
//! comme `qa_build_callback` passent par lui. Deux lectures écrites à la main
//! classeraient un jour sur une marque et rapporteraient l'autre.

use mika_common::llm::{LlmContent, LlmMessage, LlmProvider, LlmRequest, LlmRole};

/// Instant de la dernière entrée en retenue (clé de `tasks.metadata`).
pub const DELIVERY_CREDIT_HELD_AT_KEY: &str = "delivery_credit_held_at";

/// Instant du dernier retour de crédit constaté (clé de `tasks.metadata`).
pub const DELIVERY_CREDIT_RELEASED_AT_KEY: &str = "delivery_credit_released_at";

/// Nombre de sondes envoyées depuis la première retenue (clé de
/// `tasks.metadata`).
///
/// C'est le compteur qui rend le résidu lisible : `delivery_attempts` continue
/// de monter pendant une retenue longue, donc la ligne finit par franchir le
/// seuil de quarantaine de mika#2179. Ce franchissement est délibéré — il dit
/// « ça dure » — et `delivery_credit_probes` dit *combien de sondes* ont été
/// payées pour en arriver là, c'est-à-dire la facture réelle de la retenue.
pub const DELIVERY_CREDIT_PROBES_KEY: &str = "delivery_credit_probes";

/// Les trois noms d'événement de la retenue (mika#2944), définis ici parce que
/// le dispatcher les émet et les tests les lisent : deux orthographes d'un même
/// nom scindent une population sans le dire (mika#2205).
pub const CREDIT_HELD_EVENT: &str = "callback_delivery_credit_held";
/// Le crédit est revenu, la revue reprend dans le tour qui suit (AC2).
pub const CREDIT_RESTORED_EVENT: &str = "callback_delivery_credit_restored";
/// La sonde n'a pas conclu — retenue maintenue, fail-closed assumé.
pub const CREDIT_PROBE_INCONCLUSIVE_EVENT: &str = "callback_delivery_credit_probe_inconclusive";

/// La ligne est-elle en retenue pour crédit épuisé ? (mika#2944)
///
/// Seul lecteur de la représentation à deux marques décrite en tête de module.
///
/// # Fail-open, et c'est le bon sens de l'erreur
///
/// Un `metadata` absent, vide ou illisible rend `false` : la ligne est libre.
/// L'inverse retiendrait un callback sur un champ qui ne dit rien — un verdict
/// retardé d'un quart d'heure par une donnée illisible, ce qui est exactement la
/// panne que ce ticket ferme, reproduite par son remède.
#[must_use]
pub fn is_held(metadata: Option<&str>) -> bool {
    let parsed: serde_json::Value = metadata
        .filter(|m| !m.trim().is_empty())
        .and_then(|m| serde_json::from_str(m).ok())
        .unwrap_or(serde_json::Value::Null);

    let stamp = |key: &str| -> Option<String> {
        parsed
            .get(key)
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    };

    match (
        stamp(DELIVERY_CREDIT_HELD_AT_KEY),
        stamp(DELIVERY_CREDIT_RELEASED_AT_KEY),
    ) {
        (Some(held), Some(released)) => held > released,
        (Some(_), None) => true,
        (None, _) => false,
    }
}

/// La requête de sonde : un message, et le `max_tokens` du fournisseur
/// (mika#2944).
///
/// # Pourquoi `max_tokens` du fournisseur et non un petit littéral
///
/// Le contrôle de crédit d'OpenRouter est un **pré-vol sur le coût maximum de
/// la requête**, pas sur ce qu'elle consommera. Une sonde à `max_tokens: 16`
/// passerait donc sur un solde qui refuserait encore le tour de revue qui suit,
/// et la retenue serait levée pour rien — une revue complète rejouée, c'est-à-dire
/// le défaut d'origine, déclenché par le remède. La sonde doit demander la même
/// enveloppe que le tour qu'elle autorise.
///
/// Ce qui coûte : un aller-retour d'un message par intervalle de retenue, dont
/// la facture réelle est la poignée de tokens qu'il produit, pas l'enveloppe.
///
/// Aucun outil, aucun système : la sonde n'est pas un tour, elle ne doit rien
/// pouvoir décider.
#[must_use]
pub fn probe_request(llm: &dyn LlmProvider) -> LlmRequest {
    LlmRequest {
        model: llm.model_name().to_string(),
        system: None,
        messages: vec![LlmMessage {
            role: LlmRole::User,
            content: LlmContent::Text("ok".into()),
        }],
        tools: None,
        max_tokens: llm.max_tokens(),
        thinking: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta(pairs: &[(&str, &str)]) -> String {
        let map: serde_json::Map<String, serde_json::Value> = pairs
            .iter()
            .map(|(k, v)| {
                (
                    (*k).to_string(),
                    serde_json::Value::String((*v).to_string()),
                )
            })
            .collect();
        serde_json::Value::Object(map).to_string()
    }

    #[test]
    fn mika2944_held_at_seul_est_une_retenue() {
        let m = meta(&[(DELIVERY_CREDIT_HELD_AT_KEY, "2026-10-28T06:58:21Z")]);
        assert!(is_held(Some(&m)));
    }

    /// Le cœur de la représentation : l'ordre lexicographique des deux marques
    /// *est* l'ordre du temps, parce que `crate::timestamp` écrit un ISO 8601
    /// UTC de largeur fixe.
    #[test]
    fn mika2944_la_marque_la_plus_recente_decide() {
        let relachee = meta(&[
            (DELIVERY_CREDIT_HELD_AT_KEY, "2026-10-28T06:58:21Z"),
            (DELIVERY_CREDIT_RELEASED_AT_KEY, "2026-10-28T08:51:00Z"),
        ]);
        assert!(!is_held(Some(&relachee)), "relâchée après la retenue");

        let reprise = meta(&[
            (DELIVERY_CREDIT_HELD_AT_KEY, "2026-10-28T09:10:00Z"),
            (DELIVERY_CREDIT_RELEASED_AT_KEY, "2026-10-28T08:51:00Z"),
        ]);
        assert!(
            is_held(Some(&reprise)),
            "re-retenue après un relâchement : la marque la plus récente gagne"
        );

        let egales = meta(&[
            (DELIVERY_CREDIT_HELD_AT_KEY, "2026-10-28T08:51:00Z"),
            (DELIVERY_CREDIT_RELEASED_AT_KEY, "2026-10-28T08:51:00Z"),
        ]);
        assert!(
            !is_held(Some(&egales)),
            "à égalité, la ligne est libre : une retenue doit être postérieure pour mordre"
        );
    }

    /// Fail-open — l'absence, le vide et l'illisible rendent la ligne libre.
    /// Un remède qui retient sur une donnée muette rejoue la panne d'origine.
    #[test]
    fn mika2944_une_metadata_muette_laisse_la_ligne_libre() {
        assert!(!is_held(None));
        assert!(!is_held(Some("")));
        assert!(!is_held(Some("   ")));
        assert!(!is_held(Some("{pas du json")));
        assert!(!is_held(Some("{}")));
        assert!(
            !is_held(Some(&meta(&[(DELIVERY_CREDIT_HELD_AT_KEY, "")]))),
            "une chaîne vide ne retient pas : `json_set` peut en écrire une"
        );
        assert!(
            !is_held(Some(&meta(&[("delivery_attempts", "3")]))),
            "une ligne de mika#2179 sans marque de crédit est libre"
        );
    }

    /// La sonde demande la même enveloppe que le tour qu'elle autorise — sans
    /// quoi elle passerait sur un solde qui refuserait encore la revue.
    #[test]
    fn mika2944_la_sonde_porte_lenveloppe_du_fournisseur() {
        let llm = mika_common::llm::mock::MockLlmProvider::builder()
            .model_name("z-ai/glm-5.3")
            .max_tokens(8192)
            .build();
        let req = probe_request(&llm);

        assert_eq!(req.model, "z-ai/glm-5.3");
        assert_eq!(
            req.max_tokens, 8192,
            "le pré-vol de crédit porte sur le coût MAXIMUM : un petit littéral \
             lèverait la retenue sur un solde qui refuse encore la revue"
        );
        assert_eq!(req.messages.len(), 1, "un message, pas un tour");
        assert!(req.tools.is_none(), "la sonde ne doit rien pouvoir décider");
        assert!(req.system.is_none());
        assert!(req.thinking.is_none());
    }
}
