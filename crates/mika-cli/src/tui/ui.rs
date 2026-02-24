use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Padding, Paragraph, Wrap};

use crate::tui::app::{AgentStatus, App, ChatRole};
use crate::tui::markdown;

pub fn draw(f: &mut Frame<'_>, app: &mut App<'_>) {
    let chunks = Layout::vertical([
        Constraint::Length(1), // header
        Constraint::Min(5),    // messages
        Constraint::Length(3), // input
        Constraint::Length(1), // footer
    ])
    .split(f.area());

    draw_header(f, app, chunks[0]);
    draw_messages(f, app, chunks[1]);
    draw_input(f, app, chunks[2]);
    draw_footer(f, app, chunks[3]);
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
                let md_lines = markdown::render(&msg.content);
                lines.extend(md_lines);
            }
            ChatRole::System => {
                lines.push(Line::default());
                lines.push(Line::from(vec![Span::styled(
                    msg.content.clone(),
                    Style::default().fg(Color::Red),
                )]));
            }
        }
    }

    // Progressive reveal of pending response
    if let Some(ref full) = app.pending_response {
        lines.push(Line::default());
        let prefix = Line::from(vec![Span::styled(
            format!("{}: ", app.identity_name),
            Style::default().add_modifier(Modifier::BOLD),
        )]);
        lines.push(prefix);

        let revealed = &full[..app.reveal_index.min(full.len())];
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

    // Calculate scroll: show the bottom of messages by default
    let total_lines = lines.len() as u16;
    let visible_height = inner.height;
    let max_scroll = total_lines.saturating_sub(visible_height);
    let effective_scroll = max_scroll.saturating_sub(app.scroll_offset);

    let paragraph = Paragraph::new(lines)
        .scroll((effective_scroll, 0))
        .wrap(Wrap { trim: false });
    f.render_widget(paragraph, inner);
}

fn draw_input(f: &mut Frame<'_>, app: &mut App<'_>, area: Rect) {
    let block = Block::default()
        .borders(Borders::NONE)
        .padding(Padding::horizontal(1));
    let inner = block.inner(area);
    f.render_widget(block, area);

    // Prefix "> " before the textarea
    let chunks = Layout::horizontal([Constraint::Length(2), Constraint::Min(1)]).split(inner);

    let prompt = Paragraph::new(Span::styled(
        "> ",
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    ));
    f.render_widget(prompt, chunks[0]);
    f.render_widget(&app.textarea, chunks[1]);
}

fn draw_footer(f: &mut Frame<'_>, app: &App<'_>, area: Rect) {
    let status_text = match &app.status {
        AgentStatus::Idle => "ready",
        AgentStatus::Thinking => "thinking...",
        AgentStatus::Responding(_) => "responding...",
    };

    let footer = Line::from(vec![
        Span::styled(
            format!(" {} ", app.model),
            Style::default().fg(Color::DarkGray),
        ),
        Span::styled(" | ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            format!("{status_text}"),
            Style::default().fg(Color::DarkGray),
        ),
        Span::styled(" | ", Style::default().fg(Color::DarkGray)),
        Span::styled("Ctrl+C quit", Style::default().fg(Color::DarkGray)),
    ]);
    f.render_widget(Paragraph::new(footer), area);
}
