//! Solid context extension (Epic 001, T10), end to end against copies of the
//! reference template.
//!
//! The agent must not invent the state shape, the initial value, the provider
//! scope or the consumers: each missing one is a persisted blocking question,
//! and nothing is written until the context is complete.

use std::fs;
use std::path::{Path, PathBuf};

use protopie_ui_agent::contracts::*;
use protopie_ui_agent::project::{self, extract_region, load_project, load_session};
use protopie_ui_agent::{init_named, modify, modify_with, ModifyOptions};

fn fresh() -> (tempfile::TempDir, PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    let dir = init_named(tmp.path(), "p").unwrap();
    (tmp, dir)
}

fn model(dir: &Path) -> ProjectSnapshot {
    match load_project(dir).unwrap() {
        project::ProjectState::Loaded(m) => m,
        project::ProjectState::NoMetadata => panic!("no model"),
    }
}

fn read(dir: &Path, rel: &str) -> String {
    fs::read_to_string(dir.join(rel)).unwrap_or_else(|e| panic!("{rel}: {e}"))
}

fn kind(result: &ModifyResult) -> &'static str {
    match &result.outcome {
        ModifyOutcome::Preview { .. } => "preview",
        ModifyOutcome::Applied { .. } => "applied",
        ModifyOutcome::NeedsClarification { .. } => "needs_clarification",
        ModifyOutcome::NoChange { .. } => "no_change",
        ModifyOutcome::Unsupported { .. } => "unsupported",
        ModifyOutcome::Conflict { .. } => "conflict",
    }
}

fn expect(dir: &Path, text: &str, want: &str) -> ModifyResult {
    let result = modify(dir, text).unwrap();
    assert_eq!(kind(&result), want, "{text}: {result:?}");
    result
}

fn question(result: &ModifyResult) -> Question {
    match &result.outcome {
        ModifyOutcome::NeedsClarification { questions } => {
            assert_eq!(questions.len(), 1, "{result:?}");
            questions[0].clone()
        }
        other => panic!("{other:?}"),
    }
}

/// The question's field, asserting it is a context question.
fn field(q: &Question) -> ContextField {
    match &q.continuation {
        Continuation::ProvideContext { field, .. } => *field,
        other => panic!("{other:?}"),
    }
}

fn clean(dir: &Path) {
    assert!(
        project::verify_owned(dir, &model(dir)).unwrap().is_empty(),
        "owned source drifted"
    );
}

/// Every file of the project, for "nothing was written" checks.
fn snapshot_of_files(dir: &Path) -> Vec<(String, String)> {
    fn walk(root: &Path, dir: &Path, out: &mut Vec<(String, String)>) {
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(root, &path, out);
            } else if let Ok(text) = fs::read_to_string(&path) {
                let rel = path.strip_prefix(root).unwrap().display().to_string();
                if !rel.starts_with(".protopie/sessions") {
                    out.push((rel, text));
                }
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, dir, &mut out);
    out.sort();
    out
}

/// The agreed fixture: a project with a Doctors page and a Visits page.
fn with_pages() -> (tempfile::TempDir, PathBuf) {
    let (tmp, dir) = fresh();
    expect(&dir, "Add a Doctors page", "applied");
    expect(&dir, "Add a Visits page", "applied");
    (tmp, dir)
}

#[test]
fn sharing_without_a_page_is_rejected_not_invented() {
    let (_tmp, dir) = fresh();
    let before = snapshot_of_files(&dir);
    let result = expect(&dir, "Share the selected doctor across pages", "unsupported");
    assert!(
        matches!(&result.outcome, ModifyOutcome::Unsupported { rejection }
            if rejection.explanation.contains("Create a page first")),
        "{result:?}"
    );
    assert_eq!(snapshot_of_files(&dir), before);
}

