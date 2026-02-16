---
status: pending
priority: p2
issue_id: "364"
tags: [code-review, configuration, gateway]
dependencies: []
---

# Add Gateway Environment Variables to .env.example

## Problem Statement

Now that mika-gateway lives in the public repo, developers will look to `.env.example` for setup guidance. The file currently lacks the five required gateway-only environment variables.

## Findings

Missing variables:
- `MIKA_DATABASE_URL` — Postgres connection string
- `MIKA_TELEGRAM_BOT_TOKEN` — Telegram Bot API token
- `MIKA_TELEGRAM_WEBHOOK_SECRET` — 64-char hex secret for webhook validation
- `MIKA_TELEGRAM_WEBHOOK_URL` — Public webhook URL
- `MIKA_GATEWAY_PORT` — Listen port (optional, default 8080)
- `MIKA_AGENT_BASE_URL` — Override for local E2E testing (optional)

Note: `MIKA_INTERNAL_TOKEN` is already documented.

## Proposed Solutions

### Option A: Add commented gateway section (Recommended)
- Add a clearly labeled "Gateway mode" section with all vars commented out
- Effort: Small
- Risk: None

## Technical Details

**Affected files:**
- `.env.example`

## Acceptance Criteria

- [ ] All gateway-specific env vars are documented in .env.example
- [ ] Variables are in a clearly labeled section
- [ ] Each variable has a description comment
