use chrono::{Local, TimeZone};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Span, Text},
};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::app::App;
use crate::live::ActivityState;
use crate::theme::{
    ACCENT_AMBER, ACCENT_BLUE, ACCENT_GREEN, ACCENT_RED, BG_SURFACE, FG_OVERLAY,
};

/// Render a `tui_input` buffer as spans with a block cursor drawn at the
/// input's actual cursor position, so Left/Right/Home/End movement is visible.
///
/// Every text field in the app routes through this: an editable field that
/// always parks a cursor at the end of the line looks like it does not support
/// cursor movement even when the underlying `Input` handles it.
pub fn input_spans(input: &tui_input::Input, text_style: Style) -> Vec<Span<'static>> {
    let value = input.value();
    let cursor = input.visual_cursor();
    let char_count = value.chars().count();
    let before: String = value.chars().take(cursor).collect();

    if cursor >= char_count {
        vec![
            Span::styled(before, text_style),
            Span::styled("\u{2588}", Style::default().fg(ACCENT_BLUE)),
        ]
    } else {
        let on_cursor: String = value.chars().skip(cursor).take(1).collect();
        let after: String = value.chars().skip(cursor + 1).collect();
        vec![
            Span::styled(before, text_style),
            Span::styled(on_cursor, Style::default().bg(ACCENT_BLUE).fg(BG_SURFACE)),
            Span::styled(after, text_style),
        ]
    }
}

/// Like `input_spans`, but shows `placeholder` (dimmed) when the buffer is empty.
pub fn input_spans_with_placeholder(
    input: &tui_input::Input,
    text_style: Style,
    placeholder: &str,
) -> Vec<Span<'static>> {
    if input.value().is_empty() && !placeholder.is_empty() {
        return vec![
            Span::styled(placeholder.to_string(), Style::default().fg(FG_OVERLAY)),
            Span::styled("\u{2588}", Style::default().fg(ACCENT_BLUE)),
        ];
    }
    input_spans(input, text_style)
}

/// Build styled spans showing activity counts: `● <active> | ● <idle> [| ▶ <waiting>]`.
/// Returns an empty vec when all counts are zero.
pub fn activity_count_spans(active: usize, idle: usize, waiting: usize) -> Vec<Span<'static>> {
    if active == 0 && idle == 0 && waiting == 0 {
        return Vec::new();
    }
    let mut spans = vec![
        Span::styled(
            format!(" ● {}", active),
            Style::default().fg(ACCENT_GREEN).add_modifier(Modifier::BOLD),
        ),
        Span::styled(" |", Style::default().fg(FG_OVERLAY)),
        Span::styled(format!(" ● {}", idle), Style::default().fg(ACCENT_AMBER)),
    ];
    if waiting > 0 {
        spans.push(Span::styled(" |", Style::default().fg(FG_OVERLAY)));
        spans.push(Span::styled(
            format!(" ▶ {}", waiting),
            Style::default().fg(ACCENT_RED).add_modifier(Modifier::BOLD),
        ));
    }
    spans
}

/// Returns the dot character and style for a live session indicator based on its activity state.
pub fn live_dot_style(app: &App, live_index: usize) -> (&'static str, Style) {
    let name = &app.live_sessions[live_index].tmux_name;
    let state = app.activity_states.get(name).copied().unwrap_or(ActivityState::Unknown);
    match state {
        ActivityState::Active => {
            ("●", Style::default().fg(ACCENT_GREEN).add_modifier(Modifier::BOLD))
        }
        ActivityState::Idle => {
            ("●", Style::default().fg(ACCENT_AMBER))
        }
        ActivityState::Waiting => {
            ("▶", Style::default().fg(ACCENT_RED).add_modifier(Modifier::BOLD))
        }
        ActivityState::Unknown => {
            ("●", Style::default().fg(ACCENT_GREEN))
        }
    }
}

/// Format a millisecond timestamp as a short human-readable relative date
/// (e.g. `"5m ago"`, `"3h ago"`, `"2d ago"`, or `"Jan 02"` for older dates).
pub fn format_relative_date(timestamp_ms: i64) -> String {
    let dt = match Local.timestamp_millis_opt(timestamp_ms) {
        chrono::LocalResult::Single(dt) => dt,
        _ => return "unknown".to_string(),
    };
    let now = Local::now();
    let diff = now.signed_duration_since(dt);

    if diff.num_hours() < 1 {
        let mins = diff.num_minutes();
        if mins <= 0 {
            "just now".to_string()
        } else {
            format!("{}m ago", mins)
        }
    } else if diff.num_hours() < 24 {
        format!("{}h ago", diff.num_hours())
    } else if diff.num_days() < 7 {
        format!("{}d ago", diff.num_days())
    } else {
        dt.format("%b %d").to_string()
    }
}

