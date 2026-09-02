//! Cascade loop-résistance probe for mika-manager Phase 2 (mika#1947 — Porte 1, AC5).
//!
//! Walks the five-step cascade a Phase-2 mika-manager dispatch would produce —
//! `milestone_manager_report → hypothetical_dispatch → PR_open → CI_success →
//! callback` — and asserts that no step can enqueue or auto-merge work targeting
//! `crates/mika-agent/src/milestone_manager/**`. That is the loop the manager must
//! not be able to close around itself: dispatch authority is DECISION-CORE by
//! construction (Vincent's 2026-07-25 forge-gate bearing), so a manager that could
//! merge changes to its own surface would be able to widen its own authority.
//!
//! **Why not `EvalHarness`** (AC5 names it; F1, mika-arch first pass). `EvalHarness`
//! drives `run_agent()` against a seeded `MockLlmProvider` — it exists to make an LLM
//! turn deterministic. Phase 1 mika-manager has no LLM turn: `Reader`, `Assessor` and
//! `Reporter` are pure code, and the perimeter classifier is a pure function. Wrapping
//! them would add scaffolding and no assertion power (review-guide.md § KISS: match the
//! test tool to the system under test). If Phase 2 later gives the manager a
//! dispatch-verdict LLM turn, *that* test will need the harness. This one does not.
//!
//! **Gated** behind `#[ignore]` + `MIKA_MANAGER_LOOP_RESISTANCE_TEST=1` — this is a
//! pre-Phase-2-cut probe, not per-PR CI weight. Run it with:
//!
//! ```text
//! MIKA_MANAGER_LOOP_RESISTANCE_TEST=1 cargo test -p mika-agent --test eval -- \
//!   --ignored cascade_never_dispatches_into_milestone_manager
//! ```

use anyhow::Result;

use mika_agent::async_db::AsyncDatabase;
use mika_agent::db::Database;
use mika_agent::milestone_manager::{
    Assessor, AssessorConfig, MilestoneRef, Severity, compose_from_gh_outputs,
};
use mika_agent::perimeter::{Classification, classify_path, classify_pr_files};
use mika_agent::server::ci_success_handler::try_handle_ci_success;
use mika_agent::server::verdict_handler::{VerdictAction, try_handle_pr_review_verdict};
use mika_agent::skills::SkillRegistry;

const AGENT_ID: &str = "mika";
const SESSION_ID: &str = "manager-loop-resistance-session";
const GATE_ENV: &str = "MIKA_MANAGER_LOOP_RESISTANCE_TEST";

/// The file set a Phase-2 manager dispatch touching its own surface would produce.
const MANAGER_SURFACE_FILES: &[&str] = &[
    "crates/mika-agent/src/milestone_manager/reader.rs",
    "crates/mika-agent/src/milestone_manager/assessor.rs",
];

fn test_skills() -> SkillRegistry {
    let tmp = tempfile::tempdir().expect("tempdir");
    SkillRegistry::from_dir(tmp.path())
}

async fn test_db() -> AsyncDatabase {
    let db = Database::open_in_memory().expect("open in-memory db");
    db.create_session(SESSION_ID, AGENT_ID, "github")
        .expect("create session");
    AsyncDatabase::new(db)
}

