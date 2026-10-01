//! Auto-resume WIP-rescue drafts (mika#1852, RT#004).
//!
//! A periodic, low-priority scan that picks up the draft PRs dispatch-lib
//! creates when a pilot session rescues uncommitted / un-PR'd work (the
//! `wip-rescue` label, mika#1282 / mika#1396) and drives them back toward a
//! reviewable state:
//!
//! ```text
//!   scan drafts (age > threshold)
//!    ├─ bailed marker?  → skip                              ── mika#2199
//!    ├─ parked marker (and body not `yes`)? → skip          ── mika#2286
//!    ├─ HELD? (≥1 ConvertToDraftEvent on the timeline) → skip ─ mika#2597
//!    └─ oldest survivor → resume_chain:
//!        └─ RESCUE_DEPTH gate  (bail-to-human at max)       ── F2
//!           └─ git ≥ 2.38 guard (bail: git-too-old-for-dry-run) ── F3
//!              └─ dry-run rebase (merge-tree, non-mutating)  ── AC2
//!                 └─ live rebase onto origin/main
//!                    └─ clippy gate (bail on errors)
//!                       └─ push rebased branch (force-with-lease)
//!                          └─ substrate-diff perimeter classify ── AC4 (#1831)
//!                             ├─ MECHANICAL, or DECISION-CORE whose body reads
//!                             │  `rescue-pipeline-verified: yes`
//!                             │   └─ un-draft (gh pr ready)  ── F1
//!                             └─ DECISION-CORE, marker not `yes`
//!                                 └─ park unverified, stay draft ── mika#2286
//! ```
//!
//! ## Safety invariants
//!
//! - **An explicit hold is respected (mika#2597).** A draft this daemon may
//!   promote is one that was *born* a draft. A draft somebody **put back** is a
//!   hold, and the daemon does not touch it: not selected, not rebased, not
//!   pushed, not un-drafted, `$.wip_rescue.depth` not incremented. The
//!   discriminant is the presence of a `ConvertToDraftEvent` on the timeline —
//!   see [`HoldVerdict`] for why presence alone suffices and why no instant is
//!   compared. Measured twice in 35 minutes on PR #2589 (2026-09-29): the
//!   autonomous merge was only stopped by the forge gate's decision-core
//!   perimeter, i.e. by coincidence of file set rather than by design.
//! - **Never mutate before a clean dry-run.** The `git merge-tree --write-tree`
//!   dry-run (non-mutating: writes only loose objects, never the worktree or a
//!   ref) runs before the live rebase. If the deploy host's git predates 2.38
//!   (no `--write-tree`), we **fail closed** — bail-to-human rather than fall
//!   back to a mutating rebase that would skip the dry-run (F3).
//! - **Bail-to-human is terminal, and terminal does not depend on GitHub
//!   (mika#2199).** Any uncertain condition (rebase conflict, clippy errors,
//!   un-draft failure, depth exhausted) writes a durable `audit_events` marker
//!   keyed by PR — **unconditionally, before any GitHub call** — and only then
//!   tries to park the PR with the `human-review-required` label + a comment
//!   naming the reason. The marker is what the eligibility filter reads, so a
//!   bail whose label write fails still excludes the draft from the next scan.
//!   Before mika#2199 the label was the sole exclusion and its failure was
//!   swallowed: the daemon re-elected the same oldest draft every tick — 17
//!   bails traced on 2026-09-05, **14 of them on PR #2197 alone** between 09:43
//!   and 16:05, with one `wip_rescue_error` each. (Counts are deduplicated:
//!   every line of the spirit log is written twice, so the raw greps read
//!   double.) The draft is preserved; a human decides. No further auto-attempts
//!   (AC3).
//! - **Perimeter gate is authoritative (AC4/AC5), and a DECISION-CORE draft is
//!   only un-drafted once verified (mika#2286).** Every draft — even a one-line
//!   diff — is classified by the mika#1831 perimeter classifier
//!   ([`crate::perimeter`]). A MECHANICAL draft is un-drafted normally so the
//!   verdict handler's merge path can fire. A DECISION-CORE draft is un-drafted
//!   **only** when its body reads `<!-- rescue-pipeline-verified: yes -->`, and
//!   then **with** a hand-merge comment (never auto-merged); otherwise it is
//!   *parked* — left a draft, commented, and excluded from the scan by a durable
//!   marker until the `yes` re-arms it ([`park_unverified`]). There is no
//!   trivial-diff carve-out.
//!
//!   This revises step 6 of the mika#1852 spec rather than contradicting it. That
//!   spec placed the un-draft **after** step 5, *"re-run pilot"* — i.e. after a
//!   fresh verification of the pipeline. The v1 below descoped steps 4-fix and 5
//!   (§ *Scope boundary (v1)*) and kept the un-draft, so what the spec treated as
//!   verified no longer was, and the marker that says so was read nowhere: on
//!   2026-09-10 PR #2285 was un-drafted by this daemon, classified DECISION-CORE,
//!   with `rescue-pipeline-verified: no` still in its body. Without a re-run, the
//!   daemon has no right to un-draft the sensitive class blind.
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
use mika_common::label_write::LabelWriteToken;
use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tracing::{debug, info, warn};

/// The repository the wip-rescue scan targets. Matches `auto_pull::DEFAULT_REPO`.
///
/// `pub(crate)` since mika#2624: the `run_gh` hold term needs the same fallback
/// when its caller declared no `repo`, and importing it is strictly better than
/// writing the literal a third time in the tree.
pub(crate) const DEFAULT_REPO: &str = "senara-solutions/mika";

/// Label dispatch-lib applies to rescued draft PRs (mika#1631). The scan only
/// ever touches drafts carrying this label.
const WIP_RESCUE_LABEL: &str = "wip-rescue";

/// Terminal escalation label applied by [`bail_to_human`].
const HUMAN_REVIEW_LABEL: &str = "human-review-required";

/// Colour + description used when the daemon has to create
/// [`HUMAN_REVIEW_LABEL`] itself (mika#2199 §4.1). Both are taken verbatim from
/// the label GitHub already accepted, so a `label create` here cannot introduce
/// a value the API refuses. The description is 82 characters — the 100-char
/// ceiling has bitten this repo twice (mika#2130, mika#2168).
const HUMAN_REVIEW_LABEL_COLOR: &str = "d93f0b";
const HUMAN_REVIEW_LABEL_DESC: &str =
    "wip-rescue: le démon a bailé vers un humain (conflit/erreur), à résoudre à la main";

/// `audit_events.tool_name` of the durable per-PR bail marker (mika#2199 §4.2).
///
/// Distinct from the `wip_rescue` tool name the other events use: this row is a
/// *state* marker read by the eligibility filter, not a log of an action, and
/// counting it must never pick up a `wip_rescue_success` or a resume attempt.
///
/// Precedent: `ci_success_handler_processed` (mika#1869) — an audit-durable gate
/// keyed by PR that survives a process restart. Same `pr:{repo}#{n}` key
/// convention; no migration, because `audit_events.tool_name` is free-form TEXT.
const BAILED_MARKER_TOOL: &str = "wip_rescue_bailed";

/// `audit_events.tool_name` of the durable per-PR **parked-unverified** marker
/// (mika#2286 §4.3).
///
/// Sibling of [`BAILED_MARKER_TOOL`], deliberately a different name because it
/// is a different state. A bail says *the daemon found something wrong and a
/// human owns this PR from here*; a park says *nothing is wrong, the class just
/// requires a verification that has not happened yet*. Collapsing the two would
/// make the operator who reads the PR look for a conflict or a red clippy that
/// does not exist — and would make the two populations uncountable apart.
///
/// The exclusion it drives is **re-armable**, which is the other half of the
/// difference: nothing clears a bail, whereas `rescue-pipeline-verified: yes`
/// in the body makes this marker stop excluding (see [`select_eligible`]).
const PARKED_MARKER_TOOL: &str = "wip_rescue_parked_unverified";

/// Lower bound for the per-PR marker lookups ([`BAILED_MARKER_TOOL`],
/// [`PARKED_MARKER_TOOL`]). Neither is a burst dedup, so the window is the whole
/// audit retention rather than a few seconds.
///
/// The real bound is therefore `compact_old_audit_events(90)`
/// (`server/mod.rs`): a draft still open after 90 days would be re-attempted
/// **once**. That is not a livelock, and its age is itself the signal that a
/// human should be looking at it.
const MARKER_LOOKUP_SINCE: &str = "1970-01-01T00:00:00Z";

/// `audit_events.target_key` of a per-PR state marker.
///
/// Shared by the bail and the park markers: the key identifies the *PR*, the
/// `tool_name` identifies *which* state. Reading one and finding the other is
/// therefore impossible, which is what lets the two exclusions coexist on one
/// draft without either needing to know about the other.
fn pr_marker_key(pr_number: u64) -> String {
    format!("pr:{DEFAULT_REPO}#{pr_number}")
}

// -- Hold observability (mika#2597) -----------------------------------------

/// `audit_events.tool_name` written once per 24 h per held draft.
///
/// **Expected regime: non-empty.** Each row is a hold the daemon did *not*
/// violate — the direct measure that the guard bites, which nothing gave
/// before. Its silence under a PR that keeps leaving draft is the halt: read
/// the positive control (`wip_rescue_resume_attempt`) before touching the
/// predicate.
const HOLD_RESPECTED_TOOL: &str = "wip_rescue_hold_respected";

/// `audit_events.tool_name` written once per 24 h per draft whose timeline
/// could not be read.
///
/// **Expected regime: empty.** A separate name from [`HOLD_RESPECTED_TOOL`]
/// because the two call for opposite remedies: one is the mechanism working,
/// the other is `wip_rescue` frozen by its own fail-closed rule and the cause is
/// the token or the API. Merging them would make an outage read as a success —
/// the `phantom_aged_out` / `phantom_sweep_spared` motif (mika#2156).
const HOLD_UNREADABLE_TOOL: &str = "wip_rescue_hold_unreadable";

/// Deduplication horizon of the two hold surfaces above (doctrine mika#2131).
///
/// A hold is a **state**, not an event: a draft held for three weeks would
/// otherwise write 288 lines a day. Same value and same reasoning as
/// `worktree_reaper`'s `REFUSAL_DEDUP_SECS`. The first observation is written
/// immediately, so the operator sees their hold acknowledged on the next tick
/// (≤ 5 min); the 289th is not.
const HOLD_DEDUP_SECS: i64 = 86_400;

/// `audit_events.target_key` of a hold observability row: `pr:{repo}#{n}@{motif}`.
///
/// The `@` separator is what makes a prefix `LIKE` safe — `pr:…#234@…` cannot be
/// matched by a query aimed at `#2343` (mika#2361). The motif rides in the key
/// **as well as** in the `tool_name` so a change of motif on the same PR writes
/// a distinct row rather than being swallowed by the first one's window.
fn hold_audit_key(pr_number: u64, motif: &str) -> String {
    format!("pr:{DEFAULT_REPO}#{pr_number}@{motif}")
}

/// `(owner, repo)` of [`DEFAULT_REPO`], split once rather than at each call.
pub(crate) fn default_repo_parts() -> (&'static str, &'static str) {
    DEFAULT_REPO
        .split_once('/')
        .expect("DEFAULT_REPO is `owner/repo`")
}

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

