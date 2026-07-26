//! The Jobs tab: a peer of the session list in the main window, plus the
//! job create/edit form and the y/n confirmation for destructive actions,
//! which remain modal overlays on top of it.
//!
//! Mirrors `config_popup.rs`'s shape of holding both the `impl App` key
//! handlers and the draw functions in one file.

use crate::app::{App, AppMode, JobConfirmAction, LaunchRequest, MainTab};
use crate::config::PauseMode;
use crate::keys::normalize_key;
use crate::schedule::{Job, JobState};
use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap},
    Frame,
};

use crate::theme::{
    ACCENT_AMBER, ACCENT_BLUE, ACCENT_GREEN, ACCENT_MAUVE, ACCENT_PEACH, ACCENT_RED, BG_SURFACE,
    FG_OVERLAY, FG_SUBTEXT, FG_TEXT, HIGHLIGHT_BG,
};

use super::util::{centered_rect, format_relative_date, input_spans};

/// Maximum row index in the job form (0-based); row 8 is the submit action.
const JOB_FORM_MAX_ROW: usize = 8;

/// Job-form row index of the working-directory field, which is browsable.
const JOB_FORM_CWD_ROW: usize = 1;

impl App {
    /// Handle a key event while the Jobs tab has focus (Normal mode).
    ///
    /// Handles the global keys the Sessions tab also honours (quit, help,
    /// config, tab switching) so the two tabs feel like one window, then the
    /// job-specific actions.
    pub(crate) fn handle_jobs_tab_event(&mut self, key: crossterm::event::KeyEvent) {
        self.poll_schedule_changed();
        match (key.code, key.modifiers) {
            (KeyCode::Char('c'), KeyModifiers::CONTROL) | (KeyCode::Char('q'), _) => {
                self.should_quit = true;
            }
            (KeyCode::Esc, _) => self.open_sessions_tab(),
            (KeyCode::Tab, _) => self.cycle_main_tab(true),
            (KeyCode::BackTab, _) => self.cycle_main_tab(false),
            (KeyCode::Char('?'), _) | (KeyCode::Char('/'), KeyModifiers::SHIFT) => {
                self.open_help();
            }
            (KeyCode::Char('o'), KeyModifiers::NONE) => {
                self.config_selected = 0;
                self.mode = AppMode::Config;
            }
            (KeyCode::Char('j'), _) | (KeyCode::Down, _) => self.jobs_move_down(),
            (KeyCode::Char('k'), _) | (KeyCode::Up, _) => self.jobs_move_up(),
            (KeyCode::Enter, _) => {
                if let Some(tmux_name) = self.selected_job().and_then(|j| j.tmux_name.clone()) {
                    self.launch_session = Some(LaunchRequest::AttachLive { tmux_name });
                }
            }
            (KeyCode::Char('n'), _) => {
                if !self.missing_usage {
                    self.open_job_form_new();
                }
            }
            (KeyCode::Char('e'), _) => {
                if !self.missing_usage {
                    self.open_job_form_edit();
                }
            }
            (KeyCode::Char('p'), _) => self.pause_selected_job(),
            (KeyCode::Char('c'), _) => self.resume_selected_job(),
            (KeyCode::Char('x'), _) => self.open_job_confirm(JobConfirmAction::Stop),
            (KeyCode::Char('d'), _) => self.open_job_confirm(JobConfirmAction::Delete),
            (KeyCode::Char(' '), _) => self.toggle_selected_job_auto_resume(),
            (KeyCode::Char('s'), _) => self.toggle_watcher(),
            (KeyCode::Char('L'), _) => {
                self.launch_session = Some(LaunchRequest::AttachLive {
                    tmux_name: "ccsm-watch".to_string(),
                });
            }
            _ => {}
        }
    }

