//! KG configuration helpers.
//!
//! This module provides the docs-root resolution chain consumed by the
//! lexical ingestor at server startup and by #778's per-agent resolver.
//! Extended by #798 for multi-corpus per-agent support.

use std::path::{Path, PathBuf};

use mika_common::config::Settings;
use sha2::{Digest, Sha256};

use crate::prompt::Identity;

/// A single corpus configuration: a validated docs root path and its
/// precomputed hash. Used inside `KgAgentConfig::Enabled`.
#[derive(Debug, Clone)]
pub struct CorpusConfig {
    pub docs_root: PathBuf,
    pub docs_root_hash: String,
}

/// Reason why KG is disabled for an agent.
#[derive(Debug, Clone)]
pub enum DisabledReason {
    /// `identity.kg.enabled = false` — operator-explicit opt-out.
    OperatorOptOut,
    /// CWD-default fell through and `<CWD>/docs/solutions` does not exist.
    CwdDefaultMissing,
    /// Plural source listed N paths; every one failed validation.
    AllPathsUnresolvable {
        source: PathSource,
        attempted: usize,
    },
}

/// Per-agent KG configuration resolved at `init_agent` time and cached on
/// `AgentState`. Eliminates partial states: either the agent has validated
/// corpora, or KG is entirely disabled.
///
/// # Pre-1.0 breaking change (#798)
///
/// Shape changed from `Enabled { docs_root, docs_root_hash }` to
/// `Enabled { corpora: Vec<CorpusConfig> }`. Single-corpus agents carry
/// a one-entry vec — behavior is byte-equivalent to the pre-#798 shape.
#[derive(Debug, Clone)]
pub enum KgAgentConfig {
    /// KG subsystem is disabled for this agent.
    Disabled { reason: DisabledReason },
    /// KG subsystem is enabled with one or more validated corpora.
    Enabled { corpora: Vec<CorpusConfig> },
}

/// Errors from per-agent KG config validation.
///
/// Uses `thiserror` per crate convention (root `CLAUDE.md`). Only covers
/// operator-set misconfiguration paths — runtime errors (LLM timeout, etc.)
/// use the existing KG subsystem's `anyhow` / log-and-skip pattern.
#[derive(Debug, thiserror::Error)]
pub enum KgConfigError {
    #[error("docs_root path does not exist: {path}")]
    PathNotFound {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("docs_root path is not a directory: {path}")]
    NotADirectory { path: PathBuf },
}

