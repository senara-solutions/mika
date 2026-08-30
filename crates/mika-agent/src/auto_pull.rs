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

// ───────────────────── Re-drive budget consts (mika#2020) ─────────────────────

/// Default per-ticket re-drive budget before Phase 2 gives up and says so
/// (mika#2020). Three, for parity with [`CIRCUIT_BREAKER_THRESHOLD`] and because
/// at the 900 s age threshold three re-drives span at least ~45 min — well past
/// a dropped webhook or a mika-dev that was busy at fire time.
const MAX_REDRIVES_DEFAULT: i64 = 3;

/// Env override for the re-drive budget (mika#2020). The literal `0` disables
/// the budget (unbounded re-drives — the pre-fix behaviour), mirroring the
/// disable sentinel of [`AUTO_FEEDER_MIN_READY_ENV`].
const MAX_REDRIVES_ENV: &str = "MIKA_AUTO_PULL_MAX_REDRIVES";

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

/// Pure parse of the re-drive budget from an optional env value (mika#2020 R14).
/// Same three-tier contract as [`parse_stuck_ready_threshold`]: absent/empty →
/// default; unparseable/negative → default with a WARN; `0` → unbounded.
fn parse_max_redrives(raw: Option<&str>) -> i64 {
    match raw {
        Some(v) if !v.trim().is_empty() => match v.trim().parse::<i64>() {
            Ok(n) if n >= 0 => n,
            _ => {
                warn!(
                    value = %v,
                    default = MAX_REDRIVES_DEFAULT,
                    "auto_pull: invalid {MAX_REDRIVES_ENV}, using default"
                );
                MAX_REDRIVES_DEFAULT
            }
        },
        _ => MAX_REDRIVES_DEFAULT,
    }
}

/// Read the per-ticket re-drive budget from the environment (mika#2020 R14).
/// `0` means unbounded.
fn max_redrives() -> i64 {
    parse_max_redrives(std::env::var(MAX_REDRIVES_ENV).ok().as_deref())
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

// ───────────────────── Plan ownership (mika#2020) ─────────────────────

/// Verdict on whether the plan a ticket's callout points at belongs to that
/// ticket (mika#2020).
///
/// [`is_groomed`] answers a different question — whether the callout has the
/// canonical *shape*. It never checks who the plan belongs to, which is how
/// mika#1887 came to carry a callout pointing at
/// `docs/plans/2026-08-21-002-fix-1933-reader-completed-section-avancement-plan.md`:
/// a real file, a real plan, another ticket's intent. The pilot opened it, read
/// it, and had no way to know it was implementing #1933.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanOwnership {
    /// The plan filename's canonical issue slot carries this ticket's number.
    Owned,
    /// The slot carries a *different* issue number — positive evidence of
    /// misattribution. The only verdict that refuses.
    OwnedByOther(u64),
    /// No callout, or a filename with no canonical issue slot (historical plan
    /// names like `918-eval-kg-fixtures-…md`). We cannot tell, so we do not
    /// accuse — dev-groom's `_find_issue_plan` has a content fallback on an
    /// `**Issue:**` header that auto-pull cannot evaluate without per-ticket
    /// I/O.
    Unattributable,
}

/// Extract the plan path from a ticket's grooming callout, if it has one.
fn extract_plan_path(body: &str) -> Option<String> {
    static PLAN_CALLOUT_RE: OnceLock<Regex> = OnceLock::new();
    let callout_re = PLAN_CALLOUT_RE.get_or_init(|| {
        Regex::new(r"(?m)^> - \*\*Plan:\*\* `(docs/plans/[^`]+)`")
            .expect("plan callout regex must compile")
    });
    callout_re.captures(body).map(|c| c[1].to_string())
}

