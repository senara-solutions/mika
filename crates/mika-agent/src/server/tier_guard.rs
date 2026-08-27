//! Boot-time assertion that on-disk family provisioning matches the process
//! tier env (mika#1962).
//!
//! # Why
//!
//! `MIKA_AGENT_TIER` selects the persona, the skill allowlist, and the
//! dispatch semantics a being runs under. It is read from the process
//! environment, but the provisioning it selected is written to disk *once*, at
//! first bootstrap, and never rewritten (`write_default_if_missing`). Those two
//! authorities can drift apart.
//!
//! When they do, the drift is silent and one-directional: a container
//! bootstrapped as `family` — persona scrubbed of the operator, allowlist
//! narrowed to daily-life surfaces — that restarts without `MIKA_AGENT_TIER`
//! set falls through to `AgentTier::Default`. Nothing fails. The on-disk
//! persona still says family; the runtime tier now says operator. Real vectors
//! are ordinary operations, not exotic ones: a K8s ConfigMap edit, a Helm
//! value change, a systemd drop-in change, a `docker exec` into a running
//! container, a manual restart from a shell that never had the var.
//!
//! There is no telemetry for this today and no alarm. It surfaces as a user
//! complaint — which is exactly how the founding incident (mika#1783) was
//! found. This guard converts that silent policy inversion into a loud,
//! named startup failure.
//!
//! # Shape
//!
//! Two independent detection axes, OR-combined, both in `mika-common`:
//! [`soul_has_family_marker`] reads the `FAMILY_SOUL_MARKER` sentinel, and
//! [`identity_allowlist_matches_family`] set-compares the skill allowlist. Two
//! axes because neither is sufficient alone — the sentinel is not retroactive
//! (agents provisioned before mika#1962 do not carry it, so the allowlist is
//! the load-bearing detector for the installed base), and the allowlist can be
//! legitimately edited by an operator. Both would have to be broken to hide
//! family provisioning.
//!
//! Fail-fast is deliberate: a single misconfigured agent refuses startup for
//! the whole process. That is the intended trade. Silent policy inversion is a
//! worse failure than a hard stop carrying the agent's name and the fix.

use std::path::Path;

use anyhow::{Result, bail};
use mika_common::home::{
    AgentTier, identity_allowlist_matches_family, resolve_agent_home, soul_has_family_marker,
};

