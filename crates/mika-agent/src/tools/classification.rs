//! Tool read/write classification for the send_message turn boundary guard (#771).
//!
//! Every tool call is classified as either **read** (state-observing) or **write**
//! (state-changing). The classification drives the post-send_message gating logic:
//! after a `send_message` to the user, only read tools may execute in the same turn.
//!
//! Unknown tools and MCP tools (`mcp__*`) default to **write** (conservative).

/// Returns `true` if the tool with the given name is classified as a write (state-changing) tool.
///
/// This covers all builtin tools registered via `default_tools()` and
/// `management_tools_if_needed()`. Exec handler tools (skill-provided) and MCP tools
/// are handled separately by [`is_write_tool_call`].
pub fn is_write_tool(name: &str) -> bool {
    matches!(
        name,
        // Task mutations
        "create_task"
            | "update_task_status"
            | "cancel_task"
            | "complete_task"
            // Dispatch state mutations
            | "promote_deferred_callback"
            // Reminder mutations
            | "create_reminder"
            | "cancel_reminder"
            // Scheduled task mutations
            | "create_scheduled_task"
            // PR merge
            | "pr_merge_with_gate"
            // Memory mutations
            | "store_fact"
            | "update_fact"
            | "update_core_memory"
            // Skill mutations
            | "create_skill"
            | "delete_skill"
            | "toggle_skill"
            | "update_skill"
            // Config mutation
            | "set_config"
            // File mutations
            | "write_agent_file"
            // Agent/team mutations
            | "create_agent"
            | "create_team"
            | "delete_team"
            | "update_team"
            | "add_team_member"
            | "remove_team_member"
            // Delegation/dispatch
            | "delegate_task"
            | "run_team"
            // Cross-agent invocation
            | "a2a_call"
            // Messaging — second send_message is treated as write by the caller,
            // but the first is not gated by *this* function; the boundary tracker
            // handles the first/subsequent distinction.
            | "send_message"
    )
}

/// Returns `true` if the tool with the given name is classified as a read (state-observing) tool.
pub fn is_read_tool(name: &str) -> bool {
    matches!(
        name,
        "list_tasks"
            | "check_task"
            | "get_task"
            | "list_reminders"
            | "list_scheduled_tasks"
            | "search_memory"
            | "list_skills"
            | "get_config"
            | "read_agent_file"
            | "list_agent_files"
            | "list_agents"
            | "list_teams"
            | "get_team_status"
            | "get_team_history"
            | "query_timeline"
            | "get_session_messages"
            | "list_audit_events"
            | "search_tool_history"
            | "query_knowledge_graph"
            | "resolve_issue_order"
    )
}

/// Classify a tool call as write/read using both the tool name and, for mixed
/// tools like `run_gh`, the input summary content.
///
/// Classification priority:
/// 1. Builtin tools: explicit read/write via [`is_write_tool`] / [`is_read_tool`]
/// 2. `run_gh`: input-summary subcommand discrimination
/// 3. Known read-only handler builtins: `get_documentation`, `gh_read`, `web_search`
/// 4. MCP tools (`mcp__*`): write (conservative)
/// 5. Unknown/exec handler tools: write (conservative)
pub fn is_write_tool_call(name: &str, input_summary: &str) -> bool {
    // Fast path: known builtins
    if is_read_tool(name) {
        return false;
    }
    if is_write_tool(name) {
        return true;
    }

    // Handler builtins with known read-only behavior
    if matches!(
        name,
        "get_documentation" | "gh_read" | "web_search" | "review_skill" | "git_ops"
    ) {
        return false;
    }

    // run_gh: mixed — discriminate by subcommand in input summary
    if name == "run_gh" {
        return is_run_gh_write(input_summary);
    }

    // run_gws: Google Workspace — conservative write
    if name == "run_gws" {
        return true;
    }

    // MCP tools: conservative write
    if name.starts_with("mcp__") {
        return true;
    }

    // Unknown/exec handler tools (run_claude_pilot, deploy_mika, build_mika, etc.):
    // conservative write
    true
}