/// Decide whether the plan named in a ticket's grooming callout belongs to that
/// ticket, from the issue body alone (mika#2020 R1/R2).
///
/// Fail-open on ambiguity, fail-closed on contradiction. The canonical plan
/// name is `<YYYY-MM-DD>-<seq>-<type>-<issue>-<slug>-plan.md`, and the issue
/// slot is read **anchored at that position** — never searched freely. The
/// anchor is the point: mika#2038 documented a permissive `*-2026-*` glob
/// matching `rustsec-2026-0097` and sending a pilot to an April plan. An
/// unanchored pattern here would commit the mirror-image fault — refusing a
/// ticket because some number in its slug resembles an issue number.
pub fn plan_ownership(body: &str, issue_number: u64) -> PlanOwnership {
    let Some(path) = extract_plan_path(body) else {
        return PlanOwnership::Unattributable;
    };
    let basename = path.rsplit('/').next().unwrap_or(&path);

    static ISSUE_SLOT_RE: OnceLock<Regex> = OnceLock::new();
    let slot_re = ISSUE_SLOT_RE.get_or_init(|| {
        // `2026-08-29-002-fix-2038-…` → 2038. The `\d{3,4}` sequence field also
        // covers the 4-digit time-style variant (`2026-08-29-1249-security-2039-…`).
        Regex::new(r"^\d{4}-\d{2}-\d{2}-\d{3,4}-[a-z]+-(\d+)-")
            .expect("plan issue-slot regex must compile")
    });

    match slot_re
        .captures(basename)
        .and_then(|c| c[1].parse::<u64>().ok())
    {
        Some(slot) if slot == issue_number => PlanOwnership::Owned,
        Some(slot) => PlanOwnership::OwnedByOther(slot),
        None => PlanOwnership::Unattributable,
    }
}

/// Phase 0/Phase 1 candidate filter (mika#2020 R1, KTD7): `true` when the
/// ticket's plan is positively attributed to a *different* issue, in which case
/// the ticket is dropped from the candidate set and the mismatch is logged.
///
/// Phase 0 and Phase 1 pick one candidate out of many, so a ticket they drop is
/// not an announced dead end — it simply is not this tick's pick, and a `warn!`
/// is the proportionate signal. The full gesture (remove `ready`, apply
/// `operator-review`, comment) belongs to Phase 2, where the ticket already
/// carries `ready` and a dispatch is imminent.
fn warn_and_reject_foreign_plan(issue: &Issue) -> bool {
    match plan_ownership(&issue.body, issue.number) {
        PlanOwnership::OwnedByOther(owner) => {
            warn!(
                issue = issue.number,
                plan = %extract_plan_path(&issue.body).unwrap_or_else(|| "<unparseable>".to_string()),
                owner,
                "auto_pull_plan_ownership_mismatch"
            );
            true
        }
        PlanOwnership::Owned | PlanOwnership::Unattributable => false,
    }
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
        .filter(|i| !warn_and_reject_foreign_plan(i))
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
/// The per-ticket state Phase 2 resolves from GitHub + the DB before deciding
/// (mika#2020). Split from the decision itself so the decision is pure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct StuckReadyFacts {
    /// An open PR closes this ticket.
    has_open_pr: bool,
    /// A self_dev task is in flight for this ticket.
    in_flight: bool,
    /// `auto_pull_stats.failure_count` is at or past the circuit-breaker threshold.
    circuit_broken: bool,
    /// Successful re-drives since the last observed progress.
    redrive_count: i64,
    /// `redrive_abandoned_at` is set — this ticket was handed to the operator.
    abandoned: bool,
}

