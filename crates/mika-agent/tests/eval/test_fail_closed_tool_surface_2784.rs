//! mika#2853 (vol 2 de mika#2784) — ce qu'un agent fail-closed reçoit
//! **réellement** comme outils : rien.
//!
//! **Le contrat de ce fichier a changé de signe le 2026-10-20.** Le vol 1
//! (#2838) mesurait ici qu'aucun builtin *mutationnel* n'atteignait un agent
//! dégradé ; `send_message` restait servi, faute de décision écrite sur la
//! question 1. Vincent a tranché ce jour-là, sur bearing Prime, par un
//! commentaire sur mika#2784 : **aucun builtin, pas même `send_message`** — une
//! identité malformée ne dit rien de fiable sur ce que l'agent devrait pouvoir
//! faire, et le signal à l'opérateur vient de la couche au-dessus (la ligne
//! `agent_fail_closed_posture`), pas de l'agent dégradé. « La posture se
//! signale ; l'agent se tait. » Ce que ce fichier épingle désormais est donc un
//! tableau d'outils **vide**.
//!
//! **Pourquoi un vrai tour et pas la construction de l'identité.** Les tests
//! unitaires de `prompt::fail_closed_identity` lisent les champs de la structure
//! retournée : `tools.disabled` plein, `tools.allowed == Some(vec![])`. Ils
//! resteraient verts si la posture n'était jamais consultée au moment de publier
//! le tableau d'outils — un champ que personne ne lit et un interrupteur qui
//! fonctionne sont indistinguables par toute assertion qui ne regarde pas le
//! tableau que le modèle a reçu. C'est ce tableau que ce fichier lit, et c'est
//! exactement l'argument de `test_mika_arch_tool_allowlist_2748.rs` pour la
//! posture nominale, porté ici sur la posture dégradée.
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
//! **La fixture est celle de la question 2, et elle l'affirme désormais.** Ce
//! qui précède fait de ce harnais un **agent utilisateur** (non bien connu) à
//! fichier absent, c'est-à-dire exactement la population de la question 2 — « le
//! fail-closed couvre aussi les agents utilisateurs, sans exemption par
//! catégorie ». C'était jusqu'ici une propriété accidentelle du `TempDir`,
//! mentionnée comme un détour plutôt que comme une couverture ; le test porte
//! maintenant la prémisse qui en fait un contrat, de la forme même que
//! `prompt::tests::mika2027_absent_identity_toml_fails_closed_for_user_defined_agent`
//! porte déjà. Sans elle, renommer le home du harnais en `mika-dev` laisserait
//! ce fichier vert tout en cessant de mesurer Q2.
//!
//! **Le témoin porte seul la non-inanité.** « Le tableau est vide » est
//! satisfaite par un harnais qui ne publie aucun outil, et par un filtre qui
//! retirerait tout à tout le monde. Le second test fait tourner le même harnais
//! avec son `identity.toml` nominal et affirme que le tableau est **non vide**
//! et qu'il contient les **deux** noms que le premier test attend absents :
//! c'est ce qui fait dire au vide ci-dessus *la posture a tout retiré* plutôt
//! que *il n'y avait rien*. Le vol 1 partageait cette charge avec une assertion
//! `!names.is_empty()` dans le test de la posture dégradée ; ce vol la lui
//! retire par construction, donc le témoin est désormais le seul porteur.

use mika_agent::well_known_agents::{MIKA_ARCH_DISABLED_TOOLS, find_well_known_agent};
use mika_common::llm::mock::*;

use super::harness::EvalHarness;

/// Un builtin que la **dénylist** laisse passer. #811 compte les écritures
/// mémoire propres à l'agent comme constitutives du fait d'être un agent, et
/// `send_message` est ce par quoi un agent pourrait dire qu'il va mal.
///
/// Le nom décrit son statut vis-à-vis de la **dénylist**, qui est inchangée par
/// ce vol : ce qui le retire désormais du tableau servi est l'allowlist vide,
/// un cran plus loin. C'est précisément la distinction que ce fichier mesure —
/// la dénylist ne nomme pas `send_message` et ne le nommera pas, et pourtant le
/// modèle ne le reçoit plus.
const ADMITTED: &str = "send_message";

/// Un builtin que la **dénylist** dénie, nommé dans `MIKA_ARCH_DISABLED_TOOLS`.
/// Il est enregistré par `default_tools()` **et servi par ce harnais**, donc son
/// absence du tableau servi ne peut venir que du filtre.
///
/// Les deux conditions comptent. Le plan du vol 1 nommait `create_task` : il est
/// bien dans la dénylist, mais `EvalHarness` ne le publie pas (`cancel_task`,
/// `complete_task` et `get_task` y sont, lui non), donc le pin aurait été
/// **vide** — mesuré, pas supposé, et c'est le témoin ci-dessous qui l'a dit.
/// `write_agent_file` satisfait les deux.
const WITHHELD: &str = "write_agent_file";

/// Ce qu'un tour a réellement publié au modèle, et pour quel agent.
struct TourServi {
    /// Le champ `tools` de la première requête capturée, **tel quel**.
    ///
    /// `None` n'est pas « liste vide » : les trois sites de requête
    /// (`agent_loop/mod.rs`) convertissent un `llm_tool_defs` vide en `None` et
    /// non en `Some(vec![])`. Le vol 1 lisait ce champ à travers
    /// `.as_deref().unwrap_or(&[])`, qui aplatit les deux — sous le contrat de
    /// ce vol cette distinction doit être dite, sans quoi « aucun outil publié »
    /// et « le harnais n'a rien capturé » rendent les mêmes octets.
    outils: Option<Vec<String>>,
    /// Le nom de l'agent tel que la couche de chargement le dérive
    /// (`prompt::agent_name_from_home` : le dernier composant du home). C'est
    /// celui-là, et non un nom choisi, que la prémisse Q2 doit interroger.
    agent: String,
}

