//! Production-path coverage for the build-evidence branch B3 (mika#2565).
//!
//! Layer A (predicate units) lives in `evidence::guards::tests::mika2565`. This
//! file is Layer B: the wiring contract, driven through `run_agent` with
//! `MockLlmProvider` — **no network, no LLM key, and no `gh`**. It is the
//! sibling of `test_dependabot_verdict_coherence_2519.rs` and borrows its seam
//! wholesale: the substitution happens at the **raw stdout of the subprocess**,
//! so the JSON parse, the `author.login` traversal and the new `files[].path`
//! traversal are all the production ones.
//!
//! # What this file measures that Layer A cannot
//!
//! Layer A proves `classify_dependabot_verdict` refuses the right tuple. It says
//! nothing about where the seventh argument comes from. The whole of mika#2565
//! rests on that argument being a **fact the engine recorded itself** — a
//! `build_mika` row in `tool_calls` for this session — rather than a sentence in
//! the verdict body, because mika#2519 measured what happens when a guard asks
//! the model for an assertion: it gets one, sincere, detailed, and insufficient.
//!
//! So the tests here drive the real `builtin_handlers` gate against a real
//! database, and the three observables are:
//!
//! - a `pass` on a Cargo diff with an empty `tool_calls` is **refused**, even
//!   when its body carries the `API-SURFACE:` line that satisfies B2 — the test
//!   that distinguishes this work from mika#2519;
//! - the same body with a real `build_mika` row inserted **goes through** —
//!   without it, "B3 refuses" and "B3 refuses everything" are indistinguishable;
//! - a body cannot buy its way through by *claiming* a build (AC6).

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use dashmap::DashMap;
use mika_agent::evidence::guards::{DEPENDABOT_VERDICT_AUDIT_TOOL, DependabotAbstention};
use mika_agent::qa_build_callback::BUILD_MIKA_TOOL;
use mika_agent::skills::SkillRegistry;
use mika_agent::skills::builtin_handlers;
use mika_agent::skills::index::SkillEntry;
use mika_agent::skills::manifest::{Constraints, SkillInfo, SkillManifest, Triggers};
use mika_agent::tools::{Tool, ToolContext, ToolOutput, ToolRegistry, default_tools};
use mika_common::claude::ToolDefinition;
use mika_common::llm::mock::*;
use serde_json::json;

use super::harness::EvalHarness;

/// B3's stable identifier, as the LLM receives it.
const REFUSAL_B3: &str = "dependabot_cargo_bump_unbuilt";
/// B2's, so the precedence assertions can name what they are *not* seeing.
const REFUSAL_B2: &str = "dependabot_major_bump_unverified";

// ---------------------------------------------------------------------------
// The `gh pr view --json author,title,files` payloads
// ---------------------------------------------------------------------------

/// **#2561, verbatim in shape.** `utoipa 5.5.0 → 6.0.0`, Cargo-only diff. Four
/// `pass` verdicts were posted on this PR with `Check` red behind them.
const PR_2561: &str = r#"{
    "author": {"id": "MDM6Qm90", "is_bot": true, "login": "app/dependabot", "name": ""},
    "title": "Bump utoipa from 5.5.0 to 6.0.0",
    "files": [
        {"path": "Cargo.toml", "additions": 1, "deletions": 1},
        {"path": "Cargo.lock", "additions": 12, "deletions": 12}
    ]
}"#;

/// **#2560.** `sha2 0.10.9 → 0.11.0` — `NoMajorJump`, so B2 never applied, and
/// the three measured `pass` verdicts carried no `API-SURFACE:` line at all.
/// The incompatibility was transverse and showed on no call site.
const PR_2560: &str = r#"{
    "author": {"id": "MDM6Qm90", "is_bot": true, "login": "dependabot[bot]", "name": ""},
    "title": "Bump sha2 from 0.10.9 to 0.11.0",
    "files": [
        {"path": "Cargo.toml", "additions": 2, "deletions": 2},
        {"path": "Cargo.lock", "additions": 20, "deletions": 18}
    ]
}"#;

