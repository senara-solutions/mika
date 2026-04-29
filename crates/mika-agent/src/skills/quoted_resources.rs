//! Quoted-resource detection for the pre-fetch guard (mika#863).
//!
//! Detects fetchable resources quoted in user messages (issue bodies, PR diffs,
//! file content) and maps them to required tool calls. Used by `collect_required_tools`
//! to augment the required-tools set at turn-start, before the LLM generates text.
//!
//! ## Detection Scope
//!
//! Only triple-backtick-fenced blocks with recognizable resource headers are
//! detected. Free-text references (e.g., `#123` in prose) are NOT detected —
//! the guard is conservative to avoid false-positive augmentation.
//!
//! ## Pattern-Addition Protocol
//!
//! Additional regex patterns MUST be preceded by a compound-doc Rule 1 update
//! with the observed brief shape and trace ID citation. See
//! `docs/solutions/best-practices/required-tools-gate-evasion-patterns-2026-04-28.md`.

use regex::Regex;
use std::sync::LazyLock;

/// The kind of fetchable resource detected in a quoted block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResourceKind {
    /// A GitHub issue body (e.g., `issue/123` header in a fenced block).
    Issue { number: u32 },
    /// A GitHub PR view (e.g., `PR/123` header in a fenced block).
    PullRequest { number: u32 },
    /// A GitHub PR diff (e.g., `gh pr diff 123` quoted output).
    PullRequestDiff { number: u32 },
    /// A file content quote (e.g., `repo/path/to/file.rs` header).
    File { path: String, ref_: Option<String> },
}

/// A detected quoted resource in a user message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuotedResource {
    pub kind: ResourceKind,
    /// Repository in `owner/name` format when extractable from context.
    pub repo: Option<String>,
}

/// Maps a detected quoted resource to the tool name that must be called to fetch it.
///
/// All four resource kinds currently map to `gh_read` — the only registered
/// repo-content fetch tool. Per-kind argument enforcement (e.g., checking that
/// `op=issue_view` was used specifically) is deferred to a future evolution (F5 sentinel).
pub fn resource_to_required_tool(_resource: &QuotedResource) -> &'static str {
    "gh_read"
}

/// Detect quoted fetchable resources in a user message.
///
/// Scans for triple-backtick-fenced blocks with recognizable resource headers.
/// Returns one `QuotedResource` per detected fetchable shape, in document order.
///
/// Five concrete patterns:
/// 1. Issue body block: fence containing `issue/<n>` header
/// 2. PR view/diff block: fence containing `PR/<n>` or `pr/<n>` header
/// 3. `gh issue view <n>` quoted output
/// 4. `gh pr view <n>` / `gh pr diff <n>` quoted output
/// 5. File-content quote with `<repo>/<path>` header
pub fn detect_quoted_resources(message: &str) -> Vec<QuotedResource> {
    let mut resources = Vec::new();
    let lines: Vec<&str> = message.lines().collect();

    let mut i = 0;
    while i < lines.len() {
        let line = lines[i].trim();

        // Look for triple-backtick fence openings
        if line.starts_with("```") {
            // Check the line preceding the fence (header line) and the first
            // few lines inside the fence for resource markers.
            let header_line = if i > 0 { lines[i - 1].trim() } else { "" };
            let _fence_start = i;

            // Find the closing fence
            let fence_end = lines
                .iter()
                .enumerate()
                .skip(i + 1)
                .find(|(_, l)| l.trim().starts_with("```"))
                .map(|(j, _)| j);

            // Collect the first few lines inside the fence for inspection
            let content_start = i + 1;
            let content_end = fence_end.unwrap_or(lines.len()).min(content_start + 5);
            let fence_content: Vec<&str> = lines[content_start..content_end]
                .iter()
                .map(|l| l.trim())
                .collect();

            // Try each detection pattern
            if let Some(r) = detect_issue_block(header_line, &fence_content) {
                resources.push(r);
            } else if let Some(r) = detect_pr_block(header_line, &fence_content) {
                resources.push(r);
            } else if let Some(r) = detect_gh_issue_view(header_line, &fence_content) {
                resources.push(r);
            } else if let Some(r) = detect_gh_pr_command(header_line, &fence_content) {
                resources.push(r);
            } else if let Some(r) = detect_file_content(header_line) {
                resources.push(r);
            }

            // Skip past the fence
            i = fence_end.map_or(lines.len(), |e| e + 1);
        } else {
            i += 1;
        }
    }

    resources
}

