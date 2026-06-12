//! Tool-execution types — summary structs, metadata serialization, helper functions.

use tracing::warn;

/// Summary of a single tool call for persistence in conversation metadata.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ToolCallSummary {
    pub step: u32,
    pub name: String,
    pub input_summary: String,
    pub output_summary: String,
    pub success: bool,
    /// True when the tool output starts with a non-zero exit code prefix
    /// (e.g. "Exit code: 1" or "Killed by signal: 9"). When set, `success`
    /// is `false`. This field provides additional detail about *why* it failed.
    #[serde(default)]
    pub non_zero_exit: bool,
}

/// Check whether tool output content starts with a non-zero exit code prefix
/// produced by the exec handler for subprocesses that exit non-zero.
pub fn has_non_zero_exit_prefix(content: &str) -> bool {
    if let Some(rest) = content.strip_prefix("Exit code: ") {
        // "Exit code: 0" is never emitted (exit 0 has no prefix), but guard anyway
        rest.starts_with(|c: char| c.is_ascii_digit()) && !rest.starts_with('0')
    } else {
        content.starts_with("Killed by signal:")
    }
}

/// Truncate a string to approximately `max_len` bytes, appending "..." if truncated.
/// Always cuts at a valid UTF-8 char boundary to avoid panics on multi-byte input.
pub fn truncate_summary(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        let cut = max_len.saturating_sub(3);
        // Walk back to a valid char boundary
        let mut boundary = cut;
        while boundary > 0 && !s.is_char_boundary(boundary) {
            boundary -= 1;
        }
        format!("{}...", &s[..boundary])
    }
}

/// Serialize tool call summaries to JSON metadata string, capped at [`crate::planning::policy::TOOL_METADATA_MAX`].
///
/// Strategy: preserve all entries by progressively truncating per-field content.
/// 1. Try full serialization with the initial field lengths.
/// 2. If over budget, re-truncate `input_summary` and `output_summary` to fit all entries.
/// 3. Only as a last resort, drop tail entries (with a warning).
pub fn tool_calls_metadata_json(summaries: &[ToolCallSummary]) -> Option<String> {
    if summaries.is_empty() {
        return None;
    }
    let wrapper = serde_json::json!({ "tool_calls": summaries });
    let json = serde_json::to_string(&wrapper).ok()?;
    if json.len() <= crate::planning::policy::TOOL_METADATA_MAX {
        return Some(json);
    }

    // Phase 1: Aggressively re-truncate fields to fit all entries.
    let shrunk: Vec<ToolCallSummary> = summaries
        .iter()
        .map(|s| ToolCallSummary {
            step: s.step,
            name: s.name.clone(),
            input_summary: truncate_summary(&s.input_summary, 30),
            output_summary: truncate_summary(&s.output_summary, 50),
            success: s.success,
            non_zero_exit: s.non_zero_exit,
        })
        .collect();
    let wrapper = serde_json::json!({ "tool_calls": shrunk });
    if let Ok(json) = serde_json::to_string(&wrapper)
        && json.len() <= crate::planning::policy::TOOL_METADATA_MAX
    {
        return Some(json);
    }

    // Phase 2: Last resort — drop tail entries from the already-shrunk vector.
    warn!(
        total_entries = summaries.len(),
        max = crate::planning::policy::TOOL_METADATA_MAX,
        "tool_calls metadata exceeds cap after field truncation, dropping tail entries"
    );
    for count in (1..shrunk.len()).rev() {
        let wrapper = serde_json::json!({ "tool_calls": &shrunk[..count] });
        if let Ok(json) = serde_json::to_string(&wrapper)
            && json.len() <= crate::planning::policy::TOOL_METADATA_MAX
        {
            return Some(json);
        }
    }
    warn!(
        total_entries = summaries.len(),
        "tool_calls metadata: unable to fit even a single entry, returning None"
    );
    None
}

