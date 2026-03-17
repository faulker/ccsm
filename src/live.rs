// Manages the dedicated ccsm tmux server and live sessions

pub const TMUX_SOCKET: &str = "ccsm";

pub struct LiveSession {
    pub tmux_name: String,
    pub display_name: String,
    pub cwd: String,
    pub project_name: String,
}

pub fn is_server_running() -> bool {
    std::process::Command::new("tmux")
        .args(["-L", TMUX_SOCKET, "list-sessions"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

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

pub fn conf_path() -> std::path::PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join(".config")
        .join("ccsm")
        .join("tmux.conf")
}

pub fn ensure_server_configured() {
    let conf_path = conf_path();
    let _ = std::fs::create_dir_all(conf_path.parent().unwrap());
    // Within tmux double-quoted strings, \\ is a literal backslash.
    // For bind-key in config files, single-quote the key spec to avoid backslash escape issues.
    let conf_content = concat!(
        "set -g history-limit 50000\n",
        "set -g mouse on\n",
        "set -g default-terminal \"tmux-256color\"\n",
        "set -g status on\n",
        "set -g status-interval 1\n",
        "set -g status-style \"bg=#1e1e2e,fg=#cdd6f4\"\n",
        "set -g status-format[0] \"#[align=left,bg=#1e1e2e,fg=#cdd6f4]#[align=centre]#{?pane_in_mode,#[fg=#f38ba8 bold]Hit the ESC key to exit scroll mode}#[align=right]#[fg=#f38ba8 bold]Ctrl+\\\\ #[fg=#a6adc8]detach  #[fg=#f38ba8 bold]Ctrl+n #[fg=#a6adc8]next  #[fg=#f38ba8 bold]Ctrl+p #[fg=#a6adc8]prev \"\n",
        "unbind-key -q -n C-]\n",
        "unbind-key -q -n C-[\n",
        "bind-key -n 'C-\\' detach-client\n",
        "bind-key -n C-n switch-client -n\n",
        "bind-key -n C-p switch-client -p\n",
    );
    let _ = std::fs::write(&conf_path, conf_content);
    // If the server is already running, source the config to update bindings.
    // If not running, start-server is unreliable on tmux 3.x — the server is started
    // implicitly when new-session runs (see start_live_session which passes -f).
    let _ = std::process::Command::new("tmux")
        .args(["-L", TMUX_SOCKET, "source-file", &conf_path.to_string_lossy()])
        .output();
}

pub fn is_tmux_available() -> bool {
    std::process::Command::new("tmux")
        .arg("-V")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

pub fn start_live_session(name: &str, cwd: &str, claude_args: &[&str]) -> anyhow::Result<()> {
    if !is_tmux_available() {
        anyhow::bail!("tmux is not installed — live sessions require tmux");
    }
    ensure_server_configured();
    let conf_path_str = conf_path().to_string_lossy().into_owned();
    // Pass -f so that if the server isn't running yet, it starts with our config.
    // If the server is already running, -f is ignored by tmux.
    let mut cmd_args = vec![
        "-L", TMUX_SOCKET,
        "-f", &conf_path_str,
        "new-session", "-d",
        "-s", name,
        "-c", cwd,
        "claude",
    ];
    cmd_args.extend(claude_args);
    let output = std::process::Command::new("tmux")
        .args(&cmd_args)
        .output()?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("Failed to create session '{}': {}", name, stderr.trim());
    }
    Ok(())
}

pub fn attach_to_session(name: &str) -> anyhow::Result<()> {
    if !is_tmux_available() {
        anyhow::bail!("tmux is not installed — live sessions require tmux");
    }
    ensure_server_configured();
    let status = std::process::Command::new("tmux")
        .args(["-L", TMUX_SOCKET, "attach-session", "-t", name])
        .status()?;
    if !status.success() {
        anyhow::bail!("Failed to attach to session '{}'", name);
    }
    Ok(())
}

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

pub fn poll_pane_buffer(name: &str, lines: usize) -> Vec<String> {
    let lines_str = format!("-{}", lines);
    let output = std::process::Command::new("tmux")
        .args([
            "-L", TMUX_SOCKET,
            "capture-pane", "-t", name,
            "-p", "-S", &lines_str,
        ])
        .output();
    match output {
        Ok(o) if o.status.success() => {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .map(|l| l.to_string())
                .collect()
        }
        _ => vec![],
    }
}

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