/// The HTML-comment key dispatch-lib stamps in every rescue PR body
/// (`_compose_rescue_pr_body`, mika#1618) — always `no` at creation.
///
/// The producer/consumer split is the whole point: dispatch-lib writes the
/// state once, a human flips it, and both readers (qa-review Step 1.5 and
/// [`pipeline_verified`] below) read rather than re-judge.
const PIPELINE_VERIFIED_KEY: &str = "rescue-pipeline-verified";

/// Whether a rescue PR body declares its pipeline verified.
///
/// `<!-- rescue-pipeline-verified: yes -->` → `true`. Everything else — `no`,
/// absent, any other value, an empty body, two markers that disagree — reads as
/// **not** verified (mika#2286, fail-closed).
///
/// Tolerance stops at whitespace around the value and the ASCII case of `yes`:
/// a human who types `Yes` has made the gesture, a human who types `probably`
/// has not. The "absent marker means pre-mika#1618 PR, proceed" fallback that
/// qa-review allows itself has **no** place here — every `wip-rescue` draft is
/// produced by dispatch-lib, which has stamped the marker since 2026-06-29, so
/// an absent one means the body was mangled, not that it predates the contract.
///
/// A mention of the key that is not a well-formed marker (this doc comment, the
/// PR comment [`park_unverified`] posts) contributes no value: it can neither
/// verify nor contradict. A marker value is a bare token, so a candidate span
/// carrying `<` is a prose mention that ran on until some *later* comment's
/// `-->` — counting it would let one sentence swallow the real marker behind it
/// and turn a verified PR unverified.
fn pipeline_verified(body: &str) -> bool {
    let mut saw_marker = false;
    let mut every_marker_says_yes = true;

    // `match_indices` enumerates every occurrence left to right, so a prose
    // mention that this loop skips does not hide the well-formed marker behind
    // it.
    for (idx, _) in body.match_indices(PIPELINE_VERIFIED_KEY) {
        let after = &body[idx + PIPELINE_VERIFIED_KEY.len()..];
        let Some(tail) = after.trim_start().strip_prefix(':') else {
            continue;
        };
        let Some(end) = tail.find("-->") else {
            continue;
        };
        let value = tail[..end].trim();
        if value.contains('<') {
            continue;
        }
        saw_marker = true;
        every_marker_says_yes &= value.eq_ignore_ascii_case("yes");
    }

    saw_marker && every_marker_says_yes
}

/// What to do with a draft once it is classified and its marker is read (AC2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UndraftDecision {
    /// Proceed to `gh pr ready` (MECHANICAL, or DECISION-CORE already verified).
    Undraft,
    /// Leave the draft a draft and park it until a human verifies
    /// (DECISION-CORE whose marker does not read `yes`).
    ParkUnverified,
}

/// The mika#2286 gate, as a pure function of the two facts that decide it.
///
/// | route | marker `yes` | decision |
/// |---|---|---|
/// | MECHANICAL | yes | `Undraft` |
/// | MECHANICAL | no | `Undraft` — unchanged, deliberately (see below) |
/// | DECISION-CORE | yes | `Undraft` + hand-merge comment |
/// | DECISION-CORE | no | `ParkUnverified` |
///
/// **MECHANICAL keeps its unverified auto-path on purpose.** That is the
/// mika#1852 design (RT#004): the mechanical class is already held by the
/// perimeter classifier, qa-review, CI and the forge gate. mika#2286 is scoped
/// to DECISION-CORE and ratifies the rest as it stands; closing the mechanical
/// path too is a separate ticket with its own measurement.
///
/// Note how this composes with the fail-closed classification: `classify_route`
/// answers DECISION-CORE when it cannot read the diff, so an unreadable
/// classification on an unverified marker **parks** instead of un-drafting. That
/// is "fail-closed" propagated one step further, not a new policy.
fn undraft_decision(route: UndraftRoute, verified: bool) -> UndraftDecision {
    match (route, verified) {
        (UndraftRoute::Mechanical, _) => UndraftDecision::Undraft,
        (UndraftRoute::DecisionCore, true) => UndraftDecision::Undraft,
        (UndraftRoute::DecisionCore, false) => UndraftDecision::ParkUnverified,
    }
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
    /// The PR body as the listing saw it, read by [`select_eligible`] to decide
    /// whether a parked draft has been re-armed (mika#2286). `gh pr list`
    /// already returns it for free; the *decision* re-reads a fresh copy, since
    /// rebase + clippy can take minutes and a human may flip the marker inside
    /// that window.
    #[serde(default)]
    body: String,
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

/// `gh pr view --json body` (mika#2286). `body` is defaulted because GitHub
/// renders an empty body as JSON `null`, and an empty body is a legitimate —
/// and, per [`pipeline_verified`], unverified — state.
#[derive(Debug, Deserialize)]
struct PrBodyEnvelope {
    #[serde(default)]
    body: String,
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
    ///
    /// `parked` says whether the GitHub label actually landed. It is reported
    /// rather than swallowed (mika#2199): a bail that could not park the PR is
    /// still terminal — the durable marker saw to that — but the operator who
    /// goes looking for the draft on GitHub will not find the label, so the
    /// difference has to be visible.
    ///
    /// Note this field's `parked` is **not** [`ChainOutcome::ParkedUnverified`]:
    /// here it means *the bail's label landed on GitHub*, there it names the
    /// mika#2286 state. Two senses of one word, kept because the first predates
    /// the second and the second is pinned by the marker name operators query.
    Bailed { reason: String, parked: bool },
    /// DECISION-CORE draft whose pipeline-verification marker does not read
    /// `yes` — left a draft, commented, and excluded until the `yes` re-arms it
    /// (mika#2286). **Not** a bail: nothing went wrong, and the exclusion lifts
    /// itself.
    ParkedUnverified,
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
    label_auth: &LabelWriteToken,
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

    let selected = select_eligible(
        drafts,
        &now,
        threshold,
        |pr_number| has_bailed_marker(db, pr_number, trace_id),
        |pr_number| has_parked_marker(db, pr_number, trace_id),
        |pr_number| hold_verdict(db, github_token, pr_number, trace_id, session_id),
    )
    .await;

    let Some((age_secs, pr)) = selected else {
        debug!(
            trace_id,
            threshold, "wip_rescue: no eligible drafts this tick"
        );
        return None;
    };

    match resume_chain(
        db,
        github_token,
        label_auth,
        trace_id,
        session_id,
        &pr,
        age_secs,
    )
    .await
    {
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
        ChainOutcome::Bailed { reason, parked } => {
            info!(pr_number = pr.number, reason = %reason, parked, trace_id, "wip_rescue: bailed");
            Some(0)
        }
        // `Some(0)` like a bail: a draft was examined and not resumed. The two
        // are told apart by the event name, never by the return value.
        ChainOutcome::ParkedUnverified => Some(0),
        ChainOutcome::Skipped(reason) => {
            debug!(pr_number = pr.number, reason = %reason, trace_id, "wip_rescue_skipped");
            Some(0)
        }
    }
}

/// Pick the draft this tick should work on: `wip-rescue`-labelled, not carrying
/// [`HUMAN_REVIEW_LABEL`], older than `threshold`, oldest first — and **not
/// already bailed** (mika#2199 AC2).
///
/// The bail check is the whole point. The other three predicates are pure
/// functions of the `gh` payload; this one asks the durable marker, which is why
/// `is_bailed` is injected: it makes the selection falsifiable without a
/// database, and lets a test supply the always-false predicate that reproduces
/// `main`'s livelock.
///
/// It is consulted **in age order with a short-circuit on the first non-bailed
/// candidate**, not eagerly over the whole list. Nominal cost is therefore one
/// read per 5-minute tick; the upper bound is the list length, itself capped at
/// 100 by the `--limit 100` on [`list_wip_rescue_drafts`].
///
/// Because the marker is read during filtering rather than between ticks, a
/// parked draft disappears from the candidate set **on the same tick** and the
/// next draft is returned immediately. mika#2199 only required the following
/// tick; this falls out of the shape of the code rather than widening the scope.
///
/// The predicates are async (the plan sketched them as `&dyn Fn(u64) -> bool`)
/// because the real ones are DB reads: a synchronous signature would have forced
/// either a blocking read or the eager fan-out the cost bound above rules out.
///
/// **Parked drafts (mika#2286).** A second, *re-armable* exclusion sits beside
/// the bail: a DECISION-CORE draft the daemon left unverified is skipped while
/// its marker is set **and** its body does not read
/// `rescue-pipeline-verified: yes`. Without it the draft would be re-elected on
/// every 5-minute tick — fetch, rebase, clippy (up to 900 s), push, classify,
/// *still `no`* — holding the single slot (AC6 cap = 1, oldest first) and
/// starving everything behind it. That is the mika#2199 livelock shape, measured
/// there as 14 bails on one PR in six hours.
///
/// The body test comes **first** on purpose: it is free, and a re-armed draft
/// must not pay a database read to discover it is eligible again.
///
/// **Held drafts (mika#2597).** A third exclusion sits last, for two reasons.
/// Cost: it is the only term that costs a network call, so it is paid on the
/// survivors of the two cheap ones, and the short-circuit bounds the nominal
/// regime to one GraphQL call per tick. Placement: putting a hold inside
/// [`resume_chain`] instead would consume the single per-tick slot (mika#1852
/// AC6, cap = 1, oldest first) **every tick, indefinitely**, starving everything
/// behind it — a hold draft is old by nature, so it stays the oldest candidate
/// for ever. That is exactly the livelock shape mika#2199 measured and mika#2286
/// had to close a second time.
async fn select_eligible<F, Fut, G, GFut, H, HFut>(
    drafts: Vec<DraftPr>,
    now: &str,
    threshold: i64,
    is_bailed: F,
    is_parked: G,
    is_held: H,
) -> Option<(i64, DraftPr)>
where
    F: Fn(u64) -> Fut,
    Fut: std::future::Future<Output = bool>,
    G: Fn(u64) -> GFut,
    GFut: std::future::Future<Output = bool>,
    H: Fn(u64) -> HFut,
    HFut: std::future::Future<Output = HoldVerdict>,
{
    let mut ranked: Vec<(i64, DraftPr)> = drafts
        .into_iter()
        .filter(|pr| pr.has_label(WIP_RESCUE_LABEL) && !pr.has_label(HUMAN_REVIEW_LABEL))
        .filter_map(|pr| {
            let age = draft_age_secs(&pr.created_at, now)?;
            (age >= threshold).then_some((age, pr))
        })
        .collect();
    ranked.sort_by(|a, b| b.0.cmp(&a.0));

    for (age, pr) in ranked {
        if is_bailed(pr.number).await {
            debug!(
                pr_number = pr.number,
                reason = "already_bailed",
                "wip_rescue_skipped"
            );
            continue;
        }
        if !pipeline_verified(&pr.body) && is_parked(pr.number).await {
            debug!(
                pr_number = pr.number,
                reason = "parked_unverified",
                "wip_rescue_skipped"
            );
            continue;
        }
        // Third and last (mika#2597): the only term costing a network call, so
        // it is paid on the survivors of the two cheap ones, and — thanks to the
        // short-circuit below — on at most one draft per tick in the nominal
        // regime. Its two excluding states are reported by the predicate itself,
        // at a collected level, so no `debug!` here would be read (mika#2131).
        if is_held(pr.number).await.excludes() {
            continue;
        }
        return Some((age, pr));
    }
    None
}

/// Whether a durable bail marker exists for this PR (mika#2199 §4.2).
///
/// **Fail-closed**: an unreadable audit trail reads as *already bailed*. This
/// inverts `ci_success_handler`'s fail-open policy on the same primitive, and
/// the inversion is the point — there, fail-open protects a legitimate merge;
/// here the arbitrage runs the other way and costs nothing. A PR that has bailed
/// is by definition destined for a human: excluding it wrongly loses no work (it
/// stays open, labelled, commented), while re-attempting it in a loop costs the
/// entire queue.
async fn has_bailed_marker(db: &AsyncDatabase, pr_number: u64, trace_id: &str) -> bool {
    has_marker(db, BAILED_MARKER_TOOL, pr_number, trace_id).await
}

/// Whether a durable *parked-unverified* marker exists for this PR (mika#2286).
///
/// **Fail-closed** for the same arbitrage as [`has_bailed_marker`], reached by a
/// slightly different road: a parked DECISION-CORE draft excluded in error loses
/// nothing — it stays open, a draft, carrying the comment that names the two
/// gestures which free it, and the very next tick after a human writes `yes`
/// re-elects it, because the body test in [`select_eligible`] runs before this
/// read. Re-attempting it in a loop, by contrast, costs the whole queue.
async fn has_parked_marker(db: &AsyncDatabase, pr_number: u64, trace_id: &str) -> bool {
    has_marker(db, PARKED_MARKER_TOOL, pr_number, trace_id).await
}

/// Shared read behind the two exclusion predicates. Fail-closed: an unreadable
/// audit trail answers *excluded*.
async fn has_marker(db: &AsyncDatabase, tool_name: &str, pr_number: u64, trace_id: &str) -> bool {
    let key = pr_marker_key(pr_number);
    match db
        .count_recent_audit_events_for_target(tool_name, &key, MARKER_LOOKUP_SINCE)
        .await
    {
        Ok(count) => count > 0,
        Err(e) => {
            warn!(pr_number, marker = tool_name, error = %e, trace_id, "wip_rescue_error");
            true
        }
    }
}

// ---------------------------------------------------------------------------
// The hold predicate (mika#2597)
// ---------------------------------------------------------------------------

/// Whether a draft is a **hold** — something a human or the orchestrator put
/// back into draft after the work — rather than a **rescue draft**, the state
/// `dispatch-lib` creates it in and which this daemon exists to promote.
///
/// Three states, never a `bool`: two of them exclude, and they call for opposite
/// remedies. `Held` means the mechanism worked; `Unreadable` means the mechanism
/// is frozen and the cause is the token or the API. A boolean would make the
/// second read as the first.
///
/// # Why the mere presence of a `ConvertToDraftEvent` suffices
///
/// A rescue PR is **created** a draft (`gh pr create --draft`), so its timeline
/// carries **no** `ConvertToDraftEvent` at birth. The only two draft-state
/// transitions are `ConvertToDraftEvent` and `ReadyForReviewEvent`. The listing
/// already filters `--draft`, so a PR carrying ≥ 1 `ConvertToDraftEvent` **and**
/// being a draft *now* necessarily had its last transition towards draft: that
/// is a hold. No date to compare, no ordering to establish, no pagination.
///
/// **Named fragility:** removing `--draft` from
/// [`list_wip_rescue_drafts`] breaks that equivalence. The direction of the
/// breakage is inertia (a ready PR would carry a phantom hold and be excluded
/// from a population it is not part of anyway), never a violation — and
/// [`tests::mika2597_le_listing_filtre_toujours_draft`] refuses the edit.
///
/// # Why the ticket's "posterior to the last push" is refused
///
/// mika#2597's option (b) compares the hold's instant to the last push. Two
/// measurements refuse it. (1) **`wip_rescue` pushes by itself** —
/// [`prepare_branch`] rebases and force-pushes at step 4, *before* the un-draft
/// of step 7, in the same chain: a "hold posterior to the last push" predicate
/// would be voided by the push of the very daemon about to violate the hold.
/// (2) A later push does not lift an operator's decision — a hold of renunciation
/// holds until the operator lifts it, not until the next commit.
///
/// # Why no machine-actor filter
///
/// mika#2315 carries one because its own `remove → add` would park itself. Here
/// **no** path writes a `ConvertToDraftEvent` (exhaustive search over `crates/`,
/// `skills/`, `scripts/`), so the term would have an empty population — and if a
/// machine ever did, reading its gesture as a hold is the **safe** side. The
/// actor and the instant are therefore *reported* on the observability line
/// without deciding anything, which is what makes the day it stops being empty
/// visible.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum HoldVerdict {
    /// No `ConvertToDraftEvent`: a rescue draft. Byte-for-byte the pre-mika#2597
    /// behaviour.
    NotHeld,
    /// At least one `ConvertToDraftEvent`. Excluded.
    Held {
        since: Option<String>,
        actor: Option<String>,
    },
    /// The timeline could not be read. **Excluded**, see [`hold_verdict`].
    Unreadable { error: String },
}

