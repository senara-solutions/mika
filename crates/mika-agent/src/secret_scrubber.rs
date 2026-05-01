//! Secret-shaped value scrubber for tool_calls persistence.
//!
//! Redacts known secret patterns (API keys, tokens, PEM private keys, env var
//! assignments containing secrets) from arbitrary text before it is written to
//! the `tool_calls` table. The LLM's in-memory tool output is NOT scrubbed —
//! only the durable copy is sanitized.
//!
//! # Pattern list
//!
//! | Pattern | Replacement | Example |
//! |---------|-------------|---------|
//! | `github_pat_[A-Za-z0-9_]+` | `github_pat_<REDACTED>` | GitHub fine-grained PAT |
//! | `ghp_[A-Za-z0-9]+` | `ghp_<REDACTED>` | GitHub classic PAT |
//! | `gho_[A-Za-z0-9]+` | `gho_<REDACTED>` | GitHub OAuth token |
//! | `ghs_[A-Za-z0-9]+` | `ghs_<REDACTED>` | GitHub server token |
//! | `ghu_[A-Za-z0-9]+` | `ghu_<REDACTED>` | GitHub user token |
//! | `sk-ant-(api\|oat)[A-Za-z0-9_-]+` | `sk-ant-{type}<REDACTED>` | Anthropic API/OAuth |
//! | `sk-proj-[A-Za-z0-9_-]+` | `sk-proj-<REDACTED>` | OpenAI project key |
//! | `sk-or-[A-Za-z0-9_-]+` | `sk-or-<REDACTED>` | OpenRouter key |
//! | `gsk_[A-Za-z0-9]+` | `gsk_<REDACTED>` | Groq key |
//! | `xoxb-[A-Za-z0-9-]+` | `xoxb-<REDACTED>` | Slack bot token |
//! | `xoxp-[A-Za-z0-9-]+` | `xoxp-<REDACTED>` | Slack user token |
//! | `MIKA_*{TOKEN,KEY,SECRET}=\S+` | `{name}=<REDACTED>` | Env var assignment |
//! | `GH(_APP)?_TOKEN=\S+` | `{name}=<REDACTED>` | GH_TOKEN env var |
//! | PEM private key block | `<REDACTED-PRIVATE-KEY>` | RSA/EC/etc private key |
//!
//! To add a new pattern, append a `(regex, replacement)` pair to
//! [`SECRET_PATTERNS`] and add positive + negative test cases.

use std::borrow::Cow;
use std::sync::LazyLock;

use regex::{Regex, RegexSet};

