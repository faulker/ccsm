//! Account usage: the 5-hour and 7-day rate-limit windows, read natively.
//!
//! Two sources, the same two Claude's own tooling uses. `local` parses Claude
//! Desktop's `plan-usage-history.json`, which needs no auth, no network, and no
//! Keychain prompt, but carries no authoritative reset time. `api` calls the
//! OAuth usage endpoint with Claude Code's own credentials, which is
//! authoritative about reset times but has to read a token. `auto` prefers a
//! fresh local sample and only falls back to the API when it is stale or
//! missing.
//!
//! Everything except `api::fetch_usage_body`, `credentials::oauth_token`, and
//! the file reads in `local` is pure, so the source-selection and parsing rules
//! are testable without a network, a Keychain, or a Claude install.

pub mod api;
pub mod credentials;
pub mod local;
#[cfg(test)]
mod tests;

use anyhow::{anyhow, Result};
use chrono::DateTime;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

/// One usage window (5-hour session or 7-day).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct UsageWindow {
    /// Percentage of the window's quota used so far, when known.
    pub used_percentage: Option<f64>,
    /// Authoritative reset timestamp, an ISO 8601 string straight from the API.
    /// Always `None` on the local source, which does not report one.
    pub resets_at: Option<String>,
    /// Reset time derived from window boundaries visible in the local history.
    /// The only reset signal the local source has, and a best-effort supplement
    /// on the api source.
    pub resets_at_estimated_ms: Option<i64>,
}

/// A full usage sample, normalized across sources.
#[derive(Debug, Clone, Default)]
pub struct UsageSnapshot {
    /// Which source produced this sample: `"local"` or `"api"`. Part of the
    /// normalized model and asserted on in tests; the TUI reads the derived
    /// scalars the daemon persists rather than the snapshot itself.
    #[allow(dead_code)]
    pub source: Option<String>,
    /// When the underlying measurement was taken, in epoch milliseconds.
    pub sampled_at_ms: Option<i64>,
    /// Age of the sample in seconds at the time it was taken.
    pub age_seconds: Option<i64>,
    /// True when the sample is older than the caller's staleness threshold.
    pub stale: bool,
    /// The rolling 5-hour usage window.
    pub five_hour: Option<UsageWindow>,
    /// The rolling 7-day usage window across all models.
    pub seven_day: Option<UsageWindow>,
    /// Extra usage billed in dollars beyond the plan's included quota, when reported.
    #[allow(dead_code)]
    pub extra_usage_dollars: Option<f64>,
}

impl UsageWindow {
    /// A window with a percentage and nothing else known about it.
    pub fn from_percentage(used_percentage: f64) -> Self {
        UsageWindow {
            used_percentage: Some(used_percentage),
            resets_at: None,
            resets_at_estimated_ms: None,
        }
    }

    /// Epoch ms when this window resets. Prefers the authoritative `resets_at`
    /// timestamp, falling back to the estimate. `None` when neither is usable.
    pub fn reset_at_ms(&self) -> Option<i64> {
        if let Some(exact) = self.resets_at.as_deref().and_then(parse_reset_timestamp) {
            return Some(exact);
        }
        self.resets_at_estimated_ms
    }
}

/// Parse a `resets_at` timestamp string as RFC 3339, falling back to a
/// naive-UTC parse. Returns `None` on failure rather than erroring.
fn parse_reset_timestamp(s: &str) -> Option<i64> {
    if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
        return Some(dt.timestamp_millis());
    }
    chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S")
        .ok()
        .map(|naive| naive.and_utc().timestamp_millis())
}

impl UsageSnapshot {
    /// True when the sample is recent enough to act on. Never resume work on a
    /// stale sample. A missing `age_seconds` counts as fresh only if not stale.
    pub fn is_fresh(&self, max_age_seconds: u64) -> bool {
        if self.stale {
            return false;
        }
        match self.age_seconds {
            Some(age) => age >= 0 && (age as u64) <= max_age_seconds,
            None => true,
        }
    }
}

/// Current wall-clock time in epoch milliseconds. Clamps to 0 for a pre-epoch
/// clock rather than panicking.
pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Which sources `fetch` is allowed to consult.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// Local file when fresh, otherwise the API.
    Auto,
    /// Claude Desktop's history file only. Never touches credentials.
    Local,
    /// The OAuth API only.
    Api,
}

impl Source {
    /// Parse a configured source string. Anything unrecognized is `Auto`, since
    /// a typo in the config should degrade to the safe default rather than
    /// making usage unreadable.
    pub fn parse(s: &str) -> Source {
        match s.trim().to_ascii_lowercase().as_str() {
            "local" => Source::Local,
            "api" => Source::Api,
            _ => Source::Auto,
        }
    }
}

