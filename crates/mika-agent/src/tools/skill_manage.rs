//! `skill_manage` builtin tool — agent-authored skill lifecycle management (mika#1582).
//!
//! Three actions: `create`, `update`, `inspect`. All authored skills land
//! `staged` and require operator promotion before the resolver injects them.
//! Identity-gated: checks `identity.skills.allow_authoring` before executing.

use anyhow::Result;
use async_trait::async_trait;
use mika_common::claude::ToolDefinition;
use serde_json::{Value, json};
use std::sync::atomic::Ordering;

use super::{Tool, ToolContext, ToolOutput};
use crate::prompt::load_identity;
use crate::skills::index::validate_skill;
use crate::tools::create_skill::{
    validate_dependencies, validate_description, validate_keywords, validate_skill_name,
    validate_system_prompt, verify_skill_path,
};

pub struct SkillManageTool;

#[async_trait]
impl Tool for SkillManageTool {
    fn name(&self) -> &str {
        "skill_manage"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "skill_manage".to_string(),
            description: "Manage agent-authored skills: create new skills, update existing ones, \
                or inspect skill details. All authored skills land in 'staged' state and require \
                operator promotion before they activate. Only available when allow_authoring is \
                enabled in identity config."
                .to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": ["create", "update", "inspect"],
                        "description": "Action to perform: create a new skill, update an existing one, or inspect skill details"
                    },
                    "name": {
                        "type": "string",
                        "description": "Skill name (alphanumeric + hyphens/underscores, max 50 chars)"
                    },
                    "description": {
                        "type": "string",
                        "description": "Brief description of what the skill does (required for create/update)"
                    },
                    "keywords": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Keywords that trigger this skill (required for create/update unless always_on)"
                    },
                    "system_prompt": {
                        "type": "string",
                        "description": "The prompt snippet injected when this skill is active (required for create/update)"
                    },
                    "always_on": {
                        "type": "boolean",
                        "description": "If true, this skill is always active regardless of keywords. Default: false"
                    },
                    "dependencies": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Other skill names this skill depends on (max 20)"
                    }
                },
                "required": ["action", "name"]
            }),
        }
    }

    async fn execute(&self, input: Value, ctx: &ToolContext<'_>) -> Result<ToolOutput> {
        // Identity gate: check allow_authoring before any action. Uses the
        // canonical `authoring_enabled()` accessor (mika#1583) instead of the
        // raw field so the identity-config vocabulary stays orthogonal to the
        // Option representation.
        let identity = load_identity(ctx.home_dir);
        if !identity.skills.authoring_enabled() {
            return Ok(ToolOutput::error(
                "Skill authoring is not enabled for this agent. \
                 Set allow_authoring = true in the [skills] section of identity.toml.",
            ));
        }

        let action = input["action"].as_str().unwrap_or("");
        let name = input["name"].as_str().unwrap_or("").trim();

        if let Err(e) = validate_skill_name(name) {
            return Ok(ToolOutput::error(e));
        }

        match action {
            "create" => self.handle_create(name, &input, ctx).await,
            "update" => self.handle_update(name, &input, ctx).await,
            "inspect" => self.handle_inspect(name, ctx).await,
            _ => Ok(ToolOutput::error(format!(
                "Unknown action '{action}'. Valid actions: create, update, inspect"
            ))),
        }
    }
}

