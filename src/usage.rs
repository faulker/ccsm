//! Wraps the `claude-usage` CLI, which reports Claude account usage
//! (5-hour and 7-day rate limit windows). Parsing is pure and has zero I/O;
//! the single I/O function shells out to the binary.

use anyhow::{bail, Context, Result};
use chrono::DateTime;
use serde::Deserialize;
use std::process::Command;

/// One usage window (5-hour, 7-day, or per-model) as reported by claude-usage.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct UsageWindow {
    /// Percentage of the window's quota used so far, when known.
    pub used_percentage: Option<f64>,
    /// Exact reset timestamp as reported by the source, when known. Format is
    /// unverified against a real sample (always null so far); treated as an
    /// RFC 3339 string with a naive-UTC fallback.
    pub resets_at: Option<String>,
    /// Estimated reset time in epoch milliseconds, when known.
    pub resets_at_estimated_ms: Option<i64>,
}

/// A full usage sample. Mirrors the JSON contract of `claude-usage --format json`.
/// Kept fully in sync with the contract even where fields aren't consumed yet,
/// so parsing stays exercised by tests and the data is ready when the UI needs it.
#[derive(Debug, Clone, Default, Deserialize)]
#[allow(dead_code)]
pub struct UsageSnapshot {
    /// Where the sample was sourced from (e.g. `"local"` or `"api"`).
    pub source: Option<String>,
    /// When the sample was taken, in epoch milliseconds.
    pub sampled_at_ms: Option<i64>,
    /// Age of the sample in seconds at the time it was reported.
    pub age_seconds: Option<i64>,
    /// True when the underlying source flagged this sample as stale. Defaults
    /// to false if absent so a future payload without the field still parses.
    #[serde(default)]
    pub stale: bool,
    /// The rolling 5-hour usage window.
    pub five_hour: Option<UsageWindow>,
    /// The rolling 7-day usage window across all models.
    pub seven_day: Option<UsageWindow>,
    /// The rolling 7-day usage window for Opus specifically, when reported.
    pub seven_day_opus: Option<UsageWindow>,
    /// The rolling 7-day usage window for Sonnet specifically, when reported.
    pub seven_day_sonnet: Option<UsageWindow>,
    /// Extra usage billed in dollars beyond the plan's included quota, when reported.
    pub extra_usage_dollars: Option<f64>,
}