impl HoldVerdict {
    /// Whether this verdict takes the draft out of the candidate set.
    ///
    /// `NotHeld` is the only selectable state: *a term one cannot read is never
    /// a satisfied term* (mika#2277), applied here to the term "this draft is
    /// **not** held".
    fn excludes(&self) -> bool {
        !matches!(self, HoldVerdict::NotHeld)
    }

    /// The `audit_events` name this verdict writes, or `None` when it writes
    /// nothing. Exhaustive `match`, no `_` arm: a fourth state must decide.
    fn observability_tool(&self) -> Option<&'static str> {
        match self {
            HoldVerdict::NotHeld => None,
            HoldVerdict::Held { .. } => Some(HOLD_RESPECTED_TOOL),
            HoldVerdict::Unreadable { .. } => Some(HOLD_UNREADABLE_TOOL),
        }
    }
}

/// Classify a timeline read into a [`HoldVerdict`] — the **sole** site turning
/// `ConvertToDraftEvent`s into a hold decision.
///
/// Extracted from [`hold_verdict`] by mika#2624 so a second un-draft surface
/// (`run_gh pr ready`, `builtin_handlers.rs`) can reach the *same* discriminant
/// by calling it rather than by transcribing it. The fetch stays with
/// [`crate::github_graphql::fetch_convert_to_draft_events`] and the reporting
/// stays with [`report_hold`]: `run_gh` has neither an `AsyncDatabase` shaped
/// like this module's nor a `senara-solutions/mika`-hardcoded repo, so sharing
/// the whole of `hold_verdict` was never available — only the classification is
/// common, and only the classification moves.
///
/// Pure, synchronous, takes the `Result` rather than the `Vec` so the
/// **fail-closed** half is a property of this function instead of a branch at
/// each caller: an unreachable API, a 401/403/429, an unparseable payload or a
/// shape the extractor does not recognise all land on `Unreadable`.
/// `canonical_tokens::tests::mika2624_le_predicat_de_hold_a_un_lecteur_unique`
/// refuses a second site.
///
/// The three states and the reasoning behind "mere presence suffices" live on
/// [`HoldVerdict`]; this function only decides, it does not re-argue.
pub(crate) fn classify_hold_verdict(
    events: Result<Vec<crate::github_graphql::ConvertToDraftEvent>, String>,
) -> HoldVerdict {
    match events {
        Ok(events) => match events.last() {
            None => HoldVerdict::NotHeld,
            Some(last) => HoldVerdict::Held {
                since: last.created_at.clone(),
                actor: last.actor_login.clone(),
            },
        },
        Err(error) => HoldVerdict::Unreadable { error },
    }
}

/// Read the hold state of a draft and **report** it (R4), in that order.
///
/// This is the predicate production injects into [`select_eligible`]. Reading
/// and reporting live together because the report must name the *verdict*, and
/// the selection loop is deliberately free of a database and of a trace id — the
/// property that makes the whole fix testable without a network.
///
/// **Fail-closed towards "held".** An unreachable API, a 401/403/429, an
/// unparseable payload, a shape we do not recognise: all exclude — decided once
/// in [`classify_hold_verdict`]. The asymmetry is measured, not supposed — a
/// false "held" leaves a rescue draft waiting for an operator gesture (visible,
/// recoverable, and the pre-mika#1852 behaviour), while a false "not held"
/// un-drafts a PR whose merge is forbidden. It is also this module's **uniform**
/// policy: `has_bailed_marker`, `has_parked_marker`, `classify_route` and
/// `fresh_pipeline_verified` are all four fail-closed.
///
/// **Named cost:** a durable GitHub API outage freezes `wip_rescue` entirely.
/// That is why `Unreadable` carries its own event name — the inertia must be
/// greppable and must not be confused with a nominal hold.
async fn hold_verdict(
    db: &AsyncDatabase,
    token: &str,
    pr_number: u64,
    trace_id: &str,
    session_id: &str,
) -> HoldVerdict {
    let (owner, repo) = default_repo_parts();
    let verdict = classify_hold_verdict(
        crate::github_graphql::fetch_convert_to_draft_events(token, owner, repo, pr_number).await,
    );
    report_hold(db, session_id, pr_number, trace_id, &verdict).await;
    verdict
}

/// Emit the hold observability for one verdict, deduplicated per `(PR, motif)`
/// over [`HOLD_DEDUP_SECS`] (R4/R5).
///
/// `NotHeld` writes nothing at all: an observability that logs everybody
/// distinguishes nobody (mika#2131 AC7, the rule this module's siblings follow).
///
/// The log line goes through the **same** gate as the audit row, deliberately —
/// the neighbouring precedent (`worktree_reaper`'s main-checkout dirtiness
/// probe, mika#2449) does the same, and splitting them would give the two
/// surfaces two different populations for one fact. (That probe's name is
/// deliberately not spelled out here: it is SOLE WRITER of its own module and
/// its guard counts a doc-comment mention as a hit — correctly.) Named cost:
/// when the dedup read itself fails, both
/// are skipped, so a database that cannot be read is also a hold that cannot be
/// reported. That is an incident of its own, and the *decision* never depends on
/// it — the draft is excluded either way.
async fn report_hold(
    db: &AsyncDatabase,
    session_id: &str,
    pr_number: u64,
    trace_id: &str,
    verdict: &HoldVerdict,
) {
    let Some(tool) = verdict.observability_tool() else {
        return;
    };
    let key = hold_audit_key(pr_number, tool);
    let since = crate::timestamp::now_minus(chrono::Duration::seconds(HOLD_DEDUP_SECS));
    match db
        .count_recent_audit_events_for_target(tool, &key, &since)
        .await
    {
        Ok(0) => {}
        Ok(_) => return,
        Err(e) => {
            debug!(
                pr_number,
                marker = tool,
                error = %e,
                trace_id,
                "wip_rescue: hold marker re-read failed, observability skipped"
            );
            return;
        }
    }

    let detail = match verdict {
        HoldVerdict::Held { since, actor } => {
            let held_since = since.as_deref().unwrap_or("unknown");
            let held_by = actor.as_deref().unwrap_or("unknown");
            info!(
                event = HOLD_RESPECTED_TOOL,
                pr_number,
                held_since,
                held_by,
                trace_id,
                "wip_rescue: this draft was put back into draft — it is a hold, \
                 not a rescue draft; not selected, not rebased, not un-drafted \
                 (mika#2597)"
            );
            format!("held_since={held_since} held_by={held_by}")
        }
        HoldVerdict::Unreadable { error } => {
            warn!(
                event = HOLD_UNREADABLE_TOOL,
                pr_number,
                error = %error,
                trace_id,
                "wip_rescue: the PR timeline could not be read — excluded \
                 fail-closed; `wip_rescue` is frozen on this draft until the API \
                 answers (mika#2597)"
            );
            format!("error={error}")
        }
        // Unreachable: `observability_tool` answered `None` above.
        HoldVerdict::NotHeld => return,
    };

    if let Err(e) = db
        .log_audit_event(
            session_id,
            tool,
            &key,
            None,
            Some(&detail),
            Some("wip_rescue hold observability (mika#2597)"),
            Some(trace_id),
        )
        .await
    {
        warn!(pr_number, marker = tool, error = %e, trace_id, "wip_rescue_error");
    }
}

