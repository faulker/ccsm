//! Headless `ccsm --watch` daemon. Runs detached in its own tmux session
//! (`live::WATCH_SESSION`), dispatching queued jobs into tmux, pausing them
//! when account usage crosses a threshold, and resuming or relaunching them
//! once usage falls or the reset window arrives. The decision logic itself
//! lives in the pure `schedule::engine::plan`; this module is the effectful
//! shell that gathers inputs, calls it, and carries out the resulting
//! actions against real tmux sessions and the `claude-usage` binary.

use crate::config::{Config, PauseMode};
use crate::live::{self, ActivityState};
use crate::schedule::command::{self, Command, JobPatch};
use crate::schedule::engine::{self, Action};
use crate::schedule::store;
use crate::schedule::{self, Job, JobState, Schedule};
use crate::usage::{self, UsageSnapshot};
use anyhow::{Context, Result};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::time::{Duration, Instant};

/// Heartbeat freshness window used by `is_running`/`heartbeat_is_fresh`:
/// roughly 3 heartbeat-write intervals (the daemon writes its heartbeat file
/// every 5s).
const HEARTBEAT_FRESH_MS: i64 = 15_000;

/// How long to wait for a freshly dispatched or relaunched pane's output to
/// settle before checking it for a trust prompt.
const DISPATCH_SETTLE_TIMEOUT: Duration = Duration::from_secs(10);

/// How long to keep retrying `schedule::discover_session_id` for an adopted
/// job before giving up.
const SESSION_DISCOVER_TIMEOUT_MS: i64 = 5 * 60 * 1000;

/// Minimum spacing between successive escalation steps while a job is stuck
/// in `Pausing`.
const ESCALATION_STEP_SPACING_MS: i64 = 10_000;

/// How long to wait after the initial Interrupt before starting escalation.
const ESCALATION_START_MS: i64 = 20_000;

/// How long to wait for `detect_activity` to report `Active` after a resume
/// prompt before giving up and reverting to `Paused` for another attempt.
const RESUME_CONFIRM_TIMEOUT_MS: i64 = 30_000;

/// Max size in bytes before `watch.log` is rotated to `watch.log.1`.
const LOG_ROTATE_BYTES: u64 = 1024 * 1024;

// --- Public API ------------------------------------------------------------

/// True if the watch daemon's tmux session exists and its heartbeat file is
/// fresh (see `heartbeat_is_fresh`). A live session with a stale heartbeat is
/// treated as not running, since the process behind it likely died without
/// cleaning up its tmux session.
pub fn is_running(tmux: &str) -> bool {
    if !live::session_exists(tmux, live::WATCH_SESSION) {
        return false;
    }
    match store::load_watch_state() {
        Some(state) => heartbeat_is_fresh(now_ms(), state.heartbeat_ms),
        None => false,
    }
}

/// Start the watch daemon if it is not already running. Returns `true` if a
/// new daemon was started, `false` if one was already running.
pub fn ensure_running(tmux: &str) -> Result<bool> {
    if is_running(tmux) {
        return Ok(false);
    }
    start(tmux)?;
    Ok(true)
}

/// Launch the watch daemon detached into its own tmux session
/// (`live::WATCH_SESSION`), reusing the existing `--spawn` precedent: the
/// daemon survives the TUI process and dies only with the ccsm tmux server.
pub fn start(tmux: &str) -> Result<()> {
    let exe = std::env::current_exe().context("Failed to determine current executable path")?;
    let exe_str = exe.to_string_lossy().to_string();
    let home = dirs::home_dir()
        .map(|h| h.to_string_lossy().to_string())
        .unwrap_or_else(|| ".".to_string());
    live::start_live_session(tmux, live::WATCH_SESSION, &home, &[&exe_str, "--watch"])
}

