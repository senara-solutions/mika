//! Auto-pull groomed tickets for mika-dev (mika#1363).
//!
//! When mika-dev's dispatch queue is idle, this module selects the
//! highest-priority groomed-not-ready ticket and applies the `ready`
//! label to trigger the webhook-driven dispatch flow.

use anyhow::{Result, anyhow};
use regex::Regex;
use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;
use tracing::{debug, info, warn};

use crate::async_db::AsyncDatabase;

/// Default repo for auto-pull (mika-only for v1).
const DEFAULT_REPO: &str = "senara-solutions/mika";

/// Maximum failure count before a ticket is skipped by the circuit-breaker.
const CIRCUIT_BREAKER_THRESHOLD: i64 = 3;

/// Default age (seconds) a `ready` label must exceed before Phase 2 treats the
/// ticket as stuck and eligible for a remove→add rescue (mika#1824 R3).
const STUCK_READY_THRESHOLD_DEFAULT_SECS: i64 = 900;

/// Env override for the stuck-ready age threshold (mika#1824 R3).
const STUCK_READY_THRESHOLD_ENV: &str = "MIKA_AUTO_PULL_STUCK_READY_THRESHOLD_SECS";

/// Defensive cap on the number of stuck-ready rescues per tick (mika#1824 D3).
/// Overflow is logged and left for the next tick.
const MAX_STUCK_RESCUE_PER_TICK: usize = 5;

/// Pure parse of the stuck-ready threshold from an optional env value. Returns
/// [`STUCK_READY_THRESHOLD_DEFAULT_SECS`] when the value is absent, empty, or
/// unparseable/negative (WARN on invalid). Split out from the env read so it is
/// unit-testable without mutating process environment (mika#1824 step 1).
fn parse_stuck_ready_threshold(raw: Option<&str>) -> i64 {
    match raw {
        Some(v) if !v.trim().is_empty() => match v.trim().parse::<i64>() {
            Ok(n) if n >= 0 => n,
            _ => {
                warn!(
                    value = %v,
                    default = STUCK_READY_THRESHOLD_DEFAULT_SECS,
                    "auto_pull: invalid {STUCK_READY_THRESHOLD_ENV}, using default"
                );
                STUCK_READY_THRESHOLD_DEFAULT_SECS
            }
        },
        _ => STUCK_READY_THRESHOLD_DEFAULT_SECS,
    }
}

/// Read the stuck-ready age threshold in seconds from the environment,
/// falling back to [`STUCK_READY_THRESHOLD_DEFAULT_SECS`] (mika#1824 R3).
fn stuck_ready_threshold_secs() -> i64 {
    parse_stuck_ready_threshold(std::env::var(STUCK_READY_THRESHOLD_ENV).ok().as_deref())
}

// ───────────────────── Issue types ─────────────────────

#[derive(Debug, Clone)]
pub struct IssueLabel {
    pub name: String,
}

#[derive(Debug, Clone)]
pub struct Issue {
    pub number: u64,
    pub body: String,
    pub labels: Vec<IssueLabel>,
    pub updated_at: String,
}

// ───────────────────── Grooming detection (F1) ─────────────────────

/// Structural detection of a groomed issue body.
///
/// Matches the canonical callout block emitted by the grooming pipeline:
/// ```text
/// > - **Branch:** `<branch>`
/// > - **Plan:** `docs/plans/<file>.md` (committed on branch @ <sha>)
/// > - **Grooming history:** <...> → second-pass (GROOMED) — session-id: <uuid>
/// ```
pub fn is_groomed(body: &str) -> bool {
    static GROOMING_HISTORY_RE: OnceLock<Regex> = OnceLock::new();
    let re = GROOMING_HISTORY_RE.get_or_init(|| {
        // Mirrors GROOMED_VERDICT_RE in skills/executor.rs (#1725): accept the
        // canonical strict form AND parameterized/annotated variants like
        // `second-pass (GROOMED, session abc)` or `— session-id: uuid`.
        // The character class after `GROOMED` is the structural discriminator.
        Regex::new(r"(?m)^> - \*\*Grooming history:\*\*.+second-pass \(GROOMED[\s\)\.,;:—-]")
            .expect("grooming history regex must compile")
    });
    re.is_match(body)
        && body.contains("> - **Branch:** `")
        && body.contains("> - **Plan:** `docs/plans/")
}

// ───────────────────── Priority ranking ─────────────────────

/// Returns a numeric rank for priority labels: p0=4, p1=3, p2=2, p3=1, unlabelled=0.
/// Higher rank = higher priority.
pub fn priority_rank(labels: &[IssueLabel]) -> u8 {
    for label in labels {
        match label.name.as_str() {
            "p0" => return 4,
            "p1" => return 3,
            "p2" => return 2,
            "p3" => return 1,
            _ => {}
        }
    }
    0 // unlabelled
}

// ───────────────────── Selection logic ─────────────────────

/// Select the best groomed-not-ready ticket from a list of open issues.
///
/// Filters to groomed-only, excludes issues that already have an open PR
/// closing them (mika#1517 — prevents duplicate pilot dispatch when
/// dispatch-lib's recovery paths leave a DRAFT PR), then ranks by priority
/// (p0 > p1 > p2 > p3 > unlabelled) and by oldest `updated_at` within same
/// priority.
pub fn select_best_candidate(
    issues: Vec<Issue>,
    open_pr_issue_numbers: &HashSet<u64>,
) -> Option<Issue> {
    let candidates: Vec<_> = issues
        .into_iter()
        .filter(|i| !i.labels.iter().any(|l| l.name == "ready"))
        .filter(|i| !open_pr_issue_numbers.contains(&i.number))
        .filter(|i| is_groomed(&i.body))
        .collect();

    if candidates.is_empty() {
        return None;
    }

    candidates.into_iter().max_by(|a, b| {
        let pa = priority_rank(&a.labels);
        let pb = priority_rank(&b.labels);
        pa.cmp(&pb).then_with(|| b.updated_at.cmp(&a.updated_at))
    })
}

