use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

/// Convert markdown text to styled ratatui Lines.
pub fn render(text: &str) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    let mut in_code_block = false;

    for raw in text.lines() {
        if raw.starts_with("```") {
            in_code_block = !in_code_block;
            continue;
        }

        if in_code_block {
            lines.push(Line::from(vec![Span::styled(
                format!("  {raw}"),
                Style::default().fg(Color::Green),
            )]));
            continue;
        }

        if let Some(heading) = raw.strip_prefix("# ") {
            lines.push(Line::from(vec![Span::styled(
                heading.to_string(),
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            )]));
        } else if let Some(heading) = raw.strip_prefix("## ") {
            lines.push(Line::from(vec![Span::styled(
                heading.to_string(),
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            )]));
        } else if let Some(heading) = raw.strip_prefix("### ") {
            lines.push(Line::from(vec![Span::styled(
                heading.to_string(),
                Style::default().add_modifier(Modifier::BOLD),
            )]));
        } else if raw.starts_with("- ") || raw.starts_with("* ") {
            let bullet_content = &raw[2..];
            lines.push(Line::from(render_inline(&format!(
                "  \u{2022} {bullet_content}"
            ))));
        } else if raw.is_empty() {
            lines.push(Line::default());
        } else {
            lines.push(Line::from(render_inline(raw)));
        }
    }

    lines
}

/// Render inline formatting: **bold** and `code`.
fn render_inline(text: &str) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    let mut remaining = text;

    while !remaining.is_empty() {
        // Look for the next formatting marker
        if let Some(pos) = remaining.find("**") {
            if pos > 0 {
                spans.push(Span::raw(remaining[..pos].to_string()));
            }
            let after = &remaining[pos + 2..];
            if let Some(end) = after.find("**") {
                spans.push(Span::styled(
                    after[..end].to_string(),
                    Style::default().add_modifier(Modifier::BOLD),
                ));
                remaining = &after[end + 2..];
            } else {
                spans.push(Span::raw(remaining[pos..].to_string()));
                return spans;
            }
        } else if let Some(pos) = remaining.find('`') {
            if pos > 0 {
                spans.push(Span::raw(remaining[..pos].to_string()));
            }
            let after = &remaining[pos + 1..];
            if let Some(end) = after.find('`') {
                spans.push(Span::styled(
                    after[..end].to_string(),
                    Style::default().fg(Color::Green),
                ));
                remaining = &after[end + 1..];
            } else {
                spans.push(Span::raw(remaining[pos..].to_string()));
                return spans;
            }
        } else {
            spans.push(Span::raw(remaining.to_string()));
            return spans;
        }
    }

    spans
}
