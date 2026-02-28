---
status: pending
priority: p3
issue_id: "332"
tags: [code-review, api, agent-native]
dependencies: []
---

# Server Rejects Image-Only API Requests

## Problem Statement

The agent server rejects requests with empty `text` (line 69), even when `images` are present. The TUI allows image-only sends. A future WhatsApp adapter would need to know to inject synthetic text. This is a leaky abstraction.

## Findings

- Flagged by: agent-native-reviewer
- Location: `crates/mika-agent/src/server/handlers.rs:69-77`

## Proposed Solutions

### Option A: Allow empty text when images present
- **Pros:** Correct parity with TUI, future-proof for other channels
- **Effort:** Small

## Acceptance Criteria

- [ ] Empty text allowed when images are present
- [ ] Empty text without images still rejected
