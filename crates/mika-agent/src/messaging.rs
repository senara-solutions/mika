use anyhow::{Result, anyhow};
use async_trait::async_trait;
use secrecy::{ExposeSecret, SecretString};
use std::time::Duration;
use tracing::{error, warn};

use crate::async_db::AsyncDatabase;

/// Outcome of a message delivery attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SendOutcome {
    /// Message was delivered successfully (gateway returned 2xx).
    Delivered,
    /// Delivery failed after retries. The message was saved to `failed_sends`
    /// for later flush, but the agent should treat this as a failure.
    Failed {
        /// Human-readable reason including HTTP status or network error classification.
        reason: String,
    },
    /// No reply channel available for this session. `chat_id == 0` is the
    /// documented sentinel for GitHub webhook and other non-Telegram channels
    /// where the agent should use channel-appropriate tools (e.g., `run_gh`)
    /// instead of `send_message`. This is a permanent session condition, not a
    /// transient error — callers should not retry or save to `failed_sends`.
    NoChannel,
}

/// Severity tier of an operator notification (mika#1381).
///
/// There is exactly one question this axis answers: *must this line cross a
/// mute?* Two variants, therefore — a `P1` without a distinct destination would
/// only be a third name for the normal chat.
///
/// - `P0` — « the machine is broken or stalled, a human must step in ».
///   Routed to the escalation chat when one is configured.
/// - `P2` — everything else, including nominal hand-backs where the machine did
///   its job. Routed to the normal chat, which the operator may mute.
///
/// The P0 surface stays **low-volume by construction**: a "never-muted" surface
/// receiving seventeen classes of message is a surface that will be muted. See
/// `docs/plans/2026-10-19-001-feat-1381-notifications-severity-tiered-operator-plan.md`
/// § *Classification des 17 sites* for the per-site arbitration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotificationSeverity {
    /// Breakage or stall requiring operator intervention.
    P0,
    /// Routine acknowledgement — the machine is working.
    P2,
}

impl NotificationSeverity {
    /// Stable token for structured log fields.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::P0 => "p0",
            Self::P2 => "p2",
        }
    }
}

/// Which surface a notification resolved to (mika#1381).
///
/// This is the discriminant the routing tests assert and the fallback WARN
/// reads — the resolved `chat_id` alone cannot distinguish "the escalation chat
/// happens to equal the normal chat" from "there is no escalation chat".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetKind {
    /// The normal chat, as every notification used before mika#1381.
    Normal,
    /// The dedicated escalation chat.
    Escalation,
    /// A P0 that found no usable escalation chat and fell back to the normal
    /// one. Always paired with the `operator_escalation_chat_unset` WARN — a
    /// silent fallback is the "alarm into the void" this work exists to close.
    EscalationUnsetFallback,
}

/// Cette valeur est-elle une route P0 utilisable ? (mika#2871)
///
/// Extrait de [`GatewayMessageSender::resolve_notification_target`] pour que la
/// garde de démarrage et le transport posent *la même* question. `Some(0)` est
/// faux : voir la doc de `resolve_notification_target` — c'est le sentinelle
/// « aucun canal de réponse » (mika#650/#1090), donc un chat d'escalade à `0`
/// retombe sur le chat normal exactement comme une variable absente. Une garde
/// qui ne testerait que `is_none()` serait verte sur cette configuration, et la
/// notification d'opérateur partirait chez l'utilisateur final.
pub(crate) fn p0_route_is_usable(escalation_chat_id: Option<i64>) -> bool {
    matches!(escalation_chat_id, Some(id) if id != 0)
}

/// Trait for sending outbound messages to the user.
/// CLI mode uses the fallback in send_message tool; HTTP mode will POST to the gateway.
///
/// **Text-only outbound:** This trait accepts only `text: &str` — agent responses
/// delivered to users are always plain text. Tool-produced images (e.g., from exec
/// handler scripts returning `__mika_v1` envelopes, or the file-reader skill) are
/// included in the Claude API `tool_result` content blocks for the LLM's visual
/// analysis, but are never forwarded to the end user through this trait. If outbound
/// image delivery is needed in the future, the `send` signature and the gateway
/// `/send` payload would need to be extended.
///
/// Send + Sync bounds allow `Arc<dyn MessageSender>` in AppState and ToolContext.
#[async_trait]
pub trait MessageSender: Send + Sync {
    async fn send(&self, text: &str) -> Result<SendOutcome>;

    /// Send carrying an explicit operator severity (mika#1381).
    ///
    /// **Default: the severity is ignored.** A rail with no second channel
    /// (`NoopSender`, the CLI rail, every test double) keeps exactly its
    /// current behaviour and needs no edit. Only `GatewayMessageSender`
    /// overrides this.
    async fn send_with_severity(
        &self,
        text: &str,
        _severity: NotificationSeverity,
    ) -> Result<SendOutcome> {
        self.send(text).await
    }
}

/// What the `failed_sends` flush needs, and nothing more: deliver a text and
/// say whether it went through (mika#2935).
///
/// Deliberately **not** `MessageSender` — that trait parks, and the parking is
/// the defect mika#2935 closes. A `MessageSender` double would not park while
/// the real `GatewayMessageSender` does, so a test written against it would
/// stay green above the bug.
#[async_trait]
pub trait FlushDelivery: Send + Sync {
    async fn try_deliver(&self, text: &str) -> Result<SendOutcome>;
}

