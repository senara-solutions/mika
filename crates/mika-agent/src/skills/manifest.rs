use serde::Deserialize;
use std::collections::HashMap;

/// Parsed skill manifest from `skill.toml`.
#[derive(Debug, Clone, Deserialize)]
pub struct SkillManifest {
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub triggers: Triggers,
    pub handler: Handler,
    #[serde(default)]
    pub options: SkillOptions,
}

/// Keyword triggers that control when a skill is injected into a turn.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Triggers {
    #[serde(default)]
    pub keywords: Vec<String>,
}

/// How tool calls for this skill are dispatched.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Handler {
    Builtin {
        tools: Vec<String>,
    },
    Exec {
        command: String,
        #[serde(default)]
        args: Vec<String>,
        tools: Vec<String>,
    },
    Http {
        url: String,
        tools: Vec<String>,
        #[serde(default)]
        headers: HashMap<String, String>,
    },
}

impl Handler {
    /// Tool names owned by this handler.
    pub fn tool_names(&self) -> &[String] {
        match self {
            Handler::Builtin { tools } => tools,
            Handler::Exec { tools, .. } => tools,
            Handler::Http { tools, .. } => tools,
        }
    }
}

fn default_timeout() -> u64 {
    30
}

/// Optional per-skill settings.
#[derive(Debug, Clone, Deserialize)]
pub struct SkillOptions {
    #[serde(default)]
    pub always_on: bool,
    #[serde(default = "default_timeout")]
    pub timeout_secs: u64,
}

impl Default for SkillOptions {
    fn default() -> Self {
        Self {
            always_on: false,
            timeout_secs: default_timeout(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_builtin_handler() {
        let toml_str = r#"
            name = "memory"
            description = "Manage memory"
            [triggers]
            keywords = ["remember", "memory"]
            [handler]
            type = "builtin"
            tools = ["update_core_memory", "store_fact"]
            [options]
            always_on = true
        "#;
        let manifest: SkillManifest = toml::from_str(toml_str).unwrap();
        assert_eq!(manifest.name, "memory");
        assert!(matches!(manifest.handler, Handler::Builtin { .. }));
        assert_eq!(
            manifest.handler.tool_names(),
            &["update_core_memory", "store_fact"]
        );
        assert!(manifest.options.always_on);
        assert_eq!(manifest.options.timeout_secs, 30);
        assert_eq!(manifest.triggers.keywords, vec!["remember", "memory"]);
    }

    #[test]
    fn test_parse_exec_handler() {
        let toml_str = r#"
            name = "calendar"
            description = "Calendar integration"
            [handler]
            type = "exec"
            command = "/usr/local/bin/cal-tool"
            args = ["--format", "json"]
            tools = ["get_events", "create_event"]
        "#;
        let manifest: SkillManifest = toml::from_str(toml_str).unwrap();
        assert_eq!(manifest.name, "calendar");
        match &manifest.handler {
            Handler::Exec {
                command,
                args,
                tools,
            } => {
                assert_eq!(command, "/usr/local/bin/cal-tool");
                assert_eq!(args, &["--format", "json"]);
                assert_eq!(tools, &["get_events", "create_event"]);
            }
            _ => panic!("expected Exec handler"),
        }
        // Defaults
        assert!(!manifest.options.always_on);
        assert!(manifest.triggers.keywords.is_empty());
    }

    #[test]
    fn test_parse_http_handler() {
        let toml_str = r#"
            name = "weather"
            description = "Weather API"
            [handler]
            type = "http"
            url = "http://localhost:8080/tools"
            tools = ["get_weather"]
            [handler.headers]
            Authorization = "Bearer token123"
        "#;
        let manifest: SkillManifest = toml::from_str(toml_str).unwrap();
        match &manifest.handler {
            Handler::Http {
                url,
                tools,
                headers,
            } => {
                assert_eq!(url, "http://localhost:8080/tools");
                assert_eq!(tools, &["get_weather"]);
                assert_eq!(headers.get("Authorization").unwrap(), "Bearer token123");
            }
            _ => panic!("expected Http handler"),
        }
    }

    #[test]
    fn test_parse_missing_optional_fields() {
        let toml_str = r#"
            name = "minimal"
            description = "Minimal skill"
            [handler]
            type = "builtin"
            tools = []
        "#;
        let manifest: SkillManifest = toml::from_str(toml_str).unwrap();
        assert!(manifest.triggers.keywords.is_empty());
        assert!(!manifest.options.always_on);
        assert_eq!(manifest.options.timeout_secs, 30);
    }

    #[test]
    fn test_parse_rejects_missing_required() {
        // Missing handler entirely
        let toml_str = r#"
            name = "broken"
            description = "No handler"
        "#;
        assert!(toml::from_str::<SkillManifest>(toml_str).is_err());

        // Missing name
        let toml_str = r#"
            description = "No name"
            [handler]
            type = "builtin"
            tools = []
        "#;
        assert!(toml::from_str::<SkillManifest>(toml_str).is_err());
    }
}
