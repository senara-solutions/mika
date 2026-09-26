//! Skill variant management: scanning, validation, diffing, reflection, and promotion.
//!
//! This module provides the shared data model and operations for managing
//! per-provider/per-model prompt variants across skills. It is consumed by
//! both CLI commands and HTTP API handlers.
//!
//! Design principle: **pure computation, no DB access**. All data comes from
//! `SkillEntry` fields and filesystem reads.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use anyhow::{Context, Result, bail};
use mika_common::llm::ProviderKind;
use serde::Serialize;
use tracing::warn;

use super::index::{
    DiagnosticLevel, SkillDiagnostic, effective_prompt_limit, sanitize_model_dir_name,
};
use super::manifest::SkillManifest;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Directory names reserved by the engine. These are never treated as provider
/// names during variant scanning, even if a `ProviderKind` with the same name
/// were to be added in the future (defense-in-depth).
pub const RESERVED_VARIANT_DIRS: &[&str] = &["generated", "experimental"];

/// Minimum ratio of variant size to base prompt size.
/// Variants smaller than 50% of the base are likely truncated or corrupt.
const MIN_VARIANT_RATIO: f64 = 0.5;

// ---------------------------------------------------------------------------
// Variant source types
// ---------------------------------------------------------------------------

/// Source tier of a variant prompt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum VariantSource {
    /// Hand-authored model variant under `<provider>/<model>/`.
    HandAuthored,
    /// Auto-generated variant under `generated/<provider>/<model>/`.
    Generated,
    /// Experimental variant under `experimental/<provider>/<model>/`.
    Experimental,
}

impl std::fmt::Display for VariantSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::HandAuthored => write!(f, "hand-authored"),
            Self::Generated => write!(f, "generated"),
            Self::Experimental => write!(f, "experimental"),
        }
    }
}

// ---------------------------------------------------------------------------
// Variant metadata
// ---------------------------------------------------------------------------

/// Metadata for a single variant prompt file.
#[derive(Debug, Clone, Serialize)]
pub struct VariantMetadata {
    /// Skill name (for cross-skill reports).
    pub skill_name: String,
    /// Provider name (e.g. "anthropic").
    pub provider: String,
    /// Model directory name (sanitized, e.g. "claude-sonnet-4-6").
    pub model: String,
    /// Which tier this variant belongs to.
    pub source: VariantSource,
    /// Absolute path to the `system_prompt.md` file.
    #[serde(skip)]
    pub file_path: PathBuf,
    /// File size in bytes.
    pub size_bytes: u64,
    /// Line count.
    pub line_count: usize,
    /// Last modified timestamp (ISO 8601).
    pub last_modified: Option<String>,
    /// Whether this variant is stale relative to the base prompt.
    /// `None` if staleness cannot be determined (e.g. base missing).
    pub stale: Option<bool>,
}

/// Cross-skill summary of variant counts.
#[derive(Debug, Clone, Serialize)]
pub struct VariantSummary {
    pub skill_name: String,
    pub total: usize,
    pub hand_authored: usize,
    pub generated: usize,
    pub experimental: usize,
    pub stale_count: usize,
}

// ---------------------------------------------------------------------------
// Scanning
// ---------------------------------------------------------------------------

/// Scan a skill directory for ALL variant prompts (hand-authored, generated, experimental).
///
/// This is the management-surface scanner — it discovers variants across all three
/// tiers for listing, diffing, and reflection. Unlike `scan_provider_variants()`
/// (which loads prompts into memory for runtime resolution), this returns metadata
/// without loading prompt content.
pub fn scan_all_variants(skill_dir: &Path, manifest: &SkillManifest) -> Vec<VariantMetadata> {
    let skill_name = &manifest.skill.name;
    let base_mtime = base_prompt_mtime(skill_dir);

    let mut variants = Vec::new();

    // 1. Hand-authored: <skill>/<provider>/<model>/system_prompt.md
    scan_tier(
        skill_dir,
        skill_name,
        VariantSource::HandAuthored,
        base_mtime,
        &mut variants,
    );

    // 2. Generated: <skill>/generated/<provider>/<model>/system_prompt.md
    let generated_root = skill_dir.join("generated");
    scan_tier(
        &generated_root,
        skill_name,
        VariantSource::Generated,
        base_mtime,
        &mut variants,
    );

    // 3. Experimental: <skill>/experimental/<provider>/<model>/system_prompt.md
    let experimental_root = skill_dir.join("experimental");
    scan_tier(
        &experimental_root,
        skill_name,
        VariantSource::Experimental,
        base_mtime,
        &mut variants,
    );

    variants
}

/// Scan a single tier (provider/model tree) for variant prompts.
fn scan_tier(
    root: &Path,
    skill_name: &str,
    source: VariantSource,
    base_mtime: Option<SystemTime>,
    out: &mut Vec<VariantMetadata>,
) {
    let provider_dirs = match std::fs::read_dir(root) {
        Ok(rd) => rd,
        Err(_) => return,
    };

    for provider_entry in provider_dirs.flatten() {
        let provider_path = provider_entry.path();
        if !provider_path.is_dir() {
            continue;
        }
        // Defense-in-depth: skip symlinks
        if std::fs::symlink_metadata(&provider_path)
            .map(|m| m.file_type().is_symlink())
            .unwrap_or(true)
        {
            continue;
        }

        let provider_name = match provider_path.file_name().and_then(|n| n.to_str()) {
            Some(n) if !n.starts_with('.') => n.to_string(),
            _ => continue,
        };

        // Skip reserved directory names
        if RESERVED_VARIANT_DIRS.contains(&provider_name.as_str()) {
            continue;
        }

        // Only recognize known providers
        if provider_name.parse::<ProviderKind>().is_err() {
            continue;
        }

        let model_dirs = match std::fs::read_dir(&provider_path) {
            Ok(rd) => rd,
            Err(_) => continue,
        };

        for model_entry in model_dirs.flatten() {
            let model_path = model_entry.path();
            if !model_path.is_dir() {
                continue;
            }
            // Defense-in-depth: skip symlinks
            if std::fs::symlink_metadata(&model_path)
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
            if !prompt_path.exists() {
                continue;
            }

            let file_meta = match std::fs::metadata(&prompt_path) {
                Ok(m) => m,
                Err(_) => continue,
            };

            let variant_mtime = file_meta.modified().ok();
            let stale = match (base_mtime, variant_mtime) {
                (Some(base), Some(variant)) => Some(variant < base),
                _ => None,
            };

            let line_count = std::fs::read_to_string(&prompt_path)
                .map(|c| c.lines().count())
                .unwrap_or(0);

            out.push(VariantMetadata {
                skill_name: skill_name.to_string(),
                provider: provider_name.clone(),
                model: model_name,
                source,
                file_path: prompt_path,
                size_bytes: file_meta.len(),
                line_count,
                last_modified: variant_mtime.map(format_system_time),
                stale,
            });
        }
    }
}

