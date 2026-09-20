//! Production-path coverage for the verdict↔flag guard (mika#2237, U4b).
//!
//! Layer A (predicate units) lives in `evidence::guards::tests::mika2237`. This
//! file is Layer B: the wiring contract, driven through `run_agent` with
//! `MockLlmProvider` so no network and no LLM key are involved.
//!
//! **Why this layer is necessary and not redundant.** Vincent's second comment
//! on the ticket is right that the *decision* — a learned memory outweighing an
//! explicit skill mapping — is arbitrated by the LLM at runtime and is out of
//! reach of a unit test. What this ticket changes is that the decision's
//! *manifestation* became a fact in the argv, and a fact in the argv is exactly
//! what a deterministic test can hold. The calibration scenario (U4d) keeps the
//! other half — that the model follows the skill spontaneously.
//!
//! Two properties are load-bearing across the file and are asserted rather than
//! assumed (V13): two `run_gh` calls emitted in the same turn are processed in
//! order, and the first is persisted to `tool_calls` before the second reaches
//! the guard. Without them the D4 escape hatch could not read anything.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use dashmap::DashMap;
use mika_agent::skills::SkillRegistry;
use mika_agent::skills::builtin_handlers;
use mika_agent::skills::index::SkillEntry;
use mika_agent::skills::manifest::{Constraints, SkillInfo, SkillManifest, Triggers};
use mika_agent::tools::{Tool, ToolContext, ToolOutput, ToolRegistry, default_tools};
use mika_common::claude::ToolDefinition;
use mika_common::llm::mock::*;
use serde_json::json;

use super::assertions::*;
use super::harness::EvalHarness;

/// The refusal's stable identifier, as the LLM receives it.
const REFUSAL: &str = "pr_review_flag_mismatch";

/// A review body carrying the literal shape of the founding incident: the
/// decorated `VERDICT: pass ✅` of mika#2236, plus the DEPTH line qa-review's
/// own mika#275 gate requires.
fn pass_body() -> String {
    "VERDICT: pass \u{2705}\nDEPTH: code-level\nREASON: every AC is satisfied.".to_string()
}

fn hold_body() -> String {
    "VERDICT: hold[review] \u{23f8}\u{fe0f}\nDEPTH: code-level\nREASON: needs an operator look."
        .to_string()
}

fn block_body() -> String {
    "VERDICT: block[ac] \u{274c}\nDEPTH: code-level\nREASON: AC3 unsatisfied.".to_string()
}

// -- The real builtin, reached through the agent loop --

/// Delegates to `builtin_handlers::execute("run_gh", …)` so the guard runs in
/// its production position — inside `run_gh`, before the subprocess.
struct BuiltinRunGhTool;

#[async_trait]
impl Tool for BuiltinRunGhTool {
    fn name(&self) -> &str {
        "run_gh"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "run_gh".to_string(),
            description: "Execute a GitHub CLI command".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "command": {"type": "array", "items": {"type": "string"}},
                    "repo": {"type": "string"}
                },
                "required": ["command"]
            }),
        }
    }

    async fn execute(
        &self,
        input: serde_json::Value,
        ctx: &ToolContext<'_>,
    ) -> anyhow::Result<ToolOutput> {
        Ok(builtin_handlers::execute("run_gh", input, ctx).await)
    }
}

fn tools_with_builtin_run_gh() -> ToolRegistry {
    let mut registry = default_tools();
    registry.register(Box::new(BuiltinRunGhTool));
    registry
}

/// A minimal qa-review skill so the turn resembles a real review turn.
///
/// Note the guard does NOT depend on this: its gating is the body, never the
/// active skill (D1). It is here so the turn also traverses the mika#275 depth
/// check and the mika#1196 scope check, i.e. so the ordering of the chain is
/// exercised rather than assumed.
fn make_qa_review_skill(dir: PathBuf) -> SkillEntry {
    SkillEntry {
        manifest: SkillManifest {
            skill: SkillInfo {
                name: "qa-review".to_string(),
                description: "Quality gate for PR review".to_string(),
                version: "0.9.0".to_string(),
                always_on: true,
                timeout_secs: 30,
                dependencies: vec![],
                max_prompt_size: None,
                data_grade: Default::default(),
            },
            triggers: Triggers {
                keywords: vec!["review".to_string(), "pr".to_string()],
            },
            llm: Default::default(),
            constraints: Constraints {
                required_tools: vec![],
                required_fetches_for_quoted_resources: false,
            },
            output: Default::default(),
            context: HashMap::new(),
            variants: Default::default(),
        },
        dir,
        keywords_lower: vec!["review".to_string(), "pr".to_string()],
        prompt_snippet: "You are the QA review agent. Review PRs.".to_string(),
        skill_tools: vec![],
        enabled: true,
        has_override: false,
        provider_overrides: HashMap::new(),
        prompt_sources: SkillEntry::empty_prompt_sources(),
        model_overrides: HashMap::new(),
    }
}

