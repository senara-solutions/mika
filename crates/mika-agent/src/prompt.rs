use crate::db::{Commitment, CoreMemoryEntry, Task, core_memory_section_names};
use chrono::{DateTime, NaiveTime, Utc};
use mika_common::{agent, team};
use serde::Deserialize;
use std::fmt::Write;
use std::path::Path;

/// Configuration for periodic memory reflection.
#[derive(Debug, Deserialize, Clone)]
pub struct ReflectionConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_reflection_time")]
    pub time: String,
    #[serde(default)]
    pub notify: bool,
    #[serde(default)]
    pub timezone: Option<String>,
}

fn default_reflection_time() -> String {
    "20:00".to_string()
}

impl Default for ReflectionConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            time: default_reflection_time(),
            notify: false,
            timezone: None,
        }
    }
}

impl ReflectionConfig {
    /// Parse the configured time string (HH:MM format) into a NaiveTime.
    /// Returns None if the format is invalid.
    pub fn parse_time(&self) -> Option<NaiveTime> {
        NaiveTime::parse_from_str(&self.time, "%H:%M").ok()
    }
}

/// Agent identity loaded from ~/.mika/identity.toml.
#[derive(Debug, Deserialize, Clone)]
pub struct Identity {
    #[serde(default = "default_name")]
    pub name: String,
    #[serde(default = "default_emoji")]
    pub emoji: String,
    #[serde(default)]
    pub reflection: Option<ReflectionConfig>,
}

fn default_name() -> String {
    "Mika".to_string()
}

fn default_emoji() -> String {
    "✦".to_string()
}

impl Default for Identity {
    fn default() -> Self {
        Self {
            name: default_name(),
            emoji: default_emoji(),
            reflection: None,
        }
    }
}

/// Load identity from ~/.mika/identity.toml.
/// Returns defaults if file is missing or invalid.
pub fn load_identity(home_dir: &Path) -> Identity {
    let path = home_dir.join("identity.toml");
    match std::fs::read_to_string(&path) {
        Ok(content) => toml::from_str(&content).unwrap_or_default(),
        Err(_) => Identity::default(),
    }
}

/// Async version of [`load_identity`] using `tokio::fs` to avoid
/// blocking the async runtime.
pub async fn load_identity_async(home_dir: &Path) -> Identity {
    let path = home_dir.join("identity.toml");
    match tokio::fs::read_to_string(&path).await {
        Ok(content) => toml::from_str(&content).unwrap_or_default(),
        Err(_) => Identity::default(),
    }
}

/// Context needed to build the system prompt.
pub struct PromptContext<'a> {
    pub soul_content: &'a str,
    pub identity: &'a Identity,
    pub core_memory: &'a [CoreMemoryEntry],
    pub is_onboarding: bool,
    pub current_utc: DateTime<Utc>,
    pub timezone: Option<String>,
    /// Global home directory, used to discover teams.
    /// When `None`, the teams section is omitted from the prompt.
    pub global_home_dir: Option<&'a Path>,
    /// The channel this message arrived on (e.g., "telegram", "cli").
    /// When `None`, the channel section is omitted (team agents, tests).
    pub channel_type: Option<&'a str>,
    /// Whether Telegram integration is configured (chat_id exists in customer_config).
    pub telegram_configured: bool,
    /// Per-agent home directory (e.g. `~/.mika/agents/mika/`).
    /// Surfaced in the Tool Usage section so the agent knows write_agent_file's base path.
    pub home_dir: Option<&'a Path>,
    /// When set, this is a callback result turn. Injects a guard section telling the agent
    /// to process the results directly and not spawn new long-running tasks.
    pub callback_context: Option<&'a str>,
}

fn onboarding_prompt() -> String {
    let section_names = core_memory_section_names();
    format!(
        "## First Session\n\
         This is your first conversation with the user. Introduce yourself briefly and warmly. \
         Ask who they are and what they're working on. Use update_core_memory to seed all \
         {} blocks ({}) from their \
         responses. Also use store_fact(category=\"person\") to create a record for the user \
         with just their first name (e.g. name=\"Vincent\") and relationship \"The user\". \
         Keep it to 2-3 natural exchanges, then transition to being helpful \
         with whatever they need.",
        section_names.len(),
        section_names.join(", ")
    )
}

/// Write the soul content section (personality baseline from soul.md).
fn write_soul_section(prompt: &mut String, soul_content: &str) {
    if !soul_content.is_empty() {
        prompt.push_str(soul_content);
        prompt.push_str("\n\n");
    }
}

/// Write the identity section.
fn write_identity_section(prompt: &mut String, identity: &Identity) {
    write!(prompt, "## Identity\nYou are {}.\n\n", identity.name).unwrap();
}

/// Write the current time section with optional timezone.
fn write_time_section(prompt: &mut String, current_utc: DateTime<Utc>, timezone: Option<&str>) {
    prompt.push_str("## Current Time\n");
    writeln!(prompt, "UTC: {}", current_utc.format("%Y-%m-%dT%H:%M:%SZ")).unwrap();
    if let Some(tz) = timezone {
        writeln!(prompt, "User timezone: {tz}").unwrap();
    }
    prompt.push('\n');
}

/// Write the communication channel section.
/// Informs the agent which channel the conversation is on and which integrations are active.
/// Known valid channel types. Unknown channels are silently skipped to prevent
/// prompt injection via a compromised gateway sending arbitrary channel strings.
const VALID_CHANNELS: &[&str] = &["cli", "telegram", "whatsapp", "api"];

fn write_channel_section(
    prompt: &mut String,
    channel_type: Option<&str>,
    telegram_configured: bool,
) {
    // Only include recognized channels
    let valid_channel = channel_type.filter(|ch| VALID_CHANNELS.contains(ch));
    if valid_channel.is_none() && !telegram_configured {
        return;
    }
    prompt.push_str("## Communication Channel\n");
    if let Some(ch) = valid_channel {
        writeln!(prompt, "This conversation is happening via: {ch}").unwrap();
    }
    if telegram_configured {
        prompt.push_str(
            "Telegram integration is active. You can reach the user via Telegram using send_message.\n",
        );
    }
    prompt.push('\n');
}

/// Write the core memory section with `<core-memory>` XML delimiters.
/// An optional `description` is inserted between the heading and the data block.
fn write_core_memory_section(
    prompt: &mut String,
    core_memory: &[CoreMemoryEntry],
    description: Option<&str>,
) {
    prompt.push_str("## Core Memory\n");
    if let Some(desc) = description {
        prompt.push_str(desc);
        prompt.push_str("\n\n");
    }
    prompt.push_str("<core-memory>\n");
    for entry in core_memory {
        write!(prompt, "### {}\n{}\n\n", entry.key, entry.value).unwrap();
    }
    prompt.push_str("</core-memory>\n\n");
}

