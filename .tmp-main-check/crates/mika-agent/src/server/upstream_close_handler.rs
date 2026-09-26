//! Complete-on-upstream-close cleanup for phantom tracking rows (mika#1934 AC4).
//!
//! The escalation surface writes a tracking row's parent-status to `blocked` and
//! then never converts it to a terminal state, because the operator resolves the
//! underlying ticket out-of-band on GitHub (merges the fix in a different PR,
//! closes the issue, supersedes it). None of those out-of-band events reach back
//! to the tracking row today — the mika#1712 sweep is the only thing that drains
//! them, at a 3600s grace. This handler closes that gap synchronously: when an
//! `issues.closed` or `pull_request.closed` webhook arrives, it terminal-marks
//! any `blocked`/`in_progress` tracking rows whose `reference_url` matches the
//! closed issue.
//!
//! Side-effect-only injector: it never returns `Handled`/`Dispatched` — the LLM
//! still owns whatever the webhook turn does. Mirrors the shape of
//! `milestone_context_handler` and `draft_pr_opened_handler`.
//!
//! Idempotent + no-op safe (AC4.c): the guarded UPDATE short-circuits on
//! already-terminal rows (re-delivery safe), and a webhook with zero matching
//! rows logs at DEBUG only — no WARN, no audit event.

use super::verdict_handler::VerdictAction;
use crate::async_db::AsyncDatabase;
use crate::task_state::tasks::{
    ISSUE_CLOSED_UPSTREAM, TRACKING_ROW_UPSTREAM_CLOSED_TOOL, UPSTREAM_PR_CLOSED_UNMERGED,
    UPSTREAM_PR_MERGED, strip_groom_phase_suffix,
};
use std::sync::LazyLock;
use tracing::{debug, info, warn};

/// Prefix on issue-closed webhook messages from the gateway.
const ISSUE_CLOSED_PREFIX: &str = "[GitHub] Issue closed:";

/// Prefix on PR-closed webhook messages from the gateway.
const PR_CLOSED_PREFIX: &str = "[GitHub] PR closed:";

/// Sentinel line indicating a merged PR (gateway appends this on close events).
const MERGED_TRUE_LINE: &str = "\nMerged: true";

/// Suffix `truncate_body` appends when it cut a webhook body
/// (`mika-gateway/src/github.rs`).
///
/// The coupling to the gateway's formatting is the one this module **already**
/// assumes for `ISSUE_CLOSED_PREFIX`, `PR_CLOSED_PREFIX` and `MERGED_TRUE_LINE`,
/// and it is fail-open: if the gateway changes the marker we lose the R6
/// instrument, never the behaviour. Declared in `scripts/canonical-tokens.tsv`
/// next to this file's two other entries — on déclare, on n'allowliste pas.
const TRUNCATED_BODY_MARKER: &str = "[truncated]";

/// `audit_events.tool_name` of the mika#2242 dé-groomage marker.
///
/// # The name states what was MEASURED, never its interpretation
///
/// A closing PR that is closed without merging unties its `Closes #N`
/// sub-tickets from the plan the PR's branch carries. Whether `#N` was
/// *groomed* is not knowable here: this handler holds the PR's body, not the
/// issue's, and fetching it would be a network read in a handler that performs
/// none. "Dé-groomé" is an interpretation, and the entitled writer is the
/// reader — `server::ready_label_handler`, which holds the issue body and finds
/// the callouts missing.
///
/// # SOLE WRITER
///
/// This module is the only production site writing this name, which is what
/// makes the `GROUP BY` of `CLAUDE.md` § *un dé-groomage est attribuable*
/// subtractible against its sibling `ready_label_degroomed`. Pinned by
/// `canonical_tokens::tests::mika2242_the_two_audit_names_have_a_single_writer`.
pub const CLOSING_PR_CLOSED_UNMERGED_TOOL: &str = "closing_pr_closed_unmerged";

/// `audit_events.target_key` of the marker, and the key the reader queries by
/// **exact equality** — never a `LIKE`, which would match `#234` against
/// `#2343` (the mika#2347 trap).
///
/// One writer, one reader, one function: the two halves cannot spell the key
/// differently, which is the only way they could fail to meet.
pub fn degroom_marker_key(owner_repo: &str, issue_number: u64) -> String {
    format!("issue:{owner_repo}#{issue_number}")
}

