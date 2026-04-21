//! HTTP handlers for variant management endpoints.

use std::collections::HashSet;
use std::sync::atomic::Ordering;

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use serde_json::json;

use crate::skills::variants;

use super::state::AppState;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Find a skill directory by name from the first agent's skill registry.
fn find_skill_dir(
    state: &AppState,
    skill_name: &str,
) -> Result<
    (std::path::PathBuf, crate::skills::manifest::SkillManifest),
    (StatusCode, Json<serde_json::Value>),
> {
    let agent_state = state.agents.values().next().ok_or_else(|| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": "no agents configured"})),
        )
    })?;

    let registry = agent_state.skills.lock().unwrap().clone();
    let entry = registry
        .skills()
        .iter()
        .find(|s| s.manifest.skill.name.eq_ignore_ascii_case(skill_name))
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                Json(json!({"error": format!("skill '{skill_name}' not found")})),
            )
        })?;

    Ok((entry.dir.clone(), entry.manifest.clone()))
}

// ---------------------------------------------------------------------------
// GET /api/v1/skills/variants — cross-skill summary
// ---------------------------------------------------------------------------

pub async fn handle_variants_summary(State(state): State<AppState>) -> impl IntoResponse {
    let agent_state = match state.agents.values().next() {
        Some(a) => a,
        None => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": "no agents configured"})),
            )
                .into_response();
        }
    };

    let registry = agent_state.skills.lock().unwrap().clone();
    let mut all_variants = Vec::new();

    for entry in registry.skills() {
        let mut skill_variants = variants::scan_all_variants(&entry.dir, &entry.manifest);
        all_variants.append(&mut skill_variants);
    }

    let summaries = variants::summarize_variants(&all_variants);
    Json(summaries).into_response()
}

// ---------------------------------------------------------------------------
// GET /api/v1/skills/{skill}/variants — per-skill list
// ---------------------------------------------------------------------------

pub async fn handle_skill_variants(
    State(state): State<AppState>,
    Path(skill_name): Path<String>,
) -> impl IntoResponse {
    let (skill_dir, manifest) = match find_skill_dir(&state, &skill_name) {
        Ok(v) => v,
        Err(e) => return e.into_response(),
    };

    let all_variants = variants::scan_all_variants(&skill_dir, &manifest);
    Json(all_variants).into_response()
}

// ---------------------------------------------------------------------------
// GET /api/v1/skills/{skill}/variants/reflect — audit report
// ---------------------------------------------------------------------------

pub async fn handle_variant_reflect(
    State(state): State<AppState>,
    Path(skill_name): Path<String>,
) -> impl IntoResponse {
    let (skill_dir, manifest) = match find_skill_dir(&state, &skill_name) {
        Ok(v) => v,
        Err(e) => return e.into_response(),
    };

    match variants::reflect_skill(&skill_dir, &manifest) {
        Ok(report) => Json(report).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": format!("{e}")})),
        )
            .into_response(),
    }
}

// ---------------------------------------------------------------------------
// GET /api/v1/skills/{skill}/variants/{provider}/{model} — content + metadata
// ---------------------------------------------------------------------------

pub async fn handle_variant_detail(
    State(state): State<AppState>,
    Path((skill_name, provider, model)): Path<(String, String, String)>,
) -> impl IntoResponse {
    let (skill_dir, manifest) = match find_skill_dir(&state, &skill_name) {
        Ok(v) => v,
        Err(e) => return e.into_response(),
    };

    let all_variants = variants::scan_all_variants(&skill_dir, &manifest);
    let key = format!("{provider}/{model}");

    // Find the runtime-resolved variant (hand-authored > generated > experimental)
    let variant = all_variants.iter().find(|v| {
        let vkey = format!("{}/{}", v.provider, v.model);
        vkey == key
    });

    match variant {
        Some(v) => {
            let content = std::fs::read_to_string(&v.file_path).unwrap_or_default();
            Json(json!({
                "metadata": v,
                "content": content,
            }))
            .into_response()
        }
        None => (
            StatusCode::NOT_FOUND,
            Json(json!({"error": format!("variant '{key}' not found")})),
        )
            .into_response(),
    }
}

// ---------------------------------------------------------------------------
// GET /api/v1/skills/{skill}/variants/{provider}/{model}/diff
// ---------------------------------------------------------------------------

