//! Well-known agent provisioning for dev mode.
//!
//! When `dev_mode = true` in config, the startup sequence auto-provisions
//! development agents (`mika-dev`, `mika-qa`) with role-specific identity,
//! soul, and skill assignments.

use std::path::Path;

use tracing::{info, warn};

use crate::db::Database;

/// Specification for a well-known development agent.
pub struct WellKnownAgent {
    /// Agent name (lowercase, hyphenated).
    pub name: &'static str,
    /// Display name for identity.toml.
    pub display_name: &'static str,
    /// Emoji for identity.toml.
    pub emoji: &'static str,
    /// Soul.md content.
    pub soul: &'static str,
    /// Bundled skills to disable for this agent.
    pub disabled_skills: &'static [&'static str],
}

/// mika-dev agent specification.
pub static MIKA_DEV: WellKnownAgent = WellKnownAgent {
    name: "mika-dev",
    display_name: "Dev",
    emoji: "🛠",
    soul: MIKA_DEV_SOUL,
    disabled_skills: &["qa-review", "qa-review-build-callback", "skill-review"],
};

/// mika-qa agent specification.
pub static MIKA_QA: WellKnownAgent = WellKnownAgent {
    name: "mika-qa",
    display_name: "QA",
    emoji: "🔍",
    soul: MIKA_QA_SOUL,
    disabled_skills: &[
        "self-dev",
        "self-dev-iterate",
        "self-dev-webhook-qa",
        "self-dev-webhook-ci",
        "claude-pilot",
        "permission-policy",
        "agents-teams",
        "address-pr-comments",
        "resolve-pr-conflicts",
    ],
};

/// All well-known agents.
pub static WELL_KNOWN_AGENTS: &[&WellKnownAgent] = &[&MIKA_DEV, &MIKA_QA];

/// Look up a well-known agent by name.
pub fn find_well_known_agent(name: &str) -> Option<&'static WellKnownAgent> {
    WELL_KNOWN_AGENTS.iter().find(|a| a.name == name).copied()
}

/// Provision well-known development agents on the filesystem.
///
/// For each well-known agent that doesn't already exist:
/// 1. Calls `bootstrap_agent()` to create dirs + default files
/// 2. Overwrites `identity.toml` and `soul.md` with agent-specific content
///
/// When `disabled` is true, logs a warning and returns without changes.
/// This is the filesystem phase — DB skill overrides are set separately
/// in [`seed_well_known_skill_overrides`] during agent init.
pub fn provision_well_known_agents(home_dir: &Path, disabled: bool) {
    if disabled {
        warn!(
            "agent provisioning disabled by config \
             (MIKA_DISABLE_AGENT_PROVISIONING=true) — well-known agents \
             will not be auto-created or updated"
        );
        return;
    }

    for spec in WELL_KNOWN_AGENTS {
        if mika_common::agent::agent_exists(home_dir, spec.name) {
            info!(
                agent = spec.name,
                "well-known agent already exists, skipping"
            );
            continue;
        }

        match mika_common::home::bootstrap_agent(home_dir, spec.name) {
            Ok(()) => {
                let agent_home = mika_common::agent::agent_dir(home_dir, spec.name);

                // Overwrite identity.toml with agent-specific content
                let identity_content = format!(
                    "name = \"{}\"\nemoji = \"{}\"\n",
                    spec.display_name, spec.emoji
                );
                if let Err(e) = std::fs::write(agent_home.join("identity.toml"), &identity_content)
                {
                    warn!(
                        agent = spec.name,
                        error = %e,
                        "failed to write identity.toml for well-known agent"
                    );
                    continue;
                }

                // Overwrite soul.md with agent-specific content
                if let Err(e) = std::fs::write(agent_home.join("soul.md"), spec.soul) {
                    warn!(
                        agent = spec.name,
                        error = %e,
                        "failed to write soul.md for well-known agent"
                    );
                    continue;
                }

                info!(
                    agent = spec.name,
                    display_name = spec.display_name,
                    "provisioned well-known agent"
                );
            }
            Err(e) => {
                warn!(
                    agent = spec.name,
                    error = %e,
                    "failed to bootstrap well-known agent"
                );
            }
        }
    }
}

