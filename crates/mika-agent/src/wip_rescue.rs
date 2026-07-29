//! Auto-resume WIP-rescue drafts (mika#1852, RT#004).
//!
//! A periodic, low-priority scan that picks up the draft PRs dispatch-lib
//! creates when a pilot session rescues uncommitted / un-PR'd work (the
//! `wip-rescue` label, mika#1282 / mika#1396) and drives them back toward a
//! reviewable state:
//!
//! ```text
//!   scan drafts (age > threshold)
//!     └─ RESCUE_DEPTH gate  (bail-to-human at max)          ── F2
//!        └─ git ≥ 2.38 guard (bail: git-too-old-for-dry-run) ── F3
//!           └─ dry-run rebase (merge-tree, non-mutating)     ── AC2
//!              └─ live rebase onto origin/main
//!                 └─ clippy gate (bail on errors)
//!                    └─ push rebased branch (force-with-lease)
//!                       └─ substrate-diff perimeter classify  ── AC4 (reuse #1831)
//!                          └─ un-draft (gh pr ready)          ── F1
//! ```
//!
//! ## Safety invariants
//!
//! - **Never mutate before a clean dry-run.** The `git merge-tree --write-tree`
//!   dry-run (non-mutating: writes only loose objects, never the worktree or a
//!   ref) runs before the live rebase. If the deploy host's git predates 2.38
//!   (no `--write-tree`), we **fail closed** — bail-to-human rather than fall
//!   back to a mutating rebase that would skip the dry-run (F3).
//! - **Bail-to-human is terminal.** Any uncertain condition (rebase conflict,
//!   clippy errors, un-draft failure, depth exhausted) adds the
//!   `human-review-required` label + a PR comment naming the reason and ENDS
//!   the chain. The draft is preserved; a human decides. No further
//!   auto-attempts (AC3).
//! - **Perimeter gate is authoritative (AC4/AC5).** Every draft — even a
//!   one-line diff — is classified by the mika#1831 perimeter classifier
//!   ([`crate::perimeter`]). A DECISION-CORE draft is un-drafted **with** a
//!   hand-merge comment (never auto-merged); a MECHANICAL draft is un-drafted
//!   normally so the verdict handler's merge path can fire. There is no
//!   trivial-diff carve-out.
//! - **Concurrency cap of 1 (AC6).** The scan processes at most one draft per
//!   tick (oldest-eligible first). Excess drafts wait for the next tick.
//!
//! ## Scope boundary (v1)
//!
//! The plan's Step 4 (clippy-fix dispatch to mika-dev) and Step 5 (re-run
//! pilot) call for dispatching a fresh claude-pilot session. Dispatching a
//! pilot is an LLM-turn / webhook-driven primitive — it is not available from
//! this cron-driven daemon function (the sibling `auto_pull` scanner labels and
//! lets the webhook flow dispatch, rather than dispatching inline). So in v1 a
//! branch that does not rebase-clean or does not pass clippy **bails to human**
//! with a distinct reason rather than auto-dispatching a fix. The happy path —
//! branch rebases clean, clippy passes → perimeter gate → un-draft → qa-review
//! → verdict — is fully automated. Auto-dispatch of the fix loop is a tracked
//! follow-up.
//!
//! The local checkout the git steps operate on is resolved from
//! `MIKA_WIP_RESCUE_REPO_DIR`; when unset or invalid the scan is a safe no-op
//! (drafts are left untouched, `wip_rescue_skipped` emitted).

use crate::async_db::AsyncDatabase;
use crate::perimeter::{self, Classification};
use crate::tools::pr_merge_with_gate::run_gh_subprocess;
use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tracing::{debug, info, warn};

/// The repository the wip-rescue scan targets. Matches `auto_pull::DEFAULT_REPO`.
const DEFAULT_REPO: &str = "senara-solutions/mika";

/// Label dispatch-lib applies to rescued draft PRs (mika#1631). The scan only
/// ever touches drafts carrying this label.
const WIP_RESCUE_LABEL: &str = "wip-rescue";

/// Terminal escalation label applied by [`bail_to_human`].
const HUMAN_REVIEW_LABEL: &str = "human-review-required";

// -- Env-var knobs (three-tier: absent → default, invalid → WARN + default) --

const MIN_AGE_ENV: &str = "MIKA_WIP_RESCUE_MIN_AGE_SECS";
const MIN_AGE_DEFAULT_SECS: i64 = 900; // 15 min

const MAX_DEPTH_ENV: &str = "MIKA_WIP_RESCUE_MAX_DEPTH";
const MAX_DEPTH_DEFAULT: i64 = 2;

/// Directory of a local `mika` checkout the git steps operate against. When
/// unset/invalid the scan is a no-op (see module docs).
const REPO_DIR_ENV: &str = "MIKA_WIP_RESCUE_REPO_DIR";

/// Per-subprocess timeout for the `gh` calls in the chain (matches the other
/// gh calls elsewhere in the module).
const GH_TIMEOUT: Duration = Duration::from_secs(30);