/// Matches closing keywords (`Closes`/`Fixes`/`Resolves`, any inflection) plus a
/// same-repo issue reference (`#<n>`) in a PR body. Case-insensitive.
static CLOSING_REF_RE: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(r"(?i)\b(?:clos(?:e|es|ed)|fix(?:e|es|ed)|resolv(?:e|es|ed))\s+#(\d+)")
        .expect("closing-ref regex is valid")
});

/// Inspect an `issues.closed` / `pull_request.closed` webhook message and
/// terminal-mark any phantom tracking rows whose `reference_url` matches the
/// closed issue (mika#1934 AC4).
///
/// Always returns `Passthrough` — side-effect only. `Some(enrichment)` is never
/// produced; the handler mutates DB rows, it does not rewrite the message.
pub async fn try_handle_upstream_close(
    text: &str,
    db: &AsyncDatabase,
    session_id: &str,
    trace_id: &str,
) -> VerdictAction {
    if text.starts_with(ISSUE_CLOSED_PREFIX) {
        handle_issue_closed(text, db, session_id, trace_id).await;
    } else if text.starts_with(PR_CLOSED_PREFIX) {
        handle_pr_closed(text, db, session_id, trace_id).await;
    }
    // Not a close event we own, or handled above — either way, pass through.
    VerdictAction::Passthrough { enrichment: None }
}

/// `issues.closed`: the closed issue's own URL is the tracking `reference_url`.
async fn handle_issue_closed(text: &str, db: &AsyncDatabase, session_id: &str, trace_id: &str) {
    let Some(issue_url) = extract_issue_url(text) else {
        debug!("upstream_close: no issue URL in issues.closed message");
        return;
    };
    cleanup_rows_for_issue_url(
        db,
        session_id,
        trace_id,
        &issue_url,
        "issues.closed",
        "cancelled",
        ISSUE_CLOSED_UPSTREAM,
    )
    .await;
}

/// `pull_request.closed`: the tracking `reference_url` is the ISSUE the PR
/// closes, parsed from the PR body's `Closes #<n>` / `Fixes #<n>` refs. A merged
/// PR completes the row; an unmerged close cancels it.
async fn handle_pr_closed(text: &str, db: &AsyncDatabase, session_id: &str, trace_id: &str) {
    let Some(pr_url) = extract_pr_url(text) else {
        debug!("upstream_close: no PR URL in pull_request.closed message");
        return;
    };
    let Some((owner, repo)) = extract_owner_repo_from_url(&pr_url) else {
        debug!("upstream_close: could not parse owner/repo from PR URL");
        return;
    };

    let merged = text.contains(MERGED_TRUE_LINE);
    let (new_status, result) = if merged {
        ("completed", UPSTREAM_PR_MERGED)
    } else {
        ("cancelled", UPSTREAM_PR_CLOSED_UNMERGED)
    };

    let issue_numbers = parse_closing_issue_refs(text);
    if issue_numbers.is_empty() {
        // mika#2242 R6 — the blind spot, said out loud rather than inherited in
        // silence. `format_event_text` truncates the PR body at 2 000 chars, and
        // `parse_closing_issue_refs` reads THAT body: an umbrella's `Closes #N`
        // lines are typically long-bodied, so they can fall past the cut. The
        // producer below would then be inert for exactly the population it
        // targets — and a silently inert detector reads exactly like a healthy
        // one (mika#2205).
        //
        // Scoped to the unmerged branch because that is the only branch that
        // writes a marker; a truncated merged close loses nothing this ticket
        // adds. Named and instrumented, NOT corrected: closing it means fetching
        // the full body over the network, which changes the population of the
        // row cleanup below — a blast radius R2 forbids. Its follow-up is
        // conditioned on this line being non-empty.
        if !merged && body_is_truncated(text) {
            warn!(
                event = "closing_pr_body_truncated_no_refs",
                pr_url = %pr_url,
                "upstream_close: unmerged PR closed with a TRUNCATED body and no \
                 Closes/Fixes ref parsed — a dé-groomage marker may be missing \
                 (mika#2242 R6)"
            );
        }
        debug!(
            pr_url = %pr_url,
            "upstream_close: PR body carries no Closes/Fixes issue refs — nothing to clean up"
        );
        return;
    }

    // Read once, outside the loop: the branch is a property of the PR, not of
    // the ref. Free — it rides on the webhook header the gateway already writes.
    let head_branch = extract_head_branch(text);

    for number in issue_numbers {
        let issue_url = format!("https://github.com/{owner}/{repo}/issues/{number}");

        // mika#2242 — on the unmerged branch ONLY. A merged PR closes its issue;
        // there is no dé-groomage to attribute. The write is fire-and-forget and
        // precedes nothing: it must not be able to fail the row cleanup below
        // (R2), so its failure is a `warn!` exactly like the `log_audit_event`
        // already inside `cleanup_rows_for_issue_url`.
        if !merged {
            record_closing_pr_closed_unmerged(
                db,
                session_id,
                trace_id,
                &format!("{owner}/{repo}"),
                number,
                &pr_url,
                head_branch.as_deref(),
            )
            .await;
        }

        cleanup_rows_for_issue_url(
            db,
            session_id,
            trace_id,
            &issue_url,
            "pull_request.closed",
            new_status,
            result,
        )
        .await;
    }
}

