//! Styling and relational selectors (Epic 001, T7), end to end against copies of
//! the reference template.
//!
//! The template only ships a hero, so each test seeds the model with a fixture
//! page the way the T8 generators will: a wholly owned stylesheet
//! (`src/components/Media.module.css`) plus model elements bound to its rules.
//!
//! Page order (siblings in a known vertical container, the home page):
//! `hero_1, form_1, img_1, img_2, img_3, form_2`.
//!
//! * `img_1` and `img_2` are instances of one reusable component and share the
//!   `.photo` class (`Definition` scope). `img_3` owns `.portrait`.
//! * `form_1` (`.signup`) is the first form below the hero; `form_2`
//!   (`.contact`) comes later.

use std::fs;
use std::path::{Path, PathBuf};

use protopie_ui_agent::apply::{apply_plan, ApplyOptions, ApplyOutcome};
use protopie_ui_agent::contracts::*;
use protopie_ui_agent::pipeline::plan_prompt;
use protopie_ui_agent::project::{self, fingerprint, load_project, load_session, save_session};
use protopie_ui_agent::{init_named, modify, modify_with, ModifyOptions};

const MEDIA_CSS: &str = "src/components/Media.module.css";
const CSS: &str = "\
.photo {
  display: block;
  max-width: 100%;
}

.portrait {
  display: block;
}

.signup {
  padding: 2rem 1rem;
  background: #fff;
}

.contact {
  padding: var(--space-8);
}
";

fn binding(class: &str, scope: StyleScope, values: &[(&str, &str)]) -> StyleBinding {
    StyleBinding {
        file: MEDIA_CSS.into(),
        class: class.into(),
        region: None,
        scope,
        values: values
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect(),
        instance_override: false,
    }
}

fn element(id: &str, kind: ElementKind, style: Option<StyleBinding>) -> ElementRecord {
    let mut e = ElementRecord::new(ElementId::new(id), kind, DEFAULT_PAGE);
    e.style = style;
    e
}

/// A fresh project whose model holds the fixture page described above.
fn styled_project() -> (tempfile::TempDir, PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    let dir = init_named(tmp.path(), "p").unwrap();
    fs::create_dir_all(dir.join("src/components")).unwrap();
    fs::write(dir.join(MEDIA_CSS), CSS).unwrap();
    let ProjectState::Loaded(mut model) = load_project(&dir).unwrap() else {
        panic!("no model")
    };
    model.owned.push(OwnedSource {
        path: MEDIA_CSS.into(),
        region: None,
        fingerprint: fingerprint(CSS),
    });
    let photo = || binding("photo", StyleScope::Definition, &[]);
    model.elements.extend([
        element(
            "form_1",
            ElementKind::Form,
            Some(binding(
                "signup",
                StyleScope::Instance,
                &[("padding", "2rem 1rem")],
            )),
        ),
        element("img_1", ElementKind::Image, Some(photo())),
        element("img_2", ElementKind::Image, Some(photo())),
        element(
            "img_3",
            ElementKind::Image,
            Some(binding("portrait", StyleScope::Instance, &[])),
        ),
        element(
            "form_2",
            ElementKind::Form,
            Some(binding(
                "contact",
                StyleScope::Instance,
                &[("padding", "var(--space-8)")],
            )),
        ),
    ]);
    project::save_project(&dir, &model).unwrap();
    (tmp, dir)
}

use project::ProjectState;

fn model(dir: &Path) -> ProjectSnapshot {
    match load_project(dir).unwrap() {
        ProjectState::Loaded(m) => m,
        ProjectState::NoMetadata => panic!("no model"),
    }
}

fn css(dir: &Path) -> String {
    fs::read_to_string(dir.join(MEDIA_CSS)).unwrap()
}

fn rule(css: &str, selector: &str) -> String {
    let start = css
        .find(&format!("{selector} {{"))
        .unwrap_or_else(|| panic!("{selector} in {css}"));
    let end = start + css[start..].find("}\n").unwrap() + 2;
    css[start..end].to_string()
}

