//! mika#1883 — the per-turn token total is a SUM, and an unmeasured turn
//! attests nothing.
//!
//! # What this covers that its neighbours do not
//!
//! `mika_common::llm::types::tests::mika1883_cache_fields_accumulate_across_none_and_some`
//! asserts the fold; `mika_a2a::params::tests` assert the wire key and its
//! fail-soft read. Neither runs a turn, and the property this ticket exists for
//! only exists on one: that the field carries **every** call of the turn rather
//! than the last.
//!
//! # Why the fixtures carry DISTINCT usages
//!
//! This is the whole load-bearing decision of the file. A turn whose three
//! calls all reported the same usage would pass identically against the correct
//! implementation (a sum) and against the defect (`last_usage`), differing only
//! by a factor nobody asserted — so it would attest nothing at all. Each call
//! therefore reports a different, prime-ish figure, and the assertions state
//! both the expected total **and** the value the defective implementation would
//! have produced, so a reader can see the test discriminate rather than take it
//! on trust.

use std::time::Duration;

use mika_common::llm::mock::*;
use mika_common::llm::{LlmResponse, LlmResponseContent, LlmStopReason, LlmUsage};
use tokio::time::Instant;

use super::harness::EvalHarness;

/// A successful response carrying a caller-chosen usage.
///
/// `mock.rs`'s helpers all hand back `LlmUsage::default()` — four zeros — which
/// is exactly the shape that cannot separate a sum from a last-call read.
fn usage(input: u64, output: u64, cache_read: Option<u64>, cache_write: Option<u64>) -> LlmUsage {
    LlmUsage {
        input_tokens: input,
        output_tokens: output,
        cache_creation_input_tokens: cache_write,
        cache_read_input_tokens: cache_read,
    }
}

fn tool_call_with_usage(name: &str, u: LlmUsage) -> MockResponse {
    MockResponse::Success(LlmResponse {
        content: vec![LlmResponseContent::ToolCall {
            id: format!("tc_{}", uuid::Uuid::new_v4().as_simple()),
            name: name.to_string(),
            arguments: serde_json::json!({}),
        }],
        reasoning: None,
        stop_reason: LlmStopReason::ToolUse,
        usage: u,
    })
}

fn text_with_usage(text: &str, u: LlmUsage) -> MockResponse {
    MockResponse::Success(LlmResponse {
        content: vec![LlmResponseContent::Text(text.to_string())],
        reasoning: None,
        stop_reason: LlmStopReason::EndTurn,
        usage: u,
    })
}

/// **AC1** — a three-call turn reports the sum of its three calls.
///
/// The negative control is inside the assertion rather than in a second test:
/// `last_usage` on this fixture is the *third* call's `1_009`, and the expected
/// total is `3_034`. An implementation that forwarded `AgentOutput.usage` would
/// produce the first number, so the assertion fails for the right reason and
/// the message says which of the two it saw.
#[tokio::test]
async fn mika1883_run_usage_sums_every_call_of_the_turn() {
    let first = usage(1_009, 11, None, Some(64));
    let second = usage(1_016, 13, Some(1_000), None);
    let third = usage(1_009, 17, Some(1_004), None);

    let harness = EvalHarness::builder()
        .responses(vec![
            tool_call_with_usage("get_current_time", first.clone()),
            tool_call_with_usage("get_current_time", second.clone()),
            text_with_usage("It is time.", third.clone()),
        ])
        .build()
        .await
        .unwrap();

    let trace = harness.run("what time is it?").await.unwrap();

    let run = trace
        .output
        .run_usage
        .as_ref()
        .expect("a turn that made three calls has a total to report");

    // The defect this ticket exists to refuse, named so a failure reads.
    let last_call_input = third.input_tokens;
    let expected_input = first.input_tokens + second.input_tokens + third.input_tokens;
    assert_ne!(
        expected_input, last_call_input,
        "the fixture must make sum and last-call differ, or this test attests nothing"
    );
    assert_eq!(
        run.input_tokens, expected_input,
        "run_usage must be the SUM of the turn's calls ({expected_input}); \
         {last_call_input} would mean `last_usage` is being served"
    );
    assert_eq!(
        run.output_tokens,
        first.output_tokens + second.output_tokens + third.output_tokens
    );

    // Cache fields reported on *some* calls only — the ordinary case, and the
    // one a naive `Option` fold erases.
    assert_eq!(
        run.cache_read_input_tokens,
        Some(2_004),
        "a cache read reported on two of three calls must survive the fold"
    );
    assert_eq!(
        run.cache_creation_input_tokens,
        Some(64),
        "a cache write reported on one call of three must survive the fold"
    );

    // And the sibling field keeps its own meaning: `usage` is still the last
    // call. Reusing one field for both is what the ticket refused.
    assert_eq!(
        trace.output.usage.as_ref().map(|u| u.input_tokens),
        Some(last_call_input),
        "AgentOutput.usage must keep meaning 'the last call' — the two fields \
         answer different questions and a merge would lose one of them"
    );
}