/// Stamp the durable record that PR `pr_url` was closed **without merging**
/// while declaring `Closes #issue_number` (mika#2242 R1).
///
/// The link "umbrella → sub-ticket" is readable at exactly one instant — this
/// one, where the PR's body is in hand. From the sub-ticket afterwards it is not
/// reconstructible: an umbrella plan's canonical filename carries no sub-ticket
/// number (the `<issue>` slot holds the word `umbrella`), and GitHub exposes
/// `PullRequest.closingIssuesReferences` but no issue → closing-PRs direction.
/// So this is a fact stamped by its producer, never reconstructed afterwards —
/// the rule mika#2026 wrote for PR origin and mika#2249 for the dispatch
/// worktree.
async fn record_closing_pr_closed_unmerged(
    db: &AsyncDatabase,
    session_id: &str,
    trace_id: &str,
    owner_repo: &str,
    issue_number: u64,
    pr_url: &str,
    head_branch: Option<&str>,
) {
    let target_key = degroom_marker_key(owner_repo, issue_number);
    let pr_ref = pr_reference(pr_url);
    // A `clé=valeur` record separated by spaces — the convention already in
    // force in this file (`event_type={} reference_url={}`) and in
    // `ready_label_handler` (`repo={} number={} target_skill={} groomed={}`).
    // Not a new format; the house's.
    //
    // An unreadable branch omits the key rather than writing an empty one: the
    // reader's DECISION rests on the row's PRESENCE, and the branch is an
    // enrichment for the operator. A `head_branch=` with nothing after it would
    // read as a branch literally named "".
    let reasoning = match head_branch {
        Some(branch) => format!("pr_url={pr_url} head_branch={branch} merged=false"),
        None => format!("pr_url={pr_url} merged=false"),
    };

    if let Err(e) = db
        .log_audit_event(
            session_id,
            CLOSING_PR_CLOSED_UNMERGED_TOOL,
            &target_key,
            None,
            Some(&pr_ref),
            Some(&reasoning),
            Some(trace_id),
        )
        .await
    {
        warn!(
            event = "closing_pr_marker_write_failed",
            target_key = %target_key,
            pr_url = %pr_url,
            error = %e,
            "upstream_close: failed to write the dé-groomage marker (non-fatal)"
        );
        return;
    }

    info!(
        event = "closing_pr_closed_unmerged",
        issue = %target_key,
        pr = %pr_ref,
        pr_url = %pr_url,
        head_branch = head_branch.unwrap_or("<unknown>"),
        "upstream_close: closing PR closed without merging — its sub-ticket is \
         untied from the plan that branch carries (mika#2242)"
    );
}