pub async fn handle_variant_diff(
    State(state): State<AppState>,
    Path((skill_name, provider, model)): Path<(String, String, String)>,
) -> impl IntoResponse {
    let (skill_dir, manifest) = match find_skill_dir(&state, &skill_name) {
        Ok(v) => v,
        Err(e) => return e.into_response(),
    };

    let base_path = skill_dir.join("system_prompt.md");
    let base_content = match std::fs::read_to_string(&base_path) {
        Ok(c) => c,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": format!("cannot read base prompt: {e}")})),
            )
                .into_response();
        }
    };

    let sanitized = crate::skills::index::sanitize_model_dir_name(&model);
    // Try hand-authored > generated > experimental
    let variant_path = {
        let hand = skill_dir
            .join(&provider)
            .join(&sanitized)
            .join("system_prompt.md");
        if hand.exists() {
            hand
        } else {
            let generated = skill_dir
                .join("generated")
                .join(&provider)
                .join(&sanitized)
                .join("system_prompt.md");
            if generated.exists() {
                generated
            } else {
                skill_dir
                    .join("experimental")
                    .join(&provider)
                    .join(&sanitized)
                    .join("system_prompt.md")
            }
        }
    };

    if !variant_path.exists() {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({"error": format!("variant '{provider}/{model}' not found")})),
        )
            .into_response();
    }

    let variant_content = match std::fs::read_to_string(&variant_path) {
        Ok(c) => c,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": format!("cannot read variant: {e}")})),
            )
                .into_response();
        }
    };

    let report = variants::diff_variant(&base_content, &variant_content);
    let _ = manifest; // silence unused
    Json(report).into_response()
}

// ---------------------------------------------------------------------------
// POST /api/v1/skills/{skill}/variants/{provider}/{model}/validate
// ---------------------------------------------------------------------------

pub async fn handle_variant_validate(
    State(state): State<AppState>,
    Path((skill_name, provider, model)): Path<(String, String, String)>,
) -> impl IntoResponse {
    let (skill_dir, manifest) = match find_skill_dir(&state, &skill_name) {
        Ok(v) => v,
        Err(e) => return e.into_response(),
    };

    let sanitized = crate::skills::index::sanitize_model_dir_name(&model);
    let all_variants = variants::scan_all_variants(&skill_dir, &manifest);
    let key = format!("{provider}/{sanitized}");

    let variant = all_variants.iter().find(|v| {
        let vkey = format!("{}/{}", v.provider, v.model);
        vkey == key
    });

    let variant = match variant {
        Some(v) => v,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(json!({"error": format!("variant '{provider}/{model}' not found")})),
            )
                .into_response();
        }
    };

    let content = match std::fs::read_to_string(&variant.file_path) {
        Ok(c) => c,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": format!("cannot read variant: {e}")})),
            )
                .into_response();
        }
    };

    let base_content = std::fs::read_to_string(skill_dir.join("system_prompt.md")).ok();
    let tool_names = HashSet::new();

    let results =
        variants::validate_variant(&content, base_content.as_deref(), &manifest, &tool_names);
    let passed = !variants::has_failures(&results);

    Json(json!({
        "skill": skill_name,
        "provider": provider,
        "model": model,
        "passed": passed,
        "results": results,
    }))
    .into_response()
}

// ---------------------------------------------------------------------------
// POST /api/v1/skills/{skill}/variants/{provider}/{model}/promote
// ---------------------------------------------------------------------------

pub async fn handle_variant_promote(
    State(state): State<AppState>,
    Path((skill_name, provider, model)): Path<(String, String, String)>,
) -> impl IntoResponse {
    let agent_state = match state.agents.values().next() {
        Some(a) => a.clone(),
        None => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": "no agents configured"})),
            )
                .into_response();
        }
    };

    let (skill_dir, manifest) = match find_skill_dir(&state, &skill_name) {
        Ok(v) => v,
        Err(e) => return e.into_response(),
    };

    let base_content = std::fs::read_to_string(skill_dir.join("system_prompt.md")).ok();
    let tool_names = HashSet::new();

    match variants::promote_variant(
        &skill_dir,
        &manifest,
        &provider,
        &model,
        base_content.as_deref(),
        &tool_names,
    ) {
        Ok(result) => {
            // Signal that the skill registry needs a refresh
            agent_state.skills_dirty.store(true, Ordering::Relaxed);

            Json(json!({
                "promoted": true,
                "source": result.source_path.display().to_string(),
                "target": result.target_path.display().to_string(),
                "warnings": result.warnings,
            }))
            .into_response()
        }
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": format!("{e}")})),
        )
            .into_response(),
    }
}