impl SkillManageTool {
    async fn handle_create(
        &self,
        name: &str,
        input: &Value,
        ctx: &ToolContext<'_>,
    ) -> Result<ToolOutput> {
        let description = input["description"].as_str().unwrap_or("").trim();
        let system_prompt = input["system_prompt"].as_str().unwrap_or("").trim();
        let always_on = input["always_on"].as_bool().unwrap_or(false);
        let keywords = extract_string_array(&input["keywords"]);
        let dependencies = extract_string_array(&input["dependencies"]);

        // Validate inputs
        if let Err(e) = validate_description(description) {
            return Ok(ToolOutput::error(e));
        }
        if let Err(e) = validate_system_prompt(system_prompt) {
            return Ok(ToolOutput::error(e));
        }
        if keywords.is_empty() && !always_on {
            return Ok(ToolOutput::error(
                "At least one keyword is required (or set always_on to true).",
            ));
        }
        if let Err(e) = validate_keywords(&keywords) {
            return Ok(ToolOutput::error(e));
        }
        // Reject skill name in keywords (#510)
        {
            let name_lower = name.to_ascii_lowercase();
            if keywords
                .iter()
                .any(|k| k.to_ascii_lowercase() == name_lower)
            {
                return Ok(ToolOutput::error(format!(
                    "Skill name '{name}' must not appear in keywords — skills are already matched by name.",
                )));
            }
        }
        if let Err(e) = validate_dependencies(&dependencies) {
            return Ok(ToolOutput::error(e));
        }

        let skills_dir = ctx.home_dir.join("skills");
        let skill_dir = skills_dir.join(name);

        // Check for existing skill
        if skill_dir.exists() {
            return Ok(ToolOutput::error(format!(
                "Skill '{name}' already exists. Use action='update' to modify it, \
                 or choose a different name.",
            )));
        }

        // Atomic write: write to temp dir, validate, rename
        let tmp_dir = skills_dir.join(format!(".{name}.tmp"));
        if tmp_dir.exists() {
            let _ = std::fs::remove_dir_all(&tmp_dir);
        }

        if let Err(e) = write_skill_files(
            &tmp_dir,
            name,
            description,
            system_prompt,
            always_on,
            &keywords,
            &dependencies,
        ) {
            let _ = std::fs::remove_dir_all(&tmp_dir);
            return Ok(ToolOutput::error(format!(
                "Failed to write skill files: {e}"
            )));
        }

        // Validate the skill
        let diagnostics = validate_skill(&tmp_dir);
        let errors: Vec<_> = diagnostics
            .iter()
            .filter(|d| d.level == crate::skills::index::DiagnosticLevel::Fail)
            .collect();
        if !errors.is_empty() {
            let _ = std::fs::remove_dir_all(&tmp_dir);
            let msgs: Vec<String> = errors.iter().map(|d| d.message.clone()).collect();
            return Ok(ToolOutput::error(format!(
                "Skill validation failed:\n{}",
                msgs.join("\n")
            )));
        }

        // Atomic rename
        if let Err(e) = std::fs::rename(&tmp_dir, &skill_dir) {
            let _ = std::fs::remove_dir_all(&tmp_dir);
            return Ok(ToolOutput::error(format!(
                "Failed to finalize skill directory: {e}"
            )));
        }

        // Verify path safety
        if let Err(e) = verify_skill_path(&skills_dir, &skill_dir) {
            let _ = std::fs::remove_dir_all(&skill_dir);
            return Ok(ToolOutput::error(e));
        }

        // Insert skill_overrides row with lifecycle_state = 'staged'
        let agent_id = &ctx.db.agent_id;
        if let Err(e) = ctx
            .db
            .with_db({
                let agent_id = agent_id.clone();
                let skill_name = name.to_string();
                move |db| {
                    db.conn.execute(
                        "INSERT INTO skill_overrides (agent_id, skill_name, lifecycle_state)
                         VALUES (?1, ?2, 'staged')
                         ON CONFLICT(agent_id, skill_name) DO UPDATE SET lifecycle_state = 'staged'",
                        rusqlite::params![agent_id, skill_name],
                    )?;
                    Ok(())
                }
            })
            .await
        {
            tracing::warn!(skill = name, error = %e, "failed to insert skill_overrides row");
        }

        // Mark skills dirty for hot-reload
        ctx.skills_dirty.store(true, Ordering::Release);

        let warnings: Vec<String> = diagnostics
            .iter()
            .filter(|d| d.level == crate::skills::index::DiagnosticLevel::Warn)
            .map(|d| d.message.clone())
            .collect();

        let mut result = json!({
            "status": "created",
            "name": name,
            "lifecycle_state": "staged",
            "message": format!(
                "Skill '{name}' created in staged state. An operator must promote it \
                 via `mika skills promote {name}` or the API before it activates."
            ),
        });
        if !warnings.is_empty() {
            result["validation_warnings"] = json!(warnings);
        }

        Ok(ToolOutput::success(
            serde_json::to_string_pretty(&result).unwrap_or_default(),
        ))
    }

