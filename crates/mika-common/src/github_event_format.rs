//! Shared format-prefix constants for GitHub webhook event text emitted by
//! `mika-gateway::github::format_event_text` and consumed by
//! `mika-agent::webhook_dispatch`. Single source of truth — drift between
//! producer and consumer is a contract violation; the cross-crate test in
//! `mika-gateway::github::tests` enforces the contract at CI time.

use crate::forge_identity::MERGE_READY_BRANCH_MARKER;
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
// La tête d'un `check_suite`, transmise au lieu d'être jetée (mika#2917)
// ---------------------------------------------------------------------------
//
// Jusqu'à mika#2917, `format_event_text` jetait `check_suite.head_sha` et
// `check_suite.pull_requests[]` : son texte ne nommait donc **aucune tête**, et
// la seule façon pour le moteur d'apprendre de quelle tête l'événement parlait
// était un appel `gh` — que le bearing de mika#2680 refuse (mika#2260). Les
// deux lignes déclarées ci-dessous lèvent cet obstacle matériel.
//
// **Lignes 2 et 3, jamais la ligne 1, et c'est une contrainte.** Quatre
// lecteurs du texte `check_suite` existent, et les quatre ne lisent que la
// PREMIÈRE ligne, qu'ils terminent sur son `)` :
// `ci_success_handler::parse_check_suite_success`,
// `ci_failure_handler::parse_check_suite_failure`,
// `webhook_queue::CHECK_SUITE_RE` (mika#528) et `webhook_queue_v2::CHECK_SUITE_RE`.
// Aucun n'est ancré en fin de texte, aucun n'utilise `(?m)`, et leur `[^)]+`
// s'arrête au `)`. Ajouter des lignes **après** la première les laisse donc
// tous intacts — et c'est ce qu'épingle
// `webhook_prefilter::tests::mika2917_les_quatre_lecteurs_lisent_lancien_et_le_nouveau_texte`.
//
// Producteur : `mika_gateway::github::format_event_text` (bras `check_suite`).
// Consommateur : [`parse_check_suite_heads`], juste en dessous.

/// Deuxième ligne d'un texte de `check_suite` : la tête de la suite.
/// Forme complète : `Head-Sha: {sha}`.
///
/// `unknown` quand le payload n'en portait pas — même modèle que `conclusion`
/// et `branch` sur la première ligne : le gateway n'invente pas, et le
/// consommateur traite `unknown` comme une tête non nommée.
pub const CHECK_SUITE_HEAD_LINE_PREFIX: &str = "Head-Sha: ";

/// Troisième ligne : les PR que la suite nomme, `{n}@{sha}` séparés par un
/// espace, ou [`CHECK_SUITE_PRS_NONE`] quand aucune n'est nommable.
/// Forme complète : `Check-Suite-PRs: 2917@1a2b3c 2918@9f8e7d`.
pub const CHECK_SUITE_PRS_LINE_PREFIX: &str = "Check-Suite-PRs: ";

/// Le payload ne nommait **aucune** tête de PR — ce qui n'est pas la même
/// chose que la ligne absente (mika#2340).
///
/// Sa cause nominale est un tableau `pull_requests` présent et **vide**, l'état
/// mesuré sur mika#2681. Elle couvre aussi le cas dégénéré d'un tableau non
/// vide dont aucune entrée ne porte de `head.sha` : une PR sans tête n'est pas
/// une PR nommée, et le producteur rend donc `none` là aussi plutôt que
/// d'émettre une valeur vide. **Une seule population sous ce nom** — « le
/// payload ne nomme aucune tête » — déclarée identiquement aux deux bouts, et
/// non deux populations additionnées (mika#2205).
pub const CHECK_SUITE_PRS_NONE: &str = "none";

/// Ce que le texte dit des têtes de la suite. Quatre valeurs, et aucune ne se
/// déduit d'une autre.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckSuiteHeads {
    /// La ligne [`CHECK_SUITE_PRS_LINE_PREFIX`] est absente : binaire d'avant
    /// mika#2917 (classe mika#2340). **Jamais confondu avec [`Self::Empty`]** :
    /// l'absence nomme un régime de déploiement, `Empty` un comportement de la
    /// forge, et les confondre, c'est additionner deux populations sous un nom
    /// (mika#2205).
    LineAbsent,
    /// La ligne est là et ne nomme aucune tête — voir [`CHECK_SUITE_PRS_NONE`].
    Empty,
    /// Exactement une PR, avec son `head.sha`. Le seul cas qu'un décideur
    /// pourra comparer.
    One { pr: u64, head_sha: String },
    /// Plusieurs PR : le payload ne nomme pas LA tête de l'événement.
    Many(usize),
}

