//! Which Codex rollout file represents a thread, and how one thread's several
//! rollout files reconstruct a single conversation.
//!
//! Codex does not keep one file per thread. `codex resume` appends to the same
//! rollout, but `thread/revert` keeps the thread id stable while switching the
//! thread to a new immutable rollout file, and a `paginated` thread's newer
//! rollout can *reference* an older one through `SessionMeta.history_base`
//! instead of copying its records. In that layout the newest file is a suffix,
//! not a superset — the ancestor's records stay behind in the prefix file.
//!
//! Two consequences this module exists to handle:
//!
//! - **Which file represents a thread is not a filesystem question.** Codex's own
//!   resolver (`codex-rs/thread-store/src/local/thread_rollout_resolver.rs`) puts
//!   it plainly: "after `thread/revert`, a scan could find an older immutable
//!   rollout for the same thread", so the Codex CLI's state database
//!   (`state_<N>.sqlite`, a sibling of `sessions/`) is the authority and a scan
//!   can only offer a fallback. [`select_canonical_index`] encodes that order.
//! - **A thread's conversation is its lineage**, walked through `history_base`
//!   oldest-first, where each segment contributes only its own records up to the
//!   byte offset its child recorded. [`resolve_lineage`] builds that list.
//!
//! Everything here is best-effort by design: the state database is a private,
//! versioned Codex artifact that may be absent, locked by a running Codex, or
//! have drifted to a schema we do not know. Every entry point degrades to a
//! sensible answer instead of failing, so no caller has to guard against it.
//!
//! Reference implementation, verified against the local Codex checkout:
//! `codex-rs/rollout/src/rollout_file_name.rs` (filename grammar),
//! `codex-rs/protocol/src/protocol.rs` (history mode and history base),
//! `codex-rs/thread-store/src/local/rollout_lineage.rs` (lineage walk),
//! `codex-rs/thread-store/src/local/delete_thread.rs` (delete reference guard).

use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{BufRead, BufReader, Read, Seek};
use std::path::{Path, PathBuf};
use std::time::Duration;

use rusqlite::{Connection, OpenFlags};
use serde_json::Value;

const STATE_DB_PREFIX: &str = "state_";
const STATE_DB_SUFFIX: &str = ".sqlite";
const CONFIG_FILE_NAME: &str = "config.toml";
const CODEX_SQLITE_HOME_ENV: &str = "CODEX_SQLITE_HOME";
const CODEX_SQLITE_HOME_CONFIG_KEY: &str = "sqlite_home";

/// `rollout-<YYYY-MM-DDTHH-MM-SS>-<ids>.jsonl`: the filename's fixed parts.
const ROLLOUT_PREFIX: &str = "rollout-";
const ROLLOUT_SUFFIX: &str = ".jsonl";
const ROLLOUT_TIMESTAMP_LEN: usize = 19;

/// Codex compresses cold rollouts in place (`<name>.jsonl.zst`) and reads them
/// back through a transparent decoder, so both spellings carry the same
/// identity. Identity, deletion and the `history_base` guard must all see the
/// compressed form: archived copies of a deleted thread are compressed first,
/// and a compressed reference is still a reference.
const ROLLOUT_COMPRESSED_SUFFIX: &str = ".zst";

const SESSIONS_DIR_NAME: &str = "sessions";
const ARCHIVED_SESSIONS_DIR_NAME: &str = "archived_sessions";

/// Bound on how many lines are read while looking for the `session_meta` record.
const HEAD_LINE_LIMIT: usize = 64;

/// Bound on how many decoded bytes that search may consume.
///
/// The `session_meta` record is a rollout's first line, so a byte cap costs
/// nothing for real files while keeping a damaged or hostile `.jsonl.zst` from
/// ballooning a list scan (a compressed file can decode to far more than its
/// size on disk).
pub(super) const HEAD_BYTE_LIMIT: u64 = 1 << 20;

/// Ceiling on the rollouts one [`rollout_cwds`] pass may read.
///
/// The pass exists to protect live sessions from a cleanup, so hitting the
/// ceiling must fail closed instead of returning a half-sampled answer.
const ROLLOUT_CWD_SCAN_LIMIT: usize = 50_000;

/// Reads are best-effort: a long lock retry would stall the session list, and we
/// always have the filesystem fallback. Wait briefly for a checkpoint, then give
/// up and let the caller fall back.
const STATE_DB_BUSY_TIMEOUT: Duration = Duration::from_millis(500);

/// The reference recorded in a child rollout that points at the prefix it did not
/// copy.
///
/// Mirrors `codex_protocol::protocol::HistoryPosition`, including its confusing
/// field name: `thread_id` holds a **rollout id**, not a thread id. Codex's own
/// doc comment says to "Treat its value as a `rollout_id`".
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct HistoryBase {
    pub(super) rollout_id: String,
    /// First rollout ordinal not included from the prefix file.
    pub(super) end_ordinal_exclusive: u64,
    /// Byte offset immediately after the last included prefix record.
    pub(super) end_byte_offset: u64,
}

/// The parts of a rollout's `session_meta` record this module needs.
#[derive(Debug, Clone)]
pub(super) struct RolloutHead {
    pub(crate) thread_id: String,
    /// `history_mode == "paginated"`, the only mode that keeps rollouts.
    pub(super) paginated: bool,
    pub(super) history_base: Option<HistoryBase>,
    /// The working directory Codex recorded for the thread.
    ///
    /// For a project-less chat this is the scratch workspace the Codex app
    /// created, which is what the delete-time and residue cleanups match
    /// against. `None` for a damaged head.
    pub(super) cwd: Option<String>,
}

/// The rollout ids encoded in a canonical Codex rollout filename.
///
/// Ordinary files encode one id that is both ids. A reverted thread's file
/// appends an underscore and a distinct rollout id after the stable thread id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RolloutFileName {
    pub(crate) thread_id: String,
    pub(crate) rollout_id: String,
}

impl RolloutFileName {
    /// Parses `rollout-<ts>-<thread>[ _<rollout>].jsonl[.zst]`.
    ///
    /// The timestamp is a fixed 19 characters (`2026-09-17T09-00-00`), so the
    /// ids start right after it plus the separator. Following Codex, the ids
    /// must be UUIDs — that keeps unrelated `.jsonl` files in the tree from
    /// parsing as rollouts. The optional compression suffix is Codex's own
    /// cold-rollout representation of the same file.
    pub(crate) fn parse(name: &str) -> Option<Self> {
        let name = name.strip_suffix(ROLLOUT_COMPRESSED_SUFFIX).unwrap_or(name);
        let core = name
            .strip_prefix(ROLLOUT_PREFIX)?
            .strip_suffix(ROLLOUT_SUFFIX)?;
        if !is_rollout_timestamp(core.get(..ROLLOUT_TIMESTAMP_LEN)?) {
            return None;
        }
        if core.get(ROLLOUT_TIMESTAMP_LEN..ROLLOUT_TIMESTAMP_LEN + 1)? != "-" {
            return None;
        }
        let ids = core.get(ROLLOUT_TIMESTAMP_LEN + 1..)?;
        let (thread_id, rollout_id) = ids.split_once('_').unwrap_or((ids, ids));
        if !is_uuid(thread_id) || !is_uuid(rollout_id) {
            return None;
        }
        Some(Self {
            thread_id: thread_id.to_string(),
            rollout_id: rollout_id.to_string(),
        })
    }
}

/// Parses a path's file name as a rollout name.
pub(crate) fn parse_rollout_path(path: &Path) -> Option<RolloutFileName> {
    RolloutFileName::parse(path.file_name()?.to_str()?)
}

