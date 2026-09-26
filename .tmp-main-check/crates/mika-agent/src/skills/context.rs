//! Skill context resolution — engine-owned pre-fetch for typed data requirements.
//!
//! Skills declare `[context.*]` sections in `skill.toml` with a `type` field identifying
//! an engine-owned fetch handler. Before the LLM turn, the engine resolves each requirement,
//! fetching data and returning it as a `ContextBlock` for template injection.

use std::collections::HashMap;

use anyhow::{Context as _, Result};
use regex::Regex;
use reqwest::Client;
use std::sync::LazyLock;

use super::index::SkillEntry;

/// Known context type identifiers. Unknown types produce validation errors at load time.
pub const KNOWN_CONTEXT_TYPES: &[&str] = &["gh_pr_diff"];

/// Default character budget for context injection (~50K tokens at 4 chars/token).
pub const DEFAULT_CONTEXT_CHAR_BUDGET: usize = 200_000;

/// Timeout for context resolution HTTP requests.
const CONTEXT_FETCH_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);

/// Shared HTTP client for context resolution.
static CONTEXT_CLIENT: LazyLock<Client> = LazyLock::new(|| {
    Client::builder()
        .timeout(CONTEXT_FETCH_TIMEOUT)
        .user_agent("mika-agent")
        .build()
        .expect("failed to build context HTTP client")
});

/// Status of a resolved context block, derived from resolution outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContextStatus {
    /// Full content was available and injected.
    Full,
    /// Content was truncated due to budget constraints.
    Truncated,
    /// Context resolution failed; sentinel text was injected.
    Unavailable,
}

impl ContextStatus {
    /// Label used in the `<!-- context_meta: ... -->` annotation.
    pub fn as_str(&self) -> &'static str {
        match self {
            ContextStatus::Full => "full",
            ContextStatus::Truncated => "truncated",
            ContextStatus::Unavailable => "unavailable",
        }
    }
}

/// Sentinel prefix for unavailable context blocks.
const UNAVAILABLE_SENTINEL_PREFIX: &str = "(Context unavailable:";

/// Resolved context block ready for template injection.
#[derive(Debug, Clone)]
pub struct ContextBlock {
    pub name: String,
    pub content: String,
    pub truncated: bool,
    /// The context type from the skill manifest (e.g., "gh_pr_diff").
    pub context_type: String,
}

impl ContextBlock {
    /// Derive the context status from the block's state.
    pub fn status(&self) -> ContextStatus {
        if self.content.starts_with(UNAVAILABLE_SENTINEL_PREFIX) {
            ContextStatus::Unavailable
        } else if self.truncated {
            ContextStatus::Truncated
        } else {
            ContextStatus::Full
        }
    }

    /// Build the metadata annotation comment line for this context block.
    pub fn metadata_annotation(&self) -> String {
        format!(
            "<!-- context_meta: type={}, status={}, chars={} -->",
            self.context_type,
            self.status().as_str(),
            self.content.len(),
        )
    }
}

