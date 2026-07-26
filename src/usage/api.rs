//! The live source: the Anthropic OAuth usage endpoint.
//!
//! Has an authoritative reset time, unlike the local history file, but needs
//! Claude Code's OAuth token. Fetch and parse are separate so the parsing rules
//! stay testable offline.
//!
//! The response also carries `seven_day_opus` / `seven_day_sonnet`, but they are
//! consistently `null` in practice, so they are not surfaced.

use super::{UsageSnapshot, UsageWindow};
use anyhow::{bail, Context, Result};
use serde::Deserialize;
use std::time::Duration;

/// Base URL of the usage endpoint.
pub const DEFAULT_API_BASE: &str = "https://api.anthropic.com";

/// The endpoint is OAuth-gated behind this beta header.
const OAUTH_BETA: &str = "oauth-2025-04-20";

/// How long to wait on the usage request. The watch daemon polls on a timer, so
/// a hung request must not stall the loop.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

/// Every field is optional so a change on the API side degrades the output
/// instead of failing the poll. Unlisted response fields (`limits`, `spend`,
/// and friends) are ignored.
#[derive(Debug, Default, Deserialize)]
struct ApiResponse {
    #[serde(default)]
    five_hour: Option<ApiWindow>,
    #[serde(default)]
    seven_day: Option<ApiWindow>,
    #[serde(default)]
    extra_usage: Option<ApiExtraUsage>,
}

#[derive(Debug, Deserialize)]
struct ApiWindow {
    /// Used percentage. The wire name is `utilization`; the alias covers the
    /// `used_percentage` spelling Claude Code uses internally.
    #[serde(default, alias = "used_percentage")]
    utilization: Option<f64>,
    #[serde(default)]
    resets_at: Option<String>,
}

impl ApiWindow {
    /// Drops windows carrying no percentage; a window without a number tells the
    /// caller nothing and would render as a misleading "0% used".
    fn to_window(&self) -> Option<UsageWindow> {
        Some(UsageWindow {
            used_percentage: Some(self.utilization?),
            resets_at: self.resets_at.clone(),
            resets_at_estimated_ms: None,
        })
    }
}

/// Extra usage spend. `used_credits` is in minor units, scaled by
/// `decimal_places` (2 for USD).
#[derive(Debug, Deserialize)]
struct ApiExtraUsage {
    #[serde(default)]
    used_credits: Option<f64>,
    #[serde(default)]
    decimal_places: Option<u32>,
}

impl ApiExtraUsage {
    /// Converts credits to whole currency units, so the value is comparable to
    /// the local source's `xu`.
    fn to_dollars(&self) -> Option<f64> {
        let scale = 10_f64.powi(self.decimal_places.unwrap_or(2) as i32);
        Some(self.used_credits? / scale)
    }
}

/// Performs the usage request and returns the raw response body.
pub fn fetch_usage_body(base_url: &str, token: &str) -> Result<String> {
    let url = format!("{}/api/oauth/usage", base_url.trim_end_matches('/'));

    match ureq::get(&url)
        .set("Authorization", &format!("Bearer {token}"))
        .set("anthropic-beta", OAUTH_BETA)
        .set("Content-Type", "application/json")
        .set("User-Agent", concat!("ccsm/", env!("CARGO_PKG_VERSION")))
        .timeout(REQUEST_TIMEOUT)
        .call()
    {
        Ok(response) => response
            .into_string()
            .context("failed to read the usage API response body"),
        // The body carries the reason (expired token, wrong plan), so it is
        // worth surfacing rather than reporting a bare status code.
        Err(ureq::Error::Status(code, response)) => {
            let body = response.into_string().unwrap_or_default();
            bail!("usage API returned {code}: {}", body.trim());
        }
        Err(e) => Err(anyhow::Error::new(e).context("usage API request failed")),
    }
}

