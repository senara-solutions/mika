use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Padding, Paragraph, Wrap};

use crate::tui::app::{AgentStatus, App, ChatRole};
use crate::tui::markdown;

pub fn draw(f: &mut Frame<'_>, app: &mut App<'_>) {
    // Dynamic input height: grow with content, capped at 6 lines.
    // Account for both wrapped lines (long lines) and explicit newlines (pasted text).
    let available_width = f.area().width.saturating_sub(4) as usize; // borders + "> " prompt
    let input_lines = if available_width > 0 {
        let wrapped: usize = app
            .textarea
            .lines()
            .iter()
            .map(|l| (l.len() / available_width) + 1)
            .sum();
        let line_count = app.textarea.lines().len();
        wrapped.max(line_count).clamp(1, 6) as u16
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
    if app.autocomplete.visible && !app.autocomplete.items.is_empty() {
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
                lines.push(Line::from(vec![
                    Span::styled(
                        "You: ",
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::raw(msg.content.clone()),
                ]));
            }
            ChatRole::Assistant => {
                lines.push(Line::default());
                let prefix = Line::from(vec![Span::styled(
                    format!("{}: ", app.identity_name),
                    Style::default().add_modifier(Modifier::BOLD),
                )]);
                lines.push(prefix);
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

    // Calculate scroll: show the bottom of messages by default.
    // Use usize for all arithmetic to avoid u16 truncation on long conversations.
    // Account for line wrapping: Paragraph::scroll() + Wrap operates on *wrapped* visual
    // rows, so we must count how many rows each Line occupies after wrapping at viewport width.
    let viewport_width = inner.width as usize;
    let total_lines: usize = if viewport_width == 0 {
        lines.len()
    } else {
        lines
            .iter()
            .map(|line| {
                let w = line.width();
                if w == 0 {
                    1
                } else {
                    (w.saturating_sub(1) / viewport_width) + 1
                }
            })
            .sum()
    };
    let visible_height = inner.height as usize;
    let max_scroll = total_lines.saturating_sub(visible_height);
    let effective_scroll = max_scroll.saturating_sub(app.scroll_offset);

    // Clamp to u16::MAX at the ratatui call site (Paragraph::scroll takes u16)
    let scroll_u16 = effective_scroll.min(u16::MAX as usize) as u16;

    let paragraph = Paragraph::new(lines)
        .scroll((scroll_u16, 0))
        .wrap(Wrap { trim: false });
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
    f.render_widget(&app.textarea, input_chunks[1]);
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
    let item_count = app.autocomplete.items.len().min(10);
    let popup_height = item_count as u16 + 2; // +2 for border

    let popup_area = Rect {
        x: input_area.x + 2,
        y: input_area.y.saturating_sub(popup_height),
        width: 50.min(input_area.width.saturating_sub(2)),
        height: popup_height,
    };

    // Clear the area behind the popup
    f.render_widget(Clear, popup_area);

    let items: Vec<ListItem<'_>> = app
        .autocomplete
        .items
        .iter()
        .take(10)
        .enumerate()
        .map(|(i, cmd)| {
            let args = cmd.args_hint.unwrap_or("");
            let text = format!("/{} {} — {}", cmd.name, args, cmd.description);
            let style = if i == app.autocomplete.selected {
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::White)
            };
            ListItem::new(Line::from(vec![Span::styled(text, style)]))
        })
        .collect();

    let list = List::new(items).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::DarkGray))
            .title(" Commands "),
    );

    f.render_widget(list, popup_area);
}
