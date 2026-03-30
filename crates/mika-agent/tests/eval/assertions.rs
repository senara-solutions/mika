//! Assertion helpers for `AgentTrace`.
//!
//! All functions produce clear failure messages with actual vs expected values.

use std::collections::HashSet;

use mika_agent::db::ToolCallRow;

use super::trace::AgentTrace;

/// Assert the exact set of tools that were called (set equality).
///
/// Fails if any expected tool was not called, or any unexpected tool was called.
pub fn assert_tools(trace: &AgentTrace, expected: &[&str]) {
    let actual: HashSet<&str> = trace.tool_names().into_iter().collect();
    let expected_set: HashSet<&str> = expected.iter().copied().collect();

    let missing: Vec<&&str> = expected.iter().filter(|t| !actual.contains(**t)).collect();
    let extra: Vec<&&str> = actual
        .iter()
        .filter(|t| !expected_set.contains(**t))
        .collect();

    if !missing.is_empty() || !extra.is_empty() {
        panic!(
            "assert_tools failed:\n  expected: {:?}\n  actual:   {:?}\n  missing:  {:?}\n  extra:    {:?}",
            expected,
            trace.tool_names(),
            missing,
            extra
        );
    }
}

/// Assert that at least these tools were called (subset check).
pub fn assert_tools_include(trace: &AgentTrace, expected: &[&str]) {
    let actual: HashSet<&str> = trace.tool_names().into_iter().collect();
    let missing: Vec<&&str> = expected.iter().filter(|t| !actual.contains(**t)).collect();

    if !missing.is_empty() {
        panic!(
            "assert_tools_include failed:\n  expected to include: {:?}\n  actual tools: {:?}\n  missing: {:?}",
            expected,
            trace.tool_names(),
            missing
        );
    }
}

/// Assert that none of these tools were called (exclusion check).
pub fn assert_tools_exclude(trace: &AgentTrace, excluded: &[&str]) {
    let actual: HashSet<&str> = trace.tool_names().into_iter().collect();
    let present: Vec<&&str> = excluded.iter().filter(|t| actual.contains(**t)).collect();

    if !present.is_empty() {
        panic!(
            "assert_tools_exclude failed:\n  should not have called: {:?}\n  actual tools: {:?}\n  found: {:?}",
            excluded,
            trace.tool_names(),
            present
        );
    }
}

/// Assert tools were called in the given relative order (subsequence check).
///
/// Other tools may appear between the expected tools.
pub fn assert_tool_order(trace: &AgentTrace, expected_order: &[&str]) {
    let names = trace.tool_names();
    let mut idx = 0;
    for &expected in expected_order {
        match names[idx..].iter().position(|n| *n == expected) {
            Some(pos) => idx += pos + 1,
            None => {
                panic!(
                    "assert_tool_order failed:\n  expected order: {:?}\n  actual order:   {:?}\n  could not find '{}' after position {}",
                    expected_order, names, expected, idx
                );
            }
        }
    }
}

/// Assert the arguments of the Nth call to a named tool (exact match).
pub fn assert_tool_args(
    trace: &AgentTrace,
    tool_name: &str,
    call_index: usize,
    expected: serde_json::Value,
) {
    let calls: Vec<&ToolCallRow> = trace.calls_for_tool(tool_name);
    if call_index >= calls.len() {
        panic!(
            "assert_tool_args failed: tool '{}' was called {} times, but expected call_index {}",
            tool_name,
            calls.len(),
            call_index
        );
    }
    let input_str = calls[call_index].input.as_deref().unwrap_or("{}");
    let actual: serde_json::Value = serde_json::from_str(input_str)
        .unwrap_or_else(|_| serde_json::Value::String(input_str.to_string()));
    if actual != expected {
        panic!(
            "assert_tool_args failed for '{}' call {}:\n  expected: {}\n  actual:   {}",
            tool_name, call_index, expected, actual
        );
    }
}

/// Assert the arguments of the Nth call contain the expected keys/values (partial match).
pub fn assert_tool_args_contain(
    trace: &AgentTrace,
    tool_name: &str,
    call_index: usize,
    expected: serde_json::Value,
) {
    let calls: Vec<&ToolCallRow> = trace.calls_for_tool(tool_name);
    if call_index >= calls.len() {
        panic!(
            "assert_tool_args_contain failed: tool '{}' was called {} times, but expected call_index {}",
            tool_name,
            calls.len(),
            call_index
        );
    }
    let input_str = calls[call_index].input.as_deref().unwrap_or("{}");
    let actual: serde_json::Value = serde_json::from_str(input_str)
        .unwrap_or_else(|_| serde_json::Value::String(input_str.to_string()));

    if let (Some(expected_obj), Some(actual_obj)) = (expected.as_object(), actual.as_object()) {
        for (key, val) in expected_obj {
            match actual_obj.get(key) {
                Some(actual_val) if actual_val == val => {}
                Some(actual_val) => {
                    panic!(
                        "assert_tool_args_contain failed for '{}' call {}: key '{}'\n  expected: {}\n  actual:   {}",
                        tool_name, call_index, key, val, actual_val
                    );
                }
                None => {
                    panic!(
                        "assert_tool_args_contain failed for '{}' call {}: missing key '{}'\n  actual keys: {:?}",
                        tool_name,
                        call_index,
                        key,
                        actual_obj.keys().collect::<Vec<_>>()
                    );
                }
            }
        }
    } else {
        panic!(
            "assert_tool_args_contain requires object values.\n  expected: {}\n  actual:   {}",
            expected, actual
        );
    }
}

