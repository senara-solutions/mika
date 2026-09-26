//! Une image non lue est un fait dit, jamais un « hiccup » (mika#1784).
//!
//! Les tests unitaires de `image_disposition` prouvent que la fonction pure
//! décide juste et que le prédicat ne se lit qu'à un endroit. **Aucun d'eux ne
//! rougirait si la boucle appelait `decide()` au mauvais moment, ou l'appelait et
//! n'en faisait rien.** C'est exactement la forme que mika#2205 a mesurée sur une
//! autre garde : le prédicat était bon et l'appelant ne l'appelait pas. Ces
//! tests-ci font tourner la vraie boucle et lisent ce que le modèle a reçu.
//!
//! Le cas d'Al, à la lettre : une photo avec la légende « Extrais les numéros de
//! cette photo. », sur un tenant dont le provider ne déclare pas la vision.

use mika_common::llm::LlmImage;
use mika_common::llm::mock::*;

use super::harness::EvalHarness;
use super::trace::AgentTrace;

/// La légende d'Al, verbatim depuis le corps du ticket.
const AL_CAPTION: &str = "Extrais les numéros de cette photo.";

fn photo() -> LlmImage {
    LlmImage {
        media_type: "image/jpeg".to_string(),
        data: "/9j/4AAQSkZJRg==".to_string(),
    }
}

