//! The local, auth-free source: Claude Desktop's usage history file.
//!
//! Claude Desktop appends a sample roughly every 15-18 minutes while it is
//! running. Reading it needs no credentials and no network, which is why it is
//! the preferred source. It carries only the 5-hour and 7-day percentages plus
//! extra-usage dollars, so it has no authoritative reset time; the reset is
//! derived from the window boundaries visible in the sample history.

use super::{UsageSnapshot, UsageWindow};
use anyhow::{anyhow, Context, Result};
use serde::Deserialize;
use std::path::{Path, PathBuf};

/// The only history schema version this reader understands.
const SUPPORTED_VERSION: u32 = 2;

/// Length of the session window the `fh` percentage tracks.
const FIVE_HOURS_MS: i64 = 5 * 60 * 60 * 1000;

/// How far apart two samples may be for a boundary between them to be trusted.
/// Desktop's cadence is 15-18 minutes; a wider gap means it was closed and the
/// true boundary is unknown.
const MAX_BOUNDARY_GAP_MS: i64 = 20 * 60 * 1000;

/// File name of the history document inside Claude Desktop's config directory.
const HISTORY_FILE_NAME: &str = "plan-usage-history.json";

/// Env var overriding the history file location, primarily for tests.
const HISTORY_PATH_ENV: &str = "CCSM_USAGE_HISTORY_FILE";

/// A parsed history document.
#[derive(Debug, Deserialize)]
pub struct History {
    version: u32,
    samples: Vec<Sample>,
}

/// One sample. Unknown fields (`org`, future additions) are ignored.
#[derive(Debug, Deserialize)]
struct Sample {
    /// Sample time, epoch milliseconds.
    t: i64,
    u: SampleUsage,
}

#[derive(Debug, Deserialize)]
struct SampleUsage {
    /// Five-hour session window, used percentage.
    fh: f64,
    /// Seven-day window, used percentage.
    sd: f64,
    /// Extra usage in dollars. Only present while extra usage is enabled.
    xu: Option<f64>,
}

/// Resolves the history file path: explicit override, then env var, then
/// Claude Desktop's location inside the platform config directory
/// (`~/Library/Application Support` on macOS, `%APPDATA%` on Windows,
/// `~/.config` on Linux).
pub fn history_path(override_path: Option<&str>) -> PathBuf {
    if let Some(path) = override_path.filter(|p| !p.is_empty()) {
        return PathBuf::from(path);
    }
    if let Ok(from_env) = std::env::var(HISTORY_PATH_ENV) {
        if !from_env.is_empty() {
            return PathBuf::from(from_env);
        }
    }
    let base = dirs::config_dir().unwrap_or_else(|| PathBuf::from("."));
    base.join("Claude").join(HISTORY_FILE_NAME)
}

/// Parses and validates a history document. Pure, so the format rules are
/// testable without touching the filesystem.
pub fn parse_history(text: &str) -> Result<History> {
    let history: History =
        serde_json::from_str(text).context("failed to parse usage history JSON")?;

    if history.version != SUPPORTED_VERSION {
        return Err(anyhow!(
            "unsupported usage history version {} (expected {SUPPORTED_VERSION})",
            history.version
        ));
    }
    if history.samples.is_empty() {
        return Err(anyhow!("usage history contains no samples"));
    }

    Ok(history)
}

/// The most recent sample. Safe to unwrap internally because `parse_history`
/// rejects empty sample lists.
fn newest_sample(history: &History) -> &Sample {
    history
        .samples
        .iter()
        .max_by_key(|s| s.t)
        .expect("parse_history rejects empty samples")
}

