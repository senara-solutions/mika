//! mika#2517 — a Webhook Fallthrough turn is not handed `create_task`.
//!
//! **Why the production path and not the predicate.** `effective_disabled_tools`
//! has its own unit tests in `agent_loop`, and they would stay green if the
//! function were never *called* — which is the whole failure mode here: the
//! defect is not a wrong decision, it is a decision nobody consults. Only a real
//! turn, whose captured LLM request carries the tool array the model actually
//! received, closes that.
//!
//! **The two negative controls are load-bearing, not decoration.** V5 alone is
//! satisfied by a filter that strips `create_task` from *every* turn — i.e. by a
//! fix that breaks the entire autonomous loop while looking like it closes the
//! ticket. V6 (ready-label, inside the `[GitHub]` domain but outside the
//! fallthrough one) and V7 (no `[GitHub]` prefix at all) are the two axes that
//! separate "the predicate decides" from "the filter always fires".

use mika_agent::tools::{Tool, ToolContext, ToolOutput, ToolRegistry, default_tools};
use mika_common::claude::ToolDefinition;
use mika_common::llm::mock::*;
use serde_json::json;

use super::harness::EvalHarness;

/// The founding population, as the gateway emits it: an issue created with
/// three labels produces three of these, one per label applied.
const FALLTHROUGH_MSG: &str = "[GitHub] Issue labeled bug on senara-solutions/mika#2516 — title";

/// Inside the `[GitHub]` domain, **outside** the fallthrough one — the dispatch
/// path, which must keep every tool it had.
const READY_LABEL_MSG: &str = "[GitHub] Issue labeled ready on senara-solutions/mika#2516 — title";

/// Outside the `[GitHub]` domain entirely.
const PLAIN_MSG: &str = "implement mika issue#2516";

/// `create_task` is registered by `management_tools_if_needed`, which
/// `default_tools()` does not call — so a harness registry carries no tool of
/// that name and the assertions below would be vacuous. This stub supplies the
/// **name**, which is what the visibility filter matches on.
struct StubCreateTask;

#[async_trait::async_trait]
impl Tool for StubCreateTask {
    fn name(&self) -> &str {
        "create_task"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "create_task".to_string(),
            description: "Stub create_task (mika#2517 eval)".to_string(),
            input_schema: json!({"type": "object", "properties": {"label": {"type": "string"}}}),
        }
    }

    async fn execute(
        &self,
        _input: serde_json::Value,
        _ctx: &ToolContext<'_>,
    ) -> anyhow::Result<ToolOutput> {
        Ok(ToolOutput::success(
            r#"{"task_id": "00000000-0000-0000-0000-000000000042"}"#.to_string(),
        ))
    }
}

fn tools_with_create_task() -> ToolRegistry {
    let mut tools = default_tools();
    tools.register(Box::new(StubCreateTask));
    tools
}

/// Enough text responses to absorb the intent-guard re-prompts.
///
/// A ready-label turn that calls no tool is re-prompted by
/// `webhook_ready_label_dispatch` (and, off the fallthrough domain, by
/// `webhook_zero_tools`) — so it consumes more than one LLM call. That is the
/// nominal behaviour of the path V6 exercises, not a defect of the fixture; the
/// tool array is identical on every iteration of a turn, and the assertions read
/// the first captured request.
fn enough_text_responses(n: usize) -> Vec<MockResponse> {
    (0..n).map(|_| text_response("Acknowledged.")).collect()
}

/// The tool names the model was served on the first LLM call of the turn.
async fn served_tool_names(message: &str) -> Vec<String> {
    let harness = EvalHarness::builder()
        .responses(enough_text_responses(4))
        .tools(tools_with_create_task())
        .build()
        .await
        .unwrap();

    let trace = harness.run(message).await.unwrap();
    let request = trace
        .captured_requests
        .first()
        .expect("the mock provider must have captured the request");
    request
        .tools
        .as_deref()
        .unwrap_or(&[])
        .iter()
        .map(|t| t.name.clone())
        .collect()
}