/// Sends outbound messages by POSTing to the gateway's /send endpoint.
///
/// Two entries, and the difference between them is the whole of mika#2935:
/// [`Self::try_deliver_with_severity`] delivers and reports, with no side
/// effect on `failed_sends`; `MessageSender::send_with_severity` adds the
/// parking. See the latter's doc-comment.
pub struct GatewayMessageSender {
    client: reqwest::Client,
    gateway_url: String,
    internal_token: SecretString,
    db: AsyncDatabase,
    request_id: Option<String>,
    agent_name: Option<String>,
    /// Explicit chat_id override. When set, skips the DB lookup in `send()`.
    /// Used by delegated agents whose agent-scoped `customer_config` doesn't
    /// contain the chat_id (it's stored under the orchestrator's agent_id).
    chat_id: Option<i64>,
    /// Per-customer identifier (from `MIKA_CUSTOMER_ID` → `Settings.customer_id`).
    /// When `Some`, included in the `/send` payload so the gateway can resolve
    /// the per-customer bot token in multi-bot mode (#1607). When `None`
    /// (CLI/dev, single-bot deployments), the field is omitted and the gateway
    /// falls back to its global single-bot path — no behavioral change.
    customer_id: Option<String>,
    /// Dedicated escalation chat for `P0` notifications (mika#1381), from
    /// `MIKA_OPERATOR_ESCALATION_CHAT_ID` → `Settings.operator_escalation_chat_id`.
    ///
    /// `None` is the nominal state (a deployment with a single channel): a P0
    /// then falls back to the normal chat with a named WARN. Armed via
    /// [`GatewayMessageSender::with_escalation_chat_id`], never through
    /// `new()` — see that method for why.
    escalation_chat_id: Option<i64>,
}

