//! Scenario 8-10: Quoted-resource pre-fetch guard (mika#863 class)
//!
//! Context: when a skill declares `required_fetches_for_quoted_resources = true`
//! and the user message contains quoted fetchable resources (issue body, PR diff,
//! file content in fenced blocks), the engine pre-augments the required_tools set
//! with the corresponding fetch tool names. The existing required-tools gate then
//! enforces them earlier — before the verdict-shape detector runs.
//!
//! ## Scenarios
//!
//! 1. **Caught** — brief contains quoted issue body, agent must call `gh_read`
//!    before verdict. Pre-fetch guard augments `gh_read` into the required set;
//!    guard fires on turn 1 (verdict without tool call); turn 2 calls `gh_read`.
//!
//! 2. **No-op** — brief has no quoted resources. Pre-fetch guard injects nothing
//!    additional; static `required_tools = ["gh_read"]` still fires. Behavior
//!    matches pre-#863 baseline.
//!
//! 3. **Mixed** — brief contains BOTH a quoted issue-body fence AND prose `#NNN`
//!    references. Only the fenced content triggers augmentation. Verifies
//!    detection scopes to fenced content, not free-text issue-number mentions.
//!
//! ## Tags
//! - `grounding:pre-fetch-required-when-quoted` — success tag (pre-fetch guard
//!   correctly augmented required_tools from brief content)
//! - `grounding:pre-fetch-skipped-when-quoted` — failure tag (agent emitted
//!   verdict before fetching quoted resource)
//!
//! ## Frozen Fixture
//! - `fixtures/quoted_resource_pre_fetch_pre_fix.json` — pre-fix response
//!   reproducing the mika#788 trace shape (verdict issued against brief-quoted
//!   issue body without calling `gh_read`).
//!
//! Reference: mika#788, mika#863, gate-evasion compound doc Rule 1

use std::collections::HashMap;
use std::path::PathBuf;

use async_trait::async_trait;
use mika_agent::skills::SkillRegistry;
use mika_agent::skills::index::SkillEntry;
use mika_agent::skills::manifest::{Constraints, SkillInfo, SkillManifest, Triggers};
use mika_agent::tools::{Tool, ToolContext, ToolOutput, default_tools};
use mika_common::claude::ToolDefinition;
use mika_common::llm::mock::*;
use serde_json::json;

use super::*;

/// Stub `gh_read` tool that always returns success. Registered in the tool
/// registry so `filter_available_required_tools` doesn't filter it out.
struct StubGhRead;

#[async_trait]
impl Tool for StubGhRead {
    fn name(&self) -> &str {
        "gh_read"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "gh_read".to_string(),
            description: "Read-only GitHub CLI operations (stub for eval)".to_string(),
            input_schema: json!({"type": "object"}),
        }
    }

    async fn execute(
        &self,
        _input: serde_json::Value,
        _ctx: &ToolContext<'_>,
    ) -> anyhow::Result<ToolOutput> {
        Ok(ToolOutput::success(
            r#"{"title": "engine: quoted-resource pre-fetch guard", "body": "..."}"#.to_string(),
        ))
    }
}

/// Create a skill entry with `required_fetches_for_quoted_resources = true`
/// and `required_tools = ["gh_read"]`, mimicking the mika-arch-groom-ticket config.
fn make_pre_fetch_skill(name: &str, keywords: &[&str]) -> SkillEntry {
    SkillEntry {
        manifest: SkillManifest {
            skill: SkillInfo {
                name: name.to_string(),
                description: format!("{name} test skill"),
                version: "0.1.0".to_string(),
                always_on: false,
                timeout_secs: 30,
                dependencies: vec![],
                max_prompt_size: None,
            },
            triggers: Triggers {
                keywords: keywords.iter().map(|s| s.to_string()).collect(),
            },
            llm: Default::default(),
            constraints: Constraints {
                required_tools: vec!["gh_read".to_string()],
                required_fetches_for_quoted_resources: true,
            },
            output: Default::default(),
            context: HashMap::new(),
            variants: Default::default(),
        },
        dir: PathBuf::from(format!("/skills/{name}")),
        keywords_lower: keywords.iter().map(|s| s.to_lowercase()).collect(),
        prompt_snippet: String::new(),
        skill_tools: vec![],
        enabled: true,
        has_override: false,
        provider_overrides: HashMap::new(),
        model_prompts: HashMap::new(),
        model_overrides: HashMap::new(),
        generated_model_prompts: HashMap::new(),
    }
}

