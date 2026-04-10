---
title: "fix(skills): harden skill-review prompt to enforce full inspect→persist→verify cycle"
type: fix
status: completed
date: 2026-04-10
issue: 514
---

# Fix: Harden Skill-Review Prompt

## Problem

The `skill-review` built-in skill's system prompt is too permissive — agents sometimes stop after the inspect step without persisting the variant, or persist invalid content (JSON/dicts instead of markdown). The `content` parameter description in `tools.json` doesn't specify format requirements, leading to malformed variants.

## Root Cause

1. The system prompt's workflow section described the two-call pattern but didn't make it mandatory with explicit enforcement language.
2. No verification step existed — agents claimed success without checking the tool response for `"written": true`.
3. The `content` parameter description in `tools.json` was generic ("Full adapted prompt content") with no format requirements.
4. No loop prevention — agents could re-inspect endlessly without persisting.
5. No tool disambiguation — agents sometimes used `write_agent_file` or `update_skill` instead of `review_skill`.

## Solution

### `system_prompt.md` changes
- Replace permissive opening with mandatory 3-step sequence (INSPECT → PERSIST → VERIFY)
- Add **MANDATORY SEQUENCE** block with explicit no-exceptions language
- Add **VERIFY** step requiring `"written": true` confirmation in tool response
- Add **LOOP PREVENTION** guard (max 3 calls per skill)
- Add **TOOL DISAMBIGUATION** clarifying `review_skill` is the only correct tool
- Add **ARTIFACT VERIFICATION** block requiring response checking after every persist call

### `tools.json` changes
- Improve `content` parameter description to require markdown format starting with `##` heading
- Explicitly reject JSON/dict/summary content
- Add minimum size requirement (50% of root prompt)

## Impact

- **No Rust code changes** — prompt-only modifications to bundled skill templates
- **No schema changes** — no DB migrations
- **No API changes** — no endpoint modifications
- **Backward compatible** — existing variants remain valid

## Files Changed

| File | Change |
|------|--------|
| `crates/mika-agent/templates/skills/skill-review/system_prompt.md` | Hardened prompt with mandatory sequence, verification, loop prevention |
| `crates/mika-agent/templates/skills/skill-review/tools.json` | Improved `content` parameter description with format requirements |

## Testing

Manual validation only — this is a prompt-level change. The skill-review tool handler enforces the 50% size check and path computation server-side; the prompt changes ensure the agent follows the correct workflow.
