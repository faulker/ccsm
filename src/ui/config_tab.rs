//! The Config tab: a peer of the session list and the jobs manager in the main
//! window, laid out as a settings list on the left and an explanation of the
//! selected setting on the right.
//!
//! This used to be a centered popup. Settings are a place you *browse*, not an
//! interruption, and as a popup it had to scroll ~30 lines through 18 rows on a
//! small terminal while hiding whatever you were configuring. As a tab it gets
//! the full window height and a detail pane wide enough to say what a setting
//! actually does.
//!
//! Mirrors `jobs_tab.rs`'s shape of holding both the `impl App` key handler and
//! the draw functions in one file. The missing-dependencies dialog stays here
//! (and stays a modal) because it is the one config-adjacent thing that must
//! block the app before any tab is usable.

use crate::app::{App, AppMode, PickerTarget};
use crate::config::{Config, PauseMode};
use crate::keys::normalize_key;
use crossterm::event::{KeyCode, KeyModifiers};

/// Maximum row index in the config list (0-based).
pub const CONFIG_MAX_ROW: usize = 15;

/// Row index of the hide-empty-projects toggle.
const HIDE_EMPTY_ROW: usize = 0;

/// Row index of the group-session-chains toggle.
const GROUP_CHAINS_ROW: usize = 1;

/// Row index of the session view/display mode cycler.
const VIEW_ROW: usize = 2;

/// Row index of the `claude` binary path.
const CLAUDE_PATH_ROW: usize = 3;

/// Row index of the Cursor `agent` binary path.
const AGENT_PATH_ROW: usize = 4;

/// Row index of the `tmux` binary path.
const TMUX_PATH_ROW: usize = 5;

/// Row index of the usage history file path.
const USAGE_PATH_ROW: usize = 6;

/// Row index of the usage percentage at which jobs pause.
const PAUSE_PERCENT_ROW: usize = 7;

/// Row index of the usage percentage at which jobs resume.
const RESUME_PERCENT_ROW: usize = 8;

/// Row index of the usage-staleness window, in minutes.
pub(crate) const USAGE_STALE_ROW: usize = 9;

/// Row index of the pause-mode toggle.
const PAUSE_MODE_ROW: usize = 10;

/// Row index of the watcher auto-start toggle.
const WATCH_AUTOSTART_ROW: usize = 11;

/// Row index of the idle-completion timeout, in minutes.
pub(crate) const IDLE_COMPLETE_ROW: usize = 12;

/// Row index of the default continue-prompt text field.
pub(crate) const CONTINUE_PROMPT_ROW: usize = 13;

/// Row index of the developer credit, which is highlightable but inert.
const CREDIT_ROW: usize = 14;

/// Row index of the project URL, which opens in a browser rather than editing.
pub(crate) const URL_ROW: usize = 15;

/// Rows 3..=6 are file paths (three binaries and the usage history file):
/// they open the file picker, and can also be typed by hand.
const PATH_ROWS: std::ops::RangeInclusive<usize> = 3..=6;
use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, Paragraph, Wrap},
    Frame,
};

use crate::theme::{
    ACCENT_BLUE, ACCENT_GREEN, ACCENT_MAUVE, ACCENT_PEACH, ACCENT_RED, BG_SURFACE, FG_OVERLAY,
    FG_SUBTEXT, FG_TEXT, HIGHLIGHT_BG,
};
use crate::ui::util::{centered_rect, input_spans};

/// What the primary key does on a given settings row, so the status bar can
/// name it instead of always claiming `Space toggle`.
pub(crate) enum RowAction {
    /// A browsable path: `Enter` browses, `i` types it by hand.
    Browse,
    /// The project URL: `Enter` opens a browser.
    OpenUrl,
    /// Anything typed rather than toggled.
    Edit,
    /// A checkbox or a cycled value.
    Toggle,
}

/// Classify a settings row for the status bar.
pub(crate) fn row_action(row: usize) -> RowAction {
    match row {
        r if PATH_ROWS.contains(&r) => RowAction::Browse,
        URL_ROW => RowAction::OpenUrl,
        PAUSE_PERCENT_ROW | RESUME_PERCENT_ROW | USAGE_STALE_ROW | IDLE_COMPLETE_ROW
        | CONTINUE_PROMPT_ROW => RowAction::Edit,
        _ => RowAction::Toggle,
    }
}