/// The conversation of one thread, oldest segment first.
///
/// Built by [`resolve_lineage`]. A thread with a single rollout (the common
/// case, and every `legacy` thread) yields exactly one unbounded segment, so
/// callers can read a lineage unconditionally.
#[derive(Debug, Clone)]
pub(crate) struct Lineage {
    pub(crate) segments: Vec<LineageSegment>,
}

#[derive(Debug, Clone)]
pub(crate) struct LineageSegment {
    pub(crate) path: PathBuf,
    /// Read this file only up to here.
    ///
    /// `Some` for every ancestor: the value is the byte offset its child recorded
    /// in `history_base`, i.e. how much of the prefix the child depended on.
    /// `None` for the thread's own newest rollout, which is read to the end.
    pub(crate) end_byte_offset: Option<u64>,
}

impl Lineage {
    /// A single-segment lineage that reads one file end to end.
    pub(crate) fn single(path: PathBuf) -> Self {
        Self {
            segments: vec![LineageSegment {
                path,
                end_byte_offset: None,
            }],
        }
    }
}

/// Resolve which rollout file Codex itself would open for this thread.
///
/// Returns an index into `candidates`, where each candidate is
/// `(source_path, activity timestamp)`. `authority` is the state-database index
/// when one could be read.
///
/// Order follows Codex's resolver: the database's selected rollout path wins
/// when it names one of the artifacts we actually found; otherwise the newest
/// artifact wins, with the greater path breaking ties so the pick stays
/// deterministic. A scan cannot resolve the reverted case, which is exactly why
/// the database is consulted first.
pub(super) fn select_canonical_index(
    authority: Option<&CodexStateIndex>,
    session_id: &str,
    candidates: &[(String, i64)],
) -> Option<usize> {
    if candidates.is_empty() {
        return None;
    }

    if let Some(selected) = authority.and_then(|index| index.selected_path(session_id)) {
        if let Some(position) = candidates
            .iter()
            .position(|(source_path, _)| same_rollout_path(source_path, selected))
        {
            return Some(position);
        }
        // The database names a file we did not scan (archived, deleted, or moved
        // since it was last repaired). Codex would repair the row; we must not
        // write to its database, so fall through to the newest artifact rather
        // than hiding a session the user can plainly see on disk.
    }

    candidates
        .iter()
        .enumerate()
        .max_by(
            |(left_index, (left_path, left_ts)), (right_index, (right_path, right_ts))| {
                (left_ts, left_path, left_index).cmp(&(right_ts, right_path, right_index))
            },
        )
        .map(|(position, _)| position)
}

/// Codex's own state database, read as a selection hint.
///
/// Deliberately narrow: this exposes only the rollout path Codex selected for a
/// thread. Session content stays sourced from the rollout files (the module's
/// documented source of truth), so a stale or schema-drifted database can only
/// cost us the selection hint, never the rows themselves.
#[derive(Debug, Default)]
pub(super) struct CodexStateIndex {
    /// Thread id -> the rollout path Codex selected for it.
    selected_rollout_paths: HashMap<String, String>,
}

impl CodexStateIndex {
    /// Reads the thread table, or `None` when the database cannot be used.
    ///
    /// Every failure — no database, unknown schema, a running Codex holding the
    /// write lock — collapses to `None`, which callers treat as "no hint".
    pub(super) fn load(codex_home: &Path) -> Option<Self> {
        let db_path = resolve_state_db_path(codex_home)?;
        let connection = Connection::open_with_flags(
            &db_path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .ok()?;
        connection.busy_timeout(STATE_DB_BUSY_TIMEOUT).ok()?;

        let mut statement = connection
            .prepare("SELECT id, rollout_path FROM threads")
            .ok()?;
        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, Option<String>>(0)?,
                    row.get::<_, Option<String>>(1)?,
                ))
            })
            .ok()?;

        let mut selected_rollout_paths = HashMap::new();
        for row in rows.flatten() {
            let (Some(thread_id), Some(rollout_path)) = row else {
                continue;
            };
            if thread_id.is_empty() || rollout_path.is_empty() {
                continue;
            }
            selected_rollout_paths.insert(thread_id, rollout_path);
        }

        Some(Self {
            selected_rollout_paths,
        })
    }

    pub(super) fn selected_path(&self, thread_id: &str) -> Option<&str> {
        self.selected_rollout_paths
            .get(thread_id)
            .map(String::as_str)
    }
}

/// Locate the Codex state database under `codex_home`.
///
/// The file name carries a schema version (`state_5.sqlite` today), so the whole
/// `state_<N>.sqlite` family is matched and the highest version wins — an exact
/// name would silently stop resolving the day Codex bumps it.
///
/// An explicitly configured SQLite home (`config.toml`'s `sqlite_home`, then
/// `CODEX_SQLITE_HOME`) takes precedence over the Codex home itself, matching how
/// Codex resolves its own `SqliteConfig`.
pub(super) fn resolve_state_db_path(codex_home: &Path) -> Option<PathBuf> {
    let configured = sqlite_home_from_config(codex_home).or_else(sqlite_home_from_env);
    for directory in configured
        .into_iter()
        .chain(std::iter::once(codex_home.to_path_buf()))
    {
        if let Some(path) = highest_state_db_in(&directory) {
            return Some(path);
        }
    }
    None
}

fn highest_state_db_in(directory: &Path) -> Option<PathBuf> {
    let entries = std::fs::read_dir(directory).ok()?;
    let mut best: Option<(u64, PathBuf)> = None;
    for entry in entries.flatten() {
        let Some(name) = entry.file_name().to_str().map(str::to_string) else {
            continue;
        };
        let Some(version) = name
            .strip_prefix(STATE_DB_PREFIX)
            .and_then(|rest| rest.strip_suffix(STATE_DB_SUFFIX))
            .and_then(|version| version.parse::<u64>().ok())
        else {
            continue;
        };
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        if best.as_ref().is_none_or(|(seen, _)| version > *seen) {
            best = Some((version, path));
        }
    }
    best.map(|(_, path)| path)
}

fn sqlite_home_from_config(codex_home: &Path) -> Option<PathBuf> {
    let text = std::fs::read_to_string(codex_home.join(CONFIG_FILE_NAME)).ok()?;
    let document = text.parse::<toml_edit::DocumentMut>().ok()?;
    let raw = document.get(CODEX_SQLITE_HOME_CONFIG_KEY)?.as_str()?.trim();
    if raw.is_empty() {
        return None;
    }
    Some(resolve_user_path(raw))
}

fn sqlite_home_from_env() -> Option<PathBuf> {
    let raw = std::env::var(CODEX_SQLITE_HOME_ENV).ok()?;
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    Some(resolve_user_path(raw))
}

/// Expands a leading `~`, matching how the Codex history modules read the same
/// setting.
fn resolve_user_path(raw: &str) -> PathBuf {
    for prefix in ["~/", "~\\"] {
        if let Some(rest) = raw.strip_prefix(prefix) {
            if let Some(home) = dirs::home_dir() {
                return home.join(rest);
            }
        }
    }
    if raw == "~" {
        return dirs::home_dir().unwrap_or_else(|| PathBuf::from(raw));
    }
    PathBuf::from(raw)
}

/// Whether a path names a compressed rollout (`<name>.jsonl.zst`).
pub(super) fn is_compressed_rollout_path(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.ends_with(ROLLOUT_COMPRESSED_SUFFIX))
}

