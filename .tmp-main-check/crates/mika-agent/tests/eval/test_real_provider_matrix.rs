//! Real-provider matrix runner (D3).
//!
//! Iterates configured providers × scenarios, reporting per-tuple outcomes.
//! All tests carry `#[ignore]` — structural intent gate per D8.
//! Run with: `cargo test -p mika-agent --test eval -- --ignored`
//! And set: `MIKA_EVAL_REAL_PROVIDERS=anthropic,openai,kimi,groq`

use std::sync::Arc;

use super::providers::{create_real_provider, parse_real_providers};
use super::scenarios::ScenarioRegistry;

/// Run the full matrix: all configured providers × all scenarios.
/// Reports results as a formatted table to stdout.
#[tokio::test]
#[ignore]
async fn real_provider_matrix() {
    let providers = parse_real_providers();
    if providers.is_empty() {
        println!("MIKA_EVAL_REAL_PROVIDERS not set — skipping matrix.");
        return;
    }

    let registry = ScenarioRegistry::default_scenarios();
    let mut results = Vec::new();

    for kind in &providers {
        let provider = match create_real_provider(*kind) {
            Some(p) => p,
            None => {
                println!("⏭ Skipping {} — no API key configured", kind);
                continue;
            }
        };

        for scenario in &registry.scenarios {
            println!(
                "▶ Running {} against {} ({})",
                scenario.name,
                kind,
                provider.model_name()
            );
            let outcome = (scenario.run)(Arc::clone(&provider)).await;
            let status = if outcome.success { "✓" } else { "✗" };
            println!(
                "  {} {} — {}ms{}",
                status,
                scenario.name,
                outcome.latency_ms,
                outcome
                    .error
                    .as_ref()
                    .map(|e| format!(" — {e}"))
                    .unwrap_or_default()
            );
            results.push(outcome);
        }
    }

    // Print summary table
    println!("\n=== Matrix Results ===");
    let header = format!(
        "{:<25} {:<15} {:<10} {:<10} Tokens (in/out)",
        "Scenario", "Provider", "Status", "Latency"
    );
    println!("{header}");
    let separator = "-".repeat(80);
    println!("{separator}");
    for r in &results {
        println!(
            "{:<25} {:<15} {:<10} {:<10} {}/{}",
            r.scenario,
            r.provider,
            if r.success { "PASS" } else { "FAIL" },
            format!("{}ms", r.latency_ms),
            r.input_tokens.unwrap_or(0),
            r.output_tokens.unwrap_or(0),
        );
    }

    let pass_count = results.iter().filter(|r| r.success).count();
    let fail_count = results.len() - pass_count;
    println!(
        "\nTotal: {} passed, {} failed out of {} runs",
        pass_count,
        fail_count,
        results.len()
    );

    // Write calibration artifact if MIKA_EVAL_CALIBRATE is set
    if std::env::var("MIKA_EVAL_CALIBRATE").is_ok() {
        use super::calibration::CalibrationArtifact;
        let artifact = CalibrationArtifact::from_outcomes(&results);
        match artifact.write_to_target() {
            Ok(path) => println!("\nCalibration artifact written to: {}", path.display()),
            Err(e) => eprintln!("\nFailed to write calibration artifact: {e}"),
        }
    }
}

/// Per-provider convenience test: Anthropic only.
#[tokio::test]
#[ignore]
async fn anthropic_matrix() {
    run_single_provider_matrix("anthropic").await;
}

/// Per-provider convenience test: OpenAI only.
#[tokio::test]
#[ignore]
async fn openai_matrix() {
    run_single_provider_matrix("openai").await;
}

/// Per-provider convenience test: Kimi only.
#[tokio::test]
#[ignore]
async fn kimi_matrix() {
    run_single_provider_matrix("kimi").await;
}

/// Per-provider convenience test: Groq only.
#[tokio::test]
#[ignore]
async fn groq_matrix() {
    run_single_provider_matrix("groq").await;
}

async fn run_single_provider_matrix(name: &str) {
    let kind = name
        .parse::<mika_common::llm::ProviderKind>()
        .unwrap_or_else(|e| panic!("Invalid provider: {e}"));
    let provider = match create_real_provider(kind) {
        Some(p) => p,
        None => {
            println!(
                "⏭ Skipping {} — no API key configured. Set MIKA_{}_API_KEY",
                name,
                kind.config_prefix().to_uppercase()
            );
            return;
        }
    };

    let registry = ScenarioRegistry::default_scenarios();
    for scenario in &registry.scenarios {
        println!(
            "▶ Running {} against {}",
            scenario.name,
            provider.model_name()
        );
        let outcome = (scenario.run)(Arc::clone(&provider)).await;
        let status = if outcome.success {
            "✓ PASS"
        } else {
            "✗ FAIL"
        };
        println!(
            "  {} — {}ms, {}/{} tokens{}",
            status,
            outcome.latency_ms,
            outcome.input_tokens.unwrap_or(0),
            outcome.output_tokens.unwrap_or(0),
            outcome
                .error
                .as_ref()
                .map(|e| format!("\n  Error: {e}"))
                .unwrap_or_default()
        );
    }
}
