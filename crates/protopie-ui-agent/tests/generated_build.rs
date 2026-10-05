//! Generated-project build check (Epic 001, T5). Needs Node, so it is ignored
//! by default and additionally skips itself when `npm` or the reference
//! lockfile install (`reference/node_modules`) is unavailable:
//!
//! ```text
//! (cd reference && npm ci)
//! cargo test -p protopie-ui-agent --test generated_build -- --ignored
//! ```
//!
//! It generates a navigation with an unlinked item, then runs the project's
//! own build script (`tsc --noEmit && vite build`). A second test runs the
//! whole destination conversation (new page, existing section, external URL,
//! unlinked, direct page) before building.

use std::path::Path;
use std::process::Command;

fn npm_available() -> bool {
    Command::new("npm")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
}

fn reference_modules() -> Option<std::path::PathBuf> {
    let node_modules = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../reference/node_modules");
    if !npm_available() || !node_modules.is_dir() {
        eprintln!("skipping: npm or reference/node_modules is missing");
        return None;
    }
    Some(node_modules)
}

/// Runs each prompt (it must apply or ask a question), then the project build.
/// The reference's installed dependencies are reused through a symlink; the
/// agent never follows symlinks, only this test's build does.
fn generate_and_build(slug: &str, prompts: &[&str]) {
    generate_build_check(slug, |_| {}, prompts, |_| {});
}

/// Like [`generate_and_build`], with `setup` run on the fresh project before
/// the prompts and `check` run on it after a successful build.
fn generate_build_check(
    slug: &str,
    setup: impl FnOnce(&Path),
    prompts: &[&str],
    check: impl FnOnce(&Path),
) {
    let Some(node_modules) = reference_modules() else {
        return;
    };
    let tmp = tempfile::tempdir().unwrap();
    let project = protopie_ui_agent::init_named(tmp.path(), slug).unwrap();
    setup(&project);
    for prompt in prompts {
        let r = protopie_ui_agent::modify(&project, prompt).unwrap();
        assert!(
            matches!(
                r.outcome,
                protopie_ui_agent::contracts::ModifyOutcome::Applied { .. }
                    | protopie_ui_agent::contracts::ModifyOutcome::NeedsClarification { .. }
            ),
            "{prompt}: {r:?}"
        );
    }
    #[cfg(unix)]
    std::os::unix::fs::symlink(&node_modules, project.join("node_modules")).unwrap();
    #[cfg(not(unix))]
    {
        let _ = node_modules;
        eprintln!("skipping: dependency linking is only set up on Unix");
        return;
    }
    let out = Command::new("npm")
        .args(["run", "build"])
        .current_dir(&project)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "build failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    check(&project);
}

#[test]
#[ignore = "needs Node: run `npm ci` in reference/, then pass --ignored"]
fn generated_navigation_project_type_checks_and_builds() {
    generate_and_build(
        "built",
        &[
            "Need a top navigation",
            "Add Contact Us",
            "Add \"Say \\\"hi\\\" <b>{x}</b>\" to top nav",
            "Add Get in Touch",
        ],
    );
}

#[test]
#[ignore = "needs Node: run `npm ci` in reference/, then pass --ignored"]
fn generated_destination_conversation_type_checks_and_builds() {
    generate_and_build(
        "destinations",
        &[
            "Need a top navigation",
            // New page: page, route and link in one plan.
            "Add Contact Us",
            "new page",
            // Existing section of the home page.
            "Add Overview",
            "existing page or section",
            "2",
            // External URL, after a rejected one.
            "Add Docs",
            "external url",
            "not a url",
            "https://example.com/docs?a=1&b=2",
            // Existing page (home).
            "Add Home",
            "existing page or section",
            "1",
            // Left unlinked.
            "Add Soon",
            "leave unlinked",
            // Direct page creation, then a collision resolved with an alternative.
            "Add a Terms of Use page",
            "Add Terms of Use",
            "new page",
            "2",
        ],
    );
}

/// Seeds the model with a page of shared images and forms (reusable component
/// instances the T8 generators do not produce) whose stylesheet the style
/// prompts then edit.
fn seed_media(project: &Path) {
    use protopie_ui_agent::contracts::*;
    use protopie_ui_agent::project::{fingerprint, load_project, save_project, ProjectState};
    let css = ".photo {\n  display: block;\n  max-width: 100%;\n}\n\n.signup {\n  padding: 2rem 1rem;\n}\n";
    let tsx = "import styles from \"./Media.module.css\";\n\nexport default function Media() {\n\treturn (\n\t\t<div>\n\t\t\t<form data-protopie-id=\"form_1\" class={styles.signup} />\n\t\t\t<img data-protopie-id=\"img_1\" class={styles.photo} alt=\"\" />\n\t\t\t<img data-protopie-id=\"img_2\" class={styles.photo} alt=\"\" />\n\t\t</div>\n\t);\n}\n";
    std::fs::create_dir_all(project.join("src/components")).unwrap();
    std::fs::write(project.join("src/components/Media.module.css"), css).unwrap();
    std::fs::write(project.join("src/components/Media.tsx"), tsx).unwrap();
    // Outside every owned region: load the stylesheet and type-check the component.
    let index = project.join("src/index.tsx");
    let text = std::fs::read_to_string(&index).unwrap();
    std::fs::write(
        &index,
        format!("import Media from \"./components/Media\";\nconsole.debug(Media);\n{text}"),
    )
    .unwrap();
    let ProjectState::Loaded(mut model) = load_project(project).unwrap() else {
        panic!("no model")
    };
    model.owned.push(OwnedSource {
        path: "src/components/Media.module.css".into(),
        region: None,
        fingerprint: fingerprint(css),
    });
    let bind = |class: &str, scope, values: &[(&str, &str)]| StyleBinding {
        file: "src/components/Media.module.css".into(),
        class: class.into(),
        region: None,
        scope,
        values: values
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
        instance_override: false,
    };
    for (id, kind, style) in [
        (
            "form_1",
            ElementKind::Form,
            bind("signup", StyleScope::Instance, &[("padding", "2rem 1rem")]),
        ),
        (
            "img_1",
            ElementKind::Image,
            bind("photo", StyleScope::Definition, &[]),
        ),
        (
            "img_2",
            ElementKind::Image,
            bind("photo", StyleScope::Definition, &[]),
        ),
    ] {
        let mut e = ElementRecord::new(ElementId::new(id), kind, DEFAULT_PAGE);
        e.style = Some(style);
        model.elements.push(e);
    }
    save_project(project, &model).unwrap();
}

