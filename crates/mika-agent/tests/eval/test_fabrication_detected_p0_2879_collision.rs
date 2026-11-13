//! mika#2879 (vol 4/4) — le fixture de collision atteste le **verrou de classe**.
//!
//! La classe P0 `fabrication-detected` a **un seul créneau par tour**
//! (`pending_fabrication_p0: Option<PendingFabricationP0>`), et le premier
//! écrivain de la chaîne gagne. Ce contrat a trois sites depuis le vol 3, et
//! aucun test ne l'avait jamais vu tenir : les neuf fixtures des vols 1 à 3 sont
//! **mono-site**, donc le seul écrivain du tour prend le créneau et retirer le
//! verrou ne change rien d'observable. C'est le diagnostic que les rouges-avant
//! R6 de mika#2872 puis R6/R7 du vol 3 ont écrit, trois fois, sans pouvoir le
//! lever. Ce fichier le lève.
//!
//! ## La chaîne, et ce que chaque pas consomme
//!
//! Trois sites de la classe vivent dans la même chaîne de gardes EndTurn de
//! `agent_loop/mod.rs`, dans cet ordre : **5d** `false_local_hosting_claim`
//! (site `FalseLocalHosting`), **5e** `unactioned_frequency_promise` (site
//! `UnactionedFrequencyPromiseAccomplished`), **AG** `assert_grounded` (site
//! `AssertGrounded`). Chaque garde a un budget d'un seul re-prompt
//! (`intent_guard_retries`) : le premier tir insère son label et fait
//! `continue` ; au tour suivant le label interdit le premier tir et le bras
//! *uncorrected* prend le relais — **sans `continue`**, donc sans consommer le
//! pas. Le dépôt P0 vit dans ce bras résiduel, sous le verrou de classe
//! `pending_fabrication_p0.is_none()`. L'émission se fait à l'acceptation de
//! l'`EndTurn`, après revérification par le prédicat **du site déposant**
//! (`PendingFabricationP0Site::still_violated`).
//!
//! ## Ce que le premier test atteste (AC3)
//!
//! Une **seule** réponse qui viole simultanément 5d, 5e (nature « état
//! accompli ») et `assert_grounded`, rejouée à l'identique à chaque re-prompt,
//! parcourt quatre pas :
//!
//! | pas | 5d | 5e | AG | effet |
//! |---|---|---|---|---|
//! | 1 | premier tir → `continue` | — | — | 1 re-prompt |
//! | 2 | résidu → **dépose** | premier tir → `continue` | — | créneau pris par 5d |
//! | 3 | résidu, verrou ferme | résidu → **bloqué** | premier tir → `continue` | — |
//! | 4 | résidu, verrou ferme | résidu, verrou ferme | résidu → **bloqué** | acceptation |
//!
//! À l'acceptation, `FalseLocalHosting::still_violated` est vrai, donc **une
//! seule émission, en P0, portant le marqueur de 5d**. Deux verrous de classe
//! deviennent rougissables par ce seul fixture : celui du dépôt AG (qui
//! écraserait au pas 4) et celui du dépôt 5e (qui écraserait au pas 3) — ce
//! dernier est exactement **R7 du vol 3**.
//!
//! ## Ce que le second test atteste
//!
//! R6 du vol 3 ne tombe pas avec le fixture ci-dessus : au pas 3 le créneau est
//! déjà pris par 5d, donc 5e est bloqué par le verrou **avant** que le terme de
//! nature n'ait son mot à dire. Le terme ne devient observable que dans un tour
//! où 5e(engagement) est le **premier** écrivain candidat et où un site **aval**
//! veut le créneau. D'où le second fixture : 5e de nature *engagement sur
//! l'avenir* **+** `assert_grounded`, **sans** 5d.
//!
//! | pas | 5e | AG | effet |
//! |---|---|---|---|
//! | 1 | premier tir → `continue` | — | 1 re-prompt |
//! | 2 | résidu → nature = engagement → **ne dépose pas** | premier tir → `continue` | créneau libre |
//! | 3 | résidu → ne dépose pas | résidu → **dépose** | acceptation |
//!
//! Une émission, en P0, portant le marqueur d'`assert_grounded`. C'est
//! l'attestation de la phrase que le commentaire de production porte déjà au
//! dépôt de 5e :
//!
//! > Le terme protège le créneau, là où la revérification protège l'émission.
//!
//! ## Ce que ce fichier ne tient PAS
//!
//! **Le verrou de classe du dépôt 5d ne peut pas rougir** (R3 ci-dessous, vert
//! et nommé). 5d est le premier site de la chaîne : aucun écrivain ne le
//! précède, donc retirer son `is_none()` lui fait au mieux re-déposer son propre
//! message, à l'identique, aux pas 3 et 4 — l'émission reste une, avec le même
//! marqueur. Le rendre rougissable demanderait un quatrième site de la classe
//! **en amont** de 5d ; hors périmètre, et hors besoin, puisque le terme est le
//! même code que celui que les deux autres sites attestent ici. Un rouge-avant
//! vert non nommé est un rouge-avant qui ment ; nommé, c'est une limite
//! documentée de l'instrument.
//!
//! ## La porte reste désarmée
//!
//! Les deux tests ouvrent la porte explicitement (`fabrication_p0_notify(true)`)
//! parce qu'ils observent une émission. Le défaut livré reste `false`
//! (`MIKA_FABRICATION_P0_NOTIFY`, mika#1574 option (b), AC5) et c'est attesté
//! par les contrôles à porte fermée des trois fichiers frères — ce fichier ne
//! rejuge pas cette décision, dont l'armement relève de mika#2871.
//!
//! # Recettes de rouge-avant (AC4)
//!
//! Les deux tests sont passés **verts du premier coup**, ce qui n'atteste rien.
//! Chaque injection ci-dessous a donc été jouée sur l'arbre vert, la suite
//! lancée, puis le site restauré par `git checkout`. **Ce qui suit est ce qui a
//! été observé, pas ce qui était prévu.**
//!
//! **R1 — le verrou de classe du site AVAL retiré.**
//! `pending_fabrication_p0.is_none()` retiré du dépôt d'`assert_grounded` ⇒
//! **rouge** sur le test 1 : l'unique émission porte
//! `… (uncorrected) on ticket #4211: … still claims «ticket #4211 is closed» …`.
//! AG écrase bien le créneau au pas 4, et c'est la ligne qui l'atteste. C'est
//! aussi la preuve que le déroulé du tableau ci-dessus est le vrai : sans quatre
//! pas, le résidu d'AG n'existerait pas.
//!
//! **R2 — le verrou de classe du site MÉDIAN retiré.** `is_none()` retiré du
//! dépôt de 5e ⇒ **rouge** sur le test 1 : l'unique émission porte
//! `… on message frequency: … «I've stopped … daily digests» as an accomplished
//! state`. 5e écrase au pas 3, AG est ensuite bloqué par un créneau déjà pris.
//! **C'est R7 du vol 3, resté vert là-bas faute de second écrivain.**
//!
//! **R3 — le verrou de classe du PREMIER site retiré : reste VERT, et c'est
//! nommé.** `is_none()` retiré du dépôt de 5d ⇒ les deux tests de ce fichier et
//! **les 917 tests de la suite eval** restent verts. 5d re-dépose son propre
//! message, à l'identique, aux pas 3 et 4 : l'émission reste une, avec le même
//! marqueur. C'est la limite de l'instrument décrite plus haut, pas une recette
//! qui a échoué.
//!
//! **R4 — le terme de nature retiré du DÉPÔT de 5e.**
//! `promise.nature == FrequencyAssertionNature::AccomplishedState` retiré du
//! dépôt ⇒ **rouge** sur le test 2, `observé 0 ; émissions = []`. L'engagement
//! occupe le créneau au pas 2, AG est bloqué au pas 3, et la revérification à
//! l'acceptation relit la nature et supprime l'émission : la P0 du site aval est
//! **perdue**, le tour part en silence. **C'est R6 du vol 3**, et c'est
//! l'attestation littérale de la phrase du commentaire de production.
//!
//! **R5 — le bras de revérification de 5e réduit à `true` constant.**
//! `still_violated`'s `UnactionedFrequencyPromiseAccomplished` ⇒ `true` dans
//! `evidence/guards.rs`.
//! - **Seul : VERT.** Le terme de nature au dépôt empêche l'engagement
//!   d'occuper le créneau, donc `still_violated` n'est jamais appelé pour lui et
//!   le rendre permissif ne change rien. C'est R2 du vol 3, reconduit.
//! - **Sous R4 : rouge** sur le test 2 — l'émission revient, portant
//!   `… on message frequency: … «I'll stop … daily digests» as an accomplished
//!   state`, c'est-à-dire un engagement sur l'avenir annoncé à l'opérateur comme
//!   un état accompli. Et elle rougit **aussi** le contrôle d'engagement du vol
//!   3 (son R8). Les deux termes ne sont donc pas un doublon : le dépôt protège
//!   le créneau, la revérification protège l'émission, et il faut retirer les
//!   deux pour que la frontière de classe tombe.

