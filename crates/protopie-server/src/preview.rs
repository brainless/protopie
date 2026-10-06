//! One locally owned Vite preview. Every child starts in a process group on Unix;
//! cancellation sends TERM to the group, then KILL after two seconds, and reaps
//! the direct child. SIGKILL/abnormal server death cannot guarantee cleanup, so
//! each launch chooses a new port and requires Vite's own ready line before HTTP.
use std::{
    collections::VecDeque,
    hash::Hasher,
    path::{Path, PathBuf},
    process::Stdio,
    sync::Arc,
    time::Duration,
};

use protopie_api::{
    PreviewFailure, PreviewLogLine, PreviewLogRequest, PreviewLogResponse, PreviewLogStream,
    PreviewState, PreviewStatusResponse, RuntimeCheckResponse, RuntimeStatus,
};
use protopie_ui_agent::project::{load_project, seed_snapshot, ProjectState};

/// The same eligibility check is used by project listing and launch.
pub(crate) fn validate_project(path: &Path) -> protopie_ui_agent::Result<()> {
    match load_project(path)? {
        ProjectState::Loaded(_) => seed_snapshot(path).map(|_| ()),
        ProjectState::NoMetadata => Err(protopie_ui_agent::Error::UnsupportedProject(
            "Project has no model. Create a project from the reference template.".into(),
        )),
    }
}
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::{Child, Command},
    sync::{oneshot, Mutex},
    time::{sleep, timeout, Instant},
};

const LOG_CAP: usize = 300;
const LOG_LINE_CAP: usize = 2048;
#[cfg(not(test))]
const START_TIMEOUT: Duration = Duration::from_secs(25);
#[cfg(test)]
const START_TIMEOUT: Duration = Duration::from_secs(2);
const INSTALL_TIMEOUT: Duration = Duration::from_secs(300);

#[derive(Clone)]
pub(crate) struct PreviewManager(Arc<Inner>);

struct Inner {
    operation: Mutex<()>,
    state: Mutex<State>,
    runtime: Arc<dyn Fn() -> RuntimeCheckResponse + Send + Sync>,
    port: Arc<dyn Fn() -> Result<u16, String> + Send + Sync>,
}

struct State {
    project: Option<PathBuf>,
    generation: u64,
    state: PreviewState,
    lines: VecDeque<PreviewLogLine>,
    cursor: u64,
    cancel: Option<oneshot::Sender<()>>,
    worker: Option<tokio::task::JoinHandle<()>>,
    child_pid: Option<u32>,
}

impl PreviewManager {
    pub(crate) fn new() -> Self {
        Self::with_runtime(Arc::new(crate::runtime::check_runtime))
    }

    fn with_runtime(runtime: Arc<dyn Fn() -> RuntimeCheckResponse + Send + Sync>) -> Self {
        Self(Arc::new(Inner {
            operation: Mutex::new(()),
            runtime,
            port: Arc::new(free_port),
            state: Mutex::new(State {
                project: None,
                generation: 0,
                state: PreviewState::Stopped,
                lines: VecDeque::new(),
                cursor: 0,
                cancel: None,
                worker: None,
                child_pid: None,
            }),
        }))
    }