/// What Phase 2 should do with one `ready` ticket (mika#2020).
#[derive(Debug, Clone, PartialEq, Eq)]
enum StuckReadyVerdict {
    /// Eligible for a re-drive, pending the label-age check.
    Eligible,
    /// Not this tick. `reason` is the value of the existing
    /// `stuck_ready_reconcile_skipped` DEBUG field.
    Skip { reason: &'static str },
    /// Not this tick, and the re-drive budget goes back to zero — the ticket
    /// shows observable progress (mika#2020 R6).
    SkipAndResetBudget { reason: &'static str },
    /// The operator lifted an abandonment by removing `operator-review`: clear
    /// the budget and let the ticket back in (mika#2020 R12).
    ReEntry,
    /// Stop re-driving, and say so (mika#2020 R3, R5).
    Abandon(AbandonReason),
}

/// The in-memory half of the Phase 2 decision — the two checks that need no I/O
/// (mika#2020 KTD6). Called first by the async wrapper so a ticket refused here
/// costs no DB round-trip, and again from [`classify_stuck_ready`] so the pure
/// decision stays whole and testable in one place.
fn classify_stuck_ready_in_memory(issue: &Issue) -> Option<StuckReadyVerdict> {
    // Filter A: the ticket is already in the operator's hands (or blocked).
    if is_feeder_excluded(issue) {
        return Some(StuckReadyVerdict::Skip {
            reason: "operator_review_or_blocked",
        });
    }

    // Filter B: the plan belongs to another issue. Refused on sight, without
    // spending a re-drive — `is_groomed` says `true` here, so a re-drive would
    // send dev-groom straight past grooming into implementing the wrong plan.
    if let PlanOwnership::OwnedByOther(owner) = plan_ownership(&issue.body, issue.number) {
        let plan = extract_plan_path(&issue.body).unwrap_or_else(|| "<unparseable>".to_string());
        return Some(StuckReadyVerdict::Abandon(
            AbandonReason::PlanOwnedByOtherIssue { plan, owner },
        ));
    }

    None
}

/// The whole Phase 2 decision for one ticket (mika#2020). Pure — every input is
/// already resolved.
fn classify_stuck_ready(
    issue: &Issue,
    facts: &StuckReadyFacts,
    redrive_budget: i64,
) -> StuckReadyVerdict {
    if let Some(verdict) = classify_stuck_ready_in_memory(issue) {
        return verdict;
    }

    // Progress: an open PR or a live dispatch means the re-drives worked. Both
    // are already-computed filters, so the reset costs nothing extra (KTD5).
    if facts.has_open_pr {
        return StuckReadyVerdict::SkipAndResetBudget {
            reason: "open_pr_closing",
        };
    }
    if facts.in_flight {
        return StuckReadyVerdict::SkipAndResetBudget {
            reason: "in_flight_self_dev",
        };
    }

    // Past filter A, a ticket carrying an abandonment stamp no longer carries
    // `operator-review` — the operator removed it. That is the re-entry gesture.
    if facts.abandoned {
        return StuckReadyVerdict::ReEntry;
    }

    if facts.circuit_broken {
        return StuckReadyVerdict::Skip {
            reason: "circuit_breaker",
        };
    }

    // `0` disables the budget (pre-fix behaviour, kept as an escape hatch).
    if redrive_budget > 0 && facts.redrive_count >= redrive_budget {
        return StuckReadyVerdict::Abandon(AbandonReason::RedriveBudgetExhausted {
            redrives: facts.redrive_count,
            budget: redrive_budget,
        });
    }

    StuckReadyVerdict::Eligible
}

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
        .filter(|i| !warn_and_reject_foreign_plan(i))
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

/// Post a comment on a GitHub issue (mika#2020). Mirrors [`gh_apply_label`]'s
/// process setup.
///
/// This is the channel that makes an abandonment reach a human. A `debug!` line
/// is not a refusal anyone reads — mika#1901 was re-driven 16 times, and the
/// only way to notice was to count timeline events by hand.
async fn gh_comment_issue(github_token: &str, issue_number: u64, body: &str) -> Result<()> {
    let mut cmd = tokio::process::Command::new("gh");
    cmd.args([
        "issue",
        "comment",
        &issue_number.to_string(),
        "--repo",
        DEFAULT_REPO,
        "--body",
        body,
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
            "gh issue comment failed for #{}: {}",
            issue_number,
            stderr
        ));
    }
    Ok(())
}

// ───────────────────── Named abandonment (mika#2020) ─────────────────────

/// Why the reconciler stopped re-driving a ticket (mika#2020).
///
/// The two variants carry different urgency by design. A ticket whose plan
/// belongs to another issue is abandoned on sight, because re-driving it is
/// *actively harmful*: [`is_groomed`] answers `true`, so dev-groom skips
/// grooming and dispatches straight into implementing the wrong plan. Every
/// other dead end — including a ticket with no callout at all, the mika#1901
/// case — spends its budget first, because a `ready` label on an ungroomed
/// ticket is the pipeline's *nominal* entry state and re-driving it is how
/// dev-groom gets another chance to produce the plan.
///
/// A ticket with no plan is less dangerous than a ticket with the wrong plan.
/// Both are refused; they are not refused at the same speed.
#[derive(Debug, Clone, PartialEq, Eq)]
enum AbandonReason {
    /// The body's plan callout points at a plan belonging to another issue.
    PlanOwnedByOtherIssue { plan: String, owner: u64 },
    /// The per-ticket re-drive budget is spent with no observable progress.
    RedriveBudgetExhausted { redrives: i64, budget: i64 },
}

impl AbandonReason {
    /// Stable short slug for structured logs and audit events.
    fn slug(&self) -> &'static str {
        match self {
            Self::PlanOwnedByOtherIssue { .. } => "plan_owned_by_other_issue",
            Self::RedriveBudgetExhausted { .. } => "redrive_budget_exhausted",
        }
    }

