use super::*;
use std::path::{Path, PathBuf};

/// Whether a picker is choosing a directory (e.g. a session's cwd) or a file
/// (e.g. the path to the `claude` binary).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PickerKind {
    /// Only directories are listed and only a directory can be committed.
    Directory,
    /// Directories are listed for navigation; files are listed and selectable.
    File,
}

/// Which field the directory picker is currently choosing a path for. Drives
/// both what kind of path is accepted and where the picker returns on commit
/// or cancel, so one picker serves the new-session flow, the job form, and the
/// config popup's binary-path fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PickerTarget {
    /// Working directory for a new live session (hands off to the naming popup).
    NewSession,
    /// Working directory for the job currently being created or edited.
    JobCwd,
    /// Path to the `claude` binary.
    ConfigClaude,
    /// Path to the `tmux` binary.
    ConfigTmux,
    /// Path to the `claude-usage` binary.
    ConfigUsage,
}

impl PickerTarget {
    /// Whether this target wants a directory or a file.
    pub fn kind(self) -> PickerKind {
        match self {
            PickerTarget::NewSession | PickerTarget::JobCwd => PickerKind::Directory,
            PickerTarget::ConfigClaude | PickerTarget::ConfigTmux | PickerTarget::ConfigUsage => {
                PickerKind::File
            }
        }
    }

    /// Title shown in the picker's path box.
    pub fn title(self) -> &'static str {
        match self {
            PickerTarget::NewSession => " New Session \u{2014} Directory ",
            PickerTarget::JobCwd => " Job \u{2014} Working Directory ",
            PickerTarget::ConfigClaude => " Config \u{2014} claude Binary ",
            PickerTarget::ConfigTmux => " Config \u{2014} tmux Binary ",
            PickerTarget::ConfigUsage => " Config \u{2014} claude-usage Binary ",
        }
    }

    /// The mode the picker returns to when it closes.
    fn origin_mode(self) -> AppMode {
        match self {
            PickerTarget::NewSession => AppMode::Normal,
            PickerTarget::JobCwd => AppMode::JobForm,
            PickerTarget::ConfigClaude | PickerTarget::ConfigTmux | PickerTarget::ConfigUsage => {
                AppMode::Config
            }
        }
    }
}

/// A single entry shown in the directory-picker list.
pub struct DirEntryItem {
    /// Entry name, or `".."` for the parent-directory entry.
    pub name: String,
    /// True for directories, false for files (files only appear in `PickerKind::File`).
    pub is_dir: bool,
}

/// State for the directory-picker modal (`AppMode::DirPicker`), used to choose
/// a working directory or a binary path.
///
/// This deviates from the flat-per-modal-field convention used elsewhere on
/// `App`: the picker's state (current directory, entries, scroll position,
/// path input) is cohesive enough that a single owned struct is clearer than
/// half a dozen loose fields, while `AppMode::DirPicker` itself stays
/// data-free, matching the existing `AppMode` pattern.
pub struct DirBrowser {
    /// Directory currently being browsed.
    pub current_dir: PathBuf,
    /// Entries of `current_dir` (plus a leading `".."` unless at the filesystem root).
    pub entries: Vec<DirEntryItem>,
    /// Index of the currently highlighted entry in `entries`.
    pub selected: usize,
    /// Vertical scroll offset for the entry list, kept in sync with `selected` at render time.
    pub scroll: usize,
    /// Input state for the "type a path" mode, activated with `/`.
    pub path_input: Input,
    /// True while the path input is focused for editing.
    pub input_active: bool,
    /// Last error (e.g. permission denied, or an invalid typed path) shown in the popup.
    pub error: Option<String>,
    /// Whether this browser is picking a directory or a file.
    pub kind: PickerKind,
}

impl DirBrowser {
    /// Construct a directory-picking browser rooted at `start_dir`.
    pub fn new(start_dir: PathBuf) -> Self {
        Self::with_kind(start_dir, PickerKind::Directory)
    }

