//! UI prototype agent: scaffolds TypeScript + SolidJS projects and (later) modifies them.
//!
//! Template location: the template is NOT embedded in the binary. It is read at
//! runtime from `<CARGO_MANIFEST_DIR>/../../reference`, a path fixed at compile
//! time so it never depends on the current working directory. (Embedding via
//! `include_dir` would also pull in `node_modules/` and `dist/`.)

use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};

pub mod apply;
pub mod contracts;
pub mod emit;
pub mod parser;
pub mod pipeline;
pub mod project;

use contracts::{ModifyOutcome, ModifyResult, PlanOutcome, Rejection, RejectionReason};

/// Directory names never copied from the template.
const SKIP: &[&str] = &["node_modules", "dist", ".git", "target", ".DS_Store"];

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("io error at {path}: {source}")]
    Io { path: PathBuf, source: io::Error },
    #[error("project path is not an existing directory: {0}")]
    NotADirectory(PathBuf),
    #[error("path escapes the project: {0}")]
    Escape(PathBuf),
    #[error("invalid project name {0:?}: use lowercase letters, digits and '-' only")]
    InvalidSlug(String),
    #[error("project already exists: {0}")]
    AlreadyExists(PathBuf),
    #[error("invalid project metadata at {path}: {message}")]
    Metadata { path: PathBuf, message: String },
    #[error("unsupported project: {0}")]
    UnsupportedProject(String),
    #[error("project already has a model: {0}")]
    AlreadyInitialized(PathBuf),
    #[error("invalid conversation id {0:?}: use lowercase letters, digits and '-' only")]
    InvalidConversationId(String),
    #[error("project revision is {actual}, expected {expected}")]
    StaleRevision { expected: u64, actual: u64 },
}

pub type Result<T> = std::result::Result<T, Error>;

pub(crate) fn io_err(path: &Path) -> impl FnOnce(io::Error) -> Error + '_ {
    move |source| Error::Io {
        path: path.to_path_buf(),
        source,
    }
}

fn template_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../reference")
}

fn copy_template(src: &Path, dst: &Path) -> Result<()> {
    fs::create_dir_all(dst).map_err(io_err(dst))?;
    for entry in fs::read_dir(src).map_err(io_err(src))? {
        let entry = entry.map_err(io_err(src))?;
        let name = entry.file_name();
        if SKIP.iter().any(|s| name == *s) {
            continue;
        }
        let to = dst.join(&name);
        let from = entry.path();
        if entry.file_type().map_err(io_err(&from))?.is_dir() {
            copy_template(&from, &to)?;
        } else {
            fs::copy(&from, &to).map_err(io_err(&from))?;
        }
    }
    Ok(())
}

/// Creates `dir` (failing if it exists, so concurrent callers cannot collide),
/// copies the template into it and returns its canonicalised path.
fn create_project_dir(dir: &Path) -> Result<PathBuf> {
    match fs::create_dir(dir) {
        Ok(()) => {
            copy_template(&template_dir(), dir)?;
            project::initialize_project(dir)?;
            dir.canonicalize().map_err(io_err(dir))
        }
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
            Err(Error::AlreadyExists(dir.to_path_buf()))
        }
        Err(e) => Err(Error::Io {
            path: dir.to_path_buf(),
            source: e,
        }),
    }
}

/// Creates a new `project-<n>` folder under `root` (created if missing), copies
/// the template into it and returns its absolute, canonicalised path.
/// Existing folders are never reused: `n` is the first unused number from 1.
pub fn init(root: &Path) -> Result<PathBuf> {
    fs::create_dir_all(root).map_err(io_err(root))?;
    let mut n = 1u32;
    loop {
        match create_project_dir(&root.join(format!("project-{n}"))) {
            Err(Error::AlreadyExists(_)) => n += 1,
            other => return other,
        }
    }
}

