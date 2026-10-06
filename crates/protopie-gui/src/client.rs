//! Non-blocking HTTP client: requests run on worker threads, typed events come back over a
//! channel that the render loop polls once per frame.

use std::sync::mpsc::{channel, Receiver, Sender};

use crate::preview::Action;
use crate::review::{Expected, Sent};
use protopie_api::{
    AbortProjectRequest, AbortProjectResponse, AnswerProjectRequest, ChatRequest, ChatResponse,
    CreateProjectRequest, CreateProjectResponse, ErrorResponse, ListProjectsResponse,
    ModifyProjectRequest, ModifyProjectResponse, PreviewLogRequest, PreviewLogResponse,
    PreviewProjectRequest, PreviewStatusResponse, ProjectInfo, RuntimeCheckResponse, CHAT_PATH,
    PREVIEW_LAUNCH_PATH, PREVIEW_LOGS_PATH, PREVIEW_RESTART_PATH, PREVIEW_RUNTIME_PATH,
    PREVIEW_STATUS_PATH, PREVIEW_STOP_PATH, PROJECTS_ABORT_PATH, PROJECTS_ANSWER_PATH,
    PROJECTS_MODIFY_PATH, PROJECTS_PATH,
};
use serde::de::DeserializeOwned;
use serde::Serialize;

/// Result of a finished request. Errors are display-ready strings.
pub enum Event {
    /// Reply to a prompt, from either `/chat` or the modify endpoint.
    Reply(Result<String, String>),
    /// Reply to a project command or answer. `applied` is false for a preview.
    Modified {
        applied: bool,
        project_path: String,
        request_id: String,
        result: Result<ModifyProjectResponse, String>,
    },
    Projects(Result<Vec<ProjectInfo>, String>),
    ProjectCreated(Result<CreateProjectResponse, String>),
    PreviewRuntime {
        epoch: u64,
        result: Result<RuntimeCheckResponse, String>,
    },
    PreviewStatus {
        epoch: u64,
        operation: bool,
        result: Result<PreviewStatusResponse, String>,
    },
    PreviewLogs {
        epoch: u64,
        result: Result<PreviewLogResponse, String>,
    },
}

pub struct ApiClient {
    base_url: String,
    tx: Sender<Event>,
    rx: Receiver<Event>,
}

/// Maps a ureq error to a message, preferring the server's JSON `error` field.
fn describe(e: ureq::Error) -> String {
    match e {
        ureq::Error::Status(code, resp) => match resp.into_json::<ErrorResponse>() {
            Ok(body) => body.error,
            Err(_) => format!("server returned status {code}"),
        },
        other => other.to_string(),
    }
}

fn get_json<T: DeserializeOwned>(url: &str) -> Result<T, String> {
    ureq::get(url)
        .call()
        .map_err(describe)?
        .into_json()
        .map_err(|e| e.to_string())
}

fn post_json<B: Serialize, T: DeserializeOwned>(url: &str, body: B) -> Result<T, String> {
    ureq::post(url)
        .send_json(body)
        .map_err(describe)?
        .into_json()
        .map_err(|e| e.to_string())
}

impl ApiClient {
    pub fn new(base_url: String) -> Self {
        let (tx, rx) = channel();
        Self { base_url, tx, rx }
    }

