//! La cérémonie de handsoff atteint-elle vraiment le prompt ? (mika#1748, V3/V4)
//!
//! Les tests unitaires voisins prouvent trois choses séparément : que le prédicat
//! franchit à 43 (`compaction_handsoff`), que le builder sait rendre le bloc
//! (`prompt`), et que le champ existe sur `PromptContext`. **Aucun des trois ne
//! rougirait si `load_agent_context` ne comptait jamais les messages, ou comptait
//! et laissait le champ à `None`** — la chaîne entière resterait verte avec un
//! plafond de contexte muet. C'est exactement la forme que mika#2205 a mesurée
//! sur une autre garde : le prédicat était juste et l'appelant n'appelait pas.
//!
//! Ces deux tests font tourner la vraie boucle et lisent le prompt système que le
//! modèle a reçu. Le second est le **contrôle négatif** : à un message près, le
//! bloc doit disparaître. Sans lui, un bloc rendu inconditionnellement passerait
//! le premier test pour la mauvaise raison — et « rendu au franchissement » et
//! « rendu toujours » renverraient les mêmes octets.
//!
//! Écrits dans la forme de `test_context_window_budget_2295.rs`, son voisin sur
//! le même entonnoir.

use mika_common::llm::mock::*;

use super::harness::EvalHarness;
use super::trace::AgentTrace;

/// Le prompt système envoyé au premier appel LLM du tour.
fn system_prompt(trace: &AgentTrace) -> String {
    assert!(
        !trace.captured_requests.is_empty(),
        "the turn must have reached the provider"
    );
    trace.captured_requests[0]
        .system
        .as_deref()
        .unwrap_or("")
        .to_string()
}

/// Sème `n` messages dans la session du harnais.
///
/// Le message du tour lui-même est persisté **avant** que `load_agent_context`
/// ne compte, donc le prédicat voit `n + 1`. C'est la correction mesurée du D4
/// du plan (qui annonçait un retard de 2) : le décalage est de 1 ici, et c'est
/// pourquoi ces tests sèment 42 et 41, pas 43 et 42.
async fn seed_messages(harness: &EvalHarness, n: usize) {
    for i in 1..=n {
        harness
            .db
            .save_message(
                &harness.session_id,
                if i % 2 == 0 { "assistant" } else { "user" },
                &format!("tour {i}"),
                None,
            )
            .await
            .unwrap();
    }
}

/// V3 — au franchissement, le bloc de cérémonie est dans le prompt système.
#[tokio::test]
async fn mika1748_au_franchissement_la_ceremonie_atteint_le_prompt() {
    let harness = EvalHarness::builder()
        .responses(vec![text_response("Noté.")])
        .build()
        .await
        .unwrap();

    // 42 semés + le message du tour = 43, le franchissement exact à 85 % de 50.
    seed_messages(&harness, 42).await;

    let trace = harness.run("Et maintenant ?").await.unwrap();
    let system = system_prompt(&trace);

    assert!(
        system.contains("<compaction-imminent"),
        "le bloc de cérémonie doit atteindre le prompt au franchissement — \
         prompt de {} caractères",
        system.len()
    );
    assert!(
        system.contains(r#"messages="43""#),
        "le compte porté par le bloc est celui que le prédicat a vu, pas une \
         approximation : le décalage d'un message est la correction mesurée du D4"
    );
    assert!(
        system.contains("handsoffs/"),
        "la règle qui nomme le chemin d'écriture doit traverser"
    );
    assert!(
        system.contains("testimony, not transmission"),
        "la discipline mp#187 doit traverser — c'est elle qui distingue cette \
         cérémonie d'un résumé"
    );
}

/// V4 — un message sous le seuil, et le bloc n'existe pas.
///
/// Le contrôle négatif de V3. Même semis à un message près : seul le
/// franchissement change.
#[tokio::test]
async fn mika1748_un_message_sous_le_seuil_le_bloc_est_absent() {
    let harness = EvalHarness::builder()
        .responses(vec![text_response("Noté.")])
        .build()
        .await
        .unwrap();

    // 41 semés + le message du tour = 42, un message sous le franchissement.
    seed_messages(&harness, 41).await;

    let trace = harness.run("Et maintenant ?").await.unwrap();
    let system = system_prompt(&trace);

    assert!(
        !system.contains("<compaction-imminent"),
        "sous le seuil, le bloc ne doit rien rendre — sinon « rendu au \
         franchissement » et « rendu toujours » sont indiscernables"
    );
    assert!(
        !system.contains("Context Ceiling"),
        "ni le bloc, ni son titre de section"
    );
    // Sanité : l'absence du bloc ne vient pas d'un prompt vide ou d'un tour qui
    // n'a jamais atteint le fournisseur.
    assert!(
        system.contains("## Identity"),
        "sanité : le prompt système est bien celui du builder conversationnel"
    );
}
