---
status: ready
priority: p2
issue_id: "163"
tags: [code-review, ux]
---

# Handle Bare /start Command (No Payload)

## Problem Statement
`text.strip_prefix("/start ")` in telegram.rs:73 requires a space after `/start`. Bare `/start` (no token) falls through to `ParsedMessage::Text` and gets forwarded to the container as regular text. The user likely clicked a malformed deep link.

## Findings
- **Agent-native reviewer**: UX issue — container processes "/start" as user text instead of helpful response

## Proposed Solutions

### Option A: Treat bare /start as Unsupported with friendly reply (Recommended)
```rust
// In parse_update:
if text == "/start" || text.strip_prefix("/start ").map_or(false, |p| p.trim().is_empty()) {
    return ParsedMessage::Unsupported { chat_id };
}
```
Then the Unsupported handler sends "I can only read text messages..." — or add a specific Start variant for this case.
- Effort: Small (10 min)
- Risk: None

## Technical Details
- **Affected files**: `crates/mika-gateway/src/telegram.rs` (parse_update function)

## Acceptance Criteria
- [ ] Bare `/start` (no payload) does not forward to container
- [ ] User gets a helpful reply
- [ ] Test added for bare `/start`

## Work Log
- 2026-02-24: Created from PR #6 code review

## Resources
- PR: #6
