//! Page elements and positioning (Epic 001, T8), end to end against copies of
//! the reference template.
//!
//! Every generated element must be a first-class model citizen: it carries
//! `data-protopie-id`, a style binding whose `values` reflect its stylesheet,
//! and a layout where it is a container, so T7 styling and relational
//! selectors work on it. Content is never invented: it is asked for first.

use std::fs;
use std::path::{Path, PathBuf};

use protopie_ui_agent::contracts::*;
use protopie_ui_agent::pipeline::plan_prompt;
use protopie_ui_agent::project::{self, extract_region, load_project, load_session, save_session};
use protopie_ui_agent::{answer_with, init_named, modify, modify_with, ModifyOptions};

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

fn say(dir: &Path, text: &str) -> ModifyResult {
    modify(dir, text).unwrap()
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
    let result = say(dir, text);
    assert_eq!(kind(&result), want, "{text}: {result:?}");
    result
}

fn changed(result: &ModifyResult) -> Vec<String> {
    match &result.outcome {
        ModifyOutcome::Applied { changed_files, .. }
        | ModifyOutcome::Preview { changed_files, .. } => {
            let mut files = changed_files.clone();
            files.sort();
            files
        }
        other => panic!("{other:?}"),
    }
}

fn clean(dir: &Path) {
    assert!(
        project::verify_owned(dir, &model(dir)).unwrap().is_empty(),
        "owned source drifted"
    );
}

/// `(above, below)` component names in the `home-flow` region, in order.
fn flow(dir: &Path) -> (Vec<String>, Vec<String>) {
    let region = extract_region(&read(dir, "src/pages/Home.tsx"), "home-flow").unwrap();
    let section = |name: &str| -> Vec<String> {
        let start = region.find(&format!("function {name}()")).unwrap();
        let rest = &region[start..];
        let end = rest.find("\n}\n").unwrap();
        rest[..end]
            .lines()
            .filter_map(|l| l.trim().strip_prefix('<'))
            .filter_map(|l| l.strip_suffix(" />"))
            .filter(|l| *l != ">" && !l.is_empty() && !l.starts_with('/'))
            .map(str::to_string)
            .collect()
    };
    (section("HomeAbove"), section("HomeBelow"))
}

fn names(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| s.to_string()).collect()
}

fn add_form(dir: &Path, prompt: &str) {
    expect(dir, prompt, "needs_clarification");
    expect(dir, "Name, Email", "needs_clarification");
    expect(dir, "Send", "applied");
}

fn add_footer(dir: &Path, prompt: &str) {
    expect(dir, prompt, "needs_clarification");
    expect(dir, "Made with care", "applied");
}

fn add_image(dir: &Path, prompt: &str, alt: &str) {
    expect(dir, prompt, "needs_clarification");
    expect(dir, "https://example.com/cat.png", "needs_clarification");
    expect(dir, alt, "applied");
}

/// Forgets the element focused by the last edit, so a tie is not broken by it.
fn clear_focus(dir: &Path) {
    let mut session = load_session(dir, DEFAULT_CONVERSATION_ID).unwrap();
    session.focus = None;
    save_session(dir, &session).unwrap();
}

fn pending(dir: &Path) -> Vec<Question> {
    load_session(dir, DEFAULT_CONVERSATION_ID)
        .unwrap()
        .pending_questions
}

fn rule(css: &str, selector: &str) -> String {
    let start = css.find(&format!("{selector} {{")).unwrap();
    let rest = &css[start..];
    rest[..rest.find("}\n").unwrap() + 2].to_string()
}

#[test]
fn seeded_hero_reports_its_declared_padding_and_layout() {
    let (_tmp, dir) = fresh();
    let model = model(&dir);
    let hero = model.element(&ElementId::new("hero_1")).unwrap();
    let style = hero.style.as_ref().unwrap();
    assert_eq!(style.values["padding"], "2rem 1rem");
    assert_eq!(hero.layout, Some(ContainerLayout::Vertical));
    assert_eq!(
        model.page_layouts.get(DEFAULT_PAGE),
        Some(&ContainerLayout::Vertical)
    );
    let tsx = read(&dir, "src/pages/Home.tsx");
    assert!(tsx.contains("data-protopie-id=\"hero_1\""));
}