    /// Runs `work` on a worker thread and queues its event.
    fn spawn(&self, work: impl FnOnce(&str) -> Event + Send + 'static) {
        let base = self.base_url.clone();
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(work(&base));
        });
    }

    /// Prompt with no project selected: the server echoes it.
    pub fn chat(&self, prompt: String) {
        self.spawn(move |base| {
            let r =
                post_json::<_, ChatResponse>(&format!("{base}{CHAT_PATH}"), ChatRequest { prompt });
            Event::Reply(r.map(|r| r.reply))
        });
    }

    /// Request for a selected project in the GUI's conversation, whose focus
    /// and pending questions the project keeps across restarts. Without
    /// `expected` it is a dry run (a preview, nothing written); with it, the
    /// change is applied only against the previewed plan and revision. Every
    /// apply is a deliberate new request (fresh ID).
    pub fn send(&self, project_path: String, sent: Sent, expected: Option<Expected>) {
        let request_id = new_request_id();
        let applied = expected.is_some();
        let input = match &sent {
            Sent::Command(command) => serde_json::json!({
                "kind": "command", "command": protopie_api::diagnostics::bounded_text(command),
            }),
            Sent::Answer {
                question_id,
                option_key,
            } => serde_json::json!({
                "kind": "answer", "question_id": question_id,
                "option_key": protopie_api::diagnostics::bounded_text(option_key),
            }),
        };
        let _ = protopie_api::diagnostics::log_project_event(
            std::path::Path::new(&project_path),
            "gui",
            serde_json::json!({
                "kind": "request", "request_id": request_id, "conversation_id": CONVERSATION_ID,
                "dry_run": !applied, "input": input,
                "expected_revision": expected.as_ref().map(|e| e.revision),
                "expected_plan_id": expected.as_ref().map(|e| &e.plan_id),
            }),
        );
        self.spawn(move |base| {
            let (expected_revision, expected_plan_id) = match expected {
                Some(e) => (Some(e.revision), Some(e.plan_id)),
                None => (None, None),
            };
            let result = match sent {
                Sent::Command(command) => post_json(
                    &format!("{base}{PROJECTS_MODIFY_PATH}"),
                    ModifyProjectRequest {
                        project_path: project_path.clone(),
                        command,
                        conversation_id: Some(CONVERSATION_ID.into()),
                        request_id: Some(request_id.clone()),
                        dry_run: !applied,
                        expected_revision,
                        expected_plan_id,
                    },
                ),
                Sent::Answer {
                    question_id,
                    option_key,
                } => post_json(
                    &format!("{base}{PROJECTS_ANSWER_PATH}"),
                    AnswerProjectRequest {
                        project_path: project_path.clone(),
                        question_id,
                        option_key: Some(option_key),
                        text: None,
                        conversation_id: Some(CONVERSATION_ID.into()),
                        request_id: Some(request_id.clone()),
                        dry_run: !applied,
                        expected_revision,
                        expected_plan_id,
                    },
                ),
            };
            Event::Modified {
                applied,
                project_path,
                request_id,
                result,
            }
        });
    }

    pub fn abort_interrupted_apply(&self, project_path: String) {
        self.spawn(move |base| {
            let result = post_json::<_, AbortProjectResponse>(
                &format!("{base}{PROJECTS_ABORT_PATH}"),
                AbortProjectRequest { project_path },
            );
            Event::Reply(result.map(|r| r.display_text()))
        });
    }

    pub fn list_projects(&self) {
        self.spawn(|base| {
            let r = get_json::<ListProjectsResponse>(&format!("{base}{PROJECTS_PATH}"));
            Event::Projects(r.map(|r| r.projects))
        });
    }

    pub fn create_project(&self, base_path: String, name: String) {
        self.spawn(move |base| {
            Event::ProjectCreated(post_json(
                &format!("{base}{PROJECTS_PATH}"),
                CreateProjectRequest { base_path, name },
            ))
        });
    }

    pub fn preview_runtime(&self, epoch: u64) {
        self.spawn(move |base| Event::PreviewRuntime {
            epoch,
            result: get_json(&format!("{base}{PREVIEW_RUNTIME_PATH}")),
        });
    }

    pub fn preview_status(&self, epoch: u64) {
        self.spawn(move |base| Event::PreviewStatus {
            epoch,
            operation: false,
            result: get_json(&format!("{base}{PREVIEW_STATUS_PATH}")),
        });
    }

    pub fn preview_action(&self, epoch: u64, action: Action, project_path: String) {
        self.spawn(move |base| {
            let result = match action {
                Action::Launch => post_json(
                    &format!("{base}{PREVIEW_LAUNCH_PATH}"),
                    PreviewProjectRequest { project_path },
                ),
                Action::Restart => post_json(
                    &format!("{base}{PREVIEW_RESTART_PATH}"),
                    PreviewProjectRequest { project_path },
                ),
                Action::Stop => post_json(
                    &format!("{base}{PREVIEW_STOP_PATH}"),
                    PreviewProjectRequest { project_path },
                ),
            };
            Event::PreviewStatus {
                epoch,
                operation: true,
                result,
            }
        });
    }

    pub fn preview_logs(&self, epoch: u64, project_path: String, cursor: u64) {
        self.spawn(move |base| Event::PreviewLogs {
            epoch,
            result: post_json(
                &format!("{base}{PREVIEW_LOGS_PATH}"),
                PreviewLogRequest {
                    project_path,
                    cursor,
                },
            ),
        });
    }

    pub fn try_recv(&self) -> Option<Event> {
        self.rx.try_recv().ok()
    }
}

/// Conversation the chat view uses in every project.
pub const CONVERSATION_ID: &str = "gui";

/// Unique per send, also across GUI restarts.
fn new_request_id() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    format!(
        "gui-{}-{nanos}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_ids_are_unique_and_conversation_id_is_valid() {
        let a = new_request_id();
        assert_ne!(a, new_request_id());
        assert!(CONVERSATION_ID
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-'));
    }

    #[test]
    fn gui_request_log_keeps_prompt_and_correlation_id() {
        let temp = tempfile::tempdir().unwrap();
        let project = temp.path().join("p");
        std::fs::create_dir(&project).unwrap();
        let client = ApiClient::new("http://127.0.0.1:1".into());
        client.send(
            project.to_string_lossy().into_owned(),
            Sent::Command("Share the selected doctor across pages".into()),
            None,
        );
        let dir = protopie_api::diagnostics::project_log_dir(&project).unwrap();
        let log = std::fs::read_to_string(dir.join("events.jsonl")).unwrap();
        let entry: serde_json::Value = serde_json::from_str(log.lines().last().unwrap()).unwrap();
        assert_eq!(entry["source"], "gui");
        assert_eq!(
            entry["event"]["input"]["command"],
            "Share the selected doctor across pages"
        );
        assert_eq!(entry["event"]["dry_run"], true);
        assert!(entry["event"]["request_id"]
            .as_str()
            .unwrap()
            .starts_with("gui-"));
    }
}
