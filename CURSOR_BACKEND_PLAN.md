# Cursor CLI Agent backend: implementation plan

Adds **Cursor Agent CLI** sessions to ccsm alongside Claude Code, in one merged
list. Claude behaviour must not change.

Format details live in [`docs/CURSOR-CLI-STORE-FORMAT.md`](docs/CURSOR-CLI-STORE-FORMAT.md),
which is verified by experiment and supersedes `docs/CURSOR-SESSION-ADAPTATION.md`
wherever they disagree. Read the format doc before touching the parser.

## Scope

In scope (Phase 1 + 2):

- List Cursor CLI chats from `~/.cursor/chats/`, merged with Claude sessions
- Preview Cursor transcripts out of `store.db`
- Resume a Cursor chat, and launch new Cursor sessions in tmux
- A persisted source filter to show both backends, Claude only, or Cursor only

Out of scope, deliberately:

- **Scheduler jobs for Cursor.** Jobs stay Claude-only and the UI must say so.
  Note this is a *scope* decision, not a technical one: `agent create-chat`
  followed by `agent --resume <id>` does let a chat id be assigned in advance,
  so the invariant ccsm's job model needs is achievable later. The real
  remaining gaps are no `--settings` (so no per-job Stop hook), no
  `--append-system-prompt`, and no usage windows.
- **Cursor Desktop IDE transcripts** (`~/.cursor/projects/*/agent-transcripts/`).
  Those UUIDs are not valid `agent --resume` ids.
- **Editing Cursor chat titles.** Renaming would mean writing an undocumented
  SQLite store.
- **Cursor usage tracking.** No local equivalent of Claude's usage windows exists.

## Work packages

Each package should build, pass `cargo test`, and be independently reviewable.

### WP1 — Types, listing, merge, source filter

- `src/data/types.rs`: add `AgentBackend { ClaudeCode, CursorAgent }` and a
  `backend` field on `SessionInfo`. Serde-default it to `ClaudeCode` so any
  persisted state keeps loading.
- New `src/data/cursor_history.rs`: `load_cursor_sessions(filter_path)` walking
  `~/.cursor/chats/{hash}/{chatId}/`. Build `SessionInfo` from `meta.json`
  (`cwd`, `createdAtMs`, `updatedAtMs`, `hasConversation`) plus the chat title
  from the `store.db` `meta` row. `slug` is **always `None`** (Cursor has no
  chains). Never compute the MD5 hash; `meta.json`'s `cwd` is authoritative.
- New `load_all_sessions(filter_path)` that concatenates both backends and sorts
  by `last_timestamp` descending, as today. A Cursor scan failure must degrade to
  Claude-only rather than failing the TUI. Call it from `main.rs` and
  `App::reload_sessions` so those still receive a single `Vec<SessionInfo>`.
- Canonicalize paths on both sides when comparing, so `/tmp/x` and
  `/private/tmp/x` do not split one project into two groups.
- `src/config.rs`: `source_filter: "both" | "claude" | "cursor"`, defaulting to
  `both`, persisted like the existing `live_filter`.
- `src/app/filter.rs`: apply the source filter in `recompute_filter` before
  hide-empty, text, and chain grouping.
- `src/keys.rs`: bind **`s`** to cycle both → claude → cursor, then recompute and
  save config. Verified free in the Sessions normal-mode dispatch.

Tests: fixture-backed `load_cursor_sessions` (see Fixtures below), filter
cycling, and that a Cursor session never enters `chain_map`.

### WP2 — Transcript preview

- New `src/data/cursor_store.rs`: open `store.db` read-only, hex-decode the
  `meta` row, walk `latestRootBlobId`'s ordered field-1 refs, parse each
  raw-JSON message blob into `PreviewMessage`.
- Hand-parse the protobuf refs; **do not add a protobuf crate**. The reference
  implementation is `~/Dev/ccsm-cursor-fixtures/prototypes/rust_poc.rs`, already
  proven against real data.
- Skip `reasoning` blocks (their `text` is normally empty), render `tool-call` /
  `tool-result` as compact markers, and strip the `<user_info>`,
  `<timestamp>`, and `<user_query>` wrappers. Suppress the first user message
  entirely; it is an environment preamble, not something the user typed.
- Skip opening `store.db` when its length is 0. Tolerate a missing root or a
  corrupt blob by surfacing one clear error line, never a panic.
- Handle WAL: if a read-only open or query fails, copy `store.db` plus `-wal`
  and `-shm` to a temp dir and read the copy. Do **not** fall back to reading
  `store.db` alone, which silently drops the newest turns.
- `src/data/preview.rs` and `src/app/preview.rs` dispatch on `backend`. Prefix
  the preview cache key with the backend so ids cannot collide across stores.

### WP3 — Dependency

Add `rusqlite = { version = "0.37", features = ["bundled"] }`.

**Pin 0.37.** The current release resolves `libsqlite3-sys` 0.38, whose build
script uses unstable `cfg_select!` and **fails on the project's toolchain**
(rustc 1.93.0). 0.37 resolves `libsqlite3-sys` 0.35, builds in ~13s, and adds 14
packages. `bundled` is safe for the release matrix because every target in
`.github/workflows/release.yml` builds on a native runner, so a C compiler is
always present.

### WP4 — Launch and resume

