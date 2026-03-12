mod app;
mod config;
mod data;
mod ui;

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
) -> Result<()> {
    let config = config::Config::load();
    let mut app = App::new(sessions, filter_path.clone(), config);
    if flat {
        app.tree_view = false;
    }

    loop {
        terminal.draw(|frame| ui::draw(frame, &mut app))?;

        if event::poll(std::time::Duration::from_millis(100))? {
            app.handle_event()?;
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
    }

    Ok(())
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
            eprintln!("No Claude Code sessions found in ~/.claude/history.jsonl");
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
    let result = run_app(&mut terminal, sessions, filter_path, flat);
    restore_terminal()?;
    result
}