impl App {
    /// Handle a key event while the Config tab has focus (Normal mode).
    ///
    /// Handles the global keys the other tabs also honour (quit, help, tab
    /// switching) so all three tabs feel like one window, then the settings
    /// navigation and editing.
    pub(crate) fn handle_config_tab_event(&mut self, key: crossterm::event::KeyEvent) {
        // If editing a path field, delegate to text input
        if self.config_editing {
            use crossterm::event::Event;
            use tui_input::backend::crossterm::EventHandler;

            match key.code {
                KeyCode::Enter => {
                    let value = self.config_path_input.value().trim().to_string();
                    let mut commit_ok = true;
                    match self.config_selected {
                        CLAUDE_PATH_ROW => {
                            self.config.claude_path = optional_from_input(value);
                        }
                        AGENT_PATH_ROW => {
                            self.config.agent_path = optional_from_input(value);
                        }
                        TMUX_PATH_ROW => {
                            self.config.tmux_path = optional_from_input(value);
                        }
                        USAGE_PATH_ROW => {
                            self.config.usage_history_path = optional_from_input(value);
                        }
                        PAUSE_PERCENT_ROW => match parse_percent(&value) {
                            Some(v) if percent_ordering_ok(v, self.config.usage_resume_percent) => {
                                self.config.usage_pause_percent = v;
                            }
                            Some(_) => {
                                self.status_error =
                                    Some("Pause % must be greater than resume %".to_string());
                                commit_ok = false;
                            }
                            None => {
                                self.status_error =
                                    Some("Pause % must be a number between 1 and 100".to_string());
                                commit_ok = false;
                            }
                        },
                        RESUME_PERCENT_ROW => match parse_percent(&value) {
                            Some(v) if percent_ordering_ok(self.config.usage_pause_percent, v) => {
                                self.config.usage_resume_percent = v;
                            }
                            Some(_) => {
                                self.status_error =
                                    Some("Resume % must be less than pause %".to_string());
                                commit_ok = false;
                            }
                            None => {
                                self.status_error =
                                    Some("Resume % must be a number between 1 and 100".to_string());
                                commit_ok = false;
                            }
                        },
                        // Also entered in minutes, but "0" is not a legal value
                        // here: a zero window makes every sample stale, and a
                        // stale sample can never resume a paused job, so the
                        // scheduler would stop resuming entirely.
                        USAGE_STALE_ROW => match parse_stale_minutes(&value) {
                            Some(minutes) => {
                                self.config.usage_max_age_seconds = minutes * 60;
                            }
                            None => {
                                self.status_error = Some(
                                    "Usage staleness must be a whole number of minutes (1 or more)"
                                        .to_string(),
                                );
                                commit_ok = false;
                            }
                        },
                        // Entered in minutes because the useful range is tens
                        // of minutes; stored in seconds like every other
                        // duration in the config. "0" (or "off") disables it.
                        IDLE_COMPLETE_ROW => match parse_minutes(&value) {
                            Some(minutes) => {
                                self.config.idle_complete_seconds = minutes * 60;
                            }
                            None => {
                                self.status_error = Some(
                                    "Idle completion must be a whole number of minutes (0 = off)"
                                        .to_string(),
                                );
                                commit_ok = false;
                            }
                        },
                        // An emptied field falls back to the built-in default
                        // rather than storing "", which would paste nothing
                        // into a paused session and leave it stuck.
                        CONTINUE_PROMPT_ROW => {
                            self.config.continue_prompt = if value.is_empty() {
                                Config::default().continue_prompt
                            } else {
                                value
                            };
                        }
                        _ => {}
                    }
                    if commit_ok {
                        if let Err(e) = self.config.save() {
                            self.status_error = Some(format!("Failed to save config: {e}"));
                        }
                        if PATH_ROWS.contains(&self.config_selected) {
                            self.refresh_bin_availability();
                            self.missing_usage = crate::usage::source_unavailable(
                                &self.config.usage_source,
                                self.config.usage_history_override(),
                            );
                        }
                    }
                    self.config_editing = false;
                    self.config_path_input = tui_input::Input::default();
                }
                KeyCode::Esc => {
                    self.config_editing = false;
                    self.config_path_input = tui_input::Input::default();
                }
                _ => {
                    // Normalized so Shift+letter types uppercase under the
                    // enhanced keyboard protocol; arrows/Home/End/Delete are
                    // handled by tui_input, so the cursor moves mid-string.
                    self.config_path_input.handle_event(&Event::Key(normalize_key(key)));
                }
            }
            return;
        }

        match (key.code, key.modifiers) {
            (KeyCode::Char('c'), KeyModifiers::CONTROL) | (KeyCode::Char('q'), _) => {
                self.should_quit = true;
            }
            (KeyCode::Esc, _) => {
                if self.leave_config_tab() {
                    self.open_sessions_tab();
                }
            }
            (KeyCode::Tab, _) => {
                if self.leave_config_tab() {
                    self.cycle_main_tab(true);
                }
            }
            (KeyCode::BackTab, _) => {
                if self.leave_config_tab() {
                    self.cycle_main_tab(false);
                }
            }
            (KeyCode::Char('?'), _) | (KeyCode::Char('/'), KeyModifiers::SHIFT) => {
                self.open_help();
            }
            (KeyCode::Char('j'), _) | (KeyCode::Down, _) => {
                if self.config_selected < CONFIG_MAX_ROW {
                    self.config_selected += 1;
                }
            }
            (KeyCode::Char('k'), _) | (KeyCode::Up, _) => {
                if self.config_selected > 0 {
                    self.config_selected -= 1;
                }
            }
            (KeyCode::Home, _) => self.config_selected = 0,
            (KeyCode::End, _) => self.config_selected = CONFIG_MAX_ROW,
            // The view row cycles six combinations, so it gets a way back as
            // well as a way forward. Shift+Tab used to be it, but Tab now
            // belongs to the tab strip.
            (KeyCode::Left, _) if self.config_selected == VIEW_ROW => {
                self.cycle_view_backward();
                self.selected = 0;
                self.preview_scroll = u16::MAX;
                self.save_config();
            }
            (KeyCode::Right, _) if self.config_selected == VIEW_ROW => {
                self.cycle_view_forward();
                self.selected = 0;
                self.preview_scroll = u16::MAX;
                self.save_config();
            }
            (KeyCode::Char(' '), _) | (KeyCode::Enter, _) => {
                match self.config_selected {
                    HIDE_EMPTY_ROW => {
                        self.hide_empty = !self.hide_empty;
                        self.recompute_filter();
                        self.preview_scroll = u16::MAX;
                        self.save_config();
                    }
                    GROUP_CHAINS_ROW => {
                        self.group_chains = !self.group_chains;
                        self.preview_cache.clear();
                        self.recompute_filter();
                        self.preview_scroll = u16::MAX;
                        self.save_config();
                    }
                    VIEW_ROW => {
                        self.cycle_view_forward();
                        self.selected = 0;
                        self.preview_scroll = u16::MAX;
                        self.save_config();
                    }
                    // Binary paths open the file picker; `i` (below) types one by hand.
                    CLAUDE_PATH_ROW => self.browse_config_path(PickerTarget::ConfigClaude),
                    AGENT_PATH_ROW => self.browse_config_path(PickerTarget::ConfigAgent),
                    TMUX_PATH_ROW => self.browse_config_path(PickerTarget::ConfigTmux),
                    USAGE_PATH_ROW => self.browse_config_path(PickerTarget::ConfigUsage),
                    PAUSE_PERCENT_ROW => {
                        self.config_editing = true;
                        let current = format!("{:.1}", self.config.usage_pause_percent);
                        self.config_path_input = tui_input::Input::from(current);
                    }
                    RESUME_PERCENT_ROW => {
                        self.config_editing = true;
                        let current = format!("{:.1}", self.config.usage_resume_percent);
                        self.config_path_input = tui_input::Input::from(current);
                    }
                    USAGE_STALE_ROW => {
                        self.config_editing = true;
                        let current = (self.config.usage_max_age_seconds / 60).to_string();
                        self.config_path_input = tui_input::Input::from(current);
                    }
                    PAUSE_MODE_ROW => {
                        self.config.pause_mode = match self.config.pause_mode {
                            PauseMode::Soft => PauseMode::Hard,
                            PauseMode::Hard => PauseMode::Soft,
                        };
                        self.save_config();
                    }
                    WATCH_AUTOSTART_ROW => {
                        self.config.watch_autostart = !self.config.watch_autostart;
                        self.save_config();
                    }
                    IDLE_COMPLETE_ROW => {
                        self.config_editing = true;
                        let current = (self.config.idle_complete_seconds / 60).to_string();
                        self.config_path_input = tui_input::Input::from(current);
                    }
                    CONTINUE_PROMPT_ROW => {
                        self.config_editing = true;
                        self.config_path_input =
                            tui_input::Input::from(self.config.continue_prompt.clone());
                    }
                    URL_ROW => {
                        open_url("https://github.com/faulker/ccsm");
                    }
                    _ => {}
                }
            }
            // Manual entry for the browsable path rows, so a path that isn't
            // convenient to browse to (or doesn't exist yet) can still be typed.
            (KeyCode::Char('i'), _) if PATH_ROWS.contains(&self.config_selected) => {
                self.config_editing = true;
                let current = match self.config_selected {
                    CLAUDE_PATH_ROW => self.config.claude_path.clone(),
                    AGENT_PATH_ROW => self.config.agent_path.clone(),
                    TMUX_PATH_ROW => self.config.tmux_path.clone(),
                    _ => self.config.usage_history_path.clone(),
                }
                .unwrap_or_default();
                self.config_path_input = tui_input::Input::from(current);
            }
            _ => {}
        }
    }

    /// Re-check binaries on the way out of the Config tab.
    ///
    /// Returns `false` when the blocking dialog must come back: tmux missing,
    /// or both agent binaries missing. Exactly one of claude/agent missing is
    /// soft and does not block leaving.
    fn leave_config_tab(&mut self) -> bool {
        self.refresh_bin_availability();
        if self.deps_blocking() {
            self.mode = AppMode::MissingDeps;
            return false;
        }
        true
    }

    /// Open the file picker for one of the binary-path settings, starting from
    /// its current value.
    fn browse_config_path(&mut self, target: PickerTarget) {
        let current = match target {
            PickerTarget::ConfigClaude => self.config.claude_path.clone(),
            PickerTarget::ConfigAgent => self.config.agent_path.clone(),
            PickerTarget::ConfigTmux => self.config.tmux_path.clone(),
            _ => self.config.usage_history_path.clone(),
        }
        .unwrap_or_default();
        self.open_path_picker(target, &current);
    }

