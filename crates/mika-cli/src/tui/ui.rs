use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Position, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Padding, Paragraph, Wrap};
use unicode_width::UnicodeWidthChar;

use crate::tui::app::{AgentStatus, App, ChatRole};
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
    draw_messages(f, app, chunks[1]);
    draw_input(f, app, chunks[2]);
    draw_footer(f, app, chunks[3]);

    // Autocomplete popup (rendered last to overlay)
    if app.autocomplete.visible() && app.autocomplete.item_count() > 0 {
        draw_autocomplete(f, app, chunks[2]);
    }
}

fn draw_header(f: &mut Frame<'_>, app: &App<'_>, area: Rect) {
    let now = chrono::Utc::now().format("%H:%M UTC");
    let short_session = &app.session_id[..8.min(app.session_id.len())];
    let header = Line::from(vec![
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
    ]);
    f.render_widget(Paragraph::new(header), area);
}

fn draw_messages(f: &mut Frame<'_>, app: &App<'_>, area: Rect) {
    let block = Block::default()
        .borders(Borders::TOP | Borders::BOTTOM)
        .border_style(Style::default().fg(Color::DarkGray))
        .padding(Padding::horizontal(1));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let mut lines: Vec<Line<'static>> = Vec::new();

    for msg in &app.messages {
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
                    format!("{}: ", app.identity_name),
                    Style::default().add_modifier(Modifier::BOLD),
                ));
                lines.push(Line::from(prefix_spans));
                // Use cached rendered lines if available, otherwise render now
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
                // Use cached rendered lines if available
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
                // Render each line of command output in cyan
                for line in msg.content.lines() {
                    lines.push(Line::from(vec![Span::styled(
                        line.to_string(),
                        Style::default().fg(Color::DarkGray),
                    )]));
                }
            }
        }
    }

    // Progressive reveal of pending response (re-rendered each frame since content changes)
    if let Some(ref full) = app.pending_response {
        lines.push(Line::default());
        let prefix = Line::from(vec![Span::styled(
            format!("{}: ", app.identity_name),
            Style::default().add_modifier(Modifier::BOLD),
        )]);
        lines.push(prefix);

        // Use floor_char_boundary to ensure we slice on a valid UTF-8 char boundary
        let safe_index = full.floor_char_boundary(app.reveal_index.min(full.len()));
        let revealed = &full[..safe_index];
        let md_lines = markdown::render(revealed);
        lines.extend(md_lines);
    }

    // Thinking indicator
    if app.status == AgentStatus::Thinking {
        let dots = match (app.tick_count / 5) % 4 {
            0 => ".",
            1 => "..",
            2 => "...",
            _ => "",
        };
        lines.push(Line::default());
        lines.push(Line::from(vec![Span::styled(
            format!("  thinking{dots}"),
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::ITALIC),
        )]));
    }

    // Build paragraph with wrapping first, then use ratatui's accurate line counting
    // to calculate scroll. This avoids the discrepancy between our manual character-count
    // estimation and ratatui's word-boundary wrapping (WordWrapper), which can produce
    // more visual rows when words straddle line boundaries.
    let paragraph = Paragraph::new(lines).wrap(Wrap { trim: false });
    let total_lines = paragraph.line_count(inner.width);
    let visible_height = inner.height as usize;
    let max_scroll = total_lines.saturating_sub(visible_height);
    let effective_scroll = max_scroll.saturating_sub(app.scroll_offset);

    let scroll_u16 = effective_scroll.min(u16::MAX as usize) as u16;
    let paragraph = paragraph.scroll((scroll_u16, 0));
    f.render_widget(paragraph, inner);
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
        AgentStatus::Thinking => "thinking...",
        AgentStatus::Responding(_) => "responding...",
    };

    let mut spans = vec![
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

    // Thinking level indicator (always shown)
    spans.push(Span::styled(" | ", Style::default().fg(Color::DarkGray)));
    match app.thinking_level {
        Some((_, level)) => {
            spans.push(Span::styled(
                format!("think: {level}"),
                Style::default().fg(Color::Magenta),
            ));
        }
        None => {
            spans.push(Span::styled(
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
        spans.push(Span::styled(" | ", Style::default().fg(Color::DarkGray)));
        spans.push(Span::styled(
            format!("ctx: {tokens_k}k/{limit_k}k ({pct}%)"),
            Style::default().fg(color),
        ));
    }

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
    spans.push(Span::styled(
        "Ctrl+C quit",
        Style::default().fg(Color::DarkGray),
    ));

    let footer = Line::from(spans);
    f.render_widget(Paragraph::new(footer), area);
}

fn draw_autocomplete(f: &mut Frame<'_>, app: &App<'_>, input_area: Rect) {
    use crate::tui::commands::autocomplete::CompletionMode;

    let item_count = app.autocomplete.item_count().min(10);
    let popup_height = item_count as u16 + 2; // +2 for border
    let selected = app.autocomplete.selected_index();
    let title = app.autocomplete.title();

    // Adapt width based on completion mode
    let max_width = match &app.autocomplete.mode {
        CompletionMode::Argument { title: t, .. } if *t == " Files " => {
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
            .map(|(i, (cmd, _item))| {
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