/// Lit les têtes qu'un texte de `check_suite` nomme. Fonction **pure** : aucun
/// état, aucun réseau, aucune allocation hors le `sha` retenu.
///
/// # Ce qu'elle ne fait pas
///
/// Elle ne décide rien, et au vol 1 de mika#2917 **aucun décideur ne la lit** :
/// `webhook_prefilter::classify` transporte sa valeur pour que le vol 3 n'ait
/// qu'un décideur à écrire, et sa présence ne change aucune porte.
///
/// # Un jeton illisible est sauté, pas promu en erreur
///
/// Numéro non numérique, `@` absent, `sha` vide : le jeton est ignoré. Le texte
/// vient du gateway et non d'un utilisateur, donc un jeton qu'on ne sait pas
/// lire doit se comporter comme si le payload n'avait pas nommé cette PR —
/// fail-safe, dans le sens où le tour a lieu.
///
/// # Aucun découpage d'octets
///
/// `strip_prefix`, `split_whitespace` et `split_once` seulement : un `sha` est
/// de l'ASCII, mais rien ne garantit que le reste de la ligne le soit, et
/// `scripts/check-byte-slices.sh` (mika#764) refuse la forme indexée.
pub fn parse_check_suite_heads(text: &str) -> CheckSuiteHeads {
    let Some(value) = text
        .lines()
        .find_map(|line| line.strip_prefix(CHECK_SUITE_PRS_LINE_PREFIX))
    else {
        return CheckSuiteHeads::LineAbsent;
    };

    let value = value.trim();
    if value == CHECK_SUITE_PRS_NONE {
        return CheckSuiteHeads::Empty;
    }

    let mut heads: Vec<(u64, &str)> = Vec::new();
    for token in value.split_whitespace() {
        let Some((number, sha)) = token.split_once('@') else {
            continue;
        };
        let Ok(pr) = number.parse::<u64>() else {
            continue;
        };
        if sha.is_empty() {
            continue;
        }
        heads.push((pr, sha));
    }

    match heads.as_slice() {
        // Ligne présente, rien de nommable : la même valeur que `none`, parce
        // que c'est la même chose dite du payload. Inatteignable depuis un
        // gateway conforme, qui écrit `none` dans ce cas.
        [] => CheckSuiteHeads::Empty,
        [(pr, head_sha)] => CheckSuiteHeads::One {
            pr: *pr,
            head_sha: (*head_sha).to_string(),
        },
        many => CheckSuiteHeads::Many(many.len()),
    }
}

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

/// Suffixe de l'identifiant d'une balise d'enveloppe de gestionnaire.
///
/// Déclaré ici plutôt qu'écrit en littéral dans
/// [`is_handler_envelope_open_tag`] pour que la **forme** soit le seul critère :
/// aucun nom de gestionnaire n'apparaît dans le code de production du prédicat.
const HANDLER_ENVELOPE_TAG_SUFFIX: &str = "_handler";

