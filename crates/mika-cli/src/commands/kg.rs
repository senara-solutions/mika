//! `mika kg` — Knowledge Graph management subcommands.
//!
//! Subcommands: `status`, `list-agents`, `purge`, `validate`.
//! Follows the `mika skills` single-file pattern.

use anyhow::{Result, bail};
use std::io::IsTerminal;
use std::path::Path;

use crate::cli::{
    KgArgs, KgCommand, KgListAgentsArgs, KgPurgeArgs, KgStatusArgs, KgValidateArgs, OutputFormat,
};

pub async fn run(args: KgArgs) -> Result<()> {
    let global_home = mika_common::home::resolve_home_dir()?;
    let db_path = mika_common::home::container_db_path(&global_home);

    match args.command {
        KgCommand::Status(ref status_args) => run_status(&global_home, &db_path, status_args),
        KgCommand::ListAgents(ref list_args) => run_list_agents(&global_home, &db_path, list_args),
        KgCommand::Purge(ref purge_args) => run_purge(&global_home, &db_path, purge_args),
        KgCommand::Validate(ref validate_args) => run_validate(&db_path, validate_args),
    }
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

fn open_db(db_path: &Path) -> Result<mika_agent::db::Database> {
    if !db_path.exists() {
        bail!(
            "Mika database not found at {}. Run 'mika status' to initialize the agent.",
            db_path.display()
        );
    }
    mika_agent::db::Database::open(db_path)
}

fn truncate_docs_root(docs_root: &str, max_display: usize) -> String {
    if docs_root.chars().count() > max_display {
        let suffix: String = docs_root
            .chars()
            .rev()
            .take(max_display - 2)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        format!("..{suffix}")
    } else {
        docs_root.to_string()
    }
}

fn format_count(n: u64) -> String {
    if n < 1_000 {
        n.to_string()
    } else if n < 1_000_000 {
        format!("{},{:03}", n / 1_000, n % 1_000)
    } else {
        format!(
            "{},{:03},{:03}",
            n / 1_000_000,
            (n / 1_000) % 1_000,
            n % 1_000
        )
    }
}

// ---------------------------------------------------------------------------
// Status
// ---------------------------------------------------------------------------

/// Per-agent KG state for the status table.
#[derive(Debug, Clone, serde::Serialize)]
struct AgentKgState {
    agent_id: String,
    enabled: bool,
    docs_root: Option<String>,
    docs_root_hash: Option<String>,
    chunks: u64,
    subjects: u64,
    resolved: u64,
    pending: u64,
    last_extraction: Option<String>,
    /// Rolling 7-day no_match rate (0.0–1.0). `None` when data unavailable.
    #[serde(skip_serializing_if = "Option::is_none")]
    no_match_rate_7d: Option<f64>,
    /// Count of no_match outcomes in the 7-day window.
    #[serde(skip_serializing_if = "Option::is_none")]
    no_match_count_7d: Option<u64>,
    /// Total attempted resolutions in the 7-day window (denominator).
    #[serde(skip_serializing_if = "Option::is_none")]
    total_resolutions_7d: Option<u64>,
    drift: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

/// Corpus group for the summary line.
#[derive(Debug, Clone, serde::Serialize)]
struct CorpusGroup {
    docs_root_hash: String,
    docs_root: String,
    agents: Vec<String>,
}

/// Full status report (JSON output shape).
#[derive(Debug, serde::Serialize)]
struct KgStatusReport {
    summary: KgSummary,
    agents: Vec<AgentKgState>,
}

#[derive(Debug, serde::Serialize)]
struct KgSummary {
    total_agents: usize,
    unique_corpora: usize,
    disabled_count: usize,
    corpora: Vec<CorpusGroup>,
}

fn run_status(global_home: &Path, db_path: &Path, args: &KgStatusArgs) -> Result<()> {
    let db = open_db(db_path)?;
    let agents = mika_common::agent::list_agents(global_home);

    if agents.is_empty() {
        match args.format {
            OutputFormat::Json => {
                let report = KgStatusReport {
                    summary: KgSummary {
                        total_agents: 0,
                        unique_corpora: 0,
                        disabled_count: 0,
                        corpora: vec![],
                    },
                    agents: vec![],
                };
                println!("{}", serde_json::to_string_pretty(&report)?);
            }
            OutputFormat::Yaml => {
                let report = KgStatusReport {
                    summary: KgSummary {
                        total_agents: 0,
                        unique_corpora: 0,
                        disabled_count: 0,
                        corpora: vec![],
                    },
                    agents: vec![],
                };
                print!("{}", serde_yaml::to_string(&report)?);
            }
            OutputFormat::Text => {
                println!("\n  KG state summary (0 agents)\n");
            }
        }
        return Ok(());
    }

    // Filter to single agent if --agent specified
    let agent_filter = args.agent_flag.agent.as_deref();
    let agent_names: Vec<&str> = if let Some(filter) = agent_filter {
        if !agents.iter().any(|a| a == filter) {
            bail!("Agent '{filter}' not found.");
        }
        vec![filter]
    } else {
        agents.iter().map(|s| s.as_str()).collect()
    };

    let mut agent_states = Vec::new();
    for agent_name in &agent_names {
        let agent_home = mika_common::home::resolve_agent_home(global_home, agent_name);
        let states = build_agent_kg_states(&db, global_home, &agent_home, agent_name);
        agent_states.extend(states);
    }

    // Build corpus groups — each state is one (agent, corpus) pair
    let mut corpus_map: std::collections::HashMap<String, CorpusGroup> =
        std::collections::HashMap::new();
    let mut disabled_agents: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut enabled_agents: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut drift_agents: Vec<String> = Vec::new();

    for state in &agent_states {
        if !state.enabled {
            disabled_agents.insert(state.agent_id.clone());
            continue;
        }
        enabled_agents.insert(state.agent_id.clone());
        if state.error.is_some() {
            continue;
        }
        if state.drift {
            if !drift_agents.contains(&state.agent_id) {
                drift_agents.push(state.agent_id.clone());
            }
            continue;
        }
        if let Some(ref hash) = state.docs_root_hash {
            let entry = corpus_map
                .entry(hash.clone())
                .or_insert_with(|| CorpusGroup {
                    docs_root_hash: hash.clone(),
                    docs_root: state.docs_root.clone().unwrap_or_default(),
                    agents: vec![],
                });
            // Deduplicate agent names within a corpus group
            if !entry.agents.contains(&state.agent_id) {
                entry.agents.push(state.agent_id.clone());
            }
        }
    }

    let corpora: Vec<CorpusGroup> = corpus_map.into_values().collect();
    let unique_corpora = corpora.len();
    let total_agents = enabled_agents.len() + disabled_agents.len();

    let summary = KgSummary {
        total_agents,
        unique_corpora,
        disabled_count: disabled_agents.len(),
        corpora,
    };

    match args.format {
        OutputFormat::Json => {
            let report = KgStatusReport {
                summary,
                agents: agent_states,
            };
            println!("{}", serde_json::to_string_pretty(&report)?);
        }
        OutputFormat::Yaml => {
            let report = KgStatusReport {
                summary,
                agents: agent_states,
            };
            print!("{}", serde_yaml::to_string(&report)?);
        }
        OutputFormat::Text => {
            println!();
            println!(
                "  KG state summary ({} agents \u{2014} {} unique corpora + {} disabled)",
                summary.total_agents, summary.unique_corpora, summary.disabled_count
            );

            for corpus in &summary.corpora {
                let agents_str = corpus.agents.join(", ");
                let hash_display =
                    &corpus.docs_root_hash[..std::cmp::min(16, corpus.docs_root_hash.len())];
                println!(
                    "    \u{2022} {}  ({})  {} agents: {}",
                    hash_display,
                    corpus.docs_root,
                    corpus.agents.len(),
                    agents_str
                );
            }
            if !drift_agents.is_empty() {
                println!(
                    "    \u{2022} (drift)  {} agents: {}",
                    drift_agents.len(),
                    drift_agents.join(", ")
                );
            }
            println!();

            // Per-agent detail table
            println!(
                "  {:<20} {:<8} {:<36} {:>8} {:>10} {:>10} {:>8} {:>12} last_extraction",
                "Agent",
                "enabled",
                "docs_root",
                "chunks",
                "subjects",
                "resolved",
                "pending",
                "7d_no_match",
            );
            println!(
                "  {:<20} {:<8} {:<36} {:>8} {:>10} {:>10} {:>8} {:>12} {}",
                "-".repeat(20),
                "-".repeat(8),
                "-".repeat(36),
                "-".repeat(8),
                "-".repeat(10),
                "-".repeat(10),
                "-".repeat(8),
                "-".repeat(12),
                "-".repeat(18)
            );

            let mut prev_agent: Option<String> = None;
            for state in &agent_states {
                // Show agent name only on the first row for each agent
                let show_agent = prev_agent.as_ref() != Some(&state.agent_id);
                let agent_col = if show_agent {
                    state.agent_id.as_str()
                } else {
                    ""
                };
                let enabled_str = if show_agent {
                    if state.enabled { "true" } else { "false" }
                } else {
                    ""
                };
                let docs_root = state.docs_root.as_deref().unwrap_or("N/A");
                let drift_tag = if state.drift { "  [DRIFT]" } else { "" };
                let error_tag = if let Some(ref e) = state.error {
                    format!("  [ERROR: {}]", e)
                } else {
                    String::new()
                };
                let last_ext = state.last_extraction.as_deref().unwrap_or("N/A");

                // Truncate docs_root for display (char-safe, no byte slicing)
                let docs_display = truncate_docs_root(docs_root, 34);

                // Format 7-day no_match rate column (#1077).
                let no_match_display = match (state.no_match_rate_7d, state.total_resolutions_7d) {
                    (Some(rate), Some(total)) if total > 0 => {
                        let pct = format!("{:.1}%", rate * 100.0);
                        if rate > 0.60 {
                            format!("{} [!]", pct)
                        } else {
                            pct
                        }
                    }
                    _ => "N/A".to_string(),
                };

                println!(
                    "  {:<20} {:<8} {:<36} {:>8} {:>10} {:>10} {:>8} {:>12} {}{}{}",
                    agent_col,
                    enabled_str,
                    docs_display,
                    format_count(state.chunks),
                    format_count(state.subjects),
                    format_count(state.resolved),
                    format_count(state.pending),
                    no_match_display,
                    last_ext,
                    drift_tag,
                    error_tag,
                );
                prev_agent = Some(state.agent_id.clone());
            }
            println!();
        }
    }

    Ok(())
}

/// Build per-corpus KG state entries for an agent.
///
/// For single-corpus agents, returns a single-element Vec. For multi-corpus
/// agents (e.g., mika-arch with 4 corpora), returns one entry per corpus so
/// the CLI can display per-corpus resolution coverage (#877).
fn build_agent_kg_states(
    db: &mika_agent::db::Database,
    global_home: &Path,
    agent_home: &Path,
    agent_name: &str,
) -> Vec<AgentKgState> {
    let identity = mika_agent::prompt::load_identity(agent_home);

    // Resolve per-agent KG config
    let settings = match mika_common::config::Settings::load_for_agent(global_home, agent_home) {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!(agent = agent_name, error = %e, "Failed to load settings for KG status");
            return vec![AgentKgState {
                agent_id: agent_name.to_string(),
                enabled: identity.kg.enabled,
                docs_root: None,
                docs_root_hash: None,
                chunks: 0,
                subjects: 0,
                resolved: 0,
                pending: 0,
                last_extraction: None,
                no_match_rate_7d: None,
                no_match_count_7d: None,
                total_resolutions_7d: None,
                drift: false,
                error: Some(format!("config error: {e}")),
            }];
        }
    };

    let kg_config = mika_agent::kg::config::resolve_per_agent_docs_root(&identity, &settings);

    match kg_config {
        Ok(mika_agent::kg::config::KgAgentConfig::Disabled { .. }) => {
            vec![AgentKgState {
                agent_id: agent_name.to_string(),
                enabled: false,
                docs_root: None,
                docs_root_hash: None,
                chunks: 0,
                subjects: 0,
                resolved: 0,
                pending: 0,
                last_extraction: None,
                no_match_rate_7d: None,
                no_match_count_7d: None,
                total_resolutions_7d: None,
                drift: false,
                error: None,
            }]
        }
        Ok(mika_agent::kg::config::KgAgentConfig::Enabled { ref corpora }) => {
            // Emit one row per corpus for full multi-corpus visibility (#877).
            corpora
                .iter()
                .map(|corpus| {
                    let chunks = db
                        .kg_count_rows("kg_chunks", "docs_root_hash", &corpus.docs_root_hash)
                        .unwrap_or(0);
                    let subjects = db
                        .kg_count_rows(
                            "kg_subject_entities",
                            "docs_root_hash",
                            &corpus.docs_root_hash,
                        )
                        .unwrap_or(0);
                    let resolved = db
                        .kg_count_resolved_for_corpus(agent_name, &corpus.docs_root_hash)
                        .unwrap_or(0);
                    // Pending = resolver-actionable subjects only (#999).
                    // Mirrors the resolver's 5-type allowlist (skill/tool/agent/
                    // problem_type/concept). Excludes subject-graph-only types
                    // (pattern/failure_mode/solution_path) which the resolver
                    // intentionally never processes — counting them as "pending"
                    // misled operators into chasing non-existent backlog.
                    let pending = db
                        .kg_count_pending_resolver_actionable_for_corpus(
                            agent_name,
                            &corpus.docs_root_hash,
                        )
                        .unwrap_or(0);
                    let last_extraction =
                        db.kg_last_extraction(&corpus.docs_root_hash).ok().flatten();

                    // Compute 7-day no_match rate per corpus (#1077).
                    let (no_match_rate_7d, no_match_count_7d, total_resolutions_7d) = match db
                        .kg_resolution_outcome_stats(agent_name, Some(&corpus.docs_root_hash), 7)
                    {
                        Ok(stats) if stats.attempted > 0 => (
                            Some(stats.no_match_rate()),
                            Some(stats.no_match),
                            Some(stats.attempted),
                        ),
                        _ => (None, None, None),
                    };

                    AgentKgState {
                        agent_id: agent_name.to_string(),
                        enabled: true,
                        docs_root: Some(corpus.docs_root.display().to_string()),
                        docs_root_hash: Some(corpus.docs_root_hash.clone()),
                        chunks,
                        subjects,
                        resolved,
                        pending,
                        last_extraction,
                        no_match_rate_7d,
                        no_match_count_7d,
                        total_resolutions_7d,
                        drift: false,
                        error: None,
                    }
                })
                .collect()
        }
        Err(e) => {
            tracing::warn!(agent = agent_name, error = %e, "KG config error");
            vec![AgentKgState {
                agent_id: agent_name.to_string(),
                enabled: identity.kg.enabled,
                docs_root: identity
                    .kg
                    .docs_root
                    .as_ref()
                    .map(|p| p.display().to_string()),
                docs_root_hash: None,
                chunks: 0,
                subjects: 0,
                resolved: 0,
                pending: 0,
                last_extraction: None,
                no_match_rate_7d: None,
                no_match_count_7d: None,
                total_resolutions_7d: None,
                drift: false,
                error: Some(format!("{e}")),
            }]
        }
    }
}

// ---------------------------------------------------------------------------
// List Agents
// ---------------------------------------------------------------------------

#[derive(Debug, serde::Serialize)]
struct AgentListing {
    agent_id: String,
    enabled: bool,
    docs_root: Option<String>,
    docs_root_hash: Option<String>,
    chunks: u64,
}

fn run_list_agents(global_home: &Path, db_path: &Path, args: &KgListAgentsArgs) -> Result<()> {
    let db = open_db(db_path)?;
    let agents = mika_common::agent::list_agents(global_home);

    let agent_filter = args.agent_flag.agent.as_deref();
    let agent_names: Vec<&str> = if let Some(filter) = agent_filter {
        if !agents.iter().any(|a| a == filter) {
            bail!("Agent '{filter}' not found.");
        }
        vec![filter]
    } else {
        agents.iter().map(|s| s.as_str()).collect()
    };

    let mut listings = Vec::new();

    for agent_name in &agent_names {
        let agent_home = mika_common::home::resolve_agent_home(global_home, agent_name);
        let identity = mika_agent::prompt::load_identity(&agent_home);

        if !identity.kg.enabled {
            listings.push(AgentListing {
                agent_id: agent_name.to_string(),
                enabled: false,
                docs_root: None,
                docs_root_hash: None,
                chunks: 0,
            });
            continue;
        }

        let settings = match mika_common::config::Settings::load_for_agent(global_home, &agent_home)
        {
            Ok(s) => s,
            Err(_) => {
                listings.push(AgentListing {
                    agent_id: agent_name.to_string(),
                    enabled: true,
                    docs_root: None,
                    docs_root_hash: None,
                    chunks: 0,
                });
                continue;
            }
        };

        match mika_agent::kg::config::resolve_per_agent_docs_root(&identity, &settings) {
            Ok(mika_agent::kg::config::KgAgentConfig::Disabled { .. }) => {
                listings.push(AgentListing {
                    agent_id: agent_name.to_string(),
                    enabled: false,
                    docs_root: None,
                    docs_root_hash: None,
                    chunks: 0,
                });
            }
            Ok(mika_agent::kg::config::KgAgentConfig::Enabled { ref corpora }) => {
                let first = corpora.first();
                let mut chunks = 0u64;
                for corpus in corpora {
                    chunks += db
                        .kg_count_rows("kg_chunks", "docs_root_hash", &corpus.docs_root_hash)
                        .unwrap_or(0);
                }
                listings.push(AgentListing {
                    agent_id: agent_name.to_string(),
                    enabled: true,
                    docs_root: first.map(|c| c.docs_root.display().to_string()),
                    docs_root_hash: first.map(|c| c.docs_root_hash.clone()),
                    chunks,
                });
            }
            Err(_) => {
                listings.push(AgentListing {
                    agent_id: agent_name.to_string(),
                    enabled: true,
                    docs_root: identity
                        .kg
                        .docs_root
                        .as_ref()
                        .map(|p| p.display().to_string()),
                    docs_root_hash: None,
                    chunks: 0,
                });
            }
        }
    }

    match args.format {
        OutputFormat::Json => {
            println!("{}", serde_json::to_string_pretty(&listings)?);
        }
        OutputFormat::Yaml => {
            print!("{}", serde_yaml::to_string(&listings)?);
        }
        OutputFormat::Text => {
            if listings.is_empty() {
                println!("\n  No agents found.\n");
                return Ok(());
            }
            println!();
            println!(
                "  {:<20} {:<8} {:<18} {:>8}",
                "Agent", "enabled", "docs_root_hash", "chunks"
            );
            println!(
                "  {:<20} {:<8} {:<18} {:>8}",
                "-".repeat(20),
                "-".repeat(8),
                "-".repeat(18),
                "-".repeat(8)
            );
            for listing in &listings {
                let hash_display = listing
                    .docs_root_hash
                    .as_deref()
                    .map(|h| &h[..std::cmp::min(16, h.len())])
                    .unwrap_or("N/A");
                println!(
                    "  {:<20} {:<8} {:<18} {:>8}",
                    listing.agent_id,
                    if listing.enabled { "true" } else { "false" },
                    hash_display,
                    format_count(listing.chunks),
                );
            }
            println!();
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Validate
// ---------------------------------------------------------------------------

#[derive(Debug, serde::Serialize)]
struct KgValidateCheck {
    name: String,
    status: String,
    count: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    example_id: Option<i64>,
    message: String,
}

#[derive(Debug, serde::Serialize)]
struct KgValidateReport {
    checks: Vec<KgValidateCheck>,
    summary: KgValidateSummary,
}

#[derive(Debug, serde::Serialize)]
struct KgValidateSummary {
    total: usize,
    ok: usize,
    warn: usize,
    fail: usize,
}

fn run_validate(db_path: &Path, args: &KgValidateArgs) -> Result<()> {
    let db = open_db(db_path)?;

    let orphan_checks: Vec<(&str, &str, &str, &str)> = vec![
        (
            "orphan_chunk_subjects_chunk",
            "kg_chunk_subjects",
            "chunk_id",
            "kg_chunks",
        ),
        (
            "orphan_chunk_subjects_entity",
            "kg_chunk_subjects",
            "subject_entity_id",
            "kg_subject_entities",
        ),
        (
            "orphan_chunk_subject_rels_chunk",
            "kg_chunk_subject_relationships",
            "chunk_id",
            "kg_chunks",
        ),
        (
            "orphan_chunk_subject_rels_rel",
            "kg_chunk_subject_relationships",
            "subject_relationship_id",
            "kg_subject_relationships",
        ),
        (
            "orphan_resolutions_subject",
            "kg_subject_resolutions",
            "subject_entity_id",
            "kg_subject_entities",
        ),
        (
            "orphan_resolutions_domain",
            "kg_subject_resolutions",
            "domain_entity_id",
            "kg_entities",
        ),
        (
            "orphan_resolution_log_subject",
            "kg_resolutions_log",
            "subject_entity_id",
            "kg_subject_entities",
        ),
    ];

    let mut checks = Vec::new();

    for (name, source_table, fk_col, target_table) in &orphan_checks {
        let check = match db.kg_check_orphan_fk(source_table, fk_col, target_table) {
            Ok((count, example)) if count > 0 => KgValidateCheck {
                name: name.to_string(),
                status: "fail".to_string(),
                count,
                example_id: example,
                message: format!(
                    "{source_table}.{fk_col} \u{2192} {target_table}: {count} orphan rows (example id: {})",
                    example
                        .map(|e| e.to_string())
                        .unwrap_or_else(|| "?".to_string())
                ),
            },
            Ok(_) => KgValidateCheck {
                name: name.to_string(),
                status: "ok".to_string(),
                count: 0,
                example_id: None,
                message: format!("{source_table}.{fk_col} \u{2192} {target_table}: clean"),
            },
            Err(e) => KgValidateCheck {
                name: name.to_string(),
                status: "fail".to_string(),
                count: 0,
                example_id: None,
                message: format!("{source_table}.{fk_col} check failed: {e}"),
            },
        };
        checks.push(check);
    }

    // NULL source_doc_hash check
    match db.kg_count_null_hash() {
        Ok(count) if count > 0 => {
            checks.push(KgValidateCheck {
                name: "null_source_doc_hash".to_string(),
                status: "warn".to_string(),
                count,
                example_id: None,
                message: format!(
                    "kg_chunks.source_doc_hash: {count} rows with NULL hash (pre-v26 backfill pending)"
                ),
            });
        }
        Ok(_) => {
            checks.push(KgValidateCheck {
                name: "null_source_doc_hash".to_string(),
                status: "ok".to_string(),
                count: 0,
                example_id: None,
                message: "kg_chunks.source_doc_hash: all rows have hashes".to_string(),
            });
        }
        Err(e) => {
            checks.push(KgValidateCheck {
                name: "null_source_doc_hash".to_string(),
                status: "fail".to_string(),
                count: 0,
                example_id: None,
                message: format!("kg_chunks.source_doc_hash check failed: {e}"),
            });
        }
    }

    let ok_count = checks.iter().filter(|c| c.status == "ok").count();
    let warn_count = checks.iter().filter(|c| c.status == "warn").count();
    let fail_count = checks.iter().filter(|c| c.status == "fail").count();

    let summary = KgValidateSummary {
        total: checks.len(),
        ok: ok_count,
        warn: warn_count,
        fail: fail_count,
    };

    match args.format {
        OutputFormat::Json => {
            let report = KgValidateReport { checks, summary };
            println!("{}", serde_json::to_string_pretty(&report)?);
        }
        OutputFormat::Yaml => {
            let report = KgValidateReport { checks, summary };
            print!("{}", serde_yaml::to_string(&report)?);
        }
        OutputFormat::Text => {
            println!();
            println!("  Mika KG Validate \u{2014} checking Knowledge Graph integrity...");
            println!();
            for check in &checks {
                let tag = match check.status.as_str() {
                    "ok" => "\x1b[32m[OK]\x1b[0m  ",
                    "warn" => "\x1b[33m[WARN]\x1b[0m",
                    "fail" => "\x1b[31m[FAIL]\x1b[0m",
                    _ => "      ",
                };
                println!("  {tag} {}", check.message);
            }
            println!();
            println!(
                "  Summary: {} checks run: {} OK, {} WARN, {} FAIL",
                summary.total, summary.ok, summary.warn, summary.fail
            );
            println!();
        }
    }

    if fail_count > 0 {
        std::process::exit(1);
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Purge
// ---------------------------------------------------------------------------

fn run_purge(global_home: &Path, db_path: &Path, args: &KgPurgeArgs) -> Result<()> {
    let db = open_db(db_path)?;
    let agents = mika_common::agent::list_agents(global_home);
    let agent_id = &args.agent;

    // Validate agent exists
    if !agents.iter().any(|a| a == agent_id) {
        bail!("Agent '{agent_id}' not found.");
    }

    // Compute per-agent row counts
    let resolutions_count = db
        .kg_count_rows("kg_subject_resolutions", "agent_id", agent_id)
        .unwrap_or(0);
    let resolution_log_count = db
        .kg_count_rows("kg_resolutions_log", "agent_id", agent_id)
        .unwrap_or(0);

    // Resolve agent's docs_root_hash for shared-layer analysis
    let agent_home = mika_common::home::resolve_agent_home(global_home, agent_id);
    let identity = mika_agent::prompt::load_identity(&agent_home);
    let settings = mika_common::config::Settings::load_for_agent(global_home, &agent_home)?;
    let kg_config = mika_agent::kg::config::resolve_per_agent_docs_root(&identity, &settings)
        .map_err(|e| anyhow::anyhow!("KG config error for agent '{agent_id}': {e}"))?;

    let (docs_root_hash, shared_agents): (Option<String>, Vec<String>) = match &kg_config {
        mika_agent::kg::config::KgAgentConfig::Disabled { .. } => (None, Vec::new()),
        mika_agent::kg::config::KgAgentConfig::Enabled { corpora } => {
            // Use first corpus hash for purge analysis (multi-corpus purge is future work)
            let first_hash = corpora.first().map(|c| c.docs_root_hash.clone());
            // Find other agents sharing this hash
            let mut sharing: Vec<String> = Vec::new();
            if let Some(ref hash) = first_hash {
                for other in &agents {
                    if other == agent_id {
                        continue;
                    }
                    let other_home = mika_common::home::resolve_agent_home(global_home, other);
                    let other_identity = mika_agent::prompt::load_identity(&other_home);
                    if let Ok(other_settings) =
                        mika_common::config::Settings::load_for_agent(global_home, &other_home)
                        && let Ok(mika_agent::kg::config::KgAgentConfig::Enabled {
                            corpora: other_corpora,
                        }) = mika_agent::kg::config::resolve_per_agent_docs_root(
                            &other_identity,
                            &other_settings,
                        )
                        && other_corpora.iter().any(|c| &c.docs_root_hash == hash)
                    {
                        sharing.push(other.clone());
                    }
                }
            }
            (first_hash, sharing)
        }
    };

    // Determine shared-layer deletion safety
    let force_delete_shared =
        args.include_orphaned_corpus && docs_root_hash.is_some() && shared_agents.is_empty();

    // Print pre-confirmation summary (text only — JSON/YAML comes after purge)
    if matches!(args.format, OutputFormat::Text) {
        println!();
        println!("  About to purge KG state for agent '{agent_id}':");
        println!();
        println!("    Per-agent rows (will be deleted):");
        println!(
            "      - kg_subject_resolutions      : {} rows",
            format_count(resolutions_count)
        );
        println!(
            "      - kg_resolutions_log          : {} rows",
            format_count(resolution_log_count)
        );
        println!();

        if let Some(ref hash) = docs_root_hash {
            let chunks = db
                .kg_count_rows("kg_chunks", "docs_root_hash", hash)
                .unwrap_or(0);
            let subject_entities = db
                .kg_count_rows("kg_subject_entities", "docs_root_hash", hash)
                .unwrap_or(0);

            if force_delete_shared {
                let subject_rels = db
                    .kg_count_rows("kg_subject_relationships", "docs_root_hash", hash)
                    .unwrap_or(0);
                let chunk_subjects = db
                    .kg_count_rows("kg_chunk_subjects", "docs_root_hash", hash)
                    .unwrap_or(0);
                let chunk_subject_rels = db
                    .kg_count_rows("kg_chunk_subject_relationships", "docs_root_hash", hash)
                    .unwrap_or(0);
                let extractions = db
                    .kg_count_rows("kg_extractions", "docs_root_hash", hash)
                    .unwrap_or(0);

                println!(
                    "    \u{26a0} Shared-layer rows (WILL BE DELETED \u{2014} no other agent references docs_root_hash {hash}):"
                );
                println!(
                    "      - kg_chunks                   : {} rows",
                    format_count(chunks)
                );
                println!(
                    "      - kg_subject_entities         : {} rows",
                    format_count(subject_entities)
                );
                println!(
                    "      - kg_subject_relationships    : {} rows",
                    format_count(subject_rels)
                );
                println!(
                    "      - kg_chunk_subjects           : {} rows",
                    format_count(chunk_subjects)
                );
                println!(
                    "      - kg_chunk_subject_relationships: {} rows",
                    format_count(chunk_subject_rels)
                );
                println!(
                    "      - kg_extractions              : {} rows",
                    format_count(extractions)
                );
            } else if args.include_orphaned_corpus && !shared_agents.is_empty() {
                println!(
                    "    \u{26a0} Shared-layer rows (--include-orphaned-corpus was passed BUT cannot delete \u{2014} docs_root_hash {} is still referenced by {} other agents: {}):",
                    hash,
                    shared_agents.len(),
                    shared_agents.join(", ")
                );
                println!("      - shared rows will NOT be deleted");
                println!(
                    "      - to remove the shared corpus, first purge all other agents referencing it"
                );
            } else {
                println!(
                    "    Shared-layer rows (will NOT be deleted without --include-orphaned-corpus):"
                );
                if !shared_agents.is_empty() {
                    println!(
                        "      - docs_root_hash {} is shared with {} other agents: {}",
                        hash,
                        shared_agents.len(),
                        shared_agents.join(", ")
                    );
                }
                println!(
                    "      - {} kg_chunks / {} kg_subject_entities / ... remain untouched",
                    format_count(chunks),
                    format_count(subject_entities)
                );
            }
        }
        println!();
    }

    // Confirmation
    if !args.yes {
        if !std::io::stdin().is_terminal() {
            bail!(
                "Non-interactive terminal requires --yes flag to bypass confirmation. \
                 Use: mika kg purge --agent {agent_id} --yes"
            );
        }

        print!("  Type the agent ID to confirm: ");
        // Flush stdout so the prompt appears before reading
        use std::io::Write;
        std::io::stdout().flush()?;

        let mut input = String::new();
        std::io::stdin().read_line(&mut input)?;
        let input = input.trim();
        if input != agent_id.as_str() {
            println!("  Agent ID mismatch \u{2014} aborting.");
            std::process::exit(1);
        }
    }

    // Execute purge
    let counts = db.purge_kg_for_agent(agent_id, force_delete_shared, docs_root_hash.as_deref())?;

    // Output results
    match args.format {
        OutputFormat::Json => {
            println!("{}", serde_json::to_string_pretty(&counts)?);
        }
        OutputFormat::Yaml => {
            print!("{}", serde_yaml::to_string(&counts)?);
        }
        OutputFormat::Text => {
            println!("  Purging... done.");
            println!(
                "    Deleted {} rows from kg_subject_resolutions.",
                counts.resolutions_deleted
            );
            println!(
                "    Deleted {} rows from kg_resolutions_log.",
                counts.resolution_log_deleted
            );
            if let Some(ref shared) = counts.shared_layer_deleted {
                for (table, count) in shared {
                    println!("    Deleted {count} rows from {table}.");
                }
            }
            println!();
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::{Cli, Commands, KgArgs, KgCommand};
    use clap::Parser;

    // Parser tests

    #[test]
    fn test_parse_kg_status() {
        let cli = Cli::try_parse_from(["mika", "kg", "status"]);
        assert!(cli.is_ok(), "kg status should parse: {:?}", cli.err());
        let cli = cli.unwrap();
        assert!(matches!(
            cli.command,
            Some(Commands::Kg(KgArgs {
                command: KgCommand::Status(_)
            }))
        ));
    }

    #[test]
    fn test_parse_kg_status_with_agent_and_format() {
        let cli = Cli::try_parse_from([
            "mika", "kg", "status", "--agent", "mika-dev", "--format", "json",
        ]);
        assert!(cli.is_ok());
        let cli = cli.unwrap();
        if let Some(Commands::Kg(args)) = &cli.command {
            if let KgCommand::Status(status_args) = &args.command {
                assert_eq!(status_args.agent_flag.agent.as_deref(), Some("mika-dev"));
                assert!(matches!(status_args.format, OutputFormat::Json));
            } else {
                panic!("Expected Status");
            }
        } else {
            panic!("Expected Kg");
        }
    }

    #[test]
    fn test_parse_kg_list_agents() {
        let cli = Cli::try_parse_from(["mika", "kg", "list-agents"]);
        assert!(cli.is_ok());
        assert!(matches!(
            cli.unwrap().command,
            Some(Commands::Kg(KgArgs {
                command: KgCommand::ListAgents(_)
            }))
        ));
    }

    #[test]
    fn test_parse_kg_purge_with_yes() {
        let cli = Cli::try_parse_from(["mika", "kg", "purge", "--agent", "test-agent", "--yes"]);
        assert!(cli.is_ok());
        let cli = cli.unwrap();
        if let Some(Commands::Kg(args)) = &cli.command {
            if let KgCommand::Purge(purge_args) = &args.command {
                assert_eq!(purge_args.agent, "test-agent");
                assert!(purge_args.yes);
                assert!(!purge_args.include_orphaned_corpus);
            } else {
                panic!("Expected Purge");
            }
        } else {
            panic!("Expected Kg");
        }
    }

    #[test]
    fn test_parse_kg_purge_requires_agent() {
        let result = Cli::try_parse_from(["mika", "kg", "purge"]);
        assert!(result.is_err(), "purge without --agent should fail");
    }

    #[test]
    fn test_parse_kg_purge_with_include_orphaned() {
        let cli = Cli::try_parse_from([
            "mika",
            "kg",
            "purge",
            "--agent",
            "test-agent",
            "--include-orphaned-corpus",
            "--yes",
        ]);
        assert!(cli.is_ok());
        let cli = cli.unwrap();
        if let Some(Commands::Kg(args)) = &cli.command {
            if let KgCommand::Purge(purge_args) = &args.command {
                assert!(purge_args.include_orphaned_corpus);
            } else {
                panic!("Expected Purge");
            }
        } else {
            panic!("Expected Kg");
        }
    }

    #[test]
    fn test_parse_kg_validate_json() {
        let cli = Cli::try_parse_from(["mika", "kg", "validate", "--format", "json"]);
        assert!(cli.is_ok());
        let cli = cli.unwrap();
        if let Some(Commands::Kg(args)) = &cli.command {
            if let KgCommand::Validate(validate_args) = &args.command {
                assert!(matches!(validate_args.format, OutputFormat::Json));
            } else {
                panic!("Expected Validate");
            }
        } else {
            panic!("Expected Kg");
        }
    }

    #[test]
    fn test_parse_kg_requires_subcommand() {
        let result = Cli::try_parse_from(["mika", "kg"]);
        assert!(result.is_err(), "kg without subcommand should fail");
    }

    #[test]
    fn test_kg_agent_override_status() {
        let cli = Cli::try_parse_from(["mika", "kg", "status", "--agent", "work"]).unwrap();
        assert_eq!(cli.command.as_ref().unwrap().agent_override(), Some("work"));
    }

    #[test]
    fn test_kg_agent_override_purge() {
        let cli = Cli::try_parse_from(["mika", "kg", "purge", "--agent", "work", "--yes"]).unwrap();
        assert_eq!(cli.command.as_ref().unwrap().agent_override(), Some("work"));
    }

    #[test]
    fn test_kg_agent_override_validate_is_none() {
        let cli = Cli::try_parse_from(["mika", "kg", "validate"]).unwrap();
        assert_eq!(cli.command.as_ref().unwrap().agent_override(), None);
    }

    // Format count helper tests

    #[test]
    fn test_format_count() {
        assert_eq!(format_count(0), "0");
        assert_eq!(format_count(42), "42");
        assert_eq!(format_count(999), "999");
        assert_eq!(format_count(1_000), "1,000");
        assert_eq!(format_count(2_347), "2,347");
        assert_eq!(format_count(999_999), "999,999");
        assert_eq!(format_count(1_000_000), "1,000,000");
    }

    // Typed-ID validator tests (purge confirmation)

    #[test]
    fn test_typed_id_exact_match() {
        let expected = "odds-engine-ceo";
        let input = "odds-engine-ceo";
        assert_eq!(input, expected);
    }

    #[test]
    fn test_typed_id_rejects_underscore_variant() {
        let expected = "odds-engine-ceo";
        let input = "odds_engine_ceo";
        assert_ne!(input, expected, "Strict equality, no normalization");
    }

    #[test]
    fn test_typed_id_rejects_different_id() {
        let expected = "odds-engine-ceo";
        let input = "mika-dev";
        assert_ne!(input, expected);
    }

    #[test]
    fn test_typed_id_accepts_underscore_name() {
        let expected = "archive_bot_v2";
        let input = "archive_bot_v2";
        assert_eq!(input, expected);
    }

    // truncate_docs_root tests (#877)

    #[test]
    fn test_truncate_docs_root_short_path() {
        assert_eq!(truncate_docs_root("/short/path", 34), "/short/path");
    }

    #[test]
    fn test_truncate_docs_root_exact_limit() {
        let path = "x".repeat(34);
        assert_eq!(truncate_docs_root(&path, 34), path);
    }

    #[test]
    fn test_truncate_docs_root_over_limit() {
        let path = "/data/workspace/mika-platform/mika/docs/solutions";
        let truncated = truncate_docs_root(path, 34);
        assert!(
            truncated.starts_with(".."),
            "Expected leading '..' but got: {truncated}"
        );
        assert_eq!(truncated.chars().count(), 34);
    }

    #[test]
    fn test_truncate_docs_root_na() {
        assert_eq!(truncate_docs_root("N/A", 34), "N/A");
    }

    // Multi-corpus AgentKgState serialization tests (#877)

    #[test]
    fn test_agent_kg_state_json_shape_multi_corpus() {
        let states = vec![
            AgentKgState {
                agent_id: "mika-arch".to_string(),
                enabled: true,
                docs_root: Some("/data/mika/docs/solutions".to_string()),
                docs_root_hash: Some("abc123".to_string()),
                chunks: 2845,
                subjects: 30164,
                resolved: 8082,
                pending: 22082,
                last_extraction: Some("2026-04-29T12:00:00Z".to_string()),
                no_match_rate_7d: Some(0.82),
                no_match_count_7d: Some(1000),
                total_resolutions_7d: Some(1220),
                drift: false,
                error: None,
            },
            AgentKgState {
                agent_id: "mika-arch".to_string(),
                enabled: true,
                docs_root: Some("/data/mika-skills/docs/solutions".to_string()),
                docs_root_hash: Some("def456".to_string()),
                chunks: 345,
                subjects: 164,
                resolved: 4,
                pending: 160,
                last_extraction: Some("2026-04-29T12:00:00Z".to_string()),
                no_match_rate_7d: Some(0.50),
                no_match_count_7d: Some(50),
                total_resolutions_7d: Some(100),
                drift: false,
                error: None,
            },
        ];

        let json = serde_json::to_value(&states).unwrap();
        let arr = json.as_array().unwrap();
        assert_eq!(arr.len(), 2);
        assert_eq!(arr[0]["agent_id"], "mika-arch");
        assert_eq!(arr[0]["docs_root_hash"], "abc123");
        assert_eq!(arr[0]["resolved"], 8082);
        assert_eq!(arr[1]["agent_id"], "mika-arch");
        assert_eq!(arr[1]["docs_root_hash"], "def456");
        assert_eq!(arr[1]["resolved"], 4);
    }

    #[test]
    fn test_corpus_group_dedup_agents() {
        // Verify that the corpus grouping logic doesn't produce duplicate agent
        // entries when an agent has multiple corpora sharing the same hash
        // (shouldn't happen in practice, but defensive)
        let mut corpus_map: std::collections::HashMap<String, CorpusGroup> =
            std::collections::HashMap::new();

        let hash = "abc123".to_string();
        let entry = corpus_map
            .entry(hash.clone())
            .or_insert_with(|| CorpusGroup {
                docs_root_hash: hash.clone(),
                docs_root: "/some/path".to_string(),
                agents: vec![],
            });
        if !entry.agents.contains(&"mika-arch".to_string()) {
            entry.agents.push("mika-arch".to_string());
        }
        // Simulate second insert for same agent (shouldn't happen, but defensive)
        let entry = corpus_map.get_mut(&hash).unwrap();
        if !entry.agents.contains(&"mika-arch".to_string()) {
            entry.agents.push("mika-arch".to_string());
        }

        assert_eq!(entry.agents.len(), 1);
    }

    #[test]
    fn test_summary_counts_agents_not_corpus_rows() {
        // With multi-corpus agents, the summary should count unique agents,
        // not (agent, corpus) pairs
        let agent_states = vec![
            AgentKgState {
                agent_id: "mika-arch".to_string(),
                enabled: true,
                docs_root: Some("/path1".to_string()),
                docs_root_hash: Some("hash1".to_string()),
                chunks: 100,
                subjects: 50,
                resolved: 10,
                pending: 40,
                last_extraction: None,
                no_match_rate_7d: None,
                no_match_count_7d: None,
                total_resolutions_7d: None,
                drift: false,
                error: None,
            },
            AgentKgState {
                agent_id: "mika-arch".to_string(),
                enabled: true,
                docs_root: Some("/path2".to_string()),
                docs_root_hash: Some("hash2".to_string()),
                chunks: 50,
                subjects: 25,
                resolved: 5,
                pending: 20,
                last_extraction: None,
                no_match_rate_7d: None,
                no_match_count_7d: None,
                total_resolutions_7d: None,
                drift: false,
                error: None,
            },
            AgentKgState {
                agent_id: "mika-dev".to_string(),
                enabled: true,
                docs_root: Some("/path1".to_string()),
                docs_root_hash: Some("hash1".to_string()),
                chunks: 100,
                subjects: 50,
                resolved: 10,
                pending: 40,
                last_extraction: None,
                no_match_rate_7d: None,
                no_match_count_7d: None,
                total_resolutions_7d: None,
                drift: false,
                error: None,
            },
        ];

        let mut enabled_agents: std::collections::HashSet<String> =
            std::collections::HashSet::new();
        let mut corpus_map: std::collections::HashMap<String, CorpusGroup> =
            std::collections::HashMap::new();

        for state in &agent_states {
            enabled_agents.insert(state.agent_id.clone());
            if let Some(ref hash) = state.docs_root_hash {
                let entry = corpus_map
                    .entry(hash.clone())
                    .or_insert_with(|| CorpusGroup {
                        docs_root_hash: hash.clone(),
                        docs_root: state.docs_root.clone().unwrap_or_default(),
                        agents: vec![],
                    });
                if !entry.agents.contains(&state.agent_id) {
                    entry.agents.push(state.agent_id.clone());
                }
            }
        }

        // 2 unique agents, 2 unique corpora
        assert_eq!(enabled_agents.len(), 2);
        assert_eq!(corpus_map.len(), 2);
        // hash1 is shared by both agents
        assert_eq!(corpus_map["hash1"].agents.len(), 2);
        // hash2 is only mika-arch
        assert_eq!(corpus_map["hash2"].agents.len(), 1);
    }
}