    /// Human-readable statement of what went wrong.
    fn reason(&self, issue_number: u64) -> String {
        match self {
            Self::PlanOwnedByOtherIssue { plan, owner } => format!(
                "Le callout `> - **Plan:**` de #{issue_number} désigne `{plan}`, \
                 dont le créneau d'issue porte **#{owner}** — le plan d'un autre ticket. \
                 Un pilote dispatché ici implémenterait l'intention de #{owner} \
                 en croyant travailler sur #{issue_number}."
            ),
            Self::RedriveBudgetExhausted { redrives, budget } => format!(
                "L'auto-pull a re-drivé #{issue_number} **{redrives} fois** \
                 (budget : {budget}) sans progrès observable — aucune PR ouverte \
                 fermant ce ticket, aucune tâche `self_dev` en vol. \
                 Chaque re-drive consomme un créneau de dispatch."
            ),
        }
    }

    /// What it would take to pass.
    fn remedy(&self, issue_number: u64) -> String {
        match self {
            Self::PlanOwnedByOtherIssue { owner, .. } => format!(
                "corrige le callout `> - **Plan:**` du corps de #{issue_number} \
                 pour qu'il désigne un plan de #{issue_number} (ou re-groome le ticket \
                 pour en produire un), et vérifie que le plan de #{owner} n'a pas été \
                 écrasé au passage"
            ),
            Self::RedriveBudgetExhausted { .. } => format!(
                "groome #{issue_number} jusqu'à ce que son corps porte un callout complet \
                 (`Branch:` + `Plan:` + `second-pass (GROOMED)`), ou détermine ce qui \
                 empêche le dispatch de démarrer"
            ),
        }
    }

    /// The comment body posted on the abandoned ticket. Names the three things a
    /// predictable refusal owes its reader: the ticket, the reason, the remedy.
    fn comment_body(&self, issue_number: u64) -> String {
        format!(
            "## Auto-pull : re-drive abandonné pour #{issue_number}\n\n\
             **Raison.** {}\n\n\
             **Ce qu'il faudrait pour passer.** Pour remettre ce ticket en jeu : {}, \
             puis retire le label `operator-review`.\n\n\
             Le label `ready` a été retiré et `operator-review` posé — l'auto-pull ne \
             re-drivera plus ce ticket tant qu'il porte ce label. Retirer `operator-review` \
             remet son compteur de re-drives à zéro.\n\n\
             <sub>Émis par le reconciler stuck-ready (mika#1824), borné par mika#2020. \
             Événement : `auto_pull_redrive_abandoned` (`reason={}`).</sub>",
            self.reason(issue_number),
            self.remedy(issue_number),
            self.slug(),
        )
    }
}

