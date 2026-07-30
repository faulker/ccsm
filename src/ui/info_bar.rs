use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};
use unicode_width::UnicodeWidthStr;

use crate::app::{App, MainTab, SourceFilter};
use crate::ui::config_tab::{row_action, RowAction};
use crate::update::UpdateStatus;
use crate::theme::{
    ACCENT_AMBER, ACCENT_BLUE, ACCENT_GREEN, ACCENT_PEACH, ACCENT_RED, ACCENT_TEAL, FG_OVERLAY,
    FG_SUBTEXT, FG_TEXT, HIGHLIGHT_BG,
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
/// vec only when there is nothing at all to say: no jobs, no watcher that has
/// ever run, and no usage reading of ccsm's own. Shows `⏱ off` in red when jobs
/// exist but the watcher heartbeat is stale — a silently dead watcher is the
/// worst failure mode and must always be visible.
pub(crate) fn build_usage_status_spans(app: &App) -> Vec<Span<'static>> {
    // Usage windows are Claude-only; hide the chip when browsing Cursor alone.
    if app.source_filter == crate::app::SourceFilter::Cursor {
        return vec![
            Span::styled(" ⏱ ", Style::default().fg(FG_SUBTEXT)),
            Span::styled("Claude-only", Style::default().fg(FG_OVERLAY)),
        ];
    }
    let job_count = app.jobs.len();
    if job_count == 0 && app.watch_state.is_none() && app.usage.is_none() {
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

    let reading = app.usage_reading();
    match reading.pct {
        Some(pct) => {
            let stale = reading
                .sampled_at_ms
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

    if let Some(reset_str) = reading.reset_at_ms.and_then(format_reset_duration) {
        spans.push(Span::styled(
            format!(" · resets {}", reset_str),
            Style::default().fg(FG_SUBTEXT),
        ));
    }

    spans
}

/// Render the session list title spans (shown in the list block border).
///
/// `width` is the list pane's full width. The pane is only 30% of the screen —
/// 18 columns at 60 — so the state badges are spelled out only when they fit
/// and fall back to single glyphs (then drop entirely) as it narrows. The
/// spelled-out state is always available on the Config tab.
pub fn build_title_spans(app: &App, width: u16) -> Vec<Span<'static>> {
    let title_style = Style::default().fg(ACCENT_BLUE).add_modifier(Modifier::BOLD);
    // Two border corners plus a trailing space.
    let budget = width.saturating_sub(3) as usize;

    let mut title_spans = vec![Span::styled(" Sessions ", title_style)];
    let mut used = " Sessions ".width();

    let (active, idle, waiting) = app.total_activity_counts();
    let counts = activity_count_spans(active, idle, waiting);
    let counts_width: usize = counts.iter().map(|s| s.content.width()).sum();

    // Badges for non-default view state. Long form first, then glyphs.
    let mut badges: Vec<(String, String, Style)> = Vec::new();
    if !app.hide_empty {
        badges.push((" [showing empty]".into(), " \u{2205}".into(), title_style));
    }
    if !app.group_chains {
        badges.push((" [ungrouped]".into(), " \u{2261}".into(), title_style));
    }
    if let Some(p) = &app.filter_path {
        badges.push((format!(" ({})", p), " \u{25b8}".into(), title_style));
    }
    if app.live_filter {
        badges.push((
            " [live only]".into(),
            " \u{26a1}".into(),
            Style::default().fg(ACCENT_GREEN),
        ));
    }
    match app.source_filter {
        SourceFilter::Both => {}
        SourceFilter::Claude => badges.push((
            " [claude]".into(),
            " C".into(),
            Style::default().fg(ACCENT_PEACH),
        )),
        SourceFilter::Cursor => badges.push((
            " [cursor]".into(),
            " A".into(),
            Style::default().fg(ACCENT_TEAL),
        )),
    }

    let long_total: usize = badges.iter().map(|(l, _, _)| l.width()).sum();
    let short_total: usize = badges.iter().map(|(_, s, _)| s.width()).sum();
    // Activity counts outrank the badges: they describe live work, not a toggle.
    let long_fits = used + long_total + counts_width <= budget;
    let short_fits = used + short_total + counts_width <= budget;

    for (long, short, style) in &badges {
        if long_fits {
            title_spans.push(Span::styled(long.clone(), *style));
            used += long.width();
        } else if short_fits {
            title_spans.push(Span::styled(short.clone(), *style));
            used += short.width();
        }
    }

    if used + counts_width <= budget {
        title_spans.extend(counts);
    }
    title_spans.push(Span::styled(" ", title_style));
    title_spans
}

/// How important a status-bar hint is. When the bar cannot fit everything,
/// hints are dropped from the highest priority number down.
///
/// `P0` exists so that `? help` and `q quit` can never be dropped: the whole
/// failure mode this replaces was a 60-column terminal showing neither, which
/// left no visible route to the help overlay that documents the rest.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum HintPriority {
    /// Never dropped, at any width.
    P0,
    /// The primary interaction: navigation and the main action.
    P1,
    /// Frequently used actions.
    P2,
    /// Everything else; documented in the help overlay.
    P3,
}

/// One status-bar key hint.
pub(crate) struct Hint {
    pub key: String,
    pub label: String,
    pub priority: HintPriority,
    /// When true, this hint is styled as a Shift-modified binding.
    pub shifted: bool,
}

impl Hint {
    /// Mark this hint as a Shift-modified binding, so it is styled as one.
    fn shifted(mut self) -> Self {
        self.shifted = true;
        self
    }

    fn new(key: &str, label: &str, priority: HintPriority) -> Self {
        Self {
            key: key.to_string(),
            label: label.to_string(),
            priority,
            shifted: false,
        }
    }

    /// Rendered width: key plus label, both measured in display columns.
    fn width(&self) -> u16 {
        (self.key.width() + self.label.width()) as u16
    }
}

/// Marker appended when at least one hint was dropped, so the bar admits it is
/// incomplete instead of silently truncating.
const OVERFLOW_MARKER: &str = "…";

/// What `select_hints` decided to draw.
pub(crate) struct HintLayout {
    /// Indices into the original slice, in the caller's order.
    pub chosen: Vec<usize>,
    /// True when at least one hint was dropped *and* the `…` marker fits.
    pub overflow: bool,
}

/// Choose which hints fit in `width`, in priority order.
///
/// `P0` hints are reserved first and survive at any width that can physically
/// hold them; the remainder of the budget is filled by ascending priority. The
/// `…` marker is budgeted for whenever anything is dropped, so the returned
/// layout never renders wider than `width`.
///
/// Pure and unit-tested: the whole point of this refactor is that "does the bar
/// fit" stops being something you have to eyeball at four terminal sizes.
pub(crate) fn select_hints(hints: &[Hint], width: u16) -> HintLayout {
    let empty = HintLayout { chosen: Vec::new(), overflow: false };
    if hints.is_empty() || width == 0 {
        return empty;
    }
    let gap: u16 = 2;
    let marker_cost = OVERFLOW_MARKER.width() as u16 + gap;

    // Cost of adding one more hint to a set that already has `count` members.
    let cost = |h: &Hint, count: usize| h.width() + if count > 0 { gap } else { 0 };

    let mut chosen: Vec<usize> = Vec::new();
    let mut used: u16 = 0;

    // Reserve the P0 hints first, but still only while they fit: below roughly
    // 15 columns not even "? help  q quit" can be drawn, and emitting spans
    // wider than the area would just hand ratatui a garbled fragment to clip.
    for (i, h) in hints.iter().enumerate() {
        if h.priority == HintPriority::P0 {
            let need = cost(h, chosen.len());
            if used + need <= width {
                used += need;
                chosen.push(i);
            }
        }
    }

    // Fill the rest by priority, then by original order within a priority.
    let mut rest: Vec<usize> = (0..hints.len())
        .filter(|i| hints[*i].priority != HintPriority::P0)
        .collect();
    rest.sort_by_key(|&i| hints[i].priority);

    let mut dropped_any = false;
    for i in rest {
        let need = cost(&hints[i], chosen.len());
        // Once something has been dropped the marker is owed space, so keep
        // room for it before accepting any further hint.
        let reserve = if dropped_any { marker_cost } else { 0 };
        if used + need + reserve <= width {
            used += need;
            chosen.push(i);
        } else {
            dropped_any = true;
        }
    }

    // The marker only earns its space if it actually fits. Evict the
    // lowest-priority non-P0 hints to make room; if even that is not enough
    // (a terminal too narrow for the P0 pair plus `…`), drop the marker rather
    // than overflow the area.
    let mut overflow = false;
    if dropped_any {
        while used + marker_cost > width {
            let Some(pos) = chosen
                .iter()
                .enumerate()
                .filter(|(_, &idx)| hints[idx].priority != HintPriority::P0)
                .max_by_key(|(_, &idx)| hints[idx].priority)
                .map(|(pos, _)| pos)
            else {
                break;
            };
            let removed = chosen.remove(pos);
            used -= cost(&hints[removed], chosen.len());
        }
        overflow = used + marker_cost <= width;
    }

    chosen.sort_unstable();
    HintLayout { chosen, overflow }
}

/// The four styles a hint row draws with.
struct HintStyles {
    bar: Style,
    key: Style,
    label: Style,
    shift_key: Style,
    shift_label: Style,
}

/// Lay out a row of key hints across `area`, dropping the lowest-priority ones
/// that do not fit and marking the overflow with `…`.
fn render_hint_row(frame: &mut Frame, hints: &[Hint], area: Rect, styles: &HintStyles) {
    let HintStyles {
        bar: bar_style,
        key: key_style,
        label: hint_style,
        shift_key: shift_key_style,
        shift_label: shift_hint_style,
    } = *styles;
    if hints.is_empty() || area.width == 0 {
        return;
    }
    frame.render_widget(Paragraph::new("").style(bar_style), area);

    let HintLayout { chosen, overflow } = select_hints(hints, area.width);
    if chosen.is_empty() {
        return;
    }

    let mut spans: Vec<Span> = Vec::new();
    for (n, &i) in chosen.iter().enumerate() {
        if n > 0 {
            spans.push(Span::styled("  ", bar_style));
        }
        let h = &hints[i];
        let (k, l) = if h.shifted {
            (shift_key_style, shift_hint_style)
        } else {
            (key_style, hint_style)
        };
        spans.push(Span::styled(h.key.clone(), k.bg(HIGHLIGHT_BG)));
        spans.push(Span::styled(h.label.clone(), l.bg(HIGHLIGHT_BG)));
    }
    if overflow {
        spans.push(Span::styled("  ", bar_style));
        spans.push(Span::styled(
            OVERFLOW_MARKER,
            Style::default().fg(FG_OVERLAY).bg(HIGHLIGHT_BG),
        ));
    }

    frame.render_widget(Paragraph::new(Line::from(spans)).style(bar_style), area);
}

/// Build the Jobs-tab hints, in display order.
///
/// When the watcher is stopped, `s start` is promoted to P1 so the recovery
/// key survives the same narrow terminals that used to clip the list banner.
pub(crate) fn jobs_hints(watch_running: bool) -> Vec<Hint> {
    use HintPriority::*;
    let watcher = if watch_running {
        Hint::new("s", " watcher", P3)
    } else {
        Hint::new("s", " start", P1)
    };
    vec![
        Hint::new("↑↓/jk", " nav", P1),
        Hint::new("Enter", " attach", P1),
        Hint::new("n", " new", P1),
        Hint::new("e", " edit", P2),
        Hint::new("p", " pause", P2),
        Hint::new("c", " resume", P2),
        Hint::new("x", " stop", P2),
        Hint::new("d", " delete", P3),
        Hint::new("f", " done", P3),
        Hint::new("Space", " auto", P3),
        watcher,
        Hint::new("L", " log", P3),
        Hint::new("Tab", " sessions", P2),
        Hint::new("?", " help", P0),
        Hint::new("q", " quit", P0),
    ]
}

/// Build the Config-tab hints, in display order.
///
/// The primary hint follows the selected row: `i` only does something on a
/// path row, and `Enter` on the URL row opens a browser rather than toggling
/// anything, so a fixed `Space toggle` would be wrong on both.
pub(crate) fn config_hints(action: RowAction) -> Vec<Hint> {
    use HintPriority::*;
    let mut hints = vec![Hint::new("\u{2191}\u{2193}/jk", " nav", P1)];
    match action {
        RowAction::Browse => {
            hints.push(Hint::new("Enter", " browse", P1));
            hints.push(Hint::new("i", " type path", P2));
        }
        RowAction::OpenUrl => hints.push(Hint::new("Enter", " open", P1)),
        RowAction::Edit => hints.push(Hint::new("Enter", " edit", P1)),
        RowAction::Toggle => hints.push(Hint::new("Space/Enter", " toggle", P1)),
    }
    hints.extend([
        Hint::new("Tab", " sessions", P2),
        Hint::new("Esc", " back", P3),
        Hint::new("?", " help", P0),
        Hint::new("q", " quit", P0),
    ]);
    hints
}

/// Build the Sessions-tab hints, in display order.
///
/// `is_live` adds the stop hint only when a live session is selected, since it
/// is the only one that does nothing otherwise. While Shift is held, the two
/// hints that change meaning say so rather than silently doing something else.
pub(crate) fn sessions_hints(
    is_live: bool,
    shift_active: bool,
    is_historical: bool,
    source_filter: SourceFilter,
) -> Vec<Hint> {
    use HintPriority::*;
    let nav = if shift_active {
        Hint::new("↑↓/JK", " scroll", P1).shifted()
    } else {
        Hint::new("↑↓/jk", " nav", P1)
    };
    let open = if shift_active && is_historical {
        Hint::new("Enter", " open direct", P1).shifted()
    } else {
        Hint::new("Enter", " open", P1)
    };
    let source_label = match source_filter {
        SourceFilter::Both => " source",
        SourceFilter::Claude => " claude",
        SourceFilter::Cursor => " cursor",
    };
    let mut hints = vec![
        nav,
        open,
        Hint::new("n", " new", P1),
        Hint::new("/", " search", P2),
        Hint::new("r", " rename", P2),
    ];
    if is_live {
        hints.push(Hint::new("x", " stop", P2));
    }
    hints.extend([
        Hint::new("Tab", " jobs", P2),
        Hint::new("Space", " favorite", P3),
        Hint::new("v", " view", P3),
        Hint::new("b", " browse", P3),
        Hint::new("l", " live only", P3),
        Hint::new("s", source_label, P3),
    ]);
    // Jobs and usage tracking are Claude-only.
    if source_filter != SourceFilter::Cursor {
        hints.push(Hint::new("m", " job", P3));
    }
    hints.extend([
        Hint::new("o", " config", P3),
        Hint::new("?", " help", P0),
        Hint::new("q", " quit", P0),
    ]);
    hints
}

/// Render the bottom status/help bar.
///
/// The version label used to live here, permanently reserving 8 columns at
/// every terminal size. It moved to the Config tab's About block so that
/// space goes to shortcuts instead.
pub fn render_status_bar(frame: &mut Frame, app: &App, bar_area: Rect) {
    let bar_style = Style::default().bg(HIGHLIGHT_BG);
    let bar_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Min(0)])
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
        // The Shift styling only ever applies to hints that are genuinely
        // Shift-modified, and only while Shift is actually held. It used to be
        // applied unconditionally to a few hints, which made them look "lit"
        // forever in terminals that never report a bare modifier press.
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

        // A transient message (update progress or an error) takes the whole
        // bar for as long as it is showing: it matters more than the hints,
        // and interleaving the two is what made the old bar unreadable.
        let banner = match (&app.update_status, &app.status_error) {
            (UpdateStatus::Downloading, _) => Some((
                " Updating... ".to_string(),
                Style::default().fg(ACCENT_PEACH).add_modifier(Modifier::BOLD),
            )),
            (UpdateStatus::Failed(msg), _) => Some((
                format!(" Update failed: {msg} "),
                Style::default()
                    .fg(Color::Rgb(243, 139, 168))
                    .add_modifier(Modifier::BOLD),
            )),
            (_, Some(err)) => Some((
                format!(" {err} "),
                Style::default()
                    .fg(Color::Rgb(243, 139, 168))
                    .add_modifier(Modifier::BOLD),
            )),
            _ => None,
        };
        if let Some((text, style)) = banner {
            frame.render_widget(
                Paragraph::new(Line::from(Span::styled(text, style))).style(bar_style),
                bar_chunks[0],
            );
            return;
        }

        let hints = match app.main_tab {
            MainTab::Jobs => jobs_hints(app.watch_running),
            MainTab::Config => config_hints(row_action(app.config_selected)),
            MainTab::Sessions => sessions_hints(
                app.selected_live_index().is_some(),
                app.shift_active,
                app.is_historical_selected(),
                app.source_filter,
            ),
        };

        render_hint_row(
            frame,
            &hints,
            bar_chunks[0],
            &HintStyles {
                bar: bar_style,
                key: key_style,
                label: hint_style,
                shift_key: shift_key_style,
                shift_label: shift_hint_style,
            },
        );
    }
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
            version: Some(env!("CARGO_PKG_VERSION").to_string()),
        }
    }

    /// `App::new` calls `reload_schedule`, which reads the real ccsm dir, so
    /// clear whatever this machine happens to have before asserting.
    fn bare_app() -> App {
        let mut app = App::new(vec![], None, Config::default());
        app.jobs = vec![];
        app.watch_state = None;
        app.usage = None;
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
    fn chip_shows_our_own_reading_with_no_watcher_at_all() {
        // The whole point of reading usage in-tree: no daemon has ever run, and
        // the chip is still live.
        let mut app = bare_app();
        app.usage = Some(crate::usage::UsageSnapshot {
            sampled_at_ms: Some(now_ms()),
            five_hour: Some(crate::usage::UsageWindow {
                used_percentage: Some(64.0),
                resets_at: None,
                resets_at_estimated_ms: Some(now_ms() + 72 * 60_000),
            }),
            ..Default::default()
        });
        let text = chip_text(&app);
        assert!(text.contains("64%"), "{text}");
        assert!(text.contains("resets 1h12m"), "{text}");
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
        let title: String = build_title_spans(&app, 120)
            .iter()
            .map(|s| s.content.as_ref())
            .collect();
        assert!(!title.contains("78%"), "{title}");
    }

    // --- Responsive status bar ---

    /// Render the status bar at `width` and return its text content.
    fn bar_text(app: &App, width: u16) -> String {
        use ratatui::backend::TestBackend;
        use ratatui::Terminal;

        let mut terminal = Terminal::new(TestBackend::new(width, 3)).unwrap();
        terminal
            .draw(|f| {
                let area = Rect { x: 0, y: 0, width, height: 1 };
                render_status_bar(f, app, area);
            })
            .unwrap();
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .take(width as usize)
            .map(|c| c.symbol())
            .collect()
    }

    /// The failure this whole refactor exists to prevent: a narrow terminal
    /// that shows neither how to get help nor how to quit.
    #[test]
    fn help_and_quit_survive_every_width() {
        let mut app = bare_app();
        for width in [40u16, 60, 80, 100, 120, 200] {
            let text = bar_text(&app, width);
            assert!(text.contains("? help"), "width {width}: {text}");
            assert!(text.contains("q quit"), "width {width}: {text}");
        }
        app.main_tab = MainTab::Jobs;
        for width in [40u16, 60, 80, 100, 120, 200] {
            let text = bar_text(&app, width);
            assert!(text.contains("? help"), "jobs width {width}: {text}");
            assert!(text.contains("q quit"), "jobs width {width}: {text}");
        }
        app.main_tab = MainTab::Config;
        for width in [40u16, 60, 80, 100, 120, 200] {
            let text = bar_text(&app, width);
            assert!(text.contains("? help"), "config width {width}: {text}");
            assert!(text.contains("q quit"), "config width {width}: {text}");
        }
    }

    /// `i` is only meaningful on a path row and `Enter` on the URL row opens a
    /// browser, so a fixed primary hint would be wrong on both.
    #[test]
    fn config_hints_follow_the_selected_row() {
        let keys = |action| -> Vec<String> {
            config_hints(action)
                .into_iter()
                .map(|h| h.key.to_string())
                .collect()
        };
        assert!(keys(RowAction::Browse).contains(&"i".to_string()));
        assert!(!keys(RowAction::Toggle).contains(&"i".to_string()));

        let toggles = |action| -> bool {
            config_hints(action)
                .into_iter()
                .any(|h| h.label.contains("toggle"))
        };
        assert!(toggles(RowAction::Toggle));
        assert!(!toggles(RowAction::OpenUrl));
        assert!(!toggles(RowAction::Edit));
    }

    #[test]
    fn jobs_hints_promote_start_when_the_watcher_is_off() {
        let off = jobs_hints(false);
        let start = off.iter().find(|h| h.key == "s").expect("s hint");
        assert_eq!(start.label, " start");
        assert_eq!(start.priority, HintPriority::P1);

        let on = jobs_hints(true);
        let watcher = on.iter().find(|h| h.key == "s").expect("s hint");
        assert_eq!(watcher.label, " watcher");
        assert_eq!(watcher.priority, HintPriority::P3);

        // At 60 columns the promoted start hint must survive; the old P3
        // "s watcher" was routinely dropped by select_hints.
        let mut app = bare_app();
        app.main_tab = MainTab::Jobs;
        app.watch_running = false;
        assert!(bar_text(&app, 60).contains("s start"), "{}", bar_text(&app, 60));
    }

    #[test]
    fn a_narrow_bar_admits_that_it_dropped_hints() {
        let app = bare_app();
        assert!(bar_text(&app, 60).contains(OVERFLOW_MARKER));
    }

    #[test]
    fn a_wide_bar_shows_everything_and_no_marker() {
        let app = bare_app();
        let text = bar_text(&app, 200);
        assert!(!text.contains(OVERFLOW_MARKER), "{text}");
        assert!(text.contains("browse"), "{text}");
        assert!(text.contains("config"), "{text}");
    }

    #[test]
    fn the_version_label_left_the_status_bar() {
        let app = bare_app();
        let text = bar_text(&app, 200);
        assert!(
            !text.contains(env!("CARGO_PKG_VERSION")),
            "version should live in the config About block: {text}"
        );
    }

    #[test]
    fn selection_drops_the_lowest_priority_hints_first() {
        let hints = sessions_hints(false, false, false, SourceFilter::Both);
        // Wide enough for the essentials but not for all the P3 extras.
        let chosen = select_hints(&hints, 60).chosen;
        let kept: Vec<HintPriority> = chosen.iter().map(|&i| hints[i].priority).collect();
        assert!(kept.contains(&HintPriority::P0));
        assert!(kept.contains(&HintPriority::P1));
        // Nothing at a given priority may be kept while a higher-priority,
        // non-P0 hint was dropped.
        let worst_kept = kept.iter().filter(|p| **p != HintPriority::P0).max().copied();
        if let Some(worst) = worst_kept {
            let dropped_better = (0..hints.len())
                .filter(|i| !chosen.contains(i))
                .any(|i| hints[i].priority < worst);
            assert!(!dropped_better, "dropped a higher-priority hint than one kept");
        }
    }

    #[test]
    fn selection_never_exceeds_the_available_width() {
        let hints = sessions_hints(true, false, false, SourceFilter::Both);
        for width in 1u16..=200 {
            let HintLayout { chosen, overflow } = select_hints(&hints, width);
            let gap = 2usize;
            let mut used: usize = chosen
                .iter()
                .map(|&i| hints[i].width() as usize)
                .sum::<usize>()
                + gap * chosen.len().saturating_sub(1);
            if overflow {
                used += gap + OVERFLOW_MARKER.width();
            }
            assert!(
                used <= width as usize,
                "width {width}: used {used} for {} hints",
                chosen.len()
            );
        }
    }

    #[test]
    fn a_status_error_takes_the_whole_bar() {
        let mut app = bare_app();
        app.status_error = Some("No running session 'foo'".into());
        let text = bar_text(&app, 120);
        assert!(text.contains("No running session"), "{text}");
        // The hints step aside rather than interleaving with the alert.
        assert!(!text.contains("q quit"), "{text}");
    }

    #[test]
    fn the_list_title_fits_a_narrow_pane() {
        let mut app = bare_app();
        app.hide_empty = false;
        app.group_chains = false;
        app.live_filter = true;
        for width in [18u16, 24, 30, 40, 60] {
            let title: String = build_title_spans(&app, width)
                .iter()
                .map(|s| s.content.as_ref())
                .collect();
            assert!(
                title.width() <= width.saturating_sub(2) as usize,
                "width {width}: {:?} is {} cols",
                title,
                title.width()
            );
            assert!(title.contains("Sessions"), "width {width}: {title}");
        }
    }

    #[test]
    fn a_wide_list_title_still_spells_the_badges_out() {
        let mut app = bare_app();
        app.live_filter = true;
        let title: String = build_title_spans(&app, 120)
            .iter()
            .map(|s| s.content.as_ref())
            .collect();
        assert!(title.contains("[live only]"), "{title}");
    }

    #[test]
    fn source_filter_badge_appears_when_narrowed() {
        let mut app = bare_app();
        app.source_filter = SourceFilter::Claude;
        let title: String = build_title_spans(&app, 120)
            .iter()
            .map(|s| s.content.as_ref())
            .collect();
        assert!(title.contains("[claude]"), "{title}");
        assert!(!title.contains("[cursor]"), "{title}");

        app.source_filter = SourceFilter::Cursor;
        let title: String = build_title_spans(&app, 120)
            .iter()
            .map(|s| s.content.as_ref())
            .collect();
        assert!(title.contains("[cursor]"), "{title}");

        app.source_filter = SourceFilter::Both;
        let title: String = build_title_spans(&app, 120)
            .iter()
            .map(|s| s.content.as_ref())
            .collect();
        assert!(!title.contains("[claude]"), "{title}");
        assert!(!title.contains("[cursor]"), "{title}");
    }

    #[test]
    fn sessions_hints_include_the_source_filter_key() {
        let both = sessions_hints(false, false, false, SourceFilter::Both);
        assert!(both.iter().any(|h| h.key == "s" && h.label.contains("source")));

        let claude = sessions_hints(false, false, false, SourceFilter::Claude);
        assert!(claude.iter().any(|h| h.key == "s" && h.label.contains("claude")));

        let cursor = sessions_hints(false, false, false, SourceFilter::Cursor);
        assert!(cursor.iter().any(|h| h.key == "s" && h.label.contains("cursor")));
        assert!(!cursor.iter().any(|h| h.key == "m"), "job hint hidden when Cursor-only");
    }
}
