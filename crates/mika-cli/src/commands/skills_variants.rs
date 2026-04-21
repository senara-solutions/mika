use std::collections::HashSet;
use std::path::Path;

use anyhow::{Result, bail};
use mika_agent::skills::index::sanitize_model_dir_name;
use mika_agent::skills::manifest::SkillManifest;
use mika_agent::skills::variants;

use crate::cli::{OutputFormat, SkillVariantsAction};

/// Parse a `provider/model` specifier, splitting on the first `/`.
fn parse_variant_spec(spec: &str) -> Result<(&str, &str)> {
    let (provider, model) = spec.split_once('/').ok_or_else(|| {
        anyhow::anyhow!(
            "Expected `provider/model`, got '{spec}'. Example: anthropic/claude-sonnet-4-6"
        )
    })?;
    if provider.is_empty() || model.is_empty() {
        bail!("Both provider and model are required (got '{spec}').");
    }
    Ok((provider, model))
}

/// Load a skill manifest from a skill directory.
fn load_manifest(skill_dir: &Path) -> Result<SkillManifest> {
    let manifest_path = skill_dir.join("skill.toml");
    if !manifest_path.exists() {
        bail!("skill.toml not found in {}", skill_dir.display());
    }
    let content = std::fs::read_to_string(&manifest_path)?;
    let manifest: SkillManifest = toml::from_str(&content)?;
    Ok(manifest)
}

pub fn run(action: SkillVariantsAction, skills_dir: &Path) -> Result<()> {
    match action {
        SkillVariantsAction::List { name, format } => run_list(skills_dir, &name, &format),
        SkillVariantsAction::Status { format } => run_status(skills_dir, &format),
        SkillVariantsAction::Diff {
            name,
            variant,
            source,
            format,
        } => run_diff(skills_dir, &name, &variant, source.as_deref(), &format),
        SkillVariantsAction::Reflect { name, format } => run_reflect(skills_dir, &name, &format),
        SkillVariantsAction::Validate { name, format } => {
            run_validate(skills_dir, name.as_deref(), &format)
        }
        SkillVariantsAction::Promote { name, variant } => run_promote(skills_dir, &name, &variant),
        SkillVariantsAction::Regen { name, variant } => run_regen(&name, &variant),
    }
}

fn run_list(skills_dir: &Path, name: &str, format: &OutputFormat) -> Result<()> {
    let skill_dir = skills_dir.join(name);
    if !skill_dir.is_dir() {
        bail!("Skill '{name}' not found at {}", skill_dir.display());
    }
    let manifest = load_manifest(&skill_dir)?;
    let all_variants = variants::scan_all_variants(&skill_dir, &manifest);

    match format {
        OutputFormat::Json => {
            println!("{}", serde_json::to_string_pretty(&all_variants)?);
        }
        OutputFormat::Text => {
            if all_variants.is_empty() {
                println!("\n  No variants found for skill '{name}'.\n");
                return Ok(());
            }
            println!(
                "\n  Variants for '{name}' ({} total):\n",
                all_variants.len()
            );
            println!(
                "  {:15} {:25} {:12} {:>8} {:>6} {:10} LAST MODIFIED",
                "PROVIDER", "MODEL", "SOURCE", "SIZE", "LINES", "STALE"
            );
            for v in &all_variants {
                let stale_str = match v.stale {
                    Some(true) => "yes",
                    Some(false) => "no",
                    None => "?",
                };
                let modified = v.last_modified.as_deref().unwrap_or("-");
                println!(
                    "  {:15} {:25} {:12} {:>7}B {:>5} {:10} {}",
                    v.provider,
                    v.model,
                    v.source.to_string(),
                    v.size_bytes,
                    v.line_count,
                    stale_str,
                    modified
                );
            }
            println!();
        }
    }
    Ok(())
}

fn run_status(skills_dir: &Path, format: &OutputFormat) -> Result<()> {
    if !skills_dir.is_dir() {
        match format {
            OutputFormat::Json => println!("[]"),
            OutputFormat::Text => {
                println!("\n  No skills directory found.\n");
            }
        }
        return Ok(());
    }

    let mut all_variants = Vec::new();
    if let Ok(rd) = std::fs::read_dir(skills_dir) {
        for entry in rd.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            if let Ok(manifest) = load_manifest(&path) {
                let mut skill_variants = variants::scan_all_variants(&path, &manifest);
                all_variants.append(&mut skill_variants);
            }
        }
    }

    let summaries = variants::summarize_variants(&all_variants);

    match format {
        OutputFormat::Json => {
            println!("{}", serde_json::to_string_pretty(&summaries)?);
        }
        OutputFormat::Text => {
            if summaries.is_empty() {
                println!("\n  No variants found across any skills.\n");
                return Ok(());
            }
            println!(
                "\n  Variant status ({} skills with variants):\n",
                summaries.len()
            );
            println!(
                "  {:25} {:>6} {:>6} {:>6} {:>8} {:>6}",
                "SKILL", "TOTAL", "HAND", "GEN", "EXPER.", "STALE"
            );
            for s in &summaries {
                println!(
                    "  {:25} {:>6} {:>6} {:>6} {:>8} {:>6}",
                    s.skill_name,
                    s.total,
                    s.hand_authored,
                    s.generated,
                    s.experimental,
                    s.stale_count
                );
            }
            println!();
        }
    }
    Ok(())
}