    pub(crate) async fn launch(&self, path: &str, restart: bool) -> PreviewStatusResponse {
        let _operation = self.0.operation.lock().await;
        let canonical = match std::fs::canonicalize(path) {
            Ok(p) if p.is_dir() => p,
            Ok(_) => {
                return self
                    .fail_selected(
                        PathBuf::from(path),
                        PreviewFailure::UnsupportedProject {
                            message: "Project is not a directory".into(),
                        },
                    )
                    .await
            }
            Err(e) => {
                return self
                    .fail_selected(
                        PathBuf::from(path),
                        PreviewFailure::UnsupportedProject {
                            message: format!("Cannot open project: {e}"),
                        },
                    )
                    .await
            }
        };
        {
            let state = self.0.state.lock().await;
            if !restart
                && state.project.as_ref() == Some(&canonical)
                && matches!(
                    state.state,
                    PreviewState::Preparing { .. }
                        | PreviewState::Starting
                        | PreviewState::Running { .. }
                )
            {
                return snapshot(&state);
            }
        }
        self.stop_inner().await;
        let valid = validate_project(&canonical);
        if let Err(e) = valid {
            return self
                .fail_selected(
                    canonical,
                    PreviewFailure::UnsupportedProject {
                        message: e.to_string(),
                    },
                )
                .await;
        }
        let runtime = (self.0.runtime)();
        if let RuntimeStatus::Unavailable { reason } = runtime.status {
            return self
                .fail_selected(canonical, PreviewFailure::Environment { reason })
                .await;
        }
        let node = runtime.node.expect("available runtime has Node").path;
        let npm = runtime.npm.expect("available runtime has npm").path;
        let (tx, rx) = oneshot::channel();
        let mut state = self.0.state.lock().await;
        state.project = Some(canonical.clone());
        state.generation += 1;
        state.state = PreviewState::Preparing {
            step: "Checking dependencies".into(),
        };
        state.lines.clear();
        state.cancel = Some(tx);
        let generation = state.generation;
        let manager = self.clone();
        state.worker = Some(tokio::spawn(async move {
            manager.run(canonical, generation, node, npm, rx).await;
        }));
        snapshot(&state)
    }

    async fn fail_selected(&self, path: PathBuf, reason: PreviewFailure) -> PreviewStatusResponse {
        self.stop_inner().await;
        let mut state = self.0.state.lock().await;
        state.project = Some(path);
        state.generation += 1;
        state.state = PreviewState::Failed { reason };
        state.lines.clear();
        snapshot(&state)
    }

    pub(crate) async fn stop(&self) -> PreviewStatusResponse {
        let _operation = self.0.operation.lock().await;
        self.stop_inner().await;
        self.status().await
    }

    async fn stop_inner(&self) {
        let worker = {
            let mut state = self.0.state.lock().await;
            if let Some(tx) = state.cancel.take() {
                let _ = tx.send(());
            }
            state.worker.take()
        };
        if let Some(worker) = worker {
            let _ = worker.await;
        }
        let mut state = self.0.state.lock().await;
        state.generation += 1;
        state.state = PreviewState::Stopped;
        state.child_pid = None;
    }

    pub(crate) async fn status(&self) -> PreviewStatusResponse {
        snapshot(&*self.0.state.lock().await)
    }

    #[cfg(test)]
    pub(crate) async fn child_pid(&self) -> Option<u32> {
        self.0.state.lock().await.child_pid
    }

    pub(crate) async fn logs(&self, request: PreviewLogRequest) -> PreviewLogResponse {
        let state = self.0.state.lock().await;
        let project_path = state
            .project
            .as_ref()
            .map(|p| p.to_string_lossy().into_owned());
        let wanted = std::fs::canonicalize(&request.project_path)
            .unwrap_or_else(|_| PathBuf::from(&request.project_path));
        if state.project.as_ref() != Some(&wanted) {
            return PreviewLogResponse {
                project_path,
                generation: state.generation,
                next_cursor: state.cursor,
                truncated: false,
                lines: vec![],
            };
        }
        let oldest = state
            .lines
            .front()
            .map_or(state.cursor + 1, |line| line.cursor);
        PreviewLogResponse {
            project_path,
            generation: state.generation,
            next_cursor: state.cursor,
            truncated: request.cursor.saturating_add(1) < oldest,
            lines: state
                .lines
                .iter()
                .filter(|line| line.cursor > request.cursor)
                .cloned()
                .collect(),
        }
    }

    async fn set(&self, generation: u64, value: PreviewState) {
        let mut state = self.0.state.lock().await;
        if state.generation == generation {
            state.state = value;
        }
    }

    async fn log(&self, generation: u64, stream: PreviewLogStream, text: String) {
        let mut state = self.0.state.lock().await;
        if state.generation != generation {
            return;
        }
        state.cursor += 1;
        let cursor = state.cursor;
        let text: String = text.chars().take(LOG_LINE_CAP).collect();
        state.lines.push_back(PreviewLogLine {
            cursor,
            stream,
            text,
        });
        if state.lines.len() > LOG_CAP {
            state.lines.pop_front();
        }
    }

