//! Le geste tombe-t-il **avant** le tour LLM — mika#3022, AC1 et AC3.
//!
//! AC1 a deux moitiés. *Quoi* : un `PR closed` portant `Merged: true` et une
//! branche `<type>/<N>/…` laisse #N sans `ready` — mesuré dans le `mod tests`
//! du seam, contrôle négatif compris, sans forge ni token. *Quand* : le geste a
//! lieu **avant le tour LLM**. Cette seconde moitié est une question de
//! **callsite**, pas de calcul : elle ne se mesure pas sur une valeur rendue,
//! parce qu'un handler câblé après `webhook_prefilter::skip_turn` retirerait
//! exactement la même étiquette, avec exactement les mêmes six tests verts, et
//! l'énoncé d'AC1 serait faux sans que rien ne rougisse. C'est la classe
//! mika#2158, et seul un scan de source la voit — d'où ce fichier, sur le patron
//! de `test_merge_identity_2248.rs § ac3_lacteur_est_cable_la_ou_il_lit_le_signal`.
//!
//! Les sources sont épinglées par `include_str!`, donc les offsets lus ici sont
//! ceux de l'arbre qui compile, jamais ceux d'une copie périmée sur le disque.

/// La chaîne structurelle, et la porte du pré-filtre qu'elle doit précéder.
const HANDLERS_SRC: &str = include_str!("../../src/server/handlers.rs");

/// Le handler lui-même : l'ordre *interne* du geste s'y mesure.
const HANDLER_SRC: &str = include_str!("../../src/server/merged_pr_ready_cleanup_handler.rs");

/// Le site de câblage, tel qu'il est écrit dans la chaîne.
const WIRING_NEEDLE: &str = "merged_pr_ready_cleanup_handler::try_handle_merged_pr_ready_cleanup(";

/// La porte déterministe de mika#2675 : ce qui est après elle est après le tour.
const PREFILTER_NEEDLE: &str = "super::webhook_prefilter::skip_turn(";

/// **Allowlist née vide, et qui doit le rester.**
///
/// Une entrée ici désarmerait l'assertion d'ordre pour le site nommé. La
/// Fire-Disposition du plan de mika#3022 est explicite : si ce détecteur rougit,
/// la réponse est (c) **halt-and-escalate** sous le titre
/// `Fire-Disposition: le geste est tombé après le pré-filtre` — on **déplace le
/// câblage**, on n'ajoute pas une ligne ici. Doctrine mika#2201 : quand un
/// détecteur tire, on retire l'infraction, on n'allowliste pas.
///
/// Elle existe quand même, plutôt que de n'être qu'un commentaire, parce qu'un
/// futur éditeur pressé cherchera un endroit pour l'exception — et le trouvera
/// sous l'assertion sœur qui refuse de le laisser faire.
const WIRING_ORDER_EXCEPTIONS: &[(&str, &str)] = &[];

/// **L'assertion sœur d'auto-vacuité**, sur le patron exact de
/// `ready_label.rs § mika2995_lallowlist_de_surface_est_vide`.
///
/// Une allowlist vide est un emplacement où déposer la prochaine infraction.
/// Celle-ci rougit à la première entrée : c'est ce qui empêche le détecteur
/// d'ordre de se désarmer en silence.
#[test]
fn mika3022_lallowlist_dordre_de_cablage_est_vide() {
    assert!(
        WIRING_ORDER_EXCEPTIONS.is_empty(),
        "une exception a été ajoutée à l'ordre de câblage : {:?}. La disposition de ce \
         détecteur est de DÉPLACER le câblage avant `webhook_prefilter::skip_turn`, pas de \
         l'allowlister (mika#2201). Un geste posé après la porte du pré-filtre rend la \
         première phrase d'AC1 de mika#3022 fausse — « le geste se fait avant le tour LLM » \
         — sans qu'aucun test de comportement ne bouge.",
        WIRING_ORDER_EXCEPTIONS
    );
}