fn run_diff(
    skills_dir: &Path,
    name: &str,
    variant_spec: &str,
    source_filter: Option<&str>,
    format: &OutputFormat,
) -> Result<()> {
    let skill_dir = skills_dir.join(name);
    if !skill_dir.is_dir() {
        bail!("Skill '{name}' not found at {}", skill_dir.display());
    }
    let manifest = load_manifest(&skill_dir)?;

    let (provider, model) = parse_variant_spec(variant_spec)?;
    let sanitized_model = sanitize_model_dir_name(model);

    // Read base content
    let base_path = skill_dir.join("system_prompt.md");
    let base_content = std::fs::read_to_string(&base_path)
        .map_err(|e| anyhow::anyhow!("Cannot read base prompt: {e}"))?;

    // Find the variant to diff
    let variant_path = match source_filter {
        Some("experimental") => skill_dir
            .join("experimental")
            .join(provider)
            .join(&sanitized_model)
            .join("system_prompt.md"),
        Some("generated") => skill_dir
            .join("generated")
            .join(provider)
            .join(&sanitized_model)
            .join("system_prompt.md"),
        Some("hand-authored") => skill_dir
            .join(provider)
            .join(&sanitized_model)
            .join("system_prompt.md"),
        None => {
            // Default: runtime-resolved (hand-authored > generated)
            let hand = skill_dir
                .join(provider)
                .join(&sanitized_model)
                .join("system_prompt.md");
            if hand.exists() {
                hand
            } else {
                let generated = skill_dir
                    .join("generated")
                    .join(provider)
                    .join(&sanitized_model)
                    .join("system_prompt.md");
                if generated.exists() {
                    generated
                } else {
                    // Try experimental as last resort
                    skill_dir
                        .join("experimental")
                        .join(provider)
                        .join(&sanitized_model)
                        .join("system_prompt.md")
                }
            }
        }
        Some(other) => bail!(
            "Invalid --source '{other}'. Must be one of: hand-authored, generated, experimental"
        ),
    };

    if !variant_path.exists() {
        bail!("Variant not found at {}", variant_path.display());
    }

    let variant_content = std::fs::read_to_string(&variant_path)?;
    let report = variants::diff_variant(&base_content, &variant_content);

    match format {
        OutputFormat::Json => {
            println!("{}", serde_json::to_string_pretty(&report)?);
        }
        OutputFormat::Text => {
            println!("\n  Diff: {name} {provider}/{model}");
            println!("  Classification: {}", report.classification);
            println!("  Lines: +{} -{}", report.added_lines, report.removed_lines);
            if !report.missing_base_sections.is_empty() {
                println!(
                    "  Missing base sections: {}",
                    report.missing_base_sections.join(", ")
                );
            }
            if !report.unified_diff.is_empty() {
                println!();
                print!("{}", report.unified_diff);
            }
            println!();
            let _ = &manifest; // silence unused warning
        }
    }
    Ok(())
}

fn run_reflect(skills_dir: &Path, name: &str, format: &OutputFormat) -> Result<()> {
    let skill_dir = skills_dir.join(name);
    if !skill_dir.is_dir() {
        bail!("Skill '{name}' not found at {}", skill_dir.display());
    }
    let manifest = load_manifest(&skill_dir)?;
    let report = variants::reflect_skill(&skill_dir, &manifest)?;

    match format {
        OutputFormat::Json => {
            println!("{}", serde_json::to_string_pretty(&report)?);
        }
        OutputFormat::Text => {
            println!("\n  Reflection report for '{name}':");
            println!(
                "  Base last modified: {}",
                report.base_last_modified.as_deref().unwrap_or("unknown")
            );
            if report.impacts.is_empty() {
                println!("  No production variants found.\n");
                return Ok(());
            }
            println!("  {} variant(s) analyzed:\n", report.impacts.len());
            for impact in &report.impacts {
                let v = &impact.metadata;
                let stale = match v.stale {
                    Some(true) => " [STALE]",
                    _ => "",
                };
                println!("    {}/{} ({}){}", v.provider, v.model, v.source, stale);
                println!(
                    "      Classification: {}, +{} -{} lines",
                    impact.diff_summary.classification,
                    impact.diff_summary.added_lines,
                    impact.diff_summary.removed_lines
                );
                if !impact.diff_summary.missing_sections.is_empty() {
                    println!(
                        "      Missing sections: {}",
                        impact.diff_summary.missing_sections.join(", ")
                    );
                }
                println!("      Recommendation: {}", impact.recommendation);
            }
            println!();
        }
    }
    Ok(())
}

