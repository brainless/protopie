use super::*;
use crate::pipeline::plan_prompt;
use crate::project::{load_project, load_session, save_project, verify_owned, ProjectState};
use std::fs;
use std::path::{Path, PathBuf};

const CONV: &str = DEFAULT_CONVERSATION_ID;

fn fresh() -> (tempfile::TempDir, PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    let project = crate::init_named(tmp.path(), "p").unwrap();
    (tmp, project)
}

fn model(dir: &Path) -> ProjectSnapshot {
    match load_project(dir).unwrap() {
        ProjectState::Loaded(s) => s,
        ProjectState::NoMetadata => panic!("no metadata"),
    }
}

fn nav_plan(dir: &Path) -> Plan {
    match plan_prompt(
        "Add top nav",
        &model(dir),
        &load_session(dir, CONV).unwrap(),
    ) {
        PlanOutcome::Ready { plan } => plan,
        other => panic!("{other:?}"),
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
                files.push((
                    p.strip_prefix(dir).unwrap().to_path_buf(),
                    fs::read(&p).unwrap(),
                ));
            }
        }
    }
    files.sort();
    files
}

fn opts() -> ApplyOptions<'static> {
    ApplyOptions::default()
}

#[test]
fn prepare_renders_changes_and_diff_without_touching_anything() {
    let (_t, dir) = fresh();
    let plan = nav_plan(&dir);
    let before = disk(&dir);
    let PrepareOutcome::Ready(p) = prepare(&dir, &plan, CONV).unwrap() else {
        panic!()
    };
    assert_eq!(disk(&dir), before);
    let paths: Vec<_> = p.files.iter().map(|f| f.path.as_str()).collect();
    assert_eq!(
        paths,
        [
            "src/components/TopNav.tsx",
            "src/components/TopNav.module.css",
            "src/App.tsx"
        ]
    );
    assert!(p.files[2].diff.contains("+") && p.files[2].diff.contains("TopNav"));
    assert!(p.files[0].before.is_none());
    assert_eq!(model(&dir).revision, 0);
    assert_eq!(load_session(&dir, CONV).unwrap().turn, 0);
}

#[test]
fn apply_writes_code_model_and_session_consistently() {
    let (_t, dir) = fresh();
    let plan = nav_plan(&dir);
    let ApplyOutcome::Applied(r) = apply_plan(&dir, CONV, &plan, &opts()).unwrap() else {
        panic!()
    };
    assert_eq!(r.revision, 1);
    assert!(fs::read_to_string(dir.join("src/App.tsx"))
        .unwrap()
        .contains("<TopNav />"));
    assert!(dir.join("src/components/TopNav.tsx").is_file());
    assert!(dir.join("src/components/TopNav.module.css").is_file());
    let m = model(&dir);
    assert_eq!(m.revision, 1);
    assert!(m.element(&ElementId::new("nav_1")).is_some());
    assert!(verify_owned(&dir, &m).unwrap().is_empty());
    let s = load_session(&dir, CONV).unwrap();
    assert_eq!(s.focus, Some(ElementId::new("nav_1")));
    assert!(!dir.join(".protopie/journal.json").exists());
    // Ensure semantics: asking again plans no change.
    assert!(matches!(
        plan_prompt("Add top nav", &m, &s),
        PlanOutcome::NoChange { .. }
    ));
}

#[test]
fn applying_the_same_plan_twice_does_not_duplicate_effects() {
    let (_t, dir) = fresh();
    let plan = nav_plan(&dir);
    apply_plan(&dir, CONV, &plan, &opts()).unwrap();
    let after_first = disk(&dir);
    let again = apply_plan(&dir, CONV, &plan, &opts()).unwrap();
    assert!(
        matches!(again, ApplyOutcome::Replayed(ref r) if r.revision == 1),
        "{again:?}"
    );
    assert_eq!(disk(&dir), after_first);
    assert_eq!(
        fs::read_to_string(dir.join("src/App.tsx"))
            .unwrap()
            .matches("<TopNav />")
            .count(),
        1
    );
}

#[test]
fn request_id_retry_replays_even_after_session_moved_on() {
    let (_t, dir) = fresh();
    let plan = nav_plan(&dir);
    let o = ApplyOptions {
        request_id: Some("req-1"),
        ..Default::default()
    };
    apply_plan(&dir, CONV, &plan, &o).unwrap();
    assert!(matches!(
        apply_plan(&dir, CONV, &plan, &o).unwrap(),
        ApplyOutcome::Replayed(_)
    ));
    // A new request ID cannot silently alias this already applied plan: the
    // ledger would otherwise forget that ID and allow its later reuse.
    let new_request = ApplyOptions {
        request_id: Some("req-2"),
        ..Default::default()
    };
    assert!(matches!(
        apply_plan(&dir, CONV, &plan, &new_request).unwrap(),
        ApplyOutcome::Conflict { .. }
    ));
}

