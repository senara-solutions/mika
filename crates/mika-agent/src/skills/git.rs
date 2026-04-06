//! Git command wrappers and source resolution for marketplace skill installation.
//!
//! All git operations shell out to the `git` binary. Environment variables
//! prefixed with `MIKA_` are scrubbed from child processes (defense-in-depth).

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};
use tempfile::TempDir;

/// Discriminant for skill install source types.
#[derive(Debug, Clone, PartialEq)]
pub enum SourceKind {
    /// Remote git repository URL (https://, ssh://, git@, or GitHub shorthand).
    Git(String),
    /// Local filesystem path (absolute, canonicalized).
    Local(PathBuf),
}

/// Resolve a source string into either a git URL or a local filesystem path.
///
/// Resolution order:
/// 1. `file://` prefix → strip and treat as local path
/// 2. Absolute path that exists on disk → local path
/// 3. Everything else → delegate to git URL resolution
pub fn resolve_source(input: &str) -> Result<SourceKind> {
    let input = input.trim();

    if input.is_empty() {
        bail!("source cannot be empty");
    }

    // Handle file:// URIs
    if let Some(path) = input.strip_prefix("file://") {
        let p = PathBuf::from(path);
        if !p.is_absolute() {
            bail!("file:// URI must use an absolute path, got: {path}");
        }
        if !p.exists() {
            bail!("path does not exist: {}", p.display());
        }
        return Ok(SourceKind::Local(std::fs::canonicalize(&p).with_context(
            || format!("failed to canonicalize {}", p.display()),
        )?));
    }

    // Handle bare absolute paths
    let p = Path::new(input);
    if p.is_absolute() && p.exists() {
        return Ok(SourceKind::Local(std::fs::canonicalize(p).with_context(
            || format!("failed to canonicalize {}", p.display()),
        )?));
    }

    // Fall through to git URL resolution
    Ok(SourceKind::Git(resolve_git_url(input)?))
}

/// Check if git is available on PATH.
pub fn check_git() -> Result<()> {
    let output = git_command().arg("--version").output().context(
        "git is not installed or not found on PATH. Install git to use marketplace skills",
    )?;

    if !output.status.success() {
        bail!(
            "git --version failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    Ok(())
}

/// Clone a repo to a temp directory using a shallow clone (--depth=1).
///
/// When `github_token` is provided and the URL is an HTTPS GitHub URL,
/// the token is injected into the URL for authentication (private repos).
///
/// Returns the `TempDir` which is automatically cleaned up on drop.
pub fn clone_to_temp(url: &str, github_token: Option<&str>) -> Result<TempDir> {
    let tmp = TempDir::new().context("failed to create temp directory for clone")?;

    let effective_url = match github_token {
        Some(token) => inject_github_token(url, token),
        None => url.to_string(),
    };

    let output = git_command()
        .args(["clone", "--depth", "1", &effective_url])
        .arg(tmp.path())
        .output()
        .context("failed to run git clone")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        // Use original URL in error message — never expose token
        bail!("git clone failed: {stderr}");
    }

    Ok(tmp)
}

/// Inject a GitHub token into an HTTPS GitHub URL for authentication.
///
/// Only rewrites `https://github.com/...` URLs. All other URLs are returned unchanged.
pub(crate) fn inject_github_token(url: &str, token: &str) -> String {
    if url.starts_with("https://github.com/") {
        url.replacen(
            "https://github.com/",
            &format!("https://x-access-token:{token}@github.com/"),
            1,
        )
    } else {
        url.to_string()
    }
}

/// Get the HEAD commit hash from a cloned repository.
pub fn get_head_commit(repo_dir: &Path) -> Result<String> {
    let output = git_command()
        .args(["rev-parse", "HEAD"])
        .current_dir(repo_dir)
        .output()
        .context("failed to run git rev-parse HEAD")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("git rev-parse HEAD failed: {stderr}");
    }

    let hash = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if hash.len() < 40 || !hash.chars().all(|c| c.is_ascii_hexdigit()) {
        bail!("unexpected git rev-parse output: {hash}");
    }

    Ok(hash)
}

/// Resolve a source string into a full git clone URL.
///
/// Rules:
/// - `http://...` → rejected (insecure, use https://)
/// - `https://...` → pass through
/// - `git@...` → pass through (SSH URL)
/// - `ssh://...` → pass through
/// - `user/repo` (exactly one `/`, no protocol) → `https://github.com/user/repo.git`
/// - Everything else → error
fn resolve_git_url(source: &str) -> Result<String> {
    let source = source.trim();

    if source.is_empty() {
        bail!("source URL cannot be empty");
    }

    // Reject insecure HTTP URLs
    if source.starts_with("http://") {
        bail!(
            "Insecure URL: '{source}'. Use https:// instead. \
             Plain HTTP is vulnerable to man-in-the-middle attacks."
        );
    }

    // Pass through full URLs
    if source.starts_with("https://") || source.starts_with("ssh://") || source.starts_with("git@")
    {
        return Ok(source.to_string());
    }

    // GitHub shorthand: user/repo (exactly one slash, no dots before it which would indicate a domain)
    let parts: Vec<&str> = source.splitn(3, '/').collect();
    if parts.len() == 2
        && !parts[0].is_empty()
        && !parts[1].is_empty()
        && !parts[0].contains('.')
        && !parts[0].contains(':')
    {
        let url = format!("https://github.com/{}.git", source);
        return Ok(url);
    }

    bail!(
        "Invalid source: '{source}'. Use a full URL (https://...) or GitHub shorthand (user/repo)."
    );
}

