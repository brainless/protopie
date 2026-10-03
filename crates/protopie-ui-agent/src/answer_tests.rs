use super::*;
use crate::apply::{apply_plan, prepare, ApplyOptions, ApplyOutcome, PrepareOutcome};
use crate::project::{load_project, load_session, verify_owned, ProjectState};
use crate::{answer_with, init_named, modify, ModifyOptions};
use serde_json::json;
use std::fs;
use std::path::{Path, PathBuf};

const CONV: &str = DEFAULT_CONVERSATION_ID;

fn model(dir: &Path) -> ProjectSnapshot {
    match load_project(dir).unwrap() {
        ProjectState::Loaded(s) => s,
        ProjectState::NoMetadata => panic!("no model"),
    }
}

fn disk(dir: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    let mut files = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in fs::read_dir(&d).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                stack.push(p);
            } else if p.file_name().is_some_and(|n| n != "lock") {
                files.push((p.clone(), fs::read(&p).unwrap()));
            }
        }
    }
    files.sort();
    files
}

/// Project with a top navigation and an unlinked "Contact Us" item whose
/// destination question is pending.
fn with_item() -> (tempfile::TempDir, PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    let dir = init_named(tmp.path(), "p").unwrap();
    modify(&dir, "Need a top navigation").unwrap();
    modify(&dir, "Add Contact Us").unwrap();
    (tmp, dir)
}

fn pending(dir: &Path) -> Vec<Question> {
    load_session(dir, CONV).unwrap().pending_questions
}

fn answer(question: &str, key: &str) -> Answer {
    Answer {
        question_id: question.into(),
        choice: AnswerChoice::Option { key: key.into() },
    }
}

fn plan_of(outcome: PlanOutcome) -> Plan {
    match outcome {
        PlanOutcome::Ready { plan } => plan,
        other => panic!("{other:?}"),
    }
}

// ---------------------------------------------------------------- planner fixtures

#[test]
fn new_page_plan_creates_page_route_and_link_together_and_nothing_else() {
    let (_t, dir) = with_item();
    let q = pending(&dir).remove(0);
    let plan = plan_of(plan_answer(
        &answer(&q.id, "new_page"),
        &model(&dir),
        &load_session(&dir, CONV).unwrap(),
    ));
    assert_eq!(
        serde_json::to_value(&plan.operations).unwrap(),
        json!([
            {"op":"create_page","page":"contact-us","label":"Contact Us"},
            {"op":"register_route","page":"contact-us","path":"/contact-us"},
            {"op":"set_navigation_destination","item":"item_1",
             "destination":{"kind":"page","page":"contact-us"}}
        ])
    );
    assert_eq!(plan.resolves, [q.id]);
    assert!(plan.follow_up.is_empty());
    assert!(plan.preconditions.contains(&Precondition::PageAbsent {
        page: "contact-us".into()
    }));
}

#[test]
fn alternatives_to_a_new_page_plan_no_page_or_route() {
    let (_t, dir) = with_item();
    let (project, session) = (model(&dir), load_session(&dir, CONV).unwrap());
    let q = &session.pending_questions[0];
    for key in ["existing", "external", "unlinked"] {
        let plan = plan_of(plan_answer(&answer(&q.id, key), &project, &session));
        assert!(plan.operations.is_empty(), "{key}: {:?}", plan.operations);
        assert_eq!(plan.resolves, [q.id.clone()]);
        let expected_follow_up = usize::from(key != "unlinked");
        assert_eq!(plan.follow_up.len(), expected_follow_up, "{key}");
    }
    // Existing destinations offer the home page and its anchorable hero.
    let plan = plan_of(plan_answer(&answer(&q.id, "existing"), &project, &session));
    let keys: Vec<_> = plan.follow_up[0]
        .options
        .iter()
        .map(|o| o.key.as_str())
        .collect();
    assert_eq!(keys, ["page:home", "section:home:hero_1"]);
    assert_eq!(plan.follow_up[0].kind, QuestionKind::BlockingClarification);
}

