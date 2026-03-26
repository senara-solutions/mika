//! `mika doctor` — diagnostic command to validate installation health.

use anyhow::Result;
use std::path::Path;

use crate::cli::DoctorArgs;

/// Status level for a diagnostic check.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum CheckStatus {
    Ok,
    Warn,
    Fail,
}

/// Result of a single diagnostic check.
#[derive(Debug, Clone, serde::Serialize)]
pub struct CheckResult {
    pub name: String,
    pub status: CheckStatus,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<String>,
}

/// Summary of all checks.
#[derive(Debug, serde::Serialize)]
pub struct DoctorReport {
    pub checks: Vec<CheckResult>,
    pub summary: DoctorSummary,
}

#[derive(Debug, serde::Serialize)]
pub struct DoctorSummary {
    pub ok: usize,
    pub warn: usize,
    pub fail: usize,
}

pub async fn run(args: DoctorArgs, agent_name: &str) -> Result<()> {
    let global_home = mika_common::home::resolve_home_dir()?;
    let agent_home = mika_common::home::resolve_agent_home(&global_home, agent_name);

    let mut checks = vec![
        check_home_dir(&global_home),
        check_agent_dir(&agent_home, agent_name),
        check_api_key(&global_home),
        check_database(&global_home),
        check_env_permissions(&global_home),
        check_config_toml(&agent_home, &global_home),
        check_optional_key(
            "OpenAI API key",
            "MIKA_OPENAI_API_KEY",
            &global_home,
            "vector search disabled",
        ),
        check_optional_key(
            "Brave API key",
            "MIKA_BRAVE_API_KEY",
            &global_home,
            "web search disabled",
        ),
        check_optional_key(
            "GitHub token (agent)",
            "MIKA_GITHUB_TOKEN",
            &global_home,
            "agent GitHub operations use MIKA_INVESTIGATE_GITHUB_TOKEN fallback",
        ),
        check_optional_key(
            "GitHub token (investigation)",
            "MIKA_INVESTIGATE_GITHUB_TOKEN",
            &global_home,
            "dashboard issue creation disabled",
        ),
        check_optional_key(
            "GitHub repo",
            "MIKA_GITHUB_REPO",
            &global_home,
            "dashboard issue creation disabled",
        ),
        check_jq(),
        check_mcp(&agent_home),
        check_skills(&agent_home),
    ];

    if args.verify_api {
        checks.push(check_api_live(&global_home, &agent_home).await);
    }

    let summary = DoctorSummary {
        ok: checks
            .iter()
            .filter(|c| c.status == CheckStatus::Ok)
            .count(),
        warn: checks
            .iter()
            .filter(|c| c.status == CheckStatus::Warn)
            .count(),
        fail: checks
            .iter()
            .filter(|c| c.status == CheckStatus::Fail)
            .count(),
    };

    let has_failures = summary.fail > 0;

    if args.json {
        let report = DoctorReport { checks, summary };
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        println!();
        println!("  Mika Doctor — checking installation health...");
        println!();
        for check in &checks {
            let tag = match check.status {
                CheckStatus::Ok => "\x1b[32m[OK]\x1b[0m  ",
                CheckStatus::Warn => "\x1b[33m[WARN]\x1b[0m",
                CheckStatus::Fail => "\x1b[31m[FAIL]\x1b[0m",
            };
            println!("  {tag} {}", check.message);
        }
        println!();
        println!(
            "  Summary: {} OK, {} warnings, {} failures",
            summary.ok, summary.warn, summary.fail
        );
        println!();
    }

    if has_failures {
        std::process::exit(1);
    }

    Ok(())
}