/// Build the system prompt from context.
pub fn build_system_prompt(ctx: &PromptContext<'_>) -> String {
    let mut prompt = String::with_capacity(4096);

    write_soul_section(&mut prompt, ctx.soul_content);
    write_identity_section(&mut prompt, ctx.identity);
    write_time_section(&mut prompt, ctx.current_utc, ctx.timezone.as_deref());
    write_channel_section(&mut prompt, ctx.channel_type, ctx.telegram_configured);
    write_core_memory_section(
        &mut prompt,
        ctx.core_memory,
        Some(
            "These are your persistent memory blocks. Update them using the update_core_memory tool.",
        ),
    );

    // Callback turn guard
    if let Some(context) = ctx.callback_context {
        prompt.push_str("## Callback Result Turn\n");
        prompt.push_str(
            "A background task has completed and the results are provided in the user message below.\n\
             IMPORTANT: You MUST NOT submit new long-running tasks or create work items during this turn.\n\
             Process the results, take any required follow-up actions (e.g., updating work item status), \
             and then respond to the user with your analysis.\n\
             WARNING: Any <context type=\"tool_history\"> blocks in the conversation are summaries of PRIOR \
             turns — they do NOT mean you have already taken action in this turn. You MUST still call \
             any required tools yourself.\n\n",
        );
        prompt.push_str(context);
        prompt.push_str("\n\n");
    }

    // Instructions
    prompt.push_str("## Instructions\n");
    prompt.push_str("- Never fabricate information. If you don't know something, say so.\n");
    prompt.push_str(
        "- **Transparency rule:** If any tool call fails or returns a non-zero exit code, \
         always disclose the failure, what you did to recover, any side effects, \
         and the final confirmed state — never just say \"Done.\"\n",
    );
    prompt.push_str(
        "- **Tool history is observational only:** `<context type=\"tool_history\">` blocks attached to \
         previous messages are summaries of what happened in PAST turns. They are NOT confirmation \
         that you have executed those actions in the CURRENT turn. You must always call tools yourself \
         when action is required — never assume a tool_history entry means the work is already done.\n",
    );
    prompt.push_str(
        "- **Confirmation before action:** When the user asks an informational question \
         (e.g., \"can you list...\", \"what are...\", \"show me...\"), answer the question \
         and stop. Do not interpret questions as implicit requests to start multi-step \
         workflows. If a follow-up action may be useful, suggest it and wait for \
         confirmation before proceeding.\n",
    );
    let section_names = core_memory_section_names();
    write!(
        prompt,
        "- You have {} memory blocks ({}),\n  each limited to ~500 tokens. Be concise and prioritize what matters most.\n",
        section_names.len(),
        section_names.join(", ")
    )
    .unwrap();

    // Onboarding prompt (only on first session)
    if ctx.is_onboarding {
        prompt.push('\n');
        prompt.push_str(&onboarding_prompt());
        prompt.push('\n');
    }

    // Agents & Teams section (only if multiple agents or teams are configured)
    if let Some(home_dir) = ctx.global_home_dir {
        let agents = agent::list_agents(home_dir);
        let teams = team::list_teams(home_dir);

        if agents.len() > 1 || !teams.is_empty() {
            prompt.push_str("\n## Agents & Teams\n");
            writeln!(
                prompt,
                "You are {} {}. Do not delegate tasks to yourself.\n",
                ctx.identity.emoji, ctx.identity.name
            )
            .unwrap();

            if agents.len() > 1 {
                prompt.push_str(
                    "You can delegate tasks to other agents using `delegate_task`. Available agents:\n",
                );
                for name in &agents {
                    let agent_home = agent::agent_dir(home_dir, name);
                    let identity = load_identity(&agent_home);
                    writeln!(prompt, "- {} ({} {})", name, identity.emoji, identity.name).unwrap();
                }
            }

            if !teams.is_empty() {
                prompt.push_str("You can run team workflows using `run_team`. Available teams:\n");
                for name in &teams {
                    match team::load_team(home_dir, name) {
                        Ok(def) => {
                            writeln!(prompt, "- {} ({} agents)", name, def.agents.len()).unwrap();
                        }
                        Err(_) => {
                            writeln!(prompt, "- {} (unable to load)", name).unwrap();
                        }
                    }
                }
            }

            prompt.push_str(
                "Use `list_agents` for details. Use `get_team_status`/`get_team_history` for run results.\n",
            );
        }

        // Agent creation guidance (always shown when home_dir is available)
        prompt.push_str(
            "\nYou can create new agents using the `create_agent` tool. When creating agents \
             based on real people, use your knowledge to craft an appropriate personality in the \
             `soul` parameter — capture their known communication style, philosophy, and expertise. \
             Use `web_search` if you need to research someone you're less familiar with. The `name` \
             parameter must be lowercase with hyphens (e.g. \"elon-musk\"), while `display_name` is \
             the human-readable name (e.g. \"Elon Musk\").\n",
        );
        prompt.push_str(
            "\nTo create a team, use the `create_team` tool — do NOT write team.toml manually. \
             Provide a `name`, `orchestrator` (must be one of the agents), and an `agents` array \
             where each entry has `name`, `role`, and `mandate`. All agents must already exist \
             (create them first with `create_agent` if needed).\n\
             To modify a team, use `update_team` — you can change the orchestrator, agents, or max_iterations.\n\
             To delete a team, use `delete_team`.\n",
        );
    }

    // Tool usage instructions (builtin tools are always available)
    prompt.push_str("\n## Tool Usage\n");
    prompt.push_str("- Update your core memory when you learn important things about the user.\n");
    prompt.push_str(
        "- When the user mentions a person by name for the first time, store them using \
store_fact(category=\"person\") with their name, relationship, and any context. \
Use only their first name (or first + last if ambiguous). Never prefix with \"User:\", \
\"Mr.\", \"Dr.\", etc. — the relationship field captures the role. \
Core memory tracks key people briefly — the people table is the full record.\n",
    );
    prompt.push_str(
        "- Before asking a clarifying question, check conversation history and search_memory — \
         never re-ask something the user already provided.\n",
    );
    prompt.push_str(
        "- When the user asks for multiple actions in one message, handle ALL of them \
         in the same turn using multiple tool calls. Do not process one and ask about the rest.\n",
    );
    prompt.push_str(
        "- Before creating or storing anything (reminders, facts, people, events), \
         first check existing state using the appropriate query tool (list_reminders, \
         search_memory). If a similar entry already exists, inform the user rather than \
         creating a duplicate. After compaction, conversation history may be summarized — \
         always verify current state through tools rather than relying on memory of past actions.\n",
    );
    prompt.push_str(
        "- When asked about your own configuration, setup files, or how specific parts of you work, \
         check your own files (list_agent_files, read_agent_file) and documentation (get_documentation) \
         before answering. Never guess about your own internals.\n",
    );
    prompt.push_str("- Mark commitments as completed or cancelled using the update_fact tool.\n");
    prompt.push_str(
        "- You can create reminders with create_reminder: one-shot (fire_at in ISO 8601 UTC) or periodic (cron_expr, 6-field cron with seconds first, e.g. '0 0 9 * * 1' for every Monday 9am UTC).\n",
    );
    prompt
        .push_str("- You can list and cancel reminders with list_reminders and cancel_reminder.\n");
    prompt.push_str(
        "- You can create new skills using create_skill to extend your capabilities with custom prompt snippets.\n",
    );
    prompt.push_str(
        "- You have built-in skills (use list_skills to see which). Built-in skills cannot be overwritten.\n",
    );
    prompt.push_str("- You can enable or disable skills with toggle_skill.\n");
    prompt.push_str("- To change whether a skill runs on every turn, use update_skill with always_on. For built-in skills, this persists in the database and survives restarts. Always use this tool rather than inspecting skill files directly.\n");
    prompt.push_str(
        "- You can also update skill descriptions, keywords, and prompts with update_skill.\n",
    );
    prompt.push_str("- Skills may be [built-in], [marketplace] (installed from Git repos via CLI), or [custom] (created locally). You can delete marketplace and custom skills.\n");
    prompt.push_str("- You can permanently remove custom skills with delete_skill. Built-in skills cannot be deleted.\n");
    prompt.push_str(
        "- You can read and update customer config (timezone, chat_id, thinking_level) with get_config and set_config.\n",
    );
    prompt.push_str(
        "- Tools may return images (screenshots, image files); you will see and can describe their contents.\n",
    );
    prompt.push_str(
        "- When a tool produces an image file path (e.g., screenshot saved to /path/to/image.png), use read_agent_file on that path to view the image contents.\n",
    );
    prompt.push_str(
        "- You can delegate tasks to specialized agents with delegate_task when other agents are configured.\n",
    );
    prompt.push_str(
        "- Use create_work_item to track significant pieces of work (feature implementations, \
         research projects, items waiting on external input). Check list_work_items before creating \
         to avoid duplicates. Use update_work_item_status to progress work items through their lifecycle \
         (pending → in_progress → blocked → completed).\n",
    );
    prompt.push_str(
        "- **Delegation Rule:** Before delegating any implementation work (via delegate_task \
         or long-running skills), you MUST first create a work item using create_work_item, then pass \
         the work_item_id to the delegation tool. The tool will reject calls without a valid work_item_id.\n",
    );
    prompt.push_str(
        "- Some tools are long-running and return a task ID instead of immediate results. \
         When this happens, inform the user that a background task is running and you'll follow up \
         when results arrive. Do not retry the tool.\n",
    );
    if let Some(home) = ctx.home_dir {
        writeln!(
            prompt,
            "- You can write files to your home directory with write_agent_file. \
             Your home directory is {} — all paths are relative to this directory (~ is accepted as a prefix, e.g. '~/notes.md'). \
             For example, to write identity.toml at the root of your home, use path 'identity.toml'. \
             If the file exists, you must review the current content and call again with confirm: true to overwrite.",
            home.display()
        )
        .unwrap();
        writeln!(
            prompt,
            "- You can read files from your home directory with read_agent_file (path relative to {}). \
             Files larger than 100 KB are rejected.",
            home.display()
        )
        .unwrap();
    } else {
        prompt.push_str(
            "- You can write files to your home directory with write_agent_file. Paths are relative to your home. \
             If the file exists, you must review the current content and call again with confirm: true to overwrite.\n",
        );
        prompt.push_str(
            "- You can read files from your home directory with read_agent_file (relative paths only). \
             Files larger than 100 KB are rejected.\n",
        );
    }
    prompt.push_str(
        "- You can list files in your home directory with list_agent_files. \
         Omit path or pass an empty string to list the root. Pass a relative subdirectory path to list that directory.\n",
    );

    prompt
}

