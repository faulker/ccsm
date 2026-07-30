mod activity;
mod chain;
mod dir_browser;
mod display;
mod filter;
mod flat;
mod jobs;
mod preview;
mod selection;
mod tree;

#[cfg(test)]
mod tests;

use crate::config::{Config, DisplayMode, PauseMode};
use crate::data::{self, AgentBackend, PreviewMessage, SessionInfo, SessionMeta};
use crate::live::{self, ActivityState, LiveSession};
use crate::models;
use crate::schedule::{self, Job};
use crate::update;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::time::Instant;
use tui_input::Input;

pub use dir_browser::{DirBrowser, PickerKind, PickerTarget};

/// Enrichment from the background session-meta loader.
#[derive(Debug, Clone)]
pub struct SessionMetaUpdate {
    /// Custom title, when known. Cursor may send `None` for untitled chats.
    pub name: Option<String>,
    /// Cursor turn count from `store.db`. Claude leaves this `None`.
    pub entry_count: Option<usize>,
}

/// Which agent backends are visible in the Sessions list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceFilter {
    /// Show Claude Code and Cursor Agent sessions.
    Both,
    /// Show only Claude Code sessions.
    Claude,
    /// Show only Cursor Agent sessions.
    Cursor,
}

impl SourceFilter {
    /// Parse the persisted config string (`"both"` / `"claude"` / `"cursor"`).
    pub fn from_config(s: &str) -> Self {
        match s {
            "claude" => Self::Claude,
            "cursor" => Self::Cursor,
            _ => Self::Both,
        }
    }

    /// Persistable config value for this filter.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Both => "both",
            Self::Claude => "claude",
            Self::Cursor => "cursor",
        }
    }

    /// Cycle Both → Claude → Cursor → Both.
    pub fn cycle(self) -> Self {
        match self {
            Self::Both => Self::Claude,
            Self::Claude => Self::Cursor,
            Self::Cursor => Self::Both,
        }
    }
}

/// Which top-level tab of the main window is currently showing.
///
/// Tabs replace what used to be a Jobs popup and, later, a Config popup: both
/// are peers of the session list rather than overlays, so all three share the
/// list/detail layout and none of them can be buried under another modal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MainTab {
    /// The session browser (default).
    Sessions,
    /// The scheduler jobs manager.
    Jobs,
    /// Application settings.
    Config,
}

impl MainTab {
    /// The next tab in the strip, wrapping around.
    pub fn next(self) -> Self {
        match self {
            MainTab::Sessions => MainTab::Jobs,
            MainTab::Jobs => MainTab::Config,
            MainTab::Config => MainTab::Sessions,
        }
    }

    /// The previous tab in the strip, wrapping around.
    pub fn prev(self) -> Self {
        match self {
            MainTab::Sessions => MainTab::Config,
            MainTab::Jobs => MainTab::Sessions,
            MainTab::Config => MainTab::Jobs,
        }
    }
}

/// Which page of the tabbed help overlay is showing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HelpTab {
    /// Session-list navigation and actions.
    Sessions,
    /// Jobs tab, job form, and watcher keys.
    Jobs,
    /// Global keys plus config and directory-picker keys.
    General,
}

impl HelpTab {
    /// All help tabs in display order.
    pub const ALL: [HelpTab; 3] = [HelpTab::Sessions, HelpTab::Jobs, HelpTab::General];

    /// Tab label shown in the help tab strip.
    pub fn label(self) -> &'static str {
        match self {
            HelpTab::Sessions => "Sessions",
            HelpTab::Jobs => "Jobs",
            HelpTab::General => "General",
        }
    }

    /// The next help tab, wrapping around.
    pub fn next(self) -> Self {
        match self {
            HelpTab::Sessions => HelpTab::Jobs,
            HelpTab::Jobs => HelpTab::General,
            HelpTab::General => HelpTab::Sessions,
        }
    }

    /// The previous help tab, wrapping around.
    pub fn prev(self) -> Self {
        match self {
            HelpTab::Sessions => HelpTab::General,
            HelpTab::Jobs => HelpTab::Sessions,
            HelpTab::General => HelpTab::Jobs,
        }
    }
}