    async fn handle_update(
        &self,
        name: &str,
        input: &Value,
        ctx: &ToolContext<'_>,
    ) -> Result<ToolOutput> {
        let description = input["description"].as_str().unwrap_or("").trim();
        let system_prompt = input["system_prompt"].as_str().unwrap_or("").trim();
        let always_on = input["always_on"].as_bool().unwrap_or(false);
        let keywords = extract_string_array(&input["keywords"]);
        let dependencies = extract_string_array(&input["dependencies"]);

        // Validate inputs
        if let Err(e) = validate_description(description) {
            return Ok(ToolOutput::error(e));
        }
        if let Err(e) = validate_system_prompt(system_prompt) {
            return Ok(ToolOutput::error(e));
        }
        if keywords.is_empty() && !always_on {
            return Ok(ToolOutput::error(
                "At least one keyword is required (or set always_on to true).",
            ));
        }
        if let Err(e) = validate_keywords(&keywords) {
            return Ok(ToolOutput::error(e));
        }
        if let Err(e) = validate_dependencies(&dependencies) {
            return Ok(ToolOutput::error(e));
        }

        let skills_dir = ctx.home_dir.join("skills");
        let skill_dir = skills_dir.join(name);

        if !skill_dir.exists() {
            return Ok(ToolOutput::error(format!(
                "Skill '{name}' does not exist. Use action='create' to create it.",
            )));
        }

        // Atomic write: write to temp dir, validate, swap
        let tmp_dir = skills_dir.join(format!(".{name}.tmp"));
        if tmp_dir.exists() {
            let _ = std::fs::remove_dir_all(&tmp_dir);
        }

        if let Err(e) = write_skill_files(
            &tmp_dir,
            name,
            description,
            system_prompt,
            always_on,
            &keywords,
            &dependencies,
        ) {
            let _ = std::fs::remove_dir_all(&tmp_dir);
            return Ok(ToolOutput::error(format!(
                "Failed to write skill files: {e}"
            )));
        }

        // Validate
        let diagnostics = validate_skill(&tmp_dir);
        let errors: Vec<_> = diagnostics
            .iter()
            .filter(|d| d.level == crate::skills::index::DiagnosticLevel::Fail)
            .collect();
        if !errors.is_empty() {
            let _ = std::fs::remove_dir_all(&tmp_dir);
            let msgs: Vec<String> = errors.iter().map(|d| d.message.clone()).collect();
            return Ok(ToolOutput::error(format!(
                "Skill validation failed:\n{}",
                msgs.join("\n")
            )));
        }

        // Remove old, rename new
        let _ = std::fs::remove_dir_all(&skill_dir);
        if let Err(e) = std::fs::rename(&tmp_dir, &skill_dir) {
            let _ = std::fs::remove_dir_all(&tmp_dir);
            return Ok(ToolOutput::error(format!(
                "Failed to finalize skill directory: {e}"
            )));
        }

        // Reset lifecycle_state to 'staged'
        let agent_id = &ctx.db.agent_id;
        if let Err(e) = ctx
            .db
            .with_db({
                let agent_id = agent_id.clone();
                let skill_name = name.to_string();
                move |db| {
                    db.conn.execute(
                        "INSERT INTO skill_overrides (agent_id, skill_name, lifecycle_state)
                         VALUES (?1, ?2, 'staged')
                         ON CONFLICT(agent_id, skill_name) DO UPDATE SET lifecycle_state = 'staged'",
                        rusqlite::params![agent_id, skill_name],
                    )?;
                    Ok(())
                }
            })
            .await
        {
            tracing::warn!(skill = name, error = %e, "failed to update skill_overrides row");
        }

        ctx.skills_dirty.store(true, Ordering::Release);

        let warnings: Vec<String> = diagnostics
            .iter()
            .filter(|d| d.level == crate::skills::index::DiagnosticLevel::Warn)
            .map(|d| d.message.clone())
            .collect();

        let mut result = json!({
            "status": "updated",
            "name": name,
            "lifecycle_state": "staged",
            "message": format!(
                "Skill '{name}' updated and reset to staged state. \
                 An operator must re-promote it before it activates."
            ),
        });
        if !warnings.is_empty() {
            result["validation_warnings"] = json!(warnings);
        }

        Ok(ToolOutput::success(
            serde_json::to_string_pretty(&result).unwrap_or_default(),
        ))
    }