/// Open a rollout for reading, transparently decoding the compressed spelling.
///
/// Codex rewrites cold rollouts as `.jsonl.zst` and reads them back through its
/// own transparent reader (`open_rollout_line_reader`). A caller that opened the
/// file directly would read compressed bytes as text and mistake a real rollout
/// for an unreadable one.
///
/// `byte_limit` bounds the *decoded* bytes, which is what keeps a compressed
/// file from ballooning a scan that only wants its first record.
pub(crate) fn open_rollout_reader(
    path: &Path,
    byte_limit: Option<u64>,
) -> Option<Box<dyn BufRead>> {
    let file = File::open(path).ok()?;
    if is_compressed_rollout_path(path) {
        let decoder = zstd::stream::read::Decoder::new(file).ok()?;
        Some(match byte_limit {
            Some(limit) => Box::new(BufReader::new(decoder.take(limit))),
            None => Box::new(BufReader::new(decoder)),
        })
    } else {
        let reader = BufReader::new(file);
        Some(match byte_limit {
            Some(limit) => Box::new(BufReader::new(reader.take(limit))),
            None => Box::new(reader),
        })
    }
}

/// Read the `session_meta` record of a rollout file.
///
/// Both the line count and the decoded byte count are bounded: the record is the
/// first line, so the caps only ever stop a pathological file.
pub(super) fn read_rollout_head(path: &Path) -> Option<RolloutHead> {
    read_rollout_head_or_unreadable(path).ok().flatten()
}

/// Read a rollout's head, telling "this rollout has no head" apart from "this
/// rollout could not be read at all".
///
/// A caller that treats the head as a claim about the world — the cleanup's
/// reference scan — has to know the difference: a record that is simply absent
/// hides nothing, while a file that cannot be opened (or, for `.jsonl.zst`,
/// cannot be decoded) may hold a reference it never got to see.
fn read_rollout_head_or_unreadable(path: &Path) -> Result<Option<RolloutHead>, HeadUnreadable> {
    let reader = open_rollout_reader(path, Some(HEAD_BYTE_LIMIT)).ok_or(HeadUnreadable)?;
    let mut read_error = false;

    for line in reader.lines().take(HEAD_LINE_LIMIT) {
        let line = match line {
            Ok(line) => line,
            Err(_) => {
                read_error = true;
                continue;
            }
        };
        if line.trim().is_empty() {
            continue;
        }
        let Ok(value) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if value.get("type").and_then(Value::as_str) != Some("session_meta") {
            continue;
        }
        // The record we came for was readable, so whatever failed later is not
        // this caller's problem.
        return Ok(value.get("payload").map(rollout_head_from_session_meta));
    }

    // No `session_meta` record. Content that could not be read is unknown (a
    // damaged or not-actually-compressed file), while content that read fine and
    // simply has no such record holds no working directory to hide.
    if read_error {
        Err(HeadUnreadable)
    } else {
        Ok(None)
    }
}

/// A rollout file that could not be read.
#[derive(Debug)]
struct HeadUnreadable;

fn rollout_head_from_session_meta(payload: &Value) -> RolloutHead {
    // An absent `history_mode` means `legacy`: that is Codex's serde default, and
    // the field only appeared with paginated rollouts.
    let paginated = payload
        .get("history_mode")
        .and_then(Value::as_str)
        .is_some_and(|mode| mode.eq_ignore_ascii_case("paginated"));

    RolloutHead {
        thread_id: payload
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        paginated,
        history_base: history_base_from_session_meta(payload),
        cwd: payload
            .get("cwd")
            .and_then(Value::as_str)
            .map(str::to_string),
    }
}

/// The working directories of every rollout still on disk.
#[derive(Debug, Clone, Default)]
pub(super) struct RolloutCwds {
    pub(super) cwds: HashSet<String>,
    /// `false` when the scan stopped at [`ROLLOUT_CWD_SCAN_LIMIT`], i.e. the cwd
    /// set is a sample. Callers use it to protect live data from a cleanup they
    /// cannot prove is safe.
    pub(super) complete: bool,
}

/// Collect the `cwd` of every rollout under `sessions/` and its sibling
/// `archived_sessions/`.
///
/// This is the reference check behind every Codex scratch cleanup: a workspace is
/// leftovers only when no rollout — active *or* archived — still records it as
/// its working directory. Archived rollouts count because Codex can resume a
/// thread from there even though our session list does not show it.
///
/// Only each rollout's first record is read (see [`read_rollout_head`]), so this
/// stays far cheaper than the metadata scan behind the session list.
///
/// Anything that could hide a rollout — an unreachable root, an unreadable
/// directory, an unreadable head — marks the result incomplete instead of being
/// skipped, because the caller's promise is "nothing is removed while a
/// reference set could not be read in full". See [`rollout_root_state`] for why
/// a missing root is not enough to answer that question.
pub(super) fn rollout_cwds(sessions_root: &Path) -> RolloutCwds {
    let mut roots = vec![sessions_root.to_path_buf()];
    if let Some(archived) = archived_sessions_root(sessions_root) {
        roots.push(archived);
    }

    let mut scan = RolloutCwds {
        cwds: HashSet::new(),
        complete: true,
    };
    let mut visited = 0usize;

    for root in roots {
        match rollout_root_state(&root) {
            RolloutRootState::Readable => {}
            RolloutRootState::Absent => continue,
            RolloutRootState::Unreachable => {
                scan.complete = false;
                continue;
            }
        }

        let mut pending = vec![root];
        while let Some(directory) = pending.pop() {
            let Ok(entries) = std::fs::read_dir(&directory) else {
                // It was listed a moment ago, so a failure here is not an absent
                // directory: rollouts may be hiding behind it.
                scan.complete = false;
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    pending.push(path);
                    continue;
                }
                if parse_rollout_path(&path).is_none() {
                    continue;
                }
                if visited >= ROLLOUT_CWD_SCAN_LIMIT {
                    scan.complete = false;
                    return scan;
                }
                visited += 1;

                match read_rollout_head_or_unreadable(&path) {
                    Ok(Some(head)) => {
                        if let Some(cwd) = head.cwd {
                            scan.cwds.insert(cwd);
                        }
                    }
                    // Readable, just without a `session_meta` record: it has no
                    // working directory to hide.
                    Ok(None) => {}
                    // A rollout that cannot be read may name a workspace.
                    Err(HeadUnreadable) => scan.complete = false,
                }
            }
        }
    }

    scan
}

/// How a rollout root looks from here.
enum RolloutRootState {
    /// Nothing is there: a Codex home that simply never had rollouts.
    Absent,
    Readable,
    /// The path cannot be reached at all.
    Unreachable,
}

/// Whether a rollout root can be read — or is plainly absent rather than
/// unreachable.
///
/// "Not found" is ambiguous: an absent directory and a stopped WSL distro report
/// it alike, and reading a dead share as "no rollouts here" would let the cleanup
/// treat every workspace as leftovers. A WSL path is therefore asked about its
/// distro root, which is present whenever the distro is.
fn rollout_root_state(root: &Path) -> RolloutRootState {
    match std::fs::read_dir(root) {
        Ok(_) => RolloutRootState::Readable,
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => RolloutRootState::Unreachable,
        Err(_) if wsl_distro_is_unreachable(root) => RolloutRootState::Unreachable,
        Err(_) => RolloutRootState::Absent,
    }
}

/// Whether a path sits inside a WSL distro that answers nothing right now.
fn wsl_distro_is_unreachable(path: &Path) -> bool {
    let display = path.to_string_lossy();
    let Some(location) = crate::coding::runtime_location::parse_wsl_unc_path(&display) else {
        return false;
    };
    let distro_root =
        crate::coding::runtime_location::build_windows_unc_path(&location.distro, "/");
    std::fs::read_dir(distro_root).is_err()
}

/// The `sessions/` root a rollout path lives under.
///
/// Codex files rollouts in a dated tree (`sessions/<year>/<month>/<day>/`), so
/// the root is the nearest ancestor named `sessions` — the anchor
/// [`resolve_lineage`] needs to follow `history_base` pointers across the tree.
/// `None` for a path outside that layout (an archived copy moved elsewhere),
/// which callers treat as "cannot follow a chain from here".
pub(crate) fn sessions_root_of(path: &Path) -> Option<PathBuf> {
    path.ancestors()
        .find(|ancestor| {
            ancestor
                .file_name()
                .is_some_and(|name| name == SESSIONS_DIR_NAME)
        })
        .map(Path::to_path_buf)
}

