//! Perimeter classifier tests.
//!
//! Two invariants get named coverage:
//!
//! 1. **Self-reference (Prime NN-a).** The perimeter module and its rules
//!    file are themselves DECISION-CORE — editing them auto-gates. If a
//!    future refactor moves them under a MECHANICAL prefix, these tests
//!    fail loudly.
//! 2. **Fail-closed default (Prime FAIL-CLOSED).** Any path not matching
//!    the allowlist defaults to DECISION-CORE. Unknown paths, empty diffs,
//!    and single-DECISION-CORE mixes all gate.

use super::{Classification, classify_path, classify_pr_files};

// ---------------------------------------------------------------------------
// Self-reference (non-negotiable a) — perimeter defines itself.
// ---------------------------------------------------------------------------

#[test]
fn perimeter_module_files_are_decision_core() {
    for path in [
        "crates/mika-agent/src/perimeter/mod.rs",
        "crates/mika-agent/src/perimeter/rules.rs",
        "crates/mika-agent/src/perimeter/fetch.rs",
        "crates/mika-agent/src/perimeter/tests.rs",
    ] {
        assert_eq!(
            classify_path(path),
            Classification::DecisionCore,
            "perimeter self-reference invariant: {path} must be DECISION-CORE (Prime NN-a)"
        );
    }
}

#[test]
fn perimeter_doc_is_decision_core() {
    assert_eq!(
        classify_path("docs/gate/perimeter.md"),
        Classification::DecisionCore,
        "the perimeter doc must be gated as authority (not narrative docs)"
    );
}

// ---------------------------------------------------------------------------
// Explicitly-named DECISION-CORE zones (grep-anchored).
// ---------------------------------------------------------------------------

#[test]
fn verdict_parser_is_decision_core() {
    assert_eq!(
        classify_path("crates/mika-agent/src/server/verdict.rs"),
        Classification::DecisionCore
    );
    assert_eq!(
        classify_path("crates/mika-agent/src/server/verdict_handler.rs"),
        Classification::DecisionCore
    );
}

#[test]
fn pr_merge_gate_is_decision_core() {
    assert_eq!(
        classify_path("crates/mika-agent/src/tools/pr_merge_with_gate.rs"),
        Classification::DecisionCore
    );
}

#[test]
fn dispatch_authority_is_decision_core() {
    assert_eq!(
        classify_path("crates/mika-agent/src/skills/executor.rs"),
        Classification::DecisionCore
    );
}

#[test]
fn permission_policy_skill_is_decision_core() {
    assert_eq!(
        classify_path("skills/bundled/permission-policy/system_prompt.md"),
        Classification::DecisionCore
    );
    assert_eq!(
        classify_path("skills/bundled/permission-policy/skill.toml"),
        Classification::DecisionCore
    );
}

#[test]
fn dispatch_lib_is_decision_core() {
    // dispatch-lib mixes retry (mechanical) and authority (dispatch-lib
    // decides who runs what). Impossible to safely classify per-diff-hunk
    // without diff-content analysis — whole file gates.
    assert_eq!(
        classify_path("skills/bundled/_shared/dispatch-lib.sh"),
        Classification::DecisionCore
    );
}

#[test]
fn qa_review_skill_is_decision_core() {
    // The qa-review prompt defines the verdict format contract.
    // Changing it is verdict-form authority.
    assert_eq!(
        classify_path("skills/bundled/qa-review/system_prompt.md"),
        Classification::DecisionCore
    );
}

#[test]
fn labels_taxonomy_is_decision_core() {
    assert_eq!(
        classify_path(".github/labels.yml"),
        Classification::DecisionCore
    );
}

#[test]
fn authority_docs_are_decision_core() {
    for path in [
        "docs/architecture/review-guide.md",
        "docs/adr/0042-forge-gate.md",
        "docs/design/luminescent-core.md",
    ] {
        assert_eq!(
            classify_path(path),
            Classification::DecisionCore,
            "authority doc {path} must be gated"
        );
    }
}

// ---------------------------------------------------------------------------
// Positive-case MECHANICAL matches (the allowlist works as advertised).
// ---------------------------------------------------------------------------

#[test]
fn narrative_docs_are_mechanical() {
    for path in [
        "docs/logs/2026-07-25-handsoff.md",
        "docs/plans/2026-07-25-001-forge-gate.md",
        "docs/solutions/best-practices/pattern-2026-07-25.md",
        "docs/eval/calibration/baselines/mika-qa.json",
    ] {
        assert_eq!(
            classify_path(path),
            Classification::Mechanical,
            "narrative doc {path} should be MECHANICAL"
        );
    }
}

#[test]
fn integration_tests_are_mechanical() {
    for path in [
        "crates/mika-agent/tests/eval/golden/foo.rs",
        "crates/mika-common/tests/some_test.rs",
        "crates/mika-gateway/tests/admin_customers.rs",
    ] {
        assert_eq!(
            classify_path(path),
            Classification::Mechanical,
            "integration test {path} should be MECHANICAL"
        );
    }
}

