//! Write-path functions for operational items.
//!
//! Each subsystem that produces operationally-relevant events has a dedicated
//! function here. All writes use [`Database::upsert_operational_item_by_source`]
//! for idempotency (same source event → same operational item).
//!
//! Chat-message classification (Task vs Goal vs Commitment vs Decision vs
//! Blocker) is deferred to Layer 2 — it requires LLM inference which cannot
//! run inside the same DB transaction as the source-of-truth write.

use crate::db::Database;
use crate::operational::types::*;
use anyhow::Result;
use rusqlite::Transaction;
use tracing::warn;

// ---------------------------------------------------------------------------
// 3.2 Reminders → Task (Scheduled)
// ---------------------------------------------------------------------------

/// Write an operational item for a reminder-type task.
///
/// Called from `create_task()` when `trigger_type = 'reminder'` or the task
/// has a `next_fire_at` set.
pub fn write_reminder_item_tx(
    tx: &Transaction<'_>,
    db: &Database,
    agent_id: &str,
    task_id: &str,
    label: &str,
    next_fire_at: Option<&str>,
) -> Result<String> {
    let item = NewOperationalItem {
        kind: OperationalKind::Task,
        title: label.to_string(),
        status: OperationalStatus::Scheduled,
        owner: Owner::User,
        due_at: next_fire_at.map(|s| s.to_string()),
        confidence: 1.0,
        source_table: Some("tasks".to_string()),
        source_id: Some(task_id.to_string()),
        agent_id: agent_id.to_string(),
        ..Default::default()
    };
    db.upsert_operational_item_by_source_tx(tx, &item)
}

// ---------------------------------------------------------------------------
// 3.3 GitHub Webhooks → Task / Status Updates
// ---------------------------------------------------------------------------

/// Write an operational item for a ready-label webhook event.
pub fn write_ready_label_item(
    db: &Database,
    agent_id: &str,
    issue_number: u64,
    repo: &str,
) -> Result<String> {
    let item = NewOperationalItem {
        kind: OperationalKind::Task,
        title: format!("Issue #{issue_number} ready for dispatch ({repo})"),
        status: OperationalStatus::Now,
        owner: Owner::Agent("mika-dev".to_string()),
        confidence: 1.0,
        source_table: Some("github_issue".to_string()),
        source_id: Some(format!("{repo}#{issue_number}")),
        agent_id: agent_id.to_string(),
        ..Default::default()
    };
    db.upsert_operational_item_by_source(&item)
}

/// Update an existing operational item's status based on a webhook event.
/// Returns Ok(true) if an item was found and updated, Ok(false) if no match.
pub fn update_webhook_item_status(
    db: &Database,
    agent_id: &str,
    source_table: &str,
    source_id: &str,
    new_status: NonTerminalStatus,
) -> Result<bool> {
    if let Some(item) = db.get_operational_item_by_source(agent_id, source_table, source_id)? {
        // Don't update terminal items
        if item.status == OperationalStatus::Done {
            return Ok(false);
        }
        db.update_operational_item_status(&item.id, new_status)?;
        Ok(true)
    } else {
        Ok(false)
    }
}

// ---------------------------------------------------------------------------
// 3.4 Skills (dispatch) → Delegated Task
// ---------------------------------------------------------------------------

/// Write an operational item when a skill dispatch succeeds.
pub fn write_dispatch_item(
    db: &Database,
    agent_id: &str,
    task_id: &str,
    skill_name: &str,
) -> Result<String> {
    let delegate_agent = derive_delegate_agent(skill_name);
    let item = NewOperationalItem {
        kind: OperationalKind::Task,
        title: format!(
            "Dispatched: {skill_name} (task {})",
            &task_id[..8.min(task_id.len())]
        ),
        status: OperationalStatus::Delegated,
        owner: Owner::Agent(delegate_agent),
        confidence: 1.0,
        source_table: Some("tasks".to_string()),
        source_id: Some(task_id.to_string()),
        agent_id: agent_id.to_string(),
        ..Default::default()
    };
    db.upsert_operational_item_by_source(&item)
}

/// Derive the delegate agent name from the skill name.
fn derive_delegate_agent(skill_name: &str) -> String {
    match skill_name {
        "dev-pilot" | "self-dev" => "mika-dev".to_string(),
        "dev-groom" => "mika-arch".to_string(),
        "qa-review" => "mika-qa".to_string(),
        "deploy-mika" | "build-mika" => "mika-dev".to_string(),
        _ => skill_name.to_string(),
    }
}

// ---------------------------------------------------------------------------
// 3.5 Team Runs → Delegated Task
// ---------------------------------------------------------------------------

/// Write an operational item for a team run.
pub fn write_team_run_item(
    db: &Database,
    agent_id: &str,
    run_id: &str,
    goal: &str,
) -> Result<String> {
    let item = NewOperationalItem {
        kind: OperationalKind::Task,
        title: format!("Team run: {goal}"),
        status: OperationalStatus::Delegated,
        owner: Owner::Agent("team".to_string()),
        confidence: 1.0,
        source_table: Some("team_runs".to_string()),
        source_id: Some(run_id.to_string()),
        agent_id: agent_id.to_string(),
        ..Default::default()
    };
    db.upsert_operational_item_by_source(&item)
}