/// Tout ce que le modèle a reçu sur le premier appel, contenu des blocs compris.
fn window_text(trace: &AgentTrace) -> String {
    assert!(
        !trace.captured_requests.is_empty(),
        "le tour doit avoir atteint le provider"
    );
    trace.captured_requests[0]
        .messages
        .iter()
        .map(|m| format!("{:?}", m.content))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Le tour porte-t-il un bloc image vers le provider ?
fn carries_image_blocks(trace: &AgentTrace) -> bool {
    trace.captured_requests[0].messages.iter().any(|m| {
        matches!(&m.content, mika_common::llm::LlmContent::Blocks(blocks)
            if blocks.iter().any(|b| matches!(b, mika_common::llm::LlmContentBlock::Image(_))))
    })
}

/// AC1 branche B + AC2, moitié *intent* — le modèle SAIT, au lieu de deviner.
///
/// Avant mika#1784 l'image était droppée en silence : un `warn!` dans un fichier
/// de log, et rien d'autre — ni au modèle, ni à l'utilisateur. Le modèle recevait
/// la légende seule et répondait honnêtement qu'il ne voyait pas l'image
/// (réponse 2 d'Al). Il avait raison par chance, pas par construction.
///
/// **L'ordonnancement est ce que ce test épingle** : le marqueur est persisté
/// entre la résolution d'`effective_llm` et le rechargement de l'historique, donc
/// il doit arriver dans la fenêtre du tour **courant**. S'il migrait après le
/// rechargement, il ne servirait qu'au tour suivant et ce test rougirait.
#[tokio::test]
async fn mika1784_a_withheld_image_is_named_to_the_model_on_the_same_turn() {
    let harness = EvalHarness::builder()
        .responses(vec![text_response(
            "Je ne peux pas lire les images, donc je ne vais pas deviner les numéros.",
        )])
        .supports_vision(false)
        .provider_name("zai")
        .model_name("glm-5.2")
        .user_images(vec![photo()])
        .build()
        .await
        .unwrap();

    let trace = harness.run(AL_CAPTION).await.unwrap();
    let window = window_text(&trace);

    assert!(
        window.contains("Image not transmitted"),
        "le marqueur doit atteindre le TOUR COURANT — s'il est persisté après le \
         rechargement de l'historique, il n'arrive qu'au tour suivant.\n{window}"
    );
    assert!(
        window.contains("Do not describe or infer"),
        "le marqueur doit interdire l'inférence, pas seulement constater\n{window}"
    );
    assert!(
        !carries_image_blocks(&trace),
        "un provider qui ne déclare pas la vision ne doit recevoir aucun bloc image"
    );
}

/// AC2 — l'historique persisté n'affirme plus une pièce jointe jamais reçue.
///
/// C'est le point (3) du diagnostic : deux des quatre sites lisant `user_images`
/// écrivaient `[1 image(s) attached]` en base **même quand l'image avait été
/// droppée**. Au tour suivant, l'historique affirmait donc au modèle qu'une image
/// était jointe alors qu'il ne l'avait jamais reçue — une invitation directe à la
/// fabrication. Al a eu la chance que Mika refuse de fabriquer ; rien dans le code
/// ne le garantissait.
///
/// Le second tour est le test réel : c'est là que l'affirmation d'hier devient le
/// contexte d'aujourd'hui.
#[tokio::test]
async fn mika1784_the_next_turn_inherits_the_fact_and_no_claim_of_attachment() {
    let harness = EvalHarness::builder()
        .responses(vec![text_response("Je ne vois pas d'image.")])
        .supports_vision(false)
        .user_images(vec![photo()])
        .build()
        .await
        .unwrap();

    harness.run(AL_CAPTION).await.unwrap();

    // Second tour, sur la même session. Le harness porte toujours l'image (Al
    // ré-envoie), ce qui est exactement le cas 3 du ticket.
    let trace = harness
        .run_turn(
            "Alors qu'est-ce qu'il y avait sur la photo ?",
            vec![text_response(
                "Je ne l'ai pas reçue, je ne peux pas te le dire.",
            )],
        )
        .await
        .unwrap();

    let window = window_text(&trace);
    assert!(
        !window.contains("image(s) attached"),
        "l'historique affirmait une transmission qui n'a pas eu lieu — c'est le \
         défaut que l'AC2 ferme\n{window}"
    );
    assert!(
        window.contains("image(s) received"),
        "le fait certain — une image a été reçue de l'utilisateur — doit rester \
         dit ; c'est `received` qui remplace `attached`, pas le silence\n{window}"
    );
    assert!(
        window.contains("NOT available to you"),
        "le marqueur du premier tour doit être repris par le rechargement de \
         l'historique : un seul geste ferme le tour courant ET les suivants\n{window}"
    );
}

/// Le contrôle négatif : sur un rail qui déclare la vision, rien ne change.
///
/// La branche A de l'AC1 reste disponible et inchangée. Sans ce test, un
/// correctif qui retiendrait *toutes* les images passerait tous les autres.
#[tokio::test]
async fn mika1784_a_vision_provider_still_gets_the_image_and_no_marker() {
    let harness = EvalHarness::builder()
        .responses(vec![text_response("06 12 34 56 78")])
        .supports_vision(true)
        .provider_name("anthropic")
        .user_images(vec![photo()])
        .build()
        .await
        .unwrap();

    let trace = harness.run(AL_CAPTION).await.unwrap();
    let window = window_text(&trace);

    assert!(
        carries_image_blocks(&trace),
        "un provider qui déclare la vision doit recevoir les blocs image"
    );
    assert!(
        !window.contains("Image not transmitted"),
        "aucune ligne marqueur ne doit être écrite sur `Transmitted` : elle \
         ajouterait une écriture en base à chaque tour avec image pour dire ce que \
         l'image dit déjà mieux\n{window}"
    );
}

/// Un tour sans image n'écrit rien du tout — ni marqueur, ni préfixe.
///
/// `None` n'est pas `Withheld`, et les confondre ferait écrire une ligne marqueur
/// sur chaque tour de conversation ordinaire.
#[tokio::test]
async fn mika1784_a_turn_without_images_is_byte_identical_to_before() {
    let harness = EvalHarness::builder()
        .responses(vec![text_response("Bonjour !")])
        .supports_vision(false)
        .build()
        .await
        .unwrap();

    let trace = harness.run("Bonjour").await.unwrap();
    let window = window_text(&trace);

    assert!(!window.contains("Image not transmitted"), "{window}");
    assert!(!window.contains("image(s) received"), "{window}");
    assert!(!window.contains("image(s) attached"), "{window}");
}

/// AC4 — la trace existe, et elle nomme le couple provider/modèle.
///
/// C'est l'entrée du bloc 3 : l'opérateur répond par une requête, et non par une
/// intuition, à la seule question que l'AC3 pose — *quel couple sert ce tenant, et
/// lit-il les images ?* `image_withheld_no_vision` est un **sole writer**, donc la
/// requête est la liste exacte et non une approximation.
#[tokio::test]
async fn mika1784_the_audit_row_names_the_provider_and_model_that_withheld() {
    let harness = EvalHarness::builder()
        .responses(vec![text_response("Je ne lis pas les images.")])
        .supports_vision(false)
        .provider_name("zai")
        .model_name("glm-5.2")
        .user_images(vec![photo(), photo()])
        .build()
        .await
        .unwrap();

    harness.run(AL_CAPTION).await.unwrap();

    let events = harness
        .db
        .get_audit_events(&harness.session_id)
        .await
        .unwrap();
    let row = events
        .iter()
        .find(|e| e.tool_name == mika_agent::image_disposition::IMAGE_WITHHELD_AUDIT_TOOL_NAME)
        .expect(
            "une image retenue doit laisser une ligne d'audit — sans elle, l'AC3 \
             n'est pas décidable et l'opérateur choisit un modèle à l'intuition",
        );

    assert_eq!(
        row.after_value.as_deref(),
        Some("zai/glm-5.2"),
        "le couple provider/modèle est ce que la requête du bloc 3 agrège"
    );
    assert!(
        row.reasoning.as_deref().is_some_and(|r| r.contains('2')),
        "le nombre d'images perdues doit être lisible : {:?}",
        row.reasoning
    );
}