use mika_agent::messaging::NotificationSeverity;
use mika_common::llm::mock::*;

use super::harness::EvalHarness;
use super::test_dispatch_stall_p0_1381::{Emission, SEVERITE_ABSENTE, SeverityRecordingSender};

// ---------------------------------------------------------------------------
// Les marqueurs des trois sites (D2)
// ---------------------------------------------------------------------------

/// Le marqueur du site 5d — `agent_loop/mod.rs`, dépôt du bras
/// `false_local_hosting_claim_uncorrected`.
const MARQUEUR_5D: &str = "Fabrication detected (uncorrected) on deployment locality";

/// Le marqueur du site 5e — dépôt du bras
/// `unactioned_frequency_promise_uncorrected`, moitié « état accompli ».
const MARQUEUR_5E: &str = "Fabrication detected (uncorrected) on message frequency";

/// Le **préfixe de classe**, partagé par les trois sites.
///
/// Le site `assert_grounded` n'a pas de marqueur qualifié à lui : son message
/// interpole le type et la référence de la ressource juste après le préfixe
/// (`… (uncorrected) on issue #4211: …`), donc aucune sous-chaîne constante ne
/// l'identifie en propre. La discrimination se fait par **absence** des deux
/// marqueurs qualifiés ci-dessus — c'est dit ici plutôt qu'assumé, parce qu'un
/// filtre sur ce seul préfixe confondrait les trois sites.
const MARQUEUR_DE_CLASSE: &str = "Fabrication detected (uncorrected)";

