//! Golden scenario: QA review catches a resource leak bug in code.
//!
//! Class: SkillSpecific | Expected tokens: 3500

use super::*;

pub fn register(registry: &GoldenRegistry) {
    registry.register(
        "skill_qa_review_bug_catch",
        GoldenScenarioMeta {
            class: ScenarioClass::SkillSpecific,
            expected_tokens: 3500,
            description: "Agent identifies a resource leak bug in a code review (pure analysis, no tools)",
        },
    );
}

#[tokio::test]
async fn test_skill_qa_review_bug_catch() {
    let harness = EvalHarness::builder()
        .responses(vec![
            // Agent provides a detailed review identifying the resource leak
            text_response(
                "I found a critical resource leak in this code change. The function returns early \
                 on the error path without closing the database connection. This will cause connection \
                 pool exhaustion under load. The fix is to ensure the connection is closed in a \
                 `finally` block or use RAII/drop semantics so the resource is released regardless \
                 of the return path.",
            ),
        ])
        .build()
        .await
        .unwrap();

    let trace = harness
        .run(
            "Review this code change for potential bugs: the function returns early \
             without closing the database connection",
        )
        .await
        .unwrap();

    // Hard assertions: pure analysis — no tools needed
    assert_has_output(&trace);
    assert_no_tools(&trace);

    // Output should identify the resource leak issue
    let output = trace.output.text.as_deref().unwrap_or("");
    assert!(
        output.contains("connection")
            || output.contains("resource")
            || output.contains("leak")
            || output.contains("close"),
        "Output should mention the resource leak (got: {output})"
    );
}