/// The per-draft cost-bounded chain (AC2). See module docs for the step map.
#[allow(clippy::too_many_arguments)]
async fn resume_chain(
    db: &AsyncDatabase,
    token: &str,
    label_auth: &LabelWriteToken,
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
            label_auth,
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
                label_auth,
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
                label_auth,
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
            return bail_to_human(
                token, label_auth, trace_id, session_id, db, pr.number, reason,
            )
            .await;
        }
        PrepareOutcome::Skip(reason) => {
            return ChainOutcome::Skipped(reason);
        }
    }

    // Step 6: substrate-diff perimeter classification (AC4/AC5). Fail-closed —
    // a fetch/parse failure classifies DECISION-CORE (never auto-merge).
    let route = classify_route(pr.number, token).await;

    // Step 6b (mika#2286): read the verification marker off a FRESH copy of the
    // body, not off the listing. Steps 2–5 above can take minutes (clippy alone
    // is budgeted 900 s) and the gesture the rescue body asks of the operator —
    // "set the marker above to `yes`" — can land inside that window. A failed
    // read is not verified: the next tick re-reads the listing and re-arms on
    // its own if the body does say `yes`.
    //
    // The read is unconditional even though `undraft_decision` ignores it on
    // the MECHANICAL route. Skipping it there would cost one `gh` call per
    // 5-minute tick and buy a *second* reader of the decision table — the
    // "only DECISION-CORE consults the marker" row, restated at the call site,
    // which would keep un-drafting on its own the day that row changes. This
    // repo has a compound entry for that failure class
    // (`two-predicates-for-one-concept-livelock-2026-09-03.md`); one subprocess
    // per tick is the cheaper side of the trade.
    let verified = fresh_pipeline_verified(pr.number, token, trace_id).await;

    if undraft_decision(route, verified) == UndraftDecision::ParkUnverified {
        return park_unverified(token, trace_id, session_id, db, pr.number).await;
    }

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
            label_auth,
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

