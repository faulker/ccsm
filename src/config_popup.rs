use crate::app::{App, AppMode};
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
const HIGHLIGHT_BG: Color = Color::Rgb(69, 71, 90);

impl App {
    /// Handle a key event while the config popup is open.
    pub(crate) fn handle_config_event(&mut self, key: crossterm::event::KeyEvent) {
        match key.code {
            KeyCode::Esc => {
                self.mode = AppMode::Normal;
            }
            KeyCode::Char('j') | KeyCode::Down => {
                if self.config_selected < 2 {
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
}

/// Render the config popup with toggleable options for hide-empty, group-chains, and view mode.
pub fn draw_config_popup(frame: &mut Frame, app: &App) {
    let area = centered_rect(40, 20, frame.area());
    let area = Rect { height: 7.min(area.height), ..area };
    frame.render_widget(Clear, area);

    let selected = app.config_selected;
    let key_style = Style::default().fg(ACCENT_PEACH).add_modifier(Modifier::BOLD);
    let hint_style = Style::default().fg(FG_SUBTEXT);

    let view_label = format!(
        "{} · {}",
        if app.tree_view { "Tree" } else { "Flat" },
        app.display_mode.label()
    );

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
    ];

    let mut content: Vec<Line> = Vec::new();
    content.push(Line::from(""));
    content.extend(items);
    content.push(Line::from(""));
    content.push(Line::from(vec![
        Span::styled("  Space/Enter", key_style),
        Span::styled(" toggle  ", hint_style),
        Span::styled("j/k", key_style),
        Span::styled(" navigate", hint_style),
    ]));

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