/// `true` when the gateway cut this message's body.
///
/// The body is the **last** segment `format_event_text` appends for a `closed`
/// action, so the marker lands at the end of the text. Anchoring on the tail is
/// what separates it from prose that merely *contains* the word — the false
/// positive class mika#2050 measured on Signal S, where a pilot writing *about*
/// a marker was counted as an emission of it.
fn body_is_truncated(text: &str) -> bool {
    text.trim_end().ends_with(TRUNCATED_BODY_MARKER)
}

/// The PR's head branch, from the header line the gateway writes:
/// `[GitHub] PR closed: {repo}#{n} — {title} (branch: {branch})`.
///
/// Read from the **first** line only, and from its **last** `(branch: …)` run,
/// so a PR whose title happens to contain that shape cannot displace the real
/// one. Returns `None` on anything else — an unreadable branch costs the
/// operator a pointer, never a decision.
fn extract_head_branch(text: &str) -> Option<String> {
    let header = text.lines().next()?;
    let tail = header.rsplit_once(" (branch: ")?.1;
    let branch = tail.strip_suffix(')')?.trim();
    (!branch.is_empty()).then(|| branch.to_string())
}

/// `pr#{n}` — the `after_value` wire format both halves of mika#2242 write.
///
/// This is the column an operator `GROUP BY`s ("which umbrella untied what"), so
/// the producer and the reader must emit the same shape; the reader gets it by
/// re-emitting this row's value verbatim rather than recomputing it.
fn pr_reference(pr_url: &str) -> String {
    let tail = pr_url.rsplit('/').next().unwrap_or_default();
    let digits: String = tail.chars().take_while(char::is_ascii_digit).collect();
    if digits.is_empty() {
        format!("pr#{tail}")
    } else {
        format!("pr#{digits}")
    }
}

/// Find phantom tracking rows for `issue_url` (exact + `?phase=groom` variant),
/// transition each to `new_status` with `result`, and emit one
/// `tracking_row_upstream_closed` audit event per transitioned row. No-op safe:
/// zero rows → DEBUG log, no audit event, no WARN.
async fn cleanup_rows_for_issue_url(
    db: &AsyncDatabase,
    session_id: &str,
    trace_id: &str,
    issue_url: &str,
    event_type: &str,
    new_status: &str,
    result: &str,
) {
    let base_url = strip_groom_phase_suffix(issue_url);
    let rows = match db
        .find_active_tracking_rows_by_reference_url_and_variants(base_url)
        .await
    {
        Ok(rows) => rows,
        Err(e) => {
            warn!(
                event = "upstream_close_lookup_failed",
                issue_url = %issue_url,
                error = %e,
                "upstream_close: tracking-row lookup failed"
            );
            return;
        }
    };

    if rows.is_empty() {
        debug!(
            issue_url = %issue_url,
            event_type = %event_type,
            "upstream_close: no matching tracking rows (no-op)"
        );
        return;
    }

    for task in rows {
        match db
            .terminal_mark_tracking_row_upstream_closed(&task.id, new_status, result)
            .await
        {
            Ok(true) => {
                let reasoning = format!("event_type={event_type} reference_url={issue_url}");
                if let Err(e) = db
                    .log_audit_event(
                        session_id,
                        TRACKING_ROW_UPSTREAM_CLOSED_TOOL,
                        &format!("task:{}", task.id),
                        Some(&task.status),
                        Some(result),
                        Some(&reasoning),
                        Some(trace_id),
                    )
                    .await
                {
                    warn!(
                        event = "upstream_close_audit_failed",
                        task_id = %task.id,
                        error = %e,
                        "upstream_close: failed to write audit event (non-fatal)"
                    );
                }
                info!(
                    event = "tracking_row_upstream_closed",
                    task_id = %task.id,
                    issue_url = %issue_url,
                    event_type = %event_type,
                    new_status = %new_status,
                    result = %result,
                    "upstream_close: terminal-marked tracking row on upstream close"
                );
            }
            // Already terminal (idempotent re-delivery) or not phantom-shaped.
            Ok(false) => {}
            Err(e) => {
                warn!(
                    event = "upstream_close_transition_failed",
                    task_id = %task.id,
                    error = %e,
                    "upstream_close: guarded transition failed"
                );
            }
        }
    }
}