fn check_home_dir(global_home: &Path) -> CheckResult {
    if !global_home.is_dir() {
        return CheckResult {
            name: "home_directory".into(),
            status: CheckStatus::Fail,
            message: format!("Home directory ({}) — not found", global_home.display()),
            details: Some(global_home.display().to_string()),
        };
    }

    #[cfg(unix)]
    {
        match mika_common::validation::check_path_permissions(global_home) {
            Ok((true, mode)) => {
                if mode == 0o700 {
                    CheckResult {
                        name: "home_directory".into(),
                        status: CheckStatus::Ok,
                        message: format!(
                            "Home directory ({}) — permissions {:04o}",
                            global_home.display(),
                            mode
                        ),
                        details: Some(global_home.display().to_string()),
                    }
                } else {
                    CheckResult {
                        name: "home_directory".into(),
                        status: CheckStatus::Warn,
                        message: format!(
                            "Home directory ({}) — permissions {:04o} (expected 0700)",
                            global_home.display(),
                            mode
                        ),
                        details: Some(global_home.display().to_string()),
                    }
                }
            }
            Ok((false, _)) => CheckResult {
                name: "home_directory".into(),
                status: CheckStatus::Fail,
                message: format!("Home directory ({}) — not found", global_home.display()),
                details: Some(global_home.display().to_string()),
            },
            Err(e) => CheckResult {
                name: "home_directory".into(),
                status: CheckStatus::Fail,
                message: format!("Home directory ({}) — {e}", global_home.display()),
                details: Some(global_home.display().to_string()),
            },
        }
    }

    #[cfg(not(unix))]
    CheckResult {
        name: "home_directory".into(),
        status: CheckStatus::Ok,
        message: format!("Home directory ({}) — exists", global_home.display()),
        details: Some(global_home.display().to_string()),
    }
}

fn check_agent_dir(agent_home: &Path, agent_name: &str) -> CheckResult {
    if !agent_home.is_dir() {
        return CheckResult {
            name: "agent_directory".into(),
            status: CheckStatus::Fail,
            message: format!("Agent directory ({agent_name}) — not found"),
            details: Some(agent_home.display().to_string()),
        };
    }

    let required = ["identity.toml", "soul.md"];
    let mut missing: Vec<&str> = Vec::new();
    for file in &required {
        if !agent_home.join(file).exists() {
            missing.push(file);
        }
    }

    if missing.is_empty() {
        CheckResult {
            name: "agent_directory".into(),
            status: CheckStatus::Ok,
            message: format!(
                "Agent directory ({agent_name}) — {} present",
                required.join(", ")
            ),
            details: Some(agent_home.display().to_string()),
        }
    } else {
        CheckResult {
            name: "agent_directory".into(),
            status: CheckStatus::Fail,
            message: format!(
                "Agent directory ({agent_name}) — missing: {}",
                missing.join(", ")
            ),
            details: Some(agent_home.display().to_string()),
        }
    }
}

fn check_api_key(global_home: &Path) -> CheckResult {
    // Check env var first (highest priority), then .env file
    let (key_value, source) = if let Ok(val) = std::env::var("MIKA_LLM_API_KEY") {
        (Some(val), "env var")
    } else {
        // Try reading from .env file directly (don't call load_dotenv to avoid side effects)
        let env_path = global_home.join(".env");
        let from_file = std::fs::read_to_string(&env_path).ok().and_then(|content| {
            content.lines().find_map(|line| {
                let trimmed = line.trim();
                if trimmed.starts_with('#') || trimmed.is_empty() {
                    return None;
                }
                let (k, v) = trimmed.split_once('=')?;
                if k.trim() == "MIKA_LLM_API_KEY" {
                    Some(v.trim().trim_matches('"').to_string())
                } else {
                    None
                }
            })
        });
        (from_file, ".env")
    };

    match key_value {
        Some(val) if !val.trim().is_empty() => {
            match mika_common::validation::validate_api_key_format(&val) {
                Ok(fmt) => {
                    let fmt_str = match fmt {
                        mika_common::validation::ApiKeyFormat::ApiKey => "Anthropic API key",
                        mika_common::validation::ApiKeyFormat::OAuthToken => "OAuth token",
                        mika_common::validation::ApiKeyFormat::ThirdParty => "third-party key",
                    };
                    CheckResult {
                        name: "api_key".into(),
                        status: CheckStatus::Ok,
                        message: format!("API key — {fmt_str} format valid (source: {source})"),
                        details: None,
                    }
                }
                Err(e) => CheckResult {
                    name: "api_key".into(),
                    status: CheckStatus::Fail,
                    message: format!("API key — {e} (source: {source})"),
                    details: None,
                },
            }
        }
        _ => CheckResult {
            name: "api_key".into(),
            status: CheckStatus::Fail,
            message: "API key — not set (run `mika setup`)".into(),
            details: None,
        },
    }
}

