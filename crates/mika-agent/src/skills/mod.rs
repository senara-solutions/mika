pub mod builtin_handlers;
pub mod context;
pub mod curator;
pub mod executor;
pub mod git;
pub mod index;
pub mod install;
pub mod manifest;
pub mod marketplace;
pub mod matcher;
pub mod quoted_resources;
pub mod review_filter;
pub mod variants;

use std::path::Path;

use self::index::{DisabledSkill, SkillEntry, SkillValidationWarning, SkippedSkill};
use crate::async_db::AsyncDatabase;
use crate::db::{Database, SkillOverride};

/// The effective tool surface for `[constraints] required_tools` coherence checking
/// (mika#1576): engine builtins (`tools::BUILTIN_TOOL_NAMES`, which subsumes
/// `builtin_handlers::KNOWN_BUILTINS` via the mika#1217 parity test — the same builtin
/// set mika#1575's build-time check uses) ∪ the tool names declared by `skills`'
/// `tools.json`. The **full** `BUILTIN_TOOL_NAMES` (including conditionally-injected
/// management tools) is used, mirroring mika#1575 and avoiding false-positive fires.
///
/// Shared by the runtime check (`SkillRegistry::apply_required_tools_coherence_check`,
/// allowlist-aware — pass the loaded skill set) and the `mika skills validate` CLI
/// diagnostic (allowlist-unaware — pass all installed skills). The allowlist-aware vs
/// -unaware difference lives at the call site (*which* skills are passed); centralizing
/// the surface primitive keeps the two from drifting.
pub fn effective_tool_surface(skills: &[SkillEntry]) -> std::collections::HashSet<String> {
    let mut surface: std::collections::HashSet<String> = crate::tools::BUILTIN_TOOL_NAMES
        .iter()
        .map(|s| s.to_string())
        .collect();
    for entry in skills {
        for tool in &entry.skill_tools {
            surface.insert(tool.definition.name.clone());
        }
    }
    surface
}

/// Whether a `[constraints] required_tools` token resolves against `surface`.
///
/// `mcp__*` tokens always resolve — they are provided by the MCP client at startup,
/// not the builtin/skill surface, so firing on them would wrongly skip a skill that
/// can in fact call the tool (mirrors `validate_skill` step 5b's leniency). Shared by
/// the runtime coherence check and the CLI diagnostic so the exemption rule cannot
/// drift between them (mika#1576).
pub fn required_tool_resolves(token: &str, surface: &std::collections::HashSet<String>) -> bool {
    token.starts_with("mcp__") || surface.contains(token)
}

/// Migrate `.disabled` marker files to DB `skill_overrides` rows.
///
/// Scans all skill directories under `skills_dir`. For each skill that has a
/// `.disabled` marker file, writes `enabled = false` to the DB and attempts to
/// remove the marker. If marker removal fails (e.g., read-only filesystem),
/// logs a warning and continues (fail-open).
///
/// Idempotent: after the first pass, no markers remain (or they're on a
/// read-only FS and the DB already has the override).
pub fn migrate_disabled_markers(
    skills_dir: &Path,
    db: &mut Database,
    agent_id: &str,
) -> anyhow::Result<()> {
    let entries = match std::fs::read_dir(skills_dir) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e.into()),
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let marker = path.join(".disabled");
        if !marker.exists() {
            continue;
        }

        let name = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) => n.to_string(),
            None => continue,
        };

        // Write DB override.
        if let Err(e) = db.set_skill_enabled(agent_id, &name, false) {
            tracing::warn!(
                skill = %name,
                error = %e,
                "failed to migrate .disabled marker to DB"
            );
            continue;
        }

        // Remove the marker file (fail-open).
        if let Err(e) = std::fs::remove_file(&marker) {
            tracing::warn!(
                skill = %name,
                error = %e,
                "migrated .disabled to DB but failed to remove marker file"
            );
        } else {
            tracing::info!(skill = %name, "migrated .disabled marker to DB");
        }
    }
    Ok(())
}

/// Async wrapper for `migrate_disabled_markers`. Collects skill names with
/// `.disabled` markers from the filesystem, writes DB overrides, and removes
/// markers. Uses `AsyncDatabase::with_db` for thread-safe DB access.
pub async fn migrate_disabled_markers_async(
    skills_dir: &Path,
    db: &AsyncDatabase,
    agent_id: &str,
) -> anyhow::Result<()> {
    // Phase 1: Collect marker file paths (filesystem work, no DB).
    let entries = match std::fs::read_dir(skills_dir) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e.into()),
    };

    let mut to_migrate: Vec<(String, std::path::PathBuf)> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let marker = path.join(".disabled");
        if !marker.exists() {
            continue;
        }
        if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
            to_migrate.push((name.to_string(), marker));
        }
    }

    // Phase 2: Write DB overrides and remove markers.
    for (name, marker) in to_migrate {
        let n = name.clone();
        let a = agent_id.to_owned();
        if let Err(e) = db.set_skill_enabled(&a, &n, false).await {
            tracing::warn!(skill = %name, error = %e, "failed to migrate .disabled marker to DB");
            continue;
        }
        if let Err(e) = std::fs::remove_file(&marker) {
            tracing::warn!(
                skill = %name,
                error = %e,
                "migrated .disabled to DB but failed to remove marker file"
            );
        } else {
            tracing::info!(skill = %name, "migrated .disabled marker to DB");
        }
    }
    Ok(())
}

/// Registry of discovered skills, built once at startup.
/// Result of [`SkillRegistry::apply_transient_always_on`].
///
/// Separates skills that are installed-but-disabled from skills that are
/// not found at all, so callers can emit accurate user-facing warnings.
#[derive(Debug, Default)]
pub struct TransientOverrideResult {
    /// Skill names that matched a disabled (evicted) entry.
    pub disabled: Vec<String>,
    /// Skill names that were not found in loaded or disabled lists.
    pub not_found: Vec<String>,
}

impl TransientOverrideResult {
    /// Returns true if all requested skills were resolved successfully.
    pub fn is_empty(&self) -> bool {
        self.disabled.is_empty() && self.not_found.is_empty()
    }
}

/// Result of [`SkillRegistry::apply_transient_disable`].
///
/// Reports skill names that were not found in either the loaded or disabled lists.
#[derive(Debug, Default)]
pub struct TransientDisableResult {
    /// Skill names that were not found in loaded or disabled lists.
    pub not_found: Vec<String>,
}

/// Skill origin for filtering (mika#606).
///
/// Groups the four-tier origin display (`[built-in]`, `[marketplace]`,
/// `[marketplace/linked]`, `[custom]`) into two filter buckets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkillSource {
    /// Engine-bundled skills (`[built-in]` display origin).
    Bundle,
    /// All non-bundled skills (`[marketplace]`, `[marketplace/linked]`, `[custom]`).
    Marketplace,
}

impl SkillSource {
    /// Parse from a CLI / HTTP string value. Case-insensitive.
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "bundle" | "bundled" => Some(Self::Bundle),
            "marketplace" => Some(Self::Marketplace),
            _ => None,
        }
    }
}

/// Property filter for skill listing (mika#606). Plain data, no behavior.
///
/// All fields use AND semantics — a skill must match every non-`None` predicate.
#[derive(Debug, Clone, Default)]
pub struct SkillListFilter {
    pub source: Option<SkillSource>,
    pub always_on: Option<bool>,
}

/// Apply property filters to a skill iterator (mika#606).
///
/// Each predicate is independent with AND semantics — a skill must match all
/// non-`None` fields. `SkillListFilter::default()` passes everything through.
pub fn apply_filter<'a>(
    skills: impl Iterator<Item = &'a SkillEntry> + 'a,
    filter: &'a SkillListFilter,
) -> impl Iterator<Item = &'a SkillEntry> + 'a {
    skills.filter(move |s| {
        if let Some(want) = filter.source {
            let is_bundled = crate::bundled_skills::is_bundled_skill(&s.manifest.skill.name);
            let actual = if is_bundled {
                SkillSource::Bundle
            } else {
                SkillSource::Marketplace
            };
            if actual != want {
                return false;
            }
        }
        if let Some(want) = filter.always_on
            && s.manifest.skill.always_on != want
        {
            return false;
        }
        true
    })
}

#[derive(Debug)]
pub struct SkillRegistry {
    skills: Vec<SkillEntry>,
    skipped: Vec<SkippedSkill>,
    /// Skills evicted by DB `enabled = false` override.
    disabled: Vec<DisabledSkill>,
    /// Warnings from load-time crash-protection (`apply_load_safety_check()`).
    /// These skills are still loaded and functional but have non-fatal issues.
    validated_warnings: Vec<SkillValidationWarning>,
}

impl SkillRegistry {
    /// Scan a skills directory and build the registry.
    ///
    /// Does not log — call [`log_summary()`](Self::log_summary) after
    /// [`apply_overrides()`](Self::apply_overrides) to emit accurate three-state counts.
    pub fn from_dir(skills_dir: &Path) -> Self {
        let result = index::scan_skills_dir(skills_dir);
        Self {
            skills: result.entries,
            skipped: result.skipped,
            disabled: Vec::new(),
            validated_warnings: Vec::new(),
        }
    }

    /// Create an empty registry (no skills directory).
    pub fn empty() -> Self {
        Self {
            skills: Vec::new(),
            skipped: Vec::new(),
            disabled: Vec::new(),
            validated_warnings: Vec::new(),
        }
    }

    /// Create a registry from pre-built entries (for integration tests).
    pub fn from_test_entries(entries: Vec<SkillEntry>) -> Self {
        Self {
            skills: entries,
            skipped: Vec::new(),
            disabled: Vec::new(),
            validated_warnings: Vec::new(),
        }
    }

    /// Create a registry with pre-populated skipped skills (for testing/display).
    pub fn with_skipped(skipped: Vec<SkippedSkill>) -> Self {
        Self {
            skills: Vec::new(),
            skipped,
            disabled: Vec::new(),
            validated_warnings: Vec::new(),
        }
    }

    /// Log a three-state summary of loaded, disabled, and skipped skills.
    ///
    /// Call **after** both [`apply_overrides()`](Self::apply_overrides) and
    /// [`apply_load_safety_check()`](Self::apply_load_safety_check) so the counts reflect the
    /// final registry state (validation may promote broken skills to skipped).
    /// Emits one `DEBUG` summary line and a per-skip `WARN` line for each
    /// skipped skill.
    pub fn log_summary(&self) {
        tracing::debug!(
            loaded = self.skills.len(),
            disabled = self.disabled.len(),
            skipped = self.skipped.len(),
            "skills loaded"
        );
        for s in &self.skipped {
            tracing::warn!(
                name = %s.name,
                reason = %s.reason,
                "skill skipped"
            );
        }
    }

    /// Number of skill directories that were skipped during scan (invalid, legacy, etc.).
    pub fn skipped_count(&self) -> usize {
        self.skipped.len()
    }

    /// Details of skills that were skipped during scan (name + reason).
    pub fn skipped(&self) -> &[SkippedSkill] {
        &self.skipped
    }

    /// Skills evicted from the registry by DB `enabled = false` override.
    pub fn disabled(&self) -> &[DisabledSkill] {
        &self.disabled
    }

    /// Number of skills disabled via DB override.
    pub fn disabled_count(&self) -> usize {
        self.disabled.len()
    }

    /// Validation warnings from `apply_load_safety_check()` — skills that loaded
    /// but have non-fatal semantic issues.
    pub fn validated_warnings(&self) -> &[SkillValidationWarning] {
        &self.validated_warnings
    }

    /// Run load-time crash-protection on all loaded skills.
    ///
    /// This is NOT the validation gate — CI and `mika skills validate` own change-time
    /// validation. This method is a runtime safety net that prevents malformed manifests
    /// from crashing or poisoning the running agent.
    ///
    /// Skills with skip-worthy structural failures (missing handler, broken tools.json,
    /// unreadable manifest, oversized always_on prompt) are removed from `self.skills`
    /// and added to `self.skipped`. Skills with non-fatal warnings are kept loaded and
    /// recorded in `self.validated_warnings`.
    ///
    /// Must be called **after** `apply_overrides()` since DB overrides can change
    /// `always_on` state and LLM configuration, affecting validation context.
    pub fn apply_load_safety_check(&mut self) {
        use index::{DiagnosticLevel, is_skip_worthy_failure, validate_skill};

        // Phase 1: Collect validation results into local vectors.
        // We cannot mutate self.skipped/self.validated_warnings while iterating self.skills.
        let mut to_skip: Vec<SkippedSkill> = Vec::new();
        let mut to_warn: Vec<SkillValidationWarning> = Vec::new();
        let mut skip_names: std::collections::HashSet<String> = std::collections::HashSet::new();

        for entry in &self.skills {
            let diags = validate_skill(&entry.dir);

            // Catch-all check uses the FULL diagnostic set (before filtering).
            // If validate_skill() returned zero Ok diagnostics and at least one Fail,
            // the skill's manifest/structure is fundamentally broken (e.g., symlink
            // race where skill.toml disappeared between scan and validate).
            let has_any_ok = diags.iter().any(|d| d.level == DiagnosticLevel::Ok);
            let has_any_fail = diags.iter().any(|d| d.level == DiagnosticLevel::Fail);
            let all_fail_no_ok = has_any_fail && !has_any_ok;

            // Filter to only Warn and Fail diagnostics for processing
            let issues: Vec<_> = diags
                .into_iter()
                .filter(|d| matches!(d.level, DiagnosticLevel::Warn | DiagnosticLevel::Fail))
                .collect();

            if issues.is_empty() {
                continue;
            }

            let skill_name = entry.manifest.skill.name.clone();
            let has_skip_worthy = issues.iter().any(is_skip_worthy_failure);

            if has_skip_worthy || all_fail_no_ok {
                // Find the first skip-worthy failure for the reason, or use the first Fail
                let reason = issues
                    .iter()
                    .find(|d| is_skip_worthy_failure(d))
                    .or_else(|| issues.iter().find(|d| d.level == DiagnosticLevel::Fail))
                    .map(|d| d.message.clone())
                    .unwrap_or_else(|| "validation failed".to_string());

                tracing::warn!(
                    skill = %skill_name,
                    error_kind = "skip",
                    message = %reason,
                    "skill removed by startup validation — run `mika skills validate {}` for full diagnostics",
                    skill_name,
                );

                skip_names.insert(skill_name.clone());
                to_skip.push(SkippedSkill {
                    name: skill_name,
                    reason: format!("validation: {reason}"),
                });
            } else {
                // Non-fatal warnings — log each one
                for diag in &issues {
                    tracing::warn!(
                        skill = %skill_name,
                        error_kind = %diag.tag(),
                        message = %diag.message,
                        "skill loaded with validation warning",
                    );
                }
                to_warn.push(SkillValidationWarning {
                    skill_name,
                    diagnostics: issues,
                });
            }
        }

        // Phase 2: Apply collected results.
        if !to_skip.is_empty() {
            self.skills
                .retain(|e| !skip_names.contains(&e.manifest.skill.name));
            self.skipped.extend(to_skip);
        }
        self.validated_warnings = to_warn;
    }

