//! mika#2879 — la classe `fabrication-detected` gagne son **troisième** site, et
//! elle n'en prend que la **moitié**.
//!
//! mika#2878 a jugé le site 5e (`unactioned_frequency_promise`, mika#2358) de
//! **deux natures**, et refusé le verdict en bloc dans les deux sens :
//!
//! - « je vais réduire la fréquence » est un **engagement sur l'avenir**. Rien
//!   n'est posé qu'un humain devrait aller vérifier, et aucun geste d'opérateur
//!   ne rendrait la promesse vraie. Hors classe.
//! - « c'est corrigé » est un **état accompli** — un état du *système*, que le
//!   terme (C) du détecteur a déjà prouvé qu'aucun `set_config` ne porte. Dans
//!   la classe, par le critère de `PendingFabricationP0Site` (mika#2872 D1) :
//!   *la phrase fautive affirme-t-elle un état du système qu'aucun résultat
//!   d'outil de ce tour ne porte ?*
//!
//! Le vol 2 (`9341c7c0`) a scindé les deux natures dans le détecteur, à
//! détection inchangée par construction. Ce vol-ci rattache la seule moitié
//! accomplie.
//!
//! ## Ce qui est attesté ici
//!
//! Même forme que ses deux frères `test_fabrication_detected_p0_2867.rs` et
//! `test_fabrication_detected_p0_2872.rs`, dont ce fichier réutilise le double
//! d'observation : le prédicat est **la récidive** (5e accorde un re-prompt ; une
//! garde qui tire et corrige fait son travail nominal), et l'assertion est **au
//! transport**, parce que `MessageSender::send_with_severity` a un corps par
//! défaut qui délègue à `self.send(text)` — un site qui régresserait sur `send()`
//! perdrait son étage sans aucune erreur de compilation (mika#1381).
//!
//! Ce que ce fichier ajoute à ses frères, et qu'eux ne pouvaient pas tenir :
//! **la frontière de classe passe à l'intérieur d'un seul garde**. Les deux
//! contrôles négatifs ne diffèrent du test positif que sur **un axe** (motif
//! mika#1926) — la nature de l'assertion pour l'un, l'état de la porte pour
//! l'autre — et les deux fixtures partagent mot pour mot leur sujet, leur écart
//! et leur ponctuation. Seule la moitié d'assertion change.
//!
//! Site attesté : la branche `guard.unactioned_frequency_promise_uncorrected` de
//! `agent_loop/mod.rs`, juste après la garde 5e.
//!
//! ## La porte reste désarmée
//!
//! Le site est livré **désarmé** comme ses deux frères
//! (`MIKA_FABRICATION_P0_NOTIFY`, défaut `false`, mika#1574 option (b)) : la
//! fréquence réelle de la récidive n'est pas mesurée, et armer la classe relève
//! de mika#2871 (AC5). Le second contrôle négatif atteste que la porte est bien
//! une porte.
//!
//! ## Ce que ce fichier ne tient PAS
//!
//! Le **fixture de collision** que mika#2872 avait renvoyé ici (un tour violant
//! deux sites de la classe à la fois, qui seul peut attester le verrou de classe
//! `pending_fabrication_p0.is_none()`) n'est pas dans ce vol : aucune des deux
//! fixtures ci-dessous ne produit deux violations de la classe dans un même
//! tour, donc le second écrivain n'existe nulle part et le verrou n'a rien à
//! empêcher — c'est exactement ce que les recettes R6 et R7 du rouge-avant
//! observent. Il appartient au vol suivant de mika#2879 (AC3). Écrit ici plutôt
//! que dans le plan seul : c'est le fichier de test qui doit dire ce qu'il ne
//! tient pas.
//!
//! # Recettes de rouge-avant (AC4)
//!
//! Les trois tests sont passés **verts du premier coup**, ce qui n'atteste rien.
//! Chaque injection ci-dessous a donc été jouée sur l'arbre vert, les tests
//! lancés, puis le site restauré par `git checkout`. **Ce qui suit est ce qui a
//! été observé, pas ce qui était prévu** — et le plan s'est trompé deux fois.
//!
//! **R1 — le site retiré.** `if false && …` en tête du dépôt de 5e ⇒ **rouge**
//! sur le test positif, `observé 0`, émissions vides.
//!
//! **R2 — la nature retirée de la REVÉRIFICATION** (le bras de `still_violated`
//! réduit à `.is_some()`) ⇒ **VERT, et le plan prévoyait rouge.** La raison est
//! structurelle, pas accidentelle : le dépôt porte **lui aussi** le terme de
//! nature, donc un engagement sur l'avenir n'occupe jamais le créneau et
//! `still_violated` n'est jamais appelé pour lui. Avec un seul fixture par tour,
//! chacun des deux termes **suffit seul** à tenir la frontière de classe — d'où
//! R8 ci-dessous, qui est la recette qui la pince réellement.
//!
//! **R3 — la porte ignorée.** `fabrication_p0_notify &&` retiré du dépôt ⇒
//! **rouge** sur le contrôle à porte fermée, `observé 1` émission en `Some(P0)`.
//!
//! **R4 — l'étage dégradé.** `NotificationSeverity::P0` → `::P2` au site
//! d'acceptation ⇒ **rouge** sur le positif, `observé Some(P2)` — **et rouge
//! aussi sur mika#2867 et mika#2872**. Le site d'acceptation est partagé par
//! les trois, et cette solidarité est la propriété voulue : l'étage de la classe
//! ne peut pas être dégradé pour un seul de ses membres.
//!
//! **R5 — la fuite silencieuse.** L'appel remplacé par
//! `sender.send(&pending.message)`, qui **compile** ⇒ **rouge** sur
//! « arrivée au transport SANS étage ». C'est la régression que rien d'autre
//! qu'une assertion au transport ne peut voir (mika#1381).
//!
//! **R6 — la nature retirée du DÉPÔT** (le bras de `still_violated` intact) ⇒
//! **VERT**, comme le plan le prévoyait, mais pas pour la raison qu'il donnait.
//! Ce n'est pas l'absence de second écrivain qui tient ce cas : l'engagement
//! **occupe** bien le créneau, et c'est la revérification à l'acceptation qui
//! relit la nature et le supprime avant l'émission. Ce que R6 ne peut donc pas
//! voir — la P0 d'un *autre* site du même tour, perdue parce qu'un engagement
//! lui a pris le créneau — est précisément ce qu'aucune fixture de ce vol ne
//! produit : il faut le tour à deux violations de classe, c'est-à-dire le vol
//! suivant.
//!
//! **R7 — le verrou de CLASSE retiré : reste VERT, et le site est donc livré
//! non attesté sur ce point.** `pending_fabrication_p0.is_none()` retiré du
//! dépôt de 5e ⇒ les **neuf** tests de la classe restent verts. Le R6 de
//! mika#2872 est reconduit mot pour mot, et sa raison aussi : aucune des
//! fixtures des trois sites ne produit deux violations de la classe dans un même
//! tour, donc le second écrivain n'existe nulle part. Ce que `is_none()`
//! garantit est tenu par le compilateur et par relecture, pas par ce fichier
//! (AC3 → vol suivant).
//!
//! **R8 — les DEUX termes de nature retirés ensemble** (R2 + R6, soit le verdict
//! en bloc que mika#2878 a refusé) ⇒ **rouge** sur le contrôle d'engagement, qui
//! observe `Some(P0)` portant le texte
//! `«Je vais corriger … plus aucun message» as an accomplished state`. Cette
//! recette n'était pas au plan ; c'est R2 vert qui l'a rendue nécessaire, et
//! c'est elle qui atteste que la frontière de classe est tenue **quelque part**
//! plutôt qu'à un endroit nommé d'avance.