#[test]
fn linking_to_a_created_page_needs_its_route() {
    let mut project = ProjectSnapshot::empty();
    project.pages.push(PageRecord {
        id: "about".into(),
        label: "About".into(),
        path: "/about".into(),
        component: "AboutPage".into(),
        registered: false,
    });
    assert_eq!(
        link_targets(&project).len(),
        1,
        "unregistered pages are not link targets"
    );
    project.pages[0].registered = true;
    let keys: Vec<_> = link_targets(&project).into_iter().map(|t| t.key).collect();
    assert_eq!(keys, ["page:home", "page:about"]);
    // Collisions consider both slug and derived component name.
    assert!(conflicting_page(&project, "about").is_some());
    assert!(conflicting_page(&project, "home").is_some());
    assert!(conflicting_page(&project, "contact").is_none());
    assert_eq!(
        alternative_slug(&project, "about").as_deref(),
        Some("about-2")
    );
    project.pages.push(PageRecord {
        id: "about-2".into(),
        label: "About".into(),
        path: "/about-2".into(),
        component: "About2Page".into(),
        registered: true,
    });
    assert_eq!(
        alternative_slug(&project, "about").as_deref(),
        Some("about-3")
    );
    // `a-1` and `a1` would both be A1Page.
    let mut p = ProjectSnapshot::empty();
    p.pages.push(PageRecord {
        id: "a1".into(),
        label: "A1".into(),
        path: "/a1".into(),
        component: "A1Page".into(),
        registered: true,
    });
    assert_eq!(conflicting_page(&p, "a-1").unwrap().id, "a1");
}

#[test]
fn answers_to_unknown_stale_or_unanswerable_questions_are_conflicts() {
    let (_t, dir) = with_item();
    let (project, mut session) = (model(&dir), load_session(&dir, CONV).unwrap());
    let q = session.pending_questions[0].clone();
    let conflict_of = |a: &Answer, p: &ProjectSnapshot, s: &Session| match plan_answer(a, p, s) {
        PlanOutcome::Conflict { reason } => reason,
        other => panic!("{other:?}"),
    };
    assert!(
        conflict_of(&answer("q9-nope", "unlinked"), &project, &session).contains("not pending")
    );
    // Target deleted.
    let mut gone = project.clone();
    gone.elements.retain(|e| e.id.0 != "item_1");
    assert!(conflict_of(&answer(&q.id, "new_page"), &gone, &session).contains("no longer exists"));
    // Target already linked: a stale answer must not override it.
    let mut linked = project.clone();
    linked
        .elements
        .iter_mut()
        .find(|e| e.id.0 == "item_1")
        .unwrap()
        .destination = Some(Destination::External {
        url: "https://example.com".into(),
    });
    assert!(conflict_of(&answer(&q.id, "new_page"), &linked, &session).contains("already leads"));
    // A target kind swap is not an item.
    let mut wrong = project.clone();
    wrong
        .elements
        .iter_mut()
        .find(|e| e.id.0 == "item_1")
        .unwrap()
        .kind = ElementKind::Image;
    assert!(matches!(
        plan_answer(&answer(&q.id, "new_page"), &wrong, &session),
        PlanOutcome::Conflict { .. }
    ));
    // Questions without an item cannot be answered by ID.
    session.pending_questions.push(Question {
        id: "q1-boundary".into(),
        kind: QuestionKind::BlockingClarification,
        target: None,
        prompt: "?".into(),
        options: vec![],
        continuation: Continuation::Rephrase {
            suggestion: "x".into(),
        },
    });
    assert!(matches!(
        plan_answer(&answer("q1-boundary", "x"), &project, &session),
        PlanOutcome::Unsupported { .. }
    ));
    // A section that no longer exists is stale, not silently relinked.
    let ask = plan_of(plan_answer(&answer(&q.id, "existing"), &project, &session));
    let mut with_question = session.clone();
    with_question.pending_questions = ask.follow_up.clone();
    let mut no_hero = project.clone();
    no_hero.elements.retain(|e| e.kind != ElementKind::Hero);
    assert!(matches!(
        plan_answer(
            &answer(&ask.follow_up[0].id, "section:home:hero_1"),
            &no_hero,
            &with_question
        ),
        PlanOutcome::Conflict { .. }
    ));
}