impl GatewayMessageSender {
    // Constructor threads transport config + routing identity (request_id,
    // agent_name, chat_id override, customer_id) through positionally. The arg
    // count is inherent to the sender's responsibilities, not accidental
    // complexity; a builder would add ceremony without clarifying intent.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        gateway_url: String,
        internal_token: SecretString,
        db: AsyncDatabase,
        client: reqwest::Client,
        request_id: Option<String>,
        agent_name: Option<String>,
        chat_id: Option<i64>,
        customer_id: Option<String>,
    ) -> Self {
        Self {
            client,
            gateway_url,
            internal_token,
            db,
            request_id,
            agent_name,
            chat_id,
            customer_id,
            escalation_chat_id: None,
        }
    }

    /// Arm the second channel for `P0` notifications (mika#1381). Chainable
    /// behind `new()` so that the constructor's arity — and its 16 callsites,
    /// 10 of which are tests — does not move for a field only 2 of them care
    /// about.
    ///
    /// Chained rather than a `builder()` entry point for a second, structural
    /// reason: `qa_head_supersession.rs` scans the literal
    /// `"GatewayMessageSender::new("` inside `run_agent_for_message` and
    /// asserts it follows `skip_superseded_review_turn(`. A chained builder
    /// leaves that literal in place; a `GatewayMessageSender::builder()` entry
    /// point would redden `mika2671_b2_la_garde_de_revue_precede_le_tour` with
    /// an error message that does not explain why.
    pub fn with_escalation_chat_id(mut self, chat_id: Option<i64>) -> Self {
        self.escalation_chat_id = chat_id;
        self
    }

    /// Resolve which chat a notification of this severity goes to (mika#1381).
    ///
    /// Pure past the already-resolved `normal_chat_id`, so routing is testable
    /// without an HTTP server — the same property `resolve_chat_id` and
    /// `build_payload` already have.
    ///
    /// `Some(0)` is refused as a route: `chat_id == 0` is the documented
    /// "no reply channel" sentinel that `send()` short-circuits on before any
    /// POST (mika#650/#1090), so an escalation chat set to `0` would swallow
    /// every P0 silently — the exact failure mode mika#1381 closes. Depuis
    /// mika#2871 ce refus est porté par [`p0_route_is_usable`], que la garde de
    /// démarrage lit aussi — une seconde écriture du prédicat serait libre de
    /// divergir en silence.
    fn resolve_notification_target(
        &self,
        severity: NotificationSeverity,
        normal_chat_id: i64,
    ) -> (i64, TargetKind) {
        match severity {
            NotificationSeverity::P2 => (normal_chat_id, TargetKind::Normal),
            NotificationSeverity::P0 => {
                if p0_route_is_usable(self.escalation_chat_id) {
                    (
                        self.escalation_chat_id.unwrap_or(normal_chat_id),
                        TargetKind::Escalation,
                    )
                } else {
                    (normal_chat_id, TargetKind::EscalationUnsetFallback)
                }
            }
        }
    }

    async fn resolve_chat_id(&self) -> Result<i64> {
        match self.chat_id {
            Some(id) => Ok(id),
            None => self
                .db
                .get_customer_config("chat_id")
                .await?
                .ok_or_else(|| anyhow!("chat_id not configured — no Telegram pairing yet"))?
                .parse::<i64>()
                .map_err(|e| anyhow!("invalid chat_id: {e}")),
        }
    }

    /// Build the `/send` request payload. In multi-bot gateway mode (`customer_id`
    /// is `Some`), the `customer_id` field is included so the gateway can resolve
    /// the per-customer bot token (#1607). In single-bot/CLI mode (`None`), the
    /// field is omitted — the gateway's `SendPayload.customer_id` defaults to
    /// `None` and it falls back to its global single-bot path. Extracted from
    /// `send()` so the payload contract is unit-testable without an HTTP round-trip.
    fn build_payload(&self, chat_id: i64, text: &str) -> serde_json::Value {
        let mut payload = serde_json::json!({
            "chat_id": chat_id,
            "text": text,
            "request_id": self.request_id,
            "agent_name": self.agent_name,
        });

        if let Some(customer_id) = &self.customer_id {
            payload["customer_id"] = serde_json::json!(customer_id);
        }

        payload
    }

    async fn try_send(&self, payload: &serde_json::Value) -> Result<()> {
        let resp = self
            .client
            .post(format!("{}/send", self.gateway_url))
            .bearer_auth(self.internal_token.expose_secret())
            .json(payload)
            .timeout(Duration::from_secs(10))
            .send()
            .await
            .map_err(Self::classify_reqwest_error)?;

        if resp.status().is_success() {
            Ok(())
        } else {
            let status = resp.status();
            let body_snippet = resp
                .text()
                .await
                .unwrap_or_default()
                .chars()
                .take(200)
                .collect::<String>();
            if body_snippet.is_empty() {
                anyhow::bail!("gateway /send returned {status}")
            } else {
                anyhow::bail!("gateway /send returned {status}: {body_snippet}")
            }
        }
    }

    /// Classify a reqwest transport error into a human-readable message.
    fn classify_reqwest_error(e: reqwest::Error) -> anyhow::Error {
        if e.is_connect() {
            anyhow::anyhow!("gateway unreachable (connection error): {e}")
        } else if e.is_timeout() {
            anyhow::anyhow!("gateway request timed out: {e}")
        } else {
            anyhow::anyhow!("gateway request failed: {e}")
        }
    }

    /// Deliver `text` and report the outcome — **without any side effect on
    /// `failed_sends`** (mika#2935).
    ///
    /// Resolves the chat, routes the severity, short-circuits on the
    /// `NoChannel` sentinel, POSTs and retries once after 2s. Everything
    /// `send_with_severity` used to do, minus the parking, which is why a
    /// caller that already owns a `failed_sends` row — the flush — can use it
    /// without the row being duplicated underneath it.
    pub async fn try_deliver_with_severity(
        &self,
        text: &str,
        severity: NotificationSeverity,
    ) -> Result<SendOutcome> {
        let normal_chat_id = self.resolve_chat_id().await?;
        let (chat_id, target) = self.resolve_notification_target(severity, normal_chat_id);

        // A P0 that found no escalation chat must never fall back in silence —
        // that is the "alarm into the void" this work exists to close. One named
        // WARN, greppable, carrying the chat it fell back to.
        if target == TargetKind::EscalationUnsetFallback {
            warn!(
                event = "operator_escalation_chat_unset",
                agent_name = ?self.agent_name,
                request_id = ?self.request_id,
                severity = severity.as_str(),
                fallback_chat_id = chat_id,
                "operator_escalation_chat_unset: P0 notification falls back to the \
                 normal chat — set MIKA_OPERATOR_ESCALATION_CHAT_ID to a non-zero \
                 chat so breakage alarms survive a mute"
            );
        }

        // chat_id == 0 is the documented sentinel for "no reply channel" (GitHub
        // webhooks, non-Telegram sessions). Short-circuit before the HTTP POST —
        // no retry, no failed_sends entry, since this is a permanent condition.
        if chat_id == 0 {
            // Operator-visible: message content that was lost. Truncated to 500 chars
            // because the operator needs to assess severity of the lost escalation,
            // and most escalation messages fit within this budget.
            error!(
                agent_name = ?self.agent_name,
                request_id = ?self.request_id,
                message_text = %truncate_for_log(text, 500),
                severity = severity.as_str(),
                "send_message_nochannel: message lost — chat_id=0, no reply channel available"
            );
            return Ok(SendOutcome::NoChannel);
        }

        tracing::debug!(
            agent_name = ?self.agent_name,
            chat_id,
            text_len = text.len(),
            severity = severity.as_str(),
            target = ?target,
            "GatewayMessageSender: sending outbound message"
        );

        let payload = self.build_payload(chat_id, text);

        // First attempt
        match self.try_send(&payload).await {
            Ok(()) => return Ok(SendOutcome::Delivered),
            Err(e) => warn!(error = %e, "first /send attempt failed, retrying in 2s"),
        }

        // Retry after 2s
        tokio::time::sleep(Duration::from_secs(2)).await;
        match self.try_send(&payload).await {
            Ok(()) => Ok(SendOutcome::Delivered),
            Err(e) => {
                // Says what this function did, and nothing it did not: the
                // parking happens one level up, in `send_with_severity`
                // (mika#2935). The former wording — "saving to failed_sends" —
                // is what made a failed flush testify to a parking that had
                // not happened on this path.
                warn!(error = %e, "both /send attempts failed");
                Ok(SendOutcome::Failed {
                    reason: e.to_string(),
                })
            }
        }
    }
}

#[async_trait]
impl MessageSender for GatewayMessageSender {
    /// Delegates to [`Self::send_with_severity`] with `P2` (mika#1381).
    ///
    /// This direction — and not the reverse — is what guarantees the two paths
    /// cannot drift: there is exactly one copy of the resolve/payload/retry/
    /// failed_sends sequence, and `P2` resolves to the same chat `send()`
    /// always used.
    async fn send(&self, text: &str) -> Result<SendOutcome> {
        self.send_with_severity(text, NotificationSeverity::P2)
            .await
    }