/// Read one usage sample. `history_override` points at a non-default
/// `plan-usage-history.json`; `None` uses the standard location.
///
/// The only entry point that performs I/O. A failure here is always an `Err`
/// with context, never a panic, so the watch daemon can log it and retry.
pub fn fetch(
    source: &str,
    max_age_seconds: u64,
    history_override: Option<&str>,
) -> Result<UsageSnapshot> {
    let now = now_ms();
    let path = local::history_path(history_override);

    match Source::parse(source) {
        Source::Local => local::load(&path, now, max_age_seconds),
        Source::Api => load_api(&path, now),
        Source::Auto => {
            let local_result = local::load(&path, now, max_age_seconds);

            // A fresh local sample is the whole point of the local source: no
            // credentials, no network, no Keychain prompt in the daemon.
            if let Ok(snapshot) = &local_result {
                if !snapshot.stale {
                    return Ok(snapshot.clone());
                }
            }

            match (load_api(&path, now), local_result) {
                (Ok(snapshot), _) => Ok(snapshot),
                // Stale numbers still beat nothing: `is_fresh` keeps the engine
                // from resuming on them, and a pause decision is safe either way.
                (Err(_), Ok(stale_local)) => Ok(stale_local),
                (Err(api_err), Err(local_err)) => Err(anyhow!(
                    "no usage data available (local: {local_err:#}; api: {api_err:#})"
                )),
            }
        }
    }
}

/// Fetch from the API and supplement it with the local reset estimate, so
/// `resets_at_estimated_ms` is populated on both sources. A missing or
/// unreadable history file just leaves the estimate `None`.
fn load_api(history_path: &Path, now_ms: i64) -> Result<UsageSnapshot> {
    let token = credentials::oauth_token()?;
    let body = api::fetch_usage_body(api::DEFAULT_API_BASE, &token)?;
    let mut snapshot = api::parse(&body, now_ms)?;

    if let Some(five_hour) = snapshot.five_hour.as_mut() {
        five_hour.resets_at_estimated_ms = local::try_estimate_five_hour_reset(history_path, now_ms);
    }

    Ok(snapshot)
}

/// Renders an elapsed duration as a short relative age ("4m", "3h 12m").
fn format_age(seconds: i64) -> String {
    if seconds <= 0 {
        return "just now".to_string();
    }
    if seconds < 60 {
        return format!("{seconds}s");
    }
    if seconds < 3600 {
        return format!("{}m", seconds / 60);
    }
    if seconds < 86_400 {
        let (hours, minutes) = (seconds / 3600, (seconds % 3600) / 60);
        return if minutes == 0 {
            format!("{hours}h")
        } else {
            format!("{hours}h {minutes}m")
        };
    }
    let (days, hours) = (seconds / 86_400, (seconds % 86_400) / 3600);
    if hours == 0 {
        format!("{days}d")
    } else {
        format!("{days}d {hours}h")
    }
}

/// Formats one window line, e.g. `5h window   79% used   resets in ~1h 20m`.
fn render_window(label: &str, window: &UsageWindow, now_ms: i64) -> String {
    let mut line = match window.used_percentage {
        Some(pct) => format!("{label:<11}{pct:>3.0}% used"),
        None => format!("{label:<11}  ? used"),
    };
    if let Some(reset_ms) = window.reset_at_ms() {
        let remaining = (reset_ms - now_ms) / 1000;
        let when = if remaining <= 0 {
            "now".to_string()
        } else {
            format_age(remaining)
        };
        // Only the api source knows a real reset time; everything else is
        // inferred from observed window boundaries and is labelled as such.
        let estimated = if window.resets_at.is_none() { " (est)" } else { "" };
        line.push_str(&format!("   resets in ~{when}{estimated}"));
    }
    line
}

/// Renders a snapshot as the human-readable `ccsm --usage` report.
pub fn render(snapshot: &UsageSnapshot) -> String {
    let sampled_at = snapshot.sampled_at_ms.unwrap_or(0);
    let age = snapshot.age_seconds.unwrap_or(0);
    let now = sampled_at + age * 1000;
    let mut lines = Vec::new();

    for (label, window) in [
        ("5h window", &snapshot.five_hour),
        ("7d window", &snapshot.seven_day),
    ] {
        if let Some(window) = window {
            lines.push(render_window(label, window, now));
        }
    }

    if let Some(dollars) = snapshot.extra_usage_dollars {
        lines.push(format!("extra usage: ${dollars:.2}"));
    }

    match snapshot.source.as_deref() {
        Some("api") => lines.push("source: api, live".to_string()),
        _ => {
            let mut line = format!("source: local, sampled {} ago", format_age(age));
            if snapshot.stale {
                line.push_str(" (stale — is Claude Desktop running?)");
            }
            lines.push(line);
        }
    }

    lines.join("\n")
}

/// True when the configured source cannot produce data at all: the local
/// history file is absent and the source is pinned to `local`, so there is no
/// API to fall back to. Cheap enough to call on startup; it stats one file and
/// never touches credentials.
pub fn source_unavailable(source: &str, history_override: Option<&str>) -> bool {
    Source::parse(source) == Source::Local && !local::history_path(history_override).exists()
}
