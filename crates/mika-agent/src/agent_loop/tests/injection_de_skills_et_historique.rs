//! Le contrat de prompt : injection de skills et préparation de l'historique.
//!
//! Les variantes de prompt d'`inject_skills`, `format_step_exceeded_fallback`,
//! les métadonnées de message lues en base et `strip_prior_images`.
//!
//! Visibilité : enfant de `agent_loop::tests`, donc descendant
//! d'`agent_loop` — les items privés d'`agent_loop` et les helpers promus
//! dans `tests/mod.rs` restent visibles via `use super::*`.

use super::*;

#[test]
fn test_inject_skills_skips_empty_snippets() {
    let tools = ToolRegistry::new();
    let entry = make_skill_entry("quiet", 30, &["quiet_tool"]);
    // prompt_snippet is empty by default from make_skill_entry
    let matched: Vec<&SkillEntry> = vec![&entry];
    let mut system = "Base.".to_string();

    let no_ctx = HashMap::new();
    let (_defs, variant, _) = inject_skills_and_resolve_tools(
        &matched,
        &tools,
        &mut system,
        "anthropic",
        "claude-sonnet-4-6",
        &no_ctx,
        "mika-test",
        &[],
        None,
        false,
    );

    // Should NOT add skill context section when snippet is empty
    assert!(!system.contains("quiet Skill"));
    assert_eq!(system, "Base.");
    // No variant info when prompt is empty
    assert!(variant.is_none());
}

#[test]
fn test_inject_skills_falls_back_to_root() {
    let tools = ToolRegistry::new();
    let mut entry = make_skill_entry("search", 30, &[]);
    entry.prompt_snippet = "Root prompt for search.".to_string();
    let matched: Vec<&SkillEntry> = vec![&entry];
    let mut system = String::new();

    // No model variant — should fall back to root
    let no_ctx = HashMap::new();
    let (_defs, _variant, _) = inject_skills_and_resolve_tools(
        &matched,
        &tools,
        &mut system,
        "groq",
        "llama-3.3-70b-versatile",
        &no_ctx,
        "mika-test",
        &[],
        None,
        false,
    );

    assert!(system.contains("Root prompt for search."));
}

#[test]
fn test_inject_skills_no_prompt_at_all() {
    let tools = ToolRegistry::new();
    let entry = make_skill_entry("quiet", 30, &["quiet_tool"]);
    // No root prompt, no provider prompts
    let matched: Vec<&SkillEntry> = vec![&entry];
    let mut system = "Base.".to_string();

    let no_ctx = HashMap::new();
    let (_defs, variant, _) = inject_skills_and_resolve_tools(
        &matched,
        &tools,
        &mut system,
        "anthropic",
        "claude-sonnet-4-6",
        &no_ctx,
        "mika-test",
        &[],
        None,
        false,
    );

    // Should NOT add any skill context section
    assert_eq!(system, "Base.");
    assert!(variant.is_none());
}

#[test]
fn test_inject_skills_uses_model_prompt() {
    let tools = ToolRegistry::new();
    let mut entry = make_skill_entry("search", 30, &[]);
    entry.prompt_snippet = "Root prompt.".to_string();
    entry.model_prompts_mut().insert(
        "anthropic/claude-sonnet-4-6".to_string(),
        "Sonnet-specific prompt.".to_string(),
    );
    let matched: Vec<&SkillEntry> = vec![&entry];
    let mut system = String::new();

    let no_ctx = HashMap::new();
    let (_defs, variant, _) = inject_skills_and_resolve_tools(
        &matched,
        &tools,
        &mut system,
        "anthropic",
        "claude-sonnet-4-6",
        &no_ctx,
        "mika-test",
        &[],
        None,
        false,
    );

    assert!(system.contains("Sonnet-specific prompt."));
    assert!(!system.contains("Root prompt."));
    // Should record hand-authored model variant
    let variant_json: HashMap<String, String> =
        serde_json::from_str(variant.as_ref().unwrap()).unwrap();
    assert_eq!(
        variant_json.get("search").unwrap(),
        "hand_authored_model:anthropic/claude-sonnet-4-6"
    );
}

#[test]
fn test_inject_skills_model_no_match_falls_back_to_root() {
    let tools = ToolRegistry::new();
    let mut entry = make_skill_entry("search", 30, &[]);
    entry.prompt_snippet = "Root prompt.".to_string();
    entry.model_prompts_mut().insert(
        "anthropic/claude-sonnet-4-6".to_string(),
        "Sonnet prompt.".to_string(),
    );
    // No model variant for claude-opus-4 — should fall back to root
    let matched: Vec<&SkillEntry> = vec![&entry];
    let mut system = String::new();

    let no_ctx = HashMap::new();
    let (_defs, variant, _) = inject_skills_and_resolve_tools(
        &matched,
        &tools,
        &mut system,
        "anthropic",
        "claude-opus-4",
        &no_ctx,
        "mika-test",
        &[],
        None,
        false,
    );

    assert!(system.contains("Root prompt."));
    assert!(!system.contains("Sonnet prompt."));
    let variant_json: HashMap<String, String> =
        serde_json::from_str(variant.as_ref().unwrap()).unwrap();
    assert_eq!(variant_json.get("search").unwrap(), "base");
}

