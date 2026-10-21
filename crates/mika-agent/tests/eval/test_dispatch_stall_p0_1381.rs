//! mika#1381 (vol 3/3) — le site P0 amiral part bien en P0, bout en bout.
//!
//! Ferme l'AC9 différée du vol 1. Les vols 1 et 2 ont attesté la **frontière**
//! (`send_notification` nomme un étage, l'aide le transmet) ; aucun test ne
//! traversait `run_agent` jusqu'à un vrai site de production pour observer
//! l'étage **tel que le transport le reçoit**.
//!
//! Ce que l'angle mort vaut : `MessageSender::send_with_severity` a un **corps
//! par défaut** qui délègue à `self.send(text)` (`src/messaging.rs`). Un site
//! qui retomberait sur `send()` — par régression, ou par un transport qui
//! n'implémente pas la surjection — perdrait l'étage **sans aucune erreur** :
//! le message part, la P0 devient une P-rien, et la surface jamais muette n'est
//! jamais atteinte. C'est pourquoi le double d'observation ci-dessous surcharge
//! **les deux** méthodes et marque tout passage par `send()` d'une sentinelle
//! `SEVERITE_ABSENTE` : la fuite est un témoin, pas un silence.
//!
//! Site attesté : `agent_loop/mod.rs:4337` (notification de calage de
//! dispatch, `NotificationSeverity::P0`). Le site de production n'est **pas**
//! modifié par ce vol.
//!
//! # Recettes de rouge-avant (mika#1381 D5/AC7)
//!
//! Un test de sévérité qui n'a jamais été rouge n'atteste rien. Les trois
//! injections ci-dessous ont été exécutées, le site de production restauré à
//! l'identique après chacune, et la sortie d'échec est recopiée verbatim.
//!
//! **R1 — l'étage dégradé.** `NotificationSeverity::P0` → `::P2` à `:4337`.
//! ```text
//! assertion `left == right` failed: le calage de dispatch doit partir en P0
//! (surface jamais muette), observé Some(P2) ; texte : "Ready-label dispatch
//! stalled on senara-solutions/mika#1381: ..."
//!   left: Some(P2)
//!  right: Some(P0)
//! ```
//!
//! **R2 — la fuite silencieuse.** L'appel à `:4337` remplacé par
//! `sender.send(&notification)`. Cette forme **compile** : c'est exactement la
//! régression que le corps par défaut rend indolore.
//! ```text
//! assertion `left != right` failed: la notification de calage est arrivée au
//! transport SANS étage : le site agent_loop/mod.rs:4337 est passé par
//! `MessageSender::send`, dont le corps par défaut de `send_with_severity`
//! perd la sévérité en silence. La P0 n'atteint pas la surface jamais muette.
//! Texte : "Ready-label dispatch stalled on senara-solutions/mika#1381: ..."
//!   left: None
//!  right: None
//! ```
//!
//! **R3 — le prédicat.** `&& !ready_label_dispatch_satisfied(&all_tool_summaries)`
//! retiré du prédicat de `:4297`. Le contrôle négatif tombe sur une émission
//! qui n'aurait pas dû avoir lieu.
//! ```text
//! un dispatch a eu lieu, le transport ne devait rien observer ; émissions =
//! [("Ready-label dispatch stalled on senara-solutions/mika#1381: ...",
//! Some(P0))]
//! ```

use std::sync::{Arc, Mutex};

use anyhow::Result;
use async_trait::async_trait;
use mika_agent::messaging::{MessageSender, NotificationSeverity, SendOutcome};
use mika_common::llm::mock::*;
use serde_json::json;

use super::grounding_regressions::engine_correction_rejection::tools_with_dispatch_stubs;
use super::harness::EvalHarness;

/// Une émission observée au transport : le texte, et l'étage **tel qu'il est
/// arrivé**.
pub(crate) type Emission = (String, Option<NotificationSeverity>);

/// La sentinelle de fuite. Un site qui appelle `send()` au lieu de
/// `send_with_severity()` n'échoue pas — il arrive ici sans étage. Nommer
/// l'absence, c'est ce qui permet de l'assertionner.
pub(crate) const SEVERITE_ABSENTE: Option<NotificationSeverity> = None;