/// **AC1, seconde moitié — le geste est câblé avant le tour LLM.**
///
/// Les deux ancres sont cherchées avec `.expect` : un renommage qui ferait
/// disparaître l'une des deux doit faire rougir le détecteur, jamais le rendre
/// trivialement vert. C'est le mode de mensonge d'un scan de source, et le seul
/// moyen de le fermer est de refuser l'ancre absente.
#[test]
fn mika3022_le_geste_est_cable_avant_le_prefiltre() {
    if let Some((_, raison)) = WIRING_ORDER_EXCEPTIONS
        .iter()
        .find(|(site, _)| *site == WIRING_NEEDLE)
    {
        panic!(
            "l'ordre de câblage est excusé par une allowlist ({raison}) — voir \
             `mika3022_lallowlist_dordre_de_cablage_est_vide`"
        );
    }

    let wiring_at = HANDLERS_SRC.find(WIRING_NEEDLE).unwrap_or_else(|| {
        panic!(
            "le handler de mika#3022 doit être câblé dans la chaîne structurelle de \
             `handlers.rs` : ancre `{WIRING_NEEDLE}` introuvable. Un handler non câblé ne \
             retire rien, et ses six tests de seam restent verts."
        )
    });
    let prefilter_at = HANDLERS_SRC.find(PREFILTER_NEEDLE).unwrap_or_else(|| {
        panic!(
            "la porte du pré-filtre (mika#2675) doit être dans `handlers.rs` : ancre \
             `{PREFILTER_NEEDLE}` introuvable. Si elle a été renommée, cette ancre doit \
             suivre — pas être retirée."
        )
    });

    assert!(
        wiring_at < prefilter_at,
        "le retrait de `ready` au merge doit être câblé AVANT \
         `webhook_prefilter::skip_turn` (offsets {wiring_at} contre {prefilter_at}). \
         Après la porte, le geste tombe après le tour LLM : AC1 de mika#3022 exige \
         l'inverse, et le ticket reste exposé au feeder entre les deux."
    );
}

/// **Décider avant d'agir, et lire avant de retirer** (mika#2331).
///
/// Trois offsets dans le handler, dans cet ordre : le seam pur décide quel
/// ticket est servi, puis les étiquettes sont lues, puis — et seulement alors —
/// le retrait est posé. L'ordre est ce qui empêche un instrument de dire
/// « retiré » sur un ticket qui ne portait rien : absent n'est pas zéro, et le
/// dénominateur de la sonde d'AC4 en dépend.
#[test]
fn mika3022_le_handler_decide_et_lit_avant_de_retirer() {
    // Sur les lignes de code seules : l'ordre de trois *appels*, jamais celui de
    // trois mentions dans la doc du module. Voir [`lignes_de_code`].
    let code = lignes_de_code(HANDLER_SRC);
    let plan_at = code
        .find("plan_ready_cleanup_on_merge(")
        .expect("le seam pur `plan_ready_cleanup_on_merge` doit être dans le handler");
    let fetch_at = code
        .find("fetch_issue_labels(")
        .expect("le handler doit lire les étiquettes avant de retirer (mika#2331)");
    let remove_at = code
        .find("remove_ready_label(")
        .expect("le handler doit appeler l'unique retireur `remove_ready_label`");

    assert!(
        plan_at < remove_at,
        "la décision du seam doit précéder le geste (offsets {plan_at} contre {remove_at}) : \
         un retrait posé avant la décision est un retrait sur une population devinée"
    );
    assert!(
        fetch_at < remove_at,
        "les étiquettes doivent être lues avant le retrait (offsets {fetch_at} contre \
         {remove_at}) — mika#2331 : un instrument qui dit « retiré » sur un ticket qui ne \
         portait rien est un instrument qui ment"
    );
    assert!(
        code.contains("issue_has_ready_label(&labels)"),
        "la lecture doit être *consultée* : lire les étiquettes puis retirer sans tester la \
         présence redonne exactement le mensonge que mika#2331 nomme"
    );
}

