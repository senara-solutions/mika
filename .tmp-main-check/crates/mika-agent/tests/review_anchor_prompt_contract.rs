//! The review-anchor contract must be declared *and* taught (mika#2037).
//!
//! The engine guard is only half the fix. The other half is that the prompt stopped teaching
//! the defect: `mika-arch-groom-ticket/system_prompt.md` said in so many words that a READY
//! "may stay short since no iteration is needed", and its worked READY example was literally
//! the shape of the 302-byte acknowledgement measured in mika#2037. A model following that
//! prompt would trip the guard on every clean plan and burn a corrective re-prompt learning
//! what the prompt could have told it.
//!
//! These are literal-text assertions on purpose. The manifest declaration is checked by
//! `make verify-bundled-skills`; what no other test covers is that the prose and the guard
//! still agree.

use std::path::{Path, PathBuf};

fn bundled(skill: &str, file: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../skills/bundled")
        .join(skill)
        .join(file)
}

fn read(skill: &str, file: &str) -> String {
    let path = bundled(skill, file);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("reading {}: {e}", path.display()))
}

const VERDICT_PRODUCERS: &[&str] = &[
    "mika-arch-groom-ticket",
    "mika-arch-second-review",
    "mika-arch-groom-milestone",
];

#[test]
fn every_verdict_producer_declares_the_anchor_contract() {
    for skill in VERDICT_PRODUCERS {
        let toml = read(skill, "skill.toml");
        assert!(
            toml.contains("required_review_anchor_prefixes"),
            "{skill} declares no review-anchor contract — its non-terminal disposition is \
             still unguarded (mika#2037)"
        );
        assert!(
            toml.contains("review_anchor_min_count = 3"),
            "{skill} must require 3 anchors; one is crossable by copying a single line"
        );
        assert!(
            toml.contains("review_anchor_min_quote_chars = 40"),
            "{skill} must require a 40-character verbatim quote"
        );
    }
}

#[test]
fn every_verdict_producer_teaches_the_anchor_contract() {
    for skill in VERDICT_PRODUCERS {
        let prompt = read(skill, "system_prompt.md");
        assert!(
            prompt.contains("Review-Anchor Attestation Contract"),
            "{skill}'s prompt does not describe the contract its manifest enforces"
        );
        // mika#2338 — the second failure is no longer a silent withhold: the engine rewrites
        // the disposition into a terminal ESCALATE carrying a `[mika-engine]` finding line
        // that names the cause. The prompt must describe that outcome, not the retired
        // marker, or the model is taught a failure shape the engine no longer produces.
        assert!(
            prompt.contains("[mika-engine]") && prompt.contains("ESCALATE"),
            "{skill}'s prompt must describe the engine's ESCALATE-with-cause outcome, so the \
             model knows the failure is terminal, visible, and attributed to the engine"
        );
        assert!(
            !prompt.contains("replaced with `Disposition-Withheld"),
            "{skill}'s prompt still teaches the retired withheld-marker outcome (mika#2338)"
        );
    }
}

/// The three citation rules the comparator actually checks (mika#2338): the source of the
/// quote is the brief message of this turn, the quote lives on the anchor line, and the words
/// must be exact while inline markup is tolerated. A prompt that omits any of them teaches a
/// failure the engine will refuse.
#[test]
fn every_verdict_producer_teaches_the_three_citation_rules() {
    for skill in VERDICT_PRODUCERS {
        let prompt = read(skill, "system_prompt.md");
        assert!(
            prompt.contains("brief message of this turn"),
            "{skill}'s prompt must say the quote is compared against the brief message of \
             this turn — not a file read with a tool, not an earlier ticket in the session"
        );
        assert!(
            prompt.contains("the anchor line itself is compared"),
            "{skill}'s prompt must say a quote that wraps onto the next line is truncated at \
             the anchor line"
        );
        assert!(
            prompt.contains("markup") && prompt.contains("tolerated"),
            "{skill}'s prompt must say inline markup is tolerated while the words must be exact"
        );
    }
}

/// The milestone prompt used to tell the architect to quote "the sub-issue plans you actually
/// read". The milestone brief carries each sub-issue's key decisions, not the plans' text, and
/// a plan read through `gh_read` is never in the text the comparator holds — so that sentence
/// prescribed exactly the failure the guard refuses (mika#2338).
#[test]
fn the_milestone_prompt_no_longer_points_anchors_at_plans_read_by_tool() {
    let milestone = read("mika-arch-groom-milestone", "system_prompt.md");
    assert!(
        !milestone.contains("quote the sub-issue plans you actually read"),
        "mika-arch-groom-milestone still tells the model to quote plan files it read by tool; \
         those are never in the brief the engine compares against"
    );
}

/// The founding prescription: the prompt used to authorize exactly the response mika#2037
/// measured. If this sentence ever comes back, the guard starts fighting the prompt.
#[test]
fn the_brevity_prescription_is_gone() {
    let groom = read("mika-arch-groom-ticket", "system_prompt.md");
    assert!(
        !groom.contains("the message may stay short since no iteration is needed"),
        "mika-arch-groom-ticket still tells the model a READY may be a short acknowledgement — \
         that sentence is what mika#2037 measured being followed"
    );

    let second = read("mika-arch-second-review", "system_prompt.md");
    assert!(
        !second.contains("the message may stay short since no iteration is needed"),
        "mika-arch-second-review still authorizes a thin GROOMED"
    );
}

/// The worked READY example must satisfy the contract it illustrates. An example the guard
/// would reject teaches the model to fail.
#[test]
fn the_worked_ready_example_carries_three_anchors() {
    let groom = read("mika-arch-groom-ticket", "system_prompt.md");
    let example = groom
        .split("#### Disposition: READY example")
        .nth(1)
        .expect("groom-ticket must keep a worked READY example");
    // Bound the slice to the example's own fenced block.
    let block = example
        .split("```")
        .nth(1)
        .expect("the READY example must be a fenced block");

    for prefix in ["A1:", "A2:", "A3:"] {
        assert!(
            block.contains(prefix),
            "the worked READY example is missing {prefix} — it would be rejected by the guard \
             it is meant to demonstrate"
        );
    }
    assert!(
        block.contains("Disposition: READY"),
        "the worked READY example must still end on its disposition"
    );
}
