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

// ───────────────────── Phase 0 auto-feeder consts (mika#1863) ─────────────────────

/// Env override for the auto-feeder ready-pool target (mika#1863 R2/AC1).
const AUTO_FEEDER_MIN_READY_ENV: &str = "MIKA_AUTO_FEEDER_MIN_READY";

/// Default pullable-ready pool target (mika#1863 R2). The feeder tops the pool
/// up to this count each tick.
const AUTO_FEEDER_MIN_READY_DEFAULT: u32 = 3;

/// Lower clamp for the pool target (mika#1863 R2). The literal `0` bypasses the
/// clamp as a disable sentinel — see [`parse_min_ready`].
const AUTO_FEEDER_MIN_READY_MIN: u32 = 1;

/// Upper clamp for the pool target (mika#1863 R2).
const AUTO_FEEDER_MIN_READY_MAX: u32 = 10;

/// Working-set cap on the groomed-not-ready backlog the feeder ranks per tick
/// (mika#1863 R4/D6/F4). A pagination + grooming-signal bound, not an arbitrary
/// limit: a dispatchable backlog exceeding this is itself a grooming-throughput
/// signal (surfaced via `auto_feeder_no_backlog` and AC9 pool-sampling) rather
/// than a feeder-visibility problem. Single-line raise if real operation ever
/// shows a legitimate >50 dispatchable backlog.
const FEEDER_WORKING_SET_CAP: usize = 50;

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

/// Pure parse of the auto-feeder pool target from an optional env value
/// (mika#1863 R2). Split out from the env read so it is unit-testable without
/// mutating process environment, mirroring [`parse_stuck_ready_threshold`].
///
/// - Missing/empty/unparseable → [`AUTO_FEEDER_MIN_READY_DEFAULT`] (WARN on invalid).
/// - Literal `0` → `0` (disable sentinel; preserved through the clamp so Phase 0
///   returns early — the feeder is turned off).
/// - `1..=10` → as-is.
/// - `>10` → clamped to [`AUTO_FEEDER_MIN_READY_MAX`].
fn parse_min_ready(raw: Option<&str>) -> u32 {
    match raw {
        Some(v) if !v.trim().is_empty() => match v.trim().parse::<u32>() {
            Ok(0) => 0, // disable sentinel — bypasses the [MIN, MAX] clamp
            Ok(n) => n.clamp(AUTO_FEEDER_MIN_READY_MIN, AUTO_FEEDER_MIN_READY_MAX),
            Err(_) => {
                warn!(
                    value = %v,
                    default = AUTO_FEEDER_MIN_READY_DEFAULT,
                    "auto_feeder: invalid {AUTO_FEEDER_MIN_READY_ENV}, using default"
                );
                AUTO_FEEDER_MIN_READY_DEFAULT
            }
        },
        _ => AUTO_FEEDER_MIN_READY_DEFAULT,
    }
}

