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

use std::sync::Arc;

use tracing::{info, warn};

use crate::async_db::AsyncDatabase;
use crate::messaging::MessageSender;
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
        if self.repo_ref.contains('/') {
            self.repo_ref.clone()
        } else {
            format!("senara-solutions/{}", self.repo_ref)
        }
    }
}

/// Attempt to handle a `[GitHub] Issue labeled ready on …` webhook structurally
/// before the LLM turn.
///
/// Returns `VerdictAction::Handled` when the handler pre-flighted the dispatch
/// and returns a prescriptive pre-digest. Returns `VerdictAction::Passthrough`
/// when the event is not a ready-label marker, when parsing fails, or when a
/// required precondition (github token, issue body fetch) cannot be satisfied.
///
/// **Reused `VerdictAction` enum** — the action surface is identical
/// (`Handled { pre_digest }` / `Passthrough { enrichment }`), so adding a new
/// enum just for naming would create drift between sibling handlers.
pub async fn try_handle_ready_label_dispatch(
    text: &str,
    db: &AsyncDatabase,
    github_token: Option<&str>,
    _message_sender: Option<&Arc<dyn MessageSender>>,
    session_id: &str,
    trace_id: &str,
) -> VerdictAction {
    // 1. Early-return for non-ready-label messages. Cheapest predicate.
    if !text.starts_with(READY_LABEL_DISPATCH_MARKER) {
        return VerdictAction::Passthrough { enrichment: None };
    }

    // 2. Parse `<repo>#<num>` from the marker text. On parse failure, pass
    //    through and let the existing INTENT_GUARDS path log the issue.
    let location = match parse_ready_label_location(text) {
        Some(loc) => loc,
        None => {
            warn!(
                event = "ready_label_parse_failed",
                text_excerpt = %text.chars().take(120).collect::<String>(),
                "ready_label_handler: could not parse <repo>#<n> from marker — passthrough"
            );
            return VerdictAction::Passthrough { enrichment: None };
        }
    };

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
            return VerdictAction::Passthrough { enrichment: None };
        }
    };

    // 4. Fetch issue body via `gh issue view`. Used to determine groomed-state
    //    via the same predicate the dispatch gate uses (#919, #1108).
    let body = match fetch_issue_body(&location, token).await {
        Ok(b) => b,
        Err(e) => {
            warn!(
                event = "ready_label_body_fetch_failed",
                repo = %location.owner_repo(),
                num = location.number,
                error = %e,
                "ready_label_handler: gh issue view failed — passthrough"
            );
            return VerdictAction::Passthrough { enrichment: None };
        }
    };

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
    //    decision-point from the LLM's path.
    let issue_url = format!(
        "https://github.com/{}/issues/{}",
        location.owner_repo(),
        location.number
    );
    let agent_id = db.agent_id().to_string();
    let task_label = format!("ready-label: {}#{}", location.owner_repo(), location.number);
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
            warn!(
                event = "ready_label_task_create_failed",
                repo = %location.owner_repo(),
                num = location.number,
                error = %e,
                "ready_label_handler: failed to pre-create task — passthrough"
            );
            return VerdictAction::Passthrough { enrichment: None };
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

    info!(
        event = "ready_label_handler_prepared_dispatch",
        repo = %location.owner_repo(),
        number = location.number,
        target_tool,
        target_skill,
        dispatch_class,
        groomed = is_groomed,
        task_id = %task_id,
        "ready_label_handler: pre-flight complete; returning prescriptive pre-digest"
    );

    VerdictAction::Handled {
        pre_digest: format_ready_label_pre_digest(
            &location,
            is_groomed,
            target_tool,
            target_skill,
            &task_id,
        ),
    }
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

/// Fetch the issue body via `gh issue view --json body`. Returns the `body`
/// field on success or a descriptive error string on failure.
async fn fetch_issue_body(loc: &ReadyLabelLocation, token: &str) -> Result<String, String> {
    let owner_repo = loc.owner_repo();
    let number_str = loc.number.to_string();
    let args = [
        "issue",
        "view",
        &number_str,
        "--repo",
        &owner_repo,
        "--json",
        "body",
        "-q",
        ".body",
    ];
    let stdout = run_gh_subprocess(&args, token).await?;
    Ok(stdout)
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
        owner_repo,
        loc.number,
        task_id,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

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
            digest.contains("\"prompt\": \"senara-solutions/mika#1384\""),
            "names prompt arg"
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
}
