use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use std::path::Path;
use tui_textarea::CursorMove;

use crate::tui::app::{
    AgentStatus, App, ChatMessage, ChatRole, DashboardAction, MessagePosition, SelectionState,
    TextPosition,
};
use crate::tui::attachment::ImageAttachment;
use crate::tui::commands::autocomplete::{
    CompletionContext, CompletionMode, longest_common_prefix,
};
use crate::tui::ui;
use crate::tui::wrap::{WrappedChars, find_char_at_display_col};

/// Check if a screen position falls within the textarea area.
fn is_in_textarea(app: &App<'_>, col: u16, row: u16) -> bool {
    if let Some(rect) = app.textarea_inner_rect {
        col >= rect.x && col < rect.x + rect.width && row >= rect.y && row < rect.y + rect.height
    } else {
        false
    }
}

/// Map a screen position to textarea logical coordinates (row, char_col).
///
/// Returns `None` if the position cannot be mapped. Uses the shared wrapping
/// iterator to find which logical row and character index the screen position
/// corresponds to.
fn screen_to_textarea_pos(
    app: &App<'_>,
    screen_col: u16,
    screen_row: u16,
) -> Option<(usize, usize)> {
    let rect = app.textarea_inner_rect?;
    let rel_col = screen_col.saturating_sub(rect.x) as usize;
    // Account for scroll offset: the visible area starts at display_row = scroll_offset
    let rel_row = screen_row.saturating_sub(rect.y) as usize + app.textarea_scroll_offset as usize;

    let width = rect.width as usize;
    if width == 0 {
        return None;
    }

    let lines = app.textarea.lines();

    // Walk through text lines, wrapping as we go, to find which logical line
    // and display sub-line the target row falls on
    let mut display_row = 0usize;

    for (line_idx, line) in lines.iter().enumerate() {
        if line.is_empty() {
            if display_row == rel_row {
                return Some((line_idx, 0));
            }
            display_row += 1;
            continue;
        }

        let mut seg_start_char = 0;
        let mut seg_start_byte = 0;
        let mut prev_wrap_row = 0;

        for wc in WrappedChars::new(line, width) {
            if wc.display_row != prev_wrap_row {
                // Wrap break: this starts a new display row
                if display_row == rel_row {
                    // Target is in the previous segment
                    return Some((
                        line_idx,
                        find_char_at_display_col(
                            &line[seg_start_byte..wc.byte_offset],
                            seg_start_char,
                            rel_col,
                        ),
                    ));
                }
                display_row += 1;
                seg_start_char = wc.char_idx;
                seg_start_byte = wc.byte_offset;
                prev_wrap_row = wc.display_row;
            }
        }

        // Final segment of this line
        if display_row == rel_row {
            return Some((
                line_idx,
                find_char_at_display_col(&line[seg_start_byte..], seg_start_char, rel_col),
            ));
        }
        display_row += 1;
    }

    // Past the end -- clamp to last position
    if let Some(last_line) = lines.last() {
        let last_idx = lines.len().saturating_sub(1);
        Some((last_idx, last_line.chars().count()))
    } else {
        Some((0, 0))
    }
}

