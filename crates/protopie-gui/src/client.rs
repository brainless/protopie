//! Non-blocking HTTP client: requests run on worker threads, typed events come back over a
//! channel that the render loop polls once per frame.

use std::sync::mpsc::{channel, Receiver, Sender};

use protopie_api::{
    AbortProjectRequest, AbortProjectResponse, ChatRequest, ChatResponse, CreateProjectRequest,
    CreateProjectResponse, ErrorResponse, ListProjectsResponse, ModifyProjectRequest,
    ModifyProjectResponse, ProjectInfo, CHAT_PATH, PROJECTS_ABORT_PATH, PROJECTS_MODIFY_PATH,
    PROJECTS_PATH,
};
use serde::de::DeserializeOwned;
use serde::Serialize;

/// Result of a finished request. Errors are display-ready strings.
pub enum Event {
    /// Reply to a prompt, from either `/chat` or the modify endpoint.
    Reply(Result<String, String>),
    Projects(Result<Vec<ProjectInfo>, String>),
    ProjectCreated(Result<CreateProjectResponse, String>),
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

    /// Prompt for a selected project. Every send is a deliberate new request
    /// (fresh ID) in the GUI's conversation, whose focus and pending questions
    /// the project keeps across restarts.
    pub fn modify(&self, project_path: String, command: String) {
        let request_id = new_request_id();
        self.spawn(move |base| {
            let r = post_json::<_, ModifyProjectResponse>(
                &format!("{base}{PROJECTS_MODIFY_PATH}"),
                ModifyProjectRequest {
                    project_path,
                    command,
                    conversation_id: Some(CONVERSATION_ID.into()),
                    request_id: Some(request_id),
                    dry_run: false,
                },
            );
            Event::Reply(r.map(|r| r.display_text()))
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
}