#[test]
fn hero_is_a_style_target() {
    let (_tmp, dir) = fresh();
    let before_css = read(&dir, "src/pages/Home.module.css");
    let result = expect(&dir, "Give the hero more padding", "applied");
    assert_eq!(changed(&result), ["src/pages/Home.module.css"]);
    let css = read(&dir, "src/pages/Home.module.css");
    assert!(css.contains("padding: var(--space-7) var(--space-5);"), "{css}");
    // Only the declaration inside the owned region changed.
    assert_eq!(
        css.replace("padding: var(--space-7) var(--space-5);", "padding: 2rem 1rem;"),
        before_css
    );
    // A second fresh request advances again.
    expect(&dir, "Give the hero more padding", "applied");
    assert!(read(&dir, "src/pages/Home.module.css")
        .contains("padding: var(--space-8) var(--space-6);"));
    expect(&dir, "Hero needs rounded corners", "applied");
    assert!(rule(&read(&dir, "src/pages/Home.module.css"), ".hero")
        .contains("border-radius: var(--radius-md);"));
    expect(&dir, "Hero needs rounded corners", "no_change");
    clean(&dir);
}

#[test]
fn form_below_hero_asks_for_content_and_then_generates() {
    let (_tmp, dir) = fresh();
    let before = model(&dir);
    let home_before = read(&dir, "src/pages/Home.tsx");

    // Content is asked for, persisted, and nothing is written.
    let result = expect(&dir, "add a form below hero", "needs_clarification");
    let ModifyOutcome::NeedsClarification { questions } = &result.outcome else {
        unreachable!()
    };
    assert_eq!(questions.len(), 1);
    assert!(questions[0].prompt.contains("Which fields"), "{}", questions[0].prompt);
    assert_eq!(questions[0].kind, QuestionKind::BlockingClarification);
    assert_eq!(pending(&dir).len(), 1);
    assert_eq!(read(&dir, "src/pages/Home.tsx"), home_before);
    assert!(!dir.join("src/components/Form1.tsx").exists());
    assert_eq!(model(&dir).elements, before.elements);

    expect(&dir, "Name, Email", "needs_clarification");
    assert_eq!(pending(&dir).len(), 1);
    let result = expect(&dir, "Send", "applied");
    assert_eq!(
        changed(&result),
        [
            "src/components/Form1.module.css",
            "src/components/Form1.tsx",
            "src/pages/Home.tsx"
        ]
    );
    assert!(result.summary.contains("Added a form (form_1)"), "{}", result.summary);
    assert!(result.summary.contains("Placed form_1 below hero_1"), "{}", result.summary);
    assert!(result.summary.contains("does nothing yet"));
    assert!(pending(&dir).is_empty());

    let tsx = read(&dir, "src/components/Form1.tsx");
    assert!(tsx.contains("data-protopie-id=\"form_1\""));
    assert!(tsx.contains("{\"Name\"}") && tsx.contains("{\"Email\"}") && tsx.contains("{\"Send\"}"));
    assert!(tsx.contains("name=\"field-2\""));
    assert_eq!(flow(&dir), (names(&[]), names(&["Form1"])));
    let home = read(&dir, "src/pages/Home.tsx");
    assert!(home.contains("import Form1 from \"../components/Form1\";"));
    // Outside the owned regions Home.tsx is untouched.
    assert!(home.contains("<HomeAbove />") && home.contains("<HomeBelow />"));

    let model = model(&dir);
    let form = model.element(&ElementId::new("form_1")).unwrap();
    assert_eq!(form.kind, ElementKind::Form);
    assert_eq!(form.layout, Some(ContainerLayout::Vertical));
    let style = form.style.as_ref().unwrap();
    assert_eq!(style.class, "form");
    assert_eq!(style.file, "src/components/Form1.module.css");
    assert_eq!(style.values["padding"], "2rem 1rem");
    assert!(!style.instance_override);
    let content = form.content.as_ref().unwrap();
    assert_eq!(content.fields, ["Name", "Email"]);
    assert_eq!(content.text.as_deref(), Some("Send"));
    // Sibling order: hero first, then the form.
    let order: Vec<_> = model.elements.iter().map(|e| e.id.0.as_str()).collect();
    assert_eq!(order, ["hero_1", "form_1"]);
    assert_eq!(
        load_session(&dir, DEFAULT_CONVERSATION_ID).unwrap().focus,
        Some(ElementId::new("form_1"))
    );
    clean(&dir);
}

