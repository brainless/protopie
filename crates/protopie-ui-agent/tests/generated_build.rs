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
//! own build script (`tsc --noEmit && vite build`).

use std::path::Path;
use std::process::Command;

fn npm_available() -> bool {
    Command::new("npm")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
}

#[test]
#[ignore = "needs Node: run `npm ci` in reference/, then pass --ignored"]
fn generated_navigation_project_type_checks_and_builds() {
    let reference = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../reference");
    let node_modules = reference.join("node_modules");
    if !npm_available() || !node_modules.is_dir() {
        eprintln!("skipping: npm or reference/node_modules is missing");
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let project = protopie_ui_agent::init_named(tmp.path(), "built").unwrap();
    for prompt in [
        "Need a top navigation",
        "Add Contact Us",
        "Add \"Say \\\"hi\\\" <b>{x}</b>\" to top nav",
        "Add Get in Touch",
    ] {
        let r = protopie_ui_agent::modify(&project, prompt).unwrap();
        assert!(
            matches!(
                r.outcome,
                protopie_ui_agent::contracts::ModifyOutcome::Applied { .. }
            ),
            "{prompt}: {r:?}"
        );
    }
    // Reuse the reference's installed dependencies. The agent never follows
    // symlinks; only this test's build does.
    #[cfg(unix)]
    std::os::unix::fs::symlink(&node_modules, project.join("node_modules")).unwrap();
    #[cfg(not(unix))]
    {
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
}