#[test]
fn invalid_option_choices_reask_without_changing_anything() {
    let (_t, dir) = with_item();
    let (project, session) = (model(&dir), load_session(&dir, CONV).unwrap());
    let q = &session.pending_questions[0];
    for choice in [
        AnswerChoice::Option {
            key: "bogus".into(),
        },
        AnswerChoice::Text { text: "7".into() },
        AnswerChoice::Text { text: "0".into() },
        AnswerChoice::Text {
            text: "banana".into(),
        },
    ] {
        let outcome = plan_answer(
            &Answer {
                question_id: q.id.clone(),
                choice,
            },
            &project,
            &session,
        );
        let PlanOutcome::NeedsClarification { questions } = outcome else {
            panic!("{outcome:?}")
        };
        assert_eq!(questions[0].id, q.id);
        assert!(questions[0].prompt.contains("not one of the options"));
    }
}

// ---------------------------------------------------------------- bare answers

fn bare(dir: &Path, prompt: &str) -> BareAnswer {
    interpret_bare(prompt, &model(dir), &load_session(dir, CONV).unwrap())
}

#[test]
fn bare_answers_resolve_only_when_unambiguous() {
    let (_t, dir) = with_item();
    let id = pending(&dir)[0].id.clone();
    let expect = |prompt: &str, key: &str| {
        assert_eq!(
            bare(&dir, prompt),
            BareAnswer::Answer(answer(&id, key)),
            "{prompt:?}"
        );
    };
    expect("1", "new_page");
    expect(" 4 ", "unlinked");
    expect("new page", "new_page");
    expect("New_Page", "new_page");
    expect("A new page.", "new_page");
    expect("an external URL", "external");
    expect("leave unlinked", "unlinked");
    expect("existing page or section", "existing");
    // Not options: treated as ordinary commands.
    for prompt in ["5", "0", "maybe", "Add Get in Touch", "yes", "no"] {
        assert_eq!(bare(&dir, prompt), BareAnswer::NotAnAnswer, "{prompt:?}");
    }
    // Several pending questions: the same reply is ambiguous, never arbitrary.
    modify(&dir, "Add Get in Touch").unwrap();
    assert_eq!(pending(&dir).len(), 2);
    for prompt in ["1", "new page", "leave unlinked"] {
        let BareAnswer::Ambiguous(qs) = bare(&dir, prompt) else {
            panic!("{prompt}")
        };
        assert_eq!(qs.len(), 2);
    }
    // An explicit question ID disambiguates.
    let first = pending(&dir)[0].id.clone();
    assert_eq!(
        bare(&dir, &format!("{first}: 4")),
        BareAnswer::Answer(Answer {
            question_id: first,
            choice: AnswerChoice::Text { text: "4".into() }
        })
    );
    // No pending questions at all: nothing is an answer.
    let tmp = tempfile::tempdir().unwrap();
    let fresh = init_named(tmp.path(), "q").unwrap();
    assert_eq!(bare(&fresh, "1"), BareAnswer::NotAnAnswer);
}

#[test]
fn yes_and_no_apply_only_to_a_single_yes_no_question() {
    let (_t, dir) = with_item();
    let project = model(&dir);
    let item = ElementId::new("item_1");
    let yes_no = |id: &str| Question {
        id: id.into(),
        kind: QuestionKind::BlockingClarification,
        target: Some(item.clone()),
        prompt: "Link it?".into(),
        options: vec![
            QuestionOption {
                key: "yes".into(),
                label: "Yes".into(),
            },
            QuestionOption {
                key: "no".into(),
                label: "No".into(),
            },
        ],
        continuation: Continuation::ChooseDestination { item: item.clone() },
    };
    let mut session = load_session(&dir, CONV).unwrap();
    // One yes/no question beside the destination question: "yes" is its answer.
    session.add_pending(yes_no("q8-link"));
    assert_eq!(
        interpret_bare("Yes!", &project, &session),
        BareAnswer::Answer(answer("q8-link", "yes"))
    );
    assert_eq!(
        interpret_bare("no", &project, &session),
        BareAnswer::Answer(answer("q8-link", "no"))
    );
    // Two yes/no questions: ambiguous.
    session.add_pending(yes_no("q9-link"));
    assert!(matches!(
        interpret_bare("yes", &project, &session),
        BareAnswer::Ambiguous(qs) if qs.len() == 2
    ));
}