/// Format tool call metadata into a concise summary block for injection into history.
///
/// Includes truncated input so the agent can introspect what arguments it passed
/// (e.g., "what command did you send?") and output for result context.
/// Malformed entries are skipped rather than causing the entire block to be dropped.
pub fn format_tool_summary_block(metadata_json: &str) -> Option<String> {
    let parsed: serde_json::Value = serde_json::from_str(metadata_json).ok()?;
    let calls = parsed.get("tool_calls")?.as_array()?;
    if calls.is_empty() {
        return None;
    }
    let parts: Vec<String> = calls
        .iter()
        .filter_map(|call| {
            let name = call.get("name")?.as_str()?;
            let input = call
                .get("input_summary")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let output = call
                .get("output_summary")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let success = call
                .get("success")
                .and_then(|v| v.as_bool())
                .unwrap_or(true);
            let non_zero_exit = call
                .get("non_zero_exit")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let status = if non_zero_exit {
                " [NON-ZERO]"
            } else if !success {
                " [FAILED]"
            } else {
                ""
            };
            let short_input = truncate_summary(input, 60);
            let short_output = truncate_summary(output, 80);
            if short_input.is_empty() {
                Some(format!("{name}{status} → {short_output}"))
            } else {
                Some(format!("{name}({short_input}){status} → {short_output}"))
            }
        })
        .collect();
    if parts.is_empty() {
        return None;
    }
    Some(format!(
        "\n<context type=\"tool_history\" trust=\"metadata\">\n{}\n</context>",
        parts.join("\n")
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    // -- truncate_summary tests --

    #[test]
    fn test_truncate_summary_no_op_for_short_strings() {
        assert_eq!(truncate_summary("hello", 10), "hello");
    }

    #[test]
    fn test_truncate_summary_truncates_long_strings() {
        let long = "a".repeat(300);
        let result = truncate_summary(&long, 200);
        assert!(result.len() <= 200);
        assert!(result.ends_with("..."));
    }

    #[test]
    fn test_truncate_summary_exact_length_not_truncated() {
        let exact = "a".repeat(200);
        assert_eq!(truncate_summary(&exact, 200), exact);
    }

    #[test]
    fn test_truncate_summary_safe_with_multibyte_chars() {
        // Euro sign is 3 bytes: \xe2\x82\xac
        let s = "\u{20AC}".repeat(100); // 300 bytes, 100 chars
        let result = truncate_summary(&s, 10);
        assert!(result.ends_with("..."));
        // Must not panic and must be valid UTF-8
        assert!(std::str::from_utf8(result.as_bytes()).is_ok());
    }

    #[test]
    fn test_truncate_summary_safe_with_emoji() {
        // Emoji is 4 bytes
        let s = "Hello \u{1F600} world! More text here to exceed the limit easily.";
        let result = truncate_summary(s, 10);
        assert!(result.ends_with("..."));
        assert!(result.len() <= 13); // 10 bytes + "..."
    }

    // -- tool_calls_metadata_json tests --

    #[test]
    fn test_tool_calls_metadata_json_empty_returns_none() {
        assert!(tool_calls_metadata_json(&[]).is_none());
    }

    #[test]
    fn test_tool_calls_metadata_json_single_call() {
        let summaries = vec![ToolCallSummary {
            step: 0,
            name: "search_memory".to_string(),
            input_summary: r#"{"query":"meetings"}"#.to_string(),
            output_summary: "Found 3 results".to_string(),
            success: true,
            non_zero_exit: false,
        }];
        let json = tool_calls_metadata_json(&summaries).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        let calls = parsed["tool_calls"].as_array().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0]["name"], "search_memory");
        assert_eq!(calls[0]["success"], true);
    }

    #[test]
    fn test_tool_calls_metadata_json_respects_max_size() {
        // Create many tool calls with large outputs to exceed TOOL_METADATA_MAX
        let summaries: Vec<ToolCallSummary> = (0..50)
            .map(|i| ToolCallSummary {
                step: i,
                name: format!("tool_{i}"),
                input_summary: "x".repeat(200),
                output_summary: "y".repeat(300),
                success: true,
                non_zero_exit: false,
            })
            .collect();
        let json = tool_calls_metadata_json(&summaries).unwrap();
        // Must produce valid JSON within the size cap
        assert!(
            json.len() <= crate::planning::policy::TOOL_METADATA_MAX,
            "metadata exceeded TOOL_METADATA_MAX: {} chars",
            json.len()
        );
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert!(!parsed["tool_calls"].as_array().unwrap().is_empty());
    }

    #[test]
    fn test_tool_call_summary_truncates_large_inputs() {
        // Simulate what happens when building a ToolCallSummary with large content
        let large_input = "x".repeat(10_000);
        let large_output = "y".repeat(10_000);
        let input_summary =
            truncate_summary(&large_input, crate::planning::policy::INPUT_SUMMARY_MAX);
        let output_summary =
            truncate_summary(&large_output, crate::planning::policy::OUTPUT_SUMMARY_MAX);

        assert!(
            input_summary.len() <= crate::planning::policy::INPUT_SUMMARY_MAX,
            "input_summary too long: {} chars",
            input_summary.len()
        );
        assert!(
            output_summary.len() <= crate::planning::policy::OUTPUT_SUMMARY_MAX,
            "output_summary too long: {} chars",
            output_summary.len()
        );
        assert!(input_summary.ends_with("..."));
        assert!(output_summary.ends_with("..."));
    }

    #[test]
    fn test_all_entries_preserved_at_max_steps() {
        // With reduced per-field limits, 10 entries with typical tool names should
        // all fit within TOOL_METADATA_MAX without tail-drop
        let summaries: Vec<ToolCallSummary> = (0..10)
            .map(|i| ToolCallSummary {
                step: i,
                name: "search_memory".to_string(),
                input_summary: truncate_summary(
                    &"x".repeat(10_000),
                    crate::planning::policy::INPUT_SUMMARY_MAX,
                ),
                output_summary: truncate_summary(
                    &"y".repeat(10_000),
                    crate::planning::policy::OUTPUT_SUMMARY_MAX,
                ),
                success: true,
                non_zero_exit: false,
            })
            .collect();
        let json = tool_calls_metadata_json(&summaries).unwrap();
        assert!(
            json.len() <= crate::planning::policy::TOOL_METADATA_MAX,
            "truncated summaries exceed cap: {} chars",
            json.len()
        );
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(
            parsed["tool_calls"].as_array().unwrap().len(),
            10,
            "all 10 entries must be preserved"
        );
    }

    #[test]
    fn test_safety_net_drops_tail_on_overflow() {
        // With pathologically long tool names or extreme content, the safety net
        // tail-drop should still produce valid JSON within the cap
        let summaries: Vec<ToolCallSummary> = (0..20)
            .map(|i| ToolCallSummary {
                step: i,
                name: format!("mcp__very_long_server_name__tool_with_long_name_{i}"),
                input_summary: "x".repeat(crate::planning::policy::INPUT_SUMMARY_MAX),
                output_summary: "y".repeat(crate::planning::policy::OUTPUT_SUMMARY_MAX),
                success: true,
                non_zero_exit: false,
            })
            .collect();
        let json = tool_calls_metadata_json(&summaries).unwrap();
        assert!(
            json.len() <= crate::planning::policy::TOOL_METADATA_MAX,
            "safety net failed: {} chars",
            json.len()
        );
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        let entries = parsed["tool_calls"].as_array().unwrap();
        assert!(!entries.is_empty(), "must retain at least one entry");
        assert!(entries.len() < 20, "some entries should have been dropped");
    }

    /// Regression test for #744: milestone-workflow turns with 21+ tool calls have
    /// their tail entries dropped by the 4KB metadata cap. This is acceptable because
    /// the dashboard now fetches from the `tool_calls` table (via `useTraceToolCalls`)
    /// instead of parsing this metadata. The metadata path is only used for the LLM
    /// history builder's `format_tool_summary_block()`.
    #[test]
    fn test_metadata_cap_drops_tail_on_milestone_workflow_turns() {
        // Simulate a milestone-workflow turn: 14 bookkeeping calls + 7 status updates + 1 dispatch
        let tool_names = [
            "run_gh",
            "create_task",
            "run_gh",
            "resolve_issue_order",
            "run_gh",
            "list_tasks",
            "list_tasks",
            "create_task",
            "create_task",
            "create_task",
            "create_task",
            "create_task",
            "create_task",
            "update_task_status",
            "update_task_status",
            "update_task_status",
            "update_task_status",
            "update_task_status",
            "update_task_status",
            "update_task_status",
            "run_claude_pilot",
        ];
        let summaries: Vec<ToolCallSummary> = tool_names
            .iter()
            .enumerate()
            .map(|(i, name)| ToolCallSummary {
                step: i as u32 / 3, // group into steps like the real agent loop
                name: name.to_string(),
                input_summary: truncate_summary(
                    &"x".repeat(10_000),
                    crate::planning::policy::INPUT_SUMMARY_MAX,
                ),
                output_summary: truncate_summary(
                    &"y".repeat(10_000),
                    crate::planning::policy::OUTPUT_SUMMARY_MAX,
                ),
                success: true,
                non_zero_exit: false,
            })
            .collect();

        assert_eq!(
            summaries.len(),
            21,
            "milestone-workflow turn should have 21 calls"
        );

        let json = tool_calls_metadata_json(&summaries).unwrap();
        assert!(
            json.len() <= crate::planning::policy::TOOL_METADATA_MAX,
            "metadata must respect cap: {} chars",
            json.len()
        );

        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        let entries = parsed["tool_calls"].as_array().unwrap();

        // With max-length fields, the 4KB cap forces tail-drop — fewer entries than input.
        // This is the documented limitation (#744). The dashboard uses the tool_calls table
        // instead of this metadata, so tail-drop is acceptable for the LLM history context.
        assert!(!entries.is_empty(), "must retain at least one entry");
        assert!(
            entries.len() < summaries.len(),
            "4KB cap should force tail-drop: got {} entries from {} inputs",
            entries.len(),
            summaries.len()
        );

        // Verify structural integrity of kept entries
        for entry in entries {
            assert!(entry["name"].is_string(), "entries must have name");
            assert!(entry["step"].is_number(), "entries must have step");
        }
    }

    // -- format_tool_summary_block tests --

    #[test]
    fn test_format_tool_summary_block_valid_json() {
        let json = r#"{"tool_calls":[{"step":0,"name":"tmux_send_command","input_summary":"{\"session\":\"mika\",\"text\":\"cargo test\"}","output_summary":"Command sent","success":true}]}"#;
        let block = format_tool_summary_block(json).unwrap();
        assert!(block.contains("tmux_send_command"));
        assert!(block.contains("cargo test")); // input is now surfaced
        assert!(block.contains("Command sent"));
        assert!(block.starts_with("\n<context type=\"tool_history\""));
        assert!(block.contains("</context>"));
    }

    #[test]
    fn test_format_tool_summary_block_failed_tool() {
        let json = r#"{"tool_calls":[{"step":0,"name":"bad_tool","input_summary":"","output_summary":"error","success":false}]}"#;
        let block = format_tool_summary_block(json).unwrap();
        assert!(block.contains("[FAILED]"));
    }

    #[test]
    fn test_format_tool_summary_block_skips_malformed_entries() {
        // One good entry, one missing name — should produce partial result
        let json = r#"{"tool_calls":[{"step":0,"name":"good_tool","input_summary":"","output_summary":"ok","success":true},{"step":1,"output_summary":"no name"}]}"#;
        let block = format_tool_summary_block(json).unwrap();
        assert!(block.contains("good_tool"));
        // The malformed entry should be skipped, not cause None
    }

    #[test]
    fn test_format_tool_summary_block_empty_calls_returns_none() {
        let json = r#"{"tool_calls":[]}"#;
        assert!(format_tool_summary_block(json).is_none());
    }

    #[test]
    fn test_format_tool_summary_block_invalid_json_returns_none() {
        assert!(format_tool_summary_block("not json").is_none());
    }

    #[test]
    fn test_format_tool_summary_block_non_zero_exit_old_format() {
        // Backward compat: old metadata had success: true with non_zero_exit: true
        let json = r#"{"tool_calls":[{"step":0,"name":"shell_exec","input_summary":"grep foo","output_summary":"Exit code: 1\nno matches","success":true,"non_zero_exit":true}]}"#;
        let block = format_tool_summary_block(json).unwrap();
        assert!(
            block.contains("[NON-ZERO]"),
            "expected [NON-ZERO] tag in: {block}"
        );
        assert!(!block.contains("[FAILED]"));
    }

    #[test]
    fn test_format_tool_summary_block_non_zero_exit_new_format() {
        // New format: success is false when non_zero_exit is true
        let json = r#"{"tool_calls":[{"step":0,"name":"shell_exec","input_summary":"grep foo","output_summary":"Exit code: 1\nno matches","success":false,"non_zero_exit":true}]}"#;
        let block = format_tool_summary_block(json).unwrap();
        assert!(
            block.contains("[NON-ZERO]"),
            "expected [NON-ZERO] tag in: {block}"
        );
        assert!(!block.contains("[FAILED]"));
    }

    #[test]
    fn test_format_tool_summary_block_non_zero_exit_missing_defaults_false() {
        // Backward compat: old metadata without non_zero_exit field
        let json = r#"{"tool_calls":[{"step":0,"name":"shell_exec","input_summary":"ls","output_summary":"files","success":true}]}"#;
        let block = format_tool_summary_block(json).unwrap();
        assert!(!block.contains("[NON-ZERO]"));
        assert!(!block.contains("[FAILED]"));
    }

    // -- has_non_zero_exit_prefix tests --

    #[test]
    fn test_has_non_zero_exit_prefix() {
        assert!(has_non_zero_exit_prefix("Exit code: 1\nsome output"));
        assert!(has_non_zero_exit_prefix("Exit code: 127\n"));
        assert!(has_non_zero_exit_prefix("Killed by signal: 9\n"));
        assert!(!has_non_zero_exit_prefix("Exit code: unknown\nstuff"));
        assert!(!has_non_zero_exit_prefix("All good, no errors"));
        assert!(!has_non_zero_exit_prefix(""));
    }
}
