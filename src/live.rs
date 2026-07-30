// Manages the dedicated ccsm tmux server and live sessions

use anyhow::Context;
use regex::Regex;
use std::io::Write;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::LazyLock;
use std::time::{Duration, Instant};

use crate::data::AgentBackend;

pub const TMUX_SOCKET: &str = "ccsm";

/// Parent-process env vars that poison a nested Cursor Agent launch when ccsm
/// itself is started from inside another `agent` session.
const CURSOR_PARENT_ENV_VARS: &[&str] = &[
    "CURSOR_AGENT",
    "CURSOR_CONVERSATION_ID",
    "CURSOR_INVOKED_AS",
    "CURSOR_SESSION_ID",
    "CURSOR_ASKPASS_SOCKET",
    "CURSOR_ASKPASS_SECRET",
    "SUDO_ASKPASS",
];

/// How long to watch a freshly started Cursor live session for an immediate
/// exit before attaching. Interactive `agent --resume` on 2026.07.23 dies
/// within about a second after loading the conversation.
pub const CURSOR_STARTUP_WATCH: Duration = Duration::from_millis(1200);

const CURSOR_STARTUP_POLL: Duration = Duration::from_millis(100);

/// The tmux session name reserved for the ccsm scheduler daemon. Filtered out
/// of `discover_live_sessions` so it never appears in the user's session list
/// and never gets activity-polled against its own log output.
pub const WATCH_SESSION: &str = "ccsm-watch";

/// A running tmux session managed by ccsm on the dedicated `ccsm` tmux socket.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveSession {
    /// The tmux session name used to target it in tmux commands.
    pub tmux_name: String,
    /// The name shown in the UI (same as `tmux_name` unless renamed).
    pub display_name: String,
    /// Working directory of the tmux session (from `#{session_path}`).
    pub cwd: String,
    /// Base name of the working directory, used as a short project label.
    pub project_name: String,
    /// Scheduler job id tagged on this session via `set_job_tag`, if any.
    /// Survives `rename-session`, unlike matching on `tmux_name`.
    pub job_id: Option<String>,
    /// Agent backend tagged via `set_backend_tag` when the session was started.
    /// `None` for sessions created before tagging existed (or untagged spawns).
    pub backend: Option<AgentBackend>,
}

/// Returns true if the ccsm tmux server is currently running (i.e. `tmux -L ccsm list-sessions` succeeds).
pub fn is_server_running(tmux: &str) -> bool {
    std::process::Command::new(tmux)
        .args(["-L", TMUX_SOCKET, "list-sessions"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Query the ccsm tmux server for all running sessions and return them as `LiveSession` values.
/// Returns an empty vec if the server is not running or the command fails.
pub fn discover_live_sessions(tmux: &str) -> Vec<LiveSession> {
    if !is_server_running(tmux) {
        return vec![];
    }
    let output = std::process::Command::new(tmux)
        .args([
            "-L",
            TMUX_SOCKET,
            "list-sessions",
            "-F",
            "#{session_name}\t#{session_path}\t#{@ccsm_job}\t#{@ccsm_backend}\t#{pane_start_command}",
        ])
        .output();
    let output = match output {
        Ok(o) if o.status.success() => o,
        _ => return vec![],
    };
    let text = String::from_utf8_lossy(&output.stdout);
    parse_session_lines(&text)
}

/// Parse list-sessions output into `LiveSession` values.
///
/// Columns: `name`, `path`, `@ccsm_job`, `@ccsm_backend`, `pane_start_command`.
/// Trailing columns may be missing. An explicit `@ccsm_backend` tag wins; when
/// it is unset (sessions started before tagging), fall back to inferring from
/// the pane's start command so the preview info bar still names the agent.
/// Excludes `WATCH_SESSION`.
fn parse_session_lines(text: &str) -> Vec<LiveSession> {
    text.lines()
        .filter_map(|line| {
            let mut parts = line.splitn(5, '\t');
            let name = parts.next()?.to_string();
            let path = parts.next()?.to_string();
            if name == WATCH_SESSION {
                return None;
            }
            let job_id = parts.next().and_then(|tag| {
                if tag.is_empty() {
                    None
                } else {
                    Some(tag.to_string())
                }
            });
            let tagged = parts
                .next()
                .and_then(|tag| AgentBackend::from_tmux_tag(tag));
            let start_cmd = parts.next().unwrap_or("");
            let backend = tagged.or_else(|| infer_backend_from_start_command(start_cmd));
            let project_name = std::path::Path::new(&path)
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "session".to_string());
            Some(LiveSession {
                display_name: name.clone(),
                tmux_name: name,
                cwd: path,
                project_name,
                job_id,
                backend,
            })
        })
        .collect()
}

/// Infer Claude vs Cursor from the pane's start command when `@ccsm_backend`
/// was never set. Matches the executable basename only (`claude`, `agent`).
/// Skips a leading `env -u VAR …` wrapper used to scrub parent Cursor env.
fn infer_backend_from_start_command(cmd: &str) -> Option<AgentBackend> {
    let tokens: Vec<&str> = cmd.split_whitespace().collect();
    let mut i = 0;
    if tokens.first().copied() == Some("env") {
        i = 1;
        while i + 1 < tokens.len() && tokens[i] == "-u" {
            i += 2;
        }
    }
    let first = tokens.get(i)?;
    let base = std::path::Path::new(first)
        .file_name()
        .and_then(|n| n.to_str())?;
    if base == "agent" {
        Some(AgentBackend::CursorAgent)
    } else if base == "claude" {
        Some(AgentBackend::ClaudeCode)
    } else {
        None
    }
}

/// Returns the path to the ccsm tmux configuration file (`~/.config/ccsm/tmux.conf`).
pub fn conf_path() -> Option<std::path::PathBuf> {
    dirs::home_dir().map(|h| h.join(".config").join("ccsm").join("tmux.conf"))
}

/// Write the ccsm tmux config file and, if the server is already running, source it to apply changes.
pub fn ensure_server_configured(tmux: &str) -> anyhow::Result<()> {
    let conf_path = conf_path()
        .ok_or_else(|| anyhow::anyhow!("Could not determine home directory for config path"))?;
    if let Some(parent) = conf_path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create config directory: {}", parent.display()))?;
    }
    // Within tmux double-quoted strings, \\ is a literal backslash.
    // For bind-key in config files, single-quote the key spec to avoid backslash escape issues.
    let exe_path = std::env::current_exe()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|_| "ccsm".to_string());
    let mut conf_content = format!(
        concat!(
            "set -g history-limit 50000\n",
            "set -g mouse on\n",
            "set -g default-terminal \"tmux-256color\"\n",
            "set -g extended-keys on\n",
            "set -g status on\n",
            "set -g status-interval 1\n",
            "set -g status-style \"bg=#1e1e2e,fg=#cdd6f4\"\n",
            "set -g status-format[0] \"#[align=left,bg=#1e1e2e,fg=#cdd6f4]#[align=centre]#{{?pane_in_mode,#[fg=#f38ba8 bold]Hit the ESC key to exit scroll mode}}#[align=right]#[fg=#f38ba8 bold]Ctrl+\\\\ #[fg=#a6adc8]detach  #[fg=#f38ba8 bold]Ctrl+l #[fg=#a6adc8]new  #[fg=#f38ba8 bold]Ctrl+n #[fg=#a6adc8]next  #[fg=#f38ba8 bold]Ctrl+p #[fg=#a6adc8]prev \"\n",
            "unbind-key -q -n C-]\n",
            "unbind-key -q -n C-[\n",
            "bind-key -n 'C-\\' detach-client\n",
            "bind-key -n C-l run-shell 'cd \"#{{pane_current_path}}\" && \"{}\" --spawn'\n",
            "bind-key -n C-n switch-client -n\n",
            "bind-key -n C-p switch-client -p\n",
        ),
        exe_path,
    );

    // When running inside Ghostty, bind Shift+Enter to send ESC + Enter (\x1b\r).
    // Ghostty supports the kitty keyboard protocol, so tmux (with extended-keys on)
    // receives Shift+Enter as S-Enter; we forward it as the escape sequence that
    // Claude interprets as "new line without submitting".
    if std::env::var("TERM_PROGRAM").ok().as_deref() == Some("ghostty") {
        conf_content.push_str("bind-key -n S-Enter send-keys Escape Enter\n");
    }

    std::fs::write(&conf_path, &conf_content)
        .with_context(|| format!("Failed to write tmux config: {}", conf_path.display()))?;
    // If the server is already running, source the config to update bindings.
    // If not running, start-server is unreliable on tmux 3.x — the server is started
    // implicitly when new-session runs (see start_live_session which passes -f).
    // Sourcing failure is non-fatal.
    let _ = std::process::Command::new(tmux)
        .args([
            "-L",
            TMUX_SOCKET,
            "source-file",
            &conf_path.to_string_lossy(),
        ])
        .output();
    Ok(())
}

