---
status: complete
priority: p3
issue_id: "036"
tags: [code-review, devex, rust-v2]
dependencies: []
---

# config/local.toml Not Gitignored + No .env.example

## Problem Statement

1. `config/local.toml` is loaded by config-rs but not in `.gitignore`. A developer who places API keys there could commit secrets.
2. No `.env.example` or documentation of required environment variables (`MIKA_ANTHROPIC_API_KEY`, `MIKA_ENCRYPTION_KEY`).

**Why it matters:** Accidental secret commits and poor developer onboarding experience.

## Findings

- **Source:** Security Sentinel (L1), Architecture Strategist (N)
- **Locations:** `.gitignore`, `crates/mika-common/src/config.rs:63-64`

## Proposed Solutions

### Option A: Add gitignore entries + .env.example (Recommended)
- Add `config/local.*` to `.gitignore`
- Create `.env.example` documenting required vars
- **Effort:** Small
- **Risk:** Low

## Acceptance Criteria

- [ ] `config/local.toml` is gitignored
- [ ] `.env.example` exists with all required env vars documented
