//! Production-path coverage for the CI↔verdict gate (mika#2455, U4).
//!
//! Layer A (predicate units) lives in `evidence::guards::tests::mika2455`. This
//! file is Layer B: the wiring contract, driven through `run_agent` with
//! `MockLlmProvider` — **no network, no LLM key, and no `gh`**.
//!
//! # The one thing this file cannot have, and how it is worked around
//!
//! The gate's production reader is `gh pr checks --required`. A test has no
//! token and no network, so the reader is the one part that must be
//! substituted. The seam is deliberately placed at the **raw stdout of the
//! subprocess** rather than at the parsed checks: a test that injected already
//! parsed `GhCheck`s would leave `parse_gh_checks` and `classify_ci_coherence`
//! unexercised on the production path, and would attest its own parser instead
//! of this one. With the seam where it is, every layer below the network is the
//! production one.
//!
//! Two tools are therefore defined here, and the split is what makes the
//! coverage honest:
//!
//! - [`BuiltinRunGhTool`] delegates to `builtin_handlers::execute("run_gh", …)`
//!   verbatim, so the **wiring** — the gate is reached at all, and only for the
//!   verdicts it claims — is observed in its real position inside the `run_gh`
//!   chain. It cannot observe a refusal (the real reader abstains here), which
//!   is precisely what the second tool is for.
//! - [`CiGatedRunGhTool`] runs the same gate function with an injected reader,
//!   so the **decision** — refuse, allow-green, allow-pending — is observed
//!   against real check payloads.
//!
//! Neither tool alone would be enough: the first would go green against a gate
//! that decides nothing, the second against a gate nobody calls.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use dashmap::DashMap;
use mika_agent::evidence::guards::{CiAbstention, QA_CI_COHERENCE_AUDIT_TOOL};
use mika_agent::skills::SkillRegistry;
use mika_agent::skills::builtin_handlers;
use mika_agent::skills::index::SkillEntry;
use mika_agent::skills::manifest::{Constraints, SkillInfo, SkillManifest, Triggers};
use mika_agent::tools::{Tool, ToolContext, ToolOutput, ToolRegistry, default_tools};
use mika_common::claude::ToolDefinition;
use mika_common::llm::mock::*;
use serde_json::json;

use super::harness::EvalHarness;

/// The refusal's stable identifier, as the LLM receives it.
const REFUSAL: &str = "qa_ci_coherence_violation";

/// The two red required checks measured on PR #2439 at head `73ec3e3e`.
const RED_CHECKS_2439: &str = r#"[
    {"name": "SIGPIPE grep-q Lint", "state": "FAILURE", "bucket": "fail"},
    {"name": "Check", "state": "FAILURE", "bucket": "fail"},
    {"name": "docker-build", "state": "SUCCESS", "bucket": "pass"}
]"#;

/// The single red required check measured on PR #2461 at head `1302b0d0` — a
/// different failure surface (a `mika-cli` unit test), same blind spot.
const RED_CHECKS_2461: &str = r#"[
    {"name": "Check", "state": "FAILURE", "bucket": "fail"}
]"#;

const GREEN_CHECKS: &str = r#"[
    {"name": "Check", "state": "SUCCESS", "bucket": "pass"},
    {"name": "docker-build", "state": "SUCCESS", "bucket": "pass"}
]"#;

const PENDING_CHECKS: &str = r#"[
    {"name": "Check", "state": "IN_PROGRESS", "bucket": "pending"},
    {"name": "docker-build", "state": "SUCCESS", "bucket": "pass"}
]"#;

/// A review body carrying the literal shape of the two measured incidents: a
/// decorated `VERDICT: pass`, plus the DEPTH line qa-review's own mika#275 gate
/// requires.
fn pass_body() -> String {
    "VERDICT: pass \u{2705}\nDEPTH: code-level\nREASON: every AC is satisfied.".to_string()
}

fn hold_body() -> String {
    "VERDICT: hold[review] \u{23f8}\u{fe0f}\nDEPTH: code-level\nREASON: needs an operator look."
        .to_string()
}

