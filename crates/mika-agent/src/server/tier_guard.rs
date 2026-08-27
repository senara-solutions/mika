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

    for agent_name in servable_agent_names(home_dir) {
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

    // Derived from the `tier` argument, never re-read from the environment:
    // the guard is a pure function of (disk state, tier), and a message that
    // printed a live env var while the caller passed a different tier would
    // contradict itself and point the operator at the wrong fix.
    let observed = match std::env::var("MIKA_AGENT_TIER") {
        Ok(raw) if AgentTier::from_env() == tier => format!("MIKA_AGENT_TIER={raw}"),
        Err(_) if tier == AgentTier::Default => "MIKA_AGENT_TIER=<unset>".to_string(),
        // The process env does not explain the tier we were handed — say so
        // rather than printing a value that disagrees with it.
        _ => format!("the resolved tier is {tier:?}"),
    };
    let detail = mismatched
        .iter()
        .map(|(name, evidence)| format!("  - {name}: {evidence}"))
        .collect::<Vec<_>>()
        .join("\n");

    bail!(
        "family-tier provisioning drift detected — refusing to start.\n\n\
         These agents are provisioned as family tier on disk:\n{detail}\n\n\
         But {observed} resolves to {tier:?} tier. Starting \
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

/// Per-agent form of [`assert_family_tier_env_consistency`], for agents that
/// appear **after** boot.
///
/// The boot guard scans `agents/` once, in `run_server`. But
/// `AppState::resolve_agent`'s slow path (mika#1399) lazy-constructs an
/// `AgentState` at request time for any agent home that has appeared on disk
/// since, and that path re-reads the tier from the server's own environment.
/// Without this check the drift class re-opens in the exact direction mika#1962
/// closes, and by a route that is more reachable than the ones the module doc
/// lists — a running process's environ cannot be mutated by a ConfigMap edit or
/// `docker exec`, but two processes can trivially disagree:
///
/// 1. mika-spirit runs with `MIKA_AGENT_TIER` unset; the boot guard passes.
/// 2. An operator runs `mika agents create <name>` from a shell that DOES
///    export `MIKA_AGENT_TIER=family`. `home::bootstrap` reads that shell's env
///    and writes `FAMILY_IDENTITY` + `FAMILY_SOUL` to disk.
/// 3. The first `/send` for that agent hits `resolve_agent`'s slow path, which
///    resolves the tier from the *server's* env — `Default`.
/// 4. A family-provisioned persona is now served under operator semantics,
///    silently, until someone restarts the process.
///
/// Returns `Err` on mismatch so the caller can refuse to construct the agent.
/// Unlike the boot guard this must not take the process down — a request for
/// one drifted agent is not a reason to stop serving the healthy ones — so the
/// caller declines that agent and logs, rather than propagating.
pub fn check_agent_tier_consistency(
    agent_home: &Path,
    agent_name: &str,
    tier: AgentTier,
) -> Result<()> {
    if tier == AgentTier::Family {
        return Ok(());
    }

    let evidence = if soul_has_family_marker(agent_home)? {
        "soul.md carries the family provisioning marker"
    } else if identity_allowlist_matches_family(agent_home)? {
        "identity.toml carries the family-tier skill allowlist"
    } else {
        return Ok(());
    };

    bail!(
        "agent '{agent_name}' is provisioned as family tier on disk ({evidence}), \
         but this process resolved {tier:?} tier. Refusing to serve it rather \
         than running a family being under operator semantics. This usually \
         means the agent was created from a shell with MIKA_AGENT_TIER=family \
         while mika-spirit itself was started without it — restart mika-spirit \
         with MIKA_AGENT_TIER=family, or re-provision the agent as operator \
         tier. See mika#1962."
    );
}

/// Every agent name this process could end up serving.
///
/// `agent::list_agents` filters on `config.toml`, but the server's own
/// definition of a servable agent is looser: `AppState::resolve_agent` gates
/// only on `identity.toml`, and `Settings::load_for_agent` adds the per-agent
/// `config.toml` with `.required(false)`. An agent home carrying
/// `identity.toml` + `soul.md` but no `config.toml` — a partial restore, a
/// hand-assembled directory, a deleted config, an interrupted `bootstrap` — is
/// therefore fully resolvable and fully servable while being invisible to
/// `list_agents`.
///
/// Two predicates disagreeing about what an agent is would leave the guard on
/// the looser-consequence side of the disagreement, so this takes the union:
/// anything `list_agents` finds, plus any directory under `agents/` carrying an
/// `identity.toml`.
fn servable_agent_names(home_dir: &Path) -> Vec<String> {
    let mut names: std::collections::BTreeSet<String> = mika_common::agent::list_agents(home_dir)
        .into_iter()
        .collect();

    if let Ok(entries) = std::fs::read_dir(home_dir.join("agents")) {
        for entry in entries.flatten() {
            if !entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false) {
                continue;
            }
            if entry.path().join("identity.toml").exists() {
                names.insert(entry.file_name().to_string_lossy().to_string());
            }
        }
    }

    names.into_iter().collect()
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
            .trim_end()
            .strip_suffix(mika_common::home::FAMILY_SOUL_MARKER)
            .expect("FAMILY_SOUL must end with the marker");
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

    /// mika#1962 F3 — an agent home with `identity.toml` but no `config.toml`
    /// is invisible to `agent::list_agents` yet fully servable by
    /// `AppState::resolve_agent`. The guard must see what the server serves.
    #[test]
    fn detects_family_agent_without_config_toml() {
        let tmp = tempfile::tempdir().unwrap();
        let agent_dir = tmp.path().join("agents").join("nadia");
        fs::create_dir_all(&agent_dir).unwrap();
        // Deliberately NO config.toml.
        fs::write(agent_dir.join("identity.toml"), FAMILY_IDENTITY).unwrap();
        fs::write(agent_dir.join("soul.md"), FAMILY_SOUL).unwrap();

        // Precondition: the narrower predicate really does miss it.
        assert!(mika_common::agent::list_agents(tmp.path()).is_empty());

        let err = assert_family_tier_env_consistency(tmp.path(), AgentTier::Default).unwrap_err();
        assert!(err.to_string().contains("nadia"));
    }

    /// Isolate the diagnostic clause ("But <observed> resolves to <tier> tier")
    /// from the remediation clause, which legitimately contains the literal
    /// `MIKA_AGENT_TIER=family` as the fix to apply.
    fn diagnostic_clause(msg: &str) -> String {
        let start = msg
            .find("But ")
            .expect("message must carry the diagnostic clause");
        let rest = &msg[start..];
        let end = rest
            .find(" tier.")
            .expect("diagnostic clause must name the tier");
        rest[..end].to_string()
    }

    /// mika#1962 F5 — when the process env explains the tier we were handed,
    /// the message names the env var and its value.
    #[test]
    #[serial_test::serial]
    fn error_message_names_the_env_when_it_explains_the_tier() {
        // Safety: serialized against every other MIKA_AGENT_TIER test.
        unsafe { std::env::remove_var("MIKA_AGENT_TIER") };
        let home = home_with_agent("mika", FAMILY_IDENTITY, FAMILY_SOUL);
        let err = assert_family_tier_env_consistency(home.path(), AgentTier::Default).unwrap_err();
        let clause = diagnostic_clause(&err.to_string());
        assert!(
            clause.contains("MIKA_AGENT_TIER=<unset>") && clause.contains("Default"),
            "unset env + Default tier agree, so the clause should name both: {clause}"
        );
    }

    /// The case F5 was actually about: the caller passes a tier the process env
    /// does NOT explain. The old message rendered
    /// "MIKA_AGENT_TIER=family ... resolves to Default tier" — self-contradictory,
    /// and it points the operator at a variable that is already set correctly.
    #[test]
    #[serial_test::serial]
    fn error_message_does_not_contradict_a_tier_the_env_disagrees_with() {
        // Safety: serialized against every other MIKA_AGENT_TIER test.
        unsafe { std::env::set_var("MIKA_AGENT_TIER", "family") };
        let home = home_with_agent("mika", FAMILY_IDENTITY, FAMILY_SOUL);
        // Caller passes Default even though the env says family.
        let err = assert_family_tier_env_consistency(home.path(), AgentTier::Default).unwrap_err();
        unsafe { std::env::remove_var("MIKA_AGENT_TIER") };

        let clause = diagnostic_clause(&err.to_string());
        assert!(
            !clause.contains("MIKA_AGENT_TIER=family"),
            "diagnostic clause must not claim the env says family while resolving \
             Default: {clause}"
        );
        assert!(
            clause.contains("Default"),
            "clause must still name the resolved tier: {clause}"
        );
    }

    /// mika#1962 F1 — the per-agent guard used by `resolve_agent`'s lazy path.
    #[test]
    fn per_agent_check_refuses_family_agent_under_default_tier() {
        let home = home_with_agent("nadia", FAMILY_IDENTITY, FAMILY_SOUL);
        let agent_home = resolve_agent_home(home.path(), "nadia");
        let err =
            check_agent_tier_consistency(&agent_home, "nadia", AgentTier::Default).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("nadia"), "must name the agent: {msg}");
        assert!(
            msg.contains("MIKA_AGENT_TIER=family"),
            "must name the remediation: {msg}"
        );
    }

    #[test]
    fn per_agent_check_allows_operator_agent_under_default_tier() {
        let home = home_with_agent("nadia", DEFAULT_IDENTITY, DEFAULT_SOUL);
        let agent_home = resolve_agent_home(home.path(), "nadia");
        check_agent_tier_consistency(&agent_home, "nadia", AgentTier::Default).unwrap();
    }

    #[test]
    fn per_agent_check_allows_family_agent_under_family_tier() {
        let home = home_with_agent("nadia", FAMILY_IDENTITY, FAMILY_SOUL);
        let agent_home = resolve_agent_home(home.path(), "nadia");
        check_agent_tier_consistency(&agent_home, "nadia", AgentTier::Family).unwrap();
    }

    /// mika#1962 F4 — an operator agent with an unrelated typo in its identity
    /// must NOT take the whole process down.
    #[test]
    fn malformed_operator_identity_does_not_stop_startup() {
        let home = home_with_agent(
            "mika",
            "name = \"Mika\"\n[skills\nallowlist = [\"github\", \"shell-exec\"]\n",
            DEFAULT_SOUL,
        );
        assert_family_tier_env_consistency(home.path(), AgentTier::Default).unwrap();
    }

    /// A present-but-malformed identity.toml must stop startup rather than
    /// read as "not family" — that silent `false` is how a corrupted family
    /// agent would slip past the guard.
    #[test]
    fn malformed_identity_stops_startup() {
        let home = home_with_agent(
            "mika",
            "name = \"Mika\"\n[skills\nallowlist = [\"calendar\", \"google-workspace\"]\n",
            DEFAULT_SOUL,
        );
        let err = assert_family_tier_env_consistency(home.path(), AgentTier::Default).unwrap_err();
        assert!(err.to_string().contains("identity.toml"));
    }
}