/// Build a cross-skill summary from variant metadata.
pub fn summarize_variants(variants: &[VariantMetadata]) -> Vec<VariantSummary> {
    let mut by_skill: std::collections::HashMap<String, VariantSummary> =
        std::collections::HashMap::new();

    for v in variants {
        let entry = by_skill
            .entry(v.skill_name.clone())
            .or_insert_with(|| VariantSummary {
                skill_name: v.skill_name.clone(),
                total: 0,
                hand_authored: 0,
                generated: 0,
                experimental: 0,
                stale_count: 0,
            });
        entry.total += 1;
        match v.source {
            VariantSource::HandAuthored => entry.hand_authored += 1,
            VariantSource::Generated => entry.generated += 1,
            VariantSource::Experimental => entry.experimental += 1,
        }
        if v.stale == Some(true) {
            entry.stale_count += 1;
        }
    }

    let mut summaries: Vec<_> = by_skill.into_values().collect();
    summaries.sort_by(|a, b| a.skill_name.cmp(&b.skill_name));
    summaries
}

// ---------------------------------------------------------------------------
// Validation gate (4 rules)
// ---------------------------------------------------------------------------

/// Which validation rule produced a finding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ValidationRule {
    SizeLimit,
    RequiredSections,
    MarkdownWellFormedness,
    ToolReferenceLint,
}

impl std::fmt::Display for ValidationRule {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SizeLimit => write!(f, "size_limit"),
            Self::RequiredSections => write!(f, "required_sections"),
            Self::MarkdownWellFormedness => write!(f, "markdown_well_formedness"),
            Self::ToolReferenceLint => write!(f, "tool_reference_lint"),
        }
    }
}

/// A single validation finding.
#[derive(Debug, Clone, Serialize)]
pub struct ValidationResult {
    pub rule: ValidationRule,
    pub passed: bool,
    pub message: String,
    pub level: DiagnosticLevel,
}

impl ValidationResult {
    fn ok(rule: ValidationRule, msg: impl Into<String>) -> Self {
        Self {
            rule,
            passed: true,
            message: msg.into(),
            level: DiagnosticLevel::Ok,
        }
    }
    fn warn(rule: ValidationRule, msg: impl Into<String>) -> Self {
        Self {
            rule,
            passed: true, // warnings don't block promotion
            message: msg.into(),
            level: DiagnosticLevel::Warn,
        }
    }
    fn fail(rule: ValidationRule, msg: impl Into<String>) -> Self {
        Self {
            rule,
            passed: false,
            message: msg.into(),
            level: DiagnosticLevel::Fail,
        }
    }
}

/// Run the 4-rule validation gate on variant content.
///
/// `base_content` is the base `system_prompt.md` content (for ratio check).
/// `tool_names` is the set of known tool names (for reference lint).
/// `manifest` provides size limits and required sections.
pub fn validate_variant(
    content: &str,
    base_content: Option<&str>,
    manifest: &SkillManifest,
    tool_names: &HashSet<String>,
) -> Vec<ValidationResult> {
    let mut results = Vec::with_capacity(4);

    // Rule 1: Size limit
    let max_size = effective_prompt_limit(manifest.skill.max_prompt_size);
    let content_bytes = content.len() as u64;

    if content_bytes > max_size {
        results.push(ValidationResult::fail(
            ValidationRule::SizeLimit,
            format!(
                "variant is {} bytes, exceeds limit of {} bytes",
                content_bytes, max_size
            ),
        ));
    } else if let Some(base) = base_content {
        let min_size = ((base.len() as f64) * MIN_VARIANT_RATIO).ceil() as u64;
        if content_bytes < min_size {
            results.push(ValidationResult::fail(
                ValidationRule::SizeLimit,
                format!(
                    "variant is {} bytes, below {} minimum ({}% of base {} bytes)",
                    content_bytes,
                    min_size,
                    (MIN_VARIANT_RATIO * 100.0) as u32,
                    base.len()
                ),
            ));
        } else {
            results.push(ValidationResult::ok(
                ValidationRule::SizeLimit,
                format!("variant is {} bytes (within limits)", content_bytes),
            ));
        }
    } else {
        results.push(ValidationResult::ok(
            ValidationRule::SizeLimit,
            format!("variant is {} bytes (within limits)", content_bytes),
        ));
    }

    // Rule 2: Required sections
    let required_sections = manifest
        .variants
        .required_sections
        .as_deref()
        .unwrap_or(&[]);
    if required_sections.is_empty() {
        results.push(ValidationResult::ok(
            ValidationRule::RequiredSections,
            "no required sections configured".to_string(),
        ));
    } else {
        let missing: Vec<&str> = required_sections
            .iter()
            .filter(|section| {
                !content.lines().any(|line| {
                    let trimmed = line.trim_start();
                    // Match any heading level (#, ##, ###, etc.)
                    trimmed.starts_with('#')
                        && trimmed.trim_start_matches('#').starts_with(' ')
                        && trimmed
                            .trim_start_matches('#')
                            .trim()
                            .eq_ignore_ascii_case(section)
                })
            })
            .map(|s| s.as_str())
            .collect();

        if missing.is_empty() {
            results.push(ValidationResult::ok(
                ValidationRule::RequiredSections,
                "all required sections present".to_string(),
            ));
        } else {
            results.push(ValidationResult::fail(
                ValidationRule::RequiredSections,
                format!("missing required sections: {}", missing.join(", ")),
            ));
        }
    }

    // Rule 3: Markdown well-formedness
    match super::validate_markdown_content(content) {
        Ok(()) => {
            results.push(ValidationResult::ok(
                ValidationRule::MarkdownWellFormedness,
                "markdown is well-formed".to_string(),
            ));
        }
        Err(reason) => {
            results.push(ValidationResult::fail(
                ValidationRule::MarkdownWellFormedness,
                format!("markdown issue: {reason}"),
            ));
        }
    }

    // Rule 4: Tool reference lint (warn-only)
    if tool_names.is_empty() {
        results.push(ValidationResult::ok(
            ValidationRule::ToolReferenceLint,
            "no tool names to lint against".to_string(),
        ));
    } else {
        let mut unknown_refs = Vec::new();
        for line in content.lines() {
            // Check for backtick-quoted tool references like `tool_name`
            for cap in line.split('`').enumerate() {
                // Odd indices are inside backticks
                if cap.0 % 2 == 1 {
                    let word = cap.1.trim();
                    if !word.is_empty()
                        && !word.contains(' ')
                        && word.chars().all(|c| c.is_alphanumeric() || c == '_')
                        && !tool_names.contains(word)
                        && looks_like_tool_name(word)
                    {
                        unknown_refs.push(word.to_string());
                    }
                }
            }
        }
        unknown_refs.sort();
        unknown_refs.dedup();

        if unknown_refs.is_empty() {
            results.push(ValidationResult::ok(
                ValidationRule::ToolReferenceLint,
                "no unrecognized tool references found".to_string(),
            ));
        } else {
            results.push(ValidationResult::warn(
                ValidationRule::ToolReferenceLint,
                format!(
                    "unrecognized tool references (may be false positives): {}",
                    unknown_refs.join(", ")
                ),
            ));
        }
    }

    results
}

