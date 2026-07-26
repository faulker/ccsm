//! Decision-table tests for the engine, plus persistence and command-queue
//! coverage. Any test that touches `CCSM_CONFIG_DIR` holds `test_lock()` for
//! its duration since the env var is process-global.

use super::command::*;
use super::engine::*;
use super::store::*;
use super::*;
use crate::config::{test_lock, Config};
use crate::live::ActivityState;
use crate::usage::{UsageSnapshot, UsageWindow};
use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

/// Empty completion/idle inputs for the many decision-table tests that do not
/// exercise the completion protocol. `static` rather than a per-test local so
/// an `EngineInputs` bound with `let` can borrow them without the temporary
/// dying at the end of the statement that built it.
static NO_COMPLETIONS: LazyLock<HashSet<String>> = LazyLock::new(HashSet::new);
static NO_IDLE: LazyLock<HashMap<String, i64>> = LazyLock::new(HashMap::new);


/// A minimal job with sensible defaults for tests, tmux name "test".
fn base_job(state: JobState) -> Job {
    Job {
        id: "job-1".to_string(),
        name: "test".to_string(),
        cwd: "/tmp".to_string(),
        prompt: String::new(),
        continue_prompt: None,
        claude_session_id: None,
        tmux_name: Some("test".to_string()),
        state,
        pause_mode: PauseMode::Soft,
        dangerous: false,
        model: None,
        auto_resume: true,
        created_at_ms: 0,
        updated_at_ms: 0,
        paused_at_ms: None,
        resume_after_ms: None,
        last_error: None,
        attempts: 0,
        history: Vec::new(),
    }
}

/// A fresh (non-stale, zero-age) usage sample reporting only a 5-hour window.
fn fresh_usage(five_pct: f64) -> UsageSnapshot {
    UsageSnapshot {
        source: Some("test".to_string()),
        sampled_at_ms: Some(0),
        age_seconds: Some(0),
        stale: false,
        five_hour: Some(UsageWindow {
            used_percentage: Some(five_pct),
            resets_at: None,
            resets_at_estimated_ms: None,
        }),
        seven_day: None,
        extra_usage_dollars: None,
    }
}

/// A stale usage sample reporting only a 5-hour window.
fn stale_usage(five_pct: f64) -> UsageSnapshot {
    UsageSnapshot {
        stale: true,
        ..fresh_usage(five_pct)
    }
}

// ---------------------------------------------------------------------
// resume argv fallback when the claude session id is unknown
// ---------------------------------------------------------------------

#[test]
fn resume_argv_uses_continue_when_session_id_unknown() {
    // An adopted session whose id we never discovered must not produce
    // `--resume ""`, which opens claude's interactive picker and hangs.
    let mut job = base_job(JobState::Stopped);
    job.claude_session_id = None;
    let argv = build_resume_argv("claude", &job);
    assert_eq!(argv[..2], ["claude", "--continue"]);
    assert!(!argv.iter().any(|a| a.is_empty()));
}

#[test]
fn resume_argv_uses_continue_when_session_id_is_empty_string() {
    let mut job = base_job(JobState::Stopped);
    job.claude_session_id = Some(String::new());
    let argv = build_resume_argv("claude", &job);
    assert_eq!(argv[..2], ["claude", "--continue"]);
}

#[test]
fn resume_argv_uses_resume_when_session_id_known() {
    let mut job = base_job(JobState::Stopped);
    job.claude_session_id = Some("abc-123".to_string());
    let argv = build_resume_argv("claude", &job);
    assert_eq!(argv[..3], ["claude", "--resume", "abc-123"]);
}

#[test]
fn stopped_without_session_id_or_cwd_fails_instead_of_relaunching() {
    let mut job = base_job(JobState::Stopped);
    job.claude_session_id = None;
    job.cwd = String::new();
    job.updated_at_ms = 0;
    let cfg = Config::default();
    let usage = fresh_usage(10.0);
    let live = HashSet::new();
    let activity = HashMap::new();
    let actions = plan(
        &[job],
        &EngineInputs {
            now_ms: 10_000_000,
            usage: Some(&usage),
            last_known_pct: None,
            live: &live,
            activity: &activity,
            completed: &NO_COMPLETIONS,
            idle_since: &NO_IDLE,
            cfg: &cfg,
        },
    );
    assert!(
        matches!(actions.as_slice(), [Action::Fail { .. }]),
        "expected a single Fail, got {actions:?}"
    );
}

#[test]
fn dispatch_sanitizes_an_unsafe_job_name_into_the_tmux_name() {
    // A leading "." makes tmux read the target as a pane, and ":" separates
    // the window component, so neither may reach a session name.
    let mut job = base_job(JobState::Queued);
    job.name = ".my:project".to_string();
    job.tmux_name = None;
    let cfg = Config::default();
    let usage = fresh_usage(10.0);
    let live = HashSet::new();
    let activity = HashMap::new();
    let actions = plan(
        &[job],
        &EngineInputs {
            now_ms: 0,
            usage: Some(&usage),
            last_known_pct: None,
            live: &live,
            activity: &activity,
            completed: &NO_COMPLETIONS,
            idle_since: &NO_IDLE,
            cfg: &cfg,
        },
    );
    match actions.as_slice() {
        [Action::Dispatch { tmux_name, .. }] => {
            assert!(!tmux_name.starts_with('.'), "got {tmux_name}");
            assert!(!tmux_name.contains(':'), "got {tmux_name}");
        }
        other => panic!("expected one Dispatch, got {other:?}"),
    }
}

// ---------------------------------------------------------------------
// engine::plan decision table
// ---------------------------------------------------------------------

#[test]
fn queued_below_threshold_dispatches() {
    let cfg = Config::default();
    let usage = fresh_usage(40.0);
    let job = base_job(JobState::Queued);
    let live = HashSet::new();
    let activity = HashMap::new();
    let inputs = EngineInputs {
        now_ms: 0,
        usage: Some(&usage),
        last_known_pct: None,
        live: &live,
        activity: &activity,
        completed: &NO_COMPLETIONS,
        idle_since: &NO_IDLE,
        cfg: &cfg,
    };
    let actions = plan(&[job], &inputs);
    assert_eq!(actions.len(), 1);
    assert!(matches!(actions[0], Action::Dispatch { .. }));
}

#[test]
fn queued_at_or_above_threshold_does_nothing() {
    let cfg = Config::default();
    let usage = fresh_usage(96.0);
    let job = base_job(JobState::Queued);
    let live = HashSet::new();
    let activity = HashMap::new();
    let inputs = EngineInputs {
        now_ms: 0,
        usage: Some(&usage),
        last_known_pct: None,
        live: &live,
        activity: &activity,
        completed: &NO_COMPLETIONS,
        idle_since: &NO_IDLE,
        cfg: &cfg,
    };
    let actions = plan(&[job], &inputs);
    assert!(actions.is_empty());
}