/// Returns true if the configured tmux binary is installed and reachable.
pub fn is_tmux_available(tmux: &str) -> bool {
    std::process::Command::new(tmux)
        .arg("-V")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Prefix `cmd` with `env -u …` so a child Cursor Agent does not inherit the
/// parent chat's identity when ccsm was launched from inside another agent.
pub fn with_cleared_cursor_env(cmd: &[&str]) -> Vec<String> {
    let mut out = Vec::with_capacity(2 + CURSOR_PARENT_ENV_VARS.len() * 2 + cmd.len());
    out.push("env".to_string());
    for var in CURSOR_PARENT_ENV_VARS {
        out.push("-u".to_string());
        out.push((*var).to_string());
    }
    out.extend(cmd.iter().map(|s| (*s).to_string()));
    out
}

/// Remove Cursor parent-session env vars from a foreground `Command`.
pub fn clear_cursor_parent_env(cmd: &mut Command) {
    for var in CURSOR_PARENT_ENV_VARS {
        cmd.env_remove(var);
    }
}

/// Create a new detached tmux session named `name` with working directory `cwd`,
/// running `cmd` as the initial command. Starts the ccsm tmux server if needed.
///
/// When `backend` is `Some`, tags the session with `@ccsm_backend` so the TUI
/// can show which agent is running after a rename or restart.
///
/// Cursor launches are wrapped with [`with_cleared_cursor_env`] so nested
/// agent env (from launching ccsm inside another Cursor session) cannot
/// poison the new pane.
pub fn start_live_session(
    tmux: &str,
    name: &str,
    cwd: &str,
    cmd: &[&str],
    backend: Option<AgentBackend>,
) -> anyhow::Result<()> {
    if !is_tmux_available(tmux) {
        anyhow::bail!("tmux is not installed — live sessions require tmux");
    }
    ensure_server_configured(tmux)?;
    let conf_path_str = conf_path()
        .ok_or_else(|| anyhow::anyhow!("Could not determine home directory for config path"))?
        .to_string_lossy()
        .into_owned();
    let scrubbed = match backend {
        Some(AgentBackend::CursorAgent) => Some(with_cleared_cursor_env(cmd)),
        _ => None,
    };
    // Pass -f so that if the server isn't running yet, it starts with our config.
    // If the server is already running, -f is ignored by tmux.
    let mut cmd_args: Vec<String> = vec![
        "-L".into(),
        TMUX_SOCKET.into(),
        "-f".into(),
        conf_path_str,
        "new-session".into(),
        "-d".into(),
        "-s".into(),
        name.to_string(),
        "-c".into(),
        cwd.to_string(),
    ];
    match &scrubbed {
        Some(argv) => cmd_args.extend(argv.iter().cloned()),
        None => cmd_args.extend(cmd.iter().map(|s| (*s).to_string())),
    }
    let output = std::process::Command::new(tmux).args(&cmd_args).output()?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("Failed to create session '{}': {}", name, stderr.trim());
    }
    if let Some(backend) = backend {
        set_backend_tag(tmux, name, backend)?;
        // Cursor interactive resume can exit within ~1s; keep the dead pane
        // around long enough for ensure_live_pane_stays_up to capture it.
        if backend == AgentBackend::CursorAgent {
            set_remain_on_exit(tmux, name, true);
        }
    }
    Ok(())
}

/// Build argv for `display-message -p -t =name: "#{pane_dead}"`.
fn pane_dead_args(name: &str) -> Vec<String> {
    vec![
        "-L".to_string(),
        TMUX_SOCKET.to_string(),
        "display-message".to_string(),
        "-p".to_string(),
        "-t".to_string(),
        pane_target(name),
        "#{pane_dead}".to_string(),
    ]
}

/// True when the session's active pane has exited (tmux `pane_dead`).
pub fn pane_is_dead(tmux: &str, name: &str) -> bool {
    std::process::Command::new(tmux)
        .args(pane_dead_args(name))
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim() == "1")
        .unwrap_or(false)
}

fn set_remain_on_exit_args(name: &str, on: bool) -> Vec<String> {
    vec![
        "-L".to_string(),
        TMUX_SOCKET.to_string(),
        "set-option".to_string(),
        "-t".to_string(),
        pane_target(name),
        "remain-on-exit".to_string(),
        if on { "on" } else { "off" }.to_string(),
    ]
}

fn set_remain_on_exit(tmux: &str, name: &str, on: bool) {
    let _ = std::process::Command::new(tmux)
        .args(set_remain_on_exit_args(name, on))
        .output();
}

/// Collapse a pane capture into a short, single-line hint for `status_error`.
pub fn summarize_pane_tail(tail: &str) -> String {
    let plain = strip_ansi(tail);
    let lines: Vec<&str> = plain
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    if lines.is_empty() {
        return String::new();
    }
    let take = lines.len().saturating_sub(4);
    let snippet = lines[take..].join(" | ");
    if snippet.chars().count() > 160 {
        let truncated: String = snippet.chars().take(157).collect();
        format!("{truncated}…")
    } else {
        snippet
    }
}