/// Handle a mouse event (scroll, click-drag for text selection).
pub fn handle_mouse(app: &mut App<'_>, mouse: MouseEvent) {
    match mouse.kind {
        MouseEventKind::ScrollUp => {
            app.scroll_up(3);
        }
        MouseEventKind::ScrollDown => {
            app.scroll_down(3);
        }
        MouseEventKind::Down(MouseButton::Left) => {
            // Check footer [start]/[stop] button
            if let Some(rect) = app.footer_start_stop_rect
                && mouse.column >= rect.x
                && mouse.column < rect.x + rect.width
                && mouse.row >= rect.y
                && mouse.row < rect.y + rect.height
            {
                let action = if app.dashboard_running {
                    DashboardAction::Stop
                } else {
                    DashboardAction::Start
                };
                app.pending_dashboard_action = Some(action);
                return;
            }

            // Check footer [open] button
            if let Some(rect) = app.footer_open_rect
                && mouse.column >= rect.x
                && mouse.column < rect.x + rect.width
                && mouse.row >= rect.y
                && mouse.row < rect.y + rect.height
            {
                app.pending_dashboard_action = Some(DashboardAction::Open);
                return;
            }

            // Check textarea area first
            if is_in_textarea(app, mouse.column, mouse.row) {
                if let Some((row, col)) = screen_to_textarea_pos(app, mouse.column, mouse.row) {
                    // Cancel any existing message selection
                    if !app.selection_state.is_none() {
                        app.selection_state.clear();
                    }
                    app.textarea.cancel_selection();
                    app.textarea
                        .move_cursor(CursorMove::Jump(row as u16, col as u16));
                    app.textarea.start_selection();
                    app.textarea_selecting = true;
                    app.needs_redraw = true;
                }
                return;
            }

            // Then check message area
            if let Some((msg_idx, pos)) = ui::hit_test(mouse.column, mouse.row, app) {
                let mp = MessagePosition {
                    message_idx: msg_idx,
                    text_pos: pos,
                };
                app.selection_state = SelectionState::Dragging {
                    anchor: mp,
                    current: mp,
                };
                // Clear any textarea selection
                app.textarea.cancel_selection();
                app.textarea_selecting = false;
                app.needs_redraw = true;
            } else {
                // Clicked outside messages — clear all selections
                if !app.selection_state.is_none() {
                    app.selection_state.clear();
                    app.needs_redraw = true;
                }
                if app.textarea.selection_range().is_some() {
                    app.textarea.cancel_selection();
                    app.textarea_selecting = false;
                    app.needs_redraw = true;
                }
            }
        }
        MouseEventKind::Drag(MouseButton::Left) => {
            // Textarea drag
            if app.textarea_selecting {
                if let Some((row, col)) = screen_to_textarea_pos(app, mouse.column, mouse.row) {
                    app.textarea
                        .move_cursor(CursorMove::Jump(row as u16, col as u16));
                    app.needs_redraw = true;
                }
                return;
            }

            // Message area drag
            if let SelectionState::Dragging { anchor, .. } = app.selection_state
                && let Some((msg_idx, pos)) = ui::hit_test(mouse.column, mouse.row, app)
            {
                app.selection_state = SelectionState::Dragging {
                    anchor,
                    current: MessagePosition {
                        message_idx: msg_idx,
                        text_pos: pos,
                    },
                };
                app.needs_redraw = true;
            }
        }
        MouseEventKind::Up(MouseButton::Left) => {
            // Textarea mouse-up
            if app.textarea_selecting {
                app.textarea_selecting = false;
                // If no actual range selected, cancel
                if let Some(((sr, sc), (er, ec))) = app.textarea.selection_range()
                    && sr == er
                    && sc == ec
                {
                    app.textarea.cancel_selection();
                }
                app.needs_redraw = true;
                return;
            }

            // Message area mouse-up
            if let SelectionState::Dragging { anchor, current } = app.selection_state {
                if anchor == current {
                    app.selection_state.clear();
                } else {
                    let (start, end) = if anchor <= current {
                        (anchor, current)
                    } else {
                        (current, anchor)
                    };
                    app.selection_state = SelectionState::Selected { start, end };
                }
                app.needs_redraw = true;
            }
        }
        _ => {}
    }
}

/// Handle a key event with autocomplete-aware dispatch.
pub fn handle_key(app: &mut App<'_>, key: KeyEvent) {
    // Ctrl+C: copy when selection exists, quit when it doesn't
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
        // Textarea selection takes priority (from Ctrl+A select all)
        if app.textarea.selection_range().is_some() {
            app.textarea.copy();
            let yanked = app.textarea.yank_text();
            if !yanked.is_empty() {
                copy_text_to_clipboard(&yanked);
            }
            app.textarea.cancel_selection();
            app.needs_redraw = true;
            return;
        }

        match &app.selection_state {
            SelectionState::Selected { start, end } => {
                // Extract text and copy to clipboard
                let start = *start;
                let end = *end;
                copy_selection_to_clipboard(app, start, end);
                app.selection_state.clear();
                app.needs_redraw = true;
            }
            SelectionState::Dragging { .. } => {
                // Cancel drag
                app.selection_state.clear();
                app.needs_redraw = true;
            }
            SelectionState::None => {
                let input_text = app.input_text();
                if !input_text.is_empty() {
                    copy_text_to_clipboard(&input_text);
                } else {
                    app.should_quit = true;
                }
            }
        }
        return;
    }

    // Clear selection on any keypress (Ctrl+C already handled above)
    if !app.selection_state.is_none() {
        app.selection_state.clear();
        app.needs_redraw = true;
    }

    // Clear textarea selection on any keypress (except Ctrl+A/Ctrl+C handled above)
    if app.textarea.selection_range().is_some() {
        app.textarea.cancel_selection();
        app.needs_redraw = true;
    }

    // Always-safe quit commands bypass the Idle gate and autocomplete.
    // Critical for autonomous agents that are rarely Idle — without this,
    // `tmux send-keys "/exit" Enter` is silently dropped while busy.
    if key.code == KeyCode::Enter
        && !key
            .modifiers
            .intersects(KeyModifiers::SHIFT | KeyModifiers::ALT)
    {
        let input_text = app.input_text();
        if matches!(input_text.trim(), "/exit" | "/quit" | "/q") {
            app.should_quit = true;
            return;
        }
    }

    if app.autocomplete.visible() {
        handle_key_autocomplete(app, key);
    } else {
        handle_key_normal(app, key);
    }
}

/// Copy text to the system clipboard using subprocess-based approach.
///
/// On macOS, uses `pbcopy`. On Linux, tries `xclip -selection clipboard`
/// then `wl-copy` (Wayland). Falls back to arboard as a last resort.
/// Returns `true` on success.
fn copy_text_to_clipboard(text: &str) -> bool {
    if cfg!(target_os = "macos") {
        return try_subprocess_copy(text, "pbcopy", &[]);
    }

    // Linux: try xclip, then wl-copy, then arboard fallback
    if try_subprocess_copy(text, "xclip", &["-selection", "clipboard"]) {
        return true;
    }
    if try_subprocess_copy(text, "wl-copy", &[]) {
        return true;
    }

    // Last resort (Windows or if subprocesses unavailable)
    arboard::Clipboard::new()
        .and_then(|mut cb| cb.set_text(text))
        .is_ok()
}