#[test]
fn the_context_conversation_asks_for_every_requirement_then_generates() {
    let (_tmp, dir) = with_pages();

    // Scope is in the prompt; the shape is the first question.
    let q = question(&expect(
        &dir,
        "Share the selected doctor across pages",
        "needs_clarification",
    ));
    assert_eq!(field(&q), ContextField::Shape);
    assert_eq!(
        q.options.iter().map(|o| o.key.as_str()).collect::<Vec<_>>(),
        ["text", "number", "flag", "optional_text"]
    );
    // Nothing is generated while questions are open.
    assert!(model(&dir).contexts.is_empty());
    assert!(!dir.join("src/context").exists());

    let q = question(&expect(&dir, "text", "needs_clarification"));
    assert_eq!(field(&q), ContextField::Initial);
    assert!(q.continuation.takes_text());
    assert!(!dir.join("src/context").exists());

    // The initial value is the user's own answer.
    let q = question(&expect(&dir, "Dr. Rao", "needs_clarification"));
    assert_eq!(field(&q), ContextField::Consumers);
    assert_eq!(
        q.options.iter().map(|o| o.key.as_str()).collect::<Vec<_>>(),
        ["doctors", "visits"]
    );
    assert!(!dir.join("src/context").exists());

    // First consumer, then the choice to add another, then done.
    let q = question(&expect(&dir, "doctors", "needs_clarification"));
    assert_eq!(field(&q), ContextField::Consumers);
    assert_eq!(
        q.options.iter().map(|o| o.key.as_str()).collect::<Vec<_>>(),
        ["visits", "done"]
    );
    let done = expect(&dir, "no, that is all", "applied");
    assert!(done.summary.contains("Shared"), "{}", done.summary);

    let m = model(&dir);
    assert_eq!(m.contexts.len(), 1);
    let c = &m.contexts[0];
    assert_eq!(
        (c.id.as_str(), c.name.as_str(), c.label.as_str()),
        ("selected_doctor", "SelectedDoctor", "selected doctor")
    );
    assert_eq!(c.shape, StateShape::Text);
    assert_eq!(c.initial, InitialValue::Text { value: "Dr. Rao".into() });
    assert_eq!(c.scope, ContextScope::AllPages);
    assert_eq!(c.consumers, ["doctors"]);

    // The provider and its context.
    let ctx = read(&dir, "src/context/SelectedDoctor.tsx");
    assert!(ctx.contains("export const SelectedDoctorContext = createContext<Signal<string>>();"), "{ctx}");
    assert!(ctx.contains("export function SelectedDoctorProvider"), "{ctx}");
    assert!(ctx.contains("createSignal<string>(\"Dr. Rao\")"), "{ctx}");
    // The provider wraps the layout, in the owned region of App.tsx.
    let app = read(&dir, "src/App.tsx");
    let providers = extract_region(&app, "providers").unwrap();
    assert!(providers.contains("<SelectedDoctorProvider>"), "{providers}");
    assert!(providers.contains("{props.children}"), "{providers}");
    assert!(app.contains("<Providers>"), "{app}");
    // Only the chosen page reads it.
    let doctors = read(&dir, "src/pages/DoctorsPage.tsx");
    assert!(doctors.contains("useContext(SelectedDoctorContext)"), "{doctors}");
    assert!(doctors.contains("selectedDoctorValue()"), "{doctors}");
    assert!(doctors.contains("data-protopie-context=\"selected_doctor\""), "{doctors}");
    let visits = read(&dir, "src/pages/VisitsPage.tsx");
    assert!(!visits.contains("useContext"), "{visits}");
    clean(&dir);

    // Asking again changes nothing.
    let before = snapshot_of_files(&dir);
    expect(&dir, "Share the selected doctor across pages", "no_change");
    assert_eq!(snapshot_of_files(&dir), before);
    assert!(load_session(&dir, DEFAULT_CONVERSATION_ID)
        .unwrap()
        .pending_questions
        .is_empty());
}

