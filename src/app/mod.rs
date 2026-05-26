mod activity;
mod chain;
mod display;
mod filter;
mod flat;
mod preview;
mod selection;
mod tree;

#[cfg(test)]
mod tests;

use crate::config::{Config, DisplayMode};
use crate::data::{self, CcsmOrigin, CcsmSessionRecord, PreviewMessage, SessionInfo, SessionMeta};
use crate::live::{self, ActivityState, LiveSession};
use crate::update;
use std::collections::{HashMap, HashSet};
use std::time::Instant;
use tui_input::Input;

/// Maximum number of `reload_sessions()` cycles a pending launch waits to be
/// matched against a real Claude session before it is dropped. Sessions that
/// never wrote any history (e.g. the user exited immediately) shouldn't keep
/// piling up.
const PENDING_LAUNCH_MAX_ATTEMPTS: u8 = 2;

/// Higher attempt cap for `AttachLive` launches. The tmux session we attached
/// to may run an idle Claude session that only becomes visible in Claude's
/// history once the user types something in it, which can take many reload
/// cycles; we still cap eventually so a missing/non-Claude tmux doesn't leak.
const PENDING_LAUNCH_ATTACH_LIVE_MAX_ATTEMPTS: u8 = 20;

/// Time window (milliseconds) used to match a CCSM launch without a known
/// session_id (NewLive/NewDirect/AttachLive) against a freshly observed Claude
/// session. Generous because clock skew and slow startup both eat into it.
const PENDING_LAUNCH_MATCH_WINDOW_MS: i64 = 60_000;

/// A session launch we've issued and want to record into CCSM's history once
/// we can identify the resulting Claude session_id.
#[derive(Debug, Clone)]
pub struct PendingLaunch {
    pub origin: CcsmOrigin,
    pub cwd: Option<String>,
    pub launched_at: i64,
    pub known_session_id: Option<String>,
    pub attempts: u8,
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

/// Describes how to launch or attach to a Claude session after the TUI exits.
#[derive(Debug, Clone)]
pub enum LaunchRequest {
    /// Resume a historical session inside a new tmux live session.
    Resume { session_id: String, cwd: String },
    /// Resume a historical session directly in the foreground (no tmux).
    Direct { session_id: String, cwd: String },
    /// Attach the terminal to an already-running live tmux session.
    AttachLive { tmux_name: String },
    /// Create and attach to a new live tmux session running claude.
    NewLive { name: String, cwd: String },
    /// Create and attach to a new live tmux session running claude with --dangerously-skip-permissions.
    NewLiveDangerous { name: String, cwd: String },
    /// Start a new claude session directly in the foreground (no tmux).
    NewDirect { cwd: String },
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
    /// A duplicate session name was entered; waiting for the user to choose open vs. rename.
    DuplicateSession,
    /// The config popup is open.
    Config,
    /// One or more required binaries (claude/tmux) are missing.
    MissingDeps,
}

/// Tracks which popup triggered the duplicate-name check, so we can return to the right mode.
#[derive(Debug, Clone, PartialEq)]
pub enum DuplicateSource {
    /// Duplicate detected while naming a new live session.
    NamingSession,
    /// Duplicate detected while renaming an existing live session.
    Renaming,
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
    pub names_receiver: Option<std::sync::mpsc::Receiver<HashMap<String, String>>>,
    /// Set to true when the process should exec-restart itself after an update.
    pub should_restart: bool,
    /// Set to true whenever state changes require the screen to be redrawn.
    pub needs_redraw: bool,
    /// All currently running live tmux sessions on the ccsm socket.
    pub live_sessions: Vec<LiveSession>,
    /// When true, only projects with active live sessions are shown.
    pub live_filter: bool,
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
    /// Currently selected row in the config popup (0..=CONFIG_MAX_ROW).
    pub config_selected: usize,
    /// True when the `claude` binary cannot be found at startup.
    pub missing_claude: bool,
    /// True when the `tmux` binary cannot be found at startup.
    pub missing_tmux: bool,
    /// True when editing a text field in the config popup (path fields).
    pub config_editing: bool,
    /// Input state for the path text field in the config popup.
    pub config_path_input: Input,
    /// Per-session activity state (Active, Idle, Unknown).
    pub activity_states: HashMap<String, ActivityState>,
    /// Per-session timestamp of last activity poll, for throttling.
    pub activity_last_poll: HashMap<String, Instant>,
    /// Monotonic tick counter, incremented each redraw to drive pulse animation.
    pub tick: u64,
    /// When true, the naming popup is for a --dangerously-skip-permissions session.
    pub naming_dangerous: bool,
    /// Last error message to display in the status bar.
    pub status_error: Option<String>,
    /// Session IDs CCSM has previously recorded into its own history file.
    /// Used to decide which sessions to re-snapshot on each reload and which
    /// orphaned (Claude-cleaned-up) sessions belong to CCSM.
    pub ccsm_owned_ids: HashSet<String>,
    /// Launches CCSM has issued that haven't yet been matched to a Claude
    /// session in the history. NewLive/NewDirect/AttachLive start without a
    /// known session_id and get resolved by cwd + launched_at on a later reload.
    pub pending_launches: Vec<PendingLaunch>,
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
        let favorites = config.favorites.clone();
        let live_sessions = live::discover_live_sessions(config.tmux_bin());
        let ccsm_owned_ids: HashSet<String> = sessions
            .iter()
            .filter(|s| s.ccsm_owned)
            .map(|s| s.session_id.clone())
            .collect();
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
            missing_tmux: false,
            config_editing: false,
            config_path_input: Input::default(),
            activity_states: HashMap::new(),
            activity_last_poll: HashMap::new(),
            tick: 0,
            naming_dangerous: false,
            status_error: None,
            ccsm_owned_ids,
            pending_launches: Vec::new(),
        };