/// Pure selection predicate for the Phase 2 stuck-ready reconciler (mika#1824
/// D2, step 7). Given the already-fetched issue list plus the sets/maps the
/// async wrapper resolves from GitHub + DB, return the issue numbers eligible
/// for a remove→add rescue, in ascending order.
///
/// A ticket is selected iff it (a) has the `ready` label, (b) has no open PR
/// closing it, (c) has no in-flight self_dev task, and (d) its `ready` label
/// age (from `ages_by_issue`, in seconds) is `>= threshold_secs`. A missing age
/// entry means the age is unknown/absent → not selected (fail-open on unknown
/// age, per D1).
///
/// The per-issue circuit breaker (D4, step 4 of the D2 ordering) is applied by
/// the async caller *before* ages are fetched, so it does not appear here — a
/// circuit-broken ticket simply never gets an `ages_by_issue` entry.
fn select_stuck_ready_candidates(
    issues: &[Issue],
    open_pr_issue_numbers: &HashSet<u64>,
    in_flight_issue_numbers: &HashSet<u64>,
    ages_by_issue: &HashMap<u64, i64>,
    threshold_secs: i64,
) -> Vec<u64> {
    let mut selected: Vec<u64> = issues
        .iter()
        .filter(|i| i.labels.iter().any(|l| l.name == "ready"))
        .filter(|i| !open_pr_issue_numbers.contains(&i.number))
        .filter(|i| !in_flight_issue_numbers.contains(&i.number))
        .filter(|i| {
            ages_by_issue
                .get(&i.number)
                .is_some_and(|&age| age >= threshold_secs)
        })
        .map(|i| i.number)
        .collect();
    selected.sort_unstable();
    selected
}

// ───────────────────── GitHub CLI helpers ─────────────────────

/// List open issues from the default repo via `gh issue list`.
///
/// Returns all open issues with number, body, labels, and updatedAt.
/// `gh` CLI has no negative label filter, so we fetch all and filter client-side.
async fn gh_list_open_issues(github_token: &str) -> Result<Vec<Issue>> {
    let mut cmd = tokio::process::Command::new("gh");
    cmd.args([
        "issue",
        "list",
        "--repo",
        DEFAULT_REPO,
        "--state",
        "open",
        "--json",
        "number,body,labels,updatedAt",
        "--limit",
        "100",
    ]);
    cmd.env("GH_TOKEN", github_token);
    cmd.stdin(std::process::Stdio::null());
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());
    cmd.kill_on_drop(true);

    let output = cmd.output().await?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow!("gh issue list failed: {}", stderr));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let raw: Vec<serde_json::Value> = serde_json::from_str(&stdout)?;

    let issues = raw
        .into_iter()
        .filter_map(|v| {
            let number = v["number"].as_u64()?;
            let body = v["body"].as_str().unwrap_or_default().to_string();
            let updated_at = v["updatedAt"].as_str().unwrap_or_default().to_string();
            let labels = v["labels"]
                .as_array()
                .map(|arr| {
                    arr.iter()
                        .filter_map(|l| {
                            l["name"].as_str().map(|n| IssueLabel {
                                name: n.to_string(),
                            })
                        })
                        .collect()
                })
                .unwrap_or_default();
            Some(Issue {
                number,
                body,
                labels,
                updated_at,
            })
        })
        .collect();

    Ok(issues)
}

/// List open PR closing-issue references via `gh pr list` (mika#1517).
///
/// Returns the set of issue numbers that have at least one open PR closing
/// them. Powered by GitHub's `closingIssuesReferences` field — populated when
/// a PR body contains `Closes #N` (or equivalent keyword) or the PR is
/// manually linked to an issue. Issues not so linked are absent from the set.
async fn gh_list_open_pr_closing_issues(github_token: &str) -> Result<HashSet<u64>> {
    let mut cmd = tokio::process::Command::new("gh");
    cmd.args([
        "pr",
        "list",
        "--repo",
        DEFAULT_REPO,
        "--state",
        "open",
        "--json",
        "number,closingIssuesReferences",
        "--limit",
        "100",
    ]);
    cmd.env("GH_TOKEN", github_token);
    cmd.stdin(std::process::Stdio::null());
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());
    cmd.kill_on_drop(true);

    let output = cmd.output().await?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow!("gh pr list failed: {}", stderr));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let raw: Vec<serde_json::Value> = serde_json::from_str(&stdout)?;

    let mut closed_issue_numbers = HashSet::new();
    for pr in raw {
        if let Some(refs) = pr["closingIssuesReferences"].as_array() {
            for ref_obj in refs {
                if let Some(n) = ref_obj["number"].as_u64() {
                    closed_issue_numbers.insert(n);
                }
            }
        }
    }
    Ok(closed_issue_numbers)
}

/// Apply a label to a GitHub issue.
async fn gh_apply_label(github_token: &str, issue_number: u64, label: &str) -> Result<()> {
    let mut cmd = tokio::process::Command::new("gh");
    cmd.args([
        "issue",
        "edit",
        &issue_number.to_string(),
        "--repo",
        DEFAULT_REPO,
        "--add-label",
        label,
    ]);
    cmd.env("GH_TOKEN", github_token);
    cmd.stdin(std::process::Stdio::null());
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());
    cmd.kill_on_drop(true);

    let output = cmd.output().await?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow!(
            "gh issue edit --add-label failed for #{}: {}",
            issue_number,
            stderr
        ));
    }
    Ok(())
}

