---
status: pending
priority: p1
issue_id: "118"
tags: [code-review, architecture, agent-native]
dependencies: []
---

# Hardcoded Channel Filter Excludes WhatsApp from Agent Context

## Problem Statement

In `agent.rs:125`, the conversation history retrieval uses a hardcoded channel filter `Some(vec!["cli", "telegram"])` which explicitly excludes WhatsApp messages. When WhatsApp support is added in Phase 2, agent context will silently miss all WhatsApp conversation history, making the agent appear to have amnesia for WhatsApp users.

## Findings

- **Source:** agent-native-reviewer (CRITICAL-1)
- **Location:** `crates/mika-agent/src/agent.rs:125`
- **Evidence:** `let messages = db.get_recent_messages(50, Some(vec!["cli", "telegram"])).await?;` — only "cli" and "telegram" are included
- **Impact:** WhatsApp messages would be stored but never retrieved for agent context, breaking conversation continuity for WhatsApp users

## Proposed Solutions

### Option 1: Remove channel filter entirely
- **Pros**: All channels included automatically, no maintenance when adding new channels
- **Cons**: Could include irrelevant cross-channel context (unlikely in single-user containers)
- **Effort**: Small (change `Some(vec![...])` to `None`)
- **Risk**: Low

### Option 2: Add "whatsapp" to the filter list
- **Pros**: Explicit control over which channels provide context
- **Cons**: Must update every time a new channel is added — easy to forget
- **Effort**: Small
- **Risk**: Medium (same bug will recur for next channel)

## Recommended Action

Option 1 — remove the filter. In a single-user container, all messages are from the same person. Cross-channel context is a feature, not a bug.

## Technical Details

- **Affected Files**: `crates/mika-agent/src/agent.rs`
- **Related Components**: Agent loop, conversation context assembly
- **Database Changes**: None

## Acceptance Criteria

- [ ] Agent retrieves messages from all channels (no filter)
- [ ] Existing tests pass unchanged
- [ ] WhatsApp messages (when added) will appear in agent context

## Work Log

### 2026-02-24 - Identified during PR #5 review
**By:** agent-native-reviewer
**Actions:** Flagged hardcoded channel filter as P1 — breaks agent-native parity for future channels

## Resources

- PR #5: Phase 2 Container HTTP Server
- Related: WhatsApp channel adapter (Phase 2 planned work)