/// Resolve the per-agent KG configuration from identity.toml `[kg]` section
/// and the global fallback chain (#738, #798).
///
/// # Resolution chain (first hit wins, six sources)
///
/// 1. `identity.kg.docs_roots` (plural, TOML array) — `IdentityPathPlural`
/// 2. `identity.kg.docs_root`  (singular)            — `IdentityPath`
/// 3. `MIKA_KG_DOCS_ROOTS` env (colon-separated)     — `EnvVarPlural`
/// 4. `MIKA_KG_DOCS_ROOT` env  (single path)         — `EnvVar`
/// 5. `settings.kg_docs_roots` (plural, TOML array)   — `ConfigFilePlural`
/// 6. `settings.kg_docs_root`  (singular)             — `ConfigFile`
/// 7. `<CWD>/docs/solutions`                          — `CwdDefault`
///
/// # Validation policy (asymmetric by design)
///
/// - **Singular sources** keep #778's all-or-nothing: path missing → hard error.
/// - **Plural sources** use per-path validate-and-skip: missing paths emit
///   `kg_corpus_skipped` warn and are dropped. Agent goes `Disabled` only if
///   zero paths resolve.
/// - **CWD default** passes through without validation (downstream warn-and-skip).
///
/// # Call-site contract
///
/// This function is init-only, called from exactly one site (`server::init_agent`)
/// per process lifetime per agent. The dual-set warn has no dedup state and
/// would emit per call. Future hot-reload must re-warn with dedup.
pub fn resolve_per_agent_docs_root(
    identity: &Identity,
    settings: &Settings,
) -> Result<KgAgentConfig, KgConfigError> {
    // Disabled — early return, no validation.
    if !identity.kg.enabled {
        return Ok(KgAgentConfig::Disabled {
            reason: DisabledReason::OperatorOptOut,
        });
    }

    // Check for dual-set warnings at each tier.
    emit_dual_set_warns(identity, settings);

    // Tier 1: identity.kg.docs_roots (plural)
    if let Some(ref roots) = identity.kg.docs_roots
        && !roots.is_empty()
    {
        return build_corpora(roots, PathSource::IdentityPathPlural);
    }

    // Tier 2: identity.kg.docs_root (singular)
    if let Some(ref path) = identity.kg.docs_root {
        return build_corpora(std::slice::from_ref(path), PathSource::IdentityPath);
    }

    // Tier 3: MIKA_KG_DOCS_ROOTS env (colon-separated)
    if let Ok(s) = std::env::var("MIKA_KG_DOCS_ROOTS") {
        let paths: Vec<PathBuf> = s
            .split(':')
            .filter(|p| !p.is_empty())
            .map(PathBuf::from)
            .collect();
        if !paths.is_empty() {
            return build_corpora(&paths, PathSource::EnvVarPlural);
        }
    }

    // Tier 4: MIKA_KG_DOCS_ROOT env (singular)
    if let Ok(env_path) = std::env::var("MIKA_KG_DOCS_ROOT") {
        // Empty-path guard (#738): an empty string in env is a misconfiguration.
        if env_path.is_empty() {
            tracing::warn!(
                "kg_docs_root is set to empty string — check MIKA_KG_DOCS_ROOT env var; \
                 skipping KG for this agent"
            );
            return Ok(KgAgentConfig::Disabled {
                reason: DisabledReason::CwdDefaultMissing,
            });
        }
        return build_corpora(&[PathBuf::from(env_path)], PathSource::EnvVar);
    }

    // Tier 5: settings.kg_docs_roots (plural)
    if let Some(ref roots) = settings.kg_docs_roots
        && !roots.is_empty()
    {
        return build_corpora(roots, PathSource::ConfigFilePlural);
    }

    // Tier 6: settings.kg_docs_root (singular)
    if let Some(ref config_path) = settings.kg_docs_root {
        if config_path.as_os_str().is_empty() {
            tracing::warn!(
                "kg_docs_root is set to empty string in config.toml; \
                 skipping KG for this agent"
            );
            return Ok(KgAgentConfig::Disabled {
                reason: DisabledReason::CwdDefaultMissing,
            });
        }
        return build_corpora(std::slice::from_ref(config_path), PathSource::ConfigFile);
    }

    // Tier 7: CWD-based default (container-native).
    let cwd_default = std::env::current_dir()
        .unwrap_or_default()
        .join("docs")
        .join("solutions");
    build_corpora(&[cwd_default], PathSource::CwdDefault)
}

/// Emit dual-set warnings when both plural and singular are set at the same tier.
fn emit_dual_set_warns(identity: &Identity, settings: &Settings) {
    // Identity tier
    if identity
        .kg
        .docs_roots
        .as_ref()
        .is_some_and(|v| !v.is_empty())
        && identity.kg.docs_root.is_some()
    {
        tracing::warn!(
            event = "kg_docs_roots_singular_ignored",
            source = "identity",
            ignored_path = ?identity.kg.docs_root,
            "both [kg].docs_roots and [kg].docs_root set in identity.toml — \
             plural takes precedence, singular ignored"
        );
    }

    // Env tier
    let has_env_plural = std::env::var("MIKA_KG_DOCS_ROOTS")
        .ok()
        .is_some_and(|s| !s.is_empty() && s.split(':').any(|p| !p.is_empty()));
    let has_env_singular = std::env::var("MIKA_KG_DOCS_ROOT").is_ok();
    if has_env_plural && has_env_singular {
        tracing::warn!(
            event = "kg_docs_roots_singular_ignored",
            source = "env",
            "both MIKA_KG_DOCS_ROOTS and MIKA_KG_DOCS_ROOT env vars set — \
             plural takes precedence, singular ignored"
        );
    }

    // Config tier
    if settings
        .kg_docs_roots
        .as_ref()
        .is_some_and(|v| !v.is_empty())
        && settings.kg_docs_root.is_some()
    {
        tracing::warn!(
            event = "kg_docs_roots_singular_ignored",
            source = "config",
            ignored_path = ?settings.kg_docs_root,
            "both kg_docs_roots and kg_docs_root set in config.toml — \
             plural takes precedence, singular ignored"
        );
    }
}