#[test]
fn command_bound_request_replay_checks_payload_under_apply_lock() {
    let (_t, dir) = fresh();
    let plan = nav_plan(&dir);
    let original = ApplyOptions {
        request_id: Some("req-command"),
        command: Some("Add top nav"),
        ..Default::default()
    };
    assert!(matches!(
        apply_plan(&dir, CONV, &plan, &original).unwrap(),
        ApplyOutcome::Applied(_)
    ));
    let before = disk(&dir);
    assert!(matches!(
        apply_plan(&dir, CONV, &plan, &original).unwrap(),
        ApplyOutcome::Replayed(_)
    ));
    let different = ApplyOptions {
        command: Some("Add a top navigation"),
        ..original.clone()
    };
    assert!(matches!(
        apply_plan(&dir, CONV, &plan, &different).unwrap(),
        ApplyOutcome::Conflict { .. }
    ));
    let missing = ApplyOptions {
        command: None,
        ..original
    };
    assert!(matches!(
        apply_plan(&dir, CONV, &plan, &missing).unwrap(),
        ApplyOutcome::Conflict { .. }
    ));
    assert_eq!(disk(&dir), before);
}

#[test]
fn oldest_request_replays_after_more_than_64_applications() {
    let (_t, dir) = fresh();
    apply_plan(&dir, CONV, &nav_plan(&dir), &opts()).unwrap();
    let first_options = crate::ModifyOptions {
        request_id: Some("first-item-request"),
        ..Default::default()
    };
    assert!(matches!(
        crate::modify_with(&dir, "Add First Item to top nav", &first_options)
            .unwrap()
            .outcome,
        ModifyOutcome::Applied { .. }
    ));
    for index in 0..65 {
        assert!(matches!(
            crate::modify(&dir, &format!("Add Item {index} to top nav"))
                .unwrap()
                .outcome,
            ModifyOutcome::Applied { .. }
        ));
    }
    let before = disk(&dir);
    let retry = crate::modify_with(&dir, "Add First Item to top nav", &first_options).unwrap();
    assert!(matches!(retry.outcome, ModifyOutcome::Applied { .. }));
    assert!(retry.summary.contains("already applied"));
    assert_eq!(disk(&dir), before);
    assert_eq!(
        model(&dir)
            .elements
            .iter()
            .filter(|e| e.label.as_deref() == Some("First Item"))
            .count(),
        1
    );
}

#[test]
fn distinct_plans_at_one_revision_and_turn_have_distinct_ids() {
    let (_t, dir) = fresh();
    let project = model(&dir);
    let a = Session::default();
    let mut b = a.clone();
    b.conversation_id = "second".into();
    let plan = |session: &Session| match plan_prompt("Add top nav", &project, session) {
        PlanOutcome::Ready { plan } => plan,
        other => panic!("{other:?}"),
    };
    assert_ne!(plan(&a).id, plan(&b).id);
    assert_eq!(plan(&a).id, plan(&a).id);

    let mut with_nav = project;
    with_nav.elements.push(ElementRecord::new(
        ElementId::new("nav_1"),
        ElementKind::Navigation,
        DEFAULT_PAGE,
    ));
    let item = |label: &str| match plan_prompt(&format!("Add {label} to top nav"), &with_nav, &a) {
        PlanOutcome::Ready { plan } => plan,
        other => panic!("{other:?}"),
    };
    assert_ne!(item("Contact Us").id, item("Pricing").id);
}

#[test]
fn colliding_plan_and_request_ids_fail_closed() {
    let (_t, dir) = fresh();
    let plan = nav_plan(&dir);
    let opts = ApplyOptions {
        request_id: Some("req-1"),
        ..Default::default()
    };
    assert!(matches!(
        apply_plan(&dir, CONV, &plan, &opts).unwrap(),
        ApplyOutcome::Applied(_)
    ));

    let mut different = plan.clone();
    different.operations.pop();
    let before = disk(&dir);
    assert!(matches!(
        apply_plan(&dir, CONV, &different, &opts).unwrap(),
        ApplyOutcome::Conflict { .. }
    ));
    assert!(matches!(
        apply_plan(
            &dir,
            CONV,
            &different,
            &ApplyOptions {
                request_id: Some("req-2"),
                ..Default::default()
            }
        )
        .unwrap(),
        ApplyOutcome::Conflict { .. }
    ));
    different.id = "a-distinct-plan-id".into();
    assert!(matches!(
        apply_plan(&dir, CONV, &different, &opts).unwrap(),
        ApplyOutcome::Conflict { .. }
    ));
    assert!(matches!(
        apply_plan(&dir, "second", &plan, &opts).unwrap(),
        ApplyOutcome::Conflict { .. }
    ));
    assert!(matches!(
        apply_plan(&dir, "second", &plan, &ApplyOptions::default()).unwrap(),
        ApplyOutcome::Conflict { .. }
    ));
    assert_eq!(disk(&dir), before);
}