/// Watch a freshly started live session briefly; if the pane dies before
/// attach, capture the tail, kill the session, and return a
/// [`CursorResumeFailure`] suitable for the TUI popover.
///
/// Used for Cursor resume, where the CLI can flash "Loading conversation"
/// then exit with SIGTERM before the user can interact. `_what` labels the
/// call site for logs; the popover owns the user-facing explanation.
pub fn ensure_live_pane_stays_up(
    tmux: &str,
    name: &str,
    _what: &str,
    timeout: Duration,
) -> anyhow::Result<()> {
    set_remain_on_exit(tmux, name, true);
    let deadline = Instant::now() + timeout;
    loop {
        let exists = session_exists(tmux, name);
        let dead = exists && pane_is_dead(tmux, name);
        if !exists || dead {
            let tail = if exists {
                poll_pane_tail(tmux, name, 24)
            } else {
                String::new()
            };
            let snippet = summarize_pane_tail(&tail);
            if exists {
                let _ = stop_live_session(tmux, name);
            }
            return Err(cursor_early_exit_error(&snippet));
        }
        if Instant::now() >= deadline {
            set_remain_on_exit(tmux, name, false);
            return Ok(());
        }
        std::thread::sleep(CURSOR_STARTUP_POLL);
    }
}

/// Typed failure for interactive Cursor resume dying before the user can
/// interact. The TUI downcasts this to open a dedicated popover; other launch
/// errors stay on the status bar.
#[derive(Debug)]
pub struct CursorResumeFailure {
    /// Pane snippet or exit-status detail; may be empty.
    pub detail: String,
}

impl std::fmt::Display for CursorResumeFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.detail.is_empty() {
            write!(f, "Cursor Agent resume exited early")
        } else {
            write!(f, "Cursor Agent resume exited early: {}", self.detail)
        }
    }
}

impl std::error::Error for CursorResumeFailure {}

/// Build a [`CursorResumeFailure`] when a Cursor live session dies before attach.
fn cursor_early_exit_error(snippet: &str) -> anyhow::Error {
    CursorResumeFailure {
        detail: snippet.to_string(),
    }
    .into()
}

/// Attach the current process to the named tmux session on the ccsm socket.
pub fn attach_to_session(tmux: &str, name: &str) -> anyhow::Result<()> {
    if !is_tmux_available(tmux) {
        anyhow::bail!("tmux is not installed — live sessions require tmux");
    }
    ensure_server_configured(tmux)?;
    let status = std::process::Command::new(tmux)
        .args(["-L", TMUX_SOCKET, "attach-session", "-t", name])
        .status()?;
    if !status.success() {
        anyhow::bail!("Failed to attach to session '{}'", name);
    }
    Ok(())
}

/// Switch the current tmux client to the named session on the ccsm socket.
/// Only works when already inside a tmux client (i.e. the `--spawn` use case).
pub fn switch_to_session(tmux: &str, name: &str) -> anyhow::Result<()> {
    if !is_tmux_available(tmux) {
        anyhow::bail!("tmux is not installed — live sessions require tmux");
    }
    let status = std::process::Command::new(tmux)
        .args(["-L", TMUX_SOCKET, "switch-client", "-t", name])
        .status()?;
    if !status.success() {
        anyhow::bail!("Failed to switch to session '{}'", name);
    }
    Ok(())
}

/// Send Ctrl+C to interrupt any running process, then kill the named tmux session.
pub fn stop_live_session(tmux: &str, name: &str) -> anyhow::Result<()> {
    // Send Ctrl+C to interrupt any running process before killing the session
    let _ = std::process::Command::new(tmux)
        .args(["-L", TMUX_SOCKET, "send-keys", "-t", name, "C-c", ""])
        .output();
    let output = std::process::Command::new(tmux)
        .args(["-L", TMUX_SOCKET, "kill-session", "-t", name])
        .output()?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("Failed to stop session '{}': {}", name, stderr.trim());
    }
    Ok(())
}

/// Capture the last `lines` lines from the pane of the named tmux session,
/// preserving ANSI escape sequences. Returns an empty string if the session
/// does not exist or the command fails.
pub fn poll_pane_buffer(tmux: &str, name: &str, lines: usize) -> String {
    let lines_str = format!("-{}", lines);
    let output = std::process::Command::new(tmux)
        .args([
            "-L",
            TMUX_SOCKET,
            "capture-pane",
            "-t",
            name,
            "-p",
            "-e",
            "-S",
            &lines_str,
        ])
        .output();
    match output {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).into_owned(),
        _ => String::new(),
    }
}

/// Activity state of a live session, determined by examining pane content.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActivityState {
    /// Claude is actively working (running a tool or thinking).
    Active,
    /// Claude is idle (waiting for user input or approval).
    Idle,
    /// Claude is waiting for user input on a prompt (e.g., "Do you want to proceed?").
    Waiting,
    /// State not yet determined (session just started or capture failed).
    Unknown,
}

/// Strip ANSI escape sequences from a string for cleaner keyword matching.
pub fn strip_ansi(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            // Skip CSI sequence: ESC [ ... <letter>
            if let Some(next) = chars.next() {
                if next == '[' {
                    // Consume until we hit a letter (ASCII 0x40-0x7E)
                    for c2 in chars.by_ref() {
                        if c2.is_ascii_alphabetic() || c2 == '~' {
                            break;
                        }
                    }
                } else if next == ']' {
                    // OSC sequence: ESC ] ... terminated by BEL or ST (ESC \)
                    while let Some(c2) = chars.next() {
                        if c2 == '\x07' {
                            break;
                        }
                        if c2 == '\x1b' && chars.next() == Some('\\') {
                            break;
                        }
                    }
                }
                // Other sequences: just skip the next char
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Capture only the last `lines` lines from a pane, without ANSI escape codes.
/// Lightweight alternative to `poll_pane_buffer` for non-selected sessions.
pub fn poll_pane_tail(tmux: &str, name: &str, lines: usize) -> String {
    let lines_str = format!("-{}", lines);
    let output = std::process::Command::new(tmux)
        .args([
            "-L",
            TMUX_SOCKET,
            "capture-pane",
            "-t",
            name,
            "-p",
            "-S",
            &lines_str,
        ])
        .output();
    match output {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).into_owned(),
        _ => String::new(),
    }
}

/// Fixed strings that indicate a session is **waiting** for user input.
/// Simple substring matching is sufficient here — no regex needed.
static WAITING_PATTERNS: &[&str] = &[
    "Do you want to proceed?",
    "Enter to select",
    "Yes, clear context",
    "Yes, allow all edits during this session",
];

/// Regex patterns that indicate an **active** (working) session.
/// Add new patterns here to extend detection without changing logic.
static ACTIVE_PATTERNS: &[&LazyLock<Regex>] = &[
    &PATTERN_ACTIVE_TIMER,
    &PATTERN_MORE_TOOL_USES,
    &PATTERN_TIP_INDICATOR,
    &PATTERN_ACTIVE_THOUGHT,
    &PATTERN_ACTIVE_THINKING,
    &PATTERN_ACTIVE_SEARCH_PATTERN,
    &PATTERN_ACTIVE_READING_FILE,
];

/// Matches Claude Code's active timer line using Unicode ellipsis (U+2026):
/// e.g. `Thinking… (10m · 13.0k tokens)`, `Thinking… (1h 35m 22s · 42.3k tokens)`
static PATTERN_ACTIVE_TIMER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\w*\u{2026} \((\d+[smh]\s*)+·.*\d+.*tokens").unwrap());

/// Matches the collapsed tool-use indicator shown while Claude is working:
/// e.g. `+3 more tool uses (ctrl+o to expand)`
static PATTERN_MORE_TOOL_USES: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\+\d more tool uses \(ctrl\+o to expand\)").unwrap());

static PATTERN_TIP_INDICATOR: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"Tip: .*").unwrap());