/// Remove a label from a GitHub issue (mika#1824). Mirrors [`gh_apply_label`]
/// with `--remove-label`. `gh issue edit` computes the new label set
/// client-side, so removing an absent label is a no-op that exits 0 — the
/// operation is idempotent. On the off chance a "not found" surfaces, it is
/// tolerated as success.
async fn gh_remove_label(github_token: &str, issue_number: u64, label: &str) -> Result<()> {
    let mut cmd = tokio::process::Command::new("gh");
    cmd.args([
        "issue",
        "edit",
        &issue_number.to_string(),
        "--repo",
        DEFAULT_REPO,
        "--remove-label",
        label,
    ]);
    cmd.env("GH_TOKEN", github_token);
    cmd.stdin(std::process::Stdio::null());
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());
    cmd.kill_on_drop(true);

    let output = cmd.output().await?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        // Tolerate the idempotent "label not present" case as success.
        if stderr.contains("not found") || stderr.contains("Label does not exist") {
            debug!(
                issue = issue_number,
                label, "auto_pull: remove-label no-op (label not present)"
            );
            return Ok(());
        }
        return Err(anyhow!(
            "gh issue edit --remove-label failed for #{}: {}",
            issue_number,
            stderr
        ));
    }
    Ok(())
}

/// Parse an issue-timeline JSON array and return the `created_at` of the LAST
/// `labeled` event whose label name is `ready` (mika#1824 D1). A remove→add
/// cycle appends a fresh `labeled` event, so `last` is the authoritative
/// apply-time. Returns `None` when no such event exists.
fn parse_last_ready_labeled_at(timeline_json: &str) -> Option<String> {
    let events: Vec<serde_json::Value> = serde_json::from_str(timeline_json).ok()?;
    events
        .iter()
        .filter(|e| {
            e["event"].as_str() == Some("labeled") && e["label"]["name"].as_str() == Some("ready")
        })
        .filter_map(|e| e["created_at"].as_str())
        .next_back()
        .map(|s| s.to_string())
}

/// Age in seconds since the `ready` label was last applied to `issue_number`,
/// read from the issue timeline (mika#1824 D1). Returns `Ok(None)` when no
/// `labeled(ready)` event exists (treat as not-stuck / skip). Fail-open: on an
/// API error the caller skips the ticket (does not rescue on unknown age).
///
/// Single page (`per_page=100`) — timeline events beyond the 100th are not
/// inspected; a `ready` re-label is near the tail for any recently-touched
/// ticket, so this bound is safe in practice.
async fn gh_ready_label_age_secs(github_token: &str, issue_number: u64) -> Result<Option<i64>> {
    let mut cmd = tokio::process::Command::new("gh");
    cmd.args([
        "api",
        &format!(
            "repos/{}/issues/{}/timeline?per_page=100",
            DEFAULT_REPO, issue_number
        ),
    ]);
    cmd.env("GH_TOKEN", github_token);
    cmd.stdin(std::process::Stdio::null());
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());
    cmd.kill_on_drop(true);

    let output = cmd.output().await?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow!(
            "gh api timeline failed for #{}: {}",
            issue_number,
            stderr
        ));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let Some(applied_at) = parse_last_ready_labeled_at(&stdout) else {
        return Ok(None);
    };
    let applied = crate::timestamp::parse(&applied_at)?;
    let age = (chrono::Utc::now() - applied).num_seconds();
    Ok(Some(age))
}

// ───────────────────── Orchestration ─────────────────────

/// Run the auto-pull logic for one tick (mika#1363 Phase 1 + mika#1824 Phase 2).
///
/// Fetches the open-issue list and open-PR closing-issue set **once** and shares
/// both across the two phases (D5). Phase 1 promotes a groomed-not-ready ticket
/// to `ready` (queue-gated, unchanged semantics). Phase 2 reconciles tickets
/// that already *have* `ready` but were never dispatched — it runs regardless of
/// queue depth.
///
/// Returns `Some(issue_number)` if Phase 1 promoted a ticket, `None` otherwise.
/// The Phase 2 rescue count is logged but not returned (the dispatcher log at
/// `dispatcher.rs` keys off the Phase 1 promotion).
pub async fn auto_pull_groomed_ticket(
    db: &AsyncDatabase,
    github_token: &str,
    trace_id: &str,
    session_id: &str,
) -> Option<u64> {
    // Fetch open issues once (F4: client-side filter for the `ready` label).
    let issues = match gh_list_open_issues(github_token).await {
        Ok(issues) => issues,
        Err(e) => {
            warn!(error = %e, "auto_pull: failed to list open issues");
            return None;
        }
    };

    // Fetch open-PR closing-issue refs once (mika#1517). Both phases consume it.
    // Fail-open: an empty set preserves pre-fix behavior on infra glitches.
    let open_pr_issue_numbers = match gh_list_open_pr_closing_issues(github_token).await {
        Ok(set) => set,
        Err(e) => {
            warn!(error = %e, "auto_pull: failed to list open PR closing-issue refs; proceeding without filter");
            HashSet::new()
        }
    };

    // Phase 1 — promote a groomed-not-ready ticket (queue-empty gate lives inside).
    let promoted = phase1_promote_groomed(
        db,
        github_token,
        &issues,
        &open_pr_issue_numbers,
        trace_id,
        session_id,
    )
    .await;

    // Phase 2 — reconcile stuck-ready tickets, independent of queue depth (mika#1824).
    let rescued =
        phase2_reconcile_stuck_ready(db, github_token, &issues, &open_pr_issue_numbers).await;
    debug!(
        rescued,
        "auto_pull: phase 2 stuck-ready reconciler complete"
    );

    promoted
}

