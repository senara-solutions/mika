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
    /// Optional config.toml content to write on first creation.
    /// When `Some`, overwrites the default config.toml with agent-specific
    /// LLM provider/model settings.
    pub config_toml: Option<&'static str>,
}

/// mika-dev agent specification.
pub static MIKA_DEV: WellKnownAgent = WellKnownAgent {
    name: "mika-dev",
    display_name: "Dev",
    emoji: "🛠",
    soul: MIKA_DEV_SOUL,
    disabled_skills: &["qa-review", "qa-review-build-callback", "skill-review"],
    config_toml: None,
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
    config_toml: None,
};

/// mika-relay agent specification.
///
/// Lightweight agent for handling claude-pilot `can_use_tool` permission
/// relay events. Only the `permission-policy` skill is enabled; all other
/// bundled skills are disabled. Uses haiku for cheap, fast JSON classification.
pub static MIKA_RELAY: WellKnownAgent = WellKnownAgent {
    name: "mika-relay",
    display_name: "Relay",
    emoji: "🔑",
    soul: MIKA_RELAY_SOUL,
    disabled_skills: &[
        // Disable all bundled skills except permission-policy.
        // Engine-coupled (skills/bundled/):
        "self-dev",
        "self-dev-iterate",
        "self-dev-webhook-qa",
        "self-dev-webhook-ci",
        "qa-review",
        "qa-review-build-callback",
        "skill-review",
        "claude-pilot",
        "build-mika",
        "deploy-mika",
        "agents-teams",
        "address-pr-comments",
        "resolve-pr-conflicts",
        "self-check",
        // Community (hardcoded BUNDLED_SKILLS):
        "tmux",
        "shell-exec",
        "web-search",
        "file-reader",
        "self-knowledge",
        "git-ops",
        "google-workspace",
        "github",
        "mcp",
        "browser-control",
    ],
    config_toml: Some(MIKA_RELAY_CONFIG),
};

