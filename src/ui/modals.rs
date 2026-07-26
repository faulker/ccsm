use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, BorderType, Borders, Clear, Paragraph, Wrap},
    Frame,
};

use crate::app::HelpTab;
use crate::theme::{
    ACCENT_GREEN, ACCENT_MAUVE, ACCENT_PEACH, ACCENT_RED, BG_SURFACE, FG_OVERLAY, FG_SUBTEXT,
    FG_TEXT,
};

use super::util::{centered_rect, input_spans, input_spans_with_placeholder};

/// Render the centered popup for naming a new live session, showing a placeholder when the buffer is empty.
pub fn draw_naming_popup(frame: &mut Frame, input: &tui_input::Input, placeholder: &str, dangerous: bool) {
    let area = centered_rect(40, 3, frame.area());
    let area = if area.height < 3 {
        Rect { height: 3, ..area }
    } else {
        area
    };
    frame.render_widget(Clear, area);

    let content = Line::from(input_spans_with_placeholder(
        input,
        Style::default().fg(FG_TEXT),
        placeholder,
    ));

    let (title, border_color) = if dangerous {
        (" New Session ⚠ skip-permissions (Esc to cancel) ", ACCENT_RED)
    } else {
        (" New Session (Esc to cancel) ", ACCENT_PEACH)
    };

    let popup = Paragraph::new(content).block(
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(border_color))
            .title(Span::styled(
                title,
                Style::default()
                    .fg(border_color)
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

    let content = Line::from(input_spans(input, Style::default().fg(FG_TEXT)));

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

/// Render the tabbed help overlay. `tab` selects the page; the caller picks the
/// page that matches what the user is currently looking at (Jobs help while on
/// the Jobs tab), so help opens on something relevant rather than page one.
pub fn render_help_popup(frame: &mut Frame, area: Rect, tab: HelpTab) {
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

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(2), Constraint::Min(1), Constraint::Length(1)])
        .split(inner);

    // Tab strip
    let active = Style::default()
        .fg(ACCENT_PEACH)
        .add_modifier(Modifier::BOLD | Modifier::UNDERLINED);
    let inactive = Style::default().fg(FG_SUBTEXT);
    let mut tab_spans: Vec<Span> = vec![Span::raw("  ")];
    for t in HelpTab::ALL {
        tab_spans.push(Span::styled(
            format!(" {} ", t.label()),
            if t == tab { active } else { inactive },
        ));
        tab_spans.push(Span::raw(" "));
    }
    frame.render_widget(
        Paragraph::new(Text::from(vec![Line::from(tab_spans), Line::from("")])),
        chunks[0],
    );

    let lines = match tab {
        HelpTab::Sessions => sessions_help_lines(),
        HelpTab::Jobs => jobs_help_lines(),
        HelpTab::General => general_help_lines(),
    };
    frame.render_widget(
        Paragraph::new(Text::from(lines)).wrap(Wrap { trim: false }),
        chunks[1],
    );

    let sub = Style::default().fg(FG_SUBTEXT);
    let key = Style::default().fg(ACCENT_PEACH).add_modifier(Modifier::BOLD);
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("  Tab/←/→", key),
            Span::styled(" switch help tab   ", sub),
            Span::styled("? or Esc", key),
            Span::styled(" close", sub),
        ])),
        chunks[2],
    );
}

/// One help row: a fixed-width key column followed by its description.
fn help_row(keys: &str, description: &str) -> Line<'static> {
    Line::from(vec![
        Span::styled(
            format!("  {:<16}", keys),
            Style::default().fg(ACCENT_PEACH).add_modifier(Modifier::BOLD),
        ),
        Span::styled(description.to_string(), Style::default().fg(FG_TEXT)),
    ])
}

/// A section heading inside a help page.
fn help_header(text: &str) -> Line<'static> {
    Line::from(Span::styled(
        format!("  {text}"),
        Style::default().fg(ACCENT_MAUVE).add_modifier(Modifier::BOLD),
    ))
}

