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

// ---------------------------------------------------------------------------
// claude-pilot (mika#1860) — precondition for cpp#79 autonomous onboarding.
// ---------------------------------------------------------------------------

#[test]
fn cpp_non_security_python_is_mechanical() {
    // Files enumerated in MECHANICAL_PREFIXES as cpp non-security python.
    // Each has documented rationale (see rules.rs comment).
    for path in [
        "src/claude_pilot/agent.py",
        "src/claude_pilot/cli.py",
        "src/claude_pilot/guardrails.py",
        "src/claude_pilot/inbox_writer.py",
        "src/claude_pilot/logger.py",
        "src/claude_pilot/transport.py",
        "src/claude_pilot/types.py",
        "src/claude_pilot/ui.py",
    ] {
        assert_eq!(
            classify_path(path),
            Classification::Mechanical,
            "cpp non-security python must be MECHANICAL: {path} (mika#1860)"
        );
    }
}

#[test]
fn cpp_ipython_subdir_is_mechanical() {
    // `src/claude_pilot/ipython/` prefix covers all optional IPython magics.
    for path in [
        "src/claude_pilot/ipython/__init__.py",
        "src/claude_pilot/ipython/magics.py",
    ] {
        assert_eq!(classify_path(path), Classification::Mechanical, "{path}");
    }
}

#[test]
fn cpp_tests_dir_is_mechanical() {
    // cpp tests live at repo root under `tests/` (mika tests live under
    // `crates/*/tests/` and are covered by MECHANICAL_CONTAINS).
    for path in [
        "tests/test_tier1.py",
        "tests/test_policy_devpilot.py",
        "tests/test_permissions.py",
        "tests/replay/replay_relay_decisions.py",
    ] {
        assert_eq!(classify_path(path), Classification::Mechanical, "{path}");
    }
}

#[test]
fn cpp_root_artifacts_are_mechanical() {
    // Python packaging + license — no security surface.
    for path in ["pyproject.toml", "uv.lock", "LICENSE"] {
        assert_eq!(classify_path(path), Classification::Mechanical, "{path}");
    }
}

#[test]
fn mika_root_dependency_manifests_are_mechanical() {
    // mika#1729: Dependabot-managed workspace-root manifests — Rust equivalents
    // of cpp's pyproject.toml / uv.lock. Exact-match (root-only) so the
    // autonomous Dependabot review chain clears the forge-gate for a root
    // dependency bump (AC4).
    for path in ["Cargo.toml", "Cargo.lock"] {
        assert_eq!(classify_path(path), Classification::Mechanical, "{path}");
    }
}

#[test]
fn mika_nested_cargo_manifests_stay_decision_core() {
    // mika#1729: exact-match is deliberately root-only. A member-crate manifest
    // (or any nested Cargo.toml/Cargo.lock) must fail-closed to DECISION-CORE —
    // a prefix/suffix rule would open an auto-merge bypass for decision-core PRs.
    for path in [
        "crates/mika-agent/Cargo.toml",
        "crates/mika-common/Cargo.toml",
        "crates/mika-agent/Cargo.lock",
    ] {
        assert_eq!(
            classify_path(path),
            Classification::DecisionCore,
            "nested manifest must fail-closed to DECISION-CORE: {path}"
        );
    }
}

#[test]
fn mika_root_cargo_bump_pr_classifies_mechanical() {
    // The common grouped-minor/patch Dependabot shape on a
    // `[workspace.dependencies]`-centric workspace: root manifest + lockfile
    // only. This is the AC4 happy path — auto-merge clears the forge-gate.
    let files = vec!["Cargo.toml".to_string(), "Cargo.lock".to_string()];
    let result = classify_pr_files(&files);
    assert_eq!(result.verdict, Classification::Mechanical);
    assert_eq!(result.mechanical_files.len(), 2);
    assert!(result.decision_core_files.is_empty());
}

#[test]
fn mika_dependabot_workflow_bump_stays_decision_core() {
    // github-actions ecosystem Dependabot PRs touch `.github/workflows/*.yml`,
    // which is NOT on the MECHANICAL allowlist (workflows carry secrets /
    // permissions surface). Deliberately operator-gated (mika#1729): a workflow
    // bump routes to the operator rather than auto-merging.
    let files = vec![".github/workflows/ci.yml".to_string()];
    let result = classify_pr_files(&files);
    assert_eq!(result.verdict, Classification::DecisionCore);
}

#[test]
fn cpp_docs_solutions_and_plans_are_mechanical() {
    // cpp shares `docs/solutions/` and `docs/plans/` with mika; already
    // covered by the shared MECHANICAL_PREFIXES. Assert cpp paths get the
    // right classification through the shared rules.
    for path in [
        "docs/solutions/security-issues/foo.md",
        "docs/plans/2026-07-28-001-cpp-plan.md",
    ] {
        assert_eq!(classify_path(path), Classification::Mechanical, "{path}");
    }
}

