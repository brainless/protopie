//! Preview presentation state. Network requests carry an epoch so replies from a
//! previous selection or operation cannot change the current project's controls.

use std::time::{Duration, Instant};

use protopie_api::{
    PreviewFailure, PreviewLogResponse, PreviewState, PreviewStatusResponse, RuntimeCheckResponse,
    RuntimeFailure, RuntimeStatus,
};

const POLL_INTERVAL: Duration = Duration::from_secs(1);
const MAX_VISIBLE_LOGS: usize = 40;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Launch,
    Stop,
    Restart,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Controls {
    pub launch: bool,
    pub stop: bool,
    pub restart: bool,
    pub open: bool,
}

pub struct PreviewUi {
    project_path: Option<String>,
    epoch: u64,
    generation: u64,
    state: PreviewState,
    runtime_error: Option<String>,
    request_error: Option<String>,
    logs: Vec<String>,
    cursor: u64,
    pub logs_open: bool,
    operation_pending: bool,
    status_pending: bool,
    logs_pending: bool,
    open_when_running: bool,
    next_poll: Instant,
}

impl PreviewUi {
    pub fn new() -> Self {
        Self {
            project_path: None,
            epoch: 0,
            generation: 0,
            state: PreviewState::Stopped,
            runtime_error: None,
            request_error: None,
            logs: Vec::new(),
            cursor: 0,
            logs_open: false,
            operation_pending: false,
            status_pending: false,
            logs_pending: false,
            open_when_running: false,
            next_poll: Instant::now(),
        }
    }

    /// Returns a new epoch only when the selected project changes.
    pub fn select(&mut self, path: Option<&str>) -> Option<u64> {
        if self.project_path.as_deref() == path {
            return None;
        }
        self.epoch = self.epoch.wrapping_add(1);
        self.project_path = path.map(str::to_owned);
        self.reset();
        self.project_path.as_ref().map(|_| self.epoch)
    }

    fn reset(&mut self) {
        self.generation = 0;
        self.state = PreviewState::Stopped;
        self.runtime_error = None;
        self.request_error = None;
        self.logs.clear();
        self.cursor = 0;
        self.logs_open = false;
        self.operation_pending = false;
        self.status_pending = false;
        self.logs_pending = false;
        self.open_when_running = false;
        self.next_poll = Instant::now();
    }

    pub fn controls(&self) -> Controls {
        if self.project_path.is_none() {
            return Controls {
                launch: false,
                stop: false,
                restart: false,
                open: false,
            };
        }
        let running = matches!(self.state, PreviewState::Running { .. });
        let active = matches!(
            self.state,
            PreviewState::Preparing { .. } | PreviewState::Starting | PreviewState::Running { .. }
        );
        Controls {
            launch: !active && !self.operation_pending,
            stop: active && !self.operation_pending,
            restart: active && !self.operation_pending,
            open: running && !self.operation_pending,
        }
    }

    pub fn begin_action(&mut self, action: Action) -> Option<(u64, String)> {
        let allowed = match action {
            Action::Launch => self.controls().launch,
            Action::Stop => self.controls().stop,
            Action::Restart => self.controls().restart,
        };
        if !allowed {
            return None;
        }
        let path = self.project_path.clone()?;
        self.epoch = self.epoch.wrapping_add(1);
        self.operation_pending = true;
        self.status_pending = false;
        self.logs_pending = false;
        self.request_error = None;
        self.open_when_running = matches!(action, Action::Launch | Action::Restart);
        if self.open_when_running {
            self.logs.clear();
            self.cursor = 0;
            self.state = PreviewState::Preparing {
                step: "Requesting preview…".into(),
            };
        }
        Some((self.epoch, path))
    }

    pub fn mark_status_requested(&mut self) -> Option<u64> {
        if self.project_path.is_none() || self.status_pending {
            return None;
        }
        self.status_pending = true;
        Some(self.epoch)
    }

    pub fn mark_logs_requested(&mut self) -> Option<(u64, String, u64)> {
        if self.project_path.is_none() || self.logs_pending {
            return None;
        }
        self.logs_pending = true;
        Some((self.epoch, self.project_path.clone()?, self.cursor))
    }

    pub fn poll_due(&mut self, now: Instant) -> bool {
        if self.project_path.is_none()
            || now < self.next_poll
            || self.operation_pending
            || self.status_pending
        {
            return false;
        }
        if !matches!(
            self.state,
            PreviewState::Preparing { .. } | PreviewState::Starting | PreviewState::Running { .. }
        ) {
            return false;
        }
        self.next_poll = now + POLL_INTERVAL;
        true
    }

    pub fn receive_runtime(&mut self, epoch: u64, result: Result<RuntimeCheckResponse, String>) {
        if epoch != self.epoch {
            return;
        }
        self.runtime_error = match result {
            Ok(RuntimeCheckResponse {
                status: RuntimeStatus::Unavailable { reason },
                ..
            }) => Some(runtime_message(&reason)),
            Ok(_) => None,
            Err(e) => Some(format!("Runtime check failed: {e}")),
        };
    }

