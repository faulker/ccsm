# Adaptation spec: Claude Code sessions → Cursor Agent sessions

Use this document in a new AI coding session to extend an app that currently reads/resumes/manages **Claude Code** agent sessions so it also works with **Cursor Agent** (CLI + desktop).

**Primary target app:** CCSM (`~/Dev/ccsm`), a Rust TUI that browses session history, previews transcripts, launches/resumes sessions in tmux, and runs a job scheduler daemon against Claude Code. The patterns below apply to any similar tool.

**Goal:** One tool, two backends. User picks `claude` or `agent` (Cursor CLI). Session listing, preview, resume, job dispatch, and completion detection should work for both where possible.

---

## 1. Executive summary

| Concern | Claude Code | Cursor Agent CLI | Cursor Desktop GUI |
|---|---|---|---|
| Session index | `~/.claude/history.jsonl` | Scan `~/.cursor/chats/` + `agent ls` | Agents Window search (indexed); no documented public index file |
| Session transcript | `~/.claude/projects/{encodedPath}/{sessionId}.jsonl` | `~/.cursor/chats/{projectHash}/{chatId}/store.db` (SQLite) + possibly JSONL export via hooks | `~/.cursor/projects/{encodedPath}/agent-transcripts/{uuid}/{uuid}.jsonl` |
| Project path encoding | Replace non-alnum with `-` | **MD5 hex digest of canonical absolute cwd** | Replace non-alnum with `-` (same style as Claude) |
| Resume by ID | `claude --resume <sessionId>` | `agent --resume <chatId>` | N/A from CLI; IDE resumes in-app |
| Resume latest | `claude --continue` | `agent --continue` or `agent resume` | N/A |
| Pre-assign session ID | `claude --session-id <uuid>` | `agent create-chat` → prints UUID; no `--session-id` on start | N/A |
| List sessions (official) | Read `history.jsonl` | `agent ls` (interactive TUI; fails without TTY) | Cmd/Ctrl+K in Agents Window |
| Shared store? | — | **No.** CLI, IDE, and ACP use separate stores | **No.** |
| Completion hook | Claude `Stop` hook via `--settings` JSON | Cursor `stop` hook via `~/.cursor/hooks.json` | Same hooks for CLI-launched sessions |
| Dangerous mode | `--dangerously-skip-permissions` | `--force` / `--yolo` | IDE setting |
| Worktree | `--worktree <name>` | `--worktree [name]` | N/A |
| Config dir | `~/.claude/` | `~/.cursor/` (`cli-config.json`, `hooks.json`) | `~/Library/Application Support/Cursor/` (macOS) |

**Critical rule:** Desktop IDE transcript UUIDs under `agent-transcripts/` are **not** valid CLI resume IDs. Only chat IDs from `~/.cursor/chats/` (or `agent create-chat`) work with `agent --resume`.

---

## 2. On-disk layout (verified on macOS, 2026-07-29)

### 2.1 Claude Code (current)

```
~/.claude/history.jsonl          # one line per activity; session index
~/.claude/projects/{dirName}/{sessionId}.jsonl   # full transcript
```

- `{dirName}` = project path with non-alphanumeric chars → `-`
  - Example: `/Users/sane/Dev/wtm` → `-Users-sane-Dev-wtm`
- `history.jsonl` line shape:
  ```json
  {"display":"...","timestamp":1769639762861,"project":"/abs/path","sessionId":"uuid"}
  ```
- Transcript line shape (varies by entry type):
  ```json
  {"type":"user","message":{"role":"user","content":[{"type":"text","text":"..."}]},"cwd":"/abs/path","gitBranch":"main","timestamp":"2026-..."}
  {"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"..."}]}}
  {"type":"custom-title","customTitle":"My title"}
  ```

CCSM reads history via `src/data/history.rs`, transcripts via `src/data/preview.rs` + `src/data/io.rs`, session discovery via `src/schedule/mod.rs::discover_session_id`.

### 2.2 Cursor Agent CLI

```
~/.cursor/chats/{projectHash}/{chatId}/meta.json
~/.cursor/chats/{projectHash}/{chatId}/store.db    # SQLite (may be empty until conversation)
```

