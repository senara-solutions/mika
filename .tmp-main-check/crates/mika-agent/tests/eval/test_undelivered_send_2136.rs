//! mika#2136 — un `send_message` échoué ne peut pas se clore en silence.
//!
//! Ces tests pilotent le **chemin de production** : `run_agent` /
//! `run_silent_agent` avec `MockLlmProvider` et un `MessageSender` scripté.
//!
//! Le ticket relève à juste titre que le harnais golden rejoue des réponses
//! scriptées et « ne peut donc pas attester une règle de comportement ». C'est
//! exact, et **c'est un argument POUR la structure** : ce qu'on atteste ici
//! n'est pas qu'un modèle dira la vérité, c'est qu'un moteur refuse de clore.
//! Un refus décidé par le moteur se teste précisément avec des réponses
//! scriptées, puisque ce n'est pas le modèle qui décide.

use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use anyhow::Result;
use async_trait::async_trait;
use mika_agent::agent::{SilentAgentParams, SilentTrigger, run_silent_agent};
use mika_agent::messaging::{MessageSender, SendOutcome};
use mika_common::llm::mock::*;
use serde_json::json;

use super::harness::EvalHarness;

// ---------------------------------------------------------------------------
// Un `MessageSender` scripté
// ---------------------------------------------------------------------------

/// Sender whose outcome is decided per call by a scripted queue.
///
/// This is the injection point E7 identified: `EvalHarnessBuilder::message_sender`
/// takes an `Arc<dyn MessageSender>`, so a transport that refuses the first
/// fragment and accepts the rest is drivable end to end through `run_agent`.
struct ScriptedSender {
    /// One entry per expected call, consumed front to back. Once exhausted,
    /// every further call is `Delivered` — so a test states only the failures
    /// it cares about.
    script: Mutex<std::collections::VecDeque<SendOutcome>>,
    sent: Mutex<Vec<String>>,
}

impl ScriptedSender {
    fn new(script: Vec<SendOutcome>) -> Arc<Self> {
        Arc::new(Self {
            script: Mutex::new(script.into()),
            sent: Mutex::new(Vec::new()),
        })
    }

    /// A sender that delivers everything — the negative control's transport.
    fn always_delivers() -> Arc<Self> {
        Self::new(vec![])
    }

    fn sent(&self) -> Vec<String> {
        self.sent.lock().unwrap().clone()
    }
}

#[async_trait]
impl MessageSender for ScriptedSender {
    async fn send(&self, text: &str) -> Result<SendOutcome> {
        self.sent.lock().unwrap().push(text.to_string());
        // Release the guard before returning so it is never held across an
        // await point (clippy::sig_drop, #1724).
        let next = self.script.lock().unwrap().pop_front();
        Ok(next.unwrap_or(SendOutcome::Delivered))
    }
}

fn transport_failure() -> SendOutcome {
    SendOutcome::Failed {
        reason: "gateway /send returned 502 Bad Gateway".to_string(),
    }
}

/// The user-facing corrections the guard pushes, recognizable in a captured
/// request without pinning the whole sentence.
const CORRECTION_MARKER: &str = "[mika-engine]";

fn corrections_seen(trace: &super::trace::AgentTrace, needle: &str) -> usize {
    use mika_common::llm::{LlmContent, LlmRole};
    trace
        .captured_requests
        .iter()
        .filter(|r| {
            r.messages.iter().any(|m| {
                matches!(m.role, LlmRole::User)
                    && matches!(&m.content, LlmContent::Text(t) if t.contains(needle))
            })
        })
        .count()
}

// ---------------------------------------------------------------------------
// AC6-a — rejeu du 2026-09-01, étage 1 : « Le voici en entier 👆 »
// ---------------------------------------------------------------------------