/// Phase 1 (mika#1363): promote the best groomed-not-ready ticket to `ready`.
///
/// Verbatim of the original `auto_pull_groomed_ticket` body, including its own
/// queue-empty early-return — Phase 1 only fires when mika-dev's dispatch queue
/// is idle. Now receives the shared issue list and open-PR set (D5) instead of
/// fetching them itself.
async fn phase1_promote_groomed(
    db: &AsyncDatabase,
    github_token: &str,
    issues: &[Issue],
    open_pr_issue_numbers: &HashSet<u64>,
    trace_id: &str,
    session_id: &str,
) -> Option<u64> {
    // 1. Queue-empty gate (F2): check if mika-dev has active self_dev tasks.
    let queue_count = match db.count_active_self_dev_tasks().await {
        Ok(count) => count,
        Err(e) => {
            warn!(error = %e, "auto_pull: failed to count active self_dev tasks");
            return None;
        }
    };
    if queue_count > 0 {
        debug!(
            queue_count,
            "auto_pull: queue not empty, skipping (webhook-driven dispatch covering)"
        );
        return None;
    }

    // 3. Select the best groomed-not-ready candidate (skip those with open PRs).
    let candidate = match select_best_candidate(issues.to_vec(), open_pr_issue_numbers) {
        Some(c) => c,
        None => {
            debug!("auto_pull: no groomed-not-ready candidates found");
            return None;
        }
    };

    // 4. Circuit-breaker check (AC3): skip if failure_count >= threshold.
    match db
        .get_auto_pull_failure_count(DEFAULT_REPO, candidate.number)
        .await
    {
        Ok(count) if count >= CIRCUIT_BREAKER_THRESHOLD => {
            info!(
                issue = candidate.number,
                failure_count = count,
                "auto_pull: circuit-breaker skip for #{} ({}× failures)",
                candidate.number,
                count
            );
            // Audit the skip decision
            if let Err(e) = db
                .log_audit_event(
                    session_id,
                    "auto_pull",
                    "auto_pull_skip",
                    None,
                    Some(&format!(
                        "#{} skipped: failure_count={} >= threshold={}",
                        candidate.number, count, CIRCUIT_BREAKER_THRESHOLD
                    )),
                    None,
                    Some(trace_id),
                )
                .await
            {
                warn!(error = %e, "auto_pull: failed to write skip audit event");
            }
            return None;
        }
        Err(e) => {
            warn!(error = %e, "auto_pull: failed to check circuit-breaker, proceeding");
            // Fail-open: proceed with the selection
        }
        _ => {} // count < threshold, proceed
    }

    let rank = priority_rank(&candidate.labels);

    // 5. Apply the `ready` label to trigger webhook-driven dispatch.
    if let Err(e) = gh_apply_label(github_token, candidate.number, "ready").await {
        warn!(
            error = %e,
            issue = candidate.number,
            "auto_pull: failed to apply ready label"
        );
        // AC3: increment circuit-breaker failure counter on label-apply failure.
        if let Err(e) = db
            .increment_auto_pull_failure(DEFAULT_REPO, candidate.number)
            .await
        {
            warn!(error = %e, "auto_pull: failed to increment failure counter");
        }
        return None;
    }

    // AC3: reset circuit-breaker failure counter on successful label application.
    if let Err(e) = db
        .reset_auto_pull_failure(DEFAULT_REPO, candidate.number)
        .await
    {
        warn!(error = %e, "auto_pull: failed to reset failure counter");
    }

    info!(
        issue = candidate.number,
        priority_rank = rank,
        "auto_pull: selected #{} and applied ready label",
        candidate.number
    );

    // 6. Record the auto-pull event for circuit-breaker tracking.
    if let Err(e) = db.record_auto_pull(DEFAULT_REPO, candidate.number).await {
        warn!(error = %e, "auto_pull: failed to record auto-pull event");
    }

    // 7. Audit the selection decision (AC5).
    if let Err(e) = db
        .log_audit_event(
            session_id,
            "auto_pull",
            "auto_pull",
            None,
            Some(&format!(
                "selected #{} (priority_rank={}, updated_at={})",
                candidate.number, rank, candidate.updated_at
            )),
            None,
            Some(trace_id),
        )
        .await
    {
        warn!(error = %e, "auto_pull: failed to write selection audit event");
    }

    Some(candidate.number)
}

