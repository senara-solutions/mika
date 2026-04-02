use anyhow::Result;

use crate::cli::TokenCommand;

/// Run the `mika token` subcommand.
///
/// Lightweight path: loads dotenv + Settings + GitHubApp only.
/// No tracing, no DB, no agent resolution.
pub async fn run(command: TokenCommand, home_dir: &std::path::Path) -> Result<()> {
    match command {
        TokenCommand::Github => github(home_dir).await,
    }
}

/// Print a GitHub App installation token to stdout.
///
/// Loads config from `~/.mika/.env`, generates a JWT, exchanges it for an
/// installation token (with file-based caching), and prints the bare token.
/// All diagnostics go to stderr. Exits with code 1 on any failure.
async fn github(home_dir: &std::path::Path) -> Result<()> {
    let settings = mika_common::config::Settings::load(home_dir)?;
    let github_app =
        mika_common::github_app::GitHubApp::from_settings(&settings).ok_or_else(|| {
            anyhow::anyhow!(
                "GitHub App not configured. Set MIKA_GITHUB_APP_ID, \
                 MIKA_GITHUB_APP_PRIVATE_KEY, and MIKA_GITHUB_APP_INSTALLATION_ID \
                 in ~/.mika/.env"
            )
        })?;

    let cache_path = home_dir.join("github_app_token.json");
    let token = github_app
        .installation_token_with_file_cache(&cache_path)
        .await?;

    // Bare token to stdout — credential helpers and scripts consume this
    print!("{token}");

    Ok(())
}
