use std::path::{Path, PathBuf};
use std::sync::Arc;

use ratatui::layout::Rect;
use ratatui::text::Line;
use tokio::sync::mpsc;
use tui_textarea::TextArea;

use mika_agent::async_db::AsyncDatabase;
use mika_agent::db::TeamRunSummary;
use mika_agent::skills::SkillRegistry;
use mika_agent::teams::types::{TeamEvent, TeamPhase};
use mika_common::llm::LlmProvider;

use crate::tui::attachment::ImageAttachment;
use crate::tui::commands;
use crate::tui::commands::autocomplete::AutocompleteState;
use crate::tui::markdown;

/// Messages flowing between the TUI and the team worker.
pub enum TeamRequest {
    Goal(String),
    Quit,
}

/// Dashboard action dispatched from footer button clicks (processed in tick).
pub enum DashboardAction {
    Start,
    Stop,
    Open,
}

/// Agent processing status.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AgentStatus {
    Idle,
    Thinking,
    /// Progressively revealing response. Value = characters revealed so far.
    Responding(usize),
}

/// A single chat message for display.
#[derive(Clone, Debug)]
pub struct ChatMessage {
    pub role: ChatRole,
    pub content: String,
    /// Pre-rendered markdown lines (cached to avoid re-parsing every frame).
    pub rendered: Option<Vec<Line<'static>>>,
    /// Source channel: None = CLI (local), Some("telegram") = Telegram, etc.
    pub channel: Option<String>,
    /// Whether this message is internal (agent-to-agent). Hidden in inbox mode.
    /// Read by audit-mode rendering (future: dimmed styling for internal messages).
    #[allow(dead_code)]
    pub internal: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ChatRole {
    User,
    Assistant,
    System,
    Command,
    Thinking,
}

/// A position within a specific message (message index + position within that message).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MessagePosition {
    pub message_idx: usize,
    pub text_pos: TextPosition,
}

impl PartialOrd for MessagePosition {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for MessagePosition {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.message_idx
            .cmp(&other.message_idx)
            .then(self.text_pos.cmp(&other.text_pos))
    }
}

/// Text position within a message's rendered lines.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct TextPosition {
    /// Line index within the message's rendered lines.
    pub line: usize,
    /// Character offset within the line (by unicode width columns).
    pub char_offset: usize,
}

/// Selection state machine for text selection in the messages panel.
#[derive(Clone, Debug, Default)]
pub enum SelectionState {
    #[default]
    None,
    /// Mouse button is held, drag in progress.
    Dragging {
        anchor: MessagePosition,
        current: MessagePosition,
    },
    /// Selection complete (mouse released).
    Selected {
        start: MessagePosition,
        end: MessagePosition,
    },
}

impl SelectionState {
    pub fn is_none(&self) -> bool {
        matches!(self, Self::None)
    }

    /// Clear selection back to None.
    pub fn clear(&mut self) {
        *self = Self::None;
    }
}

/// Layout info for a single rendered message.
#[derive(Clone, Debug)]
pub struct MessageLayout {
    /// Index into app.messages (or special: pending response, thinking indicator).
    pub message_idx: usize,
    /// The rendered lines for this message (including spacer).
    pub lines: Vec<Line<'static>>,
    /// Wrapped line count at current width.
    pub wrapped_line_count: usize,
}

/// Cached layout state for the messages panel.
#[derive(Clone, Debug, Default)]
pub struct MessagesLayout {
    /// Per-message layout entries in display order.
    pub entries: Vec<MessageLayout>,
    /// Sum of all wrapped line counts.
    pub total_lines: usize,
    /// Width used to compute this layout.
    pub computed_at_width: u16,
    /// Message count + pending state when last computed.
    pub computed_at_count: usize,
    /// Whether pending_response was present when computed.
    pub had_pending: bool,
    /// Reveal index when last computed (for streaming).
    pub computed_at_reveal: usize,
    /// Agent status when last computed (for thinking indicator).
    pub computed_at_thinking: bool,
    /// Thinking dots phase when last computed (0-3).
    pub computed_at_dots_phase: u64,
}

impl MessagesLayout {
    /// Check if the layout needs recomputation.
    pub fn is_stale(
        &self,
        width: u16,
        msg_count: usize,
        has_pending: bool,
        reveal_index: usize,
        is_thinking: bool,
        dots_phase: u64,
    ) -> bool {
        self.computed_at_width != width
            || self.computed_at_count != msg_count
            || self.had_pending != has_pending
            || (has_pending && self.computed_at_reveal != reveal_index)
            || self.computed_at_thinking != is_thinking
            || (is_thinking && self.computed_at_dots_phase != dots_phase)
    }
}

/// Extract callback task label from message metadata JSON.
pub fn callback_label_from_metadata(metadata: &Option<String>) -> String {
    metadata
        .as_ref()
        .and_then(|m| serde_json::from_str::<serde_json::Value>(m).ok())
        .and_then(|v| v.get("label")?.as_str().map(|s| s.to_string()))
        .unwrap_or_else(|| "background task".to_string())
}

/// Convert a `SessionMessage` to a `ChatMessage` for TUI display.
///
/// Returns `None` if the message should be skipped (unsupported role, stale framing, etc.).
/// This is the single conversion point used by startup history load, cross-channel poll,
/// and rewind reload to ensure consistent filtering and formatting.
pub fn session_message_to_chat_message(
    msg: &mika_agent::db::SessionMessage,
) -> Option<ChatMessage> {
    let role = match msg.role.as_str() {
        "user" => {
            // Skip stale framing messages saved before the callback save fix
            if msg.content.starts_with("A background task has completed.") {
                return None;
            }
            ChatRole::User
        }
        "assistant" => ChatRole::Assistant,
        "tool_result" => ChatRole::System,
        _ => return None,
    };
    let channel = if msg.channel_type == "cli" {
        None
    } else {
        Some(msg.channel_type.clone())
    };
    let content = if msg.role == "tool_result" {
        let label = callback_label_from_metadata(&msg.metadata);
        format!("[Task: {}] Result received", label)
    } else {
        msg.content.clone()
    };
    Some(ChatMessage {
        role,
        content,
        rendered: None,
        channel,
        internal: msg.internal,
    })
}

/// Format a previous team run summary as a human-readable context block.
fn format_previous_run_context(summary: &TeamRunSummary) -> String {
    fn truncate(s: &str, max_chars: usize) -> String {
        if s.chars().count() <= max_chars {
            s.to_string()
        } else {
            let truncated: String = s.chars().take(max_chars).collect();
            format!("{truncated}...")
        }
    }

    let run = &summary.run;
    let status_note = match run.status.as_str() {
        "running" => " (still in progress — context may be incomplete)",
        "suspended" => " (suspended — has pending callbacks)",
        _ => "",
    };

    let mut lines = Vec::new();
    lines.push("━━━ Previous Run Context ━━━".to_string());
    lines.push(format!(
        "Run:    {} ({}{})",
        &run.id[..8.min(run.id.len())],
        run.status,
        status_note,
    ));
    lines.push(format!("Goal:   {}", truncate(&run.goal, 200)));

    if let Some(ended) = &run.ended_at {
        lines.push(format!("Time:   {} → {}", run.started_at, ended));
    } else {
        lines.push(format!("Time:   {}", run.started_at));
    }

    if !summary.agent_results.is_empty() {
        let agents: Vec<String> = summary
            .agent_results
            .iter()
            .map(|a| a.agent_name.clone())
            .collect();
        lines.push(format!("Agents: {}", agents.join(", ")));
    }

    if let Some(critic) = &summary.critic_feedback {
        lines.push(format!("Critic: {}", truncate(critic, 300)));
    }

    if let Some(deliverable) = &run.deliverable {
        lines.push(String::new());
        lines.push("Deliverable:".to_string());
        for line in truncate(deliverable, 500).lines() {
            lines.push(format!("  {line}"));
        }
    } else {
        lines.push("Deliverable: (none)".to_string());
    }

    lines.push("━━━━━━━━━━━━━━━━━━━━━━━━━━━━".to_string());
    lines.join("\n")
}

