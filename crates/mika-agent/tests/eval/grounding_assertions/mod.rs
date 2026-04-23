//! # Grounding Assertion Helpers (#741)
//!
//! Crate-shared assertion helpers for fabrication-detection scenarios.
//! Each helper targets a specific grounding failure shape:
//!
//! - **Forbidden-word assertion:** response text MUST NOT contain specific words
//! - **Required-tool assertion:** specific tool(s) MUST be called before EndTurn
//! - **Ordered-content assertion:** response MUST contain items in a specific order
//!
//! These are hard assertions (test-gating), not soft tags.
//!
//! ## Design
//!
//! Per plan D1: two hard-assertion shapes, composable per scenario.
//! No LLM-judge gating — each fabrication class has objectively checkable signals.
//!
//! Reference: mika#741 D1

use super::trace::AgentTrace;

/// Assert that the agent's response text does NOT contain any of the forbidden words.
///
/// Normalization: case-insensitive, strips surrounding punctuation from each word
/// in the response before matching. The forbidden list is scenario-specific.
///
/// # Panics
/// Panics with a descriptive message listing each forbidden word found and its
/// location in the response.
pub fn assert_response_forbids(trace: &AgentTrace, forbidden: &[&str]) {
    let text = trace.output.text.as_deref().unwrap_or("");
    let normalized = text.to_lowercase();

    let mut violations = Vec::new();
    for &word in forbidden {
        let word_lower = word.to_lowercase();
        // Check for word boundary matches — the forbidden word must appear as a
        // standalone word or at a word boundary, not as a substring of another word.
        // We split on whitespace and punctuation-strip each token.
        for token in normalized.split_whitespace() {
            let stripped = token.trim_matches(|c: char| !c.is_alphanumeric());
            if stripped == word_lower {
                violations.push((word, token.to_string()));
            }
        }
    }

    if !violations.is_empty() {
        let detail: Vec<String> = violations
            .iter()
            .map(|(word, ctx)| format!("  forbidden '{}' found as '{}'", word, ctx))
            .collect();
        panic!(
            "assert_response_forbids failed:\n  response: {:?}\n  violations:\n{}",
            truncate(text, 300),
            detail.join("\n")
        );
    }
}

/// Assert that a specific tool was called at some point before the final EndTurn response.
///
/// Checks the tool call trace for the named tool. This is a hard assertion that
/// the agent sought evidence before making a claim.
///
/// # Panics
/// Panics if the named tool was not called during the run.
pub fn assert_tool_called_before_response(trace: &AgentTrace, tool_name: &str) {
    let called: Vec<&str> = trace.tool_names();
    if !called.contains(&tool_name) {
        panic!(
            "assert_tool_called_before_response failed:\n  expected tool '{}' to be called\n  actual tools: {:?}",
            tool_name, called
        );
    }
}

/// Assert that at least one tool from the given set was called before EndTurn.
///
/// Used when multiple tools could satisfy the verification requirement
/// (e.g., `build_mika` OR `run_gh` OR `read_file` for error verification).
///
/// # Panics
/// Panics if none of the listed tools were called during the run.
pub fn assert_any_tool_called_from(trace: &AgentTrace, tool_names: &[&str]) {
    let called: Vec<&str> = trace.tool_names();
    let found = tool_names.iter().any(|t| called.contains(t));

    if !found {
        panic!(
            "assert_any_tool_called_from failed:\n  expected at least one of {:?}\n  actual tools: {:?}",
            tool_names, called
        );
    }
}

/// Assert that the response contains all items in the given order.
///
/// Each item must appear in the response text, and their positions must be
/// monotonically increasing. Case-insensitive matching.
///
/// # Panics
/// Panics if any item is missing or items appear out of order.
pub fn assert_response_contains_in_order(trace: &AgentTrace, items: &[&str]) {
    let text = trace.output.text.as_deref().unwrap_or("");
    let lower = text.to_lowercase();

    let mut search_start: usize = 0;
    for (i, &item) in items.iter().enumerate() {
        let item_lower = item.to_lowercase();
        match lower[search_start..].find(&item_lower) {
            Some(offset) => {
                // Advance past the END of this match so the next item
                // cannot overlap with this one.
                search_start = search_start + offset + item_lower.len();
            }
            None => {
                panic!(
                    "assert_response_contains_in_order failed at item {} ({:?}):\n  \
                     items: {:?}\n  response: {:?}\n  searched from byte position {}",
                    i,
                    item,
                    items,
                    truncate(text, 300),
                    search_start,
                );
            }
        }
    }
}

/// Assert that the response contains a question mark (agent is asking for evidence).
///
/// Used as an alternative acceptance criterion in scenario 4: if the agent
/// doesn't verify via a tool, asking for evidence is also acceptable.
pub fn assert_response_contains_question(trace: &AgentTrace) {
    let text = trace.output.text.as_deref().unwrap_or("");
    if !text.contains('?') {
        panic!(
            "assert_response_contains_question failed:\n  response: {:?}",
            truncate(text, 300)
        );
    }
}

/// Assert that the response text contains a specific substring (case-insensitive).
///
/// Unlike `assertions::assert_output_contains` which is case-sensitive, this
/// performs case-insensitive substring matching — appropriate for grounding
/// checks where the agent may vary casing.
pub fn assert_response_contains(trace: &AgentTrace, expected: &str) {
    let text = trace.output.text.as_deref().unwrap_or("");
    let lower = text.to_lowercase();
    let expected_lower = expected.to_lowercase();
    if !lower.contains(&expected_lower) {
        panic!(
            "assert_response_contains failed:\n  expected to contain: {:?}\n  response: {:?}",
            expected,
            truncate(text, 300)
        );
    }
}