#[test]
fn old_ledger_record_cannot_authorize_a_replay() {
    let (_t, dir) = fresh();
    let plan = nav_plan(&dir);
    let ledger = serde_json::json!({"entries": [{"plan_id": plan.id, "request_id": "old", "revision": 1, "changed_files": []}]});
    fs::write(
        dir.join(".protopie/applied.json"),
        serde_json::to_string(&ledger).unwrap(),
    )
    .unwrap();
    assert!(matches!(
        apply_plan(&dir, CONV, &plan, &ApplyOptions::default()).unwrap(),
        ApplyOutcome::Conflict { .. }
    ));
    assert!(matches!(
        apply_plan(
            &dir,
            CONV,
            &plan,
            &ApplyOptions {
                request_id: Some("old"),
                ..Default::default()
            }
        )
        .unwrap(),
        ApplyOutcome::Conflict { .. }
    ));
}

#[test]
fn external_edit_to_owned_region_conflicts_and_nothing_is_written() {
    let (_t, dir) = fresh();
    let plan = nav_plan(&dir);
    let app = dir.join("src/App.tsx");
    let text = fs::read_to_string(&app).unwrap().replace(
        "protopie:begin layout-top\n",
        "protopie:begin layout-top\n// manual\n",
    );
    fs::write(&app, text).unwrap();
    // Region now holds an extra line owned by the agent: drift.
    let before = disk(&dir);
    assert!(matches!(
        prepare(&dir, &plan, CONV).unwrap(),
        PrepareOutcome::Conflict { .. }
    ));
    assert!(matches!(
        apply_plan(&dir, CONV, &plan, &opts()).unwrap(),
        ApplyOutcome::Conflict { .. }
    ));
    assert_eq!(disk(&dir), before);
}

#[test]
fn edits_outside_owned_regions_are_preserved() {
    let (_t, dir) = fresh();
    let plan = nav_plan(&dir);
    let app = dir.join("src/App.tsx");
    let text = format!("// mine\n{}", fs::read_to_string(&app).unwrap());
    fs::write(&app, text).unwrap();
    apply_plan(&dir, CONV, &plan, &opts()).unwrap();
    let out = fs::read_to_string(&app).unwrap();
    assert!(out.starts_with("// mine\n") && out.contains("<TopNav />"));
}

#[test]
fn unowned_existing_file_is_never_overwritten() {
    let (_t, dir) = fresh();
    let plan = nav_plan(&dir);
    fs::create_dir_all(dir.join("src/components")).unwrap();
    fs::write(dir.join("src/components/TopNav.tsx"), "mine").unwrap();
    assert!(matches!(
        apply_plan(&dir, CONV, &plan, &opts()).unwrap(),
        ApplyOutcome::Conflict { .. }
    ));
    assert_eq!(
        fs::read_to_string(dir.join("src/components/TopNav.tsx")).unwrap(),
        "mine"
    );
}

#[test]
fn stale_plan_conflicts() {
    let (_t, dir) = fresh();
    let plan = nav_plan(&dir);
    let mut m = model(&dir);
    m.revision += 1;
    save_project(&dir, &m).unwrap();
    let before = disk(&dir);
    assert!(matches!(
        apply_plan(&dir, CONV, &plan, &opts()).unwrap(),
        ApplyOutcome::Conflict { .. }
    ));
    assert_eq!(disk(&dir), before);
}

#[test]
fn unsupported_operations_write_nothing() {
    let (_t, dir) = fresh();
    let edit = |value: &str| StyleEdit {
        property: StyleProperty::BorderRadius,
        value: value.into(),
    };
    let set_style = |scope, edits| Operation::SetStyle {
        target: ElementId::new("hero_1"),
        change: crate::parser::StyleChange::RoundedCorners,
        scope,
        edits,
    };
    for op in [
        // Nothing to write, a definition-wide edit, and a value outside the policy.
        set_style(StyleScope::Instance, vec![]),
        set_style(StyleScope::Definition, vec![edit("var(--radius-md)")]),
        set_style(StyleScope::Instance, vec![edit("5px; color: red")]),
        Operation::SetNavigationDestination {
            item: ElementId::new("hero_1"),
            destination: Destination::Unresolved,
        },
    ] {
        let mut plan = nav_plan(&dir);
        plan.operations = vec![op];
        let before = disk(&dir);
        assert!(matches!(
            apply_plan(&dir, CONV, &plan, &opts()).unwrap(),
            ApplyOutcome::Unsupported { .. }
        ));
        assert_eq!(disk(&dir), before);
    }
}