/// Classify a `run_gh` call as write based on its input summary.
///
/// Write-indicative subcommands: `pr merge`, `pr close`, `pr review`, `pr edit`,
/// `issue create`, `issue close`, `issue edit`, `label create`, `release create`,
/// `api` with non-GET methods.
///
/// When the input summary is too short to determine the subcommand, defaults to write.
fn is_run_gh_write(input_summary: &str) -> bool {
    let lower = input_summary.to_lowercase();

    // Write subcommands — check for known write patterns
    let write_patterns = [
        "pr merge",
        "pr close",
        "pr review",
        "pr edit",
        "issue create",
        "issue close",
        "issue edit",
        "label create",
        "release create",
        "release delete",
    ];

    for pattern in &write_patterns {
        if lower.contains(pattern) {
            return true;
        }
    }

    // gh api: write if non-GET method detected
    if lower.contains("api ") || lower.contains("\"api\"") {
        // Check for explicit method flags
        if lower.contains("-x ") || lower.contains("--method ") {
            // Any explicit method specification is write (GET wouldn't need -X)
            return true;
        }
        // gh api with --input or -f (field) implies mutation
        if lower.contains("--input") || lower.contains("-f ") {
            return true;
        }
        // Default: gh api without method flag is GET (read)
        return false;
    }

    // Known read subcommands — explicit allowlist
    let read_patterns = [
        "pr view",
        "pr list",
        "pr diff",
        "pr checks",
        "pr status",
        "issue view",
        "issue list",
        "issue status",
        "run view",
        "run list",
        "workflow view",
        "workflow list",
        "release view",
        "release list",
        "repo view",
        "search ",
        "label list",
    ];

    for pattern in &read_patterns {
        if lower.contains(pattern) {
            return false;
        }
    }

    // Truncated or unrecognizable summary: default to write (safe direction)
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    // === Builtin tool classification ===

    #[test]
    fn test_write_tools() {
        let write_tools = [
            "create_task",
            "update_task_status",
            "cancel_task",
            "complete_task",
            "create_reminder",
            "cancel_reminder",
            "create_scheduled_task",
            "pr_merge_with_gate",
            "store_fact",
            "update_fact",
            "update_core_memory",
            "create_skill",
            "delete_skill",
            "toggle_skill",
            "update_skill",
            "set_config",
            "write_agent_file",
            "create_agent",
            "create_team",
            "delete_team",
            "update_team",
            "add_team_member",
            "remove_team_member",
            "delegate_task",
            "run_team",
            "a2a_call",
            "send_message",
        ];

        for tool in &write_tools {
            assert!(is_write_tool(tool), "{tool} should be classified as write");
        }
    }

    #[test]
    fn test_read_tools() {
        let read_tools = [
            "list_tasks",
            "check_task",
            "get_task",
            "list_reminders",
            "list_scheduled_tasks",
            "search_memory",
            "list_skills",
            "get_config",
            "read_agent_file",
            "list_agent_files",
            "list_agents",
            "list_teams",
            "get_team_status",
            "get_team_history",
            "query_timeline",
            "get_session_messages",
            "list_audit_events",
            "search_tool_history",
            "query_knowledge_graph",
            "resolve_issue_order",
        ];

        for tool in &read_tools {
            assert!(is_read_tool(tool), "{tool} should be classified as read");
            assert!(
                !is_write_tool_call(tool, ""),
                "{tool} should not be classified as write via is_write_tool_call"
            );
        }
    }

    #[test]
    fn test_every_builtin_tool_has_explicit_classification() {
        // All tools from BUILTIN_TOOL_NAMES must be explicitly classified.
        // This test catches new tools added without classification.
        let all_builtins = crate::tools::BUILTIN_TOOL_NAMES;

        for name in all_builtins {
            let is_read = is_read_tool(name);
            let is_write = is_write_tool(name);
            let is_handler = matches!(
                *name,
                "get_documentation"
                    | "gh_read"
                    | "git_ops"
                    | "review_skill"
                    | "run_gh"
                    | "run_gws"
                    | "web_search"
            );

            assert!(
                is_read || is_write || is_handler,
                "Builtin tool '{name}' has no explicit read/write classification. \
                 Add it to is_write_tool() or is_read_tool() in tools/classification.rs."
            );

            // A tool should not be both read and write
            if is_read {
                assert!(
                    !is_write,
                    "Tool '{name}' is classified as both read AND write"
                );
            }
        }
    }

    // === run_gh subcommand discrimination ===

    #[test]
    fn test_run_gh_read_subcommands() {
        assert!(!is_write_tool_call("run_gh", "gh pr view 123"));
        assert!(!is_write_tool_call("run_gh", "gh pr list --state open"));
        assert!(!is_write_tool_call("run_gh", "gh pr diff 123"));
        assert!(!is_write_tool_call("run_gh", "gh issue view 456"));
        assert!(!is_write_tool_call("run_gh", "gh issue list --milestone 5"));
        assert!(!is_write_tool_call("run_gh", "gh run view 789"));
        assert!(!is_write_tool_call("run_gh", "gh pr checks 123"));
    }

    #[test]
    fn test_run_gh_write_subcommands() {
        assert!(is_write_tool_call("run_gh", "gh pr merge 123"));
        assert!(is_write_tool_call("run_gh", "gh pr close 123"));
        assert!(is_write_tool_call("run_gh", "gh pr review 123 --approve"));
        assert!(is_write_tool_call(
            "run_gh",
            "gh issue create --title 'bug'"
        ));
        assert!(is_write_tool_call("run_gh", "gh issue close 456"));
        assert!(is_write_tool_call("run_gh", "gh issue edit 456"));
        assert!(is_write_tool_call("run_gh", "gh label create bug"));
        assert!(is_write_tool_call("run_gh", "gh release create v1.0"));
    }

    #[test]
    fn test_run_gh_api_discrimination() {
        // GET (default, no method flag) is read
        assert!(!is_write_tool_call(
            "run_gh",
            "gh api repos/owner/repo/pulls/1"
        ));
        // Explicit method is write
        assert!(is_write_tool_call(
            "run_gh",
            "gh api -X PATCH repos/owner/repo/milestones/1"
        ));
        assert!(is_write_tool_call(
            "run_gh",
            "gh api --method POST repos/owner/repo/issues"
        ));
        // Input flag implies mutation
        assert!(is_write_tool_call(
            "run_gh",
            "gh api repos/owner/repo/issues -f title=bug"
        ));
    }

    // === Unknown/MCP tools ===

    #[test]
    fn test_unknown_tools_default_to_write() {
        assert!(is_write_tool_call("some_unknown_tool", ""));
        assert!(is_write_tool_call("run_claude_pilot", ""));
        assert!(is_write_tool_call("run_claude_pilot_groom", ""));
        assert!(is_write_tool_call("deploy_mika", ""));
        assert!(is_write_tool_call("build_mika", ""));
    }

    #[test]
    fn test_mcp_tools_default_to_write() {
        assert!(is_write_tool_call("mcp__slack__send_message", ""));
        assert!(is_write_tool_call("mcp__github__create_issue", ""));
        assert!(is_write_tool_call("mcp__custom__read_data", "")); // even if name suggests read
    }

    // === Handler builtins ===

    #[test]
    fn test_known_read_only_handlers() {
        assert!(!is_write_tool_call("get_documentation", ""));
        assert!(!is_write_tool_call("gh_read", ""));
        assert!(!is_write_tool_call("web_search", ""));
        assert!(!is_write_tool_call("review_skill", ""));
        assert!(!is_write_tool_call("git_ops", ""));
    }

    #[test]
    fn test_run_gh_truncated_summary_defaults_write() {
        // If the summary is truncated before the subcommand is parsable,
        // default to write (safe direction).
        assert!(is_write_tool_call("run_gh", "gh "));
        assert!(is_write_tool_call("run_gh", ""));
    }
}