#[test]
fn generated_elements_work_with_styles_and_relational_selectors() {
    let (_tmp, dir) = fresh();
    add_form(&dir, "add a form below hero");
    add_form(&dir, "Add a form above hero");
    assert_eq!(flow(&dir), (names(&["Form2"]), names(&["Form1"])));
    let form2_css = read(&dir, "src/components/Form2.module.css");

    // "the form below hero" is structural: form_1, never form_2 (above it).
    let result = expect(&dir, "Give the form below hero more padding", "applied");
    assert_eq!(changed(&result), ["src/components/Form1.module.css"]);
    assert!(read(&dir, "src/components/Form1.module.css")
        .contains("padding: var(--space-7) var(--space-5);"));
    // Two fresh requests advance twice.
    expect(&dir, "Give the form below hero more padding", "applied");
    assert!(read(&dir, "src/components/Form1.module.css")
        .contains("padding: var(--space-8) var(--space-6);"));
    assert_eq!(read(&dir, "src/components/Form2.module.css"), form2_css);
    assert_eq!(
        model(&dir)
            .element(&ElementId::new("form_1"))
            .unwrap()
            .style
            .as_ref()
            .unwrap()
            .values["padding"],
        "var(--space-8) var(--space-6)"
    );

    // Two forms and no relation: ask which one, persisting nothing. (The
    // last edit focused form_1, so forget it first: focus breaks ties.)
    clear_focus(&dir);
    let result = expect(&dir, "Increase form padding", "needs_clarification");
    let ModifyOutcome::NeedsClarification { questions } = &result.outcome else {
        unreachable!()
    };
    let keys: Vec<_> = questions[0].options.iter().map(|o| o.key.as_str()).collect();
    assert_eq!(keys, ["form_2", "form_1"]);
    clean(&dir);
}

#[test]
fn footer_is_ensure_style_and_hero_already_exists() {
    let (_tmp, dir) = fresh();
    let result = expect(&dir, "Add a hero", "no_change");
    assert!(result.summary.contains("hero_1"), "{}", result.summary);
    add_footer(&dir, "Need a footer");
    assert_eq!(flow(&dir), (names(&[]), names(&["Footer1"])));
    let tsx = read(&dir, "src/components/Footer1.tsx");
    assert!(tsx.contains("data-protopie-id=\"footer_1\"") && tsx.contains("{\"Made with care\"}"));
    let before = model(&dir);
    expect(&dir, "add a footer", "no_change");
    assert_eq!(model(&dir), before);
    // Footers are style targets too.
    expect(&dir, "Increase footer padding", "applied");
    assert!(read(&dir, "src/components/Footer1.module.css")
        .contains("padding: var(--space-7) var(--space-5);"));
    clean(&dir);
}

#[test]
fn images_ask_for_address_and_alt_and_are_style_targets() {
    let (_tmp, dir) = fresh();
    add_footer(&dir, "Need a footer");
    expect(&dir, "add an image above the footer", "needs_clarification");
    // A bad address is rejected, asked again, and nothing is written.
    let result = expect(&dir, "not a url", "needs_clarification");
    assert!(summary_has(&result, "not a valid http(s) URL"));
    assert!(!dir.join("src/components/Image1.tsx").exists());
    expect(&dir, "https://example.com/a?b=1&c=2", "needs_clarification");
    expect(&dir, "He said \"hi\" <b>{x}</b>", "applied");
    assert_eq!(flow(&dir), (names(&[]), names(&["Image1", "Footer1"])));
    let tsx = read(&dir, "src/components/Image1.tsx");
    assert!(tsx.contains("src={\"https://example.com/a?b=1\\u0026c=2\"}"), "{tsx}");
    assert!(tsx.contains("alt={\"He said \\\"hi\\\" \\u003cb\\u003e\\u007bx\\u007d\\u003c/b\\u003e\"}"), "{tsx}");
    assert!(!tsx.contains("<b>"));

    expect(&dir, "Image needs rounded corners", "applied");
    assert!(rule(&read(&dir, "src/components/Image1.module.css"), ".image")
        .contains("border-radius: var(--radius-md);"));
    expect(&dir, "Image needs full width", "applied");
    assert!(rule(&read(&dir, "src/components/Image1.module.css"), ".image")
        .contains("width: 100%;"));
    expect(&dir, "Image needs full width", "no_change");
    clean(&dir);
}