/// Pipe text to a clipboard subprocess. Returns `true` on success.
fn try_subprocess_copy(text: &str, cmd: &str, args: &[&str]) -> bool {
    use std::io::Write;
    use std::process::{Command, Stdio};

    let Ok(mut child) = Command::new(cmd)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    else {
        return false;
    };
    let Some(mut stdin) = child.stdin.take() else {
        return false;
    };
    if stdin.write_all(text.as_bytes()).is_err() {
        return false;
    }
    drop(stdin);
    child.wait().map(|s| s.success()).unwrap_or(false)
}

/// Copy the selected text to the system clipboard (supports cross-message selection).
fn copy_selection_to_clipboard(app: &mut App<'_>, start: MessagePosition, end: MessagePosition) {
    let mut parts: Vec<String> = Vec::new();
    for msg_idx in start.message_idx..=end.message_idx {
        let Some(entry) = app
            .messages_layout
            .entries
            .iter()
            .find(|e| e.message_idx == msg_idx)
        else {
            continue;
        };
        let text_start = if msg_idx == start.message_idx {
            start.text_pos
        } else {
            TextPosition {
                line: 0,
                char_offset: 0,
            }
        };
        let text_end = if msg_idx == end.message_idx {
            end.text_pos
        } else {
            TextPosition {
                line: usize::MAX,
                char_offset: usize::MAX,
            }
        };
        let text = ui::extract_selected_text(&entry.lines, text_start, text_end);
        if !text.is_empty() {
            parts.push(text);
        }
    }
    let combined = parts.join("\n");
    if !combined.is_empty() && !copy_text_to_clipboard(&combined) {
        tracing::debug!("clipboard write failed");
    }
}

/// Key handling when the autocomplete popup is visible.
fn handle_key_autocomplete(app: &mut App<'_>, key: KeyEvent) {
    match key.code {
        // Esc dismisses popup (keeps typed text)
        KeyCode::Esc => {
            app.autocomplete.dismiss();
        }

        // Down: next suggestion
        KeyCode::Down => {
            app.autocomplete.next();
        }

        // Up: previous suggestion
        KeyCode::Up => {
            app.autocomplete.previous();
        }

        // Tab: bash-style completion
        KeyCode::Tab => {
            handle_tab_completion(app);
        }

        // Enter: accept selected completion (only plain Enter, not Shift/Alt+Enter)
        KeyCode::Enter
            if !key
                .modifiers
                .intersects(KeyModifiers::SHIFT | KeyModifiers::ALT) =>
        {
            handle_enter_completion(app);
        }

        // Shift+Enter or Alt+Enter: dismiss autocomplete and insert newline
        KeyCode::Enter
            if key
                .modifiers
                .intersects(KeyModifiers::SHIFT | KeyModifiers::ALT) =>
        {
            app.autocomplete.dismiss();
            app.textarea.input(key);
        }

        // Any other key: pass to textarea, then update autocomplete filter
        _ => {
            app.textarea.input(key);
            update_autocomplete_for_input(app);
        }
    }
}

/// Handle Tab key: bash-style longest common prefix completion.
fn handle_tab_completion(app: &mut App<'_>) {
    let values = app.autocomplete.item_values();
    if values.is_empty() {
        return;
    }

    match &app.autocomplete.mode {
        CompletionMode::Command { items, .. } => {
            let lcp = longest_common_prefix(&values);
            let input = app.input_text();
            let current_prefix = &input[1..]; // strip "/"

            if lcp.len() > current_prefix.len() {
                // Partial completion: extend to longest common prefix
                let new_input = format!("/{lcp}");
                set_textarea(app, &new_input);
                app.autocomplete.update_command(&new_input);
            } else if items.len() == 1 {
                // Unique match: complete fully
                let cmd = items[0];
                if cmd.args_hint.is_some() {
                    // Command takes args: complete and add space for argument input
                    let new_input = format!("/{} ", cmd.name);
                    set_textarea(app, &new_input);
                    app.autocomplete.dismiss();
                } else {
                    // No args: complete and execute
                    let new_input = format!("/{}", cmd.name);
                    set_textarea(app, &new_input);
                    app.autocomplete.dismiss();
                    if app.status == AgentStatus::Idle {
                        app.send_message();
                    }
                }
            } else {
                // Multiple matches with no further common prefix: cycle
                app.autocomplete.next();
            }
        }
        CompletionMode::Argument { items, .. } => {
            let lcp = longest_common_prefix(&values);
            let input = app.input_text();

            // Find where the current argument starts
            let arg_start = find_current_arg_start(&input);
            let current_arg = &input[arg_start..];

            if lcp.len() > current_arg.len() {
                // Partial completion
                let new_input = format!("{}{lcp}", &input[..arg_start]);
                set_textarea(app, &new_input);
                // Re-trigger argument completion with new prefix
                trigger_argument_completion(app);
            } else if items.len() == 1 {
                // Unique match: complete fully and add space
                let new_input = format!("{}{} ", &input[..arg_start], items[0].value);
                set_textarea(app, &new_input);
                app.autocomplete.dismiss();
            } else {
                // Multiple matches: cycle
                app.autocomplete.next();
            }
        }
        CompletionMode::Hidden => {}
    }
}