static PATTERN_ACTIVE_THOUGHT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\(thought for (?:\d+h\s*)?(?:\d+m\s*)?(?:\d+s)?\)").unwrap());

static PATTERN_ACTIVE_THINKING: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\w*\u{2026} \((\d+[smh]\s*)+·.*thinking").unwrap());

static PATTERN_ACTIVE_SEARCH_PATTERN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"Searched\sfor\s\d+\spattern").unwrap());

static PATTERN_ACTIVE_READING_FILE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"Reading\s\d+\sfile").unwrap());

/// Detect whether a live session is active or idle based on its pane content.
///
/// Strips ANSI escapes, then scans the last 8 non-empty lines **bottom-up**
/// looking for any `ACTIVE_PATTERNS` match. If none match and the content
/// is non-empty, the session is considered idle.
pub fn detect_activity(content: &str) -> ActivityState {
    if content.trim().is_empty() {
        return ActivityState::Unknown;
    }
    let clean = strip_ansi(content);
    let mut checked = 0usize;
    for line in clean.lines().rev() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        for pat in WAITING_PATTERNS {
            if trimmed.contains(pat) {
                return ActivityState::Waiting;
            }
        }
        for pat in ACTIVE_PATTERNS {
            if pat.is_match(trimmed) {
                return ActivityState::Active;
            }
        }
        checked += 1;
        if checked >= 8 {
            break;
        }
    }
    ActivityState::Idle
}

/// Generate a unique session name of the form `<project>-A`, `<project>-B`, etc.,
/// skipping letters already used by sessions in `existing`. Falls back to numeric
/// suffixes starting at 27 once all 26 letters are taken.
/// Make a string safe to use as a tmux session name.
///
/// tmux forbids `.` and `:` in session names, and treats a leading `.` in a
/// target as a pane reference — so a project folder like `.claude` would create
/// a session that can never be attached, captured, or killed by name. Replace
/// the illegal characters with `-` and trim leading/trailing `-` (falling back
/// to `"session"` if nothing usable remains).
pub(crate) fn sanitize_session_name(name: &str) -> String {
    let replaced: String = name
        .chars()
        .map(|c| if c == '.' || c == ':' { '-' } else { c })
        .collect();
    let trimmed = replaced.trim_matches('-');
    if trimmed.is_empty() {
        "session".to_string()
    } else {
        trimmed.to_string()
    }
}

/// True when `dir` is inside a git working tree. Used to reject `--worktree`
/// launches up front, since claude would otherwise fail inside a tmux session
/// the user has to attach to before seeing the error.
pub fn is_git_repo(dir: &str) -> bool {
    std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .arg("rev-parse")
        .arg("--is-inside-work-tree")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

pub fn generate_auto_name(cwd: &str, existing: &[LiveSession]) -> String {
    let project = std::path::Path::new(cwd)
        .file_name()
        .map(|n| sanitize_session_name(&n.to_string_lossy()))
        .unwrap_or_else(|| "session".to_string());

    let prefix = format!("{}-", project);
    let taken: std::collections::HashSet<String> = existing
        .iter()
        .filter(|ls| ls.tmux_name.starts_with(&prefix))
        .map(|ls| ls.tmux_name[prefix.len()..].to_string())
        .collect();

    for c in b'A'..=b'Z' {
        let letter = (c as char).to_string();
        if !taken.contains(&letter) {
            return format!("{}{}", prefix, letter);
        }
    }
    // All 26 letters taken — fall back to numeric suffixes
    let taken_nums: std::collections::HashSet<String> = existing
        .iter()
        .filter(|ls| ls.tmux_name.starts_with(&prefix))
        .map(|ls| ls.tmux_name[prefix.len()..].to_string())
        .collect();
    let mut n = 27u32;
    loop {
        let suffix = n.to_string();
        if !taken_nums.contains(&suffix) {
            return format!("{}{}", prefix, suffix);
        }
        n += 1;
    }
}

// --- Scheduler primitives -------------------------------------------------
//
// Every tmux invocation below is split into a pure `*_args(...) -> Vec<String>`
// builder plus a thin runner, so the argv is unit-testable without tmux
// present.
//
// Two exact-match target forms are needed, and they differ by command: a
// plain `-t name` does prefix/fnmatch matching, which is a real hazard (a
// session named `ccsm` would also match `ccsm-watch`). Session-scoped
// commands (has-session, kill-session, rename-session, list-clients) want
// `=name`; pane-scoped commands (send-keys, capture-pane, paste-buffer,
// display-message, set-option) want `=name:` — the trailing colon is
// required because pane targets parse as session:window.pane, and without it
// tmux reports "can't find pane" (or, worse, `display-message` silently
// returns an empty string with exit code 0 instead of erroring).

/// Exact-match tmux target for a session-scoped command ("=name").
fn session_target(name: &str) -> String {
    format!("={}", name)
}

/// Exact-match tmux target for a pane-scoped command ("=name:"). The trailing
/// colon is required: pane targets parse as session:window.pane, and without
/// it tmux reports "can't find pane".
fn pane_target(name: &str) -> String {
    format!("={}:", name)
}

/// Build argv for `has-session -t =name`.
fn session_exists_args(name: &str) -> Vec<String> {
    vec![
        "-L".to_string(),
        TMUX_SOCKET.to_string(),
        "has-session".to_string(),
        "-t".to_string(),
        session_target(name),
    ]
}

/// True if a session with exactly this name exists on the ccsm socket.
pub fn session_exists(tmux: &str, name: &str) -> bool {
    std::process::Command::new(tmux)
        .args(session_exists_args(name))
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Build argv for `display-message -p -t =name: "#{pane_in_mode}"`.
fn pane_in_mode_args(name: &str) -> Vec<String> {
    vec![
        "-L".to_string(),
        TMUX_SOCKET.to_string(),
        "display-message".to_string(),
        "-p".to_string(),
        "-t".to_string(),
        pane_target(name),
        "#{pane_in_mode}".to_string(),
    ]
}

/// True if the pane is in copy/scroll mode, where send-keys would be
/// swallowed (a paste still lands, but Escape/Enter/C-u are consumed by copy
/// mode and never reach the application).
pub fn pane_in_mode(tmux: &str, name: &str) -> bool {
    std::process::Command::new(tmux)
        .args(pane_in_mode_args(name))
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim() == "1")
        .unwrap_or(false)
}

/// Build argv for `send-keys -t =name: -X cancel`.
fn cancel_copy_mode_args(name: &str) -> Vec<String> {
    vec![
        "-L".to_string(),
        TMUX_SOCKET.to_string(),
        "send-keys".to_string(),
        "-t".to_string(),
        pane_target(name),
        "-X".to_string(),
        "cancel".to_string(),
    ]
}

/// Leave copy mode so subsequent send-keys reach the application. Failure is
/// non-fatal: callers guard send-keys calls with `pane_in_mode` and only need
/// a best-effort attempt to clear it.
pub fn cancel_copy_mode(tmux: &str, name: &str) {
    let _ = std::process::Command::new(tmux)
        .args(cancel_copy_mode_args(name))
        .output();
}

/// Build argv for `list-clients -t =name`.
fn list_clients_args(name: &str) -> Vec<String> {
    vec![
        "-L".to_string(),
        TMUX_SOCKET.to_string(),
        "list-clients".to_string(),
        "-t".to_string(),
        session_target(name),
    ]
}

/// True if any client is attached, meaning the user is looking at this
/// session.
pub fn has_attached_client(tmux: &str, name: &str) -> bool {
    std::process::Command::new(tmux)
        .args(list_clients_args(name))
        .output()
        .map(|o| o.status.success() && !String::from_utf8_lossy(&o.stdout).trim().is_empty())
        .unwrap_or(false)
}

/// Build argv for `send-keys -t =name: Escape`.
fn send_escape_args(name: &str) -> Vec<String> {
    vec![
        "-L".to_string(),
        TMUX_SOCKET.to_string(),
        "send-keys".to_string(),
        "-t".to_string(),
        pane_target(name),
        "Escape".to_string(),
    ]
}

/// Send a single Escape to interrupt the current turn, leaving claude alive
/// at its prompt. Exits copy mode first. Never send two in rapid succession:
/// Escape at an idle empty prompt is a harmless no-op, but a double Escape
/// sent while the first is still being processed is unverified and best
/// avoided.
pub fn interrupt_session(tmux: &str, name: &str) -> anyhow::Result<()> {
    if pane_in_mode(tmux, name) {
        cancel_copy_mode(tmux, name);
    }
    let output = std::process::Command::new(tmux)
        .args(send_escape_args(name))
        .output()?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("Failed to interrupt session '{}': {}", name, stderr.trim());
    }
    Ok(())
}

