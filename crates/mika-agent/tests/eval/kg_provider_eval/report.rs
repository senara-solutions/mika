//! Report generation for the KG provider eval (#762).
//!
//! Aggregates extraction and resolution outcomes per provider, computes summary
//! statistics, prints comparison tables, and writes calibration artifacts.

use std::collections::BTreeMap;
use std::sync::Arc;

use mika_common::llm::LlmProvider;
use serde::Serialize;

use super::cost;
use super::{KgExtractionOutcome, KgProviderSpec, KgResolutionOutcome};

// ---------------------------------------------------------------------------
// Aggregated stats
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
struct ProviderExtractionStats {
    provider: String,
    doc_count: usize,
    json_parse_rate: f64,
    avg_entity_count: f64,
    avg_relationship_count: f64,
    avg_name_quality: f64,
    avg_type_quality: f64,
    p50_latency_ms: u64,
    p95_latency_ms: u64,
    avg_input_tokens: u64,
    avg_output_tokens: u64,
    per_call_cost: f64,
    steady_state_cost: f64,
    error_count: usize,
}

#[derive(Debug, Clone, Serialize)]
struct ProviderResolutionStats {
    provider: String,
    case_count: usize,
    accuracy: f64,
    avg_confidence: f64,
    p50_latency_ms: u64,
    p95_latency_ms: u64,
    avg_input_tokens: u64,
    avg_output_tokens: u64,
    per_call_cost: f64,
    steady_state_cost: f64,
    error_count: usize,
    /// Per-category accuracy breakdown.
    category_accuracy: BTreeMap<String, f64>,
}

// ---------------------------------------------------------------------------
// Calibration artifact
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
struct KgCalibrationArtifact {
    version: u32,
    timestamp: String,
    extraction: BTreeMap<String, ProviderExtractionStats>,
    resolution: BTreeMap<String, ProviderResolutionStats>,
}

// ---------------------------------------------------------------------------
// Report generation
// ---------------------------------------------------------------------------

/// Generate and print the full comparison report, and write calibration artifacts.
pub fn generate_report(
    extraction_outcomes: &[KgExtractionOutcome],
    resolution_outcomes: &[KgResolutionOutcome],
    providers: &[(KgProviderSpec, Arc<dyn LlmProvider>)],
) {
    let provider_names: Vec<String> = providers
        .iter()
        .map(|(s, _)| s.display_name.clone())
        .collect();

    let extraction_stats = compute_extraction_stats(extraction_outcomes, &provider_names);
    let resolution_stats = compute_resolution_stats(resolution_outcomes, &provider_names);

    print_extraction_table(&extraction_stats);
    println!();
    print_resolution_table(&resolution_stats);
    println!();
    print_cost_comparison(&extraction_stats, &resolution_stats);

    // Write calibration artifact
    write_calibration_artifact(&extraction_stats, &resolution_stats);
}

/// Print extraction-only summary.
pub fn print_extraction_summary(outcomes: &[KgExtractionOutcome]) {
    let provider_names: Vec<String> = outcomes
        .iter()
        .map(|o| o.provider.clone())
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect();
    let stats = compute_extraction_stats(outcomes, &provider_names);
    print_extraction_table(&stats);
}

/// Print resolution-only summary.
pub fn print_resolution_summary(outcomes: &[KgResolutionOutcome]) {
    let provider_names: Vec<String> = outcomes
        .iter()
        .map(|o| o.provider.clone())
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect();
    let stats = compute_resolution_stats(outcomes, &provider_names);
    print_resolution_table(&stats);
}

// ---------------------------------------------------------------------------
// Stats computation
// ---------------------------------------------------------------------------