    async fn tail(&self) -> String {
        let state = self.0.state.lock().await;
        state
            .lines
            .iter()
            .rev()
            .take(8)
            .map(|l| l.text.as_str())
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect::<Vec<_>>()
            .join(" | ")
    }

    async fn run(
        &self,
        project: PathBuf,
        generation: u64,
        node: String,
        npm: String,
        mut cancel: oneshot::Receiver<()>,
    ) {
        let result = self
            .run_steps(&project, generation, &node, &npm, &mut cancel)
            .await;
        if let Err(failure) = result {
            self.set(generation, PreviewState::Failed { reason: failure })
                .await;
        }
    }

    async fn run_steps(
        &self,
        project: &Path,
        generation: u64,
        node: &str,
        npm: &str,
        cancel: &mut oneshot::Receiver<()>,
    ) -> Result<(), PreviewFailure> {
        let fingerprint =
            package_fingerprint(project).map_err(|e| PreviewFailure::Install { message: e })?;
        let stamp_path = project.join(".protopie/preview-install-stamp");
        if !project.join("node_modules").is_dir()
            || std::fs::read_to_string(&stamp_path).ok().as_deref() != Some(&fingerprint)
        {
            self.set(
                generation,
                PreviewState::Preparing {
                    step: "Installing dependencies".into(),
                },
            )
            .await;
            let mut command = Command::new(npm);
            command
                .arg("ci")
                .current_dir(project)
                .env("PATH", preferred_path(node));
            let mut child = spawn(command).map_err(|e| PreviewFailure::Install { message: e })?;
            let install_pid = child.id();
            let mut readers = capture(self.clone(), generation, &mut child);
            let status = tokio::select! {
                _ = &mut *cancel => { terminate(&mut child).await; return Ok(()); }
                result = timeout(INSTALL_TIMEOUT, child.wait()) => match result {
                    Ok(Ok(status)) => status,
                    Ok(Err(e)) => { terminate(&mut child).await; drain_logs(&mut readers).await; return Err(PreviewFailure::Install { message: e.to_string() }); }
                    Err(_) => { terminate(&mut child).await; drain_logs(&mut readers).await; return Err(PreviewFailure::Install { message: format!("npm ci timed out: {}", self.tail().await) }); }
                },
            };
            cleanup_exited_group(install_pid).await;
            drain_logs(&mut readers).await;
            if !status.success() {
                return Err(PreviewFailure::Install {
                    message: format!("npm ci exited with {status}: {}", self.tail().await),
                });
            }
            std::fs::write(&stamp_path, &fingerprint).map_err(|e| PreviewFailure::Install {
                message: format!("Cannot record install: {e}"),
            })?;
        }
        self.set(generation, PreviewState::Starting).await;
        let port = (self.0.port)().map_err(|message| PreviewFailure::Start { message })?;
        let url = format!("http://127.0.0.1:{port}/");
        let mut command = Command::new(node);
        command
            .arg(project.join("node_modules/vite/bin/vite.js"))
            .args([
                "--host",
                "127.0.0.1",
                "--port",
                &port.to_string(),
                "--strictPort",
            ])
            .current_dir(project);
        let mut child = spawn(command).map_err(|e| PreviewFailure::Start { message: e })?;
        let child_pid = child.id();
        self.0.state.lock().await.child_pid = child_pid;
        let mut readers = capture(self.clone(), generation, &mut child);
        let deadline = Instant::now() + START_TIMEOUT;
        loop {
            tokio::select! {
                _ = &mut *cancel => { terminate(&mut child).await; return Ok(()); }
                result = child.wait() => { cleanup_exited_group(child_pid).await; drain_logs(&mut readers).await; return Err(PreviewFailure::Exited { message: format!("Vite exited before readiness ({}): {}", result.map(|s| s.to_string()).unwrap_or_else(|e| e.to_string()), self.tail().await) }); },
                _ = sleep(Duration::from_millis(100)) => {}
            }
            if Instant::now() >= deadline {
                terminate(&mut child).await;
                return Err(PreviewFailure::Readiness {
                    message: format!("Vite did not become ready at {url}: {}", self.tail().await),
                });
            }
            let ready_line = {
                self.0
                    .state
                    .lock()
                    .await
                    .lines
                    .iter()
                    .any(|l| l.text.contains(&url) && l.text.contains("Local:"))
            };
            if ready_line && http_ready(port).await {
                match child.try_wait() {
                    Ok(None) => {
                        self.set(generation, PreviewState::Running { url: url.clone() })
                            .await;
                        break;
                    }
                    Ok(Some(status)) => {
                        cleanup_exited_group(child_pid).await;
                        drain_logs(&mut readers).await;
                        return Err(PreviewFailure::Exited {
                            message: format!("Vite exited with {status}: {}", self.tail().await),
                        });
                    }
                    Err(e) => {
                        terminate(&mut child).await;
                        drain_logs(&mut readers).await;
                        return Err(PreviewFailure::Exited {
                            message: e.to_string(),
                        });
                    }
                }
            }
        }
        tokio::select! {
            _ = &mut *cancel => { terminate(&mut child).await; Ok(()) }
            result = child.wait() => { cleanup_exited_group(child_pid).await; drain_logs(&mut readers).await; Err(PreviewFailure::Exited { message: format!("Vite exited unexpectedly ({}): {}", result.map(|s| s.to_string()).unwrap_or_else(|e| e.to_string()), self.tail().await) }) },
        }
    }
}