/// **V5 (AC1)** — a fallthrough turn is served a tool array **without**
/// `create_task`.
///
/// Seen red against the pre-fix code: with `&ctx.identity.tools.disabled` at the
/// call site, `create_task` is present and this assertion fails.
#[tokio::test]
async fn mika2517_a_fallthrough_turn_is_not_served_create_task() {
    let names = served_tool_names(FALLTHROUGH_MSG).await;

    assert!(
        !names.iter().any(|n| n == "create_task"),
        "a Webhook Fallthrough turn must not be handed create_task — it is the \
         only 'useful' tool the turn finds under pressure, and the row it creates \
         is the phantom mika#2517 exists to stop producing. Served: {names:?}"
    );

    // The asymmetry of § 3.3 (b), asserted rather than merely stated:
    // `run_claude_pilot` is NOT hidden, because gate 0 of
    // `validate_dispatch_readiness` refusing it is the measurement that the
    // model tried. Note this array carries no `run_claude_pilot` (it is a skill
    // tool, absent from `default_tools`), so what is asserted here is the shape
    // of the withheld list rather than the presence of that tool.
    assert!(
        !names.is_empty(),
        "the turn must still be served its ordinary tools — withholding one tool \
         is not withholding the tool array"
    );
    assert!(
        names.iter().any(|n| n == "send_message"),
        "send_message is how a fallthrough turn acknowledges; withholding it \
         would break the contract the HARD GATE prescribes: {names:?}"
    );
}

/// **V6 (negative control, axis 1)** — the ready-label turn, which is inside the
/// `[GitHub]` domain and outside the fallthrough one, keeps `create_task`.
///
/// Must be seen **green before** V5 is seen green. Without it, a filter that
/// stripped `create_task` from every `[GitHub]` turn would satisfy V5 while
/// breaking the dispatch path — the one this loop runs on.
#[tokio::test]
async fn mika2517_a_ready_label_turn_still_gets_create_task() {
    let names = served_tool_names(READY_LABEL_MSG).await;

    assert!(
        names.iter().any(|n| n == "create_task"),
        "the ready-label dispatch path creates the tracking row the dispatch \
         needs — mika#2517 must not reach it. Served: {names:?}"
    );
}

/// **V7 (negative control, axis 2)** — a turn with no `[GitHub]` prefix keeps
/// `create_task`.
///
/// The second axis: V6 alone would be satisfied by a filter keyed on the
/// ready-label marker rather than on the domain.
#[tokio::test]
async fn mika2517_a_non_webhook_turn_still_gets_create_task() {
    let names = served_tool_names(PLAIN_MSG).await;

    assert!(
        names.iter().any(|n| n == "create_task"),
        "an ordinary conversation turn is outside the domain entirely: \
         {names:?}"
    );
}

/// **V8 (AC3)** — the turn is counted, once, with its class; and a turn outside
/// the domain writes nothing.
///
/// This row **is** the positive control of the ticket's acceptance. The
/// acceptance is an absence (zero phantom `pending`), and without a count of the
/// turns that could have produced one, zero phantoms reads exactly like zero
/// turns (mika#2205).
#[tokio::test]
async fn mika2517_the_fallthrough_turn_is_counted_with_its_class() {
    let harness = EvalHarness::builder()
        .responses(enough_text_responses(4))
        .tools(tools_with_create_task())
        .build()
        .await
        .unwrap();

    harness.run(FALLTHROUGH_MSG).await.unwrap();

    let rows = fallthrough_rows(&harness).await;

    assert_eq!(
        rows.len(),
        1,
        "exactly one row per fallthrough turn — not deduplicated, because this \
         is a dated event an operator counts, not a tick classifying a \
         population (mika#2131). Got: {rows:?}"
    );
    assert_eq!(
        rows[0].0.as_deref(),
        Some("issue_labeled"),
        "the marker class is the wire format an operator GROUP BYs on"
    );
    assert_eq!(
        rows[0].1.as_deref(),
        Some("withheld=create_task"),
        "the row must name what the turn was not handed"
    );
}

/// The `(after_value, reasoning)` of every `webhook_fallthrough_turn` row this
/// harness's session carries.
async fn fallthrough_rows(harness: &EvalHarness) -> Vec<(Option<String>, Option<String>)> {
    harness
        .db
        .get_audit_events(&harness.session_id)
        .await
        .expect("audit events must be readable")
        .into_iter()
        .filter(|e| e.tool_name == "webhook_fallthrough_turn")
        .map(|e| (e.after_value, e.reasoning))
        .collect()
}

/// **V8, negative half** — a turn outside the domain writes no row.
///
/// Without it, "the event fires on fallthrough turns" would be indistinguishable
/// from "the event fires on every turn", and the distribution an operator reads
/// would be noise.
#[tokio::test]
async fn mika2517_a_turn_outside_the_domain_writes_no_row() {
    let harness = EvalHarness::builder()
        .responses(enough_text_responses(8))
        .tools(tools_with_create_task())
        .build()
        .await
        .unwrap();

    harness.run(READY_LABEL_MSG).await.unwrap();
    harness.run(PLAIN_MSG).await.unwrap();

    let rows = fallthrough_rows(&harness).await;

    assert!(
        rows.is_empty(),
        "neither the ready-label path nor an ordinary turn is Webhook \
         Fallthrough; counting them would drown the population the row exists \
         to size: {rows:?}"
    );
}