// ---------------------------------------------------------------------------
// 3.6 Callbacks (claude-pilot completion) → Status Transitions
// ---------------------------------------------------------------------------

/// Update operational item status on callback completion.
///
/// If the callback produced a PR URL, the item is completed with the PR as
/// evidence (terminal transition per Decision G). Otherwise the item is
/// moved to `AtRisk` (non-terminal — available for next pilot run).
pub fn write_callback_completion(
    db: &Database,
    agent_id: &str,
    task_id: &str,
    pr_url: Option<&str>,
) -> Result<()> {
    if let Some(item) = db.get_operational_item_by_source(agent_id, "tasks", task_id)? {
        if let Some(url) = pr_url {
            let evidence = EvidenceRef {
                kind: EvidenceRefKind::GithubPr,
                id: url.to_string(),
            };
            if let Err(e) = db.complete_operational_item(&item.id, evidence) {
                warn!(
                    item_id = %item.id,
                    error = %e,
                    "failed to complete operational item on callback"
                );
            }
        } else {
            db.update_operational_item_status(&item.id, NonTerminalStatus::AtRisk)?;
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// 3.7 Manual Task Creation (create_task tool) → Task
// ---------------------------------------------------------------------------

/// Write an operational item for a manually created task.
pub fn write_manual_task_item_tx(
    tx: &Transaction<'_>,
    db: &Database,
    agent_id: &str,
    task_id: &str,
    label: &str,
) -> Result<String> {
    let item = NewOperationalItem {
        kind: OperationalKind::Task,
        title: label.to_string(),
        status: OperationalStatus::Now,
        owner: Owner::User,
        confidence: 1.0,
        source_table: Some("tasks".to_string()),
        source_id: Some(task_id.to_string()),
        agent_id: agent_id.to_string(),
        ..Default::default()
    };
    db.upsert_operational_item_by_source_tx(tx, &item)
}

// ---------------------------------------------------------------------------
// 3.8 mika-arch DECISION NEEDED → Decision
// ---------------------------------------------------------------------------

/// Write an operational item for a mika-arch decision marker.
///
/// The `ArchDecisionMarker` is parsed from architect response text. Detection
/// regex lives here (Layer 1); the trigger site in `tool_execution/` is
/// Layer 3's concern.
pub fn write_arch_decision_item(
    db: &Database,
    agent_id: &str,
    marker: &ArchDecisionMarker,
    session_id: &str,
) -> Result<String> {
    let item = NewOperationalItem {
        kind: OperationalKind::Decision,
        title: format!(
            "Stamp marker {} ({}) on {}",
            marker.id, marker.title, marker.doc_path
        ),
        status: OperationalStatus::Now,
        owner: Owner::User,
        evidence_refs: vec![EvidenceRef {
            kind: EvidenceRefKind::External,
            id: format!("mika-arch-session:{session_id}"),
        }],
        confidence: 1.0,
        source_table: Some("tool_calls".to_string()),
        source_id: Some(marker.tool_call_id.clone()),
        agent_id: agent_id.to_string(),
        ..Default::default()
    };
    db.upsert_operational_item_by_source(&item)
}

/// Parse `DECISION NEEDED` markers from mika-arch response text.
///
/// Pattern: `**DECISION NEEDED — <ID> (<title>):**`
pub fn parse_arch_decision_markers(
    text: &str,
    doc_path: &str,
    tool_call_id: &str,
) -> Vec<ArchDecisionMarker> {
    use regex::Regex;
    use std::sync::LazyLock;

    static RE: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"\*\*DECISION NEEDED\s*[—–-]\s*(\S+)\s*\(([^)]+)\)\s*:\*\*").unwrap()
    });

    RE.captures_iter(text)
        .map(|cap| ArchDecisionMarker {
            id: cap[1].to_string(),
            title: cap[2].to_string(),
            doc_path: doc_path.to_string(),
            tool_call_id: tool_call_id.to_string(),
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derive_delegate_agent_known_skills() {
        assert_eq!(derive_delegate_agent("dev-pilot"), "mika-dev");
        assert_eq!(derive_delegate_agent("dev-groom"), "mika-arch");
        assert_eq!(derive_delegate_agent("qa-review"), "mika-qa");
        assert_eq!(derive_delegate_agent("unknown-skill"), "unknown-skill");
    }

    #[test]
    fn parse_arch_decision_markers_basic() {
        let text = "Some text before\n\
                     **DECISION NEEDED — D1 (API shape for read endpoint):**\n\
                     Some discussion...\n\
                     **DECISION NEEDED — D2 (Schema migration strategy):**\n\
                     More text";
        let markers = parse_arch_decision_markers(text, "docs/plan.md", "tc-123");
        assert_eq!(markers.len(), 2);
        assert_eq!(markers[0].id, "D1");
        assert_eq!(markers[0].title, "API shape for read endpoint");
        assert_eq!(markers[1].id, "D2");
        assert_eq!(markers[1].title, "Schema migration strategy");
    }

    #[test]
    fn parse_arch_decision_markers_empty() {
        let markers = parse_arch_decision_markers("no markers here", "doc.md", "tc-1");
        assert!(markers.is_empty());
    }
}