/// Heuristic: does this identifier look like a tool name?
/// Tool names typically use snake_case and are not common English words.
fn looks_like_tool_name(word: &str) -> bool {
    word.contains('_') && word.len() >= 3
}

/// Check if any validation results have failures (not warnings).
pub fn has_failures(results: &[ValidationResult]) -> bool {
    results.iter().any(|r| !r.passed)
}

/// Convert validation results to `SkillDiagnostic` for consistent CLI output.
pub fn to_diagnostics(results: &[ValidationResult]) -> Vec<SkillDiagnostic> {
    results
        .iter()
        .map(|r| match r.level {
            DiagnosticLevel::Ok => SkillDiagnostic::ok(&r.message),
            DiagnosticLevel::Warn => SkillDiagnostic::warn(&r.message),
            DiagnosticLevel::Fail => SkillDiagnostic::fail(&r.message),
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Diff and reflection
// ---------------------------------------------------------------------------

/// Classification of changes between a variant and its base.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DiffClassification {
    /// Only whitespace or formatting differences.
    Cosmetic,
    /// Text content changes without structural heading changes.
    Content,
    /// Heading additions or removals indicating structural change.
    Structural,
    /// No differences.
    Identical,
}

impl std::fmt::Display for DiffClassification {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Cosmetic => write!(f, "cosmetic"),
            Self::Content => write!(f, "content"),
            Self::Structural => write!(f, "structural"),
            Self::Identical => write!(f, "identical"),
        }
    }
}

/// Report from diffing a variant against its base prompt.
#[derive(Debug, Clone, Serialize)]
pub struct DiffReport {
    pub classification: DiffClassification,
    pub added_lines: usize,
    pub removed_lines: usize,
    pub missing_base_sections: Vec<String>,
    /// Unified diff text for display.
    pub unified_diff: String,
}

/// Compute a diff between base and variant content.
pub fn diff_variant(base_content: &str, variant_content: &str) -> DiffReport {
    if base_content == variant_content {
        return DiffReport {
            classification: DiffClassification::Identical,
            added_lines: 0,
            removed_lines: 0,
            missing_base_sections: vec![],
            unified_diff: String::new(),
        };
    }

    // Simple line-by-line diff
    let base_lines: Vec<&str> = base_content.lines().collect();
    let variant_lines: Vec<&str> = variant_content.lines().collect();

    // Count added/removed lines using LCS-based approach
    let (added, removed) = count_diff_lines(&base_lines, &variant_lines);

    // Build unified diff output
    let unified_diff = build_unified_diff(&base_lines, &variant_lines);

    // Check for heading changes (structural)
    let base_headings = extract_headings(base_content);
    let variant_headings = extract_headings(variant_content);

    let missing_base_sections: Vec<String> = base_headings
        .iter()
        .filter(|h| !variant_headings.contains(h))
        .cloned()
        .collect();

    let has_heading_changes =
        !missing_base_sections.is_empty() || base_headings.len() != variant_headings.len();

    // Classify
    let classification = if has_heading_changes {
        DiffClassification::Structural
    } else if is_whitespace_only_diff(base_content, variant_content) {
        DiffClassification::Cosmetic
    } else {
        DiffClassification::Content
    };

    DiffReport {
        classification,
        added_lines: added,
        removed_lines: removed,
        missing_base_sections,
        unified_diff,
    }
}

/// Extract markdown headings (any level) from content.
fn extract_headings(content: &str) -> Vec<String> {
    content
        .lines()
        .filter(|line| {
            let t = line.trim_start();
            t.starts_with('#') && t.trim_start_matches('#').starts_with(' ')
        })
        .map(|line| line.trim_start().trim_start_matches('#').trim().to_string())
        .collect()
}

/// Check if differences are whitespace-only.
fn is_whitespace_only_diff(a: &str, b: &str) -> bool {
    let normalize =
        |s: &str| -> String { s.lines().map(|l| l.trim()).collect::<Vec<_>>().join("\n") };
    normalize(a) == normalize(b)
}

/// Count added and removed lines using frequency maps (handles duplicate lines correctly).
fn count_diff_lines(base: &[&str], variant: &[&str]) -> (usize, usize) {
    let mut base_freq: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    for line in base {
        *base_freq.entry(line).or_insert(0) += 1;
    }
    let mut variant_freq: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    for line in variant {
        *variant_freq.entry(line).or_insert(0) += 1;
    }

    let mut removed = 0;
    for (line, &count) in &base_freq {
        let var_count = variant_freq.get(line).copied().unwrap_or(0);
        if count > var_count {
            removed += count - var_count;
        }
    }
    let mut added = 0;
    for (line, &count) in &variant_freq {
        let base_count = base_freq.get(line).copied().unwrap_or(0);
        if count > base_count {
            added += count - base_count;
        }
    }

    (added, removed)
}

/// Build a simple unified diff string.
fn build_unified_diff(base: &[&str], variant: &[&str]) -> String {
    let mut out = String::new();
    out.push_str("--- base/system_prompt.md\n");
    out.push_str("+++ variant/system_prompt.md\n");

    // Simple approach: show context around changes
    let base_set: HashSet<&str> = base.iter().copied().collect();
    let variant_set: HashSet<&str> = variant.iter().copied().collect();

    // Show removed lines
    for line in base {
        if !variant_set.contains(*line) {
            out.push('-');
            out.push_str(line);
            out.push('\n');
        }
    }

    // Show added lines
    for line in variant {
        if !base_set.contains(*line) {
            out.push('+');
            out.push_str(line);
            out.push('\n');
        }
    }

    out
}

// ---------------------------------------------------------------------------
// Reflection
// ---------------------------------------------------------------------------

/// Recommended action for a variant after a base edit.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RecommendedAction {
    /// Variant is up to date, no action needed.
    UpToDate,
    /// Variant should be re-reviewed after base changes.
    ReReview,
    /// Variant should be regenerated (significant structural drift).
    ConsiderRegenerate,
}