    /// Handle a key event while the missing-deps dialog is shown.
    pub(crate) fn handle_missing_deps_event(&mut self, key: crossterm::event::KeyEvent) {
        match key.code {
            KeyCode::Char('s') => {
                self.open_config_tab();
                // The binary paths are the reason this dialog appeared, so land
                // on the first of them rather than on the session preferences.
                self.config_selected = CLAUDE_PATH_ROW;
            }
            KeyCode::Char('q') | KeyCode::Esc => {
                self.should_quit = true;
            }
            _ => {}
        }
    }
}

/// Render the Config tab: the settings list on the left, grouped under labelled
/// sections so it is obvious which ones drive the jobs manager and which are
/// session-list preferences, and an explanation of the selected setting on the
/// right.
pub fn draw_config_tab(frame: &mut Frame, app: &App, list_area: Rect, detail_area: Rect) {
    draw_settings_list(frame, app, list_area);
    draw_setting_detail(frame, app, detail_area);
}

/// Render the left pane: every setting as one row, with the selected one marked
/// and scrolled into view.
fn draw_settings_list(frame: &mut Frame, app: &App, area: Rect) {
    let selected = app.config_selected;
    let header_style = Style::default().fg(ACCENT_MAUVE).add_modifier(Modifier::BOLD);

    let row_style = |idx: usize| -> Style {
        if selected == idx {
            Style::default().fg(ACCENT_BLUE).bg(HIGHLIGHT_BG)
        } else {
            Style::default().fg(FG_TEXT)
        }
    };
    let marker = |idx: usize| -> &'static str {
        if selected == idx { "▶ " } else { "  " }
    };
    let row = |idx: usize, text: String| -> Line<'static> {
        Line::from(Span::styled(format!("{}{}", marker(idx), text), row_style(idx)))
    };
    let section = |title: &str| -> Line<'static> {
        Line::from(Span::styled(format!("  {title}"), header_style))
    };

    // A path row renders the live input (with a real cursor) while it is being
    // edited by hand, otherwise the stored value or the "(default)" fallback.
    let path_row = |idx: usize, label: &str, value: Option<&String>, default_label: String| -> Line<'static> {
        let style = row_style(idx);
        let mut spans = vec![Span::styled(format!("{}{}: ", marker(idx), label), style)];
        if app.config_editing && selected == idx {
            spans.extend(input_spans(&app.config_path_input, style));
        } else {
            spans.push(Span::styled(
                value.cloned().unwrap_or(default_label),
                style,
            ));
        }
        Line::from(spans)
    };

    let percent_row = |idx: usize, label: &str, value: f64| -> Line<'static> {
        let style = row_style(idx);
        let mut spans = vec![Span::styled(format!("{}{}: ", marker(idx), label), style)];
        if app.config_editing && selected == idx {
            spans.extend(input_spans(&app.config_path_input, style));
        } else {
            spans.push(Span::styled(format!("{:.1}%", value), style));
        }
        Line::from(spans)
    };

    let view_label = format!(
        "{} · {}",
        if app.tree_view { "Tree" } else { "Flat" },
        app.display_mode.label()
    );

    let mut content: Vec<Line> = Vec::new();
    content.push(Line::from(""));

    content.push(section("Sessions"));
    content.push(row(
        HIDE_EMPTY_ROW,
        format!("[{}] Hide empty projects", if app.hide_empty { "x" } else { " " }),
    ));
    content.push(row(
        GROUP_CHAINS_ROW,
        format!("[{}] Group session chains", if app.group_chains { "x" } else { " " }),
    ));
    content.push(row(VIEW_ROW, format!("View: {}", view_label)));
    content.push(path_row(
        CLAUDE_PATH_ROW,
        "Claude binary",
        app.config.claude_path.as_ref(),
        "claude (default)".to_string(),
    ));
    content.push(path_row(
        AGENT_PATH_ROW,
        "Agent binary",
        app.config.agent_path.as_ref(),
        "agent (default)".to_string(),
    ));
    content.push(path_row(
        TMUX_PATH_ROW,
        "Tmux binary",
        app.config.tmux_path.as_ref(),
        "tmux (default)".to_string(),
    ));

    content.push(Line::from(""));
    content.push(section("Jobs manager (scheduler & watcher)"));
    content.push(path_row(
        USAGE_PATH_ROW,
        "Usage history file",
        app.config.usage_history_path.as_ref(),
        format!(
            "{} (default)",
            crate::usage::local::history_path(None).display()
        ),
    ));
    content.push(percent_row(PAUSE_PERCENT_ROW, "Pause jobs at usage", app.config.usage_pause_percent));
    content.push(percent_row(RESUME_PERCENT_ROW, "Resume jobs at usage", app.config.usage_resume_percent));

    content.push({
        // Phrased as the wait it causes rather than as "max age", because that
        // is what it does to a paused job: a stale reading can pause but never
        // resume, so a job sits until a sample inside this window arrives.
        let style = row_style(USAGE_STALE_ROW);
        let mut spans = vec![Span::styled(
            format!("{}Usage sample stale after: ", marker(USAGE_STALE_ROW)),
            style,
        )];
        if app.config_editing && selected == USAGE_STALE_ROW {
            spans.extend(input_spans(&app.config_path_input, style));
        } else {
            spans.push(Span::styled(
                format!("{} min", app.config.usage_max_age_seconds / 60),
                style,
            ));
        }
        Line::from(spans)
    });

    content.push(row(PAUSE_MODE_ROW, format!("Pause mode: {}", app.config.pause_mode.label())));
    content.push(row(
        WATCH_AUTOSTART_ROW,
        format!(
            "[{}] Auto-start watcher",
            if app.config.watch_autostart { "x" } else { " " }
        ),
    ));

    content.push({
        // Spelled out rather than shown as a bare number, because "0" alone
        // reads as "immediately" when it actually means the fallback is off
        // and only the completion marker can finish a job.
        let style = row_style(IDLE_COMPLETE_ROW);
        let mut spans = vec![Span::styled(
            format!("{}Idle completion: ", marker(IDLE_COMPLETE_ROW)),
            style,
        )];
        if app.config_editing && selected == IDLE_COMPLETE_ROW {
            spans.extend(input_spans(&app.config_path_input, style));
        } else if app.config.idle_complete_seconds == 0 {
            spans.push(Span::styled("off (marker only)".to_string(), style));
        } else {
            spans.push(Span::styled(
                format!("after {} min idle", app.config.idle_complete_seconds / 60),
                style,
            ));
        }
        Line::from(spans)
    });

    content.push({
        // Shown, not hidden behind "(default)", because this is the text the
        // watcher actually pastes to wake every paused job.
        let style = row_style(CONTINUE_PROMPT_ROW);
        let mut spans = vec![Span::styled(
            format!("{}Continue prompt: ", marker(CONTINUE_PROMPT_ROW)),
            style,
        )];
        if app.config_editing && selected == CONTINUE_PROMPT_ROW {
            spans.extend(input_spans(&app.config_path_input, style));
        } else {
            spans.push(Span::styled(app.config.continue_prompt.clone(), style));
        }
        Line::from(spans)
    });

    content.push(Line::from(""));
    content.push(section("About"));
    // The version lives here rather than in the status bar, where it used to
    // reserve 8 columns of shortcut space at every terminal width.
    content.push(Line::from(Span::styled(
        format!("  ccsm v{}", env!("CARGO_PKG_VERSION")),
        Style::default().fg(FG_SUBTEXT),
    )));
    content.push({
        let style = if selected == CREDIT_ROW {
            Style::default().fg(ACCENT_MAUVE).bg(HIGHLIGHT_BG)
        } else {
            Style::default().fg(ACCENT_MAUVE)
        };
        Line::from(Span::styled(
            format!("{}Developed by Winter Faulk", marker(CREDIT_ROW)),
            style,
        ))
    });
    content.push({
        let marker_style = if selected == URL_ROW {
            Style::default().fg(ACCENT_BLUE).bg(HIGHLIGHT_BG)
        } else {
            Style::default().fg(ACCENT_BLUE)
        };
        let url_style = if selected == URL_ROW {
            Style::default()
                .fg(ACCENT_BLUE)
                .add_modifier(Modifier::UNDERLINED)
                .bg(HIGHLIGHT_BG)
        } else {
            Style::default()
                .fg(ACCENT_BLUE)
                .add_modifier(Modifier::UNDERLINED)
        };
        Line::from(vec![
            Span::styled(marker(URL_ROW), marker_style),
            Span::styled("https://github.com/faulker/ccsm", url_style),
        ])
    });

    // The list holds ~28 lines and the tab gives it the full window height, but
    // an 80x24 terminal is still 6 rows short. Scroll to keep the selected row
    // in view: the list is already j/k-navigable, so following the cursor is
    // better than a second scroll offset the user has to drive.
    // The marker is the ground truth for which line is selected.
    let selected_line = content
        .iter()
        .position(|l| {
            l.spans
                .first()
                .is_some_and(|s| s.content.starts_with('\u{25b6}'))
        })
        .unwrap_or(0) as u16;
    let view_height = area.height.saturating_sub(2); // borders
    let max_scroll = (content.len() as u16).saturating_sub(view_height);
    // Keep one row of context below the cursor where there is room for it.
    let scroll = selected_line
        .saturating_sub(view_height.saturating_sub(2))
        .min(max_scroll);

    let list = Paragraph::new(content).scroll((scroll, 0)).block(
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(ACCENT_PEACH))
            .title(Span::styled(
                " Settings ",
                Style::default()
                    .fg(ACCENT_PEACH)
                    .add_modifier(Modifier::BOLD),
            ))
            .style(Style::default().bg(BG_SURFACE)),
    );
    frame.render_widget(list, area);
}

