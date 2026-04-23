//! eval-diff — Calibration baseline management CLI (#742).
//!
//! Subcommands:
//! - `bootstrap <artifact>` — Copy a calibration artifact as the committed baseline.
//! - `diff <baseline> <new>` — Semantic diff between two calibration artifacts.
//!   Exit code 0 = no drift, 1 = drift detected, 2 = error.
//! - `format-pr-body <baseline> <new>` — Output a Markdown PR body for a drift PR.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use serde::{Deserialize, Serialize};

// ── Calibration types (mirrored from tests/eval/calibration.rs) ─────────────

/// Top-level calibration artifact, written as JSON.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct CalibrationArtifact {
    version: u32,
    timestamp: String,
    providers: BTreeMap<String, ProviderCalibration>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ProviderCalibration {
    model: String,
    scenarios: BTreeMap<String, ScenarioCalibration>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ScenarioCalibration {
    outcome: String,
    error_class: Option<String>,
    input_tokens: Option<u64>,
    output_tokens: Option<u64>,
    latency_ms: u64,
}

/// A single change detected between two calibration artifacts.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct CalibrationDiff {
    provider: String,
    scenario: String,
    old_outcome: String,
    new_outcome: String,
    change_type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DiffResult {
    changes: Vec<CalibrationDiff>,
    token_summary: BTreeMap<String, TokenSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct TokenSummary {
    total_input_tokens: u64,
    total_output_tokens: u64,
    provider_count: u32,
}

// ── PR body types ───────────────────────────────────────────────────────────

/// JSON structure embedded in drift PR body (D5 format).
#[derive(Debug, Serialize)]
struct PrDiffSummary {
    previous_baseline_captured_at: String,
    new_calibration_captured_at: String,
    changes: Vec<PrDiffChange>,
}

#[derive(Debug, Serialize)]
struct PrDiffChange {
    provider: String,
    model: String,
    scenario: String,
    previous_outcome: String,
    new_outcome: String,
    classification: String,
}

// ── Diff logic (mirrored from tests/eval/calibration.rs) ────────────────────

fn diff_calibrations(old: &CalibrationArtifact, new: &CalibrationArtifact) -> DiffResult {
    let mut changes = Vec::new();
    let mut token_summary: BTreeMap<String, TokenSummary> = BTreeMap::new();

    for (provider, new_cal) in &new.providers {
        for (scenario, new_scenario) in &new_cal.scenarios {
            let summary = token_summary
                .entry(scenario.clone())
                .or_insert(TokenSummary {
                    total_input_tokens: 0,
                    total_output_tokens: 0,
                    provider_count: 0,
                });
            summary.total_input_tokens += new_scenario.input_tokens.unwrap_or(0);
            summary.total_output_tokens += new_scenario.output_tokens.unwrap_or(0);
            summary.provider_count += 1;

            let old_outcome = old
                .providers
                .get(provider)
                .and_then(|p| p.scenarios.get(scenario))
                .map(|s| s.outcome.as_str());

            match old_outcome {
                Some(old_out) if old_out != new_scenario.outcome => {
                    changes.push(CalibrationDiff {
                        provider: provider.clone(),
                        scenario: scenario.clone(),
                        old_outcome: old_out.to_string(),
                        new_outcome: new_scenario.outcome.clone(),
                        change_type: "tolerance_change".to_string(),
                    });
                }
                None => {
                    let change_type = if old.providers.contains_key(provider) {
                        "new_scenario"
                    } else {
                        "new_provider"
                    };
                    changes.push(CalibrationDiff {
                        provider: provider.clone(),
                        scenario: scenario.clone(),
                        old_outcome: String::new(),
                        new_outcome: new_scenario.outcome.clone(),
                        change_type: change_type.to_string(),
                    });
                }
                _ => {}
            }
        }
    }

    for (provider, old_cal) in &old.providers {
        for (scenario, old_scenario) in &old_cal.scenarios {
            let still_present = new
                .providers
                .get(provider)
                .and_then(|p| p.scenarios.get(scenario))
                .is_some();
            if !still_present {
                changes.push(CalibrationDiff {
                    provider: provider.clone(),
                    scenario: scenario.clone(),
                    old_outcome: old_scenario.outcome.clone(),
                    new_outcome: String::new(),
                    change_type: "removed".to_string(),
                });
            }
        }
    }

    DiffResult {
        changes,
        token_summary,
    }
}

// ── Helpers ─────────────────────────────────────────────────────────────────

fn load_artifact(path: &Path) -> Result<CalibrationArtifact, String> {
    let content = std::fs::read_to_string(path)
        .map_err(|e| format!("Failed to read {}: {e}", path.display()))?;
    serde_json::from_str(&content).map_err(|e| format!("Failed to parse {}: {e}", path.display()))
}

/// Classify a change type into PR-body classification vocabulary (D4/D5).
fn classify_change(change: &CalibrationDiff) -> &'static str {
    match change.change_type.as_str() {
        "tolerance_change" => "outcome_change",
        "new_provider" => "new_provider",
        "new_scenario" => "new_scenario",
        "removed" => "removed",
        _ => "unknown",
    }
}

/// Count outcome changes (tolerance_change type only) for PR title.
fn count_outcome_changes(diff: &DiffResult) -> usize {
    diff.changes
        .iter()
        .filter(|c| c.change_type == "tolerance_change")
        .count()
}

/// Build a PR body diff summary from two artifacts and their diff.
fn build_pr_summary(
    old: &CalibrationArtifact,
    new: &CalibrationArtifact,
    diff: &DiffResult,
) -> PrDiffSummary {
    let changes = diff
        .changes
        .iter()
        .map(|c| {
            let model = new
                .providers
                .get(&c.provider)
                .or_else(|| old.providers.get(&c.provider))
                .map(|p| p.model.clone())
                .unwrap_or_default();
            PrDiffChange {
                provider: c.provider.clone(),
                model,
                scenario: c.scenario.clone(),
                previous_outcome: c.old_outcome.clone(),
                new_outcome: c.new_outcome.clone(),
                classification: classify_change(c).to_string(),
            }
        })
        .collect();

    PrDiffSummary {
        previous_baseline_captured_at: old.timestamp.clone(),
        new_calibration_captured_at: new.timestamp.clone(),
        changes,
    }
}

/// Count distinct providers that have changes.
fn count_affected_providers(diff: &DiffResult) -> usize {
    let providers: std::collections::HashSet<&str> =
        diff.changes.iter().map(|c| c.provider.as_str()).collect();
    providers.len()
}

// ── Subcommands ─────────────────────────────────────────────────────────────

fn cmd_bootstrap(artifact_path: &Path, baseline_path: &Path) -> ExitCode {
    // Validate artifact exists and is valid JSON
    if let Err(e) = load_artifact(artifact_path) {
        eprintln!("Error: {e}");
        return ExitCode::from(2);
    }

    // Create parent directories
    if let Some(parent) = baseline_path.parent()
        && let Err(e) = std::fs::create_dir_all(parent)
    {
        eprintln!(
            "Error: failed to create directory {}: {e}",
            parent.display()
        );
        return ExitCode::from(2);
    }

    // Copy artifact to baseline
    if let Err(e) = std::fs::copy(artifact_path, baseline_path) {
        eprintln!(
            "Error: failed to copy {} to {}: {e}",
            artifact_path.display(),
            baseline_path.display()
        );
        return ExitCode::from(2);
    }

    println!(
        "Baseline bootstrapped: {} -> {}",
        artifact_path.display(),
        baseline_path.display()
    );
    ExitCode::SUCCESS
}

fn cmd_diff(baseline_path: &Path, new_path: &Path) -> ExitCode {
    let old = match load_artifact(baseline_path) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("Error: {e}");
            return ExitCode::from(2);
        }
    };
    let new = match load_artifact(new_path) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("Error: {e}");
            return ExitCode::from(2);
        }
    };

    let result = diff_calibrations(&old, &new);

    if result.changes.is_empty() {
        println!("No drift detected.");
        ExitCode::SUCCESS
    } else {
        println!(
            "Drift detected: {} changes across {} providers.",
            result.changes.len(),
            count_affected_providers(&result)
        );
        let json = serde_json::to_string_pretty(&result).unwrap_or_default();
        println!("{json}");
        ExitCode::from(1)
    }
}

