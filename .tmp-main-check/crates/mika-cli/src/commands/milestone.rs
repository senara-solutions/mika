//! `mika milestone {read,assess,report,reports}` — thin CLI adapter over
//! `mika_agent::milestone_manager` (Phase 1, LECTURE seule).
//!
//! **Wrapper-only INV-2.** No GitHub logic lives here — the CLI dispatches
//! straight into the agent-crate module. See
//! `crates/mika-agent/src/milestone_manager/mod.rs` for architecture.
//!
//! Same rule for the offline sink (mika#2267): the path resolution and the
//! directory read live in `milestone_manager::sink_dir`, beside the writer.
//! Recomposing either here would make the reader able to look somewhere the
//! writer does not — which is the defect the subcommand exists to close,
//! reproduced one layer up.

use crate::cli::{MilestoneCommand, OutputFormat};
use anyhow::{Context, Result};
use mika_agent::milestone_manager::{
    Assessor, AssessorConfig, ENV_OFFLINE_SINK_DIR, MilestoneRef, Reader, Reporter, SinkListing,
    list_sink_reports,
};

pub async fn run(command: MilestoneCommand) -> Result<()> {
    // Forward the ambient GH_TOKEN if present — mirrors how other `gh`
    // wrappers in the codebase pick it up.
    let token = std::env::var("GH_TOKEN")
        .ok()
        .or_else(|| std::env::var("MIKA_GITHUB_TOKEN").ok());

    match command {
        MilestoneCommand::Read { target, format } => {
            let r =
                MilestoneRef::parse(&target).map_err(|e| anyhow::anyhow!("invalid target: {e}"))?;
            let state = Reader::new(token)
                .read(&r)
                .await
                .context("gh read failed")?;
            emit(&state, format)
        }
        MilestoneCommand::Assess {
            target,
            format,
            silence_threshold_days,
        } => {
            let r =
                MilestoneRef::parse(&target).map_err(|e| anyhow::anyhow!("invalid target: {e}"))?;
            let state = Reader::new(token)
                .read(&r)
                .await
                .context("gh read failed")?;
            let a = Assessor::new(AssessorConfig {
                silence_threshold_days,
            })
            .assess(&state);
            emit(&a, format)
        }
        MilestoneCommand::Report {
            target,
            silence_threshold_days,
        } => {
            let r =
                MilestoneRef::parse(&target).map_err(|e| anyhow::anyhow!("invalid target: {e}"))?;
            let state = Reader::new(token)
                .read(&r)
                .await
                .context("gh read failed")?;
            let a = Assessor::new(AssessorConfig {
                silence_threshold_days,
            })
            .assess(&state);
            let md = Reporter::new().report(&state, &a);
            print!("{md}");
            Ok(())
        }
        MilestoneCommand::Reports {
            target,
            latest,
            limit,
            format,
        } => run_reports(target, latest, limit, format),
    }
}

// ---- mika#2267 C2 — le puits a un lecteur ---------------------------------

fn run_reports(
    target: Option<String>,
    latest: bool,
    limit: usize,
    format: OutputFormat,
) -> Result<()> {
    let slug = match target.as_deref() {
        Some(raw) => Some(
            MilestoneRef::parse(raw)
                .map_err(|e| anyhow::anyhow!("invalid --target: {e}"))?
                .slug(),
        ),
        None => None,
    };

    let listing = list_sink_reports(slug.as_deref());

    if latest {
        return print_latest(&listing, target.as_deref());
    }

    match format {
        OutputFormat::Text => print_listing_text(&listing, limit, target.as_deref())?,
        OutputFormat::Json => println!(
            "{}",
            serde_json::to_string_pretty(&listing_json(&listing, limit))?
        ),
        OutputFormat::Yaml => print!("{}", serde_yaml::to_string(&listing_json(&listing, limit))?),
    }
    Ok(())
}

