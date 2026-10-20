//! La déposition atteint-elle le successeur, et une seule fois ? (mika#2805, V10–V12)
//!
//! Les tests unitaires voisins prouvent chacun une pièce : que `unread` trouve la
//! plus récente non relue (`compaction_handsoff`), que `resolve` pose le reçu, et
//! que le builder sait rendre le bloc (`prompt`). **Aucun des trois ne rougirait
//! si `run_agent_inner` n'appelait jamais `resolve`** — la chaîne entière
//! resterait verte avec un successeur qui ne lit rien. C'est la forme que
//! mika#2205 a mesurée ailleurs : le prédicat juste, et l'appelant qui n'appelle
//! pas.
//!
//! Les trois tests ci-dessous font tourner la vraie boucle, lisent le prompt
//! système que le modèle a reçu, **et** regardent le disque. Le troisième est le
//! contrôle négatif de la porte D2 : à déposition identique, le seul changement
//! est le compte de messages, et il doit faire basculer la cérémonie du vol 1 à
//! la place du rituel — sans poser de reçu.
//!
//! Écrits dans la forme de `test_compaction_handsoff_1748.rs`, leur voisin de
//! vol 1 sur le même entonnoir.

use mika_common::llm::mock::*;

use super::harness::EvalHarness;
use super::trace::AgentTrace;

/// Le nom d'une déposition, dans la forme que le vol 1 écrit.
const DEPOSITION: &str = "handsoffs/2026-10-19-1200-compaction-handsoff.md";

/// Le prompt système du **dernier** appel LLM de la trace.
///
/// `captured_requests` est cumulatif sur la vie du mock, donc un second tour
/// s'y ajoute au lieu de remplacer le premier : c'est exactement ce que V11
/// mesure.
fn last_system_prompt(trace: &AgentTrace) -> String {
    let last = trace
        .captured_requests
        .last()
        .expect("the turn must have reached the provider");
    last.system.as_deref().unwrap_or("").to_string()
}

/// Sème `n` messages dans la session du harnais.
///
/// Le message du tour lui-même est persisté **avant** que `load_agent_context`
/// ne compte, donc le prédicat voit `n + 1` — le décalage de 1 mesuré par le
/// vol 1.
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

/// Pose une déposition dans l'agent home du harnais.
async fn poser_deposition(harness: &EvalHarness) {
    let path = harness.home_dir.path().join(DEPOSITION);
    tokio::fs::create_dir_all(path.parent().unwrap())
        .await
        .unwrap();
    tokio::fs::write(&path, "Fact: le résidu que le résumé ne portera pas.\n")
        .await
        .unwrap();
}

/// Le reçu de relecture existe-t-il sur le disque ?
async fn recu_pose(harness: &EvalHarness) -> bool {
    let receipt = mika_agent::read_receipt::receipt_path(DEPOSITION);
    harness
        .home_dir
        .path()
        .join(receipt)
        .try_exists()
        .unwrap_or(false)
}

/// V10 (AC1) — une déposition non relue, la condensation passée : le bloc du
/// rituel atteint le prompt système, et le reçu est posé.
#[tokio::test]
async fn mika2805_v10_la_deposition_atteint_le_successeur_et_le_recu_est_pose() {
    let harness = EvalHarness::builder()
        .responses(vec![text_response("Je la lis.")])
        .build()
        .await
        .unwrap();

    poser_deposition(&harness).await;
    assert!(
        !recu_pose(&harness).await,
        "sanité : aucun reçu avant le tour"
    );

    let trace = harness.run("Où en étions-nous ?").await.unwrap();
    let system = last_system_prompt(&trace);

    assert!(
        system.contains("<compaction-handsoff"),
        "le bloc du rituel doit atteindre le prompt — prompt de {} caractères",
        system.len()
    );
    assert!(
        system.contains(DEPOSITION),
        "le bloc nomme le chemin exact de la déposition, pas un répertoire : \
         sans lui, `read_agent_file` est une invitation vague"
    );
    assert!(
        system.contains("handsoffs/2026-10-19-1200-compaction-handsoff-response.md"),
        "le canal de réponse (AC4) est nommé dans le même bloc"
    );
    assert!(
        system.contains("free to contradict"),
        "AC3 — une déposition qu'on ne peut pas contredire est un ordre, et \
         personne n'a autorisé ce tour-là à en donner"
    );
    assert!(
        system.contains("primed_from_handsoff"),
        "AC3 — le marqueur reste la porte de promotion à n=2"
    );

    assert!(
        recu_pose(&harness).await,
        "AC2 — le reçu est posé pendant le tour servi, pas au tour suivant"
    );
}

/// V11 (AC2) — le second tour ne rend plus le bloc.
///
/// La relecture est marquée, donc un handsoff déjà servi ne se re-sert pas. Sans
/// ce test, « servi une fois » et « servi à chaque tour » renvoient les mêmes
/// octets au premier tour.
#[tokio::test]
async fn mika2805_v11_le_second_tour_ne_ressert_pas_la_deposition() {
    let harness = EvalHarness::builder()
        .responses(vec![
            text_response("Je la lis."),
            text_response("Rien à ajouter."),
        ])
        .build()
        .await
        .unwrap();

    poser_deposition(&harness).await;

    let premier = harness.run("Où en étions-nous ?").await.unwrap();
    assert!(
        last_system_prompt(&premier).contains("<compaction-handsoff"),
        "sanité : le premier tour sert bien la déposition"
    );

    let second = harness.run("Et sinon ?").await.unwrap();
    let system = last_system_prompt(&second);

    assert!(
        !system.contains("<compaction-handsoff"),
        "une déposition déjà servie ne se re-sert pas : {system}"
    );
    assert!(
        !system.contains("Handsoff Review"),
        "ni le bloc, ni son titre de section"
    );
    // Sanité : l'absence ne vient pas d'un prompt vide ni d'un tour qui n'a
    // jamais atteint le fournisseur.
    assert!(
        system.contains("## Identity"),
        "sanité : le prompt système est bien celui du builder conversationnel"
    );
}

/// V12 — **contrôle négatif de la porte D2** : déposition présente, mais le
/// compte est au-dessus du pré-seuil.
///
/// Le déclencheur du rituel est la condensation **passée**, pas la présence d'un
/// fichier. Au franchissement, c'est la cérémonie du vol 1 qui rend — et le
/// rituel ne doit ni rendre, ni consommer : un reçu posé ici brûlerait une
/// déposition que personne n'a lue.
#[tokio::test]
async fn mika2805_v12_au_franchissement_la_ceremonie_passe_devant_sans_consommer() {
    let harness = EvalHarness::builder()
        .responses(vec![text_response("Noté.")])
        .build()
        .await
        .unwrap();

    poser_deposition(&harness).await;
    // 42 semés + le message du tour = 43, le franchissement exact à 85 % de 50.
    seed_messages(&harness, 42).await;

    let trace = harness.run("Et maintenant ?").await.unwrap();
    let system = last_system_prompt(&trace);

    assert!(
        system.contains("<compaction-imminent"),
        "au franchissement, la cérémonie du vol 1 rend : {system}"
    );
    assert!(
        !system.contains("<compaction-handsoff"),
        "les deux bras sont mutuellement exclusifs par construction — c'est ce \
         qui empêche l'auteur de se relire lui-même"
    );
    assert!(
        !recu_pose(&harness).await,
        "aucun reçu sur cette branche : le bras `Imminent` ne consomme rien, \
         sinon la déposition serait brûlée sans avoir été lue"
    );
}
