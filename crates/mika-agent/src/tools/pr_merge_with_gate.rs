use std::sync::LazyLock;

use anyhow::Result;
use async_trait::async_trait;
use mika_common::claude::ToolDefinition;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::io::AsyncReadExt;
use tracing::warn;

use super::{Tool, ToolContext, ToolOutput};

/// Maximum bytes to read from a `gh` subprocess stdout/stderr (256 KB).
const MAX_OUTPUT_LEN: usize = 256 * 1024;

/// Allowed merge methods — passed as `--{method}` to `gh pr merge`.
const ALLOWED_MERGE_METHODS: &[&str] = &["squash", "merge", "rebase"];

/// Regex for validating `owner/repo` format.
static REPO_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[a-zA-Z0-9._-]+/[a-zA-Z0-9._-]+$").unwrap());

// ---------------------------------------------------------------------------
// Public tool struct
// ---------------------------------------------------------------------------

pub struct PrMergeWithGateTool;

#[async_trait]
impl Tool for PrMergeWithGateTool {
    fn name(&self) -> &str {
        "pr_merge_with_gate"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "pr_merge_with_gate".to_string(),
            description: "Merge a GitHub pull request with a CI gate. Checks the status of \
                required CI checks before merging. If any required check is failing, the merge \
                is blocked and the failing checks are returned. If checks are still pending \
                (but none failing), auto-merge is enabled so GitHub merges automatically when \
                checks pass. If all required checks pass, the PR is merged immediately.\n\n\
                IMPORTANT: 'auto_merge_enabled' means GitHub will merge when all checks pass — \
                the PR is NOT yet merged. Do not claim the PR is merged until you confirm it.\n\n\
                After a successful merge (action: 'merged'), update the work item status before \
                reporting to the user."
                .to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "pr_number": {
                        "type": "integer",
                        "description": "Pull request number"
                    },
                    "repo": {
                        "type": "string",
                        "description": "Repository in owner/repo format (e.g. 'senara-solutions/mika')"
                    },
                    "merge_method": {
                        "type": "string",
                        "description": "Merge method: 'squash' (default), 'merge', or 'rebase'",
                        "enum": ["squash", "merge", "rebase"],
                        "default": "squash"
                    },
                    "delete_branch": {
                        "type": "boolean",
                        "description": "Delete head branch after merge (default: true)",
                        "default": true
                    }
                },
                "required": ["pr_number", "repo"],
                "additionalProperties": false
            }),
        }
    }

    fn timeout_secs(&self) -> Option<u64> {
        Some(60)
    }

    async fn execute(&self, input: Value, ctx: &ToolContext<'_>) -> Result<ToolOutput> {
        // -- Extract and validate inputs --
        let pr_number = match input.get("pr_number").and_then(|v| v.as_u64()) {
            Some(n) if n > 0 => n,
            _ => {
                return Ok(ToolOutput::error(
                    "pr_number is required and must be a positive integer",
                ));
            }
        };

        let repo = match input.get("repo").and_then(|v| v.as_str()) {
            Some(r) => r,
            None => return Ok(ToolOutput::error("repo is required (owner/repo format)")),
        };
        if let Err(e) = validate_repo(repo) {
            return Ok(ToolOutput::error(e));
        }

        let merge_method = input
            .get("merge_method")
            .and_then(|v| v.as_str())
            .unwrap_or("squash");
        if let Err(e) = validate_merge_method(merge_method) {
            return Ok(ToolOutput::error(e));
        }

        let delete_branch = input
            .get("delete_branch")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);

        // -- Require GitHub token --
        let token = match ctx.github_token {
            Some(t) => t,
            None => {
                return Ok(ToolOutput::error(
                    "GitHub token required for pr_merge_with_gate. \
                     Set MIKA_GITHUB_TOKEN or configure a GitHub App.",
                ));
            }
        };

        // -- Step 1: Fetch required check statuses --
        let checks_result = run_gh_checks(pr_number, repo, token).await;
        let checks = match checks_result {
            Ok(c) => c,
            Err(e) => {
                return Ok(ToolOutput::error(format!(
                    "Failed to fetch check statuses: {e}"
                )));
            }
        };

        // -- Step 2: Classify and act --
        let classification = classify_checks(&checks);

        match classification {
            CheckClassification::HasFailures => {
                let failing: Vec<&GhCheck> = checks
                    .iter()
                    .filter(|c| matches!(c.bucket.as_str(), "fail" | "cancel"))
                    .collect();

                let result = MergeGateResult::Blocked {
                    failing_checks: failing
                        .iter()
                        .map(|c| CheckInfo {
                            name: c.name.clone(),
                            state: c.state.clone(),
                            link: c.link.clone(),
                        })
                        .collect(),
                };
                Ok(ToolOutput::success(serde_json::to_string_pretty(&result)?))
            }
            CheckClassification::HasPending => {
                // Enable auto-merge — GitHub merges when checks pass
                let auto_result =
                    run_gh_merge(pr_number, repo, merge_method, delete_branch, true, token).await;

                match auto_result {
                    Ok(_output) => {
                        let pending: Vec<CheckInfo> = checks
                            .iter()
                            .filter(|c| c.bucket == "pending")
                            .map(|c| CheckInfo {
                                name: c.name.clone(),
                                state: c.state.clone(),
                                link: None,
                            })
                            .collect();

                        let result = MergeGateResult::AutoMergeEnabled {
                            pending_checks: pending,
                        };
                        Ok(ToolOutput::success(serde_json::to_string_pretty(&result)?))
                    }
                    Err(e) => Ok(ToolOutput::error(format!("Auto-merge failed: {e}"))),
                }
            }
            CheckClassification::AllPassed => {
                // Merge immediately
                let merge_result =
                    run_gh_merge(pr_number, repo, merge_method, delete_branch, false, token).await;

                // Unify success/error into a single string for "already merged" detection
                let (output, is_err) = match merge_result {
                    Ok(s) => (s, false),
                    Err(s) => (s, true),
                };

                let output_lower = output.to_lowercase();
                if output_lower.contains("already been merged") {
                    let result = MergeGateResult::AlreadyMerged;
                    Ok(ToolOutput::success(serde_json::to_string_pretty(&result)?))
                } else if !is_err {
                    let result = MergeGateResult::Merged;
                    Ok(ToolOutput::success(serde_json::to_string_pretty(&result)?))
                } else if output_lower.contains("draft") {
                    Ok(ToolOutput::error(
                        "PR is a draft — convert to ready before merging",
                    ))
                } else if output_lower.contains("merge conflict")
                    || output_lower.contains("not mergeable")
                {
                    Ok(ToolOutput::error(
                        "Merge conflicts — resolve before merging",
                    ))
                } else if output_lower.contains("review") && output_lower.contains("required") {
                    Ok(ToolOutput::error("Required reviews not met"))
                } else {
                    Ok(ToolOutput::error(format!("Merge failed: {output}")))
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Structured result returned by the tool as JSON.
#[derive(Debug, Serialize)]
#[serde(tag = "action")]
enum MergeGateResult {
    #[serde(rename = "merged")]
    Merged,
    #[serde(rename = "auto_merge_enabled")]
    AutoMergeEnabled { pending_checks: Vec<CheckInfo> },
    #[serde(rename = "blocked")]
    Blocked { failing_checks: Vec<CheckInfo> },
    #[serde(rename = "already_merged")]
    AlreadyMerged,
}

/// Check info included in the structured result.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub(crate) struct CheckInfo {
    pub(crate) name: String,
    pub(crate) state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) link: Option<String>,
}

/// A single check from `gh pr checks --json` output.
#[derive(Debug, Clone, Deserialize, PartialEq)]
pub(crate) struct GhCheck {
    pub(crate) name: String,
    pub(crate) state: String,
    pub(crate) bucket: String,
    #[serde(default)]
    pub(crate) link: Option<String>,
}

/// Classification of the overall check status.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum CheckClassification {
    /// All checks passed (or no required checks exist).
    AllPassed,
    /// Some checks are pending, but none have failed.
    HasPending,
    /// At least one check has failed or been cancelled.
    HasFailures,
}