/// **AC1, the second addition site** — the max-steps continuation call is
/// **added** to the total, not substituted for it.
///
/// `AgentOutput.usage` uses `cont.usage.or(usage)` — a substitution, correct for
/// "the last call". The aggregate must not copy that shape: the continuation
/// emits its own `turn_usage` line (`save_continuation_llm_call`), so a
/// substitution here would make the field disagree with the log stream it is
/// supposed to sum. The fixture makes the two arithmetics differ by making the
/// continuation's own figure smaller than the twenty steps behind it.
#[tokio::test]
async fn mika1883_the_continuation_call_is_counted() {
    let per_step = usage(101, 7, None, None);
    let continuation = usage(1_009, 23, None, None);

    let mut responses: Vec<MockResponse> = (0..20)
        .map(|_| tool_call_with_usage("get_current_time", per_step.clone()))
        .collect();
    responses.push(text_with_usage(
        "Here is a summary of what I did.",
        continuation.clone(),
    ));

    let harness = EvalHarness::builder()
        .responses(responses)
        .build()
        .await
        .unwrap();

    let trace = harness.run("do many things").await.unwrap();

    let run = trace
        .output
        .run_usage
        .as_ref()
        .expect("twenty-one calls have a total");

    let steps_total = per_step.input_tokens * 20;
    let expected = steps_total + continuation.input_tokens;
    assert_eq!(
        run.input_tokens, expected,
        "the continuation must be ADDED ({expected}); {} would mean it replaced \
         the twenty steps, and {steps_total} would mean it was dropped",
        continuation.input_tokens
    );

    // The sibling field went the other way, and must keep going the other way.
    assert_eq!(
        trace.output.usage.as_ref().map(|u| u.input_tokens),
        Some(continuation.input_tokens),
        "`usage` substitutes the continuation because it means 'the last call'; \
         harmonising the two expressions breaks whichever one is moved"
    );
}

/// **AC2** — a turn that made no readable call attests nothing, never a zero.
///
/// The provider errors on its only call, so the loop never reaches the
/// accumulator. `Some(LlmUsage::default())` here would be four zeros served as
/// a measurement, which the client cannot tell from a real turn.
#[tokio::test]
async fn mika1883_an_unmeasured_turn_attests_nothing() {
    let harness = EvalHarness::builder()
        .responses(vec![MockResponse::Error(
            mika_common::llm::LlmError::ProviderError("mika#1883 fixture".to_string()),
        )])
        .build()
        .await
        .unwrap();

    let result = harness.run("ping").await;

    // The loop propagates the provider error, so there is no `AgentOutput` at
    // all — which is the same population the server reads as "nothing to
    // attest". What must never happen is a zeroed total.
    if let Ok(trace) = result {
        assert!(
            trace.output.run_usage.is_none(),
            "a turn with no readable call must attest nothing — a zeroed total \
             is indistinguishable from a real turn (mika#2331's `null` is never `0`)"
        );
    }
}

/// **AC1, the arm that can be forgotten alone** — a turn cut off by its
/// envelope still reports what it spent.
///
/// `persist_deadline_fallback` is the `AgentOutput` constructor outside
/// `run_agent_inner`'s scope, so it takes the value as a parameter; leaving it
/// at `None` would keep every other assertion in this file green while the
/// population an operator most often investigates — *what did the turn that ran
/// out of time cost?* — went unanswerable. Same reasoning, same site, as
/// mika#2304's FD7.
#[tokio::test]
async fn mika1883_a_turn_cut_off_by_its_envelope_still_reports_what_it_spent() {
    let first = usage(1_009, 11, None, None);

    let harness = EvalHarness::builder()
        .responses(vec![
            tool_call_with_usage("get_current_time", first.clone()),
            delayed_response(
                1_500,
                tool_call_with_usage("get_current_time", usage(1_016, 13, None, None)),
            ),
        ])
        .build()
        .await
        .unwrap();

    let trace = harness
        .run_with_deadline("ping", Instant::now() + Duration::from_secs(1))
        .await
        .unwrap();

    assert!(
        trace.output.deadline_exceeded.is_some(),
        "the fixture must actually reach the deadline path, or it attests nothing about it"
    );
    let run = trace
        .output
        .run_usage
        .as_ref()
        .expect("a cut-off turn spent every token it spent before the cut");
    assert!(
        run.input_tokens >= first.input_tokens,
        "the calls made before the envelope ran out must be counted, got {}",
        run.input_tokens
    );
}
