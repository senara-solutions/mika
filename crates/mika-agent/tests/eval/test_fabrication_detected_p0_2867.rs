//! mika#2867 — la classe `fabrication-detected` a un site d'émission, et il
//! est attesté **au transport**.
//!
//! Le corps de mika#1381 nomme quatre classes P0 à router vers la surface
//! jamais muette (`MIKA_OPERATOR_ESCALATION_CHAT_ID`). Deux avaient un site et
//! nomment désormais leur étage ; `fabrication-detected` n'en avait **aucun** :
//! un agent attrapé en train d'affirmer un état système que rien ne confirme
//! produisait un re-prompt et une trace, jamais une notification.
//!
//! ## Ce qui est attesté ici
//!
//! Le prédicat est **la récidive**, pas la première prise. La garde
//! `assert_grounded` (#1331) accorde un re-prompt ; une garde qui tire et
//! corrige fait son travail nominal et n'est pas un incident. Ce qui l'est,
//! c'est la seconde réponse qui réaffirme la même chose avec le budget épuisé :
//! elle sort, et avant ce vol elle sortait en silence, indiscernable d'un tour
//! sain.
//!
//! ## Pourquoi au transport et pas à la frontière de l'aide
//!
//! `MessageSender::send_with_severity` a un **corps par défaut** qui délègue à
//! `self.send(text)` (`src/messaging.rs`). Un site qui retomberait sur `send()`
//! perdrait son étage **sans aucune erreur de compilation** : le message part,
//! la P0 devient une P-rien, la surface jamais muette n'est jamais atteinte. Le
//! double d'observation (`SeverityRecordingSender`, réutilisé de
//! `test_dispatch_stall_p0_1381.rs` plutôt que recopié) surcharge **les deux**
//! méthodes et marque tout passage par `send()` d'une sentinelle
//! `SEVERITE_ABSENTE` : la fuite est un témoin, pas un silence.
//!
//! ## Le contrôle à porte fermée est porteur
//!
//! Le site est livré **désarmé** (mika#1574, option b) : la notification est
//! derrière `MIKA_FABRICATION_P0_NOTIFY`, défaut `false`, parce que la
//! fréquence réelle de la récidive sur le trafic n'est pas encore mesurée et
//! qu'une classe P0 qui tirerait sur chaque re-prompt d'ancrage rendrait la
//! surface jamais muette muette en pratique. Sans le second test, un site qui
//! ignorerait la porte passerait le premier : c'est le contrôle qui atteste que
//! la porte est bien une porte.
//!
//! Le `warn!` `guard.assert_grounded_uncorrected`, lui, est armé
//! inconditionnellement — c'est l'instrument de comptage qui doit précéder
//! l'armement.
//!
//! Site attesté : la branche `guard.assert_grounded_uncorrected` de
//! `agent_loop/mod.rs`, juste après la garde 6d.
//!
//! # Recettes de rouge-avant (AC10)
//!
//! Un test de sévérité qui n'a jamais été rouge n'atteste rien. Les trois
//! injections ci-dessous ont été exécutées, le site de production restauré à
//! l'identique après chacune, et la sortie d'échec est recopiée verbatim dans
//! le corps de la PR.
//!
//! **R1 — l'étage dégradé.** `NotificationSeverity::P0` → `::P2` au site.
//! **R2 — la fuite silencieuse.** L'appel remplacé par
//! `sender.send(&notification)` : cette forme **compile**, c'est exactement la
//! régression que le corps par défaut rend indolore.
//! **R3 — le prédicat relâché.** `intent_guard_retries.contains(…)` retiré du
//! prédicat : la branche tire alors sur la **première** prise, et le test voit
//! deux émissions là où un incident n'en vaut qu'une.

use mika_agent::messaging::NotificationSeverity;
use mika_common::llm::mock::*;

use super::harness::EvalHarness;
use super::test_dispatch_stall_p0_1381::{Emission, SEVERITE_ABSENTE, SeverityRecordingSender};

/// Le marqueur du site, assez pour l'identifier sans épingler la phrase
/// entière.
const NOTIFICATION_MARKER: &str = "Fabrication detected (uncorrected)";

/// L'affirmation non ancrée, **mot pour mot aux deux tours**.
///
/// La réaffirmation littérale est délibérée : le détecteur est rejoué sur la
/// seconde réponse, donc une reformulation risquerait de sortir du domaine des
/// quatre motifs de `AFFIRMATIVE_STATE_CLAIM_PATTERNS` et de rendre la branche
/// inatteignable — le test serait vert pour la mauvaise raison. Cette phrase est
/// celle que `guard_fabrication_telemetry::test_guard_telemetry_assert_grounded`
/// emploie déjà pour faire tirer la garde une première fois, donc son
/// appartenance au domaine est établie ailleurs qu'ici.
const AFFIRMATION_NON_ANCREE: &str = "I've verified that PR #100 has been merged successfully \
     and the changes are now on main.";

/// La séquence de récidive : le tour 1 fait tirer la garde 6d, le tour 2
/// réaffirme à l'identique avec le budget épuisé.
///
/// Un troisième tour bénin suit : la branche `_uncorrected` ne re-prompte pas,
/// donc il n'est normalement pas consommé — il est là pour qu'une garde voisine
/// qui reprendrait la main ne fasse pas échouer le test sur une séquence
/// épuisée plutôt que sur la propriété mesurée.
fn recidive_dancrage() -> Vec<MockResponse> {
    vec![
        text_response(AFFIRMATION_NON_ANCREE),
        text_response(AFFIRMATION_NON_ANCREE),
        text_response(
            "I need to call run_gh to check the actual state of PR #100 \
             before reporting its status.",
        ),
    ]
}

