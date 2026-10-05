//! Preparation and reliable application of plans (Epic 001, T4).
//!
//! # Stages
//!
//! * [`prepare`] renders a [`Plan`] against the project on disk into a
//!   complete set of proposed [`FileChange`]s plus the resulting model and
//!   session. It only reads; previews therefore leave disk and state untouched.
//! * [`apply_plan`] writes a new plan. Under a per-project lock it recovers any
//!   interrupted application, answers retries from a ledger, re-prepares
//!   against current disk state (revision, preconditions, ownership drift),
//!   then commits through a journal. [`recover`] and [`abort_interrupted_apply`]
//!   write only to resolve an interrupted transaction.
//!
//! # Commit protocol and recovery
//!
//! Per-file atomic replacement is not a multi-file transaction, so the commit is
//! write-ahead:
//!
//! 1. Under `.protopie/lock` (an OS advisory lock, released if the process dies)
//!    the complete set of staged files (code, `model.json`, the session and the
//!    ledger `applied.json`), each with its `before` and `after` contents, is
//!    written atomically to `.protopie/journal.json`. On Unix, the journal's
//!    durable existence is the commit point.
//! 2. Each staged file is replaced atomically (temp file + rename), followed
//!    by a parent-directory sync on Unix.
//! 3. The journal is removed and its parent directory synced on Unix.
//! On other platforms this protocol guarantees recovery after process crashes;
//! power-loss durability depends on directory-sync support.
//!
//! [`recover`] (run before every application) rolls a leftover
//! journal *forward*: every file whose content is still `before` is written
//! with `after`; one already at `after` is skipped (idempotent). A file that
//! matches neither was edited externally since the failure; recovery then
//! reports a conflict and writes nothing further. Call
//! [`abort_interrupted_apply`] to preserve external source edits, roll back
//! exact staged writes, and clear the journal. The abort is itself journaled
//! so an interrupted rollback can finish on the next recovery.
//! Every path touched by application, metadata and recovery goes through
//! [`crate::resolve_in_project`]; journal paths are all validated before the
//! first write.
//!
//! # Retry identity
//!
//! The ledger records each applied plan, its conversation, and an optional API
//! request ID. Applying an exact plan in the same conversation returns its result
//! ([`ApplyOutcome::Replayed`]) without touching the project; an unapplied plan
//! whose base revision no longer matches is a conflict that requires replanning.
//! A replay with a new request ID conflicts because that ID cannot be recorded
//! without changing the already committed transaction.

use std::fs::{self, File};
use std::io::{self, Write};
#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Component, Path};

use serde::{Deserialize, Serialize};

use crate::contracts::*;
use crate::emit;
use crate::project::{self, extract_region, fingerprint, replace_region, verify_owned};
use crate::style;
use crate::{io_err, resolve_in_project, Error, Result};

const LOCK_FILE: &str = ".protopie/lock";
const JOURNAL_FILE: &str = ".protopie/journal.json";
const LEDGER_FILE: &str = ".protopie/applied.json";

const LAYOUT_FILE: &str = "src/App.tsx";
const LAYOUT_REGION: &str = "layout-top";
const PROVIDERS_REGION: &str = "providers";
const HOME_FILE: &str = "src/pages/Home.tsx";
const HOME_FLOW_REGION: &str = "home-flow";
const ROUTER_FILE: &str = "src/router.ts";
const ROUTES_REGION: &str = "routes";

#[derive(Debug, Default, Clone)]
pub struct ApplyOptions<'a> {
    /// API-boundary request ID distinguishing transport retries from
    /// deliberate repeated commands.
    pub request_id: Option<&'a str>,
    /// Exact natural-language command for request-ID replay validation.
    /// Direct plan callers may leave this unset and retain plan-based replay.
    pub command: Option<&'a str>,
    /// Test hook: fail with an I/O error after this many staged files were written.
    pub fail_after_writes: Option<usize>,
    /// Test hook: fail after all staged files are written, before journal removal.
    pub fail_before_journal_removal: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppliedRecord {
    pub plan_id: String,
    #[serde(default)]
    pub request_id: Option<String>,
    /// Missing on older records; these cannot authorize command retries.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    /// Missing on older ledger records. Such records cannot prove retry identity.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversation_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan: Option<Plan>,
    pub revision: u64,
    pub changed_files: Vec<String>,
}

/// One proposed change to a code file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileChange {
    pub path: String,
    /// `None` when the file does not exist yet.
    pub before: Option<String>,
    pub after: String,
    /// Set for a change confined to a marker region of a shared file.
    pub region: Option<String>,
    pub diff: String,
}

/// A fully rendered plan: proposed code changes and the resulting state.
#[derive(Debug, Clone)]
pub struct Prepared {
    pub plan_id: String,
    pub base_revision: u64,
    pub files: Vec<FileChange>,
    snapshot: ProjectSnapshot,
    session: Session,
}

#[derive(Debug)]
pub enum PrepareOutcome {
    Ready(Prepared),
    /// Disk or model no longer matches the plan; replan.
    Conflict {
        reason: String,
    },
    Unsupported {
        reason: String,
    },
}

#[derive(Debug)]
pub enum ApplyOutcome {
    Applied(AppliedRecord),
    /// The plan (or request) was already applied; nothing was written.
    Replayed(AppliedRecord),
    Conflict {
        reason: String,
    },
    Unsupported {
        reason: String,
    },
}

#[derive(Debug, PartialEq, Eq)]
pub enum RecoverOutcome {
    NothingToDo,
    RolledForward {
        plan_id: String,
    },
    Aborted {
        plan_id: String,
        preserved_paths: Vec<String>,
    },
    Conflict {
        reason: String,
    },
}

// ------------------------------------------------------------------ lock

/// Exclusive per-project lock (advisory, OS-released on process exit).
pub struct ProjectLock {
    _file: File,
}

fn open_lock(dir: &Path) -> Result<File> {
    let path = resolve_in_project(dir, LOCK_FILE)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(io_err(parent))?;
    }
    resolve_in_project(dir, LOCK_FILE)?;
    let mut options = fs::OpenOptions::new();
    options.create(true).truncate(false).write(true);
    #[cfg(unix)]
    options.custom_flags(libc::O_NOFOLLOW);
    options.open(&path).map_err(io_err(&path))
}

impl ProjectLock {
    /// Blocks until the lock is held.
    pub fn acquire(dir: &Path) -> Result<Self> {
        let file = open_lock(dir)?;
        file.lock().map_err(io_err(dir))?;
        Ok(Self { _file: file })
    }

    /// `None` if another holder has it.
    pub fn try_acquire(dir: &Path) -> Result<Option<Self>> {
        let file = open_lock(dir)?;
        match file.try_lock() {
            Ok(()) => Ok(Some(Self { _file: file })),
            Err(fs::TryLockError::WouldBlock) => Ok(None),
            Err(fs::TryLockError::Error(e)) => Err(Error::Io {
                path: dir.to_path_buf(),
                source: e,
            }),
        }
    }
}

// ------------------------------------------------------------------ files

