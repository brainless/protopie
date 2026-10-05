//! Project model, ownership and conversation persistence (Epic 001, T3).
//!
//! # Ownership convention
//!
//! * Files the agent generates *wholly* are recorded as an [`OwnedSource`] with
//!   `region: None` and a fingerprint of the entire file.
//! * Shared files (the reference `App.tsx`, `router.ts`, `Home.tsx`,
//!   `Home.module.css`, `index.css`) are
//!   owned only inside *regions* delimited by marker lines containing
//!   `protopie:begin <id>` and `protopie:end <id>` (in any comment syntax).
//!   Everything outside the markers is never touched. A region's fingerprint
//!   covers the lines strictly between the two marker lines.
//! * `.protopie/model.json` is the manifest: it names entities (IDs, roles,
//!   bindings) and records the fingerprint of every owned source. Before a plan
//!   is prepared or applied, [`verify_owned`] must report no drift, otherwise
//!   the change is an external-edit conflict rather than an overwrite.
//!
//! # Projects without metadata
//!
//! [`load_project`] returns [`ProjectState::NoMetadata`]; `modify` reports it as
//! unsupported with an actionable message. [`initialize_project`] is the one
//! explicit migration path: it seeds a model from a project whose files still
//! carry the template's markers, and fails with
//! [`Error::UnsupportedProject`] (listing what is missing) otherwise.
//!
//! Fingerprints are FNV-1a 64-bit: deterministic, dependency-free change
//! detection, not a security boundary.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::contracts::*;
use crate::{io_err, is_valid_slug, Error, Result};

pub const PROTOPIE_DIR: &str = ".protopie";
pub const MODEL_FILE: &str = "model.json";
pub const SESSIONS_DIR: &str = "sessions";

/// Shared files of the reference template and the regions it exposes.
const TEMPLATE_REGIONS: &[(&str, &str)] = &[
    ("src/pages/Home.tsx", "hero_1"),
    ("src/pages/Home.tsx", "home-flow"),
    ("src/pages/Home.module.css", "hero_1"),
    ("src/App.tsx", "providers"),
    ("src/App.tsx", "layout-top"),
    ("src/router.ts", "routes"),
    ("src/index.css", "tokens"),
];

/// Result of loading a project's model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProjectState {
    Loaded(ProjectSnapshot),
    /// No `.protopie/model.json`: not yet initialized.
    NoMetadata,
}

/// An owned source that no longer matches its recorded fingerprint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Drift {
    pub path: String,
    pub region: Option<String>,
    pub kind: DriftKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DriftKind {
    /// Content differs from the recorded fingerprint (external edit).
    Modified,
    /// File, or the region's markers, no longer exist.
    Missing,
}

/// FNV-1a 64-bit, as 16 lowercase hex digits.
pub fn fingerprint(text: &str) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for b in text.bytes() {
        hash ^= u64::from(b);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}")
}

// A marker matches only as a whole token so `hero_1` never matches `hero_10`.
fn has_marker(line: &str, marker: &str) -> bool {
    line.find(marker).is_some_and(|at| {
        !line[at + marker.len()..]
            .starts_with(|c: char| c.is_alphanumeric() || c == '_' || c == '-')
    })
}

/// Content strictly between the `protopie:begin <id>` and `protopie:end <id>`
/// marker lines, or `None` if either marker is missing or out of order.
pub fn extract_region(text: &str, id: &str) -> Option<String> {
    let begin = format!("protopie:begin {id}");
    let end = format!("protopie:end {id}");
    let mut lines = text.split_inclusive('\n');
    let mut body = String::new();
    lines.find(|l| has_marker(l, &begin))?;
    for line in lines {
        if has_marker(line, &end) {
            return Some(body);
        }
        body.push_str(line);
    }
    None
}