/// Resolve context requirements for matched skills.
///
/// For each matched skill with `[context]` declarations, dispatches to the appropriate
/// handler by `context_type`. Returns resolved context blocks and indices of skills
/// to exclude (because a `required = true` context failed).
///
/// Deduplication: same key + same type across multiple skills = fetch once.
/// Same key + different type = warn, skip both.
pub async fn resolve_contexts(
    matched: &[&SkillEntry],
    message: &str,
    github_token: Option<&str>,
) -> (HashMap<String, ContextBlock>, Vec<usize>) {
    let mut resolved: HashMap<String, ContextBlock> = HashMap::new();
    let mut exclude_indices: Vec<usize> = Vec::new();

    // Collect all unique (key, requirement) pairs with their skill indices
    // key -> (type, required, skill_indices)
    let mut requirements: HashMap<String, (String, bool, Vec<usize>)> = HashMap::new();

    for (idx, entry) in matched.iter().enumerate() {
        for (key, req) in &entry.manifest.context {
            // Check existing entry type (if any) without holding a borrow across mutations
            let conflict = requirements
                .get(key)
                .map(|(existing_type, _, _)| existing_type != &req.context_type);

            match conflict {
                Some(true) => {
                    // Type conflict — remove the existing entry so neither skill gets this context
                    let (existing_type, existing_required, existing_indices) =
                        requirements.remove(key).unwrap();
                    tracing::warn!(
                        key = key,
                        type1 = existing_type.as_str(),
                        type2 = req.context_type.as_str(),
                        "conflicting context types for key '{}', skipping both",
                        key
                    );
                    if existing_required {
                        exclude_indices.extend(existing_indices);
                    }
                    if req.required {
                        exclude_indices.push(idx);
                    }
                }
                Some(false) => {
                    // Same type — dedup, just track the skill index
                    let entry = requirements.get_mut(key).unwrap();
                    entry.2.push(idx);
                    // Upgrade required if any declaring skill says required
                    if req.required {
                        entry.1 = true;
                    }
                }
                None => {
                    requirements.insert(
                        key.clone(),
                        (req.context_type.clone(), req.required, vec![idx]),
                    );
                }
            }
        }
    }

    // Resolve each unique requirement
    for (key, (context_type, required, skill_indices)) in &requirements {
        let result = match context_type.as_str() {
            "gh_pr_diff" => resolve_gh_pr_diff(message, github_token).await,
            _ => {
                tracing::warn!(context_type = context_type.as_str(), "unknown context type");
                Err(anyhow::anyhow!("unknown context type: {}", context_type))
            }
        };

        match result {
            Ok(content) => {
                tracing::info!(
                    key = key.as_str(),
                    context_type = context_type.as_str(),
                    bytes = content.content.len(),
                    truncated = content.truncated,
                    "context resolved"
                );
                resolved.insert(key.clone(), content);
            }
            Err(err) => {
                tracing::warn!(
                    key = key.as_str(),
                    context_type = context_type.as_str(),
                    error = %err,
                    required = required,
                    "context resolution failed"
                );
                if *required {
                    exclude_indices.extend(skill_indices);
                } else {
                    // Insert sentinel so the LLM sees a descriptive message
                    // instead of raw {{key}} placeholder text
                    resolved.insert(
                        key.clone(),
                        ContextBlock {
                            name: key.clone(),
                            content: format!(
                                "(Context unavailable: {} resolution failed)",
                                context_type
                            ),
                            truncated: false,
                            context_type: context_type.clone(),
                        },
                    );
                }
            }
        }
    }

    // Deduplicate exclusion indices
    exclude_indices.sort_unstable();
    exclude_indices.dedup();

    (resolved, exclude_indices)
}

/// Static regex matching `{{key}}` placeholders (word characters only).
static PLACEHOLDER_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\{\{(\w+)\}\}").unwrap());

/// Apply single-pass template variable replacement on a prompt string.
///
/// Replaces all `{{key}}` placeholders with resolved context content.
/// This is a true single-pass: the regex scans the original string once,
/// and replaced content is never re-scanned (injection-safe).
/// Unknown keys (not in the context map) are left as-is.
pub fn apply_context_replacements(prompt: &str, context: &HashMap<String, ContextBlock>) -> String {
    if context.is_empty() || !prompt.contains("{{") {
        return prompt.to_string();
    }

    PLACEHOLDER_RE
        .replace_all(prompt, |caps: &regex::Captures| {
            let key = &caps[1];
            context
                .get(key)
                .map(|b| format!("{}\n{}", b.metadata_annotation(), b.content))
                .unwrap_or_else(|| caps[0].to_string())
        })
        .into_owned()
}

// --- gh_pr_diff handler ---

static PR_URL_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"https://github\.com/([^/\s]+)/([^/\s]+)/pull/(\d+)").unwrap());

/// Extract the first GitHub PR URL from a message.
/// Returns (owner, repo, number).
pub fn extract_pr_url(message: &str) -> Option<(String, String, u64)> {
    PR_URL_RE.captures(message).and_then(|caps| {
        let owner = caps.get(1)?.as_str().to_string();
        let repo = caps.get(2)?.as_str().to_string();
        let number: u64 = caps.get(3)?.as_str().parse().ok()?;
        Some((owner, repo, number))
    })
}

