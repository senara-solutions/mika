//! Fetch the touched-file list for a PR from GitHub (mika#1829).
//!
//! Kept in a dedicated submodule so the pure classifier ([`super::mod`]) has
//! no I/O dependency and stays trivially unit-testable. Callers on the
//! auto-merge path (`verdict_handler`, `pr_merge_with_gate`) wire the fetched
//! list into [`super::classify_pr_files`].
//!
//! Uses the crate-local `run_gh_subprocess` helper so a single gh CLI binary
//! path + auth flow is shared with every other gh call.

use crate::tools::pr_merge_with_gate::run_gh_subprocess;
use serde::Deserialize;
use tracing::warn;

/// Minimal file entry from `gh pr view --json files`. Only `path` is used.
#[derive(Debug, Deserialize)]
struct PrFileEntry {
    path: String,
}

/// Minimal wrapper matching the `--json files` shape.
#[derive(Debug, Deserialize)]
struct PrFilesEnvelope {
    files: Vec<PrFileEntry>,
}

/// Error class for `fetch_pr_files`. Fail-closed callers (both integration
/// sites) treat every variant identically — as an "assume DECISION-CORE"
/// signal. Kept separate for observability.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FetchError {
    /// The `gh` CLI returned a non-zero exit code.
    GhCliFailure(String),
    /// The response could not be parsed as the expected JSON shape.
    ParseError(String),
}

impl std::fmt::Display for FetchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::GhCliFailure(msg) => write!(f, "gh CLI failure: {msg}"),
            Self::ParseError(msg) => write!(f, "parse error: {msg}"),
        }
    }
}

impl std::error::Error for FetchError {}

/// Fetch the list of files touched by a PR.
///
/// Uses `gh pr view <number> --repo <repo> --json files`. The `--json files`
/// shape is stable in gh CLI and returns one entry per file (paginated
/// server-side by gh). Suitable for typical PRs; extremely-large PRs (>3000
/// files) may truncate — but such PRs are outliers for the auto-merge path.
///
/// Returns the plain path strings (repo-relative, forward-slash separators
/// on all platforms — GitHub's canonical form). Deduplicated + sorted for
/// stable comparisons.
pub async fn fetch_pr_files(
    pr_number: u64,
    repo: &str,
    token: &str,
) -> Result<Vec<String>, FetchError> {
    let pr_str = pr_number.to_string();
    let args = vec!["pr", "view", &pr_str, "--repo", repo, "--json", "files"];

    let output = run_gh_subprocess(&args, token)
        .await
        .map_err(FetchError::GhCliFailure)?;

    let trimmed = output.trim();
    if trimmed.is_empty() {
        warn!(
            pr_number,
            repo, "fetch_pr_files: empty output from gh pr view --json files"
        );
        return Ok(Vec::new());
    }

    let envelope: PrFilesEnvelope = serde_json::from_str(trimmed)
        .map_err(|e| FetchError::ParseError(format!("gh pr view --json files: {e}")))?;

    let mut paths: Vec<String> = envelope.files.into_iter().map(|f| f.path).collect();
    paths.sort();
    paths.dedup();
    Ok(paths)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_error_display() {
        let e = FetchError::ParseError("boom".into());
        assert_eq!(format!("{e}"), "parse error: boom");
    }

    #[test]
    fn gh_cli_failure_display() {
        let e = FetchError::GhCliFailure("Exit code: 1\nboom".into());
        assert!(format!("{e}").contains("gh CLI failure"));
    }

    // Envelope-parsing shape test — pins the gh JSON contract we depend on.
    #[test]
    fn parse_envelope_from_gh_pr_view_files_shape() {
        let sample = r#"{"files":[{"path":"a/b.rs"},{"path":"c.md"}]}"#;
        let env: PrFilesEnvelope = serde_json::from_str(sample).unwrap();
        assert_eq!(env.files.len(), 2);
        assert_eq!(env.files[0].path, "a/b.rs");
        assert_eq!(env.files[1].path, "c.md");
    }

    #[test]
    fn parse_envelope_empty_files_array() {
        let sample = r#"{"files":[]}"#;
        let env: PrFilesEnvelope = serde_json::from_str(sample).unwrap();
        assert!(env.files.is_empty());
    }
}