impl std::fmt::Display for RecommendedAction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UpToDate => write!(f, "Variant is up to date"),
            Self::ReReview => write!(f, "Re-review variant after base changes"),
            Self::ConsiderRegenerate => write!(f, "Consider regenerating variant"),
        }
    }
}

/// Per-variant impact assessment in a reflection report.
#[derive(Debug, Clone, Serialize)]
pub struct VariantImpact {
    pub metadata: VariantMetadata,
    pub diff_summary: DiffSummary,
    pub recommendation: RecommendedAction,
}

/// Compact diff summary for reflection reports.
#[derive(Debug, Clone, Serialize)]
pub struct DiffSummary {
    pub classification: DiffClassification,
    pub added_lines: usize,
    pub removed_lines: usize,
    pub missing_sections: Vec<String>,
}

/// Reflection report for a skill after a base prompt edit.
#[derive(Debug, Clone, Serialize)]
pub struct ReflectionReport {
    pub skill_name: String,
    pub base_path: PathBuf,
    pub base_last_modified: Option<String>,
    pub impacts: Vec<VariantImpact>,
}

/// Generate a reflection report for a skill.
///
/// Reads the base prompt and all production variants (generated + hand-authored),
/// computes diffs, and produces per-variant impact assessments with recommendations.
/// Pure computation — never writes any files.
pub fn reflect_skill(skill_dir: &Path, manifest: &SkillManifest) -> Result<ReflectionReport> {
    let base_path = skill_dir.join("system_prompt.md");
    let base_content = std::fs::read_to_string(&base_path)
        .with_context(|| format!("cannot read base prompt: {}", base_path.display()))?;

    let base_mtime = std::fs::metadata(&base_path)
        .ok()
        .and_then(|m| m.modified().ok());

    let all_variants = scan_all_variants(skill_dir, manifest);

    // Reflect on production variants only (hand-authored + generated)
    let production_variants: Vec<_> = all_variants
        .into_iter()
        .filter(|v| v.source != VariantSource::Experimental)
        .collect();

    let mut impacts = Vec::new();

    for variant in production_variants {
        let variant_content = match std::fs::read_to_string(&variant.file_path) {
            Ok(c) => c,
            Err(e) => {
                warn!(
                    path = %variant.file_path.display(),
                    error = %e,
                    "cannot read variant for reflection"
                );
                continue;
            }
        };

        let diff = diff_variant(&base_content, &variant_content);

        let recommendation = match diff.classification {
            DiffClassification::Identical => RecommendedAction::UpToDate,
            DiffClassification::Cosmetic => RecommendedAction::UpToDate,
            DiffClassification::Content => {
                if variant.stale == Some(true) {
                    RecommendedAction::ReReview
                } else {
                    RecommendedAction::UpToDate
                }
            }
            DiffClassification::Structural => {
                if !diff.missing_base_sections.is_empty() {
                    RecommendedAction::ConsiderRegenerate
                } else {
                    RecommendedAction::ReReview
                }
            }
        };

        impacts.push(VariantImpact {
            metadata: variant,
            diff_summary: DiffSummary {
                classification: diff.classification,
                added_lines: diff.added_lines,
                removed_lines: diff.removed_lines,
                missing_sections: diff.missing_base_sections,
            },
            recommendation,
        });
    }

    Ok(ReflectionReport {
        skill_name: manifest.skill.name.clone(),
        base_path,
        base_last_modified: base_mtime.map(format_system_time),
        impacts,
    })
}

// ---------------------------------------------------------------------------
// Promote
// ---------------------------------------------------------------------------

/// Result of a successful variant promotion.
#[derive(Debug, Clone, Serialize)]
pub struct PromoteResult {
    pub source_path: PathBuf,
    pub target_path: PathBuf,
    pub warnings: Vec<String>,
}

