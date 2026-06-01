use regex::Regex;
use std::sync::LazyLock;

/// Parsed PR review event from gateway-formatted text.
#[derive(Debug, Clone)]
pub(crate) struct PrReviewEvent {
    pub state: String,
    pub repo: String,
    pub pr_number: u64,
    #[allow(dead_code)] // Parsed for completeness; used in Debug output and tests
    pub title: String,
    pub reviewer: String,
    pub review_url: String,
    pub body: String,
}

/// Review depth parsed from a verdict body (mika#275).
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum ReviewDepth {
    CodeLevel,
    CodeLevelPartial,
    MetadataOnly,
    /// DEPTH line was present but had an unrecognized value.
    Unknown(String),
}

impl PrReviewEvent {
    /// Construct the PR HTML URL from repo and number.
    pub fn pr_url(&self) -> String {
        format!("https://github.com/{}/pull/{}", self.repo, self.pr_number)
    }
}

/// Parsed verdict from a review body.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Verdict {
    Pass,
    Block(String),
    Hold(String),
    Missing { truncated: bool },
}

static HEADER_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^\[GitHub\] PR review \(([^)]+)\) on ([^#]+)#(\d+) \((.+)\) by @(\S+)$")
        .expect("header regex")
});

static VERDICT_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?mi)^VERDICT:\s*(.+)$").expect("verdict regex"));

static DEPTH_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?mi)^DEPTH:\s*(.+)$").expect("depth regex"));

static BLOCK_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^block\[([^\]]+)\]$").expect("block regex"));

static HOLD_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^hold\[([^\]]+)\]$").expect("hold regex"));

/// Parse a gateway-formatted PR review event into structured data.
///
/// Returns `None` if the text does not match the expected format.
pub(crate) fn parse_pr_review_event(text: &str) -> Option<PrReviewEvent> {
    let mut lines = text.lines();

    let first_line = lines.next()?;
    let caps = HEADER_RE.captures(first_line)?;

    let state = caps[1].to_string();
    let repo = caps[2].to_string();
    let pr_number: u64 = caps[3].parse().ok()?;
    let title = caps[4].to_string();
    let reviewer = caps[5].to_string();

    let review_url = lines
        .next()
        .map(|l| l.trim().to_string())
        .unwrap_or_default();

    // Body is everything after the first blank line
    let body = {
        // Skip until we find a blank line
        let mut found_blank = false;
        let mut body_lines = Vec::new();
        for line in lines {
            if !found_blank {
                if line.trim().is_empty() {
                    found_blank = true;
                }
            } else {
                body_lines.push(line);
            }
        }
        body_lines.join("\n")
    };

    Some(PrReviewEvent {
        state,
        repo,
        pr_number,
        title,
        reviewer,
        review_url,
        body,
    })
}

/// Parse a verdict from a review body.
pub(crate) fn parse_verdict(body: &str) -> Verdict {
    if let Some(caps) = VERDICT_RE.captures(body) {
        let value = caps[1].trim();

        if value.eq_ignore_ascii_case("pass") {
            return Verdict::Pass;
        }

        if let Some(bcaps) = BLOCK_RE.captures(value) {
            return Verdict::Block(bcaps[1].to_string());
        }

        if let Some(hcaps) = HOLD_RE.captures(value) {
            return Verdict::Hold(hcaps[1].to_string());
        }

        // Unrecognized verdict value
        return Verdict::Missing {
            truncated: body.contains("[truncated]"),
        };
    }

    Verdict::Missing {
        truncated: body.contains("[truncated]"),
    }
}