#[test]
fn failing_preconditions_write_nothing() {
    let (_t, dir) = fresh();
    let original = nav_plan(&dir);
    let cases = [
        Precondition::RevisionIs { revision: 1 },
        Precondition::ElementExists {
            id: ElementId::new("missing"),
        },
        Precondition::ElementAbsent {
            id: ElementId::new("hero_1"),
        },
    ];
    for precondition in cases {
        let mut plan = original.clone();
        plan.preconditions = vec![precondition];
        let before = disk(&dir);
        assert!(matches!(
            prepare(&dir, &plan, CONV).unwrap(),
            PrepareOutcome::Conflict { .. }
        ));
        assert!(matches!(
            apply_plan(&dir, CONV, &plan, &opts()).unwrap(),
            ApplyOutcome::Conflict { .. }
        ));
        assert_eq!(disk(&dir), before);
    }
}

#[test]
fn duplicate_id_and_foreign_component_owner_write_nothing() {
    let (_t, dir) = fresh();
    let mut plan = nav_plan(&dir);
    // Bypass the planner's ElementAbsent guard to exercise prepare's own
    // defensive duplicate-ID check on a malformed or older plan.
    plan.preconditions
        .retain(|pre| !matches!(pre, Precondition::ElementAbsent { id } if id.0 == "nav_1"));
    let mut snapshot = model(&dir);
    snapshot.elements.push(ElementRecord::new(
        ElementId::new("nav_1"),
        ElementKind::Navigation,
        DEFAULT_PAGE,
    ));
    save_project(&dir, &snapshot).unwrap();
    let before = disk(&dir);
    assert!(matches!(
        prepare(&dir, &plan, CONV).unwrap(),
        PrepareOutcome::Conflict { reason } if reason.contains("element id nav_1 already exists")
    ));
    assert!(matches!(
        apply_plan(&dir, CONV, &plan, &opts()).unwrap(),
        ApplyOutcome::Conflict { reason } if reason.contains("element id nav_1 already exists")
    ));
    assert_eq!(disk(&dir), before);

    snapshot.elements.pop();
    let mut other = ElementRecord::new(
        ElementId::new("nav_2"),
        ElementKind::Navigation,
        DEFAULT_PAGE,
    );
    other.source = Some(SourceBinding {
        file: "src/components/TopNav.tsx".into(),
        region: None,
    });
    snapshot.elements.push(other);
    save_project(&dir, &snapshot).unwrap();
    let before = disk(&dir);
    assert!(matches!(
        prepare(&dir, &plan, CONV).unwrap(),
        PrepareOutcome::Conflict { reason } if reason.contains("already owned by element nav_2")
    ));
    assert!(matches!(
        apply_plan(&dir, CONV, &plan, &opts()).unwrap(),
        ApplyOutcome::Conflict { reason } if reason.contains("already owned by element nav_2")
    ));
    assert_eq!(disk(&dir), before);
}

#[test]
fn repeated_insertion_writes_nothing() {
    let (_t, dir) = fresh();
    let mut plan = nav_plan(&dir);
    let insertion = plan
        .operations
        .iter()
        .find(|op| matches!(op, Operation::InsertElement { .. }))
        .unwrap()
        .clone();
    plan.operations.push(insertion);
    let before = disk(&dir);
    assert!(matches!(
        prepare(&dir, &plan, CONV).unwrap(),
        PrepareOutcome::Conflict { .. }
    ));
    assert!(matches!(
        apply_plan(&dir, CONV, &plan, &opts()).unwrap(),
        ApplyOutcome::Conflict { .. }
    ));
    assert_eq!(disk(&dir), before);
}

#[test]
fn injected_write_failure_is_recoverable_at_every_point() {
    let (_t2, clean) = fresh();
    apply_plan(&clean, CONV, &nav_plan(&clean), &opts()).unwrap();
    let expected = disk(&clean);
    for fail_after in 0..6 {
        let (_t, dir) = fresh();
        let plan = nav_plan(&dir);
        let o = ApplyOptions {
            fail_after_writes: Some(fail_after),
            ..Default::default()
        };
        let res = apply_plan(&dir, CONV, &plan, &o);
        assert!(res.is_err(), "fail_after={fail_after}");
        assert!(
            dir.join(".protopie/journal.json").exists(),
            "fail_after={fail_after}"
        );
        let r = recover(&dir).unwrap();
        assert!(matches!(r, RecoverOutcome::RolledForward { .. }), "{r:?}");
        assert_eq!(disk(&dir), expected, "fail_after={fail_after}");
        assert_eq!(recover(&dir).unwrap(), RecoverOutcome::NothingToDo);
        // Retrying the plan after recovery replays rather than duplicating.
        assert!(matches!(
            apply_plan(&dir, CONV, &plan, &opts()).unwrap(),
            ApplyOutcome::Replayed(_)
        ));
        assert_eq!(disk(&dir), expected);
    }
}

