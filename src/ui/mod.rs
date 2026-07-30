mod ansi;
pub(crate) mod config_tab;
mod dir_picker;
mod info_bar;
mod jobs_tab;
mod modals;
mod preview_pane;
mod session_list;
pub(crate) mod util;

use crate::app::{App, AppMode, MainTab};
use crate::theme::{
    ACCENT_BLUE, ACCENT_MAUVE, ACCENT_PEACH, ACCENT_TEAL, BG_SURFACE, FG_OVERLAY, FG_SUBTEXT,
    HIGHLIGHT_BG,
};
use crate::update::UpdateStatus;
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, List, ListState, Paragraph, Wrap},
    Frame,
};
use unicode_width::UnicodeWidthStr;

use self::config_tab::{draw_config_tab, draw_missing_deps_popup};
use self::dir_picker::draw_dir_picker;
use self::info_bar::{build_title_spans, build_usage_status_spans, render_status_bar};
use self::jobs_tab::{draw_job_confirm_popup, draw_job_form_popup, draw_jobs_tab};
use self::modals::{
    draw_duplicate_popup, draw_naming_popup, draw_rename_popup, draw_stop_confirm_popup,
    draw_update_prompt, render_help_popup,
};
use self::preview_pane::{build_live_preview_text, build_preview_text};
use self::session_list::{build_flat_items, build_tree_items};
use self::util::{estimate_wrapped_height, live_dot_style};

/// Render the full TUI frame: tab strip, the active tab's two panes, the
/// status bar, and any active modal overlay (rename, update prompt, help,
/// naming popup, job form).
pub fn draw(frame: &mut Frame, app: &mut App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(3),
            Constraint::Length(1),
        ])
        .split(frame.area());

    render_tab_bar(frame, app, chunks[0]);

    // Sessions and Jobs are list-plus-detail, where the detail is the point.
    // Config is the other way round: its rows carry full paths and prompt text,
    // so the list needs the wider half or every value renders as an ellipsis.
    let (left, right) = match app.main_tab {
        MainTab::Config => (55, 45),
        _ => (30, 70),
    };
    let main_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(left), Constraint::Percentage(right)])
        .split(chunks[1]);

    match app.main_tab {
        MainTab::Sessions => draw_sessions_tab(frame, app, main_chunks[0], main_chunks[1]),
        MainTab::Jobs => draw_jobs_tab(frame, app, main_chunks[0], main_chunks[1]),
        MainTab::Config => draw_config_tab(frame, app, main_chunks[0], main_chunks[1]),
    }

    // Status bar
    render_status_bar(frame, app, chunks[2]);

    draw_overlays(frame, app);
}

/// Render the top tab strip. The active tab is highlighted; the Jobs tab
/// carries a job count so a running schedule is visible from the other tabs.
/// The usage/watcher chip is right-aligned here rather than in either tab's
/// list title so it stays visible no matter which tab is open.
fn render_tab_bar(frame: &mut Frame, app: &App, area: Rect) {
    let bar_style = Style::default().bg(HIGHLIGHT_BG);
    let active = Style::default()
        .fg(ACCENT_PEACH)
        .bg(HIGHLIGHT_BG)
        .add_modifier(Modifier::BOLD | Modifier::UNDERLINED);
    let inactive = Style::default().fg(FG_SUBTEXT).bg(HIGHLIGHT_BG);

    let jobs_label = if app.jobs.is_empty() {
        " Jobs ".to_string()
    } else {
        format!(" Jobs ({}) ", app.jobs.len())
    };

    let spans = vec![
        Span::styled(
            " Sessions ",
            if app.main_tab == MainTab::Sessions { active } else { inactive },
        ),
        Span::styled("  ", bar_style),
        Span::styled(
            jobs_label,
            if app.main_tab == MainTab::Jobs { active } else { inactive },
        ),
        Span::styled("  ", bar_style),
        Span::styled(
            " Config ",
            if app.main_tab == MainTab::Config { active } else { inactive },
        ),
        Span::styled("   ", bar_style),
        Span::styled("Tab", Style::default().fg(FG_OVERLAY).bg(HIGHLIGHT_BG)),
        Span::styled(" switch", Style::default().fg(FG_OVERLAY).bg(HIGHLIGHT_BG)),
    ];

    // The chip is built without a background, so paint the bar background onto
    // each span before it lands in the tab strip.
    let chip: Vec<Span<'static>> = build_usage_status_spans(app)
        .into_iter()
        .map(|s| Span::styled(s.content, s.style.bg(HIGHLIGHT_BG)))
        .collect();

    if chip.is_empty() {
        frame.render_widget(Paragraph::new(Line::from(spans)).style(bar_style), area);
        return;
    }

    let chip_width: u16 = chip
        .iter()
        .map(|s| s.content.width() as u16)
        .sum::<u16>()
        .saturating_add(1); // trailing pad from the right edge
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Fill(1), Constraint::Length(chip_width)])
        .split(area);

    frame.render_widget(Paragraph::new(Line::from(spans)).style(bar_style), cols[0]);
    frame.render_widget(
        Paragraph::new(Line::from(chip))
            .style(bar_style)
            .alignment(Alignment::Right),
        cols[1],
    );
}

