//! UI prototype agent: scaffolds TypeScript + SolidJS projects and (later) modifies them.
//!
//! Template location: the template is NOT embedded in the binary. It is read at
//! runtime from `<CARGO_MANIFEST_DIR>/../../reference`, a path fixed at compile
//! time so it never depends on the current working directory. (Embedding via
//! `include_dir` would also pull in `node_modules/` and `dist/`.)

use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};

pub mod parser;

/// Directory names never copied from the template.
const SKIP: &[&str] = &["node_modules", "dist", ".git", "target", ".DS_Store"];

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("io error at {path}: {source}")]
    Io { path: PathBuf, source: io::Error },
    #[error("project path is not an existing directory: {0}")]
    NotADirectory(PathBuf),
    #[error("path escapes the project: {0}")]
    Escape(PathBuf),
    #[error("invalid project name {0:?}: use lowercase letters, digits and '-' only")]
    InvalidSlug(String),
    #[error("project already exists: {0}")]
    AlreadyExists(PathBuf),
}

pub type Result<T> = std::result::Result<T, Error>;

fn io_err(path: &Path) -> impl FnOnce(io::Error) -> Error + '_ {
    move |source| Error::Io { path: path.to_path_buf(), source }
}

fn template_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../reference")
}

fn copy_template(src: &Path, dst: &Path) -> Result<()> {
    fs::create_dir_all(dst).map_err(io_err(dst))?;
    for entry in fs::read_dir(src).map_err(io_err(src))? {
        let entry = entry.map_err(io_err(src))?;
        let name = entry.file_name();
        if SKIP.iter().any(|s| name == *s) {
            continue;
        }
        let to = dst.join(&name);
        let from = entry.path();
        if entry.file_type().map_err(io_err(&from))?.is_dir() {
            copy_template(&from, &to)?;
        } else {
            fs::copy(&from, &to).map_err(io_err(&from))?;
        }
    }
    Ok(())
}

/// Creates `dir` (failing if it exists, so concurrent callers cannot collide),
/// copies the template into it and returns its canonicalised path.
fn create_project_dir(dir: &Path) -> Result<PathBuf> {
    match fs::create_dir(dir) {
        Ok(()) => {
            copy_template(&template_dir(), dir)?;
            dir.canonicalize().map_err(io_err(dir))
        }
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
            Err(Error::AlreadyExists(dir.to_path_buf()))
        }
        Err(e) => Err(Error::Io { path: dir.to_path_buf(), source: e }),
    }
}

/// Creates a new `project-<n>` folder under `root` (created if missing), copies
/// the template into it and returns its absolute, canonicalised path.
/// Existing folders are never reused: `n` is the first unused number from 1.
pub fn init(root: &Path) -> Result<PathBuf> {
    fs::create_dir_all(root).map_err(io_err(root))?;
    let mut n = 1u32;
    loop {
        match create_project_dir(&root.join(format!("project-{n}"))) {
            Err(Error::AlreadyExists(_)) => n += 1,
            other => return other,
        }
    }
}