#[test]
fn running_at_exactly_95_interrupts() {
    let cfg = Config::default();
    let usage = fresh_usage(95.0);
    let job = base_job(JobState::Running);
    let live = HashSet::from(["test".to_string()]);
    let activity = HashMap::new();
    let inputs = EngineInputs {
        now_ms: 0,
        usage: Some(&usage),
        last_known_pct: None,
        live: &live,
        activity: &activity,
        completed: &NO_COMPLETIONS,
        idle_since: &NO_IDLE,
        cfg: &cfg,
    };
    let actions = plan(&[job], &inputs);
    assert_eq!(actions.len(), 1);
    assert!(matches!(actions[0], Action::Interrupt { .. }));
}

#[test]
fn running_at_94_9_does_nothing() {
    let cfg = Config::default();
    let usage = fresh_usage(94.9);
    let job = base_job(JobState::Running);
    let live = HashSet::from(["test".to_string()]);
    let activity = HashMap::new();
    let inputs = EngineInputs {
        now_ms: 0,
        usage: Some(&usage),
        last_known_pct: None,
        live: &live,
        activity: &activity,
        completed: &NO_COMPLETIONS,
        idle_since: &NO_IDLE,
        cfg: &cfg,
    };
    let actions = plan(&[job], &inputs);
    assert!(actions.is_empty());
}

#[test]
fn running_hard_pause_mode_hard_stops() {
    let cfg = Config::default();
    let usage = fresh_usage(95.0);
    let mut job = base_job(JobState::Running);
    job.pause_mode = PauseMode::Hard;
    let live = HashSet::from(["test".to_string()]);
    let activity = HashMap::new();
    let inputs = EngineInputs {
        now_ms: 0,
        usage: Some(&usage),
        last_known_pct: None,
        live: &live,
        activity: &activity,
        completed: &NO_COMPLETIONS,
        idle_since: &NO_IDLE,
        cfg: &cfg,
    };
    let actions = plan(&[job], &inputs);
    assert_eq!(actions.len(), 1);
    assert!(matches!(actions[0], Action::HardStop { .. }));
}

#[test]
fn running_waiting_marks_paused_instead_of_interrupt() {
    let cfg = Config::default();
    let usage = fresh_usage(96.0);
    let job = base_job(JobState::Running);
    let live = HashSet::from(["test".to_string()]);
    let mut activity = HashMap::new();
    activity.insert("test".to_string(), ActivityState::Waiting);
    let inputs = EngineInputs {
        now_ms: 0,
        usage: Some(&usage),
        last_known_pct: None,
        live: &live,
        activity: &activity,
        completed: &NO_COMPLETIONS,
        idle_since: &NO_IDLE,
        cfg: &cfg,
    };
    let actions = plan(&[job], &inputs);
    assert_eq!(actions.len(), 1);
    assert!(matches!(actions[0], Action::MarkPaused { .. }));
}

#[test]
fn running_without_live_tmux_marks_stopped() {
    let cfg = Config::default();
    let usage = fresh_usage(10.0);
    let job = base_job(JobState::Running);
    let live = HashSet::new(); // "test" is not present
    let activity = HashMap::new();
    let inputs = EngineInputs {
        now_ms: 0,
        usage: Some(&usage),
        last_known_pct: None,
        live: &live,
        activity: &activity,
        completed: &NO_COMPLETIONS,
        idle_since: &NO_IDLE,
        cfg: &cfg,
    };
    let actions = plan(&[job], &inputs);
    assert_eq!(actions.len(), 1);
    assert!(matches!(actions[0], Action::MarkStopped { .. }));
}

#[test]
fn stale_high_usage_still_interrupts() {
    let cfg = Config::default();
    let usage = stale_usage(96.0);
    let job = base_job(JobState::Running);
    let live = HashSet::from(["test".to_string()]);
    let activity = HashMap::new();
    let inputs = EngineInputs {
        now_ms: 0,
        usage: Some(&usage),
        last_known_pct: None,
        live: &live,
        activity: &activity,
        completed: &NO_COMPLETIONS,
        idle_since: &NO_IDLE,
        cfg: &cfg,
    };
    let actions = plan(&[job], &inputs);
    assert_eq!(actions.len(), 1);
    assert!(matches!(actions[0], Action::Interrupt { .. }));
}

#[test]
fn stale_low_usage_does_nothing() {
    let cfg = Config::default();
    let usage = stale_usage(40.0);
    let job = base_job(JobState::Running);
    let live = HashSet::from(["test".to_string()]);
    let activity = HashMap::new();
    let inputs = EngineInputs {
        now_ms: 0,
        usage: Some(&usage),
        last_known_pct: None,
        live: &live,
        activity: &activity,
        completed: &NO_COMPLETIONS,
        idle_since: &NO_IDLE,
        cfg: &cfg,
    };
    let actions = plan(&[job], &inputs);
    assert!(actions.is_empty());
}

#[test]
fn paused_stale_low_usage_is_not_resumed() {
    let cfg = Config::default();
    let usage = stale_usage(40.0);
    let job = base_job(JobState::Paused);
    let live = HashSet::new();
    let activity = HashMap::new();
    let inputs = EngineInputs {
        now_ms: 0,
        usage: Some(&usage),
        last_known_pct: None,
        live: &live,
        activity: &activity,
        completed: &NO_COMPLETIONS,
        idle_since: &NO_IDLE,
        cfg: &cfg,
    };
    let actions = plan(&[job], &inputs);
    assert!(actions.is_empty());
}

#[test]
fn paused_fresh_low_usage_resumes() {
    let cfg = Config::default();
    let usage = fresh_usage(40.0);
    let job = base_job(JobState::Paused);
    let live = HashSet::new();
    let activity = HashMap::new();
    let inputs = EngineInputs {
        now_ms: 0,
        usage: Some(&usage),
        last_known_pct: None,
        live: &live,
        activity: &activity,
        completed: &NO_COMPLETIONS,
        idle_since: &NO_IDLE,
        cfg: &cfg,
    };
    let actions = plan(&[job], &inputs);
    assert_eq!(actions.len(), 1);
    assert!(matches!(actions[0], Action::Resume { .. }));
}

