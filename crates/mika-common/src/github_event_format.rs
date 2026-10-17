//! Shared format-prefix constants for GitHub webhook event text emitted by
//! `mika-gateway::github::format_event_text` and consumed by
//! `mika-agent::webhook_dispatch`. Single source of truth — drift between
//! producer and consumer is a contract violation; the cross-crate test in
//! `mika-gateway::github::tests` enforces the contract at CI time.

use std::borrow::Cow;

/// Prefix emitted by `format_event_text` for `issues.labeled` events
/// where the label name is `ready`. The trailing space is significant —
/// the consumer parses `<repo>#<n>` immediately after the prefix.
///
/// Producer: `mika_gateway::github::format_event_text` (`issues.labeled`
/// arm; constructs via `format!`, asserted to match this prefix in test
/// `test_format_event_text_issue_labeled_extracts_label_name`).
///
/// Consumers: `mika_agent::webhook_dispatch` (`is_ready_label_dispatch_marker`,
/// `is_unauthorized_webhook_dispatch`); `mika_agent::agent`
/// (`parse_ready_label_location`).
pub const READY_LABEL_DISPATCH_MARKER: &str = "[GitHub] Issue labeled ready on ";

/// Prefix of the **last** line `format_event_text` appends to an
/// `issues.labeled` event, naming the GitHub identity that applied the label
/// (mika#2323). Full line shape: `Labeled by: @samidarko`.
///
/// # Why this exists
///
/// mika#2323 asked whether the ready-label handler carries an actor filter.
/// It does not, and it could not: `GitHubWebhookEvent.sender` was deserialized
/// by the gateway and **never emitted**, so the agent had no identity to filter
/// on even had it wanted to. This line closes that gap so the question is
/// answerable next time without re-running the investigation.
///
/// # Hard invariant — informational only
///
/// **No refusal decision reads this value.** It is a log field and an audit
/// field; wiring it into a predicate would create exactly the actor filter
/// mika#2323 established does not exist, and which that ticket places out of
/// scope. Enforced by the source scan
/// `mika2323_no_gate_predicate_reads_the_actor` in
/// `mika_agent::server::ready_label_handler`.
///
/// # Why appending here is safe for the ready-label parse
///
/// The line is appended **after** the existing text, so
/// `starts_with(READY_LABEL_DISPATCH_MARKER)` is untouched, and
/// `parse_ready_label_location` bounds its `<repo>#<n>` token at the first
/// whitespace after the marker — a trailing line cannot reach it. Pinned by
/// `mika2323_actor_line_preserves_the_marker_and_the_parse`.
///
/// Producer: `mika_gateway::github::format_event_text` (`issues.labeled` arm).
/// Consumer: `mika_agent::server::ready_label_handler::parse_event_actor`.
pub const LABELED_BY_LINE_PREFIX: &str = "Labeled by: @";

// ---------------------------------------------------------------------------
// La grammaire d'origine, déclarée pour la taxonomie (mika#2680)
// ---------------------------------------------------------------------------
//
// Les constantes qui suivent couvrent la **totalité** des formes que
// `format_event_text` émet, y compris son bras attrape-tout. Elles existent
// pour que `mika_agent::server::webhook_prefilter::trigger_class` classe le
// texte d'origine d'un webhook **sans écrire un seul littéral de grammaire** :
// un `starts_with(CONST)` ne porte pas de littéral, donc la garde des jetons
// canoniques (mika#2201) ne voit aucun nouveau site de match à déclarer, et la
// dérive producteur/consommateur reste refusée par le test cross-crate de
// `mika-gateway::github::tests`.
//
// **Observabilité seulement — sauf deux, et cette exception est gardée.**
// `trigger_class` nomme un événement, elle n'en décide rien (même posture que
// `webhook_dispatch::fallthrough_marker_class`). Mais [`PR_EVENT_PREFIX`] et
// [`CHECK_SUITE_EVENT_PREFIX`] portent aussi, depuis mika#2680, la définition
// du **domaine Webhook Fallthrough** de `mika_agent::webhook_dispatch`, qui
// elle décide : ce module les déclarait en propre, et la garde
// `canonical_tokens::tests::mika2517_the_fallthrough_domain_has_a_single_definition`
// refuse que deux fichiers écrivent cette conjonction. Les router ici était la
// résolution que cette garde prescrit — un `use` n'écrit aucun littéral — et
// son `owner` désigne maintenant CE fichier. Conséquence à connaître avant d'y
// toucher : **changer la valeur de l'une de ces deux constantes change une
// frontière de refus**, pas seulement un nom dans un log.
//
// Les trois constantes de préfixe privées restantes (`upstream_close_handler`,
// `draft_pr_opened_handler`, `milestone_context_handler`) restent en place :
// les rerouter ici est un refactor hors périmètre, et aucune garde ne l'exige.
//
// Producteur commun : `mika_gateway::github::format_event_text`.
// Consommateurs : `mika_agent::server::webhook_prefilter::trigger_class`
// (toutes) et `mika_agent::webhook_dispatch` (les deux ci-dessus).