- `{projectHash}` = **MD5 hex of canonical absolute workspace path**
  - Verified: `md5("/Users/sane/Dev/wtm")` = `0bea7e4457e117119b86262ae8847f6f`
- `meta.json` shape (observed):
  ```json
  {"schemaVersion":1,"createdAtMs":1785368614683,"updatedAtMs":1785368615088,"hasConversation":false,"cwd":"/Users/sane/Dev/wtm"}
  ```
- `store.db`: SQLite schema is **undocumented** and was empty (0 bytes) for a chat with no messages. Treat as opaque until reverse-engineered or exported via hooks/SDK. Do not depend on it for v1 unless you inspect a populated DB.

**Listing sessions without TTY:** `agent ls` uses an Ink TUI and fails in non-interactive contexts (`Raw mode is not supported`). For programmatic listing, scan:

```rust
// Pseudocode
for entry in read_dir("~/.cursor/chats") {
    let project_hash = entry.name();
    for chat_dir in read_dir(entry.path()) {
        let meta = parse_json(chat_dir.join("meta.json"));
        sessions.push(SessionInfo {
            session_id: chat_dir.name(),  // UUID
            project: meta.cwd,
            first_timestamp: meta.created_at_ms,
            last_timestamp: meta.updated_at_ms,
            has_data: chat_dir.join("store.db").metadata()?.len() > 0,
        });
    }
}
```

Reverse `{projectHash}` → path: build a cache at runtime by hashing known project roots, or store `cwd` from `meta.json` (authoritative).

### 2.3 Cursor Desktop IDE (optional read-only source)

```
~/.cursor/projects/{dirName}/agent-transcripts/{uuid}/{uuid}.jsonl
```

- `{dirName}` uses Claude-style hyphen encoding, e.g. `empty-window`, `Users-sane-Dev-wtm`
- Transcript line shape (observed):
  ```json
  {"role":"user","message":{"content":[{"type":"text","text":"..."}]}}
  {"role":"assistant","message":{"content":[{"type":"text","text":"..."},{"type":"tool_use","name":"Shell","input":{...}}]}}
  {"type":"turn_ended","status":"success"}
  ```
- Differences from Claude JSONL:
  - Top-level `role` instead of `type: user|assistant`
  - `message.content` is always a block array (no plain string shortcut)
  - Tool calls use `tool_use` blocks, not Claude's nested structure
  - No guaranteed `cwd` / `gitBranch` on every line
  - **These UUIDs cannot resume via `agent --resume`**

Use IDE transcripts only for preview/history of desktop sessions, not for CLI resume.

---

## 3. CLI command mapping

| Operation | Claude Code | Cursor Agent |
|---|---|---|
| Binary | `claude` | `agent` (often `~/.local/bin/agent`) |
| New session (interactive) | `claude` | `agent` |
| New session + initial prompt | `claude "do thing"` | `agent "do thing"` |
| Resume specific | `claude --resume <id>` | `agent --resume <chatId>` |
| Resume latest in cwd | `claude --continue` | `agent --continue` |
| Resume picker | (interactive) | `agent ls` or in-session `/resume` |
| Create empty chat, get ID | `claude --session-id <uuid>` then start | `agent create-chat` → prints UUID |
| Name session | `claude --name <name>` | `/rename` in session; no direct `--name` on CLI start |
| Skip permissions | `--dangerously-skip-permissions` | `--force` or `--yolo` |
| Plan mode | — | `--plan` / `--mode plan` |
| Worktree | `--worktree <name>` | `--worktree [name]` |
| Trust workspace | — | `--trust` |
| Print/non-interactive | `claude -p "..."` | `agent -p "..."` |
| Copy conversation ID | — | `/copy-conversation-id` |

Docs:
- https://cursor.com/docs/cli/overview.md
- https://cursor.com/docs/cli/reference/parameters.md
- https://cursor.com/docs/cli/reference/slash-commands.md

---

## 4. CCSM modules → required changes

### 4.1 Introduce an agent backend enum

```rust
enum AgentBackend {
    ClaudeCode,
    CursorAgent,
}
```