    /// Construct a browser rooted at `start_dir` for the given kind, loading its
    /// entries immediately.
    pub fn with_kind(start_dir: PathBuf, kind: PickerKind) -> Self {
        let mut browser = Self {
            current_dir: start_dir,
            entries: Vec::new(),
            selected: 0,
            scroll: 0,
            path_input: Input::default(),
            input_active: false,
            error: None,
            kind,
        };
        browser.refresh();
        browser
    }

    /// Reload `entries` from `current_dir`: directories always (hidden ones
    /// included) and files too in `PickerKind::File`, case-insensitively sorted
    /// with directories first and a leading `".."` (unless already at the
    /// filesystem root). On an unreadable directory, sets `error` and leaves the
    /// previous entries in place rather than panicking or clearing the view.
    pub fn refresh(&mut self) {
        let read_dir = match std::fs::read_dir(&self.current_dir) {
            Ok(rd) => rd,
            Err(e) => {
                self.error = Some(format!("Cannot read directory: {e}"));
                return;
            }
        };

        let mut dirs: Vec<String> = Vec::new();
        let mut files: Vec<String> = Vec::new();
        for entry in read_dir.flatten() {
            let is_dir = entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false);
            let Some(name) = entry.file_name().to_str().map(|s| s.to_string()) else {
                continue;
            };
            if is_dir {
                dirs.push(name);
            } else if self.kind == PickerKind::File {
                files.push(name);
            }
        }
        dirs.sort_by_key(|n| n.to_lowercase());
        files.sort_by_key(|n| n.to_lowercase());

        let mut entries = Vec::with_capacity(dirs.len() + files.len() + 1);
        if self.current_dir.parent().is_some() {
            entries.push(DirEntryItem { name: "..".to_string(), is_dir: true });
        }
        entries.extend(dirs.into_iter().map(|name| DirEntryItem { name, is_dir: true }));
        entries.extend(files.into_iter().map(|name| DirEntryItem { name, is_dir: false }));

        self.entries = entries;
        self.error = None;
        self.selected = 0;
        self.scroll = 0;
    }

    /// Descend into the selected entry, or ascend via `go_up` when `".."` is
    /// selected. Returns the selected file's path when the highlighted entry is
    /// a file, since a file cannot be descended into and instead completes the pick.
    pub fn enter_selected(&mut self) -> Option<PathBuf> {
        let Some(entry) = self.entries.get(self.selected) else {
            return None;
        };
        if entry.name == ".." {
            self.go_up();
            return None;
        }
        if !entry.is_dir {
            return Some(self.current_dir.join(&entry.name));
        }
        self.current_dir = self.current_dir.join(&entry.name);
        self.refresh();
        None
    }

    /// Move up to the parent directory, if any, and refresh.
    pub fn go_up(&mut self) {
        if let Some(parent) = self.current_dir.parent() {
            self.current_dir = parent.to_path_buf();
            self.refresh();
        }
    }

    /// The path that would be chosen if the user selects right now: the browsed
    /// directory in `Directory` kind, or the highlighted file in `File` kind
    /// (`None` when a directory is highlighted, since it is not a valid pick).
    pub fn selected_path(&self) -> Option<PathBuf> {
        match self.kind {
            PickerKind::Directory => Some(self.current_dir.clone()),
            PickerKind::File => {
                let entry = self.entries.get(self.selected)?;
                if entry.is_dir {
                    None
                } else {
                    Some(self.current_dir.join(&entry.name))
                }
            }
        }
    }

    /// Commit the text in `path_input` as a navigation target: expand a leading
    /// `~`, and if the resulting path exists and is a directory, navigate there
    /// and close the input; otherwise set `error` and leave the input open for
    /// correction.
    pub fn apply_typed_path(&mut self) {
        let raw = self.path_input.value().trim().to_string();
        if raw.is_empty() {
            self.error = Some("Path is empty".to_string());
            return;
        }
        let expanded = expand_path(&raw);

        if expanded.is_dir() {
            self.current_dir = expanded;
            self.input_active = false;
            self.path_input = Input::default();
            self.refresh();
        } else {
            self.error = Some(format!("Not a directory: {}", expanded.display()));
        }
    }

    /// Resolve the typed path without touching browser state, for callers that
    /// need to decide between committing it and navigating to it.
    pub fn resolve_typed_path(&self) -> Option<PathBuf> {
        let raw = self.path_input.value().trim();
        if raw.is_empty() {
            None
        } else {
            Some(expand_path(raw))
        }
    }
}