fn compute_extraction_stats(
    outcomes: &[KgExtractionOutcome],
    provider_names: &[String],
) -> Vec<ProviderExtractionStats> {
    let mut stats = Vec::new();

    for name in provider_names {
        let provider_outcomes: Vec<&KgExtractionOutcome> =
            outcomes.iter().filter(|o| &o.provider == name).collect();

        if provider_outcomes.is_empty() {
            continue;
        }

        let doc_count = provider_outcomes.len();
        let json_parse_ok = provider_outcomes.iter().filter(|o| o.json_parse_ok).count();
        let error_count = provider_outcomes
            .iter()
            .filter(|o| o.error.is_some())
            .count();

        let successful: Vec<&&KgExtractionOutcome> = provider_outcomes
            .iter()
            .filter(|o| o.json_parse_ok)
            .collect();

        let avg_entity_count = if successful.is_empty() {
            0.0
        } else {
            successful
                .iter()
                .map(|o| o.entity_count as f64)
                .sum::<f64>()
                / successful.len() as f64
        };

        let avg_relationship_count = if successful.is_empty() {
            0.0
        } else {
            successful
                .iter()
                .map(|o| o.relationship_count as f64)
                .sum::<f64>()
                / successful.len() as f64
        };

        let avg_name_quality = if successful.is_empty() {
            0.0
        } else {
            successful.iter().map(|o| o.name_quality).sum::<f64>() / successful.len() as f64
        };

        let avg_type_quality = if successful.is_empty() {
            0.0
        } else {
            successful.iter().map(|o| o.type_quality).sum::<f64>() / successful.len() as f64
        };

        let mut latencies: Vec<u64> = provider_outcomes.iter().map(|o| o.latency_ms).collect();
        latencies.sort();

        let p50_latency_ms = percentile(&latencies, 50);
        let p95_latency_ms = percentile(&latencies, 95);

        let avg_input_tokens = if provider_outcomes.is_empty() {
            0
        } else {
            provider_outcomes
                .iter()
                .map(|o| o.input_tokens)
                .sum::<u64>()
                / provider_outcomes.len() as u64
        };

        let avg_output_tokens = if provider_outcomes.is_empty() {
            0
        } else {
            provider_outcomes
                .iter()
                .map(|o| o.output_tokens)
                .sum::<u64>()
                / provider_outcomes.len() as u64
        };

        let model_name = name.split('/').skip(1).collect::<Vec<_>>().join("/");
        let model_for_cost = if model_name.is_empty() {
            name.as_str()
        } else {
            &model_name
        };
        let call_cost = cost::per_call_cost(model_for_cost, avg_input_tokens, avg_output_tokens);
        let ss_cost =
            cost::project_steady_state_cost(model_for_cost, avg_input_tokens, avg_output_tokens);

        stats.push(ProviderExtractionStats {
            provider: name.clone(),
            doc_count,
            json_parse_rate: json_parse_ok as f64 / doc_count as f64,
            avg_entity_count,
            avg_relationship_count,
            avg_name_quality,
            avg_type_quality,
            p50_latency_ms,
            p95_latency_ms,
            avg_input_tokens,
            avg_output_tokens,
            per_call_cost: call_cost,
            steady_state_cost: ss_cost,
            error_count,
        });
    }

    stats
}

fn compute_resolution_stats(
    outcomes: &[KgResolutionOutcome],
    provider_names: &[String],
) -> Vec<ProviderResolutionStats> {
    let mut stats = Vec::new();

    for name in provider_names {
        let provider_outcomes: Vec<&KgResolutionOutcome> =
            outcomes.iter().filter(|o| &o.provider == name).collect();

        if provider_outcomes.is_empty() {
            continue;
        }

        let case_count = provider_outcomes.len();
        let correct_count = provider_outcomes.iter().filter(|o| o.correct).count();
        let error_count = provider_outcomes
            .iter()
            .filter(|o| o.error.is_some())
            .count();

        let non_error: Vec<&&KgResolutionOutcome> = provider_outcomes
            .iter()
            .filter(|o| o.error.is_none())
            .collect();

        let avg_confidence = if non_error.is_empty() {
            0.0
        } else {
            non_error.iter().map(|o| o.confidence).sum::<f64>() / non_error.len() as f64
        };

        let mut latencies: Vec<u64> = provider_outcomes.iter().map(|o| o.latency_ms).collect();
        latencies.sort();

        let p50_latency_ms = percentile(&latencies, 50);
        let p95_latency_ms = percentile(&latencies, 95);

        let avg_input_tokens = if provider_outcomes.is_empty() {
            0
        } else {
            provider_outcomes
                .iter()
                .map(|o| o.input_tokens)
                .sum::<u64>()
                / provider_outcomes.len() as u64
        };

        let avg_output_tokens = if provider_outcomes.is_empty() {
            0
        } else {
            provider_outcomes
                .iter()
                .map(|o| o.output_tokens)
                .sum::<u64>()
                / provider_outcomes.len() as u64
        };

        // Per-category accuracy
        let mut category_totals: BTreeMap<String, (usize, usize)> = BTreeMap::new();
        for o in &provider_outcomes {
            let entry = category_totals
                .entry(o.case_category.clone())
                .or_insert((0, 0));
            entry.0 += 1; // total
            if o.correct {
                entry.1 += 1; // correct
            }
        }
        let category_accuracy: BTreeMap<String, f64> = category_totals
            .into_iter()
            .map(|(cat, (total, correct))| (cat, correct as f64 / total as f64))
            .collect();

        let model_name = name.split('/').skip(1).collect::<Vec<_>>().join("/");
        let model_for_cost = if model_name.is_empty() {
            name.as_str()
        } else {
            &model_name
        };
        let call_cost = cost::per_call_cost(model_for_cost, avg_input_tokens, avg_output_tokens);
        let ss_cost =
            cost::project_steady_state_cost(model_for_cost, avg_input_tokens, avg_output_tokens);

        stats.push(ProviderResolutionStats {
            provider: name.clone(),
            case_count,
            accuracy: correct_count as f64 / case_count as f64,
            avg_confidence,
            p50_latency_ms,
            p95_latency_ms,
            avg_input_tokens,
            avg_output_tokens,
            per_call_cost: call_cost,
            steady_state_cost: ss_cost,
            error_count,
            category_accuracy,
        });
    }

    stats
}