/// True if `slug` is non-empty and made only of `[a-z0-9-]` (so it can never
/// contain a path separator or `..`).
pub(crate) fn is_valid_slug(slug: &str) -> bool {
    !slug.is_empty()
        && slug
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// Like [`init`] but names the folder `slug`. Fails with [`Error::InvalidSlug`]
/// for an invalid slug and [`Error::AlreadyExists`] if the folder exists.
pub fn init_named(root: &Path, slug: &str) -> Result<PathBuf> {
    if !is_valid_slug(slug) {
        return Err(Error::InvalidSlug(slug.to_string()));
    }
    fs::create_dir_all(root).map_err(io_err(root))?;
    create_project_dir(&root.join(slug))
}

/// Lowercases `name`, turns every run of non-alphanumeric characters into a
/// single `-` and trims leading/trailing dashes. May return an empty string.
pub fn slugify(name: &str) -> String {
    let mut out = String::new();
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    out
}

/// Sorted names of the project folders directly under `root`; empty if `root`
/// does not exist.
pub fn list_projects(root: &Path) -> Result<Vec<String>> {
    let entries = match fs::read_dir(root) {
        Ok(e) => e,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => {
            return Err(Error::Io {
                path: root.to_path_buf(),
                source: e,
            })
        }
    };
    let mut names = Vec::new();
    for entry in entries {
        let entry = entry.map_err(io_err(root))?;
        let path = entry.path();
        if path.is_dir() {
            if let Some(name) = entry.file_name().to_str() {
                names.push(name.to_string());
            }
        }
    }
    names.sort();
    Ok(names)
}

/// Options of [`modify_with`].
#[derive(Debug, Default, Clone)]
pub struct ModifyOptions<'a> {
    /// Conversation scope; the default conversation when absent.
    pub conversation_id: Option<&'a str>,
    /// API-boundary request ID. A repeated ID is a transport retry: the
    /// recorded result is returned and nothing is applied again.
    pub request_id: Option<&'a str>,
    /// Prepare only: report what would change and write nothing.
    pub dry_run: bool,
}

/// Applies a natural-language `command` to the project at `project_path` in the
/// default conversation. See [`modify_with`].
pub fn modify(project_path: &Path, command: &str) -> Result<ModifyResult> {
    modify_with(project_path, command, &ModifyOptions::default())
}

fn applied_summary(plan: &contracts::Plan) -> String {
    use contracts::Operation as Op;
    let parts: Vec<String> = plan
        .operations
        .iter()
        .filter_map(|op| match op {
            Op::CreateComponent { id, .. } => Some(format!("Added a top navigation ({}).", id.0)),
            Op::AddNavigationItem {
                navigation, label, ..
            } => Some(format!(
                "Added \u{201c}{label}\u{201d} to {} (not linked yet).",
                navigation.0
            )),
            _ => None,
        })
        .collect();
    if parts.is_empty() {
        "Applied the change.".into()
    } else {
        parts.join(" ")
    }
}

fn applied_result(
    plan_id: String,
    changed_files: Vec<String>,
    plan: Option<&contracts::Plan>,
    summary_suffix: &str,
) -> ModifyResult {
    let summary = match plan {
        Some(plan) => applied_summary(plan),
        None => "Applied the change.".into(),
    } + summary_suffix;
    ModifyResult {
        summary,
        outcome: ModifyOutcome::Applied {
            plan_id,
            changed_files,
            follow_up: plan.map(|p| p.follow_up.clone()).unwrap_or_default(),
        },
    }
}

fn conflict_result(reason: String) -> ModifyResult {
    ModifyResult {
        summary: reason.clone(),
        outcome: ModifyOutcome::Conflict { reason },
    }
}