/// Préfixe d'un `check_suite`, **toutes conclusions confondues** (`success`,
/// `failure`, `cancelled`, …). Forme complète :
/// `[GitHub] Check suite {conclusion} on {repo} (branch: {branch})`.
///
/// Volontairement agnostique de la conclusion, contrairement à
/// `ci_success_handler::parse_check_suite_success` qui épingle `success on `.
pub const CHECK_SUITE_EVENT_PREFIX: &str = "[GitHub] Check suite ";

/// Marqueur de la branche dans un texte de `check_suite`. Forme complète :
/// ` (branch: {branch})` — la branche se lit entre ce marqueur et le `)` final.
///
/// Consommateurs : `trigger_class` (distinguer branche par défaut de branche de
/// PR). `ci_success_handler` / `ci_failure_handler` / `webhook_queue_v2` en
/// portent chacun leur copie locale ; les rerouter est hors périmètre.
pub const BRANCH_MARKER: &str = " (branch: ";

/// Préfixe générique d'un événement `pull_request`, toutes actions confondues.
/// Forme complète : `[GitHub] PR {action}: {repo}#{n} — {titre} (branch: {b})`.
///
/// **À tester après** [`PR_REVIEW_EVENT_PREFIX`] : `PR review ` commence par
/// `PR `, l'ordre des bras décide.
pub const PR_EVENT_PREFIX: &str = "[GitHub] PR ";

/// Préfixe d'un `pull_request_review`. Forme complète :
/// `[GitHub] PR review ({state}) on {repo}#{n} ({titre}) by @{relecteur}`.
pub const PR_REVIEW_EVENT_PREFIX: &str = "[GitHub] PR review ";

/// Préfixe de l'action `opened` d'une PR.
pub const PR_OPENED_EVENT_PREFIX: &str = "[GitHub] PR opened:";

/// Préfixe de l'action `closed` d'une PR.
///
/// **Ne distingue pas mergée de fermée** : le gateway rend `PR closed:` dans
/// les deux cas et n'ajoute `Merged: {bool}` qu'en ligne suivante. La taxonomie
/// de mika#2680 ne le prétend pas et rend `pr_closed` dans les deux cas.
pub const PR_CLOSED_EVENT_PREFIX: &str = "[GitHub] PR closed:";

/// Préfixe générique d'un événement `issues`, toutes actions confondues.
/// Forme complète : `[GitHub] Issue {action}: {repo}#{n} — {titre}`.
///
/// **À tester après** [`ISSUE_LABELED_EVENT_PREFIX`] : le bras `labeled` du
/// gateway n'émet pas la forme `Issue {action}:`, mais `Issue labeled {label}
/// on …`, qui commence pourtant par `Issue `.
pub const ISSUE_EVENT_PREFIX: &str = "[GitHub] Issue ";

/// Préfixe d'un `issues.labeled`, **quel que soit le label**. Forme complète :
/// `[GitHub] Issue labeled {label} on {repo}#{n} — {titre}`.
///
/// [`READY_LABEL_DISPATCH_MARKER`] en est l'instance `ready`, celle qui porte
/// une décision ; celle-ci n'en porte aucune.
pub const ISSUE_LABELED_EVENT_PREFIX: &str = "[GitHub] Issue labeled ";

/// Préfixe de l'action `opened` d'un ticket — instance canonique de la forme
/// générique `Issue {action}:`, et la seule par laquelle un test peut épingler
/// ce gabarit : `starts_with(ISSUE_EVENT_PREFIX)` passerait aussi sur la forme
/// `labeled` et ne prouverait donc rien.
///
/// Consommateurs : le test cross-crate de `mika-gateway::github::tests` et le
/// test d'exhaustivité de `trigger_class` (AC3 de mika#2680). La taxonomie n'a
/// **pas** de terme `issue_opened` : cette forme retombe sur `issue_other`.
pub const ISSUE_OPENED_EVENT_PREFIX: &str = "[GitHub] Issue opened:";

/// Préfixe de l'action `closed` d'un ticket.
pub const ISSUE_CLOSED_EVENT_PREFIX: &str = "[GitHub] Issue closed:";

/// Préfixe d'un `issue_comment`. Forme complète :
/// `[GitHub] New comment on {repo}#{n} ({titre}) by @{auteur}`.
pub const ISSUE_COMMENT_EVENT_PREFIX: &str = "[GitHub] New comment on ";