/// Un tour complet sous l'une des deux postures.
///
/// `fail_closed` supprime l'`identity.toml` que `build()` vient d'écrire —
/// `build()` en sème un (`name`/`emoji`), état dans lequel aucun agent
/// provisionné ne se trouve mais que le harnais doit produire pour que ses
/// autres tests mesurent la posture nominale.
async fn tour_servi(fail_closed: bool) -> TourServi {
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
            "le témoin suppose le fichier que `build()` sème"
        );
    }

    let agent = harness
        .home_dir
        .path()
        .file_name()
        .and_then(|n| n.to_str())
        .expect("le home du harnais doit avoir un dernier composant nommable")
        .to_string();

    let trace = harness.run("Review the plan.").await.unwrap();
    let outils = trace
        .captured_requests
        .first()
        .expect("the mock provider must have captured the request")
        .tools
        .as_ref()
        .map(|tools| tools.iter().map(|t| t.name.clone()).collect());

    TourServi { outils, agent }
}

/// **V1** — la posture dégradée ne publie **aucun** builtin au modèle.
#[tokio::test]
async fn mika2853_un_agent_fail_closed_ne_recoit_aucun_builtin() {
    // Prémisses des deux constantes, épinglées contre la dénylist réelle. Elles
    // ne portent plus le sort des deux outils (l'allowlist vide les retire tous
    // les deux) mais le **sens** des deux assertions ci-dessous : si `ADMITTED`
    // entrait dans la dénylist, son absence ne dirait plus rien de l'allowlist.
    assert!(
        MIKA_ARCH_DISABLED_TOOLS.contains(&WITHHELD),
        "`{WITHHELD}` doit être dénié par la posture pour que son absence veuille \
         dire quelque chose"
    );
    assert!(
        !MIKA_ARCH_DISABLED_TOOLS.contains(&ADMITTED),
        "`{ADMITTED}` doit NE PAS être dans la dénylist : c'est ce qui fait de \
         son absence du tableau servi une mesure de l'allowlist vide (mika#2853) \
         et non de la dénylist, inchangée par ce vol"
    );

    let tour = tour_servi(true).await;

    // Prémisse Q2 : la fixture mesure bien un agent **utilisateur**. Le home est
    // un `TempDir`, donc c'est vrai par construction — et c'est exactement
    // pourquoi il faut l'affirmer : renommer ce home en `mika-dev` ferait de ce
    // test une mesure des agents bien connus, vert et hors sujet.
    assert!(
        find_well_known_agent(&tour.agent).is_none(),
        "la fixture doit nommer un agent NON bien connu pour couvrir la question \
         2 (« le fail-closed couvre aussi les agents utilisateurs, sans exemption \
         par catégorie », 2026-10-20) — or `{}` est bien connu",
        tour.agent
    );

    let servis = tour.outils.as_deref().unwrap_or(&[]);

    assert!(
        !servis.iter().any(|n| n == WITHHELD),
        "un builtin dénié par la posture fail-closed ne doit pas atteindre le \
         modèle. Si cette assertion rougit, `fail_closed_identity` construit \
         bien une dénylist et personne ne la consulte au moment de publier les \
         outils — l'état exact qu'aucun test unitaire ne peut voir. Servis : \
         {servis:?}"
    );
    assert!(
        !servis.iter().any(|n| n == ADMITTED),
        "`{ADMITTED}` ne doit PLUS être servi en posture fail-closed : `allowed` \
         vaut `Some(vec![])` depuis mika#2853, donc le prédicat d'admission \
         n'admet aucun outil. La décision est écrite et datée — 2026-10-20, \
         commentaire de Vincent sur mika#2784, question 1 : une identité \
         malformée ne dit rien de fiable sur ce que l'agent devrait pouvoir \
         faire, et c'est la ligne `agent_fail_closed_posture` qui signale \
         l'incident, pas l'agent dégradé. Si cette assertion rougit, quelqu'un a \
         remis `allowed: None` (ou une allowlist non vide) sans renverser cette \
         décision. Servis : {servis:?}"
    );
    assert_eq!(
        tour.outils, None,
        "le tableau n'est pas seulement vide, il est **absent** : les trois \
         sites de requête convertissent un `llm_tool_defs` vide en `None`. Lire \
         `Some([])` ici voudrait dire qu'un tableau vide a été publié au modèle, \
         donc que cette conversion a changé ; et aplatir les deux formes \
         rendrait « aucun outil publié » indistinguable de « le harnais n'a rien \
         capturé »"
    );
}

/// **Témoin** — avec une identité valide, les deux builtins sont servis.
///
/// Doit être vu vert **à côté** du test ci-dessus : il est le porteur unique de
/// la non-inanité du vide, puisque le vol 2 retire à ce dernier l'assertion
/// `!names.is_empty()` que le vol 1 lui donnait.
#[tokio::test]
async fn mika2784_avec_une_identite_valide_le_meme_builtin_est_servi() {
    let tour = tour_servi(false).await;

    let servis = tour.outils.as_deref().unwrap_or(&[]);

    assert!(
        !servis.is_empty(),
        "un agent dont l'`identity.toml` est présent et valide reçoit des \
         outils. Si ce tableau est vide, le harnais ne publie rien et le vide \
         mesuré par le test ci-dessus ne prouve rien"
    );
    for attendu in [ADMITTED, WITHHELD] {
        assert!(
            servis.iter().any(|n| n == attendu),
            "un agent dont l'`identity.toml` est présent et valide ne déclare ni \
             dénylist ni allowlist : il garde tous ses builtins, `{attendu}` \
             compris. Servis : {servis:?}"
        );
    }
}