/// Estimates when the current 5-hour window resets, from the window boundaries
/// visible in the history.
///
/// A boundary is a sample whose `fh` rose from 0 (a new window's first usage) or
/// dropped below the previous sample (a rollover, which in real data often lands
/// on a non-zero value like 79 -> 1 rather than on 0). The most recent boundary
/// is treated as the window start, so the window resets five hours later.
///
/// Returns `None` whenever the estimate would be a guess: no boundary in the
/// history, a sampling gap wide enough to hide the real boundary, or an already
/// elapsed result. A 5-hour window is always running once it has started, even
/// at 0% used, so a low or zero current percentage is not by itself a reason to
/// give up on an estimate.
fn estimate_five_hour_reset(history: &History, now_ms: i64) -> Option<i64> {
    let mut samples: Vec<&Sample> = history.samples.iter().collect();
    samples.sort_by_key(|s| s.t);

    for i in (1..samples.len()).rev() {
        let (previous, current) = (samples[i - 1], samples[i]);
        let rose_from_zero = previous.u.fh <= 0.0 && current.u.fh > 0.0;
        let rolled_over = current.u.fh < previous.u.fh;
        if !rose_from_zero && !rolled_over {
            continue;
        }

        // Desktop was closed across this boundary, so its real time is unknown.
        if current.t - previous.t > MAX_BOUNDARY_GAP_MS {
            return None;
        }

        let reset_ms = current.t + FIVE_HOURS_MS;
        return if reset_ms > now_ms { Some(reset_ms) } else { None };
    }

    None
}

/// Builds a normalized snapshot from the newest sample in the history.
fn to_snapshot(history: &History, now_ms: i64, max_age_seconds: u64) -> UsageSnapshot {
    let sample = newest_sample(history);
    let age_seconds = (now_ms - sample.t) / 1000;

    let mut five_hour = UsageWindow::from_percentage(sample.u.fh);
    five_hour.resets_at_estimated_ms = estimate_five_hour_reset(history, now_ms);

    UsageSnapshot {
        source: Some("local".to_string()),
        sampled_at_ms: Some(sample.t),
        age_seconds: Some(age_seconds),
        // A negative age is clock skew, not freshness lost.
        stale: age_seconds > max_age_seconds as i64,
        five_hour: Some(five_hour),
        seven_day: Some(UsageWindow::from_percentage(sample.u.sd)),
        extra_usage_dollars: sample.u.xu,
    }
}

/// Reads and normalizes the local history file.
pub fn load(path: &Path, now_ms: i64, max_age_seconds: u64) -> Result<UsageSnapshot> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read usage history at {}", path.display()))?;
    let history = parse_history(&text)?;
    Ok(to_snapshot(&history, now_ms, max_age_seconds))
}

