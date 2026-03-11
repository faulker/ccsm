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

fn main() -> Result<()> {
    let sessions = data::load_sessions()?;
    if sessions.is_empty() {
        eprintln!("No Claude Code sessions found in ~/.claude/history.jsonl");
        return Ok(());
    }

    let mut terminal = setup_terminal()?;
    let mut app = App::new(sessions);

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
            terminal = setup_terminal()?;
        }
    }

    restore_terminal()?;
    Ok(())
}