fn summary_has(result: &ModifyResult, needle: &str) -> bool {
    result.summary.contains(needle)
}

#[test]
fn button_with_a_label_asks_for_its_destination_and_invents_nothing() {
    let (_tmp, dir) = fresh();
    let result = expect(&dir, "Add a button called Get a callback", "applied");
    let ModifyOutcome::Applied { follow_up, .. } = &result.outcome else {
        unreachable!()
    };
    assert_eq!(follow_up.len(), 1);
    assert!(follow_up[0].prompt.contains("Get a callback"));
    assert_eq!(follow_up[0].target, Some(ElementId::new("button_1")));
    let tsx = read(&dir, "src/components/Button1.tsx");
    assert!(tsx.contains("disabled") && !tsx.contains("href"), "{tsx}");
    assert!(tsx.contains("data-protopie-id=\"button_1\"") && tsx.contains("{\"Get a callback\"}"));
    // No page, no route.
    let model = model(&dir);
    assert!(model.pages.is_empty());
    assert!(!read(&dir, "src/router.ts").contains("get-a-callback"));
    assert_eq!(
        model.element(&ElementId::new("button_1")).unwrap().destination,
        Some(Destination::Unresolved)
    );

    // The destination flow of T6 works on buttons.
    let result = expect(&dir, "new page", "applied");
    assert!(result.summary.contains("Linked"), "{}", result.summary);
    let tsx = read(&dir, "src/components/Button1.tsx");
    assert!(tsx.contains("href={\"/get-a-callback\"}") && !tsx.contains("disabled"), "{tsx}");
    assert!(read(&dir, "src/router.ts").contains("/get-a-callback"));
    assert!(read(&dir, "src/pages/GetACallbackPage.tsx").contains("Get a callback"));
    // Asking again or answering again changes nothing.
    assert!(pending(&dir).is_empty());
    clean(&dir);
}

#[test]
fn button_without_a_label_asks_for_one_and_can_stay_unlinked() {
    let (_tmp, dir) = fresh();
    let result = expect(&dir, "Add a button", "needs_clarification");
    assert!(result.summary.contains("What should the button say?"));
    expect(&dir, "Docs", "applied");
    expect(&dir, "leave unlinked", "applied");
    assert!(read(&dir, "src/components/Button1.tsx").contains("disabled"));
    assert!(model(&dir).pages.is_empty());
    // External URL destination on a second button.
    expect(&dir, "Add a button \"Go <now>\"", "applied");
    expect(&dir, "external url", "needs_clarification");
    expect(&dir, "https://example.com/x", "applied");
    let tsx = read(&dir, "src/components/Button2.tsx");
    assert!(tsx.contains("href={\"https://example.com/x\"} rel=\"noopener noreferrer\""));
    assert!(tsx.contains("{\"Go \\u003cnow\\u003e\"}") && !tsx.contains("<now>"));
    // Button 1 below button 2 in the order they were added.
    assert_eq!(flow(&dir), (names(&[]), names(&["Button1", "Button2"])));
    clean(&dir);
}

fn order(dir: &Path) -> Vec<String> {
    model(dir)
        .elements
        .iter()
        .filter(|e| e.kind.is_flow())
        .map(|e| e.id.0.clone())
        .collect()
}