    /// Deliver, then park on failure: retries once after 2s, and if both
    /// attempts fail, saves to the `failed_sends` table for later flush and
    /// returns `Ok(SendOutcome::Failed)` so the caller can surface the delivery
    /// failure to the LLM.
    ///
    /// **The parking belongs to whoever owns no row** (mika#2935). A normal
    /// turn owns none, so it parks here. The flush owns one, so it decides that
    /// row's fate itself (`delete` or `increment`) and goes through
    /// [`Self::try_deliver_with_severity`] instead — otherwise a failed flush
    /// keeps its row *and* gets a second, already-prefixed one from this
    /// clause, one extra `⏳ from earlier — ` per pass.
    async fn send_with_severity(
        &self,
        text: &str,
        severity: NotificationSeverity,
    ) -> Result<SendOutcome> {
        let outcome = self.try_deliver_with_severity(text, severity).await?;
        if let SendOutcome::Failed { .. } = &outcome {
            // Site UNIQUE de parquage du crate (mika#2935). Voir le détecteur
            // `mika2935_le_parquage_a_un_seul_site`.
            warn!(
                event = "failed_send_parked",
                agent_name = ?self.agent_name,
                request_id = ?self.request_id,
                text_len = text.len(),
                "failed_send_parked: saving the undelivered text to failed_sends"
            );
            // Capture the delivery failure reason before attempting DB write.
            // If save_failed_send fails, we still surface the original gateway
            // error to the caller (not the DB error).
            if let Err(db_err) = self.db.save_failed_send(text, None).await {
                warn!(error = %db_err, "failed to save to failed_sends table");
            }
        }
        Ok(outcome)
    }
}

/// The flush's delivery rail (mika#2935): the severity `send()` already
/// resolved on this path, so routing does not move — only the parking does.
#[async_trait]
impl FlushDelivery for GatewayMessageSender {
    async fn try_deliver(&self, text: &str) -> Result<SendOutcome> {
        self.try_deliver_with_severity(text, NotificationSeverity::P2)
            .await
    }
}

/// A no-op message sender that silently accepts all messages.
///
/// Used to suppress user-facing notifications in contexts where the silent
/// turn should still run but not produce outbound messages (e.g., team-child
/// callback turns where the consolidated team notification handles delivery).
pub struct NoopSender;

#[async_trait]
impl MessageSender for NoopSender {
    async fn send(&self, _text: &str) -> Result<SendOutcome> {
        Ok(SendOutcome::Delivered)
    }
}

