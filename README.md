# ccsm — Claude Code Session Manager

A terminal UI for browsing your Claude Code session history, previewing conversations, and resuming sessions in their original working directory.

## Features

- Lists all Claude Code sessions sorted by most recent
- Shows a scrollable preview of conversation messages
- Resume any session directly — opens `claude --resume <id>` in the original project directory
- Lazy-loads and caches session previews for fast navigation

## Requirements

- Rust 1.75+
- [Claude Code CLI](https://docs.anthropic.com/en/docs/claude-code) installed and on your `PATH`
- Existing session history in `~/.claude/`

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

## Key Bindings

| Key | Action |
|---|---|
| `j` / `↓` | Next session |
| `k` / `↑` | Previous session |
| `Enter` | Resume session in claude |
| `J` (shift) | Scroll preview down |
| `K` (shift) | Scroll preview up |
| `q` / `Esc` | Quit |

## Layout

```
┌─ Sessions ────────────────────┬─ Preview ────────────────────────┐
│ ▶ my-project         2h ago  │ USER:                            │
│   other-project      3d ago  │ can you update the readme         │
│   ...                        │                                   │
│                              │ ASSISTANT:                        │
│                              │ Sure, let me read the file...     │
├──────────────────────────────┴───────────────────────────────────┤
│ ↑↓/jk navigate  Enter: open in claude  J/K scroll preview  q quit│
└──────────────────────────────────────────────────────────────────┘
```

## How It Works

1. Reads `~/.claude/history.jsonl` to build a list of sessions with project paths and timestamps
2. On selection, loads the session file from `~/.claude/projects/{path}/{sessionId}.jsonl`
3. Filters to user/assistant messages and displays the last 20 turns as a preview
4. On Enter, suspends the TUI and runs `claude --resume <id>` in the session's original directory
5. After claude exits, the TUI resumes

## Tests

```sh
cargo test
```
