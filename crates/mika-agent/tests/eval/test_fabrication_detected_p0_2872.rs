//! mika#2872 — la classe `fabrication-detected` gagne son **second** site, et
//! le créneau devient typé.
//!
//! mika#2867 a rattaché un seul garde de la famille `_uncorrected` à la classe
//! P0 `fabrication-detected` : la récidive d'`assert_grounded`. Six autres
//! sites de la famille avaient leur `warn!` et aucun étage de notification. Ce
//! vol propose un verdict pour chacun des six (ils sont dans le corps de la PR,
//! un par site) et implémente le seul dont le verdict est entier :
//! `false_local_hosting_claim` (garde 5d, mika#2290).
//!
//! ## Le critère qui décide du rattachement
//!
//! Du bearing de Prime du 2026-10-22, rendu décidable : *la phrase fautive
//! affirme-t-elle un état du système qu'aucun résultat d'outil de ce tour ne
//! porte ?* Pour 5d, oui — « je tourne en local sur ta machine » est un état du
//! **déploiement**, aucun outil ne le porte, et `tool_ctx.deployment` le
//! contredit. Quatre des six sites sont hors classe par ce même critère : une
//! dérive de langue, un « bonsoir » à 9 h, une alternative d'accès et un accusé
//! d'envoi omis sont des divergences de forme, de registre ou de conduite,
//! mesurées contre un fait que le garde **tient déjà** — il n'y a rien à aller
//! vérifier, donc pas de geste pour l'opérateur, donc pas de P0. Le critère
//! entier vit sur `PendingFabricationP0Site`, pas dans ce fichier seul.
//!
//! ## Ce qui est attesté ici
//!
//! Même forme que `test_fabrication_detected_p0_2867.rs`, dont ce fichier est
//! le frère et dont il réutilise le double d'observation : le prédicat est **la
//! récidive**, pas la première prise (5d accorde un re-prompt ; une garde qui
//! tire et corrige fait son travail nominal), et l'assertion est **au
//! transport**, parce que `MessageSender::send_with_severity` a un corps par
//! défaut qui délègue à `self.send(text)` — un site qui régresserait sur
//! `send()` perdrait son étage sans aucune erreur de compilation.
//!
//! ## Ce que le refactor du créneau ajoute, et que ce fichier tient
//!
//! Le créneau `pending_fabrication_p0` est passé de `Option<String>` à
//! `Option<PendingFabricationP0>`, qui porte **son site**. La revérification au
//! point d'acceptation de l'`EndTurn` rejoue désormais le prédicat du site qui
//! a déposé plutôt qu'un prédicat câblé en dur sur `assert_grounded`. C'est ce
//! que tient le troisième test, et c'est pourquoi le fixture ci-dessous ne
//! porte **délibérément aucune** forme reconnue par
//! `detect_affirmative_state_claim` : si elle en portait une, les deux bras du
//! `match` de `still_violated` seraient interchangeables sur ce scénario et le
//! refactor serait décoratif — le test resterait vert avec le mauvais prédicat
//! câblé.
//!
//! Site attesté : la branche `guard.false_local_hosting_claim_uncorrected` de
//! `agent_loop/mod.rs`, juste après la garde 5d.
//!
//! ## La porte reste désarmée
//!
//! Le site est livré **désarmé** comme son frère (`MIKA_FABRICATION_P0_NOTIFY`,
//! défaut `false`, mika#1574 option (b)) : la fréquence réelle de la récidive
//! n'est pas mesurée, et armer la classe relève de mika#2871. Le second test
//! atteste que la porte est bien une porte.
//!
//! # Recettes de rouge-avant (plan R1)
//!
//! Les trois tests sont passés **verts du premier coup**, ce qui n'atteste
//! rien. Chaque injection ci-dessous a donc été jouée sur l'arbre, le test
//! lancé, puis le site restauré par `git checkout`. Ce qui suit est ce qui a
//! été observé, pas ce qui était prévu.
//!
//! **R1 — l'étage dégradé.** `NotificationSeverity::P0` → `::P2` au site
//! d'acceptation ⇒ rouge sur AC6(f), `observé Some(P2)`.
//!
//! **R2 — la fuite silencieuse.** L'appel remplacé par
//! `sender.send(&pending.message)`, qui **compile** ⇒ rouge sur AC6(e),
//! « arrivée au transport SANS étage ». C'est la régression que rien d'autre
//! qu'une assertion au transport ne peut voir.
//!
//! **R3 — le site retiré.** `if false && …` en tête du dépôt de 5d ⇒ rouge sur
//! AC6(d), `observé 0`.
//!
//! **R4 — la porte ignorée.** `fabrication_p0_notify &&` retiré du dépôt ⇒
//! rouge sur `mika2872_porte_fermee_le_transport_nobserve_rien`, « le site
//! ignore sa porte », avec l'émission observée en `Some(P0)`.
//!
//! **R5 — le refactor défait, et c'est la recette qui le justifie.** Le
//! prédicat de revérification recâblé en dur sur
//! `detect_affirmative_state_claim` + `assert_grounded_satisfied`, comme avant
//! ce vol ⇒ **les trois tests de mika#2867 restent verts** et le test positif
//! ci-dessus rougit à `observé 0` : la P0 déposée par 5d est supprimée par le
//! prédicat de l'autre site, sur un texte qui porte pourtant toujours la fausse
//! revendication. Sans cette recette, « le créneau est typé » serait une
//! affirmation sans contrôle.
//!
//! **R6 — le verrou de CLASSE retiré : reste VERT, et le site est donc livré
//! non attesté.** `pending_fabrication_p0.is_none()` retiré du dépôt de 5d ⇒
//! les six tests de la classe restent verts. La raison est dans les fixtures :
//! aucun des six ne produit **deux** violations de la classe dans un même tour,
//! donc le second écrivain n'existe nulle part et le verrou n'a rien à
//! empêcher. Ce que `is_none()` garantit — une seule P0 par tour pour toute la
//! classe, le premier écrivain de la chaîne gagnant — est tenu par le compilateur
//! et par relecture, pas par ce fichier. **Le fixture de collision (un tour qui
//! viole 5d *et* `assert_grounded`) appartient au vol 2** avec le site 2
//! (`unactioned_frequency_promise`), parce qu'il n'a de sens qu'avec un
//! troisième site qui rend la collision ordinaire. Écrit ici plutôt que dans
//! le plan : c'est le fichier de test qui doit dire ce qu'il ne tient pas.

