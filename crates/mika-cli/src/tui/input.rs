use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::tui::app::{AgentStatus, App};

/// Handle a key event with autocomplete-aware dispatch.
pub fn handle_key(app: &mut App<'_>, key: KeyEvent) {
    // Ctrl+C always quits
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
        app.should_quit = true;
        return;
    }

    if app.autocomplete.visible {
        handle_key_autocomplete(app, key);
    } else {
        handle_key_normal(app, key);
    }
}

/// Key handling when the autocomplete popup is visible.
fn handle_key_autocomplete(app: &mut App<'_>, key: KeyEvent) {
    match key.code {
        // Esc dismisses popup (keeps typed text)
        KeyCode::Esc => {
            app.autocomplete.dismiss();
        }

        // Tab or Down: next suggestion
        KeyCode::Tab | KeyCode::Down => {
            app.autocomplete.next();
        }

        // Up: previous suggestion
        KeyCode::Up => {
            app.autocomplete.previous();
        }

        // Enter: accept selected completion and execute
        KeyCode::Enter if !key.modifiers.contains(KeyModifiers::SHIFT) => {
            if let Some(name) = app.autocomplete.selected_name() {
                // Set textarea to the full command and execute
                let cmd = format!("/{name}");
                app.textarea = tui_textarea::TextArea::from(vec![cmd.clone()]);
                app.textarea
                    .set_cursor_line_style(ratatui::style::Style::default());
                app.autocomplete.dismiss();
                if app.status == AgentStatus::Idle {
                    app.send_message();
                }
            }
        }

        // Any other key: pass to textarea, then update autocomplete filter
        _ => {
            app.textarea.input(key);
            let input = app.input_text();
            app.autocomplete.update(&input);
        }
    }
}

/// Key handling when autocomplete is NOT visible (normal mode).
fn handle_key_normal(app: &mut App<'_>, key: KeyEvent) {
    // Esc clears input
    if key.code == KeyCode::Esc {
        app.textarea = tui_textarea::TextArea::default();
        app.textarea
            .set_cursor_line_style(ratatui::style::Style::default());
        app.textarea.set_placeholder_text("Type a message...");
        app.history_index = None;
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

    // Enter sends message or executes slash command (only when idle and not shift-held)
    if key.code == KeyCode::Enter && !key.modifiers.contains(KeyModifiers::SHIFT) {
        if app.status == AgentStatus::Idle {
            app.send_message();
        }
        return;
    }

    // Tab: if input starts with "/", open autocomplete
    if key.code == KeyCode::Tab {
        let input = app.input_text();
        if input.starts_with('/') {
            app.autocomplete.update(&input);
            return;
        }
        // Otherwise let tab fall through to textarea
    }

    // Up/Down for history when input is empty
    let input_empty = app.textarea.lines().iter().all(|l| l.trim().is_empty());
    if input_empty && key.code == KeyCode::Up {
        app.history_previous();
        return;
    }
    if input_empty && key.code == KeyCode::Down {
        app.history_next();
        return;
    }

    // Pass everything else to textarea
    app.textarea.input(key);

    // After typing, check if we should show autocomplete (e.g., user just typed "/")
    let input = app.input_text();
    if input.starts_with('/') && !input[1..].contains(' ') {
        app.autocomplete.update(&input);
    }
}