/// Timeout for git plumbing calls (fetch / rebase / push may hit the network).
const GIT_TIMEOUT: Duration = Duration::from_secs(180);

/// Timeout for the clippy gate — a cold compile can be slow.
const CLIPPY_TIMEOUT: Duration = Duration::from_secs(900);

/// Minimum git version supporting `git merge-tree --write-tree` (F3).
const GIT_MIN_MAJOR: u32 = 2;
const GIT_MIN_MINOR: u32 = 38;

// ---------------------------------------------------------------------------
// Pure, unit-testable helpers
// ---------------------------------------------------------------------------

/// Parse the min-age threshold from an optional env value (three-tier).
///
/// Absent / empty / unparseable / negative → [`MIN_AGE_DEFAULT_SECS`]
/// (WARN on an explicitly-invalid value).
fn parse_min_age(raw: Option<&str>) -> i64 {
    match raw {
        Some(v) if !v.trim().is_empty() => match v.trim().parse::<i64>() {
            Ok(n) if n >= 0 => n,
            _ => {
                warn!(
                    value = %v,
                    default = MIN_AGE_DEFAULT_SECS,
                    "wip_rescue: invalid {MIN_AGE_ENV}, using default"
                );
                MIN_AGE_DEFAULT_SECS
            }
        },
        _ => MIN_AGE_DEFAULT_SECS,
    }
}

/// Parse the max rescue-depth from an optional env value (three-tier).
///
/// Absent / empty / unparseable / negative → [`MAX_DEPTH_DEFAULT`]
/// (WARN on an explicitly-invalid value).
fn parse_max_depth(raw: Option<&str>) -> i64 {
    match raw {
        Some(v) if !v.trim().is_empty() => match v.trim().parse::<i64>() {
            Ok(n) if n >= 0 => n,
            _ => {
                warn!(
                    value = %v,
                    default = MAX_DEPTH_DEFAULT,
                    "wip_rescue: invalid {MAX_DEPTH_ENV}, using default"
                );
                MAX_DEPTH_DEFAULT
            }
        },
        _ => MAX_DEPTH_DEFAULT,
    }
}

fn min_age_secs() -> i64 {
    parse_min_age(std::env::var(MIN_AGE_ENV).ok().as_deref())
}

fn max_depth() -> i64 {
    parse_max_depth(std::env::var(MAX_DEPTH_ENV).ok().as_deref())
}

/// Age of a draft in whole seconds, given its `createdAt` and a reference
/// `now`. `None` when either timestamp is unparseable. Negative clamped to 0
/// (clock skew: a draft created "in the future" is treated as age 0).
fn draft_age_secs(created_at: &str, now: &str) -> Option<i64> {
    let created = crate::timestamp::parse(created_at).ok()?;
    let now = crate::timestamp::parse(now).ok()?;
    Some((now - created).num_seconds().max(0))
}

/// Read the current rescue depth from a task's metadata JSON.
///
/// Reads `$.wip_rescue.depth`. Absent key, absent object, non-integer, or
/// unparseable metadata all read as `0` — a missing counter means "no prior
/// attempt recorded", which is the safe under-count (we would rather attempt
/// once more than strand real work).
fn read_rescue_depth(metadata: Option<&str>) -> i64 {
    let Some(raw) = metadata else { return 0 };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(raw) else {
        return 0;
    };
    value
        .get("wip_rescue")
        .and_then(|v| v.get("depth"))
        .and_then(|v| v.as_i64())
        .unwrap_or(0)
}

/// Produce the merged metadata JSON string that bumps `$.wip_rescue.depth` to
/// `new_depth`, preserving every other field via the shared two-level shallow
/// merge ([`crate::task_state::metadata::merge_metadata`]).
fn bump_rescue_depth_metadata(existing: Option<&str>, new_depth: i64) -> String {
    let mut base = existing
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok())
        .filter(|v| v.is_object())
        .unwrap_or_else(|| serde_json::json!({}));
    let incoming = serde_json::json!({ "wip_rescue": { "depth": new_depth } });
    crate::task_state::metadata::merge_metadata(&mut base, &incoming);
    base.to_string()
}

/// Parse a `git --version` line into `(major, minor, patch)`.
///
/// Accepts the canonical `git version 2.39.5` shape and vendor suffixes
/// (`2.39.3 (Apple Git-146)`). `None` when no `<maj>.<min>` prefix is found.
fn parse_git_version(output: &str) -> Option<(u32, u32, u32)> {
    let token = output
        .split_whitespace()
        .find(|t| t.chars().next().is_some_and(|c| c.is_ascii_digit()) && t.contains('.'))?;
    let mut parts = token.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    // Patch may carry a non-numeric suffix ("5-rc0"); take the leading digits.
    let patch = parts
        .next()
        .map(|p| {
            p.chars()
                .take_while(|c| c.is_ascii_digit())
                .collect::<String>()
                .parse()
                .unwrap_or(0)
        })
        .unwrap_or(0);
    Some((major, minor, patch))
}

