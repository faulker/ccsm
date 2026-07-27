//! Job completion detection.
//!
//! A dispatched job has no natural end: claude finishes its turn and sits at
//! an idle prompt, so without an explicit completion signal the daemon keeps
//! resuming and relaunching it forever. This module supplies that signal.
//!
//! There are two independent signals, because asking the model to report its
//! own completion turned out not to work.
//!
//! **The stop hook is the primary signal.** Every session ccsm launches carries
//! a `Stop` hook ([`hook_settings_args`]) that runs `ccsm --job-complete <id>`
//! when the agent finishes responding, which drops a stamp file
//! ([`record_stop`]) the daemon reads. This is deterministic: the harness fires
//! it, so no amount of model drift can lose it.
//!
//! **The marker is the fallback**, for sessions ccsm merely adopted and so
//! never launched with a hook. A short protocol instruction asks the agent to
//! emit [`COMPLETION_MARKER`] on a line of its own once the work is finished,
//! delivered in the system prompt ([`system_prompt_args`]) and on every
//! continuation prompt ([`with_completion_protocol`]). It is genuinely
//! unreliable: a probe found that the identical `--append-system-prompt` text
//! produced the marker under `claude -p` but not in an interactive session,
//! which is why it is no longer the thing jobs depend on.
//!
//! When the marker is used, the daemon looks for it in the session's **own**
//! transcript rather than in the tmux pane, because the instruction itself is
//! echoed into the pane the moment it is pasted; scraping the pane would
//! match ccsm's own text and declare every job complete on dispatch. The
//! transcript distinguishes who said what, so only an `assistant` entry counts.

use super::Job;
use crate::data::types::{ContentValue, SessionEntry};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

/// The literal line an agent emits to report that its job is finished.
pub const COMPLETION_MARKER: &str = "CCSM_JOB_COMPLETE";

/// How much of the tail of a session transcript to read when looking for the
/// marker. Transcripts reach many megabytes, so reading the whole file every
/// poll would be pure waste, but the window has to be much wider than one
/// assistant turn: while the daemon is up the marker is at EOF and any size
/// works, and the case that matters is the daemon being *down* when the marker
/// lands. A real 1.0 MB transcript had its marker sitting 428 KB from EOF,
/// which the original 256 KB window would have missed forever.
pub const TRANSCRIPT_TAIL_BYTES: u64 = 2 * 1024 * 1024;

/// The protocol sentence appended to every prompt the daemon sends.
///
/// Written as one line because `live::send_prompt` collapses newlines when it
/// pastes into a pane, and phrased to discourage the agent from repeating the
/// marker conversationally (a mention inside a sentence is ignored by
/// [`transcript_shows_completion`], but not emitting it early is better still).
pub fn completion_instruction() -> String {
    format!(
        "When this task is completely finished and no work remains, make the \
         very last line of your final message exactly {COMPLETION_MARKER}, on \
         a line by itself and nothing else on that line. Do not write that \
         line for any other reason, and do not write it while work is still \
         outstanding."
    )
}

/// Append the completion protocol to a prompt. An empty prompt stays empty:
/// a job with no prompt is one the daemon starts at an idle session, and
/// sending the bare protocol instruction would be an instruction with no task.
///
/// A slash-command prompt is returned unchanged. Claude Code treats everything
/// after the command name as that command's `$ARGUMENTS`, newlines included, so
/// appending here does not instruct the agent at all: it silently corrupts the
/// command's argument (a probe of `/goal @PLAN.md` swallowed the whole protocol
/// paragraph into the goal condition). Those sessions get the protocol through
/// [`system_prompt_args`] instead.
pub fn with_completion_protocol(prompt: &str) -> String {
    if prompt.trim().is_empty() {
        return String::new();
    }
    if is_slash_command(prompt) {
        return prompt.to_string();
    }
    format!("{prompt}\n\n{}", completion_instruction())
}

/// True if `prompt` invokes a Claude Code slash command, i.e. its first
/// non-empty line begins with `/`.
pub fn is_slash_command(prompt: &str) -> bool {
    prompt
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .is_some_and(|line| line.starts_with('/'))
}

