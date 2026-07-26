use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};
use unicode_width::UnicodeWidthStr;

use crate::app::{App, MainTab};
use crate::update::UpdateStatus;
use crate::theme::{
    ACCENT_AMBER, ACCENT_BLUE, ACCENT_GREEN, ACCENT_PEACH, ACCENT_RED, FG_OVERLAY, FG_SUBTEXT,
    FG_TEXT, HIGHLIGHT_BG,
};

use super::util::{activity_count_spans, input_spans};

/// A watcher heartbeat older than this is considered dead. Independent of
/// (and slightly more lenient than) the daemon's own internal freshness
/// check in `watch.rs`: this one only drives a display-only staleness hint.
const WATCHER_HEARTBEAT_STALE_MS: i64 = 20_000;

/// Current wall-clock time in epoch milliseconds.
fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// True when there is no watch daemon state, or its heartbeat hasn't been
/// refreshed recently enough to trust it as still running.
fn watcher_is_stale(app: &App) -> bool {
    match &app.watch_state {
        None => true,
        Some(state) => now_ms() - state.heartbeat_ms >= WATCHER_HEARTBEAT_STALE_MS,
    }
}

/// Format the time remaining until a usage window resets as a short duration
/// like `"1h12m"` or `"45m"`. Returns `None` for a reset time in the past.
fn format_reset_duration(reset_at_ms: i64) -> Option<String> {
    let diff_ms = reset_at_ms - now_ms();
    if diff_ms <= 0 {
        return None;
    }
    let total_minutes = diff_ms / 60_000;
    let hours = total_minutes / 60;
    let minutes = total_minutes % 60;
    if hours > 0 {
        Some(format!("{}h{}m", hours, minutes))
    } else {
        Some(format!("{}m", minutes.max(1)))
    }
}

/// Build the ambient scheduler status chip shown in the tab bar, so current
/// usage is visible from every tab: `⏱ 78% · resets 1h12m`. Returns an empty
/// vec when there are no jobs and the watcher has never run. Shows `⏱ off` in
/// red when jobs exist but the watcher heartbeat is stale — a silently dead
/// watcher is the worst failure mode and must always be visible.
pub(crate) fn build_usage_status_spans(app: &App) -> Vec<Span<'static>> {
    let job_count = app.jobs.len();
    if job_count == 0 && app.watch_state.is_none() {
        return Vec::new();
    }

    let mut spans = vec![Span::styled(" ⏱ ", Style::default().fg(FG_SUBTEXT))];

    if job_count > 0 && watcher_is_stale(app) {
        spans.push(Span::styled(
            "off",
            Style::default().fg(ACCENT_RED).add_modifier(Modifier::BOLD),
        ));
        return spans;
    }

    match app.watch_state.as_ref().and_then(|s| s.last_usage_pct) {
        Some(pct) => {
            let stale = app
                .watch_state
                .as_ref()
                .and_then(|s| s.last_usage_at_ms)
                .map(|at| now_ms() - at >= (app.config.usage_max_age_seconds as i64) * 1000)
                .unwrap_or(true);
            let style = if stale {
                Style::default().fg(FG_SUBTEXT)
            } else if pct >= app.config.usage_pause_percent {
                Style::default().fg(ACCENT_RED).add_modifier(Modifier::BOLD)
            } else if pct >= app.config.usage_resume_percent {
                Style::default().fg(ACCENT_AMBER).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(ACCENT_GREEN).add_modifier(Modifier::BOLD)
            };
            let suffix = if stale { "?" } else { "" };
            spans.push(Span::styled(format!("{:.0}%{}", pct, suffix), style));
        }
        None => {
            spans.push(Span::styled("?%", Style::default().fg(FG_SUBTEXT)));
        }
    }

    if let Some(reset_str) = app
        .watch_state
        .as_ref()
        .and_then(|s| s.reset_at_ms)
        .and_then(format_reset_duration)
    {
        spans.push(Span::styled(
            format!(" · resets {}", reset_str),
            Style::default().fg(FG_SUBTEXT),
        ));
    }

    spans
}