// ---------------------------------------------------------------------------
// Printing
// ---------------------------------------------------------------------------

fn print_extraction_table(stats: &[ProviderExtractionStats]) {
    println!("### Extraction Comparison\n");
    println!(
        "| {:<40} | {:>5} | {:>6} | {:>6} | {:>5} | {:>5} | {:>7} | {:>7} | {:>9} | {:>12} |",
        "Provider",
        "Docs",
        "Parse%",
        "Ents",
        "Name%",
        "Type%",
        "p50ms",
        "p95ms",
        "$/call",
        "$/steady"
    );
    println!(
        "|{:-<42}|{:-<7}|{:-<8}|{:-<8}|{:-<7}|{:-<7}|{:-<9}|{:-<9}|{:-<11}|{:-<14}|",
        "", "", "", "", "", "", "", "", "", ""
    );

    for s in stats {
        println!(
            "| {:<40} | {:>5} | {:>5.0}% | {:>6.1} | {:>4.0}% | {:>4.0}% | {:>7} | {:>7} | ${:>8.6} | ${:>11.2} |",
            s.provider,
            s.doc_count,
            s.json_parse_rate * 100.0,
            s.avg_entity_count,
            s.avg_name_quality * 100.0,
            s.avg_type_quality * 100.0,
            s.p50_latency_ms,
            s.p95_latency_ms,
            s.per_call_cost,
            s.steady_state_cost,
        );
    }
}

fn print_resolution_table(stats: &[ProviderResolutionStats]) {
    println!("### Resolution Comparison\n");
    println!(
        "| {:<40} | {:>5} | {:>6} | {:>6} | {:>7} | {:>7} | {:>9} | {:>12} |",
        "Provider", "Cases", "Acc%", "Conf", "p50ms", "p95ms", "$/call", "$/steady"
    );
    println!(
        "|{:-<42}|{:-<7}|{:-<8}|{:-<8}|{:-<9}|{:-<9}|{:-<11}|{:-<14}|",
        "", "", "", "", "", "", "", ""
    );

    for s in stats {
        println!(
            "| {:<40} | {:>5} | {:>5.1}% | {:>6.3} | {:>7} | {:>7} | ${:>8.6} | ${:>11.2} |",
            s.provider,
            s.case_count,
            s.accuracy * 100.0,
            s.avg_confidence,
            s.p50_latency_ms,
            s.p95_latency_ms,
            s.per_call_cost,
            s.steady_state_cost,
        );
    }

    // Per-category breakdown
    if !stats.is_empty() {
        println!("\n### Resolution Accuracy by Category\n");
        // Collect all categories
        let mut all_cats: Vec<String> = stats
            .iter()
            .flat_map(|s| s.category_accuracy.keys().cloned())
            .collect::<std::collections::HashSet<_>>()
            .into_iter()
            .collect();
        all_cats.sort();

        print!("| {:<20} |", "Category");
        for s in stats {
            let short_name: &str = s
                .provider
                .rsplit_once('/')
                .map(|(_, m)| m)
                .unwrap_or(&s.provider);
            print!(" {:>15} |", short_name);
        }
        println!();

        print!("|{:-<22}|", "");
        for _ in stats {
            print!("{:-<17}|", "");
        }
        println!();

        for cat in &all_cats {
            print!("| {:<20} |", cat);
            for s in stats {
                let acc = s.category_accuracy.get(cat).copied().unwrap_or(f64::NAN);
                if acc.is_nan() {
                    print!(" {:>14}% |", "N/A");
                } else {
                    print!(" {:>14.1}% |", acc * 100.0);
                }
            }
            println!();
        }
    }
}