/// (pattern, replacement) pairs for known secret shapes.
///
/// The replacement string may contain capture-group backreferences (e.g. `$1`).
/// Patterns are applied in order; earlier patterns take priority when they
/// overlap with later ones on the same input region.
const SECRET_PATTERNS: &[(&str, &str)] = &[
    // GitHub fine-grained PAT (longest prefix first to avoid partial matches)
    (r"github_pat_[A-Za-z0-9_]{10,}", "github_pat_<REDACTED>"),
    // GitHub classic PAT
    (r"ghp_[A-Za-z0-9]{10,}", "ghp_<REDACTED>"),
    // GitHub OAuth token
    (r"gho_[A-Za-z0-9]{10,}", "gho_<REDACTED>"),
    // GitHub server-to-server token
    (r"ghs_[A-Za-z0-9]{10,}", "ghs_<REDACTED>"),
    // GitHub user-to-server token
    (r"ghu_[A-Za-z0-9]{10,}", "ghu_<REDACTED>"),
    // Anthropic API key or OAuth token (preserve type prefix for diagnostics)
    (
        r"sk-ant-(api|oat)[A-Za-z0-9_-]{10,}",
        "sk-ant-$1-<REDACTED>",
    ),
    // OpenAI project key
    (r"sk-proj-[A-Za-z0-9_-]{10,}", "sk-proj-<REDACTED>"),
    // OpenRouter key
    (r"sk-or-[A-Za-z0-9_-]{10,}", "sk-or-<REDACTED>"),
    // Groq key
    (r"gsk_[A-Za-z0-9]{10,}", "gsk_<REDACTED>"),
    // Slack bot token
    (r"xoxb-[A-Za-z0-9-]{10,}", "xoxb-<REDACTED>"),
    // Slack user token
    (r"xoxp-[A-Za-z0-9-]{10,}", "xoxp-<REDACTED>"),
    // PEM private key blocks (multi-line, greedy within block)
    (
        r"-----BEGIN [A-Z ]*PRIVATE KEY-----[\s\S]*?-----END [A-Z ]*PRIVATE KEY-----",
        "<REDACTED-PRIVATE-KEY>",
    ),
    // Env var assignments: MIKA_*TOKEN, MIKA_*KEY, MIKA_*SECRET
    // Matches both quoted and unquoted values, stops at whitespace or quote boundary.
    (
        r#"(MIKA_[A-Z_]*(?:TOKEN|KEY|SECRET))="?([^\s"]+)"?"#,
        "$1=<REDACTED>",
    ),
    // GH_TOKEN / GH_APP_TOKEN env var assignments
    (r#"(GH(?:_APP)?_TOKEN)="?([^\s"]+)"?"#, "$1=<REDACTED>"),
];

/// Fast-rejection set: if none of these patterns match, we skip individual
/// regex replacement entirely. Built from the same pattern strings as
/// [`SECRET_PATTERNS`].
static FAST_REJECT: LazyLock<RegexSet> = LazyLock::new(|| {
    let patterns: Vec<&str> = SECRET_PATTERNS.iter().map(|(p, _)| *p).collect();
    RegexSet::new(patterns).expect("SECRET_PATTERNS contains invalid regex")
});

/// Individual compiled regexes, parallel to [`SECRET_PATTERNS`].
static INDIVIDUAL_REGEXES: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    SECRET_PATTERNS
        .iter()
        .map(|(p, _)| Regex::new(p).expect("SECRET_PATTERNS contains invalid regex"))
        .collect()
});