/// One visible row in the tree-view session list.
#[derive(Debug, Clone, PartialEq)]
pub enum TreeRow {
    /// Top-level collapsible header for a project directory.
    Header {
        project: String,
        project_name: String,
        session_count: usize,
    },
    /// A historical (non-live) Claude session row.
    Session {
        session_index: usize,
    },
    /// Collapsible sub-header grouping the live sessions for a project.
    RunningHeader {
        project: String,
        count: usize,
    },
    /// Collapsible sub-header grouping the historical sessions for a project.
    HistoryHeader {
        project: String,
        count: usize,
    },
    /// A running live tmux session row.
    LiveItem {
        live_index: usize,
    },
    /// Visual divider between favorited and non-favorited project groups.
    FavoritesSeparator,
}

/// Describes how to launch or attach to an agent session after the TUI exits.
#[derive(Debug, Clone)]
pub enum LaunchRequest {
    /// Resume a historical session inside a new tmux live session.
    Resume {
        session_id: String,
        cwd: String,
        backend: AgentBackend,
    },
    /// Resume a historical session directly in the foreground (no tmux).
    Direct {
        session_id: String,
        cwd: String,
        backend: AgentBackend,
    },
    /// Attach the terminal to an already-running live tmux session.
    AttachLive { tmux_name: String },
    /// Create and attach to a new live tmux session running an agent.
    NewLive {
        /// tmux session name (also passed to Claude as `--name`; Cursor has no name flag).
        name: String,
        /// Working directory the session starts in.
        cwd: String,
        /// Launch with `--dangerously-skip-permissions` (Claude) or `--force` (Cursor).
        dangerous: bool,
        /// Launch with `--worktree <name>`.
        worktree: bool,
        backend: AgentBackend,
    },
    /// Start a new agent session directly in the foreground (no tmux).
    NewDirect {
        cwd: String,
        backend: AgentBackend,
    },
}

/// One visible row in the flat-view session list.
#[derive(Debug, Clone, PartialEq)]
pub enum FlatRow {
    /// Header row showing the total count of running live sessions.
    RunningHeader { count: usize },
    /// A running live tmux session row.
    LiveItem { live_index: usize },
    /// Visual divider between the live section and the history section.
    Separator,
    /// A historical (non-live) Claude session row.
    HistoryItem { session_index: usize },
    /// Visual divider between favorited and non-favorited history items.
    FavoritesSeparator,
}

/// The current interaction mode of the application, controlling how key events are dispatched.
#[derive(Debug, Clone, PartialEq)]
pub enum AppMode {
    /// Default navigation mode.
    Normal,
    /// The rename input popup is open.
    Renaming,
    /// The update-available confirmation prompt is shown.
    UpdatePrompt,
    /// The help overlay is displayed.
    Help,
    /// The new-session naming popup is open.
    NamingSession,
    /// The directory-picker modal is open (choosing a cwd for a new session).
    DirPicker,
    /// A duplicate session name was entered; waiting for the user to choose open vs. rename.
    DuplicateSession,
    /// One or more required binaries (claude/tmux) are missing.
    MissingDeps,
    /// The job create/edit form is open (reached only from the Jobs tab).
    JobForm,
    /// A destructive job action (stop/delete) is awaiting y/n confirmation (reached only from the Jobs tab).
    JobConfirm,
    /// Stopping a live session is awaiting y/n confirmation (reached only from the Sessions tab).
    StopSessionConfirm,
}

/// Which row of the new-session naming popup has keyboard focus.
///
/// Focus starts on the name. Down moves to Agent (when both backends are
/// shown) then Type; Left/Right cycle the focused switcher. The name field
/// keeps Left/Right for the text cursor, so agent/type never steal letter keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NamingFocus {
    /// The session name input (or the "name unused" note in direct mode).
    Name,
    /// Agent backend switcher (only when the source filter is Both).
    Agent,
    /// Launch-mode switcher (plain / danger / worktree / direct).
    Type,
}

/// How a new session launched from the naming popup should be started.
///
/// These used to be a pair of booleans set by whichever key opened the popup.
/// They are one cycled enum so a single `n` covers every launch mode, which is
/// what keeps the status bar down to one "new" hint instead of four.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NewSessionMode {
    /// A normal live session in tmux.
    Plain,
    /// A live session launched with `--dangerously-skip-permissions`.
    Dangerous,
    /// A live session that gets its own git worktree.
    Worktree,
    /// A plain `claude` process with no tmux session (the name is unused).
    Direct,
}

impl NewSessionMode {
    /// Every mode, in cycle order.
    pub const ALL: [NewSessionMode; 4] = [
        NewSessionMode::Plain,
        NewSessionMode::Dangerous,
        NewSessionMode::Worktree,
        NewSessionMode::Direct,
    ];