/// Le document de plus de 12 000 caractères est refusé par la garde de
/// longueur, et le modèle répond quand même qu'il l'a livré.
///
/// **Le scénario ne contient aucun autre `send_message`, et c'est fidèle au cas
/// mesuré, pas une commodité** : Al n'a rien reçu du tout. Un scénario qui
/// glisserait un message d'excuse délivré entre le refus et la clôture passerait
/// sous le faux négatif nommé en D3 (« extinction par un envoi sans rapport ») —
/// il deviendrait vert sans rien prouver. Ne pas le « réparer » ainsi.
#[tokio::test]
async fn mika2136_ac6a_rejeu_le_voici_en_entier() {
    let sender = ScriptedSender::always_delivers();
    let document = "D".repeat(12_000);

    let harness = EvalHarness::builder()
        .responses(vec![
            // Pas 1 : l'envoi du document entier. Refusé par la garde (mika#2134).
            tool_call_response("send_message", json!({ "text": document })),
            // Pas 2 : la phrase du 2026-09-01, mot pour mot.
            text_response("Le voici en entier 👆"),
            // Pas 3 : ce que le modèle répond après le re-prompt du moteur.
            // Toujours sans rien renvoyer — donc l'échec reste non réparé.
            text_response("Le voici en entier 👆"),
        ])
        .message_sender(sender.clone())
        .build()
        .await
        .unwrap();

    let trace = harness.run("montre-moi le document").await.unwrap();

    assert!(
        sender.sent().is_empty(),
        "le document refusé ne doit jamais atteindre le transport"
    );

    // Le tour a été refusé une fois : le moteur a re-prompté.
    assert_eq!(
        corrections_seen(&trace, CORRECTION_MARKER),
        1,
        "le guard 6f doit refuser la clôture exactement une fois"
    );
    assert_eq!(
        trace.llm_call_count, 3,
        "1 appel pour l'outil, 1 pour la clôture refusée, 1 pour la re-clôture"
    );

    // Sur la seconde clôture non réparée, le budget est épuisé et le moteur
    // porte le constat au-delà du tour.
    let undelivered = trace
        .output
        .undelivered_sends
        .as_ref()
        .expect("le constat doit survivre au tour");
    assert_eq!(
        undelivered.stage,
        mika_agent::evidence::guards::UndeliveredStage::RefusedTooLong
    );
    assert_eq!(undelivered.failed_count, 1);
    assert_eq!(undelivered.failed_index, 1);
}

// ---------------------------------------------------------------------------
// AC6-b — rejeu du 2026-09-01, étage 2 : « Partie 2/4 » sans un mot
// ---------------------------------------------------------------------------

/// La partie 1/4 meurt au transport, la partie 2/4 passe, et le modèle enchaîne
/// sans rien dire.
///
/// **Le scénario s'arrête à deux envois, et c'est la forme exacte du cas mesuré,
/// pas une simplification.** Le boundary #771 s'arme sur le *succès* de la
/// partie 2 et supprimerait les parties 3 et 4 : un test écrit avec quatre
/// envois échouerait sur une suppression parfaitement légitime et ferait
/// chercher le défaut au mauvais endroit. C'est d'ailleurs ce boundary qui a
/// rendu la séquence d'Al possible — la partie 1/4 ayant échoué, il ne s'est pas
/// armé, et la partie 2/4 est passée.
#[tokio::test]
async fn mika2136_ac6b_rejeu_partie_1_morte_partie_2_passe() {
    let sender = ScriptedSender::new(vec![transport_failure()]);

    let harness = EvalHarness::builder()
        .responses(vec![
            multi_tool_response(vec![
                ("send_message", json!({ "text": "Partie 1/4 : le début." })),
                ("send_message", json!({ "text": "Partie 2/4 : la suite." })),
            ]),
            text_response("Voilà la Partie 2/4 👆"),
            text_response("Voilà la Partie 2/4 👆"),
        ])
        .message_sender(sender.clone())
        .build()
        .await
        .unwrap();

    let trace = harness.run("découpe-le").await.unwrap();

    assert_eq!(
        sender.sent().len(),
        2,
        "les deux fragments atteignent le transport; le premier y meurt"
    );

    assert_eq!(
        corrections_seen(&trace, CORRECTION_MARKER),
        1,
        "le guard 6f doit refuser la clôture exactement une fois"
    );

    let undelivered = trace
        .output
        .undelivered_sends
        .as_ref()
        .expect("le fragment mort doit survivre au tour");
    assert_eq!(
        undelivered.stage,
        mika_agent::evidence::guards::UndeliveredStage::Failed
    );
    assert_eq!(
        undelivered.failed_index, 1,
        "c'est la partie 1 qui est morte, pas la 2"
    );
    assert_eq!(undelivered.failed_count, 1);
    assert!(
        undelivered.preview.starts_with("Partie 1/4"),
        "le constat nomme le fragment perdu : {}",
        undelivered.preview
    );
}

