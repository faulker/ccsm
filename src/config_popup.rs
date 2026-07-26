use crate::app::{App, AppMode, PickerTarget};
use crate::config::{Config, PauseMode};
use crate::keys::normalize_key;
use crossterm::event::KeyCode;

/// Maximum row index in the config popup (0-based).
pub const CONFIG_MAX_ROW: usize = 11;

/// Rows 3..=5 are binary paths: they open the file picker, and can also be typed by hand.
const PATH_ROWS: std::ops::RangeInclusive<usize> = 3..=5;
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, Paragraph},
    Frame,
};

use crate::theme::{
    ACCENT_BLUE, ACCENT_GREEN, ACCENT_MAUVE, ACCENT_PEACH, ACCENT_RED, BG_SURFACE, FG_SUBTEXT,
    FG_TEXT, HIGHLIGHT_BG,
};
use crate::ui::util::input_spans;

impl App {
    /// Handle a key event while the config popup is open.
    pub(crate) fn handle_config_event(&mut self, key: crossterm::event::KeyEvent) {
        // If editing a path field, delegate to text input
        if self.config_editing {
            use crossterm::event::Event;
            use tui_input::backend::crossterm::EventHandler;

            match key.code {
                KeyCode::Enter => {
                    let value = self.config_path_input.value().trim().to_string();
                    let mut commit_ok = true;
                    match self.config_selected {
                        3 => {
                            self.config.claude_path = optional_from_input(value);
                        }
                        4 => {
                            self.config.tmux_path = optional_from_input(value);
                        }
                        5 => {
                            self.config.usage_path = optional_from_input(value);
                        }
                        6 => match parse_percent(&value) {
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
                        7 => match parse_percent(&value) {
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
                        _ => {}
                    }
                    if commit_ok {
                        if let Err(e) = self.config.save() {
                            self.status_error = Some(format!("Failed to save config: {e}"));
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

        match key.code {
            KeyCode::Esc => {
                // Re-check binaries if they were previously missing
                if self.missing_claude || self.missing_tmux {
                    self.missing_claude = !Config::is_bin_available(self.config.claude_bin());
                    self.missing_tmux = !Config::is_bin_available(self.config.tmux_bin());
                    if self.missing_claude || self.missing_tmux {
                        self.mode = AppMode::MissingDeps;
                        return;
                    }
                }
                self.mode = AppMode::Normal;
            }
            KeyCode::Char('j') | KeyCode::Down => {
                if self.config_selected < CONFIG_MAX_ROW {
                    self.config_selected += 1;
                }
            }
            KeyCode::Char('k') | KeyCode::Up => {
                if self.config_selected > 0 {
                    self.config_selected -= 1;
                }
            }
            KeyCode::Char(' ') | KeyCode::Enter => {
                match self.config_selected {
                    0 => {
                        self.hide_empty = !self.hide_empty;
                        self.recompute_filter();
                        self.preview_scroll = u16::MAX;
                        self.save_config();
                    }
                    1 => {
                        self.group_chains = !self.group_chains;
                        self.preview_cache.clear();
                        self.recompute_filter();
                        self.preview_scroll = u16::MAX;
                        self.save_config();
                    }
                    2 => {
                        self.cycle_view_forward();
                        self.selected = 0;
                        self.preview_scroll = u16::MAX;
                        self.save_config();
                    }
                    // Binary paths open the file picker; `i` (below) types one by hand.
                    3 => self.browse_config_path(PickerTarget::ConfigClaude),
                    4 => self.browse_config_path(PickerTarget::ConfigTmux),
                    5 => self.browse_config_path(PickerTarget::ConfigUsage),
                    6 => {
                        self.config_editing = true;
                        let current = format!("{:.1}", self.config.usage_pause_percent);
                        self.config_path_input = tui_input::Input::from(current);
                    }
                    7 => {
                        self.config_editing = true;
                        let current = format!("{:.1}", self.config.usage_resume_percent);
                        self.config_path_input = tui_input::Input::from(current);
                    }
                    8 => {
                        self.config.pause_mode = match self.config.pause_mode {
                            PauseMode::Soft => PauseMode::Hard,
                            PauseMode::Hard => PauseMode::Soft,
                        };
                        self.save_config();
                    }
                    9 => {
                        self.config.watch_autostart = !self.config.watch_autostart;
                        self.save_config();
                    }
                    11 => {
                        open_url("https://github.com/faulker/ccsm");
                    }
                    _ => {}
                }
            }
            // Manual entry for the browsable path rows, so a path that isn't
            // convenient to browse to (or doesn't exist yet) can still be typed.
            KeyCode::Char('i') if PATH_ROWS.contains(&self.config_selected) => {
                self.config_editing = true;
                let current = match self.config_selected {
                    3 => self.config.claude_path.clone(),
                    4 => self.config.tmux_path.clone(),
                    _ => self.config.usage_path.clone(),
                }
                .unwrap_or_default();
                self.config_path_input = tui_input::Input::from(current);
            }
            KeyCode::Tab => {
                self.cycle_view_forward();
                self.selected = 0;
                self.preview_scroll = u16::MAX;
                self.save_config();
            }
            KeyCode::BackTab => {
                self.cycle_view_backward();
                self.selected = 0;
                self.preview_scroll = u16::MAX;
                self.save_config();
            }
            _ => {}
        }
    }

    /// Open the file picker for one of the binary-path settings, starting from
    /// its current value.
    fn browse_config_path(&mut self, target: PickerTarget) {
        let current = match target {
            PickerTarget::ConfigClaude => self.config.claude_path.clone(),
            PickerTarget::ConfigTmux => self.config.tmux_path.clone(),
            _ => self.config.usage_path.clone(),
        }
        .unwrap_or_default();
        self.open_path_picker(target, &current);
    }

    /// Handle a key event while the missing-deps dialog is shown.
    pub(crate) fn handle_missing_deps_event(&mut self, key: crossterm::event::KeyEvent) {
        match key.code {
            KeyCode::Char('s') => {
                self.mode = AppMode::Config;
            }
            KeyCode::Char('q') | KeyCode::Esc => {
                self.should_quit = true;
            }
            _ => {}
        }
    }
}

/// Render the config popup: settings grouped under labelled sections so it is
/// obvious which ones drive the jobs manager and which are session-list
/// preferences, followed by the About block.
pub fn draw_config_popup(frame: &mut Frame, app: &App) {
    let area = centered_rect(60, 80, frame.area());
    let area = Rect { height: 26.min(area.height), ..area };
    frame.render_widget(Clear, area);

    let selected = app.config_selected;
    let key_style = Style::default().fg(ACCENT_PEACH).add_modifier(Modifier::BOLD);
    let hint_style = Style::default().fg(FG_SUBTEXT);
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
        0,
        format!("[{}] Hide empty projects", if app.hide_empty { "x" } else { " " }),
    ));
    content.push(row(
        1,
        format!("[{}] Group session chains", if app.group_chains { "x" } else { " " }),
    ));
    content.push(row(2, format!("View: {}", view_label)));
    content.push(path_row(
        3,
        "Claude binary",
        app.config.claude_path.as_ref(),
        "claude (default)".to_string(),
    ));
    content.push(path_row(
        4,
        "Tmux binary",
        app.config.tmux_path.as_ref(),
        "tmux (default)".to_string(),
    ));

    content.push(Line::from(""));
    content.push(section("Jobs manager (scheduler & watcher)"));
    content.push(path_row(
        5,
        "claude-usage binary",
        app.config.usage_path.as_ref(),
        format!("{} (default)", app.config.usage_bin()),
    ));
    content.push(percent_row(6, "Pause jobs at usage", app.config.usage_pause_percent));
    content.push(percent_row(7, "Resume jobs at usage", app.config.usage_resume_percent));
    content.push(row(8, format!("Pause mode: {}", app.config.pause_mode.label())));
    content.push(row(
        9,
        format!(
            "[{}] Auto-start watcher",
            if app.config.watch_autostart { "x" } else { " " }
        ),
    ));

    content.push(Line::from(""));
    content.push(section("About"));
    content.push({
        let style = if selected == 10 {
            Style::default().fg(ACCENT_MAUVE).bg(HIGHLIGHT_BG)
        } else {
            Style::default().fg(ACCENT_MAUVE)
        };
        Line::from(Span::styled(format!("{}Developed by Winter Faulk", marker(10)), style))
    });
    content.push({
        let marker_style = if selected == 11 {
            Style::default().fg(ACCENT_BLUE).bg(HIGHLIGHT_BG)
        } else {
            Style::default().fg(ACCENT_BLUE)
        };
        let url_style = if selected == 11 {
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
            Span::styled(marker(11), marker_style),
            Span::styled("https://github.com/faulker/ccsm", url_style),
        ])
    });

    content.push(Line::from(""));
    if app.config_editing {
        content.push(Line::from(vec![
            Span::styled("  Enter", key_style),
            Span::styled(" save  ", hint_style),
            Span::styled("←/→", key_style),
            Span::styled(" move cursor  ", hint_style),
            Span::styled("Esc", key_style),
            Span::styled(" cancel  ", hint_style),
            Span::styled("(empty = default)", hint_style),
        ]));
    } else if PATH_ROWS.contains(&selected) {
        content.push(Line::from(vec![
            Span::styled("  Enter", key_style),
            Span::styled(" browse  ", hint_style),
            Span::styled("i", key_style),
            Span::styled(" type path  ", hint_style),
            Span::styled("j/k", key_style),
            Span::styled(" navigate", hint_style),
        ]));
    } else if selected == 11 {
        content.push(Line::from(vec![
            Span::styled("  Enter", key_style),
            Span::styled(" open in browser  ", hint_style),
            Span::styled("j/k", key_style),
            Span::styled(" navigate", hint_style),
        ]));
    } else {
        content.push(Line::from(vec![
            Span::styled("  Space/Enter", key_style),
            Span::styled(" toggle/edit  ", hint_style),
            Span::styled("j/k", key_style),
            Span::styled(" navigate", hint_style),
        ]));
    }

    let popup = Paragraph::new(content).block(
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(ACCENT_PEACH))
            .title(Span::styled(
                " Config (Esc to close) ",
                Style::default()
                    .fg(ACCENT_PEACH)
                    .add_modifier(Modifier::BOLD),
            ))
            .style(Style::default().bg(BG_SURFACE)),
    );
    frame.render_widget(popup, area);
}

/// Render the missing-dependencies dialog.
pub fn draw_missing_deps_popup(frame: &mut Frame, app: &App) {
    let area = centered_rect(50, 30, frame.area());
    let area = Rect { height: 9.min(area.height), ..area };
    frame.render_widget(Clear, area);

    let key_style = Style::default().fg(ACCENT_PEACH).add_modifier(Modifier::BOLD);
    let hint_style = Style::default().fg(FG_SUBTEXT);

    let claude_line = if app.missing_claude {
        Line::from(Span::styled("  ✗ claude not found", Style::default().fg(ACCENT_RED)))
    } else {
        Line::from(Span::styled("  ✓ claude found", Style::default().fg(ACCENT_GREEN)))
    };

    let tmux_line = if app.missing_tmux {
        Line::from(Span::styled("  ✗ tmux not found", Style::default().fg(ACCENT_RED)))
    } else {
        Line::from(Span::styled("  ✓ tmux found", Style::default().fg(ACCENT_GREEN)))
    };

    let content = vec![
        Line::from(""),
        claude_line,
        tmux_line,
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

/// Compute a centered `Rect` that is `percent_x`% wide and `percent_y`% tall within `area`.
fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
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

#[cfg(test)]
mod tests {
    use super::*;

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
    fn optional_from_input_maps_empty_string_to_none() {
        assert_eq!(optional_from_input(String::new()), None);
        assert_eq!(
            optional_from_input("/usr/local/bin/claude-usage".to_string()),
            Some("/usr/local/bin/claude-usage".to_string())
        );
    }
}