/// Build corpora from a list of paths, applying validation per source type.
fn build_corpora(paths: &[PathBuf], source: PathSource) -> Result<KgAgentConfig, KgConfigError> {
    let is_plural = matches!(
        source,
        PathSource::IdentityPathPlural | PathSource::EnvVarPlural | PathSource::ConfigFilePlural
    );

    // Deduplicate by canonical hash.
    let deduped = dedupe_paths(paths);

    let mut corpora = Vec::with_capacity(deduped.len());

    for path in &deduped {
        match source {
            PathSource::CwdDefault => {
                // Per #738: CWD missing is warn-and-skip downstream.
                let hash = hash_docs_root(path);
                corpora.push(CorpusConfig {
                    docs_root: path.clone(),
                    docs_root_hash: hash,
                });
            }
            _ if is_plural => {
                // Per-path validate-and-skip for plural sources.
                match validate_explicit_path(path) {
                    Ok(c) => corpora.push(c),
                    Err(e) => {
                        tracing::warn!(
                            event = "kg_corpus_skipped",
                            source = ?source,
                            bad_path = %path.display(),
                            resolved_count = corpora.len(),
                            error = %e,
                            "plural-source path skipped; agent will run with remaining corpora"
                        );
                    }
                }
            }
            _ => {
                // Singular sources: all-or-nothing (#778 policy).
                corpora.push(validate_explicit_path(path)?);
            }
        }
    }

    if corpora.is_empty() {
        if is_plural {
            tracing::warn!(
                event = "kg_all_corpora_skipped",
                source = ?source,
                attempted = deduped.len(),
                "every plural-source path failed validation; agent KG disabled"
            );
            return Ok(KgAgentConfig::Disabled {
                reason: DisabledReason::AllPathsUnresolvable {
                    source,
                    attempted: deduped.len(),
                },
            });
        }
        Ok(KgAgentConfig::Disabled {
            reason: DisabledReason::CwdDefaultMissing,
        })
    } else {
        Ok(KgAgentConfig::Enabled { corpora })
    }
}

/// Deduplicate paths, emitting info for literal dupes and warn for canonical collisions.
fn dedupe_paths(paths: &[PathBuf]) -> Vec<PathBuf> {
    use std::collections::HashMap;

    let mut seen_literal: std::collections::HashSet<PathBuf> = std::collections::HashSet::new();
    let mut seen_hash: HashMap<String, PathBuf> = HashMap::new();
    let mut result = Vec::with_capacity(paths.len());

    for path in paths {
        // Literal duplicate check
        if !seen_literal.insert(path.clone()) {
            tracing::info!(
                event = "kg_docs_roots_duplicate_literal",
                path = %path.display(),
                "duplicate literal path in docs_roots — ignored"
            );
            continue;
        }

        // Canonical collision check
        let hash = hash_docs_root(path);
        if let Some(prev) = seen_hash.get(&hash) {
            tracing::warn!(
                event = "kg_docs_roots_duplicate_canonical",
                source_path = %path.display(),
                prev_path = %prev.display(),
                canonical_hash = %hash,
                "distinct paths canonicalize to same hash — only the first is used"
            );
            continue;
        }

        seen_hash.insert(hash, path.clone());
        result.push(path.clone());
    }

    result
}