fn binding_of(dir: &Path, id: &str) -> StyleBinding {
    model(dir)
        .element(&ElementId::new(id))
        .unwrap()
        .style
        .clone()
        .unwrap()
}

fn select(dir: &Path, selection: Option<&str>, focus: Option<&str>) {
    let mut session = load_session(dir, DEFAULT_CONVERSATION_ID).unwrap();
    session.selection = selection.map(ElementId::new);
    session.focus = focus.map(ElementId::new);
    save_session(dir, &session).unwrap();
}

fn assert_clean(dir: &Path) {
    assert!(project::verify_owned(dir, &model(dir)).unwrap().is_empty());
}

fn applied(result: &ModifyResult) -> Vec<String> {
    match &result.outcome {
        ModifyOutcome::Applied { changed_files, .. } => changed_files.clone(),
        other => panic!("expected applied, got {other:?}"),
    }
}

#[test]
fn form_below_hero_is_resolved_structurally_and_gets_scoped_padding() {
    let (_tmp, dir) = styled_project();
    let result = modify(&dir, "Give the form below hero more padding").unwrap();
    assert_eq!(applied(&result), [MEDIA_CSS]);
    let text = css(&dir);
    // 2rem 1rem -> 3rem 1.5rem: both axes advance one token, independently.
    assert_eq!(
        rule(&text, ".signup"),
        ".signup {\n  padding: var(--space-7) var(--space-5);\n  background: #fff;\n}\n"
    );
    // The later form and every other rule are untouched.
    assert!(text.contains(".contact {\n  padding: var(--space-8);\n}\n"));
    assert!(text.starts_with(&CSS[..CSS.find(".signup").unwrap()]));
    assert_eq!(
        binding_of(&dir, "form_1").values["padding"],
        "var(--space-7) var(--space-5)"
    );
    assert_eq!(
        binding_of(&dir, "form_2").values["padding"],
        "var(--space-8)"
    );
    assert_eq!(
        load_session(&dir, DEFAULT_CONVERSATION_ID).unwrap().focus,
        Some(ElementId::new("form_1"))
    );
    assert!(result.summary.contains("form_1"), "{}", result.summary);
    assert_clean(&dir);
}

#[test]
fn rounded_corners_apply_to_a_resolved_image_only() {
    let (_tmp, dir) = styled_project();
    select(&dir, Some("img_3"), None);
    let result = modify(&dir, "Image needs rounded corners").unwrap();
    assert_eq!(applied(&result), [MEDIA_CSS]);
    let text = css(&dir);
    assert_eq!(
        rule(&text, ".portrait"),
        ".portrait {\n  display: block;\n  border-radius: var(--radius-md);\n}\n"
    );
    assert_eq!(rule(&text, ".photo"), rule(CSS, ".photo"));
    assert_eq!(
        binding_of(&dir, "img_3").values["border-radius"],
        "var(--radius-md)"
    );
    // Ensure-style: asking again changes nothing.
    let before = css(&dir);
    let again = modify(&dir, "Image needs rounded corners").unwrap();
    assert!(
        matches!(again.outcome, ModifyOutcome::NoChange { .. }),
        "{again:?}"
    );
    assert_eq!(css(&dir), before);
    assert_clean(&dir);
}

#[test]
fn ambiguous_images_ask_which_and_write_nothing() {
    let (_tmp, dir) = styled_project();
    let before = css(&dir);
    let result = modify(&dir, "Image needs rounded corners").unwrap();
    let ModifyOutcome::NeedsClarification { questions } = &result.outcome else {
        panic!("{result:?}")
    };
    assert_eq!(questions.len(), 1);
    let q = &questions[0];
    assert_eq!(q.kind, QuestionKind::BlockingClarification);
    assert!(q.prompt.contains("Which image"), "{}", q.prompt);
    let keys: Vec<_> = q.options.iter().map(|o| o.key.as_str()).collect();
    assert_eq!(keys, ["img_1", "img_2", "img_3"]);
    assert_eq!(css(&dir), before);
    assert_eq!(model(&dir).revision, 0);
    assert_eq!(load_session(&dir, DEFAULT_CONVERSATION_ID).unwrap().turn, 0);

    // Compatible focus (a recently edited image) resolves it; a focused form does not.
    select(&dir, None, Some("form_1"));
    assert!(matches!(
        modify(&dir, "Image needs rounded corners").unwrap().outcome,
        ModifyOutcome::NeedsClarification { .. }
    ));
    select(&dir, None, Some("img_3"));
    let result = modify(&dir, "Image needs rounded corners").unwrap();
    assert_eq!(applied(&result), [MEDIA_CSS]);
    assert!(rule(&css(&dir), ".portrait").contains("border-radius"));
}