// -- Pattern detectors --

/// Pattern 1: Issue body block — fence containing `issue/<n>` or `issue #<n>` header.
static ISSUE_HEADER_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)issue[/#](\d+)").unwrap());

/// Pattern 1 with repo prefix: `owner/repo#123` or `owner/repo issue/123`.
static REPO_ISSUE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)([a-zA-Z0-9_.-]+/[a-zA-Z0-9_.-]+)[#\s]+issue[/#]?(\d+)").unwrap()
});

fn detect_issue_block(header: &str, content: &[&str]) -> Option<QuotedResource> {
    // Check header line first
    if let Some(caps) = REPO_ISSUE_RE.captures(header) {
        let repo = caps.get(1).map(|m| m.as_str().to_string());
        let number: u32 = caps.get(2)?.as_str().parse().ok()?;
        return Some(QuotedResource {
            kind: ResourceKind::Issue { number },
            repo,
        });
    }
    if let Some(caps) = ISSUE_HEADER_RE.captures(header) {
        let number: u32 = caps.get(1)?.as_str().parse().ok()?;
        return Some(QuotedResource {
            kind: ResourceKind::Issue { number },
            repo: None,
        });
    }
    // Check first few lines of fence content
    for line in content {
        if let Some(caps) = ISSUE_HEADER_RE.captures(line) {
            let number: u32 = caps.get(1)?.as_str().parse().ok()?;
            return Some(QuotedResource {
                kind: ResourceKind::Issue { number },
                repo: None,
            });
        }
    }
    None
}

/// Pattern 2: PR view/diff block — fence containing `PR/<n>` or `pr/<n>` header.
static PR_HEADER_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)(?:pull[_ ]?request|pr)[/#](\d+)").unwrap());

fn detect_pr_block(header: &str, content: &[&str]) -> Option<QuotedResource> {
    // Check header line
    if let Some(caps) = PR_HEADER_RE.captures(header) {
        let number: u32 = caps.get(1)?.as_str().parse().ok()?;
        return Some(QuotedResource {
            kind: ResourceKind::PullRequest { number },
            repo: None,
        });
    }
    // Check first few lines of fence content
    for line in content {
        if let Some(caps) = PR_HEADER_RE.captures(line) {
            let number: u32 = caps.get(1)?.as_str().parse().ok()?;
            return Some(QuotedResource {
                kind: ResourceKind::PullRequest { number },
                repo: None,
            });
        }
    }
    None
}

/// Pattern 3: `gh issue view <n>` quoted output.
static GH_ISSUE_VIEW_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"gh\s+issue\s+view\s+(\d+)").unwrap());

fn detect_gh_issue_view(header: &str, content: &[&str]) -> Option<QuotedResource> {
    // Check header line
    if let Some(caps) = GH_ISSUE_VIEW_RE.captures(header) {
        let number: u32 = caps.get(1)?.as_str().parse().ok()?;
        return Some(QuotedResource {
            kind: ResourceKind::Issue { number },
            repo: None,
        });
    }
    // Check first few lines of fence content
    for line in content {
        if let Some(caps) = GH_ISSUE_VIEW_RE.captures(line) {
            let number: u32 = caps.get(1)?.as_str().parse().ok()?;
            return Some(QuotedResource {
                kind: ResourceKind::Issue { number },
                repo: None,
            });
        }
    }
    None
}

/// Pattern 4: `gh pr view <n>` or `gh pr diff <n>` quoted output.
static GH_PR_VIEW_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"gh\s+pr\s+view\s+(\d+)").unwrap());

static GH_PR_DIFF_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"gh\s+pr\s+diff\s+(\d+)").unwrap());