/// Le marqueur de dispatch ready-label, forme que
/// `webhook_dispatch::is_ready_label_dispatch_marker` reconnaît et dont
/// `parse_ready_label_location` extrait `senara-solutions/mika#1381`.
const READY_LABEL_MSG: &str = "[GitHub] Issue labeled ready on senara-solutions/mika#1381 — feat(notifications): notifications opérateur à sévérité\nhttps://github.com/senara-solutions/mika/issues/1381";

/// Le texte que le site amiral émet, assez pour l'identifier sans épingler la
/// phrase entière.
const NOTIFICATION_MARKER: &str = "Ready-label dispatch stalled";

// ---------------------------------------------------------------------------
// Le double d'observation — à double surcharge (D3)
// ---------------------------------------------------------------------------

/// Enregistre chaque émission **et par quelle porte elle est passée**.
///
/// Ne pas remplacer par `SeverityRecordingSender` de `verdict_handler.rs` : ce
/// type est `#[cfg(test)]` à l'intérieur du crate source, donc inatteignable
/// depuis le binaire de test `eval`. C'est le motif qui est réutilisé, pas le
/// type.
///
/// **Second consommateur, depuis le vol 4 :
/// `tests/eval/test_p2_ack_suppression_1381.rs`**, qui observe l'autre moitié de
/// la même propriété — l'absence d'émission sur le domaine Webhook Fallthrough,
/// et l'étage absent d'un accusé passé par l'outil `send_message`. `pub(crate)`
/// est à lui : la sentinelle `SEVERITE_ABSENTE` et la double surcharge sont ce
/// qui rend la fuite observable, et une seconde copie de ce double dériverait de
/// celle-ci sans que rien ne le dise (classe `grooming_marker`, mika#2158). Le
/// refus du `#[cfg(test)]` du crate source ci-dessus n'est **pas** inversé : il
/// reste inatteignable ; c'est ce fichier-ci qui est devenu le site unique du
/// motif pour le binaire `eval`.
///
/// **Troisième consommateur, depuis mika#2867** :
/// `test_fabrication_detected_p0_2867.rs` atteste au transport le site de la
/// classe `fabrication-detected` (la récidive de la garde `assert_grounded`).
/// Le double est donc `pub(crate)` plutôt que privé — un second double serait
/// un second comportement libre de diverger sur l'exacte question que ces
/// tests existent pour trancher : par quelle porte l'émission est passée.
pub(crate) struct SeverityRecordingSender {
    emissions: Mutex<Vec<Emission>>,
}

impl SeverityRecordingSender {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self {
            emissions: Mutex::new(Vec::new()),
        })
    }

    pub(crate) fn emissions(&self) -> Vec<Emission> {
        self.emissions.lock().unwrap().clone()
    }
}

#[async_trait]
impl MessageSender for SeverityRecordingSender {
    /// La porte sans étage. Tout ce qui arrive ici est un **témoin de fuite**.
    async fn send(&self, text: &str) -> Result<SendOutcome> {
        self.emissions
            .lock()
            .unwrap()
            .push((text.to_string(), SEVERITE_ABSENTE));
        // Le garde est relâché avant le retour : jamais tenu à travers un point
        // d'await (clippy::sig_drop, #1724).
        Ok(SendOutcome::Delivered)
    }

    async fn send_with_severity(
        &self,
        text: &str,
        severity: NotificationSeverity,
    ) -> Result<SendOutcome> {
        self.emissions
            .lock()
            .unwrap()
            .push((text.to_string(), Some(severity)));
        Ok(SendOutcome::Delivered)
    }
}

/// Le tour 1 commun aux deux scénarios : la garde d'intention
/// `webhook_ready_label_dispatch` n'est pas satisfaite (aucun outil de
/// dispatch), donc le moteur re-prompte une fois.
fn tour_1_sans_dispatch() -> Vec<MockResponse> {
    vec![
        tool_call_response(
            "run_gh",
            json!({"args": "issue edit 1381 --remove-label ready"}),
        ),
        tool_call_response(
            "send_message",
            json!({"message": "Ready label removed on mika#1381."}),
        ),
        text_response("Notified the operator on mika#1381."),
    ]
}

// ---------------------------------------------------------------------------
// AC2/AC3/AC4 — le test positif
// ---------------------------------------------------------------------------

