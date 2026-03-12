# ccsm — Claude Code Session Manager

A terminal UI for browsing your Claude Code session history, previewing conversations, and resuming or starting sessions in their original working directory.

## Features

- **Tree view** (default) — sessions grouped by project, collapsed on startup, expand/collapse with arrow keys
- **Flat view** — all sessions in a single sorted list with project name, date, and message count
- **Display modes** — cycle through Name, Short Dir, and Full Dir labels for project groups in tree view
- Shows a scrollable preview of conversation messages (last 20 turns)
- **Session info bar** — displays working directory and git branch for the selected session
- Resume any session directly — opens `claude --resume <id>` in the original project directory
- **New session** — launch a new Claude session in the selected project's directory (`n`) or browse to any directory (`N`)
- **Directory browser** — full overlay for navigating the filesystem, with path input and directory listing
- Search and filter sessions by project name or path
- Toggle visibility of empty sessions (no data file) with `e`
- Lazy-loads and caches session previews for fast navigation
- **Persistent config** — view mode, display mode, and hide-empty preference saved to `~/.config/ccsm/config.json`
- Optional path argument to scope sessions to a specific directory
- Catppuccin Mocha-inspired color theme

## Requirements

- Rust 1.75+
- [Claude Code CLI](https://docs.anthropic.com/en/docs/claude-code) installed and on your `PATH`
- Existing session history in `~/.claude/`

## Install

```sh
./install.sh
```

This builds a release binary and symlinks it to `~/.local/bin/ccsm`. Make sure `~/.local/bin` is in your `PATH`.

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

## Key Bindings

| Key | Action |
|---|---|
| `j` / `↓` | Next session |
| `k` / `↑` | Previous session |
| `l` / `→` | Expand group (tree view) |
| `h` / `←` | Collapse group (tree view) |
| `Enter` | Resume session / toggle group |
| `Tab` | Cycle: tree [name] → tree [short dir] → tree [full dir] → flat → tree [name] |
| `J` (shift) | Scroll preview down |
| `K` (shift) | Scroll preview up |
| `/` | Activate search/filter mode |
| `e` | Toggle show/hide empty sessions |
| `n` | New Claude session in selected project's directory |
| `N` (shift) | Open directory browser to start a new session anywhere |
| `q` / `Esc` / `Ctrl+C` | Quit |

### Filter Mode

When filter mode is active (triggered by `/`):

| Key | Action |
|---|---|
| Type characters | Filter sessions by project name or path (case-insensitive) |
| `↓` / `↑` | Navigate results (stays in filter mode) |
| `Enter` | Exit filter mode (keeps filter active) |
| `Backspace` | Delete last character |
| `Esc` | Clear filter text and exit filter mode |

### Directory Browser

When the directory browser is open (triggered by `N`):

| Key | Action |
|---|---|
| `↑` / `↓` | Navigate directory listing |
| `Enter` | Enter selected directory |
| `Space` | Select current directory and launch new session |
| `/` | Type a path directly |
| `Esc` | Cancel and close browser |

## Layout

### Tree View (default)
```
┌─ Sessions [tree] [name] ────┬─ Preview ─────────────────────────┐
│ ▶ ▸ my-project (3)          │  ~/Dev/my-project  ⎇ main        │
│   ▸ other-project (2)       │───────────────────────────────────│
│   ...                       │ ▎ USER:                           │
│                              │ can you update the readme         │
│                              │                                   │
│                              │ ▎ ASSISTANT:                     │
│                              │ Sure, let me read the file...     │
├──────────────────────────────┴───────────────────────────────────┤
│ ↑↓/jk navigate  Enter open  J/K scroll  / search  Tab view      │
│ e show empty  n new  N browse  q quit                            │
└──────────────────────────────────────────────────────────────────┘
```

### Flat View
```
┌─ Sessions [flat] ───────────┬─ Preview ─────────────────────────┐
│ ▶ my-project        2h ago  │ ▎ USER:                           │
│   other-project     3d ago  │ can you update the readme         │
│   ...                       │                                   │
├─────────────────────────────┴───────────────────────────────────┤
│ ↑↓/jk navigate  Enter open  J/K scroll  / search  Tab view     │
│ e show empty  n new  N browse  q quit                           │
└─────────────────────────────────────────────────────────────────┘
```

## Configuration

Settings are persisted to `~/.config/ccsm/config.json` and automatically saved when changed:

```json
{
  "tree_view": true,
  "display_mode": "name",
  "hide_empty": true
}
```

| Field | Values | Description |
|---|---|---|
| `tree_view` | `true` / `false` | Start in tree or flat view |
| `display_mode` | `"name"`, `"short_dir"`, `"full_dir"` | How project groups are labeled in tree view |
| `hide_empty` | `true` / `false` | Whether to hide sessions with no data file |

## How It Works

1. Reads `~/.claude/history.jsonl` to build a list of sessions with project paths and timestamps
2. On selection, loads the session file from `~/.claude/projects/{path}/{sessionId}.jsonl`
3. Extracts session metadata (working directory, git branch) and displays it in an info bar
4. Filters to user/assistant messages and displays the last 20 turns as a preview
5. On Enter, suspends the TUI and runs `claude --resume <id>` in the session's original directory
6. On `n`/`N`, launches a new `claude` session in the chosen directory
7. After Claude exits, the TUI resumes

## Dependencies

- `ratatui` — TUI rendering framework
- `crossterm` — terminal backend and event handling
- `serde` / `serde_json` — JSON parsing
- `dirs` — home directory and config directory detection
- `chrono` — relative timestamp formatting
- `anyhow` — error handling

## Tests

```sh
cargo test
```