    /// Handle a key event while the job create/edit form is open.
    ///
    /// When a text field is being edited, delegates to `job_form_input` via
    /// `tui_input` (so Left/Right/Home/End/Delete all work mid-string);
    /// otherwise `j`/`k`/`Tab` move between fields, `Enter` edits a text field
    /// or toggles a bool/enum field, `b` browses for the working directory, and
    /// `Esc` returns to the Jobs tab (never to the Sessions tab — the form is
    /// only ever reached from Jobs).
    pub(crate) fn handle_job_form_event(&mut self, key: crossterm::event::KeyEvent) {
        if self.job_form_editing {
            use crossterm::event::Event;
            use tui_input::backend::crossterm::EventHandler;
            match key.code {
                KeyCode::Esc => {
                    self.job_form_editing = false;
                    self.job_form_input = tui_input::Input::default();
                }
                KeyCode::Enter => {
                    let value = self.job_form_input.value().to_string();
                    match self.job_form_field {
                        0 => self.job_form_name = value,
                        1 => self.job_form_cwd = value,
                        2 => self.job_form_prompt = value,
                        3 => self.job_form_continue_prompt = value,
                        4 => self.job_form_model = value,
                        _ => {}
                    }
                    self.job_form_editing = false;
                    self.job_form_input = tui_input::Input::default();
                }
                _ => {
                    self.job_form_input.handle_event(&Event::Key(normalize_key(key)));
                }
            }
            return;
        }

        match key.code {
            KeyCode::Esc => {
                self.mode = AppMode::Normal;
                self.main_tab = MainTab::Jobs;
            }
            KeyCode::Char('j') | KeyCode::Down | KeyCode::Tab => {
                if self.job_form_field < JOB_FORM_MAX_ROW {
                    self.job_form_field += 1;
                }
            }
            KeyCode::Char('k') | KeyCode::Up | KeyCode::BackTab => {
                self.job_form_field = self.job_form_field.saturating_sub(1);
            }
            KeyCode::Char('b') if self.job_form_field == JOB_FORM_CWD_ROW => {
                self.browse_job_cwd();
            }
            KeyCode::Enter => match self.job_form_field {
                JOB_FORM_CWD_ROW => self.browse_job_cwd(),
                0 | 2..=4 => {
                    let current = match self.job_form_field {
                        0 => self.job_form_name.clone(),
                        2 => self.job_form_prompt.clone(),
                        3 => self.job_form_continue_prompt.clone(),
                        4 => self.job_form_model.clone(),
                        _ => unreachable!(),
                    };
                    self.job_form_input = tui_input::Input::from(current);
                    self.job_form_editing = true;
                }
                5 => self.job_form_dangerous = !self.job_form_dangerous,
                6 => {
                    self.job_form_pause_mode = match self.job_form_pause_mode {
                        PauseMode::Soft => PauseMode::Hard,
                        PauseMode::Hard => PauseMode::Soft,
                    };
                }
                7 => self.job_form_auto_resume = !self.job_form_auto_resume,
                8 => self.submit_job_form(),
                _ => {}
            },
            KeyCode::Char('i') if self.job_form_field == JOB_FORM_CWD_ROW => {
                // Type the directory by hand instead of browsing for it.
                self.job_form_input = tui_input::Input::from(self.job_form_cwd.clone());
                self.job_form_editing = true;
            }
            KeyCode::Char(' ') => match self.job_form_field {
                5 => self.job_form_dangerous = !self.job_form_dangerous,
                7 => self.job_form_auto_resume = !self.job_form_auto_resume,
                _ => {}
            },
            _ => {}
        }
    }

    /// Handle a key event while the job stop/delete confirmation is open.
    pub(crate) fn handle_job_confirm_event(&mut self, key: crossterm::event::KeyEvent) {
        match key.code {
            KeyCode::Char('y') | KeyCode::Enter => self.confirm_job_action(),
            KeyCode::Char('n') | KeyCode::Esc => self.cancel_job_confirm(),
            _ => {}
        }
    }
}