/// Normalizes a usage response body.
pub fn parse(body: &str, now_ms: i64) -> Result<UsageSnapshot> {
    let parsed: ApiResponse =
        serde_json::from_str(body).context("failed to parse the usage API response")?;

    Ok(UsageSnapshot {
        source: Some("api".to_string()),
        sampled_at_ms: Some(now_ms),
        age_seconds: Some(0),
        stale: false,
        five_hour: parsed.five_hour.as_ref().and_then(ApiWindow::to_window),
        seven_day: parsed.seven_day.as_ref().and_then(ApiWindow::to_window),
        extra_usage_dollars: parsed
            .extra_usage
            .as_ref()
            .and_then(ApiExtraUsage::to_dollars),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Trimmed from a real response: `utilization` is the wire field name, the
    /// per-model windows are present but null, and `limits`/`spend` are ignored.
    const BODY: &str = r#"{
        "five_hour": { "utilization": 60.0, "resets_at": "2026-07-25T19:19:59.113565+00:00",
                       "limit_dollars": null, "used_dollars": null, "remaining_dollars": null },
        "seven_day": { "utilization": 24.0, "resets_at": "2026-07-31T22:00:00.113586+00:00",
                       "limit_dollars": null, "used_dollars": null, "remaining_dollars": null },
        "seven_day_opus": null,
        "seven_day_sonnet": null,
        "extra_usage": { "is_enabled": true, "monthly_limit": null, "used_credits": 13777.0,
                         "utilization": null, "currency": "USD", "decimal_places": 2 },
        "limits": [],
        "spend": { "used": { "amount_minor": 13777, "currency": "USD", "exponent": 2 } }
    }"#;

    #[test]
    fn parse_reads_windows_and_reset_times() {
        let snapshot = parse(BODY, 1_784_989_528_179).expect("should parse");

        assert_eq!(snapshot.source.as_deref(), Some("api"));
        assert_eq!(snapshot.age_seconds, Some(0));
        assert!(!snapshot.stale);
        assert!(snapshot.is_fresh(0), "a live sample is always fresh");

        let five_hour = snapshot.five_hour.expect("five_hour window");
        assert_eq!(five_hour.used_percentage, Some(60.0));
        assert_eq!(
            five_hour.resets_at.as_deref(),
            Some("2026-07-25T19:19:59.113565+00:00")
        );
        assert!(
            five_hour.resets_at_estimated_ms.is_none(),
            "the api source never estimates on its own"
        );

        assert_eq!(snapshot.seven_day.unwrap().used_percentage, Some(24.0));
    }

    #[test]
    fn parse_resolves_the_reset_time_to_epoch_ms() {
        let snapshot = parse(BODY, 1).unwrap();
        let expected = chrono::DateTime::parse_from_rfc3339("2026-07-25T19:19:59.113565+00:00")
            .unwrap()
            .timestamp_millis();
        assert_eq!(snapshot.five_hour.unwrap().reset_at_ms(), Some(expected));
    }

    #[test]
    fn parse_converts_credits_to_dollars() {
        assert_eq!(parse(BODY, 1).unwrap().extra_usage_dollars, Some(137.77));
    }

    #[test]
    fn parse_defaults_the_credit_scale_to_two_places() {
        let snapshot = parse(r#"{"extra_usage":{"used_credits":4166.0}}"#, 1).unwrap();
        assert_eq!(snapshot.extra_usage_dollars, Some(41.66));
    }

    #[test]
    fn parse_accepts_the_used_percentage_alias() {
        let snapshot = parse(r#"{"five_hour":{"used_percentage":79.0}}"#, 1).unwrap();
        assert_eq!(snapshot.five_hour.unwrap().used_percentage, Some(79.0));
    }

    #[test]
    fn parse_tolerates_an_empty_object() {
        let snapshot = parse("{}", 1).expect("must not fail on unknown shapes");
        assert!(snapshot.five_hour.is_none());
        assert!(snapshot.seven_day.is_none());
        assert!(snapshot.extra_usage_dollars.is_none());
    }

    #[test]
    fn parse_skips_a_window_without_a_percentage() {
        let snapshot =
            parse(r#"{"five_hour":{"resets_at":"2026-07-25T20:00:00Z"}}"#, 1).expect("should parse");
        assert!(
            snapshot.five_hour.is_none(),
            "a window with no percentage must not render as 0%"
        );
    }

    #[test]
    fn parse_rejects_malformed_json() {
        assert!(parse("not json", 1).is_err());
    }

    #[test]
    fn fetch_usage_body_reports_a_connection_failure() {
        // Port 1 refuses connections, so this exercises the transport error path.
        let err = fetch_usage_body("http://127.0.0.1:1", "test-token").expect_err("should fail");
        assert!(
            format!("{err:#}").contains("usage API request failed"),
            "got: {err:#}"
        );
    }
}
