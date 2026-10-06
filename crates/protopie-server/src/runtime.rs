use std::{
    env,
    path::{Path, PathBuf},
    process::Command,
};

use protopie_api::{
    RuntimeCheckResponse, RuntimeExecutable, RuntimeFailure, RuntimeStatus, REQUIRED_NODE_RANGE,
};

/// The preview manager uses the paths in this result for `npm ci` and Vite.
/// A fresh check at launch also catches changes to PATH since the last GUI check.
pub(crate) fn check_runtime() -> RuntimeCheckResponse {
    check_with(&SystemProbe)
}

trait RuntimeProbe {
    fn find(&self, tool: &str) -> Option<PathBuf>;
    fn version(&self, path: &Path) -> Result<String, String>;
}

struct SystemProbe;

impl RuntimeProbe for SystemProbe {
    fn find(&self, tool: &str) -> Option<PathBuf> {
        let path = env::var_os("PATH")?;
        for dir in env::split_paths(&path) {
            // On Windows the shell may use a PATHEXT suffix. npm is commonly npm.cmd.
            #[cfg(windows)]
            let names = [
                tool.to_string(),
                format!("{tool}.cmd"),
                format!("{tool}.exe"),
            ];
            #[cfg(not(windows))]
            let names = [tool.to_string()];
            for name in names {
                let candidate = dir.join(name);
                if is_executable(&candidate) {
                    if let Ok(path) = candidate.canonicalize() {
                        return Some(path);
                    }
                }
            }
        }
        None
    }

    fn version(&self, path: &Path) -> Result<String, String> {
        let output = Command::new(path)
            .arg("--version")
            .output()
            .map_err(|e| e.to_string())?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            return Err(if stderr.is_empty() {
                format!("exited with {}", output.status)
            } else {
                format!("exited with {}: {stderr}", output.status)
            });
        }
        let version = String::from_utf8(output.stdout)
            .map_err(|e| format!("non-UTF-8 version output: {e}"))?
            .trim()
            .to_string();
        if version.is_empty() {
            return Err("empty version output".into());
        }
        Ok(version)
    }
}