/// Stop re-driving a ticket, and say so where a human will see it (mika#2020
/// R8–R10, R13).
///
/// Order matters: `ready` comes off first because that is what actually stops
/// the loop; everything after it is visibility layered on top of an arrest
/// already secured. A failure to comment degrades the refusal's reach, not its
/// effect.
async fn abandon_stuck_ready(
    db: &AsyncDatabase,
    github_token: &str,
    issue_number: u64,
    reason: AbandonReason,
    trace_id: &str,
    session_id: &str,
) {
    if let Err(e) = gh_remove_label(github_token, issue_number, "ready").await {
        warn!(error = %e, issue = issue_number, "auto_pull: abandon could not remove ready label");
        if let Err(e2) = db
            .increment_auto_pull_failure(DEFAULT_REPO, issue_number)
            .await
        {
            warn!(error = %e2, "auto_pull: failed to increment failure counter");
        }
        return;
    }

    if let Err(e) = gh_apply_label(github_token, issue_number, "operator-review").await {
        warn!(error = %e, issue = issue_number, "auto_pull: abandon could not apply operator-review label");
    }

    if let Err(e) = gh_comment_issue(
        github_token,
        issue_number,
        &reason.comment_body(issue_number),
    )
    .await
    {
        warn!(error = %e, issue = issue_number, "auto_pull: abandon could not post comment");
    }

    if let Err(e) = db
        .mark_auto_pull_redrive_abandoned(DEFAULT_REPO, issue_number)
        .await
    {
        warn!(error = %e, issue = issue_number, "auto_pull: failed to stamp abandonment");
    }

    warn!(
        issue = issue_number,
        reason = reason.slug(),
        detail = %reason.reason(issue_number),
        "auto_pull_redrive_abandoned"
    );

    if let Err(e) = db
        .log_audit_event(
            session_id,
            "auto_pull",
            "auto_pull_redrive_abandoned",
            None,
            Some(&format!(
                "#{} abandoned: reason={}",
                issue_number,
                reason.slug()
            )),
            None,
            Some(trace_id),
        )
        .await
    {
        warn!(error = %e, "auto_pull: failed to write abandonment audit event");
    }
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
    let rescued = phase2_reconcile_stuck_ready(
        db,
        github_token,
        &issues,
        &open_pr_issue_numbers,
        trace_id,
        session_id,
    )
    .await;
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
    trace_id: &str,
    session_id: &str,
) -> usize {
    let threshold = stuck_ready_threshold_secs();
    let redrive_budget = max_redrives();

    // Filter 1 (in-mem): keep only tickets WITH the `ready` label (inverse of
    // Phase 1's filter). Everything without `ready` is Phase 1 territory.
    let ready: Vec<&Issue> = issues
        .iter()
        .filter(|i| i.labels.iter().any(|l| l.name == "ready"))
        .collect();
    if ready.is_empty() {
        return 0;
    }

    // Filters 2–8. Ordering is cheapest-first (in-mem → DB → GitHub API), so the
    // timeline call in the age step fires for as few tickets as possible. The
    // decision itself is the pure `classify_stuck_ready`; this loop only
    // resolves its inputs and enacts its verdict.
    let mut in_flight_issue_numbers: HashSet<u64> = HashSet::new();
    let mut survivors: Vec<u64> = Vec::new();
    for issue in &ready {
        let n = issue.number;

        // Filters 2–3 (in-mem): operator-held tickets and misattributed plans.
        // Decided before any I/O (mika#2020 KTD6).
        if let Some(verdict) = classify_stuck_ready_in_memory(issue) {
            match verdict {
                StuckReadyVerdict::Skip { reason } => {
                    debug!(issue = n, reason, "stuck_ready_reconcile_skipped");
                }
                StuckReadyVerdict::Abandon(reason) => {
                    abandon_stuck_ready(db, github_token, n, reason, trace_id, session_id).await;
                }
                // `classify_stuck_ready_in_memory` yields only those two.
                other => debug!(
                    issue = n,
                    ?other,
                    "stuck_ready_reconcile_unexpected_verdict"
                ),
            }
            continue;
        }

        // Filter 4 (DB, cheap): in-flight self_dev task for this issue.
        let issue_url = format!("https://github.com/{}/issues/{}", DEFAULT_REPO, n);
        let in_flight = match db.has_active_self_dev_task_for_issue(&issue_url).await {
            Ok(v) => v,
            Err(e) => {
                warn!(error = %e, issue = n, "auto_pull: phase 2 in-flight check failed; skipping ticket");
                continue;
            }
        };
        if in_flight {
            in_flight_issue_numbers.insert(n);
        }

        // Filter 5 (DB, cheap): circuit-breaker. Fail-open on error.
        let circuit_broken = match db.get_auto_pull_failure_count(DEFAULT_REPO, n).await {
            Ok(count) => count >= CIRCUIT_BREAKER_THRESHOLD,
            Err(e) => {
                warn!(error = %e, issue = n, "auto_pull: phase 2 circuit-breaker check failed; proceeding");
                false
            }
        };

        // Filter 6 (DB, cheap): re-drive budget + abandonment stamp (mika#2020).
        // Fail-open on error — an unreadable counter must not strand a ticket.
        let (redrive_count, abandoned) = match db.get_auto_pull_redrive_state(DEFAULT_REPO, n).await
        {
            Ok(state) => state,
            Err(e) => {
                warn!(error = %e, issue = n, "auto_pull: phase 2 re-drive state read failed; proceeding");
                (0, false)
            }
        };

        let facts = StuckReadyFacts {
            has_open_pr: open_pr_issue_numbers.contains(&n),
            in_flight,
            circuit_broken,
            redrive_count,
            abandoned,
        };

        match classify_stuck_ready(issue, &facts, redrive_budget) {
            StuckReadyVerdict::Eligible => survivors.push(n),
            StuckReadyVerdict::Skip { reason } => {
                debug!(issue = n, reason, "stuck_ready_reconcile_skipped");
            }
            StuckReadyVerdict::SkipAndResetBudget { reason } => {
                debug!(issue = n, reason, "stuck_ready_reconcile_skipped");
                if redrive_count > 0
                    && let Err(e) = db.reset_auto_pull_redrive(DEFAULT_REPO, n).await
                {
                    warn!(error = %e, issue = n, "auto_pull: failed to reset re-drive budget on progress");
                }
            }
            StuckReadyVerdict::ReEntry => {
                if let Err(e) = db.reset_auto_pull_redrive(DEFAULT_REPO, n).await {
                    warn!(error = %e, issue = n, "auto_pull: failed to clear abandonment on re-entry");
                    continue;
                }
                info!(
                    issue = n,
                    prior_redrives = redrive_count,
                    "auto_pull_redrive_reentry"
                );
                survivors.push(n);
            }
            StuckReadyVerdict::Abandon(reason) => {
                abandon_stuck_ready(db, github_token, n, reason, trace_id, session_id).await;
            }
        }
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
        // mika#2020: the line above is exactly where the loop used to erase its
        // own memory. `failure_count` going back to zero is correct — the API
        // call succeeded — but it left nothing counting the rescues themselves,
        // so #1901 could be re-driven 16 times and look brand new each round.
        // The re-drive budget increments on the same successful event.
        if let Err(e) = db.increment_auto_pull_redrive(DEFAULT_REPO, n).await {
            warn!(error = %e, issue = n, "auto_pull: failed to increment re-drive counter");
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

    // ── mika#2020: plan ownership ──

    /// Body shape whose plan callout carries a canonical issue slot.
    fn body_with_plan(plan_basename: &str) -> String {
        format!(
            "> - **Branch:** `fix/1/x`\n\
             > - **Plan:** `docs/plans/{plan_basename}` (committed on branch @ abc1234)\n\
             > - **Grooming history:** first-pass (READY) → second-pass (GROOMED) — session-id: 550e8400"
        )
    }

    #[test]
    fn test_plan_ownership_owned() {
        let body =
            body_with_plan("2026-08-29-002-fix-2038-find-issue-plan-tier1-refutation-plan.md");
        assert_eq!(plan_ownership(&body, 2038), PlanOwnership::Owned);
    }

    #[test]
    fn test_plan_ownership_owned_by_other_the_1887_incident() {
        // The literal callout mika#1887 carried: a real plan file belonging to
        // #1933. The pilot opened it and had no way to know.
        let body =
            body_with_plan("2026-08-21-002-fix-1933-reader-completed-section-avancement-plan.md");
        assert_eq!(
            plan_ownership(&body, 1887),
            PlanOwnership::OwnedByOther(1933),
            "a plan positively attributed to another issue must be refused"
        );
    }

    #[test]
    fn test_plan_ownership_four_digit_sequence_field() {
        // `1249` is the sequence field, `2039` the issue slot — the regex must
        // not mistake one for the other.
        let body =
            body_with_plan("2026-08-29-1249-security-2039-pat-github-hors-argv-bwrap-plan.md");
        assert_eq!(plan_ownership(&body, 2039), PlanOwnership::Owned);
        assert_eq!(
            plan_ownership(&body, 1249),
            PlanOwnership::OwnedByOther(2039)
        );
    }

    #[test]
    fn test_plan_ownership_historical_names_are_unattributable() {
        for name in [
            "2047-disable-release-please-workflow.md",
            "918-eval-kg-fixtures-schema-pin-v29-const-assert-e2e-test.md",
            "mika-1221-pre-fix-self-model-backup.txt",
        ] {
            let body = body_with_plan(name);
            assert_eq!(
                plan_ownership(&body, 1),
                PlanOwnership::Unattributable,
                "{name} has no canonical issue slot — ambiguity must fail open"
            );
        }
    }

    #[test]
    fn test_plan_ownership_no_callout_is_unattributable() {
        assert_eq!(
            plan_ownership(UNGROOMED_BODY, 1901),
            PlanOwnership::Unattributable,
            "mika#1901 carried no callout at all"
        );
    }

    #[test]
    fn test_plan_ownership_anchored_against_the_2038_glob_trap() {
        // mika#2038: a permissive `*-2026-*` glob matched `rustsec-2026-0097`.
        // The anchored slot regex must not read a slug number as an issue slot.
        let body =
            body_with_plan("2026-04-11-003-chore-deps-bump-rand-clear-rustsec-2026-0097-plan.md");
        // `chore` is the type field, `deps` is not a number → no slot at the
        // canonical position, so no accusation.
        assert_eq!(plan_ownership(&body, 2026), PlanOwnership::Unattributable);
    }

    #[test]
    fn test_plan_ownership_path_outside_docs_plans_is_unattributable() {
        let body = "> - **Plan:** `notes/2026-08-29-002-fix-2038-x-plan.md`";
        assert_eq!(plan_ownership(body, 2038), PlanOwnership::Unattributable);
    }

    // ── mika#2020: re-drive budget parsing ──

    #[test]
    fn test_max_redrives_parse_contract() {
        assert_eq!(parse_max_redrives(None), MAX_REDRIVES_DEFAULT);
        assert_eq!(parse_max_redrives(Some("")), MAX_REDRIVES_DEFAULT);
        assert_eq!(parse_max_redrives(Some("   ")), MAX_REDRIVES_DEFAULT);
        assert_eq!(parse_max_redrives(Some("-1")), MAX_REDRIVES_DEFAULT);
        assert_eq!(parse_max_redrives(Some("abc")), MAX_REDRIVES_DEFAULT);
        assert_eq!(parse_max_redrives(Some("7")), 7);
        assert_eq!(
            parse_max_redrives(Some("0")),
            0,
            "0 is the disable sentinel"
        );
    }

    // ── mika#2020: Phase 2 classification ──

    fn facts(redrive_count: i64) -> StuckReadyFacts {
        StuckReadyFacts {
            has_open_pr: false,
            in_flight: false,
            circuit_broken: false,
            redrive_count,
            abandoned: false,
        }
    }

    #[test]
    fn test_classify_operator_review_is_skipped_never_abandoned() {
        let issue = make_issue(1, UNGROOMED_BODY, &["ready", "operator-review"], "t");
        assert_eq!(
            classify_stuck_ready(&issue, &facts(99), 3),
            StuckReadyVerdict::Skip {
                reason: "operator_review_or_blocked"
            },
            "a ticket already in the operator's hands gets no second gesture"
        );
    }

    #[test]
    fn test_classify_foreign_plan_abandons_without_spending_budget() {
        let body =
            body_with_plan("2026-08-21-002-fix-1933-reader-completed-section-avancement-plan.md");
        let issue = make_issue(1887, &body, &["ready"], "t");
        match classify_stuck_ready(&issue, &facts(0), 3) {
            StuckReadyVerdict::Abandon(AbandonReason::PlanOwnedByOtherIssue { plan, owner }) => {
                assert_eq!(owner, 1933);
                assert!(plan.contains("fix-1933"));
            }
            other => panic!("expected immediate abandon, got {other:?}"),
        }
    }

    #[test]
    fn test_classify_ungroomed_ticket_spends_budget_then_abandons() {
        // The mika#1901 shape: `ready` on a ticket with no callout at all.
        // `ready` on an ungroomed ticket is the pipeline's NOMINAL entry state
        // — re-driving it is how dev-groom gets another chance — so it is
        // bounded, not refused on sight.
        let issue = make_issue(1901, UNGROOMED_BODY, &["ready"], "t");
        for count in 0..3 {
            assert_eq!(
                classify_stuck_ready(&issue, &facts(count), 3),
                StuckReadyVerdict::Eligible,
                "re-drive {count} is still within budget"
            );
        }
        assert_eq!(
            classify_stuck_ready(&issue, &facts(3), 3),
            StuckReadyVerdict::Abandon(AbandonReason::RedriveBudgetExhausted {
                redrives: 3,
                budget: 3,
            }),
            "16 re-drives becomes 3"
        );
    }

    #[test]
    fn test_classify_zero_budget_never_abandons_for_budget() {
        let issue = make_issue(1901, UNGROOMED_BODY, &["ready"], "t");
        assert_eq!(
            classify_stuck_ready(&issue, &facts(999), 0),
            StuckReadyVerdict::Eligible,
            "budget 0 restores the pre-fix unbounded behaviour"
        );
    }

    #[test]
    fn test_classify_progress_resets_budget() {
        let issue = make_issue(1, UNGROOMED_BODY, &["ready"], "t");

        let mut f = facts(2);
        f.has_open_pr = true;
        assert_eq!(
            classify_stuck_ready(&issue, &f, 3),
            StuckReadyVerdict::SkipAndResetBudget {
                reason: "open_pr_closing"
            }
        );

        let mut f = facts(2);
        f.in_flight = true;
        assert_eq!(
            classify_stuck_ready(&issue, &f, 3),
            StuckReadyVerdict::SkipAndResetBudget {
                reason: "in_flight_self_dev"
            }
        );
    }

    #[test]
    fn test_classify_progress_outranks_an_exhausted_budget() {
        // A ticket that produced a PR must not be abandoned for a stale count.
        let issue = make_issue(1, UNGROOMED_BODY, &["ready"], "t");
        let mut f = facts(3);
        f.has_open_pr = true;
        assert_eq!(
            classify_stuck_ready(&issue, &f, 3),
            StuckReadyVerdict::SkipAndResetBudget {
                reason: "open_pr_closing"
            }
        );
    }

    #[test]
    fn test_classify_reentry_when_operator_lifts_the_label() {
        // Abandoned earlier (stamp set), `operator-review` no longer present →
        // the operator put it back in play.
        let issue = make_issue(1901, UNGROOMED_BODY, &["ready"], "t");
        let mut f = facts(3);
        f.abandoned = true;
        assert_eq!(
            classify_stuck_ready(&issue, &f, 3),
            StuckReadyVerdict::ReEntry
        );
    }

    #[test]
    fn test_classify_abandoned_and_still_labelled_stays_silent() {
        // R13: no second comment on subsequent ticks.
        let issue = make_issue(1901, UNGROOMED_BODY, &["ready", "operator-review"], "t");
        let mut f = facts(3);
        f.abandoned = true;
        assert_eq!(
            classify_stuck_ready(&issue, &f, 3),
            StuckReadyVerdict::Skip {
                reason: "operator_review_or_blocked"
            }
        );
    }

    #[test]
    fn test_classify_circuit_breaker_still_wins_over_budget() {
        let issue = make_issue(1, UNGROOMED_BODY, &["ready"], "t");
        let mut f = facts(0);
        f.circuit_broken = true;
        assert_eq!(
            classify_stuck_ready(&issue, &f, 3),
            StuckReadyVerdict::Skip {
                reason: "circuit_breaker"
            },
            "the mika#1363 breaker keeps its own semantics (R7)"
        );
    }

    // ── mika#2020: abandonment message contract ──

    #[test]
    fn test_abandon_reason_names_ticket_reason_and_remedy() {
        let reason = AbandonReason::PlanOwnedByOtherIssue {
            plan: "docs/plans/2026-08-21-002-fix-1933-x-plan.md".to_string(),
            owner: 1933,
        };
        let body = reason.comment_body(1887);
        assert!(body.contains("#1887"), "names the ticket");
        assert!(body.contains("fix-1933"), "names the offending plan");
        assert!(body.contains("#1933"), "names the plan's real owner");
        assert!(
            body.contains("operator-review"),
            "names the re-entry gesture — R12's remedy must survive rewrites"
        );
        assert!(body.contains("auto_pull_redrive_abandoned"));
    }

    #[test]
    fn test_abandon_reason_budget_names_counts() {
        let reason = AbandonReason::RedriveBudgetExhausted {
            redrives: 3,
            budget: 3,
        };
        let body = reason.comment_body(1901);
        assert!(body.contains("#1901"));
        assert!(body.contains("3 fois"), "names the measured count");
        assert!(body.contains("operator-review"));
        assert_eq!(reason.slug(), "redrive_budget_exhausted");
    }

    // ── mika#2020: Phase 0 / Phase 1 tightening ──

    #[test]
    fn test_select_best_candidate_rejects_foreign_plan() {
        let foreign =
            body_with_plan("2026-08-21-002-fix-1933-reader-completed-section-avancement-plan.md");
        let issues = vec![
            // Higher priority, but its plan belongs to #1933.
            make_issue(1887, &foreign, &["p0"], "2026-08-01T00:00:00Z"),
            make_issue(2, GROOMED_BODY, &["p1"], "2026-08-01T00:00:00Z"),
        ];
        let selected = select_best_candidate(issues, &HashSet::new()).unwrap();
        assert_eq!(
            selected.number, 2,
            "a misattributed plan loses to a correctly-attributed one, whatever its priority"
        );
    }

    #[test]
    fn test_select_feeder_candidates_rejects_foreign_plan() {
        let foreign =
            body_with_plan("2026-08-21-002-fix-1933-reader-completed-section-avancement-plan.md");
        let issues = vec![
            make_issue(1887, &foreign, &["p0-critical"], "2026-08-01T00:00:00Z"),
            make_issue(2, GROOMED_BODY, &["p2-normal"], "2026-08-01T00:00:00Z"),
        ];
        let selected = select_feeder_candidates(&issues, &HashSet::new(), &HashSet::new(), 5);
        assert_eq!(selected, vec![2]);
    }

    #[test]
    fn test_unattributable_plan_still_passes_every_phase() {
        // R2: the guard accuses on contradiction, never on ambiguity.
        let historical = body_with_plan("918-eval-kg-fixtures-schema-pin.md");
        let issues = vec![make_issue(1, &historical, &["p1-important"], "t")];
        assert_eq!(
            select_feeder_candidates(&issues, &HashSet::new(), &HashSet::new(), 5),
            vec![1]
        );
        assert!(select_best_candidate(issues, &HashSet::new()).is_some());

        let ready_issue = make_issue(1, &historical, &["ready"], "t");
        assert_eq!(
            classify_stuck_ready(&ready_issue, &facts(0), 3),
            StuckReadyVerdict::Eligible
        );
    }
}