use mika_agent::messaging::NotificationSeverity;
use mika_common::llm::mock::*;

use super::harness::EvalHarness;
use super::test_dispatch_stall_p0_1381::{Emission, SEVERITE_ABSENTE, SeverityRecordingSender};

/// Le marqueur du site, assez pour l'identifier **et** pour le distinguer de
/// celui de mika#2867 : les deux notifications partagent le préfixe de classe
/// `Fabrication detected (uncorrected)`, et un filtre sur ce seul préfixe
/// confondrait les deux sites.
const NOTIFICATION_MARKER: &str = "Fabrication detected (uncorrected) on deployment locality";

/// La fausse revendication de localité, **mot pour mot aux deux tours**.
///
/// La réaffirmation littérale est délibérée, pour la raison que mika#2867 a
/// déjà écrite : le détecteur est rejoué sur la seconde réponse, donc une
/// reformulation risquerait de sortir du domaine des deux couches de
/// `detect_false_local_hosting_claim` et de rendre la branche inatteignable —
/// le test serait vert pour la mauvaise raison.
///
/// **Elle ne porte aucune forme d'`detect_affirmative_state_claim`**, et c'est
/// une contrainte, pas un hasard : ni `issue`/`PR`/`task`/`ticket`, ni
/// « I checked/verified … », ni `#N`. C'est ce qui rend le troisième test
/// capable de pincer le prédicat de revérification (voir l'en-tête).
///
/// Deux couches tirent sur cette phrase. Couche B (assertion portant son
/// sujet) : `je tourne`. Couche A (prédicat de localité) : `en local`. Elles
/// sont à deux caractères l'une de l'autre, loin sous `CLAIM_GAP_MAX`, dans la
/// même phrase, sans marqueur d'auto-hébergement ni interrogation — les trois
/// suppresseurs de la garde.
const FAUSSE_LOCALITE: &str = "Aucune inquiétude pour la confidentialité : je tourne en local \
     sur ta machine, donc tes données ne quittent jamais ton ordinateur.";

/// La séquence de récidive : le tour 1 fait tirer la garde 5d, le tour 2
/// réaffirme à l'identique avec le budget épuisé.
fn recidive_de_localite() -> Vec<MockResponse> {
    vec![
        text_response(FAUSSE_LOCALITE),
        text_response(FAUSSE_LOCALITE),
    ]
}