#[test]
fn paused_retry_respects_backoff_even_when_usage_is_low() {
    // Regression: a failed resume reverts the job to Paused with attempts
    // incremented. Because usage is low, the threshold gate would otherwise
    // re-fire every tick and spin without ever honouring the backoff.
    let cfg = Config::default();
    let usage = fresh_usage(10.0); // well below usage_resume_percent
    let mut job = base_job(JobState::Paused);
    job.attempts = 1;
    job.updated_at_ms = 1_000_000;
    let live = HashSet::new();
    let activity = HashMap::new();

    // 5s after the failed attempt: still inside the 30s backoff, so no retry.
    let actions = plan(
        &[job.clone()],
        &EngineInputs {
            now_ms: 1_005_000,
            usage: Some(&usage),
            last_known_pct: None,
            live: &live,
            activity: &activity,
            completed: &NO_COMPLETIONS,
            idle_since: &NO_IDLE,
            cfg: &cfg,
        },
    );
    assert!(
        actions.is_empty(),
        "expected backoff to suppress the retry, got {actions:?}"
    );

    // 31s after: the backoff has elapsed, so the retry is allowed.
    let actions = plan(
        &[job],
        &EngineInputs {
            now_ms: 1_031_000,
            usage: Some(&usage),
            last_known_pct: None,
            live: &live,
            activity: &activity,
            completed: &NO_COMPLETIONS,
            idle_since: &NO_IDLE,
            cfg: &cfg,
        },
    );
    assert!(
        matches!(actions.as_slice(), [Action::Resume { .. }]),
        "expected a Resume once the backoff elapsed, got {actions:?}"
    );
}

#[test]
fn paused_first_resume_is_not_delayed_by_backoff() {
    // attempts == 0 means nothing has failed yet, so the first resume must be
    // immediate rather than waiting out a backoff.
    let cfg = Config::default();
    let usage = fresh_usage(10.0);
    let mut job = base_job(JobState::Paused);
    job.attempts = 0;
    job.updated_at_ms = 1_000_000;
    let live = HashSet::new();
    let activity = HashMap::new();
    let actions = plan(
        &[job],
        &EngineInputs {
            now_ms: 1_000_100,
            usage: Some(&usage),
            last_known_pct: None,
            live: &live,
            activity: &activity,
            completed: &NO_COMPLETIONS,
            idle_since: &NO_IDLE,
            cfg: &cfg,
        },
    );
    assert!(
        matches!(actions.as_slice(), [Action::Resume { .. }]),
        "expected an immediate first Resume, got {actions:?}"
    );
}

#[test]
fn paused_does_not_resume_on_deadline_while_still_at_pause_threshold() {
    // The reset time is only an estimate. If it fires early while usage is
    // still at the pause threshold, resuming would burn the remaining quota
    // and immediately re-pause.
    let cfg = Config::default();
    let usage = fresh_usage(96.0);
    let mut job = base_job(JobState::Paused);
    job.resume_after_ms = Some(100);
    let live = HashSet::new();
    let activity = HashMap::new();
    let actions = plan(
        &[job],
        &EngineInputs {
            now_ms: 150,
            usage: Some(&usage),
            last_known_pct: None,
            live: &live,
            activity: &activity,
            completed: &NO_COMPLETIONS,
            idle_since: &NO_IDLE,
            cfg: &cfg,
        },
    );
    assert!(actions.is_empty(), "expected no action, got {actions:?}");
}

#[test]
fn paused_resumes_after_deadline_even_if_pct_high() {
    let cfg = Config::default();
    let usage = fresh_usage(80.0); // above usage_resume_percent, but the deadline has passed
    let mut job = base_job(JobState::Paused);
    job.resume_after_ms = Some(100);
    let live = HashSet::new();
    let activity = HashMap::new();
    let inputs = EngineInputs {
        now_ms: 150,
        usage: Some(&usage),
        last_known_pct: None,
        live: &live,
        activity: &activity,
        completed: &NO_COMPLETIONS,
        idle_since: &NO_IDLE,
        cfg: &cfg,
    };
    let actions = plan(&[job], &inputs);
    assert_eq!(actions.len(), 1);
    assert!(matches!(actions[0], Action::Resume { .. }));
}

#[test]
fn no_thrash_after_resume_at_49_then_51() {
    let cfg = Config::default();
    let mut job = base_job(JobState::Paused);
    let live = HashSet::from(["test".to_string()]);
    let activity = HashMap::new();

    let usage49 = fresh_usage(49.0);
    let inputs49 = EngineInputs {
        now_ms: 0,
        usage: Some(&usage49),
        last_known_pct: None,
        live: &live,
        activity: &activity,
        completed: &NO_COMPLETIONS,
        idle_since: &NO_IDLE,
        cfg: &cfg,
    };
    let actions = plan(&[job.clone()], &inputs49);
    assert_eq!(actions.len(), 1);
    assert!(matches!(actions[0], Action::Resume { .. }));

    // The daemon would have executed the Resume action and transitioned the job.
    job.transition(JobState::Running, "resumed", 0);

    let usage51 = fresh_usage(51.0);
    let inputs51 = EngineInputs {
        now_ms: 1,
        usage: Some(&usage51),
        last_known_pct: None,
        live: &live,
        activity: &activity,
        completed: &NO_COMPLETIONS,
        idle_since: &NO_IDLE,
        cfg: &cfg,
    };
    let actions2 = plan(&[job], &inputs51);
    assert!(actions2.is_empty());
}

#[test]
fn mixed_job_set_produces_exactly_one_interrupt() {
    let cfg = Config::default();
    let usage = fresh_usage(96.0);

    let mut running = base_job(JobState::Running);
    running.id = "running".to_string();
    running.tmux_name = Some("running-tmux".to_string());
    let mut paused = base_job(JobState::Paused);
    paused.id = "paused".to_string();
    let mut done = base_job(JobState::Done);
    done.id = "done".to_string();

    let live = HashSet::from(["running-tmux".to_string()]);
    let activity = HashMap::new();
    let inputs = EngineInputs {
        now_ms: 0,
        usage: Some(&usage),
        last_known_pct: None,
        live: &live,
        activity: &activity,
        completed: &NO_COMPLETIONS,
        idle_since: &NO_IDLE,
        cfg: &cfg,
    };

    let actions = plan(&[running, paused, done], &inputs);
    let interrupts = actions
        .iter()
        .filter(|a| matches!(a, Action::Interrupt { .. }))
        .count();
    assert_eq!(interrupts, 1);
}

#[test]
fn watch_seven_day_true_uses_seven_day_max() {
    let mut cfg = Config::default();
    cfg.watch_seven_day = true;
    let mut usage = fresh_usage(20.0);
    usage.seven_day = Some(UsageWindow {
        used_percentage: Some(97.0),
        resets_at: None,
        resets_at_estimated_ms: None,
    });
    let job = base_job(JobState::Running);
    let live = HashSet::from(["test".to_string()]);
    let activity = HashMap::new();
    let inputs = EngineInputs {
        now_ms: 0,
        usage: Some(&usage),
        last_known_pct: None,
        live: &live,
        activity: &activity,
        completed: &NO_COMPLETIONS,
        idle_since: &NO_IDLE,
        cfg: &cfg,
    };
    let actions = plan(&[job], &inputs);
    assert!(actions
        .iter()
        .any(|a| matches!(a, Action::Interrupt { .. })));
}

