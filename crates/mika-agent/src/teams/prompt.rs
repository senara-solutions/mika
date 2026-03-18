use std::fmt::Write;
use std::path::Path;

use mika_common::team::TeamDefinition;

use crate::db::{TeamRunRow, TeamRunSummary, truncate_chars};

use super::types::TeamRun;

/// Build the team context injected into the orchestrator's system prompt.
///
/// Includes: team definition, workspace state, instructions for producing
/// task assignments, and any previous review feedback.
pub fn build_orchestrator_context(
    def: &TeamDefinition,
    workspace_listing: &str,
    previous_feedback: Option<&str>,
    history: &[TeamRunRow],
    previous_run: Option<&TeamRunSummary>,
) -> String {
    let mut members = String::new();
    for agent in &def.agents {
        if agent.name == def.team.orchestrator {
            continue;
        }
        let _ = writeln!(
            members,
            "- **{}** (role: {}): {}",
            agent.name, agent.role, agent.mandate
        );
    }

    // Build enriched previous run context (most recent) + condensed older runs
    let previous_run_section = previous_run
        .map(build_previous_run_context)
        .unwrap_or_default();

    // Exclude the most recent run from condensed history if we have an enriched summary
    let skip_run_id = previous_run.map(|s| s.run.id.as_str());

    let history_section = if history.is_empty() && previous_run_section.is_empty() {
        String::new()
    } else {
        let mut buf = String::new();

        // First: enriched previous run summary (up to 2500 chars)
        if !previous_run_section.is_empty() {
            buf.push('\n');
            buf.push_str(&previous_run_section);
        }

        // Then: condensed older runs
        let older_runs: Vec<_> = history
            .iter()
            .filter(|r| skip_run_id != Some(r.id.as_str()))
            .collect();

        if !older_runs.is_empty() {
            buf.push_str("\n## Older Runs\n");
            let mut budget = 1500usize;
            for run in older_runs.iter().rev() {
                if budget == 0 {
                    break;
                }
                let entry_start = buf.len();
                let _ = writeln!(buf, "<context type=\"history_goal\">{}</context>", run.goal);
                if let Some(ref d) = run.deliverable {
                    let _ = writeln!(buf, "<context type=\"history_deliverable\">{d}</context>");
                }
                buf.push('\n');
                let entry_len = buf.len() - entry_start;
                budget = budget.saturating_sub(entry_len);
            }
        }

        buf
    };

    let feedback_section = match previous_feedback {
        Some(feedback) => format!(
            "\n## Previous Review Feedback\n\
             The critic rejected the previous iteration:\n\
             {feedback}\n"
        ),
        None => String::new(),
    };

    let workspace_section = if workspace_listing.is_empty() {
        "Workspace is empty (no files yet).".to_string()
    } else {
        workspace_listing.to_string()
    };

    format!(
        "You are the ORCHESTRATOR of team '{team_name}'.\n\
         \n\
         ## Team Members\n\
         {members}\n\
         ## Flow\n\
         Max iterations: {mi}\n\
         \n\
         ## Workspace State\n\
         {workspace_section}\n\
         {history_section}\
         {feedback_section}\n\
         ## Instructions\n\
         If the input is NOT an actionable goal (e.g., a greeting, status question, \
         or casual conversation), respond with: {{\"reply\": \"your response\"}}\n\
         \n\
         For actionable goals, decompose into tasks for your team members. \
         Respond with a JSON array of task assignments. Each assignment has:\n\
         - \"agent\": the team member's name\n\
         - \"task\": a clear, specific description of what to do\n\
         - \"output_file\": the workspace filename where results should be written\n\
         \n\
         Use `list_workspace` to check the current workspace state before planning.\n\
         \n\
         Respond ONLY with compact (single-line) JSON, not pretty-printed. Examples:\n\
         Conversational: {{\"reply\": \"Hey! The team is ready and waiting for a goal.\"}}\n\
         Actionable: [{{\"agent\": \"researcher\", \"task\": \"Research X and write findings\", \"output_file\": \"research.md\"}}]\n",
        team_name = def.team.name,
        mi = def.flow.max_iterations,
    )
}

