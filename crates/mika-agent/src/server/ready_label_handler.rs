//! Structural ready-label dispatch handler (mika#1384).
//!
//! Intercepts `[GitHub] Issue labeled ready on <repo>#<num>` webhook events
//! **before** the LLM turn and PRE-RESOLVES all dispatch parameters mika-dev
//! has historically failed to call `run_claude_pilot[_groom]` for. Returns a
//! prescriptive pre-digest naming the exact tool, args, and pre-created
//! `task_id` — eliminating LLM ambiguity.
//!
//! This is the Option Α-pragmatic implementation per mika#1384's 2026-06-06
//! routing call ("structural dispatch as a property of the trigger, not the
//! LLM's call"). The engine pre-creates the task, fetches the issue body, and
//! resolves target skill / dispatch class before the LLM turn fires. The LLM
//! still owns the final `run_claude_pilot*` tool call — empirical validation
//! against the n≥5 stuck-dispatch cases (2026-06-26) measures whether
//! prescriptive pre-digest is sufficient or if engine-side subprocess spawn is
//! required (deferred to a follow-up if the pre-digest path doesn't bind).
//!
//! Composes with the existing `webhook_ready_label_dispatch` INTENT_GUARDS
//! entry: this handler runs **before** the LLM turn; the guard runs **after**
//! the LLM turn. Two layers, two failure modes.

use std::path::Path;
use std::sync::Arc;

use tracing::{info, warn};

use crate::async_db::AsyncDatabase;
use crate::messaging::MessageSender;
use crate::skills::SkillRegistry;
use crate::skills::manifest::ToolHandler;
use crate::task_state::tasks::NewTask;
use crate::tools::pr_merge_with_gate::run_gh_subprocess;
use crate::webhook_dispatch::READY_LABEL_DISPATCH_MARKER;

use super::verdict_handler::VerdictAction;

/// Parsed location from a ready-label marker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ReadyLabelLocation {
    /// `owner/repo` form when the marker carries an owner prefix, otherwise the
    /// bare repo name (e.g. `mika`). The latter is supported for backwards
    /// compat with gateway formats that emit short references.
    pub repo_ref: String,
    pub number: u64,
}

impl ReadyLabelLocation {
    /// Returns the fully-qualified `owner/repo` form, applying the
    /// `senara-solutions` default owner when the marker omits it.
    pub fn owner_repo(&self) -> String {
        // Delegates so the defaulting rule lives in exactly one place — the same
        // module that owns the dispatchable-repo allowlist (mika#2046). Two
        // copies of "what does a bare `mika#N` mean" is how the handler and the
        // tool-boundary gate would drift apart.
        crate::webhook_dispatch::normalize_owner_repo(&self.repo_ref)
    }

    /// Returns the bare repo basename (e.g. `mika`), stripping any `owner/`
    /// prefix from `repo_ref`.
    ///
    /// This is the form the dispatch tool schemas document and dispatch-lib's
    /// worktree-setup parser requires for the dispatch `prompt` argument —
    /// distinct from [`owner_repo`](Self::owner_repo), which is for `gh` calls,
    /// task labels, and display. Emitting the owner-qualified form as a dispatch
    /// prompt fails dispatch-lib's `^[a-zA-Z0-9_-]+#[0-9]+$` parse, silently
    /// routing the dispatch into no-worktree free-text mode (mika#1593).
    pub fn repo_name(&self) -> String {
        match self.repo_ref.rsplit_once('/') {
            Some((_, name)) => name.to_string(),
            None => self.repo_ref.clone(),
        }
    }
}

/// Exit gate of the ready-label handler — which of the fifteen ways out of
/// [`try_handle_ready_label_dispatch_with_fetcher`] was taken (mika#2323).
///
/// # WIRE FORMAT
///
/// These values land in `audit_events.after_value` and operators `GROUP BY`
/// them. Two spellings of one gate would split a population without saying so —
/// the same reasoning, and the same guard shape, as mika#2131's
/// `FILTER_*` names. Renaming one is a dated breaking change to be written down
/// in `CLAUDE.md`, never a silent test update.
///
/// # Why this type exists
///
/// mika#2323 reported that a hand-applied `ready` label produced no dispatch,
/// and read the absence of `ready_label_engine_dispatched` as evidence of an
/// actor filter. There is no actor filter (see the module-level note on
/// `parse_event_actor`) — but that absence was compatible with **fourteen**
/// distinct causes plus four upstream losses that write nothing at all, and one
/// of the fourteen was entirely mute. The measured defect is not a filter; it is
/// that a non-dispatch could not be attributed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ReadyLabelGate {
    /// The text is not a ready-label marker.
    ///
    /// **Never emitted**, deliberately: the wrapper returns before the entry
    /// line. Every message on the `github` channel reaches this handler, so
    /// logging here would write a line per message — an observability that
    /// records everyone distinguishes no one (mika#2131 AC7). It is a variant
    /// rather than an absence so the vocabulary enumerates every way out.
    NotAMarker,
    /// Marker recognized, `<repo>#<n>` unparseable.
    ///
    /// Also not carried by the entry/outcome pair: without a location there is
    /// no `target_key` to key an audit row on, and inventing one would be worse
    /// than the existing `ready_label_parse_failed` WARN, which already covers
    /// this case by name.
    ParseFailed,
    /// Gate 2b — the repository is not in `DISPATCHABLE_REPOS` (mika#2046).
    RepoNotDispatchable,
    /// Gate 2c — a pilot is still running for this issue (mika#2279).
    PilotInFlight,
    /// Gate 2d — the host egress relay is down, so no pilot can leave contained
    /// (mika#2049).
    EgressRelayDown,
    /// Step 3 — no GitHub token resolved.
    NoToken,
    /// Step 4 — `gh issue view` failed.
    BodyFetchFailed,
    /// Gate 4b — the issue belongs to another dispatch seat (mika#2084).
    SeatMismatch,
    /// Gate 4c — an operator is holding the ticket (mika#2263).
    OperatorHeld,
    /// Step 7 — the parent tracking row could not be pre-created.
    TaskCreateFailed,
    /// Step 9a — the dispatch tool is not in the `SkillRegistry`.
    ToolNotFound,
    /// Step 9b — the dispatch tool is not a long-running exec handler.
    ToolNotLongRunning,
    /// Step 9d — `validate_dispatch_readiness` refused.
    DispatchReadinessFailed,
    /// Step 9e — the callback child could not be created.
    CallbackCreateFailed,
    /// Step 9f — the handler script is missing on disk.
    HandlerNotFound,
    /// Step 9i — the dispatch subprocess was spawned.
    Dispatched,
}

impl ReadyLabelGate {
    /// The wire name. An exhaustive `match`, never a `_ =>` arm — that is what
    /// makes "a new exit cannot stay anonymous" a compile error rather than a
    /// review convention (mika#2323 R3/AC4).
    pub(crate) fn wire_name(self) -> &'static str {
        match self {
            Self::NotAMarker => "not_a_marker",
            Self::ParseFailed => "parse_failed",
            Self::RepoNotDispatchable => "repo_not_dispatchable",
            Self::PilotInFlight => "pilot_in_flight",
            // mika#2049 — the SAME value as `auto_pull`'s `FILTER_EGRESS_DOWN`,
            // deliberately: one cause, two audit surfaces. Pinned on both sides
            // by `auto_pull::tests::mika2131_filter_names_are_a_wire_format`.
            Self::EgressRelayDown => "egress_relay_down",
            Self::NoToken => "no_token",
            Self::BodyFetchFailed => "body_fetch_failed",
            Self::SeatMismatch => "seat_mismatch",
            Self::OperatorHeld => "operator_held",
            Self::TaskCreateFailed => "task_create_failed",
            Self::ToolNotFound => "tool_not_found",
            Self::ToolNotLongRunning => "tool_not_long_running",
            Self::DispatchReadinessFailed => "dispatch_readiness_failed",
            Self::CallbackCreateFailed => "callback_create_failed",
            Self::HandlerNotFound => "handler_not_found",
            Self::Dispatched => "dispatched",
        }
    }
}

/// The two exits that precede a usable location: named, and deliberately
/// **not** reported (mika#2323).
///
/// Taking the gate as an argument rather than writing it in a comment is what
/// keeps the vocabulary total — every way out of this handler has a name, and
/// these two are marked as the ones that write nothing, in code. The assertion
/// is what stops a later exit being quietly routed through here: any gate other
/// than these two owes the operator an entry and an outcome line.
fn unreported_exit(gate: ReadyLabelGate) -> VerdictAction {
    debug_assert!(
        matches!(
            gate,
            ReadyLabelGate::NotAMarker | ReadyLabelGate::ParseFailed
        ),
        "only the two pre-location exits are unreported; {gate:?} must go through \
         emit_ready_label_outcome"
    );
    VerdictAction::Passthrough { enrichment: None }
}

/// The coarse disposition of a [`VerdictAction`], for the outcome line's
/// `action` field. Complements `gate`: `gate` says *which* exit, `action` says
/// what the caller does with it.
fn action_label(action: &VerdictAction) -> &'static str {
    match action {
        VerdictAction::Dispatched { .. } => "dispatched",
        VerdictAction::Handled { .. } => "handled",
        VerdictAction::Passthrough { .. } => "passthrough",
    }
}

/// Read the GitHub identity that applied the label, when the gateway supplied
/// one (mika#2323).
///
/// # This value decides NOTHING
///
/// It is a log field and an audit field. **No refusal predicate in this module
/// reads it**, and the source scan `mika2323_no_gate_predicate_reads_the_actor`
/// refuses one. Wiring it into a gate would create precisely the actor filter
/// mika#2323 set out to find and did not: `route_event("issues",
/// Some("labeled"))` returns `Some("mika-dev")` with no condition on `sender`,
/// and the gateway records that non-implementation as a decision in prose
/// (`mika-gateway/src/github.rs`, just above the routing step). Introducing one
/// here would be a filtering policy nobody has taken, which that ticket's
/// out-of-scope section names explicitly.
///
/// # Tolerant by construction
///
/// A missing line, a malformed one, an empty login, an old gateway serving a
/// new agent (or the reverse) all yield `None` — never an error, never a
/// refusal. The line is matched on the **last** line only, which is where the
/// producer appends it.
pub(crate) fn parse_event_actor(text: &str) -> Option<&str> {
    let login = text
        .lines()
        .next_back()?
        .strip_prefix(mika_common::github_event_format::LABELED_BY_LINE_PREFIX)?
        .trim();
    (!login.is_empty()).then_some(login)
}

/// Entry line — a `labeled ready` event was received, parsed, and is about to
/// be decided (mika#2323 R1).
///
/// Emitted **after** the marker match and a successful parse, never before: see
/// [`ReadyLabelGate::NotAMarker`] and [`ReadyLabelGate::ParseFailed`] for why
/// each of those two exits stays out of this pair.
///
/// Its absence is what the operator could not read before: nothing
/// distinguished "the event never arrived" from "it arrived and was refused".
fn emit_ready_label_received(location: &ReadyLabelLocation, actor: Option<&str>, trace_id: &str) {
    info!(
        event = "ready_label_received",
        repo = %location.owner_repo(),
        num = location.number,
        actor = actor.unwrap_or("<unknown>"),
        trace_id,
        "ready_label_handler: `labeled ready` event received — deciding"
    );
}

/// Outcome line + audit row — which gate decided, and what the caller gets
/// (mika#2323 R2).
///
/// The audit row is what turns "why was this ticket never dispatched?" into one
/// SQL query instead of a grep over nineteen gigabytes.
///
/// # No deduplication, and that is reasoned
///
/// mika#2131 had to deduplicate because one `auto_pull` tick classifies a
/// hundred tickets every ten minutes. Here the population is a `labeled ready`
/// event — a few dozen a day at most — and each one is a distinct dated fact the
/// operator wants to **count**. Deduplicating would erase the very measurement
/// the ticket asked for ("how many times was this ticket triggered?").
///
/// # Non-fatal
///
/// A failed audit write logs a WARN and changes no dispatch decision — the same
/// contract as the four pre-existing gate audit writes, which this does not
/// replace (mika#2323 R6).
async fn emit_ready_label_outcome(
    db: &AsyncDatabase,
    session_id: &str,
    trace_id: &str,
    location: &ReadyLabelLocation,
    gate: ReadyLabelGate,
    action: &VerdictAction,
    actor: Option<&str>,
) {
    let owner_repo = location.owner_repo();
    let gate_name = gate.wire_name();
    let action_name = action_label(action);
    info!(
        event = "ready_label_outcome",
        repo = %owner_repo,
        num = location.number,
        gate = gate_name,
        action = action_name,
        actor = actor.unwrap_or("<unknown>"),
        trace_id,
        "ready_label_handler: `labeled ready` event decided"
    );

    if let Err(e) = db
        .log_audit_event(
            session_id,
            "ready_label_outcome",
            &format!("{}#{}", owner_repo, location.number),
            None,
            Some(gate_name),
            Some(&format!(
                "repo={} number={} gate={} action={} actor={}",
                owner_repo,
                location.number,
                gate_name,
                action_name,
                actor.unwrap_or("<unknown>")
            )),
            Some(trace_id),
        )
        .await
    {
        warn!(
            event = "ready_label_audit_log_failed",
            repo = %owner_repo,
            num = location.number,
            gate = gate_name,
            error = %e,
            "ready_label_handler: failed to write outcome audit event (non-fatal)"
        );
    }
}