    async fn handle_inspect(&self, name: &str, ctx: &ToolContext<'_>) -> Result<ToolOutput> {
        let skills_dir = ctx.home_dir.join("skills");
        let skill_dir = skills_dir.join(name);

        let exists = skill_dir.exists();

        // Get lifecycle state from DB
        let lifecycle_state = ctx
            .db
            .get_skill_lifecycle_state(&ctx.db.agent_id, name)
            .await
            .unwrap_or(None);

        if !exists && lifecycle_state.is_none() {
            return Ok(ToolOutput::error(format!(
                "Skill '{name}' not found on disk or in the database.",
            )));
        }

        let mut result = json!({
            "name": name,
            "lifecycle_state": lifecycle_state.as_deref().unwrap_or("none (bundled/marketplace)"),
            "exists_on_disk": exists,
        });

        // List files if directory exists
        if exists {
            let mut files = Vec::new();
            if let Ok(entries) = std::fs::read_dir(&skill_dir) {
                for entry in entries.flatten() {
                    if let Some(fname) = entry.file_name().to_str() {
                        files.push(fname.to_string());
                    }
                }
            }
            files.sort();
            result["files"] = json!(files);

            // Run validation
            let diagnostics = validate_skill(&skill_dir);
            let warnings: Vec<String> = diagnostics
                .iter()
                .filter(|d| d.level == crate::skills::index::DiagnosticLevel::Warn)
                .map(|d| d.message.clone())
                .collect();
            if !warnings.is_empty() {
                result["validation_warnings"] = json!(warnings);
            }
        }

        // Get last updated from file metadata
        if exists
            && let Ok(metadata) = std::fs::metadata(skill_dir.join("skill.toml"))
            && let Ok(modified) = metadata.modified()
            && let Ok(duration) = modified.duration_since(std::time::UNIX_EPOCH)
        {
            result["last_updated_unix"] = json!(duration.as_secs());
        }

        Ok(ToolOutput::success(
            serde_json::to_string_pretty(&result).unwrap_or_default(),
        ))
    }
}

fn extract_string_array(val: &Value) -> Vec<String> {
    val.as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str())
                .map(|s| s.to_string())
                .collect()
        })
        .unwrap_or_default()
}

fn write_skill_files(
    dir: &std::path::Path,
    name: &str,
    description: &str,
    system_prompt: &str,
    always_on: bool,
    keywords: &[String],
    dependencies: &[String],
) -> Result<()> {
    std::fs::create_dir_all(dir)?;

    // Write skill.toml
    let mut manifest = String::new();
    manifest.push_str("[skill]\n");
    manifest.push_str(&format!("name = \"{name}\"\n"));
    manifest.push_str(&format!("description = \"{}\"\n", escape_toml(description)));
    manifest.push_str("version = \"0.1.0\"\n");
    if always_on {
        manifest.push_str("always_on = true\n");
    }
    manifest.push('\n');

    if !keywords.is_empty() {
        manifest.push_str("[triggers]\n");
        manifest.push_str("keywords = [");
        let kw_strs: Vec<String> = keywords
            .iter()
            .map(|k| format!("\"{}\"", escape_toml(k)))
            .collect();
        manifest.push_str(&kw_strs.join(", "));
        manifest.push_str("]\n");
    }

    if !dependencies.is_empty() {
        manifest.push_str("\n[dependencies]\n");
        for dep in dependencies {
            manifest.push_str(&format!("{dep} = {{ source = \"sibling\" }}\n"));
        }
    }

    std::fs::write(dir.join("skill.toml"), manifest)?;

    // Write system_prompt.md
    std::fs::write(dir.join("system_prompt.md"), system_prompt)?;

    Ok(())
}

fn escape_toml(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t")
}
