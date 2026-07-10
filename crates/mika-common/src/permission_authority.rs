//! Scoped permission-decision authority resolver (mika#1733 AC6).
//!
//! Resolves the effective [`DecisionAuthority`] for a permission decision by
//! walking a three-tier precedence chain: per-agent env → per-tenant env →
//! global [`Settings`]. Compile-time default remains [`DecisionAuthority::Strict`]
//! per AC8.
//!
//! ## Precedence order
//!
//! 1. `MIKA_DECISION_AUTHORITY__AGENT__<agent_id>` (env)
//! 2. `MIKA_DECISION_AUTHORITY__TENANT__<tenant_id>` (env)
//! 3. `Settings.decision_authority` (config file / `MIKA_DECISION_AUTHORITY`)
//! 4. Compile-time default: `DecisionAuthority::Strict`
//!
//! The env-var separator (`__`) matches `config-rs`'s prefix convention so a
//! single `Environment::with_prefix("MIKA").separator("__")` pipeline covers
//! all three tiers if we ever fold this back into `config-rs`; for now the
//! per-scope keys are dynamic and read directly.
//!
//! Agent-id and tenant-id keys are lowercase; hyphens preserved. Consumers
//! MUST normalize before calling.
//!
//! ## Startup validation
//!
//! Invalid enum values fail loud at startup. Call
//! [`validate_env_authority_vars`] over the process environment during
//! `Settings::new()` (or wherever the config pipeline finalizes) to reject
//! misconfiguration before the server accepts requests.

use crate::config::{DecisionAuthority, Settings};

/// Scope of a permission decision — populated by the classifier from the
/// active tenant and agent identifiers at decision time. Either field may be
/// `None` (global-scoped call), which shrinks the resolver chain accordingly.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DecisionScope {
    pub tenant_id: Option<String>,
    pub agent_id: Option<String>,
}

impl DecisionScope {
    pub fn new(tenant_id: Option<String>, agent_id: Option<String>) -> Self {
        Self {
            tenant_id,
            agent_id,
        }
    }

    pub fn global() -> Self {
        Self::default()
    }
}

/// Env-var prefix for scoped authority overrides. Format:
/// `MIKA_DECISION_AUTHORITY__{TIER}__{id}` where `TIER ∈ {AGENT, TENANT}`.
pub const AUTHORITY_ENV_PREFIX: &str = "MIKA_DECISION_AUTHORITY";

fn agent_env_key(agent_id: &str) -> String {
    format!("{AUTHORITY_ENV_PREFIX}__AGENT__{}", agent_id.to_lowercase())
}

fn tenant_env_key(tenant_id: &str) -> String {
    format!(
        "{AUTHORITY_ENV_PREFIX}__TENANT__{}",
        tenant_id.to_lowercase()
    )
}

fn parse_authority(raw: &str) -> Option<DecisionAuthority> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "strict" => Some(DecisionAuthority::Strict),
        "override" => Some(DecisionAuthority::Override),
        _ => None,
    }
}

/// Resolve the effective authority for `scope` using `env_reader` for env-var
/// lookups. `env_reader` is a closure so tests can inject a `HashMap`-backed
/// stub; production callers pass a closure over `std::env::var`.
///
/// Unparseable env values are treated as **absent** at this layer — startup
/// validation is the fail-loud site (see [`validate_env_authority_vars`]).
pub fn resolve_authority<F>(
    settings: &Settings,
    scope: &DecisionScope,
    env_reader: F,
) -> DecisionAuthority
where
    F: Fn(&str) -> Option<String>,
{
    if let Some(agent_id) = scope.agent_id.as_deref()
        && let Some(raw) = env_reader(&agent_env_key(agent_id))
        && let Some(parsed) = parse_authority(&raw)
    {
        return parsed;
    }

    if let Some(tenant_id) = scope.tenant_id.as_deref()
        && let Some(raw) = env_reader(&tenant_env_key(tenant_id))
        && let Some(parsed) = parse_authority(&raw)
    {
        return parsed;
    }

    settings.decision_authority
}