/// Render the most recent report's Markdown.
///
/// A sink with nothing to show is a **failure** here, not an empty success:
/// the caller asked for content. The two no-content cases keep their distinct
/// wording — a directory that does not exist and a directory that is empty say
/// opposite things about the cadence.
fn print_latest(listing: &SinkListing, target: Option<&str>) -> Result<()> {
    match listing.entries().first() {
        Some(entry) => {
            let body = std::fs::read_to_string(&entry.path)
                .with_context(|| format!("reading {}", entry.path.display()))?;
            print!("{body}");
            Ok(())
        }
        None => anyhow::bail!("{}", no_content_reason(listing, target)),
    }
}

/// The sentence that distinguishes « le puits n'existe pas » from « le puits
/// est vide ». Without it, the reader would reproduce at its own surface the
/// silence it exists to lift.
fn no_content_reason(listing: &SinkListing, target: Option<&str>) -> String {
    let dir = listing.dir().display();
    let source = listing.source().as_str();
    match listing {
        SinkListing::DirAbsent { .. } => format!(
            "no offline sink at {dir} (path source: {source}) — the cadence has written nothing \
             HERE.\nIf reports are written elsewhere, set {ENV_OFFLINE_SINK_DIR} to that path. \
             Compare with the `offline_sink_dir` field of the `manager_delivery_resolved` log \
             line: the CLI and the daemon can resolve different paths (different HOME, service \
             running as another user)."
        ),
        _ => match target {
            Some(t) => format!(
                "no report for {t} in {dir} (path source: {source}) — the sink exists and holds \
                 nothing for this milestone."
            ),
            None => format!(
                "no report in {dir} (path source: {source}) — the sink exists and is empty."
            ),
        },
    }
}

fn print_listing_text(listing: &SinkListing, limit: usize, target: Option<&str>) -> Result<()> {
    // The consulted path is named on EVERY path, including the nominal one —
    // that is what makes a CLI/daemon path mismatch readable in one line
    // instead of an investigation.
    println!(
        "Sink: {} (path source: {})",
        listing.dir().display(),
        listing.source().as_str()
    );
    if let Some(t) = target {
        println!("Target: {t}");
    }
    println!();

    let entries = listing.entries();
    if entries.is_empty() {
        println!("{}", no_content_reason(listing, target));
        return Ok(());
    }

    for e in entries.iter().take(limit) {
        println!(
            "{:<26}  {:<28}  {:>8}  {}",
            e.timestamp.as_deref().unwrap_or("(undated)"),
            e.milestone_slug.as_deref().unwrap_or("(unknown)"),
            human_size(e.size_bytes),
            e.file_name
        );
    }
    println!();
    let shown = entries.len().min(limit);
    if shown < entries.len() {
        println!(
            "{shown} of {} report(s) — raise --limit to see the rest.",
            entries.len()
        );
    } else {
        println!("{} report(s).", entries.len());
    }
    Ok(())
}

fn listing_json(listing: &SinkListing, limit: usize) -> serde_json::Value {
    let state = match listing {
        SinkListing::DirAbsent { .. } => "dir_absent",
        SinkListing::Empty { .. } => "empty",
        SinkListing::Entries { .. } => "entries",
    };
    serde_json::json!({
        "sink_dir": listing.dir().display().to_string(),
        "sink_dir_source": listing.source().as_str(),
        // Three states, never a bare empty list: an absent sink and an empty
        // one call for different operator gestures.
        "state": state,
        "total": listing.entries().len(),
        "reports": listing.entries().iter().take(limit).collect::<Vec<_>>(),
    })
}

fn human_size(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    }
}

/// Emit a serializable value in the requested format.
fn emit<T: serde::Serialize>(value: &T, format: OutputFormat) -> Result<()> {
    match format {
        OutputFormat::Json => {
            println!("{}", serde_json::to_string_pretty(value)?);
        }
        OutputFormat::Yaml => {
            print!("{}", serde_yaml::to_string(value)?);
        }
        OutputFormat::Text => {
            // For the `read`/`assess` subcommands, text output falls back
            // to pretty JSON — the human-friendly text surface is
            // `mika milestone report`.
            println!("{}", serde_json::to_string_pretty(value)?);
        }
    }
    Ok(())
}