/// Colour a job state so the list scans at a glance: running/done green,
/// paused/blocked amber, failed red, everything else neutral.
fn state_style(state: JobState) -> Style {
    match state {
        JobState::Running | JobState::Done => Style::default().fg(ACCENT_GREEN),
        JobState::Paused | JobState::Pausing | JobState::Blocked => Style::default().fg(ACCENT_AMBER),
        JobState::Failed => Style::default().fg(ACCENT_RED),
        JobState::Starting | JobState::Resuming => Style::default().fg(ACCENT_BLUE),
        JobState::Queued | JobState::Stopped => Style::default().fg(FG_SUBTEXT),
    }
}

/// Short lowercase label for a job state.
fn state_label(state: JobState) -> &'static str {
    match state {
        JobState::Queued => "queued",
        JobState::Starting => "starting",
        JobState::Running => "running",
        JobState::Pausing => "pausing",
        JobState::Paused => "paused",
        JobState::Resuming => "resuming",
        JobState::Stopped => "stopped",
        JobState::Blocked => "blocked",
        JobState::Done => "done",
        JobState::Failed => "failed",
    }
}

/// Render the Jobs tab into the main window's two panes: the job list on the
/// left and the selected job's detail on the right.
pub(crate) fn draw_jobs_tab(frame: &mut Frame, app: &App, list_area: Rect, right_area: Rect) {
    draw_jobs_list(frame, app, list_area);

    let right_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(vec![Constraint::Length(3), Constraint::Min(3)])
        .split(right_area);
    draw_job_info_bar(frame, app, right_chunks[0]);
    draw_job_detail(frame, app, right_chunks[1]);
}

/// Render the left-hand job list, including the watcher-state banner lines that
/// explain why nothing is progressing when the daemon is not running.
fn draw_jobs_list(frame: &mut Frame, app: &App, area: Rect) {
    let watcher_ok = app.watch_running;
    let border_color = if watcher_ok { ACCENT_PEACH } else { ACCENT_RED };

    let mut title_spans = vec![Span::styled(
        " Jobs ",
        Style::default().fg(ACCENT_PEACH).add_modifier(Modifier::BOLD),
    )];
    title_spans.push(Span::styled(
        format!("({}) ", app.jobs.len()),
        Style::default().fg(FG_SUBTEXT),
    ));
    title_spans.push(if watcher_ok {
        Span::styled("● watcher on ", Style::default().fg(ACCENT_GREEN))
    } else {
        Span::styled(
            "● watcher off ",
            Style::default().fg(ACCENT_RED).add_modifier(Modifier::BOLD),
        )
    });

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(border_color))
        .title(Line::from(title_spans))
        .style(Style::default().bg(BG_SURFACE));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let mut banner: Vec<Line> = Vec::new();
    if app.missing_usage {
        banner.push(Line::from(Span::styled(
            " claude-usage not found — set its path in Config (o)",
            Style::default().fg(ACCENT_RED).add_modifier(Modifier::BOLD),
        )));
    }
    if !watcher_ok {
        let pending = app.pending_command_count();
        let msg = if pending > 0 {
            format!(" watcher stopped — {pending} pending command(s). Press s to start it.")
        } else {
            " watcher stopped — jobs will not run. Press s to start it.".to_string()
        };
        banner.push(Line::from(Span::styled(
            msg,
            Style::default().fg(ACCENT_RED).add_modifier(Modifier::BOLD),
        )));
    }

    let banner_height = banner.len().min(inner.height as usize) as u16;
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(banner_height), Constraint::Min(0)])
        .split(inner);

    if banner_height > 0 {
        frame.render_widget(
            Paragraph::new(banner).wrap(Wrap { trim: false }),
            chunks[0],
        );
    }

    let list_area = chunks[1];
    if app.jobs.is_empty() {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                " No jobs yet. Press n to create one.",
                Style::default().fg(FG_SUBTEXT),
            )))
            .wrap(Wrap { trim: false }),
            list_area,
        );
        return;
    }

    // Windowed manually (rather than via `ListState`'s own scrolling) so the
    // offset maths stays a pure, unit-tested function.
    let visible = (list_area.height as usize).max(1);
    let start = jobs_scroll_offset(app.jobs_selected, app.jobs.len(), visible);
    let end = (start + visible).min(app.jobs.len());

    let items: Vec<ListItem> = app
        .jobs
        .iter()
        .enumerate()
        .take(end)
        .skip(start)
        .map(|(i, job)| {
            let selected = i == app.jobs_selected;
            let name_style = if selected {
                Style::default().fg(ACCENT_BLUE).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(FG_TEXT)
            };
            let auto = if job.auto_resume { "auto" } else { "manual" };
            ListItem::new(Line::from(vec![
                Span::styled(truncate_name(&job.name, 18), name_style),
                Span::raw(" "),
                Span::styled(state_label(job.state), state_style(job.state)),
                Span::styled(format!(" · {auto}"), Style::default().fg(FG_OVERLAY)),
            ]))
        })
        .collect();

    let list = List::new(items)
        .highlight_style(Style::default().bg(HIGHLIGHT_BG))
        .highlight_symbol("▶ ");

    let mut state = ListState::default();
    state.select(Some(app.jobs_selected.saturating_sub(start)));
    frame.render_stateful_widget(list, list_area, &mut state);
}