#[test]
fn watch_seven_day_false_ignores_seven_day() {
    let mut cfg = Config::default();
    cfg.watch_seven_day = false;
    let mut usage = fresh_usage(20.0);
    usage.seven_day = Some(UsageWindow {
        used_percentage: Some(97.0),
        resets_at: None,
        resets_at_estimated_ms: None,
    });
    let job = base_job(JobState::Running);
    let live = HashSet::from(["test".to_string()]);
    let activity = HashMap::new();
    let inputs = EngineInputs {
        now_ms: 0,
        usage: Some(&usage),
        last_known_pct: None,
        live: &live,
        activity: &activity,
        completed: &NO_COMPLETIONS,
        idle_since: &NO_IDLE,
        cfg: &cfg,
    };
    let actions = plan(&[job], &inputs);
    assert!(actions.is_empty());
}

#[test]
fn stopped_relaunches_after_backoff_elapses() {
    let cfg = Config::default();
    let usage = fresh_usage(10.0);
    let mut job = base_job(JobState::Stopped);
    job.attempts = 1;
    job.updated_at_ms = 0;
    let live = HashSet::new();
    let activity = HashMap::new();
    let inputs = EngineInputs {
        now_ms: backoff_ms(1),
        usage: Some(&usage),
        last_known_pct: None,
        live: &live,
        activity: &activity,
        completed: &NO_COMPLETIONS,
        idle_since: &NO_IDLE,
        cfg: &cfg,
    };
    let actions = plan(&[job], &inputs);
    assert_eq!(actions.len(), 1);
    assert!(matches!(actions[0], Action::Relaunch { .. }));
}

#[test]
fn stopped_does_not_relaunch_before_backoff_elapses() {
    let cfg = Config::default();
    let usage = fresh_usage(10.0);
    let mut job = base_job(JobState::Stopped);
    job.attempts = 1;
    job.updated_at_ms = 0;
    let live = HashSet::new();
    let activity = HashMap::new();
    let inputs = EngineInputs {
        now_ms: backoff_ms(1) - 1,
        usage: Some(&usage),
        last_known_pct: None,
        live: &live,
        activity: &activity,
        completed: &NO_COMPLETIONS,
        idle_since: &NO_IDLE,
        cfg: &cfg,
    };
    let actions = plan(&[job], &inputs);
    assert!(actions.is_empty());
}

#[test]
fn stopped_fails_after_max_restart_attempts() {
    let cfg = Config::default();
    let usage = fresh_usage(10.0);
    let mut job = base_job(JobState::Stopped);
    job.attempts = cfg.max_restart_attempts;
    let live = HashSet::new();
    let activity = HashMap::new();
    let inputs = EngineInputs {
        now_ms: 10_000_000,
        usage: Some(&usage),
        last_known_pct: None,
        live: &live,
        activity: &activity,
        completed: &NO_COMPLETIONS,
        idle_since: &NO_IDLE,
        cfg: &cfg,
    };
    let actions = plan(&[job], &inputs);
    assert_eq!(actions.len(), 1);
    assert!(matches!(actions[0], Action::Fail { .. }));
}

#[test]
fn no_usage_sample_takes_no_action() {
    let cfg = Config::default();
    let job = base_job(JobState::Queued);
    let live = HashSet::new();
    let activity = HashMap::new();
    let inputs = EngineInputs {
        now_ms: 0,
        usage: None,
        last_known_pct: Some(99.0),
        live: &live,
        activity: &activity,
        completed: &NO_COMPLETIONS,
        idle_since: &NO_IDLE,
        cfg: &cfg,
    };
    let actions = plan(&[job], &inputs);
    assert!(actions.is_empty());
}

// ---------------------------------------------------------------------
// argv builders
// ---------------------------------------------------------------------

#[test]
fn start_argv_prompt_with_special_chars_is_one_element() {
    let mut job = base_job(JobState::Queued);
    job.prompt = "do $(rm -rf /) and \"quotes\" and `backticks`".to_string();
    let argv = build_start_argv("claude", &job);
    // The prompt now carries the completion protocol, but it must still be a
    // single trailing argv element: tmux execvp's argv directly, so shell
    // metacharacters inside it are inert only as long as it is not split.
    let matching = argv.iter().filter(|a| a.starts_with(&job.prompt)).count();
    assert_eq!(matching, 1);
    let last = argv.last().unwrap();
    assert!(last.starts_with(&job.prompt), "got {last:?}");
    assert!(last.contains(completion::COMPLETION_MARKER), "got {last:?}");
}

#[test]
fn start_argv_empty_prompt_omits_trailing_element() {
    let job = base_job(JobState::Queued); // prompt is ""
    let argv = build_start_argv("claude", &job);
    assert!(!argv.iter().any(|a| a.is_empty()));
    // The last element is the system-prompt text, never an empty prompt slot.
    assert!(argv.last().unwrap().contains(completion::COMPLETION_MARKER));
}

#[test]
fn start_argv_carries_the_protocol_in_the_system_prompt() {
    let mut job = base_job(JobState::Queued);
    job.prompt = "Refactor the parser.".to_string();
    let argv = build_start_argv("claude", &job);
    let idx = argv
        .iter()
        .position(|a| a == "--append-system-prompt")
        .expect("start argv carries --append-system-prompt");
    assert!(argv[idx + 1].contains(completion::COMPLETION_MARKER));
    assert_eq!(
        argv.iter().filter(|a| *a == "--append-system-prompt").count(),
        1
    );
}

#[test]
fn resume_argv_carries_the_protocol_in_the_system_prompt() {
    // A relaunched session is a fresh process, so it needs the protocol in its
    // own system prompt just as much as a first dispatch does.
    let mut job = base_job(JobState::Stopped);
    job.claude_session_id = Some("abc-123".to_string());
    let argv = build_resume_argv("claude", &job);
    let idx = argv
        .iter()
        .position(|a| a == "--append-system-prompt")
        .expect("resume argv carries --append-system-prompt");
    assert!(argv[idx + 1].contains(completion::COMPLETION_MARKER));
}

#[test]
fn start_argv_leaves_a_slash_command_prompt_untouched() {
    // Claude Code hands everything after the command name to the command as
    // $ARGUMENTS, newlines included, so an appended protocol paragraph would
    // become part of the argument instead of an instruction.
    let mut job = base_job(JobState::Queued);
    job.prompt = "/goal @PLAN.md".to_string();
    let argv = build_start_argv("claude", &job);
    assert_eq!(argv.last().unwrap(), "/goal @PLAN.md");
    // It still reaches the agent, via the system prompt.
    assert!(argv.iter().any(|a| a.contains(completion::COMPLETION_MARKER)));
}