    /// Short label shown in the naming popup's mode row.
    pub fn label(self) -> &'static str {
        match self {
            NewSessionMode::Plain => "plain",
            NewSessionMode::Dangerous => "danger",
            NewSessionMode::Worktree => "worktree",
            NewSessionMode::Direct => "direct",
        }
    }

    /// The next mode in cycle order, wrapping.
    pub fn next(self) -> NewSessionMode {
        let i = Self::ALL.iter().position(|&m| m == self).unwrap_or(0);
        Self::ALL[(i + 1) % Self::ALL.len()]
    }

    /// The previous mode in cycle order, wrapping.
    pub fn prev(self) -> NewSessionMode {
        let i = Self::ALL.iter().position(|&m| m == self).unwrap_or(0);
        Self::ALL[(i + Self::ALL.len() - 1) % Self::ALL.len()]
    }

    /// True when this mode launches through tmux and therefore needs a name.
    pub fn needs_name(self) -> bool {
        self != NewSessionMode::Direct
    }
}

/// Tracks which popup triggered the duplicate-name check, so we can return to the right mode.
#[derive(Debug, Clone, PartialEq)]
pub enum DuplicateSource {
    /// Duplicate detected while naming a new live session.
    NamingSession,
    /// Duplicate detected while renaming an existing live session.
    Renaming,
}

/// How a submitted job form ties back to an existing session, if at all.
/// Set by `job_form_from_selection` (the `m` binding) and consumed by `submit_job_form`.
#[derive(Debug, Clone, PartialEq)]
pub enum JobBind {
    /// A brand-new job with no existing session to bind to.
    New,
    /// Prefilled from a historical session; carries the chain-latest session id to resume.
    Resume(String),
    /// Prefilled from a live tmux session; carries the tmux session name to adopt.
    Live(String),
}

/// What a `JobConfirm` prompt is asking the user to confirm.
#[derive(Debug, Clone, PartialEq)]
pub enum JobConfirmAction {
    /// Hard-stop the job (`Command::StopJob`).
    Stop,
    /// Delete the job entirely (`Command::DeleteJob`).
    Delete,
    /// Mark the job finished by hand (`Command::MarkDone`), for work the agent
    /// completed without ever emitting the completion marker.
    Done,
}