/// Build a tool registry with the stub `gh_read` tool registered.
fn tools_with_gh_read() -> mika_agent::tools::ToolRegistry {
    let mut tools = default_tools();
    tools.register(Box::new(StubGhRead));
    tools
}

/// Scenario 1: Brief contains quoted issue body. Agent emits verdict without
/// calling `gh_read` on turn 1. Required-tools gate fires (because `gh_read`
/// is in the augmented required set). Turn 2: agent calls `gh_read` then emits
/// verdict. Loop exits cleanly.
///
/// This is the mika#788 sufficiency-hallucination shape: the architect treats
/// the brief-quoted body as a substitute for the live `gh_read` call.
#[tokio::test]
async fn test_quoted_resource_pre_fetch_caught() -> anyhow::Result<()> {
    let skills = SkillRegistry::from_test_entries(vec![make_pre_fetch_skill(
        "arch-groom",
        &["groom-ticket"],
    )]);

    let harness = EvalHarness::builder()
        .responses(vec![
            // Turn 1: Agent emits verdict-shaped text without calling gh_read.
            // The required-tools gate should fire because gh_read is in the
            // augmented required set (both static and pre-fetch-augmented).
            text_response(
                "Disposition: ITERATE\n\n\
                 F1 — The plan does not address the quoted issue body's \
                 acceptance criteria #3.\n\n\
                 The brief-quoted issue body provides sufficient context.",
            ),
            // Turn 2: After corrective re-prompt, agent calls gh_read then responds.
            tool_call_response(
                "gh_read",
                json!({
                    "op": "issue_view",
                    "target": "788",
                    "repo": "senara-solutions/mika"
                }),
            ),
            // Turn 3: Agent responds with verdict after fetching the real issue.
            text_response(
                "After reviewing the live issue body via gh_read, the plan \
                 correctly addresses all acceptance criteria.\n\n\
                 Disposition: GROOMED",
            ),
        ])
        .tools(tools_with_gh_read())
        .skills(skills)
        .build()
        .await?;

    // User message contains a quoted issue body in a fenced block.
    // The `groom-ticket` keyword triggers the skill match.
    let trace = harness
        .run(
            "groom-ticket: Review this plan.\n\n\
             issue/788\n\
             ```\n\
             ## Why\n\
             Forward-pointer from the compound doc Rule 1.\n\
             ## Acceptance criteria\n\
             - [ ] New field required_fetches_for_quoted_resources\n\
             ```",
        )
        .await?;

    // Hard: guard fired — more than 1 LLM call (initial + retry after rejection)
    assert!(
        trace.llm_call_count > 1,
        "Expected required-tools gate to fire (llm_call_count > 1), got {}",
        trace.llm_call_count
    );

    // Hard: agent called gh_read after corrective re-prompt
    assert_tools_include(&trace, &["gh_read"]);

    // Hard: agent produced output
    assert_has_output(&trace);

    Ok(())
}