/// Build a structured context block from a previous team run summary.
///
/// Renders goal, agent results, deliverable, task statuses, and pending tasks
/// into a markdown block. Enforces a 2500-character budget.
pub fn build_previous_run_context(summary: &TeamRunSummary) -> String {
    let run = &summary.run;

    // Format date from ISO 8601 timestamp
    let date = crate::timestamp::parse(&run.started_at)
        .map(|dt| dt.format("%Y-%m-%d").to_string())
        .unwrap_or_else(|_| "unknown".to_string());

    let status_label = if run.status == "completed" {
        format!(
            "completed in {}/{} iterations",
            run.iteration, run.max_iterations
        )
    } else {
        run.status.clone()
    };

    let mut buf = format!(
        "<context type=\"previous_run\">\n## Previous Team Run ({date}, {status_label})\n\n"
    );

    // Goal (truncated to 200 chars)
    let goal = truncate_chars(&run.goal, 200);
    let _ = writeln!(buf, "**Goal:** {goal}\n");

    // Agent Results
    if !summary.agent_results.is_empty() {
        buf.push_str("**Agent Results:**\n");
        for ar in &summary.agent_results {
            let _ = writeln!(buf, "- {}: {}", ar.agent_name, ar.response_preview);
        }
        buf.push('\n');
    }

    // Deliverable (truncated to 500 chars)
    if let Some(ref deliverable) = run.deliverable {
        let d = truncate_chars(deliverable, 500);
        let _ = writeln!(buf, "**Deliverable:** {d}\n");
    }

    // Critic feedback
    if let Some(ref feedback) = summary.critic_feedback {
        let _ = writeln!(buf, "**Critic Feedback:** {feedback}\n");
    }

    // Task statuses
    if !summary.task_statuses.is_empty() {
        buf.push_str("**Task Status:**\n");
        for ts in &summary.task_statuses {
            let _ = writeln!(buf, "- [{}] {}: {}", ts.status, ts.agent_id, ts.label);
        }
        buf.push('\n');
    }

    // Pending tasks (highlighted)
    if !summary.pending_tasks.is_empty() {
        buf.push_str("**Pending from previous run:**\n");
        for pt in &summary.pending_tasks {
            let _ = writeln!(
                buf,
                "- [{}] {}: {} (task {})",
                pt.status, pt.agent_id, pt.label, pt.task_id
            );
        }
        buf.push('\n');
    }

    // Failure reason
    if let Some(ref reason) = run.failure_reason {
        let _ = writeln!(buf, "**Failure reason:** {reason}\n");
    }

    buf.push_str("</context>\n");

    // Enforce 2500-char budget
    if buf.len() > 2500 {
        // Find a safe char boundary to avoid panicking on multi-byte UTF-8
        let mut end = 2490;
        while !buf.is_char_boundary(end) && end > 0 {
            end -= 1;
        }
        buf.truncate(end);
        buf.push_str("...\n</context>\n");
    }

    buf
}

/// Build the team context injected into a specialist agent's system prompt.
///
/// Includes: role, mandate, specific task, and workspace instructions.
pub fn build_specialist_context(
    role: &str,
    mandate: &str,
    task: &str,
    output_file: &str,
) -> String {
    format!(
        "You are working as part of a TEAM.\n\
         \n\
         ## Your Role\n\
         Role: {role}\n\
         Mandate: {mandate}\n\
         \n\
         ## Your Task\n\
         {task}\n\
         \n\
         ## Instructions\n\
         1. Use `list_workspace` to see what shared files are available.\n\
         2. Use `read_workspace` to read any relevant context from other team members.\n\
         3. For deep codebase analysis, use `analyze_codebase` with a specific question \
         instead of reading individual files.\n\
         4. Do your work and write your results to `{output_file}` using `write_workspace`.\n\
         5. Respond with a brief summary of what you accomplished.\n\
         \n\
         ## Important\n\
         - ALWAYS write your deliverable to the workspace using `write_workspace`. \
         Your text response alone is NOT visible to other team members.\n\
         - Prefer deep-analysis tools (like `analyze_codebase`) over reading many files \
         individually — it preserves your tool step budget.\n"
    )
}

