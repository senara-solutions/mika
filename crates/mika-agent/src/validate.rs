//! Validation for agent and team configurations.
//!
//! Produces `SkillDiagnostic` findings (reuses the same type from skill validation)
//! with `[OK]`, `[WARN]`, `[FAIL]` badges.

use std::path::Path;

use mika_common::agent;
use mika_common::config::Settings;
use mika_common::home;
use mika_common::llm::ProviderKind;
use mika_common::team;

use crate::mcp::config::McpConfig;
use crate::skills::index::{SkillDiagnostic, scan_skills_dir};

/// Validate a single agent's configuration and return diagnostics.
pub fn validate_agent(global_home: &Path, agent_name: &str) -> Vec<SkillDiagnostic> {
    let mut diags = Vec::new();
    let agent_home = home::resolve_agent_home(global_home, agent_name);

    // 1. Config loads
    let settings = match Settings::load_for_agent(global_home, &agent_home) {
        Ok(s) => {
            let active = s.active_llm_config();
            diags.push(SkillDiagnostic::ok(format!(
                "config loaded — provider={}, model={}",
                active.provider, active.model,
            )));
            s
        }
        Err(e) => {
            diags.push(SkillDiagnostic::fail(format!("config failed to load: {e}")));
            return diags;
        }
    };

    let active_provider = settings.llm_provider;

    // 2. Provider/model pairing
    let (model, _, _) = settings.provider_fields(active_provider);
    if model.filter(|m| !m.trim().is_empty()).is_some() {
        diags.push(SkillDiagnostic::ok(format!(
            "{}_model explicitly set",
            active_provider.config_prefix(),
        )));
    } else {
        diags.push(SkillDiagnostic::warn(format!(
            "{}_model not set — using default \"{}\"",
            active_provider.config_prefix(),
            active_provider.default_model(),
        )));
    }

    // 3. Stale model fields
    for &pk in ProviderKind::ALL {
        if pk == active_provider {
            continue;
        }
        let (model, _, _) = settings.provider_fields(pk);
        if let Some(m) = model.filter(|m| !m.trim().is_empty()) {
            diags.push(SkillDiagnostic::warn(format!(
                "stale model field: {}_model=\"{m}\" (active provider is {active_provider})",
                pk.config_prefix(),
            )));
        }
    }

    // 4. max_tokens range
    let max_tokens = settings.llm_max_tokens;
    if max_tokens == 0 {
        diags.push(SkillDiagnostic::fail("llm_max_tokens is 0".to_string()));
    } else if max_tokens > 32768 {
        diags.push(SkillDiagnostic::warn(format!(
            "llm_max_tokens={max_tokens} — unusually high, may exceed provider limits"
        )));
    } else {
        diags.push(SkillDiagnostic::ok(format!("llm_max_tokens={max_tokens}")));
    }

    // 5. API key presence
    if active_provider == ProviderKind::Ollama {
        diags.push(SkillDiagnostic::ok(
            "API key not required (ollama)".to_string(),
        ));
    } else {
        let (_, api_key, _) = settings.provider_fields(active_provider);
        let env_var = format!(
            "MIKA_{}_API_KEY",
            active_provider.config_prefix().to_uppercase(),
        );
        if api_key.filter(|k| !k.trim().is_empty()).is_some() {
            diags.push(SkillDiagnostic::ok(format!("API key present ({env_var})")));
        } else {
            diags.push(SkillDiagnostic::fail(format!(
                "API key missing — set {env_var}"
            )));
        }
    }

    // 6. soul.md exists
    let soul_path = agent_home.join("soul.md");
    if soul_path.exists() {
        diags.push(SkillDiagnostic::ok("soul.md found".to_string()));
    } else {
        diags.push(SkillDiagnostic::warn("soul.md not found".to_string()));
    }

    // 7. MCP config
    let mcp_path = agent_home.join("mcp.json");
    if mcp_path.exists() {
        match McpConfig::load(&agent_home) {
            Ok(config) => {
                let mut mcp_ok = true;
                for (name, server) in &config.mcp_servers {
                    if let Err(e) = server.validate(name) {
                        diags.push(SkillDiagnostic::fail(format!("mcp server \"{name}\": {e}")));
                        mcp_ok = false;
                    }
                }
                if mcp_ok {
                    let count = config.mcp_servers.len();
                    diags.push(SkillDiagnostic::ok(format!(
                        "mcp.json valid — {count} server(s)"
                    )));
                }
            }
            Err(e) => {
                diags.push(SkillDiagnostic::fail(format!(
                    "mcp.json failed to parse: {e}"
                )));
            }
        }
    }

    // 8. Skill LLM overrides (from DB via apply_overrides — no longer from skill.toml #504)
    let skills_dir = agent_home.join("skills");
    if skills_dir.is_dir() {
        let scan = scan_skills_dir(&skills_dir);
        for entry in &scan.entries {
            if entry.manifest.skill.always_on
                && let Some(ref provider_str) = entry.manifest.llm.provider
                && let Ok(skill_provider) = provider_str.parse::<ProviderKind>()
                && skill_provider != active_provider
            {
                diags.push(SkillDiagnostic::warn(format!(
                    "always-on skill \"{}\" overrides provider to {} \
                     (agent uses {active_provider})",
                    entry.manifest.skill.name, skill_provider,
                )));
            }
        }
    }

    diags
}

