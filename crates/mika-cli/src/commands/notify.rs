use anyhow::{Context, Result};
use secrecy::{ExposeSecret, SecretString};

use crate::cli::{NotifyChannel, NotifySeverity};
use crate::init;

/// Well-known fixed-UUID for the notifications session.
/// Deliberately synthetic (zero-filled prefix + "notify" digits) to sort distinctly
/// from real UUIDs and be instantly recognizable in DB queries.
const NOTIFICATIONS_SESSION_ID: &str = "00000000-0000-0000-0000-700000710717";

/// Agent that owns the notifications session.
/// `mika notify` always writes to the default "mika" agent's DB.
const NOTIFICATIONS_AGENT: &str = "mika";

/// Send an operator notification.
///
/// Writes a message to the well-known notifications session in the DB.
/// Optionally sends via Telegram through the gateway's `/send` endpoint.
pub async fn run(text: &str, channel: &NotifyChannel, severity: &NotifySeverity) -> Result<()> {
    let ctx = init::init_db_only_for_agent(NOTIFICATIONS_AGENT)?;

    let severity_str = match severity {
        NotifySeverity::Info => "info",
        NotifySeverity::Warn => "warn",
        NotifySeverity::Escalate => "escalate",
    };

    let channel_str = match channel {
        NotifyChannel::Cli => "cli",
        NotifyChannel::Telegram => "telegram",
    };

    // Ensure the notifications session exists (idempotent).
    let session_metadata = serde_json::json!({
        "source": "mika-notify",
        "name": "mika notify",
    });
    ctx.async_db
        .create_session_if_not_exists(
            NOTIFICATIONS_SESSION_ID,
            NOTIFICATIONS_AGENT,
            "notifications",
            Some(&session_metadata.to_string()),
        )
        .await
        .context("failed to create notifications session")?;

    // Format the notification message with severity prefix.
    let formatted = format!("[{severity_str}] {text}");

    // Build message metadata.
    let metadata = serde_json::json!({
        "source": "mika-notify",
        "severity": severity_str,
        "channel": channel_str,
    });

    // Save the notification as a system-role message.
    ctx.async_db
        .save_message_with_metadata(
            NOTIFICATIONS_SESSION_ID,
            "system",
            &formatted,
            Some(&metadata.to_string()),
            None,  // no trace_id — CLI invocation
            false, // not internal — operator-visible
        )
        .await
        .context("failed to save notification message")?;

    // Telegram delivery (optional).
    if matches!(channel, NotifyChannel::Telegram) {
        match send_via_gateway(&ctx, &formatted).await {
            Ok(()) => {
                eprintln!("✓ Notification sent via Telegram.");
            }
            Err(e) => {
                eprintln!("⚠ Telegram delivery failed: {e}");
                eprintln!("  Notification saved to DB — check `mika status` or the dashboard.");
            }
        }
    }

    eprintln!(
        "✓ Notification saved (session={}, severity={severity_str}).",
        NOTIFICATIONS_SESSION_ID
    );

    Ok(())
}

/// Resolve the gateway base URL from env or default (same pattern as webhook.rs).
fn gateway_url() -> String {
    std::env::var("MIKA_GATEWAY_URL").unwrap_or_else(|_| "http://localhost:3001".to_string())
}

/// POST the notification text to the gateway's `/send` endpoint for Telegram delivery.
async fn send_via_gateway(ctx: &init::DbContext, text: &str) -> Result<()> {
    let gw_url = gateway_url();

    let internal_token: &SecretString = ctx.settings.internal_token.as_ref().ok_or_else(|| {
        anyhow::anyhow!("MIKA_INTERNAL_TOKEN not configured — cannot send via gateway")
    })?;

    // Resolve the operator's Telegram chat_id from the DB.
    let chat_id_str = ctx
        .async_db
        .get_customer_config("chat_id")
        .await?
        .ok_or_else(|| {
            anyhow::anyhow!(
                "chat_id not configured — no Telegram pairing yet. \
                 Send a message to the bot on Telegram first."
            )
        })?;

    let chat_id: i64 = chat_id_str
        .parse()
        .map_err(|e| anyhow::anyhow!("invalid chat_id in DB: {e}"))?;

    if chat_id == 0 {
        anyhow::bail!("chat_id is 0 (no reply channel) — cannot deliver via Telegram");
    }

    let payload = serde_json::json!({
        "chat_id": chat_id,
        "text": text,
        "agent_name": NOTIFICATIONS_AGENT,
    });

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{gw_url}/send"))
        .bearer_auth(internal_token.expose_secret())
        .json(&payload)
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
        .map_err(|e| {
            if e.is_connect() {
                anyhow::anyhow!("gateway unreachable at {gw_url}: {e}")
            } else if e.is_timeout() {
                anyhow::anyhow!("gateway request timed out: {e}")
            } else {
                anyhow::anyhow!("gateway request failed: {e}")
            }
        })?;

    if resp.status().is_success() {
        Ok(())
    } else {
        let status = resp.status();
        let body = resp
            .text()
            .await
            .unwrap_or_default()
            .chars()
            .take(200)
            .collect::<String>();
        if body.is_empty() {
            anyhow::bail!("gateway /send returned {status}")
        } else {
            anyhow::bail!("gateway /send returned {status}: {body}")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_notifications_session_id_is_valid_uuid() {
        // The fixed UUID must parse as a valid UUID.
        uuid::Uuid::parse_str(NOTIFICATIONS_SESSION_ID)
            .expect("NOTIFICATIONS_SESSION_ID must be a valid UUID");
    }

    #[test]
    fn test_notifications_session_id_is_clearly_synthetic() {
        // Must start with zeros (clearly not a random v4 UUID).
        assert!(
            NOTIFICATIONS_SESSION_ID.starts_with("00000000-"),
            "session ID should be clearly synthetic"
        );
    }

    #[test]
    fn test_severity_formatting() {
        // Verify the formatted message includes severity prefix.
        let text = "test message";
        let formatted = format!("[info] {text}");
        assert_eq!(formatted, "[info] test message");

        let formatted = format!("[escalate] {text}");
        assert_eq!(formatted, "[escalate] test message");
    }
}