#[test]
fn explicit_positions_order_elements_and_move_requests_reorder_them() {
    let (_tmp, dir) = fresh();
    add_footer(&dir, "Need a footer");
    add_form(&dir, "Add a form below hero");
    assert_eq!(order(&dir), ["hero_1", "form_1", "footer_1"]);
    assert_eq!(flow(&dir), (names(&[]), names(&["Form1", "Footer1"])));
    // Without a position an element goes to the end of the page.
    add_image(&dir, "Add an image", "A cat");
    assert_eq!(order(&dir), ["hero_1", "form_1", "footer_1", "img_1"]);

    let result = expect(&dir, "Move the footer above the form", "applied");
    assert_eq!(changed(&result), ["src/pages/Home.tsx"]);
    assert!(result.summary.contains("Moved footer_1 above form_1"), "{}", result.summary);
    assert_eq!(order(&dir), ["hero_1", "footer_1", "form_1", "img_1"]);
    assert_eq!(flow(&dir), (names(&[]), names(&["Footer1", "Form1", "Image1"])));
    // Already there: nothing to do, and it is not a second move.
    let before = model(&dir);
    let result = expect(&dir, "Move the footer above the form", "no_change");
    assert!(result.summary.contains("already above"), "{}", result.summary);
    assert_eq!(model(&dir), before);

    // Moving above the hero changes which side of it the element renders on.
    expect(&dir, "Move the form above the hero", "applied");
    assert_eq!(order(&dir), ["form_1", "hero_1", "footer_1", "img_1"]);
    assert_eq!(flow(&dir), (names(&["Form1"]), names(&["Footer1", "Image1"])));
    // The hero can move too: the split follows the model, not the markup.
    expect(&dir, "move the hero below the image", "applied");
    assert_eq!(order(&dir), ["form_1", "footer_1", "img_1", "hero_1"]);
    assert_eq!(flow(&dir), (names(&["Form1", "Footer1", "Image1"]), names(&[])));
    assert_eq!(
        load_session(&dir, DEFAULT_CONVERSATION_ID).unwrap().focus,
        Some(ElementId::new("hero_1"))
    );
    // A move creates nothing and writes no component.
    assert!(!dir.join("src/components/Hero2.tsx").exists());
    clean(&dir);
}

#[test]
fn moving_an_element_relative_to_itself_or_a_missing_anchor_is_rejected() {
    let (_tmp, dir) = fresh();
    add_footer(&dir, "Need a footer");
    let before = model(&dir);
    let home = read(&dir, "src/pages/Home.tsx");
    let result = expect(&dir, "Move the footer below the footer", "unsupported");
    assert!(matches!(
        &result.outcome,
        ModifyOutcome::Unsupported { rejection } if rejection.reason == RejectionReason::TargetNotFound
    ), "{result:?}");
    let result = expect(&dir, "Move the form below hero", "unsupported");
    assert!(matches!(
        &result.outcome,
        ModifyOutcome::Unsupported { rejection } if rejection.reason == RejectionReason::TargetNotFound
    ));
    let result = expect(&dir, "Move the footer above the button", "unsupported");
    assert!(result.summary.contains("received"), "{}", result.summary);
    assert_eq!(model(&dir), before);
    assert_eq!(read(&dir, "src/pages/Home.tsx"), home);
}