/// Extract the first GitHub issue URL from the webhook message text.
/// Scans lines for `https://github.com/<owner>/<repo>/issues/<number>`.
fn extract_issue_url(text: &str) -> Option<String> {
    extract_github_url_with_segment(text, "issues")
}

/// Extract the first GitHub PR URL from the webhook message text.
/// Scans lines for `https://github.com/<owner>/<repo>/pull/<number>`.
fn extract_pr_url(text: &str) -> Option<String> {
    extract_github_url_with_segment(text, "pull")
}

/// Shared scanner: first line shaped `https://github.com/<owner>/<repo>/<seg>/<n>`.
fn extract_github_url_with_segment(text: &str, seg: &str) -> Option<String> {
    for line in text.lines() {
        let trimmed = line.trim();
        let Some(suffix) = trimmed.strip_prefix("https://github.com/") else {
            continue;
        };
        let parts: Vec<&str> = suffix.splitn(4, '/').collect();
        if parts.len() >= 4 && parts[2] == seg && !parts[3].is_empty() {
            return Some(trimmed.to_string());
        }
    }
    None
}

/// Extract (owner, repo) from a GitHub URL of the form
/// `https://github.com/{owner}/{repo}/...`.
fn extract_owner_repo_from_url(url: &str) -> Option<(String, String)> {
    let path = url.strip_prefix("https://github.com/")?;
    let parts: Vec<&str> = path.splitn(3, '/').collect();
    if parts.len() < 2 || parts[0].is_empty() || parts[1].is_empty() {
        return None;
    }
    Some((parts[0].to_string(), parts[1].to_string()))
}

