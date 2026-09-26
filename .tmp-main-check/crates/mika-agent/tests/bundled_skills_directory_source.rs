//! End-to-end proof for the `skills/bundled/` discovery path.
//!
//! This test does NOT exercise the real build-time `ENTRIES` table — that one
//! is empty in production during this refactor ticket. Instead, it invokes the
//! same `discover_bundled_skills` helper that `build.rs` uses, pointing at a
//! fixture tree under `tests/fixtures/skills/bundled/`. If discovery fails the
//! fixture, the production build path would fail identically.

#[path = "../build_support/bundled_skills_discover.rs"]
mod discover;

use std::path::{Path, PathBuf};

fn fixture_base() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("skills")
        .join("bundled")
}

#[test]
fn happy_path_fixture_is_discovered() {
    let skills = discover::discover_bundled_skills(&fixture_base());
    assert_eq!(skills.len(), 1, "expected exactly one fixture skill");

    let echo = &skills[0];
    assert_eq!(echo.name, "test-echo");

    // Sorted alphabetically: handlers/run.sh, skill.toml, system_prompt.md, tools.json
    let rel_paths: Vec<&str> = echo.files.iter().map(|f| f.rel_path.as_str()).collect();
    assert_eq!(
        rel_paths,
        vec![
            "handlers/run.sh",
            "skill.toml",
            "system_prompt.md",
            "tools.json",
        ],
        "fixture file list mismatch"
    );

    // Handler is marked executable; non-handlers are not.
    let handler = echo
        .files
        .iter()
        .find(|f| f.rel_path == "handlers/run.sh")
        .unwrap();
    assert!(
        handler.executable,
        "handlers/*.sh must be marked executable"
    );

    let toml = echo
        .files
        .iter()
        .find(|f| f.rel_path == "skill.toml")
        .unwrap();
    assert!(!toml.executable, "skill.toml must not be marked executable");
}

#[test]
fn missing_directory_returns_empty() {
    let bogus = Path::new(env!("CARGO_MANIFEST_DIR")).join("definitely-does-not-exist-xyz");
    let skills = discover::discover_bundled_skills(&bogus);
    assert!(
        skills.is_empty(),
        "missing directory must return empty vec, got {} entries",
        skills.len()
    );
}

#[test]
fn subdirectory_without_skill_toml_is_skipped() {
    // Build a temp layout: one valid skill, one subdir missing skill.toml,
    // one stray dotfile dir. Only the valid skill should be discovered.
    let tmp = tempfile::tempdir().unwrap();
    let base = tmp.path();

    // Valid skill
    std::fs::create_dir_all(base.join("valid")).unwrap();
    std::fs::write(base.join("valid/skill.toml"), "name = \"valid\"\n").unwrap();

    // Invalid — no skill.toml
    std::fs::create_dir_all(base.join("not-a-skill")).unwrap();
    std::fs::write(base.join("not-a-skill/README.md"), "orphan\n").unwrap();

    // Stray dotfile dir — should be ignored outright
    std::fs::create_dir_all(base.join(".hidden")).unwrap();
    std::fs::write(base.join(".hidden/skill.toml"), "name = \".hidden\"\n").unwrap();

    let skills = discover::discover_bundled_skills(base);
    assert_eq!(skills.len(), 1, "only `valid` should be discovered");
    assert_eq!(skills[0].name, "valid");
}

#[test]
fn nested_subdirectories_beyond_handlers_are_ignored() {
    // Defense-in-depth: the generator only descends into `handlers/`. Any other
    // nested subdirectory (e.g. `prompts/openai/`) is out of scope for this
    // refactor ticket and must be skipped, not discovered.
    let tmp = tempfile::tempdir().unwrap();
    let base = tmp.path().join("deep-skill");
    std::fs::create_dir_all(&base).unwrap();
    std::fs::write(base.join("skill.toml"), "name = \"deep\"\n").unwrap();
    std::fs::create_dir_all(base.join("handlers")).unwrap();
    std::fs::write(base.join("handlers/run.sh"), "#!/bin/sh\n").unwrap();
    // Nested subdir that should be IGNORED
    std::fs::create_dir_all(base.join("prompts/openai")).unwrap();
    std::fs::write(base.join("prompts/openai/system.md"), "nested\n").unwrap();

    let skills = discover::discover_bundled_skills(tmp.path());
    assert_eq!(skills.len(), 1);
    let rel: Vec<&str> = skills[0]
        .files
        .iter()
        .map(|f| f.rel_path.as_str())
        .collect();
    assert_eq!(rel, vec!["handlers/run.sh", "skill.toml"]);
    assert!(
        !rel.iter().any(|p| p.starts_with("prompts/")),
        "nested non-handlers/ subdirectories must be skipped"
    );
}

#[cfg(unix)]
#[test]
fn symlinked_skill_dir_is_skipped() {
    let tmp = tempfile::tempdir().unwrap();
    let base = tmp.path();
    let real = tmp.path().parent().unwrap().join("real-symlink-target");
    let _ = std::fs::remove_dir_all(&real);
    std::fs::create_dir_all(&real).unwrap();
    std::fs::write(real.join("skill.toml"), "name = \"real\"\n").unwrap();

    // Symlinked skill dir — must be filtered
    std::os::unix::fs::symlink(&real, base.join("symlinked")).unwrap();

    // Plus a legitimate regular skill alongside
    std::fs::create_dir_all(base.join("regular")).unwrap();
    std::fs::write(base.join("regular/skill.toml"), "name = \"regular\"\n").unwrap();

    let skills = discover::discover_bundled_skills(base);
    assert_eq!(skills.len(), 1);
    assert_eq!(skills[0].name, "regular");

    // Cleanup
    let _ = std::fs::remove_dir_all(&real);
}