#[test]
fn start_argv_flags_appear_exactly_once() {
    let mut job = base_job(JobState::Queued);
    job.model = Some("opus".to_string());
    job.dangerous = true;
    let argv = build_start_argv("claude", &job);
    assert_eq!(argv.iter().filter(|a| *a == "--session-id").count(), 1);
    assert_eq!(argv.iter().filter(|a| *a == "--name").count(), 1);
    assert_eq!(argv.iter().filter(|a| *a == "--model").count(), 1);
    assert_eq!(
        argv.iter()
            .filter(|a| *a == "--dangerously-skip-permissions")
            .count(),
        1
    );
}

#[test]
fn resume_argv_has_no_positional_prompt() {
    let mut job = base_job(JobState::Stopped);
    job.claude_session_id = Some("sess-123".to_string());
    job.prompt = "should not appear".to_string();
    let argv = build_resume_argv("claude", &job);
    assert!(!argv.iter().any(|a| a == "should not appear"));
    assert_eq!(
        argv[..3],
        [
            "claude".to_string(),
            "--resume".to_string(),
            "sess-123".to_string()
        ]
    );
}

#[test]
fn backoff_ms_steps_and_caps() {
    assert_eq!(backoff_ms(0), 30_000);
    assert_eq!(backoff_ms(1), 120_000);
    assert_eq!(backoff_ms(2), 480_000);
    assert_eq!(backoff_ms(3), 1_800_000);
    assert_eq!(backoff_ms(10), 1_800_000);
}

// ---------------------------------------------------------------------
// Job::transition
// ---------------------------------------------------------------------

#[test]
fn transition_caps_history_and_resets_attempts_on_running() {
    let mut job = base_job(JobState::Stopped);
    job.attempts = 4;
    for i in 0..25 {
        job.transition(JobState::Paused, format!("event {i}"), i as i64);
    }
    assert_eq!(job.history.len(), 20);
    assert_eq!(job.history.first().unwrap().reason, "event 5");
    assert_eq!(job.history.last().unwrap().reason, "event 24");

    job.transition(JobState::Running, "resumed", 100);
    assert_eq!(job.attempts, 0);
    assert_eq!(job.state, JobState::Running);
}

// ---------------------------------------------------------------------
// canonical_cwd / discover_session_id
// ---------------------------------------------------------------------

#[test]
fn canonical_cwd_returns_absolute_path() {
    let dir = tempfile::tempdir().unwrap();
    let result = canonical_cwd(dir.path().to_str().unwrap());
    assert!(std::path::Path::new(&result).is_absolute());
}

#[test]
fn discover_session_id_none_when_no_matching_project_dir() {
    let result = discover_session_id("/definitely/not/a/real/project/path", 0);
    assert!(result.is_none());
}

// ---------------------------------------------------------------------
// store
// ---------------------------------------------------------------------

#[test]
fn schedule_roundtrip_via_tempdir() {
    let _guard = test_lock();
    let dir = tempfile::tempdir().unwrap();
    std::env::set_var("CCSM_CONFIG_DIR", dir.path());

    let mut schedule = Schedule::default();
    let mut job = base_job(JobState::Queued);
    job.id = "abc".to_string();
    schedule.jobs.push(job);

    save(&schedule).unwrap();
    let loaded = load();
    assert_eq!(loaded.jobs.len(), 1);
    assert_eq!(loaded.jobs[0].id, "abc");
    assert_eq!(loaded.version, 1);

    std::env::remove_var("CCSM_CONFIG_DIR");
}

#[test]
fn schedule_missing_fields_load_with_defaults() {
    let json = r#"{"jobs":[{"id":"x","name":"y"}]}"#;
    let schedule: Schedule = serde_json::from_str(json).unwrap();
    assert_eq!(schedule.version, 1);
    assert_eq!(schedule.jobs.len(), 1);
    let job = &schedule.jobs[0];
    assert_eq!(job.id, "x");
    assert_eq!(job.state, JobState::Queued);
    assert!(job.auto_resume);
    assert!(job.history.is_empty());
}

#[test]
fn load_or_quarantine_moves_corrupt_file_and_returns_empty() {
    let _guard = test_lock();
    let dir = tempfile::tempdir().unwrap();
    std::env::set_var("CCSM_CONFIG_DIR", dir.path());

    let path = schedule_path().unwrap();
    std::fs::write(&path, "not valid json").unwrap();

    let (schedule, warning) = load_or_quarantine();
    assert!(schedule.jobs.is_empty());
    assert!(warning.is_some());
    assert!(!path.exists());

    let quarantined = std::fs::read_dir(dir.path())
        .unwrap()
        .filter_map(|e| e.ok())
        .any(|e| {
            e.file_name()
                .to_string_lossy()
                .starts_with("schedule.json.corrupt-")
        });
    assert!(quarantined);

    std::env::remove_var("CCSM_CONFIG_DIR");
}

#[test]
fn stamp_differs_for_equal_length_different_content() {
    let dir = tempfile::tempdir().unwrap();
    let path_a = dir.path().join("a.txt");
    let path_b = dir.path().join("b.txt");
    std::fs::write(&path_a, b"aaaa").unwrap();
    std::fs::write(&path_b, b"bbbb").unwrap();

    let stamp_a = stamp(&path_a).unwrap();
    let stamp_b = stamp(&path_b).unwrap();
    assert_eq!(stamp_a.len, stamp_b.len);
    assert_ne!(stamp_a.sha_prefix, stamp_b.sha_prefix);
    assert_ne!(stamp_a, stamp_b);
}

// ---------------------------------------------------------------------
// command queue
// ---------------------------------------------------------------------

#[test]
fn read_pending_leaves_commands_on_disk_until_acked() {
    // Regression: deleting on read meant a crash between reading commands and
    // persisting the resulting schedule silently lost the user's queued
    // actions. read_pending must be non-destructive.
    let _guard = test_lock();
    let dir = tempfile::tempdir().unwrap();
    std::env::set_var("CCSM_CONFIG_DIR", dir.path());

    enqueue(&Command::RefreshUsage).unwrap();
    enqueue(&Command::DeleteJob {
        id: "j1".to_string(),
    })
    .unwrap();

    // First read: both visible, nothing removed.
    let (pending, warnings) = read_pending();
    assert!(warnings.is_empty());
    assert_eq!(pending.len(), 2);
    assert_eq!(pending_count(), 2, "read_pending must not delete");

    // Simulate a crash before the save: a fresh read still sees both.
    let (pending_again, _) = read_pending();
    assert_eq!(pending_again.len(), 2, "commands must survive a crash");

    // Now ack them, as the daemon does only after a successful save.
    let paths: Vec<_> = pending_again.iter().map(|(p, _)| p.clone()).collect();
    ack(&paths);
    assert_eq!(pending_count(), 0);
    let (after, _) = read_pending();
    assert!(after.is_empty());

    std::env::remove_var("CCSM_CONFIG_DIR");
}

