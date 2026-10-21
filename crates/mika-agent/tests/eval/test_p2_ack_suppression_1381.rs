//! mika#1381 (vol 4/4) — la suppression des accusés P2, observée au transport.
//!
//! Ferme le second axe du ticket : *« batch/suppress P2 acks via INTENT_GUARD
//! relaxation (let mika-dev EndTurn without send_message on non-actionable
//! webhooks) »*. **Aucune ligne de moteur n'est écrite par ce vol** : la
//! relaxation demandée est déjà en production depuis mika#2517 U3 (commit
//! `b1d96948`, PR #2530, mergée le 2026-09-25), qui a stand-down `webhook_zero_tools`
//! sur tout le domaine Webhook Fallthrough. Ce fichier est ce qui manquait pour
//! que la fermeture soit **attestée** plutôt qu'affirmée.
//!
//! # Pourquoi au transport et pas à la trace d'outils
//!
//! `test_webhook_zero_tools_guard.rs § guard_stands_down_on_issue_webhook`
//! atteste déjà la **garde** : un tour fallthrough sans outil n'est pas
//! re-prompté. Mais « la garde ne re-prompte pas » et « l'opérateur ne reçoit
//! rien » sont deux propositions distinctes, et c'est la seconde que le ticket
//! demande. Un compteur de pas reste vert si un site de notification émet par
//! lui-même, hors de toute trace d'outil — exactement la forme des sept sites P0
//! du vol 1, qui n'apparaissent dans aucun `calls_for_tool`. Le seul instrument
//! qui sépare les deux est un `MessageSender` injecté : il voit ce que
//! l'opérateur verrait, et rien d'autre.
//!
//! Le double d'observation est donc celui du vol 3,
//! [`super::test_dispatch_stall_p0_1381::SeverityRecordingSender`], réutilisé et
//! non recopié (D3) : sa double surcharge et sa sentinelle
//! [`SEVERITE_ABSENTE`](super::test_dispatch_stall_p0_1381::SEVERITE_ABSENTE)
//! sont ce qui rend une émission sans étage **observable**. Une seconde copie
//! aurait dérivé de celle-ci sans que rien ne le dise.
//!
//! # Ce que le troisième test attache au ticket
//!
//! `tools/send_message.rs:142` passe par `sender.send(&cleaned)` — jamais par
//! `send_with_severity`. Un accusé émis par l'outil arrive donc au transport
//! **sans étage**, et le corps par défaut de `send_with_severity`
//! (`src/messaging.rs`) rend cette perte indolore. Conséquence, et c'est la
//! propriété que ce vol épingle : **la suppression des accusés P2 est une
//! propriété du garde, jamais du routage.** Rien en aval ne peut distinguer un
//! accusé P2 d'un message ordinaire, donc rien en aval ne peut le retenir ; la
//! seule porte est celle qui décide si le tour doit parler.
//!
//! # Recettes de rouge-avant (mika#1381 D8/AC7)
//!
//! Un test de suppression qui n'a jamais été rouge n'atteste rien. Les trois
//! injections ci-dessous ont été exécutées, le site de production restauré à
//! l'identique après chacune (`git diff` sur `src/` vide à l'arrivée), et la
//! sortie d'échec est recopiée verbatim.
//!
//! **R1 — la relaxation retirée.** Les trois lignes
//! `if crate::webhook_dispatch::is_webhook_fallthrough_domain(msg) { return false; }`
//! supprimées de `webhook_zero_tools_trigger` (`agent_loop/mod.rs:11251`). La
//! garde se ré-arme sur le domaine, le tour qui avait obéi est re-prompté.
//! ```text
//! assertion `left == right` failed: la garde webhook_zero_tools s'est ré-armée
//! sur le domaine Webhook Fallthrough : le tour avait obéi et il a été
//! re-prompté (llm_call_count = 2), ce que mika#2517 U3 a précisément retiré
//!   left: 2
//!  right: 1
//! ```
//!
//! **R2 — la garde retirée en bloc.** `is_webhook_fallthrough_domain`
//! (`webhook_dispatch.rs:99`) rendue **`true`** partout. Le contrôle positif
//! tombe : sans lui, le premier test lirait exactement pareil que la garde soit
//! stand-down sur le bon domaine ou retirée partout.
//!
//! > **Écart assumé avec le plan (D8/R2), et c'est une correction.** Le plan
//! > prescrivait `false` partout en annonçant que le second test rougirait.
//! > `false` partout *arme* la garde partout : c'est le **premier** test qui
//! > rougit, à l'identique de R1, et le second reste vert — l'injection
//! > n'atteste donc rien du rôle du contrôle positif. L'injection qui le met à
//! > l'épreuve est l'inverse, et c'est celle qui a été exécutée.
//! ```text
//! assertion `left == right` failed: la garde webhook_zero_tools n'a pas tiré
//! hors du domaine Webhook Fallthrough (llm_call_count = 1) : le stand-down de
//! mika#2517 U3 a débordé sur une population qu'il ne devait pas toucher, et le
//! test frère serait vert pour la mauvaise raison
//!   left: 1
//!  right: 2
//! ```
//!
//! **R3 — le double débranché.** `SeverityRecordingSender` remplacé par
//! `mika_agent::messaging::NoopSender` à l'injection du troisième test.
//! L'anti-vacuité tombe : le test n'observe plus rien, donc ne peut plus
//! attester que le chemin d'observation est branché.
//! ```text
//! assertion `left == right` failed: attendu exactement une émission au
//! transport (celle de l'outil), observé 0 ; émissions = []
//!   left: 0
//!  right: 1
//! ```

