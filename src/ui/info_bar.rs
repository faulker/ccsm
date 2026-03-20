use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};
use unicode_width::UnicodeWidthStr;

use crate::app::App;
use crate::update::UpdateStatus;
use crate::theme::{
    ACCENT_BLUE, ACCENT_GREEN, ACCENT_PEACH, FG_OVERLAY, FG_SUBTEXT, FG_TEXT, HIGHLIGHT_BG,
};

use super::util::activity_count_spans;

/// Render the session list title spans (shown in the list block border).
pub fn build_title_spans(app: &App) -> Vec<Span<'static>> {
    let title_style = Style::default().fg(ACCENT_BLUE).add_modifier(Modifier::BOLD);

    let mut title_spans = vec![Span::styled(" Sessions ", title_style)];
    if !app.hide_empty {
        title_spans.push(Span::styled(" [showing empty]", title_style));
    }
    if !app.group_chains {
        title_spans.push(Span::styled(" [ungrouped]", title_style));
    }
    if let Some(p) = &app.filter_path {
        title_spans.push(Span::styled(format!(" ({})", p), title_style));
    }
    let (active, idle, waiting) = app.total_activity_counts();
    title_spans.extend(activity_count_spans(active, idle, waiting));
    if app.live_filter {
        title_spans.push(Span::styled(" [live only]", Style::default().fg(ACCENT_GREEN)));
    }
    title_spans.push(Span::styled(" ", title_style));
    title_spans
}

