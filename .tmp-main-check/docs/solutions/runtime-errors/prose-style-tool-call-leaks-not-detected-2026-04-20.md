---
title: "Prose-style tool call leaks not detected by EndTurn guards"
date: 2026-04-20
category: runtime-errors
module: mika-agent::agent
problem_type: runtime_error
component: assistant
symptoms:
  - "LLM outputs tool_name({\"key\": \"value\"}) as prose text instead of structured tool call"
  - "Tool call appears as rendered text in TUI but never executes"
  - "tool_calls table has no entry for the prose-style invocation"
  - "Agent proceeds as if tool was called — inconsistent state"
root_cause: logic_error
resolution_type: code_fix
severity: medium
tags: [tool-calling, prose-detection, agent-loop, endturn-guard, post-condition]
---

# Prose-style tool call leaks not detected by EndTurn guards

## Problem

The LLM occasionally emits tool invocations as function-call-style prose text — e.g., `check_work_item({"task_id": "48cbb025-..."})` — instead of using the structured tool-calling API. The existing five EndTurn guards did not detect this pattern, so the text rendered verbatim in the TUI and the tool never executed, leaving the agent in an inconsistent state.

## Symptoms

- `check_work_item({"task_id": "48cbb025-6d8e-430f-a957-9ce2e32800bb"})` appeared as rendered text in a mika-dev session (observed 2026-04-14)
- The `tool_calls` table confirmed no tool call with that UUID was made on that turn
- The agent proceeded as if the tool had been called, potentially cascading into fabricated follow-up claims

## What Didn't Work

- The existing Layer 2 guard (`detect_text_based_tool_call`) only catches XML-style patterns (`<function=...>`) — its fast path returns `false` if the text contains no `<` character, which is the case for prose-style patterns
- The completion-claim and fabricated-action guards detect different failure classes entirely — they wouldn't fire on this pattern

## Solution

Added a new EndTurn post-condition guard `detect_prose_style_tool_call()` as guard #2 in the chain (between Layer 2 XML detection and the required-tools gate).

**Detection approach — two-phase with tool-name gating:**

1. Static `LazyLock<Regex>` with pattern `\b(\w+)\s*\(\s*\{` extracts candidate identifiers
2. Each candidate is checked against a `HashSet<String>` of registered tool names (builtins + skills + MCP)
3. Returns `Some(tool_name)` on first match for the re-prompt message

**Key design decisions:**

- **Fast-path:** `!text.contains('(')` returns early on the vast majority of responses with zero regex cost
- **Tool-name gating:** Only fires when the identifier matches a registered tool — eliminates false positives on code examples and explanatory prose like `"Use check_work_item to verify..."`
- **Separate retry flag:** `prose_tool_call_retry_done` is independent from `text_tool_call_retry_done` because a turn could theoretically contain both XML-style and prose-style leaks
- **HashSet built once:** The tool name set is constructed before the loop (alongside `effective_required_tools`) — the tool set is stable across loop iterations

```rust
static PROSE_TOOL_CALL_RE: std::sync::LazyLock<regex::Regex> =
    std::sync::LazyLock::new(|| regex::Regex::new(r"\b(\w+)\s*\(\s*\{").unwrap());

fn detect_prose_style_tool_call(
    text: &str,
    tool_names: &HashSet<String>,
) -> Option<String> {
    if !text.contains('(') {
        return None;
    }
    for caps in PROSE_TOOL_CALL_RE.captures_iter(text) {
        if let Some(m) = caps.get(1) {
            let candidate = m.as_str();
            if tool_names.contains(candidate) {
                return Some(candidate.to_string());
            }
        }
    }
    None
}
```

## Why This Works

The root cause is that the LLM sometimes outputs tool invocations in a function-call text format (`name({...})`) rather than using the structured tool-calling API. This goes against the model gradient — the model "wants" to demonstrate tool use textually rather than invoke the API.

The regex `\b(\w+)\s*\(\s*\{` captures the universal shape of these leaks: an identifier immediately followed by a parenthesized JSON object. Gating against the registered tool set is the critical innovation — it means the guard only fires when the identifier is an actual tool the agent could have called, not random function-like syntax in explanatory text.

The single-retry pattern (same as all other guards) bounds the cost: one re-prompt telling the agent to use the structured API, then if it still leaks, the response passes through.

## Prevention

- All six EndTurn guards now cover both XML-style (`<function=...>`) and prose-style (`name({...})`) tool call leaks
- 10 unit tests cover the detection function: positive matches, negative cases (unknown identifiers, explanatory prose, empty text), whitespace variations, multiline JSON
- 2 eval harness integration tests prove the full guard lifecycle (guard fires → re-prompt → structured call on retry) and the negative case (unknown tool → no retry)
- When adding new tool-call leak patterns in the future, follow the established guard pattern: fast-path substring check → `LazyLock<Regex>` → `_retry_done` flag → push assistant + correction messages → continue

## Related Issues

- #569 — this issue
- #447 — Layer 1 XML extraction + Layer 2 text-based tool call detector (predecessor)
- #308 — fabricated action-claim guard
- #483 — completion-claim guard
- #582 — per-turn tool use dedup guard
- #648 — persistence evaluation guard
- `docs/solutions/runtime-errors/xml-tool-calls-not-executed.md` — predecessor guard for the XML surface
- `docs/solutions/architecture-patterns/engine-guards-vs-prompt-rules-for-agent-behavior-2026-04-19.md` — justification for engine-level guards