/// Attempt to handle a `[GitHub] Issue labeled ready on …` webhook structurally
/// before the LLM turn.
///
/// Returns `VerdictAction::Dispatched` when the engine spawned the dispatch
/// subprocess directly (mika#1572 — the LLM has no decision left to make).
/// Returns `VerdictAction::Handled` (the #1571 prescriptive pre-digest) when
/// engine-side dispatch is attempted but fails any precondition (tool not in the
/// registry, dispatch-readiness rejection, handler missing) — Resolution F3's
/// degraded path. Returns `VerdictAction::Passthrough` when the event is not a
/// ready-label marker, when parsing fails, or when a required precondition
/// (github token, issue body fetch, task pre-create) cannot be satisfied.
#[allow(clippy::too_many_arguments)]
pub async fn try_handle_ready_label_dispatch(
    text: &str,
    db: &AsyncDatabase,
    github_token: Option<&str>,
    message_sender: Option<&Arc<dyn MessageSender>>,
    session_id: &str,
    trace_id: &str,
    skills: &SkillRegistry,
    global_home_dir: &Path,
) -> VerdictAction {
    try_handle_ready_label_dispatch_with_fetcher(
        text,
        db,
        github_token,
        message_sender,
        session_id,
        trace_id,
        skills,
        global_home_dir,
        |owner_repo, number, token| async move {
            fetch_issue_body_and_labels_via_gh(&owner_repo, number, &token).await
        },
    )
    .await
}

/// [`try_handle_ready_label_dispatch`] with the issue fetch injected.
///
/// The seam exists for one reason: the refusal gates in this handler are
/// defined by what they do BEFORE the step-7 pre-create — "zero task created"
/// is the property, and no test can observe it while the only way in runs `gh
/// issue view` against the real GitHub. Production always passes
/// [`fetch_issue_body_and_labels_via_gh`]; tests pass the labels they want to
/// gate on.
///
/// `fetch_issue` receives `(owner_repo, number, token)` and yields
/// `(body, labels)`.
///
/// # Shape: a thin wrapper around `…_inner` (mika#2323)
///
/// This function owns the two exits that precede a usable location — the mute
/// non-marker return and the parse failure — then emits the entry line, calls
/// `…_inner`, and emits the outcome line naming the gate that decided. The
/// thirteen remaining exits therefore cannot forget to report themselves,
/// because they do not report themselves: they return a
/// [`ReadyLabelGate`] and the wrapper writes the line.
///
/// This is the "single reader" form the house already applies to
/// `grooming_marker` (mika#2158) and `live_pilot` (mika#2279), and for the same
/// reason: fifteen scattered emission sites would drift, exactly as the grooming
/// regex drifted for months while promotion and dispatch routing answered the
/// same question differently.
#[allow(clippy::too_many_arguments)]
pub async fn try_handle_ready_label_dispatch_with_fetcher<F, Fut>(
    text: &str,
    db: &AsyncDatabase,
    github_token: Option<&str>,
    message_sender: Option<&Arc<dyn MessageSender>>,
    session_id: &str,
    trace_id: &str,
    skills: &SkillRegistry,
    // The **global** home, for the mika#2049 egress-relay gate. Passed rather
    // than resolved here: the stamp is written by the dispatch child under
    // `$HOME/.mika`, and the engine must read it under the same home its own
    // `global_home_dir` resolves — see `pilot_egress_stamp`'s module doc on what
    // a divergence between the two costs.
    global_home_dir: &Path,
    fetch_issue: F,
) -> VerdictAction
where
    F: FnOnce(String, u64, String) -> Fut,
    Fut: std::future::Future<Output = Result<(String, Vec<String>), String>>,
{
    // 1. Early-return for non-ready-label messages. Cheapest predicate, and
    //    deliberately silent — see `ReadyLabelGate::NotAMarker`.
    if !text.starts_with(READY_LABEL_DISPATCH_MARKER) {
        return unreported_exit(ReadyLabelGate::NotAMarker);
    }

    // 2. Parse `<repo>#<num>` from the marker text. On parse failure, pass
    //    through and let the existing INTENT_GUARDS path log the issue. No
    //    entry/outcome pair here: without a location there is no audit
    //    `target_key`, and `ready_label_parse_failed` already names this case.
    let location = match parse_ready_label_location(text) {
        Some(loc) => loc,
        None => {
            warn!(
                event = "ready_label_parse_failed",
                text_excerpt = %text.chars().take(120).collect::<String>(),
                "ready_label_handler: could not parse <repo>#<n> from marker — passthrough"
            );
            return unreported_exit(ReadyLabelGate::ParseFailed);
        }
    };

    let actor = parse_event_actor(text);
    emit_ready_label_received(&location, actor, trace_id);

    let (action, gate) = try_handle_ready_label_dispatch_inner(
        &location,
        text,
        db,
        github_token,
        message_sender,
        session_id,
        trace_id,
        skills,
        global_home_dir,
        fetch_issue,
    )
    .await;

    emit_ready_label_outcome(db, session_id, trace_id, &location, gate, &action, actor).await;

    action
}

