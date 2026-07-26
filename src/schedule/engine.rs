//! Pure decision engine for the watch daemon. `plan` looks at the current
//! jobs, the latest usage sample, and live tmux state, and returns the list
//! of actions to take. It performs zero I/O so it can be exhaustively unit
//! tested without tmux, claude-usage, or the filesystem.

use super::*;
use crate::config::Config;
use crate::live::ActivityState;
use crate::usage::UsageSnapshot;
use std::collections::{HashMap, HashSet};

/// Everything `plan` needs to make its decisions, gathered by the caller
/// before invoking it.
pub struct EngineInputs<'a> {
    /// Current wall-clock time in epoch milliseconds.
    pub now_ms: i64,
    /// The latest usage sample, if one is available. `None` means no action
    /// is taken at all this tick.
    pub usage: Option<&'a UsageSnapshot>,
    /// Last known usage percentage, used to err toward pausing when `usage`
    /// is stale.
    pub last_known_pct: Option<f64>,
    /// tmux session names that currently exist.
    pub live: &'a HashSet<String>,
    /// Activity state of each live tmux session, keyed by tmux name.
    pub activity: &'a HashMap<String, ActivityState>,
    /// Ids of jobs whose session transcript reports the completion marker.
    pub completed: &'a HashSet<String>,
    /// Epoch ms at which each job's pane was first seen idle in its current
    /// idle stretch, keyed by job id. Absent means the job is not idle.
    pub idle_since: &'a HashMap<String, i64>,
    /// The active configuration.
    pub cfg: &'a Config,
}

/// One concrete action the daemon should take, as decided by `plan`. The
/// daemon executes these and persists any resulting job state changes;
/// `plan` itself has no side effects.
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    /// Start a brand-new tmux session for a queued job.
    Dispatch {
        /// Job to dispatch.
        job_id: String,
        /// Full argv to launch (element 0 is the claude binary).
        argv: Vec<String>,
        /// tmux session name to create.
        tmux_name: String,
    },
    /// Send an interrupt (Escape) to a running session to pause it softly.
    Interrupt {
        /// Job to interrupt.
        job_id: String,
    },
    /// Kill the tmux session outright to pause it hard.
    HardStop {
        /// Job to hard-stop.
        job_id: String,
    },
    /// Record that a job is now paused without sending any keys, e.g.
    /// because it was already blocked on a permission prompt.
    MarkPaused {
        /// Job being marked paused.
        job_id: String,
        /// Reason recorded in the job's history.
        reason: String,
    },
    /// Paste text into a paused session to make it continue.
    Resume {
        /// Job to resume.
        job_id: String,
        /// Text to send.
        text: String,
    },
    /// Relaunch a stopped job with `claude --resume`.
    Relaunch {
        /// Job to relaunch.
        job_id: String,
        /// Full argv to launch.
        argv: Vec<String>,
    },
    /// Record that a job's tmux session ended on its own.
    MarkStopped {
        /// Job being marked stopped.
        job_id: String,
        /// Reason recorded in the job's history.
        reason: String,
    },
    /// Give up on a job after exhausting restart attempts.
    Fail {
        /// Job being failed.
        job_id: String,
        /// Reason recorded in the job's history.
        reason: String,
    },
    /// Record that a job's work is finished. The daemon stops its tmux session
    /// and moves it to `Done`, after which nothing ever dispatches it again.
    MarkDone {
        /// Job being completed.
        job_id: String,
        /// Reason recorded in the job's history.
        reason: String,
    },
}