#[test]
fn test_inject_skills_model_falls_back_to_root() {
    let tools = ToolRegistry::new();
    let mut entry = make_skill_entry("search", 30, &[]);
    entry.prompt_snippet = "Root prompt.".to_string();
    // No provider or model variant for groq/llama
    let matched: Vec<&SkillEntry> = vec![&entry];
    let mut system = String::new();

    let no_ctx = HashMap::new();
    let (_defs, _variant, _) = inject_skills_and_resolve_tools(
        &matched,
        &tools,
        &mut system,
        "groq",
        "llama-3.3-70b-versatile",
        &no_ctx,
        "mika-test",
        &[],
        None,
        false,
    );

    assert!(system.contains("Root prompt."));
}

#[test]
fn test_inject_skills_model_with_slash() {
    let tools = ToolRegistry::new();
    let mut entry = make_skill_entry("search", 30, &[]);
    entry.prompt_snippet = "Root prompt.".to_string();
    entry.model_prompts_mut().insert(
        "openrouter/anthropic--claude-sonnet-4".to_string(),
        "OpenRouter model prompt.".to_string(),
    );
    let matched: Vec<&SkillEntry> = vec![&entry];
    let mut system = String::new();

    // Model name contains a slash — sanitize_model_dir_name should match
    let no_ctx = HashMap::new();
    let (_defs, variant, _) = inject_skills_and_resolve_tools(
        &matched,
        &tools,
        &mut system,
        "openrouter",
        "anthropic/claude-sonnet-4",
        &no_ctx,
        "mika-test",
        &[],
        None,
        false,
    );

    assert!(system.contains("OpenRouter model prompt."));
    assert!(!system.contains("Root prompt."));
    let variant_json: HashMap<String, String> =
        serde_json::from_str(variant.as_ref().unwrap()).unwrap();
    assert_eq!(
        variant_json.get("search").unwrap(),
        "hand_authored_model:openrouter/anthropic--claude-sonnet-4"
    );
}

#[test]
fn test_max_skill_timeout_uses_model_override() {
    let mut s1 = make_skill_entry("search", 30, &[]);
    s1.provider_overrides.insert(
        "anthropic".to_string(),
        crate::skills::manifest::ProviderSkillFields {
            timeout_secs: Some(90),
            max_prompt_size: None,
        },
    );
    s1.model_overrides.insert(
        "anthropic/claude-sonnet-4-6".to_string(),
        crate::skills::manifest::ProviderSkillFields {
            timeout_secs: Some(120),
            max_prompt_size: None,
        },
    );
    let matched: Vec<&SkillEntry> = vec![&s1];
    // With model override, should use model
    assert_eq!(
        max_skill_timeout(&matched, "anthropic", "claude-sonnet-4-6"),
        120
    );
    // Without model override, should use provider
    assert_eq!(
        max_skill_timeout(&matched, "anthropic", "claude-opus-4"),
        90
    );
    // Without provider or model, should use root
    assert_eq!(max_skill_timeout(&matched, "groq", "llama"), 30);
}

#[test]
fn test_format_step_exceeded_fallback_non_zero_exit_old_format() {
    // Backward compat: old metadata had success: true with non_zero_exit: true
    let summaries = vec![ToolCallSummary {
        step: 0,
        name: "shell_exec".to_string(),
        input_summary: "grep foo".to_string(),
        output_summary: "Exit code: 1".to_string(),
        success: true,
        non_zero_exit: true,
    }];
    let result = format_step_exceeded_fallback(&summaries);
    assert!(
        result.contains("- shell_exec (non-zero exit)"),
        "expected non-zero exit status in: {result}"
    );
}

#[test]
fn test_format_step_exceeded_fallback_non_zero_exit_new_format() {
    // New format: success is false when non_zero_exit is true
    let summaries = vec![ToolCallSummary {
        step: 0,
        name: "shell_exec".to_string(),
        input_summary: "grep foo".to_string(),
        output_summary: "Exit code: 1".to_string(),
        success: false,
        non_zero_exit: true,
    }];
    let result = format_step_exceeded_fallback(&summaries);
    assert!(
        result.contains("- shell_exec (non-zero exit)"),
        "expected non-zero exit status in: {result}"
    );
}

