//! Git command wrappers for marketplace skill installation.
//!
//! All git operations shell out to the `git` binary. Environment variables
//! prefixed with `MIKA_` are scrubbed from child processes (defense-in-depth).

use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result, bail};
use tempfile::TempDir;

/// Check if git is available on PATH. Returns the version string.
pub fn check_git() -> Result<String> {
    let output = git_command().arg("--version").output().context(
        "git is not installed or not found on PATH. Install git to use marketplace skills",
    )?;

    if !output.status.success() {
        bail!(
            "git --version failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// Clone a repo to a temp directory using a shallow clone (--depth=1).
///
/// Returns the `TempDir` which is automatically cleaned up on drop.
pub fn clone_to_temp(url: &str) -> Result<TempDir> {
    let tmp = TempDir::new().context("failed to create temp directory for clone")?;

    let output = git_command()
        .args(["clone", "--depth", "1", url])
        .arg(tmp.path())
        .output()
        .context("failed to run git clone")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("git clone failed: {stderr}");
    }

    Ok(tmp)
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
/// - `https://...` or `http://...` → pass through
/// - `git@...` → pass through (SSH URL)
/// - `ssh://...` → pass through
/// - `user/repo` (exactly one `/`, no protocol) → `https://github.com/user/repo.git`
/// - Everything else → error
pub fn resolve_url(source: &str) -> Result<String> {
    let source = source.trim();

    if source.is_empty() {
        bail!("source URL cannot be empty");
    }

    // Pass through full URLs
    if source.starts_with("https://")
        || source.starts_with("http://")
        || source.starts_with("ssh://")
        || source.starts_with("git@")
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

    // Scrub MIKA_* env vars from the child process (defense-in-depth)
    for (key, _) in std::env::vars() {
        if key.starts_with("MIKA_") {
            cmd.env_remove(&key);
        }
    }

    cmd
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- resolve_url tests ---

    #[test]
    fn test_resolve_github_shorthand() {
        let url = resolve_url("user/repo").unwrap();
        assert_eq!(url, "https://github.com/user/repo.git");
    }

    #[test]
    fn test_resolve_github_shorthand_trimmed() {
        let url = resolve_url("  user/repo  ").unwrap();
        assert_eq!(url, "https://github.com/user/repo.git");
    }

    #[test]
    fn test_resolve_https_passthrough() {
        let url = resolve_url("https://github.com/user/repo.git").unwrap();
        assert_eq!(url, "https://github.com/user/repo.git");
    }

    #[test]
    fn test_resolve_http_passthrough() {
        let url = resolve_url("http://example.com/repo.git").unwrap();
        assert_eq!(url, "http://example.com/repo.git");
    }

    #[test]
    fn test_resolve_ssh_passthrough() {
        let url = resolve_url("ssh://git@github.com/user/repo.git").unwrap();
        assert_eq!(url, "ssh://git@github.com/user/repo.git");
    }

    #[test]
    fn test_resolve_git_at_passthrough() {
        let url = resolve_url("git@github.com:user/repo.git").unwrap();
        assert_eq!(url, "git@github.com:user/repo.git");
    }

    #[test]
    fn test_resolve_empty_error() {
        assert!(resolve_url("").is_err());
        assert!(resolve_url("  ").is_err());
    }

    #[test]
    fn test_resolve_invalid_single_word() {
        assert!(resolve_url("justrepo").is_err());
    }

    #[test]
    fn test_resolve_domain_with_slash_not_shorthand() {
        // "example.com/repo" has a dot before the slash, so it's not GitHub shorthand
        assert!(resolve_url("example.com/repo").is_err());
    }

    #[test]
    fn test_resolve_triple_path_not_shorthand() {
        // "user/repo/extra" has more than one slash
        assert!(resolve_url("user/repo/extra").is_err());
    }

    // --- check_git test ---

    #[test]
    fn test_check_git_available() {
        // This test assumes git is installed (it is in the Docker image and on dev machines)
        let version = check_git().unwrap();
        assert!(version.starts_with("git version"), "got: {version}");
    }
}