/// Scrub known secret-shaped values from `input`.
///
/// Returns `Cow::Borrowed` when no secrets are detected (the common case,
/// zero allocation). Returns `Cow::Owned` with redacted values when at least
/// one pattern matches.
///
/// This function is infallible — regex replacement never panics on valid UTF-8.
pub fn scrub_secrets(input: &str) -> Cow<'_, str> {
    if input.is_empty() || !FAST_REJECT.is_match(input) {
        return Cow::Borrowed(input);
    }

    let mut result = input.to_owned();
    for idx in FAST_REJECT.matches(input).iter() {
        let (_, replacement) = SECRET_PATTERNS[idx];
        result = INDIVIDUAL_REGEXES[idx]
            .replace_all(&result, replacement)
            .into_owned();
    }
    Cow::Owned(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- Happy path: each pattern individually ----

    #[test]
    fn scrubs_github_pat() {
        let input = "token=github_pat_11CBQ5ABC1234567890abcdef";
        let result = scrub_secrets(input);
        assert_eq!(result, "token=github_pat_<REDACTED>");
        assert!(matches!(result, Cow::Owned(_)));
    }

    #[test]
    fn scrubs_ghp_token() {
        let input = "token=ghp_ABCDEFghij1234567890";
        let result = scrub_secrets(input);
        assert_eq!(result, "token=ghp_<REDACTED>");
    }

    #[test]
    fn scrubs_gho_token() {
        let input = "oauth=gho_ABCDEFghij1234567890";
        let result = scrub_secrets(input);
        assert_eq!(result, "oauth=gho_<REDACTED>");
    }

    #[test]
    fn scrubs_ghs_token() {
        let input = "server=ghs_ABCDEFghij1234567890";
        let result = scrub_secrets(input);
        assert_eq!(result, "server=ghs_<REDACTED>");
    }

    #[test]
    fn scrubs_ghu_token() {
        let input = "user=ghu_ABCDEFghij1234567890";
        let result = scrub_secrets(input);
        assert_eq!(result, "user=ghu_<REDACTED>");
    }

    #[test]
    fn scrubs_anthropic_api_key() {
        let input = "key=sk-ant-api03-abcdefghij1234567890";
        let result = scrub_secrets(input);
        assert_eq!(result, "key=sk-ant-api-<REDACTED>");
    }

    #[test]
    fn scrubs_anthropic_oauth_token() {
        let input = "token=sk-ant-oat01-abcdefghij1234567890";
        let result = scrub_secrets(input);
        assert_eq!(result, "token=sk-ant-oat-<REDACTED>");
    }

    #[test]
    fn scrubs_openai_project_key() {
        let input = "OPENAI_KEY=sk-proj-abcdefghij1234567890";
        let result = scrub_secrets(input);
        assert_eq!(result, "OPENAI_KEY=sk-proj-<REDACTED>");
    }

    #[test]
    fn scrubs_openrouter_key() {
        let input = "key=sk-or-v1-abcdefghij1234567890";
        let result = scrub_secrets(input);
        assert_eq!(result, "key=sk-or-<REDACTED>");
    }

    #[test]
    fn scrubs_groq_key() {
        let input = "GROQ=gsk_ABCDEFghij1234567890";
        let result = scrub_secrets(input);
        assert_eq!(result, "GROQ=gsk_<REDACTED>");
    }

    #[test]
    fn scrubs_slack_bot_token() {
        let input = "SLACK_TOKEN=xoxb-1234567890-abcdefghij";
        let result = scrub_secrets(input);
        assert_eq!(result, "SLACK_TOKEN=xoxb-<REDACTED>");
    }

    #[test]
    fn scrubs_slack_user_token() {
        let input = "SLACK_USER=xoxp-1234567890-abcdefghij";
        let result = scrub_secrets(input);
        assert_eq!(result, "SLACK_USER=xoxp-<REDACTED>");
    }

    #[test]
    fn scrubs_pem_private_key() {
        let input = "before\n-----BEGIN RSA PRIVATE KEY-----\nMIIE...base64...\n-----END RSA PRIVATE KEY-----\nafter";
        let result = scrub_secrets(input);
        assert_eq!(result, "before\n<REDACTED-PRIVATE-KEY>\nafter");
    }

    #[test]
    fn scrubs_ec_private_key() {
        let input = "-----BEGIN EC PRIVATE KEY-----\ndata\n-----END EC PRIVATE KEY-----";
        let result = scrub_secrets(input);
        assert_eq!(result, "<REDACTED-PRIVATE-KEY>");
    }

    #[test]
    fn scrubs_mika_env_var_token() {
        let input = "MIKA_GITHUB_TOKEN=github_pat_11CBQ5ABC1234567890abcdef";
        let result = scrub_secrets(input);
        assert_eq!(result, "MIKA_GITHUB_TOKEN=<REDACTED>");
    }

    #[test]
    fn scrubs_mika_env_var_key() {
        let input = "MIKA_ANTHROPIC_API_KEY=sk-ant-api03-xyz";
        let result = scrub_secrets(input);
        assert_eq!(result, "MIKA_ANTHROPIC_API_KEY=<REDACTED>");
    }

    #[test]
    fn scrubs_mika_env_var_secret() {
        let input = "MIKA_INTERNAL_SECRET=super_secret_value";
        let result = scrub_secrets(input);
        assert_eq!(result, "MIKA_INTERNAL_SECRET=<REDACTED>");
    }

    #[test]
    fn scrubs_mika_env_var_quoted() {
        let input = r#"MIKA_GITHUB_TOKEN="github_pat_11CBQ5ABC1234567890abcdef""#;
        let result = scrub_secrets(input);
        assert_eq!(result, "MIKA_GITHUB_TOKEN=<REDACTED>");
    }

    #[test]
    fn scrubs_gh_token_env_var() {
        let input = "GH_TOKEN=ghp_ABCDEFghij1234567890";
        let result = scrub_secrets(input);
        // The raw secret must be gone regardless of which pattern wins
        assert!(
            !result.contains("ghp_ABCDEFghij"),
            "raw secret must be redacted: {result}"
        );
        assert!(
            result.contains("<REDACTED>"),
            "should have redaction marker: {result}"
        );
    }

    #[test]
    fn scrubs_gh_app_token_env_var() {
        let input = "GH_APP_TOKEN=some_token_value";
        let result = scrub_secrets(input);
        assert_eq!(result, "GH_APP_TOKEN=<REDACTED>");
    }

    // ---- Happy path: multiple secrets in one string ----

    #[test]
    fn scrubs_multiple_secrets() {
        let input = "MIKA_GITHUB_TOKEN=github_pat_11CBQ5ABC1234567890abcdef\nMIKA_ANTHROPIC_API_KEY=sk-ant-api03-abcdefghij1234567890\nGH_TOKEN=ghp_ABCDEFghij1234567890";
        let result = scrub_secrets(input);
        assert!(!result.contains("github_pat_11CBQ5"));
        assert!(!result.contains("sk-ant-api03-abcdefghij"));
        assert!(!result.contains("ghp_ABCDEFghij"));
        assert!(result.contains("<REDACTED>"));
    }

    // ---- Happy path: actual incident shape ----

    #[test]
    fn scrubs_actual_incident_shape() {
        let input = r#"Contents of '.env':

MIKA_GITHUB_TOKEN="github_pat_11CBQ5ABC1234567890abcdef"
MIKA_ANTHROPIC_API_KEY="sk-ant-api03-abcdefghij1234567890"
GH_TOKEN="ghp_ABCDEFghij1234567890"
"#;
        let result = scrub_secrets(input);
        assert!(!result.contains("github_pat_11CBQ5"));
        assert!(!result.contains("sk-ant-api03-abcdefghij"));
        assert!(!result.contains("ghp_ABCDEFghij"));
    }

    // ---- Edge case: no secrets ----

    #[test]
    fn no_secrets_returns_borrowed() {
        let input = "This is a normal tool output with no secrets.";
        let result = scrub_secrets(input);
        assert!(matches!(result, Cow::Borrowed(_)));
        assert_eq!(result, input);
    }

    #[test]
    fn empty_string_returns_borrowed() {
        let result = scrub_secrets("");
        assert!(matches!(result, Cow::Borrowed(_)));
        assert_eq!(result, "");
    }

    // ---- Edge case: near-match but not matching ----

    #[test]
    fn short_prefix_not_matched() {
        // Patterns require at least 10 chars after prefix to avoid false positives
        let input = "ghp_short";
        let result = scrub_secrets(input);
        assert!(matches!(result, Cow::Borrowed(_)));
        assert_eq!(result, "ghp_short");
    }

    #[test]
    fn prefix_only_not_matched() {
        let input = "sk-ant- without suffix";
        let result = scrub_secrets(input);
        assert!(matches!(result, Cow::Borrowed(_)));
    }

    #[test]
    fn partial_prefix_not_matched() {
        let input = "github_pat without underscore suffix";
        let result = scrub_secrets(input);
        // "github_pat" is only 10 chars, needs 10+ alphanumeric after prefix
        assert!(matches!(result, Cow::Borrowed(_)));
    }

    // ---- Edge case: env example placeholders (defensive redaction is acceptable) ----

    #[test]
    fn env_example_placeholder_redacted() {
        let input = "MIKA_API_KEY=your_key_here";
        let result = scrub_secrets(input);
        assert_eq!(result, "MIKA_API_KEY=<REDACTED>");
    }

    // ---- Edge case: non-secret MIKA_ env vars not redacted ----

    #[test]
    fn mika_non_secret_env_var_not_redacted() {
        let input = "MIKA_LOG_FORMAT=json";
        let result = scrub_secrets(input);
        // MIKA_LOG_FORMAT does not end in TOKEN, KEY, or SECRET
        assert!(matches!(result, Cow::Borrowed(_)));
        assert_eq!(result, "MIKA_LOG_FORMAT=json");
    }

    #[test]
    fn mika_dev_mode_not_redacted() {
        let input = "MIKA_DEV_MODE=true";
        let result = scrub_secrets(input);
        assert!(matches!(result, Cow::Borrowed(_)));
    }
}