/// Resolve a `gh_pr_diff` context requirement.
async fn resolve_gh_pr_diff(message: &str, github_token: Option<&str>) -> Result<ContextBlock> {
    let (owner, repo, number) =
        extract_pr_url(message).context("no GitHub PR URL found in message")?;

    let diff = fetch_pr_diff(&owner, &repo, number, github_token).await?;

    if diff.is_empty() {
        return Ok(ContextBlock {
            name: "pr_diff".to_string(),
            content: "(No file changes in this pull request.)".to_string(),
            truncated: false,
            context_type: "gh_pr_diff".to_string(),
        });
    }

    // Apply truncation if needed
    if diff.len() > DEFAULT_CONTEXT_CHAR_BUDGET {
        let truncated = truncate_diff(&diff, DEFAULT_CONTEXT_CHAR_BUDGET);
        Ok(truncated)
    } else {
        Ok(ContextBlock {
            name: "pr_diff".to_string(),
            content: diff,
            truncated: false,
            context_type: "gh_pr_diff".to_string(),
        })
    }
}

/// Fetch PR diff from GitHub API.
async fn fetch_pr_diff(
    owner: &str,
    repo: &str,
    number: u64,
    github_token: Option<&str>,
) -> Result<String> {
    let url = format!(
        "https://api.github.com/repos/{}/{}/pulls/{}",
        owner, repo, number
    );

    let mut request = CONTEXT_CLIENT
        .get(&url)
        .header("Accept", "application/vnd.github.v3.diff")
        .header("X-GitHub-Api-Version", "2022-11-28");

    if let Some(token) = github_token {
        request = request.header("Authorization", format!("Bearer {}", token));
    }

    let response = request.send().await?;
    let status = response.status();

    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        anyhow::bail!(
            "GitHub API returned {} for PR {}/{}/{}:  {}",
            status,
            owner,
            repo,
            number,
            mika_common::text::safe_truncate(&body, 200)
        );
    }

    response
        .text()
        .await
        .context("failed to read response body")
}

/// Patterns that identify generated/vendored files to deprioritize during truncation.
const GENERATED_PATTERNS: &[&str] = &[
    "Cargo.lock",
    "package-lock.json",
    "yarn.lock",
    "pnpm-lock.yaml",
    "Gemfile.lock",
    "poetry.lock",
    "composer.lock",
    ".generated.",
    ".gen.",
    "_generated.",
    ".pb.go",
    ".pb.rs",
    "schema.rb",
    "structure.sql",
];

const GENERATED_DIR_PREFIXES: &[&str] = &["dist/", "build/", "node_modules/", "target/"];

fn is_generated_file(path: &str) -> bool {
    GENERATED_PATTERNS.iter().any(|p| path.contains(p))
        || GENERATED_DIR_PREFIXES.iter().any(|p| path.starts_with(p))
}

/// A parsed file from a unified diff.
struct DiffFile {
    path: String,
    content: String,
    added: usize,
    removed: usize,
}

/// Parse a unified diff into file-level segments.
fn parse_diff_files(diff: &str) -> Vec<DiffFile> {
    let mut files = Vec::new();
    let mut current_path = String::new();
    let mut current_content = String::new();
    let mut added = 0usize;
    let mut removed = 0usize;

    for line in diff.lines() {
        if line.starts_with("diff --git ") {
            // Save previous file
            if !current_path.is_empty() {
                files.push(DiffFile {
                    path: current_path.clone(),
                    content: current_content.clone(),
                    added,
                    removed,
                });
            }
            // Parse new file path from "diff --git a/path b/path"
            current_path = line.split(" b/").nth(1).unwrap_or("unknown").to_string();
            current_content = format!("{}\n", line);
            added = 0;
            removed = 0;
        } else {
            current_content.push_str(line);
            current_content.push('\n');
            if line.starts_with('+') && !line.starts_with("+++") {
                added += 1;
            } else if line.starts_with('-') && !line.starts_with("---") {
                removed += 1;
            }
        }
    }
    // Don't forget the last file
    if !current_path.is_empty() {
        files.push(DiffFile {
            path: current_path,
            content: current_content,
            added,
            removed,
        });
    }

    files
}