/// The decision body of the ready-label handler: everything from the repository
/// allowlist to the engine-side spawn.
///
/// Returns the action **and the gate that produced it**, so the wrapper above
/// can attribute a non-dispatch without this function knowing how attribution
/// is reported (mika#2323).
///
/// `location` is already parsed — the two exits that precede it belong to the
/// wrapper.
#[allow(clippy::too_many_arguments)]
async fn try_handle_ready_label_dispatch_inner<F, Fut>(
    location: &ReadyLabelLocation,
    text: &str,
    db: &AsyncDatabase,
    github_token: Option<&str>,
    _message_sender: Option<&Arc<dyn MessageSender>>,
    session_id: &str,
    trace_id: &str,
    skills: &SkillRegistry,
    // mika#2049 — see the wrapper's note on why this is passed, not resolved.
    global_home_dir: &Path,
    fetch_issue: F,
) -> (VerdictAction, ReadyLabelGate)
where
    F: FnOnce(String, u64, String) -> Fut,
    Fut: std::future::Future<Output = Result<(String, Vec<String>), String>>,
{
    // 2b. Repository allowlist (mika#2046). The earliest point at which the
    //     target repository is known, and deliberately ahead of every side
    //     effect: no task is pre-created, no `gh issue view` subprocess runs.
    //
    //     The refusal returns `Handled`, never `Passthrough`. `Passthrough`
    //     would leave `req.text` as the raw ready-label marker, which is exactly
    //     what `webhook_ready_label_dispatch`'s trigger matches — the guard
    //     would then re-prompt the LLM until it called `run_claude_pilot`,
    //     dispatching the very repository this gate just refused. The pre-digest
    //     below opens with `<ready_label_handler>` for the same reason the
    //     prescriptive and post-dispatch digests do: it does not match the
    //     trigger, so the guard composes by construction.
    if !crate::webhook_dispatch::is_dispatchable_repo(&location.repo_ref) {
        let owner_repo = location.owner_repo();
        let allowed = crate::webhook_dispatch::dispatchable_repos_display();
        warn!(
            event = "ready_label_repo_not_dispatchable",
            repo = %owner_repo,
            num = location.number,
            allowed = %allowed,
            "ready_label_handler: `ready` label on a repository the loop may not \
             dispatch into — refused before task creation"
        );

        // Operator-visible record. No `task_id` exists yet by construction —
        // refusing before the pre-create is the point — so the audit target is
        // the issue reference itself.
        if let Err(e) = db
            .log_audit_event(
                session_id,
                "ready_label_repo_not_dispatchable",
                &format!("{}#{}", owner_repo, location.number),
                None,
                Some("dispatch_refused"),
                Some(&format!(
                    "repo={} number={} refused=not_in_dispatchable_repos allowed={}",
                    owner_repo, location.number, allowed
                )),
                Some(trace_id),
            )
            .await
        {
            warn!(
                event = "ready_label_audit_log_failed",
                repo = %owner_repo,
                num = location.number,
                error = %e,
                "ready_label_handler: failed to write refusal audit event (non-fatal)"
            );
        }

        return (
            VerdictAction::Handled {
                pre_digest: format_repo_not_dispatchable_pre_digest(location, &allowed),
            },
            ReadyLabelGate::RepoNotDispatchable,
        );
    }

    // The canonical issue URL. Resolved here rather than at step 7 because the
    // live-pilot gate below is keyed on it — and because it depends on nothing
    // but the marker, which is what lets that gate run before any round trip.
    let issue_url = format!(
        "https://github.com/{}/issues/{}",
        location.owner_repo(),
        location.number
    );

    // 2c. Live-pilot gate (mika#2279). A `labeled ready` event for a ticket
    //     whose pilot is still running is a NO-OP: no task, no supersession, no
    //     kill, no deferred dispatch.
    //
    //     Measured on #2276, 2026-09-10: a second `labeled ready` landed 31
    //     seconds after the dispatch, superseded the parent — which since
    //     mika#2335 also KILLS the running pilot — then failed the readiness
    //     check and registered a duplicate deferred dispatch. `auto_pull` then
    //     read the cancelled parent as "nothing in flight" and re-drove the
    //     label, closing a loop that turned every ~20 minutes with no human in
    //     it. The supersede's kill is the correct contract for a dispatch that
    //     is *new*; a replayed `labeled` event is not a new dispatch, so the
    //     missing guard belongs on the trigger, not in the supersede (putting it
    //     there would undo mika#2335).
    //
    //     Placement is reasoned, not inherited. **Before step 3**, so it costs
    //     no `gh issue view` — the predicate reads the issue's URL, never its
    //     body, and the measured loop was paying one round trip every 20 minutes
    //     for nothing. **Before 6b and 7**, which is the property that matters
    //     and that the three neighbouring gates already state: zero task
    //     created, zero process killed, zero deferred dispatch. The step-9d
    //     deferral disappears because step 9d is never reached, not because an
    //     exception was added to the deferral registry.
    //
    //     Ordering against 4c (operator-held) is a deliberate choice: a ticket
    //     both in flight and operator-held is refused as "in flight". Both
    //     refusals create zero tasks, only the event name differs, and the
    //     cheaper one wins. Named so it does not read later as an oversight.
    //
    //     Refusal returns `Handled`, never `Passthrough` — for the fourth time
    //     in this function and for the reason written at each of the other
    //     three: `Passthrough` leaves `req.text` on the ready-label marker,
    //     which is exactly what `webhook_ready_label_dispatch` triggers on, and
    //     the guard would re-prompt the LLM until it dispatched the ticket this
    //     gate just refused.
    //
    //     Fail-safe: only `Alive` refuses. A dead pilot, an unreadable
    //     `process_start_time` or a DB error yield `None` / `Unreadable`, and
    //     the dispatch proceeds exactly as it did before this gate existed —
    //     see `live_pilot`'s module doc for why the asymmetry leans this way.
    if let crate::live_pilot::LivePilotVerdict::Alive {
        child_task_id,
        parent_task_id,
        pid,
    } = crate::live_pilot::live_pilot_for_issue(db, &issue_url).await
    {
        let owner_repo = location.owner_repo();
        // INFO, not WARN: a `ready` event arriving on a ticket already in flight
        // is a nominal consequence of how the feeder and the webhook path
        // compose. What would be anomalous is a sustained stream of these on one
        // ticket — that is a producer still beating the label, and it is this
        // line that names it.
        info!(
            event = "ready_label_pilot_in_flight",
            repo = %owner_repo,
            num = location.number,
            child_task_id = %child_task_id,
            parent_task_id = %parent_task_id,
            pid,
            "ready_label_handler: `ready` event on a ticket whose pilot is still \
             running — no-op before task creation, supersession and dispatch"
        );

        // Operator-visible record. As at the three gates around it, no `task_id`
        // exists yet by construction, so the audit target is the issue reference:
        // `SELECT target_key, count(*) … WHERE tool_name =
        // 'ready_label_pilot_in_flight' GROUP BY 1` answers "how many times was
        // this ticket re-triggered during its own dispatch?" directly.
        if let Err(e) = db
            .log_audit_event(
                session_id,
                "ready_label_pilot_in_flight",
                &format!("{}#{}", owner_repo, location.number),
                None,
                Some("dispatch_refused"),
                Some(&format!(
                    "repo={} number={} refused=live_pilot pid={} child_task_id={} \
                     parent_task_id={}",
                    owner_repo, location.number, pid, child_task_id, parent_task_id
                )),
                Some(trace_id),
            )
            .await
        {
            warn!(
                event = "ready_label_audit_log_failed",
                repo = %owner_repo,
                num = location.number,
                error = %e,
                "ready_label_handler: failed to write live-pilot refusal audit event \
                 (non-fatal)"
            );
        }

        return (
            VerdictAction::Handled {
                pre_digest: format_pilot_in_flight_pre_digest(
                    location,
                    pid,
                    &child_task_id,
                    &parent_task_id,
                ),
            },
            ReadyLabelGate::PilotInFlight,
        );
    }

    // 2d. Egress-relay gate (mika#2049). The host egress relay is down, so
    //     `dispatch-lib` would refuse this launch anyway (fail-closed since the
    //     operator decision of 2026-09-20). Refusing here spends no token, no
    //     `gh issue view`, creates no tracking row and queues no deferred
    //     dispatch — the same property the three gates above state.
    //
    //     THIS GATE PROTECTS NOTHING, and saying so is what keeps it honest. The
    //     protection is the shell guard, which probes the socket on every
    //     dispatch and reads no persistent state. This is an economy: it keeps a
    //     relay outage from burning tickets' re-drive budget, which is what
    //     turns « the loop resumes on its own » into a fact rather than a hope.
    //     Anyone tempted to harden it because it is fail-open should know the
    //     safety does not rest on it; anyone tempted to make the SHELL guard read
    //     this stamp would turn the protection into a cache, and a stale cache is
    //     a fail-open with one more step.
    //
    //     Placement mirrors 2c and for the same reasons: after the cheap
    //     in-memory gates, before step 3's token resolution. Ordering against 2c
    //     is deliberate — a ticket both in flight and behind a dead relay is
    //     refused as "in flight", because that pilot started before the outage
    //     and its own refusal is the more precise statement.
    //
    //     Refusal returns `Handled`, never `Passthrough` — for the fifth time in
    //     this function and for the reason written at each of the other four.
    //
    //     Fail-open: an absent, unreadable, unparseable or stale stamp reads as
    //     "serving" and the dispatch proceeds exactly as before this gate
    //     existed. None of those readings can open the network.
    if let crate::pilot_egress_stamp::RelayVerdict::Down { motif, age_secs } =
        crate::pilot_egress_stamp::relay_verdict(
            global_home_dir,
            crate::pilot_egress_stamp::ttl_secs(),
        )
    {
        let owner_repo = location.owner_repo();
        // WARN, not INFO — unlike 2c, this is not a nominal consequence of how
        // the feeder and the webhook compose. A `ready` event refused because the
        // host relay is down means the loop is stopped, which is the cost the
        // operator accepted in writing and wants to see.
        warn!(
            event = "ready_label_egress_relay_down",
            repo = %owner_repo,
            num = location.number,
            motif = %motif,
            age_secs,
            "ready_label_handler: `ready` event refused — the host egress relay is \
             down, so no pilot can leave contained (mika#2049). The ticket keeps \
             its label and is not parked."
        );

        if let Err(e) = db
            .log_audit_event(
                session_id,
                "ready_label_egress_relay_down",
                &format!("{}#{}", owner_repo, location.number),
                None,
                Some("dispatch_refused"),
                Some(&format!(
                    "repo={} number={} refused=egress_relay_down motif={} stamp_age_secs={}",
                    owner_repo, location.number, motif, age_secs
                )),
                Some(trace_id),
            )
            .await
        {
            warn!(
                event = "ready_label_audit_log_failed",
                repo = %owner_repo,
                num = location.number,
                error = %e,
                "ready_label_handler: failed to write egress-relay refusal audit event \
                 (non-fatal)"
            );
        }

        return (
            VerdictAction::Handled {
                pre_digest: format_egress_relay_down_pre_digest(location, &motif),
            },
            ReadyLabelGate::EgressRelayDown,
        );
    }

    // 3. Need a GitHub token to fetch the issue body. Without it we cannot
    //    determine groomed-state, so degrade to passthrough.
    let token = match github_token {
        Some(t) => t,
        None => {
            warn!(
                event = "ready_label_no_token",
                repo = %location.owner_repo(),
                num = location.number,
                "ready_label_handler: no GitHub token configured — passthrough"
            );
            return (
                VerdictAction::Passthrough { enrichment: None },
                ReadyLabelGate::NoToken,
            );
        }
    };

    // 4. Fetch issue body via `gh issue view`. Used to determine groomed-state
    //    via the same predicate the dispatch gate uses (#919, #1108).
    let (body, labels) =
        match fetch_issue(location.owner_repo(), location.number, token.to_string()).await {
            Ok(pair) => pair,
            Err(e) => {
                warn!(
                    event = "ready_label_body_fetch_failed",
                    repo = %location.owner_repo(),
                    num = location.number,
                    error = %e,
                    "ready_label_handler: gh issue view failed — passthrough"
                );
                return (
                    VerdictAction::Passthrough { enrichment: None },
                    ReadyLabelGate::BodyFetchFailed,
                );
            }
        };

    // 4b. Dispatch-seat gate (mika#2084). Placed here because it is the first
    //     point at which the issue's labels are known — they ride along on the
    //     step-4 `gh` call, so the gate costs no extra round trip — and still
    //     ahead of the step-7 task pre-create, which is what AC1 requires.
    //
    //     Refusal returns `Handled`, never `Passthrough`, for the same reason
    //     spelled out at the repo-allowlist gate above: `Passthrough` leaves
    //     `req.text` on the ready-label marker, which is exactly what
    //     `webhook_ready_label_dispatch` triggers on — the guard would re-prompt
    //     the LLM until it dispatched the very issue this gate just refused.
    //
    //     A missing/unreadable label set never reaches here: step 4 already
    //     passthrough'd on fetch failure, unchanged from before #2084 (AC3).
    let label_refs: Vec<&str> = labels.iter().map(String::as_str).collect();
    let seat_verdict = crate::webhook_dispatch::classify_dispatch_seat(label_refs);
    if seat_verdict.refuses() {
        let owner_repo = location.owner_repo();
        let found = seat_verdict.label().unwrap_or("<none>");
        let why = seat_verdict.refusal_reason().unwrap_or("seat_refused");
        let current = crate::webhook_dispatch::CURRENT_DISPATCH_SEAT;
        warn!(
            event = "ready_label_seat_mismatch",
            repo = %owner_repo,
            num = location.number,
            found_label = %found,
            current_seat = current,
            reason = why,
            "ready_label_handler: `ready` label on an issue another dispatch seat \
             owns — refused before task creation"
        );

        // Operator-visible record. As at the allowlist gate, no `task_id` exists
        // yet by construction, so the audit target is the issue reference.
        if let Err(e) = db
            .log_audit_event(
                session_id,
                "ready_label_seat_mismatch",
                &format!("{}#{}", owner_repo, location.number),
                None,
                Some("dispatch_refused"),
                Some(&format!(
                    "repo={} number={} found_label={} current_seat={} reason={}",
                    owner_repo, location.number, found, current, why
                )),
                Some(trace_id),
            )
            .await
        {
            warn!(
                event = "ready_label_audit_log_failed",
                repo = %owner_repo,
                num = location.number,
                error = %e,
                "ready_label_handler: failed to write seat-refusal audit event (non-fatal)"
            );
        }

        return (
            VerdictAction::Handled {
                pre_digest: format_seat_mismatch_pre_digest(location, &seat_verdict, current),
            },
            ReadyLabelGate::SeatMismatch,
        );
    }

    // 4c. Operator-held gate (mika#2263 défaut (c)). Same placement rationale as
    //     the seat gate above — the labels are already in hand from the step-4
    //     `gh` call, and this is still ahead of the step-7 pre-create, which is
    //     the property that matters: a held ticket produces ZERO tasks.
    //
    //     Measured 2026-09-09: #1781 carried `blocked` and had had its `ready`
    //     label removed, and this handler still re-dispatched it twice (pgid
    //     478551, 492118, row daba9416) on stale/redelivered `ready` events.
    //     `blocked` excluded the ticket from the `auto_pull` feeder and from
    //     nothing else — so the label did not contain the dispatch, it
    //     contained half the paths to it. The predicate is now shared with
    //     `auto_pull::feeder_exclusion_label`, which is what makes the two
    //     surfaces unable to drift apart again.
    //
    //     Refusal returns `Handled`, never `Passthrough`, for the third time in
    //     this function and for the same reason: `Passthrough` leaves `req.text`
    //     on the ready-label marker, which is exactly what the
    //     `webhook_ready_label_dispatch` INTENT_GUARD triggers on — it would
    //     re-prompt the LLM until it dispatched the ticket this gate just
    //     refused. Zero tasks created here and a guard-driven dispatch two
    //     steps later is not a gate.
    if let Some(held_by) =
        crate::webhook_dispatch::operator_held_label(labels.iter().map(String::as_str))
    {
        let owner_repo = location.owner_repo();
        warn!(
            event = "ready_label_operator_held",
            repo = %owner_repo,
            num = location.number,
            held_by = %held_by,
            "ready_label_handler: `ready` event on a ticket an operator is holding —              refused before task creation"
        );

        // Operator-visible record. As at the two gates above, no `task_id`
        // exists yet by construction, so the audit target is the issue
        // reference itself.
        if let Err(e) = db
            .log_audit_event(
                session_id,
                "ready_label_operator_held",
                &format!("{}#{}", owner_repo, location.number),
                None,
                Some("dispatch_refused"),
                Some(&format!(
                    "repo={} number={} held_by={} refused=operator_held",
                    owner_repo, location.number, held_by
                )),
                Some(trace_id),
            )
            .await
        {
            warn!(
                event = "ready_label_audit_log_failed",
                repo = %owner_repo,
                num = location.number,
                error = %e,
                "ready_label_handler: failed to write operator-held refusal audit event \
                 (non-fatal)"
            );
        }

        return (
            VerdictAction::Handled {
                pre_digest: format_operator_held_pre_digest(location, held_by),
            },
            ReadyLabelGate::OperatorHeld,
        );
    }

    // 5. Determine groomed-state via the canonical predicate. Same code path as
    //    `validate_dispatch_readiness` gate (#919) — drift between the two
    //    sites would re-introduce the bug class this handler closes.
    let missing_markers = crate::skills::executor::check_grooming_markers(&body);
    let is_groomed = missing_markers.is_empty();

    // 6. Target tool + skill + dispatch class. dev-groom for ungroomed, dev-pilot
    //    for groomed. This mirrors the auto-groom-on-dispatch behavior (mika#996)
    //    that the LLM was supposed to perform.
    let (target_tool, target_skill, dispatch_class) = if is_groomed {
        ("run_claude_pilot", "dev-pilot", "implement")
    } else {
        ("run_claude_pilot_groom", "dev-groom", "groom")
    };

    // 7. Pre-create the task in DB. The LLM's tool call will reuse this
    //    `task_id` rather than calling `create_task` first — removes one
    //    decision-point from the LLM's path. (`issue_url` was resolved at step
    //    2c, which needed it first.)
    let agent_id = db.agent_id().to_string();
    let task_label = format!("ready-label: {}#{}", location.owner_repo(), location.number);

    // 6b. Supersede-on-new-dispatch (mika#1934 AC2). Cancel any phantom tracking
    //     rows (blocked/in_progress, no process) left behind by a prior dispatch
    //     for this same issue before we insert the fresh one — this both prevents
    //     the idx_tasks_manual_active_ref_url collision that would fail step 7 and
    //     drains the escalation-artefact accumulation the mika#1712 sweep exists to
    //     mop up. Fail-open: never blocks the dispatch.
    let superseded = crate::tracking_cleanup::supersede_prior_tracking_rows(
        db,
        session_id,
        Some(trace_id),
        &issue_url,
        &task_label,
    )
    .await;
    if superseded > 0 {
        info!(
            event = "ready_label_superseded_prior_rows",
            repo = %location.owner_repo(),
            num = location.number,
            superseded = superseded,
            "ready_label_handler: superseded prior phantom tracking rows before dispatch"
        );
    }

    let new_task = NewTask {
        agent_id,
        team_run_id: None,
        parent_task_id: None,
        depth: 0,
        label: task_label,
        trigger_type: "manual".to_string(),
        cron_expr: None,
        event_source: None,
        event_offset_secs: None,
        condition_expr: None,
        next_fire_at: None,
        timeout_at: None,
        action_type: "none".to_string(),
        action_config: "{}".to_string(),
        input_context: None,
        created_by_session: Some(session_id.to_string()),
        created_trace_id: Some(trace_id.to_string()),
        reference_url: Some(issue_url.clone()),
        source: Some("self_dev".to_string()),
        metadata: None,
        r#type: Some("issue".to_string()),
        dispatch_class: Some(dispatch_class.to_string()),
    };
    let task_id = match db.create_task(new_task).await {
        Ok(id) => id,
        Err(e) => {
            // mika#2045 — name the victim. The overwhelming cause here is a
            // collision on `idx_tasks_manual_active_ref_url`: an existing active
            // task already holds this issue's slot. Logging the raw SQL error
            // said so 1966 times without ever naming which issue was refused or
            // how old the task refusing it was, so the log could not be turned
            // into a list of blocked issues.
            let blocker = describe_blocking_task(db, &issue_url).await;
            warn!(
                event = "ready_label_task_create_failed",
                repo = %location.owner_repo(),
                num = location.number,
                issue_url = %issue_url,
                blocking_task_id = blocker.as_ref().map(|b| b.id.as_str()),
                blocking_task_status = blocker.as_ref().map(|b| b.status.as_str()),
                blocking_task_age_secs = blocker.as_ref().map(|b| b.age_secs),
                error = %e,
                "ready_label_handler: failed to pre-create task — passthrough"
            );
            return (
                VerdictAction::Passthrough { enrichment: None },
                ReadyLabelGate::TaskCreateFailed,
            );
        }
    };

    // 8. Audit event — operator-visible record of the structural intervention.
    if let Err(e) = db
        .log_audit_event(
            session_id,
            "ready_label_handled",
            &format!("task:{task_id}"),
            None,
            Some(&format!("{target_skill}_dispatch_prepared")),
            Some(&format!(
                "repo={} number={} target_skill={} groomed={} task_id={}",
                location.owner_repo(),
                location.number,
                target_skill,
                is_groomed,
                task_id
            )),
            Some(trace_id),
        )
        .await
    {
        warn!(
            event = "ready_label_audit_log_failed",
            task_id = %task_id,
            error = %e,
            "ready_label_handler: failed to write audit event (non-fatal)"
        );
    }

    // 9. Engine-side dispatch (mika#1572, full Option Α). Spawn the dispatch
    //    subprocess directly instead of asking the LLM to call the tool. On ANY
    //    precondition failure, fall back to the #1571 prescriptive pre-digest
    //    (`VerdictAction::Handled`) per Resolution F3 — the engine dispatch is an
    //    upgrade over the prescriptive path, not a replacement of its fallback.
    let fallback = || VerdictAction::Handled {
        pre_digest: format_ready_label_pre_digest(
            location,
            is_groomed,
            target_tool,
            target_skill,
            &task_id,
        ),
    };

    // 9a. Resolve the dispatch tool from the agent's loaded SkillRegistry (KTD-1).
    let skill_tool = match skills.resolve_tool_by_name(target_tool) {
        Some(t) => t,
        None => {
            warn!(
                event = "ready_label_tool_not_found",
                target_tool,
                task_id = %task_id,
                "ready_label_handler: dispatch tool not in SkillRegistry — \
                 fallback to #1571 prescriptive pre-digest"
            );
            return (fallback(), ReadyLabelGate::ToolNotFound);
        }
    };

    // 9b. Resolve the long-running exec command + estimated duration from the
    //     tool handler. The estimate drives the callback timeout below; binding
    //     it (rather than hardcoding the 3600s default) keeps the callback's
    //     `timeout_at` identical to the LLM tool-call path for these tools
    //     (dev-pilot/dev-groom declare estimated_duration_secs=7200).
    let (command, estimated_duration_secs) = match &skill_tool.handler {
        ToolHandler::Exec {
            command,
            long_running: true,
            estimated_duration_secs,
        } => (command.clone(), *estimated_duration_secs),
        _ => {
            warn!(
                event = "ready_label_tool_not_long_running",
                target_tool,
                task_id = %task_id,
                "ready_label_handler: dispatch tool is not a long-running exec handler — \
                 fallback to #1571 prescriptive pre-digest"
            );
            return (fallback(), ReadyLabelGate::ToolNotLongRunning);
        }
    };

    // 9c. The dispatch input the LLM would have provided. `prompt` is the bare
    //     `<repo>#<num>` reference (NOT owner-qualified) — dispatch-lib's
    //     worktree-setup parser only accepts the bare form the tool schemas
    //     document; an owner-qualified prompt silently routes the dispatch into
    //     no-worktree free-text mode (mika#1593). `task_id` is the pre-created
    //     parent.
    let dispatch_input = serde_json::json!({
        "skill": target_skill,
        "prompt": format!("{}#{}", location.repo_name(), location.number),
        "task_id": task_id,
    });

    // 9d. Re-use the existing dispatch-readiness gate (slot availability,
    //     grooming markers, blockedBy). `originating_message = text` (the raw
    //     ready-label marker) keeps guard (0) authorized — it is not a webhook
    //     fallthrough event. On rejection, fall back (Resolution F3).
    if let Err(rejection) = crate::skills::executor::validate_dispatch_readiness(
        db,
        &task_id,
        github_token,
        Some(&dispatch_input),
        Some(text),
    )
    .await
    {
        warn!(
            event = "ready_label_dispatch_readiness_failed",
            task_id = %task_id,
            target_skill,
            rejection = %rejection,
            "ready_label_handler: dispatch readiness check failed — \
             fallback to #1571 prescriptive pre-digest"
        );
        return (fallback(), ReadyLabelGate::DispatchReadinessFailed);
    }

    // 9e. Create the callback child task (same shape as the LLM tool-call path
    //     via the shared `build_callback_task` helper). Timeout matches
    //     `execute_long_running` exactly: estimated_duration_secs (default 3600)
    //     × 3, clamped to 10min..90days (executor.rs).
    let timeout_secs = (estimated_duration_secs.unwrap_or(3600) * 3).clamp(600, 7_776_000);
    let callback_task = crate::skills::executor::build_callback_task(
        db.agent_id().to_string(),
        Some(task_id.clone()),
        target_tool,
        &dispatch_input,
        timeout_secs,
        session_id,
        trace_id,
        // mika#2368 : pas de cible QA à stamper — un dispatch ready-label est
        // un pilote, pas un build de revue.
        None,
    );
    let callback_task_id = match db.create_task(callback_task).await {
        Ok(id) => id,
        Err(e) => {
            warn!(
                event = "ready_label_callback_create_failed",
                task_id = %task_id,
                error = %e,
                "ready_label_handler: failed to create callback child — \
                 fallback to #1571 prescriptive pre-digest"
            );
            return (fallback(), ReadyLabelGate::CallbackCreateFailed);
        }
    };

    // 9f. Resolve and verify the handler script exists before committing to the
    //     spawn. If missing, mark the callback child failed so it does not dangle.
    let cmd_path = skill_tool.skill_dir.join(&command);
    if !cmd_path.exists() {
        warn!(
            event = "ready_label_handler_not_found",
            task_id = %task_id,
            callback_task_id = %callback_task_id,
            cmd_path = %cmd_path.display(),
            "ready_label_handler: dispatch handler script not found — \
             fallback to #1571 prescriptive pre-digest"
        );
        let _ = db
            .update_task_failed(
                &callback_task_id,
                &format!("handler not found: {}", cmd_path.display()),
            )
            .await;
        return (fallback(), ReadyLabelGate::HandlerNotFound);
    }

    // 9g. Auto-transition the pre-created parent task to in_progress and stamp
    //     `fired_at` (mirrors execute_long_running's #525 transition; the stamp
    //     is mika#2335). Non-fatal on error — the stamp is observability, it
    //     must never fail a dispatch.
    //
    //     This is the path of the 2026-09-15 incident: the parent row of a
    //     ready-label dispatch is the one an operator reads, and it read
    //     `fired_at = NULL` while its pilot was writing files.
    if let Err(e) = db.mark_parent_dispatched(&task_id).await {
        warn!(
            task_id = %task_id,
            error = %e,
            "ready_label_handler: failed to auto-transition parent task to in_progress"
        );
    }

    // 9h. Inject subprocess task metadata, mirroring execute_long_running:
    //     `__mika_task_id` is the callback child (delivery target), `__mika_agent`
    //     is this agent.
    let mut enriched_input = dispatch_input.clone();
    if let serde_json::Value::Object(ref mut map) = enriched_input {
        map.insert(
            "__mika_task_id".to_string(),
            serde_json::Value::String(callback_task_id.clone()),
        );
        map.insert(
            "__mika_agent".to_string(),
            serde_json::Value::String(db.agent_id().to_string()),
        );
    }

    // 9i. Spawn the detached subprocess. The LLM turn that follows only
    //     acknowledges — it has no dispatch decision left to make.
    crate::skills::executor::spawn_long_running_exec(
        cmd_path,
        skill_tool.skill_dir.clone(),
        enriched_input,
        callback_task_id.clone(),
        db.clone(),
        github_token.map(|s| s.to_string()),
    );

    info!(
        event = "ready_label_engine_dispatched",
        repo = %location.owner_repo(),
        number = location.number,
        target_tool,
        target_skill,
        dispatch_class,
        groomed = is_groomed,
        task_id = %task_id,
        callback_task_id = %callback_task_id,
        "ready_label_handler: engine-side dispatch spawned; LLM turn acknowledges only"
    );

    (
        VerdictAction::Dispatched {
            pre_digest: format_engine_dispatch_pre_digest(
                location,
                is_groomed,
                target_tool,
                target_skill,
                &task_id,
            ),
            task_id,
        },
        ReadyLabelGate::Dispatched,
    )
}

