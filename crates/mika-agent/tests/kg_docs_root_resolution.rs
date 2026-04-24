//! Integration tests for KG docs root resolution (#738).
//!
//! Unit-level scenarios (env wins, config wins, CWD default, empty values) are
//! covered by `kg::config::tests` inline. These integration tests verify the
//! public API surface and exercise scenarios that benefit from running outside
//! the crate (verifying re-exports work, testing from a consumer perspective).

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

#[test]
#[serial]
fn cwd_default_produces_full_path() {
    clean_kg_env();

    let settings = Settings::test_defaults();
    let (path, source) = resolve_kg_docs_root(&settings);

    assert_eq!(source, PathSource::CwdDefault);
    let expected = std::env::current_dir()
        .unwrap()
        .join("docs")
        .join("solutions");
    assert_eq!(path, expected);

    clean_kg_env();
}

/// Verify that the public API types are accessible from outside the crate.
#[test]
fn public_api_accessible() {
    let _: fn(&Settings) -> (PathBuf, PathSource) = resolve_kg_docs_root;
}
