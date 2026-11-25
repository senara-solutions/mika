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

use mika_agent::skills::index::{effective_prompt_limit, scan_skills_dir};
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

        let effective_cap = effective_prompt_limit(manifest_max);

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

/// The margin a skill has left under the mika#852 gauge, in bytes.
///
/// Same arithmetic as the gauge above — `0.95 × effective_prompt_limit` — so a
/// floor expressed in these units is literally "measured by the gauge test"
/// (mika#3166 AC1) rather than by a second, drifting notion of margin.
fn gauge_margin_bytes(skill: &str) -> (u64, u64, u64) {
    let dir = bundled_skills_dir().join(skill);
    let manifest_path = dir.join("skill.toml");
    let prompt_path = dir.join("system_prompt.md");

    let toml_str = fs::read_to_string(&manifest_path)
        .unwrap_or_else(|e| panic!("failed to read {}: {e}", manifest_path.display()));
    let manifest: SkillManifest = toml::from_str(&toml_str)
        .unwrap_or_else(|e| panic!("failed to parse {}: {e}", manifest_path.display()));

    let effective_cap = effective_prompt_limit(manifest.skill.max_prompt_size);
    let gate = (effective_cap as f64 * 0.95).floor() as u64;
    let actual = fs::metadata(&prompt_path)
        .unwrap_or_else(|e| panic!("failed to stat {}: {e}", prompt_path.display()))
        .len();

    (actual, gate, gate.saturating_sub(actual))
}

/// The margin #3164 had to pay for, and the floor it names (mika#3166 AC1).
///
/// 1 296 bytes is not a round number: it is the exact weight of the two
/// Data-Integrity rules PR #3164 **removed** from `qa-review/system_prompt.md`
/// because the prompt had 42 bytes left under the gauge — the CI-abstention rule
/// and the canonical `CI-Status:` line of mika#3137. That is the first
/// measurement of what the gauge costs in *behaviour* rather than in comfort,
/// so it is the floor worth holding: below it, the next rule that has to reach
/// mika-qa cannot.
const QA_REVIEW_MARGIN_FLOOR_BYTES: u64 = 1_296;

/// # Why this is named on `qa-review` and not asserted class-wide
///
/// Not timidity — a class-wide floor would redden on a prompt this flight does
/// not touch. `self-dev` carries 68 696 bytes against a 73 728 cap, i.e. 1 345
/// bytes of margin: **49 bytes** above this floor. A class-wide assertion would
/// therefore make mika#3166's gate depend on `self-dev`'s next edit, and the
/// natural response to that red — raising `self-dev`'s floor exemption — is how
/// an allowlist is born (mika#2309). `self-dev` deserves its own shard ticket
/// with its own seam analysis; it does not deserve to be dragged into this one.
///
/// # What reddens it
///
/// Any addition to `qa-review/system_prompt.md` that eats the margin mika#3166
/// bought back. The message says what to do, and the order matters: the third
/// remediation is now *primed* — `qa-review-dependabot` exists, the outgoing
/// edge is declared, the scope-header idiom is established and the reachability
/// detector is in place — so sharding is no longer the expensive option it was
/// when `skill.toml` declined it twice.
#[test]
fn mika3166_le_prompt_qa_review_garde_la_marge_que_3164_a_payee() {
    let (actual, gate, margin) = gauge_margin_bytes("qa-review");

    assert!(
        margin >= QA_REVIEW_MARGIN_FLOOR_BYTES,
        "\nqa-review/system_prompt.md has {margin} bytes of margin under the \
         mika#852 gauge ({actual} bytes against a gate at {gate}), below the \
         {QA_REVIEW_MARGIN_FLOOR_BYTES}-byte floor of mika#3166.\n\n\
         {QA_REVIEW_MARGIN_FLOOR_BYTES} bytes is the weight of the two \
         Data-Integrity rules PR #3164 had to REMOVE from this prompt for lack \
         of room (CI abstention + the canonical `CI-Status:` line, mika#3137). \
         A margin below that means the next rule that must reach mika-qa \
         cannot, and the engine guard is left alone to carry it.\n\n\
         Remediation, in order:\n\
         \x20 - Shard the addition into a sibling skill. This is now the cheap \
         option: `qa-review-dependabot` (mika#3166) is the worked example — \
         outgoing edge in `qa-review/skill.toml`, scope header at the top of \
         the shard's prompt, identity allowlist entry, reachability detector in \
         `src/bundled_skills.rs`.\n\
         \x20 - Trim, and say in the commit what the removed text was holding.\n\
         \x20 - Raising `max_prompt_size` is NOT an option: it is already at \
         `MAX_PROMPT_SIZE_CEILING` (81 920), which `effective_prompt_limit` \
         applies through a `.min()`.\n\n\
         Lowering this floor is not a remediation — it is deleting the \
         measurement that says the gauge has started costing behaviour."
    );
}