fn is_executable(path: &Path) -> bool {
    let Ok(metadata) = path.metadata() else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

fn check_with(probe: &impl RuntimeProbe) -> RuntimeCheckResponse {
    let node = inspect(probe, "node");
    let npm = inspect(probe, "npm");
    let version_error = node.as_ref().ok().and_then(|node| {
        if supported_node(&node.version) {
            None
        } else {
            Some(RuntimeFailure::UnsupportedVersion {
                tool: "node".into(),
                found: node.version.clone(),
                required: REQUIRED_NODE_RANGE.into(),
                message: format!(
                    "Node {} is unsupported. Install Node {} and restart ProtoPie.",
                    node.version, REQUIRED_NODE_RANGE
                ),
            })
        }
    });
    let reason = node
        .as_ref()
        .err()
        .cloned()
        .or(version_error)
        .or_else(|| npm.as_ref().err().cloned());
    RuntimeCheckResponse {
        node: node.ok(),
        npm: npm.ok(),
        required_node_range: REQUIRED_NODE_RANGE.into(),
        status: match reason {
            Some(reason) => RuntimeStatus::Unavailable { reason },
            None => RuntimeStatus::Available,
        },
    }
}

fn inspect(probe: &impl RuntimeProbe, tool: &str) -> Result<RuntimeExecutable, RuntimeFailure> {
    let path = probe
        .find(tool)
        .ok_or_else(|| RuntimeFailure::MissingExecutable {
            tool: tool.into(),
            message: format!(
                "{tool} was not found on PATH. Install Node.js with npm and restart ProtoPie."
            ),
        })?;
    let version = probe.version(&path).map_err(|details| RuntimeFailure::CommandFailed {
        tool: tool.into(),
        path: path.to_string_lossy().into_owned(),
        message: format!("Could not run {tool} --version at {}. Repair the Node.js installation or choose a working executable on PATH.", path.display()),
        details,
    })?;
    Ok(RuntimeExecutable {
        path: path.to_string_lossy().into_owned(),
        version,
    })
}

fn supported_node(version: &str) -> bool {
    let value = version.trim().strip_prefix('v').unwrap_or(version.trim());
    let mut numbers = value.split('.');
    let (Some(Ok(major)), Some(Ok(minor)), Some(Ok(_patch)), None) = (
        numbers.next().map(str::parse::<u64>),
        numbers.next().map(str::parse::<u64>),
        numbers.next().map(str::parse::<u64>),
        numbers.next(),
    ) else {
        return false;
    };
    (major == 20 && minor >= 19) || major == 22 && minor >= 12 || major > 22
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[derive(Default)]
    struct FakeProbe(HashMap<String, Result<String, String>>);

    impl FakeProbe {
        fn with(mut self, tool: &str, result: Result<&str, &str>) -> Self {
            self.0.insert(
                tool.into(),
                result.map(str::to_string).map_err(str::to_string),
            );
            self
        }
    }

    impl RuntimeProbe for FakeProbe {
        fn find(&self, tool: &str) -> Option<PathBuf> {
            self.0
                .contains_key(tool)
                .then(|| PathBuf::from(format!("/fake/{tool}")))
        }
        fn version(&self, path: &Path) -> Result<String, String> {
            self.0[path.file_name().unwrap().to_str().unwrap()].clone()
        }
    }

    #[test]
    fn missing_tools_have_actionable_structured_failure() {
        let response = check_with(&FakeProbe::default());
        assert!(response.node.is_none() && response.npm.is_none());
        assert!(
            matches!(response.status, RuntimeStatus::Unavailable { reason: RuntimeFailure::MissingExecutable { ref tool, ref message } } if tool == "node" && message.contains("Install Node.js"))
        );
        let response = check_with(&FakeProbe::default().with("node", Ok("v22.12.0")));
        assert!(
            matches!(response.status, RuntimeStatus::Unavailable { reason: RuntimeFailure::MissingExecutable { ref tool, .. } } if tool == "npm")
        );
    }

    #[test]
    fn node_range_boundaries_and_valid_versions() {
        let npm = || FakeProbe::default().with("npm", Ok("11.1.0"));
        for version in ["v20.18.9", "v21.9.0", "v22.11.9", "garbage"] {
            let response = check_with(&npm().with("node", Ok(version)));
            assert!(
                matches!(response.status, RuntimeStatus::Unavailable { reason: RuntimeFailure::UnsupportedVersion { ref found, ref required, .. } } if found == version && required == REQUIRED_NODE_RANGE)
            );
            assert_eq!(response.npm.unwrap().version, "11.1.0");
        }
        for version in ["v20.19.0", "v20.20.0", "v22.12.0", "v23.0.0"] {
            let response = check_with(&npm().with("node", Ok(version)));
            assert_eq!(response.status, RuntimeStatus::Available);
            assert_eq!(response.node.unwrap().path, "/fake/node");
        }
    }

    #[test]
    fn command_failures_preserve_tool_path_and_details() {
        let response = check_with(
            &FakeProbe::default()
                .with("node", Err("exit 7"))
                .with("npm", Ok("10.0.0")),
        );
        assert!(
            matches!(response.status, RuntimeStatus::Unavailable { reason: RuntimeFailure::CommandFailed { ref tool, ref path, ref details, .. } } if tool == "node" && path == "/fake/node" && details == "exit 7")
        );
        let response = check_with(
            &FakeProbe::default()
                .with("node", Ok("v22.12.0"))
                .with("npm", Err("exit 8")),
        );
        assert!(
            matches!(response.status, RuntimeStatus::Unavailable { reason: RuntimeFailure::CommandFailed { ref tool, ref details, .. } } if tool == "npm" && details == "exit 8")
        );
    }
}