/// **AC3's payload.** A dependency PR whose diff carries no Rust dependency
/// resolution: out of B3's population, and `build_mika` could not compile it
/// anyway. The bump is deliberately **minor**, so B2 is not in play either and
/// the test measures the term it names.
const PR_ACTIONS: &str = r#"{
    "author": {"id": "MDM6Qm90", "is_bot": true, "login": "app/dependabot", "name": ""},
    "title": "Bump actions/checkout from 4.1.1 to 4.2.0",
    "files": [
        {"path": ".github/workflows/ci.yml", "additions": 2, "deletions": 2}
    ]
}"#;

/// The negative control of sonde S5: a human-authored PR touching Cargo. B3's
/// first term is the automated author, and an occurrence outside that class
/// would mean it has been relaxed.
const PR_HUMAN_CARGO: &str = r#"{
    "author": {"id": "MDQ6VXNlcg", "is_bot": false, "login": "samidarko", "name": "Vincent"},
    "title": "feat: mika#2565 — a Rust bump is verified by compiling",
    "files": [
        {"path": "Cargo.toml", "additions": 1, "deletions": 1},
        {"path": "crates/mika-agent/src/evidence/guards.rs", "additions": 90, "deletions": 4}
    ]
}"#;

/// **The `no_files` population (sonde S4).** `gh` answered, the author is
/// readable, and `files` is absent — an older `gh`, a field rename, a paginated
/// response. Fail-open, under a name of its own.
const PR_NO_FILES: &str = r#"{
    "author": {"id": "MDM6Qm90", "is_bot": true, "login": "app/dependabot", "name": ""},
    "title": "Bump utoipa from 5.5.0 to 6.0.0"
}"#;

/// `files` present but empty — same abstention, and it must not read as "no
/// Cargo file, therefore allowed", which would be a silent inversion.
const PR_EMPTY_FILES: &str = r#"{
    "author": {"id": "MDM6Qm90", "is_bot": true, "login": "app/dependabot", "name": ""},
    "title": "Bump utoipa from 5.5.0 to 6.0.0",
    "files": []
}"#;

// ---------------------------------------------------------------------------
// The verdict bodies
// ---------------------------------------------------------------------------

/// **The body that mika#2519 lets through and mika#2565 must not.**
///
/// It carries the `API-SURFACE:` line — eleven lines' worth in the real #2561,
/// abridged here — so B2 is satisfied. That is the point: the reading was real,
/// sourced by grep, and wrong on the conclusion.
fn pass_body_with_api_surface() -> String {
    "VERDICT: pass \u{2705}\nDEPTH: code-level\nDEP-REVIEW:\nPackage(s): utoipa 5.5.0 \
     \u{2192} 6.0.0\nAdvisory query: clean (0 advisories in delta)\nChangelog scan: no \
     breaking-change entry in delta\nAPI-SURFACE: 29 #[derive(ToSchema)] sites (mika-agent: \
     27, mika-gateway: 2) — 14 #[utoipa::path(...)] call sites — 0 #[into_params] usages — \
     unaffected\nBUILD VERIFICATION: skipped (pipeline-exempt label)\nSignal: pass"
        .to_string()
}

/// #2560's body: no `API-SURFACE:` line, and none was required — `0.10 → 0.11`
/// is not a major jump.
fn pass_body_2560() -> String {
    "VERDICT: pass \u{2705}\nDEPTH: code-level\nDEP-REVIEW:\nPackage(s): sha2 0.10.9 \
     \u{2192} 0.11.0\nAdvisory query: clean (0 advisories in delta)\nChangelog scan: no \
     breaking-change entry in delta\nBUILD VERIFICATION: skipped (pipeline-exempt \
     label)\nSignal: pass"
        .to_string()
}

fn pass_body_actions() -> String {
    "VERDICT: pass \u{2705}\nDEPTH: code-level\nDEP-REVIEW:\nPackage(s): actions/checkout \
     4.1.1 \u{2192} 4.2.0\nAdvisory query: clean (0 advisories in delta)\nBUILD \
     VERIFICATION: skipped (dependency PR, no Rust dependency resolution in the \
     diff)\nSignal: pass"
        .to_string()
}

