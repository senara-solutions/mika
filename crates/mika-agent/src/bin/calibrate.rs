//! Model calibration binary — pre-swap gate for agent model changes.
//!
//! Runs a role-scoped scenario suite against a target model and produces
//! a JSON artifact + markdown report.
//!
//! Usage:
//!   calibrate --role mika-dev --model anthropic/claude-sonnet-4-6 --baseline docs/eval/calibration/baselines/mika-dev.json
//!   calibrate --role mika-arch --model anthropic/claude-opus-4-6 --establish-baseline --baseline docs/eval/calibration/baselines/mika-arch.json
//!
//! ## Gate contract (#1701)
//!
//! This binary is the sole structural guard on agent model swaps (#1190). Its
//! exit code — not just its stdout — reflects the verdict:
//!
//! - **exit 0** — gate passed: 100% of the role's scenarios passed AND the run
//!   met or exceeded the baseline. Also emitted when `--establish-baseline`
//!   successfully writes a new baseline.
//! - **exit 1** — gate failed: pass-rate fell below the 100% floor (the failing
//!   scenarios are named), or regressed below the baseline. Also emitted when
//!   `--establish-baseline` refuses to persist a failing baseline (override with
//!   `--force-failing-baseline`).
//! - **exit 2** — gate not enforceable: the `--baseline` file is absent, missing,
//!   or unloadable (e.g. schema drift). A missing baseline is NOT a silent pass —
//!   supply a valid baseline, or pass `--establish-baseline` to create one.
//!
//! Before #1701 every path exited 0 regardless of pass-rate, so any swap "gated"
//! by this command was never actually gated. The decision logic lives in
//! `mika_agent::calibration::gate` (unit-tested there + in `tests/calibrate_integration.rs`).
//!
//! ## Authentication
//!
//! The binary performs the same dotenv initialization as mika-spirit
//! (`resolve_home_dir` + `load_dotenv`) before creating the LLM provider.
//! For Anthropic OAuth tokens (`sk-ant-oat*`), this ensures the
//! `OAuthTokenManager` can resolve the token cache at `~/.mika/oauth.json`.
//! The `check_health()` preflight (below) verifies auth before running
//! scenarios — if mika-spirit can call Anthropic on this host, so can
//! `calibrate`.

use std::path::PathBuf;

use clap::Parser;

