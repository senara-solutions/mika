use std::fmt::Write;
use std::path::Path;

use mika_common::team::TeamDefinition;

use super::types::TeamRun;

/// Build the team context injected into the orchestrator's system prompt.
///
/// Includes: team definition, workspace state, instructions for producing
/// task assignments, and any previous review feedback.
pub fn build_orchestrator_context(
    def: &TeamDefinition,
    workspace_listing: &str,
    previous_feedback: Option<&str>,
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
         {feedback_section}\n\
         ## Instructions\n\
         Decompose the goal into tasks for your team members. \
         Respond with a JSON array of task assignments. Each assignment has:\n\
         - \"agent\": the team member's name\n\
         - \"task\": a clear, specific description of what to do\n\
         - \"output_file\": the workspace filename where results should be written\n\
         \n\
         Use `list_workspace` to check the current workspace state before planning.\n\
         \n\
         Respond ONLY with the JSON array. Example:\n\
         [{{\"agent\": \"researcher\", \"task\": \"Research X and write findings\", \"output_file\": \"research.md\"}}]\n",
        team_name = def.team.name,
        mi = def.flow.max_iterations,
    )
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
         3. Do your work and write your results to `{output_file}` using `write_workspace`.\n\
         4. Respond with a brief summary of what you accomplished.\n"
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
        let ctx = build_orchestrator_context(&def, "", None);
        assert!(ctx.contains("ORCHESTRATOR"));
        assert!(ctx.contains("researcher"));
        assert!(ctx.contains("Research topics"));
        // Orchestrator itself should not be listed as a team member
        assert!(!ctx.contains("- **planner**"));
    }

    #[test]
    fn test_orchestrator_context_with_feedback() {
        let def = test_def();
        let ctx = build_orchestrator_context(&def, "", Some("Needs more detail"));
        assert!(ctx.contains("Previous Review Feedback"));
        assert!(ctx.contains("Needs more detail"));
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
            started_at: "2026-02-25T10:00:00Z".to_string(),
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
            started_at: "2026-02-25T10:00:00Z".to_string(),
            ended_at: None,
            deliverable: None,
        };

        let ctx = build_deliverable_context(&run);
        assert!(ctx.contains("FINAL DELIVERABLE"));
        assert!(ctx.contains("Write summary"));
        assert!(ctx.contains("deliverable.md"));
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
