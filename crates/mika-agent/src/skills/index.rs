use std::collections::{BTreeSet, HashMap};
use std::fmt;
use std::path::{Path, PathBuf};
use tracing::{error, warn};

use mika_common::claude::ToolDefinition;
use mika_common::llm::ProviderKind;

use super::builtin_handlers::KNOWN_BUILTINS;
use super::manifest::{
    ProviderSkillFields, ProviderSkillOverride, SkillManifest, SkillToolDef, ToolHandler,
};

/// Maximum size for skill.toml files (64 KB).
const MAX_SKILL_TOML_SIZE: u64 = 64 * 1024;

/// Default maximum size for system_prompt.md snippets (16 KB).
pub(super) const MAX_PROMPT_SNIPPET_SIZE: u64 = 16 * 1024;

/// Hard ceiling for per-skill `max_prompt_size` override (64 KB).
/// Prevents marketplace skills from loading arbitrarily large prompts.
pub(super) const MAX_PROMPT_SIZE_CEILING: u64 = 64 * 1024;

/// Maximum size for tools.json files (256 KB).
const MAX_TOOLS_JSON_SIZE: u64 = 256 * 1024;

/// A skill tool with its Claude-facing definition and dispatch handler.
#[derive(Debug, Clone)]
pub struct ResolvedSkillTool {
    pub definition: ToolDefinition,
    pub handler: ToolHandler,
    pub skill_dir: PathBuf,
}

/// Describes which step in the `resolve_prompt()` fallback chain produced the
/// winning prompt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptVariantSource {
    /// Step 1: hand-authored model variant under `<provider>/<model>/`.
    HandAuthoredModel,
    /// Step 2: auto-generated variant under `generated/<provider>/<model>/`.
    GeneratedModel,
    /// Step 3: auto-generated variant under canonical `generated/<canonical_provider>/<canonical_model>/`.
    GeneratedCanonical,
    /// Step 4: root `system_prompt.md`.
    Base,
}

impl fmt::Display for PromptVariantSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::HandAuthoredModel => write!(f, "hand_authored_model"),
            Self::GeneratedModel => write!(f, "generated_model"),
            Self::GeneratedCanonical => write!(f, "generated_canonical"),
            Self::Base => write!(f, "base"),
        }
    }
}

/// Result of resolving a prompt variant via `SkillEntry::resolve_prompt()`.
#[derive(Debug, Clone)]
pub struct ResolvedPrompt<'a> {
    /// The resolved prompt text.
    pub text: &'a str,
    /// Which fallback step produced this prompt.
    pub source: PromptVariantSource,
    /// The lookup key that matched (e.g. `"anthropic/claude-sonnet-4-6"`).
    /// `None` when `source` is `Base`.
    pub key: Option<String>,
}

impl ResolvedPrompt<'_> {
    /// Compact descriptor for storage in `llm_calls.prompt_variant`.
    ///
    /// Returns `"base"` for the root prompt, or `"{source}:{key}"` for variant
    /// hits (e.g. `"generated_model:anthropic/claude-sonnet-4-6"`).
    pub fn variant_descriptor(&self) -> String {
        match &self.key {
            Some(key) => format!("{}:{}", self.source, key),
            None => self.source.to_string(),
        }
    }
}

/// A loaded skill entry with its manifest and pre-processed data.
#[derive(Debug, Clone)]
pub struct SkillEntry {
    pub manifest: SkillManifest,
    pub dir: PathBuf,
    /// Pre-lowercased keywords for fast substring matching.
    pub keywords_lower: Vec<String>,
    /// Cached prompt snippet content (loaded at startup, empty if no file).
    pub prompt_snippet: String,
    /// Tools defined in this skill's `tools.json`.
    pub skill_tools: Vec<ResolvedSkillTool>,
    /// Whether the skill is enabled (no `.disabled` marker file).
    pub enabled: bool,
    /// Whether this entry has a DB override applied (for display purposes).
    pub has_override: bool,
    /// Provider-specific manifest field overrides.
    /// Key = provider name, value = sparse override fields.
    /// Empty map if no variants exist.
    pub provider_overrides: HashMap<String, ProviderSkillFields>,
    /// Model-specific prompt snippet overrides.
    /// Key = "{provider}/{sanitized_model}" (e.g., "anthropic/claude-sonnet-4-6").
    /// Empty map if no model variants exist. Populated eagerly at scan time.
    pub model_prompts: HashMap<String, String>,
    /// Model-specific manifest field overrides.
    /// Key = "{provider}/{sanitized_model}", value = sparse override fields.
    /// Empty map if no model variants exist.
    pub model_overrides: HashMap<String, ProviderSkillFields>,
    /// Auto-generated model prompts under `generated/{provider}/{sanitized_model}/`.
    /// Key = "{provider}/{sanitized_model}". Populated by `review_skill` (when
    /// called with `content`) at runtime; loaded eagerly at scan time. Hand-authored variants in
    /// `model_prompts` always win over generated entries here (see `resolve_prompt`).
    pub generated_model_prompts: HashMap<String, String>,
}

impl SkillEntry {
    /// Effective timeout: model override > provider override > root.
    pub fn effective_timeout(&self, provider: &str, model: &str) -> u64 {
        let model_key = format!("{}/{}", provider, sanitize_model_dir_name(model));
        self.model_overrides
            .get(&model_key)
            .and_then(|o| o.timeout_secs)
            .or_else(|| {
                self.provider_overrides
                    .get(provider)
                    .and_then(|o| o.timeout_secs)
            })
            .unwrap_or(self.manifest.skill.timeout_secs)
    }

    /// Resolve the best prompt for a given provider + model combination.
    ///
    /// Fallback chain (first match wins):
    /// 1. Hand-authored model variant under requesting `<provider>/<model>/`
    /// 2. Auto-generated variant under requesting `generated/<provider>/<model>/`
    /// 3. Auto-generated variant under canonical `generated/<canonical_provider>/<canonical_model>/`
    ///    (so an openrouter caller picks up a variant written under the underlying
    ///    provider — `openrouter` + `minimax/minimax-m2.7` → `minimax/minimax-m2.7`)
    /// 4. Root `system_prompt.md`
    ///
    /// Hand-authored entries always win — they represent intentional human
    /// curation and must not be silently shadowed by autogenerated content.
    /// Provider-level prompts are intentionally not supported.
    pub fn resolve_prompt(&self, provider: &str, model: &str) -> ResolvedPrompt<'_> {
        let requesting_key = format!("{}/{}", provider, sanitize_model_dir_name(model));
        if let Some(prompt) = self.model_prompts.get(&requesting_key) {
            return ResolvedPrompt {
                text: prompt,
                source: PromptVariantSource::HandAuthoredModel,
                key: Some(requesting_key),
            };
        }
        if let Some(prompt) = self.generated_model_prompts.get(&requesting_key) {
            return ResolvedPrompt {
                text: prompt,
                source: PromptVariantSource::GeneratedModel,
                key: Some(requesting_key),
            };
        }
        // Generated variants are written under the *canonical* provider/model
        // tuple (aggregator namespace stripped), so an openrouter caller must
        // also probe the canonical key. Hand-authored variants intentionally
        // do not get this fallback — users author against their requesting
        // provider explicitly.
        let (canonical_provider, canonical_model) =
            resolve_canonical_provider_model(provider, model);
        if canonical_provider != provider || canonical_model != model {
            let canonical_key = format!(
                "{}/{}",
                canonical_provider,
                sanitize_model_dir_name(canonical_model)
            );
            if let Some(prompt) = self.generated_model_prompts.get(&canonical_key) {
                return ResolvedPrompt {
                    text: prompt,
                    source: PromptVariantSource::GeneratedCanonical,
                    key: Some(canonical_key),
                };
            }
        }
        ResolvedPrompt {
            text: &self.prompt_snippet,
            source: PromptVariantSource::Base,
            key: None,
        }
    }

    /// Sorted set of all provider names that have any variant (override or model).
    pub fn variant_providers(&self) -> BTreeSet<&str> {
        let mut providers = BTreeSet::new();
        for key in self.provider_overrides.keys() {
            providers.insert(key.as_str());
        }
        // Also include providers from model variants
        for key in self.model_prompts.keys() {
            if let Some(provider) = key.split('/').next() {
                providers.insert(provider);
            }
        }
        for key in self.model_overrides.keys() {
            if let Some(provider) = key.split('/').next() {
                providers.insert(provider);
            }
        }
        providers
    }

    /// Model variants for a specific provider.
    pub fn variant_models(&self, provider: &str) -> BTreeSet<&str> {
        let prefix = format!("{provider}/");
        let mut models = BTreeSet::new();
        for key in self.model_prompts.keys() {
            if let Some(model) = key.strip_prefix(&prefix) {
                models.insert(model);
            }
        }
        for key in self.model_overrides.keys() {
            if let Some(model) = key.strip_prefix(&prefix) {
                models.insert(model);
            }
        }
        models
    }

    /// Total number of distinct variant entries (providers + models).
    pub fn variant_count(&self) -> usize {
        let mut all_keys = BTreeSet::new();
        // Provider-level keys (overrides only)
        for key in self.provider_overrides.keys() {
            all_keys.insert(key.as_str());
        }
        // Model-level composite keys
        for key in self.model_prompts.keys() {
            all_keys.insert(key.as_str());
        }
        for key in self.model_overrides.keys() {
            all_keys.insert(key.as_str());
        }
        all_keys.len()
    }
}

/// Sanitize a model name for use as a directory name.
/// Replaces '/' with '--' to avoid filesystem path conflicts.
/// Applied at both scan time (directory discovery) and resolution time (lookup).
pub(crate) fn sanitize_model_dir_name(model: &str) -> String {
    model.replace('/', "--")
}

/// Resolve the canonical (provider, model) tuple for a requesting provider/model.
///
/// For aggregator providers (e.g. OpenRouter) whose model names contain a slash
/// (`anthropic/claude-sonnet-4`), extracts the underlying provider and model so
/// that variants written under the canonical tuple can be looked up via the
/// aggregator alias and vice versa. For direct providers the inputs are
/// returned unchanged.
pub(crate) fn resolve_canonical_provider_model<'a>(
    provider_name: &'a str,
    model_name: &'a str,
) -> (&'a str, &'a str) {
    let Ok(kind) = provider_name.parse::<ProviderKind>() else {
        return (provider_name, model_name);
    };

    if kind.model_names_contain_slash()
        && let Some((real_provider, real_model)) = model_name.split_once('/')
        && !real_provider.is_empty()
        && !real_model.is_empty()
    {
        return (real_provider, real_model);
    }

    (provider_name, model_name)
}

/// A skill that was found but could not be loaded.
#[derive(Debug, Clone)]
pub struct SkippedSkill {
    /// Directory name (not manifest name — manifest may be unreadable).
    pub name: String,
    /// Human-readable reason for skipping.
    pub reason: String,
}

/// Result of scanning a skills directory.
pub struct ScanResult {
    pub entries: Vec<SkillEntry>,
    /// Details of skills that were skipped during scan.
    pub skipped: Vec<SkippedSkill>,
}

/// Scan a skills directory and load all valid skill manifests.
///
/// Each immediate subdirectory is expected to contain a `skill.toml`.
/// Invalid skills are logged at `warn` and skipped — never break startup.
/// Legacy-format skills (has `[handler]` section) are skipped with a
/// deprecation warning. Returns entries and the count of skipped directories.
pub fn scan_skills_dir(skills_dir: &Path) -> ScanResult {
    let read_dir = match std::fs::read_dir(skills_dir) {
        Ok(rd) => rd,
        Err(e) => {
            warn!(path = %skills_dir.display(), error = %e, "cannot read skills directory");
            return ScanResult {
                entries: Vec::new(),
                skipped: Vec::new(),
            };
        }
    };

    let mut entries = Vec::new();
    let mut skipped: Vec<SkippedSkill> = Vec::new();
    for dir_entry in read_dir {
        let dir_entry = match dir_entry {
            Ok(de) => de,
            Err(e) => {
                warn!(error = %e, "error reading skills directory entry");
                continue;
            }
        };

        let path = dir_entry.path();
        let dir_name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown");

        // Detect broken symlinks (linked skills whose target was removed)
        if let Ok(meta) = std::fs::symlink_metadata(&path)
            && meta.file_type().is_symlink()
            && !path.exists()
        {
            let target = std::fs::read_link(&path).ok();
            let reason = match &target {
                Some(t) => format!("broken symlink \u{2192} {}", t.display()),
                None => "broken symlink".to_string(),
            };
            warn!(
                skill = dir_name,
                target = ?target,
                "Broken symlink for skill '{}': target no longer exists. \
                 Reinstall or remove with 'mika skills uninstall {}'",
                dir_name,
                dir_name
            );
            skipped.push(SkippedSkill {
                name: dir_name.to_string(),
                reason,
            });
            continue;
        }

        if !path.is_dir() {
            continue;
        }

        let manifest_path = path.join("skill.toml");

        // Check file size before reading to prevent OOM from oversized files
        if let Ok(meta) = std::fs::metadata(&manifest_path)
            && meta.len() > MAX_SKILL_TOML_SIZE
        {
            warn!(
                path = %manifest_path.display(),
                size = meta.len(),
                "skill.toml exceeds 64KB, skipping"
            );
            skipped.push(SkippedSkill {
                name: dir_name.to_string(),
                reason: format!("skill.toml exceeds 64KB ({}B)", meta.len()),
            });
            continue;
        }

        let content = match std::fs::read_to_string(&manifest_path) {
            Ok(c) => c,
            Err(e) => {
                warn!(path = %manifest_path.display(), error = %e, "cannot read skill manifest");
                skipped.push(SkippedSkill {
                    name: dir_name.to_string(),
                    reason: format!("cannot read manifest: {e}"),
                });
                continue;
            }
        };

        // Detect legacy format: has [handler] section with type field but no [skill]
        if is_legacy_format(&content) {
            warn!(
                path = %manifest_path.display(),
                "skipping legacy-format skill (has [handler] section). \
                 Migrate to new [skill] section format — handler config belongs in tools.json."
            );
            skipped.push(SkippedSkill {
                name: dir_name.to_string(),
                reason: "legacy format (has [handler] section)".to_string(),
            });
            continue;
        }

        let manifest: SkillManifest = match toml::from_str(&content) {
            Ok(m) => m,
            Err(e) => {
                warn!(path = %manifest_path.display(), error = %e, "invalid skill manifest");
                skipped.push(SkippedSkill {
                    name: dir_name.to_string(),
                    reason: format!("invalid TOML: {e}"),
                });
                continue;
            }
        };

        let keywords_lower = manifest
            .triggers
            .keywords
            .iter()
            .map(|k| k.to_lowercase())
            .collect();

        // Load prompt snippet eagerly at startup (cached in SkillEntry)
        let snippet_path = path.join("system_prompt.md");
        let max_size = manifest
            .skill
            .max_prompt_size
            .map(|v| v.min(MAX_PROMPT_SIZE_CEILING))
            .unwrap_or(MAX_PROMPT_SNIPPET_SIZE);
        if let Some(requested) = manifest.skill.max_prompt_size
            && requested > MAX_PROMPT_SIZE_CEILING
        {
            warn!(
                skill = %manifest.skill.name,
                requested = requested,
                ceiling = MAX_PROMPT_SIZE_CEILING,
                "max_prompt_size exceeds ceiling, clamping"
            );
        }
        let prompt_snippet = match load_snippet_with_limit(&snippet_path, max_size) {
            SnippetLoadResult::Ok(content) => content,
            SnippetLoadResult::Empty => String::new(),
            SnippetLoadResult::Oversized { size, limit } => {
                if manifest.skill.always_on {
                    error!(
                        skill = %manifest.skill.name,
                        path = %snippet_path.display(),
                        size,
                        limit,
                        "always_on skill prompt exceeds size limit — skill NOT loaded. \
                         An always_on skill without its prompt is functionally broken. \
                         Increase max_prompt_size in skill.toml (ceiling: 64KB) or reduce the prompt."
                    );
                    skipped.push(SkippedSkill {
                        name: manifest.skill.name.clone(),
                        reason: format!("oversized prompt ({size}B, limit {limit}B)"),
                    });
                    continue;
                }
                error!(
                    skill = %manifest.skill.name,
                    path = %snippet_path.display(),
                    size,
                    limit,
                    "prompt snippet exceeds size limit — prompt will be empty. \
                     Increase max_prompt_size in skill.toml (ceiling: 64KB) or reduce the prompt."
                );
                String::new()
            }
            SnippetLoadResult::ReadError(e) => {
                if manifest.skill.always_on {
                    error!(
                        skill = %manifest.skill.name,
                        path = %snippet_path.display(),
                        error = %e,
                        "always_on skill prompt unreadable — skill NOT loaded"
                    );
                    skipped.push(SkippedSkill {
                        name: manifest.skill.name.clone(),
                        reason: format!("unreadable prompt: {e}"),
                    });
                    continue;
                }
                warn!(
                    skill = %manifest.skill.name,
                    path = %snippet_path.display(),
                    error = %e,
                    "cannot read prompt snippet"
                );
                String::new()
            }
        };

        // Check for .disabled marker file
        let enabled = !path.join(".disabled").exists();

        // Parse tools.json if present
        let skill_tools = load_tools_json(&path);

        // Scan for provider and model variant directories
        let variants = scan_provider_variants(&path, &manifest);

        entries.push(SkillEntry {
            manifest,
            dir: path,
            keywords_lower,
            prompt_snippet,
            skill_tools,
            enabled,
            has_override: false,
            provider_overrides: variants.provider_overrides,
            model_prompts: variants.model_prompts,
            generated_model_prompts: variants.generated_model_prompts,
            model_overrides: variants.model_overrides,
        });
    }

    ScanResult { entries, skipped }
}

