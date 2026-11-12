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

use crate::commands::tasks::CallbackFeed;
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
        "assistant" => {
            // mika#1667: tool-only EndTurn turns persist an empty-content assistant
            // row (carrying tool_calls metadata) alongside the separate send_message
            // body row. Rendering it produces a bare "<agent>:" header with no body.
            // Skip it here; the row stays in the DB for introspection/replay.
            if msg.content.trim().is_empty() {
                return None;
            }
            ChatRole::Assistant
        }
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
        /// The parent task ID, if the callback task has a parent linkage.
        /// Threaded through to `build_callback_trigger_context()` so the agent
        /// knows which task this callback relates to. See #313.
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

/// Events the worker sends back to the TUI. `Reply` carries the normal
/// turn result; `WorkerCrashed` is emitted by the worker supervisor when
/// the spawned worker task exits unexpectedly (panic or premature clean exit
/// while the session is still active). See mika#1149.
pub enum AgentResponse {
    Reply {
        content: String,
        is_error: bool,
        thinking: Option<String>,
        input_tokens: Option<u32>,
        /// If skills were hot-reloaded during this turn, contains the new registry.
        updated_skills: Option<Arc<SkillRegistry>>,
    },
    /// Un avis du worker à destination de l'opérateur, rendu en `System` et
    /// sans rapport avec la réussite du tour (mika#1727 V1 § D3 : les entrées de
    /// la TUI qui n'atteignent plus le tour se disent à voix haute).
    ///
    /// Distinct de `Reply { is_error: true }` : un avis n'est pas un échec, et
    /// il peut précéder une réponse réussie dans le même tour.
    Notice { content: String },
    /// The agent worker task exited unexpectedly. `/restart` recovers.
    WorkerCrashed { reason: String },
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
    /// True when this agent is single-session-by-nature (mika#1401): `session_id`
    /// is the agent's canonical session. Gates `/clear` (preserves the session
    /// instead of minting a new one) and the session-teardown `end_session` calls.
    pub is_singleton_session: bool,
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

    /// True after the worker supervisor reported a crash (mika#1149). Set by
    /// `tick_agent_mode` on `AgentResponse::WorkerCrashed`. Cleared by a
    /// successful `/restart`. Gates `/restart`'s healthy-worker refusal.
    pub worker_crashed: bool,

    /// If true, the chat loop should tear down the dead worker and spawn a
    /// fresh one on the next iteration. Set by the `/restart` handler.
    pub pending_restart: bool,

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
    /// Cached count of actively-executing background tasks (process_id IS NOT NULL).
    /// Agent-scoped — NOT reset on /clear.
    pub executing_task_count: usize,
    /// Cached count of queued background tasks (process_id IS NULL).
    /// Agent-scoped — NOT reset on /clear.
    pub queued_task_count: usize,

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

    /// La couture vers le fil des tâches (mika#1727, vol V2b-2b-2). Posée après
    /// construction, comme `dashboard_running` ; `None` en mode équipe et quand
    /// l'ouverture échoue — le minuteur de `poll_callback_tasks` couvre alors,
    /// et seulement alors depuis le vol V2b-3b : une couture qui suit l'agent
    /// courant le fait taire, une qui suit un autre agent le laisse balayer.
    pub callback_feed: Option<CallbackFeed>,