/// Messages flowing between the TUI and the agent worker.
pub enum AgentRequest {
    Message {
        text: String,
        images: Vec<ImageAttachment>,
        thinking_budget: Option<u32>,
    },
    /// A background callback task has completed or failed — inject into the conversation.
    CallbackResult {
        task_id: String,
        label: String,
        result: String,
        trace_id: Option<String>,
        /// Original task status before delivery ("completed" or "failed").
        /// Used by error-recovery to reset to the correct pre-delivery status.
        original_status: String,
        /// The parent work item ID, if the callback task has a parent linkage.
        /// Threaded through to `build_callback_trigger_context()` so the agent
        /// knows which work item this callback relates to. See #313.
        parent_task_id: Option<String>,
    },
    SetModel {
        model: String,
    },
    /// Session was reset (e.g., by /clear). Worker should update its session ID.
    NewSession {
        session_id: String,
    },
    Quit,
}

pub struct AgentResponse {
    pub content: String,
    pub is_error: bool,
    pub thinking: Option<String>,
    pub input_tokens: Option<u32>,
    /// If skills were hot-reloaded during this turn, contains the new registry.
    pub updated_skills: Option<Arc<SkillRegistry>>,
}

/// Shell-like input history with draft saving and optional disk persistence.
pub struct InputHistory {
    entries: Vec<String>,
    index: Option<usize>,
    saved_draft: Option<String>,
    /// Path to the history file (None = no persistence, e.g. in tests).
    file_path: Option<PathBuf>,
}

const HISTORY_MAX_SIZE: usize = 500;
const HISTORY_FILENAME: &str = ".input_history";

impl InputHistory {
    /// Load history from disk, or start empty if file is missing/corrupt.
    pub fn load(home_dir: &Path) -> Self {
        let file_path = home_dir.join(HISTORY_FILENAME);
        let entries = Self::read_file(&file_path).unwrap_or_default();
        Self {
            entries,
            index: None,
            saved_draft: None,
            file_path: Some(file_path),
        }
    }

    /// In-memory only (for tests).
    #[cfg(test)]
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
            index: None,
            saved_draft: None,
            file_path: None,
        }
    }

    /// Add a sent message to history. Resets navigation state. Persists to disk.
    pub fn push(&mut self, entry: String) {
        if !entry.is_empty() {
            self.entries.push(entry);
            if self.entries.len() > HISTORY_MAX_SIZE {
                self.entries.remove(0);
            }
        }
        self.index = None;
        self.saved_draft = None;
        self.save();
    }

    /// Persist history entries to disk (no-op if no file_path).
    fn save(&self) {
        let Some(path) = &self.file_path else { return };
        if let Err(e) = Self::write_file(path, &self.entries) {
            tracing::warn!("failed to save input history: {e}");
        }
    }

    fn read_file(path: &Path) -> Option<Vec<String>> {
        let data = std::fs::read_to_string(path).ok()?;
        serde_json::from_str(&data).ok()
    }

    fn write_file(path: &Path, entries: &[String]) -> std::io::Result<()> {
        let json = serde_json::to_string(entries).map_err(std::io::Error::other)?;
        let tmp = path.with_extension("tmp");
        // Create temp file with 0600 permissions from the start (no race window).
        #[cfg(unix)]
        {
            use std::io::Write;
            use std::os::unix::fs::OpenOptionsExt;
            std::fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .mode(0o600)
                .open(&tmp)?
                .write_all(json.as_bytes())?;
        }
        #[cfg(not(unix))]
        {
            std::fs::write(&tmp, &json)?;
        }
        std::fs::rename(&tmp, path)?;
        Ok(())
    }

    /// Navigate to the previous (older) history entry.
    /// On first call, saves `current_input` as a draft.
    /// Returns the history entry text, or None if history is empty.
    pub fn previous(&mut self, current_input: &str) -> Option<String> {
        if self.entries.is_empty() {
            return None;
        }
        let idx = match self.index {
            None => {
                // First time entering history — save current input as draft
                self.saved_draft = Some(current_input.to_string());
                self.entries.len() - 1
            }
            Some(0) => return Some(self.entries[0].clone()), // Already at oldest — stay
            Some(i) => i - 1,
        };
        self.index = Some(idx);
        Some(self.entries[idx].clone())
    }

    /// Navigate to the next (newer) history entry.
    /// Returns Some(text) when navigating forward, None if not browsing.
    /// When cycling past newest, restores the saved draft.
    pub fn next(&mut self) -> Option<String> {
        match self.index {
            None => None,
            Some(i) => {
                if i + 1 >= self.entries.len() {
                    // Past newest — restore draft
                    self.index = None;
                    Some(self.saved_draft.take().unwrap_or_default())
                } else {
                    self.index = Some(i + 1);
                    Some(self.entries[i + 1].clone())
                }
            }
        }
    }

    /// Reset navigation state (called on Esc, etc.).
    pub fn reset(&mut self) {
        self.index = None;
        self.saved_draft = None;
    }

    /// Whether we are currently browsing history.
    pub fn is_browsing(&self) -> bool {
        self.index.is_some()
    }
}

/// Status of an agent in the team dashboard.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DashboardAgentStatus {
    Running,
    Completed,
    Failed,
}

/// Entry for a single agent in the team dashboard.
#[derive(Debug)]
pub struct DashboardAgentEntry {
    pub name: String,
    pub role: String,
    pub status: DashboardAgentStatus,
}

/// Live dashboard state during team runs.
#[derive(Debug)]
pub struct TeamDashboardState {
    pub phase: Option<TeamPhase>,
    pub iteration: u32,
    pub agents: Vec<DashboardAgentEntry>,
    pub run_started: std::time::Instant,
}

impl TeamDashboardState {
    pub fn new() -> Self {
        Self {
            phase: None,
            iteration: 0,
            agents: Vec::new(),
            run_started: std::time::Instant::now(),
        }
    }
}

/// Placeholder text for the message input field (normal mode).
pub const PLACEHOLDER_MESSAGE: &str = "Type a message... (Alt+Enter for new line)";
/// Placeholder text for the message input field (team mode).
pub const PLACEHOLDER_TEAM_GOAL: &str = "Type a goal for the team... (Alt+Enter for new line)";

