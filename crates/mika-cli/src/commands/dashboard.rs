use std::process::{Command, Stdio};

use anyhow::{Context, Result};

use crate::cli::DashboardCommand;

/// Run the dashboard subcommand.
pub async fn run(command: DashboardCommand) -> Result<()> {
    match command {
        DashboardCommand::Start => start().await,
        DashboardCommand::Stop => stop().await,
        DashboardCommand::Status => status().await,
        DashboardCommand::Open => open(),
    }
}

/// Resolve the mika-server base URL from env or default.
pub fn server_url() -> String {
    if let Ok(url) = std::env::var("MIKA_SERVER_URL") {
        return url;
    }
    // Load dotenv so MIKA_SERVER_PORT from ~/.mika/.env is available
    if let Ok(home) = mika_common::home::resolve_home_dir() {
        mika_common::dotenv::load_dotenv(&home);
    }
    let port = std::env::var("MIKA_SERVER_PORT").unwrap_or_else(|_| "8080".to_string());
    format!("http://localhost:{port}")
}

/// Read an auth token from env. Tries `MIKA_INTERNAL_TOKEN` first, then
/// falls back to `MIKA_DASHBOARD_TOKEN` (dashboard toggle endpoints accept either).
pub fn auth_token() -> Result<String> {
    // Load dotenv so tokens are available
    if let Ok(home) = mika_common::home::resolve_home_dir() {
        mika_common::dotenv::load_dotenv(&home);
    }
    std::env::var("MIKA_INTERNAL_TOKEN")
        .or_else(|_| std::env::var("MIKA_DASHBOARD_TOKEN"))
        .context(
            "MIKA_INTERNAL_TOKEN or MIKA_DASHBOARD_TOKEN is required to communicate with mika-server",
        )
}

/// Query the dashboard status from mika-server.
/// Returns `Some((enabled, has_assets, has_token))` if reachable, `None` otherwise.
pub async fn query_dashboard_status() -> Option<(bool, bool, bool)> {
    let url = format!("{}/api/v1/dashboard/status", server_url());
    let token = auth_token().ok()?;
    let client = reqwest::Client::new();
    let resp = client
        .get(&url)
        .header("authorization", format!("Bearer {token}"))
        .timeout(std::time::Duration::from_secs(3))
        .send()
        .await
        .ok()?;
    let json: serde_json::Value = resp.json().await.ok()?;
    Some((
        json["enabled"].as_bool().unwrap_or(false),
        json["has_assets"].as_bool().unwrap_or(false),
        json["has_token"].as_bool().unwrap_or(false),
    ))
}

/// Check if the embedded dashboard is currently enabled on the running server.
pub async fn is_dashboard_running() -> bool {
    query_dashboard_status()
        .await
        .is_some_and(|(enabled, _, _)| enabled)
}

/// Open the dashboard URL in the default browser. Returns a status message.
pub fn open_dashboard_in_browser() -> String {
    let url = format!("{}/dashboard", server_url());

    #[cfg(target_os = "linux")]
    let result = Command::new("xdg-open")
        .arg(&url)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
    #[cfg(target_os = "macos")]
    let result = Command::new("open")
        .arg(&url)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
    #[cfg(target_os = "windows")]
    let result = Command::new("cmd")
        .args(["/c", "start", &url])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();

    match result {
        Ok(_) => format!("Opening {url} in browser..."),
        Err(e) => format!("Could not open browser: {e}\n  URL: {url}"),
    }
}

async fn start() -> Result<()> {
    let token = auth_token()?;
    let url = format!("{}/api/v1/dashboard/enable", server_url());
    let client = reqwest::Client::new();
    let resp = client
        .post(&url)
        .header("authorization", format!("Bearer {token}"))
        .timeout(std::time::Duration::from_secs(5))
        .send()
        .await
        .context("Failed to connect to mika-server. Is it running?")?;

    if resp.status().is_success() {
        println!("Dashboard enabled.");
        println!("  URL: {}/dashboard", server_url());
    } else {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        println!("Failed to enable dashboard: {status} {body}");
    }
    Ok(())
}

async fn stop() -> Result<()> {
    let token = auth_token()?;
    let url = format!("{}/api/v1/dashboard/disable", server_url());
    let client = reqwest::Client::new();
    let resp = client
        .post(&url)
        .header("authorization", format!("Bearer {token}"))
        .timeout(std::time::Duration::from_secs(5))
        .send()
        .await
        .context("Failed to connect to mika-server. Is it running?")?;

    if resp.status().is_success() {
        println!("Dashboard disabled.");
    } else {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        println!("Failed to disable dashboard: {status} {body}");
    }
    Ok(())
}

async fn status() -> Result<()> {
    match query_dashboard_status().await {
        Some((enabled, has_assets, has_token)) => {
            if enabled {
                println!("Dashboard is enabled.");
                println!("  URL: {}/dashboard", server_url());
                if !has_assets {
                    println!("  Warning: No embedded assets found. Build the dashboard first.");
                }
                if !has_token {
                    println!("  Warning: MIKA_DASHBOARD_TOKEN is not set.");
                }
            } else {
                println!("Dashboard is disabled.");
                println!("  Enable with: mika dashboard start");
            }
        }
        None => {
            println!("Could not reach mika-server.");
            println!("  Is it running at {}?", server_url());
        }
    }
    Ok(())
}

fn open() -> Result<()> {
    println!("{}", open_dashboard_in_browser());
    Ok(())
}