/// Build a harness whose turn carries a qa-review skill and the real `run_gh`.
///
/// The session-scoped dedup map is threaded because `run_gh` carries a
/// `debug_assert!(ctx.pr_reviews_posted.is_some())` for production `pr review`
/// calls — and because it is part of the contract under test: the D4 escape
/// hatch posts a *second* `pr review` on the same PR in the same session, so it
/// must not trip that dedup. It does not, since mika#2237 made the ledger
/// register only reviews that actually landed.
async fn harness_with_qa_review(responses: Vec<MockResponse>) -> EvalHarness {
    let mut harness = EvalHarness::builder()
        .responses(responses)
        .tools(tools_with_builtin_run_gh())
        .pr_reviews_posted(Arc::new(DashMap::<String, HashSet<String>>::new()))
        .build()
        .await
        .unwrap();

    let skill_dir = harness.home_dir.path().join("skills/qa-review");
    std::fs::create_dir_all(&skill_dir).unwrap();
    std::fs::write(
        skill_dir.join("system_prompt.md"),
        "You are the QA review agent. Review PRs.",
    )
    .unwrap();
    harness.skills = SkillRegistry::from_test_entries(vec![make_qa_review_skill(skill_dir)]);
    harness
}

/// A cheap `run_gh` call whose only purpose is to put a row in `tool_calls`
/// before the review lands.
///
/// It is not decoration. The guard abstains on an empty turn history — it
/// cannot tell "no attempt was made" from "tool-call persistence is off" (D5),
/// so a turn whose very first call is the review is deliberately out of its
/// reach. A real qa-review turn always reads the diff first; this reproduces
/// that shape, and the abstention test at the bottom pins the other one.
fn prior_read(pr: &str) -> MockResponse {
    tool_call_response("run_gh", json!({"command": ["pr", "diff", pr]}))
}

// -------------------------------------------------------------------------
// V1 — `pass` posted as `--comment` with no attempt is refused.
//
// This is the founding incident, replayed through the production path: the
// argv recorded at 08:03:34Z on 2026-09-08 was
// `["pr","review","2236","--comment",…]` with zero approve attempt.
// -------------------------------------------------------------------------

#[tokio::test]
async fn pass_degraded_to_comment_without_an_attempt_is_refused() {
    let harness = harness_with_qa_review(vec![
        prior_read("2236"),
        tool_call_response(
            "run_gh",
            json!({"command": ["pr", "review", "2236", "--comment", "--body", pass_body()]}),
        ),
        text_response("The review flag was refused by the verdict-coherence guard."),
    ])
    .await;

    let trace = harness.run("Review PR 2236").await.unwrap();

    let calls = trace.calls_for_tool("run_gh");
    assert!(
        calls.len() >= 2,
        "expected the read and the review, got {} run_gh call(s)",
        calls.len()
    );
    let review_output = calls[1].output.as_deref().unwrap_or("");
    assert!(
        review_output.contains(REFUSAL),
        "a `pass` body posted under --comment with no prior attempt must be refused; \
         got: {review_output}"
    );
    assert!(
        review_output.contains("--approve"),
        "the refusal must name the flag the verdict imposes; got: {review_output}"
    );
    assert!(
        review_output.contains("mika#2237"),
        "the refusal must carry its doctrine reference; got: {review_output}"
    );
    // D6 — the refusal names the second correct way out, so a model held by its
    // memory is not pushed into rewriting its VERDICT instead of its flag.
    assert!(
        review_output.contains("not evidence about THIS pull request"),
        "the refusal must say what a remembered failure is not evidence of (fix (c)); \
         got: {review_output}"
    );
}

// -------------------------------------------------------------------------
// V13 — the escape hatch can actually read the turn (A1), and the failure
// signal it reads is the one A2 predicted (V12, end to end).
// -------------------------------------------------------------------------

