use crate::app::App;
use crate::data::PreviewMessage;
use chrono::{Local, TimeZone};
use ratatui::{
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap},
    Frame,
};

pub fn draw(frame: &mut Frame, app: &mut App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(3), Constraint::Length(1)])
        .split(frame.area());

    let main_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
        .split(chunks[0]);

    // Session list
    let items: Vec<ListItem> = app
        .sessions
        .iter()
        .map(|s| {
            let date = format_relative_date(s.last_timestamp);
            let line = Line::from(vec![
                Span::styled(
                    truncate(&s.project_name, 28),
                    Style::default().fg(Color::White),
                ),
                Span::raw("  "),
                Span::styled(date, Style::default().fg(Color::DarkGray)),
            ]);
            ListItem::new(line)
        })
        .collect();

    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Sessions "),
        )
        .highlight_style(
            Style::default()
                .bg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("▶ ");

    let mut state = ListState::default();
    state.select(Some(app.selected));
    frame.render_stateful_widget(list, main_chunks[0], &mut state);

    // Preview
    let preview = app.current_preview().to_vec();
    let preview_text = build_preview_text(&preview);

    let preview_widget = Paragraph::new(preview_text)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Preview "),
        )
        .wrap(Wrap { trim: false })
        .scroll((app.preview_scroll, 0));

    frame.render_widget(preview_widget, main_chunks[1]);

    // Help bar
    let help = Paragraph::new(Line::from(vec![
        Span::styled(" ↑↓/jk", Style::default().fg(Color::Yellow)),
        Span::raw(" navigate  "),
        Span::styled("Enter", Style::default().fg(Color::Yellow)),
        Span::raw(" open in claude  "),
        Span::styled("J/K", Style::default().fg(Color::Yellow)),
        Span::raw(" scroll preview  "),
        Span::styled("q", Style::default().fg(Color::Yellow)),
        Span::raw(" quit"),
    ]));

    frame.render_widget(help, chunks[1]);
}

fn build_preview_text(messages: &[PreviewMessage]) -> Text<'static> {
    let mut lines: Vec<Line<'static>> = Vec::new();

    for msg in messages {
        let (label, color) = match msg.role.as_str() {
            "user" => ("USER", Color::Cyan),
            "assistant" => ("ASSISTANT", Color::Green),
            _ => ("SYSTEM", Color::Yellow),
        };

        lines.push(Line::from(Span::styled(
            format!("{}:", label),
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        )));

        // Truncate very long messages
        let text = if msg.text.len() > 2000 {
            format!("{}...", &msg.text[..2000])
        } else {
            msg.text.clone()
        };

        for line in text.lines() {
            lines.push(Line::from(Span::raw(line.to_string())));
        }

        lines.push(Line::from(""));
    }

    Text::from(lines)
}

fn format_relative_date(timestamp_ms: i64) -> String {
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

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        format!("{:<width$}", s, width = max)
    } else {
        format!("{}…", &s[..max - 1])
    }
}
