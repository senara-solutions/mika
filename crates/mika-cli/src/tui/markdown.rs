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

        // Note: check longer prefixes first so "### " isn't matched by "## " or "# ".
        if let Some(heading) = raw.strip_prefix("### ") {
            lines.push(Line::from(vec![Span::styled(
                heading.to_string(),
                Style::default().fg(Color::Yellow),
            )]));
        } else if let Some(heading) = raw.strip_prefix("## ") {
            lines.push(Line::from(vec![Span::styled(
                heading.to_string(),
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            )]));
        } else if let Some(heading) = raw.strip_prefix("# ") {
            lines.push(Line::from(vec![Span::styled(
                heading.to_string(),
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
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
/// Processes whichever marker appears first to avoid incorrect nesting
/// (e.g., backtick-wrapped text containing `**` should not be treated as bold).
fn render_inline(text: &str) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    let mut remaining = text;

    while !remaining.is_empty() {
        let bold_pos = remaining.find("**");
        let code_pos = remaining.find('`');

        match (bold_pos, code_pos) {
            (Some(b), Some(c)) if b < c => {
                // Bold marker comes first
                if b > 0 {
                    spans.push(Span::raw(remaining[..b].to_string()));
                }
                let after = &remaining[b + 2..];
                if let Some(end) = after.find("**") {
                    spans.push(Span::styled(
                        after[..end].to_string(),
                        Style::default().add_modifier(Modifier::BOLD),
                    ));
                    remaining = &after[end + 2..];
                } else {
                    // No closing **, treat as literal text
                    spans.push(Span::raw(remaining[b..].to_string()));
                    return spans;
                }
            }
            (_, Some(c)) => {
                // Code marker comes first (or at same position as bold)
                if c > 0 {
                    spans.push(Span::raw(remaining[..c].to_string()));
                }
                let after = &remaining[c + 1..];
                if let Some(end) = after.find('`') {
                    spans.push(Span::styled(
                        after[..end].to_string(),
                        Style::default().fg(Color::Green),
                    ));
                    remaining = &after[end + 1..];
                } else {
                    // No closing backtick, treat as literal text
                    spans.push(Span::raw(remaining[c..].to_string()));
                    return spans;
                }
            }
            (Some(b), None) => {
                // Only bold marker found
                if b > 0 {
                    spans.push(Span::raw(remaining[..b].to_string()));
                }
                let after = &remaining[b + 2..];
                if let Some(end) = after.find("**") {
                    spans.push(Span::styled(
                        after[..end].to_string(),
                        Style::default().add_modifier(Modifier::BOLD),
                    ));
                    remaining = &after[end + 2..];
                } else {
                    spans.push(Span::raw(remaining[b..].to_string()));
                    return spans;
                }
            }
            (None, None) => {
                // No formatting markers, push remaining as plain text
                spans.push(Span::raw(remaining.to_string()));
                return spans;
            }
        }
    }

    spans
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::{Color, Modifier};

    #[test]
    fn test_h1_style_yellow_bold_underlined() {
        let lines = render("# Hello");
        assert_eq!(lines.len(), 1);
        let span = &lines[0].spans[0];
        assert_eq!(span.content, "Hello");
        let style = span.style;
        assert_eq!(style.fg, Some(Color::Yellow));
        assert!(style.add_modifier.contains(Modifier::BOLD));
        assert!(style.add_modifier.contains(Modifier::UNDERLINED));
    }

    #[test]
    fn test_h2_style_yellow_bold() {
        let lines = render("## World");
        assert_eq!(lines.len(), 1);
        let span = &lines[0].spans[0];
        assert_eq!(span.content, "World");
        let style = span.style;
        assert_eq!(style.fg, Some(Color::Yellow));
        assert!(style.add_modifier.contains(Modifier::BOLD));
        assert!(!style.add_modifier.contains(Modifier::UNDERLINED));
    }

    #[test]
    fn test_h3_style_yellow_no_bold() {
        let lines = render("### Details");
        assert_eq!(lines.len(), 1);
        let span = &lines[0].spans[0];
        assert_eq!(span.content, "Details");
        let style = span.style;
        assert_eq!(style.fg, Some(Color::Yellow));
        assert!(!style.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn test_code_before_bold_not_misinterpreted() {
        // Backtick-wrapped text containing ** should not be treated as bold
        let spans = render_inline("`code **not bold**` rest");
        assert_eq!(spans.len(), 2);
        // First span: inline code
        assert_eq!(spans[0].content, "code **not bold**");
        assert_eq!(spans[0].style.fg, Some(Color::Green));
        // Second span: plain text
        assert_eq!(spans[1].content, " rest");
    }

    #[test]
    fn test_bold_before_code() {
        let spans = render_inline("**bold** then `code`");
        assert_eq!(spans.len(), 3);
        assert_eq!(spans[0].content, "bold");
        assert!(spans[0].style.add_modifier.contains(Modifier::BOLD));
        assert_eq!(spans[1].content, " then ");
        assert_eq!(spans[2].content, "code");
        assert_eq!(spans[2].style.fg, Some(Color::Green));
    }

    #[test]
    fn test_plain_text() {
        let spans = render_inline("just plain text");
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].content, "just plain text");
    }
}
