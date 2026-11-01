//! La résolution du jeton de lecture CI est **unique**, et les trois handlers
//! webhook la reçoivent (mika#2965, AC1).
//!
//! # Pourquoi un scan de source, et pas un test de comportement
//!
//! AC1 énonce deux choses, dont une seule est observable depuis l'extérieur.
//! « Les trois handlers reçoivent un `Option<&CiReadToken>` » se mesure sur
//! `verdict_handler` — c'est ce que fait
//! `verdict_handler.rs § mika2965_tests` — mais « résolu **une seule fois** »
//! est une propriété de la **forme** du site d'appel. Deux résolutions
//! successives dans la même chaîne `github` rendraient exactement le même jeton,
//! produiraient exactement les mêmes lignes `ci_read_token_source`, et
//! laisseraient toute assertion de comportement verte — tout en payant deux
//! échanges de jeton d'installation par événement webhook, et en créant deux
//! endroits où la priorité App-first peut diverger silencieusement.
//! C'est la classe mika#2158 : un défaut qu'aucune assertion de comportement ne
//! peut voir, parce que le comportement est identique.
//!
//! Même raison pour la moitié « les trois le reçoivent » : le compilateur garde
//! déjà le *type* du paramètre, mais rien ne garde la **valeur**. Un `None,`
//! écrit à la place de `ci_read_token.as_ref()` compile, et les ~40 sites de
//! test mécaniques de ce dépôt passent précisément un `None` là — donc la
//! régression est à une frappe de distance, elle est invisible au compilateur,
//! et sur deux des trois handlers (`ci_success`, `ci_failure`) aucun test de
//! provenance ne la verrait.
//!
//! # Ce que ce fichier ne garde pas, et où cela vit
//!
//! Il ne dit rien de la **priorité** App-first — c'est `resolve_ci_read_token`
//! qui la porte, gardée au vol 1 (mika#2963) — ni de l'identité de **post**, qui
//! reste PAT-first par `resolve_github_token` (AC3, tranché au vol 1 et non
//! relitigé ici). `merge_ready_handler` est délibérément hors de la liste : il
//! merge et relit le périmètre, il n'appelle jamais `run_gh_checks`.

/// Le site de résolution unique. `include_str!` et non une lecture au runtime :
/// le chemin est résolu à la compilation, donc un fichier déplacé casse la
/// construction au lieu de rendre le test vert sur une absence.
const HANDLERS_SRC: &str = include_str!("../../src/server/handlers.rs");

/// Les trois handlers que mika#2965 câble, nommés comme `handlers.rs` les
/// appelle. Le premier est importé, d'où l'absence de préfixe de module ; la
/// parenthèse ouvrante est ce qui distingue l'appel de l'import.
const HANDLERS_CABLES: &[&str] = &[
    "try_handle_pr_review_verdict(",
    "ci_success_handler::try_handle_ci_success(",
    "ci_failure_handler::try_handle_ci_failure(",
];

/// L'argument que chacun doit recevoir.
const ARGUMENT_ATTENDU: &str = "ci_read_token.as_ref()";

/// AC1, première moitié — la résolution a lieu **une seule fois**.
#[test]
fn mika2965_ac1_la_resolution_du_jeton_de_lecture_est_unique() {
    let n = HANDLERS_SRC.matches("resolve_ci_read_token(").count();
    assert_eq!(
        n, 1,
        "mika#2965 AC1 — `server/handlers.rs` doit résoudre le jeton de lecture CI \
         exactement une fois, et en a {n}. Une seconde résolution dans la même chaîne \
         `github` rendrait le même jeton et laisserait tout test de comportement vert, \
         en payant un second échange de jeton d'installation par événement et en \
         ouvrant un second endroit où la priorité App-first peut diverger. Si un \
         nouveau handler a besoin du jeton, passez-lui celui qui est déjà résolu — \
         n'ajoutez pas un appel (doctrine mika#2201)."
    );
}

/// AC1, seconde moitié — les trois handlers reçoivent le jeton résolu.
///
/// Le compilateur garde le type du paramètre, jamais sa valeur : un `None,` à la
/// place de l'argument compile, et c'est exactement ce que passent les sites de
/// test mécaniques de ce dépôt.
#[test]
fn mika2965_ac1_les_trois_handlers_recoivent_le_jeton_resolu() {
    for needle in HANDLERS_CABLES {
        let debut = HANDLERS_SRC.find(needle).unwrap_or_else(|| {
            panic!(
                "mika#2965 AC1 — `{needle}` est introuvable dans `server/handlers.rs`. \
                 Si le handler a été renommé, mettez ce needle à jour ; s'il a été \
                 débranché de la chaîne `github`, c'est l'énoncé d'AC1 qui a changé et \
                 ce test doit changer avec lui — pas être retiré."
            )
        });
        let reste = &HANDLERS_SRC[debut..];
        let fin = reste.find(".await").unwrap_or_else(|| {
            panic!(
                "mika#2965 — l'appel à `{needle}` doit être suivi d'un `.await` ; sans \
                 borne de fin la région scannée déborderait sur l'appel suivant et le \
                 test deviendrait vacu."
            )
        });
        let region = &reste[..fin];

        assert!(
            region.contains(ARGUMENT_ATTENDU),
            "mika#2965 AC1 — l'appel à `{needle}` doit recevoir `{ARGUMENT_ATTENDU}`, et sa \
             liste d'arguments ne le contient pas. Un `None` à cette position compile et \
             ne fait rougir aucun test : le handler retombe alors silencieusement sur le PAT, \
             qui ne peut pas lire `statusCheckRollup` (77 échecs mesurés du 2026-10-07 au \
             2026-10-29) — c'est-à-dire exactement le défaut que mika#2965 ferme.\n\n\
             région scannée :\n{region}"
        );
    }
}

/// Contrôle de bonne foi — les trois needles désignent de vrais appels.
///
/// Sans lui, un renommage silencieux rendrait le test ci-dessus vert sur zéro
/// handler scanné : `for` sur un ensemble vide ne mesure rien. Le `find` y
/// panique déjà, mais cette assertion le dit comme un énoncé plutôt que comme un
/// effet de bord, et elle garde en plus le fait que `merge_ready_handler` est
/// bien dans le fichier — donc que son absence de la liste est un choix et non
/// un oubli.
#[test]
fn mika2965_les_needles_designent_de_vrais_appels() {
    assert_eq!(
        HANDLERS_CABLES.len(),
        3,
        "AC1 nomme trois handlers ; la liste scannée doit en porter trois"
    );
    for needle in HANDLERS_CABLES {
        assert!(
            HANDLERS_SRC.contains(needle),
            "contrôle de bonne foi : `{needle}` doit être présent dans \
             `server/handlers.rs` — un needle périmé rendrait le scan d'AC1 vacu"
        );
    }
    assert!(
        HANDLERS_SRC.contains("merge_ready_handler::try_handle_merge_ready("),
        "contrôle de bonne foi : `merge_ready_handler` est câblé dans la même chaîne et \
         n'est délibérément PAS dans la liste — il merge et relit le périmètre, il \
         n'appelle jamais `run_gh_checks`. Si ce needle disparaît, c'est la chaîne qui a \
         bougé, et la liste d'AC1 doit être réexaminée plutôt que ce test ajusté."
    );
}
