//! mika#2784 — ce qu'un agent fail-closed reçoit **réellement** comme outils.
//!
//! **Pourquoi un vrai tour et pas la construction de l'identité.** Les tests
//! unitaires de `prompt::fail_closed_identity` lisent les champs de la structure
//! retournée : `tools.disabled` plein, `tools.allowed == None`. Ils resteraient
//! verts si la posture n'était jamais consultée au moment de publier le tableau
//! d'outils — un champ que personne ne lit et un interrupteur qui fonctionne
//! sont indistinguables par toute assertion qui ne regarde pas le tableau que le
//! modèle a reçu. C'est ce tableau que ce fichier lit, et c'est exactement
//! l'argument de `test_mika_arch_tool_allowlist_2748.rs` pour la posture
//! nominale, porté ici sur la posture dégradée.
//!
//! **Pourquoi `missing_file` et pas `parse_error`.** Le nom du home de
//! `EvalHarness` est celui d'un `TempDir`, donc une chaîne aléatoire que
//! `find_well_known_agent` ne reconnaîtra jamais. Or la branche `parse_error`
//! ne sert la posture fail-closed **qu'aux agents bien connus** (#811) : dans ce
//! harnais, un `identity.toml` invalide produit `Identity::default()`, c'est-à-
//! dire l'inverse de ce qu'on veut mesurer. Le fichier **absent**, lui, fait
//! fail-closed pour *tout* agent depuis mika#2027 — donc ici. La couverture de
//! `parse_error` est tenue par les tests unitaires de `prompt.rs`, qui peuvent
//! nommer leur répertoire.
//!
//! **Le contrôle négatif est portant.** Seule, « `create_task` est absent » est
//! satisfaite par un registre qui ne l'a jamais porté, et par un filtre qui
//! retirerait tout à tout le monde. Le second test fait tourner le même harnais
//! avec son `identity.toml` nominal et affirme que **le même nom est présent** :
//! c'est ce qui fait dire à l'absence ci-dessus *la posture l'a retiré* plutôt
//! que *il n'y était pas*.

use mika_agent::well_known_agents::MIKA_ARCH_DISABLED_TOOLS;
use mika_common::llm::mock::*;

use super::harness::EvalHarness;

/// Un builtin que la posture fail-closed laisse passer. #811 compte les
/// écritures mémoire propres à l'agent comme constitutives du fait d'être un
/// agent, et `send_message` est ce par quoi un agent peut dire qu'il va mal —
/// c'est la question 1 du ticket, et le morceau 1 ne la tranche pas.
const ADMITTED: &str = "send_message";

/// Un builtin que la posture dénie, nommé dans `MIKA_ARCH_DISABLED_TOOLS`. Il
/// est enregistré par `default_tools()` **et servi par ce harnais**, donc son
/// absence du tableau servi ne peut venir que du filtre.
///
/// Les deux conditions comptent. Le plan nommait `create_task` : il est bien
/// dans la dénylist, mais `EvalHarness` ne le publie pas (`cancel_task`,
/// `complete_task` et `get_task` y sont, lui non), donc le pin aurait été
/// **vide** — mesuré, pas supposé, et c'est le contrôle négatif ci-dessous qui
/// l'a dit. `write_agent_file` satisfait les deux.
const WITHHELD: &str = "write_agent_file";

/// Les noms d'outils que le modèle a reçus au premier appel LLM du tour.
///
/// `fail_closed` supprime l'`identity.toml` que `build()` vient d'écrire —
/// `build()` en sème un (`name`/`emoji`), état dans lequel aucun agent
/// provisionné ne se trouve mais que le harnais doit produire pour que ses
/// autres tests mesurent la posture nominale.
async fn served_tool_names(fail_closed: bool) -> Vec<String> {
    let harness = EvalHarness::builder()
        .responses(vec![text_response("Noted.")])
        .build()
        .await
        .unwrap();

    let identity_path = harness.home_dir.path().join("identity.toml");
    if fail_closed {
        std::fs::remove_file(&identity_path).unwrap();
        assert!(
            !identity_path.exists(),
            "la fixture doit produire l'ABSENCE du fichier : c'est la seule \
             cause qui fait fail-closed un agent non bien connu (mika#2027)"
        );
    } else {
        assert!(
            identity_path.exists(),
            "le contrôle négatif suppose le fichier que `build()` sème"
        );
    }

    let trace = harness.run("Review the plan.").await.unwrap();
    trace
        .captured_requests
        .first()
        .expect("the mock provider must have captured the request")
        .tools
        .as_deref()
        .unwrap_or(&[])
        .iter()
        .map(|t| t.name.clone())
        .collect()
}

/// **V7** — la posture dégradée atteint le tableau réellement servi.
#[tokio::test]
async fn mika2784_un_agent_fail_closed_ne_recoit_aucun_builtin_mutationnel() {
    // Prémisses des deux constantes, épinglées contre la dénylist réelle : un
    // `WITHHELD` qui en sortirait, ou un `ADMITTED` qui y entrerait, rendraient
    // les deux assertions ci-dessous vraies pour la mauvaise raison.
    assert!(
        MIKA_ARCH_DISABLED_TOOLS.contains(&WITHHELD),
        "`{WITHHELD}` doit être dénié par la posture pour que son absence veuille \
         dire quelque chose"
    );
    assert!(
        !MIKA_ARCH_DISABLED_TOOLS.contains(&ADMITTED),
        "`{ADMITTED}` doit NE PAS être dans la dénylist pour que sa présence \
         veuille dire quelque chose"
    );

    let names = served_tool_names(true).await;

    assert!(
        !names.iter().any(|n| n == WITHHELD),
        "un builtin dénié par la posture fail-closed ne doit pas atteindre le \
         modèle. Si cette assertion rougit, `fail_closed_identity` construit \
         bien une dénylist et personne ne la consulte au moment de publier les \
         outils — l'état exact qu'aucun test unitaire ne peut voir. Servis : \
         {names:?}"
    );
    assert!(
        !names.is_empty(),
        "un tableau vide satisferait l'assertion ci-dessus sans rien prouver — \
         il voudrait dire que le harnais n'a publié aucun outil, pas que la \
         posture en a retiré un. Servis : {names:?}"
    );
    assert!(
        names.iter().any(|n| n == ADMITTED),
        "`{ADMITTED}` reste servi en posture fail-closed : `allowed` vaut `None` \
         et la dénylist ne le nomme pas. C'est l'état de ce vol, pas une \
         préférence — le resserrement est le morceau 2, suspendu à la question 1 \
         (un agent fail-closed garde-t-il de quoi signaler son état ?). Si cette \
         assertion rougit, le morceau 2 a été fait sans décision écrite. \
         Servis : {names:?}"
    );
}

/// **Contrôle négatif** — avec une identité valide, le même builtin est servi.
///
/// Doit être vu vert **à côté** du test ci-dessus : sans lui, l'absence qu'il
/// affirme tiendrait aussi d'un registre qui n'a jamais porté `create_task`.
#[tokio::test]
async fn mika2784_avec_une_identite_valide_le_meme_builtin_est_servi() {
    let names = served_tool_names(false).await;

    assert!(
        names.iter().any(|n| n == WITHHELD),
        "un agent dont l'`identity.toml` est présent et valide ne déclare ni \
         dénylist ni allowlist : il garde tous ses builtins. Servis : {names:?}"
    );
}