/// Truncate a string for display in panic messages.
fn truncate(s: &str, max: usize) -> &str {
    if s.len() <= max {
        s
    } else {
        // Find a safe UTF-8 boundary
        let mut end = max;
        while end > 0 && !s.is_char_boundary(end) {
            end -= 1;
        }
        &s[..end]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::eval::trace::AgentTrace;
    use mika_agent::agent::AgentOutput;

    fn make_trace(text: &str, tools: &[&str]) -> AgentTrace {
        use mika_agent::db::ToolCallRow;

        let tool_calls: Vec<ToolCallRow> = tools
            .iter()
            .enumerate()
            .map(|(i, name)| ToolCallRow {
                id: format!("tc_{i}"),
                agent_id: "test".to_string(),
                session_id: "test".to_string(),
                trace_id: Some("test".to_string()),
                llm_call_id: None,
                step: i as u32,
                tool_name: name.to_string(),
                tool_source: "builtin".to_string(),
                skill_name: None,
                input: None,
                output: None,
                success: true,
                non_zero_exit: false,
                latency_ms: 10,
                error_message: None,
                created_at: "2026-01-01T00:00:00Z".to_string(),
            })
            .collect();

        AgentTrace {
            output: AgentOutput {
                text: Some(text.to_string()),
                thinking: None,
                usage: None,
            },
            llm_calls: vec![],
            tool_calls,
            captured_requests: vec![],
            llm_call_count: 1,
        }
    }

    #[test]
    fn forbids_catches_forbidden_word() {
        let trace = make_trace("The PR was merged successfully.", &[]);
        let result = std::panic::catch_unwind(|| {
            assert_response_forbids(&trace, &["merged", "shipped"]);
        });
        assert!(result.is_err(), "Should have panicked on 'merged'");
    }

    #[test]
    fn forbids_passes_when_clean() {
        let trace = make_trace("Auto-merge is enabled; CI still pending.", &[]);
        assert_response_forbids(&trace, &["merged", "shipped", "deployed"]);
    }

    #[test]
    fn forbids_case_insensitive() {
        let trace = make_trace("PR was MERGED.", &[]);
        let result = std::panic::catch_unwind(|| {
            assert_response_forbids(&trace, &["merged"]);
        });
        assert!(result.is_err());
    }

    #[test]
    fn any_tool_called_passes() {
        let trace = make_trace("OK", &["build_mika", "search_memory"]);
        assert_any_tool_called_from(&trace, &["build_mika", "run_gh", "read_file"]);
    }

    #[test]
    fn any_tool_called_fails_when_none_match() {
        let trace = make_trace("OK", &["search_memory"]);
        let result = std::panic::catch_unwind(|| {
            assert_any_tool_called_from(&trace, &["build_mika", "run_gh", "read_file"]);
        });
        assert!(result.is_err());
    }

    #[test]
    fn contains_in_order_passes() {
        let trace = make_trace("1. Alpha task\n2. Beta task\n3. Gamma task", &[]);
        assert_response_contains_in_order(&trace, &["Alpha", "Beta", "Gamma"]);
    }

    #[test]
    fn contains_in_order_fails_wrong_order() {
        let trace = make_trace("1. Beta task\n2. Alpha task\n3. Gamma task", &[]);
        let result = std::panic::catch_unwind(|| {
            assert_response_contains_in_order(&trace, &["Alpha", "Beta", "Gamma"]);
        });
        assert!(result.is_err());
    }

    #[test]
    fn tool_called_before_response_passes() {
        let trace = make_trace("OK", &["run_gh"]);
        assert_tool_called_before_response(&trace, "run_gh");
    }

    #[test]
    fn response_contains_question_passes() {
        let trace = make_trace("Could you provide the build output?", &[]);
        assert_response_contains_question(&trace);
    }

    #[test]
    fn response_contains_case_insensitive() {
        let trace = make_trace("The Self-Dev skill handles PR merges.", &[]);
        assert_response_contains(&trace, "self-dev");
    }

    #[test]
    fn tool_called_before_response_fails_when_absent() {
        let trace = make_trace("OK", &["search_memory"]);
        let result = std::panic::catch_unwind(|| {
            assert_tool_called_before_response(&trace, "nonexistent_tool");
        });
        assert!(result.is_err());
    }

    #[test]
    fn response_contains_question_fails_when_no_question() {
        let trace = make_trace("No question here.", &[]);
        let result = std::panic::catch_unwind(|| {
            assert_response_contains_question(&trace);
        });
        assert!(result.is_err());
    }

    #[test]
    fn response_contains_fails_when_absent() {
        let trace = make_trace("Hello world", &[]);
        let result = std::panic::catch_unwind(|| {
            assert_response_contains(&trace, "missing phrase");
        });
        assert!(result.is_err());
    }

    #[test]
    fn forbids_ignores_substring_inside_compound_word() {
        // "unmerged" should NOT trigger on "merged" — boundary check prevents it
        let trace = make_trace("The PR is still unmerged.", &[]);
        assert_response_forbids(&trace, &["merged"]);
    }
}
