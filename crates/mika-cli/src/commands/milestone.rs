//! `mika milestone {read,assess,report}` — thin CLI adapter over
//! `mika_agent::milestone_manager` (Phase 1, LECTURE seule).
//!
//! **Wrapper-only INV-2.** No GitHub logic lives here — the CLI dispatches
//! straight into the agent-crate module. See
//! `crates/mika-agent/src/milestone_manager/mod.rs` for architecture.

use crate::cli::{MilestoneCommand, OutputFormat};
use anyhow::{Context, Result};
use mika_agent::milestone_manager::{Assessor, AssessorConfig, MilestoneRef, Reader, Reporter};

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
