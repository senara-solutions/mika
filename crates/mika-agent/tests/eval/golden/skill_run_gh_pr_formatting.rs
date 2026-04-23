//! Golden scenario: Create a GitHub issue via run_gh tool.
//!
//! Class: SkillSpecific | Expected tokens: 3500

use super::*;

pub fn register(registry: &GoldenRegistry) {
    registry.register(
        "skill_run_gh_pr_formatting",
        GoldenScenarioMeta {
            class: ScenarioClass::SkillSpecific,
            expected_tokens: 3500,
            description: "Agent uses run_gh to create a GitHub issue for tracking work",
        },
    );
}

#[tokio::test]
async fn test_skill_run_gh_pr_formatting() {
    let harness = EvalHarness::builder()
        .github_token("test-token")
        .responses(vec![
            // Step 1: Agent calls run_gh to create the issue
            tool_call_response(
                "run_gh",
                json!({
                    "args": ["issue", "create", "--title", "Login page redesign", "--body", "Track the login page redesign work", "--repo", "senara-solutions/mika"]
                }),
            ),
            // Step 2: Agent confirms issue creation
            text_response(
                "I've created GitHub issue #123 for tracking the login page redesign \
                 in senara-solutions/mika.",
            ),
        ])
        .build()
        .await
        .unwrap();

    let trace = harness
        .run("Create a GitHub issue for tracking the login page redesign")
        .await
        .unwrap();

    // Hard assertions: run_gh called and output confirms creation
    assert_tools_include(&trace, &["run_gh"]);
    assert_has_output(&trace);

    let output = trace.output.text.as_deref().unwrap_or("");
    assert!(
        output.contains("issue") || output.contains("created"),
        "Output should mention 'issue' or 'created' (got: {output})"
    );
}