/// Handle Enter key in autocomplete mode.
fn handle_enter_completion(app: &mut App<'_>) {
    match &app.autocomplete.mode {
        CompletionMode::Command { items, selected } => {
            if let Some(cmd) = items.get(*selected) {
                let cmd_name = cmd.name;
                let has_args = cmd.args_hint.is_some();

                if has_args {
                    // Command takes args: accept name, add space, dismiss popup
                    let new_input = format!("/{cmd_name} ");
                    set_textarea(app, &new_input);
                    app.autocomplete.dismiss();
                } else {
                    // No args: accept and execute immediately
                    let cmd_text = format!("/{cmd_name}");
                    set_textarea(app, &cmd_text);
                    app.autocomplete.dismiss();
                    if app.status == AgentStatus::Idle {
                        app.send_message();
                    }
                }
            }
        }
        CompletionMode::Argument {
            items, selected, ..
        } => {
            if let Some(item) = items.get(*selected) {
                let input = app.input_text();
                let arg_start = find_current_arg_start(&input);
                let new_input = format!("{}{} ", &input[..arg_start], item.value);
                set_textarea(app, &new_input);
                app.autocomplete.dismiss();
            }
        }
        CompletionMode::Hidden => {}
    }
}

/// Update autocomplete state based on current input.
fn update_autocomplete_for_input(app: &mut App<'_>) {
    let input = app.input_text();

    if !input.starts_with('/') {
        app.autocomplete.dismiss();
        return;
    }

    // Check if we're in command or argument territory
    if !input[1..].contains(' ') {
        // Still typing the command name
        app.autocomplete.update_command(&input);
    } else {
        // Past the command name — in argument territory
        // Don't auto-show argument completions (lazy/Tab-triggered per design)
        // But if we were in argument mode, dismiss since text changed
        if matches!(app.autocomplete.mode, CompletionMode::Argument { .. }) {
            // Re-trigger argument completion with the new prefix
            trigger_argument_completion(app);
        }
    }
}

/// Key handling when autocomplete is NOT visible (normal mode).
fn handle_key_normal(app: &mut App<'_>, key: KeyEvent) {
    // Ctrl+A: select all input text
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('a') {
        if !app.textarea.is_empty() {
            app.textarea.select_all();
            app.needs_redraw = true;
        }
        return;
    }

    // Ctrl+V: check clipboard for image first, else fall through to normal paste
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('v') {
        match try_clipboard_image() {
            ClipboardResult::Image(attachment) => {
                if app.is_team_mode() {
                    app.messages.push(crate::tui::app::ChatMessage {
                        role: crate::tui::app::ChatRole::System,
                        content: "Image attachments are not supported in team mode.".to_string(),
                        rendered: None,
                        channel: None,
                    });
                } else if let Some(err) = app.attach_image(attachment) {
                    app.messages.push(crate::tui::app::ChatMessage {
                        role: crate::tui::app::ChatRole::System,
                        content: err,
                        rendered: None,
                        channel: None,
                    });
                }
                return;
            }
            ClipboardResult::NoImage => {
                // No image in clipboard — fall through to text paste
            }
            ClipboardResult::Error(msg) => {
                tracing::debug!("clipboard image failed: {msg}");
                app.messages.push(ChatMessage {
                    role: ChatRole::System,
                    content:
                        "Clipboard image not available. Use /attach <path> to attach an image file."
                            .to_string(),
                    rendered: None,
                    channel: None,
                });
                return;
            }
        }
    }

    // Esc: clear attachments first, then input
    if key.code == KeyCode::Esc {
        if app.has_attachments() {
            app.clear_attachments();
            return;
        }
        app.reset_textarea();
        app.history.reset();
        return;
    }

    // PageUp / PageDown scroll messages
    if key.code == KeyCode::PageUp {
        app.scroll_up(5);
        return;
    }
    if key.code == KeyCode::PageDown {
        app.scroll_down(5);
        return;
    }

    // Enter sends message or executes slash command (only when idle and not shift/alt-held)
    // Shift+Enter and Alt+Enter insert a newline instead (Alt+Enter as universal fallback
    // for terminals where Shift+Enter is indistinguishable from Enter)
    if key.code == KeyCode::Enter
        && !key
            .modifiers
            .intersects(KeyModifiers::SHIFT | KeyModifiers::ALT)
    {
        if app.status == AgentStatus::Idle {
            app.send_message();
        }
        return;
    }

    // Tab: if input starts with "/", trigger completion
    if key.code == KeyCode::Tab {
        let input = app.input_text();
        if let Some(after_slash) = input.strip_prefix('/') {
            if after_slash.contains(' ') {
                // In argument territory — trigger argument completion
                trigger_argument_completion(app);
            } else {
                // Command name completion
                app.autocomplete.update_command(&input);
            }
            return;
        }
        // Otherwise let tab fall through to textarea
    }

    // Ctrl+Up/Down: scroll conversation without leaving input field
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Up {
        app.scroll_up(1);
        return;
    }
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Down {
        app.scroll_down(1);
        return;
    }

    // Up/Down: cursor-position-aware history navigation
    // Up triggers history when cursor is on the first row
    if key.code == KeyCode::Up && app.textarea.cursor().0 == 0 {
        app.history_previous();
        return;
    }
    // Down triggers history when cursor is on the last row
    if key.code == KeyCode::Down
        && app.textarea.cursor().0 == app.textarea.lines().len().saturating_sub(1)
    {
        app.history_next();
        return;
    }

    // Pass everything else to textarea
    app.textarea.input(key);

    // After typing, check if we should show autocomplete (e.g., user just typed "/")
    let input = app.input_text();
    if input.starts_with('/') && !input[1..].contains(' ') {
        app.autocomplete.update_command(&input);
    }
}