    /// Runtime coherence check: every loaded skill's `[constraints] required_tools`
    /// token must resolve to a tool in the agent's effective tool surface (mika#1576).
    ///
    /// Closes the silent vacuous-pass risk: the per-turn required_tools gate (#516)
    /// only fires for keyword-matched skills when the LLM is actually asked to call
    /// the tool, so a structurally-broken allowlist↔required_tools pairing surfaces
    /// only mid-work — when the agent reaches for a tool that isn't there. This
    /// load-time check runs after `apply_identity_allowlist` + `apply_overrides` +
    /// transient overrides, the one point where both the allowlist and each skill's
    /// `required_tools` are simultaneously visible, and catches the incoherence at
    /// startup instead.
    ///
    /// **Scope is deliberately broader than the #516/#463 runtime *enforcement*
    /// scope.** #463 declines to *enforce* `required_tools` on always-on-only
    /// (non-keyword-matched) skills — it won't force a tool call. This guard is about
    /// a different invariant: whether a *held* skill is internally coherent with the
    /// agent's tool surface. An always-on skill whose prompt presumes a tool the
    /// agent can't call is incoherent regardless of whether #516 would force the
    /// call — and the motivating mika#1406 case (Prime's always-on bearing skill) is
    /// exactly that shape. So every loaded skill is checked, keyword-matched or not.
    ///
    /// Effective surface = engine builtins ∪ tools declared by *loaded* skills'
    /// tools.json (see [`effective_tool_surface`]). Unlike mika#1575's allowlist-
    /// unaware build-time check 4, this surface is allowlist-aware (only loaded skills
    /// contribute), making it the runtime sibling of mika#1575's check 5.
    ///
    /// **Fixpoint.** Evicting a skill removes its declared tools from the surface,
    /// which can make a *consumer* skill's cross-skill `required_tool` newly
    /// unresolvable. The scan therefore repeats until a round evicts nothing —
    /// `self.skills` strictly shrinks each round, so it terminates. (A single pass
    /// would leave a consumer loaded while its provider was coherence-evicted in the
    /// same pass — exactly the silent hold-a-tool-you-can't-call state this guards.)
    ///
    /// On fire: the offending skill is skipped (evicted into `self.skipped`) and an
    /// error-level `required_tool_unresolvable` event is emitted. The agent starts
    /// degraded — the broken skill stays unavailable until the operator fixes the
    /// coherence violation (adds the providing skill to the allowlist, or removes
    /// the dangling token) and restarts. This is the established
    /// `apply_load_safety_check` "load with warning + skip the broken skill"
    /// pattern, not refuse-to-start.
    ///
    /// Must run **after** `apply_load_safety_check()` so the effective surface
    /// reflects only skills that survived structural validation.
    pub fn apply_required_tools_coherence_check(&mut self, agent_id: &str) {
        loop {
            let effective = effective_tool_surface(&self.skills);

            // Collect coherence violations without mutating self.skills.
            let mut to_skip: Vec<SkippedSkill> = Vec::new();
            let mut skip_names: std::collections::HashSet<String> =
                std::collections::HashSet::new();

            for entry in &self.skills {
                let skill_name = &entry.manifest.skill.name;
                // First unresolvable token is enough to skip the skill.
                if let Some(token) = entry
                    .manifest
                    .constraints
                    .required_tools
                    .iter()
                    .find(|t| !required_tool_resolves(t.as_str(), &effective))
                {
                    tracing::error!(
                        agent_id = %agent_id,
                        skill = %skill_name,
                        unresolvable_token = %token,
                        available_tool_count = effective.len(),
                        event = "required_tool_unresolvable",
                        "skill required_tools references a tool absent from the agent's effective \
                         surface — skipping skill; run `mika skills validate` for diagnostics",
                    );
                    skip_names.insert(skill_name.clone());
                    to_skip.push(SkippedSkill {
                        name: skill_name.clone(),
                        reason: format!(
                            "coherence: required_tool '{token}' unresolvable in effective surface"
                        ),
                    });
                }
            }

            if to_skip.is_empty() {
                break;
            }
            self.skills
                .retain(|e| !skip_names.contains(&e.manifest.skill.name));
            self.skipped.extend(to_skip);
        }
    }

    /// Match skills against a user message.
    /// Only returns enabled skills, annotated with match reason.
    pub fn match_message(&self, user_message: &str) -> Vec<matcher::MatchedSkill<'_>> {
        matcher::match_skills(&self.skills, user_message)
    }

    /// Whether any skills are loaded.
    pub fn has_skills(&self) -> bool {
        !self.skills.is_empty()
    }

    /// Access the underlying skill entries.
    pub fn skills(&self) -> &[SkillEntry] {
        &self.skills
    }

    /// Resolve a skill tool by its Claude-facing tool name (e.g.
    /// `"run_claude_pilot"`), returning the first match across all loaded
    /// skills' `skill_tools`.
    ///
    /// Used by the engine-side ready-label dispatch path (mika#1572) to obtain
    /// the `ResolvedSkillTool` for direct `spawn_long_running_exec` invocation,
    /// the same surface the agent loop would have resolved for the LLM's tool
    /// call. Scoped to this dispatch use — not a general-purpose tool registry.
    pub fn resolve_tool_by_name(&self, name: &str) -> Option<self::index::ResolvedSkillTool> {
        self.skills
            .iter()
            .flat_map(|e| &e.skill_tools)
            .find(|t| t.definition.name == name)
            .cloned()
    }

    /// Return all always-on skills (no keyword matching needed).
    /// Only returns enabled skills.
    ///
    /// Used by silent-mode heartbeats where there's no real user message
    /// to match against.
    pub fn always_on_skills(&self) -> Vec<&SkillEntry> {
        self.skills
            .iter()
            .filter(|e| e.enabled && e.manifest.skill.always_on)
            .collect()
    }

    /// Apply identity-driven skill allowlist (Phase -1 of the override chain).
    ///
    /// When an agent's `identity.toml` declares a `[skills].allowlist`, only
    /// the listed skills remain active. All other skills are evicted from the
    /// registry. This runs **before** `apply_overrides()` so DB-backed overrides
    /// (always_on, LLM provider/model) still apply to surviving skills.
    ///
    /// Call with a non-empty allowlist for well-known agents that own their skill
    /// set via identity.toml. For user-defined agents (no allowlist), skip this call.
    pub fn apply_identity_allowlist(&mut self, allowlist: &[String]) {
        if allowlist.is_empty() {
            return;
        }

        let mut evicted = Vec::new();
        self.skills.retain(|entry| {
            let is_allowed = allowlist
                .iter()
                .any(|a| entry.manifest.skill.name.eq_ignore_ascii_case(a));
            if !is_allowed {
                tracing::info!(
                    skill = %entry.manifest.skill.name,
                    "skill not in identity allowlist — evicted from registry"
                );
                evicted.push(DisabledSkill {
                    name: entry.manifest.skill.name.clone(),
                });
            }
            is_allowed
        });
        self.disabled.extend(evicted);

        // Warn about allowlisted skills that don't exist in the registry
        for name in allowlist {
            if !self
                .skills
                .iter()
                .any(|e| e.manifest.skill.name.eq_ignore_ascii_case(name))
            {
                tracing::warn!(
                    skill = %name,
                    "identity allowlist references unknown skill — not in bundled manifests"
                );
            }
        }
    }

    /// Apply database-backed overrides to skill entries and validate dependencies.
    ///
    /// For each override, finds the matching skill by name (case-insensitive)
    /// and applies the `always_on` value, marking the entry as overridden.
    ///
    /// After applying overrides, logs warnings for any declared dependency that
    /// doesn't match an installed skill name. Does not fail — only emits
    /// `tracing::warn` for each unresolvable dependency.
    pub fn apply_overrides(&mut self, overrides: &[SkillOverride]) {
        // Phase 0: Evict disabled skills.
        // Collect disabled skill names first, then evict using retain() + staging vec.
        // Can't borrow `self.disabled` while `self.skills` is mutably borrowed by retain().
        let disabled_names: Vec<String> = overrides
            .iter()
            .filter(|ov| ov.enabled == Some(false))
            .map(|ov| ov.skill_name.clone())
            .collect();

        if !disabled_names.is_empty() {
            let mut evicted = Vec::new();
            self.skills.retain(|entry| {
                let is_disabled = disabled_names
                    .iter()
                    .any(|n| entry.manifest.skill.name.eq_ignore_ascii_case(n));
                if is_disabled {
                    tracing::info!(
                        skill = %entry.manifest.skill.name,
                        "skill disabled via DB override — evicted from registry"
                    );
                    evicted.push(DisabledSkill {
                        name: entry.manifest.skill.name.clone(),
                    });
                }
                !is_disabled
            });
            self.disabled.extend(evicted);
        }

        // Phase 1: Apply remaining overrides (always_on, LLM).
        for ov in overrides {
            let Some(entry) = self
                .skills
                .iter_mut()
                .find(|e| e.manifest.skill.name.eq_ignore_ascii_case(&ov.skill_name))
            else {
                continue;
            };

            if let Some(always_on) = ov.always_on {
                entry.manifest.skill.always_on = always_on;
                entry.has_override = true;
            }

            if ov.llm_provider.is_some() || ov.llm_model.is_some() {
                if let Some(p) = &ov.llm_provider {
                    entry.manifest.llm.provider = Some(p.clone());
                }
                if let Some(m) = &ov.llm_model {
                    entry.manifest.llm.model = Some(m.clone());
                }
                // Mark as DB-sourced so resolve_skill_llm_override() can
                // distinguish operator intent from developer-time [llm] sections.
                // AlwaysOn skills with this flag qualify for override resolution
                // on autonomous-loop turns (mika#1011).
                entry.manifest.llm.from_db_override = true;
                entry.has_override = true;
            }
        }

        // Validate dependencies after overrides are applied
        for entry in &self.skills {
            for dep in &entry.manifest.skill.dependencies {
                if !self
                    .skills
                    .iter()
                    .any(|e| e.manifest.skill.name.eq_ignore_ascii_case(dep))
                {
                    tracing::warn!(
                        skill = %entry.manifest.skill.name,
                        dependency = %dep,
                        "skill declares dependency on unknown skill"
                    );
                }
            }
        }
    }

    /// Apply transient `always_on` overrides from CLI flags.
    ///
    /// For each skill name, finds the matching entry (case-insensitive) and sets
    /// `always_on = true`. This is a runtime-only overlay — nothing is persisted
    /// to the database. Call this **after** `apply_overrides()` so it stacks on
    /// top of both manifest defaults and DB overrides.
    ///
    /// Skills that were disabled (evicted by `apply_overrides()`) or skipped
    /// (oversized prompt, broken handler) cannot be resurrected. Returns a
    /// structured result distinguishing not-found from disabled/skipped so the
    /// caller can emit accurate warnings.
    pub fn apply_transient_always_on(&mut self, skill_names: &[String]) -> TransientOverrideResult {
        let mut result = TransientOverrideResult::default();

        for name in skill_names {
            if let Some(entry) = self
                .skills
                .iter_mut()
                .find(|e| e.manifest.skill.name.eq_ignore_ascii_case(name))
            {
                entry.manifest.skill.always_on = true;
                entry.has_override = true;
            } else if self
                .disabled
                .iter()
                .any(|d| d.name.eq_ignore_ascii_case(name))
            {
                tracing::warn!(
                    skill = %name,
                    "cannot force always_on — skill is disabled via DB override"
                );
                result.disabled.push(name.clone());
            } else {
                result.not_found.push(name.clone());
            }
        }

        result
    }

    /// Apply transient disable overrides from CLI flags.
    ///
    /// For each skill name, finds the matching entry (case-insensitive) and evicts
    /// it from the loaded skills list into the disabled list. This is a runtime-only
    /// overlay — nothing is persisted to the database. Call this **after**
    /// `apply_overrides()` so it stacks on top of both manifest defaults and DB
    /// overrides.
    ///
    /// Skills that are already disabled (evicted by `apply_overrides()`) or not
    /// found are treated as no-ops. Returns a structured result with skill names
    /// that were not found anywhere.
    pub fn apply_transient_disable(&mut self, skill_names: &[String]) -> TransientDisableResult {
        let mut result = TransientDisableResult::default();

        // Collect names to disable (case-insensitive), then evict using retain() + staging vec.
        let disable_names: Vec<&String> = skill_names
            .iter()
            .filter(|name| {
                // Check if the skill is loaded
                let in_loaded = self
                    .skills
                    .iter()
                    .any(|e| e.manifest.skill.name.eq_ignore_ascii_case(name));
                // Check if already disabled (no-op)
                let in_disabled = self
                    .disabled
                    .iter()
                    .any(|d| d.name.eq_ignore_ascii_case(name));
                if !in_loaded && !in_disabled {
                    result.not_found.push((*name).clone());
                    false
                } else {
                    in_loaded
                }
            })
            .collect();

        if !disable_names.is_empty() {
            let mut evicted = Vec::new();
            self.skills.retain(|entry| {
                let is_disabled = disable_names
                    .iter()
                    .any(|n| entry.manifest.skill.name.eq_ignore_ascii_case(n));
                if is_disabled {
                    tracing::info!(
                        skill = %entry.manifest.skill.name,
                        "skill transiently disabled via CLI flag — evicted from registry"
                    );
                    evicted.push(DisabledSkill {
                        name: entry.manifest.skill.name.clone(),
                    });
                }
                !is_disabled
            });
            self.disabled.extend(evicted);
        }

        result
    }

    /// Return always-on skills that are safe for silent/background mode.
    ///
    /// Filters out skills whose tools use `Exec` or `Http` handlers
    /// (e.g., tmux, shell-exec) since those should not run autonomously
    /// in heartbeat or reminder contexts without user interaction.
    ///
    /// Note: This method does NOT resolve skill dependencies (unlike `match_skills()`).
    /// This is intentional — dependency resolution could pull in Exec/Http handler
    /// skills that must not run in autonomous background contexts.
    pub fn safe_always_on_skills(&self) -> Vec<&SkillEntry> {
        use crate::skills::manifest::ToolHandler;

        self.skills
            .iter()
            .filter(|e| {
                e.enabled
                    && e.manifest.skill.always_on
                    && !e.skill_tools.iter().any(|t| {
                        matches!(
                            t.handler,
                            ToolHandler::Exec { .. } | ToolHandler::Http { .. }
                        )
                    })
            })
            .collect()
    }

    /// Return always-on skills **and their transitive dependencies** for
    /// `SilentTrigger::Callback` turns — includes skills with `Exec` and
    /// `Http` handlers.
    ///
    /// A callback turn is the agent's continuation of a tool call it already
    /// made in conversation mode (e.g. a `run_claude_pilot` long-running task
    /// that completed or failed). The agent was already exposed to the full
    /// skill set when it initiated that call, so the retry/continue workflow
    /// must have access to the same tools. Stripping exec handlers here
    /// causes retries to fail with `Unknown tool` errors (#567).
    ///
    /// Unlike `safe_always_on_skills()`, this method resolves skill
    /// dependencies via BFS (same algorithm as `match_skills()` in
    /// `matcher.rs`). This ensures dependency skills like `dev-pilot`
    /// (which provides `run_claude_pilot` but has `always_on = false`) are
    /// included when an always-on skill (e.g. `self-dev`) declares them as
    /// dependencies (#578).
    ///
    /// Use `safe_always_on_skills()` for all other silent triggers
    /// (`Heartbeat`, `Reflection`, `Reminder`, `SkillRun`), which are fully
    /// autonomous and must not trigger exec handlers without the agent or
    /// user explicitly initiating the workflow.
    pub fn callback_safe_skills(&self) -> Vec<&SkillEntry> {
        use std::collections::{HashSet, VecDeque};

        // Seed: enabled + always_on skills
        let mut included: HashSet<usize> = HashSet::new();
        let mut queue: VecDeque<usize> = VecDeque::new();

        for (i, entry) in self.skills.iter().enumerate() {
            if entry.enabled && entry.manifest.skill.always_on {
                included.insert(i);
                queue.push_back(i);
            }
        }

        // BFS transitive dependency resolution (mirrors match_skills() in matcher.rs)
        while let Some(idx) = queue.pop_front() {
            for dep_name in &self.skills[idx].manifest.skill.dependencies {
                if let Some(dep_idx) = self
                    .skills
                    .iter()
                    .position(|e| e.manifest.skill.name.eq_ignore_ascii_case(dep_name))
                {
                    // Disabled dependency breaks its sub-tree
                    if self.skills[dep_idx].enabled && !included.contains(&dep_idx) {
                        included.insert(dep_idx);
                        queue.push_back(dep_idx);
                    }
                }
            }
        }

        // Collect in original order
        self.skills
            .iter()
            .enumerate()
            .filter(|(i, _)| included.contains(i))
            .map(|(_, entry)| entry)
            .collect()
    }
}

