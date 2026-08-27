# ccsm: Claude Code Session Manager

A terminal UI for [Claude Code](https://docs.anthropic.com/en/docs/claude-code) and [Cursor Agent CLI](https://cursor.com/docs/cli/overview) sessions. Browse past conversations, resume where you left off, start new ones, and (for Claude) schedule jobs that pause when your usage budget runs out.

You do not need both agents installed. One is enough. Listing and previewing Cursor chats does not even need the `agent` binary.

[Project page on sleepymagpie.com](https://sleepymagpie.com/tools/ccsm.html)

Sessions grouped by project, preview on the right, usage in the tab strip.

![Tree view with session preview](screenshots/sessions-tree-view.png)

## Requirements

- **macOS** (ARM64, x86_64), **Linux** (x86_64, ARM64), or **Windows** (x86_64)
- Something to browse: Claude history in `~/.claude/` and/or Cursor chats in `~/.cursor/chats/`
- `claude` and/or `agent` on your `PATH` to resume or start sessions (listing Cursor chats does not need `agent`)
- `tmux` for live sessions (optional: browsing history works without it)
  - **macOS:** `brew install tmux` ([Homebrew](https://brew.sh))
  - **Linux:** `sudo apt install tmux` / `sudo dnf install tmux` / your distro's package manager

## Install

**macOS / Linux:**

```sh
curl -fsSL https://raw.githubusercontent.com/faulker/ccsm/main/remote-install.sh | bash
```

Installs to `~/.local/bin/ccsm`. Make sure `~/.local/bin` is on your `PATH`.

**Windows (PowerShell):**

```powershell
irm https://raw.githubusercontent.com/faulker/ccsm/main/remote-install.ps1 | iex
```

Installs `ccsm.exe` to `%LOCALAPPDATA%\ccsm` and adds it to your user `PATH`.

**From source** (Rust required):

```sh
./install.sh
```

Builds a release binary and symlinks it to `~/.local/bin/ccsm`. Or `cargo build --release` and run `target/release/ccsm`.

## First run

```sh
ccsm
```

You land on the **Sessions** tab: projects on the left, a conversation preview on the right. Press `?` any time for the full key list. `q` quits. `Esc` never quits; it backs out of a popup or clears a filter.

### Resume a past session

1. Move with `j` / `k` (or the arrow keys). Sessions start grouped by project.
2. Press `→` or `Enter` on a project header to expand it.
3. The right pane shows a preview of the selected conversation.
4. Press `Enter` to resume in tmux, or `Shift+Enter` to resume in the foreground (no tmux).

Rows are marked `C` (Claude Code) or `A` (Cursor Agent). Press `s` to show both, Claude only, or Cursor only.

### Start a new session

1. Highlight a project (or press `b` to pick a directory).
2. Press `n`. Type a name, or leave it blank for the suggested one.
3. `↓` moves to **Agent** (if both backends are available) then **Type**. `←` / `→` cycle the focused row.
4. Press `Enter` to launch.

**Type** is how the session runs:

| Type | What it launches |
|---|---|
| `plain` | A named live session in tmux |
| `danger` | Same, with `--dangerously-skip-permissions` (Claude) or `--force` (Cursor) |
| `worktree` | Its own git worktree. Skipped outside a git repo, so it cannot be chosen by mistake |
| `direct` | The agent in the foreground, no tmux (the name is unused) |

Skip the TUI and start a Claude session in the current directory with `ccsm --new`.

### Attach, switch, detach

Live sessions sit at the top of the list with a status dot: **●** green (working), **●** amber (idle at the prompt), **▶** red (waiting for you). `Enter` attaches.

Once you are inside tmux:

| Key | Action |
|---|---|
| `Ctrl+\` | Detach and return to ccsm |
| `Ctrl+n` / `Ctrl+p` | Next / previous live session |
| `Ctrl+l` | Spawn a new session and switch to it |

## Screenshots

### Flat view
The same sessions as one newest-first list instead of project groups. Start with `--flat`, press `v` to cycle the view, or change it on the Config tab.

![Flat session list](screenshots/sessions-flat-view.png)

### Jobs tab
Scheduled Claude sessions on the left, the selected job's detail on the right. The history at the bottom is where pause/resume against your usage window shows up.

![Jobs tab](screenshots/jobs-tab.png)

### Job form
On the Jobs tab, `n` opens a form: name, directory, prompt, and how the watcher should pause and resume.

![New job form](screenshots/job-form.png)

### Directory browser
Any path the app needs can be browsed instead of typed: a new session's directory, a job's working directory, binary paths, or the usage history file. Press `/` inside the picker to type a path by hand.

![Directory browser](screenshots/directory-picker.png)

### Config tab
Settings on the left, an explanation of the selected one on the right. `Tab` from Jobs, or `o` from anywhere.

![Config tab](screenshots/config-popup.png)

### Help
`?` opens a keybinding reference on the page that matches the tab you are on. `Tab` switches pages.

![Help popup](screenshots/help-popup.png)

## Everyday keys

| Key | Action |
|---|---|
| `j` / `↓` | Next item |
| `k` / `↑` | Previous item |
| `→` | Expand group (tree view) |
| `←` | Collapse group, or jump to the project header |
| `Enter` | Resume in tmux / attach to a live session / toggle a group |
| `Shift+Enter` | Resume a historical session in the foreground (no tmux) |
| `Tab` / `Shift+Tab` | Cycle Sessions, Jobs, Config |
| `Shift+J` / mouse wheel | Scroll the preview |
| `/` | Filter by project name or path |
| `o` | Jump to Config |
| `Space` | Favorite a project (pins it to the top, shown with ★) |
| `n` | New session popup |
| `v` | Cycle the list view (tree vs flat, and how project names are labeled) |
| `l` | Show only running live sessions |
| `s` | Cycle source filter: both → Claude → Cursor |
| `r` | Rename a Claude session or live tmux session (Cursor titles: `/rename` inside the agent) |
| `x` | Stop the selected live session (asks first) |
| `b` | Browse for a directory, then start a new session there |
| `m` | Manage the selected Claude session as a job (Cursor chats cannot be jobs) |
| `?` | Help |
| `q` / `Ctrl+C` | Quit |
| `Esc` | Back out of a popup, or clear the filter. Never quits |

On a small terminal the status bar keeps `? help` and `q quit` and hides the rest behind `…`. Press `?` for the full list; it scrolls with `j` / `k`.

### Filter (`/`)

| Key | Action |
|---|---|
| Type | Filter by project name or path (case-insensitive) |
| `↓` / `↑` | Move through matches (stays in filter mode) |
| `Enter` | Keep the filter and go back to normal mode |
| `←` / `→`, `Home` / `End` | Move the cursor |
| `Ctrl+W` / `Ctrl+U` | Delete the previous word / clear the line |
| `Esc` | Clear the filter and exit |

### Config tab

`j` / `k` move between settings. The right pane names the setting, shows its current value, and lists the keys that act on it. Changes save as you make them.

| Key | Action |
|---|---|
| `Space` / `Enter` | Toggle, edit, or browse for a path |
| `i` | Type a path by hand instead of browsing |
| `←` / `→` | On **View**: cycle the list view either way |
| `Esc` | Back to Sessions |

### Updates

If a newer release is available, a dialog offers `y` to install and restart, or `n` / `Esc` to skip until next run.

## Live sessions

Live sessions run on a dedicated tmux server (the `ccsm` socket), so they do not mix with your other tmux sessions. They appear at the top of the list.

- **Start:** `n`, then `↓` to Type and `←` / `→` to pick `plain`, `danger`, `worktree`, or `direct`
- **Attach / detach:** `Enter` to attach, `Ctrl+\` to come back
- **Switch without detaching:** `Ctrl+n` / `Ctrl+p`
- **Stop / rename:** `x` / `r`
- **Running only:** `l`

The tmux status bar shows those keys. ccsm writes its tmux config to `~/.config/ccsm/tmux.conf`.

Interactive Cursor resume depends on the Cursor Agent CLI. If `agent --resume` flashes and exits (a known CLI bug on some builds), ccsm shows a popover rather than vanishing. Headless `agent --resume <id> -p "…"` still works. A second interactive `agent` in another terminal can interfere.

## Scheduled jobs

A Pro/Max plan has a fixed budget per rolling 5-hour window. A long Claude task that burns through it usually means waiting at the keyboard for the reset. A **job** is a Claude session ccsm starts with a prompt, pauses before the budget runs out, and continues when the window resets.

Jobs are Claude-only. Cursor chats cannot be managed this way.

Open the Jobs tab with `Tab`. `n` creates a job, or press `m` on a Claude session in the Sessions list to adopt it. The watcher daemon starts itself the first time you create a job (unless you turned **Auto-start watcher** off). You do not have to run `--watch` yourself.

The tab strip shows current usage (`⏱ 61% · resets 1h11m`) on every tab, hidden when the source filter is Cursor-only. `⏱ off` in red means jobs exist but the watcher is not running.

| Key | Action |
|---|---|
| `j` / `k` | Move between jobs |
| `Enter` | Attach to the job's tmux session |
| `n` / `e` | New job / edit selected |
| `p` / `c` | Pause now / continue now |
| `x` / `d` | Hard stop / delete (both confirm first) |
| `f` | Mark done for good (confirms first) |
| `Space` | Toggle auto-resume |
| `s` | Start or stop the watcher |
| `L` | Attach to the watcher's live log |
| `Esc` | Back to Sessions |

The form explains the selected field at the bottom. **Directory** opens the browser (`Enter` or `b`; `i` to type). **Model** is a picker (`Enter`, `Space`, or `←` / `→`; `i` to type an id). The list is read from Claude Code at startup, plus the tier aliases `opus`, `sonnet`, `haiku`, and `fable`.

A blank **Continue prompt** inherits the global default (`Continue where you left off.`), shown in full rather than as "(default)". Change the default under **Jobs manager → Continue prompt** on the Config tab.

### Pausing

The default is a **soft** pause: one `Escape` in the pane. That interrupts the current turn and leaves the conversation intact, so resume is just pasting the continue prompt. An idle session costs nothing against the API. **Hard** pause kills the tmux session and relaunches later with `claude --resume`.

Two thresholds stop it flapping: **Pause at %** (default 95) and **Resume at %** (default 50). A job paused at 95% does not come back until usage falls to 50% or the window actually resets.

### When a job finishes

A dispatched session has no natural end: Claude prints a summary and sits at a prompt. Three things mark it done, stop its tmux session, and make sure it is never re-dispatched:

1. **Stop hook.** Sessions ccsm launches get a `Stop` hook (`ccsm --job-complete <id>`). Claude Code fires it when the agent finishes a turn, so the watcher does not have to trust the model to say it is done. If you are attached, completion waits until you detach (set `defer_while_attached` to `false` in the config file to turn that off).
2. **Idle fallback.** A pane that has looked idle for an unbroken stretch (default 15 minutes) is also marked done. A permission prompt counts as *waiting*, not idle. Set the window under **Idle completion**, or `0` to turn this off.
3. **`f`** on the Jobs tab marks it done by hand.

Adopted sessions (ones ccsm did not launch) have no hook, so they also get a backstop instruction to end with the line `CCSM_JOB_COMPLETE`. Treat that as a fallback, not a guarantee.

### The watcher

The watcher is a headless process in its own `ccsm-watch` tmux session. It keeps running after you quit the TUI, and the TUI restarts it on launch if it is from an older ccsm version.

```sh
ccsm --watch-status   # health and a summary of every job
ccsm --watch          # run the daemon in the foreground (usually not needed)
ccsm --usage          # print the current 5-hour and 7-day reading
```

`tmux -L ccsm attach -t ccsm-watch` tails the live log. The same log is `watch.log` in the [config directory](#configuration).

If the watcher is down, the Jobs tab says so in red and shows how many commands are waiting, so queued work is not silently lost.

### Usage data

No extra binary. Two sources, chosen by `usage_source` in config:

- **`local`** reads Claude Desktop's `plan-usage-history.json` (no auth, no network). Desktop writes a sample every 15 to 18 minutes. There is no authoritative reset time, so the 5-hour reset is estimated from window rollovers and labelled `(est)`.
- **`api`** calls the OAuth usage endpoint with Claude Code's token (`CCSM_USAGE_TOKEN` or `CLAUDE_CODE_OAUTH_TOKEN`, then `~/.claude/.credentials.json`, then the macOS Keychain, which can prompt for a password). This one reports a real reset time.
- **`auto`** (default) uses local data while it is fresh, and only hits the API when that data is stale or missing.

```
5h window   72% used   resets in ~3h 39m (est)
7d window   66% used
source: local, sampled 55s ago
```

If Claude Desktop is not installed, set `"usage_source": "api"` in the config file, or point **Usage history file** at a non-standard path on the Config tab.

A stale sample can still pause a job (safer to stop) but never resume one. **Usage sample stale after** (default 5 minutes) is how long a paused job will wait on an old reading.

## Configuration

Most settings are on the Config tab (`o`). They save as you change them.

Config directory:

- **macOS:** `~/Library/Application Support/ccsm/`
- **Linux:** `~/.config/ccsm/`
- **Windows:** `%APPDATA%\ccsm\`

The file is `config.json` in that directory. A few scheduler knobs are JSON-only (not on the Config tab).

| Field | Default | Description |
|---|---|---|
| `tree_view` | `true` | Tree (grouped by project) vs flat list |
| `display_mode` | `"name"` | How project groups are labeled: `"name"`, `"short_dir"`, `"full_dir"` |
| `hide_empty` | `true` | Hide sessions with no data, and exit-only sessions |
| `group_chains` | `true` | Group chained (parent → child) sessions as one row |
| `live_filter` | `false` | Show only running live sessions |
| `source_filter` | `"both"` | `"both"`, `"claude"`, or `"cursor"` |
| `favorites` | `[]` | Project directories pinned to the top |
| `claude_path` / `agent_path` / `tmux_path` | unset | Override binaries; unset means look on `PATH` |
| `usage_pause_percent` | `95.0` | Pause jobs once 5-hour usage reaches this % |
| `usage_resume_percent` | `50.0` | Resume only once usage falls to this % |
| `pause_mode` | `"soft"` | `soft` sends Escape; `hard` kills the session and later `--resume` |
| `watch_autostart` | `true` | Start the watcher when a job is created |
| `watch_seven_day` | `true` | Also pause on the 7-day window |
| `usage_poll_seconds` | `60` | How often to sample usage while a job is active |
| `usage_max_age_seconds` | `300` | Older than this counts as stale |
| `usage_source` | `"auto"` | `"auto"`, `"local"`, or `"api"` |
| `usage_history_path` | unset | Override path to `plan-usage-history.json` |
| `defer_while_attached` | `true` | Do not auto-complete a job while you are attached |
| `continue_prompt` | `"Continue where you left off."` | Default text pasted to wake a paused job |
| `idle_complete_seconds` | `900` | Mark a job done after this many idle seconds (`0` disables) |
| `max_restart_attempts` | `5` | Give up after this many consecutive failures |

## Command-line flags

```sh
ccsm                          # open the TUI
ccsm ~/projects/my-app        # only sessions from that directory
ccsm --flat                   # start in flat view
ccsm --live                   # only running tmux sessions (implies --flat)
ccsm --new                    # skip the TUI; start a live Claude session here and attach
ccsm --spawn                  # from inside a live session: new Claude session and switch to it
ccsm --watch                  # watcher daemon in the foreground (usually not needed)
ccsm --watch-status           # watcher health and job summary
ccsm --usage                  # print 5-hour and 7-day usage and exit
```

`--new` and `--spawn` are Claude-only. After Claude exits, `--new` relaunches ccsm.

## Build & test

```sh
cargo build --release
cargo test
```

macOS release binaries are codesigned and notarized when the Apple secrets are set in the release workflow; without them the release still ships unsigned. See [docs/macos-signing.md](docs/macos-signing.md).