#[test]
fn answering_the_which_image_question_applies_the_change() {
    use protopie_ui_agent::answer_with;
    let (_tmp, dir) = styled_project();
    let ModifyOutcome::NeedsClarification { questions } =
        modify(&dir, "Image needs rounded corners").unwrap().outcome
    else {
        panic!()
    };
    // The resolver's question is not persisted; a client that does persist it
    // (for example a review surface) can still answer it by ID.
    let mut session = load_session(&dir, DEFAULT_CONVERSATION_ID).unwrap();
    session.add_pending(questions[0].clone());
    save_session(&dir, &session).unwrap();
    let answer = |key: &str| Answer {
        question_id: questions[0].id.clone(),
        choice: AnswerChoice::Option { key: key.into() },
    };
    // Only offered options are valid.
    let bad = answer_with(&dir, &answer("form_1"), &ModifyOptions::default()).unwrap();
    assert!(matches!(
        bad.outcome,
        ModifyOutcome::NeedsClarification { .. }
    ));
    assert_eq!(css(&dir), CSS);
    let ok = answer_with(&dir, &answer("img_3"), &ModifyOptions::default()).unwrap();
    assert_eq!(applied(&ok), [MEDIA_CSS]);
    assert!(rule(&css(&dir), ".portrait").contains("border-radius"));
    let session = load_session(&dir, DEFAULT_CONVERSATION_ID).unwrap();
    assert!(session.pending_questions.is_empty());
    assert_eq!(session.focus, Some(ElementId::new("img_3")));
}

#[test]
fn instance_only_request_leaves_shared_instances_unaffected() {
    let (_tmp, dir) = styled_project();
    select(&dir, Some("img_1"), None);
    let result = modify(&dir, "Image needs rounded corners").unwrap();
    assert_eq!(applied(&result), [MEDIA_CSS]);
    let text = css(&dir);
    // The shared class rule is byte-identical; img_1 got a more specific rule.
    assert_eq!(rule(&text, ".photo"), rule(CSS, ".photo"));
    assert_eq!(
        rule(&text, ".photo[data-protopie-id=\"img_1\"]"),
        ".photo[data-protopie-id=\"img_1\"] {\n  border-radius: var(--radius-md);\n}\n"
    );
    assert!(!text.contains("img_2"));
    let one = binding_of(&dir, "img_1");
    assert_eq!(one.scope, StyleScope::Instance);
    assert!(one.instance_override);
    let two = binding_of(&dir, "img_2");
    assert_eq!(two.scope, StyleScope::Definition);
    assert!(!two.instance_override && two.values.is_empty());

    // A second edit of the same instance extends its own rule in place.
    let result = modify(&dir, "Image needs full width").unwrap();
    assert_eq!(applied(&result), [MEDIA_CSS]);
    let text = css(&dir);
    assert_eq!(text.matches("img_1").count(), 1);
    assert_eq!(
        rule(&text, ".photo[data-protopie-id=\"img_1\"]"),
        ".photo[data-protopie-id=\"img_1\"] {\n  border-radius: var(--radius-md);\n  width: 100%;\n}\n"
    );
    assert_eq!(rule(&text, ".photo"), rule(CSS, ".photo"));
    // The other instance can still be styled on its own.
    select(&dir, Some("img_2"), None);
    modify(&dir, "Image needs rounded corners").unwrap();
    let text = css(&dir);
    assert!(text.contains(".photo[data-protopie-id=\"img_2\"] {"));
    assert_eq!(rule(&text, ".photo"), rule(CSS, ".photo"));
    assert_clean(&dir);
}

