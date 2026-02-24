---
status: complete
priority: p1
issue_id: "180"
tags: [code-review, security, plan-review]
dependencies: []
---

# Shell Injection via API Key in AWS Secret-String JSON

## Problem Statement
The provision.sh script interpolates MIKA_ANTHROPIC_API_KEY directly into a JSON string for the AWS CLI: `--secret-string "{\"anthropic_api_key\": \"${MIKA_ANTHROPIC_API_KEY}\"}"`. If the API key contains double quotes, backslashes, or shell metacharacters, the JSON is malformed or command injection occurs.

## Findings
- **Security sentinel**: P1 — Shell expansion of API key value in JSON string

## Proposed Solutions

### Option A: Use jq for safe JSON construction (Recommended)
```bash
SECRET_JSON=$(jq -n --arg key "${MIKA_ANTHROPIC_API_KEY}" '{"anthropic_api_key": $key}')
aws secretsmanager create-secret \
    --name "${AWS_SECRET_NAME}" \
    --secret-string "${SECRET_JSON}" \
    --region "${AWS_REGION}" ...
```
- **Effort**: Small (15 min)
- **Risk**: None (adds jq as dependency — commonly available)

### Option B: Use printf with proper escaping
```bash
printf -v SECRET_JSON '{"anthropic_api_key": "%s"}' "${MIKA_ANTHROPIC_API_KEY//\"/\\\"}"
```
- **Effort**: Small
- **Risk**: Low — manual escaping is error-prone

## Acceptance Criteria
- [ ] API key not interpolated directly into shell-expanded JSON string
- [ ] Works with API keys containing special characters

## Work Log

### 2026-02-24 - Plan Review Finding
**By:** Security sentinel