/// Cette ligne est-elle la balise **ouvrante** d'une enveloppe de gestionnaire ?
///
/// Reconnue **sur sa forme** — `<` + identifiant `[a-z0-9_]+` finissant par
/// `_handler` + `>` — et jamais sur une liste de noms. C'est la doctrine de
/// `landing-tokens-lint` (mika#1804) : la régression réaliste n'est pas de
/// réécrire `<ci_success_handler>`, c'est d'ajouter un huitième gestionnaire et
/// d'oublier la liste. Un espacement autour de la balise est toléré ; tout le
/// reste est refusé, y compris la balise **fermante** (`/` n'est pas un
/// caractère d'identifiant).
///
/// Consommateur : [`mask_cited_branch`], par son prédicat d'engagement.
pub fn is_handler_envelope_open_tag(line: &str) -> bool {
    let Some(inner) = line
        .trim()
        .strip_prefix('<')
        .and_then(|rest| rest.strip_suffix('>'))
    else {
        return false;
    };
    let Some(stem) = inner.strip_suffix(HANDLER_ENVELOPE_TAG_SUFFIX) else {
        return false;
    };
    // `<_handler>` n'est pas un identifiant de gestionnaire : il n'en nomme
    // aucun.
    !stem.is_empty()
        && inner
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

/// Ce texte cite-t-il un événement GitHub ? — prédicat d'**engagement** du
/// masque (mika#2768).
///
/// Vrai si *au moins une* ligne commence par [`GITHUB_EVENT_TEXT_PREFIX`],
/// **ou** si la première ligne est une balise ouvrante de gestionnaire
/// ([`is_handler_envelope_open_tag`]).
///
/// La seconde clause est une **défense en profondeur**, pas le mécanisme
/// principal : tous les pré-digests recensés portent aussi une ligne
/// `[GitHub] `, donc c'est la clause par ligne qui décide pour eux. Elle couvre
/// l'enveloppe qui citerait un slug en prose sans porter le gabarit, et son
/// test dédié est ce qui l'empêche d'être du code mort.
fn cites_a_github_event(text: &str) -> bool {
    text.lines()
        .any(|line| line.starts_with(GITHUB_EVENT_TEXT_PREFIX))
        || text
            .lines()
            .next()
            .is_some_and(is_handler_envelope_open_tag)
}

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
/// # Ce que mika#2768 a changé — à lire avant de relire le vol 1
///
/// Le vol 1 n'engageait le masque que si la **première ligne** portait le
/// gabarit, et ne coupait qu'**une** occurrence. Un pré-digest enveloppé
/// (`<ci_success_handler>\n[GitHub] …`) échappait donc entier : le 2026-10-17 à
/// 08:59:18Z, le `groom` du slug `fix/2762/rollback-reaper-groom` a armé
/// `required_tools`, mika-dev a groomé mika#2766 — un ticket **sans** label
/// `ready`, donc hors du consentement positif de mika#841 — et a failli en
/// dispatcher l'implémentation. Trois propriétés documentées du vol 1 ont
/// changé, et aucune n'est un accident :
///
/// 1. **L'engagement est par ligne, plus par première ligne.** Un texte dont
///    *une* ligne commence par [`GITHUB_EVENT_TEXT_PREFIX`] est engagé, où que
///    cette ligne soit : derrière une enveloppe de gestionnaire, derrière un
///    enrichissement préfixé. Conséquence assumée : un humain qui **recopie**
///    une ligne d'événement cite lui aussi une donnée, et son slug est masqué.
/// 2. **Le refus de forme est par occurrence, plus global.** Une occurrence du
///    marqueur sans `)` sur sa ligne est **sautée** — on ne coupe pas à
///    l'aveugle — sans empêcher les autres d'être masquées.
/// 3. **« `find`, pas `rfind` » a disparu, et avec elle « un titre qui imite le
///    marqueur est mangé ».** Chaque occurrence étant traitée, tous les slugs
///    partent *et* le texte entre eux survit. La raison fail-safe de manger le
///    titre était qu'on ne savait couper qu'une fois. Épinglé par
///    `mask_cited_branch_masks_every_run_and_keeps_the_title_between_them` —
///    sans ce test, un relecteur lirait une régression du vol 1.
///
/// # Deux grammaires de citation, et pourquoi la seconde a failli être oubliée
///
/// Un pré-digest cite sa branche de **deux** façons, et le masque doit vider
/// les deux :
///
/// | marqueur | fin de valeur | producteur |
/// |---|---|---|
/// | [`BRANCH_MARKER`] (` (branch: `) | le `)` de la ligne | `format_event_text` du gateway |
/// | [`MERGE_READY_BRANCH_MARKER`] (` branch=`) | la prochaine espace | `MergeReadySignal` (ligne `MERGE-READY-SIGNAL:`) |
///
/// L'extrait du ticket mika#2768 élidait la ligne du signal derrière un `…`, si
/// bien que ce vol 2 a d'abord été écrit pour la première forme seule. Le test
/// de l'AC2 — celui qui fait produire le pré-digest par
/// `format_merge_ready_pre_digest` au lieu de recopier un littéral — est ce qui
/// a montré que le `groom` de `fix/2762/rollback-reaper-groom` sortait encore
/// par la seconde, et donc que le correctif aurait été **inerte sur l'incident
/// fondateur**. C'est l'argument du « producteur réel, pas un littéral », payé
/// comptant.
///
/// # Contrat — deux refus, tous deux fail-safe
///
/// Renvoie `Cow::Borrowed(text)` **inchangé** quand :
///
/// 1. [`cites_a_github_event`] est faux — aucune ligne d'événement, aucune
///    enveloppe de gestionnaire : rien n'y est « cité », et un message humain
///    est le seul canal par lequel un opérateur exprime une intention en texte
///    libre ;
/// 2. aucune occurrence d'aucun des deux marqueurs n'était masquable — texte
///    sans marqueur (`issue_comment`, `issues`, `pull_request_review`, `push`),
///    valeurs déjà vides, ou occurrences toutes de forme inattendue.
///
/// # Portée
///
/// Le texte masqué sert **uniquement** à l'évaluation des mots-clés de skill
/// (`mika_agent::skills::matcher::match_skills`). `context::resolve_contexts`,
/// `review_filter::apply_review_filter`, les `INTENT_GUARDS` et le prompt lisent
/// tous `params.user_message` intact : le modèle voit la branche, c'est la
/// *détection d'intention* qui ne la lit plus.
pub fn mask_cited_branch(text: &str) -> Cow<'_, str> {
    if !cites_a_github_event(text) {
        return Cow::Borrowed(text);
    }
    // Les deux passes sont indépendantes : chacune emprunte quand elle n'a rien
    // trouvé, de sorte qu'un texte sans aucun slug masquable reste `Borrowed`.
    match mask_marker(text, BRANCH_MARKER, ValueEnd::ClosingParen) {
        Cow::Borrowed(_) => mask_marker(text, MERGE_READY_BRANCH_MARKER, ValueEnd::Whitespace),
        Cow::Owned(once) => {
            match mask_marker(&once, MERGE_READY_BRANCH_MARKER, ValueEnd::Whitespace) {
                Cow::Borrowed(_) => Cow::Owned(once),
                Cow::Owned(twice) => Cow::Owned(twice),
            }
        }
    }
}

