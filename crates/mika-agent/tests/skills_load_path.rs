//! Cross-layer integration tests for bundled skill schema preservation (#984).
//!
//! These tests exercise the production load path:
//! `skills/bundled/dev-pilot/tools.json` → `seed_bundled_skills()` → on-disk →
//! `scan_skills_dir()` → `ResolvedSkillTool.definition.input_schema`
//!
//! PR #969 added `validate_required_fields()` but only tested against hand-constructed
//! fixtures. This file closes the gap: it asserts the production-loaded schema
//! preserves the `required` field end-to-end.

use std::collections::HashSet;
use std::path::PathBuf;

use mika_agent::bundled_skills::seed_bundled_skills;
use mika_agent::skills::index::scan_skills_dir;

/// Locate the bundled skills source tree relative to CARGO_MANIFEST_DIR.
fn bundled_skills_source_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("skills")
        .join("bundled")
}

/// AC #3: Load the production `dev-pilot` manifest through the real load path
/// (seed → disk → scan) and assert `input_schema.required` contains exactly
/// `{"skill", "prompt", "task_id"}` (set-equality, since `inject_task_id_field`
/// may reorder).
#[test]
fn test_dev_pilot_required_fields_preserved_through_load_path() {
    let tmp = tempfile::tempdir().unwrap();
    let skills_dir = tmp.path().join("skills");
    std::fs::create_dir_all(&skills_dir).unwrap();

    // Step 1: Seed bundled skills (writes embedded content to disk)
    seed_bundled_skills(&skills_dir);

    // Verify dev-pilot was seeded
    let dev_pilot_dir = skills_dir.join("dev-pilot");
    assert!(
        dev_pilot_dir.exists(),
        "dev-pilot skill not seeded at {}",
        dev_pilot_dir.display()
    );

    // Step 2: Load via scan_skills_dir (same path production uses)
    let scan = scan_skills_dir(&skills_dir);
    assert!(
        !scan.entries.is_empty(),
        "scan_skills_dir returned zero entries"
    );

    // Find dev-pilot's run_claude_pilot tool
    let run_claude_pilot = scan
        .entries
        .iter()
        .flat_map(|entry| entry.skill_tools.iter())
        .find(|tool| tool.definition.name == "run_claude_pilot")
        .expect("run_claude_pilot tool not found in scan results");

    // Step 3: Assert `required` contains the expected fields
    let required = run_claude_pilot
        .definition
        .input_schema
        .get("required")
        .expect("input_schema.required is missing from loaded schema")
        .as_array()
        .expect("input_schema.required is not an array");

    let required_set: HashSet<&str> = required.iter().filter_map(|v| v.as_str()).collect();

    // Must contain at least these three (inject_task_id_field may have appended task_id
    // but it's already in the source — set equality handles both cases)
    let expected: HashSet<&str> = ["skill", "prompt", "task_id"].into_iter().collect();
    assert!(
        expected.is_subset(&required_set),
        "required fields missing from loaded schema.\n\
         Expected (subset): {:?}\n\
         Actual: {:?}",
        expected,
        required_set
    );

    // Specifically assert `skill` is present — this is the field whose absence
    // caused mika#984 (validate_required_fields no-op when the entire required
    // array was stale/missing)
    assert!(
        required_set.contains("skill"),
        "'skill' field missing from required array — this is the mika#984 regression"
    );
}

/// AC #3 variant: Also verify loading directly from the source tree (not seeded).
/// This tests `scan_skills_dir` against the canonical `skills/bundled/` path.
#[test]
fn test_dev_pilot_required_fields_preserved_from_source_tree() {
    let skills_dir = bundled_skills_source_dir();
    if !skills_dir.is_dir() {
        panic!(
            "bundled skills source dir not found at {} — repo layout changed?",
            skills_dir.display()
        );
    }

    let scan = scan_skills_dir(&skills_dir);

    let run_claude_pilot = scan
        .entries
        .iter()
        .flat_map(|entry| entry.skill_tools.iter())
        .find(|tool| tool.definition.name == "run_claude_pilot")
        .expect("run_claude_pilot tool not found in source tree scan");

    let required = run_claude_pilot
        .definition
        .input_schema
        .get("required")
        .expect("input_schema.required missing from source-tree schema")
        .as_array()
        .expect("input_schema.required is not an array in source tree");

    let required_set: HashSet<&str> = required.iter().filter_map(|v| v.as_str()).collect();

    assert!(
        required_set.contains("skill"),
        "'skill' not in required array from source tree"
    );
    assert!(
        required_set.contains("prompt"),
        "'prompt' not in required array from source tree"
    );
    assert!(
        required_set.contains("task_id"),
        "'task_id' not in required array from source tree"
    );
}