/// Startup validation — scan the process environment for any
/// `MIKA_DECISION_AUTHORITY*` variable and fail loud on unparseable values.
///
/// Runs over an iterator of `(key, value)` pairs so tests can inject stubs
/// and production can pass `std::env::vars()`.
pub fn validate_env_authority_vars<I>(env: I) -> Result<(), String>
where
    I: IntoIterator<Item = (String, String)>,
{
    for (key, value) in env {
        if !key.starts_with(AUTHORITY_ENV_PREFIX) {
            continue;
        }
        // Skip the base env for the global tier — its parse is validated by
        // `Settings::new()` via `serde`.
        if key == AUTHORITY_ENV_PREFIX {
            continue;
        }
        if parse_authority(&value).is_none() {
            return Err(format!(
                "{key}={value:?}: expected 'strict' or 'override' (mika#1733 AC3.2)"
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn env_map(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect()
    }

    fn reader(map: HashMap<String, String>) -> impl Fn(&str) -> Option<String> {
        move |k: &str| map.get(k).cloned()
    }

    fn settings_with(authority: DecisionAuthority) -> Settings {
        let mut s = Settings::test_defaults();
        s.decision_authority = authority;
        s
    }

    #[test]
    fn global_default_falls_through_to_settings() {
        let settings = settings_with(DecisionAuthority::Strict);
        let scope = DecisionScope::global();
        let env = reader(env_map(&[]));
        assert_eq!(
            resolve_authority(&settings, &scope, env),
            DecisionAuthority::Strict
        );
    }

    #[test]
    fn global_override_via_settings() {
        let settings = settings_with(DecisionAuthority::Override);
        let scope = DecisionScope::global();
        let env = reader(env_map(&[]));
        assert_eq!(
            resolve_authority(&settings, &scope, env),
            DecisionAuthority::Override
        );
    }

    #[test]
    fn tenant_env_overrides_settings() {
        let settings = settings_with(DecisionAuthority::Strict);
        let scope = DecisionScope::new(Some("t1".to_string()), None);
        let env = reader(env_map(&[(
            "MIKA_DECISION_AUTHORITY__TENANT__t1",
            "override",
        )]));
        assert_eq!(
            resolve_authority(&settings, &scope, env),
            DecisionAuthority::Override
        );
    }

    #[test]
    fn tenant_env_does_not_bleed_across_tenants() {
        let settings = settings_with(DecisionAuthority::Strict);
        let scope = DecisionScope::new(Some("t2".to_string()), None);
        let env = reader(env_map(&[(
            "MIKA_DECISION_AUTHORITY__TENANT__t1",
            "override",
        )]));
        assert_eq!(
            resolve_authority(&settings, &scope, env),
            DecisionAuthority::Strict,
            "tenant t1 override must NOT affect tenant t2 (AC6 isolation)"
        );
    }

    #[test]
    fn agent_env_wins_over_tenant_env() {
        let settings = settings_with(DecisionAuthority::Strict);
        let scope = DecisionScope::new(Some("t1".to_string()), Some("agentA".to_string()));
        let env = reader(env_map(&[
            ("MIKA_DECISION_AUTHORITY__TENANT__t1", "override"),
            ("MIKA_DECISION_AUTHORITY__AGENT__agenta", "strict"),
        ]));
        assert_eq!(
            resolve_authority(&settings, &scope, env),
            DecisionAuthority::Strict
        );
    }

    #[test]
    fn agent_env_wins_over_settings_when_no_tenant() {
        let settings = settings_with(DecisionAuthority::Strict);
        let scope = DecisionScope::new(None, Some("agentA".to_string()));
        let env = reader(env_map(&[(
            "MIKA_DECISION_AUTHORITY__AGENT__agenta",
            "override",
        )]));
        assert_eq!(
            resolve_authority(&settings, &scope, env),
            DecisionAuthority::Override
        );
    }

    #[test]
    fn agent_id_lowercased_before_lookup() {
        let settings = settings_with(DecisionAuthority::Strict);
        let scope = DecisionScope::new(None, Some("AgentA".to_string()));
        let env = reader(env_map(&[(
            "MIKA_DECISION_AUTHORITY__AGENT__agenta",
            "override",
        )]));
        assert_eq!(
            resolve_authority(&settings, &scope, env),
            DecisionAuthority::Override
        );
    }

    #[test]
    fn hyphens_in_agent_id_preserved() {
        let settings = settings_with(DecisionAuthority::Strict);
        let scope = DecisionScope::new(None, Some("mika-dev".to_string()));
        let env = reader(env_map(&[(
            "MIKA_DECISION_AUTHORITY__AGENT__mika-dev",
            "override",
        )]));
        assert_eq!(
            resolve_authority(&settings, &scope, env),
            DecisionAuthority::Override
        );
    }

    #[test]
    fn unparseable_env_falls_through_to_next_tier() {
        let settings = settings_with(DecisionAuthority::Override);
        let scope = DecisionScope::new(None, Some("agentA".to_string()));
        // Agent env is garbage — resolver treats as absent and falls to settings.
        let env = reader(env_map(&[(
            "MIKA_DECISION_AUTHORITY__AGENT__agenta",
            "loose",
        )]));
        assert_eq!(
            resolve_authority(&settings, &scope, env),
            DecisionAuthority::Override
        );
    }

    // -- validate_env_authority_vars --

    #[test]
    fn validation_accepts_valid_values() {
        let env = vec![
            (
                "MIKA_DECISION_AUTHORITY__AGENT__agenta".to_string(),
                "override".to_string(),
            ),
            (
                "MIKA_DECISION_AUTHORITY__TENANT__t1".to_string(),
                "strict".to_string(),
            ),
            ("OTHER_VAR".to_string(), "irrelevant".to_string()),
        ];
        assert!(validate_env_authority_vars(env).is_ok());
    }

    #[test]
    fn validation_rejects_bad_scoped_value() {
        let env = vec![(
            "MIKA_DECISION_AUTHORITY__AGENT__agenta".to_string(),
            "loose".to_string(),
        )];
        let err = validate_env_authority_vars(env).expect_err("should fail");
        assert!(err.contains("agenta"), "err was: {err}");
        assert!(err.contains("loose"), "err was: {err}");
    }

    #[test]
    fn validation_skips_base_env() {
        // Base MIKA_DECISION_AUTHORITY is validated by serde in Settings::new().
        let env = vec![(
            "MIKA_DECISION_AUTHORITY".to_string(),
            "not-valid-either".to_string(),
        )];
        assert!(validate_env_authority_vars(env).is_ok());
    }

    #[test]
    fn validation_case_insensitive_on_value() {
        let env = vec![(
            "MIKA_DECISION_AUTHORITY__AGENT__agenta".to_string(),
            "OVERRIDE".to_string(),
        )];
        assert!(validate_env_authority_vars(env).is_ok());
    }

    /// AC8 compile-time invariant: the shipped default is [`DecisionAuthority::Strict`].
    /// Structural — nothing outside a review can flip this.
    #[test]
    fn default_authority_is_strict() {
        assert_eq!(DecisionAuthority::default(), DecisionAuthority::Strict);
    }

    /// AC8 grep-discipline check: `override_used = true` may only appear in
    /// tests and comments within this crate (`mika-common`). Full-tree
    /// enforcement lives in the `override_used_true_only_in_tests_and_comments`
    /// integration test at `tests/ac8_grep_discipline.rs`.
    #[test]
    fn override_used_true_absent_from_common_sources() {
        // Walk this crate's `src/` tree; skip our own test module by
        // whitelisting this file.
        use std::fs;
        use std::path::Path;

        fn scan(dir: &Path, offenders: &mut Vec<String>) {
            let Ok(entries) = fs::read_dir(dir) else {
                return;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    scan(&path, offenders);
                } else if path.extension().and_then(|e| e.to_str()) == Some("rs")
                    && !path.ends_with("permission_authority.rs")
                    && let Ok(content) = fs::read_to_string(&path)
                {
                    for (idx, line) in content.lines().enumerate() {
                        let trimmed = line.trim_start();
                        if trimmed.starts_with("//") || trimmed.starts_with("*") {
                            continue;
                        }
                        if trimmed.contains("override_used = true")
                            || trimmed.contains("override_used=true")
                        {
                            offenders.push(format!("{}:{}: {trimmed}", path.display(), idx + 1));
                        }
                    }
                }
            }
        }

        let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut offenders = Vec::new();
        scan(&src, &mut offenders);
        assert!(
            offenders.is_empty(),
            "AC8 grep discipline: `override_used = true` must only appear in \
             tests and comments (mika-common scope). Offenders:\n{}",
            offenders.join("\n")
        );
    }
}
