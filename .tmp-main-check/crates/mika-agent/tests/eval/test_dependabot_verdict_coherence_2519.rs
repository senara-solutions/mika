//! Production-path coverage for the verdict↔dependabot gate (mika#2519).
//!
//! Layer A (predicate units) lives in `evidence::guards::tests::mika2519`. This
//! file is Layer B: the wiring contract, driven through `run_agent` with
//! `MockLlmProvider` — **no network, no LLM key, and no `gh`**.
//!
//! # The one thing this file cannot have, and how it is worked around
//!
//! The gate's production reader is `gh pr view --json author,title`. A test has
//! no token and no network, so the reader is the one part that must be
//! substituted. The seam sits at the **raw stdout of the subprocess** rather
//! than at the extracted facts: a test that injected an already-parsed
//! `(author, title)` pair would leave the JSON parse and the `author.login`
//! traversal unexercised on the production path — and `author` being an
//! **object** rather than a string is precisely the imprecision mika#2519 had to
//! correct one file over, in Step 1.6. With the seam where it is, every layer
//! below the network is the production one.
//!
//! Two tools are therefore defined here, and the split is what makes the
//! coverage honest:
//!
//! - [`BuiltinRunGhTool`] delegates to `builtin_handlers::execute("run_gh", …)`
//!   verbatim, so the **wiring** — the gate is reached at all, and only for the
//!   verdicts it claims — is observed in its real position at the tail of the
//!   `run_gh` chain. It cannot observe a refusal (the real reader abstains
//!   here), which is what the second tool is for.
//! - [`DependabotGatedRunGhTool`] runs the same gate function with an injected
//!   reader, so the **decision** — refuse B1, refuse B2, allow — is observed
//!   against real `gh pr view` payloads.
//!
//! Neither tool alone would be enough: the first would go green against a gate
//! that decides nothing, the second against a gate nobody calls.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use dashmap::DashMap;
use mika_agent::evidence::guards::{DEPENDABOT_VERDICT_AUDIT_TOOL, DependabotAbstention};
use mika_agent::skills::SkillRegistry;
use mika_agent::skills::builtin_handlers;
use mika_agent::skills::index::SkillEntry;
use mika_agent::skills::manifest::{Constraints, SkillInfo, SkillManifest, Triggers};
use mika_agent::tools::{Tool, ToolContext, ToolOutput, ToolRegistry, default_tools};
use mika_common::claude::ToolDefinition;
use mika_common::llm::mock::*;
use serde_json::json;

use super::harness::EvalHarness;

/// The two refusals' stable identifiers, as the LLM receives them.
const REFUSAL_B1: &str = "dependabot_verdict_unreachable";
const REFUSAL_B2: &str = "dependabot_major_bump_unverified";

/// What `gh pr view --json author,title` answers for #2453 — the witness PR the
/// ticket says must pass. `author` is an **object**; that is the point.
const PR_2453: &str = r#"{
    "author": {"id": "MDM6Qm90", "is_bot": true, "login": "app/dependabot", "name": ""},
    "title": "Bump base64 from 0.22.1 to 0.23.0"
}"#;

/// #2454 — `jsonwebtoken 9.3.1 → 11.1.0`. Build green, `generate_jwt`
/// panicking (mika#2525). The class B2 exists for.
const PR_2454: &str = r#"{
    "author": {"id": "MDM6Qm90", "is_bot": true, "login": "dependabot[bot]", "name": ""},
    "title": "Bump jsonwebtoken from 9.3.1 to 11.1.0"
}"#;

/// A human-authored PR — the negative control for both branches.
const PR_HUMAN: &str = r#"{
    "author": {"id": "MDQ6VXNlcg", "is_bot": false, "login": "samidarko", "name": "Vincent"},
    "title": "fix: mika#2519 — the exemption is held, not written a third time"
}"#;

/// A dependabot grouped bump: no version pair in the title at all, so B2
/// abstains. The named limit of § 8 of the plan.
const PR_GROUPED: &str = r#"{
    "author": {"id": "MDM6Qm90", "is_bot": true, "login": "dependabot[bot]", "name": ""},
    "title": "Bump the cargo group with 5 updates"
}"#;