/// Build the team context injected into the critic/QA agent's system prompt.
///
/// Includes: review criteria and instructions for approve/reject JSON output.
pub fn build_critic_context(def: &TeamDefinition, run: &TeamRun) -> String {
    let mut tasks_section = String::new();
    for task in &run.tasks {
        let _ = writeln!(
            tasks_section,
            "- **{}** ({}): {} -> {}",
            task.agent, task.role, task.task, task.output_file
        );
    }

    format!(
        "You are the CRITIC/QA reviewer for team '{team_name}'.\n\
         \n\
         ## Goal\n\
         The team is working on: {goal}\n\
         \n\
         ## Tasks Completed\n\
         {tasks_section}\n\
         ## Instructions\n\
         1. Use `list_workspace` to see all output files.\n\
         2. Use `read_workspace` to read each output file.\n\
         3. Evaluate whether the outputs collectively achieve the goal.\n\
         4. Respond with a JSON object:\n\
         \x20  {{\"approved\": true/false, \"feedback\": \"your detailed feedback\"}}\n\
         \n\
         ## Review Criteria\n\
         - Completeness: Does the work address the full goal?\n\
         - Quality: Is the output well-structured and accurate?\n\
         - Coherence: Do the outputs from different agents fit together?\n\
         Iteration: {iteration}/{max_iterations}\n",
        team_name = def.team.name,
        goal = run.goal,
        iteration = run.iteration,
        max_iterations = run.max_iterations,
    )
}

/// Build context for the deliverable step (final synthesis).
pub fn build_deliverable_context(run: &TeamRun) -> String {
    format!(
        "You are producing the FINAL DELIVERABLE for a team run.\n\
         \n\
         ## Goal\n\
         {goal}\n\
         \n\
         ## Instructions\n\
         1. Use `list_workspace` and `read_workspace` to review all team outputs.\n\
         2. Synthesize the outputs into a clear, cohesive final deliverable.\n\
         3. Write the final deliverable to `deliverable.md` using `write_workspace`.\n\
         4. Respond with the deliverable content directly.\n",
        goal = run.goal,
    )
}

/// Get a simple listing of workspace files for prompt injection.
pub fn workspace_listing(workspace_dir: &Path) -> String {
    if !workspace_dir.exists() {
        return String::new();
    }

    let mut files = Vec::new();
    collect_files_simple(workspace_dir, workspace_dir, &mut files);
    files.sort();
    files.join("\n")
}

