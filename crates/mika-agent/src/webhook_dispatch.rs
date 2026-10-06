//! Shared predicates for webhook dispatch gating (mika#933).
//!
//! Both `agent.rs` (INTENT_GUARDS post-hoc) and `skills/executor.rs`
//! (tool-boundary pre-hoc) consume these predicates. Single source of truth
//! prevents drift between the two guard layers.

/// Marker prefix emitted by `mika_gateway::github::format_event_text` for
/// `issues.labeled` events where the label name is `ready`. Re-exported
/// from `mika_common::github_event_format` for cross-crate single-source-of-
/// truth coupling. See mika#852.
pub(crate) use mika_common::github_event_format::READY_LABEL_DISPATCH_MARKER;

/// Préfixe des événements de territoire qa (revue, action de PR).
///
/// **Site de définition unique des deux préfixes**, lu par les deux faces de la
/// même frontière : [`is_webhook_fallthrough_domain`], qui sort cette famille du
/// domaine Fallthrough, et [`webhook_event_target`] (mika#2649), qui l'y
/// retrouve pour la borner à sa cible. Ce qui sort de l'un est exactement ce
/// qu'il faut borner dans l'autre, donc les deux doivent lire le même octet.
///
/// **Déclarés ICI, en tête de module, et ce n'est pas un choix de mise en
/// page :** `canonical_tokens`'s `production_sources` tronque chaque fichier à
/// son premier marqueur d'attribut `cfg(test)` **où qu'il soit, commentaires
/// compris**, et le doc-comment d'[`ALL_MARKER_CLASSES`] en porte un. Déclarées
/// plus bas, ces constantes seraient invisibles à
/// `mika2517_the_fallthrough_domain_has_a_single_definition`, dont
/// l'anti-vacuité l'a dit en rougissant. Les déplacer ici est ce qui garde ce
/// scan exact plutôt que vert par troncature — et ce paragraphe lui-même évite
/// d'écrire la séquence littérale, faute de quoi il tronquerait le fichier juste
/// au-dessus de la constante qu'il décrit.
const PR_EVENT_PREFIX: &str = "[GitHub] PR ";

/// Préfixe des événements de territoire ci (suite de checks).
///
/// Même site, même raison que [`PR_EVENT_PREFIX`].
const CHECK_SUITE_EVENT_PREFIX: &str = "[GitHub] Check suite ";

/// True when the message is a `[GitHub]` webhook event in the
/// **Webhook Fallthrough** domain — i.e., a turn that MUST NOT call
/// `run_claude_pilot`. The fallthrough domain is the complement of:
/// (a) the authorized ready-label dispatch marker, and (b) the qa/ci
/// handler-skill territory (PR events, check suites).
///
/// Allowlist rationale: the gateway emits `[GitHub] PR ...` and `[GitHub]
/// Check suite ...` prefixes specifically for events that `self-dev-webhook-qa`
/// and `self-dev-webhook-ci` activate on. Those skills own legitimate
/// `run_claude_pilot` dispatch flows (CI-fix iteration, QA hold retries) and
/// must not be blocked. The fallthrough rejection scope is exactly the
/// `[GitHub] Issue ...` / `[GitHub] New comment on ...` / unknown-catchall
/// surface where no handler skill activates — the same scope the self-dev
/// prompt's Webhook Fallthrough section governs.
///
/// Mutually exclusive with `is_ready_label_dispatch_marker` on the
/// `[GitHub]` domain (mika#910).
//
// DOCTRINE: pre-classifier structural gate (mika#1733 AC2)
// Applies per crates/mika-agent/docs/permission-decision-protocol-2026-07-06.md §AC2:
// "This agent structurally cannot do X" applies to pre-classifier engine gates
// only, NEVER to LLM classifier decisions. This predicate is such a gate — it
// rejects unauthorized webhook-triggered `run_claude_pilot` calls based on the
// message SOURCE (webhook prefix + kind), which is a structural fact the LLM
// classifier cannot itself verify without begging the question.
//
// NOTE: The tier1/tier2/tier3 permission classifier code lives in
// claude-pilot-py; the companion doctrine anchor for those sites is tracked
// as a cross-repo follow-up filed alongside this PR (see PR body §Follow-ups).
// This annotation covers the in-mika-agent structural gate only.
pub(crate) fn is_unauthorized_webhook_dispatch(msg: &str) -> bool {
    is_webhook_fallthrough_domain(msg)
}

/// True when `msg` is a `[GitHub]` webhook event in the **Webhook Fallthrough**
/// domain — the complement of (a) the ready-label dispatch marker, (b) PR events
/// (qa skill territory) and (c) check-suite events (ci skill territory).
///
/// **The same set as [`is_unauthorized_webhook_dispatch`], asked as a different
/// question**, and that is why there are two names for one body. That predicate
/// answers *"may this turn dispatch a pilot?"* and its two callers are refusals
/// (the tool-boundary gate 0 of `validate_dispatch_readiness`, the
/// `webhook_no_unauthorized_dispatch` intent guard); this one answers *"is this
/// turn in the acknowledge-and-stop domain?"* and its callers withhold a tool
/// (mika#2517 U2) and stand a guard down (U3). One definition, so the four
/// consumers cannot drift — the class `grooming_marker` had to engrave once
/// (mika#2158: a copied regex whose own comment said "Mirrors …" and which then
/// missed two widenings).
///
/// Renaming `is_unauthorized_webhook_dispatch` was refused: ~20 test references
/// and two refusal sites documented by three tickets (mika#910 / #933 / #1102)
/// would churn for nothing, on a name that carries its own meaning correctly.
///
/// The body is the one that used to live in `is_unauthorized_webhook_dispatch`,
/// moved and not modified — the eight-row matrix that pinned it there
/// (`test_is_unauthorized_webhook_dispatch_predicate`) still runs against that
/// name, which is what attests the move is a move.
pub(crate) fn is_webhook_fallthrough_domain(msg: &str) -> bool {
    if !msg.starts_with("[GitHub]") {
        return false;
    }
    if msg.starts_with(READY_LABEL_DISPATCH_MARKER) {
        return false;
    }
    // qa skill territory (Phase 0 prefix surface rows E, F).
    //
    // Depuis mika#2649 les deux littéraux de préfixe ont un **site de définition
    // unique** ([`PR_EVENT_PREFIX`], [`CHECK_SUITE_EVENT_PREFIX`]) partagé avec
    // `webhook_event_target`, qui lit la même frontière depuis l'autre côté : ce
    // qui sort d'ici est exactement ce qu'il faut borner à sa cible. Même valeur,
    // même comportement — la matrice à huit lignes de
    // `test_is_unauthorized_webhook_dispatch_predicate` passe sans modification.
    if msg.starts_with(PR_EVENT_PREFIX) {
        return false;
    }
    // ci skill territory (Phase 0 prefix surface row G).
    if msg.starts_with(CHECK_SUITE_EVENT_PREFIX) {
        return false;
    }
    // Everything else in [GitHub] domain (rows B, C, D, H) is fallthrough.
    true
}

/// True when `msg` is the turn `verdict_handler` hands to the LLM for a
/// `hold[review]` with no active task (mika#2667 AC2).
///
/// Single reader of the marker that handler writes. Two consumers, both
/// existing gates: `effective_disabled_tools` withholds `create_task`, and
/// `validate_dispatch_readiness` refuses every long-running dispatch. The
/// message starts with `[verdict_handler]`, so it never enters the Webhook
/// Fallthrough domain: none of the mika#2517 consumers move.
pub(crate) fn is_hold_review_without_task_turn(msg: &str) -> bool {
    msg.starts_with(crate::server::verdict_handler::HOLD_REVIEW_NO_TASK_MARKER)
}

/// The event class of a Webhook Fallthrough turn, as a **wire format**
/// (mika#2517 U4).
///
/// These five values land in `audit_events.after_value` and an operator writes
/// `GROUP BY` over them, so two spellings of one class would split a population
/// without saying so. One definition site, pinned by
/// `mika2517_marker_class_is_a_wire_format`.
///
/// Only meaningful on a message [`is_webhook_fallthrough_domain`] accepts; the
/// caller establishes that first. `MARKER_CLASS_OTHER` covers the unknown-event
/// catchall (row H) and anything the gateway starts emitting tomorrow — a class
/// nobody enumerated is still a class we can count.
pub(crate) const MARKER_CLASS_ISSUE_LABELED: &str = "issue_labeled";
pub(crate) const MARKER_CLASS_ISSUE_COMMENT: &str = "issue_comment";
pub(crate) const MARKER_CLASS_ISSUE_ASSIGNED: &str = "issue_assigned";
pub(crate) const MARKER_CLASS_ISSUE_CLOSED: &str = "issue_closed";
pub(crate) const MARKER_CLASS_OTHER: &str = "other";

/// Every value [`fallthrough_marker_class`] can return, for the wire-format pin.
///
/// Its only consumer is the pinning test, and it stays in production rather
/// than behind `#[cfg(test)]` on purpose: the registry of a wire format is what
/// an operator reads to know what a `GROUP BY` can return, and a registry that
/// exists only under `cfg(test)` is one a reader of this file cannot find.
#[allow(dead_code)]
pub(crate) const ALL_MARKER_CLASSES: &[&str] = &[
    MARKER_CLASS_ISSUE_LABELED,
    MARKER_CLASS_ISSUE_COMMENT,
    MARKER_CLASS_ISSUE_ASSIGNED,
    MARKER_CLASS_ISSUE_CLOSED,
    MARKER_CLASS_OTHER,
];

/// Classify a Webhook Fallthrough message into one of [`ALL_MARKER_CLASSES`].
///
/// The prefixes are those `mika_gateway::github::format_event_text` emits. This
/// is **observability only** — no decision reads it, which is why an unknown
/// shape falls to `other` rather than being refused.
pub(crate) fn fallthrough_marker_class(msg: &str) -> &'static str {
    if msg.starts_with("[GitHub] Issue labeled ") {
        MARKER_CLASS_ISSUE_LABELED
    } else if msg.starts_with("[GitHub] New comment on ") {
        MARKER_CLASS_ISSUE_COMMENT
    } else if msg.starts_with("[GitHub] Issue assigned") {
        MARKER_CLASS_ISSUE_ASSIGNED
    } else if msg.starts_with("[GitHub] Issue closed") {
        MARKER_CLASS_ISSUE_CLOSED
    } else {
        MARKER_CLASS_OTHER
    }
}

