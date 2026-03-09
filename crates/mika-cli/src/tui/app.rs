use std::path::{Path, PathBuf};
use std::sync::Arc;

use ratatui::layout::Rect;
use ratatui::text::Line;
use tokio::sync::mpsc;
use tui_textarea::TextArea;

use mika_agent::async_db::AsyncDatabase;
use mika_agent::skills::SkillRegistry;
use mika_agent::teams::types::{TeamEvent, TeamPhase};
use mika_common::claude::ClaudeClient;

use crate::tui::attachment::ImageAttachment;
use crate::tui::commands;
use crate::tui::commands::autocomplete::AutocompleteState;
use crate::tui::markdown;

/// Messages flowing between the TUI and the team worker.
pub enum TeamRequest {
    Goal(String),
    Quit,
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
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ChatRole {
    User,
    Assistant,
    System,
    Command,
    Thinking,
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
        message_idx: usize,
        anchor: TextPosition,
        current: TextPosition,
    },
    /// Selection complete (mouse released).
    Selected {
        message_idx: usize,
        start: TextPosition,
        end: TextPosition,
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

/// Messages flowing between the TUI and the agent worker.
pub enum AgentRequest {
    Message {
        text: String,
        images: Vec<ImageAttachment>,
        thinking_budget: Option<u32>,
    },
    /// A background callback task has completed — inject into the conversation.
    CallbackResult {
        task_id: String,
        label: String,
        result: String,
    },
    SetModel {
        model: String,
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

    // Shared resources for slash commands
    pub db: AsyncDatabase,
    pub claude: ClaudeClient,
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
    /// Live dashboard state during team runs (created on first PhaseChanged event).
    pub team_dashboard: Option<TeamDashboardState>,
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
        claude: ClaudeClient,
        home_dir: PathBuf,
        skills: Arc<SkillRegistry>,
        agent_name: String,
        global_home: PathBuf,
    ) -> Self {
        let mut textarea = TextArea::default();
        textarea.set_cursor_line_style(ratatui::style::Style::default());
        textarea.set_placeholder_text("Type a message...");

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
            tick_count: 0,
            needs_redraw: true,
            selection_state: SelectionState::default(),
            messages_layout: MessagesLayout::default(),
            messages_inner_rect: None,
            db,
            claude,
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
            team_tx: None,
            team_rx: None,
            team_name: None,
            team_dir: None,
            verbose_mode: false,
            team_dashboard: None,
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
    ) -> Self {
        let mut textarea = TextArea::default();
        textarea.set_cursor_line_style(ratatui::style::Style::default());
        textarea.set_placeholder_text("Type a goal for the team...");

        // Use dummy agent channels — team mode does not use them.
        let (agent_tx, _) = mpsc::unbounded_channel::<AgentRequest>();
        let (_, agent_rx) = mpsc::unbounded_channel::<AgentResponse>();

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
            history: InputHistory::load(&team_dir),
            has_new_message: false,
            session_id: String::new(),
            model: String::new(),
            identity_name: format!("Team: {team_name}"),
            tick_count: 0,
            needs_redraw: true,
            selection_state: SelectionState::default(),
            messages_layout: MessagesLayout::default(),
            messages_inner_rect: None,
            // Team mode does not use agent-specific resources. These are set to
            // safe defaults. Slash command handlers check `is_team_mode()` before
            // accessing them.
            db,
            claude: ClaudeClient::dummy(),
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
            team_tx: Some(team_tx),
            team_rx: Some(team_rx),
            team_name: Some(team_name.to_string()),
            team_dir: Some(team_dir),
            verbose_mode: false,
            team_dashboard: None,
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
                });
                self.auto_scroll_to_bottom();
            }
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
                        });
                    }

                    if response.content.is_empty() {
                        // Agent responded with tool-use only (no text) — show feedback
                        self.messages.push(ChatMessage {
                            role: ChatRole::System,
                            content: mika_agent::agent::EMPTY_RESPONSE_FALLBACK.to_string(),
                            rendered: None,
                            channel: None,
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
            self.textarea.set_placeholder_text("Type a message...");
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
                self.textarea.set_placeholder_text("Type a message...");
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
            let role = match msg.role.as_str() {
                "user" => {
                    // Skip stale framing messages saved before the callback save fix
                    if msg.content.starts_with("A background task has completed.") {
                        continue;
                    }
                    ChatRole::User
                }
                "assistant" => ChatRole::Assistant,
                "tool_result" => ChatRole::System,
                _ => continue,
            };
            // CLI messages from other processes don't need a channel badge
            // (same channel as the TUI), matching the history loader behavior.
            let channel = if msg.channel_type == "cli" {
                None
            } else {
                Some(msg.channel_type.clone())
            };
            // For tool_result, show a brief summary with label from metadata
            let content = if msg.role == "tool_result" {
                let label = callback_label_from_metadata(&msg.metadata);
                format!("[Task: {}] Result received", label)
            } else {
                msg.content.clone()
            };
            self.messages.push(ChatMessage {
                role,
                content,
                rendered: None,
                channel,
            });
        }

        // Update watermark
        if let Some(last) = new_msgs.last() {
            self.last_seen_msg_id = last.id;
        }
        // Auto-scroll: stays at bottom if scroll_offset == 0; preserves position otherwise.
        self.needs_redraw = true;
    }

    /// Poll for completed-but-undelivered callback tasks and inject them into the conversation.
    async fn poll_callback_tasks(&mut self) {
        // Look back 7 days for undelivered callbacks
        let since = chrono::Utc::now().timestamp() - 7 * 24 * 3600;
        let tasks = match self.db.get_undelivered_callback_tasks(since).await {
            Ok(t) => t,
            Err(e) => {
                tracing::warn!("callback poll failed: {e}");
                return;
            }
        };

        for task in tasks {
            // Atomically claim this task — prevents double-processing by multiple TUI instances
            match self.db.mark_task_delivered(&task.id).await {
                Ok(true) => {}
                Ok(false) => continue, // already claimed by another instance
                Err(e) => {
                    tracing::warn!(task_id = %task.id, "failed to mark task delivered: {e}");
                    continue;
                }
            }

            let result = task.result.unwrap_or_default();
            if result.is_empty() {
                continue;
            }

            // Show a system message that a callback arrived
            self.messages.push(ChatMessage {
                role: ChatRole::System,
                content: format!("[{}] completed", task.label),
                rendered: None,
                channel: None,
            });

            // Send to agent worker for processing
            let _ = self.agent_tx.send(AgentRequest::CallbackResult {
                task_id: task.id,
                label: task.label,
                result,
            });

            self.status = AgentStatus::Thinking;
            self.needs_redraw = true;
            // Only inject one callback per tick to keep the agent responsive
            break;
        }
    }

    fn reset_textarea(&mut self) {
        self.textarea = TextArea::default();
        self.textarea
            .set_cursor_line_style(ratatui::style::Style::default());
        let placeholder = if self.is_team_mode() {
            "Type a goal for the team..."
        } else {
            "Type a message..."
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
}