#[test]
fn inline_test_modules_are_mechanical() {
    // Inline test modules anywhere under a crate — the substring rule.
    assert_eq!(
        classify_path("crates/mika-common/src/foo/tests/mod.rs"),
        Classification::Mechanical
    );
    assert_eq!(
        classify_path("crates/mika-agent/src/some_module/tests.rs"),
        // Note: `tests.rs` alone is NOT a `/tests/` substring — the
        // substring is `/tests/`. This case falls through to
        // DECISION-CORE. That is intentional: an inline `mod tests`
        // sibling test file can only run under `#[cfg(test)]`, but the
        // file itself lives next to production code and might be
        // accidentally cfg-flipped. Fail-closed here.
        Classification::DecisionCore
    );
}

#[test]
fn telemetry_plumbing_is_mechanical() {
    assert_eq!(
        classify_path("crates/mika-agent/src/telemetry/mod.rs"),
        Classification::Mechanical
    );
    assert_eq!(
        classify_path("crates/mika-agent/src/telemetry/otel.rs"),
        Classification::Mechanical
    );
}

#[test]
fn dashboard_and_ui_are_mechanical() {
    for path in [
        "dashboard/src/App.tsx",
        "dashboard/src/components/TokenBudgetBar.tsx",
        "packages/ui/src/StatusBadge.tsx",
        "docs-site/config.yaml",
    ] {
        assert_eq!(
            classify_path(path),
            Classification::Mechanical,
            "frontend/dashboard path {path} should be MECHANICAL"
        );
    }
}

#[test]
fn release_tooling_is_mechanical() {
    for path in [
        "CHANGELOG.md",
        "cliff.toml",
        "release-please-manifest.json",
        "README.md",
    ] {
        assert_eq!(
            classify_path(path),
            Classification::Mechanical,
            "release/packaging file {path} should be MECHANICAL"
        );
    }
}

// ---------------------------------------------------------------------------
// PR-level classification (fail-closed on empty; taint on any DECISION-CORE).
// ---------------------------------------------------------------------------

#[test]
fn empty_file_list_fails_closed() {
    let result = classify_pr_files(&[]);
    assert_eq!(
        result.verdict,
        Classification::DecisionCore,
        "empty diff must fail-closed to DECISION-CORE (we cannot verify what a zero-file diff does)"
    );
    assert!(result.mechanical_files.is_empty());
    assert!(result.decision_core_files.is_empty());
}

#[test]
fn all_mechanical_files_yield_mechanical_verdict() {
    let files = vec![
        "docs/logs/handsoff.md".to_string(),
        "docs/solutions/foo.md".to_string(),
        "dashboard/src/App.tsx".to_string(),
    ];
    let result = classify_pr_files(&files);
    assert_eq!(result.verdict, Classification::Mechanical);
    assert_eq!(result.mechanical_files.len(), 3);
    assert!(result.decision_core_files.is_empty());
}

#[test]
fn one_decision_core_file_taints_the_batch() {
    // Even if 99 files are MECHANICAL, ONE decision-core file gates.
    let files = vec![
        "docs/logs/a.md".to_string(),
        "docs/solutions/b.md".to_string(),
        "crates/mika-agent/src/server/verdict_handler.rs".to_string(),
        "dashboard/c.tsx".to_string(),
    ];
    let result = classify_pr_files(&files);
    assert_eq!(result.verdict, Classification::DecisionCore);
    assert_eq!(result.mechanical_files.len(), 3);
    assert_eq!(result.decision_core_files.len(), 1);
    assert!(
        result
            .decision_core_files
            .contains(&"crates/mika-agent/src/server/verdict_handler.rs".to_string())
    );
}

#[test]
fn unknown_path_fails_closed() {
    // A path nowhere in the allowlist fails closed to DECISION-CORE.
    let files = vec!["some/random/production/code.rs".to_string()];
    let result = classify_pr_files(&files);
    assert_eq!(result.verdict, Classification::DecisionCore);
    assert_eq!(result.decision_core_files, files);
}

// ---------------------------------------------------------------------------
// Summary formatting (log lines / notifications).
// ---------------------------------------------------------------------------

#[test]
fn summary_names_decision_core_files() {
    let files = vec![
        "docs/logs/a.md".to_string(),
        "crates/mika-agent/src/server/verdict.rs".to_string(),
    ];
    let result = classify_pr_files(&files);
    let summary = result.summary();
    assert!(summary.contains("DECISION-CORE"));
    assert!(summary.contains("crates/mika-agent/src/server/verdict.rs"));
}

#[test]
fn summary_caps_named_files_at_five() {
    // 7 decision-core files: named list caps at 5, "+2 more" tail.
    let files = (1..=7)
        .map(|i| format!("crates/decision-core-{i}.rs"))
        .collect::<Vec<_>>();
    let result = classify_pr_files(&files);
    let summary = result.summary();
    assert!(summary.contains("+2 more"), "summary = {summary:?}");
}

#[test]
fn summary_reports_mechanical_count() {
    let files = vec!["docs/logs/a.md".to_string(), "docs/plans/b.md".to_string()];
    let result = classify_pr_files(&files);
    let summary = result.summary();
    assert!(summary.contains("MECHANICAL"));
    assert!(summary.contains('2'));
}