#[test]
fn an_omitted_scope_is_asked_and_other_shapes_have_their_own_initial_rules() {
    let (_tmp, dir) = with_pages();
    let q = question(&expect(&dir, "Share the visit count", "needs_clarification"));
    assert_eq!(field(&q), ContextField::Scope);
    assert_eq!(q.options.len(), 1);
    // An unknown scope is not guessed: a targeted answer asks again, and a
    // bare unrecognized phrase is not an answer at all.
    let again = question(&expect(
        &dir,
        &format!("{}: just this page", q.id),
        "needs_clarification",
    ));
    assert_eq!(field(&again), ContextField::Scope);
    expect(&dir, "just this page", "unsupported");
    expect(&dir, "across all pages", "needs_clarification");
    let q = question(&expect(&dir, "a number", "needs_clarification"));
    assert_eq!(field(&q), ContextField::Initial);
    // A bad number is re-asked, not coerced.
    let again = question(&expect(&dir, "lots", "needs_clarification"));
    assert_eq!(field(&again), ContextField::Initial);
    expect(&dir, "007", "needs_clarification");
    // Both pages: the first chosen, the other offered, then bound by choice.
    expect(&dir, "visits", "needs_clarification");
    expect(&dir, "doctors", "applied");
    let m = model(&dir);
    let c = &m.contexts[0];
    assert_eq!(c.initial, InitialValue::Number { value: "7".into() });
    assert_eq!(c.consumers, ["visits", "doctors"]);
    let ctx = read(&dir, "src/context/VisitCount.tsx");
    assert!(ctx.contains("createSignal<number>(7)"), "{ctx}");
    assert!(ctx.contains("Signal<number>"), "{ctx}");
    for page in ["DoctorsPage", "VisitsPage"] {
        let text = read(&dir, &format!("src/pages/{page}.tsx"));
        assert!(text.contains("visitCountValue()"), "{text}");
    }
    clean(&dir);
}

#[test]
fn flag_and_optional_text_shapes_generate_typed_state() {
    let (_tmp, dir) = with_pages();
    expect(&dir, "Share the \"Is Open\" across pages", "needs_clarification");
    expect(&dir, "yes or no", "needs_clarification");
    let again = question(&expect(&dir, "maybe", "needs_clarification"));
    assert_eq!(field(&again), ContextField::Initial);
    expect(&dir, "no", "needs_clarification");
    expect(&dir, "doctors", "needs_clarification");
    expect(&dir, "done", "applied");
    // Optional text needs no initial question: it starts unset by its shape.
    expect(&dir, "Share the selected doctor across pages", "needs_clarification");
    let q = question(&expect(&dir, "optional text", "needs_clarification"));
    assert_eq!(field(&q), ContextField::Consumers);
    expect(&dir, "visits", "needs_clarification");
    expect(&dir, "done", "applied");

    let m = model(&dir);
    assert_eq!(m.contexts[0].initial, InitialValue::Flag { value: false });
    assert_eq!(m.contexts[1].initial, InitialValue::Unset);
    let flag = read(&dir, "src/context/IsOpen.tsx");
    assert!(flag.contains("createSignal<boolean>(false)"), "{flag}");
    let optional = read(&dir, "src/context/SelectedDoctor.tsx");
    assert!(optional.contains("createSignal<string | undefined>()"), "{optional}");
    assert!(optional.contains("Signal<string | undefined>"), "{optional}");
    let doctors = read(&dir, "src/pages/DoctorsPage.tsx");
    assert!(doctors.contains("isOpenValue() ? \"Yes\" : \"No\""), "{doctors}");
    let visits = read(&dir, "src/pages/VisitsPage.tsx");
    assert!(visits.contains("selectedDoctorValue() ?? \"none\""), "{visits}");
    // Providers nest in creation order, outermost first.
    let providers = extract_region(&read(&dir, "src/App.tsx"), "providers").unwrap();
    let outer = providers.find("<IsOpenProvider>").unwrap();
    let inner = providers.find("<SelectedDoctorProvider>").unwrap();
    assert!(outer < inner, "{providers}");
    clean(&dir);
}

