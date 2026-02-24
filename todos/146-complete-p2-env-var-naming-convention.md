---
status: complete
priority: p2
issue_id: "146"
tags: [plan-review, architecture, conventions]
dependencies: []
---

# Gateway env vars should follow MIKA_ prefix convention

## Problem Statement
The plan specifies env vars like `TELEGRAM_BOT_TOKEN`, `DATABASE_URL`, and `GATEWAY_PORT` without the `MIKA_` prefix. The existing codebase uses `MIKA_` prefix for all config (MIKA_ANTHROPIC_API_KEY, MIKA_INTERNAL_TOKEN, etc.) via config-rs with `Environment::with_prefix("MIKA")`.

**Why it matters:** Inconsistent naming creates confusion and makes it harder to identify Mika-specific env vars in a K8s deployment with many services.

## Findings
- Source: Architecture Strategist (Medium)
- Existing convention: All env vars use MIKA_ prefix (see mika-common/src/config.rs)
- Plan uses: TELEGRAM_BOT_TOKEN, DATABASE_URL, GATEWAY_PORT
- Should be: MIKA_TELEGRAM_BOT_TOKEN, MIKA_DATABASE_URL, MIKA_GATEWAY_PORT

## Proposed Solutions

### Option 1: Use MIKA_ prefix consistently (Recommended)
Rename all gateway env vars to use MIKA_ prefix. Use config-rs in the gateway crate matching the existing pattern.
- **Pros**: Consistent with codebase, clear namespace
- **Cons**: Slightly longer names
- **Effort**: Small
- **Risk**: Low

## Technical Details
- **Affected files**: Plan config.rs, .env.example, deployment manifests

## Acceptance Criteria
- [ ] All gateway env vars use MIKA_ prefix
- [ ] Gateway uses config-rs with MIKA prefix (matching mika-common pattern)
- [ ] .env.example updated with correct names

## Work Log
### 2026-02-24 - Discovery
**By:** Claude Code (multi-agent plan review)
**Actions:** Architecture Strategist flagged env var naming inconsistency
