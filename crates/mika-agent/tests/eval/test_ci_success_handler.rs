//! Forge-gate coverage for the CI-success merge path (mika#1947 — Porte 1).
//!
//! `ci_success_handler` is the second merge-authority callsite. It is the one that
//! actually breached: mika#1851 auto-merged four DECISION-CORE files through this
//! path on 2026-07-27 because the perimeter classifier was never consulted here.
//! mika#1853 wired the classifier in. This file asserts the wiring holds for the
//! mika-manager surface specifically, which Phase 2 would make reachable.
//!
//! **Why the shape differs from `test_verdict_handler.rs`.** The verdict handler
//! consults the perimeter before any `gh` call, so an eval-environment test reaches
//! its DECISION-CORE branch through the fail-closed clause. This handler resolves the
//! open PR first (`find_open_pr`, step 2) and returns `Passthrough` when `gh` cannot
//! run — which it cannot here — so the perimeter block at step 5c is unreachable from
//! a test that does not fake the `gh` subprocess. Making it reachable means injecting
//! a seam into `perimeter::fetch`, which mika#1947 lists as out of scope.
//!
//! So the behavioural assertion covers what is reachable (the handler does not merge),
//! and the invariant that actually matters — the classifier is consulted *before*
//! any merge can be issued — is asserted structurally against the source. That is the
//! same shape as the ordering bug mika#1851 was: not a wrong verdict, a merge that
//! ran before the verdict existed.

use anyhow::Result;

use mika_agent::async_db::AsyncDatabase;
use mika_agent::db::Database;
use mika_agent::perimeter::{Classification, classify_pr_files};
use mika_agent::server::ci_success_handler::try_handle_ci_success;
use mika_agent::server::verdict_handler::VerdictAction;

const AGENT_ID: &str = "mika";
const SESSION_ID: &str = "ci-success-porte1-session";

async fn test_db() -> AsyncDatabase {
    let db = Database::open_in_memory().expect("open in-memory db");
    db.create_session(SESSION_ID, AGENT_ID, "github")
        .expect("create session");
    AsyncDatabase::new(db)
}

/// Source of the handler under test, pinned at compile time so the structural
/// assertions below cannot pass against a stale copy on disk.
const CI_SUCCESS_HANDLER_SRC: &str = include_str!("../../src/server/ci_success_handler.rs");

/// Body of `try_handle_ci_success`, from its signature to the first column-0 `}`.
fn try_handle_ci_success_body() -> &'static str {
    let start = CI_SUCCESS_HANDLER_SRC
        .find("pub async fn try_handle_ci_success(")
        .expect("try_handle_ci_success must exist — renamed or removed?");
    let rest = &CI_SUCCESS_HANDLER_SRC[start..];
    let end = rest
        .find("\n}\n")
        .expect("unterminated try_handle_ci_success body");
    &rest[..end]
}

#[tokio::test]
async fn ci_success_milestone_manager_pr_holds_for_operator() -> Result<()> {
    // Layer A — the classifier verdict on the mika-manager surface. Shared with
    // verdict_handler: one classifier, both callsites.
    let files = vec![
        "crates/mika-agent/src/milestone_manager/reader.rs".to_string(),
        "crates/mika-agent/src/milestone_manager/assessor.rs".to_string(),
    ];
    let classification = classify_pr_files(&files);
    assert_eq!(
        classification.verdict,
        Classification::DecisionCore,
        "mika-manager surface must classify DECISION-CORE on the CI-success path too"
    );
    assert_eq!(classification.decision_core_files, files);

    // Layer B — the handler does not merge. With `gh` unresolvable, `find_open_pr`
    // bails before step 5c, so this asserts the fail-safe direction of that bail:
    // no PR resolved means no merge issued, never an optimistic merge.
    let db = test_db().await;
    let text = "[GitHub] Check suite success on senara-solutions/mika \
                (branch: test/1947/perimeter-manager-forge-gate-loop-r)";

    let action = try_handle_ci_success(
        text,
        &db,
        Some("fake-token"),
        None,
        SESSION_ID,
        "trace-porte1-ci",
    )
    .await;

    match action {
        VerdictAction::Passthrough { .. } => {}
        VerdictAction::Handled { pre_digest } => {
            // Reachable only if a future change lets the handler get past
            // `find_open_pr` here. If it does, the hold branch is the only
            // acceptable outcome for this file set.
            assert!(
                pre_digest.contains("forge-gate") || pre_digest.contains("DECISION-CORE"),
                "if the handler acts on a milestone_manager PR it must hold, not merge: {pre_digest}"
            );
        }
        other => panic!("CI success on an unresolvable PR must not dispatch: {other:?}"),
    }
    assert_eq!(
        db.count_audit_events_by_tool_name("ci_success_handler_merge_initiated")
            .await?,
        0,
        "no merge may be initiated for a mika-manager PR"
    );

    // Layer C — the ordering invariant, asserted against the source. This is the
    // shape of the mika#1851 breach: the merge call ran and the classifier never did.
    let body = try_handle_ci_success_body();
    let classify_at = body
        .find("perimeter::classify_pr_files")
        .expect("try_handle_ci_success must consult the perimeter classifier (mika#1853)");
    let fail_closed_at = body
        .find("verdict: Classification::DecisionCore")
        .expect("the perimeter fetch error must fail closed to DECISION-CORE (mika#1853)");
    let gate_event_at = body
        .find("\"ci_success_handler_human_gate_required\"")
        .expect("the DECISION-CORE branch must write a greppable audit row");
    let merge_at = body
        .find("run_gh_merge(")
        .expect("try_handle_ci_success must still contain the merge call");

    assert!(
        classify_at < merge_at,
        "the perimeter classifier must be consulted BEFORE any merge is issued — \
         a merge that runs first is mika#1851 verbatim"
    );
    assert!(
        fail_closed_at < merge_at,
        "the fail-closed clause must be evaluated before the merge call"
    );
    assert!(
        gate_event_at < merge_at,
        "the DECISION-CORE hold (and its audit row) must precede the merge branch"
    );

    Ok(())
}