/// Expand a leading `~` to the user's home directory. Any other path is used verbatim.
pub fn expand_path(raw: &str) -> PathBuf {
    if let Some(rest) = raw.strip_prefix('~') {
        if let Some(home) = dirs::home_dir() {
            let rest = rest.strip_prefix('/').unwrap_or(rest);
            return if rest.is_empty() { home } else { home.join(rest) };
        }
    }
    PathBuf::from(raw)
}

impl App {
    /// Open the directory-picker modal for a new session's cwd (bound to `b` in
    /// Normal mode), starting at the currently selected session's cwd if there
    /// is one, else the process cwd.
    pub fn open_dir_picker(&mut self) {
        let start_dir = self
            .selected_cwd()
            .map(PathBuf::from)
            .filter(|p| p.is_dir())
            .or_else(|| std::env::current_dir().ok())
            .unwrap_or_else(|| PathBuf::from("."));
        self.dir_picker_target = PickerTarget::NewSession;
        self.dir_browser = Some(DirBrowser::new(start_dir));
        self.mode = AppMode::DirPicker;
    }

    /// Open the picker for `target`, starting from `current` (its existing
    /// value, which may be empty or point at a file) and falling back to the
    /// process cwd. Used by the job form's Directory field and the config
    /// popup's binary-path fields so every path field is browsable.
    pub fn open_path_picker(&mut self, target: PickerTarget, current: &str) {
        let kind = target.kind();
        let start = expand_path(current.trim());
        let start_dir = if current.trim().is_empty() {
            None
        } else if start.is_dir() {
            Some(start.clone())
        } else {
            start.parent().map(Path::to_path_buf).filter(|p| p.is_dir())
        };
        let start_dir = start_dir
            .or_else(|| std::env::current_dir().ok())
            .unwrap_or_else(|| PathBuf::from("."));

        let mut browser = DirBrowser::with_kind(start_dir, kind);
        // Pre-select the existing file so re-opening the picker lands on it.
        if kind == PickerKind::File {
            if let Some(name) = start.file_name().and_then(|n| n.to_str()) {
                if let Some(idx) = browser.entries.iter().position(|e| e.name == name && !e.is_dir) {
                    browser.selected = idx;
                }
            }
        }
        self.dir_picker_target = target;
        self.dir_browser = Some(browser);
        self.mode = AppMode::DirPicker;
    }

    /// Move the directory-picker selection up by one, if possible.
    pub fn dir_picker_move_up(&mut self) {
        if let Some(browser) = self.dir_browser.as_mut() {
            browser.selected = browser.selected.saturating_sub(1);
        }
    }

    /// Move the directory-picker selection down by one, if possible.
    pub fn dir_picker_move_down(&mut self) {
        if let Some(browser) = self.dir_browser.as_mut() {
            if browser.selected + 1 < browser.entries.len() {
                browser.selected += 1;
            }
        }
    }

    /// Descend into (or ascend from, for `".."`) the currently selected entry.
    /// Selecting a file in a file picker commits it instead.
    pub fn dir_picker_enter(&mut self) {
        let picked = self.dir_browser.as_mut().and_then(|b| b.enter_selected());
        if let Some(path) = picked {
            self.dir_picker_commit(path.to_string_lossy().to_string());
        }
    }

    /// Commit the current selection: the browsed directory for a directory
    /// picker, or the highlighted file for a file picker.
    pub fn dir_picker_select(&mut self) {
        let Some(path) = self.dir_browser.as_ref().and_then(|b| b.selected_path()) else {
            if let Some(browser) = self.dir_browser.as_mut() {
                browser.error = Some("Select a file, or press Enter to open a directory".to_string());
            }
            return;
        };
        self.dir_picker_commit(path.to_string_lossy().to_string());
    }