// -- DB metadata integration tests --

#[tokio::test]
async fn test_save_and_load_message_with_metadata() {
    let db = test_async_db();
    let metadata = r#"{"tool_calls":[{"step":0,"name":"search_memory","input_summary":"q","output_summary":"found","success":true}]}"#;
    db.save_message_with_metadata(
        "test-session",
        "assistant",
        "I searched your memory.",
        Some(metadata),
        None,
        false,
    )
    .await
    .unwrap();

    let messages = db.load_recent_messages(10).await.unwrap();
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].role, "assistant");
    assert_eq!(messages[0].content, "I searched your memory.");
    assert_eq!(messages[0].metadata.as_deref(), Some(metadata));
}

#[tokio::test]
async fn test_save_message_without_metadata_loads_as_none() {
    let db = test_async_db();
    db.save_message("test-session", "user", "Hello", None)
        .await
        .unwrap();

    let messages = db.load_recent_messages(10).await.unwrap();
    assert_eq!(messages.len(), 1);
    assert!(messages[0].metadata.is_none());
}

#[tokio::test]
async fn test_save_message_with_null_metadata() {
    let db = test_async_db();
    db.save_message_with_metadata(
        "test-session",
        "assistant",
        "No tools used.",
        None,
        None,
        false,
    )
    .await
    .unwrap();

    let messages = db.load_recent_messages(10).await.unwrap();
    assert_eq!(messages.len(), 1);
    assert!(messages[0].metadata.is_none());
}

// -- strip_prior_images tests --

#[test]
fn test_strip_prior_images_removes_image_blocks() {
    let mut messages = vec![
        // Prior turn: user message with tool results containing images
        LlmMessage {
            role: LlmRole::User,
            content: LlmContent::Blocks(vec![LlmContentBlock::ToolResult {
                tool_call_id: "tu_1".to_string(),
                content: LlmToolResultContent::Blocks(vec![
                    LlmToolResultBlock::Text("Screenshot taken.".to_string()),
                    LlmToolResultBlock::Image(LlmImage {
                        media_type: "image/png".to_string(),
                        data: "iVBORw0KGgo=".to_string(),
                    }),
                ]),
                is_error: false,
            }]),
        },
        // Prior turn: assistant response
        LlmMessage {
            role: LlmRole::Assistant,
            content: LlmContent::Text("I see a desktop.".to_string()),
        },
        // Current turn: new tool results (should be preserved)
        LlmMessage {
            role: LlmRole::User,
            content: LlmContent::Blocks(vec![LlmContentBlock::ToolResult {
                tool_call_id: "tu_2".to_string(),
                content: LlmToolResultContent::Blocks(vec![
                    LlmToolResultBlock::Text("New screenshot.".to_string()),
                    LlmToolResultBlock::Image(LlmImage {
                        media_type: "image/png".to_string(),
                        data: "iVBORw0KGgo=".to_string(),
                    }),
                ]),
                is_error: false,
            }]),
        },
    ];

    strip_prior_images(&mut messages);

    // First message should have images stripped
    if let LlmContent::Blocks(blocks) = &messages[0].content {
        if let LlmContentBlock::ToolResult { content, .. } = &blocks[0] {
            assert!(
                matches!(content, LlmToolResultContent::Text(t) if t.contains("Screenshot taken.") && t.contains("omitted"))
            );
        } else {
            panic!("expected ToolResult");
        }
    } else {
        panic!("expected Blocks");
    }

    // Last message should still have images
    if let LlmContent::Blocks(blocks) = &messages[2].content {
        if let LlmContentBlock::ToolResult { content, .. } = &blocks[0] {
            assert!(matches!(content, LlmToolResultContent::Blocks(_)));
        } else {
            panic!("expected ToolResult");
        }
    } else {
        panic!("expected Blocks");
    }
}

#[test]
fn test_strip_prior_images_preserves_text_only() {
    let mut messages = vec![
        LlmMessage {
            role: LlmRole::User,
            content: LlmContent::Blocks(vec![LlmContentBlock::ToolResult {
                tool_call_id: "tu_1".to_string(),
                content: LlmToolResultContent::Text("just text".to_string()),
                is_error: false,
            }]),
        },
        LlmMessage {
            role: LlmRole::User,
            content: LlmContent::Text("current turn".to_string()),
        },
    ];

    strip_prior_images(&mut messages);

    // Text-only tool result should be unchanged
    if let LlmContent::Blocks(blocks) = &messages[0].content
        && let LlmContentBlock::ToolResult { content, .. } = &blocks[0]
    {
        assert!(matches!(content, LlmToolResultContent::Text(t) if t == "just text"));
    }
}