Config additions (`src/config.rs`):
- `agent_backend: "claude" | "cursor"` (default `"claude"`)
- Rename or generalize `claude_path` → `agent_bin_path` (or keep both for backward compat)
- Cursor equivalent of `claude_path`: default `"agent"`

### 4.2 Session data layer (`src/data/`)

Abstract behind a trait:

```rust
trait SessionStore {
    fn load_sessions(filter_path: Option<&str>) -> Result<Vec<SessionInfo>>;
    fn session_file_path(project: &str, session_id: &str) -> Option<PathBuf>;
    fn load_session_messages(project: &str, session_id: &str) -> (SessionMeta, Vec<PreviewMessage>);
    fn project_to_key(project: &str) -> String;  // hyphen encoding vs md5
}
```

Implement:
- `ClaudeSessionStore` — existing code, unchanged behavior
- `CursorCliSessionStore` — scan `~/.cursor/chats/**/meta.json`
- (Optional) `CursorIdeSessionStore` — read `agent-transcripts/*.jsonl` for desktop-only history

**Preview parser:** Add `CursorTranscriptEntry` deserializer. Map:
- `role: user|assistant` at top level
- `message.content[]` blocks where `type == "text"`
- Skip `tool_use`, `turn_ended`, subagent lines unless you want tool preview
- Strip `<timestamp>...</timestamp>` and `<user_query>...</user_query>` wrappers (Cursor-specific)

**History index:** Cursor CLI has no `history.jsonl`. Build `SessionInfo` entirely from chat directories.

### 4.3 Launch / resume argv (`src/schedule/engine.rs`, `src/main.rs`)

Current Claude start (`build_start_argv`):
```
claude --session-id <job.id> --name <name> --settings '{hooks...}' --append-system-prompt '...' [--model M] [--dangerously-skip-permissions] [prompt]
```

Cursor start (proposed):
```
agent --trust [--force] [--model M] [--worktree name] [prompt]
```

**Session ID strategy for Cursor jobs:**
1. Before dispatch: run `agent create-chat` in job cwd → capture printed UUID
2. Store as `cursor_session_id` / generalize `claude_session_id` → `agent_session_id`
3. Start session: `agent --resume <id>` is wrong for brand-new; instead either:
   - `agent --resume <id> "initial prompt"` if supported, or
   - `agent "initial prompt"` and rely on `--continue` being wrong
   - **Best:** verify whether `agent --resume <newId>` on an empty chat works; if not, use `create-chat` + first prompt via tmux paste after attach

Current Claude resume (`build_resume_argv`):
```
claude --resume <id>   # or --continue if id unknown
```

Cursor resume:
```
agent --resume <chatId>   # or agent --continue
```

No positional prompt on resume for either; continuation text is pasted via tmux (`Action::Resume`).

**New interactive session** (`main.rs::new_session_argv`):
- Claude: `claude [--dangerously-skip-permissions] [--worktree name] --name name`
- Cursor: `agent [--force] [--worktree name]` (no `--name`; user renames with `/rename`)

### 4.4 Session discovery (`src/schedule/mod.rs::discover_session_id`)

Claude: scan `~/.claude/projects/{dir}/{id}.jsonl`, match first line timestamp + cwd.

Cursor: scan `~/.cursor/chats/{md5(cwd)}/`, read `meta.json`:
- `createdAtMs >= since_ms - 5000`
- `meta.cwd` canonicalized matches job cwd
- Return chat dir name if exactly one candidate

### 4.5 Completion detection (`src/schedule/completion.rs`)

Claude primary: `Stop` hook via inline `--settings`:
```json
{"hooks":{"Stop":[{"hooks":[{"type":"command","command":"ccsm --job-complete <jobId>"}]}]}}
```

Cursor primary: `stop` hook in `~/.cursor/hooks.json` (user-level or project-level). Cannot pass per-job command inline on argv. Options:

**A. User-level hook (simplest):** Extend `~/.cursor/hooks.json`:
```json
{
  "version": 1,
  "hooks": {
    "stop": [{ "command": "./hooks/ccsm-job-complete.sh" }]
  }
}
```
Hook script reads stdin JSON, extracts `session_id`, maps session → job (see B).

