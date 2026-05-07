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
///
/// [constraints]
/// required_tools = ["run_gh"]  # optional — tools that must be called before response
/// ```
///
/// Note: The `[llm]` section is no longer supported in skill.toml (#504).
/// Use `mika skills llm <name> set <provider>/<model>` to configure
/// per-skill LLM overrides (stored in the `skill_overrides` DB table).
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SkillManifest {
    pub skill: SkillInfo,
    #[serde(default)]
    pub triggers: Triggers,
    /// Runtime-only LLM provider/model override for this skill.
    /// Populated from the `skill_overrides` DB table via `apply_overrides()`.
    /// Not deserialized from skill.toml — the `[llm]` section is no longer
    /// supported in manifests (use `mika skills llm <name> set` instead).
    #[serde(skip)]
    pub llm: LlmOverride,
    /// Optional constraints on agent behavior when this skill is active.
    /// Used to enforce tool execution before accepting a response.
    #[serde(default)]
    pub constraints: Constraints,
    /// Optional output format constraints for skill responses.
    /// Used to enforce verdict-line contracts via EndTurn post-condition guard.
    /// See mika#864.
    #[serde(default)]
    pub output: Output,
    /// Declarative context requirements. Each key maps to a `{{key}}` placeholder
    /// in the system prompt. The engine pre-fetches the data before the LLM turn.
    #[serde(default)]
    pub context: std::collections::HashMap<String, ContextRequirement>,
    /// Optional variant management configuration.
    /// Declares which providers have variants, size limits, and required sections
    /// for the validation gate. Backward-compatible — absent section defaults to empty.
    #[serde(default)]
    pub variants: VariantsConfig,
}

/// Per-skill LLM provider and model preferences.
///
/// Populated at runtime from the `skill_overrides` DB table via
/// `SkillRegistry::apply_overrides()`. No longer deserialized from
/// skill.toml — the `[llm]` section is unsupported since #504.
///
/// Both fields are optional:
/// - `provider` only: use this provider with its configured or default model
/// - `model` only: use this model on the agent's active provider
/// - both: fully explicit override
/// - neither (default): use the agent's active provider and model
#[derive(Debug, Clone, Deserialize, Serialize, Default, PartialEq, Eq)]
pub struct LlmOverride {
    /// Provider name matching a `ProviderKind` variant (e.g., "openai", "anthropic").
    /// Case-insensitive at parse time.
    pub provider: Option<String>,
    /// Model identifier to use (e.g., "gpt-4o-mini", "claude-sonnet-4-6").
    pub model: Option<String>,
    /// Whether these values were populated from the `skill_overrides` DB table
    /// (operator intent) rather than a `skill.toml` `[llm]` section (developer
    /// hardcode). Used by `resolve_skill_llm_override()` to allow `AlwaysOn`
    /// skills with DB-sourced overrides to impose their LLM override, while
    /// still filtering out developer-time `[llm]` hijacks per #463.
    /// See mika#1011 for the carve-out rationale.
    #[serde(default, skip_serializing)]
    pub from_db_override: bool,
}

impl LlmOverride {
    /// Returns `true` if no LLM override is configured.
    pub fn is_empty(&self) -> bool {
        self.provider.is_none() && self.model.is_none()
    }
}

/// Constraints on agent behavior when a skill is active.
///
/// Declared in the `[constraints]` section of `skill.toml`. Currently supports:
/// - `required_tools`: Tool names that must be called at least once before the agent
///   engine accepts the assistant's response. If the assistant produces a response
///   without calling all required tools, the engine rejects the response and re-prompts.
///   This prevents the model from fabricating results instead of actually using tools.
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct Constraints {
    /// Tool names that must be called at least once before the response is accepted.
    /// Names can reference skill-defined tools, builtin tools, or MCP tools.
    #[serde(default)]
    pub required_tools: Vec<String>,

    /// When true, the engine inspects the user message at turn-start for quoted
    /// fetchable resources (issue/PR/file blocks) and augments the required_tools
    /// set with the corresponding fetch tool calls. Closes mika#863 (Rule 1
    /// brief-as-claims-not-facts pattern). Opt-in per skill — defaults to false.
    #[serde(default)]
    pub required_fetches_for_quoted_resources: bool,
}