/// Render the right pane: what the selected setting does, the value it holds
/// right now, and the keys that act on it.
///
/// The popup this replaced had room for one hint line and nothing else, so
/// every setting's meaning lived only in the README. A setting you cannot
/// explain in place is one people leave at its default.
fn draw_setting_detail(frame: &mut Frame, app: &App, area: Rect) {
    let title_style = Style::default()
        .fg(ACCENT_PEACH)
        .add_modifier(Modifier::BOLD);
    let label_style = Style::default().fg(FG_SUBTEXT);
    let value_style = Style::default().fg(FG_TEXT);
    let key_style = Style::default().fg(ACCENT_PEACH).add_modifier(Modifier::BOLD);

    let help = setting_help(app.config_selected);

    let mut lines: Vec<Line> = vec![
        Line::from(Span::styled(help.title.to_string(), title_style)),
        Line::from(""),
    ];

    if let Some(value) = current_value(app, app.config_selected) {
        lines.push(Line::from(vec![
            Span::styled("Current: ", label_style),
            Span::styled(value, value_style),
        ]));
        lines.push(Line::from(""));
    }

    for paragraph in help.body {
        lines.push(Line::from(Span::styled(
            paragraph.to_string(),
            Style::default().fg(FG_TEXT),
        )));
        lines.push(Line::from(""));
    }

    lines.push(Line::from(Span::styled(
        "Keys".to_string(),
        Style::default().fg(ACCENT_MAUVE).add_modifier(Modifier::BOLD),
    )));
    for (keys, what) in key_hints(app.config_selected, app.config_editing) {
        lines.push(Line::from(vec![
            Span::styled(format!("{:<14}", keys), key_style),
            Span::styled(what.to_string(), label_style),
        ]));
    }

    let detail = Paragraph::new(lines)
        .wrap(Wrap { trim: false })
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(FG_OVERLAY))
                .title(Span::styled(" About this setting ", label_style))
                .style(Style::default().bg(BG_SURFACE)),
        );
    frame.render_widget(detail, area);
}

/// A setting's heading and explanation, shown in the detail pane.
struct SettingHelp {
    /// Heading, matching the row's label in the list.
    title: &'static str,
    /// One paragraph per element, rendered with a blank line between them.
    body: &'static [&'static str],
}

/// What each settings row means. Kept as data rather than a match arm per line
/// so a new row is one entry, not three edits.
fn setting_help(row: usize) -> SettingHelp {
    let (title, body): (&str, &[&str]) = match row {
        HIDE_EMPTY_ROW => (
            "Hide empty projects",
            &["Drops projects whose sessions hold no readable messages from the session list. They are usually sessions that crashed or were closed before the first prompt landed."],
        ),
        GROUP_CHAINS_ROW => (
            "Group session chains",
            &["Resumed sessions form a chain: each resume writes a new file that continues the last one. Grouped, a chain shows as one row with a count; ungrouped, every link gets its own row."],
        ),
        VIEW_ROW => (
            "View",
            &[
                "How the Sessions tab lists sessions: Tree groups them under their project directory, Flat is one newest-first list.",
                "The second half is what each row is labelled with: the session name, its first prompt, or its directory.",
                "Also cycled with v from the Sessions tab, or with --flat at startup.",
            ],
        ),
        CLAUDE_PATH_ROW => (
            "Claude binary",
            &["Which claude executable ccsm launches. Leave it empty to use whatever claude is on PATH. Set it when claude lives somewhere your login shell does not export."],
        ),
        AGENT_PATH_ROW => (
            "Agent binary",
            &["Which Cursor agent executable ccsm launches. Leave it empty to use whatever agent is on PATH. Listing and previewing Cursor chats needs no binary at all."],
        ),
        TMUX_PATH_ROW => (
            "Tmux binary",
            &["Which tmux executable runs live sessions and the watcher. Leave it empty to use whatever tmux is on PATH."],
        ),
        USAGE_PATH_ROW => (
            "Usage history file",
            &["Claude Desktop's plan-usage-history.json, which ccsm reads for the account usage percentage that drives the pause and resume thresholds. Point at it here if it lives somewhere non-standard."],
        ),
        PAUSE_PERCENT_ROW => (
            "Pause jobs at usage",
            &[
                "Once account usage reaches this percentage, the watcher pauses running jobs so a long-running job cannot burn through the rest of the window.",
                "Must be greater than the resume percentage.",
            ],
        ),
        RESUME_PERCENT_ROW => (
            "Resume jobs at usage",
            &[
                "Paused jobs with auto-resume on restart once usage falls back to this percentage, or when the window resets.",
                "Must be less than the pause percentage, otherwise the watcher would pause and resume the same job on the same reading.",
            ],
        ),
        USAGE_STALE_ROW => (
            "Usage sample stale after",
            &[
                "How old a usage reading may be and still count. A stale reading can pause a job but never resume one, so a paused job waits until a fresh sample arrives.",
                "Cannot be 0: that would make every sample stale and nothing would ever resume.",
            ],
        ),
        PAUSE_MODE_ROW => (
            "Pause mode",
            &[
                "Soft leaves the tmux session running and simply stops sending it work, so its scrollback survives.",
                "Hard stops the session outright, freeing the process; the job is relaunched from its transcript on resume.",
            ],
        ),
        WATCH_AUTOSTART_ROW => (
            "Auto-start watcher",
            &["Starts the ccsm-watch daemon automatically when a job is created. With it off, jobs sit queued until you start the watcher yourself with s on the Jobs tab."],
        ),
        IDLE_COMPLETE_ROW => (
            "Idle completion",
            &[
                "A fallback for agents that never emit the CCSM_JOB_COMPLETE marker: a running job whose pane sits idle for this long is treated as finished.",
                "The timer measures one unbroken stretch, so any activity resets it. 0 (or \"off\") means only the marker can finish a job.",
            ],
        ),
        CONTINUE_PROMPT_ROW => (
            "Continue prompt",
            &[
                "The text the watcher pastes into a paused session to wake it back up. Individual jobs can override it in the job form.",
                "Clearing the field restores the built-in default rather than storing an empty prompt, which would wake a session with nothing to do.",
            ],
        ),
        CREDIT_ROW | URL_ROW => (
            "About ccsm",
            &["Claude Code Session Manager: a terminal UI for browsing, resuming, and scheduling Claude Code sessions through tmux."],
        ),
        _ => ("Settings", &[]),
    };
    SettingHelp { title, body }
}

