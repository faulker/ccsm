use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, BorderType, Borders, Clear, Paragraph, Wrap},
    Frame,
};

use crate::theme::{
    ACCENT_BLUE, ACCENT_GREEN, ACCENT_MAUVE, ACCENT_PEACH, BG_SURFACE, FG_OVERLAY, FG_SUBTEXT,
    FG_TEXT,
};

use super::util::centered_rect;

/// Render the centered popup for naming a new live session, showing a placeholder when the buffer is empty.
pub fn draw_naming_popup(frame: &mut Frame, input: &tui_input::Input, placeholder: &str) {
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
pub fn draw_duplicate_popup(frame: &mut Frame, name: &str) {
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

/// Render the centered rename popup showing the current `text` and a block cursor.
pub fn draw_rename_popup(frame: &mut Frame, input: &tui_input::Input) {
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
pub fn draw_update_prompt(frame: &mut Frame, info: &crate::update::UpdateInfo) {
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

/// Render the full-screen help overlay listing all keyboard shortcuts.
pub fn render_help_popup(frame: &mut Frame, area: Rect) {
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