/// True if `slug` is non-empty and made only of `[a-z0-9-]` (so it can never
/// contain a path separator or `..`).
fn is_valid_slug(slug: &str) -> bool {
    !slug.is_empty() && slug.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// Like [`init`] but names the folder `slug`. Fails with [`Error::InvalidSlug`]
/// for an invalid slug and [`Error::AlreadyExists`] if the folder exists.
pub fn init_named(root: &Path, slug: &str) -> Result<PathBuf> {
    if !is_valid_slug(slug) {
        return Err(Error::InvalidSlug(slug.to_string()));
    }
    fs::create_dir_all(root).map_err(io_err(root))?;
    create_project_dir(&root.join(slug))
}

/// Lowercases `name`, turns every run of non-alphanumeric characters into a
/// single `-` and trims leading/trailing dashes. May return an empty string.
pub fn slugify(name: &str) -> String {
    let mut out = String::new();
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    out
}

/// Sorted names of the project folders directly under `root`; empty if `root`
/// does not exist.
pub fn list_projects(root: &Path) -> Result<Vec<String>> {
    let entries = match fs::read_dir(root) {
        Ok(e) => e,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(Error::Io { path: root.to_path_buf(), source: e }),
    };
    let mut names = Vec::new();
    for entry in entries {
        let entry = entry.map_err(io_err(root))?;
        let path = entry.path();
        if path.is_dir() {
            if let Some(name) = entry.file_name().to_str() {
                names.push(name.to_string());
            }
        }
    }
    names.sort();
    Ok(names)
}

/// Applies a natural-language `command` to the project at `project_path`.
/// For now it only validates the project and echoes the command.
pub fn modify(project_path: &Path, command: &str) -> Result<String> {
    if !project_path.is_dir() {
        return Err(Error::NotADirectory(project_path.to_path_buf()));
    }
    Ok(format!("received: {command}"))
}

/// Resolves `relative` inside `project_path`, rejecting absolute paths, `..`
/// components and symlinks that point outside the project. The target need not
/// exist yet; its deepest existing ancestor is canonicalised and checked.
#[allow(dead_code)] // used by future file-changing code
pub(crate) fn resolve_in_project(project_path: &Path, relative: &str) -> Result<PathBuf> {
    let rel = Path::new(relative);
    let escape = || Error::Escape(rel.to_path_buf());
    if rel.is_absolute() || rel.components().any(|c| !matches!(c, Component::Normal(_) | Component::CurDir)) {
        return Err(escape());
    }
    let base = project_path.canonicalize().map_err(io_err(project_path))?;
    let joined = base.join(rel);
    // Canonicalise the deepest existing ancestor to catch symlink escapes.
    let mut existing = joined.as_path();
    while !existing.exists() {
        existing = existing.parent().ok_or_else(escape)?;
    }
    let real = existing.canonicalize().map_err(io_err(existing))?;
    if !real.starts_with(&base) {
        return Err(escape());
    }
    Ok(joined)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn init_copies_template() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("new-root");
        let a = init(&root).unwrap();
        let b = init(&root).unwrap();
        assert!(a.is_absolute());
        assert_ne!(a, b);
        assert!(a.join("package.json").is_file());
        assert!(a.join("src").is_dir());
        assert!(!a.join("node_modules").exists());
        assert!(!a.join("dist").exists());
    }

    #[test]
    fn slugify_normalises() {
        assert_eq!(slugify("My Cool App!"), "my-cool-app");
        assert_eq!(slugify("  --Hello__World--  "), "hello-world");
        assert_eq!(slugify("a   b"), "a-b");
        assert_eq!(slugify("***"), "");
        assert_eq!(slugify("../etc"), "etc");
    }

    #[test]
    fn init_named_validates_and_never_overwrites() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("root");
        let p = init_named(&root, "my-app").unwrap();
        assert!(p.ends_with("my-app"));
        assert!(p.join("package.json").is_file());
        assert!(matches!(init_named(&root, "my-app"), Err(Error::AlreadyExists(_))));
        for bad in ["", "..", "a/b", "A", "a b", "a\\b", "../x"] {
            assert!(matches!(init_named(&root, bad), Err(Error::InvalidSlug(_))), "{bad:?}");
        }
    }

    #[test]
    fn list_projects_sorted() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("root");
        assert!(list_projects(&root).unwrap().is_empty());
        init_named(&root, "zeta").unwrap();
        init_named(&root, "alpha").unwrap();
        fs::write(root.join("stray.txt"), "x").unwrap();
        assert_eq!(list_projects(&root).unwrap(), vec!["alpha", "zeta"]);
    }

    #[test]
    fn modify_echoes() {
        let tmp = tempfile::tempdir().unwrap();
        assert_eq!(modify(tmp.path(), "Add top header").unwrap(), "received: Add top header");
        assert!(matches!(modify(&tmp.path().join("nope"), "x"), Err(Error::NotADirectory(_))));
    }

    #[test]
    fn resolve_rejects_escapes() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("proj");
        fs::create_dir(&p).unwrap();
        assert!(resolve_in_project(&p, "src/App.tsx").unwrap().ends_with("src/App.tsx"));
        assert!(resolve_in_project(&p, "/etc/passwd").is_err());
        assert!(resolve_in_project(&p, "../x").is_err());
        assert!(resolve_in_project(&p, "a/../../x").is_err());
        #[cfg(unix)]
        {
            let outside = tmp.path().join("outside");
            fs::create_dir(&outside).unwrap();
            std::os::unix::fs::symlink(&outside, p.join("link")).unwrap();
            assert!(resolve_in_project(&p, "link/file.txt").is_err());
        }
    }
}
