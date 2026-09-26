---
status: pending
priority: p2
issue_id: 691
tags: [code-review, security]
dependencies: []
---

# Add Warning When Sending API Key to OpenAI-Compatible Endpoints

## Problem Statement

The old `make_llm_provider()` had a security guard that prevented `MIKA_ANTHROPIC_API_KEY` from being sent to user-specified `OpenAiCompatible` endpoints. With the unified `MIKA_LLM_API_KEY`, a user who sets their Anthropic key and then switches `llm_model` to `openai-compatible/some-model` with `llm_base_url=https://untrusted-endpoint.com` will send their Anthropic key to that untrusted endpoint. This is a design trade-off documented in the solution doc, but the removed guard was there for a reason.

## Findings

- Old code: separate `anthropic_api_key` field was never sent to `OpenAiCompatible` providers
- New code: single `llm_api_key` is sent to all providers unconditionally
- The solution doc acknowledges this trade-off
- User misconfiguration could leak an Anthropic key to an untrusted endpoint

## Proposed Solutions

### Option A: Add a tracing::warn! for OpenAI-compatible providers
- Log a warning when `ProviderKind::OpenAiCompatible` is selected, noting the key will be sent to the custom endpoint
- **Pros:** Low effort, alerts users to potential misconfiguration
- **Cons:** Only visible in logs, not in TUI
- **Effort:** Small
- **Risk:** None

### Option B: Add a `mika doctor` check
- Warn in `mika doctor` when an Anthropic-format key is used with a non-Anthropic provider
- **Pros:** User-visible diagnostic
- **Cons:** Slightly more work
- **Effort:** Small
- **Risk:** None

## Recommended Action

*(To be filled during triage)*

## Technical Details

- **Affected files:** `crates/mika-common/src/config.rs` (`make_llm_provider()`)

## Acceptance Criteria

- [ ] Warning logged or displayed when Anthropic-format key is used with `OpenAiCompatible` provider
- [ ] User informed which endpoint their key will be sent to

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-03-17 | Created from code review of PR #193 | |

## Resources

- [PR #193](https://github.com/senara-solutions/mika/pull/193)
- `docs/solutions/architecture-patterns/unified-llm-api-key-consolidation.md`