/// Help page for the Sessions tab.
fn sessions_help_lines() -> Vec<Line<'static>> {
    vec![
        help_header("Navigation"),
        help_row("j/k  ↑/↓", "Move selection up/down (Shift to scroll preview)"),
        help_row("←/→", "Collapse/expand group (tree view) or jump to parent header"),
        help_row("Enter", "Open selected session (via tmux)"),
        help_row("Shift+Enter", "Open historical session directly (no tmux)"),
        Line::from(""),
        help_header("Actions"),
        help_row("n", "Start new live session"),
        help_row("Shift+N", "Open direct claude session (no tmux)"),
        help_row("Shift+D", "New live session with --dangerously-skip-permissions"),
        help_row("b", "Browse filesystem to choose a directory for a new session"),
        help_row("m", "Create/edit a job prefilled from the current selection"),
        help_row("x", "Stop selected live session gracefully"),
        help_row("l", "Toggle live-only filter"),
        help_row("r", "Rename selected session or live session"),
        help_row("f", "Toggle favorite — pins project to top of list (shown with ★)"),
        Line::from(""),
        help_header("Filter mode"),
        help_row("/", "Enter filter/search mode"),
        help_row("←/→  Home/End", "Move the cursor within the filter text"),
        help_row("Enter", "Confirm filter and return to Normal mode"),
        help_row("Esc", "Clear filter and return to Normal mode"),
        Line::from(""),
        help_header("Rename mode"),
        help_row("←/→  Home/End", "Move the cursor within the name"),
        help_row("Enter", "Save new name"),
        help_row("Esc", "Cancel rename"),
    ]
}

/// Help page for the Jobs tab, its form, and the watcher daemon.
fn jobs_help_lines() -> Vec<Line<'static>> {
    vec![
        help_header("Jobs tab"),
        help_row("j/k  ↑/↓", "Move between jobs"),
        help_row("Enter", "Attach to the job's tmux session"),
        help_row("n / e", "New / edit job"),
        help_row("p / c", "Pause / resume job"),
        help_row("x / d", "Stop / delete job (with confirmation)"),
        help_row("Space", "Toggle auto-resume for the selected job"),
        help_row("s", "Start/stop the watcher daemon"),
        help_row("L", "Attach to the watcher's own log session"),
        help_row("Esc", "Back to the Sessions tab"),
        Line::from(""),
        help_header("Job form"),
        help_row("j/k  Tab", "Move between fields"),
        help_row("Enter", "Edit text field, toggle a checkbox, or submit"),
        help_row("Enter / b", "On Directory: browse the filesystem for the cwd"),
        help_row("i", "On Directory: type the path by hand instead"),
        help_row("←/→  Home/End", "Move the cursor while editing a field"),
        help_row("Esc", "Cancel the current edit, then leave the form"),
        Line::from(""),
        help_header("How the watcher works"),
        help_row("pause", "Jobs pause once usage reaches the configured pause %"),
        help_row("resume", "Auto-resume jobs restart when usage falls or resets"),
        help_row("state", "The tab title shows watcher on/off; off means nothing runs"),
    ]
}

/// Help page for global keys, the config popup, and the directory picker.
fn general_help_lines() -> Vec<Line<'static>> {
    vec![
        help_header("Global"),
        help_row("Tab / Shift+Tab", "Switch between the Sessions and Jobs tabs"),
        help_row("w", "Jump straight to the Jobs tab"),
        help_row("o", "Open the config popup"),
        help_row("?", "Open this help"),
        help_row("q / Ctrl+C", "Quit"),
        Line::from(""),
        help_header("Config popup"),
        help_row("j/k", "Move between settings"),
        help_row("Space/Enter", "Toggle a setting, edit a value, or browse for a path"),
        help_row("i", "Type a path by hand instead of browsing"),
        help_row("Tab / Shift+Tab", "Cycle the session view mode"),
        help_row("Esc", "Close the config popup"),
        Line::from(""),
        help_header("Text fields (filter, rename, job form, config paths)"),
        help_row("←/→", "Move the cursor one character"),
        help_row("Ctrl+←/→", "Move the cursor one word"),
        help_row("Home/End", "Jump to the start/end of the line"),
        help_row("Backspace/Del", "Delete before/after the cursor"),
        help_row("Ctrl+W", "Delete the previous word"),
        help_row("Ctrl+U / Ctrl+K", "Clear the line / delete to end of line"),
        Line::from(""),
        help_header("Directory picker"),
        help_row("j/k  ↑/↓", "Move between entries"),
        help_row("Enter", "Open a directory (or pick a file, in file mode)"),
        help_row("Space", "Select the current directory / highlighted file"),
        help_row("/", "Type a path by hand (Enter accepts it)"),
        help_row("←/→  Home/End", "Move the cursor while typing a path"),
        help_row("Esc", "Close the typed path, then the picker"),
        Line::from(""),
        Line::from(Span::styled(
            "  https://github.com/faulker/ccsm",
            Style::default().fg(FG_SUBTEXT),
        )),
    ]
}