fn print_cost_comparison(
    extraction_stats: &[ProviderExtractionStats],
    resolution_stats: &[ProviderResolutionStats],
) {
    println!("### Cost Summary\n");
    println!(
        "| {:<40} | {:>12} | {:>12} | {:>12} |",
        "Provider", "Extract/call", "Resolve/call", "Total/steady"
    );
    println!("|{:-<42}|{:-<14}|{:-<14}|{:-<14}|", "", "", "", "");

    for e_stat in extraction_stats {
        let r_stat = resolution_stats
            .iter()
            .find(|r| r.provider == e_stat.provider);

        let resolve_cost = r_stat.map(|r| r.per_call_cost).unwrap_or(0.0);
        let total_steady =
            e_stat.steady_state_cost + r_stat.map(|r| r.steady_state_cost).unwrap_or(0.0);

        println!(
            "| {:<40} | ${:>11.6} | ${:>11.6} | ${:>11.2} |",
            e_stat.provider, e_stat.per_call_cost, resolve_cost, total_steady,
        );
    }
}

// ---------------------------------------------------------------------------
// Calibration artifact
// ---------------------------------------------------------------------------

fn write_calibration_artifact(
    extraction_stats: &[ProviderExtractionStats],
    resolution_stats: &[ProviderResolutionStats],
) {
    let calibrate = std::env::var("MIKA_EVAL_CALIBRATE")
        .map(|v| v == "1")
        .unwrap_or(false);

    if !calibrate {
        return;
    }

    let artifact_dir =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/eval-calibration");
    if let Err(e) = std::fs::create_dir_all(&artifact_dir) {
        eprintln!(
            "[kg_provider_eval] failed to create calibration dir {:?}: {}",
            artifact_dir, e
        );
        return;
    }

    let mut extraction_map = BTreeMap::new();
    for s in extraction_stats {
        extraction_map.insert(s.provider.clone(), s.clone());
    }

    let mut resolution_map = BTreeMap::new();
    for s in resolution_stats {
        resolution_map.insert(s.provider.clone(), s.clone());
    }

    let artifact = KgCalibrationArtifact {
        version: 1,
        timestamp: chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string(),
        extraction: extraction_map,
        resolution: resolution_map,
    };

    let artifact_path = artifact_dir.join("kg_provider_eval.json");
    match serde_json::to_string_pretty(&artifact) {
        Ok(json) => {
            if let Err(e) = std::fs::write(&artifact_path, json) {
                eprintln!(
                    "[kg_provider_eval] failed to write calibration artifact: {}",
                    e
                );
            } else {
                println!("\n[calibration] wrote artifact to {:?}", artifact_path);
            }
        }
        Err(e) => {
            eprintln!(
                "[kg_provider_eval] failed to serialize calibration artifact: {}",
                e
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Compute percentile from a sorted list of values.
fn percentile(sorted: &[u64], pct: usize) -> u64 {
    if sorted.is_empty() {
        return 0;
    }
    let idx = (pct as f64 / 100.0 * (sorted.len() as f64 - 1.0)).round() as usize;
    sorted[idx.min(sorted.len() - 1)]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_percentile() {
        assert_eq!(percentile(&[100, 200, 300, 400, 500], 50), 300);
        assert_eq!(percentile(&[100, 200, 300, 400, 500], 95), 500);
        assert_eq!(percentile(&[], 50), 0);
        assert_eq!(percentile(&[42], 50), 42);
    }
}
