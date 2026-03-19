use crate::app::{App, AppMode};
use crate::config::Config;
use crossterm::event::KeyCode;
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, Paragraph},
    Frame,
};

// Re-use the same palette constants as ui.rs
const BG_SURFACE: Color = Color::Rgb(30, 30, 46);
const FG_TEXT: Color = Color::Rgb(205, 214, 244);
const FG_SUBTEXT: Color = Color::Rgb(147, 153, 178);
const ACCENT_BLUE: Color = Color::Rgb(137, 180, 250);
const ACCENT_PEACH: Color = Color::Rgb(250, 179, 135);
const ACCENT_GREEN: Color = Color::Rgb(166, 218, 149);
const ACCENT_RED: Color = Color::Rgb(243, 139, 168);
const HIGHLIGHT_BG: Color = Color::Rgb(69, 71, 90);

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
                    match self.config_selected {
                        3 => {
                            self.config.claude_path = if value.is_empty() { None } else { Some(value) };
                        }
                        4 => {
                            self.config.tmux_path = if value.is_empty() { None } else { Some(value) };
                        }
                        _ => {}
                    }
                    let _ = self.config.save();
                    self.config_editing = false;
                    self.config_path_input = tui_input::Input::default();
                }
                KeyCode::Esc => {
                    self.config_editing = false;
                    self.config_path_input = tui_input::Input::default();
                }
                _ => {
                    self.config_path_input.handle_event(&Event::Key(key));
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
                if self.config_selected < 4 {
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
                    3 => {
                        self.config_editing = true;
                        let current = self.config.claude_path.clone().unwrap_or_default();
                        self.config_path_input = tui_input::Input::from(current);
                    }
                    4 => {
                        self.config_editing = true;
                        let current = self.config.tmux_path.clone().unwrap_or_default();
                        self.config_path_input = tui_input::Input::from(current);
                    }
                    _ => {}
                }
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

/// Render the config popup with toggleable options and editable path fields.
pub fn draw_config_popup(frame: &mut Frame, app: &App) {
    let area = centered_rect(50, 30, frame.area());
    let area = Rect { height: 11.min(area.height), ..area };
    frame.render_widget(Clear, area);

    let selected = app.config_selected;
    let key_style = Style::default().fg(ACCENT_PEACH).add_modifier(Modifier::BOLD);
    let hint_style = Style::default().fg(FG_SUBTEXT);

    let view_label = format!(
        "{} · {}",
        if app.tree_view { "Tree" } else { "Flat" },
        app.display_mode.label()
    );

    let claude_display = if app.config_editing && selected == 3 {
        format!("{}|", app.config_path_input.value())
    } else {
        match &app.config.claude_path {
            Some(p) => p.clone(),
            None => "claude (default)".to_string(),
        }
    };

    let tmux_display = if app.config_editing && selected == 4 {
        format!("{}|", app.config_path_input.value())
    } else {
        match &app.config.tmux_path {
            Some(p) => p.clone(),
            None => "tmux (default)".to_string(),
        }
    };

    let items: Vec<Line> = vec![
        {
            let check = if app.hide_empty { "x" } else { " " };
            let marker = if selected == 0 { "▶ " } else { "  " };
            let style = if selected == 0 {
                Style::default().fg(ACCENT_BLUE).bg(HIGHLIGHT_BG)
            } else {
                Style::default().fg(FG_TEXT)
            };
            Line::from(Span::styled(format!("{}[{}] Hide empty projects", marker, check), style))
        },
        {
            let check = if app.group_chains { "x" } else { " " };
            let marker = if selected == 1 { "▶ " } else { "  " };
            let style = if selected == 1 {
                Style::default().fg(ACCENT_BLUE).bg(HIGHLIGHT_BG)
            } else {
                Style::default().fg(FG_TEXT)
            };
            Line::from(Span::styled(format!("{}[{}] Group session chains", marker, check), style))
        },
        {
            let marker = if selected == 2 { "▶ " } else { "  " };
            let style = if selected == 2 {
                Style::default().fg(ACCENT_BLUE).bg(HIGHLIGHT_BG)
            } else {
                Style::default().fg(FG_TEXT)
            };
            Line::from(Span::styled(format!("{}View: {}", marker, view_label), style))
        },
        {
            let marker = if selected == 3 { "▶ " } else { "  " };
            let style = if selected == 3 {
                Style::default().fg(ACCENT_BLUE).bg(HIGHLIGHT_BG)
            } else {
                Style::default().fg(FG_TEXT)
            };
            Line::from(Span::styled(format!("{}Claude: {}", marker, claude_display), style))
        },
        {
            let marker = if selected == 4 { "▶ " } else { "  " };
            let style = if selected == 4 {
                Style::default().fg(ACCENT_BLUE).bg(HIGHLIGHT_BG)
            } else {
                Style::default().fg(FG_TEXT)
            };
            Line::from(Span::styled(format!("{}Tmux: {}", marker, tmux_display), style))
        },
    ];

    let mut content: Vec<Line> = Vec::new();
    content.push(Line::from(""));
    content.extend(items);
    content.push(Line::from(""));
    if app.config_editing {
        content.push(Line::from(vec![
            Span::styled("  Enter", key_style),
            Span::styled(" save  ", hint_style),
            Span::styled("Esc", key_style),
            Span::styled(" cancel  ", hint_style),
            Span::styled("(empty = default)", hint_style),
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
