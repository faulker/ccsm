use ratatui::{
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::ListItem,
};
use unicode_width::UnicodeWidthStr;

use crate::app::{App, FlatRow, TreeRow};
use crate::config::DisplayMode;
use crate::data::AgentBackend;
use crate::theme::{
    ACCENT_GREEN, ACCENT_PEACH, ACCENT_TEAL, FG_OVERLAY, FG_SUBTEXT, FG_TEXT,
};

use super::util::{activity_count_spans, format_relative_date, live_dot_style, truncate_left_plain};

/// One-column backend mark for a historical session row (`C` Claude Code, `A` Cursor Agent).
fn backend_mark(backend: AgentBackend) -> Span<'static> {
    let color = match backend {
        AgentBackend::ClaudeCode => ACCENT_PEACH,
        AgentBackend::CursorAgent => ACCENT_TEAL,
    };
    Span::styled(backend.mark(), Style::default().fg(color))
}

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
                // Nest under the History header the same way live rows nest under Running
                // (four spaces), so the A/C mark sits to the right of the History ▾/▸.
                let mut spans = vec![
                    Span::raw("    "),
                    backend_mark(s.backend),
                    Span::raw(" "),
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
                let mut spans = vec![
                    Span::raw("    "),
                    Span::styled(format!("{} ", dot), dot_style),
                ];
                if let Some(backend) = ls.backend {
                    spans.push(backend_mark(backend));
                    spans.push(Span::raw(" "));
                }
                spans.push(Span::styled(
                    ls.display_name.clone(),
                    Style::default().fg(FG_TEXT).add_modifier(Modifier::BOLD),
                ));
                ListItem::new(Line::from(spans))
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
                let mut spans = vec![Span::styled(format!("{} ", dot), dot_style)];
                if let Some(backend) = ls.backend {
                    spans.push(backend_mark(backend));
                    spans.push(Span::raw(" "));
                }
                spans.push(Span::styled(
                    ls.display_name.clone(),
                    Style::default().fg(FG_TEXT).add_modifier(Modifier::BOLD),
                ));
                spans.push(Span::raw("  "));
                spans.push(Span::styled(
                    ls.project_name.clone(),
                    Style::default().fg(FG_SUBTEXT),
                ));
                ListItem::new(Line::from(spans))
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
                    backend_mark(s.backend),
                    Span::raw(" "),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::App;
    use crate::config::Config;
    use crate::data::{AgentBackend, SessionInfo};

    fn sample_sessions() -> Vec<SessionInfo> {
        vec![
            SessionInfo {
                session_id: "claude-1".into(),
                project: "/p".into(),
                project_name: "p".into(),
                first_timestamp: 1,
                last_timestamp: 3000,
                entry_count: 2,
                has_data: true,
                name: Some("claude chat".into()),
                slug: None,
                backend: AgentBackend::ClaudeCode,
            },
            SessionInfo {
                session_id: "cursor-1".into(),
                project: "/p".into(),
                project_name: "p".into(),
                first_timestamp: 1,
                last_timestamp: 4000,
                entry_count: 2,
                has_data: true,
                name: Some("cursor chat".into()),
                slug: None,
                backend: AgentBackend::CursorAgent,
            },
        ]
    }

    #[test]
    fn flat_history_rows_carry_backend_marks() {
        let mut app = App::new(sample_sessions(), None, Config::default());
        app.live_sessions.clear();
        app.tree_view = false;
        app.group_chains = false;
        app.hide_empty = false;
        app.recompute_filter();
        // Debug formatting includes Span content; enough to assert the marks.
        let debug = format!("{:?}", build_flat_items(&app));
        assert!(
            debug.contains("Span::from(\"A\")"),
            "Cursor row should show A:\n{debug}"
        );
        assert!(
            debug.contains("Span::from(\"C\")"),
            "Claude row should show C:\n{debug}"
        );
    }

    #[test]
    fn backend_mark_letters() {
        assert_eq!(backend_mark(AgentBackend::ClaudeCode).content.as_ref(), "C");
        assert_eq!(backend_mark(AgentBackend::CursorAgent).content.as_ref(), "A");
        assert_eq!(AgentBackend::ClaudeCode.label(), "Claude Code");
        assert_eq!(AgentBackend::CursorAgent.label(), "Cursor Agent");
        assert_eq!(AgentBackend::ClaudeCode.tmux_tag(), "claude");
        assert_eq!(AgentBackend::CursorAgent.tmux_tag(), "cursor");
        assert_eq!(
            AgentBackend::from_tmux_tag("cursor"),
            Some(AgentBackend::CursorAgent)
        );
        assert_eq!(AgentBackend::from_tmux_tag("nope"), None);
    }

    #[test]
    fn flat_live_rows_carry_backend_marks() {
        let mut app = App::new(vec![], None, Config::default());
        app.live_sessions = vec![crate::live::LiveSession {
            tmux_name: "live-1".into(),
            display_name: "live-1".into(),
            cwd: "/p".into(),
            project_name: "p".into(),
            job_id: None,
            backend: Some(AgentBackend::CursorAgent),
        }];
        app.tree_view = false;
        app.recompute_flat_rows();
        let debug = format!("{:?}", build_flat_items(&app));
        assert!(
            debug.contains("Span::from(\"A\")"),
            "live Cursor row should show A:\n{debug}"
        );
    }

    /// Flatten a list item's spans into plain text for indent assertions.
    fn item_text(item: &ListItem<'_>) -> String {
        // ratatui's Span Debug uses `Span::from("…")` for plain content.
        let debug = format!("{item:?}");
        let mut out = String::new();
        let mut rest = debug.as_str();
        while let Some(start) = rest.find("Span::from(\"") {
            rest = &rest[start + "Span::from(\"".len()..];
            let Some(end) = rest.find('"') else { break };
            out.push_str(&rest[..end]);
            rest = &rest[end + 1..];
        }
        out
    }

    #[test]
    fn tree_history_sessions_indent_past_history_arrow() {
        let mut app = App::new(sample_sessions(), None, Config::default());
        app.live_sessions.clear();
        app.tree_view = true;
        app.hide_empty = false;
        app.group_chains = false;
        app.recompute_filter();
        // Projects start collapsed; expand so History + session rows are visible.
        app.collapsed.clear();
        app.recompute_tree();

        let items = build_tree_items(&app, 60);
        let texts: Vec<String> = items.iter().map(item_text).collect();
        let history = texts
            .iter()
            .find(|t| t.contains("History"))
            .expect(&format!("missing History header in {texts:?}"));
        let session = texts
            .iter()
            .find(|t| t.starts_with("    C") || t.starts_with("    A"))
            .expect(&format!("missing indented session row in {texts:?}"));

        assert!(
            history.starts_with("  ▾ History") || history.starts_with("  ▸ History"),
            "History header should use two-space section indent: {history:?}"
        );
        assert!(
            session.starts_with("    "),
            "History session rows should use four-space nest indent: {session:?}"
        );
        let hist_mark_col = 2usize; // "  ▾" — arrow at column 2
        let sess_mark_col = session.chars().take_while(|c| c.is_whitespace()).count();
        assert!(
            sess_mark_col > hist_mark_col,
            "A/C mark (col {sess_mark_col}) must sit right of History arrow (col {hist_mark_col}):\n{texts:?}"
        );
    }
}