    pub fn receive_status(
        &mut self,
        epoch: u64,
        operation: bool,
        result: Result<PreviewStatusResponse, String>,
    ) -> Option<String> {
        if epoch != self.epoch {
            return None;
        }
        if operation {
            self.operation_pending = false;
        } else {
            self.status_pending = false;
        }
        let status = match result {
            Ok(status) => status,
            Err(e) => {
                self.request_error = Some(format!("Preview request failed: {e}"));
                self.open_when_running = false;
                return None;
            }
        };
        if status.generation < self.generation {
            return None;
        }
        if status.project_path.as_deref() != self.project_path.as_deref() {
            // The server owns one preview. Another selected project is shown as stopped.
            self.state = PreviewState::Stopped;
            self.open_when_running = false;
            self.logs.clear();
            self.cursor = 0;
            return None;
        }
        if status.generation != self.generation {
            self.logs.clear();
            self.cursor = 0;
        }
        self.generation = status.generation;
        self.state = status.state;
        self.request_error = None;
        self.next_poll = Instant::now() + POLL_INTERVAL;
        if let PreviewState::Running { url } = &self.state {
            if self.open_when_running {
                self.open_when_running = false;
                return Some(url.clone());
            }
        } else if matches!(
            self.state,
            PreviewState::Stopped | PreviewState::Failed { .. }
        ) {
            self.open_when_running = false;
        }
        None
    }

    pub fn receive_logs(&mut self, epoch: u64, result: Result<PreviewLogResponse, String>) {
        if epoch != self.epoch {
            return;
        }
        self.logs_pending = false;
        match result {
            Ok(response)
                if response.project_path.as_deref() == self.project_path.as_deref()
                    && response.generation == self.generation =>
            {
                if response.truncated {
                    self.logs
                        .push("… earlier preview logs were discarded …".into());
                }
                self.logs.extend(
                    response
                        .lines
                        .into_iter()
                        .map(|line| format!("[{:?}] {}", line.stream, line.text)),
                );
                if self.logs.len() > MAX_VISIBLE_LOGS {
                    self.logs.drain(..self.logs.len() - MAX_VISIBLE_LOGS);
                }
                self.cursor = response.next_cursor;
            }
            Err(e) => self.request_error = Some(format!("Could not read preview logs: {e}")),
            _ => {}
        }
    }

    pub fn status_text(&self) -> String {
        if let Some(e) = &self.request_error {
            return e.clone();
        }
        match &self.state {
            PreviewState::Stopped => self
                .runtime_error
                .clone()
                .unwrap_or_else(|| "Preview stopped".into()),
            PreviewState::Preparing { step } => format!("Preparing preview: {step}"),
            PreviewState::Starting => "Starting Vite…".into(),
            PreviewState::Running { url } => format!("Running at {url}"),
            PreviewState::Failed { reason } => failure_message(reason),
        }
    }

    pub fn visible_logs(&self) -> &[String] {
        &self.logs
    }
    pub fn running_url(&self) -> Option<&str> {
        match &self.state {
            PreviewState::Running { url } => Some(url),
            _ => None,
        }
    }
}

fn runtime_message(reason: &RuntimeFailure) -> String {
    match reason {
        RuntimeFailure::MissingExecutable { message, .. }
        | RuntimeFailure::UnsupportedVersion { message, .. }
        | RuntimeFailure::CommandFailed { message, .. } => message.clone(),
    }
}