/// Seed skill overrides for a well-known agent if none exist yet.
///
/// Writes `set_skill_enabled(false)` for each skill in the agent's
/// `disabled_skills` list. Only runs on first creation — if any
/// `skill_overrides` rows already exist for this agent, the function
/// returns early to preserve user customizations.
pub fn seed_well_known_skill_overrides(db: &Database, agent_name: &str) {
    let spec = match find_well_known_agent(agent_name) {
        Some(s) => s,
        None => return,
    };

    // Check if any overrides already exist (user has customized)
    match db.get_skill_overrides(agent_name) {
        Ok(overrides) if !overrides.is_empty() => {
            return;
        }
        Err(e) => {
            warn!(
                agent = agent_name,
                error = %e,
                "failed to check skill overrides, skipping well-known seeding"
            );
            return;
        }
        _ => {}
    }

    for skill_name in spec.disabled_skills {
        if let Err(e) = db.set_skill_enabled(agent_name, skill_name, false) {
            warn!(
                agent = agent_name,
                skill = skill_name,
                error = %e,
                "failed to disable skill for well-known agent"
            );
        }
    }

    info!(
        agent = agent_name,
        disabled_count = spec.disabled_skills.len(),
        "seeded skill overrides for well-known agent"
    );
}

const MIKA_DEV_SOUL: &str = r#"# Mika Dev — Development Agent

## Role
You are Mika Dev, a senior software development agent. You implement features,
fix bugs, review code, and manage the development lifecycle autonomously.

## Communication style
- Be direct and technical
- Lead with actions taken, then explain reasoning
- Reference specific files, functions, and line numbers
- When stuck, state the blocker clearly

## Behaviors
- Follow existing codebase patterns and conventions
- Write tests for new behavior
- Run the full test suite before declaring work done
- Create focused, well-described PRs
- Never merge your own PRs without QA review
"#;

const MIKA_QA_SOUL: &str = r#"# Mika QA — Quality Assurance Agent

## Role
You are Mika QA, a senior quality assurance agent. You review pull requests,
verify implementations against requirements, and ensure code quality standards.

## Communication style
- Be precise about what passes and what doesn't
- Use structured verdicts (VERDICT: pass/block/hold)
- Reference specific code locations when flagging issues
- Separate blocking issues from suggestions