#[test]
fn failure_after_last_write_before_journal_removal_recovers() {
    let (_t2, clean) = fresh();
    apply_plan(&clean, CONV, &nav_plan(&clean), &opts()).unwrap();
    let expected = disk(&clean);
    let (_t, dir) = fresh();
    let plan = nav_plan(&dir);
    let options = ApplyOptions {
        fail_before_journal_removal: true,
        ..Default::default()
    };
    assert!(apply_plan(&dir, CONV, &plan, &options).is_err());
    assert!(dir.join(".protopie/journal.json").exists());
    assert!(matches!(
        recover(&dir).unwrap(),
        RecoverOutcome::RolledForward { .. }
    ));
    assert_eq!(disk(&dir), expected);
    assert!(matches!(
        apply_plan(&dir, CONV, &plan, &opts()).unwrap(),
        ApplyOutcome::Replayed(_)
    ));
}

#[test]
fn abort_without_pending_journal_is_read_only() {
    let (_t, dir) = fresh();
    let lock = dir.join(".protopie/lock");
    assert!(!lock.exists());
    let before = disk(&dir);
    assert_eq!(
        abort_interrupted_apply(&dir).unwrap(),
        RecoverOutcome::NothingToDo
    );
    assert_eq!(disk(&dir), before);
    assert!(!lock.exists());
}

#[test]
fn next_apply_recovers_an_interrupted_one_first() {
    let (_t, dir) = fresh();
    let plan = nav_plan(&dir);
    let o = ApplyOptions {
        fail_after_writes: Some(1),
        ..Default::default()
    };
    assert!(apply_plan(&dir, CONV, &plan, &o).is_err());
    assert!(matches!(
        apply_plan(&dir, CONV, &plan, &opts()).unwrap(),
        ApplyOutcome::Replayed(_)
    ));
    assert!(verify_owned(&dir, &model(&dir)).unwrap().is_empty());
}

#[test]
fn recovery_refuses_to_overwrite_edits_made_after_the_failure() {
    let (_t, dir) = fresh();
    let plan = nav_plan(&dir);
    let o = ApplyOptions {
        fail_after_writes: Some(1),
        ..Default::default()
    };
    assert!(apply_plan(&dir, CONV, &plan, &o).is_err());
    // App.tsx was not yet written; the user edits it.
    let app = dir.join("src/App.tsx");
    fs::write(&app, "user edit").unwrap();
    assert!(matches!(
        recover(&dir).unwrap(),
        RecoverOutcome::Conflict { .. }
    ));
    assert_eq!(fs::read_to_string(&app).unwrap(), "user edit");
    assert!(dir.join(".protopie/journal.json").exists());
}

#[test]
fn conflicted_recovery_can_be_aborted_without_losing_external_edit() {
    let (_t, dir) = fresh();
    let plan = nav_plan(&dir);
    let o = ApplyOptions {
        fail_after_writes: Some(1),
        ..Default::default()
    };
    assert!(apply_plan(&dir, CONV, &plan, &o).is_err());
    let app = dir.join("src/App.tsx");
    let external = format!("// user edit\n{}", fs::read_to_string(&app).unwrap());
    fs::write(&app, &external).unwrap();
    assert!(matches!(
        recover(&dir).unwrap(),
        RecoverOutcome::Conflict { .. }
    ));
    assert!(matches!(
        abort_interrupted_apply(&dir).unwrap(),
        RecoverOutcome::Aborted { .. }
    ));
    assert_eq!(fs::read_to_string(&app).unwrap(), external);
    assert!(!dir.join("src/components/TopNav.tsx").exists());
    assert!(!dir.join(".protopie/journal.json").exists());
    assert_eq!(model(&dir).revision, 0);
    assert!(matches!(
        apply_plan(&dir, CONV, &plan, &opts()).unwrap(),
        ApplyOutcome::Applied(_)
    ));
    assert!(fs::read_to_string(&app)
        .unwrap()
        .starts_with("// user edit\n"));
}

#[test]
fn interrupted_abort_resumes_without_overwriting_preserved_edit() {
    let (_t, dir) = fresh();
    let plan = nav_plan(&dir);
    let o = ApplyOptions {
        fail_after_writes: Some(1),
        ..Default::default()
    };
    assert!(apply_plan(&dir, CONV, &plan, &o).is_err());
    let app = dir.join("src/App.tsx");
    let external = format!("// user edit\n{}", fs::read_to_string(&app).unwrap());
    fs::write(&app, &external).unwrap();
    // Simulate a crash after the abort decision was journaled and the first
    // rollback was performed.
    let journal_path = dir.join(".protopie/journal.json");
    let mut journal: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&journal_path).unwrap()).unwrap();
    journal["mode"] = "abort".into();
    journal["preserved_paths"] = serde_json::json!(["src/App.tsx"]);
    fs::write(&journal_path, serde_json::to_string(&journal).unwrap()).unwrap();
    fs::remove_file(dir.join("src/components/TopNav.tsx")).unwrap();
    assert!(matches!(
        recover(&dir).unwrap(),
        RecoverOutcome::Aborted { .. }
    ));
    assert_eq!(fs::read_to_string(&app).unwrap(), external);
    assert_eq!(model(&dir).revision, 0);
    assert!(!journal_path.exists());
}