/// Estimate the number of terminal rows a `Text` will occupy when word-wrapped to `width` columns.
/// Used to compute the maximum valid scroll offset before rendering.
pub fn estimate_wrapped_height(text: &Text, width: usize) -> usize {
    if width == 0 {
        return text.lines.len();
    }
    text.lines
        .iter()
        .map(|line| {
            let line_width: usize = line.spans.iter().map(|s| s.content.width()).sum();
            if line_width == 0 {
                1
            } else {
                (line_width + width - 1) / width
            }
        })
        .sum()
}

/// Compute a centered `Rect` that is `percent_x`% wide and `percent_y`% tall within `area`.
pub fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(area);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}

/// Truncate `s` to at most `max` display columns, appending `…` if truncated, and right-padding with spaces to exactly `max` columns.
pub fn truncate(s: &str, max: usize) -> String {
    if max == 0 {
        return String::new();
    }
    let width = s.width();
    if width <= max {
        let pad = max - width;
        format!("{}{}", s, " ".repeat(pad))
    } else {
        let mut truncated = String::new();
        let mut w = 0;
        for c in s.chars() {
            let cw = c.width().unwrap_or(0);
            if w + cw > max - 1 {
                break;
            }
            truncated.push(c);
            w += cw;
        }
        let pad = max - w - 1;
        format!("{}…{}", truncated, " ".repeat(pad))
    }
}

/// Like `truncate` but truncates from the left, prepending `…` when the string exceeds `max` columns.
/// Useful for long directory paths where the end (project name) is most relevant.
pub fn truncate_left(s: &str, max: usize) -> String {
    if max == 0 {
        return String::new();
    }
    let width = s.width();
    if width <= max {
        let pad = max - width;
        format!("{}{}", s, " ".repeat(pad))
    } else {
        // Walk from the end to collect characters that fit in (max - 1) columns
        let chars: Vec<char> = s.chars().collect();
        let mut w = 0;
        let mut start = chars.len();
        for i in (0..chars.len()).rev() {
            let cw = chars[i].width().unwrap_or(0);
            if w + cw > max - 1 {
                break;
            }
            w += cw;
            start = i;
        }
        let truncated: String = chars[start..].iter().collect();
        let pad = max - w - 1;
        format!("…{}{}", truncated, " ".repeat(pad))
    }
}

/// Like `truncate_left` but returns the raw string without trailing space padding,
/// used for header labels in the tree view where padding is not needed.
pub fn truncate_left_plain(s: &str, max: usize) -> String {
    if max == 0 {
        return String::new();
    }
    let width = s.width();
    if width <= max {
        s.to_string()
    } else {
        let chars: Vec<char> = s.chars().collect();
        let mut w = 0;
        let mut start = chars.len();
        for i in (0..chars.len()).rev() {
            let cw = chars[i].width().unwrap_or(0);
            if w + cw > max - 1 {
                break;
            }
            w += cw;
            start = i;
        }
        format!("…{}", chars[start..].iter().collect::<String>())
    }
}

#[cfg(test)]
mod input_span_tests {
    use super::{input_spans, input_spans_with_placeholder};
    use ratatui::style::Style;
    use tui_input::Input;

    #[test]
    fn cursor_at_the_end_renders_a_trailing_block() {
        let input = Input::from("abc".to_string());
        let spans = input_spans(&input, Style::default());
        assert_eq!(spans.len(), 2);
        assert_eq!(spans[0].content, "abc");
        assert_eq!(spans[1].content, "\u{2588}");
    }

    #[test]
    fn cursor_in_the_middle_splits_the_value_around_the_highlighted_char() {
        let mut input = Input::from("abc".to_string());
        input = input.with_cursor(1);
        let spans = input_spans(&input, Style::default());
        assert_eq!(spans.len(), 3);
        assert_eq!(spans[0].content, "a");
        assert_eq!(spans[1].content, "b", "the char under the cursor is inverted");
        assert_eq!(spans[2].content, "c");
    }

    #[test]
    fn an_empty_buffer_shows_the_placeholder() {
        let input = Input::default();
        let spans = input_spans_with_placeholder(&input, Style::default(), "auto-name");
        assert_eq!(spans[0].content, "auto-name");
        assert_eq!(spans[1].content, "\u{2588}");

        // A non-empty buffer ignores the placeholder entirely.
        let typed = Input::from("x".to_string());
        let spans = input_spans_with_placeholder(&typed, Style::default(), "auto-name");
        assert_eq!(spans[0].content, "x");
    }
}