#[test]
fn free_text_answers_only_the_single_text_question_and_never_hijacks_commands() {
    let (_t, dir) = with_item();
    modify(&dir, "3").unwrap();
    let url_q = pending(&dir)
        .into_iter()
        .find(|q| q.continuation.takes_text())
        .unwrap();
    assert_eq!(
        bare(&dir, "https://example.com"),
        BareAnswer::Answer(Answer {
            question_id: url_q.id.clone(),
            choice: AnswerChoice::Text {
                text: "https://example.com".into()
            }
        })
    );
    // A recognizable command proceeds independently while the question waits.
    assert_eq!(bare(&dir, "Add Get in Touch"), BareAnswer::NotAnAnswer);
    assert_eq!(bare(&dir, "do not add a nav"), BareAnswer::NotAnAnswer);
    let r = modify(&dir, "Add Get in Touch").unwrap();
    assert!(matches!(r.outcome, ModifyOutcome::Applied { .. }), "{r:?}");
    assert!(pending(&dir).iter().any(|q| q.id == url_q.id));
    // With two text questions free text is no longer assigned to either.
    modify(&dir, "q3-destination: 3").unwrap();
    let texts = pending(&dir)
        .iter()
        .filter(|q| q.continuation.takes_text())
        .count();
    assert_eq!(texts, 2);
    assert_eq!(bare(&dir, "https://example.com"), BareAnswer::NotAnAnswer);
}

// ---------------------------------------------------------------- application

#[test]
fn applying_the_new_page_plan_twice_creates_one_page_and_one_route() {
    let (_t, dir) = with_item();
    let (project, session) = (model(&dir), load_session(&dir, CONV).unwrap());
    let q = &session.pending_questions[0];
    let plan = plan_of(plan_answer(&answer(&q.id, "new_page"), &project, &session));
    let opts = ApplyOptions::default();
    assert!(matches!(
        apply_plan(&dir, CONV, &plan, &opts).unwrap(),
        ApplyOutcome::Applied(_)
    ));
    let after = disk(&dir);
    assert!(matches!(
        apply_plan(&dir, CONV, &plan, &opts).unwrap(),
        ApplyOutcome::Replayed(_)
    ));
    assert_eq!(disk(&dir), after);
    let router = fs::read_to_string(dir.join("src/router.ts")).unwrap();
    assert_eq!(router.matches("/contact-us").count(), 1, "{router}");
    let m = model(&dir);
    assert_eq!(m.pages.len(), 1);
    assert!(m.pages[0].registered);
    assert!(verify_owned(&dir, &m).unwrap().is_empty());
    assert!(pending(&dir).is_empty());
    // The same plan against a changed project is a conflict, not a replay.
    let mut stale = plan.clone();
    stale.id = "other".into();
    assert!(matches!(
        apply_plan(&dir, CONV, &stale, &opts).unwrap(),
        ApplyOutcome::Conflict { .. }
    ));
    assert_eq!(disk(&dir), after);
}

#[test]
fn answer_request_ids_replay_and_do_not_reapply() {
    let (_t, dir) = with_item();
    let q = pending(&dir).remove(0);
    let options = ModifyOptions {
        request_id: Some("ans-1"),
        ..Default::default()
    };
    let a = answer(&q.id, "new_page");
    let first = answer_with(&dir, &a, &options).unwrap();
    assert!(
        matches!(first.outcome, ModifyOutcome::Applied { .. }),
        "{first:?}"
    );
    let after = disk(&dir);
    let retry = answer_with(&dir, &a, &options).unwrap();
    assert!(
        matches!(retry.outcome, ModifyOutcome::Applied { .. }),
        "{retry:?}"
    );
    assert!(retry.summary.contains("already applied"));
    assert_eq!(disk(&dir), after);
    // The same request ID for a different answer is a conflict.
    let other = answer_with(&dir, &answer(&q.id, "unlinked"), &options).unwrap();
    assert!(
        matches!(other.outcome, ModifyOutcome::Conflict { .. }),
        "{other:?}"
    );
    assert_eq!(disk(&dir), after);
    // A deliberate repeat (new request ID) finds the question gone.
    let again = answer_with(
        &dir,
        &a,
        &ModifyOptions {
            request_id: Some("ans-2"),
            ..Default::default()
        },
    )
    .unwrap();
    assert!(
        matches!(again.outcome, ModifyOutcome::Conflict { .. }),
        "{again:?}"
    );
    assert_eq!(disk(&dir), after);
}