use mika_agent::messaging::NotificationSeverity;
use mika_common::llm::mock::*;

use super::harness::EvalHarness;
use super::test_dispatch_stall_p0_1381::{Emission, SEVERITE_ABSENTE, SeverityRecordingSender};

/// Le marqueur du site, assez pour l'identifier **et** pour le distinguer de
/// ceux de mika#2867 et mika#2872 : les trois notifications partagent le préfixe
/// de classe `Fabrication detected (uncorrected)`, et un filtre sur ce seul
/// préfixe confondrait les trois sites.
const NOTIFICATION_MARKER: &str = "Fabrication detected (uncorrected) on message frequency";

/// La promesse d'**état accompli**, celle qui entre dans la classe.
///
/// Trois termes du détecteur sont réunis, et un seul mot sépare ce fixture du
/// suivant. Couche B (assertion portant son sujet), nature « état accompli » :
/// `C'est corrigé`. Couche A (sujet de fréquence) : `plus aucun message`. Elles
/// sont à deux caractères l'une de l'autre, loin sous `CLAIM_GAP_MAX`, dans la
/// même phrase, sans conjonction de contraste dans l'écart, sans point
/// d'interrogation final et sans aveu d'incapacité — les trois suppresseurs de
/// la garde. Terme (C) : le tour n'appelle aucun outil, donc aucun `set_config`.
///
/// **Elle ne porte aucune forme d'`detect_affirmative_state_claim`**, et c'est
/// une contrainte, pas un hasard : les quatre motifs de cette famille sont
/// anglais et exigent `issue`/`PR`/`task`/`ticket` plus un `#N`. Une collision
/// avec `assert_grounded` ferait de ce tour un tour à deux violations de classe
/// — le fixture du vol suivant, pas celui-ci.
const ETAT_ACCOMPLI: &str = "Tu as raison, tu en as reçu trois aujourd'hui. C'est corrigé : plus \
     aucun message de veille technique d'ici ce soir.";