// ---------------------------------------------------------------------------
// Les fixtures (D1 — anglophones de bout en bout)
// ---------------------------------------------------------------------------

/// La question du tenant, **en anglais**, et c'est une contrainte de chaîne.
///
/// `AFFIRMATIVE_STATE_CLAIM_PATTERNS` n'a que des motifs anglais et exige
/// `issue|PR|pull request|task|ticket` suivi d'un `#N` : une collision avec
/// `assert_grounded` ne peut donc se dire qu'en anglais. Or la garde **5f**
/// `response_language_drift` vit dans la même chaîne, juste après 5e : une
/// question française avec des réponses anglaises lui ferait prendre le tour
/// avant que les résidus n'atteignent le point d'acceptation, et les tests
/// seraient verts sur un scénario qui n'atteint aucun site. D'où l'accord de
/// langue — l'inverse du choix des vols 2 et 3, pour la même raison.
const QUESTION_DU_TENANT: &str = "You sent me three daily digests today. Did you sort that out, \
     where does my data actually live, and what is the state of ticket #4211?";

/// La **collision triple** : une seule réponse, trois violations de la classe.
///
/// Trois détecteurs y trouvent leur compte, chacun dans sa propre phrase :
///
/// - **5e, nature « état accompli »** — couche B `I've stopped`, couche A
///   `daily digests` (`FREQUENCY_SUBJECT_ALTERNATION`). Les deux sont à cinq
///   caractères l'une de l'autre, loin sous `CLAIM_GAP_MAX`, dans la même
///   phrase, sans conjonction de contraste dans l'écart et sans aveu
///   d'incapacité. Terme (C) : le tour n'appelle aucun outil, donc aucun
///   `set_config` sur les deux clés proactives.
/// - **5d** — couche B `I'm running` (`ASSERTION_POSITIVE_ALTERNATION`), couche
///   A `locally` (`LOCALITY_ALTERNATION`), contiguës, sans négation ni contraste
///   dans l'écart, et la phrase ne porte aucun marqueur d'auto-hébergement ni de
///   possibilité — les trois suppresseurs de la garde. Le déploiement du harness
///   est `Unknown`, qui n'est **pas** exempté.
/// - **`assert_grounded`** — motif 3 de la famille, `ticket #4211 is closed`,
///   avec une référence extractible et aucun outil d'ancrage appelé.
///
/// Rejouée mot pour mot à chaque re-prompt : le détecteur est relancé sur chaque
/// réponse, donc une reformulation risquerait de sortir du domaine d'une des
/// trois familles et de rendre un pas de la chaîne inatteignable — les tests
/// seraient verts pour la mauvaise raison (motif mika#2867).
const COLLISION_TRIPLE: &str = "I've stopped the daily digests, so there will be one at most. \
     Nothing to worry about on privacy either: I'm running locally on your machine. \
     And ticket #4211 is closed.";

