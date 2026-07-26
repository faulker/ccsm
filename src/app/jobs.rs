use super::*;
use crate::schedule::command::{Command, JobPatch};
use crate::schedule::JobState;

/// Current wall-clock time in epoch milliseconds.
fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Fingerprint of `schedule.json`, or `None` if it doesn't exist or the path can't be determined.
fn current_schedule_stamp() -> Option<schedule::store::Stamp> {
    schedule::store::schedule_path().and_then(|p| schedule::store::stamp(&p))
}

/// Fingerprint of `watch_state.json`, or `None` if it doesn't exist or the path can't be determined.
fn current_watch_stamp() -> Option<schedule::store::Stamp> {
    schedule::store::watch_state_path().and_then(|p| schedule::store::stamp(&p))
}

impl App {
    /// Unconditionally reload `jobs` and `watch_state` from disk, refreshing the
    /// cached stamps that `poll_schedule_changed` compares against.
    pub fn reload_schedule(&mut self) {
        self.jobs = schedule::store::load().jobs;
        self.schedule_stamp = current_schedule_stamp();
        self.watch_state = schedule::store::load_watch_state();
        self.watch_stamp = current_watch_stamp();
    }

    /// Cheap change detection: compares the current on-disk stamps for
    /// `schedule.json` and `watch_state.json` against the cached ones from the
    /// last reload, and only re-parses either file when its stamp changed.
    // TODO(step-6): call from run_app's idle branch for live refresh
    pub fn poll_schedule_changed(&mut self) {
        if current_schedule_stamp() != self.schedule_stamp || current_watch_stamp() != self.watch_stamp {
            self.reload_schedule();
        }
    }

    /// Enqueue a command for the watch daemon, recording the error in
    /// `status_error` on failure rather than panicking or silently dropping it.
    ///
    /// Commands are only ever acted on by the daemon, so this also starts the
    /// daemon when `watch_autostart` is set. Without that, creating a job in a
    /// fresh install would queue a command that nothing ever reads and the job
    /// would sit at `Queued` forever with no indication why.
    pub fn enqueue_command(&mut self, cmd: Command) {
        // StopWatcher is the one command that must not resurrect the daemon.
        let autostart = self.config.watch_autostart && !matches!(cmd, Command::StopWatcher);
        if let Err(e) = schedule::command::enqueue(&cmd) {
            self.status_error = Some(format!("Failed to enqueue command: {e}"));
            return;
        }
        if autostart {
            self.ensure_watcher_running();
        }
    }

    /// Start the watch daemon if it is not already running. Idempotent: the
    /// daemon itself also refuses to start a second copy.
    pub fn ensure_watcher_running(&mut self) {
        match crate::watch::ensure_running(self.config.tmux_bin()) {
            Ok(started) => {
                if started {
                    self.status_error = None;
                }
                self.watch_running = true;
            }
            Err(e) => {
                self.status_error = Some(format!("Failed to start watcher: {e}"));
                self.watch_running = false;
            }
        }
    }

    /// Toggle the watch daemon on or off, for the Jobs tab's `s` key.
    pub fn toggle_watcher(&mut self) {
        let tmux = self.config.tmux_bin().to_string();
        if crate::watch::is_running(&tmux) {
            if let Err(e) = crate::watch::stop(&tmux) {
                self.status_error = Some(format!("Failed to stop watcher: {e}"));
                return;
            }
            self.watch_running = false;
        } else {
            self.ensure_watcher_running();
        }
        self.reload_schedule();
    }

    /// The job currently highlighted on the Jobs tab, if any.
    pub fn selected_job(&self) -> Option<&Job> {
        self.jobs.get(self.jobs_selected)
    }

    /// Move the Jobs tab selection up by one, clamped at zero.
    pub fn jobs_move_up(&mut self) {
        self.jobs_selected = self.jobs_selected.saturating_sub(1);
    }

    /// Move the Jobs tab selection down by one, clamped at the last job.
    pub fn jobs_move_down(&mut self) {
        if self.jobs_selected + 1 < self.jobs.len() {
            self.jobs_selected += 1;
        }
    }