/// Promote an experimental variant to generated.
///
/// Steps:
/// 1. Verify experimental source exists
/// 2. Check for hand-authored variant shadow
/// 3. Validate content (4-rule gate)
/// 4. Copy to generated/
/// 5. Clean up experimental source (expected files only)
pub fn promote_variant(
    skill_dir: &Path,
    manifest: &SkillManifest,
    provider: &str,
    model: &str,
    base_content: Option<&str>,
    tool_names: &HashSet<String>,
) -> Result<PromoteResult> {
    let sanitized_model = sanitize_model_dir_name(model);
    let mut warnings = Vec::new();

    // Check for symlinked skill directory
    if let Ok(meta) = std::fs::symlink_metadata(skill_dir)
        && meta.file_type().is_symlink()
    {
        warnings.push(format!(
            "skill directory is a symlink (--link mode) — writes will affect the shared source at {}",
            std::fs::read_link(skill_dir).unwrap_or_else(|_| skill_dir.to_path_buf()).display()
        ));
    }

    // 1. Verify experimental source
    let experimental_dir = skill_dir
        .join("experimental")
        .join(provider)
        .join(&sanitized_model);
    let source_prompt = experimental_dir.join("system_prompt.md");

    if !source_prompt.exists() {
        bail!(
            "experimental variant not found: {}",
            source_prompt.display()
        );
    }

    // 2. Check for hand-authored variant shadow
    let hand_authored_prompt = skill_dir
        .join(provider)
        .join(&sanitized_model)
        .join("system_prompt.md");
    if hand_authored_prompt.exists() {
        bail!(
            "hand-authored variant exists at {} and would shadow the promoted variant at runtime. \
             Remove the hand-authored variant first.",
            hand_authored_prompt.display()
        );
    }

    // 3. Validate content
    let content = std::fs::read_to_string(&source_prompt).with_context(|| {
        format!(
            "cannot read experimental variant: {}",
            source_prompt.display()
        )
    })?;

    let validation = validate_variant(&content, base_content, manifest, tool_names);
    if has_failures(&validation) {
        let violations: Vec<String> = validation
            .iter()
            .filter(|r| !r.passed)
            .map(|r| format!("{}: {}", r.rule, r.message))
            .collect();
        bail!(
            "experimental variant failed validation:\n  {}",
            violations.join("\n  ")
        );
    }

    // 4. Copy to generated
    let target_dir = skill_dir
        .join("generated")
        .join(provider)
        .join(&sanitized_model);
    std::fs::create_dir_all(&target_dir)
        .with_context(|| format!("cannot create target directory: {}", target_dir.display()))?;

    let target_prompt = target_dir.join("system_prompt.md");
    std::fs::copy(&source_prompt, &target_prompt).with_context(|| {
        format!(
            "cannot copy variant from {} to {}",
            source_prompt.display(),
            target_prompt.display()
        )
    })?;

    // Also copy skill.toml override if present
    let source_toml = experimental_dir.join("skill.toml");
    if source_toml.exists() {
        let target_toml = target_dir.join("skill.toml");
        std::fs::copy(&source_toml, &target_toml).with_context(|| {
            format!(
                "cannot copy skill.toml from {} to {}",
                source_toml.display(),
                target_toml.display()
            )
        })?;
    }

    // 5. Clean up experimental source (expected files only)
    for expected_file in &["system_prompt.md", "skill.toml"] {
        let file = experimental_dir.join(expected_file);
        if file.exists()
            && let Err(e) = std::fs::remove_file(&file)
        {
            warnings.push(format!(
                "cannot remove experimental file {}: {e}",
                file.display()
            ));
        }
    }

    // Check for unexpected files before removing directories
    if let Ok(remaining) = std::fs::read_dir(&experimental_dir) {
        let unexpected: Vec<_> = remaining.flatten().collect();
        if !unexpected.is_empty() {
            warnings.push(format!(
                "experimental directory {} contains {} unexpected file(s), not removing directory",
                experimental_dir.display(),
                unexpected.len()
            ));
        } else {
            // Remove model directory if empty
            let _ = std::fs::remove_dir(&experimental_dir);
            // Remove provider directory if empty
            let provider_dir = skill_dir.join("experimental").join(provider);
            let _ = std::fs::remove_dir(&provider_dir);
            // Remove experimental root if empty
            let experimental_root = skill_dir.join("experimental");
            let _ = std::fs::remove_dir(&experimental_root);
        }
    }

    Ok(PromoteResult {
        source_path: source_prompt,
        target_path: target_prompt,
        warnings,
    })
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Get the base prompt's mtime.
fn base_prompt_mtime(skill_dir: &Path) -> Option<SystemTime> {
    let base_path = skill_dir.join("system_prompt.md");
    std::fs::metadata(&base_path)
        .ok()
        .and_then(|m| m.modified().ok())
}

/// Format a `SystemTime` as ISO 8601 UTC.
fn format_system_time(t: SystemTime) -> String {
    let dur = t.duration_since(SystemTime::UNIX_EPOCH).unwrap_or_default();
    let secs = dur.as_secs();
    // Simple UTC formatting
    let (year, month, day, hour, min, sec) = timestamp_parts(secs);
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{min:02}:{sec:02}Z")
}

/// Break Unix timestamp into date/time components.
fn timestamp_parts(secs: u64) -> (u64, u64, u64, u64, u64, u64) {
    let sec = secs % 60;
    let min = (secs / 60) % 60;
    let hour = (secs / 3600) % 24;

    // Days since epoch
    let mut days = secs / 86400;
    let mut year = 1970u64;

    loop {
        let days_in_year = if is_leap(year) { 366 } else { 365 };
        if days < days_in_year {
            break;
        }
        days -= days_in_year;
        year += 1;
    }

    let leap = is_leap(year);
    let month_days = [
        31,
        if leap { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    let mut month = 12u64; // Default to December; overwritten if days < month boundary
    for (i, &md) in month_days.iter().enumerate() {
        if days < md {
            month = i as u64 + 1;
            break;
        }
        days -= md;
    }
    let day = days + 1;

    (year, month, day, hour, min, sec)
}

fn is_leap(year: u64) -> bool {
    (year.is_multiple_of(4) && !year.is_multiple_of(100)) || year.is_multiple_of(400)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::skills::manifest::{Constraints, LlmOverride, SkillInfo, Triggers, VariantsConfig};
    use std::io::Write;
    use tempfile::TempDir;

    fn make_manifest(name: &str) -> SkillManifest {
        SkillManifest {
            skill: SkillInfo {
                name: name.to_string(),
                description: "Test skill".to_string(),
                version: "0.1.0".to_string(),
                always_on: false,
                timeout_secs: 30,
                dependencies: vec![],
                max_prompt_size: None,
                data_grade: Default::default(),
            },
            triggers: Triggers::default(),
            llm: LlmOverride::default(),
            constraints: Constraints::default(),
            output: Default::default(),
            context: Default::default(),
            variants: VariantsConfig::default(),
        }
    }

    fn write_file(dir: &Path, content: &str) {
        if let Some(parent) = dir.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        let mut f = std::fs::File::create(dir).unwrap();
        f.write_all(content.as_bytes()).unwrap();
    }

    fn create_skill_dir(tmp: &TempDir) -> PathBuf {
        let skill_dir = tmp.path().join("test-skill");
        std::fs::create_dir_all(&skill_dir).unwrap();
        write_file(
            &skill_dir.join("system_prompt.md"),
            "# Test Skill\n\n## Tools\n\nUse `search_memory` to find facts.\n\n## Constraints\n\nBe concise.\n",
        );
        write_file(
            &skill_dir.join("skill.toml"),
            "[skill]\nname = \"test-skill\"\ndescription = \"Test\"\nversion = \"0.1.0\"\n",
        );
        skill_dir
    }

    // -- Scanning tests --

    #[test]
    fn test_scan_empty_skill_dir() {
        let tmp = TempDir::new().unwrap();
        let skill_dir = create_skill_dir(&tmp);
        let manifest = make_manifest("test-skill");

        let variants = scan_all_variants(&skill_dir, &manifest);
        assert!(variants.is_empty(), "no variant dirs → empty list");
    }

    #[test]
    fn test_scan_all_three_tiers() {
        let tmp = TempDir::new().unwrap();
        let skill_dir = create_skill_dir(&tmp);
        let manifest = make_manifest("test-skill");

        // Hand-authored
        write_file(
            &skill_dir
                .join("anthropic")
                .join("claude-sonnet-4-6")
                .join("system_prompt.md"),
            "# Hand authored variant\n",
        );

        // Generated
        write_file(
            &skill_dir
                .join("generated")
                .join("anthropic")
                .join("claude-sonnet-4-6")
                .join("system_prompt.md"),
            "# Generated variant\n",
        );

        // Experimental
        write_file(
            &skill_dir
                .join("experimental")
                .join("openai")
                .join("gpt-4o")
                .join("system_prompt.md"),
            "# Experimental variant\n",
        );

        let variants = scan_all_variants(&skill_dir, &manifest);
        assert_eq!(variants.len(), 3, "should find all three tiers");

        let hand = variants
            .iter()
            .find(|v| v.source == VariantSource::HandAuthored);
        assert!(hand.is_some(), "should find hand-authored");
        assert_eq!(hand.unwrap().provider, "anthropic");

        let generated = variants
            .iter()
            .find(|v| v.source == VariantSource::Generated);
        assert!(generated.is_some(), "should find generated");

        let exp = variants
            .iter()
            .find(|v| v.source == VariantSource::Experimental);
        assert!(exp.is_some(), "should find experimental");
        assert_eq!(exp.unwrap().provider, "openai");
    }

    #[test]
    fn test_scan_skips_unknown_provider() {
        let tmp = TempDir::new().unwrap();
        let skill_dir = create_skill_dir(&tmp);
        let manifest = make_manifest("test-skill");

        // Unknown provider
        write_file(
            &skill_dir
                .join("unknown-provider")
                .join("some-model")
                .join("system_prompt.md"),
            "# Unknown\n",
        );

        let variants = scan_all_variants(&skill_dir, &manifest);
        assert!(variants.is_empty(), "unknown provider should be skipped");
    }

    #[test]
    fn test_scan_skips_dotfiles() {
        let tmp = TempDir::new().unwrap();
        let skill_dir = create_skill_dir(&tmp);
        let manifest = make_manifest("test-skill");

        // Dotdir
        write_file(
            &skill_dir
                .join(".hidden")
                .join("model")
                .join("system_prompt.md"),
            "# Hidden\n",
        );

        let variants = scan_all_variants(&skill_dir, &manifest);
        assert!(variants.is_empty(), "dotdirs should be skipped");
    }

    #[test]
    fn test_staleness_detection() {
        let tmp = TempDir::new().unwrap();
        let skill_dir = create_skill_dir(&tmp);
        let manifest = make_manifest("test-skill");

        // Create variant BEFORE touching base (so variant is older)
        write_file(
            &skill_dir
                .join("anthropic")
                .join("claude-sonnet-4-6")
                .join("system_prompt.md"),
            "# Old variant\n",
        );

        // Small delay and touch the base
        std::thread::sleep(std::time::Duration::from_millis(50));
        write_file(
            &skill_dir.join("system_prompt.md"),
            "# Updated base\n\n## Tools\n\nNew content.\n",
        );

        let variants = scan_all_variants(&skill_dir, &manifest);
        assert_eq!(variants.len(), 1);
        assert_eq!(variants[0].stale, Some(true), "variant should be stale");
    }

    #[test]
    fn test_scan_skips_reserved_dirs_in_hand_authored() {
        let tmp = TempDir::new().unwrap();
        let skill_dir = create_skill_dir(&tmp);
        let manifest = make_manifest("test-skill");

        // The `generated` and `experimental` directories under the skill root
        // should NOT be treated as provider directories in the hand-authored tier
        write_file(
            &skill_dir
                .join("generated")
                .join("anthropic")
                .join("model")
                .join("system_prompt.md"),
            "# This is generated, not hand-authored\n",
        );

        let variants = scan_all_variants(&skill_dir, &manifest);
        // Should find the variant only as Generated, not as HandAuthored
        assert_eq!(variants.len(), 1);
        assert_eq!(variants[0].source, VariantSource::Generated);
    }

    // -- Summary tests --

    #[test]
    fn test_summarize_variants() {
        let variants = vec![
            VariantMetadata {
                skill_name: "skill-a".to_string(),
                provider: "anthropic".to_string(),
                model: "claude-sonnet-4-6".to_string(),
                source: VariantSource::HandAuthored,
                file_path: PathBuf::new(),
                size_bytes: 100,
                line_count: 10,
                last_modified: None,
                stale: Some(true),
            },
            VariantMetadata {
                skill_name: "skill-a".to_string(),
                provider: "openai".to_string(),
                model: "gpt-4o".to_string(),
                source: VariantSource::Generated,
                file_path: PathBuf::new(),
                size_bytes: 200,
                line_count: 20,
                last_modified: None,
                stale: Some(false),
            },
            VariantMetadata {
                skill_name: "skill-b".to_string(),
                provider: "anthropic".to_string(),
                model: "claude-sonnet-4-6".to_string(),
                source: VariantSource::Experimental,
                file_path: PathBuf::new(),
                size_bytes: 150,
                line_count: 15,
                last_modified: None,
                stale: None,
            },
        ];

        let summaries = summarize_variants(&variants);
        assert_eq!(summaries.len(), 2);

        let a = summaries
            .iter()
            .find(|s| s.skill_name == "skill-a")
            .unwrap();
        assert_eq!(a.total, 2);
        assert_eq!(a.hand_authored, 1);
        assert_eq!(a.generated, 1);
        assert_eq!(a.stale_count, 1);

        let b = summaries
            .iter()
            .find(|s| s.skill_name == "skill-b")
            .unwrap();
        assert_eq!(b.total, 1);
        assert_eq!(b.experimental, 1);
    }

    // -- Validation tests --

    #[test]
    fn test_validate_variant_all_pass() {
        let manifest = make_manifest("test");
        let content = "# Test\n\nSome content here.\n";
        let base = "# Base\n\nBase content here.\n";
        let tools = HashSet::new();

        let results = validate_variant(content, Some(base), &manifest, &tools);
        assert!(results.iter().all(|r| r.passed), "all should pass");
    }

    #[test]
    fn test_validate_variant_size_exceeds_limit() {
        let mut manifest = make_manifest("test");
        manifest.skill.max_prompt_size = Some(10); // Very small limit
        let content = "# Test\n\nThis is definitely more than 10 bytes.\n";
        let tools = HashSet::new();

        let results = validate_variant(content, None, &manifest, &tools);
        let size_result = results
            .iter()
            .find(|r| r.rule == ValidationRule::SizeLimit)
            .unwrap();
        assert!(!size_result.passed, "should fail size limit");
    }

    #[test]
    fn test_validate_variant_below_min_ratio() {
        let manifest = make_manifest("test");
        let base = "x".repeat(1000);
        let content = "tiny"; // Way below 50% of base
        let tools = HashSet::new();

        let results = validate_variant(content, Some(&base), &manifest, &tools);
        let size_result = results
            .iter()
            .find(|r| r.rule == ValidationRule::SizeLimit)
            .unwrap();
        assert!(!size_result.passed, "should fail min ratio check");
    }

    #[test]
    fn test_validate_variant_no_required_sections() {
        let manifest = make_manifest("test");
        let content = "# Test\n\nContent.\n";
        let tools = HashSet::new();

        let results = validate_variant(content, None, &manifest, &tools);
        let section_result = results
            .iter()
            .find(|r| r.rule == ValidationRule::RequiredSections)
            .unwrap();
        assert!(
            section_result.passed,
            "should pass when no sections required"
        );
    }

    #[test]
    fn test_validate_variant_missing_required_section() {
        let mut manifest = make_manifest("test");
        manifest.variants.required_sections = Some(vec!["Tools".to_string(), "Setup".to_string()]);
        let content = "# Test\n\n## Tools\n\nSome tools.\n";
        let tools = HashSet::new();

        let results = validate_variant(content, None, &manifest, &tools);
        let section_result = results
            .iter()
            .find(|r| r.rule == ValidationRule::RequiredSections)
            .unwrap();
        assert!(
            !section_result.passed,
            "should fail when 'Setup' is missing"
        );
        assert!(
            section_result.message.contains("Setup"),
            "should list missing section"
        );
    }

    #[test]
    fn test_validate_variant_markdown_wellformedness() {
        let manifest = make_manifest("test");
        let tools = HashSet::new();

        // Unclosed fence
        let content = "# Test\n\n```rust\nfn main() {}\n";
        let results = validate_variant(content, None, &manifest, &tools);
        let md_result = results
            .iter()
            .find(|r| r.rule == ValidationRule::MarkdownWellFormedness)
            .unwrap();
        assert!(!md_result.passed, "unclosed fence should fail");

        // Null bytes
        let content = "# Test\n\nHello\0world\n";
        let results = validate_variant(content, None, &manifest, &tools);
        let md_result = results
            .iter()
            .find(|r| r.rule == ValidationRule::MarkdownWellFormedness)
            .unwrap();
        assert!(!md_result.passed, "null bytes should fail");
    }

    #[test]
    fn test_validate_variant_tool_reference_lint() {
        let manifest = make_manifest("test");
        let mut tools = HashSet::new();
        tools.insert("search_memory".to_string());
        tools.insert("store_fact".to_string());

        // Reference to known tool - pass
        let content = "# Test\n\nUse `search_memory` for lookups.\n";
        let results = validate_variant(content, None, &manifest, &tools);
        let lint_result = results
            .iter()
            .find(|r| r.rule == ValidationRule::ToolReferenceLint)
            .unwrap();
        assert!(lint_result.passed, "known tool should pass");
        assert_eq!(lint_result.level, DiagnosticLevel::Ok);

        // Reference to unknown tool - warn (still passes)
        let content = "# Test\n\nUse `unknown_tool` for something.\n";
        let results = validate_variant(content, None, &manifest, &tools);
        let lint_result = results
            .iter()
            .find(|r| r.rule == ValidationRule::ToolReferenceLint)
            .unwrap();
        assert!(lint_result.passed, "unknown tool should still pass (warn)");
        assert_eq!(
            lint_result.level,
            DiagnosticLevel::Warn,
            "should be a warning"
        );
    }

    #[test]
    fn test_validate_variant_empty_tools() {
        let manifest = make_manifest("test");
        let tools = HashSet::new();

        let content = "# Test\n\nUse `some_tool` for something.\n";
        let results = validate_variant(content, None, &manifest, &tools);
        let lint_result = results
            .iter()
            .find(|r| r.rule == ValidationRule::ToolReferenceLint)
            .unwrap();
        assert!(lint_result.passed, "empty tools should pass trivially");
    }

    // -- Diff tests --

    #[test]
    fn test_diff_identical() {
        let content = "# Test\n\nSame content.\n";
        let report = diff_variant(content, content);
        assert_eq!(report.classification, DiffClassification::Identical);
        assert_eq!(report.added_lines, 0);
        assert_eq!(report.removed_lines, 0);
    }

    #[test]
    fn test_diff_content_changes() {
        let base = "# Test\n\nOld content.\n";
        let variant = "# Test\n\nNew content.\n";
        let report = diff_variant(base, variant);
        assert_eq!(report.classification, DiffClassification::Content);
        assert!(report.added_lines > 0 || report.removed_lines > 0);
    }

    #[test]
    fn test_diff_structural_missing_section() {
        let base = "# Test\n\n## Tools\n\nTools here.\n\n## Constraints\n\nConstraints here.\n";
        let variant = "# Test\n\n## Tools\n\nTools here.\n";
        let report = diff_variant(base, variant);
        assert_eq!(report.classification, DiffClassification::Structural);
        assert!(
            report
                .missing_base_sections
                .contains(&"Constraints".to_string())
        );
    }

    #[test]
    fn test_diff_cosmetic_whitespace() {
        let base = "# Test\n\nContent here.\n";
        let variant = "# Test\n\n  Content here.  \n";
        let report = diff_variant(base, variant);
        assert_eq!(report.classification, DiffClassification::Cosmetic);
    }

    // -- Reflection tests --

    #[test]
    fn test_reflect_no_variants() {
        let tmp = TempDir::new().unwrap();
        let skill_dir = create_skill_dir(&tmp);
        let manifest = make_manifest("test-skill");

        let report = reflect_skill(&skill_dir, &manifest).unwrap();
        assert!(report.impacts.is_empty());
    }

    #[test]
    fn test_reflect_with_stale_variant() {
        let tmp = TempDir::new().unwrap();
        let skill_dir = create_skill_dir(&tmp);
        let manifest = make_manifest("test-skill");

        // Create variant first (older)
        write_file(
            &skill_dir
                .join("anthropic")
                .join("claude-sonnet-4-6")
                .join("system_prompt.md"),
            "# Old variant\n\nOld content.\n",
        );

        std::thread::sleep(std::time::Duration::from_millis(50));

        // Touch base (newer)
        write_file(
            &skill_dir.join("system_prompt.md"),
            "# Test Skill\n\n## Tools\n\nUpdated tools.\n\n## New Section\n\nNew content.\n",
        );

        let report = reflect_skill(&skill_dir, &manifest).unwrap();
        assert_eq!(report.impacts.len(), 1);

        let impact = &report.impacts[0];
        assert_eq!(impact.metadata.stale, Some(true));
    }

    #[test]
    fn test_reflect_excludes_experimental() {
        let tmp = TempDir::new().unwrap();
        let skill_dir = create_skill_dir(&tmp);
        let manifest = make_manifest("test-skill");

        // Experimental variant should NOT appear in reflection
        write_file(
            &skill_dir
                .join("experimental")
                .join("anthropic")
                .join("claude-sonnet-4-6")
                .join("system_prompt.md"),
            "# Experimental\n",
        );

        let report = reflect_skill(&skill_dir, &manifest).unwrap();
        assert!(
            report.impacts.is_empty(),
            "experimental variants excluded from reflection"
        );
    }

    // -- Promote tests --

    #[test]
    fn test_promote_success() {
        let tmp = TempDir::new().unwrap();
        let skill_dir = create_skill_dir(&tmp);
        let manifest = make_manifest("test-skill");

        let content = "# Promoted Variant\n\n## Tools\n\nUse tools wisely.\n\n## Constraints\n\nBe concise and precise in responses.\n";
        write_file(
            &skill_dir
                .join("experimental")
                .join("anthropic")
                .join("claude-sonnet-4-6")
                .join("system_prompt.md"),
            content,
        );

        let base = std::fs::read_to_string(skill_dir.join("system_prompt.md")).unwrap();
        let tools = HashSet::new();

        let result = promote_variant(
            &skill_dir,
            &manifest,
            "anthropic",
            "claude-sonnet-4-6",
            Some(&base),
            &tools,
        )
        .unwrap();

        // Target should exist
        assert!(result.target_path.exists());
        let target_content = std::fs::read_to_string(&result.target_path).unwrap();
        assert_eq!(target_content, content);

        // Source should be gone
        assert!(!result.source_path.exists());
    }

    #[test]
    fn test_promote_with_skill_toml() {
        let tmp = TempDir::new().unwrap();
        let skill_dir = create_skill_dir(&tmp);
        let manifest = make_manifest("test-skill");

        write_file(
            &skill_dir
                .join("experimental")
                .join("anthropic")
                .join("claude-sonnet-4-6")
                .join("system_prompt.md"),
            "# Variant\n\n## Tools\n\nUse search tools.\n\n## Constraints\n\nBe precise in all responses.\n",
        );
        write_file(
            &skill_dir
                .join("experimental")
                .join("anthropic")
                .join("claude-sonnet-4-6")
                .join("skill.toml"),
            "[skill]\ntimeout_secs = 60\n",
        );

        let base = std::fs::read_to_string(skill_dir.join("system_prompt.md")).unwrap();
        let tools = HashSet::new();

        let result = promote_variant(
            &skill_dir,
            &manifest,
            "anthropic",
            "claude-sonnet-4-6",
            Some(&base),
            &tools,
        )
        .unwrap();

        // Both files should be in generated
        assert!(result.target_path.exists());
        assert!(
            skill_dir
                .join("generated/anthropic/claude-sonnet-4-6/skill.toml")
                .exists()
        );
    }

    #[test]
    fn test_promote_missing_source() {
        let tmp = TempDir::new().unwrap();
        let skill_dir = create_skill_dir(&tmp);
        let manifest = make_manifest("test-skill");
        let tools = HashSet::new();

        let result = promote_variant(
            &skill_dir,
            &manifest,
            "anthropic",
            "claude-sonnet-4-6",
            None,
            &tools,
        );
        assert!(result.is_err(), "missing source should error");
        assert!(
            result.unwrap_err().to_string().contains("not found"),
            "error should mention not found"
        );
    }

    #[test]
    fn test_promote_blocked_by_hand_authored() {
        let tmp = TempDir::new().unwrap();
        let skill_dir = create_skill_dir(&tmp);
        let manifest = make_manifest("test-skill");

        // Create both experimental and hand-authored
        write_file(
            &skill_dir
                .join("experimental")
                .join("anthropic")
                .join("claude-sonnet-4-6")
                .join("system_prompt.md"),
            "# Experimental Variant\n\n## Tools\n\nUse search tools.\n\n## Constraints\n\nBe precise in responses.\n",
        );
        write_file(
            &skill_dir
                .join("anthropic")
                .join("claude-sonnet-4-6")
                .join("system_prompt.md"),
            "# Hand Authored Variant\n\n## Tools\n\nHand authored tools.\n\n## Constraints\n\nHand authored constraints.\n",
        );

        let tools = HashSet::new();
        let result = promote_variant(
            &skill_dir,
            &manifest,
            "anthropic",
            "claude-sonnet-4-6",
            None,
            &tools,
        );
        assert!(result.is_err(), "should be blocked by hand-authored");
        assert!(
            result.unwrap_err().to_string().contains("hand-authored"),
            "error should mention hand-authored"
        );

        // Verify no filesystem changes occurred
        assert!(
            skill_dir
                .join("experimental/anthropic/claude-sonnet-4-6/system_prompt.md")
                .exists()
        );
    }

    #[test]
    fn test_promote_blocked_by_validation() {
        let tmp = TempDir::new().unwrap();
        let skill_dir = create_skill_dir(&tmp);
        let mut manifest = make_manifest("test-skill");
        manifest.skill.max_prompt_size = Some(10); // Very small limit

        write_file(
            &skill_dir
                .join("experimental")
                .join("anthropic")
                .join("claude-sonnet-4-6")
                .join("system_prompt.md"),
            "# This variant is definitely too large for the 10 byte limit.\n",
        );

        let tools = HashSet::new();
        let result = promote_variant(
            &skill_dir,
            &manifest,
            "anthropic",
            "claude-sonnet-4-6",
            None,
            &tools,
        );
        assert!(result.is_err(), "should be blocked by validation");
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("failed validation")
        );

        // Source should still exist
        assert!(
            skill_dir
                .join("experimental/anthropic/claude-sonnet-4-6/system_prompt.md")
                .exists()
        );
    }

    #[test]
    fn test_promote_overwrites_existing_generated() {
        let tmp = TempDir::new().unwrap();
        let skill_dir = create_skill_dir(&tmp);
        let manifest = make_manifest("test-skill");

        // Existing generated variant
        write_file(
            &skill_dir
                .join("generated")
                .join("anthropic")
                .join("claude-sonnet-4-6")
                .join("system_prompt.md"),
            "# Old generated\n",
        );

        // New experimental
        let new_content = "# New Promoted Variant\n\n## Tools\n\nUse tools wisely.\n\n## Constraints\n\nBe concise and precise.\n";
        write_file(
            &skill_dir
                .join("experimental")
                .join("anthropic")
                .join("claude-sonnet-4-6")
                .join("system_prompt.md"),
            new_content,
        );

        let base = std::fs::read_to_string(skill_dir.join("system_prompt.md")).unwrap();
        let tools = HashSet::new();

        let result = promote_variant(
            &skill_dir,
            &manifest,
            "anthropic",
            "claude-sonnet-4-6",
            Some(&base),
            &tools,
        )
        .unwrap();
        let target_content = std::fs::read_to_string(&result.target_path).unwrap();
        assert_eq!(target_content, new_content, "should overwrite");
    }

    // -- has_failures tests --

    #[test]
    fn test_has_failures_with_no_failures() {
        let results = vec![
            ValidationResult::ok(ValidationRule::SizeLimit, "ok"),
            ValidationResult::warn(ValidationRule::ToolReferenceLint, "warn"),
        ];
        assert!(!has_failures(&results), "warnings are not failures");
    }

    #[test]
    fn test_has_failures_with_failure() {
        let results = vec![
            ValidationResult::ok(ValidationRule::SizeLimit, "ok"),
            ValidationResult::fail(ValidationRule::MarkdownWellFormedness, "bad"),
        ];
        assert!(has_failures(&results), "should detect failure");
    }
}