/// La question du tenant, en français comme la réponse.
///
/// La langue est accordée exprès : la garde 5f (`response_language_drift`)
/// est dans la même chaîne, et une question anglaise avec une réponse
/// française lui ferait reprendre la main avant que la récidive de 5d
/// n'atteigne le point d'acceptation. Le test serait alors vert sur un
/// scénario qui n'atteint pas le site.
const QUESTION_DU_TENANT: &str = "Est-ce que mes données partent sur un serveur quelque part ?";

// ---------------------------------------------------------------------------
// AC6 — le test positif, porte ouverte
// ---------------------------------------------------------------------------

/// La récidive de fausse localité atteint le transport **en P0**, par la porte
/// qui porte l'étage.
#[tokio::test]
async fn mika2872_la_recidive_de_fausse_localite_part_en_p0_sur_la_surface_jamais_muette()
-> anyhow::Result<()> {
    let sender = SeverityRecordingSender::new();

    // `.deployment(…)` n'est pas appelé : le défaut du harness est
    // `Deployment::Unknown`, qui n'est **pas** exempté par le détecteur — c'est
    // l'état de tout tenant cloud aujourd'hui, et l'état de l'incident mesuré
    // du 2026-09-11. Seul `Local` est exempt, et un `Local` ici rendrait la
    // branche inatteignable.
    let harness = EvalHarness::builder()
        .responses(recidive_de_localite())
        .message_sender(sender.clone())
        .fabrication_p0_notify(true)
        .build()
        .await?;

    let trace = harness.run(QUESTION_DU_TENANT).await?;

    // AC6(a) — la garde a bien tiré une première fois : sans re-prompt,
    // `intent_guard_retries` ne contient pas le label et la branche
    // `_uncorrected` est structurellement inatteignable. Le test n'attesterait
    // alors rien, et le dirait vert.
    assert!(
        trace.llm_call_count > 1,
        "la garde false_local_hosting_claim n'a pas tiré (llm_call_count = {}) : \
         la branche guard.false_local_hosting_claim_uncorrected est restée hors \
         d'atteinte, le test n'atteste rien",
        trace.llm_call_count
    );

    let emissions = sender.emissions();
    let fabrications: Vec<&Emission> = emissions
        .iter()
        .filter(|(text, _)| text.contains(NOTIFICATION_MARKER))
        .collect();

    // AC6(d) — exactement une notification. Zéro signalerait un site retiré ;
    // deux, un verrou de classe (`pending_fabrication_p0.is_none()`) cassé. La
    // première prise ne peut pas en produire une : 5d fait `continue` avant la
    // branche.
    assert_eq!(
        fabrications.len(),
        1,
        "attendu exactement une notification de fausse localité au transport, \
         observé {} ; émissions = {:?}",
        fabrications.len(),
        emissions
    );

    let (texte, etage) = fabrications[0];

    // AC6(e) — l'étage n'est pas absent : le site est passé par
    // `send_with_severity`, pas par le corps par défaut qui délègue à `send`.
    assert_ne!(
        *etage, SEVERITE_ABSENTE,
        "la notification de fausse localité est arrivée au transport SANS \
         étage : le site est passé par `MessageSender::send`, dont le corps par \
         défaut de `send_with_severity` perd la sévérité en silence. La P0 \
         n'atteint pas la surface jamais muette. Texte : {texte:?}"
    );

    // AC6(f) — et cet étage est P0, pas un autre.
    assert_eq!(
        *etage,
        Some(NotificationSeverity::P0),
        "la fausse localité non corrigée doit partir en P0 (surface jamais \
         muette), observé {etage:?} ; texte : {texte:?}"
    );

    // AC8 — la notification porte de quoi agir : ce qui a été affirmé, la
    // vérité de terrain qui le contredit, et le `trace_id` du tour. Sans ces
    // trois-là l'opérateur reçoit une alarme qu'il ne peut pas suivre jusqu'au
    // log.
    assert!(
        texte.contains("en local"),
        "la notification ne nomme pas le prédicat de localité affirmé ; \
         texte : {texte:?}"
    );
    assert!(
        texte.contains("Unknown"),
        "la notification ne nomme pas le déploiement réel qui contredit la \
         revendication ; texte : {texte:?}"
    );
    assert!(
        texte.contains(&harness.trace_id),
        "la notification ne nomme pas le trace_id du tour ({}) ; texte : {texte:?}",
        harness.trace_id
    );

    Ok(())
}

// ---------------------------------------------------------------------------
// AC7 — le contrôle à porte fermée
// ---------------------------------------------------------------------------

