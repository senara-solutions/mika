---
status: complete
priority: p3
issue_id: 353
tags: [code-review, testing, agent-native]
dependencies: []
---

# Add Server-Path Integration Test for Image Tool Results

## Problem Statement

The existing tests cover the executor's envelope protocol and the agent loop's `strip_prior_images`, but there is no integration test that verifies the full server path: inbound `MessageRequest` with images -> `AgentParams` -> tool that returns images -> `process_tool_calls` builds multi-block content. A test harness that exercises `handle_message` end-to-end would catch regressions.

## Findings

- **Source:** agent-native-reviewer
- **Location:** `crates/mika-agent/src/server/handlers.rs`
- **Evidence:** No integration test for image flow through server handler

## Proposed Solutions

### Option A: Add server handler integration test (Recommended)
Create a test that constructs a `MessageRequest` with images, invokes the handler, and verifies `AgentParams` receives images correctly. Mock the Claude API to verify multi-block tool results are serialized.
- Effort: Medium
- Risk: Low

## Acceptance Criteria

- [ ] Integration test exercises server handler with image payloads
- [ ] Test verifies images flow into AgentParams
- [ ] Test verifies multi-block tool results are correctly constructed

## Work Log

| Date | Action | Result |
|------|--------|--------|
| 2026-02-28 | Identified during agent-native review | Pending |