#[test]
fn horizontal_placement_needs_a_known_horizontal_layout() {
    let (_tmp, dir) = fresh();
    add_image(&dir, "Add an image below hero", "A cat");
    let before = model(&dir);
    let home = read(&dir, "src/pages/Home.tsx");
    // The home page is a known vertical flow: side by side is not supported.
    for prompt in [
        "Add a button called Go right of the image",
        "Add a button called Go to the left of the image",
        "Move the image left of the hero",
    ] {
        let result = expect(&dir, prompt, "unsupported");
        let ModifyOutcome::Unsupported { rejection } = &result.outcome else {
            unreachable!()
        };
        assert_eq!(rejection.reason, RejectionReason::UnsupportedLayout, "{prompt}");
        assert!(rejection.explanation.contains("horizontal"), "{}", rejection.explanation);
    }
    assert_eq!(model(&dir), before);
    assert_eq!(read(&dir, "src/pages/Home.tsx"), home);

    // An unknown layout rejects above/below as well; nothing is guessed.
    let mut unknown = before.clone();
    unknown.page_layouts.clear();
    let session = Session::default();
    let outcome = plan_prompt("Add a button called Go below the image", &unknown, &session);
    let PlanOutcome::Unsupported { rejection } = outcome else {
        panic!("{outcome:?}")
    };
    assert_eq!(rejection.reason, RejectionReason::UnsupportedLayout);

    // With a known horizontal layout, left/right are planned and below is not.
    let mut horizontal = before.clone();
    horizontal
        .page_layouts
        .insert(DEFAULT_PAGE.into(), ContainerLayout::Horizontal);
    let outcome = plan_prompt("Add a button called Go right of the image", &horizontal, &session);
    let PlanOutcome::Ready { plan } = outcome else {
        panic!("{outcome:?}")
    };
    assert!(matches!(
        &plan.operations[1],
        Operation::InsertElement { at: Some(Position { relation: PositionRelation::RightOf, anchor }), .. }
            if anchor.0 == "img_1"
    ));
    let outcome = plan_prompt("Add a button called Go below the image", &horizontal, &session);
    assert!(matches!(
        outcome,
        PlanOutcome::Unsupported { rejection } if rejection.reason == RejectionReason::UnsupportedLayout
    ));
    let outcome = plan_prompt("Move the image left of the hero", &horizontal, &session);
    assert!(matches!(outcome, PlanOutcome::Ready { .. }), "{outcome:?}");

    // The emitter only renders a vertical flow: a model that claims a
    // horizontal page is reported, and nothing is written.
    project::save_project(&dir, &horizontal).unwrap();
    let result = expect(&dir, "Add a button called Go right of the image", "unsupported");
    assert!(result.summary.contains("received"));
    assert_eq!(read(&dir, "src/pages/Home.tsx"), home);
    assert!(!dir.join("src/components/Button1.tsx").exists());
}

#[test]
fn ambiguous_anchors_ask_which_and_the_answer_places_the_element() {
    let (_tmp, dir) = fresh();
    add_image(&dir, "Add an image", "First");
    add_image(&dir, "Add an image", "Second");
    clear_focus(&dir);
    let result = expect(&dir, "Add a button called Go below the image", "needs_clarification");
    let ModifyOutcome::NeedsClarification { questions } = &result.outcome else {
        unreachable!()
    };
    assert!(questions[0].prompt.contains("Which image"));
    let keys: Vec<_> = questions[0].options.iter().map(|o| o.key.as_str()).collect();
    assert_eq!(keys, ["img_1", "img_2"]);
    // The question is persisted, so a chat reply answers it; no code yet.
    assert_eq!(pending(&dir).len(), 1);
    assert!(!dir.join("src/components/Button1.tsx").exists());
    let result = expect(&dir, "img_1", "applied");
    assert!(result.summary.contains("below img_1"), "{}", result.summary);
    assert_eq!(order(&dir), ["hero_1", "img_1", "button_1", "img_2"]);
    assert_eq!(flow(&dir), (names(&[]), names(&["Image1", "Button1", "Image2"])));
    clean(&dir);
}

#[test]
fn ambiguous_move_targets_ask_which_then_resolve_the_anchor() {
    let (_tmp, dir) = fresh();
    add_image(&dir, "Add an image", "First");
    add_image(&dir, "Add an image", "Second");
    add_footer(&dir, "Need a footer");
    let result = expect(&dir, "Move the image below the footer", "needs_clarification");
    assert!(result.summary.contains("Which image"));
    expect(&dir, "2", "applied");
    assert_eq!(order(&dir), ["hero_1", "img_1", "footer_1", "img_2"]);
    clean(&dir);
}