/// Whether a parsed git version supports `git merge-tree --write-tree` (≥ 2.38).
fn version_supports_writetree(v: (u32, u32, u32)) -> bool {
    let (major, minor, _) = v;
    major > GIT_MIN_MAJOR || (major == GIT_MIN_MAJOR && minor >= GIT_MIN_MINOR)
}

/// The un-draft routing decision for a classified draft (AC4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UndraftRoute {
    /// All-mechanical diff — un-draft normally; the verdict handler may merge.
    Mechanical,
    /// Touches a decision-core zone — un-draft but leave a hand-merge comment;
    /// never auto-merge.
    DecisionCore,
}

impl From<Classification> for UndraftRoute {
    fn from(c: Classification) -> Self {
        match c {
            Classification::Mechanical => UndraftRoute::Mechanical,
            Classification::DecisionCore => UndraftRoute::DecisionCore,
        }
    }
}

// ---------------------------------------------------------------------------
// GitHub JSON shapes
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct DraftPr {
    number: u64,
    #[serde(rename = "headRefName")]
    head_ref: String,
    #[allow(dead_code)]
    title: String,
    #[serde(default)]
    labels: Vec<GhLabel>,
    #[serde(rename = "createdAt")]
    created_at: String,
}

#[derive(Debug, Deserialize)]
struct GhLabel {
    name: String,
}

impl DraftPr {
    fn has_label(&self, name: &str) -> bool {
        self.labels
            .iter()
            .any(|l| l.name.eq_ignore_ascii_case(name))
    }
}

#[derive(Debug, Deserialize)]
struct ClosingIssueRef {
    number: u64,
}

#[derive(Debug, Deserialize)]
struct ClosingIssuesEnvelope {
    #[serde(rename = "closingIssuesReferences", default)]
    closing_issues_references: Vec<ClosingIssueRef>,
}

// ---------------------------------------------------------------------------
// Subprocess wrappers (timeout-bounded)
// ---------------------------------------------------------------------------

/// `gh` call with a bounded timeout. Reuses the crate's token-injecting +
/// env-scrubbing subprocess wrapper; adds the per-call timeout the plan
/// specifies (the underlying wrapper has none).
async fn gh(args: &[&str], token: &str) -> Result<String, String> {
    match tokio::time::timeout(GH_TIMEOUT, run_gh_subprocess(args, token)).await {
        Ok(res) => res,
        Err(_) => Err(format!("gh timed out after {}s", GH_TIMEOUT.as_secs())),
    }
}

/// Run a `git` subcommand in `dir` with a bounded timeout, scrubbing MIKA_*/
/// GH_TOKEN and disabling terminal prompts (matches the git-subprocess hygiene
/// used elsewhere — see root CLAUDE.md § Secrets).
async fn git(dir: &Path, args: &[&str], timeout: Duration) -> Result<String, String> {
    let mut cmd = tokio::process::Command::new("git");
    cmd.arg("-C").arg(dir).args(args);
    cmd.env("GIT_TERMINAL_PROMPT", "0");
    crate::skills::executor::scrub_mika_env_vars(&mut cmd);
    cmd.stdin(std::process::Stdio::null());
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());
    cmd.kill_on_drop(true);
    run_capturing(cmd, timeout, "git").await
}

/// Run `cargo clippy --tests -- -D warnings` against the worktree manifest.
async fn clippy(worktree: &Path) -> Result<String, String> {
    let manifest = worktree.join("Cargo.toml");
    let manifest_str = manifest.to_string_lossy().into_owned();
    let mut cmd = tokio::process::Command::new("cargo");
    cmd.args([
        "clippy",
        "--manifest-path",
        &manifest_str,
        "--tests",
        "--",
        "-D",
        "warnings",
    ]);
    crate::skills::executor::scrub_mika_env_vars(&mut cmd);
    cmd.stdin(std::process::Stdio::null());
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());
    cmd.kill_on_drop(true);
    run_capturing(cmd, CLIPPY_TIMEOUT, "clippy").await
}

/// Shared spawn+capture: returns `Ok(stdout)` on exit 0, else `Err` with an
/// exit-code + stderr snippet, or a timeout marker.
async fn run_capturing(
    mut cmd: tokio::process::Command,
    timeout: Duration,
    what: &str,
) -> Result<String, String> {
    let fut = cmd.output();
    let output = match tokio::time::timeout(timeout, fut).await {
        Ok(Ok(o)) => o,
        Ok(Err(e)) => return Err(format!("{what} spawn failed: {e}")),
        Err(_) => return Err(format!("{what} timed out after {}s", timeout.as_secs())),
    };
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    } else {
        let code = output
            .status
            .code()
            .map(|c| c.to_string())
            .unwrap_or_else(|| "signal".to_string());
        let stderr = String::from_utf8_lossy(&output.stderr);
        let snippet: String = stderr.chars().take(400).collect();
        Err(format!("{what} exit {code}: {snippet}"))
    }
}

// ---------------------------------------------------------------------------
// Chain orchestration
// ---------------------------------------------------------------------------

/// Outcome of a single draft's rescue attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
enum ChainOutcome {
    /// Un-drafted (resumed). Carries the route taken for observability.
    Resumed(UndraftRoute),
    /// Escalated to a human with the given reason. Terminal.
    Bailed(String),
    /// Not attempted this tick (e.g., no local checkout). Left untouched.
    Skipped(String),
}