/// Truncate a unified diff to fit within a character budget.
///
/// Priority: non-generated files first, highest churn density first.
/// Uses `str::floor_char_boundary()` for UTF-8 safety.
pub fn truncate_diff(raw_diff: &str, char_budget: usize) -> ContextBlock {
    let files = parse_diff_files(raw_diff);
    if files.is_empty() {
        return ContextBlock {
            name: "pr_diff".to_string(),
            content: raw_diff[..raw_diff.floor_char_boundary(char_budget)].to_string(),
            truncated: true,
            context_type: "gh_pr_diff".to_string(),
        };
    }

    // Score files: non-generated get priority, then by churn density
    let mut scored: Vec<(usize, f64)> = files
        .iter()
        .enumerate()
        .map(|(i, f)| {
            let is_gen = is_generated_file(&f.path);
            let total = f.content.len().max(1) as f64;
            let churn = (f.added + f.removed) as f64 / total;
            let score = if is_gen { churn * 0.01 } else { 1.0 + churn };
            (i, score)
        })
        .collect();

    scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

    let mut result = String::new();
    let mut omitted: Vec<(String, usize, usize)> = Vec::new();
    let mut remaining_budget = char_budget;

    // Reserve space for truncation notice
    let notice_reserve = 500;
    remaining_budget = remaining_budget.saturating_sub(notice_reserve);

    for (idx, _score) in &scored {
        let file = &files[*idx];
        if file.content.len() <= remaining_budget {
            result.push_str(&file.content);
            remaining_budget -= file.content.len();
        } else if remaining_budget > 200 {
            // Truncate within this file
            let boundary = file.content.floor_char_boundary(remaining_budget);
            result.push_str(&file.content[..boundary]);
            result.push_str("\n... [file truncated]\n");
            remaining_budget = 0;
        } else {
            omitted.push((file.path.clone(), file.added, file.removed));
        }
    }

    if !omitted.is_empty() {
        result.push_str(&format!(
            "\n--- Diff truncated at ~{}K chars ---\n",
            char_budget / 1000
        ));
        result.push_str(&format!(
            "Files omitted ({} remaining, excluded by size/generated policy):\n",
            omitted.len()
        ));
        for (path, added, removed) in &omitted {
            result.push_str(&format!("- {} (+{} -{})\n", path, added, removed));
        }
        result.push_str("Review scope is limited to files shown above.\n");
    }

    ContextBlock {
        name: "pr_diff".to_string(),
        content: result,
        truncated: true,
        context_type: "gh_pr_diff".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_pr_url_standard() {
        let msg = "review https://github.com/senara-solutions/mika/pull/281";
        let (owner, repo, number) = extract_pr_url(msg).unwrap();
        assert_eq!(owner, "senara-solutions");
        assert_eq!(repo, "mika");
        assert_eq!(number, 281);
    }

    #[test]
    fn test_extract_pr_url_in_text() {
        let msg = "Please review this PR: https://github.com/org/repo/pull/42 thanks";
        let (owner, repo, number) = extract_pr_url(msg).unwrap();
        assert_eq!(owner, "org");
        assert_eq!(repo, "repo");
        assert_eq!(number, 42);
    }

    #[test]
    fn test_extract_pr_url_none() {
        assert!(extract_pr_url("no url here").is_none());
        assert!(extract_pr_url("https://github.com/org/repo/issues/5").is_none());
    }

    #[test]
    fn test_extract_pr_url_multiple_takes_first() {
        let msg = "review https://github.com/a/b/pull/1 and https://github.com/c/d/pull/2";
        let (owner, repo, number) = extract_pr_url(msg).unwrap();
        assert_eq!(owner, "a");
        assert_eq!(repo, "b");
        assert_eq!(number, 1);
    }

    /// Helper to build a test ContextBlock with default context_type.
    fn test_block(name: &str, content: &str, truncated: bool) -> ContextBlock {
        ContextBlock {
            name: name.to_string(),
            content: content.to_string(),
            truncated,
            context_type: "test_type".to_string(),
        }
    }

    #[test]
    fn test_apply_context_replacements_basic() {
        let mut ctx = HashMap::new();
        ctx.insert(
            "pr_diff".to_string(),
            test_block("pr_diff", "the diff content", false),
        );
        let result = apply_context_replacements("Review this: {{pr_diff}} end", &ctx);
        assert!(result.contains("<!-- context_meta: type=test_type, status=full, chars=16 -->"));
        assert!(result.contains("the diff content"));
        assert!(result.ends_with(" end"));
    }

    #[test]
    fn test_apply_context_replacements_empty_context() {
        let ctx = HashMap::new();
        let result = apply_context_replacements("no {{replacements}} here", &ctx);
        assert_eq!(result, "no {{replacements}} here");
    }

    #[test]
    fn test_apply_context_replacements_no_placeholders() {
        let mut ctx = HashMap::new();
        ctx.insert("key".to_string(), test_block("key", "value", false));
        let result = apply_context_replacements("no placeholders", &ctx);
        assert_eq!(result, "no placeholders");
    }

    #[test]
    fn test_apply_context_replacements_single_pass_no_recursion() {
        // Content that itself contains {{another_key}} should NOT be expanded
        let mut ctx = HashMap::new();
        ctx.insert(
            "pr_diff".to_string(),
            test_block("pr_diff", "contains {{other_key}} text", false),
        );
        ctx.insert(
            "other_key".to_string(),
            test_block("other_key", "SHOULD NOT APPEAR", false),
        );
        let result = apply_context_replacements("{{pr_diff}}", &ctx);
        // The {{other_key}} inside pr_diff's content should NOT be replaced
        assert!(result.contains("contains {{other_key}} text"));
    }

    #[test]
    fn test_is_generated_file() {
        assert!(is_generated_file("Cargo.lock"));
        assert!(is_generated_file("package-lock.json"));
        assert!(is_generated_file("dist/bundle.js"));
        assert!(is_generated_file("src/schema.generated.rs"));
        assert!(!is_generated_file("src/main.rs"));
        assert!(!is_generated_file("lib/handler.ts"));
    }

    #[test]
    fn test_truncate_diff_under_budget() {
        let diff = "diff --git a/file.rs b/file.rs\n+added line\n-removed line\n";
        let result = truncate_diff(diff, 10000);
        assert!(result.content.contains("file.rs"));
    }

    #[test]
    fn test_truncate_diff_over_budget() {
        // Create a diff larger than budget
        let mut diff = String::new();
        for i in 0..100 {
            diff.push_str(&format!("diff --git a/file{}.rs b/file{}.rs\n", i, i));
            for j in 0..50 {
                diff.push_str(&format!("+added line {} in file {}\n", j, i));
            }
        }
        let result = truncate_diff(&diff, 5000);
        assert!(result.truncated);
        // Budget (5000) + notice reserve (500) + omitted file summaries
        assert!(result.content.len() <= 10000);
    }

    #[test]
    fn test_parse_diff_files() {
        let diff = "diff --git a/src/main.rs b/src/main.rs\n+line1\n-line2\ndiff --git a/Cargo.lock b/Cargo.lock\n+dep1\n";
        let files = parse_diff_files(diff);
        assert_eq!(files.len(), 2);
        assert_eq!(files[0].path, "src/main.rs");
        assert_eq!(files[0].added, 1);
        assert_eq!(files[0].removed, 1);
        assert_eq!(files[1].path, "Cargo.lock");
        assert_eq!(files[1].added, 1);
    }

    #[test]
    fn test_truncate_diff_prioritizes_non_generated() {
        // Create a diff with a generated file (Cargo.lock) and a source file
        let mut diff = String::new();
        // Large generated file
        diff.push_str("diff --git a/Cargo.lock b/Cargo.lock\n");
        for _ in 0..100 {
            diff.push_str("+some dependency change\n");
        }
        // Small source file
        diff.push_str("diff --git a/src/main.rs b/src/main.rs\n");
        diff.push_str("+fn main() {}\n");

        // Budget that fits both, but check that source file is included
        let result = truncate_diff(&diff, 3000);
        assert!(result.content.contains("src/main.rs"));
    }

    #[test]
    fn test_apply_context_replacements_multiple_keys() {
        let mut ctx = HashMap::new();
        ctx.insert("key_a".to_string(), test_block("key_a", "AAA", false));
        ctx.insert("key_b".to_string(), test_block("key_b", "BBB", false));
        let result = apply_context_replacements("start {{key_a}} middle {{key_b}} end", &ctx);
        assert!(result.contains("AAA"));
        assert!(result.contains("BBB"));
    }

    // --- Step 1 tests: context metadata annotation ---

    #[test]
    fn test_context_status_full() {
        let block = test_block("pr_diff", "diff content", false);
        assert_eq!(block.status(), ContextStatus::Full);
    }

    #[test]
    fn test_context_status_truncated() {
        let block = test_block("pr_diff", "partial diff...", true);
        assert_eq!(block.status(), ContextStatus::Truncated);
    }

    #[test]
    fn test_context_status_unavailable() {
        let block = test_block(
            "pr_diff",
            "(Context unavailable: gh_pr_diff resolution failed)",
            false,
        );
        assert_eq!(block.status(), ContextStatus::Unavailable);
    }

    #[test]
    fn test_metadata_annotation_full() {
        let block = ContextBlock {
            name: "pr_diff".to_string(),
            content: "some diff".to_string(),
            truncated: false,
            context_type: "gh_pr_diff".to_string(),
        };
        assert_eq!(
            block.metadata_annotation(),
            "<!-- context_meta: type=gh_pr_diff, status=full, chars=9 -->"
        );
    }

    #[test]
    fn test_metadata_annotation_truncated() {
        let block = ContextBlock {
            name: "pr_diff".to_string(),
            content: "partial...".to_string(),
            truncated: true,
            context_type: "gh_pr_diff".to_string(),
        };
        assert_eq!(
            block.metadata_annotation(),
            "<!-- context_meta: type=gh_pr_diff, status=truncated, chars=10 -->"
        );
    }

    #[test]
    fn test_metadata_annotation_unavailable() {
        let block = ContextBlock {
            name: "pr_diff".to_string(),
            content: "(Context unavailable: gh_pr_diff resolution failed)".to_string(),
            truncated: false,
            context_type: "gh_pr_diff".to_string(),
        };
        assert_eq!(
            block.metadata_annotation(),
            "<!-- context_meta: type=gh_pr_diff, status=unavailable, chars=51 -->"
        );
    }

    #[test]
    fn test_apply_context_replacements_prepends_metadata() {
        let mut ctx = HashMap::new();
        ctx.insert(
            "pr_diff".to_string(),
            ContextBlock {
                name: "pr_diff".to_string(),
                content: "the diff".to_string(),
                truncated: false,
                context_type: "gh_pr_diff".to_string(),
            },
        );
        let result = apply_context_replacements("BEGIN {{pr_diff}} END", &ctx);
        assert!(result.contains("<!-- context_meta: type=gh_pr_diff, status=full, chars=8 -->"));
        assert!(result.contains("the diff"));
        // Metadata annotation should come before the content
        let meta_pos = result.find("<!-- context_meta:").unwrap();
        let content_pos = result.find("the diff").unwrap();
        assert!(meta_pos < content_pos);
    }

    #[test]
    fn test_truncate_diff_includes_context_type() {
        let diff = "diff --git a/file.rs b/file.rs\n+line\n";
        let result = truncate_diff(diff, 100);
        assert_eq!(result.context_type, "gh_pr_diff");
    }
}
