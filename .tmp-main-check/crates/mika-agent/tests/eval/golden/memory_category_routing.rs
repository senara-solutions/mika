//! Golden scenario: Person-related fact is stored with correct category routing.
//!
//! Class: Memory | Expected tokens: 2000

use super::*;

pub fn register(registry: &GoldenRegistry) {
    registry.register(
        "memory_category_routing",
        GoldenScenarioMeta {
            class: ScenarioClass::Memory,
            expected_tokens: 2000,
            description: "Agent stores a person-related fact with the People category",
        },
    );
}

#[tokio::test]
async fn test_memory_category_routing() {
    let harness = EvalHarness::builder()
        .responses(vec![
            // Step 1: Agent stores the fact with correct category
            tool_call_response(
                "store_fact",
                json!({
                    "category": "person",
                    "name": "David",
                    "relationship": "manager",
                    "notes": "Prefers async communication"
                }),
            ),
            // Step 2: Agent confirms
            text_response("I've noted that David is your manager and prefers async communication."),
        ])
        .build()
        .await
        .unwrap();

    let trace = harness
        .run("David is my manager. He prefers async communication.")
        .await
        .unwrap();

    // Hard assertions
    assert_tools_include(&trace, &["store_fact"]);
    assert_has_output(&trace);

    // Verify the store_fact call used the "person" category
    assert_tool_args_contain(&trace, "store_fact", 0, json!({"category": "person"}));
}