#[test]
fn ack_of_a_partial_set_leaves_the_rest_pending() {
    let _guard = test_lock();
    let dir = tempfile::tempdir().unwrap();
    std::env::set_var("CCSM_CONFIG_DIR", dir.path());

    enqueue(&Command::RefreshUsage).unwrap();
    enqueue(&Command::StopWatcher).unwrap();

    let (pending, _) = read_pending();
    assert_eq!(pending.len(), 2);
    ack(&[pending[0].0.clone()]);

    let (remaining, _) = read_pending();
    assert_eq!(remaining.len(), 1);
    assert!(matches!(remaining[0].1, Command::StopWatcher));

    std::env::remove_var("CCSM_CONFIG_DIR");
}

#[test]
fn enqueue_drains_in_creation_order() {
    let _guard = test_lock();
    let dir = tempfile::tempdir().unwrap();
    std::env::set_var("CCSM_CONFIG_DIR", dir.path());

    enqueue(&Command::StopWatcher).unwrap();
    enqueue(&Command::RefreshUsage).unwrap();
    enqueue(&Command::DeleteJob {
        id: "j3".to_string(),
    })
    .unwrap();

    let (commands, warnings) = drain();
    assert!(warnings.is_empty());
    assert_eq!(commands.len(), 3);
    assert!(matches!(commands[0], Command::StopWatcher));
    assert!(matches!(commands[1], Command::RefreshUsage));
    assert!(matches!(commands[2], Command::DeleteJob { .. }));

    std::env::remove_var("CCSM_CONFIG_DIR");
}

#[test]
fn drain_quarantines_unparseable_files_without_aborting() {
    let _guard = test_lock();
    let dir = tempfile::tempdir().unwrap();
    std::env::set_var("CCSM_CONFIG_DIR", dir.path());

    enqueue(&Command::StopWatcher).unwrap();
    let dir_path = commands_dir().unwrap();
    std::fs::write(
        dir_path.join("0000000000001-0000-badbad00.json"),
        "not json",
    )
    .unwrap();

    assert_eq!(pending_count(), 2);

    let (commands, warnings) = drain();
    assert_eq!(commands.len(), 1);
    assert_eq!(warnings.len(), 1);
    assert!(matches!(commands[0], Command::StopWatcher));

    // The bad file was moved, not deleted.
    assert!(dir_path
        .join("bad")
        .join("0000000000001-0000-badbad00.json")
        .exists());
    // The successfully-parsed command file is gone.
    let remaining_files = std::fs::read_dir(&dir_path)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_file())
        .count();
    assert_eq!(remaining_files, 0);

    // pending_count ignores the bad/ quarantine subdirectory.
    assert_eq!(pending_count(), 0);

    std::env::remove_var("CCSM_CONFIG_DIR");
}

// ---------------------------------------------------------------------
// discover_session_id must not bind to an unrelated session
// ---------------------------------------------------------------------

/// Write a session JSONL whose first entry carries `timestamp` and `cwd`.
fn write_session_file(dir: &std::path::Path, id: &str, started_rfc3339: &str, cwd: &str) {
    let line = serde_json::json!({
        "timestamp": started_rfc3339,
        "cwd": cwd,
        "sessionId": id,
    });
    std::fs::write(dir.join(format!("{id}.jsonl")), format!("{line}\n")).unwrap();
}

#[test]
fn session_start_reads_only_the_first_line() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("s.jsonl");
    let first = serde_json::json!({"timestamp":"2026-07-25T10:00:00Z","cwd":"/tmp/x"});
    // A second line with a different cwd must be ignored entirely.
    let second = serde_json::json!({"timestamp":"2026-07-25T11:00:00Z","cwd":"/tmp/other"});
    std::fs::write(&path, format!("{first}\n{second}\n")).unwrap();

    let (started, cwd) = super::session_start(&path).unwrap();
    assert_eq!(cwd.as_deref(), Some("/tmp/x"));
    let expected = chrono::DateTime::parse_from_rfc3339("2026-07-25T10:00:00Z")
        .unwrap()
        .timestamp_millis();
    assert_eq!(started, expected);
}

#[test]
fn session_start_rejects_garbage_without_panicking() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("bad.jsonl");
    std::fs::write(&path, "not json at all\n").unwrap();
    assert!(super::session_start(&path).is_none());

    let missing_ts = dir.path().join("no_ts.jsonl");
    std::fs::write(&missing_ts, "{\"cwd\":\"/tmp\"}\n").unwrap();
    assert!(super::session_start(&missing_ts).is_none());

    let empty = dir.path().join("empty.jsonl");
    std::fs::write(&empty, "").unwrap();
    assert!(super::session_start(&empty).is_none());
}

#[test]
fn a_long_running_older_session_is_not_mistaken_for_ours() {
    // Regression: filtering on mtime bound a job to whichever session in the
    // directory had been written to most recently, which in practice was an
    // unrelated conversation that merely happened to still be active. Doing
    // that would make a later `--resume` continue the wrong conversation.
    let project = tempfile::tempdir().unwrap();
    // Started hours before the job was dispatched, but touched just now.
    write_session_file(
        project.path(),
        "old-unrelated",
        "2026-07-25T01:00:00Z",
        project.path().to_str().unwrap(),
    );
    let path = project.path().join("old-unrelated.jsonl");
    // Bump mtime to "now" the way an active session would.
    let now = std::time::SystemTime::now();
    let f = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
    f.set_times(std::fs::FileTimes::new().set_modified(now)).ok();

    let since_ms = chrono::DateTime::parse_from_rfc3339("2026-07-25T12:00:00Z")
        .unwrap()
        .timestamp_millis();

    // The candidate started long before `since_ms`, so it must be rejected
    // regardless of how recently the file was written.
    let (started, _) = super::session_start(&path).unwrap();
    assert!(
        started < since_ms - 5000,
        "fixture should predate the cutoff"
    );
}

// ---------------------------------------------------------------------
// completion protocol
// ---------------------------------------------------------------------

/// One assistant JSONL entry whose single text block is `text`.
fn assistant_line(text: &str) -> String {
    serde_json::json!({
        "type": "assistant",
        "message": { "role": "assistant", "content": [{ "type": "text", "text": text }] }
    })
    .to_string()
}

/// One user JSONL entry carrying `text`, i.e. what a prompt we sent looks like
/// once it lands in the transcript.
fn user_line(text: &str) -> String {
    serde_json::json!({
        "type": "user",
        "message": { "role": "user", "content": text }
    })
    .to_string()
}