/// Trigger argument completion for the current input.
fn trigger_argument_completion(app: &mut App<'_>) {
    let input = app.input_text();
    let (cmd_name, args_str) = crate::tui::commands::parse_command(&input);

    let Some(cmd) = crate::tui::commands::find_command(cmd_name) else {
        app.autocomplete.dismiss();
        return;
    };

    let Some(completer) = cmd.completer else {
        app.autocomplete.dismiss();
        return;
    };

    // Determine arg_index and current arg prefix
    let (arg_index, arg_prefix) = parse_arg_position(args_str);

    let cwd = std::env::current_dir().unwrap_or_default();
    let ctx = CompletionContext {
        home_dir: &app.home_dir,
        global_home: &app.global_home,
        skills: &app.skills,
        current_agent: &app.agent_name,
        cwd: &cwd,
        args_str,
        provider: app.provider,
    };

    let (items, title) = completer(arg_prefix, arg_index, &ctx);
    let wide = title == " Files ";
    app.autocomplete.show_arguments(items, title, wide);
}

/// Parse args string to determine which argument index and prefix we're on.
/// Returns (arg_index, current_arg_prefix).
fn parse_arg_position(args: &str) -> (usize, &str) {
    if args.is_empty() {
        return (0, "");
    }

    let parts: Vec<&str> = args.split_whitespace().collect();

    // If the args string ends with whitespace, we're starting a new argument
    if args.ends_with(' ') {
        return (parts.len(), "");
    }

    // Otherwise, the last part is the current prefix being typed
    let index = parts.len().saturating_sub(1);
    let prefix = parts.last().copied().unwrap_or("");
    (index, prefix)
}

/// Find the byte offset where the current argument starts in the input.
fn find_current_arg_start(input: &str) -> usize {
    // Input is like "/cmd arg1 arg2 prefix"
    // We want the start of "prefix"
    if let Some(last_space) = input.rfind(' ') {
        last_space + 1
    } else {
        // No space found — shouldn't happen in argument mode, but handle gracefully
        input.len()
    }
}

/// Set the textarea to new content, preserving style settings.
/// Cursor is placed at the end of the text.
fn set_textarea(app: &mut App<'_>, text: &str) {
    app.textarea = tui_textarea::TextArea::from(vec![text.to_string()]);
    app.textarea
        .set_cursor_line_style(ratatui::style::Style::default());
    app.textarea.move_cursor(tui_textarea::CursorMove::End);
}

/// Maximum paste size (100KB) to prevent UI freezing.
const MAX_PASTE_BYTES: usize = 100 * 1024;

/// Handle a bracketed paste event — insert text at the current cursor position.
pub fn handle_paste(app: &mut App<'_>, text: &str) {
    if text.is_empty() {
        return;
    }

    let text = if text.len() > MAX_PASTE_BYTES {
        app.messages.push(ChatMessage {
            role: ChatRole::System,
            content: format!(
                "Paste truncated to {}KB (was {}KB).",
                MAX_PASTE_BYTES / 1024,
                text.len() / 1024
            ),
            rendered: None,
            channel: None,
        });
        &text[..text.floor_char_boundary(MAX_PASTE_BYTES)]
    } else {
        text
    };

    // Normalize line endings: \r\n → \n, bare \r → \n
    let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
    app.textarea.insert_str(&normalized);

    // Re-evaluate autocomplete after paste
    let input = app.input_text();
    if input.starts_with('/') && !input[1..].contains(' ') {
        app.autocomplete.update_command(&input);
    } else {
        app.autocomplete.dismiss();
    }

    app.needs_redraw = true;
}

/// Maximum pixels allowed for clipboard images (20 megapixels).
const MAX_IMAGE_PIXELS: usize = 20_000_000;
/// Maximum image dimension in either axis.
const MAX_IMAGE_DIMENSION: u32 = 8192;
/// Maximum file size for image loading (10MB).
const MAX_IMAGE_BYTES: usize = 10 * 1024 * 1024;

/// Result of attempting to read an image from the clipboard.
enum ClipboardResult {
    /// Successfully read an image.
    Image(ImageAttachment),
    /// Clipboard accessible but no image content.
    NoImage,
    /// Clipboard not accessible or another error.
    Error(String),
}