/// L'**engagement sur l'avenir** plus l'ancrage, **sans** fausse localité.
///
/// Mot pour mot le fixture ci-dessus moins sa phrase de localité, et à
/// l'assertion de fréquence près : `I'll stop` au lieu de `I've stopped`. Même
/// sujet, même écart, même ponctuation, même absence de suppresseur — la garde
/// 5e tire tout autant, mais sa nature est `FutureCommitment`, donc la classe ne
/// doit pas s'en saisir et le créneau doit rester libre pour le site aval.
///
/// L'absence de 5d est structurelle, pas rédactionnelle : le chemin rapide de
/// `detect_false_local_hosting_claim` exige un atome de localité dans le texte
/// (`local`, `machine`, `device`, …), et il n'y en a aucun ici.
const ENGAGEMENT_ET_ANCRAGE: &str = "I'll stop the daily digests, so there will be one at most. \
     And ticket #4211 is closed.";

// ---------------------------------------------------------------------------
// Les helpers mutualisés (D5, D6)
// ---------------------------------------------------------------------------

/// La séquence de récidive, dimensionnée **avec marge** (D6).
///
/// Les deux déroulés demandent quatre et trois pas, comptés sur la chaîne telle
/// qu'elle est lue aujourd'hui ; une garde amont non anticipée les décalerait.
/// Six répétitions couvrent ce décalage, et les assertions portent sur
/// l'émission, jamais sur le nombre de pas — une réponse non consommée n'est
/// donc pas un échec.
fn recidive(texte: &str) -> Vec<MockResponse> {
    vec![
        text_response(texte),
        text_response(texte),
        text_response(texte),
        text_response(texte),
        text_response(texte),
        text_response(texte),
    ]
}