/// Diagnostic level for skill validation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum DiagnosticLevel {
    Ok,
    Warn,
    Fail,
}

/// A single diagnostic finding from skill validation.
#[derive(Debug, Clone, serde::Serialize)]
pub struct SkillDiagnostic {
    pub level: DiagnosticLevel,
    pub message: String,
}

impl SkillDiagnostic {
    pub fn ok(msg: impl Into<String>) -> Self {
        Self {
            level: DiagnosticLevel::Ok,
            message: msg.into(),
        }
    }
    pub fn warn(msg: impl Into<String>) -> Self {
        Self {
            level: DiagnosticLevel::Warn,
            message: msg.into(),
        }
    }
    pub fn fail(msg: impl Into<String>) -> Self {
        Self {
            level: DiagnosticLevel::Fail,
            message: msg.into(),
        }
    }

    pub fn tag(&self) -> &'static str {
        match self.level {
            DiagnosticLevel::Ok => "[OK]",
            DiagnosticLevel::Warn => "[WARN]",
            DiagnosticLevel::Fail => "[FAIL]",
        }
    }
}

/// Emit startup warnings for skills with `[llm]` overrides that reference
/// providers without configured API keys. Call after `scan_skills_dir()`.
pub fn warn_missing_llm_api_keys(entries: &[SkillEntry], settings: &mika_common::config::Settings) {
    for entry in entries {
        if let Some(ref provider_str) = entry.manifest.llm.provider
            && let Ok(pk) = provider_str.parse::<ProviderKind>()
        {
            let (_, api_key, _) = settings.provider_fields(pk);
            // Ollama doesn't require an API key
            if pk != ProviderKind::Ollama && api_key.filter(|k| !k.trim().is_empty()).is_none() {
                warn!(
                    skill = %entry.manifest.skill.name,
                    provider = %provider_str,
                    "skill declares [llm].provider but no API key is configured for this provider — \
                     LLM calls will fail when this skill is active"
                );
            }
        }
    }
}