**B. Job registry file:** When dispatching a Cursor job, write `~/.config/ccsm/cursor-sessions/{chatId} → {jobId}`. Hook script:
```bash
input=$(cat)
chat_id=$(echo "$input" | jq -r '.session_id // empty')
# lookup job_id from registry; run ccsm --job-complete "$job_id"
```

**C. Marker fallback (both backends):** Keep `CCSM_JOB_COMPLETE` in system prompt / continuation prompts. Parse transcript tail for assistant `text` blocks containing the marker on its own line.

Cursor transcript parsing for marker:
- CLI: read from hook-provided `transcript_path` on `stop`, or export path if documented
- IDE JSONL: top-level `role: assistant`, block `type: text`
- Claude JSONL: `type: assistant`, `message.content` text blocks

**Cursor hook stdin fields** (from existing `log-conversation.sh`):
```bash
CWD=$(echo "$HOOK_INPUT" | jq -r '.cwd // .workspace.current_dir // ""')
TRANSCRIPT_PATH=$(echo "$HOOK_INPUT" | jq -r '.transcript_path // ""')
SESSION_ID=$(echo "$HOOK_INPUT" | jq -r '.session_id // ""')
```

Docs: https://cursor.com/docs/cli/reference/configuration.md (hooks), create-hook skill.

**Do not use Claude's `--append-system-prompt` on Cursor** — flag may not exist. Inject completion instruction via:
- first prompt text (`with_completion_protocol`)
- continuation prompts (already done)
- Cursor rules (global, less reliable for completion)

### 4.6 Models picker (`src/models.rs`)

Claude: reads `~/.claude.json`.

Cursor: run `agent --list-models` or `agent models` (non-interactive). Parse output for job form picker.

### 4.7 Usage / rate limits (`src/usage/`)

Claude-specific (Desktop history + OAuth API). Cursor has different billing/limits.

**v1 recommendation:** When `agent_backend == cursor`, disable or hide usage-gated scheduling; show "usage tracking Claude-only" in Config tab. Revisit later if Cursor exposes a local usage file.

### 4.8 Config tab / deps check

- `leave_config_tab`: check `agent` binary when backend is Cursor
- Labels: "Claude binary" → "Agent binary"
- Missing deps modal: mention `agent` install (`curl https://cursor.com/install -fsS | bash` or Cursor app bundle)

### 4.9 Field renames (optional but clearer)

| Current | Generalized |
|---|---|
| `claude_session_id` | `agent_session_id` |
| `claude_bin()` | `agent_bin()` |
| `missing_claude` | `missing_agent` |

Keep serde aliases for backward compat in `schedule.json`.

---

## 5. Suggested implementation phases

### Phase 1 — Read-only Cursor support
- [ ] `CursorCliSessionStore::load_sessions()` from `~/.cursor/chats`
- [ ] Preview parser for Cursor IDE JSONL (or CLI export if available)
- [ ] Config: backend selector
- [ ] Resume in foreground: `agent --resume <id>` from session list
- [ ] Tests with fixture `meta.json` + sample JSONL

### Phase 2 — Launch + tmux
- [ ] `build_start_argv` / `build_resume_argv` for Cursor
- [ ] `agent create-chat` before managed job dispatch
- [ ] `new_session_argv` for manual new sessions
- [ ] Map `--dangerously-skip-permissions` → `--force`

### Phase 3 — Scheduler integration
- [ ] `discover_session_id` for Cursor
- [ ] Completion: hook script + session registry
- [ ] Marker fallback parser for Cursor JSONL shape
- [ ] Clear stop stamps on dispatch/resume (unchanged logic)

### Phase 4 — Polish
- [ ] Merge CLI + IDE session lists (clearly label source: `cli` vs `ide`)
- [ ] `agent ls` fallback note in UI when scan finds nothing
- [ ] README + Config tab help text

---

## 6. Test fixtures to create

```
tests/fixtures/cursor/
  chats/0bea7e4457e117119b86262ae8847f6f/
    960a64fd-7a8c-4d8e-9285-e0cd8d593b81/
      meta.json
      store.db          # optional; copy from real session after conversation
  transcripts/ide/
    sample.jsonl        # 5-10 lines: user, assistant+tool_use, turn_ended
  hooks/stop-payload.json
```

