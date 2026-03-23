use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use tracing::warn;

use mika_common::claude::ToolDefinition;
use mika_common::llm::ProviderKind;

use super::builtin_handlers::KNOWN_BUILTINS;
use super::manifest::{
    LlmOverride, ProviderSkillFields, ProviderSkillOverride, SkillManifest, SkillToolDef,
    ToolHandler,
};

/// Maximum size for skill.toml files (64 KB).
const MAX_SKILL_TOML_SIZE: u64 = 64 * 1024;

/// Default maximum size for system_prompt.md snippets (16 KB).
const MAX_PROMPT_SNIPPET_SIZE: u64 = 16 * 1024;

/// Hard ceiling for per-skill `max_prompt_size` override (64 KB).
/// Prevents marketplace skills from loading arbitrarily large prompts.
const MAX_PROMPT_SIZE_CEILING: u64 = 64 * 1024;

/// Maximum size for tools.json files (256 KB).
const MAX_TOOLS_JSON_SIZE: u64 = 256 * 1024;

/// A skill tool with its Claude-facing definition and dispatch handler.
#[derive(Debug, Clone)]
pub struct ResolvedSkillTool {
    pub definition: ToolDefinition,
    pub handler: ToolHandler,
    pub skill_dir: PathBuf,
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
    /// Per-skill LLM provider/model override from root `[llm]` section.
    /// Copied from `manifest.llm` at scan time for convenient access.
    pub llm: LlmOverride,
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
    /// Two-level fallback: model-specific > root.
    /// Provider-level prompts are intentionally not supported — models from the
    /// same provider (e.g., gpt-4o vs gpt-5) have different prompt requirements.
    pub fn resolve_prompt(&self, provider: &str, model: &str) -> &str {
        let model_key = format!("{}/{}", provider, sanitize_model_dir_name(model));
        if let Some(prompt) = self.model_prompts.get(&model_key) {
            return prompt;
        }
        &self.prompt_snippet
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

/// Result of scanning a skills directory.
pub struct ScanResult {
    pub entries: Vec<SkillEntry>,
    pub skipped_count: usize,
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
                skipped_count: 0,
            };
        }
    };

    let mut entries = Vec::new();
    let mut skipped_count: usize = 0;
    for dir_entry in read_dir {
        let dir_entry = match dir_entry {
            Ok(de) => de,
            Err(e) => {
                warn!(error = %e, "error reading skills directory entry");
                continue;
            }
        };

        let path = dir_entry.path();

        // Detect broken symlinks (linked skills whose target was removed)
        if let Ok(meta) = std::fs::symlink_metadata(&path)
            && meta.file_type().is_symlink()
            && !path.exists()
        {
            let target = std::fs::read_link(&path).ok();
            let dir_name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("unknown");
            warn!(
                skill = dir_name,
                target = ?target,
                "Broken symlink for skill '{}': target no longer exists. \
                 Reinstall or remove with 'mika skills uninstall {}'",
                dir_name,
                dir_name
            );
            skipped_count += 1;
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
            skipped_count += 1;
            continue;
        }

        let content = match std::fs::read_to_string(&manifest_path) {
            Ok(c) => c,
            Err(e) => {
                warn!(path = %manifest_path.display(), error = %e, "cannot read skill manifest");
                skipped_count += 1;
                continue;
            }
        };

        // Detect legacy format: has [handler] section with type = "builtin"
        if is_legacy_format(&content) {
            warn!(
                path = %manifest_path.display(),
                "skipping legacy-format skill (has [handler] section). \
                 Migrate to new [skill] section format — handler config belongs in tools.json."
            );
            skipped_count += 1;
            continue;
        }

        let manifest: SkillManifest = match toml::from_str(&content) {
            Ok(m) => m,
            Err(e) => {
                warn!(path = %manifest_path.display(), error = %e, "invalid skill manifest");
                skipped_count += 1;
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
        let prompt_snippet = load_snippet_with_limit(&snippet_path, max_size);

        // Check for .disabled marker file
        let enabled = !path.join(".disabled").exists();

        // Parse tools.json if present
        let skill_tools = load_tools_json(&path);

        // Scan for provider and model variant directories
        let variants = scan_provider_variants(&path, &manifest);

        let llm = manifest.llm.clone();
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
            model_overrides: variants.model_overrides,
            llm,
        });
    }

    ScanResult {
        entries,
        skipped_count,
    }
}

/// Diagnostic level for skill validation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiagnosticLevel {
    Ok,
    Warn,
    Fail,
}

/// A single diagnostic finding from skill validation.
#[derive(Debug, Clone)]
pub struct SkillDiagnostic {
    pub level: DiagnosticLevel,
    pub message: String,
}

