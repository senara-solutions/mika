//! Shared format-prefix constants for GitHub webhook event text emitted by
//! `mika-gateway::github::format_event_text` and consumed by
//! `mika-agent::webhook_dispatch`. Single source of truth — drift between
//! producer and consumer is a contract violation; the cross-crate test in
//! `mika-gateway::github::tests` enforces the contract at CI time.

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
