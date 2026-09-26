//! No bundled skill may declare a Webhook-Fallthrough-withheld tool as
//! `required_tools` (mika#2517 V2).
//!
//! # The claim this test replaces
//!
//! The mika#2517 plan states, in § 3.3 (c), that the `required_tools` gate
//! cannot break because `filter_available_required_tools()` "pre-filters
//! required tools against the set actually available, so a `create_task`
//! declared `required` by a skill *and* withheld is simply dropped from the
//! contract rather than producing an impossible re-prompt."
//!
//! **Reading the code refuses that reassurance.**
//! `filter_available_required_tools` (`agent_loop/mod.rs`) tests
//! `tools.get(t).is_some()` — the shared `ToolRegistry` — while
//! `apply_agent_tool_visibility` filters `tool_defs`, the *presentation* array.
//! The registry is unchanged by the withholding, so a withheld-and-required
//! tool would survive the pre-filter and the gate would re-prompt for a tool
//! the model cannot see.
//!
//! That gap is **not introduced here**: it has existed since mika#811 for the
//! identity `[tools].disabled` denylist, which withholds through exactly the
//! same hook. What mika#2517 changes is that one more tool can now be withheld,
//! so the gap becomes reachable for `create_task` specifically.
//!
//! # What makes it safe today, and what this test pins
//!
//! The population is **empty**: no bundled skill declares `create_task` in
//! `[constraints] required_tools` (measured 2026-09-25 — the nearest neighbour
//! is `self-dev`, which declares `run_claude_pilot`, a tool mika#2517
//! deliberately keeps served). Emptiness is therefore the whole safety
//! argument, and an argument that rests on a measurement needs a guard, or it
//! decays into a belief the day someone adds a manifest line.
//!
//! When this test fires the resolution is **not** to add the skill to an
//! allowlist: either remove the declaration, or close the underlying gap by
//! teaching the pre-filter about the withheld set. An allowlist here would
//! silence the one signal that says a turn is about to be re-prompted for the
//! impossible.

use std::path::{Path, PathBuf};

/// Mirror of `agent_loop::FALLTHROUGH_WITHHELD_TOOLS`, which is `pub(crate)`.
///
/// A second copy, and it is the reason this file carries an anti-vacuity
/// assertion below: a list that drifted out of sync with the production one
/// would make this guard look green while watching nothing.
const WITHHELD: &[&str] = &["create_task"];

fn bundled_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../skills/bundled")
}

/// Every `(skill name, skill.toml contents)` under `skills/bundled/`.
fn manifests() -> Vec<(String, String)> {
    let root = bundled_root();
    let mut out = Vec::new();
    let entries = std::fs::read_dir(&root)
        .unwrap_or_else(|e| panic!("reading {}: {e}", root.display()))
        .filter_map(Result::ok);
    for entry in entries {
        let name = entry.file_name().to_string_lossy().into_owned();
        // `_shared/` is a support directory, not a skill (mika#923).
        if name.starts_with('_') || name.starts_with('.') {
            continue;
        }
        let toml = entry.path().join("skill.toml");
        if let Ok(contents) = std::fs::read_to_string(&toml) {
            out.push((name, contents));
        }
    }
    out
}

#[test]
fn mika2517_no_bundled_skill_requires_a_withheld_tool() {
    let manifests = manifests();

    // Anti-vacuity: a scan that reads no manifest is indistinguishable from a
    // clean tree (mika#2103 / mika#2205). The directory is discovered at build
    // time by `build.rs`, so an empty read here means the path moved.
    assert!(
        manifests.len() > 10,
        "expected the bundled-skill manifests to be readable at {}; found {} — \
         this scan is looking at nothing",
        bundled_root().display(),
        manifests.len()
    );

    // Second anti-vacuity term, on the needle rather than the haystack: at
    // least one manifest must declare `required_tools` at all, or the parse
    // below could be silently wrong about what it is looking for.
    assert!(
        manifests
            .iter()
            .any(|(_, toml)| toml.contains("required_tools")),
        "no manifest declares required_tools — the predicate is aiming at a \
         dead key and verifies nothing"
    );

    let mut offenders = Vec::new();
    for (skill, toml) in &manifests {
        for line in toml.lines() {
            let line = line.trim_start();
            if line.starts_with('#') || !line.starts_with("required_tools") {
                continue;
            }
            for tool in WITHHELD {
                if line.contains(&format!("\"{tool}\"")) {
                    offenders.push(format!("{skill}: {line}"));
                }
            }
        }
    }

    assert!(
        offenders.is_empty(),
        "mika#2517 — a bundled skill requires a tool a Webhook Fallthrough turn \
         is not handed:\n{}\n\n\
         The required-tools gate pre-filters against the shared ToolRegistry, \
         NOT against the per-turn visibility array, so this tool survives the \
         pre-filter and the gate re-prompts for something the model cannot see.\n\n\
         RESOLUTION: remove the declaration, or teach \
         `filter_available_required_tools` about the withheld set. Do NOT \
         allowlist the skill — the empty population is the whole reason the \
         withholding is safe.",
        offenders.join("\n")
    );
}