impl Constraints {
    /// Returns `true` if no constraints are configured.
    pub fn is_empty(&self) -> bool {
        self.required_tools.is_empty() && !self.required_fetches_for_quoted_resources
    }
}

/// Output format constraints on skill responses.
///
/// Declared in the `[output]` section of `skill.toml`. Skills that need to enforce
/// a verdict-line contract opt in via `required_suffix_lines` listing the literal
/// exhaustive accept-set. The EndTurn post-condition guard scans the assistant's last
/// 3 non-empty lines for an exact match; missing match rejects EndTurn once.
///
/// See mika#864 for the verdict-ghosting failure mode this addresses.
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct Output {
    /// Exhaustive list of literal lines, one of which must appear as one of the
    /// assistant's last 3 non-empty lines (after trimming whitespace) in any turn
    /// that ends with EndTurn while this skill is active. Empty list = no constraint.
    #[serde(default)]
    pub required_suffix_lines: Vec<String>,
}

impl Output {
    /// Returns `true` if no output constraints are configured.
    pub fn is_empty(&self) -> bool {
        self.required_suffix_lines.is_empty()
    }
}

/// A single context requirement declared in `[context.*]` sections of `skill.toml`.
///
/// Skills declare what data the engine should pre-fetch before the LLM turn.
/// The engine resolves each requirement by dispatching to a handler matched by `context_type`.
///
/// Example in `skill.toml`:
/// ```toml
/// [context.pr_diff]
/// type = "gh_pr_diff"
/// required = true
/// ```
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct ContextRequirement {
    /// Engine-owned context type identifier (e.g., "gh_pr_diff").
    /// Matched to a built-in handler at runtime.
    #[serde(rename = "type")]
    pub context_type: String,
    /// If true (the default), the declaring skill is excluded from the turn
    /// when this context cannot be resolved. If false, the placeholder remains empty.
    #[serde(default = "default_true")]
    pub required: bool,
}

fn default_true() -> bool {
    true
}

/// Core skill metadata from the `[skill]` section.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SkillInfo {
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub always_on: bool,
    #[serde(default = "default_timeout")]
    pub timeout_secs: u64,
    /// Other skills that should be loaded when this skill is active.
    /// Full transitive resolution via BFS at runtime and install time.
    #[serde(default)]
    pub dependencies: Vec<String>,
    /// Optional max prompt snippet size in bytes. Overrides the global 16KB default.
    #[serde(default)]
    pub max_prompt_size: Option<u64>,
}

/// Keyword triggers that control when a skill is injected into a turn.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
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
        #[serde(default)]
        long_running: bool,
        #[serde(default)]
        estimated_duration_secs: Option<u64>,
    },
    Http {
        url: String,
        #[serde(default = "default_http_method")]
        method: String,
    },
    Builtin {
        function: String,
    },
}

fn default_http_method() -> String {
    "POST".to_string()
}

/// Sparse skill.toml overrides from a provider variant directory.
/// Only fields that make sense to vary per-provider are included.
/// Identity fields (name, description, triggers) cannot be overridden.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct ProviderSkillOverride {
    #[serde(default)]
    pub skill: ProviderSkillFields,
}

/// Overridable fields in a provider-specific `skill.toml`.
///
/// All fields are optional — only present fields override the root manifest.
/// `always_on` is intentionally omitted from v1 to avoid matching complexity.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct ProviderSkillFields {
    #[serde(default)]
    pub timeout_secs: Option<u64>,
    #[serde(default)]
    pub max_prompt_size: Option<u64>,
}