#[test]
fn previews_and_unanswered_questions_never_write_source() {
    let (_tmp, dir) = with_pages();
    let before = snapshot_of_files(&dir);
    let dry = ModifyOptions {
        dry_run: true,
        ..Default::default()
    };
    // A dry run of the opening request asks but leaves the conversation alone.
    let r = modify_with(&dir, "Share the selected doctor across pages", &dry).unwrap();
    assert_eq!(kind(&r), "needs_clarification", "{r:?}");
    assert!(load_session(&dir, DEFAULT_CONVERSATION_ID)
        .unwrap()
        .pending_questions
        .is_empty());
    // Open questions leave source untouched.
    expect(&dir, "Share the selected doctor across pages", "needs_clarification");
    expect(&dir, "text", "needs_clarification");
    expect(&dir, "Dr. Rao", "needs_clarification");
    assert_eq!(
        snapshot_of_files(&dir)
            .into_iter()
            .filter(|(p, _)| p != ".protopie/model.json" && !p.starts_with(".protopie/"))
            .collect::<Vec<_>>(),
        before
            .into_iter()
            .filter(|(p, _)| p != ".protopie/model.json" && !p.starts_with(".protopie/"))
            .collect::<Vec<_>>()
    );
    // The final step previews as a diff of exactly the files it generates.
    let r = modify_with(&dir, "doctors", &dry).unwrap();
    assert_eq!(kind(&r), "needs_clarification", "{r:?}");
    let r = modify(&dir, "doctors").unwrap();
    assert_eq!(kind(&r), "needs_clarification", "{r:?}");
    let r = modify_with(&dir, "done", &dry).unwrap();
    match &r.outcome {
        ModifyOutcome::Preview { changed_files, .. } => {
            let mut files = changed_files.clone();
            files.sort();
            assert_eq!(
                files,
                [
                    "src/App.tsx",
                    "src/context/SelectedDoctor.tsx",
                    "src/pages/DoctorsPage.tsx"
                ]
            );
        }
        other => panic!("{other:?}"),
    }
    assert!(model(&dir).contexts.is_empty());
}

#[test]
fn unsupported_labels_and_unsupported_scopes_are_rejected_with_nothing_written() {
    let (_tmp, dir) = with_pages();
    let before = snapshot_of_files(&dir);
    for prompt in [
        "Share the \"3 doctors\" across pages",
        "Share the \"!!!\" across pages",
        "Share the doctor across the app",
        "Share doctor across pages",
        "Share the doctor across pages.",
    ] {
        let r = modify(&dir, prompt).unwrap();
        assert_eq!(kind(&r), "unsupported", "{prompt}: {r:?}");
    }
    assert_eq!(snapshot_of_files(&dir), before);
}

#[test]
fn stale_answers_conflict_and_a_project_without_the_providers_region_is_not_edited() {
    let (_tmp, dir) = with_pages();
    expect(&dir, "Share the selected doctor across pages", "needs_clarification");
    expect(&dir, "text", "needs_clarification");
    expect(&dir, "Dr. Rao", "needs_clarification");
    expect(&dir, "doctors", "needs_clarification");

    // An older project predates the `providers` region: apply reports why and
    // writes nothing. (Owned drift would report first, so the region is
    // removed through the model as an upgrade would not: strip it and refresh
    // the recorded fingerprints.)
    let app_path = dir.join("src/App.tsx");
    let app = fs::read_to_string(&app_path).unwrap();
    let stripped = app
        .replace("// protopie:begin providers\n", "")
        .replace("// protopie:end providers\n", "");
    fs::write(&app_path, &stripped).unwrap();
    let mut m = model(&dir);
    m.owned
        .retain(|o| !(o.path == "src/App.tsx" && o.region.as_deref() == Some("providers")));
    project::save_project(&dir, &m).unwrap();
    let before = snapshot_of_files(&dir);
    let r = modify(&dir, "done").unwrap();
    assert_eq!(kind(&r), "conflict", "{r:?}");
    assert!(r.summary.contains("providers"), "{}", r.summary);
    assert_eq!(snapshot_of_files(&dir), before);
    assert!(model(&dir).contexts.is_empty());
}