/// Central application state shared by the event handler, update loop, and rendering code.
pub struct App {
    /// All sessions loaded from history (unfiltered).
    pub sessions: Vec<SessionInfo>,
    /// Zero-based index of the currently highlighted row.
    pub selected: usize,
    /// Cache mapping session/chain cache keys to their loaded preview data.
    pub preview_cache: HashMap<String, (SessionMeta, Vec<PreviewMessage>)>,
    /// Current vertical scroll offset in the preview pane (`u16::MAX` = scroll to bottom).
    pub preview_scroll: u16,
    /// When true, the preview pane automatically follows new output (scrolls to bottom).
    pub preview_auto_scroll: bool,
    /// Set to true when the user requests to exit the application.
    pub should_quit: bool,
    /// Populated when a session launch has been requested; consumed by the main loop.
    pub launch_session: Option<LaunchRequest>,
    /// Input state for the live filter bar.
    pub filter_input: Input,
    /// True while the filter input is in focus (editing mode).
    pub filter_active: bool,
    /// Indices into `sessions` that match the current filter, sorted by recency.
    pub filtered_indices: Vec<usize>,
    /// Optional path prefix used to restrict sessions to a specific project directory.
    pub filter_path: Option<String>,
    /// When true, sessions are displayed in a collapsible tree grouped by project.
    pub tree_view: bool,
    /// Controls how session labels are rendered.
    pub display_mode: DisplayMode,
    /// Flattened sequence of rows for the tree view, recomputed on state changes.
    pub tree_rows: Vec<TreeRow>,
    /// Set of project keys (and sub-keys like `"running:<project>"`) that are collapsed in tree view.
    pub collapsed: HashSet<String>,
    /// When true, sessions with no JSONL data file are hidden.
    pub hide_empty: bool,
    /// When true, sessions sharing a slug are grouped into a single chain entry.
    pub group_chains: bool,
    /// canonical_idx → all indices in the chain, sorted oldest→newest
    pub chain_map: HashMap<usize, Vec<usize>>,
    /// Current interaction mode controlling key dispatch.
    pub mode: AppMode,
    /// Which top-level tab of the main window is showing.
    pub main_tab: MainTab,
    /// Which page of the help overlay is showing while `mode == AppMode::Help`.
    pub help_tab: HelpTab,
    /// Persisted configuration; updated and saved when settings change.
    pub config: Config,
    /// True while a Shift key is held down, used to highlight shift-key hints in the status bar.
    pub shift_active: bool,
    /// Input state for the rename popup.
    pub rename_input: Input,
    /// Session ID being renamed, or tmux name if renaming a live session.
    pub rename_session_id: Option<String>,
    /// Project path for the session being renamed (`None` when renaming a live session).
    pub rename_project: Option<String>,
    /// Current state of the update check / download lifecycle.
    pub update_status: update::UpdateStatus,
    /// Populated when the user confirms an update; consumed by the main loop to run the download.
    pub perform_update: Option<update::UpdateInfo>,
    /// Receiver end of the background update-check thread channel.
    pub update_receiver: Option<std::sync::mpsc::Receiver<update::UpdateInfo>>,
    /// Receiver end of the background session-name loading thread channel.
    pub names_receiver: Option<std::sync::mpsc::Receiver<HashMap<String, SessionMetaUpdate>>>,
    /// Set to true when the process should exec-restart itself after an update.
    pub should_restart: bool,
    /// Set to true whenever state changes require the screen to be redrawn.
    pub needs_redraw: bool,
    /// All currently running live tmux sessions on the ccsm socket.
    pub live_sessions: Vec<LiveSession>,
    /// When true, only projects with active live sessions are shown.
    pub live_filter: bool,
    /// Which agent backends are shown in the Sessions list.
    pub source_filter: SourceFilter,
    /// Input state for the new-session naming popup.
    pub naming_input: Input,
    /// Auto-generated placeholder shown when `naming_text` is empty.
    pub naming_placeholder: String,
    /// Working directory to use for the new session being named.
    pub naming_cwd: Option<String>,
    /// Cache of recently captured tmux pane output (with ANSI codes) keyed by tmux session name,
    /// with the per-session timestamp of the last refresh.
    pub live_preview_cache: HashMap<String, (String, Instant)>,
    /// Flattened sequence of rows for the flat view, recomputed on state changes.
    pub flat_rows: Vec<FlatRow>,
    /// Set of project paths pinned to the top of the list.
    pub favorites: HashSet<String>,
    /// The conflicting session name that triggered `AppMode::DuplicateSession`.
    pub duplicate_name: Option<String>,
    /// Which popup triggered the duplicate check, so we know where to return.
    pub duplicate_source: Option<DuplicateSource>,
    /// The cwd to restore if the user chooses to pick a different name (NamingSession source only).
    pub duplicate_cwd: Option<String>,
    /// Currently selected row in the Config tab's settings list (0..=CONFIG_MAX_ROW).
    pub config_selected: usize,
    /// True when the `claude` binary cannot be found (soft unless agent is also missing).
    pub missing_claude: bool,
    /// True when the `agent` binary cannot be found (soft unless claude is also missing).
    pub missing_agent: bool,
    /// True when the `tmux` binary cannot be found (always blocking).
    pub missing_tmux: bool,
    /// True when editing a text field on the Config tab (paths, percentages, prompts).
    pub config_editing: bool,
    /// Input state for the text field being edited on the Config tab.
    pub config_path_input: Input,
    /// Per-session activity state (Active, Idle, Unknown).
    pub activity_states: HashMap<String, ActivityState>,
    /// Per-session timestamp of last activity poll, for throttling.
    pub activity_last_poll: HashMap<String, Instant>,
    /// Monotonic tick counter, incremented each redraw to drive pulse animation.
    pub tick: u64,
    /// Which launch mode the open naming popup will use. Focus the Type row
    /// and cycle with Left/Right rather than choosing by the key that opened it.
    pub naming_mode: NewSessionMode,
    /// Which agent backend a new session from the naming popup will launch.
    /// Follows the source filter when it is Claude/Cursor-only; when Both,
    /// defaults to the last agent used in that directory (else Claude) and is
    /// cycled on the Agent focus row.
    pub naming_backend: AgentBackend,
    /// Which naming-popup row receives Up/Down focus and Left/Right cycling.
    pub naming_focus: NamingFocus,
    /// The live session awaiting stop confirmation in `AppMode::StopSessionConfirm`.
    pub stop_confirm_name: Option<String>,
    /// Vertical scroll offset for the help overlay, in lines.
    pub help_scroll: u16,
    /// Last error message to display in the status bar.
    pub status_error: Option<String>,
    /// State for the directory-picker modal, or `None` when it's not open.
    pub dir_browser: Option<DirBrowser>,
    /// Which field the currently open directory picker is choosing a path for.
    pub dir_picker_target: PickerTarget,
    /// All jobs known to the scheduler, reloaded from `schedule.json` by `reload_schedule`.
    pub jobs: Vec<Job>,
    /// Currently highlighted row in the Jobs popup.
    pub jobs_selected: usize,
    /// Fingerprint of `schedule.json` as of the last `reload_schedule`, used by `poll_schedule_changed`.
    pub schedule_stamp: Option<schedule::store::Stamp>,
    /// The watch daemon's last-persisted state, or `None` if it has never run.
    pub watch_state: Option<schedule::store::WatchState>,
    /// Fingerprint of `watch_state.json` as of the last `reload_schedule`, used by `poll_schedule_changed`.
    pub watch_stamp: Option<schedule::store::Stamp>,
    /// True when the configured usage source cannot produce data at all (the
    /// local history file is missing and the source is pinned to `local`).
    pub missing_usage: bool,
    /// Usage sampled by the TUI itself from the local history file. The chip
    /// prefers this over `watch_state`, so it stays live with no watcher
    /// running at all. `None` until the first successful poll.
    pub usage: Option<crate::usage::UsageSnapshot>,
    /// When `poll_usage` last ran, so it can rate-limit itself independently of
    /// the event loop's tick rate.
    pub usage_polled_at: Option<std::time::Instant>,
    /// Whether the watch daemon was running as of the last check. Drives the
    /// title-bar indicator, so a silently dead watcher stays visible.
    pub watch_running: bool,
    /// The job id and action awaiting confirmation in `AppMode::JobConfirm`, or `None`.
    pub jobs_confirm: Option<(String, JobConfirmAction)>,
    /// Currently highlighted field row in the job form (0..=8).
    pub job_form_field: usize,
    /// True while a text field in the job form is being edited.
    pub job_form_editing: bool,
    /// Input state for whichever text field in the job form is being edited.
    pub job_form_input: Input,
    /// `Some(id)` when the job form is editing an existing job rather than creating one.
    pub job_form_edit_id: Option<String>,
    /// Job-form field: display name.
    pub job_form_name: String,
    /// Job-form field: working directory.
    pub job_form_cwd: String,
    /// Job-form field: initial prompt.
    pub job_form_prompt: String,
    /// Job-form field: continue-prompt override (empty means use the configured default).
    pub job_form_continue_prompt: String,
    /// Job-form field: model override (empty means use claude's default).
    pub job_form_model: String,
    /// Job-form field: whether to launch with `--dangerously-skip-permissions`.
    pub job_form_dangerous: bool,
    /// Job-form field: pause strategy for this job.
    pub job_form_pause_mode: PauseMode,
    /// Job-form field: whether the daemon should auto-resume this job.
    pub job_form_auto_resume: bool,
    /// How the in-progress job form ties back to an existing session, set by `job_form_from_selection`.
    pub job_form_bind: JobBind,
    /// Models offered by the job form's model picker, discovered once at startup
    /// from Claude Code's own state (see `crate::models`).
    pub model_options: Vec<models::ModelOption>,
}