/// Build argv for `send-keys -t =name: C-c`.
fn send_ctrl_c_args(name: &str) -> Vec<String> {
    vec![
        "-L".to_string(),
        TMUX_SOCKET.to_string(),
        "send-keys".to_string(),
        "-t".to_string(),
        pane_target(name),
        "C-c".to_string(),
    ]
}

/// Harder interrupt, used only as an escalation step after Escape fails.
pub fn send_ctrl_c(tmux: &str, name: &str) -> anyhow::Result<()> {
    if pane_in_mode(tmux, name) {
        cancel_copy_mode(tmux, name);
    }
    let output = std::process::Command::new(tmux)
        .args(send_ctrl_c_args(name))
        .output()?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!(
            "Failed to send Ctrl+C to session '{}': {}",
            name,
            stderr.trim()
        );
    }
    Ok(())
}

/// Build argv for `send-keys -t =name: C-u`.
fn clear_input_line_args(name: &str) -> Vec<String> {
    vec![
        "-L".to_string(),
        TMUX_SOCKET.to_string(),
        "send-keys".to_string(),
        "-t".to_string(),
        pane_target(name),
        "C-u".to_string(),
    ]
}

/// Clear the pane's current input line so a paste does not concatenate with
/// whatever is already typed there (two successive pastes without this
/// produce a single run-on line of the old and new text).
pub fn clear_input_line(tmux: &str, name: &str) -> anyhow::Result<()> {
    if pane_in_mode(tmux, name) {
        cancel_copy_mode(tmux, name);
    }
    let output = std::process::Command::new(tmux)
        .args(clear_input_line_args(name))
        .output()?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!(
            "Failed to clear input line for session '{}': {}",
            name,
            stderr.trim()
        );
    }
    Ok(())
}

/// Build argv for `send-keys -t =name: Enter`.
fn send_enter_args(name: &str) -> Vec<String> {
    vec![
        "-L".to_string(),
        TMUX_SOCKET.to_string(),
        "send-keys".to_string(),
        "-t".to_string(),
        pane_target(name),
        "Enter".to_string(),
    ]
}

/// Submit the current input line.
pub fn send_enter(tmux: &str, name: &str) -> anyhow::Result<()> {
    if pane_in_mode(tmux, name) {
        cancel_copy_mode(tmux, name);
    }
    let output = std::process::Command::new(tmux)
        .args(send_enter_args(name))
        .output()?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!(
            "Failed to submit input for session '{}': {}",
            name,
            stderr.trim()
        );
    }
    Ok(())
}

/// Monotonic counter appended to generated tmux buffer names so concurrent
/// calls to `send_text` never race on the same buffer name.
static BUFFER_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Generate a unique tmux paste-buffer name, so the daemon never clobbers the
/// user's paste buffer or races another in-flight call.
fn unique_buffer_name() -> String {
    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let n = BUFFER_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("ccsm-{}-{}", millis, n)
}

/// Collapse embedded `\r?\n` into a single space, so an embedded newline
/// cannot submit mid-prompt once it lands in the pane's single-line input box.
fn normalize_prompt_text(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\n', " ")
}

/// Build argv for `load-buffer -b <buf_name> -` (buffer contents are supplied
/// on stdin, not argv).
fn load_buffer_args(buf_name: &str) -> Vec<String> {
    vec![
        "-L".to_string(),
        TMUX_SOCKET.to_string(),
        "load-buffer".to_string(),
        "-b".to_string(),
        buf_name.to_string(),
        "-".to_string(),
    ]
}

/// Build argv for `paste-buffer -d -p -b <buf_name> -t =name:`.
fn paste_buffer_args(buf_name: &str, name: &str) -> Vec<String> {
    vec![
        "-L".to_string(),
        TMUX_SOCKET.to_string(),
        "paste-buffer".to_string(),
        "-d".to_string(),
        "-p".to_string(),
        "-b".to_string(),
        buf_name.to_string(),
        "-t".to_string(),
        pane_target(name),
    ]
}

/// Insert text as a single bracketed paste without submitting it. Copy mode
/// swallows send-keys but not pastes, so no `pane_in_mode` guard is needed
/// here. The text is written to `load-buffer`'s stdin rather than passed on
/// argv, so there is no argv length limit and no shell-quoting to get wrong.
pub fn send_text(tmux: &str, name: &str, text: &str) -> anyhow::Result<()> {
    let normalized = normalize_prompt_text(text);
    let buf_name = unique_buffer_name();

    let mut child = std::process::Command::new(tmux)
        .args(load_buffer_args(&buf_name))
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .with_context(|| format!("Failed to spawn load-buffer for session '{}'", name))?;
    child
        .stdin
        .take()
        .expect("stdin was piped")
        .write_all(normalized.as_bytes())
        .with_context(|| format!("Failed to write buffer contents for session '{}'", name))?;
    let output = child
        .wait_with_output()
        .with_context(|| format!("Failed to load buffer for session '{}'", name))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!(
            "Failed to load-buffer for session '{}': {}",
            name,
            stderr.trim()
        );
    }

    let output = std::process::Command::new(tmux)
        .args(paste_buffer_args(&buf_name, name))
        .output()?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!(
            "Failed to paste buffer into session '{}': {}",
            name,
            stderr.trim()
        );
    }
    Ok(())
}

/// Full prompt delivery: exit copy mode, clear the input line, paste, settle,
/// then submit. The ~200ms settle between paste and Enter gives the TUI time
/// to process the paste before the Enter key is sent; without it the Enter
/// can race the paste and land before the text has been read into the input
/// box.
pub fn send_prompt(tmux: &str, name: &str, text: &str) -> anyhow::Result<()> {
    if pane_in_mode(tmux, name) {
        cancel_copy_mode(tmux, name);
    }
    clear_input_line(tmux, name)?;
    send_text(tmux, name, text)?;
    std::thread::sleep(Duration::from_millis(200));
    send_enter(tmux, name)?;
    Ok(())
}

