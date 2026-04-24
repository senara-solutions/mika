//! KG configuration helpers.
//!
//! This module provides the docs-root resolution chain consumed by the
//! lexical ingestor at server startup and (in the future) by #778's
//! per-agent resolver.

use std::path::{Path, PathBuf};

use mika_common::config::Settings;
use sha2::{Digest, Sha256};

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

/// Compute a 16-hex-char hash of a docs root path for use as a shared-corpus
/// key in the KG schema (v27+).
///
/// # Semantics
///
/// - `sha256(fs::canonicalize(path))[:16]` — 16 hex chars = 64 bits.
/// - If canonicalization fails (e.g., path does not exist), the raw path
///   bytes are hashed instead. Consumers that care about path existence
///   (e.g., #778's per-agent resolver) check separately.
/// - **Per-host stability only.** `~/.mika/data/mika.db` is machine-local
///   (same category as `~/.cache`). The hash is stable across restarts on
///   the same host but NOT portable across hosts with different filesystem
///   layouts.
/// - Canonicalization is OS-dependent — on Windows, `fs::canonicalize`
///   yields UNC-prefixed paths (`\\?\C:\...`). The codebase targets
///   Linux/macOS; Windows behavior is documented, not tested.
///
/// # Public contract
///
/// Consumed by #778's per-agent resolver and #779's KG CLI status output.
/// Signature changes require coordinated update across those tickets. The
/// `signature_binding` test below catches mechanical drift at compile time.
pub fn hash_docs_root(path: &Path) -> String {
    let canonical = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let mut hasher = Sha256::new();
    hasher.update(canonical.as_os_str().as_encoded_bytes());
    let digest = hasher.finalize();
    // First 8 bytes = 16 hex chars = 64 bits.
    // Use per-byte formatting to guarantee zero-padded 16-char output.
    digest[..8]
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>()
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

    // ── hash_docs_root tests ──────────────────────────────────────────────

    #[test]
    fn hash_docs_root_returns_16_hex_chars() {
        let hash = hash_docs_root(Path::new("/tmp/foo"));
        assert_eq!(hash.len(), 16, "hash must be exactly 16 hex chars");
        assert!(
            hash.chars().all(|c| c.is_ascii_hexdigit()),
            "hash must contain only hex digits, got: {hash}"
        );
    }

    #[test]
    fn hash_docs_root_deterministic() {
        let a = hash_docs_root(Path::new("/tmp/foo"));
        let b = hash_docs_root(Path::new("/tmp/foo"));
        assert_eq!(a, b, "same path must produce same hash");
    }

    #[test]
    fn hash_docs_root_canonicalization() {
        // Create real directories so canonicalize resolves both paths identically.
        // Both `target` AND `aux` must exist for `aux/../target` to canonicalize.
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("target");
        std::fs::create_dir_all(&target).unwrap();
        let aux = dir.path().join("aux");
        std::fs::create_dir_all(&aux).unwrap();

        let via_dotdot = dir.path().join("aux").join("..").join("target");
        assert_eq!(
            hash_docs_root(&target),
            hash_docs_root(&via_dotdot),
            "canonicalized paths to the same directory must produce the same hash"
        );
    }

    #[test]
    fn hash_docs_root_nonexistent_path_no_panic() {
        let hash = hash_docs_root(Path::new("/does/not/exist/xyz"));
        assert_eq!(
            hash.len(),
            16,
            "non-existent path still returns 16-char hash"
        );
        // Determinism holds even on non-existent paths.
        let hash2 = hash_docs_root(Path::new("/does/not/exist/xyz"));
        assert_eq!(hash, hash2);
    }

    #[test]
    fn hash_docs_root_different_paths_differ() {
        let a = hash_docs_root(Path::new("/a"));
        let b = hash_docs_root(Path::new("/b"));
        assert_ne!(a, b, "different paths should produce different hashes");
    }

    /// Signature binding — prevents silent drift from the public contract
    /// that #778 and #779 depend on.
    #[test]
    fn hash_docs_root_signature_binding() {
        let _: fn(&Path) -> String = hash_docs_root;
    }
}
