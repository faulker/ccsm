use ratatui::{
    layout::{Constraint, Direction, Layout},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, List, ListItem, ListState, Paragraph},
    Frame,
};

use crate::app::{App, PickerKind};
use crate::theme::{
    ACCENT_BLUE, ACCENT_PEACH, ACCENT_RED, BG_SURFACE, FG_SUBTEXT, FG_TEXT, HIGHLIGHT_BG,
};

use super::util::{centered_rect, input_spans};

/// Render the directory-picker modal: a path box, a scrollable directory list, and a hint bar.
pub fn draw_dir_picker(frame: &mut Frame, app: &mut App) {
    let title = app.dir_picker_target.title();
    let Some(browser) = app.dir_browser.as_mut() else {
        return;
    };
    let file_mode = browser.kind == PickerKind::File;

    let area = centered_rect(60, 60, frame.area());
    frame.render_widget(Clear, area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(3), Constraint::Length(1)])
        .split(area);

    // Path box: shows the current directory, or the editable path input when typing.
    let path_line = if browser.input_active {
        Line::from(input_spans(&browser.path_input, Style::default().fg(FG_TEXT)))
    } else {
        Line::from(Span::styled(
            browser.current_dir.to_string_lossy().to_string(),
            Style::default().fg(FG_TEXT),
        ))
    };

    let path_box = Paragraph::new(path_line).block(
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(ACCENT_PEACH))
            .title(Span::styled(
                title,
                Style::default().fg(ACCENT_PEACH).add_modifier(Modifier::BOLD),
            ))
            .style(Style::default().bg(BG_SURFACE)),
    );
    frame.render_widget(path_box, chunks[0]);

    // Directory list. Scroll is kept manually (rather than via `ListState`'s
    // built-in scrolling) so `browser.scroll` stays the single source of truth
    // and can be unit-tested independent of rendering.
    let list_area = chunks[1];
    let visible_rows = list_area.height.saturating_sub(2) as usize;
    if visible_rows > 0 {
        if browser.selected < browser.scroll {
            browser.scroll = browser.selected;
        } else if browser.selected >= browser.scroll + visible_rows {
            browser.scroll = browser.selected + 1 - visible_rows;
        }
    }

    let items: Vec<ListItem> = browser
        .entries
        .iter()
        .skip(browser.scroll)
        .take(visible_rows.max(1))
        .map(|entry| {
            let icon = if entry.is_dir { "\u{1F4C1}" } else { "\u{1F4C4}" };
            ListItem::new(Line::from(vec![
                Span::raw(format!("{icon} ")),
                Span::raw(entry.name.clone()),
            ]))
        })
        .collect();

    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(ACCENT_PEACH))
                .style(Style::default().bg(BG_SURFACE)),
        )
        .highlight_style(
            Style::default()
                .bg(HIGHLIGHT_BG)
                .fg(ACCENT_BLUE)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("\u{25B6} ");

    let mut state = ListState::default();
    state.select(Some(browser.selected.saturating_sub(browser.scroll)));
    frame.render_stateful_widget(list, list_area, &mut state);

    // Hint bar (or error message, if a navigation/path error is set).
    let key_style = Style::default().fg(ACCENT_PEACH).add_modifier(Modifier::BOLD);
    let hint_style = Style::default().fg(FG_SUBTEXT);
    let hint_line = if let Some(err) = &browser.error {
        Line::from(Span::styled(
            format!(" {err} "),
            Style::default().fg(ACCENT_RED).add_modifier(Modifier::BOLD),
        ))
    } else {
        Line::from(vec![
            Span::styled(" \u{2191}\u{2193}", key_style),
            Span::styled(" nav   ", hint_style),
            Span::styled("Enter", key_style),
            Span::styled(
                if file_mode { " open/pick file   " } else { " open   " },
                hint_style,
            ),
            Span::styled("Space", key_style),
            Span::styled(
                if file_mode { " pick file   " } else { " select this dir   " },
                hint_style,
            ),
            Span::styled("/", key_style),
            Span::styled(" type path   ", hint_style),
            Span::styled("Esc", key_style),
            Span::styled(" cancel", hint_style),
        ])
    };
    frame.render_widget(
        Paragraph::new(hint_line).style(Style::default().bg(HIGHLIGHT_BG)),
        chunks[2],
    );
}