fn cmd_format_pr_body(baseline_path: &Path, new_path: &Path) -> ExitCode {
    let old = match load_artifact(baseline_path) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("Error: {e}");
            return ExitCode::from(2);
        }
    };
    let new = match load_artifact(new_path) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("Error: {e}");
            return ExitCode::from(2);
        }
    };

    let diff = diff_calibrations(&old, &new);
    if diff.changes.is_empty() {
        println!("No drift detected — no PR body to generate.");
        return ExitCode::SUCCESS;
    }

    let summary = build_pr_summary(&old, &new, &diff);
    let outcome_count = count_outcome_changes(&diff);
    let provider_count = count_affected_providers(&diff);
    let today = chrono::Utc::now().format("%Y-%m-%d");
    let summary_json = serde_json::to_string_pretty(&summary).unwrap_or_default();

    // Output title on first line, then blank, then body
    println!("eval-calibration: drift detected on {today} ({outcome_count} outcome changes)");
    println!();
    println!("## Eval Calibration Drift");
    println!();
    println!("Weekly calibration run detected drift against committed baseline.");
    println!();
    println!(
        "**Summary:** {} total changes across {} providers.",
        diff.changes.len(),
        provider_count
    );
    println!();
    println!("```json");
    println!("{summary_json}");
    println!("```");
    println!();
    println!("## Review checklist");
    println!(
        "- [ ] Outcome changes are legitimate (provider/model actually behaves differently), not a workflow bug"
    );
    println!("- [ ] No calibration-reset flag triggered unexpectedly");
    println!("- [ ] Scenarios affected are reasonable targets for the provider/model change");
    println!("- [ ] Merge updates baseline; decline closes this PR without merging");

    ExitCode::from(1) // Drift detected
}