// ---------------------------------------------------------------------------
// AC4-bis — contrôle négatif du chemin nominal en conversation
// ---------------------------------------------------------------------------

/// Un envoi unique livré : aucun constat, aucun re-prompt, et **le nombre
/// d'appels LLM est exactement celui d'avant mika#2136**.
///
/// Sans lui, AC4 n'attesterait la neutralité que dans un mode où l'annexe de D5
/// ne s'applique pas — et le chemin d'Al *réussi* resterait non couvert.
///
/// **Ce que ce test a corrigé dans sa propre écriture, et qui vaut d'être lu**
/// (E10) : en conversation, le boundary #771 s'arme sur le **succès** du premier
/// `send_message` et clôt le tour *sans second appel LLM*. Un contrôle négatif
/// écrit avec `llm_call_count == 2` et une assertion sur le texte final scripté
/// échoue — non pas sur le correctif, mais sur une suppression parfaitement
/// légitime et antérieure à lui. C'est la même méprise que D9 range hors de
/// portée pour AC3/AC4, et c'est pourquoi ceux-là tournent en mode silencieux.
#[tokio::test]
async fn mika2136_ac4bis_controle_negatif_conversation() {
    let sender = ScriptedSender::always_delivers();

    let harness = EvalHarness::builder()
        .responses(vec![
            tool_call_response("send_message", json!({ "text": "Voici le résumé." })),
            // Jamais atteinte : le boundary #771 force la clôture.
            text_response("C'est envoyé."),
        ])
        .message_sender(sender.clone())
        .build()
        .await
        .unwrap();

    let trace = harness.run("envoie-moi le résumé").await.unwrap();

    assert_eq!(sender.sent(), vec!["Voici le résumé.".to_string()]);
    assert!(
        trace.output.undelivered_sends.is_none(),
        "le chemin heureux ne produit aucun constat"
    );
    assert_eq!(
        corrections_seen(&trace, CORRECTION_MARKER),
        0,
        "aucun re-prompt sur le chemin heureux"
    );
    assert_eq!(
        trace.llm_call_count, 1,
        "un seul appel : le boundary #771 clôt le tour sur le succès de l'envoi, \
         et mika#2136 n'en ajoute aucun"
    );
}

// ---------------------------------------------------------------------------
// Pilotage du tour silencieux
// ---------------------------------------------------------------------------

/// D9 : les scénarios multi-fragments tournent en mode **silencieux**, parce que
/// le boundary #771 est conditionné à `conversation_mode` et qu'en conversation
/// « quatre fragments tous livrés » n'est pas un scénario difficile, c'est un
/// scénario **impossible** — le premier succès fait supprimer les suivants.
/// `SilentTrigger::Reminder` est choisi plutôt que `Callback` parce que le
/// message y est utilisateur-authored et n'arme pas le guard #870, qui exigerait
/// `update_task_status` et brouillerait l'assertion.
async fn run_reminder(harness: &EvalHarness, sender: Arc<ScriptedSender>, message: &str) {
    let skills_dirty = AtomicBool::new(false);
    let params = SilentAgentParams {
        tier: harness.tier,
        deployment: mika_common::home::Deployment::Unknown,
        db: &harness.db,
        llm: harness.llm.as_ref(),
        tools: &harness.tools,
        skills: &harness.skills,
        trigger: SilentTrigger::Reminder {
            task_id: "task-2136".to_string(),
            message: message.to_string(),
        },
        home_dir: harness.home_dir.path(),
        session_id: &harness.session_id,
        message_sender: Some(sender),
        embedding_client: None,
        brave_api_key: None,
        github_token: None,
        gateway_url: None,
        internal_token: None,
        github_app: None,
        skills_dirty: &skills_dirty,
        settings: Some(&harness.settings),
        trace_id: Some(harness.trace_id.clone()),
        pr_reviews_posted: None,
    };
    run_silent_agent(&params).await.expect("silent turn runs");
}