fn read_text(dir: &Path, relative: &str) -> Result<Option<String>> {
    let path = resolve_in_project(dir, relative)?;
    match fs::read_to_string(&path) {
        Ok(t) => Ok(Some(t)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(Error::Io { path, source: e }),
    }
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> Result<()> {
    File::open(path)
        .map_err(io_err(path))?
        .sync_all()
        .map_err(io_err(path))
}

#[cfg(not(unix))]
fn sync_directory(_path: &Path) -> Result<()> {
    Ok(())
}

fn create_parent_directories(parent: &Path) -> Result<()> {
    let mut missing = Vec::new();
    let mut cursor = parent;
    while !cursor.exists() {
        missing.push(cursor.to_path_buf());
        cursor = cursor.parent().expect("project parent exists");
    }
    fs::create_dir_all(parent).map_err(io_err(parent))?;
    for created in missing.into_iter().rev() {
        sync_directory(created.parent().expect("directory has a parent"))?;
    }
    Ok(())
}

/// Atomic replace of one file: exclusive random temp file in the same
/// directory, fsync, rename, then directory sync on Unix. A pre-existing
/// temp symlink is never followed.
fn write_file(dir: &Path, relative: &str, content: &str) -> Result<()> {
    let path = resolve_in_project(dir, relative)?;
    let parent = path.parent().expect("project file has a parent");
    create_parent_directories(parent)?;
    resolve_in_project(dir, relative)?;
    let mut tmp = tempfile::Builder::new()
        .prefix(".protopie-tmp-")
        .tempfile_in(parent)
        .map_err(io_err(parent))?;
    tmp.write_all(content.as_bytes())
        .map_err(io_err(tmp.path()))?;
    tmp.as_file().sync_all().map_err(io_err(tmp.path()))?;
    resolve_in_project(dir, relative)?;
    tmp.persist(&path).map_err(|e| Error::Io {
        path: path.clone(),
        source: e.error,
    })?;
    sync_directory(parent)?;
    Ok(())
}

fn remove_file(dir: &Path, relative: &str) -> Result<()> {
    let path = resolve_in_project(dir, relative)?;
    match fs::remove_file(&path) {
        Ok(()) => {
            sync_directory(path.parent().expect("project file has a parent"))?;
            Ok(())
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(Error::Io { path, source: e }),
    }
}

/// Minimal line diff: common prefix and suffix kept, the middle shown as -/+.
fn render_diff(path: &str, before: Option<&str>, after: &str) -> String {
    let old: Vec<&str> = before.unwrap_or("").lines().collect();
    let new: Vec<&str> = after.lines().collect();
    let prefix = old.iter().zip(&new).take_while(|(a, b)| a == b).count();
    let suffix = old[prefix..]
        .iter()
        .rev()
        .zip(new[prefix..].iter().rev())
        .take_while(|(a, b)| a == b)
        .count();
    let mut out = format!(
        "--- {}\n+++ {path}\n",
        if before.is_some() { path } else { "/dev/null" }
    );
    for l in &old[prefix..old.len() - suffix] {
        out.push_str(&format!("-{l}\n"));
    }
    for l in &new[prefix..new.len() - suffix] {
        out.push_str(&format!("+{l}\n"));
    }
    out
}

// ------------------------------------------------------------------ journal

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Entry {
    path: String,
    before: Option<String>,
    after: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    region: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    abort_source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    abort_target: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Journal {
    record: AppliedRecord,
    entries: Vec<Entry>,
    #[serde(default)]
    mode: JournalMode,
    #[serde(default)]
    preserved_paths: Vec<String>,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum JournalMode {
    #[default]
    RollForward,
    Abort,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Ledger {
    #[serde(default)]
    entries: Vec<AppliedRecord>,
}

fn metadata_error(dir: &Path, relative: &str, message: impl ToString) -> Error {
    Error::Metadata {
        path: dir.join(relative),
        message: message.to_string(),
    }
}

fn is_metadata_path(relative: &str) -> bool {
    matches!(Path::new(relative).components().next(), Some(Component::Normal(name)) if name == ".protopie")
}

fn owned_region(entry: &Entry) -> Option<&str> {
    entry
        .region
        .as_deref()
        .or_else(|| (entry.path == LAYOUT_FILE).then_some(LAYOUT_REGION))
}

fn load_ledger(dir: &Path) -> Result<Ledger> {
    match read_text(dir, LEDGER_FILE)? {
        None => Ok(Ledger::default()),
        Some(t) => serde_json::from_str(&t).map_err(|e| metadata_error(dir, LEDGER_FILE, e)),
    }
}

fn json<T: Serialize>(dir: &Path, relative: &str, value: &T) -> Result<String> {
    let mut s =
        serde_json::to_string_pretty(value).map_err(|e| metadata_error(dir, relative, e))?;
    s.push('\n');
    Ok(s)
}

fn recover_locked(dir: &Path) -> Result<RecoverOutcome> {
    let Some(text) = read_text(dir, JOURNAL_FILE)? else {
        return Ok(RecoverOutcome::NothingToDo);
    };
    let journal: Journal =
        serde_json::from_str(&text).map_err(|e| metadata_error(dir, JOURNAL_FILE, e))?;
    match journal.mode {
        JournalMode::RollForward => roll_forward_locked(dir, &journal),
        JournalMode::Abort => finish_abort_locked(dir, &journal),
    }
}

fn roll_forward_locked(dir: &Path, journal: &Journal) -> Result<RecoverOutcome> {
    // Validate every path and state before the first write.
    let mut pending = Vec::new();
    for entry in &journal.entries {
        let current = read_text(dir, &entry.path)?;
        if current.as_deref() == Some(entry.after.as_str()) {
            continue;
        }
        if current != entry.before {
            return Ok(RecoverOutcome::Conflict {
                reason: format!(
                    "{} changed after an interrupted application of {}; resolve it manually",
                    entry.path, journal.record.plan_id
                ),
            });
        }
        pending.push(entry);
    }
    for entry in pending {
        write_file(dir, &entry.path, &entry.after)?;
    }
    remove_file(dir, JOURNAL_FILE)?;
    Ok(RecoverOutcome::RolledForward {
        plan_id: journal.record.plan_id.clone(),
    })
}

fn finish_abort_locked(dir: &Path, journal: &Journal) -> Result<RecoverOutcome> {
    // Check all paths before writing. A preserved file with an abort target
    // may need its owned region restored; edits outside that region survive.
    for path in &journal.preserved_paths {
        if is_metadata_path(path) || !journal.entries.iter().any(|entry| &entry.path == path) {
            return Err(metadata_error(
                dir,
                JOURNAL_FILE,
                "invalid preserved path in abort journal",
            ));
        }
    }
    let mut rollback = Vec::new();
    for entry in &journal.entries {
        let current = read_text(dir, &entry.path)?;
        if let Some(target) = &entry.abort_target {
            if current.as_deref() == Some(target) {
                continue;
            }
            let Some(region) = owned_region(entry) else {
                return Err(metadata_error(
                    dir,
                    JOURNAL_FILE,
                    "abort target has no owned region",
                ));
            };
            let Some(before_body) = entry
                .before
                .as_deref()
                .and_then(|s| extract_region(s, region))
            else {
                return Err(metadata_error(
                    dir,
                    JOURNAL_FILE,
                    "abort source is missing its owned region",
                ));
            };
            let Some(after_body) = extract_region(&entry.after, region) else {
                return Err(metadata_error(
                    dir,
                    JOURNAL_FILE,
                    "abort destination is missing its owned region",
                ));
            };
            let Some(current_text) = current.as_deref() else {
                return Ok(RecoverOutcome::Conflict {
                    reason: format!("{} was removed during abort", entry.path),
                });
            };
            let Some(current_body) = extract_region(current_text, region) else {
                return Ok(RecoverOutcome::Conflict {
                    reason: format!("{} lost its {region} region during abort", entry.path),
                });
            };
            if current_body == before_body {
                // A prior abort attempt already restored the owned region.
                continue;
            }
            if current_body != after_body {
                return Ok(RecoverOutcome::Conflict {
                    reason: format!("{} changed during abort; preserve the edit and resolve the journal manually", entry.path),
                });
            }
            let restored = replace_region(current_text, region, &before_body)
                .expect("region was just extracted");
            rollback.push((entry, Some(restored)));
            continue;
        }
        if journal.preserved_paths.contains(&entry.path) || current == entry.before {
            continue;
        }
        if current.as_deref() != Some(entry.after.as_str()) {
            return Ok(RecoverOutcome::Conflict {
                reason: format!(
                    "{} changed during abort; resolve the interrupted application manually",
                    entry.path
                ),
            });
        }
        rollback.push((entry, entry.before.clone()));
    }
    for (entry, target) in rollback.into_iter().rev() {
        match target {
            Some(before) => write_file(dir, &entry.path, &before)?,
            None => remove_file(dir, &entry.path)?,
        }
    }
    remove_file(dir, JOURNAL_FILE)?;
    Ok(RecoverOutcome::Aborted {
        plan_id: journal.record.plan_id.clone(),
        preserved_paths: journal.preserved_paths.clone(),
    })
}

/// Completes an interrupted application, if any. Cheap when there is none.
pub fn recover(dir: &Path) -> Result<RecoverOutcome> {
    if read_text(dir, JOURNAL_FILE)?.is_none() {
        return Ok(RecoverOutcome::NothingToDo);
    }
    let _lock = ProjectLock::acquire(dir)?;
    recover_locked(dir)
}

/// The recorded result of an earlier application with `request_id`, if any.
/// Read-only; lets transports answer retries without replanning.
pub fn lookup_request(dir: &Path, request_id: &str) -> Result<Option<AppliedRecord>> {
    Ok(load_ledger(dir)?
        .entries
        .into_iter()
        .find(|e| e.request_id.as_deref() == Some(request_id)))
}

/// Read-only check for a pending transaction. Planning callers can report the
/// need for recovery without changing files or creating a lock file.
pub fn recovery_pending(dir: &Path) -> Result<bool> {
    Ok(read_text(dir, JOURNAL_FILE)?.is_some())
}

/// Abandons an interrupted application after an external source edit. Exact
/// staged writes are rolled back; externally edited source files are preserved.
/// The journal records the abort first, so another interruption can be resumed
/// by `recover`. Edited metadata is rejected because preserving it could leave
/// an untrustworthy model or retry ledger.
pub fn abort_interrupted_apply(dir: &Path) -> Result<RecoverOutcome> {
    if read_text(dir, JOURNAL_FILE)?.is_none() {
        return Ok(RecoverOutcome::NothingToDo);
    }
    let _lock = ProjectLock::acquire(dir)?;
    let Some(text) = read_text(dir, JOURNAL_FILE)? else {
        return Ok(RecoverOutcome::NothingToDo);
    };
    let mut journal: Journal =
        serde_json::from_str(&text).map_err(|e| metadata_error(dir, JOURNAL_FILE, e))?;
    if journal.mode == JournalMode::Abort {
        return finish_abort_locked(dir, &journal);
    }
    let mut preserved = Vec::new();
    for entry in &mut journal.entries {
        let current = read_text(dir, &entry.path)?;
        if current != entry.before && current.as_deref() != Some(entry.after.as_str()) {
            if is_metadata_path(&entry.path) {
                return Ok(RecoverOutcome::Conflict {
                    reason: format!(
                        "{} changed externally; cannot safely abort metadata",
                        entry.path
                    ),
                });
            }
            if let Some(region) = owned_region(entry) {
                let before_body = entry
                    .before
                    .as_deref()
                    .and_then(|s| extract_region(s, region));
                let after_body = extract_region(&entry.after, region);
                let current_body = current.as_deref().and_then(|s| extract_region(s, region));
                match (before_body, after_body, current_body) {
                    (Some(before), Some(_), Some(ref now)) if *now == before => {
                        preserved.push(entry.path.clone());
                    }
                    (Some(before), Some(after), Some(ref now)) if *now == after => {
                        let target = replace_region(current.as_deref().unwrap(), region, &before)
                            .expect("region was just extracted");
                        entry.abort_source = current;
                        entry.abort_target = Some(target);
                        preserved.push(entry.path.clone());
                    }
                    _ => return Ok(RecoverOutcome::Conflict {
                        reason: format!("{} has an external edit inside the owned {region} region; restore or reconcile that region before aborting", entry.path),
                    }),
                }
            } else {
                preserved.push(entry.path.clone());
            }
        }
    }
    journal.mode = JournalMode::Abort;
    journal.preserved_paths = preserved;
    write_file(dir, JOURNAL_FILE, &json(dir, JOURNAL_FILE, &journal)?)?;
    finish_abort_locked(dir, &journal)
}

// ------------------------------------------------------------------ prepare

fn is_ident(s: &str, upper_first: bool) -> bool {
    let mut chars = s.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() && (!upper_first || c.is_ascii_uppercase()))
        && chars.all(|c| c.is_ascii_alphanumeric())
}

fn is_element_id(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

fn conflict(reason: impl Into<String>) -> Result<PrepareOutcome> {
    Ok(PrepareOutcome::Conflict {
        reason: reason.into(),
    })
}

fn unsupported(reason: impl Into<String>) -> Result<PrepareOutcome> {
    Ok(PrepareOutcome::Unsupported {
        reason: reason.into(),
    })
}

/// Working copy of the files an operation sequence touches.
struct Workspace<'a> {
    dir: &'a Path,
    files: Vec<FileChange>,
}

impl Workspace<'_> {
    fn current(&mut self, path: &str) -> Result<Option<String>> {
        match self.files.iter().find(|f| f.path == path) {
            Some(f) => Ok(Some(f.after.clone())),
            None => read_text(self.dir, path),
        }
    }

    fn set(&mut self, path: &str, region: Option<&str>, after: String) -> Result<()> {
        if let Some(f) = self.files.iter_mut().find(|f| f.path == path) {
            f.after = after;
            return Ok(());
        }
        let before = read_text(self.dir, path)?;
        self.files.push(FileChange {
            path: path.into(),
            before,
            after,
            region: region.map(Into::into),
            diff: String::new(),
        });
        Ok(())
    }
}

/// Component name of a generated element: the stem of its source file.
fn component_name(rec: &ElementRecord) -> Option<String> {
    let file = &rec.source.as_ref()?.file;
    let name = file.strip_prefix("src/components/")?.strip_suffix(".tsx")?;
    is_ident(name, true).then(|| name.to_string())
}

/// A navigation link target checked against the model.
enum ResolvedLink {
    Internal {
        path: String,
        anchor: Option<String>,
    },
    External(String),
}

enum LinkError {
    /// The destination refers to something that does not exist (any more).
    Missing(String),
    /// The destination itself is malformed.
    Invalid(String),
}

/// Checks `destination` against the model and resolves it to a link.
/// `Unresolved` is no link at all.
fn link_for(
    snapshot: &ProjectSnapshot,
    destination: &Destination,
) -> std::result::Result<Option<ResolvedLink>, LinkError> {
    let page_path = |page: &str| {
        snapshot.page_path(page).map(str::to_string).ok_or_else(|| {
            LinkError::Missing(format!("page {page} does not exist or has no route"))
        })
    };
    match destination {
        Destination::Unresolved => Ok(None),
        Destination::Page { page } => Ok(Some(ResolvedLink::Internal {
            path: page_path(page)?,
            anchor: None,
        })),
        Destination::Section { page, section } => {
            let path = page_path(page)?;
            let anchorable = is_element_id(section)
                && snapshot
                    .element(&ElementId::new(section.as_str()))
                    .is_some_and(|e| e.kind == ElementKind::Hero && &e.page == page);
            if !anchorable {
                return Err(LinkError::Missing(format!(
                    "section {section} does not exist on page {page}"
                )));
            }
            Ok(Some(ResolvedLink::Internal {
                path,
                anchor: Some(section.clone()),
            }))
        }
        Destination::External { url } => match emit::validate_external_url(url) {
            Some(valid) if &valid == url => Ok(Some(ResolvedLink::External(valid))),
            _ => Err(LinkError::Invalid(format!(
                "{url:?} is not a valid http(s) URL"
            ))),
        },
    }
}

/// Renders a navigation component from the model: its items in order.
fn render_navigation(dir: &Path, snapshot: &ProjectSnapshot, nav: &ElementId) -> Result<String> {
    let rec = snapshot.element(nav).expect("navigation is in the model");
    let name = component_name(rec).expect("navigation has a component source");
    let mut links = Vec::new();
    for e in snapshot
        .elements
        .iter()
        .filter(|e| e.kind == ElementKind::NavigationItem && e.parent.as_ref() == Some(nav))
    {
        let link = match e.destination.as_ref().map(|d| link_for(snapshot, d)) {
            None | Some(Ok(None)) => None,
            Some(Ok(Some(link))) => Some(link),
            Some(Err(LinkError::Missing(m) | LinkError::Invalid(m))) => {
                return Err(metadata_error(
                    dir,
                    &project::model_rel(),
                    format!("item {} has an invalid destination: {m}", e.id.0),
                ))
            }
        };
        links.push((e, link));
    }
    let items: Vec<_> = links
        .iter()
        .filter_map(|(e, link)| {
            Some(emit::NavItemView {
                id: &e.id,
                label: e.label.as_deref()?,
                link: link.as_ref().map(|l| match l {
                    ResolvedLink::Internal { path, anchor } => emit::LinkView::Internal {
                        path,
                        anchor: anchor.as_deref(),
                    },
                    ResolvedLink::External(url) => emit::LinkView::External { url },
                }),
            })
        })
        .collect();
    Ok(emit::navigation_tsx(
        &name,
        nav,
        rec.placement.unwrap_or(Placement::Top),
        &items,
    ))
}

/// Re-renders a navigation's component, and its stylesheet when it predates
/// the link styles (so linked items are styled in older projects too).
fn refresh_navigation(
    ws: &mut Workspace,
    snapshot: &ProjectSnapshot,
    nav: &ElementId,
    file: &str,
) -> Result<()> {
    ws.set(file, None, render_navigation(ws.dir, snapshot, nav)?)?;
    let css = file.replace(".tsx", ".module.css");
    let owned = snapshot
        .owned
        .iter()
        .any(|o| o.path == css && o.region.is_none());
    if owned && ws.current(&css)?.is_some_and(|c| !c.contains(".link {")) {
        ws.set(&css, None, emit::navigation_css())?;
    }
    Ok(())
}

/// Checks a placement against the model and the emitter's capability: the
/// anchor must exist, the layout must be known, and only a vertical flow can
/// be rendered (the home page's `HomeAbove` / `HomeBelow` stack).
fn check_placement(
    snapshot: &ProjectSnapshot,
    element: &ElementId,
    position: &Position,
) -> std::result::Result<(), PrepareOutcome> {
    let Some(anchor) = snapshot.element(&position.anchor) else {
        return Err(PrepareOutcome::Conflict {
            reason: format!("anchor {} no longer exists", position.anchor.0),
        });
    };
    if &anchor.id == element {
        return Err(PrepareOutcome::Unsupported {
            reason: format!("{} cannot be placed relative to itself", element.0),
        });
    }
    if let Err(r) = crate::place::check_position(snapshot, position.relation, anchor) {
        return Err(PrepareOutcome::Unsupported {
            reason: r.explanation,
        });
    }
    if matches!(
        position.relation,
        PositionRelation::LeftOf | PositionRelation::RightOf
    ) {
        return Err(PrepareOutcome::Unsupported {
            reason: "no generated container can lay out elements side by side yet".into(),
        });
    }
    Ok(())
}

/// Moves `element` within `snapshot.elements` so its sibling order matches
/// `at` (`None`: the end).
fn reorder(snapshot: &mut ProjectSnapshot, element: &ElementId, at: Option<&Position>) {
    let Some(from) = snapshot.elements.iter().position(|e| &e.id == element) else {
        return;
    };
    let record = snapshot.elements.remove(from);
    let index = match at {
        None => snapshot.elements.len(),
        Some(position) => {
            let anchor = snapshot
                .elements
                .iter()
                .position(|e| e.id == position.anchor)
                .unwrap_or(snapshot.elements.len().saturating_sub(1));
            match position.relation {
                PositionRelation::Below | PositionRelation::RightOf => anchor + 1,
                PositionRelation::Above | PositionRelation::LeftOf => anchor,
            }
        }
    };
    snapshot.elements.insert(index, record);
}

/// Rewrites the `home-flow` region of `Home.tsx` from the model: generated
/// elements before the seeded hero render in `HomeAbove`, the rest in
/// `HomeBelow`. Returns the reason when the region cannot be written.
fn refresh_home_flow(ws: &mut Workspace, snapshot: &ProjectSnapshot) -> Result<Option<String>> {
    let Some(text) = ws.current(HOME_FILE)? else {
        return Ok(Some(format!("{HOME_FILE} is missing")));
    };
    if extract_region(&text, HOME_FLOW_REGION).is_none() {
        return Ok(Some(format!(
            "{HOME_FILE} has no protopie:begin/end {HOME_FLOW_REGION} region (the project \
             predates page elements); add the region and the HomeAbove / HomeBelow \
             components as in the reference template"
        )));
    }
    let siblings = crate::place::flow_siblings(snapshot, DEFAULT_PAGE);
    let hero_at = siblings
        .iter()
        .position(|e| e.source.as_ref().is_some_and(|s| s.file == HOME_FILE));
    let (mut above, mut below) = (Vec::new(), Vec::new());
    for (index, rec) in siblings.iter().enumerate() {
        let Some(name) = component_name(rec) else {
            continue;
        };
        if hero_at.is_some_and(|hero| index < hero) {
            above.push(name);
        } else {
            below.push(name);
        }
    }
    let body = emit::home_flow_region(&above, &below);
    let new_text = replace_region(&text, HOME_FLOW_REGION, &body).expect("region was just found");
    ws.set(HOME_FILE, Some(HOME_FLOW_REGION), new_text)?;
    Ok(None)
}

/// Rewrites the `providers` region of `App.tsx` from the model: one import
/// and one nested provider per context, in creation order. Returns the reason
/// when the region cannot be written.
fn refresh_providers(ws: &mut Workspace, snapshot: &ProjectSnapshot) -> Result<Option<String>> {
    let Some(text) = ws.current(LAYOUT_FILE)? else {
        return Ok(Some(format!("{LAYOUT_FILE} is missing")));
    };
    if extract_region(&text, PROVIDERS_REGION).is_none() {
        return Ok(Some(format!(
            "{LAYOUT_FILE} has no protopie:begin/end {PROVIDERS_REGION} region (the project \
             predates context support); add the region, the `Providers` component and its \
             use around the layout as in the reference template"
        )));
    }
    let contexts: Vec<&ContextRecord> = snapshot.contexts.iter().collect();
    let body = emit::providers_region(&contexts);
    let new_text = replace_region(&text, PROVIDERS_REGION, &body).expect("region was just found");
    ws.set(LAYOUT_FILE, Some(PROVIDERS_REGION), new_text)?;
    Ok(None)
}

/// Re-renders a created page from the model, including the contexts it reads.
/// Returns the reason when the page cannot be written.
fn refresh_page(
    ws: &mut Workspace,
    snapshot: &ProjectSnapshot,
    page: &str,
) -> Result<Option<String>> {
    let Some(rec) = snapshot.page(page) else {
        return Ok(Some(format!("page {page} does not exist")));
    };
    let path = format!("src/pages/{}.tsx", rec.component);
    if !snapshot
        .owned
        .iter()
        .any(|o| o.path == path && o.region.is_none())
        || ws.current(&path)?.is_none()
    {
        return Ok(Some(format!("{path} is missing or not owned")));
    }
    let contexts: Vec<&ContextRecord> = snapshot
        .contexts
        .iter()
        .filter(|c| c.consumers.iter().any(|p| p == page))
        .collect();
    ws.set(
        &path,
        None,
        emit::page_tsx_with_contexts(&rec.component, &rec.id, &rec.label, &contexts),
    )?;
    Ok(None)
}

/// Renders the component of a generated page element from the model.
fn render_element(dir: &Path, snapshot: &ProjectSnapshot, id: &ElementId) -> Result<String> {
    let rec = snapshot.element(id).expect("element is in the model");
    let name = component_name(rec).expect("element has a component source");
    let content = rec.content.clone().unwrap_or_default();
    Ok(match rec.kind {
        ElementKind::Hero => emit::hero_tsx(&name, id, &content),
        ElementKind::Image => emit::image_tsx(&name, id, &content),
        ElementKind::Form => emit::form_tsx(&name, id, &content),
        ElementKind::Footer => emit::footer_tsx(&name, id, &content),
        ElementKind::Button => {
            let link = match rec.destination.as_ref().map(|d| link_for(snapshot, d)) {
                None | Some(Ok(None)) => None,
                Some(Ok(Some(link))) => Some(link),
                Some(Err(LinkError::Missing(m) | LinkError::Invalid(m))) => {
                    return Err(metadata_error(
                        dir,
                        &project::model_rel(),
                        format!("button {} has an invalid destination: {m}", id.0),
                    ))
                }
            };
            let view = link.as_ref().map(|l| match l {
                ResolvedLink::Internal { path, anchor } => emit::LinkView::Internal {
                    path,
                    anchor: anchor.as_deref(),
                },
                ResolvedLink::External(url) => emit::LinkView::External { url },
            });
            emit::button_tsx(&name, id, &content, view)
        }
        ElementKind::Navigation | ElementKind::NavigationItem => {
            unreachable!("navigation is rendered by render_navigation")
        }
    })
}

fn note_issued(snapshot: &mut ProjectSnapshot, id: &ElementId) {
    if let Some((prefix, n)) = id.0.rsplit_once('_') {
        if let Ok(n) = n.parse::<u64>() {
            let e = snapshot.id_counters.entry(prefix.into()).or_insert(0);
            *e = (*e).max(n);
        }
    }
}

/// Renders `plan` against the project at `dir` without writing anything.
pub fn prepare(dir: &Path, plan: &Plan, conversation_id: &str) -> Result<PrepareOutcome> {
    let mut snapshot = match project::load_project(dir)? {
        project::ProjectState::Loaded(s) => s,
        project::ProjectState::NoMetadata => {
            return unsupported("the project has no .protopie model; initialize it first")
        }
    };
    let mut session = project::load_session(dir, conversation_id)?;
    if snapshot.revision != plan.base_revision {
        return conflict(format!(
            "the plan was made at revision {} but the project is at {}; replan",
            plan.base_revision, snapshot.revision
        ));
    }
    for pre in &plan.preconditions {
        let ok = match pre {
            Precondition::RevisionIs { revision } => snapshot.revision == *revision,
            Precondition::ElementExists { id } => snapshot.element(id).is_some(),
            Precondition::ElementAbsent { id } => snapshot.element(id).is_none(),
            Precondition::ContextAbsent { id } => snapshot.context(id).is_none(),
            Precondition::PageExists { page } => snapshot.page_exists(page),
            Precondition::PageAbsent { page } => !snapshot.page_exists(page),
            Precondition::StyleValueIs {
                id,
                property,
                value,
            } => {
                snapshot
                    .element(id)
                    .and_then(|e| e.style.as_ref())
                    .and_then(|b| b.values.get(property.css_name()))
                    == value.as_ref()
            }
        };
        if !ok {
            return conflict(format!("precondition no longer holds: {pre:?}"));
        }
    }
    let drift = verify_owned(dir, &snapshot)?;
    if !drift.is_empty() {
        let list: Vec<_> = drift
            .iter()
            .map(|d| match &d.region {
                Some(r) => format!("{} ({r})", d.path),
                None => d.path.clone(),
            })
            .collect();
        return conflict(format!(
            "owned source was edited externally: {}",
            list.join(", ")
        ));
    }

    for id in &plan.resolves {
        if !session.pending_questions.iter().any(|q| &q.id == id) {
            return conflict(format!("question {id} is no longer pending"));
        }
    }

    let mut ws = Workspace {
        dir,
        files: Vec::new(),
    };
    let mut focus: Option<(ElementId, ElementKind)> = None;
    for op in &plan.operations {
        match op {
            Operation::CreateComponent {
                id,
                kind: ElementKind::Navigation,
                name,
                ..
            } => {
                if !is_element_id(&id.0) || !is_ident(name, true) {
                    return unsupported(format!("invalid component id or name: {} / {name}", id.0));
                }
                if snapshot.element(id).is_some() {
                    return conflict(format!("element id {} already exists", id.0));
                }
                let path = format!("src/components/{name}.tsx");
                let css_path = format!("src/components/{name}.module.css");
                for p in [&path, &css_path] {
                    if let Some(owner) = snapshot
                        .elements
                        .iter()
                        .find(|e| e.source.as_ref().is_some_and(|s| &s.file == p))
                    {
                        return conflict(format!("{p} is already owned by element {}", owner.id.0));
                    }
                    if snapshot.owned.iter().any(|o| &o.path == p) || ws.current(p)?.is_some() {
                        return conflict(format!("{p} already exists or is owned"));
                    }
                }
                let mut e = ElementRecord::new(id.clone(), ElementKind::Navigation, DEFAULT_PAGE);
                e.placement = Some(Placement::Top);
                e.source = Some(SourceBinding {
                    file: path.clone(),
                    region: None,
                });
                snapshot.elements.push(e);
                ws.set(&path, None, render_navigation(dir, &snapshot, id)?)?;
                ws.set(&css_path, None, emit::navigation_css())?;
                note_issued(&mut snapshot, id);
                focus = Some((id.clone(), ElementKind::Navigation));
            }
            Operation::CreateComponent {
                id,
                kind,
                name,
                content,
            } if kind.is_flow() => {
                let Some(content) = content else {
                    return unsupported(format!("the {} has no content", kind.word()));
                };
                if !is_element_id(&id.0)
                    || emit::element_component_name(*kind, id).as_deref() != Some(name.as_str())
                {
                    return unsupported(format!("invalid component id or name: {} / {name}", id.0));
                }
                if let Err(message) = emit::validate_content(*kind, content) {
                    return unsupported(message);
                }
                if snapshot.element(id).is_some() {
                    return conflict(format!("element id {} already exists", id.0));
                }
                let path = format!("src/components/{name}.tsx");
                let css_path = format!("src/components/{name}.module.css");
                for p in [&path, &css_path] {
                    if let Some(owner) = snapshot
                        .elements
                        .iter()
                        .find(|e| e.source.as_ref().is_some_and(|s| &s.file == p))
                    {
                        return conflict(format!("{p} is already owned by element {}", owner.id.0));
                    }
                    if snapshot.owned.iter().any(|o| &o.path == p) || ws.current(p)?.is_some() {
                        return conflict(format!("{p} already exists or is owned"));
                    }
                }
                let css = emit::element_css(*kind);
                let class = emit::root_class(*kind);
                let mut e = ElementRecord::new(id.clone(), *kind, DEFAULT_PAGE);
                // A button's label is its display label (destination questions
                // and summaries quote it).
                e.label = (*kind == ElementKind::Button).then(|| content.text.clone()).flatten();
                e.content = Some(content.clone());
                e.source = Some(SourceBinding {
                    file: path.clone(),
                    region: None,
                });
                // The planner's view of what the stylesheet declares.
                e.style = Some(StyleBinding {
                    file: css_path.clone(),
                    class: class.into(),
                    region: None,
                    scope: StyleScope::Instance,
                    values: emit::css_declared_values(&css, &format!(".{class}")),
                    instance_override: false,
                });
                e.layout = matches!(kind, ElementKind::Hero | ElementKind::Form)
                    .then_some(ContainerLayout::Vertical);
                if *kind == ElementKind::Button {
                    e.destination = Some(Destination::Unresolved);
                }
                snapshot.elements.push(e);
                ws.set(&path, None, render_element(dir, &snapshot, id)?)?;
                ws.set(&css_path, None, css)?;
                note_issued(&mut snapshot, id);
                focus = Some((id.clone(), *kind));
            }
            Operation::InsertElement {
                element,
                page,
                parent: None,
                at,
            } if snapshot
                .element(element)
                .is_some_and(|e| e.kind.is_flow()) =>
            {
                if page != DEFAULT_PAGE {
                    return unsupported("only the home page can receive elements");
                }
                if let Some(position) = at {
                    if let Err(outcome) = check_placement(&snapshot, element, position) {
                        return Ok(outcome);
                    }
                }
                reorder(&mut snapshot, element, at.as_ref());
                if let Some(reason) = refresh_home_flow(&mut ws, &snapshot)? {
                    return conflict(reason);
                }
            }
            Operation::InsertElement {
                element,
                page,
                parent: None,
                ..
            } => {
                let Some(rec) = snapshot.element(element) else {
                    return unsupported(format!("unknown element {}", element.0));
                };
                if rec.kind != ElementKind::Navigation || page != DEFAULT_PAGE {
                    return unsupported("only a top navigation on the home page can be inserted");
                }
                let Some(name) = component_name(rec) else {
                    return unsupported(format!("element {} has no component source", element.0));
                };
                let Some(text) = ws.current(LAYOUT_FILE)? else {
                    return conflict(format!("{LAYOUT_FILE} is missing"));
                };
                let Some(body) = extract_region(&text, LAYOUT_REGION) else {
                    return conflict(format!("{LAYOUT_FILE} lost its {LAYOUT_REGION} markers"));
                };
                let mut components = emit::layout_top_components(&body);
                if components.contains(&name) {
                    return conflict(format!(
                        "element {} is already inserted in {LAYOUT_FILE}",
                        element.0
                    ));
                }
                components.push(name);
                let body = emit::layout_top_region(&components);
                let Some(new_text) = replace_region(&text, LAYOUT_REGION, &body) else {
                    return conflict(format!("{LAYOUT_FILE} lost its {LAYOUT_REGION} markers"));
                };
                ws.set(LAYOUT_FILE, Some(LAYOUT_REGION), new_text)?;
            }
            Operation::AddNavigationItem {
                navigation,
                item,
                label,
                destination,
            } => {
                match link_for(&snapshot, destination) {
                    Ok(_) => {}
                    Err(LinkError::Missing(m)) => return conflict(m),
                    Err(LinkError::Invalid(m)) => return unsupported(m),
                }
                let trimmed = label.trim();
                if trimmed.is_empty() || label.chars().count() > emit::MAX_LABEL_CHARS {
                    return unsupported(format!(
                        "a navigation label must have 1 to {} characters",
                        emit::MAX_LABEL_CHARS
                    ));
                }
                if !is_element_id(&item.0) {
                    return unsupported(format!("invalid element id: {}", item.0));
                }
                let Some(nav) = snapshot.element(navigation) else {
                    return conflict(format!("navigation {} no longer exists", navigation.0));
                };
                if nav.kind != ElementKind::Navigation {
                    return unsupported(format!("{} is not a navigation", navigation.0));
                }
                let Some(source) = nav.source.as_ref().filter(|s| s.region.is_none()) else {
                    return unsupported(format!(
                        "navigation {} is not a generated component",
                        navigation.0
                    ));
                };
                let file = source.file.clone();
                let page = nav.page.clone();
                if !snapshot
                    .owned
                    .iter()
                    .any(|o| o.path == file && o.region.is_none())
                    || ws.current(&file)?.is_none()
                {
                    return conflict(format!("{file} is missing or not owned"));
                }
                if snapshot.element(item).is_some() {
                    return conflict(format!("element id {} already exists", item.0));
                }
                let mut record =
                    ElementRecord::new(item.clone(), ElementKind::NavigationItem, page);
                record.label = Some(label.clone());
                record.parent = Some(navigation.clone());
                record.destination = Some(destination.clone());
                snapshot.elements.push(record);
                note_issued(&mut snapshot, item);
                refresh_navigation(&mut ws, &snapshot, navigation, &file)?;
                focus = Some((navigation.clone(), ElementKind::Navigation));
            }
            Operation::MoveElement { element, position } => {
                let Some(rec) = snapshot.element(element) else {
                    return conflict(format!("element {} no longer exists", element.0));
                };
                if !rec.kind.is_flow() || rec.page != DEFAULT_PAGE || rec.parent.is_some() {
                    return unsupported(format!("{} cannot be moved", element.0));
                }
                let kind = rec.kind;
                if let Err(outcome) = check_placement(&snapshot, element, position) {
                    return Ok(outcome);
                }
                reorder(&mut snapshot, element, Some(position));
                if let Some(reason) = refresh_home_flow(&mut ws, &snapshot)? {
                    return conflict(reason);
                }
                focus = Some((element.clone(), kind));
            }
            Operation::SetNavigationDestination { item, destination } => {
                match link_for(&snapshot, destination) {
                    Ok(Some(_)) => {}
                    Ok(None) => {
                        return unsupported(
                            "an unresolved destination cannot be set; leave the item unlinked",
                        )
                    }
                    Err(LinkError::Missing(m)) => return conflict(m),
                    Err(LinkError::Invalid(m)) => return unsupported(m),
                }
                let Some(rec) = snapshot.element(item) else {
                    return conflict(format!("navigation item {} no longer exists", item.0));
                };
                if rec.kind == ElementKind::Button {
                    if rec.destination != Some(Destination::Unresolved) {
                        return conflict(format!("button {} already has a destination", item.0));
                    }
                    let Some(file) = rec
                        .source
                        .as_ref()
                        .filter(|s| s.region.is_none())
                        .map(|s| s.file.clone())
                    else {
                        return unsupported(format!(
                            "button {} is not a generated component",
                            item.0
                        ));
                    };
                    if !snapshot
                        .owned
                        .iter()
                        .any(|o| o.path == file && o.region.is_none())
                        || ws.current(&file)?.is_none()
                    {
                        return conflict(format!("{file} is missing or not owned"));
                    }
                    snapshot
                        .elements
                        .iter_mut()
                        .find(|e| &e.id == item)
                        .expect("button was just found")
                        .destination = Some(destination.clone());
                    ws.set(&file, None, render_element(dir, &snapshot, item)?)?;
                    focus = Some((item.clone(), ElementKind::Button));
                    continue;
                }
                if rec.kind != ElementKind::NavigationItem {
                    return unsupported(format!("{} is not a navigation item", item.0));
                }
                if rec.destination != Some(Destination::Unresolved) {
                    return conflict(format!(
                        "navigation item {} already has a destination",
                        item.0
                    ));
                }
                let Some(navigation) = rec.parent.clone() else {
                    return unsupported(format!("navigation item {} has no navigation", item.0));
                };
                let Some(source) = snapshot
                    .element(&navigation)
                    .filter(|n| n.kind == ElementKind::Navigation)
                    .and_then(|n| n.source.as_ref())
                    .filter(|s| s.region.is_none())
                else {
                    return unsupported(format!(
                        "navigation {} is not a generated component",
                        navigation.0
                    ));
                };
                let file = source.file.clone();
                if !snapshot
                    .owned
                    .iter()
                    .any(|o| o.path == file && o.region.is_none())
                    || ws.current(&file)?.is_none()
                {
                    return conflict(format!("{file} is missing or not owned"));
                }
                snapshot
                    .elements
                    .iter_mut()
                    .find(|e| &e.id == item)
                    .expect("item was just found")
                    .destination = Some(destination.clone());
                refresh_navigation(&mut ws, &snapshot, &navigation, &file)?;
                focus = Some((navigation, ElementKind::Navigation));
            }
            Operation::CreatePage { page, label } => {
                if !emit::is_valid_slug(page) || page == DEFAULT_PAGE {
                    return unsupported(format!("invalid page id: {page}"));
                }
                if label.trim().is_empty() || label.chars().count() > emit::MAX_LABEL_CHARS {
                    return unsupported(format!(
                        "a page name must have 1 to {} characters",
                        emit::MAX_LABEL_CHARS
                    ));
                }
                let component = emit::page_component_name(page);
                if snapshot.page_exists(page)
                    || snapshot.pages.iter().any(|p| p.component == component)
                {
                    return conflict(format!("page {page} already exists"));
                }
                let path = format!("src/pages/{component}.tsx");
                let css_path = format!("src/pages/{component}.module.css");
                for p in [&path, &css_path] {
                    if snapshot.owned.iter().any(|o| &o.path == p) || ws.current(p)?.is_some() {
                        return conflict(format!("{p} already exists or is owned"));
                    }
                }
                snapshot.pages.push(PageRecord {
                    id: page.clone(),
                    label: label.clone(),
                    path: emit::page_path(page),
                    component: component.clone(),
                    registered: false,
                });
                ws.set(&path, None, emit::page_tsx(&component, page, label))?;
                ws.set(&css_path, None, emit::page_css())?;
            }
            Operation::RegisterRoute { page, path } => {
                let Some(rec) = snapshot.pages.iter_mut().find(|p| &p.id == page) else {
                    return conflict(format!("page {page} does not exist"));
                };
                if &rec.path != path {
                    return unsupported(format!(
                        "route {path} does not match the path of page {page}"
                    ));
                }
                if rec.registered {
                    return conflict(format!("the route of page {page} is already registered"));
                }
                rec.registered = true;
                let Some(text) = ws.current(ROUTER_FILE)? else {
                    return conflict(format!("{ROUTER_FILE} is missing"));
                };
                if extract_region(&text, ROUTES_REGION).is_none() {
                    return conflict(format!(
                        "{ROUTER_FILE} has no protopie:begin/end {ROUTES_REGION} region \
                         (the project predates page support); add the markers inside the \
                         routes array as in the reference template"
                    ));
                }
                let routes: Vec<_> = snapshot
                    .pages
                    .iter()
                    .filter(|p| p.registered)
                    .map(|p| emit::RouteView {
                        path: &p.path,
                        component: &p.component,
                    })
                    .collect();
                let body = emit::routes_region(&routes);
                let new_text =
                    replace_region(&text, ROUTES_REGION, &body).expect("region was just found");
                ws.set(ROUTER_FILE, Some(ROUTES_REGION), new_text)?;
            }
            Operation::SetStyle {
                target,
                scope,
                edits,
                ..
            } => {
                if *scope != StyleScope::Instance {
                    return unsupported("only instance-scoped style edits are supported");
                }
                if edits.is_empty()
                    || edits
                        .iter()
                        .any(|e| !style::is_valid_value(e.property, &e.value))
                {
                    return unsupported("a style edit is empty or outside the style policy");
                }
                for (i, e) in edits.iter().enumerate() {
                    if edits[..i].iter().any(|p| p.property == e.property) {
                        return unsupported("a style property is edited twice");
                    }
                }
                let Some(rec) = snapshot.element(target) else {
                    return conflict(format!("element {} no longer exists", target.0));
                };
                let kind = rec.kind;
                let Some(binding) = rec.style.clone() else {
                    return unsupported(format!("element {} has no style binding", target.0));
                };
                if !is_element_id(&target.0) || !emit::is_css_class(&binding.class) {
                    return unsupported(format!("invalid style binding of {}", target.0));
                }
                if !snapshot
                    .owned
                    .iter()
                    .any(|o| o.path == binding.file && o.region == binding.region)
                {
                    return conflict(format!(
                        "{} ({}) is not owned by the agent",
                        binding.file,
                        binding.region.as_deref().unwrap_or("whole file")
                    ));
                }
                let Some(text) = ws.current(&binding.file)? else {
                    return conflict(format!("{} is missing", binding.file));
                };
                let body = match &binding.region {
                    Some(r) => match extract_region(&text, r) {
                        Some(b) => b,
                        None => return conflict(format!("{} lost its {r} markers", binding.file)),
                    },
                    None => text.clone(),
                };
                let pairs: Vec<(&str, &str)> = edits
                    .iter()
                    .map(|e| (e.property.css_name(), e.value.as_str()))
                    .collect();
                // A shared (definition) class is never edited for one instance:
                // the instance gets its own more specific rule instead.
                let creating = binding.scope == StyleScope::Definition;
                let new_body = if creating {
                    emit::css_append_rule(
                        &body,
                        &emit::instance_selector(&binding.class, target),
                        &pairs,
                    )
                } else {
                    let selector = if binding.instance_override {
                        emit::instance_selector(&binding.class, target)
                    } else {
                        format!(".{}", binding.class)
                    };
                    match emit::css_set_declarations(&body, &selector, &pairs) {
                        Some(b) => b,
                        None => {
                            return conflict(format!(
                                "rule {selector} was not found in {}",
                                binding.file
                            ))
                        }
                    }
                };
                let new_text = match &binding.region {
                    Some(r) => replace_region(&text, r, &new_body).expect("region was just found"),
                    None => new_body,
                };
                ws.set(&binding.file, binding.region.as_deref(), new_text)?;
                let rec = snapshot
                    .elements
                    .iter_mut()
                    .find(|e| &e.id == target)
                    .expect("element was just found");
                let style = rec.style.as_mut().expect("binding was just found");
                if creating {
                    style.scope = StyleScope::Instance;
                    style.instance_override = true;
                }
                for e in edits {
                    style
                        .values
                        .insert(e.property.css_name().to_string(), e.value.clone());
                }
                focus = Some((target.clone(), kind));
            }
            Operation::CreateContext {
                id,
                label,
                name,
                shape,
                initial,
                scope,
            } => {
                if !emit::is_context_id(id)
                    || !emit::is_context_name(name)
                    || emit::context_names(label).as_ref() != Some(&(id.clone(), name.clone()))
                    || label.chars().count() > emit::MAX_LABEL_CHARS
                {
                    return unsupported(format!("invalid context id, name or label: {id} / {name}"));
                }
                if let Err(message) = emit::validate_initial(*shape, initial) {
                    return unsupported(message);
                }
                if snapshot.context(id).is_some() {
                    return conflict(format!("context {id} already exists"));
                }
                if snapshot
                    .contexts
                    .iter()
                    .any(|c| c.name.eq_ignore_ascii_case(name))
                {
                    return conflict(format!("a context file named {name} already exists"));
                }
                let path = format!("src/context/{name}.tsx");
                if snapshot.owned.iter().any(|o| o.path == path) || ws.current(&path)?.is_some() {
                    return conflict(format!("{path} already exists or is owned"));
                }
                snapshot.contexts.push(ContextRecord {
                    id: id.clone(),
                    label: label.clone(),
                    name: name.clone(),
                    shape: *shape,
                    initial: initial.clone(),
                    scope: *scope,
                    consumers: Vec::new(),
                });
                ws.set(&path, None, emit::context_tsx(name, *shape, initial))?;
                if let Some(reason) = refresh_providers(&mut ws, &snapshot)? {
                    return conflict(reason);
                }
            }
            Operation::AddContextConsumer { context, page } => {
                let Some(rec) = snapshot.contexts.iter_mut().find(|c| &c.id == context) else {
                    return conflict(format!("context {context} no longer exists"));
                };
                if rec.consumers.iter().any(|p| p == page) {
                    return conflict(format!("page {page} already reads context {context}"));
                }
                if snapshot.pages.iter().all(|p| &p.id != page) {
                    return conflict(format!("page {page} no longer exists"));
                }
                snapshot
                    .contexts
                    .iter_mut()
                    .find(|c| &c.id == context)
                    .expect("context was just found")
                    .consumers
                    .push(page.clone());
                if let Some(reason) = refresh_page(&mut ws, &snapshot, page)? {
                    return conflict(reason);
                }
            }
            other => return unsupported(format!("operation not supported yet: {other:?}")),
        }
    }

    let mut files = ws.files;
    for f in &mut files {
        f.diff = render_diff(&f.path, f.before.as_deref(), &f.after);
        let fp = match &f.region {
            Some(r) => extract_region(&f.after, r).map(|b| fingerprint(&b)),
            None => Some(fingerprint(&f.after)),
        };
        let fp = fp.expect("rendered region exists");
        match snapshot
            .owned
            .iter_mut()
            .find(|o| o.path == f.path && o.region == f.region)
        {
            Some(o) => o.fingerprint = fp,
            None => snapshot.owned.push(OwnedSource {
                path: f.path.clone(),
                region: f.region.clone(),
                fingerprint: fp,
            }),
        }
    }
    snapshot.revision += 1;
    session.advance_turn();
    if let Some((id, kind)) = &focus {
        session.record_success(id.clone(), *kind);
    }
    session
        .pending_questions
        .retain(|q| !plan.resolves.contains(&q.id));
    // Questions about items that were deleted or got a destination are stale.
    session.pending_questions.retain(|q| {
        q.continuation.item().map_or(true, |id| {
            snapshot.element(id).is_some_and(|e| {
                e.kind.has_destination() && e.destination == Some(Destination::Unresolved)
            })
        })
    });
    for q in &plan.follow_up {
        session.add_pending(q.clone());
    }
    Ok(PrepareOutcome::Ready(Prepared {
        plan_id: plan.id.clone(),
        base_revision: plan.base_revision,
        files,
        snapshot,
        session,
    }))
}

// ------------------------------------------------------------------ apply

/// Applies `plan` to the project at `dir`; the only writing stage.
pub fn apply_plan(
    dir: &Path,
    conversation_id: &str,
    plan: &Plan,
    options: &ApplyOptions,
) -> Result<ApplyOutcome> {
    let _lock = ProjectLock::acquire(dir)?;
    if let RecoverOutcome::Conflict { reason } = recover_locked(dir)? {
        return Ok(ApplyOutcome::Conflict { reason });
    }
    let mut ledger = load_ledger(dir)?;
    let by_request = options.request_id.and_then(|r| {
        ledger
            .entries
            .iter()
            .find(|e| e.request_id.as_deref() == Some(r))
    });
    if let Some(rec) = by_request {
        return Ok(
            if rec.plan_id == plan.id
                && rec.conversation_id.as_deref() == Some(conversation_id)
                && rec.plan.as_ref() == Some(plan)
                && rec.command.as_deref() == options.command
            {
                ApplyOutcome::Replayed(rec.clone())
            } else {
                ApplyOutcome::Conflict {
                    reason: "request id was already used for a different plan".into(),
                }
            },
        );
    }
    if let Some(rec) = ledger.entries.iter().find(|e| e.plan_id == plan.id) {
        return Ok(
            if rec.conversation_id.as_deref() == Some(conversation_id)
                && rec.plan.as_ref() == Some(plan)
                && (options.request_id.is_none() || rec.request_id.as_deref() == options.request_id)
                && (rec.request_id.is_none() || rec.command.as_deref() == options.command)
            {
                ApplyOutcome::Replayed(rec.clone())
            } else {
                ApplyOutcome::Conflict {
                    reason:
                        "plan id was already used for a different plan, conversation, or request id"
                            .into(),
                }
            },
        );
    }

    let prepared = match prepare(dir, plan, conversation_id)? {
        PrepareOutcome::Ready(p) => p,
        PrepareOutcome::Conflict { reason } => return Ok(ApplyOutcome::Conflict { reason }),
        PrepareOutcome::Unsupported { reason } => return Ok(ApplyOutcome::Unsupported { reason }),
    };
    let record = AppliedRecord {
        plan_id: plan.id.clone(),
        request_id: options.request_id.map(Into::into),
        command: options.command.map(Into::into),
        conversation_id: Some(conversation_id.into()),
        plan: Some(plan.clone()),
        revision: prepared.snapshot.revision,
        changed_files: prepared.files.iter().map(|f| f.path.clone()).collect(),
    };
    ledger.entries.push(record.clone());

    let mut entries: Vec<Entry> = prepared
        .files
        .iter()
        .map(|f| Entry {
            path: f.path.clone(),
            before: f.before.clone(),
            after: f.after.clone(),
            region: f.region.clone(),
            abort_source: None,
            abort_target: None,
        })
        .collect();
    let model_rel = project::model_rel();
    let session_rel = project::session_rel(conversation_id)?;
    for (path, after) in [
        (model_rel, json(dir, "model.json", &prepared.snapshot)?),
        (session_rel, json(dir, "session", &prepared.session)?),
        (LEDGER_FILE.to_string(), json(dir, LEDGER_FILE, &ledger)?),
    ] {
        let before = read_text(dir, &path)?;
        entries.push(Entry {
            path,
            before,
            after,
            region: None,
            abort_source: None,
            abort_target: None,
        });
    }
    let journal = Journal {
        record: record.clone(),
        entries,
        mode: JournalMode::RollForward,
        preserved_paths: Vec::new(),
    };
    // Commit point: once the journal is durable, recovery can finish the job.
    write_file(dir, JOURNAL_FILE, &json(dir, JOURNAL_FILE, &journal)?)?;
    for (written, entry) in journal.entries.iter().enumerate() {
        if options.fail_after_writes == Some(written) {
            return Err(Error::Io {
                path: dir.join(&entry.path),
                source: io::Error::other("injected write failure"),
            });
        }
        write_file(dir, &entry.path, &entry.after)?;
    }
    if options.fail_before_journal_removal {
        return Err(Error::Io {
            path: dir.join(JOURNAL_FILE),
            source: io::Error::other("injected failure before journal removal"),
        });
    }
    remove_file(dir, JOURNAL_FILE)?;
    Ok(ApplyOutcome::Applied(record))
}

#[cfg(test)]
#[path = "apply_tests.rs"]
mod tests;
