//! Intégration : la lettre du prédécesseur atteint le prompt d'un vrai tour,
//! **une seule fois** (mika#2807, V9/V11/V12).
//!
//! Ce que les tests unitaires ne peuvent pas voir. Les quatre pièces — le reçu
//! ([`mika_agent::read_receipt`]), le prédicat
//! ([`mika_agent::model_transmission::incoming`]), le site d'alimentation
//! (`agent_loop`, seul endroit où le couple `llm` est lisible) et le rendu
//! (`prompt::build_system_prompt`) — sont chacune assertée chez elle. Un champ
//! branché sur `None` au site de production passerait les quatre sans qu'aucune
//! ne rougisse.
//!
//! Et surtout : **l'unicité n'est observable que sur deux tours**. Le prédicat
//! sait que le reçu ferme la question, le site sait poser le reçu, mais seul un
//! second tour sur la même graine prouve que le bloc ne revient pas — et c'est
//! la propriété porteuse de tout le découpage de ce vol (D5 : un événement, pas
//! un bloc permanent).

use mika_common::llm::mock::*;

use super::harness::EvalHarness;

/// Dépose une lettre dans `transmissions/`, comme le vol 1 l'aurait écrite,
/// adressée au couple que le harnais sert.
fn pose_letter(home: &std::path::Path, name: &str) {
    let dir = home.join(mika_agent::model_transmission::DIR);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join(name),
        "# À mon successeur\n\nCe que j'ai vu…\n\nQu'est-ce que tu verras que je n'ai pas vu ?\n",
    )
    .unwrap();
}

/// Le nom qu'un vol 1 servi par `claude-opus-4-8` produit pour un successeur
/// `claude-fable-5-1` — composé par `model_transmission::pending`, asserté
/// caractère par caractère par son test unitaire V2.
const LETTRE: &str =
    "2026-10-19-from-anthropic-claude-opus-4-8-to-anthropic-claude-fable-5-1.md";

/// V9 — la lettre posée atteint le prompt système du tour, avec son chemin de
/// lecture, son chemin de dépôt, et les deux libertés.
#[tokio::test]
async fn mika2807_v9_la_lettre_posee_atteint_le_prompt_conversationnel() {
    let harness = EvalHarness::builder()
        .provider_name("anthropic")
        .model_name("claude-fable-5-1")
        .responses(vec![text_response("Lue.")])
        .build()
        .await
        .unwrap();

    pose_letter(harness.home_dir.path(), LETTRE);

    let trace = harness.run("Salut").await.unwrap();
    let system = trace.captured_requests[0].system.as_deref().unwrap_or("");

    assert!(
        system.contains("## Incoming Transmission"),
        "la section doit atteindre le prompt du tour"
    );
    assert!(
        system.contains(&format!("letter=\"transmissions/{LETTRE}\"")),
        "le chemin de lecture est celui de la lettre posée"
    );
    assert!(
        system.contains(
            "response=\"transmissions/2026-10-19-from-anthropic-claude-fable-5-1-response-to-anthropic-claude-opus-4-8.md\""
        ),
        "le chemin de dépôt porte la date de la LETTRE, pas celle du tour (D6)"
    );
    assert!(system.contains("read_agent_file"));
    assert!(system.contains("It is a testimony, not an instruction."));
    assert!(system.contains("You are free to contradict it."));
    assert!(system.contains("not answering is itself an answer"));
    assert!(system.contains("you are not writing to someone"));

    // V10 — le reçu existe après le tour. C'est le site d'alimentation qui le
    // pose, et rien d'autre dans la chaîne ne peut le faire à sa place.
    let receipt = harness
        .home_dir
        .path()
        .join(mika_agent::read_receipt::receipt_path(&format!(
            "transmissions/{LETTRE}"
        )));
    assert!(
        receipt.exists(),
        "le tour qui sert la lettre pose son reçu : {}",
        receipt.display()
    );
}