impl SkillDiagnostic {
    fn ok(msg: impl Into<String>) -> Self {
        Self {
            level: DiagnosticLevel::Ok,
            message: msg.into(),
        }
    }
    fn warn(msg: impl Into<String>) -> Self {
        Self {
            level: DiagnosticLevel::Warn,
            message: msg.into(),
        }
    }
    fn fail(msg: impl Into<String>) -> Self {
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
        if let Some(ref provider_str) = entry.llm.provider {
            if let Ok(pk) = provider_str.parse::<ProviderKind>() {
                let (_, api_key, _) = settings.provider_fields(pk);
                // Ollama doesn't require an API key
                if pk != ProviderKind::Ollama && api_key.filter(|k| !k.trim().is_empty()).is_none()
                {
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
                diags.push(SkillDiagnostic::fail(format!(
                    "system_prompt.md ({} bytes) exceeds limit ({} bytes) — snippet will be skipped at startup",
                    size, effective_limit
                )));
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
    // Legacy format has top-level "handler" table with any "type" field
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

    let read_dir = match std::fs::read_dir(skill_dir) {
        Ok(rd) => rd,
        Err(_) => {
            return VariantScanResult {
                provider_overrides: overrides,
                model_prompts,
                model_overrides,
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
                    let snippet = load_snippet_with_limit(&model_prompt_path, max_size);
                    if !snippet.is_empty() {
                        model_prompts.insert(composite_key.clone(), snippet);
                        model_has_content = true;
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
    }
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

/// Load a prompt snippet file with size limit enforcement.
fn load_snippet_with_limit(path: &Path, max_size: u64) -> String {
    if let Ok(meta) = std::fs::metadata(path)
        && meta.len() > max_size
    {
        warn!(
            path = %path.display(),
            size = meta.len(),
            limit = max_size,
            "prompt snippet exceeds size limit, skipping"
        );
        return String::new();
    }
    std::fs::read_to_string(path).unwrap_or_default()
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
        assert_eq!(scan.skipped_count, 0);
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
        assert_eq!(scan.skipped_count, 1);
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
        assert_eq!(scan.skipped_count, 2); // bad TOML + missing manifest
        assert_eq!(scan.entries[0].manifest.skill.name, "good");
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
        assert_eq!(scan.skipped_count, 1);
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

        let snippet = load_snippet_with_limit(&path, MAX_PROMPT_SNIPPET_SIZE);
        assert_eq!(snippet, "");
    }

    #[test]
    fn test_snippet_size_limit_custom() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("system_prompt.md");
        // 10KB file — under 16KB default, tested with explicit 32KB limit
        let content = "x".repeat(10 * 1024);
        fs::write(&path, &content).unwrap();

        let snippet = load_snippet_with_limit(&path, 32 * 1024);
        assert_eq!(snippet.len(), 10 * 1024);
    }

    #[test]
    fn test_snippet_size_limit_zero_always_skips() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("system_prompt.md");
        fs::write(&path, "tiny").unwrap();

        let snippet = load_snippet_with_limit(&path, 0);
        assert_eq!(snippet, "");
    }

    #[test]
    fn test_snippet_under_default_limit_loads() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("system_prompt.md");
        let content = "x".repeat(15 * 1024); // 15KB, under 16KB default
        fs::write(&path, &content).unwrap();

        let snippet = load_snippet_with_limit(&path, MAX_PROMPT_SNIPPET_SIZE);
        assert_eq!(snippet.len(), 15 * 1024);
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
            llm: Default::default(),
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
            llm: Default::default(),
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
            llm: Default::default(),
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
            llm: Default::default(),
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
            llm: Default::default(),
        };
        entry.model_prompts.insert(
            "anthropic/claude-sonnet-4-6".to_string(),
            "Sonnet prompt.".to_string(),
        );

        // Model-specific wins
        assert_eq!(
            entry.resolve_prompt("anthropic", "claude-sonnet-4-6"),
            "Sonnet prompt."
        );
        // No model variant for opus — falls back to root
        assert_eq!(
            entry.resolve_prompt("anthropic", "claude-opus-4"),
            "Root prompt."
        );
        // No model variant for groq — falls back to root
        assert_eq!(
            entry.resolve_prompt("groq", "llama-3.3-70b-versatile"),
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
            llm: Default::default(),
        };
        entry.model_prompts.insert(
            "openrouter/anthropic--claude-sonnet-4".to_string(),
            "OpenRouter model prompt.".to_string(),
        );

        // Model name with slash gets sanitized, matching the stored key
        assert_eq!(
            entry.resolve_prompt("openrouter", "anthropic/claude-sonnet-4"),
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
            llm: Default::default(),
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
            llm: Default::default(),
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
            llm: Default::default(),
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
            llm: Default::default(),
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
}
