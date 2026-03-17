# ccsm — Claude Code Session Manager

A terminal UI for browsing your Claude Code session history, previewing conversations, resuming or starting sessions in their original working directory, and managing live tmux-backed Claude sessions.

## Screenshots

### Tree View with Session Preview
Sessions grouped by project with an expanded group showing individual sessions. The right pane displays a scrollable conversation preview with the session's working directory and git branch in the info bar.

![Tree view with session preview](screenshots/session-tree-view.png)

## Features

- **Tree view** (default) — sessions grouped by project, collapsed on startup, expand/collapse with arrow keys
- **Flat view** — all sessions in a single sorted list with project name, date, and message count
- **Display modes** — cycle through Name, Short Dir, and Full Dir labels for project groups in tree view
- Shows a scrollable preview of conversation messages (last 20 turns)
- **Session info bar** — displays working directory and git branch for the selected session; shows the project directory even when a header row is selected with no active session
- Resume any session via tmux (`Enter`) or directly in the foreground without tmux (`Shift+Enter`)
- **Live sessions** — start and manage tmux-backed Claude sessions; running sessions appear at the top of the list with a live indicator
- **New session** — start a new live tmux session in the selected project's directory (`n`, prompts for a name); or start a foreground claude session directly in the selected directory (`Shift+N`, no tmux); or bypass the TUI entirely with `ccsm --new` to start a session in the current directory
- **Live-only filter** — toggle with `l` to show only running sessions; persisted in config
- **Stop live session** — kill the selected running session with `x`
- Search and filter sessions by project name or path
- Toggle visibility of empty sessions (no data file) with `e`
- **Session grouping** — toggle with `c` to group chained sessions (sequences where each was started from the previous)
- Lazy-loads and caches session previews for fast navigation
- **Live session preview** — shows live tmux pane output for running sessions
- **Auto-update** — checks GitHub Releases in the background on startup (every 24h), shows a centered prompt with current vs new version, and self-updates the binary on confirm
- **Session names** — custom titles loaded in the background for fast startup
- **Help overlay** — press `?` for a full in-app keybinding reference
- **Favorites** — pin projects to the top of the list with `f`; shown with a ★ indicator; persisted in config
- **Persistent config** — view mode, display mode, hide-empty, session grouping, live filter preference, favorites, and update check timestamp saved to `~/.config/ccsm/config.json`
- Optional path argument to scope sessions to a specific directory
- Version label displayed in the bottom-right of the help bar
- Catppuccin Mocha-inspired color theme

## Requirements