/// V5 — a failed `--approve` first, then `--comment`: accepted.
///
/// The first call targets a repository the test environment cannot reach, so
/// `gh` exits non-zero (or fails to spawn). Either way `dispatch` records
/// `success = false`, which is the `!row.success` predicate the hatch reads —
/// the three populations of A2 collapsing to one observable fact.
///
/// This test is also the only place the two A-assumptions are verified
/// together: if the first call were not persisted before the second traversed
/// the guard (A1), or if a non-zero `gh` exit did not land as `success = false`
/// (A2), the review below would be refused instead of accepted.
#[tokio::test]
async fn a_failed_approve_earlier_in_the_turn_permits_the_degradation() {
    let harness = harness_with_qa_review(vec![
        tool_call_response(
            "run_gh",
            json!({
                "command": ["pr", "review", "999999", "--approve", "--body", pass_body()],
                "repo": "senara-solutions/does-not-exist-mika2237"
            }),
        ),
        tool_call_response(
            "run_gh",
            json!({
                "command": ["pr", "review", "999999", "--comment", "--body", pass_body()],
                "repo": "senara-solutions/does-not-exist-mika2237"
            }),
        ),
        text_response("Approve failed; posted a comment citing the failure."),
    ])
    .await;

    let trace = harness.run("Review PR 999999").await.unwrap();

    let calls = trace.calls_for_tool("run_gh");
    assert!(calls.len() >= 2, "expected two run_gh calls");

    let approve_output = calls[0].output.as_deref().unwrap_or("");
    assert!(
        !approve_output.contains(REFUSAL),
        "the --approve call matches its verdict and must never be refused by this guard"
    );
    assert!(
        !calls[0].success,
        "precondition for this test (A2): the unreachable --approve must be recorded as a \
         failure. If this trips, `gh` succeeded unexpectedly or the non-zero-exit signal \
         changed shape — the escape hatch reads `!success` and nothing else. Output: \
         {approve_output}"
    );

    let comment_output = calls[1].output.as_deref().unwrap_or("");
    assert!(
        !comment_output.contains(REFUSAL),
        "after a measured --approve failure the degradation is legitimate and must pass \
         (D4); got: {comment_output}"
    );
    // Found while wiring this test, and load-bearing for AC2: the mika#821
    // session dedup used to register its key on any `gh` invocation that merely
    // spawned, so a REFUSED --approve consumed the right to post at all and the
    // escape hatch was decorative. The ledger now registers only reviews that
    // landed. Without this assertion the test above would go green for the wrong
    // reason — refused by the dedup instead of admitted by the hatch.
    assert!(
        !comment_output.contains("duplicate_pr_review"),
        "a --approve that GitHub refused posted nothing, so it must not consume the \
         session dedup slot the fallback --comment needs; got: {comment_output}"
    );

    // Accepted BY THE HATCH, not by the abstention. Both produce an unrefused
    // call, so without this the test would be green against a guard that simply
    // never read the history — the exact false-green D4 is built to avoid. This
    // is also where AC3's two populations become countable apart.
    let events = harness
        .db
        .get_audit_events(&harness.session_id)
        .await
        .expect("audit events");
    let outcomes: Vec<&str> = events
        .iter()
        .filter(|e| e.tool_name == "pr_review_flag_guard")
        .filter_map(|e| e.after_value.as_deref())
        .collect();
    assert!(
        outcomes.contains(&"degraded_after_attempt"),
        "expected a `degraded_after_attempt` audit row proving the hatch read the turn's \
         failed --approve; got outcomes {outcomes:?}"
    );
    assert!(
        !outcomes.contains(&"abstained"),
        "the turn has a readable, non-empty history, so the guard must decide rather than \
         abstain; got outcomes {outcomes:?}"
    );
}

// -------------------------------------------------------------------------
// The negative controls. Without them the tests above would pass just as well
// against a guard that refuses everything, and "the guard decides" would be
// indistinguishable from "the guard blocks".
// -------------------------------------------------------------------------

/// V4 — `hold[review]` under `--comment` is the nominal mapping: no refusal.
#[tokio::test]
async fn hold_review_posted_as_comment_traverses_without_a_word() {
    let harness = harness_with_qa_review(vec![
        prior_read("2236"),
        tool_call_response(
            "run_gh",
            json!({"command": ["pr", "review", "2236", "--comment", "--body", hold_body()]}),
        ),
        text_response("Hold posted."),
    ])
    .await;

    let trace = harness.run("Review PR 2236").await.unwrap();
    let calls = trace.calls_for_tool("run_gh");
    let output = calls[1].output.as_deref().unwrap_or("");
    assert!(
        !output.contains(REFUSAL),
        "hold[review] maps to --comment; this is the nominal path: {output}"
    );
}

/// The other nominal path — `pass` under `--approve`.
#[tokio::test]
async fn pass_posted_as_approve_traverses_without_a_word() {
    let harness = harness_with_qa_review(vec![
        prior_read("2236"),
        tool_call_response(
            "run_gh",
            json!({"command": ["pr", "review", "2236", "--approve", "--body", pass_body()]}),
        ),
        text_response("Approved."),
    ])
    .await;

    let trace = harness.run("Review PR 2236").await.unwrap();
    let calls = trace.calls_for_tool("run_gh");
    let output = calls[1].output.as_deref().unwrap_or("");
    assert!(
        !output.contains(REFUSAL),
        "pass maps to --approve; this is the nominal path: {output}"
    );
}