    /// Route a chosen path back to whatever field opened the picker, close the
    /// picker, and return to the originating mode.
    pub fn dir_picker_commit(&mut self, path: String) {
        let target = self.dir_picker_target;
        self.dir_browser = None;
        match target {
            PickerTarget::NewSession => {
                self.naming_placeholder = live::generate_auto_name(&path, &self.live_sessions);
                self.naming_cwd = Some(path);
                self.naming_input = Input::default();
                self.mode = AppMode::NamingSession;
            }
            PickerTarget::JobCwd => {
                self.job_form_cwd = path;
                self.mode = AppMode::JobForm;
            }
            PickerTarget::ConfigClaude => {
                self.config.claude_path = Some(path);
                self.commit_config_path_change();
            }
            PickerTarget::ConfigTmux => {
                self.config.tmux_path = Some(path);
                self.commit_config_path_change();
            }
            PickerTarget::ConfigUsage => {
                self.config.usage_path = Some(path);
                self.commit_config_path_change();
            }
        }
    }

    /// Persist a binary-path change made from the picker, re-check availability,
    /// and return to the config popup.
    fn commit_config_path_change(&mut self) {
        if let Err(e) = self.config.save() {
            self.status_error = Some(format!("Failed to save config: {e}"));
        }
        self.missing_claude = !Config::is_bin_available(self.config.claude_bin());
        self.missing_tmux = !Config::is_bin_available(self.config.tmux_bin());
        self.missing_usage = !Config::is_bin_available(self.config.usage_bin());
        self.mode = AppMode::Config;
    }

    /// Activate the "type a path" input, pre-filled with the current directory.
    pub fn dir_picker_activate_input(&mut self) {
        if let Some(browser) = self.dir_browser.as_mut() {
            let prefill = match browser.kind {
                PickerKind::Directory => browser.current_dir.to_string_lossy().to_string(),
                PickerKind::File => browser
                    .selected_path()
                    .unwrap_or_else(|| browser.current_dir.clone())
                    .to_string_lossy()
                    .to_string(),
            };
            browser.path_input = Input::from(prefill);
            browser.input_active = true;
            browser.error = None;
        }
    }

    /// Commit the typed path (Enter while `input_active`). A path that already
    /// matches what the picker wants is committed directly, so manual entry is
    /// a complete alternative to browsing; a directory typed into a file picker
    /// navigates there instead.
    pub fn dir_picker_commit_input(&mut self) {
        let Some((kind, resolved)) = self
            .dir_browser
            .as_ref()
            .map(|b| (b.kind, b.resolve_typed_path()))
        else {
            return;
        };
        let Some(path) = resolved else {
            if let Some(browser) = self.dir_browser.as_mut() {
                browser.error = Some("Path is empty".to_string());
            }
            return;
        };

        let commit = match kind {
            PickerKind::Directory => path.is_dir(),
            PickerKind::File => path.is_file(),
        };
        if commit {
            self.dir_picker_commit(path.to_string_lossy().to_string());
            return;
        }

        if let Some(browser) = self.dir_browser.as_mut() {
            if kind == PickerKind::File && path.is_dir() {
                browser.apply_typed_path();
            } else if kind == PickerKind::File {
                browser.error = Some(format!("Not a file: {}", path.display()));
            } else {
                browser.error = Some(format!("Not a directory: {}", path.display()));
            }
        }
    }

    /// Handle Esc: close just the path input if active, otherwise close the
    /// whole picker and return to whichever mode opened it.
    pub fn dir_picker_escape(&mut self) {
        let target = self.dir_picker_target;
        let Some(browser) = self.dir_browser.as_mut() else {
            self.mode = target.origin_mode();
            return;
        };
        if browser.input_active {
            browser.input_active = false;
            browser.path_input = Input::default();
            browser.error = None;
        } else {
            self.dir_browser = None;
            self.mode = target.origin_mode();
        }
    }
}