/// Parse the same-repo issue numbers a PR body says it closes
/// (`Closes #N` / `Fixes #N` / `Resolves #N`, any inflection, case-insensitive).
/// Deduplicated, order-preserving.
fn parse_closing_issue_refs(text: &str) -> Vec<u64> {
    let mut out: Vec<u64> = Vec::new();
    for cap in CLOSING_REF_RE.captures_iter(text) {
        if let Some(m) = cap.get(1)
            && let Ok(n) = m.as_str().parse::<u64>()
            && !out.contains(&n)
        {
            out.push(n);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_issue_url_valid() {
        let text = "[GitHub] Issue closed: senara-solutions/mika#1574 — title\nhttps://github.com/senara-solutions/mika/issues/1574";
        assert_eq!(
            extract_issue_url(text),
            Some("https://github.com/senara-solutions/mika/issues/1574".to_string())
        );
    }

    #[test]
    fn test_extract_issue_url_ignores_pr() {
        let text = "https://github.com/senara-solutions/mika/pull/1574";
        assert_eq!(extract_issue_url(text), None);
    }

    #[test]
    fn test_extract_pr_url_valid() {
        let text = "[GitHub] PR closed: senara-solutions/mika#1000 — t (branch: b)\nhttps://github.com/senara-solutions/mika/pull/1000\nMerged: true";
        assert_eq!(
            extract_pr_url(text),
            Some("https://github.com/senara-solutions/mika/pull/1000".to_string())
        );
    }

    #[test]
    fn test_extract_owner_repo_from_url() {
        assert_eq!(
            extract_owner_repo_from_url("https://github.com/senara-solutions/mika/pull/1000"),
            Some(("senara-solutions".to_string(), "mika".to_string()))
        );
        assert_eq!(extract_owner_repo_from_url("https://gitlab.com/a/b"), None);
    }

    #[test]
    fn test_parse_closing_issue_refs_variants() {
        let body = "This PR Closes #10, fixes #20 and Resolved #30. Also closes #10 again.";
        assert_eq!(parse_closing_issue_refs(body), vec![10, 20, 30]);
    }

    #[test]
    fn test_parse_closing_issue_refs_none() {
        let body = "No refs here, just #hashtag-ish text without a keyword.";
        assert_eq!(parse_closing_issue_refs(body), Vec::<u64>::new());
    }

    #[test]
    fn test_merged_sentinel() {
        let merged = "[GitHub] PR closed: a/b#1 — t (branch: x)\nurl\nMerged: true\n\nCloses #1";
        let unmerged = "[GitHub] PR closed: a/b#1 — t (branch: x)\nurl\nMerged: false\n\nCloses #1";
        assert!(merged.contains(MERGED_TRUE_LINE));
        assert!(!unmerged.contains(MERGED_TRUE_LINE));
    }

    // -- mika#2242 — les fonctions pures du producteur ----------------------

    #[test]
    fn mika2242_extract_head_branch_reads_the_gateway_header() {
        let text = "[GitHub] PR closed: senara-solutions/mika#2226 — fix: umbrella \
                    (branch: fix/umbrella-auto-pull-exclusion-observability)\n\
                    https://github.com/senara-solutions/mika/pull/2226\nMerged: false";
        assert_eq!(
            extract_head_branch(text).as_deref(),
            Some("fix/umbrella-auto-pull-exclusion-observability")
        );
    }

    /// Un titre portant lui-même la forme `(branch: …)` ne doit pas déplacer la
    /// vraie branche : c'est la **dernière** occurrence de la ligne d'en-tête qui
    /// compte, jamais la première.
    #[test]
    fn mika2242_a_title_carrying_the_shape_does_not_displace_the_branch() {
        let text = "[GitHub] PR closed: a/b#1 — doc: explain (branch: foo) syntax \
                    (branch: real/branch)\nurl\nMerged: false";
        assert_eq!(extract_head_branch(text).as_deref(), Some("real/branch"));
    }

    #[test]
    fn mika2242_extract_head_branch_is_none_without_the_shape() {
        assert_eq!(extract_head_branch(""), None);
        assert_eq!(
            extract_head_branch("[GitHub] PR closed: a/b#1 — t\nurl"),
            None
        );
        // Une branche vide n'est pas une branche : mieux vaut omettre la clé que
        // d'écrire `head_branch=` et laisser lire un nom littéralement vide.
        assert_eq!(
            extract_head_branch("[GitHub] PR closed: a/b#1 — t (branch: )"),
            None
        );
    }

    /// La troncature est **ancrée en queue**. Un corps qui parle du marqueur sans
    /// le porter en fin de texte n'est pas tronqué — le faux positif de prose que
    /// mika#2050 a mesuré sur le Signal S.
    #[test]
    fn mika2242_truncation_is_tail_anchored_not_a_substring() {
        let cut = "[GitHub] PR closed: a/b#1 — t (branch: x)\nurl\nMerged: false\n\nCorps…\n\n[truncated]";
        assert!(body_is_truncated(cut));
        // Tolère un saut de ligne final, que le transport peut ajouter.
        assert!(body_is_truncated(&format!("{cut}\n")));

        let prose = "[GitHub] PR closed: a/b#1 — t (branch: x)\nurl\nMerged: false\n\n\
                     Le gateway appose [truncated] quand il coupe un corps.";
        assert!(
            !body_is_truncated(prose),
            "un corps qui MENTIONNE le marqueur n'est pas un corps tronqué"
        );
        assert!(!body_is_truncated(""));
    }

    #[test]
    fn mika2242_pr_reference_is_the_wire_shape() {
        assert_eq!(
            pr_reference("https://github.com/senara-solutions/mika/pull/2226"),
            "pr#2226"
        );
        // Forme dégradée : on rend quelque chose de lisible plutôt que de perdre
        // la ligne — le `GROUP BY` reste honnête, il nomme ce qu'on a lu.
        assert_eq!(pr_reference("https://github.com/a/b/pull/abc"), "pr#abc");
    }

    #[test]
    fn mika2242_marker_key_is_exact_and_unambiguous() {
        assert_eq!(
            degroom_marker_key("senara-solutions/mika", 2131),
            "issue:senara-solutions/mika#2131"
        );
        // Le piège mika#2347 : la clé de `#234` ne doit jamais être un préfixe
        // de celle de `#2343` sous une comparaison exacte — et elles diffèrent.
        assert_ne!(
            degroom_marker_key("a/b", 234),
            degroom_marker_key("a/b", 2343)
        );
    }
}