/// Render the session list title spans (shown in the list block border).
pub fn build_title_spans(app: &App) -> Vec<Span<'static>> {
    let title_style = Style::default().fg(ACCENT_BLUE).add_modifier(Modifier::BOLD);

    let mut title_spans = vec![Span::styled(" Sessions ", title_style)];
    if !app.hide_empty {
        title_spans.push(Span::styled(" [showing empty]", title_style));
    }
    if !app.group_chains {
        title_spans.push(Span::styled(" [ungrouped]", title_style));
    }
    if let Some(p) = &app.filter_path {
        title_spans.push(Span::styled(format!(" ({})", p), title_style));
    }
    let (active, idle, waiting) = app.total_activity_counts();
    title_spans.extend(activity_count_spans(active, idle, waiting));
    if app.live_filter {
        title_spans.push(Span::styled(" [live only]", Style::default().fg(ACCENT_GREEN)));
    }
    title_spans.push(Span::styled(" ", title_style));
    title_spans
}

/// Display width of one hint (key span plus label span).
fn hint_width(line: &Line) -> u16 {
    line.spans.iter().map(|s| s.content.width() as u16).sum()
}

/// Lay out a row of key hints across `area`, spreading them with an even gap
/// (capped at 6 columns) and filling the remainder with the bar background.
fn render_hint_row(frame: &mut Frame, hints: &[Line], area: Rect, bar_style: Style) {
    if hints.is_empty() {
        return;
    }
    let hint_widths: Vec<u16> = hints.iter().map(hint_width).collect();
    let total_hint_width: u16 = hint_widths.iter().sum();
    let num_gaps = hints.len().saturating_sub(1) as u16;
    let gap_size = if num_gaps > 0 && area.width > total_hint_width {
        ((area.width - total_hint_width) / num_gaps).min(6)
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
        .split(area);

    for (i, hint) in hints.iter().enumerate() {
        let chunk_idx = i * 2; // each hint is at even indices (0, 2, 4, ...)
        frame.render_widget(
            Paragraph::new(hint.clone()).style(bar_style),
            hint_chunks[chunk_idx],
        );
        // Gap chunks (odd indices) get bar background
        if i > 0 {
            frame.render_widget(
                Paragraph::new("").style(bar_style),
                hint_chunks[chunk_idx - 1],
            );
        }
    }
    // Fill remaining space with bar background
    frame.render_widget(
        Paragraph::new("").style(bar_style),
        hint_chunks[hints.len() * 2 - 1],
    );
}

/// Render the bottom status/help bar.
pub fn render_status_bar(frame: &mut Frame, app: &App, bar_area: Rect) {
    let bar_style = Style::default().bg(HIGHLIGHT_BG);
    let version_label = format!("v{} ", env!("CARGO_PKG_VERSION"));
    let version_width = version_label.len() as u16;
    let bar_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Min(0),
            Constraint::Length(version_width),
        ])
        .split(bar_area);

    if app.filter_active {
        let mut cursor_spans = vec![Span::styled(
            " /",
            Style::default().fg(ACCENT_PEACH).add_modifier(Modifier::BOLD),
        )];
        cursor_spans.extend(input_spans(&app.filter_input, Style::default().fg(FG_TEXT)));
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

        if let Some(err) = &app.status_error {
            hints.push(Line::from(Span::styled(
                format!(" {err} "),
                Style::default()
                    .fg(Color::Rgb(243, 139, 168))
                    .add_modifier(Modifier::BOLD),
            )));
        }

        if app.main_tab == MainTab::Jobs {
            for (k, label) in [
                ("↑↓/jk", " navigate"),
                ("Enter", " attach"),
                ("n", " new"),
                ("e", " edit"),
                ("p", " pause"),
                ("c", " resume"),
                ("x", " stop"),
                ("d", " delete"),
                ("Space", " auto-resume"),
                ("s", " watcher"),
                ("L", " watcher log"),
                ("Tab", " sessions"),
                ("?", " help"),
            ] {
                hints.push(Line::from(vec![
                    Span::styled(k, key_style),
                    Span::styled(label, hint_style),
                ]));
            }
            render_hint_row(frame, &hints, bar_chunks[0], bar_style);
            frame.render_widget(
                Paragraph::new(Span::styled(version_label, Style::default().fg(FG_OVERLAY)))
                    .style(bar_style)
                    .alignment(ratatui::layout::Alignment::Right),
                bar_chunks[1],
            );
            return;
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
            Span::styled("w", key_style),
            Span::styled(" jobs", hint_style),
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
            Span::styled("D", shift_key_style),
            Span::styled(" new dangerous", shift_hint_style),
        ]));
        hints.push(Line::from(vec![
            Span::styled("b", key_style),
            Span::styled(" browse", hint_style),
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

        render_hint_row(frame, &hints, bar_chunks[0], bar_style);
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::schedule::store::WatchState;

    /// A live watcher heartbeat with a fresh usage sample at `pct`.
    fn watch_state(pct: Option<f64>) -> WatchState {
        let now = now_ms();
        WatchState {
            pid: 1234,
            started_at_ms: now,
            heartbeat_ms: now,
            last_usage_pct: pct,
            last_usage_at_ms: pct.map(|_| now),
            reset_at_ms: Some(now + 72 * 60_000),
            usage_error: None,
        }
    }

    /// `App::new` calls `reload_schedule`, which reads the real ccsm dir, so
    /// clear whatever this machine happens to have before asserting.
    fn bare_app() -> App {
        let mut app = App::new(vec![], None, Config::default());
        app.jobs = vec![];
        app.watch_state = None;
        app
    }

    fn chip_text(app: &App) -> String {
        build_usage_status_spans(app)
            .iter()
            .map(|s| s.content.as_ref())
            .collect()
    }

    #[test]
    fn no_chip_without_jobs_or_a_watcher() {
        let app = bare_app();
        assert!(build_usage_status_spans(&app).is_empty());
    }

    #[test]
    fn chip_shows_usage_and_reset_but_not_the_job_count() {
        let mut app = bare_app();
        app.watch_state = Some(watch_state(Some(78.0)));
        let text = chip_text(&app);
        assert!(text.contains("78%"), "{text}");
        assert!(text.contains("resets 1h12m"), "{text}");
        // The job count lives in the tab strip, so the chip must not repeat it.
        assert!(!text.contains("job"), "{text}");
    }

    #[test]
    fn chip_marks_a_stale_usage_sample() {
        let mut app = bare_app();
        let mut state = watch_state(Some(50.0));
        state.last_usage_at_ms =
            Some(now_ms() - (app.config.usage_max_age_seconds as i64 + 60) * 1000);
        app.watch_state = Some(state);
        assert!(chip_text(&app).contains("50%?"));
    }

    #[test]
    fn chip_reads_off_when_jobs_exist_but_the_watcher_is_dead() {
        let mut app = bare_app();
        let mut state = watch_state(Some(10.0));
        state.heartbeat_ms = now_ms() - WATCHER_HEARTBEAT_STALE_MS - 1;
        app.watch_state = Some(state);
        app.jobs = vec![serde_json::from_str::<crate::schedule::Job>("{}").unwrap()];
        assert!(chip_text(&app).contains("off"));
    }

    #[test]
    fn session_title_no_longer_carries_the_usage_chip() {
        let mut app = bare_app();
        app.watch_state = Some(watch_state(Some(78.0)));
        let title: String = build_title_spans(&app)
            .iter()
            .map(|s| s.content.as_ref())
            .collect();
        assert!(!title.contains("78%"), "{title}");
    }
}