#[tokio::test]
#[ignore = "pre-Phase-2-cut probe; set MIKA_MANAGER_LOOP_RESISTANCE_TEST=1 to run"]
async fn cascade_never_dispatches_into_milestone_manager() -> Result<()> {
    if std::env::var(GATE_ENV).as_deref() != Ok("1") {
        eprintln!("skipping: {GATE_ENV} is not set to 1");
        return Ok(());
    }

    // ── Step 1 — milestone_manager_report ────────────────────────────────────
    // A healthy milestone read produces an Assessment. This is the whole of what
    // Phase 1 can do, and the input a Phase 2 dispatch would be derived from.
    let milestone_json = r#"{"title": "Surface produit", "description": "agent et interfaces", "state": "open", "created_at": "2026-08-01T00:00:00Z", "due_on": null}"#;
    let issues_json = r#"[
        {"number": 1947, "title": "Porte 1", "state": "OPEN", "body": "> - **Plan:** docs/plans/p1.md\n> - **Branch:** `test/1947/x`", "labels": [{"name":"p1-important"}], "updatedAt": "2026-08-31T00:00:00Z"},
        {"number": 1948, "title": "Porte 2", "state": "CLOSED", "body": "> - **Plan:** docs/plans/p2.md", "labels": [{"name":"p1-important"}], "updatedAt": "2026-08-30T00:00:00Z"}
    ]"#;
    let pr_json = r#"[
        {"number": 2100, "state": "MERGED", "closingIssuesReferences": [{"number": 1948}], "statusCheckRollup": []}
    ]"#;
    let milestone_ref = MilestoneRef {
        repo: "senara-solutions/mika".into(),
        number: 35,
    };
    let state = compose_from_gh_outputs(&milestone_ref, milestone_json, issues_json, pr_json)?;
    let assessment = Assessor::new(AssessorConfig::default()).assess(&state);
    assert_ne!(
        assessment.severity,
        Severity::Blocked,
        "fixture must model a healthy milestone — a Blocked one would not reach the \
         dispatch path this cascade is probing"
    );

    // ── Step 2 — hypothetical_dispatch ───────────────────────────────────────
    // The primary loop-résistance invariant. A Phase-2 dispatch derived from the
    // report above, touching the manager's own surface, classifies DECISION-CORE.
    let files: Vec<String> = MANAGER_SURFACE_FILES
        .iter()
        .map(|s| s.to_string())
        .collect();
    let classification = classify_pr_files(&files);
    assert_eq!(
        classification.verdict,
        Classification::DecisionCore,
        "a manager-authored PR touching milestone_manager/** must never auto-merge"
    );
    assert_eq!(classification.decision_core_files, files);
    assert!(classification.mechanical_files.is_empty());

    // ── Step 3 — PR_open ─────────────────────────────────────────────────────
    // The verdict-handler merge-authority callsite holds instead of merging, even
    // with an APPROVED review carrying `VERDICT: pass` from mika-qa.
    let db = test_db().await;
    let review_text = "[GitHub] PR review (approved) on senara-solutions/mika#2101 \
                       (test: porte 1) by @mika-qa\n\
                       https://github.com/senara-solutions/mika/pull/2101#pullrequestreview-12345\n\
                       \n\
                       VERDICT: pass\n\nAll good.";
    let action = try_handle_pr_review_verdict(
        review_text,
        &db,
        Some("fake-token"),
        None,
        SESSION_ID,
        "trace-cascade-pr-open",
        &test_skills(),
    )
    .await;
    match action {
        VerdictAction::Handled { pre_digest } => assert!(
            pre_digest.contains("forge-gate") || pre_digest.contains("DECISION-CORE"),
            "PR_open must hold at the gate: {pre_digest}"
        ),
        other => panic!("PR_open step must hold for the operator, got {other:?}"),
    }

    // ── Step 4 — CI_success ──────────────────────────────────────────────────
    // The CI-success race path (mika#1851's founding breach) issues no merge.
    let ci_text = "[GitHub] Check suite success on senara-solutions/mika \
                   (branch: test/1947/perimeter-manager-forge-gate-loop-r)";
    let ci_action = try_handle_ci_success(
        ci_text,
        &db,
        Some("fake-token"),
        None,
        SESSION_ID,
        "trace-cascade-ci",
    )
    .await;
    assert!(
        !matches!(ci_action, VerdictAction::Dispatched { .. }),
        "CI_success must not dispatch a follow-up on a gated PR"
    );
    // `ci_success_merge` is the row written after `run_gh_merge` fires
    // (`after = "merge_initiated"`). The name is checked against the handler
    // source first: a count assertion on a name nothing emits is vacuous.
    crate::eval::test_ci_success_handler::assert_audit_event_name_is_real("ci_success_merge");
    assert_eq!(
        db.count_audit_events_by_tool_name("ci_success_merge")
            .await?,
        0,
        "no merge may be initiated through the CI-success path"
    );

    // ── Step 5 — callback ────────────────────────────────────────────────────
    // The cascade's last hop. Phase 1 has no dispatch class to enqueue from — the
    // structural proof of that lives in `milestone_manager/no_dispatch_test.rs`,
    // which is the canonical owner of the forbidden-token scan and runs on every
    // `cargo test -p mika-agent`. What this step adds is the receiving-side
    // guarantee: whatever a follow-up dispatch targeted under the manager's tree,
    // present file or future one, it lands on DECISION-CORE and cannot auto-merge.
    let module_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/milestone_manager");
    let mut checked = 0usize;
    for entry in std::fs::read_dir(&module_dir)? {
        let path = entry?.path();
        if path.extension().is_some_and(|e| e == "rs") {
            let repo_relative = format!(
                "crates/mika-agent/src/milestone_manager/{}",
                path.file_name().unwrap().to_str().unwrap()
            );
            assert_eq!(
                classify_path(&repo_relative),
                Classification::DecisionCore,
                "callback step: {repo_relative} must be DECISION-CORE"
            );
            checked += 1;
        }
    }
    assert!(
        checked >= 5,
        "expected the module to have its files enumerated, saw {checked}"
    );
    assert_eq!(
        classify_path("crates/mika-agent/src/milestone_manager/dispatcher.rs"),
        Classification::DecisionCore,
        "callback step: a future dispatch module must gate before it can ever merge"
    );

    Ok(())
}
