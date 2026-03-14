mod app;
mod config;
mod data;
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

fn restore_terminal() -> Result<()> {
    let _ = io::stdout().execute(PopKeyboardEnhancementFlags);
    terminal::disable_raw_mode()?;
    io::stdout().execute(LeaveAlternateScreen)?;
    Ok(())
}

fn run_app(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    sessions: Vec<data::SessionInfo>,
    filter_path: Option<String>,
    flat: bool,
) -> Result<bool> {
    let config = config::Config::load();
    let mut app = App::new(sessions, filter_path.clone(), config);
    if flat {
        app.tree_view = false;
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
        terminal.draw(|frame| ui::draw(frame, &mut app))?;

        if event::poll(std::time::Duration::from_millis(100))? {
            app.handle_event()?;
        }

        // Check for background update result
        if let Some(rx) = &app.update_receiver {
            if let Ok(info) = rx.try_recv() {
                app.update_status = update::UpdateStatus::Available(info);
                app.update_receiver = None;
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
            }
        }

        if app.should_quit {
            break;
        }

        if let Some(req) = app.launch_session.take() {
            restore_terminal()?;
            match req {
                app::LaunchRequest::Resume { session_id, cwd } => {
                    App::launch_claude(&session_id, &cwd)?;
                }
                app::LaunchRequest::New { cwd } => {
                    App::launch_claude_new(&cwd)?;
                }
            }
            // Reload sessions so the just-finished session appears in the list
            if let Ok(sessions) = data::load_sessions(filter_path.as_deref()) {
                app.reload_sessions(sessions);
            }
            *terminal = setup_terminal()?;
        }

        if let Some(info) = app.perform_update.take() {
            restore_terminal()?;
            eprintln!("Downloading {}...", info.tag);
            match update::perform_update(&info) {
                Ok(()) => {
                    app.update_status =
                        update::UpdateStatus::Done(info.tag.clone());
                    *terminal = setup_terminal()?;
                    app.mode = app::AppMode::RestartPrompt;
                }
                Err(e) => {
                    let msg = format!("{:#}", e);
                    eprintln!("Update failed: {}", msg);
                    app.update_status = update::UpdateStatus::Failed(msg);
                    std::thread::sleep(std::time::Duration::from_secs(2));
                    *terminal = setup_terminal()?;
                }
            }
        }
    }

    Ok(app.should_restart)
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let flat = args.iter().any(|a| a == "--flat");
    let filter_path = args.iter().find(|a| !a.starts_with('-')).map(|arg| {
        std::fs::canonicalize(arg)
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|_| arg.clone())
    });

    let sessions = data::load_sessions(filter_path.as_deref())?;
    if sessions.is_empty() {
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
    let should_restart = run_app(&mut terminal, sessions, filter_path, flat)?;
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
