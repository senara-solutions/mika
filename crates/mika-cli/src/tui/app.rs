use std::path::PathBuf;
use std::sync::Arc;

use ratatui::text::Line;
use tokio::sync::mpsc;
use tui_textarea::TextArea;

use mika_agent::async_db::AsyncDatabase;
use mika_agent::skills::SkillRegistry;
use mika_common::claude::ClaudeClient;

use crate::tui::attachment::ImageAttachment;
use crate::tui::commands;
use crate::tui::commands::autocomplete::AutocompleteState;
use crate::tui::markdown;

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

/// Messages flowing between the TUI and the agent worker.
pub enum AgentRequest {
    Message {
        text: String,
        images: Vec<ImageAttachment>,
        thinking_budget: Option<u32>,
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
    pub input_history: Vec<String>,
    pub history_index: Option<usize>,

    // Display info
    pub session_id: String,
    pub model: String,
    pub identity_name: String,

    // Animated thinking dots
    pub tick_count: u64,

    /// Whether the UI needs to be redrawn (set true on state changes).
    pub needs_redraw: bool,

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
}

/// Context window limit for the model (Claude's 200K context).
pub const MODEL_CONTEXT_LIMIT: u32 = 200_000;

/// Channels to poll for cross-channel messages (includes "cli" for messages
/// from other CLI processes like `mika ask`; the TUI's own messages are
/// excluded via the watermark which is bumped after each send).
pub const POLLED_CHANNELS: &[&str] = &["telegram", "cli"];

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
            input_history: Vec::new(),
            history_index: None,
            session_id,
            model,
            identity_name,
            tick_count: 0,
            needs_redraw: true,
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
        if !text.is_empty() {
            self.input_history.push(text.clone());
        }
        self.history_index = None;

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
                self.scroll_offset = 0;
            }
            self.needs_redraw = true;
        }

        // Check for agent response
        match self.agent_rx.try_recv() {
            Ok(response) => {
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
                // Auto-scroll to bottom
                self.scroll_offset = 0;
                self.needs_redraw = true;
                // Update watermark to avoid re-polling our own messages
                if let Ok(max_id) = self.db.max_message_id().await {
                    self.last_seen_msg_id = max_id;
                }
            }
        }

        // Cross-channel polling: check for new messages from other channels every ~5 seconds.
        // Only poll when idle to avoid visual confusion during agent processing.
        if self.tick_count % POLL_INTERVAL_TICKS == 0 && self.status == AgentStatus::Idle {
            self.poll_cross_channel_messages().await;
        }

        // Thinking animation needs redraw every tick while active
        if self.status == AgentStatus::Thinking {
            self.needs_redraw = true;
        }
    }

    pub fn scroll_up(&mut self, amount: usize) {
        self.scroll_offset = self.scroll_offset.saturating_add(amount);
        self.needs_redraw = true;
    }

    pub fn scroll_down(&mut self, amount: usize) {
        self.scroll_offset = self.scroll_offset.saturating_sub(amount);
        self.needs_redraw = true;
    }

    pub fn history_previous(&mut self) {
        if self.input_history.is_empty() {
            return;
        }
        let idx = match self.history_index {
            None => self.input_history.len() - 1,
            Some(i) => {
                if i == 0 {
                    return;
                }
                i - 1
            }
        };
        self.history_index = Some(idx);
        let text = self.input_history[idx].clone();
        self.textarea = TextArea::from(text.lines().map(String::from).collect::<Vec<_>>());
        self.textarea
            .set_cursor_line_style(ratatui::style::Style::default());
    }

    pub fn history_next(&mut self) {
        match self.history_index {
            None => {}
            Some(i) => {
                if i + 1 >= self.input_history.len() {
                    self.history_index = None;
                    self.reset_textarea();
                } else {
                    self.history_index = Some(i + 1);
                    let text = self.input_history[i + 1].clone();
                    self.textarea =
                        TextArea::from(text.lines().map(String::from).collect::<Vec<_>>());
                    self.textarea
                        .set_cursor_line_style(ratatui::style::Style::default());
                }
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
        let channels: Vec<String> = POLLED_CHANNELS.iter().map(|s| s.to_string()).collect();
        let new_msgs = match self
            .db
            .load_messages_after(self.last_seen_msg_id, Some(channels))
            .await
        {
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
                "user" => ChatRole::User,
                "assistant" => ChatRole::Assistant,
                _ => continue,
            };
            // CLI messages from other processes don't need a channel badge
            // (same channel as the TUI), matching the history loader behavior.
            let channel = if msg.channel_type == "cli" {
                None
            } else {
                Some(msg.channel_type.clone())
            };
            self.messages.push(ChatMessage {
                role,
                content: msg.content.clone(),
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

    fn reset_textarea(&mut self) {
        self.textarea = TextArea::default();
        self.textarea
            .set_cursor_line_style(ratatui::style::Style::default());
        self.textarea.set_placeholder_text("Type a message...");
    }
}