/// The selected setting's current value, spelled out in full for the detail
/// pane. The list truncates long paths and prompts; this does not.
fn current_value(app: &App, row: usize) -> Option<String> {
    let path_value = |stored: Option<&String>, default: String| -> String {
        stored
            .cloned()
            .unwrap_or_else(|| format!("{default} (default)"))
    };
    Some(match row {
        HIDE_EMPTY_ROW => bool_label(app.hide_empty),
        GROUP_CHAINS_ROW => bool_label(app.group_chains),
        VIEW_ROW => format!(
            "{} · {}",
            if app.tree_view { "Tree" } else { "Flat" },
            app.display_mode.label()
        ),
        CLAUDE_PATH_ROW => path_value(app.config.claude_path.as_ref(), "claude".to_string()),
        AGENT_PATH_ROW => path_value(app.config.agent_path.as_ref(), "agent".to_string()),
        TMUX_PATH_ROW => path_value(app.config.tmux_path.as_ref(), "tmux".to_string()),
        USAGE_PATH_ROW => path_value(
            app.config.usage_history_path.as_ref(),
            crate::usage::local::history_path(None).display().to_string(),
        ),
        PAUSE_PERCENT_ROW => format!("{:.1}%", app.config.usage_pause_percent),
        RESUME_PERCENT_ROW => format!("{:.1}%", app.config.usage_resume_percent),
        USAGE_STALE_ROW => format!("{} min", app.config.usage_max_age_seconds / 60),
        PAUSE_MODE_ROW => app.config.pause_mode.label().to_string(),
        WATCH_AUTOSTART_ROW => bool_label(app.config.watch_autostart),
        IDLE_COMPLETE_ROW => {
            if app.config.idle_complete_seconds == 0 {
                "off (marker only)".to_string()
            } else {
                format!("after {} min idle", app.config.idle_complete_seconds / 60)
            }
        }
        CONTINUE_PROMPT_ROW => app.config.continue_prompt.clone(),
        // The version lives here rather than in the status bar, where it used
        // to reserve 8 columns of shortcut space at every terminal width.
        CREDIT_ROW | URL_ROW => format!(
            "v{} · https://github.com/faulker/ccsm",
            env!("CARGO_PKG_VERSION")
        ),
        _ => return None,
    })
}

/// "on"/"off" rather than "true"/"false", matching how the row's checkbox reads.
fn bool_label(on: bool) -> String {
    if on { "on".to_string() } else { "off".to_string() }
}

/// The keys that act on the selected row, in display order.
fn key_hints(row: usize, editing: bool) -> Vec<(&'static str, &'static str)> {
    if editing {
        return vec![
            ("Enter", "save"),
            ("Esc", "cancel"),
            ("←/→ Home/End", "move the cursor"),
            ("(empty)", "restore the default"),
        ];
    }
    let mut hints = match row {
        r if PATH_ROWS.contains(&r) => vec![
            ("Enter", "browse for a path"),
            ("i", "type the path by hand"),
        ],
        URL_ROW => vec![("Enter", "open in a browser")],
        CREDIT_ROW => vec![],
        VIEW_ROW => vec![("Enter", "next view"), ("←/→", "cycle either way")],
        PAUSE_PERCENT_ROW | RESUME_PERCENT_ROW | USAGE_STALE_ROW | IDLE_COMPLETE_ROW
        | CONTINUE_PROMPT_ROW => vec![("Enter", "edit the value")],
        _ => vec![("Space/Enter", "toggle")],
    };
    hints.push(("j/k ↑/↓", "move between settings"));
    hints.push(("Tab", "next tab"));
    hints
}

/// Render the missing-dependencies dialog.
pub fn draw_missing_deps_popup(frame: &mut Frame, app: &App) {
    let area = centered_rect(50, 30, frame.area());
    // Three binary lines (claude / agent / tmux) plus the key hints.
    let area = Rect { height: 10.min(area.height), ..area };
    frame.render_widget(Clear, area);

    let key_style = Style::default().fg(ACCENT_PEACH).add_modifier(Modifier::BOLD);
    let hint_style = Style::default().fg(FG_SUBTEXT);

    let bin_line = |ok: bool, name: &str| -> Line<'static> {
        if ok {
            Line::from(Span::styled(
                format!("  ✓ {name} found"),
                Style::default().fg(ACCENT_GREEN),
            ))
        } else {
            Line::from(Span::styled(
                format!("  ✗ {name} not found"),
                Style::default().fg(ACCENT_RED),
            ))
        }
    };

    let content = vec![
        Line::from(""),
        bin_line(!app.missing_claude, "claude"),
        bin_line(!app.missing_agent, "agent"),
        bin_line(!app.missing_tmux, "tmux"),
        Line::from(""),
        Line::from(vec![
            Span::styled("  s", key_style),
            Span::styled(" set paths  ", hint_style),
            Span::styled("q", key_style),
            Span::styled(" quit", hint_style),
        ]),
    ];

    let popup = Paragraph::new(content).block(
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(ACCENT_RED))
            .title(Span::styled(
                " Missing Dependencies ",
                Style::default()
                    .fg(ACCENT_RED)
                    .add_modifier(Modifier::BOLD),
            ))
            .style(Style::default().bg(BG_SURFACE)),
    );
    frame.render_widget(popup, area);
}

/// Parse a usage-threshold percentage from user input. Returns `None` when the
/// string does not parse as a finite `f64` in the inclusive range `1.0..=100.0`.
fn parse_percent(s: &str) -> Option<f64> {
    let v: f64 = s.trim().parse().ok()?;
    if !v.is_finite() || !(1.0..=100.0).contains(&v) {
        return None;
    }
    Some(v)
}