/// Re-read the PR body and answer [`pipeline_verified`] on it (mika#2286).
///
/// Fail-closed at every step: an unreachable `gh`, an unparseable payload, an
/// absent body all answer `false`. The cost of being wrong here is one tick of
/// delay for a draft that stays exactly where it is; the cost of the other
/// direction is un-drafting the decision core on a failed read.
async fn fresh_pipeline_verified(pr_number: u64, token: &str, trace_id: &str) -> bool {
    let out = match gh(
        &[
            "pr",
            "view",
            &pr_number.to_string(),
            "--repo",
            DEFAULT_REPO,
            "--json",
            "body",
        ],
        token,
    )
    .await
    {
        Ok(out) => out,
        Err(e) => {
            warn!(pr_number, error = %e, trace_id, "wip_rescue_error");
            return false;
        }
    };
    match serde_json::from_str::<PrBodyEnvelope>(out.trim()) {
        Ok(env) => pipeline_verified(&env.body),
        Err(e) => {
            warn!(pr_number, error = %e, trace_id, "wip_rescue_error");
            false
        }
    }
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

/// Bail-to-human: durable marker, then label + comment + structured event;
/// terminal (AC3).
///
/// The order is load-bearing (mika#2199). The marker is written **first and
/// unconditionally**, because it is the only exclusion that does not travel over
/// the network: everything below it can fail, and the draft is still out of the
/// next scan. The label is the second route, kept because it is where the human
/// who must pick the draft up will look for it.
async fn bail_to_human(
    token: &str,
    label_auth: &LabelWriteToken,
    trace_id: &str,
    session_id: &str,
    db: &AsyncDatabase,
    pr_number: u64,
    reason: String,
) -> ChainOutcome {
    warn!(pr_number, reason = %reason, trace_id, "wip_rescue_bail_to_human");

    mark_bailed(db, session_id, pr_number, trace_id, &reason).await;

    let parked = apply_human_review_label(label_auth, pr_number, trace_id).await;

    // The comment must not promise a re-arm gesture that no longer works.
    // Until mika#2199 the label WAS the eligibility gate, so removing it did
    // re-arm the scan. The gate is now the durable marker, which nothing
    // clears; telling a human to remove the label would send them to do
    // something with no effect — and on a `parked = false` bail the label they
    // are told to remove was never applied in the first place.
    let comment = format!(
        "Auto-resume (wip-rescue, mika#1852) stopped and handed this PR to a \
         human.\n\n**Reason:** `{reason}`\n\nThis draft is now permanently out \
         of the auto-resume scan (mika#2199): the exclusion is a durable marker, \
         not the `{HUMAN_REVIEW_LABEL}` label, so removing the label does not \
         re-arm anything. A human owns this PR from here — finish it by hand, or \
         close it."
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
    ChainOutcome::Bailed { reason, parked }
}

/// Park a DECISION-CORE draft whose pipeline-verification marker does not read
/// `yes` (mika#2286 §4.3). Mirror of [`bail_to_human`] **without** the label and
/// **without** the bail marker — because this is not a bail.
///
/// The order is the same load-bearing one: the durable marker first and
/// unconditionally, so the exclusion holds even if every GitHub call below
/// fails. The marker also guarantees a single pass, hence a single comment — no
/// spam every five minutes.
///
/// Nothing here increments `$.wip_rescue.depth`: no resume happened. The resume
/// that follows the human's `yes` will increment it, which is the accounting
/// this draft deserves.
async fn park_unverified(
    token: &str,
    trace_id: &str,
    session_id: &str,
    db: &AsyncDatabase,
    pr_number: u64,
) -> ChainOutcome {
    info!(pr_number, trace_id, "wip_rescue_parked_unverified");

    write_pr_marker(
        db,
        session_id,
        PARKED_MARKER_TOOL,
        pr_number,
        trace_id,
        PARKED_REASON,
        "wip_rescue parked-unverified exclusion marker (mika#2286)",
    )
    .await;

    // The wording is deliberately not the bail's. The daemon found nothing
    // wrong: it rebased the branch, clippy passed, and the only thing missing is
    // the verification this class requires. Saying "a human owns this PR from
    // here" would send the reader looking for a conflict that does not exist.
    let comment = format!(
        "Auto-resume (wip-rescue, mika#1852) rebased this branch onto `main` and \
         it passes clippy, but substrate-diff classified it **DECISION-CORE** and \
         its pipeline-verification marker is not `yes` — so it stays a draft \
         (mika#2286). To make it reviewable: verify the pipeline, then either \
         mark it Ready for Review yourself, or set \
         `<!-- {PIPELINE_VERIFIED_KEY}: yes -->` in the body and the next scan \
         will un-draft it for you. Merge stays a Vincent hand-merge either way."
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

    // The action row, distinct from the state marker above. Both carry the same
    // string, in different columns and with different meanings: the marker is
    // `tool_name = PARKED_MARKER_TOOL` (one row per PR, read by the eligibility
    // filter), the action is `tool_name = 'wip_rescue'` with this string as
    // `target_key` (one row per park, alongside every other wip_rescue action).
    // Same split as `wip_rescue_bailed` / `wip_rescue_bail_to_human`.
    log_audit(
        db,
        session_id,
        PARKED_MARKER_TOOL,
        pr_number,
        trace_id,
        PARKED_REASON,
    )
    .await;

    ChainOutcome::ParkedUnverified
}

/// Reason string carried by both rows [`park_unverified`] writes. One constant,
/// so the audit row and the marker cannot drift into two spellings of one fact.
const PARKED_REASON: &str = "decision_core_marker_not_yes";

/// Write the durable per-PR bail marker (mika#2199 §4.2).
///
/// See [`write_pr_marker`] for what a failed write costs.
async fn mark_bailed(
    db: &AsyncDatabase,
    session_id: &str,
    pr_number: u64,
    trace_id: &str,
    reason: &str,
) {
    write_pr_marker(
        db,
        session_id,
        BAILED_MARKER_TOOL,
        pr_number,
        trace_id,
        reason,
        "wip_rescue bail exclusion marker (mika#2199)",
    )
    .await;
}

/// Write a durable per-PR exclusion marker (bail, mika#2199 — or park,
/// mika#2286).
///
/// Best-effort like every other audit write in this module — but note what a
/// failure here costs: the marker **is** the exclusion. A bail whose marker
/// could not be written falls back to depending on the label alone, which is the
/// pre-mika#2199 behaviour for that one PR; a park whose marker could not be
/// written has no fallback at all, so the draft is re-elected next tick and
/// re-parked — degraded, bounded by the cost of one tick, and never an un-draft.
///
/// It therefore gets its **own** event name rather than the module's shared
/// `wip_rescue_error`, which is written at ten sites: the one condition that
/// restores the livelock must not be greppably indistinguishable from a
/// `gh pr comment` that timed out. Any hit on `wip_rescue_marker_write_failed`
/// is a PR whose exclusion is not held.
async fn write_pr_marker(
    db: &AsyncDatabase,
    session_id: &str,
    tool_name: &str,
    pr_number: u64,
    trace_id: &str,
    reason: &str,
    note: &str,
) {
    let key = pr_marker_key(pr_number);
    if let Err(e) = db
        .log_audit_event(
            session_id,
            tool_name,
            &key,
            None,
            Some(reason),
            Some(note),
            Some(trace_id),
        )
        .await
    {
        warn!(pr_number, marker = tool_name, error = %e, trace_id, "wip_rescue_marker_write_failed");
    }
}

/// Park the PR on GitHub with [`HUMAN_REVIEW_LABEL`], creating the label when
/// the repository does not declare it. Returns whether the label is now on the
/// PR — the caller no longer swallows this (mika#2199 §4.1).
///
/// Transposition of the one precedent in this repo, `_stamp_pr_origin`
/// (`skills/bundled/_shared/dispatch-lib.sh`, mika#2026), whose header states
/// the pattern: *"a failed edit is retried once behind an idempotent
/// `label create`"*. The failure it answers is measured, not hypothetical —
/// every one of the 17 bails traced on 2026-09-05 died on
/// `gh exit code 1: 'human-review-required' not found`.
///
/// This is the runtime half. The durable half is `.github/labels.yml`
/// (§4.4): without the declaration, `delete-other-labels: true` deletes the
/// label again on the next push touching that file, and this function would be
/// re-creating it forever.
async fn apply_human_review_label(
    label_auth: &LabelWriteToken,
    pr_number: u64,
    trace_id: &str,
) -> bool {
    let number = pr_number.to_string();
    let add_label = [
        "pr",
        "edit",
        &number,
        "--repo",
        DEFAULT_REPO,
        "--add-label",
        HUMAN_REVIEW_LABEL,
    ];

    if gh(&add_label, label_auth.token()).await.is_ok() {
        return true;
    }

    // The likeliest cause is that the repo has no such label. Create it, then
    // retry once. A `create` that fails because the label already exists is
    // indistinguishable here from one that fails for want of permission, so its
    // result is not the verdict — the retry below is.
    let _ = gh(
        &[
            "label",
            "create",
            HUMAN_REVIEW_LABEL,
            "--repo",
            DEFAULT_REPO,
            "--color",
            HUMAN_REVIEW_LABEL_COLOR,
            "--description",
            HUMAN_REVIEW_LABEL_DESC,
        ],
        label_auth.token(),
    )
    .await;

    match gh(&add_label, label_auth.token()).await {
        Ok(_) => true,
        Err(e) => {
            label_auth.report_write_failure("wip_rescue", pr_number, HUMAN_REVIEW_LABEL, &e);
            warn!(pr_number, error = %e, trace_id, "wip_rescue_error");
            false
        }
    }
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
            "number,headRefName,title,labels,createdAt,body",
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
            body: MARKER_NO.into(),
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

    // -----------------------------------------------------------------------
    // mika#2199 — the bail must exclude, and the exclusion must not depend on
    // a label GitHub may not have.
    // -----------------------------------------------------------------------

    const TRACE: &str = "00000000000000000000000000002199";

    /// The two drafts of the founding incident: #2197, the oldest, bailed 28
    /// times between 09:43 and 16:05 without ever being parked, and #2198 which
    /// only got its turn once #2197 was closed by hand.
    const OLDEST: u64 = 2197;
    const NEXT: u64 = 2198;
    const NOW: &str = "2026-09-05T16:00:00Z";
    const OLDEST_CREATED: &str = "2026-09-05T09:00:00Z";
    const NEXT_CREATED: &str = "2026-09-05T12:00:00Z";

    /// The marker as dispatch-lib stamps it at PR creation, and as it stays
    /// until a human edits the body.
    const MARKER_NO: &str = "<!-- rescue-pipeline-verified: no -->";
    /// The marker after the gesture the rescue body asks the operator for.
    const MARKER_YES: &str = "<!-- rescue-pipeline-verified: yes -->";

    fn draft(number: u64, created_at: &str, labels: &[&str]) -> DraftPr {
        draft_with_body(number, created_at, labels, MARKER_NO)
    }

    fn draft_with_body(number: u64, created_at: &str, labels: &[&str], body: &str) -> DraftPr {
        DraftPr {
            number,
            head_ref: format!("wip/{number}"),
            title: format!("wip(x): {number}"),
            labels: labels
                .iter()
                .map(|n| GhLabel {
                    name: (*n).to_string(),
                })
                .collect(),
            created_at: created_at.to_string(),
            body: body.to_string(),
        }
    }

    /// The two exclusion predicates of [`select_eligible`], as they read when
    /// the axis a test is *not* exercising is absent. Named functions rather
    /// than inline `|_| async { false }` so each call site says which axis it
    /// holds constant.
    async fn never_bailed(_pr_number: u64) -> bool {
        false
    }
    async fn never_parked(_pr_number: u64) -> bool {
        false
    }
    /// The mika#2597 axis at rest: a rescue draft, born a draft, nobody put it
    /// back. Every pre-existing test holds this axis constant so its assertions
    /// keep measuring what they were written for.
    async fn never_held(_pr_number: u64) -> HoldVerdict {
        HoldVerdict::NotHeld
    }

    /// The queue of the incident: two `wip-rescue` drafts, neither carrying
    /// `human-review-required` — because the label write never succeeded.
    fn incident_queue() -> Vec<DraftPr> {
        vec![
            draft(OLDEST, OLDEST_CREATED, &[WIP_RESCUE_LABEL]),
            draft(NEXT, NEXT_CREATED, &[WIP_RESCUE_LABEL]),
        ]
    }

    // -- §4.5(a) selection --

    /// AC2. The oldest draft has bailed; the scan must move on to the next one
    /// **on this tick**, not stay on a PR it cannot park.
    #[tokio::test]
    async fn a_bailed_draft_is_not_re_elected_and_the_scan_advances() {
        let selected = select_eligible(
            incident_queue(),
            NOW,
            900,
            |n| async move { n == OLDEST },
            never_parked,
            never_held,
        )
        .await;

        assert_eq!(
            selected.map(|(_, pr)| pr.number),
            Some(NEXT),
            "the marked draft must drop out of the candidate set and the next \
             one be returned in the same pass"
        );
    }

    /// The negative control the whole test rests on: with a predicate that
    /// always answers `false` — which is exactly `main`, where nothing consults
    /// a bail marker because there is none — the same input re-elects the same
    /// draft, tick after tick. If this ever passes on `main`, the test above
    /// measures nothing.
    #[tokio::test]
    async fn without_the_marker_the_same_draft_is_re_elected_forever() {
        for tick in 0..3 {
            let selected = select_eligible(
                incident_queue(),
                NOW,
                900,
                never_bailed,
                never_parked,
                never_held,
            )
            .await;
            assert_eq!(
                selected.map(|(_, pr)| pr.number),
                Some(OLDEST),
                "tick {tick}: this is the livelock — the oldest draft is \
                 returned again and the queue never advances"
            );
        }
    }

    /// Every candidate bailed → nothing to do. The distinction matters: an empty
    /// selection here means the queue is genuinely drained, not blocked.
    #[tokio::test]
    async fn all_bailed_selects_nothing() {
        let selected = select_eligible(
            incident_queue(),
            NOW,
            900,
            |_| async { true },
            never_parked,
            never_held,
        )
        .await;
        assert!(selected.is_none());
    }

    /// The marker is an addition, not a replacement: the label exclusion, the
    /// `wip-rescue` requirement and the age threshold all still hold.
    #[tokio::test]
    async fn the_pre_existing_predicates_are_unchanged() {
        // Already parked on GitHub → excluded even with no marker.
        let parked = vec![draft(
            OLDEST,
            OLDEST_CREATED,
            &[WIP_RESCUE_LABEL, HUMAN_REVIEW_LABEL],
        )];
        assert!(
            select_eligible(parked, NOW, 900, never_bailed, never_parked, never_held)
                .await
                .is_none()
        );

        // Not a wip-rescue draft → never a candidate.
        let unrelated = vec![draft(OLDEST, OLDEST_CREATED, &["p1-important"])];
        assert!(
            select_eligible(unrelated, NOW, 900, never_bailed, never_parked, never_held)
                .await
                .is_none()
        );

        // Younger than the threshold → waits.
        let fresh = vec![draft(NEXT, "2026-09-05T15:59:00Z", &[WIP_RESCUE_LABEL])];
        assert!(
            select_eligible(fresh, NOW, 900, never_bailed, never_parked, never_held)
                .await
                .is_none()
        );
    }

    /// Fail-closed (§4.2): an unreadable audit trail reads as *already bailed*.
    /// A closed database is the cheapest way to make the read fail for real
    /// rather than mock the failure.
    #[tokio::test]
    async fn an_unreadable_audit_trail_excludes_the_draft() {
        let db = crate::db::Database::open_in_memory().unwrap();
        db.conn.execute_batch("DROP TABLE audit_events").unwrap();
        let db = AsyncDatabase::new(db);

        assert!(
            has_bailed_marker(&db, OLDEST, TRACE).await,
            "a PR that has bailed is destined for a human; excluding it wrongly \
             loses nothing, re-attempting it in a loop costs the whole queue"
        );
    }

    // -- §4.5(b) end-to-end replay against a failing `gh` --
    //
    // `run_gh_subprocess` resolves `gh` through `PATH` and `scrub_mika_env_vars`
    // removes only `MIKA_*` and `GH_TOKEN`, so a fake `gh` at the head of `PATH`
    // reproduces the exact production failure. The scenarios are named after
    // `test_stamp_pr_origin.sh` (`nominal`, `needs-label`).
    //
    // The fake must not communicate through any `MIKA_`-prefixed variable — they
    // are purged before the exec — so its tempdir is baked into the script.

    // What `#[serial_test::serial]` does and does not buy here, stated plainly
    // because the obvious reading is wrong: it serialises these tests against
    // *other `#[serial]` tests*, not against the whole binary. Tests without the
    // attribute still run in parallel, so `PATH` is mutated under them. Two
    // residual exposures follow, and both are bounded rather than eliminated:
    //
    //  1. `std::env::set_var` is `unsafe` in edition 2024 precisely because a
    //     concurrent `getenv` is a data race. Nothing in this crate's test set
    //     reads `PATH` on a hot path, and the window is a few milliseconds.
    //  2. During that window a parallel test that spawns `gh` would find the
    //     fake. Only `run_gh_subprocess` resolves `gh`, and its own unit tests
    //     do not spawn — but this is a real property of the harness, not a
    //     thing the attribute rules out.
    //
    // The plan (§4.5) anticipated this: if it ever proves flaky in CI, the
    // §4.5(a) selection tests are the non-negotiable deliverable and these three
    // degrade to a hand-run script.

    struct PathGuard(Option<std::ffi::OsString>);

    impl Drop for PathGuard {
        fn drop(&mut self) {
            // SAFETY: see the module-level note above — bounded, not absent.
            unsafe {
                match self.0.take() {
                    Some(prev) => std::env::set_var("PATH", prev),
                    None => std::env::remove_var("PATH"),
                }
            }
        }
    }

    /// Put `dir` at the head of `PATH` for the lifetime of the returned guard.
    fn prepend_to_path(dir: &Path) -> PathGuard {
        let previous = std::env::var_os("PATH");
        let mut next = std::ffi::OsString::from(dir);
        if let Some(prev) = &previous {
            next.push(":");
            next.push(prev);
        }
        // SAFETY: see the module-level note above — bounded, not absent.
        unsafe { std::env::set_var("PATH", &next) };
        PathGuard(previous)
    }

    fn install_fake_gh(dir: &Path, body: &str) {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join("gh");
        std::fs::write(&path, body).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    /// `needs-label`: `pr edit --add-label` fails with the production error
    /// until the label exists; `label create` creates it.
    fn needs_label_script(dir: &Path) -> String {
        let d = dir.display();
        format!(
            "#!/bin/sh\n\
             echo \"$*\" >> {d}/calls.log\n\
             if [ \"$1 $2\" = \"label create\" ]; then : > {d}/label-exists; exit 0; fi\n\
             if [ \"$1 $2\" = \"pr edit\" ]; then\n\
             \x20 if [ -f {d}/label-exists ]; then exit 0; fi\n\
             \x20 echo \"'{HUMAN_REVIEW_LABEL}' not found\" >&2; exit 1\n\
             fi\n\
             exit 0\n"
        )
    }

    /// `nominal`: the label is already declared, every call succeeds.
    fn nominal_script(dir: &Path) -> String {
        let d = dir.display();
        format!("#!/bin/sh\necho \"$*\" >> {d}/calls.log\nexit 0\n")
    }

    /// Everything GitHub refuses — the token cannot create a label either.
    fn hostile_script(dir: &Path) -> String {
        let d = dir.display();
        format!("#!/bin/sh\necho \"$*\" >> {d}/calls.log\necho boom >&2\nexit 1\n")
    }

    fn calls(dir: &Path) -> String {
        std::fs::read_to_string(dir.join("calls.log")).unwrap_or_default()
    }

    /// Un token d'écriture de label pour les tests de bail.
    ///
    /// Provenance `Pat` : `report_write_failure` ne doit pas émettre
    /// `label_write_app_token_insufficient` sur un refus de `gh` simulé — cet
    /// event nomme une cause réelle de déploiement, pas un scénario de fixture.
    fn test_label_auth() -> LabelWriteToken {
        LabelWriteToken::new(
            "token".to_string(),
            mika_common::label_write::LabelWriteTokenSource::Pat,
        )
    }

    fn bail_db() -> AsyncDatabase {
        let db = crate::db::Database::open_in_memory().unwrap();
        db.create_session("test-session", "mika", "cli").unwrap();
        AsyncDatabase::new(db)
    }

    /// AC1/AC3 `needs-label`: the label is missing, so the daemon creates it and
    /// retries — the sequence `_stamp_pr_origin` (mika#2026) already uses.
    #[tokio::test]
    #[serial_test::serial]
    async fn bail_creates_the_missing_label_then_parks_the_pr() {
        let tmp = tempfile::tempdir().unwrap();
        install_fake_gh(tmp.path(), &needs_label_script(tmp.path()));
        let _path = prepend_to_path(tmp.path());
        let db = bail_db();

        let outcome = bail_to_human(
            "token",
            &test_label_auth(),
            TRACE,
            "test-session",
            &db,
            OLDEST,
            "rebase-conflict-on-main".to_string(),
        )
        .await;

        assert!(
            matches!(outcome, ChainOutcome::Bailed { parked: true, .. }),
            "got {outcome:?} — the retry behind `label create` must park the PR"
        );
        let log = calls(tmp.path());
        assert!(
            log.contains(&format!("label create {HUMAN_REVIEW_LABEL}")),
            "the missing label must be created, not given up on: {log}"
        );
        assert_eq!(
            log.matches("pr edit").count(),
            2,
            "one attempt before the create and one after: {log}"
        );
        assert!(has_bailed_marker(&db, OLDEST, TRACE).await);
    }

    /// `nominal`: when the label already exists the first edit succeeds and the
    /// daemon does not touch the label registry at all.
    #[tokio::test]
    #[serial_test::serial]
    async fn a_declared_label_needs_no_creation() {
        let tmp = tempfile::tempdir().unwrap();
        install_fake_gh(tmp.path(), &nominal_script(tmp.path()));
        let _path = prepend_to_path(tmp.path());
        let db = bail_db();

        let outcome = bail_to_human(
            "token",
            &test_label_auth(),
            TRACE,
            "test-session",
            &db,
            OLDEST,
            "clippy-errors-need-human".to_string(),
        )
        .await;

        assert!(matches!(outcome, ChainOutcome::Bailed { parked: true, .. }));
        let log = calls(tmp.path());
        assert!(!log.contains("label create"), "nothing to create: {log}");
        assert_eq!(log.matches("pr edit").count(), 1, "{log}");
    }

    /// AC2, end to end. GitHub refuses everything — including the label
    /// creation, the risk named in §8 of the plan. The bail is still terminal,
    /// and the next scan elects the *next* draft. This is the replay the ticket
    /// asks for: on `main` this loops, here it advances.
    #[tokio::test]
    #[serial_test::serial]
    async fn a_bail_that_cannot_park_the_pr_still_excludes_it() {
        let tmp = tempfile::tempdir().unwrap();
        install_fake_gh(tmp.path(), &hostile_script(tmp.path()));
        let _path = prepend_to_path(tmp.path());
        let db = bail_db();

        let outcome = bail_to_human(
            "token",
            &test_label_auth(),
            TRACE,
            "test-session",
            &db,
            OLDEST,
            "rebase-conflict-on-main".to_string(),
        )
        .await;

        assert!(
            matches!(outcome, ChainOutcome::Bailed { parked: false, .. }),
            "got {outcome:?} — a failed parking must be reported, not swallowed"
        );
        assert!(
            has_bailed_marker(&db, OLDEST, TRACE).await,
            "the durable marker is written before any GitHub call, precisely so \
             that GitHub failing changes nothing about the exclusion"
        );

        let selected = select_eligible(
            incident_queue(),
            NOW,
            900,
            |n| has_bailed_marker(&db, n, TRACE),
            never_parked,
            never_held,
        )
        .await;
        assert_eq!(
            selected.map(|(_, pr)| pr.number),
            Some(NEXT),
            "the queue must advance even though the label never landed"
        );
    }

    // -- §4.4 label registry (AC4) --

    /// The labels this module writes must be declared in `.github/labels.yml`.
    ///
    /// `label-sync` runs with `delete-other-labels: true`, so a label created by
    /// hand — or by [`apply_human_review_label`] at runtime — is deleted again on
    /// the next push touching that file. This is the third occurrence of the
    /// class on this repo (`dispatch:ssc`, then `operator-review`/`blocked`),
    /// and prose has not stopped it twice; reading the declaration file does.
    ///
    /// The assertions are on the **constants**, never on copied literals: a name
    /// typed out here would be a name nothing writes, and the test would stay
    /// green for ever
    /// (`docs/solutions/best-practices/a-count-assertion-on-an-event-name-nothing-emits-is-always-green-2026-08-31.md`).
    ///
    /// `wip_rescue.rs` handles exactly these two labels and *applies* only one,
    /// so this guard is exhaustive for the module (AC4's "and the others" is the
    /// empty set). The equivalent guard for `auto_pull.rs` remains mika#2127 AC2.
    #[test]
    fn labels_this_module_writes_are_declared_in_labels_yml() {
        let yml = include_str!("../../../.github/labels.yml");
        let declared = |name: &str| yml.contains(&format!("- name: {name}"));

        // Positive control: the guard can see a label that IS declared.
        assert!(
            declared(WIP_RESCUE_LABEL),
            "labels.yml must declare `{WIP_RESCUE_LABEL}`"
        );
        // Negative control: a check that answers `true` for everything proves
        // nothing.
        assert!(
            !declared("a-label-nobody-has-ever-declared"),
            "the guard must be able to detect an undeclared label"
        );

        assert!(
            declared(HUMAN_REVIEW_LABEL),
            "bail_to_human applies `{HUMAN_REVIEW_LABEL}`, which .github/labels.yml \
             does not declare — `delete-other-labels: true` would prune it on the \
             next push touching that file, and every bail would fail exactly as \
             the 17 traced on 2026-09-05 did"
        );
    }

    /// GitHub caps a label description at 100 characters; this repo has hit that
    /// wall twice (mika#2130, mika#2168). The value is used verbatim by
    /// [`apply_human_review_label`], so a too-long one would make the runtime
    /// creation fail on the very path that exists to stop a failure.
    #[test]
    fn the_label_description_fits_githubs_limit() {
        let len = HUMAN_REVIEW_LABEL_DESC.chars().count();
        assert!(len <= 100, "{len} chars, GitHub caps descriptions at 100");
    }

    /// Two writers create this label — [`apply_human_review_label`] at runtime
    /// and `label-sync` from `.github/labels.yml` — and they must agree, or each
    /// push would revert the other's colour and description.
    ///
    /// The assertion is scoped to **this label's own block**, not the whole
    /// file: `d93f0b` is also `calibration-reset`'s colour, so a whole-file
    /// `contains` would stay green through exactly the drift this test exists
    /// to catch.
    #[test]
    fn the_declared_label_matches_the_one_the_daemon_creates() {
        let yml = include_str!("../../../.github/labels.yml");
        let block = yml
            .split("- name: ")
            .find(|entry| entry.starts_with(HUMAN_REVIEW_LABEL))
            .unwrap_or_else(|| {
                panic!("labels.yml declares no `{HUMAN_REVIEW_LABEL}` entry to check")
            });

        assert!(
            block.contains(HUMAN_REVIEW_LABEL_COLOR),
            "the `{HUMAN_REVIEW_LABEL}` entry must carry colour \
             `{HUMAN_REVIEW_LABEL_COLOR}`, the one apply_human_review_label \
             creates it with; entry was:\n{block}"
        );
        assert!(
            block.contains(HUMAN_REVIEW_LABEL_DESC),
            "the `{HUMAN_REVIEW_LABEL}` entry must carry the same description the \
             daemon creates the label with; entry was:\n{block}"
        );
    }

    // -----------------------------------------------------------------------
    // mika#2286 — a DECISION-CORE draft is un-drafted only once verified, and
    // the draft it leaves behind must not eat the queue.
    // -----------------------------------------------------------------------

    /// PR #2285, the rescue of mika#2023: classified DECISION-CORE and
    /// un-drafted by the daemon on 2026-09-10 with its marker still `no`.
    const PARKED_PR: u64 = 2285;
    /// A second draft waiting behind it — the queue #2285 would hold if the park
    /// did not exclude.
    const BEHIND_PR: u64 = 2287;
    /// `wip_rescue_resume_attempt pr=2285 age_secs=1028`.
    const PARK_NOW: &str = "2026-09-10T18:00:01Z";
    /// 1028 s before `PARK_NOW`, so the fixture reproduces the logged age.
    const PARKED_CREATED: &str = "2026-09-10T17:42:53Z";
    const BEHIND_CREATED: &str = "2026-09-10T17:45:00Z";

    /// The two drafts of the incident, both past the 900 s threshold, both
    /// carrying the marker dispatch-lib stamps and nobody has flipped.
    fn park_queue() -> Vec<DraftPr> {
        vec![
            draft_with_body(PARKED_PR, PARKED_CREATED, &[WIP_RESCUE_LABEL], MARKER_NO),
            draft_with_body(BEHIND_PR, BEHIND_CREATED, &[WIP_RESCUE_LABEL], MARKER_NO),
        ]
    }

    /// AC1. Only the literal `yes` verifies; every other shape is unverified.
    #[test]
    fn mika2286_pipeline_verified_reads_only_an_explicit_yes() {
        // The gesture the rescue body asks for, in the forms a human types it.
        assert!(pipeline_verified(MARKER_YES));
        assert!(pipeline_verified("<!-- rescue-pipeline-verified:yes-->"));
        assert!(pipeline_verified(
            "<!--  rescue-pipeline-verified :  Yes  -->"
        ));
        assert!(
            pipeline_verified(&format!(
                "## Auto-rescued PR\n\n{MARKER_YES}\n\nCloses #2023"
            )),
            "the marker is read inside a real body, not on a line of its own"
        );

        // The state dispatch-lib writes, and the state of a body nobody touched.
        assert!(!pipeline_verified(MARKER_NO));
        assert!(!pipeline_verified(""));
        assert!(
            !pipeline_verified("## Auto-rescued PR\n\nCloses #2023"),
            "an absent marker is not verified: every wip-rescue draft is produced \
             by dispatch-lib, which has stamped it since 2026-06-29, so absence \
             means a mangled body — not a PR predating the contract"
        );

        // Anything that is not the literal `yes`.
        assert!(!pipeline_verified(
            "<!-- rescue-pipeline-verified: maybe -->"
        ));
        assert!(!pipeline_verified(
            "<!-- rescue-pipeline-verified: YES SIR -->"
        ));
        assert!(!pipeline_verified("<!-- rescue-pipeline-verified: -->"));

        // Two markers that disagree resolve to the safe side, whichever order.
        assert!(!pipeline_verified(&format!("{MARKER_YES}\n{MARKER_NO}")));
        assert!(!pipeline_verified(&format!("{MARKER_NO}\n{MARKER_YES}")));

        // A mention that is not a marker contributes nothing — it can neither
        // verify nor contradict. This is the shape of the comment
        // `park_unverified` posts, quoted back into a body.
        assert!(
            !pipeline_verified("set rescue-pipeline-verified in the body"),
            "an unterminated mention must not be read as a value"
        );
        assert!(
            pipeline_verified(&format!(
                "{MARKER_YES}\n\nsee `rescue-pipeline-verified` above"
            )),
            "…and must not cancel a well-formed `yes` either"
        );
        assert!(
            pipeline_verified(&format!(
                "set rescue-pipeline-verified: as shown below\n{MARKER_YES}"
            )),
            "a prose mention that happens to carry a colon must not run on to \
             the next comment's `-->` and swallow the real marker behind it"
        );
        assert!(
            !pipeline_verified(&format!(
                "set rescue-pipeline-verified: as shown below\n{MARKER_NO}"
            )),
            "…and skipping it must not turn an unverified body into a verified one"
        );
    }

    /// AC2. The four rows of the decision table, each named.
    #[test]
    fn mika2286_undraft_decision_gates_decision_core_on_the_marker() {
        assert_eq!(
            undraft_decision(UndraftRoute::DecisionCore, false),
            UndraftDecision::ParkUnverified,
            "the hole this ticket closes: PR #2285 was un-drafted here"
        );
        assert_eq!(
            undraft_decision(UndraftRoute::DecisionCore, true),
            UndraftDecision::Undraft,
            "a human who set the marker to `yes` has made the gesture the rescue \
             body asks for; the daemon honours it"
        );
        assert_eq!(
            undraft_decision(UndraftRoute::Mechanical, false),
            UndraftDecision::Undraft,
            "the mechanical auto-path of mika#1852 is deliberately unchanged"
        );
        assert_eq!(
            undraft_decision(UndraftRoute::Mechanical, true),
            UndraftDecision::Undraft
        );
    }

    /// AC4, the replay of #2285. The parked draft drops out of the candidate set
    /// and the one behind it is returned **on the same tick** — the queue moves.
    ///
    /// The negative control is in the same test, term by term: with a predicate
    /// that always answers `false` — which is `main`, where nothing consults a
    /// park marker because there is none — the same input re-elects #2285 tick
    /// after tick. Without it, the assertion above measures nothing.
    #[tokio::test]
    async fn mika2286_a_parked_draft_is_not_re_elected_and_the_scan_advances() {
        let selected = select_eligible(
            park_queue(),
            PARK_NOW,
            900,
            never_bailed,
            |n| async move { n == PARKED_PR },
            never_held,
        )
        .await;
        assert_eq!(
            selected.map(|(_, pr)| pr.number),
            Some(BEHIND_PR),
            "a parked draft must leave the candidate set and the next one be \
             returned in the same pass"
        );

        for tick in 0..3 {
            let selected = select_eligible(
                park_queue(),
                PARK_NOW,
                900,
                never_bailed,
                never_parked,
                never_held,
            )
            .await;
            assert_eq!(
                selected.map(|(_, pr)| pr.number),
                Some(PARKED_PR),
                "tick {tick}: this is the livelock the marker prevents — the \
                 oldest draft is returned again and the queue never advances"
            );
        }
    }

    /// AC4, the other half: the exclusion is re-armable. Both directions live in
    /// one test so neither can pass while the other rots.
    #[tokio::test]
    async fn mika2286_the_yes_marker_re_arms_the_parked_draft() {
        let parked_pred = |n: u64| async move { n == PARKED_PR };

        // Marker still `no` → held back, as above.
        let still_no = vec![draft_with_body(
            PARKED_PR,
            PARKED_CREATED,
            &[WIP_RESCUE_LABEL],
            MARKER_NO,
        )];
        assert!(
            select_eligible(
                still_no,
                PARK_NOW,
                900,
                never_bailed,
                parked_pred,
                never_held
            )
            .await
            .is_none(),
            "the marker alone excludes while the body is unverified"
        );

        // Same PR, same marker row, body now says `yes` → eligible again. No
        // gesture on the audit trail is required: the body is what lifts it.
        let now_yes = vec![draft_with_body(
            PARKED_PR,
            PARKED_CREATED,
            &[WIP_RESCUE_LABEL],
            MARKER_YES,
        )];
        assert_eq!(
            select_eligible(
                now_yes,
                PARK_NOW,
                900,
                never_bailed,
                parked_pred,
                never_held
            )
            .await
            .map(|(_, pr)| pr.number),
            Some(PARKED_PR),
            "`yes` in the body must re-arm the draft without anyone clearing the \
             marker — otherwise the park is a bail wearing another name"
        );
    }

    /// AC4, fail-closed. An unreadable audit trail reads as *already parked* —
    /// the same arbitrage as the bail, reached by a different road: a parked
    /// draft excluded in error stays open, a draft, and commented, and the body
    /// test in front of this read re-elects it the moment a human writes `yes`.
    #[tokio::test]
    async fn mika2286_an_unreadable_audit_trail_parks_the_draft() {
        let db = crate::db::Database::open_in_memory().unwrap();
        db.conn.execute_batch("DROP TABLE audit_events").unwrap();
        let db = AsyncDatabase::new(db);

        assert!(has_parked_marker(&db, PARKED_PR, TRACE).await);
    }

    /// AC3, against a `gh` that refuses everything: the park is durable before
    /// any GitHub call, it is **not** a bail, and it says so.
    #[tokio::test]
    #[serial_test::serial]
    async fn mika2286_parking_is_durable_and_is_not_a_bail() {
        let tmp = tempfile::tempdir().unwrap();
        install_fake_gh(tmp.path(), &hostile_script(tmp.path()));
        let _path = prepend_to_path(tmp.path());
        let db = bail_db();

        let outcome = park_unverified("token", TRACE, "test-session", &db, PARKED_PR).await;

        assert_eq!(outcome, ChainOutcome::ParkedUnverified);
        assert!(
            has_parked_marker(&db, PARKED_PR, TRACE).await,
            "the marker is written before any GitHub call, precisely so that \
             GitHub failing changes nothing about the exclusion"
        );
        assert!(
            !has_bailed_marker(&db, PARKED_PR, TRACE).await,
            "a park is not a bail: writing the bail marker would make the \
             exclusion terminal, and `yes` would never re-arm it"
        );

        let log = calls(tmp.path());
        assert!(
            log.contains("pr comment"),
            "the operator must be told which two gestures free the draft: {log}"
        );
        assert!(
            !log.contains("pr ready"),
            "the whole point is that the draft stays a draft: {log}"
        );
        assert!(
            !log.contains(HUMAN_REVIEW_LABEL),
            "`{HUMAN_REVIEW_LABEL}` says a human owns this PR because something \
             went wrong; nothing did: {log}"
        );
    }

    /// AC3, the comment's content. It must name **both** freeing gestures, and
    /// it must not carry the bail's "a human owns this PR from here" — a reader
    /// who believes that goes looking for a conflict that does not exist.
    #[tokio::test]
    #[serial_test::serial]
    async fn mika2286_the_park_comment_names_both_freeing_gestures() {
        let tmp = tempfile::tempdir().unwrap();
        install_fake_gh(tmp.path(), &nominal_script(tmp.path()));
        let _path = prepend_to_path(tmp.path());
        let db = bail_db();

        park_unverified("token", TRACE, "test-session", &db, PARKED_PR).await;

        let log = calls(tmp.path());
        assert!(
            log.contains(MARKER_YES),
            "the comment must quote the marker verbatim, so the gesture can be \
             copy-pasted rather than reconstructed: {log}"
        );
        assert!(
            log.contains("Ready for Review"),
            "…and name the un-draft gesture too: {log}"
        );
        assert!(
            !log.contains("owns this PR"),
            "the bail's wording must not leak into a park: {log}"
        );
    }

    // =======================================================================
    // mika#2597 — an explicit hold holds against `wip_rescue`.
    // =======================================================================

    /// PR #2589, the rescue of mika#2105: held in draft by the plan's
    /// Fire-Disposition (rebuild +190 %, past the +50 % renunciation threshold),
    /// and un-drafted by this daemon **twice in 35 minutes** on 2026-09-29.
    const HELD_PR: u64 = 2589;
    /// A second draft waiting behind it — the queue #2589 would hold for ever if
    /// the hold lived in `resume_chain` instead of in the selection.
    const BEHIND_HELD_PR: u64 = 2591;
    /// `ReadyForReviewEvent by mika-platform-dev`, second occurrence.
    const HOLD_NOW: &str = "2026-09-29T21:36:55Z";
    const HELD_CREATED: &str = "2026-09-29T19:10:00Z";
    const BEHIND_HELD_CREATED: &str = "2026-09-29T19:20:00Z";

    /// The hold as GitHub reports it: the orchestrator put the PR back at 21:31Z.
    fn held() -> HoldVerdict {
        HoldVerdict::Held {
            since: Some("2026-09-29T21:31:00Z".to_string()),
            actor: Some("samidarko".to_string()),
        }
    }

    fn unreadable() -> HoldVerdict {
        HoldVerdict::Unreadable {
            error: "GitHub GraphQL API error: rate limit exceeded".to_string(),
        }
    }

    /// The two drafts of the incident, both past the 900 s threshold.
    fn hold_queue() -> Vec<DraftPr> {
        vec![
            draft(HELD_PR, HELD_CREATED, &[WIP_RESCUE_LABEL]),
            draft(BEHIND_HELD_PR, BEHIND_HELD_CREATED, &[WIP_RESCUE_LABEL]),
        ]
    }

    /// V1 / AC1 / AC2. A held draft is **not selected** — so it is not rebased,
    /// not pushed, not un-drafted, and its `$.wip_rescue.depth` is not bumped,
    /// all four of which live downstream of this return.
    ///
    /// Reddens on `main`: with no third predicate the draft is returned.
    #[tokio::test]
    async fn mika2597_un_brouillon_tenu_nest_pas_selectionne() {
        let only_held = vec![draft(HELD_PR, HELD_CREATED, &[WIP_RESCUE_LABEL])];
        assert!(
            select_eligible(only_held, HOLD_NOW, 900, never_bailed, never_parked, |_| {
                async { held() }
            })
            .await
            .is_none(),
            "a draft somebody put back into draft is a hold, and the daemon does \
             not touch it — this is the two 2026-09-29 un-drafts of #2589"
        );
    }

    /// V2. The hold must not eat the single per-tick slot: the draft behind it is
    /// returned **on the same tick**. This is the property the placement in
    /// `select_eligible` buys, and which the same predicate in `resume_chain`
    /// would destroy — a hold draft is old by nature, so it stays the oldest
    /// candidate for ever (the mika#2199 livelock shape).
    #[tokio::test]
    async fn mika2597_un_brouillon_tenu_ne_bloque_pas_le_suivant() {
        let selected = select_eligible(
            hold_queue(),
            HOLD_NOW,
            900,
            never_bailed,
            never_parked,
            |n| async move {
                if n == HELD_PR {
                    held()
                } else {
                    HoldVerdict::NotHeld
                }
            },
        )
        .await;
        assert_eq!(
            selected.map(|(_, pr)| pr.number),
            Some(BEHIND_HELD_PR),
            "the held draft must leave the candidate set and the next one be \
             returned in the same pass"
        );
    }

    /// V3, **the negative control, and it is the load-bearing one**. Without it,
    /// "the guard decides" is indistinguishable from "the guard blocks
    /// everything", and the whole mika#1852 mechanism could be dead with every
    /// test green.
    #[tokio::test]
    async fn mika2597_un_brouillon_de_rescue_reste_selectionne() {
        let selected = select_eligible(
            hold_queue(),
            HOLD_NOW,
            900,
            never_bailed,
            never_parked,
            |_| async { HoldVerdict::NotHeld },
        )
        .await;
        assert_eq!(
            selected.map(|(_, pr)| pr.number),
            Some(HELD_PR),
            "a draft born a draft — no ConvertToDraftEvent — is still promoted: \
             that is what wip_rescue exists for (mika#1852)"
        );
    }

    /// V4, fail-closed. *A term one cannot read is never a satisfied term*
    /// (mika#2277), applied to the term « this draft is **not** held ».
    #[tokio::test]
    async fn mika2597_une_timeline_illisible_exclut() {
        let only_held = vec![draft(HELD_PR, HELD_CREATED, &[WIP_RESCUE_LABEL])];
        assert!(
            select_eligible(only_held, HOLD_NOW, 900, never_bailed, never_parked, |_| {
                async { unreadable() }
            })
            .await
            .is_none(),
            "a false « held » makes a rescue draft wait for an operator gesture; \
             a false « not held » un-drafts a PR whose merge is forbidden"
        );
    }

    /// V5. The three exclusions compose without any of them knowing about the
    /// others: three drafts, one per motif, and the fourth — clean on all three
    /// axes — is the one returned.
    #[tokio::test]
    async fn mika2597_les_trois_exclusions_composent() {
        const BAILED: u64 = 2101;
        const PARKED: u64 = 2102;
        const CLEAN: u64 = 2104;

        // Oldest first, so the three excluded ones are consulted before the
        // clean one: the selection really has to walk past all three.
        let queue = vec![
            draft(BAILED, "2026-09-29T10:00:00Z", &[WIP_RESCUE_LABEL]),
            draft(PARKED, "2026-09-29T11:00:00Z", &[WIP_RESCUE_LABEL]),
            draft(HELD_PR, "2026-09-29T12:00:00Z", &[WIP_RESCUE_LABEL]),
            draft(CLEAN, "2026-09-29T13:00:00Z", &[WIP_RESCUE_LABEL]),
        ];

        let selected = select_eligible(
            queue,
            HOLD_NOW,
            900,
            |n| async move { n == BAILED },
            |n| async move { n == PARKED },
            |n| async move {
                if n == HELD_PR {
                    held()
                } else {
                    HoldVerdict::NotHeld
                }
            },
        )
        .await;
        assert_eq!(
            selected.map(|(_, pr)| pr.number),
            Some(CLEAN),
            "bail, park and hold must each exclude their own draft and none of \
             the others"
        );
    }

    /// V6 / AC3. Three states, **two** names — and `NotHeld` writes nothing.
    ///
    /// The two names must stay countable apart: one says the mechanism worked,
    /// the other that `wip_rescue` is frozen by its own fail-closed rule. Merging
    /// them would make an outage read as a success.
    #[tokio::test]
    async fn mika2597_le_verdict_a_trois_etats_et_deux_noms() {
        let db = bail_db();
        let count = async |tool: &str, pr: u64| {
            db.count_recent_audit_events_for_target(
                tool,
                &hold_audit_key(pr, tool),
                "1970-01-01T00:00:00Z",
            )
            .await
            .unwrap()
        };

        report_hold(&db, "test-session", HELD_PR, TRACE, &held()).await;
        assert_eq!(count(HOLD_RESPECTED_TOOL, HELD_PR).await, 1);
        assert_eq!(
            count(HOLD_UNREADABLE_TOOL, HELD_PR).await,
            0,
            "a respected hold must not be counted as an unreadable timeline"
        );

        report_hold(&db, "test-session", BEHIND_HELD_PR, TRACE, &unreadable()).await;
        assert_eq!(count(HOLD_UNREADABLE_TOOL, BEHIND_HELD_PR).await, 1);
        assert_eq!(
            count(HOLD_RESPECTED_TOOL, BEHIND_HELD_PR).await,
            0,
            "an unreadable timeline must not be counted as a hold held"
        );

        // The nominal case is silent: an observability that logs everybody
        // distinguishes nobody (mika#2131 AC7).
        report_hold(&db, "test-session", CLEAN_PR, TRACE, &HoldVerdict::NotHeld).await;
        assert_eq!(count(HOLD_RESPECTED_TOOL, CLEAN_PR).await, 0);
        assert_eq!(count(HOLD_UNREADABLE_TOOL, CLEAN_PR).await, 0);
    }

    /// A draft nobody held, used by V6 and V7 as the silent control.
    const CLEAN_PR: u64 = 2593;

    /// V7 / R5. A hold lasting weeks writes **one** row a day, not 288 — and a
    /// change of motif on the same PR still writes its own row, because the key
    /// carries the motif.
    #[tokio::test]
    async fn mika2597_la_dedup_borne_a_une_ligne_par_24h() {
        let db = bail_db();
        let count = async |tool: &str| {
            db.count_recent_audit_events_for_target(
                tool,
                &hold_audit_key(HELD_PR, tool),
                "1970-01-01T00:00:00Z",
            )
            .await
            .unwrap()
        };

        for _tick in 0..3 {
            report_hold(&db, "test-session", HELD_PR, TRACE, &held()).await;
        }
        assert_eq!(
            count(HOLD_RESPECTED_TOOL).await,
            1,
            "three ticks, one row — a hold is a state, not an event (mika#2131)"
        );

        // Same PR, different motif → its own row. Without the motif in the key,
        // the first window would swallow it and the two populations would stop
        // being subtractable.
        report_hold(&db, "test-session", HELD_PR, TRACE, &unreadable()).await;
        assert_eq!(count(HOLD_UNREADABLE_TOOL).await, 1);
        assert_eq!(count(HOLD_RESPECTED_TOOL).await, 1);
    }

    /// Sites allowed to hold a `gh pr ready` call besides [`resume_chain`]'s
    /// step 7. **Shipped empty and pinned empty** — when the scan below fires,
    /// route the new site through the guard; do not add a line here (doctrine
    /// mika#2201; an allowlist born empty is a drawer to drop the next
    /// infraction in, mika#2323).
    const UNDRAFT_SITES_ALLOWED: &[&str] = &[];

    #[test]
    fn mika2597_lallowlist_dundraft_est_livree_vide() {
        assert!(
            UNDRAFT_SITES_ALLOWED.is_empty(),
            "an un-draft site is a site to route through the hold guard, never \
             one to exempt"
        );
    }

    /// V8 / R6. The un-draft and the selection cannot divorce: exactly **one**
    /// production site issues `gh pr ready`, and it is the one this module's
    /// selection gates.
    ///
    /// A source scan, because no behavioural test can see this class: a second
    /// un-draft site makes **no** existing decision wrong the day it is written
    /// — every assertion stays green while a path un-drafts without ever
    /// consulting the hold.
    ///
    /// The needle is searched on **whitespace-normalised** source so rustfmt's
    /// line breaks cannot hide it. `validate_pr_ready_undraft_scope`
    /// (`builtin_handlers.rs`, mika#1682) inspects an argv and un-drafts nothing
    /// — it compares `verb == "ready"` against a variable and is out of the
    /// population by shape, not by exemption.
    #[test]
    fn mika2597_un_seul_site_dundraft_en_production() {
        let src_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let this_module = src_root.join("wip_rescue.rs");
        let needle = format!("{}{}", "\"pr\",", "\"ready\"");

        let mut sites: Vec<(String, usize)> = Vec::new();
        let mut scanned = 0usize;
        let mut stack = vec![src_root.clone()];
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir).expect("read of src/").flatten() {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                    continue;
                }
                if path.extension().is_none_or(|e| e != "rs")
                    || crate::source_scan::is_test_source_path(&path)
                {
                    continue;
                }
                let content = std::fs::read_to_string(&path).expect("read of a source file");
                scanned += 1;
                // Production half only: a fixture argv in a `#[cfg(test)]` block
                // is not a site that un-drafts anything.
                let production = match content.find("#[cfg(test)]") {
                    Some(i) => &content[..i],
                    None => &content[..],
                };
                let normalised: String = crate::source_scan::strip_comment_lines(production)
                    .split_whitespace()
                    .collect();
                let hits = normalised.matches(&needle).count();
                if hits > 0 {
                    let name = path.strip_prefix(&src_root).unwrap_or(&path);
                    sites.push((name.display().to_string(), hits));
                }
            }
        }

        assert!(scanned > 0, "the guard scanned no file at all");
        // Anti-vacuity: a scan aiming at a dead needle reads exactly like a clean
        // tree (mika#2205).
        let total: usize = sites.iter().map(|(_, n)| n).sum();
        assert_eq!(
            total, 1,
            "exactly one production site may issue `gh pr ready`; found {sites:?}"
        );

        let offenders: Vec<_> = sites
            .iter()
            .filter(|(name, _)| {
                name != &this_module
                    .strip_prefix(&src_root)
                    .unwrap()
                    .display()
                    .to_string()
                    && !UNDRAFT_SITES_ALLOWED.contains(&name.as_str())
            })
            .collect();
        assert!(
            offenders.is_empty(),
            "mika#2597 — the un-draft lives in `wip_rescue.rs`, behind the hold \
             gate. Route the new site through it; do not allowlist it.\n{offenders:?}"
        );
    }

    /// V9. `--draft` in the listing is the **premise** of the simple predicate:
    /// it is what makes "carries a ConvertToDraftEvent" equivalent to "was put
    /// back into draft" without comparing a single instant (see [`HoldVerdict`]).
    ///
    /// Not decorative: a future editor widening the population by dropping the
    /// filter makes a test redden instead of making the predicate silently false.
    #[test]
    fn mika2597_le_listing_filtre_toujours_draft() {
        let src = include_str!("wip_rescue.rs");
        let (_, body) = crate::source_scan::fn_bodies(src)
            .into_iter()
            .find(|(name, _)| name == "list_wip_rescue_drafts")
            .expect("list_wip_rescue_drafts must exist — it is the listing");

        assert!(
            body.contains("\"--draft\""),
            "the listing must keep `--draft`: it is what makes the presence of a \
             ConvertToDraftEvent equivalent to a hold, with no instant compared"
        );
        assert!(
            body.contains("\"open\""),
            "…and `--state open`, the other half of the same premise"
        );
    }
}