#[test]
fn two_fresh_padding_requests_advance_twice() {
    let (_tmp, dir) = styled_project();
    let prompt = "Give the form below hero more padding";
    modify(&dir, prompt).unwrap();
    let after_one = rule(&css(&dir), ".signup");
    assert!(after_one.contains("padding: var(--space-7) var(--space-5);"));
    modify(&dir, prompt).unwrap();
    assert!(rule(&css(&dir), ".signup").contains("padding: var(--space-8) var(--space-6);"));
    assert_eq!(model(&dir).revision, 2);
    assert_clean(&dir);
}

#[test]
fn retrying_either_plan_does_not_advance_again() {
    let (_tmp, dir) = styled_project();
    let prompt = "Give the form below hero more padding";
    let plan_now = |dir: &Path| match plan_prompt(
        prompt,
        &model(dir),
        &load_session(dir, DEFAULT_CONVERSATION_ID).unwrap(),
    ) {
        PlanOutcome::Ready { plan } => plan,
        other => panic!("{other:?}"),
    };
    let apply = |dir: &Path, plan: &Plan| {
        apply_plan(dir, DEFAULT_CONVERSATION_ID, plan, &ApplyOptions::default()).unwrap()
    };

    // The plan carries the absolute result, not an increment.
    let first = plan_now(&dir);
    let Operation::SetStyle { edits, scope, .. } = &first.operations[0] else {
        panic!("{first:?}")
    };
    assert_eq!(*scope, StyleScope::Instance);
    assert_eq!(
        edits,
        &[StyleEdit {
            property: StyleProperty::Padding,
            value: "var(--space-7) var(--space-5)".into()
        }]
    );
    assert!(matches!(apply(&dir, &first), ApplyOutcome::Applied(_)));
    let after_first = css(&dir);

    // Retrying the first plan replays instead of advancing.
    assert!(matches!(apply(&dir, &first), ApplyOutcome::Replayed(_)));
    assert_eq!(css(&dir), after_first);

    // A fresh request plans from the new state and advances once more.
    let second = plan_now(&dir);
    assert_ne!(first.id, second.id);
    assert!(matches!(apply(&dir, &second), ApplyOutcome::Applied(_)));
    let after_second = css(&dir);
    assert_ne!(after_second, after_first);
    for plan in [&second, &first] {
        assert!(matches!(apply(&dir, plan), ApplyOutcome::Replayed(_)));
        assert_eq!(css(&dir), after_second);
    }
    assert_eq!(model(&dir).revision, 2);

    // Transport retries through the API boundary replay too, and a stale
    // unapplied plan (made before the changes above) is a conflict.
    let options = ModifyOptions {
        request_id: Some("req-1"),
        ..Default::default()
    };
    modify_with(&dir, prompt, &options).unwrap();
    let after_third = css(&dir);
    let retry = modify_with(&dir, prompt, &options).unwrap();
    assert!(retry.summary.contains("already applied"), "{retry:?}");
    assert_eq!(css(&dir), after_third);
    let mut stale = plan_now(&dir);
    stale.base_revision -= 1;
    assert!(matches!(apply(&dir, &stale), ApplyOutcome::Conflict { .. }));
    assert_eq!(css(&dir), after_third);
    assert_clean(&dir);
}