/// Scan open `wip-rescue` draft PRs and resume the oldest eligible one.
///
/// Returns `Some(1)` when a draft was un-drafted, `Some(0)` when a draft was
/// examined but bailed/skipped, `None` when nothing was eligible or the scan
/// could not run. Processes at most one draft per invocation (AC6 concurrency
/// cap = 1); excess drafts wait for the next scan tick.
pub async fn auto_resume_wip_rescue_drafts(
    db: &AsyncDatabase,
    github_token: &str,
    trace_id: &str,
    session_id: &str,
) -> Option<usize> {
    let drafts = match list_wip_rescue_drafts(github_token).await {
        Ok(d) => d,
        Err(e) => {
            warn!(error = %e, trace_id, "wip_rescue_error");
            return None;
        }
    };

    let threshold = min_age_secs();
    let now = crate::timestamp::now();

    // Eligible = wip-rescue-labelled, not already bailed, older than threshold.
    // Oldest-first so the most-stale draft is rescued first.
    let mut eligible: Vec<(i64, DraftPr)> = drafts
        .into_iter()
        .filter(|pr| pr.has_label(WIP_RESCUE_LABEL) && !pr.has_label(HUMAN_REVIEW_LABEL))
        .filter_map(|pr| {
            let age = draft_age_secs(&pr.created_at, &now)?;
            (age >= threshold).then_some((age, pr))
        })
        .collect();
    eligible.sort_by(|a, b| b.0.cmp(&a.0));

    let Some((age_secs, pr)) = eligible.into_iter().next() else {
        debug!(
            trace_id,
            threshold, "wip_rescue: no eligible drafts this tick"
        );
        return None;
    };

    match resume_chain(db, github_token, trace_id, session_id, &pr, age_secs).await {
        ChainOutcome::Resumed(route) => {
            info!(
                pr_number = pr.number,
                classification = ?route,
                trace_id,
                "wip_rescue_success"
            );
            log_audit(
                db,
                session_id,
                "wip_rescue_success",
                pr.number,
                trace_id,
                &format!("{route:?}"),
            )
            .await;
            Some(1)
        }
        ChainOutcome::Bailed(reason) => {
            info!(pr_number = pr.number, reason = %reason, trace_id, "wip_rescue: bailed");
            Some(0)
        }
        ChainOutcome::Skipped(reason) => {
            debug!(pr_number = pr.number, reason = %reason, trace_id, "wip_rescue_skipped");
            Some(0)
        }
    }
}

/// The per-draft cost-bounded chain (AC2). See module docs for the step map.
async fn resume_chain(
    db: &AsyncDatabase,
    token: &str,
    trace_id: &str,
    session_id: &str,
    pr: &DraftPr,
    age_secs: i64,
) -> ChainOutcome {
    // Resolve the parent task + current rescue depth (F2).
    let (parent_task, rescue_depth) = resolve_parent_depth(db, token, pr, trace_id).await;

    info!(
        pr_number = pr.number,
        age_secs, rescue_depth, trace_id, "wip_rescue_resume_attempt"
    );

    // Step 1: RESCUE_DEPTH gate (AC2/AC3).
    if rescue_depth >= max_depth() {
        return bail_to_human(
            token,
            trace_id,
            session_id,
            db,
            pr.number,
            format!("rescue-depth-exceeded ({rescue_depth} ≥ {})", max_depth()),
        )
        .await;
    }

    // Step F3: fail-closed git version guard (must precede any git mutation).
    match git_version().await {
        Some(v) if version_supports_writetree(v) => {}
        Some(v) => {
            return bail_to_human(
                token,
                trace_id,
                session_id,
                db,
                pr.number,
                format!("git-too-old-for-dry-run:{}.{}.{}", v.0, v.1, v.2),
            )
            .await;
        }
        None => {
            warn!(pr_number = pr.number, trace_id, "wip_rescue_error");
            return bail_to_human(
                token,
                trace_id,
                session_id,
                db,
                pr.number,
                "git-too-old-for-dry-run:unknown".to_string(),
            )
            .await;
        }
    }

    // Steps 2–5: rebase (dry-run → live) + clippy against a local checkout.
    let Some(repo_dir) = resolve_repo_dir() else {
        return ChainOutcome::Skipped("no_repo_dir".to_string());
    };
    match prepare_branch(&repo_dir, &pr.head_ref, pr.number, trace_id).await {
        PrepareOutcome::Ready => {}
        PrepareOutcome::Bail(reason) => {
            return bail_to_human(token, trace_id, session_id, db, pr.number, reason).await;
        }
        PrepareOutcome::Skip(reason) => {
            return ChainOutcome::Skipped(reason);
        }
    }

    // Step 6: substrate-diff perimeter classification (AC4/AC5). Fail-closed —
    // a fetch/parse failure classifies DECISION-CORE (never auto-merge).
    let route = classify_route(pr.number, token).await;

    // Step 7: un-draft (F1). Any failure bails; the draft stays a draft.
    if let Err(e) = gh(
        &[
            "pr",
            "ready",
            &pr.number.to_string(),
            "--repo",
            DEFAULT_REPO,
        ],
        token,
    )
    .await
    {
        let snippet: String = e.chars().take(200).collect();
        return bail_to_human(
            token,
            trace_id,
            session_id,
            db,
            pr.number,
            format!("un-draft-failed:{snippet}"),
        )
        .await;
    }

    // DECISION-CORE → leave a hand-merge comment (do NOT auto-merge).
    if route == UndraftRoute::DecisionCore {
        let body = "Auto-resumed by wip-rescue (mika#1852). Substrate-diff \
                    classified this PR as DECISION-CORE — it touches a gated \
                    zone. Un-drafted for review, but the autonomous merge path \
                    will NOT fire: this needs a Vincent hand-merge.";
        if let Err(e) = gh(
            &[
                "pr",
                "comment",
                &pr.number.to_string(),
                "--repo",
                DEFAULT_REPO,
                "--body",
                body,
            ],
            token,
        )
        .await
        {
            warn!(pr_number = pr.number, error = %e, trace_id, "wip_rescue_error");
        }
    }

    // Increment the rescue depth on the parent task (best-effort).
    if let Some(task) = parent_task {
        let merged = bump_rescue_depth_metadata(task.metadata.as_deref(), rescue_depth + 1);
        if let Err(e) = db.update_task_metadata(&task.id, &merged).await {
            warn!(task_id = %task.id, error = %e, trace_id, "wip_rescue: depth bump failed");
        }
    }

    ChainOutcome::Resumed(route)
}