    /// La dernière session que le drainage a balayée (mika#1727, vol V2b-3b).
    ///
    /// `None` tant qu'aucune ne l'a été, ce qui fait du premier drainage un
    /// balayage : au lancement, tout callback fini de cette session est un trou.
    /// Une `Option<String>` et non un booléen — c'est la **comparaison** qui
    /// porte la propriété, un « déjà balayé » ne couvrirait jamais la seconde
    /// session.
    derniere_session_balayee: Option<String>,
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
        start_in_audit_mode: bool,
        is_singleton_session: bool,
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
            is_singleton_session,
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
            worker_crashed: false,
            pending_restart: false,
            pending_images: Vec::new(),
            thinking_level: None,
            context_tokens: None,
            last_seen_msg_id: 0,
            pending_task_count: 0,
            executing_task_count: 0,
            queued_task_count: 0,
            team_tx: None,
            team_rx: None,
            team_name: None,
            team_dir: None,
            verbose_mode: false,
            // When --inbox flag is set, start in audit mode (inbox_mode=false shows internals).
            inbox_mode: !start_in_audit_mode,
            hidden_internal_count: 0,
            team_dashboard: None,
            dashboard_running: false,
            footer_open_rect: None,
            footer_start_stop_rect: None,
            pending_dashboard_action: None,
            callback_feed: None,
            derniere_session_balayee: None,
        }
    }

    /// Create a new App in team mode.
    #[allow(clippy::too_many_arguments)]
    pub fn new_team(
        team_tx: mpsc::UnboundedSender<TeamRequest>,
        team_rx: mpsc::UnboundedReceiver<TeamEvent>,
        team_name: &str,
        team_dir: PathBuf,
        global_home: PathBuf,
        db: AsyncDatabase,
        previous_run: Option<TeamRunSummary>,
        start_in_audit_mode: bool,
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
            is_singleton_session: false,
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
            worker_crashed: false,
            pending_restart: false,
            pending_images: Vec::new(),
            thinking_level: None,
            context_tokens: None,
            last_seen_msg_id: 0,
            pending_task_count: 0,
            executing_task_count: 0,
            queued_task_count: 0,
            team_tx: Some(team_tx),
            team_rx: Some(team_rx),
            team_name: Some(team_name.to_string()),
            team_dir: Some(team_dir),
            verbose_mode: false,
            // When --inbox flag is set, start in audit mode (inbox_mode=false shows internals).
            inbox_mode: !start_in_audit_mode,
            hidden_internal_count: 0,
            team_dashboard: None,
            dashboard_running: false,
            footer_open_rect: None,
            footer_start_stop_rect: None,
            pending_dashboard_action: None,
            callback_feed: None,
            derniere_session_balayee: None,
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

        // Post-crash guard (mika#1150 cohort): once `WorkerCrashed` has been
        // surfaced the worker's `user_rx` is gone. Without this guard
        // `let _ = self.agent_tx.send(...)` below swallows the resulting
        // `SendError`, the user message is still persisted/rendered, and the
        // "Thinking" spinner is set — recreating the silent-drop UX that
        // mika#1149's supervisor was meant to surface. Refuse the send and
        // point the operator at the recovery affordance.
        if self.worker_crashed {
            self.messages.push(ChatMessage {
                role: ChatRole::System,
                content: "Agent worker has crashed. Run /restart before sending more messages."
                    .to_string(),
                rendered: None,
                channel: None,
            });
            self.reset_textarea();
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
                                crate::commands::dashboard::spirit_url()
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
                                Err(e) => format!("Failed to reach mika-spirit: {e}"),
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
            && let Ok(counts) = self.db.get_background_task_counts().await
            && (counts.executing != self.executing_task_count
                || counts.queued != self.queued_task_count)
        {
            self.executing_task_count = counts.executing;
            self.queued_task_count = counts.queued;
            self.needs_redraw = true;
        }

        // Livraison par le fil (mika#1727, vol V2b-2b-2), **sans**
        // `POLL_INTERVAL_TICKS` : cette attente-là est ce que le fil rachète.
        if !self.is_team_mode() && self.status == AgentStatus::Idle {
            self.drain_callback_deliveries().await;
        }

        // Le minuteur ne balaie plus que là où la couture ne couvre pas
        // (mika#1727, vol V2b-3b) : sans couture, et quand celle qui existe suit
        // un **autre** agent — l'agent du fil est figé à son ouverture, celui de
        // la TUI change. Sous une couture qui suit l'agent courant, le balayage
        // vient du signal du fil et du changement de session, tous deux portés
        // par le drainage ci-dessus.
        //
        // `is_some_and` nié : une couture absente rend `false`, donc la négation
        // rend `true` et le minuteur balaie. Le biais va vers le balayage, jamais
        // vers le silence — une livraison de trop est réclamée, une de moins est
        // un tour qui ne repart pas.
        if !self.is_team_mode()
            && self.tick_count.is_multiple_of(POLL_INTERVAL_TICKS)
            && self.status == AgentStatus::Idle
            && !self
                .callback_feed
                .as_ref()
                .is_some_and(|f| f.suit_lagent(&self.agent_name))
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
            Ok(AgentResponse::Reply {
                content,
                is_error,
                thinking,
                input_tokens,
                updated_skills,
            }) => {
                // Update skills if hot-reloaded during this turn
                if let Some(new_skills) = updated_skills {
                    self.skills = new_skills;
                }

                // Update context token tracking
                if let Some(tokens) = input_tokens {
                    self.context_tokens = Some(tokens);
                }

                if is_error {
                    self.messages.push(ChatMessage {
                        role: ChatRole::System,
                        content: format!("Error: {content}"),
                        rendered: None,
                        channel: None,
                    });
                    self.status = AgentStatus::Idle;
                } else {
                    // Show thinking block (instantly, not progressively revealed)
                    if let Some(thinking) = thinking {
                        let rendered = Self::render_thinking(&thinking);
                        self.messages.push(ChatMessage {
                            role: ChatRole::Thinking,
                            content: thinking,
                            rendered: Some(rendered),
                            channel: None,
                        });
                    }

                    if content.is_empty() {
                        // Agent responded with tool-use only (no text) — show feedback
                        self.messages.push(ChatMessage {
                            role: ChatRole::System,
                            content: mika_agent::agent::EMPTY_RESPONSE_FALLBACK.to_string(),
                            rendered: None,
                            channel: None,
                        });
                        self.status = AgentStatus::Idle;
                    } else {
                        self.pending_response = Some(content);
                        self.reveal_index = 0;
                        self.status = AgentStatus::Responding(0);
                    }
                }
                self.needs_redraw = true;
            }
            Ok(AgentResponse::Notice { content }) => {
                // mika#1727 V1 § D3 — un avis se rend, et ne touche NI le statut
                // NI `pending_response` : le tour qu'il annote est encore en
                // vol, et remettre le spinner à `Idle` ici ferait croire que la
                // réponse est arrivée.
                self.messages.push(ChatMessage {
                    role: ChatRole::System,
                    content,
                    rendered: None,
                    channel: None,
                });
                self.needs_redraw = true;
            }
            Ok(AgentResponse::WorkerCrashed { reason }) => {
                // Worker died — surface a visible error, clear the spinner,
                // and arm /restart. Silent toasts defeat the point (mika#1149).
                self.messages.push(ChatMessage {
                    role: ChatRole::System,
                    content: format!(
                        "\u{26a0} Agent worker crashed: {reason}. Use /restart to recover."
                    ),
                    rendered: None,
                    channel: None,
                });
                self.status = AgentStatus::Idle;
                self.worker_crashed = true;
                self.needs_redraw = true;

                // Persist the crash to audit_events for post-incident triage
                // (mika#1150 cohort). Fire-and-forget on a fresh task: the
                // tick loop is the input path, and AsyncDatabase is most
                // likely to back-pressure precisely on this code path (the
                // worker may have crashed mid-DB-closure). The structured
                // `agent_worker_silenced` log event from the supervisor
                // (chat.rs) remains the primary observability surface; this
                // audit row complements it with session-scoped grep.
                let db = self.db.clone();
                let session = self.session_id.clone();
                tokio::spawn(async move {
                    if let Err(e) = db
                        .log_audit_event(
                            &session,
                            "system",
                            "agent_worker_crashed",
                            None,
                            Some(&reason),
                            None,
                            None,
                        )
                        .await
                    {
                        tracing::warn!(
                            error = %e,
                            "failed to log agent_worker_crashed audit event"
                        );
                    }
                });
            }
            Err(mpsc::error::TryRecvError::Disconnected) => {
                // Channel closed without a WorkerCrashed event in flight. With
                // supervision (mika#1149) this is rare — typically only happens
                // if the supervisor itself is torn down ahead of the channel.
                if self.status == AgentStatus::Thinking {
                    self.messages.push(ChatMessage {
                        role: ChatRole::System,
                        content: "Agent worker stopped unexpectedly.".to_string(),
                        rendered: None,
                        channel: None,
                    });
                    self.status = AgentStatus::Idle;
                    self.worker_crashed = true;
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

    /// Les gardes 3 et 4 du vol V2b-2b-2 : réclamer, puis livrer un candidat que
    /// la couture a déjà jugé recevable. Le vol V2b-2b-3 y amène les `failed`.
    ///
    /// La parité avec [`Self::poll_callback_tasks`] **est** le contrat, et son
    /// échéance est le vol qui sortira la lecture vers spirit : ce qui n'est pas
    /// ici ce jour-là est perdu. Le vol V2b-3b, lui, ne retire pas le balayage —
    /// il le retire du **minuteur**, ce qui suffit déjà à rendre la parité
    /// exigible : sous une couture qui suit l'agent courant, ce qui manque ici
    /// n'est plus rattrapé cinq secondes plus tard. Deux divergences, voulues.
    ///
    /// L'ordre. Le statut se lit **avant** la réclamation, qui écrit `delivered`
    /// sur la ligne ; la garde d'ancienneté, elle, s'applique **après**, avec un
    /// `continue`. Le poll réclame en deux endroits parce qu'il tient un compteur
    /// de lignes écartées — ici une réclamation unique suffit.
    ///
    /// L'absence du résumé `"Cleared N stale failed task(s)"`. Le critère 3 veut
    /// qu'un `failed` périmé ne pousse **aucun** message système, et un décompte
    /// à la ligne serait du bruit. L'écart se lit dans les traces `debug`.
    ///
    /// Un candidat refusé par la réclamation **ne rend pas la main** : sinon une
    /// ligne que le poll vient de prendre retarderait d'un tick celle qui la suit
    /// dans le canal.
    async fn drain_callback_deliveries(&mut self) {
        // Le fil ne rejoue rien : à son ouverture comme après un `OverflowMarker`,
        // il dit seulement qu'il a pu manquer quelque chose. Le balayage est la
        // lecture que le poll fait déjà — appelée, jamais recopiée.
        //
        // Deux sources, un seul balayage (vol V2b-3b). Le signal est pris
        // **avant** le `if` et hors de tout court-circuit : placé en second dans
        // un `||`, il survivrait au balayage que le changement de session vient
        // de faire, et le passage suivant en ferait un second. Un passage, un
        // appel.
        let signal = self
            .callback_feed
            .as_ref()
            .is_some_and(|f| f.prendre_le_rattrapage());
        // La requête du balayage filtre sur `session_id` : un balayage fait sous
        // l'ancienne session ne couvre pas la nouvelle, d'où la comparaison
        // plutôt qu'un drapeau. Le drainage tourne à chaque tick, donc la
        // détection coûte un tick, pas un tour de minuteur.
        let session_neuve =
            self.derniere_session_balayee.as_deref() != Some(self.session_id.as_str());
        if signal || session_neuve {
            // Retenue même quand le balayage vient du signal : la requête filtre
            // sur `session_id`, donc cette session **est** balayée dans les deux
            // cas.
            self.derniere_session_balayee = Some(self.session_id.clone());
            self.poll_callback_tasks().await;
            // Une livraison relance le tour, et la boucle du fil ne doit pas en
            // empiler une seconde : même règle que le `break` du poll.
            if self.status != AgentStatus::Idle {
                return;
            }
        }
        // Copiées avant la boucle : `next_candidate` emprunte la couture, donc
        // `self`. Elles voyagent par appel — `/new` les réassigne.
        let agent = self.agent_name.clone();
        let session = self.session_id.clone();
        let stale_threshold =
            chrono::Duration::minutes(mika_agent::planning::policy::STALE_FAILED_CALLBACK_MINUTES);
        loop {
            let Some(feed) = self.callback_feed.as_mut() else {
                return;
            };
            let Some(detail) = feed.next_candidate(&agent, &session).await else {
                return;
            };

            // Lu avant la réclamation, qui écrit `delivered` sur la ligne. Le
            // prédicat de la couture garantit `completed` ou `failed`.
            let est_un_echec = detail.status == "failed";

            match self.db.mark_task_delivered(&detail.id).await {
                Ok(true) => {}
                Ok(false) => continue, // le poll ou une autre TUI l'a déjà prise
                Err(e) => {
                    tracing::warn!(task_id = %detail.id, "failed to mark task delivered: {e}");
                    continue;
                }
            }

            // Un `failed` trop vieux ne vaut plus pour la conversation en cours ;
            // un `completed`, lui, vaut encore. `is_older_than` rend `false` sur
            // un horodatage illisible et `is_some_and` sur son absence : les deux
            // biais vont vers la livraison, jamais vers le silence.
            if est_un_echec
                && detail
                    .completed_at
                    .as_deref()
                    .is_some_and(|ts| mika_agent::timestamp::is_older_than(ts, stale_threshold))
            {
                tracing::debug!(
                    task_id = %detail.id,
                    label = %detail.label,
                    "skipped stale failed callback"
                );
                continue;
            }

            let result = match detail.result {
                Some(r) if !r.is_empty() => r,
                _ if est_un_echec => mika_agent::agent::FAILED_TASK_FALLBACK.to_string(),
                _ => continue, // un `completed` sans résultat est sauté, comme au poll
            };

            let status_label = if est_un_echec { "failed" } else { "completed" };
            self.messages.push(ChatMessage {
                role: ChatRole::System,
                content: format!("[{}] {}", detail.label, status_label),
                rendered: None,
                channel: None,
            });

            let _ = self.agent_tx.send(AgentRequest::CallbackResult {
                task_id: detail.id,
                label: detail.label,
                result,
                trace_id: detail.created_trace_id,
                original_status: detail.status,
                parent_task_id: detail.parent_task_id,
            });

            self.status = AgentStatus::Thinking;
            self.needs_redraw = true;
            return;
        }
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
            chrono::Duration::minutes(mika_agent::planning::policy::STALE_FAILED_CALLBACK_MINUTES);
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
                    mika_agent::planning::policy::STALE_FAILED_CALLBACK_MINUTES
                ),
                rendered: None,
                channel: None,
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

    // === session_message_to_chat_message tests (mika#1667) ===

    fn session_message(role: &str, content: &str) -> mika_agent::db::SessionMessage {
        mika_agent::db::SessionMessage {
            id: 1,
            session_id: "test-session".to_string(),
            agent_id: "mika-dev".to_string(),
            role: role.to_string(),
            content: content.to_string(),
            channel_type: "cli".to_string(),
            metadata: None,
            trace_id: None,
            created_at: "2026-07-02T00:00:00Z".to_string(),
            internal: false,
        }
    }

    #[test]
    fn test_empty_assistant_row_skipped() {
        let msg = session_message("assistant", "");
        assert!(session_message_to_chat_message(&msg).is_none());
    }

    #[test]
    fn test_whitespace_assistant_row_skipped() {
        let msg = session_message("assistant", "   \n");
        assert!(session_message_to_chat_message(&msg).is_none());
    }

    #[test]
    fn test_nonempty_assistant_row_rendered() {
        let msg = session_message("assistant", "real reply");
        let chat = session_message_to_chat_message(&msg).expect("should render");
        assert_eq!(chat.role, ChatRole::Assistant);
        assert_eq!(chat.content, "real reply");
    }

    #[test]
    fn test_empty_user_row_still_rendered() {
        // The skip is assistant-scoped; empty user rows are unaffected
        // (regression guard against over-broad filtering).
        let msg = session_message("user", "");
        let chat = session_message_to_chat_message(&msg).expect("should render");
        assert_eq!(chat.role, ChatRole::User);
    }

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
                delegation_count: 0,
                solo_absorption: false,
                failure_context: None,
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
                delegation_count: 0,
                solo_absorption: false,
                failure_context: None,
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
                delegation_count: 0,
                solo_absorption: false,
                failure_context: None,
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

    // === tick_agent_mode WorkerCrashed handling (mika#1149) ===

    /// Build a minimal `App` whose `agent_rx` is paired with the returned sender,
    /// so a test can simulate the supervisor delivering events to the TUI.
    /// Le receveur des `AgentRequest` est **rendu**, non jeté : « livré une seule
    /// fois » ne se mesure que là où la livraison atterrit (mika#1727).
    async fn app_with_response_sender() -> (
        App<'static>,
        mpsc::UnboundedSender<AgentResponse>,
        mpsc::UnboundedReceiver<AgentRequest>,
        tempfile::TempDir,
    ) {
        use mika_agent::async_db::AsyncDatabase;
        use mika_agent::db::Database;
        use mika_agent::skills::SkillRegistry;

        let temp_dir = tempfile::tempdir().unwrap();
        let home_dir = temp_dir.path().to_path_buf();
        let global_home = temp_dir.path().to_path_buf();
        let db = Database::open_in_memory().unwrap();
        let async_db = AsyncDatabase::new(db);
        let session_id = uuid::Uuid::new_v4().to_string();
        async_db
            .create_session(&session_id, "mika", "cli")
            .await
            .unwrap();

        let (agent_tx, request_rx) = mpsc::unbounded_channel();
        let (response_tx, response_rx) = mpsc::unbounded_channel();

        let app = App::new(
            agent_tx,
            response_rx,
            session_id,
            "test-model".to_string(),
            "Mika".to_string(),
            async_db,
            mika_common::llm::dummy_provider(),
            home_dir,
            Arc::new(SkillRegistry::empty()),
            "mika".to_string(),
            global_home,
            mika_common::llm::ProviderKind::Anthropic,
            false, // start_in_audit_mode
            false, // is_singleton_session (mika#1401)
        );

        (app, response_tx, request_rx, temp_dir)
    }

    #[tokio::test]
    async fn tick_agent_mode_surfaces_worker_crashed_event() {
        let (mut app, response_tx, _rrx, _td) = app_with_response_sender().await;
        app.status = AgentStatus::Thinking; // simulate an in-flight prompt

        response_tx
            .send(AgentResponse::WorkerCrashed {
                reason: "worker panicked: simulated tool panic".to_string(),
            })
            .unwrap();

        app.tick_agent_mode().await;

        assert!(app.worker_crashed, "must arm /restart by flipping the flag");
        assert_eq!(app.status, AgentStatus::Idle, "must clear the spinner");
        let last = app.messages.last().expect("banner pushed");
        assert!(
            matches!(last.role, ChatRole::System),
            "banner must be a System message, got {:?}",
            last.role
        );
        assert!(
            last.content.contains("Agent worker crashed"),
            "banner must mention the crash: {}",
            last.content
        );
        assert!(
            last.content.contains("simulated tool panic"),
            "banner must include the supervisor's reason: {}",
            last.content
        );
        assert!(
            last.content.contains("/restart"),
            "banner must point at the recovery affordance: {}",
            last.content
        );
    }

    #[tokio::test]
    async fn tick_agent_mode_replies_do_not_set_worker_crashed() {
        let (mut app, response_tx, _rrx, _td) = app_with_response_sender().await;
        app.status = AgentStatus::Thinking;

        response_tx
            .send(AgentResponse::Reply {
                content: "hello".to_string(),
                is_error: false,
                thinking: None,
                input_tokens: None,
                updated_skills: None,
            })
            .unwrap();

        app.tick_agent_mode().await;

        assert!(!app.worker_crashed, "Reply must not arm /restart");
        assert_eq!(app.pending_response.as_deref(), Some("hello"));
    }

    // === livraison d'un callback par le fil (mika#1727, vols V2b-2b-2 et -3) ===

    /// Une ligne de callback en base, un mika-spirit factice qui sert son détail,
    /// et une couture dont le canal porte son id. Les trois tiennent dans un seul
    /// helper parce qu'ils doivent porter le **même** id, choisi par la base :
    /// séparés, ils divergeraient en silence et les tests seraient verts sans
    /// rien livrer (mika#2205).
    ///
    /// Les trois paramètres sont les trois axes que le vol V2b-2b-3 fait varier,
    /// et le `statut` va **à la fois** sur la ligne et sur le détail : la ligne
    /// parce que `mark_task_delivered` ne prend que `completed` et `failed`, le
    /// détail parce que c'est lui que la livraison lit.
    async fn couture_dun_callback_livrable(
        app: &App<'static>,
        statut: &str,
        resultat: Option<&str>,
        completed_at: Option<&str>,
    ) -> (String, mpsc::UnboundedSender<String>, CallbackFeed) {
        use mika_agent::db::NewTask;

        let id = app
            .db
            .create_task(NewTask {
                agent_id: app.agent_name.clone(),
                team_run_id: None,
                // Ni parent ni trace : la livraison les lit sur le **détail**, et
                // les laisser divergents ici le prouve au lieu de le supposer.
                parent_task_id: None,
                depth: 1,
                label: "long_running:run_claude_pilot".to_string(),
                trigger_type: "callback".to_string(),
                cron_expr: None,
                event_source: None,
                event_offset_secs: None,
                condition_expr: None,
                next_fire_at: None,
                timeout_at: None,
                action_type: "resume_agent".to_string(),
                action_config: "{}".to_string(),
                input_context: None,
                created_by_session: Some(app.session_id.clone()),
                created_trace_id: None,
                reference_url: None,
                source: Some("self_dev".to_string()),
                metadata: None,
                r#type: None,
                dispatch_class: Some("implement".to_string()),
            })
            .await
            .expect("create callback row");
        app.db
            .update_task_status(&id, statut)
            .await
            .expect("set status");

        // Servi par HTTP : c'est lui, non la ligne, que la livraison lit.
        let corps = serde_json::json!({
            "id": id, "agent_id": app.agent_name, "status": statut,
            "label": "long_running:run_claude_pilot", "trigger_type": "callback",
            "action_type": "resume_agent", "created_by_session": app.session_id,
            "result": resultat, "created_trace_id": "tr-9",
            "parent_task_id": "t-parent", "completed_at": completed_at,
        })
        .to_string();
        let (base, _) = crate::commands::tasks::fake_spirit::spawn_detail(200, corps).await;
        let (tx, feed) = CallbackFeed::from_channel(&app.agent_name, base, "jeton".to_string());
        (id, tx, feed)
    }

    /// **Critère 1** — le même candidat est livré **une fois**, pas deux. Le
    /// canal porte son id deux fois ; au second tour la ligne est déjà réclamée,
    /// `mark_task_delivered` rend `Ok(false)`, la garde 3 passe, rien ne repart.
    #[tokio::test]
    async fn mika1727_v2b2b2_un_completed_du_fil_est_livre_une_seule_fois() {
        let (mut app, _rtx, mut request_rx, _td) = app_with_response_sender().await;
        let (id, tx, feed) =
            couture_dun_callback_livrable(&app, "completed", Some("le pilote a fini"), None).await;
        tx.send(id.clone()).unwrap();
        tx.send(id.clone()).unwrap();
        app.callback_feed = Some(feed);

        app.drain_callback_deliveries().await;

        let AgentRequest::CallbackResult {
            task_id,
            label,
            result,
            trace_id,
            original_status,
            parent_task_id,
        } = request_rx.try_recv().expect("une livraison")
        else {
            panic!("la livraison doit être un CallbackResult");
        };
        assert_eq!(task_id, id);
        assert_eq!(label, "long_running:run_claude_pilot");
        assert_eq!(result, "le pilote a fini", "le résultat vient du détail");
        assert_eq!(trace_id.as_deref(), Some("tr-9"));
        assert_eq!(original_status, "completed");
        assert_eq!(parent_task_id.as_deref(), Some("t-parent"));
        assert_eq!(app.status, AgentStatus::Thinking, "le tour repart");

        app.status = AgentStatus::Idle;
        app.drain_callback_deliveries().await;
        assert!(
            request_rx.try_recv().is_err(),
            "le second passage ne livre rien"
        );
    }

    /// **Critère 1** — rien n'est livré quand le poll a déjà réclamé la ligne. Sa
    /// réclamation est jouée telle quelle, par `mark_task_delivered` : c'est elle
    /// qui est atomique, et le drainage n'a aucun autre moyen de savoir que
    /// l'autre chemin est passé avant lui.
    #[tokio::test]
    async fn mika1727_v2b2b2_rien_nest_livre_si_le_poll_a_deja_reclame() {
        let (mut app, _rtx, mut request_rx, _td) = app_with_response_sender().await;
        let (id, tx, feed) =
            couture_dun_callback_livrable(&app, "completed", Some("le pilote a fini"), None).await;
        assert!(
            app.db.mark_task_delivered(&id).await.unwrap(),
            "le poll réclame le premier"
        );
        tx.send(id).unwrap();
        app.callback_feed = Some(feed);

        app.drain_callback_deliveries().await;

        assert!(request_rx.try_recv().is_err(), "rien n'est livré");
        assert_eq!(app.status, AgentStatus::Idle, "le tour ne repart pas");
        assert!(app.messages.is_empty(), "aucun message système");
    }

    // === livraison d'un `failed` par le fil (mika#1727, vol V2b-2b-3) ===

    /// **Critère 1** — un `failed` avec résultat est livré **une fois**, et il
    /// porte `original_status = "failed"`, lu sur le détail et non écrit en dur.
    /// Le canal porte son id deux fois : au second tour la ligne est déjà
    /// réclamée, `mark_task_delivered` rend `Ok(false)`, rien ne repart.
    #[tokio::test]
    async fn mika1727_v2b2b3_un_failed_du_fil_est_livre_une_seule_fois() {
        let (mut app, _rtx, mut request_rx, _td) = app_with_response_sender().await;
        let (id, tx, feed) =
            couture_dun_callback_livrable(&app, "failed", Some("le pilote a cassé"), None).await;
        tx.send(id.clone()).unwrap();
        tx.send(id.clone()).unwrap();
        app.callback_feed = Some(feed);

        app.drain_callback_deliveries().await;

        let AgentRequest::CallbackResult {
            task_id,
            label,
            result,
            trace_id,
            original_status,
            parent_task_id,
        } = request_rx.try_recv().expect("une livraison")
        else {
            panic!("la livraison doit être un CallbackResult");
        };
        assert_eq!(task_id, id);
        assert_eq!(label, "long_running:run_claude_pilot");
        assert_eq!(result, "le pilote a cassé", "le résultat vient du détail");
        assert_eq!(trace_id.as_deref(), Some("tr-9"));
        assert_eq!(original_status, "failed", "lu sur le détail");
        assert_eq!(parent_task_id.as_deref(), Some("t-parent"));
        assert_eq!(
            app.messages.last().map(|m| m.content.as_str()),
            Some("[long_running:run_claude_pilot] failed"),
            "le message système dit `failed`, comme au poll"
        );
        assert_eq!(app.status, AgentStatus::Thinking, "le tour repart");

        app.status = AgentStatus::Idle;
        app.drain_callback_deliveries().await;
        assert!(
            request_rx.try_recv().is_err(),
            "le second passage ne livre rien"
        );
    }

    /// **Critère 2** — un `failed` sans résultat est livré quand même, avec le
    /// texte de repli du poll. L'attendu est la constante elle-même : recopiée,
    /// le test resterait vert le jour où le poll et le drainage divergeraient.
    #[tokio::test]
    async fn mika1727_v2b2b3_un_failed_sans_resultat_prend_le_repli() {
        let (mut app, _rtx, mut request_rx, _td) = app_with_response_sender().await;
        let (id, tx, feed) = couture_dun_callback_livrable(&app, "failed", None, None).await;
        tx.send(id).unwrap();
        app.callback_feed = Some(feed);

        app.drain_callback_deliveries().await;

        let AgentRequest::CallbackResult { result, .. } =
            request_rx.try_recv().expect("une livraison")
        else {
            panic!("la livraison doit être un CallbackResult");
        };
        assert_eq!(result, mika_agent::agent::FAILED_TASK_FALLBACK);
    }

    /// **Critère 2** — l'autre moitié de la règle : un `completed` sans résultat
    /// reste sauté. Le voisin du test précédent, parce que c'est la **paire** qui
    /// est la garde : seul, chacun passerait avec un `if` qui ignore le statut.
    /// La ligne est tout de même réclamée — le drainage ne la rendra pas au poll.
    #[tokio::test]
    async fn mika1727_v2b2b3_un_completed_sans_resultat_reste_saute() {
        let (mut app, _rtx, mut request_rx, _td) = app_with_response_sender().await;
        let (id, tx, feed) = couture_dun_callback_livrable(&app, "completed", None, None).await;
        tx.send(id.clone()).unwrap();
        app.callback_feed = Some(feed);

        app.drain_callback_deliveries().await;

        assert!(request_rx.try_recv().is_err(), "rien n'est livré");
        assert!(app.messages.is_empty(), "aucun message système");
        assert_eq!(app.status, AgentStatus::Idle, "le tour ne repart pas");
    }

    /// **Critère 3** — un `failed` plus vieux que
    /// `STALE_FAILED_CALLBACK_MINUTES` est **réclamé sans être livré**. Les trois
    /// assertions sont les trois moitiés de cette phrase : aucun `CallbackResult`,
    /// aucun message système, et la ligne relue en `delivered` — sans quoi
    /// « écarté » serait indistinguable de « jamais vu », et le poll la
    /// reprendrait au tick suivant.
    ///
    /// L'horodatage est **dérivé du seuil**, pas écrit à la main : une minute
    /// au-delà de ce que la policy déclare. Un `6` en dur mentirait le jour où le
    /// seuil bouge, et le test serait vert sur la mauvaise raison.
    #[tokio::test]
    async fn mika1727_v2b2b3_un_failed_perime_est_reclame_sans_etre_livre() {
        let (mut app, _rtx, mut request_rx, _td) = app_with_response_sender().await;
        let trop_vieux = mika_agent::timestamp::now_minus(chrono::Duration::minutes(
            mika_agent::planning::policy::STALE_FAILED_CALLBACK_MINUTES + 1,
        ));
        let (id, tx, feed) = couture_dun_callback_livrable(
            &app,
            "failed",
            Some("le pilote a cassé il y a longtemps"),
            Some(&trop_vieux),
        )
        .await;
        tx.send(id.clone()).unwrap();
        app.callback_feed = Some(feed);

        app.drain_callback_deliveries().await;

        assert!(request_rx.try_recv().is_err(), "aucun CallbackResult");
        assert!(app.messages.is_empty(), "aucun message système");
        let ligne = app
            .db
            .get_task(&id)
            .await
            .expect("relire la ligne")
            .expect("la ligne existe");
        assert_eq!(ligne.status, "delivered", "la ligne est bien réclamée");
    }

    /// Terminer la ligne comme le pilote : statut, résultat et `completed_at` en
    /// une écriture. La couture pose le sien par `update_task_status`, qui laisse
    /// `completed_at` nul — or le balayage lit la **ligne**, pas le détail HTTP.
    async fn terminer_la_ligne(app: &App<'static>, id: &str) {
        let fini = app.db.update_task_completed(id, Some("fini")).await;
        assert!(fini.expect("terminer la ligne"), "pending → completed");
    }

    /// Une ligne de callback que le **balayage** verra : le helper ci-dessus la
    /// crée, `terminer_la_ligne` la rend visible. Les deux, parce qu'un statut
    /// posé par `update_task_status` laisse `completed_at` nul et reste
    /// invisible de `get_undelivered_callback_tasks_for_session` — un test monté
    /// sur lui seul serait vert sans rien mesurer (mika#2205).
    async fn une_ligne_balayable(app: &App<'static>) -> String {
        let (id, _tx, _feed) = couture_dun_callback_livrable(app, "pending", None, None).await;
        terminer_la_ligne(app, &id).await;
        id
    }

    /// Un tour de minuteur, et rien d'autre. La session est déjà balayée et
    /// aucun signal n'est armé : une livraison ne peut donc venir que du bloc
    /// `POLL_INTERVAL_TICKS`. Sans ces deux mises au neutre, le drainage
    /// livrerait à sa place et les trois tests du minuteur seraient verts sur
    /// sa raison, pas sur la leur (mika#2205).
    async fn un_tour_de_minuteur(app: &mut App<'static>) {
        app.derniere_session_balayee = Some(app.session_id.clone());
        app.tick_count = POLL_INTERVAL_TICKS - 1;
        app.tick().await;
        assert!(
            app.tick_count.is_multiple_of(POLL_INTERVAL_TICKS),
            "le tick doit tomber sur le minuteur"
        );
    }

    /// Qu'une ligne balayable soit restée intacte : ni réclamée, ni livrée. Le
    /// statut, parce que `mark_task_delivered` écrit `delivered` — un silence du
    /// canal sans cette seconde mesure ne distinguerait pas « pas balayé » de
    /// « balayé, réclamé, puis perdu ».
    async fn la_ligne_est_intacte(app: &App<'static>, id: &str) {
        let ligne = app
            .db
            .get_task(id)
            .await
            .expect("lire la ligne")
            .expect("la ligne existe");
        assert_eq!(ligne.status, "completed", "rien n'a été réclamé");
    }

    /// **Critère 1 du vol V2b-3b** — sous une couture qui suit l'agent courant,
    /// le minuteur ne balaie plus. Le canal de la couture reste vide et son
    /// signal désarmé : ce que le test mesure est le silence du minuteur seul.
    #[tokio::test]
    async fn mika1727_v2b3b_le_minuteur_se_tait_sous_la_couture_de_lagent_courant() {
        let (mut app, _rtx, mut request_rx, _td) = app_with_response_sender().await;
        let id = une_ligne_balayable(&app).await;
        let (_tx, feed) = CallbackFeed::from_channel(&app.agent_name, String::new(), String::new());
        app.callback_feed = Some(feed);

        un_tour_de_minuteur(&mut app).await;

        assert!(
            request_rx.try_recv().is_err(),
            "le minuteur ne balaie plus sous la couture"
        );
        la_ligne_est_intacte(&app, &id).await;
    }

    /// **Critère 2** — sans couture, le même tour livre, comme avant le vol.
    /// C'est le témoin qui sépare « le minuteur s'est tu » de « la ligne
    /// n'était pas balayable » : les deux tests posent la même ligne.
    #[tokio::test]
    async fn mika1727_v2b3b_sans_couture_le_minuteur_balaie_toujours() {
        let (mut app, _rtx, mut request_rx, _td) = app_with_response_sender().await;
        let id = une_ligne_balayable(&app).await;
        // Explicite, bien que ce soit l'état du harnais : c'est l'axe du test.
        app.callback_feed = None;

        un_tour_de_minuteur(&mut app).await;

        let AgentRequest::CallbackResult { task_id, .. } =
            request_rx.try_recv().expect("une livraison")
        else {
            panic!("la livraison doit être un CallbackResult");
        };
        assert_eq!(task_id, id, "la ligne du test, pas une autre");
    }

    /// **Critère 3** — une couture qui suit **un autre** agent ne fait pas taire
    /// le minuteur. C'est le cas du changement d'agent : le fil reste figé sur
    /// l'agent de son ouverture, la TUI en sert un autre, et personne ne
    /// couvrirait le nouveau si le minuteur se taisait aussi (dette D1).
    #[tokio::test]
    async fn mika1727_v2b3b_une_couture_dun_autre_agent_laisse_le_minuteur_balayer() {
        let (mut app, _rtx, mut request_rx, _td) = app_with_response_sender().await;
        let id = une_ligne_balayable(&app).await;
        assert_ne!(app.agent_name, "mika-qa", "l'autre agent doit être autre");
        let (_tx, feed) = CallbackFeed::from_channel("mika-qa", String::new(), String::new());
        app.callback_feed = Some(feed);

        un_tour_de_minuteur(&mut app).await;

        let AgentRequest::CallbackResult { task_id, .. } =
            request_rx.try_recv().expect("une livraison")
        else {
            panic!("la livraison doit être un CallbackResult");
        };
        assert_eq!(task_id, id, "la ligne du test, pas une autre");
    }

    /// **Critère 4 du vol V2b-3b** — un changement de session balaie une fois,
    /// et une seule. Les deux moitiés tiennent dans un test parce que la seconde
    /// ne veut rien dire sans la première.
    ///
    /// Le passage se fait par `tick`, et `tick_count = 1` le laisse **hors**
    /// multiple de `POLL_INTERVAL_TICKS` : c'est la moitié « sans passer par le
    /// minuteur » du critère. Appeler `drain_callback_deliveries` seul la
    /// prouverait par construction plutôt que par mesure, et la remplacer par un
    /// multiple « parce que ça livre quand même » l'effacerait.
    #[tokio::test]
    async fn mika1727_v2b3b_un_changement_de_session_balaie_une_fois() {
        let (mut app, _rtx, mut request_rx, _td) = app_with_response_sender().await;
        une_ligne_balayable(&app).await;
        let (_tx, feed) = CallbackFeed::from_channel(&app.agent_name, String::new(), String::new());
        app.callback_feed = Some(feed);
        app.derniere_session_balayee = Some("s-precedente".to_string());
        app.tick_count = 1;

        app.tick().await;

        assert!(
            request_rx.try_recv().is_ok(),
            "la session neuve est balayée"
        );
        assert!(request_rx.try_recv().is_err(), "une seule livraison");
        assert_eq!(
            app.derniere_session_balayee.as_deref(),
            Some(app.session_id.as_str()),
            "la session balayée est retenue"
        );

        // Remis à la main : sinon la garde du tour en cours expliquerait seule le
        // silence, et le test serait vert sur une autre raison que la sienne
        // (mika#2205).
        app.status = AgentStatus::Idle;
        let deuxieme = une_ligne_balayable(&app).await;

        app.tick().await;

        assert!(
            request_rx.try_recv().is_err(),
            "la même session ne se rebalaie pas"
        );
        let ligne = app
            .db
            .get_task(&deuxieme)
            .await
            .expect("lire la ligne")
            .expect("la ligne existe");
        assert_eq!(ligne.status, "completed", "et rien n'a été réclamé");
    }

    /// **Critère 3** — un callback semé en base, sans **aucune** trame dans le
    /// canal, est livré sur le seul signal. `trace_id` et `parent_task_id` à
    /// `None` sont la signature du balayage, tout en base : le chemin du fil les
    /// lit sur le détail et les aurait à `tr-9` et `t-parent`.
    #[tokio::test]
    async fn mika1727_v2b3a_un_callback_sans_trame_est_livre_sur_le_signal() {
        let (mut app, _rtx, mut request_rx, _td) = app_with_response_sender().await;
        let (id, _tx, feed) = couture_dun_callback_livrable(&app, "pending", None, None).await;
        terminer_la_ligne(&app, &id).await;
        feed.armer_le_rattrapage();
        app.callback_feed = Some(feed);

        app.drain_callback_deliveries().await;
        let AgentRequest::CallbackResult {
            task_id,
            result,
            trace_id,
            original_status,
            parent_task_id,
            ..
        } = request_rx.try_recv().expect("une livraison")
        else {
            panic!("la livraison doit être un CallbackResult");
        };
        assert_eq!(task_id, id);
        assert_eq!(result, "fini");
        assert_eq!(original_status, "completed");
        assert_eq!(trace_id, None, "le balayage lit la ligne, pas le détail");
        assert_eq!(parent_task_id, None, "idem");
        assert_eq!(app.status, AgentStatus::Thinking, "le tour repart");
        let dernier = app.messages.last().expect("un message système");
        assert_eq!(dernier.content, "[long_running:run_claude_pilot] completed");
        assert!(request_rx.try_recv().is_err(), "une seule livraison");
    }

    /// **Critère 4** — deux signaux en file ne livrent pas deux fois. Le second
    /// passage repart d'un `Idle` remis à la main : sinon la garde du tour en
    /// cours suffirait seule à expliquer le silence, et le test serait vert sur
    /// une autre raison que la sienne (mika#2205).
    #[tokio::test]
    async fn mika1727_v2b3a_deux_signaux_ne_livrent_pas_deux_fois_le_meme_callback() {
        let (mut app, _rtx, mut request_rx, _td) = app_with_response_sender().await;
        let (id, _tx, feed) = couture_dun_callback_livrable(&app, "pending", None, None).await;
        terminer_la_ligne(&app, &id).await;
        feed.armer_le_rattrapage();
        feed.armer_le_rattrapage();
        app.callback_feed = Some(feed);

        app.drain_callback_deliveries().await;
        assert!(request_rx.try_recv().is_ok(), "une livraison");
        assert!(request_rx.try_recv().is_err(), "une seule");

        app.status = AgentStatus::Idle;
        let couture = app.callback_feed.as_ref().expect("la couture est là");
        couture.armer_le_rattrapage();
        couture.armer_le_rattrapage();
        app.drain_callback_deliveries().await;
        assert!(request_rx.try_recv().is_err(), "rien au second passage");
    }
}