/// Le calage de dispatch atteint le transport **en P0**, par la porte qui
/// porte l'étage.
#[tokio::test]
async fn mika1381_le_calage_de_dispatch_part_en_p0_sur_la_surface_jamais_muette()
-> anyhow::Result<()> {
    let sender = SeverityRecordingSender::new();

    let mut responses = tour_1_sans_dispatch();
    // Tour 2 (re-prompt correctif) : toujours aucun outil de dispatch — le
    // calage est consommé, le site amiral tire.
    responses.push(text_response("Still waiting on the dispatch slot."));

    let harness = EvalHarness::builder()
        .responses(responses)
        .tools(tools_with_dispatch_stubs())
        .message_sender(sender.clone())
        .build()
        .await?;

    let trace = harness.run(READY_LABEL_MSG).await?;

    // AC4 — la garde a bien tiré : sans re-prompt, `intent_guard_retries` ne
    // contient pas la garde et le site de `:4297` est inatteignable.
    assert!(
        trace.llm_call_count > 1,
        "la garde webhook_ready_label_dispatch n'a pas tiré \
         (llm_call_count = {}) : le site :4337 est resté hors d'atteinte, \
         le test n'atteste rien",
        trace.llm_call_count
    );

    let emissions = sender.emissions();
    let calages: Vec<&Emission> = emissions
        .iter()
        .filter(|(text, _)| text.contains(NOTIFICATION_MARKER))
        .collect();

    // AC2 — exactement une notification de calage.
    assert_eq!(
        calages.len(),
        1,
        "attendu exactement une notification de calage au transport, \
         observé {} ; émissions = {:?}",
        calages.len(),
        emissions
    );

    let (texte, etage) = calages[0];

    // AC3 — l'étage n'est pas absent : le site est passé par
    // `send_with_severity`, pas par le corps par défaut qui délègue à `send`.
    assert_ne!(
        *etage, SEVERITE_ABSENTE,
        "la notification de calage est arrivée au transport SANS étage : le \
         site agent_loop/mod.rs:4337 est passé par `MessageSender::send`, dont \
         le corps par défaut de `send_with_severity` perd la sévérité en \
         silence. La P0 n'atteint pas la surface jamais muette. Texte : {texte:?}"
    );

    // AC2 — et cet étage est P0, pas un autre.
    assert_eq!(
        *etage,
        Some(NotificationSeverity::P0),
        "le calage de dispatch doit partir en P0 (surface jamais muette), \
         observé {etage:?} ; texte : {texte:?}"
    );

    // La location parsée du marqueur voyage bien dans le corps.
    assert!(
        texte.contains("senara-solutions/mika#1381"),
        "la notification ne nomme pas le ticket calé ; texte : {texte:?}"
    );

    Ok(())
}

// ---------------------------------------------------------------------------
// AC5 — le contrôle négatif, sur le prédicat (D4)
// ---------------------------------------------------------------------------

/// Un dispatch qui a bien eu lieu ne notifie rien.
///
/// Sans ce contrôle, un site qui notifierait **à chaque** tour webhook
/// passerait le test positif. Ce qui est attesté ici, c'est le prédicat de
/// `:4297` : `ready_label_dispatch_satisfied` devient vrai au tour 2, et le
/// transport n'observe plus rien du tout.
#[tokio::test]
async fn mika1381_un_dispatch_qui_a_eu_lieu_ne_notifie_rien() -> anyhow::Result<()> {
    let sender = SeverityRecordingSender::new();

    let mut responses = tour_1_sans_dispatch();
    // Tour 2 : la correction est honorée — `create_task` + `run_claude_pilot`
    // satisfont la garde.
    responses.push(tool_call_response(
        "create_task",
        json!({"label": "dispatch mika#1381"}),
    ));
    responses.push(tool_call_response(
        "run_claude_pilot",
        json!({"skill": "dev-pilot", "prompt": "senara-solutions/mika#1381"}),
    ));
    responses.push(text_response("Dispatch fired for mika#1381."));

    let harness = EvalHarness::builder()
        .responses(responses)
        .tools(tools_with_dispatch_stubs())
        .message_sender(sender.clone())
        .build()
        .await?;

    let trace = harness.run(READY_LABEL_MSG).await?;

    assert!(
        trace.llm_call_count > 1,
        "la garde n'a pas tiré (llm_call_count = {}) : le contrôle négatif \
         serait vert pour la mauvaise raison",
        trace.llm_call_count
    );

    let emissions = sender.emissions();
    assert!(
        emissions.is_empty(),
        "un dispatch a eu lieu, le transport ne devait rien observer ; \
         émissions = {emissions:?}"
    );

    Ok(())
}