/// Main application state.
pub struct App<'a> {
    pub messages: Vec<ChatMessage>,
    pub scroll_offset: usize,
    pub status: AgentStatus,
    pub should_quit: bool,

    // Agent communication
    pub agent_tx: mpsc::UnboundedSender<AgentRequest>,
    pub agent_rx: mpsc::UnboundedReceiver<AgentResponse>,

    // Progressive reveal state
    pub pending_response: Option<String>,
    pub reveal_index: usize,

    // Input
    pub textarea: TextArea<'a>,
    pub history: InputHistory,

    // New message indicator (set when content arrives while user is scrolled up)
    pub has_new_message: bool,

    // Display info
    pub session_id: String,
    pub model: String,
    pub identity_name: String,
    /// The active LLM provider (for /provider command).
    pub provider: mika_common::llm::ProviderKind,

    // Animated thinking dots
    pub tick_count: u64,

    /// Whether the UI needs to be redrawn (set true on state changes).
    pub needs_redraw: bool,

    /// Text selection state for the messages panel.
    pub selection_state: SelectionState,
    /// Cached messages layout for per-message rendering.
    pub messages_layout: MessagesLayout,
    /// The inner rect of the messages panel (set during draw, used for hit-testing).
    pub messages_inner_rect: Option<Rect>,
    /// The inner rect of the textarea (set during draw, used for mouse hit-testing).
    pub textarea_inner_rect: Option<Rect>,
    /// Textarea scroll offset (display rows skipped, set during draw).
    pub textarea_scroll_offset: u16,
    /// Whether the user is actively dragging a selection in the textarea.
    pub textarea_selecting: bool,

    // Shared resources for slash commands
    pub db: AsyncDatabase,
    pub llm: Arc<dyn LlmProvider>,
    pub home_dir: PathBuf,
    pub skills: Arc<SkillRegistry>,

    // Slash command state
    pub autocomplete: AutocompleteState,
    pub pending_command: Option<String>,

    // Agent info
    pub agent_name: String,
    /// The global Mika home directory (e.g. ~/.mika/).
    pub global_home: PathBuf,

    /// If set, the chat loop should switch to this agent after the current worker stops.
    pub pending_switch: Option<String>,

    // Image attachments pending send
    pub pending_images: Vec<ImageAttachment>,

    // Persistent thinking level: (budget_tokens, level_name)
    pub thinking_level: Option<(u32, &'static str)>,

    // Context usage tracking
    pub context_tokens: Option<u32>,

    // Cross-channel polling
    /// Watermark: highest message id seen (used to avoid re-fetching).
    pub last_seen_msg_id: i64,
    /// Cached count of pending/active tasks (polled periodically for footer badge).
    pub pending_task_count: usize,
    /// Cached count of active background callback tasks (polled periodically for footer badge).
    /// Agent-scoped (not session-scoped) — NOT reset on /clear.
    pub active_background_task_count: usize,

    // Team mode fields (None when in agent mode)
    /// Team worker channel for sending goals.
    pub team_tx: Option<mpsc::UnboundedSender<TeamRequest>>,
    /// Team worker channel for receiving team events.
    pub team_rx: Option<mpsc::UnboundedReceiver<TeamEvent>>,
    /// Team name (set when in team mode).
    pub team_name: Option<String>,
    /// Team directory path (e.g., ~/.mika/teams/{name}/).
    pub team_dir: Option<PathBuf>,
    /// Verbose mode: show individual agent responses in team mode.
    pub verbose_mode: bool,
    /// Inbox mode: hide internal (agent-to-agent) messages. Default: true.
    pub inbox_mode: bool,
    /// Count of hidden internal messages (for footer badge).
    pub hidden_internal_count: usize,
    /// Live dashboard state during team runs (created on first PhaseChanged event).
    pub team_dashboard: Option<TeamDashboardState>,

    /// Whether the embedded dashboard is enabled on the running server (checked once at startup).
    pub dashboard_running: bool,

    /// Screen rect of the `[open]` button in the footer (set during draw, used for click handling).
    pub footer_open_rect: Option<Rect>,

    /// Screen rect of the `[start]`/`[stop]` button in the footer (set during draw).
    pub footer_start_stop_rect: Option<Rect>,

    /// Pending dashboard action from footer button click (processed in tick).
    pub pending_dashboard_action: Option<DashboardAction>,
}

/// Context window limit for the model (Claude's 200K context).
pub const MODEL_CONTEXT_LIMIT: u32 = 200_000;

// Channel filtering removed: sessions table replaces channel_type column.
// Cross-channel messages are now discovered via load_messages_after (watermark-based).

/// Poll interval for cross-channel messages in ticks (~5 seconds at 30ms tick rate).
const POLL_INTERVAL_TICKS: u64 = 167;