/// Lightweight markdown well-formedness check (#511).
///
/// Returns `Ok(())` for valid-looking markdown, or `Err(description)` for
/// common corruption patterns: empty/whitespace-only, binary content (null
/// bytes or control characters), and unclosed code fences.
///
/// This is intentionally lightweight — no AST parsing or heavyweight
/// dependencies. It catches the most common corruption from generated prompts.
pub(crate) fn validate_markdown_content(content: &str) -> Result<(), String> {
    // 1. Reject empty/whitespace-only
    if content.trim().is_empty() {
        return Err("content is empty or whitespace-only".to_string());
    }
    // 2. Reject binary content (null bytes)
    if content.bytes().any(|b| b == 0) {
        return Err("content contains null bytes — likely binary data".to_string());
    }
    // 3. Reject control characters (except newline, carriage return, tab)
    let control_count = content
        .bytes()
        .filter(|&b| b < 0x20 && b != b'\n' && b != b'\r' && b != b'\t')
        .count();
    if control_count > 0 {
        return Err(format!(
            "content contains {control_count} control character(s) — likely corrupted"
        ));
    }
    // 4. Check for unclosed code fences
    let fence_count = content
        .lines()
        .filter(|l| l.trim_start().starts_with("```"))
        .count();
    if fence_count % 2 != 0 {
        return Err(format!(
            "content has {fence_count} code fence(s) — odd count suggests an unclosed code block"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::skills::manifest::{SkillInfo, SkillManifest, Triggers};
    use std::path::PathBuf;

    fn make_entry(name: &str, always_on: bool, enabled: bool) -> SkillEntry {
        make_entry_with_deps(name, always_on, enabled, &[])
    }

    fn make_entry_with_deps(
        name: &str,
        always_on: bool,
        enabled: bool,
        deps: &[&str],
    ) -> SkillEntry {
        SkillEntry {
            manifest: SkillManifest {
                skill: SkillInfo {
                    name: name.to_string(),
                    description: format!("{name} skill"),
                    version: String::new(),
                    always_on,
                    timeout_secs: 30,
                    dependencies: deps.iter().map(|s| s.to_string()).collect(),
                    max_prompt_size: None,
                },
                triggers: Triggers { keywords: vec![] },
                llm: Default::default(),
                constraints: Default::default(),
                output: Default::default(),
                context: std::collections::HashMap::new(),
                variants: Default::default(),
            },
            dir: PathBuf::from(format!("/skills/{name}")),
            keywords_lower: vec![],
            prompt_snippet: String::new(),
            skill_tools: vec![],
            enabled,
            has_override: false,
            provider_overrides: std::collections::HashMap::new(),
            prompt_sources: SkillEntry::empty_prompt_sources(),
            model_overrides: std::collections::HashMap::new(),
        }
    }

    #[test]
    fn test_registry_from_empty_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let registry = SkillRegistry::from_dir(tmp.path());
        assert!(!registry.has_skills());
        let matched = registry.match_message("hello");
        assert!(matched.is_empty());
    }

    #[test]
    fn test_registry_empty() {
        let registry = SkillRegistry::empty();
        assert!(!registry.has_skills());
    }

    #[test]
    fn test_always_on_skills() {
        let registry = SkillRegistry {
            skipped: Vec::new(),
            disabled: Vec::new(),
            validated_warnings: Vec::new(),
            skills: vec![
                make_entry("memory", true, true),
                make_entry("reminders", false, true),
                make_entry("messaging", true, true),
            ],
        };
        let always_on = registry.always_on_skills();
        assert_eq!(always_on.len(), 2);
        assert_eq!(always_on[0].manifest.skill.name, "memory");
        assert_eq!(always_on[1].manifest.skill.name, "messaging");
    }

    #[test]
    fn test_always_on_skills_filters_disabled() {
        let registry = SkillRegistry {
            skipped: Vec::new(),
            disabled: Vec::new(),
            validated_warnings: Vec::new(),
            skills: vec![
                make_entry("memory", true, true),
                make_entry("disabled-skill", true, false),
            ],
        };
        let always_on = registry.always_on_skills();
        assert_eq!(always_on.len(), 1);
        assert_eq!(always_on[0].manifest.skill.name, "memory");
    }

    #[test]
    fn test_always_on_skills_empty() {
        let registry = SkillRegistry::empty();
        assert!(registry.always_on_skills().is_empty());
    }

    // ── SkillSource parse tests ──

    #[test]
    fn test_skill_source_parse_bundle() {
        assert_eq!(SkillSource::parse("bundle"), Some(SkillSource::Bundle));
        assert_eq!(SkillSource::parse("Bundle"), Some(SkillSource::Bundle));
        assert_eq!(SkillSource::parse("BUNDLED"), Some(SkillSource::Bundle));
        assert_eq!(SkillSource::parse("bundled"), Some(SkillSource::Bundle));
    }

    #[test]
    fn test_skill_source_parse_marketplace() {
        assert_eq!(
            SkillSource::parse("marketplace"),
            Some(SkillSource::Marketplace)
        );
        assert_eq!(
            SkillSource::parse("Marketplace"),
            Some(SkillSource::Marketplace)
        );
    }

    #[test]
    fn test_skill_source_parse_invalid() {
        assert_eq!(SkillSource::parse("junk"), None);
        assert_eq!(SkillSource::parse(""), None);
        assert_eq!(SkillSource::parse("built-in"), None);
    }

    // ── apply_filter tests ──

    #[test]
    fn test_filter_default_passes_all() {
        let skills = [
            make_entry("alpha", true, true),
            make_entry("beta", false, true),
        ];
        let filter = SkillListFilter::default();
        let result: Vec<_> = apply_filter(skills.iter(), &filter).collect();
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn test_filter_by_always_on() {
        let skills = [
            make_entry("alpha", true, true),
            make_entry("beta", false, true),
            make_entry("gamma", true, true),
        ];
        let filter = SkillListFilter {
            always_on: Some(true),
            ..Default::default()
        };
        let result: Vec<_> = apply_filter(skills.iter(), &filter).collect();
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].manifest.skill.name, "alpha");
        assert_eq!(result[1].manifest.skill.name, "gamma");
    }

    #[test]
    fn test_filter_by_always_on_false() {
        let skills = [
            make_entry("alpha", true, true),
            make_entry("beta", false, true),
        ];
        let filter = SkillListFilter {
            always_on: Some(false),
            ..Default::default()
        };
        let result: Vec<_> = apply_filter(skills.iter(), &filter).collect();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].manifest.skill.name, "beta");
    }

    #[test]
    fn test_filter_by_source_bundle() {
        // "tmux" is a bundled skill name that is_bundled_skill recognizes
        let skills = [
            make_entry("tmux", false, true),
            make_entry("my-custom-skill", false, true),
        ];
        let filter = SkillListFilter {
            source: Some(SkillSource::Bundle),
            ..Default::default()
        };
        let result: Vec<_> = apply_filter(skills.iter(), &filter).collect();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].manifest.skill.name, "tmux");
    }

    #[test]
    fn test_filter_by_source_marketplace() {
        let skills = [
            make_entry("tmux", false, true),
            make_entry("my-custom-skill", false, true),
        ];
        let filter = SkillListFilter {
            source: Some(SkillSource::Marketplace),
            ..Default::default()
        };
        let result: Vec<_> = apply_filter(skills.iter(), &filter).collect();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].manifest.skill.name, "my-custom-skill");
    }

    #[test]
    fn test_filter_composed_and_semantics() {
        let skills = [
            make_entry("tmux", true, true),         // bundled + always_on
            make_entry("tmux", false, true),        // bundled + not always_on (hypothetical)
            make_entry("my-skill", true, true),     // marketplace + always_on
            make_entry("other-skill", false, true), // marketplace + not always_on
        ];
        // Only bundled AND always_on
        let filter = SkillListFilter {
            source: Some(SkillSource::Bundle),
            always_on: Some(true),
        };
        let result: Vec<_> = apply_filter(skills.iter(), &filter).collect();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].manifest.skill.name, "tmux");
        assert!(result[0].manifest.skill.always_on);
    }

    #[test]
    fn test_filter_no_matches() {
        let skills = [
            make_entry("alpha", true, true),
            make_entry("beta", true, true),
        ];
        let filter = SkillListFilter {
            always_on: Some(false),
            ..Default::default()
        };
        let result: Vec<_> = apply_filter(skills.iter(), &filter).collect();
        assert!(result.is_empty());
    }

    #[test]
    fn test_safe_always_on_skills_filters_exec_and_http() {
        use crate::skills::index::ResolvedSkillTool;
        use crate::skills::manifest::ToolHandler;
        use mika_common::claude::ToolDefinition;

        let dummy_def = ToolDefinition {
            name: "dummy".to_string(),
            description: "dummy".to_string(),
            input_schema: serde_json::json!({"type": "object"}),
        };

        // A safe always-on skill with only builtin tools
        let mut safe_entry = make_entry("memory", true, true);
        safe_entry.skill_tools = vec![ResolvedSkillTool {
            definition: dummy_def.clone(),
            handler: ToolHandler::Builtin {
                function: "get_documentation".to_string(),
            },
            skill_dir: PathBuf::from("/skills/memory"),
        }];

        // An unsafe always-on skill with an exec handler
        let mut exec_entry = make_entry("tmux", true, true);
        exec_entry.skill_tools = vec![ResolvedSkillTool {
            definition: dummy_def.clone(),
            handler: ToolHandler::Exec {
                command: "./run.sh".to_string(),
                long_running: false,
                estimated_duration_secs: None,
            },
            skill_dir: PathBuf::from("/skills/tmux"),
        }];

        // An unsafe always-on skill with an http handler
        let mut http_entry = make_entry("webhook", true, true);
        http_entry.skill_tools = vec![ResolvedSkillTool {
            definition: dummy_def.clone(),
            handler: ToolHandler::Http {
                url: "https://example.com".to_string(),
                method: "POST".to_string(),
            },
            skill_dir: PathBuf::from("/skills/webhook"),
        }];

        // A safe always-on skill with no tools (prompt-only)
        let prompt_only = make_entry("guidelines", true, true);

        let registry = SkillRegistry {
            skipped: Vec::new(),
            disabled: Vec::new(),
            validated_warnings: Vec::new(),
            skills: vec![safe_entry, exec_entry, http_entry, prompt_only],
        };

        // always_on_skills returns all 4
        assert_eq!(registry.always_on_skills().len(), 4);

        // safe_always_on_skills filters out exec and http
        let safe = registry.safe_always_on_skills();
        assert_eq!(safe.len(), 2);
        assert_eq!(safe[0].manifest.skill.name, "memory");
        assert_eq!(safe[1].manifest.skill.name, "guidelines");
    }

    #[test]
    fn test_safe_always_on_skills_empty() {
        let registry = SkillRegistry::empty();
        assert!(registry.safe_always_on_skills().is_empty());
    }

    // ── apply_required_tools_coherence_check tests (mika#1576) ──

    fn entry_with_required_tools(name: &str, required: &[&str]) -> SkillEntry {
        let mut e = make_entry(name, false, true);
        e.manifest.constraints.required_tools = required.iter().map(|s| s.to_string()).collect();
        e
    }

    /// A skill that both requires `required` and provides `provides` via its own tools.json.
    fn entry_requiring_and_providing(
        name: &str,
        required: &[&str],
        provides: &[&str],
    ) -> SkillEntry {
        use crate::skills::index::ResolvedSkillTool;
        use crate::skills::manifest::ToolHandler;
        use mika_common::claude::ToolDefinition;

        let mut e = entry_with_required_tools(name, required);
        e.skill_tools = provides
            .iter()
            .map(|tool_name| ResolvedSkillTool {
                definition: ToolDefinition {
                    name: tool_name.to_string(),
                    description: "tool".to_string(),
                    input_schema: serde_json::json!({"type": "object"}),
                },
                handler: ToolHandler::Builtin {
                    function: tool_name.to_string(),
                },
                skill_dir: PathBuf::from(format!("/skills/{name}")),
            })
            .collect();
        e
    }

    fn registry_of(skills: Vec<SkillEntry>) -> SkillRegistry {
        SkillRegistry {
            skipped: Vec::new(),
            disabled: Vec::new(),
            validated_warnings: Vec::new(),
            skills,
        }
    }

    #[test]
    fn test_coherence_pass_builtin_and_cross_skill() {
        // "search_memory" is a builtin (BUILTIN_TOOL_NAMES); "tool_from_b" is
        // provided by a co-loaded skill. Both resolve → no fire.
        let mut registry = registry_of(vec![
            entry_with_required_tools("skill-a", &["search_memory", "tool_from_b"]),
            entry_requiring_and_providing("skill-b", &[], &["tool_from_b"]),
        ]);
        registry.apply_required_tools_coherence_check("test-agent");
        assert_eq!(registry.skills.len(), 2);
        assert!(registry.skipped.is_empty());
    }

    #[test]
    fn test_coherence_fail_synthetic_unresolvable_token() {
        // A token that exists nowhere → the skill is evicted.
        let mut registry = registry_of(vec![entry_with_required_tools(
            "broken",
            &["totally_unresolvable_tool"],
        )]);
        registry.apply_required_tools_coherence_check("test-agent");
        assert!(registry.skills.is_empty());
        assert_eq!(registry.skipped.len(), 1);
        assert_eq!(registry.skipped[0].name, "broken");
        assert!(
            registry.skipped[0]
                .reason
                .contains("totally_unresolvable_tool")
        );
    }

    #[test]
    fn test_coherence_fail_realistic_allowlist_exclusion() {
        // skill-a requires a tool only skill-b provides, but skill-b is NOT loaded
        // (simulating allowlist exclusion — the mika#1406 motivating scenario).
        let mut registry =
            registry_of(vec![entry_with_required_tools("skill-a", &["tool_from_b"])]);
        registry.apply_required_tools_coherence_check("test-agent");
        assert!(registry.skills.is_empty(), "skill-a should be evicted");
        assert_eq!(registry.skipped.len(), 1);
        assert_eq!(registry.skipped[0].name, "skill-a");

        // With skill-b loaded, the same required_tool resolves → no fire.
        let mut registry = registry_of(vec![
            entry_with_required_tools("skill-a", &["tool_from_b"]),
            entry_requiring_and_providing("skill-b", &[], &["tool_from_b"]),
        ]);
        registry.apply_required_tools_coherence_check("test-agent");
        assert_eq!(registry.skills.len(), 2);
        assert!(registry.skipped.is_empty());
    }

    #[test]
    fn test_coherence_edge_builtin_always_resolves() {
        // "run_gh" ∈ KNOWN_BUILTINS ⊂ BUILTIN_TOOL_NAMES, so it resolves as a
        // builtin even when no loaded skill declares it. (This is why mika#1576's
        // AC3 literal `run_gh` FAIL example was inconsistent with F1 — run_gh can
        // never fire; see plan KTD-4.)
        let mut registry = registry_of(vec![entry_with_required_tools("uses-gh", &["run_gh"])]);
        registry.apply_required_tools_coherence_check("test-agent");
        assert_eq!(registry.skills.len(), 1);
        assert!(registry.skipped.is_empty());
    }

    #[test]
    fn test_coherence_edge_empty_required_tools() {
        let mut registry = registry_of(vec![make_entry("no-constraints", false, true)]);
        registry.apply_required_tools_coherence_check("test-agent");
        assert_eq!(registry.skills.len(), 1);
        assert!(registry.skipped.is_empty());
    }

    #[test]
    fn test_coherence_skips_only_offending_skill() {
        // One broken skill is evicted; a coherent sibling survives.
        let mut registry = registry_of(vec![
            entry_with_required_tools("good", &["search_memory"]),
            entry_with_required_tools("bad", &["totally_unresolvable_tool"]),
        ]);
        registry.apply_required_tools_coherence_check("test-agent");
        assert_eq!(registry.skills.len(), 1);
        assert_eq!(registry.skills[0].manifest.skill.name, "good");
        assert_eq!(registry.skipped.len(), 1);
        assert_eq!(registry.skipped[0].name, "bad");
    }

    #[test]
    fn test_coherence_mcp_token_is_exempt() {
        // `mcp__*` tokens resolve via the MCP client at startup, not the builtin/skill
        // surface — they must NOT fire even when nothing in the surface provides them.
        let mut registry = registry_of(vec![entry_with_required_tools(
            "uses-mcp",
            &["mcp__server__tool"],
        )]);
        registry.apply_required_tools_coherence_check("test-agent");
        assert_eq!(registry.skills.len(), 1, "mcp__ token must be exempt");
        assert!(registry.skipped.is_empty());
    }

    #[test]
    fn test_coherence_fixpoint_evicts_consumer_after_provider() {
        // Cascade: consumer requires a tool only the provider declares, but the
        // provider has its OWN unresolvable token. Round 1 evicts the provider;
        // round 2 sees the consumer's tool is now gone and evicts it too. A single
        // pass would wrongly leave the consumer holding an uncallable tool.
        let mut registry = registry_of(vec![
            entry_with_required_tools("consumer", &["tool_from_provider"]),
            entry_requiring_and_providing(
                "provider",
                &["totally_unresolvable_tool"],
                &["tool_from_provider"],
            ),
        ]);
        registry.apply_required_tools_coherence_check("test-agent");
        assert!(
            registry.skills.is_empty(),
            "both provider (own bad token) and consumer (cascade) should be evicted"
        );
        let skipped: Vec<&str> = registry.skipped.iter().map(|s| s.name.as_str()).collect();
        assert!(skipped.contains(&"provider"));
        assert!(skipped.contains(&"consumer"));
    }

    #[test]
    fn test_required_tool_resolves_helper() {
        let surface: std::collections::HashSet<String> =
            ["search_memory".to_string(), "tool_x".to_string()]
                .into_iter()
                .collect();
        assert!(required_tool_resolves("search_memory", &surface)); // builtin/in-surface
        assert!(required_tool_resolves("tool_x", &surface)); // skill-provided
        assert!(required_tool_resolves("mcp__srv__t", &surface)); // mcp exempt
        assert!(!required_tool_resolves("nope", &surface)); // genuinely unresolvable
    }

    #[test]
    fn test_effective_tool_surface_includes_builtins_and_skill_tools() {
        let surface =
            effective_tool_surface(&[entry_requiring_and_providing("p", &[], &["custom_tool"])]);
        assert!(
            surface.contains("custom_tool"),
            "skill-declared tool present"
        );
        assert!(surface.contains("search_memory"), "engine builtin present");
        assert!(surface.contains("run_gh"), "KNOWN_BUILTINS member present");
    }

    #[test]
    fn test_callback_safe_skills_includes_exec_and_http() {
        use crate::skills::index::ResolvedSkillTool;
        use crate::skills::manifest::ToolHandler;
        use mika_common::claude::ToolDefinition;

        let dummy_def = ToolDefinition {
            name: "dummy".to_string(),
            description: "dummy".to_string(),
            input_schema: serde_json::json!({"type": "object"}),
        };

        // Safe builtin-only always_on skill
        let mut safe_entry = make_entry("memory", true, true);
        safe_entry.skill_tools = vec![ResolvedSkillTool {
            definition: dummy_def.clone(),
            handler: ToolHandler::Builtin {
                function: "get_documentation".to_string(),
            },
            skill_dir: PathBuf::from("/skills/memory"),
        }];

        // Exec-handler always_on skill (e.g. dev-pilot)
        let mut exec_entry = make_entry("dev-pilot", true, true);
        exec_entry.skill_tools = vec![ResolvedSkillTool {
            definition: dummy_def.clone(),
            handler: ToolHandler::Exec {
                command: "./run.sh".to_string(),
                long_running: true,
                estimated_duration_secs: Some(600),
            },
            skill_dir: PathBuf::from("/skills/dev-pilot"),
        }];

        // Http-handler always_on skill
        let mut http_entry = make_entry("webhook", true, true);
        http_entry.skill_tools = vec![ResolvedSkillTool {
            definition: dummy_def.clone(),
            handler: ToolHandler::Http {
                url: "https://example.com".to_string(),
                method: "POST".to_string(),
            },
            skill_dir: PathBuf::from("/skills/webhook"),
        }];

        // Prompt-only always_on skill
        let prompt_only = make_entry("guidelines", true, true);

        let registry = SkillRegistry {
            skipped: Vec::new(),
            disabled: Vec::new(),
            validated_warnings: Vec::new(),
            skills: vec![safe_entry, exec_entry, http_entry, prompt_only],
        };

        // callback_safe_skills includes all four (exec/http not filtered)
        let callback = registry.callback_safe_skills();
        assert_eq!(callback.len(), 4);
        let names: Vec<&str> = callback
            .iter()
            .map(|e| e.manifest.skill.name.as_str())
            .collect();
        assert!(names.contains(&"memory"));
        assert!(names.contains(&"dev-pilot"));
        assert!(names.contains(&"webhook"));
        assert!(names.contains(&"guidelines"));

        // safe_always_on_skills still strips exec and http (regression check)
        let safe = registry.safe_always_on_skills();
        assert_eq!(safe.len(), 2);
    }

    #[test]
    fn test_callback_safe_skills_respects_enabled_and_always_on() {
        use crate::skills::index::ResolvedSkillTool;
        use crate::skills::manifest::ToolHandler;
        use mika_common::claude::ToolDefinition;

        let dummy_def = ToolDefinition {
            name: "dummy".to_string(),
            description: "dummy".to_string(),
            input_schema: serde_json::json!({"type": "object"}),
        };

        let exec_tool = || ResolvedSkillTool {
            definition: dummy_def.clone(),
            handler: ToolHandler::Exec {
                command: "./run.sh".to_string(),
                long_running: false,
                estimated_duration_secs: None,
            },
            skill_dir: PathBuf::from("/skills/x"),
        };

        // Enabled + always_on + exec — included
        let mut included = make_entry("included", true, true);
        included.skill_tools = vec![exec_tool()];

        // Disabled + always_on + exec — excluded
        let mut disabled = make_entry("disabled", true, false);
        disabled.skill_tools = vec![exec_tool()];

        // Enabled + NOT always_on + exec — excluded
        let mut not_always_on = make_entry("not-always-on", false, true);
        not_always_on.skill_tools = vec![exec_tool()];

        let registry = SkillRegistry {
            skipped: Vec::new(),
            disabled: Vec::new(),
            validated_warnings: Vec::new(),
            skills: vec![included, disabled, not_always_on],
        };

        let callback = registry.callback_safe_skills();
        assert_eq!(callback.len(), 1);
        assert_eq!(callback[0].manifest.skill.name, "included");
    }

    #[test]
    fn test_callback_safe_skills_empty() {
        let registry = SkillRegistry::empty();
        assert!(registry.callback_safe_skills().is_empty());
    }

    #[test]
    fn test_callback_safe_skills_resolves_dependencies() {
        // Simulates self-dev (always_on) depending on claude-pilot (not always_on).
        // callback_safe_skills must include both (#578).
        use crate::skills::index::ResolvedSkillTool;
        use crate::skills::manifest::ToolHandler;
        use mika_common::claude::ToolDefinition;

        let dummy_def = ToolDefinition {
            name: "run_claude_pilot".to_string(),
            description: "Run claude pilot".to_string(),
            input_schema: serde_json::json!({"type": "object"}),
        };

        // always_on skill that depends on "dev-pilot"
        let self_dev = make_entry_with_deps("self-dev", true, true, &["dev-pilot"]);

        // NOT always_on, but provides the exec tool
        let mut dev_pilot = make_entry_with_deps("dev-pilot", false, true, &[]);
        dev_pilot.skill_tools = vec![ResolvedSkillTool {
            definition: dummy_def.clone(),
            handler: ToolHandler::Exec {
                command: "./run.sh".to_string(),
                long_running: true,
                estimated_duration_secs: Some(600),
            },
            skill_dir: PathBuf::from("/skills/dev-pilot"),
        }];

        let registry = SkillRegistry {
            skipped: Vec::new(),
            disabled: Vec::new(),
            validated_warnings: Vec::new(),
            skills: vec![self_dev, dev_pilot],
        };

        let callback = registry.callback_safe_skills();
        let names: Vec<&str> = callback
            .iter()
            .map(|e| e.manifest.skill.name.as_str())
            .collect();
        assert_eq!(names.len(), 2);
        assert!(names.contains(&"self-dev"));
        assert!(names.contains(&"dev-pilot"));
    }

    #[test]
    fn test_callback_safe_skills_resolves_transitive_dependencies() {
        // A -> B -> C (transitive chain)
        let a = make_entry_with_deps("a", true, true, &["b"]);
        let b = make_entry_with_deps("b", false, true, &["c"]);
        let c = make_entry_with_deps("c", false, true, &[]);

        let registry = SkillRegistry {
            skipped: Vec::new(),
            disabled: Vec::new(),
            validated_warnings: Vec::new(),
            skills: vec![a, b, c],
        };

        let callback = registry.callback_safe_skills();
        assert_eq!(callback.len(), 3);
        let names: Vec<&str> = callback
            .iter()
            .map(|e| e.manifest.skill.name.as_str())
            .collect();
        assert!(names.contains(&"a"));
        assert!(names.contains(&"b"));
        assert!(names.contains(&"c"));
    }

    #[test]
    fn test_callback_safe_skills_disabled_dep_breaks_subtree() {
        // A (always_on) -> B (disabled) -> C
        // B is disabled, so neither B nor C should appear
        let a = make_entry_with_deps("a", true, true, &["b"]);
        let b = make_entry_with_deps("b", false, false, &["c"]); // disabled
        let c = make_entry_with_deps("c", false, true, &[]);

        let registry = SkillRegistry {
            skipped: Vec::new(),
            disabled: Vec::new(),
            validated_warnings: Vec::new(),
            skills: vec![a, b, c],
        };

        let callback = registry.callback_safe_skills();
        assert_eq!(callback.len(), 1);
        assert_eq!(callback[0].manifest.skill.name, "a");
    }

    #[test]
    fn test_callback_safe_skills_no_duplicate_for_always_on_dep() {
        // A (always_on) -> B (also always_on) — B should appear once
        let a = make_entry_with_deps("a", true, true, &["b"]);
        let b = make_entry_with_deps("b", true, true, &[]);

        let registry = SkillRegistry {
            skipped: Vec::new(),
            disabled: Vec::new(),
            validated_warnings: Vec::new(),
            skills: vec![a, b],
        };

        let callback = registry.callback_safe_skills();
        assert_eq!(callback.len(), 2);
    }

    #[test]
    fn test_callback_safe_skills_circular_deps_no_infinite_loop() {
        // A -> B -> A (circular)
        let a = make_entry_with_deps("a", true, true, &["b"]);
        let b = make_entry_with_deps("b", false, true, &["a"]);

        let registry = SkillRegistry {
            skipped: Vec::new(),
            disabled: Vec::new(),
            validated_warnings: Vec::new(),
            skills: vec![a, b],
        };

        let callback = registry.callback_safe_skills();
        assert_eq!(callback.len(), 2);
    }

    #[test]
    fn test_callback_safe_skills_unknown_dep_silently_skipped() {
        // A depends on "nonexistent" — should not panic
        let a = make_entry_with_deps("a", true, true, &["nonexistent"]);

        let registry = SkillRegistry {
            skipped: Vec::new(),
            disabled: Vec::new(),
            validated_warnings: Vec::new(),
            skills: vec![a],
        };

        let callback = registry.callback_safe_skills();
        assert_eq!(callback.len(), 1);
        assert_eq!(callback[0].manifest.skill.name, "a");
    }

    #[test]
    fn test_safe_always_on_does_not_resolve_deps_but_callback_does() {
        // Regression: safe_always_on_skills does NOT resolve deps,
        // callback_safe_skills DOES. Same registry, different results.
        use crate::skills::index::ResolvedSkillTool;
        use crate::skills::manifest::ToolHandler;
        use mika_common::claude::ToolDefinition;

        let dummy_def = ToolDefinition {
            name: "run_claude_pilot".to_string(),
            description: "Run claude pilot".to_string(),
            input_schema: serde_json::json!({"type": "object"}),
        };

        let self_dev = make_entry_with_deps("self-dev", true, true, &["dev-pilot"]);

        let mut dev_pilot = make_entry_with_deps("dev-pilot", false, true, &[]);
        dev_pilot.skill_tools = vec![ResolvedSkillTool {
            definition: dummy_def.clone(),
            handler: ToolHandler::Exec {
                command: "./run.sh".to_string(),
                long_running: true,
                estimated_duration_secs: Some(600),
            },
            skill_dir: PathBuf::from("/skills/dev-pilot"),
        }];

        let registry = SkillRegistry {
            skipped: Vec::new(),
            disabled: Vec::new(),
            validated_warnings: Vec::new(),
            skills: vec![self_dev, dev_pilot],
        };

        // safe_always_on_skills: only self-dev (no dep resolution, exec filtered anyway)
        let safe = registry.safe_always_on_skills();
        assert_eq!(safe.len(), 1);
        assert_eq!(safe[0].manifest.skill.name, "self-dev");

        // callback_safe_skills: both (dep resolution + exec preserved)
        let callback = registry.callback_safe_skills();
        assert_eq!(callback.len(), 2);
    }

    #[test]
    fn test_safe_always_on_skills_excludes_exec_dependency() {
        use crate::skills::index::ResolvedSkillTool;
        use crate::skills::manifest::ToolHandler;
        use mika_common::claude::ToolDefinition;

        let dummy_def = ToolDefinition {
            name: "dummy".to_string(),
            description: "dummy".to_string(),
            input_schema: serde_json::json!({"type": "object"}),
        };

        // A safe always-on skill with only builtin tools that depends on "tmux"
        let mut safe_with_dep = make_entry_with_deps("self-knowledge", true, true, &["tmux"]);
        safe_with_dep.skill_tools = vec![ResolvedSkillTool {
            definition: dummy_def.clone(),
            handler: ToolHandler::Builtin {
                function: "get_documentation".to_string(),
            },
            skill_dir: PathBuf::from("/skills/self-knowledge"),
        }];

        // An exec-handler skill that is a dependency (not always-on itself)
        let mut exec_dep = make_entry("tmux", false, true);
        exec_dep.skill_tools = vec![ResolvedSkillTool {
            definition: dummy_def.clone(),
            handler: ToolHandler::Exec {
                command: "./run.sh".to_string(),
                long_running: false,
                estimated_duration_secs: None,
            },
            skill_dir: PathBuf::from("/skills/tmux"),
        }];

        let registry = SkillRegistry {
            skipped: Vec::new(),
            disabled: Vec::new(),
            validated_warnings: Vec::new(),
            skills: vec![safe_with_dep, exec_dep],
        };

        // safe_always_on_skills must NOT resolve dependencies — only the safe
        // builtin-handler skill should be returned, not its exec-handler dependency.
        let safe = registry.safe_always_on_skills();
        assert_eq!(safe.len(), 1);
        assert_eq!(safe[0].manifest.skill.name, "self-knowledge");
    }

    #[test]
    fn test_apply_overrides_sets_always_on() {
        use crate::db::SkillOverride;

        let mut registry = SkillRegistry {
            skipped: Vec::new(),
            disabled: Vec::new(),
            validated_warnings: Vec::new(),
            skills: vec![
                make_entry("web-search", false, true),
                make_entry("tmux", false, true),
            ],
        };

        registry.apply_overrides(&[SkillOverride {
            skill_name: "web-search".to_string(),
            always_on: Some(true),
            llm_provider: None,
            llm_model: None,
            enabled: None,
            ..Default::default()
        }]);

        assert!(registry.skills[0].manifest.skill.always_on);
        assert!(registry.skills[0].has_override);
        assert!(!registry.skills[1].manifest.skill.always_on);
        assert!(!registry.skills[1].has_override);
    }

    #[test]
    fn test_apply_overrides_case_insensitive_match() {
        use crate::db::SkillOverride;

        let mut registry = SkillRegistry {
            skipped: Vec::new(),
            disabled: Vec::new(),
            validated_warnings: Vec::new(),
            skills: vec![make_entry("Web-Search", false, true)],
        };

        registry.apply_overrides(&[SkillOverride {
            skill_name: "web-search".to_string(),
            always_on: Some(true),
            llm_provider: None,
            llm_model: None,
            enabled: None,
            ..Default::default()
        }]);

        assert!(registry.skills[0].manifest.skill.always_on);
        assert!(registry.skills[0].has_override);
    }

    #[test]
    fn test_apply_overrides_none_skips() {
        use crate::db::SkillOverride;

        let mut registry = SkillRegistry {
            skipped: Vec::new(),
            disabled: Vec::new(),
            validated_warnings: Vec::new(),
            skills: vec![make_entry("web-search", false, true)],
        };

        registry.apply_overrides(&[SkillOverride {
            skill_name: "web-search".to_string(),
            always_on: None,
            llm_provider: None,
            llm_model: None,
            enabled: None,
            ..Default::default()
        }]);

        assert!(!registry.skills[0].manifest.skill.always_on);
        assert!(!registry.skills[0].has_override);
    }

    #[test]
    fn test_apply_overrides_nonexistent_skill_ignored() {
        use crate::db::SkillOverride;

        let mut registry = SkillRegistry {
            skipped: Vec::new(),
            disabled: Vec::new(),
            validated_warnings: Vec::new(),
            skills: vec![make_entry("web-search", false, true)],
        };

        registry.apply_overrides(&[SkillOverride {
            skill_name: "nonexistent".to_string(),
            always_on: Some(true),
            llm_provider: None,
            llm_model: None,
            enabled: None,
            ..Default::default()
        }]);

        // No crash, web-search unchanged
        assert!(!registry.skills[0].manifest.skill.always_on);
    }

    #[test]
    fn test_apply_overrides_affects_always_on_skills_filter() {
        use crate::db::SkillOverride;

        let mut registry = SkillRegistry {
            skipped: Vec::new(),
            disabled: Vec::new(),
            validated_warnings: Vec::new(),
            skills: vec![
                make_entry("web-search", false, true),
                make_entry("shell-exec", true, true),
            ],
        };

        assert_eq!(registry.always_on_skills().len(), 1);

        registry.apply_overrides(&[SkillOverride {
            skill_name: "web-search".to_string(),
            always_on: Some(true),
            llm_provider: None,
            llm_model: None,
            enabled: None,
            ..Default::default()
        }]);

        assert_eq!(registry.always_on_skills().len(), 2);
    }

    #[test]
    fn test_apply_overrides_validates_dependencies_silent_on_valid() {
        let mut registry = SkillRegistry {
            skipped: Vec::new(),
            disabled: Vec::new(),
            validated_warnings: Vec::new(),
            skills: vec![
                make_entry_with_deps("self-dev", true, true, &["tmux"]),
                make_entry("tmux", false, true),
            ],
        };
        // Should not panic — valid dependency validated during apply_overrides
        registry.apply_overrides(&[]);
    }

    #[test]
    fn test_apply_overrides_validates_dependencies_warns_on_missing() {
        let mut registry = SkillRegistry {
            skipped: Vec::new(),
            disabled: Vec::new(),
            validated_warnings: Vec::new(),
            skills: vec![make_entry_with_deps(
                "self-dev",
                true,
                true,
                &["nonexistent"],
            )],
        };
        // Should not panic — logs warning but doesn't fail
        registry.apply_overrides(&[]);
    }

    #[test]
    fn test_apply_overrides_validates_dependencies_case_insensitive() {
        let mut registry = SkillRegistry {
            skipped: Vec::new(),
            disabled: Vec::new(),
            validated_warnings: Vec::new(),
            skills: vec![
                make_entry_with_deps("self-dev", true, true, &["TMUX"]),
                make_entry("tmux", false, true),
            ],
        };
        // Should not warn — case-insensitive match via eq_ignore_ascii_case
        registry.apply_overrides(&[]);
    }

    #[test]
    fn test_apply_overrides_validates_dependencies_no_deps_no_warn() {
        let mut registry = SkillRegistry {
            skipped: Vec::new(),
            disabled: Vec::new(),
            validated_warnings: Vec::new(),
            skills: vec![
                make_entry("web-search", true, true),
                make_entry("tmux", false, true),
            ],
        };
        // No dependencies declared — nothing to validate
        registry.apply_overrides(&[]);
    }

    #[test]
    fn test_apply_overrides_merges_llm_columns() {
        use crate::db::SkillOverride;

        let mut registry = SkillRegistry {
            skipped: Vec::new(),
            disabled: Vec::new(),
            validated_warnings: Vec::new(),
            skills: vec![make_entry("qa-review", false, true)],
        };
        // Baseline: manifest [llm] empty.
        assert!(registry.skills[0].manifest.llm.is_empty());

        registry.apply_overrides(&[SkillOverride {
            skill_name: "qa-review".to_string(),
            always_on: None,
            llm_provider: Some("anthropic".to_string()),
            llm_model: Some("claude-sonnet-4-6".to_string()),
            enabled: None,
            ..Default::default()
        }]);

        assert!(registry.skills[0].has_override);
        assert_eq!(
            registry.skills[0].manifest.llm.provider.as_deref(),
            Some("anthropic")
        );
        assert_eq!(
            registry.skills[0].manifest.llm.model.as_deref(),
            Some("claude-sonnet-4-6")
        );
    }

    #[test]
    fn test_apply_overrides_llm_partial_merges_onto_manifest() {
        use crate::db::SkillOverride;
        use crate::skills::manifest::LlmOverride;

        let mut entry = make_entry("qa-review", false, true);
        // Manifest already sets provider + model (author default).
        entry.manifest.llm = LlmOverride {
            provider: Some("deepseek".to_string()),
            model: Some("deepseek-chat".to_string()),
            ..Default::default()
        };
        let mut registry = SkillRegistry {
            skipped: Vec::new(),
            disabled: Vec::new(),
            validated_warnings: Vec::new(),
            skills: vec![entry],
        };

        // DB override supplies only the model — provider should stay as manifest.
        registry.apply_overrides(&[SkillOverride {
            skill_name: "qa-review".to_string(),
            always_on: None,
            llm_provider: None,
            llm_model: Some("deepseek-reasoner".to_string()),
            enabled: None,
            ..Default::default()
        }]);

        assert!(registry.skills[0].has_override);
        assert_eq!(
            registry.skills[0].manifest.llm.provider.as_deref(),
            Some("deepseek"),
            "provider should remain as manifest default"
        );
        assert_eq!(
            registry.skills[0].manifest.llm.model.as_deref(),
            Some("deepseek-reasoner"),
            "model should be overridden by DB"
        );
    }

    #[test]
    fn test_apply_overrides_keeps_always_on_override_with_empty_prompt() {
        use crate::db::SkillOverride;

        // Tool-only skill with empty prompt — always_on override should apply
        // without removing the skill. Empty prompt is valid for tool-only skills.
        let mut entry = make_entry("tool-only", false, true);
        entry.prompt_snippet = String::new();

        let mut registry = SkillRegistry {
            skipped: Vec::new(),
            disabled: Vec::new(),
            validated_warnings: Vec::new(),
            skills: vec![entry],
        };

        registry.apply_overrides(&[SkillOverride {
            skill_name: "tool-only".to_string(),
            always_on: Some(true),
            llm_provider: None,
            llm_model: None,
            enabled: None,
            ..Default::default()
        }]);

        // Should still be present — no prompt file means tool-only, not broken
        assert_eq!(registry.skills.len(), 1);
        assert!(registry.skills[0].manifest.skill.always_on);
    }

    // -- Eviction tests (#629) --

    #[test]
    fn test_apply_overrides_evicts_disabled_skill() {
        let mut registry = SkillRegistry {
            skipped: Vec::new(),
            disabled: Vec::new(),
            validated_warnings: Vec::new(),
            skills: vec![
                make_entry("web-search", false, true),
                make_entry("memory", false, true),
            ],
        };

        registry.apply_overrides(&[SkillOverride {
            skill_name: "web-search".to_string(),
            always_on: None,
            llm_provider: None,
            llm_model: None,
            enabled: Some(false),
            ..Default::default()
        }]);

        assert_eq!(registry.skills.len(), 1);
        assert_eq!(registry.skills[0].manifest.skill.name, "memory");
        assert_eq!(registry.disabled.len(), 1);
        assert_eq!(registry.disabled[0].name, "web-search");
    }

    #[test]
    fn test_apply_overrides_enabled_none_keeps_skill() {
        let mut registry = SkillRegistry {
            skipped: Vec::new(),
            disabled: Vec::new(),
            validated_warnings: Vec::new(),
            skills: vec![make_entry("web-search", false, true)],
        };

        registry.apply_overrides(&[SkillOverride {
            skill_name: "web-search".to_string(),
            always_on: None,
            llm_provider: None,
            llm_model: None,
            enabled: None,
            ..Default::default()
        }]);

        assert_eq!(registry.skills.len(), 1);
        assert!(registry.disabled.is_empty());
    }

    #[test]
    fn test_apply_overrides_enabled_true_keeps_skill() {
        let mut registry = SkillRegistry {
            skipped: Vec::new(),
            disabled: Vec::new(),
            validated_warnings: Vec::new(),
            skills: vec![make_entry("web-search", false, true)],
        };

        registry.apply_overrides(&[SkillOverride {
            skill_name: "web-search".to_string(),
            always_on: None,
            llm_provider: None,
            llm_model: None,
            enabled: Some(true),
            ..Default::default()
        }]);

        assert_eq!(registry.skills.len(), 1);
        assert!(registry.disabled.is_empty());
    }

    #[test]
    fn test_apply_overrides_disabled_wins_over_always_on() {
        let mut registry = SkillRegistry {
            skipped: Vec::new(),
            disabled: Vec::new(),
            validated_warnings: Vec::new(),
            skills: vec![make_entry("web-search", true, true)],
        };

        registry.apply_overrides(&[SkillOverride {
            skill_name: "web-search".to_string(),
            always_on: Some(true),
            llm_provider: None,
            llm_model: None,
            enabled: Some(false),
            ..Default::default()
        }]);

        assert!(registry.skills.is_empty());
        assert_eq!(registry.disabled.len(), 1);
    }

    #[test]
    fn test_apply_overrides_disabled_nonexistent_skill_ignored() {
        let mut registry = SkillRegistry {
            skipped: Vec::new(),
            disabled: Vec::new(),
            validated_warnings: Vec::new(),
            skills: vec![make_entry("web-search", false, true)],
        };

        registry.apply_overrides(&[SkillOverride {
            skill_name: "nonexistent".to_string(),
            always_on: None,
            llm_provider: None,
            llm_model: None,
            enabled: Some(false),
            ..Default::default()
        }]);

        assert_eq!(registry.skills.len(), 1);
        assert!(registry.disabled.is_empty());
    }

    #[test]
    fn test_apply_overrides_disabled_not_passed_to_validation() {
        let tmp = tempfile::tempdir().unwrap();
        let skills_dir = tmp.path();
        create_valid_skill(skills_dir, "alpha");
        create_valid_skill(skills_dir, "beta");

        let mut registry = registry_from_temp(skills_dir);
        assert_eq!(registry.skills().len(), 2);

        registry.apply_overrides(&[SkillOverride {
            skill_name: "alpha".to_string(),
            always_on: None,
            llm_provider: None,
            llm_model: None,
            enabled: Some(false),
            ..Default::default()
        }]);
        registry.apply_load_safety_check();

        // alpha evicted before apply_load_safety_check, not in validated_warnings either
        assert_eq!(registry.skills().len(), 1);
        assert_eq!(registry.disabled().len(), 1);
    }

    // -- migrate_disabled_markers tests (#629) --

    #[test]
    fn test_migrate_disabled_markers_basic() {
        let tmp = tempfile::tempdir().unwrap();
        let skills_dir = tmp.path();
        let skill_dir = skills_dir.join("foo");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(skill_dir.join(".disabled"), "").unwrap();

        let mut db = crate::db::Database::open_in_memory().unwrap();
        migrate_disabled_markers(skills_dir, &mut db, "mika").unwrap();

        let overrides = db.get_skill_overrides("mika").unwrap();
        assert_eq!(overrides.len(), 1);
        assert_eq!(overrides[0].skill_name, "foo");
        assert_eq!(overrides[0].enabled, Some(false));
        // Marker file should be removed
        assert!(!skill_dir.join(".disabled").exists());
    }

    #[test]
    fn test_migrate_disabled_markers_no_markers() {
        let tmp = tempfile::tempdir().unwrap();
        let skills_dir = tmp.path();
        let skill_dir = skills_dir.join("foo");
        std::fs::create_dir_all(&skill_dir).unwrap();
        // No .disabled marker

        let mut db = crate::db::Database::open_in_memory().unwrap();
        migrate_disabled_markers(skills_dir, &mut db, "mika").unwrap();

        assert!(db.get_skill_overrides("mika").unwrap().is_empty());
    }

    #[test]
    fn test_migrate_disabled_markers_idempotent() {
        let tmp = tempfile::tempdir().unwrap();
        let skills_dir = tmp.path();
        let skill_dir = skills_dir.join("foo");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(skill_dir.join(".disabled"), "").unwrap();

        let mut db = crate::db::Database::open_in_memory().unwrap();
        // First run: writes override, removes marker
        migrate_disabled_markers(skills_dir, &mut db, "mika").unwrap();
        // Second run: no markers, no-op
        migrate_disabled_markers(skills_dir, &mut db, "mika").unwrap();

        let overrides = db.get_skill_overrides("mika").unwrap();
        assert_eq!(overrides.len(), 1);
    }

    #[test]
    fn test_migrate_disabled_markers_multiple_skills() {
        let tmp = tempfile::tempdir().unwrap();
        let skills_dir = tmp.path();
        // Two skills with markers, one without
        let foo = skills_dir.join("foo");
        let bar = skills_dir.join("bar");
        let baz = skills_dir.join("baz");
        std::fs::create_dir_all(&foo).unwrap();
        std::fs::create_dir_all(&bar).unwrap();
        std::fs::create_dir_all(&baz).unwrap();
        std::fs::write(foo.join(".disabled"), "").unwrap();
        std::fs::write(bar.join(".disabled"), "").unwrap();
        // baz has no marker

        let mut db = crate::db::Database::open_in_memory().unwrap();
        migrate_disabled_markers(skills_dir, &mut db, "mika").unwrap();

        let overrides = db.get_skill_overrides("mika").unwrap();
        assert_eq!(overrides.len(), 2);
        assert!(overrides.iter().all(|o| o.enabled == Some(false)));
        assert!(!foo.join(".disabled").exists());
        assert!(!bar.join(".disabled").exists());
    }

    #[tokio::test]
    async fn test_migrate_disabled_markers_async_basic() {
        let tmp = tempfile::tempdir().unwrap();
        let skills_dir = tmp.path();
        let skill_dir = skills_dir.join("foo");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(skill_dir.join(".disabled"), "").unwrap();

        let db = crate::db::Database::open_in_memory().unwrap();
        let async_db = crate::async_db::AsyncDatabase::new_with_agent(db, "mika");
        migrate_disabled_markers_async(skills_dir, &async_db, "mika")
            .await
            .unwrap();

        let overrides = async_db.get_skill_overrides("mika").await.unwrap();
        assert_eq!(overrides.len(), 1);
        assert_eq!(overrides[0].skill_name, "foo");
        assert_eq!(overrides[0].enabled, Some(false));
        assert!(!skill_dir.join(".disabled").exists());
    }

    #[test]
    fn test_migrate_disabled_markers_nonexistent_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let skills_dir = tmp.path().join("nonexistent");

        let mut db = crate::db::Database::open_in_memory().unwrap();
        // Should not error on missing directory
        migrate_disabled_markers(&skills_dir, &mut db, "mika").unwrap();
    }

    // -- validate_markdown_content tests (#511) --

    #[test]
    fn test_validate_markdown_content_valid() {
        assert!(validate_markdown_content("# Hello\n\nSome text.\n").is_ok());
        assert!(validate_markdown_content("Simple text.").is_ok());
        assert!(validate_markdown_content("```rust\nfn main() {}\n```\n").is_ok());
    }

    #[test]
    fn test_validate_markdown_content_empty() {
        assert!(validate_markdown_content("").is_err());
        assert!(validate_markdown_content("   ").is_err());
        assert!(validate_markdown_content("\n\n").is_err());
    }

    #[test]
    fn test_validate_markdown_content_null_bytes() {
        assert!(validate_markdown_content("hello\0world").is_err());
    }

    #[test]
    fn test_validate_markdown_content_control_chars() {
        assert!(validate_markdown_content("hello\x01world").is_err());
        assert!(validate_markdown_content("hello\x07world").is_err());
    }

    #[test]
    fn test_validate_markdown_content_unclosed_fence() {
        assert!(validate_markdown_content("```\ncode here\n").is_err());
        assert!(validate_markdown_content("text\n```rust\ncode\n").is_err());
    }

    #[test]
    fn test_validate_markdown_content_balanced_fences() {
        assert!(validate_markdown_content("```\ncode\n```\n").is_ok());
        assert!(validate_markdown_content("```rust\ncode\n```\n```\nmore\n```\n").is_ok());
    }

    #[test]
    fn test_validate_markdown_content_allows_common_whitespace() {
        // Tabs, newlines, carriage returns are fine
        assert!(validate_markdown_content("hello\tworld\r\n").is_ok());
    }

    // -- apply_load_safety_check() tests (#530, #1335) --

    /// Helper: create a minimal valid skill directory for apply_load_safety_check() tests.
    /// Returns the skill subdirectory path.
    fn create_valid_skill(parent: &std::path::Path, name: &str) -> std::path::PathBuf {
        let skill_dir = parent.join(name);
        std::fs::create_dir_all(&skill_dir).unwrap();
        // Include a keyword to avoid "skill will never activate" warning from validate_skill()
        std::fs::write(
            skill_dir.join("skill.toml"),
            format!(
                r#"[skill]
name = "{name}"
description = "test skill"

[triggers]
keywords = ["test-{name}"]
"#
            ),
        )
        .unwrap();
        std::fs::write(
            skill_dir.join("system_prompt.md"),
            "# Test\nValid prompt content.\n",
        )
        .unwrap();
        skill_dir
    }

    /// Helper: build a SkillRegistry from a temp dir with real skills on disk.
    fn registry_from_temp(skills_dir: &std::path::Path) -> SkillRegistry {
        SkillRegistry::from_dir(skills_dir)
    }

    #[test]
    fn test_apply_load_safety_check_no_issues() {
        let tmp = tempfile::tempdir().unwrap();
        create_valid_skill(tmp.path(), "good-skill");
        let mut registry = registry_from_temp(tmp.path());
        registry.apply_load_safety_check();

        assert_eq!(registry.skills().len(), 1);
        assert!(registry.skipped().is_empty());
        assert!(
            registry.validated_warnings().is_empty(),
            "unexpected warnings: {:?}",
            registry.validated_warnings()
        );
    }

    #[test]
    fn test_apply_load_safety_check_empty_registry() {
        let mut registry = SkillRegistry::empty();
        registry.apply_load_safety_check();

        assert!(registry.skills().is_empty());
        assert!(registry.skipped().is_empty());
        assert!(registry.validated_warnings().is_empty());
    }

    #[test]
    fn test_apply_load_safety_check_llm_section_warns_not_skipped() {
        let tmp = tempfile::tempdir().unwrap();
        let skill_dir = create_valid_skill(tmp.path(), "has-llm");
        // Add a deprecated [llm] section to skill.toml
        std::fs::write(
            skill_dir.join("skill.toml"),
            r#"[skill]
name = "has-llm"
description = "test"

[triggers]
keywords = ["llm-test"]

[llm]
provider = "anthropic"
model = "claude-sonnet-4-6"
"#,
        )
        .unwrap();
        let mut registry = registry_from_temp(tmp.path());
        registry.apply_load_safety_check();

        // Skill should still be loaded (not skipped)
        assert_eq!(registry.skills().len(), 1);
        assert!(registry.skipped().is_empty());
        // Should have a validation warning
        assert_eq!(registry.validated_warnings().len(), 1);
        assert_eq!(registry.validated_warnings()[0].skill_name, "has-llm");
    }

    #[test]
    fn test_apply_load_safety_check_name_in_keywords_warns_not_skipped() {
        let tmp = tempfile::tempdir().unwrap();
        let skill_dir = create_valid_skill(tmp.path(), "mem-skill");
        // Overwrite with name-in-keywords: "mem-skill" appears both as name and keyword
        std::fs::write(
            skill_dir.join("skill.toml"),
            r#"[skill]
name = "mem-skill"
description = "test"

[triggers]
keywords = ["mem-skill", "remember"]
"#,
        )
        .unwrap();
        let mut registry = registry_from_temp(tmp.path());
        registry.apply_load_safety_check();

        assert_eq!(registry.skills().len(), 1);
        assert!(registry.skipped().is_empty());
        assert_eq!(registry.validated_warnings().len(), 1);
        assert_eq!(registry.validated_warnings()[0].skill_name, "mem-skill");
    }

    #[test]
    fn test_apply_load_safety_check_missing_handler_script_skips() {
        let tmp = tempfile::tempdir().unwrap();
        let skill_dir = create_valid_skill(tmp.path(), "broken-handler");
        // Add tools.json referencing a handler that doesn't exist
        std::fs::write(
            skill_dir.join("tools.json"),
            r#"[{
                "name": "run_something",
                "description": "runs something",
                "input_schema": {"type": "object"},
                "handler": {"type": "exec", "command": "./nonexistent.sh"}
            }]"#,
        )
        .unwrap();
        let mut registry = registry_from_temp(tmp.path());
        assert_eq!(registry.skills().len(), 1);
        registry.apply_load_safety_check();

        // Skill should be skipped
        assert_eq!(registry.skills().len(), 0);
        assert!(
            registry
                .skipped()
                .iter()
                .any(|s| s.name == "broken-handler")
        );
        assert!(registry.validated_warnings().is_empty());
    }

    #[test]
    fn test_apply_load_safety_check_handler_not_executable_skips() {
        let tmp = tempfile::tempdir().unwrap();
        let skill_dir = create_valid_skill(tmp.path(), "not-exec");
        // Create handler script without execute permission
        let script_path = skill_dir.join("run.sh");
        std::fs::write(&script_path, "#!/bin/sh\necho hello").unwrap();
        // Explicitly remove execute permission
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&script_path, std::fs::Permissions::from_mode(0o644)).unwrap();
        }
        std::fs::write(
            skill_dir.join("tools.json"),
            r#"[{
                "name": "run_it",
                "description": "runs it",
                "input_schema": {"type": "object"},
                "handler": {"type": "exec", "command": "./run.sh"}
            }]"#,
        )
        .unwrap();
        let mut registry = registry_from_temp(tmp.path());
        assert_eq!(registry.skills().len(), 1);
        registry.apply_load_safety_check();

        // Skill should be skipped (handler not executable)
        assert_eq!(registry.skills().len(), 0);
        assert!(registry.skipped().iter().any(|s| s.name == "not-exec"));
    }

    #[test]
    fn test_apply_load_safety_check_invalid_tools_json_skips() {
        let tmp = tempfile::tempdir().unwrap();
        let skill_dir = create_valid_skill(tmp.path(), "bad-tools");
        std::fs::write(skill_dir.join("tools.json"), "not valid json!!!").unwrap();
        let mut registry = registry_from_temp(tmp.path());
        assert_eq!(registry.skills().len(), 1);
        registry.apply_load_safety_check();

        assert_eq!(registry.skills().len(), 0);
        assert!(registry.skipped().iter().any(|s| s.name == "bad-tools"));
    }

    #[test]
    fn test_apply_load_safety_check_skip_worthy_and_warn_both_present_skips() {
        let tmp = tempfile::tempdir().unwrap();
        let skill_dir = create_valid_skill(tmp.path(), "mixed-issues");
        // Has both: deprecated [llm] section (warn) AND missing handler (skip)
        std::fs::write(
            skill_dir.join("skill.toml"),
            r#"[skill]
name = "mixed-issues"
description = "test"

[triggers]
keywords = ["mixed-test"]

[llm]
provider = "anthropic"
"#,
        )
        .unwrap();
        std::fs::write(
            skill_dir.join("tools.json"),
            r#"[{
                "name": "do_thing",
                "description": "does thing",
                "input_schema": {"type": "object"},
                "handler": {"type": "exec", "command": "./missing.sh"}
            }]"#,
        )
        .unwrap();
        let mut registry = registry_from_temp(tmp.path());
        assert_eq!(registry.skills().len(), 1);
        registry.apply_load_safety_check();

        // Skip takes precedence over warn
        assert_eq!(registry.skills().len(), 0);
        assert!(registry.skipped().iter().any(|s| s.name == "mixed-issues"));
        // Should NOT appear in validated_warnings since it was skipped
        assert!(
            registry
                .validated_warnings()
                .iter()
                .all(|w| w.skill_name != "mixed-issues")
        );
    }

    #[test]
    fn test_apply_load_safety_check_warn_only_diagnostics_kept() {
        let tmp = tempfile::tempdir().unwrap();
        let skill_dir = create_valid_skill(tmp.path(), "warn-only");
        // Create a markdown file with unclosed code fence (warns, doesn't skip)
        std::fs::write(
            skill_dir.join("system_prompt.md"),
            "# Test\n```\nunclosed code\n",
        )
        .unwrap();
        let mut registry = registry_from_temp(tmp.path());
        registry.apply_load_safety_check();

        assert_eq!(registry.skills().len(), 1);
        assert!(registry.skipped().is_empty());
        // Should have validation warning for markdown
        assert_eq!(registry.validated_warnings().len(), 1);
        assert_eq!(registry.validated_warnings()[0].skill_name, "warn-only");
    }

    #[test]
    fn test_apply_load_safety_check_multiple_skills_mixed() {
        let tmp = tempfile::tempdir().unwrap();
        // Good skill — no issues
        create_valid_skill(tmp.path(), "good");
        // Warn skill — deprecated [llm] section
        let warn_dir = create_valid_skill(tmp.path(), "has-warn");
        std::fs::write(
            warn_dir.join("skill.toml"),
            r#"[skill]
name = "has-warn"
description = "test"

[triggers]
keywords = ["warn-test"]

[llm]
provider = "anthropic"
"#,
        )
        .unwrap();
        // Skip skill — missing handler
        let skip_dir = create_valid_skill(tmp.path(), "will-skip");
        std::fs::write(
            skip_dir.join("tools.json"),
            r#"[{
                "name": "broken",
                "description": "broken",
                "input_schema": {"type": "object"},
                "handler": {"type": "exec", "command": "./missing.sh"}
            }]"#,
        )
        .unwrap();

        let mut registry = registry_from_temp(tmp.path());
        assert_eq!(registry.skills().len(), 3);
        registry.apply_load_safety_check();

        // 2 skills remain (good + has-warn), 1 skipped (will-skip)
        assert_eq!(registry.skills().len(), 2);
        assert!(registry.skipped().iter().any(|s| s.name == "will-skip"));
        // 1 validation warning (has-warn)
        assert_eq!(registry.validated_warnings().len(), 1);
        assert_eq!(registry.validated_warnings()[0].skill_name, "has-warn");
    }

    #[test]
    fn test_apply_load_safety_check_symlink_race_all_fail_no_ok() {
        let tmp = tempfile::tempdir().unwrap();
        let skill_dir = create_valid_skill(tmp.path(), "disappearing");
        // Load registry while skill exists
        let mut registry = registry_from_temp(tmp.path());
        assert_eq!(registry.skills().len(), 1);
        // Remove the skill.toml to simulate symlink race
        std::fs::remove_file(skill_dir.join("skill.toml")).unwrap();
        registry.apply_load_safety_check();

        // Should be skipped due to catch-all (all-Fail, zero-Ok)
        assert!(registry.skills().is_empty());
        assert!(registry.skipped().iter().any(|s| s.name == "disappearing"));
    }

    // -- is_skip_worthy_failure() tests --

    #[test]
    fn test_apply_load_safety_check_always_on_oversized_prompt_skips() {
        let tmp = tempfile::tempdir().unwrap();
        let skill_dir = create_valid_skill(tmp.path(), "big-prompt");
        // Make it always_on with a prompt exceeding the default 16KB limit
        std::fs::write(
            skill_dir.join("skill.toml"),
            r#"[skill]
name = "big-prompt"
description = "test"
always_on = true

[triggers]
keywords = ["big-test"]
"#,
        )
        .unwrap();
        // Write an oversized prompt (20KB exceeds the 16KB default)
        let big_content = format!("# Big Prompt\n{}", "x".repeat(20 * 1024));
        std::fs::write(skill_dir.join("system_prompt.md"), &big_content).unwrap();

        let mut registry = registry_from_temp(tmp.path());
        // Skill loads during scan (prompt is silently emptied for non-always_on at scan)
        // but the entry may still be present
        let loaded_before = registry.skills().len();
        registry.apply_load_safety_check();

        // validate_skill() emits a Fail saying "skill will be SKIPPED at startup"
        // for always_on skills with oversized prompts — is_skip_worthy_failure must catch it
        if loaded_before > 0 {
            assert!(
                registry.skipped().iter().any(|s| s.name == "big-prompt"),
                "always_on skill with oversized prompt should be skipped"
            );
        }
    }

    #[test]
    fn test_apply_load_safety_check_skip_reason_contains_diagnostic() {
        let tmp = tempfile::tempdir().unwrap();
        let skill_dir = create_valid_skill(tmp.path(), "bad-handler");
        std::fs::write(
            skill_dir.join("tools.json"),
            r#"[{
                "name": "run_it",
                "description": "runs it",
                "input_schema": {"type": "object"},
                "handler": {"type": "exec", "command": "./missing.sh"}
            }]"#,
        )
        .unwrap();
        let mut registry = registry_from_temp(tmp.path());
        registry.apply_load_safety_check();

        let skipped = registry.skipped().iter().find(|s| s.name == "bad-handler");
        assert!(skipped.is_some(), "skill should be skipped");
        let reason = &skipped.unwrap().reason;
        assert!(
            reason.starts_with("validation: "),
            "reason should start with 'validation: ', got: {reason}"
        );
        assert!(
            reason.contains("handler command not found"),
            "reason should mention the handler, got: {reason}"
        );
    }

    #[test]
    fn test_is_skip_worthy_failure_oversized_always_on_prompt() {
        use index::{SkillDiagnostic, is_skip_worthy_failure};
        let diag = SkillDiagnostic::fail(
            "system_prompt.md (20480 bytes) exceeds limit (16384 bytes) — skill will be SKIPPED at startup \
             (always_on skills require their prompt to function)",
        );
        assert!(is_skip_worthy_failure(&diag));
    }

    #[test]
    fn test_is_skip_worthy_failure_handler_not_found() {
        use index::{SkillDiagnostic, is_skip_worthy_failure};
        let diag = SkillDiagnostic::fail(
            "tool 'run_it': handler command not found: ./run.sh (resolved to /skills/run.sh)",
        );
        assert!(is_skip_worthy_failure(&diag));
    }

    #[test]
    fn test_is_skip_worthy_failure_handler_not_executable() {
        use index::{SkillDiagnostic, is_skip_worthy_failure};
        let diag =
            SkillDiagnostic::fail("tool 'run_it': handler command not executable: /skills/run.sh");
        assert!(is_skip_worthy_failure(&diag));
    }

    #[test]
    fn test_is_skip_worthy_failure_invalid_tools_json() {
        use index::{SkillDiagnostic, is_skip_worthy_failure};
        let diag = SkillDiagnostic::fail("invalid tools.json: expected ident at line 1 column 2");
        assert!(is_skip_worthy_failure(&diag));
    }

    #[test]
    fn test_is_skip_worthy_failure_cannot_read_tools_json() {
        use index::{SkillDiagnostic, is_skip_worthy_failure};
        let diag = SkillDiagnostic::fail("cannot read tools.json: Permission denied");
        assert!(is_skip_worthy_failure(&diag));
    }

    #[test]
    fn test_is_skip_worthy_failure_tools_json_oversized() {
        use index::{SkillDiagnostic, is_skip_worthy_failure};
        let diag = SkillDiagnostic::fail("tools.json exceeds 256KB (512KB)");
        assert!(is_skip_worthy_failure(&diag));
    }

    #[test]
    fn test_is_skip_worthy_failure_manifest_not_found() {
        use index::{SkillDiagnostic, is_skip_worthy_failure};
        let diag = SkillDiagnostic::fail("skill.toml not found");
        assert!(is_skip_worthy_failure(&diag));
    }

    #[test]
    fn test_is_skip_worthy_failure_manifest_unreadable() {
        use index::{SkillDiagnostic, is_skip_worthy_failure};
        let diag = SkillDiagnostic::fail("cannot read skill.toml: Permission denied");
        assert!(is_skip_worthy_failure(&diag));
    }

    #[test]
    fn test_is_skip_worthy_failure_llm_section_not_skip() {
        use index::{SkillDiagnostic, is_skip_worthy_failure};
        let diag = SkillDiagnostic::fail(
            "[llm] section is no longer supported in skill.toml. Use `mika skills llm`...",
        );
        assert!(!is_skip_worthy_failure(&diag));
    }

    #[test]
    fn test_is_skip_worthy_failure_name_in_keywords_not_skip() {
        use index::{SkillDiagnostic, is_skip_worthy_failure};
        let diag = SkillDiagnostic::fail(
            "skill name 'memory' appears in [triggers].keywords — this is redundant",
        );
        assert!(!is_skip_worthy_failure(&diag));
    }

    #[test]
    fn test_is_skip_worthy_failure_warn_level_not_skip() {
        use index::{SkillDiagnostic, is_skip_worthy_failure};
        // Even if the message matches a skip pattern, Warn level should not be skip-worthy
        let diag = SkillDiagnostic::warn("tool 'x': handler command not found: ./run.sh");
        assert!(!is_skip_worthy_failure(&diag));
    }

    #[test]
    fn test_is_skip_worthy_failure_ok_level_not_skip() {
        use index::{SkillDiagnostic, is_skip_worthy_failure};
        let diag = SkillDiagnostic::ok("skill.toml valid");
        assert!(!is_skip_worthy_failure(&diag));
    }

    // ── apply_transient_always_on tests ──────────────────────────────────

    #[test]
    fn test_transient_always_on_sets_flag() {
        let mut registry = SkillRegistry {
            skipped: Vec::new(),
            disabled: Vec::new(),
            validated_warnings: Vec::new(),
            skills: vec![make_entry("self-dev", false, true)],
        };

        let result = registry.apply_transient_always_on(&["self-dev".to_string()]);
        assert!(result.is_empty());
        assert!(registry.skills[0].manifest.skill.always_on);
        assert!(registry.skills[0].has_override);
    }

    #[test]
    fn test_transient_always_on_multiple_skills() {
        let mut registry = SkillRegistry {
            skipped: Vec::new(),
            disabled: Vec::new(),
            validated_warnings: Vec::new(),
            skills: vec![
                make_entry("skill-a", false, true),
                make_entry("skill-b", false, true),
                make_entry("skill-c", false, true),
            ],
        };

        let result =
            registry.apply_transient_always_on(&["skill-a".to_string(), "skill-b".to_string()]);
        assert!(result.is_empty());
        assert!(registry.skills[0].manifest.skill.always_on);
        assert!(registry.skills[1].manifest.skill.always_on);
        assert!(!registry.skills[2].manifest.skill.always_on);
    }

    #[test]
    fn test_transient_always_on_idempotent_on_already_on() {
        let mut registry = SkillRegistry {
            skipped: Vec::new(),
            disabled: Vec::new(),
            validated_warnings: Vec::new(),
            skills: vec![make_entry("self-dev", true, true)],
        };

        let result = registry.apply_transient_always_on(&["self-dev".to_string()]);
        assert!(result.is_empty());
        assert!(registry.skills[0].manifest.skill.always_on);
        assert!(registry.skills[0].has_override);
    }

    #[test]
    fn test_transient_always_on_nonexistent_returns_not_found() {
        let mut registry = SkillRegistry {
            skipped: Vec::new(),
            disabled: Vec::new(),
            validated_warnings: Vec::new(),
            skills: vec![make_entry("web-search", false, true)],
        };

        let result = registry.apply_transient_always_on(&["nonexistent".to_string()]);
        assert_eq!(result.not_found, vec!["nonexistent"]);
        assert!(result.disabled.is_empty());
        // Existing skill unchanged
        assert!(!registry.skills[0].manifest.skill.always_on);
    }

    #[test]
    fn test_transient_always_on_case_insensitive() {
        let mut registry = SkillRegistry {
            skipped: Vec::new(),
            disabled: Vec::new(),
            validated_warnings: Vec::new(),
            skills: vec![make_entry("Self-Dev", false, true)],
        };

        let result = registry.apply_transient_always_on(&["self-dev".to_string()]);
        assert!(result.is_empty());
        assert!(registry.skills[0].manifest.skill.always_on);
    }

    #[test]
    fn test_transient_always_on_disabled_skill_returns_disabled() {
        let mut registry = SkillRegistry {
            skipped: Vec::new(),
            disabled: vec![DisabledSkill {
                name: "self-dev".to_string(),
            }],
            validated_warnings: Vec::new(),
            skills: vec![make_entry("web-search", false, true)],
        };

        let result = registry.apply_transient_always_on(&["self-dev".to_string()]);
        assert_eq!(result.disabled, vec!["self-dev"]);
        assert!(result.not_found.is_empty());
        // Loaded skill unchanged
        assert!(!registry.skills[0].manifest.skill.always_on);
    }

    #[test]
    fn test_transient_always_on_empty_input_noop() {
        let mut registry = SkillRegistry {
            skipped: Vec::new(),
            disabled: Vec::new(),
            validated_warnings: Vec::new(),
            skills: vec![make_entry("self-dev", false, true)],
        };

        let result = registry.apply_transient_always_on(&[]);
        assert!(result.is_empty());
        assert!(!registry.skills[0].manifest.skill.always_on);
    }

    #[test]
    fn test_transient_always_on_affects_match_skills() {
        use super::matcher::match_skills;

        let mut registry = SkillRegistry {
            skipped: Vec::new(),
            disabled: Vec::new(),
            validated_warnings: Vec::new(),
            skills: vec![make_entry("self-dev", false, true)],
        };

        // Before: no match on unrelated message
        let matched = match_skills(&registry.skills, "hello world");
        assert!(matched.is_empty());

        // Apply transient always_on
        registry.apply_transient_always_on(&["self-dev".to_string()]);

        // After: matches as AlwaysOn
        let matched = match_skills(&registry.skills, "hello world");
        assert_eq!(matched.len(), 1);
        assert_eq!(matched[0].reason, super::matcher::MatchReason::AlwaysOn);
    }

    // ── apply_transient_disable tests ──────────────────────────────────

    #[test]
    fn test_transient_disable_evicts_loaded_skill() {
        let mut registry = SkillRegistry {
            skipped: Vec::new(),
            disabled: Vec::new(),
            validated_warnings: Vec::new(),
            skills: vec![
                make_entry("self-dev", true, true),
                make_entry("web-search", false, true),
            ],
        };

        let result = registry.apply_transient_disable(&["self-dev".to_string()]);
        assert!(result.not_found.is_empty());
        assert_eq!(registry.skills.len(), 1);
        assert_eq!(registry.skills[0].manifest.skill.name, "web-search");
        assert_eq!(registry.disabled.len(), 1);
        assert_eq!(registry.disabled[0].name, "self-dev");
    }

    #[test]
    fn test_transient_disable_multiple_skills() {
        let mut registry = SkillRegistry {
            skipped: Vec::new(),
            disabled: Vec::new(),
            validated_warnings: Vec::new(),
            skills: vec![
                make_entry("skill-a", false, true),
                make_entry("skill-b", true, true),
                make_entry("skill-c", false, true),
            ],
        };

        let result =
            registry.apply_transient_disable(&["skill-a".to_string(), "skill-c".to_string()]);
        assert!(result.not_found.is_empty());
        assert_eq!(registry.skills.len(), 1);
        assert_eq!(registry.skills[0].manifest.skill.name, "skill-b");
        assert_eq!(registry.disabled.len(), 2);
    }

    #[test]
    fn test_transient_disable_nonexistent_returns_not_found() {
        let mut registry = SkillRegistry {
            skipped: Vec::new(),
            disabled: Vec::new(),
            validated_warnings: Vec::new(),
            skills: vec![make_entry("web-search", false, true)],
        };

        let result = registry.apply_transient_disable(&["nonexistent".to_string()]);
        assert_eq!(result.not_found, vec!["nonexistent"]);
        // Existing skill unchanged
        assert_eq!(registry.skills.len(), 1);
    }

    #[test]
    fn test_transient_disable_already_db_disabled_is_noop() {
        let mut registry = SkillRegistry {
            skipped: Vec::new(),
            disabled: vec![DisabledSkill {
                name: "self-dev".to_string(),
            }],
            validated_warnings: Vec::new(),
            skills: vec![make_entry("web-search", false, true)],
        };

        let result = registry.apply_transient_disable(&["self-dev".to_string()]);
        // Already disabled — not reported as not_found
        assert!(result.not_found.is_empty());
        // Still one disabled entry (not duplicated)
        assert_eq!(registry.disabled.len(), 1);
        assert_eq!(registry.skills.len(), 1);
    }

    #[test]
    fn test_transient_disable_case_insensitive() {
        let mut registry = SkillRegistry {
            skipped: Vec::new(),
            disabled: Vec::new(),
            validated_warnings: Vec::new(),
            skills: vec![make_entry("Self-Dev", true, true)],
        };

        let result = registry.apply_transient_disable(&["self-dev".to_string()]);
        assert!(result.not_found.is_empty());
        assert!(registry.skills.is_empty());
        assert_eq!(registry.disabled.len(), 1);
        assert_eq!(registry.disabled[0].name, "Self-Dev");
    }

    #[test]
    fn test_transient_disable_empty_input_noop() {
        let mut registry = SkillRegistry {
            skipped: Vec::new(),
            disabled: Vec::new(),
            validated_warnings: Vec::new(),
            skills: vec![make_entry("self-dev", true, true)],
        };

        let result = registry.apply_transient_disable(&[]);
        assert!(result.not_found.is_empty());
        assert_eq!(registry.skills.len(), 1);
    }

    #[test]
    fn test_transient_disable_removes_from_match_skills() {
        use super::matcher::match_skills;

        let mut registry = SkillRegistry {
            skipped: Vec::new(),
            disabled: Vec::new(),
            validated_warnings: Vec::new(),
            skills: vec![make_entry("self-dev", true, true)],
        };

        // Before: matches as AlwaysOn
        let matched = match_skills(&registry.skills, "hello world");
        assert_eq!(matched.len(), 1);

        // Transiently disable
        registry.apply_transient_disable(&["self-dev".to_string()]);

        // After: no match
        let matched = match_skills(&registry.skills, "hello world");
        assert!(matched.is_empty());
    }

    #[test]
    fn test_transient_disable_removes_from_always_on_skills() {
        let mut registry = SkillRegistry {
            skipped: Vec::new(),
            disabled: Vec::new(),
            validated_warnings: Vec::new(),
            skills: vec![
                make_entry("self-dev", true, true),
                make_entry("qa-review", true, true),
            ],
        };

        assert_eq!(registry.always_on_skills().len(), 2);

        registry.apply_transient_disable(&["self-dev".to_string()]);

        let always_on = registry.always_on_skills();
        assert_eq!(always_on.len(), 1);
        assert_eq!(always_on[0].manifest.skill.name, "qa-review");
    }

    // -- apply_identity_allowlist tests --

    #[test]
    fn test_identity_allowlist_keeps_only_listed_skills() {
        let mut registry = SkillRegistry {
            skipped: Vec::new(),
            disabled: Vec::new(),
            validated_warnings: Vec::new(),
            skills: vec![
                make_entry("skill-a", false, true),
                make_entry("skill-b", false, true),
                make_entry("skill-c", false, true),
            ],
        };

        registry.apply_identity_allowlist(&["skill-a".to_string(), "skill-c".to_string()]);

        assert_eq!(registry.skills.len(), 2);
        assert_eq!(registry.skills[0].manifest.skill.name, "skill-a");
        assert_eq!(registry.skills[1].manifest.skill.name, "skill-c");
        assert_eq!(registry.disabled.len(), 1);
        assert_eq!(registry.disabled[0].name, "skill-b");
    }

    #[test]
    fn test_identity_allowlist_empty_is_noop() {
        let mut registry = SkillRegistry {
            skipped: Vec::new(),
            disabled: Vec::new(),
            validated_warnings: Vec::new(),
            skills: vec![
                make_entry("skill-a", false, true),
                make_entry("skill-b", false, true),
            ],
        };

        registry.apply_identity_allowlist(&[]);

        assert_eq!(registry.skills.len(), 2);
        assert!(registry.disabled.is_empty());
    }

    #[test]
    fn test_identity_allowlist_case_insensitive() {
        let mut registry = SkillRegistry {
            skipped: Vec::new(),
            disabled: Vec::new(),
            validated_warnings: Vec::new(),
            skills: vec![
                make_entry("Skill-A", false, true),
                make_entry("skill-b", false, true),
            ],
        };

        registry.apply_identity_allowlist(&["skill-a".to_string()]);

        assert_eq!(registry.skills.len(), 1);
        assert_eq!(registry.skills[0].manifest.skill.name, "Skill-A");
    }

    #[test]
    fn test_identity_allowlist_unknown_skill_name() {
        // Allowlist references a skill that doesn't exist — no crash, just a warn
        let mut registry = SkillRegistry {
            skipped: Vec::new(),
            disabled: Vec::new(),
            validated_warnings: Vec::new(),
            skills: vec![make_entry("skill-a", false, true)],
        };

        registry.apply_identity_allowlist(&["skill-a".to_string(), "nonexistent".to_string()]);

        assert_eq!(registry.skills.len(), 1);
        assert_eq!(registry.skills[0].manifest.skill.name, "skill-a");
    }

    #[test]
    fn test_identity_allowlist_composes_with_db_overrides() {
        let mut registry = SkillRegistry {
            skipped: Vec::new(),
            disabled: Vec::new(),
            validated_warnings: Vec::new(),
            skills: vec![
                make_entry("skill-a", false, true),
                make_entry("skill-b", false, true),
                make_entry("skill-c", false, true),
            ],
        };

        // Phase -1: identity allowlist keeps only a and b
        registry.apply_identity_allowlist(&["skill-a".to_string(), "skill-b".to_string()]);
        assert_eq!(registry.skills.len(), 2);

        // Phase 0+1: DB overrides apply to survivors
        let overrides = vec![SkillOverride {
            skill_name: "skill-a".to_string(),
            always_on: Some(true),
            enabled: None,
            llm_provider: Some("anthropic".to_string()),
            llm_model: Some("claude-opus-4-7".to_string()),
            ..Default::default()
        }];
        registry.apply_overrides(&overrides);

        // skill-a should have the LLM override applied
        let entry = registry
            .skills
            .iter()
            .find(|e| e.manifest.skill.name == "skill-a")
            .unwrap();
        assert!(entry.manifest.skill.always_on);
        assert_eq!(entry.manifest.llm.provider.as_deref(), Some("anthropic"));
        assert_eq!(entry.manifest.llm.model.as_deref(), Some("claude-opus-4-7"));
    }

    #[test]
    fn test_identity_allowlist_db_disable_wins() {
        // A skill that survives the identity allowlist can still be evicted
        // by a DB-backed enabled=false override (operator customization).
        let mut registry = SkillRegistry {
            skipped: Vec::new(),
            disabled: Vec::new(),
            validated_warnings: Vec::new(),
            skills: vec![
                make_entry("skill-a", false, true),
                make_entry("skill-b", false, true),
            ],
        };

        // Phase -1: allowlist keeps both
        registry.apply_identity_allowlist(&["skill-a".to_string(), "skill-b".to_string()]);
        assert_eq!(registry.skills.len(), 2);

        // Phase 0: DB override disables skill-a
        let overrides = vec![SkillOverride {
            skill_name: "skill-a".to_string(),
            always_on: None,
            enabled: Some(false),
            llm_provider: None,
            llm_model: None,
            ..Default::default()
        }];
        registry.apply_overrides(&overrides);

        // skill-a should be evicted by DB override
        assert_eq!(registry.skills.len(), 1);
        assert_eq!(registry.skills[0].manifest.skill.name, "skill-b");
        // skill-a in disabled list (twice — once identity didn't evict it, then DB did)
        assert!(registry.disabled.iter().any(|d| d.name == "skill-a"));
    }
}
