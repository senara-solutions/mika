//! Integration tests for `[context.summary].inject` load-prevention gate (mika#1016).
//!
//! Verifies that the per-agent `identity.toml` flag controls whether the
//! conversational summary is loaded and injected into the system prompt.

use mika_common::llm::mock::*;

use super::harness::EvalHarness;

/// When `[context.summary].inject = false`, the system prompt must NOT contain
/// the conversation summary — load-prevention gate ensures the DB call is skipped.
#[tokio::test]
async fn test_summary_not_injected_when_inject_false() {
    let harness = EvalHarness::builder()
        .responses(vec![text_response("I'm the architect.")])
        .build()
        .await
        .unwrap();

    // Write identity.toml with inject = false
    std::fs::write(
        harness.home_dir.path().join("identity.toml"),
        r#"
name = "Architect"

[context.summary]
inject = false
"#,
    )
    .unwrap();

    // Seed a conversation summary in the DB so it would be injected if not gated
    harness
        .db
        .replace_with_summary("This is a leaked summary about dispatch decisions.", 0)
        .await
        .unwrap();

    let trace = harness.run("Who are you?").await.unwrap();

    // The system prompt must NOT contain the summary
    assert!(!trace.captured_requests.is_empty());
    let system = trace.captured_requests[0].system.as_deref().unwrap_or("");
    assert!(
        !system.contains("Conversation Summary"),
        "System prompt should NOT contain '## Conversation Summary' when inject = false"
    );
    assert!(
        !system.contains("leaked summary about dispatch"),
        "Summary content must not appear in system prompt when inject = false"
    );
}

/// Default behavior (no `[context]` section): the summary IS injected.
#[tokio::test]
async fn test_summary_injected_by_default() {
    let harness = EvalHarness::builder()
        .responses(vec![text_response("Hello!")])
        .build()
        .await
        .unwrap();

    // No identity.toml — defaults apply (inject = true)

    // Seed a conversation summary
    harness
        .db
        .replace_with_summary("User prefers morning standups.", 0)
        .await
        .unwrap();

    let trace = harness.run("Tell me about our standups").await.unwrap();

    // The system prompt MUST contain the summary
    assert!(!trace.captured_requests.is_empty());
    let system = trace.captured_requests[0].system.as_deref().unwrap_or("");
    assert!(
        system.contains("Conversation Summary"),
        "System prompt should contain '## Conversation Summary' by default (inject = true)"
    );
    assert!(
        system.contains("morning standups"),
        "Summary content should appear in system prompt by default"
    );
}