#[test]
fn padding_steps_clamp_and_report_no_change_at_the_limits() {
    let (_tmp, dir) = styled_project();
    select(&dir, Some("form_2"), None);
    let before = css(&dir);
    // form_2 is at --space-8: one more step reaches the top, then nothing moves.
    assert_eq!(
        applied(&modify(&dir, "Increase form padding").unwrap()),
        [MEDIA_CSS]
    );
    assert!(rule(&css(&dir), ".contact").contains("padding: var(--space-9);"));
    let top = css(&dir);
    let r = modify(&dir, "Increase form padding").unwrap();
    assert!(matches!(r.outcome, ModifyOutcome::NoChange { .. }), "{r:?}");
    assert_eq!(css(&dir), top);
    assert_ne!(top, before);
    // Decreasing walks back down the scale one token at a time.
    modify(&dir, "Decrease form padding").unwrap();
    assert!(rule(&css(&dir), ".contact").contains("padding: var(--space-8);"));
}

#[test]
fn unsupported_values_are_explained_and_never_rewritten() {
    let (_tmp, dir) = styled_project();
    let mut m = model(&dir);
    m.elements
        .iter_mut()
        .find(|e| e.id.0 == "form_2")
        .unwrap()
        .style
        .as_mut()
        .unwrap()
        .values
        .insert("padding".into(), "calc(1rem + 2px)".into());
    project::save_project(&dir, &m).unwrap();
    select(&dir, Some("form_2"), None);
    let before = css(&dir);
    let r = modify(&dir, "Increase form padding").unwrap();
    let ModifyOutcome::Unsupported { rejection } = &r.outcome else {
        panic!("{r:?}")
    };
    assert!(rejection.explanation.contains("calc(1rem + 2px)"));
    assert_eq!(css(&dir), before);
}

#[test]
fn full_width_requires_a_known_vertical_parent_layout() {
    let (_tmp, dir) = styled_project();
    let mut m = model(&dir);
    // img_3 now sits in a container with no known layout; img_1 in a horizontal one.
    let mut row = element("card_1", ElementKind::Hero, None);
    row.layout = Some(ContainerLayout::Horizontal);
    m.elements.push(row);
    m.elements.push(element("card_2", ElementKind::Hero, None));
    for (id, parent) in [("img_3", "card_2"), ("img_1", "card_1")] {
        m.elements.iter_mut().find(|e| e.id.0 == id).unwrap().parent = Some(ElementId::new(parent));
    }
    project::save_project(&dir, &m).unwrap();
    for id in ["img_3", "img_1"] {
        select(&dir, Some(id), None);
        let before = css(&dir);
        let r = modify(&dir, "Image needs full width").unwrap();
        let ModifyOutcome::Unsupported { rejection } = &r.outcome else {
            panic!("{id}: {r:?}")
        };
        assert_eq!(rejection.reason, RejectionReason::UnsupportedLayout);
        assert_eq!(css(&dir), before);
    }
    // Directly on the vertical home page it works, and repeats are no-ops.
    select(&dir, Some("img_2"), None);
    assert_eq!(
        applied(&modify(&dir, "Image needs full width").unwrap()),
        [MEDIA_CSS]
    );
    assert!(rule(&css(&dir), ".photo[data-protopie-id=\"img_2\"]").contains("width: 100%;"));
    let r = modify(&dir, "Image needs full width").unwrap();
    assert!(matches!(r.outcome, ModifyOutcome::NoChange { .. }), "{r:?}");
}

#[test]
fn relational_selectors_never_guess_layout_or_invent_matches() {
    let (_tmp, dir) = styled_project();
    let prompt = "Give the form below hero more padding";
    let before = css(&dir);

    // Unknown container layout: source order is not geometry.
    let mut m = model(&dir);
    m.page_layouts.clear();
    project::save_project(&dir, &m).unwrap();
    let r = modify(&dir, prompt).unwrap();
    let ModifyOutcome::Unsupported { rejection } = &r.outcome else {
        panic!("{r:?}")
    };
    assert_eq!(rejection.reason, RejectionReason::UnsupportedLayout);

    // Known vertical layout but no form after the hero.
    m.page_layouts
        .insert(DEFAULT_PAGE.into(), ContainerLayout::Vertical);
    m.elements.retain(|e| e.kind != ElementKind::Form);
    project::save_project(&dir, &m).unwrap();
    let r = modify(&dir, prompt).unwrap();
    let ModifyOutcome::Unsupported { rejection } = &r.outcome else {
        panic!("{r:?}")
    };
    assert_eq!(rejection.reason, RejectionReason::TargetNotFound);
    assert_eq!(css(&dir), before);
    assert_eq!(model(&dir).revision, 0);
}