/// Préfixe d'un `push`, tel qu'il arrive **réellement** au moteur.
///
/// `format_event_text` n'a **aucun bras `push`** : un `push` tombe dans son
/// attrape-tout `[GitHub] {event_type}.{action} on {repo}`, et `action` y vaut
/// `"unknown"` par défaut. Le texte est donc `[GitHub] push.unknown on {repo}`,
/// d'où un préfixe qui s'arrête au point. Épingler `[GitHub] push: ` aurait
/// décrit une forme que rien n'émet.
pub const PUSH_EVENT_PREFIX: &str = "[GitHub] push.";

// ---------------------------------------------------------------------------
// Les données citées ne sont pas du texte d'intention (mika#2755)
// ---------------------------------------------------------------------------

/// Préfixe commun à **toutes** les formes que `format_event_text` émet — le
/// dénominateur de la grammaire déclarée ci-dessus, bras attrape-tout inclus.
///
/// Son unique rôle : décider si un texte est un événement webhook **avant** d'y
/// toucher. Il ne nomme aucun type d'événement et ne porte aucune décision de
/// routage — pour cela, les préfixes spécifiques ci-dessus sont les seuls
/// légitimes.
///
/// Producteur : `mika_gateway::github::format_event_text` (tous les bras).
/// Consommateur : [`mask_cited_branch`].
pub const GITHUB_EVENT_TEXT_PREFIX: &str = "[GitHub] ";

