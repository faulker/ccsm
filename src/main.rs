mod app;
mod data;
mod ui;

use anyhow::Result;
use app::App;
use crossterm::{
    event,
    terminal::{self, EnterAlternateScreen, LeaveAlternateScreen},
    ExecutableCommand,
};
use ratatui::prelude::CrosstermBackend;
use ratatui::Terminal;
use std::io;

fn setup_terminal() -> Result<Terminal<CrosstermBackend<io::Stdout>>> {
    terminal::enable_raw_mode()?;
    io::stdout().execute(EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(io::stdout());
    let terminal = Terminal::new(backend)?;
    Ok(terminal)
}

fn restore_terminal() -> Result<()> {
    terminal::disable_raw_mode()?;
    io::stdout().execute(LeaveAlternateScreen)?;
    Ok(())
}

fn run_app(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    sessions: Vec<data::SessionInfo>,
    filter_path: Option<String>,
) -> Result<()> {
    let mut app = App::new(sessions, filter_path);

    loop {
        terminal.draw(|frame| ui::draw(frame, &mut app))?;

        if event::poll(std::time::Duration::from_millis(100))? {
            app.handle_event()?;
        }

        if app.should_quit {
            break;
        }

        if let Some((session_id, cwd)) = app.launch_session.take() {
            restore_terminal()?;
            App::launch_claude(&session_id, &cwd)?;
            *terminal = setup_terminal()?;
        }
    }

    Ok(())
}

fn main() -> Result<()> {
    let filter_path = std::env::args().nth(1).map(|arg| {
        std::fs::canonicalize(&arg)
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or(arg)
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
    let result = run_app(&mut terminal, sessions, filter_path);
    restore_terminal()?;
    result
}
