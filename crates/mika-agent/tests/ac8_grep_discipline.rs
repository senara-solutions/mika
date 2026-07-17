//! AC8 grep-discipline test (mika#1733).
//!
//! Under `Strict` authority — the shipped default per AC8 — `override_used`
//! must NEVER be recorded as `true`. This test walks `crates/mika-agent/src`
//! (and via companion tests, `crates/mika-common/src`) and asserts the
//! literal token `override_used = true` (with either spacing) appears only
//! in comments or `#[cfg(test)]` scopes. The production emit path must
//! derive the flag from `(classifier_verdict, decision, authority)` and MUST
//! NOT hard-set it.
//!
//! Scope: source files under `crates/mika-agent/src/**`. Test file itself
//! excluded (the phrase in the assertion message would false-positive).

use std::fs;
use std::path::Path;

fn scan_dir(dir: &Path, offenders: &mut Vec<String>) {
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            scan_dir(&path, offenders);
            continue;
        }
        if path.extension().and_then(|e| e.to_str()) != Some("rs") {
            continue;
        }
        let Ok(content) = fs::read_to_string(&path) else {
            continue;
        };
        let mut in_test_module = false;
        let mut brace_depth_at_test_start: i32 = 0;
        let mut brace_depth: i32 = 0;
        for (idx, raw_line) in content.lines().enumerate() {
            let line = raw_line;
            // Track cfg(test) module entry — coarse but sufficient for our
            // single-crate scan.
            if line.contains("#[cfg(test)]") || line.contains("#[cfg(any(test,") {
                in_test_module = true;
                brace_depth_at_test_start = brace_depth;
            }
            let opens = line.matches('{').count() as i32;
            let closes = line.matches('}').count() as i32;
            brace_depth += opens - closes;
            if in_test_module && brace_depth <= brace_depth_at_test_start {
                in_test_module = false;
            }

            let trimmed = line.trim_start();
            if trimmed.starts_with("//") || trimmed.starts_with("*") {
                continue;
            }
            let needle_spaced = "override_used = true";
            let needle_tight = "override_used=true";
            if !(line.contains(needle_spaced) || line.contains(needle_tight)) {
                continue;
            }
            if in_test_module {
                continue;
            }
            offenders.push(format!("{}:{}: {trimmed}", path.display(), idx + 1));
        }
    }
}

#[test]
fn override_used_true_only_in_tests_and_comments() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut offenders = Vec::new();
    scan_dir(&src, &mut offenders);
    assert!(
        offenders.is_empty(),
        "AC8 grep discipline (mika#1733): the literal token `override_used = true` \
         (either spacing) must only appear inside comments or `#[cfg(test)]` scopes. \
         Production emit paths must derive the flag from \
         (classifier_verdict, decision, authority). Offenders:\n{}",
        offenders.join("\n")
    );
}