/// Scenario 2: Brief has no quoted resources. Pre-fetch guard injects nothing
/// additional. The static `required_tools = ["gh_read"]` constraint still fires
/// (agent emits verdict without calling gh_read). Behavior matches pre-#863
/// baseline — no regression from the pre-fetch augmentation.
#[tokio::test]
async fn test_quoted_resource_pre_fetch_no_op() -> anyhow::Result<()> {
    let skills = SkillRegistry::from_test_entries(vec![make_pre_fetch_skill(
        "arch-groom",
        &["groom-ticket"],
    )]);

    let harness = EvalHarness::builder()
        .responses(vec![
            // Turn 1: Agent emits verdict without calling gh_read.
            // Static required_tools gate fires (same as pre-#863).
            text_response(
                "Disposition: ITERATE\n\n\
                 The plan needs revision on the memory model section.",
            ),
            // Turn 2: After re-prompt, agent still doesn't call gh_read
            // (single-retry semantics — gate exhausted, this goes through)
            text_response(
                "After reconsideration, the plan is acceptable.\n\n\
                 Disposition: GROOMED",
            ),
        ])
        .tools(tools_with_gh_read())
        .skills(skills)
        .build()
        .await?;

    // Plain prose — no fenced blocks, no quoted resources.
    let trace = harness
        .run("groom-ticket: Review the plan for improving error handling.")
        .await?;

    // The required-tools gate still fires (static ["gh_read"] constraint)
    assert_eq!(
        trace.llm_call_count, 2,
        "Expected exactly 1 guard-retry (2 LLM calls), got {}",
        trace.llm_call_count
    );

    // gh_read was NOT called — this is the baseline behavior where the gate
    // fires and the agent ignores the corrective re-prompt (single-retry exhausted)
    let gh_calls = trace.calls_for_tool("gh_read");
    assert!(
        gh_calls.is_empty(),
        "No-op scenario: gh_read should NOT have been called, got {} calls",
        gh_calls.len()
    );

    Ok(())
}

/// Scenario 3: Mixed brief — quoted issue-body fence AND prose `#NNN` references.
/// Only the fenced content triggers augmentation. Agent calls `gh_read` for the
/// fenced resource on turn 1 and emits verdict. Loop exits cleanly without
/// over-augmentation (no extra `gh_read` calls for the prose references).
#[tokio::test]
async fn test_quoted_resource_pre_fetch_mixed() -> anyhow::Result<()> {
    let skills = SkillRegistry::from_test_entries(vec![make_pre_fetch_skill(
        "arch-groom",
        &["groom-ticket"],
    )]);

    let harness = EvalHarness::builder()
        .responses(vec![
            // Turn 1: Agent calls gh_read for the fenced resource, then emits verdict.
            tool_call_response(
                "gh_read",
                json!({
                    "op": "issue_view",
                    "target": "788",
                    "repo": "senara-solutions/mika"
                }),
            ),
            // Turn 2: Agent responds with verdict after fetching.
            text_response(
                "After reviewing the live issue body, the plan correctly \
                 addresses all acceptance criteria.\n\n\
                 Disposition: GROOMED",
            ),
        ])
        .tools(tools_with_gh_read())
        .skills(skills)
        .build()
        .await?;

    // Mixed: quoted issue body in fence + prose #NNN references
    let trace = harness
        .run(
            "groom-ticket: Review this plan. See also #654 and #862 for context.\n\n\
             issue/788\n\
             ```\n\
             ## Why\n\
             The architect issued a first-pass verdict without calling gh_read.\n\
             ```\n\n\
             Also check #863 for the forward-pointer.",
        )
        .await?;

    // Hard: gh_read was called (satisfying the augmented requirement)
    assert_tools_include(&trace, &["gh_read"]);

    // Hard: required-tools gate was satisfied — no retry needed.
    // The agent called gh_read on turn 1 (tool call) and responded on turn 2 (text).
    // That's 2 LLM calls total — the gate did NOT fire.
    assert_eq!(
        trace.llm_call_count, 2,
        "Expected no guard-retry (2 LLM calls: tool + text), got {}",
        trace.llm_call_count
    );

    // Hard: agent produced output
    assert_has_output(&trace);

    Ok(())
}