#[test]
fn resumed_abort_preserves_later_outside_region_edits() {
    for already_rolled_back in [false, true] {
        let (_t, dir) = fresh();
        let plan = nav_plan(&dir);
        let options = ApplyOptions {
            fail_after_writes: Some(3),
            ..Default::default()
        };
        assert!(apply_plan(&dir, CONV, &plan, &options).is_err());
        let app = dir.join("src/App.tsx");
        let first_edit = format!("// first edit\n{}", fs::read_to_string(&app).unwrap());
        fs::write(&app, &first_edit).unwrap();

        // Simulate interruption immediately after the abort journal was saved.
        let journal_path = dir.join(".protopie/journal.json");
        let mut journal: Journal =
            serde_json::from_str(&fs::read_to_string(&journal_path).unwrap()).unwrap();
        journal.mode = JournalMode::Abort;
        journal.preserved_paths.push(LAYOUT_FILE.into());
        let app_entry = journal
            .entries
            .iter_mut()
            .find(|e| e.path == LAYOUT_FILE)
            .unwrap();
        let before_body =
            extract_region(app_entry.before.as_deref().unwrap(), LAYOUT_REGION).unwrap();
        let target = replace_region(&first_edit, LAYOUT_REGION, &before_body).unwrap();
        app_entry.abort_source = Some(first_edit.clone());
        app_entry.abort_target = Some(target.clone());
        fs::write(&journal_path, serde_json::to_string(&journal).unwrap()).unwrap();

        let later_edit = format!(
            "// second edit\n{}",
            if already_rolled_back {
                &target
            } else {
                &first_edit
            }
        );
        fs::write(&app, &later_edit).unwrap();
        assert!(matches!(
            recover(&dir).unwrap(),
            RecoverOutcome::Aborted { .. }
        ));
        let actual = fs::read_to_string(&app).unwrap();
        assert!(actual.starts_with("// second edit\n// first edit\n"));
        assert!(!actual.contains("<TopNav />"));
        assert!(!actual.contains("components/TopNav"));
        assert!(!dir.join("src/components/TopNav.tsx").exists());
        assert_eq!(model(&dir).revision, 0);
        assert!(!journal_path.exists());
    }
}

#[test]
fn modify_reports_recovery_conflict() {
    let (_t, dir) = fresh();
    let plan = nav_plan(&dir);
    let o = ApplyOptions {
        fail_after_writes: Some(1),
        ..Default::default()
    };
    assert!(apply_plan(&dir, CONV, &plan, &o).is_err());
    fs::write(dir.join("src/App.tsx"), "user edit").unwrap();
    let result = crate::modify(&dir, "Add top nav").unwrap();
    assert!(matches!(result.outcome, ModifyOutcome::Conflict { .. }));
    assert!(result.summary.contains("/abort-interrupted-apply"));
}

#[test]
fn modify_does_not_recover_or_write_a_pending_journal() {
    let (_t, dir) = fresh();
    let plan = nav_plan(&dir);
    let o = ApplyOptions {
        fail_after_writes: Some(1),
        ..Default::default()
    };
    assert!(apply_plan(&dir, CONV, &plan, &o).is_err());
    let before = disk(&dir);
    let result = crate::modify(&dir, "Add top nav").unwrap();
    assert!(matches!(result.outcome, ModifyOutcome::Conflict { .. }));
    assert_eq!(disk(&dir), before);
}

#[test]
fn abort_preserves_outside_region_edit_and_removes_staged_navigation() {
    let (_t, dir) = fresh();
    let plan = nav_plan(&dir);
    // Component files and App.tsx are written before this interruption; model and ledger are not.
    let o = ApplyOptions {
        fail_after_writes: Some(3),
        ..Default::default()
    };
    assert!(apply_plan(&dir, CONV, &plan, &o).is_err());
    let app = dir.join("src/App.tsx");
    let with_user_edit = format!("// user edit\n{}", fs::read_to_string(&app).unwrap());
    fs::write(&app, with_user_edit).unwrap();
    assert!(matches!(
        recover(&dir).unwrap(),
        RecoverOutcome::Conflict { .. }
    ));
    assert!(matches!(
        abort_interrupted_apply(&dir).unwrap(),
        RecoverOutcome::Aborted { .. }
    ));
    let result = fs::read_to_string(&app).unwrap();
    assert!(result.starts_with("// user edit\n"));
    assert!(!result.contains("<TopNav />"));
    assert!(!result.contains("components/TopNav"));
    assert!(!dir.join("src/components/TopNav.tsx").exists());
    assert_eq!(model(&dir).revision, 0);
    assert!(verify_owned(&dir, &model(&dir)).unwrap().is_empty());
}