#[test]
fn rejected_and_previewed_answers_write_nothing() {
    let (_t, dir) = with_item();
    let q = pending(&dir).remove(0);
    let before = disk(&dir);
    let dry = ModifyOptions {
        dry_run: true,
        ..Default::default()
    };
    for (a, kind) in [
        (answer(&q.id, "new_page"), "preview"),
        (answer(&q.id, "bogus"), "needs_clarification"),
        (answer("q7-gone", "new_page"), "conflict"),
    ] {
        for options in [&dry, &ModifyOptions::default()] {
            if kind != "preview" || options.dry_run {
                let r = answer_with(&dir, &a, options).unwrap();
                let got = serde_json::to_value(&r.outcome).unwrap()["kind"].clone();
                assert_eq!(got, kind, "{r:?}");
                assert_eq!(disk(&dir), before);
            }
        }
    }
    assert_eq!(pending(&dir).len(), 1);
}

#[test]
fn external_edit_of_the_router_region_is_a_conflict_and_nothing_is_written() {
    let (_t, dir) = with_item();
    let router = dir.join("src/router.ts");
    let text = fs::read_to_string(&router).unwrap();
    fs::write(
        &router,
        text.replace(
            "// protopie:begin routes\n",
            "// protopie:begin routes\n\t\t{ path: \"/mine\", component: lazy(() => import(\"./pages/Home\")) },\n",
        ),
    )
    .unwrap();
    // The template markers were seeded when the project was created, so this
    // is drift of owned source.
    let before = disk(&dir);
    let q = pending(&dir).remove(0);
    let r = answer_with(&dir, &answer(&q.id, "new_page"), &ModifyOptions::default()).unwrap();
    let ModifyOutcome::Conflict { reason } = &r.outcome else {
        panic!("{r:?}")
    };
    assert!(reason.contains("src/router.ts"), "{reason}");
    assert_eq!(disk(&dir), before);
    assert_eq!(pending(&dir).len(), 1);
}

#[test]
fn router_without_routes_markers_gets_an_actionable_conflict() {
    let (_t, dir) = with_item();
    // A project generated before the routes region existed: no markers, and
    // (as the seeding was by an older version) no recorded router ownership.
    let router = dir.join("src/router.ts");
    let text = fs::read_to_string(&router).unwrap();
    fs::write(
        &router,
        text.replace("// protopie:begin routes\n", "")
            .replace("// protopie:end routes\n", ""),
    )
    .unwrap();
    let mut m = model(&dir);
    m.owned.retain(|o| o.path != "src/router.ts");
    crate::project::save_project(&dir, &m).unwrap();
    let before = disk(&dir);
    let q = pending(&dir).remove(0);
    let r = answer_with(&dir, &answer(&q.id, "new_page"), &ModifyOptions::default()).unwrap();
    let ModifyOutcome::Conflict { reason } = &r.outcome else {
        panic!("{r:?}")
    };
    assert!(
        reason.contains("routes region") && reason.contains("add the markers"),
        "{reason}"
    );
    assert_eq!(disk(&dir), before);
    // Adding the markers by hand is the supported migration.
    fs::write(&router, text).unwrap();
    let r = answer_with(&dir, &answer(&q.id, "new_page"), &ModifyOptions::default()).unwrap();
    assert!(matches!(r.outcome, ModifyOutcome::Applied { .. }), "{r:?}");
    assert!(verify_owned(&dir, &model(&dir)).unwrap().is_empty());
}

