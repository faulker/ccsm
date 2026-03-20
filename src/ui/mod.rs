mod ansi;
mod info_bar;
mod modals;
mod preview_pane;
mod session_list;
pub(crate) mod util;

use crate::app::{App, AppMode};
use crate::theme::{
    ACCENT_BLUE, ACCENT_MAUVE, ACCENT_PEACH, BG_SURFACE, FG_OVERLAY, FG_SUBTEXT, HIGHLIGHT_BG,
};
use crate::update::UpdateStatus;
use ratatui::{
    layout::{Constraint, Direction, Layout},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, List, ListState, Paragraph, Wrap},
    Frame,
};

use self::info_bar::{build_title_spans, render_status_bar};
use self::modals::{
    draw_duplicate_popup, draw_naming_popup, draw_rename_popup, draw_update_prompt,
    render_help_popup,
};
use self::preview_pane::{build_live_preview_text, build_preview_text};
use self::session_list::{build_flat_items, build_tree_items};
use self::util::{estimate_wrapped_height, live_dot_style};

/// Render the full TUI frame: session list, preview pane, info bar, status bar,
/// and any active modal overlay (rename, update prompt, help, naming popup).
pub fn draw(frame: &mut Frame, app: &mut App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(3), Constraint::Length(1)])
        .split(frame.area());

    let main_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(30), Constraint::Percentage(70)])
        .split(chunks[0]);

    let session_panel_inner_width = main_chunks[0].width.saturating_sub(2) as usize;

    // Session list (filtered or tree)
    let items = if app.tree_view {
        build_tree_items(app, session_panel_inner_width)
    } else {
        build_flat_items(app)
    };

    let title_spans = build_title_spans(app);

    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(ACCENT_BLUE))
                .title(Line::from(title_spans))
                .style(Style::default().bg(BG_SURFACE)),
        )
        .highlight_style(
            Style::default()
                .bg(HIGHLIGHT_BG)
                .fg(ACCENT_BLUE)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("▶ ");

    let mut state = ListState::default();
    state.select(Some(app.selected));
    frame.render_stateful_widget(list, main_chunks[0], &mut state);

    // Preview
    let is_live_selected = app.selected_live_index().is_some();
    let live_preview_raw = if is_live_selected {
        app.current_live_preview()
    } else {
        String::new()
    };

    let (meta, preview_slice) = app.current_preview();
    let meta = meta.clone();
    let preview = preview_slice.to_vec();
    let preview_text = if is_live_selected {
        build_live_preview_text(&live_preview_raw)
    } else {
        build_preview_text(&preview)
    };

    let right_area = main_chunks[1];

    let right_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(vec![Constraint::Length(3), Constraint::Min(3)])
        .split(right_area);

    // Session info bar (always visible)
    let mut spans: Vec<Span> = Vec::new();
    if is_live_selected {
        if let Some(idx) = app.selected_live_index() {
            let ls = &app.live_sessions[idx];
            let (dot, dot_style) = live_dot_style(app, idx);
            spans.push(Span::styled(format!("{} ", dot), dot_style));
            spans.push(Span::styled(ls.display_name.clone(), Style::default().fg(ACCENT_PEACH).add_modifier(Modifier::BOLD)));
            spans.push(Span::raw("  "));
            spans.push(Span::styled(" ", Style::default().fg(FG_OVERLAY)));
            spans.push(Span::styled(ls.cwd.clone(), Style::default().fg(FG_SUBTEXT)));
        }
    } else {
        if meta.all_session_ids.len() > 1 {
            let last_id: String = meta.all_session_ids.last()
                .map(|id| id.chars().take(8).collect())
                .unwrap_or_default();
            let extra = meta.all_session_ids.len() - 1;
            spans.push(Span::styled(
                format!(" # {}", last_id),
                Style::default().fg(ACCENT_BLUE),
            ));
            spans.push(Span::styled(
                format!(" +{}", extra),
                Style::default().fg(FG_SUBTEXT),
            ));
        } else if let Some(id) = &meta.session_id {
            let short_id: String = id.chars().take(8).collect();
            spans.push(Span::styled(
                format!(" # {}", short_id),
                Style::default().fg(ACCENT_BLUE),
            ));
        }
        if let Some(name) = &meta.session_name {
            if !spans.is_empty() {
                spans.push(Span::raw("  "));
            }
            spans.push(Span::styled(
                name.clone(),
                Style::default().fg(ACCENT_PEACH).add_modifier(Modifier::BOLD),
            ));
        }
        let fallback_cwd = if meta.cwd.is_some() {
            meta.cwd.clone()
        } else {
            app.selected_cwd()
        };
        if let Some(cwd) = &fallback_cwd {
            if !spans.is_empty() {
                spans.push(Span::raw("  "));
            }
            spans.push(Span::styled(" ", Style::default().fg(FG_OVERLAY)));
            spans.push(Span::styled(cwd.clone(), Style::default().fg(FG_SUBTEXT)));
        }
        if let Some(branch) = &meta.git_branch {
            if !spans.is_empty() {
                spans.push(Span::raw("  "));
            }
            spans.push(Span::styled(" ⎇ ", Style::default().fg(ACCENT_MAUVE)));
            spans.push(Span::styled(
                branch.clone(),
                Style::default().fg(ACCENT_MAUVE),
            ));
        }
    }

    let info_bar = Paragraph::new(Line::from(spans)).block(
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(FG_OVERLAY))
            .style(Style::default().bg(BG_SURFACE)),
    );
    frame.render_widget(info_bar, right_chunks[0]);

    // Preview pane
    let preview_area = right_chunks[1];
    let inner_width = preview_area.width.saturating_sub(2) as usize; // borders
    let inner_height = preview_area.height.saturating_sub(2); // borders
    let content_height = if is_live_selected {
        preview_text.lines.len()
    } else {
        estimate_wrapped_height(&preview_text, inner_width)
    };
    let max_scroll = (content_height as u16).saturating_sub(inner_height);
    if app.preview_auto_scroll && is_live_selected {
        app.preview_scroll = max_scroll;
    } else if app.preview_scroll > max_scroll {
        app.preview_scroll = max_scroll;
        if is_live_selected {
            app.preview_auto_scroll = true;
        }
    }

    let mut preview_widget = Paragraph::new(preview_text)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(FG_OVERLAY))
                .title(Span::styled(
                    " Preview ",
                    Style::default().fg(FG_SUBTEXT),
                ))
                .style(Style::default().bg(BG_SURFACE)),
        )
        .scroll((app.preview_scroll, 0));
    if !is_live_selected {
        preview_widget = preview_widget.wrap(Wrap { trim: false });
    }

    frame.render_widget(preview_widget, preview_area);

    // Status bar
    render_status_bar(frame, app, chunks[1]);

    // Rename popup overlay
    if app.mode == AppMode::Renaming {
        draw_rename_popup(frame, &app.rename_input);
    }

    // Update prompt overlay
    if app.mode == AppMode::UpdatePrompt {
        if let UpdateStatus::Available(ref info) = app.update_status {
            draw_update_prompt(frame, info);
        }
    }

    // Help overlay
    if app.mode == AppMode::Help {
        render_help_popup(frame, frame.area());
    }

    // NamingSession overlay (centered popup)
    if app.mode == AppMode::NamingSession {
        draw_naming_popup(frame, &app.naming_input, &app.naming_placeholder);
    }

    // Config popup
    if app.mode == AppMode::Config {
        crate::config_popup::draw_config_popup(frame, app);
    }

    // MissingDeps popup
    if app.mode == AppMode::MissingDeps {
        crate::config_popup::draw_missing_deps_popup(frame, app);
    }

    // DuplicateSession confirmation popup
    if app.mode == AppMode::DuplicateSession {
        if let Some(ref name) = app.duplicate_name.clone() {
            draw_duplicate_popup(frame, name);
        }
    }
}
