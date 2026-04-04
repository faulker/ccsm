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
use crate::data::{self, PreviewMessage, SessionInfo, SessionMeta};
use crate::live::{self, ActivityState, LiveSession};
use crate::update;
use std::collections::{HashMap, HashSet};
use std::time::Instant;
use tui_input::Input;

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
