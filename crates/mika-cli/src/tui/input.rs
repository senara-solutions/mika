use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::tui::app::{AgentStatus, App};

/// Handle a key event, returning true if the event was consumed.
pub fn handle_key(app: &mut App<'_>, key: KeyEvent) -> bool {
    // Ctrl+C always quits
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
        app.should_quit = true;
        return true;
    }

    // Esc clears input
    if key.code == KeyCode::Esc {
        app.textarea = tui_textarea::TextArea::default();
        app.textarea
            .set_cursor_line_style(ratatui::style::Style::default());
        app.textarea.set_placeholder_text("Type a message...");
        app.history_index = None;
        return true;
    }

    // PageUp / PageDown scroll messages
    if key.code == KeyCode::PageUp {
        app.scroll_up(5);
        return true;
    }
    if key.code == KeyCode::PageDown {
        app.scroll_down(5);
        return true;
    }

    // Enter sends message (only when idle and not shift-held)
    if key.code == KeyCode::Enter && !key.modifiers.contains(KeyModifiers::SHIFT) {
        if app.status == AgentStatus::Idle {
            app.send_message();
        }
        return true;
    }

    // Up/Down for history when input is empty
    let input_empty = app.textarea.lines().iter().all(|l| l.trim().is_empty());
    if input_empty && key.code == KeyCode::Up {
        app.history_previous();
        return true;
    }
    if input_empty && key.code == KeyCode::Down {
        app.history_next();
        return true;
    }

    // Pass everything else to textarea
    app.textarea.input(key);
    true
}