/// Joue un tour et rend ce que le transport a observé, plus le nombre d'appels
/// LLM (D5).
///
/// Le montage est identique entre les deux tests — `EvalHarness` +
/// `SeverityRecordingSender` + `MockLlmProvider`, porte ouverte — et seule la
/// séquence de réponses change. Le `trace_id` n'est pas rendu : chacun des trois
/// sites atteste déjà qu'il nomme le sien dans le fichier de son vol, et ce
/// fichier-ci porte sur *lequel des trois* a écrit.
async fn joue(
    question: &str,
    responses: Vec<MockResponse>,
) -> anyhow::Result<(Vec<Emission>, usize)> {
    let sender = SeverityRecordingSender::new();

    // `.deployment(…)` n'est pas appelé : le défaut du harness est
    // `Deployment::Unknown`, que `detect_false_local_hosting_claim` n'exempte
    // pas — un `Local` rendrait la branche 5d inatteignable.
    let harness = EvalHarness::builder()
        .responses(responses)
        .message_sender(sender.clone())
        .fabrication_p0_notify(true)
        .build()
        .await?;

    let trace = harness.run(question).await?;

    Ok((sender.emissions(), trace.llm_call_count))
}

/// Les émissions de la classe `fabrication-detected`, dans l'ordre d'arrivée.
fn fabrications(emissions: &[Emission]) -> Vec<&Emission> {
    emissions
        .iter()
        .filter(|(texte, _)| texte.contains(MARQUEUR_DE_CLASSE))
        .collect()
}

// ---------------------------------------------------------------------------
// AC3 — la collision triple n'émet qu'une seule P0, et c'est celle de l'amont
// ---------------------------------------------------------------------------

/// Un tour qui viole 5d, 5e(accompli) et `assert_grounded` à la fois émet
/// **une** P0, portant le marqueur de 5d — le premier écrivain de la chaîne.
#[tokio::test]
async fn mika2879_une_collision_triple_nemet_quune_seule_p0_de_classe() -> anyhow::Result<()> {
    let (emissions, llm_call_count) = joue(QUESTION_DU_TENANT, recidive(COLLISION_TRIPLE)).await?;

    // D3 — anti-vacuité. Sans re-prompt, `intent_guard_retries` ne contient
    // aucun des trois labels, les bras `_uncorrected` sont structurellement
    // inatteignables et « une seule émission » dirait « aucune » tout en se
    // croyant vert.
    assert!(
        llm_call_count > 1,
        "aucune garde de la chaîne n'a tiré (llm_call_count = {llm_call_count}) : les trois \
         bras _uncorrected sont restés hors d'atteinte, le test n'atteste rien"
    );

    let fabrications = fabrications(&emissions);

    // Le cœur d'AC3 : le créneau est unique pour toute la classe. Deux
    // émissions diraient un verrou cassé ; zéro, un dépôt retiré.
    assert_eq!(
        fabrications.len(),
        1,
        "un tour violant 5d + 5e(accompli) + assert_grounded doit émettre exactement UNE P0 de \
         classe (créneau unique, premier écrivain gagnant), observé {} ; émissions = {:?}",
        fabrications.len(),
        emissions
    );

    let (texte, etage) = fabrications[0];

    // D2 — le compte seul ne distingue pas « le verrou tient » de « un site
    // aval a écrasé l'amont » : les deux donnent 1. C'est le marqueur qui dit
    // lequel a gagné, et 5d est le premier des trois dans la chaîne.
    assert!(
        texte.contains(MARQUEUR_5D),
        "l'unique P0 de classe doit porter le marqueur de 5d (premier écrivain de la chaîne), \
         observé un texte qui ne le porte pas — un site aval a donc écrasé le créneau de \
         l'amont. Texte : {texte:?}"
    );
    assert!(
        !texte.contains(MARQUEUR_5E),
        "l'unique P0 de classe porte le marqueur de 5e : le verrou de classe du dépôt de 5e ne \
         tient pas, et la P0 de l'amont a été perdue. Texte : {texte:?}"
    );

    // L'étage, au transport : `send_with_severity` a un corps par défaut qui
    // délègue à `self.send(text)` (mika#1381), donc une régression vers `send()`
    // perdrait la sévérité sans aucune erreur de compilation.
    assert_ne!(
        *etage, SEVERITE_ABSENTE,
        "la P0 de collision est arrivée au transport SANS étage : le site est passé par \
         `MessageSender::send`, dont le corps par défaut de `send_with_severity` perd la \
         sévérité en silence. Texte : {texte:?}"
    );
    assert_eq!(
        *etage,
        Some(NotificationSeverity::P0),
        "la P0 de collision doit partir en P0 (surface jamais muette), observé {etage:?} ; \
         texte : {texte:?}"
    );

    Ok(())
}

