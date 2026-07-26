# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project

CCSM (Claude Code Session Manager), a Rust TUI application for browsing, resuming, and managing Claude Code sessions via tmux. Built with `ratatui` + `crossterm`. Version managed via `Cargo.toml`.

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

CLI flags: `--flat`, `--live`, `--new`, `--spawn`, `--watch`, `--watch-status`, or a path argument to filter sessions.

## Architecture

**Main loop** (`main.rs`): CLI parsing → session loading → terminal raw mode setup → event loop (`run_app`) with background threads for update checks and session name loading → session launch on exit.

**Core modules** (4 directory-based, 7 single-file):

### `src/app/`: Application state & logic
Central `App` struct holding all UI state. Each sub-file adds `impl App` methods for a specific domain:

| File | Concern |
|------|---------|
| `mod.rs` | `App` struct, enums (`TreeRow`, `FlatRow`, `AppMode`, `MainTab`, `HelpTab`, `LaunchRequest`, `DuplicateSource`), `new()`, `spawn_load_session_names()`, `apply_session_names()`, `reload_sessions()`, `save_config()` |
| `tree.rs` | `init_tree()`, `recompute_tree()`: tree-view row computation |
| `flat.rs` | `recompute_flat_rows()`: flat-view row computation |
| `filter.rs` | `recompute_filter()`: filter text + hide-empty + chain grouping logic |
| `selection.rs` | `visible_item_count()`, `selected_session_index()`, `is_historical_selected()`, `selected_live_index()`, `selected_cwd()`, `toggle_favorite()` |
| `chain.rs` | `chain_name_for()`, `resume_session_id_for()`, `chain_entry_count()` |
| `jobs.rs` | Scheduler job state for the TUI: `reload_schedule()`, `poll_schedule_changed()`, `enqueue_command()`, `submit_job_form()`, `toggle_watcher()`, `stop_selected_live_session()`, tab switching (`open_jobs_tab()`, `cycle_main_tab()`) |
| `dir_browser.rs` | `DirBrowser` filesystem picker (`PickerKind` directory/file, `PickerTarget` routing) used for new-session cwds, job cwds, and the config popup's binary paths |
| `display.rs` | `display_name()`, `cycle_view_forward()`, `cycle_view_backward()` |
| `preview.rs` | `current_preview()`, `current_live_preview()` |
| `activity.rs` | `total_activity_counts()`, `project_activity_counts()`, `reload_live_sessions()`, `poll_all_activity()` |
| `tests.rs` | All `#[cfg(test)]` tests |

### `src/data/`: Session data I/O
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

### `src/ui/`: TUI rendering
Renders the TUI frame: 30/70 horizontal split (session list + preview pane), info bar, status bar, and modal overlays.

| File | Concern |
|------|---------|
| `mod.rs` | Top-level `draw()` orchestrator: delegates to sub-modules; owns `render_tab_bar()` (tab strip + right-aligned usage chip) |
| `session_list.rs` | `build_tree_items()`, `build_flat_items()`: session list `ListItem` construction |
| `preview_pane.rs` | `build_preview_text()`, `build_live_preview_text()`: preview pane content |
| `info_bar.rs` | `build_title_spans()`, `build_usage_status_spans()`, `render_status_bar()`: title bar, usage chip, status/help bar |
| `ansi.rs` | `parse_ansi_line()`, `apply_sgr()`: ANSI escape sequence parsing |
| `modals.rs` | `draw_naming_popup()`, `draw_duplicate_popup()`, `draw_rename_popup()`, `draw_update_prompt()`, `render_help_popup()` (tabbed: Sessions/Jobs/General) |
| `jobs_tab.rs` | Jobs tab list + detail panes, the job form and confirm modals, and their `impl App` key handlers |
| `util.rs` | `input_spans()`/`input_spans_with_placeholder()` (shared text-cursor rendering), `format_relative_date()`, `estimate_wrapped_height()`, `centered_rect()`, `truncate()`, `truncate_left()`, `truncate_left_plain()`, `activity_count_spans()`, `live_dot_style()` |

### `src/schedule/`: Usage-aware job scheduler
Persistent job model plus the decision logic the `watch.rs` daemon executes. State lives in `ccsm_dir()` (`schedule.json`, `watch_state.json`, `commands/`, `watch.log`), separate from `config.json`.

| File | Concern |
|------|---------|
| `mod.rs` | `Job`, `JobState`, `JobEvent`, `Schedule`, `Job::transition()`, `canonical_cwd()`, `discover_session_id()` |
| `store.rs` | `load()`, `load_or_quarantine()`, `save()`, `write_atomic()`, `WatchState`, `Stamp` change detection |
| `command.rs` | `Command`, `JobPatch`, `enqueue()`, `read_pending()`, `ack()`, `pending_count()` |
| `engine.rs` | **Pure** `plan()` returning `Vec<Action>`, plus `build_start_argv()`, `build_resume_argv()`, `backoff_ms()` |
| `tests.rs` | Decision-table tests plus store/command coverage |

### Single-file modules

