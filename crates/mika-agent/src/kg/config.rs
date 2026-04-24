//! KG configuration helpers.
//!
//! This module provides the docs-root resolution chain consumed by the
//! lexical ingestor at server startup and (in the future) by #778's
//! per-agent resolver.

use std::path::PathBuf;

use mika_common::config::Settings;

/// Source of the resolved docs root path.
///
/// Exposed so downstream consumers (e.g., #778's per-agent policy classifier)
/// can distinguish "operator explicitly set this path; hard-error if missing"
/// from "fell through to container-friendly default; warn-and-skip if missing".
///
/// **If you add a new variant**, update `resolve_per_agent_docs_root` in this
/// module (when #778 lands) to classify it correctly. The exhaustive match
/// will force a compile error, but this breadcrumb names the _why_.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathSource {
    /// Path came from `MIKA_KG_DOCS_ROOT` env var.
    EnvVar,
    /// Path came from `settings.kg_docs_root` (config.toml).
    ConfigFile,
    /// Path fell through to `std::env::current_dir().join("docs/solutions")`.
    CwdDefault,
}

/// Resolve the KG docs root using the config cascade.
///
/// # Resolution order (first hit wins)
///
/// 1. `MIKA_KG_DOCS_ROOT` environment variable (absolute path).
/// 2. `settings.kg_docs_root` field from config.toml (absolute path).
/// 3. `<CWD>/docs/solutions` — container-native default.
///
/// # Why re-inspect the env var
///
/// Config-rs merges `MIKA_KG_DOCS_ROOT` into `settings.kg_docs_root`, so
/// `Some(...)` could be _either_ env or config file. We re-inspect the env
/// var directly to distinguish the two sources for `PathSource`.
///
/// # Public contract
///
/// This function is consumed by #778's per-agent resolver. Signature changes
/// require coordinated update across both tickets. The `signature_binding`
/// test below catches mechanical drift at compile time.
///
/// # No validation
///
/// Path existence is **not** checked here — the consumer site (`server/mod.rs`)
/// owns the warn-and-skip policy. This keeps the resolver pure and allows
/// #778 to apply a different policy (hard-error) downstream.
pub fn resolve_kg_docs_root(settings: &Settings) -> (PathBuf, PathSource) {
    // 1. Env var (highest priority).
    if let Ok(env_path) = std::env::var("MIKA_KG_DOCS_ROOT") {
        return (PathBuf::from(env_path), PathSource::EnvVar);
    }

    // 2. Config file field.
    if let Some(config_path) = settings.kg_docs_root.clone() {
        return (config_path, PathSource::ConfigFile);
    }

    // 3. CWD-based default (container-native).
    let cwd_default = std::env::current_dir()
        .unwrap_or_default()
        .join("docs")
        .join("solutions");
    (cwd_default, PathSource::CwdDefault)
}

#[cfg(test)]
mod tests {
    use super::*;
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
    fn env_var_wins_over_config() {
        clean_kg_env();
        unsafe { std::env::set_var("MIKA_KG_DOCS_ROOT", "/env/docs") };

        let mut settings = Settings::test_defaults();
        settings.kg_docs_root = Some(PathBuf::from("/config/docs"));

        let (path, source) = resolve_kg_docs_root(&settings);
        assert_eq!(path, PathBuf::from("/env/docs"));
        assert_eq!(source, PathSource::EnvVar);

        clean_kg_env();
    }

    #[test]
    #[serial]
    fn config_file_used_when_no_env() {
        clean_kg_env();

        let mut settings = Settings::test_defaults();
        settings.kg_docs_root = Some(PathBuf::from("/config/docs"));

        let (path, source) = resolve_kg_docs_root(&settings);
        assert_eq!(path, PathBuf::from("/config/docs"));
        assert_eq!(source, PathSource::ConfigFile);
    }

    #[test]
    #[serial]
    fn cwd_fallback_when_nothing_set() {
        clean_kg_env();

        let settings = Settings::test_defaults();
        let (path, source) = resolve_kg_docs_root(&settings);

        assert_eq!(source, PathSource::CwdDefault);
        // Verify the full path is CWD-rooted, not just checking the suffix.
        let expected = std::env::current_dir()
            .unwrap()
            .join("docs")
            .join("solutions");
        assert_eq!(path, expected);
    }

    #[test]
    #[serial]
    fn empty_env_var_returns_env_source() {
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
    fn empty_config_returns_config_source() {
        clean_kg_env();

        let mut settings = Settings::test_defaults();
        settings.kg_docs_root = Some(PathBuf::from(""));

        let (path, source) = resolve_kg_docs_root(&settings);
        assert_eq!(path, PathBuf::from(""));
        assert_eq!(source, PathSource::ConfigFile);
    }

    /// Signature binding — prevents silent drift from the public contract
    /// that #778 depends on.
    #[test]
    fn signature_binding() {
        let _: fn(&Settings) -> (PathBuf, PathSource) = resolve_kg_docs_root;
    }

    /// Exhaustiveness check — adding a `PathSource` variant without updating
    /// this match (and by extension, #778's `resolve_per_agent_docs_root`)
    /// produces a compile error.
    #[test]
    fn path_source_exhaustive() {
        let source = PathSource::CwdDefault;
        match source {
            PathSource::EnvVar => (),
            PathSource::ConfigFile => (),
            PathSource::CwdDefault => (),
        }
    }
}