fn collect_files_simple(base: &Path, dir: &Path, out: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_files_simple(base, &path, out);
        } else if path.is_file()
            && let Ok(relative) = path.strip_prefix(base)
        {
            out.push(format!("- {}", relative.display()));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::teams::types::*;
    use mika_common::team::*;

    fn test_def() -> TeamDefinition {
        TeamDefinition {
            team: TeamMeta {
                name: "dev-team".to_string(),
                orchestrator: "planner".to_string(),
            },
            agents: vec![
                TeamAgent {
                    name: "planner".to_string(),
                    role: "orchestrator".to_string(),
                    mandate: "Decompose goals".to_string(),
                },
                TeamAgent {
                    name: "researcher".to_string(),
                    role: "specialist".to_string(),
                    mandate: "Research topics".to_string(),
                },
                TeamAgent {
                    name: "critic".to_string(),
                    role: "qa".to_string(),
                    mandate: "Review quality".to_string(),
                },
            ],
            flow: TeamFlow { max_iterations: 3 },
        }
    }

    #[test]
    fn test_orchestrator_context_includes_team_members() {
        let def = test_def();
        let ctx = build_orchestrator_context(&def, "", None, &[], None);
        assert!(ctx.contains("ORCHESTRATOR"));
        assert!(ctx.contains("researcher"));
        assert!(ctx.contains("Research topics"));
        // Orchestrator itself should not be listed as a team member
        assert!(!ctx.contains("- **planner**"));
    }

    #[test]
    fn test_orchestrator_context_with_feedback() {
        let def = test_def();
        let ctx = build_orchestrator_context(&def, "", Some("Needs more detail"), &[], None);
        assert!(ctx.contains("Previous Review Feedback"));
        assert!(ctx.contains("Needs more detail"));
    }

    #[test]
    fn test_orchestrator_context_with_history() {
        let def = test_def();
        let history = vec![
            TeamRunRow {
                id: "run-1".to_string(),
                team_name: "dev-team".to_string(),
                goal: "How is the team?".to_string(),
                status: "completed".to_string(),
                failure_reason: None,
                iteration: 0,
                max_iterations: 3,
                deliverable: Some("The team is ready!".to_string()),
                started_at: "2026-01-01T00:00:00Z".to_string(),
                ended_at: Some("2026-01-01T00:00:01Z".to_string()),
                trace_id: None,
            },
            TeamRunRow {
                id: "run-2".to_string(),
                team_name: "dev-team".to_string(),
                goal: "Ping everyone".to_string(),
                status: "completed".to_string(),
                failure_reason: None,
                iteration: 1,
                max_iterations: 3,
                deliverable: Some("Done, pinged all agents.".to_string()),
                started_at: "2026-01-01T00:00:02Z".to_string(),
                ended_at: Some("2026-01-01T00:00:03Z".to_string()),
                trace_id: None,
            },
        ];
        let ctx = build_orchestrator_context(&def, "", None, &history, None);
        assert!(ctx.contains("## Older Runs"));
        // History returned DESC, rendered oldest first — run-2 (index 0) is newer
        assert!(ctx.contains("How is the team?"));
        assert!(ctx.contains("The team is ready!"));
        assert!(ctx.contains("Ping everyone"));
        assert!(ctx.contains("Done, pinged all agents."));
        // Entries should be wrapped in context delimiters
        assert!(ctx.contains("<context type=\"history_goal\">"));
        assert!(ctx.contains("<context type=\"history_deliverable\">"));
    }

    #[test]
    fn test_orchestrator_context_no_history_section_when_empty() {
        let def = test_def();
        let ctx = build_orchestrator_context(&def, "", None, &[], None);
        assert!(!ctx.contains("Conversation History"));
    }

    #[test]
    fn test_specialist_context() {
        let ctx = build_specialist_context(
            "specialist",
            "Research topics",
            "Find best practices for Rust async",
            "research.md",
        );
        assert!(ctx.contains("TEAM"));
        assert!(ctx.contains("specialist"));
        assert!(ctx.contains("Research topics"));
        assert!(ctx.contains("research.md"));
        assert!(ctx.contains("write_workspace"));
        assert!(ctx.contains("analyze_codebase"));
        assert!(ctx.contains("NOT visible to other team members"));
    }

    #[test]
    fn test_critic_context() {
        let run = TeamRun {
            run_id: "test".to_string(),
            team_name: "dev-team".to_string(),
            goal: "Research Rust".to_string(),
            status: RunStatus::Running,
            iteration: 1,
            max_iterations: 3,
            tasks: vec![TaskAssignment {
                agent: "researcher".to_string(),
                role: "specialist".to_string(),
                task: "Research async".to_string(),
                output_file: "research.md".to_string(),
                status: TaskStatus::Completed,
            }],
            started_at: "2025-02-25T11:00:00Z".to_string(),
            ended_at: None,
            deliverable: None,
        };

        let def = test_def();
        let ctx = build_critic_context(&def, &run);
        assert!(ctx.contains("CRITIC"));
        assert!(ctx.contains("Research Rust"));
        assert!(ctx.contains("researcher"));
        assert!(ctx.contains("approved"));
    }

    #[test]
    fn test_deliverable_context() {
        let run = TeamRun {
            run_id: "test".to_string(),
            team_name: "dev-team".to_string(),
            goal: "Write summary".to_string(),
            status: RunStatus::Running,
            iteration: 1,
            max_iterations: 3,
            tasks: vec![],
            started_at: "2025-02-25T11:00:00Z".to_string(),
            ended_at: None,
            deliverable: None,
        };

        let ctx = build_deliverable_context(&run);
        assert!(ctx.contains("FINAL DELIVERABLE"));
        assert!(ctx.contains("Write summary"));
        assert!(ctx.contains("deliverable.md"));
    }

    #[test]
    fn test_orchestrator_context_history_renders_goals_as_is() {
        // Truncation is handled by `load_team_runs_for_prompt` in SQL,
        // so `build_orchestrator_context` renders whatever it receives.
        let def = test_def();
        let goal = "x".repeat(500); // pre-truncated by SQL
        let history = vec![TeamRunRow {
            id: "run-1".to_string(),
            team_name: "dev-team".to_string(),
            goal: goal.clone(),
            status: "completed".to_string(),
            failure_reason: None,
            iteration: 0,
            max_iterations: 3,
            deliverable: Some("result".to_string()),
            started_at: "2026-01-01T00:00:00Z".to_string(),
            ended_at: Some("2026-01-01T00:00:01Z".to_string()),
            trace_id: None,
        }];
        let ctx = build_orchestrator_context(&def, "", None, &history, None);
        assert!(ctx.contains(&goal));
    }

    #[test]
    fn test_build_previous_run_context_completed() {
        use crate::db::{AgentResultSummary, TaskStatusSummary, TeamRunSummary};

        let summary = TeamRunSummary {
            run: TeamRunRow {
                id: "run-1".to_string(),
                team_name: "dev-team".to_string(),
                goal: "Research Rust patterns".to_string(),
                status: "completed".to_string(),
                failure_reason: None,
                iteration: 2,
                max_iterations: 3,
                deliverable: Some("Found 5 key patterns for async Rust".to_string()),
                started_at: "2025-03-03T14:26:40Z".to_string(),
                ended_at: Some("2025-03-03T15:26:40Z".to_string()),
                trace_id: None,
            },
            agent_results: vec![
                AgentResultSummary {
                    agent_name: "researcher".to_string(),
                    response_preview: "Analyzed async patterns in popular crates".to_string(),
                },
                AgentResultSummary {
                    agent_name: "writer".to_string(),
                    response_preview: "Drafted summary of findings".to_string(),
                },
            ],
            task_statuses: vec![TaskStatusSummary {
                agent_id: "researcher".to_string(),
                label: "research-task".to_string(),
                status: "completed".to_string(),
                task_id: "task-1".to_string(),
            }],
            pending_tasks: vec![],
            critic_feedback: Some("Good coverage but needs more examples".to_string()),
        };

        let ctx = build_previous_run_context(&summary);
        assert!(ctx.contains("<context type=\"previous_run\">"));
        assert!(ctx.contains("</context>"));
        assert!(ctx.contains("Previous Team Run"));
        assert!(ctx.contains("completed in 2/3 iterations"));
        assert!(ctx.contains("Research Rust patterns"));
        assert!(ctx.contains("researcher: Analyzed async patterns"));
        assert!(ctx.contains("writer: Drafted summary"));
        assert!(ctx.contains("Found 5 key patterns"));
        assert!(ctx.contains("Good coverage but needs more examples"));
        assert!(ctx.contains("[completed] researcher: research-task"));
    }

    #[test]
    fn test_build_previous_run_context_failed_with_pending() {
        use crate::db::{TaskStatusSummary, TeamRunSummary};

        let summary = TeamRunSummary {
            run: TeamRunRow {
                id: "run-2".to_string(),
                team_name: "dev-team".to_string(),
                goal: "Deploy service".to_string(),
                status: "failed".to_string(),
                failure_reason: Some("Timeout during execution".to_string()),
                iteration: 1,
                max_iterations: 3,
                deliverable: None,
                started_at: "2025-03-03T14:26:40Z".to_string(),
                ended_at: Some("2025-03-03T15:26:40Z".to_string()),
                trace_id: None,
            },
            agent_results: vec![],
            task_statuses: vec![],
            pending_tasks: vec![TaskStatusSummary {
                agent_id: "deployer".to_string(),
                label: "deploy-prod".to_string(),
                status: "pending".to_string(),
                task_id: "task-99".to_string(),
            }],
            critic_feedback: None,
        };

        let ctx = build_previous_run_context(&summary);
        assert!(ctx.contains("failed"));
        assert!(ctx.contains("Timeout during execution"));
        assert!(ctx.contains("Pending from previous run"));
        assert!(ctx.contains("[pending] deployer: deploy-prod (task task-99)"));
    }

    #[test]
    fn test_build_previous_run_context_empty_summary() {
        use crate::db::TeamRunSummary;

        let summary = TeamRunSummary {
            run: TeamRunRow {
                id: "run-3".to_string(),
                team_name: "dev-team".to_string(),
                goal: "Simple goal".to_string(),
                status: "completed".to_string(),
                failure_reason: None,
                iteration: 1,
                max_iterations: 3,
                deliverable: None,
                started_at: "2025-03-03T14:26:40Z".to_string(),
                ended_at: Some("2025-03-03T15:26:40Z".to_string()),
                trace_id: None,
            },
            agent_results: vec![],
            task_statuses: vec![],
            pending_tasks: vec![],
            critic_feedback: None,
        };

        let ctx = build_previous_run_context(&summary);
        assert!(ctx.contains("Previous Team Run"));
        assert!(ctx.contains("Simple goal"));
        // No agent results, tasks, or critic sections
        assert!(!ctx.contains("Agent Results"));
        assert!(!ctx.contains("Task Status"));
        assert!(!ctx.contains("Pending from"));
    }

    #[test]
    fn test_orchestrator_context_with_previous_run() {
        use crate::db::TeamRunSummary;

        let def = test_def();
        let summary = TeamRunSummary {
            run: TeamRunRow {
                id: "run-1".to_string(),
                team_name: "dev-team".to_string(),
                goal: "Previous goal".to_string(),
                status: "completed".to_string(),
                failure_reason: None,
                iteration: 1,
                max_iterations: 3,
                deliverable: Some("Previous deliverable".to_string()),
                started_at: "2025-03-03T14:26:40Z".to_string(),
                ended_at: Some("2025-03-03T15:26:40Z".to_string()),
                trace_id: None,
            },
            agent_results: vec![],
            task_statuses: vec![],
            pending_tasks: vec![],
            critic_feedback: None,
        };

        let ctx = build_orchestrator_context(&def, "", None, &[], Some(&summary));
        assert!(ctx.contains("Previous Team Run"));
        assert!(ctx.contains("Previous goal"));
        assert!(ctx.contains("Previous deliverable"));
    }

    #[test]
    fn test_orchestrator_context_history_excludes_enriched_run() {
        use crate::db::TeamRunSummary;

        let def = test_def();
        let summary = TeamRunSummary {
            run: TeamRunRow {
                id: "run-2".to_string(),
                team_name: "dev-team".to_string(),
                goal: "Latest goal".to_string(),
                status: "completed".to_string(),
                failure_reason: None,
                iteration: 1,
                max_iterations: 3,
                deliverable: Some("Latest deliverable".to_string()),
                started_at: "2026-01-01T00:00:02Z".to_string(),
                ended_at: Some("2026-01-01T00:00:03Z".to_string()),
                trace_id: None,
            },
            agent_results: vec![],
            task_statuses: vec![],
            pending_tasks: vec![],
            critic_feedback: None,
        };

        let history = vec![
            TeamRunRow {
                id: "run-2".to_string(),
                team_name: "dev-team".to_string(),
                goal: "Latest goal".to_string(),
                status: "completed".to_string(),
                failure_reason: None,
                iteration: 1,
                max_iterations: 3,
                deliverable: Some("Latest deliverable".to_string()),
                started_at: "2026-01-01T00:00:02Z".to_string(),
                ended_at: Some("2026-01-01T00:00:03Z".to_string()),
                trace_id: None,
            },
            TeamRunRow {
                id: "run-1".to_string(),
                team_name: "dev-team".to_string(),
                goal: "Older goal".to_string(),
                status: "completed".to_string(),
                failure_reason: None,
                iteration: 1,
                max_iterations: 3,
                deliverable: Some("Older deliverable".to_string()),
                started_at: "2026-01-01T00:00:00Z".to_string(),
                ended_at: Some("2026-01-01T00:00:01Z".to_string()),
                trace_id: None,
            },
        ];

        let ctx = build_orchestrator_context(&def, "", None, &history, Some(&summary));
        // Enriched summary should appear
        assert!(ctx.contains("Previous Team Run"));
        // Older run should appear in Older Runs section
        assert!(ctx.contains("Older goal"));
        // run-2 should NOT be duplicated in the Older Runs section
        let older_section_pos = ctx.find("## Older Runs");
        if let Some(pos) = older_section_pos {
            let older_section = &ctx[pos..];
            assert!(
                !older_section.contains("Latest goal"),
                "run-2 should be excluded from older runs since it's the enriched run"
            );
        }
    }

    #[test]
    fn test_workspace_listing_empty() {
        let tmp = tempfile::tempdir().unwrap();
        let listing = workspace_listing(&tmp.path().join("nonexistent"));
        assert!(listing.is_empty());
    }

    #[test]
    fn test_workspace_listing_with_files() {
        let tmp = tempfile::tempdir().unwrap();
        let ws = tmp.path();
        std::fs::write(ws.join("research.md"), "content").unwrap();
        std::fs::write(ws.join("summary.md"), "content").unwrap();

        let listing = workspace_listing(ws);
        assert!(listing.contains("research.md"));
        assert!(listing.contains("summary.md"));
    }
}