/// True when the message matches the ready-label dispatch marker prefix.
pub(crate) fn is_ready_label_dispatch_marker(msg: &str) -> bool {
    msg.starts_with(READY_LABEL_DISPATCH_MARKER)
}

/// Le verbe d'intention, à un seul site — il décide d'un refus d'outil et son
/// orthographe est donc porteuse.
const GROOMING_INTENT_VERB: &str = "groom";

/// True when the turn was opened by an **explicit grooming request**
/// (mika#2484 D5).
///
/// # Le défaut que ça ferme
///
/// `mika ask --agent mika-dev "groom mika issue#2471"` sur un ticket portant
/// déjà les callouts de grooming a produit un callback **implement** qui a
/// ouvert une PR — une implémentation sur un grooming que le chemin moteur n'a
/// jamais vérifié, c'est-à-dire un contournement de la porte de preuve. Le même
/// message, après retrait des callouts du corps, a correctement dispatché un
/// `dev-groom`. Une intention explicite ne peut pas dépendre de l'état apparent
/// du corps du ticket.
///
/// # L'ancrage sur le mot est ce qui rend le prédicat sûr
///
/// Ce n'est pas un détail de regex : `starts_with("groom")` nu mordrait sur
/// « grooming report for mika#N », qui est une demande de rapport et non une
/// demande de grooming. L'espace (ou la tabulation) obligatoire sépare
/// `groom ` de `grooming`, et le contrôle négatif est un test nommé.
///
/// Insensible à la casse, tolérant au blanc de tête — un opérateur écrit
/// « Groom … » et « ␣groom … » indifféremment.
///
/// # Pourquoi ce prédicat ne peut pas tuer le chemin nominal (R8)
///
/// Propriété **structurelle**, établie par lecture et non par prudence : il lit
/// `originating_message`, qui vaut `None` sur l'auto-fire post-groom
/// (mika#1614, posé explicitement à `None`) et sur tout tour de callback, et
/// qui commence par `[GitHub] Issue labeled ready on` sur le chemin webhook ou
/// par le texte d'une revue de PR sur la relance de verdict. Aucun des quatre
/// ne commence par `groom `.
pub(crate) fn is_grooming_intent_message(msg: &str) -> bool {
    let trimmed = msg.trim_start();
    let Some(rest) = trimmed.get(..GROOMING_INTENT_VERB.len()) else {
        return false;
    };
    if !rest.eq_ignore_ascii_case(GROOMING_INTENT_VERB) {
        return false;
    }
    // Le séparateur obligatoire : c'est lui qui sépare `groom ` de `grooming`.
    matches!(
        trimmed.as_bytes().get(GROOMING_INTENT_VERB.len()),
        Some(b' ' | b'\t')
    )
}

/// Owner applied to a bare `<repo>` reference. The loop only ever operates on
/// `senara-solutions` repositories; a marker that omits the owner is a gateway
/// short-form, not an invitation to guess another org.
pub(crate) const DEFAULT_DISPATCH_OWNER: &str = "senara-solutions";

/// The repositories the autonomous loop is allowed to dispatch into, fully
/// qualified as `owner/repo` (mika#2046).
///
/// **Default-deny.** Before this list existed the effective policy was "whatever
/// the webhook names": every link from the `ready` label to a worktree — the
/// marker parse, `ReadyLabelLocation::repo_name`, dispatch-lib's `repo#number`
/// parse, and its `SUB_REPO_DIR` resolution — is pure string handling, so a
/// `ready` label on any repository reachable from the workspace would have
/// created a worktree there and run the pipeline in it.
///
/// **Why this is a hand-held constant and not derived from the workspace.**
/// Deriving the list from "which directories are git repositories" is precisely
/// the predicate that fails: `control-monitor` and `claude-pilot` *are* git
/// repositories sitting next to `mika` in the workspace, and the 2026-08-29
/// operator decision is that they are spawn-CC-only and must never be reached by
/// the loop. Presence describes what exists, not what is permitted; the two are
/// different questions and only one of them is the policy. So the list is held
/// in exactly one place and every refusal quotes it — see
/// [`dispatchable_repos_display`].
///
/// **`wizzard` is deliberately absent.** It is a read-write controlled repo, but
/// the loop has never dispatched into it and the 2026-08-29 arbitrage named these
/// four. Listing a repo the loop cannot actually drive would be the same
/// permitted-versus-exists confusion in the other direction. Because refusal is
/// noisy and named, the first `ready` label on a wizzard issue reports itself
/// rather than failing quietly — that is the intended way to revisit this.
///
/// Churn here is rare and a rebuild is the accepted cost, per the same reasoning
/// recorded for `DISPATCH_TRIGGER_ALLOWLIST` in
/// `docs/solutions/1053-dispatch-trigger-allowlist-config-constant.md`.
pub(crate) const DISPATCHABLE_REPOS: &[&str] = &[
    "senara-solutions/mika",
    "senara-solutions/mika-cloud",
    "senara-solutions/mika-skills",
    "senara-solutions/mika-platform",
];

/// Normalize a repository reference to its fully-qualified `owner/repo` form,
/// applying [`DEFAULT_DISPATCH_OWNER`] when the reference carries no owner.
///
/// Single source of truth for the defaulting rule: `ReadyLabelLocation::owner_repo`
/// delegates here so the handler and the tool-boundary gate cannot drift on what
/// `mika#2046` means.
pub(crate) fn normalize_owner_repo(repo_ref: &str) -> String {
    if repo_ref.contains('/') {
        repo_ref.to_string()
    } else {
        format!("{DEFAULT_DISPATCH_OWNER}/{repo_ref}")
    }
}

/// True when `repo_ref` names a repository the loop may dispatch into.
///
/// Accepts either form — `mika` or `senara-solutions/mika` — and compares the
/// **owner-qualified** result against [`DISPATCHABLE_REPOS`]. Matching on the
/// bare basename would accept `another-org/mika`, whose basename is `mika` but
/// which is not our repository.
///
// DOCTRINE: pre-classifier structural gate (mika#2046)
// Applies per crates/mika-agent/docs/permission-decision-protocol-2026-07-06.md §AC2:
// "This agent structurally cannot do X" applies to pre-classifier engine gates
// only, NEVER to LLM classifier decisions. This predicate is such a gate — which
// repository a dispatch targets is a structural fact read off the trigger, not a
// judgement the LLM classifier is asked to make.
pub(crate) fn is_dispatchable_repo(repo_ref: &str) -> bool {
    if repo_ref.is_empty() {
        return false;
    }
    let owner_repo = normalize_owner_repo(repo_ref);
    DISPATCHABLE_REPOS.contains(&owner_repo.as_str())
}

/// Extract the repository reference from a dispatch `prompt` argument.
///
/// Recognizes the anchored `[owner/]repo#number` shape that `dispatch-lib.sh`'s
/// worktree-setup parser accepts, so the tool-boundary gate and the shell agree
/// on which prompts are repository references at all.
///
/// **This must never be stricter than the shell**, because the shell is what
/// actually creates the worktree. Anything this returns `None` for is a prompt
/// the allowlist never judges — so a prompt the shell routes into worktree mode
/// but this reads as free text walks straight past the gate. Two places where
/// the shell is laxer than a naive reading, both covered here:
///
/// * **Surrounding whitespace.** `dispatch-lib.sh:769` reads the prompt as
///   `PROMPT=$(… jq -r '.prompt')`, and command substitution strips trailing
///   newlines. `"control-monitor#159\n"` therefore reaches the shell's regex as
///   `control-monitor#159` and matches. Hence the trim.
/// * **Multi-line prompts.** The shell test is `grep -qE '^…$'`, which succeeds
///   when *any* line matches, not only when the whole string does. Hence the
///   per-line scan: the first line that is a repository reference is the one the
///   allowlist judges.
///
/// Returns `None` for genuine free text — including free text that merely
/// contains a `#`. A free-text dispatch resolves no repository, so the allowlist
/// has nothing to judge and must not refuse it.
pub(crate) fn parse_repo_ref_from_dispatch_prompt(prompt: &str) -> Option<&str> {
    parse_issue_ref_line_from_prompt(prompt).map(|(repo_ref, _)| repo_ref)
}

/// Extract the repository reference **and issue number** from a dispatch
/// `prompt` argument (mika#2084).
///
/// The seat gate needs the issue number, not just the repository — a seat label
/// lives on one issue. This shares [`parse_repo_ref_line`] with
/// [`parse_repo_ref_from_dispatch_prompt`] rather than parsing the prompt a
/// second time: two independent parses of the same string is precisely the
/// drift the "must never be stricter than the shell" note above guards against.
///
/// The single added strictness is numeric overflow — a `#` number too large for
/// `u64` yields `None` here while the repo-level parse still accepts it. That
/// direction is deliberate: an unparseable number means the seat gate has no
/// issue to look up, and per mika#2084 D2 missing information lets the dispatch
/// through rather than refusing it.
pub(crate) fn parse_issue_ref_from_dispatch_prompt(prompt: &str) -> Option<(&str, u64)> {
    let (repo_ref, number) = parse_issue_ref_line_from_prompt(prompt)?;
    Some((repo_ref, number.parse::<u64>().ok()?))
}

/// First line of `prompt` that is a repository reference, as `(repo_ref, number)`
/// with the number still in its unparsed textual form.
fn parse_issue_ref_line_from_prompt(prompt: &str) -> Option<(&str, &str)> {
    prompt.lines().find_map(parse_repo_ref_line)
}

/// The single-reference form, applied to one already-split line.
fn parse_repo_ref_line(line: &str) -> Option<(&str, &str)> {
    let (repo_ref, number) = line.trim().split_once('#')?;
    if number.is_empty() || !number.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let segment_ok = |s: &str| {
        !s.is_empty()
            && s.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    };
    let shape_ok = match repo_ref.split_once('/') {
        Some((owner, repo)) => segment_ok(owner) && segment_ok(repo),
        None => segment_ok(repo_ref),
    };
    shape_ok.then_some((repo_ref, number))
}

/// The allowlist rendered for a refusal message, so every refusal states what
/// would have been accepted instead of only what was denied (mika#2046).
pub(crate) fn dispatchable_repos_display() -> String {
    DISPATCHABLE_REPOS.join(", ")
}

// ───────────────────────── Dispatch seat (mika#2084) ─────────────────────────