- **macOS** (ARM64, x86_64), **Linux** (x86_64, ARM64), or **Windows** (x86_64)
- [Claude Code CLI](https://docs.anthropic.com/en/docs/claude-code) installed and on your `PATH`
- Existing session history in `~/.claude/`
- `tmux` installed for live session support (optional — history browsing works without it)
  - **macOS:** `brew install tmux` (requires [Homebrew](https://brew.sh))
  - **Linux:** `sudo apt install tmux` / `sudo dnf install tmux` / your distro's package manager

## Install

### Quick Install (pre-built binary)

**macOS / Linux:**

```sh
curl -fsSL https://raw.githubusercontent.com/faulker/ccsm/main/remote-install.sh | bash
```

This downloads the latest release binary from GitHub and installs it to `~/.local/bin/ccsm`. Make sure `~/.local/bin` is in your `PATH`.

**Windows (PowerShell):**

```powershell
irm https://raw.githubusercontent.com/faulker/ccsm/main/remote-install.ps1 | iex
```

This downloads the latest release and installs `ccsm.exe` to `%LOCALAPPDATA%\ccsm`, adding it to your user `PATH`.

### Build from Source

```sh
./install.sh
```

This builds a release binary and symlinks it to `~/.local/bin/ccsm`. Requires Rust 1.75+.

## Build

```sh
cargo build --release
```

The binary will be at `target/release/ccsm`.

## Run

```sh
cargo run --release
# or
./target/release/ccsm
```

Optionally pass a path to show only sessions from that directory:

```sh
./target/release/ccsm ~/projects/my-app
```

Use `--flat` to start in flat view instead of the default grouped tree view:

```sh
./target/release/ccsm --flat
./target/release/ccsm --flat ~/projects/my-app
```

Use `--live` to start directly in live-only filter mode (implies `--flat`), showing only running tmux sessions:

```sh
./target/release/ccsm --live
```

Use `--new` to immediately start a new live Claude session in the current directory and attach to it, without opening the TUI. After Claude exits, ccsm relaunches automatically:

```sh
./target/release/ccsm --new
# or from anywhere:
ccsm --new
```

## Key Bindings

| Key | Action |
|---|---|
| `j` / `↓` | Next session |
| `k` / `↑` | Previous session |
| `→` | Expand group (tree view) |
| `←` | Collapse group or jump to parent header (tree view) |
| `Enter` | Resume session in tmux / attach to live session / toggle group |
| `Shift+Enter` | Resume historical session directly in the foreground (no tmux) |
| `Tab` / `Shift+Tab` | Cycle: tree [name] → tree [short dir] → tree [full dir] → flat → tree [name] |
| `Shift+J` | Scroll preview down |
| `Shift+K` | Scroll preview up |
| `/` | Activate search/filter mode |
| `c` | Toggle session grouping (group/ungroup related sessions) |
| `e` | Toggle show/hide empty sessions |
| `f` | Toggle favorite — pins project to top of list (shown with ★) |
| `n` | Start new live session in selected project's directory (prompts for name) |
| `Shift+N` | Start new foreground claude session in selected project's directory (no tmux) |
| `l` | Toggle live-only filter (show only running sessions) |
| `r` | Rename selected session or live session |
| `x` | Stop (kill) selected live session |
| `?` | Open help overlay |
| `q` / `Esc` / `Ctrl+C` | Quit |

### Update Prompt

When an update is available, a centered dialog appears:

| Key | Action |
|---|---|
| `y` | Download and install the update |
| `n` / `Esc` | Dismiss until next run |

### Filter Mode

When filter mode is active (triggered by `/`):

| Key | Action |
|---|---|
| Type characters | Filter sessions by project name or path (case-insensitive) |
| `↓` / `↑` | Navigate results (stays in filter mode) |
| `Enter` | Exit filter mode (keeps filter active) |
| `Backspace` | Delete last character |
| `Esc` | Clear filter text and exit filter mode |

### Session Naming

When naming a new live session (after selecting a directory):

| Key | Action |
|---|---|
| Type characters | Enter session name (placeholder shown if left blank) |
| `Enter` | Confirm name and launch session |
| `Esc` | Cancel |

### Live Session tmux Keybindings

While attached to a live session in tmux:

| Key | Action |
|---|---|
| `Ctrl+\` | Detach and return to ccsm |
| `Ctrl+n` | Switch to next live session |
| `Ctrl+p` | Switch to previous live session |

## Live Sessions

Live sessions are tmux-backed Claude Code sessions managed through a dedicated tmux server (`ccsm` socket). They appear at the top of the session list with a green `●` indicator.

- **Start**: press `n` (starts a named live tmux session in the current project dir) or `Shift+N` (starts claude directly in the foreground, no tmux)
- **Attach**: press `Enter` on any live session to attach
- **Detach**: press `Ctrl+\` inside a live session to return to ccsm
- **Navigate**: use `Ctrl+n` / `Ctrl+p` to cycle between live sessions without detaching
- **Stop**: press `x` to gracefully kill the selected live session
- **Rename**: press `r` on a live session to rename the tmux window
- **Filter**: press `l` to hide history and show only running sessions

The tmux server uses a custom config at `~/.config/ccsm/tmux.conf` with a status bar showing the available keybindings. Requires `tmux` to be installed.

## Configuration

Settings are persisted to `~/.config/ccsm/config.json` and automatically saved when changed:

```json
{
  "tree_view": true,
  "display_mode": "name",
  "hide_empty": true,
  "group_chains": true,
  "live_filter": false,
  "favorites": [],
  "last_update_check": 1710200000
}
```

| Field | Values | Description |
|---|---|---|
| `tree_view` | `true` / `false` | Start in tree or flat view |
| `display_mode` | `"name"`, `"short_dir"`, `"full_dir"` | How project groups are labeled in tree view |
| `hide_empty` | `true` / `false` | Whether to hide sessions with no data file |
| `group_chains` | `true` / `false` | Whether to group chained (parent → child) sessions |
| `live_filter` | `true` / `false` | Whether to show only running live sessions |
| `favorites` | Array of paths | Project directories pinned to the top of the list |
| `last_update_check` | Unix timestamp | When the last update check was performed (auto-managed) |

## How It Works

1. Reads `~/.claude/history.jsonl` to build a list of sessions with project paths and timestamps
2. On selection, loads the session file from `~/.claude/projects/{path}/{sessionId}.jsonl`
3. Extracts session metadata (working directory, git branch) and displays it in an info bar
4. Filters to user/assistant messages and displays the last 20 turns as a preview
5. On startup, spawns a background thread to check GitHub Releases for newer versions (respects 24h cooldown)
6. Session custom titles are loaded in the background to avoid blocking startup
7. On `Enter` (history session), wraps the resume in a new tmux live session and attaches to it; on `Shift+Enter`, runs `claude --resume <id>` directly in the foreground without tmux; on return, sessions are reloaded
8. On `Enter` (live session), attaches to the tmux session and suspends the TUI; detach with `Ctrl+\` to return
9. On `n`/`N`, prompts for a session name then starts a new detached tmux session running `claude` in the chosen directory and attaches to it; uses a dedicated tmux server (`-L ccsm`) with a custom status bar
10. With `--new`, skips the TUI, creates a live tmux session in the current directory, attaches immediately, and re-execs ccsm when Claude exits
11. If the user accepts an update, the TUI suspends, downloads the new binary, replaces the current executable, and resumes
12. After Claude exits, the TUI resumes and reloads the session list

## Dependencies

- `ratatui` — TUI rendering framework
- `crossterm` — terminal backend and event handling
- `serde` / `serde_json` — JSON parsing
- `dirs` — home directory and config directory detection
- `chrono` — relative timestamp formatting
- `anyhow` — error handling
- `ureq` — lightweight HTTP client for GitHub Releases API
- `flate2` — gzip decompression for release archives
- `tar` / `zip` — archive extraction for release downloads
- `tempfile` — temporary directories for safe binary replacement
- `unicode-width` — correct text width calculation for multi-byte characters

## Tests

```sh
cargo test
```