#[test]
fn abort_rejects_edits_to_staged_owned_region() {
    let (_t, dir) = fresh();
    let plan = nav_plan(&dir);
    let o = ApplyOptions {
        fail_after_writes: Some(3),
        ..Default::default()
    };
    assert!(apply_plan(&dir, CONV, &plan, &o).is_err());
    let app = dir.join("src/App.tsx");
    let edited = fs::read_to_string(&app)
        .unwrap()
        .replace("<TopNav />", "<UserNav />");
    fs::write(&app, &edited).unwrap();
    assert!(matches!(
        abort_interrupted_apply(&dir).unwrap(),
        RecoverOutcome::Conflict { .. }
    ));
    assert_eq!(fs::read_to_string(&app).unwrap(), edited);
    assert!(dir.join(".protopie/journal.json").exists());
}

#[cfg(unix)]
#[test]
fn predictable_temp_symlink_is_not_followed() {
    let (t, dir) = fresh();
    let outside = t.path().join("outside.txt");
    fs::write(&outside, "sentinel").unwrap();
    let old_temp = dir.join(".protopie/journal.json.protopie-tmp");
    std::os::unix::fs::symlink(&outside, &old_temp).unwrap();
    assert!(matches!(
        apply_plan(&dir, CONV, &nav_plan(&dir), &opts()).unwrap(),
        ApplyOutcome::Applied(_)
    ));
    assert_eq!(fs::read_to_string(&outside).unwrap(), "sentinel");
    assert!(old_temp.is_symlink());
}

#[cfg(unix)]
#[test]
fn dangling_lock_symlink_cannot_create_outside_file() {
    let (t, dir) = fresh();
    let outside = t.path().join("outside.txt");
    std::os::unix::fs::symlink(&outside, dir.join(".protopie/lock")).unwrap();
    assert!(apply_plan(&dir, CONV, &nav_plan(&dir), &opts()).is_err());
    assert!(!outside.exists());
}

#[cfg(unix)]
#[test]
fn journal_target_symlink_cannot_write_outside_during_recovery() {
    let (t, dir) = fresh();
    let plan = nav_plan(&dir);
    let o = ApplyOptions {
        fail_after_writes: Some(0),
        ..Default::default()
    };
    assert!(apply_plan(&dir, CONV, &plan, &o).is_err());
    let outside = t.path().join("outside.txt");
    fs::write(&outside, "sentinel").unwrap();
    let target = dir.join("src/components/TopNav.tsx");
    fs::create_dir_all(target.parent().unwrap()).unwrap();
    std::os::unix::fs::symlink(&outside, &target).unwrap();
    assert!(recover(&dir).is_err());
    assert_eq!(fs::read_to_string(&outside).unwrap(), "sentinel");
}

#[test]
fn aliased_metadata_path_cannot_be_preserved_by_abort() {
    let (_t, dir) = fresh();
    let plan = nav_plan(&dir);
    let o = ApplyOptions {
        fail_after_writes: Some(1),
        ..Default::default()
    };
    assert!(apply_plan(&dir, CONV, &plan, &o).is_err());
    let journal_path = dir.join(".protopie/journal.json");
    let mut journal: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&journal_path).unwrap()).unwrap();
    let entry = journal["entries"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|e| e["path"] == ".protopie/model.json")
        .unwrap();
    entry["path"] = "./.protopie/model.json".into();
    fs::write(&journal_path, serde_json::to_string(&journal).unwrap()).unwrap();
    assert!(abort_interrupted_apply(&dir).is_err());
    assert!(journal_path.exists());
}

#[test]
fn tampered_journal_paths_cannot_escape_the_project() {
    let (t, dir) = fresh();
    let plan = nav_plan(&dir);
    let o = ApplyOptions {
        fail_after_writes: Some(1),
        ..Default::default()
    };
    assert!(apply_plan(&dir, CONV, &plan, &o).is_err());
    let jp = dir.join(".protopie/journal.json");
    let text = fs::read_to_string(&jp)
        .unwrap()
        .replace("src/App.tsx", "../escaped.txt");
    fs::write(&jp, text).unwrap();
    assert!(matches!(recover(&dir), Err(crate::Error::Escape(_))));
    assert!(!t.path().join("escaped.txt").exists());
}

#[cfg(unix)]
#[test]
fn metadata_writes_cannot_escape_through_a_symlinked_protopie_dir() {
    let (t, dir) = fresh();
    let plan = nav_plan(&dir);
    let outside = t.path().join("outside");
    fs::create_dir(&outside).unwrap();
    fs::remove_dir_all(dir.join(".protopie")).unwrap();
    std::os::unix::fs::symlink(&outside, dir.join(".protopie")).unwrap();
    assert!(apply_plan(&dir, CONV, &plan, &opts()).is_err());
    assert_eq!(fs::read_dir(&outside).unwrap().count(), 0);
}