/// Compute the effective usage percentage to act on: the 5-hour window, and
/// (when `watch_seven_day` is enabled) the max of that and the 7-day window.
fn effective_pct(usage: &UsageSnapshot, watch_seven_day: bool) -> Option<f64> {
    let five = usage.five_hour.as_ref().and_then(|w| w.used_percentage);
    let seven = if watch_seven_day {
        usage.seven_day.as_ref().and_then(|w| w.used_percentage)
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

/// Decide what actions to take for the given jobs, given the current usage
/// sample, tmux state, and config. Pure: performs no I/O and has no side
/// effects. `Done` and `Failed` jobs are never acted on at all. `Blocked`,
/// `Starting`, `Pausing`, and `Resuming` jobs are driven by the daemon
/// through observation rather than planning, and are only touched here to
/// record a completion that has already been reported.
pub fn plan(jobs: &[Job], inputs: &EngineInputs) -> Vec<Action> {
    let mut actions = Vec::new();

    // No sample at all: take no action whatsoever this tick.
    let Some(usage) = inputs.usage else {
        return actions;
    };

    let fresh = usage.is_fresh(inputs.cfg.usage_max_age_seconds);
    let current_pct = effective_pct(usage, inputs.cfg.watch_seven_day);

    // Asymmetric staleness policy: a fresh sample is trusted on its own. A
    // stale sample still drives pause decisions, but errs toward the higher
    // of the current and last-known percentage; it is never used to resume.
    let pause_pct = if fresh {
        current_pct
    } else {
        match (current_pct, inputs.last_known_pct) {
            (Some(c), Some(l)) => Some(c.max(l)),
            (Some(c), None) => Some(c),
            (None, Some(l)) => Some(l),
            (None, None) => None,
        }
    };

    for job in jobs {
        // Completion outranks every other transition. A job that has reported
        // its work finished must not be dispatched, resumed, or relaunched
        // again, and checking it here rather than inside a single state arm is
        // what closes the loop: the marker usually lands while the job is
        // Running, but the session often ends on its own straight afterwards,
        // and a `Stopped` job with auto-resume on would otherwise be relaunched
        // forever against work that is already done.
        if !matches!(job.state, JobState::Done | JobState::Failed)
            && inputs.completed.contains(&job.id)
        {
            actions.push(Action::MarkDone {
                job_id: job.id.clone(),
                reason: format!("session reported {}", completion::COMPLETION_MARKER),
            });
            continue;
        }

        match job.state {
            JobState::Queued => {
                if let Some(pct) = pause_pct {
                    if pct < inputs.cfg.usage_pause_percent {
                        // The session name must be tmux-safe: a leading "." makes
                        // tmux read the target as a pane, and ":" separates the
                        // window component of a target.
                        let tmux_name = job
                            .tmux_name
                            .clone()
                            .unwrap_or_else(|| crate::live::sanitize_session_name(&job.name));
                        actions.push(Action::Dispatch {
                            job_id: job.id.clone(),
                            argv: build_start_argv(inputs.cfg.claude_bin(), job),
                            tmux_name,
                        });
                    }
                }
            }
            JobState::Running => {
                let tmux_name = job.tmux_name.as_deref().unwrap_or("");
                if !inputs.live.contains(tmux_name) {
                    actions.push(Action::MarkStopped {
                        job_id: job.id.clone(),
                        reason: "tmux session no longer exists".to_string(),
                    });
                    continue;
                }
                // Checked before the pause decision: pausing a job that has
                // already stopped working just to resume it later is churn.
                if let Some(reason) = idle_completion_reason(job, inputs) {
                    actions.push(Action::MarkDone {
                        job_id: job.id.clone(),
                        reason,
                    });
                    continue;
                }
                if let Some(pct) = pause_pct {
                    if pct >= inputs.cfg.usage_pause_percent {
                        let activity = inputs.activity.get(tmux_name).copied();
                        if activity == Some(ActivityState::Waiting) {
                            actions.push(Action::MarkPaused {
                                job_id: job.id.clone(),
                                reason: format!("usage at {pct:.1}%, already waiting on a prompt"),
                            });
                        } else {
                            match job.pause_mode {
                                PauseMode::Soft => actions.push(Action::Interrupt {
                                    job_id: job.id.clone(),
                                }),
                                PauseMode::Hard => actions.push(Action::HardStop {
                                    job_id: job.id.clone(),
                                }),
                            }
                        }
                    }
                }
            }
            JobState::Paused => {
                if !job.auto_resume || !fresh {
                    continue;
                }
                // A resume that failed to take reverts the job to Paused with
                // `attempts` incremented. The usage-threshold gate below would
                // re-fire on the very next tick (usage being low is usually why
                // the resume was attempted at all), so without an
                // attempts-gated delay a failing session retries in a tight
                // loop. Make the backoff apply regardless of usage.
                if job.attempts > 0
                    && inputs.now_ms - job.updated_at_ms
                        < backoff_ms(job.attempts.saturating_sub(1))
                {
                    continue;
                }
                let below_threshold = current_pct
                    .map(|pct| pct <= inputs.cfg.usage_resume_percent)
                    .unwrap_or(false);
                // `resume_after_ms` comes from an *estimated* reset time. If the
                // estimate runs early, honouring it blindly would resume into a
                // still-exhausted window and immediately re-pause, spending the
                // little quota that remains. So the deadline only releases a job
                // whose usage is not itself still at the pause threshold.
                let still_exhausted = current_pct
                    .map(|pct| pct >= inputs.cfg.usage_pause_percent)
                    .unwrap_or(false);
                let time_elapsed = job
                    .resume_after_ms
                    .map(|t| inputs.now_ms >= t)
                    .unwrap_or(false)
                    && !still_exhausted;
                if below_threshold || time_elapsed {
                    let text = continuation_text(job, inputs.cfg);
                    actions.push(Action::Resume {
                        job_id: job.id.clone(),
                        text,
                    });
                }
            }
            JobState::Stopped => {
                if !job.auto_resume {
                    continue;
                }
                if job.attempts >= inputs.cfg.max_restart_attempts {
                    actions.push(Action::Fail {
                        job_id: job.id.clone(),
                        reason: "exhausted max restart attempts".to_string(),
                    });
                } else if inputs.now_ms - job.updated_at_ms >= backoff_ms(job.attempts) {
                    // A job with neither a known claude session id nor a cwd we
                    // could resume in has nothing to relaunch into. Fail it
                    // rather than looping on `--continue` in the wrong place.
                    if job.claude_session_id.is_none() && job.cwd.is_empty() {
                        actions.push(Action::Fail {
                            job_id: job.id.clone(),
                            reason: "cannot relaunch: no claude session id and no cwd".to_string(),
                        });
                    } else {
                        actions.push(Action::Relaunch {
                            job_id: job.id.clone(),
                            argv: build_resume_argv(inputs.cfg.claude_bin(), job),
                        });
                    }
                }
            }
            JobState::Starting
            | JobState::Pausing
            | JobState::Resuming
            | JobState::Blocked
            | JobState::Done
            | JobState::Failed => {}
        }
    }

    actions
}

/// Reason to mark a running job done because its pane has been idle for
/// longer than `idle_complete_seconds`, or `None` when it has not.
///
/// This is the backstop for an agent that finishes its work but never emits
/// the completion marker (it forgot, or the job was adopted from a session
/// that was never told the protocol). Without it such a job sits `Running`
/// forever and every pause/resume cycle keeps poking at finished work.
/// A pane showing a permission prompt reads as `Waiting`, not `Idle`, so it is
/// never swept up by this.
fn idle_completion_reason(job: &Job, inputs: &EngineInputs) -> Option<String> {
    let limit_ms = (inputs.cfg.idle_complete_seconds as i64).saturating_mul(1000);
    if limit_ms <= 0 {
        return None;
    }
    let since = *inputs.idle_since.get(&job.id)?;
    let elapsed_ms = inputs.now_ms - since;
    if elapsed_ms < limit_ms {
        return None;
    }
    Some(format!(
        "idle for {} with no completion marker",
        format_minutes(elapsed_ms)
    ))
}

/// Render a duration in whole minutes (`"18m"`), or hours and minutes past an
/// hour (`"1h20m"`), for job history reasons.
fn format_minutes(ms: i64) -> String {
    let total = (ms / 60_000).max(0);
    let (hours, minutes) = (total / 60, total % 60);
    if hours > 0 {
        format!("{hours}h{minutes}m")
    } else {
        format!("{minutes}m")
    }
}

/// The text to paste into a session to continue it: the job's continue prompt
/// (or the config default when it has none) with the completion protocol
/// appended. Every prompt the daemon sends carries the protocol, so a job that
/// was resumed or relaunched still knows how to report that it is finished.
pub fn continuation_text(job: &Job, cfg: &Config) -> String {
    let base = job
        .continue_prompt
        .clone()
        .unwrap_or_else(|| cfg.continue_prompt.clone());
    completion::with_completion_protocol(&base)
}

/// Build the argv to start a brand-new claude session for `job`. Element 0
/// is the claude binary. A tmux probe confirmed `new-session` execvp's its
/// argv directly (no shell), so a prompt containing spaces, quotes,
/// `$(...)`, or backticks is safe as a single trailing argv element.
///
/// The completion protocol rides `--append-system-prompt`, so it reaches the
/// agent even when the job's prompt is a slash command.
pub fn build_start_argv(claude_bin: &str, job: &Job) -> Vec<String> {
    let mut argv = vec![
        claude_bin.to_string(),
        "--session-id".to_string(),
        job.id.clone(),
        "--name".to_string(),
        job.name.clone(),
    ];
    argv.extend(completion::system_prompt_args());
    if let Some(model) = &job.model {
        argv.push("--model".to_string());
        argv.push(model.clone());
    }
    if job.dangerous {
        argv.push("--dangerously-skip-permissions".to_string());
    }
    if !job.prompt.is_empty() {
        argv.push(completion::with_completion_protocol(&job.prompt));
    }
    argv
}

/// Build the argv to resume an existing claude session for `job`. Never
/// appends a positional prompt: `-r, --resume [value]` takes an optional
/// value, so a trailing prompt argument would be a parse ambiguity. The
/// continuation text is delivered separately via `Action::Resume`.
///
/// When the claude session id is unknown (an adopted session whose id we
/// never discovered), falls back to `--continue`, which resumes the most
/// recent conversation in the job's cwd. Passing `--resume ""` instead would
/// open claude's interactive session picker and hang the job forever.
pub fn build_resume_argv(claude_bin: &str, job: &Job) -> Vec<String> {
    let mut argv = match &job.claude_session_id {
        Some(id) if !id.is_empty() => {
            vec![claude_bin.to_string(), "--resume".to_string(), id.clone()]
        }
        _ => vec![claude_bin.to_string(), "--continue".to_string()],
    };
    argv.extend(completion::system_prompt_args());
    if let Some(model) = &job.model {
        argv.push("--model".to_string());
        argv.push(model.clone());
    }
    if job.dangerous {
        argv.push("--dangerously-skip-permissions".to_string());
    }
    argv
}

/// Delay before relaunching a stopped job, keyed by consecutive attempts:
/// 30s, 2m, 8m, 30m, then capped at 30m.
pub fn backoff_ms(attempts: u32) -> i64 {
    const STEPS_MS: [i64; 4] = [30_000, 120_000, 480_000, 1_800_000];
    let idx = (attempts as usize).min(STEPS_MS.len() - 1);
    STEPS_MS[idx]
}