use mika_agent::tools::default_tools;
use mika_common::llm::mock::*;
use serde_json::json;

use super::harness::EvalHarness;
use super::test_dispatch_stall_p0_1381::{SEVERITE_ABSENTE, SeverityRecordingSender};

/// Le déclencheur du domaine Webhook Fallthrough : un commentaire d'issue, ni
/// marqueur de dispatch ready-label, ni événement PR (territoire qa), ni
/// événement check-suite (territoire ci).
///
/// `New comment on` plutôt que `Issue opened:` (D5) : les deux sont dans le
/// domaine, et la seconde forme est déjà couverte au niveau de la garde par
/// `guard_stands_down_on_issue_webhook`. Prendre l'autre forme fait que les deux
/// attestations ne tombent pas ensemble si un jour le domaine se rétrécit sur
/// une seule des deux.
const FALLTHROUGH_MSG: &str = "[GitHub] New comment on senara-solutions/mika#1381 by samidarko\nhttps://github.com/senara-solutions/mika/issues/1381#issuecomment-1381";

/// **Hors** du domaine : `CHECK_SUITE_EVENT_PREFIX` l'en exclut (territoire ci),
/// et `webhook_zero_tools_trigger` n'écarte que `Check suite success on`. Un
/// échec de CI est donc le tour `[GitHub]` sur lequel la garde est **encore
/// armée** — c'est ce qui en fait le contrôle positif.
const HORS_DOMAINE_MSG: &str = "[GitHub] Check suite failure on senara-solutions/mika@c5589849 — branch feat/1381/notifications-severity-tiered-operator\nhttps://github.com/senara-solutions/mika/commit/c5589849";

// ---------------------------------------------------------------------------
// AC1/AC2 — le tour non actionnable ne parle à personne
// ---------------------------------------------------------------------------

/// Un tour Webhook Fallthrough sans appel d'outil se clôt **sans re-prompt et
/// sans aucune émission au transport**.
///
/// Les deux moitiés comptent. Le `llm_call_count == 1` dit que la garde est bien
/// stand-down ; les `emissions` vides disent que personne n'a prévenu
/// l'opérateur par une porte que la trace d'outils ne montre pas. La première
/// seule est déjà tenue ailleurs ; la seconde est ce que le vol 4 ajoute.
#[tokio::test]
async fn mika1381_un_webhook_non_actionnable_ne_part_en_aucun_accuse() -> anyhow::Result<()> {
    let sender = SeverityRecordingSender::new();

    let harness = EvalHarness::builder()
        .responses(vec![
            // Le tour obéit à la HARD GATE § Webhook Fallthrough : accuser et
            // s'arrêter, sans outil.
            text_response("Noted the comment on mika#1381; no action is required."),
            // Une seconde réponse est fournie **exprès** (D6). Si la garde se
            // ré-armait, le mock la servirait et `llm_call_count` passerait à 2 ;
            // sans elle le harnais tomberait sur un manque de réponse, et
            // l'échec ne dirait pas *pourquoi*.
            text_response("Still nothing to do on mika#1381."),
        ])
        .tools(default_tools())
        .message_sender(sender.clone())
        .build()
        .await?;

    let trace = harness.run(FALLTHROUGH_MSG).await?;

    assert_eq!(
        trace.llm_call_count, 1,
        "la garde webhook_zero_tools s'est ré-armée sur le domaine Webhook \
         Fallthrough : le tour avait obéi et il a été re-prompté \
         (llm_call_count = {}), ce que mika#2517 U3 a précisément retiré",
        trace.llm_call_count
    );

    let emissions = sender.emissions();
    assert!(
        emissions.is_empty(),
        "un tour non actionnable a tout de même parlé à l'opérateur ; l'accusé P2 \
         que mika#1381 demande de supprimer est arrivé au transport — \
         émissions = {emissions:?}"
    );

    Ok(())
}

// ---------------------------------------------------------------------------
// AC3 — le contrôle positif, hors du domaine
// ---------------------------------------------------------------------------

