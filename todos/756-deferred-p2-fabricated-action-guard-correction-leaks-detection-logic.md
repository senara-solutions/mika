# Fabricated action guard: correction message leaks detection logic

**Priority:** P2
**File:** `crates/mika-agent/src/agent.rs`
**Issue:** #308

## Problem

The correction message injected on detection includes the matched verb and URL:

```
[Your response was rejected because you claimed to have {verb} a resource ({url})
but you did not call any tool in this turn. You MUST use tools (e.g., run_gh) to
perform actions -- do not fabricate URLs or assume actions happened. Call the
appropriate tool now to actually perform the action, or explain that you cannot
perform it.]
```

This reveals to the LLM:
1. That the detection is verb-based (the matched verb is echoed back)
2. That the detection requires a URL (the URL is echoed back)
3. That zero tool calls is the trigger ("you did not call any tool in this turn")

After the one allowed retry, the LLM has enough information to evade the guard on
future turns by:
- Using a synonym verb not in the list
- Omitting the URL and making a vaguer claim
- Calling an innocuous tool first to make `tools_called` non-empty

## Impact

Medium. The one-retry cap means the LLM only gets one correction per turn, so
evasion requires planning across turns. However, the correction message teaches the
LLM exactly what triggered detection, which is counterproductive for a security guard.

This is a known tension in all four EndTurn guards -- the completion-claim guard
(#483) also echoes the matched keyword. The pattern is intentional: the correction
is designed to help the LLM recover and do the right thing, not to be adversarial.

## Recommendation

Consider whether the correction message should be less specific about detection
mechanics. For example:

```
[Your response was rejected because it appears to claim an action was performed
without actually performing it. You MUST use tools to perform actions. Call the
appropriate tool now, or explain that you cannot perform the action.]
```

This still guides the LLM toward the correct behavior without revealing the detection
heuristic. However, this may reduce correction effectiveness (the LLM may not
understand what was wrong). The existing approach is consistent with the other guards
and is a reasonable design choice.

No immediate action required -- this is a design tradeoff discussion, not a bug.