/// Render the bottom status/help bar.
pub fn render_status_bar(frame: &mut Frame, app: &App, bar_area: Rect) {
    let bar_style = Style::default().bg(HIGHLIGHT_BG);
    let version_label = format!("v{} ", env!("CARGO_PKG_VERSION"));
    let version_width = version_label.len() as u16;
    let bar_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Min(0),
            Constraint::Length(version_width),
        ])
        .split(bar_area);

    if app.filter_active {
        let filter_val = app.filter_input.value();
        let cursor = app.filter_input.visual_cursor();
        let char_count = filter_val.chars().count();
        let before: String = filter_val.chars().take(cursor).collect();
        let mut cursor_spans = vec![
            Span::styled(" /", Style::default().fg(ACCENT_PEACH).add_modifier(Modifier::BOLD)),
            Span::styled(before, Style::default().fg(FG_TEXT)),
        ];
        if cursor >= char_count {
            cursor_spans.push(Span::styled("█", Style::default().fg(ACCENT_BLUE)));
        } else {
            let on_cursor: String = filter_val.chars().nth(cursor).unwrap().to_string();
            let after: String = filter_val.chars().skip(cursor + 1).collect();
            cursor_spans.push(Span::styled(on_cursor, Style::default().bg(ACCENT_BLUE).fg(crate::theme::BG_SURFACE)));
            cursor_spans.push(Span::styled(after, Style::default().fg(FG_TEXT)));
        }
        frame.render_widget(
            Paragraph::new(Line::from(cursor_spans)).style(bar_style),
            bar_chunks[0],
        );
    } else if !app.filter_input.value().is_empty() {
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(" filter: ", Style::default().fg(FG_SUBTEXT)),
                Span::styled(app.filter_input.value(), Style::default().fg(FG_TEXT)),
                Span::raw("  "),
                Span::styled(
                    "/",
                    Style::default()
                        .fg(ACCENT_PEACH)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(" edit  ", Style::default().fg(FG_SUBTEXT)),
                Span::styled(
                    "Esc",
                    Style::default()
                        .fg(ACCENT_PEACH)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(" clear", Style::default().fg(FG_SUBTEXT)),
            ]))
            .style(bar_style),
            bar_chunks[0],
        );
    } else {
        let key_style = Style::default()
            .fg(ACCENT_PEACH)
            .add_modifier(Modifier::BOLD);
        let hint_style = Style::default().fg(FG_SUBTEXT);
        let shift_key_style = if app.shift_active {
            Style::default().fg(Color::Rgb(255, 210, 170)).add_modifier(Modifier::BOLD)
        } else {
            key_style
        };
        let shift_hint_style = if app.shift_active {
            Style::default().fg(Color::Rgb(190, 195, 220)).add_modifier(Modifier::BOLD)
        } else {
            hint_style
        };

        fn hint_width(line: &Line) -> u16 {
            line.spans.iter().map(|s| s.content.width() as u16).sum()
        }

        let mut hints: Vec<Line> = Vec::new();

        // Show post-update status in help bar
        match &app.update_status {
            UpdateStatus::Downloading => {
                hints.push(Line::from(Span::styled(
                    " Updating... ",
                    Style::default().fg(ACCENT_PEACH).add_modifier(Modifier::BOLD),
                )));
            }
            UpdateStatus::Failed(msg) => {
                hints.push(Line::from(Span::styled(
                    format!(" Update failed: {} ", msg),
                    Style::default()
                        .fg(Color::Rgb(243, 139, 168))
                        .add_modifier(Modifier::BOLD),
                )));
            }
            _ => {}
        }

        if let Some(err) = &app.status_error {
            hints.push(Line::from(Span::styled(
                format!(" {err} "),
                Style::default()
                    .fg(Color::Rgb(243, 139, 168))
                    .add_modifier(Modifier::BOLD),
            )));
        }

        let is_live = app.selected_live_index().is_some();

        hints.push(Line::from(vec![
            Span::styled(
                if app.shift_active { " ↑↓/JK" } else { " ↑↓/jk" },
                if app.shift_active { shift_key_style } else { key_style },
            ),
            Span::styled(
                if app.shift_active { " scroll" } else { " navigate" },
                if app.shift_active { shift_hint_style } else { hint_style },
            ),
        ]));
        let enter_shift = app.shift_active && app.is_historical_selected();
        hints.push(Line::from(vec![
            Span::styled(
                "Enter",
                if enter_shift { shift_key_style } else { key_style },
            ),
            Span::styled(
                if enter_shift { " open direct" } else { " open" },
                if enter_shift { shift_hint_style } else { hint_style },
            ),
        ]));
        hints.push(Line::from(vec![
            Span::styled("/", key_style),
            Span::styled(" search", hint_style),
        ]));
        hints.push(Line::from(vec![
            Span::styled("o", key_style),
            Span::styled(" config", hint_style),
        ]));
        hints.push(Line::from(vec![
            Span::styled("r", key_style),
            Span::styled(" rename", hint_style),
        ]));
        hints.push(Line::from(vec![
            Span::styled(
                if app.shift_active { "N" } else { "n" },
                if app.shift_active { shift_key_style } else { key_style },
            ),
            Span::styled(
                if app.shift_active { " new direct" } else { " new live" },
                if app.shift_active { shift_hint_style } else { hint_style },
            ),
        ]));
        hints.push(Line::from(vec![
            Span::styled("l", shift_key_style),
            Span::styled(" live filter", shift_hint_style),
        ]));
        hints.push(Line::from(vec![
            Span::styled("f", key_style),
            Span::styled(" favorite", hint_style),
        ]));
        if is_live {
            hints.push(Line::from(vec![
                Span::styled("x", key_style),
                Span::styled(" stop session", hint_style),
            ]));
        }
        hints.push(Line::from(vec![
            Span::styled("q", key_style),
            Span::styled(" quit", hint_style),
        ]));
        hints.push(Line::from(vec![
            Span::styled("?", shift_key_style),
            Span::styled(" help", shift_hint_style),
        ]));

        // Calculate dynamic spacing between hints
        let hint_widths: Vec<u16> = hints.iter().map(|h| hint_width(h)).collect();
        let total_hint_width: u16 = hint_widths.iter().sum();
        let available = bar_chunks[0].width;
        let num_gaps = hints.len().saturating_sub(1) as u16;
        let gap_size = if num_gaps > 0 && available > total_hint_width {
            ((available - total_hint_width) / num_gaps).min(6)
        } else {
            1
        };

        // Build constraints: hint, gap, hint, gap, ..., hint, Fill(1)
        let mut constraints: Vec<Constraint> = Vec::new();
        for (i, w) in hint_widths.iter().enumerate() {
            if i > 0 {
                constraints.push(Constraint::Length(gap_size));
            }
            constraints.push(Constraint::Length(*w));
        }
        constraints.push(Constraint::Fill(1));

        let hint_chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints(constraints)
            .split(bar_chunks[0]);

        for (i, hint) in hints.iter().enumerate() {
            let chunk_idx = i * 2; // each hint is at even indices (0, 2, 4, ...)
            frame.render_widget(
                Paragraph::new(hint.clone()).style(bar_style),
                hint_chunks[chunk_idx],
            );
            // Gap chunks (odd indices) get bar background
            if i > 0 {
                let gap_idx = chunk_idx - 1;
                frame.render_widget(
                    Paragraph::new("").style(bar_style),
                    hint_chunks[gap_idx],
                );
            }
        }
        // Fill remaining space with bar background
        let last_idx = hints.len() * 2 - 1;
        frame.render_widget(
            Paragraph::new("").style(bar_style),
            hint_chunks[last_idx],
        );
    }

    frame.render_widget(
        Paragraph::new(Span::styled(
            version_label,
            Style::default().fg(FG_OVERLAY),
        ))
        .style(bar_style)
        .alignment(ratatui::layout::Alignment::Right),
        bar_chunks[1],
    );
}