/// The argv fragment that carries the completion protocol in the session's
/// system prompt rather than in a user message.
///
/// This is the primary delivery channel for every session ccsm launches: it
/// applies for the whole session, survives slash-command prompts, and cannot be
/// absorbed into a command's arguments. Sessions ccsm merely adopted were
/// launched without it, which is why continuation prompts still carry the
/// instruction too.
pub fn system_prompt_args() -> Vec<String> {
    vec!["--append-system-prompt".to_string(), completion_instruction()]
}

// --- Stop hook -----------------------------------------------------------

/// Directory holding one stamp file per job whose agent has finished a turn.
/// Lives under `ccsm_dir()` alongside `schedule.json`, not in the user's
/// config, because it is daemon state rather than settings.
fn stamp_dir() -> Option<PathBuf> {
    crate::config::ccsm_dir().map(|d| d.join("completions"))
}

/// Path of `job_id`'s stop stamp. Returns `None` for an id that is not a plain
/// file name, so a malformed or hostile id can never make the hook write
/// outside the stamp directory.
pub fn stamp_path(job_id: &str) -> Option<PathBuf> {
    if job_id.is_empty()
        || job_id.contains('/')
        || job_id.contains('\\')
        || job_id.contains('\0')
        || job_id == "."
        || job_id == ".."
    {
        return None;
    }
    Some(stamp_dir()?.join(job_id))
}

/// Record that `job_id`'s agent finished responding. Called by
/// `ccsm --job-complete <id>`, which is what the `Stop` hook runs.
pub fn record_stop(job_id: &str) -> anyhow::Result<()> {
    let path = stamp_path(job_id).ok_or_else(|| anyhow::anyhow!("invalid job id"))?;
    let now = chrono::Utc::now().timestamp_millis();
    super::store::write_atomic(&path, now.to_string().as_bytes())
}

/// Drop any recorded stop for `job_id`. The daemon calls this every time it
/// gives the job new work (dispatch, relaunch, resume): without it a stamp
/// left over from the previous turn would complete the job the instant it
/// started running again, before the agent had done anything.
pub fn clear_stop(job_id: &str) {
    if let Some(path) = stamp_path(job_id) {
        let _ = std::fs::remove_file(path);
    }
}

/// Whether `job_id` has an uncleared stop stamp.
pub fn stop_recorded(job_id: &str) -> bool {
    stamp_path(job_id).is_some_and(|p| p.exists())
}

/// The argv fragment installing the `Stop` hook for `job_id`.
///
/// `--settings` takes inline JSON and *adds* to the user's settings rather than
/// replacing them, so this cannot clobber hooks they configured themselves.
/// Returns an empty vector when the ccsm executable cannot be located, which
/// degrades the job to the marker fallback rather than failing the launch.
pub fn hook_settings_args(job_id: &str) -> Vec<String> {
    let Ok(exe) = std::env::current_exe() else {
        return Vec::new();
    };
    if stamp_path(job_id).is_none() {
        return Vec::new();
    }
    let command = format!(
        "{} --job-complete {}",
        shell_quote(&exe.to_string_lossy()),
        shell_quote(job_id)
    );
    let settings = serde_json::json!({
        "hooks": {
            "Stop": [ { "hooks": [ { "type": "command", "command": command } ] } ]
        }
    });
    vec!["--settings".to_string(), settings.to_string()]
}

/// Wrap `s` in single quotes for a POSIX shell. Hook commands are run through
/// a shell, and the ccsm binary can sit under a path with spaces.
fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

// --- Completion marker ---------------------------------------------------

/// Path to the session transcript backing `job`, or `None` when it does not
/// exist yet. Dispatched jobs run under `--session-id <job id>`, so the job id
/// doubles as the session id until a discovered `claude_session_id` (adopted
/// sessions) says otherwise.
pub fn transcript_path(job: &Job) -> Option<PathBuf> {
    if job.cwd.is_empty() {
        return None;
    }
    let session_id = match job.claude_session_id.as_deref() {
        Some(id) if !id.is_empty() => id,
        _ => job.id.as_str(),
    };
    if session_id.is_empty() {
        return None;
    }
    crate::data::io::session_file_path(&job.cwd, session_id)
}