/// **AC3 — chaque sortie sans geste porte un nom, dans la source.**
///
/// Une ligne de log se mesure mal en test unitaire : `tracing` n'émet rien sans
/// souscripteur, et un `#[traced_test]` par cas coûterait plus que ce qu'il
/// prouve. Ce que ce détecteur refuse est le défaut réel : une sortie **sans
/// nom**, donc invisible au `jq` de l'opérateur. Les trois cas qu'AC3 énumère —
/// branche hors forme, dépôt non reconnu, erreur `gh` — y sont, plus le token
/// absent et l'étiquette déjà absente, qui sont les deux autres fail-open.
#[test]
fn mika3022_chaque_sortie_sans_geste_est_nommee() {
    // Les lignes de code seules : le tableau de la doc du handler récite les
    // mêmes sept noms, et un détecteur qui s'en contenterait resterait vert sur
    // un handler devenu muet. Voir [`lignes_de_code`].
    let code = lignes_de_code(HANDLER_SRC);
    for nom in [
        "merged_pr_ready_cleanup_branch_off_form",
        "merged_pr_ready_cleanup_repo_unrecognized",
        "merged_pr_ready_cleanup_gh_failed",
        "merged_pr_ready_cleanup_no_token",
        "merged_pr_ready_cleanup_label_absent",
        "ready_label_cleaned_up_on_merge",
        "merged_pr_ready_label_cleaned",
    ] {
        assert!(
            code.contains(nom),
            "AC3 de mika#3022 : la sortie `{nom}` doit porter son nom dans la source — une \
             sortie anonyme est invisible au `jq` de l'opérateur, et la sonde d'AC4 n'a plus \
             de dénominateur"
        );
    }
    assert!(
        code.contains("step = \"issue_view\"") && code.contains("step = \"remove_label\""),
        "les deux erreurs `gh` partagent un nom d'événement, donc le champ `step` est ce qui \
         les sépare : deux populations sous un seul nom ne sont pas soustractibles (mika#2205)"
    );
}

/// Les lignes de **code** de la source, commentaires entiers retirés.
///
/// Un prédicat d'absence qui lit la prose se trompe de population : ce module
/// *nomme* `--add-label` pour dire qu'il ne l'atteint pas, et `VerdictAction::Handled`
/// pour dire qu'il ne le rend jamais. La prose **à propos** d'un drapeau n'est pas
/// un appel — c'est le signal S de mika#2050, et le repo ancre déjà ses scans sur
/// les lignes non-commentaires pour cette raison exacte
/// (`mika2425_identity_context_history_has_a_single_reader`). Le mode de mensonge
/// inverse — un commentaire de fin de ligne qui cacherait un appel — n'existe pas :
/// la troncature est sur les lignes dont le *début* est `//`.
fn lignes_de_code(src: &str) -> String {
    src.lines()
        .filter(|l| !l.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// **AC2 — aucune route d'ajout de `ready` n'est créée.**
///
/// Les gardes de voie B de mika#2995 tiennent sans entrée nouvelle dans leurs
/// allowlists, et cette assertion-ci dit pourquoi au plus près du vol : le seul
/// drapeau que ce handler atteint est `--remove-label`, par l'unique retireur du
/// voisin. Retirer, c'est rendre un consentement, jamais s'en donner un.
#[test]
fn mika3022_le_handler_ne_porte_aucune_route_dajout() {
    let code = lignes_de_code(HANDLER_SRC);
    assert!(
        !code.contains("--add-label"),
        "voie B (mika#2995) : ce handler ne doit porter aucune route d'ajout de `ready`"
    );
    assert!(
        !code.contains("apply_ready"),
        "voie B (mika#2995) : ce handler ne doit pas atteindre l'applicateur gouverné"
    );
    assert!(
        !code.contains("VerdictAction::Handled") && !code.contains("VerdictAction::Dispatched"),
        "injecteur à effet de bord seul : le handler ne doit jamais rendre `Handled` ni \
         `Dispatched` — le câblage le tient par trois `unreachable!()`, et cette assertion \
         tient l'autre bout"
    );
}