/// Pad or truncate a job name to a fixed column width.
fn truncate_name(name: &str, width: usize) -> String {
    let chars: Vec<char> = name.chars().collect();
    if chars.len() > width {
        let mut s: String = chars.into_iter().take(width.saturating_sub(1)).collect();
        s.push('…');
        s
    } else {
        format!("{:<width$}", name, width = width)
    }
}

/// Render the info bar above the job detail pane: name, state, and directory.
fn draw_job_info_bar(frame: &mut Frame, app: &App, area: Rect) {
    let mut spans: Vec<Span> = Vec::new();
    if let Some(job) = app.selected_job() {
        spans.push(Span::styled(
            job.name.clone(),
            Style::default().fg(ACCENT_PEACH).add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::raw("  "));
        spans.push(Span::styled(
            state_label(job.state),
            state_style(job.state).add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::raw("  "));
        spans.push(Span::styled(" ", Style::default().fg(FG_OVERLAY)));
        spans.push(Span::styled(job.cwd.clone(), Style::default().fg(FG_SUBTEXT)));
    } else {
        spans.push(Span::styled(
            "No job selected",
            Style::default().fg(FG_SUBTEXT),
        ));
    }

    let info_bar = Paragraph::new(Line::from(spans)).block(
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(FG_OVERLAY))
            .style(Style::default().bg(BG_SURFACE)),
    );
    frame.render_widget(info_bar, area);
}

/// Render the selected job's full detail: configuration, timing, last error,
/// and the tail of its state history.
fn draw_job_detail(frame: &mut Frame, app: &App, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(FG_OVERLAY))
        .title(Span::styled(" Job ", Style::default().fg(FG_SUBTEXT)))
        .style(Style::default().bg(BG_SURFACE));

    let content: Vec<Line> = match app.selected_job() {
        Some(job) => job_detail_lines(job),
        None => vec![
            Line::from(""),
            Line::from(Span::styled(
                "  Press n to create a job, or m on a session in the Sessions tab.",
                Style::default().fg(FG_SUBTEXT),
            )),
        ],
    };

    frame.render_widget(
        Paragraph::new(content).block(block).wrap(Wrap { trim: false }),
        area,
    );
}

