//! Regression hardening (Epic 001, T9): property-style checks over generated
//! prompts. There is no external fuzz dependency: a fixed-seed generator mixes
//! the command vocabulary with hostile text, so runs are reproducible.
//!
//! Properties:
//! - the parser never panics and every span it reports is ordered, in bounds
//!   and on character boundaries;
//! - a dry run never changes anything on disk;
//! - a request that is rejected, asks a question or changes nothing never
//!   changes code or the model, and owned source never drifts from the model;
//! - the same prompt against the same project state gives the same result.
//!
//! Arbitrary input may legitimately parse as a supported command, so
//! applying outcomes are allowed; they just must keep the project consistent.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use protopie_ui_agent::contracts::ModifyOutcome;
use protopie_ui_agent::parser::parse;
use protopie_ui_agent::{init_named, modify_with, project, ModifyOptions};
use serde_json::Value;

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        // xorshift64*
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[(self.next() % items.len() as u64) as usize]
    }
}

const WORDS: &[&str] = &[
    "add", "need", "remove", "move", "give", "increase", "decrease", "a", "the", "an", "to", "of",
    "top", "nav", "navigation", "hero", "section", "image", "form", "footer", "button", "page",
    "below", "above", "inside", "left", "right", "called", "named", "more", "less", "full",
    "width", "padding", "rounded", "corners", "radius", "needs", "do", "not", "don't", "and",
    "then", "also", "Contact", "Us", "Get", "in", "Touch", "Back", "School", "yes", "no", "1",
    "2", "3", "new", "existing", "external", "unlinked", "https://example.com", "\"", "\"quoted\"",
    "'", ",", ".", ";", "\u{201c}curly\u{201d}", "\u{00e9}t\u{00e9}", "\u{1F600}", "\u{0}", "\t",
    "\n", "share", "across", "pages", "<b>", "{x}", "../..", "/etc/passwd", "&", "\\", "%s", "Terms of Use",
];

fn prompt(rng: &mut Rng) -> String {
    let n = 1 + rng.next() % 9;
    let mut out = String::new();
    for i in 0..n {
        if i > 0 && rng.next() % 6 != 0 {
            out.push(' ');
        }
        out.push_str(rng.pick(WORDS));
    }
    out
}

/// Regression prompts from the epic's rejection families and review findings.
/// Each must be handled without panic; those that must never write are listed
/// in `NEVER_WRITE`.
const REGRESSIONS: &[&str] = &[
    "",
    "   ",
    "do not add a nav",
    "don't add a top nav",
    "Add top nav and then delete the footer",
    "Add Contact Us to top nav, and make it blue",
    "Add Back to School",
    "Add Terms of Use to top nav",
    "Add \"unterminated to top nav",
    "Add \u{1F600} to top nav",
    "Add ../../etc/passwd to top nav",
    "Add <script>alert(1)</script> to top nav",
    "Image needs rounded corners and a shadow",
    "give the form below hero more padding please",
    "yes",
    "no",
    "7",
    "Share the selected doctor across pages",
    "Share the \"unterminated across pages",
    "Share the doctor across pages.",
    "Share the ../../etc/passwd across pages",
];

/// Prompts that name something unsupported or malformed: no write, ever.
const NEVER_WRITE: &[&str] = &[
    "",
    "   ",
    "do not add a nav",
    "don't add a top nav",
    "Add top nav and then delete the footer",
    "Image needs rounded corners and a shadow",
    "yes",
    "no",
    "7",
    "Share the \"unterminated across pages",
    "Share the doctor across pages.",
    "Share the ../../etc/passwd across pages",
];