- **`keys.rs`**: Key event handlers split by modal context (rename, naming, duplicate) and normal mode navigation/actions.
- **`live.rs`**: Tmux integration using dedicated `ccsm` socket. Discovers running sessions, manages attach/detach/rename/kill, captures pane output for live preview.
- **`config.rs`**: Config struct serialized to `~/.config/ccsm/config.json`. Fields: view mode, display mode, hide_empty, group_chains, live_filter, favorites, custom binary paths.
- **`config_popup.rs`**: Config popup modal UI and event handling.
- **`update.rs`**: Background version check against GitHub Releases API (24h cooldown). Downloads platform-specific archive, replaces binary in-place, triggers auto-restart.
- **`usage.rs`**: Parses `claude-usage --format json`. Pure `parse()`/`reset_at_ms()`/`is_fresh()` plus a shelling-out `fetch()`.
- **`watch.rs`**: The `ccsm --watch` daemon. Owns all job state, runs a 1s loop (drain commands, reconcile tmux, poll activity, adaptive usage fetch, `engine::plan`, execute, persist). Lives in its own `ccsm-watch` tmux session.
- **`theme.rs`**: Catppuccin Mocha color palette constants shared across UI.

### Key patterns
- Two top-level tabs (`MainTab::Sessions` / `MainTab::Jobs`) share the 30/70 list + detail layout; the Jobs tab lives in Normal mode (it is not an `AppMode`), so `keys.rs` dispatches to `handle_jobs_tab_event` before the Sessions bindings
- **Cross-tab status belongs in the tab strip**, not in a tab's list title. The usage chip renders once in `render_tab_bar`, and `main.rs` polls `poll_schedule_changed()` on every tick regardless of tab so it cannot freeze on whatever was on disk when the Sessions tab opened
- Modal state machine via `AppMode` enum drives which key handlers and UI overlays are active
- `LaunchRequest` enum returned from the event loop tells `main.rs` what to do after terminal teardown (resume, attach, new live/direct session)
- Directory modules use `use super::*` in sub-files, each adds `impl App`/`impl` blocks without duplicating the struct
- Background work uses `mpsc` channels (update checker, session name loader)
- Shell command safety: all tmux commands use array-based execution, binary paths validated before use
- **Single-writer job state**: the `watch.rs` daemon is the only writer of `schedule.json`. The TUI never writes it; it enqueues command files that the daemon drains. This is what removes the need for file locking, so do not add a second writer.
- **Pure planning, effectful execution**: `engine::plan()` performs zero I/O and is exhaustively unit tested; every side effect lives in `watch::execute_action`. Keep that split.
- **tmux exact targeting differs by command type**: session-scoped commands (`has-session`, `kill-session`, `rename-session`, `list-clients`) take `=name`; pane-scoped commands (`send-keys`, `capture-pane`, `paste-buffer`, `display-message`, `set-option`) take `=name:` with a trailing colon. Use `session_target()` / `pane_target()` in `live.rs` accordingly. Getting this wrong fails loudly except for `display-message`, which silently returns an empty string.
- **Copy mode swallows `send-keys` but not `paste-buffer`**, so guard key sends with `pane_in_mode()` + `cancel_copy_mode()`. Pastes also concatenate with existing input, so clear the line with `clear_input_line()` first.
- **Usage staleness is asymmetric**: a stale sample may trigger a pause but never a resume. The retained snapshot is aged via `watch::aged_usage` before use so holding it cannot fake freshness.
- **One picker, many targets**: `PickerTarget` decides what kind of path is valid, where the committed path is written, and which mode the picker returns to. Add new browsable path fields by extending that enum, not by cloning the picker.
- **Text fields render through `ui::util::input_spans`**: `tui_input` already handles Left/Right/word/Home/End/Ctrl+U, so a field that looks like it can't move its cursor is a *rendering* bug. Never draw a trailing `|` in place of the real cursor.
- Preview caching via `HashMap` to avoid redundant JSONL parsing

## Tests

Tests live in `#[cfg(test)]` modules: `app/tests.rs`, `data/tests.rs`, `schedule/tests.rs`, `config.rs`, `live.rs`, `usage.rs`, `watch.rs`, `config_popup.rs`, `ui/jobs_tab.rs`, `ui/util.rs`, and `update.rs`. They use `tempfile` for filesystem isolation. No external test harness: just `cargo test`.

Tests that set `$CCSM_CONFIG_DIR` must hold `config::test_lock()` for their duration and clear the var afterwards, since env vars are process-global and would otherwise corrupt tests running in parallel.

Note the repo is **not** rustfmt-formatted (there is pre-existing drift). Run `cargo fmt --check` if you like, but do not run `cargo fmt`: it reformats ~24 unrelated files and swamps the diff.

## CI/CD

`.github/workflows/release.yml`: workflow_dispatch bumps version in Cargo.toml, builds for 5 platform targets (macOS ARM64/x86_64, Linux x86_64/ARM64, Windows x86_64), creates GitHub Release with archived binaries.

## Model Selection

- **Claude Fable 5** (`claude-fable-5`): concurrency bugs in the event loop and `mpsc` background threads, tmux integration and shell-command safety in `live.rs`, and the self-update binary-replace path in `update.rs`.
- **Claude Opus 4.8** (`claude-opus-4-8`): default for new UI features spanning `app/`, `ui/`, and `keys.rs`, or new session data sources in `data/`.
- **Claude Sonnet 5** (`claude-sonnet-5`): single-module edits, ANSI/format tweaks, and adding or updating `#[cfg(test)]` tests.
- **Claude Haiku 4.5** (`claude-haiku-4-5`): README updates, help-text/docs, boilerplate, and quick lookups.