/// Combien de requêtes capturées portent `needle` dans un message `User`.
fn silent_corrections(harness: &EvalHarness, needle: &str) -> usize {
    use mika_common::llm::{LlmContent, LlmRole};
    harness
        .mock()
        .captured_requests()
        .iter()
        .filter(|r| {
            r.messages.iter().any(|m| {
                matches!(m.role, LlmRole::User)
                    && matches!(&m.content, LlmContent::Text(t) if t.contains(needle))
            })
        })
        .count()
}

// ---------------------------------------------------------------------------
// AC3 — l'échec d'un fragment n'est pas masqué par la réussite des suivants
// ---------------------------------------------------------------------------

/// Quatre fragments, le premier meurt, les trois suivants passent. L'assertion
/// porte sur la **séquence**, pas sur un envoi isolé : c'est précisément la
/// réussite des suivants qui, sans le prédicat, faisait disparaître le premier.
#[tokio::test]
async fn mika2136_ac3_un_fragment_mort_nest_pas_masque_par_les_suivants() {
    let sender = ScriptedSender::new(vec![transport_failure()]);

    let harness = EvalHarness::builder()
        .responses(vec![
            multi_tool_response(vec![
                ("send_message", json!({ "text": "Partie 1/4" })),
                ("send_message", json!({ "text": "Partie 2/4" })),
                ("send_message", json!({ "text": "Partie 3/4" })),
                ("send_message", json!({ "text": "Partie 4/4" })),
            ]),
            text_response("Tout est envoyé."),
            text_response("Tout est envoyé."),
        ])
        .message_sender(sender.clone())
        .build()
        .await
        .unwrap();

    run_reminder(
        &harness,
        sender.clone(),
        "envoie le document en quatre parties",
    )
    .await;

    assert_eq!(
        sender.sent().len(),
        4,
        "le mode silencieux est exempt du boundary #771 : les quatre partent"
    );
    assert_eq!(
        silent_corrections(&harness, CORRECTION_MARKER),
        1,
        "le guard 6f refuse la clôture malgré trois réussites après l'échec"
    );
}

// ---------------------------------------------------------------------------
// AC4 — contrôle négatif : le chemin heureux ne paie rien
// ---------------------------------------------------------------------------

/// Quatre fragments tous livrés : aucun re-prompt, aucun tour supplémentaire,
/// et zéro occurrence du vocabulaire d'échec dans la conversation.
///
/// AC4 porte sur le **comportement observable** et exige de le montrer : un
/// correctif qui alourdirait le chemin heureux n'aurait pas réparé, il aurait
/// taxé.
#[tokio::test]
async fn mika2136_ac4_controle_negatif_quatre_fragments_livres() {
    let sender = ScriptedSender::always_delivers();

    let harness = EvalHarness::builder()
        .responses(vec![
            multi_tool_response(vec![
                ("send_message", json!({ "text": "Partie 1/4" })),
                ("send_message", json!({ "text": "Partie 2/4" })),
                ("send_message", json!({ "text": "Partie 3/4" })),
                ("send_message", json!({ "text": "Partie 4/4" })),
            ]),
            text_response("Tout est envoyé."),
        ])
        .message_sender(sender.clone())
        .build()
        .await
        .unwrap();

    run_reminder(
        &harness,
        sender.clone(),
        "envoie le document en quatre parties",
    )
    .await;

    assert_eq!(sender.sent().len(), 4);
    assert_eq!(
        silent_corrections(&harness, CORRECTION_MARKER),
        0,
        "aucun re-prompt sur une séquence entièrement livrée"
    );
    assert_eq!(
        harness.mock().captured_requests().len(),
        2,
        "1 appel pour les outils, 1 pour la clôture — aucun tour supplémentaire"
    );
}