fn snapshot(state: &State) -> PreviewStatusResponse {
    PreviewStatusResponse {
        project_path: state
            .project
            .as_ref()
            .map(|p| p.to_string_lossy().into_owned()),
        generation: state.generation,
        state: state.state.clone(),
    }
}

fn package_fingerprint(project: &Path) -> Result<String, String> {
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    for name in ["package.json", "package-lock.json"] {
        let bytes =
            std::fs::read(project.join(name)).map_err(|e| format!("Cannot read {name}: {e}"))?;
        hash.write_u64(bytes.len() as u64);
        hash.write(&bytes);
    }
    Ok(format!("{:016x}\n", hash.finish()))
}

fn preferred_path(node: &str) -> std::ffi::OsString {
    let dir = Path::new(node).parent().unwrap_or(Path::new("."));
    let mut paths = vec![dir.to_path_buf()];
    paths.extend(std::env::split_paths(
        &std::env::var_os("PATH").unwrap_or_default(),
    ));
    std::env::join_paths(paths).unwrap_or_else(|_| std::env::var_os("PATH").unwrap_or_default())
}

fn free_port() -> Result<u16, String> {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
    listener
        .local_addr()
        .map(|a| a.port())
        .map_err(|e| e.to_string())
}

fn spawn(mut command: Command) -> Result<Child, String> {
    command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.as_std_mut().process_group(0);
    }
    command.spawn().map_err(|e| e.to_string())
}

fn capture(
    manager: PreviewManager,
    generation: u64,
    child: &mut Child,
) -> Vec<tokio::task::JoinHandle<()>> {
    let mut readers = Vec::new();
    for (stream, pipe) in [
        (
            PreviewLogStream::Stdout,
            child
                .stdout
                .take()
                .map(|p| Box::new(p) as Box<dyn tokio::io::AsyncRead + Send + Unpin>),
        ),
        (
            PreviewLogStream::Stderr,
            child
                .stderr
                .take()
                .map(|p| Box::new(p) as Box<dyn tokio::io::AsyncRead + Send + Unpin>),
        ),
    ] {
        if let Some(pipe) = pipe {
            let manager = manager.clone();
            readers.push(tokio::spawn(async move {
                let mut lines = BufReader::new(pipe).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    manager.log(generation, stream.clone(), line).await;
                }
            }));
        }
    }
    readers
}

async fn drain_logs(readers: &mut Vec<tokio::task::JoinHandle<()>>) {
    while let Some(reader) = readers.pop() {
        let _ = timeout(Duration::from_millis(300), reader).await;
    }
}