/// Build argv for `set-option -t =name: @ccsm_job <job_id>`.
fn set_job_tag_args(name: &str, job_id: &str) -> Vec<String> {
    vec![
        "-L".to_string(),
        TMUX_SOCKET.to_string(),
        "set-option".to_string(),
        "-t".to_string(),
        pane_target(name),
        "@ccsm_job".to_string(),
        job_id.to_string(),
    ]
}

/// Tag a tmux session with a ccsm job id. Survives `rename-session`, unlike a
/// name-based binding.
pub fn set_job_tag(tmux: &str, name: &str, job_id: &str) -> anyhow::Result<()> {
    let output = std::process::Command::new(tmux)
        .args(set_job_tag_args(name, job_id))
        .output()?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!(
            "Failed to tag session '{}' with job id: {}",
            name,
            stderr.trim()
        );
    }
    Ok(())
}

/// Build argv for `set-option -t =name: @ccsm_backend <tag>`.
fn set_backend_tag_args(name: &str, backend: AgentBackend) -> Vec<String> {
    vec![
        "-L".to_string(),
        TMUX_SOCKET.to_string(),
        "set-option".to_string(),
        "-t".to_string(),
        pane_target(name),
        "@ccsm_backend".to_string(),
        backend.tmux_tag().to_string(),
    ]
}

/// Tag a tmux session with the agent backend that is running in it.
pub fn set_backend_tag(tmux: &str, name: &str, backend: AgentBackend) -> anyhow::Result<()> {
    let output = std::process::Command::new(tmux)
        .args(set_backend_tag_args(name, backend))
        .output()?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!(
            "Failed to tag session '{}' with backend: {}",
            name,
            stderr.trim()
        );
    }
    Ok(())
}

/// `start_live_session` followed by `set_job_tag`, so a newly dispatched
/// session is immediately findable by job id. Jobs are Claude-only.
pub fn start_managed_session(
    tmux: &str,
    name: &str,
    cwd: &str,
    job_id: &str,
    cmd: &[&str],
) -> anyhow::Result<()> {
    start_live_session(tmux, name, cwd, cmd, Some(AgentBackend::ClaudeCode))?;
    set_job_tag(tmux, name, job_id)
}