/// Resolve the parent task row and current rescue depth for a draft (F2).
///
/// PR → `closingIssuesReferences` → issue URL → active task by `reference_url`.
/// When no task row is found the depth defaults to 0 and `wip_rescue_skipped`
/// (reason `no_parent_task`) is emitted — the draft still proceeds (a missing
/// ledger row must not strand real work).
async fn resolve_parent_depth(
    db: &AsyncDatabase,
    token: &str,
    pr: &DraftPr,
    trace_id: &str,
) -> (Option<crate::task_state::tasks::Task>, i64) {
    let Some(issue_number) = closing_issue_number(pr.number, token).await else {
        debug!(
            pr_number = pr.number,
            reason = "no_closing_issue",
            trace_id,
            "wip_rescue_skipped"
        );
        return (None, 0);
    };
    let issue_url = format!("https://github.com/{DEFAULT_REPO}/issues/{issue_number}");
    match db.find_active_task_by_ref_url(&issue_url).await {
        Ok(Some(task)) => {
            let depth = read_rescue_depth(task.metadata.as_deref());
            (Some(task), depth)
        }
        Ok(None) => {
            debug!(
                pr_number = pr.number,
                reason = "no_parent_task",
                trace_id,
                "wip_rescue_skipped"
            );
            (None, 0)
        }
        Err(e) => {
            warn!(pr_number = pr.number, error = %e, trace_id, "wip_rescue_error");
            (None, 0)
        }
    }
}

/// First closing-issue number for a PR (wip-rescue drafts carry a single
/// `Closes #<issue>`).
async fn closing_issue_number(pr_number: u64, token: &str) -> Option<u64> {
    let out = gh(
        &[
            "pr",
            "view",
            &pr_number.to_string(),
            "--repo",
            DEFAULT_REPO,
            "--json",
            "closingIssuesReferences",
        ],
        token,
    )
    .await
    .ok()?;
    let env: ClosingIssuesEnvelope = serde_json::from_str(out.trim()).ok()?;
    env.closing_issues_references.first().map(|r| r.number)
}

/// Perimeter classification route for a PR, fail-closed to DECISION-CORE on any
/// fetch error (mirrors the merge-path callers of mika#1831).
async fn classify_route(pr_number: u64, token: &str) -> UndraftRoute {
    match perimeter::fetch::fetch_pr_files(pr_number, DEFAULT_REPO, token).await {
        Ok(files) => perimeter::classify_pr_files(&files).verdict.into(),
        Err(_) => UndraftRoute::DecisionCore,
    }
}

/// Outcome of the git rebase + clippy preparation.
enum PrepareOutcome {
    Ready,
    Bail(String),
    Skip(String),
}