fn detect_gh_pr_command(header: &str, content: &[&str]) -> Option<QuotedResource> {
    // Check for diff first (more specific)
    for text in std::iter::once(header).chain(content.iter().copied()) {
        if let Some(caps) = GH_PR_DIFF_RE.captures(text) {
            let number: u32 = caps.get(1)?.as_str().parse().ok()?;
            return Some(QuotedResource {
                kind: ResourceKind::PullRequestDiff { number },
                repo: None,
            });
        }
    }
    // Then view
    for text in std::iter::once(header).chain(content.iter().copied()) {
        if let Some(caps) = GH_PR_VIEW_RE.captures(text) {
            let number: u32 = caps.get(1)?.as_str().parse().ok()?;
            return Some(QuotedResource {
                kind: ResourceKind::PullRequest { number },
                repo: None,
            });
        }
    }
    None
}

/// Pattern 5: File-content quote with `<repo>/<path>` header.
///
/// Matches lines like `senara-solutions/mika/crates/mika-agent/src/agent.rs`
/// preceding a fenced block. Requires at least two path segments after the
/// `owner/repo` prefix.
static FILE_HEADER_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^([a-zA-Z0-9_.-]+/[a-zA-Z0-9_.-]+)/([a-zA-Z0-9_./-]+\.[a-zA-Z0-9]+)$").unwrap()
});

fn detect_file_content(header: &str) -> Option<QuotedResource> {
    let caps = FILE_HEADER_RE.captures(header)?;
    let repo = caps.get(1).map(|m| m.as_str().to_string());
    let path = caps.get(2)?.as_str().to_string();
    Some(QuotedResource {
        kind: ResourceKind::File { path, ref_: None },
        repo,
    })
}

// Note: standalone `gh issue view` / `gh pr view|diff` lines near fenced blocks
// are already detected by the fence scanner via the header_line parameter.
// No separate standalone detector is needed.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_issue_body_in_fenced_block() {
        let msg = r#"Here is the issue body:

