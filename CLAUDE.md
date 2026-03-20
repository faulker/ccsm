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

**Core modules** (3 directory-based, 5 single-file):

### `src/app/` — Application state & logic
Central `App` struct holding all UI state. Each sub-file adds `impl App` methods for a specific domain:

| File | Concern |
|------|---------|
| `mod.rs` | `App` struct, enums (`TreeRow`, `FlatRow`, `AppMode`, `LaunchRequest`, `DuplicateSource`), `new()`, `spawn_load_session_names()`, `apply_session_names()`, `reload_sessions()`, `save_config()` |
| `tree.rs` | `init_tree()`, `recompute_tree()` — tree-view row computation |
| `flat.rs` | `recompute_flat_rows()` — flat-view row computation |
| `filter.rs` | `recompute_filter()` — filter text + hide-empty + chain grouping logic |
| `selection.rs` | `visible_item_count()`, `selected_session_index()`, `is_historical_selected()`, `selected_live_index()`, `selected_cwd()`, `toggle_favorite()` |
| `chain.rs` | `chain_name_for()`, `resume_session_id_for()`, `chain_entry_count()` |
| `display.rs` | `display_name()`, `cycle_view_forward()`, `cycle_view_backward()` |
| `preview.rs` | `current_preview()`, `current_live_preview()` |
| `activity.rs` | `total_activity_counts()`, `project_activity_counts()`, `reload_live_sessions()`, `poll_all_activity()` |
| `tests.rs` | All `#[cfg(test)]` tests |

### `src/data/` — Session data I/O
Reads `~/.claude/history.jsonl` and individual session JSONL files from `~/.claude/projects/{path}/{id}.jsonl`.

| File | Concern |
|------|---------|
| `mod.rs` | Re-exports public types and functions |
| `types.rs` | `SessionInfo`, `SessionMeta`, `PreviewMessage`, and all private deserialization structs |
| `io.rs` | `project_to_dir_name()`, `session_file_path()`, `format_session_boundary_date()` |
| `history.rs` | `load_sessions()`, `read_session_meta()`, `strip_xml_tags()` |
| `preview.rs` | `load_session_messages()`, `load_chain_preview()`, `load_preview()` |
| `titles.rs` | `load_custom_title()`, `save_custom_title()` |
| `tests.rs` | All `#[cfg(test)]` tests |

### `src/ui/` — TUI rendering
Renders the TUI frame: 30/70 horizontal split (session list + preview pane), info bar, status bar, and modal overlays.

| File | Concern |
|------|---------|
| `mod.rs` | Top-level `draw()` orchestrator — delegates to sub-modules |
| `session_list.rs` | `build_tree_items()`, `build_flat_items()` — session list `ListItem` construction |
| `preview_pane.rs` | `build_preview_text()`, `build_live_preview_text()` — preview pane content |
| `info_bar.rs` | `build_title_spans()`, `render_status_bar()` — title bar, status/help bar |
| `ansi.rs` | `parse_ansi_line()`, `apply_sgr()` — ANSI escape sequence parsing |
| `modals.rs` | `draw_naming_popup()`, `draw_duplicate_popup()`, `draw_rename_popup()`, `draw_update_prompt()`, `render_help_popup()` |
| `util.rs` | `format_relative_date()`, `estimate_wrapped_height()`, `centered_rect()`, `truncate()`, `truncate_left()`, `truncate_left_plain()`, `activity_count_spans()`, `live_dot_style()` |

### Single-file modules

- **`keys.rs`** — Key event handlers split by modal context (rename, naming, duplicate) and normal mode navigation/actions.
- **`live.rs`** — Tmux integration using dedicated `ccsm` socket. Discovers running sessions, manages attach/detach/rename/kill, captures pane output for live preview.
- **`config.rs`** — Config struct serialized to `~/.config/ccsm/config.json`. Fields: view mode, display mode, hide_empty, group_chains, live_filter, favorites, custom binary paths.
- **`config_popup.rs`** — Config popup modal UI and event handling.
- **`update.rs`** — Background version check against GitHub Releases API (24h cooldown). Downloads platform-specific archive, replaces binary in-place, triggers auto-restart.
- **`theme.rs`** — Catppuccin Mocha color palette constants shared across UI.

### Key patterns
- Modal state machine via `AppMode` enum drives which key handlers and UI overlays are active
- `LaunchRequest` enum returned from the event loop tells `main.rs` what to do after terminal teardown (resume, attach, new live/direct session)
- Directory modules use `use super::*` in sub-files — each adds `impl App`/`impl` blocks without duplicating the struct
- Background work uses `mpsc` channels (update checker, session name loader)
- Shell command safety: all tmux commands use array-based execution, binary paths validated before use
- Preview caching via `HashMap` to avoid redundant JSONL parsing

## Tests

Tests live in `#[cfg(test)]` modules: `app/tests.rs`, `data/tests.rs`, `config.rs`, and `update.rs`. They use `tempfile` for filesystem isolation. No external test harness — just `cargo test`.

## CI/CD

`.github/workflows/release.yml`: workflow_dispatch bumps version in Cargo.toml, builds for 5 platform targets (macOS ARM64/x86_64, Linux x86_64/ARM64, Windows x86_64), creates GitHub Release with archived binaries.
