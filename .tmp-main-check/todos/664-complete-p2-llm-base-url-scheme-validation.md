---
status: pending
priority: p2
issue_id: "664"
tags: [code-review, security]
dependencies: []
---

# Add URL Scheme Validation for llm_base_url

## Problem Statement

The `llm_base_url` config value is accepted as-is from environment variables or config files with no validation. A user could set `MIKA_LLM_BASE_URL=file:///etc/passwd` or a non-HTTP scheme. While reqwest would likely reject non-HTTP schemes at the transport level, early validation provides clearer error messages and defense-in-depth. The `validate_file_key` function in `validation.rs` has no case for `llm_base_url`.

## Findings

- `crates/mika-common/src/validation.rs` — match falls through to catch-all `_ => {}` for `llm_base_url`
- `crates/mika-common/src/llm/openai.rs:148` — base_url used directly in `format!("{}/chat/completions", ...)`
- Contrast with `routing_url` validation in `server/mod.rs` which validates scheme is `http`/`https`

## Proposed Solutions

### Option 1: Add validation arm in validate_file_key (Recommended)

Add a `"llm_base_url"` arm that rejects empty values and validates `http://` or `https://` scheme using `reqwest::Url::parse`.

**Pros:** Consistent with routing_url pattern. Early, clear error.
**Cons:** Minimal.
**Effort:** Small.

## Acceptance Criteria

- [ ] `validate_file_key("llm_base_url", "file:///etc/passwd")` returns error
- [ ] `validate_file_key("llm_base_url", "http://localhost:11434/v1")` succeeds
- [ ] `mika config set llm_base_url gopher://evil` rejected with clear message