/// Create a `Command` for git with MIKA_* env vars scrubbed.
fn git_command() -> Command {
    let mut cmd = Command::new("git");

    // Prevent git from prompting for credentials on private repos, which would
    // hang the CLI indefinitely. Git will fail immediately instead.
    cmd.env("GIT_TERMINAL_PROMPT", "0");

    // Scrub MIKA_* env vars from the child process (defense-in-depth)
    super::executor::scrub_mika_env_vars_std(&mut cmd);

    cmd
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    // --- resolve_git_url tests ---

    #[test]
    fn test_resolve_github_shorthand() {
        let url = resolve_git_url("user/repo").unwrap();
        assert_eq!(url, "https://github.com/user/repo.git");
    }

    #[test]
    fn test_resolve_github_shorthand_trimmed() {
        let url = resolve_git_url("  user/repo  ").unwrap();
        assert_eq!(url, "https://github.com/user/repo.git");
    }

    #[test]
    fn test_resolve_https_passthrough() {
        let url = resolve_git_url("https://github.com/user/repo.git").unwrap();
        assert_eq!(url, "https://github.com/user/repo.git");
    }

    #[test]
    fn test_resolve_http_rejected() {
        let result = resolve_git_url("http://example.com/repo.git");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Insecure"));
    }

    #[test]
    fn test_resolve_ssh_passthrough() {
        let url = resolve_git_url("ssh://git@github.com/user/repo.git").unwrap();
        assert_eq!(url, "ssh://git@github.com/user/repo.git");
    }

    #[test]
    fn test_resolve_git_at_passthrough() {
        let url = resolve_git_url("git@github.com:user/repo.git").unwrap();
        assert_eq!(url, "git@github.com:user/repo.git");
    }

    #[test]
    fn test_resolve_empty_error() {
        assert!(resolve_git_url("").is_err());
        assert!(resolve_git_url("  ").is_err());
    }

    #[test]
    fn test_resolve_invalid_single_word() {
        assert!(resolve_git_url("justrepo").is_err());
    }

    #[test]
    fn test_resolve_domain_with_slash_not_shorthand() {
        // "example.com/repo" has a dot before the slash, so it's not GitHub shorthand
        assert!(resolve_git_url("example.com/repo").is_err());
    }

    #[test]
    fn test_resolve_triple_path_not_shorthand() {
        // "user/repo/extra" has more than one slash
        assert!(resolve_git_url("user/repo/extra").is_err());
    }

    // --- resolve_source tests ---

    #[test]
    fn test_resolve_source_github_shorthand() {
        let source = resolve_source("user/repo").unwrap();
        assert_eq!(
            source,
            SourceKind::Git("https://github.com/user/repo.git".to_string())
        );
    }

    #[test]
    fn test_resolve_source_https_url() {
        let source = resolve_source("https://github.com/user/repo.git").unwrap();
        assert_eq!(
            source,
            SourceKind::Git("https://github.com/user/repo.git".to_string())
        );
    }

    #[test]
    fn test_resolve_source_local_path() {
        let tmp = TempDir::new().unwrap();
        let source = resolve_source(tmp.path().to_str().unwrap()).unwrap();
        match source {
            SourceKind::Local(p) => assert!(p.exists()),
            _ => panic!("expected SourceKind::Local"),
        }
    }

    #[test]
    fn test_resolve_source_file_uri() {
        let tmp = TempDir::new().unwrap();
        let uri = format!("file://{}", tmp.path().display());
        let source = resolve_source(&uri).unwrap();
        match source {
            SourceKind::Local(p) => assert!(p.exists()),
            _ => panic!("expected SourceKind::Local"),
        }
    }

    #[test]
    fn test_resolve_source_file_uri_relative_rejected() {
        let result = resolve_source("file://relative/path");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("absolute"));
    }

    #[test]
    fn test_resolve_source_nonexistent_path() {
        let result = resolve_source("/nonexistent/path/that/does/not/exist");
        // Non-existent absolute paths fall through to git URL resolution (and fail there)
        assert!(result.is_err());
    }

    #[test]
    fn test_resolve_source_nonexistent_file_uri() {
        let result = resolve_source("file:///nonexistent/path/that/does/not/exist");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("does not exist"));
    }

    #[test]
    fn test_resolve_source_empty() {
        assert!(resolve_source("").is_err());
        assert!(resolve_source("  ").is_err());
    }

    // --- inject_github_token tests ---

    #[test]
    fn test_inject_github_token_https() {
        let url = inject_github_token("https://github.com/user/repo.git", "ghs_abc123");
        assert_eq!(
            url,
            "https://x-access-token:ghs_abc123@github.com/user/repo.git"
        );
    }

    #[test]
    fn test_inject_github_token_non_github() {
        let url = inject_github_token("https://gitlab.com/user/repo.git", "token");
        assert_eq!(url, "https://gitlab.com/user/repo.git");
    }

    #[test]
    fn test_inject_github_token_ssh_unchanged() {
        let url = inject_github_token("git@github.com:user/repo.git", "token");
        assert_eq!(url, "git@github.com:user/repo.git");
    }

    #[test]
    fn test_inject_github_token_without_dotgit() {
        let url = inject_github_token("https://github.com/user/repo", "tok");
        assert_eq!(url, "https://x-access-token:tok@github.com/user/repo");
    }

    // --- check_git test ---

    #[test]
    fn test_check_git_available() {
        // This test assumes git is installed (it is in the Docker image and on dev machines)
        check_git().unwrap();
    }
}
