---
status: pending
priority: p3
issue_id: 352
tags: [code-review, documentation, agent-native]
dependencies: []
---

# Document Outbound Image Limitation

## Problem Statement

The `MessageSender` trait only accepts `text: &str`. When the agent processes a tool result containing images, it can analyze the image (via Claude's vision) and describe it in text, but it cannot forward the actual image to the Telegram user. The gateway's `/send` endpoint likewise only accepts `{ chat_id, text, request_id }`.

This is an intentional design (tool-produced images are for the LLM's "eyes"), but should be documented to prevent confusion.

## Findings

- **Source:** agent-native-reviewer
- **Location:** `crates/mika-agent/src/messaging.rs:14-16` (trait), `crates/mika-gateway/src/routes.rs:568-607` (/send endpoint)
- **Evidence:** `MessageSender::send(&self, text: &str)` — text-only interface

## Proposed Solutions

### Option A: Add documentation note (Recommended)
Add a note to architecture docs and/or the `MessageSender` trait doc comment explaining that agent responses are text-only, and tool-produced images are consumed by the LLM for analysis, not forwarded to users.
- Effort: Trivial
- Risk: None

### Option B: Extend for future image forwarding
Add `send_rich(text: &str, images: &[ImageData])` to `MessageSender` and extend gateway `/send` to support Telegram `sendPhoto`. Future work for a separate PR.
- Effort: Medium
- Risk: Low

## Acceptance Criteria

- [ ] Architecture docs note that outbound messages are text-only
- [ ] MessageSender trait has a doc comment explaining the limitation

## Work Log

| Date | Action | Result |
|------|--------|--------|
| 2026-02-28 | Identified during agent-native review | Pending |