/// The exact motif the ticket reports, which no guard in this tree emits — and
/// that is the measurement (R3). Used as the refused body so the test replays
/// the incident rather than a reconstruction of it.
fn block_pipeline_body() -> String {
    "VERDICT: block[pipeline]\nDEPTH: code-level\nREASON: Dependabot dependency bump — no plan \
     document or Pipeline-Exempt trailer present. Build verified successfully."
        .to_string()
}

/// A `pass` with the `DEP-REVIEW:` section Step 1.6 requires, and **no**
/// `API-SURFACE:` line.
fn pass_body_without_api_surface() -> String {
    "VERDICT: pass \u{2705}\nDEPTH: code-level\nDEP-REVIEW:\nPackage(s): jsonwebtoken 9.3.1 \
     \u{2192} 11.1.0\nAdvisory query: clean (0 advisories in delta)\nChangelog scan: no \
     breaking-change entry in delta\nSignal: pass"
        .to_string()
}

/// The same `pass`, with the call-site assertion the engine reads.
fn pass_body_with_api_surface() -> String {
    "VERDICT: pass \u{2705}\nDEPTH: code-level\nDEP-REVIEW:\nPackage(s): jsonwebtoken 9.3.1 \
     \u{2192} 11.1.0\nAdvisory query: clean (0 advisories in delta)\nChangelog scan: no \
     breaking-change entry in delta\nAPI-SURFACE: EncodingKey::from_rsa_pem — call sites read: \
     github_app.rs:95, github_app.rs:131, doctor.rs:536 — unaffected\nSignal: pass"
        .to_string()
}

fn pass_body_plain() -> String {
    "VERDICT: pass \u{2705}\nDEPTH: code-level\nDEP-REVIEW:\nPackage(s): base64 0.22.1 \
     \u{2192} 0.23.0\nAdvisory query: clean (0 advisories in delta)\nSignal: pass"
        .to_string()
}

fn hold_body() -> String {
    "VERDICT: hold[review] \u{23f8}\u{fe0f}\nDEPTH: code-level\nREASON: advisory query failed."
        .to_string()
}

fn block_dependency_body() -> String {
    "VERDICT: block[dependency] \u{274c}\nDEPTH: code-level\nDEP-REVIEW:\nPackage(s): \
     jsonwebtoken 9.3.1 \u{2192} 11.1.0\nChangelog scan: BREAKING: ring backend removed\nSignal: \
     block[dependency]"
        .to_string()
}

/// A body with no `VERDICT:` line at all — a human or ad-hoc review.
fn unclassified_body() -> String {
    "DEPTH: code-level\nLooks reasonable to me, shipping notes below.".to_string()
}

/// The pipeline class in upper case. Pins the parity with `verdict_handler`,
/// which reads `reason.to_lowercase().as_str()`: reading the class less
/// permissively here would let the measured defect through under another case.
fn block_pipeline_body_uppercase() -> String {
    "VERDICT: block[PIPELINE]\nDEPTH: code-level\nREASON: no plan document.".to_string()
}

/// `block[ci]` — a neighbouring class the guard must not touch. Its own gate is
/// mika#2455's, and stealing its population here would refuse a verdict
/// `handle_block_ci` is waiting to route to a bounded CI-fix pilot.
fn block_ci_body() -> String {
    "VERDICT: block[ci] \u{274c}\nDEPTH: code-level\nREASON: Check is red.".to_string()
}

// ---------------------------------------------------------------------------
// The two tools
// ---------------------------------------------------------------------------

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

/// The real builtin, reached through the agent loop — wiring coverage.
struct BuiltinRunGhTool;

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
struct DependabotGatedRunGhTool {
    /// What the substituted `gh pr view` answers.
    pr: Result<String, String>,
}

#[async_trait]
impl Tool for DependabotGatedRunGhTool {
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
        let answer = self.pr.clone();