impl UsageWindow {
    /// Epoch ms when this window resets. Prefers the exact `resets_at`
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

/// Parse one `claude-usage --format json` payload. Tolerates leading warning
/// lines by taking the last non-empty line.
pub fn parse(stdout: &str) -> Result<UsageSnapshot> {
    let last_line = stdout
        .lines()
        .rev()
        .find(|line| !line.trim().is_empty())
        .context("claude-usage produced no output")?;
    serde_json::from_str(last_line).context("Failed to parse claude-usage JSON output")
}

/// Build the argv for invoking `claude-usage --format json`. Pure so it is
/// testable without the binary present. `max_age_seconds` is converted to
/// minutes (clamped to a minimum of 1) since the CLI's `--max-age` flag takes
/// minutes.
fn fetch_args(bin: &str, source: &str, max_age_seconds: u64) -> Vec<String> {
    let minutes = (max_age_seconds / 60).max(1);
    vec![
        bin.to_string(),
        "--format".to_string(),
        "json".to_string(),
        "--source".to_string(),
        source.to_string(),
        "--max-age".to_string(),
        minutes.to_string(),
    ]
}

/// Run the claude-usage binary and parse its output. Uses array-based
/// execution (never a shell string) per the repo's shell-safety convention.
/// A non-zero exit or unparseable output is an `Err` with context, never a panic.
pub fn fetch(bin: &str, source: &str, max_age_seconds: u64) -> Result<UsageSnapshot> {
    let args = fetch_args(bin, source, max_age_seconds);
    let output = Command::new(&args[0])
        .args(&args[1..])
        .output()
        .with_context(|| format!("Failed to execute {bin}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("{bin} exited with {}: {stderr}", output.status);
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    parse(&stdout)
}

#[cfg(test)]
mod tests {
    use super::*;

    const OBSERVED_JSON: &str = r#"{"source":"local","sampled_at_ms":1784997089620,"age_seconds":237,"stale":false,"five_hour":{"used_percentage":68.0,"resets_at":null,"resets_at_estimated_ms":1785007827659},"seven_day":{"used_percentage":25.0,"resets_at":null,"resets_at_estimated_ms":null},"seven_day_opus":null,"seven_day_sonnet":null,"extra_usage_dollars":null}"#;

    #[test]
    fn parses_observed_json() {
        let snap = parse(OBSERVED_JSON).unwrap();
        assert_eq!(snap.source.as_deref(), Some("local"));
        assert_eq!(snap.sampled_at_ms, Some(1784997089620));
        assert_eq!(snap.age_seconds, Some(237));
        assert!(!snap.stale);
        let five_hour = snap.five_hour.unwrap();
        assert_eq!(five_hour.used_percentage, Some(68.0));
        assert_eq!(five_hour.resets_at, None);
        assert_eq!(five_hour.resets_at_estimated_ms, Some(1785007827659));
    }

    #[test]
    fn reset_at_ms_prefers_exact_over_estimate() {
        let window = UsageWindow {
            used_percentage: None,
            resets_at: Some("2026-07-25T12:00:00Z".to_string()),
            resets_at_estimated_ms: Some(1),
        };
        let expected = DateTime::parse_from_rfc3339("2026-07-25T12:00:00Z")
            .unwrap()
            .timestamp_millis();
        assert_eq!(window.reset_at_ms(), Some(expected));
    }

    #[test]
    fn reset_at_ms_falls_back_to_estimate_when_resets_at_is_null() {
        let window = UsageWindow {
            used_percentage: None,
            resets_at: None,
            resets_at_estimated_ms: Some(1785007827659),
        };
        assert_eq!(window.reset_at_ms(), Some(1785007827659));
    }

    #[test]
    fn reset_at_ms_none_when_both_missing() {
        let window = UsageWindow {
            used_percentage: None,
            resets_at: None,
            resets_at_estimated_ms: None,
        };
        assert_eq!(window.reset_at_ms(), None);
    }

    #[test]
    fn reset_at_ms_falls_back_to_estimate_when_resets_at_is_unparseable() {
        let window = UsageWindow {
            used_percentage: None,
            resets_at: Some("not a timestamp".to_string()),
            resets_at_estimated_ms: Some(42),
        };
        assert_eq!(window.reset_at_ms(), Some(42));
    }

    #[test]
    fn parses_null_per_model_and_extra_fields() {
        let snap = parse(OBSERVED_JSON).unwrap();
        assert!(snap.seven_day_opus.is_none());
        assert!(snap.seven_day_sonnet.is_none());
        assert!(snap.extra_usage_dollars.is_none());
    }

    #[test]
    fn parses_unknown_extra_keys_without_error() {
        let json = r#"{"source":"local","sampled_at_ms":1,"age_seconds":1,"stale":false,"five_hour":null,"seven_day":null,"seven_day_opus":null,"seven_day_sonnet":null,"extra_usage_dollars":null,"totally_new_field":"whatever"}"#;
        let snap = parse(json).unwrap();
        assert!(snap.five_hour.is_none());
    }

    #[test]
    fn missing_seven_day_field_parses_as_none() {
        let json = r#"{"source":"local","sampled_at_ms":1,"age_seconds":1,"stale":false,"five_hour":null,"seven_day_opus":null,"seven_day_sonnet":null,"extra_usage_dollars":null}"#;
        let snap = parse(json).unwrap();
        assert!(snap.seven_day.is_none());
    }

    #[test]
    fn missing_stale_field_defaults_to_false() {
        let json = r#"{"source":"local","five_hour":{"used_percentage":10.0}}"#;
        let snap = parse(json).unwrap();
        assert!(!snap.stale);
        assert!(snap.is_fresh(900));
    }

    #[test]
    fn used_percentage_null_parses_as_none() {
        let json = r#"{"used_percentage":null,"resets_at":null,"resets_at_estimated_ms":null}"#;
        let window: UsageWindow = serde_json::from_str(json).unwrap();
        assert_eq!(window.used_percentage, None);
    }

    #[test]
    fn empty_string_is_err() {
        assert!(parse("").is_err());
    }

    #[test]
    fn garbage_is_err() {
        assert!(parse("not json at all").is_err());
    }

    #[test]
    fn truncated_json_is_err() {
        assert!(parse(r#"{"source":"local","sampled_at_ms":1784997089620,"age_sec"#).is_err());
    }

    #[test]
    fn multi_line_stdout_with_warning_and_trailing_newline_parses() {
        let stdout = format!("warning: something noisy happened\n{OBSERVED_JSON}\n");
        let snap = parse(&stdout).unwrap();
        assert_eq!(snap.source.as_deref(), Some("local"));
    }

    #[test]
    fn is_fresh_stale_is_always_false() {
        let snap = UsageSnapshot {
            stale: true,
            age_seconds: Some(0),
            ..Default::default()
        };
        assert!(!snap.is_fresh(3600));
    }

    #[test]
    fn is_fresh_within_limit_is_true() {
        let snap = UsageSnapshot {
            stale: false,
            age_seconds: Some(100),
            ..Default::default()
        };
        assert!(snap.is_fresh(200));
    }

    #[test]
    fn is_fresh_over_limit_is_false() {
        let snap = UsageSnapshot {
            stale: false,
            age_seconds: Some(300),
            ..Default::default()
        };
        assert!(!snap.is_fresh(200));
    }

    #[test]
    fn is_fresh_missing_age_seconds_counts_as_fresh_when_not_stale() {
        let snap = UsageSnapshot {
            stale: false,
            age_seconds: None,
            ..Default::default()
        };
        assert!(snap.is_fresh(200));
    }

    #[test]
    fn fetch_args_shape_and_conversion() {
        let args = fetch_args("claude-usage", "auto", 600);
        assert_eq!(
            args,
            vec![
                "claude-usage",
                "--format",
                "json",
                "--source",
                "auto",
                "--max-age",
                "10",
            ]
        );
    }

    #[test]
    fn fetch_args_clamps_minutes_to_minimum_of_one() {
        let args = fetch_args("claude-usage", "local", 30);
        assert_eq!(args.last().unwrap(), "1");
    }
}