/// Truncate a path to its last 2 components (e.g. "/Users/sane/Dev/ccsm" -> "Dev/ccsm").
fn truncate_path(path: &str) -> String {
    let trimmed = path.trim_end_matches('/');
    let parts: Vec<&str> = trimmed.rsplitn(3, '/').collect();
    if parts.len() >= 2 {
        format!("{}/{}", parts[1], parts[0])
    } else {
        trimmed.to_string()
    }
}

impl App {
    /// Construct a new `App`, applying configuration defaults, discovering live sessions,
    /// spawning the background session-name loader, and building initial filter/tree state.
    pub fn new(sessions: Vec<SessionInfo>, filter_path: Option<String>, config: Config) -> Self {
        let filtered_indices: Vec<usize> = (0..sessions.len()).collect();
        let group_chains = config.group_chains;
        let live_filter = config.live_filter;
        let source_filter = SourceFilter::from_config(&config.source_filter);
        let favorites = config.favorites.clone();
        let live_sessions = live::discover_live_sessions(config.tmux_bin());
        let mut app = Self {
            sessions,
            selected: 0,
            preview_cache: HashMap::new(),
            preview_scroll: u16::MAX,
            preview_auto_scroll: true,
            should_quit: false,
            launch_session: None,
            filter_input: Input::default(),
            filter_active: false,
            filtered_indices,
            filter_path,
            tree_view: config.tree_view,
            display_mode: config.display_mode,
            hide_empty: config.hide_empty,
            group_chains,
            chain_map: HashMap::new(),
            tree_rows: Vec::new(),
            collapsed: HashSet::new(),
            mode: AppMode::Normal,
            main_tab: MainTab::Sessions,
            help_tab: HelpTab::Sessions,
            config,
            shift_active: false,
            rename_input: Input::default(),
            rename_session_id: None,
            rename_project: None,
            update_status: update::UpdateStatus::None,
            perform_update: None,
            update_receiver: None,
            names_receiver: None,
            should_restart: false,
            needs_redraw: true,
            live_sessions,
            live_filter,
            source_filter,
            naming_input: Input::default(),
            naming_placeholder: String::new(),
            naming_cwd: None,
            live_preview_cache: HashMap::new(),
            flat_rows: Vec::new(),
            favorites,
            duplicate_name: None,
            duplicate_source: None,
            duplicate_cwd: None,
            config_selected: 0,
            missing_claude: false,
            missing_agent: false,
            missing_tmux: false,
            config_editing: false,
            config_path_input: Input::default(),
            activity_states: HashMap::new(),
            activity_last_poll: HashMap::new(),
            tick: 0,
            naming_mode: NewSessionMode::Plain,
            naming_backend: AgentBackend::ClaudeCode,
            naming_focus: NamingFocus::Name,
            stop_confirm_name: None,
            help_scroll: 0,
            status_error: None,
            dir_browser: None,
            dir_picker_target: PickerTarget::NewSession,
            jobs: Vec::new(),
            jobs_selected: 0,
            schedule_stamp: None,
            watch_state: None,
            watch_stamp: None,
            missing_usage: false,
            usage: None,
            usage_polled_at: None,
            watch_running: false,
            jobs_confirm: None,
            job_form_field: 0,
            job_form_editing: false,
            job_form_input: Input::default(),
            job_form_edit_id: None,
            job_form_name: String::new(),
            job_form_cwd: String::new(),
            job_form_prompt: String::new(),
            job_form_continue_prompt: String::new(),
            job_form_model: String::new(),
            job_form_dangerous: false,
            job_form_pause_mode: PauseMode::default(),
            job_form_auto_resume: true,
            job_form_bind: JobBind::New,
            model_options: models::available(),
        };

        // tmux missing always blocks. Exactly one of claude/agent missing is soft.
        app.refresh_bin_availability();
        if app.deps_blocking() {
            app.mode = AppMode::MissingDeps;
        }
        app.missing_usage = crate::usage::source_unavailable(
            &app.config.usage_source,
            app.config.usage_history_override(),
        );

        app.spawn_load_session_names();
        app.init_tree();
        app.recompute_filter();
        app.reload_schedule();
        app
    }

