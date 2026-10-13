//! Every `CLAUDE.md` in this repository is loaded whole into a pilot's prompt:
//! cap their size.
//!
//! # The failure this closes, measured 2026-10-13
//!
//! Every PR documented its variables, operator surfaces and subsystem details in
//! a `CLAUDE.md`, and nothing bounded those files. On `main` the root
//! `CLAUDE.md` reached 825 140 bytes (767 KB of it in § Environment Variables)
//! and `crates/mika-agent/CLAUDE.md` 594 290 bytes. Claude Code puts the root
//! file in the system prompt of a pilot running in this repository, so the
//! request exceeded the pilot model's context window before the first turn: the
//! SDK answered a synthetic "Prompt is too long" in ~140 ms, at zero cost, and
//! the dispatch died with "zero new commits". Pilots on this repo died that way
//! on 2026-09-22, 2026-09-30 (×4), 2026-10-01, 2026-10-05 and 2026-10-13 (groom
//! of mika#2683). A crate's `CLAUDE.md` is loaded as soon as the pilot reads a
//! file under that crate, so the mika-agent one would have killed the same
//! pilots one step later.
//!
//! The material now lives in `docs/operator/environment-variables.md` and
//! `docs/agent-internals/`, which no pilot loads unless it reads them.
//!
//! # Why a test, and why this number
//!
//! The files kept growing for the plain reason that appending to them was the
//! documented gesture: many comments in this tree still say "date it in
//! CLAUDE.md". A cap that fails the build is the only thing that stops the next
//! 800 KB from arriving one paragraph at a time.
//!
//! The cap is 100 000 bytes per file. After the move the largest is 76 078
//! bytes (`crates/mika-agent/CLAUDE.md`). Prose with code runs at roughly 3 to
//! 4 bytes per token, so 100 KB is under 35 000 tokens: the root file plus one
//! crate file stays well inside a 200 000-token window, leaving room for the
//! workspace `CLAUDE.md`, the skills and the work itself. It is a ceiling, not a
//! target. When this test fails, move the new material to `docs/` and leave a
//! pointer. Do not raise the number to make room.

use std::path::{Path, PathBuf};

/// Upper bound, in bytes, for any `CLAUDE.md` in the repository.
const CLAUDE_MD_MAX_BYTES: u64 = 100_000;

/// Directories that hold no tracked `CLAUDE.md` and are expensive to walk.
const SKIPPED_DIRS: &[&str] = &["target", "node_modules", ".git"];

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn claude_md_files(dir: &Path, found: &mut Vec<PathBuf>) {
    let entries =
        std::fs::read_dir(dir).unwrap_or_else(|e| panic!("cannot read {}: {e}", dir.display()));
    for entry in entries {
        let entry =
            entry.unwrap_or_else(|e| panic!("cannot read an entry of {}: {e}", dir.display()));
        let path = entry.path();
        let file_type = entry
            .file_type()
            .unwrap_or_else(|e| panic!("cannot stat {}: {e}", path.display()));
        if file_type.is_dir() {
            let name = entry.file_name();
            if !SKIPPED_DIRS.iter().any(|s| name == *s) {
                claude_md_files(&path, found);
            }
        } else if file_type.is_file() && entry.file_name() == "CLAUDE.md" {
            found.push(path);
        }
    }
}

#[test]
fn every_claude_md_stays_under_the_pilot_prompt_cap() {
    let mut files = Vec::new();
    claude_md_files(&repo_root(), &mut files);

    // Guard against a vacuous green: a wrong root finds nothing and passes.
    let root_file = repo_root().join("CLAUDE.md");
    assert!(
        files
            .iter()
            .any(|f| f.ends_with("CLAUDE.md") && f.parent() == root_file.parent()),
        "the walk did not find the repository-root CLAUDE.md under {}; the root is wrong",
        repo_root().display()
    );
    assert!(
        files.len() >= 5,
        "found only {} CLAUDE.md files; the repository has one per crate",
        files.len()
    );

    let over: Vec<String> = files
        .iter()
        .filter_map(|f| {
            let len = std::fs::metadata(f)
                .unwrap_or_else(|e| panic!("cannot stat {}: {e}", f.display()))
                .len();
            (len > CLAUDE_MD_MAX_BYTES).then(|| format!("{} ({len} bytes)", f.display()))
        })
        .collect();
    assert!(
        over.is_empty(),
        "over the {CLAUDE_MD_MAX_BYTES}-byte cap: {over:?}. A CLAUDE.md is loaded whole \
         into a pilot's prompt; at 825 KB pilots died on turn 1 (\"Prompt is too long\"). \
         Move the new material to docs/ (variables and operator surfaces go in \
         docs/operator/environment-variables.md, subsystem references in docs/agent-internals/) \
         and leave a pointer. Do not raise the cap."
    );
}

/// Negative control: the cap must bite on the sizes that broke the loop, so a
/// later edit that makes the bound vacuous fails here. Evaluated at compile
/// time: raising the cap past these sizes does not build.
const _: () = {
    const ROOT_SIZE_THAT_BROKE_THE_LOOP: u64 = 825_140;
    const AGENT_SIZE_THAT_WOULD_HAVE_BROKEN_IT: u64 = 594_290;
    assert!(ROOT_SIZE_THAT_BROKE_THE_LOOP > CLAUDE_MD_MAX_BYTES);
    assert!(AGENT_SIZE_THAT_WOULD_HAVE_BROKEN_IT > CLAUDE_MD_MAX_BYTES);
};