/// Où se termine la valeur citée, pour un marqueur donné.
///
/// Les deux formes ont la même borne haute — la fin de **leur** ligne — et
/// diffèrent sur ce qui clôt la valeur à l'intérieur.
#[derive(Debug, Clone, Copy)]
enum ValueEnd {
    /// `… (branch: {slug})` : la valeur va jusqu'au `)`. Son absence sur la
    /// ligne est une forme inattendue, et l'occurrence est sautée.
    ClosingParen,
    /// `… branch={slug} task=…` : la valeur va jusqu'à la prochaine espace, ou
    /// jusqu'au bout de la ligne quand le champ la termine. Il n'y a donc pas
    /// de forme inattendue ici — seulement le cas « champ vide », sauté parce
    /// qu'il n'a rien à masquer.
    Whitespace,
}

/// Vide la valeur de chaque occurrence de `marker`, en laissant le marqueur.
///
/// Tous les offsets sont prouvés sur frontière de caractère (mika#764) : `find`
/// en rend une, `marker.len()` ajouté à une position où le marqueur a matché en
/// est une, `line.len()` est la frontière où `'\n'` a été trouvé, et `')'`,
/// `'\n'` et les espaces ASCII le sont aussi. Un slug multi-octets ne peut donc
/// pas être coupé en milieu de caractère.
fn mask_marker<'a>(text: &'a str, marker: &str, value_end: ValueEnd) -> Cow<'a, str> {
    let mut out = String::new();
    let mut last = 0usize;
    let mut cursor = 0usize;
    let mut masked_any = false;

    while let Some(rel) = text[cursor..].find(marker) {
        let after = cursor + rel + marker.len();
        // La coupe est bornée à la ligne de l'occurrence : la ligne suivante
        // (une URL de PR, la prose d'un enrichissement) n'a rien à voir ici.
        let line_end = text[after..].find('\n').map_or(text.len(), |nl| after + nl);
        let line = &text[after..line_end];
        let close = match value_end {
            ValueEnd::ClosingParen => line.find(')'),
            ValueEnd::Whitespace => Some(line.find(char::is_whitespace).unwrap_or(line.len())),
        };
        match close {
            // Une valeur non vide : on la retire.
            Some(rel_close) if rel_close > 0 => {
                let close = after + rel_close;
                out.push_str(&text[last..after]);
                masked_any = true;
                last = close;
                cursor = close;
            }
            // Valeur déjà vide, ou forme inattendue : on ne coupe pas à
            // l'aveugle et on passe à l'occurrence suivante.
            _ => cursor = after,
        }
    }

    if !masked_any {
        return Cow::Borrowed(text);
    }
    out.push_str(&text[last..]);
    Cow::Owned(out)
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
    fn mask_cited_branch_masks_every_run_and_keeps_the_title_between_them() {
        // Ce test épinglait l'inverse au vol 1 (mika#2755) : il s'appelait
        // `…_eats_a_title_that_imitates_the_marker` et exigeait `… — lire
        // (branch: )`, titre tronqué. Ce n'était pas une préférence, c'était la
        // conséquence de ne savoir couper qu'UNE fois : manger le titre était le
        // bon sens de l'erreur, puisque masquer plus de données citées vaut
        // mieux que moins.
        //
        // Le masquage par occurrence (mika#2768) retire cette contrainte : tous
        // les slugs partent ET le texte entre eux survit. Ce n'est donc pas une
        // régression du vol 1, c'est sa propriété fail-safe devenue inutile.
        let text = "[GitHub] PR opened: senara-solutions/mika#2755 — lire (branch: x) comme une intention (branch: fix/2755/groom-dans-un-nom)\nhttps://example.invalid/pull/2755";

        let masked = mask_cited_branch(text);

        assert_eq!(
            masked.as_ref(),
            "[GitHub] PR opened: senara-solutions/mika#2755 — lire (branch: ) comme une intention (branch: )\nhttps://example.invalid/pull/2755",
            "chaque occurrence est masquée, et le titre entre les deux survit"
        );
    }

    #[test]
    fn mika2768_masks_the_slug_of_the_wrapped_pre_digest_word_for_word() {
        // Le message reçu par le tour mika-dev du 2026-10-17 08:59:18Z, mot
        // pour mika#2768. C'est le vol 1 qui le laissait passer entier : son
        // `starts_with` portait sur la PREMIÈRE ligne, et celle-ci est la
        // balise ouvrante du gestionnaire.
        let text = "<ci_success_handler>\n\
                    [GitHub] Check suite success on senara-solutions/mika#2765 (branch: fix/2762/rollback-reaper-groom)\n\
                    CI checks all green + VERDICT: pass from @mika-platform-qa — every gate cleared for head abc123.\n\
                    </ci_success_handler>";

        let masked = mask_cited_branch(text);

        assert!(
            !masked.contains("groom"),
            "c'est ce `groom`-là, dans le slug, qui a armé `required_tools` de \
             dev-groom et fait groomer mika#2766 sans label `ready` : {masked}"
        );
        assert!(
            masked.contains("senara-solutions/mika#2765 (branch: )"),
            "le marqueur reste en place, seul le slug part : {masked}"
        );
        assert!(
            masked.contains("</ci_success_handler>"),
            "l'enveloppe traverse intacte : {masked}"
        );
    }

    #[test]
    fn mika2768_masks_a_prefixed_enrichment_without_eating_the_sentence() {
        // Chemin 3 : un enrichissement de `Passthrough` cite le slug dans sa
        // prose, puis le texte brut du webhook suit. La coupe doit s'arrêter au
        // `)` — la phrase qui vient après porte la consigne que le modèle lit.
        let text = "[ci_failure_handler] CI failure on senara-solutions/mika#2765 (branch: fix/2762/rollback-reaper-groom). \
                    A fix is already in-flight for task t-42. Wait for the callback to finish before taking further action.\n\n\
                    [GitHub] Check suite failure on senara-solutions/mika#2765 (branch: fix/2762/rollback-reaper-groom)";

        let masked = mask_cited_branch(text);

        assert!(
            !masked.contains("groom"),
            "les deux occurrences doivent partir, celle de la prose comme celle \
             du gabarit : {masked}"
        );
        assert!(
            masked.contains("(branch: ). A fix is already in-flight for task t-42."),
            "la phrase qui suit le `)` survit intacte : {masked}"
        );
    }

    #[test]
    fn mika2768_masks_an_envelope_that_cites_a_slug_only_in_prose() {
        // Le seul cas où la clause « enveloppe » décide seule : aucune ligne ne
        // porte `[GitHub] `. Sans ce test, cette clause serait du code mort non
        // testé (risque nommé au § 11 du plan).
        let text = "<verdict_handler>\n\
                    Le correctif en vol vise (branch: fix/2762/rollback-reaper-groom) et rien d'autre.\n\
                    </verdict_handler>";

        let masked = mask_cited_branch(text);

        assert!(
            !masked.contains("groom"),
            "l'enveloppe engage le masque à elle seule : {masked}"
        );
        assert!(
            masked.contains("vise (branch: ) et rien d'autre."),
            "la prose autour du marqueur survit : {masked}"
        );
    }

    #[test]
    fn mika2768_skips_an_unclosed_occurrence_without_sparing_its_neighbour() {
        // Le refus de forme du vol 1 était global : une occurrence sans `)`
        // faisait refuser le texte ENTIER, donc laissait passer les slugs des
        // autres occurrences. Il est désormais par occurrence.
        let text = "[GitHub] PR opened: senara-solutions/mika#2766 (branch: fix/x/groom-un) — suite (branch: fix/y/groom-deux";

        let masked = mask_cited_branch(text);

        assert_eq!(
            masked.as_ref(),
            "[GitHub] PR opened: senara-solutions/mika#2766 (branch: ) — suite (branch: fix/y/groom-deux",
            "la première occurrence est masquée ; la seconde, sans `)` sur sa \
             ligne, est laissée intacte plutôt que coupée à l'aveugle"
        );
    }

    #[test]
    fn mika2768_borrows_an_engaged_text_with_nothing_maskable() {
        // Discipline d'emprunt : engagé n'est pas masqué. Rien n'a bougé, donc
        // rien n'est alloué.
        let text = "<ci_failure_handler>\nAucune branche n'est citée ici.\n</ci_failure_handler>";

        let masked = mask_cited_branch(text);

        assert!(
            matches!(masked, Cow::Borrowed(_)),
            "aucune occurrence masquée : le texte doit être emprunté"
        );
        assert_eq!(masked.as_ref(), text);
    }

    #[test]
    fn mika2768_the_envelope_is_recognised_by_shape_not_by_name() {
        for tag in [
            "<ci_success_handler>",
            "<ci_failure_handler>",
            "<verdict_handler>",
            "<ready_label_handler>",
            // Nom inventé : c'est lui qui prouve qu'aucune liste n'est tenue.
            "<zorglub8_handler>",
        ] {
            assert!(
                is_handler_envelope_open_tag(tag),
                "forme valide refusée : {tag:?} — un huitième gestionnaire doit \
                 être reconnu le jour où il est écrit, sans liste à tenir"
            );
        }

        for not_a_tag in [
            "<html>",
            "</ci_success_handler>",
            "<not-a-handler>",
            "<_handler>",
            "[GitHub] Check suite success on senara-solutions/mika (branch: x)",
            "<ci_success_handler",
            "<CiSuccessHandler>",
        ] {
            assert!(
                !is_handler_envelope_open_tag(not_a_tag),
                "forme invalide acceptée : {not_a_tag:?}"
            );
        }
    }

    // -----------------------------------------------------------------------
    // La tête d'un `check_suite` (mika#2917, vol 1)
    // -----------------------------------------------------------------------

    /// Seul assembleur de texte `check_suite` **nouveau** de ce module : les
    /// trois lignes dans leur ordre, par leurs constantes.
    fn check_suite_text_with_heads(prs: &str) -> String {
        format!(
            "[GitHub] Check suite success on senara-solutions/mika (branch: feat/2917/x)\n\
             {CHECK_SUITE_HEAD_LINE_PREFIX}1a2b3c4d\n\
             {CHECK_SUITE_PRS_LINE_PREFIX}{prs}"
        )
    }

    #[test]
    fn mika2917_une_ligne_absente_nest_pas_un_tableau_vide() {
        // Le texte d'un binaire d'avant ce vol : une seule ligne.
        let ancien = check_suite_text("feat/2917/x");
        assert_eq!(
            parse_check_suite_heads(&ancien),
            CheckSuiteHeads::LineAbsent,
            "sans la ligne, on ne sait RIEN du payload — pas « il était vide »"
        );

        let vide = check_suite_text_with_heads(CHECK_SUITE_PRS_NONE);
        assert_eq!(
            parse_check_suite_heads(&vide),
            CheckSuiteHeads::Empty,
            "la ligne `none` dit que le payload ne nommait aucune tête"
        );

        assert_ne!(
            parse_check_suite_heads(&ancien),
            parse_check_suite_heads(&vide),
            "mika#2205 — un régime de déploiement et un comportement de la \
             forge ne s'additionnent pas sous un nom"
        );
    }

    #[test]
    fn mika2917_une_seule_pr_est_le_seul_cas_comparable() {
        assert_eq!(
            parse_check_suite_heads(&check_suite_text_with_heads("2917@1a2b3c4d")),
            CheckSuiteHeads::One {
                pr: 2917,
                head_sha: "1a2b3c4d".to_string()
            }
        );
    }

    #[test]
    fn mika2917_plusieurs_pr_ne_nomment_pas_la_tete_de_levenement() {
        assert_eq!(
            parse_check_suite_heads(&check_suite_text_with_heads("2917@1a2b3c4d 2918@9f8e7d6c")),
            CheckSuiteHeads::Many(2),
            "deux PR : le payload ne dit pas de LAQUELLE la suite parle"
        );
    }

    #[test]
    fn mika2917_un_jeton_illisible_est_saute_et_non_promu_en_erreur() {
        // `@` absent, numéro non numérique, `sha` vide : chacun saute, et la PR
        // lisible qui les voisine survit. Le résultat est donc `One`, pas
        // `Many(4)` — un jeton qu'on ne sait pas lire ne nomme aucune PR.
        assert_eq!(
            parse_check_suite_heads(&check_suite_text_with_heads(
                "2917 nan@1a2b3c4d 2919@ 2918@9f8e7d6c"
            )),
            CheckSuiteHeads::One {
                pr: 2918,
                head_sha: "9f8e7d6c".to_string()
            }
        );

        // Tous illisibles : la ligne est là, elle ne nomme rien. Même valeur
        // que `none`, parce que c'est la même chose dite du payload.
        assert_eq!(
            parse_check_suite_heads(&check_suite_text_with_heads("2917 nan@abc 2919@")),
            CheckSuiteHeads::Empty
        );
    }

    #[test]
    fn mika2917_la_ligne_des_pr_se_lit_ou_quelle_soit() {
        // Le prédicat est par ligne et non positionnel : un enrichissement
        // préfixé ou une enveloppe de gestionnaire décale les lignes sans que
        // le parse en souffre (même leçon que mika#2768 sur `mask_cited_branch`).
        let enveloppe = format!(
            "<ci_success_handler>\n{}\n</ci_success_handler>",
            check_suite_text_with_heads("2917@1a2b3c4d")
        );
        assert_eq!(
            parse_check_suite_heads(&enveloppe),
            CheckSuiteHeads::One {
                pr: 2917,
                head_sha: "1a2b3c4d".to_string()
            }
        );
    }

    #[test]
    fn mika2917_le_masque_de_branche_ne_touche_pas_les_lignes_de_tete() {
        // Les deux mécanismes cohabitent sur le même texte : `mask_cited_branch`
        // borne sa coupe à la ligne de l'occurrence, donc les lignes 2 et 3
        // traversent intactes et le parse les lit encore après masquage.
        let text = check_suite_text_with_heads("2917@1a2b3c4d");
        let masked = mask_cited_branch(&text);

        assert_eq!(
            parse_check_suite_heads(masked.as_ref()),
            CheckSuiteHeads::One {
                pr: 2917,
                head_sha: "1a2b3c4d".to_string()
            },
            "le masque a mangé une ligne qu'il ne vise pas : {masked}"
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