#[test]
#[ignore = "needs Node: run `npm ci` in reference/, then pass --ignored"]
fn generated_styles_type_check_and_build() {
    generate_build_check(
        "styled",
        seed_media,
        &[
            "Give the form below hero more padding",
            "Give the form below hero more padding",
            "Image needs rounded corners", // ambiguous between two images: asks
            "Increase form padding",
        ],
        |project| {
            let css =
                std::fs::read_to_string(project.join("src/components/Media.module.css")).unwrap();
            // Three fresh requests: 2rem 1rem -> 3rem 1.5rem -> 4rem 2rem -> 6rem 3rem.
            assert!(
                css.contains("padding: var(--space-9) var(--space-7);"),
                "{css}"
            );
            // The built stylesheet carries the tokens the generated rules use.
            let dist = project.join("dist/assets");
            let built: String = std::fs::read_dir(&dist)
                .unwrap()
                .filter_map(|e| e.ok())
                .filter(|e| e.path().extension().is_some_and(|x| x == "css"))
                .map(|e| std::fs::read_to_string(e.path()).unwrap())
                .collect();
            for needle in ["--space-6:", "--radius-md:", "var(--space-"] {
                assert!(built.contains(needle), "{needle} missing in built css");
            }
        },
    );
}

#[test]
#[ignore = "needs Node: run `npm ci` in reference/, then pass --ignored"]
fn generated_page_elements_type_check_and_build() {
    generate_build_check(
        "elements",
        |_| {},
        &[
            "Need a top navigation",
            // Form: fields, then the submit label, placed right below the hero.
            "add a form below hero",
            "Name, Email, \"Phone\" <number>",
            "Get a callback",
            // Footer at the end, an image above it with hostile alt text.
            "Need a footer",
            "Made with <3 & care",
            "add an image above the footer",
            "https://example.com/cat.png?a=1&b=2",
            "A \"cat\" on {a} sofa",
            // Buttons: unlinked, new page, existing hero section, external URL.
            "Add a button called Soon",
            "leave unlinked",
            "Add a button called Get in Touch below the form",
            "new page",
            "Add a button called Overview",
            "existing page or section",
            "2",
            "Add a button called Docs",
            "external url",
            "https://example.com/docs?a=1&b=2",
            // Styles on generated elements, and a reorder.
            "Give the form below hero more padding",
            "Give the hero more padding",
            "Image needs rounded corners",
            "Image needs full width",
            "Increase footer padding",
            "Move the footer above the image",
            "Move the hero below the footer",
        ],
        |project| {
            let home = std::fs::read_to_string(project.join("src/pages/Home.tsx")).unwrap();
            assert!(home.contains("<Footer1 />") && home.contains("<Form1 />"), "{home}");
            let form = std::fs::read_to_string(project.join("src/components/Form1.module.css"))
                .unwrap();
            assert!(form.contains("padding: var(--space-7) var(--space-5);"), "{form}");
            let button =
                std::fs::read_to_string(project.join("src/components/Button2.tsx")).unwrap();
            assert!(button.contains("href={\"/get-in-touch\"}"), "{button}");
        },
    );
}

#[test]
#[ignore = "needs Node: run `npm ci` in reference/, then pass --ignored"]
fn generated_contexts_type_check_and_build() {
    generate_build_check(
        "contexts",
        |_| {},
        &[
            "Add a Doctors page",
            "Add a Visits page",
            // Text state read by one page; scope is in the prompt.
            "Share the selected doctor across pages",
            "text",
            "Dr. \"Rao\" <b>{x}</b>",
            "doctors",
            "done",
            // Number state read by both pages; the scope is asked.
            "Share the visit count",
            "across all pages",
            "number",
            "007",
            "visits",
            "doctors",
            // Yes/no state and an optional text that starts unset.
            "Share the \"Is Open\" across pages",
            "yes or no",
            "yes",
            "doctors",
            "done",
            "Share the last note across pages",
            "optional text",
            "visits",
            "done",
        ],
        |project| {
            let app = std::fs::read_to_string(project.join("src/App.tsx")).unwrap();
            assert!(app.contains("<SelectedDoctorProvider>"), "{app}");
            assert!(app.contains("<LastNoteProvider>"), "{app}");
            let doctors =
                std::fs::read_to_string(project.join("src/pages/DoctorsPage.tsx")).unwrap();
            assert!(doctors.contains("useContext(VisitCountContext)"), "{doctors}");
            let note = std::fs::read_to_string(project.join("src/context/LastNote.tsx")).unwrap();
            assert!(note.contains("createSignal<string | undefined>()"), "{note}");
        },
    );
}
