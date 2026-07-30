# Cursor CLI chat store format (reverse-engineered)

Reference for reading Cursor Agent CLI sessions from disk. Everything here was
verified by direct experiment on macOS against `agent` **2026.07.23**, not taken
from documentation. The format is **undocumented and unstable**, so treat every
read as best-effort and degrade gracefully rather than erroring.

> This document supersedes the Cursor CLI sections of
> `CURSOR-SESSION-ADAPTATION.md`, which contains verified errors (see
> [Corrections](#corrections-to-cursor-session-adaptationmd)).

## Directory layout

```
~/.cursor/chats/{projectHash}/{chatId}/meta.json
~/.cursor/chats/{projectHash}/{chatId}/store.db
```

- `{projectHash}` is the MD5 hex digest of the **canonicalized** absolute
  workspace path. Canonicalization matters: `/tmp/ccsm-probe` hashes as its
  realpath `/private/tmp/ccsm-probe`.
- `{chatId}` is a UUID and **is** the id accepted by `agent --resume <chatId>`.

ccsm never needs to compute the MD5. Session discovery walks the two directory
levels and reads `cwd` out of `meta.json`, which is authoritative. Avoiding the
hash also avoids an MD5 dependency and the canonicalization mismatches that
would come with it.

### `meta.json`

```json
{
  "schemaVersion": 1,
  "createdAtMs": 1785373391533,
  "updatedAtMs": 1785373395180,
  "hasConversation": true,
  "cwd": "/private/tmp/ccsm-probe"
}
```

`hasConversation` is `false` for a chat that was created but never used; such a
chat has a **0-byte `store.db`**. Check the length before opening it.

## `store.db`

SQLite with exactly two tables:

```sql
CREATE TABLE blobs (id TEXT PRIMARY KEY, data BLOB);
CREATE TABLE meta  (key TEXT PRIMARY KEY, value TEXT);
```

Blob ids are `sha256(data)`, so the store is content-addressed. Identical
content is shared across chats (the system prompt blob has the same id in every
chat on this machine).

### WAL mode

The database runs in **WAL mode**, so `store.db-wal` and `store.db-shm` sit
alongside it and recent turns may live only in the `-wal` file. Two consequences:

- Open read-only (`SQLITE_OPEN_READ_ONLY`) and never write. ccsm is a reader
  here; Cursor owns this data.
- A read-only open of a WAL database can fail when ccsm cannot create or write
  the `-shm` file, which is possible while a Cursor session holds the database.
  If the open or query fails, copy `store.db`, `store.db-wal`, and `store.db-shm`
  into a temp dir and read the copy. Do not fall back to reading `store.db`
  alone, since that silently drops the newest turns.

### The `meta` table

One row, key `"0"`, whose `value` is **hex-encoded JSON**:

```json
{
  "agentId": "f42dd464-c77f-428f-8863-5c42b3b0ae81",
  "latestRootBlobId": "07cea122c1f99b7d89fb72b2def19f9b6b668609a67b4f4efea5d1866237ee74",
  "name": "New Agent",
  "mode": "default",
  "isRunEverything": false,
  "createdAt": 1785373391533,
  "blobEncryptionKey": "ab3344360f6d5bb8c46198953eec12f205d561bb34edac04b1fd0449db3113a8"
}
```

- `name` is the chat title. It defaults to `"New Agent"`; the user changes it
  with `/rename` inside a session. Use it as the display title, but treat the
  default as "untitled" rather than showing it verbatim.
- `latestRootBlobId` points at the current root index blob and is rewritten on
  every turn.
- **`blobEncryptionKey` is a red herring.** Blobs are stored unencrypted. Do not
  attempt decryption.

### Blob kinds

Two kinds, distinguished by the first byte:

**Message blobs** start with `{` and are raw UTF-8 JSON:

```json
{ "role": "user", "content": [ { "type": "text", "text": "..." } ] }
```

**Index blobs** are protobuf. Repeated **field 1** (wire type 2, 32-byte
payloads) is the **ordered** list of child message-blob ids. Later fields hold
token counts and other metadata that ccsm does not need.

### Reconstructing a transcript

1. Read the `meta` row, hex-decode, parse JSON, take `latestRootBlobId`.
2. Load that root index blob and collect its field-1 refs **in order**.
3. For each ref, load the blob and parse it as a JSON message.

Older root blobs stay in the database as unreferenced garbage from previous
turns. Ignore anything not reachable from `latestRootBlobId`; walking all blobs
instead would replay superseded history out of order.

Verified against a 2-turn conversation (6 refs) and a tool-using conversation
(8 refs), both in correct chronological order.

## Message shapes

`role` is one of `system`, `user`, `assistant`, `tool`.

`content` is either a plain string or an array of typed blocks. Observed block
types, exhaustively enumerated across every chat on the dev machine:

| Role | `content` form | Block types |
|---|---|---|
| `system` | plain string | — |
| `user` | plain string or blocks | `text` |
| `assistant` | blocks | `text`, `reasoning`, `tool-call` |
| `tool` | blocks | `tool-result` |

Notes for a preview renderer:

- `reasoning` blocks carry an opaque `signature` and usually an **empty**
  `text`. Skip them; rendering them yields blank entries.
- `tool-result` blocks carry `toolName` and a `result` string (plus a duplicate
  under `experimental_content`). Render as a tool marker, not as prose.
- `tool-call` is the assistant side of the same exchange.
- Some blocks nest `providerOptions.cursor.modelName`, which names the model
  that produced the turn. Useful but inconsistently present, so do not rely on
  it.

### Wrapper text that must be stripped

Cursor injects context into the user turns, so raw text is not display-ready:

- The **first** user message is an environment preamble wrapped in
  `<user_info>...</user_info>`. It is not something the user typed and should be
  suppressed from previews entirely.
- Real user messages are wrapped in `<timestamp>...</timestamp>` followed by
  `<user_query>...</user_query>`. Strip both; the useful text is the
  `<user_query>` body.

ccsm already has `data::history::strip_xml_tags` for comparable Claude cleanup.

## CLI behaviour relevant to launching

- **No `--name` flag.** The tmux session name stays ccsm's concern, and the
  Cursor chat title can only be changed with `/rename` inside a session.
- **No `--session-id` flag, but ids can still be pre-assigned.** `agent
  create-chat` prints a fresh UUID without creating anything on disk, and
  `agent --resume <thatId> "prompt"` then starts the chat under exactly that id
  (verified: `create-chat` returned `275655ab-…`, and after the resume a
  directory of that name appeared). So ccsm's invariant that a dispatched job's
  session id is known at dispatch **is** achievable for Cursor, in two steps
  instead of one flag.
- **`agent ls` exists but is not a lister.** Its help reads "Resume a chat
  session": it is an interactive Ink picker and fails without a TTY ("Raw mode
  is not supported"). Session listing must still come from a filesystem scan.
- `agent update` and `agent resume` (latest chat) also exist.
- `--trust` is **required** for a workspace Cursor has not seen before.
  Without it the agent refuses to start and prints "Workspace Trust Required",
  which in a detached tmux pane would look like a silent hang.
- `-f` / `--force` (alias `--yolo`) is the equivalent of Claude's
  `--dangerously-skip-permissions`.
- `-w` / `--worktree [name]` mirrors Claude's `--worktree`.
- `agent --resume <chatId> -p "prompt"` appends a turn to an existing chat and
  reuses the same chat directory rather than creating a new one (verified).
- **Interactive resume can exit immediately** on some CLI builds (observed on
  `2026.07.23-e383d2b`): the TUI shows "Loading conversation", then the
  process dies with SIGTERM (`143`). New sessions (`agent --trust`) stay up;
  print-mode resume (`-p`) still works. Another interactive `agent` in a
  second terminal can make this worse. ccsm watches the tmux pane briefly
  after a Cursor live resume and opens a dismissible popover explaining that
  this is a known Cursor Agent CLI bug outside ccsm's control (instead of a
  status-bar flash or quitting); it also clears parent `CURSOR_*` / askpass env
  when spawning so a nested launch from inside another agent session cannot
  poison the child.

## Corrections to `CURSOR-SESSION-ADAPTATION.md`

| That document claims | Verified truth |
|---|---|
| `store.db` schema is unknown; "treat as opaque"; "do not depend on it" | Fully readable: two tables, unencrypted blobs, format documented above |
| `store.db` may be encrypted (`blobEncryptionKey`) | Blobs are plaintext; the key is unused |
| Chat titles come from `meta.json` | `meta.json` has no title. The title is `name` in the store.db `meta` row |
| `agent ls` lists sessions | It is a *resume picker*, not a lister, and needs a TTY. Listing must scan the filesystem |
| Hooks are documented in `cli/reference/configuration.md` | That page covers `cli-config.json` only; hooks live at <https://cursor.com/docs/hooks> |

Its `create-chat`, `--resume`, `--continue`, `--force`/`--yolo`, `--worktree`,
MD5 project hash, and "IDE UUIDs are not CLI resume ids" claims all checked out.
It omits that `--trust` is mandatory for unseen workspaces.
