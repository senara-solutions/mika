//! Integration tests for KG docs root resolution (#738).
//!
//! Verifies that `resolve_kg_docs_root` returns the correct path and source
//! under the scenarios described in the issue: env-set, config-set, CWD
//! fallback, and empty-value edge cases.

use mika_agent::kg::config::{PathSource, resolve_kg_docs_root};
use mika_common::config::Settings;
use serial_test::serial;
use std::path::PathBuf;

/// Clean env to avoid leaking across tests.
fn clean_kg_env() {
    // Safety: tests set env vars; no production thread reads these.
    unsafe {
        std::env::remove_var("MIKA_KG_DOCS_ROOT");
    }
}

#[test]
#[serial]
fn env_var_overrides_cwd() {
    clean_kg_env();

    // Point env var at a known directory (doesn't need to exist for the
    // resolver — existence checking is the consumer's responsibility).
    let target = "/srv/mika-repo/docs/solutions";
    unsafe { std::env::set_var("MIKA_KG_DOCS_ROOT", target) };

    let settings = Settings::test_defaults();
    let (path, source) = resolve_kg_docs_root(&settings);

    assert_eq!(path, PathBuf::from(target));
    assert_eq!(source, PathSource::EnvVar);

    clean_kg_env();
}

#[test]
#[serial]
fn config_file_overrides_cwd() {
    clean_kg_env();

    let mut settings = Settings::test_defaults();
    settings.kg_docs_root = Some(PathBuf::from("/opt/mika/docs/solutions"));

    let (path, source) = resolve_kg_docs_root(&settings);

    assert_eq!(path, PathBuf::from("/opt/mika/docs/solutions"));
    assert_eq!(source, PathSource::ConfigFile);
}

#[test]
#[serial]
fn cwd_default_when_nothing_configured() {
    clean_kg_env();

    let settings = Settings::test_defaults();
    let (path, source) = resolve_kg_docs_root(&settings);

    assert_eq!(source, PathSource::CwdDefault);
    assert!(
        path.ends_with("docs/solutions"),
        "expected path ending with docs/solutions, got: {}",
        path.display()
    );
}

#[test]
#[serial]
fn env_var_wins_over_config_file() {
    clean_kg_env();
    unsafe { std::env::set_var("MIKA_KG_DOCS_ROOT", "/env/path") };

    let mut settings = Settings::test_defaults();
    settings.kg_docs_root = Some(PathBuf::from("/config/path"));

    let (path, source) = resolve_kg_docs_root(&settings);

    assert_eq!(path, PathBuf::from("/env/path"));
    assert_eq!(source, PathSource::EnvVar);

    clean_kg_env();
}

#[test]
#[serial]
fn empty_env_var_returns_empty_path() {
    clean_kg_env();
    unsafe { std::env::set_var("MIKA_KG_DOCS_ROOT", "") };

    let settings = Settings::test_defaults();
    let (path, source) = resolve_kg_docs_root(&settings);

    assert_eq!(path, PathBuf::from(""));
    assert_eq!(source, PathSource::EnvVar);

    clean_kg_env();
}

#[test]
#[serial]
fn nonexistent_env_path_returns_verbatim() {
    clean_kg_env();
    unsafe { std::env::set_var("MIKA_KG_DOCS_ROOT", "/nonexistent/path/docs/solutions") };

    let settings = Settings::test_defaults();
    let (path, source) = resolve_kg_docs_root(&settings);

    // Resolver returns the path verbatim — no existence check.
    assert_eq!(path, PathBuf::from("/nonexistent/path/docs/solutions"));
    assert_eq!(source, PathSource::EnvVar);

    clean_kg_env();
}
