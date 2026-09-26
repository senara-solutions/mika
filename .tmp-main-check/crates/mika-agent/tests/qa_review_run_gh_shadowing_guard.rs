//! Hermetic regression guard for mika#1168 Phase B variant b2 (Step 9).
//!
//! Asserts that the bundled `qa-review` skill does NOT register a per-skill
//! `run_gh` exec handler — its registration shadowed the global builtin and
//! broke the ready-label dispatch-ack path (`run_gh issue edit
//! --remove-label ready` hit qa-review's allowlist and failed).
//!
//! The hermetic shape is: read `qa-review/tools.json` from the source tree
//! and assert the JSON does not declare a tool named `run_gh`. Companion
//! invariant: the handler script `qa-review/handlers/run_gh.sh` must be
//! absent (b2 deletes it). The two checks together close the regression
//! class — re-adding the registration without the script (or vice versa)
//! would fail one and warn the operator before the prod dispatch breaks
//! again.
//!
//! ## Boundary
//!
//! This test does NOT exercise the full skill executor's handler-resolution
//! order — that pathway is tested elsewhere (the agent loop integration
//! tests under `tests/eval/`). It tests the *source-of-truth* artifact
//! (`tools.json`) directly so regressions are caught at the manifest layer
//! before they reach runtime.
//!
//! Reference: mika#1168 Phase B Step 9.

use std::path::PathBuf;

use serde_json::Value;

/// Resolve the workspace root from the test binary's CARGO_MANIFEST_DIR,
/// which is the crate dir (`crates/mika-agent/`). Walk up two levels.
fn workspace_root() -> PathBuf {
    let crate_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    crate_dir.parent().unwrap().parent().unwrap().to_path_buf()
}

#[test]
fn qa_review_tools_json_does_not_register_run_gh() {
    let path = workspace_root().join("skills/bundled/qa-review/tools.json");
    let raw = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("could not read {}: {e}", path.display()));
    let parsed: Value = serde_json::from_str(&raw)
        .unwrap_or_else(|e| panic!("{} is not valid JSON: {e}", path.display()));

    let tools = parsed
        .as_array()
        .unwrap_or_else(|| panic!("{} root is not a JSON array", path.display()));

    let names: Vec<&str> = tools
        .iter()
        .filter_map(|t| t.get("name").and_then(|v| v.as_str()))
        .collect();

    assert!(
        !names.contains(&"run_gh"),
        "qa-review/tools.json registers a `run_gh` per-skill exec handler — \
         this re-introduces the global-builtin shadowing class fixed in \
         mika#1168 (variant b2). Tools currently declared: {:?}. \
         If qa-review legitimately needs a narrower allowlist again, add \
         the restriction at the skill system prompt level (or via per-tool \
         arg validators) — not by shadowing the global builtin, which \
         breaks every other always-on or dispatch-ack handler that calls \
         the same tool name.",
        names
    );
}

#[test]
fn qa_review_run_gh_handler_script_is_absent() {
    let path = workspace_root().join("skills/bundled/qa-review/handlers/run_gh.sh");
    assert!(
        !path.exists(),
        "qa-review/handlers/run_gh.sh was re-introduced — even without the \
         tools.json registration this script is a footgun (anyone re-adding \
         the registration finds a working handler waiting). Remove it. \
         mika#1168 Phase B b2 deleted both."
    );
}