/// Parse `<repo>#<num>` from the ready-label marker text.
///
/// Format: `[GitHub] Issue labeled ready on <repo_ref>#<num> — <title>`.
/// `<repo_ref>` may be `owner/repo` or bare `repo`. Mirrors the parser shape
/// of `agent_loop::parse_ready_label_location` but returns structured data
/// instead of a free-form string.
pub(crate) fn parse_ready_label_location(text: &str) -> Option<ReadyLabelLocation> {
    let rest = text.strip_prefix(READY_LABEL_DISPATCH_MARKER)?;
    // Split on the FIRST space to bound the `<repo>#<num>` token.
    let token_end = rest.find(|c: char| c.is_whitespace()).unwrap_or(rest.len());
    let token = &rest[..token_end];
    let (repo_ref, num_str) = token.split_once('#')?;
    let number: u64 = num_str.parse().ok()?;
    if repo_ref.is_empty() {
        return None;
    }
    Some(ReadyLabelLocation {
        repo_ref: repo_ref.to_string(),
        number,
    })
}

/// Fetch the issue body **and its label names** via one `gh issue view
/// --json body,labels` call.
///
/// The labels ride along on the call the handler already makes for the body, so
/// the mika#2084 seat gate costs no extra round trip. Returns a descriptive
/// error string on failure — which the caller turns into a passthrough, exactly
/// as it did before the labels were added.
pub async fn fetch_issue_body_and_labels_via_gh(
    owner_repo: &str,
    number: u64,
    token: &str,
) -> Result<(String, Vec<String>), String> {
    let number_str = number.to_string();
    let args = [
        "issue",
        "view",
        &number_str,
        "--repo",
        owner_repo,
        "--json",
        "body,labels",
    ];
    let stdout = run_gh_subprocess(&args, token).await?;
    let parsed: serde_json::Value = serde_json::from_str(&stdout)
        .map_err(|e| format!("gh issue view returned non-JSON: {e}"))?;
    let body = parsed
        .get("body")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    let labels = parsed
        .get("labels")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|l| l.get("name").and_then(|n| n.as_str()))
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    Ok((body, labels))
}