/// Current wall-clock time in epoch milliseconds.
fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Format a scheduled-for-the-future timestamp as `"in 1h12m"`. Times that have
/// already passed fall back to the relative-past wording, so a stale
/// `resume_after` never reads as if it were still pending.
fn format_eta(at_ms: i64, now: i64) -> String {
    let diff_ms = at_ms - now;
    if diff_ms <= 0 {
        return format_relative_date(at_ms);
    }
    let total_minutes = diff_ms / 60_000;
    let hours = total_minutes / 60;
    let minutes = total_minutes % 60;
    if hours > 0 {
        format!("in {}h{}m", hours, minutes)
    } else {
        format!("in {}m", minutes.max(1))
    }
}

/// Build the detail-pane lines for one job.
fn job_detail_lines(job: &Job) -> Vec<Line<'static>> {
    let label = Style::default().fg(FG_SUBTEXT);
    let value = Style::default().fg(FG_TEXT);
    let header = Style::default().fg(ACCENT_MAUVE).add_modifier(Modifier::BOLD);

    let field = |name: &str, val: String| -> Line<'static> {
        Line::from(vec![
            Span::styled(format!("  {:<18}", name), label),
            Span::styled(val, value),
        ])
    };

    let mut lines: Vec<Line> = vec![Line::from("")];
    lines.push(field("Directory", job.cwd.clone()));
    lines.push(field(
        "Prompt",
        if job.prompt.is_empty() { "(none)".to_string() } else { job.prompt.clone() },
    ));
    lines.push(field(
        "Continue prompt",
        job.continue_prompt.clone().unwrap_or_else(|| "(default)".to_string()),
    ));
    lines.push(field(
        "Model",
        job.model.clone().unwrap_or_else(|| "(claude default)".to_string()),
    ));
    lines.push(field("Pause mode", job.pause_mode.label().to_string()));
    lines.push(field(
        "Auto-resume",
        if job.auto_resume { "on".to_string() } else { "off".to_string() },
    ));
    lines.push(field(
        "Skip permissions",
        if job.dangerous { "yes".to_string() } else { "no".to_string() },
    ));
    lines.push(field(
        "tmux session",
        job.tmux_name.clone().unwrap_or_else(|| "(not running)".to_string()),
    ));
    lines.push(field(
        "Claude session",
        job.claude_session_id
            .as_ref()
            .map(|id| id.chars().take(8).collect::<String>())
            .unwrap_or_else(|| "(unknown)".to_string()),
    ));
    lines.push(field("Created", format_relative_date(job.created_at_ms)));
    lines.push(field("Updated", format_relative_date(job.updated_at_ms)));
    if let Some(paused) = job.paused_at_ms {
        lines.push(field("Paused", format_relative_date(paused)));
    }
    if let Some(resume_after) = job.resume_after_ms {
        lines.push(field("Resume after", format_eta(resume_after, now_ms())));
    }
    if job.attempts > 0 {
        lines.push(field("Failed attempts", job.attempts.to_string()));
    }

    if let Some(err) = &job.last_error {
        lines.push(Line::from(""));
        lines.push(Line::from(vec![
            Span::styled("  Last error: ", Style::default().fg(ACCENT_RED).add_modifier(Modifier::BOLD)),
            Span::styled(err.clone(), Style::default().fg(ACCENT_RED)),
        ]));
    }

    if !job.history.is_empty() {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled("  History", header)));
        for event in job.history.iter().rev().take(8) {
            lines.push(Line::from(vec![
                Span::styled(format!("  {:<14}", format_relative_date(event.at_ms)), label),
                Span::styled(state_label(event.from), state_style(event.from)),
                Span::styled(" → ", Style::default().fg(FG_OVERLAY)),
                Span::styled(state_label(event.to), state_style(event.to)),
                Span::styled(
                    if event.reason.is_empty() {
                        String::new()
                    } else {
                        format!("  {}", event.reason)
                    },
                    Style::default().fg(FG_SUBTEXT),
                ),
            ]));
        }
    }

    lines
}