/// Assert the agent loop took at most `max` steps.
pub fn assert_max_steps(trace: &AgentTrace, max: usize) {
    if trace.steps > max {
        panic!(
            "assert_max_steps failed: expected at most {} steps, got {}",
            max, trace.steps
        );
    }
}

/// Assert the agent loop took exactly `expected` steps.
pub fn assert_exact_steps(trace: &AgentTrace, expected: usize) {
    if trace.steps != expected {
        panic!(
            "assert_exact_steps failed: expected {} steps, got {}",
            expected, trace.steps
        );
    }
}

/// Assert the agent's output text contains a substring.
pub fn assert_output_contains(trace: &AgentTrace, substring: &str) {
    let text = trace.output.text.as_deref().unwrap_or("");
    if !text.contains(substring) {
        panic!(
            "assert_output_contains failed:\n  expected to contain: {:?}\n  actual output: {:?}",
            substring, text
        );
    }
}

/// Assert the final stop reason of the last LLM call.
pub fn assert_stop_reason(trace: &AgentTrace, expected: &str) {
    if let Some(last) = trace.llm_calls.last() {
        let actual = last.stop_reason.as_deref().unwrap_or("unknown");
        if actual != expected {
            panic!(
                "assert_stop_reason failed:\n  expected: {:?}\n  actual:   {:?}",
                expected, actual
            );
        }
    } else {
        panic!("assert_stop_reason failed: no LLM calls recorded");
    }
}

/// Assert no tool calls resulted in errors.
pub fn assert_no_tool_errors(trace: &AgentTrace) {
    let failures: Vec<_> = trace
        .tool_calls
        .iter()
        .filter(|tc| !tc.success)
        .map(|tc| format!("{}(step {})", tc.tool_name, tc.step))
        .collect();
    if !failures.is_empty() {
        panic!(
            "assert_no_tool_errors failed: {} tool(s) failed: {:?}",
            failures.len(),
            failures
        );
    }
}

/// Assert the output of the Nth call to a named tool contains a substring.
pub fn assert_tool_output_contains(
    trace: &AgentTrace,
    tool_name: &str,
    call_index: usize,
    substring: &str,
) {
    let calls: Vec<&ToolCallRow> = trace.calls_for_tool(tool_name);
    if call_index >= calls.len() {
        panic!(
            "assert_tool_output_contains failed: tool '{}' was called {} times, but expected call_index {}",
            tool_name,
            calls.len(),
            call_index
        );
    }
    let output = calls[call_index].output.as_deref().unwrap_or("");
    if !output.contains(substring) {
        panic!(
            "assert_tool_output_contains failed for '{}' call {}:\n  expected to contain: {:?}\n  actual output: {:?}",
            tool_name,
            call_index,
            substring,
            &output[..output.len().min(500)]
        );
    }
}

/// Assert no tools were called during the run.
pub fn assert_no_tools(trace: &AgentTrace) {
    if !trace.tool_calls.is_empty() {
        panic!(
            "assert_no_tools failed: {} tool(s) were called: {:?}",
            trace.tool_calls.len(),
            trace.tool_names()
        );
    }
}

/// Assert the system prompt (from the first captured request) contains a substring.
pub fn assert_system_prompt_contains(trace: &AgentTrace, substring: &str) {
    if trace.captured_requests.is_empty() {
        panic!("assert_system_prompt_contains failed: no captured requests");
    }
    let system = trace.captured_requests[0].system.as_deref().unwrap_or("");
    if !system.contains(substring) {
        panic!(
            "assert_system_prompt_contains failed:\n  expected to contain: {:?}\n  system prompt length: {} chars",
            substring,
            system.len()
        );
    }
}

/// Assert that specific tools were registered (present in tool definitions).
pub fn assert_tools_registered(trace: &AgentTrace, tool_names: &[&str]) {
    if trace.captured_requests.is_empty() {
        panic!("assert_tools_registered failed: no captured requests");
    }
    let registered: HashSet<&str> = trace.captured_requests[0]
        .tools
        .as_ref()
        .map(|tools| tools.iter().map(|t| t.name.as_str()).collect())
        .unwrap_or_default();
    let missing: Vec<&&str> = tool_names
        .iter()
        .filter(|n| !registered.contains(**n))
        .collect();
    if !missing.is_empty() {
        panic!(
            "assert_tools_registered failed:\n  expected to be registered: {:?}\n  missing: {:?}\n  registered count: {}",
            tool_names,
            missing,
            registered.len()
        );
    }
}

/// Assert the agent produced some text output (not None/empty).
pub fn assert_has_output(trace: &AgentTrace) {
    match &trace.output.text {
        Some(text) if !text.trim().is_empty() => {}
        _ => {
            panic!("assert_has_output failed: agent produced no text output");
        }
    }
}