fn block_ac_body() -> String {
    "VERDICT: block[ac] \u{274c}\nDEPTH: code-level\nREASON: AC3 unsatisfied.".to_string()
}

/// A body with no `VERDICT:` line at all — a human or ad-hoc review.
fn unclassified_body() -> String {
    "DEPTH: code-level\nLooks reasonable to me, shipping notes below.".to_string()
}

// ---------------------------------------------------------------------------
// The two tools
// ---------------------------------------------------------------------------

/// The real builtin, reached through the agent loop — wiring coverage.
struct BuiltinRunGhTool;

fn run_gh_definition() -> ToolDefinition {
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

#[async_trait]
impl Tool for BuiltinRunGhTool {
    fn name(&self) -> &str {
        "run_gh"
    }

    fn definition(&self) -> ToolDefinition {
        run_gh_definition()
    }

    async fn execute(
        &self,
        input: serde_json::Value,
        ctx: &ToolContext<'_>,
    ) -> anyhow::Result<ToolOutput> {
        Ok(builtin_handlers::execute("run_gh", input, ctx).await)
    }
}

/// The same gate, with the subprocess substituted — decision coverage.
///
/// It does **not** fall through to the real `run_gh` afterwards, on purpose: the
/// real one would run the gate a second time with the production reader, write a
/// second `abstained` audit row, and make the ledger assertions ambiguous about
/// which decision they are reading. Letting the call "succeed" once the gate has
/// allowed it is exactly the observable the plan asks for — the review is not
/// prevented.
struct CiGatedRunGhTool {
    /// What the substituted `gh pr checks` answers.
    checks: Result<String, String>,
}

#[async_trait]
impl Tool for CiGatedRunGhTool {
    fn name(&self) -> &str {
        "run_gh"
    }

    fn definition(&self) -> ToolDefinition {
        run_gh_definition()
    }

    async fn execute(
        &self,
        input: serde_json::Value,
        ctx: &ToolContext<'_>,
    ) -> anyhow::Result<ToolOutput> {
        let argv: Vec<String> = input
            .get("command")
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        let repo = input.get("repo").and_then(|v| v.as_str());
        let answer = self.checks.clone();

        if let Err(refusal) = builtin_handlers::validate_qa_ci_coherence_with_reader(
            &argv,
            repo,
            ctx,
            move |_pr, _repo, _token| async move { answer },
        )
        .await
        {
            return Ok(refusal);
        }
        Ok(ToolOutput::success("review posted".to_string()))
    }
}

/// A minimal qa-review skill so the turn resembles a real review turn.
///
/// The gate does NOT depend on it — its gating is the body, never the active
/// skill — but its presence makes the turn traverse the mika#275 depth check and
/// the mika#1196 scope check, i.e. exercises the chain's ordering rather than
/// assuming it.
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

/// A harness carrying a GitHub token — required by every **decision** test.
///
/// The token is one of the gate's fail-open terms: without it the gate abstains
/// on `no_token` *before* the reader is ever consulted, and every decision test
/// would go green for the wrong reason. It is never used for anything — the
/// reader is injected — but it has to be present for the gate to get that far,
/// and that dependency is itself part of the contract.
///
/// **The builder's `.github_token()` alone does not do it, and that is worth
/// knowing before writing the next such test.** `AgentParams.github_token` is
/// only consulted when `settings` is `None` (`agent_loop`: `if let Some(settings)
/// … resolve_github_token(…) else { params.github_token }`), and the harness
/// always passes `Some(&self.settings)`. So the builder method is **inert** on
/// every harness-driven turn — a knob that activates nothing, the mika#1971
/// shape. Found while wiring this file; repairing the shared harness is out of
/// this ticket's perimeter (three existing tests call that method and would
/// change behaviour), so it is named here and carried as a follow-up. The token
/// is set where the loop actually reads it.
async fn harness_with(responses: Vec<MockResponse>, tool: Box<dyn Tool>) -> EvalHarness {
    let mut harness = harness_inner(responses, tool, Some(FAKE_TOKEN)).await;
    harness.settings.github_token = Some(secrecy::SecretString::from(FAKE_TOKEN));
    harness
}

/// Never used to reach the network — the reader is always injected on the
/// decision path.
const FAKE_TOKEN: &str = "ghp_fake_token_for_mika2455";

/// A harness with **no** token — used by the two wiring tests, where the
/// production reader's abstention is the observable.
async fn harness_without_token(responses: Vec<MockResponse>, tool: Box<dyn Tool>) -> EvalHarness {
    harness_inner(responses, tool, None).await
}

async fn harness_inner(
    responses: Vec<MockResponse>,
    tool: Box<dyn Tool>,
    token: Option<&str>,
) -> EvalHarness {
    let mut registry: ToolRegistry = default_tools();
    registry.register(tool);

    let mut builder = EvalHarness::builder()
        .responses(responses)
        .tools(registry)
        .pr_reviews_posted(Arc::new(DashMap::<String, HashSet<String>>::new()));
    if let Some(token) = token {
        builder = builder.github_token(token);
    }
    let mut harness = builder.build().await.unwrap();

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

async fn gate_outcomes(harness: &EvalHarness) -> Vec<String> {
    harness
        .db
        .get_audit_events(&harness.session_id)
        .await
        .expect("audit events")
        .iter()
        .filter(|e| e.tool_name == QA_CI_COHERENCE_AUDIT_TOOL)
        .filter_map(|e| e.after_value.clone())
        .collect()
}

async fn gate_reasonings(harness: &EvalHarness) -> Vec<String> {
    harness
        .db
        .get_audit_events(&harness.session_id)
        .await
        .expect("audit events")
        .iter()
        .filter(|e| e.tool_name == QA_CI_COHERENCE_AUDIT_TOOL)
        .filter_map(|e| e.reasoning.clone())
        .collect()
}

// ---------------------------------------------------------------------------
// AC1 — the ticket's negative test, first direction, on both measured heads.
// ---------------------------------------------------------------------------

/// The founding incident of the ticket body, replayed: PR #2439 at head
/// `73ec3e3e` carried `SIGPIPE grep-q Lint` and `Check` red, and mika-qa posted
/// `pass` / APPROVED anyway.
#[tokio::test]
async fn a_pass_on_a_red_required_check_is_refused_and_names_the_checks() {
    let harness = harness_with(
        vec![
            tool_call_response(
                "run_gh",
                json!({
                    "command": ["pr", "review", "2439", "--approve", "--body", pass_body()],
                    "repo": "senara-solutions/mika"
                }),
            ),
            text_response("The CI-coherence gate refused the pass verdict."),
        ],
        Box::new(CiGatedRunGhTool {
            checks: Ok(RED_CHECKS_2439.to_string()),
        }),
    )
    .await;

    let trace = harness.run("Review PR 2439").await.unwrap();
    let calls = trace.calls_for_tool("run_gh");
    assert!(!calls.is_empty(), "expected the review call");
    let output = calls[0].output.as_deref().unwrap_or("");

    assert!(
        output.contains(REFUSAL),
        "a `pass` body on a head with red required checks must be refused; got: {output}"
    );
    // R2/AC4 — the refusal is self-sufficient: it names what it saw, so the
    // rewrite is not blind. Without this the model is told "no" and nothing else.
    assert!(
        output.contains("SIGPIPE grep-q Lint") && output.contains("Check"),
        "the refusal must name the failing required checks; got: {output}"
    );
    assert!(
        output.contains("mika#2455"),
        "the refusal must carry its doctrine reference; got: {output}"
    );
    // A green check must not be reported as failing — a refusal that over-reports
    // sends the rewrite after a check that is fine.
    assert!(
        !output.contains("docker-build"),
        "a passing check must not appear in the failing list; got: {output}"
    );

    let outcomes = gate_outcomes(&harness).await;
    assert!(
        outcomes.contains(&"refused".to_string()),
        "expected a `refused` audit row (AC8); got {outcomes:?}"
    );
}

/// The second measured case (comment 1 of the ticket, n=2): PR #2461 at head
/// `1302b0d0`, a single red `Check` on a `mika-cli` unit test. A different
/// failure surface, the same blind spot — so the gate must not be keyed to the
/// shape of #2439's failure.
#[tokio::test]
async fn the_second_measured_case_is_refused_too() {
    let harness = harness_with(
        vec![
            tool_call_response(
                "run_gh",
                json!({
                    "command": ["pr", "review", "2461", "--approve", "--body", pass_body()],
                    "repo": "senara-solutions/mika"
                }),
            ),
            text_response("Refused."),
        ],
        Box::new(CiGatedRunGhTool {
            checks: Ok(RED_CHECKS_2461.to_string()),
        }),
    )
    .await;

    let trace = harness.run("Review PR 2461").await.unwrap();
    let output = trace.calls_for_tool("run_gh")[0]
        .output
        .as_deref()
        .unwrap_or("")
        .to_string();
    assert!(output.contains(REFUSAL), "got: {output}");
    assert!(output.contains("Check"), "got: {output}");
}

// ---------------------------------------------------------------------------
// AC4 — the refusal leaves an exit, and mika#2237 does not close it.
//
// This is the point of M4 and the assertion that protects against it: a gate
// written on the FLAG instead of the verdict would compose with mika#2237 into
// "no review postable at all". Without this test the deadlock could come back
// with every other assertion green.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn the_named_way_out_is_actually_postable_under_the_mika2237_guard() {
    // The refusal names `block[ci]` and `hold[review]`, both with `--comment`.
    // Here the model takes the second: same PR, same turn, red CI — and it must
    // go through. `hold[review]` maps to `--comment` under mika#2237, and this
    // gate only ever looks at `pass`.
    let harness = harness_with(
        vec![
            tool_call_response(
                "run_gh",
                json!({
                    "command": ["pr", "review", "2439", "--approve", "--body", pass_body()],
                    "repo": "senara-solutions/mika"
                }),
            ),
            tool_call_response(
                "run_gh",
                json!({
                    "command": ["pr", "review", "2439", "--comment", "--body", hold_body()],
                    "repo": "senara-solutions/mika"
                }),
            ),
            text_response("Rewrote the verdict as a hold and posted it."),
        ],
        Box::new(CiGatedRunGhTool {
            checks: Ok(RED_CHECKS_2439.to_string()),
        }),
    )
    .await;

    let trace = harness.run("Review PR 2439").await.unwrap();
    let calls = trace.calls_for_tool("run_gh");
    assert!(calls.len() >= 2, "expected the refusal and the rewrite");

    let refused = calls[0].output.as_deref().unwrap_or("");
    assert!(refused.contains(REFUSAL), "got: {refused}");
    // AC4 — both correct outputs are named. Naming only one would push the model
    // into spending a CI-fix dispatch slot on a failure no pilot can repair.
    assert!(
        refused.contains("block[ci]") && refused.contains("hold[review]"),
        "the refusal must name BOTH correct outputs (D3); got: {refused}"
    );
    assert!(
        refused.contains("--comment"),
        "the refusal must name the flag the rewrite needs, so it does not walk into \
         mika#2237's refusal; got: {refused}"
    );

    let rewritten = calls[1].output.as_deref().unwrap_or("");
    assert!(
        !rewritten.contains(REFUSAL),
        "the way out this gate names must actually be postable — if this trips, the two \
         guards compose into a deadlock and the turn dies with no review (M4); got: {rewritten}"
    );
    assert!(
        !rewritten.contains("pr_review_flag_mismatch"),
        "the rewrite must not be refused by the mika#2237 sibling either; got: {rewritten}"
    );
}

// ---------------------------------------------------------------------------
// The negative controls. Without them every test above would pass just as well
// against a gate that refuses every review, and "the gate decides" would be
// indistinguishable from "the gate blocks".
// ---------------------------------------------------------------------------

/// AC2 — the second half of the ticket's negative test, and the assertion that
/// separates "accepted because green" from "accepted because it abstained".
#[tokio::test]
async fn a_pass_on_a_green_ci_is_allowed_and_says_so() {
    let harness = harness_with(
        vec![
            tool_call_response(
                "run_gh",
                json!({
                    "command": ["pr", "review", "2439", "--approve", "--body", pass_body()],
                    "repo": "senara-solutions/mika"
                }),
            ),
            text_response("Approved."),
        ],
        Box::new(CiGatedRunGhTool {
            checks: Ok(GREEN_CHECKS.to_string()),
        }),
    )
    .await;

    let trace = harness.run("Review PR 2439").await.unwrap();
    let output = trace.calls_for_tool("run_gh")[0]
        .output
        .as_deref()
        .unwrap_or("")
        .to_string();
    assert!(
        !output.contains(REFUSAL),
        "a green CI must let a pass verdict through; got: {output}"
    );

    // R9 — the positive control. An acceptance and an abstention both produce
    // an unrefused call, so without this the test is green against a gate that
    // never read anything. This is also what makes "zero refusals" readable as
    // a healthy fleet rather than an inert gate (mika#2205).
    let outcomes = gate_outcomes(&harness).await;
    assert!(
        outcomes.contains(&"allowed_green".to_string()),
        "expected an `allowed_green` audit row proving the gate read and decided; \
         got {outcomes:?}"
    );
    assert!(
        !outcomes.contains(&"abstained".to_string()),
        "the reader answered, so the gate must decide rather than abstain; got {outcomes:?}"
    );
}

/// AC3 — a pending required check refuses nothing.
///
/// Not asked for by the ticket, and made necessary by M5: `pull_request.opened`
/// routes to mika-qa with no CI term, so a review that starts before the CI has
/// concluded is the nominal case. A gate that refused here would refuse the
/// nominal.
#[tokio::test]
async fn a_pending_required_check_lets_the_review_through() {
    let harness = harness_with(
        vec![
            tool_call_response(
                "run_gh",
                json!({
                    "command": ["pr", "review", "2439", "--approve", "--body", pass_body()],
                    "repo": "senara-solutions/mika"
                }),
            ),
            text_response("Approved while CI is still running."),
        ],
        Box::new(CiGatedRunGhTool {
            checks: Ok(PENDING_CHECKS.to_string()),
        }),
    )
    .await;

    let trace = harness.run("Review PR 2439").await.unwrap();
    let output = trace.calls_for_tool("run_gh")[0]
        .output
        .as_deref()
        .unwrap_or("")
        .to_string();
    assert!(!output.contains(REFUSAL), "got: {output}");

    let outcomes = gate_outcomes(&harness).await;
    assert!(
        outcomes.contains(&"allowed_pending".to_string()),
        "the pending population must be countable apart from the green one; got {outcomes:?}"
    );
}

/// R3 — `hold[review]` on a red CI passes. The gate's subject is `pass` and
/// nothing else; a blocking verdict already says what the CI says.
#[tokio::test]
async fn a_hold_verdict_on_a_red_ci_is_untouched() {
    let harness = harness_with(
        vec![
            tool_call_response(
                "run_gh",
                json!({
                    "command": ["pr", "review", "2439", "--comment", "--body", hold_body()],
                    "repo": "senara-solutions/mika"
                }),
            ),
            text_response("Hold posted."),
        ],
        Box::new(CiGatedRunGhTool {
            checks: Ok(RED_CHECKS_2439.to_string()),
        }),
    )
    .await;

    let trace = harness.run("Review PR 2439").await.unwrap();
    let output = trace.calls_for_tool("run_gh")[0]
        .output
        .as_deref()
        .unwrap_or("")
        .to_string();
    assert!(!output.contains(REFUSAL), "got: {output}");
    assert!(
        gate_outcomes(&harness).await.is_empty(),
        "a non-pass verdict is out of the gate's population entirely — it must not even \
         write a ledger row"
    );
}

/// R3 — `block[ac]` on a red CI passes too, and for the same reason.
#[tokio::test]
async fn a_blocking_verdict_on_a_red_ci_is_untouched() {
    let harness = harness_with(
        vec![
            tool_call_response(
                "run_gh",
                json!({
                    "command": ["pr", "review", "2439", "--comment", "--body", block_ac_body()],
                    "repo": "senara-solutions/mika"
                }),
            ),
            text_response("Block posted."),
        ],
        Box::new(CiGatedRunGhTool {
            checks: Ok(RED_CHECKS_2439.to_string()),
        }),
    )
    .await;

    let trace = harness.run("Review PR 2439").await.unwrap();
    let output = trace.calls_for_tool("run_gh")[0]
        .output
        .as_deref()
        .unwrap_or("")
        .to_string();
    assert!(!output.contains(REFUSAL), "got: {output}");
    assert!(gate_outcomes(&harness).await.is_empty());
}

/// R3 — a body with no `VERDICT:` line at all (a human or ad-hoc review) is
/// none of this gate's business, red CI or not.
#[tokio::test]
async fn a_body_without_a_verdict_line_is_untouched() {
    let harness = harness_with(
        vec![
            tool_call_response(
                "run_gh",
                json!({
                    "command": [
                        "pr", "review", "2439", "--comment", "--body", unclassified_body()
                    ],
                    "repo": "senara-solutions/mika"
                }),
            ),
            text_response("Comment posted."),
        ],
        Box::new(CiGatedRunGhTool {
            checks: Ok(RED_CHECKS_2439.to_string()),
        }),
    )
    .await;

    let trace = harness.run("Review PR 2439").await.unwrap();
    let output = trace.calls_for_tool("run_gh")[0]
        .output
        .as_deref()
        .unwrap_or("")
        .to_string();
    assert!(!output.contains(REFUSAL), "got: {output}");
    assert!(gate_outcomes(&harness).await.is_empty());
}

// ---------------------------------------------------------------------------
// AC6 / R5 — every unreadable signal abstains, and says which one.
//
// Four separate controls rather than one, because a disjunction of fail-open
// terms is not proven by neutralising them all at once — the mika#2277 lesson,
// where a predicate reading a single surface would have gone green against a
// fixture that freshened all three.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_failing_reader_abstains_and_names_the_cause() {
    let harness = harness_with(
        vec![
            tool_call_response(
                "run_gh",
                json!({
                    "command": ["pr", "review", "2439", "--approve", "--body", pass_body()],
                    "repo": "senara-solutions/mika"
                }),
            ),
            text_response("Approved."),
        ],
        Box::new(CiGatedRunGhTool {
            checks: Err("gh exit code 1: HTTP 502".to_string()),
        }),
    )
    .await;

    let trace = harness.run("Review PR 2439").await.unwrap();
    let output = trace.calls_for_tool("run_gh")[0]
        .output
        .as_deref()
        .unwrap_or("")
        .to_string();
    assert!(
        !output.contains(REFUSAL),
        "an unreadable CI signal is never a satisfied term — it must not refuse; got: {output}"
    );
    assert_eq!(gate_outcomes(&harness).await, vec!["abstained".to_string()]);
    assert_eq!(
        gate_reasonings(&harness).await,
        vec![CiAbstention::GH_FAILED.to_string()],
        "the abstention must name its cause, or it is indistinguishable from a healthy \
         acceptance (mika#2205)"
    );
}