/// Try to read an image from the system clipboard.
///
/// Tries arboard first, then falls back to xclip/wl-paste on Linux.
fn try_clipboard_image() -> ClipboardResult {
    // Try arboard first (works on X11, macOS, Windows)
    match try_arboard_image() {
        ClipboardResult::Image(img) => return ClipboardResult::Image(img),
        ClipboardResult::NoImage => return ClipboardResult::NoImage,
        ClipboardResult::Error(e) => {
            tracing::debug!("arboard clipboard failed: {e}, trying xclip/wl-paste fallback");
        }
    }

    // Linux fallback: try xclip, then wl-paste
    #[cfg(target_os = "linux")]
    {
        if let Some(img) = try_xclip_image() {
            return ClipboardResult::Image(img);
        }
        if let Some(img) = try_wl_paste_image() {
            return ClipboardResult::Image(img);
        }
    }

    ClipboardResult::Error("clipboard image not available".to_string())
}

/// Try arboard clipboard image reading.
fn try_arboard_image() -> ClipboardResult {
    let mut clipboard = match arboard::Clipboard::new() {
        Ok(c) => c,
        Err(e) => return ClipboardResult::Error(format!("clipboard init: {e}")),
    };

    let img = match clipboard.get_image() {
        Ok(img) => img,
        Err(arboard::Error::ContentNotAvailable) => return ClipboardResult::NoImage,
        Err(e) => return ClipboardResult::Error(format!("get_image: {e}")),
    };

    match encode_rgba_to_attachment(&img.bytes, img.width, img.height) {
        Some(att) => ClipboardResult::Image(att),
        None => ClipboardResult::Error("image too large or invalid".to_string()),
    }
}

/// Encode raw RGBA pixel data into a PNG-based ImageAttachment.
fn encode_rgba_to_attachment(bytes: &[u8], width: usize, height: usize) -> Option<ImageAttachment> {
    let width = u32::try_from(width).ok()?;
    let height = u32::try_from(height).ok()?;

    if width > MAX_IMAGE_DIMENSION || height > MAX_IMAGE_DIMENSION {
        return None;
    }

    let pixel_count = (width as usize).checked_mul(height as usize)?;
    if pixel_count > MAX_IMAGE_PIXELS {
        return None;
    }

    let estimated_size = bytes.len() / 2;
    let mut png_data = Vec::with_capacity(estimated_size);
    {
        let mut encoder = png::Encoder::new(&mut png_data, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().ok()?;
        writer.write_image_data(bytes).ok()?;
    }

    let size_bytes = png_data.len();
    if size_bytes > MAX_IMAGE_BYTES {
        return None;
    }

    use base64::Engine;
    let base64_data = base64::engine::general_purpose::STANDARD.encode(&png_data);

    Some(ImageAttachment {
        base64_data,
        media_type: "image/png".to_string(),
        size_bytes,
        label: format!(
            "clipboard image ({})",
            ImageAttachment::format_size(size_bytes)
        ),
    })
}

/// Validate PNG bytes and convert to an `ImageAttachment`.
#[cfg(target_os = "linux")]
fn png_bytes_to_attachment(data: Vec<u8>) -> Option<ImageAttachment> {
    if data.len() > MAX_IMAGE_BYTES || data.len() < 4 {
        return None;
    }
    if &data[..4] != b"\x89PNG" {
        return None;
    }
    let size_bytes = data.len();
    use base64::Engine;
    let base64_data = base64::engine::general_purpose::STANDARD.encode(&data);
    Some(ImageAttachment {
        base64_data,
        media_type: "image/png".to_string(),
        size_bytes,
        label: format!(
            "clipboard image ({})",
            ImageAttachment::format_size(size_bytes)
        ),
    })
}

/// Timeout for clipboard subprocess calls (prevents hanging on broken display servers).
#[cfg(target_os = "linux")]
const CLIPBOARD_SUBPROCESS_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(3);

/// Run a clipboard command with a timeout. Returns None on timeout or failure.
#[cfg(target_os = "linux")]
fn run_clipboard_command(program: &str, args: &[&str]) -> Option<Vec<u8>> {
    use std::process::Command;

    let prog = program.to_string();
    let arg_vec: Vec<String> = args.iter().map(|s| s.to_string()).collect();

    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let result = Command::new(&prog).args(&arg_vec).output();
        let _ = tx.send(result);
    });

    match rx.recv_timeout(CLIPBOARD_SUBPROCESS_TIMEOUT) {
        Ok(Ok(output)) if output.status.success() && !output.stdout.is_empty() => {
            Some(output.stdout)
        }
        Ok(_) => None,
        Err(_) => {
            tracing::debug!("{program} clipboard read timed out after 3s");
            None
        }
    }
}

/// Linux fallback: try reading image from clipboard via xclip.
#[cfg(target_os = "linux")]
fn try_xclip_image() -> Option<ImageAttachment> {
    let data = run_clipboard_command(
        "xclip",
        &["-selection", "clipboard", "-t", "image/png", "-o"],
    )?;
    png_bytes_to_attachment(data)
}

/// Linux fallback: try reading image from clipboard via wl-paste (Wayland).
#[cfg(target_os = "linux")]
fn try_wl_paste_image() -> Option<ImageAttachment> {
    let data = run_clipboard_command("wl-paste", &["--type", "image/png"])?;
    png_bytes_to_attachment(data)
}

