//! Golden scenario: Check GitHub PR status via run_gh.
//!
//! Class: ToolSelection | Expected tokens: 2500

use super::*;

pub fn register(registry: &GoldenRegistry) {
    registry.register(
        "tool_selection_github_pr",
        GoldenScenarioMeta {
            class: ScenarioClass::ToolSelection,
            expected_tokens: 2500,
            description: "User asks about a PR status; agent calls run_gh with github_token",
        },
    );
}

#[tokio::test]
async fn test_tool_selection_github_pr() {
    let harness = EvalHarness::builder()
        .github_token("test-token")
        .responses(vec![
            // Step 1: Agent calls run_gh to check PR status
            tool_call_response(
                "run_gh",
                json!({
                    "args": ["pr", "view", "42", "--repo", "senara-solutions/mika"]
                }),
            ),
            // Step 2: Agent responds with PR status summary
            text_response("PR #42 in senara-solutions/mika is currently open. It has 2 approvals and all CI checks are passing."),
        ])
        .build()
        .await
        .unwrap();

    let trace = harness
        .run("Check the status of PR #42 in senara-solutions/mika")
        .await
        .unwrap();

    // Hard assertions
    assert_tools_include(&trace, &["run_gh"]);
    assert_has_output(&trace);
    assert_output_contains(&trace, "PR");
}
