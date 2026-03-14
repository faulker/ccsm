use crate::app::{App, AppMode, TreeRow};
use crate::data::PreviewMessage;
use crate::update::UpdateStatus;
use chrono::{Local, TimeZone};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, BorderType, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap},
    Frame,
};

// Catppuccin Mocha-inspired palette
const BG_SURFACE: Color = Color::Rgb(30, 30, 46);
const FG_TEXT: Color = Color::Rgb(205, 214, 244);
const FG_SUBTEXT: Color = Color::Rgb(147, 153, 178);
const FG_OVERLAY: Color = Color::Rgb(88, 91, 112);
const ACCENT_BLUE: Color = Color::Rgb(137, 180, 250);
const ACCENT_GREEN: Color = Color::Rgb(166, 218, 149);
const ACCENT_MAUVE: Color = Color::Rgb(203, 166, 247);
const ACCENT_PEACH: Color = Color::Rgb(250, 179, 135);
const ACCENT_TEAL: Color = Color::Rgb(148, 226, 213);
const HIGHLIGHT_BG: Color = Color::Rgb(69, 71, 90);

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
                            .fg(ACCENT_TEAL)
                            .add_modifier(Modifier::BOLD),
                    )]);
                    ListItem::new(line)
                }
                TreeRow::Session { session_index } => {
                    let s = &app.sessions[*session_index];
                    let date = format_relative_date(s.last_timestamp);
                    let mut spans = vec![
                        Span::raw("  "),
                        Span::styled(date, Style::default().fg(FG_SUBTEXT)),
                        Span::styled(
                            format!("  {} msg", s.entry_count),
                            Style::default()
                                .fg(FG_OVERLAY)
                                .add_modifier(Modifier::DIM),
                        ),
                    ];
                    if let Some(name) = &s.name {
                        spans.push(Span::styled(
                            format!("  {}", name),
                            Style::default().fg(ACCENT_PEACH),
                        ));
                    }
                    ListItem::new(Line::from(spans))
                }
            })
            .collect()
    } else {
        app.filtered_indices
            .iter()
            .map(|&i| &app.sessions[i])
            .map(|s| {
                let name = app.display_name(s);
                let date = format_relative_date(s.last_timestamp);
                let line = Line::from(vec![
                    Span::styled(
                        truncate(&name, 28),
                        Style::default().fg(FG_TEXT),
                    ),
                    Span::raw("  "),
                    Span::styled(date, Style::default().fg(FG_SUBTEXT)),
                    Span::styled(
                        format!("  {} msg", s.entry_count),
                        Style::default()
                            .fg(FG_OVERLAY)
                            .add_modifier(Modifier::DIM),
                    ),
                ]);
                ListItem::new(line)
            })
            .collect()
    };

    let title_style = Style::default().fg(ACCENT_BLUE).add_modifier(Modifier::BOLD);
    let mode_style = Style::default().fg(ACCENT_GREEN).add_modifier(Modifier::BOLD);
    let view_label = if app.tree_view { "[tree]" } else { "[flat]" };

    let mut title_spans = vec![
        Span::styled(" Sessions ", title_style),
        Span::styled(view_label, title_style),
    ];
    title_spans.push(Span::styled(" ", title_style));
    title_spans.push(Span::styled(app.display_mode.label(), mode_style));
    if !app.hide_empty {
        title_spans.push(Span::styled(" [showing empty]", title_style));
    }
    if let Some(p) = &app.filter_path {
        title_spans.push(Span::styled(format!(" ({})", p), title_style));
    }
    title_spans.push(Span::styled(" ", title_style));

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
    let (meta, preview_slice) = app.current_preview();
    let meta = meta.clone();
    let preview = preview_slice.to_vec();
    let preview_text = build_preview_text(&preview);

    let right_area = main_chunks[1];

    let right_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(vec![Constraint::Length(3), Constraint::Min(3)])
        .split(right_area);

    // Session info bar (always visible)
    let mut spans: Vec<Span> = Vec::new();
    if let Some(id) = &meta.session_id {
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
    let content_height = estimate_wrapped_height(&preview_text, inner_width);
    let max_scroll = (content_height as u16).saturating_sub(inner_height);
    if app.preview_scroll > max_scroll {
        app.preview_scroll = max_scroll;
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
        .wrap(Wrap { trim: false })
        .scroll((app.preview_scroll, 0));

    frame.render_widget(preview_widget, preview_area);

    // Help / search bar
    let bar_style = Style::default().bg(HIGHLIGHT_BG);
    let bottom_bar = if app.filter_active {
        Paragraph::new(Line::from(vec![
            Span::styled(
                " /",
                Style::default()
                    .fg(ACCENT_PEACH)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(&app.filter_text, Style::default().fg(FG_TEXT)),
            Span::styled("█", Style::default().fg(ACCENT_BLUE)),
        ]))
        .style(bar_style)
    } else if !app.filter_text.is_empty() {
        Paragraph::new(Line::from(vec![
            Span::styled(" filter: ", Style::default().fg(FG_SUBTEXT)),
            Span::styled(&app.filter_text, Style::default().fg(FG_TEXT)),
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
        .style(bar_style)
    } else {
        let key_style = Style::default()
            .fg(ACCENT_PEACH)
            .add_modifier(Modifier::BOLD);
        let hint_style = Style::default().fg(FG_SUBTEXT);
        let shift_key_style = if app.shift_active {
            Style::default().fg(FG_TEXT).add_modifier(Modifier::BOLD)
        } else {
            key_style
        };
        let shift_hint_style = if app.shift_active {
            Style::default().fg(FG_TEXT).add_modifier(Modifier::BOLD)
        } else {
            hint_style
        };

        let mut spans = Vec::new();

        // Show post-update status in help bar
        match &app.update_status {
            UpdateStatus::Downloading => {
                spans.push(Span::styled(
                    " Updating... ",
                    Style::default().fg(ACCENT_PEACH).add_modifier(Modifier::BOLD),
                ));
            }
            UpdateStatus::Done(v) => {
                spans.push(Span::styled(
                    format!(" Updated to {} (restart to apply) ", v),
                    Style::default().fg(ACCENT_GREEN).add_modifier(Modifier::BOLD),
                ));
            }
            UpdateStatus::Failed(msg) => {
                spans.push(Span::styled(
                    format!(" Update failed: {} ", msg),
                    Style::default().fg(Color::Rgb(243, 139, 168)).add_modifier(Modifier::BOLD),
                ));
            }
            _ => {}
        }

        spans.extend_from_slice(&[
            Span::styled(" ↑↓/jk", key_style),
            Span::styled(" navigate  ", hint_style),
            Span::styled("Enter", key_style),
            Span::styled(" open  ", hint_style),
            Span::styled("J/K", shift_key_style),
            Span::styled(" scroll  ", shift_hint_style),
            Span::styled("/", key_style),
            Span::styled(" search  ", hint_style),
            Span::styled("Tab", shift_key_style),
            Span::styled(" view  ", shift_hint_style),
            Span::styled("e", key_style),
            Span::styled(" show empty  ", hint_style),
            Span::styled("r", key_style),
            Span::styled(" rename  ", hint_style),
            Span::styled("n", key_style),
            Span::styled(" new  ", hint_style),
            Span::styled("N", shift_key_style),
            Span::styled(" browse  ", shift_hint_style),
            Span::styled("q", key_style),
            Span::styled(" quit", hint_style),
        ]);

        Paragraph::new(Line::from(spans))
        .style(bar_style)
    };

    let version_label = format!("v{} ", env!("CARGO_PKG_VERSION"));
    let version_width = version_label.len() as u16;
    let bar_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Min(0),
            Constraint::Length(version_width),
        ])
        .split(chunks[1]);

    frame.render_widget(bottom_bar, bar_chunks[0]);
    frame.render_widget(
        Paragraph::new(Span::styled(
            version_label,
            Style::default().fg(FG_OVERLAY),
        ))
        .style(bar_style)
        .alignment(ratatui::layout::Alignment::Right),
        bar_chunks[1],
    );

    // Rename popup overlay
    if app.mode == AppMode::Renaming {
        draw_rename_popup(frame, &app.rename_text);
    }

    // Directory browser overlay
    if app.mode == AppMode::DirBrowser {
        if let Some(browser) = &app.dir_browser {
            draw_dir_browser(frame, browser);
        }
    }

    // Update prompt overlay
    if app.mode == AppMode::UpdatePrompt {
        if let UpdateStatus::Available(ref info) = app.update_status {
            draw_update_prompt(frame, info);
        }
    }
}

fn build_preview_text(messages: &[PreviewMessage]) -> Text<'static> {
    let mut lines: Vec<Line<'static>> = Vec::new();

    for (i, msg) in messages.iter().enumerate() {
        if i > 0 {
            // Separator between messages
            lines.push(Line::from(Span::styled(
                "───────────────────────────────────────",
                Style::default().fg(FG_OVERLAY),
            )));
        }

        let (label, color) = match msg.role.as_str() {
            "user" => ("USER", ACCENT_MAUVE),
            "assistant" => ("ASSISTANT", ACCENT_GREEN),
            _ => ("SYSTEM", ACCENT_PEACH),
        };

        lines.push(Line::from(vec![
            Span::styled(
                "▎ ",
                Style::default().fg(color),
            ),
            Span::styled(
                format!("{}:", label),
                Style::default().fg(color).add_modifier(Modifier::BOLD),
            ),
        ]));

        // Truncate very long messages (char-aware to avoid UTF-8 panics)
        let text = if msg.text.chars().count() > 2000 {
            let truncated: String = msg.text.chars().take(2000).collect();
            format!("{}...", truncated)
        } else {
            msg.text.clone()
        };

        for line in text.lines() {
            lines.push(Line::from(Span::styled(
                line.to_string(),
                Style::default().fg(FG_TEXT),
            )));
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

fn draw_dir_browser(frame: &mut Frame, browser: &crate::app::DirBrowser) {
    let area = centered_rect(60, 70, frame.area());
    frame.render_widget(Clear, area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(3),
            Constraint::Length(1),
        ])
        .split(area);

    // Path bar
    let path_content = if browser.input_active {
        Line::from(vec![
            Span::styled(&browser.input_text, Style::default().fg(FG_TEXT)),
            Span::styled("█", Style::default().fg(ACCENT_BLUE)),
        ])
    } else {
        Line::from(Span::styled(
            browser.current_dir.to_string_lossy().to_string(),
            Style::default().fg(FG_TEXT),
        ))
    };

    let path_bar = Paragraph::new(path_content).block(
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(ACCENT_BLUE))
            .title(Span::styled(
                " New Session — Directory ",
                Style::default()
                    .fg(ACCENT_BLUE)
                    .add_modifier(Modifier::BOLD),
            ))
            .style(Style::default().bg(BG_SURFACE)),
    );
    frame.render_widget(path_bar, chunks[0]);

    // Directory list
    let items: Vec<ListItem> = browser
        .entries
        .iter()
        .map(|entry| {
            let style = if entry.name == ".." {
                Style::default().fg(FG_SUBTEXT)
            } else {
                Style::default().fg(ACCENT_GREEN)
            };
            let prefix = if entry.is_dir { "📁 " } else { "  " };
            ListItem::new(Line::from(Span::styled(
                format!("{}{}", prefix, entry.name),
                style,
            )))
        })
        .collect();

    let dir_list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(FG_OVERLAY))
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
    state.select(Some(browser.selected));
    frame.render_stateful_widget(dir_list, chunks[1], &mut state);

    // Help bar
    let help = Paragraph::new(Line::from(vec![
        Span::styled(
            " ↑↓",
            Style::default()
                .fg(ACCENT_PEACH)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" nav  ", Style::default().fg(FG_SUBTEXT)),
        Span::styled(
            "Enter",
            Style::default()
                .fg(ACCENT_PEACH)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" open  ", Style::default().fg(FG_SUBTEXT)),
        Span::styled(
            "Space",
            Style::default()
                .fg(ACCENT_PEACH)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" select  ", Style::default().fg(FG_SUBTEXT)),
        Span::styled(
            "/",
            Style::default()
                .fg(ACCENT_PEACH)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" type path  ", Style::default().fg(FG_SUBTEXT)),
        Span::styled(
            "Esc",
            Style::default()
                .fg(ACCENT_PEACH)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" cancel", Style::default().fg(FG_SUBTEXT)),
    ]))
    .style(Style::default().bg(HIGHLIGHT_BG));
    frame.render_widget(help, chunks[2]);
}

fn draw_rename_popup(frame: &mut Frame, text: &str) {
    let area = centered_rect(40, 15, frame.area());
    // Ensure minimum usable height of 3 lines
    let area = if area.height < 3 {
        Rect { height: 3, ..area }
    } else {
        area
    };
    frame.render_widget(Clear, area);

    let content = Line::from(vec![
        Span::styled(text, Style::default().fg(FG_TEXT)),
        Span::styled("\u{2588}", Style::default().fg(ACCENT_BLUE)),
    ]);

    let popup = Paragraph::new(content).block(
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(ACCENT_PEACH))
            .title(Span::styled(
                " Rename Session ",
                Style::default()
                    .fg(ACCENT_PEACH)
                    .add_modifier(Modifier::BOLD),
            ))
            .style(Style::default().bg(BG_SURFACE)),
    );
    frame.render_widget(popup, area);
}

fn draw_update_prompt(frame: &mut Frame, info: &crate::update::UpdateInfo) {
    let area = centered_rect(40, 15, frame.area());
    let area = if area.height < 6 {
        Rect { height: 6, ..area }
    } else {
        area
    };
    frame.render_widget(Clear, area);

    let key_style = Style::default()
        .fg(ACCENT_PEACH)
        .add_modifier(Modifier::BOLD);
    let text_style = Style::default().fg(FG_TEXT);

    let content = vec![
        Line::from(""),
        Line::from(vec![
            Span::styled(format!("v{}", info.current), Style::default().fg(FG_SUBTEXT)),
            Span::styled("  →  ", Style::default().fg(FG_OVERLAY)),
            Span::styled(info.tag.clone(), Style::default().fg(ACCENT_GREEN).add_modifier(Modifier::BOLD)),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("  y", key_style),
            Span::styled(" update   ", text_style),
            Span::styled("n/Esc", key_style),
            Span::styled(" skip", text_style),
        ]),
    ];

    let popup = Paragraph::new(content)
        .alignment(ratatui::layout::Alignment::Center)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(ACCENT_GREEN))
                .title(Span::styled(
                    " Update Available ",
                    Style::default()
                        .fg(ACCENT_GREEN)
                        .add_modifier(Modifier::BOLD),
                ))
                .style(Style::default().bg(BG_SURFACE)),
        );
    frame.render_widget(popup, area);
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