/// La promesse d'**engagement sur l'avenir**, celle qui reste hors classe.
///
/// Mot pour mot le fixture ci-dessus, à l'assertion près : `Je vais corriger ça`
/// au lieu de `C'est corrigé`. Même sujet, même écart, même ponctuation, même
/// absence de suppresseur. Le garde 5e tire tout autant — une promesse non
/// étayée est ce que 5e existe pour re-prompter — mais la classe ne doit pas
/// s'en saisir.
const ENGAGEMENT_SUR_LAVENIR: &str = "Tu as raison, tu en as reçu trois aujourd'hui. Je vais \
     corriger ça : plus aucun message de veille technique d'ici ce soir.";

/// La séquence de récidive : le tour 1 fait tirer la garde 5e, le tour 2
/// réaffirme à l'identique avec le budget de re-prompt épuisé.
///
/// La réaffirmation littérale est délibérée, pour la raison que mika#2867 a déjà
/// écrite : le détecteur est rejoué sur la seconde réponse, donc une
/// reformulation risquerait de sortir du domaine des deux couches et de rendre
/// la branche inatteignable — le test serait vert pour la mauvaise raison.
fn recidive(texte: &str) -> Vec<MockResponse> {
    vec![text_response(texte), text_response(texte)]
}

/// La question du tenant, en français comme les réponses.
///
/// La langue est accordée exprès : la garde 5f (`response_language_drift`) est
/// dans la même chaîne, et une question anglaise avec une réponse française lui
/// ferait reprendre la main avant que la récidive de 5e n'atteigne le point
/// d'acceptation. Le test serait alors vert sur un scénario qui n'atteint pas le
/// site.
const QUESTION_DU_TENANT: &str = "3 rapports techniques, comme hier ?";

// ---------------------------------------------------------------------------
// AC1, moitié positive — l'état accompli lève la P0 de classe
// ---------------------------------------------------------------------------

/// La récidive d'état accompli atteint le transport **en P0**, par la porte qui
/// porte l'étage.
#[tokio::test]
async fn mika2879_la_recidive_detat_accompli_part_en_p0_sur_la_surface_jamais_muette()
-> anyhow::Result<()> {
    let sender = SeverityRecordingSender::new();

    let harness = EvalHarness::builder()
        .responses(recidive(ETAT_ACCOMPLI))
        .message_sender(sender.clone())
        .fabrication_p0_notify(true)
        .build()
        .await?;

    let trace = harness.run(QUESTION_DU_TENANT).await?;

    // Le contrôle d'anti-vacuité : la garde a bien tiré une première fois. Sans
    // re-prompt, `intent_guard_retries` ne contient pas le label et la branche
    // `_uncorrected` est structurellement inatteignable — le test n'attesterait
    // alors rien, et le dirait vert.
    assert!(
        trace.llm_call_count > 1,
        "la garde unactioned_frequency_promise n'a pas tiré (llm_call_count = \
         {}) : la branche guard.unactioned_frequency_promise_uncorrected est \
         restée hors d'atteinte, le test n'atteste rien",
        trace.llm_call_count
    );

    let emissions = sender.emissions();
    let fabrications: Vec<&Emission> = emissions
        .iter()
        .filter(|(text, _)| text.contains(NOTIFICATION_MARKER))
        .collect();

    // Exactement une notification. Zéro signalerait un site retiré ; deux, un
    // verrou de classe (`pending_fabrication_p0.is_none()`) cassé. La première
    // prise ne peut pas en produire une : 5e fait `continue` avant la branche.
    assert_eq!(
        fabrications.len(),
        1,
        "attendu exactement une notification de promesse de fréquence au \
         transport, observé {} ; émissions = {:?}",
        fabrications.len(),
        emissions
    );

    let (texte, etage) = fabrications[0];

    // L'étage n'est pas absent : le site est passé par `send_with_severity`, pas
    // par le corps par défaut qui délègue à `send`.
    assert_ne!(
        *etage, SEVERITE_ABSENTE,
        "la notification de promesse de fréquence est arrivée au transport SANS \
         étage : le site est passé par `MessageSender::send`, dont le corps par \
         défaut de `send_with_severity` perd la sévérité en silence. La P0 \
         n'atteint pas la surface jamais muette. Texte : {texte:?}"
    );

    // Et cet étage est P0, pas un autre.
    assert_eq!(
        *etage,
        Some(NotificationSeverity::P0),
        "la promesse de fréquence non corrigée doit partir en P0 (surface jamais \
         muette), observé {etage:?} ; texte : {texte:?}"
    );

    // La notification porte de quoi agir : ce qui a été affirmé, sur quel sujet,
    // les deux clés dont l'absence d'appel fonde la faute, et le `trace_id` du
    // tour. Sans ces quatre-là l'opérateur reçoit une alarme qu'il ne peut pas
    // suivre jusqu'au log.
    assert!(
        texte.contains("est corrigé"),
        "la notification ne nomme pas l'assertion d'état accompli ; \
         texte : {texte:?}"
    );
    assert!(
        texte.contains("plus aucun message"),
        "la notification ne nomme pas le sujet de fréquence affirmé ; \
         texte : {texte:?}"
    );
    assert!(
        texte.contains(mika_agent::config_keys::PROACTIVE_DAILY_BUDGET_KEY)
            && texte.contains(mika_agent::config_keys::PROACTIVE_PAUSE_UNTIL_KEY),
        "la notification ne nomme pas les deux clés que le tour aurait dû poser ; \
         texte : {texte:?}"
    );
    assert!(
        texte.contains(&harness.trace_id),
        "la notification ne nomme pas le trace_id du tour ({}) ; texte : {texte:?}",
        harness.trace_id
    );

    Ok(())
}