// ---------------------------------------------------------------------------
// Validation helpers
// ---------------------------------------------------------------------------

fn validate_repo(repo: &str) -> Result<(), String> {
    if repo.is_empty() {
        return Err("repo cannot be empty".to_string());
    }
    if repo.len() > 200 {
        return Err("repo is too long (max 200 characters)".to_string());
    }
    if !REPO_RE.is_match(repo) {
        return Err(format!(
            "Invalid repo format: '{repo}'. Expected owner/repo (e.g. 'senara-solutions/mika')"
        ));
    }
    Ok(())
}

fn validate_merge_method(method: &str) -> Result<(), String> {
    if ALLOWED_MERGE_METHODS.contains(&method) {
        Ok(())
    } else {
        Err(format!(
            "Invalid merge_method: '{method}'. Allowed: {}",
            ALLOWED_MERGE_METHODS.join(", ")
        ))
    }
}

// ---------------------------------------------------------------------------
// Check classification (pure function — easily testable)
// ---------------------------------------------------------------------------

pub(crate) fn classify_checks(checks: &[GhCheck]) -> CheckClassification {
    let has_failures = checks
        .iter()
        .any(|c| matches!(c.bucket.as_str(), "fail" | "cancel"));
    let has_pending = checks.iter().any(|c| c.bucket == "pending");

    if has_failures {
        CheckClassification::HasFailures
    } else if has_pending {
        CheckClassification::HasPending
    } else {
        CheckClassification::AllPassed
    }
}