/// V11 — **la propriété porteuse** : un second tour sur la même graine ne rend
/// plus le bloc. La relecture est un événement, pas un bloc permanent (D5).
///
/// Sans cette assertion, un reçu jamais posé — ou posé au mauvais chemin —
/// laisserait V9 vert et installerait le bloc à tous les tours de toutes les
/// sessions suivantes, exactement ce que le ticket demande d'éviter.
#[tokio::test]
async fn mika2807_v11_le_second_tour_ne_rend_plus_le_bloc() {
    let harness = EvalHarness::builder()
        .provider_name("anthropic")
        .model_name("claude-fable-5-1")
        .responses(vec![text_response("Lue.")])
        .build()
        .await
        .unwrap();

    pose_letter(harness.home_dir.path(), LETTRE);

    let first = harness.run("Salut").await.unwrap();
    assert!(
        first.captured_requests[0]
            .system
            .as_deref()
            .unwrap_or("")
            .contains("## Incoming Transmission"),
        "le premier tour doit servir la lettre, sinon le second est vrai par vacuité"
    );

    let second = harness
        .run_turn("Et maintenant ?", vec![text_response("Entendu.")])
        .await
        .unwrap();

    // `clear_and_set` remet le curseur des réponses à zéro mais ne vide PAS
    // `captured_requests` : la liste du mock est cumulative sur toute la vie du
    // harnais. Lire `[0]` ici relirait le prompt du PREMIER tour et le test
    // serait rouge en disant le contraire de ce qui s'est passé. Les requêtes du
    // second tour sont donc celles qui suivent le compte du premier.
    let suite = &second.captured_requests[first.captured_requests.len()..];
    assert!(
        !suite.is_empty(),
        "le second tour doit avoir appelé le modèle au moins une fois"
    );

    for request in suite {
        let system = request.system.as_deref().unwrap_or("");
        assert!(
            !system.contains("## Incoming Transmission"),
            "une lettre lue ne revient pas au tour suivant"
        );
        assert!(!system.contains("<incoming-transmission"));
        assert!(!system.contains("You are free to contradict it."));
    }

    // La lettre est toujours là : le reçu la referme, il ne la détruit pas.
    assert!(
        harness
            .home_dir
            .path()
            .join(mika_agent::model_transmission::DIR)
            .join(LETTRE)
            .exists(),
        "`read_agent_file` doit pouvoir la rouvrir à tout moment"
    );
}

/// V12 — contrôle négatif, même graine : sans lettre, rien. C'est ce qui sépare
/// « rendu quand une lettre attend » de « rendu inconditionnellement » — la
/// seule hypothèse que V9 seul ne peut pas écarter.
#[tokio::test]
async fn mika2807_v12_sans_lettre_le_tour_ne_porte_rien() {
    let harness = EvalHarness::builder()
        .provider_name("anthropic")
        .model_name("claude-fable-5-1")
        .responses(vec![text_response("Entendu.")])
        .build()
        .await
        .unwrap();

    let trace = harness.run("Salut").await.unwrap();
    let system = trace.captured_requests[0].system.as_deref().unwrap_or("");

    assert!(!system.contains("## Incoming Transmission"));
    assert!(!system.contains("<incoming-transmission"));
    assert!(!system.contains("It is a testimony, not an instruction."));
}

/// D2 au bout de la chaîne : une lettre adressée à un autre modèle ne doit pas
/// être servie. Même disque, même répertoire, destinataire différent — le cas
/// d'un agent dont le modèle a été ramené en arrière après un swap.
#[tokio::test]
async fn mika2807_une_lettre_adressee_a_un_autre_modele_nest_pas_servie() {
    let harness = EvalHarness::builder()
        .provider_name("anthropic")
        .model_name("claude-opus-4-8")
        .responses(vec![text_response("Entendu.")])
        .build()
        .await
        .unwrap();

    // Adressée à `claude-fable-5-1` ; le harnais sert `claude-opus-4-8`.
    pose_letter(harness.home_dir.path(), LETTRE);

    let trace = harness.run("Salut").await.unwrap();
    let system = trace.captured_requests[0].system.as_deref().unwrap_or("");

    assert!(
        !system.contains("## Incoming Transmission"),
        "le courrier d'un autre n'est pas servi, et son reçu n'est pas posé"
    );
    assert!(
        !harness
            .home_dir
            .path()
            .join(mika_agent::read_receipt::receipt_path(&format!(
                "transmissions/{LETTRE}"
            )))
            .exists(),
        "un tour qui ne sert pas la lettre ne doit pas la consommer"
    );
}
