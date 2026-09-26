# Fabricated action guard bypass: verb synonym evasion

**Priority:** P2
**File:** `crates/mika-agent/src/agent.rs`
**Issue:** #308

## Problem

The `ACTION_CLAIM_RE` regex covers 8 verbs and 2 multi-word phrases, but LLMs commonly
use synonyms that bypass the guard entirely while conveying the same fabricated-action
semantics. The following verbs all evade detection:

- `added` ("I added a review comment at ...")
- `wrote` ("I wrote a comment on ...")
- `replied` ("I replied at ...")
- `approved` ("I approved the PR ...")
- `filed` ("I filed an issue ...")
- `raised` ("I raised an issue at ...")
- `merged` ("I merged the PR ...") -- note: `merged` IS caught by the completion-claim
  guard (#483) but only when `update_work_item_status` is in the registry and active
  work items exist, so it is not caught in all contexts.

Additionally, the implicit-claim pattern where the agent presents a fabricated URL
without any action verb is not detected:

- "Here's the PR comment: https://github.com/org/repo/pull/1#issuecomment-99"
- "See the comment here: https://github.com/org/repo/pull/1#issuecomment-99"

These implicit patterns are arguably the most dangerous because they are harder for
human reviewers to spot.

## Impact

An LLM that has learned the guard's verb list (through the correction message itself,
which names specific verbs like "claimed to have {verb}") could adapt to use synonyms.
The one-retry-only design means after the first correction the LLM sees the guard's
detection logic, and on subsequent turns it could choose a synonym to evade detection.

However, this is mitigated by:
1. The system prompt instructs "NEVER fabricate URLs" (primary defense)
2. The `tools_called.is_empty()` condition is very strong -- zero tool calls is unusual
   for an agent that claims to have done something
3. The guard is defense-in-depth, not the primary control

## Recommendation

Consider expanding the verb list to include common synonyms:

```
added|wrote|replied|approved|filed|raised|merged|requested|assigned
```

Alternatively, consider an inverted approach: detect any GitHub resource URL + zero
tool calls as suspicious (removing the verb requirement entirely), since a resource URL
with a numeric ID and zero tool calls is inherently suspicious regardless of surrounding
text. This would catch implicit-claim patterns too.

The tradeoff is false positives when the agent legitimately references a URL from a
previous turn's tool result (though `tools_called` accumulates across the entire loop,
so a URL from an earlier tool call in the same turn would correctly be allowed through).
