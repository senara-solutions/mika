//! Intégration : la fenêtre de transmission atteint le prompt conversationnel
//! d'un vrai tour (mika#1747 vol 1, V6 et son contrôle négatif V7).
//!
//! Ce que les tests unitaires ne peuvent pas voir : les trois pièces —
//! le prédicat (`model_transmission::pending`), le site d'alimentation
//! (`agent_loop`, qui est le seul endroit où le couple `llm` est lisible) et le
//! rendu (`prompt::build_system_prompt`) — sont chacune assertée chez elle. Un
//! champ branché sur `None` au site de production passerait les trois sans
//! qu'aucune ne rougisse. Seul un tour réel, avec un signal réel sur le disque,
//! referme cet écart.

use mika_common::llm::mock::*;

use super::harness::EvalHarness;

/// Pose le signal dans l'agent home du harnais, après `build()` et avant
/// `run()` — l'ordre que suit un opérateur : poser, puis laisser passer un tour.
fn pose_signal(home: &std::path::Path, incoming: &str) {
    let path = home.join(mika_agent::model_transmission::SIGNAL_PATH);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, format!("{incoming}\n")).unwrap();
}

/// V6 — le signal posé ouvre la fenêtre, et le bloc `<model-transmission>`
/// atteint le prompt système du tour avec ses quatre règles et sa contrainte.
#[tokio::test]
async fn mika1747_v6_le_signal_pose_atteint_le_prompt_conversationnel() {
    let harness = EvalHarness::builder()
        .provider_name("anthropic")
        .model_name("claude-opus-4-8")
        .responses(vec![text_response("Entendu.")])
        .build()
        .await
        .unwrap();

    pose_signal(harness.home_dir.path(), "anthropic/claude-fable-5-1");

    let trace = harness.run("Salut").await.unwrap();
    let system = trace.captured_requests[0].system.as_deref().unwrap_or("");

    assert!(
        system.contains("## Model Transmission"),
        "la section doit atteindre le prompt du tour"
    );
    // Le couple sortant vient de `llm`, l'entrant du signal (D2), et la lettre
    // porte la date du mtime (D4) — donc on n'assère pas le chemin entier ici
    // (le test unitaire le fait caractère par caractère), mais bien les deux
    // moitiés de la transition et la forme du chemin.
    assert!(
        system.contains("from=\"anthropic/claude-opus-4-8\""),
        "le sortant est le modèle en service, jamais une valeur du signal"
    );
    assert!(system.contains("to=\"anthropic/claude-fable-5-1\""));
    assert!(system.contains("letter=\"transmissions/"));
    assert!(
        system.contains("-from-anthropic-claude-opus-4-8-to-anthropic-claude-fable-5-1.md\">"),
        "le nom de la lettre est entièrement déterminé par la transition"
    );

    // Les quatre règles.
    assert!(system.contains("write_agent_file"));
    assert!(system.contains("what you saw"));
    assert!(system.contains("offered, not commanded"));
    assert!(system.contains("the question you leave"));
    assert!(system.contains("what you release now"));
    assert!(system.contains("Write only what does not already cross."));

    // La contrainte porteuse du ticket.
    assert!(
        system.contains("It ends with a question, not an instruction."),
        "la distinction question/consigne est ce que mika#1747 demande"
    );
}

/// V7 — contrôle négatif, même graine : sans signal, rien. C'est ce qui sépare
/// « rendu dans la fenêtre » de « rendu inconditionnellement » — la seule
/// hypothèse que V6 seul ne peut pas écarter.
#[tokio::test]
async fn mika1747_v7_sans_signal_le_tour_ne_porte_rien() {
    let harness = EvalHarness::builder()
        .provider_name("anthropic")
        .model_name("claude-opus-4-8")
        .responses(vec![text_response("Entendu.")])
        .build()
        .await
        .unwrap();

    let trace = harness.run("Salut").await.unwrap();
    let system = trace.captured_requests[0].system.as_deref().unwrap_or("");

    assert!(!system.contains("## Model Transmission"));
    assert!(!system.contains("<model-transmission"));
    assert!(!system.contains("It ends with a question, not an instruction."));
}

/// D3-2 au bout de la chaîne : le signal résiduel survit au redémarrage, et le
/// successeur ne doit pas recevoir l'ordre d'écrire l'adieu d'un modèle qu'il
/// n'est pas. Même signal que V6, mais servi par le modèle entrant.
#[tokio::test]
async fn mika1747_le_successeur_ne_recoit_pas_ladieu_de_son_predecesseur() {
    let harness = EvalHarness::builder()
        .provider_name("anthropic")
        .model_name("claude-fable-5-1")
        .responses(vec![text_response("Entendu.")])
        .build()
        .await
        .unwrap();

    pose_signal(harness.home_dir.path(), "anthropic/claude-fable-5-1");

    let trace = harness.run("Salut").await.unwrap();
    let system = trace.captured_requests[0].system.as_deref().unwrap_or("");

    assert!(
        !system.contains("## Model Transmission"),
        "un signal tautologique ferme la fenêtre : le successeur ne reçoit rien"
    );
}
