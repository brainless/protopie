//! Conversation corpus (Epic 001, T6): ordered turns against a fresh copy of
//! the reference template, with expected outcomes, files and persisted
//! session state after every turn. Fixtures live in `tests/corpus/*.json`.
//!
//! Turn format:
//!
//! ```json
//! { "say": "Add Contact Us",              // or { "answer": {"question": "destination", "option": "new_page"} }
//!   "dry_run": false,
//!   "expect": {
//!     "outcome": "applied",               // outcome kind
//!     "reply_contains": ["..."],
//!     "questions": ["substring of each returned question prompt"],
//!     "changed_files": ["src/..."],       // exact set, order-insensitive
//!     "files": {"src/x.tsx": {"contains": ["..."], "lacks": ["..."]}},
//!     "absent": ["src/pages/Foo.tsx"],    // files that must not exist
//!     "pending": 1,                       // persisted pending questions
//!     "focus": "nav_1" } }
//! ```
//!
//! A structured answer addresses the newest pending question whose ID ends
//! with `-<question>`. Every turn also checks that no owned source drifted.

use std::fs;
use std::path::Path;

use protopie_ui_agent::contracts::{Answer, AnswerChoice, ModifyOutcome};
use protopie_ui_agent::{answer_with, init_named, modify_with, project, ModifyOptions};
use serde_json::Value;

fn outcome_kind(outcome: &ModifyOutcome) -> &'static str {
    match outcome {
        ModifyOutcome::Preview { .. } => "preview",
        ModifyOutcome::Applied { .. } => "applied",
        ModifyOutcome::NeedsClarification { .. } => "needs_clarification",
        ModifyOutcome::NoChange { .. } => "no_change",
        ModifyOutcome::Unsupported { .. } => "unsupported",
        ModifyOutcome::Conflict { .. } => "conflict",
    }
}

fn strings(v: &Value) -> Vec<String> {
    v.as_array()
        .map(|a| a.iter().map(|s| s.as_str().unwrap().to_string()).collect())
        .unwrap_or_default()
}

fn run_conversation(fixture: &Value) {
    let name = fixture["name"].as_str().unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let dir = init_named(tmp.path(), "p").unwrap();
    for (n, turn) in fixture["turns"].as_array().unwrap().iter().enumerate() {
        let at = format!("{name}, turn {n}");
        let options = ModifyOptions {
            dry_run: turn["dry_run"].as_bool().unwrap_or(false),
            ..Default::default()
        };
        let result = if let Some(say) = turn["say"].as_str() {
            modify_with(&dir, say, &options).unwrap()
        } else {
            let spec = &turn["answer"];
            let suffix = format!("-{}", spec["question"].as_str().unwrap());
            let session = project::load_session(&dir, "default").unwrap();
            let id = session
                .pending_questions
                .iter()
                .rev()
                .find(|q| q.id.ends_with(&suffix))
                .unwrap_or_else(|| panic!("{at}: no pending *{suffix}"))
                .id
                .clone();
            let choice = match (spec["option"].as_str(), spec["text"].as_str()) {
                (Some(key), _) => AnswerChoice::Option { key: key.into() },
                (_, Some(text)) => AnswerChoice::Text { text: text.into() },
                _ => panic!("{at}: answer needs option or text"),
            };
            answer_with(
                &dir,
                &Answer {
                    question_id: id,
                    choice,
                },
                &options,
            )
            .unwrap()
        };
        let expect = &turn["expect"];
        assert_eq!(
            outcome_kind(&result.outcome),
            expect["outcome"].as_str().unwrap(),
            "{at}: {result:?}"
        );
        for needle in strings(&expect["reply_contains"]) {
            assert!(
                result.summary.contains(&needle),
                "{at}: {needle:?} in {:?}",
                result.summary
            );
        }
        let questions = match &result.outcome {
            ModifyOutcome::Applied { follow_up, .. }
            | ModifyOutcome::Preview { follow_up, .. }
            | ModifyOutcome::NoChange { follow_up, .. } => follow_up.clone(),
            ModifyOutcome::NeedsClarification { questions, .. } => questions.clone(),
            _ => Vec::new(),
        };
        let want = strings(&expect["questions"]);
        if expect.get("questions").is_some() {
            assert_eq!(questions.len(), want.len(), "{at}: {questions:?}");
            for (q, needle) in questions.iter().zip(&want) {
                assert!(
                    q.prompt.contains(needle),
                    "{at}: {needle:?} in {:?}",
                    q.prompt
                );
            }
        }
        if let Some(expected) = expect.get("changed_files") {
            let (ModifyOutcome::Applied { changed_files, .. }
            | ModifyOutcome::Preview { changed_files, .. }) = &result.outcome
            else {
                panic!("{at}: changed_files expects an applied or preview outcome")
            };
            let mut got = changed_files.clone();
            let mut want = strings(expected);
            got.sort();
            want.sort();
            assert_eq!(got, want, "{at}");
        }
        for (path, spec) in expect["files"].as_object().into_iter().flatten() {
            let text = fs::read_to_string(dir.join(path))
                .unwrap_or_else(|e| panic!("{at}: cannot read {path}: {e}"));
            for needle in strings(&spec["contains"]) {
                assert!(
                    text.contains(&needle),
                    "{at}: {path} lacks {needle:?}\n{text}"
                );
            }
            for needle in strings(&spec["lacks"]) {
                assert!(
                    !text.contains(&needle),
                    "{at}: {path} has {needle:?}\n{text}"
                );
            }
        }
        for path in strings(&expect["absent"]) {
            assert!(!dir.join(&path).exists(), "{at}: {path} must not exist");
        }
        let session = project::load_session(&dir, "default").unwrap();
        if let Some(count) = expect["pending"].as_u64() {
            assert_eq!(
                session.pending_questions.len() as u64,
                count,
                "{at}: {session:?}"
            );
        }
        if let Some(focus) = expect["focus"].as_str() {
            assert_eq!(
                session.focus.as_ref().map(|f| f.0.as_str()),
                Some(focus),
                "{at}"
            );
        }
        if let project::ProjectState::Loaded(model) = project::load_project(&dir).unwrap() {
            assert!(
                project::verify_owned(&dir, &model).unwrap().is_empty(),
                "{at}: owned source drifted"
            );
        }
    }
}

fn corpus(file: &str) -> Vec<Value> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/corpus")
        .join(file);
    let text = fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    serde_json::from_str::<Value>(&text)
        .unwrap()
        .as_array()
        .unwrap()
        .clone()
}

#[test]
fn destination_conversations() {
    let all = corpus("destinations.json");
    assert!(all.len() >= 8, "corpus shrank");
    for fixture in &all {
        run_conversation(fixture);
    }
}

#[test]
fn element_conversations() {
    let all = corpus("elements.json");
    assert!(all.len() >= 4, "corpus shrank");
    for fixture in &all {
        run_conversation(fixture);
    }
}