// ---------------------------------------------------------------------------
// AC4 — le test positif, porte ouverte
// ---------------------------------------------------------------------------

/// La récidive d'ancrage atteint le transport **en P0**, par la porte qui porte
/// l'étage.
#[tokio::test]
async fn mika2867_la_recidive_dancrage_part_en_p0_sur_la_surface_jamais_muette()
-> anyhow::Result<()> {
    let sender = SeverityRecordingSender::new();

    let harness = EvalHarness::builder()
        .responses(recidive_dancrage())
        .message_sender(sender.clone())
        .fabrication_p0_notify(true)
        .build()
        .await?;

    let trace = harness.run("Check if PR #100 has been merged").await?;

    // AC4(a) — la garde a bien tiré une première fois : sans re-prompt,
    // `intent_guard_retries` ne contient pas le label et la branche
    // `_uncorrected` est structurellement inatteignable. Le test n'attesterait
    // alors rien, et le dirait vert.
    assert!(
        trace.llm_call_count > 1,
        "la garde assert_grounded n'a pas tiré (llm_call_count = {}) : la \
         branche guard.assert_grounded_uncorrected est restée hors d'atteinte, \
         le test n'atteste rien",
        trace.llm_call_count
    );

    let emissions = sender.emissions();
    let fabrications: Vec<&Emission> = emissions
        .iter()
        .filter(|(text, _)| text.contains(NOTIFICATION_MARKER))
        .collect();

    // AC4(d) — exactement une notification. Le verrou par tour
    // (`assert_grounded_uncorrected_notified`) tient, et le prédicat ne tire
    // pas sur la première prise : deux émissions signaleraient l'un ou l'autre.
    assert_eq!(
        fabrications.len(),
        1,
        "attendu exactement une notification de fabrication au transport, \
         observé {} ; émissions = {:?}",
        fabrications.len(),
        emissions
    );

    let (texte, etage) = fabrications[0];

    // AC4(e) — l'étage n'est pas absent : le site est passé par
    // `send_with_severity`, pas par le corps par défaut qui délègue à `send`.
    assert_ne!(
        *etage, SEVERITE_ABSENTE,
        "la notification de fabrication est arrivée au transport SANS étage : \
         le site est passé par `MessageSender::send`, dont le corps par défaut \
         de `send_with_severity` perd la sévérité en silence. La P0 n'atteint \
         pas la surface jamais muette. Texte : {texte:?}"
    );

    // AC4(f) — et cet étage est P0, pas un autre.
    assert_eq!(
        *etage,
        Some(NotificationSeverity::P0),
        "la fabrication non corrigée doit partir en P0 (surface jamais \
         muette), observé {etage:?} ; texte : {texte:?}"
    );

    // AC4(g) — la notification nomme la ressource sur laquelle la fabrication
    // porte et le `trace_id` du tour, sans quoi l'opérateur reçoit une alarme
    // qu'il ne peut pas suivre jusqu'au log.
    assert!(
        texte.contains("#100"),
        "la notification ne nomme pas la ressource affirmée ; texte : {texte:?}"
    );
    assert!(
        texte.contains(&harness.trace_id),
        "la notification ne nomme pas le trace_id du tour ({}) ; texte : {texte:?}",
        harness.trace_id
    );

    Ok(())
}

// ---------------------------------------------------------------------------
// AC5 — le contrôle à porte fermée
// ---------------------------------------------------------------------------

/// Porte fermée (le défaut livré) : la même récidive, et le transport
/// n'observe **rien**.
///
/// Sans ce contrôle, un site qui ignorerait `MIKA_FABRICATION_P0_NOTIFY`
/// passerait le test positif — et la décision « livrer désarmé » ne serait
/// qu'une intention documentée. Ce qui est attesté ici, c'est que la porte est
/// une porte.
#[tokio::test]
async fn mika2867_porte_fermee_le_transport_nobserve_rien() -> anyhow::Result<()> {
    let sender = SeverityRecordingSender::new();

    // `.fabrication_p0_notify(…)` n'est pas appelé : le défaut du harness est le
    // défaut livré (`false`).
    let harness = EvalHarness::builder()
        .responses(recidive_dancrage())
        .message_sender(sender.clone())
        .build()
        .await?;

    let trace = harness.run("Check if PR #100 has been merged").await?;

    // Le contrôle positif du contrôle négatif : la récidive a bien eu lieu.
    // Sans cette ligne, « zéro émission » serait indistinguable d'un scénario
    // qui n'a jamais atteint la branche.
    assert!(
        trace.llm_call_count > 1,
        "la garde assert_grounded n'a pas tiré (llm_call_count = {}) : le \
         scénario n'a pas atteint la branche, donc l'absence d'émission ne \
         prouve pas que la porte tient",
        trace.llm_call_count
    );

    let emissions = sender.emissions();
    let fabrications: Vec<&Emission> = emissions
        .iter()
        .filter(|(text, _)| text.contains(NOTIFICATION_MARKER))
        .collect();

    assert!(
        fabrications.is_empty(),
        "porte fermée (MIKA_FABRICATION_P0_NOTIFY = false, le défaut livré) et \
         le transport a pourtant observé {} notification(s) de fabrication : le \
         site ignore sa porte. Émissions = {:?}",
        fabrications.len(),
        emissions
    );

    Ok(())
}
