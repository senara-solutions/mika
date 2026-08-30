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
    if !msg.starts_with("[GitHub]") {
        return false;
    }
    if msg.starts_with(READY_LABEL_DISPATCH_MARKER) {
        return false;
    }
    // qa skill territory (Phase 0 prefix surface rows E, F).
    if msg.starts_with("[GitHub] PR ") {
        return false;
    }
    // ci skill territory (Phase 0 prefix surface row G).
    if msg.starts_with("[GitHub] Check suite ") {
        return false;
    }
    // Everything else in [GitHub] domain (rows B, C, D, H) is fallthrough.
    true
}

/// True when the message matches the ready-label dispatch marker prefix.
pub(crate) fn is_ready_label_dispatch_marker(msg: &str) -> bool {
    msg.starts_with(READY_LABEL_DISPATCH_MARKER)
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
pub(crate) const KNOWN_DISPATCH_SEATS: &[&str] = &["loop", "ssc", "mpc"];

/// What the seat labels on one issue say about whether this engine may take it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SeatVerdict {
    /// No `dispatch:*` label at all — the overwhelmingly common case, and the
    /// load-bearing one. Behaviour must be **identical** to the pre-#2084 path
    /// (mika#2084 AC3): a fix that turns "unlabelled" into "refused" stops the
    /// whole loop, which is worse than the defect it repairs.
    NoSeatLabel,
    /// Labelled for this engine's own seat — dispatch proceeds.
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

/// The known-seat list rendered for a refusal message, so a refusal states what
/// would have been accepted and not only what was denied (same discipline as
/// [`dispatchable_repos_display`]).
pub(crate) fn known_dispatch_seats_display() -> String {
    KNOWN_DISPATCH_SEATS.join(", ")
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
}