    /// Spawn a background thread that loads titles (and Cursor entry counts).
    ///
    /// Claude titles come from JSONL `custom-title` entries. Cursor titles and
    /// message counts come from `store.db`, which is deliberately not opened
    /// during the list scan so startup stays fast with many chats.
    pub fn spawn_load_session_names(&mut self) {
        let sessions: Vec<(AgentBackend, String, String)> = self
            .sessions
            .iter()
            .filter(|s| s.has_data)
            .map(|s| (s.backend, s.project.clone(), s.session_id.clone()))
            .collect();

        let (tx, rx) = std::sync::mpsc::channel();
        self.names_receiver = Some(rx);

        std::thread::spawn(move || {
            let mut updates = HashMap::new();
            for (backend, project, session_id) in sessions {
                match backend {
                    AgentBackend::ClaudeCode => {
                        if let Some(title) = data::load_custom_title(&project, &session_id) {
                            updates.insert(
                                session_id,
                                SessionMetaUpdate {
                                    name: Some(title),
                                    entry_count: None,
                                },
                            );
                        }
                    }
                    AgentBackend::CursorAgent => {
                        if let Some(path) = data::cursor_history::find_cursor_store(&session_id) {
                            if let Some(meta) = data::cursor_store::read_store_meta(&path) {
                                updates.insert(
                                    session_id,
                                    SessionMetaUpdate {
                                        name: meta.name,
                                        entry_count: Some(meta.entry_count),
                                    },
                                );
                            }
                        }
                    }
                }
            }
            let _ = tx.send(updates);
        });
    }