/// Validate a single team's configuration and return diagnostics.
pub fn validate_team_config(global_home: &Path, team_name: &str) -> Vec<SkillDiagnostic> {
    let mut diags = Vec::new();

    // 1. team.toml loads
    let def = match team::load_team(global_home, team_name) {
        Ok(d) => {
            diags.push(SkillDiagnostic::ok(format!(
                "team.toml valid — orchestrator=\"{}\"",
                d.team.orchestrator,
            )));
            d
        }
        Err(e) => {
            diags.push(SkillDiagnostic::fail(format!(
                "team.toml failed to load: {e}"
            )));
            return diags;
        }
    };

    // 2. Name matches directory
    if def.team.name != team_name {
        diags.push(SkillDiagnostic::warn(format!(
            "team.name=\"{}\" does not match directory name \"{team_name}\"",
            def.team.name,
        )));
    }

    // 3. Orchestrator exists
    if !agent::agent_exists(global_home, &def.team.orchestrator) {
        diags.push(SkillDiagnostic::fail(format!(
            "orchestrator agent \"{}\" does not exist",
            def.team.orchestrator,
        )));
    } else {
        diags.push(SkillDiagnostic::ok(format!(
            "orchestrator \"{}\" exists",
            def.team.orchestrator,
        )));
    }

    // 4. Orchestrator in agents list
    if !def.agents.iter().any(|a| a.name == def.team.orchestrator) {
        diags.push(SkillDiagnostic::fail(format!(
            "orchestrator \"{}\" not listed in [[agents]]",
            def.team.orchestrator,
        )));
    }

    // 5. All agents exist
    for ta in &def.agents {
        if !agent::agent_exists(global_home, &ta.name) {
            diags.push(SkillDiagnostic::fail(format!(
                "agent \"{}\" does not exist",
                ta.name,
            )));
        } else if ta.name != def.team.orchestrator {
            // Only emit OK for non-orchestrator agents (orchestrator already checked above)
            diags.push(SkillDiagnostic::ok(
                format!("agent \"{}\" exists", ta.name,),
            ));
        }
    }

    // 6. max_iterations range
    let max_iter = def.flow.max_iterations;
    if max_iter == 0 {
        diags.push(SkillDiagnostic::warn(
            "flow.max_iterations is 0 — team will not iterate".to_string(),
        ));
    } else if max_iter > 20 {
        diags.push(SkillDiagnostic::warn(format!(
            "flow.max_iterations={max_iter} — unusually high"
        )));
    } else {
        diags.push(SkillDiagnostic::ok(format!(
            "flow.max_iterations={max_iter}"
        )));
    }

    diags
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::skills::index::DiagnosticLevel;
    use std::fs;
    use tempfile::TempDir;

    fn setup_agent(dir: &Path, name: &str) {
        let agents_dir = dir.join("agents").join(name);
        fs::create_dir_all(&agents_dir).unwrap();
        fs::write(
            agents_dir.join("config.toml"),
            "llm_provider = \"anthropic\"\nllm_max_tokens = 4096\n",
        )
        .unwrap();
    }

    #[test]
    fn test_validate_agent_missing_config() {
        let tmp = TempDir::new().unwrap();
        let agents_dir = tmp.path().join("agents").join("broken");
        fs::create_dir_all(&agents_dir).unwrap();
        // No config.toml — should fail to load

        let diags = validate_agent(tmp.path(), "broken");
        assert!(diags.iter().any(|d| d.level == DiagnosticLevel::Fail));
    }

    #[test]
    fn test_validate_agent_basic_ok() {
        let tmp = TempDir::new().unwrap();
        setup_agent(tmp.path(), "test-agent");

        // Write soul.md
        let agent_home = tmp.path().join("agents").join("test-agent");
        fs::write(agent_home.join("soul.md"), "# Soul").unwrap();

        // Set a fake API key via env for the test
        // Since Settings uses config-rs with MIKA_ prefix, we can't easily inject
        // env vars in tests without side effects. The config will load without
        // an API key — we just check that config loading itself succeeds.
        let diags = validate_agent(tmp.path(), "test-agent");

        // Config should load successfully
        assert!(
            diags
                .iter()
                .any(|d| d.level == DiagnosticLevel::Ok && d.message.contains("config loaded"))
        );
        // soul.md should be found
        assert!(
            diags
                .iter()
                .any(|d| d.level == DiagnosticLevel::Ok && d.message.contains("soul.md found"))
        );
    }

    #[test]
    fn test_validate_agent_missing_soul() {
        let tmp = TempDir::new().unwrap();
        setup_agent(tmp.path(), "test-agent");

        let diags = validate_agent(tmp.path(), "test-agent");
        assert!(
            diags.iter().any(
                |d| d.level == DiagnosticLevel::Warn && d.message.contains("soul.md not found")
            )
        );
    }

    #[test]
    fn test_validate_agent_bad_mcp() {
        let tmp = TempDir::new().unwrap();
        setup_agent(tmp.path(), "test-agent");

        let agent_home = tmp.path().join("agents").join("test-agent");
        fs::write(agent_home.join("mcp.json"), "{ invalid json").unwrap();

        let diags = validate_agent(tmp.path(), "test-agent");
        assert!(
            diags.iter().any(|d| d.level == DiagnosticLevel::Fail
                && d.message.contains("mcp.json failed to parse"))
        );
    }

    #[test]
    fn test_validate_agent_max_tokens_zero() {
        let tmp = TempDir::new().unwrap();
        let agents_dir = tmp.path().join("agents").join("test-agent");
        fs::create_dir_all(&agents_dir).unwrap();
        fs::write(
            agents_dir.join("config.toml"),
            "llm_provider = \"anthropic\"\nllm_max_tokens = 0\n",
        )
        .unwrap();

        let diags = validate_agent(tmp.path(), "test-agent");
        assert!(
            diags
                .iter()
                .any(|d| d.level == DiagnosticLevel::Fail
                    && d.message.contains("llm_max_tokens is 0"))
        );
    }

    #[test]
    fn test_validate_team_missing_toml() {
        let tmp = TempDir::new().unwrap();
        let teams_dir = tmp.path().join("teams").join("broken");
        fs::create_dir_all(&teams_dir).unwrap();
        // No team.toml

        let diags = validate_team_config(tmp.path(), "broken");
        assert!(
            diags.iter().any(|d| d.level == DiagnosticLevel::Fail
                && d.message.contains("team.toml failed to load"))
        );
    }

    #[test]
    fn test_validate_team_missing_agents() {
        let tmp = TempDir::new().unwrap();
        // Create team config referencing non-existent agents
        let teams_dir = tmp.path().join("teams").join("test-team");
        fs::create_dir_all(&teams_dir).unwrap();
        fs::write(
            teams_dir.join("team.toml"),
            r#"
[team]
name = "test-team"
orchestrator = "ghost"

[[agents]]
name = "ghost"
role = "lead"
mandate = "Does not exist"

[flow]
max_iterations = 3
"#,
        )
        .unwrap();

        // Need agents dir to exist for agent_exists check
        fs::create_dir_all(tmp.path().join("agents")).unwrap();

        let diags = validate_team_config(tmp.path(), "test-team");
        assert!(
            diags
                .iter()
                .any(|d| d.level == DiagnosticLevel::Fail && d.message.contains("does not exist"))
        );
    }

    #[test]
    fn test_validate_team_name_mismatch() {
        let tmp = TempDir::new().unwrap();
        // Create team with mismatched name
        let teams_dir = tmp.path().join("teams").join("my-team");
        fs::create_dir_all(&teams_dir).unwrap();

        // Create the agent so it passes existence checks
        setup_agent(tmp.path(), "mika");

        fs::write(
            teams_dir.join("team.toml"),
            r#"
[team]
name = "wrong-name"
orchestrator = "mika"

[[agents]]
name = "mika"
role = "lead"
mandate = "Test"

[flow]
max_iterations = 3
"#,
        )
        .unwrap();

        let diags = validate_team_config(tmp.path(), "my-team");
        assert!(
            diags.iter().any(|d| d.level == DiagnosticLevel::Warn
                && d.message.contains("does not match directory"))
        );
    }
}