/// Hors du domaine, la garde exige toujours un outil.
///
/// Sans ce contrôle, le test précédent lirait **exactement pareil** que
/// `webhook_zero_tools` soit stand-down sur le bon domaine ou retirée en bloc —
/// et une garde retirée en bloc est une régression de sécurité, pas la
/// relaxation que le ticket demandait. C'est le même raisonnement, et le même
/// rôle, que les deux contrôles positifs de
/// `test_webhook_zero_tools_guard.rs` ; il est rejoué ici parce qu'un fichier
/// dont le seul test est une absence ne peut pas se relire tout seul.
#[tokio::test]
async fn mika1381_hors_du_domaine_la_garde_exige_toujours_un_outil() -> anyhow::Result<()> {
    let sender = SeverityRecordingSender::new();

    let harness = EvalHarness::builder()
        .responses(vec![
            // Tour 1 : texte seul — la garde rejette et re-prompte.
            text_response("The check suite failed on the feature branch."),
            // Tour 2 : texte seul à nouveau — la garde a déjà tiré (sémantique
            // de retry unique), le tour se clôt.
            text_response("Noted the CI failure."),
        ])
        .tools(default_tools())
        .message_sender(sender.clone())
        .build()
        .await?;

    let trace = harness.run(HORS_DOMAINE_MSG).await?;

    assert_eq!(
        trace.llm_call_count, 2,
        "la garde webhook_zero_tools n'a pas tiré hors du domaine Webhook \
         Fallthrough (llm_call_count = {}) : le stand-down de mika#2517 U3 \
         a débordé sur une population qu'il ne devait pas toucher, et le test \
         frère serait vert pour la mauvaise raison",
        trace.llm_call_count
    );

    Ok(())
}

// ---------------------------------------------------------------------------
// AC4 — anti-vacuité : l'instrument voit ce qu'il doit voir
// ---------------------------------------------------------------------------

/// Sur le **même** déclencheur, un accusé réellement émis par l'outil
/// `send_message` arrive au transport — et il y arrive **sans étage**.
///
/// Deux choses à la fois, et elles se tiennent. D'abord l'anti-vacuité : le
/// premier test assertionne une absence, donc il serait vert si le
/// `MessageSender` injecté n'était jamais atteint sur ce déclencheur. Ici le même
/// scénario produit une émission, ce qui prouve que le chemin d'observation est
/// branché.
///
/// Ensuite la propriété que le vol 4 attache au ticket : `tools/send_message.rs:142`
/// appelle `sender.send(&cleaned)` et jamais `send_with_severity`, donc l'étage
/// arrive [`SEVERITE_ABSENTE`]. **Rien en aval ne peut donc retenir un accusé
/// P2** : la seule porte est le garde, et c'est pourquoi la suppression demandée
/// par mika#1381 n'a jamais pu être une affaire de routage.
#[tokio::test]
async fn mika1381_un_accuse_emis_par_loutil_arrive_sans_etage() -> anyhow::Result<()> {
    let sender = SeverityRecordingSender::new();

    let harness = EvalHarness::builder()
        .responses(vec![
            // Tour 1 : l'agent accuse explicitement par l'outil. Le paramètre
            // est `text` — le vrai `SendMessageTool` refuse sur `'text' is
            // required.` et n'atteint jamais le transport sous un autre nom,
            // d'où l'assertion de non-vacuité plus bas qui, seule, ne suffit
            // pas (un refus est tout de même un appel).
            tool_call_response(
                "send_message",
                json!({"text": "Acknowledged the comment on mika#1381."}),
            ),
            text_response("Acknowledged on mika#1381."),
            // Marge, pour la même raison qu'au premier test.
            text_response("Nothing further on mika#1381."),
        ])
        .tools(default_tools())
        .message_sender(sender.clone())
        .build()
        .await?;

    let trace = harness.run(FALLTHROUGH_MSG).await?;

    // L'outil a bien été servi : sur le domaine fallthrough `create_task` est
    // retenu (mika#2517 U2) et `run_gh` refusé sur ses verbes d'écriture
    // (mika#2573), mais `send_message` est la sortie que ces refus nomment.
    assert!(
        !trace.calls_for_tool("send_message").is_empty(),
        "le scénario n'a pas atteint `send_message` : sans appel il n'y a rien à \
         observer, et l'anti-vacuité du test frère n'est pas fournie"
    );

    let emissions = sender.emissions();
    assert_eq!(
        emissions.len(),
        1,
        "attendu exactement une émission au transport (celle de l'outil), \
         observé {} ; émissions = {:?}",
        emissions.len(),
        emissions
    );

    let (texte, etage) = &emissions[0];
    assert!(
        texte.contains("mika#1381"),
        "l'émission observée n'est pas l'accusé attendu ; texte : {texte:?}"
    );
    assert_eq!(
        *etage, SEVERITE_ABSENTE,
        "un accusé passé par l'outil `send_message` est arrivé AVEC un étage : \
         `tools/send_message.rs:142` passe par `MessageSender::send`, donc \
         l'étage ne peut pas être là. S'il y est, le routage a gagné un étage \
         par défaut — et la suppression des accusés P2 cesse d'être une \
         propriété du seul garde, ce que ce vol atteste. Observé {etage:?} ; \
         texte : {texte:?}"
    );

    Ok(())
}