/// Context for building a silent mode (heartbeat/reminder/reflection) system prompt.
pub struct SilentPromptContext<'a> {
    pub soul_content: &'a str,
    pub identity: &'a Identity,
    pub core_memory: &'a [CoreMemoryEntry],
    pub pending_commitments: &'a [Commitment],
    pub trigger_context: &'a str,
    pub current_utc: DateTime<Utc>,
    pub timezone: Option<String>,
    /// Whether Telegram integration is configured for outbound delivery.
    pub telegram_configured: bool,
    /// Whether a message sender is available for outbound delivery.
    /// When false, the prompt omits instructions to use `send_message`.
    pub has_message_sender: bool,
    /// Pre-formatted digest of today's conversations (reflection mode only).
    pub recent_conversations: Option<&'a str>,
    /// Pre-formatted digest of today's audit events (reflection mode only).
    pub recent_audit_events: Option<&'a str>,
    /// Agent home directory. When set, file tool instructions include the absolute path.
    pub home_dir: Option<&'a std::path::Path>,
    /// Pending/in_progress/blocked work items for heartbeat awareness.
    pub pending_work_items: &'a [Task],
}

/// Build a system prompt for silent mode (heartbeat/reminder).
/// The agent's text output is NOT delivered — it must use send_message to contact the user.
pub fn build_silent_prompt(ctx: &SilentPromptContext<'_>) -> String {
    let mut prompt = String::with_capacity(4096);

    write_soul_section(&mut prompt, ctx.soul_content);
    write_identity_section(&mut prompt, ctx.identity);
    write_time_section(&mut prompt, ctx.current_utc, ctx.timezone.as_deref());
    write_channel_section(&mut prompt, None, ctx.telegram_configured);
    write_core_memory_section(&mut prompt, ctx.core_memory, None);

    // Pending commitments
    if !ctx.pending_commitments.is_empty() {
        prompt.push_str("## Pending Commitments\n");
        prompt.push_str("<commitments>\n");
        for c in ctx.pending_commitments {
            let due = c.due_date.as_deref().unwrap_or("no due date");
            writeln!(prompt, "- {} (due: {})", c.description, due).unwrap();
        }
        prompt.push_str("</commitments>\n\n");
    }

    // Silent mode instructions
    prompt.push_str("## Silent Mode\n");
    if ctx.has_message_sender {
        prompt.push_str(
            "You are in SILENT MODE. Your text output is NOT delivered to the user.\n\
             Use the send_message tool to contact the user. If you have nothing worthwhile \
             to say, simply respond with a brief internal note and do NOT call send_message.\n\n",
        );
    } else {
        prompt.push_str(
            "You are in SILENT MODE. Your text output is NOT delivered to the user.\n\
             No outbound messaging channel is configured, so you cannot contact the user.\n\
             Perform any background maintenance (memory updates, fact storage) silently.\n\n",
        );
    }

    // Reflection context (today's conversations and audit events)
    if let Some(conversations) = ctx.recent_conversations.filter(|c| !c.is_empty()) {
        prompt.push_str("## Today's Conversations\n");
        prompt.push_str("<conversations>\n");
        prompt.push_str(conversations);
        prompt.push_str("\n</conversations>\n\n");
    }
    if let Some(events) = ctx.recent_audit_events.filter(|e| !e.is_empty()) {
        prompt.push_str("## Recent Audit Events\n");
        prompt.push_str("<audit-events>\n");
        prompt.push_str(events);
        prompt.push_str("\n</audit-events>\n\n");
    }

    // File tools — mention home-scoped file tools so heartbeat agents can discover them
    prompt.push_str("## File Tools\n");
    if let Some(home) = ctx.home_dir {
        writeln!(
            prompt,
            "- read_agent_file: Read a file from your home directory ({}). Paths are relative to that directory.\n\
             - list_agent_files: List files and directories in your home directory. Omit path to list the root.",
            home.display()
        )
        .unwrap();
    } else {
        prompt.push_str(
            "- read_agent_file: Read a file from your home directory. Paths are relative to your home.\n\
             - list_agent_files: List files and directories in your home directory. Omit path to list the root.\n",
        );
    }
    prompt.push('\n');

    // Pending work items (labels sanitized to prevent prompt injection)
    if !ctx.pending_work_items.is_empty() {
        prompt.push_str("## Pending Work Items\n");
        prompt.push_str("<pending-work-items>\n");
        for item in ctx.pending_work_items {
            let age_days = (ctx.current_utc.timestamp() - item.created_at) / 86400;
            // Sanitize label: truncate to 200 chars and strip angle brackets
            let label = item.label.chars().take(200).collect::<String>();
            let label = label.replace(['<', '>'], "");
            let ref_url = item
                .reference_url
                .as_deref()
                .map(|u| {
                    let u = u.chars().take(200).collect::<String>();
                    format!(" ref:{}", u.replace(['<', '>'], ""))
                })
                .unwrap_or_default();
            writeln!(
                prompt,
                "- [{status}] {id}: {label} (age: {age_days}d{ref_url})",
                status = item.status,
                id = item.id,
            )
            .unwrap();
        }
        prompt.push_str("</pending-work-items>\n");
        prompt.push_str("Review these work items. If any are stale or need attention, consider notifying the user.\n\n");
    }

    // Trigger-specific context
    prompt.push_str("## Trigger\n");
    prompt.push_str(ctx.trigger_context);
    prompt.push('\n');

    prompt
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_identity() -> Identity {
        Identity {
            name: "Mika".to_string(),
            emoji: "✦".to_string(),
            reflection: None,
        }
    }

    fn test_time() -> DateTime<Utc> {
        "2026-02-24T12:00:00Z".parse().unwrap()
    }

    fn test_core_memory() -> Vec<CoreMemoryEntry> {
        vec![
            CoreMemoryEntry {
                key: "user_summary".to_string(),
                value: "Loves coffee.".to_string(),
                token_count: 3,
                updated_at: "2026-01-01".to_string(),
            },
            CoreMemoryEntry {
                key: "self_model".to_string(),
                value: "I am Mika. No interaction history yet.".to_string(),
                token_count: 8,
                updated_at: "2026-01-01".to_string(),
            },
        ]
    }

    #[test]
    fn test_prompt_includes_soul_content() {
        let identity = test_identity();
        let memory = test_core_memory();
        let ctx = PromptContext {
            soul_content: "You are a sharp, proactive executive assistant.",
            identity: &identity,
            core_memory: &memory,
            is_onboarding: false,
            current_utc: test_time(),
            timezone: None,
            global_home_dir: None,
            channel_type: None,
            telegram_configured: false,
            home_dir: None,
            callback_context: None,
        };

        let prompt = build_system_prompt(&ctx);
        assert!(prompt.starts_with("You are a sharp, proactive executive assistant."));
    }

    #[test]
    fn test_prompt_includes_all_core_memory_blocks() {
        let identity = test_identity();
        let memory = test_core_memory();
        let ctx = PromptContext {
            soul_content: "",
            identity: &identity,
            core_memory: &memory,
            is_onboarding: false,
            current_utc: test_time(),
            timezone: None,
            global_home_dir: None,
            channel_type: None,
            telegram_configured: false,
            home_dir: None,
            callback_context: None,
        };

        let prompt = build_system_prompt(&ctx);
        assert!(prompt.contains("### user_summary"));
        assert!(prompt.contains("Loves coffee."));
        assert!(prompt.contains("### self_model"));
        assert!(prompt.contains("I am Mika. No interaction history yet."));
    }

    #[test]
    fn test_prompt_includes_identity_name() {
        let identity = Identity {
            name: "TestBot".to_string(),
            emoji: "🤖".to_string(),
            reflection: None,
        };
        let ctx = PromptContext {
            soul_content: "",
            identity: &identity,
            core_memory: &[],
            is_onboarding: false,
            current_utc: test_time(),
            timezone: None,
            global_home_dir: None,
            channel_type: None,
            telegram_configured: false,
            home_dir: None,
            callback_context: None,
        };

        let prompt = build_system_prompt(&ctx);
        assert!(prompt.contains("You are TestBot."));
    }

    #[test]
    fn test_onboarding_prompt_injected() {
        let identity = test_identity();
        let ctx = PromptContext {
            soul_content: "",
            identity: &identity,
            core_memory: &[],
            is_onboarding: true,
            current_utc: test_time(),
            timezone: None,
            global_home_dir: None,
            channel_type: None,
            telegram_configured: false,
            home_dir: None,
            callback_context: None,
        };

        let prompt = build_system_prompt(&ctx);
        assert!(prompt.contains("## First Session"));
        assert!(prompt.contains("Introduce yourself briefly"));
    }

    #[test]
    fn test_no_onboarding_for_returning_user() {
        let identity = test_identity();
        let ctx = PromptContext {
            soul_content: "",
            identity: &identity,
            core_memory: &[],
            is_onboarding: false,
            current_utc: test_time(),
            timezone: None,
            global_home_dir: None,
            channel_type: None,
            telegram_configured: false,
            home_dir: None,
            callback_context: None,
        };

        let prompt = build_system_prompt(&ctx);
        assert!(!prompt.contains("## First Session"));
    }

    #[test]
    fn test_load_identity_parses_toml() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("identity.toml"),
            "name = \"Agent X\"\nemoji = \"🕶\"\n",
        )
        .unwrap();

        let identity = load_identity(tmp.path());
        assert_eq!(identity.name, "Agent X");
        assert_eq!(identity.emoji, "🕶");
    }

    #[test]
    fn test_load_identity_defaults_if_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let identity = load_identity(tmp.path());
        assert_eq!(identity.name, "Mika");
        assert_eq!(identity.emoji, "✦");
    }

    #[test]
    fn test_prompt_empty_soul_no_extra_whitespace() {
        let identity = test_identity();
        let ctx = PromptContext {
            soul_content: "",
            identity: &identity,
            core_memory: &[],
            is_onboarding: false,
            current_utc: test_time(),
            timezone: None,
            global_home_dir: None,
            channel_type: None,
            telegram_configured: false,
            home_dir: None,
            callback_context: None,
        };

        let prompt = build_system_prompt(&ctx);
        // Should start directly with Identity section when soul is empty
        assert!(prompt.starts_with("## Identity"));
    }

    #[test]
    fn test_silent_prompt_heartbeat() {
        let identity = test_identity();
        let memory = test_core_memory();
        let ctx = SilentPromptContext {
            soul_content: "",
            identity: &identity,
            core_memory: &memory,
            pending_commitments: &[],
            trigger_context: "This is a HEARTBEAT check-in.",
            current_utc: test_time(),
            timezone: None,
            telegram_configured: false,
            has_message_sender: true,
            recent_conversations: None,
            recent_audit_events: None,
            home_dir: None,
            pending_work_items: &[],
        };

        let prompt = build_silent_prompt(&ctx);
        assert!(prompt.contains("## Silent Mode"));
        assert!(prompt.contains("send_message"));
        assert!(prompt.contains("HEARTBEAT check-in"));
        assert!(prompt.contains("## Core Memory"));
    }

    #[test]
    fn test_silent_prompt_reminder() {
        let identity = test_identity();
        let ctx = SilentPromptContext {
            soul_content: "",
            identity: &identity,
            core_memory: &[],
            pending_commitments: &[],
            trigger_context: "REMINDER: Call the dentist",
            current_utc: test_time(),
            timezone: None,
            telegram_configured: false,
            has_message_sender: true,
            recent_conversations: None,
            recent_audit_events: None,
            home_dir: None,
            pending_work_items: &[],
        };

        let prompt = build_silent_prompt(&ctx);
        assert!(prompt.contains("Call the dentist"));
        assert!(prompt.contains("NOT delivered"));
    }

    #[test]
    fn test_silent_prompt_includes_commitments() {
        use crate::db::Commitment;
        let identity = test_identity();
        let commitments = vec![Commitment {
            id: 1,
            description: "Review budget".to_string(),
            status: "pending".to_string(),
            due_date: Some("2026-03-01".to_string()),
            person_id: None,
            created_at: "2026-02-24".to_string(),
            completed_at: None,
        }];
        let ctx = SilentPromptContext {
            soul_content: "",
            identity: &identity,
            core_memory: &[],
            pending_commitments: &commitments,
            trigger_context: "Heartbeat",
            current_utc: test_time(),
            timezone: None,
            telegram_configured: false,
            has_message_sender: true,
            recent_conversations: None,
            recent_audit_events: None,
            home_dir: None,
            pending_work_items: &[],
        };

        let prompt = build_silent_prompt(&ctx);
        assert!(prompt.contains("Review budget"));
        assert!(prompt.contains("2026-03-01"));
    }

    #[test]
    fn test_prompt_includes_current_time() {
        let identity = test_identity();
        let ctx = PromptContext {
            soul_content: "",
            identity: &identity,
            core_memory: &[],
            is_onboarding: false,
            current_utc: test_time(),
            timezone: None,
            global_home_dir: None,
            channel_type: None,
            telegram_configured: false,
            home_dir: None,
            callback_context: None,
        };

        let prompt = build_system_prompt(&ctx);
        assert!(prompt.contains("## Current Time"));
        assert!(prompt.contains("UTC: 2026-02-24T12:00:00Z"));
        assert!(!prompt.contains("User timezone"));
    }

    #[test]
    fn test_prompt_includes_timezone_when_set() {
        let identity = test_identity();
        let ctx = PromptContext {
            soul_content: "",
            identity: &identity,
            core_memory: &[],
            is_onboarding: false,
            current_utc: test_time(),
            timezone: Some("+08:00".to_string()),
            global_home_dir: None,
            channel_type: None,
            telegram_configured: false,
            home_dir: None,
            callback_context: None,
        };

        let prompt = build_system_prompt(&ctx);
        assert!(prompt.contains("UTC: 2026-02-24T12:00:00Z"));
        assert!(prompt.contains("User timezone: +08:00"));
    }

    #[test]
    fn test_prompt_includes_tool_usage_section() {
        let identity = test_identity();
        let ctx = PromptContext {
            soul_content: "",
            identity: &identity,
            core_memory: &[],
            is_onboarding: false,
            current_utc: test_time(),
            timezone: None,
            global_home_dir: None,
            channel_type: None,
            telegram_configured: false,
            home_dir: None,
            callback_context: None,
        };

        let prompt = build_system_prompt(&ctx);
        // Builtin tool instructions are always in the base prompt
        assert!(prompt.contains("## Tool Usage"));
        assert!(prompt.contains("search_memory"));
        assert!(prompt.contains("create_reminder"));
        assert!(prompt.contains("update_fact"));
        // Base instruction also present
        assert!(prompt.contains("Never fabricate information"));
        // Skill awareness line
        assert!(prompt.contains("built-in skills"));
        assert!(prompt.contains("list_skills"));
        assert!(prompt.contains("toggle_skill"));
        assert!(prompt.contains("update_skill"));
        assert!(prompt.contains("delete_skill"));
    }

    #[test]
    fn test_prompt_wraps_core_memory_in_xml_tags() {
        let identity = test_identity();
        let memory = test_core_memory();
        let ctx = PromptContext {
            soul_content: "",
            identity: &identity,
            core_memory: &memory,
            is_onboarding: false,
            current_utc: test_time(),
            timezone: None,
            global_home_dir: None,
            channel_type: None,
            telegram_configured: false,
            home_dir: None,
            callback_context: None,
        };

        let prompt = build_system_prompt(&ctx);
        assert!(prompt.contains("<core-memory>"));
        assert!(prompt.contains("</core-memory>"));
    }

    #[test]
    fn test_silent_prompt_wraps_commitments_in_xml_tags() {
        use crate::db::Commitment;
        let identity = test_identity();
        let commitments = vec![Commitment {
            id: 1,
            description: "Review budget".to_string(),
            status: "pending".to_string(),
            due_date: Some("2026-03-01".to_string()),
            person_id: None,
            created_at: "2026-02-24".to_string(),
            completed_at: None,
        }];
        let ctx = SilentPromptContext {
            soul_content: "",
            identity: &identity,
            core_memory: &[],
            pending_commitments: &commitments,
            trigger_context: "Heartbeat",
            current_utc: test_time(),
            timezone: None,
            telegram_configured: false,
            has_message_sender: true,
            recent_conversations: None,
            recent_audit_events: None,
            home_dir: None,
            pending_work_items: &[],
        };

        let prompt = build_silent_prompt(&ctx);
        assert!(prompt.contains("<commitments>"));
        assert!(prompt.contains("</commitments>"));
        assert!(prompt.contains("Review budget"));
    }

    #[test]
    fn test_silent_prompt_includes_current_time() {
        let identity = test_identity();
        let ctx = SilentPromptContext {
            soul_content: "",
            identity: &identity,
            core_memory: &[],
            pending_commitments: &[],
            trigger_context: "Heartbeat",
            current_utc: test_time(),
            timezone: Some("-05:00".to_string()),
            telegram_configured: false,
            has_message_sender: true,
            recent_conversations: None,
            recent_audit_events: None,
            home_dir: None,
            pending_work_items: &[],
        };

        let prompt = build_silent_prompt(&ctx);
        assert!(prompt.contains("## Current Time"));
        assert!(prompt.contains("UTC: 2026-02-24T12:00:00Z"));
        assert!(prompt.contains("User timezone: -05:00"));
    }

    #[test]
    fn test_prompt_includes_teams_when_configured() {
        let tmp = tempfile::tempdir().unwrap();
        let team_dir = tmp.path().join("teams").join("dev-team");
        std::fs::create_dir_all(&team_dir).unwrap();
        std::fs::write(
            team_dir.join("team.toml"),
            r#"
[team]
name = "dev-team"
orchestrator = "planner"

[[agents]]
name = "planner"
role = "orchestrator"
mandate = "Plan tasks"

[[agents]]
name = "coder"
role = "specialist"
mandate = "Write code"

[flow]
max_iterations = 3
"#,
        )
        .unwrap();

        let identity = test_identity();
        let ctx = PromptContext {
            soul_content: "",
            identity: &identity,
            core_memory: &[],
            is_onboarding: false,
            current_utc: test_time(),
            timezone: None,
            global_home_dir: Some(tmp.path()),
            channel_type: None,
            telegram_configured: false,
            home_dir: None,
            callback_context: None,
        };

        let prompt = build_system_prompt(&ctx);
        assert!(prompt.contains("## Agents & Teams"));
        assert!(prompt.contains("run_team"));
        assert!(prompt.contains("dev-team (2 agents)"));
    }

    #[test]
    fn test_prompt_includes_agents_when_multiple() {
        let tmp = tempfile::tempdir().unwrap();

        // Create two agents
        let mika_dir = tmp.path().join("agents").join("mika");
        std::fs::create_dir_all(&mika_dir).unwrap();
        std::fs::write(mika_dir.join("config.toml"), "# config").unwrap();
        std::fs::write(
            mika_dir.join("identity.toml"),
            "name = \"Mika\"\nemoji = \"✦\"\n",
        )
        .unwrap();

        let researcher_dir = tmp.path().join("agents").join("researcher");
        std::fs::create_dir_all(&researcher_dir).unwrap();
        std::fs::write(researcher_dir.join("config.toml"), "# config").unwrap();
        std::fs::write(
            researcher_dir.join("identity.toml"),
            "name = \"Rex\"\nemoji = \"🔬\"\n",
        )
        .unwrap();

        let identity = test_identity();
        let ctx = PromptContext {
            soul_content: "",
            identity: &identity,
            core_memory: &[],
            is_onboarding: false,
            current_utc: test_time(),
            timezone: None,
            global_home_dir: Some(tmp.path()),
            channel_type: None,
            telegram_configured: false,
            home_dir: None,
            callback_context: None,
        };

        let prompt = build_system_prompt(&ctx);
        assert!(prompt.contains("## Agents & Teams"));
        assert!(prompt.contains("delegate_task"));
        assert!(prompt.contains("mika (✦ Mika)"));
        assert!(prompt.contains("researcher (🔬 Rex)"));
    }

    #[test]
    fn test_prompt_omits_agents_teams_when_single_agent_no_teams() {
        let tmp = tempfile::tempdir().unwrap();

        // Single agent only
        let mika_dir = tmp.path().join("agents").join("mika");
        std::fs::create_dir_all(&mika_dir).unwrap();
        std::fs::write(mika_dir.join("config.toml"), "# config").unwrap();

        let identity = test_identity();
        let ctx = PromptContext {
            soul_content: "",
            identity: &identity,
            core_memory: &[],
            is_onboarding: false,
            current_utc: test_time(),
            timezone: None,
            global_home_dir: Some(tmp.path()),
            channel_type: None,
            telegram_configured: false,
            home_dir: None,
            callback_context: None,
        };

        let prompt = build_system_prompt(&ctx);
        assert!(!prompt.contains("## Agents & Teams"));
    }

    #[test]
    fn test_prompt_omits_agents_teams_when_none_configured() {
        let tmp = tempfile::tempdir().unwrap();
        let identity = test_identity();
        let ctx = PromptContext {
            soul_content: "",
            identity: &identity,
            core_memory: &[],
            is_onboarding: false,
            current_utc: test_time(),
            timezone: None,
            global_home_dir: Some(tmp.path()),
            channel_type: None,
            telegram_configured: false,
            home_dir: None,
            callback_context: None,
        };

        let prompt = build_system_prompt(&ctx);
        assert!(!prompt.contains("## Agents & Teams"));
    }

    #[test]
    fn test_prompt_omits_agents_teams_when_home_dir_none() {
        let identity = test_identity();
        let ctx = PromptContext {
            soul_content: "",
            identity: &identity,
            core_memory: &[],
            is_onboarding: false,
            current_utc: test_time(),
            timezone: None,
            global_home_dir: None,
            channel_type: None,
            telegram_configured: false,
            home_dir: None,
            callback_context: None,
        };

        let prompt = build_system_prompt(&ctx);
        assert!(!prompt.contains("## Agents & Teams"));
    }

    #[test]
    fn test_prompt_includes_channel_section_for_telegram() {
        let identity = test_identity();
        let ctx = PromptContext {
            soul_content: "",
            identity: &identity,
            core_memory: &[],
            is_onboarding: false,
            current_utc: test_time(),
            timezone: None,
            global_home_dir: None,
            channel_type: Some("telegram"),
            telegram_configured: true,
            home_dir: None,
            callback_context: None,
        };

        let prompt = build_system_prompt(&ctx);
        assert!(prompt.contains("## Communication Channel"));
        assert!(prompt.contains("This conversation is happening via: telegram"));
        assert!(prompt.contains("Telegram integration is active"));
    }

    #[test]
    fn test_prompt_includes_channel_section_for_cli() {
        let identity = test_identity();
        let ctx = PromptContext {
            soul_content: "",
            identity: &identity,
            core_memory: &[],
            is_onboarding: false,
            current_utc: test_time(),
            timezone: None,
            global_home_dir: None,
            channel_type: Some("cli"),
            telegram_configured: false,
            home_dir: None,
            callback_context: None,
        };

        let prompt = build_system_prompt(&ctx);
        assert!(prompt.contains("## Communication Channel"));
        assert!(prompt.contains("This conversation is happening via: cli"));
        assert!(!prompt.contains("Telegram integration is active"));
    }

    #[test]
    fn test_prompt_omits_channel_section_when_none() {
        let identity = test_identity();
        let ctx = PromptContext {
            soul_content: "",
            identity: &identity,
            core_memory: &[],
            is_onboarding: false,
            current_utc: test_time(),
            timezone: None,
            global_home_dir: None,
            channel_type: None,
            telegram_configured: false,
            home_dir: None,
            callback_context: None,
        };

        let prompt = build_system_prompt(&ctx);
        assert!(!prompt.contains("## Communication Channel"));
    }

    #[test]
    fn test_prompt_includes_home_dir_in_write_agent_file_instruction() {
        let identity = test_identity();
        let ctx = PromptContext {
            soul_content: "",
            identity: &identity,
            core_memory: &[],
            is_onboarding: false,
            current_utc: test_time(),
            timezone: None,
            global_home_dir: None,
            channel_type: None,
            telegram_configured: false,
            home_dir: Some(std::path::Path::new("/home/user/.mika/agents/mika")),
            callback_context: None,
        };

        let prompt = build_system_prompt(&ctx);
        assert!(
            prompt.contains("/home/user/.mika/agents/mika"),
            "prompt should include the home directory path"
        );
        assert!(
            prompt.contains("Your home directory is /home/user/.mika/agents/mika"),
            "prompt should include the home directory in write_agent_file instruction"
        );
    }

    #[test]
    fn test_prompt_fallback_when_home_dir_none() {
        let identity = test_identity();
        let ctx = PromptContext {
            soul_content: "",
            identity: &identity,
            core_memory: &[],
            is_onboarding: false,
            current_utc: test_time(),
            timezone: None,
            global_home_dir: None,
            channel_type: None,
            telegram_configured: false,
            home_dir: None,
            callback_context: None,
        };

        let prompt = build_system_prompt(&ctx);
        assert!(
            prompt.contains("Paths are relative to your home"),
            "should fall back to generic instruction when home_dir is None"
        );
    }

    #[test]
    fn test_silent_prompt_includes_telegram_when_configured() {
        let identity = test_identity();
        let ctx = SilentPromptContext {
            soul_content: "",
            identity: &identity,
            core_memory: &[],
            pending_commitments: &[],
            trigger_context: "Heartbeat",
            current_utc: test_time(),
            timezone: None,
            telegram_configured: true,
            has_message_sender: true,
            recent_conversations: None,
            recent_audit_events: None,
            home_dir: None,
            pending_work_items: &[],
        };

        let prompt = build_silent_prompt(&ctx);
        assert!(prompt.contains("Telegram integration is active"));
    }

    #[test]
    fn test_silent_prompt_omits_channel_when_no_telegram() {
        let identity = test_identity();
        let ctx = SilentPromptContext {
            soul_content: "",
            identity: &identity,
            core_memory: &[],
            pending_commitments: &[],
            trigger_context: "Heartbeat",
            current_utc: test_time(),
            timezone: None,
            telegram_configured: false,
            has_message_sender: true,
            recent_conversations: None,
            recent_audit_events: None,
            home_dir: None,
            pending_work_items: &[],
        };

        let prompt = build_silent_prompt(&ctx);
        assert!(!prompt.contains("## Communication Channel"));
    }

    #[test]
    fn test_silent_prompt_omits_send_message_when_no_sender() {
        let identity = test_identity();
        let ctx = SilentPromptContext {
            soul_content: "",
            identity: &identity,
            core_memory: &[],
            pending_commitments: &[],
            trigger_context: "Heartbeat",
            current_utc: test_time(),
            timezone: None,
            telegram_configured: false,
            has_message_sender: false,
            recent_conversations: None,
            recent_audit_events: None,
            home_dir: None,
            pending_work_items: &[],
        };

        let prompt = build_silent_prompt(&ctx);
        assert!(prompt.contains("## Silent Mode"));
        assert!(prompt.contains("NOT delivered"));
        assert!(!prompt.contains("Use the send_message tool"));
        assert!(prompt.contains("No outbound messaging channel is configured"));
    }

    #[test]
    fn test_reflection_config_parse_time_valid() {
        use chrono::Timelike;
        let config = ReflectionConfig {
            enabled: true,
            time: "20:00".to_string(),
            notify: false,
            timezone: None,
        };
        let time = config.parse_time().unwrap();
        assert_eq!(time.hour(), 20);
        assert_eq!(time.minute(), 0);
    }

    #[test]
    fn test_reflection_config_parse_time_invalid() {
        let config = ReflectionConfig {
            enabled: true,
            time: "25:00".to_string(),
            notify: false,
            timezone: None,
        };
        assert!(config.parse_time().is_none());

        let config2 = ReflectionConfig {
            enabled: true,
            time: "8pm".to_string(),
            notify: false,
            timezone: None,
        };
        assert!(config2.parse_time().is_none());
    }

    #[test]
    fn test_reflection_config_defaults() {
        let config = ReflectionConfig::default();
        assert!(!config.enabled);
        assert_eq!(config.time, "20:00");
        assert!(!config.notify);
    }

    #[test]
    fn test_load_identity_with_reflection_config() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("identity.toml"),
            r#"
