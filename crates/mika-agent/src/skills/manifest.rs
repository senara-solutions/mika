use serde::{Deserialize, Serialize};

/// Parsed skill manifest from `skill.toml`.
///
/// New format wraps fields in a `[skill]` section:
/// ```toml
/// [skill]
/// name = "web-search"
/// description = "Search the web"
/// version = "0.1.0"
/// always_on = false
///
/// [triggers]
/// keywords = ["search", "look up"]
/// ```
#[derive(Debug, Clone, Deserialize)]
pub struct SkillManifest {
    pub skill: SkillInfo,
    #[serde(default)]
    pub triggers: Triggers,
}

/// Core skill metadata from the `[skill]` section.
#[derive(Debug, Clone, Deserialize)]
pub struct SkillInfo {
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub always_on: bool,
    #[serde(default = "default_timeout")]
    pub timeout_secs: u64,
}

/// Keyword triggers that control when a skill is injected into a turn.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Triggers {
    #[serde(default)]
    pub keywords: Vec<String>,
}

fn default_timeout() -> u64 {
    30
}

/// How a skill tool call is dispatched to an external process or service.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum ToolHandler {
    Exec {
        command: String,
    },
    Http {
        url: String,
        #[serde(default = "default_http_method")]
        method: String,
    },
}

fn default_http_method() -> String {
    "POST".to_string()
}

/// A tool definition loaded from a skill's `tools.json`.
#[derive(Debug, Clone, Deserialize)]
pub struct SkillToolDef {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
    pub handler: ToolHandler,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_new_format() {
        let toml_str = r#"
            [skill]
            name = "web-search"
            description = "Search the web"
            version = "0.1.0"
            always_on = false
            timeout_secs = 60

            [triggers]
            keywords = ["search", "look up", "find"]
        "#;
        let manifest: SkillManifest = toml::from_str(toml_str).unwrap();
        assert_eq!(manifest.skill.name, "web-search");
        assert_eq!(manifest.skill.description, "Search the web");
        assert_eq!(manifest.skill.version, "0.1.0");
        assert!(!manifest.skill.always_on);
        assert_eq!(manifest.skill.timeout_secs, 60);
        assert_eq!(
            manifest.triggers.keywords,
            vec!["search", "look up", "find"]
        );
    }

    #[test]
    fn test_parse_minimal_new_format() {
        let toml_str = r#"
            [skill]
            name = "minimal"
            description = "Minimal skill"
        "#;
        let manifest: SkillManifest = toml::from_str(toml_str).unwrap();
        assert_eq!(manifest.skill.name, "minimal");
        assert!(manifest.triggers.keywords.is_empty());
        assert!(!manifest.skill.always_on);
        assert_eq!(manifest.skill.timeout_secs, 30);
        assert_eq!(manifest.skill.version, "");
    }

    #[test]
    fn test_parse_always_on() {
        let toml_str = r#"
            [skill]
            name = "memory"
            description = "Always-on memory skill"
            always_on = true
        "#;
        let manifest: SkillManifest = toml::from_str(toml_str).unwrap();
        assert!(manifest.skill.always_on);
    }

    #[test]
    fn test_parse_rejects_missing_required() {
        // Missing [skill] section entirely
        let toml_str = r#"
            name = "broken"
            description = "No skill section"
        "#;
        assert!(toml::from_str::<SkillManifest>(toml_str).is_err());

        // Missing name in [skill]
        let toml_str = r#"
            [skill]
            description = "No name"
        "#;
        assert!(toml::from_str::<SkillManifest>(toml_str).is_err());
    }

    #[test]
    fn test_parse_rejects_legacy_format() {
        // Old flat format without [skill] section should fail to parse
        let toml_str = r#"
            name = "memory"
            description = "Manage memory"
            [triggers]
            keywords = ["remember"]
            [handler]
            type = "builtin"
            tools = ["store_fact"]
            [options]
            always_on = true
        "#;
        assert!(toml::from_str::<SkillManifest>(toml_str).is_err());
    }

    #[test]
    fn test_tool_handler_exec_deserialize() {
        let json = r#"{"type": "exec", "command": "./handler.sh"}"#;
        let handler: ToolHandler = serde_json::from_str(json).unwrap();
        assert!(matches!(handler, ToolHandler::Exec { command } if command == "./handler.sh"));
    }

    #[test]
    fn test_tool_handler_http_deserialize() {
        let json = r#"{"type": "http", "url": "https://api.example.com/search"}"#;
        let handler: ToolHandler = serde_json::from_str(json).unwrap();
        match handler {
            ToolHandler::Http { url, method } => {
                assert_eq!(url, "https://api.example.com/search");
                assert_eq!(method, "POST");
            }
            _ => panic!("expected Http handler"),
        }
    }

    #[test]
    fn test_tool_handler_http_custom_method() {
        let json = r#"{"type": "http", "url": "https://api.example.com/search", "method": "GET"}"#;
        let handler: ToolHandler = serde_json::from_str(json).unwrap();
        match handler {
            ToolHandler::Http { url, method } => {
                assert_eq!(url, "https://api.example.com/search");
                assert_eq!(method, "GET");
            }
            _ => panic!("expected Http handler"),
        }
    }

    #[test]
    fn test_skill_tool_def_deserialize() {
        let json = r#"[{
            "name": "web_search",
            "description": "Search the web",
            "input_schema": {"type": "object", "properties": {"query": {"type": "string"}}},
            "handler": {"type": "exec", "command": "./search.sh"}
        }]"#;
        let tools: Vec<SkillToolDef> = serde_json::from_str(json).unwrap();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].name, "web_search");
        assert!(matches!(&tools[0].handler, ToolHandler::Exec { .. }));
    }
}
