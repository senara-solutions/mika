---
title: "feat: add MIKA_GITHUB_TOKEN for agent GitHub operations"
type: feat
status: completed
date: 2026-03-26
issue: 289
---

# feat: add MIKA_GITHUB_TOKEN for agent GitHub operations

## Overview

The context injection feature (PR #282) reuses `MIKA_INVESTIGATE_GITHUB_TOKEN` for GitHub REST API calls (fetching PR diffs, enriching work items). This token was purpose-built for the investigation panel (Issues + Metadata only) — it shouldn't be expanded for agent operations like diff fetching and PR merging.

A dedicated `MIKA_GITHUB_TOKEN` with Pull requests R/W, Issues R/W, Contents R is the right token for the job. The code needs to read it and prefer it over the investigation token, with graceful fallback for backward compatibility.

## Token Resolution

| Consumer | Resolution |
|----------|-----------|
| Agent operations (context injection, `check_work_item`, dev-run merge) | `MIKA_GITHUB_TOKEN` → `MIKA_INVESTIGATE_GITHUB_TOKEN` → None |
| Investigation panel (`investigate.rs`) | `MIKA_INVESTIGATE_GITHUB_TOKEN` only (unchanged) |

## Acceptance Criteria

- [x] New `github_token: Option<String>` field on `Settings` reads `MIKA_GITHUB_TOKEN`
- [x] `ConfigKeyInfo` registry entry with `ConfigBackend::Env`, `secret: true`
- [x] `get_effective_value()` match arm for `"github_token"`
- [x] Manual `Debug` impl redacts `github_token`
- [x] All agent construction sites resolve token as `github_token.or(investigate_github_token)`
- [x] Investigation panel (`investigate.rs`) continues using `investigate_github_token` only — no fallback
- [x] `dashboard_dev_runs.rs` merge handler uses fallback pattern (it's an agent operation)
- [x] Handler script `unset` lists updated with `MIKA_GITHUB_TOKEN`
- [x] `mika doctor` checks for `MIKA_GITHUB_TOKEN` alongside existing check
- [x] `mika setup` prompts for `MIKA_GITHUB_TOKEN` in interactive and compose modes
- [x] `.env.example` documents new env var with clear scope distinction
- [x] `CLAUDE.md` environment variables section updated
- [x] All tests pass (`cargo test`)

## Construction Sites (9-layer checklist)

Follows the proven pattern from `docs/solutions/architecture-patterns/config-key-rename-across-layers.md`.

### Layer 1: Settings struct

**`crates/mika-common/src/config.rs`**

Add field near `investigate_github_token` (~line 590):

```rust
#[serde(default)]
pub github_token: Option<String>,
```

### Layer 2: ConfigKeyInfo registry

**`crates/mika-common/src/config.rs`** — Add entry near the investigate token entry (~line 333):

```rust
ConfigKeyInfo {
    key: "github_token",
    backend: ConfigBackend::Env,
    env_var: Some("MIKA_GITHUB_TOKEN"),
    secret: true,
    description: "GitHub token for agent operations (context injection, work item enrichment, PR merge)",
},
```

### Layer 3: get_effective_value()

**`crates/mika-common/src/config.rs`** — Add match arm (~line 435):

```rust
"github_token" => settings.github_token.clone(),
```

### Layer 4: Debug impl redaction

**`crates/mika-common/src/config.rs`** — Add to manual `Debug` impl (~line 985):

```rust
.field("github_token", &self.github_token.as_ref().map(|_| "[REDACTED]"))
```

### Layer 5: Token resolution at construction sites

Create a helper method on `Settings` to centralize fallback logic:

```rust
/// Resolve the GitHub token for agent operations.
/// Prefers `MIKA_GITHUB_TOKEN`, falls back to `MIKA_INVESTIGATE_GITHUB_TOKEN`.
pub fn agent_github_token(&self) -> Option<&str> {
    self.github_token.as_deref().or(self.investigate_github_token.as_deref())
}
```

Then update all construction sites to use `settings.agent_github_token()`:

| File | Line | Current code | Change to |
|------|------|-------------|-----------|
| `crates/mika-cli/src/commands/chat.rs` | ~114 | `settings.investigate_github_token.clone()` | `settings.agent_github_token().map(String::from)` |
| `crates/mika-cli/src/commands/ask.rs` | ~197 | `settings.investigate_github_token.as_deref()` | `settings.agent_github_token()` |
| `crates/mika-agent/src/server/mod.rs` | ~452, ~472 | `settings.investigate_github_token.clone()` | `settings.agent_github_token().map(String::from)` |
| `crates/mika-agent/src/server/mod.rs` | ~531 | `settings.investigate_github_token.clone()` | `settings.agent_github_token().map(String::from)` |
| `crates/mika-agent/src/teams/engine.rs` | ~177, ~218 | `settings.investigate_github_token.clone()` | `settings.agent_github_token().map(String::from)` |
| `crates/mika-agent/src/tools/delegate_task.rs` | ~274 | `self.settings.investigate_github_token.as_deref()` | `self.settings.agent_github_token()` |
| `crates/mika-agent/src/server/dashboard_dev_runs.rs` | ~156 | `settings.investigate_github_token` | `settings.agent_github_token()` (+ update error message) |

**NOT changed:** `crates/mika-agent/src/server/investigate.rs` (~line 1037) — this must continue reading `investigate_github_token` directly (investigation panel is scoped to its own token).

### Layer 6: Test fixtures

Update `Settings` struct literals in test code to include `github_token: None`:

- `crates/mika-agent/src/test_utils.rs` (~lines 46, 113, 134, 224)
- `crates/mika-agent/src/server/mod.rs` (test `Settings` construction ~lines 668, 717, 766)

### Layer 7: Handler script unset lists

**`crates/mika-agent/templates/skills/shell-exec/handlers/run.sh`** (~line 13):

Add `MIKA_GITHUB_TOKEN` to the explicit `unset` list. Note: the executor-level `scrub_mika_env_vars()` already strips all `MIKA_*` vars via prefix matching — this is defense-in-depth.

### Layer 8: Documentation

- **`.env.example`** — Add `MIKA_GITHUB_TOKEN` entry above `MIKA_INVESTIGATE_GITHUB_TOKEN` with clear scope description
- **`CLAUDE.md`** — Add `MIKA_GITHUB_TOKEN` to the Environment Variables section
- **`docs/` config docs** — Update if any reference the investigation token's scope

### Layer 9: CLI doctor + setup

- **`crates/mika-cli/src/commands/doctor.rs`** (~line 66) — Add `MIKA_GITHUB_TOKEN` check as the primary GitHub token diagnostic
- **`crates/mika-cli/src/commands/setup.rs`** (~lines 140-145, 451) — Add `MIKA_GITHUB_TOKEN` prompt in interactive and compose modes

## Out of Scope

- **`run_gh` token injection** — Pre-existing gap where `run_gh` builtin doesn't pass `github_token` from ToolContext. Separate issue.
- **SecretString conversion** — TODO #646 tracks converting `investigate_github_token` to `SecretString`. Apply same pattern to `github_token` in that follow-up.
- **Helm chart update** — `mika-cloud` needs a companion issue for injecting `MIKA_GITHUB_TOKEN` into agent container env.

## Sources

- Issue: [#289](https://github.com/senara-solutions/mika/issues/289)
- Pattern: `docs/solutions/architecture-patterns/config-key-rename-across-layers.md` — 9-layer checklist for env var changes
- Pattern: `docs/solutions/architecture-patterns/unified-llm-api-key-consolidation.md` — proven env var addition pattern
- Security: `docs/solutions/security-issues/env-var-leakage-exec-handler-child-processes.md` — handler script unset requirements