/// Read up to the last `max_bytes` of a transcript, discarding the leading
/// partial line when the read started mid-file so every returned line is a
/// complete JSON object.
pub fn read_transcript_tail(path: &Path, max_bytes: u64) -> Option<String> {
    let mut file = std::fs::File::open(path).ok()?;
    let len = file.metadata().ok()?.len();
    let start = len.saturating_sub(max_bytes);
    file.seek(SeekFrom::Start(start)).ok()?;
    let mut buf = Vec::new();
    file.read_to_end(&mut buf).ok()?;
    let text = String::from_utf8_lossy(&buf).into_owned();
    if start == 0 {
        return Some(text);
    }
    match text.find('\n') {
        Some(idx) => Some(text[idx + 1..].to_string()),
        None => Some(String::new()),
    }
}

/// True if any assistant turn in `tail` (a chunk of session JSONL) emitted the
/// completion marker on a line of its own, at or after `since_ms`.
///
/// Only `assistant` entries are considered, since the prompt carrying the
/// protocol instruction is itself a `user` entry containing the marker text.
/// Only `text` blocks are considered, so an agent that writes the marker into
/// a file through a tool call does not accidentally end its own job.
///
/// `since_ms` exists for adopted sessions: a job bound to a conversation that
/// ccsm already ran to completion would otherwise be marked done the instant
/// it was created, on the strength of a marker from the previous run. Entries
/// with no parseable timestamp are still accepted, so a change to the
/// transcript format degrades into "no time filter" rather than into "the
/// marker never works again".
pub fn transcript_shows_completion(tail: &str, since_ms: i64) -> bool {
    tail.lines()
        .any(|line| entry_reports_completion(line, since_ms))
}

/// True if one JSONL line is an assistant message, written no earlier than
/// `since_ms`, whose text has a line that is exactly the marker.
fn entry_reports_completion(line: &str, since_ms: i64) -> bool {
    let line = line.trim();
    // Cheap prefilter: parsing every entry of a large tail is the expensive
    // part, and the marker survives JSON escaping as a plain substring.
    if line.is_empty() || !line.contains(COMPLETION_MARKER) {
        return false;
    }
    let Ok(entry) = serde_json::from_str::<SessionEntry>(line) else {
        return false;
    };
    if entry.entry_type.as_deref() != Some("assistant") {
        return false;
    }
    if let Some(at_ms) = entry.timestamp.as_deref().and_then(parse_timestamp_ms) {
        if at_ms < since_ms {
            return false;
        }
    }
    let Some(message) = entry.message else {
        return false;
    };
    if message.role.as_deref().is_some_and(|r| r != "assistant") {
        return false;
    }
    match message.content {
        Some(ContentValue::Text(text)) => text_reports_completion(&text),
        Some(ContentValue::Blocks(blocks)) => blocks.iter().any(|block| {
            block.block_type.as_deref() == Some("text")
                && block.text.as_deref().is_some_and(text_reports_completion)
        }),
        None => false,
    }
}

/// True if some line of `text` is exactly the marker once trimmed.
fn text_reports_completion(text: &str) -> bool {
    text.lines().any(|l| l.trim() == COMPLETION_MARKER)
}

/// Parse a transcript entry's RFC3339 timestamp into epoch ms.
fn parse_timestamp_ms(timestamp: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(timestamp)
        .ok()
        .map(|dt| dt.timestamp_millis())
}

/// Whether `job`'s transcript shows the completion marker, emitted no earlier
/// than the job was created. Returns `false` when the transcript cannot be
/// located or read, so an unreadable file never ends a job that is still
/// working.
pub fn job_completed(job: &Job) -> bool {
    let Some(path) = transcript_path(job) else {
        return false;
    };
    let Some(tail) = read_transcript_tail(&path, TRANSCRIPT_TAIL_BYTES) else {
        return false;
    };
    transcript_shows_completion(&tail, job.created_at_ms)
}