        if let Err(refusal) = builtin_handlers::validate_dependabot_verdict_coherence_with_reader(
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

/// Never used to reach the network — the reader is always injected on the
/// decision path.
const FAKE_TOKEN: &str = "ghp_fake_token_for_mika2519";

/// A harness carrying a GitHub token — required by every **decision** test.
///
/// The token is one of the gate's fail-open terms: without it the gate abstains
/// on `no_token` *before* the reader is ever consulted, and every decision test
/// would go green for the wrong reason. Setting `settings.github_token` directly
/// is deliberate and is the same note mika#2455 left here: the builder's
/// `.github_token()` is inert on a harness-driven turn, because `agent_loop`
/// only consults `params.github_token` when `settings` is `None` and the harness
/// always passes `Some(&self.settings)`.
async fn harness_with(responses: Vec<MockResponse>, tool: Box<dyn Tool>) -> EvalHarness {
    let mut harness = harness_inner(responses, tool, Some(FAKE_TOKEN)).await;
    harness.settings.github_token = Some(secrecy::SecretString::from(FAKE_TOKEN));
    harness
}

/// A harness with **no** token — used by the wiring tests, where the production
/// reader's abstention is the observable.
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
        .filter(|e| e.tool_name == DEPENDABOT_VERDICT_AUDIT_TOOL)
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
        .filter(|e| e.tool_name == DEPENDABOT_VERDICT_AUDIT_TOOL)
        .filter_map(|e| e.reasoning.clone())
        .collect()
}

/// One review call, and the gate's answer to it.
async fn post_review(
    pr: &str,
    number: &str,
    body: String,
    flag: &str,
) -> (EvalHarness, String, Vec<String>) {
    let harness = harness_with(
        vec![
            tool_call_response(
                "run_gh",
                json!({
                    "command": ["pr", "review", number, flag, "--body", body],
                    "repo": "senara-solutions/mika"
                }),
            ),
            text_response("Done."),
        ],
        Box::new(DependabotGatedRunGhTool {
            pr: Ok(pr.to_string()),
        }),
    )
    .await;

    let trace = harness.run(&format!("Review PR {number}")).await.unwrap();
    let calls = trace.calls_for_tool("run_gh");
    assert!(!calls.is_empty(), "expected the review call");
    let output = calls[0].output.as_deref().unwrap_or("").to_string();
    let outcomes = gate_outcomes(&harness).await;
    (harness, output, outcomes)
}

// ---------------------------------------------------------------------------
// AC5 / B1 — the measured symptom, replayed: a `block[pipeline]` on #2453.
// ---------------------------------------------------------------------------

/// The founding measurement (2026-09-24): mika-qa posted
/// `VERDICT: block[pipeline]` on a build-green Dependabot Cargo-only PR, with a
/// motif no guard in this tree emits.
///
/// The refusal must happen **before the subprocess** — that is the whole reason
/// the gate is here and not at EndTurn. The tool output being the refusal, and
/// the injected reader having been the only `gh`-shaped thing consulted, is the
/// observable: nothing was posted.
#[tokio::test]
async fn a_block_pipeline_on_a_dependabot_pr_is_refused_before_the_subprocess() {
    let (harness, output, outcomes) =
        post_review(PR_2453, "2453", block_pipeline_body(), "--comment").await;

    assert!(
        output.contains(REFUSAL_B1),
        "a `block[pipeline]` on an automated-author PR must be refused; got: {output}"
    );
    assert!(
        output.contains("mika#2519"),
        "the refusal must carry its doctrine reference; got: {output}"
    );
    // The refusal is self-sufficient: it names the author it saw and the step
    // that makes the verdict unreachable, so the rewrite is not blind.
    assert!(
        output.contains("app/dependabot"),
        "the refusal must name the author it read — from `author.login`, which is \
         the field Step 1.6 used to compare as a string; got: {output}"
    );
    assert!(
        output.contains("Step 1.6"),
        "the refusal must name the step whose skip makes this verdict \
         unreachable; got: {output}"
    );
    // AC5 — both correct ways out are named. Naming only "re-run Step 1.6" would
    // leave no exit for a review whose guard genuinely did fail.
    assert!(
        output.contains("DEP-REVIEW:"),
        "the refusal must name the section Step 1.6 produces; got: {output}"
    );
    assert!(
        output.contains("VERBATIM") || output.contains("verbatim"),
        "the refusal must name the Step 2E requirement as the second way out; \
         got: {output}"
    );

    assert!(
        outcomes.contains(&"refused_unreachable_pipeline_block".to_string()),
        "expected the B1 audit row; got {outcomes:?}"
    );
    let reasonings = gate_reasonings(&harness).await;
    assert!(
        reasonings.iter().any(|r| r.contains("app/dependabot")),
        "the audit row must carry the author, so the population is attributable; \
         got {reasonings:?}"
    );
}

/// **Negative control 1 of § 4.** A human-authored PR keeps the right to a
/// `block[pipeline]` — it is the nominal verdict when a repo guard exits
/// non-zero, and refusing it would break the pipeline gate outright.
#[tokio::test]
async fn a_block_pipeline_on_a_human_pr_goes_through() {
    let (_h, output, outcomes) =
        post_review(PR_HUMAN, "2530", block_pipeline_body(), "--comment").await;

    assert!(
        !output.contains(REFUSAL_B1),
        "a human-authored PR must not be refused; got: {output}"
    );
    assert!(
        output.contains("review posted"),
        "the review must go through; got: {output}"
    );
    // AC-V2 — the nominal decision is written down, so "zero refusals" tells a
    // healthy rail from an inert guard (mika#2205, probe S5's positive control).
    assert!(
        outcomes.contains(&"allowed".to_string()),
        "expected an `allowed` audit row; got {outcomes:?}"
    );
}

/// **Negative control 2 of § 4.** On a Dependabot PR, every other classified
/// verdict passes intact — `hold[review]` in particular, which is the fail-closed
/// output Step 1.6 itself prescribes when the advisory query cannot be verified.
#[tokio::test]
async fn a_hold_review_on_a_dependabot_pr_goes_through() {
    let (_h, output, outcomes) = post_review(PR_2453, "2453", hold_body(), "--comment").await;

    assert!(
        !output.contains(REFUSAL_B1) && !output.contains(REFUSAL_B2),
        "`hold[review]` must pass intact on a Dependabot PR; got: {output}"
    );
    assert!(output.contains("review posted"), "got: {output}");
    // It is out of the population entirely: the gate never even read the PR, so
    // it writes nothing. That silence is what keeps the nominal traffic out of
    // the ledger (mika#2131).
    assert!(
        outcomes.is_empty(),
        "an unconcerned verdict must not be audited at all; got {outcomes:?}"
    );
}

/// The class is read with the case folded, exactly as `verdict_handler` reads it
/// (`reason.to_lowercase().as_str()`).
///
/// Reading it less permissively than the state machine would let the measured
/// defect through under another spelling; reading it MORE permissively — with a
/// `trim`, say — would refuse a verdict `verdict_handler` then routes to its
/// `_ => warn` arm, i.e. refuse a review for a class nobody downstream knows.
#[tokio::test]
async fn the_pipeline_class_is_read_with_the_case_folded() {
    let (_h, output, outcomes) = post_review(
        PR_2453,
        "2453",
        block_pipeline_body_uppercase(),
        "--comment",
    )
    .await;

    assert!(
        output.contains(REFUSAL_B1),
        "`block[PIPELINE]` is the same class to `verdict_handler`; got: {output}"
    );
    assert!(
        outcomes.contains(&"refused_unreachable_pipeline_block".to_string()),
        "got {outcomes:?}"
    );
}

/// `block[ci]` belongs to mika#2455's gate, not this one. Stealing its
/// population would refuse a verdict `handle_block_ci` is waiting to route.
#[tokio::test]
async fn a_block_ci_on_a_dependabot_pr_is_none_of_this_guards_business() {
    let (_h, output, outcomes) = post_review(PR_2453, "2453", block_ci_body(), "--comment").await;

    assert!(
        !output.contains(REFUSAL_B1) && !output.contains(REFUSAL_B2),
        "`block[ci]` must pass intact; got: {output}"
    );
    assert!(
        outcomes.is_empty(),
        "out of the population entirely — no read, no row; got {outcomes:?}"
    );
}

/// `block[dependency]` is the verdict Step 1.6 routes a breaking bump to. It
/// must pass — the whole point of B2's refusal is to send the model here.
#[tokio::test]
async fn a_block_dependency_on_a_major_bump_goes_through() {
    let (_h, output, _outcomes) =
        post_review(PR_2454, "2454", block_dependency_body(), "--comment").await;

    assert!(
        !output.contains(REFUSAL_B1) && !output.contains(REFUSAL_B2),
        "`block[dependency]` is the named way out of B2 and must be postable; \
         got: {output}"
    );
    assert!(output.contains("review posted"), "got: {output}");
}

// ---------------------------------------------------------------------------
// AC6 / B2 — a `pass` on a major bump without a call-site assertion.
// ---------------------------------------------------------------------------

/// #2454's class: `jsonwebtoken 9.3.1 → 11.1.0`, a clean advisory query, a
/// green build — and a runtime panic three call sites away (mika#2525).
#[tokio::test]
async fn a_pass_on_an_unverified_major_bump_is_refused() {
    let (harness, output, outcomes) = post_review(
        PR_2454,
        "2454",
        pass_body_without_api_surface(),
        "--approve",
    )
    .await;

    assert!(
        output.contains(REFUSAL_B2),
        "a `pass` on a major bump with no call-site assertion must be refused; \
         got: {output}"
    );
    // The refusal is self-sufficient: it names the package and the delta, so the
    // model knows what to go and read.
    assert!(
        output.contains("jsonwebtoken") && output.contains("9.3.1") && output.contains("11.1.0"),
        "the refusal must name the package and the delta; got: {output}"
    );
    // AC6 — both correct ways out are named, and the first one names the line the
    // engine reads. Naming only `block[dependency]` would push the model to
    // block a bump it has not looked at.
    assert!(
        output.contains("API-SURFACE:"),
        "the refusal must name the line it reads; got: {output}"
    );
    assert!(
        output.contains("block[dependency]"),
        "the refusal must name the blocking verdict as the second way out; \
         got: {output}"
    );
    // The measured counter-example is in the body, so the model is not asked to
    // take the requirement on trust against a green build.
    assert!(
        output.contains("mika#2454") || output.contains("mika#2525"),
        "the refusal must cite the measurement that makes a green build \
         insufficient here; got: {output}"
    );

    assert!(
        outcomes.contains(&"refused_unverified_major_bump".to_string()),
        "expected the B2 audit row; got {outcomes:?}"
    );
    let reasonings = gate_reasonings(&harness).await;
    assert!(
        reasonings.iter().any(|r| r.contains("jsonwebtoken")),
        "the audit row must carry the package; got {reasonings:?}"
    );
}

/// **Negative control 3 of § 4, and the most important one.** #2453 —
/// `base64 0.22.1 → 0.23.0` — is the witness PR this ticket exists to unblock.
/// The first numeric segment is `0` on both sides, so it is not a major jump and
/// the `pass` must go through.
#[tokio::test]
async fn a_pass_on_the_witness_pr_2453_goes_through() {
    let (_h, output, outcomes) = post_review(PR_2453, "2453", pass_body_plain(), "--approve").await;

    assert!(
        !output.contains(REFUSAL_B2),
        "#2453 is the PR this ticket exists to unblock — refusing it would close \
         the rail it opens; got: {output}"
    );
    assert!(output.contains("review posted"), "got: {output}");
    assert!(
        outcomes.contains(&"allowed".to_string()),
        "expected an `allowed` audit row; got {outcomes:?}"
    );
}

/// **Negative control 4 of § 4.** A major bump **with** the assertion passes:
/// the guard demands a claim, it does not forbid the verdict.
#[tokio::test]
async fn a_pass_on_a_major_bump_with_the_assertion_goes_through() {
    let (_h, output, outcomes) =
        post_review(PR_2454, "2454", pass_body_with_api_surface(), "--approve").await;

    assert!(
        !output.contains(REFUSAL_B2),
        "the assertion is what the guard asks for; got: {output}"
    );
    assert!(output.contains("review posted"), "got: {output}");
    assert!(
        outcomes.contains(&"allowed".to_string()),
        "expected an `allowed` audit row; got {outcomes:?}"
    );
}

/// A grouped bump carries no version pair in its title, so B2 abstains rather
/// than guessing from a third party's markdown table. The named limit of § 8.
#[tokio::test]
async fn a_pass_on_a_grouped_bump_goes_through_and_is_recorded_as_allowed() {
    let (_h, output, outcomes) =
        post_review(PR_GROUPED, "2500", pass_body_plain(), "--approve").await;

    assert!(
        !output.contains(REFUSAL_B2),
        "a grouped bump's title carries no pair — abstain, never refuse; \
         got: {output}"
    );
    assert!(
        outcomes.contains(&"allowed".to_string()),
        "the abstention is recorded as an accepted decision, so the population \
         stays countable; got {outcomes:?}"
    );
}

// ---------------------------------------------------------------------------
// AC7 / V3 — every unreadable term abstains, and says so.
// ---------------------------------------------------------------------------

/// A reader that fails: the verdict goes through, and the abstention is named.
#[tokio::test]
async fn a_failing_reader_abstains_and_the_review_goes_through() {
    let harness = harness_with(
        vec![
            tool_call_response(
                "run_gh",
                json!({
                    "command": ["pr", "review", "2453", "--comment", "--body", block_pipeline_body()],
                    "repo": "senara-solutions/mika"
                }),
            ),
            text_response("Posted."),
        ],
        Box::new(DependabotGatedRunGhTool {
            pr: Err("gh: command not found".to_string()),
        }),
    )
    .await;

    let trace = harness.run("Review PR 2453").await.unwrap();
    let output = trace.calls_for_tool("run_gh")[0]
        .output
        .as_deref()
        .unwrap_or("")
        .to_string();
    assert!(
        !output.contains(REFUSAL_B1),
        "a guard that could not evaluate its term refuses nothing (U6); got: {output}"
    );

    let outcomes = gate_outcomes(&harness).await;
    assert!(
        outcomes.contains(&"abstained".to_string()),
        "the abstention must be recorded; got {outcomes:?}"
    );
    let reasonings = gate_reasonings(&harness).await;
    assert!(
        reasonings.contains(&DependabotAbstention::GH_FAILED.to_string()),
        "the abstention must name ITS cause — six causes, six remedies \
         (mika#2131); got {reasonings:?}"
    );
}

/// Output that is not JSON, and output whose `author` is a **string** rather
/// than an object — the very shape Step 1.6 used to assume.
#[tokio::test]
async fn unparseable_output_abstains_on_its_own_named_cause() {
    for raw in [
        "not json at all".to_string(),
        // `author` as a bare string: parses as JSON, carries no `login`.
        r#"{"author": "dependabot[bot]", "title": "Bump x from 1 to 2"}"#.to_string(),
        // No `author` key at all.
        r#"{"title": "Bump x from 1 to 2"}"#.to_string(),
    ] {
        let harness = harness_with(
            vec![
                tool_call_response(
                    "run_gh",
                    json!({
                        "command": ["pr", "review", "2453", "--comment", "--body", block_pipeline_body()],
                        "repo": "senara-solutions/mika"
                    }),
                ),
                text_response("Posted."),
            ],
            Box::new(DependabotGatedRunGhTool { pr: Ok(raw.clone()) }),
        )
        .await;

        let trace = harness.run("Review PR 2453").await.unwrap();
        let output = trace.calls_for_tool("run_gh")[0]
            .output
            .as_deref()
            .unwrap_or("")
            .to_string();
        assert!(
            !output.contains(REFUSAL_B1),
            "unreadable output must abstain, not refuse — raw: {raw}; got: {output}"
        );
        let reasonings = gate_reasonings(&harness).await;
        assert!(
            reasonings.contains(&DependabotAbstention::UNPARSEABLE.to_string()),
            "raw: {raw}; got {reasonings:?}"
        );
    }
}

/// An absent `title` is **not** an abstention: B1 does not read the title, so it
/// must still bite. Without this the two branches would share a fail-open term
/// that only one of them needs.
#[tokio::test]
async fn an_absent_title_does_not_disarm_b1() {
    let (_h, output, outcomes) = post_review(
        r#"{"author": {"login": "dependabot[bot]"}}"#,
        "2453",
        block_pipeline_body(),
        "--comment",
    )
    .await;

    assert!(
        output.contains(REFUSAL_B1),
        "B1 reads the author alone — a missing title must not disarm it; \
         got: {output}"
    );
    assert!(
        outcomes.contains(&"refused_unreachable_pipeline_block".to_string()),
        "got {outcomes:?}"
    );
}

/// …and the mirror: with no title, B2 abstains rather than guessing.
#[tokio::test]
async fn an_absent_title_abstains_on_b2() {
    let (_h, output, outcomes) = post_review(
        r#"{"author": {"login": "dependabot[bot]"}}"#,
        "2454",
        pass_body_without_api_surface(),
        "--approve",
    )
    .await;

    assert!(
        !output.contains(REFUSAL_B2),
        "with no title there is no readable delta — abstain; got: {output}"
    );
    assert!(
        outcomes.contains(&"allowed".to_string()),
        "got {outcomes:?}"
    );
}

// ---------------------------------------------------------------------------
// Wiring — the gate is reached at its real position, and only for the verdicts
// it claims. Driven through the REAL builtin, with no token, so the production
// reader's own abstention is the observable.
// ---------------------------------------------------------------------------

/// The gate is reached at the tail of the `run_gh` chain for a
/// `block[pipeline]`, and abstains there because no token is resolved.
///
/// This is the half the injected-reader tests cannot give: it observes that
/// `run_gh` calls the gate at all, in its real position, after mika#275,
/// mika#1196, mika#2237 and mika#2455.
#[tokio::test]
async fn the_gate_is_reached_from_the_real_run_gh_chain() {
    let harness = harness_without_token(
        vec![
            tool_call_response(
                "run_gh",
                json!({
                    "command": ["pr", "review", "2453", "--comment", "--body", block_pipeline_body()],
                    "repo": "senara-solutions/mika"
                }),
            ),
            text_response("Done."),
        ],
        Box::new(BuiltinRunGhTool),
    )
    .await;

    harness.run("Review PR 2453").await.unwrap();

    let reasonings = gate_reasonings(&harness).await;
    assert!(
        reasonings.contains(&DependabotAbstention::NO_TOKEN.to_string()),
        "the gate must be reached from the real chain and abstain on `no_token`; \
         got {reasonings:?}"
    );
}

/// …and it is **not** reached for a verdict outside its population. A body with
/// no `VERDICT:` line spends no network call and writes no row.
#[tokio::test]
async fn an_unclassified_body_never_reaches_the_reader() {
    let harness = harness_without_token(
        vec![
            tool_call_response(
                "run_gh",
                json!({
                    "command": ["pr", "review", "2453", "--comment", "--body", unclassified_body()],
                    "repo": "senara-solutions/mika"
                }),
            ),
            text_response("Done."),
        ],
        Box::new(BuiltinRunGhTool),
    )
    .await;

    harness.run("Review PR 2453").await.unwrap();

    let outcomes = gate_outcomes(&harness).await;
    assert!(
        outcomes.is_empty(),
        "a body with no classified verdict is out of the population — no row, no \
         read (fail-open recognition, by increasing cost); got {outcomes:?}"
    );
}
