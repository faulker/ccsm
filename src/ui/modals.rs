use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, BorderType, Borders, Clear, Paragraph, Wrap},
    Frame,
};

use crate::app::{HelpTab, NewSessionMode};
use crate::theme::{
    ACCENT_BLUE, ACCENT_GREEN, ACCENT_MAUVE, ACCENT_PEACH, ACCENT_RED, BG_SURFACE, FG_OVERLAY,
    FG_SUBTEXT, FG_TEXT,
};

use super::util::{centered_rect_min, input_spans, input_spans_with_placeholder};

/// Render the centered popup for naming a new live session, showing a placeholder when the buffer is empty.
///
/// The popup owns the launch mode: a mode row lists plain / danger / worktree /
/// direct, Tab cycles between them, and the title and border colour follow the
/// selection. `cwd_is_repo` dims `worktree` when it cannot apply.
pub fn draw_naming_popup(
    frame: &mut Frame,
    input: &tui_input::Input,
    placeholder: &str,
    mode: NewSessionMode,
    cwd_is_repo: bool,
) {
    // Two content rows now (the name and the mode row) plus the border. The
    // minimum width is set by the mode row, which must show all four labels:
    // a popup that clips "direct" off the end is worse than no mode row.
    let area = centered_rect_min(56, 4, 48, 4, frame.area());
    frame.render_widget(Clear, area);

    let name_line = if mode.needs_name() {
        Line::from(input_spans_with_placeholder(
            input,
            Style::default().fg(FG_TEXT),
            placeholder,
        ))
    } else {
        // A direct session never reaches tmux, so it has no name to give.
        Line::from(Span::styled(
            "(no tmux session — name unused)",
            Style::default().fg(FG_OVERLAY),
        ))
    };

    // The mode row doubles as the list of choices, so the available modes are
    // always visible rather than being something you have to Tab through blind.
    let mut mode_spans: Vec<Span> = vec![Span::styled("  ", Style::default())];
    for m in NewSessionMode::ALL {
        let selectable = m != NewSessionMode::Worktree || cwd_is_repo;
        let style = if m == mode {
            Style::default()
                .fg(ACCENT_PEACH)
                .add_modifier(Modifier::BOLD | Modifier::REVERSED)
        } else if selectable {
            Style::default().fg(FG_SUBTEXT)
        } else {
            Style::default().fg(FG_OVERLAY).add_modifier(Modifier::DIM)
        };
        mode_spans.push(Span::styled(format!(" {} ", m.label()), style));
        mode_spans.push(Span::raw(" "));
    }
    mode_spans.push(Span::styled("Tab", Style::default().fg(FG_OVERLAY)));

    let content = vec![name_line, Line::from(mode_spans)];

    let (title, border_color) = match mode {
        NewSessionMode::Dangerous => {
            (" New Session ⚠ skip-permissions (Esc cancels) ", ACCENT_RED)
        }
        NewSessionMode::Worktree => (" New Session in git worktree (Esc cancels) ", ACCENT_GREEN),
        NewSessionMode::Direct => (" New Session, direct — no tmux (Esc cancels) ", ACCENT_BLUE),
        NewSessionMode::Plain => (" New Session (Esc cancels) ", ACCENT_PEACH),
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
    let area = centered_rect_min(44, 20, 38, 8, frame.area());
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

/// Render the confirmation popup for stopping a live session.
///
/// `x` on the Jobs tab has always confirmed before stopping; this is the
/// Sessions-tab counterpart, so the same key asks the same question in both
/// tabs instead of silently killing a session in one of them.
pub fn draw_stop_confirm_popup(frame: &mut Frame, name: &str) {
    let area = centered_rect_min(44, 18, 38, 7, frame.area());
    frame.render_widget(Clear, area);

    let key_style = Style::default().fg(ACCENT_PEACH).add_modifier(Modifier::BOLD);
    let text_style = Style::default().fg(FG_TEXT);

    let content = vec![
        Line::from(""),
        Line::from(vec![
            Span::styled("Stop session ", text_style),
            Span::styled(
                format!("\"{}\"", name),
                Style::default().fg(ACCENT_PEACH).add_modifier(Modifier::BOLD),
            ),
            Span::styled("?", text_style),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("  y", key_style),
            Span::styled(" / Enter  stop it   ", text_style),
            Span::styled("n", key_style),
            Span::styled(" / Esc  cancel", Style::default().fg(FG_SUBTEXT)),
        ]),
    ];

    let popup = Paragraph::new(content).block(
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(ACCENT_RED))
            .title(Span::styled(
                " Stop Session ",
                Style::default().fg(ACCENT_RED).add_modifier(Modifier::BOLD),
            ))
            .style(Style::default().bg(BG_SURFACE)),
    );
    frame.render_widget(popup, area);
}

/// Render the centered rename popup showing the current `text` and a block cursor.
pub fn draw_rename_popup(frame: &mut Frame, input: &tui_input::Input) {
    let area = centered_rect_min(40, 3, 30, 3, frame.area());
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
    let area = centered_rect_min(40, 15, 36, 6, frame.area());
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
pub fn render_help_popup(frame: &mut Frame, area: Rect, tab: HelpTab, scroll: u16) -> u16 {
    let popup_area = centered_rect_min(70, 80, 44, 12, area);
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

    // Clamp the scroll so the last page of content cannot be scrolled past.
    // The clamped value is returned so the caller can write it back, otherwise
    // holding `j` would run the offset up with nothing appearing to happen.
    let view_height = chunks[1].height;
    let max_scroll = (lines.len() as u16).saturating_sub(view_height);
    let scroll = scroll.min(max_scroll);

    frame.render_widget(
        Paragraph::new(Text::from(lines))
            .wrap(Wrap { trim: false })
            .scroll((scroll, 0)),
        chunks[1],
    );

    let sub = Style::default().fg(FG_SUBTEXT);
    let key = Style::default().fg(ACCENT_PEACH).add_modifier(Modifier::BOLD);
    let mut footer = vec![
        Span::styled("  Tab", key),
        Span::styled(" page   ", sub),
        Span::styled("jk", key),
        Span::styled(" scroll   ", sub),
        Span::styled("Esc", key),
        Span::styled(" close", sub),
    ];
    if max_scroll > 0 {
        footer.push(Span::styled(
            format!("   {}%", (scroll as u32 * 100 / max_scroll as u32).min(100)),
            Style::default().fg(FG_OVERLAY),
        ));
    }
    frame.render_widget(Paragraph::new(Line::from(footer)), chunks[2]);

    scroll
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
        help_row("n", "New session popup (Tab picks plain/danger/worktree/direct)"),
        help_row("b", "Browse filesystem to choose a directory for a new session"),
        help_row("m", "Create/edit a job prefilled from the current selection"),
        help_row("x", "Stop selected live session (asks first)"),
        help_row("l", "Toggle live-only filter"),
        help_row("v", "Cycle view mode (tree / flat / grouped)"),
        help_row("r", "Rename selected session or live session"),
        help_row("Space", "Toggle favorite — pins project to top of list (★)"),
        help_row("o", "Open the config popup"),
        Line::from(""),
        help_header("New session popup"),
        help_row("Tab", "Cycle launch mode: plain, danger, worktree, direct"),
        help_row("", "  plain     normal live session in tmux"),
        help_row("", "  danger    adds --dangerously-skip-permissions"),
        help_row("", "  worktree  its own git worktree (git repos only)"),
        help_row("", "  direct    no tmux; the name is unused"),
        help_row("Enter", "Launch (an empty name uses the suggested one)"),
        help_row("Esc", "Cancel"),
        Line::from(""),
        help_header("Filter mode"),
        help_row("/", "Enter filter/search mode"),
        help_row("←/→  Home/End", "Move the cursor within the filter text"),
        help_row("Enter", "Confirm filter and return to Normal mode"),
        help_row("Esc", "Clear filter and return to Normal mode"),
        help_row("", "Esc never quits the app — use q or Ctrl+C"),
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
        help_row("f", "Mark job done, ending it for good (with confirmation)"),
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
        help_row("Enter / ←/→", "On Model: cycle the discovered model list"),
        help_row("i", "On Model: type a model id by hand instead"),
        help_row("←/→  Home/End", "Move the cursor while editing a field"),
        help_row("Esc", "Cancel the current edit, then leave the form"),
        help_row("(help)", "The selected field is explained at the bottom of the form"),
        Line::from(""),
        help_header("How the watcher works"),
        help_row("pause", "Jobs pause once usage reaches the configured pause %"),
        help_row("resume", "Auto-resume jobs restart when usage falls or resets"),
        help_row("state", "The tab title shows watcher on/off; off means nothing runs"),
        help_row("done", "Agents end with CCSM_JOB_COMPLETE; that job stops for good"),
        help_row("idle", "A job idle past Config's idle-completion time also finishes"),
    ]
}

/// Help page for global keys, the config popup, and the directory picker.
fn general_help_lines() -> Vec<Line<'static>> {
    vec![
        help_header("Global"),
        help_row("Tab / Shift+Tab", "Switch between the Sessions and Jobs tabs"),
        help_row("o", "Open the config popup"),
        help_row("?", "Open this help"),
        help_row("q / Ctrl+C", "Quit"),
        help_row("Esc", "Back out of a popup. Never quits the app."),
        Line::from(""),
        help_header("This help"),
        help_row("Tab / ←/→", "Switch help page"),
        help_row("1/2/3", "Jump to a page"),
        help_row("j/k  PgUp/PgDn", "Scroll the page"),
        help_row("Esc / ?", "Close"),
        Line::from(""),
        help_header("Confirmations"),
        help_row("y / Enter", "Confirm"),
        help_row("n / Esc", "Cancel"),
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