- `src/app/mod.rs`: carry `backend` on the `LaunchRequest` variants that launch
  an agent (`Resume`, `Direct`, `NewLive`, `NewDirect`). `AttachLive` is
  unchanged. Make `main.rs` match on it exhaustively so a missed path is a
  compile error, not a wrong binary at runtime.
- `src/main.rs`: add Cursor argv construction next to `new_session_argv`.
  **Always pass `--trust`** on ccsm-launched Cursor sessions: without it an
  unseen workspace prints "Workspace Trust Required" and waits, which inside a
  detached tmux pane is indistinguishable from a hang.

| `NewSessionMode` | Claude today | Cursor |
|---|---|---|
| Plain | `claude --name N` | `agent --trust` |
| Danger | `+ --dangerously-skip-permissions` | `+ --force` |
| Worktree | `+ --worktree N --name N` | `+ --worktree N` |
| Direct | bare `claude` | `agent --trust` |

Resume: `agent --trust --resume <chatId>`.

- Naming degrades honestly: the naming popup names the **tmux** session, and the
  Cursor chat keeps its default title until the user runs `/rename`. `r` on a
  Cursor row must refuse with a `status_error` explaining that, and must never
  call `save_custom_title`, which would append Claude-shaped JSONL.
- Picking a backend for `n`: when the source filter is Claude-only or
  Cursor-only, follow it. When it is both, the naming popup holds the choice,
  defaulting to Claude, cycled with `a`. `Tab` keeps cycling mode and
  Left/Right stay with `tui_input`.
- Keep `--new` / `--spawn` Claude-only for backward compatibility.

### WP5 — Config, dependency checks, and the job guard

- `src/config.rs`: `agent_path` with an `agent_bin()` accessor defaulting to
  `agent`. Leave `claude_path` alone.
- `src/ui/config_tab.rs`: an `AGENT_PATH_ROW` **named constant** (never a bare
  index) and a `PickerTarget` variant for browsing to the binary.
- Missing-deps policy, so that having only one agent installed still works:
  - `tmux` missing stays blocking.
  - Exactly one of `claude` / `agent` missing is **not** blocking. Record a soft
    flag and fail that backend's launches with a `status_error`. Listing and
    previewing Cursor sessions needs no binary at all.
  - Both missing is blocking, and the modal should name all three binaries.
  - `leave_config_tab` blocks only on missing tmux, or on both agents missing.
- `src/app/jobs.rs`: refuse to open the job form for a Cursor row with a
  `status_error` saying jobs are Claude-only, and say the same in the Jobs help.
- Hide the usage chip and usage-gated hints when the filter is Cursor-only, with
  a note that usage tracking is Claude-only. (This overrides the architecture
  research, which suggested keeping the chip always visible.)

### WP6 — Presentation and docs

- `src/ui/session_list.rs`: one-column backend mark, `C` for Claude and `A` for
  Cursor, coloured from `src/theme.rs` (Cursor teal). A letter rather than a
  coloured dot, because colour alone is not an accessible signal, and one column
  is affordable in the 30% list where a word is not.
- `src/ui/info_bar.rs`: an `s source` hint at **P3**, alongside `l`/`m`/`v`. It
  will drop on narrow terminals by design, so the help overlay must document it.
- `src/ui/modals.rs`: help lines for the source filter, the Cursor rename
  limitation, and jobs being Claude-only.
- Update `README.md` and the `CLAUDE.md` module map (new `data/` files, the
  `rusqlite` dependency, the source filter).

## Fixtures

Tests must **build synthetic `store.db` files** with `rusqlite` rather than
committing real ones. Real chat databases embed Cursor's proprietary system
prompt, which should not land in a public repo.

Ground-truth data and both working prototypes are kept outside the repo at
`~/Dev/ccsm-cursor-fixtures/` for manual comparison:

```
cursor/chats/ca31439697121595d577b6047eae1794/…   two real chats (2-turn, tool-using)
prototypes/decode.py                              Python reference
prototypes/rust_poc.rs                            Rust reference, matches it
```

Worth covering: ordered role reconstruction, wrapper stripping, a 0-byte
`store.db`, a missing root blob, a `tool-result` block, and Cursor resume argv.

## Risks

1. **The blob format is undocumented and may change.** Keep every byte-level
   assumption inside `cursor_store.rs`, fail soft to a visible error, and assert
   that root refs still look like 32-byte digests so a format change is loud.
2. **Launching without `--trust` looks like a hang.** Always pass it; unit-test
   the argv.
3. **A launch path that forgets `backend` runs the wrong binary.** Exhaustive
   matching plus argv tests.
4. **The missing-deps modal locks out a user who only has one agent installed.**
   The WP5 policy exists for this; existing `ui/config_tab.rs` tests assume the
   Claude-only rule and will need updating.
5. **WAL contention with a live Cursor session.** Read-only, copy-to-temp
   fallback, never write.
6. **Path canonicalization** splitting one project across two headers.
7. **Live tmux rows carry no backend**, so a Cursor pane can still be pointed at
   a Claude-only job. Documented gap, not a Phase 1 blocker.

## Verification

`cargo test` must stay green (baseline: **484 passing**). Also `cargo clippy`.
Do **not** run `cargo fmt`: the repo has pre-existing drift and formatting would
swamp the diff.

Manual checks: a merged list showing both backends, `s` cycling and persisting,
a Cursor preview rendering real text, `agent --trust --resume <id>` attaching in
tmux, and ccsm still starting with only one of the two binaries installed.