/// Render the Sessions tab: the session list plus the info bar and preview pane.
fn draw_sessions_tab(frame: &mut Frame, app: &mut App, list_area: Rect, right_area: Rect) {
    let main_chunks = [list_area, right_area];
    let session_panel_inner_width = main_chunks[0].width.saturating_sub(2) as usize;

    // Session list (filtered or tree)
    let items = if app.tree_view {
        build_tree_items(app, session_panel_inner_width)
    } else {
        build_flat_items(app)
    };

    let title_spans = build_title_spans(app, main_chunks[0].width);

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
            if let Some(backend) = ls.backend {
                let color = match backend {
                    crate::data::AgentBackend::ClaudeCode => ACCENT_PEACH,
                    crate::data::AgentBackend::CursorAgent => ACCENT_TEAL,
                };
                spans.push(Span::raw("  "));
                spans.push(Span::styled(
                    backend.short_label().to_string(),
                    Style::default().fg(color).add_modifier(Modifier::BOLD),
                ));
            }
            spans.push(Span::raw("  "));
            spans.push(Span::styled(ls.cwd.clone(), Style::default().fg(FG_SUBTEXT)));
        }
    } else {
        // Name the agent first so list marks are unambiguous in the details bar.
        if let Some(idx) = app.selected_session_index() {
            let backend = app.sessions[idx].backend;
            let color = match backend {
                crate::data::AgentBackend::ClaudeCode => ACCENT_PEACH,
                crate::data::AgentBackend::CursorAgent => ACCENT_TEAL,
            };
            spans.push(Span::styled(
                format!(" {} ", backend.short_label()),
                Style::default().fg(color).add_modifier(Modifier::BOLD),
            ));
        }
        if meta.all_session_ids.len() > 1 {
            let last_id: String = meta.all_session_ids.last()
                .map(|id| id.chars().take(8).collect())
                .unwrap_or_default();
            let extra = meta.all_session_ids.len() - 1;
            spans.push(Span::styled(
                format!("# {}", last_id),
                Style::default().fg(ACCENT_BLUE),
            ));
            spans.push(Span::styled(
                format!(" +{}", extra),
                Style::default().fg(FG_SUBTEXT),
            ));
        } else if let Some(id) = &meta.session_id {
            let short_id: String = id.chars().take(8).collect();
            spans.push(Span::styled(
                format!("# {}", short_id),
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
    // Live pane lines are often wider than this preview column; wrap them the
    // same way as history so they cannot paint past the pane into the list.
    let content_height = estimate_wrapped_height(&preview_text, inner_width);
    let max_scroll = (content_height as u16).saturating_sub(inner_height);
    if app.preview_auto_scroll && is_live_selected {
        app.preview_scroll = max_scroll;
    } else if app.preview_scroll > max_scroll {
        app.preview_scroll = max_scroll;
        if is_live_selected {
            app.preview_auto_scroll = true;
        }
    }

    let preview_widget = Paragraph::new(preview_text)
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
        .scroll((app.preview_scroll, 0))
        .wrap(Wrap { trim: false });

    frame.render_widget(preview_widget, preview_area);
}

/// Render whichever modal overlay the current `AppMode` calls for, on top of
/// the active tab.
fn draw_overlays(frame: &mut Frame, app: &mut App) {
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
        // The popup clamps the scroll to its content, and the clamped value is
        // written back so a held `j` cannot run the offset off the end.
        app.help_scroll = render_help_popup(frame, frame.area(), app.help_tab, app.help_scroll);
    }

    // NamingSession overlay (centered popup)
    if app.mode == AppMode::NamingSession {
        let cwd_is_repo = app
            .naming_cwd
            .as_deref()
            .map(crate::live::is_git_repo)
            .unwrap_or(false);
        draw_naming_popup(
            frame,
            &app.naming_input,
            &app.naming_placeholder,
            app.naming_mode,
            cwd_is_repo,
            app.naming_backend,
            app.source_filter == crate::app::SourceFilter::Both,
            app.naming_focus,
        );
    }

    // MissingDeps popup
    if app.mode == AppMode::MissingDeps {
        draw_missing_deps_popup(frame, app);
    }

    // Stop-live-session confirmation popup
    if app.mode == AppMode::StopSessionConfirm {
        if let Some(name) = app.stop_confirm_name.clone() {
            draw_stop_confirm_popup(frame, &name);
        }
    }

    // DuplicateSession confirmation popup
    if app.mode == AppMode::DuplicateSession {
        if let Some(ref name) = app.duplicate_name.clone() {
            draw_duplicate_popup(frame, name);
        }
    }

    // Directory-picker overlay
    if app.mode == AppMode::DirPicker {
        draw_dir_picker(frame, app);
    }

    // Job create/edit form overlay
    if app.mode == AppMode::JobForm {
        draw_job_form_popup(frame, app);
    }

    // Job stop/delete confirmation overlay
    if app.mode == AppMode::JobConfirm {
        draw_job_confirm_popup(frame, app);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::schedule::store::WatchState;
    use ratatui::{backend::TestBackend, Terminal};

    /// An App with a live watcher reporting 78% usage, and no other schedule
    /// state inherited from the host machine.
    fn app_with_usage() -> App {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        let mut app = App::new(vec![], None, Config::default());
        app.jobs = vec![];
        app.mode = AppMode::Normal;
        app.watch_state = Some(WatchState {
            pid: 1234,
            started_at_ms: now,
            heartbeat_ms: now,
            last_usage_pct: Some(78.0),
            last_usage_at_ms: Some(now),
            reset_at_ms: None,
            usage_error: None,
            version: Some(env!("CARGO_PKG_VERSION").to_string()),
        });
        app
    }

    /// Render a full frame and return the text of the tab strip (row 0).
    fn tab_bar_text(app: &mut App) -> String {
        let mut terminal = Terminal::new(TestBackend::new(100, 20)).unwrap();
        terminal.draw(|f| draw(f, app)).unwrap();
        let buffer = terminal.backend().buffer().clone();
        (0..100)
            .map(|x| buffer[(x, 0)].symbol().to_string())
            .collect()
    }

    #[test]
    fn usage_chip_renders_on_the_sessions_tab() {
        let mut app = app_with_usage();
        app.main_tab = MainTab::Sessions;
        assert!(tab_bar_text(&mut app).contains("78%"));
    }

    #[test]
    fn usage_chip_renders_on_the_jobs_tab() {
        let mut app = app_with_usage();
        app.main_tab = MainTab::Jobs;
        assert!(tab_bar_text(&mut app).contains("78%"));
    }

    // --- Small-terminal acceptance ---

    /// Draw the whole frame at `w`x`h` in `mode` and return the screen text.
    fn screen(mode: AppMode, w: u16, h: u16) -> String {
        let mut app = App::new(vec![], None, Config::default());
        app.jobs = vec![];
        app.watch_state = None;
        app.mode = mode;
        app.naming_placeholder = "auto-name".into();
        app.naming_cwd = Some("/tmp".into());
        app.stop_confirm_name = Some("sess".into());
        app.duplicate_name = Some("sess".into());

        let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
        terminal.draw(|f| draw(f, &mut app)).unwrap();
        let buf = terminal.backend().buffer().clone();
        (0..h)
            .map(|y| {
                (0..w)
                    .map(|x| buf[(x, y)].symbol().to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// The goal in PLAN.md: the relevant shortcuts stay visible on a small
    /// screen. Help and quit are the two that must never be the ones cut.
    #[test]
    fn shortcuts_survive_a_small_terminal() {
        for (w, h) in [(60u16, 20u16), (80, 24)] {
            let text = screen(AppMode::Normal, w, h);
            assert!(text.contains("? help"), "{w}x{h}:\n{text}");
            assert!(text.contains("q quit"), "{w}x{h}:\n{text}");
        }
    }

    #[test]
    fn every_modal_renders_its_content_at_60x20() {
        let cases = [
            (AppMode::Help, "Navigation"),
            (AppMode::NamingSession, "plain"),
            (AppMode::StopSessionConfirm, "Stop session"),
            (AppMode::DuplicateSession, "already exists"),
        ];
        for (mode, needle) in cases {
            let text = screen(mode.clone(), 60, 20);
            assert!(
                text.contains(needle),
                "{mode:?} lost {needle:?} at 60x20:\n{text}"
            );
        }
    }

    #[test]
    fn help_documents_the_source_filter_and_cursor_rename() {
        // Tall enough that the Actions block (including `s`) fits without scroll.
        let text = screen(AppMode::Help, 100, 40);
        assert!(
            text.contains("Cycle source") || text.contains("source filter"),
            "missing source filter help:\n{text}"
        );
        assert!(text.contains("Claude only"), "missing jobs Claude-only note:\n{text}");
    }

    /// The config list holds ~28 lines but gets 18 rows at 60x20, so the
    /// About block used to be unreachable. It follows the cursor now.
    #[test]
    fn the_config_tab_scrolls_to_reach_its_last_row() {
        let mut app = App::new(vec![], None, Config::default());
        app.jobs = vec![];
        app.watch_state = None;
        app.main_tab = MainTab::Config;
        // The URL row is the last one in the About block.
        app.config_selected = crate::ui::config_tab::URL_ROW;

        let mut terminal = Terminal::new(TestBackend::new(60, 20)).unwrap();
        terminal.draw(|f| draw(f, &mut app)).unwrap();
        let buf = terminal.backend().buffer().clone();
        let text: String = (0..20)
            .map(|y| {
                (0..60)
                    .map(|x| buf[(x, y)].symbol().to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("About"), "{text}");
        // The list pane is 33 columns wide here, so the URL is truncated; what
        // matters is that the marker reached the last row at all.
        assert!(text.contains("\u{25b6} https://github.com/faulker"), "{text}");
    }

    /// Draw a tab (rather than a mode) at `w`x`h` and return the screen text.
    fn tab_screen(tab: MainTab, w: u16, h: u16) -> String {
        let mut app = App::new(vec![], None, Config::default());
        app.jobs = vec![];
        app.watch_state = None;
        app.main_tab = tab;

        let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
        terminal.draw(|f| draw(f, &mut app)).unwrap();
        let buf = terminal.backend().buffer().clone();
        (0..h)
            .map(|y| {
                (0..w)
                    .map(|x| buf[(x, y)].symbol().to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn the_tab_strip_lists_all_three_tabs() {
        let mut app = app_with_usage();
        app.main_tab = MainTab::Config;
        let text = tab_bar_text(&mut app);
        for label in ["Sessions", "Jobs", "Config"] {
            assert!(text.contains(label), "missing {label}:\n{text}");
        }
    }

    #[test]
    fn usage_chip_renders_on_the_config_tab() {
        let mut app = app_with_usage();
        app.main_tab = MainTab::Config;
        assert!(tab_bar_text(&mut app).contains("78%"));
    }

    /// The whole point of the move off a popup: the settings and their
    /// explanation share the window instead of one covering the other.
    #[test]
    fn the_config_tab_shows_settings_beside_their_explanation() {
        let text = tab_screen(MainTab::Config, 100, 30);
        assert!(text.contains("Hide empty projects"), "{text}");
        assert!(text.contains("About this setting"), "{text}");
        // Row 0 is selected on a fresh App, so its help is what shows.
        assert!(text.contains("crashed"), "{text}");
    }

    /// Nothing draws on top of the Config tab, so its content survives the
    /// smallest terminal the rest of the app is tested at.
    #[test]
    fn the_config_tab_renders_its_first_setting_at_60x20() {
        let text = tab_screen(MainTab::Config, 60, 20);
        assert!(text.contains("Hide empty projects"), "{text}");
    }

    #[test]
    fn help_documents_session_marks_and_naming_focus() {
        // Tall enough that Session marks and the new-session popup block fit.
        let text = screen(AppMode::Help, 100, 50);
        assert!(
            text.contains("Claude Code session") && text.contains("Cursor Agent session"),
            "missing C/A mark explanations:\n{text}"
        );
        assert!(
            text.contains("Move focus") || text.contains("name → Agent"),
            "missing naming focus help:\n{text}"
        );
    }

    #[test]
    fn the_naming_popup_shows_type_row() {
        let text = screen(AppMode::NamingSession, 80, 24);
        assert!(text.contains("Type:"), "missing Type row:\n{text}");
        assert!(text.contains("plain"), "missing plain mode:\n{text}");
    }

    #[test]
    fn live_preview_info_bar_names_the_agent_after_the_session_name() {
        let mut app = App::new(vec![], None, Config::default());
        app.jobs = vec![];
        app.watch_state = None;
        app.live_sessions = vec![crate::live::LiveSession {
            tmux_name: "my-live".into(),
            display_name: "my-live".into(),
            cwd: "/tmp/proj".into(),
            project_name: "proj".into(),
            job_id: None,
            backend: Some(crate::data::AgentBackend::CursorAgent),
        }];
        app.tree_view = false;
        app.recompute_flat_rows();
        // Select the live row (after the Running header).
        app.selected = app
            .flat_rows
            .iter()
            .position(|r| matches!(r, crate::app::FlatRow::LiveItem { .. }))
            .expect("live row");

        let mut terminal = Terminal::new(TestBackend::new(100, 24)).unwrap();
        terminal.draw(|f| draw(f, &mut app)).unwrap();
        let buf = terminal.backend().buffer().clone();
        let text: String = (0..24)
            .map(|y| {
                (0..100)
                    .map(|x| buf[(x, y)].symbol().to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");
        // Name first, then short agent label — matches the info-bar layout.
        let name_at = text.find("my-live").expect("session name");
        let agent_at = text.find("Cursor").expect("agent label");
        assert!(
            agent_at > name_at,
            "agent should appear after the session name:\n{text}"
        );
        assert!(
            !text.contains("Cursor Agent"),
            "live info bar should use the short label:\n{text}"
        );
    }

    #[test]
    fn history_preview_info_bar_uses_short_agent_label() {
        let mut app = App::new(
            vec![crate::data::SessionInfo {
                session_id: "hist-1".into(),
                project: "/tmp/proj".into(),
                project_name: "proj".into(),
                first_timestamp: 1,
                last_timestamp: 2,
                entry_count: 1,
                has_data: true,
                name: Some("a chat".into()),
                slug: None,
                backend: crate::data::AgentBackend::ClaudeCode,
            }],
            None,
            Config::default(),
        );
        app.jobs = vec![];
        app.watch_state = None;
        app.live_sessions.clear();
        app.tree_view = false;
        app.recompute_filter();
        app.recompute_flat_rows();
        app.selected = app
            .flat_rows
            .iter()
            .position(|r| matches!(r, crate::app::FlatRow::HistoryItem { .. }))
            .expect("history row");

        let mut terminal = Terminal::new(TestBackend::new(100, 24)).unwrap();
        terminal.draw(|f| draw(f, &mut app)).unwrap();
        let buf = terminal.backend().buffer().clone();
        let text: String = (0..24)
            .map(|y| {
                (0..100)
                    .map(|x| buf[(x, y)].symbol().to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("Claude"), "missing short label:\n{text}");
        assert!(
            !text.contains("Claude Code"),
            "history info bar should use the short label:\n{text}"
        );
    }

    /// Long live-pane lines must wrap inside the preview column, not paint into
    /// the session list on the left.
    #[test]
    fn live_preview_long_lines_stay_in_preview_pane() {
        let mut app = App::new(vec![], None, Config::default());
        app.jobs = vec![];
        app.watch_state = None;
        app.live_sessions = vec![crate::live::LiveSession {
            tmux_name: "wide-live".into(),
            display_name: "wide-live".into(),
            cwd: "/tmp".into(),
            project_name: "tmp".into(),
            job_id: None,
            backend: Some(crate::data::AgentBackend::ClaudeCode),
        }];
        // Wider than any reasonable preview column; distinctive so list chrome
        // cannot be mistaken for leaked preview content.
        let long_line = "W".repeat(400);
        app.live_preview_cache.insert(
            "wide-live".into(),
            (long_line, std::time::Instant::now()),
        );
        app.tree_view = false;
        app.recompute_flat_rows();
        app.selected = app
            .flat_rows
            .iter()
            .position(|r| matches!(r, crate::app::FlatRow::LiveItem { .. }))
            .expect("live row");

        const W: u16 = 100;
        const H: u16 = 24;
        let mut terminal = Terminal::new(TestBackend::new(W, H)).unwrap();
        terminal.draw(|f| draw(f, &mut app)).unwrap();
        let buf = terminal.backend().buffer().clone();

        // Sessions list is 30% of the body row (under the tab strip).
        let list_width = (W as u32 * 30 / 100) as u16;
        let mut leaked = 0u32;
        let mut preview_w = 0u32;
        for y in 1..H.saturating_sub(1) {
            for x in 0..list_width.saturating_sub(1) {
                if buf[(x, y)].symbol() == "W" {
                    leaked += 1;
                }
            }
            for x in list_width..W {
                if buf[(x, y)].symbol() == "W" {
                    preview_w += 1;
                }
            }
        }
        assert!(
            preview_w > 0,
            "expected wrapped live preview content in the right pane"
        );
        assert_eq!(
            leaked, 0,
            "live preview must not paint into the session list (found {leaked} W cells)"
        );
    }
}