// ---------------------------------------------------------------------------
// Subprocess helpers
// ---------------------------------------------------------------------------

/// Run `gh pr checks <number> --repo <repo> --required --json name,state,bucket,link`
/// and return the parsed check list.
pub(crate) async fn run_gh_checks(
    pr_number: u64,
    repo: &str,
    token: &str,
) -> Result<Vec<GhCheck>, String> {
    let pr_str = pr_number.to_string();
    let args = vec![
        "pr",
        "checks",
        &pr_str,
        "--repo",
        repo,
        "--required",
        "--json",
        "name,state,bucket,link",
    ];

    let output = run_gh_subprocess(&args, token).await?;

    // Empty output or "[]" means no required checks
    let trimmed = output.trim();
    if trimmed.is_empty() || trimmed == "[]" {
        return Ok(vec![]);
    }

    serde_json::from_str::<Vec<GhCheck>>(trimmed)
        .map_err(|e| format!("Failed to parse gh pr checks output: {e}"))
}

/// Run `gh pr merge <number> --repo <repo> --<method> [--delete-branch] [--auto]`
/// and return stdout on success or stderr on failure.
pub(crate) async fn run_gh_merge(
    pr_number: u64,
    repo: &str,
    merge_method: &str,
    delete_branch: bool,
    auto_merge: bool,
    token: &str,
) -> Result<String, String> {
    let pr_str = pr_number.to_string();
    let method_flag = format!("--{merge_method}");
    let mut args = vec!["pr", "merge", &pr_str, "--repo", repo, &method_flag];

    if delete_branch {
        args.push("--delete-branch");
    }
    if auto_merge {
        args.push("--auto");
    }

    run_gh_subprocess(&args, token).await
}