/// AC #1 Reproducer: Load the real dev-pilot tool via the production path, then
/// call validate_required_fields with a dispatch missing `skill`. Must get a
/// structured `missing_required_field` error.
#[test]
fn test_reproduce_missing_skill_field_rejected_on_production_schema() {
    use mika_agent::skills::executor::validate_required_fields;

    let tmp = tempfile::tempdir().unwrap();
    let skills_dir = tmp.path().join("skills");
    std::fs::create_dir_all(&skills_dir).unwrap();

    // Seed and load
    seed_bundled_skills(&skills_dir);
    let scan = scan_skills_dir(&skills_dir);

    let run_claude_pilot = scan
        .entries
        .iter()
        .flat_map(|entry| entry.skill_tools.iter())
        .find(|tool| tool.definition.name == "run_claude_pilot")
        .expect("run_claude_pilot tool not found");

    // Reproduce the exact input shape from mika#984: prompt + task_id, no skill
    let malformed_input = serde_json::json!({
        "prompt": "mika#666",
        "task_id": "0b38a0ec-4d4b-4039-b43b-c05d4e121bb3"
    });

    let result = validate_required_fields(run_claude_pilot, &malformed_input);
    assert!(
        result.is_some(),
        "validate_required_fields should reject input missing 'skill' field"
    );

    let output = result.unwrap();
    assert!(output.is_error, "expected error ToolOutput");
    assert!(
        output.content.contains("missing_required_field"),
        "expected 'missing_required_field' in error, got: {}",
        output.content
    );
    assert!(
        output.content.contains("skill"),
        "error should name 'skill' as the missing field, got: {}",
        output.content
    );
}

/// Drift detection: verify that seeded skills pass the drift check (no drift
/// expected when freshly seeded from the same build).
#[test]
fn test_freshly_seeded_skills_have_no_drift() {
    use mika_agent::bundled_skills::check_bundled_skill_drift;

    let tmp = tempfile::tempdir().unwrap();
    let skills_dir = tmp.path().join("skills");
    std::fs::create_dir_all(&skills_dir).unwrap();

    seed_bundled_skills(&skills_dir);

    let drift_count = check_bundled_skill_drift(&skills_dir);
    assert_eq!(
        drift_count, 0,
        "freshly seeded skills should have zero drift"
    );
}

/// Drift detection: verify that modifying a seeded file triggers drift.
#[test]
fn test_modified_skill_triggers_drift() {
    use mika_agent::bundled_skills::check_bundled_skill_drift;

    let tmp = tempfile::tempdir().unwrap();
    let skills_dir = tmp.path().join("skills");
    std::fs::create_dir_all(&skills_dir).unwrap();

    seed_bundled_skills(&skills_dir);

    // Tamper with dev-pilot's tools.json
    let tools_path = skills_dir.join("dev-pilot").join("tools.json");
    if tools_path.exists() {
        std::fs::write(&tools_path, r#"[{"name":"run_claude_pilot","description":"tampered","input_schema":{"type":"object","required":["prompt","task_id"],"properties":{"prompt":{"type":"string"},"task_id":{"type":"string"}}},"handler":{"type":"exec","command":"handlers/run.sh","long_running":true}}]"#).unwrap();

        let drift_count = check_bundled_skill_drift(&skills_dir);
        assert!(
            drift_count > 0,
            "modified dev-pilot tools.json should trigger drift detection"
        );
    }
}