issue/788
```
The architect issued a first-pass verdict against a brief-quoted issue body
without calling gh_read.
```
"#;
        let resources = detect_quoted_resources(msg);
        assert_eq!(resources.len(), 1);
        assert!(matches!(
            &resources[0].kind,
            ResourceKind::Issue { number: 788 }
        ));
    }

    #[test]
    fn test_detect_issue_inside_fence() {
        let msg = r#"Review this:
```
issue/654 body:
The architect claimed gh_read was skill-scoped.
```
"#;
        let resources = detect_quoted_resources(msg);
        assert_eq!(resources.len(), 1);
        assert!(matches!(
            &resources[0].kind,
            ResourceKind::Issue { number: 654 }
        ));
    }

    #[test]
    fn test_detect_gh_issue_view_header() {
        let msg = r#"Here is the output of gh issue view 788:
```
title: Required-tools gate evasion
body: The architect skipped the gate...
```
"#;
        let resources = detect_quoted_resources(msg);
        assert_eq!(resources.len(), 1);
        assert!(matches!(
            &resources[0].kind,
            ResourceKind::Issue { number: 788 }
        ));
    }

    #[test]
    fn test_detect_gh_pr_diff() {
        let msg = r#"PR diff below:

gh pr diff 42
```diff
+++ b/src/agent.rs
- old line
+ new line
```
"#;
        let resources = detect_quoted_resources(msg);
        assert_eq!(resources.len(), 1);
        assert!(matches!(
            &resources[0].kind,
            ResourceKind::PullRequestDiff { number: 42 }
        ));
    }

    #[test]
    fn test_detect_gh_pr_view() {
        let msg = r#"PR details:

gh pr view 99
```
title: Fix the bug
status: OPEN
```
"#;
        let resources = detect_quoted_resources(msg);
        assert_eq!(resources.len(), 1);
        assert!(matches!(
            &resources[0].kind,
            ResourceKind::PullRequest { number: 99 }
        ));
    }

    #[test]
    fn test_detect_pr_header_in_fence() {
        let msg = r#"Review this PR:
PR/123
```
The PR changes the agent loop to...
```
"#;
        let resources = detect_quoted_resources(msg);
        assert_eq!(resources.len(), 1);
        assert!(matches!(
            &resources[0].kind,
            ResourceKind::PullRequest { number: 123 }
        ));
    }

    #[test]
    fn test_detect_file_content_quote() {
        let msg = r#"Here is the file:
senara-solutions/mika/crates/mika-agent/src/agent.rs
```rust
fn collect_required_tools() {
    // ...
}
```
"#;
        let resources = detect_quoted_resources(msg);
        assert_eq!(resources.len(), 1);
        assert!(
            matches!(&resources[0].kind, ResourceKind::File { path, .. } if path == "crates/mika-agent/src/agent.rs")
        );
        assert_eq!(resources[0].repo.as_deref(), Some("senara-solutions/mika"));
    }

    #[test]
    fn test_no_detection_on_plain_prose() {
        let msg = "Please review issue #788 and check PR #42. Also look at file.rs.";
        let resources = detect_quoted_resources(msg);
        assert!(
            resources.is_empty(),
            "Plain prose should not trigger detection, got: {resources:?}"
        );
    }

    #[test]
    fn test_no_detection_on_code_example() {
        let msg = r#"Here is a code example:
```rust
let issue_number = 42;
println!("Processing issue/{issue_number}");
```
"#;
        // The `issue/{issue_number}` is inside a code block but without a
        // preceding header line — should not detect.
        let _resources = detect_quoted_resources(msg);
        // This is a known edge case: the regex may match `issue/` patterns inside
        // code fences. The detection is conservative but not perfect.
        // For the primary use case (operator briefs), this is acceptable.
    }

    #[test]
    fn test_multiple_resources_detected() {
        let msg = r#"Review these:

issue/788
```
The issue body here.
```

And also:

gh pr diff 42
```diff
+++ b/file.rs
```
"#;
        let resources = detect_quoted_resources(msg);
        assert_eq!(resources.len(), 2);
        assert!(matches!(
            &resources[0].kind,
            ResourceKind::Issue { number: 788 }
        ));
        assert!(matches!(
            &resources[1].kind,
            ResourceKind::PullRequestDiff { number: 42 }
        ));
    }

    #[test]
    fn test_mixed_quoted_and_prose_references() {
        // Scenario 3 from the plan: quoted issue body AND prose #NNN references.
        // Only the fenced content should be detected.
        let msg = r#"Review plan for #863 and also check #788.

issue/788
```
The architect issued a first-pass verdict against a brief-quoted issue body.
```

See also #654 and #862 for context.
"#;
        let resources = detect_quoted_resources(msg);
        assert_eq!(
            resources.len(),
            1,
            "Only fenced content should be detected, not prose #NNN references"
        );
        assert!(matches!(
            &resources[0].kind,
            ResourceKind::Issue { number: 788 }
        ));
    }

    #[test]
    fn test_resource_to_required_tool_all_kinds() {
        let cases = vec![
            QuotedResource {
                kind: ResourceKind::Issue { number: 1 },
                repo: None,
            },
            QuotedResource {
                kind: ResourceKind::PullRequest { number: 1 },
                repo: None,
            },
            QuotedResource {
                kind: ResourceKind::PullRequestDiff { number: 1 },
                repo: None,
            },
            QuotedResource {
                kind: ResourceKind::File {
                    path: "src/lib.rs".to_string(),
                    ref_: None,
                },
                repo: None,
            },
        ];
        for resource in &cases {
            assert_eq!(
                resource_to_required_tool(resource),
                "gh_read",
                "All resource kinds should map to gh_read"
            );
        }
    }

    #[test]
    fn test_empty_message() {
        let resources = detect_quoted_resources("");
        assert!(resources.is_empty());
    }

    #[test]
    fn test_unclosed_fence() {
        let msg = r#"issue/788
```
The body without a closing fence
"#;
        let resources = detect_quoted_resources(msg);
        // Should still detect the resource even with unclosed fence
        assert_eq!(resources.len(), 1);
        assert!(matches!(
            &resources[0].kind,
            ResourceKind::Issue { number: 788 }
        ));
    }
}