/// `<codex_home>/archived_sessions`, the sibling of `sessions/`.
pub(super) fn archived_sessions_root(sessions_root: &Path) -> Option<PathBuf> {
    sessions_root
        .parent()
        .map(|home| home.join(ARCHIVED_SESSIONS_DIR_NAME))
}

fn history_base_from_session_meta(payload: &Value) -> Option<HistoryBase> {
    let base = payload.get("history_base")?;
    if base.is_null() {
        return None;
    }
    Some(HistoryBase {
        rollout_id: base.get("thread_id")?.as_str()?.to_string(),
        end_ordinal_exclusive: base.get("end_ordinal_exclusive")?.as_u64()?,
        end_byte_offset: base.get("end_byte_offset")?.as_u64()?,
    })
}

/// Walk a rollout's `history_base` chain into an ordered lineage.
///
/// Whenever the chain cannot be followed further — a `legacy` rollout, a missing
/// prefix file, a broken cutoff, a cycle — the walk stops and the segments it did
/// verify are kept. Every kept segment's records are bounded by the cutoff its
/// child recorded, so a partial chain replays real conversation and never
/// unrelated records; dropping the whole walk instead would throw away the
/// segments closest to the leaf, which were validated.
///
/// A chain that cannot be started at all — an unreadable or `legacy` rollout —
/// yields one unbounded segment for the file we were given, so a caller that
/// cannot follow a chain still shows that file rather than nothing.
pub(crate) fn resolve_lineage(sessions_root: &Path, source_path: &Path) -> Lineage {
    let single = || Lineage::single(source_path.to_path_buf());

    let Some(head) = read_rollout_head(source_path) else {
        return single();
    };
    // Only paginated threads keep rollouts. A legacy rollout that happens to name
    // a history base is not something Codex itself would follow.
    if !head.paginated {
        return single();
    }

    let mut segments: Vec<LineageSegment> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    let mut current_path = source_path.to_path_buf();
    let mut current_head = head;
    // The cutoff bounding the segment about to be pushed, recorded by its child.
    let mut end_byte_offset: Option<u64> = None;

    while let Some(name) = parse_rollout_path(&current_path) {
        // A cycle would loop forever; stop and keep the verified prefix.
        if !seen.insert(name.rollout_id.clone()) {
            break;
        }
        segments.push(LineageSegment {
            path: current_path.clone(),
            end_byte_offset,
        });

        let Some(base) = current_head.history_base.clone() else {
            break;
        };
        // Codex rejects a cutoff that would swallow the prefix's own metadata.
        if base.end_ordinal_exclusive == 0 {
            break;
        }
        let Some(prefix_path) = find_rollout_by_rollout_id(sessions_root, &base.rollout_id) else {
            break;
        };
        // Codex validates the same way before trusting a cutoff: the prefix must
        // still be at least as long as the child said it depended on.
        if !rollout_contains_prefix(&prefix_path, base.end_byte_offset) {
            break;
        }
        let Some(prefix_head) = read_rollout_head(&prefix_path) else {
            break;
        };

        current_path = prefix_path;
        current_head = prefix_head;
        end_byte_offset = Some(base.end_byte_offset);
    }

    if segments.is_empty() {
        return single();
    }
    // The walk ran newest-first; a conversation reads oldest-first.
    segments.reverse();
    Lineage { segments }
}

/// Find the rollout file carrying `rollout_id`, searching active and archived
/// sessions.
///
/// Mirrors Codex's `find_rollout_path_by_rollout_id`: unlike the thread lookup,
/// this does not consult the database and does not choose among several rollouts
/// of one thread — it is only for following a `history_base` pointer.
fn find_rollout_by_rollout_id(sessions_root: &Path, rollout_id: &str) -> Option<PathBuf> {
    let mut roots = vec![sessions_root.to_path_buf()];
    if let Some(archived) = sessions_root
        .parent()
        .map(|home| home.join(ARCHIVED_SESSIONS_DIR_NAME))
    {
        roots.push(archived);
    }

    for root in roots {
        if let Some(path) = find_rollout_in_tree(&root, rollout_id) {
            return Some(path);
        }
    }
    None
}

fn find_rollout_in_tree(root: &Path, rollout_id: &str) -> Option<PathBuf> {
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        let Ok(entries) = std::fs::read_dir(&directory) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                pending.push(path);
                continue;
            }
            if parse_rollout_path(&path).is_some_and(|name| name.rollout_id == rollout_id) {
                return Some(path);
            }
        }
    }
    None
}

/// Whether a prefix file still holds the bytes its child depended on.
///
/// A compressed prefix is measured by its *decoded* length: the cutoff is a
/// plain-text offset, so an on-disk size is not comparable to it. Mirrors
/// Codex's own `rollout_contains_prefix`, which answers from the zstd frame
/// header when that is enough and decodes only up to the bound otherwise.
fn rollout_contains_prefix(path: &Path, end_byte_offset: u64) -> bool {
    let Ok(mut file) = File::open(path) else {
        return false;
    };

    if !is_compressed_rollout_path(path) {
        return file
            .metadata()
            .map(|metadata| metadata.len() >= end_byte_offset)
            .unwrap_or(false);
    }

    // A zstd frame header occupies at most 18 bytes.
    let mut header = [0u8; 18];
    let Ok(read) = file.read(&mut header) else {
        return false;
    };
    if zstd::zstd_safe::get_frame_content_size(&header[..read])
        .ok()
        .flatten()
        .is_some_and(|size| end_byte_offset <= size)
    {
        return true;
    }

    if file.rewind().is_err() {
        return false;
    }
    let Ok(decoder) = zstd::stream::read::Decoder::new(file) else {
        return false;
    };
    let mut prefix = decoder.take(end_byte_offset);
    std::io::copy(&mut prefix, &mut std::io::sink())
        .map(|copied| copied == end_byte_offset)
        .unwrap_or(false)
}

/// Collect the rollout ids that some *other* thread names as its history base.
///
/// Deleting a rollout that another thread still depends on would break that
/// thread's replay, so [`crate::coding::session_manager::codex::delete_session`]
/// refuses in that case. This mirrors Codex's own
/// `ensure_no_external_references` guard before a thread delete.
pub(super) fn rollout_ids_referenced_by_other_threads(
    sessions_root: &Path,
    thread_id: &str,
) -> HashSet<String> {
    let mut roots = vec![sessions_root.to_path_buf()];
    if let Some(archived) = sessions_root
        .parent()
        .map(|home| home.join(ARCHIVED_SESSIONS_DIR_NAME))
    {
        roots.push(archived);
    }

    let mut referenced = HashSet::new();
    for root in roots {
        let mut pending = vec![root];
        while let Some(directory) = pending.pop() {
            let Ok(entries) = std::fs::read_dir(&directory) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    pending.push(path);
                    continue;
                }
                if parse_rollout_path(&path).is_none() {
                    continue;
                }
                let Some(head) = read_rollout_head(&path) else {
                    continue;
                };
                // A rollout referencing its own thread's prefix is not an external
                // reference; Codex skips that case too.
                if head.thread_id == thread_id {
                    continue;
                }
                if let Some(base) = head.history_base {
                    referenced.insert(base.rollout_id);
                }
            }
        }
    }
    referenced
}

/// The rollout ids a session's artifacts carry, for the delete guard.
pub(super) fn rollout_ids_of(paths: &[PathBuf]) -> HashSet<String> {
    paths
        .iter()
        .filter_map(|path| parse_rollout_path(path).map(|name| name.rollout_id))
        .collect()
}

