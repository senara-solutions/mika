use anyhow::Result;
use std::io::BufRead;

/// Run the `mika credential-helper` subcommand.
///
/// Implements the git credential helper protocol. Git invokes this as:
///   `mika credential-helper get`    — return credentials for HTTPS
///   `mika credential-helper store`  — no-op (we don't persist)
///   `mika credential-helper erase`  — no-op (nothing to erase)
///
/// Lightweight path: loads dotenv + Settings + GitHubApp only.
/// No tracing, no DB, no agent resolution.
///
/// **Security:** Only responds for `github.com` HTTPS requests. All other
/// hosts are silently ignored (exit 0, no output) to prevent token leakage.
///
/// **Error handling:** Returns exit 0 with no output on any failure, allowing
/// git to fall through to the next credential source. This preserves backward
/// compatibility (SSH, system keychain, etc.).
pub async fn run(operation: &str, home_dir: &std::path::Path) -> Result<()> {
    // Only `get` produces output; `store` and `erase` are silent no-ops
    if operation != "get" {
        return Ok(());
    }

    // Parse git credential protocol from stdin
    let stdin = std::io::stdin();
    let (protocol, host) = parse_credential_request(stdin.lock())?;

    // Security filter: only respond for github.com HTTPS
    if protocol.as_deref() != Some("https") || host.as_deref() != Some("github.com") {
        return Ok(());
    }

    // Attempt to get an installation token — any failure is silent (exit 0)
    match get_installation_token(home_dir).await {
        Some(token) => {
            println!("protocol=https");
            println!("host=github.com");
            println!("username=x-access-token");
            println!("password={token}");
        }
        None => {
            // Silent exit — git falls through to next credential source
        }
    }

    Ok(())
}

/// Parse the git credential protocol request from a reader.
///
/// Git sends key=value pairs, one per line, terminated by an empty line.
/// We extract `protocol` and `host` fields.
fn parse_credential_request(reader: impl BufRead) -> Result<(Option<String>, Option<String>)> {
    let mut protocol = None;
    let mut host = None;

    for line in reader.lines() {
        let line = line?;
        if line.is_empty() {
            break;
        }
        if let Some((key, value)) = line.split_once('=') {
            match key {
                "protocol" => protocol = Some(value.to_string()),
                "host" => host = Some(value.to_string()),
                _ => {} // Ignore unknown fields (path, username, etc.)
            }
        }
    }

    Ok((protocol, host))
}

/// Attempt to get a GitHub App installation token.
/// Returns `None` on any failure (config missing, network error, etc.).
async fn get_installation_token(home_dir: &std::path::Path) -> Option<String> {
    let settings = mika_common::config::Settings::load(home_dir).ok()?;
    let github_app = mika_common::github_app::GitHubApp::from_settings(&settings)?;
    let cache_path = home_dir.join("github_app_token.json");
    github_app
        .installation_token_with_file_cache(&cache_path)
        .await
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn test_parse_credential_request_basic() {
        let input = Cursor::new("protocol=https\nhost=github.com\n\n");
        let (protocol, host) = parse_credential_request(input).unwrap();
        assert_eq!(protocol.as_deref(), Some("https"));
        assert_eq!(host.as_deref(), Some("github.com"));
    }

    #[test]
    fn test_parse_credential_request_with_path() {
        let input =
            Cursor::new("protocol=https\nhost=github.com\npath=senara-solutions/mika.git\n\n");
        let (protocol, host) = parse_credential_request(input).unwrap();
        assert_eq!(protocol.as_deref(), Some("https"));
        assert_eq!(host.as_deref(), Some("github.com"));
    }

    #[test]
    fn test_parse_credential_request_non_github() {
        let input = Cursor::new("protocol=https\nhost=gitlab.com\n\n");
        let (protocol, host) = parse_credential_request(input).unwrap();
        assert_eq!(protocol.as_deref(), Some("https"));
        assert_eq!(host.as_deref(), Some("gitlab.com"));
    }

    #[test]
    fn test_parse_credential_request_ssh() {
        let input = Cursor::new("protocol=ssh\nhost=github.com\n\n");
        let (protocol, host) = parse_credential_request(input).unwrap();
        assert_eq!(protocol.as_deref(), Some("ssh"));
        assert_eq!(host.as_deref(), Some("github.com"));
    }

    #[test]
    fn test_parse_credential_request_empty() {
        let input = Cursor::new("\n");
        let (protocol, host) = parse_credential_request(input).unwrap();
        assert!(protocol.is_none());
        assert!(host.is_none());
    }
}
