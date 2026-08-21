use anyhow::{Context, Result};
use serde::Deserialize;
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};

use crate::TestArgs;

#[derive(Deserialize, Default)]
struct Assertions {
    #[serde(default)]
    contains: Vec<String>,
    #[serde(default)]
    not_contains: Vec<String>,
}

fn manifest_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn recordings_dir() -> PathBuf {
    manifest_dir().join("recordings")
}

fn response_content(recording_dir: &Path) -> Result<String> {
    let response_path = recording_dir.join("response.json");
    let raw = fs::read_to_string(&response_path)
        .with_context(|| format!("reading {}", response_path.display()))?;
    let response: Value =
        serde_json::from_str(&raw).with_context(|| format!("parsing {}", response_path.display()))?;
    response
        .pointer("/response/choices/0/message/content")
        .and_then(Value::as_str)
        .map(str::to_string)
        .with_context(|| {
            format!(
                "no response text found at /response/choices/0/message/content in {}",
                response_path.display()
            )
        })
}

/// Runs a single recording's assertions. Returns `Ok(true)` if it had assertions and all
/// passed, `Ok(false)` if it had assertions and at least one failed, or `None` if the
/// recording has no `assertions.toml` (untested).
fn run_recording(name: &str, recording_dir: &Path) -> Result<Option<bool>> {
    let assertions_path = recording_dir.join("assertions.toml");
    if !assertions_path.is_file() {
        println!("{name}: untested (no assertions.toml)");
        return Ok(None);
    }

    let raw = fs::read_to_string(&assertions_path)
        .with_context(|| format!("reading {}", assertions_path.display()))?;
    let assertions: Assertions = toml::from_str(&raw)
        .with_context(|| format!("parsing {}", assertions_path.display()))?;
    let content = response_content(recording_dir)?;

    let mut all_passed = true;
    for text in &assertions.contains {
        let passed = content.contains(text.as_str());
        all_passed &= passed;
        println!(
            "{name}: {} contains {text:?}",
            if passed { "PASS" } else { "FAIL" }
        );
    }
    for text in &assertions.not_contains {
        let passed = !content.contains(text.as_str());
        all_passed &= passed;
        println!(
            "{name}: {} does not contain {text:?}",
            if passed { "PASS" } else { "FAIL" }
        );
    }

    Ok(Some(all_passed))
}

pub fn run(args: TestArgs) -> Result<()> {
    let recordings_dir = recordings_dir();

    let names: Vec<String> = match &args.name {
        Some(name) => vec![name.clone()],
        None => {
            let mut names = Vec::new();
            for entry in fs::read_dir(&recordings_dir)
                .with_context(|| format!("reading {}", recordings_dir.display()))?
            {
                let entry = entry?;
                if entry.file_type()?.is_dir() {
                    names.push(entry.file_name().to_string_lossy().into_owned());
                }
            }
            names.sort();
            names
        }
    };

    if names.is_empty() {
        println!("no recordings found under {}", recordings_dir.display());
        return Ok(());
    }

    let mut any_failed = false;
    let mut any_tested = false;
    for name in &names {
        let recording_dir = recordings_dir.join(name);
        if !recording_dir.is_dir() {
            anyhow::bail!("no recording named '{name}' at {}", recording_dir.display());
        }
        match run_recording(name, &recording_dir)? {
            Some(true) => any_tested = true,
            Some(false) => {
                any_tested = true;
                any_failed = true;
            }
            None => {}
        }
    }

    if any_failed {
        anyhow::bail!("one or more assertions failed");
    }
    if !any_tested {
        println!("no recordings had assertions.toml — nothing was tested");
    }
    Ok(())
}