/// Compare two rollout paths from different sources (a database column and a
/// directory scan) without caring about separators or platform path casing.
pub(super) fn same_rollout_path(left: &str, right: &str) -> bool {
    normalize_path_for_compare(left) == normalize_path_for_compare(right)
}

/// Normalize a path for the comparison above.
///
/// Codex keeps `<name>.jsonl` in `state_<N>.sqlite` even after cold-compressing
/// the file on disk to `<name>.jsonl.zst`, so strip an optional `.zst` suffix
/// off `.jsonl.zst` paths before comparing.
fn normalize_path_for_compare(path: &str) -> String {
    let mut normalized = path.trim().replace('\\', "/");
    if cfg!(windows) {
        normalized = normalized.to_lowercase();
    }
    if let Some(stem) = normalized
        .strip_suffix(ROLLOUT_COMPRESSED_SUFFIX)
        .filter(|stem| stem.ends_with(ROLLOUT_SUFFIX))
    {
        normalized.truncate(stem.len());
    }
    normalized
}

fn is_rollout_timestamp(candidate: &str) -> bool {
    candidate.len() == ROLLOUT_TIMESTAMP_LEN
        && candidate.chars().enumerate().all(|(index, value)| {
            match index {
                // `YYYY-MM-DDTHH-MM-SS`
                4 | 7 => value == '-',
                10 => value == 'T',
                13 | 16 => value == '-',
                _ => value.is_ascii_digit(),
            }
        })
}

fn is_uuid(candidate: &str) -> bool {
    candidate.len() == 36
        && candidate
            .chars()
            .enumerate()
            .all(|(index, value)| match index {
                8 | 13 | 18 | 23 => value == '-',
                _ => value.is_ascii_hexdigit(),
            })
}

#[cfg(test)]
mod tests {
    use std::fs;

    use rusqlite::{params, Connection};
    use serde_json::{json, Value};

    use super::*;

    const PREFIX_ID: &str = "01a08e7d-5f4b-7c31-9a20-6d3f11b91882";
    const CHILD_ROLLOUT_ID: &str = "01a08e80-1111-7c31-9a20-6d3f11b91882";
    const OTHER_ID: &str = "01a08e11-2c7d-7b55-8e10-4a9c77d20453";
    /// A rollout id no file on disk carries.
    const ABSENT_ROLLOUT_ID: &str = "01a08e99-9999-7c31-9a20-6d3f11b91882";

