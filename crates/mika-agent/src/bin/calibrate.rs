//! Model calibration binary — pre-swap gate for agent model changes.
//!
//! Runs a role-scoped scenario suite against a target model and produces
//! a JSON artifact + markdown report.
//!
//! Usage:
//!   calibrate --role mika-dev --model anthropic/claude-sonnet-4-6
//!   calibrate --role mika-arch --model anthropic/claude-opus-4-6 --baseline docs/eval/calibration/baselines/latest.md

use std::path::PathBuf;

use clap::Parser;

use mika_agent::calibration::artifact::CalibrationArtifact;
use mika_agent::calibration::providers::create_provider_from_spec;
use mika_agent::calibration::role::RoleScoreReport;
use mika_agent::calibration::roles::{mika_arch, mika_dev};

/// Model calibration gate for Mika agent roles.
///
/// Runs a fixed scenario suite against a target model and reports
/// pass-rate, cost, latency, and failure-mode breakdown.
#[derive(Parser, Debug)]
#[command(name = "calibrate", version, about)]
struct Args {
    /// Agent role to calibrate (mika-dev, mika-arch).
    #[arg(short, long)]
    role: String,

    /// Target model in provider/model format (e.g., anthropic/claude-sonnet-4-6).
    #[arg(short, long)]
    model: String,

    /// Path to baseline report for comparison. Pass-rate must meet or exceed baseline.
    #[arg(short, long)]
    baseline: Option<PathBuf>,

    /// Output path for the JSON artifact. Defaults to target/eval-calibration/<role>-<timestamp>.json.
    #[arg(short, long)]
    output: Option<PathBuf>,

    /// Maximum cost budget in USD. v1: reported only (DR-7). v2 will enforce mid-suite abort.
    #[arg(long, default_value = "5.0")]
    _max_cost_usd: f64,

    /// Number of times to run each scenario. v1: always 1 (DR-8). v2 will support N≥3 averaging.
    #[arg(long, default_value = "1")]
    _runs_per_scenario: u32,
}

