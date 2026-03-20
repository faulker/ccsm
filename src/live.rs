// Manages the dedicated ccsm tmux server and live sessions

use anyhow::Context;
use regex::Regex;
use std::sync::LazyLock;

pub const TMUX_SOCKET: &str = "ccsm";

/// A running tmux session managed by ccsm on the dedicated `ccsm` tmux socket.
pub struct LiveSession {
    /// The tmux session name used to target it in tmux commands.
    pub tmux_name: String,
    /// The name shown in the UI (same as `tmux_name` unless renamed).
    pub display_name: String,
    /// Working directory of the tmux session (from `#{session_path}`).
    pub cwd: String,
    /// Base name of the working directory, used as a short project label.
    pub project_name: String,
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
            "#{session_name}\t#{session_path}",
        ])
        .output();
    let output = match output {
        Ok(o) if o.status.success() => o,
        _ => return vec![],
    };
    let text = String::from_utf8_lossy(&output.stdout);
    text.lines()
        .filter_map(|line| {
            let (name, path) = line.split_once('\t')?;
            let name = name.to_string();
            let path = path.to_string();
            let project_name = std::path::Path::new(&path)
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "session".to_string());
            Some(LiveSession {
                display_name: name.clone(),
                tmux_name: name,
                cwd: path,
                project_name,
            })
        })
        .collect()
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

/// Create a new detached tmux session named `name` with working directory `cwd`,
/// running `cmd` as the initial command. Starts the ccsm tmux server if needed.
pub fn start_live_session(tmux: &str, name: &str, cwd: &str, cmd: &[&str]) -> anyhow::Result<()> {
    if !is_tmux_available(tmux) {
        anyhow::bail!("tmux is not installed — live sessions require tmux");
    }
    ensure_server_configured(tmux)?;
    let conf_path_str = conf_path()
        .ok_or_else(|| anyhow::anyhow!("Could not determine home directory for config path"))?
        .to_string_lossy()
        .into_owned();
    // Pass -f so that if the server isn't running yet, it starts with our config.
    // If the server is already running, -f is ignored by tmux.
    let mut cmd_args = vec![
        "-L",
        TMUX_SOCKET,
        "-f",
        &conf_path_str,
        "new-session",
        "-d",
        "-s",
        name,
        "-c",
        cwd,
    ];
    cmd_args.extend(cmd);
    let output = std::process::Command::new(tmux).args(&cmd_args).output()?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("Failed to create session '{}': {}", name, stderr.trim());
    }
    Ok(())
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
pub fn generate_auto_name(cwd: &str, existing: &[LiveSession]) -> String {
    let project = std::path::Path::new(cwd)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