#[test]
fn project_lock_is_exclusive() {
    let (_t, dir) = fresh();
    let held = ProjectLock::acquire(&dir).unwrap();
    assert!(ProjectLock::try_acquire(&dir).unwrap().is_none());
    drop(held);
    assert!(ProjectLock::try_acquire(&dir).unwrap().is_some());
}

#[test]
fn concurrent_applies_of_one_plan_take_effect_once() {
    let (_t, dir) = fresh();
    let plan = nav_plan(&dir);
    let results: Vec<_> = std::thread::scope(|s| {
        let hs: Vec<_> = (0..4)
            .map(|_| s.spawn(|| apply_plan(&dir, CONV, &plan, &ApplyOptions::default()).unwrap()))
            .collect();
        hs.into_iter().map(|h| h.join().unwrap()).collect()
    });
    let applied = results
        .iter()
        .filter(|r| matches!(r, ApplyOutcome::Applied(_)))
        .count();
    let replayed = results
        .iter()
        .filter(|r| matches!(r, ApplyOutcome::Replayed(_)))
        .count();
    assert_eq!((applied, replayed), (1, 3), "{results:?}");
    assert_eq!(model(&dir).revision, 1);
}

fn item_plan(dir: &Path, destination: Destination, navigation: &str) -> Plan {
    let mut plan = Plan {
        id: "plan-item".into(),
        schema_version: PLAN_SCHEMA_VERSION,
        policy_version: POLICY_VERSION,
        base_revision: model(dir).revision,
        preconditions: vec![Precondition::RevisionIs {
            revision: model(dir).revision,
        }],
        operations: Vec::new(),
        follow_up: Vec::new(),
        resolves: Vec::new(),
    };
    plan.operations = vec![Operation::AddNavigationItem {
        navigation: ElementId::new(navigation),
        item: ElementId::new("item_1"),
        label: "Contact Us".into(),
        destination,
    }];
    plan.follow_up = Vec::new();
    plan
}

#[test]
fn navigation_item_plans_that_cannot_hold_write_nothing() {
    let (_t, dir) = fresh();
    apply_plan(&dir, CONV, &nav_plan(&dir), &opts()).unwrap();
    let before = disk(&dir);
    // A malformed external destination is never applied.
    let linked = item_plan(
        &dir,
        Destination::External {
            url: "javascript:alert(1)".into(),
        },
        "nav_1",
    );
    assert!(matches!(
        apply_plan(&dir, CONV, &linked, &opts()).unwrap(),
        ApplyOutcome::Unsupported { .. }
    ));
    // Missing navigation, and a navigation that is not one.
    for target in ["nav_9", "hero_1"] {
        let plan = item_plan(&dir, Destination::Unresolved, target);
        let out = apply_plan(&dir, CONV, &plan, &opts()).unwrap();
        assert!(
            matches!(
                out,
                ApplyOutcome::Conflict { .. } | ApplyOutcome::Unsupported { .. }
            ),
            "{target}: {out:?}"
        );
    }
    assert_eq!(disk(&dir), before);
}

#[test]
fn navigation_item_plan_updates_model_session_and_only_the_component_file() {
    let (_t, dir) = fresh();
    apply_plan(&dir, CONV, &nav_plan(&dir), &opts()).unwrap();
    let app_before = fs::read(dir.join("src/App.tsx")).unwrap();
    let plan = item_plan(&dir, Destination::Unresolved, "nav_1");
    let PrepareOutcome::Ready(p) = prepare(&dir, &plan, CONV).unwrap() else {
        panic!()
    };
    assert_eq!(
        p.files.iter().map(|f| f.path.as_str()).collect::<Vec<_>>(),
        ["src/components/TopNav.tsx"]
    );
    let ApplyOutcome::Applied(r) = apply_plan(&dir, CONV, &plan, &opts()).unwrap() else {
        panic!()
    };
    assert_eq!(r.changed_files, ["src/components/TopNav.tsx"]);
    assert_eq!(fs::read(dir.join("src/App.tsx")).unwrap(), app_before);
    let m = model(&dir);
    assert_eq!(m.revision, 2);
    assert!(verify_owned(&dir, &m).unwrap().is_empty());
    assert_eq!(
        load_session(&dir, CONV).unwrap().focus,
        Some(ElementId::new("nav_1"))
    );
    // Replaying the same plan does not add a second item.
    assert!(matches!(
        apply_plan(&dir, CONV, &plan, &opts()).unwrap(),
        ApplyOutcome::Replayed(_)
    ));
    assert_eq!(
        fs::read_to_string(dir.join("src/components/TopNav.tsx"))
            .unwrap()
            .matches("<li")
            .count(),
        1
    );
}
