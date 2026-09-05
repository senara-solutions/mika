//! Integration tests for bounded, observable callback delivery (mika#2179).
//!
//! The night of 2026-09-03/04, callback `800d739f-a0ed-485d-bef1-9990beeac396`
//! completed at `22:03:24Z` and was delivered at `03:09:16Z` — **5 h 06** later.
//! Its parent (`ready-label: mika#2140`) died `phantom_aged_out` at `02:04:01Z`,
//! an hour and five minutes *before* its own pilot's return was ever read. In
//! between, 19 `resume_agent run failed` events fired on an LLM transport
//! timeout, each holding the agent lock for up to `AGENT_TOTAL_TIMEOUT_SECS`
//! (300s) and leaving the row `status='completed'` so the 60s scan
//! (`DB_SCAN_INTERVAL_TICKS`) picked it up again. No counter, no audit event,
//! no bound: the only stop condition in the old code was the LLM eventually
//! succeeding.
//!
//! These tests pin both halves of the fix:
//!
//! * the failing path **speaks** — one `callback_delivery_failed` audit event
//!   per attempt, carrying the error class and the attempt rank (AC1);
//! * the failing path **stops taking the slot every minute** — `next_fire_at`
//!   grows exponentially and a `callback_delivery_quarantined` event fires once
//!   at the threshold (AC3);
//! * the succeeding path is **unchanged in behaviour** and now measurable —
//!   `callback_delivered` carries `wait_secs`, and no backoff is written
//!   (AC2, AC5 negative control).
//!
//! The starved row is seeded under the ticket's own id and its real 18 352 s
//! wait, so a reader meets the incident rather than a synthetic fixture.
//!
//! Injection-verification recipe (MANDATORY, per plan Phase C2): comment out the
//! `log_audit_event` call inside
//! `TaskDispatcher::record_callback_delivery_failure` and re-run this file — the
//! `callback_delivery_failed` count assertions MUST fail. Restore, re-run, they
//! pass. Same for the `update_task_next_fire_at` call in that function and the
//! `next_fire_at` growth assertions. The tests thereby prove those write paths
//! are load-bearing rather than incidental.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use mika_agent::async_db::AsyncDatabase;
use mika_agent::db::{Database, NewTask};
use mika_agent::messaging::{MessageSender, SendOutcome};
use mika_agent::skills::SkillRegistry;
use mika_agent::task_engine::dispatcher::TaskDispatcher;
use mika_agent::tools::default_tools;
use mika_common::llm::LlmProvider;
use mika_common::llm::error::LlmError;
use mika_common::llm::mock::{MockLlmProvider, text_response};
use tempfile::TempDir;

const AGENT_ID: &str = "mika";

/// The ticket's own callback id — `800d739f`, the max of the 24 h distribution
/// measured at grooming (plan M2), not a representative sample.
const STARVED_TASK_ID: &str = "800d739f-a0ed-485d-bef1-9990beeac396";

/// `2026-09-03T22:03:24Z` → `2026-09-04T03:09:16Z`, the measured 5 h 06.
const STARVED_WAIT_SECS: i64 = 18_352;

/// The exact transport error from `/var/log/mika/server.log`, produced on the
/// openrouter / `z-ai/glm-5.3` path by `crates/mika-common/src/llm/openai.rs`.
const TRANSPORT_TIMEOUT_MSG: &str = "failed to read response body: error decoding response body: \
     request or response body error: operation timed out";

/// No-op sender: a callback turn that reached `send_message` would be a
/// different failure than the one under test, and this keeps it inert.
struct NoopSender;

#[async_trait::async_trait]
impl MessageSender for NoopSender {
    async fn send(&self, _text: &str) -> anyhow::Result<SendOutcome> {
        Ok(SendOutcome::Delivered)
    }
}

fn test_db() -> AsyncDatabase {
    let db = Database::open_in_memory().expect("open in-memory DB");
    AsyncDatabase::new_with_agent(db, AGENT_ID)
}

/// Minimal agent home — `load_agent_context` reads `soul.md` and the identity
/// files and tolerates their absence, but the directory itself must exist.
fn test_home() -> TempDir {
    let home = TempDir::new().expect("tmp home");
    std::fs::create_dir_all(home.path().join("skills")).expect("skills dir");
    std::fs::create_dir_all(home.path().join("data")).expect("data dir");
    std::fs::write(home.path().join("soul.md"), "").expect("soul.md");
    home
}

/// Build a dispatcher whose LLM fails `n` times with the production transport
/// timeout. `cli_mode: false` is load-bearing — the callback delivery path is
/// server-mode only. `agent_lock: None` lets the test drive attempts back to
/// back without modelling contention, which is a separate axis.
fn starving_dispatcher(db: AsyncDatabase, home: &TempDir, n: usize) -> Arc<TaskDispatcher> {
    let mut builder = MockLlmProvider::builder();
    for _ in 0..n {
        builder = builder.error(LlmError::Transport(TRANSPORT_TIMEOUT_MSG.to_string()));
    }
    dispatcher_with_llm(db, home, Arc::new(builder.build()))
}

