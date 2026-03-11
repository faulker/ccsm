# ccsm — Claude Code Session Manager

A terminal UI for browsing your Claude Code session history, previewing conversations, and resuming sessions in their original working directory.

## Features

- **Tree view** (default) — sessions grouped by project, collapsed on startup, expand/collapse with arrow keys
- **Flat view** — all sessions in a single sorted list with project name, date, and message count
- Toggle between views with `Tab`, or start in flat view with `--flat`
- Shows a scrollable preview of conversation messages (last 20 turns)
- Resume any session directly — opens `claude --resume <id>` in the original project directory
- Search and filter sessions by project name or path
- Lazy-loads and caches session previews for fast navigation
- Optional path argument to scope sessions to a specific directory

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
| `Tab` | Toggle tree/flat view |
| `J` (shift) | Scroll preview down |
| `K` (shift) | Scroll preview up |
| `/` | Activate search/filter mode |
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

## Layout

### Tree View (default)
```
┌─ Sessions [tree] ─────────────┬─ Preview ────────────────────────┐
│ ▶ ▸ my-project (3)            │ USER:                            │
│   ▸ other-project (2)         │ can you update the readme         │
│   ...                         │                                   │
│                               │ ASSISTANT:                        │
│                               │ Sure, let me read the file...     │
├───────────────────────────────┴───────────────────────────────────┤
│ ↑↓/jk navigate  Enter open  J/K scroll  / search  Tab tree  q quit│
└───────────────────────────────────────────────────────────────────┘
```

### Flat View
```
┌─ Sessions [flat] ─────────────┬─ Preview ────────────────────────┐
│ ▶ my-project         2h ago   │ USER:                            │
│   other-project      3d ago   │ can you update the readme         │
│   ...                         │                                   │
├───────────────────────────────┴───────────────────────────────────┤
│ ↑↓/jk navigate  Enter open  J/K scroll  / search  Tab tree  q quit│
└───────────────────────────────────────────────────────────────────┘
```

## How It Works

1. Reads `~/.claude/history.jsonl` to build a list of sessions with project paths and timestamps
2. On selection, loads the session file from `~/.claude/projects/{path}/{sessionId}.jsonl`
3. Filters to user/assistant messages and displays the last 20 turns as a preview
4. On Enter, suspends the TUI and runs `claude --resume <id>` in the session's original directory
5. After claude exits, the TUI resumes

## Dependencies

- `ratatui` — TUI rendering framework
- `crossterm` — terminal backend and event handling
- `serde` / `serde_json` — JSON parsing
- `dirs` — home directory detection
- `chrono` — relative timestamp formatting
- `anyhow` — error handling

## Tests

```sh
cargo test
```