fn print_usage() {
    eprintln!("eval-diff — Calibration baseline management CLI (#742)");
    eprintln!();
    eprintln!("Usage:");
    eprintln!("  eval-diff bootstrap <artifact> [<baseline>]");
    eprintln!("    Copy a calibration artifact as the committed baseline.");
    eprintln!("    Default baseline: tests/fixtures/eval-baseline.json");
    eprintln!();
    eprintln!("  eval-diff diff <baseline> <new>");
    eprintln!("    Semantic diff between two calibration artifacts.");
    eprintln!("    Exit code: 0 = no drift, 1 = drift, 2 = error.");
    eprintln!();
    eprintln!("  eval-diff format-pr-body <baseline> <new>");
    eprintln!("    Output Markdown title + body for a drift PR.");
    eprintln!("    Exit code: 0 = no drift, 1 = drift, 2 = error.");
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        print_usage();
        return ExitCode::from(2);
    }

    let default_baseline = PathBuf::from("tests/fixtures/eval-baseline.json");

    match args[1].as_str() {
        "bootstrap" => {
            if args.len() < 3 {
                eprintln!("Error: bootstrap requires <artifact> path.");
                print_usage();
                return ExitCode::from(2);
            }
            let artifact = PathBuf::from(&args[2]);
            let baseline = if args.len() >= 4 {
                PathBuf::from(&args[3])
            } else {
                default_baseline
            };
            cmd_bootstrap(&artifact, &baseline)
        }
        "diff" => {
            if args.len() < 4 {
                eprintln!("Error: diff requires <baseline> and <new> paths.");
                print_usage();
                return ExitCode::from(2);
            }
            cmd_diff(&PathBuf::from(&args[2]), &PathBuf::from(&args[3]))
        }
        "format-pr-body" => {
            if args.len() < 4 {
                eprintln!("Error: format-pr-body requires <baseline> and <new> paths.");
                print_usage();
                return ExitCode::from(2);
            }
            cmd_format_pr_body(&PathBuf::from(&args[2]), &PathBuf::from(&args[3]))
        }
        "--help" | "-h" | "help" => {
            print_usage();
            ExitCode::SUCCESS
        }
        other => {
            eprintln!("Error: unknown subcommand '{other}'.");
            print_usage();
            ExitCode::from(2)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[allow(clippy::type_complexity)]
    fn make_artifact(providers: Vec<(&str, &str, Vec<(&str, &str)>)>) -> CalibrationArtifact {
        let mut provider_map = BTreeMap::new();
        for (provider, model, scenarios) in providers {
            let mut scenario_map = BTreeMap::new();
            for (scenario, outcome) in scenarios {
                scenario_map.insert(
                    scenario.to_string(),
                    ScenarioCalibration {
                        outcome: outcome.to_string(),
                        error_class: None,
                        input_tokens: Some(100),
                        output_tokens: Some(50),
                        latency_ms: 200,
                    },
                );
            }
            provider_map.insert(
                provider.to_string(),
                ProviderCalibration {
                    model: model.to_string(),
                    scenarios: scenario_map,
                },
            );
        }
        CalibrationArtifact {
            version: 1,
            timestamp: "2026-04-23T08:00:00Z".to_string(),
            providers: provider_map,
        }
    }

    #[test]
    fn test_no_drift_identical_artifacts() {
        let artifact = make_artifact(vec![(
            "anthropic",
            "claude-sonnet-4-6",
            vec![("basic", "pass")],
        )]);
        let result = diff_calibrations(&artifact, &artifact);
        assert!(result.changes.is_empty());
    }

    #[test]
    fn test_outcome_change_detected() {
        let old = make_artifact(vec![(
            "anthropic",
            "claude-sonnet-4-6",
            vec![("basic", "pass")],
        )]);
        let new = make_artifact(vec![(
            "anthropic",
            "claude-sonnet-4-6",
            vec![("basic", "fail")],
        )]);
        let result = diff_calibrations(&old, &new);
        assert_eq!(result.changes.len(), 1);
        assert_eq!(result.changes[0].change_type, "tolerance_change");
        assert_eq!(result.changes[0].old_outcome, "pass");
        assert_eq!(result.changes[0].new_outcome, "fail");
    }

    #[test]
    fn test_new_provider_detected() {
        let old = make_artifact(vec![(
            "anthropic",
            "claude-sonnet-4-6",
            vec![("basic", "pass")],
        )]);
        let new = make_artifact(vec![
            ("anthropic", "claude-sonnet-4-6", vec![("basic", "pass")]),
            ("openai", "gpt-4o", vec![("basic", "pass")]),
        ]);
        let result = diff_calibrations(&old, &new);
        assert_eq!(result.changes.len(), 1);
        assert_eq!(result.changes[0].change_type, "new_provider");
        assert_eq!(result.changes[0].provider, "openai");
    }

    #[test]
    fn test_removed_scenario_detected() {
        let old = make_artifact(vec![(
            "anthropic",
            "claude-sonnet-4-6",
            vec![("basic", "pass"), ("multi", "pass")],
        )]);
        let new = make_artifact(vec![(
            "anthropic",
            "claude-sonnet-4-6",
            vec![("basic", "pass")],
        )]);
        let result = diff_calibrations(&old, &new);
        assert_eq!(result.changes.len(), 1);
        assert_eq!(result.changes[0].change_type, "removed");
        assert_eq!(result.changes[0].scenario, "multi");
    }

    #[test]
    fn test_classify_change_types() {
        let diff = CalibrationDiff {
            provider: "test".into(),
            scenario: "test".into(),
            old_outcome: "pass".into(),
            new_outcome: "fail".into(),
            change_type: "tolerance_change".into(),
        };
        assert_eq!(classify_change(&diff), "outcome_change");

        let diff2 = CalibrationDiff {
            change_type: "new_provider".into(),
            ..diff.clone()
        };
        assert_eq!(classify_change(&diff2), "new_provider");
    }

    #[test]
    fn test_pr_summary_format() {
        let old = make_artifact(vec![(
            "anthropic",
            "claude-sonnet-4-6",
            vec![("basic", "pass")],
        )]);
        let mut new = make_artifact(vec![(
            "anthropic",
            "claude-sonnet-4-6",
            vec![("basic", "fail")],
        )]);
        new.timestamp = "2026-04-30T08:00:00Z".to_string();

        let diff = diff_calibrations(&old, &new);
        let summary = build_pr_summary(&old, &new, &diff);

        assert_eq!(
            summary.previous_baseline_captured_at,
            "2026-04-23T08:00:00Z"
        );
        assert_eq!(summary.new_calibration_captured_at, "2026-04-30T08:00:00Z");
        assert_eq!(summary.changes.len(), 1);
        assert_eq!(summary.changes[0].classification, "outcome_change");
        assert_eq!(summary.changes[0].model, "claude-sonnet-4-6");
    }

    #[test]
    fn test_count_outcome_changes() {
        let old = make_artifact(vec![(
            "anthropic",
            "claude-sonnet-4-6",
            vec![("basic", "pass")],
        )]);
        let new = make_artifact(vec![
            ("anthropic", "claude-sonnet-4-6", vec![("basic", "fail")]),
            ("openai", "gpt-4o", vec![("basic", "pass")]),
        ]);
        let diff = diff_calibrations(&old, &new);
        assert_eq!(count_outcome_changes(&diff), 1); // only tolerance_change counts
    }

    #[test]
    fn test_bootstrap_creates_baseline() {
        let dir = tempfile::tempdir().unwrap();
        let artifact_path = dir.path().join("artifact.json");
        let baseline_path = dir.path().join("subdir/baseline.json");

        let artifact = make_artifact(vec![(
            "anthropic",
            "claude-sonnet-4-6",
            vec![("basic", "pass")],
        )]);
        let json = serde_json::to_string_pretty(&artifact).unwrap();
        std::fs::write(&artifact_path, &json).unwrap();

        let exit = cmd_bootstrap(&artifact_path, &baseline_path);
        assert_eq!(exit, ExitCode::SUCCESS);
        assert!(baseline_path.exists());

        let content = std::fs::read_to_string(&baseline_path).unwrap();
        assert_eq!(content, json);
    }

    #[test]
    fn test_bootstrap_invalid_artifact() {
        let dir = tempfile::tempdir().unwrap();
        let artifact_path = dir.path().join("bad.json");
        std::fs::write(&artifact_path, "not json").unwrap();

        let exit = cmd_bootstrap(&artifact_path, &dir.path().join("baseline.json"));
        assert_eq!(exit, ExitCode::from(2));
    }

    #[test]
    fn test_diff_no_drift_exit_0() {
        let dir = tempfile::tempdir().unwrap();
        let artifact = make_artifact(vec![(
            "anthropic",
            "claude-sonnet-4-6",
            vec![("basic", "pass")],
        )]);
        let json = serde_json::to_string_pretty(&artifact).unwrap();

        let path_a = dir.path().join("a.json");
        let path_b = dir.path().join("b.json");
        std::fs::write(&path_a, &json).unwrap();
        std::fs::write(&path_b, &json).unwrap();

        let exit = cmd_diff(&path_a, &path_b);
        assert_eq!(exit, ExitCode::SUCCESS);
    }

    #[test]
    fn test_diff_with_drift_exit_1() {
        let dir = tempfile::tempdir().unwrap();
        let old = make_artifact(vec![(
            "anthropic",
            "claude-sonnet-4-6",
            vec![("basic", "pass")],
        )]);
        let new = make_artifact(vec![(
            "anthropic",
            "claude-sonnet-4-6",
            vec![("basic", "fail")],
        )]);

        let path_a = dir.path().join("a.json");
        let path_b = dir.path().join("b.json");
        std::fs::write(&path_a, serde_json::to_string_pretty(&old).unwrap()).unwrap();
        std::fs::write(&path_b, serde_json::to_string_pretty(&new).unwrap()).unwrap();

        let exit = cmd_diff(&path_a, &path_b);
        assert_eq!(exit, ExitCode::from(1));
    }
}