/// **AC6's body.** It *claims* a green build, in the exact wording Step 1.6
/// item 8 prescribes. No `build_mika` row backs it.
fn pass_body_claiming_a_build() -> String {
    "VERDICT: pass \u{2705}\nDEPTH: code-level\nDEP-REVIEW:\nPackage(s): utoipa 5.5.0 \
     \u{2192} 6.0.0\nAdvisory query: clean (0 advisories in delta)\nAPI-SURFACE: 29 \
     ToSchema sites read — unaffected\nBUILD VERIFICATION: Build: pass\nBUILD-VERIFIED: \
     yes\nSignal: pass"
        .to_string()
}

/// The fail-closed exit B3's own refusal prescribes. It must not be refused in
/// turn, or the review would be walled in.
fn hold_body() -> String {
    "VERDICT: hold[review] \u{23f8}\u{fe0f}\nDEPTH: code-level\nREASON: worktree could not be \
     created: fatal: invalid reference."
        .to_string()
}

/// The other prescribed exit: a red build.
fn block_dependency_body() -> String {
    "VERDICT: block[dependency] \u{274c}\nDEPTH: code-level\nDEP-REVIEW:\nPackage(s): utoipa \
     5.5.0 \u{2192} 6.0.0\nBUILD VERIFICATION: Build: fail — error[E0432]: unresolved import \
     `utoipa::IntoParams`\nSignal: block[dependency]"
        .to_string()
}

// ---------------------------------------------------------------------------
// The tool
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