name = "Mika"
emoji = "✦"

[reflection]
enabled = true
time = "21:30"
notify = true
"#,
        )
        .unwrap();

        let identity = load_identity(tmp.path());
        assert_eq!(identity.name, "Mika");
        let reflection = identity.reflection.unwrap();
        assert!(reflection.enabled);
        assert_eq!(reflection.time, "21:30");
        assert!(reflection.notify);
    }

    #[test]
    fn test_load_identity_without_reflection_config() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("identity.toml"),
            "name = \"Mika\"\nemoji = \"✦\"\n",
        )
        .unwrap();

        let identity = load_identity(tmp.path());
        assert!(identity.reflection.is_none());
    }

    #[test]
    fn test_silent_prompt_includes_reflection_context() {
        let identity = test_identity();
        let ctx = SilentPromptContext {
            soul_content: "",
            identity: &identity,
            core_memory: &[],
            pending_commitments: &[],
            trigger_context: "Reflection mode.",
            current_utc: test_time(),
            timezone: None,
            telegram_configured: false,
            has_message_sender: false,
            recent_conversations: Some("User discussed Series A fundraise with Alice."),
            recent_audit_events: Some("update_core_memory: current_priorities -> fundraise"),
            home_dir: None,
            pending_work_items: &[],
        };

        let prompt = build_silent_prompt(&ctx);
        assert!(prompt.contains("## Today's Conversations"));
        assert!(prompt.contains("Series A fundraise"));
        assert!(prompt.contains("## Recent Audit Events"));
        assert!(prompt.contains("current_priorities"));
    }

    #[test]
    fn test_silent_prompt_omits_empty_reflection_context() {
        let identity = test_identity();
        let ctx = SilentPromptContext {
            soul_content: "",
            identity: &identity,
            core_memory: &[],
            pending_commitments: &[],
            trigger_context: "Reflection mode.",
            current_utc: test_time(),
            timezone: None,
            telegram_configured: false,
            has_message_sender: false,
            recent_conversations: None,
            recent_audit_events: None,
            home_dir: None,
            pending_work_items: &[],
        };

        let prompt = build_silent_prompt(&ctx);
        assert!(!prompt.contains("## Today's Conversations"));
        assert!(!prompt.contains("## Recent Audit Events"));
    }

    #[test]
    fn test_onboarding_prompt_mentions_store_fact() {
        let prompt = onboarding_prompt();
        assert!(prompt.contains("store_fact"));
        assert!(prompt.contains("person"));
        assert!(prompt.contains("The user"));
    }

    #[test]
    fn test_tool_usage_prompt_mentions_store_fact_person() {
        let identity = test_identity();
        let ctx = PromptContext {
            soul_content: "",
            identity: &identity,
            core_memory: &[],
            is_onboarding: false,
            current_utc: chrono::Utc::now(),
            timezone: None,
            global_home_dir: None,
            channel_type: None,
            telegram_configured: false,
            home_dir: None,
            callback_context: None,
        };
        let prompt = build_system_prompt(&ctx);
        assert!(prompt.contains("store_fact(category=\"person\")"));
    }

    #[test]
    fn test_callback_context_injects_prompt_guard() {
        let identity = test_identity();
        let ctx = PromptContext {
            soul_content: "",
            identity: &identity,
            core_memory: &[],
            is_onboarding: false,
            current_utc: chrono::Utc::now(),
            timezone: None,
            global_home_dir: None,
            channel_type: None,
            telegram_configured: false,
            home_dir: None,
            callback_context: Some("Processing callback results from a long-running task."),
        };
        let prompt = build_system_prompt(&ctx);
        assert!(prompt.contains("## Callback Result Turn"));
        assert!(prompt.contains("MUST NOT submit new long-running tasks"));
    }

    #[test]
    fn test_no_callback_context_omits_prompt_guard() {
        let identity = test_identity();
        let ctx = PromptContext {
            soul_content: "",
            identity: &identity,
            core_memory: &[],
            is_onboarding: false,
            current_utc: chrono::Utc::now(),
            timezone: None,
            global_home_dir: None,
            channel_type: None,
            telegram_configured: false,
            home_dir: None,
            callback_context: None,
        };
        let prompt = build_system_prompt(&ctx);
        assert!(!prompt.contains("## Callback Result Turn"));
    }

    #[test]
    fn test_prompt_includes_multi_action_batching() {
        let identity = test_identity();
        let ctx = PromptContext {
            soul_content: "",
            identity: &identity,
            core_memory: &[],
            is_onboarding: false,
            current_utc: test_time(),
            timezone: None,
            global_home_dir: None,
            channel_type: None,
            telegram_configured: false,
            home_dir: None,
            callback_context: None,
        };

        let prompt = build_system_prompt(&ctx);
        assert!(prompt.contains("handle ALL of them"));
        assert!(prompt.contains("multiple tool calls"));
    }

    #[test]
    fn test_prompt_includes_conversation_continuity() {
        let identity = test_identity();
        let ctx = PromptContext {
            soul_content: "",
            identity: &identity,
            core_memory: &[],
            is_onboarding: false,
            current_utc: test_time(),
            timezone: None,
            global_home_dir: None,
            channel_type: None,
            telegram_configured: false,
            home_dir: None,
            callback_context: None,
        };

        let prompt = build_system_prompt(&ctx);
        assert!(prompt.contains("check conversation history and search_memory"));
        assert!(prompt.contains("never re-ask something the user already provided"));
    }

    #[test]
    fn test_prompt_includes_proactive_state_checking() {
        let identity = test_identity();
        let ctx = PromptContext {
            soul_content: "",
            identity: &identity,
            core_memory: &[],
            is_onboarding: false,
            current_utc: test_time(),
            timezone: None,
            global_home_dir: None,
            channel_type: None,
            telegram_configured: false,
            home_dir: None,
            callback_context: None,
        };

        let prompt = build_system_prompt(&ctx);
        assert!(prompt.contains("first check existing state"));
        assert!(prompt.contains("rather than creating a duplicate"));
    }

    #[test]
    fn test_prompt_includes_confirmation_before_action_guardrail() {
        let identity = test_identity();
        let ctx = PromptContext {
            identity: &identity,
            soul_content: "",
            core_memory: &[],
            is_onboarding: false,
            current_utc: test_time(),
            timezone: None,
            global_home_dir: None,
            channel_type: None,
            telegram_configured: false,
            home_dir: None,
            callback_context: None,
        };

        let prompt = build_system_prompt(&ctx);
        assert!(prompt.contains("Confirmation before action"));
        assert!(prompt.contains("Do not interpret questions as implicit requests"));
    }
}
