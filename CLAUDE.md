# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project

CCSM (Claude Code Session Manager) — a Rust TUI application for browsing, resuming, and managing Claude Code sessions via tmux. Built with `ratatui` + `crossterm`. Version managed via `Cargo.toml`.

## Build & Run

```sh
cargo build --release          # Release binary → target/release/ccsm
cargo build                    # Debug build
cargo test                     # Run all tests
cargo test config::tests       # Run tests in a specific module
cargo clippy                   # Lint
cargo fmt --check              # Check formatting
./install.sh                   # Build release + symlink to ~/.local/bin/ccsm
```

CLI flags: `--flat`, `--live`, `--new`, `--spawn`, or a path argument to filter sessions.

## Architecture

**Main loop** (`main.rs`): CLI parsing → session loading → terminal raw mode setup → event loop (`run_app`) with background threads for update checks and session name loading → session launch on exit.

**Core modules:**

- **`app.rs`** — Central `App` struct holding all UI state. Owns filtered/unfiltered session lists, tree/flat row computation, selection tracking, modal state (`AppMode` enum), and favorites. Key dispatch happens here.
- **`data.rs`** — Reads `~/.claude/history.jsonl` to build session list. Loads individual session JSONL files from `~/.claude/projects/{path}/{id}.jsonl`. Caches preview content (last 20 user/assistant messages). Manages custom title persistence.
- **`ui.rs`** — Renders the TUI frame: 30/70 horizontal split (session list + preview pane), info bar, status bar, and modal overlays (rename, help, update prompt, duplicate detection).
- **`keys.rs`** — Key event handlers split by modal context (rename, naming, duplicate) and normal mode navigation/actions.
- **`live.rs`** — Tmux integration using dedicated `ccsm` socket. Discovers running sessions, manages attach/detach/rename/kill, captures pane output for live preview.
- **`config.rs`** — Config struct serialized to `~/.config/ccsm/config.json`. Fields: view mode, display mode, hide_empty, group_chains, live_filter, favorites, custom binary paths.
- **`config_popup.rs`** — Config popup modal UI and event handling.
- **`update.rs`** — Background version check against GitHub Releases API (24h cooldown). Downloads platform-specific archive, replaces binary in-place, triggers auto-restart.
- **`theme.rs`** — Catppuccin Mocha color palette constants shared across UI.

**Key patterns:**
- Modal state machine via `AppMode` enum drives which key handlers and UI overlays are active
- `LaunchRequest` enum returned from the event loop tells `main.rs` what to do after terminal teardown (resume, attach, new live/direct session)
- Background work uses `mpsc` channels (update checker, session name loader)
- Shell command safety: all tmux commands use array-based execution, binary paths validated before use
- Preview caching via `HashMap` to avoid redundant JSONL parsing

## Tests

Tests live in `#[cfg(test)]` modules within `config.rs`, `app.rs`, `data.rs`, and `update.rs`. They use `tempfile` for filesystem isolation. No external test harness — just `cargo test`.

## CI/CD

`.github/workflows/release.yml`: workflow_dispatch bumps version in Cargo.toml, builds for 5 platform targets (macOS ARM64/x86_64, Linux x86_64/ARM64, Windows x86_64), creates GitHub Release with archived binaries.