/// Porte fermée (le défaut livré) : la même récidive, et le transport
/// n'observe **rien**.
///
/// Sans ce contrôle, un site qui ignorerait `MIKA_FABRICATION_P0_NOTIFY`
/// passerait le test positif — et la décision « livrer désarmé » ne serait
/// qu'une intention documentée. Ce qui est attesté ici, c'est que la porte est
/// une porte.
#[tokio::test]
async fn mika2872_porte_fermee_le_transport_nobserve_rien() -> anyhow::Result<()> {
    let sender = SeverityRecordingSender::new();

    // `.fabrication_p0_notify(…)` n'est pas appelé : le défaut du harness est le
    // défaut livré (`false`).
    let harness = EvalHarness::builder()
        .responses(recidive_de_localite())
        .message_sender(sender.clone())
        .build()
        .await?;

    let trace = harness.run(QUESTION_DU_TENANT).await?;

    // Le contrôle positif du contrôle négatif : la récidive a bien eu lieu.
    // Sans cette ligne, « zéro émission » serait indistinguable d'un scénario
    // qui n'a jamais atteint la branche.
    assert!(
        trace.llm_call_count > 1,
        "la garde false_local_hosting_claim n'a pas tiré (llm_call_count = {}) : \
         le scénario n'a pas atteint la branche, donc l'absence d'émission ne \
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
         le transport a pourtant observé {} notification(s) de fausse localité : \
         le site ignore sa porte. Émissions = {:?}",
        fabrications.len(),
        emissions
    );

    Ok(())
}

// ---------------------------------------------------------------------------
// AC9 — une garde aval reprend la main après la récidive : rien ne part
// ---------------------------------------------------------------------------

/// La récidive est constatée, puis une garde **en aval** de 5d (6f, l'envoi
/// mort non reconnu) refuse encore l'`EndTurn`, et la réponse finale ne
/// revendique plus la localité. Aucune P0 ne doit partir : la réponse qui a
/// récidivé n'est jamais sortie, et la notification dirait le contraire
/// (« The answer went out »).
///
/// **C'est ce test qui pince le refactor du créneau typé.** Le fixture ne
/// porte aucune forme d'`detect_affirmative_state_claim`, donc le prédicat
/// câblé en dur de mika#2867 rendrait `None` sur le texte accepté **comme sur
/// le texte fautif** : avec lui, ce test passerait — mais le test positif
/// ci-dessus, lui, rougirait, parce que la P0 déposée par 5d y serait
/// supprimée par le prédicat de l'autre site. Les deux ensemble séparent
/// « revérifié » de « revérifié avec le bon prédicat ».
#[tokio::test]
async fn mika2872_une_garde_aval_qui_reprend_la_main_annule_la_p0() -> anyhow::Result<()> {
    let sender = SeverityRecordingSender::new();
    let document = "D".repeat(12_000);

    let harness = EvalHarness::builder()
        .responses(vec![
            // Pas 1 : un envoi refusé par la garde de longueur (mika#2134). Il
            // arme la garde 6f pour la suite du tour.
            tool_call_response("send_message", serde_json::json!({ "text": document })),
            // Pas 2 : la fausse localité. La garde 5d tire.
            text_response(FAUSSE_LOCALITE),
            // Pas 3 : la récidive. La branche la constate et dépose la P0, puis
            // 6f refuse la clôture sur l'envoi mort.
            text_response(FAUSSE_LOCALITE),
            // Pas 4 : la clôture acceptée, sans revendication de localité.
            text_response("Je n'ai pas pu te livrer le document, il était trop long à envoyer."),
        ])
        .message_sender(sender.clone())
        .fabrication_p0_notify(true)
        .build()
        .await?;

    let trace = harness.run(QUESTION_DU_TENANT).await?;

    // Le scénario a bien atteint la forme visée : 5d, puis 6f après la
    // récidive, puis la clôture. Moins de quatre appels voudrait dire que la
    // branche ou 6f n'a pas été atteinte, et le zéro ci-dessous ne prouverait
    // rien.
    assert_eq!(
        trace.llm_call_count, 4,
        "attendu 4 appels (outil, 5d, récidive puis 6f, clôture), observé {}",
        trace.llm_call_count
    );

    let emissions = sender.emissions();
    let fabrications: Vec<&Emission> = emissions
        .iter()
        .filter(|(text, _)| text.contains(NOTIFICATION_MARKER))
        .collect();
    assert!(
        fabrications.is_empty(),
        "la réponse qui a récidivé n'est jamais sortie (6f a repris la main), et \
         une P0 est pourtant partie : {fabrications:?}"
    );

    Ok(())
}
