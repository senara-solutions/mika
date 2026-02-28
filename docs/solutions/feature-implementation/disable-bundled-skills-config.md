---
title: "Add disable_bundled_skills config setting"
category: feature-implementation
date: 2026-02-28
tags: [config, settings, bundled-skills, startup, serde]
severity: low
component: mika-common, mika-agent, mika-cli
root_cause: missing-config-option
resolution: config-field-addition
related_pr: 30
---

# Add `disable_bundled_skills` Config Setting

## Problem

Bundled skills (tmux, shell-exec, web-search, file-reader, calendar, self-knowledge) are re-written from compiled-in templates on every startup via `seed_bundled_skills_if_needed`. This is intentional — it ensures template updates propagate to existing installs. However, it makes it impossible to debug or customize handler scripts at runtime because edits are overwritten on the next launch. Developers needed a way to opt out of startup seeding during development.

## Root Cause

There was no configuration mechanism to skip the bundled skill re-sync. The `seed_bundled_skills_if_needed` function unconditionally seeded skills whenever the skills directory existed.

## Solution

Added `disable_bundled_skills: bool` to `Settings` with `#[serde(default)]` (defaults to `false`). The flag is threaded through all call sites:

### 1. Settings field (`crates/mika-common/src/config.rs`)

```rust
/// Disable bundled skill re-sync on startup (default: false)
#[serde(default)]
pub disable_bundled_skills: bool,
```

Added to manual `Debug` impl (non-redacted, safe to log). Env var: `MIKA_DISABLE_BUNDLED_SKILLS`.

### 2. Startup function (`crates/mika-agent/src/startup.rs`)

```rust
pub fn seed_bundled_skills_if_needed(home_dir: &Path, disabled: bool) {
    if disabled {
        tracing::warn!("bundled skill seeding disabled by config");
        return;
    }
    let skills_dir = home_dir.join("skills");
    if skills_dir.is_dir() {
        crate::bundled_skills::seed_bundled_skills(&skills_dir);
    }
}
```

Uses `warn!` level (not `debug!`) because disabling seeding is security-relevant — it prevents handler script security patches from propagating.

### 3. Call sites

| Location | Passes |
|----------|--------|
| `crates/mika-cli/src/init.rs` | `settings.disable_bundled_skills` |
| `crates/mika-agent/src/server/mod.rs` (`init_agent`) | `disable_bundled_skills` param |
| `crates/mika-cli/src/commands/agents.rs` | Always `false` (explicit creation always seeds) |

### 4. Config documentation

- `config/default.toml`: Commented-out entry with production warning
- `.env.example`: Commented-out env var with warning

```toml
# Disable bundled skill re-sync on startup (useful for debugging handlers)
# WARNING: Do not enable in production -- prevents security updates to handler scripts
# disable_bundled_skills = false
```

## Key Design Decisions

1. **`#[serde(default)]` for backwards compatibility** — Existing config files without the field deserialize correctly (defaults to `false`).

2. **`agents create` always seeds** — Explicit agent creation should never skip seeding, regardless of the config flag. Hardcoded `false` with an explanatory comment at the call site.

3. **`warn!` not `debug!` for disabled path** — Disabling skill seeding is a security-relevant action (prevents handler script patches). Warn-level ensures it appears in default log output so operators notice it.

4. **`init_agent` parameter threading** — The server's `init_agent` function received `disable_bundled_skills` as a parameter rather than accessing settings directly, maintaining the existing pattern where `init_agent` receives explicit values.

## Code Review Findings Addressed

The implementation was reviewed by 7 parallel agents (architecture, security, performance, simplicity, pattern-recognition, agent-native, learnings-researcher). Five findings were created and resolved:

- **#334**: Changed `debug!` to `warn!` for security visibility
- **#335**: Added env var override unit test
- **#336**: Added disabled-path unit tests (skip + seed verification)
- **#337**: Added explanatory comment for hardcoded `false` in `agents create`
- **#338**: Added production warning to `config/default.toml` and `.env.example`

## Testing

Three new tests added (total test count: 579 → 582):

1. `test_seed_bundled_skills_skipped_when_disabled` — Verifies `disabled=true` creates no skill directories
2. `test_seed_bundled_skills_runs_when_enabled` — Verifies `disabled=false` still seeds skills
3. `test_disable_bundled_skills_from_env` — Verifies `MIKA_DISABLE_BUNDLED_SKILLS=true` env var works

Default assertion added to existing `test_defaults`:
```rust
assert!(!settings.disable_bundled_skills);
```

## Prevention Strategies

1. **Always add new Settings fields to `dummy_settings()`** in test helpers (`run_team.rs`). Missing fields cause compile errors in tests.

2. **New bool config fields should use `#[serde(default)]`** to avoid breaking existing config files during upgrades.

3. **Security-relevant config changes should log at `warn!` level** so operators see them in default log output.

4. **Document production warnings inline** in `default.toml` and `.env.example`, not just in external docs. Operators read config files more than docs.

## Files Modified

- `crates/mika-common/src/config.rs` — Field + Debug + tests
- `crates/mika-agent/src/startup.rs` — Parameter + early return + tests
- `crates/mika-cli/src/init.rs` — Pass setting
- `crates/mika-agent/src/server/mod.rs` — Thread param through `init_agent`
- `crates/mika-cli/src/commands/agents.rs` — Hardcode `false` with comment
- `crates/mika-agent/src/tools/run_team.rs` — `dummy_settings()` update
- `config/default.toml` — Documented option with warning
- `.env.example` — Documented env var with warning
- `CLAUDE.md` — Test count, env var docs
- `docs/configuration.md` — Settings table, env var tables
- `docs/skills.md` — Disabling bundled skills subsection

## Related Documentation

- [docs/configuration.md](../../configuration.md) — Full settings reference
- [docs/skills.md](../../skills.md) — Skills system overview
- [docs/solutions/feature-implementation/telegram-image-support.md](telegram-image-support.md) — Similar multi-crate feature addition pattern