#[test]
fn external_edits_and_stale_values_conflict_instead_of_overwriting() {
    let (_tmp, dir) = styled_project();
    // Stale expectation: the recorded value changed after planning.
    let plan = match plan_prompt(
        "Give the form below hero more padding",
        &model(&dir),
        &load_session(&dir, DEFAULT_CONVERSATION_ID).unwrap(),
    ) {
        PlanOutcome::Ready { plan } => plan,
        other => panic!("{other:?}"),
    };
    let mut stale = plan.clone();
    stale.preconditions = vec![Precondition::StyleValueIs {
        id: ElementId::new("form_1"),
        property: StyleProperty::Padding,
        value: Some("1rem".into()),
    }];
    assert!(matches!(
        apply_plan(
            &dir,
            DEFAULT_CONVERSATION_ID,
            &stale,
            &ApplyOptions::default()
        )
        .unwrap(),
        ApplyOutcome::Conflict { .. }
    ));

    // External edit of the owned stylesheet.
    fs::write(dir.join(MEDIA_CSS), format!("{CSS}/* mine */\n")).unwrap();
    let edited = css(&dir);
    let r = modify(&dir, "Give the form below hero more padding").unwrap();
    assert!(matches!(r.outcome, ModifyOutcome::Conflict { .. }), "{r:?}");
    assert_eq!(css(&dir), edited);
}

#[test]
fn dry_run_previews_the_style_change_without_writing() {
    let (_tmp, dir) = styled_project();
    let before = css(&dir);
    let dry = ModifyOptions {
        dry_run: true,
        ..Default::default()
    };
    let r = modify_with(&dir, "Give the form below hero more padding", &dry).unwrap();
    let ModifyOutcome::Preview { changed_files, .. } = &r.outcome else {
        panic!("{r:?}")
    };
    assert_eq!(changed_files, &[MEDIA_CSS.to_string()]);
    assert_eq!(css(&dir), before);
    assert_eq!(model(&dir).revision, 0);
    assert_eq!(
        load_session(&dir, DEFAULT_CONVERSATION_ID).unwrap().focus,
        None
    );
}

#[test]
fn style_edits_work_inside_a_shared_files_owned_region() {
    // The hero's rule lives in a region of Home.module.css. Give a form a rule
    // in that region and check only the region changes.
    let (_tmp, dir) = styled_project();
    let path = "src/pages/Home.module.css";
    let original = fs::read_to_string(dir.join(path)).unwrap();
    let with_rule = original.replace(
        "/* protopie:end hero_1 */",
        ".note {\n  padding: 1rem;\n}\n/* protopie:end hero_1 */",
    );
    fs::write(dir.join(path), &with_rule).unwrap();
    let mut m = model(&dir);
    let region = project::extract_region(&with_rule, "hero_1").unwrap();
    for o in &mut m.owned {
        if o.path == path {
            o.fingerprint = fingerprint(&region);
        }
    }
    let form = m.elements.iter_mut().find(|e| e.id.0 == "form_1").unwrap();
    form.style = Some(StyleBinding {
        file: path.into(),
        class: "note".into(),
        region: Some("hero_1".into()),
        scope: StyleScope::Instance,
        values: [("padding".to_string(), "1rem".to_string())].into(),
        instance_override: false,
    });
    project::save_project(&dir, &m).unwrap();
    let r = modify(&dir, "Give the form below hero more padding").unwrap();
    assert_eq!(applied(&r), [path]);
    let after = fs::read_to_string(dir.join(path)).unwrap();
    assert!(after.contains(".note {\n  padding: var(--space-5);\n}\n"));
    assert_eq!(
        after.replace("var(--space-5)", "1rem"),
        with_rule,
        "only the owned declaration changed"
    );
    assert_clean(&dir);
}
