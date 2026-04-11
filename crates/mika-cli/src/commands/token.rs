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

/// Print a GitHub token to stdout for credential helpers and scripts.
///
/// Loads config from `~/.mika/.env` (global) or `~/.mika/agents/<name>/.env`
/// (per-agent) and resolves the token in this order (matching ADR-008 /
/// [`mika_common::config::Settings::resolve_github_token`]):
///
/// 1. **`MIKA_GITHUB_TOKEN` PAT** — the agent's machine user identity.
///    Required for operations where GitHub author/reviewer identity matters
///    (PR reviews, merges, comments, issue creation). Two agents must present
///    distinct PATs so they aren't collapsed into the single
///    `mika-platform-bot[bot]` App identity (GitHub rejects self-reviews).
/// 2. **GitHub App installation token** — short-lived, org-scoped, minted via
///    JWT exchange with file-based caching. Used only when no PAT is
///    configured (single-identity deployments, bootstrap).
///
/// All diagnostics go to stderr. Exits with code 1 when neither source is
/// configured.
async fn github(global_home: &Path, agent_home: Option<&Path>) -> Result<()> {
    let settings = if let Some(agent_home) = agent_home {
        mika_common::config::Settings::load_for_agent(global_home, agent_home)?
    } else {
        mika_common::config::Settings::load(global_home)?
    };

    // PAT first — the agent's machine user identity.
    if let Some(pat) = settings.github_token.as_deref() {
        print!("{pat}");
        return Ok(());
    }

    // Fall back to minting a GitHub App installation token.
    let github_app =
        mika_common::github_app::GitHubApp::from_settings(&settings).ok_or_else(|| {
            anyhow::anyhow!(
                "No GitHub credentials configured. Set MIKA_GITHUB_TOKEN (machine \
                 user PAT) in ~/.mika/agents/<name>/.env, or configure \
                 MIKA_GITHUB_APP_ID + MIKA_GITHUB_APP_PRIVATE_KEY + \
                 MIKA_GITHUB_APP_INSTALLATION_ID in ~/.mika/.env"
            )
        })?;

    // Use per-agent cache dir if available, otherwise global.
    let cache_dir = agent_home.unwrap_or(global_home);
    let cache_path = cache_dir.join("github_app_token.json");
    let token = github_app
        .installation_token_with_file_cache(&cache_path)
        .await?;

    // Bare token to stdout — credential helpers and scripts consume this.
    print!("{token}");

    Ok(())
}
