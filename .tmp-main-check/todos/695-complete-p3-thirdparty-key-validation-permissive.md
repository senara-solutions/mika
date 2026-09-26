---
status: pending
priority: p3
issue_id: 695
tags: [code-review, security]
dependencies: []
---

# Add Minimum Length Check for ThirdParty API Keys

## Problem Statement

`validate_api_key_format()` now accepts any non-empty string as `ThirdParty`. This means `mika doctor` reports PASS for obviously invalid values like `"test"` or `"password123"`. Most real API keys from OpenAI, Groq, Mistral, etc. are 40+ characters.

## Proposed Solutions

### Option A: Add minimum length check (e.g., 20 chars)
- Return error for suspiciously short keys
- **Pros:** Catches obvious misconfigurations
- **Cons:** Could reject some edge-case valid keys
- **Effort:** Small
- **Risk:** Low

## Acceptance Criteria

- [ ] `mika doctor` warns on suspiciously short third-party keys

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-03-17 | Created from code review of PR #193 | |