Test cases:
- `project_hash` = md5 of canonical path
- `load_sessions` filter by `filter_path` prefix on `meta.cwd`
- Preview extracts user/assistant text from Cursor JSONL
- `build_resume_argv` produces `agent --resume uuid`
- `discover_session_id` returns None when 0 or 2+ candidates
- Completion marker detected in Cursor assistant text block

---

## 7. Known gaps / risks

1. **`store.db` schema undocumented** — CLI transcript may only be accessible via hooks (`transcript_path` on `stop`) or SDK local store until reverse-engineered.
2. **`agent ls` requires TTY** — cannot shell out for session list in daemon; must scan filesystem.
3. **No inline per-job hooks** — Cursor hooks are file-based; need registry indirection for `ccsm --job-complete <jobId>`.
4. **No `--session-id` on start** — job id ≠ chat id unless you pre-create with `create-chat` and store mapping.
5. **IDE vs CLI IDs differ** — never mix resume IDs across stores.
6. **Usage scheduler** — Claude-specific; disable or stub for Cursor backend initially.
7. **`--name` missing on Cursor CLI** — tmux session name remains ccsm's concern; Cursor chat title is separate (`/rename`).
8. **Slash commands** — Cursor uses skills/commands differently; `with_completion_protocol` skip logic for `/foo` may need Cursor equivalent.

---

## 8. Reference: CCSM files touched

| File | Change |
|---|---|
| `src/config.rs` | `agent_backend`, generalized bin path |
| `src/data/mod.rs` | Trait + backend dispatch |
| `src/data/history.rs` | Split Claude vs Cursor loaders |
| `src/data/io.rs` | `cursor_chat_path()`, `project_to_hash()` |
| `src/data/preview.rs` | Cursor JSONL parser |
| `src/data/types.rs` | Optional `session_source: Cli \| Ide` |
| `src/schedule/engine.rs` | `build_*_argv` branches |
| `src/schedule/mod.rs` | `discover_session_id` for Cursor |
| `src/schedule/completion.rs` | Cursor hook helpers, transcript parser |
| `src/main.rs` | `new_session_argv`, launch paths |
| `src/models.rs` | Cursor model discovery |
| `src/app/mod.rs`, `keys.rs`, `ui/config_tab.rs` | Backend selector UI |
| `README.md` | Document Cursor backend |

---

## 9. Prompt to paste into the implementation session

```
I have a Rust app (CCSM at ~/Dev/ccsm) that manages Claude Code agent sessions.
Read ~/Dev/ccsm/docs/CURSOR-SESSION-ADAPTATION.md and implement Phase 1 (read-only
Cursor CLI session listing + preview + resume), then Phase 2 (launch argv).

Constraints:
- Keep Claude Code behavior unchanged when backend is "claude"
- Add config field agent_backend: "claude" | "cursor" (default "claude")
- Scan ~/.cursor/chats/**/meta.json for session index (agent ls is not scriptable)
- project hash = MD5 of canonical absolute cwd
- Resume via `agent --resume <chatId>`
- Parse Cursor IDE JSONL for preview (role + message.content text blocks)
- Do not use IDE transcript UUIDs as CLI resume IDs
- Add tests with fixtures under tests/fixtures/cursor/
- Follow existing CCSM patterns (pure plan in engine.rs, I/O in watch.rs, tests in module tests.rs)
- Run cargo test after changes
```

---

## 10. Useful commands for exploration

```bash
# Cursor CLI
agent --help
agent create-chat          # prints new chat UUID
agent --resume "<uuid>"
agent --continue

# Inspect CLI store
find ~/.cursor/chats -name meta.json -exec sh -c 'echo "=== $1 ==="; cat "$1"' _ {} \;

# Project hash
python3 -c 'import hashlib,os; p=os.path.realpath("/path/to/project"); print(hashlib.md5(p.encode()).hexdigest())'

# IDE transcripts
find ~/.cursor/projects -path '*/agent-transcripts/*/*.jsonl' | head

# Claude (comparison)
head -1 ~/.claude/history.jsonl
ls ~/.claude/projects/
```

---

*Generated 2026-07-29 from CCSM codebase analysis + live inspection of `~/.cursor/` on the author's machine.*