    /// Apply titles / Cursor counts from the background loader, then refresh views.
    pub fn apply_session_names(&mut self, updates: HashMap<String, SessionMetaUpdate>) {
        for session in &mut self.sessions {
            let Some(update) = updates.get(&session.session_id) else {
                continue;
            };
            match session.backend {
                AgentBackend::ClaudeCode => {
                    if let Some(ref title) = update.name {
                        session.name = Some(title.clone());
                    }
                }
                AgentBackend::CursorAgent => {
                    session.name = update.name.clone();
                    if let Some(count) = update.entry_count {
                        session.entry_count = count;
                    }
                }
            }
        }

        self.preview_cache.clear();
        self.recompute_tree();
        self.recompute_flat_rows();
    }

    /// Replace the session list with a freshly loaded set, reset caches, and rebuild all views.
    pub fn reload_sessions(&mut self, sessions: Vec<SessionInfo>) {
        self.sessions = sessions;
        self.spawn_load_session_names();
        self.preview_cache.clear();
        self.preview_scroll = u16::MAX;
        self.preview_auto_scroll = true;
        self.recompute_filter();
        self.recompute_tree();
        self.recompute_flat_rows();
        if self.selected >= self.visible_item_count() {
            self.selected = self.visible_item_count().saturating_sub(1);
        }
    }

    /// Re-check whether the configured claude, agent, and tmux binaries exist.
    pub(crate) fn refresh_bin_availability(&mut self) {
        self.missing_claude = !Config::is_bin_available(self.config.claude_bin());
        self.missing_agent = !Config::is_bin_available(self.config.agent_bin());
        self.missing_tmux = !Config::is_bin_available(self.config.tmux_bin());
    }

    /// True when the missing-deps dialog must block the app.
    ///
    /// tmux missing always blocks. Exactly one of claude/agent missing does not;
    /// both agents missing does.
    pub(crate) fn deps_blocking(&self) -> bool {
        self.missing_tmux || (self.missing_claude && self.missing_agent)
    }

    /// Default agent for a new session in `dir`.
    ///
    /// Claude/Cursor-only source filters force that backend. When Both, pick
    /// the most recently active historical session in `dir`, falling back to
    /// Claude when the directory has no history yet.
    pub(crate) fn default_naming_backend(&self, dir: &str) -> AgentBackend {
        match self.source_filter {
            SourceFilter::Cursor => AgentBackend::CursorAgent,
            SourceFilter::Claude => AgentBackend::ClaudeCode,
            SourceFilter::Both => self
                .last_backend_for_dir(dir)
                .unwrap_or(AgentBackend::ClaudeCode),
        }
    }

    /// Backend of the most recent historical session whose project matches `dir`.
    fn last_backend_for_dir(&self, dir: &str) -> Option<AgentBackend> {
        let dir = Path::new(dir);
        self.sessions
            .iter()
            .filter(|s| Path::new(&s.project) == dir)
            .max_by_key(|s| s.last_timestamp)
            .map(|s| s.backend)
    }

    /// Cycle the naming popup's backend when the source filter is Both.
    pub(crate) fn cycle_naming_backend(&mut self) {
        if self.source_filter != SourceFilter::Both {
            return;
        }
        self.naming_backend = match self.naming_backend {
            AgentBackend::ClaudeCode => AgentBackend::CursorAgent,
            AgentBackend::CursorAgent => AgentBackend::ClaudeCode,
        };
    }

    /// Move naming-popup focus down: Name → Agent (if Both) → Type.
    pub(crate) fn naming_focus_down(&mut self) {
        let show_agent = self.source_filter == SourceFilter::Both;
        self.naming_focus = match self.naming_focus {
            NamingFocus::Name if show_agent => NamingFocus::Agent,
            NamingFocus::Name | NamingFocus::Agent => NamingFocus::Type,
            NamingFocus::Type => NamingFocus::Type,
        };
    }

    /// Move naming-popup focus up: Type → Agent (if Both) → Name.
    pub(crate) fn naming_focus_up(&mut self) {
        let show_agent = self.source_filter == SourceFilter::Both;
        self.naming_focus = match self.naming_focus {
            NamingFocus::Type if show_agent => NamingFocus::Agent,
            NamingFocus::Type | NamingFocus::Agent => NamingFocus::Name,
            NamingFocus::Name => NamingFocus::Name,
        };
    }