#[test]
fn cpp_classifier_tiers_are_decision_core() {
    // The 5 cpp files enumerated as DECISION-CORE in the rules.rs doc header.
    // These MUST fail-closed to DecisionCore — a false MECHANICAL here would
    // let the autonomous loop auto-merge a permission-policy widening,
    // re-opening the mika#1853 invariant hole on cpp (mika#1860 rationale).
    for path in [
        "src/claude_pilot/tier1.py",
        "src/claude_pilot/policy.py",
        "src/claude_pilot/permissions.py",
        "src/claude_pilot/per_spawn.py",
        "src/claude_pilot/policies/permissions.yaml",
    ] {
        assert_eq!(
            classify_path(path),
            Classification::DecisionCore,
            "cpp classifier tier must be DECISION-CORE: {path} (mika#1860)"
        );
    }
}

#[test]
fn cpp_future_policy_files_are_decision_core() {
    // Any new .yaml in src/claude_pilot/policies/ must fail-closed to
    // DECISION-CORE — the fail-closed default catches unenumerated policy
    // files (no `src/claude_pilot/policies/` prefix in the MECHANICAL
    // allowlist, so it correctly falls through).
    for path in [
        "src/claude_pilot/policies/new_policy.yaml",
        "src/claude_pilot/policies/overrides.yaml",
        "src/claude_pilot/policies/__init__.py",
    ] {
        assert_eq!(
            classify_path(path),
            Classification::DecisionCore,
            "unenumerated cpp policy file must fail-closed to DECISION-CORE: {path}"
        );
    }
}

#[test]
fn cpp_mixed_pr_taints_to_decision_core() {
    // Cross-repo integration invariant: a cpp PR touching ONE DECISION-CORE
    // file (tier1.py) alongside MECHANICAL files (logger.py, tests/) must
    // classify the whole PR as DECISION-CORE. Mirrors the mika mixed-PR
    // taint invariant for cpp-side onboarding readiness (cpp#79 blocked
    // by mika#1860).
    let files = vec![
        "src/claude_pilot/logger.py".to_string(),
        "tests/test_tier1.py".to_string(),
        "src/claude_pilot/tier1.py".to_string(), // DECISION-CORE taint
    ];
    let result = classify_pr_files(&files);
    assert_eq!(result.verdict, Classification::DecisionCore);
    assert_eq!(
        result.decision_core_files,
        vec!["src/claude_pilot/tier1.py"]
    );
    assert_eq!(result.mechanical_files.len(), 2);
}

#[test]
fn cpp_all_mechanical_pr_classifies_mechanical() {
    // Positive case: cpp PR touching only MECHANICAL cpp files auto-merges.
    // This is the throughput case cpp#79 enables — dep bumps, doc fixes,
    // test additions, non-security refactors flow through auto-merge.
    let files = vec![
        "src/claude_pilot/logger.py".to_string(),
        "src/claude_pilot/ui.py".to_string(),
        "tests/test_permissions.py".to_string(),
        "pyproject.toml".to_string(),
    ];
    let result = classify_pr_files(&files);
    assert_eq!(result.verdict, Classification::Mechanical);
    assert!(result.decision_core_files.is_empty());
    assert_eq!(result.mechanical_files.len(), 4);
}

// ---------------------------------------------------------------------------
// Root scaffolding config (mika#1864) — exact-match, root-only.
// ---------------------------------------------------------------------------

#[test]
fn root_gitignore_is_mechanical() {
    // Repo-root scaffolding config carries zero decision-core semantics.
    for path in [".gitignore", ".gitattributes", ".editorconfig"] {
        assert_eq!(
            classify_path(path),
            Classification::Mechanical,
            "root scaffolding {path} should be MECHANICAL (mika#1864)"
        );
    }
    // A PR touching only the root `.gitignore` auto-merges.
    let result = classify_pr_files(&[".gitignore".to_string()]);
    assert_eq!(result.verdict, Classification::Mechanical);
    assert!(result.decision_core_files.is_empty());
    assert_eq!(result.mechanical_files.len(), 1);
}

#[test]
fn nested_gitignore_still_decision_core() {
    // Exact-match is root-only: a nested `.gitignore` arrives as a repo-relative
    // path (never the bare `.gitignore`), so it fails closed to DECISION-CORE.
    // No prefix/suffix rule may accidentally allow it (that would open an
    // auto-merge bypass for decision-core PRs).
    for path in [
        "crates/foo/.gitignore",
        "crates/mika-agent/src/perimeter/.gitignore",
    ] {
        assert_eq!(
            classify_path(path),
            Classification::DecisionCore,
            "nested {path} must stay DECISION-CORE (mika#1864)"
        );
    }
}