async fn terminate(child: &mut Child) {
    let pid = child.id();
    #[cfg(unix)]
    if let Some(pid) = pid {
        signal_group(pid, libc::SIGTERM);
    }
    #[cfg(not(unix))]
    {
        let _ = child.start_kill();
    }
    if timeout(Duration::from_secs(2), child.wait()).await.is_err() {
        #[cfg(unix)]
        if let Some(pid) = pid {
            signal_group(pid, libc::SIGKILL);
        }
        let _ = child.start_kill();
        let _ = child.wait().await;
    }
    cleanup_exited_group(pid).await;
}

#[cfg(unix)]
fn signal_group(pid: u32, signal: i32) {
    unsafe {
        libc::kill(-(pid as i32), signal);
    }
}

async fn cleanup_exited_group(pid: Option<u32>) {
    #[cfg(unix)]
    if let Some(pid) = pid {
        if unsafe { libc::kill(-(pid as i32), 0) } == 0 {
            signal_group(pid, libc::SIGTERM);
            sleep(Duration::from_millis(200)).await;
            if unsafe { libc::kill(-(pid as i32), 0) } == 0 {
                signal_group(pid, libc::SIGKILL);
            }
        }
    }
    #[cfg(not(unix))]
    let _ = pid;
}

async fn http_ready(port: u16) -> bool {
    let result = timeout(Duration::from_millis(350), async {
        let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", port)).await?;
        stream
            .write_all(b"GET / HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n")
            .await?;
        let mut line = String::new();
        BufReader::new(stream).read_line(&mut line).await?;
        Ok::<_, std::io::Error>(
            line.starts_with("HTTP/1.1 200") || line.starts_with("HTTP/1.0 200"),
        )
    })
    .await;
    matches!(result, Ok(Ok(true)))
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use protopie_api::{RuntimeExecutable, RuntimeStatus, REQUIRED_NODE_RANGE};
    use std::os::unix::fs::PermissionsExt;

    struct Fixture {
        _temp: tempfile::TempDir,
        project: PathBuf,
        manager: PreviewManager,
    }

    impl Fixture {
        fn new(npm_mode: &str, node_mode: &str) -> Self {
            let temp = tempfile::tempdir().unwrap();
            let project = protopie_ui_agent::init_named(temp.path(), "sample").unwrap();
            let npm = temp.path().join("npm");
            let node = temp.path().join("node");
            let npm_script = format!(
                "#!/bin/sh\necho npm-start\n{}\n",
                match npm_mode {
                    "fail" => "echo install-failed >&2; exit 9".to_string(),
                    "slow" => "sleep 30".to_string(),
                    "logs" =>
                        "i=0; while [ $i -lt 350 ]; do echo line-$i; i=$((i+1)); done".to_string(),
                    _ => "true".to_string(),
                }
            );
            let npm_script = format!(
                "{npm_script}mkdir -p node_modules/vite/bin\ntouch node_modules/vite/bin/vite.js\n"
            );
            std::fs::write(&npm, npm_script).unwrap();
            std::fs::set_permissions(&npm, std::fs::Permissions::from_mode(0o755)).unwrap();
            let node_script = format!(
                r##"#!/usr/bin/env python3
import os, socket, sys, time
args = sys.argv
port = int(args[args.index('--port') + 1])
open('.protopie/fake-vite-pid', 'w').write(str(os.getpid()))
print('vite-start', flush=True)
mode = '{}'
if mode == 'early':
    print('vite-early-failure', file=sys.stderr, flush=True)
    sys.exit(3)
if mode == 'slow':
    time.sleep(30)
s = socket.socket()
s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
s.bind(('127.0.0.1', port))
s.listen(5)
print('  Local: http://127.0.0.1:%d/' % port, flush=True)
if mode == 'no_http':
    time.sleep(30)
while True:
    c, a = s.accept()
    c.recv(4096)
    c.sendall(b'HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok')
    c.close()
"##,
                node_mode
            );
            std::fs::write(&node, node_script).unwrap();
            std::fs::set_permissions(&node, std::fs::Permissions::from_mode(0o755)).unwrap();
            let node_path = node.to_string_lossy().into_owned();
            let npm_path = npm.to_string_lossy().into_owned();
            let manager = PreviewManager::with_runtime(Arc::new(move || RuntimeCheckResponse {
                node: Some(RuntimeExecutable {
                    path: node_path.clone(),
                    version: "v22.12.0".into(),
                }),
                npm: Some(RuntimeExecutable {
                    path: npm_path.clone(),
                    version: "10.0.0".into(),
                }),
                required_node_range: REQUIRED_NODE_RANGE.into(),
                status: RuntimeStatus::Available,
            }));
            Self {
                _temp: temp,
                project,
                manager,
            }
        }

        async fn launch(&self) -> PreviewStatusResponse {
            self.manager
                .launch(self.project.to_str().unwrap(), false)
                .await
        }
        async fn until(&self, target: fn(&PreviewState) -> bool) -> PreviewStatusResponse {
            for _ in 0..100 {
                let status = self.manager.status().await;
                if target(&status.state) {
                    return status;
                }
                sleep(Duration::from_millis(50)).await;
            }
            panic!(
                "preview did not reach target: {:?}",
                self.manager.status().await
            );
        }
    }

    fn running(s: &PreviewState) -> bool {
        matches!(s, PreviewState::Running { .. })
    }
    fn failed(s: &PreviewState) -> bool {
        matches!(s, PreviewState::Failed { .. })
    }
    fn starting(s: &PreviewState) -> bool {
        matches!(s, PreviewState::Starting)
    }

    #[tokio::test]
    async fn install_start_repeat_stop_and_reap() {
        let f = Fixture::new("ok", "ok");
        assert!(matches!(
            f.launch().await.state,
            PreviewState::Preparing { .. }
        ));
        let status = f.until(running).await;
        assert!(matches!(status.state, PreviewState::Running { .. }));
        assert!(f.project.join(".protopie/preview-install-stamp").exists());
        assert_eq!(f.launch().await.generation, status.generation);
        let pid: i32 = std::fs::read_to_string(f.project.join(".protopie/fake-vite-pid"))
            .unwrap()
            .parse()
            .unwrap();
        assert!(matches!(
            f.manager.stop().await.state,
            PreviewState::Stopped
        ));
        assert_eq!(unsafe { libc::kill(pid, 0) }, -1);
    }

    #[tokio::test]
    async fn install_failure_and_early_exit_keep_log_tail() {
        let f = Fixture::new("fail", "ok");
        f.launch().await;
        let status = f.until(failed).await;
        assert!(
            matches!(status.state, PreviewState::Failed { reason: PreviewFailure::Install { ref message } } if message.contains("install-failed"))
        );
        assert!(!f.project.join(".protopie/preview-install-stamp").exists());
        let f = Fixture::new("ok", "early");
        f.launch().await;
        let status = f.until(failed).await;
        assert!(
            matches!(status.state, PreviewState::Failed { reason: PreviewFailure::Exited { ref message } } if message.contains("vite-early-failure"))
        );
    }

    #[tokio::test]
    async fn readiness_timeout_and_occupied_port_do_not_report_running() {
        let f = Fixture::new("ok", "no_http");
        f.launch().await;
        assert!(matches!(
            f.until(failed).await.state,
            PreviewState::Failed {
                reason: PreviewFailure::Readiness { .. }
            }
        ));
        let occupied = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = occupied.local_addr().unwrap().port();
        let mut f = Fixture::new("ok", "ok");
        Arc::get_mut(&mut f.manager.0).unwrap().port = Arc::new(move || Ok(port));
        f.launch().await;
        assert!(matches!(
            f.until(failed).await.state,
            PreviewState::Failed {
                reason: PreviewFailure::Exited { .. }
            }
        ));
    }

    #[tokio::test]
    async fn stop_during_install_and_start_and_project_switch() {
        let f = Fixture::new("slow", "ok");
        f.launch().await;
        f.until(
            |s| matches!(s, PreviewState::Preparing { step } if step == "Installing dependencies"),
        )
        .await;
        assert!(matches!(
            f.manager.stop().await.state,
            PreviewState::Stopped
        ));
        assert!(!f.project.join(".protopie/preview-install-stamp").exists());
        let f = Fixture::new("ok", "slow");
        f.launch().await;
        f.until(starting).await;
        assert!(matches!(
            f.manager.stop().await.state,
            PreviewState::Stopped
        ));
        let f = Fixture::new("ok", "ok");
        f.launch().await;
        f.until(running).await;
        let next = protopie_ui_agent::init_named(f._temp.path(), "other").unwrap();
        let status = f.manager.launch(next.to_str().unwrap(), false).await;
        assert_eq!(status.project_path.as_deref(), next.to_str());
        f.manager.stop().await;
    }

    #[tokio::test]
    async fn log_cursor_is_bounded_ordered_and_project_scoped() {
        let f = Fixture::new("logs", "ok");
        f.launch().await;
        f.until(running).await;
        let logs = f
            .manager
            .logs(PreviewLogRequest {
                project_path: f.project.to_string_lossy().into(),
                cursor: 0,
            })
            .await;
        assert!(logs.truncated);
        assert!(logs.lines.len() <= LOG_CAP);
        assert!(logs.lines.windows(2).all(|x| x[0].cursor < x[1].cursor));
        let empty = f
            .manager
            .logs(PreviewLogRequest {
                project_path: f.project.to_string_lossy().into(),
                cursor: logs.next_cursor,
            })
            .await;
        assert!(empty.lines.is_empty());
        let other = f
            .manager
            .logs(PreviewLogRequest {
                project_path: "/other".into(),
                cursor: 0,
            })
            .await;
        assert!(other.lines.is_empty());
        f.manager.stop().await;
    }

    #[tokio::test]
    async fn unsupported_project_fails_before_install() {
        let f = Fixture::new("ok", "ok");
        std::fs::remove_file(f.project.join("src/pages/Home.tsx")).unwrap();
        let status = f.launch().await;
        assert!(matches!(
            status.state,
            PreviewState::Failed {
                reason: PreviewFailure::UnsupportedProject { .. }
            }
        ));
        assert!(!f.project.join("node_modules").exists());
    }

    #[tokio::test]
    #[ignore = "requires Node/npm and installs the reference project"]
    async fn real_first_install_serves_and_reaps() {
        let temp = tempfile::tempdir().unwrap();
        let project = protopie_ui_agent::init_named(temp.path(), "real-preview").unwrap();
        let runtime = crate::runtime::check_runtime();
        assert!(
            matches!(runtime.status, RuntimeStatus::Available),
            "Node/npm prerequisites: {runtime:?}"
        );
        let manager = PreviewManager::new();
        manager.launch(project.to_str().unwrap(), false).await;
        let result = async {
            let deadline = Instant::now() + Duration::from_secs(330);
            loop {
                let status = manager.status().await;
                match status.state {
                    PreviewState::Running { url } => {
                        let pid = manager
                            .0
                            .state
                            .lock()
                            .await
                            .child_pid
                            .ok_or("missing child pid")?;
                        let port: u16 = url
                            .trim_end_matches('/')
                            .rsplit(':')
                            .next()
                            .unwrap()
                            .parse()
                            .unwrap();
                        if !http_ready(port).await {
                            return Err("Vite URL did not respond".to_string());
                        }
                        return Ok(pid);
                    }
                    PreviewState::Failed { reason } => {
                        return Err(format!("preview failed: {reason:?}"))
                    }
                    _ => {}
                }
                if Instant::now() >= deadline {
                    return Err("preview hung".to_string());
                }
                sleep(Duration::from_millis(200)).await;
            }
        }
        .await;
        let stopped = manager.stop().await;
        assert!(matches!(stopped.state, PreviewState::Stopped));
        let pid = result.expect("real first-install preview");
        assert_eq!(
            unsafe { libc::kill(pid as i32, 0) },
            -1,
            "Vite child was not reaped"
        );
    }
}
