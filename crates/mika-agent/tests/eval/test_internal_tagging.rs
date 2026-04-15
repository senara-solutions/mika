//! Integration tests: internal message tagging for relay sessions (#557).
//!
//! Verifies that `AgentParams.internal` correctly propagates to both user
//! and assistant messages in the DB, and that `load_recent_messages_filtered`
//! respects the filter.

use mika_common::llm::mock::*;

use super::harness::EvalHarness;

#[tokio::test]
async fn test_internal_true_tags_both_messages() {
    let harness = EvalHarness::builder()
        .responses(vec![text_response("Relay acknowledged.")])
        .internal(true)
        .build()
        .await
        .unwrap();

    let _trace = harness.run("relay message from pilot").await.unwrap();

    // Load ALL messages (no filter) — both should exist
    let all_msgs = harness.db.load_recent_messages(50).await.unwrap();
    let user_msg = all_msgs.iter().find(|m| m.role == "user").unwrap();
    let asst_msg = all_msgs.iter().find(|m| m.role == "assistant").unwrap();

    assert!(user_msg.internal, "user message should be internal");
    assert!(asst_msg.internal, "assistant message should be internal");
}

#[tokio::test]
async fn test_internal_false_leaves_messages_visible() {
    let harness = EvalHarness::builder()
        .responses(vec![text_response("Hello, user!")])
        .build()
        .await
        .unwrap();

    // Default: internal = false
    let _trace = harness.run("normal message").await.unwrap();

    let all_msgs = harness.db.load_recent_messages(50).await.unwrap();
    let user_msg = all_msgs.iter().find(|m| m.role == "user").unwrap();
    let asst_msg = all_msgs.iter().find(|m| m.role == "assistant").unwrap();

    assert!(!user_msg.internal, "user message should NOT be internal");
    assert!(
        !asst_msg.internal,
        "assistant message should NOT be internal"
    );
}

#[tokio::test]
async fn test_filtered_load_excludes_internal_messages() {
    let harness = EvalHarness::builder()
        .responses(vec![text_response("Internal reply.")])
        .internal(true)
        .build()
        .await
        .unwrap();

    let _trace = harness.run("internal relay").await.unwrap();

    // Filtered load (inbox mode) should exclude internal messages
    let filtered = harness
        .db
        .load_recent_messages_filtered(50, true)
        .await
        .unwrap();

    // No user or assistant messages should be visible in inbox mode
    let visible_user = filtered.iter().any(|m| m.role == "user");
    let visible_asst = filtered.iter().any(|m| m.role == "assistant");
    assert!(
        !visible_user,
        "internal user message should be hidden in inbox mode"
    );
    assert!(
        !visible_asst,
        "internal assistant message should be hidden in inbox mode"
    );

    // Unfiltered load should still see them
    let unfiltered = harness
        .db
        .load_recent_messages_filtered(50, false)
        .await
        .unwrap();
    assert!(
        unfiltered.iter().any(|m| m.role == "user"),
        "internal user message should be visible in audit mode"
    );
    assert!(
        unfiltered.iter().any(|m| m.role == "assistant"),
        "internal assistant message should be visible in audit mode"
    );
}