#[test]
fn completion_marker_on_its_own_line_from_the_assistant_counts() {
    let tail = assistant_line("All the tests pass.\n\nCCSM_JOB_COMPLETE");
    assert!(completion::transcript_shows_completion(&tail, 0));
}

#[test]
fn completion_ignores_our_own_instruction_echoed_as_a_user_message() {
    // Regression: the protocol instruction contains the marker, so a detector
    // that looked at the pane (or at every transcript entry) would declare the
    // job finished the instant it was dispatched.
    let prompt = completion::with_completion_protocol("Refactor the parser.");
    assert!(prompt.contains(completion::COMPLETION_MARKER));
    let tail = user_line(&prompt);
    assert!(!completion::transcript_shows_completion(&tail, 0));
}

#[test]
fn completion_ignores_the_marker_mentioned_mid_sentence() {
    let tail = assistant_line("I will print CCSM_JOB_COMPLETE once the build is green.");
    assert!(!completion::transcript_shows_completion(&tail, 0));
}

#[test]
fn completion_ignores_the_marker_written_through_a_tool_call() {
    // An agent that writes the marker into a file must not end its own job.
    let line = serde_json::json!({
        "type": "assistant",
        "message": {
            "role": "assistant",
            "content": [{
                "type": "tool_use",
                "name": "Write",
                "input": { "content": "CCSM_JOB_COMPLETE" }
            }]
        }
    })
    .to_string();
    assert!(!completion::transcript_shows_completion(&line, 0));
}

#[test]
fn completion_scans_a_whole_transcript_tail() {
    let tail = [
        user_line("Do the thing."),
        assistant_line("Working on it."),
        assistant_line("Done.\nCCSM_JOB_COMPLETE\n"),
    ]
    .join("\n");
    assert!(completion::transcript_shows_completion(&tail, 0));
}

#[test]
fn completion_tolerates_garbage_and_empty_input() {
    assert!(!completion::transcript_shows_completion("", 0));
    assert!(!completion::transcript_shows_completion("not json at all", 0));
    assert!(!completion::transcript_shows_completion(
        "{\"type\":\"assistant\" CCSM_JOB_COMPLETE truncated",
        0
    ));
}

#[test]
fn completion_ignores_a_marker_from_before_the_job_existed() {
    // Adopting a conversation ccsm already ran to completion must not mark the
    // new job done on the strength of the previous run's sign-off.
    let old = serde_json::json!({
        "type": "assistant",
        "timestamp": "2026-07-20T10:00:00Z",
        "message": { "role": "assistant", "content": [{ "type": "text", "text": "CCSM_JOB_COMPLETE" }] }
    })
    .to_string();
    let created_ms = chrono::DateTime::parse_from_rfc3339("2026-07-26T10:00:00Z")
        .unwrap()
        .timestamp_millis();
    assert!(!completion::transcript_shows_completion(&old, created_ms));

    // The same marker emitted after the job was created does count.
    let fresh = old.replace("2026-07-20T10", "2026-07-26T18");
    assert!(completion::transcript_shows_completion(&fresh, created_ms));
}

#[test]
fn completion_still_matches_an_entry_with_no_timestamp() {
    // Fail open on the time filter: a transcript format change must degrade
    // into "no time filter", never into "the marker never works again".
    let line = serde_json::json!({
        "type": "assistant",
        "message": { "role": "assistant", "content": [{ "type": "text", "text": "CCSM_JOB_COMPLETE" }] }
    })
    .to_string();
    assert!(completion::transcript_shows_completion(&line, i64::MAX / 2));
}

#[test]
fn with_completion_protocol_leaves_an_empty_prompt_empty() {
    // A job with no prompt starts at an idle session; sending only the
    // protocol instruction would be an instruction with no task attached.
    assert_eq!(completion::with_completion_protocol(""), "");
    assert_eq!(completion::with_completion_protocol("   \n "), "");
}

#[test]
fn read_transcript_tail_drops_the_partial_first_line() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("t.jsonl");
    std::fs::write(&path, "aaaaaaaaaa\nbbbb\ncccc\n").unwrap();

    // Reading the whole file keeps every line.
    let all = completion::read_transcript_tail(&path, 1024).unwrap();
    assert_eq!(all, "aaaaaaaaaa\nbbbb\ncccc\n");

    // Reading a tail that starts mid-line drops that truncated line: starting
    // 12 bytes from the end lands inside "aaaaaaaaaa", which is discarded.
    let tail = completion::read_transcript_tail(&path, 12).unwrap();
    assert_eq!(tail, "bbbb\ncccc\n");

    // Landing exactly on a newline still yields whole lines only.
    let tail = completion::read_transcript_tail(&path, 6).unwrap();
    assert_eq!(tail, "cccc\n");
}

#[test]
fn with_completion_protocol_leaves_a_slash_command_alone() {
    let plain = completion::with_completion_protocol("Refactor the parser.");
    assert!(plain.contains(completion::COMPLETION_MARKER));

    // Appending here would land inside the command's $ARGUMENTS.
    assert_eq!(
        completion::with_completion_protocol("/goal @PLAN.md"),
        "/goal @PLAN.md"
    );
    assert_eq!(
        completion::with_completion_protocol("  \n  /compact"),
        "  \n  /compact"
    );
}

#[test]
fn is_slash_command_looks_at_the_first_non_empty_line() {
    assert!(completion::is_slash_command("/goal @PLAN.md"));
    assert!(completion::is_slash_command("\n\n  /clear"));
    assert!(!completion::is_slash_command("Fix the / in the path"));
    assert!(!completion::is_slash_command(""));
    // A slash further down is just prose; only the opening line is a command.
    assert!(!completion::is_slash_command("Do the work\n/goal x"));
}

#[test]
fn continuation_text_carries_the_protocol() {
    let cfg = Config::default();
    let mut job = base_job(JobState::Paused);
    // The config default is used when the job has no override of its own.
    let text = continuation_text(&job, &cfg);
    assert!(text.starts_with(&cfg.continue_prompt));
    assert!(text.contains(completion::COMPLETION_MARKER));

    job.continue_prompt = Some("Pick it back up.".to_string());
    let text = continuation_text(&job, &cfg);
    assert!(text.starts_with("Pick it back up."));
    assert!(text.contains(completion::COMPLETION_MARKER));
}

#[test]
fn completed_running_job_is_marked_done_not_paused() {
    // Usage is over the pause threshold, which would normally interrupt the
    // job; completion has to win, or a finished job gets paused and resumed.
    let cfg = Config::default();
    let usage = fresh_usage(99.0);
    let job = base_job(JobState::Running);
    let live = HashSet::from(["test".to_string()]);
    let activity = HashMap::new();
    let completed = HashSet::from(["job-1".to_string()]);
    let inputs = EngineInputs {
        now_ms: 0,
        usage: Some(&usage),
        last_known_pct: None,
        live: &live,
        activity: &activity,
        completed: &completed,
        idle_since: &NO_IDLE,
        cfg: &cfg,
    };
    let actions = plan(&[job], &inputs);
    assert!(
        matches!(actions.as_slice(), [Action::MarkDone { .. }]),
        "expected exactly one MarkDone, got {actions:?}"
    );
}