#[test]
fn gitignore_plus_decision_core_taints() {
    // Mixed PR: root `.gitignore` (MECHANICAL) + a decision-core `.rs` still
    // taints the whole batch to DECISION-CORE — the taint invariant is
    // untouched by adding `.gitignore` to the allowlist.
    let files = vec![
        ".gitignore".to_string(),
        "crates/mika-agent/src/foo.rs".to_string(),
    ];
    let result = classify_pr_files(&files);
    assert_eq!(result.verdict, Classification::DecisionCore);
    assert_eq!(result.mechanical_files.len(), 1);
    assert_eq!(
        result.decision_core_files,
        vec!["crates/mika-agent/src/foo.rs"]
    );
}

// ---------------------------------------------------------------------------
// mika-manager Phase 2 dispatch surface (mika#1947 — Porte 1).
//
// `crates/mika-agent/src/milestone_manager/**` is DECISION-CORE by construction:
// Phase 2 promotion grants that module dispatch authority, and dispatch-authority
// sits on the DECISION-CORE side of Vincent's 2026-07-25 forge-gate bearing.
//
// Today the classification is correct only by the fail-closed default — no rule
// in `rules.rs` mentions the module, so `classify_path` falls through to
// DECISION-CORE. That is right, but it was untested: nothing stopped a future
// well-intentioned diff from adding a `milestone_manager` entry to a MECHANICAL
// table and silently opening an auto-merge path into the manager's own surface.
// The tests below make the invariant structural.
// ---------------------------------------------------------------------------

/// Absolute path to `crates/mika-agent/src/milestone_manager/`.
fn milestone_manager_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/milestone_manager")
}

/// Repo-relative paths of every `.rs` file currently under `src/milestone_manager/`.
///
/// Enumerated from the filesystem rather than hard-coded so a newly added module
/// file is covered the moment it lands — a hard-coded list would silently stop
/// protecting the surface it is meant to protect.
fn milestone_manager_source_paths() -> Vec<String> {
    let dir = milestone_manager_dir();
    let mut paths: Vec<String> = std::fs::read_dir(&dir)
        .expect("milestone_manager module directory must exist")
        .map(|entry| entry.expect("readable dir entry").path())
        .filter(|p| p.extension().is_some_and(|ext| ext == "rs"))
        .map(|p| {
            format!(
                "crates/mika-agent/src/milestone_manager/{}",
                p.file_name()
                    .expect("file name")
                    .to_str()
                    .expect("utf-8 file name")
            )
        })
        .collect();
    paths.sort();
    assert!(
        !paths.is_empty(),
        "expected at least one .rs file under {}",
        dir.display()
    );
    paths
}

#[test]
fn milestone_manager_files_are_decision_core() {
    // AC1 (mika#1947) — every current module file classifies DECISION-CORE.
    for path in milestone_manager_source_paths() {
        assert_eq!(
            classify_path(&path),
            Classification::DecisionCore,
            "mika-manager surface: {path} must be DECISION-CORE (mika#1947 Porte 1)"
        );
    }

    // Representative future paths — a file that does not exist yet must gate too,
    // otherwise Phase 2 could land new dispatch code on an unguarded path.
    for path in [
        "crates/mika-agent/src/milestone_manager/dispatcher.rs",
        "crates/mika-agent/src/milestone_manager/sub/bar.rs",
    ] {
        assert_eq!(
            classify_path(path),
            Classification::DecisionCore,
            "future mika-manager path {path} must be DECISION-CORE (mika#1947 Porte 1)"
        );
    }
}

#[test]
fn milestone_manager_solo_pr_is_decision_core() {
    // AC2 (mika#1947) — a single-file PR touching only the manager gates.
    let files = vec!["crates/mika-agent/src/milestone_manager/reader.rs".to_string()];
    let result = classify_pr_files(&files);
    assert_eq!(result.verdict, Classification::DecisionCore);
    assert_eq!(result.decision_core_files, files);
    assert!(result.mechanical_files.is_empty());
}

#[test]
fn milestone_manager_file_taints_pr_batch() {
    // AC2 (mika#1947) — the taint invariant holds for the manager surface: an
    // otherwise-MECHANICAL PR that also touches one manager file gates as a whole.
    // This is the shape a Phase-2 dispatch would most plausibly take (a docs +
    // tests + manager-code change), so the batch rule is the load-bearing one.
    let files = vec![
        "docs/logs/2026-08-22-note.md".to_string(),
        "crates/mika-agent/tests/eval/harness.rs".to_string(),
        "crates/mika-agent/src/milestone_manager/assessor.rs".to_string(),
    ];
    let result = classify_pr_files(&files);
    assert_eq!(result.verdict, Classification::DecisionCore);
    assert_eq!(
        result.decision_core_files,
        vec!["crates/mika-agent/src/milestone_manager/assessor.rs"]
    );
    assert_eq!(result.mechanical_files.len(), 2);
}