/// Best-effort reset estimate used to enrich an API result. Any failure to read
/// or parse the history file just means no estimate is available; it is never
/// an error, since this is supplementary data rather than the primary source.
pub fn try_estimate_five_hour_reset(path: &Path, now_ms: i64) -> Option<i64> {
    let text = std::fs::read_to_string(path).ok()?;
    let history = parse_history(&text).ok()?;
    estimate_five_hour_reset(&history, now_ms)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MINUTE_MS: i64 = 60 * 1000;

    /// Builds a history from `(minutes_before_now, fh)` pairs.
    fn history_at(now_ms: i64, points: &[(i64, f64)]) -> History {
        History {
            version: SUPPORTED_VERSION,
            samples: points
                .iter()
                .map(|(minutes_ago, fh)| Sample {
                    t: now_ms - minutes_ago * MINUTE_MS,
                    u: SampleUsage {
                        fh: *fh,
                        sd: 18.0,
                        xu: None,
                    },
                })
                .collect(),
        }
    }

    #[test]
    fn parse_history_accepts_a_real_shaped_document() {
        let history = parse_history(
            r#"{"version":2,"samples":[
                {"t":1783466131632,"org":"4439141b","u":{"fh":0,"sd":18,"xu":41.66}},
                {"t":1783467031632,"org":"4439141b","u":{"fh":12,"sd":18}}
            ]}"#,
        )
        .expect("should parse");
        assert_eq!(history.samples.len(), 2);
        assert_eq!(newest_sample(&history).u.fh, 12.0);
    }

    #[test]
    fn parse_history_rejects_unknown_version() {
        let err = parse_history(r#"{"version":99,"samples":[{"t":1,"u":{"fh":0,"sd":0}}]}"#)
            .expect_err("should reject");
        assert!(err.to_string().contains("99"), "should name the version");
    }

    #[test]
    fn parse_history_rejects_empty_samples() {
        assert!(parse_history(r#"{"version":2,"samples":[]}"#).is_err());
    }

    #[test]
    fn parse_history_rejects_malformed_json() {
        assert!(parse_history("not json").is_err());
    }

    #[test]
    fn newest_sample_picks_highest_timestamp() {
        let history = parse_history(
            r#"{"version":2,"samples":[
                {"t":300,"u":{"fh":3,"sd":1}},
                {"t":100,"u":{"fh":1,"sd":1}},
                {"t":200,"u":{"fh":2,"sd":1}}
            ]}"#,
        )
        .unwrap();
        assert_eq!(newest_sample(&history).t, 300);
    }

    #[test]
    fn to_snapshot_reports_percentages_from_the_newest_sample() {
        let now = 1_000_000_000_000;
        let history =
            parse_history(r#"{"version":2,"samples":[{"t":100,"u":{"fh":79,"sd":18}}]}"#).unwrap();
        let snapshot = to_snapshot(&history, now, 900);
        assert_eq!(
            snapshot.five_hour.unwrap().used_percentage,
            Some(79.0)
        );
        assert_eq!(snapshot.seven_day.unwrap().used_percentage, Some(18.0));
        assert_eq!(snapshot.sampled_at_ms, Some(100));
        assert_eq!(snapshot.source.as_deref(), Some("local"));
    }

    #[test]
    fn to_snapshot_never_reports_an_authoritative_resets_at() {
        let now = 1_000_000_000_000;
        let snapshot = to_snapshot(&history_at(now, &[(20, 0.0), (5, 40.0)]), now, 900);
        assert!(snapshot.five_hour.as_ref().unwrap().resets_at.is_none());
        assert!(snapshot.seven_day.as_ref().unwrap().resets_at.is_none());
    }

    #[test]
    fn extra_usage_is_none_when_absent_and_some_when_present() {
        let now = 1_000_000_000_000;
        let without =
            parse_history(r#"{"version":2,"samples":[{"t":1,"u":{"fh":0,"sd":18}}]}"#).unwrap();
        assert_eq!(to_snapshot(&without, now, 900).extra_usage_dollars, None);

        let with =
            parse_history(r#"{"version":2,"samples":[{"t":1,"u":{"fh":0,"sd":18,"xu":41.66}}]}"#)
                .unwrap();
        assert_eq!(
            to_snapshot(&with, now, 900).extra_usage_dollars,
            Some(41.66)
        );
    }

    #[test]
    fn stale_flag_tracks_the_threshold() {
        let now = 1_000_000_000_000;
        let fresh = history_at(now, &[(14, 10.0)]);
        assert!(!to_snapshot(&fresh, now, 900).stale, "14m old is within 15m");

        let stale = history_at(now, &[(16, 10.0)]);
        assert!(to_snapshot(&stale, now, 900).stale, "16m old is past 15m");
    }

    #[test]
    fn a_skewed_clock_does_not_read_as_stale() {
        let now = 1_000_000_000_000;
        let future = history_at(now, &[(-5, 10.0)]);
        assert!(!to_snapshot(&future, now, 900).stale);
    }

    #[test]
    fn history_path_prefers_the_explicit_override_over_env() {
        let resolved = history_path(Some("/from/arg.json"));
        assert_eq!(resolved, PathBuf::from("/from/arg.json"));
    }

    #[test]
    fn history_path_falls_back_to_the_desktop_location() {
        let resolved = history_path(None);
        assert!(
            resolved.ends_with(format!("Claude/{HISTORY_FILE_NAME}")),
            "got: {}",
            resolved.display()
        );
    }

    #[test]
    fn load_reports_a_missing_file() {
        let err = load(Path::new("/nonexistent/history.json"), 0, 900).expect_err("should fail");
        assert!(err.to_string().contains("usage history"));
    }

    #[test]
    fn try_estimate_reports_none_for_a_missing_file() {
        assert_eq!(
            try_estimate_five_hour_reset(Path::new("/nonexistent/history.json"), 0),
            None
        );
    }

    // --- estimate_five_hour_reset -------------------------------------------

    #[test]
    fn estimate_detects_a_rise_from_zero() {
        let now = 1_000_000_000_000;
        // Window started 60m ago, so it resets 4h from now.
        let history = history_at(now, &[(75, 0.0), (60, 15.0), (5, 42.0)]);
        let reset = estimate_five_hour_reset(&history, now).expect("should estimate");
        assert_eq!(reset, now - 60 * MINUTE_MS + FIVE_HOURS_MS);
    }

    #[test]
    fn estimate_detects_an_expiry_to_zero_then_reuse() {
        let now = 1_000_000_000_000;
        // 100 -> 0 expiry, then usage resumes 90m ago: that rise is the boundary.
        let history = history_at(now, &[(120, 100.0), (105, 0.0), (90, 4.0), (5, 30.0)]);
        let reset = estimate_five_hour_reset(&history, now).expect("should estimate");
        assert_eq!(reset, now - 90 * MINUTE_MS + FIVE_HOURS_MS);
    }

    #[test]
    fn estimate_detects_a_mid_use_rollover_to_nonzero() {
        let now = 1_000_000_000_000;
        // 79 -> 1 rollover 45m ago, a pattern present in real history.
        let history = history_at(now, &[(75, 60.0), (60, 79.0), (45, 1.0), (5, 20.0)]);
        let reset = estimate_five_hour_reset(&history, now).expect("should estimate");
        assert_eq!(reset, now - 45 * MINUTE_MS + FIVE_HOURS_MS);
    }

    #[test]
    fn estimate_detects_a_partial_rollover() {
        let now = 1_000_000_000_000;
        // 42 -> 15, the other real-world rollover shape.
        let history = history_at(now, &[(60, 30.0), (45, 42.0), (30, 15.0), (5, 22.0)]);
        let reset = estimate_five_hour_reset(&history, now).expect("should estimate");
        assert_eq!(reset, now - 30 * MINUTE_MS + FIVE_HOURS_MS);
    }

    #[test]
    fn estimate_is_none_when_no_boundary_is_visible() {
        let now = 1_000_000_000_000;
        // Monotonically rising usage: the window start is off the front of the file.
        let history = history_at(now, &[(45, 10.0), (30, 20.0), (15, 30.0), (5, 40.0)]);
        assert_eq!(estimate_five_hour_reset(&history, now), None);
    }

    #[test]
    fn estimate_is_none_when_a_sampling_gap_hides_the_boundary() {
        let now = 1_000_000_000_000;
        // Desktop was closed for 3h across the boundary, so its time is unknown.
        let history = history_at(now, &[(200, 0.0), (20, 15.0), (5, 20.0)]);
        assert_eq!(estimate_five_hour_reset(&history, now), None);
    }

    #[test]
    fn estimate_is_none_when_the_computed_reset_already_passed() {
        let now = 1_000_000_000_000;
        // Boundary 6h ago: the window has since rolled over unobserved.
        let history = history_at(now, &[(375, 0.0), (360, 15.0), (5, 90.0)]);
        assert_eq!(estimate_five_hour_reset(&history, now), None);
    }

    #[test]
    fn estimate_still_computes_when_current_usage_is_zero() {
        let now = 1_000_000_000_000;
        // The window rolled over 45m ago (100 -> 0) and hasn't been used since.
        // It is still running, not "inactive", so a reset estimate is expected.
        let history = history_at(now, &[(60, 100.0), (45, 0.0), (5, 0.0)]);
        let reset = estimate_five_hour_reset(&history, now)
            .expect("a window at 0% used is still due to reset five hours after it started");
        assert_eq!(reset, now - 45 * MINUTE_MS + FIVE_HOURS_MS);
    }

    #[test]
    fn estimate_is_none_for_a_single_sample() {
        let now = 1_000_000_000_000;
        assert_eq!(
            estimate_five_hour_reset(&history_at(now, &[(5, 40.0)]), now),
            None
        );
    }

    #[test]
    fn estimate_tolerates_unsorted_samples() {
        let now = 1_000_000_000_000;
        let history = history_at(now, &[(5, 42.0), (75, 0.0), (60, 15.0)]);
        let reset = estimate_five_hour_reset(&history, now).expect("should estimate");
        assert_eq!(reset, now - 60 * MINUTE_MS + FIVE_HOURS_MS);
    }
}