    /// Switch the main window to the Jobs tab (bound to `w`), refreshing from
    /// disk first and clamping the selection in case the job list shrank since
    /// it was last shown.
    pub fn open_jobs_tab(&mut self) {
        self.poll_schedule_changed();
        // Check liveness on switch rather than trusting the cached flag: the
        // daemon can die at any time, and a stale "running" indicator is the
        // one failure mode that silently strands every job.
        self.watch_running = crate::watch::is_running(self.config.tmux_bin());
        if self.jobs_selected >= self.jobs.len() {
            self.jobs_selected = self.jobs.len().saturating_sub(1);
        }
        self.mode = AppMode::Normal;
        self.main_tab = MainTab::Jobs;
    }

    /// Switch the main window back to the Sessions tab.
    pub fn open_sessions_tab(&mut self) {
        self.mode = AppMode::Normal;
        self.main_tab = MainTab::Sessions;
    }

    /// Cycle the main window to the next tab, refreshing job state when landing
    /// on the Jobs tab so it never shows a stale list.
    pub fn cycle_main_tab(&mut self, forward: bool) {
        let next = if forward { self.main_tab.next() } else { self.main_tab.prev() };
        match next {
            MainTab::Jobs => self.open_jobs_tab(),
            MainTab::Sessions => self.open_sessions_tab(),
        }
    }

    /// Close a job modal (form or confirmation) and return to the Jobs tab.
    fn return_to_jobs_tab(&mut self) {
        self.mode = AppMode::Normal;
        self.main_tab = MainTab::Jobs;
    }

    /// Move the job form's model selection one step through the discovered
    /// list, wrapping at both ends. A hand-typed id that is not in the list
    /// counts as position 0, so cycling from it lands on the neighbours of the
    /// "(claude default)" entry rather than doing nothing.
    pub fn cycle_job_form_model(&mut self, forward: bool) {
        if self.model_options.is_empty() {
            return;
        }
        let len = self.model_options.len();
        let current = crate::models::index_of(&self.model_options, &self.job_form_model);
        let next = if forward {
            (current + 1) % len
        } else {
            (current + len - 1) % len
        };
        self.job_form_model = self.model_options[next].value.clone();
    }

    /// Open the directory picker for the job form's working-directory field.
    pub fn browse_job_cwd(&mut self) {
        let current = self.job_form_cwd.clone();
        self.open_path_picker(PickerTarget::JobCwd, &current);
    }

    /// Number of commands still queued for the daemon. Non-zero while the
    /// watcher is stopped means the user's actions are waiting, not lost.
    pub fn pending_command_count(&self) -> usize {
        schedule::command::pending_count()
    }

    /// Reset all job-form fields to their creation defaults.
    fn reset_job_form(&mut self) {
        self.job_form_field = 0;
        self.job_form_editing = false;
        self.job_form_input = Input::default();
        self.job_form_edit_id = None;
        self.job_form_name = String::new();
        self.job_form_cwd = String::new();
        self.job_form_prompt = String::new();
        self.job_form_continue_prompt = String::new();
        self.job_form_model = String::new();
        self.job_form_dangerous = false;
        self.job_form_pause_mode = self.config.pause_mode;
        self.job_form_auto_resume = true;
        self.job_form_bind = JobBind::New;
    }

    /// Open a blank job form for creating a brand-new job (`n` on the Jobs tab).
    pub fn open_job_form_new(&mut self) {
        self.reset_job_form();
        self.mode = AppMode::JobForm;
    }

    /// Open the job form prefilled from the selected job for editing (`e` on the Jobs tab).
    pub fn open_job_form_edit(&mut self) {
        let Some(job) = self.selected_job().cloned() else {
            return;
        };
        self.reset_job_form();
        self.job_form_edit_id = Some(job.id.clone());
        self.job_form_name = job.name.clone();
        self.job_form_cwd = job.cwd.clone();
        self.job_form_prompt = job.prompt.clone();
        self.job_form_continue_prompt = job.continue_prompt.clone().unwrap_or_default();
        self.job_form_model = job.model.clone().unwrap_or_default();
        self.job_form_dangerous = job.dangerous;
        self.job_form_pause_mode = job.pause_mode;
        self.job_form_auto_resume = job.auto_resume;
        self.mode = AppMode::JobForm;
    }