fn check_database(global_home: &Path) -> CheckResult {
    let db_path = mika_common::home::container_db_path(global_home);
    if !db_path.exists() {
        return CheckResult {
            name: "database".into(),
            status: CheckStatus::Fail,
            message: "Database — not found (run `mika setup`)".into(),
            details: Some(db_path.display().to_string()),
        };
    }

    // Try to open the DB and check schema version
    match mika_agent::db::Database::open(&db_path) {
        Ok(db) => {
            let version = db.schema_version().unwrap_or(-1);
            CheckResult {
                name: "database".into(),
                status: CheckStatus::Ok,
                message: format!("Database — schema v{version}, opens successfully"),
                details: Some(db_path.display().to_string()),
            }
        }
        Err(e) => CheckResult {
            name: "database".into(),
            status: CheckStatus::Fail,
            message: format!("Database — failed to open: {e}"),
            details: Some(db_path.display().to_string()),
        },
    }
}

fn check_env_permissions(global_home: &Path) -> CheckResult {
    let env_path = global_home.join(".env");
    if !env_path.exists() {
        return CheckResult {
            name: "env_file".into(),
            status: CheckStatus::Ok,
            message: ".env file — not present (optional)".into(),
            details: None,
        };
    }

    #[cfg(unix)]
    {
        match mika_common::validation::check_path_permissions(&env_path) {
            Ok((true, mode)) => {
                if mode == 0o600 {
                    CheckResult {
                        name: "env_file".into(),
                        status: CheckStatus::Ok,
                        message: format!(".env permissions — {:04o}", mode),
                        details: Some(env_path.display().to_string()),
                    }
                } else {
                    CheckResult {
                        name: "env_file".into(),
                        status: CheckStatus::Warn,
                        message: format!(".env permissions — {:04o} (expected 0600)", mode),
                        details: Some(env_path.display().to_string()),
                    }
                }
            }
            Ok((false, _)) => CheckResult {
                name: "env_file".into(),
                status: CheckStatus::Ok,
                message: ".env file — not present (optional)".into(),
                details: None,
            },
            Err(e) => CheckResult {
                name: "env_file".into(),
                status: CheckStatus::Warn,
                message: format!(".env permissions — {e}"),
                details: None,
            },
        }
    }

    #[cfg(not(unix))]
    CheckResult {
        name: "env_file".into(),
        status: CheckStatus::Ok,
        message: ".env file — present".into(),
        details: Some(env_path.display().to_string()),
    }
}

fn check_config_toml(agent_home: &Path, global_home: &Path) -> CheckResult {
    // Try agent config first, then global
    let config_path = if agent_home.join("config.toml").exists() {
        agent_home.join("config.toml")
    } else if global_home.join("config.toml").exists() {
        global_home.join("config.toml")
    } else {
        return CheckResult {
            name: "config_toml".into(),
            status: CheckStatus::Warn,
            message: "config.toml — not found".into(),
            details: None,
        };
    };

    match std::fs::read_to_string(&config_path) {
        Ok(content) => match content.parse::<toml::Table>() {
            Ok(_) => CheckResult {
                name: "config_toml".into(),
                status: CheckStatus::Ok,
                message: "config.toml — parses successfully".into(),
                details: Some(config_path.display().to_string()),
            },
            Err(e) => CheckResult {
                name: "config_toml".into(),
                status: CheckStatus::Fail,
                message: format!("config.toml — parse error: {e}"),
                details: Some(config_path.display().to_string()),
            },
        },
        Err(e) => CheckResult {
            name: "config_toml".into(),
            status: CheckStatus::Fail,
            message: format!("config.toml — read error: {e}"),
            details: Some(config_path.display().to_string()),
        },
    }
}