/// Pre-digest for a `ready` event on a ticket whose pilot is still running
/// (mika#2279).
///
/// Opens with `<ready_label_handler>` for the same load-bearing reason as its
/// three neighbours: any text still matching the `webhook_ready_label_dispatch`
/// trigger would have the INTENT_GUARD demand the very dispatch this refusal
/// exists to prevent.
///
/// Names the pilot's pid and the row that carries it, **and the gesture that
/// lifts the refusal**. The last part is not politeness: a refusal that does not
/// name its own release is a refusal someone works around by guesswork, and the
/// guess available here — re-applying `ready` — is precisely the loop the gate
/// was built to stop.
fn format_pilot_in_flight_pre_digest(
    loc: &ReadyLabelLocation,
    pid: u32,
    child_task_id: &str,
    parent_task_id: &str,
) -> String {
    let owner_repo = loc.owner_repo();
    let number = loc.number;
    format!(
        "<ready_label_handler>\n\
         DISPATCH REFUSED — {owner_repo}#{number} already has a pilot running \
         (pid {pid}).\n\n\
         A repeated `ready` event on a ticket already in flight is a no-op. \
         Re-dispatching it would supersede the tracking row of the LIVE pilot, \
         kill it mid-work (mika#2335), and queue a duplicate dispatch for the same \
         ticket. Dispatch task: {parent_task_id}; dispatch child: {child_task_id}.\n\n\
         No task was created, no process was signalled, no dispatch was deferred. \
         You MUST NOT:\n\
         - call `run_claude_pilot` or `run_claude_pilot_groom` for this issue\n\
         - call `create_task` for this issue\n\
         - re-add or re-trigger the `ready` label\n\n\
         Acknowledge and end the turn; use `send_message` only if the operator \
         asked to be told.\n\n\
         To force a fresh dispatch, an operator runs `mika tasks cancel \
         {parent_task_id}` (which warns and asks for confirmation while the pilot \
         is alive) and then re-applies `ready`.\n\
         </ready_label_handler>"
    )
}

/// Pre-digest for a `ready` event refused because the host egress relay is down
/// (mika#2049).
///
/// Opens with `<ready_label_handler>` for the same load-bearing reason as its
/// four neighbours.
///
/// Two things it must say and a third it must not. It names **which organ is
/// broken** (the relay, not the ticket and not the worktree) and **that no
/// gesture is owed on the ticket** — the loop resumes on its own once the relay
/// serves, which is the operator's own acceptance criterion. It does NOT
/// prescribe a remedy on the relay: the model reading this cannot restart a host
/// daemon, and telling it to try would invite exactly the fabricated-action turn
/// the house guards against. The remedy travels on the escalation channel and in
/// the runbook, to a human who can act.
fn format_egress_relay_down_pre_digest(loc: &ReadyLabelLocation, motif: &str) -> String {
    let owner_repo = loc.owner_repo();
    let number = loc.number;
    format!(
        "<ready_label_handler>\n\
         DISPATCH REFUSED — the host egress relay is down ({motif}), so no pilot \
         can be launched with its network cut (mika#2049).\n\n\
         {owner_repo}#{number} keeps its `ready` label and is NOT parked. No task \
         was created, no dispatch was deferred, no re-drive budget was spent. The \
         loop resumes on its own once the relay serves again — no gesture is owed \
         on this ticket.\n\n\
         You MUST NOT:\n\
         - call `run_claude_pilot` or `run_claude_pilot_groom` for this issue\n\
         - call `create_task` for this issue\n\
         - remove, re-add or re-trigger the `ready` label\n\
         - claim the relay has been restarted, or attempt to restart it\n\n\
         An operator has already been escalated to on the notification channel. \
         Acknowledge and end the turn.\n\
         </ready_label_handler>"
    )
}

/// Pre-digest for a `ready` event on a ticket an operator is holding
/// (mika#2263 défaut (c)).
///
/// Opens with `<ready_label_handler>` for the same load-bearing reason as the
/// two refusals below it: any text still matching the
/// `webhook_ready_label_dispatch` trigger would have the guard demand the very
/// dispatch this refusal exists to prevent.
///
/// Names the issue AND the label holding it, because the operator reading this
/// needs to know which label to remove to release the ticket — "refused" alone
/// sends them looking.
fn format_operator_held_pre_digest(loc: &ReadyLabelLocation, held_by: &str) -> String {
    let owner_repo = loc.owner_repo();
    let number = loc.number;
    format!(
        "<ready_label_handler>\n\
         DISPATCH REFUSED — {owner_repo}#{number} carries the `{held_by}` label.\n\n\
         `{held_by}` means someone is holding this ticket: the autonomous loop does not \
         dispatch it, whatever `ready` events arrive for it (stale, redelivered, or applied \
         by hand). The same label already keeps it out of the auto-pull feeder — this gate \
         is the other half of that hold.\n\n\
         No task was created. Do NOT call run_claude_pilot or run_claude_pilot_groom for \
         this issue. Acknowledge and end the turn; use send_message only if the operator \
         asked to be told.\n\n\
         To release the ticket, an operator removes `{held_by}` and re-applies `ready`.\n\
         </ready_label_handler>"
    )
}

/// Pre-digest for a `ready` label on an issue another dispatch seat owns
/// (mika#2084).
///
/// Opens with `<ready_label_handler>` for the same load-bearing reason as the
/// allowlist pre-digest above: any text that still matches the
/// `webhook_ready_label_dispatch` trigger would have the guard demand the very
/// dispatch this refusal exists to prevent.
///
/// Names the issue, the seat label found, and the seat this engine dispatches
/// as — mika#2084 AC1 requires all three, so that an avoided collision is
/// legible without cross-referencing anything.
fn format_seat_mismatch_pre_digest(
    loc: &ReadyLabelLocation,
    verdict: &crate::webhook_dispatch::SeatVerdict,
    current_seat: &str,
) -> String {
    let found_label = verdict.label().unwrap_or("<none>");
    format!(
        "<ready_label_handler>\n\
         [GitHub] Issue labeled ready on {}#{} — DISPATCH REFUSED by engine.\n\n\
         Reason: {}\n\
         This engine dispatches as seat `{}`. One dispatcher per ticket, so a \
         ticket this engine cannot claim is not dispatched.\n\
         Known seats: {}\n\n\
         No task was created and no dispatch was prepared. You MUST NOT:\n\
         - call `run_claude_pilot` or `run_claude_pilot_groom` for this issue\n\
         - call `create_task` for this issue\n\
         - re-add or re-trigger the `ready` label\n\
         - remove or edit the `{}` label to get around this\n\n\
         REQUIRED next action: call `send_message` once to tell the operator \
         that {}#{} was refused because of its `{}` seat label, then end the \
         turn.\n\
         </ready_label_handler>",
        loc.owner_repo(),
        loc.number,
        crate::webhook_dispatch::seat_refusal_sentence(verdict),
        current_seat,
        crate::webhook_dispatch::known_dispatch_seats_display(),
        found_label,
        loc.owner_repo(),
        loc.number,
        found_label,
    )
}

/// Pre-digest for a `ready` label on a repository outside the dispatchable
/// allowlist (mika#2046).
///
/// Opens with `<ready_label_handler>` so it does not match the
/// `webhook_ready_label_dispatch` INTENT_GUARD trigger — otherwise the guard
/// would demand the dispatch this refusal exists to prevent. Quotes the
/// allowlist so the operator reading it learns what would have been accepted.
///
/// **The `send_message` instruction below is advisory, not enforced.** Replacing
/// `req.text` also takes the message out of the `[GitHub]` prefix domain, so
/// `webhook_zero_tools` does not fire either: an LLM that reads this and ends
/// its turn with no tool call is not re-prompted, and the operator learns of the
/// refusal only from the audit event and the structured warning above. That is a
/// degraded notification, not a silent dispatch — the refusal itself is
/// structural and already happened. Making the notification structural too would
/// mean an engine-side send or label removal; deliberately not done here, since
/// it widens this gate into notification policy.
fn format_repo_not_dispatchable_pre_digest(loc: &ReadyLabelLocation, allowed: &str) -> String {
    format!(
        "<ready_label_handler>\n\
         [GitHub] Issue labeled ready on {}#{} — DISPATCH REFUSED by engine.\n\n\
         Reason: `{}` is not a repository the autonomous loop may dispatch into.\n\
         Dispatchable repositories: {}\n\n\
         No task was created and no dispatch was prepared. You MUST NOT:\n\
         - call `run_claude_pilot` or `run_claude_pilot_groom` for this issue\n\
         - call `create_task` for this issue\n\
         - re-add or re-trigger the `ready` label\n\n\
         REQUIRED next action: call `send_message` once to tell the operator that \
         `ready` was set on {}#{}, that this repository is outside the loop's \
         allowlist, and that the work must be started as a Claude Code spawn \
         instead. Then EndTurn.\n\
         </ready_label_handler>",
        loc.owner_repo(),
        loc.number,
        loc.owner_repo(),
        allowed,
        loc.owner_repo(),
        loc.number,
    )
}

/// Build the prescriptive pre-digest delivered to the LLM in place of the raw
/// ready-label marker text.
///
/// The pre-digest names the exact tool, exact args (`skill`, `prompt`, `task_id`),
/// and engine state — removing decision points that LLM disobedience has
/// historically exploited (mika#1384 n>=6 incidents through 2026-06-26).
fn format_ready_label_pre_digest(
    loc: &ReadyLabelLocation,
    is_groomed: bool,
    target_tool: &str,
    target_skill: &str,
    task_id: &str,
) -> String {
    let owner_repo = loc.owner_repo();
    // Bare `repo#number` for the dispatch `prompt` arg (mika#1593) — the
    // cosmetic marker line below stays owner-qualified to mirror the webhook
    // marker, but the dispatch prompt MUST be the bare form dispatch-lib parses.
    let repo_name = loc.repo_name();
    let groomed_line = if is_groomed {
        "Groomed-state: GROOMED (Plan callout + Branch callout + second-pass marker all present)"
    } else {
        "Groomed-state: UNGROOMED (auto-groom required first; the engine will dispatch dev-groom)"
    };
    format!(
        "<ready_label_handler>\n\
         [GitHub] Issue labeled ready on {}#{} — structural dispatch prepared by engine.\n\n\
         {}\n\
         Task pre-created: {}\n\
         Target skill: {}\n\
         Target dispatch class: {}\n\n\
         REQUIRED next action: call `{}` exactly once with these arguments:\n\
         ```\n\
         {{\n  \"skill\": \"{}\",\n  \"prompt\": \"{}#{}\",\n  \"task_id\": \"{}\"\n}}\n\
         ```\n\n\
         The engine has pre-flighted dispatch readiness. You MUST NOT:\n\
         - call `create_task` (the task is already created above)\n\
         - call `run_gh` to re-fetch the issue body (the engine already checked it)\n\
         - call `send_message` before the dispatch (acknowledgement comes AFTER)\n\
         - skip the dispatch and EndTurn — the existing webhook_ready_label_dispatch\n\
           guard will reject the EndTurn and the operator will see the failure.\n\n\
         After the dispatch is registered (success returns `status: \"deferred\"` if\n\
         the slot is taken, or a fresh callback id otherwise), you MAY call\n\
         `send_message` to acknowledge to the operator and then EndTurn.\n\
         </ready_label_handler>",
        owner_repo,
        loc.number,
        groomed_line,
        task_id,
        target_skill,
        if is_groomed { "implement" } else { "groom" },
        target_tool,
        target_skill,
        repo_name,
        loc.number,
        task_id,
    )
}