/// Read the auto-feeder pool target from the environment, falling back to
/// [`AUTO_FEEDER_MIN_READY_DEFAULT`] (mika#1863 R2).
fn auto_feeder_min_ready() -> u32 {
    parse_min_ready(std::env::var(AUTO_FEEDER_MIN_READY_ENV).ok().as_deref())
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

/// Feeder priority rank keyed on the **real** `.github/labels.yml` taxonomy
/// (mika#1863 R6): `p0-critical`=5, `p1-important`=4, `agent-core`=3,
/// `p2-normal`=2, `p3-nice-to-have`=1, none=0. The rank is the **max** per-label
/// rank, so a ticket carrying both `p1-important` and `agent-core` (e.g. #1863)
/// ranks 4.
///
/// Corrects the latent `priority_rank` bug (matches bare `"p1"`, returns 0 for
/// every real issue — the taxonomy uses suffixed names). Uses the real
/// `.github/labels.yml` taxonomy. Unifying the two into one ranker requires
/// verifying agent-core label handling across the whole auto_pull surface — out
/// of scope here.
fn feeder_rank(labels: &[IssueLabel]) -> u8 {
    labels
        .iter()
        .map(|l| match l.name.as_str() {
            "p0-critical" => 5,
            "p1-important" => 4,
            "agent-core" => 3,
            "p2-normal" => 2,
            "p3-nice-to-have" => 1,
            _ => 0,
        })
        .max()
        .unwrap_or(0)
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

// ───────────────────── Phase 0 feeder selection (mika#1863) ─────────────────────

/// Returns `true` if `issue` carries a label that structurally excludes it from
/// the pullable pool AND the feeder backlog (mika#1863 R3/R4): `blocked` or
/// `operator-review`. Both mean "not dispatchable regardless of grooming state".
fn is_feeder_excluded(issue: &Issue) -> bool {
    issue
        .labels
        .iter()
        .any(|l| l.name == "blocked" || l.name == "operator-review")
}

/// Count the **pullable**-ready tickets — the threshold signal for the feeder
/// (mika#1863 R3/D2), NOT the raw `ready` count. A ticket counts toward the pool
/// iff it (a) has `ready`, (b) has no open PR closing it, (c) has no in-flight
/// self_dev task, and (d) is not labelled `blocked`/`operator-review`.
///
/// This is the founding-incident correctness fix: the 2026-07-27→28 11 h idle
/// had raw-count ≥ 1 (#1682 open-PR + #1646 in-flight) while pullable-count was
/// 0. Pure/in-memory — no GitHub or DB calls.
fn count_pullable_ready(
    issues: &[Issue],
    open_pr_issue_numbers: &HashSet<u64>,
    in_flight_issue_numbers: &HashSet<u64>,
) -> usize {
    issues
        .iter()
        .filter(|i| i.labels.iter().any(|l| l.name == "ready"))
        .filter(|i| !open_pr_issue_numbers.contains(&i.number))
        .filter(|i| !in_flight_issue_numbers.contains(&i.number))
        .filter(|i| !is_feeder_excluded(i))
        .count()
}

/// Select the groomed-not-ready tickets to promote, highest [`feeder_rank`]
/// first (oldest-`updated_at` tiebreak), capped at `slots` (mika#1863 R4/R5/D4).
///
/// Candidate filter chain: `!ready` → [`is_groomed`] (full canonical callout,
/// not the loose `Plan:` substring — see D3) → `!open_pr` → `!in_flight` →
/// `!blocked`/`!operator-review`. The surviving set is sorted by rank DESC then
/// `updated_at` ASC and truncated to `min(slots, FEEDER_WORKING_SET_CAP)` — the
/// working-set cap (D6/F4) bounds the promoted count independent of `slots`.
///
/// Pure/in-memory — no GitHub or DB calls. The async wrapper resolves the
/// `in_flight` set and wires real `gh`/DB, same split as the Phase 2 predicate.
fn select_feeder_candidates(
    issues: &[Issue],
    open_pr_issue_numbers: &HashSet<u64>,
    in_flight_issue_numbers: &HashSet<u64>,
    slots: usize,
) -> Vec<u64> {
    let mut candidates: Vec<&Issue> = issues
        .iter()
        .filter(|i| !i.labels.iter().any(|l| l.name == "ready"))
        .filter(|i| !open_pr_issue_numbers.contains(&i.number))
        .filter(|i| !in_flight_issue_numbers.contains(&i.number))
        .filter(|i| !is_feeder_excluded(i))
        .filter(|i| is_groomed(&i.body))
        .collect();

    // Rank DESC, then oldest `updated_at` first within a rank tier.
    candidates.sort_by(|a, b| {
        let ra = feeder_rank(&a.labels);
        let rb = feeder_rank(&b.labels);
        rb.cmp(&ra).then_with(|| a.updated_at.cmp(&b.updated_at))
    });

    candidates
        .into_iter()
        .take(slots.min(FEEDER_WORKING_SET_CAP))
        .map(|i| i.number)
        .collect()
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

    // Phase 0 — feeder: top the pullable-ready pool up to MIN_READY (mika#1863).
    // Runs BEFORE Phase 1 in the same tick, sharing the two `gh` fetches above,
    // so AC2's "feeder promotes → puller picks up same tick" is structural, not
    // scheduling-luck (D1).
    let fed = phase0_feed_ready_pool(
        db,
        github_token,
        &issues,
        &open_pr_issue_numbers,
        trace_id,
        session_id,
    )
    .await;
    debug!(fed, "auto_pull: phase 0 feeder complete");

    // Phase 1 — promote a groomed-not-ready ticket (queue-empty gate lives inside).
    //
    // N+1 over-promotion bound (F3, benign): Phase 0 tops the pool to MIN_READY
    // (N). Phase 1's filter is `!ready`, so when the queue is idle it may promote
    // ONE additional groomed-not-ready ticket the feeder just skipped → at most
    // N+1 ready tickets per tick. Intentional and harmless — the webhook dispatch
    // drains the pool, and the two phases have complementary intents (feeder =
    // hold a buffer; Phase 1 = kick a dispatch when idle). No coupling change.
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

/// Phase 0 (mika#1863): auto-feeder — keep the **pullable**-ready pool topped up
/// to `MIN_READY` from the groomed-dispatchable backlog. Runs before Phase 1 on
/// every tick, independent of queue depth.
///
/// Reuses the shared `issues` + `open_pr` fetches (D1), `is_groomed`,
/// `gh_apply_label`, the per-issue circuit breaker, and
/// `has_active_self_dev_task_for_issue` verbatim. Returns the number of tickets
/// promoted this tick.
///
/// Emits three `auto_feeder` audit events (R7/AC6): `auto_feeder_skip` when the
/// pool already meets the threshold, `auto_feeder_no_backlog` when the pool is
/// under threshold but no dispatchable backlog exists (true starvation signal),
/// and `auto_feeder_promoted` per successful apply.
async fn phase0_feed_ready_pool(
    db: &AsyncDatabase,
    github_token: &str,
    issues: &[Issue],
    open_pr_issue_numbers: &HashSet<u64>,
    trace_id: &str,
    session_id: &str,
) -> usize {
    // R2: read the pool target; `0` disables the feeder entirely.
    let min_ready = auto_feeder_min_ready();
    if min_ready == 0 {
        debug!("auto_feeder: disabled (MIN_READY=0), skipping");
        return 0;
    }

    // Build the in-flight set (D4): probe `has_active_self_dev_task_for_issue`
    // over the union of ready tickets (for the pullable count) and pre-in-flight
    // groomed-not-ready candidates (for selection). Bounded by FEEDER_WORKING_SET_CAP
    // on the candidate side; the ready side is bounded by the small pool. A probe
    // error is treated conservatively as in-flight (excluded from both pullable
    // and promotion) — fail-safe against re-promoting an already-dispatching ticket.
    let mut probe_targets: Vec<u64> = Vec::new();
    let mut candidate_probes = 0usize;
    for issue in issues {
        let has_ready = issue.labels.iter().any(|l| l.name == "ready");
        if has_ready {
            probe_targets.push(issue.number);
            continue;
        }
        // Pre-in-flight candidate shape: groomed, not open-PR, not excluded.
        if candidate_probes < FEEDER_WORKING_SET_CAP
            && !open_pr_issue_numbers.contains(&issue.number)
            && !is_feeder_excluded(issue)
            && is_groomed(&issue.body)
        {
            probe_targets.push(issue.number);
            candidate_probes += 1;
        }
    }

    let mut in_flight_issue_numbers: HashSet<u64> = HashSet::new();
    for n in probe_targets {
        let issue_url = format!("https://github.com/{}/issues/{}", DEFAULT_REPO, n);
        match db.has_active_self_dev_task_for_issue(&issue_url).await {
            Ok(true) => {
                in_flight_issue_numbers.insert(n);
            }
            Ok(false) => {}
            Err(e) => {
                warn!(error = %e, issue = n, "auto_feeder: in-flight probe failed; treating as in-flight");
                in_flight_issue_numbers.insert(n);
            }
        }
    }

    // R3/D2: pullable-ready count is the threshold signal, not raw `ready` count.
    let pullable = count_pullable_ready(issues, open_pr_issue_numbers, &in_flight_issue_numbers);
    if pullable >= min_ready as usize {
        debug!(
            pullable,
            min_ready, "auto_feeder: pool already at/above threshold, skipping"
        );
        if let Err(e) = db
            .log_audit_event(
                session_id,
                "auto_feeder",
                "auto_feeder_skip",
                None,
                Some(&format!(
                    "pool at threshold: pullable={pullable} >= min_ready={min_ready}"
                )),
                None,
                Some(trace_id),
            )
            .await
        {
            warn!(error = %e, "auto_feeder: failed to write skip audit event");
        }
        return 0;
    }

    // R5: promote up to `min_ready − pullable` top candidates.
    let slots = min_ready as usize - pullable;
    let candidates = select_feeder_candidates(
        issues,
        open_pr_issue_numbers,
        &in_flight_issue_numbers,
        slots,
    );

    if candidates.is_empty() {
        // R7: pool is under threshold but no dispatchable backlog — the grooming
        // pipeline (not the feeder) is the bottleneck. Surface it explicitly.
        info!(pullable, min_ready, "auto_feeder_no_backlog");
        if let Err(e) = db
            .log_audit_event(
                session_id,
                "auto_feeder",
                "auto_feeder_no_backlog",
                None,
                Some(&format!(
                    "under threshold but no dispatchable backlog: pullable={pullable}, min_ready={min_ready}"
                )),
                None,
                Some(trace_id),
            )
            .await
        {
            warn!(error = %e, "auto_feeder: failed to write no-backlog audit event");
        }
        return 0;
    }

    // R5/D5: promote each candidate, guarded by the per-issue circuit breaker
    // (identical to Phase 1). A failing apply increments the breaker and moves on.
    let mut promoted = 0usize;
    for n in candidates {
        // Circuit-breaker check: skip a persistently-failing issue.
        match db.get_auto_pull_failure_count(DEFAULT_REPO, n).await {
            Ok(count) if count >= CIRCUIT_BREAKER_THRESHOLD => {
                info!(
                    issue = n,
                    failure_count = count,
                    "auto_feeder: circuit-breaker skip for #{n} ({count}× failures)"
                );
                continue;
            }
            Err(e) => {
                warn!(error = %e, issue = n, "auto_feeder: failed to check circuit-breaker, proceeding");
                // Fail-open: proceed with the promotion.
            }
            _ => {} // count < threshold, proceed
        }

        let rank = issues
            .iter()
            .find(|i| i.number == n)
            .map(|i| feeder_rank(&i.labels))
            .unwrap_or(0);

        if let Err(e) = gh_apply_label(github_token, n, "ready").await {
            warn!(error = %e, issue = n, "auto_feeder: failed to apply ready label");
            if let Err(e2) = db.increment_auto_pull_failure(DEFAULT_REPO, n).await {
                warn!(error = %e2, "auto_feeder: failed to increment failure counter");
            }
            continue;
        }

        if let Err(e) = db.record_auto_pull(DEFAULT_REPO, n).await {
            warn!(error = %e, "auto_feeder: failed to record auto-pull event");
        }
        if let Err(e) = db.reset_auto_pull_failure(DEFAULT_REPO, n).await {
            warn!(error = %e, "auto_feeder: failed to reset failure counter");
        }

        info!(issue = n, feeder_rank = rank, "auto_feeder_promoted");
        if let Err(e) = db
            .log_audit_event(
                session_id,
                "auto_feeder",
                "auto_feeder_promoted",
                None,
                Some(&format!(
                    "promoted #{n} (feeder_rank={rank}, reason=pullable={pullable}<min_ready={min_ready})"
                )),
                None,
                Some(trace_id),
            )
            .await
        {
            warn!(error = %e, "auto_feeder: failed to write promotion audit event");
        }
        promoted += 1;
    }

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

    // ── mika#1863 Phase 0: parse_min_ready tests (R2) ──

    #[test]
    fn test_min_ready_default_when_absent() {
        assert_eq!(parse_min_ready(None), AUTO_FEEDER_MIN_READY_DEFAULT);
        assert_eq!(parse_min_ready(Some("   ")), AUTO_FEEDER_MIN_READY_DEFAULT);
    }

    #[test]
    fn test_min_ready_zero_disables() {
        // Literal 0 is the disable sentinel — must NOT be clamped up to MIN.
        assert_eq!(parse_min_ready(Some("0")), 0);
        assert_eq!(parse_min_ready(Some(" 0 ")), 0);
    }

    #[test]
    fn test_min_ready_clamp_min() {
        // Below MIN but non-zero → clamped up to MIN (1). Only literal 0 disables.
        assert_eq!(parse_min_ready(Some("1")), AUTO_FEEDER_MIN_READY_MIN);
    }

    #[test]
    fn test_min_ready_clamp_max() {
        assert_eq!(parse_min_ready(Some("11")), AUTO_FEEDER_MIN_READY_MAX);
        assert_eq!(parse_min_ready(Some("9999")), AUTO_FEEDER_MIN_READY_MAX);
    }

    #[test]
    fn test_min_ready_invalid_falls_back() {
        assert_eq!(
            parse_min_ready(Some("not-a-number")),
            AUTO_FEEDER_MIN_READY_DEFAULT
        );
        // Negative is unparseable as u32 → default.
        assert_eq!(parse_min_ready(Some("-5")), AUTO_FEEDER_MIN_READY_DEFAULT);
    }

    #[test]
    fn test_min_ready_valid_passthrough() {
        assert_eq!(parse_min_ready(Some("5")), 5);
        assert_eq!(parse_min_ready(Some(" 7 ")), 7);
        assert_eq!(parse_min_ready(Some("10")), 10);
    }

    // ── mika#1863 Phase 0: feeder_rank tests (R6) ──

    #[test]
    fn test_feeder_rank_p1_important() {
        let labels = vec![IssueLabel {
            name: "p1-important".to_string(),
        }];
        assert_eq!(feeder_rank(&labels), 4);
    }

    #[test]
    fn test_feeder_rank_agent_core() {
        let labels = vec![IssueLabel {
            name: "agent-core".to_string(),
        }];
        assert_eq!(feeder_rank(&labels), 3);
    }

    #[test]
    fn test_feeder_rank_1863_shape_max_of_two() {
        // #1863 carries both p1-important and agent-core → max rank = 4.
        let labels = vec![
            IssueLabel {
                name: "agent-core".to_string(),
            },
            IssueLabel {
                name: "p1-important".to_string(),
            },
        ];
        assert_eq!(feeder_rank(&labels), 4, "max per-label rank wins");
    }

    #[test]
    fn test_feeder_rank_p0_and_lower_tiers() {
        assert_eq!(
            feeder_rank(&[IssueLabel {
                name: "p0-critical".to_string()
            }]),
            5
        );
        assert_eq!(
            feeder_rank(&[IssueLabel {
                name: "p2-normal".to_string()
            }]),
            2
        );
        assert_eq!(
            feeder_rank(&[IssueLabel {
                name: "p3-nice-to-have".to_string()
            }]),
            1
        );
    }

    #[test]
    fn test_feeder_rank_none() {
        assert_eq!(feeder_rank(&[]), 0);
        assert_eq!(
            feeder_rank(&[IssueLabel {
                name: "bug".to_string()
            }]),
            0
        );
    }

    // ── mika#1863 Phase 0: count_pullable_ready tests (R3/D2) ──

    #[test]
    fn test_count_pullable_ready_excludes_all_skip_predicates() {
        let issues = vec![
            // #1 plain ready → counts
            make_issue(1, GROOMED_BODY, &["ready", "p1-important"], "t"),
            // #2 ready + open PR → excluded
            make_issue(2, GROOMED_BODY, &["ready"], "t"),
            // #3 ready + in-flight → excluded
            make_issue(3, GROOMED_BODY, &["ready"], "t"),
            // #4 ready + blocked → excluded
            make_issue(4, GROOMED_BODY, &["ready", "blocked"], "t"),
            // #5 ready + operator-review → excluded
            make_issue(5, GROOMED_BODY, &["ready", "operator-review"], "t"),
            // #6 not ready → excluded (not a pool member)
            make_issue(6, GROOMED_BODY, &["p1-important"], "t"),
        ];
        let mut open_pr = HashSet::new();
        open_pr.insert(2u64);
        let mut in_flight = HashSet::new();
        in_flight.insert(3u64);

        assert_eq!(
            count_pullable_ready(&issues, &open_pr, &in_flight),
            1,
            "only #1 is genuinely pullable"
        );
    }

    #[test]
    fn test_count_pullable_ready_counts_multiple_plain_ready() {
        let issues = vec![
            make_issue(1, GROOMED_BODY, &["ready"], "t"),
            make_issue(2, GROOMED_BODY, &["ready"], "t"),
            make_issue(3, GROOMED_BODY, &["ready"], "t"),
        ];
        assert_eq!(
            count_pullable_ready(&issues, &HashSet::new(), &HashSet::new()),
            3
        );
    }

    // ── mika#1863 Phase 0: select_feeder_candidates tests (R4/R5) ──

    #[test]
    fn test_select_feeder_filters_all_skip_predicates() {
        let issues = vec![
            // #1 ready → excluded (already in pool)
            make_issue(1, GROOMED_BODY, &["ready", "p1-important"], "t"),
            // #2 ungroomed → excluded
            make_issue(2, UNGROOMED_BODY, &["p1-important"], "t"),
            // #3 groomed + open PR → excluded
            make_issue(3, GROOMED_BODY, &["p1-important"], "t"),
            // #4 groomed + in-flight → excluded
            make_issue(4, GROOMED_BODY, &["p1-important"], "t"),
            // #5 groomed + blocked → excluded
            make_issue(5, GROOMED_BODY, &["p1-important", "blocked"], "t"),
            // #6 groomed + operator-review → excluded
            make_issue(6, GROOMED_BODY, &["p1-important", "operator-review"], "t"),
            // #7 clean groomed candidate → SELECTED
            make_issue(7, GROOMED_BODY, &["p1-important"], "t"),
        ];
        let mut open_pr = HashSet::new();
        open_pr.insert(3u64);
        let mut in_flight = HashSet::new();
        in_flight.insert(4u64);

        let selected = select_feeder_candidates(&issues, &open_pr, &in_flight, 10);
        assert_eq!(selected, vec![7], "only the clean groomed candidate wins");
    }

    #[test]
    fn test_select_feeder_rank_then_oldest_first_ordering() {
        let issues = vec![
            // Highest rank (p0-critical), newer.
            make_issue(1, GROOMED_BODY, &["p0-critical"], "2026-07-05T00:00:00Z"),
            // Same rank tier p1-important, older → should precede #3.
            make_issue(2, GROOMED_BODY, &["p1-important"], "2026-07-01T00:00:00Z"),
            // Same rank tier p1-important, newer.
            make_issue(3, GROOMED_BODY, &["p1-important"], "2026-07-03T00:00:00Z"),
        ];
        let selected = select_feeder_candidates(&issues, &HashSet::new(), &HashSet::new(), 10);
        assert_eq!(
            selected,
            vec![1, 2, 3],
            "rank DESC first (#1), then oldest-first within tier (#2 before #3)"
        );
    }

    #[test]
    fn test_select_feeder_respects_slots_cap() {
        let issues = vec![
            make_issue(1, GROOMED_BODY, &["p1-important"], "2026-07-01T00:00:00Z"),
            make_issue(2, GROOMED_BODY, &["p1-important"], "2026-07-02T00:00:00Z"),
            make_issue(3, GROOMED_BODY, &["p1-important"], "2026-07-03T00:00:00Z"),
        ];
        let selected = select_feeder_candidates(&issues, &HashSet::new(), &HashSet::new(), 2);
        assert_eq!(selected, vec![1, 2], "slots=2 caps the promotion count");
    }

    #[test]
    fn test_select_feeder_respects_working_set_cap() {
        // 60 clean groomed candidates + an oversized slots value → the output is
        // bounded by FEEDER_WORKING_SET_CAP (50), not slots.
        let issues: Vec<Issue> = (1..=60)
            .map(|n| {
                make_issue(
                    n,
                    GROOMED_BODY,
                    &["p1-important"],
                    &format!("2026-07-{:02}T00:00:00Z", (n % 28) + 1),
                )
            })
            .collect();
        let selected = select_feeder_candidates(&issues, &HashSet::new(), &HashSet::new(), 100);
        assert_eq!(
            selected.len(),
            FEEDER_WORKING_SET_CAP,
            "working-set cap bounds the count when slots exceeds it"
        );
    }

    #[test]
    fn test_select_feeder_end_to_end_at_predicate_level() {
        // AC8 end-to-end at predicate level: 0 pullable + 5 groomed backlog +
        // min_ready=3 → slots=3 → returns exactly the top 3 by rank.
        let issues = vec![
            make_issue(
                1,
                GROOMED_BODY,
                &["p3-nice-to-have"],
                "2026-07-01T00:00:00Z",
            ),
            make_issue(2, GROOMED_BODY, &["p0-critical"], "2026-07-01T00:00:00Z"),
            make_issue(3, GROOMED_BODY, &["p2-normal"], "2026-07-01T00:00:00Z"),
            make_issue(4, GROOMED_BODY, &["p1-important"], "2026-07-01T00:00:00Z"),
            make_issue(5, GROOMED_BODY, &["agent-core"], "2026-07-01T00:00:00Z"),
        ];
        // 0 pullable (no ready-labelled tickets), min_ready=3 → slots=3.
        assert_eq!(
            count_pullable_ready(&issues, &HashSet::new(), &HashSet::new()),
            0
        );
        let slots = 3;
        let selected = select_feeder_candidates(&issues, &HashSet::new(), &HashSet::new(), slots);
        assert_eq!(
            selected,
            vec![2, 4, 5],
            "top 3 by rank: p0-critical(#2) > p1-important(#4) > agent-core(#5)"
        );
    }
}
