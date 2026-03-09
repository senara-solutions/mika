use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Position, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Padding, Paragraph, Wrap};
use unicode_width::UnicodeWidthChar;

use crate::tui::app::{
    AgentStatus, App, ChatRole, DashboardAgentStatus, MessageLayout, SelectionState, TextPosition,
};
use crate::tui::markdown;

/// Build a yellow `[channel] ` prefix span for non-CLI messages.
fn channel_prefix_span(channel: &Option<String>) -> Option<Span<'static>> {
    channel
        .as_ref()
        .map(|ch| Span::styled(format!("[{ch}] "), Style::default().fg(Color::Yellow)))
}

/// Count visual rows a line occupies when wrapped at the given width, using character display widths.
pub(crate) fn visual_line_rows(line: &str, width: usize) -> usize {
    if width == 0 {
        return 1;
    }
    if line.is_empty() {
        return 1;
    }
    let mut rows = 1;
    let mut col = 0;
    for ch in line.chars() {
        let ch_w = UnicodeWidthChar::width(ch).unwrap_or(0);
        if col + ch_w > width && col > 0 {
            rows += 1;
            col = ch_w;
        } else {
            col += ch_w;
        }
    }
    rows
}

/// Result of wrapping multi-line input text with cursor tracking.
struct WrappedInput {
    lines: Vec<Line<'static>>,
    cursor_x: u16,
    cursor_y: u16,
}

/// Wrap input text lines at character-width boundaries and track cursor position.
fn wrap_input_with_cursor(
    text_lines: &[impl AsRef<str>],
    cursor_row: usize,
    cursor_col: usize,
    width: usize,
) -> WrappedInput {
    let mut display_lines: Vec<Line<'static>> = Vec::new();
    let mut cursor_x: u16 = 0;
    let mut cursor_y: u16 = 0;
    let mut found_cursor = false;

    for (line_idx, line) in text_lines.iter().enumerate() {
        let line = line.as_ref();
        if line.is_empty() {
            if line_idx == cursor_row && !found_cursor {
                cursor_y = display_lines.len() as u16;
                cursor_x = 0;
                found_cursor = true;
            }
            display_lines.push(Line::from(""));
            continue;
        }

        let mut seg_start = 0;
        let mut col = 0usize;

        for (char_idx, (byte_offset, ch)) in line.char_indices().enumerate() {
            let ch_w = UnicodeWidthChar::width(ch).unwrap_or(0);

            if col + ch_w > width && col > 0 {
                display_lines.push(Line::from(line[seg_start..byte_offset].to_string()));
                seg_start = byte_offset;
                col = 0;
            }

            if line_idx == cursor_row && char_idx == cursor_col && !found_cursor {
                cursor_y = display_lines.len() as u16;
                cursor_x = col as u16;
                found_cursor = true;
            }

            col += ch_w;
        }

        display_lines.push(Line::from(line[seg_start..].to_string()));

        if line_idx == cursor_row && !found_cursor {
            cursor_y = (display_lines.len() - 1) as u16;
            cursor_x = col as u16;
            found_cursor = true;
        }
    }

    WrappedInput {
        lines: display_lines,
        cursor_x,
        cursor_y,
    }
}

pub fn draw(f: &mut Frame<'_>, app: &mut App<'_>) {
    // Dynamic input height: grow with content, capped at 6 lines.
    // Use character display widths (not byte lengths) for accurate wrapping estimation.
    let available_width = f.area().width.saturating_sub(4) as usize; // borders + "> " prompt
    let input_lines = if available_width > 0 {
        let wrapped: usize = app
            .textarea
            .lines()
            .iter()
            .map(|l| visual_line_rows(l, available_width))
            .sum();
        wrapped.clamp(1, 6) as u16
    } else {
        1
    };
    let attachment_lines: u16 = if app.has_attachments() { 1 } else { 0 };
    let input_height = input_lines + 2 + attachment_lines; // +2 for top/bottom padding

    let chunks = Layout::vertical([
        Constraint::Length(1),            // header
        Constraint::Min(5),               // messages
        Constraint::Length(input_height), // input (dynamic)
        Constraint::Length(1),            // footer
    ])
    .split(f.area());

    draw_header(f, app, chunks[0]);

    // Split-pane: show dashboard panel when team dashboard is active and terminal is wide enough
    let show_dashboard = app.team_dashboard.is_some() && chunks[1].width >= 80;
    let messages_area = if show_dashboard {
        let horiz = Layout::horizontal([Constraint::Percentage(70), Constraint::Percentage(30)])
            .split(chunks[1]);
        draw_team_dashboard(f, app, horiz[1]);
        horiz[0]
    } else {
        chunks[1]
    };
    draw_messages(f, app, messages_area);

    draw_input(f, app, chunks[2]);
    draw_footer(f, app, chunks[3]);

    // Autocomplete popup (rendered last to overlay)
    if app.autocomplete.visible() && app.autocomplete.item_count() > 0 {
        draw_autocomplete(f, app, chunks[2]);
    }
}