    /// Return false (and set `status_error`) when `backend`'s binary is missing.
    pub(crate) fn ensure_backend_available(&mut self, backend: AgentBackend) -> bool {
        self.refresh_bin_availability();
        match backend {
            AgentBackend::ClaudeCode if self.missing_claude => {
                self.status_error = Some(
                    "claude binary not found — set the path on the Config tab".to_string(),
                );
                false
            }
            AgentBackend::CursorAgent if self.missing_agent => {
                self.status_error = Some(
                    "agent binary not found — set the path on the Config tab".to_string(),
                );
                false
            }
            _ => true,
        }
    }

    /// Open the new-session naming popup for the selected row's project,
    /// starting in `mode`.
    ///
    /// The launch mode is cycled inside the popup (see `cycle_naming_mode`)
    /// rather than fixed by the key that opened it, so `n` is the single entry
    /// point for every kind of new session. Returns false (leaving the mode
    /// unchanged) when there is no selectable cwd, or when `mode` is
    /// `Worktree` outside a git repo.
    pub fn open_naming_popup(&mut self, mode: NewSessionMode) -> bool {
        let Some(cwd) = self.selected_cwd() else {
            return false;
        };
        // Prefer the selected project for agent defaulting even when the path
        // is gone from disk (launch then falls back to ".").
        let naming_backend = self.default_naming_backend(&cwd);
        let dir = if Path::new(&cwd).exists() {
            cwd
        } else {
            ".".to_string()
        };
        if mode == NewSessionMode::Worktree && !live::is_git_repo(&dir) {
            self.status_error = Some(format!("{dir} is not a git repository"));
            return false;
        }
        self.naming_placeholder = live::generate_auto_name(&dir, &self.live_sessions);
        self.naming_backend = naming_backend;
        self.naming_cwd = Some(dir);
        self.naming_input = Input::default();
        self.naming_mode = mode;
        self.naming_focus = NamingFocus::Name;
        self.mode = AppMode::NamingSession;
        true
    }

    /// Cycle the open naming popup's launch mode, skipping `Worktree` when the
    /// chosen cwd is not a git repository.
    ///
    /// Validating here rather than at submit time is what preserves the old
    /// behaviour of refusing an impossible worktree up front: the mode simply
    /// cannot be selected, so there is nothing to reject later.
    pub fn cycle_naming_mode(&mut self, forward: bool) {
        let is_repo = self
            .naming_cwd
            .as_deref()
            .map(live::is_git_repo)
            .unwrap_or(false);

        let mut next = self.naming_mode;
        // At most one full lap; `Plain` is always selectable so this terminates.
        for _ in 0..NewSessionMode::ALL.len() {
            next = if forward { next.next() } else { next.prev() };
            if next != NewSessionMode::Worktree || is_repo {
                break;
            }
        }
        self.naming_mode = next;
    }

    /// Request attaching to `tmux_name` once the TUI exits, but only if that
    /// tmux session actually exists.
    ///
    /// Rows can outlive the session they point at: a job keeps its `tmux_name`
    /// after the daemon stops it, and the live list is only re-discovered
    /// periodically. Attaching to a dead session used to abort the process and
    /// close the app, so a missing session is reported in the status bar and
    /// the live list is reconciled instead.
    pub fn request_attach(&mut self, tmux_name: String) {
        if !live::session_exists(self.config.tmux_bin(), &tmux_name) {
            self.status_error = Some(format!("No running session '{tmux_name}' to attach to"));
            self.reload_live_sessions();
            return;
        }
        self.status_error = None;
        self.launch_session = Some(LaunchRequest::AttachLive { tmux_name });
    }

    /// Sync current view settings back into `self.config` and persist it to disk.
    pub(crate) fn save_config(&mut self) {
        self.config.tree_view = self.tree_view;
        self.config.display_mode = self.display_mode;
        self.config.hide_empty = self.hide_empty;
        self.config.group_chains = self.group_chains;
        self.config.live_filter = self.live_filter;
        self.config.source_filter = self.source_filter.as_str().to_string();
        self.config.favorites = self.favorites.clone();
        // claude_path and tmux_path are saved directly on config when edited
        if let Err(e) = self.config.save() {
            self.status_error = Some(format!("Failed to save config: {e}"));
        }
    }
}