/// Prefix of the label that names which dispatcher owns a ticket (mika#2084).
///
/// The labels that mean **someone else is holding this ticket** (mika#2263
/// défaut (c)).
///
/// The single list behind two surfaces: `auto_pull::feeder_exclusion_label`
/// (which keeps a held ticket out of the pullable pool and the feeder backlog)
/// and the `ready_label_handler` gate (which keeps a held ticket from being
/// dispatched by a `ready` event that arrives anyway — stale, redelivered, or
/// applied by hand).
///
/// One list, because the 2026-09-09 measurement is what happens with two:
/// `blocked` excluded #1781 from the feeder and nothing else, and the handler
/// re-dispatched it twice (pgid 478551, 492118). A label that holds a ticket
/// on one path and not the other does not hold the ticket.
///
/// `operator-gated` is here for the reason mika#2123 gave it to the feeder: it
/// is what makes a promotion refusal *persist*, and `.github/labels.yml`
/// already promised "No ready label" for it.
pub(crate) const OPERATOR_HELD_LABELS: &[&str] = &["blocked", "operator-review", "operator-gated"];

/// The operator-held label carried by `labels`, if any (mika#2263).
///
/// Returns **which** label held the ticket, never a bare boolean: an operator
/// reading a refusal needs to know which label to remove, and a counter needs
/// to distinguish "held by `blocked`" from "held by `operator-gated`".
pub(crate) fn operator_held_label<'a>(
    labels: impl IntoIterator<Item = &'a str>,
) -> Option<&'a str> {
    labels
        .into_iter()
        .find(|l| OPERATOR_HELD_LABELS.contains(l))
}

/// The match is on this **exact** prefix. `dispatched`, `dispatch-ready`, and
/// any other label that merely starts with the letters `dispatch` are ordinary
/// labels and must not enter the seat gate — treating them as seat labels would
/// refuse tickets nobody claimed, which is the failure mode that stops the loop
/// rather than protecting it.
pub(crate) const DISPATCH_SEAT_LABEL_PREFIX: &str = "dispatch:";

/// The seat this engine dispatches as.
///
/// `dispatch:ssc` and `dispatch:mpc` name interactive Claude Code seats. The
/// autonomous loop is neither: it is a third seat, `loop`. The direct and
/// intended consequence is that a ticket labelled for *either* interactive seat
/// is refused here — which is exactly the 2026-08-30 collision this constant
/// exists to prevent.
pub(crate) const CURRENT_DISPATCH_SEAT: &str = "loop";

/// Every seat this engine knows how to resolve (mika#2084).
///
/// **Hand-held, like [`DISPATCHABLE_REPOS`], and for the same reason.** A list
/// derived from "seats we have seen on tickets" would turn observation into
/// authorization: the first typo'd label would mint a seat and the gate would
/// wave it through. What exists and what is permitted are different questions,
/// and only the second one is the policy. A seat absent from this list is
/// refused (see [`classify_dispatch_seat`]), so the first ticket carrying a new
/// seat reports itself loudly instead of failing quietly — that is the intended
/// way to add one.
///
/// **This list and `.github/labels.yml` are one vocabulary written twice, and
/// they must move in the same commit** (mika#2092). A seat here without its
/// `dispatch:<seat>` entry there is not merely undeclared: the label-sync
/// workflow runs with `delete-other-labels: true`, so it is a label GitHub
/// DELETES from the repository — and from every issue carrying it — with no
/// `unlabeled` event and no log. [`classify_dispatch_seat`] then reads
/// [`SeatVerdict::NoSeatLabel`] everywhere and refuses nothing: the gate
/// disarms itself in silence, which is exactly how the 2026-08-30 collision
/// this module exists to prevent became reachable again on 2026-08-30 at
/// 09:12:51Z, an hour before mika#2084 shipped.
///
/// The other direction is the mirror and equally real: a `dispatch:*` label
/// declared without a seat here resolves to [`SeatVerdict::Unresolvable`] and
/// refuses the ticket — fail-closed, so the loop stops on a label somebody was
/// told existed.
///
/// `scripts/check-dispatch-seats-declared.sh` compares the two lists both ways
/// and fails CI on divergence, so this paragraph is enforced rather than
/// remembered. `dispatch:zorglub`, used below as the unknown-seat fixture, must
/// stay undeclared for the same reason it is a good fixture.
pub(crate) const KNOWN_DISPATCH_SEATS: &[&str] = &["loop", "ssc", "mpc"];

/// What the seat labels on one issue say about whether this engine may take it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SeatVerdict {
    /// No `dispatch:*` label at all — the overwhelmingly common case, and the
    /// load-bearing one. Behaviour must be **identical** to the pre-#2084 path
    /// (mika#2084 AC3): a fix that turns "unlabelled" into "refused" stops the
    /// whole loop, which is worse than the defect it repairs.
    NoSeatLabel,
    /// Labelled for this engine's own seat — dispatch proceeds. Stamped by
    /// dispatch-lib (`_stamp_issue_seat`) when it takes the ticket and
    /// released (`_release_issue_seat`) when it exits (mika#2155); between two
    /// dispatches an unclaimed ticket reads [`SeatVerdict::NoSeatLabel`].
    OwnedByCurrentSeat { label: String },
    /// Labelled for a different, known seat — refused (mika#2084 AC1).
    OwnedByOtherSeat { label: String, seat: String },
    /// A seat label is present but cannot be resolved to exactly one known seat
    /// — refused (mika#2084 AC2). Fail-closed: a seat we cannot identify is not
    /// an authorization.
    Unresolvable { label: String, why: &'static str },
}

impl SeatVerdict {
    /// True when this verdict must stop the dispatch.
    ///
    /// Refusal is the *narrow* case by construction: only a resolved foreign
    /// seat or an unresolvable seat label refuses. Absence of a label never
    /// does (AC3).
    pub(crate) fn refuses(&self) -> bool {
        matches!(
            self,
            SeatVerdict::OwnedByOtherSeat { .. } | SeatVerdict::Unresolvable { .. }
        )
    }

    /// The label text that drove the verdict, for refusal messages and audit
    /// events. `None` when no seat label was involved.
    pub(crate) fn label(&self) -> Option<&str> {
        match self {
            SeatVerdict::NoSeatLabel => None,
            SeatVerdict::OwnedByCurrentSeat { label }
            | SeatVerdict::OwnedByOtherSeat { label, .. }
            | SeatVerdict::Unresolvable { label, .. } => Some(label),
        }
    }

    /// Why the dispatch was refused, as a stable snake_case reason code for the
    /// audit trail (mika#2084 AC5). `None` when the verdict does not refuse.
    pub(crate) fn refusal_reason(&self) -> Option<&'static str> {
        match self {
            SeatVerdict::OwnedByOtherSeat { .. } => Some("seat_owned_by_other"),
            SeatVerdict::Unresolvable { why, .. } => Some(why),
            _ => None,
        }
    }
}

/// Classify the `dispatch:*` labels on one issue against [`CURRENT_DISPATCH_SEAT`].
///
/// Pure — the caller supplies the label names, however it obtained them. That
/// keeps the decision testable without a network, and lets the three call sites
/// (`auto_pull` selection, the ready-label handler, the tool boundary) share one
/// rule instead of three that drift.
///
/// Matching is case-insensitive on both the prefix and the seat: a label typed
/// `Dispatch:SSC` in the GitHub UI claims the same seat as `dispatch:ssc`.
///
// DOCTRINE: pre-classifier structural gate (mika#2084)
// Applies per crates/mika-agent/docs/permission-decision-protocol-2026-07-06.md §AC2:
// "This agent structurally cannot do X" applies to pre-classifier engine gates
// only, NEVER to LLM classifier decisions. This predicate is such a gate — which
// seat owns a ticket is a structural fact read off the issue's labels, not a
// judgement the LLM classifier is asked to make.
pub(crate) fn classify_dispatch_seat<'a>(labels: impl IntoIterator<Item = &'a str>) -> SeatVerdict {
    let seat_labels: Vec<String> = labels
        .into_iter()
        .map(|l| l.trim().to_ascii_lowercase())
        .filter(|l| l.starts_with(DISPATCH_SEAT_LABEL_PREFIX))
        .collect();

    // AC3. The common path, and the one that must not change.
    let label = match seat_labels.len() {
        0 => return SeatVerdict::NoSeatLabel,
        1 => seat_labels.into_iter().next().expect("len checked as 1"),
        // Two seats claimed, neither wins. Ambiguity is an unresolved seat, not
        // a tie to break — resolving it either way would invent an owner.
        _ => {
            return SeatVerdict::Unresolvable {
                label: seat_labels.join(", "),
                why: "multiple_seat_labels",
            };
        }
    };

    let seat = label
        .strip_prefix(DISPATCH_SEAT_LABEL_PREFIX)
        .expect("filtered on this prefix")
        .trim()
        .to_string();

    if seat.is_empty() {
        return SeatVerdict::Unresolvable {
            label,
            why: "empty_seat",
        };
    }
    if !KNOWN_DISPATCH_SEATS.contains(&seat.as_str()) {
        return SeatVerdict::Unresolvable {
            label,
            why: "unknown_seat",
        };
    }
    if seat == CURRENT_DISPATCH_SEAT {
        return SeatVerdict::OwnedByCurrentSeat { label };
    }
    SeatVerdict::OwnedByOtherSeat { label, seat }
}

/// One sentence naming why a verdict refuses, in the operator's terms.
///
/// `Unresolvable` is NOT "another seat owns this". A `dispatch:zorglub` typo, a
/// bare `dispatch:`, or two seat labels means nobody could be identified as the
/// owner — saying otherwise sends the operator looking for a colliding seat that
/// does not exist. The audit events already carried the distinction; this makes
/// the human-facing text carry it too.
pub(crate) fn seat_refusal_sentence(verdict: &SeatVerdict) -> String {
    match verdict {
        SeatVerdict::OwnedByOtherSeat { label, seat } => format!(
            "this issue carries the seat label `{label}`, so dispatch seat \
             `{seat}` owns it — dispatching would put a second writer on its branch."
        ),
        SeatVerdict::Unresolvable { label, why } => {
            let detail = match *why {
                "multiple_seat_labels" => {
                    "it carries more than one seat label, \
                     so no single seat can be resolved as the owner"
                }
                "empty_seat" => "its seat label names no seat at all",
                _ => "its seat label names a seat this engine does not know",
            };
            format!(
                "the seat label `{label}` could not be resolved to a known seat: \
                 {detail}. A seat that cannot be identified is not an authorization."
            )
        }
        SeatVerdict::NoSeatLabel | SeatVerdict::OwnedByCurrentSeat { .. } => {
            "no refusal applies".to_string()
        }
    }
}

