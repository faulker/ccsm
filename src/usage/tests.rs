//! Tests for the normalized usage model and source selection.
//!
//! Nothing here exercises the api source: reaching it means reading real
//! credentials, which on macOS can raise a Keychain prompt in the middle of a
//! test run. The api parsing rules are covered offline in `api.rs` instead.

use super::*;
use std::io::Write;

/// Writes a history document to a temp file and returns it. The handle is
/// returned alongside so the file outlives the test body.
fn history_file(json: &str) -> tempfile::NamedTempFile {
    let mut file = tempfile::NamedTempFile::new().expect("temp file");
    file.write_all(json.as_bytes()).expect("write history");
    file.flush().expect("flush history");
    file
}

/// A one-sample history whose sample is `age_seconds` old.
fn history_json(now_ms: i64, age_seconds: i64, five_hour: f64) -> String {
    format!(
        r#"{{"version":2,"samples":[{{"t":{},"u":{{"fh":{five_hour},"sd":18}}}}]}}"#,
        now_ms - age_seconds * 1000
    )
}

#[test]
fn source_parses_the_three_names() {
    assert_eq!(Source::parse("local"), Source::Local);
    assert_eq!(Source::parse("api"), Source::Api);
    assert_eq!(Source::parse("auto"), Source::Auto);
}

#[test]
fn source_is_case_and_whitespace_insensitive() {
    assert_eq!(Source::parse(" Local "), Source::Local);
    assert_eq!(Source::parse("API"), Source::Api);
}