/// Retire le slug de branche qu'un texte d'événement GitHub **cite**, en
/// laissant le marqueur en place : `… (branch: feat/x)` devient `… (branch: )`.
///
/// # Pourquoi
///
/// Le 2026-10-16, un `check_suite` vert sur
/// `fix/2750/dispatch-un-groom-ready-label-qui-crashe` a fait matcher le
/// mot-clé `groom` de dev-groom **dans le slug de la branche** — les deux
/// ancres `\b` de mika#1878 y sont satisfaites, les voisins étant des traits
/// d'union. Le `required_tools` du skill s'est armé, la boucle a re-promptée,
/// et le modèle a fabriqué une tâche manuelle fantôme (mika#2755). Un nom de
/// branche est une **donnée citée** par le substrat, jamais une intention
/// exprimée par quelqu'un.
///
/// # Contrat — trois refus, tous fail-safe
///
/// Renvoie `Cow::Borrowed(text)` **inchangé** quand :
///
/// 1. `text` ne commence pas par [`GITHUB_EVENT_TEXT_PREFIX`] — ce n'est pas un
///    événement webhook, donc rien n'y est « cité » ;
/// 2. la **première ligne** ne contient pas [`BRANCH_MARKER`] — aucune branche
///    citée (`issue_comment`, `issues`, `pull_request_review`, `push`) ;
/// 3. la première ligne ne finit pas par `)` — la forme attendue n'est pas là,
///    et un masquage à l'aveugle couperait du texte au hasard.
///
/// # `find`, pas `rfind` — et c'est délibéré
///
/// La coupe part de la **première** occurrence du marqueur. Un titre de PR
/// contenant ` (branch: ` fait donc manger le titre aussi. C'est le sens du
/// fail-safe : masquer plus de données citées, jamais moins. Épinglé par
/// `mask_cited_branch_eats_a_title_that_imitates_the_marker`, pour que ce soit
/// une propriété choisie et non une surprise.
///
/// # Portée
///
/// Le texte masqué sert **uniquement** à l'évaluation des mots-clés de skill
/// (`mika_agent::skills::matcher::match_skills`). `context::resolve_contexts`,
/// `review_filter::apply_review_filter`, les `INTENT_GUARDS` et le prompt lisent
/// tous `params.user_message` intact : le modèle voit la branche, c'est la
/// *détection d'intention* qui ne la lit plus.
pub fn mask_cited_branch(text: &str) -> Cow<'_, str> {
    if !text.starts_with(GITHUB_EVENT_TEXT_PREFIX) {
        return Cow::Borrowed(text);
    }
    // Même discipline « première ligne » que `webhook_prefilter::trigger_class` :
    // le corps d'un ticket ou d'une review suit, et n'a rien à voir ici.
    let head = text.lines().next().unwrap_or(text);
    let Some(at) = head.find(BRANCH_MARKER) else {
        return Cow::Borrowed(text);
    };
    if head.strip_suffix(')').is_none() {
        return Cow::Borrowed(text);
    }
    // Offsets prouvés sur frontière de caractère : `find` en renvoie une, et
    // `BRANCH_MARKER` comme `head` sont mesurés en octets sur `text` lui-même.
    let kept = at + BRANCH_MARKER.len();
    Cow::Owned(format!("{}){}", &text[..kept], &text[head.len()..]))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Message du webhook fondateur de mika#2755, **mot pour mot**.
    const TICKET_MESSAGE: &str = "[GitHub] Check suite success on senara-solutions/mika (branch: fix/2750/dispatch-un-groom-ready-label-qui-crashe)";

    /// Seul assembleur de message de ce module (garde-fou du plan : sept cas,
    /// **un** assembleur). Les quatre cas de refus portent leur littéral, qui
    /// est précisément la forme qu'ils épinglent.
    fn check_suite_text(branch: &str) -> String {
        format!("[GitHub] Check suite success on senara-solutions/mika (branch: {branch})")
    }

    #[test]
    fn mask_cited_branch_removes_the_slug_of_the_founding_webhook() {
        let text = check_suite_text("fix/2750/dispatch-un-groom-ready-label-qui-crashe");
        assert_eq!(
            text, TICKET_MESSAGE,
            "l'assembleur doit rendre le message du ticket mot pour mot"
        );

        let masked = mask_cited_branch(&text);

        assert_eq!(
            masked.as_ref(),
            "[GitHub] Check suite success on senara-solutions/mika (branch: )",
            "le marqueur reste, seul le slug part"
        );
        assert!(
            !masked.contains("groom"),
            "c'est le défaut même de mika#2755 : {masked}"
        );
    }

    #[test]
    fn mask_cited_branch_keeps_the_url_of_a_pull_request_event() {
        let text = "[GitHub] PR opened: senara-solutions/mika#2754 — un groom orphelin (branch: fix/2750/dispatch-un-groom-ready-label-qui-crashe)\nhttps://github.com/senara-solutions/mika/pull/2754";

        let masked = mask_cited_branch(text);

        assert_eq!(
            masked.as_ref(),
            "[GitHub] PR opened: senara-solutions/mika#2754 — un groom orphelin (branch: )\nhttps://github.com/senara-solutions/mika/pull/2754",
            "la coupe est bornée à la première ligne : l'URL survit intacte"
        );
    }

    #[test]
    fn mask_cited_branch_refuses_a_human_message() {
        // Refus 1 : pas de préfixe d'événement. Un humain a le droit d'écrire
        // une phrase de cette forme, et son intention doit rester lisible.
        let text = "groom mika#2750 (branch: fix/2750/dispatch-un-groom-ready-label-qui-crashe)";

        let masked = mask_cited_branch(text);

        assert!(
            matches!(masked, Cow::Borrowed(_)),
            "un texte humain doit être emprunté inchangé"
        );
        assert_eq!(masked.as_ref(), text);
    }

    #[test]
    fn mask_cited_branch_refuses_an_event_without_the_marker() {
        // Refus 2 : un événement qui ne cite aucune branche.
        let text = "[GitHub] New comment on senara-solutions/mika#2750 (un groom orphelin) by @samidarko\nhttps://example.invalid/c/1";

        let masked = mask_cited_branch(text);

        assert!(
            matches!(masked, Cow::Borrowed(_)),
            "sans BRANCH_MARKER il n'y a rien à masquer"
        );
        assert_eq!(masked.as_ref(), text);
    }

    #[test]
    fn mask_cited_branch_refuses_a_first_line_not_closed_by_a_parenthesis() {
        // Refus 3 : le marqueur est là, la forme attendue ne l'est pas.
        let text = "[GitHub] Check suite success on senara-solutions/mika (branch: fix/2750/groom";

        let masked = mask_cited_branch(text);

        assert!(
            matches!(masked, Cow::Borrowed(_)),
            "forme inattendue : on ne coupe pas à l'aveugle"
        );
        assert_eq!(masked.as_ref(), text);
    }

    #[test]
    fn mask_cited_branch_eats_a_title_that_imitates_the_marker() {
        // Le fail-safe de `find`, épinglé plutôt que subi : un titre de PR qui
        // contient lui-même ` (branch: ` est mangé avec le slug. Masquer plus
        // de données citées est le bon sens de l'erreur.
        let text = "[GitHub] PR opened: senara-solutions/mika#2755 — lire (branch: x) comme une intention (branch: fix/2755/groom-dans-un-nom)\nhttps://example.invalid/pull/2755";

        let masked = mask_cited_branch(text);

        assert_eq!(
            masked.as_ref(),
            "[GitHub] PR opened: senara-solutions/mika#2755 — lire (branch: )\nhttps://example.invalid/pull/2755",
            "la coupe part de la PREMIÈRE occurrence du marqueur"
        );
    }

    #[test]
    fn mask_cited_branch_survives_a_multi_byte_slug() {
        // mika#764 : aucun offset ne doit tomber au milieu d'un caractère.
        let text = check_suite_text("fix/2755/réparé-le-groom-à-côté-☕");

        let masked = mask_cited_branch(&text);

        assert_eq!(
            masked.as_ref(),
            "[GitHub] Check suite success on senara-solutions/mika (branch: )",
            "slug multi-octets retiré sans coupe en milieu de caractère"
        );
    }
}
