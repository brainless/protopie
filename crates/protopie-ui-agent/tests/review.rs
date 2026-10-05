//! Review surface (Epic 001, T9): inspect a prepared change (diff, recorded
//! revision and plan), apply exactly that, answer a question, and see
//! conflicts without losing edits.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use protopie_ui_agent::contracts::{Answer, AnswerChoice, ModifyOutcome, ModifyResult};
use protopie_ui_agent::{answer_with, init_named, modify_with, project, ModifyOptions};

fn fresh() -> (tempfile::TempDir, PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    let dir = init_named(tmp.path(), "p").unwrap();
    (tmp, dir)
}

fn files(dir: &Path) -> BTreeMap<String, Vec<u8>> {
    fn walk(base: &Path, dir: &Path, out: &mut BTreeMap<String, Vec<u8>>) {
        for e in fs::read_dir(dir).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                walk(base, &p, out);
            } else {
                let rel = p.strip_prefix(base).unwrap().to_string_lossy().into_owned();
                // The empty lock file is an artifact of locking, not content.
                if rel != ".protopie/lock" {
                    out.insert(rel, fs::read(&p).unwrap());
                }
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(dir, dir, &mut out);
    out
}

fn preview(dir: &Path, command: &str) -> ModifyResult {
    let options = ModifyOptions {
        dry_run: true,
        ..Default::default()
    };
    let r = modify_with(dir, command, &options).unwrap();
    assert!(matches!(r.outcome, ModifyOutcome::Preview { .. }), "{r:?}");
    r
}

fn apply_previewed(dir: &Path, command: &str, r: &ModifyResult) -> ModifyResult {
    let ModifyOutcome::Preview {
        plan_id,
        base_revision,
        ..
    } = &r.outcome
    else {
        panic!("not a preview")
    };
    modify_with(
        dir,
        command,
        &ModifyOptions {
            expected_revision: Some(*base_revision),
            expected_plan_id: Some(plan_id),
            ..Default::default()
        },
    )
    .unwrap()
}

#[test]
fn preview_shows_diffs_and_revision_and_applying_it_matches() {
    let (_t, dir) = fresh();
    let before = files(&dir);
    let r = preview(&dir, "Need a top navigation");
    assert_eq!(files(&dir), before, "a preview writes nothing");
    let ModifyOutcome::Preview {
        changed_files,
        diffs,
        base_revision,
        ..
    } = &r.outcome
    else {
        unreachable!()
    };
    assert_eq!(
        diffs.iter().map(|d| d.path.clone()).collect::<Vec<_>>(),
        *changed_files
    );
    let nav = diffs.iter().find(|d| d.path.ends_with("TopNav.tsx")).unwrap();
    assert!(nav.diff.starts_with("--- /dev/null\n+++ src/components/TopNav.tsx\n"));
    assert!(nav.diff.contains("\n+"));
    let app = diffs.iter().find(|d| d.path == "src/App.tsx").unwrap();
    assert!(app.diff.contains("<TopNav"), "{}", app.diff);
    let ModifyResult {
        outcome: ModifyOutcome::Applied {
            changed_files: applied,
            ..
        },
        ..
    } = apply_previewed(&dir, "Need a top navigation", &r)
    else {
        panic!("expected applied")
    };
    assert_eq!(&applied, changed_files);
    let m = match project::load_project(&dir).unwrap() {
        project::ProjectState::Loaded(m) => m,
        _ => panic!(),
    };
    assert_eq!(m.revision, base_revision + 1);
    assert!(fs::read_to_string(dir.join("src/App.tsx"))
        .unwrap()
        .contains("TopNav"));
}

#[test]
fn applying_against_a_moved_revision_conflicts_and_writes_nothing() {
    let (_t, dir) = fresh();
    let r = preview(&dir, "Need a top navigation");
    // Something else lands first.
    modify_with(&dir, "Add a footer", &ModifyOptions::default()).unwrap();
    let after_other = files(&dir);
    let result = apply_previewed(&dir, "Need a top navigation", &r);
    let ModifyOutcome::Conflict { reason } = &result.outcome else {
        panic!("{result:?}")
    };
    assert!(reason.contains("preview"), "{reason}");
    assert_eq!(files(&dir), after_other);
    // Previewing again recovers.
    let again = preview(&dir, "Need a top navigation");
    assert!(matches!(
        apply_previewed(&dir, "Need a top navigation", &again).outcome,
        ModifyOutcome::Applied { .. }
    ));
}

#[test]
fn applying_a_different_plan_than_previewed_conflicts() {
    let (_t, dir) = fresh();
    let r = preview(&dir, "Need a top navigation");
    let ModifyOutcome::Preview { base_revision, .. } = &r.outcome else {
        unreachable!()
    };
    let before = files(&dir);
    let result = modify_with(
        &dir,
        "Need a top navigation",
        &ModifyOptions {
            expected_revision: Some(*base_revision),
            expected_plan_id: Some("plan-from-elsewhere"),
            ..Default::default()
        },
    )
    .unwrap();
    assert!(matches!(result.outcome, ModifyOutcome::Conflict { .. }), "{result:?}");
    assert_eq!(files(&dir), before);
}

#[test]
fn external_edit_between_preview_and_apply_conflicts_and_keeps_the_edit() {
    let (_t, dir) = fresh();
    let r = preview(&dir, "Need a top navigation");
    let app = dir.join("src/App.tsx");
    let edited = fs::read_to_string(&app)
        .unwrap()
        .replace("return <></>;", "return <p>mine</p>;");
    fs::write(&app, &edited).unwrap();
    let before = files(&dir);
    let result = apply_previewed(&dir, "Need a top navigation", &r);
    let ModifyOutcome::Conflict { reason } = &result.outcome else {
        panic!("{result:?}")
    };
    assert!(reason.contains("App.tsx") || reason.contains("layout-top"), "{reason}");
    assert_eq!(fs::read_to_string(&app).unwrap(), edited);
    assert_eq!(files(&dir), before, "a conflict writes nothing");
}

#[test]
fn a_question_is_answered_by_structured_option_with_preview_then_apply() {
    let (_t, dir) = fresh();
    for command in ["Need a top navigation", "Add Contact Us"] {
        modify_with(&dir, command, &ModifyOptions::default()).unwrap();
    }
    let session = project::load_session(&dir, "default").unwrap();
    let question = session.pending_questions[0].clone();
    assert!(question.options.iter().any(|o| o.key == "new_page"));
    let answer = Answer {
        question_id: question.id.clone(),
        choice: AnswerChoice::Option {
            key: "new_page".into(),
        },
    };
    let before = files(&dir);
    let r = answer_with(
        &dir,
        &answer,
        &ModifyOptions {
            dry_run: true,
            ..Default::default()
        },
    )
    .unwrap();
    let ModifyOutcome::Preview {
        plan_id,
        base_revision,
        diffs,
        ..
    } = &r.outcome
    else {
        panic!("{r:?}")
    };
    assert_eq!(files(&dir), before);
    assert!(diffs.iter().any(|d| d.path == "src/router.ts" && d.diff.contains("/contact-us")));
    let applied = answer_with(
        &dir,
        &answer,
        &ModifyOptions {
            expected_revision: Some(*base_revision),
            expected_plan_id: Some(plan_id),
            ..Default::default()
        },
    )
    .unwrap();
    assert!(matches!(applied.outcome, ModifyOutcome::Applied { .. }), "{applied:?}");
    // The answered question is no longer pending: answering again conflicts.
    let again = answer_with(&dir, &answer, &ModifyOptions::default()).unwrap();
    assert!(matches!(again.outcome, ModifyOutcome::Conflict { .. }), "{again:?}");
}