/// Build the post-dispatch pre-digest delivered to the LLM after the engine has
/// already spawned the dispatch subprocess (mika#1572 AC2 semantics shift).
///
/// Unlike `format_ready_label_pre_digest` (which instructs the LLM to MAKE the
/// dispatch call), this tells the LLM the dispatch has ALREADY fired — there is
/// no decision left. It intentionally starts with `<ready_label_handler>` (NOT
/// the `[GitHub] Issue labeled ready on` marker) so that
/// `webhook_dispatch::is_ready_label_dispatch_marker` (a `starts_with` predicate)
/// returns false on the replaced `req.text`. The `webhook_ready_label_dispatch`
/// INTENT_GUARD therefore does not fire — the guard composes by construction with
/// no flag threading (plan addendum C4/C5).
///
/// The wording is also kept guard-safe against the *assistant-text* guards that
/// scan the LLM's reply (not the user message): the header avoids the
/// completion-claim keywords (#483 — `complete`/`merged`/`deployed`/`shipped`),
/// and the optional-ack guidance steers the model toward a grounded action
/// statement ("a dispatch subprocess was launched") rather than an affirmative
/// issue-state claim ("issue #N is ready/groomed", which `assert_grounded` #1331
/// would flag and try to "fix" with a `run_gh` call the digest forbids). This
/// avoids a wasted self-healing retry turn after every engine dispatch.
fn format_engine_dispatch_pre_digest(
    loc: &ReadyLabelLocation,
    is_groomed: bool,
    target_tool: &str,
    target_skill: &str,
    task_id: &str,
) -> String {
    let owner_repo = loc.owner_repo();
    let groomed_line = if is_groomed {
        "Groomed-state: GROOMED — dev-pilot dispatched to implement."
    } else {
        "Groomed-state: UNGROOMED — dev-groom dispatched to groom first; \
         implementation follows via callback."
    };
    format!(
        "<ready_label_handler>\n\
         [GitHub] Issue labeled ready on {}#{} — ENGINE-SIDE DISPATCH FIRED.\n\n\
         {}\n\
         The engine has ALREADY spawned `{}` with skill=`{}`, task_id=`{}`.\n\
         The claude-pilot subprocess is now running in the background; the work is \
         NOT finished.\n\n\
         There is NO dispatch decision left for you to make. You MUST NOT:\n\
         - call `{}` (the dispatch already fired; a second call would double-dispatch)\n\
         - call `create_task` or `run_gh` (the engine already pre-flighted everything)\n\
         - claim the issue is complete, merged, groomed, done, or otherwise assert its\n\
           state — the run is still in progress and you have verified no outcome\n\n\
         You MAY optionally call `send_message` to tell the operator that a dispatch \
         subprocess was launched for this ticket, then EndTurn. If there is no reply \
         channel, just EndTurn.\n\
         </ready_label_handler>",
        owner_repo, loc.number, groomed_line, target_tool, target_skill, task_id, target_tool,
    )
}

/// The active task holding an issue's slot in `idx_tasks_manual_active_ref_url`
/// (mika#2045), reduced to the three fields the failure log needs.
struct BlockingTask {
    id: String,
    status: String,
    age_secs: i64,
}

/// Identify the task refusing a `ready-label` task creation, for the failure log.
///
/// Reuses [`AsyncDatabase::find_active_task_by_ref_url`] — the same lookup
/// `create_task` uses for dedup, so the answer is the actual blocker rather than
/// a guess. Returns `None` when there is no blocker or the lookup itself fails:
/// the diagnostic must not depend on a second query succeeding, so the enriched
/// event is emitted either way, with the blocker fields simply absent.
async fn describe_blocking_task(db: &AsyncDatabase, issue_url: &str) -> Option<BlockingTask> {
    let task = match db.find_active_task_by_ref_url(issue_url).await {
        Ok(t) => t?,
        Err(e) => {
            warn!(issue_url, error = %e, "failed to look up the blocking task for the failure log");
            return None;
        }
    };
    Some(BlockingTask {
        age_secs: task_age_secs(&task.created_at),
        id: task.id,
        status: task.status,
    })
}