/// Poll until the pane's output stops changing, indicating startup has
/// settled. Heuristic: two consecutive 30-line captures that are non-empty
/// and identical. Polls every 250ms until `timeout` elapses; returns `false`
/// if it never settles in time.
pub fn wait_pane_settled(tmux: &str, name: &str, timeout: Duration) -> bool {
    let start = std::time::Instant::now();
    let mut previous: Option<String> = None;
    while start.elapsed() < timeout {
        let capture = poll_pane_tail(tmux, name, 30);
        if !capture.trim().is_empty() {
            if previous.as_deref() == Some(capture.as_str()) {
                return true;
            }
            previous = Some(capture);
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    false
}

/// True if the pane is showing claude's "do you trust this folder" dialog,
/// which blocks a dispatched session indefinitely. Callers pass
/// already-captured pane text (e.g. from `poll_pane_tail`).
pub fn detect_trust_prompt(content: &str) -> bool {
    let clean = strip_ansi(content);
    clean.contains("Quick safety check: Is this a project you created or one you trust?")
        || clean.contains("Yes, I trust this folder")
}

/// True if the pane shows claude's post-interrupt marker, confirming a soft
/// pause actually took effect. Callers pass already-captured pane text (e.g.
/// from `poll_pane_tail`).
pub fn detect_interrupted(content: &str) -> bool {
    let clean = strip_ansi(content);
    clean.contains("Interrupted \u{00b7} What should Claude do instead?")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_git_repo_distinguishes_repos_from_plain_directories() {
        // The crate root is a repo; a fresh temp dir is not.
        assert!(is_git_repo(env!("CARGO_MANIFEST_DIR")));
        let dir = tempfile::tempdir().unwrap();
        assert!(!is_git_repo(&dir.path().to_string_lossy()));
    }

    #[test]
    fn is_git_repo_is_false_for_a_missing_directory() {
        assert!(!is_git_repo("/nonexistent/path/for/ccsm/tests"));
    }

    #[test]
    fn sanitize_session_name_strips_leading_dot() {
        // Dot-folders like `.claude` must not produce a leading `.` — tmux
        // parses that as a pane target and the session becomes unreachable.
        assert_eq!(sanitize_session_name(".claude"), "claude");
        assert_eq!(sanitize_session_name(".kanbots"), "kanbots");
    }

    #[test]
    fn sanitize_session_name_replaces_illegal_chars() {
        assert_eq!(sanitize_session_name("v1.2.3"), "v1-2-3");
        assert_eq!(sanitize_session_name("a:b"), "a-b");
    }

    #[test]
    fn sanitize_session_name_falls_back_when_empty() {
        assert_eq!(sanitize_session_name("."), "session");
        assert_eq!(sanitize_session_name("..."), "session");
    }

    #[test]
    fn generate_auto_name_sanitizes_dot_folder() {
        let name = generate_auto_name("/Users/sane/.claude", &[]);
        assert_eq!(name, "claude-A");
        assert!(!name.contains('.'), "tmux session name must not contain a dot");
    }

    #[test]
    fn strip_ansi_removes_csi_sequences() {
        let input = "\x1b[32mHello\x1b[0m World";
        assert_eq!(strip_ansi(input), "Hello World");
    }

    #[test]
    fn strip_ansi_preserves_plain_text() {
        let input = "no escape codes here";
        assert_eq!(strip_ansi(input), input);
    }

    #[test]
    fn strip_ansi_removes_osc_bel_terminated() {
        // OSC sequence terminated by BEL (e.g., setting terminal title)
        let input = "\x1b]0;My Title\x07Hello";
        assert_eq!(strip_ansi(input), "Hello");
    }

    #[test]
    fn strip_ansi_removes_osc_st_terminated() {
        // OSC sequence terminated by ST (ESC \)
        let input = "\x1b]0;My Title\x1b\\Hello";
        assert_eq!(strip_ansi(input), "Hello");
    }

    #[test]
    fn detect_activity_active_timer_thinking() {
        let content = "some output\nThinking\u{2026} (10m \u{00b7} 13.0k tokens)";
        assert_eq!(detect_activity(content), ActivityState::Active);
    }

    #[test]
    fn detect_activity_active_timer_multipart() {
        let content = "Thinking\u{2026} (8m 0s \u{00b7} 13.0k tokens)";
        assert_eq!(detect_activity(content), ActivityState::Active);
    }

    #[test]
    fn detect_activity_active_timer_long_duration() {
        let content = "Thinking\u{2026} (1h 35m 22s \u{00b7} 42.3k tokens)";
        assert_eq!(detect_activity(content), ActivityState::Active);
    }

    #[test]
    fn detect_activity_active_with_ansi() {
        let content = "\x1b[32mThinking\u{2026}\x1b[0m (5m \u{00b7} 8.1k tokens)";
        assert_eq!(detect_activity(content), ActivityState::Active);
    }

    #[test]
    fn detect_activity_empty_is_unknown() {
        assert_eq!(detect_activity(""), ActivityState::Unknown);
        assert_eq!(detect_activity("   \n  "), ActivityState::Unknown);
    }

    #[test]
    fn detect_activity_plain_text_is_idle() {
        let content = "some output\nclaude output here";
        assert_eq!(detect_activity(content), ActivityState::Idle);
    }

    #[test]
    fn detect_activity_ascii_dots_not_active() {
        // ASCII "..." (three dots) should NOT match — Claude uses Unicode ellipsis
        let content = "Thinking... (10m \u{00b7} 13.0k tokens)";
        assert_eq!(detect_activity(content), ActivityState::Idle);
    }

    #[test]
    fn detect_activity_completion_summary_is_idle() {
        let content = "Brewed for 2m 30s \u{00b7} 15.2k tokens";
        assert_eq!(detect_activity(content), ActivityState::Idle);
    }

    #[test]
    fn detect_activity_active_below_idle_is_active() {
        // Bottom-up scan: active timer below idle content means active
        let content = "Brewed for 44s\nNew task\nThinking\u{2026} (2m \u{00b7} 5.0k tokens)";
        assert_eq!(detect_activity(content), ActivityState::Active);
    }

    #[test]
    fn detect_activity_idle_below_active_is_idle() {
        // Bottom-up scan: active timer earlier, but only non-matching lines at bottom
        // The scan hits non-empty lines first, none match → Idle
        let content = "Earlier active output\nDone output\nPrompt >";
        assert_eq!(detect_activity(content), ActivityState::Idle);
    }

    #[test]
    fn detect_activity_prose_ellipsis_is_idle() {
        let content = "The tests passed\u{2026} everything looks good.";
        assert_eq!(detect_activity(content), ActivityState::Idle);
    }

    #[test]
    fn detect_activity_more_tool_uses_is_active() {
        let content = "some output\n+3 more tool uses (ctrl+o to expand)";
        assert_eq!(detect_activity(content), ActivityState::Active);
    }

    #[test]
    fn detect_activity_waiting_proceed_prompt() {
        let content = "some output\nDo you want to proceed?";
        assert_eq!(detect_activity(content), ActivityState::Waiting);
    }

    #[test]
    fn detect_activity_waiting_with_ansi() {
        let content = "\x1b[33mDo you want to proceed?\x1b[0m";
        assert_eq!(detect_activity(content), ActivityState::Waiting);
    }

    #[test]
    fn detect_activity_waiting_beats_active() {
        // If both waiting and active patterns appear, waiting (checked first) wins
        let content = "Thinking\u{2026} (2m \u{00b7} 5.0k tokens)\nDo you want to proceed?";
        assert_eq!(detect_activity(content), ActivityState::Waiting);
    }

    #[test]
    fn session_target_is_exact_match_prefix() {
        assert_eq!(session_target("x"), "=x");
    }

    #[test]
    fn pane_target_has_trailing_colon() {
        assert_eq!(pane_target("x"), "=x:");
    }

    #[test]
    fn session_exists_args_shape() {
        assert_eq!(
            session_exists_args("my-sess"),
            vec!["-L", "ccsm", "has-session", "-t", "=my-sess"]
        );
    }

    #[test]
    fn pane_in_mode_args_shape() {
        assert_eq!(
            pane_in_mode_args("my-sess"),
            vec![
                "-L",
                "ccsm",
                "display-message",
                "-p",
                "-t",
                "=my-sess:",
                "#{pane_in_mode}"
            ]
        );
    }

    #[test]
    fn cancel_copy_mode_args_shape() {
        assert_eq!(
            cancel_copy_mode_args("my-sess"),
            vec!["-L", "ccsm", "send-keys", "-t", "=my-sess:", "-X", "cancel"]
        );
    }

    #[test]
    fn list_clients_args_shape() {
        assert_eq!(
            list_clients_args("my-sess"),
            vec!["-L", "ccsm", "list-clients", "-t", "=my-sess"]
        );
    }

    #[test]
    fn send_escape_args_shape() {
        // The interrupt builder: a single Escape targeted at the pane.
        assert_eq!(
            send_escape_args("my-sess"),
            vec!["-L", "ccsm", "send-keys", "-t", "=my-sess:", "Escape"]
        );
    }

    #[test]
    fn send_ctrl_c_args_shape() {
        assert_eq!(
            send_ctrl_c_args("my-sess"),
            vec!["-L", "ccsm", "send-keys", "-t", "=my-sess:", "C-c"]
        );
    }

    #[test]
    fn clear_input_line_args_shape() {
        assert_eq!(
            clear_input_line_args("my-sess"),
            vec!["-L", "ccsm", "send-keys", "-t", "=my-sess:", "C-u"]
        );
    }

    #[test]
    fn send_enter_args_shape() {
        assert_eq!(
            send_enter_args("my-sess"),
            vec!["-L", "ccsm", "send-keys", "-t", "=my-sess:", "Enter"]
        );
    }

    #[test]
    fn load_buffer_args_shape() {
        assert_eq!(
            load_buffer_args("ccsm-123-0"),
            vec!["-L", "ccsm", "load-buffer", "-b", "ccsm-123-0", "-"]
        );
    }

    #[test]
    fn paste_buffer_args_shape() {
        assert_eq!(
            paste_buffer_args("ccsm-123-0", "my-sess"),
            vec![
                "-L",
                "ccsm",
                "paste-buffer",
                "-d",
                "-p",
                "-b",
                "ccsm-123-0",
                "-t",
                "=my-sess:"
            ]
        );
    }

    #[test]
    fn set_job_tag_args_shape() {
        assert_eq!(
            set_job_tag_args("my-sess", "job-42"),
            vec![
                "-L",
                "ccsm",
                "set-option",
                "-t",
                "=my-sess:",
                "@ccsm_job",
                "job-42"
            ]
        );
    }

    #[test]
    fn targets_handle_unusual_session_names_as_single_argv_element() {
        // A name containing '=', spaces, or a leading '-' must still produce
        // exactly one argv element per target, never be split or reinterpreted
        // as extra flags.
        for name in ["=weird", "has space", "-flag-like"] {
            assert_eq!(session_target(name), format!("={}", name));
            assert_eq!(pane_target(name), format!("={}:", name));
            let args = send_escape_args(name);
            assert_eq!(args.len(), 6);
            assert_eq!(args[4], pane_target(name));
        }
    }

    #[test]
    fn normalize_prompt_text_collapses_newlines() {
        assert_eq!(normalize_prompt_text("line one\nline two"), "line one line two");
        assert_eq!(
            normalize_prompt_text("line one\r\nline two\r\nline three"),
            "line one line two line three"
        );
        assert_eq!(normalize_prompt_text("no newline"), "no newline");
    }

    #[test]
    fn parse_session_lines_splits_tagged_backend() {
        let text = "sess-a\t/home/sess-a\tjob-1\tcursor\tagent --trust\n";
        let sessions = parse_session_lines(text);
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].tmux_name, "sess-a");
        assert_eq!(sessions[0].cwd, "/home/sess-a");
        assert_eq!(sessions[0].job_id, Some("job-1".to_string()));
        assert_eq!(sessions[0].backend, Some(AgentBackend::CursorAgent));
    }

    #[test]
    fn parse_session_lines_infers_backend_from_start_command_when_untagged() {
        let text = "sess-a\t/home/sess-a\t\t\tagent --trust\n";
        let sessions = parse_session_lines(text);
        assert_eq!(sessions[0].job_id, None);
        assert_eq!(sessions[0].backend, Some(AgentBackend::CursorAgent));

        let text = "sess-b\t/home/sess-b\t\t\t/opt/bin/claude --resume abc\n";
        let sessions = parse_session_lines(text);
        assert_eq!(sessions[0].backend, Some(AgentBackend::ClaudeCode));
    }

    #[test]
    fn infer_backend_skips_env_unset_wrapper() {
        assert_eq!(
            infer_backend_from_start_command(
                "env -u CURSOR_AGENT -u CURSOR_CONVERSATION_ID agent --trust --resume abc"
            ),
            Some(AgentBackend::CursorAgent)
        );
    }

    #[test]
    fn parse_session_lines_tag_wins_over_start_command() {
        // Tag says Claude even if the start command looks like agent.
        let text = "sess-a\t/home/sess-a\t\tclaude\tagent --trust\n";
        let sessions = parse_session_lines(text);
        assert_eq!(sessions[0].backend, Some(AgentBackend::ClaudeCode));
    }

    #[test]
    fn parse_session_lines_excludes_watch_session() {
        let text = "ccsm-watch\t/home/watch\t\t\t\nsess-a\t/home/sess-a\tjob-1\tclaude\tclaude\n";
        let sessions = parse_session_lines(text);
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].tmux_name, "sess-a");
    }

    #[test]
    fn parse_session_lines_missing_trailing_columns_default_to_none() {
        let text = "sess-a\t/home/sess-a\n";
        let sessions = parse_session_lines(text);
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].tmux_name, "sess-a");
        assert_eq!(sessions[0].cwd, "/home/sess-a");
        assert_eq!(sessions[0].job_id, None);
        assert_eq!(sessions[0].backend, None);
    }

    #[test]
    fn parse_session_lines_unknown_backend_tag_falls_back_to_start_command() {
        let text = "sess-a\t/home/sess-a\t\twat\tagent --trust\n";
        let sessions = parse_session_lines(text);
        assert_eq!(sessions[0].backend, Some(AgentBackend::CursorAgent));
    }

    #[test]
    fn infer_backend_from_start_command_matches_basename() {
        assert_eq!(
            infer_backend_from_start_command("agent --trust"),
            Some(AgentBackend::CursorAgent)
        );
        assert_eq!(
            infer_backend_from_start_command("/usr/local/bin/claude"),
            Some(AgentBackend::ClaudeCode)
        );
        assert_eq!(infer_backend_from_start_command("node"), None);
        assert_eq!(infer_backend_from_start_command(""), None);
    }

    #[test]
    fn set_backend_tag_args_shape() {
        assert_eq!(
            set_backend_tag_args("my-sess", AgentBackend::CursorAgent),
            vec![
                "-L",
                "ccsm",
                "set-option",
                "-t",
                "=my-sess:",
                "@ccsm_backend",
                "cursor"
            ]
        );
    }

    #[test]
    fn detect_trust_prompt_matches_real_dialog() {
        let content = "Quick safety check: Is this a project you created or one you trust?\n\n1. Yes, I trust this folder\n2. No";
        assert!(detect_trust_prompt(content));
    }

    #[test]
    fn detect_trust_prompt_negative_on_ordinary_output() {
        let content = "some output\nclaude output here";
        assert!(!detect_trust_prompt(content));
    }

    #[test]
    fn detect_interrupted_matches_marker() {
        let content = "\u{23bf}  Interrupted \u{00b7} What should Claude do instead?";
        assert!(detect_interrupted(content));
    }

    #[test]
    fn detect_interrupted_negative_on_ordinary_output() {
        let content = "some output\nclaude output here";
        assert!(!detect_interrupted(content));
    }

    #[test]
    fn with_cleared_cursor_env_prefixes_env_unset() {
        let argv = with_cleared_cursor_env(&["agent", "--trust", "--resume", "abc"]);
        assert_eq!(argv[0], "env");
        assert!(argv.windows(2).any(|w| w == ["-u", "CURSOR_AGENT"]));
        assert!(argv.windows(2).any(|w| w == ["-u", "CURSOR_CONVERSATION_ID"]));
        assert!(argv.windows(2).any(|w| w == ["-u", "CURSOR_ASKPASS_SOCKET"]));
        assert_eq!(&argv[argv.len() - 4..], ["agent", "--trust", "--resume", "abc"]);
    }

    #[test]
    fn pane_dead_args_shape() {
        assert_eq!(
            pane_dead_args("my-sess"),
            vec![
                "-L",
                "ccsm",
                "display-message",
                "-p",
                "-t",
                "=my-sess:",
                "#{pane_dead}"
            ]
        );
    }

    #[test]
    fn summarize_pane_tail_keeps_last_nonempty_lines() {
        let tail = "\x1b[32mhello\x1b[0m\n\n\nLoading conversation\n\nboom\n";
        let s = summarize_pane_tail(tail);
        assert!(s.contains("Loading conversation"), "{s}");
        assert!(s.contains("boom"), "{s}");
        assert!(!s.contains('\x1b'), "{s}");
    }

    #[test]
    fn summarize_pane_tail_empty_when_blank() {
        assert_eq!(summarize_pane_tail("\n\n  \n"), "");
    }

    #[test]
    fn ensure_live_pane_stays_up_reports_early_exit() {
        let tmux = "tmux";
        if !is_tmux_available(tmux) {
            return;
        }
        let name = format!("ccsm-test-dead-{}", std::process::id());
        let _ = stop_live_session(tmux, &name);
        // Sleep briefly so remain-on-exit can be armed before the pane exits.
        start_live_session(
            tmux,
            &name,
            std::env::temp_dir().to_str().unwrap_or("/tmp"),
            &["sh", "-c", "echo RESUME_BLEW_UP; sleep 0.25; exit 1"],
            None,
        )
        .expect("start short-lived pane");
        let err = ensure_live_pane_stays_up(
            tmux,
            &name,
            "test agent",
            Duration::from_millis(1200),
        )
        .expect_err("pane that exits must error");
        let msg = format!("{err:#}");
        assert!(
            msg.contains("Cursor Agent resume exited early"),
            "{msg}"
        );
        assert!(
            msg.contains("RESUME_BLEW_UP") || err.downcast_ref::<CursorResumeFailure>().is_some(),
            "{msg}"
        );
        assert!(!session_exists(tmux, &name), "dead session must be cleaned up");
    }

    #[test]
    fn cursor_early_exit_error_is_typed_failure() {
        let err = cursor_early_exit_error("");
        let fail = err
            .downcast_ref::<CursorResumeFailure>()
            .expect("must be CursorResumeFailure");
        assert!(fail.detail.is_empty());

        let err = cursor_early_exit_error("Loading conversation");
        let fail = err
            .downcast_ref::<CursorResumeFailure>()
            .expect("must be CursorResumeFailure");
        assert_eq!(fail.detail, "Loading conversation");
    }
}
