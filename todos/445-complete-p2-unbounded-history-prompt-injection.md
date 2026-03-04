---
status: complete
priority: p2
issue_id: "445"
tags: [code-review, security, prompt-injection]
dependencies: []
---

# Unbounded History Prompt Injection Surface

## Problem Statement

In `crates/mika-agent/src/teams/prompt.rs:29-48`, up to 10 goals (no length limit) and 10 deliverables (500 char limit) are injected into the orchestrator system prompt. Previous deliverables (LLM outputs) are injected without data delimiters.

## Fix

1. Truncate `run.goal` to 500 chars (using `floor_char_boundary`)
2. Add total character budget (5000) for the entire history section
3. Wrap entries in `<context>` delimiters matching existing Mika patterns

## Acceptance Criteria

- [ ] Goals truncated to 500 chars
- [ ] Total history section capped at 5000 chars
- [ ] Entries wrapped in delimiters
- [ ] Tests cover truncation