/// Steps 2–5: fetch, dry-run rebase, live rebase, clippy, push. Operates on an
/// ephemeral detached worktree under the local checkout; always cleans up.
async fn prepare_branch(
    repo_dir: &Path,
    branch: &str,
    pr_number: u64,
    trace_id: &str,
) -> PrepareOutcome {
    // Fetch the branch + main.
    if let Err(e) = git(repo_dir, &["fetch", "origin", "main", branch], GIT_TIMEOUT).await {
        return PrepareOutcome::Skip(format!("fetch_failed:{}", first_line(&e)));
    }

    let worktree = repo_dir.join(format!(".wip-rescue-wt/pr-{pr_number}"));
    let worktree_str = worktree.to_string_lossy().into_owned();
    // Best-effort clean of a stale worktree from a prior interrupted run.
    let _ = git(
        repo_dir,
        &["worktree", "remove", "--force", &worktree_str],
        GIT_TIMEOUT,
    )
    .await;

    if let Err(e) = git(
        repo_dir,
        &[
            "worktree",
            "add",
            "--detach",
            &worktree_str,
            &format!("origin/{branch}"),
        ],
        GIT_TIMEOUT,
    )
    .await
    {
        return PrepareOutcome::Skip(format!("worktree_add_failed:{}", first_line(&e)));
    }

    let outcome = prepare_in_worktree(&worktree, branch, trace_id).await;

    // Always clean up the ephemeral worktree.
    let _ = git(
        repo_dir,
        &["worktree", "remove", "--force", &worktree_str],
        GIT_TIMEOUT,
    )
    .await;
    outcome
}

/// The mutation-bearing body of [`prepare_branch`], run inside the worktree.
async fn prepare_in_worktree(worktree: &Path, branch: &str, trace_id: &str) -> PrepareOutcome {
    // Step 2: dry-run rebase (non-mutating). merge-tree exits non-zero on
    // conflict. Runs BEFORE any mutation (safety invariant).
    if git(
        worktree,
        &["merge-tree", "--write-tree", "origin/main", "HEAD"],
        GIT_TIMEOUT,
    )
    .await
    .is_err()
    {
        return PrepareOutcome::Bail("rebase-conflict-on-main".to_string());
    }

    // Step 3: live rebase onto origin/main.
    if let Err(e) = git(worktree, &["rebase", "origin/main"], GIT_TIMEOUT).await {
        let _ = git(worktree, &["rebase", "--abort"], GIT_TIMEOUT).await;
        return PrepareOutcome::Bail(format!("rebase-failed:{}", first_line(&e)));
    }

    // Step 4: clippy gate. On errors, bail — auto-dispatch of the fix loop is a
    // tracked follow-up (see module § Scope boundary).
    if let Err(e) = clippy(worktree).await {
        debug!(trace_id, error = %first_line(&e), "wip_rescue: clippy gate failed");
        return PrepareOutcome::Bail("clippy-errors-need-human".to_string());
    }

    // Step 5: publish the rebased branch so the un-drafted PR reflects it.
    if let Err(e) = git(
        worktree,
        &[
            "push",
            "--force-with-lease",
            "origin",
            &format!("HEAD:refs/heads/{branch}"),
        ],
        GIT_TIMEOUT,
    )
    .await
    {
        return PrepareOutcome::Bail(format!("push-failed:{}", first_line(&e)));
    }

    PrepareOutcome::Ready
}

/// Run `git --version` (in the process CWD) and parse it. `None` on any error.
async fn git_version() -> Option<(u32, u32, u32)> {
    let mut cmd = tokio::process::Command::new("git");
    cmd.arg("--version");
    cmd.stdin(std::process::Stdio::null());
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());
    let out = tokio::time::timeout(Duration::from_secs(10), cmd.output())
        .await
        .ok()?
        .ok()?;
    if !out.status.success() {
        return None;
    }
    parse_git_version(&String::from_utf8_lossy(&out.stdout))
}

/// Resolve + validate the local checkout dir from [`REPO_DIR_ENV`].
fn resolve_repo_dir() -> Option<PathBuf> {
    let raw = std::env::var(REPO_DIR_ENV).ok()?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    let path = PathBuf::from(trimmed);
    path.is_dir().then_some(path)
}

/// Bail-to-human: label + comment + structured event; terminal (AC3).
async fn bail_to_human(
    token: &str,
    trace_id: &str,
    session_id: &str,
    db: &AsyncDatabase,
    pr_number: u64,
    reason: String,
) -> ChainOutcome {
    warn!(pr_number, reason = %reason, trace_id, "wip_rescue_bail_to_human");

    if let Err(e) = gh(
        &[
            "pr",
            "edit",
            &pr_number.to_string(),
            "--repo",
            DEFAULT_REPO,
            "--add-label",
            HUMAN_REVIEW_LABEL,
        ],
        token,
    )
    .await
    {
        warn!(pr_number, error = %e, trace_id, "wip_rescue_error");
    }

    let comment = format!(
        "Auto-resume (wip-rescue, mika#1852) stopped and handed this PR to a \
         human.\n\n**Reason:** `{reason}`\n\nNo further auto-attempts will be \
         made. Resolve the condition above, then remove the \
         `{HUMAN_REVIEW_LABEL}` label to re-arm the scan."
    );
    if let Err(e) = gh(
        &[
            "pr",
            "comment",
            &pr_number.to_string(),
            "--repo",
            DEFAULT_REPO,
            "--body",
            &comment,
        ],
        token,
    )
    .await
    {
        warn!(pr_number, error = %e, trace_id, "wip_rescue_error");
    }

    log_audit(
        db,
        session_id,
        "wip_rescue_bail_to_human",
        pr_number,
        trace_id,
        &reason,
    )
    .await;
    ChainOutcome::Bailed(reason)
}

