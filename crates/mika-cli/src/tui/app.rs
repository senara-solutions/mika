use ratatui::text::Line;
use tokio::sync::mpsc;
use tui_textarea::TextArea;

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
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ChatRole {
    User,
    Assistant,
    System,
}

/// Messages flowing between the TUI and the agent worker.
pub enum AgentRequest {
    Message(String),
    Quit,
}

pub struct AgentResponse {
    pub content: String,
    pub is_error: bool,
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
}

impl<'a> App<'a> {
    pub fn new(
        agent_tx: mpsc::UnboundedSender<AgentRequest>,
        agent_rx: mpsc::UnboundedReceiver<AgentResponse>,
        session_id: String,
        model: String,
        identity_name: String,
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
        }
    }

    /// Submit the current input to the agent.
    pub fn send_message(&mut self) {
        let text: String = self.textarea.lines().join("\n");
        let text = text.trim().to_string();
        if text.is_empty() {
            return;
        }

        // Save to history
        self.input_history.push(text.clone());
        self.history_index = None;

        // Add user message to display (no markdown rendering needed for user messages)
        self.messages.push(ChatMessage {
            role: ChatRole::User,
            content: text.clone(),
            rendered: None,
        });

        // Send to agent worker
        let _ = self.agent_tx.send(AgentRequest::Message(text));
        self.status = AgentStatus::Thinking;

        // Clear input
        self.textarea = TextArea::default();
        self.textarea
            .set_cursor_line_style(ratatui::style::Style::default());
        self.textarea.set_placeholder_text("Type a message...");

        // Auto-scroll to bottom
        self.scroll_offset = 0;
        self.needs_redraw = true;
    }

    /// Called on each tick to advance progressive reveal and check for agent responses.
    pub fn tick(&mut self) {
        self.tick_count = self.tick_count.wrapping_add(1);

        // Check for agent response
        match self.agent_rx.try_recv() {
            Ok(response) => {
                if response.is_error {
                    self.messages.push(ChatMessage {
                        role: ChatRole::System,
                        content: format!("Error: {}", response.content),
                        rendered: None,
                    });
                    self.status = AgentStatus::Idle;
                } else {
                    self.pending_response = Some(response.content);
                    self.reveal_index = 0;
                    self.status = AgentStatus::Responding(0);
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
                    });
                    self.status = AgentStatus::Idle;
                    self.needs_redraw = true;
                }
            }
            Err(mpsc::error::TryRecvError::Empty) => {}
        }

        // Advance progressive reveal
        if let Some(ref full) = self.pending_response.clone() {
            let len = full.len();
            if self.reveal_index < len {
                // Reveal in chunks for smooth appearance.
                // Use floor_char_boundary to avoid panicking on multi-byte UTF-8 chars.
                self.reveal_index = full.floor_char_boundary(self.reveal_index + 8).min(len);
                self.status = AgentStatus::Responding(self.reveal_index);
                self.needs_redraw = true;
            } else {
                // Reveal complete — add full message with pre-rendered markdown
                let rendered = markdown::render(full);
                self.messages.push(ChatMessage {
                    role: ChatRole::Assistant,
                    content: full.clone(),
                    rendered: Some(rendered),
                });
                self.pending_response = None;
                self.reveal_index = 0;
                self.status = AgentStatus::Idle;
                // Auto-scroll to bottom
                self.scroll_offset = 0;
                self.needs_redraw = true;
            }
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
                    self.textarea = TextArea::default();
                    self.textarea
                        .set_cursor_line_style(ratatui::style::Style::default());
                    self.textarea.set_placeholder_text("Type a message...");
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
}