#[test]
fn test_strip_prior_images_removes_user_attached_images() {
    let mut messages = vec![
        // Prior turn: user message with text and an attached image
        LlmMessage {
            role: LlmRole::User,
            content: LlmContent::Blocks(vec![
                LlmContentBlock::Text("What is in this picture?".to_string()),
                LlmContentBlock::Image(LlmImage {
                    media_type: "image/png".to_string(),
                    data: "iVBORw0KGgo=".to_string(),
                }),
            ]),
        },
        // Assistant response
        LlmMessage {
            role: LlmRole::Assistant,
            content: LlmContent::Text("I see a cat.".to_string()),
        },
        // Current turn: user message with a new image (should be preserved)
        LlmMessage {
            role: LlmRole::User,
            content: LlmContent::Blocks(vec![
                LlmContentBlock::Text("And this one?".to_string()),
                LlmContentBlock::Image(LlmImage {
                    media_type: "image/jpeg".to_string(),
                    data: "/9j/4AAQ=".to_string(),
                }),
            ]),
        },
    ];

    strip_prior_images(&mut messages);

    // First message: image should be replaced with placeholder text
    if let LlmContent::Blocks(blocks) = &messages[0].content {
        assert_eq!(blocks.len(), 2);
        assert!(
            matches!(&blocks[0], LlmContentBlock::Text(text) if text == "What is in this picture?")
        );
        assert!(
            matches!(&blocks[1], LlmContentBlock::Text(text) if text == "[user image from previous turn omitted]")
        );
    } else {
        panic!("expected Blocks for first message");
    }

    // Last message: image should still be intact
    if let LlmContent::Blocks(blocks) = &messages[2].content {
        assert_eq!(blocks.len(), 2);
        assert!(matches!(&blocks[0], LlmContentBlock::Text(_)));
        assert!(matches!(&blocks[1], LlmContentBlock::Image(_)));
    } else {
        panic!("expected Blocks for last message");
    }
}

#[test]
fn test_strip_prior_images_no_mutation_without_images() {
    let mut messages = vec![
        // Prior turn: user message with only text blocks
        LlmMessage {
            role: LlmRole::User,
            content: LlmContent::Blocks(vec![LlmContentBlock::Text("Hello".to_string())]),
        },
        // Current turn
        LlmMessage {
            role: LlmRole::User,
            content: LlmContent::Text("Current".to_string()),
        },
    ];

    strip_prior_images(&mut messages);

    // Text-only blocks should remain unchanged
    if let LlmContent::Blocks(blocks) = &messages[0].content {
        assert_eq!(blocks.len(), 1);
        assert!(matches!(&blocks[0], LlmContentBlock::Text(text) if text == "Hello"));
    } else {
        panic!("expected Blocks");
    }
}

// -- format_step_exceeded_fallback tests --

#[test]
fn test_format_step_exceeded_fallback_empty_summaries() {
    let result = format_step_exceeded_fallback(&[]);
    assert!(result.contains("I ran out of steps"));
    assert!(result.contains("You can ask me to continue"));
    // No bullet points when empty
    assert!(!result.contains("- "));
}

#[test]
fn test_format_step_exceeded_fallback_few_summaries() {
    let summaries = vec![
        ToolCallSummary {
            step: 0,
            name: "search_memory".to_string(),
            input_summary: "query".to_string(),
            output_summary: "found 3 results".to_string(),
            success: true,
            non_zero_exit: false,
        },
        ToolCallSummary {
            step: 1,
            name: "run_shell".to_string(),
            input_summary: "ls".to_string(),
            output_summary: "error".to_string(),
            success: false,
            non_zero_exit: false,
        },
        ToolCallSummary {
            step: 2,
            name: "read_file".to_string(),
            input_summary: "/tmp/test".to_string(),
            output_summary: "contents".to_string(),
            success: true,
            non_zero_exit: false,
        },
    ];
    let result = format_step_exceeded_fallback(&summaries);
    assert!(result.contains("- search_memory (done)"));
    assert!(result.contains("- run_shell (failed)"));
    assert!(result.contains("- read_file (done)"));
    assert!(result.contains("You can ask me to continue"));
}

#[test]
fn test_format_step_exceeded_fallback_truncates_to_last_five() {
    let summaries: Vec<ToolCallSummary> = (0..10)
        .map(|i| ToolCallSummary {
            step: i,
            name: format!("tool_{i}"),
            input_summary: String::new(),
            output_summary: String::new(),
            success: true,
            non_zero_exit: false,
        })
        .collect();
    let result = format_step_exceeded_fallback(&summaries);
    // Should only show last 5 (tool_5 through tool_9)
    assert!(!result.contains("tool_0"));
    assert!(!result.contains("tool_4"));
    assert!(result.contains("tool_5"));
    assert!(result.contains("tool_9"));
}