/// Returns `text` with the content between the region's marker lines replaced
/// by `body` (which should end in a newline unless empty). `None` if the
/// markers are missing or out of order.
pub fn replace_region(text: &str, id: &str, body: &str) -> Option<String> {
    let begin = format!("protopie:begin {id}");
    let end = format!("protopie:end {id}");
    let lines: Vec<&str> = text.split_inclusive('\n').collect();
    let b = lines.iter().position(|l| has_marker(l, &begin))?;
    let e = b + 1 + lines[b + 1..].iter().position(|l| has_marker(l, &end))?;
    let mut out: String = lines[..=b].concat();
    out.push_str(body);
    out.push_str(&lines[e..].concat());
    Some(out)
}

/// Relative path of a conversation's session file (validated slug).
pub(crate) fn session_rel(conversation_id: &str) -> Result<String> {
    if !is_valid_slug(conversation_id) {
        return Err(Error::InvalidConversationId(conversation_id.to_string()));
    }
    Ok(format!(
        "{PROTOPIE_DIR}/{SESSIONS_DIR}/{conversation_id}.json"
    ))
}

/// Relative path of the model manifest.
pub(crate) fn model_rel() -> String {
    format!("{PROTOPIE_DIR}/{MODEL_FILE}")
}

fn read_text(dir: &Path, relative: &str) -> Result<Option<String>> {
    let path = crate::resolve_in_project(dir, relative)?;
    match fs::read_to_string(&path) {
        Ok(text) => Ok(Some(text)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(Error::Io { path, source: e }),
    }
}

fn owned_fingerprint(dir: &Path, path: &str, region: Option<&str>) -> Result<Option<String>> {
    Ok(read_text(dir, path)?.and_then(|text| match region {
        None => Some(fingerprint(&text)),
        Some(id) => extract_region(&text, id).map(|r| fingerprint(&r)),
    }))
}

/// Builds the model of a template-derived project by reading it. Pure with
/// respect to the project: nothing is written.
pub fn seed_snapshot(dir: &Path) -> Result<ProjectSnapshot> {
    if !dir.is_dir() {
        return Err(Error::NotADirectory(dir.to_path_buf()));
    }
    let mut owned = Vec::new();
    let mut missing = Vec::new();
    for (path, region) in TEMPLATE_REGIONS {
        match owned_fingerprint(dir, path, Some(region))? {
            Some(fingerprint) => owned.push(OwnedSource {
                path: (*path).into(),
                region: Some((*region).into()),
                fingerprint,
            }),
            None => missing.push(format!("{path} (region {region})")),
        }
    }
    if !missing.is_empty() {
        return Err(Error::UnsupportedProject(format!(
            "missing protopie regions: {}. Create the project from the reference template.",
            missing.join(", ")
        )));
    }
    let mut hero = ElementRecord::new(ElementId::new("hero_1"), ElementKind::Hero, DEFAULT_PAGE);
    hero.source = Some(SourceBinding {
        file: "src/pages/Home.tsx".into(),
        region: Some("hero_1".into()),
    });
    // The planner's view of the hero's cascade is what its stylesheet declares.
    let hero_css = read_text(dir, "src/pages/Home.module.css")?
        .and_then(|text| extract_region(&text, "hero_1"))
        .unwrap_or_default();
    hero.style = Some(StyleBinding {
        file: "src/pages/Home.module.css".into(),
        class: "hero".into(),
        region: Some("hero_1".into()),
        scope: StyleScope::Instance,
        values: crate::emit::css_declared_values(&hero_css, ".hero"),
        instance_override: false,
    });
    // `.hero` is a centered flex column.
    hero.layout = Some(ContainerLayout::Vertical);
    let mut snapshot = ProjectSnapshot::empty();
    snapshot.elements.push(hero);
    snapshot.owned = owned;
    snapshot.id_counters.insert("hero".into(), 1);
    // Home renders its elements one after another in normal block flow.
    snapshot
        .page_layouts
        .insert(DEFAULT_PAGE.into(), ContainerLayout::Vertical);
    Ok(snapshot)
}

fn metadata_path(dir: &Path, relative: &str) -> Result<PathBuf> {
    crate::resolve_in_project(dir, &format!("{PROTOPIE_DIR}/{relative}"))
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(io_err(parent))?;
    }
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, bytes).map_err(io_err(&tmp))?;
    fs::rename(&tmp, path).map_err(io_err(path))
}