/// Parse the idle-completion timeout in whole minutes. Accepts `"0"` and the
/// words `"off"`/`"never"` (all meaning "disabled"), and an empty field falls
/// back to the built-in default rather than silently turning the fallback off.
/// Capped at a week so a typo cannot store a value that never elapses.
fn parse_minutes(s: &str) -> Option<u64> {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return Some(Config::default().idle_complete_seconds / 60);
    }
    if trimmed.eq_ignore_ascii_case("off") || trimmed.eq_ignore_ascii_case("never") {
        return Some(0);
    }
    let v: u64 = trimmed.parse().ok()?;
    if v > 7 * 24 * 60 {
        return None;
    }
    Some(v)
}

/// Parse the usage-staleness window in whole minutes. An empty field falls back
/// to the built-in default; `0` is rejected because a zero window makes every
/// sample stale, and the engine never resumes a paused job on a stale sample.
/// Capped at a day, past which the reading is worthless for pause decisions too.
fn parse_stale_minutes(s: &str) -> Option<u64> {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return Some(Config::default().usage_max_age_seconds / 60);
    }
    let v: u64 = trimmed.parse().ok()?;
    if v == 0 || v > 24 * 60 {
        return None;
    }
    Some(v)
}

/// Checks that a pause/resume percentage pair stays properly separated: resume
/// must be strictly less than pause, otherwise the watcher could thrash between
/// pausing and resuming a session.
fn percent_ordering_ok(pause: f64, resume: f64) -> bool {
    resume < pause
}