/// The negative control's dispatcher: plain text responses, no error.
///
/// Two responses, not one: a callback turn that ends on text alone is rejected
/// once by the `callback_terminal_action` EndTurn guard (position 6e), which
/// wants `update_task_status` + `send_message`. The guard's single-retry budget
/// then accepts the second. That is existing behaviour of the happy path and
/// exactly what AC5 asks us not to disturb — the fixture models it rather than
/// working around it.
fn delivering_dispatcher(db: AsyncDatabase, home: &TempDir) -> Arc<TaskDispatcher> {
    let llm = MockLlmProvider::builder()
        .responses(vec![
            text_response("Acknowledged the pilot result."),
            text_response("Acknowledged the pilot result."),
        ])
        .build();
    dispatcher_with_llm(db, home, Arc::new(llm))
}

fn dispatcher_with_llm(
    db: AsyncDatabase,
    home: &TempDir,
    llm: Arc<dyn LlmProvider>,
) -> Arc<TaskDispatcher> {
    Arc::new(TaskDispatcher {
        db,
        tier: mika_common::home::AgentTier::Default,
        llm,
        tools: Arc::new(default_tools()),
        skills: Arc::new(SkillRegistry::empty()),
        message_sender: Some(Arc::new(NoopSender)),
        home_dir: PathBuf::from(home.path()),
        embedding_client: None,
        brave_api_key: None,
        gateway_url: None,
        internal_token: None,
        github_token: None,
        github_app: None,
        skills_dirty: Arc::new(AtomicBool::new(false)),
        agent_lock: None,
        cli_mode: false,
        settings: mika_common::config::Settings::test_defaults(),
        pr_reviews_posted: None,
    })
}

/// Seed the ticket's row: a `completed` `run_claude_pilot` callback carrying a
/// pilot result, its `completed_at` backdated by the measured 5 h 06, under the
/// incident's own id.
async fn seed_starved_callback(db: &AsyncDatabase) -> String {
    let id = db
        .create_task(NewTask {
            agent_id: AGENT_ID.to_string(),
            team_run_id: None,
            parent_task_id: None,
            depth: 0,
            label: "long_running:run_claude_pilot".to_string(),
            trigger_type: "callback".to_string(),
            cron_expr: None,
            event_source: None,
            event_offset_secs: None,
            condition_expr: None,
            next_fire_at: None,
            timeout_at: None,
            action_type: "resume_agent".to_string(),
            action_config: "{}".to_string(),
            input_context: None,
            created_by_session: None,
            created_trace_id: None,
            reference_url: None,
            source: Some("self_dev".to_string()),
            metadata: None,
            r#type: None,
            dispatch_class: None,
        })
        .await
        .expect("create callback row");

    db.update_task_completed(
        &id,
        Some("claude-pilot completed (status: done).\nTurns: 91"),
    )
    .await
    .expect("mark completed");
    db.set_task_id_for_test(&id, STARVED_TASK_ID)
        .await
        .expect("force the incident's id");
    db.backdate_task_completed_at(STARVED_TASK_ID, STARVED_WAIT_SECS)
        .await
        .expect("backdate completed_at to 22:03:24Z");

    STARVED_TASK_ID.to_string()
}

async fn next_fire_at_of(db: &AsyncDatabase, id: &str) -> Option<String> {
    db.get_task(id)
        .await
        .expect("read task")
        .expect("task exists")
        .next_fire_at
}

async fn status_of(db: &AsyncDatabase, id: &str) -> String {
    db.get_task(id)
        .await
        .expect("read task")
        .expect("task exists")
        .status
}