#[test]
fn an_unknown_source_falls_back_to_auto() {
    assert_eq!(Source::parse("clau-usage"), Source::Auto);
    assert_eq!(Source::parse(""), Source::Auto);
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
fn reset_at_ms_falls_back_to_the_estimate() {
    let window = UsageWindow {
        used_percentage: None,
        resets_at: None,
        resets_at_estimated_ms: Some(1785007827659),
    };
    assert_eq!(window.reset_at_ms(), Some(1785007827659));
}

#[test]
fn reset_at_ms_falls_back_when_resets_at_is_unparseable() {
    let window = UsageWindow {
        used_percentage: None,
        resets_at: Some("not a timestamp".to_string()),
        resets_at_estimated_ms: Some(42),
    };
    assert_eq!(window.reset_at_ms(), Some(42));
}

#[test]
fn reset_at_ms_is_none_when_both_are_missing() {
    assert_eq!(UsageWindow::default().reset_at_ms(), None);
}

#[test]
fn reset_at_ms_accepts_a_naive_utc_timestamp() {
    let window = UsageWindow {
        used_percentage: None,
        resets_at: Some("2026-07-25 12:00:00".to_string()),
        resets_at_estimated_ms: None,
    };
    let expected = DateTime::parse_from_rfc3339("2026-07-25T12:00:00Z")
        .unwrap()
        .timestamp_millis();
    assert_eq!(window.reset_at_ms(), Some(expected));
}

#[test]
fn is_fresh_stale_is_always_false() {
    let snapshot = UsageSnapshot {
        stale: true,
        age_seconds: Some(0),
        ..Default::default()
    };
    assert!(!snapshot.is_fresh(3600));
}

#[test]
fn is_fresh_tracks_the_age_limit() {
    let at = |age| UsageSnapshot {
        stale: false,
        age_seconds: Some(age),
        ..Default::default()
    };
    assert!(at(100).is_fresh(200));
    assert!(at(200).is_fresh(200), "exactly at the limit is fresh");
    assert!(!at(300).is_fresh(200));
}

#[test]
fn is_fresh_missing_age_counts_as_fresh_when_not_stale() {
    let snapshot = UsageSnapshot {
        stale: false,
        age_seconds: None,
        ..Default::default()
    };
    assert!(snapshot.is_fresh(200));
}

#[test]
fn fetch_local_reads_the_history_file() {
    let now = now_ms();
    let file = history_file(&history_json(now, 60, 42.0));
    let snapshot = fetch("local", 900, file.path().to_str()).expect("should read");

    assert_eq!(snapshot.source.as_deref(), Some("local"));
    assert_eq!(
        snapshot.five_hour.as_ref().unwrap().used_percentage,
        Some(42.0)
    );
    assert_eq!(
        snapshot.seven_day.as_ref().unwrap().used_percentage,
        Some(18.0)
    );
    assert!(snapshot.is_fresh(900));
}

#[test]
fn fetch_local_marks_an_old_sample_stale_without_failing() {
    let now = now_ms();
    let file = history_file(&history_json(now, 4000, 42.0));
    let snapshot = fetch("local", 900, file.path().to_str()).expect("stale is not an error");

    assert!(snapshot.stale);
    assert!(!snapshot.is_fresh(900));
}

#[test]
fn fetch_local_errors_when_the_history_file_is_missing() {
    let err = fetch("local", 900, Some("/nonexistent/history.json")).expect_err("should fail");
    assert!(format!("{err:#}").contains("usage history"), "got: {err:#}");
}

#[test]
fn fetch_auto_uses_fresh_local_data_without_touching_credentials() {
    let now = now_ms();
    let file = history_file(&history_json(now, 30, 60.0));
    let snapshot = fetch("auto", 900, file.path().to_str()).expect("should read");

    assert_eq!(
        snapshot.source.as_deref(),
        Some("local"),
        "fresh local data must short-circuit the api source"
    );
    assert_eq!(
        snapshot.five_hour.unwrap().used_percentage,
        Some(60.0)
    );
}

#[test]
fn format_age_covers_each_scale() {
    assert_eq!(format_age(0), "just now");
    assert_eq!(format_age(-30), "just now", "clock skew is not a duration");
    assert_eq!(format_age(45), "45s");
    assert_eq!(format_age(90), "1m");
    assert_eq!(format_age(3600), "1h");
    assert_eq!(format_age(12_720), "3h 32m");
    assert_eq!(format_age(93_600), "1d 2h");
}

/// A local snapshot sampled four minutes ago, with `five_hour` as given.
fn local_snapshot(five_hour: UsageWindow) -> UsageSnapshot {
    UsageSnapshot {
        source: Some("local".to_string()),
        sampled_at_ms: Some(1_784_989_528_179),
        age_seconds: Some(240),
        stale: false,
        five_hour: Some(five_hour),
        seven_day: Some(UsageWindow::from_percentage(18.0)),
        extra_usage_dollars: None,
    }
}

#[test]
fn render_marks_a_derived_reset_as_an_estimate() {
    let sampled = 1_784_989_528_179_i64;
    let out = render(&local_snapshot(UsageWindow {
        used_percentage: Some(79.0),
        resets_at: None,
        // 1h 20m after "now" (sampled + 240s).
        resets_at_estimated_ms: Some(sampled + 240_000 + 4_800_000),
    }));

    assert!(out.contains("5h window"), "got: {out}");
    assert!(out.contains("79% used"), "got: {out}");
    assert!(out.contains("resets in ~1h 20m (est)"), "got: {out}");
    assert!(out.contains("source: local, sampled 4m ago"), "got: {out}");
}

#[test]
fn render_omits_the_reset_column_when_it_is_unknown() {
    let out = render(&local_snapshot(UsageWindow::from_percentage(0.0)));
    assert!(out.contains("0% used"), "got: {out}");
    assert!(!out.contains("resets"), "got: {out}");
}

#[test]
fn render_does_not_call_an_authoritative_reset_an_estimate() {
    let sampled = 1_784_989_528_179_i64;
    let mut snapshot = local_snapshot(UsageWindow {
        used_percentage: Some(60.0),
        resets_at: Some("2026-07-25T19:19:59Z".to_string()),
        resets_at_estimated_ms: None,
    });
    snapshot.source = Some("api".to_string());
    snapshot.age_seconds = Some(0);
    snapshot.sampled_at_ms = Some(sampled);

    let out = render(&snapshot);
    assert!(out.contains("source: api, live"), "got: {out}");
    assert!(!out.contains("(est)"), "got: {out}");
    assert!(out.contains("resets in ~"), "got: {out}");
}

#[test]
fn render_flags_a_stale_local_sample() {
    let mut snapshot = local_snapshot(UsageWindow::from_percentage(40.0));
    snapshot.stale = true;
    snapshot.age_seconds = Some(7200);

    let out = render(&snapshot);
    assert!(out.contains("sampled 2h ago"), "got: {out}");
    assert!(out.contains("stale"), "got: {out}");
}

#[test]
fn render_shows_extra_usage_dollars() {
    let mut snapshot = local_snapshot(UsageWindow::from_percentage(0.0));
    snapshot.extra_usage_dollars = Some(41.66);
    assert!(render(&snapshot).contains("extra usage: $41.66"));
}

#[test]
fn render_says_now_for_a_reset_that_has_passed() {
    let sampled = 1_784_989_528_179_i64;
    let out = render(&local_snapshot(UsageWindow {
        used_percentage: Some(10.0),
        resets_at: None,
        resets_at_estimated_ms: Some(sampled),
    }));
    assert!(out.contains("resets in ~now"), "got: {out}");
}

#[test]
fn source_unavailable_only_flags_a_pinned_local_source() {
    assert!(source_unavailable("local", Some("/nonexistent/history.json")));
    assert!(
        !source_unavailable("auto", Some("/nonexistent/history.json")),
        "auto can still fall back to the api"
    );
    assert!(!source_unavailable("api", Some("/nonexistent/history.json")));
}

#[test]
fn source_unavailable_is_false_when_the_history_file_exists() {
    let file = history_file(&history_json(now_ms(), 60, 10.0));
    assert!(!source_unavailable("local", file.path().to_str()));
}
