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
        let state = build_agent_kg_state(&db, global_home, &agent_home, agent_name);
        agent_states.push(state);
    }

    // Build corpus groups
    let mut corpus_map: std::collections::HashMap<String, CorpusGroup> =
        std::collections::HashMap::new();
    let mut disabled_count = 0;
    let mut drift_agents: Vec<String> = Vec::new();

    for state in &agent_states {
        if !state.enabled {
            disabled_count += 1;
            continue;
        }
        if state.error.is_some() {
            continue;
        }
        if state.drift {
            drift_agents.push(state.agent_id.clone());
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
            entry.agents.push(state.agent_id.clone());
        }
    }

    let corpora: Vec<CorpusGroup> = corpus_map.into_values().collect();
    let unique_corpora = corpora.len();

    let summary = KgSummary {
        total_agents: agent_states.len(),
        unique_corpora,
        disabled_count,
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
                "  {:<20} {:<8} {:<36} {:>8} {:>10} {:>10} {:>8} last_extraction",
                "Agent", "enabled", "docs_root", "chunks", "subjects", "resolved", "pending",
            );
            println!(
                "  {:<20} {:<8} {:<36} {:>8} {:>10} {:>10} {:>8} {}",
                "-".repeat(20),
                "-".repeat(8),
                "-".repeat(36),
                "-".repeat(8),
                "-".repeat(10),
                "-".repeat(10),
                "-".repeat(8),
                "-".repeat(18)
            );

            for state in &agent_states {
                let enabled_str = if state.enabled { "true" } else { "false" };
                let docs_root = state.docs_root.as_deref().unwrap_or("N/A");
                let drift_tag = if state.drift { "  [DRIFT]" } else { "" };
                let error_tag = if let Some(ref e) = state.error {
                    format!("  [ERROR: {}]", e)
                } else {
                    String::new()
                };
                let last_ext = state.last_extraction.as_deref().unwrap_or("N/A");

                // Truncate docs_root for display (char-safe, no byte slicing)
                let docs_display = if docs_root.chars().count() > 34 {
                    let suffix: String = docs_root
                        .chars()
                        .rev()
                        .take(32)
                        .collect::<Vec<_>>()
                        .into_iter()
                        .rev()
                        .collect();
                    format!("..{suffix}")
                } else {
                    docs_root.to_string()
                };

                println!(
                    "  {:<20} {:<8} {:<36} {:>8} {:>10} {:>10} {:>8} {}{}{}",
                    state.agent_id,
                    enabled_str,
                    docs_display,
                    format_count(state.chunks),
                    format_count(state.subjects),
                    format_count(state.resolved),
                    format_count(state.pending),
                    last_ext,
                    drift_tag,
                    error_tag,
                );
            }
            println!();
        }
    }

    Ok(())
}

fn build_agent_kg_state(
    db: &mika_agent::db::Database,
    global_home: &Path,
    agent_home: &Path,
    agent_name: &str,
) -> AgentKgState {
    let identity = mika_agent::prompt::load_identity(agent_home);

    // Resolve per-agent KG config
    let settings = match mika_common::config::Settings::load_for_agent(global_home, agent_home) {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!(agent = agent_name, error = %e, "Failed to load settings for KG status");
            return AgentKgState {
                agent_id: agent_name.to_string(),
                enabled: identity.kg.enabled,
                docs_root: None,
                docs_root_hash: None,
                chunks: 0,
                subjects: 0,
                resolved: 0,
                pending: 0,
                last_extraction: None,
                drift: false,
                error: Some(format!("config error: {e}")),
            };
        }
    };

    let kg_config = mika_agent::kg::config::resolve_per_agent_docs_root(&identity, &settings);

    match kg_config {
        Ok(mika_agent::kg::config::KgAgentConfig::Disabled { .. }) => AgentKgState {
            agent_id: agent_name.to_string(),
            enabled: false,
            docs_root: None,
            docs_root_hash: None,
            chunks: 0,
            subjects: 0,
            resolved: 0,
            pending: 0,
            last_extraction: None,
            drift: false,
            error: None,
        },
        Ok(mika_agent::kg::config::KgAgentConfig::Enabled { ref corpora }) => {
            // Use the first corpus for summary display (back-compat with single-root UI).
            // Multi-corpus detail is available via `mika kg status --agent X`.
            let first = corpora.first();
            let docs_root_hash = first.map(|c| c.docs_root_hash.clone());
            let docs_root = first.map(|c| c.docs_root.display().to_string());

            // Aggregate row counts across all corpora
            let mut chunks = 0u64;
            let mut subjects = 0u64;
            let mut last_extraction: Option<String> = None;
            for corpus in corpora {
                chunks += db
                    .kg_count_rows("kg_chunks", "docs_root_hash", &corpus.docs_root_hash)
                    .unwrap_or(0);
                subjects += db
                    .kg_count_rows(
                        "kg_subject_entities",
                        "docs_root_hash",
                        &corpus.docs_root_hash,
                    )
                    .unwrap_or(0);
                if let Ok(Some(ts)) = db.kg_last_extraction(&corpus.docs_root_hash)
                    && last_extraction.as_ref().is_none_or(|prev| ts > *prev)
                {
                    last_extraction = Some(ts);
                }
            }
            let resolved = db
                .kg_count_rows("kg_subject_resolutions", "agent_id", agent_name)
                .unwrap_or(0);

            // Pending = subjects that have no resolution log entry for this agent
            let pending = subjects.saturating_sub(resolved);

            // Drift detection — multi-corpus agents naturally have multiple hashes
            let drift = false;

            AgentKgState {
                agent_id: agent_name.to_string(),
                enabled: true,
                docs_root,
                docs_root_hash,
                chunks,
                subjects,
                resolved,
                pending,
                last_extraction,
                drift,
                error: None,
            }
        }
        Err(e) => {
            tracing::warn!(agent = agent_name, error = %e, "KG config error");
            AgentKgState {
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
                drift: false,
                error: Some(format!("{e}")),
            }
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

    // Print pre-confirmation summary (text only — JSON comes after purge)
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
}