/// The known-seat list rendered for a refusal message, so a refusal states what
/// would have been accepted and not only what was denied (same discipline as
/// [`dispatchable_repos_display`]).
pub(crate) fn known_dispatch_seats_display() -> String {
    KNOWN_DISPATCH_SEATS.join(", ")
}

// ───────── Lignée d'un dispatch ouvert par un événement PR (mika#2649) ─────────

/// La cible qu'un événement webhook désigne, quand il en désigne une.
///
/// # Le trou que ça ferme
///
/// [`is_unauthorized_webhook_dispatch`] juge la **nature** de l'événement
/// source : `[GitHub] PR …` et `[GitHub] Check suite …` sortent du domaine
/// Fallthrough parce que `self-dev-webhook-qa` / `-ci` y portent des dispatchs
/// légitimes. Une fois sortis de ce domaine, **plus aucun terme ne liait le
/// dispatch à la PR de l'événement** : mesuré le 2026-10-02, un tour ouvert par
/// une revue QA sur la PR #2647 a lancé un implement de mika#2646, hors fenêtre
/// `ready`, pendant qu'un autre implement volait.
///
/// # Aucun second parseur de grammaire
///
/// Les deux grammaires sont lues par leurs lecteurs **uniques** existants —
/// [`crate::server::deadline_verdict::parse_pr_target`] pour la forme PR
/// (mika#2368, dont le commentaire dit : *« Aucune des deux regex n'est recopiée
/// ici : une grammaire de fil dupliquée est exactement ce qui a laissé deux
/// lecteurs diverger dans mika#2158 »*) et
/// [`crate::server::webhook_queue_v2::classify_event`] pour la forme
/// check-suite, dont le `CHECK_SUITE_RE` existe **déjà en deux copies** dans
/// l'arbre (`webhook_queue.rs` et `webhook_queue_v2.rs`, la seconde se déclarant
/// doublon assumé). Écrire une troisième extraction de `(branch: …)` ici serait
/// la classe mika#2158 à son troisième tour ; le numéro d'issue porté par la
/// branche est lu par [`crate::worktree_reaper::issue_number_from_branch`]
/// (mika#2619, *« le deuxième segment, et rien d'autre »*).
///
/// C'est une **rectification au plan de mika#2649**, qui décrivait « extraction
/// de `(branch: …)` puis `issue_number_from_branch` » en supposant qu'aucun
/// lecteur n'existait : il en existe deux, donc on en appelle un.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum WebhookEventTarget {
    /// `[GitHub] PR …` — le numéro est celui de la **pull request**.
    Pr { repo: String, number: u64 },
    /// `[GitHub] Check suite …` — la grammaire porte une **branche**, jamais un
    /// numéro de PR (épinglé à `deadline_verdict.rs` : `parse_pr_target` rend
    /// `None` sur un texte check-suite). Le numéro d'**issue** en est le
    /// deuxième segment, ou rien.
    Branch { repo: String, issue: Option<u64> },
    /// Le message porte bien un préfixe PR / check-suite, mais sa grammaire n'a
    /// pas parsé. **Autorise** — il n'y a aucune cible à laquelle borner — mais
    /// c'est une anomalie, donc c'est audité sous son propre nom.
    Unreadable,
    /// Le message n'est pas un événement PR / check-suite du tout : un tour
    /// Telegram, un callback, un heartbeat, un `[GitHub] Issue …`. **Hors
    /// population**, et jamais audité — ce serait l'essentiel du trafic, soit
    /// très exactement le churn que la doctrine mika#2131 borne.
    NotApplicable,
}

/// La cible de l'événement qui a ouvert ce tour, s'il en désigne une.
///
/// Pur : aucun I/O, aucune base. Les deux prédicats de préfixe sont ceux
/// qu'[`is_webhook_fallthrough_domain`] emploie déjà pour sortir ces deux
/// familles du domaine Fallthrough — la même frontière, lue depuis l'autre côté.
pub(crate) fn webhook_event_target(msg: &str) -> WebhookEventTarget {
    if msg.starts_with(PR_EVENT_PREFIX) {
        return match crate::server::deadline_verdict::parse_pr_target(msg) {
            Some(target) => WebhookEventTarget::Pr {
                repo: normalize_owner_repo(&target.repo),
                number: target.pr_number,
            },
            None => WebhookEventTarget::Unreadable,
        };
    }
    if msg.starts_with(CHECK_SUITE_EVENT_PREFIX) {
        return match crate::server::webhook_queue_v2::classify_event(msg) {
            crate::server::webhook_queue_v2::WebhookEventKind::CheckSuite { repo, branch } => {
                WebhookEventTarget::Branch {
                    repo: normalize_owner_repo(&repo),
                    issue: crate::worktree_reaper::issue_number_from_branch(&branch),
                }
            }
            // Le préfixe est là et `classify_event` n'a pas reconnu la forme :
            // la grammaire a bougé sous le lecteur.
            _ => WebhookEventTarget::Unreadable,
        };
    }
    WebhookEventTarget::NotApplicable
}

/// Le `tool_name` sous lequel chaque décision de lignée est auditée (mika#2649).
///
/// **Un seul nom**, la décision dans `after_value` — le motif `ready_label_outcome`
/// (mika#2323) : les cinq issues appartiennent au même site et à la même
/// population, donc un `GROUP BY after_value` les sépare et les rend
/// soustractibles. SOLE WRITER, épinglé par
/// `canonical_tokens::tests::mika2649_le_nom_daudit_a_un_seul_ecrivain` — c'est
/// cette propriété qui rend le compte exact plutôt qu'un nombre sur lequel deux
/// sites peuvent diverger.
pub(crate) const TARGET_BINDING_AUDIT_TOOL: &str = "webhook_dispatch_target_binding";

/// Un terme de lignée a tenu : le dispatch est autorisé.
pub(crate) const TARGET_BINDING_BOUND: &str = "bound";
/// Aucun terme de lignée n'a tenu : le dispatch est refusé.
pub(crate) const TARGET_BINDING_REFUSED: &str = "refused";
/// Préfixe PR / check-suite présent, grammaire non parsée ⇒ **autorise**.
pub(crate) const TARGET_BINDING_EVENT_UNREADABLE: &str = "event_unreadable";
/// Check-suite dont la branche ne porte aucun numéro d'issue ⇒ **autorise**.
pub(crate) const TARGET_BINDING_NO_TARGET_IN_EVENT: &str = "no_target_in_event";
/// La traversée de lignée n'a pas pu être faite (erreur base) ⇒ **refuse**.
pub(crate) const TARGET_BINDING_LINEAGE_UNREADABLE: &str = "lineage_unreadable";

/// Toutes les valeurs que `after_value` peut prendre, pour l'épinglage du format
/// de fil.
///
/// Reste en production plutôt que derrière `#[cfg(test)]`, et c'est la raison
/// qu'[`ALL_MARKER_CLASSES`] a déjà dû écrire : le registre d'un format de fil
/// est ce qu'un opérateur lit pour savoir ce qu'un `GROUP BY` peut rendre, et un
/// registre qui n'existe que sous `cfg(test)` est un registre qu'un lecteur de ce
/// fichier ne trouve pas.
#[allow(dead_code)]
pub(crate) const ALL_TARGET_BINDING_VERDICTS: &[&str] = &[
    TARGET_BINDING_BOUND,
    TARGET_BINDING_REFUSED,
    TARGET_BINDING_EVENT_UNREADABLE,
    TARGET_BINDING_NO_TARGET_IN_EVENT,
    TARGET_BINDING_LINEAGE_UNREADABLE,
];

/// Le terme de lignée qui a autorisé le dispatch, pour la ligne de journal.
///
/// Format de fil au même titre que les verdicts : il atterrit dans le
/// `reasoning` de la ligne d'audit, et c'est lui qui dit à l'opérateur **lequel**
/// des quatre termes a tenu — donc quelle moitié du prédicat réparer si la
/// cascade de jalon se met à être refusée (halte 2 de la sonde S2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LineageTerm {
    /// L1 — la tâche elle-même nomme la cible de l'événement.
    TaskReference,
    /// L2 — la tâche porte la PR de l'événement en `claude_pilot.pr_url`.
    TaskPilotPrUrl,
    /// L3 — un **frère** satisfait L1 ou L2. **C'est la cascade de jalon M4.**
    Sibling,
    /// L4 — le **parent** satisfait L1 ou L2.
    Parent,
}

impl LineageTerm {
    /// Nom stable du terme, pour la ligne de journal et l'audit.
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            LineageTerm::TaskReference => "task_reference_url",
            LineageTerm::TaskPilotPrUrl => "task_pilot_pr_url",
            LineageTerm::Sibling => "sibling",
            LineageTerm::Parent => "parent",
        }
    }
}

/// Ce que la traversée de lignée a établi.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TargetBinding {
    /// Un terme a tenu — autorise, et nomme lequel.
    Bound(LineageTerm),
    /// Aucun terme n'a tenu — refuse.
    Refused,
    /// Le préfixe était là, la grammaire n'a pas parsé — autorise.
    EventUnreadable,
    /// La grammaire a parsé et ne porte aucune cible — autorise.
    NoTargetInEvent,
    /// La question n'a pas pu être posée — refuse.
    LineageUnreadable,
}

impl TargetBinding {
    /// La valeur de `after_value`, en `match` exhaustif **sans bras `_ =>`** : un
    /// sixième état doit être tranché ici par le compilateur, pas deviné.
    pub(crate) fn audit_value(&self) -> &'static str {
        match self {
            TargetBinding::Bound(_) => TARGET_BINDING_BOUND,
            TargetBinding::Refused => TARGET_BINDING_REFUSED,
            TargetBinding::EventUnreadable => TARGET_BINDING_EVENT_UNREADABLE,
            TargetBinding::NoTargetInEvent => TARGET_BINDING_NO_TARGET_IN_EVENT,
            TargetBinding::LineageUnreadable => TARGET_BINDING_LINEAGE_UNREADABLE,
        }
    }

    // Pas de `refuses()`, contrairement à [`SeatVerdict`] — et c'est une
    // décision. Là-bas trois appelants posent la question et un `matches!`
    // recopié par l'un d'eux pourrait inverser la disposition ; ici le seul site
    // qui décide (`skills::executor::report_event_target_binding`) doit de toute
    // façon composer un **corps de refus différent par verdict refusant**, donc
    // la disposition est inséparable du corps. Un `refuses()` à côté serait une
    // seconde source de vérité sur la même question, libre de diverger du `match`
    // qui compose — et c'est ce `match`, exhaustif et sans bras `_ =>`, qui tient
    // la garantie.
}