#[test]
fn interrupted_page_application_recovers_consistently_at_every_point() {
    // Journal order: page tsx, page css, router, nav, model, session, ledger.
    for fail_at in 0..7 {
        let (_t, dir) = with_item();
        let (project, session) = (model(&dir), load_session(&dir, CONV).unwrap());
        let q = &session.pending_questions[0];
        let plan = plan_of(plan_answer(&answer(&q.id, "new_page"), &project, &session));
        let options = ApplyOptions {
            fail_after_writes: Some(fail_at),
            ..Default::default()
        };
        assert!(
            apply_plan(&dir, CONV, &plan, &options).is_err(),
            "{fail_at}"
        );
        assert_eq!(
            crate::apply::recover(&dir).unwrap(),
            crate::apply::RecoverOutcome::RolledForward {
                plan_id: plan.id.clone()
            }
        );
        let m = model(&dir);
        assert_eq!(m.pages.len(), 1, "{fail_at}");
        assert!(verify_owned(&dir, &m).unwrap().is_empty(), "{fail_at}");
        assert!(pending(&dir).is_empty(), "{fail_at}");
        let nav = fs::read_to_string(dir.join("src/components/TopNav.tsx")).unwrap();
        assert!(nav.contains("href={\"/contact-us\"}"), "{fail_at}");
        assert!(dir.join("src/pages/ContactUsPage.tsx").is_file());
        assert!(fs::read_to_string(dir.join("src/router.ts"))
            .unwrap()
            .contains("/contact-us"));
    }
}

#[test]
fn page_operations_reject_hostile_ids_paths_and_labels() {
    let (_t, dir) = with_item();
    let project = model(&dir);
    let base = |ops: Vec<Operation>| Plan {
        id: "plan-x".into(),
        schema_version: PLAN_SCHEMA_VERSION,
        policy_version: POLICY_VERSION,
        base_revision: project.revision,
        preconditions: vec![],
        operations: ops,
        follow_up: vec![],
        resolves: vec![],
    };
    let before = disk(&dir);
    let create = |page: &str, label: &str| Operation::CreatePage {
        page: page.into(),
        label: label.into(),
    };
    for page in ["../x", "a/b", "home", "A", "", "a--b", "-a", "a b", "x_y"] {
        let out = prepare(&dir, &base(vec![create(page, "Label")]), CONV).unwrap();
        assert!(
            matches!(out, PrepareOutcome::Unsupported { .. }),
            "{page}: {out:?}"
        );
    }
    for label in ["", "   ", &"x".repeat(81)] {
        let out = prepare(&dir, &base(vec![create("ok", label)]), CONV).unwrap();
        assert!(
            matches!(out, PrepareOutcome::Unsupported { .. }),
            "{label:?}"
        );
    }
    // A route for an unknown page, or one that disagrees with the page path.
    for ops in [
        vec![Operation::RegisterRoute {
            page: "ghost".into(),
            path: "/ghost".into(),
        }],
        vec![
            create("ok", "Ok"),
            Operation::RegisterRoute {
                page: "ok".into(),
                path: "/../etc".into(),
            },
        ],
    ] {
        let out = prepare(&dir, &base(ops), CONV).unwrap();
        assert!(
            matches!(
                out,
                PrepareOutcome::Conflict { .. } | PrepareOutcome::Unsupported { .. }
            ),
            "{out:?}"
        );
    }
    // A hostile destination.
    for destination in [
        Destination::External {
            url: "javascript:alert(1)".into(),
        },
        Destination::External {
            url: " https://example.com ".into(),
        },
        Destination::Page {
            page: "ghost".into(),
        },
        Destination::Section {
            page: "home".into(),
            section: "../x".into(),
        },
        Destination::Section {
            page: "home".into(),
            section: "nav_1".into(),
        },
        Destination::Unresolved,
    ] {
        let out = prepare(
            &dir,
            &base(vec![Operation::SetNavigationDestination {
                item: ElementId::new("item_1"),
                destination: destination.clone(),
            }]),
            CONV,
        )
        .unwrap();
        assert!(
            matches!(
                out,
                PrepareOutcome::Conflict { .. } | PrepareOutcome::Unsupported { .. }
            ),
            "{destination:?}: {out:?}"
        );
    }
    // An existing file at the page's path is never overwritten.
    fs::write(dir.join("src/pages/OkPage.tsx"), "mine").unwrap();
    let out = prepare(&dir, &base(vec![create("ok", "Ok")]), CONV).unwrap();
    assert!(matches!(out, PrepareOutcome::Conflict { .. }), "{out:?}");
    fs::remove_file(dir.join("src/pages/OkPage.tsx")).unwrap();
    assert_eq!(disk(&dir), before);
}

