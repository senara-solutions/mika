//! Regression gate for bundled-skill startup loading.
//!
//! WHY this test exists: on the 2026-04-26 deploy, three bundled skills were
//! silently skipped at startup with `oversized prompt` errors (self-dev,
//! qa-review, qa-review-build-callback). mika-dev came up without `self-dev`
//! and mika-qa came up without `qa-review` — the autonomous loop ran in a
//! degraded state that was discoverable only by reading log warnings on the
//! host. Prior fixes ("monitor `wc -c` after every prompt edit",
//! `max_prompt_size = 32768` on the offending skill) demonstrably did not
//! hold across prompt growth.
//!
//! This test converts that silent-drop behaviour into a CI failure: if any
//! bundled skill fails to load, `cargo test` fails and names the offending
//! skill, its declared/default cap, and its actual prompt size.
//!
//! Scope boundary (intentional): this test covers `skills/bundled/`, the
//! engine-coupled bundled skills compiled into mika-agent at build time. It
//! does NOT cover marketplace skills loaded from the `mika-skills` repo at
//! runtime — those enter mika via a separate scan path. Unit 3's audit (per
//! `docs/plans/2026-04-27-001-feat-evaluate-mika-defaults-for-anthropic-plan.md`)
//! decides whether a parallel marketplace-side check is needed.
//!
//! Drives through the same `scan_skills_dir()` entry point production uses,
//! not internal helpers — refactors that change the entry point should
//! refactor this test alongside them. That coupling is the point.

use std::fs;
use std::path::PathBuf;

use mika_agent::skills::index::{
    MAX_PROMPT_SIZE_CEILING, MAX_PROMPT_SNIPPET_SIZE, scan_skills_dir,
};
use mika_agent::skills::manifest::SkillManifest;

fn bundled_skills_dir() -> PathBuf {
    // CARGO_MANIFEST_DIR = .../mika/crates/mika-agent
    // skills/bundled/    = .../mika/skills/bundled
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("skills")
        .join("bundled")
}

#[test]
fn bundled_skills_load_without_oversized_prompts() {
    let skills_dir = bundled_skills_dir();
    assert!(
        skills_dir.is_dir(),
        "bundled-skills directory not found at {} — repo layout changed?",
        skills_dir.display()
    );

    let scan = scan_skills_dir(&skills_dir);

    if !scan.skipped.is_empty() {
        let mut report = String::from(
            "\nBundled-skill scan dropped one or more skills at startup. \
             Each skipped skill below is invisible at runtime — the agent \
             will boot in a silently-degraded state until the cause is fixed.\n\n\
             Per-skill remediation:\n  - 'oversized prompt': raise `max_prompt_size` \
             in the skill's `skill.toml` (ceiling 80KB), or trim the prompt.\n  - other \
             reasons: see the skill's manifest, prompt file, and `scan_skills_dir` \
             error variants.\n\nDropped skills:\n",
        );
        for skipped in &scan.skipped {
            report.push_str(&format!("  - {}: {}\n", skipped.name, skipped.reason));
        }
        panic!("{}", report);
    }

    // Defensive sanity check: production has 22 bundled engine-coupled skills as
    // of this regression's introduction. If the count drops to zero, the test
    // is pointing at the wrong tree and `skipped.is_empty()` is meaningless.
    assert!(
        !scan.entries.is_empty(),
        "scan_skills_dir({}) returned zero entries — wrong path or empty tree?",
        skills_dir.display()
    );
}

/// mika#852 — Warn gate at 95% of each skill's effective `max_prompt_size`.
///
/// Companion to `bundled_skills_load_without_oversized_prompts` (the 100%
/// fail gate from mika#828). This test fires *before* a skill hits the
/// hard-skip cliff, giving operators time to raise the cap, trim the prompt,
/// or shard via `[dependencies]`.
///
/// Walks `skills/bundled/` directly (not via `scan_skills_dir`, which only
/// reports cap violations on the failure side) and checks each prompt file
/// against its effective cap.
#[test]
fn bundled_skills_approaching_max_prompt_size_warns() {
    let skills_dir = bundled_skills_dir();
    assert!(
        skills_dir.is_dir(),
        "bundled-skills directory not found at {} — repo layout changed?",
        skills_dir.display()
    );

    let mut near_cap: Vec<(String, u64, u64, f64)> = Vec::new();

    for entry in fs::read_dir(&skills_dir).expect("failed to read skills/bundled/") {
        let entry = entry.expect("failed to read dir entry");
        let path = entry.path();

        // Skip non-directories and underscore-prefixed support dirs (e.g. _shared/)
        if !path.is_dir() {
            continue;
        }
        let dir_name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default();
        if dir_name.starts_with('_') || dir_name.starts_with('.') {
            continue;
        }

        let manifest_path = path.join("skill.toml");
        let prompt_path = path.join("system_prompt.md");

        // Skip skills without a prompt file (tool-only skills).
        if !prompt_path.exists() {
            continue;
        }

        // Parse the manifest to get the per-skill max_prompt_size override.
        let manifest_max = if manifest_path.exists() {
            let toml_str = fs::read_to_string(&manifest_path).expect("failed to read skill.toml");
            let manifest: SkillManifest =
                toml::from_str(&toml_str).expect("failed to parse skill.toml");
            manifest.skill.max_prompt_size
        } else {
            None
        };

        let effective_cap = manifest_max
            .unwrap_or(MAX_PROMPT_SNIPPET_SIZE)
            .min(MAX_PROMPT_SIZE_CEILING);

        let actual = fs::metadata(&prompt_path)
            .unwrap_or_else(|e| panic!("failed to stat {}: {}", prompt_path.display(), e))
            .len();

        let ratio = actual as f64 / effective_cap as f64;
        if ratio >= 0.95 {
            near_cap.push((dir_name.to_string(), actual, effective_cap, ratio));
        }
    }

    if !near_cap.is_empty() {
        let mut report = String::from(
            "\nBundled-skill prompt approaching cap (≥95% of effective max_prompt_size):\n\n",
        );
        for (name, actual, cap, pct) in &near_cap {
            report.push_str(&format!(
                "  - {name}: {actual} bytes / {cap} cap ({pct:.1}%)\n",
                pct = pct * 100.0,
            ));
        }
        report.push_str(
            "\nRemediation options:\n\
             \x20 - Raise max_prompt_size in the skill's skill.toml toward the 80 KB ceiling.\n\
             \x20 - Trim the prompt.\n\
             \x20 - Shard the prompt via [dependencies] (extract a sibling skill).\n\n\
             This warn-gate (mika#852) defends against silent skill-skip at the 100% cliff\n\
             (see bundled_skills_load_without_oversized_prompts for the failure case).\n",
        );
        panic!("{}", report);
    }
}
