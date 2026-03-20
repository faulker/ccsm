use ratatui::{
    style::{Modifier, Style},
    text::{Line, Span, Text},
};

use crate::data::PreviewMessage;
use crate::theme::{ACCENT_GREEN, ACCENT_MAUVE, ACCENT_PEACH, FG_OVERLAY, FG_TEXT};

use super::ansi::parse_ansi_line;

/// Convert a slice of conversation messages into a styled ratatui `Text` ready for the preview pane.
/// Each message is preceded by a role label; messages are separated by a horizontal rule.
pub fn build_preview_text(messages: &[PreviewMessage]) -> Text<'static> {
    let mut lines: Vec<Line<'static>> = Vec::new();

    for (i, msg) in messages.iter().enumerate() {
        if i > 0 {
            // Separator between messages
            lines.push(Line::from(Span::styled(
                "───────────────────────────────────────",
                Style::default().fg(FG_OVERLAY),
            )));
        }

        let (label, color) = match msg.role.as_str() {
            "user" => ("USER", ACCENT_MAUVE),
            "assistant" => ("ASSISTANT", ACCENT_GREEN),
            _ => ("SYSTEM", ACCENT_PEACH),
        };

        lines.push(Line::from(vec![
            Span::styled(
                "▎ ",
                Style::default().fg(color),
            ),
            Span::styled(
                format!("{}:", label),
                Style::default().fg(color).add_modifier(Modifier::BOLD),
            ),
        ]));

        // Truncate very long messages (char-aware to avoid UTF-8 panics)
        let text = if msg.text.chars().count() > 2000 {
            let truncated: String = msg.text.chars().take(2000).collect();
            format!("{}...", truncated)
        } else {
            msg.text.clone()
        };

        for line in text.lines() {
            lines.push(Line::from(Span::styled(
                line.to_string(),
                Style::default().fg(FG_TEXT),
            )));
        }

        lines.push(Line::from(""));
    }

    Text::from(lines)
}

/// Convert raw tmux pane output (with ANSI escape sequences) into a styled ratatui `Text`.
pub fn build_live_preview_text(raw: &str) -> Text<'static> {
    let mut lines: Vec<Line<'static>> = Vec::new();
    for raw_line in raw.lines() {
        lines.push(parse_ansi_line(raw_line));
    }
    Text::from(lines)
}