/// Phase 2 (mika#1824): reconcile stuck-ready tickets — those that *have* the
/// `ready` label but were never dispatched (webhook dropped, mika-dev busy at
/// fire time, dispatch-gate silent-accept). Runs regardless of queue depth.
///
/// Filter chain follows the D2 cost-bounded ordering (cheapest → most
/// expensive): in-memory ready/open-PR, DB in-flight, DB circuit-breaker, then
/// one GitHub timeline API call per survivor for the label age. Each drop emits
/// a `stuck_ready_reconcile_skipped` DEBUG with its reason. Survivors past the
/// age threshold are remove→add rescued (capped at [`MAX_STUCK_RESCUE_PER_TICK`]),
/// emitting `stuck_ready_reconciled` INFO on success. Returns the rescue count.
async fn phase2_reconcile_stuck_ready(
    db: &AsyncDatabase,
    github_token: &str,
    issues: &[Issue],
    open_pr_issue_numbers: &HashSet<u64>,
) -> usize {
    let threshold = stuck_ready_threshold_secs();

    // Filter 1 (in-mem): keep only tickets WITH the `ready` label (inverse of
    // Phase 1's filter). Everything without `ready` is Phase 1 territory.
    let ready: Vec<&Issue> = issues
        .iter()
        .filter(|i| i.labels.iter().any(|l| l.name == "ready"))
        .collect();
    if ready.is_empty() {
        return 0;
    }

    // Filters 2–4 (in-mem open-PR, DB in-flight, DB circuit-breaker). Build the
    // in-flight set (for the pure predicate) and the survivor list (for the
    // age-fetch step). Ordering is cheapest-first so the API call in step 5
    // fires for as few tickets as possible.
    let mut in_flight_issue_numbers: HashSet<u64> = HashSet::new();
    let mut survivors: Vec<u64> = Vec::new();
    for issue in &ready {
        let n = issue.number;

        // Filter 2: open PR closing this issue (in-memory, already fetched).
        if open_pr_issue_numbers.contains(&n) {
            debug!(
                issue = n,
                reason = "open_pr_closing",
                "stuck_ready_reconcile_skipped"
            );
            continue;
        }

        // Filter 3: in-flight self_dev task for this issue (DB, cheap).
        let issue_url = format!("https://github.com/{}/issues/{}", DEFAULT_REPO, n);
        match db.has_active_self_dev_task_for_issue(&issue_url).await {
            Ok(true) => {
                in_flight_issue_numbers.insert(n);
                debug!(
                    issue = n,
                    reason = "in_flight_self_dev",
                    "stuck_ready_reconcile_skipped"
                );
                continue;
            }
            Ok(false) => {}
            Err(e) => {
                warn!(error = %e, issue = n, "auto_pull: phase 2 in-flight check failed; skipping ticket");
                continue;
            }
        }

        // Filter 4: circuit-breaker (DB, cheap). Fail-open on error.
        match db.get_auto_pull_failure_count(DEFAULT_REPO, n).await {
            Ok(count) if count >= CIRCUIT_BREAKER_THRESHOLD => {
                debug!(
                    issue = n,
                    reason = "circuit_breaker",
                    failure_count = count,
                    "stuck_ready_reconcile_skipped"
                );
                continue;
            }
            Ok(_) => {}
            Err(e) => {
                warn!(error = %e, issue = n, "auto_pull: phase 2 circuit-breaker check failed; proceeding");
            }
        }

        survivors.push(n);
    }

    if survivors.is_empty() {
        return 0;
    }

    // Filter 5 (GitHub API, one call each): read the `ready` label age. This is
    // the expensive step, reached only by the rare survivor set.
    let mut ages_by_issue: HashMap<u64, i64> = HashMap::new();
    for &n in &survivors {
        match gh_ready_label_age_secs(github_token, n).await {
            Ok(Some(age)) => {
                ages_by_issue.insert(n, age);
            }
            Ok(None) => {
                // No `labeled(ready)` event → treat as not-stuck / skip.
                debug!(
                    issue = n,
                    reason = "below_threshold",
                    detail = "no labeled(ready) timeline event",
                    "stuck_ready_reconcile_skipped"
                );
            }
            Err(e) => {
                warn!(error = %e, issue = n, "auto_pull: phase 2 ready-label age read failed; skipping ticket");
            }
        }
    }

    // Pure selection: apply the age threshold + deterministic ordering (D2 step 5).
    let selected = select_stuck_ready_candidates(
        issues,
        open_pr_issue_numbers,
        &in_flight_issue_numbers,
        &ages_by_issue,
        threshold,
    );

    // Emit below_threshold skips for survivors with a known-but-too-young age.
    for &n in &survivors {
        if let Some(&age) = ages_by_issue.get(&n)
            && age < threshold
        {
            debug!(
                issue = n,
                reason = "below_threshold",
                age_secs = age,
                threshold,
                "stuck_ready_reconcile_skipped"
            );
        }
    }

    // Rescue loop: remove→add the `ready` label (Option A — reuse the webhook
    // pipeline). Capped at MAX_STUCK_RESCUE_PER_TICK; overflow waits for the
    // next tick. The remove→add cycle resets the label-age timestamp, so a
    // rescued-but-still-undispatched ticket self-throttles for a full threshold
    // window (D3).
    let mut rescued = 0usize;
    for &n in &selected {
        if rescued >= MAX_STUCK_RESCUE_PER_TICK {
            warn!(
                cap = MAX_STUCK_RESCUE_PER_TICK,
                deferred = selected.len() - rescued,
                "auto_pull: phase 2 rescue cap reached; deferring rest to next tick"
            );
            break;
        }

        if let Err(e) = gh_remove_label(github_token, n, "ready").await {
            warn!(error = %e, issue = n, "auto_pull: phase 2 remove ready label failed");
            if let Err(e2) = db.increment_auto_pull_failure(DEFAULT_REPO, n).await {
                warn!(error = %e2, "auto_pull: failed to increment failure counter");
            }
            continue;
        }
        if let Err(e) = gh_apply_label(github_token, n, "ready").await {
            warn!(error = %e, issue = n, "auto_pull: phase 2 re-add ready label failed");
            if let Err(e2) = db.increment_auto_pull_failure(DEFAULT_REPO, n).await {
                warn!(error = %e2, "auto_pull: failed to increment failure counter");
            }
            continue;
        }

        if let Err(e) = db.record_auto_pull(DEFAULT_REPO, n).await {
            warn!(error = %e, "auto_pull: failed to record auto-pull event");
        }
        if let Err(e) = db.reset_auto_pull_failure(DEFAULT_REPO, n).await {
            warn!(error = %e, "auto_pull: failed to reset failure counter");
        }
        info!(issue = n, "stuck_ready_reconciled");
        rescued += 1;
    }

    rescued
}

