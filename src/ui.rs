use crate::app::{App, AppMode, FlatRow, TreeRow};
use crate::config::DisplayMode;
use crate::data::PreviewMessage;
use crate::update::UpdateStatus;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};
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
    let items: Vec<ListItem> = if app.tree_view {
        app.tree_rows
            .iter()
            .map(|row| match row {
                TreeRow::Header {
                    project_name,
                    session_count,
                    project,
                } => {
                    let running = app.project_running_count(project);
                    let arrow = if app.collapsed.contains(project) {
                        "▸"
                    } else {
                        "▾"
                    };
                    let count_str = session_count.to_string();
                    let overhead = 5 + count_str.len();
                    let available = session_panel_inner_width.saturating_sub(overhead);
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
                    if running > 0 {
                        header_spans.push(Span::styled(
                            format!(" ● {}", running),
                            Style::default().fg(ACCENT_GREEN).add_modifier(Modifier::BOLD),
                        ));
                    }
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
                    ListItem::new(Line::from(vec![
                        Span::raw("    "),
                        Span::styled("● ", Style::default().fg(ACCENT_GREEN)),
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
    } else {
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
                    ListItem::new(Line::from(vec![
                        Span::styled("● ", Style::default().fg(ACCENT_GREEN).add_modifier(Modifier::BOLD)),
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
                                truncate_left(&name, 28)
                            } else {
                                truncate(&name, 28)
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
    if !app.group_chains {
        title_spans.push(Span::styled(" [ungrouped]", title_style));
    }
    if let Some(p) = &app.filter_path {
        title_spans.push(Span::styled(format!(" ({})", p), title_style));
    }
    let running_count = app.total_running_count();
    if running_count > 0 {
        title_spans.push(Span::styled(
            format!(" ● {} running", running_count),
            Style::default().fg(ACCENT_GREEN).add_modifier(Modifier::BOLD),
        ));
    }
    if app.live_filter {
        title_spans.push(Span::styled(" [live only]", Style::default().fg(ACCENT_GREEN)));
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
            spans.push(Span::styled("● ", Style::default().fg(ACCENT_GREEN).add_modifier(Modifier::BOLD)));
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
    if app.preview_scroll > max_scroll {
        app.preview_scroll = max_scroll;
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

    // Help / search bar
    let bar_style = Style::default().bg(HIGHLIGHT_BG);
    let version_label = format!("v{} ", env!("CARGO_PKG_VERSION"));
    let version_width = version_label.len() as u16;
    let bar_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Min(0),
            Constraint::Length(version_width),
        ])
        .split(chunks[1]);

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
            cursor_spans.push(Span::styled(on_cursor, Style::default().bg(ACCENT_BLUE).fg(BG_SURFACE)));
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

    // DuplicateSession confirmation popup
    if app.mode == AppMode::DuplicateSession {
        if let Some(ref name) = app.duplicate_name.clone() {
            draw_duplicate_popup(frame, name);
        }
    }
}

/// Convert a slice of conversation messages into a styled ratatui `Text` ready for the preview pane.
/// Each message is preceded by a role label; messages are separated by a horizontal rule.
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

/// Convert raw tmux pane output (with ANSI escape sequences) into a styled ratatui `Text`.
fn build_live_preview_text(raw: &str) -> Text<'static> {
    let mut lines: Vec<Line<'static>> = Vec::new();
    for raw_line in raw.lines() {
        lines.push(parse_ansi_line(raw_line));
    }
    Text::from(lines)
}

/// Parse a single line containing ANSI SGR escape sequences into a ratatui `Line`.
fn parse_ansi_line(line: &str) -> Line<'static> {
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut style = Style::default();
    let mut text = String::new();
    let mut chars = line.chars().peekable();

    while let Some(c) = chars.next() {
        if c == '\x1b' && chars.peek() == Some(&'[') {
            chars.next(); // consume '['
            let mut seq = String::new();
            // Collect parameter bytes until a final byte (@ through ~)
            while let Some(&next) = chars.peek() {
                if next.is_ascii_alphabetic() || next == '@' {
                    chars.next();
                    if next == 'm' {
                        // SGR sequence — flush current text then apply style change
                        if !text.is_empty() {
                            spans.push(Span::styled(text.clone(), style));
                            text.clear();
                        }
                        style = apply_sgr(&seq, style);
                    }
                    // Non-'m' sequences (cursor movement etc.) are silently skipped
                    break;
                }
                seq.push(next);
                chars.next();
            }
        } else {
            text.push(c);
        }
    }
    if !text.is_empty() {
        spans.push(Span::styled(text, style));
    }
    Line::from(spans)
}

/// Apply an ANSI SGR parameter string to a base `Style`, returning the updated style.
fn apply_sgr(params: &str, mut style: Style) -> Style {
    if params.is_empty() {
        return Style::default();
    }
    let codes: Vec<u32> = params.split(';').filter_map(|s| s.parse().ok()).collect();
    let mut i = 0;
    while i < codes.len() {
        match codes[i] {
            0 => style = Style::default(),
            1 => style = style.add_modifier(Modifier::BOLD),
            2 => style = style.add_modifier(Modifier::DIM),
            3 => style = style.add_modifier(Modifier::ITALIC),
            4 => style = style.add_modifier(Modifier::UNDERLINED),
            7 => style = style.add_modifier(Modifier::REVERSED),
            // Standard foreground
            30 => style = style.fg(Color::Black),
            31 => style = style.fg(Color::Red),
            32 => style = style.fg(Color::Green),
            33 => style = style.fg(Color::Yellow),
            34 => style = style.fg(Color::Blue),
            35 => style = style.fg(Color::Magenta),
            36 => style = style.fg(Color::Cyan),
            37 => style = style.fg(Color::White),
            39 => style = style.fg(Color::Reset),
            // Bright foreground
            90 => style = style.fg(Color::DarkGray),
            91 => style = style.fg(Color::LightRed),
            92 => style = style.fg(Color::LightGreen),
            93 => style = style.fg(Color::LightYellow),
            94 => style = style.fg(Color::LightBlue),
            95 => style = style.fg(Color::LightMagenta),
            96 => style = style.fg(Color::LightCyan),
            97 => style = style.fg(Color::Gray),
            // Standard background
            40 => style = style.bg(Color::Black),
            41 => style = style.bg(Color::Red),
            42 => style = style.bg(Color::Green),
            43 => style = style.bg(Color::Yellow),
            44 => style = style.bg(Color::Blue),
            45 => style = style.bg(Color::Magenta),
            46 => style = style.bg(Color::Cyan),
            47 => style = style.bg(Color::White),
            49 => style = style.bg(Color::Reset),
            // Bright background
            100 => style = style.bg(Color::DarkGray),
            101 => style = style.bg(Color::LightRed),
            102 => style = style.bg(Color::LightGreen),
            103 => style = style.bg(Color::LightYellow),
            104 => style = style.bg(Color::LightBlue),
            105 => style = style.bg(Color::LightMagenta),
            106 => style = style.bg(Color::LightCyan),
            107 => style = style.bg(Color::Gray),
            // 256-color / true-color foreground
            38 if codes.get(i + 1) == Some(&5) => {
                if let Some(&n) = codes.get(i + 2) {
                    style = style.fg(Color::Indexed(n.min(255) as u8));
                    i += 2;
                }
            }
            38 if codes.get(i + 1) == Some(&2) => {
                if let (Some(&r), Some(&g), Some(&b)) =
                    (codes.get(i + 2), codes.get(i + 3), codes.get(i + 4))
                {
                    style = style.fg(Color::Rgb(r.min(255) as u8, g.min(255) as u8, b.min(255) as u8));
                    i += 4;
                }
            }
            // 256-color / true-color background
            48 if codes.get(i + 1) == Some(&5) => {
                if let Some(&n) = codes.get(i + 2) {
                    style = style.bg(Color::Indexed(n.min(255) as u8));
                    i += 2;
                }
            }
            48 if codes.get(i + 1) == Some(&2) => {
                if let (Some(&r), Some(&g), Some(&b)) =
                    (codes.get(i + 2), codes.get(i + 3), codes.get(i + 4))
                {
                    style = style.bg(Color::Rgb(r.min(255) as u8, g.min(255) as u8, b.min(255) as u8));
                    i += 4;
                }
            }
            _ => {}
        }
        i += 1;
    }
    style
}


/// Render the centered popup for naming a new live session, showing a placeholder when the buffer is empty.
fn draw_naming_popup(frame: &mut Frame, input: &tui_input::Input, placeholder: &str) {
    let area = centered_rect(40, 3, frame.area());
    let area = if area.height < 3 {
        Rect { height: 3, ..area }
    } else {
        area
    };
    frame.render_widget(Clear, area);

    let content = if input.value().is_empty() {
        Line::from(vec![
            Span::styled(placeholder.to_string(), Style::default().fg(FG_OVERLAY)),
            Span::styled("\u{2588}", Style::default().fg(ACCENT_BLUE)),
        ])
    } else {
        let cursor = input.visual_cursor();
        let char_count = input.value().chars().count();
        let before: String = input.value().chars().take(cursor).collect();
        let content_line = if cursor >= char_count {
            vec![
                Span::styled(before, Style::default().fg(FG_TEXT)),
                Span::styled("\u{2588}", Style::default().fg(ACCENT_BLUE)),
            ]
        } else {
            let on_cursor: String = input.value().chars().nth(cursor).unwrap().to_string();
            let after: String = input.value().chars().skip(cursor + 1).collect();
            vec![
                Span::styled(before, Style::default().fg(FG_TEXT)),
                Span::styled(on_cursor, Style::default().bg(ACCENT_BLUE).fg(BG_SURFACE)),
                Span::styled(after, Style::default().fg(FG_TEXT)),
            ]
        };
        Line::from(content_line)
    };

    let popup = Paragraph::new(content).block(
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(ACCENT_PEACH))
            .title(Span::styled(
                " New Session (Esc to cancel) ",
                Style::default()
                    .fg(ACCENT_PEACH)
                    .add_modifier(Modifier::BOLD),
            ))
            .style(Style::default().bg(BG_SURFACE)),
    );
    frame.render_widget(popup, area);
}

/// Render the duplicate-session confirmation popup with open/rename/cancel options.
fn draw_duplicate_popup(frame: &mut Frame, name: &str) {
    let area = centered_rect(44, 20, frame.area());
    let area = if area.height < 7 {
        Rect { height: 7, ..area }
    } else {
        area
    };
    frame.render_widget(Clear, area);

    let key_style = Style::default().fg(ACCENT_PEACH).add_modifier(Modifier::BOLD);
    let text_style = Style::default().fg(FG_TEXT);
    let dim_style = Style::default().fg(FG_SUBTEXT);

    let content = vec![
        Line::from(""),
        Line::from(vec![
            Span::styled("Session ", text_style),
            Span::styled(format!("\"{}\"", name), Style::default().fg(ACCENT_PEACH).add_modifier(Modifier::BOLD)),
            Span::styled(" already exists", text_style),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("  o", key_style),
            Span::styled(" / Enter  open existing session", text_style),
        ]),
        Line::from(vec![
            Span::styled("  r", key_style),
            Span::styled("          choose a different name", text_style),
        ]),
        Line::from(vec![
            Span::styled("  Esc", key_style),
            Span::styled("       cancel", dim_style),
        ]),
    ];

    let popup = Paragraph::new(content)
        .alignment(ratatui::layout::Alignment::Left)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(ACCENT_PEACH))
                .title(Span::styled(
                    " Duplicate Session Name ",
                    Style::default().fg(ACCENT_PEACH).add_modifier(Modifier::BOLD),
                ))
                .style(Style::default().bg(BG_SURFACE)),
        );
    frame.render_widget(popup, area);
}

/// Format a millisecond timestamp as a short human-readable relative date
/// (e.g. `"5m ago"`, `"3h ago"`, `"2d ago"`, or `"Jan 02"` for older dates).
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

/// Estimate the number of terminal rows a `Text` will occupy when word-wrapped to `width` columns.
/// Used to compute the maximum valid scroll offset before rendering.
fn estimate_wrapped_height(text: &Text, width: usize) -> usize {
    if width == 0 {
        return text.lines.len();
    }
    text.lines
        .iter()
        .map(|line| {
            let line_width: usize = line.spans.iter().map(|s| s.content.width()).sum();
            if line_width == 0 {
                1
            } else {
                (line_width + width - 1) / width
            }
        })
        .sum()
}

/// Render the full-screen help overlay listing all keyboard shortcuts.
fn render_help_popup(frame: &mut Frame, area: Rect) {
    let popup_area = centered_rect(70, 80, area);
    frame.render_widget(Clear, popup_area);

    let block = Block::default()
        .title(" Claude Code Session Manager (ccsm) ")
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(ACCENT_MAUVE))
        .style(Style::default().bg(BG_SURFACE));

    let inner = block.inner(popup_area);
    frame.render_widget(block, popup_area);

    let header = Style::default().fg(ACCENT_MAUVE).add_modifier(Modifier::BOLD);
    let key = Style::default().fg(ACCENT_PEACH).add_modifier(Modifier::BOLD);
    let desc = Style::default().fg(FG_TEXT);
    let sub = Style::default().fg(FG_SUBTEXT);

    let lines: Vec<Line> = vec![
        Line::from(vec![
            Span::styled("  https://github.com/faulker/ccsm", sub),
        ]),
        Line::from(""),
        Line::from(vec![Span::styled("  Navigation", header)]),
        Line::from(vec![
            Span::styled("  j/k  ↑/↓        ", key),
            Span::styled("Move selection up/down (Shift to scroll preview)", desc),
        ]),
        Line::from(vec![
            Span::styled("  ←/→             ", key),
            Span::styled("Collapse/expand group (tree view) or jump to parent header", desc),
        ]),
        Line::from(vec![
            Span::styled("  Enter           ", key),
            Span::styled("Open selected session (via tmux)", desc),
        ]),
        Line::from(vec![
            Span::styled("  Shift+Enter     ", key),
            Span::styled("Open historical session directly (no tmux)", desc),
        ]),
        Line::from(""),
        Line::from(vec![Span::styled("  Actions", header)]),
        Line::from(vec![
            Span::styled("  /               ", key),
            Span::styled("Enter filter/search mode", desc),
        ]),
        Line::from(vec![
            Span::styled("  o               ", key),
            Span::styled("Open config popup (view mode, hide empty, group chains)", desc),
        ]),
        Line::from(vec![
            Span::styled("  n               ", key),
            Span::styled("Start new live session", desc),
        ]),
        Line::from(vec![
            Span::styled("  Shift+N         ", key),
            Span::styled("Open direct claude session (no tmux)", desc),
        ]),
        Line::from(vec![
            Span::styled("  x               ", key),
            Span::styled("Stop selected live session gracefully", desc),
        ]),
        Line::from(vec![
            Span::styled("  l               ", key),
            Span::styled("Toggle live-only filter", desc),
        ]),
        Line::from(vec![
            Span::styled("  r               ", key),
            Span::styled("Rename selected session or live session", desc),
        ]),
        Line::from(vec![
            Span::styled("  f               ", key),
            Span::styled("Toggle favorite — pins project to top of list (shown with ★)", desc),
        ]),
        Line::from(vec![
            Span::styled("  q / Esc         ", key),
            Span::styled("Quit", desc),
        ]),
        Line::from(""),
        Line::from(vec![Span::styled("  Filter Mode", header)]),
        Line::from(vec![
            Span::styled("  Enter           ", key),
            Span::styled("Confirm filter and return to Normal mode", desc),
        ]),
        Line::from(vec![
            Span::styled("  Esc             ", key),
            Span::styled("Clear filter and return to Normal mode", desc),
        ]),
        Line::from(vec![
            Span::styled("  Backspace       ", key),
            Span::styled("Delete last character", desc),
        ]),
        Line::from(""),
        Line::from(vec![Span::styled("  Rename Mode", header)]),
        Line::from(vec![
            Span::styled("  Enter           ", key),
            Span::styled("Save new name", desc),
        ]),
        Line::from(vec![
            Span::styled("  Esc             ", key),
            Span::styled("Cancel rename", desc),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("  ? or Esc to close", sub),
        ]),
    ];

    let paragraph = Paragraph::new(Text::from(lines))
        .wrap(Wrap { trim: false });
    frame.render_widget(paragraph, inner);
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


/// Render the centered rename popup showing the current `text` and a block cursor.
fn draw_rename_popup(frame: &mut Frame, input: &tui_input::Input) {
    let area = centered_rect(40, 3, frame.area());
    // Ensure minimum usable height of 3 lines
    let area = if area.height < 3 {
        Rect { height: 3, ..area }
    } else {
        area
    };
    frame.render_widget(Clear, area);

    let cursor = input.visual_cursor();
    let char_count = input.value().chars().count();
    let before: String = input.value().chars().take(cursor).collect();
    let content = if cursor >= char_count {
        Line::from(vec![
            Span::styled(before, Style::default().fg(FG_TEXT)),
            Span::styled("\u{2588}", Style::default().fg(ACCENT_BLUE)),
        ])
    } else {
        let on_cursor: String = input.value().chars().nth(cursor).unwrap().to_string();
        let after: String = input.value().chars().skip(cursor + 1).collect();
        Line::from(vec![
            Span::styled(before, Style::default().fg(FG_TEXT)),
            Span::styled(on_cursor, Style::default().bg(ACCENT_BLUE).fg(BG_SURFACE)),
            Span::styled(after, Style::default().fg(FG_TEXT)),
        ])
    };

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

/// Render the update-available prompt showing the current and new version with y/n options.
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
            Span::styled(" update & restart   ", text_style),
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

/// Truncate `s` to at most `max` display columns, appending `…` if truncated, and right-padding with spaces to exactly `max` columns.
fn truncate(s: &str, max: usize) -> String {
    if max == 0 {
        return String::new();
    }
    let width = s.width();
    if width <= max {
        let pad = max - width;
        format!("{}{}", s, " ".repeat(pad))
    } else {
        let mut truncated = String::new();
        let mut w = 0;
        for c in s.chars() {
            let cw = c.width().unwrap_or(0);
            if w + cw > max - 1 {
                break;
            }
            truncated.push(c);
            w += cw;
        }
        let pad = max - w - 1;
        format!("{}…{}", truncated, " ".repeat(pad))
    }
}

/// Like `truncate` but truncates from the left, prepending `…` when the string exceeds `max` columns.
/// Useful for long directory paths where the end (project name) is most relevant.
fn truncate_left(s: &str, max: usize) -> String {
    if max == 0 {
        return String::new();
    }
    let width = s.width();
    if width <= max {
        let pad = max - width;
        format!("{}{}", s, " ".repeat(pad))
    } else {
        // Walk from the end to collect characters that fit in (max - 1) columns
        let chars: Vec<char> = s.chars().collect();
        let mut w = 0;
        let mut start = chars.len();
        for i in (0..chars.len()).rev() {
            let cw = chars[i].width().unwrap_or(0);
            if w + cw > max - 1 {
                break;
            }
            w += cw;
            start = i;
        }
        let truncated: String = chars[start..].iter().collect();
        let pad = max - w - 1;
        format!("…{}{}", truncated, " ".repeat(pad))
    }
}

/// Like `truncate_left` but returns the raw string without trailing space padding,
/// used for header labels in the tree view where padding is not needed.
fn truncate_left_plain(s: &str, max: usize) -> String {
    if max == 0 {
        return String::new();
    }
    let width = s.width();
    if width <= max {
        s.to_string()
    } else {
        let chars: Vec<char> = s.chars().collect();
        let mut w = 0;
        let mut start = chars.len();
        for i in (0..chars.len()).rev() {
            let cw = chars[i].width().unwrap_or(0);
            if w + cw > max - 1 {
                break;
            }
            w += cw;
            start = i;
        }
        format!("…{}", chars[start..].iter().collect::<String>())
    }
}
