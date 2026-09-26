# Fabricated action guard bypass: unrelated tool call satisfies gate

**Priority:** P2
**File:** `crates/mika-agent/src/agent.rs`
**Issue:** #308

## Problem

The guard only fires when `tools_called.is_empty()` -- meaning zero tool calls across
all steps in the entire turn. If the agent calls ANY tool (even one completely unrelated
to the claimed action), the guard is bypassed.

Example attack sequence within a single turn:
1. Agent calls `search_memory` (or any innocuous tool) -- `tools_called` becomes non-empty
2. Agent responds with "I posted a comment at https://github.com/org/repo/pull/1#issuecomment-99"
3. Guard does not fire because `tools_called` is not empty

This is acknowledged as a design choice in the existing review
(`review-308-fabricated-action-guard.md` section 5b). The rationale is that if tools
were called, the URL might have come from a tool result.

## Impact

Medium. In the original #308 scenario (mika-qa fabricating PR comment URLs), the agent
was producing zero tool calls -- going straight to fabricated output. If the agent
evolves to call an unrelated tool first, this guard will not catch it.

However, the `required_tools` gate (guard 2 in the EndTurn chain) already handles the
case where specific tools MUST be called. Skills like `mika-qa` that declare
`required_tools = ["run_gh"]` will enforce that `run_gh` specifically was called,
regardless of other tool calls.

## Recommendation

No code change needed if the relevant skills declare `required_tools`. Verify that
the `mika-qa` skill (or whatever skill triggered #308) has `required_tools` configured
to require the actual GitHub-interaction tool. The fabricated-action guard then serves
as a catch-all for skills that do NOT declare `required_tools`.

For a stronger guard (future consideration): track which tool results contain GitHub
URLs and only suppress the guard when a tool result actually contained a URL matching
the one in the agent's response. This would be significantly more complex and is not
warranted for defense-in-depth.
