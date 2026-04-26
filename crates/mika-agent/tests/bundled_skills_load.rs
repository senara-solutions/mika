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

use std::path::PathBuf;

use mika_agent::skills::index::scan_skills_dir;

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
             in the skill's `skill.toml` (ceiling 64KB), or trim the prompt.\n  - other \
             reasons: see the skill's manifest, prompt file, and `scan_skills_dir` \
             error variants.\n\nDropped skills:\n",
        );
        for skipped in &scan.skipped {
            report.push_str(&format!("  - {}: {}\n", skipped.name, skipped.reason));
        }
        panic!("{}", report);
    }

    // Defensive sanity check: production has 17 bundled engine-coupled skills as
    // of this regression's introduction. If the count drops to zero, the test
    // is pointing at the wrong tree and `skipped.is_empty()` is meaningless.
    assert!(
        !scan.entries.is_empty(),
        "scan_skills_dir({}) returned zero entries — wrong path or empty tree?",
        skills_dir.display()
    );
}