/// Assert that no agent is family-provisioned on disk while the process runs
/// under a non-family tier.
///
/// `tier` is passed in rather than read from the environment so the guard is
/// a pure function of (disk state, tier) — callers supply
/// `AgentTier::from_env()`, tests supply the tier directly, and no test has to
/// mutate process-global env state to exercise the guard.
///
/// Must run *after* `home::migrate_to_multi_agent`: that call is what
/// establishes `{home_dir}/agents/<name>/`, and scanning a pre-migration
/// layout would find no agents at all — a false negative in the exact
/// direction this guard exists to prevent.
///
/// Errors when an agent's `soul.md` or `identity.toml` exists but cannot be
/// read or parsed. A file that is merely absent is not an error (see the
/// helpers' own docs) — an agent directory mid-bootstrap legitimately has
/// neither.
pub fn assert_family_tier_env_consistency(home_dir: &Path, tier: AgentTier) -> Result<()> {
    if tier == AgentTier::Family {
        // Env says family. Any family provisioning on disk agrees with it, and
        // an operator-provisioned agent under a family env is the reverse
        // direction — out of scope here (see the plan's Out of scope: there is
        // no operator-side provisioning sentinel to detect against).
        return Ok(());
    }

    let mut mismatched: Vec<(String, &'static str)> = Vec::new();

    for agent_name in mika_common::agent::list_agents(home_dir) {
        let agent_home = resolve_agent_home(home_dir, &agent_name);

        let evidence = if soul_has_family_marker(&agent_home)? {
            Some("soul.md carries the family provisioning marker")
        } else if identity_allowlist_matches_family(&agent_home)? {
            Some("identity.toml carries the family-tier skill allowlist")
        } else {
            None
        };

        if let Some(evidence) = evidence {
            mismatched.push((agent_name, evidence));
        }
    }

    if mismatched.is_empty() {
        return Ok(());
    }

    let observed = std::env::var("MIKA_AGENT_TIER").unwrap_or_else(|_| "<unset>".to_string());
    let detail = mismatched
        .iter()
        .map(|(name, evidence)| format!("  - {name}: {evidence}"))
        .collect::<Vec<_>>()
        .join("\n");

    bail!(
        "family-tier provisioning drift detected — refusing to start.\n\n\
         These agents are provisioned as family tier on disk:\n{detail}\n\n\
         But MIKA_AGENT_TIER={observed} resolves to {tier:?} tier. Starting \
         anyway would silently run a family being under operator semantics: \
         the persona and skill allowlist on disk stay family, while dispatch \
         and diagnostics behave as operator. That leak is invisible until a \
         user reports it (see mika#1783).\n\n\
         Fix: set MIKA_AGENT_TIER=family in the process environment BEFORE \
         mika-spirit starts — the service EnvironmentFile, the K8s ConfigMap, \
         or the systemd drop-in, not an interactive shell. If instead these \
         agents should genuinely be operator tier, re-provision them; the tier \
         is fixed at bootstrap and is not hot-swappable."
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use mika_common::home::{DEFAULT_IDENTITY, DEFAULT_SOUL, FAMILY_IDENTITY, FAMILY_SOUL};
    use std::fs;
    use tempfile::TempDir;

    /// Build a multi-agent home with one agent provisioned from the given
    /// identity/soul templates.
    fn home_with_agent(name: &str, identity: &str, soul: &str) -> TempDir {
        let tmp = tempfile::tempdir().unwrap();
        let agent_dir = tmp.path().join("agents").join(name);
        fs::create_dir_all(&agent_dir).unwrap();
        // `list_agents` only counts directories carrying a config.toml.
        fs::write(agent_dir.join("config.toml"), "log_level = \"info\"\n").unwrap();
        fs::write(agent_dir.join("identity.toml"), identity).unwrap();
        fs::write(agent_dir.join("soul.md"), soul).unwrap();
        tmp
    }

    /// The founding case: family on disk, env says operator → refuse startup.
    #[test]
    fn refuses_start_when_family_provisioned_agent_runs_under_default_tier() {
        let home = home_with_agent("mika", FAMILY_IDENTITY, FAMILY_SOUL);
        let err = assert_family_tier_env_consistency(home.path(), AgentTier::Default).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("mika"),
            "error must name the offending agent, got: {msg}"
        );
        assert!(
            msg.contains("MIKA_AGENT_TIER"),
            "error must name the env var to set, got: {msg}"
        );
    }

    /// Same disk state, matching env → clean start.
    #[test]
    fn starts_clean_when_family_provisioning_matches_family_tier() {
        let home = home_with_agent("mika", FAMILY_IDENTITY, FAMILY_SOUL);
        assert_family_tier_env_consistency(home.path(), AgentTier::Family).unwrap();
    }

    /// An operator-provisioned agent under operator tier is the ordinary case.
    #[test]
    fn starts_clean_for_operator_provisioning_under_default_tier() {
        let home = home_with_agent("mika", DEFAULT_IDENTITY, DEFAULT_SOUL);
        assert_family_tier_env_consistency(home.path(), AgentTier::Default).unwrap();
    }

    /// Axis 2 alone must fire. This is the installed base: every family agent
    /// bootstrapped before mika#1962 has a marker-less soul, so the allowlist
    /// is the only thing that can detect it.
    #[test]
    fn detects_pre_marker_family_agent_via_identity_allowlist_alone() {
        // A family soul as it was written before the marker existed.
        let legacy_family_soul = FAMILY_SOUL
            .strip_prefix(mika_common::home::FAMILY_SOUL_MARKER)
            .expect("FAMILY_SOUL must start with the marker")
            .trim_start_matches('\n');
        let home = home_with_agent("mika", FAMILY_IDENTITY, legacy_family_soul);

        // Axis 1 is blind to this agent...
        let agent_home = resolve_agent_home(home.path(), "mika");
        assert!(!soul_has_family_marker(&agent_home).unwrap());

        // ...and axis 2 is what catches it.
        let err = assert_family_tier_env_consistency(home.path(), AgentTier::Default).unwrap_err();
        assert!(err.to_string().contains("identity.toml"));
    }

    /// Axis 1 alone must fire — an operator who narrowed a family agent's
    /// allowlist by hand has not thereby made it an operator agent.
    #[test]
    fn detects_family_agent_via_soul_marker_alone() {
        let home = home_with_agent("mika", DEFAULT_IDENTITY, FAMILY_SOUL);
        let err = assert_family_tier_env_consistency(home.path(), AgentTier::Default).unwrap_err();
        assert!(err.to_string().contains("soul.md"));
    }

    /// Every offending agent is named, not just the first — an operator fixing
    /// this should not have to restart once per drifted agent.
    #[test]
    fn names_every_drifted_agent_not_just_the_first() {
        let tmp = tempfile::tempdir().unwrap();
        for name in ["al", "mika", "teddy"] {
            let agent_dir = tmp.path().join("agents").join(name);
            fs::create_dir_all(&agent_dir).unwrap();
            fs::write(agent_dir.join("config.toml"), "log_level = \"info\"\n").unwrap();
            fs::write(agent_dir.join("identity.toml"), FAMILY_IDENTITY).unwrap();
            fs::write(agent_dir.join("soul.md"), FAMILY_SOUL).unwrap();
        }
        let err = assert_family_tier_env_consistency(tmp.path(), AgentTier::Default).unwrap_err();
        let msg = err.to_string();
        for name in ["al", "mika", "teddy"] {
            assert!(msg.contains(name), "error must name {name}, got: {msg}");
        }
    }

    /// A fresh install has no agents yet. The guard must not refuse startup
    /// before bootstrap has had a chance to run.
    #[test]
    fn no_agents_is_a_clean_start() {
        let tmp = tempfile::tempdir().unwrap();
        assert_family_tier_env_consistency(tmp.path(), AgentTier::Default).unwrap();
    }

    /// A half-bootstrapped agent directory (config.toml written, persona files
    /// not yet) must not be fatal — absence is not drift.
    #[test]
    fn agent_without_persona_files_is_a_clean_start() {
        let tmp = tempfile::tempdir().unwrap();
        let agent_dir = tmp.path().join("agents").join("mika");
        fs::create_dir_all(&agent_dir).unwrap();
        fs::write(agent_dir.join("config.toml"), "log_level = \"info\"\n").unwrap();
        assert_family_tier_env_consistency(tmp.path(), AgentTier::Default).unwrap();
    }

    /// A present-but-malformed identity.toml must stop startup rather than
    /// read as "not family" — that silent `false` is how a corrupted family
    /// agent would slip past the guard.
    #[test]
    fn malformed_identity_stops_startup() {
        let home = home_with_agent("mika", "name = \"Mika\"\n[skills\n", DEFAULT_SOUL);
        let err = assert_family_tier_env_consistency(home.path(), AgentTier::Default).unwrap_err();
        assert!(err.to_string().contains("identity.toml"));
    }
}