/// Render the job create/edit form.
pub(crate) fn draw_job_form_popup(frame: &mut Frame, app: &App) {
    let area = centered_rect(60, 70, frame.area());
    frame.render_widget(Clear, area);

    let selected = app.job_form_field;
    let key_style = Style::default().fg(ACCENT_PEACH).add_modifier(Modifier::BOLD);
    let hint_style = Style::default().fg(FG_SUBTEXT);

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

    // One text field row: renders the live input (with a real cursor) while
    // this field is being edited, otherwise its stored value.
    let text_row = |idx: usize, name: &str, value: &str| -> Line<'static> {
        let style = row_style(idx);
        let mut spans = vec![Span::styled(
            format!("{}{}: ", marker(idx), name),
            style,
        )];
        if app.job_form_editing && selected == idx {
            spans.extend(input_spans(&app.job_form_input, style));
        } else if value.is_empty() {
            spans.push(Span::styled("(empty)".to_string(), style));
        } else {
            spans.push(Span::styled(value.to_string(), style));
        }
        Line::from(spans)
    };

    let plain_row = |idx: usize, text: String| -> Line<'static> {
        Line::from(Span::styled(format!("{}{}", marker(idx), text), row_style(idx)))
    };

    let mut content: Vec<Line> = Vec::new();
    content.push(Line::from(""));
    content.push(text_row(0, "Name", &app.job_form_name));
    content.push(text_row(1, "Directory", &app.job_form_cwd));
    content.push(text_row(2, "Prompt", &app.job_form_prompt));
    content.push(text_row(3, "Continue prompt", &app.job_form_continue_prompt));
    content.push(text_row(4, "Model", &app.job_form_model));
    content.push(plain_row(
        5,
        format!("[{}] Skip permissions", if app.job_form_dangerous { "x" } else { " " }),
    ));
    content.push(plain_row(6, format!("Pause mode: {}", app.job_form_pause_mode.label())));
    content.push(plain_row(
        7,
        format!("[{}] Auto-resume", if app.job_form_auto_resume { "x" } else { " " }),
    ));
    content.push(Line::from(""));
    let action_label = if app.job_form_edit_id.is_some() {
        "[ Save job ]"
    } else {
        "[ Create job ]"
    };
    content.push(plain_row(8, action_label.to_string()));
    content.push(Line::from(""));

    if app.job_form_editing {
        content.push(Line::from(vec![
            Span::styled("  Enter", key_style),
            Span::styled(" save field  ", hint_style),
            Span::styled("←/→", key_style),
            Span::styled(" move cursor  ", hint_style),
            Span::styled("Esc", key_style),
            Span::styled(" cancel edit", hint_style),
        ]));
    } else if selected == JOB_FORM_CWD_ROW {
        content.push(Line::from(vec![
            Span::styled("  Enter/b", key_style),
            Span::styled(" browse  ", hint_style),
            Span::styled("i", key_style),
            Span::styled(" type path  ", hint_style),
            Span::styled("j/k", key_style),
            Span::styled(" navigate  ", hint_style),
            Span::styled("Esc", key_style),
            Span::styled(" back", hint_style),
        ]));
    } else {
        content.push(Line::from(vec![
            Span::styled("  j/k", key_style),
            Span::styled(" navigate  ", hint_style),
            Span::styled("Enter", key_style),
            Span::styled(" edit/toggle  ", hint_style),
            Span::styled("Esc", key_style),
            Span::styled(" back", hint_style),
        ]));
    }

    let title = if app.job_form_edit_id.is_some() {
        " Edit Job "
    } else {
        " New Job "
    };
    let popup = Paragraph::new(content).block(
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
    frame.render_widget(popup, area);
}