/// The real gate, with the subprocess substituted — decision coverage.
///
/// Everything below the network is production: the same
/// `validate_dependabot_verdict_coherence_with_reader`, the same JSON parse, the
/// same `find_recent_build_invocation` against the same real database.
struct DependabotGatedRunGhTool {
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

const FAKE_TOKEN: &str = "ghp_fake_token_for_mika2565";

/// The token is one of the gate's fail-open terms: without it the gate abstains
/// on `no_token` *before* the reader is consulted, and every test here would go
/// green for the wrong reason. Setting `settings.github_token` directly is the
/// note mika#2455 left in the sibling file — the builder's `.github_token()` is
/// inert on a harness-driven turn.
async fn harness_with(responses: Vec<MockResponse>, tool: Box<dyn Tool>) -> EvalHarness {
    let mut registry: ToolRegistry = default_tools();
    registry.register(tool);

    let mut harness = EvalHarness::builder()
        .responses(responses)
        .tools(registry)
        .pr_reviews_posted(Arc::new(DashMap::<String, HashSet<String>>::new()))
        .github_token(FAKE_TOKEN)
        .build()
        .await
        .unwrap();
    harness.settings.github_token = Some(secrecy::SecretString::from(FAKE_TOKEN));

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

/// The abstention **causes**, which mika#2519 writes to `reasoning` while
/// `after_value` carries the bare `abstained`. The plan's operator table reads
/// them as the pair `abstained` / `no_files`, and this is that pair's second
/// half.
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

/// Insert a real `build_mika` row in this session — the engine fact B3 reads.
///
/// This is the whole of the **positive control**, and it goes through
/// `save_tool_call`, the production writer, rather than a hand-rolled INSERT: a
/// fabricated row could satisfy a query the real writer never produces.
async fn record_a_build(harness: &EvalHarness, cwd: &str) {
    harness
        .db
        .save_tool_call(
            &format!("tc-{}", uuid::Uuid::new_v4()),
            &harness.session_id,
            None,
            None,
            3,
            BUILD_MIKA_TOOL,
            "builtin",
            Some("qa-review"),
            Some(&json!({ "cwd": cwd }).to_string()),
            Some("Build started, task id 1234"),
            true,
            false,
            42,
            None,
        )
        .await
        .expect("the build row must be recorded");
}

/// One review call, and the gate's answer to it.
async fn post_review(
    pr: &str,
    number: &str,
    body: String,
    flag: &str,
    with_build: bool,
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

    if with_build {
        record_a_build(
            &harness,
            "~/workspace/mika-platform/.claude/worktrees/dependabot-cargo-utoipa-6.0.0/mika/",
        )
        .await;
    }

    let trace = harness.run(&format!("Review PR {number}")).await.unwrap();
    let calls = trace.calls_for_tool("run_gh");
    assert!(!calls.is_empty(), "expected the review call");
    let output = calls[0].output.as_deref().unwrap_or("").to_string();
    let outcomes = gate_outcomes(&harness).await;
    (harness, output, outcomes)
}

// ---------------------------------------------------------------------------
// AC1 / AC2 / B3 — the measured defect, replayed
// ---------------------------------------------------------------------------

/// **The test that distinguishes this work from mika#2519.**
///
/// The body carries a real `API-SURFACE:` line, so B2 is satisfied and would
/// let it through. There is no `build_mika` row. The verdict is refused anyway
/// — and the refusal names the build, not the assertion.
///
/// This is #2561 to the byte: four `pass` verdicts, an eleven-line API reading
/// sourced by grep, and `Check` red for 6 min 21 s behind them.
#[tokio::test]
async fn a_pass_on_a_cargo_bump_with_no_build_is_refused_even_with_the_api_assertion() {
    let (_h, output, outcomes) = post_review(
        PR_2561,
        "2561",
        pass_body_with_api_surface(),
        "--approve",
        false,
    )
    .await;

    assert!(
        output.contains(REFUSAL_B3),
        "a `pass` on a Cargo diff with no build must be refused; got: {output}"
    );
    assert!(
        !output.contains(REFUSAL_B2),
        "the refusal must name the BUILD, not the API assertion — the assertion is \
         present and satisfied here, and sending the model to rewrite it would send \
         it to work on what was already not the problem; got: {output}"
    );
    assert!(
        output.contains("mika#2565"),
        "the refusal must carry its doctrine reference; got: {output}"
    );
    assert!(
        outcomes.contains(&"refused_unbuilt_cargo_bump".to_string()),
        "expected the B3 audit row under its own name, so the population stays \
         subtractable from B2's; got {outcomes:?}"
    );
}

/// **#2560, which nothing in the tree caught.** `0.10.9 → 0.11.0` is
/// `NoMajorJump`, so B2 never applied and the body carries no `API-SURFACE:`
/// line. Its break was transverse — `sha2 0.11` on `digest 0.11` against
/// `hmac 0.12` on `digest 0.10` — and visible to cargo's resolver alone.
#[tokio::test]
async fn a_pass_on_a_minor_cargo_bump_with_no_build_is_refused() {
    let (_h, output, outcomes) =
        post_review(PR_2560, "2560", pass_body_2560(), "--approve", false).await;

    assert!(
        output.contains(REFUSAL_B3),
        "a `pass` on a 0.x Cargo bump with no build must be refused — this is the \
         case B2 structurally could not see; got: {output}"
    );
    assert!(
        outcomes.contains(&"refused_unbuilt_cargo_bump".to_string()),
        "expected the B3 audit row; got {outcomes:?}"
    );
}

/// **AC6.** The refusal cannot be bought with a sentence. The body claims
/// `BUILD VERIFICATION: Build: pass` and even a `BUILD-VERIFIED: yes` line, in
/// the exact wording the prompt prescribes; no `build_mika` row backs it, so it
/// is refused. This is the difference in kind between B2 and B3, and the whole
/// reason the fact is read from the database.
#[tokio::test]
async fn a_body_cannot_claim_its_way_past_the_build_requirement() {
    let (_h, output, outcomes) = post_review(
        PR_2561,
        "2561",
        pass_body_claiming_a_build(),
        "--approve",
        false,
    )
    .await;

    assert!(
        output.contains(REFUSAL_B3),
        "a claimed build must not satisfy B3 — a guard that reads an assertion is B2 \
         under another name, and mika#2519 measured the model producing one; got: {output}"
    );
    assert!(
        outcomes.contains(&"refused_unbuilt_cargo_bump".to_string()),
        "expected the B3 audit row; got {outcomes:?}"
    );
}

// ---------------------------------------------------------------------------
// The positive control — without it, "B3 refuses" and "B3 refuses everything"
// are indistinguishable
// ---------------------------------------------------------------------------

/// **The positive control.** The same PR, the same body, with one thing added:
/// a real `build_mika` row in this session. The verdict goes through.
#[tokio::test]
async fn the_same_pass_goes_through_once_a_build_is_recorded() {
    let (_h, output, outcomes) = post_review(
        PR_2561,
        "2561",
        pass_body_with_api_surface(),
        "--approve",
        true,
    )
    .await;

    assert!(
        !output.contains(REFUSAL_B3),
        "a recorded build must clear B3; got: {output}"
    );
    assert!(
        output.contains("review posted"),
        "the review must go through; got: {output}"
    );
    assert!(
        outcomes.contains(&"allowed".to_string()),
        "the nominal case must be written — without it, zero refusals would not \
         distinguish a healthy rail from an inert guard (mika#2205); got {outcomes:?}"
    );
}

/// The build is scoped to the **session**, not merely to the agent.
///
/// A `build_mika` recorded under another session is another PR's build, and
/// counting it would make B3 satisfiable by background traffic. Reading this
/// against the real query is the only way to observe the scope: a Layer A test
/// takes the boolean already computed.
#[tokio::test]
async fn a_build_from_another_session_does_not_count() {
    let harness = harness_with(
        vec![
            tool_call_response(
                "run_gh",
                json!({
                    "command": ["pr", "review", "2561", "--approve", "--body",
                                pass_body_with_api_surface()],
                    "repo": "senara-solutions/mika"
                }),
            ),
            text_response("Done."),
        ],
        Box::new(DependabotGatedRunGhTool {
            pr: Ok(PR_2561.to_string()),
        }),
    )
    .await;

    // A build, but under a session of its own — a review of some other PR ten
    // minutes ago.
    harness
        .db
        .save_tool_call(
            "tc-other-session",
            "a-different-review-session",
            None,
            None,
            3,
            BUILD_MIKA_TOOL,
            "builtin",
            Some("qa-review"),
            Some(r#"{"cwd":"/some/other/worktree"}"#),
            Some("Build started"),
            true,
            false,
            42,
            None,
        )
        .await
        .unwrap();

    let trace = harness.run("Review PR 2561").await.unwrap();
    let output = trace.calls_for_tool("run_gh")[0]
        .output
        .as_deref()
        .unwrap_or("")
        .to_string();

    assert!(
        output.contains(REFUSAL_B3),
        "a build from another session must not satisfy B3 — the question is \
         whether THIS review compiled THIS PR; got: {output}"
    );
}

// ---------------------------------------------------------------------------
// AC3 and the negative controls
// ---------------------------------------------------------------------------

/// **AC3.** A dependency PR whose diff carries no Rust dependency resolution
/// keeps its `pass` with no build. `build_mika` compiles Rust and nothing else,
/// so demanding a compilation here would be a refusal with no way out.
#[tokio::test]
async fn a_dependency_pr_with_no_cargo_file_needs_no_build() {
    let (_h, output, outcomes) =
        post_review(PR_ACTIONS, "2559", pass_body_actions(), "--approve", false).await;

    assert!(
        !output.contains(REFUSAL_B3),
        "a non-Rust dependency bump must not be asked to compile; got: {output}"
    );
    assert!(
        output.contains("review posted"),
        "the review must go through; got: {output}"
    );
    assert!(
        outcomes.contains(&"allowed".to_string()),
        "expected the nominal audit row; got {outcomes:?}"
    );
}

/// **Sonde S5's negative control.** B3's first term is the automated author. An
/// occurrence on a human PR would mean it has been relaxed, and a guard that
/// refuses human reviews costs more than the defect it closes.
#[tokio::test]
async fn a_human_pr_touching_cargo_is_never_refused_by_b3() {
    let (_h, output, outcomes) = post_review(
        PR_HUMAN_CARGO,
        "2579",
        pass_body_with_api_surface(),
        "--approve",
        false,
    )
    .await;

    assert!(
        !output.contains(REFUSAL_B3),
        "a human-authored PR must never be refused by B3; got: {output}"
    );
    assert!(
        output.contains("review posted"),
        "the review must go through; got: {output}"
    );
    assert!(
        outcomes.contains(&"allowed".to_string()),
        "expected the nominal audit row; got {outcomes:?}"
    );
}

/// The two exits B3's own refusal prescribes must not be refused in turn. A
/// refusal whose prescribed remedy is itself refused walls the review in, and
/// the model has nowhere to go.
#[tokio::test]
async fn the_two_prescribed_exits_are_not_refused_in_turn() {
    for (label, body) in [
        ("hold[review]", hold_body()),
        ("block[dependency]", block_dependency_body()),
    ] {
        let (_h, output, _outcomes) = post_review(PR_2561, "2561", body, "--comment", false).await;
        assert!(
            !output.contains(REFUSAL_B3),
            "`{label}` is an exit B3's refusal prescribes; refusing it would wall \
             the review in; got: {output}"
        );
        assert!(
            output.contains("review posted"),
            "`{label}` must go through; got: {output}"
        );
    }
}

// ---------------------------------------------------------------------------
// The abstentions — sonde S4
// ---------------------------------------------------------------------------

/// **`no_files`, under its own name.** `gh` answered and the author is
/// readable, but `files` is not. B3 cannot speak, so the verdict goes through —
/// the defect can come back on this path, which is exactly why the population
/// is counted apart from `unparseable` rather than folded into it.
#[tokio::test]
async fn an_unreadable_files_field_abstains_under_its_own_name() {
    for (label, payload) in [("absent", PR_NO_FILES), ("empty", PR_EMPTY_FILES)] {
        let (harness, output, outcomes) = post_review(
            payload,
            "2561",
            pass_body_with_api_surface(),
            "--approve",
            false,
        )
        .await;

        assert!(
            !output.contains(REFUSAL_B3),
            "an unreadable `files` ({label}) must fail OPEN — refusing a verdict on a \
             term we could not read would be the inverse failure; got: {output}"
        );
        assert!(
            outcomes.contains(&"abstained".to_string()),
            "the gate must record an abstention ({label}); got {outcomes:?}"
        );

        let causes = gate_reasonings(&harness).await;
        assert!(
            causes.contains(&DependabotAbstention::NO_FILES.to_string()),
            "the abstention must carry its own cause, or the population that lets \
             the defect back in is not countable apart ({label}); got {causes:?}"
        );
        assert!(
            !causes.contains(&DependabotAbstention::UNPARSEABLE.to_string()),
            "`no_files` must not fold into `unparseable` — they name different \
             repairs, and sonde S4 counts only the first ({label}); got {causes:?}"
        );
    }
}

/// **The abstention is PARTIAL — it covers B3 and nothing else.**
///
/// This is a measured regression, not a hypothesis. The first wiring wrote
/// `abstain(); return` on an unreadable `files`, which took B1 and B2 out of
/// the decision along with B3 — two branches that read neither `files` nor the
/// database and whose judgment is intact without them. All nine tests of
/// mika#2519 went red, and `pass` on #2453 — the witness PR mika#2519 exists to
/// unblock — came back `abstained` instead of `allowed`.
///
/// So: a `block[pipeline]` on an automated author must still be refused by B1
/// even when B3 cannot see the diff. A term we could not evaluate is never a
/// satisfied term; it is not a reason to stop evaluating the others either.
#[tokio::test]
async fn an_unreadable_files_field_does_not_disarm_the_neighbouring_branches() {
    let block_pipeline = "VERDICT: block[pipeline]\nDEPTH: code-level\nREASON: Dependabot \
                          dependency bump — no plan document present."
        .to_string();

    let (harness, output, outcomes) =
        post_review(PR_NO_FILES, "2561", block_pipeline, "--comment", false).await;

    assert!(
        output.contains("dependabot_verdict_unreachable"),
        "B1 must still refuse a `block[pipeline]` on an automated author when \
         `files` is unreadable — B1 never reads `files`; got: {output}"
    );
    assert!(
        outcomes.contains(&"refused_unreachable_pipeline_block".to_string()),
        "the B1 audit row must still be written; got {outcomes:?}"
    );

    let causes = gate_reasonings(&harness).await;
    assert!(
        causes.contains(&DependabotAbstention::NO_FILES.to_string()),
        "and the B3 abstention must still be recorded alongside it — the two are \
         independent facts about the same verdict; got {causes:?}"
    );
}
