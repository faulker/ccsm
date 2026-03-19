mod app;
mod config;
mod config_popup;
mod data;
mod keys;
mod live;
mod ui;
mod update;

use anyhow::Result;
use app::App;
use crossterm::{
    event::{self, KeyboardEnhancementFlags, PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags},
    terminal::{self, EnterAlternateScreen, LeaveAlternateScreen},
    ExecutableCommand,
};
use ratatui::prelude::CrosstermBackend;
use ratatui::Terminal;
use std::io;
use std::path::PathBuf;

/// Enable raw mode, switch to the alternate screen, request keyboard enhancement flags,
/// and return an initialised ratatui `Terminal`.
fn setup_terminal() -> Result<Terminal<CrosstermBackend<io::Stdout>>> {
    terminal::enable_raw_mode()?;
    io::stdout().execute(EnterAlternateScreen)?;
    let _ = io::stdout().execute(PushKeyboardEnhancementFlags(
        KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
            | KeyboardEnhancementFlags::REPORT_EVENT_TYPES
            | KeyboardEnhancementFlags::REPORT_ALL_KEYS_AS_ESCAPE_CODES,
    ));
    let backend = CrosstermBackend::new(io::stdout());
    let terminal = Terminal::new(backend)?;
    Ok(terminal)
}

/// Pop keyboard enhancement flags, disable raw mode, and leave the alternate screen.
fn restore_terminal() -> Result<()> {
    let _ = io::stdout().execute(PopKeyboardEnhancementFlags);
    terminal::disable_raw_mode()?;
    io::stdout().execute(LeaveAlternateScreen)?;
    Ok(())
}

/// Run the main TUI event loop. Spawns a background update check, handles session
/// launches and in-place binary updates, and returns `true` if the process should
/// exec-restart itself (e.g. after a self-update).
fn run_app(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    sessions: Vec<data::SessionInfo>,
    filter_path: Option<String>,
    flat: bool,
    live_start: bool,
    new_session: bool,
) -> Result<bool> {
    let config = config::Config::load();
    let mut app = App::new(sessions, filter_path.clone(), config);
    if flat {
        app.tree_view = false;
    }
    if live_start {
        app.live_filter = true;
        app.recompute_flat_rows();
    }
    if new_session {
        let cwd = std::env::current_dir()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|_| ".".to_string());
        let name = live::generate_auto_name(&cwd, &app.live_sessions);
        app.launch_session = Some(app::LaunchRequest::NewLive { name, cwd });
    }

    // Always spawn background update check (non-blocking)
    {
        let (tx, rx) = std::sync::mpsc::channel();
        app.update_receiver = Some(rx);
        std::thread::spawn(move || {
            if let Some(info) = update::check_for_update() {
                let _ = tx.send(info);
            }
            let mut config = config::Config::load();
            // Silently ignore save failures — this is a non-critical background task
            // and eprintln would corrupt the TUI in raw/alternate screen mode
            let _ = config.mark_update_checked();
        });
    }

    loop {
        if app.needs_redraw {
            terminal.draw(|frame| ui::draw(frame, &mut app))?;
            app.needs_redraw = false;
        }

        let poll_timeout = if app.selected_live_index().is_some() {
            std::time::Duration::from_millis(250)
        } else {
            std::time::Duration::from_millis(1000)
        };

        if event::poll(poll_timeout)? {
            app.needs_redraw = true;
            app.handle_event()?;
        } else if app.selected_live_index().is_some() {
            // Periodic redraw so the live preview pane stays fresh
            app.needs_redraw = true;
        }

        // Check for background update result
        if let Some(rx) = &app.update_receiver {
            if let Ok(info) = rx.try_recv() {
                app.update_status = update::UpdateStatus::Available(info);
                app.update_receiver = None;
                app.needs_redraw = true;
                // Only show popup if user isn't in a modal (filter, rename, dir browser)
                if app.mode == app::AppMode::Normal && !app.filter_active {
                    app.mode = app::AppMode::UpdatePrompt;
                }
            }
        }

        // Check for background session names result
        if let Some(rx) = &app.names_receiver {
            if let Ok(names) = rx.try_recv() {
                app.names_receiver = None;
                app.apply_session_names(names);
                app.needs_redraw = true;
            }
        }

        if app.should_quit {
            break;
        }

        if let Some(req) = app.launch_session.take() {
            restore_terminal()?;
            match req {
                app::LaunchRequest::Resume { session_id, cwd } => {
                    let dir = if std::path::Path::new(&cwd).exists() { &cwd } else { "." };
                    let live_sessions = live::discover_live_sessions();
                    let tmux_name = live::generate_auto_name(dir, &live_sessions);
                    let claude = config::Config::load().claude_bin().to_string();
                    live::start_live_session(&tmux_name, dir, &[&claude, "--resume", &session_id])?;
                    live::attach_to_session(&tmux_name)?;
                }
                app::LaunchRequest::Direct { session_id, cwd } => {
                    let dir = if std::path::Path::new(&cwd).exists() { &cwd } else { "." };
                    let claude = config::Config::load().claude_bin().to_string();
                    std::process::Command::new(&claude)
                        .arg("--resume")
                        .arg(&session_id)
                        .current_dir(dir)
                        .status()?;
                }
                app::LaunchRequest::AttachLive { tmux_name } => {
                    live::attach_to_session(&tmux_name)?;
                }
                app::LaunchRequest::NewLive { name, cwd } => {
                    let claude = config::Config::load().claude_bin().to_string();
                    live::start_live_session(&name, &cwd, &[&claude, "--name", &name])?;
                    live::attach_to_session(&name)?;
                }
                app::LaunchRequest::NewDirect { cwd } => {
                    let dir = if std::path::Path::new(&cwd).exists() { &cwd } else { "." };
                    let claude = config::Config::load().claude_bin().to_string();
                    std::process::Command::new(&claude)
                        .current_dir(dir)
                        .status()?;
                }
            }
            // Reload sessions after returning from any launch
            if let Ok(sessions) = data::load_sessions(filter_path.as_deref()) {
                app.reload_sessions(sessions);
            }
            app.reload_live_sessions();
            *terminal = setup_terminal()?;
            app.needs_redraw = true;
        }

        if let Some(info) = app.perform_update.take() {
            restore_terminal()?;
            eprintln!("Downloading {}...", info.tag);
            match update::perform_update(&info) {
                Ok(()) => {
                    app.should_restart = true;
                    app.should_quit = true;
                }
                Err(e) => {
                    let msg = format!("{:#}", e);
                    eprintln!("Update failed: {}", msg);
                    app.update_status = update::UpdateStatus::Failed(msg);
                    std::thread::sleep(std::time::Duration::from_secs(2));
                    *terminal = setup_terminal()?;
                    app.needs_redraw = true;
                }
            }
        }
    }

    Ok(app.should_restart)
}