/// Writes the model atomically (`.protopie/model.json`).
pub fn save_project(dir: &Path, snapshot: &ProjectSnapshot) -> Result<()> {
    let path = metadata_path(dir, MODEL_FILE)?;
    let mut json = serde_json::to_vec_pretty(snapshot).map_err(|e| Error::Metadata {
        path: path.clone(),
        message: e.to_string(),
    })?;
    json.push(b'\n');
    write_atomic(&path, &json)
}

/// Loads the model. A missing file is [`ProjectState::NoMetadata`]; unreadable,
/// invalid or newer-schema metadata is an error (never silently trusted).
pub fn load_project(dir: &Path) -> Result<ProjectState> {
    if !dir.is_dir() {
        return Err(Error::NotADirectory(dir.to_path_buf()));
    }
    let path = metadata_path(dir, MODEL_FILE)?;
    let text = match fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(ProjectState::NoMetadata),
        Err(e) => return Err(Error::Io { path, source: e }),
    };
    let snapshot: ProjectSnapshot = serde_json::from_str(&text).map_err(|e| Error::Metadata {
        path: path.clone(),
        message: e.to_string(),
    })?;
    if snapshot.schema_version != MODEL_SCHEMA_VERSION {
        return Err(Error::Metadata {
            path,
            message: format!(
                "unsupported model schema version {} (expected {MODEL_SCHEMA_VERSION})",
                snapshot.schema_version
            ),
        });
    }
    Ok(ProjectState::Loaded(snapshot))
}

/// Explicit initialization / migration: seeds and saves a model for a project
/// that has none. Existing models are never overwritten.
pub fn initialize_project(dir: &Path) -> Result<ProjectSnapshot> {
    if let ProjectState::Loaded(_) = load_project(dir)? {
        return Err(Error::AlreadyInitialized(dir.to_path_buf()));
    }
    let snapshot = seed_snapshot(dir)?;
    save_project(dir, &snapshot)?;
    Ok(snapshot)
}

/// Compares every owned source with its recorded fingerprint.
pub fn verify_owned(dir: &Path, snapshot: &ProjectSnapshot) -> Result<Vec<Drift>> {
    let mut drift = Vec::new();
    for owned in &snapshot.owned {
        let kind = match owned_fingerprint(dir, &owned.path, owned.region.as_deref())? {
            None => Some(DriftKind::Missing),
            Some(fp) if fp != owned.fingerprint => Some(DriftKind::Modified),
            Some(_) => None,
        };
        if let Some(kind) = kind {
            drift.push(Drift {
                path: owned.path.clone(),
                region: owned.region.clone(),
                kind,
            });
        }
    }
    Ok(drift)
}

/// Fails with [`Error::StaleRevision`] unless the snapshot is at `expected`.
pub fn check_revision(snapshot: &ProjectSnapshot, expected: u64) -> Result<()> {
    if snapshot.revision == expected {
        Ok(())
    } else {
        Err(Error::StaleRevision {
            expected,
            actual: snapshot.revision,
        })
    }
}

fn session_path(dir: &Path, conversation_id: &str) -> Result<PathBuf> {
    if !is_valid_slug(conversation_id) {
        return Err(Error::InvalidConversationId(conversation_id.to_string()));
    }
    metadata_path(dir, &format!("{SESSIONS_DIR}/{conversation_id}.json"))
}

/// Loads a conversation, or a fresh [`Session`] if it has never been saved.
pub fn load_session(dir: &Path, conversation_id: &str) -> Result<Session> {
    let path = session_path(dir, conversation_id)?;
    let text = match fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Session::new(conversation_id)),
        Err(e) => return Err(Error::Io { path, source: e }),
    };
    let session: Session = serde_json::from_str(&text).map_err(|e| Error::Metadata {
        path: path.clone(),
        message: e.to_string(),
    })?;
    if session.schema_version != SESSION_SCHEMA_VERSION
        || session.conversation_id != conversation_id
    {
        return Err(Error::Metadata {
            path,
            message: "session schema version or conversation id mismatch".into(),
        });
    }
    Ok(session)
}

