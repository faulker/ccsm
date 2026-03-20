use ratatui::{
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::ListItem,
};
use unicode_width::UnicodeWidthStr;

use crate::app::{App, FlatRow, TreeRow};
use crate::config::DisplayMode;
use crate::theme::{
    ACCENT_GREEN, ACCENT_PEACH, ACCENT_TEAL, FG_OVERLAY, FG_SUBTEXT, FG_TEXT,
};

use super::util::{activity_count_spans, format_relative_date, live_dot_style, truncate_left_plain};

/// Build the list items for the session list panel in tree view.
pub fn build_tree_items(app: &App, panel_inner_width: usize) -> Vec<ListItem<'static>> {
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
                let count_str = session_count.to_string();
                let overhead = 5 + count_str.len();
                let available = panel_inner_width.saturating_sub(overhead);
                let display = if app.display_mode == DisplayMode::FullDir && project_name.width() > available {
                    truncate_left_plain(project_name, available)
                } else {
                    project_name.clone()
                };
                let is_favorite = app.favorites.contains(project);
                let mut header_spans = vec![Span::styled(
                    format!("{} {} ({})", arrow, display, session_count),
                    Style::default()
                        .fg(ACCENT_TEAL)
                        .add_modifier(Modifier::BOLD),
                )];
                if is_favorite {
                    header_spans.push(Span::styled(
                        " ★",
                        Style::default().fg(ACCENT_PEACH).add_modifier(Modifier::BOLD),
                    ));
                }
                let (active, idle, waiting) = app.project_activity_counts(project);
                header_spans.extend(activity_count_spans(active, idle, waiting));
                ListItem::new(Line::from(header_spans))
            }
            TreeRow::Session { session_index } => {
                let s = &app.sessions[*session_index];
                let date = format_relative_date(s.last_timestamp);
                let entry_count = app.chain_entry_count(*session_index);
                let chain_len = app.chain_map.get(session_index).map(|v| v.len()).unwrap_or(1);
                let mut spans = vec![
                    Span::raw("  "),
                    Span::styled(format!("{:<8}", date), Style::default().fg(FG_SUBTEXT)),
                    Span::styled(
                        format!("  {:>4} msg", entry_count),
                        Style::default()
                            .fg(FG_OVERLAY)
                            .add_modifier(Modifier::DIM),
                    ),
                ];
                if app.group_chains {
                    if chain_len > 1 {
                        spans.push(Span::styled(
                            format!("  ×{:<2}", chain_len),
                            Style::default().fg(FG_OVERLAY),
                        ));
                    } else {
                        spans.push(Span::raw("     "));
                    }
                }
                if let Some(name) = app.chain_name_for(*session_index) {
                    spans.push(Span::styled(
                        format!("  {}", name),
                        Style::default().fg(ACCENT_PEACH),
                    ));
                }
                ListItem::new(Line::from(spans))
            }
            TreeRow::RunningHeader { project, count } => {
                let key = format!("running:{}", project);
                let arrow = if app.collapsed.contains(&key) { "▸" } else { "▾" };
                ListItem::new(Line::from(vec![
                    Span::raw("  "),
                    Span::styled(
                        format!("{} Running ({})", arrow, count),
                        Style::default().fg(ACCENT_GREEN).add_modifier(Modifier::BOLD),
                    ),
                ]))
            }
            TreeRow::HistoryHeader { project, count } => {
                let key = format!("history:{}", project);
                let arrow = if app.collapsed.contains(&key) { "▸" } else { "▾" };
                ListItem::new(Line::from(vec![
                    Span::raw("  "),
                    Span::styled(
                        format!("{} History ({})", arrow, count),
                        Style::default().fg(FG_SUBTEXT),
                    ),
                ]))
            }
            TreeRow::LiveItem { live_index } => {
                let ls = &app.live_sessions[*live_index];
                let (dot, dot_style) = live_dot_style(app, *live_index);
                ListItem::new(Line::from(vec![
                    Span::raw("    "),
                    Span::styled(format!("{} ", dot), dot_style),
                    Span::styled(
                        ls.display_name.clone(),
                        Style::default().fg(FG_TEXT).add_modifier(Modifier::BOLD),
                    ),
                ]))
            }
            TreeRow::FavoritesSeparator => {
                ListItem::new(Line::from(Span::styled(
                    "───────────────────────────────────────────────",
                    Style::default().fg(FG_OVERLAY),
                )))
            }
        })
        .collect()
}

/// Build the list items for the session list panel in flat view.
pub fn build_flat_items(app: &App) -> Vec<ListItem<'static>> {
    app.flat_rows
        .iter()
        .map(|row| match row {
            FlatRow::RunningHeader { count } => {
                ListItem::new(Line::from(vec![
                    Span::styled(
                        format!("▾ Running ({})", count),
                        Style::default().fg(ACCENT_GREEN).add_modifier(Modifier::BOLD),
                    ),
                ]))
            }
            FlatRow::LiveItem { live_index } => {
                let ls = &app.live_sessions[*live_index];
                let (dot, dot_style) = live_dot_style(app, *live_index);
                ListItem::new(Line::from(vec![
                    Span::styled(format!("{} ", dot), dot_style),
                    Span::styled(ls.display_name.clone(), Style::default().fg(FG_TEXT).add_modifier(Modifier::BOLD)),
                    Span::raw("  "),
                    Span::styled(ls.project_name.clone(), Style::default().fg(FG_SUBTEXT)),
                ]))
            }
            FlatRow::Separator => {
                ListItem::new(Line::from(vec![
                    Span::styled(
                        "─────────────────────────────────── history ───",
                        Style::default().fg(FG_OVERLAY),
                    ),
                ]))
            }
            FlatRow::FavoritesSeparator => {
                ListItem::new(Line::from(Span::styled(
                    "───────────────────────────────────────────────",
                    Style::default().fg(FG_OVERLAY),
                )))
            }
            FlatRow::HistoryItem { session_index } => {
                let s = &app.sessions[*session_index];
                let is_favorite = app.favorites.contains(&s.project);
                let name = app.display_name(s);
                let date = format_relative_date(s.last_timestamp);
                let entry_count = app.chain_entry_count(*session_index);
                let chain_len = app.chain_map.get(session_index).map(|v| v.len()).unwrap_or(1);
                let mut spans = vec![
                    Span::styled(
                        if is_favorite { "★ " } else { "  " },
                        Style::default().fg(ACCENT_PEACH),
                    ),
                    Span::styled(
                        if app.display_mode == DisplayMode::FullDir {
                            super::util::truncate_left(&name, 28)
                        } else {
                            super::util::truncate(&name, 28)
                        },
                        Style::default().fg(FG_TEXT),
                    ),
                    Span::raw("  "),
                    Span::styled(format!("{:<8}", date), Style::default().fg(FG_SUBTEXT)),
                    Span::styled(
                        format!("  {:>4} msg", entry_count),
                        Style::default()
                            .fg(FG_OVERLAY)
                            .add_modifier(Modifier::DIM),
                    ),
                ];
                if app.group_chains {
                    if chain_len > 1 {
                        spans.push(Span::styled(
                            format!("  ×{:<2}", chain_len),
                            Style::default().fg(FG_OVERLAY),
                        ));
                    } else {
                        spans.push(Span::raw("     "));
                    }
                }
                ListItem::new(Line::from(spans))
            }
        })
        .collect()
}