/// Entry point. Parses CLI arguments, loads sessions, sets up the terminal and runs the TUI.
/// On `--new` starts the TUI and immediately launches a new live session (like pressing `n`).
fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let new_session = args.iter().any(|a| a == "--new");
    let spawn_session = args.iter().any(|a| a == "--spawn");
    let live_start = args.iter().any(|a| a == "--live");
    let flat = args.iter().any(|a| a == "--flat") || live_start;
    let filter_path = args.iter().find(|a| !a.starts_with('-')).map(|arg| {
        std::fs::canonicalize(arg)
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|_| arg.clone())
    });

    if spawn_session {
        let cwd = std::env::current_dir()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|_| ".".to_string());
        let live_sessions = live::discover_live_sessions();
        let tmux_name = live::generate_auto_name(&cwd, &live_sessions);
        let exe = std::env::current_exe()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|_| "ccsm".to_string());
        let claude = config::Config::load().claude_bin().to_string();
        let shell_cmd = format!("{}; exec {}", claude, exe);
        live::start_live_session(&tmux_name, &cwd, &["sh", "-c", &shell_cmd])?;
        live::switch_to_session(&tmux_name)?;
        return Ok(());
    }

    let sessions = data::load_sessions(filter_path.as_deref())?;
    if sessions.is_empty() && !live::is_server_running() {
        if filter_path.is_some() {
            eprintln!("No Claude Code sessions found for the specified path");
        } else {
            let history_path = dirs::home_dir()
                .map(|h| h.join(".claude").join("history.jsonl"))
                .unwrap_or_else(|| PathBuf::from("~/.claude/history.jsonl"));
            eprintln!(
                "No Claude Code sessions found in {}",
                history_path.display()
            );
        }
        return Ok(());
    }

    // Install panic hook to restore terminal on panic
    let original_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic_info| {
        let _ = restore_terminal();
        original_hook(panic_info);
    }));

    let mut terminal = setup_terminal()?;
    let should_restart = run_app(&mut terminal, sessions, filter_path, flat, live_start, new_session)?;
    restore_terminal()?;

    if should_restart {
        let exe = std::env::current_exe()?;
        let args: Vec<String> = std::env::args().skip(1).collect();

        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            let err = std::process::Command::new(&exe).args(&args).exec();
            return Err(err.into());
        }

        #[cfg(windows)]
        {
            std::process::Command::new(&exe).args(&args).spawn()?;
            std::process::exit(0);
        }
    }

    Ok(())
}