fn run_validate(skills_dir: &Path, name: Option<&str>, format: &OutputFormat) -> Result<()> {
    let dirs: Vec<(String, std::path::PathBuf)> = match name {
        Some(n) => {
            let skill_dir = skills_dir.join(n);
            if !skill_dir.is_dir() {
                bail!("Skill '{n}' not found at {}", skill_dir.display());
            }
            vec![(n.to_string(), skill_dir)]
        }
        None => {
            let mut found = Vec::new();
            if let Ok(rd) = std::fs::read_dir(skills_dir) {
                for entry in rd.flatten() {
                    let path = entry.path();
                    if path.is_dir()
                        && let Some(dir_name) = path.file_name().and_then(|n| n.to_str())
                    {
                        found.push((dir_name.to_string(), path));
                    }
                }
            }
            found.sort_by(|a, b| a.0.cmp(&b.0));
            found
        }
    };

    // CLI doesn't have the full tool registry, pass empty set
    let tool_names = HashSet::new();
    let mut all_json: Vec<serde_json::Value> = Vec::new();

    if matches!(format, OutputFormat::Text) {
        println!();
    }

    for (skill_name, skill_dir) in &dirs {
        let manifest = match load_manifest(skill_dir) {
            Ok(m) => m,
            Err(e) => {
                match format {
                    OutputFormat::Text => {
                        println!("  {skill_name}/");
                        println!("    [FAIL] Cannot load manifest: {e}");
                    }
                    OutputFormat::Json => {
                        all_json.push(serde_json::json!({
                            "skill": skill_name,
                            "error": format!("{e}"),
                        }));
                    }
                }
                continue;
            }
        };

        let all_variants = variants::scan_all_variants(skill_dir, &manifest);
        if all_variants.is_empty() {
            continue;
        }

        let base_content = std::fs::read_to_string(skill_dir.join("system_prompt.md")).ok();

        if matches!(format, OutputFormat::Text) {
            println!("  {skill_name}/");
        }

        for v in &all_variants {
            let content = match std::fs::read_to_string(&v.file_path) {
                Ok(c) => c,
                Err(_) => continue,
            };
            let results = variants::validate_variant(
                &content,
                base_content.as_deref(),
                &manifest,
                &tool_names,
            );
            let diags = variants::to_diagnostics(&results);

            match format {
                OutputFormat::Text => {
                    println!("    {}/{} ({}):", v.provider, v.model, v.source);
                    for d in &diags {
                        println!("      {} {}", d.tag(), d.message);
                    }
                }
                OutputFormat::Json => {
                    let rule_results: Vec<serde_json::Value> = results
                        .iter()
                        .map(|r| {
                            serde_json::json!({
                                "rule": r.rule,
                                "passed": r.passed,
                                "message": r.message,
                                "level": r.level,
                            })
                        })
                        .collect();
                    all_json.push(serde_json::json!({
                        "skill": skill_name,
                        "provider": v.provider,
                        "model": v.model,
                        "source": v.source,
                        "results": rule_results,
                    }));
                }
            }
        }
    }

    match format {
        OutputFormat::Json => {
            println!("{}", serde_json::to_string_pretty(&all_json)?);
        }
        OutputFormat::Text => println!(),
    }

    Ok(())
}

fn run_promote(skills_dir: &Path, name: &str, variant_spec: &str) -> Result<()> {
    let skill_dir = skills_dir.join(name);
    if !skill_dir.is_dir() {
        bail!("Skill '{name}' not found at {}", skill_dir.display());
    }
    let manifest = load_manifest(&skill_dir)?;

    let (provider, model) = parse_variant_spec(variant_spec)?;

    let base_content = std::fs::read_to_string(skill_dir.join("system_prompt.md")).ok();
    let tool_names = HashSet::new();

    let result = variants::promote_variant(
        &skill_dir,
        &manifest,
        provider,
        model,
        base_content.as_deref(),
        &tool_names,
    )?;

    println!("\n  Promoted variant:");
    println!("    From: {}", result.source_path.display());
    println!("    To:   {}", result.target_path.display());
    for w in &result.warnings {
        println!("    [WARN] {w}");
    }
    println!();
    Ok(())
}

fn run_regen(name: &str, variant_spec: &str) -> Result<()> {
    let (provider, model) = parse_variant_spec(variant_spec)?;

    println!("\n  To regenerate the variant for '{name}' targeting {provider}/{model}, run:\n");
    println!(
        "    mika ask --enable-skill skill-review \"review and generate variant for {name} targeting {provider}/{model}\"\n"
    );
    Ok(())
}