/// Parse review depth from a verdict body (mika#275).
///
/// Returns `None` if no `DEPTH:` line is present (backward compat).
pub(crate) fn parse_review_depth(body: &str) -> Option<ReviewDepth> {
    let caps = DEPTH_RE.captures(body)?;
    let value = caps[1].trim();
    Some(match value.to_lowercase().as_str() {
        "code-level" => ReviewDepth::CodeLevel,
        "code-level (partial)" => ReviewDepth::CodeLevelPartial,
        "metadata-only" => ReviewDepth::MetadataOnly,
        _ => ReviewDepth::Unknown(value.to_string()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_APPROVED: &str = "\
[GitHub] PR review (approved) on senara-solutions/mika#522 (fix: something) by @mika-qa
https://github.com/senara-solutions/mika/pull/522#pullrequestreview-12345

VERDICT: pass

Some review body text here...";

    #[test]
    fn parse_pr_review_approved_with_verdict_pass() {
        let event = parse_pr_review_event(SAMPLE_APPROVED).unwrap();
        assert_eq!(event.state, "approved");
        assert_eq!(event.repo, "senara-solutions/mika");
        assert_eq!(event.pr_number, 522);
        assert_eq!(event.title, "fix: something");
        assert_eq!(event.reviewer, "mika-qa");
        assert_eq!(
            event.review_url,
            "https://github.com/senara-solutions/mika/pull/522#pullrequestreview-12345"
        );
        assert_eq!(
            event.pr_url(),
            "https://github.com/senara-solutions/mika/pull/522"
        );
        assert!(event.body.contains("VERDICT: pass"));
        assert!(event.body.contains("Some review body text here..."));

        let verdict = parse_verdict(&event.body);
        assert_eq!(verdict, Verdict::Pass);
    }

    #[test]
    fn parse_pr_review_block_ci() {
        let body = "VERDICT: block[ci]\n\nCI checks are failing.";
        let verdict = parse_verdict(body);
        assert_eq!(verdict, Verdict::Block("ci".to_string()));
    }

    #[test]
    fn parse_pr_review_hold_review() {
        let body = "VERDICT: hold[review]\n\nNeeds another look.";
        let verdict = parse_verdict(body);
        assert_eq!(verdict, Verdict::Hold("review".to_string()));
    }

    #[test]
    fn parse_verdict_missing() {
        let body = "No verdict line here, just comments.";
        let verdict = parse_verdict(body);
        assert_eq!(verdict, Verdict::Missing { truncated: false });
    }

    #[test]
    fn parse_verdict_case_insensitive() {
        let body = "verdict: PASS";
        let verdict = parse_verdict(body);
        assert_eq!(verdict, Verdict::Pass);

        let body2 = "VERDICT: Block[Security]";
        let verdict2 = parse_verdict(body2);
        assert_eq!(verdict2, Verdict::Block("Security".to_string()));

        let body3 = "VERDICT: HOLD[Review]";
        let verdict3 = parse_verdict(body3);
        assert_eq!(verdict3, Verdict::Hold("Review".to_string()));
    }

    #[test]
    fn parse_verdict_no_space_after_colon() {
        let body = "VERDICT:pass";
        let verdict = parse_verdict(body);
        assert_eq!(verdict, Verdict::Pass);
    }

    #[test]
    fn parse_verdict_multiple_lines_first_wins() {
        let body = "VERDICT: pass\nVERDICT: block[ci]";
        let verdict = parse_verdict(body);
        assert_eq!(verdict, Verdict::Pass);
    }

    #[test]
    fn parse_non_review_event_returns_none() {
        let text = "This is just a regular message, not a PR review event.";
        assert!(parse_pr_review_event(text).is_none());
    }

    #[test]
    fn parse_review_changes_requested() {
        let text = "\
[GitHub] PR review (changes_requested) on senara-solutions/mika#100 (feat: add feature) by @reviewer
https://github.com/senara-solutions/mika/pull/100#pullrequestreview-99999

VERDICT: block[pipeline]

Please fix the pipeline issues.";

        let event = parse_pr_review_event(text).unwrap();
        assert_eq!(event.state, "changes_requested");
        assert_eq!(event.pr_number, 100);
        assert_eq!(event.reviewer, "reviewer");

        let verdict = parse_verdict(&event.body);
        assert_eq!(verdict, Verdict::Block("pipeline".to_string()));
    }

    #[test]
    fn parse_verdict_truncated_body_missing() {
        let body = "Some text...\n[truncated]";
        let verdict = parse_verdict(body);
        assert_eq!(verdict, Verdict::Missing { truncated: true });
    }

    #[test]
    fn parse_malformed_first_line() {
        let text = "[GitHub] PR review on something broken";
        assert!(parse_pr_review_event(text).is_none());

        let text2 = "[GitHub] PR review (approved) on repo#notanumber (title) by @user";
        assert!(parse_pr_review_event(text2).is_none());
    }

    // --- Review depth parsing tests (mika#275) ---

    #[test]
    fn parse_depth_code_level() {
        let body = "VERDICT: pass\nDEPTH: code-level\nREASON: all good";
        assert_eq!(parse_review_depth(body), Some(ReviewDepth::CodeLevel));
    }

    #[test]
    fn parse_depth_code_level_partial() {
        let body = "VERDICT: pass\nDEPTH: code-level (partial)\nREASON: truncated diff";
        assert_eq!(
            parse_review_depth(body),
            Some(ReviewDepth::CodeLevelPartial)
        );
    }

    #[test]
    fn parse_depth_metadata_only() {
        let body = "VERDICT: hold[review]\nDEPTH: metadata-only\nREASON: diff unavailable";
        assert_eq!(parse_review_depth(body), Some(ReviewDepth::MetadataOnly));
    }

    #[test]
    fn parse_depth_missing_returns_none() {
        let body = "VERDICT: pass\nREASON: all good";
        assert_eq!(parse_review_depth(body), None);
    }

    #[test]
    fn parse_depth_case_insensitive() {
        let body = "DEPTH: Code-Level";
        assert_eq!(parse_review_depth(body), Some(ReviewDepth::CodeLevel));

        let body2 = "DEPTH: METADATA-ONLY";
        assert_eq!(parse_review_depth(body2), Some(ReviewDepth::MetadataOnly));
    }

    #[test]
    fn parse_depth_unknown_value() {
        let body = "DEPTH: something-else";
        assert_eq!(
            parse_review_depth(body),
            Some(ReviewDepth::Unknown("something-else".to_string()))
        );
    }

    #[test]
    fn parse_depth_first_match_wins() {
        let body = "DEPTH: code-level\nDEPTH: metadata-only";
        assert_eq!(parse_review_depth(body), Some(ReviewDepth::CodeLevel));
    }
}