fn check_optional_key(
    label: &str,
    env_var: &str,
    global_home: &Path,
    disabled_feature: &str,
) -> CheckResult {
    let name = env_var.to_lowercase().replace("mika_", "");

    // Check env var first
    if let Ok(val) = std::env::var(env_var)
        && !val.trim().is_empty()
    {
        return CheckResult {
            name,
            status: CheckStatus::Ok,
            message: format!("{label} — set (source: env var)"),
            details: None,
        };
    }

    // Check .env file
    let env_path = global_home.join(".env");
    if let Ok(content) = std::fs::read_to_string(&env_path) {
        let found = content.lines().any(|line| {
            let trimmed = line.trim();
            !trimmed.starts_with('#')
                && !trimmed.is_empty()
                && trimmed.split_once('=').is_some_and(|(k, v)| {
                    k.trim() == env_var && !v.trim().trim_matches('"').is_empty()
                })
        });
        if found {
            return CheckResult {
                name,
                status: CheckStatus::Ok,
                message: format!("{label} — set (source: .env)"),
                details: None,
            };
        }
    }

    CheckResult {
        name,
        status: CheckStatus::Warn,
        message: format!("{label} — not set ({disabled_feature})"),
        details: None,
    }
}

fn check_jq() -> CheckResult {
    match mika_common::validation::check_binary_in_path("jq") {
        Some(path) => CheckResult {
            name: "jq".into(),
            status: CheckStatus::Ok,
            message: format!("jq — found at {}", path.display()),
            details: Some(path.display().to_string()),
        },
        None => CheckResult {
            name: "jq".into(),
            status: CheckStatus::Warn,
            message: "jq — not found in PATH (required by skill handlers)".into(),
            details: None,
        },
    }
}

fn check_mcp(agent_home: &Path) -> CheckResult {
    let mcp_path = agent_home.join("mcp.json");
    if !mcp_path.exists() {
        return CheckResult {
            name: "mcp_servers".into(),
            status: CheckStatus::Ok,
            message: "MCP servers — none configured".into(),
            details: None,
        };
    }

    match std::fs::read_to_string(&mcp_path) {
        Ok(content) => match serde_json::from_str::<serde_json::Value>(&content) {
            Ok(val) => {
                let count = val
                    .get("mcpServers")
                    .and_then(|v| v.as_object())
                    .map(|m| m.len())
                    .unwrap_or(0);
                CheckResult {
                    name: "mcp_servers".into(),
                    status: CheckStatus::Ok,
                    message: format!("MCP servers — {count} configured"),
                    details: Some(mcp_path.display().to_string()),
                }
            }
            Err(e) => CheckResult {
                name: "mcp_servers".into(),
                status: CheckStatus::Warn,
                message: format!("MCP servers — mcp.json parse error: {e}"),
                details: Some(mcp_path.display().to_string()),
            },
        },
        Err(e) => CheckResult {
            name: "mcp_servers".into(),
            status: CheckStatus::Warn,
            message: format!("MCP servers — cannot read mcp.json: {e}"),
            details: Some(mcp_path.display().to_string()),
        },
    }
}

fn check_skills(agent_home: &Path) -> CheckResult {
    let skills_dir = agent_home.join("skills");
    if !skills_dir.is_dir() {
        return CheckResult {
            name: "skills".into(),
            status: CheckStatus::Ok,
            message: "Skills — no skills directory".into(),
            details: None,
        };
    }

    // Count directories with skill.toml
    let count = std::fs::read_dir(&skills_dir)
        .map(|entries| {
            entries
                .filter_map(|e| e.ok())
                .filter(|e| e.path().join("skill.toml").exists())
                .count()
        })
        .unwrap_or(0);

    CheckResult {
        name: "skills".into(),
        status: CheckStatus::Ok,
        message: format!("Skills — {count} installed"),
        details: Some(skills_dir.display().to_string()),
    }
}

