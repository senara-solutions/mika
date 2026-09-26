use anyhow::{Context, Result};

use crate::cli::{OutputFormat, WebhookCommand};

/// Run the webhook subcommand.
pub async fn run(command: WebhookCommand, format: &OutputFormat) -> Result<()> {
    match command {
        WebhookCommand::ListDead { status, limit } => list_dead(status, limit, format).await,
        WebhookCommand::Replay { delivery_id } => replay(&delivery_id, format).await,
        WebhookCommand::ReplayAll => replay_all(format).await,
    }
}

/// Resolve the gateway base URL from env or default.
fn gateway_url() -> String {
    if let Ok(url) = std::env::var("MIKA_GATEWAY_URL") {
        return url;
    }
    // Load dotenv so env vars from ~/.mika/.env are available
    if let Ok(home) = mika_common::home::resolve_home_dir() {
        mika_common::dotenv::load_dotenv(&home);
    }
    std::env::var("MIKA_GATEWAY_URL").unwrap_or_else(|_| "http://localhost:3001".to_string())
}

/// Read the internal token for gateway auth.
fn auth_token() -> Result<String> {
    if let Ok(home) = mika_common::home::resolve_home_dir() {
        mika_common::dotenv::load_dotenv(&home);
    }
    std::env::var("MIKA_INTERNAL_TOKEN")
        .context("MIKA_INTERNAL_TOKEN is required to communicate with the gateway")
}

/// Build a reqwest client for gateway API calls.
fn build_client(token: &str) -> reqwest::Client {
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert(
        reqwest::header::AUTHORIZATION,
        reqwest::header::HeaderValue::from_str(&format!("Bearer {token}"))
            .expect("valid header value"),
    );
    reqwest::Client::builder()
        .default_headers(headers)
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .expect("failed to build HTTP client")
}

/// List dead-letter queue entries.
#[allow(clippy::print_literal)]
async fn list_dead(
    status: Option<String>,
    limit: Option<i64>,
    format: &OutputFormat,
) -> Result<()> {
    let base = gateway_url();
    let token = auth_token()?;
    let client = build_client(&token);

    let mut url = format!("{base}/webhook/dlq");
    let mut params = Vec::new();
    if let Some(ref s) = status {
        params.push(format!("status={s}"));
    }
    if let Some(l) = limit {
        params.push(format!("limit={l}"));
    }
    if !params.is_empty() {
        url.push('?');
        url.push_str(&params.join("&"));
    }

    let resp = client
        .get(&url)
        .send()
        .await
        .context("failed to connect to gateway — is it running?")?;

    if !resp.status().is_success() {
        anyhow::bail!(
            "gateway returned HTTP {} for DLQ list",
            resp.status().as_u16()
        );
    }

    let body: serde_json::Value = resp.json().await?;

    match format {
        OutputFormat::Json => {
            println!("{}", serde_json::to_string_pretty(&body)?);
        }
        OutputFormat::Yaml => {
            print!("{}", serde_yaml::to_string(&body)?);
        }
        OutputFormat::Text => {
            let deliveries = body["deliveries"].as_array();
            let count = deliveries.map(|d| d.len()).unwrap_or(0);

            if count == 0 {
                println!("No DLQ entries found.");
                return Ok(());
            }

            println!(
                "{:<38} {:<16} {:<12} {:<8} {:<20} {}",
                "DELIVERY ID", "EVENT TYPE", "AGENT", "ATTEMPTS", "CREATED", "LAST ERROR"
            );
            println!("{}", "-".repeat(120));

            for entry in deliveries.unwrap() {
                let id = entry["delivery_id"].as_str().unwrap_or("-");
                let event = entry["event_type"].as_str().unwrap_or("-");
                let agent = entry["target_agent"].as_str().unwrap_or("-");
                let attempts = entry["attempts"].as_i64().unwrap_or(0);
                let created = entry["created_at"]
                    .as_str()
                    .unwrap_or("-")
                    .chars()
                    .take(19)
                    .collect::<String>();
                let error = entry["last_error"]
                    .as_str()
                    .unwrap_or("")
                    .chars()
                    .take(40)
                    .collect::<String>();

                println!(
                    "{:<38} {:<16} {:<12} {:<8} {:<20} {}",
                    id, event, agent, attempts, created, error
                );
            }

            println!("\n{count} entries total");
        }
    }

    Ok(())
}

/// Replay a single DLQ entry.
async fn replay(delivery_id: &str, format: &OutputFormat) -> Result<()> {
    let base = gateway_url();
    let token = auth_token()?;
    let client = build_client(&token);

    let url = format!("{base}/webhook/dlq/{delivery_id}/replay");
    let resp = client
        .post(&url)
        .send()
        .await
        .context("failed to connect to gateway — is it running?")?;

    if resp.status().as_u16() == 404 {
        anyhow::bail!("delivery '{delivery_id}' not found in DLQ");
    }

    if !resp.status().is_success() {
        anyhow::bail!(
            "gateway returned HTTP {} for DLQ replay",
            resp.status().as_u16()
        );
    }

    let body: serde_json::Value = resp.json().await?;

    match format {
        OutputFormat::Json => {
            println!("{}", serde_json::to_string_pretty(&body)?);
        }
        OutputFormat::Yaml => {
            print!("{}", serde_yaml::to_string(&body)?);
        }
        OutputFormat::Text => {
            let status = body["delivery"]["status"].as_str().unwrap_or("unknown");
            if status == "delivered" {
                println!("Replay succeeded — delivery '{delivery_id}' is now 'delivered'.");
            } else {
                let error = body["delivery"]["last_error"]
                    .as_str()
                    .unwrap_or("unknown error");
                println!(
                    "Replay attempted — delivery '{delivery_id}' status: {status} (error: {error})"
                );
            }
        }
    }

    Ok(())
}

/// Replay all dead DLQ entries.
async fn replay_all(format: &OutputFormat) -> Result<()> {
    let base = gateway_url();
    let token = auth_token()?;
    let client = build_client(&token);

    let url = format!("{base}/webhook/dlq/replay-all");
    let resp = client
        .post(&url)
        .send()
        .await
        .context("failed to connect to gateway — is it running?")?;

    if !resp.status().is_success() {
        anyhow::bail!(
            "gateway returned HTTP {} for DLQ replay-all",
            resp.status().as_u16()
        );
    }

    let body: serde_json::Value = resp.json().await?;

    match format {
        OutputFormat::Json => {
            println!("{}", serde_json::to_string_pretty(&body)?);
        }
        OutputFormat::Yaml => {
            print!("{}", serde_yaml::to_string(&body)?);
        }
        OutputFormat::Text => {
            let succeeded = body["succeeded"].as_u64().unwrap_or(0);
            let failed = body["failed"].as_u64().unwrap_or(0);
            let total = body["total"].as_u64().unwrap_or(0);

            if total == 0 {
                println!("No dead entries to replay.");
            } else {
                println!(
                    "Replay complete: {succeeded} succeeded, {failed} failed ({total} total)."
                );
            }
        }
    }

    Ok(())
}