/// Source text of `rules.rs`, read at compile time.
///
/// `include_str!` (not `fs::read_to_string`) so the assertion is pinned to the
/// exact file the classifier was compiled from — a stale on-disk copy cannot
/// make these tests pass against different rules than the ones in force.
const RULES_SRC: &str = include_str!("rules.rs");

/// Extract the body of a `const <NAME>: &[&str] = &[ ... ];` slice literal.
fn mechanical_table_body(name: &str) -> &'static str {
    let needle = format!("const {name}: &[&str] = &[");
    let start = RULES_SRC
        .find(&needle)
        .unwrap_or_else(|| panic!("rules.rs must declare {name} — table renamed or removed?"))
        + needle.len();
    let rest = &RULES_SRC[start..];
    // Matches both the multi-line tables and the empty single-line
    // `= &[];` form (MECHANICAL_SUFFIXES today).
    let end = rest
        .find("];")
        .unwrap_or_else(|| panic!("unterminated slice literal for {name}"));
    &rest[..end]
}

#[test]
fn milestone_manager_prefix_not_in_mechanical_tables() {
    // AC1 (mika#1947), table-literal form — parses each MECHANICAL table body and
    // asserts none of them enumerates the manager surface. Adding
    // `crates/mika-agent/src/milestone_manager/` to any table fails here.
    for table in [
        "MECHANICAL_PREFIXES",
        "MECHANICAL_EXACT",
        "MECHANICAL_CONTAINS",
        "MECHANICAL_SUFFIXES",
    ] {
        let body = mechanical_table_body(table);
        assert!(
            !body.contains("milestone_manager"),
            "{table} must not enumerate the mika-manager surface — Phase 2 dispatch \
             authority is DECISION-CORE by construction (mika#1947 Porte 1)"
        );
    }
}

#[test]
fn milestone_manager_absent_from_all_mechanical_tables() {
    // AC1 (mika#1947), whole-file form — redundant-on-purpose with the table-literal
    // test above (F2, mika-arch first pass). The pair covers two distinct drift
    // paths: a restructure that renames the const tables would blind the parser
    // above, while an entry hidden outside a recognised table body would slip past
    // it. A whole-file scan catches both. Fail-loud is the point.
    assert!(
        !RULES_SRC.contains("milestone_manager"),
        "rules.rs must not mention `milestone_manager` anywhere — the manager surface \
         is DECISION-CORE via the fail-closed default and must stay unenumerated \
         (mika#1947 Porte 1).\n\
         \n\
         If a MECHANICAL table gained the entry: that is the regression this test \
         exists for. Remove it.\n\
         If you were only documenting the module in the `What is explicitly \
         DECISION-CORE` prose: that list is grep-anchored on paths absent from the \
         rules, so naming this one there is self-defeating — put the note in \
         `milestone_manager/mod.rs` instead, where the Porte 1 proof already lives. \
         Do not delete this assertion to make the prose fit."
    );
}

#[test]
fn milestone_manager_has_no_nested_tests_directory() {
    // AC1 (mika#1947), the hole this ticket surfaced. `MECHANICAL_CONTAINS` grants
    // MECHANICAL to any path containing `/tests/`, so a future
    // `src/milestone_manager/tests/foo.rs` would classify MECHANICAL and become
    // auto-mergeable — inside the very module Porte 1 exists to gate.
    //
    // Closing that by editing `rules.rs` is out of scope here (that file is
    // DECISION-CORE and the ticket explicitly excludes reworking the classifier).
    // Closing it structurally is not: the manager's own test code lives in
    // `no_dispatch_test.rs` beside the module files, and this test keeps it that
    // way. If a nested `tests/` directory is ever genuinely needed, the fix is to
    // narrow the `/tests/` rule first — never to delete this assertion.
    let nested = milestone_manager_dir().join("tests");
    assert!(
        !nested.exists(),
        "src/milestone_manager/tests/ must not exist: MECHANICAL_CONTAINS `/tests/` \
         would classify everything under it as auto-mergeable, opening a merge path \
         into the mika-manager surface (mika#1947 Porte 1)"
    );
    // Guard the rule this test depends on: if `/tests/` ever leaves
    // MECHANICAL_CONTAINS, this test is obsolete and should be revisited.
    assert!(
        mechanical_table_body("MECHANICAL_CONTAINS").contains("\"/tests/\""),
        "MECHANICAL_CONTAINS no longer carries `/tests/` — re-evaluate whether the \
         nested-directory guard above is still the right shape (mika#1947)"
    );
}