/// All well-known agents.
pub static WELL_KNOWN_AGENTS: &[&WellKnownAgent] = &[&MIKA_DEV, &MIKA_QA, &MIKA_RELAY];

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
                    "name = \"{}\"\nemoji = \"{}\"\n\n\
                     # [kg]\n\
                     # enabled = true                    # default: true — set false to skip KG for this agent\n\
                     # docs_root = \"/path/to/docs\"       # optional; falls back to MIKA_KG_DOCS_ROOT / kg_docs_root / CWD/docs/solutions\n",
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

                // Overwrite config.toml if the spec provides custom content
                if let Some(config_content) = spec.config_toml
                    && let Err(e) = std::fs::write(agent_home.join("config.toml"), config_content)
                {
                    warn!(
                        agent = spec.name,
                        error = %e,
                        "failed to write config.toml for well-known agent"
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

const MIKA_RELAY_SOUL: &str = r#"# Mika Relay — Permission Relay Agent

## Role
You are Mika Relay, a permission relay agent. Your sole job is handling
claude-pilot `can_use_tool` permission events — classifying tool calls
into allow/deny/answer/escalate tiers and returning structured JSON.

## Communication style
- Respond only with structured JSON as specified by the permission-policy skill
- Never engage in conversation or prose when handling `[claude-pilot]` events
- Be fast and decisive — permission decisions should not require deliberation

## Behaviors
- Apply the permission-policy skill's tier classification strictly
- When in doubt, escalate (Tier 3) rather than allow
- Never initiate workflows, create tasks, or manage development lifecycle
- You exist only to make permission decisions efficiently
"#;

const MIKA_RELAY_CONFIG: &str = r#"# Mika Relay — lightweight permission relay agent.
# Uses haiku for cheap, fast JSON classification of permission events.

llm_provider = "anthropic"
anthropic_model = "claude-haiku-4-5-20251001"
llm_max_tokens = 1024
log_level = "info"
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
        assert!(find_well_known_agent("mika-relay").is_some());
        assert_eq!(
            find_well_known_agent("mika-relay").unwrap().name,
            "mika-relay"
        );
    }

    #[test]
    fn test_find_well_known_agent_not_found() {
        assert!(find_well_known_agent("mika").is_none());
        assert!(find_well_known_agent("custom-agent").is_none());
        assert!(find_well_known_agent("").is_none());
    }

    #[test]
    fn test_provision_creates_all_agents() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        // Create the agents directory
        fs::create_dir_all(home.join("agents")).unwrap();

        provision_well_known_agents(home, false);

        // All well-known agents should exist
        assert!(mika_common::agent::agent_exists(home, "mika-dev"));
        assert!(mika_common::agent::agent_exists(home, "mika-qa"));
        assert!(mika_common::agent::agent_exists(home, "mika-relay"));
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

        let relay_identity = fs::read_to_string(
            mika_common::agent::agent_dir(home, "mika-relay").join("identity.toml"),
        )
        .unwrap();
        assert!(relay_identity.contains("name = \"Relay\""));
        assert!(relay_identity.contains("emoji = \"🔑\""));
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

        let relay_soul =
            fs::read_to_string(mika_common::agent::agent_dir(home, "mika-relay").join("soul.md"))
                .unwrap();
        assert!(relay_soul.contains("Mika Relay"));
        assert!(relay_soul.contains("Permission Relay Agent"));
    }

    #[test]
    fn test_provision_relay_config_toml() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        fs::create_dir_all(home.join("agents")).unwrap();

        provision_well_known_agents(home, false);

        let relay_config = fs::read_to_string(
            mika_common::agent::agent_dir(home, "mika-relay").join("config.toml"),
        )
        .unwrap();
        assert!(relay_config.contains("anthropic_model = \"claude-haiku-4-5-20251001\""));
        assert!(relay_config.contains("llm_max_tokens = 1024"));

        // mika-dev should have the default config.toml (not overwritten)
        let dev_config =
            fs::read_to_string(mika_common::agent::agent_dir(home, "mika-dev").join("config.toml"))
                .unwrap();
        assert!(!dev_config.contains("claude-haiku"));
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
    fn test_seed_skill_overrides_mika_relay() {
        let tmp = tempfile::tempdir().unwrap();
        let db_path = tmp.path().join("test.db");
        let db = Database::open(&db_path).unwrap();
        db.register_agent("mika-relay", "Relay", "/tmp/mika-relay")
            .unwrap();

        seed_well_known_skill_overrides(&db, "mika-relay");

        let overrides = db.get_skill_overrides("mika-relay").unwrap();
        assert_eq!(overrides.len(), MIKA_RELAY.disabled_skills.len());
        for ovr in &overrides {
            assert_eq!(ovr.enabled, Some(false));
            assert!(
                MIKA_RELAY
                    .disabled_skills
                    .contains(&ovr.skill_name.as_str()),
                "unexpected override: {}",
                ovr.skill_name
            );
        }
    }

    #[test]
    fn test_relay_only_allows_permission_policy() {
        // Verify that permission-policy is NOT in the disabled list
        assert!(
            !MIKA_RELAY.disabled_skills.contains(&"permission-policy"),
            "permission-policy must not be disabled for mika-relay"
        );
    }

    #[test]
    fn test_relay_disables_all_bundled_skills_except_permission_policy() {
        // Comprehensive check: every bundled skill except permission-policy
        // must be in mika-relay's disabled list. This catches new skills
        // added to the bundled set without updating mika-relay.
        let all_names = crate::bundled_skills::all_bundled_skill_names();
        for name in &all_names {
            if name.eq_ignore_ascii_case("permission-policy") {
                continue;
            }
            assert!(
                MIKA_RELAY
                    .disabled_skills
                    .iter()
                    .any(|d| d.eq_ignore_ascii_case(name)),
                "bundled skill '{}' is not disabled for mika-relay — \
                 add it to MIKA_RELAY.disabled_skills",
                name
            );
        }
    }

    #[test]
    fn test_relay_config_toml_is_valid_toml() {
        // Verify the config string is valid TOML and contains expected fields
        let config: toml::Table =
            toml::from_str(MIKA_RELAY_CONFIG).expect("MIKA_RELAY_CONFIG must be valid TOML");
        assert_eq!(
            config.get("llm_provider").and_then(|v| v.as_str()),
            Some("anthropic")
        );
        assert_eq!(
            config.get("anthropic_model").and_then(|v| v.as_str()),
            Some("claude-haiku-4-5-20251001")
        );
        assert_eq!(
            config.get("llm_max_tokens").and_then(|v| v.as_integer()),
            Some(1024)
        );
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
    fn test_well_known_agent_specs_dev_qa_no_overlap() {
        // Verify the dev and qa skill lists don't overlap (they have
        // complementary roles — one builds, the other reviews)
        for dev_skill in MIKA_DEV.disabled_skills {
            assert!(
                !MIKA_QA.disabled_skills.contains(dev_skill),
                "skill '{}' is disabled for both mika-dev and mika-qa",
                dev_skill
            );
        }
    }

    #[test]
    fn test_relay_disables_superset_of_dev_and_qa() {
        // mika-relay should disable everything that both dev and qa disable
        // (plus more), since it only needs permission-policy.
        // Exception: permission-policy itself — qa disables it, relay keeps it.
        for dev_skill in MIKA_DEV.disabled_skills {
            assert!(
                MIKA_RELAY.disabled_skills.contains(dev_skill),
                "mika-relay should also disable '{}' (disabled for mika-dev)",
                dev_skill
            );
        }
        for qa_skill in MIKA_QA.disabled_skills {
            if *qa_skill == "permission-policy" {
                continue; // relay's sole purpose is permission-policy
            }
            assert!(
                MIKA_RELAY.disabled_skills.contains(qa_skill),
                "mika-relay should also disable '{}' (disabled for mika-qa)",
                qa_skill
            );
        }
    }
}