/// Interprets `command` for the project at `project_path`.
///
/// Loads `.protopie/` state (model and conversation) and runs the pure
/// parse/resolve/plan pipeline. A ready plan is applied through
/// [`apply::apply_plan`], which also records focus and pending follow-up
/// questions; with `dry_run` it is only prepared and nothing is written.
/// Questions, unsupported prompts, no-change results and conflicts never write.
/// A project without a model is reported as unsupported with initialization
/// advice. Unsupported prompts keep the legacy `received: <command>` summary.
pub fn modify_with(
    project_path: &Path,
    command: &str,
    options: &ModifyOptions,
) -> Result<ModifyResult> {
    if !project_path.is_dir() {
        return Err(Error::NotADirectory(project_path.to_path_buf()));
    }
    let conversation_id = options
        .conversation_id
        .unwrap_or(contracts::DEFAULT_CONVERSATION_ID);
    // Validates the conversation ID before anything else happens.
    project::load_session(project_path, conversation_id)?;
    // Planning is read-only. A pending journal needs an explicit recovery or
    // abort before the model and source can be trusted together.
    if apply::recovery_pending(project_path)? {
        return Ok(conflict_result("An interrupted application is pending. Select the project and use /abort-interrupted-apply to preserve external edits and abandon the interrupted application.".into()));
    }
    if let (false, Some(request_id)) = (options.dry_run, options.request_id) {
        if let Some(record) = apply::lookup_request(project_path, request_id)? {
            if record.conversation_id.as_deref() != Some(conversation_id) {
                return Ok(conflict_result(
                    "request id was already used in a different conversation".into(),
                ));
            }
            if record.command.as_deref() != Some(command) {
                return Ok(conflict_result(
                    "request id was already used for a different command or has no recorded command".into(),
                ));
            }
            return Ok(applied_result(
                record.plan_id,
                record.changed_files,
                record.plan.as_ref(),
                " (already applied)",
            ));
        }
    }
    let snapshot = match project::load_project(project_path)? {
        project::ProjectState::Loaded(snapshot) => snapshot,
        project::ProjectState::NoMetadata => {
            return Ok(ModifyResult {
                summary: "This project has no .protopie model yet. Initialize it first \
                          (protopie_ui_agent::project::initialize_project)."
                    .into(),
                outcome: ModifyOutcome::Unsupported {
                    rejection: Rejection {
                        reason: RejectionReason::ProjectNotInitialized,
                        explanation: "missing .protopie/model.json; initialize the project".into(),
                        span: None,
                    },
                },
            })
        }
    };
    let session = project::load_session(project_path, conversation_id)?;
    let outcome = pipeline::plan_prompt(command, &snapshot, &session);
    Ok(match outcome {
        PlanOutcome::Unsupported { rejection } => ModifyResult {
            summary: format!("received: {command}"),
            outcome: ModifyOutcome::Unsupported { rejection },
        },
        PlanOutcome::NeedsClarification { questions } => ModifyResult {
            summary: questions
                .iter()
                .map(|q| q.prompt.as_str())
                .collect::<Vec<_>>()
                .join("\n"),
            outcome: ModifyOutcome::NeedsClarification { questions },
        },
        PlanOutcome::NoChange { reason, follow_up } => ModifyResult {
            summary: reason.clone(),
            outcome: ModifyOutcome::NoChange { reason, follow_up },
        },
        PlanOutcome::Ready { plan } if options.dry_run => {
            match apply::prepare(project_path, &plan, conversation_id)? {
                apply::PrepareOutcome::Ready(prepared) => ModifyResult {
                    summary: format!("Preview: {}", applied_summary(&plan)),
                    outcome: ModifyOutcome::Preview {
                        plan_id: plan.id.clone(),
                        changed_files: prepared.files.iter().map(|f| f.path.clone()).collect(),
                        follow_up: plan.follow_up.clone(),
                    },
                },
                apply::PrepareOutcome::Conflict { reason } => conflict_result(reason),
                apply::PrepareOutcome::Unsupported { reason } => {
                    unsupported_result(command, reason)
                }
            }
        }
        PlanOutcome::Ready { plan } => {
            let apply_options = apply::ApplyOptions {
                request_id: options.request_id,
                command: options.request_id.map(|_| command),
                ..Default::default()
            };
            match apply::apply_plan(project_path, conversation_id, &plan, &apply_options)? {
                apply::ApplyOutcome::Applied(record) => {
                    applied_result(record.plan_id, record.changed_files, Some(&plan), "")
                }
                apply::ApplyOutcome::Replayed(record) => applied_result(
                    record.plan_id,
                    record.changed_files,
                    Some(&plan),
                    " (already applied)",
                ),
                apply::ApplyOutcome::Conflict { reason } => conflict_result(reason),
                apply::ApplyOutcome::Unsupported { reason } => unsupported_result(command, reason),
            }
        }
    })
}

fn unsupported_result(command: &str, reason: String) -> ModifyResult {
    ModifyResult {
        summary: format!("received: {command}"),
        outcome: ModifyOutcome::Unsupported {
            rejection: Rejection {
                reason: RejectionReason::CapabilityNotImplemented,
                explanation: reason,
                span: None,
            },
        },
    }
}