#[tokio::main]
async fn main() {
    let args = Args::parse();

    // Validate role
    let scenarios = match args.role.as_str() {
        "mika-dev" => mika_dev::SCENARIOS,
        "mika-arch" => mika_arch::SCENARIOS,
        other => {
            eprintln!(
                "Error: unknown role '{}'. Valid roles: mika-dev, mika-arch",
                other
            );
            std::process::exit(2);
        }
    };

    // Create provider
    let provider = match create_provider_from_spec(&args.model) {
        Some(p) => p,
        None => {
            eprintln!(
                "Error: could not create provider for '{}'. Check API key is set.",
                args.model
            );
            std::process::exit(2);
        }
    };

    println!("╭─────────────────────────────────────────────────────╮");
    println!(
        "│  Model Calibration: {}                              ",
        args.role
    );
    println!(
        "│  Model: {}                                          ",
        args.model
    );
    println!(
        "│  Scenarios: {}                                      ",
        scenarios.len()
    );
    println!("╰─────────────────────────────────────────────────────╯");
    println!();

    // Run scenarios
    let mut results = Vec::new();
    for scenario in scenarios {
        print!("  Running {}... ", scenario.id);

        let result = match args.role.as_str() {
            "mika-dev" => mika_dev::run_scenario(scenario.id, provider.clone()).await,
            "mika-arch" => mika_arch::run_scenario(scenario.id, provider.clone()).await,
            _ => unreachable!(),
        };

        if result.passed {
            println!("PASS ({}ms)", result.latency_ms);
        } else {
            println!("FAIL: {}", result.error.as_deref().unwrap_or("unknown"));
        }

        results.push(result);
    }

    println!();

    // Build score report
    let report = RoleScoreReport::from_results(&args.role, &args.model, results.clone(), scenarios);

    // Print summary
    println!("═══════════════════════════════════════════════════════");
    println!(
        "  Pass rate: {:.1}% ({}/{})",
        report.pass_rate * 100.0,
        report.passed,
        report.total_scenarios
    );
    println!(
        "  Tokens: {} in / {} out",
        report.total_input_tokens, report.total_output_tokens
    );
    println!("  Latency: {}ms total", report.total_latency_ms);
    if !report.failure_breakdown.is_empty() {
        println!("  Failures:");
        for (class, count) in &report.failure_breakdown {
            println!("    {}: {}", class, count);
        }
    }
    println!("═══════════════════════════════════════════════════════");

    // Write artifact
    let outcomes: Vec<_> = results
        .iter()
        .map(|r| r.to_scenario_outcome(&args.role, &args.model))
        .collect();
    let artifact = CalibrationArtifact::from_outcomes(&outcomes);

    let output_path = args.output.unwrap_or_else(|| {
        let timestamp = chrono::Utc::now().format("%Y%m%d-%H%M%S");
        PathBuf::from(format!(
            "target/eval-calibration/{}-{}.json",
            args.role, timestamp
        ))
    });

    if let Err(e) = artifact.write_to(&output_path) {
        eprintln!("Warning: failed to write artifact: {}", e);
    } else {
        println!("\n  Artifact: {}", output_path.display());
    }

    // Write markdown report
    let md_path = output_path.with_extension("md");
    let markdown = report.to_markdown();
    if let Err(e) = std::fs::write(&md_path, &markdown) {
        eprintln!("Warning: failed to write markdown report: {}", e);
    } else {
        println!("  Report:   {}", md_path.display());
    }

    // Compare with baseline if provided
    if let Some(baseline_path) = &args.baseline {
        if baseline_path.exists() {
            println!("\n  Comparing with baseline: {}", baseline_path.display());
            match CalibrationArtifact::load(baseline_path) {
                Ok(baseline) => {
                    let diff =
                        mika_agent::calibration::artifact::diff_calibrations(&baseline, &artifact);
                    if diff.changes.is_empty() {
                        println!("  No changes from baseline.");
                    } else {
                        println!("  Changes from baseline:");
                        for change in &diff.changes {
                            println!(
                                "    {} / {}: {} → {} ({})",
                                change.provider,
                                change.scenario,
                                change.old_outcome,
                                change.new_outcome,
                                change.change_type
                            );
                        }
                    }

                    // Check pass rate against baseline (unweighted for both —
                    // artifact does not carry weights per DR-7)
                    let baseline_pass_count = baseline
                        .providers
                        .values()
                        .flat_map(|p| p.scenarios.values())
                        .filter(|s| s.outcome == "pass")
                        .count();
                    let baseline_total = baseline
                        .providers
                        .values()
                        .flat_map(|p| p.scenarios.values())
                        .count();
                    let baseline_rate = if baseline_total > 0 {
                        baseline_pass_count as f64 / baseline_total as f64
                    } else {
                        0.0
                    };

                    // Use unweighted pass rate for comparison (consistent with baseline)
                    let current_unweighted_rate =
                        report.passed as f64 / report.total_scenarios.max(1) as f64;
                    if current_unweighted_rate < baseline_rate {
                        eprintln!(
                            "\n  ❌ FAIL: pass rate {:.1}% < baseline {:.1}%",
                            current_unweighted_rate * 100.0,
                            baseline_rate * 100.0
                        );
                        std::process::exit(1);
                    } else {
                        println!(
                            "\n  ✓ PASS: pass rate {:.1}% >= baseline {:.1}%",
                            current_unweighted_rate * 100.0,
                            baseline_rate * 100.0
                        );
                    }
                }
                Err(e) => {
                    eprintln!("  Warning: could not load baseline: {}", e);
                }
            }
        } else {
            eprintln!(
                "  Warning: baseline path does not exist: {}",
                baseline_path.display()
            );
        }
    }

    // Exit code: 0 = pass, 1 = fail
    if report.pass_rate < 1.0 && args.baseline.is_some() {
        // Only fail hard if there's a baseline to compare against
        // Without baseline, we're just establishing one
    }

    println!("\n  Done.");
}