## Behaviors
- Review PRs for correctness, test coverage, and adherence to conventions
- Verify that CI checks pass before approving
- Check that PR descriptions accurately reflect the changes
- Never approve your own team's work without independent verification
- Flag security, performance, and maintainability concerns
"#;

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_find_well_known_agent_found() {
        assert!(find_well_known_agent("mika-dev").is_some());
        assert_eq!(find_well_known_agent("mika-dev").unwrap().name, "mika-dev");
        assert!(find_well_known_agent("mika-qa").is_some());
        assert_eq!(find_well_known_agent("mika-qa").unwrap().name, "mika-qa");
    }

    #[test]
    fn test_find_well_known_agent_not_found() {
        assert!(find_well_known_agent("mika").is_none());
        assert!(find_well_known_agent("custom-agent").is_none());
        assert!(find_well_known_agent("").is_none());
    }

    #[test]
    fn test_provision_creates_both_agents() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        // Create the agents directory
        fs::create_dir_all(home.join("agents")).unwrap();

        provision_well_known_agents(home, false);

        // Both agents should exist
        assert!(mika_common::agent::agent_exists(home, "mika-dev"));
        assert!(mika_common::agent::agent_exists(home, "mika-qa"));
    }

    #[test]
    fn test_provision_correct_identity() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        fs::create_dir_all(home.join("agents")).unwrap();

        provision_well_known_agents(home, false);

        let dev_identity = fs::read_to_string(
            mika_common::agent::agent_dir(home, "mika-dev").join("identity.toml"),
        )
        .unwrap();
        assert!(dev_identity.contains("name = \"Dev\""));
        assert!(dev_identity.contains("emoji = \"🛠\""));

        let qa_identity = fs::read_to_string(
            mika_common::agent::agent_dir(home, "mika-qa").join("identity.toml"),
        )
        .unwrap();
        assert!(qa_identity.contains("name = \"QA\""));
        assert!(qa_identity.contains("emoji = \"🔍\""));
    }

    #[test]
    fn test_provision_correct_soul() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        fs::create_dir_all(home.join("agents")).unwrap();

        provision_well_known_agents(home, false);

        let dev_soul =
            fs::read_to_string(mika_common::agent::agent_dir(home, "mika-dev").join("soul.md"))
                .unwrap();
        assert!(dev_soul.contains("Mika Dev"));
        assert!(dev_soul.contains("Development Agent"));

        let qa_soul =
            fs::read_to_string(mika_common::agent::agent_dir(home, "mika-qa").join("soul.md"))
                .unwrap();
        assert!(qa_soul.contains("Mika QA"));
        assert!(qa_soul.contains("Quality Assurance"));
    }

    #[test]
    fn test_provision_skips_existing_agent() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        fs::create_dir_all(home.join("agents")).unwrap();

        // Create mika-dev with custom soul
        mika_common::home::bootstrap_agent(home, "mika-dev").unwrap();
        let dev_home = mika_common::agent::agent_dir(home, "mika-dev");
        fs::write(dev_home.join("soul.md"), "custom soul content").unwrap();

        provision_well_known_agents(home, false);

        // mika-dev should keep custom soul
        let dev_soul = fs::read_to_string(dev_home.join("soul.md")).unwrap();
        assert_eq!(dev_soul, "custom soul content");

        // mika-qa should be created fresh
        assert!(mika_common::agent::agent_exists(home, "mika-qa"));
    }

    #[test]
    fn test_provision_partial_state() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        fs::create_dir_all(home.join("agents")).unwrap();

        // Only mika-dev exists
        mika_common::home::bootstrap_agent(home, "mika-dev").unwrap();
        assert!(!mika_common::agent::agent_exists(home, "mika-qa"));

        provision_well_known_agents(home, false);

        // mika-qa should now exist
        assert!(mika_common::agent::agent_exists(home, "mika-qa"));
    }

    #[test]
    fn test_provision_disabled() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        fs::create_dir_all(home.join("agents")).unwrap();

        provision_well_known_agents(home, true);

        // No agents should be created
        assert!(!mika_common::agent::agent_exists(home, "mika-dev"));
        assert!(!mika_common::agent::agent_exists(home, "mika-qa"));
    }

    #[test]
    fn test_seed_skill_overrides_mika_dev() {
        let tmp = tempfile::tempdir().unwrap();
        let db_path = tmp.path().join("test.db");
        let db = Database::open(&db_path).unwrap();
        db.register_agent("mika-dev", "Dev", "/tmp/mika-dev")
            .unwrap();

        seed_well_known_skill_overrides(&db, "mika-dev");

        let overrides = db.get_skill_overrides("mika-dev").unwrap();
        assert_eq!(overrides.len(), MIKA_DEV.disabled_skills.len());
        for ovr in &overrides {
            assert_eq!(ovr.enabled, Some(false));
            assert!(
                MIKA_DEV.disabled_skills.contains(&ovr.skill_name.as_str()),
                "unexpected override: {}",
                ovr.skill_name
            );
        }
    }

    #[test]
    fn test_seed_skill_overrides_mika_qa() {
        let tmp = tempfile::tempdir().unwrap();
        let db_path = tmp.path().join("test.db");
        let db = Database::open(&db_path).unwrap();
        db.register_agent("mika-qa", "QA", "/tmp/mika-qa").unwrap();

        seed_well_known_skill_overrides(&db, "mika-qa");

        let overrides = db.get_skill_overrides("mika-qa").unwrap();
        assert_eq!(overrides.len(), MIKA_QA.disabled_skills.len());
        for ovr in &overrides {
            assert_eq!(ovr.enabled, Some(false));
            assert!(
                MIKA_QA.disabled_skills.contains(&ovr.skill_name.as_str()),
                "unexpected override: {}",
                ovr.skill_name
            );
        }
    }

    #[test]
    fn test_seed_skill_overrides_skips_existing() {
        let tmp = tempfile::tempdir().unwrap();
        let db_path = tmp.path().join("test.db");
        let db = Database::open(&db_path).unwrap();
        db.register_agent("mika-dev", "Dev", "/tmp/mika-dev")
            .unwrap();

        // Set a custom override first
        db.set_skill_enabled("mika-dev", "some-custom-skill", false)
            .unwrap();

        // Now seed — should not add anything since overrides exist
        seed_well_known_skill_overrides(&db, "mika-dev");

        let overrides = db.get_skill_overrides("mika-dev").unwrap();
        // Should only have the one custom override, not the well-known ones
        assert_eq!(overrides.len(), 1);
        assert_eq!(overrides[0].skill_name, "some-custom-skill");
    }

    #[test]
    fn test_seed_skill_overrides_non_well_known() {
        let tmp = tempfile::tempdir().unwrap();
        let db_path = tmp.path().join("test.db");
        let db = Database::open(&db_path).unwrap();
        db.register_agent("custom-agent", "Custom", "/tmp/custom")
            .unwrap();

        seed_well_known_skill_overrides(&db, "custom-agent");

        let overrides = db.get_skill_overrides("custom-agent").unwrap();
        assert!(overrides.is_empty());
    }

    #[test]
    fn test_well_known_agent_specs() {
        // Verify the skill lists don't overlap
        for dev_skill in MIKA_DEV.disabled_skills {
            assert!(
                !MIKA_QA.disabled_skills.contains(dev_skill),
                "skill '{}' is disabled for both mika-dev and mika-qa",
                dev_skill
            );
        }
    }
}
