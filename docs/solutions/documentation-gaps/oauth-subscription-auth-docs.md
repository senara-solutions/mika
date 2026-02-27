---
title: User-facing documentation gaps for OAuth subscription token support
date: 2026-02-27
category: documentation-gaps
severity: medium
component:
  - docs (README.md, docs/getting-started.md, docs/configuration.md)
  - mika-common (Claude API client auth handling)
tags:
  - oauth-tokens
  - anthropic-api
  - documentation
  - credentials
  - user-onboarding
symptoms: |
  Code implementation of OAuth subscription token auto-detection was complete and working,
  but three user-facing documentation files still exclusively referenced API keys.
  New users following Quick Start or configuration guides would not discover that they
  could authenticate with their Claude subscription token instead of purchasing a paid API key.
root_cause: >
  Internal documentation (.env.example, CLAUDE.md, code comments, error messages) was
  updated during feature implementation, but user-facing guides were not synchronized.
---

# User-facing documentation gaps for OAuth subscription token support

## Problem

The Mika project added OAuth subscription token support to its Claude API client
(commit `44916b5`), allowing users to provide either a standard Anthropic API key
or a Claude subscription OAuth token via the `MIKA_ANTHROPIC_API_KEY` environment
variable. The system automatically detects which type was provided by checking the
`sk-ant-oat` prefix.

However, three user-facing documentation files were never updated to mention this:

- **README.md** -- Quick Start section only said "Set your API key"
- **docs/getting-started.md** -- Section 3 was titled "Setting up your API key" with
  no mention of OAuth tokens
- **docs/configuration.md** -- Settings Reference table, Security notes, and
  Environment Variables tables only referenced API keys

Users reading these files would have no awareness that they could use their Claude
subscription (OAuth token) instead of a paid API key.

## Root Cause

The documentation updates lagged behind the code implementation. The OAuth feature
commit updated internal documentation (`.env.example`, `CLAUDE.md`, code comments,
error messages) but did not touch the three main user-facing guides. This created
a gap where the feature was fully functional but undiscoverable through documentation.

## Solution

Three files were updated in commit `749b6db`:

### 1. README.md (line 55)

**Before:**
```bash
# Set your API key
```

**After:**
```bash
# Set your Anthropic credential (API key or Claude subscription token)
```

### 2. docs/getting-started.md (Section 3)

Renamed section from "Setting up your API key" to "Setting up your credentials"
and expanded it with:

- **Option A: Anthropic API key (default)** -- standard billed API key
- **Option B: Claude subscription OAuth token** -- uses Claude Pro/Team/Enterprise
  quota via `claude setup-token` from the Claude Code CLI
- **Token expiration note** -- re-run `claude setup-token` when tokens expire
- **Verification section** -- `mika config` shows `OAuth token [REDACTED]` or
  `API key [REDACTED]`

Also updated the Prerequisites section to reference "Anthropic credential" instead
of just "API key", with a link to the new credentials section.

### 3. docs/configuration.md

Four targeted updates:

- **Settings Reference table** -- `anthropic_api_key` description now mentions
  OAuth tokens and auto-detection from the `sk-ant-oat` prefix
- **Security notes** -- Added bullet about `mika config` credential type display,
  added bullet explaining the auto-detection mechanism (Bearer + `anthropic-beta`
  header for OAuth, `x-api-key` header for standard keys)
- **CLI mode env var table** -- Updated description to "Anthropic API key or OAuth
  subscription token"
- **Server mode env var table** -- Same update

## Verification

- All three files visually reviewed after editing
- `cargo test` passed (no code changes, sanity check)
- Documentation now matches the code: `OAUTH_TOKEN_PREFIX = "sk-ant-oat"` in
  `crates/mika-common/src/claude.rs`, `is_oauth_token()` helper in
  `crates/mika-cli/src/commands/config.rs`

## Prevention

### Documentation-first checklist for feature PRs

When adding authentication or configuration features, verify all of these are
updated before merging:

- [ ] `.env.example` -- environment variable reference with examples
- [ ] `CLAUDE.md` -- architecture/stack notes
- [ ] `README.md` -- quick start, feature overview
- [ ] `docs/configuration.md` -- detailed config reference table
- [ ] `docs/getting-started.md` -- onboarding flow (if it affects first-run)
- [ ] Error messages explain how to fix common misconfigurations

### Key insight

Implementation completeness does not equal documentation completeness. Treat doc
updates as part of the feature, not optional polish. Internal docs (.env.example,
CLAUDE.md, code comments) are necessary but not sufficient -- user-facing guides
are what new users actually read.

## Related

- `docs/plans/2026-02-27-feat-anthropic-oauth-subscription-auth-plan.md` -- original implementation plan
- `docs/solutions/security-issues/api-key-whitespace-opaque-401-error.md` -- related credential validation fix
- `crates/mika-common/src/claude.rs` -- `AnthropicAuth` enum and auto-detection logic
- `crates/mika-cli/src/commands/config.rs` -- `mika config` credential type display