        // Check for required binaries
        let claude_ok = Config::is_bin_available(app.config.claude_bin());
        let tmux_ok = Config::is_bin_available(app.config.tmux_bin());
        if !claude_ok || !tmux_ok {
            app.missing_claude = !claude_ok;
            app.missing_tmux = !tmux_ok;
            app.mode = AppMode::MissingDeps;
        }

        app.spawn_load_session_names();
        app.init_tree();
        app.recompute_filter();
        app
    }

    /// Spawn a background thread that loads custom titles for all sessions with data.
    pub fn spawn_load_session_names(&mut self) {
        let sessions: Vec<(String, String)> = self
            .sessions
            .iter()
            .filter(|s| s.has_data)
            .map(|s| (s.project.clone(), s.session_id.clone()))
            .collect();

        let (tx, rx) = std::sync::mpsc::channel();
        self.names_receiver = Some(rx);

        std::thread::spawn(move || {
            let mut names = HashMap::new();
            for (project, session_id) in sessions {
                if let Some(title) = data::load_custom_title(&project, &session_id) {
                    names.insert(session_id, title);
                }
            }
            let _ = tx.send(names);
        });
    }

    /// Apply custom titles received from the background loader, then refresh all views.
    pub fn apply_session_names(&mut self, names: HashMap<String, String>) {
        for session in &mut self.sessions {
            if let Some(title) = names.get(&session.session_id) {
                session.name = Some(title.clone());
            }
        }

        self.preview_cache.clear();
        self.recompute_tree();
        self.recompute_flat_rows();
    }

    /// Replace the session list with a freshly loaded set, reset caches, and rebuild all views.
    pub fn reload_sessions(&mut self, sessions: Vec<SessionInfo>) {
        self.sessions = sessions;
        self.reconcile_pending_launches_and_snapshot();
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

    /// Stage a launch for the main loop to execute after the terminal is torn
    /// down, and queue a pending entry so the resulting session gets snapshotted
    /// into CCSM's history on the next reload.
    pub fn issue_launch(&mut self, req: LaunchRequest) {
        self.record_launch(&req);
        self.launch_session = Some(req);
    }

    /// Record that we're about to issue this launch so the next reload can
    /// snapshot the resulting session into CCSM's own history. Resume/Direct
    /// already know the session_id; new/attach launches have to be matched on
    /// cwd + timestamp once the session shows up in Claude's history.
    pub fn record_launch(&mut self, req: &LaunchRequest) {
        let launched_at = chrono::Utc::now().timestamp_millis();
        let (origin, cwd, known_session_id) = match req {
            LaunchRequest::Resume { session_id, cwd } => {
                (CcsmOrigin::Resume, Some(cwd.clone()), Some(session_id.clone()))
            }
            LaunchRequest::Direct { session_id, cwd } => {
                (CcsmOrigin::Direct, Some(cwd.clone()), Some(session_id.clone()))
            }
            LaunchRequest::AttachLive { tmux_name } => {
                let cwd = self
                    .live_sessions
                    .iter()
                    .find(|ls| ls.tmux_name == *tmux_name)
                    .map(|ls| ls.cwd.clone());
                (CcsmOrigin::AttachLive, cwd, None)
            }
            LaunchRequest::NewLive { cwd, .. } => {
                (CcsmOrigin::NewLive, Some(cwd.clone()), None)
            }
            LaunchRequest::NewLiveDangerous { cwd, .. } => {
                (CcsmOrigin::NewLiveDangerous, Some(cwd.clone()), None)
            }
            LaunchRequest::NewDirect { cwd } => {
                (CcsmOrigin::NewDirect, Some(cwd.clone()), None)
            }
        };
        self.pending_launches.push(PendingLaunch {
            origin,
            cwd,
            launched_at,
            known_session_id,
            attempts: 0,
        });
    }

    /// Match outstanding launches to freshly loaded Claude sessions, snapshot
    /// every CCSM-owned session whose state has changed since the last write,
    /// and flag those sessions in `self.sessions` so downstream code knows
    /// they're backed by a CCSM record.
    fn reconcile_pending_launches_and_snapshot(&mut self) {
        let stored = data::load_ccsm_records();

        let pending = std::mem::take(&mut self.pending_launches);
        let mut still_pending: Vec<PendingLaunch> = Vec::new();
        let mut to_snapshot: Vec<(usize, CcsmOrigin, i64)> = Vec::new();
        let mut matched_ids: HashSet<String> = HashSet::new();

        for mut launch in pending {
            let matched_idx = if let Some(ref id) = launch.known_session_id {
                self.sessions.iter().position(|s| &s.session_id == id)
            } else {
                let cwd = match &launch.cwd {
                    Some(c) => c.clone(),
                    None => {
                        // Nothing to match on; drop.
                        continue;
                    }
                };
                let window_start = launch.launched_at - PENDING_LAUNCH_MATCH_WINDOW_MS;
                // AttachLive targets a session that was created before we
                // attached, so its first_timestamp is in the past and its
                // last_timestamp may be too if the user attached and detached
                // without typing. Match on cwd alone for AttachLive; require a
                // recent first_timestamp for brand-new sessions; require any
                // recent activity (last_timestamp) for the others.
                let is_attach_live = launch.origin == CcsmOrigin::AttachLive;
                self.sessions
                    .iter()
                    .enumerate()
                    .filter(|(_, s)| s.project == cwd)
                    .filter(|(_, s)| !matched_ids.contains(&s.session_id))
                    .filter(|(_, s)| {
                        if is_attach_live {
                            return true;
                        }
                        if s.last_timestamp < window_start {
                            return false;
                        }
                        match launch.origin {
                            CcsmOrigin::NewLive
                            | CcsmOrigin::NewLiveDangerous
                            | CcsmOrigin::NewDirect => s.first_timestamp >= window_start,
                            _ => true,
                        }
                    })
                    .max_by_key(|(_, s)| s.last_timestamp)
                    .map(|(i, _)| i)
            };

            match matched_idx {
                Some(idx) => {
                    matched_ids.insert(self.sessions[idx].session_id.clone());
                    to_snapshot.push((idx, launch.origin, launch.launched_at));
                }
                None => {
                    launch.attempts += 1;
                    // AttachLive may match an idle session that only shows up
                    // after the user does something in it, so we let it keep
                    // retrying across many reload cycles.
                    let cap = if launch.origin == CcsmOrigin::AttachLive {
                        PENDING_LAUNCH_ATTACH_LIVE_MAX_ATTEMPTS
                    } else {
                        PENDING_LAUNCH_MAX_ATTEMPTS
                    };
                    if launch.attempts < cap {
                        still_pending.push(launch);
                    }
                }
            }
        }
        self.pending_launches = still_pending;

        // Snapshot any session that needs writing: just-matched launches and
        // existing owned sessions whose signature changed.
        let mut snapshot_now: HashMap<usize, (CcsmOrigin, i64)> = HashMap::new();
        for (idx, origin, launched_at) in to_snapshot {
            snapshot_now.insert(idx, (origin, launched_at));
        }
        for (idx, session) in self.sessions.iter().enumerate() {
            if !session.ccsm_owned && !self.ccsm_owned_ids.contains(&session.session_id) {
                continue;
            }
            if snapshot_now.contains_key(&idx) {
                continue;
            }
            // Skip if the on-disk record's signature already matches.
            if let Some(rec) = stored.get(&session.session_id) {
                if rec.last_timestamp == session.last_timestamp
                    && rec.entry_count == session.entry_count
                    && rec.name == session.name
                {
                    continue;
                }
                snapshot_now.insert(idx, (rec.ccsm_origin, rec.ccsm_launched_at));
            }
        }

        for (idx, (origin, launched_at)) in snapshot_now {
            let session = &self.sessions[idx];
            let (meta, messages) = data::load_preview(&session.project, &session.session_id);
            let record = CcsmSessionRecord {
                session_id: session.session_id.clone(),
                project: session.project.clone(),
                project_name: session.project_name.clone(),
                first_timestamp: session.first_timestamp,
                last_timestamp: session.last_timestamp,
                entry_count: session.entry_count,
                name: session.name.clone(),
                slug: session.slug.clone(),
                cwd: meta.cwd,
                git_branch: meta.git_branch,
                ccsm_launched_at: launched_at,
                ccsm_origin: origin,
                preview_messages: messages,
                preview_cached_at: chrono::Utc::now().timestamp_millis(),
            };
            if let Err(e) = data::append_ccsm_record(&record) {
                self.status_error = Some(format!("Failed to write ccsm history: {e}"));
            } else {
                self.ccsm_owned_ids.insert(record.session_id.clone());
                self.sessions[idx].ccsm_owned = true;
            }
        }
    }

    /// Sync current view settings back into `self.config` and persist it to disk.
    pub(crate) fn save_config(&mut self) {
        self.config.tree_view = self.tree_view;
        self.config.display_mode = self.display_mode;
        self.config.hide_empty = self.hide_empty;
        self.config.group_chains = self.group_chains;
        self.config.live_filter = self.live_filter;
        self.config.favorites = self.favorites.clone();
        // claude_path and tmux_path are saved directly on config when edited
        if let Err(e) = self.config.save() {
            self.status_error = Some(format!("Failed to save config: {e}"));
        }
    }
}