#[tokio::test]
async fn an_unparseable_reader_answer_abstains_under_its_own_name() {
    let harness = harness_with(
        vec![
            tool_call_response(
                "run_gh",
                json!({
                    "command": ["pr", "review", "2439", "--approve", "--body", pass_body()],
                    "repo": "senara-solutions/mika"
                }),
            ),
            text_response("Approved."),
        ],
        Box::new(CiGatedRunGhTool {
            checks: Ok("{\"not\": \"a list of checks\"}".to_string()),
        }),
    )
    .await;

    harness.run("Review PR 2439").await.unwrap();
    // `unparseable` and `gh_failed` must stay countable apart: one says the call
    // did not go through, the other that it did and answered something we cannot
    // read. Two causes, two remedies.
    assert_eq!(
        gate_reasonings(&harness).await,
        vec![CiAbstention::UNPARSEABLE.to_string()]
    );
}

#[tokio::test]
async fn a_review_without_a_repo_abstains_rather_than_guessing_one() {
    let harness = harness_with(
        vec![
            tool_call_response(
                "run_gh",
                json!({"command": ["pr", "review", "2439", "--approve", "--body", pass_body()]}),
            ),
            text_response("Approved."),
        ],
        Box::new(CiGatedRunGhTool {
            checks: Ok(RED_CHECKS_2439.to_string()),
        }),
    )
    .await;

    let trace = harness.run("Review PR 2439").await.unwrap();
    let output = trace.calls_for_tool("run_gh")[0]
        .output
        .as_deref()
        .unwrap_or("")
        .to_string();
    assert!(!output.contains(REFUSAL), "got: {output}");
    assert_eq!(
        gate_reasonings(&harness).await,
        vec![CiAbstention::NO_REPO.to_string()]
    );
}