/// Ask the daemon to shut down via the command queue, waiting up to 3s for
/// its tmux session to disappear, then killing it directly as a backstop.
pub fn stop(tmux: &str) -> Result<()> {
    command::enqueue(&Command::StopWatcher)?;
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(3) {
        if !live::session_exists(tmux, live::WATCH_SESSION) {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    if live::session_exists(tmux, live::WATCH_SESSION) {
        live::stop_live_session(tmux, live::WATCH_SESSION)?;
    }
    Ok(())
}

/// Human-readable status summary for `--watch-status`: daemon health from
/// `watch_state.json` plus a one-line summary of every job in the schedule.
pub fn status_report() -> String {
    let cfg = Config::load();
    let tmux = cfg.tmux_bin().to_string();
    let running = is_running(&tmux);
    let mut lines = Vec::new();

    match store::load_watch_state() {
        Some(state) => {
            let now = now_ms();
            let heartbeat_age_s = (now - state.heartbeat_ms).max(0) / 1000;
            lines.push(format!(
                "watcher: {} (pid {}, heartbeat {}s ago)",
                if running { "running" } else { "not running" },
                state.pid,
                heartbeat_age_s
            ));
            if let Some(pct) = state.last_usage_pct {
                let sampled_ms = state.last_usage_at_ms.unwrap_or(state.heartbeat_ms);
                let sampled_age_s = (now - sampled_ms).max(0) / 1000;
                lines.push(format!("usage: {pct:.1}% (sampled {sampled_age_s}s ago)"));
            }
            if let Some(err) = &state.usage_error {
                lines.push(format!("usage error: {err}"));
            }
        }
        None => {
            lines.push(format!(
                "watcher: {}",
                if running { "running" } else { "not running" }
            ));
        }
    }

    let schedule = store::load();
    if schedule.jobs.is_empty() {
        lines.push("jobs: none".to_string());
    } else {
        lines.push(format!("jobs ({}):", schedule.jobs.len()));
        for job in &schedule.jobs {
            let short_id = &job.id[..job.id.len().min(8)];
            let error_suffix = job
                .last_error
                .as_ref()
                .map(|e| format!(" - {e}"))
                .unwrap_or_default();
            lines.push(format!(
                "  {short_id} {} [{:?}]{error_suffix}",
                job.name, job.state
            ));
        }
    }

    lines.join("\n")
}

/// Run the watch daemon loop. Blocks until a `StopWatcher` command is
/// received. Idempotent: if a fresh daemon is already running under a
/// different pid, logs and returns immediately rather than fighting it for
/// control of the schedule.
pub fn run() -> Result<()> {
    let boot_cfg = Config::load();
    let tmux = boot_cfg.tmux_bin().to_string();

    if live::session_exists(&tmux, live::WATCH_SESSION) {
        if let Some(state) = store::load_watch_state() {
            let now = now_ms();
            if heartbeat_is_fresh(now, state.heartbeat_ms) && state.pid != std::process::id() {
                log(&format!("watcher already running (pid {})", state.pid));
                return Ok(());
            }
            if !heartbeat_is_fresh(now, state.heartbeat_ms) {
                log(&format!("taking over from stale watcher (pid {})", state.pid));
            }
        }
    }

    rotate_log_if_needed();
    log("watcher starting");

    let (mut schedule, warning) = store::load_or_quarantine();
    if let Some(w) = warning {
        log(&w);
    }

    let started_at_ms = now_ms();
    let mut state = store::WatchState {
        pid: std::process::id(),
        started_at_ms,
        heartbeat_ms: started_at_ms,
        last_usage_pct: None,
        last_usage_at_ms: None,
        reset_at_ms: None,
        usage_error: None,
    };
    let _ = store::save_watch_state(&state);

    let mut transients = Transients::default();
    let mut stop_flag = false;
    let mut last_reconcile = Instant::now();
    let mut last_activity_poll = Instant::now();
    let mut last_heartbeat_write = Instant::now();
    let mut last_log_rotate = Instant::now();
    let mut last_usage_fetch: Option<Instant> = None;
    let mut last_usage_fetch_ms: i64 = 0;
    let mut last_known_pct: Option<f64> = None;
    let mut dirty = true;

    // The most recent successful usage sample, retained ACROSS ticks. Usage is
    // fetched at most every `usage_poll_seconds`, but the loop runs every
    // second, and `engine::plan` takes no action at all when handed `None`.
    // Scoping this inside the loop would therefore idle the whole engine on
    // every non-fetch tick, and would also lose the reset time that the
    // Pausing to Paused confirmation records. The sample is aged before use
    // (see `aged_snapshot` below) so holding it cannot masquerade as fresh.
    let mut usage_snapshot: Option<UsageSnapshot> = None;

    // Prime reconcile and activity state once up front so the first plan()
    // call this tick has fresh inputs rather than empty defaults.
    reconcile(&mut schedule, &tmux, now_ms(), &mut transients);
    let mut activity = poll_activity(&tmux, &schedule);
    let mut completed = poll_completion(&schedule);
    update_idle_tracking(&mut transients, &schedule, &activity, now_ms());

    loop {
        let tick_start = Instant::now();
        let cfg = Config::load();

        // Read without deleting: the files are acked only after the resulting
        // schedule has been persisted, so a crash mid-tick replays a command
        // rather than losing it. Every command is keyed by a caller-generated
        // id, which makes a replay idempotent.
        let (pending, warnings) = command::read_pending();
        for w in &warnings {
            log(w);
        }
        let mut force_usage_fetch = false;
        let mut applied_paths: Vec<std::path::PathBuf> = Vec::new();
        for (path, cmd) in pending {
            if matches!(cmd, Command::RefreshUsage) {
                force_usage_fetch = true;
            }
            apply_command(&tmux, &mut schedule, &cfg, &mut transients, cmd, &mut stop_flag, now_ms());
            applied_paths.push(path);
            dirty = true;
        }

        if stop_flag {
            // Persist before acking so a StopWatcher tick cannot drop the
            // commands that arrived alongside it.
            if let Err(e) = store::save(&schedule) {
                log(&format!("failed to save schedule on shutdown: {e:#}"));
            } else {
                command::ack(&applied_paths);
            }
            break;
        }

        if last_reconcile.elapsed() >= Duration::from_secs(5) {
            reconcile(&mut schedule, &tmux, now_ms(), &mut transients);
            last_reconcile = Instant::now();
            dirty = true;
        }

        if last_activity_poll.elapsed() >= Duration::from_secs(5) {
            activity = poll_activity(&tmux, &schedule);
            // Completion shares the activity cadence: both read per-job state
            // that only changes on the scale of seconds, and reading every
            // transcript tail once a second would be pure waste.
            completed = poll_completion(&schedule);
            update_idle_tracking(&mut transients, &schedule, &activity, now_ms());
            last_activity_poll = Instant::now();
        }

        let now = now_ms();
        let interval_ms = adaptive_interval_ms(&schedule.jobs, &cfg, now);
        let cadence_due = match last_usage_fetch {
            None => true,
            Some(t) => t.elapsed() >= Duration::from_millis(interval_ms.max(0) as u64),
        };
        let sleep_guard_due = should_force_fetch(now, last_usage_fetch_ms, interval_ms);
        let should_fetch = force_usage_fetch || cadence_due || sleep_guard_due;

        if should_fetch {
            match usage::fetch(cfg.usage_bin(), &cfg.usage_source, cfg.usage_max_age_seconds) {
                Ok(snap) => {
                    if let Some(pct) = snap.five_hour.as_ref().and_then(|w| w.used_percentage) {
                        last_known_pct = Some(pct);
                    }
                    state.last_usage_pct = snap.five_hour.as_ref().and_then(|w| w.used_percentage);
                    state.last_usage_at_ms = Some(snap.sampled_at_ms.unwrap_or(now));
                    state.reset_at_ms = snap.five_hour.as_ref().and_then(|w| w.reset_at_ms());
                    state.usage_error = None;
                    usage_snapshot = Some(snap);
                }
                Err(e) => {
                    let msg = format!("{e:#}");
                    state.usage_error = Some(msg.clone());
                    log(&format!("usage fetch failed: {msg}"));
                }
            }
            last_usage_fetch = Some(Instant::now());
            last_usage_fetch_ms = now;
        }

        // A retained sample keeps getting older. `age_seconds` is fixed at the
        // moment claude-usage reported it, so add the time we have held it;
        // otherwise a sample from ten minutes ago still claims to be fresh and
        // the "never resume on stale data" rule would be silently defeated.
        let aged_snapshot = aged_usage(usage_snapshot.as_ref(), last_usage_fetch_ms, now);

        let live_sessions = live::discover_live_sessions(&tmux);
        let live_set: HashSet<String> = live_sessions.iter().map(|s| s.tmux_name.clone()).collect();
        let inputs = engine::EngineInputs {
            now_ms: now,
            usage: aged_snapshot.as_ref(),
            last_known_pct,
            live: &live_set,
            activity: &activity,
            completed: &completed,
            idle_since: &transients.idle_since_ms,
            cfg: &cfg,
        };
        let actions = engine::plan(&schedule.jobs, &inputs);
        for action in actions {
            execute_action(
                &tmux,
                &mut schedule,
                action,
                &cfg,
                now_ms(),
                &mut transients,
                aged_snapshot.as_ref(),
            );
            dirty = true;
        }

        // Observation follow-ups: these drive Starting/Pausing/Resuming
        // toward their terminal states based on what the pane actually
        // shows, rather than through the pure planner.
        observe_still_starting(&tmux, &mut schedule, now_ms());
        observe_pausing(&tmux, &mut schedule, &cfg, aged_snapshot.as_ref(), &mut transients, now_ms());
        observe_resuming(&tmux, &mut schedule, &mut transients, now_ms());

        if dirty {
            match store::save(&schedule) {
                Ok(()) => {
                    // Only now is it safe to drop the command files.
                    command::ack(&applied_paths);
                    applied_paths.clear();
                    dirty = false;
                }
                Err(e) => {
                    // Leave the commands on disk so the next tick replays them.
                    log(&format!("failed to save schedule: {e:#}"));
                }
            }
        } else {
            command::ack(&applied_paths);
            applied_paths.clear();
        }

        if last_heartbeat_write.elapsed() >= Duration::from_secs(5) {
            state.heartbeat_ms = now_ms();
            if let Err(e) = store::save_watch_state(&state) {
                log(&format!("failed to save watch state: {e:#}"));
            }
            last_heartbeat_write = Instant::now();
        }

        if last_log_rotate.elapsed() >= Duration::from_secs(3600) {
            rotate_log_if_needed();
            last_log_rotate = Instant::now();
        }

        let elapsed = tick_start.elapsed();
        if elapsed < Duration::from_secs(1) {
            std::thread::sleep(Duration::from_secs(1) - elapsed);
        }
    }

    state.heartbeat_ms = now_ms();
    let _ = store::save_watch_state(&state);
    let _ = store::save(&schedule);
    log("watcher stopped");
    Ok(())
}

// --- Cross-tick state --------------------------------------------------

/// Per-job timing state that spans multiple ticks, for the escalation
/// ladder, resume-confirmation timeout, and session-id discovery retries.
/// Keyed by job id. Cleared whenever a job leaves the state the tracking
/// applies to.
#[derive(Default)]
struct Transients {
    /// Epoch ms when a job entered `Pausing`.
    pausing_since_ms: HashMap<String, i64>,
    /// Escalation steps already taken for a pausing job: 0 = none beyond the
    /// initial Interrupt, 1 = second Escape sent, 2 = Ctrl+C sent.
    pausing_escalation: HashMap<String, u8>,
    /// Epoch ms of the last escalation step, so steps stay spaced apart.
    pausing_last_step_ms: HashMap<String, i64>,
    /// Whether the previous tick's pane read for a pausing job was idle, so
    /// two consecutive idle reads can confirm a pause.
    last_idle_pausing: HashMap<String, bool>,
    /// Epoch ms when a job entered `Resuming`.
    resuming_since_ms: HashMap<String, i64>,
    /// Epoch ms when session-id discovery retries began for an adopted job.
    session_discover_since_ms: HashMap<String, i64>,
    /// Epoch ms when a running job's pane was first seen idle in its current
    /// idle stretch. Removed the moment it looks busy again, so the value is
    /// always the start of an unbroken run of idleness.
    idle_since_ms: HashMap<String, i64>,
}

impl Transients {
    /// Clear all tracked state for a job, e.g. once it reaches a terminal
    /// state or is deleted.
    fn clear_job(&mut self, job_id: &str) {
        self.clear_pausing(job_id);
        self.resuming_since_ms.remove(job_id);
        self.session_discover_since_ms.remove(job_id);
        self.idle_since_ms.remove(job_id);
    }

    /// Clear only the pausing-escalation state for a job, e.g. once its pause
    /// has been confirmed.
    fn clear_pausing(&mut self, job_id: &str) {
        self.pausing_since_ms.remove(job_id);
        self.pausing_escalation.remove(job_id);
        self.pausing_last_step_ms.remove(job_id);
        self.last_idle_pausing.remove(job_id);
    }
}

// --- Command application -----------------------------------------------

/// Apply one command from the queue to the schedule. State-changing commands
/// (`PauseJob`, `ResumeJob`) are translated into the same `Action` variants
/// `execute_action` handles for planner-driven transitions, so the two paths
/// never diverge in behavior. `RefreshUsage` is a no-op here; the caller
/// checks for it directly to force an immediate usage fetch this tick.
fn apply_command(
    tmux: &str,
    schedule: &mut Schedule,
    cfg: &Config,
    transients: &mut Transients,
    cmd: Command,
    stop_flag: &mut bool,
    now_ms: i64,
) {
    match cmd {
        Command::CreateJob { job } => {
            log(&format!("job {} ({}) created via command", job.id, job.name));
            schedule.jobs.push(job);
        }
        Command::UpdateJob { id, patch } => {
            if let Some(job) = schedule.find_mut(&id) {
                apply_patch(job, patch);
                log(&format!("job {id} updated via command"));
            } else {
                log(&format!("update for unknown job {id} ignored"));
            }
        }
        Command::DeleteJob { id } => {
            let before = schedule.jobs.len();
            schedule.jobs.retain(|j| j.id != id);
            if schedule.jobs.len() != before {
                log(&format!("job {id} deleted via command"));
            }
            transients.clear_job(&id);
        }
        Command::PauseJob { id } => {
            let Some(job) = schedule.find(&id) else {
                return;
            };
            if job.state != JobState::Running {
                log(&format!(
                    "pause requested for job {id} not in Running state, ignoring"
                ));
                return;
            }
            let action = match job.pause_mode {
                PauseMode::Soft => Action::Interrupt { job_id: id },
                PauseMode::Hard => Action::HardStop { job_id: id },
            };
            execute_action(tmux, schedule, action, cfg, now_ms, transients, None);
        }
        Command::ResumeJob { id } => {
            let Some(job) = schedule.find(&id) else {
                return;
            };
            if job.state != JobState::Paused {
                log(&format!(
                    "resume requested for job {id} not in Paused state, ignoring"
                ));
                return;
            }
            let text = engine::continuation_text(job, cfg);
            execute_action(
                tmux,
                schedule,
                Action::Resume { job_id: id, text },
                cfg,
                now_ms,
                transients,
                None,
            );
        }
        Command::StopJob { id } => {
            if let Some(job) = schedule.find_mut(&id) {
                job.auto_resume = false;
            }
            let tmux_name = schedule.find(&id).and_then(|j| j.tmux_name.clone());
            if let Some(name) = tmux_name {
                let _ = live::stop_live_session(tmux, &name);
            }
            if let Some(job) = schedule.find_mut(&id) {
                let job_name = job.name.clone();
                let from = job.state;
                let reason = "stopped by user request".to_string();
                log(&format_transition(&job_name, &id, from, JobState::Stopped, &reason));
                job.transition(JobState::Stopped, reason, now_ms);
            }
            transients.clear_job(&id);
        }
        Command::MarkDone { id } => {
            if schedule.find(&id).is_none() {
                log(&format!("mark-done for unknown job {id} ignored"));
                return;
            }
            // Same path as a self-reported completion, so a job the user
            // finishes by hand also gets its session stopped.
            execute_action(
                tmux,
                schedule,
                Action::MarkDone {
                    job_id: id,
                    reason: "marked done by user".to_string(),
                },
                cfg,
                now_ms,
                transients,
                None,
            );
        }
        Command::AdoptLive { id, tmux_name } => {
            if let Some(job) = schedule.find_mut(&id) {
                job.tmux_name = Some(tmux_name.clone());
                let job_name = job.name.clone();
                let from = job.state;
                let reason = format!("adopted live session '{tmux_name}'");
                log(&format_transition(&job_name, &id, from, JobState::Running, &reason));
                job.transition(JobState::Running, reason, now_ms);
                if let Err(e) = live::set_job_tag(tmux, &tmux_name, &id) {
                    log(&format!("failed to tag adopted session '{tmux_name}': {e:#}"));
                }
            }
        }
        Command::StopWatcher => {
            log("stop requested via command queue");
            *stop_flag = true;
        }
        Command::RefreshUsage => {}
    }
}

/// Apply a partial `JobPatch` to a job's editable fields. Fields left `None`
/// are left unchanged.
fn apply_patch(job: &mut Job, patch: JobPatch) {
    if let Some(name) = patch.name {
        job.name = name;
    }
    if let Some(cwd) = patch.cwd {
        job.cwd = schedule::canonical_cwd(&cwd);
    }
    if let Some(prompt) = patch.prompt {
        job.prompt = prompt;
    }
    if let Some(continue_prompt) = patch.continue_prompt {
        job.continue_prompt = continue_prompt;
    }
    if let Some(model) = patch.model {
        job.model = model;
    }
    if let Some(pause_mode) = patch.pause_mode {
        job.pause_mode = pause_mode;
    }
    if let Some(dangerous) = patch.dangerous {
        job.dangerous = dangerous;
    }
    if let Some(auto_resume) = patch.auto_resume {
        job.auto_resume = auto_resume;
    }
}

// --- Reconciliation against live tmux state -----------------------------

/// Reconcile the schedule against actual tmux state: mark jobs whose tmux
/// session vanished as `Stopped`, rebind jobs the user renamed with `r`, log
/// orphaned managed sessions without touching them, and retry Claude session
/// id discovery for adopted jobs.
fn reconcile(schedule: &mut Schedule, tmux: &str, now_ms: i64, transients: &mut Transients) {
    let live_sessions = live::discover_live_sessions(tmux);
    let live_names: HashSet<String> = live_sessions.iter().map(|s| s.tmux_name.clone()).collect();

    for job in schedule.jobs.iter_mut() {
        if matches!(
            job.state,
            JobState::Starting | JobState::Running | JobState::Pausing | JobState::Resuming
        ) {
            if let Some(name) = job.tmux_name.clone() {
                if !live_names.contains(&name) {
                    let from = job.state;
                    let reason = "tmux session disappeared".to_string();
                    log(&format_transition(&job.name, &job.id, from, JobState::Stopped, &reason));
                    job.transition(JobState::Stopped, reason, now_ms);
                    job.attempts += 1;
                    transients.clear_job(&job.id);
                }
            }
        }
    }

    let known_ids: HashSet<String> = schedule.jobs.iter().map(|j| j.id.clone()).collect();
    for live_session in &live_sessions {
        let Some(job_id) = &live_session.job_id else {
            continue;
        };
        if !known_ids.contains(job_id) {
            log(&format!("orphan managed session {}", live_session.tmux_name));
            continue;
        }
        if let Some(job) = schedule.find_mut(job_id) {
            if job.tmux_name.as_deref() != Some(live_session.tmux_name.as_str()) {
                log(&format!(
                    "job {} renamed: {:?} -> {}",
                    job.id, job.tmux_name, live_session.tmux_name
                ));
                job.tmux_name = Some(live_session.tmux_name.clone());
            }
        }
    }

    for job in schedule.jobs.iter_mut() {
        if job.state == JobState::Running && job.claude_session_id.is_none() {
            let since = *transients
                .session_discover_since_ms
                .entry(job.id.clone())
                .or_insert(now_ms);
            if now_ms - since > SESSION_DISCOVER_TIMEOUT_MS {
                log(&format!(
                    "giving up discovering claude session id for job {} ({})",
                    job.id, job.name
                ));
                transients.session_discover_since_ms.remove(&job.id);
                continue;
            }
            if let Some(sid) = schedule::discover_session_id(&job.cwd, since) {
                log(&format!("discovered claude session id for job {}: {}", job.id, sid));
                job.claude_session_id = Some(sid);
                transients.session_discover_since_ms.remove(&job.id);
            }
        } else {
            transients.session_discover_since_ms.remove(&job.id);
        }
    }
}

/// Poll `live::poll_pane_tail` (50 lines, matching `app/activity.rs`) for
/// every job with a known tmux name, building the activity map `plan` needs.
fn poll_activity(tmux: &str, schedule: &Schedule) -> HashMap<String, ActivityState> {
    let mut map = HashMap::new();
    for job in &schedule.jobs {
        if let Some(name) = &job.tmux_name {
            let pane = live::poll_pane_tail(tmux, name, 50);
            map.insert(name.clone(), live::detect_activity(&pane));
        }
    }
    map
}

/// Record how long each running job has looked idle, so `engine::plan` can
/// apply the idle-completion fallback without doing any I/O of its own. A job
/// that is not `Running`, or whose pane looks busy or is waiting on a prompt,
/// has its entry cleared: the timer measures one unbroken idle stretch, never
/// a total across interruptions.
fn update_idle_tracking(
    transients: &mut Transients,
    schedule: &Schedule,
    activity: &HashMap<String, ActivityState>,
    now_ms: i64,
) {
    for job in &schedule.jobs {
        let idle = job.state == JobState::Running
            && job
                .tmux_name
                .as_ref()
                .and_then(|name| activity.get(name))
                .copied()
                == Some(ActivityState::Idle);
        if idle {
            transients.idle_since_ms.entry(job.id.clone()).or_insert(now_ms);
        } else {
            transients.idle_since_ms.remove(&job.id);
        }
    }
}

/// Ids of jobs whose session transcript reports the completion marker.
///
/// `Queued` jobs are skipped because they have no session yet, and `Done`/
/// `Failed` jobs because nothing would act on the answer; everything else is
/// checked, including `Stopped`, since a job that finishes its work and then
/// exits is exactly the case that used to relaunch forever.
fn poll_completion(schedule: &Schedule) -> HashSet<String> {
    schedule
        .jobs
        .iter()
        .filter(|job| {
            !matches!(
                job.state,
                JobState::Queued | JobState::Done | JobState::Failed
            )
        })
        .filter(|job| schedule::completion::job_completed(job))
        .map(|job| job.id.clone())
        .collect()
}

// --- Action execution ----------------------------------------------------

/// Carry out one action returned by `engine::plan` (or synthesized directly
/// from a `PauseJob`/`ResumeJob` command). This is the only place in the
/// daemon that sends tmux input or starts/stops sessions.
fn execute_action(
    tmux: &str,
    schedule: &mut Schedule,
    action: Action,
    cfg: &Config,
    now_ms: i64,
    transients: &mut Transients,
    usage_snapshot: Option<&UsageSnapshot>,
) {
    match action {
        Action::Dispatch { job_id, argv, tmux_name } => {
            let Some(job) = schedule.find_mut(&job_id) else {
                return;
            };
            if !Path::new(&job.cwd).exists() {
                let job_name = job.name.clone();
                let from = job.state;
                let reason = "working directory no longer exists".to_string();
                log(&format_transition(&job_name, &job_id, from, JobState::Failed, &reason));
                job.transition(JobState::Failed, reason, now_ms);
                return;
            }
            let job_name = job.name.clone();
            let cwd = job.cwd.clone();
            let from = job.state;
            let argv_refs: Vec<&str> = argv.iter().map(|s| s.as_str()).collect();
            match live::start_managed_session(tmux, &tmux_name, &cwd, &job_id, &argv_refs) {
                Ok(()) => {
                    if let Some(job) = schedule.find_mut(&job_id) {
                        job.tmux_name = Some(tmux_name.clone());
                        let reason = "dispatched".to_string();
                        log(&format_transition(&job_name, &job_id, from, JobState::Starting, &reason));
                        job.transition(JobState::Starting, reason, now_ms);
                    }
                    observe_new_session(tmux, schedule, &job_id, now_ms);
                }
                Err(e) => {
                    if let Some(job) = schedule.find_mut(&job_id) {
                        let reason = format!("dispatch failed: {e:#}");
                        log(&format_transition(&job_name, &job_id, from, JobState::Failed, &reason));
                        job.transition(JobState::Failed, reason, now_ms);
                    }
                }
            }
        }
        Action::Interrupt { job_id } => {
            let Some(tmux_name) = schedule.find(&job_id).and_then(|j| j.tmux_name.clone()) else {
                return;
            };
            match live::interrupt_session(tmux, &tmux_name) {
                Ok(()) => {
                    if let Some(job) = schedule.find_mut(&job_id) {
                        let job_name = job.name.clone();
                        let from = job.state;
                        let reason = "usage threshold reached, sent interrupt".to_string();
                        log(&format_transition(&job_name, &job_id, from, JobState::Pausing, &reason));
                        job.transition(JobState::Pausing, reason, now_ms);
                    }
                    transients.pausing_since_ms.insert(job_id.clone(), now_ms);
                    transients.pausing_escalation.insert(job_id.clone(), 0);
                    transients.pausing_last_step_ms.insert(job_id.clone(), now_ms);
                    transients.last_idle_pausing.insert(job_id, false);
                }
                Err(e) => log(&format!("failed to interrupt job {job_id}: {e:#}")),
            }
        }
        Action::HardStop { job_id } => {
            let Some(tmux_name) = schedule.find(&job_id).and_then(|j| j.tmux_name.clone()) else {
                return;
            };
            match live::stop_live_session(tmux, &tmux_name) {
                Ok(()) => {
                    let resume_after_ms = resume_after_from_usage(usage_snapshot, cfg.watch_seven_day);
                    if let Some(job) = schedule.find_mut(&job_id) {
                        let job_name = job.name.clone();
                        let from = job.state;
                        job.paused_at_ms = Some(now_ms);
                        job.resume_after_ms = resume_after_ms;
                        let reason = "usage threshold reached, hard-stopped".to_string();
                        log(&format_transition(&job_name, &job_id, from, JobState::Paused, &reason));
                        job.transition(JobState::Paused, reason, now_ms);
                    }
                    transients.clear_pausing(&job_id);
                }
                Err(e) => log(&format!("failed to hard-stop job {job_id}: {e:#}")),
            }
        }
        Action::MarkPaused { job_id, reason } => {
            let resume_after_ms = resume_after_from_usage(usage_snapshot, cfg.watch_seven_day);
            if let Some(job) = schedule.find_mut(&job_id) {
                let job_name = job.name.clone();
                let from = job.state;
                job.paused_at_ms = Some(now_ms);
                job.resume_after_ms = resume_after_ms;
                log(&format_transition(&job_name, &job_id, from, JobState::Paused, &reason));
                job.transition(JobState::Paused, reason, now_ms);
            }
            transients.clear_pausing(&job_id);
        }
        Action::Resume { job_id, text } => {
            let Some(tmux_name) = schedule.find(&job_id).and_then(|j| j.tmux_name.clone()) else {
                return;
            };
            if cfg.defer_while_attached && live::has_attached_client(tmux, &tmux_name) {
                log(&format!("deferred: user attached (job {job_id})"));
                return;
            }
            match live::send_prompt(tmux, &tmux_name, &text) {
                Ok(()) => {
                    if let Some(job) = schedule.find_mut(&job_id) {
                        let job_name = job.name.clone();
                        let from = job.state;
                        let reason = "resume prompt sent".to_string();
                        log(&format_transition(&job_name, &job_id, from, JobState::Resuming, &reason));
                        job.transition(JobState::Resuming, reason, now_ms);
                    }
                    transients.resuming_since_ms.insert(job_id, now_ms);
                }
                Err(e) => log(&format!("failed to send resume prompt to job {job_id}: {e:#}")),
            }
        }
        Action::Relaunch { job_id, argv } => {
            let Some(job) = schedule.find_mut(&job_id) else {
                return;
            };
            if !Path::new(&job.cwd).exists() {
                let job_name = job.name.clone();
                let from = job.state;
                let reason = "working directory no longer exists".to_string();
                log(&format_transition(&job_name, &job_id, from, JobState::Failed, &reason));
                job.transition(JobState::Failed, reason, now_ms);
                return;
            }
            let job_name = job.name.clone();
            let cwd = job.cwd.clone();
            let from = job.state;
            let live_sessions = live::discover_live_sessions(tmux);
            let tmux_name = live::generate_auto_name(&cwd, &live_sessions);
            let argv_refs: Vec<&str> = argv.iter().map(|s| s.as_str()).collect();
            match live::start_managed_session(tmux, &tmux_name, &cwd, &job_id, &argv_refs) {
                Ok(()) => {
                    if let Some(job) = schedule.find_mut(&job_id) {
                        job.tmux_name = Some(tmux_name.clone());
                        let reason = "relaunched".to_string();
                        log(&format_transition(&job_name, &job_id, from, JobState::Starting, &reason));
                        job.transition(JobState::Starting, reason, now_ms);
                    }
                    observe_relaunch(tmux, schedule, &job_id, cfg, now_ms);
                }
                Err(e) => {
                    if let Some(job) = schedule.find_mut(&job_id) {
                        let reason = format!("relaunch failed: {e:#}");
                        log(&format_transition(&job_name, &job_id, from, JobState::Failed, &reason));
                        job.transition(JobState::Failed, reason, now_ms);
                    }
                }
            }
        }
        Action::MarkStopped { job_id, reason } => {
            if let Some(job) = schedule.find_mut(&job_id) {
                let job_name = job.name.clone();
                let from = job.state;
                log(&format_transition(&job_name, &job_id, from, JobState::Stopped, &reason));
                job.transition(JobState::Stopped, reason, now_ms);
            }
            transients.clear_job(&job_id);
        }
        Action::Fail { job_id, reason } => {
            if let Some(job) = schedule.find_mut(&job_id) {
                let job_name = job.name.clone();
                let from = job.state;
                log(&format_transition(&job_name, &job_id, from, JobState::Failed, &reason));
                job.transition(JobState::Failed, reason, now_ms);
            }
            transients.clear_job(&job_id);
        }
        Action::MarkDone { job_id, reason } => {
            // Stop the session before recording the state, so the tmux name a
            // Done job keeps is only ever a record of where it ran. Leaving it
            // alive would keep an idle claude sitting in the live list and keep
            // the daemon polling a pane that has nothing left to say.
            if let Some(name) = schedule.find(&job_id).and_then(|j| j.tmux_name.clone()) {
                if let Err(e) = live::stop_live_session(tmux, &name) {
                    log(&format!(
                        "job {job_id} completed but its session '{name}' could not be stopped: {e:#}"
                    ));
                }
            }
            if let Some(job) = schedule.find_mut(&job_id) {
                let job_name = job.name.clone();
                let from = job.state;
                job.resume_after_ms = None;
                log(&format_transition(&job_name, &job_id, from, JobState::Done, &reason));
                job.transition(JobState::Done, reason, now_ms);
            }
            transients.clear_job(&job_id);
        }
    }
}

/// Observe a job that was just dispatched: block until its pane settles (or
/// `DISPATCH_SETTLE_TIMEOUT` elapses), then check for the directory-trust
/// dialog. Blocking here is intentional; it happens synchronously right
/// after a successful `start_managed_session` call so the daemon never plans
/// against a job whose pane state is still unknown. If the pane never
/// settles in time, the job is left `Starting` and `observe_still_starting`
/// retries on later ticks without blocking.
fn observe_new_session(tmux: &str, schedule: &mut Schedule, job_id: &str, now_ms: i64) {
    let Some(tmux_name) = schedule.find(job_id).and_then(|j| j.tmux_name.clone()) else {
        return;
    };
    if !live::wait_pane_settled(tmux, &tmux_name, DISPATCH_SETTLE_TIMEOUT) {
        return;
    }
    let pane = live::poll_pane_tail(tmux, &tmux_name, 30);
    let blocked = live::detect_trust_prompt(&pane);
    let Some(job) = schedule.find_mut(job_id) else {
        return;
    };
    let job_name = job.name.clone();
    let from = job.state;
    if blocked {
        let reason = "claude is waiting for directory trust confirmation".to_string();
        log(&format_transition(&job_name, job_id, from, JobState::Blocked, &reason));
        job.transition(JobState::Blocked, reason, now_ms);
    } else {
        let reason = "pane settled".to_string();
        log(&format_transition(&job_name, job_id, from, JobState::Running, &reason));
        job.transition(JobState::Running, reason, now_ms);
    }
}

/// Observe a job that was just relaunched: same trust-prompt check as
/// `observe_new_session`, but on success delivers the continue prompt and
/// transitions to `Resuming` instead of `Running` directly, since the
/// daemon still needs to confirm the resumed session is actually active.
fn observe_relaunch(tmux: &str, schedule: &mut Schedule, job_id: &str, cfg: &Config, now_ms: i64) {
    let Some(tmux_name) = schedule.find(job_id).and_then(|j| j.tmux_name.clone()) else {
        return;
    };
    if !live::wait_pane_settled(tmux, &tmux_name, DISPATCH_SETTLE_TIMEOUT) {
        return;
    }
    let pane = live::poll_pane_tail(tmux, &tmux_name, 30);
    if live::detect_trust_prompt(&pane) {
        if let Some(job) = schedule.find_mut(job_id) {
            let job_name = job.name.clone();
            let from = job.state;
            let reason = "claude is waiting for directory trust confirmation".to_string();
            log(&format_transition(&job_name, job_id, from, JobState::Blocked, &reason));
            job.transition(JobState::Blocked, reason, now_ms);
        }
        return;
    }

    let text = match schedule.find(job_id) {
        Some(job) => engine::continuation_text(job, cfg),
        None => return,
    };

    if let Err(e) = live::send_prompt(tmux, &tmux_name, &text) {
        log(&format!("failed to send continue prompt for job {job_id}: {e:#}"));
        return;
    }

    if let Some(job) = schedule.find_mut(job_id) {
        let job_name = job.name.clone();
        let from = job.state;
        let reason = "continue prompt sent".to_string();
        log(&format_transition(&job_name, job_id, from, JobState::Resuming, &reason));
        job.transition(JobState::Resuming, reason, now_ms);
    }
}

/// Non-blocking retry for jobs still `Starting` whose pane never settled
/// within `observe_new_session`'s window. Runs every tick; cheap since it is
/// just a single pane capture per still-starting job.
fn observe_still_starting(tmux: &str, schedule: &mut Schedule, now_ms: i64) {
    let ids: Vec<String> = schedule
        .jobs
        .iter()
        .filter(|j| j.state == JobState::Starting)
        .map(|j| j.id.clone())
        .collect();
    for job_id in ids {
        let Some(tmux_name) = schedule.find(&job_id).and_then(|j| j.tmux_name.clone()) else {
            continue;
        };
        let pane = live::poll_pane_tail(tmux, &tmux_name, 30);
        if pane.trim().is_empty() {
            continue;
        }
        if live::detect_trust_prompt(&pane) {
            if let Some(job) = schedule.find_mut(&job_id) {
                let job_name = job.name.clone();
                let from = job.state;
                let reason = "claude is waiting for directory trust confirmation".to_string();
                log(&format_transition(&job_name, &job_id, from, JobState::Blocked, &reason));
                job.transition(JobState::Blocked, reason, now_ms);
            }
            continue;
        }
        if live::detect_activity(&pane) != ActivityState::Unknown {
            if let Some(job) = schedule.find_mut(&job_id) {
                let job_name = job.name.clone();
                let from = job.state;
                let reason = "pane active, startup complete".to_string();
                log(&format_transition(&job_name, &job_id, from, JobState::Running, &reason));
                job.transition(JobState::Running, reason, now_ms);
            }
        }
    }
}

/// Confirm (or escalate) a soft pause for every job in `Pausing`. Confirms
/// via `detect_interrupted` or two consecutive idle activity reads, then
/// records `resume_after_ms` and transitions to `Paused`. If still active
/// past the escalation ladder in `escalation_step`, sends a second Escape,
/// then Ctrl+C, then hard-stops the session outright.
fn observe_pausing(
    tmux: &str,
    schedule: &mut Schedule,
    cfg: &Config,
    usage_snapshot: Option<&UsageSnapshot>,
    transients: &mut Transients,
    now_ms: i64,
) {
    let ids: Vec<String> = schedule
        .jobs
        .iter()
        .filter(|j| j.state == JobState::Pausing)
        .map(|j| j.id.clone())
        .collect();

    for job_id in ids {
        let Some(tmux_name) = schedule.find(&job_id).and_then(|j| j.tmux_name.clone()) else {
            continue;
        };
        let pane = live::poll_pane_tail(tmux, &tmux_name, 8);
        let interrupted = live::detect_interrupted(&pane);
        let idle_now = live::detect_activity(&pane) == ActivityState::Idle;
        let idle_last = transients.last_idle_pausing.get(&job_id).copied().unwrap_or(false);
        transients.last_idle_pausing.insert(job_id.clone(), idle_now);

        if interrupted || (idle_now && idle_last) {
            let resume_after_ms = resume_after_from_usage(usage_snapshot, cfg.watch_seven_day);
            if let Some(job) = schedule.find_mut(&job_id) {
                let job_name = job.name.clone();
                let from = job.state;
                job.paused_at_ms = Some(now_ms);
                job.resume_after_ms = resume_after_ms;
                let reason = "confirmed paused".to_string();
                log(&format_transition(&job_name, &job_id, from, JobState::Paused, &reason));
                job.transition(JobState::Paused, reason, now_ms);
            }
            transients.clear_pausing(&job_id);
            continue;
        }

        let since = *transients.pausing_since_ms.entry(job_id.clone()).or_insert(now_ms);
        let elapsed = now_ms - since;
        let steps_taken = *transients.pausing_escalation.get(&job_id).unwrap_or(&0);
        let last_step_ms = *transients.pausing_last_step_ms.get(&job_id).unwrap_or(&since);
        if now_ms - last_step_ms < ESCALATION_STEP_SPACING_MS {
            continue;
        }

        match escalation_step(elapsed, steps_taken) {
            EscalationStep::Wait => {}
            EscalationStep::SecondEscape => {
                if live::interrupt_session(tmux, &tmux_name).is_ok() {
                    log(&format!("job {job_id}: escalating pause, sent second Escape"));
                    transients.pausing_escalation.insert(job_id.clone(), 1);
                    transients.pausing_last_step_ms.insert(job_id, now_ms);
                }
            }
            EscalationStep::CtrlC => {
                if live::send_ctrl_c(tmux, &tmux_name).is_ok() {
                    log(&format!("job {job_id}: escalating pause, sent Ctrl+C"));
                    transients.pausing_escalation.insert(job_id.clone(), 2);
                    transients.pausing_last_step_ms.insert(job_id, now_ms);
                }
            }
            EscalationStep::HardStop => {
                if live::stop_live_session(tmux, &tmux_name).is_ok() {
                    let resume_after_ms = resume_after_from_usage(usage_snapshot, cfg.watch_seven_day);
                    if let Some(job) = schedule.find_mut(&job_id) {
                        let job_name = job.name.clone();
                        let from = job.state;
                        job.paused_at_ms = Some(now_ms);
                        job.resume_after_ms = resume_after_ms;
                        let reason = "escalated to hard stop after unresponsive pause".to_string();
                        log(&format_transition(&job_name, &job_id, from, JobState::Paused, &reason));
                        job.transition(JobState::Paused, reason, now_ms);
                    }
                    transients.clear_pausing(&job_id);
                }
            }
        }
    }
}

/// Confirm a resume for every job in `Resuming`: once `detect_activity`
/// reports `Active`, transition to `Running`. If nothing after
/// `RESUME_CONFIRM_TIMEOUT_MS`, increment `attempts` and revert to `Paused`
/// so the next tick retries. Note: unlike the `Stopped` -> `Relaunch` path,
/// `engine::plan`'s `Paused` -> `Resume` transition has no backoff gate of
/// its own, so a job whose usage is already below the resume threshold will
/// be retried again immediately; only the resume-after-timer path benefits
/// from the backoff bump applied below.
fn observe_resuming(tmux: &str, schedule: &mut Schedule, transients: &mut Transients, now_ms: i64) {
    let ids: Vec<String> = schedule
        .jobs
        .iter()
        .filter(|j| j.state == JobState::Resuming)
        .map(|j| j.id.clone())
        .collect();

    for job_id in ids {
        let Some(tmux_name) = schedule.find(&job_id).and_then(|j| j.tmux_name.clone()) else {
            continue;
        };
        let pane = live::poll_pane_tail(tmux, &tmux_name, 8);
        if live::detect_activity(&pane) == ActivityState::Active {
            if let Some(job) = schedule.find_mut(&job_id) {
                let job_name = job.name.clone();
                let from = job.state;
                let reason = "resumed, activity confirmed".to_string();
                log(&format_transition(&job_name, &job_id, from, JobState::Running, &reason));
                job.transition(JobState::Running, reason, now_ms);
            }
            transients.resuming_since_ms.remove(&job_id);
            continue;
        }

        let since = *transients.resuming_since_ms.entry(job_id.clone()).or_insert(now_ms);
        if now_ms - since > RESUME_CONFIRM_TIMEOUT_MS {
            if let Some(job) = schedule.find_mut(&job_id) {
                job.attempts += 1;
                let job_name = job.name.clone();
                let from = job.state;
                let backoff = engine::backoff_ms(job.attempts);
                job.resume_after_ms = Some(now_ms + backoff);
                let reason = "no activity after resume, reverting to paused for retry".to_string();
                log(&format_transition(&job_name, &job_id, from, JobState::Paused, &reason));
                job.transition(JobState::Paused, reason, now_ms);
            }
            transients.resuming_since_ms.remove(&job_id);
        }
    }
}

// --- Pure helpers (unit tested below) -----------------------------------

/// Current wall-clock time in epoch milliseconds (UTC). Deadlines and
/// heartbeats are always compared in wall-clock time; only loop cadence uses
/// `Instant`. See the module doc for why the two must never be mixed.
fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

/// True if a heartbeat written at `heartbeat_ms` is still fresh at `now_ms`.
fn heartbeat_is_fresh(now_ms: i64, heartbeat_ms: i64) -> bool {
    now_ms - heartbeat_ms < HEARTBEAT_FRESH_MS
}

/// Compute the usage-poll interval in milliseconds for the current tick:
/// `usage_poll_seconds` while any job is actively `Starting`/`Running`/
/// `Resuming`; otherwise 5 minutes, tightening to 30s once within 2 minutes
/// of the earliest known reset among `Paused` jobs.
fn adaptive_interval_ms(jobs: &[Job], cfg: &Config, now_ms: i64) -> i64 {
    const IDLE_MS: i64 = 5 * 60 * 1000;
    const NEAR_RESET_MS: i64 = 30_000;
    const NEAR_RESET_WINDOW_MS: i64 = 2 * 60 * 1000;

    let any_active = jobs
        .iter()
        .any(|j| matches!(j.state, JobState::Running | JobState::Starting | JobState::Resuming));
    if any_active {
        return (cfg.usage_poll_seconds as i64).max(1) * 1000;
    }

    let earliest_reset = jobs
        .iter()
        .filter(|j| j.state == JobState::Paused)
        .filter_map(|j| j.resume_after_ms)
        .min();
    if let Some(reset_ms) = earliest_reset {
        if reset_ms - now_ms <= NEAR_RESET_WINDOW_MS {
            return NEAR_RESET_MS;
        }
    }

    IDLE_MS
}

/// The laptop-sleep guard: true if wall-clock time has advanced far enough
/// past `last_fetch_ms` that a fetch should happen even though the
/// `Instant`-based cadence timer hasn't fired. `Instant` does not advance
/// across a macOS sleep, so relying on it alone can sleep straight through a
/// usage reset.
fn should_force_fetch(now_ms: i64, last_fetch_ms: i64, interval_ms: i64) -> bool {
    now_ms - last_fetch_ms > 2 * interval_ms
}

/// Format a job state transition as a human-readable log line, e.g.
/// `job a1b2c3d4 refactor: Running -> Pausing (5h usage 96.2% >= 95.0%)`.
fn format_transition(job_name: &str, job_id: &str, from: JobState, to: JobState, reason: &str) -> String {
    let short_id = &job_id[..job_id.len().min(8)];
    format!("job {short_id} {job_name}: {from:?} -> {to:?} ({reason})")
}

/// The escalation ladder for a job stuck in `Pausing` after an initial soft
/// Interrupt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EscalationStep {
    /// Not yet time to escalate.
    Wait,
    /// Send a second Escape.
    SecondEscape,
    /// Send Ctrl+C.
    CtrlC,
    /// Give up softening and hard-stop the session.
    HardStop,
}

/// Decide the escalation step for a job that has been `Pausing` for
/// `elapsed_ms`, given the number of escalation steps already taken
/// (`steps_taken`: 0 = none, 1 = second Escape sent, 2 = Ctrl+C sent).
/// Steps are spaced at least `ESCALATION_STEP_SPACING_MS` apart, starting at
/// `ESCALATION_START_MS`.
fn escalation_step(elapsed_ms: i64, steps_taken: u8) -> EscalationStep {
    if elapsed_ms < ESCALATION_START_MS {
        return EscalationStep::Wait;
    }
    match steps_taken {
        0 => EscalationStep::SecondEscape,
        1 if elapsed_ms >= ESCALATION_START_MS + ESCALATION_STEP_SPACING_MS => EscalationStep::CtrlC,
        2 if elapsed_ms >= ESCALATION_START_MS + 2 * ESCALATION_STEP_SPACING_MS => EscalationStep::HardStop,
        _ => EscalationStep::Wait,
    }
}

/// Compute the epoch ms after which a paused job may be resumed, from the
/// usage window(s) that triggered the pause: the 5-hour window's reset, and
/// (when `watch_seven_day` is enabled) the later of that and the 7-day
/// window's reset, mirroring `engine::effective_pct`'s max-of-both policy.
/// Age a retained usage sample by however long it has been held, so that
/// `is_fresh` reflects the sample's true age rather than its age at the moment
/// claude-usage reported it. Without this, a sample kept across ticks would
/// claim indefinite freshness and defeat the "never resume on stale data" rule.
fn aged_usage(
    usage: Option<&UsageSnapshot>,
    fetched_at_ms: i64,
    now_ms: i64,
) -> Option<UsageSnapshot> {
    let usage = usage?;
    let held_seconds = ((now_ms - fetched_at_ms).max(0)) / 1000;
    let mut aged = usage.clone();
    aged.age_seconds = Some(usage.age_seconds.unwrap_or(0) + held_seconds);
    Some(aged)
}

/// Epoch ms at which a paused job may resume, derived from the reset time of
/// whichever usage window is being watched.
fn resume_after_from_usage(usage: Option<&UsageSnapshot>, watch_seven_day: bool) -> Option<i64> {
    let usage = usage?;
    let five = usage.five_hour.as_ref().and_then(|w| w.reset_at_ms());
    let seven = if watch_seven_day {
        usage.seven_day.as_ref().and_then(|w| w.reset_at_ms())
    } else {
        None
    };
    match (five, seven) {
        (Some(a), Some(b)) => Some(a.max(b)),
        (Some(a), None) => Some(a),
        (None, Some(b)) => Some(b),
        (None, None) => None,
    }
}

/// Path to the daemon's log file (`<ccsm_dir>/watch.log`).
fn log_path() -> Option<std::path::PathBuf> {
    Some(crate::config::ccsm_dir()?.join("watch.log"))
}

/// Rotate `watch.log` to `watch.log.1` (overwriting any previous rotation)
/// if it has grown past `LOG_ROTATE_BYTES`.
fn rotate_log_if_needed() {
    let Some(path) = log_path() else {
        return;
    };
    if let Ok(meta) = std::fs::metadata(&path) {
        if meta.len() > LOG_ROTATE_BYTES {
            let rotated = path.with_extension("log.1");
            let _ = std::fs::rename(&path, &rotated);
        }
    }
}

/// Append a line to `watch.log` and print the same line to stdout, so
/// `tmux -L ccsm attach -t ccsm-watch` doubles as a free live tail.
fn log(message: &str) {
    let ts = chrono::Local::now().to_rfc3339();
    let line = format!("{ts}  INFO  {message}");
    println!("{line}");
    if let Some(path) = log_path() {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
            use std::io::Write;
            let _ = writeln!(f, "{line}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::usage::UsageWindow;

    /// A minimal job with sensible defaults for tests, mirroring
    /// `schedule::tests::base_job`.
    fn base_job(id: &str, state: JobState) -> Job {
        Job {
            id: id.to_string(),
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

    #[test]
    fn adaptive_interval_no_jobs_is_five_minutes() {
        let cfg = Config::default();
        assert_eq!(adaptive_interval_ms(&[], &cfg, 0), 5 * 60 * 1000);
    }

    #[test]
    fn adaptive_interval_running_job_uses_configured_poll_seconds() {
        let mut cfg = Config::default();
        cfg.usage_poll_seconds = 45;
        let jobs = vec![base_job("a", JobState::Running)];
        assert_eq!(adaptive_interval_ms(&jobs, &cfg, 0), 45_000);
    }

    #[test]
    fn adaptive_interval_starting_and_resuming_also_count_as_active() {
        let cfg = Config::default();
        for state in [JobState::Starting, JobState::Resuming] {
            let jobs = vec![base_job("a", state)];
            assert_eq!(
                adaptive_interval_ms(&jobs, &cfg, 0),
                (cfg.usage_poll_seconds as i64) * 1000
            );
        }
    }

    #[test]
    fn adaptive_interval_all_paused_far_from_reset_is_five_minutes() {
        let cfg = Config::default();
        let mut job = base_job("a", JobState::Paused);
        job.resume_after_ms = Some(10 * 60 * 1000);
        assert_eq!(adaptive_interval_ms(&[job], &cfg, 0), 5 * 60 * 1000);
    }

    #[test]
    fn adaptive_interval_all_paused_near_reset_is_thirty_seconds() {
        let cfg = Config::default();
        let mut job = base_job("a", JobState::Paused);
        job.resume_after_ms = Some(90_000);
        assert_eq!(adaptive_interval_ms(&[job], &cfg, 0), 30_000);
    }

    #[test]
    fn adaptive_interval_paused_without_known_reset_is_five_minutes() {
        let cfg = Config::default();
        let job = base_job("a", JobState::Paused);
        assert_eq!(adaptive_interval_ms(&[job], &cfg, 0), 5 * 60 * 1000);
    }

    #[test]
    fn should_force_fetch_fires_past_double_interval() {
        assert!(should_force_fetch(1_000_000, 0, 100_000));
        assert!(!should_force_fetch(150_000, 0, 100_000));
    }

    #[test]
    fn should_force_fetch_boundary_is_exclusive() {
        assert!(!should_force_fetch(200_000, 0, 100_000));
        assert!(should_force_fetch(200_001, 0, 100_000));
    }

    #[test]
    fn format_transition_matches_expected_shape() {
        let line = format_transition(
            "refactor",
            "a1b2c3d4-e5f6",
            JobState::Running,
            JobState::Pausing,
            "5h usage 96.2% >= 95.0%",
        );
        assert_eq!(
            line,
            "job a1b2c3d4 refactor: Running -> Pausing (5h usage 96.2% >= 95.0%)"
        );
    }

    #[test]
    fn format_transition_handles_short_ids() {
        let line = format_transition("x", "abc", JobState::Queued, JobState::Starting, "go");
        assert_eq!(line, "job abc x: Queued -> Starting (go)");
    }

    #[test]
    fn escalation_step_waits_before_start_threshold() {
        assert_eq!(escalation_step(0, 0), EscalationStep::Wait);
        assert_eq!(escalation_step(19_999, 0), EscalationStep::Wait);
    }

    #[test]
    fn escalation_step_second_escape_at_twenty_seconds() {
        assert_eq!(escalation_step(20_000, 0), EscalationStep::SecondEscape);
    }

    #[test]
    fn escalation_step_ctrl_c_at_thirty_seconds_after_spacing() {
        assert_eq!(escalation_step(25_000, 1), EscalationStep::Wait);
        assert_eq!(escalation_step(30_000, 1), EscalationStep::CtrlC);
    }

    #[test]
    fn escalation_step_hard_stop_at_forty_seconds_after_spacing() {
        assert_eq!(escalation_step(35_000, 2), EscalationStep::Wait);
        assert_eq!(escalation_step(40_000, 2), EscalationStep::HardStop);
    }

    #[test]
    fn heartbeat_is_fresh_within_window() {
        assert!(heartbeat_is_fresh(10_000, 5_000));
    }

    #[test]
    fn heartbeat_is_fresh_stale_after_window() {
        assert!(!heartbeat_is_fresh(20_000, 0));
    }

    #[test]
    fn heartbeat_is_fresh_boundary_is_exclusive() {
        assert!(!heartbeat_is_fresh(HEARTBEAT_FRESH_MS, 0));
        assert!(heartbeat_is_fresh(HEARTBEAT_FRESH_MS - 1, 0));
    }

    #[test]
    fn aged_usage_adds_the_time_the_sample_was_held() {
        // Regression: the snapshot is retained across ticks, so its freshness
        // must decay. Holding it without ageing let a stale sample pass
        // `is_fresh` forever and defeated "never resume on stale data".
        let snap = UsageSnapshot {
            age_seconds: Some(60),
            ..Default::default()
        };
        // Held for 120s on top of the 60s it already was when sampled.
        let aged = aged_usage(Some(&snap), 1_000_000, 1_120_000).unwrap();
        assert_eq!(aged.age_seconds, Some(180));

        // Fresh against a generous limit, stale against a tight one.
        assert!(aged.is_fresh(900));
        assert!(!aged.is_fresh(120));
        // The original is untouched.
        assert_eq!(snap.age_seconds, Some(60));
    }

    #[test]
    fn aged_usage_handles_missing_age_and_backward_clocks() {
        let snap = UsageSnapshot {
            age_seconds: None,
            ..Default::default()
        };
        let aged = aged_usage(Some(&snap), 1_000_000, 1_030_000).unwrap();
        assert_eq!(aged.age_seconds, Some(30));

        // An NTP step backwards must not produce a negative age.
        let aged_back = aged_usage(Some(&snap), 1_000_000, 990_000).unwrap();
        assert_eq!(aged_back.age_seconds, Some(0));

        assert!(aged_usage(None, 0, 1000).is_none());
    }

    /// A schedule holding exactly the given jobs.
    fn schedule_of(jobs: Vec<Job>) -> Schedule {
        Schedule { version: 1, jobs }
    }

    #[test]
    fn idle_tracking_starts_the_clock_and_keeps_the_first_timestamp() {
        let mut transients = Transients::default();
        let schedule = schedule_of(vec![base_job("a", JobState::Running)]);
        let activity = HashMap::from([("test".to_string(), ActivityState::Idle)]);

        update_idle_tracking(&mut transients, &schedule, &activity, 1_000);
        assert_eq!(transients.idle_since_ms.get("a"), Some(&1_000));

        // Still idle later: the clock measures the whole stretch, so the
        // original timestamp must survive rather than being pushed forward.
        update_idle_tracking(&mut transients, &schedule, &activity, 9_000);
        assert_eq!(transients.idle_since_ms.get("a"), Some(&1_000));
    }

    #[test]
    fn idle_tracking_resets_when_the_job_looks_busy_again() {
        let mut transients = Transients::default();
        let schedule = schedule_of(vec![base_job("a", JobState::Running)]);

        let idle = HashMap::from([("test".to_string(), ActivityState::Idle)]);
        update_idle_tracking(&mut transients, &schedule, &idle, 1_000);
        assert!(transients.idle_since_ms.contains_key("a"));

        for state in [ActivityState::Active, ActivityState::Waiting, ActivityState::Unknown] {
            update_idle_tracking(&mut transients, &schedule, &idle, 2_000);
            let busy = HashMap::from([("test".to_string(), state)]);
            update_idle_tracking(&mut transients, &schedule, &busy, 3_000);
            assert!(
                !transients.idle_since_ms.contains_key("a"),
                "{state:?} should clear the idle clock"
            );
        }
    }

    #[test]
    fn idle_tracking_only_applies_to_running_jobs() {
        // A paused job sits idle for hours by design; only Running jobs are
        // candidates for idle completion.
        let activity = HashMap::from([("test".to_string(), ActivityState::Idle)]);
        for state in [
            JobState::Queued,
            JobState::Starting,
            JobState::Pausing,
            JobState::Paused,
            JobState::Resuming,
            JobState::Stopped,
            JobState::Blocked,
            JobState::Done,
            JobState::Failed,
        ] {
            let mut transients = Transients::default();
            let schedule = schedule_of(vec![base_job("a", state)]);
            update_idle_tracking(&mut transients, &schedule, &activity, 1_000);
            assert!(
                !transients.idle_since_ms.contains_key("a"),
                "{state:?} should not accrue idle time"
            );
        }
    }

    #[test]
    fn resume_after_from_usage_none_without_snapshot() {
        assert_eq!(resume_after_from_usage(None, true), None);
    }

    #[test]
    fn resume_after_from_usage_prefers_max_reset_when_seven_day_enabled() {
        let snap = UsageSnapshot {
            five_hour: Some(UsageWindow {
                used_percentage: None,
                resets_at: None,
                resets_at_estimated_ms: Some(100),
            }),
            seven_day: Some(UsageWindow {
                used_percentage: None,
                resets_at: None,
                resets_at_estimated_ms: Some(200),
            }),
            ..Default::default()
        };
        assert_eq!(resume_after_from_usage(Some(&snap), true), Some(200));
        assert_eq!(resume_after_from_usage(Some(&snap), false), Some(100));
    }

    #[test]
    fn resume_after_from_usage_falls_back_to_whichever_window_is_known() {
        let five_only = UsageSnapshot {
            five_hour: Some(UsageWindow {
                used_percentage: None,
                resets_at: None,
                resets_at_estimated_ms: Some(50),
            }),
            seven_day: None,
            ..Default::default()
        };
        assert_eq!(resume_after_from_usage(Some(&five_only), true), Some(50));

        let neither = UsageSnapshot::default();
        assert_eq!(resume_after_from_usage(Some(&neither), true), None);
    }
}