async fn check_api_live(global_home: &Path, agent_home: &Path) -> CheckResult {
    // Load settings to get API key and model
    let settings = match mika_common::config::Settings::load_for_agent(global_home, agent_home) {
        Ok(s) => s,
        Err(e) => {
            return CheckResult {
                name: "api_live".into(),
                status: CheckStatus::Fail,
                message: format!("API verify — cannot load config: {e}"),
                details: None,
            };
        }
    };

    let provider = match settings.make_llm_provider() {
        Ok(p) => p,
        Err(e) => {
            return CheckResult {
                name: "api_live".into(),
                status: CheckStatus::Fail,
                message: format!("API verify — provider error: {e}"),
                details: None,
            };
        }
    };

    match provider.check_health().await {
        Ok(()) => CheckResult {
            name: "api_live".into(),
            status: CheckStatus::Ok,
            message: format!(
                "API verify — successful (provider: {}, model: {})",
                provider.provider_name(),
                provider.model_name()
            ),
            details: None,
        },
        Err(e) => CheckResult {
            name: "api_live".into(),
            status: CheckStatus::Fail,
            message: format!("API verify — {e}"),
            details: None,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_check_home_dir_missing() {
        let result = check_home_dir(Path::new("/nonexistent/path"));
        assert_eq!(result.status, CheckStatus::Fail);
        assert!(result.message.contains("not found"));
    }

    #[test]
    fn test_check_home_dir_exists() {
        let tmp = tempfile::tempdir().unwrap();
        let result = check_home_dir(tmp.path());
        // Should not be Fail (exists)
        assert_ne!(result.status, CheckStatus::Fail);
    }

    #[test]
    fn test_check_agent_dir_missing() {
        let result = check_agent_dir(Path::new("/nonexistent/path"), "test");
        assert_eq!(result.status, CheckStatus::Fail);
        assert!(result.message.contains("not found"));
    }

    #[test]
    fn test_check_agent_dir_with_files() {
        let tmp = tempfile::tempdir().unwrap();
        let agent = tmp.path();
        std::fs::write(agent.join("identity.toml"), "name = \"Test\"").unwrap();
        std::fs::write(agent.join("soul.md"), "# Test").unwrap();
        let result = check_agent_dir(agent, "test");
        assert_eq!(result.status, CheckStatus::Ok);
    }

    #[test]
    fn test_check_agent_dir_missing_files() {
        let tmp = tempfile::tempdir().unwrap();
        // dir exists but no required files
        let result = check_agent_dir(tmp.path(), "test");
        assert_eq!(result.status, CheckStatus::Fail);
        assert!(result.message.contains("missing"));
    }

    #[test]
    fn test_check_jq() {
        // Just verify it doesn't panic
        let result = check_jq();
        assert!(result.status == CheckStatus::Ok || result.status == CheckStatus::Warn);
    }

    #[test]
    fn test_check_skills_empty() {
        let tmp = tempfile::tempdir().unwrap();
        let result = check_skills(tmp.path());
        assert_eq!(result.status, CheckStatus::Ok);
    }

    #[test]
    fn test_check_mcp_no_file() {
        let tmp = tempfile::tempdir().unwrap();
        let result = check_mcp(tmp.path());
        assert_eq!(result.status, CheckStatus::Ok);
        assert!(result.message.contains("none configured"));
    }

    #[test]
    fn test_check_config_toml_valid() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("config.toml"),
            "llm_provider = \"anthropic\"\n",
        )
        .unwrap();
        let result = check_config_toml(tmp.path(), tmp.path());
        assert_eq!(result.status, CheckStatus::Ok);
    }

    #[test]
    fn test_check_config_toml_invalid() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("config.toml"), "not valid toml [[[").unwrap();
        let result = check_config_toml(tmp.path(), tmp.path());
        assert_eq!(result.status, CheckStatus::Fail);
    }

    #[test]
    fn test_check_database_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let result = check_database(tmp.path());
        assert_eq!(result.status, CheckStatus::Fail);
        assert!(result.message.contains("not found"));
    }

    #[test]
    fn test_doctor_report_json_serialization() {
        let report = DoctorReport {
            checks: vec![CheckResult {
                name: "test".into(),
                status: CheckStatus::Ok,
                message: "all good".into(),
                details: None,
            }],
            summary: DoctorSummary {
                ok: 1,
                warn: 0,
                fail: 0,
            },
        };
        let json = serde_json::to_string(&report).unwrap();
        assert!(json.contains("\"status\":\"ok\""));
        assert!(json.contains("\"ok\":1"));
    }
}