/// `task.reference_url` nomme-t-il la cible de l'événement ? (terme L1)
///
/// **Strict sur le TYPE de référence**, et c'est une décision : un événement PR
/// se compare à une `reference_url` de **pull request**, un check-suite dont la
/// branche porte `N` à une `reference_url` d'**issue**. Un numéro de PR et un
/// numéro d'issue vivent dans le même espace de numérotation GitHub mais
/// désignent deux objets différents ; les apparier serait une **coïncidence de
/// numéro**, pas une lignée. Le cas réellement fréquent — une tâche implement sur
/// `issues/2641` et un événement sur la PR `pull/2647` qui l'implémente — est
/// couvert par L2, qui lit le lien que le producteur du dispatch a estampillé.
pub(crate) fn reference_url_names_target(
    target: &WebhookEventTarget,
    reference_url: Option<&str>,
) -> bool {
    let Some(url) = reference_url else {
        return false;
    };
    let Some(parsed) = crate::tools::parse_github_ref(url) else {
        return false;
    };
    match (target, parsed) {
        (
            WebhookEventTarget::Pr {
                repo: event_repo,
                number: event_number,
            },
            crate::tools::GitHubRef::PullRequest {
                owner,
                repo,
                number,
            },
        ) => number == *event_number && &format!("{owner}/{repo}") == event_repo,
        (
            WebhookEventTarget::Branch {
                repo: event_repo,
                issue: Some(event_issue),
            },
            crate::tools::GitHubRef::Issue {
                owner,
                repo,
                number,
            },
        ) => number == *event_issue && &format!("{owner}/{repo}") == event_repo,
        _ => false,
    }
}