/// Returns a UTF-8-safe prefix of `text` up to `max_chars` characters.
/// Uses `char_indices()` to find the byte boundary at the character limit,
/// avoiding panics on multi-byte UTF-8 when truncating.
fn truncate_for_log(text: &str, max_chars: usize) -> &str {
    match text.char_indices().nth(max_chars) {
        Some((byte_idx, _)) => &text[..byte_idx],
        None => text,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::test_helpers::TestHarness;

    #[tokio::test]
    async fn test_resolve_chat_id_explicit_override() {
        // Delegate agent DB has no chat_id — but explicit override is set
        let harness = TestHarness::with_agent("delegate-agent");
        let sender = GatewayMessageSender::new(
            "http://localhost:9999".to_string(),
            SecretString::from("test-token"),
            harness.db.clone(),
            reqwest::Client::new(),
            None,
            Some("delegate-agent".to_string()),
            Some(12345),
            None,
        );

        let chat_id = sender.resolve_chat_id().await.unwrap();
        assert_eq!(chat_id, 12345);
    }

    #[tokio::test]
    async fn test_resolve_chat_id_db_fallback() {
        let harness = TestHarness::new();
        harness
            .db
            .set_customer_config("chat_id", "67890")
            .await
            .unwrap();

        let sender = GatewayMessageSender::new(
            "http://localhost:9999".to_string(),
            SecretString::from("test-token"),
            harness.db.clone(),
            reqwest::Client::new(),
            None,
            Some("mika".to_string()),
            None,
            None,
        );

        let chat_id = sender.resolve_chat_id().await.unwrap();
        assert_eq!(chat_id, 67890);
    }

    #[tokio::test]
    async fn test_resolve_chat_id_no_override_no_db_fails() {
        // Neither explicit nor DB — should fail
        let harness = TestHarness::with_agent("delegate-agent");
        let sender = GatewayMessageSender::new(
            "http://localhost:9999".to_string(),
            SecretString::from("test-token"),
            harness.db.clone(),
            reqwest::Client::new(),
            None,
            Some("delegate-agent".to_string()),
            None,
            None,
        );

        let err = sender.resolve_chat_id().await.unwrap_err();
        assert!(
            err.to_string().contains("chat_id not configured"),
            "unexpected error: {err}"
        );
    }

    /// Simulates the Telegram→GitHub→resolve sequence from #580:
    /// 1. Telegram message stores chat_id=99999
    /// 2. GitHub message with no chat_id must NOT overwrite it
    /// 3. resolve_chat_id returns the original 99999
    #[tokio::test]
    async fn test_github_message_does_not_poison_chat_id() {
        let harness = TestHarness::new();

        // Step 1: Telegram message stores the real chat_id
        harness
            .db
            .set_customer_config("chat_id", "99999")
            .await
            .unwrap();

        // Step 2: GitHub message arrives — handler would see chat_id=None and skip storage.
        // (The handler guard is `if let Some(chat_id) = req.chat_id && chat_id != 0`.)
        // We simulate by NOT writing to customer_config.

        // Step 3: resolve_chat_id should return the original Telegram chat_id
        let sender = GatewayMessageSender::new(
            "http://localhost:9999".to_string(),
            SecretString::from("test-token"),
            harness.db.clone(),
            reqwest::Client::new(),
            None,
            Some("mika".to_string()),
            None,
            None,
        );

        let chat_id = sender.resolve_chat_id().await.unwrap();
        assert_eq!(
            chat_id, 99999,
            "GitHub message must not overwrite Telegram chat_id"
        );
    }

    /// Verifies that a poisoned chat_id ("0") in the DB is parsed as 0.
    /// `resolve_chat_id()` returns the raw value — the `send()` method
    /// catches `chat_id == 0` and returns `SendOutcome::NoChannel` before
    /// the HTTP POST (#650).
    #[tokio::test]
    async fn test_resolve_chat_id_poisoned_zero_in_db() {
        let harness = TestHarness::new();
        harness
            .db
            .set_customer_config("chat_id", "0")
            .await
            .unwrap();

        let sender = GatewayMessageSender::new(
            "http://localhost:9999".to_string(),
            SecretString::from("test-token"),
            harness.db.clone(),
            reqwest::Client::new(),
            None,
            Some("mika".to_string()),
            None,
            None,
        );

        let chat_id = sender.resolve_chat_id().await.unwrap();
        assert_eq!(
            chat_id, 0,
            "poisoned value returns 0 — send() catches this as NoChannel"
        );
    }

    /// chat_id == 0 via explicit override returns NoChannel without HTTP POST (#650).
    #[tokio::test]
    async fn test_send_chat_id_zero_explicit_returns_no_channel() {
        let harness = TestHarness::with_agent("test-agent");
        let sender = GatewayMessageSender::new(
            "http://localhost:9999".to_string(),
            SecretString::from("test-token"),
            harness.db.clone(),
            reqwest::Client::new(),
            None,
            Some("test-agent".to_string()),
            Some(0), // explicit chat_id override of 0
            None,
        );

        let outcome = sender.send("hello").await.unwrap();
        assert_eq!(
            outcome,
            SendOutcome::NoChannel,
            "chat_id=0 should return NoChannel, not attempt HTTP POST"
        );
    }

    /// chat_id == 0 from DB (poisoned value) returns NoChannel via send() (#650).
    #[tokio::test]
    async fn test_send_chat_id_zero_from_db_returns_no_channel() {
        let harness = TestHarness::new();
        harness
            .db
            .set_customer_config("chat_id", "0")
            .await
            .unwrap();

        let sender = GatewayMessageSender::new(
            "http://localhost:9999".to_string(),
            SecretString::from("test-token"),
            harness.db.clone(),
            reqwest::Client::new(),
            None,
            Some("mika".to_string()),
            None, // no explicit override — falls back to DB
            None,
        );

        let outcome = sender.send("hello").await.unwrap();
        assert_eq!(
            outcome,
            SendOutcome::NoChannel,
            "poisoned chat_id=0 from DB should return NoChannel"
        );
    }

    #[tokio::test]
    async fn test_noop_sender_returns_delivered() {
        let sender = NoopSender;
        let outcome = sender.send("hello").await.unwrap();
        assert_eq!(outcome, SendOutcome::Delivered);
    }

    /// resolve_chat_id() returning Err (no chat_id configured) still propagates
    /// as Err, not NoChannel.
    #[tokio::test]
    async fn test_send_no_chat_id_configured_returns_err() {
        let harness = TestHarness::with_agent("no-chat-agent");
        let sender = GatewayMessageSender::new(
            "http://localhost:9999".to_string(),
            SecretString::from("test-token"),
            harness.db.clone(),
            reqwest::Client::new(),
            None,
            Some("no-chat-agent".to_string()),
            None, // no explicit override, no DB entry
            None,
        );

        let result = sender.send("hello").await;
        assert!(
            result.is_err(),
            "missing chat_id should return Err, not NoChannel"
        );
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("chat_id not configured"),
            "error should mention chat_id not configured"
        );
    }

    // --- customer_id payload tests (mika#1607) ---

    /// Multi-bot mode: when `customer_id` is `Some`, the `/send` payload includes
    /// it so the gateway can resolve the per-customer bot token. The fixture uses a
    /// real UUID string because that is the production wire shape — the gateway's
    /// `SendPayload.customer_id: Option<Uuid>` deserializes the JSON string back
    /// into a `Uuid` (see mika-gateway `test_send_payload_with_customer_id`).
    #[tokio::test]
    async fn test_send_payload_includes_customer_id() {
        let customer_uuid = "12345678-1234-1234-1234-123456789abc";
        let harness = TestHarness::with_agent("customer-agent");
        let sender = GatewayMessageSender::new(
            "http://localhost:9999".to_string(),
            SecretString::from("test-token"),
            harness.db.clone(),
            reqwest::Client::new(),
            None,
            Some("customer-agent".to_string()),
            Some(12345),
            Some(customer_uuid.to_string()),
        );

        let payload = sender.build_payload(12345, "hello");
        assert_eq!(
            payload["customer_id"],
            serde_json::json!(customer_uuid),
            "multi-bot payload must carry customer_id"
        );
        // The emitted value must parse as a Uuid — the gateway deserializes the
        // JSON string into Option<Uuid>, so a non-UUID would 400 at the gateway.
        assert!(
            uuid::Uuid::parse_str(payload["customer_id"].as_str().unwrap()).is_ok(),
            "customer_id must be a valid UUID on the wire"
        );
        assert_eq!(payload["chat_id"], serde_json::json!(12345));
        assert_eq!(payload["text"], serde_json::json!("hello"));
    }

    /// Backward compatibility: when `customer_id` is `None` (single-bot/CLI), the
    /// payload omits the key entirely — the gateway falls back to single-bot mode.
    #[tokio::test]
    async fn test_send_payload_omits_customer_id_when_none() {
        let harness = TestHarness::with_agent("single-bot-agent");
        let sender = GatewayMessageSender::new(
            "http://localhost:9999".to_string(),
            SecretString::from("test-token"),
            harness.db.clone(),
            reqwest::Client::new(),
            None,
            Some("single-bot-agent".to_string()),
            Some(12345),
            None,
        );

        let payload = sender.build_payload(12345, "hello");
        assert!(
            payload.get("customer_id").is_none(),
            "single-bot payload must NOT include customer_id key"
        );
        assert_eq!(payload["chat_id"], serde_json::json!(12345));
        assert_eq!(payload["text"], serde_json::json!("hello"));
    }

    // --- truncate_for_log tests (mika#1090) ---

    #[test]
    fn test_truncate_for_log_empty() {
        assert_eq!(truncate_for_log("", 500), "");
    }

    #[test]
    fn test_truncate_for_log_shorter_than_limit() {
        assert_eq!(truncate_for_log("hello", 500), "hello");
    }

    #[test]
    fn test_truncate_for_log_exactly_at_limit() {
        let text = "a".repeat(500);
        assert_eq!(truncate_for_log(&text, 500), text.as_str());
    }

    #[test]
    fn test_truncate_for_log_longer_than_limit() {
        let text = "a".repeat(600);
        let result = truncate_for_log(&text, 500);
        assert_eq!(result.len(), 500);
        assert_eq!(result, &text[..500]);
    }

    #[test]
    fn test_truncate_for_log_multibyte_utf8() {
        // Each emoji is 4 bytes. 3 emojis = 12 bytes, 3 chars.
        let text = "🔥🔥🔥abc";
        // Truncate at 3 chars — should get exactly the 3 emojis (12 bytes)
        let result = truncate_for_log(text, 3);
        assert_eq!(result, "🔥🔥🔥");
        // Truncate at 5 chars — should get 3 emojis + "ab"
        let result = truncate_for_log(text, 5);
        assert_eq!(result, "🔥🔥🔥ab");
    }

    #[test]
    fn test_truncate_for_log_cjk_characters() {
        // CJK characters are 3 bytes each
        let text = "你好世界abcd";
        // Truncate at 4 chars
        let result = truncate_for_log(text, 4);
        assert_eq!(result, "你好世界");
    }

    #[test]
    fn test_truncate_for_log_zero_limit() {
        assert_eq!(truncate_for_log("hello", 0), "");
    }

    // --- severity-tiered operator notifications (mika#1381) ---
    //
    // The routing decision is asserted on `resolve_notification_target`, not
    // through `send()`: the selector is pure past the already-resolved normal
    // chat, so these four cases need no HTTP server and no gateway — the same
    // property `resolve_chat_id` and `build_payload` are tested on above.

    /// Fixture carrying only what the selector reads. `chat_id` is the explicit
    /// override so `resolve_chat_id` is deterministic, even though these tests
    /// pass the normal chat in directly.
    fn sender_with_escalation(escalation: Option<i64>) -> GatewayMessageSender {
        let harness = TestHarness::with_agent("severity-agent");
        GatewayMessageSender::new(
            "http://localhost:9999".to_string(),
            SecretString::from("test-token"),
            harness.db.clone(),
            reqwest::Client::new(),
            None,
            Some("severity-agent".to_string()),
            Some(111),
            None,
        )
        .with_escalation_chat_id(escalation)
    }

    /// AC2 — a `P0` leaves for the escalation chat when one is armed.
    #[test]
    fn mika1381_un_p0_part_sur_le_chat_descalade() {
        let sender = sender_with_escalation(Some(222));
        assert_eq!(
            sender.resolve_notification_target(NotificationSeverity::P0, 111),
            (222, TargetKind::Escalation),
            "un P0 doit quitter le chat normal pour le chat d'escalade"
        );
    }

    /// AC3 — a `P2` ignores the escalation chat even when one is armed. This is
    /// the half that makes the mute usable: a P2 routed to the never-muted
    /// surface would re-create the problem on the other side.
    #[test]
    fn mika1381_un_p2_ignore_le_chat_descalade() {
        let sender = sender_with_escalation(Some(222));
        assert_eq!(
            sender.resolve_notification_target(NotificationSeverity::P2, 111),
            (111, TargetKind::Normal),
            "un P2 reste sur le chat normal, chat d'escalade armé ou non"
        );
    }

    /// AC4 — a `P0` with no escalation chat falls back to the normal chat and is
    /// marked as such. Deliberately a fallback and not a drop: a deployment with
    /// a single channel is the nominal state (D4), and losing the alarm would be
    /// worse than delivering it to a chat the operator may have muted.
    #[test]
    fn mika1381_un_p0_sans_chat_descalade_retombe_sur_le_chat_normal() {
        let sender = sender_with_escalation(None);
        assert_eq!(
            sender.resolve_notification_target(NotificationSeverity::P0, 111),
            (111, TargetKind::EscalationUnsetFallback),
            "sans second canal, le P0 retombe sur le chat normal — et le dit"
        );
    }

    /// AC5 — `Some(0)` is refused as a route. `chat_id == 0` is the documented
    /// "no reply channel" sentinel `send()` short-circuits on, so honouring it
    /// as an escalation chat would swallow every P0 in silence.
    #[test]
    fn mika1381_un_chat_descalade_a_zero_est_refuse_comme_route() {
        let sender = sender_with_escalation(Some(0));
        assert_eq!(
            sender.resolve_notification_target(NotificationSeverity::P0, 111),
            (111, TargetKind::EscalationUnsetFallback),
            "Some(0) vaut « pas de chat d'escalade », jamais « route vers 0 »"
        );
    }

    /// AC6 — the trait's default method ignores the severity and delegates to
    /// `send`. This is what lets every rail without a second channel (the CLI,
    /// `NoopSender`, ~24 `impl MessageSender` blocks across `src/` and
    /// `tests/eval/`) stay untouched by mika#1381.
    #[tokio::test]
    async fn mika1381_le_defaut_du_trait_ignore_la_severite() {
        /// Records what reached `send`, so the test asserts delegation rather
        /// than merely a successful outcome.
        struct RecordingSender(std::sync::Mutex<Vec<String>>);

        #[async_trait]
        impl MessageSender for RecordingSender {
            async fn send(&self, text: &str) -> Result<SendOutcome> {
                self.0.lock().unwrap().push(text.to_string());
                Ok(SendOutcome::Delivered)
            }
        }

        let sender = RecordingSender(std::sync::Mutex::new(Vec::new()));
        for severity in [NotificationSeverity::P0, NotificationSeverity::P2] {
            assert_eq!(
                sender.send_with_severity("alarme", severity).await.unwrap(),
                SendOutcome::Delivered
            );
        }

        assert_eq!(
            *sender.0.lock().unwrap(),
            vec!["alarme".to_string(), "alarme".to_string()],
            "le défaut du trait passe le texte à send(), quelle que soit la sévérité"
        );
    }

    /// AC1 à la racine, et AC2 comme contrôle négatif dans le même test
    /// (mika#2935).
    ///
    /// La paire est ce qui distingue « le parquage a été déplacé » de « le
    /// parquage a été supprimé » : un seul des deux appels suffirait à l'un des
    /// deux énoncés, jamais aux deux.
    ///
    /// Le point de terminaison vient de [`mika_common::dead_endpoint::DeadEndpoint`],
    /// tenu pour toute la durée du test — jamais un `bind(:0)` suivi d'un
    /// `drop` (mika#2569 : ce binaire est le plus parallèle de l'arbre, et un
    /// port libéré est repris par le voisin, qui répond alors à l'appel censé
    /// échouer).
    #[tokio::test]
    async fn mika2935_le_parquage_appartient_a_send_jamais_a_la_tentative_nue() {
        let dead = mika_common::dead_endpoint::DeadEndpoint::reserve();
        let harness = TestHarness::new();
        harness
            .db
            .set_customer_config("chat_id", "4242")
            .await
            .unwrap();

        let sender = GatewayMessageSender::new(
            format!("http://{}", dead.addr()),
            SecretString::from("test-token"),
            harness.db.clone(),
            reqwest::Client::new(),
            None,
            Some("mika".to_string()),
            None,
            None,
        );

        // 1. La primitive échoue et n'écrit pas. C'est AC1 à la racine : le
        //    chemin que le flush emprunte ne peut pas créer de ligne.
        let outcome = FlushDelivery::try_deliver(&sender, "réponse parquée")
            .await
            .unwrap();
        assert!(
            matches!(outcome, SendOutcome::Failed { .. }),
            "un point de terminaison mort doit rendre Failed, pas {outcome:?}"
        );
        let parked = harness.db.get_pending_failed_sends(10).await.unwrap();
        assert!(
            parked.is_empty(),
            "try_deliver n'a aucun effet de bord sur failed_sends, or la table \
             porte {} ligne(s) : {:?}",
            parked.len(),
            parked.iter().map(|p| &p.text).collect::<Vec<_>>()
        );

        // 2. Même sender, l'entrée avec effet de bord : un tour normal parque
        //    toujours. C'est AC2 — le contrôle négatif demandé dans ce test.
        let outcome = MessageSender::send(&sender, "un tour normal")
            .await
            .unwrap();
        assert!(
            matches!(outcome, SendOutcome::Failed { .. }),
            "même transport, même issue attendue, pas {outcome:?}"
        );
        let parked = harness.db.get_pending_failed_sends(10).await.unwrap();
        assert_eq!(
            parked.len(),
            1,
            "un envoi raté hors flush parque exactement une ligne : {:?}",
            parked.iter().map(|p| &p.text).collect::<Vec<_>>()
        );
        assert_eq!(
            parked[0].text, "un tour normal",
            "la ligne parquée porte le texte de l'envoi, et seulement lui"
        );
    }

    // ---- mika#2935 / R4 — le parquage n'a qu'un site de production ---------

    /// Fichiers dispensés du détecteur de parquage unique. LIVRÉE VIDE, et
    /// c'est le contrat. `(chemin relatif à `src/`, ticket de suivi)`.
    const PARKING_SITE_ALLOWLIST: &[(&str, &str)] = &[];

    /// L'aiguille : un **appel** de `save_failed_send`, pas sa définition.
    const PARKING_NEEDLE: &str = ".save_failed_send(";

    /// Hors périmètre, et ce n'est **pas** une exception (mika#2935).
    ///
    /// - `messaging.rs` est le propriétaire autorisé : le fichier qui porte le
    ///   seul site de parquage, et ce détecteur.
    /// - `db.rs` porte la définition et `async_db.rs` le délégué vers le fil
    ///   DB. Ces deux-là *sont* la méthode ; les compter reviendrait à déclarer
    ///   le type en violation de lui-même. (`db.rs` n'écrit aujourd'hui
    ///   `pub fn save_failed_send(`, que l'aiguille ne reconnaît pas — la
    ///   déclarer ici rend le périmètre indépendant de cette orthographe.)
    ///
    /// La distinction compte : une exclusion de périmètre décrit ce que le
    /// détecteur **mesure**, une exception décrit une dette qu'il **tolère**.
    const PARKING_SCOPE_EXCLUSIONS: &[&str] = &["messaging.rs", "db.rs", "async_db.rs"];

    /// `true` si `src`, lu comme la production de `path`, parque une ligne
    /// `failed_sends` alors que ce n'est pas son rôle (mika#2935).
    ///
    /// Extrait du test porteur pour que le contrôle de bonne foi puisse le
    /// nourrir d'une source synthétique sans écrire sur le disque — une
    /// allowlist vide au-dessus d'un scan qui ne voit rien serait verte pour la
    /// mauvaise raison.
    fn is_unexpected_parker(path: &std::path::Path, src: &str) -> bool {
        if crate::source_scan::is_test_source_path(path) {
            return false;
        }
        let rel = path.to_string_lossy().replace('\\', "/");
        if PARKING_SCOPE_EXCLUSIONS.contains(&rel.as_str()) {
            return false;
        }
        if PARKING_SITE_ALLOWLIST.iter().any(|(p, _)| *p == rel) {
            return false;
        }
        let prod = crate::source_scan::production_half(src);
        crate::source_scan::strip_comment_lines(prod).contains(PARKING_NEEDLE)
    }

    /// La correction de mika#2935 est locale ; la classe ne l'est pas. Un futur
    /// site qui parque de son côté — ou qui rebranche le flush sur `send` —
    /// réinstalle le défaut sans rougir aucune assertion de comportement.
    #[test]
    fn mika2935_le_parquage_a_un_seul_site() {
        let files = crate::source_scan::production_files();
        assert!(
            files.len() > 50,
            "le parcours de `src/` n'a rendu que {} fichiers : un scan qui ne \
             trouve plus rien doit être rouge, pas vert",
            files.len()
        );

        let coupables: Vec<String> = files
            .iter()
            .filter(|(rel, prod)| is_unexpected_parker(std::path::Path::new(rel), prod))
            .map(|(rel, _)| rel.clone())
            .collect();

        assert!(
            coupables.is_empty(),
            "ces fichiers de production parquent une ligne `failed_sends` alors \
             que `send_with_severity` est le site unique : {coupables:?}. Un \
             second site rouvre exactement mika#2935 — la ligne ratée est \
             conservée par son propriétaire ET recréée par le transport, avec \
             un préfixe de plus à chaque passe. Retirez le second site, ne \
             l'allowlistez pas : routez-le par `send_with_severity`, ou passez \
             par `try_deliver_with_severity` si l'appelant possède déjà sa ligne."
        );
    }

    /// L'assertion auto-nettoyante de la Fire-Disposition (mika#2309) :
    /// l'exception expire par le geste qui la rend inutile.
    #[test]
    fn mika2935_lallowlist_du_parqueur_unique_est_livree_vide_et_le_reste() {
        assert!(
            PARKING_SITE_ALLOWLIST.is_empty(),
            "Une entrée a été ajoutée ici. Un second site de parquage rouvre \
             exactement mika#2935 : la ligne ratée est conservée par son \
             propriétaire ET recréée par le transport, avec un préfixe de plus \
             à chaque passe. Route le site par `send_with_severity`, ou ouvre \
             un ticket de suivi et nomme-le dans l'entrée."
        );
    }

    /// Le contrôle de bonne foi : le détecteur rougit bel et bien sur un second
    /// parqueur, et reste muet sur une fixture de test (mika#2935).
    ///
    /// Sans lui, `mika2935_le_parquage_a_un_seul_site` serait vert aussi bien
    /// parce qu'il n'y a qu'un site que parce qu'il ne sait pas en voir deux.
    #[test]
    fn mika2935_le_detecteur_rougit_sur_un_second_parqueur() {
        let second_parqueur = "\
async fn flush(db: &AsyncDatabase, text: &str) {
    let _ = db.save_failed_send(text, None).await;
}
";

        // Un chemin de production réel — celui du flush, qui est précisément
        // l'endroit où le second site vivait avant R3.
        let prod = std::path::Path::new("server/handlers.rs");
        assert!(
            is_unexpected_parker(prod, second_parqueur),
            "le détecteur ne voit pas un second site de parquage : \
             `mika2935_le_parquage_a_un_seul_site` est décoratif"
        );

        // La même source sous un chemin de test : une fixture pose
        // légitimement l'aiguille, et l'accuser rougirait sur du code sain.
        assert!(
            !is_unexpected_parker(
                std::path::Path::new("server/tests/flush_fixtures.rs"),
                second_parqueur
            ),
            "une fixture de test n'est pas un site de parquage de production"
        );
        assert!(
            !is_unexpected_parker(std::path::Path::new("server/tests.rs"), second_parqueur),
            "un `tests.rs` isolé non plus (mika#2321)"
        );

        // Et le propriétaire autorisé reste hors de cause, par périmètre.
        assert!(
            !is_unexpected_parker(std::path::Path::new("messaging.rs"), second_parqueur),
            "`messaging.rs` porte le site unique : c'est une exclusion de \
             périmètre, pas une exception"
        );
    }
}