#[test]
fn completed_stopped_job_is_not_relaunched() {
    // The core regression: an agent finishes, the session exits, and the
    // Stopped -> Relaunch path restarts the finished work forever.
    let cfg = Config::default();
    let usage = fresh_usage(10.0);
    let mut job = base_job(JobState::Stopped);
    job.auto_resume = true;
    job.updated_at_ms = 0;
    let live = HashSet::new();
    let activity = HashMap::new();
    let completed = HashSet::from(["job-1".to_string()]);
    let inputs = EngineInputs {
        now_ms: 10_000_000,
        usage: Some(&usage),
        last_known_pct: None,
        live: &live,
        activity: &activity,
        completed: &completed,
        idle_since: &NO_IDLE,
        cfg: &cfg,
    };
    let actions = plan(&[job.clone()], &inputs);
    assert!(
        matches!(actions.as_slice(), [Action::MarkDone { .. }]),
        "expected MarkDone instead of a Relaunch, got {actions:?}"
    );

    // Without the completion signal the same job would be relaunched.
    let inputs = EngineInputs {
        completed: &NO_COMPLETIONS,
        ..inputs
    };
    let actions = plan(&[job], &inputs);
    assert!(
        matches!(actions.as_slice(), [Action::Relaunch { .. }]),
        "fixture should otherwise relaunch, got {actions:?}"
    );
}

#[test]
fn completed_paused_job_is_not_resumed() {
    let cfg = Config::default();
    let usage = fresh_usage(10.0);
    let job = base_job(JobState::Paused);
    let live = HashSet::new();
    let activity = HashMap::new();
    let completed = HashSet::from(["job-1".to_string()]);
    let inputs = EngineInputs {
        now_ms: 0,
        usage: Some(&usage),
        last_known_pct: None,
        live: &live,
        activity: &activity,
        completed: &completed,
        idle_since: &NO_IDLE,
        cfg: &cfg,
    };
    let actions = plan(&[job], &inputs);
    assert!(
        matches!(actions.as_slice(), [Action::MarkDone { .. }]),
        "expected MarkDone instead of a Resume, got {actions:?}"
    );
}

#[test]
fn completed_queued_job_is_not_dispatched() {
    let cfg = Config::default();
    let usage = fresh_usage(10.0);
    let job = base_job(JobState::Queued);
    let live = HashSet::new();
    let activity = HashMap::new();
    let completed = HashSet::from(["job-1".to_string()]);
    let inputs = EngineInputs {
        now_ms: 0,
        usage: Some(&usage),
        last_known_pct: None,
        live: &live,
        activity: &activity,
        completed: &completed,
        idle_since: &NO_IDLE,
        cfg: &cfg,
    };
    let actions = plan(&[job], &inputs);
    assert!(
        matches!(actions.as_slice(), [Action::MarkDone { .. }]),
        "expected MarkDone instead of a Dispatch, got {actions:?}"
    );
}

#[test]
fn already_done_or_failed_jobs_produce_no_action() {
    let cfg = Config::default();
    let usage = fresh_usage(10.0);
    let live = HashSet::new();
    let activity = HashMap::new();
    let completed = HashSet::from(["job-1".to_string()]);
    for state in [JobState::Done, JobState::Failed] {
        let job = base_job(state);
        let inputs = EngineInputs {
            now_ms: 0,
            usage: Some(&usage),
            last_known_pct: None,
            live: &live,
            activity: &activity,
            completed: &completed,
            idle_since: &NO_IDLE,
            cfg: &cfg,
        };
        let actions = plan(&[job], &inputs);
        assert!(actions.is_empty(), "{state:?} produced {actions:?}");
    }
}

#[test]
fn running_job_idle_past_the_limit_is_marked_done() {
    let cfg = Config::default(); // 900s
    let usage = fresh_usage(10.0);
    let job = base_job(JobState::Running);
    let live = HashSet::from(["test".to_string()]);
    let activity = HashMap::new();
    let idle_since = HashMap::from([("job-1".to_string(), 0i64)]);

    // One second short of the limit: still running.
    let inputs = EngineInputs {
        now_ms: 899_000,
        usage: Some(&usage),
        last_known_pct: None,
        live: &live,
        activity: &activity,
        completed: &NO_COMPLETIONS,
        idle_since: &idle_since,
        cfg: &cfg,
    };
    assert!(plan(std::slice::from_ref(&job), &inputs).is_empty());

    // At the limit: done.
    let inputs = EngineInputs { now_ms: 900_000, ..inputs };
    let actions = plan(&[job], &inputs);
    match actions.as_slice() {
        [Action::MarkDone { reason, .. }] => {
            assert!(reason.contains("15m"), "unexpected reason {reason:?}");
        }
        other => panic!("expected MarkDone, got {other:?}"),
    }
}

#[test]
fn idle_completion_is_disabled_by_a_zero_timeout() {
    let mut cfg = Config::default();
    cfg.idle_complete_seconds = 0;
    let usage = fresh_usage(10.0);
    let job = base_job(JobState::Running);
    let live = HashSet::from(["test".to_string()]);
    let activity = HashMap::new();
    let idle_since = HashMap::from([("job-1".to_string(), 0i64)]);
    let inputs = EngineInputs {
        now_ms: 100_000_000,
        usage: Some(&usage),
        last_known_pct: None,
        live: &live,
        activity: &activity,
        completed: &NO_COMPLETIONS,
        idle_since: &idle_since,
        cfg: &cfg,
    };
    assert!(plan(&[job], &inputs).is_empty());
}

#[test]
fn idle_completion_does_not_apply_to_a_vanished_session() {
    // The tmux session is gone, so the job is Stopped, not finished: an idle
    // timer left over from before must not turn a crash into a success.
    let cfg = Config::default();
    let usage = fresh_usage(10.0);
    let job = base_job(JobState::Running);
    let live = HashSet::new(); // "test" is not live
    let activity = HashMap::new();
    let idle_since = HashMap::from([("job-1".to_string(), 0i64)]);
    let inputs = EngineInputs {
        now_ms: 100_000_000,
        usage: Some(&usage),
        last_known_pct: None,
        live: &live,
        activity: &activity,
        completed: &NO_COMPLETIONS,
        idle_since: &idle_since,
        cfg: &cfg,
    };
    let actions = plan(&[job], &inputs);
    assert!(
        matches!(actions.as_slice(), [Action::MarkStopped { .. }]),
        "expected MarkStopped, got {actions:?}"
    );
}
