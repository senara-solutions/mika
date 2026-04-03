use anyhow::Result;
use std::path::Path;

use crate::cli::TokenCommand;

/// Run the `mika token` subcommand.
///
/// Lightweight path: loads dotenv + Settings + GitHubApp only.
/// No tracing, no DB, no agent resolution.
///
/// When `agent_home` is `Some`, loads per-agent settings for agent-specific
/// GitHub App credentials. Otherwise, uses the global home directory.
pub async fn run(
    command: &TokenCommand,
    global_home: &Path,
    agent_home: Option<&Path>,
) -> Result<()> {
    match command {
        TokenCommand::Github => github(global_home, agent_home).await,
    }
}

/// Print a GitHub App installation token to stdout.
///
/// Loads config from `~/.mika/.env` (global) or `~/.mika/agents/<name>/.env` (per-agent),
/// generates a JWT, exchanges it for an installation token (with file-based caching),
/// and prints the bare token. All diagnostics go to stderr. Exits with code 1 on any failure.
async fn github(global_home: &Path, agent_home: Option<&Path>) -> Result<()> {
    let settings = if let Some(agent_home) = agent_home {
        mika_common::config::Settings::load_for_agent(global_home, agent_home)?
    } else {
        mika_common::config::Settings::load(global_home)?
    };

    let github_app =
        mika_common::github_app::GitHubApp::from_settings(&settings).ok_or_else(|| {
            anyhow::anyhow!(
                "GitHub App not configured. Set MIKA_GITHUB_APP_ID, \
                 MIKA_GITHUB_APP_PRIVATE_KEY, and MIKA_GITHUB_APP_INSTALLATION_ID \
                 in ~/.mika/.env (or per-agent ~/.mika/agents/<name>/.env)"
            )
        })?;

    // Use per-agent cache dir if available, otherwise global
    let cache_dir = agent_home.unwrap_or(global_home);
    let cache_path = cache_dir.join("github_app_token.json");
    let token = github_app
        .installation_token_with_file_cache(&cache_path)
        .await?;

    // Bare token to stdout — credential helpers and scripts consume this
    print!("{token}");

    Ok(())
}