/// Saves a conversation atomically, scoped to this project directory.
pub fn save_session(dir: &Path, session: &Session) -> Result<()> {
    let path = session_path(dir, &session.conversation_id)?;
    let mut json = serde_json::to_vec_pretty(session).map_err(|e| Error::Metadata {
        path: path.clone(),
        message: e.to_string(),
    })?;
    json.push(b'\n');
    write_atomic(&path, &json)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline::plan_prompt;

    fn fresh() -> (tempfile::TempDir, PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let project = crate::init_named(tmp.path(), "p").unwrap();
        (tmp, project)
    }

    fn loaded(dir: &Path) -> ProjectSnapshot {
        match load_project(dir).unwrap() {
            ProjectState::Loaded(s) => s,
            ProjectState::NoMetadata => panic!("no metadata"),
        }
    }

    fn nav(id: &str, placement: Option<Placement>) -> ElementRecord {
        let mut e = ElementRecord::new(ElementId::new(id), ElementKind::Navigation, DEFAULT_PAGE);
        e.placement = placement;
        e
    }

    #[test]
    fn regions_extract_between_markers() {
        let text = "a\n/* protopie:begin x */\none\ntwo\n/* protopie:end x */\nb\n";
        assert_eq!(extract_region(text, "x").as_deref(), Some("one\ntwo\n"));
        assert_eq!(extract_region(text, "y"), None);
        // Whole-token match: `x` is not `x1`.
        assert_eq!(
            extract_region("// protopie:begin x1\n// protopie:end x1\n", "x"),
            None
        );
        assert_eq!(
            extract_region("// protopie:end x\n// protopie:begin x\n", "x"),
            None
        );
        assert_eq!(extract_region("// protopie:begin x\nno end\n", "x"), None);
        assert_eq!(fingerprint("a"), fingerprint("a"));
        assert_ne!(fingerprint("a"), fingerprint("b"));
    }

    #[test]
    fn fresh_project_has_seeded_model_that_round_trips() {
        let (_tmp, dir) = fresh();
        let snapshot = loaded(&dir);
        assert_eq!(snapshot.revision, 0);
        let hero = snapshot.element(&ElementId::new("hero_1")).unwrap();
        assert_eq!(hero.kind, ElementKind::Hero);
        assert_eq!(hero.source.as_ref().unwrap().file, "src/pages/Home.tsx");
        let style = hero.style.as_ref().unwrap();
        assert_eq!(
            (style.class.as_str(), style.scope),
            ("hero", StyleScope::Instance)
        );
        assert_eq!(snapshot.owned.len(), 7);
        assert_eq!(style.values.get("padding").map(String::as_str), Some("2rem 1rem"));
        assert_eq!(hero.layout, Some(ContainerLayout::Vertical));
        assert_eq!(snapshot.next_id("hero"), ElementId::new("hero_2"));
        // Round trip: save + load yields the same value, and a fresh seed agrees.
        save_project(&dir, &snapshot).unwrap();
        assert_eq!(loaded(&dir), snapshot);
        assert_eq!(seed_snapshot(&dir).unwrap(), snapshot);
        assert!(verify_owned(&dir, &snapshot).unwrap().is_empty());
    }

    #[test]
    fn missing_metadata_is_explicit_and_initialization_is_supported() {
        let (_tmp, dir) = fresh();
        fs::remove_dir_all(dir.join(PROTOPIE_DIR)).unwrap();
        assert_eq!(load_project(&dir).unwrap(), ProjectState::NoMetadata);
        let result = crate::modify(&dir, "Add top nav").unwrap();
        let ModifyOutcome::Unsupported { rejection } = &result.outcome else {
            panic!("{:?}", result.outcome)
        };
        assert_eq!(rejection.reason, RejectionReason::ProjectNotInitialized);
        assert!(result.summary.contains("initialize"));
        initialize_project(&dir).unwrap();
        assert!(matches!(
            load_project(&dir).unwrap(),
            ProjectState::Loaded(_)
        ));
        assert!(matches!(
            initialize_project(&dir),
            Err(Error::AlreadyInitialized(_))
        ));
    }

    #[test]
    fn handwritten_projects_are_unsupported_with_actionable_error() {
        let tmp = tempfile::tempdir().unwrap();
        let err = initialize_project(tmp.path()).unwrap_err();
        let Error::UnsupportedProject(message) = err else {
            panic!("{err:?}")
        };
        assert!(message.contains("src/App.tsx") && message.contains("layout-top"));
        assert!(!tmp.path().join(PROTOPIE_DIR).exists());
    }

    #[test]
    fn corrupt_or_newer_metadata_is_an_error_not_a_default() {
        let (_tmp, dir) = fresh();
        let model = dir.join(PROTOPIE_DIR).join(MODEL_FILE);
        fs::write(&model, "{not json").unwrap();
        assert!(matches!(load_project(&dir), Err(Error::Metadata { .. })));
        let mut newer = ProjectSnapshot::empty();
        newer.schema_version = MODEL_SCHEMA_VERSION + 1;
        fs::write(&model, serde_json::to_string(&newer).unwrap()).unwrap();
        assert!(matches!(load_project(&dir), Err(Error::Metadata { .. })));
    }

    #[test]
    fn external_edits_inside_owned_regions_are_detected() {
        let (_tmp, dir) = fresh();
        let snapshot = loaded(&dir);
        let home = dir.join("src/pages/Home.tsx");
        let original = fs::read_to_string(&home).unwrap();
        // Edit outside the region: not drift.
        fs::write(&home, format!("// note\n{original}")).unwrap();
        assert!(verify_owned(&dir, &snapshot).unwrap().is_empty());
        // Edit inside the region: drift.
        fs::write(&home, original.replace("Hello World", "Howdy")).unwrap();
        assert_eq!(
            verify_owned(&dir, &snapshot).unwrap(),
            vec![Drift {
                path: "src/pages/Home.tsx".into(),
                region: Some("hero_1".into()),
                kind: DriftKind::Modified
            }]
        );
        // Markers removed: missing.
        fs::write(&home, "export default () => null;\n").unwrap();
        assert_eq!(
            verify_owned(&dir, &snapshot).unwrap()[0].kind,
            DriftKind::Missing
        );
        fs::remove_file(&home).unwrap();
        assert_eq!(
            verify_owned(&dir, &snapshot).unwrap()[0].kind,
            DriftKind::Missing
        );
    }

    #[test]
    fn revision_check_and_ids_are_never_reused() {
        let mut snapshot = ProjectSnapshot::empty();
        snapshot.revision = 3;
        assert!(check_revision(&snapshot, 3).is_ok());
        assert!(matches!(
            check_revision(&snapshot, 2),
            Err(Error::StaleRevision {
                expected: 2,
                actual: 3
            })
        ));
        assert_eq!(snapshot.reserve_id("nav"), ElementId::new("nav_1"));
        // Even with no element left, nav_1 is not issued again.
        assert_eq!(snapshot.next_id("nav"), ElementId::new("nav_2"));
        snapshot.elements.push(nav("nav_5", None));
        assert_eq!(snapshot.next_id("nav"), ElementId::new("nav_6"));
    }

    #[test]
    fn sessions_are_scoped_bounded_and_round_trip() {
        let (_tmp, dir) = fresh();
        assert_eq!(load_session(&dir, "default").unwrap(), Session::default());
        let mut a = Session::new("a");
        a.advance_turn();
        a.record_success(ElementId::new("nav_1"), ElementKind::Navigation);
        save_session(&dir, &a).unwrap();
        save_session(&dir, &Session::new("b")).unwrap();
        assert_eq!(load_session(&dir, "a").unwrap(), a);
        assert_eq!(load_session(&dir, "b").unwrap().focus, None);
        for bad in ["", "../x", "A", "a/b"] {
            assert!(
                matches!(
                    load_session(&dir, bad),
                    Err(Error::InvalidConversationId(_))
                ),
                "{bad:?}"
            );
        }
        // Bounds are turn-based and deterministic.
        let mut s = Session::default();
        for i in 0..40 {
            s.advance_turn();
            s.record_success(
                ElementId::new(format!("item_{i}")),
                ElementKind::NavigationItem,
            );
        }
        assert_eq!(s.recent.len(), MAX_RECENT_EDITS);
        assert_eq!(s.recent[0].turn, 40 - MAX_RECENT_EDITS as u64 + 1);
        let question = |id: &str| Question {
            id: id.into(),
            kind: QuestionKind::FollowUp,
            target: None,
            prompt: "?".into(),
            options: vec![],
            continuation: Continuation::Rephrase {
                suggestion: "x".into(),
            },
        };
        for i in 0..20 {
            s.add_pending(question(&format!("q{i}")));
        }
        s.add_pending(question("q19"));
        assert_eq!(s.pending_questions.len(), MAX_PENDING_QUESTIONS);
        assert!(s.take_pending("q0").is_none());
        assert!(s.take_pending("q19").is_some());
        assert_eq!(s.pending_questions.len(), MAX_PENDING_QUESTIONS - 1);
    }

    /// Simulates applying a plan (T4 owns real application): adds the planned
    /// element to the model, bumps the revision and records success.
    fn simulate_apply_nav(
        dir: &Path,
        id: &str,
        placement: Option<Placement>,
        session: &mut Session,
    ) {
        let mut snapshot = loaded(dir);
        let reserved = snapshot.reserve_id("nav");
        assert_eq!(reserved.0, id);
        snapshot.elements.push(nav(id, placement));
        snapshot.revision += 1;
        save_project(dir, &snapshot).unwrap();
        session.advance_turn();
        session.record_success(ElementId::new(id), ElementKind::Navigation);
        save_session(dir, session).unwrap();
    }

    fn added_to(outcome: PlanOutcome) -> ElementId {
        let PlanOutcome::Ready { plan } = outcome else {
            panic!("{outcome:?}")
        };
        match &plan.operations[0] {
            Operation::AddNavigationItem { navigation, .. } => navigation.clone(),
            op => panic!("{op:?}"),
        }
    }

    #[test]
    fn navigation_conversation_resolves_across_reload() {
        let (_tmp, dir) = fresh();
        let mut session = load_session(&dir, DEFAULT_CONVERSATION_ID).unwrap();

        // Turn 1: "Need a top navigation" plans creation, previews change nothing.
        let outcome = plan_prompt("Need a top navigation", &loaded(&dir), &session);
        assert!(matches!(outcome, PlanOutcome::Ready { .. }));
        assert_eq!(
            session,
            load_session(&dir, DEFAULT_CONVERSATION_ID).unwrap()
        );
        simulate_apply_nav(&dir, "nav_1", Some(Placement::Top), &mut session);
        // A second, side navigation exists but was not the focus-establishing one.
        let mut snapshot = loaded(&dir);
        snapshot.reserve_id("nav");
        snapshot.elements.push(nav("nav_2", Some(Placement::Side)));
        save_project(&dir, &snapshot).unwrap();

        // "Reload": everything comes back from disk.
        let (snapshot, session) = (
            loaded(&dir),
            load_session(&dir, DEFAULT_CONVERSATION_ID).unwrap(),
        );
        assert_eq!(snapshot.elements.len(), 3);
        assert_eq!(
            added_to(plan_prompt("Add Contact Us", &snapshot, &session)),
            ElementId::new("nav_1")
        );
    }

    #[test]
    fn explicit_references_override_focus_and_focus_beats_uniqueness() {
        let (_tmp, dir) = fresh();
        let mut session = Session::default();
        simulate_apply_nav(&dir, "nav_1", Some(Placement::Side), &mut session);
        let mut snapshot = loaded(&dir);
        snapshot.reserve_id("nav");
        snapshot.elements.push(nav("nav_2", Some(Placement::Top)));
        save_project(&dir, &snapshot).unwrap();
        let snapshot = loaded(&dir);
        // Focus is the side nav: a bare add follows focus...
        assert_eq!(
            added_to(plan_prompt("Add Contact Us", &snapshot, &session)),
            ElementId::new("nav_1")
        );
        // ...but an explicit `top nav` binds to the top navigation instead.
        assert_eq!(
            added_to(plan_prompt(
                "Add Contact Us to top nav",
                &snapshot,
                &session
            )),
            ElementId::new("nav_2")
        );
    }
}