// -------------------------------------------------------------------------
// V2 / D3 — the dangerous direction. The measured defect is conservative
// (a pass degraded to a comment); its inverse would merge a blocked PR.
// -------------------------------------------------------------------------

#[tokio::test]
async fn a_blocking_verdict_posted_as_approve_is_refused() {
    let harness = harness_with_qa_review(vec![
        tool_call_response(
            "run_gh",
            json!({"command": ["pr", "review", "2236", "--approve", "--body", block_body()]}),
        ),
        text_response("Refused."),
    ])
    .await;

    let trace = harness.run("Review PR 2236").await.unwrap();
    let calls = trace.calls_for_tool("run_gh");
    let output = calls[0].output.as_deref().unwrap_or("");
    assert!(
        output.contains(REFUSAL),
        "a block[ac] body posted under --approve must be refused: {output}"
    );
    assert!(
        output.contains("block[ac]"),
        "the refusal names the verdict it read: {output}"
    );
    // This direction has NO escape hatch and must not consult the history: no
    // past failure can make approving a blocked PR correct. The turn above has
    // an empty history, and the refusal still fires — which is the assertion.
}

// -------------------------------------------------------------------------
// V3 — fail-open recognition. A review with no classifiable verdict is not
// this guard's business, and a human or ad-hoc review must never be blocked.
// -------------------------------------------------------------------------

#[tokio::test]
async fn a_body_with_no_classifiable_verdict_is_never_refused() {
    let harness = harness_with_qa_review(vec![
        prior_read("2236"),
        tool_call_response(
            "run_gh",
            json!({
                "command": [
                    "pr", "review", "2236", "--comment", "--body",
                    // mika#1821's bound, inherited verbatim: a trailing comment
                    // is not decoration, so this does not classify.
                    "VERDICT: pass \u{2014} but see findings\nDEPTH: code-level"
                ]
            }),
        ),
        text_response("Commented."),
    ])
    .await;

    let trace = harness.run("Review PR 2236").await.unwrap();
    let calls = trace.calls_for_tool("run_gh");
    let output = calls[1].output.as_deref().unwrap_or("");
    assert!(
        !output.contains(REFUSAL),
        "a body whose verdict does not classify is out of the population (D1): {output}"
    );
}

// -------------------------------------------------------------------------
// V6 — the abstention, and its named cost.
// -------------------------------------------------------------------------

/// A turn whose review is its FIRST tool call is accepted, not refused.
///
/// This is the fail-open half of D5 and it is a real cost, pinned here so it is
/// a decision rather than a discovery: the guard cannot separate "no attempt
/// was made" from "`MIKA_STORE_TOOL_CALLS` is off", and refusing on the
/// unobservable term would produce the loop `--approve` fails → `--comment`
/// refused → `--approve` fails…, i.e. a review that never leaves.
///
/// The abstention is said out loud (`pr_review_flag_guard_abstained` + an audit
/// row), which is what keeps the inertia visible. Sustained hits on that grep
/// mean tool-call persistence is off, not that the predicate needs widening.
#[tokio::test]
async fn an_empty_turn_history_abstains_rather_than_refusing() {
    let harness = harness_with_qa_review(vec![
        tool_call_response(
            "run_gh",
            json!({"command": ["pr", "review", "2236", "--comment", "--body", pass_body()]}),
        ),
        text_response("Commented."),
    ])
    .await;

    let trace = harness.run("Review PR 2236").await.unwrap();
    let calls = trace.calls_for_tool("run_gh");
    let output = calls[0].output.as_deref().unwrap_or("");
    assert!(
        !output.contains(REFUSAL),
        "with no readable turn history the hatch stays open by design (D5); refusing here \
         would make an unreadable term a satisfied one: {output}"
    );
    assert_tools_include(&trace, &["run_gh"]);

    // The abstention is SAID — that is what keeps the inertia visible instead of
    // turning the guard into a thing that quietly stops working the day
    // `MIKA_STORE_TOOL_CALLS` is switched off.
    let events = harness
        .db
        .get_audit_events(&harness.session_id)
        .await
        .expect("audit events");
    assert!(
        events
            .iter()
            .filter(|e| e.tool_name == "pr_review_flag_guard")
            .any(|e| e.after_value.as_deref() == Some("abstained")),
        "an abstention must leave a named row; a silent one is indistinguishable from a \
         guard that decided"
    );
}