fn code_and_model(dir: &Path) -> BTreeMap<String, Vec<u8>> {
    fn walk(base: &Path, dir: &Path, out: &mut BTreeMap<String, Vec<u8>>) {
        for e in fs::read_dir(dir).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                walk(base, &p, out);
                continue;
            }
            let rel = p.strip_prefix(base).unwrap().to_string_lossy().into_owned();
            // Locks and conversation/ledger metadata may change on any request.
            if rel == ".protopie/model.json" || !rel.starts_with(".protopie/") {
                out.insert(rel, fs::read(&p).unwrap());
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(dir, dir, &mut out);
    out
}

fn code_only_of(files: &BTreeMap<String, Vec<u8>>) -> BTreeMap<String, Vec<u8>> {
    files
        .iter()
        .filter(|(k, _)| !k.starts_with(".protopie/"))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect()
}

fn code_only(dir: &Path) -> BTreeMap<String, Vec<u8>> {
    code_only_of(&code_and_model(dir))
}

fn everything(dir: &Path) -> BTreeMap<String, Vec<u8>> {
    let mut all = code_and_model(dir);
    for e in fs::read_dir(dir.join(".protopie")).into_iter().flatten() {
        let p = e.unwrap().path();
        if p.is_file() && p.file_name().unwrap() != "lock" {
            all.insert(p.to_string_lossy().into_owned(), fs::read(&p).unwrap());
        }
    }
    all
}

fn check_spans(value: &Value, input: &str, at: &str) {
    match value {
        Value::Object(map) => {
            if let (Some(Value::Number(s)), Some(Value::Number(e))) =
                (map.get("start"), map.get("end"))
            {
                let (s, e) = (s.as_u64().unwrap() as usize, e.as_u64().unwrap() as usize);
                assert!(
                    s <= e && e <= input.len(),
                    "{at}: span {s}..{e} out of order or bounds in {input:?}"
                );
                assert!(
                    input.is_char_boundary(s) && input.is_char_boundary(e),
                    "{at}: span {s}..{e} splits a character in {input:?}"
                );
            }
            map.values().for_each(|v| check_spans(v, input, at));
        }
        Value::Array(items) => items.iter().for_each(|v| check_spans(v, input, at)),
        _ => {}
    }
}

fn all_prompts() -> Vec<String> {
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    let mut prompts: Vec<String> = REGRESSIONS.iter().map(|s| s.to_string()).collect();
    prompts.extend((0..400).map(|_| prompt(&mut rng)));
    // Well-formed commands with hostile labels, so applying paths are reached.
    const LABELS: &[&str] = &[
        "Contact Us", "about", "Get in Touch", "\"Say \\\"hi\\\" <b>{x}</b>\"", "Back to School",
        "caf\u{00e9}", "a/b", "</script>", "{`${x}`}", "x y z", "'; drop", "1",
    ];
    const TEMPLATES: &[&str] = &[
        "Need a top navigation",
        "Add {} to top nav",
        "Add {}",
        "Add a {} page",
        "add a footer",
        "add a form below hero",
        "add a button called {} below the hero",
        "Give the hero more padding",
        "Give the hero less padding",
        "hero needs rounded corners",
        "increase the hero padding",
        "Share the {} across pages",
        "Share the {}",
    ];
    for _ in 0..200 {
        let t = *rng.pick(TEMPLATES);
        prompts.push(t.replace("{}", rng.pick(LABELS)));
    }
    // Long and degenerate inputs.
    prompts.push("add ".repeat(2000));
    prompts.push("\u{1F600}".repeat(500));
    prompts.push(format!("Add {} to top nav", "x".repeat(10_000)));
    prompts
}

#[test]
fn parser_never_panics_and_reports_valid_spans() {
    for p in all_prompts() {
        let outcome = parse(&p);
        check_spans(&serde_json::to_value(&outcome).unwrap(), &p, "parse");
    }
}

#[test]
fn dry_runs_of_any_prompt_never_change_the_project() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = init_named(tmp.path(), "p").unwrap();
    // Give the prompts something to talk about.
    for setup in ["Need a top navigation", "Add Contact Us", "Add a footer"] {
        modify_with(&dir, setup, &ModifyOptions::default()).unwrap();
    }
    let before = everything(&dir);
    let dry = ModifyOptions {
        dry_run: true,
        ..Default::default()
    };
    for p in all_prompts() {
        let _ = modify_with(&dir, &p, &dry).unwrap_or_else(|e| panic!("{p:?}: {e}"));
        assert_eq!(everything(&dir), before, "a dry run of {p:?} wrote");
    }
}

#[test]
fn rejected_prompts_never_write_and_applied_ones_keep_the_project_consistent() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = init_named(tmp.path(), "p").unwrap();
    let mut applied = 0;
    for p in all_prompts() {
        // The project keeps a long history; keep the run bounded.
        if applied >= 40 && !NEVER_WRITE.contains(&p.as_str()) {
            continue;
        }
        let before = code_and_model(&dir);
        let r = modify_with(&dir, &p, &ModifyOptions::default())
            .unwrap_or_else(|e| panic!("{p:?}: {e}"));
        match &r.outcome {
            ModifyOutcome::Applied { .. } => applied += 1,
            // A blocking question persists as a conversation step, which may
            // advance the model revision; it never changes code.
            ModifyOutcome::NeedsClarification { .. } => assert_eq!(
                code_only(&dir),
                code_only_of(&before),
                "{p:?} asked a question but changed code",
            ),
            _ => assert_eq!(
                code_and_model(&dir),
                before,
                "{p:?} was not applied ({:?}) but changed code or model",
                r.outcome
            ),
        }
        if NEVER_WRITE.contains(&p.as_str()) {
            assert!(
                !matches!(r.outcome, ModifyOutcome::Applied { .. }),
                "{p:?} must not apply: {r:?}"
            );
        }
        let model = match project::load_project(&dir).unwrap() {
            project::ProjectState::Loaded(m) => m,
            _ => panic!("model vanished after {p:?}"),
        };
        assert!(
            project::verify_owned(&dir, &model).unwrap().is_empty(),
            "owned source drifted from the model after {p:?}"
        );
        assert!(
            !dir.join(".protopie/journal.json").exists(),
            "a journal was left behind after {p:?}"
        );
    }
    assert!(applied > 0, "the generator should reach some applying commands");
}

#[test]
fn the_same_prompt_on_the_same_state_gives_the_same_result() {
    let mut rng = Rng(42);
    let prompts: Vec<String> = (0..60).map(|_| prompt(&mut rng)).collect();
    let run = || {
        let tmp = tempfile::tempdir().unwrap();
        let dir = init_named(tmp.path(), "p").unwrap();
        prompts
            .iter()
            .map(|p| {
                let r = modify_with(&dir, p, &ModifyOptions::default()).unwrap();
                serde_json::to_string(&r).unwrap()
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(run(), run());
}