/// Optional variant management configuration from `[variants]` in `skill.toml`.
///
/// Controls the validation gate and variant metadata. All fields are optional
/// with backward-compatible defaults (existing skill.toml files parse without changes).
///
/// Example:
/// ```toml
/// [variants]
/// required_sections = ["Tools", "Constraints"]
/// max_prompt_size = 32768
/// ```
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct VariantsConfig {
    /// Providers that have (or are expected to have) variants for this skill.
    /// Informational only — does not restrict variant creation.
    #[serde(default)]
    pub providers: Option<Vec<String>>,
    /// Per-variant max prompt size override (bytes). If set, overrides
    /// `skill.max_prompt_size` for the validation gate.
    #[serde(default)]
    pub max_prompt_size: Option<u64>,
    /// Markdown section headings required in every variant prompt.
    /// The validation gate checks that each heading appears as `# Heading` or `## Heading`.
    #[serde(default)]
    pub required_sections: Option<Vec<String>>,
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
    fn test_parse_dependencies() {
        let toml_str = r#"
            [skill]
            name = "self-dev"
            description = "Self-development skill"
            always_on = true
            dependencies = ["tmux", "shell-exec"]

            [triggers]
            keywords = ["dev"]
        "#;
        let manifest: SkillManifest = toml::from_str(toml_str).unwrap();
        assert_eq!(manifest.skill.dependencies, vec!["tmux", "shell-exec"]);
    }

    #[test]
    fn test_parse_no_dependencies_defaults_empty() {
        let toml_str = r#"
            [skill]
            name = "minimal"
            description = "No deps"
        "#;
        let manifest: SkillManifest = toml::from_str(toml_str).unwrap();
        assert!(manifest.skill.dependencies.is_empty());
    }

    #[test]
    fn test_parse_max_prompt_size() {
        let toml_str = r#"
            [skill]
            name = "big-prompt"
            description = "Skill with custom prompt size"
            max_prompt_size = 32768
        "#;
        let manifest: SkillManifest = toml::from_str(toml_str).unwrap();
        assert_eq!(manifest.skill.max_prompt_size, Some(32768));
    }

    #[test]
    fn test_parse_no_max_prompt_size_defaults_none() {
        let toml_str = r#"
            [skill]
            name = "normal"
            description = "Normal skill"
        "#;
        let manifest: SkillManifest = toml::from_str(toml_str).unwrap();
        assert_eq!(manifest.skill.max_prompt_size, None);
    }

    #[test]
    fn test_tool_handler_exec_deserialize() {
        let json = r#"{"type": "exec", "command": "./handler.sh"}"#;
        let handler: ToolHandler = serde_json::from_str(json).unwrap();
        assert!(matches!(handler, ToolHandler::Exec { command, .. } if command == "./handler.sh"));
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
    fn test_tool_handler_builtin_deserialize() {
        let json = r#"{"type": "builtin", "function": "get_cli_reference"}"#;
        let handler: ToolHandler = serde_json::from_str(json).unwrap();
        assert!(
            matches!(handler, ToolHandler::Builtin { function } if function == "get_cli_reference")
        );
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

    #[test]
    fn test_tool_handler_exec_long_running() {
        let json = r#"{"type": "exec", "command": "./analyze.sh", "long_running": true, "estimated_duration_secs": 300}"#;
        let handler: ToolHandler = serde_json::from_str(json).unwrap();
        match handler {
            ToolHandler::Exec {
                command,
                long_running,
                estimated_duration_secs,
            } => {
                assert_eq!(command, "./analyze.sh");
                assert!(long_running);
                assert_eq!(estimated_duration_secs, Some(300));
            }
            _ => panic!("expected Exec handler"),
        }
    }

    #[test]
    fn test_parse_provider_override_sparse() {
        let toml_str = r#"
            [skill]
            timeout_secs = 60
        "#;
        let parsed: ProviderSkillOverride = toml::from_str(toml_str).unwrap();
        assert_eq!(parsed.skill.timeout_secs, Some(60));
        assert_eq!(parsed.skill.max_prompt_size, None);
    }

    #[test]
    fn test_parse_provider_override_full() {
        let toml_str = r#"
            [skill]
            timeout_secs = 90
            max_prompt_size = 32768
        "#;
        let parsed: ProviderSkillOverride = toml::from_str(toml_str).unwrap();
        assert_eq!(parsed.skill.timeout_secs, Some(90));
        assert_eq!(parsed.skill.max_prompt_size, Some(32768));
    }

    #[test]
    fn test_parse_provider_override_empty_skill_section() {
        let toml_str = r#"
            [skill]
        "#;
        let parsed: ProviderSkillOverride = toml::from_str(toml_str).unwrap();
        assert_eq!(parsed.skill.timeout_secs, None);
        assert_eq!(parsed.skill.max_prompt_size, None);
    }

    #[test]
    fn test_tool_handler_exec_backward_compat() {
        // Missing long_running field should default to false
        let json = r#"{"type": "exec", "command": "./handler.sh"}"#;
        let handler: ToolHandler = serde_json::from_str(json).unwrap();
        match handler {
            ToolHandler::Exec {
                command,
                long_running,
                estimated_duration_secs,
            } => {
                assert_eq!(command, "./handler.sh");
                assert!(!long_running);
                assert_eq!(estimated_duration_secs, None);
            }
            _ => panic!("expected Exec handler"),
        }
    }

    // -- [llm] section tests (now skipped via #[serde(skip)], #504) --

    #[test]
    fn test_parse_llm_section_is_ignored() {
        // [llm] in TOML is silently ignored — field stays empty (#504)
        let toml_str = r#"
            [skill]
            name = "web-search"
            description = "Search the web"

            [llm]
            provider = "openai"
            model = "gpt-4o-mini"
        "#;
        let manifest: SkillManifest = toml::from_str(toml_str).unwrap();
        assert!(
            manifest.llm.is_empty(),
            "[llm] should be ignored with #[serde(skip)]"
        );
    }

    #[test]
    fn test_parse_no_llm_section_defaults_empty() {
        let toml_str = r#"
            [skill]
            name = "normal"
            description = "Normal skill"
        "#;
        let manifest: SkillManifest = toml::from_str(toml_str).unwrap();
        assert!(manifest.llm.is_empty());
        assert_eq!(manifest.llm.provider, None);
        assert_eq!(manifest.llm.model, None);
    }

    #[test]
    fn test_llm_override_equality() {
        let a = LlmOverride {
            provider: Some("openai".to_string()),
            model: Some("gpt-4o".to_string()),
            ..Default::default()
        };
        let b = LlmOverride {
            provider: Some("openai".to_string()),
            model: Some("gpt-4o".to_string()),
            ..Default::default()
        };
        assert_eq!(a, b);

        let c = LlmOverride {
            provider: Some("anthropic".to_string()),
            model: None,
            ..Default::default()
        };
        assert_ne!(a, c);
    }

    #[test]
    fn test_llm_section_not_serialized() {
        // With #[serde(skip)], [llm] should not appear in serialized output
        let toml_str = r#"
            [skill]
            name = "test"
            description = "Test"
        "#;
        let mut manifest: SkillManifest = toml::from_str(toml_str).unwrap();
        // Simulate DB override injection (what apply_overrides does)
        manifest.llm = LlmOverride {
            provider: Some("groq".to_string()),
            model: Some("llama3-8b".to_string()),
            ..Default::default()
        };
        let serialized = toml::to_string_pretty(&manifest).unwrap();
        assert!(
            !serialized.contains("[llm]"),
            "[llm] should not appear in serialized output. Got:\n{serialized}"
        );
    }

    #[test]
    fn test_existing_skill_toml_backward_compat() {
        // A skill.toml without [llm] should parse fine
        let toml_str = r#"
            [skill]
            name = "web-search"
            description = "Search the web"
            version = "0.1.0"
            always_on = true
            timeout_secs = 30

            [triggers]
            keywords = ["search", "look up"]
        "#;
        let manifest: SkillManifest = toml::from_str(toml_str).unwrap();
        assert!(manifest.llm.is_empty());
        assert!(manifest.constraints.is_empty());
        assert_eq!(manifest.skill.name, "web-search");
        assert!(manifest.skill.always_on);
    }

    // -- [constraints] section tests --

    #[test]
    fn test_parse_constraints_required_tools() {
        let toml_str = r#"
            [skill]
            name = "qa-review"
            description = "Review PRs"

            [constraints]
            required_tools = ["run_gh", "web_search"]
        "#;
        let manifest: SkillManifest = toml::from_str(toml_str).unwrap();
        assert!(!manifest.constraints.is_empty());
        assert_eq!(
            manifest.constraints.required_tools,
            vec!["run_gh", "web_search"]
        );
    }

    #[test]
    fn test_parse_no_constraints_defaults_empty() {
        // Backward compat: skill.toml without [constraints] should parse fine
        let toml_str = r#"
            [skill]
            name = "simple"
            description = "No constraints"
        "#;
        let manifest: SkillManifest = toml::from_str(toml_str).unwrap();
        assert!(manifest.constraints.is_empty());
        assert!(manifest.constraints.required_tools.is_empty());
    }

    #[test]
    fn test_constraints_standalone() {
        let toml_str = r#"
            [skill]
            name = "review"
            description = "Code review skill"

            [constraints]
            required_tools = ["run_gh"]
        "#;
        let manifest: SkillManifest = toml::from_str(toml_str).unwrap();
        assert!(manifest.llm.is_empty());
        assert_eq!(manifest.constraints.required_tools, vec!["run_gh"]);
    }

    #[test]
    fn test_parse_constraints_required_fetches_for_quoted_resources() {
        let toml_str = r#"
            [skill]
            name = "arch-review"
            description = "Architecture review"

            [constraints]
            required_tools = ["gh_read"]
            required_fetches_for_quoted_resources = true
        "#;
        let manifest: SkillManifest = toml::from_str(toml_str).unwrap();
        assert!(!manifest.constraints.is_empty());
        assert!(manifest.constraints.required_fetches_for_quoted_resources);
        assert_eq!(manifest.constraints.required_tools, vec!["gh_read"]);
    }

    #[test]
    fn test_constraints_is_empty_false_when_only_quoted_resources_opt_in() {
        let toml_str = r#"
            [skill]
            name = "arch-review"
            description = "Architecture review"

            [constraints]
            required_fetches_for_quoted_resources = true
        "#;
        let manifest: SkillManifest = toml::from_str(toml_str).unwrap();
        assert!(!manifest.constraints.is_empty());
        assert!(manifest.constraints.required_fetches_for_quoted_resources);
        assert!(manifest.constraints.required_tools.is_empty());
    }

    #[test]
    fn test_constraints_required_fetches_defaults_false() {
        let toml_str = r#"
            [skill]
            name = "qa-review"
            description = "Review PRs"

            [constraints]
            required_tools = ["run_gh"]
        "#;
        let manifest: SkillManifest = toml::from_str(toml_str).unwrap();
        assert!(!manifest.constraints.required_fetches_for_quoted_resources);
    }

    // -- [output] section tests --

    #[test]
    fn test_parse_output_required_suffix_lines() {
        let toml_str = r#"
            [skill]
            name = "arch-second-review"
            description = "Second-pass plan review"

            [output]
            required_suffix_lines = ["Verdict: GROOMED", "Verdict: ESCALATE"]
        "#;
        let manifest: SkillManifest = toml::from_str(toml_str).unwrap();
        assert!(!manifest.output.is_empty());
        assert_eq!(
            manifest.output.required_suffix_lines,
            vec!["Verdict: GROOMED", "Verdict: ESCALATE"]
        );
    }

    #[test]
    fn test_parse_no_output_defaults_empty() {
        // Backward compat: skill.toml without [output] should parse fine
        let toml_str = r#"
            [skill]
            name = "simple"
            description = "No output constraints"
        "#;
        let manifest: SkillManifest = toml::from_str(toml_str).unwrap();
        assert!(manifest.output.is_empty());
        assert!(manifest.output.required_suffix_lines.is_empty());
    }

    #[test]
    fn test_output_coexists_with_constraints() {
        let toml_str = r#"
            [skill]
            name = "arch-groom"
            description = "First-pass plan review"

            [constraints]
            required_tools = ["gh_read"]

            [output]
            required_suffix_lines = ["Disposition: READY", "Disposition: ITERATE", "Disposition: ESCALATE"]
        "#;
        let manifest: SkillManifest = toml::from_str(toml_str).unwrap();
        assert!(!manifest.constraints.is_empty());
        assert_eq!(manifest.constraints.required_tools, vec!["gh_read"]);
        assert!(!manifest.output.is_empty());
        assert_eq!(manifest.output.required_suffix_lines.len(), 3);
    }

    #[test]
    fn test_output_empty_list_parses() {
        let toml_str = r#"
            [skill]
            name = "test-skill"
            description = "Explicit empty list"

            [output]
            required_suffix_lines = []
        "#;
        let manifest: SkillManifest = toml::from_str(toml_str).unwrap();
        assert!(manifest.output.is_empty());
    }

    // -- [context] section tests --

    #[test]
    fn test_parse_context_section() {
        let toml_str = r#"
            [skill]
            name = "qa-review"
            description = "Review PRs"

            [context.pr_diff]
            type = "gh_pr_diff"
            required = true
        "#;
        let manifest: SkillManifest = toml::from_str(toml_str).unwrap();
        assert_eq!(manifest.context.len(), 1);
        let req = manifest.context.get("pr_diff").unwrap();
        assert_eq!(req.context_type, "gh_pr_diff");
        assert!(req.required);
    }

    #[test]
    fn test_parse_no_context_defaults_empty() {
        let toml_str = r#"
            [skill]
            name = "simple"
            description = "No context"
        "#;
        let manifest: SkillManifest = toml::from_str(toml_str).unwrap();
        assert!(manifest.context.is_empty());
    }

    #[test]
    fn test_context_required_defaults_to_true() {
        let toml_str = r#"
            [skill]
            name = "qa-review"
            description = "Review PRs"

            [context.pr_diff]
            type = "gh_pr_diff"
        "#;
        let manifest: SkillManifest = toml::from_str(toml_str).unwrap();
        let req = manifest.context.get("pr_diff").unwrap();
        assert!(req.required);
    }

    #[test]
    fn test_context_required_false() {
        let toml_str = r#"
            [skill]
            name = "qa-review"
            description = "Review PRs"

            [context.pr_diff]
            type = "gh_pr_diff"
            required = false
        "#;
        let manifest: SkillManifest = toml::from_str(toml_str).unwrap();
        let req = manifest.context.get("pr_diff").unwrap();
        assert!(!req.required);
    }

    #[test]
    fn test_context_coexists_with_constraints() {
        let toml_str = r#"
            [skill]
            name = "qa-review"
            description = "Review PRs"

            [constraints]
            required_tools = ["run_gh"]

            [context.pr_diff]
            type = "gh_pr_diff"
        "#;
        let manifest: SkillManifest = toml::from_str(toml_str).unwrap();
        assert!(manifest.llm.is_empty());
        assert_eq!(manifest.constraints.required_tools, vec!["run_gh"]);
        assert_eq!(manifest.context.len(), 1);
        assert_eq!(
            manifest.context.get("pr_diff").unwrap().context_type,
            "gh_pr_diff"
        );
    }
}
