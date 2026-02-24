---
status: pending
priority: p3
issue_id: "174"
tags: [code-review, ux]
---

# Improve Bare /start Command Response Message

## Problem Statement
When a user sends bare `/start` (no pairing token), the gateway replies "I can only read text messages right now. Please type your message." This is misleading — the user DID send a text message. The real issue is the `/start` command had no pairing payload. This happens when users tap the bot's name in Telegram.

## Findings
- **Agent-native reviewer**: UX bug — message is factually wrong for bare /start

## Proposed Solutions

### Option A: Differentiate bare /start from non-text media (Recommended)
Handle `ParsedMessage::Unsupported` differently for bare `/start`:
- Bare /start: "Welcome! If you have an invite link, please use it to get started. If you're already set up, just type a message."
- Non-text (photo/sticker/voice): "I can only read text messages right now."

This requires adding a new `ParsedMessage` variant or handling bare /start in the gateway dispatch.
- **Effort**: Small (20 min)
- **Risk**: None

### Option B: Forward bare /start to agent for paired users
Check if chat_id is paired, and if so, forward as regular text so the agent can greet them.
- **Effort**: Medium (30 min)
- **Risk**: Low

## Technical Details
- **Affected files**: `crates/mika-gateway/src/telegram.rs:71-73`, `crates/mika-gateway/src/routes.rs:131-139`

## Acceptance Criteria
- [ ] Bare /start produces a contextually appropriate message
- [ ] Non-text media still shows "text messages only" message

## Work Log
- 2026-02-24: Created from code review of commit 9de9ba6

## Resources
- Commit: 9de9ba6