#[tokio::test]
async fn a_review_whose_target_is_not_a_number_abstains() {
    // `gh pr review` with no positional argument reviews the current branch's
    // PR. The gate cannot address `gh pr checks` without a number, and it does
    // not guess one.
    let harness = harness_with(
        vec![
            tool_call_response(
                "run_gh",
                json!({
                    "command": ["pr", "review", "--approve", "--body", pass_body()],
                    "repo": "senara-solutions/mika"
                }),
            ),
            text_response("Approved."),
        ],
        Box::new(CiGatedRunGhTool {
            checks: Ok(RED_CHECKS_2439.to_string()),
        }),
    )
    .await;

    let trace = harness.run("Review the current PR").await.unwrap();
    let output = trace.calls_for_tool("run_gh")[0]
        .output
        .as_deref()
        .unwrap_or("")
        .to_string();
    assert!(!output.contains(REFUSAL), "got: {output}");
    assert_eq!(
        gate_reasonings(&harness).await,
        vec![CiAbstention::NO_PR_TARGET.to_string()]
    );
}

// ---------------------------------------------------------------------------
// The wiring, observed in the gate's real position inside the `run_gh` chain.
//
// These two use the REAL builtin — no injection anywhere — so what they attest
// is that the gate is branched at all, and branched only for `pass`. They
// cannot observe a refusal: with no token in the harness the production reader
// abstains, which is itself the assertion.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn the_gate_is_reached_from_the_real_run_gh_chain() {
    let harness = harness_without_token(
        vec![
            tool_call_response(
                "run_gh",
                json!({
                    "command": ["pr", "review", "2439", "--approve", "--body", pass_body()],
                    "repo": "senara-solutions/mika"
                }),
            ),
            text_response("Done."),
        ],
        Box::new(BuiltinRunGhTool),
    )
    .await;

    harness.run("Review PR 2439").await.unwrap();

    // The harness threads no GitHub token, so the production path stops at the
    // `no_token` term — which is exactly the fail-open contract, observed on the
    // real chain. A row here proves the branch exists; its absence would mean
    // the gate is never called, whatever the unit tests say.
    assert_eq!(
        gate_reasonings(&harness).await,
        vec![CiAbstention::NO_TOKEN.to_string()],
        "the gate must be reached from `run_gh` itself — if this list is empty the call site \
         is missing and every decision test above is testing a function nobody calls"
    );
}

#[tokio::test]
async fn the_real_chain_does_not_reach_the_gate_for_a_non_pass_verdict() {
    let harness = harness_without_token(
        vec![
            tool_call_response(
                "run_gh",
                json!({
                    "command": ["pr", "review", "2439", "--comment", "--body", hold_body()],
                    "repo": "senara-solutions/mika"
                }),
            ),
            text_response("Done."),
        ],
        Box::new(BuiltinRunGhTool),
    )
    .await;

    harness.run("Review PR 2439").await.unwrap();
    assert!(
        gate_outcomes(&harness).await.is_empty(),
        "a non-pass verdict must not even reach the token check — the negative half of the \
         wiring, without which the test above would also pass against a gate that runs on \
         every review"
    );
}