/// Write an `audit_events` row for a wip-rescue action (AC8).
async fn log_audit(
    db: &AsyncDatabase,
    session_id: &str,
    event: &str,
    pr_number: u64,
    trace_id: &str,
    detail: &str,
) {
    let resource = format!("{DEFAULT_REPO}#{pr_number}");
    if let Err(e) = db
        .log_audit_event(
            session_id,
            "wip_rescue",
            event,
            Some(&resource),
            Some(detail),
            None,
            Some(trace_id),
        )
        .await
    {
        warn!(pr_number, error = %e, trace_id, "wip_rescue: audit write failed");
    }
}

/// List open draft PRs carrying the `wip-rescue` label.
async fn list_wip_rescue_drafts(token: &str) -> Result<Vec<DraftPr>, String> {
    let out = gh(
        &[
            "pr",
            "list",
            "--repo",
            DEFAULT_REPO,
            "--draft",
            "--state",
            "open",
            "--label",
            WIP_RESCUE_LABEL,
            "--json",
            "number,headRefName,title,labels,createdAt",
            "--limit",
            "100",
        ],
        token,
    )
    .await?;
    let trimmed = out.trim();
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }
    serde_json::from_str(trimmed).map_err(|e| format!("parse gh pr list: {e}"))
}

/// First line of a (possibly multi-line) subprocess error, for compact
/// bail reasons.
fn first_line(s: &str) -> &str {
    s.lines().next().unwrap_or(s)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -- env parsing (three-tier) --

    #[test]
    fn min_age_absent_and_empty_use_default() {
        assert_eq!(parse_min_age(None), MIN_AGE_DEFAULT_SECS);
        assert_eq!(parse_min_age(Some("")), MIN_AGE_DEFAULT_SECS);
        assert_eq!(parse_min_age(Some("   ")), MIN_AGE_DEFAULT_SECS);
    }

    #[test]
    fn min_age_valid_used() {
        assert_eq!(parse_min_age(Some("0")), 0);
        assert_eq!(parse_min_age(Some("1800")), 1800);
        assert_eq!(parse_min_age(Some("  60 ")), 60);
    }

    #[test]
    fn min_age_invalid_and_negative_use_default() {
        assert_eq!(parse_min_age(Some("abc")), MIN_AGE_DEFAULT_SECS);
        assert_eq!(parse_min_age(Some("-5")), MIN_AGE_DEFAULT_SECS);
        assert_eq!(parse_min_age(Some("12.5")), MIN_AGE_DEFAULT_SECS);
    }

    #[test]
    fn max_depth_tiers() {
        assert_eq!(parse_max_depth(None), MAX_DEPTH_DEFAULT);
        assert_eq!(parse_max_depth(Some("")), MAX_DEPTH_DEFAULT);
        assert_eq!(parse_max_depth(Some("3")), 3);
        assert_eq!(parse_max_depth(Some("0")), 0);
        assert_eq!(parse_max_depth(Some("-1")), MAX_DEPTH_DEFAULT);
        assert_eq!(parse_max_depth(Some("nope")), MAX_DEPTH_DEFAULT);
    }

    // -- age filter --

    #[test]
    fn age_secs_basic() {
        let created = "2026-07-29T12:00:00Z";
        let now = "2026-07-29T12:20:00Z";
        assert_eq!(draft_age_secs(created, now), Some(1200));
    }

    #[test]
    fn age_secs_future_created_clamps_to_zero() {
        let created = "2026-07-29T12:20:00Z";
        let now = "2026-07-29T12:00:00Z";
        assert_eq!(draft_age_secs(created, now), Some(0));
    }

    #[test]
    fn age_secs_unparseable_is_none() {
        assert_eq!(draft_age_secs("not-a-date", "2026-07-29T12:00:00Z"), None);
        assert_eq!(draft_age_secs("2026-07-29T12:00:00Z", "garbage"), None);
    }

    // -- rescue depth read / bump --

    #[test]
    fn depth_absent_reads_zero() {
        assert_eq!(read_rescue_depth(None), 0);
        assert_eq!(read_rescue_depth(Some("{}")), 0);
        assert_eq!(
            read_rescue_depth(Some(r#"{"claude_pilot":{"turns":5}}"#)),
            0
        );
        assert_eq!(read_rescue_depth(Some("not json")), 0);
    }

    #[test]
    fn depth_present_reads_value() {
        assert_eq!(read_rescue_depth(Some(r#"{"wip_rescue":{"depth":2}}"#)), 2);
        assert_eq!(
            read_rescue_depth(Some(
                r#"{"claude_pilot":{"turns":5},"wip_rescue":{"depth":1}}"#
            )),
            1
        );
    }

    #[test]
    fn depth_bump_preserves_siblings() {
        let existing = r#"{"claude_pilot":{"turns":5,"pr_url":"u"},"wip_rescue":{"depth":1}}"#;
        let bumped = bump_rescue_depth_metadata(Some(existing), 2);
        let v: serde_json::Value = serde_json::from_str(&bumped).unwrap();
        assert_eq!(v["wip_rescue"]["depth"], 2);
        // sibling object preserved
        assert_eq!(v["claude_pilot"]["turns"], 5);
        assert_eq!(v["claude_pilot"]["pr_url"], "u");
    }

    #[test]
    fn depth_bump_from_empty_and_null() {
        let bumped = bump_rescue_depth_metadata(None, 1);
        let v: serde_json::Value = serde_json::from_str(&bumped).unwrap();
        assert_eq!(v["wip_rescue"]["depth"], 1);

        // Non-object existing metadata is discarded, not merged into.
        let bumped2 = bump_rescue_depth_metadata(Some("[]"), 1);
        let v2: serde_json::Value = serde_json::from_str(&bumped2).unwrap();
        assert_eq!(v2["wip_rescue"]["depth"], 1);
    }

    // -- git version guard (F3) --

    #[test]
    fn git_version_parses_canonical_and_vendor() {
        assert_eq!(parse_git_version("git version 2.39.5"), Some((2, 39, 5)));
        assert_eq!(
            parse_git_version("git version 2.39.3 (Apple Git-146)"),
            Some((2, 39, 3))
        );
        assert_eq!(parse_git_version("git version 2.38.0"), Some((2, 38, 0)));
        assert_eq!(
            parse_git_version("git version 2.45.2-rc0"),
            Some((2, 45, 2))
        );
    }

    #[test]
    fn git_version_missing_is_none() {
        assert_eq!(parse_git_version("nonsense output"), None);
        assert_eq!(parse_git_version(""), None);
    }

    #[test]
    fn writetree_support_boundary() {
        assert!(version_supports_writetree((2, 38, 0)));
        assert!(version_supports_writetree((2, 39, 5)));
        assert!(version_supports_writetree((3, 0, 0)));
        assert!(!version_supports_writetree((2, 37, 9)));
        assert!(!version_supports_writetree((1, 99, 0)));
    }

    // -- perimeter routing --

    #[test]
    fn route_from_classification() {
        assert_eq!(
            UndraftRoute::from(Classification::Mechanical),
            UndraftRoute::Mechanical
        );
        assert_eq!(
            UndraftRoute::from(Classification::DecisionCore),
            UndraftRoute::DecisionCore
        );
    }

    #[test]
    fn classify_pr_files_wiring_matches_route() {
        // Mechanical-only diff → Mechanical route.
        let mech = vec!["README.md".to_string(), "docs/solutions/x.md".to_string()];
        let r: UndraftRoute = perimeter::classify_pr_files(&mech).verdict.into();
        assert_eq!(r, UndraftRoute::Mechanical);

        // Touches the perimeter itself → DECISION-CORE (fail-closed, tainted).
        let core = vec![
            "README.md".to_string(),
            "crates/mika-agent/src/perimeter/rules.rs".to_string(),
        ];
        let r2: UndraftRoute = perimeter::classify_pr_files(&core).verdict.into();
        assert_eq!(r2, UndraftRoute::DecisionCore);

        // Empty diff → DECISION-CORE (fail-closed).
        let empty: Vec<String> = vec![];
        let r3: UndraftRoute = perimeter::classify_pr_files(&empty).verdict.into();
        assert_eq!(r3, UndraftRoute::DecisionCore);
    }

    // -- draft label matching --

    #[test]
    fn draft_label_match_is_case_insensitive() {
        let pr = DraftPr {
            number: 1,
            head_ref: "b".into(),
            title: "t".into(),
            labels: vec![GhLabel {
                name: "WIP-Rescue".into(),
            }],
            created_at: "2026-07-29T12:00:00Z".into(),
        };
        assert!(pr.has_label(WIP_RESCUE_LABEL));
        assert!(!pr.has_label(HUMAN_REVIEW_LABEL));
    }

    #[test]
    fn draft_pr_list_parses() {
        let json = r#"[
            {"number":42,"headRefName":"feat/x","title":"wip(feat): x",
             "labels":[{"name":"wip-rescue"}],"createdAt":"2026-07-29T12:00:00Z"}
        ]"#;
        let drafts: Vec<DraftPr> = serde_json::from_str(json).unwrap();
        assert_eq!(drafts.len(), 1);
        assert_eq!(drafts[0].number, 42);
        assert_eq!(drafts[0].head_ref, "feat/x");
        assert!(drafts[0].has_label("wip-rescue"));
    }

    #[test]
    fn closing_issues_envelope_parses() {
        let json = r#"{"closingIssuesReferences":[{"number":1852},{"number":9}]}"#;
        let env: ClosingIssuesEnvelope = serde_json::from_str(json).unwrap();
        assert_eq!(
            env.closing_issues_references.first().map(|r| r.number),
            Some(1852)
        );

        let empty = r#"{"closingIssuesReferences":[]}"#;
        let env2: ClosingIssuesEnvelope = serde_json::from_str(empty).unwrap();
        assert!(env2.closing_issues_references.is_empty());
    }

    #[test]
    fn first_line_helper() {
        assert_eq!(first_line("a\nb\nc"), "a");
        assert_eq!(first_line("single"), "single");
        assert_eq!(first_line(""), "");
    }
}
