//! Launches `protopie-server` as a separate OS process and tracks its address.

use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};

use anyhow::{anyhow, Context, Result};

pub struct ServerProcess {
    child: Child,
    pub base_url: String,
}

fn server_binary() -> Result<PathBuf> {
    if let Ok(p) = std::env::var("PROTOPIE_SERVER_BIN") {
        return Ok(p.into());
    }
    let name = format!("protopie-server{}", std::env::consts::EXE_SUFFIX);
    let path = std::env::current_exe()?
        .parent()
        .ok_or_else(|| anyhow!("no exe dir"))?
        .join(name);
    if !path.exists() {
        return Err(anyhow!(
            "{} not found; run `cargo build -p protopie-server` (or ./run.sh)",
            path.display()
        ));
    }
    Ok(path)
}

impl ServerProcess {
    pub fn spawn() -> Result<Self> {
        let bin = server_binary()?;
        let mut child = Command::new(&bin)
            .arg("--exit-on-stdin-close")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .with_context(|| format!("spawning {}", bin.display()))?;
        let stdout = child.stdout.take().ok_or_else(|| anyhow!("no stdout"))?;
        let mut line = String::new();
        BufReader::new(stdout).read_line(&mut line)?;
        let addr = line
            .trim()
            .strip_prefix("listening on ")
            .ok_or_else(|| anyhow!("unexpected server output: {line:?}"))?;
        Ok(Self {
            child,
            base_url: format!("http://{addr}"),
        })
    }
}

impl Drop for ServerProcess {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