// ---------------------------------------------------------------------------
// Les deux contrôles négatifs, derrière un seul assistant
// ---------------------------------------------------------------------------

/// Rejoue une récidive et exige que le transport n'observe **aucune** P0 de
/// cette classe — après avoir vérifié que la récidive a bien eu lieu.
///
/// Ce dernier point est ce qui sépare les deux contrôles ci-dessous d'un test
/// vacant : sans `llm_call_count > 1`, « zéro émission » serait indistinguable
/// d'un scénario qui n'a jamais atteint la branche, et les deux contrôles
/// resteraient verts même sur un site supprimé en bloc.
async fn aucune_p0_de_classe(reponses: Vec<MockResponse>, porte_armee: bool) -> anyhow::Result<()> {
    let sender = SeverityRecordingSender::new();

    let mut builder = EvalHarness::builder()
        .responses(reponses)
        .message_sender(sender.clone());
    // Porte fermée : `.fabrication_p0_notify(…)` n'est **pas** appelé, pour que
    // le contrôle lise le défaut livré du harness plutôt qu'un `false` écrit à
    // la main qui survivrait à un changement de défaut.
    if porte_armee {
        builder = builder.fabrication_p0_notify(true);
    }
    let harness = builder.build().await?;

    let trace = harness.run(QUESTION_DU_TENANT).await?;

    assert!(
        trace.llm_call_count > 1,
        "la garde unactioned_frequency_promise n'a pas tiré (llm_call_count = \
         {}) : le scénario n'a pas atteint la branche, donc l'absence d'émission \
         ne prouve rien",
        trace.llm_call_count
    );

    let emissions = sender.emissions();
    let fabrications: Vec<&Emission> = emissions
        .iter()
        .filter(|(text, _)| text.contains(NOTIFICATION_MARKER))
        .collect();

    assert!(
        fabrications.is_empty(),
        "attendu aucune P0 de la classe fabrication-detected (porte armée = \
         {porte_armee}), observé {} notification(s) : {:?}",
        fabrications.len(),
        emissions
    );

    Ok(())
}

/// **AC1, moitié négative.** Porte armée, et la récidive est un **engagement sur
/// l'avenir** : rien ne part.
///
/// C'est la frontière de classe, et c'est le seul test qui la voie. Le fixture
/// ne diffère du positif que par sa moitié d'assertion, donc un dépôt qui aurait
/// rattaché le détecteur **entier** — le verdict en bloc que mika#2878 a refusé —
/// passerait le test positif et rougirait ici, et nulle part ailleurs.
#[tokio::test]
async fn mika2879_la_recidive_dengagement_sur_lavenir_ne_leve_aucune_p0_de_classe()
-> anyhow::Result<()> {
    aucune_p0_de_classe(recidive(ENGAGEMENT_SUR_LAVENIR), true).await
}

/// **AC5, versant comportemental.** La porte au défaut livré, et la récidive est
/// celle qui appartient à la classe : rien ne part non plus.
///
/// Sans ce contrôle, un site qui ignorerait `MIKA_FABRICATION_P0_NOTIFY`
/// passerait le test positif — et « livré désarmé » ne serait qu'une intention
/// documentée. Ce qui est attesté ici, c'est que la porte est une porte.
#[tokio::test]
async fn mika2879_porte_fermee_letat_accompli_ne_leve_aucune_p0_de_classe() -> anyhow::Result<()> {
    aucune_p0_de_classe(recidive(ETAT_ACCOMPLI), false).await
}