#[test]
fn hostile_labels_are_escaped_in_generated_page_and_nav_source() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = init_named(tmp.path(), "p").unwrap();
    modify(&dir, "Add a \"Say \\\"hi\\\" </h1>{x}\" page").unwrap();
    let page = fs::read_to_string(dir.join("src/pages/SayHiH1XPage.tsx")).unwrap();
    assert!(
        page.contains(r#"{"Say \"hi\" \u003c/h1\u003e\u007bx\u007d"}"#),
        "{page}"
    );
    assert!(!page.contains("</h1>{x}"));
    // Markup in a nav label is escaped; the derived path keeps only letters and digits.
    modify(&dir, "Need a top navigation").unwrap();
    modify(&dir, "Add \"<script>\" to top nav").unwrap();
    modify(&dir, "new page").unwrap();
    let nav = fs::read_to_string(dir.join("src/components/TopNav.tsx")).unwrap();
    assert!(
        nav.contains(r#"href={"/script"}>{"\u003cscript\u003e"}</a>"#),
        "{nav}"
    );
}

#[test]
fn fuzz_answers_never_panic_and_rejections_never_write() {
    let (_t, dir) = with_item();
    let long = "a".repeat(5000);
    let inputs = [
        "",
        " ",
        ":",
        "::",
        "q1-destination:",
        "q1-destination: ",
        "q0-x: 1",
        "q1: 1",
        "1:1",
        "99999999999999999999",
        "-1",
        "0x1",
        "\u{0}",
        "\u{202e}new page",
        long.as_str(),
        "yes",
        "no",
        "NEW PAGE!!!",
        "q1-destination: https://example.com",
        "q1-destination:\n3",
        "http://",
        "https://exa mple.com",
        "q1-destination: new page extra",
    ];
    let mut applied = 0;
    for input in inputs {
        let before = disk(&dir);
        let before_session = load_session(&dir, CONV).unwrap();
        let r = modify(&dir, input).unwrap();
        match r.outcome {
            ModifyOutcome::Applied { .. } => applied += 1,
            _ => {
                assert_eq!(disk(&dir), before, "{input:?}: {r:?}");
                assert_eq!(
                    load_session(&dir, CONV).unwrap(),
                    before_session,
                    "{input:?}"
                );
            }
        }
    }
    // Arbitrary text may legitimately be a supported answer ("q1-destination: 3").
    assert!(applied <= 2, "{applied}");
    let m = model(&dir);
    assert!(verify_owned(&dir, &m).unwrap().is_empty());
    assert!(m.pages.len() <= 1);
}

#[test]
fn explicit_page_creation_is_ensure_style_and_does_not_touch_navigation() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = init_named(tmp.path(), "p").unwrap();
    let first = modify(&dir, "Add a Contact Us page").unwrap();
    assert!(
        matches!(first.outcome, ModifyOutcome::Applied { .. }),
        "{first:?}"
    );
    assert!(first.summary.contains("Registered the route /contact-us"));
    assert!(!dir.join("src/components").exists());
    let session = load_session(&dir, CONV).unwrap();
    assert!(session.focus.is_none() && session.pending_questions.is_empty());
    let before = disk(&dir);
    for prompt in [
        "Add a Contact Us page",
        "add a contact us page",
        "Add a Contact-Us page",
        "Add a Home page",
    ] {
        let r = modify(&dir, prompt).unwrap();
        assert!(
            matches!(r.outcome, ModifyOutcome::NoChange { .. }),
            "{prompt}: {r:?}"
        );
    }
    assert_eq!(disk(&dir), before);
    // An unusable name is explained, not guessed.
    let r = modify(&dir, "Add a \"???\" page").unwrap();
    assert!(
        matches!(r.outcome, ModifyOutcome::Unsupported { .. }),
        "{r:?}"
    );
    assert_eq!(disk(&dir), before);
}