// ───────────────────── Tests ─────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── is_groomed tests ──

    #[test]
    fn test_is_groomed_canonical_callout() {
        let body = r#"## Description

Some description of the issue.

> - **Branch:** `feat/123/some-feature`
> - **Plan:** `docs/plans/2026-06-01-001-some-plan.md` (committed on branch @ abc1234)
> - **Grooming history:** first-pass (READY) → second-pass (GROOMED) — session-id: 550e8400-e29b-41d4-a716-446655440000
"#;
        assert!(is_groomed(body));
    }

    #[test]
    fn test_is_groomed_missing_branch() {
        let body = r#"## Description

> - **Plan:** `docs/plans/2026-06-01-001-some-plan.md` (committed on branch @ abc1234)
> - **Grooming history:** first-pass (READY) → second-pass (GROOMED) — session-id: 550e8400
"#;
        assert!(!is_groomed(body));
    }

    #[test]
    fn test_is_groomed_missing_plan() {
        let body = r#"## Description

> - **Branch:** `feat/123/some-feature`
> - **Grooming history:** first-pass (READY) → second-pass (GROOMED) — session-id: 550e8400
"#;
        assert!(!is_groomed(body));
    }

    #[test]
    fn test_is_groomed_missing_grooming_history() {
        let body = r#"## Description

> - **Branch:** `feat/123/some-feature`
> - **Plan:** `docs/plans/2026-06-01-001-some-plan.md` (committed on branch @ abc1234)
"#;
        assert!(!is_groomed(body));
    }

    #[test]
    fn test_is_groomed_only_first_pass() {
        let body = r#"## Description

> - **Branch:** `feat/123/some-feature`
> - **Plan:** `docs/plans/2026-06-01-001-some-plan.md` (committed on branch @ abc1234)
> - **Grooming history:** first-pass (READY) — session-id: 550e8400
"#;
        assert!(!is_groomed(body), "first-pass only should not be groomed");
    }

    #[test]
    fn test_is_groomed_prose_groomed_not_callout() {
        let body = r#"## Description

This ticket has been GROOMED and is ready.

> - **Branch:** `feat/123/some-feature`
> - **Plan:** `docs/plans/2026-06-01-001-some-plan.md` (committed on branch @ abc1234)
"#;
        assert!(
            !is_groomed(body),
            "prose GROOMED without callout should not match"
        );
    }

    #[test]
    fn test_is_groomed_empty_body() {
        assert!(!is_groomed(""));
    }

    // ── #1725 parameterized GROOMED verdict widening ──

    #[test]
    fn test_is_groomed_comma_parameter() {
        // `second-pass (GROOMED, session-id fd4c1a14)` — a form orchestrator-CC
        // produced at mika#1723 dispatch that the strict regex rejected.
        let body = r#"## Description

> - **Branch:** `feat/123/some-feature`
> - **Plan:** `docs/plans/2026-07-04-001-fix-plan.md` (committed on branch @ abc1234)
> - **Grooming history:** first-pass (READY) → second-pass (GROOMED, session fd4c1a14)
"#;
        assert!(is_groomed(body), "comma-parameterized GROOMED must match");
    }

    #[test]
    fn test_is_groomed_em_dash_annotation() {
        // `second-pass (GROOMED — session-id: uuid)` — the shape orchestrator-CC
        // routinely emits with em-dash-separated session-id annotation.
        let body = r#"## Description

> - **Branch:** `feat/123/some-feature`
> - **Plan:** `docs/plans/2026-07-04-001-fix-plan.md` (committed on branch @ abc1234)
> - **Grooming history:** first-pass (READY) → second-pass (GROOMED — session-id: 550e8400)
"#;
        assert!(
            is_groomed(body),
            "em-dash-annotated GROOMED must match; body=\n{body}"
        );
    }

    #[test]
    fn test_is_groomed_period_terminator() {
        let body = r#"## Description

> - **Branch:** `feat/123/some-feature`
> - **Plan:** `docs/plans/2026-07-04-001-fix-plan.md` (committed on branch @ abc1234)
> - **Grooming history:** first-pass (READY) → second-pass (GROOMED. Full ratification.)
"#;
        assert!(is_groomed(body), "period-terminated GROOMED must match");
    }

    #[test]
    fn test_is_groomed_reject_prose_groomedly() {
        // Ensure `GROOMED` followed by a word-continuation char (letter) is
        // rejected — the character class after GROOMED is the discriminator.
        let body = r#"## Description

> - **Branch:** `feat/123/some-feature`
> - **Plan:** `docs/plans/2026-07-04-001-fix-plan.md` (committed on branch @ abc1234)
> - **Grooming history:** first-pass (READY) → second-pass (GROOMEDLY) — not a real form
"#;
        assert!(
            !is_groomed(body),
            "letter-continuation after GROOMED must not match"
        );
    }

    #[test]
    fn test_is_groomed_first_pass_groomed_rejected() {
        // Ensure the `second-pass (` prefix requirement blocks `first-pass (GROOMED)`
        // which is not a valid ratification signal (first-pass is READY/ITERATE/ESCALATE).
        let body = r#"## Description

> - **Branch:** `feat/123/some-feature`
> - **Plan:** `docs/plans/2026-07-04-001-fix-plan.md` (committed on branch @ abc1234)
> - **Grooming history:** first-pass (GROOMED) — no second-pass line
"#;
        assert!(
            !is_groomed(body),
            "first-pass (GROOMED) without second-pass must not match"
        );
    }

    // ── priority_rank tests ──

    #[test]
    fn test_priority_rank_p0() {
        let labels = vec![IssueLabel {
            name: "p0".to_string(),
        }];
        assert_eq!(priority_rank(&labels), 4);
    }

    #[test]
    fn test_priority_rank_p1() {
        let labels = vec![IssueLabel {
            name: "p1".to_string(),
        }];
        assert_eq!(priority_rank(&labels), 3);
    }

    #[test]
    fn test_priority_rank_p2() {
        let labels = vec![IssueLabel {
            name: "p2".to_string(),
        }];
        assert_eq!(priority_rank(&labels), 2);
    }

    #[test]
    fn test_priority_rank_p3() {
        let labels = vec![IssueLabel {
            name: "p3".to_string(),
        }];
        assert_eq!(priority_rank(&labels), 1);
    }

    #[test]
    fn test_priority_rank_unlabelled() {
        let labels = vec![IssueLabel {
            name: "bug".to_string(),
        }];
        assert_eq!(priority_rank(&labels), 0);
    }

    #[test]
    fn test_priority_rank_empty() {
        assert_eq!(priority_rank(&[]), 0);
    }

    #[test]
    fn test_priority_rank_multiple_labels() {
        let labels = vec![
            IssueLabel {
                name: "bug".to_string(),
            },
            IssueLabel {
                name: "p1".to_string(),
            },
            IssueLabel {
                name: "enhancement".to_string(),
            },
        ];
        assert_eq!(priority_rank(&labels), 3);
    }

    // ── select_best_candidate tests ──

    fn make_issue(number: u64, body: &str, label_names: &[&str], updated_at: &str) -> Issue {
        Issue {
            number,
            body: body.to_string(),
            labels: label_names
                .iter()
                .map(|n| IssueLabel {
                    name: n.to_string(),
                })
                .collect(),
            updated_at: updated_at.to_string(),
        }
    }

    const GROOMED_BODY: &str = r#"> - **Branch:** `feat/123/x`
