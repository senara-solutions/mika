//! Live smoke test for the native Z.AI provider (mika#1657).
//!
//! Ignored by default — it makes a real, paid API call to Z.AI's direct GLM
//! endpoint and requires `MIKA_ZAI_API_KEY` in the environment. Mirrors the
//! repo's `#[ignore]`-gated real-provider eval pattern.
//!
//! Run:
//! ```sh
//! MIKA_ZAI_API_KEY=$(grep MIKA_ZAI_API_KEY ~/.mika/.env | cut -d= -f2) \
//!   cargo test -p mika-common --test zai_smoke -- --ignored zai_smoke
//! ```
//!
//! Verifies the base URL (`https://api.z.ai/api/paas/v4`), the Bearer auth
//! header, and the OpenAI-compatible chat-completions request/response shape
//! all line up end-to-end against the real service.

use mika_common::llm::{
    LlmContent, LlmMessage, LlmRequest, LlmRole, ModelSpec, ProviderKind, create_provider,
};

#[tokio::test]
#[ignore = "live network + paid API call; requires MIKA_ZAI_API_KEY"]
async fn zai_smoke() {
    let api_key = std::env::var("MIKA_ZAI_API_KEY")
        .expect("MIKA_ZAI_API_KEY must be set to run the Z.AI smoke test");

    let model = std::env::var("MIKA_ZAI_MODEL")
        .unwrap_or_else(|_| ProviderKind::ZAi.default_model().to_string());

    let spec = ModelSpec {
        provider: ProviderKind::ZAi,
        model,
        base_url: None, // exercises the default https://api.z.ai/api/paas/v4
        api_key: Some(api_key),
    };

    let provider = create_provider(&spec, 256, false).expect("provider construction failed");

    let request = LlmRequest {
        model: spec.model.clone(),
        system: Some("You are a terse assistant. Reply with a single short greeting.".to_string()),
        messages: vec![LlmMessage {
            role: LlmRole::User,
            content: LlmContent::Text("Say hello.".to_string()),
        }],
        tools: None,
        max_tokens: 256,
        thinking: None,
    };

    let response = provider
        .send_message(&request)
        .await
        .expect("Z.AI send_message failed");

    let text = response.text();
    assert!(
        !text.trim().is_empty(),
        "expected a non-empty Z.AI response, got: {text:?}"
    );
}