/// Validate that an explicit path exists and is a directory, returning
/// `CorpusConfig` with computed hash on success.
fn validate_explicit_path(path: &Path) -> Result<CorpusConfig, KgConfigError> {
    match path.try_exists() {
        Ok(true) => {
            if !path.is_dir() {
                return Err(KgConfigError::NotADirectory {
                    path: path.to_path_buf(),
                });
            }
            let hash = hash_docs_root(path);
            Ok(CorpusConfig {
                docs_root: path.to_path_buf(),
                docs_root_hash: hash,
            })
        }
        Ok(false) => Err(KgConfigError::PathNotFound {
            path: path.to_path_buf(),
            source: std::io::Error::new(std::io::ErrorKind::NotFound, "path does not exist"),
        }),
        Err(e) => Err(KgConfigError::PathNotFound {
            path: path.to_path_buf(),
            source: e,
        }),
    }
}

/// Source of the resolved docs root path.
///
/// **If you add a new variant**, update `resolve_per_agent_docs_root` in this
/// module to classify it correctly. The exhaustive match in `path_source_exhaustive`
/// will force a compile error, but this breadcrumb names the _why_.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathSource {
    /// Path came from `identity.toml [kg].docs_roots` (plural, #798).
    IdentityPathPlural,
    /// Path came from `identity.toml [kg].docs_root` (singular, #778).
    IdentityPath,
    /// Path came from `MIKA_KG_DOCS_ROOTS` env var (colon-separated, #798).
    EnvVarPlural,
    /// Path came from `MIKA_KG_DOCS_ROOT` env var.
    EnvVar,
    /// Path came from `settings.kg_docs_roots` (config.toml array, #798).
    ConfigFilePlural,
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
/// # Public contract
///
/// This function is consumed by `resolve_per_agent_docs_root` in this module.
/// The `signature_binding` test below catches mechanical drift at compile time.
///
/// # No validation
///
/// Path existence is **not** checked here. `resolve_per_agent_docs_root`
/// applies hard-error for explicit sources (EnvVar/ConfigFile);
/// `server/mod.rs` handles CwdDefault warn-and-skip.
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
///   bytes are hashed instead.
/// - **Per-host stability only.**
pub fn hash_docs_root(path: &Path) -> String {
    let canonical = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let mut hasher = Sha256::new();
    hasher.update(canonical.as_os_str().as_encoded_bytes());
    let digest = hasher.finalize();
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
            std::env::remove_var("MIKA_KG_DOCS_ROOTS");
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

    /// Signature binding — prevents silent drift from the public contract.
    #[test]
    fn signature_binding() {
        let _: fn(&Settings) -> (PathBuf, PathSource) = resolve_kg_docs_root;
    }

    /// Exhaustiveness check — adding a `PathSource` variant without updating
    /// this match produces a compile error.
    #[test]
    fn path_source_exhaustive() {
        let source = PathSource::CwdDefault;
        match source {
            PathSource::IdentityPathPlural => (),
            PathSource::IdentityPath => (),
            PathSource::EnvVarPlural => (),
            PathSource::EnvVar => (),
            PathSource::ConfigFilePlural => (),
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
        let hash2 = hash_docs_root(Path::new("/does/not/exist/xyz"));
        assert_eq!(hash, hash2);
    }

    #[test]
    fn hash_docs_root_different_paths_differ() {
        let a = hash_docs_root(Path::new("/a"));
        let b = hash_docs_root(Path::new("/b"));
        assert_ne!(a, b, "different paths should produce different hashes");
    }

    #[test]
    fn hash_docs_root_signature_binding() {
        let _: fn(&Path) -> String = hash_docs_root;
    }

    // ── resolve_per_agent_docs_root tests (#778, #798) ───────────────────

    use crate::prompt::{Identity, KgIdentityConfig};

    fn identity_with_kg(
        enabled: bool,
        docs_root: Option<PathBuf>,
        docs_roots: Option<Vec<PathBuf>>,
    ) -> Identity {
        Identity {
            kg: KgIdentityConfig {
                enabled,
                docs_root,
                docs_roots,
            },
            ..Identity::default()
        }
    }

    #[test]
    fn resolve_per_agent_disabled_returns_disabled() {
        let identity = identity_with_kg(false, Some(PathBuf::from("/nonexistent")), None);
        let settings = Settings::test_defaults();
        let result = resolve_per_agent_docs_root(&identity, &settings).unwrap();
        assert!(matches!(
            result,
            KgAgentConfig::Disabled {
                reason: DisabledReason::OperatorOptOut
            }
        ));
    }

    #[test]
    fn resolve_per_agent_enabled_valid_path() {
        let dir = tempfile::tempdir().unwrap();
        let identity = identity_with_kg(true, Some(dir.path().to_path_buf()), None);
        let settings = Settings::test_defaults();
        let result = resolve_per_agent_docs_root(&identity, &settings).unwrap();
        match result {
            KgAgentConfig::Enabled { corpora } => {
                assert_eq!(corpora.len(), 1);
                assert_eq!(corpora[0].docs_root, dir.path());
                assert_eq!(corpora[0].docs_root_hash, hash_docs_root(dir.path()));
            }
            KgAgentConfig::Disabled { .. } => panic!("expected Enabled"),
        }
    }

    #[test]
    fn resolve_per_agent_enabled_nonexistent_path_errors() {
        let identity = identity_with_kg(true, Some(PathBuf::from("/does/not/exist/xyz")), None);
        let settings = Settings::test_defaults();
        let result = resolve_per_agent_docs_root(&identity, &settings);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            KgConfigError::PathNotFound { .. }
        ));
    }

    #[test]
    fn resolve_per_agent_enabled_file_not_dir_errors() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("not_a_dir.txt");
        std::fs::write(&file_path, "hello").unwrap();
        let identity = identity_with_kg(true, Some(file_path), None);
        let settings = Settings::test_defaults();
        let result = resolve_per_agent_docs_root(&identity, &settings);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            KgConfigError::NotADirectory { .. }
        ));
    }

    #[test]
    #[serial]
    fn resolve_per_agent_fallback_env_valid() {
        clean_kg_env();
        let dir = tempfile::tempdir().unwrap();
        unsafe { std::env::set_var("MIKA_KG_DOCS_ROOT", dir.path().as_os_str()) };

        let identity = identity_with_kg(true, None, None);
        let settings = Settings::test_defaults();
        let result = resolve_per_agent_docs_root(&identity, &settings).unwrap();
        match result {
            KgAgentConfig::Enabled { corpora } => {
                assert_eq!(corpora.len(), 1);
                assert_eq!(corpora[0].docs_root, dir.path());
                assert_eq!(corpora[0].docs_root_hash, hash_docs_root(dir.path()));
            }
            KgAgentConfig::Disabled { .. } => panic!("expected Enabled"),
        }
        clean_kg_env();
    }

    #[test]
    #[serial]
    fn resolve_per_agent_fallback_env_invalid_errors() {
        clean_kg_env();
        unsafe { std::env::set_var("MIKA_KG_DOCS_ROOT", "/env/nonexistent/path") };

        let identity = identity_with_kg(true, None, None);
        let settings = Settings::test_defaults();
        let result = resolve_per_agent_docs_root(&identity, &settings);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            KgConfigError::PathNotFound { .. }
        ));
        clean_kg_env();
    }

    #[test]
    #[serial]
    fn resolve_per_agent_fallback_config_valid() {
        clean_kg_env();
        let dir = tempfile::tempdir().unwrap();
        let mut settings = Settings::test_defaults();
        settings.kg_docs_root = Some(dir.path().to_path_buf());

        let identity = identity_with_kg(true, None, None);
        let result = resolve_per_agent_docs_root(&identity, &settings).unwrap();
        match result {
            KgAgentConfig::Enabled { corpora } => {
                assert_eq!(corpora.len(), 1);
                assert_eq!(corpora[0].docs_root, dir.path());
            }
            KgAgentConfig::Disabled { .. } => panic!("expected Enabled"),
        }
    }

    #[test]
    #[serial]
    fn resolve_per_agent_fallback_config_invalid_errors() {
        clean_kg_env();
        let mut settings = Settings::test_defaults();
        settings.kg_docs_root = Some(PathBuf::from("/config/nonexistent/path"));

        let identity = identity_with_kg(true, None, None);
        let result = resolve_per_agent_docs_root(&identity, &settings);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            KgConfigError::PathNotFound { .. }
        ));
    }

    #[test]
    #[serial]
    fn resolve_per_agent_cwd_default_passes_through() {
        clean_kg_env();
        let settings = Settings::test_defaults();
        let identity = identity_with_kg(true, None, None);
        let result = resolve_per_agent_docs_root(&identity, &settings).unwrap();
        match result {
            KgAgentConfig::Enabled { corpora } => {
                assert_eq!(corpora.len(), 1);
                let expected = std::env::current_dir()
                    .unwrap()
                    .join("docs")
                    .join("solutions");
                assert_eq!(corpora[0].docs_root, expected);
                assert!(!corpora[0].docs_root_hash.is_empty());
            }
            KgAgentConfig::Disabled { .. } => panic!("expected Enabled"),
        }
    }

    #[test]
    #[serial]
    fn resolve_per_agent_empty_env_returns_disabled() {
        clean_kg_env();
        unsafe { std::env::set_var("MIKA_KG_DOCS_ROOT", "") };

        let identity = identity_with_kg(true, None, None);
        let settings = Settings::test_defaults();
        let result = resolve_per_agent_docs_root(&identity, &settings).unwrap();
        assert!(matches!(result, KgAgentConfig::Disabled { .. }));
        clean_kg_env();
    }

    /// Signature binding for the per-agent resolver.
    #[test]
    fn resolve_per_agent_signature_binding() {
        let _: fn(&Identity, &Settings) -> Result<KgAgentConfig, KgConfigError> =
            resolve_per_agent_docs_root;
    }

    // ── Multi-corpus tests (#798) ────────────────────────────────────────

    #[test]
    fn resolve_per_agent_plural_identity_wins() {
        let dir_a = tempfile::tempdir().unwrap();
        let dir_b = tempfile::tempdir().unwrap();
        let identity = identity_with_kg(
            true,
            Some(dir_a.path().to_path_buf()),
            Some(vec![dir_a.path().to_path_buf(), dir_b.path().to_path_buf()]),
        );
        let settings = Settings::test_defaults();
        let result = resolve_per_agent_docs_root(&identity, &settings).unwrap();
        match result {
            KgAgentConfig::Enabled { corpora } => {
                assert_eq!(corpora.len(), 2);
            }
            KgAgentConfig::Disabled { .. } => panic!("expected Enabled with 2 corpora"),
        }
    }

    #[test]
    fn resolve_per_agent_plural_skip_missing() {
        let dir_a = tempfile::tempdir().unwrap();
        let identity = identity_with_kg(
            true,
            None,
            Some(vec![
                dir_a.path().to_path_buf(),
                PathBuf::from("/does/not/exist/798"),
            ]),
        );
        let settings = Settings::test_defaults();
        let result = resolve_per_agent_docs_root(&identity, &settings).unwrap();
        match result {
            KgAgentConfig::Enabled { corpora } => {
                assert_eq!(corpora.len(), 1, "missing path should be skipped");
                assert_eq!(corpora[0].docs_root, dir_a.path());
            }
            KgAgentConfig::Disabled { .. } => panic!("expected Enabled with 1 corpus"),
        }
    }

    #[test]
    fn resolve_per_agent_plural_all_missing_disabled() {
        let identity = identity_with_kg(
            true,
            None,
            Some(vec![
                PathBuf::from("/missing/a"),
                PathBuf::from("/missing/b"),
            ]),
        );
        let settings = Settings::test_defaults();
        let result = resolve_per_agent_docs_root(&identity, &settings).unwrap();
        assert!(matches!(
            result,
            KgAgentConfig::Disabled {
                reason: DisabledReason::AllPathsUnresolvable { .. }
            }
        ));
    }

    #[test]
    fn resolve_per_agent_empty_plural_falls_through() {
        // Empty docs_roots should fall through to singular
        let dir = tempfile::tempdir().unwrap();
        let identity = identity_with_kg(
            true,
            Some(dir.path().to_path_buf()),
            Some(vec![]), // empty
        );
        let settings = Settings::test_defaults();
        let result = resolve_per_agent_docs_root(&identity, &settings).unwrap();
        match result {
            KgAgentConfig::Enabled { corpora } => {
                assert_eq!(corpora.len(), 1);
                assert_eq!(corpora[0].docs_root, dir.path());
            }
            KgAgentConfig::Disabled { .. } => panic!("expected Enabled from singular fallback"),
        }
    }

    #[test]
    #[serial]
    fn resolve_per_agent_env_plural_wins_over_env_singular() {
        clean_kg_env();
        let dir_a = tempfile::tempdir().unwrap();
        let dir_b = tempfile::tempdir().unwrap();
        let dir_c = tempfile::tempdir().unwrap();

        let env_val = format!("{}:{}", dir_a.path().display(), dir_b.path().display());
        unsafe {
            std::env::set_var("MIKA_KG_DOCS_ROOTS", &env_val);
            std::env::set_var("MIKA_KG_DOCS_ROOT", dir_c.path().as_os_str());
        }

        let identity = identity_with_kg(true, None, None);
        let settings = Settings::test_defaults();
        let result = resolve_per_agent_docs_root(&identity, &settings).unwrap();
        match result {
            KgAgentConfig::Enabled { corpora } => {
                assert_eq!(corpora.len(), 2);
            }
            KgAgentConfig::Disabled { .. } => panic!("expected Enabled from env plural"),
        }
        clean_kg_env();
    }

    #[test]
    fn resolve_per_agent_literal_dedup() {
        let dir = tempfile::tempdir().unwrap();
        let identity = identity_with_kg(
            true,
            None,
            Some(vec![
                dir.path().to_path_buf(),
                dir.path().to_path_buf(), // literal duplicate
            ]),
        );
        let settings = Settings::test_defaults();
        let result = resolve_per_agent_docs_root(&identity, &settings).unwrap();
        match result {
            KgAgentConfig::Enabled { corpora } => {
                assert_eq!(corpora.len(), 1, "literal duplicate should be deduped");
            }
            KgAgentConfig::Disabled { .. } => panic!("expected Enabled"),
        }
    }

    #[test]
    #[serial]
    fn resolve_per_agent_config_plural() {
        clean_kg_env();
        let dir_a = tempfile::tempdir().unwrap();
        let dir_b = tempfile::tempdir().unwrap();
        let mut settings = Settings::test_defaults();
        settings.kg_docs_roots = Some(vec![dir_a.path().to_path_buf(), dir_b.path().to_path_buf()]);

        let identity = identity_with_kg(true, None, None);
        let result = resolve_per_agent_docs_root(&identity, &settings).unwrap();
        match result {
            KgAgentConfig::Enabled { corpora } => {
                assert_eq!(corpora.len(), 2);
            }
            KgAgentConfig::Disabled { .. } => panic!("expected Enabled from config plural"),
        }
    }
}