fn draw_header(f: &mut Frame<'_>, app: &App<'_>, area: Rect) {
    let now = chrono::Utc::now().format("%H:%M UTC");

    let header = if app.is_team_mode() {
        Line::from(vec![
            Span::styled(
                format!(" \u{2726} {} ", app.identity_name),
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("   "),
            Span::styled(format!("{now}"), Style::default().fg(Color::DarkGray)),
        ])
    } else {
        let short_session = &app.session_id[..8.min(app.session_id.len())];
        Line::from(vec![
            Span::styled(
                format!(" \u{2726} {} ", app.identity_name),
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("\u{2014} session {short_session}"),
                Style::default().fg(Color::DarkGray),
            ),
            Span::raw("   "),
            Span::styled(format!("{now}"), Style::default().fg(Color::DarkGray)),
        ])
    };
    f.render_widget(Paragraph::new(header), area);
}

/// Build the rendered lines for a single message (including leading spacer line).
fn build_message_lines(
    msg: &crate::tui::app::ChatMessage,
    identity_name: &str,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    match msg.role {
        ChatRole::User => {
            lines.push(Line::default());
            let mut spans = Vec::new();
            if let Some(span) = channel_prefix_span(&msg.channel) {
                spans.push(span);
            }
            spans.push(Span::styled(
                "You: ",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ));
            spans.push(Span::raw(msg.content.clone()));
            lines.push(Line::from(spans));
        }
        ChatRole::Assistant => {
            lines.push(Line::default());
            let mut prefix_spans = Vec::new();
            if let Some(span) = channel_prefix_span(&msg.channel) {
                prefix_spans.push(span);
            }
            prefix_spans.push(Span::styled(
                format!("{identity_name}: "),
                Style::default().add_modifier(Modifier::BOLD),
            ));
            lines.push(Line::from(prefix_spans));
            if let Some(ref cached) = msg.rendered {
                lines.extend(cached.clone());
            } else {
                let md_lines = markdown::render(&msg.content);
                lines.extend(md_lines);
            }
        }
        ChatRole::System => {
            lines.push(Line::default());
            for line in msg.content.lines() {
                lines.push(Line::from(vec![Span::styled(
                    line.to_string(),
                    Style::default().fg(Color::Red),
                )]));
            }
        }
        ChatRole::Thinking => {
            lines.push(Line::default());
            if let Some(ref cached) = msg.rendered {
                lines.extend(cached.clone());
            } else {
                lines.push(Line::from(vec![Span::styled(
                    "thinking:",
                    Style::default()
                        .fg(Color::DarkGray)
                        .add_modifier(Modifier::ITALIC | Modifier::BOLD),
                )]));
                for line in msg.content.lines() {
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
            }
        }
        ChatRole::Command => {
            lines.push(Line::default());
            for line in msg.content.lines() {
                lines.push(Line::from(vec![Span::styled(
                    line.to_string(),
                    Style::default().fg(Color::DarkGray),
                )]));
            }
        }
    }
    lines
}

/// Recompute the messages layout if stale.
fn ensure_messages_layout(app: &mut App<'_>, width: u16) {
    let is_thinking = app.status == AgentStatus::Thinking;
    let has_pending = app.pending_response.is_some();
    let msg_count = app.messages.len();

    if !app
        .messages_layout
        .is_stale(width, msg_count, has_pending, app.reveal_index, is_thinking)
    {
        return;
    }

    let mut entries = Vec::with_capacity(msg_count + 2);

    // Build entries for each message
    for (idx, msg) in app.messages.iter().enumerate() {
        let lines = build_message_lines(msg, &app.identity_name);
        let paragraph = Paragraph::new(lines.clone()).wrap(Wrap { trim: false });
        let wrapped_line_count = paragraph.line_count(width);
        entries.push(MessageLayout {
            message_idx: idx,
            lines,
            wrapped_line_count,
        });
    }

    // Pending response (streaming)
    if let Some(ref full) = app.pending_response {
        let mut lines = Vec::new();
        lines.push(Line::default());
        lines.push(Line::from(vec![Span::styled(
            format!("{}: ", app.identity_name),
            Style::default().add_modifier(Modifier::BOLD),
        )]));
        let safe_index = full.floor_char_boundary(app.reveal_index.min(full.len()));
        let revealed = &full[..safe_index];
        let md_lines = markdown::render(revealed);
        lines.extend(md_lines);

        let paragraph = Paragraph::new(lines.clone()).wrap(Wrap { trim: false });
        let wrapped_line_count = paragraph.line_count(width);
        entries.push(MessageLayout {
            message_idx: usize::MAX, // sentinel for pending response
            lines,
            wrapped_line_count,
        });
    }

    // Thinking indicator
    if is_thinking {
        let dots = match (app.tick_count / 5) % 4 {
            0 => ".",
            1 => "..",
            2 => "...",
            _ => "",
        };
        let lines = vec![
            Line::default(),
            Line::from(vec![Span::styled(
                format!("  thinking{dots}"),
                Style::default()
                    .fg(Color::DarkGray)
                    .add_modifier(Modifier::ITALIC),
            )]),
        ];
        let paragraph = Paragraph::new(lines.clone()).wrap(Wrap { trim: false });
        let wrapped_line_count = paragraph.line_count(width);
        entries.push(MessageLayout {
            message_idx: usize::MAX - 1, // sentinel for thinking indicator
            lines,
            wrapped_line_count,
        });
    }

    let total_lines = entries.iter().map(|e| e.wrapped_line_count).sum();

    app.messages_layout = crate::tui::app::MessagesLayout {
        entries,
        total_lines,
        computed_at_width: width,
        computed_at_count: msg_count,
        had_pending: has_pending,
        computed_at_reveal: app.reveal_index,
        computed_at_thinking: is_thinking,
    };
}

fn draw_messages(f: &mut Frame<'_>, app: &mut App<'_>, area: Rect) {
    let block = Block::default()
        .borders(Borders::TOP | Borders::BOTTOM)
        .border_style(Style::default().fg(Color::DarkGray))
        .padding(Padding::horizontal(1));
    let inner = block.inner(area);
    f.render_widget(block, area);

    // Store the inner rect for hit-testing in mouse handlers
    app.messages_inner_rect = Some(inner);

    // Recompute layout if stale
    ensure_messages_layout(app, inner.width);

    let total_lines = app.messages_layout.total_lines;
    let visible_height = inner.height as usize;
    let max_scroll = total_lines.saturating_sub(visible_height);
    let effective_scroll = max_scroll.saturating_sub(app.scroll_offset);

    // Determine which messages are visible and render them
    // We walk from the top of the layout, accumulating lines, and render
    // each message that falls within the visible viewport.
    let mut cumulative = 0usize;
    let viewport_start = effective_scroll;
    let viewport_end = effective_scroll + visible_height;

    for entry in &app.messages_layout.entries {
        let entry_start = cumulative;
        let entry_end = cumulative + entry.wrapped_line_count;
        cumulative = entry_end;

        // Skip entries entirely above the viewport
        if entry_end <= viewport_start {
            continue;
        }
        // Stop if entirely below the viewport
        if entry_start >= viewport_end {
            break;
        }

        // This entry is at least partially visible
        let lines_to_render = if entry.message_idx < app.messages.len() {
            // Check if this message has a selection to highlight
            if let Some(highlighted) =
                get_highlighted_lines(entry.message_idx, &entry.lines, &app.selection_state)
            {
                highlighted
            } else {
                entry.lines.clone()
            }
        } else {
            entry.lines.clone()
        };

        let paragraph = Paragraph::new(lines_to_render).wrap(Wrap { trim: false });

        // Calculate where this entry renders within the viewport
        let skip_lines = viewport_start.saturating_sub(entry_start);
        let available_from_top = inner.height as usize;
        let y_offset = entry_start.saturating_sub(viewport_start);

        // The sub-rect for this entry within the inner area
        let entry_rect = Rect {
            x: inner.x,
            y: inner.y + y_offset as u16,
            width: inner.width,
            height: (available_from_top - y_offset).min(entry.wrapped_line_count - skip_lines)
                as u16,
        };

        if entry_rect.height == 0 {
            continue;
        }

        let paragraph = paragraph.scroll((skip_lines as u16, 0));
        f.render_widget(paragraph, entry_rect);
    }
}

/// If the given message index has an active selection, return lines with highlight applied.
fn get_highlighted_lines(
    message_idx: usize,
    lines: &[Line<'static>],
    selection: &SelectionState,
) -> Option<Vec<Line<'static>>> {
    let (sel_msg, start, end) = match selection {
        SelectionState::Dragging {
            message_idx: m,
            anchor,
            current,
        } => {
            if *m != message_idx {
                return None;
            }
            // Normalize direction
            if anchor.is_before_or_equal(current) {
                (*m, *anchor, *current)
            } else {
                (*m, *current, *anchor)
            }
        }
        SelectionState::Selected {
            message_idx: m,
            start,
            end,
        } => {
            if *m != message_idx {
                return None;
            }
            (*m, *start, *end)
        }
        SelectionState::None => return None,
    };
    let _ = sel_msg; // used only for matching

    Some(apply_selection_highlight(lines, start, end))
}

/// Apply reverse-video selection highlight to lines within [start, end].
pub fn apply_selection_highlight(
    lines: &[Line<'static>],
    start: TextPosition,
    end: TextPosition,
) -> Vec<Line<'static>> {
    let highlight_style = Style::default().bg(Color::White).fg(Color::Black);

    lines
        .iter()
        .enumerate()
        .map(|(line_idx, line)| {
            if line_idx < start.line || line_idx > end.line {
                return line.clone();
            }

            let sel_start_col = if line_idx == start.line {
                start.char_offset
            } else {
                0
            };
            let sel_end_col = if line_idx == end.line {
                end.char_offset
            } else {
                usize::MAX
            };

            if sel_start_col == sel_end_col {
                return line.clone();
            }

            // Split spans at selection boundaries and apply highlight
            let mut new_spans: Vec<Span<'static>> = Vec::new();
            let mut col = 0usize;

            for span in &line.spans {
                let span_start = col;
                let span_len: usize = span
                    .content
                    .chars()
                    .map(|c| UnicodeWidthChar::width(c).unwrap_or(0))
                    .sum();
                let span_end = col + span_len;
                col = span_end;

                if span_end <= sel_start_col || span_start >= sel_end_col {
                    // Entirely outside selection
                    new_spans.push(span.clone());
                    continue;
                }

                // This span overlaps the selection. Split it.
                let mut char_col = span_start;
                let mut before = String::new();
                let mut selected = String::new();
                let mut after = String::new();

                for ch in span.content.chars() {
                    let w = UnicodeWidthChar::width(ch).unwrap_or(0);
                    if char_col < sel_start_col {
                        before.push(ch);
                    } else if char_col < sel_end_col {
                        selected.push(ch);
                    } else {
                        after.push(ch);
                    }
                    char_col += w;
                }

                if !before.is_empty() {
                    new_spans.push(Span::styled(before, span.style));
                }
                if !selected.is_empty() {
                    new_spans.push(Span::styled(selected, highlight_style));
                }
                if !after.is_empty() {
                    new_spans.push(Span::styled(after, span.style));
                }
            }

            Line::from(new_spans)
        })
        .collect()
}

/// Map screen (col, row) to a (message_idx, TextPosition) within a visible message.
/// Returns None if the position is outside any message or outside the messages area.
pub fn hit_test(col: u16, row: u16, app: &App<'_>) -> Option<(usize, TextPosition)> {
    let inner = app.messages_inner_rect?;

    // Check if click is within the messages area
    if col < inner.x
        || col >= inner.x + inner.width
        || row < inner.y
        || row >= inner.y + inner.height
    {
        return None;
    }

    let visible_height = inner.height as usize;
    let total_lines = app.messages_layout.total_lines;
    let max_scroll = total_lines.saturating_sub(visible_height);
    let effective_scroll = max_scroll.saturating_sub(app.scroll_offset);

    // Convert screen row to absolute line index within the full content
    let screen_row = (row - inner.y) as usize;
    let abs_line = effective_scroll + screen_row;

    // Find which message entry this line falls into
    let mut cumulative = 0usize;
    for entry in &app.messages_layout.entries {
        let entry_start = cumulative;
        let entry_end = cumulative + entry.wrapped_line_count;
        cumulative = entry_end;

        if abs_line >= entry_start && abs_line < entry_end {
            // Only allow selection on actual messages (not pending/thinking sentinels)
            if entry.message_idx >= app.messages.len() {
                return None;
            }

            // Map abs_line to a line within this entry's rendered lines,
            // accounting for word wrapping.
            let line_within_entry = abs_line - entry_start;
            let local_col = (col - inner.x) as usize;

            // Walk through the entry's lines with wrapping to find the
            // logical line and character offset.
            let text_pos = screen_to_text_position(
                &entry.lines,
                inner.width as usize,
                line_within_entry,
                local_col,
            );

            return Some((entry.message_idx, text_pos));
        }
    }

    None
}

/// Convert a (wrapped_line_index, screen_col) to a TextPosition within rendered lines.
/// This accounts for word wrapping that ratatui applies.
fn screen_to_text_position(
    lines: &[Line<'static>],
    width: usize,
    target_wrapped_line: usize,
    screen_col: usize,
) -> TextPosition {
    let mut wrapped_line = 0usize;

    for (line_idx, line) in lines.iter().enumerate() {
        // Calculate how many wrapped lines this logical line produces
        let line_text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        let wrapped_count = visual_line_rows(&line_text, width);

        if wrapped_line + wrapped_count > target_wrapped_line {
            // The target is within this logical line
            let sub_line = target_wrapped_line - wrapped_line;

            // Walk through characters to find the offset at (sub_line, screen_col)
            let char_offset =
                find_char_offset_in_wrapped_line(&line_text, width, sub_line, screen_col);

            return TextPosition {
                line: line_idx,
                char_offset,
            };
        }

        wrapped_line += wrapped_count;
    }

    // Past the end — clamp to the last position
    let last_line = lines.len().saturating_sub(1);
    let last_width: usize = if let Some(line) = lines.last() {
        line.spans
            .iter()
            .flat_map(|s| s.content.chars())
            .map(|c| UnicodeWidthChar::width(c).unwrap_or(0))
            .sum()
    } else {
        0
    };
    TextPosition {
        line: last_line,
        char_offset: last_width,
    }
}

/// Find the character column offset for a given position within a wrapped line.
fn find_char_offset_in_wrapped_line(
    text: &str,
    width: usize,
    target_sub_line: usize,
    screen_col: usize,
) -> usize {
    if width == 0 || text.is_empty() {
        return 0;
    }

    let mut current_sub_line = 0usize;
    let mut col = 0usize;
    let mut char_offset = 0usize;

    for ch in text.chars() {
        let ch_w = UnicodeWidthChar::width(ch).unwrap_or(0);

        // Check for wrap
        if col + ch_w > width && col > 0 {
            current_sub_line += 1;
            col = 0;
        }

        if current_sub_line == target_sub_line && col >= screen_col {
            return char_offset;
        }
        if current_sub_line > target_sub_line {
            return char_offset;
        }

        col += ch_w;
        char_offset += ch_w;
    }

    // Clamp to end of line
    char_offset
}

/// Extract the plain text within a selection range from rendered lines.
pub fn extract_selected_text(
    lines: &[Line<'static>],
    start: TextPosition,
    end: TextPosition,
) -> String {
    let mut result = String::new();

    for (line_idx, line) in lines.iter().enumerate() {
        if line_idx < start.line || line_idx > end.line {
            continue;
        }

        let sel_start = if line_idx == start.line {
            start.char_offset
        } else {
            0
        };
        let sel_end = if line_idx == end.line {
            end.char_offset
        } else {
            usize::MAX
        };

        if line_idx > start.line {
            result.push('\n');
        }

        let mut col = 0usize;
        for span in &line.spans {
            for ch in span.content.chars() {
                let w = UnicodeWidthChar::width(ch).unwrap_or(0);
                if col >= sel_start && col < sel_end {
                    result.push(ch);
                }
                col += w;
            }
        }
    }

    result
}

fn draw_input(f: &mut Frame<'_>, app: &mut App<'_>, area: Rect) {
    let block = Block::default()
        .borders(Borders::NONE)
        .padding(Padding::horizontal(1));
    let inner = block.inner(area);
    f.render_widget(block, area);

    // Determine the area for the prompt+textarea
    let prompt_area = if app.has_attachments() {
        let chunks = Layout::vertical([Constraint::Length(1), Constraint::Min(1)]).split(inner);

        let labels: Vec<String> = app
            .pending_images
            .iter()
            .map(|img| format!("[{} {}]", img.label, img.size_display()))
            .collect();
        let indicator = Line::from(vec![
            Span::styled("Attached: ", Style::default().fg(Color::Yellow)),
            Span::styled(labels.join(" "), Style::default().fg(Color::Yellow)),
            Span::styled(" (Esc to clear)", Style::default().fg(Color::DarkGray)),
        ]);
        f.render_widget(Paragraph::new(indicator), chunks[0]);
        chunks[1]
    } else {
        inner
    };

    // Render the "> " prompt and textarea
    let input_chunks =
        Layout::horizontal([Constraint::Length(2), Constraint::Min(1)]).split(prompt_area);
    let prompt = Paragraph::new(Span::styled(
        "> ",
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    ));
    f.render_widget(prompt, input_chunks[0]);

    let textarea_area = input_chunks[1];
    let width = textarea_area.width as usize;

    let lines = app.textarea.lines();
    let (cursor_row, cursor_col) = app.textarea.cursor();

    // Show placeholder when input is empty
    if lines.iter().all(|l| l.is_empty()) && !app.history.is_browsing() {
        let placeholder = Paragraph::new(Span::styled(
            "Type a message...",
            Style::default().fg(Color::DarkGray),
        ));
        f.render_widget(placeholder, textarea_area);
        f.set_cursor_position(Position::new(textarea_area.x, textarea_area.y));
        return;
    }

    // Wrap input and track cursor position
    let wrapped = wrap_input_with_cursor(lines, cursor_row, cursor_col, width);

    // Scroll the input view if cursor is beyond the visible area
    let visible_height = textarea_area.height;
    let scroll_offset = if wrapped.cursor_y >= visible_height {
        wrapped.cursor_y - visible_height + 1
    } else {
        0
    };

    // Render display lines (with scroll offset)
    let display_lines: Vec<Line<'static>> = wrapped
        .lines
        .into_iter()
        .skip(scroll_offset as usize)
        .collect();
    let paragraph = Paragraph::new(display_lines);
    f.render_widget(paragraph, textarea_area);

    // Set cursor position
    let cx = textarea_area.x + wrapped.cursor_x.min(textarea_area.width.saturating_sub(1));
    let cy = textarea_area.y + wrapped.cursor_y - scroll_offset;
    if cy < textarea_area.y + textarea_area.height {
        f.set_cursor_position(Position::new(cx, cy));
    }
}

fn draw_footer(f: &mut Frame<'_>, app: &App<'_>, area: Rect) {
    let status_text = match &app.status {
        AgentStatus::Idle => "ready",
        AgentStatus::Thinking => {
            if app.is_team_mode() {
                "running..."
            } else {
                "thinking..."
            }
        }
        AgentStatus::Responding(_) => "responding...",
    };

    let mut spans = if app.is_team_mode() {
        // Team mode: show dashboard summary when active
        let team_name = app.team_name.as_deref().unwrap_or("team");
        let mut s = vec![
            Span::styled(
                format!(" team: {team_name} "),
                Style::default().fg(Color::Yellow),
            ),
            Span::styled(" | ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                status_text.to_string(),
                Style::default().fg(Color::DarkGray),
            ),
        ];
        if let Some(ref dash) = app.team_dashboard {
            let elapsed = dash.run_started.elapsed().as_secs();
            let total = dash.agents.len();
            let done = dash
                .agents
                .iter()
                .filter(|a| a.status == DashboardAgentStatus::Completed)
                .count();
            if total > 0 {
                s.push(Span::styled(" | ", Style::default().fg(Color::DarkGray)));
                s.push(Span::styled(
                    format!("agents: {done}/{total}"),
                    Style::default().fg(Color::White),
                ));
            }
            s.push(Span::styled(" | ", Style::default().fg(Color::DarkGray)));
            s.push(Span::styled(
                format!("{elapsed}s"),
                Style::default().fg(Color::DarkGray),
            ));
            if let Some(ref phase) = dash.phase {
                s.push(Span::styled(" | ", Style::default().fg(Color::DarkGray)));
                s.push(Span::styled(
                    format!("{phase}"),
                    Style::default().fg(Color::Cyan),
                ));
            }
        }
        s
    } else {
        let mut s = vec![
            Span::styled(
                format!(" {} ", app.agent_name),
                Style::default().fg(Color::DarkGray),
            ),
            Span::styled(" | ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                format!("{} ", app.model),
                Style::default().fg(Color::DarkGray),
            ),
            Span::styled(" | ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                status_text.to_string(),
                Style::default().fg(Color::DarkGray),
            ),
        ];

        // Thinking level indicator (always shown in agent mode)
        s.push(Span::styled(" | ", Style::default().fg(Color::DarkGray)));
        match app.thinking_level {
            Some((_, level)) => {
                s.push(Span::styled(
                    format!("think: {level}"),
                    Style::default().fg(Color::Magenta),
                ));
            }
            None => {
                s.push(Span::styled(
                    "think: off",
                    Style::default().fg(Color::DarkGray),
                ));
            }
        }

        // Context usage indicator
        if let Some(tokens) = app.context_tokens {
            let limit = crate::tui::app::MODEL_CONTEXT_LIMIT;
            let pct = (tokens as f64 / limit as f64 * 100.0) as u32;
            let tokens_k = tokens / 1000;
            let limit_k = limit / 1000;
            let color = if pct > 80 {
                Color::Red
            } else if pct > 50 {
                Color::Yellow
            } else {
                Color::DarkGray
            };
            s.push(Span::styled(" | ", Style::default().fg(Color::DarkGray)));
            s.push(Span::styled(
                format!("ctx: {tokens_k}k/{limit_k}k ({pct}%)"),
                Style::default().fg(color),
            ));
        }

        // Pending task badge
        if app.pending_task_count > 0 {
            s.push(Span::styled(" | ", Style::default().fg(Color::DarkGray)));
            s.push(Span::styled(
                format!("[{} tasks]", app.pending_task_count),
                Style::default().fg(Color::Cyan),
            ));
        }
        s
    };

    // Scroll / new-message indicator
    if app.scroll_offset > 0 {
        spans.push(Span::styled(" | ", Style::default().fg(Color::DarkGray)));
        if app.has_new_message {
            spans.push(Span::styled(
                "\u{2193} new messages",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ));
        } else {
            spans.push(Span::styled(
                "\u{2191} scrolled",
                Style::default().fg(Color::DarkGray),
            ));
        }
    }

    spans.push(Span::styled(" | ", Style::default().fg(Color::DarkGray)));
    spans.push(Span::styled(
        "/ commands",
        Style::default().fg(Color::DarkGray),
    ));
    spans.push(Span::styled(" | ", Style::default().fg(Color::DarkGray)));
    if !app.selection_state.is_none() {
        spans.push(Span::styled(
            "Ctrl+C copy",
            Style::default().fg(Color::Cyan),
        ));
    } else {
        spans.push(Span::styled(
            "Ctrl+C quit",
            Style::default().fg(Color::DarkGray),
        ));
    }

    let footer = Line::from(spans);
    f.render_widget(Paragraph::new(footer), area);
}

fn draw_team_dashboard(f: &mut Frame<'_>, app: &App<'_>, area: Rect) {
    let dash = match &app.team_dashboard {
        Some(d) => d,
        None => return,
    };

    let block = Block::default()
        .borders(Borders::LEFT | Borders::TOP | Borders::BOTTOM)
        .border_style(Style::default().fg(Color::DarkGray))
        .title(Span::styled(
            " Dashboard ",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ))
        .padding(Padding::horizontal(1));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let mut lines: Vec<Line<'static>> = Vec::new();

    // Phase + iteration
    let phase_str = dash
        .phase
        .as_ref()
        .map(|p| p.to_string())
        .unwrap_or_else(|| "starting".to_string());
    lines.push(Line::from(vec![
        Span::styled("Phase: ", Style::default().fg(Color::DarkGray)),
        Span::styled(phase_str, Style::default().fg(Color::Cyan)),
    ]));
    if dash.iteration > 0 {
        lines.push(Line::from(vec![
            Span::styled("Iter:  ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                format!("{}", dash.iteration),
                Style::default().fg(Color::White),
            ),
        ]));
    }
    lines.push(Line::from(""));

    // Agent status grid
    if !dash.agents.is_empty() {
        lines.push(Line::from(Span::styled(
            "Agents:",
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        )));
        for agent in &dash.agents {
            let (icon, color) = match agent.status {
                DashboardAgentStatus::Running => ("\u{2026}", Color::Yellow), // …
                DashboardAgentStatus::Completed => ("\u{2713}", Color::Green), // ✓
                DashboardAgentStatus::Failed => ("\u{2717}", Color::Red),     // ✗
            };
            lines.push(Line::from(vec![
                Span::styled(format!(" {icon} "), Style::default().fg(color)),
                Span::styled(agent.name.clone(), Style::default().fg(color)),
                Span::styled(
                    format!(" ({})", agent.role),
                    Style::default().fg(Color::DarkGray),
                ),
            ]));
        }
        lines.push(Line::from(""));
    }

    // Elapsed time
    let elapsed = dash.run_started.elapsed();
    let secs = elapsed.as_secs();
    let elapsed_str = if secs >= 60 {
        format!("{}m {}s", secs / 60, secs % 60)
    } else {
        format!("{secs}s")
    };
    lines.push(Line::from(vec![
        Span::styled("Elapsed: ", Style::default().fg(Color::DarkGray)),
        Span::styled(elapsed_str, Style::default().fg(Color::White)),
    ]));

    // Agent completion summary
    let total = dash.agents.len();
    let done = dash
        .agents
        .iter()
        .filter(|a| a.status == DashboardAgentStatus::Completed)
        .count();
    if total > 0 {
        lines.push(Line::from(vec![
            Span::styled("Done:    ", Style::default().fg(Color::DarkGray)),
            Span::styled(format!("{done}/{total}"), Style::default().fg(Color::White)),
        ]));
    }

    let paragraph = Paragraph::new(lines).wrap(Wrap { trim: false });
    f.render_widget(paragraph, inner);
}

fn draw_autocomplete(f: &mut Frame<'_>, app: &App<'_>, input_area: Rect) {
    use crate::tui::commands::autocomplete::CompletionMode;

    let item_count = app.autocomplete.item_count().min(10);
    let popup_height = item_count as u16 + 2; // +2 for border
    let selected = app.autocomplete.selected_index();
    let title = app.autocomplete.title();

    // Adapt width based on completion mode
    let max_width = match &app.autocomplete.mode {
        CompletionMode::Argument { wide: true, .. } => {
            80u16.min(input_area.width.saturating_sub(2))
        }
        _ => 55u16.min(input_area.width.saturating_sub(2)),
    };

    let popup_area = Rect {
        x: input_area.x + 2,
        y: input_area.y.saturating_sub(popup_height),
        width: max_width,
        height: popup_height,
    };

    // Clear the area behind the popup
    f.render_widget(Clear, popup_area);

    let items: Vec<ListItem<'_>> = match &app.autocomplete.mode {
        CompletionMode::Command { items, .. } => items
            .iter()
            .take(10)
            .enumerate()
            .map(|(i, cmd)| {
                let args = cmd.args_hint.unwrap_or("");
                let text = format!("/{} {} \u{2014} {}", cmd.name, args, cmd.description);
                let style = if i == selected {
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(Color::White)
                };
                ListItem::new(Line::from(vec![Span::styled(text, style)]))
            })
            .collect(),
        CompletionMode::Argument { items, .. } => items
            .iter()
            .take(10)
            .enumerate()
            .map(|(i, item)| {
                let text = if let Some(ref desc) = item.description {
                    format!("{} \u{2014} {desc}", item.value)
                } else {
                    item.value.clone()
                };
                let style = if i == selected {
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(Color::White)
                };
                ListItem::new(Line::from(vec![Span::styled(text, style)]))
            })
            .collect(),
        CompletionMode::Hidden => vec![],
    };

    // Add scroll indicator if more items than visible
    let total = app.autocomplete.item_count();
    let display_title = if total > 10 {
        format!("{title}({total}) ")
    } else {
        title.to_string()
    };

    let list = List::new(items).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::DarkGray))
            .title(display_title),
    );

    f.render_widget(list, popup_area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::app::TextPosition;

    #[test]
    fn test_visual_line_rows_empty() {
        assert_eq!(visual_line_rows("", 80), 1);
    }

    #[test]
    fn test_visual_line_rows_short() {
        assert_eq!(visual_line_rows("hello", 80), 1);
    }

    #[test]
    fn test_visual_line_rows_exact_width() {
        assert_eq!(visual_line_rows("abcde", 5), 1);
    }

    #[test]
    fn test_visual_line_rows_wraps() {
        assert_eq!(visual_line_rows("abcdef", 5), 2);
    }

    #[test]
    fn test_visual_line_rows_zero_width() {
        assert_eq!(visual_line_rows("abc", 0), 1);
    }

    #[test]
    fn test_extract_selected_text_single_span() {
        let lines = vec![Line::from(vec![Span::raw("Hello, world!")])];
        let text = extract_selected_text(
            &lines,
            TextPosition {
                line: 0,
                char_offset: 0,
            },
            TextPosition {
                line: 0,
                char_offset: 5,
            },
        );
        assert_eq!(text, "Hello");
    }

    #[test]
    fn test_extract_selected_text_multi_span() {
        let lines = vec![Line::from(vec![
            Span::styled("bold", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(" normal"),
        ])];
        let text = extract_selected_text(
            &lines,
            TextPosition {
                line: 0,
                char_offset: 2,
            },
            TextPosition {
                line: 0,
                char_offset: 7,
            },
        );
        assert_eq!(text, "ld no");
    }

    #[test]
    fn test_extract_selected_text_multi_line() {
        let lines = vec![
            Line::from(vec![Span::raw("first line")]),
            Line::from(vec![Span::raw("second line")]),
            Line::from(vec![Span::raw("third line")]),
        ];
        let text = extract_selected_text(
            &lines,
            TextPosition {
                line: 0,
                char_offset: 6,
            },
            TextPosition {
                line: 2,
                char_offset: 5,
            },
        );
        assert_eq!(text, "line\nsecond line\nthird");
    }

    #[test]
    fn test_apply_selection_highlight_partial_span() {
        let lines = vec![Line::from(vec![Span::raw("Hello, world!")])];
        let result = apply_selection_highlight(
            &lines,
            TextPosition {
                line: 0,
                char_offset: 7,
            },
            TextPosition {
                line: 0,
                char_offset: 12,
            },
        );
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].spans.len(), 3); // before + highlighted + after
        assert_eq!(result[0].spans[0].content, "Hello, ");
        assert_eq!(result[0].spans[1].content, "world");
        assert_eq!(result[0].spans[1].style.bg, Some(Color::White));
        assert_eq!(result[0].spans[2].content, "!");
    }

    #[test]
    fn test_apply_selection_highlight_no_selection_on_line() {
        let lines = vec![
            Line::from(vec![Span::raw("not selected")]),
            Line::from(vec![Span::raw("selected text")]),
        ];
        let result = apply_selection_highlight(
            &lines,
            TextPosition {
                line: 1,
                char_offset: 0,
            },
            TextPosition {
                line: 1,
                char_offset: 8,
            },
        );
        // First line should be unchanged
        assert_eq!(result[0].spans.len(), 1);
        assert_eq!(result[0].spans[0].content, "not selected");
        // Second line should have selection
        assert_eq!(result[1].spans[0].content, "selected");
        assert_eq!(result[1].spans[0].style.bg, Some(Color::White));
    }

    #[test]
    fn test_find_char_offset_in_wrapped_line_first_sub_line() {
        // "abcdefghij" at width 5 wraps to:
        // sub-line 0: "abcde"
        // sub-line 1: "fghij"
        let offset = find_char_offset_in_wrapped_line("abcdefghij", 5, 0, 3);
        assert_eq!(offset, 3);
    }

    #[test]
    fn test_find_char_offset_in_wrapped_line_second_sub_line() {
        let offset = find_char_offset_in_wrapped_line("abcdefghij", 5, 1, 2);
        assert_eq!(offset, 7); // 5 (first sub-line) + 2
    }

    #[test]
    fn test_find_char_offset_empty() {
        let offset = find_char_offset_in_wrapped_line("", 5, 0, 0);
        assert_eq!(offset, 0);
    }

    #[test]
    fn test_screen_to_text_position_single_line() {
        let lines = vec![Line::from(vec![Span::raw("Hello")])];
        let pos = screen_to_text_position(&lines, 80, 0, 3);
        assert_eq!(pos.line, 0);
        assert_eq!(pos.char_offset, 3);
    }

    #[test]
    fn test_screen_to_text_position_second_line() {
        let lines = vec![
            Line::from(vec![Span::raw("first")]),
            Line::from(vec![Span::raw("second")]),
        ];
        let pos = screen_to_text_position(&lines, 80, 1, 2);
        assert_eq!(pos.line, 1);
        assert_eq!(pos.char_offset, 2);
    }

    #[test]
    fn test_text_position_ordering() {
        let a = TextPosition {
            line: 0,
            char_offset: 5,
        };
        let b = TextPosition {
            line: 1,
            char_offset: 0,
        };
        assert!(a.is_before_or_equal(&b));
        assert!(!b.is_before_or_equal(&a));

        let c = TextPosition {
            line: 0,
            char_offset: 5,
        };
        assert!(a.is_before_or_equal(&c));
    }

    #[test]
    fn test_selection_state_default_is_none() {
        let state = SelectionState::default();
        assert!(state.is_none());
    }
}