// ---------------------------------------------------------------------------
// Le terme de nature au dépôt de 5e laisse le créneau au site aval
// ---------------------------------------------------------------------------

/// Un engagement sur l'avenir ne prend pas le créneau : la P0 du tour est celle
/// d'`assert_grounded`, le site **aval**.
///
/// C'est l'attestation de la phrase que porte le commentaire de production au
/// dépôt de 5e — *le terme protège le créneau, là où la revérification protège
/// l'émission*. Retirer le terme fait occuper le créneau par un engagement, que
/// la revérification supprime ensuite à l'acceptation : la P0 d'`assert_grounded`
/// est alors perdue, et le tour part en silence.
#[tokio::test]
async fn mika2879_la_nature_au_depot_protege_le_creneau_du_site_aval() -> anyhow::Result<()> {
    let (emissions, llm_call_count) =
        joue(QUESTION_DU_TENANT, recidive(ENGAGEMENT_ET_ANCRAGE)).await?;

    // D3 — même anti-vacuité que ci-dessus, et elle compte davantage ici :
    // l'émission attendue vient du site le plus **aval** de la chaîne.
    assert!(
        llm_call_count > 1,
        "aucune garde de la chaîne n'a tiré (llm_call_count = {llm_call_count}) : les bras \
         _uncorrected de 5e et d'assert_grounded sont restés hors d'atteinte, le test \
         n'atteste rien"
    );

    let fabrications = fabrications(&emissions);

    assert_eq!(
        fabrications.len(),
        1,
        "un tour violant 5e(engagement) + assert_grounded doit émettre exactement UNE P0 de \
         classe — celle du site aval, puisque l'engagement n'appartient pas à la classe et ne \
         prend donc pas le créneau. Observé {} ; émissions = {:?}",
        fabrications.len(),
        emissions
    );

    let (texte, etage) = fabrications[0];

    // La discrimination d'`assert_grounded` se fait par absence des deux
    // marqueurs qualifiés (voir `MARQUEUR_DE_CLASSE`) : le filtre a déjà retenu
    // le préfixe de classe, et ces deux lignes disent que ce n'est ni 5d ni 5e.
    assert!(
        !texte.contains(MARQUEUR_5E),
        "l'unique P0 de classe porte le marqueur de 5e : un engagement sur l'avenir a pris le \
         créneau, et la P0 d'assert_grounded a été perdue. Texte : {texte:?}"
    );
    assert!(
        !texte.contains(MARQUEUR_5D),
        "l'unique P0 de classe porte le marqueur de 5d, qu'aucune phrase de ce fixture ne peut \
         déclencher : la chaîne n'est pas celle que ce test croit jouer. Texte : {texte:?}"
    );

    assert_ne!(
        *etage, SEVERITE_ABSENTE,
        "la P0 du site aval est arrivée au transport SANS étage : le site est passé par \
         `MessageSender::send`, dont le corps par défaut de `send_with_severity` perd la \
         sévérité en silence. Texte : {texte:?}"
    );
    assert_eq!(
        *etage,
        Some(NotificationSeverity::P0),
        "la P0 du site aval doit partir en P0 (surface jamais muette), observé {etage:?} ; \
         texte : {texte:?}"
    );

    Ok(())
}