#[test]
fn answers_are_revalidated_and_stale_ones_change_nothing() {
    let (_tmp, dir) = fresh();
    add_image(&dir, "Add an image", "First");
    add_image(&dir, "Add an image", "Second");
    clear_focus(&dir);
    expect(&dir, "Add a button called Go below the image", "needs_clarification");
    let question = pending(&dir).remove(0);
    // The chosen anchor vanishes before the answer arrives.
    let mut snapshot = model(&dir);
    snapshot.elements.retain(|e| e.id.0 != "img_2");
    project::save_project(&dir, &snapshot).unwrap();
    let home = read(&dir, "src/pages/Home.tsx");
    let result = answer_with(
        &dir,
        &Answer {
            question_id: question.id.clone(),
            choice: AnswerChoice::Option { key: "img_2".into() },
        },
        &ModifyOptions::default(),
    )
    .unwrap();
    assert_eq!(kind(&result), "conflict", "{result:?}");
    assert_eq!(read(&dir, "src/pages/Home.tsx"), home);
    assert!(!dir.join("src/components/Button1.tsx").exists());
    // A made-up option is not an answer.
    let result = answer_with(
        &dir,
        &Answer {
            question_id: question.id.clone(),
            choice: AnswerChoice::Option { key: "form_9".into() },
        },
        &ModifyOptions::default(),
    )
    .unwrap();
    assert_eq!(kind(&result), "needs_clarification");
    // An answer to an unknown question is a conflict.
    let result = answer_with(
        &dir,
        &Answer {
            question_id: "q9-nothing".into(),
            choice: AnswerChoice::Text { text: "x".into() },
        },
        &ModifyOptions::default(),
    )
    .unwrap();
    assert_eq!(kind(&result), "conflict");
}

#[test]
fn dry_runs_never_touch_files_state_or_pending_questions() {
    let (_tmp, dir) = fresh();
    let options = ModifyOptions {
        dry_run: true,
        ..Default::default()
    };
    let before = (model(&dir), load_session(&dir, DEFAULT_CONVERSATION_ID).unwrap());
    let result = modify_with(&dir, "add a form below hero", &options).unwrap();
    assert_eq!(kind(&result), "needs_clarification");
    assert_eq!(
        (model(&dir), load_session(&dir, DEFAULT_CONVERSATION_ID).unwrap()),
        before
    );
    // A complete request previews its files.
    let result = modify_with(&dir, "Add a button called Go below the hero", &options).unwrap();
    assert_eq!(kind(&result), "preview");
    assert_eq!(
        changed(&result),
        [
            "src/components/Button1.module.css",
            "src/components/Button1.tsx",
            "src/pages/Home.tsx"
        ]
    );
    assert_eq!(
        (model(&dir), load_session(&dir, DEFAULT_CONVERSATION_ID).unwrap()),
        before
    );
    assert!(!dir.join("src/components").exists());
}

#[test]
fn repeated_commands_add_again_but_retried_requests_replay() {
    let (_tmp, dir) = fresh();
    let options = ModifyOptions {
        request_id: Some("r1"),
        ..Default::default()
    };
    let first = modify_with(&dir, "Add a button called Go", &options).unwrap();
    assert_eq!(kind(&first), "applied");
    let again = modify_with(&dir, "Add a button called Go", &options).unwrap();
    assert!(again.summary.contains("already applied"), "{}", again.summary);
    assert_eq!(order(&dir), ["hero_1", "button_1"]);
    // A deliberate repeat (a new request) is an additive command: a second button.
    expect(&dir, "Add a button called Go", "applied");
    assert_eq!(order(&dir), ["hero_1", "button_1", "button_2"]);
}

#[test]
fn external_edits_conflict_and_are_never_overwritten() {
    let (_tmp, dir) = fresh();
    add_form(&dir, "add a form below hero");
    let path = dir.join("src/components/Form1.tsx");
    let edited = format!("// mine\n{}", read(&dir, "src/components/Form1.tsx"));
    fs::write(&path, &edited).unwrap();
    let before = model(&dir);
    let result = expect(&dir, "Add a button called Go", "conflict");
    assert!(result.summary.contains("edited externally"), "{}", result.summary);
    assert_eq!(read(&dir, "src/components/Form1.tsx"), edited);
    assert_eq!(model(&dir), before);
    // Home.tsx region edits are caught too.
    fs::write(&path, read(&dir, "src/components/Form1.tsx").replacen("// mine\n", "", 1)).unwrap();
    let home = read(&dir, "src/pages/Home.tsx");
    let tampered = home.replace("<Form1 />", "<Form1 />{/* mine */}");
    fs::write(dir.join("src/pages/Home.tsx"), &tampered).unwrap();
    expect(&dir, "Move the form above the hero", "conflict");
    assert_eq!(read(&dir, "src/pages/Home.tsx"), tampered);
}