/// Try to load an image from a file path.
/// Supports png, jpg, gif, webp. Max 10MB.
pub fn try_load_image_file(path: &str) -> Option<ImageAttachment> {
    let path = Path::new(path);

    // Canonicalize to resolve symlinks and prevent path traversal
    let path = path.canonicalize().ok()?;
    if !path.is_file() {
        return None;
    }

    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase())?;
    let (media_type, magic_bytes): (&str, &[u8]) = match ext.as_str() {
        "png" => ("image/png", b"\x89PNG"),
        "jpg" | "jpeg" => ("image/jpeg", b"\xFF\xD8\xFF"),
        "gif" => ("image/gif", b"GIF"),
        "webp" => ("image/webp", b"RIFF"),
        _ => return None,
    };

    // Check file size via metadata BEFORE reading contents
    let metadata = std::fs::metadata(&path).ok()?;
    if metadata.len() > MAX_IMAGE_BYTES as u64 {
        return None;
    }

    let data = std::fs::read(&path).ok()?;

    // Validate magic bytes match claimed extension
    if data.len() < magic_bytes.len() || &data[..magic_bytes.len()] != magic_bytes {
        return None;
    }

    let size_bytes = data.len();
    use base64::Engine;
    let base64_data = base64::engine::general_purpose::STANDARD.encode(&data);

    let filename = path.file_name().and_then(|n| n.to_str()).unwrap_or("image");

    Some(ImageAttachment {
        base64_data,
        media_type: media_type.to_string(),
        size_bytes,
        label: filename.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::Arc;

    use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
    use mika_agent::async_db::AsyncDatabase;
    use mika_agent::db::Database;
    use mika_agent::skills::SkillRegistry;
    use mika_common::llm::{LlmError, LlmProvider, LlmRequest, LlmResponse, ProviderKind};
    use tokio::sync::mpsc;

    use crate::tui::app::AgentRequest;

    /// Minimal no-op LLM provider for input handler tests.
    struct NoopLlmProvider;

    #[async_trait::async_trait]
    impl LlmProvider for NoopLlmProvider {
        async fn send_message(&self, _request: &LlmRequest) -> Result<LlmResponse, LlmError> {
            Err(LlmError::ProviderError("noop provider".to_string()))
        }

        fn model_name(&self) -> &str {
            "noop"
        }

        fn provider_name(&self) -> &str {
            "noop"
        }

        fn max_tokens(&self) -> u32 {
            4096
        }

        fn supports_tool_calling(&self) -> bool {
            false
        }

        async fn check_health(&self) -> Result<(), LlmError> {
            Ok(())
        }
    }

    /// Test helper: construct a minimal App for input handler tests.
    async fn test_app() -> (
        App<'static>,
        mpsc::UnboundedReceiver<AgentRequest>,
        tempfile::TempDir,
    ) {
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

        let (agent_tx, agent_rx) = mpsc::unbounded_channel();
        let (_response_tx, response_rx) = mpsc::unbounded_channel();

        let llm: Arc<dyn LlmProvider> = Arc::new(NoopLlmProvider);
        let skills = Arc::new(SkillRegistry::empty());

        let app = App::new(
            agent_tx,
            response_rx,
            session_id,
            "claude-sonnet-4-6".to_string(),
            "Mika".to_string(),
            async_db,
            llm,
            home_dir,
            skills,
            "mika".to_string(),
            global_home,
            ProviderKind::Anthropic,
        );

        (app, agent_rx, temp_dir)
    }

    /// Helper: create a plain Enter key event.
    fn enter_key() -> KeyEvent {
        KeyEvent {
            code: KeyCode::Enter,
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        }
    }

    #[tokio::test]
    async fn test_exit_while_thinking_sets_should_quit() {
        let (mut app, _rx, _tmp) = test_app().await;
        app.status = AgentStatus::Thinking;
        app.textarea.insert_str("/exit");
        assert!(!app.should_quit);

        handle_key(&mut app, enter_key());

        assert!(app.should_quit);
    }

    #[tokio::test]
    async fn test_quit_while_responding_sets_should_quit() {
        let (mut app, _rx, _tmp) = test_app().await;
        app.status = AgentStatus::Responding(42);
        app.textarea.insert_str("/quit");
        assert!(!app.should_quit);

        handle_key(&mut app, enter_key());

        assert!(app.should_quit);
    }

    #[tokio::test]
    async fn test_q_while_thinking_sets_should_quit() {
        let (mut app, _rx, _tmp) = test_app().await;
        app.status = AgentStatus::Thinking;
        app.textarea.insert_str("/q");
        assert!(!app.should_quit);

        handle_key(&mut app, enter_key());

        assert!(app.should_quit);
    }

    #[tokio::test]
    async fn test_regular_message_while_busy_does_not_send() {
        let (mut app, mut rx, _tmp): (App<'_>, mpsc::UnboundedReceiver<AgentRequest>, _) =
            test_app().await;
        app.status = AgentStatus::Thinking;
        app.textarea.insert_str("hello");

        handle_key(&mut app, enter_key());

        // should_quit must remain false
        assert!(!app.should_quit);
        // No message should have been sent
        assert!(rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn test_exit_while_idle_sets_should_quit() {
        let (mut app, _rx, _tmp) = test_app().await;
        app.status = AgentStatus::Idle;
        app.textarea.insert_str("/exit");
        assert!(!app.should_quit);

        handle_key(&mut app, enter_key());

        assert!(app.should_quit);
    }

    /// mika#1150 cohort: once the supervisor surfaces `WorkerCrashed`, App
    /// must refuse new prompts rather than silently route them through the
    /// dead `agent_tx`. Without this guard every subsequent keystroke
    /// recreates the silent-drop UX that mika#1149 was meant to surface.
    #[tokio::test]
    async fn test_send_after_crash_surfaces_system_line_and_does_not_send() {
        let (mut app, mut rx, _tmp) = test_app().await;
        app.worker_crashed = true;
        app.textarea.insert_str("hello");

        app.send_message();

        // No AgentRequest leaks onto the dead channel.
        assert!(rx.try_recv().is_err(), "must not send while worker_crashed");
        // The last message rendered must explain why the send was refused.
        let last = app
            .messages
            .last()
            .expect("a system line must be rendered after a refused send");
        assert!(
            last.content.contains("worker has crashed"),
            "refusal line missing crash explanation: {:?}",
            last.content
        );
        assert!(
            last.content.contains("/restart"),
            "refusal line must point at the recovery affordance: {:?}",
            last.content
        );
    }

    /// Healthy worker accepts the same send path the guard above rejects.
    /// Pairs with the test above so a regression that flips the guard
    /// polarity is caught in either direction.
    #[tokio::test]
    async fn test_send_while_healthy_dispatches_to_worker() {
        let (mut app, mut rx, _tmp) = test_app().await;
        assert!(!app.worker_crashed);
        app.textarea.insert_str("hello");

        app.send_message();

        let req = rx.try_recv().expect("healthy worker must receive the send");
        match req {
            AgentRequest::Message { text, .. } => assert_eq!(text, "hello"),
            _ => panic!("expected AgentRequest::Message"),
        }
    }

    #[test]
    fn test_try_load_image_file_nonexistent() {
        assert!(try_load_image_file("/tmp/nonexistent-mika-test.png").is_none());
    }

    #[test]
    fn test_try_load_image_file_unsupported_extension() {
        // Create a temp file with unsupported extension
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.txt");
        std::fs::write(&path, "not an image").unwrap();
        assert!(try_load_image_file(path.to_str().unwrap()).is_none());
    }

    #[test]
    fn test_try_load_image_file_valid_png() {
        // File must start with PNG magic bytes to pass validation
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.png");
        std::fs::write(&path, b"\x89PNG\r\n\x1a\n test data").unwrap();
        let result = try_load_image_file(path.to_str().unwrap());
        assert!(result.is_some());
        let attachment = result.unwrap();
        assert_eq!(attachment.media_type, "image/png");
        assert_eq!(attachment.label, "test.png");
        assert!(attachment.size_bytes > 0);
    }

    #[test]
    fn test_try_load_image_file_jpg() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("photo.jpg");
        std::fs::write(&path, b"\xFF\xD8\xFF test jpeg").unwrap();
        let result = try_load_image_file(path.to_str().unwrap());
        assert!(result.is_some());
        assert_eq!(result.unwrap().media_type, "image/jpeg");
    }

    #[test]
    fn test_try_load_image_file_wrong_magic_bytes() {
        // PNG extension but JPEG magic bytes — should be rejected
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("fake.png");
        std::fs::write(&path, b"\xFF\xD8\xFF not a png").unwrap();
        assert!(try_load_image_file(path.to_str().unwrap()).is_none());
    }

    #[test]
    fn test_format_size_kilobytes() {
        assert_eq!(ImageAttachment::format_size(1024), "1KB");
        assert_eq!(ImageAttachment::format_size(245 * 1024), "245KB");
    }

    #[test]
    fn test_format_size_megabytes() {
        assert_eq!(ImageAttachment::format_size(1_048_576), "1.0MB");
        assert_eq!(ImageAttachment::format_size(5 * 1_048_576), "5.0MB");
    }

    #[test]
    fn test_parse_arg_position_empty() {
        let (idx, prefix) = parse_arg_position("");
        assert_eq!(idx, 0);
        assert_eq!(prefix, "");
    }

    #[test]
    fn test_parse_arg_position_first_arg() {
        let (idx, prefix) = parse_arg_position("son");
        assert_eq!(idx, 0);
        assert_eq!(prefix, "son");
    }

    #[test]
    fn test_parse_arg_position_after_first_arg() {
        let (idx, prefix) = parse_arg_position("set ");
        assert_eq!(idx, 1);
        assert_eq!(prefix, "");
    }

    #[test]
    fn test_parse_arg_position_second_arg() {
        let (idx, prefix) = parse_arg_position("set think");
        assert_eq!(idx, 1);
        assert_eq!(prefix, "think");
    }

    #[test]
    fn test_parse_arg_position_after_second_arg() {
        let (idx, prefix) = parse_arg_position("set thinking_level ");
        assert_eq!(idx, 2);
        assert_eq!(prefix, "");
    }

    #[test]
    fn test_find_current_arg_start() {
        assert_eq!(find_current_arg_start("/model sonnet"), 7);
        assert_eq!(find_current_arg_start("/config set key"), 12);
        assert_eq!(find_current_arg_start("/model "), 7);
    }
}