/// Resolves `relative` inside `project_path`, rejecting absolute paths, `..`
/// components and symlinks in the relative path. The target need not exist yet.
/// Rejecting even internal symlinks prevents metadata and source path aliases.
pub(crate) fn resolve_in_project(project_path: &Path, relative: &str) -> Result<PathBuf> {
    let rel = Path::new(relative);
    let escape = || Error::Escape(rel.to_path_buf());
    if rel.is_absolute() || rel.components().any(|c| !matches!(c, Component::Normal(_))) {
        return Err(escape());
    }
    let base = project_path.canonicalize().map_err(io_err(project_path))?;
    let mut joined = base.clone();
    for component in rel.components() {
        joined.push(component.as_os_str());
        match fs::symlink_metadata(&joined) {
            Ok(meta) if meta.file_type().is_symlink() => return Err(escape()),
            Ok(_) => {}
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => {
                return Err(Error::Io {
                    path: joined,
                    source: e,
                })
            }
        }
    }
    Ok(joined)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn init_copies_template() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("new-root");
        let a = init(&root).unwrap();
        let b = init(&root).unwrap();
        assert!(a.is_absolute());
        assert_ne!(a, b);
        assert!(a.join("package.json").is_file());
        assert!(a.join("src").is_dir());
        assert!(!a.join("node_modules").exists());
        assert!(!a.join("dist").exists());
    }

    #[test]
    fn slugify_normalises() {
        assert_eq!(slugify("My Cool App!"), "my-cool-app");
        assert_eq!(slugify("  --Hello__World--  "), "hello-world");
        assert_eq!(slugify("a   b"), "a-b");
        assert_eq!(slugify("***"), "");
        assert_eq!(slugify("../etc"), "etc");
    }

    #[test]
    fn init_named_validates_and_never_overwrites() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("root");
        let p = init_named(&root, "my-app").unwrap();
        assert!(p.ends_with("my-app"));
        assert!(p.join("package.json").is_file());
        assert!(matches!(
            init_named(&root, "my-app"),
            Err(Error::AlreadyExists(_))
        ));
        for bad in ["", "..", "a/b", "A", "a b", "a\\b", "../x"] {
            assert!(
                matches!(init_named(&root, bad), Err(Error::InvalidSlug(_))),
                "{bad:?}"
            );
        }
    }

    #[test]
    fn list_projects_sorted() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("root");
        assert!(list_projects(&root).unwrap().is_empty());
        init_named(&root, "zeta").unwrap();
        init_named(&root, "alpha").unwrap();
        fs::write(root.join("stray.txt"), "x").unwrap();
        assert_eq!(list_projects(&root).unwrap(), vec!["alpha", "zeta"]);
    }

    #[test]
    fn modify_echoes() {
        let tmp = tempfile::tempdir().unwrap();
        let project = init_named(tmp.path(), "p").unwrap();
        let result = modify(&project, "Add top header").unwrap();
        assert_eq!(result.summary, "received: Add top header");
        assert!(matches!(result.outcome, ModifyOutcome::Unsupported { .. }));
        assert!(matches!(
            modify(&tmp.path().join("nope"), "x"),
            Err(Error::NotADirectory(_))
        ));
    }

    #[test]
    fn resolve_rejects_escapes() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("proj");
        fs::create_dir(&p).unwrap();
        assert!(resolve_in_project(&p, "src/App.tsx")
            .unwrap()
            .ends_with("src/App.tsx"));
        assert!(resolve_in_project(&p, "/etc/passwd").is_err());
        assert!(resolve_in_project(&p, "../x").is_err());
        assert!(resolve_in_project(&p, "a/../../x").is_err());
        #[cfg(unix)]
        {
            let outside = tmp.path().join("outside");
            fs::create_dir(&outside).unwrap();
            std::os::unix::fs::symlink(&outside, p.join("link")).unwrap();
            assert!(resolve_in_project(&p, "link/file.txt").is_err());
        }
    }

    fn snapshot(dir: &Path) -> Vec<(PathBuf, Vec<u8>)> {
        let mut files = Vec::new();
        let mut stack = vec![dir.to_path_buf()];
        while let Some(d) = stack.pop() {
            for e in fs::read_dir(&d).unwrap() {
                let p = e.unwrap().path();
                if p.is_dir() {
                    stack.push(p);
                } else if p.file_name().is_some_and(|n| n != "lock") {
                    // The advisory lock file is empty, content-free state.
                    files.push((p.clone(), fs::read(&p).unwrap()));
                }
            }
        }
        files.sort();
        files
    }

    #[test]
    fn modify_never_mutates_the_project_unless_a_plan_is_ready_and_applied() {
        // Rejected, unsupported and clarification prompts never write. The
        // "Add top nav" prompt is ready, so it is only previewed here (dry
        // run); applying it is covered by the T5 tests below.
        let tmp = tempfile::tempdir().unwrap();
        let project = init_named(tmp.path(), "p").unwrap();
        let before = snapshot(&project);
        for prompt in [
            "Tell me a joke",
            "do not add a nav",
            "Add Contact Us",
            "Add navigation to top nav",
        ] {
            modify(&project, prompt).unwrap();
        }
        assert_eq!(snapshot(&project), before);
        let dry = ModifyOptions {
            dry_run: true,
            ..Default::default()
        };
        for prompt in ["Add top nav", "Tell me a joke"] {
            modify_with(&project, prompt, &dry).unwrap();
        }
        assert_eq!(snapshot(&project), before);
    }

    #[test]
    fn modify_reports_questions_with_summary() {
        let tmp = tempfile::tempdir().unwrap();
        let project = init_named(tmp.path(), "p").unwrap();
        let r = modify(&project, "Add navigation to top nav").unwrap();
        assert!(matches!(
            r.outcome,
            ModifyOutcome::NeedsClarification { .. }
        ));
        assert!(r.summary.contains("Add \"navigation\" to top nav"));
    }

    // ---- T5: navigation flow through modify (applies plans) ----

    fn run(project: &Path, prompt: &str) -> ModifyResult {
        modify(project, prompt).unwrap()
    }

    fn session_of(project: &Path) -> contracts::Session {
        project::load_session(project, contracts::DEFAULT_CONVERSATION_ID).unwrap()
    }

    fn read(project: &Path, rel: &str) -> String {
        fs::read_to_string(project.join(rel)).unwrap()
    }

    #[test]
    fn need_top_navigation_then_add_contact_us_applies_and_survives_restart() {
        let tmp = tempfile::tempdir().unwrap();
        let project = init_named(tmp.path(), "p").unwrap();
        let router_before = read(&project, "src/router.ts");
        let pages_before = read(&project, "src/pages/Home.tsx");

        let first = run(&project, "Need a top navigation");
        let ModifyOutcome::Applied { changed_files, .. } = &first.outcome else {
            panic!("{first:?}")
        };
        assert!(changed_files.contains(&"src/components/TopNav.tsx".to_string()));
        assert!(changed_files.contains(&"src/App.tsx".to_string()));
        assert_eq!(
            session_of(&project).focus,
            Some(contracts::ElementId::new("nav_1"))
        );
        assert!(session_of(&project).pending_questions.is_empty());
        assert!(read(&project, "src/App.tsx").contains("<TopNav />"));

        // "Restart": nothing is held in memory between calls; state is on disk.
        let second = run(&project, "Add Contact Us");
        let ModifyOutcome::Applied { follow_up, .. } = &second.outcome else {
            panic!("{second:?}")
        };
        assert_eq!(follow_up.len(), 1);
        assert_eq!(follow_up[0].kind, contracts::QuestionKind::FollowUp);
        assert!(follow_up[0].prompt.contains("Contact Us"));
        assert!(second.summary.contains("not linked yet"));

        // Pending question and focus are persisted.
        let session = session_of(&project);
        assert_eq!(session.pending_questions, *follow_up);
        assert_eq!(session.focus, Some(contracts::ElementId::new("nav_1")));

        // The item exists, with an unresolved destination, as plain text.
        let ProjectSnapshotView(model) = ProjectSnapshotView::load(&project);
        let item = model.element(&contracts::ElementId::new("item_1")).unwrap();
        assert_eq!(item.label.as_deref(), Some("Contact Us"));
        assert_eq!(item.destination, Some(contracts::Destination::Unresolved));
        assert_eq!(item.parent, Some(contracts::ElementId::new("nav_1")));
        let nav = read(&project, "src/components/TopNav.tsx");
        assert!(nav.contains("{\"Contact Us\"}"));
        assert!(nav.contains("<span class={styles.label}>"));
        assert!(!nav.contains("<a ") && !nav.contains("href"), "{nav}");

        // No page or route was created or registered.
        assert_eq!(read(&project, "src/router.ts"), router_before);
        assert_eq!(read(&project, "src/pages/Home.tsx"), pages_before);
        let mut pages: Vec<_> = fs::read_dir(project.join("src/pages"))
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        pages.sort();
        assert_eq!(pages, ["Home.module.css", "Home.tsx"]);
        assert!(project::verify_owned(&project, &model).unwrap().is_empty());
    }

    struct ProjectSnapshotView(contracts::ProjectSnapshot);
    impl ProjectSnapshotView {
        fn load(project: &Path) -> Self {
            match project::load_project(project).unwrap() {
                project::ProjectState::Loaded(s) => Self(s),
                _ => panic!("no model"),
            }
        }
    }

    #[test]
    fn duplicate_ensure_requests_do_not_duplicate_navigation() {
        let tmp = tempfile::tempdir().unwrap();
        let project = init_named(tmp.path(), "p").unwrap();
        assert!(matches!(
            run(&project, "Add top nav").outcome,
            ModifyOutcome::Applied { .. }
        ));
        let before = snapshot(&project);
        for prompt in ["Add top nav", "Need a top navigation"] {
            let r = run(&project, prompt);
            assert!(matches!(r.outcome, ModifyOutcome::NoChange { .. }), "{r:?}");
        }
        assert_eq!(snapshot(&project), before);
        assert_eq!(
            read(&project, "src/App.tsx").matches("<TopNav />").count(),
            1
        );
        assert_eq!(
            read(&project, "src/App.tsx")
                .matches("import TopNav")
                .count(),
            1
        );
    }

    #[test]
    fn dry_run_previews_without_touching_files_or_session() {
        let tmp = tempfile::tempdir().unwrap();
        let project = init_named(tmp.path(), "p").unwrap();
        let before = snapshot(&project);
        let dry = ModifyOptions {
            dry_run: true,
            ..Default::default()
        };
        let r = modify_with(&project, "Add top nav", &dry).unwrap();
        let ModifyOutcome::Preview { changed_files, .. } = &r.outcome else {
            panic!("{r:?}")
        };
        assert_eq!(changed_files.len(), 3);
        assert_eq!(snapshot(&project), before);
        assert_eq!(session_of(&project).focus, None);
        // The real run afterwards applies exactly what was previewed.
        let real = run(&project, "Add top nav");
        let ModifyOutcome::Applied {
            changed_files: applied,
            ..
        } = &real.outcome
        else {
            panic!("{real:?}")
        };
        assert_eq!(applied, changed_files);
    }

    #[test]
    fn request_id_retries_replay_and_do_not_reapply() {
        let tmp = tempfile::tempdir().unwrap();
        let project = init_named(tmp.path(), "p").unwrap();
        let options = ModifyOptions {
            request_id: Some("req-1"),
            ..Default::default()
        };
        assert!(matches!(
            modify_with(&project, "Add top nav", &options)
                .unwrap()
                .outcome,
            ModifyOutcome::Applied { .. }
        ));
        let after = snapshot(&project);
        // A retry of the same request replays, even though a replan would say NoChange.
        let retry = modify_with(&project, "Add top nav", &options).unwrap();
        assert!(
            matches!(retry.outcome, ModifyOutcome::Applied { .. }),
            "{retry:?}"
        );
        assert!(retry.summary.contains("already applied"));
        assert_eq!(snapshot(&project), after);
        // Reusing an ID with even a semantically equivalent prompt is a conflict.
        let changed = modify_with(&project, "Add a top nav", &options).unwrap();
        assert!(
            matches!(changed.outcome, ModifyOutcome::Conflict { .. }),
            "{changed:?}"
        );
        assert_eq!(snapshot(&project), after);
        // The same request id in another conversation is rejected.
        let other = ModifyOptions {
            conversation_id: Some("other"),
            request_id: Some("req-1"),
            ..Default::default()
        };
        let r = modify_with(&project, "Add top nav", &other).unwrap();
        assert!(matches!(r.outcome, ModifyOutcome::Conflict { .. }), "{r:?}");
        assert_eq!(snapshot(&project), after);
    }

    #[test]
    fn legacy_request_without_command_does_not_authorize_modify_replay() {
        let tmp = tempfile::tempdir().unwrap();
        let project = init_named(tmp.path(), "p").unwrap();
        let plan = match pipeline::plan_prompt(
            "Add top nav",
            &match project::load_project(&project).unwrap() {
                project::ProjectState::Loaded(s) => s,
                _ => panic!("missing project"),
            },
            &project::load_session(&project, contracts::DEFAULT_CONVERSATION_ID).unwrap(),
        ) {
            PlanOutcome::Ready { plan } => plan,
            other => panic!("{other:?}"),
        };
        let legacy = apply::ApplyOptions {
            request_id: Some("legacy"),
            ..Default::default()
        };
        assert!(matches!(
            apply::apply_plan(&project, contracts::DEFAULT_CONVERSATION_ID, &plan, &legacy)
                .unwrap(),
            apply::ApplyOutcome::Applied(_)
        ));
        let before = snapshot(&project);
        let retry = modify_with(
            &project,
            "Add top nav",
            &ModifyOptions {
                request_id: Some("legacy"),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(
            matches!(retry.outcome, ModifyOutcome::Conflict { .. }),
            "{retry:?}"
        );
        assert_eq!(snapshot(&project), before);
    }

    #[test]
    fn conversations_keep_separate_focus_and_invalid_ids_are_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        let project = init_named(tmp.path(), "p").unwrap();
        let a = ModifyOptions {
            conversation_id: Some("a"),
            ..Default::default()
        };
        modify_with(&project, "Add top nav", &a).unwrap();
        modify_with(&project, "Add Contact Us", &a).unwrap();
        assert_eq!(
            project::load_session(&project, "a")
                .unwrap()
                .pending_questions
                .len(),
            1
        );
        assert_eq!(session_of(&project).focus, None);
        assert!(session_of(&project).pending_questions.is_empty());
        let bad = ModifyOptions {
            conversation_id: Some("../x"),
            ..Default::default()
        };
        assert!(matches!(
            modify_with(&project, "Add top nav", &bad),
            Err(Error::InvalidConversationId(_))
        ));
    }

    #[test]
    fn failed_and_rejected_requests_record_no_focus_or_pending_question() {
        let tmp = tempfile::tempdir().unwrap();
        let project = init_named(tmp.path(), "p").unwrap();
        // External edit of an owned region makes the application a conflict.
        let app = project.join("src/App.tsx");
        let text = fs::read_to_string(&app)
            .unwrap()
            .replace("return <></>;", "return <>{/* mine */}</>;");
        fs::write(&app, text).unwrap();
        let before = snapshot(&project);
        let r = run(&project, "Add top nav");
        assert!(matches!(r.outcome, ModifyOutcome::Conflict { .. }), "{r:?}");
        let r = run(&project, "Add Contact Us");
        assert!(matches!(
            r.outcome,
            ModifyOutcome::NeedsClarification { .. }
        ));
        assert_eq!(snapshot(&project), before);
        let session = session_of(&project);
        assert_eq!(session.focus, None);
        assert!(session.pending_questions.is_empty());
        assert_eq!(session.turn, 0);
    }

    #[test]
    fn several_items_keep_order_and_labels_are_escaped_in_generated_source() {
        let tmp = tempfile::tempdir().unwrap();
        let project = init_named(tmp.path(), "p").unwrap();
        run(&project, "Add top nav");
        for prompt in [
            "Add Contact Us",
            "Add \"Tom's <b>{Shop}</b>\" to top nav",
            "Add Get in Touch",
        ] {
            let r = run(&project, prompt);
            assert!(
                matches!(r.outcome, ModifyOutcome::Applied { .. }),
                "{prompt}: {r:?}"
            );
        }
        let nav = read(&project, "src/components/TopNav.tsx");
        let order: Vec<_> = ["Contact Us", "Tom's \\u003cb\\u003e", "Get in Touch"]
            .iter()
            .map(|l| nav.find(l).unwrap_or_else(|| panic!("{l} in {nav}")))
            .collect();
        assert!(order.windows(2).all(|w| w[0] < w[1]), "{nav}");
        assert!(!nav.contains("<b>"));
        assert_eq!(nav.matches("<li").count(), 3);
        assert_eq!(session_of(&project).pending_questions.len(), 3);
    }
}