impl<'a> App<'a> {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        agent_tx: mpsc::UnboundedSender<AgentRequest>,
        agent_rx: mpsc::UnboundedReceiver<AgentResponse>,
        session_id: String,
        model: String,
        identity_name: String,
        db: AsyncDatabase,
        llm: Arc<dyn LlmProvider>,
        home_dir: PathBuf,
        skills: Arc<SkillRegistry>,
        agent_name: String,
        global_home: PathBuf,
        provider: mika_common::llm::ProviderKind,
    ) -> Self {
        let mut textarea = TextArea::default();
        textarea.set_cursor_line_style(ratatui::style::Style::default());
        textarea.set_placeholder_text(PLACEHOLDER_MESSAGE);

        Self {
            messages: Vec::new(),
            scroll_offset: 0,
            status: AgentStatus::Idle,
            should_quit: false,
            agent_tx,
            agent_rx,
            pending_response: None,
            reveal_index: 0,
            textarea,
            history: InputHistory::load(&home_dir),
            has_new_message: false,
            session_id,
            model,
            identity_name,
            provider,
            tick_count: 0,
            needs_redraw: true,
            selection_state: SelectionState::default(),
            messages_layout: MessagesLayout::default(),
            messages_inner_rect: None,
            textarea_inner_rect: None,
            textarea_scroll_offset: 0,
            textarea_selecting: false,
            db,
            llm,
            home_dir,
            skills,
            autocomplete: AutocompleteState::new(),
            pending_command: None,
            agent_name,
            global_home,
            pending_switch: None,
            pending_images: Vec::new(),
            thinking_level: None,
            context_tokens: None,
            last_seen_msg_id: 0,
            pending_task_count: 0,
            active_background_task_count: 0,
            team_tx: None,
            team_rx: None,
            team_name: None,
            team_dir: None,
            verbose_mode: false,
            inbox_mode: true,
            hidden_internal_count: 0,
            team_dashboard: None,
            dashboard_running: false,
            footer_open_rect: None,
            footer_start_stop_rect: None,
            pending_dashboard_action: None,
        }
    }

    /// Create a new App in team mode.
    pub fn new_team(
        team_tx: mpsc::UnboundedSender<TeamRequest>,
        team_rx: mpsc::UnboundedReceiver<TeamEvent>,
        team_name: &str,
        team_dir: PathBuf,
        global_home: PathBuf,
        db: AsyncDatabase,
        previous_run: Option<TeamRunSummary>,
    ) -> Self {
        let mut textarea = TextArea::default();
        textarea.set_cursor_line_style(ratatui::style::Style::default());
        textarea.set_placeholder_text(PLACEHOLDER_TEAM_GOAL);

        // Use dummy agent channels — team mode does not use them.
        let (agent_tx, _) = mpsc::unbounded_channel::<AgentRequest>();
        let (_, agent_rx) = mpsc::unbounded_channel::<AgentResponse>();

        let messages = if let Some(summary) = previous_run {
            vec![ChatMessage {
                role: ChatRole::System,
                content: format_previous_run_context(&summary),
                rendered: None,
                channel: None,
                internal: false,
            }]
        } else {
            Vec::new()
        };

        Self {
            messages,
            scroll_offset: 0,
            status: AgentStatus::Idle,
            should_quit: false,
            agent_tx,
            agent_rx,
            pending_response: None,
            reveal_index: 0,
            textarea,
            history: InputHistory::load(&team_dir),
            has_new_message: false,
            session_id: String::new(),
            model: String::new(),
            identity_name: format!("Team: {team_name}"),
            provider: mika_common::llm::ProviderKind::Anthropic,
            tick_count: 0,
            needs_redraw: true,
            selection_state: SelectionState::default(),
            messages_layout: MessagesLayout::default(),
            messages_inner_rect: None,
            textarea_inner_rect: None,
            textarea_scroll_offset: 0,
            textarea_selecting: false,
            // Team mode does not use agent-specific resources. These are set to
            // safe defaults. Slash command handlers check `is_team_mode()` before
            // accessing them.
            db,
            llm: mika_common::llm::dummy_provider(),
            home_dir: team_dir.clone(),
            skills: Arc::new(SkillRegistry::empty()),
            autocomplete: AutocompleteState::new(),
            pending_command: None,
            agent_name: String::new(),
            global_home,
            pending_switch: None,
            pending_images: Vec::new(),
            thinking_level: None,
            context_tokens: None,
            last_seen_msg_id: 0,
            pending_task_count: 0,
            active_background_task_count: 0,
            team_tx: Some(team_tx),
            team_rx: Some(team_rx),
            team_name: Some(team_name.to_string()),
            team_dir: Some(team_dir),
            verbose_mode: false,
            inbox_mode: true,
            hidden_internal_count: 0,
            team_dashboard: None,
            dashboard_running: false,
            footer_open_rect: None,
            footer_start_stop_rect: None,
            pending_dashboard_action: None,
        }
    }

    /// Whether the app is running in team mode.
    pub fn is_team_mode(&self) -> bool {
        self.team_name.is_some()
    }

    /// Load persisted thinking level from the database.
    pub async fn load_thinking_level(&mut self) {
        self.thinking_level = None;
        if let Ok(Some(level_str)) = self.db.get_customer_config("thinking_level").await
            && let Some(resolved) = commands::resolve_thinking_level(&level_str)
        {
            self.thinking_level = Some(resolved);
        }
    }

    /// Submit the current input to the agent, or queue a slash command.
    pub fn send_message(&mut self) {
        let budget = self.thinking_level.map(|(b, _)| b);
        self.send_message_with_thinking(budget);
    }

    /// Submit the current input with optional thinking budget.
    pub fn send_message_with_thinking(&mut self, thinking_budget: Option<u32>) {
        let text: String = self.textarea.lines().join("\n");
        let text = text.trim().to_string();
        if text.is_empty() && self.pending_images.is_empty() {
            return;
        }

        self.autocomplete.dismiss();

        if text.starts_with('/') && self.pending_images.is_empty() {
            // Queue slash command for async processing in tick()
            self.reset_textarea();
            self.pending_command = Some(text);
            self.needs_redraw = true;
            return;
        }

        // Save to history
        self.history.push(text.clone());

        // Team mode: send goal to team worker
        if let Some(ref team_tx) = self.team_tx {
            self.messages.push(ChatMessage {
                role: ChatRole::User,
                content: text.clone(),
                rendered: None,
                channel: None,
                internal: false,
            });
            let _ = team_tx.send(TeamRequest::Goal(text.clone()));
            // Persist user message to DB (fire-and-forget)
            let db = self.db.clone();
            tokio::spawn(async move {
                if let Err(e) = db.save_message("", "user", &text, None).await {
                    tracing::warn!(error = %e, "failed to save team user message");
                }
            });
            self.status = AgentStatus::Thinking;
            self.reset_textarea();
            self.scroll_offset = 0;
            self.needs_redraw = true;
            return;
        }

        // Build display message with attachment info
        let display = if self.pending_images.is_empty() {
            text.clone()
        } else {
            let labels: Vec<String> = self
                .pending_images
                .iter()
                .map(|img| format!("[{}]", img.label))
                .collect();
            if text.is_empty() {
                labels.join(" ")
            } else {
                format!("{} {text}", labels.join(" "))
            }
        };

        // Add user message to display (no markdown rendering needed for user messages)
        self.messages.push(ChatMessage {
            role: ChatRole::User,
            content: display,
            rendered: None,
            channel: None,
            internal: false,
        });

        // Drain pending images
        let images = std::mem::take(&mut self.pending_images);

        // Send to agent worker
        let _ = self.agent_tx.send(AgentRequest::Message {
            text,
            images,
            thinking_budget,
        });
        self.status = AgentStatus::Thinking;

        // Clear input
        self.reset_textarea();

        // Auto-scroll to bottom
        self.scroll_offset = 0;
        self.needs_redraw = true;
    }

    /// Called on each tick to advance progressive reveal, check for agent responses,
    /// and process pending slash commands.
    pub async fn tick(&mut self) {
        self.tick_count = self.tick_count.wrapping_add(1);

        // Process pending slash command
        if let Some(cmd) = self.pending_command.take() {
            if let Some(output) = commands::handlers::dispatch(self, &cmd).await {
                self.messages.push(ChatMessage {
                    role: ChatRole::Command,
                    content: output,
                    rendered: None,
                    channel: None,
                    internal: false,
                });
                self.auto_scroll_to_bottom();
            }
            self.needs_redraw = true;
        }

        // Process pending dashboard action (from footer button clicks)
        if let Some(action) = self.pending_dashboard_action.take() {
            let msg = match action {
                DashboardAction::Start | DashboardAction::Stop => {
                    let (endpoint, verb) = match action {
                        DashboardAction::Start => ("enable", "enabled"),
                        DashboardAction::Stop => ("disable", "disabled"),
                        _ => unreachable!(),
                    };
                    match crate::commands::dashboard::auth_token() {
                        Err(_) => {
                            "MIKA_INTERNAL_TOKEN or MIKA_DASHBOARD_TOKEN is required.".to_string()
                        }
                        Ok(token) => {
                            let url = format!(
                                "{}/api/v1/dashboard/{endpoint}",
                                crate::commands::dashboard::server_url()
                            );
                            let client = reqwest::Client::new();
                            match client
                                .post(&url)
                                .header("authorization", format!("Bearer {token}"))
                                .timeout(std::time::Duration::from_secs(5))
                                .send()
                                .await
                            {
                                Ok(resp) if resp.status().is_success() => {
                                    self.dashboard_running =
                                        matches!(action, DashboardAction::Start);
                                    format!("Dashboard {verb}.")
                                }
                                Ok(resp) => {
                                    format!("Failed to {endpoint} dashboard: {}", resp.status())
                                }
                                Err(e) => format!("Failed to reach mika-server: {e}"),
                            }
                        }
                    }
                }
                DashboardAction::Open => crate::commands::dashboard::open_dashboard_in_browser(),
            };
            self.messages.push(ChatMessage {
                role: ChatRole::Command,
                content: msg,
                rendered: None,
                channel: None,
                internal: false,
            });
            self.auto_scroll_to_bottom();
            self.needs_redraw = true;
        }

        // Team mode: poll team response channel
        if self.is_team_mode() {
            self.tick_team_mode().await;
        } else {
            self.tick_agent_mode().await;
        }

        // Advance progressive reveal
        if self.pending_response.is_some() {
            let full = self.pending_response.as_ref().unwrap();
            let len = full.len();
            if self.reveal_index < len {
                // Reveal in chunks scaled by response length for smooth appearance.
                // Small (<1KB): 8 bytes/tick, medium (<4KB): 32, large: 64.
                // Use floor_char_boundary to avoid panicking on multi-byte UTF-8 chars.
                let increment = if len < 1024 {
                    8
                } else if len < 4096 {
                    32
                } else {
                    64
                };
                self.reveal_index = full
                    .floor_char_boundary(self.reveal_index + increment)
                    .min(len);
                self.status = AgentStatus::Responding(self.reveal_index);
                self.needs_redraw = true;
            } else {
                // Reveal complete — take ownership (no clone) and add full message
                let full = self.pending_response.take().unwrap();
                let rendered = markdown::render(&full);
                self.messages.push(ChatMessage {
                    role: ChatRole::Assistant,
                    content: full,
                    rendered: Some(rendered),
                    channel: None,
                    internal: false,
                });
                self.reveal_index = 0;
                self.status = AgentStatus::Idle;
                // Auto-scroll to bottom (only if user hasn't scrolled up)
                self.auto_scroll_to_bottom();
                self.needs_redraw = true;
                // Update watermark to avoid re-polling our own messages (agent mode only)
                if !self.is_team_mode()
                    && let Ok(max_id) = self.db.max_message_id().await
                {
                    self.last_seen_msg_id = max_id;
                }
            }
        }

        // Cross-channel polling: agent mode only.
        if !self.is_team_mode()
            && self.tick_count.is_multiple_of(POLL_INTERVAL_TICKS)
            && self.status == AgentStatus::Idle
        {
            self.poll_cross_channel_messages().await;
        }

        // Task count polling: refresh every ~5s for footer badge.
        if !self.is_team_mode()
            && self.tick_count.is_multiple_of(POLL_INTERVAL_TICKS)
            && let Ok(tasks) = self.db.get_user_visible_tasks().await
        {
            let new_count = tasks.len();
            if new_count != self.pending_task_count {
                self.pending_task_count = new_count;
                self.needs_redraw = true;
            }
        }

        // Background task count polling: refresh every ~5s for footer badge.
        if !self.is_team_mode()
            && self.tick_count.is_multiple_of(POLL_INTERVAL_TICKS)
            && let Ok(count) = self.db.get_active_background_task_count().await
            && count != self.active_background_task_count
        {
            self.active_background_task_count = count;
            self.needs_redraw = true;
        }

        // Callback delivery polling: check for completed-but-undelivered callback tasks.
        // Only poll when idle (agent not processing) and not in team mode.
        if !self.is_team_mode()
            && self.tick_count.is_multiple_of(POLL_INTERVAL_TICKS)
            && self.status == AgentStatus::Idle
        {
            self.poll_callback_tasks().await;
        }

        // Thinking animation needs redraw every tick while active
        if self.status == AgentStatus::Thinking {
            self.needs_redraw = true;
        }
    }

    /// Tick handler for team mode: poll team_rx for progress/deliverable/error.
    async fn tick_team_mode(&mut self) {
        let Some(ref mut team_rx) = self.team_rx else {
            return;
        };
        match team_rx.try_recv() {
            Ok(TeamEvent::Progress(msg)) => {
                self.messages.push(ChatMessage {
                    role: ChatRole::System,
                    content: msg,
                    rendered: None,
                    channel: None,
                    internal: false,
                });
                self.auto_scroll_to_bottom();
                self.needs_redraw = true;
            }
            Ok(TeamEvent::PhaseChanged { phase, iteration }) => {
                let dash = self
                    .team_dashboard
                    .get_or_insert_with(TeamDashboardState::new);
                dash.phase = Some(phase.clone());
                dash.iteration = iteration;
                dash.agents.clear();
                self.messages.push(ChatMessage {
                    role: ChatRole::System,
                    content: format!("Phase: {phase} (iteration {iteration})"),
                    rendered: None,
                    channel: None,
                    internal: false,
                });
                self.auto_scroll_to_bottom();
                self.needs_redraw = true;
            }
            Ok(TeamEvent::AgentStarted { agent, role }) => {
                if let Some(ref mut dash) = self.team_dashboard {
                    dash.agents.push(DashboardAgentEntry {
                        name: agent.clone(),
                        role: role.clone(),
                        status: DashboardAgentStatus::Running,
                    });
                }
                self.needs_redraw = true;
            }
            Ok(TeamEvent::AgentCompleted { agent, response }) => {
                // Update dashboard agent status
                if let Some(ref mut dash) = self.team_dashboard
                    && let Some(entry) = dash.agents.iter_mut().find(|a| a.name == agent)
                {
                    entry.status = DashboardAgentStatus::Completed;
                }
                if self.verbose_mode {
                    self.messages.push(ChatMessage {
                        role: ChatRole::System,
                        content: format!("[{agent}] {response}"),
                        rendered: None,
                        channel: None,
                        internal: false,
                    });
                    self.auto_scroll_to_bottom();
                }
                self.needs_redraw = true;
            }
            Ok(TeamEvent::AgentFailed { agent, error }) => {
                // Update dashboard agent status
                if let Some(ref mut dash) = self.team_dashboard
                    && let Some(entry) = dash.agents.iter_mut().find(|a| a.name == agent)
                {
                    entry.status = DashboardAgentStatus::Failed;
                }
                if self.verbose_mode {
                    self.messages.push(ChatMessage {
                        role: ChatRole::System,
                        content: format!("[{agent}] [failed] {error}"),
                        rendered: None,
                        channel: None,
                        internal: false,
                    });
                    self.auto_scroll_to_bottom();
                }
                self.needs_redraw = true;
            }
            Ok(TeamEvent::TasksAssigned { tasks, iteration }) => {
                // In verbose mode, show individual assignments
                if self.verbose_mode {
                    for task in &tasks {
                        self.messages.push(ChatMessage {
                            role: ChatRole::System,
                            content: format!("[{}] [assigned] {}", task.agent, task.task),
                            rendered: None,
                            channel: None,
                            internal: false,
                        });
                    }
                }
                let names: Vec<_> = tasks.iter().map(|t| t.agent.as_str()).collect();
                self.messages.push(ChatMessage {
                    role: ChatRole::System,
                    content: format!(
                        "Iteration {iteration}: assigned tasks to {}",
                        names.join(", ")
                    ),
                    rendered: None,
                    channel: None,
                    internal: false,
                });
                self.auto_scroll_to_bottom();
                self.needs_redraw = true;
            }
            Ok(TeamEvent::CriticReview {
                approved,
                feedback,
                iteration,
            }) => {
                let verdict = if approved { "approved" } else { "rejected" };
                self.messages.push(ChatMessage {
                    role: ChatRole::System,
                    content: format!("Critic (iteration {iteration}): {verdict}. {feedback}"),
                    rendered: None,
                    channel: None,
                    internal: false,
                });
                self.auto_scroll_to_bottom();
                self.needs_redraw = true;
            }
            Ok(TeamEvent::Deliverable(text)) => {
                self.team_dashboard = None;
                if text.is_empty() {
                    self.messages.push(ChatMessage {
                        role: ChatRole::System,
                        content: "Team completed with no deliverable.".to_string(),
                        rendered: None,
                        channel: None,
                        internal: false,
                    });
                    self.status = AgentStatus::Idle;
                } else {
                    // Persist deliverable to DB
                    if let Err(e) = self.db.save_message("", "assistant", &text, None).await {
                        tracing::warn!(error = %e, "failed to save team deliverable");
                    }
                    self.pending_response = Some(text);
                    self.reveal_index = 0;
                    self.status = AgentStatus::Responding(0);
                }
                self.needs_redraw = true;
            }
            Ok(TeamEvent::RunFailed(msg)) => {
                self.team_dashboard = None;
                self.messages.push(ChatMessage {
                    role: ChatRole::System,
                    content: format!("Team error: {msg}"),
                    rendered: None,
                    channel: None,
                    internal: false,
                });
                self.status = AgentStatus::Idle;
                self.needs_redraw = true;
            }
            Err(mpsc::error::TryRecvError::Disconnected) => {
                if self.status == AgentStatus::Thinking {
                    self.messages.push(ChatMessage {
                        role: ChatRole::System,
                        content: "Team worker stopped unexpectedly.".to_string(),
                        rendered: None,
                        channel: None,
                        internal: false,
                    });
                    self.status = AgentStatus::Idle;
                    self.needs_redraw = true;
                }
            }
            Err(mpsc::error::TryRecvError::Empty) => {}
        }
    }

    /// Tick handler for agent mode: poll agent_rx for responses.
    async fn tick_agent_mode(&mut self) {
        match self.agent_rx.try_recv() {
            Ok(response) => {
                // Update skills if hot-reloaded during this turn
                if let Some(new_skills) = response.updated_skills {
                    self.skills = new_skills;
                }

                // Update context token tracking
                if let Some(tokens) = response.input_tokens {
                    self.context_tokens = Some(tokens);
                }

                if response.is_error {
                    self.messages.push(ChatMessage {
                        role: ChatRole::System,
                        content: format!("Error: {}", response.content),
                        rendered: None,
                        channel: None,
                        internal: false,
                    });
                    self.status = AgentStatus::Idle;
                } else {
                    // Show thinking block (instantly, not progressively revealed)
                    if let Some(thinking) = response.thinking {
                        let rendered = Self::render_thinking(&thinking);
                        self.messages.push(ChatMessage {
                            role: ChatRole::Thinking,
                            content: thinking,
                            rendered: Some(rendered),
                            channel: None,
                            internal: false,
                        });
                    }

                    if response.content.is_empty() {
                        // Agent responded with tool-use only (no text) — show feedback
                        self.messages.push(ChatMessage {
                            role: ChatRole::System,
                            content: mika_agent::agent::EMPTY_RESPONSE_FALLBACK.to_string(),
                            rendered: None,
                            channel: None,
                            internal: false,
                        });
                        self.status = AgentStatus::Idle;
                    } else {
                        self.pending_response = Some(response.content);
                        self.reveal_index = 0;
                        self.status = AgentStatus::Responding(0);
                    }
                }
                self.needs_redraw = true;
            }
            Err(mpsc::error::TryRecvError::Disconnected) => {
                // Agent worker crashed or exited unexpectedly
                if self.status == AgentStatus::Thinking {
                    self.messages.push(ChatMessage {
                        role: ChatRole::System,
                        content: "Agent worker stopped unexpectedly.".to_string(),
                        rendered: None,
                        channel: None,
                        internal: false,
                    });
                    self.status = AgentStatus::Idle;
                    self.needs_redraw = true;
                }
            }
            Err(mpsc::error::TryRecvError::Empty) => {}
        }
    }

    pub fn scroll_up(&mut self, amount: usize) {
        self.scroll_offset = self.scroll_offset.saturating_add(amount);
        self.selection_state.clear();
        self.needs_redraw = true;
    }

    pub fn scroll_down(&mut self, amount: usize) {
        self.scroll_offset = self.scroll_offset.saturating_sub(amount);
        if self.scroll_offset == 0 {
            self.has_new_message = false;
        }
        self.selection_state.clear();
        self.needs_redraw = true;
    }

    /// Conditionally scroll to bottom: only if already at bottom.
    /// If user has scrolled up, set new-message flag instead.
    pub fn auto_scroll_to_bottom(&mut self) {
        if self.scroll_offset == 0 {
            return; // Already at bottom
        }
        self.has_new_message = true;
        self.needs_redraw = true;
    }

    pub fn history_previous(&mut self) {
        let current = self.input_text();
        if let Some(text) = self.history.previous(&current) {
            self.textarea = TextArea::from(text.lines().map(String::from).collect::<Vec<_>>());
            self.textarea
                .set_cursor_line_style(ratatui::style::Style::default());
            let placeholder = if self.is_team_mode() {
                PLACEHOLDER_TEAM_GOAL
            } else {
                PLACEHOLDER_MESSAGE
            };
            self.textarea.set_placeholder_text(placeholder);
        }
    }

    pub fn history_next(&mut self) {
        if let Some(text) = self.history.next() {
            if text.is_empty() {
                self.reset_textarea();
            } else {
                self.textarea = TextArea::from(text.lines().map(String::from).collect::<Vec<_>>());
                self.textarea
                    .set_cursor_line_style(ratatui::style::Style::default());
                let placeholder = if self.is_team_mode() {
                    PLACEHOLDER_TEAM_GOAL
                } else {
                    PLACEHOLDER_MESSAGE
                };
                self.textarea.set_placeholder_text(placeholder);
            }
        }
    }

    /// Get current input text from the textarea.
    pub fn input_text(&self) -> String {
        self.textarea.lines().join("\n")
    }

    /// Attach an image, enforcing attachment count and total size limits.
    /// Returns an error message if limits are exceeded.
    pub fn attach_image(&mut self, attachment: ImageAttachment) -> Option<String> {
        use crate::tui::attachment::{MAX_ATTACHMENTS, MAX_TOTAL_IMAGE_BYTES};

        if self.pending_images.len() >= MAX_ATTACHMENTS {
            return Some(format!(
                "Maximum {MAX_ATTACHMENTS} attachments allowed per message."
            ));
        }
        let total_bytes: usize = self
            .pending_images
            .iter()
            .map(|img| img.size_bytes)
            .sum::<usize>()
            + attachment.size_bytes;
        if total_bytes > MAX_TOTAL_IMAGE_BYTES {
            return Some(format!(
                "Total attachment size would exceed {}. Remove some images first.",
                ImageAttachment::format_size(MAX_TOTAL_IMAGE_BYTES)
            ));
        }
        self.pending_images.push(attachment);
        self.needs_redraw = true;
        None
    }

    pub fn clear_attachments(&mut self) {
        self.pending_images.clear();
        self.needs_redraw = true;
    }

    pub fn has_attachments(&self) -> bool {
        !self.pending_images.is_empty()
    }

    /// Pre-render thinking block lines for caching.
    fn render_thinking(content: &str) -> Vec<Line<'static>> {
        use ratatui::style::{Color, Modifier, Style};
        use ratatui::text::{Line, Span};

        let mut lines = Vec::new();
        lines.push(Line::from(vec![Span::styled(
            "thinking:",
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::ITALIC | Modifier::BOLD),
        )]));
        for line in content.lines() {
            lines.push(Line::from(vec![Span::styled(
                format!("  {line}"),
                Style::default()
                    .fg(Color::DarkGray)
                    .add_modifier(Modifier::ITALIC),
            )]));
        }
        lines.push(Line::from(vec![Span::styled(
            "  ---",
            Style::default().fg(Color::DarkGray),
        )]));
        lines
    }

    /// Poll for new messages from other channels/processes (e.g. Telegram, `mika ask`).
    async fn poll_cross_channel_messages(&mut self) {
        let new_msgs = match self.db.load_messages_after(self.last_seen_msg_id).await {
            Ok(msgs) => msgs,
            Err(e) => {
                tracing::warn!("cross-channel poll failed: {e}");
                return;
            }
        };

        if new_msgs.is_empty() {
            return;
        }

        for msg in &new_msgs {
            // In inbox mode, skip internal messages but count them
            if self.inbox_mode && msg.internal {
                self.hidden_internal_count += 1;
                continue;
            }
            if let Some(chat_msg) = session_message_to_chat_message(msg) {
                self.messages.push(chat_msg);
            }
        }

        // Update watermark (always advance, even for filtered messages)
        if let Some(last) = new_msgs.last() {
            self.last_seen_msg_id = last.id;
        }
        // Auto-scroll: stays at bottom if scroll_offset == 0; preserves position otherwise.
        self.needs_redraw = true;
    }

    /// Poll for completed-but-undelivered callback tasks and inject them into the conversation.
    async fn poll_callback_tasks(&mut self) {
        // Look back 7 days for undelivered callbacks
        let since = mika_agent::timestamp::now_minus(chrono::Duration::days(7));
        let tasks = match self
            .db
            .get_undelivered_callback_tasks_for_session(&since, &self.session_id)
            .await
        {
            Ok(t) => t,
            Err(e) => {
                tracing::warn!("callback poll failed: {e}");
                return;
            }
        };

        let stale_threshold =
            chrono::Duration::minutes(mika_agent::agent::STALE_FAILED_CALLBACK_MINUTES);
        let mut stale_skipped: usize = 0;

        for task in tasks {
            let is_failed = task.status == "failed";

            // Staleness guard: skip failed callbacks older than the threshold.
            // Completed callbacks are always delivered — they may carry legitimate results.
            if is_failed {
                let is_stale = task
                    .completed_at
                    .as_deref()
                    .is_some_and(|ts| mika_agent::timestamp::is_older_than(ts, stale_threshold));
                if is_stale {
                    // Silently mark as delivered without injecting into conversation
                    match self.db.mark_task_delivered(&task.id).await {
                        Ok(true) => {
                            stale_skipped += 1;
                            tracing::debug!(
                                task_id = %task.id,
                                label = %task.label,
                                "skipped stale failed callback"
                            );
                        }
                        Ok(false) => {} // already claimed
                        Err(e) => {
                            tracing::warn!(task_id = %task.id, "failed to mark stale task delivered: {e}");
                        }
                    }
                    continue;
                }
            }

            // Atomically claim this task — prevents double-processing by multiple TUI instances
            match self.db.mark_task_delivered(&task.id).await {
                Ok(true) => {}
                Ok(false) => continue, // already claimed by another instance
                Err(e) => {
                    tracing::warn!(task_id = %task.id, "failed to mark task delivered: {e}");
                    continue;
                }
            }

            let result = match task.result {
                Some(r) if !r.is_empty() => r,
                _ if is_failed => mika_agent::agent::FAILED_TASK_FALLBACK.to_string(),
                _ => continue, // skip completed tasks with no result
            };

            // Show a system message that a callback arrived
            let status_label = if is_failed { "failed" } else { "completed" };
            self.messages.push(ChatMessage {
                role: ChatRole::System,
                content: format!("[{}] {}", task.label, status_label),
                rendered: None,
                channel: None,
                internal: false,
            });

            let original_status = task.status.clone();

            // Send to agent worker for processing
            let _ = self.agent_tx.send(AgentRequest::CallbackResult {
                task_id: task.id,
                label: task.label,
                result,
                trace_id: task.created_trace_id,
                original_status,
                parent_task_id: task.parent_task_id,
            });

            self.status = AgentStatus::Thinking;
            self.needs_redraw = true;
            // Only inject one callback per tick to keep the agent responsive
            break;
        }

        // Show a single summary for skipped stale failures
        if stale_skipped > 0 {
            let noun = if stale_skipped == 1 { "task" } else { "tasks" };
            self.messages.push(ChatMessage {
                role: ChatRole::System,
                content: format!(
                    "Cleared {} stale failed {} (older than {} minutes)",
                    stale_skipped,
                    noun,
                    mika_agent::agent::STALE_FAILED_CALLBACK_MINUTES
                ),
                rendered: None,
                channel: None,
                internal: false,
            });
            self.needs_redraw = true;
        }
    }

    pub(crate) fn reset_textarea(&mut self) {
        self.textarea = TextArea::default();
        self.textarea
            .set_cursor_line_style(ratatui::style::Style::default());
        let placeholder = if self.is_team_mode() {
            PLACEHOLDER_TEAM_GOAL
        } else {
            PLACEHOLDER_MESSAGE
        };
        self.textarea.set_placeholder_text(placeholder);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // === InputHistory tests ===

    #[test]
    fn test_history_empty_previous_returns_none() {
        let mut h = InputHistory::new();
        assert!(h.previous("current").is_none());
    }

    #[test]
    fn test_history_push_and_previous() {
        let mut h = InputHistory::new();
        h.push("first".to_string());
        h.push("second".to_string());

        assert_eq!(h.previous("").unwrap(), "second");
        assert_eq!(h.previous("").unwrap(), "first");
    }

    #[test]
    fn test_history_cycling_past_oldest_stays() {
        let mut h = InputHistory::new();
        h.push("only".to_string());

        assert_eq!(h.previous("").unwrap(), "only");
        // Calling previous again stays at oldest
        assert_eq!(h.previous("").unwrap(), "only");
    }

    #[test]
    fn test_history_next_restores_draft() {
        let mut h = InputHistory::new();
        h.push("entry1".to_string());
        h.push("entry2".to_string());

        // Navigate into history (saves "my draft" as draft)
        assert_eq!(h.previous("my draft").unwrap(), "entry2");
        assert_eq!(h.previous("my draft").unwrap(), "entry1");

        // Navigate forward
        assert_eq!(h.next().unwrap(), "entry2");

        // Past newest restores draft
        assert_eq!(h.next().unwrap(), "my draft");
    }

    #[test]
    fn test_history_next_empty_draft() {
        let mut h = InputHistory::new();
        h.push("entry".to_string());

        // Navigate into history with empty input
        assert_eq!(h.previous("").unwrap(), "entry");

        // Navigate past newest — draft is empty
        assert!(h.next().unwrap().is_empty());
    }

    #[test]
    fn test_history_next_when_not_browsing() {
        let mut h = InputHistory::new();
        h.push("entry".to_string());

        // next() without previous() is a no-op
        assert!(h.next().is_none());
    }

    #[test]
    fn test_history_push_resets_navigation() {
        let mut h = InputHistory::new();
        h.push("first".to_string());

        // Enter history
        h.previous("draft");
        assert!(h.is_browsing());

        // Push resets
        h.push("second".to_string());
        assert!(!h.is_browsing());
    }

    #[test]
    fn test_history_max_size_cap() {
        let mut h = InputHistory::new();
        for i in 0..510 {
            h.push(format!("entry{i}"));
        }
        // Should be capped at 500
        assert_eq!(h.entries.len(), 500);
        // Oldest entries should be trimmed
        assert_eq!(h.entries[0], "entry10");
        assert_eq!(h.entries[499], "entry509");
    }

    #[test]
    fn test_history_push_empty_string_ignored() {
        let mut h = InputHistory::new();
        h.push("".to_string());
        assert!(h.entries.is_empty());
    }

    #[test]
    fn test_history_reset() {
        let mut h = InputHistory::new();
        h.push("entry".to_string());
        h.previous("draft");
        assert!(h.is_browsing());

        h.reset();
        assert!(!h.is_browsing());
    }

    // === InputHistory persistence tests ===

    #[test]
    fn test_history_load_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        let h = InputHistory::load(dir.path());
        assert!(h.entries.is_empty());
        assert!(h.file_path.is_some());
    }

    #[test]
    fn test_history_save_and_load_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let mut h = InputHistory::load(dir.path());
        h.push("hello".to_string());
        h.push("world\nwith newlines".to_string());

        let h2 = InputHistory::load(dir.path());
        assert_eq!(h2.entries.len(), 2);
        assert_eq!(h2.entries[0], "hello");
        assert_eq!(h2.entries[1], "world\nwith newlines");
    }

    #[test]
    fn test_history_load_corrupt_file() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(".input_history"), "not valid json{{{").unwrap();
        let h = InputHistory::load(dir.path());
        assert!(h.entries.is_empty());
    }

    #[test]
    fn test_history_load_empty_file() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(".input_history"), "").unwrap();
        let h = InputHistory::load(dir.path());
        assert!(h.entries.is_empty());
    }

    #[test]
    fn test_history_file_permissions() {
        let dir = tempfile::tempdir().unwrap();
        let mut h = InputHistory::load(dir.path());
        h.push("secret message".to_string());

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let path = dir.path().join(".input_history");
            let perms = std::fs::metadata(&path).unwrap().permissions();
            assert_eq!(perms.mode() & 0o777, 0o600);
        }
    }

    #[test]
    fn test_history_in_memory_does_not_save() {
        let mut h = InputHistory::new();
        h.push("entry".to_string());
        // No file_path, so save is a no-op — just verify no panic
        assert_eq!(h.entries.len(), 1);
        assert!(h.file_path.is_none());
    }

    // === Scroll behavior tests ===

    #[test]
    fn test_visual_line_rows_empty() {
        use crate::tui::ui::visual_line_rows;
        assert_eq!(visual_line_rows("", 80), 1);
    }

    #[test]
    fn test_visual_line_rows_short() {
        use crate::tui::ui::visual_line_rows;
        assert_eq!(visual_line_rows("hello", 80), 1);
    }

    #[test]
    fn test_visual_line_rows_exact_width() {
        use crate::tui::ui::visual_line_rows;
        // 10 chars in 10 width = 1 row
        assert_eq!(visual_line_rows("0123456789", 10), 1);
    }

    #[test]
    fn test_visual_line_rows_wraps() {
        use crate::tui::ui::visual_line_rows;
        // 11 chars in 10 width = 2 rows
        assert_eq!(visual_line_rows("01234567890", 10), 2);
    }

    #[test]
    fn test_visual_line_rows_zero_width() {
        use crate::tui::ui::visual_line_rows;
        assert_eq!(visual_line_rows("hello", 0), 1);
    }

    // === callback_label_from_metadata tests ===

    #[test]
    fn test_callback_label_with_valid_metadata() {
        let meta = Some(r#"{"callback_task_id":"abc","label":"analyze_codebase"}"#.to_string());
        assert_eq!(callback_label_from_metadata(&meta), "analyze_codebase");
    }

    #[test]
    fn test_callback_label_with_missing_label() {
        let meta = Some(r#"{"callback_task_id":"abc"}"#.to_string());
        assert_eq!(callback_label_from_metadata(&meta), "background task");
    }

    #[test]
    fn test_callback_label_with_none_metadata() {
        assert_eq!(callback_label_from_metadata(&None), "background task");
    }

    #[test]
    fn test_callback_label_with_invalid_json() {
        let meta = Some("not json".to_string());
        assert_eq!(callback_label_from_metadata(&meta), "background task");
    }

    // === format_previous_run_context tests ===

    fn make_test_summary(
        status: &str,
        deliverable: Option<&str>,
        critic: Option<&str>,
    ) -> TeamRunSummary {
        use mika_agent::db::{AgentResultSummary, TeamRunRow};
        TeamRunSummary {
            run: TeamRunRow {
                id: "abcd1234-5678-9abc-def0-1234567890ab".to_string(),
                team_name: "test-team".to_string(),
                goal: "Build a REST API".to_string(),
                status: status.to_string(),
                failure_reason: None,
                iteration: 1,
                max_iterations: 3,
                deliverable: deliverable.map(|s| s.to_string()),
                started_at: "2026-03-17T14:30:00Z".to_string(),
                ended_at: Some("2026-03-17T14:45:00Z".to_string()),
                trace_id: None,
            },
            agent_results: vec![
                AgentResultSummary {
                    agent_name: "alice".to_string(),
                    response_preview: "Done".to_string(),
                },
                AgentResultSummary {
                    agent_name: "bob".to_string(),
                    response_preview: "Done".to_string(),
                },
            ],
            task_statuses: vec![],
            pending_tasks: vec![],
            critic_feedback: critic.map(|s| s.to_string()),
        }
    }

    #[test]
    fn test_format_previous_run_context_completed() {
        let summary = make_test_summary(
            "completed",
            Some("REST endpoints created"),
            Some("Approved"),
        );
        let output = format_previous_run_context(&summary);

        assert!(output.contains("Previous Run Context"));
        assert!(output.contains("abcd1234"));
        assert!(output.contains("completed"));
        assert!(output.contains("Build a REST API"));
        assert!(output.contains("2026-03-17T14:30:00Z → 2026-03-17T14:45:00Z"));
        assert!(output.contains("alice, bob"));
        assert!(output.contains("Approved"));
        assert!(output.contains("REST endpoints created"));
    }

    #[test]
    fn test_format_previous_run_context_no_deliverable() {
        let summary = make_test_summary("failed", None, None);
        let output = format_previous_run_context(&summary);

        assert!(output.contains("failed"));
        assert!(output.contains("Deliverable: (none)"));
        assert!(!output.contains("Critic:"));
    }

    #[test]
    fn test_format_previous_run_context_suspended_status() {
        let summary = make_test_summary("suspended", None, None);
        let output = format_previous_run_context(&summary);

        assert!(output.contains("suspended — has pending callbacks"));
    }

    #[test]
    fn test_format_previous_run_context_running_status() {
        let summary = make_test_summary("running", None, None);
        let output = format_previous_run_context(&summary);

        assert!(output.contains("still in progress"));
    }

    #[test]
    fn test_format_previous_run_context_truncates_long_goal() {
        use mika_agent::db::TeamRunRow;
        let long_goal = "x".repeat(300);
        let summary = TeamRunSummary {
            run: TeamRunRow {
                id: "abcd1234-5678-9abc-def0-1234567890ab".to_string(),
                team_name: "t".to_string(),
                goal: long_goal,
                status: "completed".to_string(),
                failure_reason: None,
                iteration: 1,
                max_iterations: 1,
                deliverable: None,
                started_at: "2026-03-17T14:30:00Z".to_string(),
                ended_at: None,
                trace_id: None,
            },
            agent_results: vec![],
            task_statuses: vec![],
            pending_tasks: vec![],
            critic_feedback: None,
        };
        let output = format_previous_run_context(&summary);

        // Goal should be truncated to 200 chars + "..."
        assert!(output.contains("..."));
        // No ended_at means only started_at shown
        assert!(output.contains("Time:   2026-03-17T14:30:00Z"));
        assert!(!output.contains("→"));
        // No agents listed
        assert!(!output.contains("Agents:"));
    }

    #[test]
    fn test_format_previous_run_context_no_agents() {
        use mika_agent::db::TeamRunRow;
        let summary = TeamRunSummary {
            run: TeamRunRow {
                id: "abcd1234-0000-0000-0000-000000000000".to_string(),
                team_name: "t".to_string(),
                goal: "Test".to_string(),
                status: "completed".to_string(),
                failure_reason: None,
                iteration: 1,
                max_iterations: 1,
                deliverable: Some("Done".to_string()),
                started_at: "2026-03-17T14:30:00Z".to_string(),
                ended_at: Some("2026-03-17T14:35:00Z".to_string()),
                trace_id: None,
            },
            agent_results: vec![],
            task_statuses: vec![],
            pending_tasks: vec![],
            critic_feedback: None,
        };
        let output = format_previous_run_context(&summary);

        assert!(!output.contains("Agents:"));
        assert!(output.contains("Done"));
    }
}
