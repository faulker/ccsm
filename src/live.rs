// Manages the dedicated ccsm tmux server and live sessions

use anyhow::Context;

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
pub fn is_server_running() -> bool {
    std::process::Command::new("tmux")
        .args(["-L", TMUX_SOCKET, "list-sessions"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Query the ccsm tmux server for all running sessions and return them as `LiveSession` values.
/// Returns an empty vec if the server is not running or the command fails.
pub fn discover_live_sessions() -> Vec<LiveSession> {
    if !is_server_running() {
        return vec![];
    }
    let output = std::process::Command::new("tmux")
        .args([
            "-L", TMUX_SOCKET,
            "list-sessions",
            "-F", "#{session_name}:#{session_path}",
        ])
        .output();
    let output = match output {
        Ok(o) if o.status.success() => o,
        _ => return vec![],
    };
    let text = String::from_utf8_lossy(&output.stdout);
    text.lines()
        .filter_map(|line| {
            let mut parts = line.splitn(2, ':');
            let name = parts.next()?.to_string();
            let path = parts.next().unwrap_or("").to_string();
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
pub fn conf_path() -> std::path::PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join(".config")
        .join("ccsm")
        .join("tmux.conf")
}

/// Write the ccsm tmux config file and, if the server is already running, source it to apply changes.
pub fn ensure_server_configured() -> anyhow::Result<()> {
    let conf_path = conf_path();
    if let Some(parent) = conf_path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create config directory: {}", parent.display()))?;
    }
    // Within tmux double-quoted strings, \\ is a literal backslash.
    // For bind-key in config files, single-quote the key spec to avoid backslash escape issues.
    let mut conf_content = concat!(
        "set -g history-limit 50000\n",
        "set -g mouse on\n",
        "set -g default-terminal \"tmux-256color\"\n",
        "set -g extended-keys on\n",
        "set -g status on\n",
        "set -g status-interval 1\n",
        "set -g status-style \"bg=#1e1e2e,fg=#cdd6f4\"\n",
        "set -g status-format[0] \"#[align=left,bg=#1e1e2e,fg=#cdd6f4]#[align=centre]#{?pane_in_mode,#[fg=#f38ba8 bold]Hit the ESC key to exit scroll mode}#[align=right]#[fg=#f38ba8 bold]Ctrl+\\\\ #[fg=#a6adc8]detach  #[fg=#f38ba8 bold]Ctrl+n #[fg=#a6adc8]next  #[fg=#f38ba8 bold]Ctrl+p #[fg=#a6adc8]prev \"\n",
        "unbind-key -q -n C-]\n",
        "unbind-key -q -n C-[\n",
        "bind-key -n 'C-\\' detach-client\n",
        "bind-key -n C-n switch-client -n\n",
        "bind-key -n C-p switch-client -p\n",
    ).to_string();

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
    let _ = std::process::Command::new("tmux")
        .args(["-L", TMUX_SOCKET, "source-file", &conf_path.to_string_lossy()])
        .output();
    Ok(())
}

/// Returns true if `tmux` is installed and reachable on the system PATH.
pub fn is_tmux_available() -> bool {
    std::process::Command::new("tmux")
        .arg("-V")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Create a new detached tmux session named `name` with working directory `cwd`,
/// running `cmd` as the initial command. Starts the ccsm tmux server if needed.
pub fn start_live_session(name: &str, cwd: &str, cmd: &[&str]) -> anyhow::Result<()> {
    if !is_tmux_available() {
        anyhow::bail!("tmux is not installed — live sessions require tmux");
    }
    ensure_server_configured()?;
    let conf_path_str = conf_path().to_string_lossy().into_owned();
    // Pass -f so that if the server isn't running yet, it starts with our config.
    // If the server is already running, -f is ignored by tmux.
    let mut cmd_args = vec![
        "-L", TMUX_SOCKET,
        "-f", &conf_path_str,
        "new-session", "-d",
        "-s", name,
        "-c", cwd,
    ];
    cmd_args.extend(cmd);
    let output = std::process::Command::new("tmux")
        .args(&cmd_args)
        .output()?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("Failed to create session '{}': {}", name, stderr.trim());
    }
    Ok(())
}

/// Attach the current process to the named tmux session on the ccsm socket.
pub fn attach_to_session(name: &str) -> anyhow::Result<()> {
    if !is_tmux_available() {
        anyhow::bail!("tmux is not installed — live sessions require tmux");
    }
    ensure_server_configured()?;
    let status = std::process::Command::new("tmux")
        .args(["-L", TMUX_SOCKET, "attach-session", "-t", name])
        .status()?;
    if !status.success() {
        anyhow::bail!("Failed to attach to session '{}'", name);
    }
    Ok(())
}

/// Send Ctrl+C to interrupt any running process, then kill the named tmux session.
pub fn stop_live_session(name: &str) -> anyhow::Result<()> {
    // Send Ctrl+C to interrupt any running process before killing the session
    let _ = std::process::Command::new("tmux")
        .args(["-L", TMUX_SOCKET, "send-keys", "-t", name, "C-c", ""])
        .output();
    let output = std::process::Command::new("tmux")
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
pub fn poll_pane_buffer(name: &str, lines: usize) -> String {
    let lines_str = format!("-{}", lines);
    let output = std::process::Command::new("tmux")
        .args([
            "-L", TMUX_SOCKET,
            "capture-pane", "-t", name,
            "-p", "-e", "-S", &lines_str,
        ])
        .output();
    match output {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).into_owned(),
        _ => String::new(),
    }
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
