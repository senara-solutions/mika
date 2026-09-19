//! AC8 grep-discipline test (mika#1733).
//!
//! Under `Strict` authority — the shipped default per AC8 — `override_used`
//! must NEVER be recorded as `true`. This test walks `crates/mika-agent/src`
//! (and via companion tests, `crates/mika-common/src`) and asserts the
//! literal token `override_used = true` (with either spacing) appears only
//! in comments or test scopes. The production emit path must derive the flag
//! from `(classifier_verdict, decision, authority)` and MUST NOT hard-set it.
//!
//! Scope: source files under `crates/mika-agent/src/**`. Test file itself
//! excluded (the phrase in the assertion message would false-positive).
//!
//! **The production/test boundary is read by [`mika_common::source_guard`]
//! (mika#2398).** This scan used to track it with a line-by-line brace counter
//! seeded on `line.contains("#[cfg(test)]")`. Its error went **both ways** and
//! neither was visible: a JSON literal in a test fixture drifts the counter, so
//! the scan could end a test module early (false positive, loud) or never end
//! it (blindness, silent) depending on which way the braces fell. Counting
//! braces is the one thing the shared reader deliberately does not do — these
//! `mod tests` blocks are full of JSON.

use std::path::Path;

fn offending_lines(path: &Path, production: &str) -> Vec<String> {
    let needle_spaced = "override_used = true";
    let needle_tight = "override_used=true";
    production
        .lines()
        .enumerate()
        .filter_map(|(idx, line)| {
            let trimmed = line.trim_start();
            if trimmed.starts_with("//") || trimmed.starts_with('*') {
                return None;
            }
            if !(line.contains(needle_spaced) || line.contains(needle_tight)) {
                return None;
            }
            Some(format!("{}:{}: {trimmed}", path.display(), idx + 1))
        })
        .collect()
}

#[test]
fn override_used_true_only_in_tests_and_comments() {
    let scanner =
        mika_common::source_guard::ProductionScanner::for_crate(env!("CARGO_MANIFEST_DIR"));
    let mut offenders = Vec::new();
    scanner.for_each(|path, production| {
        offenders.extend(offending_lines(path, production));
    });
    assert!(
        offenders.is_empty(),
        "AC8 grep discipline (mika#1733): the literal token `override_used = true` \
         (either spacing) must only appear inside comments or test scopes. \
         Production emit paths must derive the flag from \
         (classifier_verdict, decision, authority). Offenders:\n{}",
        offenders.join("\n")
    );
}

/// Good-faith control: the scan must be able to see the token at all.
///
/// Without it, a boundary rule that masked the whole tree would leave the
/// assertion above trivially green — which is the exact failure mode this file
/// exists to make visible on the other axis.
#[test]
fn the_scan_detects_the_token_it_forbids() {
    let fixture = "    permission_decisions.override_used = true;\n";
    assert_eq!(
        offending_lines(Path::new("fixture.rs"), fixture).len(),
        1,
        "the scan no longer detects the token it forbids"
    );
    assert!(
        offending_lines(Path::new("fixture.rs"), "    // override_used = true\n").is_empty(),
        "a comment describing the rule is not a violation of it"
    );
}