    /// Build a prefilled job form from the current list selection (the `m`
    /// binding): a live row prefills the tmux name and cwd (bind `Live`), a
    /// historical row prefills the chain-latest session id and its project cwd
    /// (bind `Resume`), and a header row (or anything else) prefills only the
    /// cwd for a brand-new job (bind `New`).
    pub fn job_form_from_selection(&mut self) {
        self.reset_job_form();
        if let Some(idx) = self.selected_live_index() {
            let ls = &self.live_sessions[idx];
            self.job_form_cwd = ls.cwd.clone();
            self.job_form_name = ls.display_name.clone();
            self.job_form_bind = JobBind::Live(ls.tmux_name.clone());
        } else if let Some(idx) = self.selected_session_index() {
            let session_id = self.resume_session_id_for(idx).to_string();
            self.job_form_cwd = self.sessions[idx].project.clone();
            self.job_form_bind = JobBind::Resume(session_id);
        } else if let Some(cwd) = self.selected_cwd() {
            self.job_form_cwd = cwd;
        }
        self.mode = AppMode::JobForm;
    }

    /// Validate and submit the in-progress job form: enqueues exactly one
    /// `CreateJob` (or `UpdateJob` when `job_form_edit_id` is `Some`), or sets
    /// `status_error` and leaves the form open on validation failure. A form
    /// bound to a live session (`JobBind::Live`) additionally enqueues
    /// `AdoptLive` so the daemon tags the already-running tmux session rather
    /// than the TUI touching tmux state directly.
    pub fn submit_job_form(&mut self) {
        let name = self.job_form_name.trim().to_string();
        let cwd_input = self.job_form_cwd.trim().to_string();
        if name.is_empty() {
            self.status_error = Some("Job name cannot be empty".to_string());
            return;
        }
        if cwd_input.is_empty() || !std::path::Path::new(&cwd_input).is_dir() {
            self.status_error = Some("Directory does not exist".to_string());
            return;
        }
        let cwd = schedule::canonical_cwd(&cwd_input);
        let model = if self.job_form_model.trim().is_empty() {
            None
        } else {
            Some(self.job_form_model.trim().to_string())
        };
        let continue_prompt = if self.job_form_continue_prompt.trim().is_empty() {
            None
        } else {
            Some(self.job_form_continue_prompt.trim().to_string())
        };

        if let Some(id) = self.job_form_edit_id.clone() {
            let patch = JobPatch {
                name: Some(name),
                cwd: Some(cwd),
                prompt: Some(self.job_form_prompt.clone()),
                continue_prompt: Some(continue_prompt),
                model: Some(model),
                pause_mode: Some(self.job_form_pause_mode),
                dangerous: Some(self.job_form_dangerous),
                auto_resume: Some(self.job_form_auto_resume),
            };
            self.enqueue_command(Command::UpdateJob { id, patch });
        } else {
            let claude_session_id = match &self.job_form_bind {
                JobBind::Resume(id) => Some(id.clone()),
                JobBind::New | JobBind::Live(_) => None,
            };
            let created_ms = now_ms();
            let job = Job {
                id: uuid::Uuid::new_v4().to_string(),
                name,
                cwd,
                prompt: self.job_form_prompt.clone(),
                continue_prompt,
                claude_session_id,
                tmux_name: None,
                state: JobState::default(),
                pause_mode: self.job_form_pause_mode,
                dangerous: self.job_form_dangerous,
                model,
                auto_resume: self.job_form_auto_resume,
                created_at_ms: created_ms,
                updated_at_ms: created_ms,
                paused_at_ms: None,
                resume_after_ms: None,
                last_error: None,
                attempts: 0,
                history: Vec::new(),
            };
            let job_id = job.id.clone();
            let live_bind = match &self.job_form_bind {
                JobBind::Live(tmux_name) => Some(tmux_name.clone()),
                _ => None,
            };
            self.enqueue_command(Command::CreateJob { job });
            if let Some(tmux_name) = live_bind {
                self.enqueue_command(Command::AdoptLive { id: job_id, tmux_name });
            }
        }

        self.status_error = None;
        self.reset_job_form();
        self.return_to_jobs_tab();
    }