/// Render the job stop/delete confirmation prompt.
pub(crate) fn draw_job_confirm_popup(frame: &mut Frame, app: &App) {
    let Some((job_id, action)) = &app.jobs_confirm else {
        return;
    };
    let job_name = app
        .jobs
        .iter()
        .find(|j| &j.id == job_id)
        .map(|j| j.name.clone())
        .unwrap_or_default();

    let area = centered_rect(44, 20, frame.area());
    let area = if area.height < 7 { Rect { height: 7, ..area } } else { area };
    frame.render_widget(Clear, area);

    let key_style = Style::default().fg(ACCENT_PEACH).add_modifier(Modifier::BOLD);
    let text_style = Style::default().fg(FG_TEXT);
    let dim_style = Style::default().fg(FG_SUBTEXT);

    let verb = match action {
        JobConfirmAction::Stop => "Stop",
        JobConfirmAction::Delete => "Delete",
    };

    let content = vec![
        Line::from(""),
        Line::from(vec![
            Span::styled(format!("{} ", verb), Style::default().fg(ACCENT_RED).add_modifier(Modifier::BOLD)),
            Span::styled(format!("\"{}\"", job_name), Style::default().fg(ACCENT_PEACH).add_modifier(Modifier::BOLD)),
            Span::styled("?", text_style),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("  y", key_style),
            Span::styled(" / Enter  confirm", text_style),
        ]),
        Line::from(vec![
            Span::styled("  n", key_style),
            Span::styled(" / Esc     cancel", dim_style),
        ]),
    ];

    let popup = Paragraph::new(content).block(
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(ACCENT_RED))
            .title(Span::styled(
                " Confirm ",
                Style::default().fg(ACCENT_RED).add_modifier(Modifier::BOLD),
            ))
            .style(Style::default().bg(BG_SURFACE)),
    );
    frame.render_widget(popup, area);
}

/// First visible row index for a windowed list, keeping `selected` on screen.
/// Pure so the scrolling maths is testable without a terminal.
fn jobs_scroll_offset(selected: usize, total: usize, visible: usize) -> usize {
    if total <= visible || visible == 0 {
        return 0;
    }
    let max_start = total - visible;
    if selected < visible / 2 {
        0
    } else {
        (selected - visible / 2).min(max_start)
    }
}

#[cfg(test)]
mod tests {
    use super::{format_eta, jobs_scroll_offset, truncate_name};

    #[test]
    fn no_offset_when_everything_fits() {
        assert_eq!(jobs_scroll_offset(0, 3, 10), 0);
        assert_eq!(jobs_scroll_offset(2, 3, 10), 0);
    }

    #[test]
    fn keeps_selection_visible_and_clamps_at_the_end() {
        // Selection near the top does not scroll.
        assert_eq!(jobs_scroll_offset(1, 100, 10), 0);
        // Selection in the middle centres the window.
        assert_eq!(jobs_scroll_offset(50, 100, 10), 45);
        // Selection at the very end clamps so the window stays full.
        assert_eq!(jobs_scroll_offset(99, 100, 10), 90);
    }

    #[test]
    fn degenerate_inputs_do_not_panic() {
        assert_eq!(jobs_scroll_offset(0, 0, 0), 0);
        assert_eq!(jobs_scroll_offset(5, 5, 0), 0);
        assert_eq!(jobs_scroll_offset(0, 10, 1), 0);
    }

    #[test]
    fn format_eta_counts_down_to_future_times() {
        let now = 1_700_000_000_000;
        assert_eq!(format_eta(now + 72 * 60_000, now), "in 1h12m");
        assert_eq!(format_eta(now + 5 * 60_000, now), "in 5m");
        // Sub-minute futures still read as pending, never "0m".
        assert_eq!(format_eta(now + 5_000, now), "in 1m");
        // A past time falls back to the relative-past formatter, never a countdown.
        assert!(!format_eta(now - 60_000, now).starts_with("in "));
    }

    #[test]
    fn truncate_name_pads_short_names_and_ellipsizes_long_ones() {
        assert_eq!(truncate_name("job", 6), "job   ");
        assert_eq!(truncate_name("a-very-long-job", 6), "a-ver…");
        assert_eq!(truncate_name("exactly", 7), "exactly");
    }
}