    /// Writes a rollout whose first line is a `session_meta` record, plus a body
    /// line so the file has a non-trivial byte length. Returns the file length.
    fn write_rollout(
        path: &Path,
        thread_id: &str,
        history_mode: Option<&str>,
        history_base: Option<Value>,
    ) -> u64 {
        let mut payload = json!({ "id": thread_id, "cwd": "/tmp/project" });
        if let Some(mode) = history_mode {
            payload["history_mode"] = json!(mode);
        }
        if let Some(base) = history_base {
            payload["history_base"] = base;
        }
        let head = json!({
            "timestamp": "2026-09-17T10:00:00Z",
            "type": "session_meta",
            "payload": payload,
        });
        let body = json!({
            "timestamp": "2026-09-17T10:00:01Z",
            "ordinal": 1,
            "type": "event_msg",
            "payload": { "type": "agent_message", "message": "hello" },
        });

        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("failed to create rollout parent");
        }
        fs::write(path, format!("{head}\n{body}\n")).expect("failed to write rollout");
        fs::metadata(path).expect("rollout should exist").len()
    }

    /// Writes a rollout in Codex's cold representation: the same records, zstd
    /// compressed, as `<name>.jsonl.zst`.
    fn write_compressed_rollout(
        path: &Path,
        thread_id: &str,
        history_base: Option<Value>,
        cwd: &str,
    ) {
        let mut payload = json!({ "id": thread_id, "cwd": cwd });
        if let Some(base) = history_base {
            payload["history_mode"] = json!("paginated");
            payload["history_base"] = base;
        }
        let head = json!({
            "timestamp": "2026-09-17T10:00:00Z",
            "type": "session_meta",
            "payload": payload,
        });

        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("failed to create rollout parent");
        }
        let content = format!("{head}\n");
        let compressed =
            zstd::stream::encode_all(content.as_bytes(), 3).expect("failed to compress rollout");
        fs::write(path, compressed).expect("failed to write compressed rollout");
    }

    fn write_state_db(home: &Path, name: &str, rows: &[(&str, &str)]) {
        let connection = Connection::open(home.join(name)).expect("failed to create state db");
        connection
            .execute_batch("CREATE TABLE threads (id TEXT PRIMARY KEY, rollout_path TEXT)")
            .expect("failed to create threads table");
        for (id, rollout_path) in rows {
            connection
                .execute(
                    "INSERT INTO threads (id, rollout_path) VALUES (?1, ?2)",
                    params![id, rollout_path],
                )
                .expect("failed to insert thread row");
        }
    }

    #[test]
    fn parses_ordinary_and_reverted_rollout_names() {
        let ordinary =
            RolloutFileName::parse(&format!("rollout-2026-09-17T09-00-00-{PREFIX_ID}.jsonl"))
                .expect("ordinary name should parse");
        assert_eq!(ordinary.thread_id, PREFIX_ID);
        assert_eq!(ordinary.rollout_id, PREFIX_ID);

        // `thread/revert` appends a distinct rollout id after an underscore.
        let reverted = RolloutFileName::parse(&format!(
            "rollout-2026-09-17T09-00-00-{PREFIX_ID}_{CHILD_ROLLOUT_ID}.jsonl"
        ))
        .expect("reverted name should parse");
        assert_eq!(reverted.thread_id, PREFIX_ID);
        assert_eq!(reverted.rollout_id, CHILD_ROLLOUT_ID);

        // Codex compresses cold rollouts in place; the identity is unchanged.
        let compressed = RolloutFileName::parse(&format!(
            "rollout-2026-09-17T09-00-00-{PREFIX_ID}.jsonl.zst"
        ))
        .expect("compressed name should parse");
        assert_eq!(compressed.thread_id, PREFIX_ID);
        assert_eq!(compressed.rollout_id, PREFIX_ID);
    }

    /// A compressed rollout must answer with its thread id and cwd like a plain
    /// one: without that, a compressed reference is invisible and a compressed
    /// session looks like an unreferenced workspace.
    #[test]
    fn reads_a_compressed_rollout_head() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir
            .path()
            .join(format!("rollout-2026-09-17T09-00-00-{PREFIX_ID}.jsonl.zst"));
        write_compressed_rollout(&path, PREFIX_ID, None, "/tmp/scratch");

        let head = read_rollout_head(&path).expect("compressed head should parse");
        assert_eq!(head.thread_id, PREFIX_ID);
        assert_eq!(head.cwd.as_deref(), Some("/tmp/scratch"));
        assert!(!head.paginated);
    }

    /// The reference guard reads both rollout roots and both representations, so
    /// a compressed archived thread still protects the history it replays from.
    #[test]
    fn collects_references_from_compressed_rollouts() {
        let dir = tempfile::tempdir().expect("tempdir");
        let sessions_root = dir.path().join("sessions");
        let archived_root = dir.path().join("archived_sessions");

        write_rollout(
            &sessions_root
                .join("2026")
                .join("09")
                .join("17")
                .join(format!("rollout-2026-09-17T09-00-00-{PREFIX_ID}.jsonl")),
            PREFIX_ID,
            Some("paginated"),
            None,
        );
        write_compressed_rollout(
            &archived_root.join(format!("rollout-2026-09-18T09-00-00-{OTHER_ID}.jsonl.zst")),
            OTHER_ID,
            Some(json!({
                "thread_id": PREFIX_ID,
                "end_ordinal_exclusive": 2,
                "end_byte_offset": 128,
            })),
            "/tmp/project",
        );

        let referenced = rollout_ids_referenced_by_other_threads(&sessions_root, PREFIX_ID);
        assert!(
            referenced.contains(PREFIX_ID),
            "a compressed archived rollout is still a reference"
        );
    }

    /// The cwd set behind a scratch cleanup spans `sessions/` and
    /// `archived_sessions/`, including compressed files.
    #[test]
    fn collects_cwds_across_active_and_archived_rollouts() {
        let dir = tempfile::tempdir().expect("tempdir");
        let sessions_root = dir.path().join("sessions");
        let archived_root = dir.path().join("archived_sessions");

        write_rollout(
            &sessions_root
                .join("2026")
                .join("09")
                .join("17")
                .join(format!("rollout-2026-09-17T09-00-00-{PREFIX_ID}.jsonl")),
            PREFIX_ID,
            Some("legacy"),
            None,
        );
        write_compressed_rollout(
            &archived_root.join(format!("rollout-2026-09-18T09-00-00-{OTHER_ID}.jsonl.zst")),
            OTHER_ID,
            None,
            "/tmp/scratch-workspace",
        );

        let scan = rollout_cwds(&sessions_root);
        assert!(scan.complete);
        assert!(scan.cwds.contains("/tmp/project"));
        assert!(scan.cwds.contains("/tmp/scratch-workspace"));
    }

    /// A Codex home without any rollouts is not an unknown: there is nothing to
    /// read, and the cleanup may proceed. Most users never archive, so an absent
    /// `archived_sessions/` must not disable the whole feature.
    #[test]
    fn absent_rollout_roots_stay_complete() {
        let dir = tempfile::tempdir().expect("tempdir");
        let sessions_root = dir.path().join("sessions");

        let scan = rollout_cwds(&sessions_root);
        assert!(scan.complete, "an absent root is empty, not unknown");
        assert!(scan.cwds.is_empty());
    }

    /// A distro that is not running answers "not found" for every path inside it,
    /// exactly like an absent directory does — so the scan asks the distro itself.
    /// Reading a dead share as "no rollouts here" would offer every workspace in
    /// the user's documents as leftovers.
    #[test]
    fn a_dead_wsl_distro_makes_the_scan_incomplete() {
        let distro = format!(
            "ai-toolbox-no-such-distro-{}",
            uuid::Uuid::new_v4().simple()
        );
        let sessions_root = crate::coding::runtime_location::build_windows_unc_path(
            &distro,
            "/root/.codex/sessions",
        );

        let scan = rollout_cwds(&sessions_root);
        assert!(!scan.complete, "an unreachable distro must fail closed");
        assert!(scan.cwds.is_empty());
    }

    /// A root that cannot be read at all — a stopped distro, a dropped share —
    /// must not read as "nothing here": the whole cleanup is gated on this set.
    #[test]
    fn an_unreadable_rollout_root_is_incomplete() {
        let dir = tempfile::tempdir().expect("tempdir");
        // A file where `sessions/` should be: it exists, so this is neither an
        // absent directory nor a readable one.
        let sessions_root = dir.path().join("sessions");
        fs::write(&sessions_root, b"not a directory").expect("write should succeed");

        let scan = rollout_cwds(&sessions_root);
        assert!(!scan.complete, "an unreadable root must fail closed");
        assert!(scan.cwds.is_empty());
    }

    /// A rollout whose head cannot be decoded may name a workspace, so it makes
    /// the scan incomplete rather than being skipped.
    #[test]
    fn an_unreadable_rollout_head_is_incomplete() {
        let dir = tempfile::tempdir().expect("tempdir");
        let sessions_root = dir.path().join("sessions");
        let dated = sessions_root.join("2026").join("09").join("17");
        fs::create_dir_all(&dated).expect("rollout directory should be created");
        // Not a zstd frame, but named like a compressed rollout.
        fs::write(
            sessions_root
                .join("2026")
                .join("09")
                .join("17")
                .join(format!("rollout-2026-09-17T09-00-00-{PREFIX_ID}.jsonl.zst")),
            b"not compressed at all",
        )
        .expect("write should succeed");

        let scan = rollout_cwds(&sessions_root);
        assert!(!scan.complete, "an undecodable rollout must fail closed");

        // A readable rollout without a `session_meta` record hides no working
        // directory, so it stays harmless.
        let other = dated.join(format!("rollout-2026-09-17T10-00-00-{OTHER_ID}.jsonl"));
        fs::write(&other, b"{\"type\":\"user_message\"}\n").expect("write should succeed");

        let scan = rollout_cwds(&sessions_root);
        assert!(
            !scan.complete,
            "the undecodable rollout is still there, and completeness is not restored"
        );
        assert!(scan.cwds.is_empty());
    }

    #[test]
    fn rejects_names_that_are_not_rollouts() {
        assert!(RolloutFileName::parse("session_index.jsonl").is_none());
        assert!(RolloutFileName::parse("notes.jsonl").is_none());
        assert!(RolloutFileName::parse("rollout-2026-09-17T09-00-00-notauuid.jsonl").is_none());
        // Wrong timestamp shape.
        assert!(
            RolloutFileName::parse(&format!("rollout-2026-09-17X09-00-00-{PREFIX_ID}.jsonl"))
                .is_none()
        );
    }

    #[test]
    fn reads_history_mode_and_base_from_session_meta() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("rollout-2026-09-17T09-00-00-a.jsonl");
        write_rollout(
            &path,
            PREFIX_ID,
            Some("paginated"),
            Some(json!({
                "thread_id": CHILD_ROLLOUT_ID,
                "end_ordinal_exclusive": 7,
                "end_byte_offset": 4096,
            })),
        );

        let head = read_rollout_head(&path).expect("head should parse");
        assert_eq!(head.thread_id, PREFIX_ID);
        assert!(head.paginated);
        assert_eq!(
            head.history_base,
            Some(HistoryBase {
                rollout_id: CHILD_ROLLOUT_ID.to_string(),
                end_ordinal_exclusive: 7,
                end_byte_offset: 4096,
            })
        );
    }

    #[test]
    fn treats_missing_history_mode_as_legacy() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("rollout-2026-09-17T09-00-00-a.jsonl");
        write_rollout(&path, PREFIX_ID, None, None);

        let head = read_rollout_head(&path).expect("head should parse");
        assert!(!head.paginated, "an absent history_mode is legacy");
        assert_eq!(head.history_base, None);
    }

    #[test]
    fn resolves_the_highest_versioned_state_database() {
        let dir = tempfile::tempdir().expect("tempdir");
        write_state_db(dir.path(), "state_4.sqlite", &[]);
        write_state_db(dir.path(), "state_5.sqlite", &[]);
        write_state_db(dir.path(), "state_10.sqlite", &[]);

        let resolved = resolve_state_db_path(dir.path()).expect("state db should resolve");
        assert_eq!(
            resolved.file_name().and_then(|name| name.to_str()),
            Some("state_10.sqlite"),
            "the schema version carries the generation, so 10 beats 5"
        );
    }

    #[test]
    fn no_state_database_resolves_to_none() {
        let dir = tempfile::tempdir().expect("tempdir");
        assert!(resolve_state_db_path(dir.path()).is_none());
    }

    #[test]
    fn loads_selected_rollout_paths_and_degrades_on_drift() {
        let dir = tempfile::tempdir().expect("tempdir");
        write_state_db(
            dir.path(),
            "state_5.sqlite",
            &[
                (PREFIX_ID, "/codex/sessions/2026/09/17/rollout-a.jsonl"),
                (OTHER_ID, "/codex/sessions/2026/09/17/rollout-b.jsonl"),
            ],
        );

        let index = CodexStateIndex::load(dir.path()).expect("index should load");
        assert_eq!(
            index.selected_path(PREFIX_ID),
            Some("/codex/sessions/2026/09/17/rollout-a.jsonl")
        );
        assert_eq!(index.selected_path("not-a-thread"), None);

        // A database whose thread table lacks the columns we read must not be an
        // error, only a missing hint.
        let drifted = tempfile::tempdir().expect("tempdir");
        let connection = Connection::open(drifted.path().join("state_5.sqlite")).expect("db");
        connection
            .execute_batch("CREATE TABLE threads (unrelated TEXT)")
            .expect("create drifted table");
        drop(connection);
        assert!(CodexStateIndex::load(drifted.path()).is_none());
    }

    #[test]
    fn state_database_selection_beats_the_newest_artifact() {
        let dir = tempfile::tempdir().expect("tempdir");
        let older = "/codex/sessions/2026/09/11/rollout-old.jsonl".to_string();
        let newer = "/codex/sessions/2026/09/17/rollout-new.jsonl".to_string();
        write_state_db(dir.path(), "state_5.sqlite", &[(PREFIX_ID, older.as_str())]);
        let index = CodexStateIndex::load(dir.path()).expect("index should load");

        // A revert leaves several rollouts for one thread; only the database knows
        // which one the thread actually uses.
        let candidates = vec![(newer.clone(), 900), (older.clone(), 100)];
        let selected = select_canonical_index(Some(&index), PREFIX_ID, &candidates)
            .expect("a candidate should be selected");
        assert_eq!(candidates[selected].0, older);
    }

    #[test]
    fn a_stale_database_path_falls_back_to_the_newest_artifact() {
        let dir = tempfile::tempdir().expect("tempdir");
        write_state_db(
            dir.path(),
            "state_5.sqlite",
            &[(PREFIX_ID, "/codex/sessions/gone/rollout-missing.jsonl")],
        );
        let index = CodexStateIndex::load(dir.path()).expect("index should load");

        // Codex repairs a stale row; we must not write to its database, so the row
        // must not hide a session that is plainly on disk.
        let candidates = vec![
            (
                "/codex/sessions/2026/09/11/rollout-old.jsonl".to_string(),
                100,
            ),
            (
                "/codex/sessions/2026/09/17/rollout-new.jsonl".to_string(),
                900,
            ),
        ];
        let selected = select_canonical_index(Some(&index), PREFIX_ID, &candidates)
            .expect("a candidate should be selected");
        assert_eq!(candidates[selected].0, candidates[1].0);
    }

    #[test]
    fn without_an_authority_the_newest_artifact_wins() {
        let candidates = vec![
            (
                "/codex/sessions/2026/09/11/rollout-old.jsonl".to_string(),
                100,
            ),
            (
                "/codex/sessions/2026/09/17/rollout-new.jsonl".to_string(),
                900,
            ),
        ];
        let selected = select_canonical_index(None, PREFIX_ID, &candidates).expect("selected");
        assert_eq!(candidates[selected].0, candidates[1].0);

        assert!(select_canonical_index(None, PREFIX_ID, &[]).is_none());
    }

    #[test]
    fn database_paths_compare_across_separators() {
        let dir = tempfile::tempdir().expect("tempdir");
        write_state_db(
            dir.path(),
            "state_5.sqlite",
            &[(PREFIX_ID, r"C:\codex\sessions\2026\09\17\rollout-a.jsonl")],
        );
        let index = CodexStateIndex::load(dir.path()).expect("index should load");

        // A backslash path written by Codex's database must match the same
        // rollout as scanned with forward slashes, on every platform.
        let candidates = vec![
            (
                "/codex/sessions/2026/09/17/rollout-b.jsonl".to_string(),
                900,
            ),
            (
                "C:/codex/sessions/2026/09/17/rollout-a.jsonl".to_string(),
                100,
            ),
        ];
        let selected = select_canonical_index(Some(&index), PREFIX_ID, &candidates)
            .expect("a candidate should be selected");
        assert_eq!(candidates[selected].0, candidates[1].0);
    }

    /// Case folding is deliberately platform-scoped: Windows paths are
    /// case-insensitive, so a differently-cased database path still names the
    /// same rollout, while on Unix-like systems it names a *different* file and
    /// must not be treated as a match.
    #[test]
    fn database_paths_fold_case_only_where_the_platform_does() {
        let dir = tempfile::tempdir().expect("tempdir");
        write_state_db(
            dir.path(),
            "state_5.sqlite",
            &[(PREFIX_ID, r"C:\codex\sessions\2026\09\17\rollout-a.jsonl")],
        );
        let index = CodexStateIndex::load(dir.path()).expect("index should load");

        let candidates = vec![
            (
                "/codex/sessions/2026/09/17/rollout-b.jsonl".to_string(),
                900,
            ),
            (
                "c:/codex/sessions/2026/09/17/rollout-a.jsonl".to_string(),
                100,
            ),
        ];
        let selected = select_canonical_index(Some(&index), PREFIX_ID, &candidates)
            .expect("a candidate should be selected");

        // Case-insensitive platforms honour the database hint even though the
        // candidate is newer; case-sensitive ones fall back to newest-wins.
        let expected = if cfg!(windows) {
            &candidates[1]
        } else {
            &candidates[0]
        };
        assert_eq!(candidates[selected].0, expected.0);
    }

    #[test]
    fn database_plain_path_matches_compressed_rollout_on_disk() {
        let dir = tempfile::tempdir().expect("tempdir");
        write_state_db(
            dir.path(),
            "state_5.sqlite",
            &[(PREFIX_ID, r"C:\codex\sessions\2026\09\11\rollout-old.jsonl")],
        );
        let index = CodexStateIndex::load(dir.path()).expect("index should load");

        let candidates = vec![
            (
                "C:/codex/sessions/2026/09/17/rollout-new.jsonl.zst".to_string(),
                900,
            ),
            (
                "C:/codex/sessions/2026/09/11/rollout-old.jsonl.zst".to_string(),
                100,
            ),
        ];
        let selected = select_canonical_index(Some(&index), PREFIX_ID, &candidates)
            .expect("a candidate should be selected");
        assert_eq!(candidates[selected].0, candidates[1].0);
    }

    /// Builds a two-rollout paginated thread: a prefix and a child that references
    /// it instead of copying it.
    fn write_two_segment_thread(sessions_root: &Path) -> (PathBuf, PathBuf) {
        let prefix = sessions_root
            .join("2026")
            .join("09")
            .join("11")
            .join(format!("rollout-2026-09-11T10-00-00-{PREFIX_ID}.jsonl"));
        let child = sessions_root
            .join("2026")
            .join("09")
            .join("17")
            .join(format!(
                "rollout-2026-09-17T09-00-00-{PREFIX_ID}_{CHILD_ROLLOUT_ID}.jsonl"
            ));

        let prefix_len = write_rollout(&prefix, PREFIX_ID, Some("paginated"), None);
        write_rollout(
            &child,
            PREFIX_ID,
            Some("paginated"),
            // The prefix is named by rollout id, which for an ordinary rollout file
            // is the thread id.
            Some(json!({
                "thread_id": PREFIX_ID,
                "end_ordinal_exclusive": 4,
                "end_byte_offset": prefix_len,
            })),
        );
        (prefix, child)
    }

    #[test]
    fn resolves_a_two_segment_lineage_oldest_first() {
        let dir = tempfile::tempdir().expect("tempdir");
        let sessions_root = dir.path().join("sessions");
        let (prefix, child) = write_two_segment_thread(&sessions_root);
        let prefix_len = fs::metadata(&prefix).expect("prefix").len();

        let lineage = resolve_lineage(&sessions_root, &child);
        assert_eq!(lineage.segments.len(), 2, "both rollouts contribute");
        assert_eq!(lineage.segments[0].path, prefix);
        assert_eq!(
            lineage.segments[0].end_byte_offset,
            Some(prefix_len),
            "the child's recorded offset bounds the prefix"
        );
        assert_eq!(lineage.segments[1].path, child);
        assert_eq!(
            lineage.segments[1].end_byte_offset, None,
            "the thread's own newest rollout reads to the end"
        );
    }

    #[test]
    fn a_legacy_rollout_resolves_to_one_segment() {
        let dir = tempfile::tempdir().expect("tempdir");
        let sessions_root = dir.path().join("sessions");
        let path = sessions_root
            .join("2026")
            .join("09")
            .join("17")
            .join(format!("rollout-2026-09-17T09-00-00-{PREFIX_ID}.jsonl"));
        write_rollout(&path, PREFIX_ID, Some("legacy"), None);

        let lineage = resolve_lineage(&sessions_root, &path);
        assert_eq!(lineage.segments.len(), 1);
        assert_eq!(lineage.segments[0].end_byte_offset, None);
    }

    /// A chain that breaks midway keeps the segments it did verify. Collapsing to
    /// the leaf alone would hide the middle segments even though their records are
    /// bounded by a cutoff the child recorded and are therefore unambiguous.
    #[test]
    fn a_broken_chain_keeps_the_verified_segments() {
        let dir = tempfile::tempdir().expect("tempdir");
        let sessions_root = dir.path().join("sessions");
        let middle_id = "01a08e9a-2222-7c31-9a20-6d3f11b91882";

        // The thread's original rollout is gone; the middle segment points at it.
        let middle = sessions_root
            .join("2026")
            .join("09")
            .join("14")
            .join(format!(
                "rollout-2026-09-14T09-00-00-{PREFIX_ID}_{middle_id}.jsonl"
            ));
        let leaf = sessions_root
            .join("2026")
            .join("09")
            .join("17")
            .join(format!(
                "rollout-2026-09-17T09-00-00-{PREFIX_ID}_{CHILD_ROLLOUT_ID}.jsonl"
            ));

        let middle_len = write_rollout(
            &middle,
            PREFIX_ID,
            Some("paginated"),
            Some(json!({
                // Names a rollout no file carries.
                "thread_id": ABSENT_ROLLOUT_ID,
                "end_ordinal_exclusive": 4,
                "end_byte_offset": 64,
            })),
        );
        write_rollout(
            &leaf,
            PREFIX_ID,
            Some("paginated"),
            Some(json!({
                "thread_id": middle_id,
                "end_ordinal_exclusive": 8,
                "end_byte_offset": middle_len,
            })),
        );

        let lineage = resolve_lineage(&sessions_root, &leaf);
        assert_eq!(
            lineage.segments.len(),
            2,
            "the middle and leaf segments are both verified"
        );
        assert_eq!(lineage.segments[0].path, middle);
        assert_eq!(lineage.segments[0].end_byte_offset, Some(middle_len));
        assert_eq!(lineage.segments[1].path, leaf);
        assert_eq!(lineage.segments[1].end_byte_offset, None);
    }

    #[test]
    fn a_cutoff_past_the_prefix_keeps_only_the_leaf() {
        let dir = tempfile::tempdir().expect("tempdir");
        let sessions_root = dir.path().join("sessions");
        let prefix = sessions_root
            .join("2026")
            .join("09")
            .join("11")
            .join(format!("rollout-2026-09-11T10-00-00-{PREFIX_ID}.jsonl"));
        let child = sessions_root
            .join("2026")
            .join("09")
            .join("17")
            .join(format!(
                "rollout-2026-09-17T09-00-00-{PREFIX_ID}_{CHILD_ROLLOUT_ID}.jsonl"
            ));
        write_rollout(&prefix, PREFIX_ID, Some("paginated"), None);
        write_rollout(
            &child,
            PREFIX_ID,
            Some("paginated"),
            Some(json!({
                "thread_id": PREFIX_ID,
                // Past the end of the prefix file, so the reference is broken.
                "end_ordinal_exclusive": 4,
                "end_byte_offset": 1_000_000,
            })),
        );

        let lineage = resolve_lineage(&sessions_root, &child);
        assert_eq!(
            lineage.segments.len(),
            1,
            "an untrustworthy chain must not merge unrelated records"
        );
        assert_eq!(lineage.segments[0].path, child);
    }

    #[test]
    fn a_missing_prefix_keeps_only_the_leaf() {
        let dir = tempfile::tempdir().expect("tempdir");
        let sessions_root = dir.path().join("sessions");
        let child = sessions_root
            .join("2026")
            .join("09")
            .join("17")
            .join(format!(
                "rollout-2026-09-17T09-00-00-{PREFIX_ID}_{CHILD_ROLLOUT_ID}.jsonl"
            ));
        write_rollout(
            &child,
            PREFIX_ID,
            Some("paginated"),
            Some(json!({
                "thread_id": ABSENT_ROLLOUT_ID,
                "end_ordinal_exclusive": 4,
                "end_byte_offset": 64,
            })),
        );

        let lineage = resolve_lineage(&sessions_root, &child);
        assert_eq!(lineage.segments.len(), 1);
        assert_eq!(lineage.segments[0].path, child);
    }

    #[test]
    fn finds_prefixes_in_the_archived_sessions_directory() {
        let dir = tempfile::tempdir().expect("tempdir");
        let sessions_root = dir.path().join("sessions");
        let prefix = dir
            .path()
            .join("archived_sessions")
            .join(format!("rollout-2026-09-11T10-00-00-{PREFIX_ID}.jsonl"));
        let child = sessions_root
            .join("2026")
            .join("09")
            .join("17")
            .join(format!(
                "rollout-2026-09-17T09-00-00-{PREFIX_ID}_{CHILD_ROLLOUT_ID}.jsonl"
            ));
        let prefix_len = write_rollout(&prefix, PREFIX_ID, Some("paginated"), None);
        write_rollout(
            &child,
            PREFIX_ID,
            Some("paginated"),
            Some(json!({
                "thread_id": PREFIX_ID,
                "end_ordinal_exclusive": 4,
                "end_byte_offset": prefix_len,
            })),
        );

        // Archiving a prefix must not break the child's replay.
        let lineage = resolve_lineage(&sessions_root, &child);
        assert_eq!(lineage.segments.len(), 2);
        assert_eq!(lineage.segments[0].path, prefix);
    }

    #[test]
    fn collects_references_from_other_threads_only() {
        let dir = tempfile::tempdir().expect("tempdir");
        let sessions_root = dir.path().join("sessions");

        // A child of another thread pointing at this thread's rollout.
        let child = sessions_root
            .join("2026")
            .join("09")
            .join("17")
            .join(format!(
                "rollout-2026-09-17T09-00-00-{OTHER_ID}_{CHILD_ROLLOUT_ID}.jsonl"
            ));
        write_rollout(
            &child,
            OTHER_ID,
            Some("paginated"),
            Some(json!({
                "thread_id": PREFIX_ID,
                "end_ordinal_exclusive": 4,
                "end_byte_offset": 64,
            })),
        );
        // This thread's own continuation references its own prefix; Codex skips it.
        let own = sessions_root
            .join("2026")
            .join("09")
            .join("17")
            .join(format!(
                "rollout-2026-09-17T09-00-00-{PREFIX_ID}_{CHILD_ROLLOUT_ID}.jsonl"
            ));
        write_rollout(
            &own,
            PREFIX_ID,
            Some("paginated"),
            Some(json!({
                "thread_id": PREFIX_ID,
                "end_ordinal_exclusive": 4,
                "end_byte_offset": 64,
            })),
        );

        let referenced = rollout_ids_referenced_by_other_threads(&sessions_root, PREFIX_ID);
        assert!(referenced.contains(PREFIX_ID));
        assert!(
            !referenced.contains(CHILD_ROLLOUT_ID),
            "a thread referencing its own prefix is not an external reference"
        );
    }
}