/// Maps a committed text-field value to `Some(value)`, or `None` when the
/// field was left empty (meaning "use the default").
fn optional_from_input(s: String) -> Option<String> {
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

/// Open a URL in the default browser.
fn open_url(url: &str) {
    #[cfg(target_os = "macos")]
    let _ = std::process::Command::new("open").arg(url).spawn();

    #[cfg(target_os = "linux")]
    let _ = std::process::Command::new("xdg-open").arg(url).spawn();

    #[cfg(target_os = "windows")]
    let _ = std::process::Command::new("cmd")
        .args(["/c", "start", url])
        .spawn();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::MainTab;

    /// Render the whole frame with the Config tab open and return its text.
    fn config_screen(app: &mut App, w: u16, h: u16) -> String {
        use ratatui::{backend::TestBackend, Terminal};
        let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
        terminal.draw(|f| crate::ui::draw(f, app)).unwrap();
        let buffer = terminal.backend().buffer().clone();
        (0..h)
            .map(|y| {
                (0..w)
                    .map(|x| buffer[(x, y)].symbol().to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// A key event with no modifiers.
    fn key(code: KeyCode) -> crossterm::event::KeyEvent {
        use crossterm::event::{KeyEvent, KeyModifiers};
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    // --- The tab, rather than the popup it replaced ---

    #[test]
    fn o_opens_the_config_tab_from_the_sessions_tab() {
        let mut app = App::new(vec![], None, Config::default());
        app.dispatch_normal_key(key(KeyCode::Char('o')));
        assert_eq!(app.main_tab, MainTab::Config);
        assert_eq!(app.mode, AppMode::Normal, "the Config tab is not a modal");
        assert_eq!(app.config_selected, 0, "and always opens at the top");
    }

    #[test]
    fn the_config_tab_honours_the_global_keys() {
        let mut app = App::new(vec![], None, Config::default());

        app.open_config_tab();
        app.handle_config_tab_event(key(KeyCode::Tab));
        assert_eq!(app.main_tab, MainTab::Sessions, "Tab moves to the next tab");

        app.open_config_tab();
        app.handle_config_tab_event(key(KeyCode::BackTab));
        assert_eq!(app.main_tab, MainTab::Jobs, "Shift+Tab moves back");

        app.open_config_tab();
        app.handle_config_tab_event(key(KeyCode::Esc));
        assert_eq!(app.main_tab, MainTab::Sessions, "Esc backs out to Sessions");

        app.open_config_tab();
        app.handle_config_tab_event(key(KeyCode::Char('?')));
        assert_eq!(app.mode, AppMode::Help);

        app.open_config_tab();
        app.handle_config_tab_event(key(KeyCode::Char('q')));
        assert!(app.should_quit);
    }

    /// Tab used to cycle the session view mode from inside the popup. It now
    /// belongs to the tab strip, so the view row grew arrow keys instead.
    #[test]
    fn the_view_row_cycles_both_ways_with_the_arrow_keys() {
        let _guard = crate::config::test_lock();
        let dir = tempfile::tempdir().unwrap();
        std::env::set_var("CCSM_CONFIG_DIR", dir.path());

        let mut app = App::new(vec![], None, Config::default());
        app.open_config_tab();
        app.config_selected = VIEW_ROW;

        let start = (app.tree_view, app.display_mode);
        app.handle_config_tab_event(key(KeyCode::Right));
        assert_ne!((app.tree_view, app.display_mode), start);
        app.handle_config_tab_event(key(KeyCode::Left));
        assert_eq!((app.tree_view, app.display_mode), start, "Left undoes Right");

        // The same arrows do nothing on a row that has no direction.
        app.config_selected = HIDE_EMPTY_ROW;
        let hide_empty = app.hide_empty;
        app.handle_config_tab_event(key(KeyCode::Right));
        assert_eq!(app.hide_empty, hide_empty);

        std::env::remove_var("CCSM_CONFIG_DIR");
    }

    #[test]
    fn home_and_end_jump_to_the_ends_of_the_list() {
        let mut app = App::new(vec![], None, Config::default());
        app.open_config_tab();
        app.handle_config_tab_event(key(KeyCode::End));
        assert_eq!(app.config_selected, CONFIG_MAX_ROW);
        app.handle_config_tab_event(key(KeyCode::Home));
        assert_eq!(app.config_selected, 0);
    }

    /// The reason the popup became a tab: the explanation of a setting fits
    /// beside it, and it changes as the selection moves.
    #[test]
    fn the_detail_pane_explains_whichever_setting_is_selected() {
        let mut app = App::new(vec![], None, Config::default());
        app.jobs = vec![];
        app.watch_state = None;
        app.open_config_tab();

        app.config_selected = PAUSE_PERCENT_ROW;
        let text = config_screen(&mut app, 120, 40);
        assert!(text.contains("Pause jobs at usage"), "{text}");
        assert!(text.contains("Current: 95.0%"), "{text}");
        assert!(text.contains("greater than the resume"), "{text}");

        app.config_selected = PAUSE_MODE_ROW;
        let text = config_screen(&mut app, 120, 40);
        assert!(text.contains("scrollback survives"), "{text}");
        assert!(!text.contains("greater than the resume"), "{text}");
    }

    /// Long values are truncated in the list but never in the detail pane,
    /// which is the only place the whole thing is readable.
    #[test]
    fn the_detail_pane_shows_a_long_value_in_full() {
        let mut app = App::new(vec![], None, Config::default());
        app.jobs = vec![];
        app.watch_state = None;
        app.open_config_tab();
        app.config.continue_prompt =
            "Pick up exactly where you left off and keep going until it is done.".to_string();
        app.config_selected = CONTINUE_PROMPT_ROW;

        // The pane wraps, so the value spans rows: assert on both ends of it
        // rather than on one contiguous string.
        let text = config_screen(&mut app, 120, 40);
        assert!(text.contains("Current: Pick up exactly where"), "{text}");
        assert!(text.contains("going until it is done."), "{text}");
    }

    /// Every dependency of the missing-deps dialog is a path row, so `s` has to
    /// land on one rather than on the session preferences at the top.
    #[test]
    fn the_missing_deps_dialog_opens_the_config_tab_on_the_binary_paths() {
        let mut app = App::new(vec![], None, Config::default());
        app.mode = AppMode::MissingDeps;
        app.handle_missing_deps_event(key(KeyCode::Char('s')));
        assert_eq!(app.main_tab, MainTab::Config);
        assert_eq!(app.mode, AppMode::Normal);
        assert_eq!(app.config_selected, CLAUDE_PATH_ROW);
    }

    /// Leaving with an unusable binary would drop the user onto a session list
    /// where nothing can launch, so the blocking dialog comes back instead.
    #[test]
    fn leaving_the_config_tab_re_raises_a_still_missing_binary() {
        let mut app = App::new(vec![], None, Config::default());
        app.open_config_tab();
        app.missing_tmux = true;
        app.config.tmux_path = Some("/nonexistent/tmux".to_string());

        app.handle_config_tab_event(key(KeyCode::Esc));
        assert_eq!(app.mode, AppMode::MissingDeps);
        assert_eq!(app.main_tab, MainTab::Config, "and stays on the tab behind it");
    }

    /// Exactly one of claude/agent missing is soft: leaving Config must not
    /// re-raise the blocking dialog when tmux and the other agent are fine.
    #[test]
    fn leaving_config_with_only_claude_missing_is_not_blocking() {
        let mut app = App::new(vec![], None, Config::default());
        app.open_config_tab();
        app.config.claude_path = Some("/nonexistent/ccsm-test-claude".to_string());
        // Point agent and tmux at a binary that exists on every Unix CI image.
        app.config.agent_path = Some("/bin/sh".to_string());
        app.config.tmux_path = Some("/bin/sh".to_string());
        app.handle_config_tab_event(key(KeyCode::Esc));
        assert_eq!(app.mode, AppMode::Normal);
        assert!(app.missing_claude);
        assert!(!app.missing_agent);
        assert!(!app.missing_tmux);
    }

    #[test]
    fn leaving_config_with_both_agents_missing_is_blocking() {
        let mut app = App::new(vec![], None, Config::default());
        app.open_config_tab();
        app.config.claude_path = Some("/nonexistent/ccsm-test-claude".to_string());
        app.config.agent_path = Some("/nonexistent/ccsm-test-agent".to_string());
        app.config.tmux_path = Some("/bin/sh".to_string());
        app.handle_config_tab_event(key(KeyCode::Esc));
        assert_eq!(app.mode, AppMode::MissingDeps);
    }

    #[test]
    fn the_settings_list_includes_the_agent_binary_row() {
        let mut app = App::new(vec![], None, Config::default());
        app.jobs = vec![];
        app.watch_state = None;
        app.open_config_tab();
        let text = config_screen(&mut app, 120, 40);
        assert!(text.contains("Agent binary"), "{text}");
        assert!(text.contains("Claude binary"), "{text}");
    }

    #[test]
    fn the_status_bar_names_what_the_selected_row_actually_does() {
        let mut app = App::new(vec![], None, Config::default());
        app.jobs = vec![];
        app.watch_state = None;
        app.open_config_tab();

        app.config_selected = CLAUDE_PATH_ROW;
        assert!(config_screen(&mut app, 120, 40).contains("i type path"));

        app.config_selected = URL_ROW;
        let text = config_screen(&mut app, 120, 40);
        assert!(text.contains("Enter open"), "{text}");
        assert!(!text.contains("toggle"), "{text}");

        app.config_selected = HIDE_EMPTY_ROW;
        assert!(config_screen(&mut app, 120, 40).contains("toggle"));
    }

    #[test]
    fn parse_percent_accepts_in_range_values() {
        assert_eq!(parse_percent("95"), Some(95.0));
        assert_eq!(parse_percent("95.0"), Some(95.0));
        assert_eq!(parse_percent("1"), Some(1.0));
        assert_eq!(parse_percent("100"), Some(100.0));
    }

    #[test]
    fn parse_percent_rejects_invalid_values() {
        assert_eq!(parse_percent(""), None);
        assert_eq!(parse_percent("abc"), None);
        assert_eq!(parse_percent("0"), None);
        assert_eq!(parse_percent("101"), None);
        assert_eq!(parse_percent("-5"), None);
        assert_eq!(parse_percent("NaN"), None);
        assert_eq!(parse_percent("inf"), None);
    }

    #[test]
    fn percent_ordering_rejects_thrashing_thresholds() {
        // resume must be strictly less than pause.
        assert!(percent_ordering_ok(95.0, 50.0));
        assert!(!percent_ordering_ok(95.0, 95.0));
        assert!(!percent_ordering_ok(50.0, 95.0));
    }

    #[test]
    fn the_list_shows_the_continue_prompt_and_the_about_rows_together() {
        use ratatui::{backend::TestBackend, Terminal};
        let mut app = App::new(vec![], None, Config::default());
        app.main_tab = MainTab::Config;
        app.config.continue_prompt = "Pick it back up.".to_string();

        let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
        terminal.draw(|f| crate::ui::draw(f, &mut app)).unwrap();
        let buffer = terminal.backend().buffer().clone();
        let text: String = (0..40)
            .map(|y| {
                (0..120)
                    .map(|x| buffer[(x, y)].symbol().to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");

        assert!(text.contains("Continue prompt: Pick it back up."), "{text}");
        // The About block must still be reachable below the new row.
        assert!(text.contains("https://github.com/faulker/ccsm"), "{text}");
    }

    #[test]
    fn the_list_shows_the_idle_completion_row_in_both_states() {
        use ratatui::{backend::TestBackend, Terminal};
        let render = |seconds: u64| -> String {
            let mut app = App::new(vec![], None, Config::default());
            app.main_tab = MainTab::Config;
            app.config.idle_complete_seconds = seconds;
            let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
            terminal.draw(|f| crate::ui::draw(f, &mut app)).unwrap();
            let buffer = terminal.backend().buffer().clone();
            (0..40)
                .map(|y| {
                    (0..120)
                        .map(|x| buffer[(x, y)].symbol().to_string())
                        .collect::<String>()
                })
                .collect::<Vec<_>>()
                .join("\n")
        };

        let on = render(900);
        assert!(on.contains("Idle completion: after 15 min idle"), "{on}");
        // Adding the row must not push the About block out of the list.
        assert!(on.contains("https://github.com/faulker/ccsm"), "{on}");

        // "0" has to read as "off", not as "finish immediately".
        let off = render(0);
        assert!(off.contains("Idle completion: off (marker only)"), "{off}");
    }

    #[test]
    fn idle_completion_row_commits_minutes_as_seconds() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let _guard = crate::config::test_lock();
        let dir = tempfile::tempdir().unwrap();
        std::env::set_var("CCSM_CONFIG_DIR", dir.path());

        let mut app = App::new(vec![], None, Config::default());
        app.main_tab = MainTab::Config;
        app.config_selected = IDLE_COMPLETE_ROW;
        app.handle_config_tab_event(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(app.config_editing);
        // The field opens on the current value in minutes, not seconds.
        assert_eq!(app.config_path_input.value(), "15");

        app.config_path_input = tui_input::Input::from("30".to_string());
        app.handle_config_tab_event(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(!app.config_editing);
        assert_eq!(app.config.idle_complete_seconds, 1800);

        // A value that does not parse is rejected and leaves the setting alone.
        app.handle_config_tab_event(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        app.config_path_input = tui_input::Input::from("soon".to_string());
        app.handle_config_tab_event(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(app.config.idle_complete_seconds, 1800);
        assert!(app.status_error.is_some());

        std::env::remove_var("CCSM_CONFIG_DIR");
    }

    #[test]
    fn the_list_shows_the_usage_staleness_row_in_minutes() {
        use ratatui::{backend::TestBackend, Terminal};
        let mut app = App::new(vec![], None, Config::default());
        app.main_tab = MainTab::Config;

        let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
        terminal.draw(|f| crate::ui::draw(f, &mut app)).unwrap();
        let buffer = terminal.backend().buffer().clone();
        let text: String = (0..40)
            .map(|y| {
                (0..120)
                    .map(|x| buffer[(x, y)].symbol().to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");

        assert!(text.contains("Usage sample stale after: 5 min"), "{text}");
        // The row is stored in seconds but never shown that way.
        assert!(!text.contains("300"), "{text}");
        // Adding the row must not push the About block out of the list.
        assert!(text.contains("https://github.com/faulker/ccsm"), "{text}");
    }

    #[test]
    fn usage_staleness_row_commits_minutes_as_seconds() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let _guard = crate::config::test_lock();
        let dir = tempfile::tempdir().unwrap();
        std::env::set_var("CCSM_CONFIG_DIR", dir.path());

        let mut app = App::new(vec![], None, Config::default());
        app.main_tab = MainTab::Config;
        app.config_selected = USAGE_STALE_ROW;
        app.handle_config_tab_event(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(app.config_editing);
        // The field opens on the current value in minutes, not seconds.
        assert_eq!(app.config_path_input.value(), "5");

        app.config_path_input = tui_input::Input::from("10".to_string());
        app.handle_config_tab_event(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(!app.config_editing);
        assert_eq!(app.config.usage_max_age_seconds, 600);

        // "0" would make every sample stale, so it is rejected outright.
        app.handle_config_tab_event(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        app.config_path_input = tui_input::Input::from("0".to_string());
        app.handle_config_tab_event(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(app.config.usage_max_age_seconds, 600);
        assert!(app.status_error.is_some());

        std::env::remove_var("CCSM_CONFIG_DIR");
    }

    #[test]
    fn the_rows_below_the_new_staleness_row_still_do_their_own_jobs() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let _guard = crate::config::test_lock();
        let dir = tempfile::tempdir().unwrap();
        std::env::set_var("CCSM_CONFIG_DIR", dir.path());

        let mut app = App::new(vec![], None, Config::default());
        app.main_tab = MainTab::Config;

        app.config_selected = PAUSE_MODE_ROW;
        app.handle_config_tab_event(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(app.config.pause_mode, PauseMode::Hard);
        assert!(!app.config_editing);

        app.config_selected = WATCH_AUTOSTART_ROW;
        app.handle_config_tab_event(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(!app.config.watch_autostart);
        assert!(!app.config_editing);

        std::env::remove_var("CCSM_CONFIG_DIR");
    }

    #[test]
    fn parse_stale_minutes_rejects_zero_and_junk() {
        assert_eq!(parse_stale_minutes("10"), Some(10));
        assert_eq!(parse_stale_minutes(" 1 "), Some(1));
        // Empty falls back to the built-in default.
        assert_eq!(parse_stale_minutes(""), Some(5));
        assert_eq!(parse_stale_minutes("0"), None);
        assert_eq!(parse_stale_minutes("off"), None);
        assert_eq!(parse_stale_minutes("-5"), None);
        assert_eq!(parse_stale_minutes("2.5"), None);
        assert_eq!(parse_stale_minutes(&(24 * 60).to_string()), Some(1440));
        assert_eq!(parse_stale_minutes(&(24 * 60 + 1).to_string()), None);
    }

    #[test]
    fn parse_minutes_accepts_numbers_and_the_off_words() {
        assert_eq!(parse_minutes("30"), Some(30));
        assert_eq!(parse_minutes(" 0 "), Some(0));
        assert_eq!(parse_minutes("off"), Some(0));
        assert_eq!(parse_minutes("Never"), Some(0));
        // Empty falls back to the built-in default rather than disabling it.
        assert_eq!(parse_minutes(""), Some(15));
    }

    #[test]
    fn parse_minutes_rejects_junk_and_absurd_values() {
        assert_eq!(parse_minutes("abc"), None);
        assert_eq!(parse_minutes("-5"), None);
        assert_eq!(parse_minutes("1.5"), None);
        assert_eq!(parse_minutes(&(7 * 24 * 60 + 1).to_string()), None);
        assert_eq!(parse_minutes(&(7 * 24 * 60).to_string()), Some(10_080));
    }

    #[test]
    fn optional_from_input_maps_empty_string_to_none() {
        assert_eq!(optional_from_input(String::new()), None);
        assert_eq!(
            optional_from_input("/usr/local/bin/claude".to_string()),
            Some("/usr/local/bin/claude".to_string())
        );
    }

    #[test]
    fn a_text_field_types_shifted_characters() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let _guard = crate::config::test_lock();
        let dir = tempfile::tempdir().unwrap();
        std::env::set_var("CCSM_CONFIG_DIR", dir.path());

        let mut app = App::new(vec![], None, Config::default());
        app.main_tab = MainTab::Config;
        app.config_selected = CONTINUE_PROMPT_ROW;
        app.handle_config_tab_event(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(app.config_editing);
        app.config_path_input = tui_input::Input::default();

        // As the enhanced keyboard protocol reports them: the base key plus SHIFT.
        for c in ['2', 'h', 'i', '1'] {
            app.handle_config_tab_event(KeyEvent::new(KeyCode::Char(c), KeyModifiers::SHIFT));
        }

        assert_eq!(app.config_path_input.value(), "@HI!");

        std::env::remove_var("CCSM_CONFIG_DIR");
    }

    #[test]
    fn the_list_shows_the_usage_history_row() {
        use ratatui::{backend::TestBackend, Terminal};
        let mut app = App::new(vec![], None, Config::default());
        app.main_tab = MainTab::Config;
        let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
        terminal.draw(|f| crate::ui::draw(f, &mut app)).unwrap();
        let buffer = terminal.backend().buffer().clone();
        let text: String = (0..40)
            .map(|y| {
                (0..120)
                    .map(|x| buffer[(x, y)].symbol().to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");

        assert!(text.contains("Usage history file"), "{text}");
        // The row that replaced it must be gone: there is no binary to point at.
        assert!(!text.contains("claude-usage binary"), "{text}");
    }

    #[test]
    fn usage_history_row_commits_a_typed_path() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let _guard = crate::config::test_lock();
        let dir = tempfile::tempdir().unwrap();
        std::env::set_var("CCSM_CONFIG_DIR", dir.path());

        let mut app = App::new(vec![], None, Config::default());
        app.main_tab = MainTab::Config;
        app.config_selected = USAGE_PATH_ROW;
        app.handle_config_tab_event(KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE));
        assert!(app.config_editing, "usage history row must be typeable");

        app.config_path_input = tui_input::Input::from("/tmp/plan-usage-history.json".to_string());
        app.handle_config_tab_event(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));

        assert_eq!(
            app.config.usage_history_path.as_deref(),
            Some("/tmp/plan-usage-history.json")
        );
        assert_eq!(
            app.config.usage_history_override(),
            Some("/tmp/plan-usage-history.json")
        );

        std::env::remove_var("CCSM_CONFIG_DIR");
    }
}