    /// Toggle `auto_resume` on the selected job via `Command::UpdateJob`.
    pub fn toggle_selected_job_auto_resume(&mut self) {
        let Some(job) = self.selected_job() else {
            return;
        };
        let id = job.id.clone();
        let new_value = !job.auto_resume;
        self.enqueue_command(Command::UpdateJob {
            id,
            patch: JobPatch {
                auto_resume: Some(new_value),
                ..Default::default()
            },
        });
    }

    /// Request that the selected job be paused (`Command::PauseJob`).
    pub fn pause_selected_job(&mut self) {
        if let Some(job) = self.selected_job() {
            let id = job.id.clone();
            self.enqueue_command(Command::PauseJob { id });
        }
    }

    /// Request that the selected job be resumed (`Command::ResumeJob`).
    pub fn resume_selected_job(&mut self) {
        if let Some(job) = self.selected_job() {
            let id = job.id.clone();
            self.enqueue_command(Command::ResumeJob { id });
        }
    }

    /// Open the `JobConfirm` prompt for the selected job.
    pub fn open_job_confirm(&mut self, action: JobConfirmAction) {
        if let Some(job) = self.selected_job() {
            self.jobs_confirm = Some((job.id.clone(), action));
            self.mode = AppMode::JobConfirm;
        }
    }

    /// Enqueue the confirmed action (`StopJob`, `DeleteJob`, or `MarkDone`) and
    /// return to `Jobs`.
    pub fn confirm_job_action(&mut self) {
        if let Some((id, action)) = self.jobs_confirm.take() {
            match action {
                JobConfirmAction::Stop => self.enqueue_command(Command::StopJob { id }),
                JobConfirmAction::Delete => self.enqueue_command(Command::DeleteJob { id }),
                JobConfirmAction::Done => self.enqueue_command(Command::MarkDone { id }),
            }
        }
        self.return_to_jobs_tab();
    }

    /// Cancel the pending confirmation and return to `Jobs`.
    pub fn cancel_job_confirm(&mut self) {
        self.jobs_confirm = None;
        self.return_to_jobs_tab();
    }

    /// Stop the selected live session. When it's tagged with a scheduler job id
    /// (`LiveSession::job_id`), enqueues `Command::StopJob` so the daemon (the
    /// sole tmux writer for managed sessions) handles it instead of the TUI
    /// racing it directly; otherwise falls back to the direct `live::stop_live_session` path.
    pub fn stop_selected_live_session(&mut self) {
        let Some(idx) = self.selected_live_index() else {
            return;
        };
        let session = &self.live_sessions[idx];
        if let Some(job_id) = session.job_id.clone() {
            self.enqueue_command(Command::StopJob { id: job_id });
            // With autostart off the daemon may never drain this command, and
            // the session would sit there looking alive with no explanation.
            // Only check in that case: with autostart on we just started the
            // daemon and its heartbeat has not landed yet, so `is_running`
            // would report a false negative.
            if !self.config.watch_autostart {
                self.watch_running = crate::watch::is_running(self.config.tmux_bin());
                if !self.watch_running {
                    self.status_error = Some(
                        "Watcher not running: stop is queued until it starts (s on the Jobs tab)"
                            .to_string(),
                    );
                }
            }
            return;
        }
        let name = session.tmux_name.clone();
        if let Err(e) = live::stop_live_session(self.config.tmux_bin(), &name) {
            eprintln!("Failed to stop session: {e}");
        }
        self.live_sessions = live::discover_live_sessions(self.config.tmux_bin());
        self.live_preview_cache.remove(&name);
        self.recompute_flat_rows();
        self.recompute_tree();
    }
}