> - **Plan:** `docs/plans/2026-06-01-001-x.md` (committed on branch @ abc1234)
> - **Grooming history:** first-pass (READY) → second-pass (GROOMED) — session-id: 550e8400"#;

    const UNGROOMED_BODY: &str = "Just a regular issue description";

    #[test]
    fn test_select_filters_ungroomed() {
        let issues = vec![
            make_issue(1, UNGROOMED_BODY, &["p0"], "2026-06-01T00:00:00Z"),
            make_issue(2, GROOMED_BODY, &["p1"], "2026-06-01T00:00:00Z"),
        ];
        let selected = select_best_candidate(issues, &HashSet::new()).unwrap();
        assert_eq!(selected.number, 2);
    }

    #[test]
    fn test_select_filters_ready_label() {
        let issues = vec![
            make_issue(1, GROOMED_BODY, &["p0", "ready"], "2026-06-01T00:00:00Z"),
            make_issue(2, GROOMED_BODY, &["p1"], "2026-06-01T00:00:00Z"),
        ];
        let selected = select_best_candidate(issues, &HashSet::new()).unwrap();
        assert_eq!(selected.number, 2);
    }

    #[test]
    fn test_select_highest_priority() {
        let issues = vec![
            make_issue(1, GROOMED_BODY, &["p2"], "2026-06-01T00:00:00Z"),
            make_issue(2, GROOMED_BODY, &["p0"], "2026-06-01T00:00:00Z"),
            make_issue(3, GROOMED_BODY, &["p1"], "2026-06-01T00:00:00Z"),
        ];
        let selected = select_best_candidate(issues, &HashSet::new()).unwrap();
        assert_eq!(selected.number, 2, "p0 should be selected");
    }

    #[test]
    fn test_select_same_priority_oldest_first() {
        let issues = vec![
            make_issue(1, GROOMED_BODY, &["p1"], "2026-06-05T00:00:00Z"),
            make_issue(2, GROOMED_BODY, &["p1"], "2026-06-01T00:00:00Z"),
            make_issue(3, GROOMED_BODY, &["p1"], "2026-06-03T00:00:00Z"),
        ];
        let selected = select_best_candidate(issues, &HashSet::new()).unwrap();
        assert_eq!(
            selected.number, 2,
            "oldest updated_at within same priority should win"
        );
    }

    #[test]
    fn test_select_no_candidates() {
        let issues = vec![make_issue(
            1,
            UNGROOMED_BODY,
            &["p0"],
            "2026-06-01T00:00:00Z",
        )];
        assert!(select_best_candidate(issues, &HashSet::new()).is_none());
    }

    #[test]
    fn test_select_empty_list() {
        assert!(select_best_candidate(vec![], &HashSet::new()).is_none());
    }

    #[test]
    fn test_select_unlabelled_vs_p3() {
        let issues = vec![
            make_issue(1, GROOMED_BODY, &[], "2026-06-01T00:00:00Z"),
            make_issue(2, GROOMED_BODY, &["p3"], "2026-06-01T00:00:00Z"),
        ];
        let selected = select_best_candidate(issues, &HashSet::new()).unwrap();
        assert_eq!(selected.number, 2, "p3 beats unlabelled");
    }

    #[test]
    fn test_select_skips_issues_with_open_pr() {
        // mika#1517: when an issue has an open PR closing it, skip it even if
        // it's groomed and unlabeled — the previous pilot's work is in flight,
        // re-dispatching produces a duplicate.
        let issues = vec![
            make_issue(606, GROOMED_BODY, &["p2"], "2026-06-13T15:18:00Z"),
            make_issue(851, GROOMED_BODY, &["p2"], "2026-06-13T16:00:00Z"),
        ];
        let mut open_pr_set = HashSet::new();
        open_pr_set.insert(606); // mika#606 has an open PR — should be skipped

        let selected = select_best_candidate(issues, &open_pr_set);
        assert!(
            selected.is_some(),
            "non-PR-blocked issue should still be selectable"
        );
        assert_eq!(
            selected.unwrap().number,
            851,
            "issue with open PR (606) should be skipped; 851 wins"
        );
    }

    #[test]
    fn test_select_skips_all_issues_when_all_have_open_prs() {
        // When every candidate has an open PR, the selector returns None
        // (auto-pull will skip this tick — correct behavior).
        let issues = vec![
            make_issue(606, GROOMED_BODY, &["p2"], "2026-06-13T15:18:00Z"),
            make_issue(802, GROOMED_BODY, &["p1"], "2026-06-13T15:00:00Z"),
        ];
        let mut open_pr_set = HashSet::new();
        open_pr_set.insert(606);
        open_pr_set.insert(802);

        let selected = select_best_candidate(issues, &open_pr_set);
        assert!(selected.is_none(), "all candidates have open PRs → None");
    }

    // ── mika#1824 Phase 2: threshold reader tests ──

    #[test]
    fn test_threshold_default_when_absent() {
        assert_eq!(
            parse_stuck_ready_threshold(None),
            STUCK_READY_THRESHOLD_DEFAULT_SECS
        );
    }

    #[test]
    fn test_threshold_default_when_empty() {
        assert_eq!(
            parse_stuck_ready_threshold(Some("   ")),
            STUCK_READY_THRESHOLD_DEFAULT_SECS
        );
    }

    #[test]
    fn test_threshold_valid_override() {
        assert_eq!(parse_stuck_ready_threshold(Some("1800")), 1800);
        // Surrounding whitespace tolerated.
        assert_eq!(parse_stuck_ready_threshold(Some(" 42 ")), 42);
    }

    #[test]
    fn test_threshold_invalid_falls_back() {
        assert_eq!(
            parse_stuck_ready_threshold(Some("not-a-number")),
            STUCK_READY_THRESHOLD_DEFAULT_SECS
        );
        // Negative rejected → default.
        assert_eq!(
            parse_stuck_ready_threshold(Some("-5")),
            STUCK_READY_THRESHOLD_DEFAULT_SECS
        );
    }

    // ── mika#1824 Phase 2: timeline-parse tests ──

    #[test]
    fn test_parse_last_ready_labeled_at_picks_last() {
        // Two labeled(ready) events + noise; `last` (remove→add reset) wins.
        let json = r#"[
            {"event":"labeled","label":{"name":"ready"},"created_at":"2026-07-20T10:00:00Z"},
            {"event":"labeled","label":{"name":"p1"},"created_at":"2026-07-21T10:00:00Z"},
            {"event":"unlabeled","label":{"name":"ready"},"created_at":"2026-07-22T10:00:00Z"},
            {"event":"labeled","label":{"name":"ready"},"created_at":"2026-07-23T10:00:00Z"},
            {"event":"commented","created_at":"2026-07-24T10:00:00Z"}
        ]"#;
        assert_eq!(
            parse_last_ready_labeled_at(json).as_deref(),
            Some("2026-07-23T10:00:00Z")
        );
    }

    #[test]
    fn test_parse_last_ready_labeled_at_none_when_absent() {
        let json = r#"[
            {"event":"labeled","label":{"name":"p1"},"created_at":"2026-07-21T10:00:00Z"},
            {"event":"commented","created_at":"2026-07-24T10:00:00Z"}
        ]"#;
        assert_eq!(parse_last_ready_labeled_at(json), None);
    }

    #[test]
    fn test_parse_last_ready_labeled_at_empty_and_malformed() {
        assert_eq!(parse_last_ready_labeled_at("[]"), None);
        assert_eq!(parse_last_ready_labeled_at("not json"), None);
    }

    // ── mika#1824 Phase 2: AC5 mixed-fixture selection test ──

    fn ready_body_issue(number: u64, extra_labels: &[&str]) -> Issue {
        let mut labels = vec!["ready"];
        labels.extend_from_slice(extra_labels);
        make_issue(number, GROOMED_BODY, &labels, "2026-07-23T00:00:00Z")
    }

    #[test]
    fn test_select_stuck_ready_only_selects_stuck() {
        // Mixed fixture (plan step 8 / AC5):
        //  #10 ready + in-flight self_dev task     → excluded (in_flight)
        //  #20 ready + stuck (age > threshold)      → SELECTED
        //  #30 ready + open PR closing it           → excluded (open_pr)
        //  #40 ready + fresh (age < threshold)      → excluded (below_threshold)
        //  #50 not-ready + groomed                  → excluded (no ready label)
        let issues = vec![
            ready_body_issue(10, &["p1"]),
            ready_body_issue(20, &["p1"]),
            ready_body_issue(30, &["p1"]),
            ready_body_issue(40, &["p1"]),
            make_issue(50, GROOMED_BODY, &["p1"], "2026-07-23T00:00:00Z"),
        ];

        let mut open_pr = HashSet::new();
        open_pr.insert(30u64);

        let mut in_flight = HashSet::new();
        in_flight.insert(10u64);

        let threshold = 900;
        let mut ages = HashMap::new();
        ages.insert(20u64, 1200); // stuck: above threshold
        ages.insert(40u64, 300); // fresh: below threshold
        // #10/#30 never reach the age fetch; #50 has no ready label.

        let selected =
            select_stuck_ready_candidates(&issues, &open_pr, &in_flight, &ages, threshold);
        assert_eq!(
            selected,
            vec![20],
            "only ready+stuck+no-in-flight+no-PR wins"
        );
    }

    #[test]
    fn test_select_stuck_ready_boundary_and_missing_age() {
        // Age exactly at threshold is stuck (>=). Missing age → not selected.
        let issues = vec![ready_body_issue(1, &[]), ready_body_issue(2, &[])];
        let open_pr = HashSet::new();
        let in_flight = HashSet::new();
        let mut ages = HashMap::new();
        ages.insert(1u64, 900); // exactly threshold → selected
        // #2 has no age entry → excluded

        let selected = select_stuck_ready_candidates(&issues, &open_pr, &in_flight, &ages, 900);
        assert_eq!(selected, vec![1]);
    }

    #[test]
    fn test_select_stuck_ready_ascending_order() {
        let issues = vec![
            ready_body_issue(30, &[]),
            ready_body_issue(10, &[]),
            ready_body_issue(20, &[]),
        ];
        let open_pr = HashSet::new();
        let in_flight = HashSet::new();
        let mut ages = HashMap::new();
        for n in [30u64, 10, 20] {
            ages.insert(n, 1000);
        }
        let selected = select_stuck_ready_candidates(&issues, &open_pr, &in_flight, &ages, 900);
        assert_eq!(selected, vec![10, 20, 30], "selected numbers ascending");
    }
}