/// Spawn a `gh` subprocess with proper env scrubbing and token injection.
///
/// Returns stdout on success. On failure, returns an error string combining
/// exit code and stderr.
pub(crate) async fn run_gh_subprocess(args: &[&str], token: &str) -> Result<String, String> {
    let mut cmd = tokio::process::Command::new("gh");
    cmd.args(args);
    cmd.env("GH_PROMPT_DISABLED", "1");

    // Scrub MIKA_* and GH_TOKEN, then re-inject the correct token.
    // Uses the same pattern as run_gh in builtin_handlers.rs.
    crate::skills::executor::scrub_mika_env_vars(&mut cmd);
    cmd.env("GH_TOKEN", token);

    cmd.kill_on_drop(true);
    cmd.stdin(std::process::Stdio::null());
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            if e.kind() == std::io::ErrorKind::NotFound {
                return Err("gh CLI not found — install from https://cli.github.com".to_string());
            }
            return Err(format!("Failed to spawn gh: {e}"));
        }
    };

    // Read stdout and stderr with bounded size to prevent memory exhaustion
    let stdout_handle = child.stdout.take().expect("stdout piped");
    let stderr_handle = child.stderr.take().expect("stderr piped");
    let mut stdout_buf = Vec::with_capacity(MAX_OUTPUT_LEN);
    let mut stderr_buf = Vec::with_capacity(MAX_OUTPUT_LEN);

    let mut stdout_take = stdout_handle.take(MAX_OUTPUT_LEN as u64);
    let mut stderr_take = stderr_handle.take(MAX_OUTPUT_LEN as u64);
    let (stdout_res, stderr_res) = tokio::join!(
        stdout_take.read_to_end(&mut stdout_buf),
        stderr_take.read_to_end(&mut stderr_buf),
    );
    stdout_res.map_err(|e| format!("Failed to read stdout: {e}"))?;
    stderr_res.map_err(|e| format!("Failed to read stderr: {e}"))?;

    let status = child
        .wait()
        .await
        .map_err(|e| format!("Failed to wait for gh: {e}"))?;

    let stdout = String::from_utf8_lossy(&stdout_buf);
    let stderr = String::from_utf8_lossy(&stderr_buf);

    if status.success() {
        Ok(stdout.into_owned())
    } else {
        let code_display = status
            .code()
            .map(|c| format!("exit code {c}"))
            .unwrap_or_else(|| "unknown exit code".to_string());

        let mut err = format!("gh {code_display}");
        if !stderr.is_empty() {
            err.push_str(": ");
            err.push_str(stderr.trim());
        } else if !stdout.is_empty() {
            err.push_str(": ");
            err.push_str(stdout.trim());
        }

        // Log for observability but return the full error to the agent
        warn!(
            args = ?args,
            exit_code = status.code(),
            stderr = %stderr.trim(),
            "gh subprocess failed"
        );

        Err(err)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::test_helpers::TestHarness;

    // -- classify_checks tests --

    #[test]
    fn classify_all_passed() {
        let checks = vec![
            GhCheck {
                name: "CI".to_string(),
                state: "SUCCESS".to_string(),
                bucket: "pass".to_string(),
                link: None,
            },
            GhCheck {
                name: "Lint".to_string(),
                state: "SUCCESS".to_string(),
                bucket: "pass".to_string(),
                link: None,
            },
        ];
        assert_eq!(classify_checks(&checks), CheckClassification::AllPassed);
    }

    #[test]
    fn classify_with_skipped() {
        let checks = vec![
            GhCheck {
                name: "CI".to_string(),
                state: "SUCCESS".to_string(),
                bucket: "pass".to_string(),
                link: None,
            },
            GhCheck {
                name: "Optional".to_string(),
                state: "SKIPPED".to_string(),
                bucket: "skipping".to_string(),
                link: None,
            },
        ];
        assert_eq!(classify_checks(&checks), CheckClassification::AllPassed);
    }

    #[test]
    fn classify_empty_checks() {
        assert_eq!(classify_checks(&[]), CheckClassification::AllPassed);
    }

    #[test]
    fn classify_has_pending() {
        let checks = vec![
            GhCheck {
                name: "CI".to_string(),
                state: "SUCCESS".to_string(),
                bucket: "pass".to_string(),
                link: None,
            },
            GhCheck {
                name: "Build".to_string(),
                state: "IN_PROGRESS".to_string(),
                bucket: "pending".to_string(),
                link: None,
            },
        ];
        assert_eq!(classify_checks(&checks), CheckClassification::HasPending);
    }

    #[test]
    fn classify_has_failures() {
        let checks = vec![
            GhCheck {
                name: "CI".to_string(),
                state: "FAILURE".to_string(),
                bucket: "fail".to_string(),
                link: Some("https://github.com/org/repo/actions/runs/123".to_string()),
            },
            GhCheck {
                name: "Lint".to_string(),
                state: "SUCCESS".to_string(),
                bucket: "pass".to_string(),
                link: None,
            },
        ];
        assert_eq!(classify_checks(&checks), CheckClassification::HasFailures);
    }

    #[test]
    fn classify_cancelled_is_failure() {
        let checks = vec![GhCheck {
            name: "CI".to_string(),
            state: "CANCELLED".to_string(),
            bucket: "cancel".to_string(),
            link: None,
        }];
        assert_eq!(classify_checks(&checks), CheckClassification::HasFailures);
    }

    #[test]
    fn classify_mixed_failure_and_pending_is_failure() {
        let checks = vec![
            GhCheck {
                name: "CI".to_string(),
                state: "FAILURE".to_string(),
                bucket: "fail".to_string(),
                link: None,
            },
            GhCheck {
                name: "Build".to_string(),
                state: "PENDING".to_string(),
                bucket: "pending".to_string(),
                link: None,
            },
        ];
        // Failures take priority over pending
        assert_eq!(classify_checks(&checks), CheckClassification::HasFailures);
    }

    // -- validate_repo tests --

    #[test]
    fn validate_repo_valid() {
        assert!(validate_repo("senara-solutions/mika").is_ok());
        assert!(validate_repo("owner/repo").is_ok());
        assert!(validate_repo("my-org/my.repo").is_ok());
        assert!(validate_repo("org_name/repo_name").is_ok());
    }

    #[test]
    fn validate_repo_invalid() {
        assert!(validate_repo("").is_err());
        assert!(validate_repo("noslash").is_err());
        assert!(validate_repo("too/many/slashes").is_err());
        assert!(validate_repo("has spaces/repo").is_err());
        assert!(validate_repo("owner/").is_err());
        assert!(validate_repo("/repo").is_err());
        assert!(validate_repo("https://github.com/owner/repo").is_err());
        assert!(validate_repo("owner/repo --flag").is_err());
    }

    #[test]
    fn validate_repo_too_long() {
        let long_repo = format!("{}/{}", "a".repeat(100), "b".repeat(101));
        assert!(validate_repo(&long_repo).is_err());
    }

    // -- validate_merge_method tests --

    #[test]
    fn validate_merge_method_valid() {
        assert!(validate_merge_method("squash").is_ok());
        assert!(validate_merge_method("merge").is_ok());
        assert!(validate_merge_method("rebase").is_ok());
    }

    #[test]
    fn validate_merge_method_invalid() {
        assert!(validate_merge_method("delete").is_err());
        assert!(validate_merge_method("squash --title evil").is_err());
        assert!(validate_merge_method("").is_err());
    }

    // -- GhCheck deserialization tests --

    #[test]
    fn deserialize_gh_checks_output() {
        let json = r#"[
            {
                "name": "CI / test",
                "state": "SUCCESS",
                "bucket": "pass",
                "link": "https://github.com/org/repo/actions/runs/123"
            },
            {
                "name": "Pipeline Artifacts",
                "state": "FAILURE",
                "bucket": "fail",
                "link": "https://github.com/org/repo/actions/runs/456"
            }
        ]"#;

        let checks: Vec<GhCheck> = serde_json::from_str(json).unwrap();
        assert_eq!(checks.len(), 2);
        assert_eq!(checks[0].name, "CI / test");
        assert_eq!(checks[0].bucket, "pass");
        assert_eq!(checks[1].name, "Pipeline Artifacts");
        assert_eq!(checks[1].bucket, "fail");
    }

    #[test]
    fn deserialize_gh_checks_no_link() {
        let json = r#"[{"name": "test", "state": "PENDING", "bucket": "pending"}]"#;
        let checks: Vec<GhCheck> = serde_json::from_str(json).unwrap();
        assert_eq!(checks.len(), 1);
        assert!(checks[0].link.is_none());
    }

    // -- MergeGateResult serialization tests --

    #[test]
    fn serialize_merged_result() {
        let result = MergeGateResult::Merged;
        let json = serde_json::to_value(&result).unwrap();
        assert_eq!(json["action"], "merged");
    }

    #[test]
    fn serialize_blocked_result() {
        let result = MergeGateResult::Blocked {
            failing_checks: vec![CheckInfo {
                name: "CI".to_string(),
                state: "FAILURE".to_string(),
                link: Some("https://example.com".to_string()),
            }],
        };
        let json = serde_json::to_value(&result).unwrap();
        assert_eq!(json["action"], "blocked");
        assert_eq!(json["failing_checks"][0]["name"], "CI");
        assert_eq!(json["failing_checks"][0]["link"], "https://example.com");
    }

    #[test]
    fn serialize_auto_merge_result() {
        let result = MergeGateResult::AutoMergeEnabled {
            pending_checks: vec![CheckInfo {
                name: "Build".to_string(),
                state: "IN_PROGRESS".to_string(),
                link: None,
            }],
        };
        let json = serde_json::to_value(&result).unwrap();
        assert_eq!(json["action"], "auto_merge_enabled");
        assert_eq!(json["pending_checks"][0]["name"], "Build");
        // link should be absent (skip_serializing_if = None)
        assert!(json["pending_checks"][0].get("link").is_none());
    }

    #[test]
    fn serialize_already_merged_result() {
        let result = MergeGateResult::AlreadyMerged;
        let json = serde_json::to_value(&result).unwrap();
        assert_eq!(json["action"], "already_merged");
    }

    // -- Tool integration tests --

    #[tokio::test]
    async fn test_missing_github_token() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        // ctx.github_token is None by default in TestHarness

        let tool = PrMergeWithGateTool;
        let input = json!({
            "pr_number": 42,
            "repo": "senara-solutions/mika"
        });

        let result = tool.execute(input, &ctx).await.unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("GitHub token required"));
    }

    #[tokio::test]
    async fn test_invalid_repo_format() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();

        let tool = PrMergeWithGateTool;
        let input = json!({
            "pr_number": 42,
            "repo": "not-a-valid-repo"
        });

        let result = tool.execute(input, &ctx).await.unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("Invalid repo format"));
    }

    #[tokio::test]
    async fn test_invalid_merge_method() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();

        let tool = PrMergeWithGateTool;
        let input = json!({
            "pr_number": 42,
            "repo": "owner/repo",
            "merge_method": "delete-everything"
        });

        let result = tool.execute(input, &ctx).await.unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("Invalid merge_method"));
    }

    #[tokio::test]
    async fn test_missing_pr_number() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();

        let tool = PrMergeWithGateTool;
        let input = json!({
            "repo": "owner/repo"
        });

        let result = tool.execute(input, &ctx).await.unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("pr_number"));
    }

    #[tokio::test]
    async fn test_zero_pr_number() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();

        let tool = PrMergeWithGateTool;
        let input = json!({
            "pr_number": 0,
            "repo": "owner/repo"
        });

        let result = tool.execute(input, &ctx).await.unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("pr_number"));
    }

    #[test]
    fn test_tool_definition() {
        let tool = PrMergeWithGateTool;
        assert_eq!(tool.name(), "pr_merge_with_gate");
        assert_eq!(tool.timeout_secs(), Some(60));

        let def = tool.definition();
        assert_eq!(def.name, "pr_merge_with_gate");
        assert!(def.description.contains("CI gate"));
        assert!(def.description.contains("auto_merge_enabled"));

        // Verify schema has required fields
        let props = &def.input_schema["properties"];
        assert!(props.get("pr_number").is_some());
        assert!(props.get("repo").is_some());
        assert!(props.get("merge_method").is_some());
        assert!(props.get("delete_branch").is_some());

        let required = def.input_schema["required"].as_array().unwrap();
        assert!(required.contains(&json!("pr_number")));
        assert!(required.contains(&json!("repo")));
    }
}