/// Validate a single skill directory and return diagnostics.
pub fn validate_skill(skill_dir: &Path) -> Vec<SkillDiagnostic> {
    let mut diags = Vec::new();

    // 1. Check skill.toml exists and is readable
    let manifest_path = skill_dir.join("skill.toml");
    if !manifest_path.exists() {
        diags.push(SkillDiagnostic::fail("skill.toml not found"));
        return diags;
    }

    // Check file size
    if let Ok(meta) = std::fs::metadata(&manifest_path)
        && meta.len() > MAX_SKILL_TOML_SIZE
    {
        diags.push(SkillDiagnostic::fail(format!(
            "skill.toml exceeds 64KB ({}KB)",
            meta.len() / 1024
        )));
        return diags;
    }

    let content = match std::fs::read_to_string(&manifest_path) {
        Ok(c) => c,
        Err(e) => {
            diags.push(SkillDiagnostic::fail(format!(
                "cannot read skill.toml: {e}"
            )));
            return diags;
        }
    };

    // 2. Check for valid TOML and legacy format
    if is_legacy_format(&content) {
        diags.push(SkillDiagnostic::fail(
            "legacy format detected: has [handler] section. \
             Migrate to [skill] section + tools.json per-tool handlers."
                .to_string(),
        ));
        return diags;
    }

    // 3. Parse as SkillManifest
    let manifest: SkillManifest = match toml::from_str(&content) {
        Ok(m) => m,
        Err(e) => {
            diags.push(SkillDiagnostic::fail(format!("invalid skill.toml: {e}")));
            return diags;
        }
    };
    diags.push(SkillDiagnostic::ok(format!(
        "skill.toml valid — name={}, description={}",
        manifest.skill.name,
        manifest
            .skill
            .description
            .chars()
            .take(60)
            .collect::<String>()
    )));

    // 3b. Validate [llm] section if present
    if !manifest.llm.is_empty() {
        if let Some(ref provider_str) = manifest.llm.provider {
            match provider_str.parse::<ProviderKind>() {
                Ok(pk) => {
                    diags.push(SkillDiagnostic::ok(format!(
                        "[llm] provider '{}' is valid",
                        pk.config_prefix()
                    )));
                }
                Err(_) => {
                    diags.push(SkillDiagnostic::fail(format!(
                        "[llm] provider '{}' is not a valid provider. \
                         Valid providers: anthropic, openai, openrouter, groq, ollama, \
                         mistral, google, deepseek, minimax, kimi, qwen",
                        provider_str
                    )));
                }
            }
        }
        if let Some(ref model_str) = manifest.llm.model {
            if model_str.trim().is_empty() {
                diags.push(SkillDiagnostic::warn(
                    "[llm] model is empty — will use provider default".to_string(),
                ));
            } else {
                diags.push(SkillDiagnostic::ok(format!("[llm] model '{}'", model_str)));
            }
        }
    }

    // 4. Check tools.json if present
    let tools_path = skill_dir.join("tools.json");
    let mut skill_tool_names: std::collections::HashSet<String> = std::collections::HashSet::new();
    if tools_path.exists() {
        if let Ok(meta) = std::fs::metadata(&tools_path)
            && meta.len() > MAX_TOOLS_JSON_SIZE
        {
            diags.push(SkillDiagnostic::fail(format!(
                "tools.json exceeds 256KB ({}KB)",
                meta.len() / 1024
            )));
            return diags;
        }
        match std::fs::read_to_string(&tools_path) {
            Ok(json_content) => {
                match serde_json::from_str::<Vec<super::manifest::SkillToolDef>>(&json_content) {
                    Ok(tools) => {
                        diags.push(SkillDiagnostic::ok(format!(
                            "tools.json valid — {} tool(s)",
                            tools.len()
                        )));
                        // Collect tool names for required_tools validation (step 5b)
                        skill_tool_names = tools.iter().map(|t| t.name.clone()).collect();

                        // 5. Check exec handler commands exist and are executable
                        for tool in &tools {
                            if let ToolHandler::Exec { command, .. } = &tool.handler {
                                let cmd_path = skill_dir.join(command);
                                if !cmd_path.exists() {
                                    diags.push(SkillDiagnostic::fail(format!(
                                        "tool '{}': handler command not found: {} (resolved to {})",
                                        tool.name,
                                        command,
                                        cmd_path.display()
                                    )));
                                } else {
                                    #[cfg(unix)]
                                    {
                                        use std::os::unix::fs::PermissionsExt;
                                        if let Ok(meta) = std::fs::metadata(&cmd_path) {
                                            if meta.permissions().mode() & 0o111 == 0 {
                                                diags.push(SkillDiagnostic::fail(format!(
                                                    "tool '{}': handler command not executable: {}",
                                                    tool.name,
                                                    cmd_path.display()
                                                )));
                                            } else {
                                                // Symlink containment check
                                                if let Ok(canonical) = cmd_path.canonicalize()
                                                    && !canonical.starts_with(skill_dir)
                                                {
                                                    diags.push(SkillDiagnostic::warn(format!(
                                                        "tool '{}': handler command '{}' resolves outside skill directory",
                                                        tool.name, command
                                                    )));
                                                }
                                                diags.push(SkillDiagnostic::ok(format!(
                                                    "tool '{}': handler command OK",
                                                    tool.name
                                                )));
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                    Err(e) => {
                        diags.push(SkillDiagnostic::fail(format!("invalid tools.json: {e}")));
                    }
                }
            }
            Err(e) => {
                diags.push(SkillDiagnostic::fail(format!(
                    "cannot read tools.json: {e}"
                )));
            }
        }
    }

    // 5b. Validate [constraints] required_tools against known tool names
    for required in &manifest.constraints.required_tools {
        if !skill_tool_names.contains(required) {
            // Advisory warning — the tool might be a builtin or MCP tool
            diags.push(SkillDiagnostic::warn(format!(
                "[constraints] required_tools references '{}' which is not in this skill's \
                 tools.json — this is OK if it's a builtin or MCP tool",
                required
            )));
        }
    }

    // 5c. Warn if always_on skill with no keywords declares required_tools (#265)
    // Such constraints will never be enforced because required_tools only triggers
    // when a skill is matched via keyword, not just always_on.
    if manifest.skill.always_on
        && manifest.triggers.keywords.is_empty()
        && !manifest.constraints.required_tools.is_empty()
    {
        diags.push(SkillDiagnostic::warn(
            "[constraints] required_tools declared on always_on skill with no keywords — \
             constraints will only be enforced when the skill matches via keyword. \
             Add keywords to [triggers] or the required_tools will never be enforced."
                .to_string(),
        ));
    }

    // 5d. Validate [context] section
    for (key, req) in &manifest.context {
        if !super::context::KNOWN_CONTEXT_TYPES.contains(&req.context_type.as_str()) {
            diags.push(SkillDiagnostic::fail(format!(
                "[context.{}] declares unknown type '{}'. Known types: {:?}",
                key,
                req.context_type,
                super::context::KNOWN_CONTEXT_TYPES
            )));
        } else {
            diags.push(SkillDiagnostic::ok(format!(
                "[context.{}] type '{}' is valid (required={})",
                key, req.context_type, req.required
            )));
        }
    }

    // 5e. Cross-check {{key}} placeholders in prompts against [context.*] declarations
    {
        let placeholder_re = regex::Regex::new(r"\{\{(\w+)\}\}").unwrap();
        // Collect placeholders from the root prompt snippet
        let snippet_content =
            std::fs::read_to_string(skill_dir.join("system_prompt.md")).unwrap_or_default();
        let mut all_placeholders: std::collections::HashSet<String> =
            std::collections::HashSet::new();
        for cap in placeholder_re.captures_iter(&snippet_content) {
            all_placeholders.insert(cap.get(1).unwrap().as_str().to_string());
        }
        // Also check model-specific prompt variants
        if let Ok(rd) = std::fs::read_dir(skill_dir) {
            for dir_entry in rd.flatten() {
                let sub_path = dir_entry.path();
                if sub_path.is_dir() {
                    // Check provider/model subdirectories for system_prompt.md
                    if let Ok(sub_rd) = std::fs::read_dir(&sub_path) {
                        for sub_entry in sub_rd.flatten() {
                            let model_prompt = sub_entry.path().join("system_prompt.md");
                            if model_prompt.exists()
                                && let Ok(content) = std::fs::read_to_string(&model_prompt)
                            {
                                for cap in placeholder_re.captures_iter(&content) {
                                    all_placeholders
                                        .insert(cap.get(1).unwrap().as_str().to_string());
                                }
                            }
                        }
                    }
                    // Also check direct system_prompt.md in provider dir
                    let provider_prompt = sub_path.join("system_prompt.md");
                    if provider_prompt.exists()
                        && let Ok(content) = std::fs::read_to_string(&provider_prompt)
                    {
                        for cap in placeholder_re.captures_iter(&content) {
                            all_placeholders.insert(cap.get(1).unwrap().as_str().to_string());
                        }
                    }
                }
            }
        }
        // Placeholders without context declarations → Fail
        for ph in &all_placeholders {
            if !manifest.context.contains_key(ph) {
                diags.push(SkillDiagnostic::fail(format!(
                    "Prompt uses {{{{{}}}}} but no [context.{}] section declares it. \
                     Add [context.{}] to skill.toml or remove the placeholder.",
                    ph, ph, ph
                )));
            }
        }
        // Context declarations without placeholders → Warn
        for key in manifest.context.keys() {
            if !all_placeholders.contains(key) {
                diags.push(SkillDiagnostic::warn(format!(
                    "[context.{}] declared but no {{{{{}}}}} placeholder found in any prompt variant. \
                     The context will be fetched but never used.",
                    key, key
                )));
            }
        }
    }

    // 6. Check prompt snippet size against effective limit
    let snippet_path = skill_dir.join("system_prompt.md");
    if snippet_path.exists() {
        let effective_limit = manifest
            .skill
            .max_prompt_size
            .map(|v| v.min(MAX_PROMPT_SIZE_CEILING))
            .unwrap_or(MAX_PROMPT_SNIPPET_SIZE);

        if let Ok(meta) = std::fs::metadata(&snippet_path) {
            let size = meta.len();
            if size > effective_limit {
                if manifest.skill.always_on {
                    diags.push(SkillDiagnostic::fail(format!(
                        "system_prompt.md ({} bytes) exceeds limit ({} bytes) — skill will be SKIPPED at startup \
                         (always_on skills require their prompt to function)",
                        size, effective_limit
                    )));
                } else {
                    diags.push(SkillDiagnostic::fail(format!(
                        "system_prompt.md ({} bytes) exceeds limit ({} bytes) — prompt will be EMPTY at startup",
                        size, effective_limit
                    )));
                }
            } else if effective_limit > 0 && size > effective_limit * 3 / 4 {
                diags.push(SkillDiagnostic::warn(format!(
                    "system_prompt.md ({} bytes) is above 75% of limit ({} bytes)",
                    size, effective_limit
                )));
            } else {
                diags.push(SkillDiagnostic::ok(format!(
                    "system_prompt.md size OK ({} bytes, limit {} bytes)",
                    size, effective_limit
                )));
            }
        }

        if let Some(requested) = manifest.skill.max_prompt_size
            && requested > MAX_PROMPT_SIZE_CEILING
        {
            diags.push(SkillDiagnostic::warn(format!(
                "max_prompt_size ({} bytes) exceeds ceiling ({} bytes), will be clamped",
                requested, MAX_PROMPT_SIZE_CEILING
            )));
        }
    }

    // 7. Validate provider variant directories
    if let Ok(rd) = std::fs::read_dir(skill_dir) {
        for dir_entry in rd.flatten() {
            let sub_path = dir_entry.path();
            if !sub_path.is_dir() {
                continue;
            }
            let subdir_name = match sub_path.file_name().and_then(|n| n.to_str()) {
                Some(n) => n.to_string(),
                None => continue,
            };

            if subdir_name.parse::<ProviderKind>().is_ok() {
                // Known provider — validate its contents
                let has_override = sub_path.join("skill.toml").exists();
                let has_model_subdirs = std::fs::read_dir(&sub_path)
                    .map(|rd| {
                        rd.flatten().any(|e| {
                            e.path().is_dir()
                                && e.file_name().to_str().is_some_and(|n| !n.starts_with('.'))
                        })
                    })
                    .unwrap_or(false);

                // Warn if provider dir has system_prompt.md (no longer loaded)
                if sub_path.join("system_prompt.md").exists() {
                    diags.push(SkillDiagnostic::warn(format!(
                        "provider '{subdir_name}/system_prompt.md' is ignored — provider-level prompts are not supported. Use model-level variants instead (e.g., '{subdir_name}/<model>/system_prompt.md')"
                    )));
                }

                if !has_override && !has_model_subdirs {
                    diags.push(SkillDiagnostic::warn(format!(
                        "provider variant '{subdir_name}/' is empty (no skill.toml or model subdirectories)"
                    )));
                    continue;
                }

                let effective_limit = manifest
                    .skill
                    .max_prompt_size
                    .map(|v| v.min(MAX_PROMPT_SIZE_CEILING))
                    .unwrap_or(MAX_PROMPT_SNIPPET_SIZE);

                // Validate override parseability and check for identity fields
                if has_override {
                    let override_path = sub_path.join("skill.toml");
                    match std::fs::read_to_string(&override_path) {
                        Ok(content) => match toml::from_str::<ProviderSkillOverride>(&content) {
                            Ok(_) => {
                                diags.push(SkillDiagnostic::ok(format!(
                                    "provider '{subdir_name}/skill.toml' valid"
                                )));
                                // Warn if identity fields are present (they are silently ignored)
                                if let Ok(raw) = toml::from_str::<toml::Value>(&content) {
                                    if let Some(skill_table) =
                                        raw.get("skill").and_then(|v| v.as_table())
                                    {
                                        for field in &["name", "description"] {
                                            if skill_table.contains_key(*field) {
                                                diags.push(SkillDiagnostic::warn(format!(
                                                    "provider '{subdir_name}/skill.toml' contains identity field '{field}' which is ignored — identity fields cannot be overridden per-provider"
                                                )));
                                            }
                                        }
                                    }
                                    // [triggers] is a top-level section, not inside [skill]
                                    if raw.get("triggers").is_some() {
                                        diags.push(SkillDiagnostic::warn(format!(
                                            "provider '{subdir_name}/skill.toml' contains [triggers] section which is ignored — triggers cannot be overridden per-provider"
                                        )));
                                    }
                                    // [llm] belongs only in root skill.toml
                                    if raw.get("llm").is_some() {
                                        diags.push(SkillDiagnostic::warn(format!(
                                            "provider '{subdir_name}/skill.toml' contains [llm] section which is ignored — [llm] overrides belong in the root skill.toml only"
                                        )));
                                    }
                                }
                            }
                            Err(e) => {
                                diags.push(SkillDiagnostic::fail(format!(
                                    "provider '{subdir_name}/skill.toml' invalid: {e}"
                                )));
                            }
                        },
                        Err(e) => {
                            diags.push(SkillDiagnostic::fail(format!(
                                "cannot read provider '{subdir_name}/skill.toml': {e}"
                            )));
                        }
                    }
                }

                // Warn if provider subdir contains tools.json (not supported)
                if sub_path.join("tools.json").exists() {
                    diags.push(SkillDiagnostic::warn(format!(
                        "provider '{subdir_name}/tools.json' is not supported — tools cannot be overridden per-provider"
                    )));
                }

                // Validate model subdirectories within this provider
                if let Ok(model_rd) = std::fs::read_dir(&sub_path) {
                    for model_entry in model_rd.flatten() {
                        let model_path = model_entry.path();
                        if !model_path.is_dir() {
                            continue;
                        }
                        let model_name = match model_path.file_name().and_then(|n| n.to_str()) {
                            Some(n) => n.to_string(),
                            None => continue,
                        };
                        if model_name.starts_with('.') {
                            continue;
                        }

                        let model_has_prompt = model_path.join("system_prompt.md").exists();
                        let model_has_override = model_path.join("skill.toml").exists();

                        if !model_has_prompt && !model_has_override {
                            diags.push(SkillDiagnostic::warn(format!(
                                "model variant '{subdir_name}/{model_name}/' is empty (no system_prompt.md or skill.toml)"
                            )));
                            continue;
                        }

                        // Validate model prompt size
                        if model_has_prompt {
                            let model_prompt_path = model_path.join("system_prompt.md");
                            if let Ok(meta) = std::fs::metadata(&model_prompt_path) {
                                if meta.len() > effective_limit {
                                    diags.push(SkillDiagnostic::fail(format!(
                                        "model '{subdir_name}/{model_name}/system_prompt.md' ({} bytes) exceeds limit ({} bytes)",
                                        meta.len(), effective_limit
                                    )));
                                } else {
                                    diags.push(SkillDiagnostic::ok(format!(
                                        "model '{subdir_name}/{model_name}/system_prompt.md' size OK ({} bytes)",
                                        meta.len()
                                    )));
                                }
                            }
                        }

                        // Validate model override parseability and identity fields
                        if model_has_override {
                            let model_override_path = model_path.join("skill.toml");
                            match std::fs::read_to_string(&model_override_path) {
                                Ok(content) => {
                                    match toml::from_str::<ProviderSkillOverride>(&content) {
                                        Ok(_) => {
                                            diags.push(SkillDiagnostic::ok(format!(
                                                "model '{subdir_name}/{model_name}/skill.toml' valid"
                                            )));
                                            // Warn if identity fields are present
                                            if let Ok(raw) = toml::from_str::<toml::Value>(&content)
                                            {
                                                if let Some(skill_table) =
                                                    raw.get("skill").and_then(|v| v.as_table())
                                                {
                                                    for field in &["name", "description"] {
                                                        if skill_table.contains_key(*field) {
                                                            diags.push(SkillDiagnostic::warn(format!(
                                                                "model '{subdir_name}/{model_name}/skill.toml' contains identity field '{field}' which is ignored — identity fields cannot be overridden per-model"
                                                            )));
                                                        }
                                                    }
                                                }
                                                if raw.get("triggers").is_some() {
                                                    diags.push(SkillDiagnostic::warn(format!(
                                                        "model '{subdir_name}/{model_name}/skill.toml' contains [triggers] section which is ignored — triggers cannot be overridden per-model"
                                                    )));
                                                }
                                            }
                                        }
                                        Err(e) => {
                                            diags.push(SkillDiagnostic::fail(format!(
                                                "model '{subdir_name}/{model_name}/skill.toml' invalid: {e}"
                                            )));
                                        }
                                    }
                                }
                                Err(e) => {
                                    diags.push(SkillDiagnostic::fail(format!(
                                        "cannot read model '{subdir_name}/{model_name}/skill.toml': {e}"
                                    )));
                                }
                            }
                        }

                        // Warn if model subdir contains tools.json
                        if model_path.join("tools.json").exists() {
                            diags.push(SkillDiagnostic::warn(format!(
                                "model '{subdir_name}/{model_name}/tools.json' is not supported — tools cannot be overridden per-model"
                            )));
                        }

                        // Warn about unexpected nesting deeper than model level
                        if let Ok(deep_rd) = std::fs::read_dir(&model_path) {
                            for deep_entry in deep_rd.flatten() {
                                if deep_entry.path().is_dir() {
                                    let deep_name =
                                        deep_entry.file_name().to_string_lossy().to_string();
                                    if !deep_name.starts_with('.') {
                                        diags.push(SkillDiagnostic::warn(format!(
                                            "unexpected subdirectory '{subdir_name}/{model_name}/{deep_name}/' — only two levels of nesting supported (provider/model)"
                                        )));
                                    }
                                }
                            }
                        }

                        diags.push(SkillDiagnostic::ok(format!(
                            "model variant '{subdir_name}/{model_name}/' valid"
                        )));
                    }
                }
            } else {
                // Not a known provider — check for typos
                let known_names: Vec<&str> = ProviderKind::ALL
                    .iter()
                    .map(|p| p.config_prefix())
                    .collect();
                // Simple typo detection: check Levenshtein-like similarity
                for known in &known_names {
                    if looks_like_typo(&subdir_name, known) {
                        diags.push(SkillDiagnostic::warn(format!(
                            "subdirectory '{subdir_name}/' looks like a misspelling of provider '{known}'"
                        )));
                        break;
                    }
                }
            }
        }
    }

    // 8. Warnings for no-op or never-activates skills
    let has_tools = tools_path.exists();
    let has_snippet = snippet_path.exists();
    if !has_tools && !has_snippet {
        diags.push(SkillDiagnostic::warn(
            "no-op skill: no tools.json and no system_prompt.md",
        ));
    }
    if !manifest.skill.always_on && manifest.triggers.keywords.is_empty() {
        diags.push(SkillDiagnostic::warn(
            "skill will never activate: not always_on and no trigger keywords",
        ));
    }

    diags
}

/// Detect whether a skill.toml uses the legacy flat format.
///
/// Legacy format has a top-level `[handler]` section with a `type` field
/// (any handler type: builtin, exec, http). New format wraps skill metadata
/// under `[skill]` and puts handler config in tools.json per-tool.
fn is_legacy_format(content: &str) -> bool {
    // Parse as generic TOML table and check for legacy markers
    let table: toml::Table = match content.parse() {
        Ok(t) => t,
        Err(_) => return false,
    };
    // New format has [skill] section — never legacy
    if table.contains_key("skill") {
        return false;
    }
    // Legacy format has top-level "handler" table with any "type" field but no [skill]
    if let Some(handler) = table.get("handler").and_then(|v| v.as_table())
        && handler.get("type").and_then(|v| v.as_str()).is_some()
    {
        return true;
    }
    false
}

/// Simple typo detection: checks if two strings are close enough to be a misspelling.
/// Uses Levenshtein edit distance — two strings within edit distance 2 and
/// at least 4 characters long are considered potential typos.
fn looks_like_typo(input: &str, known: &str) -> bool {
    let a = input.to_lowercase();
    let b = known.to_lowercase();

    // Exact match is not a typo (it's a valid provider handled elsewhere)
    if a == b {
        return false;
    }

    // Too short — "foo" matches too many things
    if a.len() < 4 || b.len() < 4 {
        return false;
    }

    let len_diff = (a.len() as isize - b.len() as isize).unsigned_abs();
    if len_diff > 2 {
        return false;
    }

    // Compute Levenshtein distance
    let a_chars: Vec<char> = a.chars().collect();
    let b_chars: Vec<char> = b.chars().collect();
    let n = a_chars.len();
    let m = b_chars.len();

    let mut prev: Vec<usize> = (0..=m).collect();
    let mut curr = vec![0usize; m + 1];

    for i in 1..=n {
        curr[0] = i;
        for j in 1..=m {
            let cost = if a_chars[i - 1] == b_chars[j - 1] {
                0
            } else {
                1
            };
            curr[j] = (prev[j] + 1).min(curr[j - 1] + 1).min(prev[j - 1] + cost);
        }
        std::mem::swap(&mut prev, &mut curr);
    }

    prev[m] <= 2
}

/// Inject `work_item_id` as a required field into a tool's input schema.
///
/// Long-running exec handlers must track delegation via work items.
/// This adds the field to the JSON schema so the LLM knows to include it.
fn inject_work_item_id_field(schema: &mut serde_json::Value) {
    if let Some(props) = schema.get_mut("properties").and_then(|p| p.as_object_mut()) {
        props.insert(
            "work_item_id".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "ID of the work item tracking this task. Create one first using create_work_item."
            }),
        );
    }
    if let Some(required) = schema.get_mut("required").and_then(|r| r.as_array_mut()) {
        required.push(serde_json::Value::String("work_item_id".to_string()));
    } else {
        schema["required"] = serde_json::json!(["work_item_id"]);
    }
}

/// Result of scanning provider and model variant directories.
struct VariantScanResult {
    provider_overrides: HashMap<String, ProviderSkillFields>,
    model_prompts: HashMap<String, String>,
    model_overrides: HashMap<String, ProviderSkillFields>,
    generated_model_prompts: HashMap<String, String>,
}

/// Scan a skill directory for provider and model variant subdirectories.
///
/// Iterates over immediate subdirectories and checks if each name matches
/// a known `ProviderKind`. For matching directories, loads `skill.toml`
/// (as sparse override for timeout/max_prompt_size). Provider-level
/// `system_prompt.md` is intentionally not loaded. Then scans subdirectories
/// within each provider directory for model variants (both prompts and overrides).
fn scan_provider_variants(skill_dir: &Path, manifest: &SkillManifest) -> VariantScanResult {
    let mut overrides = HashMap::new();
    let mut model_prompts = HashMap::new();
    let mut model_overrides = HashMap::new();
    let generated_model_prompts = scan_generated_variants(skill_dir, manifest);

    let read_dir = match std::fs::read_dir(skill_dir) {
        Ok(rd) => rd,
        Err(_) => {
            return VariantScanResult {
                provider_overrides: overrides,
                model_prompts,
                model_overrides,
                generated_model_prompts,
            };
        }
    };

    let max_size = manifest
        .skill
        .max_prompt_size
        .map(|v| v.min(MAX_PROMPT_SIZE_CEILING))
        .unwrap_or(MAX_PROMPT_SNIPPET_SIZE);

    for dir_entry in read_dir.flatten() {
        let path = dir_entry.path();
        if !path.is_dir() {
            continue;
        }

        let subdir_name = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) => n.to_string(),
            None => continue,
        };

        // Only recognize subdirs that match a known ProviderKind
        if subdir_name.parse::<ProviderKind>().is_err() {
            continue;
        }

        let mut has_content = false;

        // Provider-level system_prompt.md is intentionally not loaded — models from
        // the same provider have different prompt requirements. Only model-level
        // prompts are supported. Provider directories hold overrides + model subdirs.

        // Load provider-specific skill.toml override
        let override_path = path.join("skill.toml");
        if override_path.exists() {
            match std::fs::read_to_string(&override_path) {
                Ok(content) => match toml::from_str::<ProviderSkillOverride>(&content) {
                    Ok(parsed) => {
                        overrides.insert(subdir_name.clone(), parsed.skill);
                        has_content = true;
                    }
                    Err(e) => {
                        warn!(
                            path = %override_path.display(),
                            provider = %subdir_name,
                            error = %e,
                            "malformed provider skill.toml override, skipping"
                        );
                    }
                },
                Err(e) => {
                    warn!(
                        path = %override_path.display(),
                        provider = %subdir_name,
                        error = %e,
                        "cannot read provider skill.toml override"
                    );
                }
            }
        }

        // Scan model subdirectories within this provider directory
        if let Ok(model_rd) = std::fs::read_dir(&path) {
            for model_entry in model_rd.flatten() {
                let model_path = model_entry.path();
                if !model_path.is_dir() {
                    continue;
                }
                let model_name = match model_path.file_name().and_then(|n| n.to_str()) {
                    Some(n) => n.to_string(),
                    None => continue,
                };
                // Skip dotfiles/dotdirs
                if model_name.starts_with('.') {
                    continue;
                }
                let composite_key = format!("{}/{}", subdir_name, model_name);
                let mut model_has_content = false;

                // Load model-specific prompt
                let model_prompt_path = model_path.join("system_prompt.md");
                if model_prompt_path.exists() {
                    match load_snippet_with_limit(&model_prompt_path, max_size) {
                        SnippetLoadResult::Ok(content) => {
                            model_prompts.insert(composite_key.clone(), content);
                            model_has_content = true;
                        }
                        SnippetLoadResult::Oversized { size, limit } => {
                            warn!(
                                path = %model_prompt_path.display(),
                                size,
                                limit,
                                "model variant prompt exceeds size limit — falling back to root prompt"
                            );
                        }
                        SnippetLoadResult::ReadError(e) => {
                            warn!(
                                path = %model_prompt_path.display(),
                                error = %e,
                                "cannot read model variant prompt — falling back to root prompt"
                            );
                        }
                        SnippetLoadResult::Empty => {}
                    }
                }

                // Load model-specific skill.toml override
                let model_override_path = model_path.join("skill.toml");
                if model_override_path.exists() {
                    match std::fs::read_to_string(&model_override_path) {
                        Ok(content) => match toml::from_str::<ProviderSkillOverride>(&content) {
                            Ok(parsed) => {
                                model_overrides.insert(composite_key.clone(), parsed.skill);
                                model_has_content = true;
                            }
                            Err(e) => {
                                warn!(
                                    path = %model_override_path.display(),
                                    provider = %subdir_name,
                                    model = %model_name,
                                    error = %e,
                                    "malformed model skill.toml override, skipping"
                                );
                            }
                        },
                        Err(e) => {
                            warn!(
                                path = %model_override_path.display(),
                                provider = %subdir_name,
                                model = %model_name,
                                error = %e,
                                "cannot read model skill.toml override"
                            );
                        }
                    }
                }

                if !model_has_content {
                    warn!(
                        skill = %manifest.skill.name,
                        provider = %subdir_name,
                        model = %model_name,
                        "model variant directory is empty (no system_prompt.md or skill.toml)"
                    );
                } else {
                    has_content = true;
                }
            }
        }

        if !has_content {
            warn!(
                skill = %manifest.skill.name,
                provider = %subdir_name,
                "provider variant directory is empty (no skill.toml overrides or model subdirectories)"
            );
        }
    }

    VariantScanResult {
        provider_overrides: overrides,
        model_prompts,
        model_overrides,
        generated_model_prompts,
    }
}

/// Scan `<skill_dir>/generated/<provider>/<model>/system_prompt.md` files.
///
/// These are written by the `review_skill` builtin (when called with a
/// `content` argument) at runtime — the
/// `generated/` segment is hard-coded so the agent cannot move writes outside
/// it. Generated variants are loaded into a separate map from hand-authored
/// variants so resolution can prefer hand-authored content.
fn scan_generated_variants(skill_dir: &Path, manifest: &SkillManifest) -> HashMap<String, String> {
    let mut out = HashMap::new();

    let generated_root = skill_dir.join("generated");
    let max_size = manifest
        .skill
        .max_prompt_size
        .map(|v| v.min(MAX_PROMPT_SIZE_CEILING))
        .unwrap_or(MAX_PROMPT_SNIPPET_SIZE);

    let provider_dirs = match std::fs::read_dir(&generated_root) {
        Ok(rd) => rd,
        Err(_) => return out,
    };

    for provider_entry in provider_dirs.flatten() {
        let provider_path = provider_entry.path();
        // Defense in depth: skip symlinked provider directories. The
        // `generated/` subtree is mika-owned and should never contain
        // symlinks; refusing to traverse one prevents an external write
        // from redirecting reads outside the skill tree.
        if !provider_path.is_dir()
            || std::fs::symlink_metadata(&provider_path)
                .map(|m| m.file_type().is_symlink())
                .unwrap_or(true)
        {
            continue;
        }
        let provider_name = match provider_path.file_name().and_then(|n| n.to_str()) {
            Some(n) if !n.starts_with('.') => n.to_string(),
            _ => continue,
        };
        // Recognise only known providers — same gate the hand-authored scan applies.
        if provider_name.parse::<ProviderKind>().is_err() {
            continue;
        }

        let model_dirs = match std::fs::read_dir(&provider_path) {
            Ok(rd) => rd,
            Err(_) => continue,
        };

        for model_entry in model_dirs.flatten() {
            let model_path = model_entry.path();
            if !model_path.is_dir()
                || std::fs::symlink_metadata(&model_path)
                    .map(|m| m.file_type().is_symlink())
                    .unwrap_or(true)
            {
                continue;
            }
            let model_name = match model_path.file_name().and_then(|n| n.to_str()) {
                Some(n) if !n.starts_with('.') => n.to_string(),
                _ => continue,
            };

            let prompt_path = model_path.join("system_prompt.md");
            match load_snippet_with_limit(&prompt_path, max_size) {
                SnippetLoadResult::Ok(content) => {
                    let key = format!("{provider_name}/{model_name}");
                    out.insert(key, content);
                }
                SnippetLoadResult::Oversized { size, limit } => {
                    warn!(
                        skill = %manifest.skill.name,
                        path = %prompt_path.display(),
                        size,
                        limit,
                        "generated variant prompt exceeds size limit — skipping"
                    );
                }
                SnippetLoadResult::ReadError(e) => {
                    warn!(
                        skill = %manifest.skill.name,
                        path = %prompt_path.display(),
                        error = %e,
                        "cannot read generated variant prompt — skipping"
                    );
                }
                SnippetLoadResult::Empty => {}
            }
        }
    }

    out
}

/// Load and parse `tools.json` from a skill directory.
///
/// Returns an empty vec if the file doesn't exist or is invalid.
fn load_tools_json(skill_dir: &Path) -> Vec<ResolvedSkillTool> {
    let tools_path = skill_dir.join("tools.json");

    // Check file size
    if let Ok(meta) = std::fs::metadata(&tools_path)
        && meta.len() > MAX_TOOLS_JSON_SIZE
    {
        warn!(
            path = %tools_path.display(),
            size = meta.len(),
            "tools.json exceeds 256KB, skipping"
        );
        return Vec::new();
    }

    let content = match std::fs::read_to_string(&tools_path) {
        Ok(c) => c,
        Err(_) => return Vec::new(), // File doesn't exist — normal for prompt-only skills
    };

    let tool_defs: Vec<SkillToolDef> = match serde_json::from_str(&content) {
        Ok(defs) => defs,
        Err(e) => {
            warn!(path = %tools_path.display(), error = %e, "invalid tools.json");
            return Vec::new();
        }
    };

    tool_defs
        .into_iter()
        .filter(|def| {
            if let ToolHandler::Builtin { function } = &def.handler
                && !KNOWN_BUILTINS.contains(&function.as_str())
            {
                warn!(
                    path = %tools_path.display(),
                    function = %function,
                    tool = %def.name,
                    "unknown builtin function, skipping tool"
                );
                return false;
            }
            true
        })
        .map(|def| {
            let mut schema = def.input_schema;

            // Long-running exec handlers require a work_item_id for delegation tracking
            if let ToolHandler::Exec {
                long_running: true, ..
            } = &def.handler
            {
                inject_work_item_id_field(&mut schema);
            }

            ResolvedSkillTool {
                definition: ToolDefinition {
                    name: def.name,
                    description: def.description,
                    input_schema: schema,
                },
                handler: def.handler,
                skill_dir: skill_dir.to_path_buf(),
            }
        })
        .collect()
}

/// Result of loading a prompt snippet file.
#[derive(Debug)]
pub enum SnippetLoadResult {
    /// Successfully loaded the prompt content.
    Ok(String),
    /// File does not exist or is empty (legitimate — tool-only skills).
    Empty,
    /// File exceeds the configured size limit.
    Oversized { size: u64, limit: u64 },
    /// IO error reading the file.
    ReadError(String),
}

/// Load a prompt snippet file with size limit enforcement.
fn load_snippet_with_limit(path: &Path, max_size: u64) -> SnippetLoadResult {
    let meta = match std::fs::metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return SnippetLoadResult::Empty,
        Err(e) => return SnippetLoadResult::ReadError(e.to_string()),
    };

    if meta.len() > max_size {
        return SnippetLoadResult::Oversized {
            size: meta.len(),
            limit: max_size,
        };
    }

    match std::fs::read_to_string(path) {
        Ok(content) if content.is_empty() => SnippetLoadResult::Empty,
        Ok(content) => SnippetLoadResult::Ok(content),
        Err(e) => SnippetLoadResult::ReadError(e.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_scan_valid_skills() {
        let tmp = tempfile::tempdir().unwrap();
        let skill_dir = tmp.path().join("web-search");
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(
            skill_dir.join("skill.toml"),
            r#"
            [skill]
            name = "web-search"
            description = "Search the web"
            version = "0.1.0"

            [triggers]
            keywords = ["Search", "LOOK UP"]
            "#,
        )
        .unwrap();

        let scan = scan_skills_dir(tmp.path());
        assert_eq!(scan.entries.len(), 1);
        assert_eq!(scan.skipped.len(), 0);
        assert_eq!(scan.entries[0].manifest.skill.name, "web-search");
        assert_eq!(scan.entries[0].keywords_lower, vec!["search", "look up"]);
        assert_eq!(scan.entries[0].dir, skill_dir);
        assert!(scan.entries[0].enabled);
        assert!(scan.entries[0].skill_tools.is_empty());
    }

    #[test]
    fn test_scan_skips_legacy_format() {
        let tmp = tempfile::tempdir().unwrap();

        // Legacy format skill (should be skipped)
        let legacy = tmp.path().join("memory");
        fs::create_dir_all(&legacy).unwrap();
        fs::write(
            legacy.join("skill.toml"),
            r#"
            name = "memory"
            description = "Memory tools"
            [triggers]
            keywords = ["remember"]
            [handler]
            type = "builtin"
            tools = ["store_fact"]
            [options]
            always_on = true
            "#,
        )
        .unwrap();

        // New format skill (should be loaded)
        let new_skill = tmp.path().join("web-search");
        fs::create_dir_all(&new_skill).unwrap();
        fs::write(
            new_skill.join("skill.toml"),
            r#"
            [skill]
            name = "web-search"
            description = "Search the web"
            "#,
        )
        .unwrap();

        let scan = scan_skills_dir(tmp.path());
        assert_eq!(scan.entries.len(), 1);
        assert_eq!(scan.skipped.len(), 1);
        assert_eq!(scan.entries[0].manifest.skill.name, "web-search");
    }

    #[test]
    fn test_scan_skips_invalid_manifests() {
        let tmp = tempfile::tempdir().unwrap();

        // Valid skill
        let valid = tmp.path().join("good");
        fs::create_dir_all(&valid).unwrap();
        fs::write(
            valid.join("skill.toml"),
            r#"
            [skill]
            name = "good"
            description = "Valid"
            "#,
        )
        .unwrap();

        // Invalid skill (bad TOML)
        let bad = tmp.path().join("bad");
        fs::create_dir_all(&bad).unwrap();
        fs::write(bad.join("skill.toml"), "this is not valid toml {{{}}}").unwrap();

        // Missing manifest
        let missing = tmp.path().join("missing");
        fs::create_dir_all(&missing).unwrap();

        let scan = scan_skills_dir(tmp.path());
        assert_eq!(scan.entries.len(), 1);
        assert_eq!(scan.skipped.len(), 2); // bad TOML + missing manifest
        assert_eq!(scan.entries[0].manifest.skill.name, "good");

        // Verify skipped details capture names and reasons
        let skipped_names: Vec<&str> = scan.skipped.iter().map(|s| s.name.as_str()).collect();
        assert!(
            skipped_names.contains(&"bad"),
            "should record 'bad' as skipped: {skipped_names:?}"
        );
        assert!(
            skipped_names.contains(&"missing"),
            "should record 'missing' as skipped: {skipped_names:?}"
        );
        let bad_entry = scan.skipped.iter().find(|s| s.name == "bad").unwrap();
        assert!(
            bad_entry.reason.contains("invalid TOML"),
            "bad skill should have TOML parse error reason: {}",
            bad_entry.reason
        );
        let missing_entry = scan.skipped.iter().find(|s| s.name == "missing").unwrap();
        assert!(
            missing_entry.reason.contains("cannot read manifest"),
            "missing skill should have read error reason: {}",
            missing_entry.reason
        );
    }

    #[test]
    fn test_scan_empty_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let scan = scan_skills_dir(tmp.path());
        assert!(scan.entries.is_empty());
    }

    #[test]
    fn test_scan_nonexistent_dir() {
        let scan = scan_skills_dir(Path::new("/nonexistent/skills"));
        assert!(scan.entries.is_empty());
    }

    #[test]
    fn test_scan_ignores_files() {
        let tmp = tempfile::tempdir().unwrap();
        // A file (not a directory) in the skills dir should be skipped
        fs::write(tmp.path().join("readme.txt"), "not a skill").unwrap();
        let scan = scan_skills_dir(tmp.path());
        assert!(scan.entries.is_empty());
    }

    #[test]
    fn test_scan_skips_oversized_manifest() {
        let tmp = tempfile::tempdir().unwrap();
        let skill_dir = tmp.path().join("big");
        fs::create_dir_all(&skill_dir).unwrap();
        // Write a file larger than 64KB
        let big_content = "x".repeat(65 * 1024);
        fs::write(skill_dir.join("skill.toml"), &big_content).unwrap();

        let scan = scan_skills_dir(tmp.path());
        assert!(scan.entries.is_empty());
        assert_eq!(scan.skipped.len(), 1);
    }

    #[test]
    fn test_scan_loads_prompt_snippet() {
        let tmp = tempfile::tempdir().unwrap();
        let skill_dir = tmp.path().join("web-search");
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(
            skill_dir.join("skill.toml"),
            r#"
            [skill]
            name = "web-search"
            description = "Search the web"
            "#,
        )
        .unwrap();
        fs::write(skill_dir.join("system_prompt.md"), "Use web search wisely.").unwrap();

        let scan = scan_skills_dir(tmp.path());
        assert_eq!(scan.entries.len(), 1);
        assert_eq!(scan.entries[0].prompt_snippet, "Use web search wisely.");
    }

    #[test]
    fn test_scan_missing_prompt_snippet_defaults_empty() {
        let tmp = tempfile::tempdir().unwrap();
        let skill_dir = tmp.path().join("web-search");
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(
            skill_dir.join("skill.toml"),
            r#"
            [skill]
            name = "web-search"
            description = "Search the web"
            "#,
        )
        .unwrap();

        let scan = scan_skills_dir(tmp.path());
        assert_eq!(scan.entries.len(), 1);
        assert_eq!(scan.entries[0].prompt_snippet, "");
    }

    #[test]
    fn test_snippet_size_limit_default() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("system_prompt.md");
        // Write a file larger than 16KB default
        let big_content = "x".repeat(17 * 1024);
        fs::write(&path, &big_content).unwrap();

        let result = load_snippet_with_limit(&path, MAX_PROMPT_SNIPPET_SIZE);
        assert!(matches!(result, SnippetLoadResult::Oversized { .. }));
    }

    #[test]
    fn test_snippet_size_limit_custom() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("system_prompt.md");
        // 10KB file — under 16KB default, tested with explicit 32KB limit
        let content = "x".repeat(10 * 1024);
        fs::write(&path, &content).unwrap();

        let result = load_snippet_with_limit(&path, 32 * 1024);
        match result {
            SnippetLoadResult::Ok(s) => assert_eq!(s.len(), 10 * 1024),
            other => panic!("expected Ok, got {:?}", other),
        }
    }

    #[test]
    fn test_snippet_size_limit_zero_always_skips() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("system_prompt.md");
        fs::write(&path, "tiny").unwrap();

        let result = load_snippet_with_limit(&path, 0);
        assert!(matches!(result, SnippetLoadResult::Oversized { .. }));
    }

    #[test]
    fn test_snippet_under_default_limit_loads() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("system_prompt.md");
        let content = "x".repeat(15 * 1024); // 15KB, under 16KB default
        fs::write(&path, &content).unwrap();

        let result = load_snippet_with_limit(&path, MAX_PROMPT_SNIPPET_SIZE);
        match result {
            SnippetLoadResult::Ok(s) => assert_eq!(s.len(), 15 * 1024),
            other => panic!("expected Ok, got {:?}", other),
        }
    }

    #[test]
    fn test_snippet_missing_file_returns_empty() {
        let result = load_snippet_with_limit(Path::new("/nonexistent/prompt.md"), 16 * 1024);
        assert!(matches!(result, SnippetLoadResult::Empty));
    }

    #[test]
    fn test_snippet_empty_file_returns_empty() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("system_prompt.md");
        fs::write(&path, "").unwrap();

        let result = load_snippet_with_limit(&path, 16 * 1024);
        assert!(matches!(result, SnippetLoadResult::Empty));
    }

    #[test]
    fn test_scan_loads_large_snippet_with_override() {
        let tmp = tempfile::tempdir().unwrap();
        let skill_dir = tmp.path().join("big-prompt");
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(
            skill_dir.join("skill.toml"),
            r#"
            [skill]
            name = "big-prompt"
            description = "Skill with large prompt"
            max_prompt_size = 32768
            "#,
        )
        .unwrap();
        // 20KB prompt — over 16KB default but under 32KB override
        let content = "x".repeat(20 * 1024);
        fs::write(skill_dir.join("system_prompt.md"), &content).unwrap();

        let scan = scan_skills_dir(tmp.path());
        assert_eq!(scan.entries.len(), 1);
        assert_eq!(scan.entries[0].prompt_snippet.len(), 20 * 1024);
    }

    #[test]
    fn test_scan_clamps_override_to_ceiling() {
        let tmp = tempfile::tempdir().unwrap();
        let skill_dir = tmp.path().join("huge-prompt");
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(
            skill_dir.join("skill.toml"),
            r#"
            [skill]
            name = "huge-prompt"
            description = "Skill requesting too much"
            max_prompt_size = 1048576
            "#,
        )
        .unwrap();
        // 100KB prompt — over 64KB ceiling
        let content = "x".repeat(100 * 1024);
        fs::write(skill_dir.join("system_prompt.md"), &content).unwrap();

        let scan = scan_skills_dir(tmp.path());
        assert_eq!(scan.entries.len(), 1);
        // Should be empty because 100KB > 64KB ceiling
        assert_eq!(scan.entries[0].prompt_snippet, "");
    }

    #[test]
    fn test_scan_skips_snippet_over_default() {
        let tmp = tempfile::tempdir().unwrap();
        let skill_dir = tmp.path().join("too-big");
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(
            skill_dir.join("skill.toml"),
            r#"
            [skill]
            name = "too-big"
            description = "Prompt over default limit"
            "#,
        )
        .unwrap();
        // 17KB prompt — over 16KB default
        let content = "x".repeat(17 * 1024);
        fs::write(skill_dir.join("system_prompt.md"), &content).unwrap();

        let scan = scan_skills_dir(tmp.path());
        assert_eq!(scan.entries.len(), 1);
        assert_eq!(scan.entries[0].prompt_snippet, "");
    }

    #[test]
    fn test_scan_skips_always_on_skill_with_oversized_prompt() {
        let tmp = tempfile::tempdir().unwrap();
        let skill_dir = tmp.path().join("self-dev");
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(
            skill_dir.join("skill.toml"),
            r#"
            [skill]
            name = "self-dev"
            description = "Development workflow"
            always_on = true
            "#,
        )
        .unwrap();
        // 29KB prompt — over 16KB default limit
        let content = "x".repeat(29 * 1024);
        fs::write(skill_dir.join("system_prompt.md"), &content).unwrap();

        let scan = scan_skills_dir(tmp.path());
        // always_on skill with oversized prompt should be SKIPPED entirely
        assert_eq!(scan.entries.len(), 0);
        assert_eq!(scan.skipped.len(), 1);
    }

    #[test]
    fn test_scan_always_on_with_valid_prompt_loads() {
        let tmp = tempfile::tempdir().unwrap();
        let skill_dir = tmp.path().join("memory");
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(
            skill_dir.join("skill.toml"),
            r#"
            [skill]
            name = "memory"
            description = "Memory management"
            always_on = true
            "#,
        )
        .unwrap();
        fs::write(skill_dir.join("system_prompt.md"), "Remember things.").unwrap();

        let scan = scan_skills_dir(tmp.path());
        assert_eq!(scan.entries.len(), 1);
        assert_eq!(scan.entries[0].prompt_snippet, "Remember things.");
        assert_eq!(scan.skipped.len(), 0);
    }

    #[test]
    fn test_scan_always_on_with_custom_size_loads_large_prompt() {
        let tmp = tempfile::tempdir().unwrap();
        let skill_dir = tmp.path().join("self-dev");
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(
            skill_dir.join("skill.toml"),
            r#"
            [skill]
            name = "self-dev"
            description = "Development workflow"
            always_on = true
            max_prompt_size = 65536
            "#,
        )
        .unwrap();
        // 29KB prompt — over 16KB default but under 64KB ceiling
        let content = "x".repeat(29 * 1024);
        fs::write(skill_dir.join("system_prompt.md"), &content).unwrap();

        let scan = scan_skills_dir(tmp.path());
        assert_eq!(scan.entries.len(), 1);
        assert_eq!(scan.entries[0].prompt_snippet.len(), 29 * 1024);
        assert_eq!(scan.skipped.len(), 0);
    }

    #[test]
    fn test_scan_always_on_without_prompt_file_loads() {
        // Tool-only always_on skills (no prompt file) should still load
        let tmp = tempfile::tempdir().unwrap();
        let skill_dir = tmp.path().join("agents-teams");
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(
            skill_dir.join("skill.toml"),
            r#"
            [skill]
            name = "agents-teams"
            description = "Agent management"
            always_on = true
            "#,
        )
        .unwrap();
        // No system_prompt.md — tool-only skill

        let scan = scan_skills_dir(tmp.path());
        assert_eq!(scan.entries.len(), 1);
        assert_eq!(scan.entries[0].prompt_snippet, "");
        assert_eq!(scan.skipped.len(), 0);
    }

    #[test]
    fn test_scan_non_always_on_with_oversized_prompt_still_loads() {
        // Non-always_on skills should still load with empty prompt (existing behavior)
        let tmp = tempfile::tempdir().unwrap();
        let skill_dir = tmp.path().join("optional");
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(
            skill_dir.join("skill.toml"),
            r#"
            [skill]
            name = "optional"
            description = "Optional skill"
            "#,
        )
        .unwrap();
        let content = "x".repeat(17 * 1024);
        fs::write(skill_dir.join("system_prompt.md"), &content).unwrap();

        let scan = scan_skills_dir(tmp.path());
        assert_eq!(scan.entries.len(), 1);
        assert_eq!(scan.entries[0].prompt_snippet, "");
        assert_eq!(scan.skipped.len(), 0);
    }

    #[test]
    fn test_disabled_skill() {
        let tmp = tempfile::tempdir().unwrap();
        let skill_dir = tmp.path().join("web-search");
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(
            skill_dir.join("skill.toml"),
            r#"
            [skill]
            name = "web-search"
            description = "Search the web"
            "#,
        )
        .unwrap();
        // Create .disabled marker
        fs::write(skill_dir.join(".disabled"), "").unwrap();

        let scan = scan_skills_dir(tmp.path());
        assert_eq!(scan.entries.len(), 1);
        assert!(!scan.entries[0].enabled);
    }

    #[test]
    fn test_tools_json_loading() {
        let tmp = tempfile::tempdir().unwrap();
        let skill_dir = tmp.path().join("web-search");
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(
            skill_dir.join("skill.toml"),
            r#"
            [skill]
            name = "web-search"
            description = "Search the web"
            "#,
        )
        .unwrap();
        fs::write(
            skill_dir.join("tools.json"),
            r#"[{
                "name": "web_search",
                "description": "Search the web for information",
                "input_schema": {
                    "type": "object",
                    "properties": {
                        "query": {"type": "string", "description": "Search query"}
                    },
                    "required": ["query"]
                },
                "handler": {"type": "exec", "command": "./handlers/search.sh"}
            }]"#,
        )
        .unwrap();

        let scan = scan_skills_dir(tmp.path());
        assert_eq!(scan.entries.len(), 1);
        assert_eq!(scan.entries[0].skill_tools.len(), 1);
        assert_eq!(scan.entries[0].skill_tools[0].definition.name, "web_search");
        assert_eq!(scan.entries[0].skill_tools[0].skill_dir, skill_dir);
        assert!(matches!(
            &scan.entries[0].skill_tools[0].handler,
            super::super::manifest::ToolHandler::Exec { command, .. } if command == "./handlers/search.sh"
        ));
    }

    #[test]
    fn test_tools_json_oversized() {
        let tmp = tempfile::tempdir().unwrap();
        let skill_dir = tmp.path().join("big-tools");
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(
            skill_dir.join("skill.toml"),
            r#"
            [skill]
            name = "big-tools"
            description = "Oversized tools"
            "#,
        )
        .unwrap();
        let big_json = "x".repeat(257 * 1024);
        fs::write(skill_dir.join("tools.json"), &big_json).unwrap();

        let scan = scan_skills_dir(tmp.path());
        assert_eq!(scan.entries.len(), 1);
        assert!(scan.entries[0].skill_tools.is_empty());
    }

    #[test]
    fn test_tools_json_invalid() {
        let tmp = tempfile::tempdir().unwrap();
        let skill_dir = tmp.path().join("bad-tools");
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(
            skill_dir.join("skill.toml"),
            r#"
            [skill]
            name = "bad-tools"
            description = "Invalid tools"
            "#,
        )
        .unwrap();
        fs::write(skill_dir.join("tools.json"), "not json").unwrap();

        let scan = scan_skills_dir(tmp.path());
        assert_eq!(scan.entries.len(), 1);
        assert!(scan.entries[0].skill_tools.is_empty());
    }

    #[test]
    fn test_tools_json_unknown_builtin_skipped() {
        let tmp = tempfile::tempdir().unwrap();
        let skill_dir = tmp.path().join("bad-builtin");
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(
            skill_dir.join("skill.toml"),
            r#"
            [skill]
            name = "bad-builtin"
            description = "Has unknown builtin"
            "#,
        )
        .unwrap();
        fs::write(
            skill_dir.join("tools.json"),
            r#"[
                {
                    "name": "valid_tool",
                    "description": "Valid builtin",
                    "input_schema": {"type": "object", "properties": {}},
                    "handler": {"type": "builtin", "function": "get_documentation"}
                },
                {
                    "name": "bad_tool",
                    "description": "Unknown builtin",
                    "input_schema": {"type": "object", "properties": {}},
                    "handler": {"type": "builtin", "function": "get_clii_reference"}
                }
            ]"#,
        )
        .unwrap();

        let scan = scan_skills_dir(tmp.path());
        assert_eq!(scan.entries.len(), 1);
        assert_eq!(
            scan.entries[0].skill_tools.len(),
            1,
            "unknown builtin should be filtered out"
        );
        assert_eq!(scan.entries[0].skill_tools[0].definition.name, "valid_tool");
    }

    #[test]
    fn test_is_legacy_format() {
        // Legacy builtin handler
        assert!(is_legacy_format(
            r#"
            name = "memory"
            description = "Memory"
            [handler]
            type = "builtin"
            tools = ["store_fact"]
            "#
        ));

        // Legacy exec handler (also detected now)
        assert!(is_legacy_format(
            r#"
            name = "weather"
            description = "Weather"
            [handler]
            type = "exec"
            command = "./handler.sh"
            tools = ["get_forecast"]
            "#
        ));

        // Legacy http handler
        assert!(is_legacy_format(
            r#"
            name = "weather"
            description = "Weather"
            [handler]
            type = "http"
            url = "http://localhost:8080/tools"
            "#
        ));

        // New format is NOT legacy
        assert!(!is_legacy_format(
            r#"
            [skill]
            name = "web-search"
            description = "Search"
            "#
        ));

        // New format with [handler] section is NOT legacy (has [skill])
        assert!(!is_legacy_format(
            r#"
            [skill]
            name = "qa-review"
            description = "QA review with exec handler"
            version = "0.1.0"

            [handler]
            type = "exec"
            command = "./run.sh"
            "#
        ));

        // Empty [skill] section with [handler] is still NOT legacy
        assert!(!is_legacy_format(
            r#"
            [skill]

            [handler]
            type = "exec"
            command = "./run.sh"
            "#
        ));

        // Invalid TOML is not legacy
        assert!(!is_legacy_format("{{not toml}}"));
    }

    #[test]
    fn test_long_running_tool_gets_work_item_id_injected() {
        let tmp = tempfile::tempdir().unwrap();
        let skill_dir = tmp.path().join("builder");
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(
            skill_dir.join("skill.toml"),
            r#"
            [skill]
            name = "builder"
            description = "Long-running builder"
            "#,
        )
        .unwrap();
        fs::write(
            skill_dir.join("tools.json"),
            r#"[{
                "name": "build_project",
                "description": "Build a project",
                "input_schema": {
                    "type": "object",
                    "properties": {
                        "command": {"type": "string", "description": "Build command"}
                    },
                    "required": ["command"]
                },
                "handler": {"type": "exec", "command": "./build.sh", "long_running": true, "estimated_duration_secs": 300}
            }]"#,
        )
        .unwrap();

        let result = scan_skills_dir(tmp.path());
        assert_eq!(result.entries.len(), 1);
        assert_eq!(result.entries[0].skill_tools.len(), 1);

        let schema = &result.entries[0].skill_tools[0].definition.input_schema;
        // work_item_id should be in properties
        assert!(
            schema["properties"]["work_item_id"].is_object(),
            "work_item_id property should be injected for long_running tools"
        );
        // work_item_id should be required
        let required = schema["required"].as_array().unwrap();
        assert!(
            required.contains(&serde_json::Value::String("work_item_id".to_string())),
            "work_item_id should be in required fields"
        );
    }

    #[test]
    fn test_validate_skill_prompt_size_ok() {
        let tmp = tempfile::tempdir().unwrap();
        let skill_dir = tmp.path().join("small-prompt");
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(
            skill_dir.join("skill.toml"),
            r#"
            [skill]
            name = "small-prompt"
            description = "Small prompt skill"
            "#,
        )
        .unwrap();
        fs::write(skill_dir.join("system_prompt.md"), "Small prompt.").unwrap();

        let diags = validate_skill(&skill_dir);
        let prompt_diag = diags
            .iter()
            .find(|d| d.message.contains("system_prompt.md size OK"));
        assert!(prompt_diag.is_some());
    }

    #[test]
    fn test_validate_skill_prompt_oversized() {
        let tmp = tempfile::tempdir().unwrap();
        let skill_dir = tmp.path().join("big-prompt");
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(
            skill_dir.join("skill.toml"),
            r#"
            [skill]
            name = "big-prompt"
            description = "Big prompt skill"
            "#,
        )
        .unwrap();
        // 17KB — over 16KB default
        fs::write(skill_dir.join("system_prompt.md"), "x".repeat(17 * 1024)).unwrap();

        let diags = validate_skill(&skill_dir);
        let fail_diag = diags
            .iter()
            .find(|d| d.message.contains("exceeds limit") && d.level == DiagnosticLevel::Fail);
        assert!(fail_diag.is_some());
    }

    #[test]
    fn test_validate_skill_prompt_near_limit_warns() {
        let tmp = tempfile::tempdir().unwrap();
        let skill_dir = tmp.path().join("near-limit");
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(
            skill_dir.join("skill.toml"),
            r#"
            [skill]
            name = "near-limit"
            description = "Near limit prompt"
            "#,
        )
        .unwrap();
        // 13KB — above 75% of 16KB (12288) but under 16KB
        fs::write(skill_dir.join("system_prompt.md"), "x".repeat(13 * 1024)).unwrap();

        let diags = validate_skill(&skill_dir);
        let warn_diag = diags
            .iter()
            .find(|d| d.message.contains("above 75%") && d.level == DiagnosticLevel::Warn);
        assert!(warn_diag.is_some());
    }

    #[test]
    fn test_validate_skill_prompt_with_override() {
        let tmp = tempfile::tempdir().unwrap();
        let skill_dir = tmp.path().join("override-prompt");
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(
            skill_dir.join("skill.toml"),
            r#"
            [skill]
            name = "override-prompt"
            description = "Skill with override"
            max_prompt_size = 32768
            "#,
        )
        .unwrap();
        // 20KB — over 16KB default but under 32KB override
        fs::write(skill_dir.join("system_prompt.md"), "x".repeat(20 * 1024)).unwrap();

        let diags = validate_skill(&skill_dir);
        // Should NOT have a fail diagnostic — 20KB is under 32KB override
        let fail_diag = diags.iter().find(|d| d.message.contains("exceeds limit"));
        assert!(fail_diag.is_none());
        // Should have an OK diagnostic
        let ok_diag = diags
            .iter()
            .find(|d| d.message.contains("system_prompt.md size OK"));
        assert!(ok_diag.is_some());
    }

    #[test]
    fn test_non_long_running_tool_no_work_item_id() {
        let tmp = tempfile::tempdir().unwrap();
        let skill_dir = tmp.path().join("search");
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(
            skill_dir.join("skill.toml"),
            r#"
            [skill]
            name = "search"
            description = "Web search"
            "#,
        )
        .unwrap();
        fs::write(
            skill_dir.join("tools.json"),
            r#"[{
                "name": "web_search",
                "description": "Search the web",
                "input_schema": {
                    "type": "object",
                    "properties": {
                        "query": {"type": "string"}
                    },
                    "required": ["query"]
                },
                "handler": {"type": "exec", "command": "./search.sh"}
            }]"#,
        )
        .unwrap();

        let result = scan_skills_dir(tmp.path());
        assert_eq!(result.entries.len(), 1);
        assert_eq!(result.entries[0].skill_tools.len(), 1);

        let schema = &result.entries[0].skill_tools[0].definition.input_schema;
        // work_item_id should NOT be injected for non-long-running tools
        assert!(
            schema["properties"]["work_item_id"].is_null(),
            "work_item_id should not be injected for non-long_running tools"
        );
    }

    // -- Provider variant tests --

    #[test]
    fn test_scan_provider_prompt_ignored() {
        let tmp = tempfile::tempdir().unwrap();
        let skill_dir = tmp.path().join("web-search");
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(
            skill_dir.join("skill.toml"),
            r#"
            [skill]
            name = "web-search"
            description = "Search the web"
            "#,
        )
        .unwrap();
        fs::write(skill_dir.join("system_prompt.md"), "Root prompt.").unwrap();

        // Create anthropic variant with only a system_prompt.md (no longer loaded)
        let anthropic_dir = skill_dir.join("anthropic");
        fs::create_dir_all(&anthropic_dir).unwrap();
        fs::write(
            anthropic_dir.join("system_prompt.md"),
            "Anthropic-tuned prompt.",
        )
        .unwrap();

        let scan = scan_skills_dir(tmp.path());
        assert_eq!(scan.entries.len(), 1);
        assert_eq!(scan.entries[0].prompt_snippet, "Root prompt.");
        // Provider-level prompts are no longer loaded
        assert!(scan.entries[0].provider_overrides.is_empty());
    }

    #[test]
    fn test_scan_with_provider_variant_override() {
        let tmp = tempfile::tempdir().unwrap();
        let skill_dir = tmp.path().join("web-search");
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(
            skill_dir.join("skill.toml"),
            r#"
            [skill]
            name = "web-search"
            description = "Search the web"
            timeout_secs = 30
            "#,
        )
        .unwrap();

        // Create openai variant with timeout override
        let openai_dir = skill_dir.join("openai");
        fs::create_dir_all(&openai_dir).unwrap();
        fs::write(
            openai_dir.join("skill.toml"),
            r#"
            [skill]
            timeout_secs = 60
            "#,
        )
        .unwrap();

        let scan = scan_skills_dir(tmp.path());
        assert_eq!(scan.entries.len(), 1);
        let overrides = scan.entries[0].provider_overrides.get("openai").unwrap();
        assert_eq!(overrides.timeout_secs, Some(60));
        assert_eq!(overrides.max_prompt_size, None);
    }

    #[test]
    fn test_scan_ignores_non_provider_subdirs() {
        let tmp = tempfile::tempdir().unwrap();
        let skill_dir = tmp.path().join("web-search");
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(
            skill_dir.join("skill.toml"),
            r#"
            [skill]
            name = "web-search"
            description = "Search the web"
            "#,
        )
        .unwrap();

        // Create handlers/ subdir (not a provider)
        let handlers_dir = skill_dir.join("handlers");
        fs::create_dir_all(&handlers_dir).unwrap();
        fs::write(handlers_dir.join("search.sh"), "#!/bin/sh\necho ok").unwrap();

        // Create .git subdir (not a provider)
        let git_dir = skill_dir.join(".git");
        fs::create_dir_all(&git_dir).unwrap();

        let scan = scan_skills_dir(tmp.path());
        assert_eq!(scan.entries.len(), 1);
        assert!(scan.entries[0].provider_overrides.is_empty());
    }

    #[test]
    fn test_scan_empty_provider_dir_skipped() {
        let tmp = tempfile::tempdir().unwrap();
        let skill_dir = tmp.path().join("web-search");
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(
            skill_dir.join("skill.toml"),
            r#"
            [skill]
            name = "web-search"
            description = "Search the web"
            "#,
        )
        .unwrap();

        // Create empty groq variant directory
        let groq_dir = skill_dir.join("groq");
        fs::create_dir_all(&groq_dir).unwrap();

        let scan = scan_skills_dir(tmp.path());
        assert_eq!(scan.entries.len(), 1);
        // Empty provider dir should not add to maps
        assert!(!scan.entries[0].provider_overrides.contains_key("groq"));
    }

    #[test]
    fn test_scan_malformed_provider_override_warned() {
        let tmp = tempfile::tempdir().unwrap();
        let skill_dir = tmp.path().join("web-search");
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(
            skill_dir.join("skill.toml"),
            r#"
            [skill]
            name = "web-search"
            description = "Search the web"
            "#,
        )
        .unwrap();

        // Create anthropic variant with bad TOML
        let anthropic_dir = skill_dir.join("anthropic");
        fs::create_dir_all(&anthropic_dir).unwrap();
        fs::write(anthropic_dir.join("skill.toml"), "not valid toml {{{}}}").unwrap();

        let scan = scan_skills_dir(tmp.path());
        assert_eq!(scan.entries.len(), 1);
        // Malformed override should be skipped
        assert!(!scan.entries[0].provider_overrides.contains_key("anthropic"));
    }

    #[test]
    fn test_effective_timeout_with_override() {
        let mut entry = SkillEntry {
            manifest: SkillManifest {
                skill: super::super::manifest::SkillInfo {
                    name: "test".to_string(),
                    description: "test".to_string(),
                    version: String::new(),
                    always_on: false,
                    timeout_secs: 30,
                    dependencies: vec![],
                    max_prompt_size: None,
                },
                triggers: super::super::manifest::Triggers { keywords: vec![] },
                llm: Default::default(),
                constraints: Default::default(),
                context: std::collections::HashMap::new(),
            },
            dir: PathBuf::from("/skills/test"),
            keywords_lower: vec![],
            prompt_snippet: String::new(),
            skill_tools: vec![],
            enabled: true,
            has_override: false,
            provider_overrides: HashMap::new(),
            model_prompts: HashMap::new(),
            model_overrides: HashMap::new(),
            generated_model_prompts: HashMap::new(),
        };
        entry.provider_overrides.insert(
            "openai".to_string(),
            super::super::manifest::ProviderSkillFields {
                timeout_secs: Some(90),
                max_prompt_size: None,
            },
        );
        assert_eq!(entry.effective_timeout("openai", "gpt-4o"), 90);
    }

    #[test]
    fn test_effective_timeout_without_override() {
        let entry = SkillEntry {
            manifest: SkillManifest {
                skill: super::super::manifest::SkillInfo {
                    name: "test".to_string(),
                    description: "test".to_string(),
                    version: String::new(),
                    always_on: false,
                    timeout_secs: 30,
                    dependencies: vec![],
                    max_prompt_size: None,
                },
                triggers: super::super::manifest::Triggers { keywords: vec![] },
                llm: Default::default(),
                constraints: Default::default(),
                context: std::collections::HashMap::new(),
            },
            dir: PathBuf::from("/skills/test"),
            keywords_lower: vec![],
            prompt_snippet: String::new(),
            skill_tools: vec![],
            enabled: true,
            has_override: false,
            provider_overrides: HashMap::new(),
            model_prompts: HashMap::new(),
            model_overrides: HashMap::new(),
            generated_model_prompts: HashMap::new(),
        };
        assert_eq!(
            entry.effective_timeout("anthropic", "claude-sonnet-4-6"),
            30
        );
    }

    #[test]
    fn test_effective_timeout_unknown_provider() {
        let entry = SkillEntry {
            manifest: SkillManifest {
                skill: super::super::manifest::SkillInfo {
                    name: "test".to_string(),
                    description: "test".to_string(),
                    version: String::new(),
                    always_on: false,
                    timeout_secs: 30,
                    dependencies: vec![],
                    max_prompt_size: None,
                },
                triggers: super::super::manifest::Triggers { keywords: vec![] },
                llm: Default::default(),
                constraints: Default::default(),
                context: std::collections::HashMap::new(),
            },
            dir: PathBuf::from("/skills/test"),
            keywords_lower: vec![],
            prompt_snippet: String::new(),
            skill_tools: vec![],
            enabled: true,
            has_override: false,
            provider_overrides: HashMap::new(),
            model_prompts: HashMap::new(),
            model_overrides: HashMap::new(),
            generated_model_prompts: HashMap::new(),
        };
        assert_eq!(
            entry.effective_timeout("unknown_provider", "some-model"),
            30
        );
    }

    #[test]
    fn test_variant_count() {
        let mut entry = SkillEntry {
            manifest: SkillManifest {
                skill: super::super::manifest::SkillInfo {
                    name: "test".to_string(),
                    description: "test".to_string(),
                    version: String::new(),
                    always_on: false,
                    timeout_secs: 30,
                    dependencies: vec![],
                    max_prompt_size: None,
                },
                triggers: super::super::manifest::Triggers { keywords: vec![] },
                llm: Default::default(),
                constraints: Default::default(),
                context: std::collections::HashMap::new(),
            },
            dir: PathBuf::from("/skills/test"),
            keywords_lower: vec![],
            prompt_snippet: String::new(),
            skill_tools: vec![],
            enabled: true,
            has_override: false,
            provider_overrides: HashMap::new(),
            model_prompts: HashMap::new(),
            model_overrides: HashMap::new(),
            generated_model_prompts: HashMap::new(),
        };

        assert_eq!(entry.variant_count(), 0);

        entry.provider_overrides.insert(
            "anthropic".to_string(),
            super::super::manifest::ProviderSkillFields {
                timeout_secs: Some(60),
                max_prompt_size: None,
            },
        );
        assert_eq!(entry.variant_count(), 1);

        entry.provider_overrides.insert(
            "openai".to_string(),
            super::super::manifest::ProviderSkillFields {
                timeout_secs: Some(90),
                max_prompt_size: None,
            },
        );
        assert_eq!(entry.variant_count(), 2);
    }

    #[test]
    fn test_validate_provider_variant_valid() {
        let tmp = tempfile::tempdir().unwrap();
        let skill_dir = tmp.path().join("web-search");
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(
            skill_dir.join("skill.toml"),
            r#"
            [skill]
            name = "web-search"
            description = "Search the web"
            "#,
        )
        .unwrap();
        fs::write(skill_dir.join("system_prompt.md"), "Root prompt.").unwrap();

        // Valid provider variant with skill.toml override
        let anthropic_dir = skill_dir.join("anthropic");
        fs::create_dir_all(&anthropic_dir).unwrap();
        fs::write(
            anthropic_dir.join("skill.toml"),
            r#"
            [skill]
            timeout_secs = 60
            "#,
        )
        .unwrap();

        let diags = validate_skill(&skill_dir);
        let provider_ok = diags
            .iter()
            .any(|d| d.level == DiagnosticLevel::Ok && d.message.contains("provider 'anthropic"));
        assert!(provider_ok, "Expected OK diag for provider skill.toml");
    }

    #[test]
    fn test_validate_provider_prompt_warned() {
        let tmp = tempfile::tempdir().unwrap();
        let skill_dir = tmp.path().join("web-search");
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(
            skill_dir.join("skill.toml"),
            r#"
            [skill]
            name = "web-search"
            description = "Search the web"
            "#,
        )
        .unwrap();

        // Provider-level system_prompt.md should produce a warning
        let anthropic_dir = skill_dir.join("anthropic");
        fs::create_dir_all(&anthropic_dir).unwrap();
        fs::write(anthropic_dir.join("system_prompt.md"), "Anthropic prompt.").unwrap();
        fs::write(
            anthropic_dir.join("skill.toml"),
            r#"
            [skill]
            timeout_secs = 60
            "#,
        )
        .unwrap();

        let diags = validate_skill(&skill_dir);
        let prompt_warn = diags.iter().any(|d| {
            d.level == DiagnosticLevel::Warn
                && d.message.contains("system_prompt.md")
                && d.message.contains("ignored")
        });
        assert!(
            prompt_warn,
            "Expected WARN for provider-level system_prompt.md"
        );
    }

    #[test]
    fn test_validate_provider_variant_tools_json_warn() {
        let tmp = tempfile::tempdir().unwrap();
        let skill_dir = tmp.path().join("web-search");
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(
            skill_dir.join("skill.toml"),
            r#"
            [skill]
            name = "web-search"
            description = "Search the web"
            "#,
        )
        .unwrap();

        let anthropic_dir = skill_dir.join("anthropic");
        fs::create_dir_all(&anthropic_dir).unwrap();
        fs::write(
            anthropic_dir.join("skill.toml"),
            r#"
            [skill]
            timeout_secs = 60
            "#,
        )
        .unwrap();
        fs::write(anthropic_dir.join("tools.json"), "[]").unwrap();

        let diags = validate_skill(&skill_dir);
        let warn = diags
            .iter()
            .find(|d| d.message.contains("tools.json") && d.message.contains("not supported"));
        assert!(warn.is_some());
    }

    #[test]
    fn test_validate_provider_subdir_typo_warn() {
        let tmp = tempfile::tempdir().unwrap();
        let skill_dir = tmp.path().join("web-search");
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(
            skill_dir.join("skill.toml"),
            r#"
            [skill]
            name = "web-search"
            description = "Search the web"
            "#,
        )
        .unwrap();

        // Typo: "antropic" instead of "anthropic"
        let typo_dir = skill_dir.join("antropic");
        fs::create_dir_all(&typo_dir).unwrap();

        let diags = validate_skill(&skill_dir);
        let typo_warn = diags
            .iter()
            .find(|d| d.message.contains("misspelling") && d.message.contains("anthropic"));
        assert!(
            typo_warn.is_some(),
            "Expected typo warning for 'antropic'. Got: {diags:?}"
        );
    }

    #[test]
    fn test_looks_like_typo() {
        // Should detect common typos
        assert!(looks_like_typo("antropic", "anthropic"));
        assert!(looks_like_typo("openia", "openai"));
        assert!(looks_like_typo("gogle", "google"));

        // Should NOT flag clearly different names
        assert!(!looks_like_typo("handlers", "anthropic"));
        assert!(!looks_like_typo(".git", "groq"));

        // Same string is not a typo
        assert!(!looks_like_typo("anthropic", "anthropic"));
    }

    #[test]
    fn test_scan_multiple_provider_overrides() {
        let tmp = tempfile::tempdir().unwrap();
        let skill_dir = tmp.path().join("multi-provider");
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(
            skill_dir.join("skill.toml"),
            r#"
            [skill]
            name = "multi-provider"
            description = "Multi-provider skill"
            timeout_secs = 30
            "#,
        )
        .unwrap();
        fs::write(skill_dir.join("system_prompt.md"), "Root prompt.").unwrap();

        // Create multiple provider overrides
        for (provider, timeout) in &[("anthropic", 60), ("openai", 90), ("groq", 45)] {
            let dir = skill_dir.join(provider);
            fs::create_dir_all(&dir).unwrap();
            fs::write(
                dir.join("skill.toml"),
                format!(
                    r#"
            [skill]
            timeout_secs = {timeout}
            "#
                ),
            )
            .unwrap();
        }

        let scan = scan_skills_dir(tmp.path());
        assert_eq!(scan.entries.len(), 1);
        assert_eq!(scan.entries[0].provider_overrides.len(), 3);
        assert_eq!(
            scan.entries[0]
                .provider_overrides
                .get("anthropic")
                .unwrap()
                .timeout_secs,
            Some(60)
        );
        assert_eq!(scan.entries[0].variant_count(), 3);
    }

    #[test]
    fn test_resolve_prompt_falls_back_to_generated() {
        let tmp = tempfile::tempdir().unwrap();
        let skills_root = tmp.path().to_path_buf();
        let skill_dir = skills_root.join("web-search");
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(
            skill_dir.join("skill.toml"),
            r#"
            [skill]
            name = "web-search"
            description = "Search the web"
            "#,
        )
        .unwrap();
        fs::write(skill_dir.join("system_prompt.md"), "ROOT").unwrap();
        // Only generated variant present (no hand-authored).
        let gen_dir = skill_dir.join("generated/anthropic/claude-sonnet-4-6");
        fs::create_dir_all(&gen_dir).unwrap();
        fs::write(gen_dir.join("system_prompt.md"), "GENERATED").unwrap();

        let scan = scan_skills_dir(&skills_root);
        assert_eq!(scan.entries.len(), 1);
        let entry = &scan.entries[0];
        assert_eq!(entry.generated_model_prompts.len(), 1);
        // Generated variant wins over root when no hand-authored exists.
        let resolved = entry.resolve_prompt("anthropic", "claude-sonnet-4-6");
        assert_eq!(resolved.text, "GENERATED");
        assert_eq!(resolved.source, PromptVariantSource::GeneratedModel);
        // Other models still fall back to root.
        assert_eq!(entry.resolve_prompt("openai", "gpt-4o").text, "ROOT");
    }

    #[test]
    fn test_resolve_prompt_openrouter_finds_canonical_generated_variant() {
        // End-to-end loop invariant: a variant written via openrouter ctx
        // (write path canonicalises the aggregator namespace) MUST be findable
        // when resolved via the same openrouter ctx (read path must also
        // canonicalise). This is the most load-bearing invariant of the
        // generated-variants feature — without it the agent silently falls
        // back to the root prompt after every successful write.
        let tmp = tempfile::tempdir().unwrap();
        let skills_root = tmp.path().to_path_buf();
        let skill_dir = skills_root.join("web-search");
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(
            skill_dir.join("skill.toml"),
            r#"
            [skill]
            name = "web-search"
            description = "Search the web"
            "#,
        )
        .unwrap();
        fs::write(skill_dir.join("system_prompt.md"), "ROOT").unwrap();
        // Variant written under canonical (minimax/minimax-m2.7), as
        // review_skill does for an openrouter ctx.
        let gen_dir = skill_dir.join("generated/minimax/minimax-m2.7");
        fs::create_dir_all(&gen_dir).unwrap();
        fs::write(gen_dir.join("system_prompt.md"), "GENERATED").unwrap();

        let scan = scan_skills_dir(&skills_root);
        let entry = &scan.entries[0];
        // Lookup via the requesting (openrouter) ctx — must canonicalise
        // and return the variant.
        let resolved = entry.resolve_prompt("openrouter", "minimax/minimax-m2.7");
        assert_eq!(resolved.text, "GENERATED");
        assert_eq!(resolved.source, PromptVariantSource::GeneratedCanonical);
        // Direct canonical lookup also works.
        let resolved = entry.resolve_prompt("minimax", "minimax-m2.7");
        assert_eq!(resolved.text, "GENERATED");
        assert_eq!(resolved.source, PromptVariantSource::GeneratedModel);
        // Other providers still fall through to root.
        assert_eq!(
            entry.resolve_prompt("anthropic", "claude-sonnet-4-6").text,
            "ROOT"
        );
    }

    #[test]
    fn test_resolve_prompt_handauthored_beats_generated() {
        let tmp = tempfile::tempdir().unwrap();
        let skills_root = tmp.path().to_path_buf();
        let skill_dir = skills_root.join("web-search");
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(
            skill_dir.join("skill.toml"),
            r#"
            [skill]
            name = "web-search"
            description = "Search the web"
            "#,
        )
        .unwrap();
        fs::write(skill_dir.join("system_prompt.md"), "ROOT").unwrap();
        // Hand-authored variant.
        let hand = skill_dir.join("anthropic/claude-sonnet-4-6");
        fs::create_dir_all(&hand).unwrap();
        fs::write(hand.join("system_prompt.md"), "HAND").unwrap();
        // Generated variant for the same provider/model.
        let gen_dir = skill_dir.join("generated/anthropic/claude-sonnet-4-6");
        fs::create_dir_all(&gen_dir).unwrap();
        fs::write(gen_dir.join("system_prompt.md"), "GENERATED").unwrap();

        let scan = scan_skills_dir(&skills_root);
        let entry = &scan.entries[0];
        // Hand-authored always wins — generated must not silently shadow it.
        let resolved = entry.resolve_prompt("anthropic", "claude-sonnet-4-6");
        assert_eq!(resolved.text, "HAND");
        assert_eq!(resolved.source, PromptVariantSource::HandAuthoredModel);
    }

    #[test]
    fn test_validate_provider_override_identity_field_warns() {
        let tmp = tempfile::tempdir().unwrap();
        let skill_dir = tmp.path().join("web-search");
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(
            skill_dir.join("skill.toml"),
            r#"
            [skill]
            name = "web-search"
            description = "Search the web"
            "#,
        )
        .unwrap();

        // Create provider override with identity fields (should warn)
        let anthropic_dir = skill_dir.join("anthropic");
        fs::create_dir_all(&anthropic_dir).unwrap();
        fs::write(
            anthropic_dir.join("skill.toml"),
            r#"
            [skill]
            name = "web-search-anthropic"
            description = "Anthropic-specific search"
            timeout_secs = 60
            "#,
        )
        .unwrap();

        let diags = validate_skill(&skill_dir);
        let name_warn = diags.iter().find(|d| {
            d.level == DiagnosticLevel::Warn
                && d.message.contains("identity field 'name'")
                && d.message.contains("anthropic")
        });
        assert!(
            name_warn.is_some(),
            "Expected warning for identity field 'name'. Got: {diags:?}"
        );

        let desc_warn = diags.iter().find(|d| {
            d.level == DiagnosticLevel::Warn
                && d.message.contains("identity field 'description'")
                && d.message.contains("anthropic")
        });
        assert!(
            desc_warn.is_some(),
            "Expected warning for identity field 'description'. Got: {diags:?}"
        );
    }

    // -- Model variant tests --

    #[test]
    fn test_sanitize_model_dir_name() {
        assert_eq!(
            sanitize_model_dir_name("claude-sonnet-4-6"),
            "claude-sonnet-4-6"
        );
        assert_eq!(
            sanitize_model_dir_name("anthropic/claude-sonnet-4"),
            "anthropic--claude-sonnet-4"
        );
        assert_eq!(sanitize_model_dir_name("gpt-4o"), "gpt-4o");
        assert_eq!(sanitize_model_dir_name("MiniMax-M2.7"), "MiniMax-M2.7");
        // Multiple slashes
        assert_eq!(sanitize_model_dir_name("a/b/c"), "a--b--c");
        // No slash — unchanged
        assert_eq!(sanitize_model_dir_name("no-slash"), "no-slash");
    }

    #[test]
    fn test_scan_with_model_variant_prompt() {
        let tmp = tempfile::tempdir().unwrap();
        let skill_dir = tmp.path().join("web-search");
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(
            skill_dir.join("skill.toml"),
            r#"
            [skill]
            name = "web-search"
            description = "Search the web"
            "#,
        )
        .unwrap();
        fs::write(skill_dir.join("system_prompt.md"), "Root prompt.").unwrap();

        // Create provider + model variant
        let anthropic_dir = skill_dir.join("anthropic");
        fs::create_dir_all(&anthropic_dir).unwrap();

        let model_dir = anthropic_dir.join("claude-sonnet-4-6");
        fs::create_dir_all(&model_dir).unwrap();
        fs::write(model_dir.join("system_prompt.md"), "Sonnet 4.6 prompt.").unwrap();

        let scan = scan_skills_dir(tmp.path());
        assert_eq!(scan.entries.len(), 1);
        assert_eq!(scan.entries[0].prompt_snippet, "Root prompt.");
        assert_eq!(
            scan.entries[0]
                .model_prompts
                .get("anthropic/claude-sonnet-4-6")
                .unwrap(),
            "Sonnet 4.6 prompt."
        );
    }

    #[test]
    fn test_scan_with_model_variant_override() {
        let tmp = tempfile::tempdir().unwrap();
        let skill_dir = tmp.path().join("web-search");
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(
            skill_dir.join("skill.toml"),
            r#"
            [skill]
            name = "web-search"
            description = "Search the web"
            timeout_secs = 30
            "#,
        )
        .unwrap();

        // Create openai provider + model variant with override
        let openai_dir = skill_dir.join("openai");
        fs::create_dir_all(&openai_dir).unwrap();
        fs::write(
            openai_dir.join("skill.toml"),
            r#"
            [skill]
            timeout_secs = 60
            "#,
        )
        .unwrap();

        let model_dir = openai_dir.join("gpt-4o");
        fs::create_dir_all(&model_dir).unwrap();
        fs::write(
            model_dir.join("skill.toml"),
            r#"
            [skill]
            timeout_secs = 120
            "#,
        )
        .unwrap();

        let scan = scan_skills_dir(tmp.path());
        assert_eq!(scan.entries.len(), 1);
        let model_override = scan.entries[0]
            .model_overrides
            .get("openai/gpt-4o")
            .unwrap();
        assert_eq!(model_override.timeout_secs, Some(120));
    }

    #[test]
    fn test_scan_model_with_slash_in_name() {
        let tmp = tempfile::tempdir().unwrap();
        let skill_dir = tmp.path().join("web-search");
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(
            skill_dir.join("skill.toml"),
            r#"
            [skill]
            name = "web-search"
            description = "Search the web"
            "#,
        )
        .unwrap();

        // OpenRouter model with slash: directory uses -- as separator
        let openrouter_dir = skill_dir.join("openrouter");
        fs::create_dir_all(&openrouter_dir).unwrap();
        fs::write(
            openrouter_dir.join("system_prompt.md"),
            "OpenRouter prompt.",
        )
        .unwrap();

        let model_dir = openrouter_dir.join("anthropic--claude-sonnet-4");
        fs::create_dir_all(&model_dir).unwrap();
        fs::write(
            model_dir.join("system_prompt.md"),
            "OpenRouter Claude Sonnet prompt.",
        )
        .unwrap();

        let scan = scan_skills_dir(tmp.path());
        assert_eq!(scan.entries.len(), 1);
        assert_eq!(
            scan.entries[0]
                .model_prompts
                .get("openrouter/anthropic--claude-sonnet-4")
                .unwrap(),
            "OpenRouter Claude Sonnet prompt."
        );
        // Verify sanitize_model_dir_name produces the right key
        let sanitized = sanitize_model_dir_name("anthropic/claude-sonnet-4");
        assert_eq!(sanitized, "anthropic--claude-sonnet-4");
        let lookup_key = format!("openrouter/{sanitized}");
        assert!(scan.entries[0].model_prompts.contains_key(&lookup_key));
    }

    #[test]
    fn test_scan_skips_dotdirs_inside_provider() {
        let tmp = tempfile::tempdir().unwrap();
        let skill_dir = tmp.path().join("web-search");
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(
            skill_dir.join("skill.toml"),
            r#"
            [skill]
            name = "web-search"
            description = "Search the web"
            "#,
        )
        .unwrap();

        let anthropic_dir = skill_dir.join("anthropic");
        fs::create_dir_all(&anthropic_dir).unwrap();
        fs::write(anthropic_dir.join("system_prompt.md"), "Anthropic prompt.").unwrap();

        // Dotdir inside provider — should be skipped
        let git_dir = anthropic_dir.join(".git");
        fs::create_dir_all(&git_dir).unwrap();
        fs::write(git_dir.join("system_prompt.md"), "Should be ignored.").unwrap();

        let scan = scan_skills_dir(tmp.path());
        assert_eq!(scan.entries.len(), 1);
        assert!(scan.entries[0].model_prompts.is_empty());
    }

    #[test]
    fn test_scan_empty_model_dir_warned() {
        let tmp = tempfile::tempdir().unwrap();
        let skill_dir = tmp.path().join("web-search");
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(
            skill_dir.join("skill.toml"),
            r#"
            [skill]
            name = "web-search"
            description = "Search the web"
            "#,
        )
        .unwrap();

        let anthropic_dir = skill_dir.join("anthropic");
        fs::create_dir_all(&anthropic_dir).unwrap();
        fs::write(anthropic_dir.join("system_prompt.md"), "Anthropic prompt.").unwrap();

        // Empty model dir
        let model_dir = anthropic_dir.join("claude-opus-4");
        fs::create_dir_all(&model_dir).unwrap();

        // Should not panic — warning logged
        let scan = scan_skills_dir(tmp.path());
        assert_eq!(scan.entries.len(), 1);
        // Model dir is empty so should not be in model_prompts or model_overrides
        assert!(
            !scan.entries[0]
                .model_prompts
                .contains_key("anthropic/claude-opus-4")
        );
        assert!(
            !scan.entries[0]
                .model_overrides
                .contains_key("anthropic/claude-opus-4")
        );
    }

    #[test]
    fn test_scan_multiple_model_variants() {
        let tmp = tempfile::tempdir().unwrap();
        let skill_dir = tmp.path().join("multi-model");
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(
            skill_dir.join("skill.toml"),
            r#"
            [skill]
            name = "multi-model"
            description = "Multi-model skill"
            "#,
        )
        .unwrap();
        fs::write(skill_dir.join("system_prompt.md"), "Root prompt.").unwrap();

        // Two models under anthropic
        let anthropic_dir = skill_dir.join("anthropic");
        fs::create_dir_all(&anthropic_dir).unwrap();

        let sonnet_dir = anthropic_dir.join("claude-sonnet-4-6");
        fs::create_dir_all(&sonnet_dir).unwrap();
        fs::write(sonnet_dir.join("system_prompt.md"), "Sonnet prompt.").unwrap();

        let opus_dir = anthropic_dir.join("claude-opus-4");
        fs::create_dir_all(&opus_dir).unwrap();
        fs::write(opus_dir.join("system_prompt.md"), "Opus prompt.").unwrap();

        // One model under minimax
        let minimax_dir = skill_dir.join("minimax");
        fs::create_dir_all(&minimax_dir).unwrap();

        let m27_dir = minimax_dir.join("MiniMax-M2.7");
        fs::create_dir_all(&m27_dir).unwrap();
        fs::write(m27_dir.join("system_prompt.md"), "M2.7 prompt.").unwrap();

        let scan = scan_skills_dir(tmp.path());
        assert_eq!(scan.entries.len(), 1);
        // 3 model variants only (no provider-level prompts)
        assert_eq!(scan.entries[0].model_prompts.len(), 3);
        assert_eq!(scan.entries[0].variant_count(), 3);

        // Verify specific entries
        assert_eq!(
            scan.entries[0]
                .model_prompts
                .get("anthropic/claude-sonnet-4-6")
                .unwrap(),
            "Sonnet prompt."
        );
        assert_eq!(
            scan.entries[0]
                .model_prompts
                .get("anthropic/claude-opus-4")
                .unwrap(),
            "Opus prompt."
        );
        assert_eq!(
            scan.entries[0]
                .model_prompts
                .get("minimax/MiniMax-M2.7")
                .unwrap(),
            "M2.7 prompt."
        );
    }

    #[test]
    fn test_resolve_prompt_model_level() {
        let mut entry = SkillEntry {
            manifest: SkillManifest {
                skill: super::super::manifest::SkillInfo {
                    name: "test".to_string(),
                    description: "test".to_string(),
                    version: String::new(),
                    always_on: false,
                    timeout_secs: 30,
                    dependencies: vec![],
                    max_prompt_size: None,
                },
                triggers: super::super::manifest::Triggers { keywords: vec![] },
                llm: Default::default(),
                constraints: Default::default(),
                context: std::collections::HashMap::new(),
            },
            dir: PathBuf::from("/skills/test"),
            keywords_lower: vec![],
            prompt_snippet: "Root prompt.".to_string(),
            skill_tools: vec![],
            enabled: true,
            has_override: false,
            provider_overrides: HashMap::new(),
            model_prompts: HashMap::new(),
            model_overrides: HashMap::new(),
            generated_model_prompts: HashMap::new(),
        };
        entry.model_prompts.insert(
            "anthropic/claude-sonnet-4-6".to_string(),
            "Sonnet prompt.".to_string(),
        );

        // Model-specific wins
        assert_eq!(
            entry.resolve_prompt("anthropic", "claude-sonnet-4-6").text,
            "Sonnet prompt."
        );
        // No model variant for opus — falls back to root
        assert_eq!(
            entry.resolve_prompt("anthropic", "claude-opus-4").text,
            "Root prompt."
        );
        // No model variant for groq — falls back to root
        assert_eq!(
            entry.resolve_prompt("groq", "llama-3.3-70b-versatile").text,
            "Root prompt."
        );
    }

    #[test]
    fn test_resolve_prompt_with_slash_in_model_name() {
        let mut entry = SkillEntry {
            manifest: SkillManifest {
                skill: super::super::manifest::SkillInfo {
                    name: "test".to_string(),
                    description: "test".to_string(),
                    version: String::new(),
                    always_on: false,
                    timeout_secs: 30,
                    dependencies: vec![],
                    max_prompt_size: None,
                },
                triggers: super::super::manifest::Triggers { keywords: vec![] },
                llm: Default::default(),
                constraints: Default::default(),
                context: std::collections::HashMap::new(),
            },
            dir: PathBuf::from("/skills/test"),
            keywords_lower: vec![],
            prompt_snippet: "Root prompt.".to_string(),
            skill_tools: vec![],
            enabled: true,
            has_override: false,
            provider_overrides: HashMap::new(),
            model_prompts: HashMap::new(),
            model_overrides: HashMap::new(),
            generated_model_prompts: HashMap::new(),
        };
        entry.model_prompts.insert(
            "openrouter/anthropic--claude-sonnet-4".to_string(),
            "OpenRouter model prompt.".to_string(),
        );

        // Model name with slash gets sanitized, matching the stored key
        assert_eq!(
            entry
                .resolve_prompt("openrouter", "anthropic/claude-sonnet-4")
                .text,
            "OpenRouter model prompt."
        );
    }

    #[test]
    fn test_effective_timeout_model_override() {
        let mut entry = SkillEntry {
            manifest: SkillManifest {
                skill: super::super::manifest::SkillInfo {
                    name: "test".to_string(),
                    description: "test".to_string(),
                    version: String::new(),
                    always_on: false,
                    timeout_secs: 30,
                    dependencies: vec![],
                    max_prompt_size: None,
                },
                triggers: super::super::manifest::Triggers { keywords: vec![] },
                llm: Default::default(),
                constraints: Default::default(),
                context: std::collections::HashMap::new(),
            },
            dir: PathBuf::from("/skills/test"),
            keywords_lower: vec![],
            prompt_snippet: String::new(),
            skill_tools: vec![],
            enabled: true,
            has_override: false,
            provider_overrides: HashMap::new(),
            model_prompts: HashMap::new(),
            model_overrides: HashMap::new(),
            generated_model_prompts: HashMap::new(),
        };
        entry.provider_overrides.insert(
            "anthropic".to_string(),
            super::super::manifest::ProviderSkillFields {
                timeout_secs: Some(90),
                max_prompt_size: None,
            },
        );
        entry.model_overrides.insert(
            "anthropic/claude-sonnet-4-6".to_string(),
            super::super::manifest::ProviderSkillFields {
                timeout_secs: Some(120),
                max_prompt_size: None,
            },
        );

        // Model override wins
        assert_eq!(
            entry.effective_timeout("anthropic", "claude-sonnet-4-6"),
            120
        );
        // No model override — falls back to provider
        assert_eq!(entry.effective_timeout("anthropic", "claude-opus-4"), 90);
        // No model or provider override — falls back to root
        assert_eq!(entry.effective_timeout("groq", "llama"), 30);
    }

    #[test]
    fn test_variant_count_includes_models() {
        let mut entry = SkillEntry {
            manifest: SkillManifest {
                skill: super::super::manifest::SkillInfo {
                    name: "test".to_string(),
                    description: "test".to_string(),
                    version: String::new(),
                    always_on: false,
                    timeout_secs: 30,
                    dependencies: vec![],
                    max_prompt_size: None,
                },
                triggers: super::super::manifest::Triggers { keywords: vec![] },
                llm: Default::default(),
                constraints: Default::default(),
                context: std::collections::HashMap::new(),
            },
            dir: PathBuf::from("/skills/test"),
            keywords_lower: vec![],
            prompt_snippet: String::new(),
            skill_tools: vec![],
            enabled: true,
            has_override: false,
            provider_overrides: HashMap::new(),
            model_prompts: HashMap::new(),
            model_overrides: HashMap::new(),
            generated_model_prompts: HashMap::new(),
        };

        // 2 provider overrides
        entry.provider_overrides.insert(
            "anthropic".to_string(),
            super::super::manifest::ProviderSkillFields {
                timeout_secs: Some(60),
                max_prompt_size: None,
            },
        );
        entry.provider_overrides.insert(
            "openai".to_string(),
            super::super::manifest::ProviderSkillFields {
                timeout_secs: Some(90),
                max_prompt_size: None,
            },
        );
        assert_eq!(entry.variant_count(), 2);

        // + 3 model variants = 5 total
        entry.model_prompts.insert(
            "anthropic/claude-sonnet-4-6".to_string(),
            "prompt".to_string(),
        );
        entry
            .model_prompts
            .insert("anthropic/claude-opus-4".to_string(), "prompt".to_string());
        entry
            .model_prompts
            .insert("openai/gpt-4o".to_string(), "prompt".to_string());
        assert_eq!(entry.variant_count(), 5);
    }

    #[test]
    fn test_variant_models_for_provider() {
        let mut entry = SkillEntry {
            manifest: SkillManifest {
                skill: super::super::manifest::SkillInfo {
                    name: "test".to_string(),
                    description: "test".to_string(),
                    version: String::new(),
                    always_on: false,
                    timeout_secs: 30,
                    dependencies: vec![],
                    max_prompt_size: None,
                },
                triggers: super::super::manifest::Triggers { keywords: vec![] },
                llm: Default::default(),
                constraints: Default::default(),
                context: std::collections::HashMap::new(),
            },
            dir: PathBuf::from("/skills/test"),
            keywords_lower: vec![],
            prompt_snippet: String::new(),
            skill_tools: vec![],
            enabled: true,
            has_override: false,
            provider_overrides: HashMap::new(),
            model_prompts: HashMap::new(),
            model_overrides: HashMap::new(),
            generated_model_prompts: HashMap::new(),
        };

        entry.model_prompts.insert(
            "anthropic/claude-sonnet-4-6".to_string(),
            "prompt".to_string(),
        );
        entry.model_overrides.insert(
            "anthropic/claude-opus-4".to_string(),
            super::super::manifest::ProviderSkillFields {
                timeout_secs: Some(90),
                max_prompt_size: None,
            },
        );
        entry
            .model_prompts
            .insert("openai/gpt-4o".to_string(), "prompt".to_string());

        let anthropic_models = entry.variant_models("anthropic");
        assert_eq!(anthropic_models.len(), 2);
        assert!(anthropic_models.contains("claude-sonnet-4-6"));
        assert!(anthropic_models.contains("claude-opus-4"));

        let openai_models = entry.variant_models("openai");
        assert_eq!(openai_models.len(), 1);
        assert!(openai_models.contains("gpt-4o"));

        let groq_models = entry.variant_models("groq");
        assert!(groq_models.is_empty());
    }

    #[test]
    fn test_variant_providers_includes_model_only_providers() {
        let mut entry = SkillEntry {
            manifest: SkillManifest {
                skill: super::super::manifest::SkillInfo {
                    name: "test".to_string(),
                    description: "test".to_string(),
                    version: String::new(),
                    always_on: false,
                    timeout_secs: 30,
                    dependencies: vec![],
                    max_prompt_size: None,
                },
                triggers: super::super::manifest::Triggers { keywords: vec![] },
                llm: Default::default(),
                constraints: Default::default(),
                context: std::collections::HashMap::new(),
            },
            dir: PathBuf::from("/skills/test"),
            keywords_lower: vec![],
            prompt_snippet: String::new(),
            skill_tools: vec![],
            enabled: true,
            has_override: false,
            provider_overrides: HashMap::new(),
            model_prompts: HashMap::new(),
            model_overrides: HashMap::new(),
            generated_model_prompts: HashMap::new(),
        };

        // Only model variants, no provider-level
        entry.model_prompts.insert(
            "anthropic/claude-sonnet-4-6".to_string(),
            "prompt".to_string(),
        );

        let providers = entry.variant_providers();
        assert!(providers.contains("anthropic"));
    }

    // -- Model variant validation tests --

    #[test]
    fn test_validate_model_variant_valid() {
        let tmp = tempfile::tempdir().unwrap();
        let skill_dir = tmp.path().join("web-search");
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(
            skill_dir.join("skill.toml"),
            r#"
            [skill]
            name = "web-search"
            description = "Search the web"
            "#,
        )
        .unwrap();
        fs::write(skill_dir.join("system_prompt.md"), "Root prompt.").unwrap();

        let anthropic_dir = skill_dir.join("anthropic");
        fs::create_dir_all(&anthropic_dir).unwrap();
        fs::write(anthropic_dir.join("system_prompt.md"), "Anthropic prompt.").unwrap();

        let model_dir = anthropic_dir.join("claude-sonnet-4-6");
        fs::create_dir_all(&model_dir).unwrap();
        fs::write(model_dir.join("system_prompt.md"), "Sonnet prompt.").unwrap();
        fs::write(
            model_dir.join("skill.toml"),
            r#"
            [skill]
            timeout_secs = 120
            "#,
        )
        .unwrap();

        let diags = validate_skill(&skill_dir);
        let model_ok_count = diags
            .iter()
            .filter(|d| {
                d.level == DiagnosticLevel::Ok
                    && d.message.contains("model")
                    && d.message.contains("claude-sonnet-4-6")
            })
            .count();
        assert!(
            model_ok_count >= 2,
            "Expected OK diags for model prompt and skill.toml. Got: {diags:?}"
        );
    }

    #[test]
    fn test_validate_model_variant_tools_json_warn() {
        let tmp = tempfile::tempdir().unwrap();
        let skill_dir = tmp.path().join("web-search");
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(
            skill_dir.join("skill.toml"),
            r#"
            [skill]
            name = "web-search"
            description = "Search the web"
            "#,
        )
        .unwrap();

        let anthropic_dir = skill_dir.join("anthropic");
        fs::create_dir_all(&anthropic_dir).unwrap();
        fs::write(anthropic_dir.join("system_prompt.md"), "Anthropic prompt.").unwrap();

        let model_dir = anthropic_dir.join("claude-sonnet-4-6");
        fs::create_dir_all(&model_dir).unwrap();
        fs::write(model_dir.join("system_prompt.md"), "Sonnet prompt.").unwrap();
        fs::write(model_dir.join("tools.json"), "[]").unwrap();

        let diags = validate_skill(&skill_dir);
        let tools_warn = diags.iter().find(|d| {
            d.level == DiagnosticLevel::Warn
                && d.message.contains("tools.json")
                && d.message.contains("claude-sonnet-4-6")
                && d.message.contains("not supported")
        });
        assert!(
            tools_warn.is_some(),
            "Expected warning for model tools.json. Got: {diags:?}"
        );
    }

    #[test]
    fn test_validate_model_variant_empty_warn() {
        let tmp = tempfile::tempdir().unwrap();
        let skill_dir = tmp.path().join("web-search");
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(
            skill_dir.join("skill.toml"),
            r#"
            [skill]
            name = "web-search"
            description = "Search the web"
            "#,
        )
        .unwrap();

        let anthropic_dir = skill_dir.join("anthropic");
        fs::create_dir_all(&anthropic_dir).unwrap();
        fs::write(anthropic_dir.join("system_prompt.md"), "Anthropic prompt.").unwrap();

        // Empty model dir
        let model_dir = anthropic_dir.join("claude-opus-4");
        fs::create_dir_all(&model_dir).unwrap();

        let diags = validate_skill(&skill_dir);
        let empty_warn = diags.iter().find(|d| {
            d.level == DiagnosticLevel::Warn
                && d.message.contains("model variant")
                && d.message.contains("claude-opus-4")
                && d.message.contains("empty")
        });
        assert!(
            empty_warn.is_some(),
            "Expected warning for empty model dir. Got: {diags:?}"
        );
    }

    #[test]
    fn test_validate_model_variant_deep_nesting_warn() {
        let tmp = tempfile::tempdir().unwrap();
        let skill_dir = tmp.path().join("web-search");
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(
            skill_dir.join("skill.toml"),
            r#"
            [skill]
            name = "web-search"
            description = "Search the web"
            "#,
        )
        .unwrap();

        let anthropic_dir = skill_dir.join("anthropic");
        fs::create_dir_all(&anthropic_dir).unwrap();
        fs::write(anthropic_dir.join("system_prompt.md"), "Anthropic prompt.").unwrap();

        let model_dir = anthropic_dir.join("claude-sonnet-4-6");
        fs::create_dir_all(&model_dir).unwrap();
        fs::write(model_dir.join("system_prompt.md"), "Sonnet prompt.").unwrap();

        // Create unexpected deep nesting
        let deep_dir = model_dir.join("some-subdir");
        fs::create_dir_all(&deep_dir).unwrap();

        let diags = validate_skill(&skill_dir);
        let nesting_warn = diags.iter().find(|d| {
            d.level == DiagnosticLevel::Warn
                && d.message.contains("unexpected subdirectory")
                && d.message.contains("some-subdir")
        });
        assert!(
            nesting_warn.is_some(),
            "Expected warning for deep nesting. Got: {diags:?}"
        );
    }

    // -- [context] validation tests --

    #[test]
    fn test_validate_skill_context_known_type_ok() {
        let tmp = tempfile::tempdir().unwrap();
        let skill_dir = tmp.path().join("qa-review");
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(
            skill_dir.join("skill.toml"),
            r#"
            [skill]
            name = "qa-review"
            description = "Review PRs"

            [context.pr_diff]
            type = "gh_pr_diff"
            required = true
            "#,
        )
        .unwrap();
        fs::write(
            skill_dir.join("system_prompt.md"),
            "Review the diff: {{pr_diff}}",
        )
        .unwrap();

        let diags = validate_skill(&skill_dir);
        let ok_diag = diags
            .iter()
            .find(|d| d.level == DiagnosticLevel::Ok && d.message.contains("[context.pr_diff]"));
        assert!(
            ok_diag.is_some(),
            "Expected OK diag for valid context. Got: {diags:?}"
        );
        // No fail diagnostics related to context
        let fail_ctx = diags
            .iter()
            .find(|d| d.level == DiagnosticLevel::Fail && d.message.contains("[context"));
        assert!(
            fail_ctx.is_none(),
            "Unexpected FAIL for valid context. Got: {diags:?}"
        );
    }

    #[test]
    fn test_validate_skill_context_unknown_type_fails() {
        let tmp = tempfile::tempdir().unwrap();
        let skill_dir = tmp.path().join("bad-context");
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(
            skill_dir.join("skill.toml"),
            r#"
            [skill]
            name = "bad-context"
            description = "Unknown context type"

            [context.data]
            type = "nonexistent_type"
            "#,
        )
        .unwrap();

        let diags = validate_skill(&skill_dir);
        let fail_diag = diags
            .iter()
            .find(|d| d.level == DiagnosticLevel::Fail && d.message.contains("unknown type"));
        assert!(
            fail_diag.is_some(),
            "Expected FAIL for unknown context type. Got: {diags:?}"
        );
    }

    #[test]
    fn test_validate_skill_context_placeholder_without_declaration_fails() {
        let tmp = tempfile::tempdir().unwrap();
        let skill_dir = tmp.path().join("orphan-placeholder");
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(
            skill_dir.join("skill.toml"),
            r#"
            [skill]
            name = "orphan-placeholder"
            description = "Prompt with undeclared placeholder"
            "#,
        )
        .unwrap();
        fs::write(
            skill_dir.join("system_prompt.md"),
            "Use this data: {{undeclared_var}}",
        )
        .unwrap();

        let diags = validate_skill(&skill_dir);
        let fail_diag = diags.iter().find(|d| {
            d.level == DiagnosticLevel::Fail
                && d.message.contains("{{undeclared_var}}")
                && d.message.contains("no [context.undeclared_var]")
        });
        assert!(
            fail_diag.is_some(),
            "Expected FAIL for undeclared placeholder. Got: {diags:?}"
        );
    }

    #[test]
    fn test_validate_skill_context_declaration_without_placeholder_warns() {
        let tmp = tempfile::tempdir().unwrap();
        let skill_dir = tmp.path().join("unused-context");
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(
            skill_dir.join("skill.toml"),
            r#"
            [skill]
            name = "unused-context"
            description = "Context declared but not used"

            [context.pr_diff]
            type = "gh_pr_diff"
            "#,
        )
        .unwrap();
        fs::write(skill_dir.join("system_prompt.md"), "No placeholders here.").unwrap();

        let diags = validate_skill(&skill_dir);
        let warn_diag = diags.iter().find(|d| {
            d.level == DiagnosticLevel::Warn
                && d.message.contains("[context.pr_diff] declared")
                && d.message.contains("never used")
        });
        assert!(
            warn_diag.is_some(),
            "Expected WARN for unused context declaration. Got: {diags:?}"
        );
    }

    #[test]
    fn test_validate_skill_no_context_no_placeholders_clean() {
        let tmp = tempfile::tempdir().unwrap();
        let skill_dir = tmp.path().join("clean");
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(
            skill_dir.join("skill.toml"),
            r#"
            [skill]
            name = "clean"
            description = "No context, no placeholders"
            "#,
        )
        .unwrap();
        fs::write(skill_dir.join("system_prompt.md"), "Just a prompt.").unwrap();

        let diags = validate_skill(&skill_dir);
        // No context-related fails or warns
        let ctx_issues = diags.iter().filter(|d| {
            (d.level == DiagnosticLevel::Fail || d.level == DiagnosticLevel::Warn)
                && (d.message.contains("[context") || d.message.contains("{{"))
        });
        assert_eq!(
            ctx_issues.count(),
            0,
            "Expected no context issues. Got: {diags:?}"
        );
    }

    // -- PromptVariantSource and ResolvedPrompt tests (#481) --

    #[test]
    fn test_prompt_variant_source_display() {
        assert_eq!(
            PromptVariantSource::HandAuthoredModel.to_string(),
            "hand_authored_model"
        );
        assert_eq!(
            PromptVariantSource::GeneratedModel.to_string(),
            "generated_model"
        );
        assert_eq!(
            PromptVariantSource::GeneratedCanonical.to_string(),
            "generated_canonical"
        );
        assert_eq!(PromptVariantSource::Base.to_string(), "base");
    }

    #[test]
    fn test_resolved_prompt_variant_descriptor_base() {
        let resolved = ResolvedPrompt {
            text: "some prompt",
            source: PromptVariantSource::Base,
            key: None,
        };
        assert_eq!(resolved.variant_descriptor(), "base");
    }

    #[test]
    fn test_resolved_prompt_variant_descriptor_with_key() {
        let resolved = ResolvedPrompt {
            text: "some prompt",
            source: PromptVariantSource::GeneratedModel,
            key: Some("anthropic/claude-sonnet-4-6".to_string()),
        };
        assert_eq!(
            resolved.variant_descriptor(),
            "generated_model:anthropic/claude-sonnet-4-6"
        );
    }

    #[test]
    fn test_resolve_prompt_returns_base_when_no_variants() {
        let entry = SkillEntry {
            manifest: SkillManifest {
                skill: super::super::manifest::SkillInfo {
                    name: "test".to_string(),
                    description: "test".to_string(),
                    version: String::new(),
                    always_on: false,
                    timeout_secs: 30,
                    dependencies: vec![],
                    max_prompt_size: None,
                },
                triggers: super::super::manifest::Triggers { keywords: vec![] },
                llm: Default::default(),
                constraints: Default::default(),
                context: std::collections::HashMap::new(),
            },
            dir: PathBuf::from("/skills/test"),
            keywords_lower: vec![],
            prompt_snippet: "Base prompt.".to_string(),
            skill_tools: vec![],
            enabled: true,
            has_override: false,
            provider_overrides: HashMap::new(),
            model_prompts: HashMap::new(),
            model_overrides: HashMap::new(),
            generated_model_prompts: HashMap::new(),
        };

        let resolved = entry.resolve_prompt("anthropic", "claude-sonnet-4-6");
        assert_eq!(resolved.text, "Base prompt.");
        assert_eq!(resolved.source, PromptVariantSource::Base);
        assert!(resolved.key.is_none());
        assert_eq!(resolved.variant_descriptor(), "base");
    }

    #[test]
    fn test_resolve_prompt_returns_hand_authored_model() {
        let mut entry = SkillEntry {
            manifest: SkillManifest {
                skill: super::super::manifest::SkillInfo {
                    name: "test".to_string(),
                    description: "test".to_string(),
                    version: String::new(),
                    always_on: false,
                    timeout_secs: 30,
                    dependencies: vec![],
                    max_prompt_size: None,
                },
                triggers: super::super::manifest::Triggers { keywords: vec![] },
                llm: Default::default(),
                constraints: Default::default(),
                context: std::collections::HashMap::new(),
            },
            dir: PathBuf::from("/skills/test"),
            keywords_lower: vec![],
            prompt_snippet: "Base prompt.".to_string(),
            skill_tools: vec![],
            enabled: true,
            has_override: false,
            provider_overrides: HashMap::new(),
            model_prompts: HashMap::new(),
            model_overrides: HashMap::new(),
            generated_model_prompts: HashMap::new(),
        };
        entry.model_prompts.insert(
            "anthropic/claude-sonnet-4-6".to_string(),
            "Hand-authored prompt.".to_string(),
        );

        let resolved = entry.resolve_prompt("anthropic", "claude-sonnet-4-6");
        assert_eq!(resolved.text, "Hand-authored prompt.");
        assert_eq!(resolved.source, PromptVariantSource::HandAuthoredModel);
        assert_eq!(resolved.key.as_deref(), Some("anthropic/claude-sonnet-4-6"));
        assert_eq!(
            resolved.variant_descriptor(),
            "hand_authored_model:anthropic/claude-sonnet-4-6"
        );
    }

    #[test]
    fn test_resolve_prompt_returns_generated_model() {
        let mut entry = SkillEntry {
            manifest: SkillManifest {
                skill: super::super::manifest::SkillInfo {
                    name: "test".to_string(),
                    description: "test".to_string(),
                    version: String::new(),
                    always_on: false,
                    timeout_secs: 30,
                    dependencies: vec![],
                    max_prompt_size: None,
                },
                triggers: super::super::manifest::Triggers { keywords: vec![] },
                llm: Default::default(),
                constraints: Default::default(),
                context: std::collections::HashMap::new(),
            },
            dir: PathBuf::from("/skills/test"),
            keywords_lower: vec![],
            prompt_snippet: "Base prompt.".to_string(),
            skill_tools: vec![],
            enabled: true,
            has_override: false,
            provider_overrides: HashMap::new(),
            model_prompts: HashMap::new(),
            model_overrides: HashMap::new(),
            generated_model_prompts: HashMap::new(),
        };
        entry.generated_model_prompts.insert(
            "deepseek/deepseek-v3.2".to_string(),
            "Generated prompt.".to_string(),
        );

        let resolved = entry.resolve_prompt("deepseek", "deepseek-v3.2");
        assert_eq!(resolved.text, "Generated prompt.");
        assert_eq!(resolved.source, PromptVariantSource::GeneratedModel);
        assert_eq!(resolved.key.as_deref(), Some("deepseek/deepseek-v3.2"));
    }

    #[test]
    fn test_resolve_prompt_returns_generated_canonical_for_openrouter() {
        let mut entry = SkillEntry {
            manifest: SkillManifest {
                skill: super::super::manifest::SkillInfo {
                    name: "test".to_string(),
                    description: "test".to_string(),
                    version: String::new(),
                    always_on: false,
                    timeout_secs: 30,
                    dependencies: vec![],
                    max_prompt_size: None,
                },
                triggers: super::super::manifest::Triggers { keywords: vec![] },
                llm: Default::default(),
                constraints: Default::default(),
                context: std::collections::HashMap::new(),
            },
            dir: PathBuf::from("/skills/test"),
            keywords_lower: vec![],
            prompt_snippet: "Base prompt.".to_string(),
            skill_tools: vec![],
            enabled: true,
            has_override: false,
            provider_overrides: HashMap::new(),
            model_prompts: HashMap::new(),
            model_overrides: HashMap::new(),
            generated_model_prompts: HashMap::new(),
        };
        // Generated variant under canonical provider (minimax), not openrouter.
        entry.generated_model_prompts.insert(
            "minimax/minimax-m2.7".to_string(),
            "Canonical generated.".to_string(),
        );

        // Lookup via openrouter — should canonicalize and find the variant.
        let resolved = entry.resolve_prompt("openrouter", "minimax/minimax-m2.7");
        assert_eq!(resolved.text, "Canonical generated.");
        assert_eq!(resolved.source, PromptVariantSource::GeneratedCanonical);
        assert_eq!(resolved.key.as_deref(), Some("minimax/minimax-m2.7"));
        assert_eq!(
            resolved.variant_descriptor(),
            "generated_canonical:minimax/minimax-m2.7"
        );
    }

    #[test]
    fn test_resolve_prompt_hand_authored_beats_generated() {
        let mut entry = SkillEntry {
            manifest: SkillManifest {
                skill: super::super::manifest::SkillInfo {
                    name: "test".to_string(),
                    description: "test".to_string(),
                    version: String::new(),
                    always_on: false,
                    timeout_secs: 30,
                    dependencies: vec![],
                    max_prompt_size: None,
                },
                triggers: super::super::manifest::Triggers { keywords: vec![] },
                llm: Default::default(),
                constraints: Default::default(),
                context: std::collections::HashMap::new(),
            },
            dir: PathBuf::from("/skills/test"),
            keywords_lower: vec![],
            prompt_snippet: "Base prompt.".to_string(),
            skill_tools: vec![],
            enabled: true,
            has_override: false,
            provider_overrides: HashMap::new(),
            model_prompts: HashMap::new(),
            model_overrides: HashMap::new(),
            generated_model_prompts: HashMap::new(),
        };
        let key = "anthropic/claude-sonnet-4-6".to_string();
        entry.model_prompts.insert(key.clone(), "Hand.".to_string());
        entry
            .generated_model_prompts
            .insert(key, "Generated.".to_string());

        let resolved = entry.resolve_prompt("anthropic", "claude-sonnet-4-6");
        assert_eq!(resolved.text, "Hand.");
        assert_eq!(resolved.source, PromptVariantSource::HandAuthoredModel);
    }
}