/// Seconds since `created_at`, or 0 when the timestamp cannot be parsed.
/// Conservative: an unparseable timestamp must not report a huge age and send an
/// operator chasing a task that is actually fresh.
fn task_age_secs(created_at: &str) -> i64 {
    crate::timestamp::parse(created_at)
        .map(|dt| (chrono::Utc::now() - dt).num_seconds())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---------------------------------------------------------------------
    // mika#2323 — gate vocabulary, actor readability, and the invariant that
    // the actor decides nothing.
    // ---------------------------------------------------------------------

    /// Test-only affordances for the gate vocabulary.
    ///
    /// They live **inside `mod tests`** rather than beside `wire_name`, under
    /// their own `#[cfg(test)]`, for a reason worth knowing before moving them
    /// back: `test_dispatch_fired_at_stamped::every_production_dispatch_path_stamps`
    /// (mika#2335) bounds "the production half of this file" at the **first**
    /// `#[cfg(test)]`. A `#[cfg(test)]` item placed near the top of the file
    /// truncates that scan before it reaches `spawn_long_running_exec`, and the
    /// mika#2335 guard fails — loudly and correctly, but for a reason that has
    /// nothing to do with `fired_at`. Keeping test items in the test module
    /// keeps that assumption true. (The assumption is itself fragile and
    /// undocumented at its site; flagged, not fixed here — out of scope.)
    impl ReadyLabelGate {
        /// Every variant, in declaration order.
        ///
        /// Completeness is enforced by `index_in_all`, not by convention.
        pub(crate) const ALL: &'static [ReadyLabelGate] = &[
            Self::NotAMarker,
            Self::ParseFailed,
            Self::RepoNotDispatchable,
            Self::PilotInFlight,
            Self::EgressRelayDown,
            Self::NoToken,
            Self::BodyFetchFailed,
            Self::SeatMismatch,
            Self::OperatorHeld,
            Self::TaskCreateFailed,
            Self::ToolNotFound,
            Self::ToolNotLongRunning,
            Self::DispatchReadinessFailed,
            Self::CallbackCreateFailed,
            Self::HandlerNotFound,
            Self::Dispatched,
        ];

        /// Compile-time witness that [`ALL`](Self::ALL) is complete.
        ///
        /// Exhaustive, so a new variant fails to compile here; the index must
        /// then point at a real `ALL` slot or
        /// `mika2323_every_gate_variant_has_a_wire_name` reddens. Without it,
        /// `ALL` would be a hand-kept list and a variant could be added —
        /// correctly named by `wire_name` — while silently escaping every test
        /// that iterates the vocabulary.
        fn index_in_all(self) -> usize {
            match self {
                Self::NotAMarker => 0,
                Self::ParseFailed => 1,
                Self::RepoNotDispatchable => 2,
                Self::PilotInFlight => 3,
                Self::EgressRelayDown => 4,
                Self::NoToken => 5,
                Self::BodyFetchFailed => 6,
                Self::SeatMismatch => 7,
                Self::OperatorHeld => 8,
                Self::TaskCreateFailed => 9,
                Self::ToolNotFound => 10,
                Self::ToolNotLongRunning => 11,
                Self::DispatchReadinessFailed => 12,
                Self::CallbackCreateFailed => 13,
                Self::HandlerNotFound => 14,
                Self::Dispatched => 15,
            }
        }
    }

    /// Predicates allowed to read the actor identity. **MUST STAY EMPTY.**
    ///
    /// Any entry added here is an identity-filtering policy nobody has decided
    /// (mika#2323 R4; explicitly out of scope in that plan's §9) and requires a
    /// ticket named in the second member of the pair.
    ///
    /// # Why it is born empty, and why it exists at all
    ///
    /// The pre-existing violation population is **empty by construction, and
    /// that was verifiable before a line was written**: the field this detector
    /// forbids reading did not exist on the agent side — this same change
    /// introduces it. `GitHubWebhookEvent.sender` was deserialized by the
    /// gateway and never emitted, so no predicate here *could* read an identity
    /// it never received. "Land disabled" would have shipped an inert detector
    /// during precisely the window in which the watched field is born — the one
    /// moment an accidental read can be introduced.
    ///
    /// The self-cleaning assertion below is vacuously true at zero entries. It
    /// is written now because an allowlist that does not clean itself turns its
    /// first entry into a permanent permission nobody re-reads, and the moment
    /// to write that guard is before there is anything to guard.
    const ACTOR_READING_PREDICATES_ALLOWED: &[(&str, &str)] = &[]; // (function, ticket)

    /// Functions that are *supposed* to handle the actor: the two emission
    /// sites and the parser itself. Everything else in this module is a
    /// decision path and must not touch it.
    const ACTOR_EMISSION_SITES: &[&str] = &[
        "parse_event_actor",
        "emit_ready_label_received",
        "emit_ready_label_outcome",
    ];

    /// Split the module source into `(fn name, body)` pairs, ignoring the test
    /// module (which legitimately names the actor everywhere).
    ///
    /// **Comments are stripped first**, the same discipline as
    /// `milestone_manager::no_dispatch_test`: without it a function's "body"
    /// swallows the doc-comment of the function that follows it, and prose
    /// describing what is forbidden reads as a violation of it. That is not a
    /// hypothetical — it is what the first run of this guard reported.
    fn production_fn_bodies(src: &str) -> Vec<(String, String)> {
        let production = match src.find("\nmod tests {") {
            Some(i) => &src[..i],
            None => src,
        };
        let stripped: String = production
            .lines()
            .filter(|l| !l.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");

        let mut out = Vec::new();
        let mut cursor = 0usize;
        // Bound each body at the NEXT `fn ` at any indentation — `\nfn ` would
        // miss `pub async fn` and every method inside an `impl`, which is how
        // the first version of this scan let every body run to the end of file.
        while let Some(pos) = stripped[cursor..].find("fn ") {
            let sig_start = cursor + pos + 3;
            let after = &stripped[sig_start..];
            let name_end = after.find(['(', '<', ' ']).unwrap_or(after.len());
            let name = after[..name_end].to_string();
            let body = match after[name_end..].find("fn ") {
                Some(next) => &after[name_end..name_end + next],
                None => &after[name_end..],
            };
            out.push((name, body.to_string()));
            cursor = sig_start + name_end;
        }
        out
    }

    /// The detector proper: which production functions of a given source read
    /// the actor identity, excluding the declared emission sites.
    ///
    /// Extracted from the guard so it can be run against a synthetic source and
    /// **shown to bite** — a detector whose positive control is only ever the
    /// real file is a detector nobody has watched fail.
    fn actor_reading_fns(src: &str) -> Vec<String> {
        production_fn_bodies(src)
            .into_iter()
            .filter(|(name, _)| !ACTOR_EMISSION_SITES.contains(&name.as_str()))
            // The wrapper resolves the actor and hands it to the two emission
            // sites; it takes no refusal decision of its own (every gate lives
            // in `…_inner`), so the binding itself is not a read-in-a-predicate.
            .filter(|(name, _)| name != "try_handle_ready_label_dispatch_with_fetcher")
            .filter(|(_, body)| {
                body.contains("parse_event_actor") || body.contains("LABELED_BY_LINE_PREFIX")
            })
            .map(|(name, _)| name)
            .collect()
    }

    /// Positive control for the guard below (mika#2323).
    ///
    /// Without this, `mika2323_no_gate_predicate_reads_the_actor` passing would
    /// be compatible with a scan that matches nothing at all — the failure mode
    /// of every source-scan guard, and the one a green test cannot distinguish
    /// from compliance.
    #[test]
    fn mika2323_the_actor_guard_bites_on_a_planted_read() {
        let planted = r#"
fn some_refusal_gate(text: &str) -> bool {
    let actor = parse_event_actor(text);
    actor == Some("mika-platform-bot")
}

fn an_innocent_helper(x: u32) -> u32 {
    x + 1
}

mod tests {
    fn this_is_test_code(text: &str) { let _ = parse_event_actor(text); }
}
"#;
        let found = actor_reading_fns(planted);
        assert_eq!(
            found,
            vec!["some_refusal_gate".to_string()],
            "the guard must catch a predicate reading the actor, must not flag an \
             unrelated helper, and must not reach into the test module"
        );
    }

    /// mika#2323 R4/AC5 — **no refusal predicate reads the actor.**
    ///
    /// A source scan, and it has to be: a read of the actor would make no
    /// decision *wrong*, it would introduce a policy. Every behavioural
    /// assertion in this crate would stay green while the ready-label handler
    /// silently acquired the identity filter the founding ticket set out to
    /// find and did not. Same shape, and same reasoning, as
    /// `mika2205_periodic_scans_do_not_read_the_pat_field_directly`.
    ///
    /// **Resolution when it fires: remove the read.** Adding an allowlist entry
    /// decides an identity-filtering policy, which is out of scope by
    /// construction — it needs its own ticket, named in the pair.
    #[test]
    fn mika2323_no_gate_predicate_reads_the_actor() {
        let src = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("src/server/ready_label_handler.rs"),
        )
        .expect("the guard must be able to read ready_label_handler.rs");

        let violations = actor_reading_fns(&src);

        let allowed: Vec<&str> = ACTOR_READING_PREDICATES_ALLOWED
            .iter()
            .map(|(f, _)| *f)
            .collect();
        let unexpected: Vec<&String> = violations
            .iter()
            .filter(|v| !allowed.contains(&v.as_str()))
            .collect();
        assert!(
            unexpected.is_empty(),
            "mika#2323 R4 VIOLATED — these functions read the actor identity: {unexpected:?}. \
             The actor is informational: no refusal decision may read it, or the handler \
             acquires the identity filter mika#2323 established does not exist. \
             Resolution: REMOVE the read. Adding an entry to \
             ACTOR_READING_PREDICATES_ALLOWED decides a filtering policy and needs its own \
             ticket."
        );

        // Self-cleaning half: an allowlist entry that no longer matches a real
        // violation is a stale permission, and stale permissions are how an
        // exception outlives the reason for it.
        for (f, ticket) in ACTOR_READING_PREDICATES_ALLOWED {
            assert!(
                violations.iter().any(|v| v == f),
                "stale allowlist entry {f:?} (ticket {ticket}) — it no longer reads the \
                 actor; remove the entry"
            );
        }
    }

    /// mika#2323 — the gate names are a WIRE FORMAT.
    ///
    /// They land in `audit_events.after_value` and operators `GROUP BY` them
    /// (mika#2131 doctrine). When this test fires it fires on a **rename**:
    /// resolution is to revert the rename, or to date the break in `CLAUDE.md`
    /// — never to update the expectation in silence.
    #[test]
    fn mika2323_gate_names_are_a_wire_format() {
        let expected = [
            (ReadyLabelGate::NotAMarker, "not_a_marker"),
            (ReadyLabelGate::ParseFailed, "parse_failed"),
            (ReadyLabelGate::RepoNotDispatchable, "repo_not_dispatchable"),
            (ReadyLabelGate::PilotInFlight, "pilot_in_flight"),
            (ReadyLabelGate::NoToken, "no_token"),
            (ReadyLabelGate::BodyFetchFailed, "body_fetch_failed"),
            (ReadyLabelGate::SeatMismatch, "seat_mismatch"),
            (ReadyLabelGate::OperatorHeld, "operator_held"),
            (ReadyLabelGate::TaskCreateFailed, "task_create_failed"),
            (ReadyLabelGate::ToolNotFound, "tool_not_found"),
            (ReadyLabelGate::ToolNotLongRunning, "tool_not_long_running"),
            (
                ReadyLabelGate::DispatchReadinessFailed,
                "dispatch_readiness_failed",
            ),
            (
                ReadyLabelGate::CallbackCreateFailed,
                "callback_create_failed",
            ),
            (ReadyLabelGate::HandlerNotFound, "handler_not_found"),
            (ReadyLabelGate::Dispatched, "dispatched"),
        ];
        for (gate, name) in expected {
            assert_eq!(
                gate.wire_name(),
                name,
                "{gate:?} renamed — this value is read by operator SQL; renaming it splits \
                 one population in two without saying so"
            );
        }

        let mut names: Vec<&str> = ReadyLabelGate::ALL.iter().map(|g| g.wire_name()).collect();
        names.sort_unstable();
        let before = names.len();
        names.dedup();
        assert_eq!(
            before,
            names.len(),
            "two gates share a wire name — a GROUP BY would merge two distinct refusals"
        );
    }

    /// mika#2323 R3/AC4 — a new exit cannot stay anonymous.
    ///
    /// The real guarantee is the exhaustive `match` in `wire_name` (adding a
    /// variant is a compile error there). This asserts the second half:
    /// `ALL` is complete, so a variant cannot be correctly named yet escape
    /// every test that iterates the vocabulary.
    #[test]
    fn mika2323_every_gate_variant_has_a_wire_name() {
        for (i, gate) in ReadyLabelGate::ALL.iter().enumerate() {
            assert_eq!(
                gate.index_in_all(),
                i,
                "{gate:?} is not at its declared index in ALL — the list and the witness \
                 disagree, so ALL can no longer be trusted to be complete"
            );
            assert!(
                !gate.wire_name().is_empty(),
                "{gate:?} has an empty wire name"
            );
        }
        assert_eq!(
            ReadyLabelGate::ALL.len(),
            16,
            "the handler has sixteen ways out (mika#2323 M3, +1 for the mika#2049 \
             egress-relay gate); if that changed, update the inventory in CLAUDE.md \
             in the same commit"
        );
    }

    /// mika#2323 AC7 — appending the actor line breaks neither the marker nor
    /// the `<repo>#<n>` parse.
    ///
    /// This is the compatibility claim the plan makes by reading the code; it
    /// is asserted here rather than trusted, because the whole of axis 2 rides
    /// on it.
    #[test]
    fn mika2323_actor_line_preserves_the_marker_and_the_parse() {
        let text = "[GitHub] Issue labeled ready on senara-solutions/mika#2323 — titre\n\
                    https://github.com/senara-solutions/mika/issues/2323\n\
                    Labeled by: @samidarko";
        assert!(
            text.starts_with(READY_LABEL_DISPATCH_MARKER),
            "the actor line must be APPENDED — a prefix change would make the handler \
             stop recognizing its own trigger"
        );
        let loc = parse_ready_label_location(text).expect("location must still parse");
        assert_eq!(loc.owner_repo(), "senara-solutions/mika");
        assert_eq!(loc.number, 2323);
        assert_eq!(parse_event_actor(text), Some("samidarko"));
    }

    /// mika#2323 AC5 — an absent or malformed actor yields `None`, never an
    /// error, and never changes an outcome.
    ///
    /// The negative controls matter more than the positive one: an old gateway
    /// serving a new agent (or the reverse) must behave exactly as before.
    #[test]
    fn mika2323_absent_actor_is_none_never_an_error() {
        let no_actor = "[GitHub] Issue labeled ready on senara-solutions/mika#2323 — titre\n\
                        https://github.com/senara-solutions/mika/issues/2323";
        assert_eq!(
            parse_event_actor(no_actor),
            None,
            "pre-mika#2323 gateway output must read as 'no actor', not as an error"
        );
        assert!(
            parse_ready_label_location(no_actor).is_some(),
            "and the event must still be handled exactly as before"
        );

        // Malformed shapes: the prefix without a login, whitespace only, and
        // the line present but not last (the producer always appends it last,
        // so anything else is not the contract and must not be trusted).
        for malformed in [
            "[GitHub] Issue labeled ready on a/b#1 — t\nLabeled by: @",
            "[GitHub] Issue labeled ready on a/b#1 — t\nLabeled by: @   ",
            "[GitHub] Issue labeled ready on a/b#1 — t\nLabeled by: samidarko",
            "[GitHub] Issue labeled ready on a/b#1 — t\nLabeled by: @sami\nhttps://x/y",
        ] {
            assert_eq!(
                parse_event_actor(malformed),
                None,
                "malformed actor line must read as absent, never as a login: {malformed:?}"
            );
        }
    }

    /// The `action` field of the outcome line tracks the three `VerdictAction`
    /// shapes. Pinned because `gate` and `action` answer different questions
    /// and an operator reads them together.
    #[test]
    fn mika2323_action_label_covers_every_verdict_shape() {
        assert_eq!(
            action_label(&VerdictAction::Passthrough { enrichment: None }),
            "passthrough"
        );
        assert_eq!(
            action_label(&VerdictAction::Handled {
                pre_digest: String::new()
            }),
            "handled"
        );
        assert_eq!(
            action_label(&VerdictAction::Dispatched {
                pre_digest: String::new(),
                task_id: String::new()
            }),
            "dispatched"
        );
    }

    // -- blocking-task identification for the failure log (mika#2045) --

    fn stuck_parent(issue_url: &str) -> NewTask {
        NewTask {
            agent_id: "mika".to_string(),
            team_run_id: None,
            parent_task_id: None,
            depth: 0,
            label: "ready-label: senara-solutions/mika#2013".to_string(),
            trigger_type: "manual".to_string(),
            cron_expr: None,
            event_source: None,
            event_offset_secs: None,
            condition_expr: None,
            next_fire_at: None,
            timeout_at: None,
            action_type: "none".to_string(),
            action_config: "{}".to_string(),
            input_context: None,
            created_by_session: None,
            created_trace_id: None,
            reference_url: Some(issue_url.to_string()),
            source: Some("self_dev".to_string()),
            metadata: None,
            r#type: Some("issue".to_string()),
            dispatch_class: Some("implement".to_string()),
        }
    }

    /// The whole point of the enrichment: turn `UNIQUE constraint failed` into a
    /// row an operator can act on.
    #[tokio::test]
    async fn names_the_task_holding_the_issue_slot() {
        let db = crate::db::Database::open_in_memory().unwrap();
        let async_db = crate::async_db::AsyncDatabase::new(db);
        let issue_url = "https://github.com/senara-solutions/mika/issues/2013";

        let blocker_id = async_db.create_task(stuck_parent(issue_url)).await.unwrap();
        async_db
            .with_db({
                let id = blocker_id.clone();
                move |d| {
                    d.conn.execute(
                        "UPDATE tasks SET created_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now', '-3600 seconds')
                         WHERE id = ?1",
                        rusqlite::params![id],
                    )?;
                    Ok(())
                }
            })
            .await
            .unwrap();

        let blocker = describe_blocking_task(&async_db, issue_url)
            .await
            .expect("the colliding task must be named");

        assert_eq!(blocker.id, blocker_id);
        assert_eq!(blocker.status, "pending");
        assert!(
            blocker.age_secs >= 3600,
            "the age is what tells the operator this is stuck, not merely busy"
        );
    }

    /// A creation failure for any other reason still emits the enriched event —
    /// the blocker fields are simply absent, and nothing panics.
    #[tokio::test]
    async fn reports_no_blocker_when_the_slot_is_free() {
        let db = crate::db::Database::open_in_memory().unwrap();
        let async_db = crate::async_db::AsyncDatabase::new(db);

        assert!(
            describe_blocking_task(
                &async_db,
                "https://github.com/senara-solutions/mika/issues/9999"
            )
            .await
            .is_none()
        );
    }

    /// A terminal task released the slot, so it is not the blocker.
    #[tokio::test]
    async fn ignores_a_terminal_task_on_the_same_issue() {
        let db = crate::db::Database::open_in_memory().unwrap();
        let async_db = crate::async_db::AsyncDatabase::new(db);
        let issue_url = "https://github.com/senara-solutions/mika/issues/2013";

        let id = async_db.create_task(stuck_parent(issue_url)).await.unwrap();
        async_db.update_task_status(&id, "failed").await.unwrap();

        assert!(describe_blocking_task(&async_db, issue_url).await.is_none());
    }

    #[test]
    fn unparseable_created_at_reports_zero_age() {
        assert_eq!(task_age_secs("not a timestamp"), 0);
        assert!(task_age_secs("2020-01-01T00:00:00Z") > 0);
    }

    /// mika#2046 — the refusal digest must not re-arm the intent guard it is
    /// meant to sidestep. Asserted through the predicate the guard actually
    /// calls, not by eyeballing the string.
    #[test]
    fn refusal_pre_digest_does_not_trigger_the_ready_label_intent_guard() {
        let loc = ReadyLabelLocation {
            repo_ref: "senara-solutions/control-monitor".to_string(),
            number: 159,
        };
        let digest = format_repo_not_dispatchable_pre_digest(
            &loc,
            &crate::webhook_dispatch::dispatchable_repos_display(),
        );
        assert!(digest.starts_with("<ready_label_handler>"));
        assert!(
            !crate::webhook_dispatch::is_ready_label_dispatch_marker(&digest),
            "refusal digest must not match the ready-label trigger, or the guard \
             would demand the dispatch this refusal prevents"
        );
    }

    #[test]
    fn refusal_pre_digest_names_the_repo_and_quotes_the_allowlist() {
        let loc = ReadyLabelLocation {
            repo_ref: "senara-solutions/claude-pilot".to_string(),
            number: 119,
        };
        let allowed = crate::webhook_dispatch::dispatchable_repos_display();
        let digest = format_repo_not_dispatchable_pre_digest(&loc, &allowed);
        assert!(digest.contains("senara-solutions/claude-pilot"));
        assert!(digest.contains("119"));
        assert!(digest.contains("DISPATCH REFUSED"));
        for repo in crate::webhook_dispatch::DISPATCHABLE_REPOS {
            assert!(
                digest.contains(repo),
                "refusal must quote {repo} so the operator sees what is allowed"
            );
        }
        assert!(digest.contains("send_message"));
    }

    /// The positive half of the anti-vacuity requirement: a gate that refused
    /// everything would pass the negative tests above.
    #[test]
    fn loop_repos_are_not_caught_by_the_allowlist_gate() {
        for marker in [
            "[GitHub] Issue labeled ready on senara-solutions/mika#2046 — title",
            "[GitHub] Issue labeled ready on mika#2046 — title",
            "[GitHub] Issue labeled ready on senara-solutions/mika-cloud#127 — title",
            "[GitHub] Issue labeled ready on mika-skills#8 — title",
            "[GitHub] Issue labeled ready on mika-platform#58 — title",
        ] {
            let loc = parse_ready_label_location(marker).expect("marker should parse");
            assert!(
                crate::webhook_dispatch::is_dispatchable_repo(&loc.repo_ref),
                "{marker} must still reach dispatch"
            );
        }
    }

    #[test]
    fn spawn_cc_only_repos_are_caught_by_the_allowlist_gate() {
        for marker in [
            "[GitHub] Issue labeled ready on senara-solutions/control-monitor#159 — title",
            "[GitHub] Issue labeled ready on control-monitor#159 — title",
            "[GitHub] Issue labeled ready on senara-solutions/claude-pilot#119 — title",
            "[GitHub] Issue labeled ready on claude-pilot#119 — title",
            "[GitHub] Issue labeled ready on another-org/mika#1 — title",
        ] {
            let loc = parse_ready_label_location(marker).expect("marker should parse");
            assert!(
                !crate::webhook_dispatch::is_dispatchable_repo(&loc.repo_ref),
                "{marker} must be refused before task creation"
            );
        }
    }

    #[test]
    fn parses_owner_repo_form() {
        let loc = parse_ready_label_location(
            "[GitHub] Issue labeled ready on senara-solutions/mika#1384 — title here",
        )
        .expect("parse should succeed");
        assert_eq!(loc.repo_ref, "senara-solutions/mika");
        assert_eq!(loc.number, 1384);
        assert_eq!(loc.owner_repo(), "senara-solutions/mika");
    }

    #[test]
    fn parses_bare_repo_form_with_default_owner() {
        let loc = parse_ready_label_location("[GitHub] Issue labeled ready on mika#999")
            .expect("parse should succeed");
        assert_eq!(loc.repo_ref, "mika");
        assert_eq!(loc.number, 999);
        assert_eq!(loc.owner_repo(), "senara-solutions/mika");
    }

    #[test]
    fn parses_mika_cloud_repo() {
        let loc = parse_ready_label_location(
            "[GitHub] Issue labeled ready on senara-solutions/mika-cloud#127",
        )
        .expect("parse should succeed");
        assert_eq!(loc.owner_repo(), "senara-solutions/mika-cloud");
        assert_eq!(loc.number, 127);
    }

    #[test]
    fn repo_name_strips_owner_prefix() {
        // mika#1593: the dispatch `prompt` must be the bare repo basename, not
        // the owner-qualified form, regardless of the marker's repo_ref shape.
        let owner_qualified = ReadyLabelLocation {
            repo_ref: "senara-solutions/mika".to_string(),
            number: 1,
        };
        assert_eq!(owner_qualified.repo_name(), "mika");

        let bare = ReadyLabelLocation {
            repo_ref: "mika".to_string(),
            number: 1,
        };
        assert_eq!(bare.repo_name(), "mika");

        let cloud = ReadyLabelLocation {
            repo_ref: "senara-solutions/mika-cloud".to_string(),
            number: 1,
        };
        assert_eq!(cloud.repo_name(), "mika-cloud");
    }

    #[test]
    fn pre_digest_prompt_is_bare_for_mika_cloud() {
        // mika#1593: an owner-qualified mika-cloud marker must still emit a bare
        // `mika-cloud#<n>` dispatch prompt.
        let loc = ReadyLabelLocation {
            repo_ref: "senara-solutions/mika-cloud".to_string(),
            number: 50,
        };
        let digest = format_ready_label_pre_digest(
            &loc,
            false,
            "run_claude_pilot_groom",
            "dev-groom",
            "tid",
        );
        assert!(
            digest.contains("\"prompt\": \"mika-cloud#50\""),
            "dispatch prompt must be bare repo#number for owner-qualified mika-cloud ref"
        );
    }

    #[test]
    fn rejects_non_marker_text() {
        assert!(parse_ready_label_location("hello world").is_none());
        assert!(parse_ready_label_location("[GitHub] PR closed: foo").is_none());
        assert!(parse_ready_label_location("[GitHub] Issue labeled bug on mika#1").is_none());
    }

    #[test]
    fn rejects_missing_number() {
        assert!(parse_ready_label_location("[GitHub] Issue labeled ready on mika").is_none());
        assert!(parse_ready_label_location("[GitHub] Issue labeled ready on mika#abc").is_none());
        assert!(parse_ready_label_location("[GitHub] Issue labeled ready on #999").is_none());
    }

    #[test]
    fn pre_digest_names_required_args() {
        let loc = ReadyLabelLocation {
            repo_ref: "senara-solutions/mika".to_string(),
            number: 1384,
        };
        let digest = format_ready_label_pre_digest(
            &loc,
            false,
            "run_claude_pilot_groom",
            "dev-groom",
            "task-uuid-abc",
        );
        // Pre-digest must name the target tool, the skill, the prompt format,
        // and the pre-created task_id literally so the LLM cannot ambiguate.
        assert!(digest.contains("run_claude_pilot_groom"), "names tool");
        assert!(
            digest.contains("\"skill\": \"dev-groom\""),
            "names skill arg"
        );
        assert!(
            digest.contains("\"prompt\": \"mika#1384\""),
            "names prompt arg as bare repo#number (dispatch-lib contract, mika#1593)"
        );
        // The cosmetic marker line still shows the owner-qualified form — only
        // the dispatch `prompt` argument is normalized to bare.
        assert!(
            digest.contains("[GitHub] Issue labeled ready on senara-solutions/mika#1384"),
            "marker line stays owner-qualified"
        );
        assert!(
            digest.contains("\"task_id\": \"task-uuid-abc\""),
            "names task_id arg"
        );
        assert!(
            digest.contains("UNGROOMED"),
            "names ungroomed state for ungroomed input"
        );
        assert!(
            digest.contains("MUST NOT"),
            "names prohibited actions explicitly"
        );
    }

    #[test]
    fn pre_digest_groomed_state_branches_correctly() {
        let loc = ReadyLabelLocation {
            repo_ref: "mika".to_string(),
            number: 1,
        };
        let groomed_digest =
            format_ready_label_pre_digest(&loc, true, "run_claude_pilot", "dev-pilot", "tid");
        let ungroomed_digest = format_ready_label_pre_digest(
            &loc,
            false,
            "run_claude_pilot_groom",
            "dev-groom",
            "tid",
        );

        assert!(groomed_digest.contains("GROOMED"));
        assert!(groomed_digest.contains("run_claude_pilot"));
        assert!(groomed_digest.contains("dev-pilot"));
        assert!(groomed_digest.contains("implement"));

        assert!(ungroomed_digest.contains("UNGROOMED"));
        assert!(ungroomed_digest.contains("run_claude_pilot_groom"));
        assert!(ungroomed_digest.contains("dev-groom"));
        assert!(ungroomed_digest.contains("groom"));
    }

    #[test]
    fn engine_dispatch_pre_digest_does_not_trigger_intent_guard() {
        // AC3 / plan addendum C4-C5: the post-dispatch pre-digest must NOT match
        // the ready-label or unauthorized-webhook trigger predicates, so the
        // `webhook_ready_label_dispatch` INTENT_GUARD does not fire after an
        // engine-side dispatch. The guarantee is structural — the digest starts
        // with `<ready_label_handler>`, not the `[GitHub] Issue labeled ready on`
        // marker that `is_ready_label_dispatch_marker` (a `starts_with` predicate)
        // keys on. No `engine_dispatched` flag threading is required.
        let loc = ReadyLabelLocation {
            repo_ref: "senara-solutions/mika".to_string(),
            number: 1572,
        };
        let digest = format_engine_dispatch_pre_digest(
            &loc,
            true,
            "run_claude_pilot",
            "dev-pilot",
            "tid-1572",
        );

        assert!(
            digest.starts_with("<ready_label_handler>"),
            "engine-dispatch digest must start with the handler tag, not the marker"
        );
        assert!(
            !crate::webhook_dispatch::is_ready_label_dispatch_marker(&digest),
            "engine-dispatch digest must NOT match the ready-label dispatch trigger"
        );
        assert!(
            !crate::webhook_dispatch::is_unauthorized_webhook_dispatch(&digest),
            "engine-dispatch digest must NOT match the unauthorized-webhook trigger"
        );
    }

    #[test]
    fn engine_dispatch_pre_digest_names_task_and_state() {
        let loc = ReadyLabelLocation {
            repo_ref: "mika".to_string(),
            number: 999,
        };
        let groomed = format_engine_dispatch_pre_digest(
            &loc,
            true,
            "run_claude_pilot",
            "dev-pilot",
            "task-abc",
        );
        // AC2: post-dispatch shape — "already fired", not "make the call".
        assert!(groomed.contains("ENGINE-SIDE DISPATCH FIRED"));
        // Guard-safe wording (mika#1572 review): the header must not carry a
        // completion-claim keyword (#483 detect_completion_claim scans
        // `\b(merged|deployed|completed?|shipped)\b`), and the digest must forbid
        // asserting issue state (assert_grounded #1331) — both prevent a wasted
        // self-healing retry on the LLM's acknowledgment turn.
        let header = groomed.lines().nth(1).unwrap_or("").to_lowercase();
        assert!(
            !header.contains("complete")
                && !header.contains("merged")
                && !header.contains("deployed")
                && !header.contains("shipped"),
            "digest header must not prime a completion-claim keyword: {header}"
        );
        assert!(
            groomed.contains("you have verified no outcome"),
            "digest must forbid asserting issue state (assert_grounded safety)"
        );
        assert!(
            groomed.contains("task-abc"),
            "names the pre-created task_id"
        );
        assert!(
            groomed.contains("run_claude_pilot"),
            "names the spawned tool"
        );
        assert!(
            groomed.contains("MUST NOT"),
            "instructs the LLM not to re-dispatch"
        );
        assert!(groomed.contains("GROOMED"));

        let ungroomed = format_engine_dispatch_pre_digest(
            &loc,
            false,
            "run_claude_pilot_groom",
            "dev-groom",
            "task-xyz",
        );
        assert!(ungroomed.contains("UNGROOMED"));
        assert!(ungroomed.contains("run_claude_pilot_groom"));
        assert!(ungroomed.contains("task-xyz"));
    }

    #[test]
    fn handled_and_dispatched_pre_digests_share_guard_safe_prefix() {
        // Both the #1571 prescriptive (Handled) and the #1572 engine-dispatch
        // (Dispatched) pre-digests must share the same guard-safe prefix so
        // neither trips the post-LLM-turn INTENT_GUARD.
        let loc = ReadyLabelLocation {
            repo_ref: "mika".to_string(),
            number: 1,
        };
        let handled =
            format_ready_label_pre_digest(&loc, true, "run_claude_pilot", "dev-pilot", "t");
        let dispatched =
            format_engine_dispatch_pre_digest(&loc, true, "run_claude_pilot", "dev-pilot", "t");
        assert!(handled.starts_with("<ready_label_handler>"));
        assert!(dispatched.starts_with("<ready_label_handler>"));
        assert!(!crate::webhook_dispatch::is_ready_label_dispatch_marker(
            &handled
        ));
        assert!(!crate::webhook_dispatch::is_ready_label_dispatch_marker(
            &dispatched
        ));
    }

    /// AC1 + AC5 — the refusal names the issue, the label found, and the seat
    /// this engine dispatches as, so an avoided collision reads on its own.
    #[test]
    fn seat_mismatch_pre_digest_names_issue_label_and_current_seat() {
        let loc = ReadyLabelLocation {
            repo_ref: "senara-solutions/mika".to_string(),
            number: 2055,
        };
        let verdict = crate::webhook_dispatch::classify_dispatch_seat(["dispatch:ssc"]);
        let digest = format_seat_mismatch_pre_digest(
            &loc,
            &verdict,
            crate::webhook_dispatch::CURRENT_DISPATCH_SEAT,
        );
        assert!(digest.contains("senara-solutions/mika"));
        assert!(digest.contains("2055"));
        assert!(digest.contains("dispatch:ssc"));
        assert!(digest.contains(crate::webhook_dispatch::CURRENT_DISPATCH_SEAT));
        assert!(digest.contains("DISPATCH REFUSED"));
        for seat in crate::webhook_dispatch::KNOWN_DISPATCH_SEATS {
            assert!(
                digest.contains(seat),
                "refusal must quote {seat} so the operator sees the seat vocabulary"
            );
        }
        // Must not match the `webhook_ready_label_dispatch` trigger, or the
        // guard would demand the dispatch this refusal prevents.
        assert!(digest.starts_with("<ready_label_handler>"));
        // The block must close, like every other pre-digest in this file. The
        // tag delimits engine territory in the LLM's context, and `starts_with`
        // alone would not have caught an unclosed one.
        assert!(
            digest.trim_end().ends_with("</ready_label_handler>"),
            "pre-digest must close its block"
        );
        assert!(!crate::webhook_dispatch::is_ready_label_dispatch_marker(
            &digest
        ));
        assert!(digest.contains("send_message"));
    }
}
