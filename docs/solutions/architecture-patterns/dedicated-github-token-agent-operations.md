---
title: "Dedicated GitHub token for agent operations"
category: architecture-patterns
date: 2026-03-26
updated: 2026-03-31
tags: [config, env-var, github, token, settings]
related_issues: [289, 359]
related_docs:
  - docs/solutions/architecture-patterns/config-key-rename-across-layers.md
  - docs/solutions/architecture-patterns/unified-llm-api-key-consolidation.md
  - docs/solutions/security-issues/env-var-leakage-exec-handler-child-processes.md
---

# Dedicated GitHub token for agent operations

## Problem

`MIKA_INVESTIGATE_GITHUB_TOKEN` was purpose-built for the investigation panel (dashboard issue creation) but was being reused across agent operations — context injection, work item enrichment, and dev-run PR merges. This conflated two different permission scopes: the investigation panel only needs Issues R/W, while agent operations need Pull requests R/W, Issues R/W, and Contents R.

Discovered when mika-qa failed to review a private repo PR — the investigation token returned 404 because it lacked Pull requests permission.

## Root Cause

No dedicated token existed for agent operations. When `github_token` was needed in `AgentParams`/`TeamAgentParams`/`SilentAgentParams`, the code directly read `settings.investigate_github_token` at ~10 construction sites. This meant expanding the investigation token's permissions was the only path — breaking the principle of least privilege.

## Solution

Added `MIKA_GITHUB_TOKEN` as a dedicated token for agent operations. The two tokens serve strictly separate purposes with no fallback between them.

### Key design decision: centralized method, no fallback

Instead of scattering token logic across 8+ construction sites, a single method on `Settings` returns the agent token:

```rust
impl Settings {
    /// Resolve the GitHub token for agent operations.
    /// Returns MIKA_GITHUB_TOKEN only — no fallback to MIKA_INVESTIGATE_GITHUB_TOKEN.
    pub fn agent_github_token(&self) -> Option<&str> {
        self.github_token.as_deref()
    }
}
```

All construction sites call `settings.agent_github_token()` — no duplicated logic.

**Why no fallback (#359):** The original implementation fell back to `MIKA_INVESTIGATE_GITHUB_TOKEN` for backward compatibility. This caused mika-qa to use the investigation PAT when `MIKA_GITHUB_TOKEN` was intentionally unset during a token swap, resulting in a self-approval attempt on a PR. Silent fallbacks mask configuration intent — when a token is deliberately unset, nothing should silently replace it.

### Token resolution matrix

| Scenario | Agent operations | Investigation panel |
|----------|-----------------|-------------------|
| Both tokens set | `MIKA_GITHUB_TOKEN` | `MIKA_INVESTIGATE_GITHUB_TOKEN` |
| Only agent token | `MIKA_GITHUB_TOKEN` | None (graceful degradation) |
| Only investigate token | None (graceful degradation) | `MIKA_INVESTIGATE_GITHUB_TOKEN` |
| Neither set | None | None |

### Investigation panel isolation

`investigate.rs` reads `settings.investigate_github_token` directly — it does not use `agent_github_token()`. The two token paths are completely independent.

## 9-layer checklist followed

This change followed the proven pattern from `config-key-rename-across-layers.md`:

1. `Settings` struct field (`github_token: Option<String>`)
2. `ConfigKeyInfo` registry entry (`ConfigBackend::Env`, `secret: true`)
3. `get_effective_value()` match arm
4. Manual `Debug` impl redaction
5. Construction sites (8 sites via `agent_github_token()`)
6. Test fixtures (2 `Settings` literals)
7. Handler script `unset` lists (shell-exec `run.sh`)
8. Documentation (`.env.example`, `CLAUDE.md`, `docs/configuration.md`)
9. CLI `doctor` + `setup` updates

## Prevention

- **Follow the 9-layer checklist** from `config-key-rename-across-layers.md` for any env var addition/rename.
- **Use centralized helper methods** for token resolution — don't scatter logic across construction sites.
- **Keep investigation panel isolated** — its token scope should never expand to cover agent operations.
- **Never add silent fallbacks between tokens** — when a token is deliberately unset, the system should degrade gracefully, not silently substitute a different-scoped token.
