use crate::app::{App, TreeRow};
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
        .constraints([Constraint::Percentage(30), Constraint::Percentage(70)])
        .split(chunks[0]);

    // Session list (filtered or tree)
    let items: Vec<ListItem> = if app.tree_view {
        app.tree_rows
            .iter()
            .map(|row| match row {
                TreeRow::Header {
                    project_name,
                    session_count,
                    project,
                } => {
                    let arrow = if app.collapsed.contains(project) {
                        "▸"
                    } else {
                        "▾"
                    };
                    let line = Line::from(vec![Span::styled(
                        format!("{} {} ({})", arrow, project_name, session_count),
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD),
                    )]);
                    ListItem::new(line)
                }
                TreeRow::Session { session_index } => {
                    let s = &app.sessions[*session_index];
                    let date = format_relative_date(s.last_timestamp);
                    let line = Line::from(vec![
                        Span::raw("  "),
                        Span::styled(date, Style::default().fg(Color::DarkGray)),
                        Span::raw(format!("  {} msg", s.entry_count)),
                    ]);
                    ListItem::new(line)
                }
            })
            .collect()
    } else {
        app.filtered_indices
            .iter()
            .map(|&i| &app.sessions[i])
            .map(|s| {
                let date = format_relative_date(s.last_timestamp);
                let line = Line::from(vec![
                    Span::styled(
                        truncate(&s.project_name, 28),
                        Style::default().fg(Color::White),
                    ),
                    Span::raw("  "),
                    Span::styled(date, Style::default().fg(Color::DarkGray)),
                    Span::styled(
                        format!("  {} msg", s.entry_count),
                        Style::default().fg(Color::DarkGray),
                    ),
                ]);
                ListItem::new(line)
            })
            .collect()
    };

    let view_label = if app.tree_view { "[tree]" } else { "[flat]" };
    let title = match &app.filter_path {
        Some(p) => format!(" Sessions {} ({}) ", view_label, p),
        None => format!(" Sessions {} ", view_label),
    };

    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(title),
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

    // Calculate content height to resolve scroll-to-bottom
    let preview_area = main_chunks[1];
    let inner_width = preview_area.width.saturating_sub(2) as usize; // borders
    let inner_height = preview_area.height.saturating_sub(2); // borders
    let content_height = estimate_wrapped_height(&preview_text, inner_width);
    let max_scroll = (content_height as u16).saturating_sub(inner_height);
    if app.preview_scroll > max_scroll {
        app.preview_scroll = max_scroll;
    }

    let preview_widget = Paragraph::new(preview_text)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Preview "),
        )
        .wrap(Wrap { trim: false })
        .scroll((app.preview_scroll, 0));

    frame.render_widget(preview_widget, preview_area);

    // Help / search bar
    let bottom_bar = if app.filter_active {
        Paragraph::new(Line::from(vec![
            Span::styled(" /", Style::default().fg(Color::Yellow)),
            Span::raw(&app.filter_text),
            Span::styled("█", Style::default().fg(Color::Yellow)),
        ]))
    } else if !app.filter_text.is_empty() {
        Paragraph::new(Line::from(vec![
            Span::styled(" filter: ", Style::default().fg(Color::DarkGray)),
            Span::raw(&app.filter_text),
            Span::raw("  "),
            Span::styled("/", Style::default().fg(Color::Yellow)),
            Span::raw(" edit  "),
            Span::styled("Esc", Style::default().fg(Color::Yellow)),
            Span::raw(" clear"),
        ]))
    } else {
        Paragraph::new(Line::from(vec![
            Span::styled(" ↑↓/jk", Style::default().fg(Color::Yellow)),
            Span::raw(" navigate  "),
            Span::styled("Enter", Style::default().fg(Color::Yellow)),
            Span::raw(" open  "),
            Span::styled("J/K", Style::default().fg(Color::Yellow)),
            Span::raw(" scroll  "),
            Span::styled("/", Style::default().fg(Color::Yellow)),
            Span::raw(" search  "),
            Span::styled("Tab", Style::default().fg(Color::Yellow)),
            Span::raw(" tree  "),
            Span::styled("q", Style::default().fg(Color::Yellow)),
            Span::raw(" quit"),
        ]))
    };

    frame.render_widget(bottom_bar, chunks[1]);
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

        // Truncate very long messages (char-aware to avoid UTF-8 panics)
        let text = if msg.text.chars().count() > 2000 {
            let truncated: String = msg.text.chars().take(2000).collect();
            format!("{}...", truncated)
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

fn estimate_wrapped_height(text: &Text, width: usize) -> usize {
    if width == 0 {
        return text.lines.len();
    }
    text.lines
        .iter()
        .map(|line| {
            let line_width: usize = line.spans.iter().map(|s| s.content.len()).sum();
            if line_width == 0 {
                1
            } else {
                (line_width + width - 1) / width
            }
        })
        .sum()
}

fn truncate(s: &str, max: usize) -> String {
    let char_count = s.chars().count();
    if char_count <= max {
        format!("{:<width$}", s, width = max)
    } else {
        let truncated: String = s.chars().take(max - 1).collect();
        format!("{}…", truncated)
    }
}