fn failure_message(reason: &PreviewFailure) -> String {
    match reason {
        PreviewFailure::Environment { reason } => runtime_message(reason),
        PreviewFailure::UnsupportedProject { message }
        | PreviewFailure::Install { message }
        | PreviewFailure::Start { message }
        | PreviewFailure::Readiness { message }
        | PreviewFailure::Exited { message } => message.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use protopie_api::{PreviewLogLine, PreviewLogStream};

    fn status(path: &str, generation: u64, state: PreviewState) -> PreviewStatusResponse {
        PreviewStatusResponse {
            project_path: Some(path.into()),
            generation,
            state,
        }
    }

    #[test]
    fn controls_and_transitions() {
        let mut ui = PreviewUi::new();
        assert!(!ui.controls().launch);
        ui.select(Some("/a"));
        assert!(ui.controls().launch);
        let (epoch, _) = ui.begin_action(Action::Launch).unwrap();
        assert!(!ui.controls().launch);
        assert!(ui
            .receive_status(
                epoch,
                true,
                Ok(status(
                    "/a",
                    1,
                    PreviewState::Preparing {
                        step: "npm ci".into()
                    }
                ))
            )
            .is_none());
        assert!(ui.controls().stop);
        assert!(!ui.controls().open);
        assert!(ui
            .receive_status(epoch, false, Ok(status("/a", 1, PreviewState::Starting)))
            .is_none());
        let url = "http://127.0.0.1:1234/";
        assert_eq!(
            ui.receive_status(
                epoch,
                false,
                Ok(status("/a", 1, PreviewState::Running { url: url.into() }))
            ),
            Some(url.into())
        );
        assert!(ui.controls().open);
        assert!(ui
            .receive_status(
                epoch,
                false,
                Ok(status("/a", 1, PreviewState::Running { url: url.into() }))
            )
            .is_none());
        let (epoch, _) = ui.begin_action(Action::Stop).unwrap();
        ui.receive_status(epoch, true, Ok(status("/a", 2, PreviewState::Stopped)));
        assert!(ui.controls().launch);
        assert!(!ui.controls().open);
    }

    #[test]
    fn discovered_running_preview_does_not_auto_open_and_polling_stops_on_switch() {
        let mut ui = PreviewUi::new();
        let epoch = ui.select(Some("/a")).unwrap();
        ui.receive_status(
            epoch,
            false,
            Ok(status(
                "/a",
                3,
                PreviewState::Running {
                    url: "http://127.0.0.1:1".into(),
                },
            )),
        );
        assert!(ui.controls().open);
        assert!(ui.poll_due(Instant::now() + Duration::from_secs(2)));
        ui.select(None);
        assert!(!ui.poll_due(Instant::now() + Duration::from_secs(2)));
        assert!(!ui.controls().open);
    }

    #[test]
    fn stale_replies_and_generations_are_ignored() {
        let mut ui = PreviewUi::new();
        let old = ui.select(Some("/a")).unwrap();
        ui.select(Some("/b"));
        ui.receive_status(
            old,
            false,
            Ok(status("/a", 4, PreviewState::Running { url: "old".into() })),
        );
        assert!(ui.controls().launch);
        let current = ui.epoch;
        ui.receive_status(
            current,
            false,
            Ok(status("/b", 5, PreviewState::Running { url: "new".into() })),
        );
        ui.receive_status(current, false, Ok(status("/b", 4, PreviewState::Stopped)));
        assert_eq!(ui.running_url(), Some("new"));
        ui.receive_status(
            current,
            false,
            Ok(status(
                "/a",
                6,
                PreviewState::Running {
                    url: "other".into(),
                },
            )),
        );
        assert!(ui.controls().launch);
    }

    #[test]
    fn failures_and_log_cursor_are_displayed() {
        let mut ui = PreviewUi::new();
        let epoch = ui.select(Some("/a")).unwrap();
        ui.receive_runtime(
            epoch,
            Ok(RuntimeCheckResponse {
                node: None,
                npm: None,
                required_node_range: "^20.19.0 || >=22.12.0".into(),
                status: RuntimeStatus::Unavailable {
                    reason: RuntimeFailure::MissingExecutable {
                        tool: "node".into(),
                        message: "Install Node".into(),
                    },
                },
            }),
        );
        assert_eq!(ui.status_text(), "Install Node");
        ui.receive_status(
            epoch,
            false,
            Ok(status(
                "/a",
                1,
                PreviewState::Failed {
                    reason: PreviewFailure::Install {
                        message: "npm ci failed".into(),
                    },
                },
            )),
        );
        assert_eq!(ui.status_text(), "npm ci failed");
        ui.receive_logs(
            epoch,
            Ok(PreviewLogResponse {
                project_path: Some("/a".into()),
                generation: 1,
                next_cursor: 8,
                truncated: true,
                lines: vec![PreviewLogLine {
                    cursor: 8,
                    stream: PreviewLogStream::Stderr,
                    text: "bad package".into(),
                }],
            }),
        );
        assert_eq!(ui.cursor, 8);
        assert!(ui.visible_logs().iter().any(|s| s.contains("bad package")));
    }

    #[test]
    fn old_operation_and_log_replies_cannot_replace_new_launch() {
        let mut ui = PreviewUi::new();
        ui.select(Some("/a"));
        let (old, _) = ui.begin_action(Action::Launch).unwrap();
        let (current, _) = {
            ui.receive_status(
                old,
                true,
                Ok(status(
                    "/a",
                    1,
                    PreviewState::Running {
                        url: "http://127.0.0.1:1".into(),
                    },
                )),
            );
            ui.begin_action(Action::Restart).unwrap()
        };
        ui.receive_status(current, true, Ok(status("/a", 2, PreviewState::Starting)));
        ui.receive_status(old, false, Ok(status("/a", 1, PreviewState::Stopped)));
        ui.receive_logs(
            old,
            Ok(PreviewLogResponse {
                project_path: Some("/a".into()),
                generation: 1,
                next_cursor: 99,
                truncated: false,
                lines: vec![],
            }),
        );
        assert_eq!(ui.cursor, 0);
        assert_eq!(ui.status_text(), "Starting Vite…");
        assert!(!ui.controls().open);
    }
}