/// `metadata.claude_pilot.pr_url` nomme-t-il la PR de l'événement ? (terme L2)
///
/// Le lien est **déjà porté en base** : `try_extract_callback_metadata`
/// l'estampille sur le parent à la fin de chaque dispatch, et
/// `iterate_dispatch` / `verdict_handler` le posent à la création. C'est ce qui
/// permet de répondre à « le ticket fermé par cette PR » **sans appel réseau** —
/// doctrine maison : *la cible est dite, jamais dérivée* (mika#2249, mika#2368).
///
/// Inapplicable à une cible `Branch` : une URL de PR ne dit pas quelle issue la
/// PR ferme.
pub(crate) fn pilot_pr_url_names_target(target: &WebhookEventTarget, pr_url: Option<&str>) -> bool {
    let WebhookEventTarget::Pr {
        repo: event_repo,
        number: event_number,
    } = target
    else {
        return false;
    };
    let Some(url) = pr_url else {
        return false;
    };
    match crate::tools::parse_github_ref(url) {
        Some(crate::tools::GitHubRef::PullRequest {
            owner,
            repo,
            number,
        }) => number == *event_number && &format!("{owner}/{repo}") == event_repo,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Exhaustive matrix mapped to the Phase 0 prefix surface table in the
    /// plan (docs/plans/2026-05-13-002-fix-933-webhook-fallthrough-readygate-plan.md).
    #[test]
    fn test_is_unauthorized_webhook_dispatch_predicate() {
        // Row A — authorized ready-label dispatch → false
        assert!(
            !is_unauthorized_webhook_dispatch(
                "[GitHub] Issue labeled ready on senara-solutions/mika#933 — title"
            ),
            "Row A: ready-label dispatch must be allowed"
        );

        // Row B — non-ready label → true (fallthrough)
        assert!(
            is_unauthorized_webhook_dispatch(
                "[GitHub] Issue labeled bug on senara-solutions/mika#999"
            ),
            "Row B: non-ready label must be rejected"
        );
        assert!(
            is_unauthorized_webhook_dispatch(
                "[GitHub] Issue labeled p1-important on senara-solutions/mika#999"
            ),
            "Row B: non-ready label must be rejected"
        );

        // Row C — issue actions → true (fallthrough)
        assert!(
            is_unauthorized_webhook_dispatch(
                "[GitHub] Issue opened: senara-solutions/mika#100 — title"
            ),
            "Row C: issue opened must be rejected"
        );
        assert!(
            is_unauthorized_webhook_dispatch(
                "[GitHub] Issue assigned: senara-solutions/mika#100 — title"
            ),
            "Row C: issue assigned must be rejected"
        );

        // Row D — issue comments → true (the mika#932 incident class)
        assert!(
            is_unauthorized_webhook_dispatch(
                "[GitHub] New comment on senara-solutions/mika#933 (title) by @samidarko"
            ),
            "Row D: issue comment must be rejected"
        );

        // Row E — PR events → false (qa skill territory)
        assert!(
            !is_unauthorized_webhook_dispatch(
                "[GitHub] PR opened: senara-solutions/mika#1000 — title (branch: foo)"
            ),
            "Row E: PR opened must be allowed (qa skill territory)"
        );
        assert!(
            !is_unauthorized_webhook_dispatch(
                "[GitHub] PR closed: senara-solutions/mika#1000 — title (branch: foo)"
            ),
            "Row E: PR closed must be allowed (qa skill territory)"
        );

        // Row F — PR reviews → false (qa skill territory)
        assert!(
            !is_unauthorized_webhook_dispatch(
                "[GitHub] PR review (approved) on senara-solutions/mika#1000 (title) by @reviewer"
            ),
            "Row F: PR review approved must be allowed (qa skill territory)"
        );
        assert!(
            !is_unauthorized_webhook_dispatch(
                "[GitHub] PR review (changes_requested) on senara-solutions/mika#1000 (title) by @reviewer"
            ),
            "Row F: PR review changes_requested must be allowed (qa skill territory)"
        );

        // Row G — check suites → false (ci skill territory)
        assert!(
            !is_unauthorized_webhook_dispatch(
                "[GitHub] Check suite failure on senara-solutions/mika (branch: fix/foo)"
            ),
            "Row G: check suite failure must be allowed (ci skill territory)"
        );
        assert!(
            !is_unauthorized_webhook_dispatch(
                "[GitHub] Check suite success on senara-solutions/mika (branch: main)"
            ),
            "Row G: check suite success must be allowed (ci skill territory)"
        );

        // Row H — unknown event catchall → true (fail-closed)
        assert!(
            is_unauthorized_webhook_dispatch(
                "[GitHub] discussion.created on senara-solutions/mika"
            ),
            "Row H: unknown event type must be rejected (fail-closed)"
        );

        // Non-domain — not a [GitHub] prefix → false
        assert!(
            !is_unauthorized_webhook_dispatch("[claude-pilot] callback ..."),
            "Non-domain: claude-pilot prefix must not be caught"
        );
        assert!(
            !is_unauthorized_webhook_dispatch(""),
            "Non-domain: empty string must not be caught"
        );
        assert!(
            !is_unauthorized_webhook_dispatch("Implement mika#933"),
            "Non-domain: direct mika ask prompt must not be caught"
        );
    }

    // ---------------------------------------------------------------------
    // mika#2517 U1 — one body, two names.
    // ---------------------------------------------------------------------

    /// The Phase 0 prefix surface, as data, so the two names can be asserted
    /// against **the same** matrix rather than against two hand-copied ones.
    ///
    /// Deliberately a second copy of the rows
    /// `test_is_unauthorized_webhook_dispatch_predicate` asserts inline: that
    /// test is left byte-for-byte untouched, because it is what attests the
    /// mika#2517 move is a move and not a rewrite. Folding it into this table
    /// would have made it a test edited in the same commit as the code it pins.
    const PREFIX_SURFACE_MATRIX: &[(&str, bool, &str)] = &[
        (
            "[GitHub] Issue labeled ready on senara-solutions/mika#933 — title",
            false,
            "Row A — authorized ready-label dispatch",
        ),
        (
            "[GitHub] Issue labeled bug on senara-solutions/mika#999",
            true,
            "Row B — non-ready label",
        ),
        (
            "[GitHub] Issue labeled p1-important on senara-solutions/mika#999",
            true,
            "Row B — non-ready label",
        ),
        (
            "[GitHub] Issue opened: senara-solutions/mika#100 — title",
            true,
            "Row C — issue opened",
        ),
        (
            "[GitHub] Issue assigned: senara-solutions/mika#100 — title",
            true,
            "Row C — issue assigned",
        ),
        (
            "[GitHub] New comment on senara-solutions/mika#933 (title) by @samidarko",
            true,
            "Row D — issue comment (the mika#932 incident class)",
        ),
        (
            "[GitHub] PR opened: senara-solutions/mika#1000 — title (branch: foo)",
            false,
            "Row E — PR opened (qa skill territory)",
        ),
        (
            "[GitHub] PR closed: senara-solutions/mika#1000 — title (branch: foo)",
            false,
            "Row E — PR closed (qa skill territory)",
        ),
        (
            "[GitHub] PR review (approved) on senara-solutions/mika#1000 (title) by @reviewer",
            false,
            "Row F — PR review approved (qa skill territory)",
        ),
        (
            "[GitHub] PR review (changes_requested) on senara-solutions/mika#1000 (title) by @reviewer",
            false,
            "Row F — PR review changes_requested (qa skill territory)",
        ),
        (
            "[GitHub] Check suite failure on senara-solutions/mika (branch: fix/foo)",
            false,
            "Row G — check suite failure (ci skill territory)",
        ),
        (
            "[GitHub] Check suite success on senara-solutions/mika (branch: main)",
            false,
            "Row G — check suite success (ci skill territory)",
        ),
        (
            "[GitHub] discussion.created on senara-solutions/mika",
            true,
            "Row H — unknown event catchall (fail-closed)",
        ),
        (
            "[claude-pilot] callback ...",
            false,
            "Non-domain — claude-pilot prefix",
        ),
        ("", false, "Non-domain — empty string"),
        (
            "Implement mika#933",
            false,
            "Non-domain — direct mika ask prompt",
        ),
    ];

    /// **V1** — the move is a move: the eight-row matrix holds on the new name
    /// exactly as it holds on the old one.
    ///
    /// Pinning both names separately is what makes a future narrowing of the
    /// primitive legible. With one test only, an editor who tightened
    /// `is_webhook_fallthrough_domain` would see a single failure and could read
    /// it as "the alias is stale" rather than "I changed the domain".
    #[test]
    fn mika2517_the_same_matrix_holds_for_the_domain_name() {
        for (msg, expected, why) in PREFIX_SURFACE_MATRIX {
            assert_eq!(
                is_webhook_fallthrough_domain(msg),
                *expected,
                "{why}: is_webhook_fallthrough_domain({msg:?})"
            );
            assert_eq!(
                is_unauthorized_webhook_dispatch(msg),
                *expected,
                "{why}: is_unauthorized_webhook_dispatch({msg:?}) — the two names \
                 share one body and must never disagree"
            );
        }
    }

    /// **U4** — `marker_class` is a wire format: every value the classifier can
    /// return is declared, and every declared value is reachable.
    ///
    /// Both directions. Without the second half a value could be declared,
    /// grouped on by an operator, and produced by nothing — a column of zeros
    /// that reads like a healthy population.
    #[test]
    fn mika2517_marker_class_is_a_wire_format() {
        let cases = [
            (
                "[GitHub] Issue labeled bug on senara-solutions/mika#999",
                MARKER_CLASS_ISSUE_LABELED,
            ),
            (
                "[GitHub] New comment on senara-solutions/mika#933 (title) by @samidarko",
                MARKER_CLASS_ISSUE_COMMENT,
            ),
            (
                "[GitHub] Issue assigned: senara-solutions/mika#100 — title",
                MARKER_CLASS_ISSUE_ASSIGNED,
            ),
            (
                "[GitHub] Issue closed: senara-solutions/mika#100 — title",
                MARKER_CLASS_ISSUE_CLOSED,
            ),
            (
                "[GitHub] discussion.created on senara-solutions/mika",
                MARKER_CLASS_OTHER,
            ),
        ];

        for (msg, expected) in cases {
            assert_eq!(
                fallthrough_marker_class(msg),
                expected,
                "marker class of {msg:?}"
            );
            assert!(
                ALL_MARKER_CLASSES.contains(&expected),
                "{expected} is produced but not declared"
            );
        }

        let produced: Vec<&str> = cases.iter().map(|(_, c)| *c).collect();
        for declared in ALL_MARKER_CLASSES {
            assert!(
                produced.contains(declared),
                "{declared} is declared but unreachable — an operator would \
                 GROUP BY a value nothing writes"
            );
        }
    }

    /// The classifier never looks at a message the domain predicate refuses —
    /// but if a caller ever hands it one, it must not invent a class.
    #[test]
    fn mika2517_a_non_domain_message_classifies_as_other() {
        for msg in [
            "[GitHub] PR opened: senara-solutions/mika#1000 — title",
            "[GitHub] Check suite success on senara-solutions/mika (branch: main)",
            "Implement mika#933",
            "",
        ] {
            assert_eq!(fallthrough_marker_class(msg), MARKER_CLASS_OTHER);
        }
    }

    #[test]
    fn test_is_ready_label_dispatch_marker() {
        assert!(is_ready_label_dispatch_marker(
            "[GitHub] Issue labeled ready on senara-solutions/mika#933 — title"
        ));
        assert!(!is_ready_label_dispatch_marker(
            "[GitHub] Issue labeled bug on senara-solutions/mika#933"
        ));
        assert!(!is_ready_label_dispatch_marker(
            "[GitHub] New comment on senara-solutions/mika#933"
        ));
        assert!(!is_ready_label_dispatch_marker("not a github event"));
    }

    /// mika#2046 — both directions. The negative half alone would also be
    /// satisfied by a predicate that refuses everything, so the positive half is
    /// what makes this suite non-vacuous.
    #[test]
    fn test_is_dispatchable_repo_accepts_the_four_loop_repos() {
        for repo in [
            "senara-solutions/mika",
            "senara-solutions/mika-cloud",
            "senara-solutions/mika-skills",
            "senara-solutions/mika-platform",
        ] {
            assert!(
                is_dispatchable_repo(repo),
                "{repo} is a loop repository and must stay dispatchable"
            );
        }
    }

    #[test]
    fn test_is_dispatchable_repo_accepts_the_bare_short_form() {
        // The gateway emits short references for some event shapes; they resolve
        // under the default owner rather than being refused.
        for repo in ["mika", "mika-cloud", "mika-skills", "mika-platform"] {
            assert!(
                is_dispatchable_repo(repo),
                "bare {repo} must resolve under the default owner and stay dispatchable"
            );
        }
    }

    #[test]
    fn test_is_dispatchable_repo_refuses_spawn_cc_only_repos() {
        // 2026-08-29 operator decision: control-monitor and claude-pilot are
        // spawn-CC-only. Both are git repositories in the workspace, which is why
        // presence cannot be the predicate.
        for repo in [
            "control-monitor",
            "claude-pilot",
            "senara-solutions/control-monitor",
            "senara-solutions/claude-pilot",
        ] {
            assert!(
                !is_dispatchable_repo(repo),
                "{repo} is spawn-CC-only and must never be dispatchable"
            );
        }
    }

    #[test]
    fn test_is_dispatchable_repo_refuses_a_foreign_owner_with_a_familiar_basename() {
        // Matching on the basename alone would accept these: their basenames are
        // exactly the allowlisted names.
        assert!(!is_dispatchable_repo("another-org/mika"));
        assert!(!is_dispatchable_repo("attacker/mika-cloud"));
        assert!(!is_dispatchable_repo("senara-solutions-evil/mika"));
    }

    #[test]
    fn test_is_dispatchable_repo_refuses_empty_and_unknown() {
        assert!(!is_dispatchable_repo(""));
        assert!(!is_dispatchable_repo("wizzard"));
        assert!(!is_dispatchable_repo("senara-solutions/wizzard"));
    }

    #[test]
    fn test_normalize_owner_repo_applies_the_default_owner_once() {
        assert_eq!(normalize_owner_repo("mika"), "senara-solutions/mika");
        assert_eq!(
            normalize_owner_repo("senara-solutions/mika"),
            "senara-solutions/mika"
        );
        assert_eq!(normalize_owner_repo("another-org/mika"), "another-org/mika");
    }

    #[test]
    fn test_parse_repo_ref_from_dispatch_prompt_reads_both_forms() {
        assert_eq!(
            parse_repo_ref_from_dispatch_prompt("mika#2046"),
            Some("mika")
        );
        assert_eq!(
            parse_repo_ref_from_dispatch_prompt("senara-solutions/mika#2046"),
            Some("senara-solutions/mika")
        );
        assert_eq!(
            parse_repo_ref_from_dispatch_prompt("control-monitor#159"),
            Some("control-monitor")
        );
    }

    /// Regression for the bypass found in review of mika#2046: the tool-boundary
    /// gate is only load-bearing if it is at least as permissive as the shell
    /// that actually creates the worktree. `dispatch-lib.sh:769` reads the
    /// prompt through command substitution, which strips trailing newlines, so
    /// this exact string reaches the shell regex as `control-monitor#159` and
    /// matches. A parser that returned `None` here would let it through.
    #[test]
    fn test_parse_repo_ref_from_dispatch_prompt_survives_surrounding_whitespace() {
        for prompt in [
            "control-monitor#159\n",
            "  control-monitor#159  ",
            "\tcontrol-monitor#159\n\n",
            "control-monitor#159\r\n",
        ] {
            assert_eq!(
                parse_repo_ref_from_dispatch_prompt(prompt),
                Some("control-monitor"),
                "{prompt:?} reaches dispatch-lib as a repo reference and must be judged"
            );
            assert!(!is_dispatchable_repo(
                parse_repo_ref_from_dispatch_prompt(prompt).unwrap()
            ));
        }
        assert_eq!(
            parse_repo_ref_from_dispatch_prompt("mika#2046\n"),
            Some("mika")
        );
    }

    /// The shell's `grep -qE` succeeds when any line matches, so a multi-line
    /// prompt whose first line is a repo reference still routes into worktree
    /// mode. The gate must see it too.
    #[test]
    fn test_parse_repo_ref_from_dispatch_prompt_reads_multiline_prompts() {
        let iteration = "control-monitor#159\n\nITERATION CONTEXT:\nfix the thing";
        assert_eq!(
            parse_repo_ref_from_dispatch_prompt(iteration),
            Some("control-monitor")
        );
        let legit = "mika#2046\n\nITERATION CONTEXT:\nfix the thing";
        assert_eq!(parse_repo_ref_from_dispatch_prompt(legit), Some("mika"));
    }

    #[test]
    fn test_parse_repo_ref_from_dispatch_prompt_ignores_free_text() {
        // Free text resolves no repository, so the allowlist must not judge it.
        for prompt in [
            "fix the ready-label handler",
            "implement mika#2046 with care",
            "see #2046",
            "mika#",
            "mika#abc",
            "#2046",
            "",
            "a/b/c#1",
            "please look at control-monitor#159 when you get a chance",
        ] {
            assert_eq!(
                parse_repo_ref_from_dispatch_prompt(prompt),
                None,
                "{prompt:?} is not an anchored repo#number reference"
            );
        }
    }

    #[test]
    fn test_dispatchable_repos_display_names_every_allowed_repo() {
        let shown = dispatchable_repos_display();
        for repo in DISPATCHABLE_REPOS {
            assert!(
                shown.contains(repo),
                "a refusal must be able to quote {repo}; display was {shown}"
            );
        }
    }

    // ─────────────── Dispatch seat gate (mika#2084) ───────────────
    //
    // Anti-vacuity runs in BOTH directions here, deliberately. A gate that
    // refused every issue would satisfy every refusal test below on its own —
    // and it would also stop the entire loop, which is a worse outcome than the
    // collision the gate prevents. So each refusal case is paired with a pass
    // case, and the pass cases are the ones to look at first when this module
    // is edited.

    /// AC3 / AC4 positive half — the load-bearing case.
    ///
    /// The overwhelming majority of tickets carry no seat label at all. If this
    /// test ever goes red, the loop has stopped dispatching.
    #[test]
    fn no_seat_label_still_dispatches() {
        let verdict = classify_dispatch_seat(["bug", "p1-important", "ready"]);
        assert_eq!(verdict, SeatVerdict::NoSeatLabel);
        assert!(
            !verdict.refuses(),
            "an unlabelled issue must still dispatch"
        );
        assert_eq!(verdict.refusal_reason(), None);

        // The empty label set is the same case.
        let empty = classify_dispatch_seat(std::iter::empty::<&str>());
        assert_eq!(empty, SeatVerdict::NoSeatLabel);
        assert!(!empty.refuses());
    }

    /// AC4 positive half — a ticket labelled for our own seat proceeds.
    #[test]
    fn current_seat_label_still_dispatches() {
        let label = format!("{DISPATCH_SEAT_LABEL_PREFIX}{CURRENT_DISPATCH_SEAT}");
        let verdict = classify_dispatch_seat([label.as_str(), "bug"]);
        assert_eq!(
            verdict,
            SeatVerdict::OwnedByCurrentSeat {
                label: label.clone()
            }
        );
        assert!(!verdict.refuses(), "our own seat must not be refused");
    }

    /// AC1 — the 2026-08-30 collision. mika#2055 carried `dispatch:ssc` while
    /// SSC had PR#2082 open; the loop dispatched anyway.
    #[test]
    fn other_seat_label_is_refused() {
        for (label, seat) in [("dispatch:ssc", "ssc"), ("dispatch:mpc", "mpc")] {
            let verdict = classify_dispatch_seat([label, "bug", "ready"]);
            assert_eq!(
                verdict,
                SeatVerdict::OwnedByOtherSeat {
                    label: label.to_string(),
                    seat: seat.to_string(),
                },
                "{label} must be refused"
            );
            assert!(verdict.refuses());
            assert_eq!(verdict.refusal_reason(), Some("seat_owned_by_other"));
            assert_eq!(verdict.label(), Some(label));
        }
    }

    /// AC2 — fail-closed. A seat we cannot resolve is not an authorization.
    #[test]
    fn unknown_seat_label_is_refused() {
        let verdict = classify_dispatch_seat(["dispatch:zorglub"]);
        assert!(
            verdict.refuses(),
            "an unknown seat must not be waved through"
        );
        assert_eq!(verdict.refusal_reason(), Some("unknown_seat"));
    }

    /// AC2 — a bare `dispatch:` names no seat, so it resolves to none.
    #[test]
    fn empty_seat_label_is_refused() {
        for label in ["dispatch:", "dispatch:   "] {
            let verdict = classify_dispatch_seat([label]);
            assert!(verdict.refuses(), "{label} must be refused");
            assert_eq!(verdict.refusal_reason(), Some("empty_seat"));
        }
    }

    /// AC2 — two seats claimed, neither wins. Picking one would invent an owner.
    #[test]
    fn multiple_seat_labels_are_refused() {
        let verdict = classify_dispatch_seat(["dispatch:ssc", "dispatch:mpc", "bug"]);
        assert!(verdict.refuses());
        assert_eq!(verdict.refusal_reason(), Some("multiple_seat_labels"));

        // Even two labels naming our own seat are ambiguous, not permission.
        let ours = format!("{DISPATCH_SEAT_LABEL_PREFIX}{CURRENT_DISPATCH_SEAT}");
        let dup = classify_dispatch_seat([ours.as_str(), "dispatch:ssc"]);
        assert!(dup.refuses());
    }

    /// A label typed in the GitHub UI with different casing claims the same seat.
    #[test]
    fn seat_label_is_case_insensitive() {
        let verdict = classify_dispatch_seat(["Dispatch:SSC"]);
        assert_eq!(
            verdict,
            SeatVerdict::OwnedByOtherSeat {
                label: "dispatch:ssc".to_string(),
                seat: "ssc".to_string(),
            }
        );
        assert!(verdict.refuses());
    }

    /// AC3 — the false-positive that would stop the loop.
    ///
    /// The prefix is `dispatch:` exactly. Labels that merely begin with the
    /// letters `dispatch` are ordinary labels; reading them as seat claims would
    /// refuse tickets nobody ever claimed.
    #[test]
    fn labels_merely_starting_with_dispatch_are_not_seat_labels() {
        let verdict = classify_dispatch_seat([
            "dispatched",
            "dispatch-ready",
            "dispatch",
            "wip-rescue-dispatch",
        ]);
        assert_eq!(verdict, SeatVerdict::NoSeatLabel);
        assert!(
            !verdict.refuses(),
            "near-miss names must not enter the gate"
        );
    }

    /// D4 anti-drift — the two prompt parses must agree on what a repository
    /// reference is. Two independent parses of the same string is the drift
    /// this shared implementation exists to prevent.
    #[test]
    fn parse_issue_ref_agrees_with_parse_repo_ref() {
        for prompt in [
            "mika#2055",
            "senara-solutions/mika#2055",
            "control-monitor#159\n",
            "  mika-cloud#127  ",
            "first line is prose\nmika-skills#8\nmore prose",
            "free text with a # in it",
            "mika#",
            "#123",
        ] {
            let repo_only = parse_repo_ref_from_dispatch_prompt(prompt);
            let with_number = parse_issue_ref_from_dispatch_prompt(prompt);
            assert_eq!(
                repo_only,
                with_number.map(|(r, _)| r),
                "parses disagree on {prompt:?}"
            );
        }

        assert_eq!(
            parse_issue_ref_from_dispatch_prompt("senara-solutions/mika#2055"),
            Some(("senara-solutions/mika", 2055))
        );

        // The single deliberate asymmetry: a number too large for `u64` still
        // parses as a repository reference but yields no issue, so the seat gate
        // simply has nothing to look up (mika#2084 D2 — missing information
        // lets the dispatch through rather than refusing it).
        let overflow = "mika#99999999999999999999999";
        assert_eq!(parse_repo_ref_from_dispatch_prompt(overflow), Some("mika"));
        assert_eq!(parse_issue_ref_from_dispatch_prompt(overflow), None);
    }

    /// AC5 — the refusal sentence must not claim an owner that does not exist.
    ///
    /// `dispatch:zorglub` is a typo, not a collision. Telling the operator that
    /// "another seat already owns this" sends them looking for a colliding seat
    /// that was never there, and inflates any tally of avoided collisions.
    #[test]
    fn refusal_sentence_distinguishes_a_foreign_seat_from_an_unresolved_one() {
        let foreign = classify_dispatch_seat(["dispatch:ssc"]);
        let sentence = seat_refusal_sentence(&foreign);
        assert!(sentence.contains("dispatch:ssc"));
        assert!(sentence.contains("owns it"));

        for (labels, expected_fragment) in [
            (vec!["dispatch:zorglub"], "does not know"),
            (vec!["dispatch:"], "names no seat"),
            (
                vec!["dispatch:ssc", "dispatch:mpc"],
                "more than one seat label",
            ),
        ] {
            let verdict = classify_dispatch_seat(labels.clone());
            let sentence = seat_refusal_sentence(&verdict);
            assert!(
                sentence.contains(expected_fragment),
                "{labels:?} should say {expected_fragment:?}, got: {sentence}"
            );
            assert!(
                !sentence.contains("owns it"),
                "{labels:?} has no identified owner — claiming one misleads the \
                 operator: {sentence}"
            );
        }
    }

    // ---------------------------------------------------------------------
    // mika#2484 — l'intention de grooming, et l'ancrage sur le mot.
    // ---------------------------------------------------------------------

    /// La forme mesurée sur #2471, et les deux autres que la table de routage
    /// de `self-dev` reconnaît.
    #[test]
    fn mika2484_une_demande_de_grooming_est_reconnue() {
        for msg in [
            "groom mika issue#2471",
            "groom mika#2471",
            "groom ticket mika#2471",
            "groom senara-solutions/mika issue#2471 en priorité",
        ] {
            assert!(
                is_grooming_intent_message(msg),
                "{msg:?} est une intention de grooming explicite"
            );
        }
    }

    /// **Test 8 — le contrôle négatif du mot, et c'est le test porteur.**
    ///
    /// `starts_with("groom")` nu mordrait sur « grooming report » : une demande
    /// de *rapport* deviendrait un refus de dispatch. C'est l'espace obligatoire
    /// qui sépare `groom ` de `grooming`, et ce test est ce qui empêche
    /// quelqu'un de « simplifier » le prédicat en le cassant.
    #[test]
    fn mika2484_grooming_report_n_est_pas_une_intention() {
        for msg in [
            "grooming report for mika#2471",
            "grooming status",
            "groomed tickets this week",
            "groom",
            "",
        ] {
            assert!(
                !is_grooming_intent_message(msg),
                "{msg:?} n'est PAS une demande de grooming — un faux positif ici \
                 refuse un dispatch légitime"
            );
        }
    }

    /// **Test 9 — les quatre chemins moteur ne mordent pas.**
    ///
    /// Propriété structurelle et non prudentielle : trois d'entre eux ont
    /// `originating_message = None` (auto-fire mika#1614, tour de callback), et
    /// le quatrième porte un préfixe `[GitHub]`. Ce test épingle la moitié
    /// observable — qu'aucun texte réel de ces chemins ne commence par `groom `.
    #[test]
    fn mika2484_les_quatre_chemins_moteur_ne_mordent_pas() {
        for msg in [
            // Chemin webhook ready-label.
            "[GitHub] Issue labeled ready on senara-solutions/mika#2471 — reprise",
            // Relance de verdict : le texte d'une revue de PR.
            "[GitHub] PR review submitted on senara-solutions/mika#2483\nVERDICT: block[ci]",
            // Un tour de callback.
            "[callback: long_running:run_claude_pilot] Outcome: PLAN_GROOMED",
            // La forme typée d'une implémentation.
            "implement mika issue#2471",
        ] {
            assert!(
                !is_grooming_intent_message(msg),
                "{msg:?} est un chemin moteur : la garde d'intention doit être \
                 structurellement hors de sa route (R8)"
            );
        }
        // Le cas `None` n'est pas un texte : la garde ne s'arme que sous
        // `if let Some(msg) = originating_message`, et c'est ce qui couvre
        // l'auto-fire et le tour de callback.
    }

    /// **Test 10 — insensibilité à la casse et tolérance au blanc de tête.**
    #[test]
    fn mika2484_casse_et_blanc_de_tete() {
        for msg in [
            "Groom mika issue#2471",
            "GROOM mika#2471",
            "  groom mika issue#2471",
            "\n\tgroom mika#2471",
            "groom\tmika#2471",
        ] {
            assert!(
                is_grooming_intent_message(msg),
                "{msg:?} — un opérateur écrit indifféremment"
            );
        }
    }

    // ─────────────────────────────────────────────────────────────────────
    // mika#2649 — la cible de l'événement, et le vocabulaire d'audit.
    // ─────────────────────────────────────────────────────────────────────

    /// **V10 — la cible est lue par les lecteurs uniques, sur les grammaires
    /// réelles de `format_event_text`.**
    ///
    /// Les textes sont ceux que `mika_gateway::github::format_event_text`
    /// produit, repris verbatim des fixtures du dépôt — un test écrit sur une
    /// grammaire inventée attesterait de l'invention.
    #[test]
    fn mika2649_la_cible_devenement_est_lue_par_les_lecteurs_uniques() {
        // Forme `PR review` — celle de l'incident du 2026-10-02.
        assert_eq!(
            webhook_event_target(
                "[GitHub] PR review (approved) on senara-solutions/mika#2647 (un titre) by @samidarko"
            ),
            WebhookEventTarget::Pr {
                repo: "senara-solutions/mika".to_string(),
                number: 2647,
            },
        );

        // Forme `PR {action}:` — celle de la cascade de jalon M4.
        assert_eq!(
            webhook_event_target("[GitHub] PR closed: senara-solutions/mika#2600 — un titre"),
            WebhookEventTarget::Pr {
                repo: "senara-solutions/mika".to_string(),
                number: 2600,
            },
        );

        // Check-suite : la grammaire porte une BRANCHE, et le numéro d'issue en
        // est le deuxième segment.
        assert_eq!(
            webhook_event_target(
                "[GitHub] Check suite failure on senara-solutions/mika (branch: fix/2646/slug)"
            ),
            WebhookEventTarget::Branch {
                repo: "senara-solutions/mika".to_string(),
                issue: Some(2646),
            },
        );

        // Une branche non conforme ne porte aucun numéro — jamais inventé.
        assert_eq!(
            webhook_event_target(
                "[GitHub] Check suite success on senara-solutions/mika (branch: main)"
            ),
            WebhookEventTarget::Branch {
                repo: "senara-solutions/mika".to_string(),
                issue: None,
            },
        );

        // Le dépôt est normalisé par le lecteur unique de mika#2046, donc une
        // forme courte du gateway s'aligne sur la forme des `reference_url`.
        assert_eq!(
            webhook_event_target("[GitHub] PR closed: mika#2600 — un titre"),
            WebhookEventTarget::Pr {
                repo: "senara-solutions/mika".to_string(),
                number: 2600,
            },
        );

        // Préfixe présent, grammaire non parsée : la grammaire a bougé sous le
        // lecteur. Autorise, mais sous son propre nom.
        assert_eq!(
            webhook_event_target("[GitHub] PR quelque chose que personne n'émet"),
            WebhookEventTarget::Unreadable,
        );

        // Hors population : tout le reste.
        for msg in [
            "Implement mika#2649",
            "[GitHub] Issue labeled ready on senara-solutions/mika#2649 — titre",
            "[GitHub] New comment on senara-solutions/mika#2649 (titre) by @samidarko",
            "[callback: long_running:run_claude_pilot]",
            "",
        ] {
            assert_eq!(
                webhook_event_target(msg),
                WebhookEventTarget::NotApplicable,
                "{msg:?} ne désigne aucune cible de PR / check-suite"
            );
        }
    }

    /// **V10-bis — les deux termes purs de lignée.**
    ///
    /// Le contrôle porteur est le **strict sur le type de référence** : un
    /// événement PR #2647 ne doit PAS être apparié à une `reference_url`
    /// d'issue #2647 — ce serait une coïncidence de numéro, pas une lignée.
    #[test]
    fn mika2649_les_deux_termes_purs_sont_stricts_sur_le_type() {
        let pr_event = WebhookEventTarget::Pr {
            repo: "senara-solutions/mika".to_string(),
            number: 2647,
        };

        // L1 positif.
        assert!(reference_url_names_target(
            &pr_event,
            Some("https://github.com/senara-solutions/mika/pull/2647")
        ));
        // L1 — coïncidence de numéro sur un AUTRE type d'objet : refusé.
        assert!(!reference_url_names_target(
            &pr_event,
            Some("https://github.com/senara-solutions/mika/issues/2647")
        ));
        // L1 — même numéro, autre dépôt.
        assert!(!reference_url_names_target(
            &pr_event,
            Some("https://github.com/senara-solutions/mika-cloud/pull/2647")
        ));
        // L1 — un signal illisible n'est jamais un terme satisfait.
        for url in [None, Some(""), Some("pas une url"), Some("mika#2647")] {
            assert!(!reference_url_names_target(&pr_event, url), "{url:?}");
        }

        // L2 positif — le lien que le producteur du dispatch a estampillé.
        assert!(pilot_pr_url_names_target(
            &pr_event,
            Some("https://github.com/senara-solutions/mika/pull/2647")
        ));
        assert!(!pilot_pr_url_names_target(
            &pr_event,
            Some("https://github.com/senara-solutions/mika/pull/2646")
        ));
        // L2 est inapplicable à une cible `Branch` : une URL de PR ne dit pas
        // quelle issue la PR ferme.
        let branch_event = WebhookEventTarget::Branch {
            repo: "senara-solutions/mika".to_string(),
            issue: Some(2646),
        };
        assert!(!pilot_pr_url_names_target(
            &branch_event,
            Some("https://github.com/senara-solutions/mika/pull/2647")
        ));
        // …et L1 sur une cible `Branch` apparie une ISSUE, pas une PR.
        assert!(reference_url_names_target(
            &branch_event,
            Some("https://github.com/senara-solutions/mika/issues/2646")
        ));
        assert!(!reference_url_names_target(
            &branch_event,
            Some("https://github.com/senara-solutions/mika/pull/2646")
        ));
        // Une branche sans numéro n'apparie rien, même pas elle-même.
        let no_target = WebhookEventTarget::Branch {
            repo: "senara-solutions/mika".to_string(),
            issue: None,
        };
        assert!(!reference_url_names_target(
            &no_target,
            Some("https://github.com/senara-solutions/mika/issues/2646")
        ));
    }

    /// **V11 — le vocabulaire d'audit est un format de fil.**
    ///
    /// Ces cinq valeurs atterrissent dans `audit_events.after_value` et un
    /// opérateur en fait des `GROUP BY` : deux orthographes d'une même issue
    /// couperaient une population en deux sans le dire. Site de définition
    /// unique, et le `match` d'`audit_value` est exhaustif **sans bras `_ =>`**
    /// — ce test épingle la valeur de chaque bras.
    #[test]
    fn mika2649_le_vocabulaire_daudit_est_un_format_de_fil() {
        assert_eq!(TARGET_BINDING_AUDIT_TOOL, "webhook_dispatch_target_binding");
        assert_eq!(TARGET_BINDING_BOUND, "bound");
        assert_eq!(TARGET_BINDING_REFUSED, "refused");
        assert_eq!(TARGET_BINDING_EVENT_UNREADABLE, "event_unreadable");
        assert_eq!(TARGET_BINDING_NO_TARGET_IN_EVENT, "no_target_in_event");
        assert_eq!(TARGET_BINDING_LINEAGE_UNREADABLE, "lineage_unreadable");

        // Chaque état rend sa valeur, et le registre les porte toutes.
        for (state, expected) in [
            (
                TargetBinding::Bound(LineageTerm::TaskReference),
                TARGET_BINDING_BOUND,
            ),
            (TargetBinding::Refused, TARGET_BINDING_REFUSED),
            (
                TargetBinding::EventUnreadable,
                TARGET_BINDING_EVENT_UNREADABLE,
            ),
            (
                TargetBinding::NoTargetInEvent,
                TARGET_BINDING_NO_TARGET_IN_EVENT,
            ),
            (
                TargetBinding::LineageUnreadable,
                TARGET_BINDING_LINEAGE_UNREADABLE,
            ),
        ] {
            assert_eq!(state.audit_value(), expected);
            assert!(
                ALL_TARGET_BINDING_VERDICTS.contains(&expected),
                "{expected} doit être au registre que l'opérateur lit"
            );
        }

        // Le registre ne porte que ces cinq valeurs, et aucune en double : un
        // doublon rendrait un `GROUP BY` ambigu sans le dire.
        assert_eq!(ALL_TARGET_BINDING_VERDICTS.len(), 5);
        let mut sorted = ALL_TARGET_BINDING_VERDICTS.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), 5, "aucune valeur en double au registre");

        // Les quatre termes de lignée ont eux aussi un nom stable : c'est lui
        // qui dit à l'opérateur LEQUEL a tenu, donc quelle moitié du prédicat
        // lire quand la cascade de jalon est refusée (halte 2 de la sonde S2).
        assert_eq!(LineageTerm::TaskReference.as_str(), "task_reference_url");
        assert_eq!(LineageTerm::TaskPilotPrUrl.as_str(), "task_pilot_pr_url");
        assert_eq!(LineageTerm::Sibling.as_str(), "sibling");
        assert_eq!(LineageTerm::Parent.as_str(), "parent");
    }

    /// **La frontière est lue d'un seul côté.**
    ///
    /// `is_webhook_fallthrough_domain` sort les deux familles du domaine
    /// Fallthrough ; `webhook_event_target` les y retrouve pour les borner. Les
    /// deux lisent les **mêmes** constantes de préfixe depuis mika#2649, donc
    /// l'invariant est : *tout message sorti du domaine par l'un des deux
    /// préfixes porte une cible, lisible ou non.* Une divergence future laisserait
    /// une famille hors du domaine ET hors de la garde — exactement le trou de
    /// mika#2649.
    #[test]
    fn mika2649_tout_message_hors_domaine_par_prefixe_porte_une_cible() {
        for msg in [
            "[GitHub] PR review (approved) on senara-solutions/mika#1 (t) by @x",
            "[GitHub] PR closed: senara-solutions/mika#1 — t",
            "[GitHub] PR une-forme-inconnue",
            "[GitHub] Check suite success on senara-solutions/mika (branch: main)",
            "[GitHub] Check suite une-forme-inconnue",
        ] {
            assert!(
                !is_webhook_fallthrough_domain(msg),
                "{msg:?} doit être hors du domaine Fallthrough"
            );
            assert_ne!(
                webhook_event_target(msg),
                WebhookEventTarget::NotApplicable,
                "{msg:?} est hors du domaine Fallthrough, donc la garde de lignée doit \
                 l'interroger — un message hors des deux est le trou de mika#2649"
            );
        }

        // Le contrôle négatif : ce qui RESTE dans le domaine n'est pas interrogé
        // par la garde de lignée (le domaine a sa propre garde, gate 0).
        for msg in [
            "[GitHub] Issue labeled bug on senara-solutions/mika#1",
            "[GitHub] New comment on senara-solutions/mika#1 (t) by @x",
        ] {
            assert!(is_webhook_fallthrough_domain(msg), "{msg:?}");
            assert_eq!(
                webhook_event_target(msg),
                WebhookEventTarget::NotApplicable,
                "{msg:?} reste dans le domaine Fallthrough : gate 0 le juge, pas la lignée"
            );
        }
    }
}