/// AC1 + AC3, the anti-vacuity replay. Three consecutive transport timeouts on
/// the ticket's own row.
///
/// On `main` every assertion below is red: the `Err` branch of
/// `run_silent_agent` writes a `warn!` and nothing else, so the audit counts
/// stay at 0 and `next_fire_at` stays NULL for ever — which is precisely why
/// the row was re-selected once a minute for five hours.
#[tokio::test]
async fn transport_timeouts_are_counted_backed_off_and_quarantined() {
    let db = test_db();
    let home = test_home();
    let dispatcher = starving_dispatcher(db.clone(), &home, 3);
    let task_id = seed_starved_callback(&db).await;

    assert_eq!(
        db.count_audit_events_by_tool_name("callback_delivery_failed")
            .await
            .unwrap(),
        0,
        "baseline: no delivery-failure events before the first attempt"
    );

    let mut backoffs: Vec<String> = Vec::new();
    for attempt in 1..=3 {
        let _ = dispatcher.dispatch(&task_id).await;
        let fire_at = next_fire_at_of(&db, &task_id).await.unwrap_or_else(|| {
            panic!("attempt {attempt} left next_fire_at NULL — the slot is still taken every scan")
        });
        backoffs.push(fire_at);
    }

    // AC1 — one named event per failed attempt.
    assert_eq!(
        db.count_audit_events_by_tool_name("callback_delivery_failed")
            .await
            .unwrap(),
        3,
        "each resume_agent failure on a callback must write one audit event"
    );

    let rows = db
        .get_audit_event_rows_by_tool_name("callback_delivery_failed")
        .await
        .unwrap();
    let (target_key, _before, after, reasoning) = &rows[0];
    assert_eq!(target_key, &format!("task:{task_id}"));
    assert_eq!(
        after.as_deref(),
        Some("transport_timeout"),
        "the error class must come from the LlmError variant, not from a message match"
    );
    let reasoning = reasoning.as_deref().unwrap_or_default();
    assert!(
        reasoning.contains("attempt:1"),
        "first event must carry the attempt rank, got: {reasoning}"
    );
    assert!(
        rows[2]
            .3
            .as_deref()
            .unwrap_or_default()
            .contains("attempt:3"),
        "the rank must advance across attempts, got: {:?}",
        rows[2].3
    );

    // AC3 — the slot is no longer retaken every scan: the backoff grows.
    assert!(
        backoffs[0] < backoffs[1] && backoffs[1] < backoffs[2],
        "next_fire_at must grow across attempts (60s → 120s → 240s), got {backoffs:?}"
    );
    assert!(
        backoffs[0].as_str() > mika_agent::timestamp::now().as_str(),
        "the first backoff must already be in the future — that is what the engine's \
         next_fire_at guard reads to skip the row"
    );

    // AC3 — the quarantine is visible, and fires once at the threshold.
    assert_eq!(
        db.count_audit_events_by_tool_name("callback_delivery_quarantined")
            .await
            .unwrap(),
        1,
        "quarantine must be announced exactly once at the threshold crossing"
    );

    // The pilot's return is never thrown away: no terminal transition.
    assert_eq!(
        status_of(&db, &task_id).await,
        "completed",
        "a quarantined callback keeps its result — marking it delivered or failed would \
         discard the pilot's return"
    );
    assert_eq!(
        db.get_task_metadata_field(&task_id, "delivery_attempts")
            .await
            .unwrap()
            .as_deref(),
        Some("3"),
        "the attempt counter must be readable without the log"
    );
}

/// AC5 negative control — without it the red battery above proves nothing.
///
/// A delivery that succeeds on the first try keeps its old behaviour (the row
/// goes `delivered`), writes **no** backoff, and now reports its own latency.
#[tokio::test]
async fn first_try_delivery_is_measured_and_otherwise_unchanged() {
    let db = test_db();
    let home = test_home();
    let dispatcher = delivering_dispatcher(db.clone(), &home);
    let task_id = seed_starved_callback(&db).await;

    dispatcher.dispatch(&task_id).await.expect("dispatch");

    assert_eq!(
        status_of(&db, &task_id).await,
        "delivered",
        "the happy path must still mark the callback delivered"
    );
    assert_eq!(
        next_fire_at_of(&db, &task_id).await,
        None,
        "a successful delivery must not write a backoff"
    );
    assert_eq!(
        db.count_audit_events_by_tool_name("callback_delivery_failed")
            .await
            .unwrap(),
        0,
        "no failure event on the happy path"
    );

    // AC2 — the latency is readable from the DB, not from the log.
    let rows = db
        .get_audit_event_rows_by_tool_name("callback_delivered")
        .await
        .unwrap();
    assert_eq!(rows.len(), 1, "one delivery, one measurement");
    let (target_key, _before, after, reasoning) = &rows[0];
    assert_eq!(target_key, &format!("task:{task_id}"));
    let wait: i64 = after
        .as_deref()
        .expect("wait_secs in after_value")
        .parse()
        .expect("wait_secs is an integer");
    assert!(
        (STARVED_WAIT_SECS..STARVED_WAIT_SECS + 300).contains(&wait),
        "wait_secs must be completed_at → now, expected ~{STARVED_WAIT_SECS}, got {wait}"
    );
    assert!(
        reasoning
            .as_deref()
            .unwrap_or_default()
            .contains("wait_secs="),
        "the reasoning line must name the measurement, got {reasoning:?}"
    );
}

/// A non-transport failure is classified as itself, not lumped into the
/// transport bucket — the classifier reads the `LlmError` variant, and a future
/// triage that greps for `transport_timeout` must not be polluted.
#[tokio::test]
async fn non_transport_failure_carries_its_own_class() {
    let db = test_db();
    let home = test_home();
    let llm = MockLlmProvider::builder()
        .error(LlmError::HttpError {
            status: 429,
            message: "rate limited".into(),
            retryable: true,
        })
        .build();
    let dispatcher = dispatcher_with_llm(db.clone(), &home, Arc::new(llm));
    let task_id = seed_starved_callback(&db).await;

    let _ = dispatcher.dispatch(&task_id).await;

    let rows = db
        .get_audit_event_rows_by_tool_name("callback_delivery_failed")
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(
        rows[0].2.as_deref(),
        Some("http_429"),
        "an HTTP failure must carry its status, not the transport class"
    );
}