use mika_agent::calibration::artifact::{CalibrationArtifact, diff_calibrations};
use mika_agent::calibration::gate::{BaselineState, GateOutcome, PASS_RATE_FLOOR, evaluate_gate};
use mika_agent::calibration::providers::create_provider_from_spec;
use mika_agent::calibration::role::RoleScoreReport;
use mika_agent::calibration::roles::{mika_arch, mika_dev, mika_orchestrator, mika_qa};

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
    /// Required in gate mode — a missing/unloadable baseline exits 2 (gate not enforceable).
    #[arg(short, long)]
    baseline: Option<PathBuf>,

    /// Write this run's artifact as the baseline (at --baseline, or --output) and exit,
    /// bypassing the gate. Refuses to persist a failing (< 100%) run unless
    /// --force-failing-baseline is also set.
    #[arg(long)]
    establish_baseline: bool,

    /// With --establish-baseline, allow writing a baseline even when the run failed
    /// the 100% floor. A failing baseline lowers the bar for every future compare,
    /// so this is off by default (#1701 AC3).
    #[arg(long)]
    force_failing_baseline: bool,

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

    // Initialize environment — same sequence as mika-spirit
    let home_dir = match mika_common::home::resolve_home_dir() {
        Ok(h) => h,
        Err(e) => {
            eprintln!("Error: could not resolve Mika home directory: {e}");
            std::process::exit(2);
        }
    };
    mika_common::dotenv::load_dotenv(&home_dir);

    // Validate role
    let scenarios = match args.role.as_str() {
        "mika-dev" => mika_dev::SCENARIOS,
        "mika-arch" => mika_arch::SCENARIOS,
        "mika-qa" => mika_qa::SCENARIOS,
        "mika-orchestrator" => mika_orchestrator::SCENARIOS,
        other => {
            eprintln!(
                "Error: unknown role '{}'. Valid roles: mika-dev, mika-arch, mika-qa, mika-orchestrator",
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

    // Pre-flight: verify provider can authenticate
    print!("  Verifying provider authentication... ");
    match provider.check_health().await {
        Ok(()) => println!("OK"),
        Err(e) => {
            let err_str = e.to_string();
            eprintln!("FAIL");
            eprintln!();
            // Surface OAuth-specific guidance when the error chain mentions it
            if err_str.to_lowercase().contains("oauth") {
                eprintln!("Error: OAuth authentication failed for Anthropic provider.");
                eprintln!("  The calibrate binary uses the same OAuth flow as mika-spirit.");
                eprintln!("  Ensure `mika setup --mode oauth` has been completed and");
                eprintln!("  that ~/.mika/oauth.json exists with valid, non-expired tokens.");
                eprintln!();
                eprintln!("  To verify: check that mika-spirit can call Anthropic successfully.");
                eprintln!("  If mika-spirit works but calibrate doesn't, the subscription token");
                eprintln!("  in MIKA_ANTHROPIC_API_KEY may have been rotated since the last");
                eprintln!("  `mika setup --mode oauth` run.");
            }
            eprintln!("  Provider error: {}", err_str);
            std::process::exit(2);
        }
    }
    println!();

    // Run scenarios
    let mut results = Vec::new();
    for scenario in scenarios {
        print!("  Running {}... ", scenario.id);

        let result = match args.role.as_str() {
            "mika-dev" => mika_dev::run_scenario(scenario.id, provider.clone()).await,
            "mika-arch" => mika_arch::run_scenario(scenario.id, provider.clone()).await,
            "mika-qa" => mika_qa::run_scenario(scenario.id, provider.clone()).await,
            "mika-orchestrator" => {
                mika_orchestrator::run_scenario(scenario.id, provider.clone()).await
            }
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

    // ── Swap-gate (#1701) ────────────────────────────────────────────────
    // The gate's *exit code* is the verdict — not just stdout. Resolve the
    // baseline state, print the informative diff when one loads, then delegate
    // the exit-code decision to the pure, unit-tested `gate::evaluate_gate`.
    let current_rate = report.unweighted_pass_rate();

    // Load the baseline (if a path was given) into a `BaselineState`. A path
    // that is missing or fails to load is NOT a silent pass — it renders the
    // gate unenforceable (exit 2).
    let baseline_state = match &args.baseline {
        None => BaselineState::NotProvided,
        Some(path) if !path.exists() => BaselineState::Missing,
        Some(path) => match CalibrationArtifact::load(path) {
            Ok(baseline) => {
                println!("\n  Comparing with baseline: {}", path.display());
                let diff = diff_calibrations(&baseline, &artifact);
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
                BaselineState::Loaded {
                    pass_rate: baseline.unweighted_pass_rate(),
                }
            }
            Err(e) => {
                eprintln!(
                    "\n  Error: could not load baseline {}: {}",
                    path.display(),
                    e
                );
                BaselineState::Unloadable
            }
        },
    };

    let outcome = evaluate_gate(
        current_rate,
        &baseline_state,
        args.establish_baseline,
        args.force_failing_baseline,
    );

    // Establish target: --baseline path if given, else the artifact output path.
    let establish_target = args.baseline.clone().unwrap_or_else(|| output_path.clone());

    println!();
    match &outcome {
        GateOutcome::Pass => {
            println!(
                "  ✓ PASS: pass rate {:.1}% meets the 100% floor{}.",
                current_rate * 100.0,
                match &baseline_state {
                    BaselineState::Loaded { pass_rate } =>
                        format!(" and baseline {:.1}%", pass_rate * 100.0),
                    _ => String::new(),
                }
            );
        }
        GateOutcome::FailFloor => {
            eprintln!(
                "  ❌ FAIL: pass rate {:.1}% is below the required 100% floor (delta {:.1}%).",
                current_rate * 100.0,
                (PASS_RATE_FLOOR - current_rate) * 100.0
            );
            eprintln!(
                "  Failing scenarios: {}",
                report.failing_scenario_ids().join(", ")
            );
            eprintln!("  A model that fails scenarios must not be swapped in (#1190).");
        }
        GateOutcome::FailBaseline => {
            let baseline_rate = match &baseline_state {
                BaselineState::Loaded { pass_rate } => *pass_rate,
                _ => 0.0,
            };
            eprintln!(
                "  ❌ FAIL: pass rate {:.1}% regressed below baseline {:.1}%.",
                current_rate * 100.0,
                baseline_rate * 100.0
            );
            eprintln!(
                "  Failing scenarios: {}",
                report.failing_scenario_ids().join(", ")
            );
        }
        GateOutcome::NotEnforceable => {
            let path_hint = args
                .baseline
                .as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| "<none provided>".to_string());
            eprintln!(
                "  ⚠️  GATE NOT ENFORCEABLE: no usable baseline ({}).",
                path_hint
            );
            eprintln!(
                "  Supply a valid --baseline, or pass --establish-baseline to write one from this run."
            );
            eprintln!("  Exiting 2 — this is NOT a pass. Nothing was verified.");
        }
        GateOutcome::BaselineEstablished => {
            if let Err(e) = artifact.write_to(&establish_target) {
                eprintln!(
                    "  Error: failed to write baseline to {}: {}",
                    establish_target.display(),
                    e
                );
                std::process::exit(2);
            }
            let forced = if current_rate < PASS_RATE_FLOOR {
                " (FORCED — baseline is below the 100% floor)"
            } else {
                ""
            };
            println!(
                "  ✓ Baseline established at {} — pass rate {:.1}%{}.",
                establish_target.display(),
                current_rate * 100.0,
                forced
            );
        }
        GateOutcome::BaselineRefused => {
            eprintln!(
                "  ❌ REFUSED: will not write a failing baseline (pass rate {:.1}% < 100%).",
                current_rate * 100.0
            );
            eprintln!(
                "  Failing scenarios: {}",
                report.failing_scenario_ids().join(", ")
            );
            eprintln!(
                "  A failing baseline poisons every future compare. Pass --force-failing-baseline to override."
            );
        }
    }

    println!("\n  Done.");
    std::process::exit(outcome.exit_code());
}