#[test]
fn labels_and_content_are_escaped_in_generated_source() {
    let (_tmp, dir) = fresh();
    expect(&dir, "Add a button \"Say \\\"hi\\\" <b>{x}</b>\"", "applied");
    let tsx = read(&dir, "src/components/Button1.tsx");
    assert!(tsx.contains("{\"Say \\\"hi\\\" \\u003cb\\u003e\\u007bx\\u007d\\u003c/b\\u003e\"}"), "{tsx}");
    assert!(!tsx.contains("<b>"));
    // Form fields: duplicates, empties and too many are rejected with a reason.
    expect(&dir, "Add a form", "needs_clarification");
    for bad in ["Name, name", "Name,, Email", "a,b,c,d,e,f,g,h,i"] {
        let result = expect(&dir, bad, "needs_clarification");
        assert!(result.summary.contains("Which fields"), "{bad}: {}", result.summary);
    }
    assert!(!dir.join("src/components/Form1.tsx").exists());
    expect(&dir, "Full \"name\", E-mail <x>", "needs_clarification");
    expect(&dir, "Go", "applied");
    let form = read(&dir, "src/components/Form1.tsx");
    assert!(form.contains("{\"Full \\\"name\\\"\"}") && form.contains("{\"E-mail \\u003cx\\u003e\"}"), "{form}");
    clean(&dir);
}

#[test]
fn elements_are_only_supported_on_the_home_page() {
    let (_tmp, dir) = fresh();
    let mut session = load_session(&dir, DEFAULT_CONVERSATION_ID).unwrap();
    session.page = Some("contact-us".into());
    save_session(&dir, &session).unwrap();
    for prompt in ["Add a form", "Move the form below hero"] {
        let result = expect(&dir, prompt, "unsupported");
        assert!(matches!(
            &result.outcome,
            ModifyOutcome::Unsupported { rejection } if rejection.reason == RejectionReason::CapabilityNotImplemented
        ));
    }
    assert!(pending(&dir).is_empty());
}

#[test]
fn unsupported_remainders_negation_and_other_grammars_never_mutate() {
    let (_tmp, dir) = fresh();
    let before = model(&dir);
    for prompt in [
        "Add a form below hero and delete the footer",
        "do not add a form",
        "Add a form called Contact",
        "Add a form below header",
        "Move the form below hero then add a footer",
    ] {
        let result = expect(&dir, prompt, "unsupported");
        assert!(result.summary.contains("received"), "{prompt}");
    }
    assert_eq!(model(&dir), before);
    assert!(pending(&dir).is_empty());
    // `Add a form to top nav` is still a labelled addition to a navigation.
    let result = expect(&dir, "Add a form to top nav", "needs_clarification");
    assert!(result.summary.contains("no navigation") || result.summary.contains("navigation"));
    assert!(!dir.join("src/components/Form1.tsx").exists());
}

#[test]
fn navigation_links_to_generated_hero_sections_and_buttons_coexist_with_navigation() {
    let (_tmp, dir) = fresh();
    expect(&dir, "Need a top navigation", "applied");
    add_form(&dir, "add a form below hero");
    // Navigation is not part of the page flow: the form is still right below the hero.
    assert_eq!(order(&dir), ["hero_1", "form_1"]);
    expect(&dir, "Add Contact Us", "applied");
    // The navigation item's question is the one that takes `existing`.
    expect(&dir, "unlinked", "applied");
    clean(&dir);
    // The navigation was focused by its own edit, so the next form is placed
    // relative to the hero explicitly.
    add_form(&dir, "add a form above the hero");
    assert_eq!(flow(&dir), (names(&["Form2"]), names(&["Form1"])));
    clean(&dir);
}
