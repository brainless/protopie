use std::path::{Path, PathBuf};

use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use protopie_api::{
    AbortProjectRequest, AbortProjectResponse, AnswerProjectRequest, ChatRequest, ChatResponse,
    CreateProjectRequest, CreateProjectResponse, ErrorResponse, ListProjectsResponse,
    ModifyOutcome, ModifyProjectRequest, ModifyProjectResponse, ProjectInfo, QuestionSummary,
    CHAT_PATH, HEALTH_PATH, PROJECTS_ABORT_PATH, PROJECTS_ANSWER_PATH, PROJECTS_MODIFY_PATH,
    PROJECTS_PATH,
};
use protopie_ui_agent as agent;

/// Folder `GET /projects` lists. Resolved against the server's cwd when relative.
const PROJECTS_BASE_PATH: &str = ".projects";

/// An HTTP error with a JSON `{"error": ...}` body.
struct ApiError(StatusCode, String);

impl From<agent::Error> for ApiError {
    fn from(e: agent::Error) -> Self {
        let status = match &e {
            agent::Error::InvalidSlug(_)
            | agent::Error::Escape(_)
            | agent::Error::InvalidConversationId(_) => StatusCode::BAD_REQUEST,
            agent::Error::AlreadyExists(_)
            | agent::Error::AlreadyInitialized(_)
            | agent::Error::StaleRevision { .. } => StatusCode::CONFLICT,
            agent::Error::UnsupportedProject(_) => StatusCode::UNPROCESSABLE_ENTITY,
            agent::Error::NotADirectory(_) => StatusCode::NOT_FOUND,
            agent::Error::Io { .. } | agent::Error::Metadata { .. } => {
                StatusCode::INTERNAL_SERVER_ERROR
            }
        };
        ApiError(status, e.to_string())
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(ErrorResponse { error: self.1 })).into_response()
    }
}

/// Runs blocking filesystem work off the async runtime.
async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> agent::Result<T> + Send + 'static,
) -> Result<T, ApiError> {
    match tokio::task::spawn_blocking(f).await {
        Ok(r) => r.map_err(ApiError::from),
        Err(e) => Err(ApiError(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("task failed: {e}"),
        )),
    }
}

fn path_string(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

async fn chat(Json(req): Json<ChatRequest>) -> Json<ChatResponse> {
    Json(ChatResponse { reply: req.prompt })
}

async fn list_projects() -> Result<Json<ListProjectsResponse>, ApiError> {
    let projects = blocking(|| {
        let root = Path::new(PROJECTS_BASE_PATH);
        // Absolute so clients can send the path back regardless of cwd.
        let abs = std::path::absolute(root).unwrap_or_else(|_| root.to_path_buf());
        Ok(agent::list_projects(root)?
            .into_iter()
            .map(|name| ProjectInfo {
                path: path_string(&abs.join(&name)),
                name,
            })
            .collect())
    })
    .await?;
    Ok(Json(ListProjectsResponse { projects }))
}

async fn create_project(
    Json(req): Json<CreateProjectRequest>,
) -> Result<(StatusCode, Json<CreateProjectResponse>), ApiError> {
    let slug = agent::slugify(&req.name);
    let base = PathBuf::from(req.base_path);
    let s = slug.clone();
    let path = blocking(move || agent::init_named(&base, &s)).await?;
    Ok((
        StatusCode::CREATED,
        Json(CreateProjectResponse {
            slug,
            path: path_string(&path),
        }),
    ))
}

async fn modify_project(
    Json(req): Json<ModifyProjectRequest>,
) -> Result<Json<ModifyProjectResponse>, ApiError> {
    let result = blocking(move || {
        agent::modify_with(
            Path::new(&req.project_path),
            &req.command,
            &agent::ModifyOptions {
                conversation_id: req.conversation_id.as_deref(),
                request_id: req.request_id.as_deref(),
                dry_run: req.dry_run,
            },
        )
    })
    .await?;
    Ok(Json(ModifyProjectResponse {
        reply: result.summary,
        outcome: Some(to_wire(result.outcome)),
    }))
}

async fn answer_project(
    Json(req): Json<AnswerProjectRequest>,
) -> Result<Json<ModifyProjectResponse>, ApiError> {
    let choice = match (req.option_key, req.text) {
        (Some(key), None) => agent::contracts::AnswerChoice::Option { key },
        (None, Some(text)) => agent::contracts::AnswerChoice::Text { text },
        _ => {
            return Err(ApiError(
                StatusCode::BAD_REQUEST,
                "send exactly one of option_key or text".into(),
            ))
        }
    };
    let answer = agent::contracts::Answer {
        question_id: req.question_id,
        choice,
    };
    let result = blocking(move || {
        agent::answer_with(
            Path::new(&req.project_path),
            &answer,
            &agent::ModifyOptions {
                conversation_id: req.conversation_id.as_deref(),
                request_id: req.request_id.as_deref(),
                dry_run: req.dry_run,
            },
        )
    })
    .await?;
    Ok(Json(ModifyProjectResponse {
        reply: result.summary,
        outcome: Some(to_wire(result.outcome)),
    }))
}

async fn abort_project(
    Json(req): Json<AbortProjectRequest>,
) -> Result<Json<AbortProjectResponse>, ApiError> {
    let path = PathBuf::from(req.project_path);
    let result = blocking(move || {
        if !path.is_dir() {
            return Err(agent::Error::NotADirectory(path));
        }
        agent::apply::abort_interrupted_apply(&path)
    })
    .await?;
    let response = match result {
        agent::apply::RecoverOutcome::Aborted {
            plan_id,
            preserved_paths,
        } => AbortProjectResponse::Aborted {
            plan_id,
            preserved_paths,
        },
        agent::apply::RecoverOutcome::NothingToDo => AbortProjectResponse::NothingToDo,
        agent::apply::RecoverOutcome::Conflict { reason } => {
            AbortProjectResponse::Conflict { reason }
        }
        agent::apply::RecoverOutcome::RolledForward { .. } => {
            return Err(ApiError(
                StatusCode::INTERNAL_SERVER_ERROR,
                "unexpected recovery result during abort".into(),
            ));
        }
    };
    Ok(Json(response))
}

fn question_summaries(questions: Vec<agent::contracts::Question>) -> Vec<QuestionSummary> {
    questions
        .into_iter()
        .map(|q| QuestionSummary {
            blocking: q.kind == agent::contracts::QuestionKind::BlockingClarification,
            takes_text: q.continuation.takes_text(),
            id: q.id,
            prompt: q.prompt,
            option_keys: q.options.iter().map(|o| o.key.clone()).collect(),
            options: q.options.into_iter().map(|o| o.label).collect(),
        })
        .collect()
}

fn to_wire(outcome: agent::contracts::ModifyOutcome) -> ModifyOutcome {
    use agent::contracts::ModifyOutcome as O;
    match outcome {
        O::Preview {
            plan_id,
            changed_files,
            follow_up,
        } => ModifyOutcome::Preview {
            plan_id,
            changed_files,
            questions: question_summaries(follow_up),
        },
        O::Applied {
            plan_id,
            changed_files,
            follow_up,
        } => ModifyOutcome::Applied {
            plan_id,
            changed_files,
            questions: question_summaries(follow_up),
        },
        O::NeedsClarification { questions } => ModifyOutcome::NeedsClarification {
            questions: question_summaries(questions),
        },
        O::NoChange { reason, follow_up } => ModifyOutcome::NoChange {
            reason,
            questions: question_summaries(follow_up),
        },
        O::Unsupported { rejection } => ModifyOutcome::Unsupported {
            explanation: rejection.explanation,
        },
        O::Conflict { reason } => ModifyOutcome::Conflict { reason },
    }
}

fn app() -> Router {
    Router::new()
        .route(CHAT_PATH, post(chat))
        .route(HEALTH_PATH, get(|| async { "ok" }))
        .route(PROJECTS_PATH, get(list_projects).post(create_project))
        .route(PROJECTS_MODIFY_PATH, post(modify_project))
        .route(PROJECTS_ANSWER_PATH, post(answer_project))
        .route(PROJECTS_ABORT_PATH, post(abort_project))
}

#[tokio::main]
async fn main() -> std::io::Result<()> {
    // `--addr HOST:PORT`; defaults to an ephemeral port on loopback.
    let mut args = std::env::args().skip(1);
    let mut addr = "127.0.0.1:0".to_string();
    let mut exit_on_stdin_close = false;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--addr" => addr = args.next().unwrap_or(addr),
            // Set by a parent process that holds our stdin pipe: when it dies, we die.
            "--exit-on-stdin-close" => exit_on_stdin_close = true,
            _ => {}
        }
    }
    if exit_on_stdin_close {
        std::thread::spawn(|| {
            let _ = std::io::copy(&mut std::io::stdin(), &mut std::io::sink());
            std::process::exit(0);
        });
    }

    let app = app();
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    // The GUI parses this line to learn the bound port.
    println!("listening on {}", listener.local_addr()?);
    axum::serve(listener, app).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{body::Body, http::Request};
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    async fn call(
        method: &str,
        uri: &str,
        body: serde_json::Value,
    ) -> (StatusCode, serde_json::Value) {
        let req = Request::builder()
            .method(method)
            .uri(uri)
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap();
        let resp = app().oneshot(req).await.unwrap();
        let status = resp.status();
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null),
        )
    }

    #[tokio::test]
    async fn create_then_modify_and_errors() {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path().join("projects").to_string_lossy().into_owned();

        let (st, body) = call(
            "POST",
            "/projects",
            serde_json::json!({"base_path": base, "name": "My App"}),
        )
        .await;
        assert_eq!(st, StatusCode::CREATED);
        assert_eq!(body["slug"], "my-app");
        let path = body["path"].as_str().unwrap().to_string();

        let (st, _) = call(
            "POST",
            "/projects",
            serde_json::json!({"base_path": base, "name": "my app"}),
        )
        .await;
        assert_eq!(st, StatusCode::CONFLICT);
        let (st, body) = call(
            "POST",
            "/projects",
            serde_json::json!({"base_path": base, "name": "!!!"}),
        )
        .await;
        assert_eq!(st, StatusCode::BAD_REQUEST);
        assert!(body["error"].is_string());

        let (st, body) = call(
            "POST",
            "/projects/modify",
            serde_json::json!({"project_path": path, "command": "Add top header"}),
        )
        .await;
        assert_eq!(st, StatusCode::OK);
        assert_eq!(body["reply"], "received: Add top header");
        assert_eq!(body["outcome"]["kind"], "unsupported");
        let (st, body) = call(
            "POST",
            "/projects/modify",
            serde_json::json!({"project_path": path, "command": "Add navigation to top nav"}),
        )
        .await;
        assert_eq!(st, StatusCode::OK);
        assert_eq!(body["outcome"]["kind"], "needs_clarification");
        assert_eq!(body["outcome"]["questions"][0]["blocking"], true);
        let (st, _) = call(
            "POST",
            "/projects/modify",
            serde_json::json!({"project_path": "/nonexistent/x", "command": "x"}),
        )
        .await;
        assert_eq!(st, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn navigation_flow_uses_conversation_and_request_ids() {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path().join("projects").to_string_lossy().into_owned();
        let (_, body) = call(
            "POST",
            "/projects",
            serde_json::json!({"base_path": base, "name": "nav"}),
        )
        .await;
        let path = body["path"].as_str().unwrap().to_string();
        let modify = |command: &str, request_id: &str, extra: serde_json::Value| {
            let mut body = serde_json::json!({
                "project_path": path, "command": command,
                "conversation_id": "chat-1", "request_id": request_id,
            });
            body.as_object_mut()
                .unwrap()
                .extend(extra.as_object().unwrap().clone());
            body
        };

        let (st, body) = call(
            "POST",
            "/projects/modify",
            modify(
                "Need a top navigation",
                "r1",
                serde_json::json!({"dry_run": true}),
            ),
        )
        .await;
        assert_eq!(st, StatusCode::OK);
        assert_eq!(body["outcome"]["kind"], "preview");
        assert!(!std::path::Path::new(&path).join("src/components").exists());

        let (_, body) = call(
            "POST",
            "/projects/modify",
            modify("Need a top navigation", "r1", serde_json::json!({})),
        )
        .await;
        assert_eq!(body["outcome"]["kind"], "applied");
        let (_, body) = call(
            "POST",
            "/projects/modify",
            modify("Add Contact Us", "r2", serde_json::json!({})),
        )
        .await;
        assert_eq!(body["outcome"]["kind"], "applied");
        assert_eq!(body["outcome"]["questions"][0]["blocking"], false);
        assert!(body["outcome"]["questions"][0]["prompt"]
            .as_str()
            .unwrap()
            .contains("Contact Us"));

        // A transport retry replays; a deliberate repeat (new id) adds nothing.
        let nav = || {
            std::fs::read_to_string(std::path::Path::new(&path).join("src/components/TopNav.tsx"))
                .unwrap()
        };
        let once = nav();
        let (_, body) = call(
            "POST",
            "/projects/modify",
            modify("Add Contact Us", "r2", serde_json::json!({})),
        )
        .await;
        assert_eq!(body["outcome"]["kind"], "applied");
        assert!(body["reply"].as_str().unwrap().contains("already applied"));
        assert_eq!(nav(), once);
        let (_, body) = call(
            "POST",
            "/projects/modify",
            modify("Need a top navigation", "r3", serde_json::json!({})),
        )
        .await;
        assert_eq!(body["outcome"]["kind"], "no_change");
        assert_eq!(nav(), once);

        let (st, _) = call(
            "POST",
            "/projects/modify",
            serde_json::json!({"project_path": path, "command": "x", "conversation_id": "../x"}),
        )
        .await;
        assert_eq!(st, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn destination_conversation_through_chat_replies_and_structured_answers() {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path().join("projects").to_string_lossy().into_owned();
        let (_, body) = call(
            "POST",
            "/projects",
            serde_json::json!({"base_path": base, "name": "dest"}),
        )
        .await;
        let path = body["path"].as_str().unwrap().to_string();
        let read = |rel: &str| {
            std::fs::read_to_string(std::path::Path::new(&path).join(rel)).unwrap_or_default()
        };
        let modify = |command: &str, request_id: &str| {
            serde_json::json!({
                "project_path": path, "command": command,
                "conversation_id": "chat-1", "request_id": request_id,
            })
        };
        call(
            "POST",
            "/projects/modify",
            modify("Need a top navigation", "r1"),
        )
        .await;
        let (_, body) = call("POST", "/projects/modify", modify("Add Contact Us", "r2")).await;
        let question = body["outcome"]["questions"][0].clone();
        assert_eq!(question["option_keys"][0], "new_page");
        assert_eq!(question["takes_text"], false);
        let id = question["id"].as_str().unwrap().to_string();

        // Both a bare chat reply and a structured answer are accepted; a
        // malformed structured answer is a 400 and an unknown question a conflict.
        let answer = |body: serde_json::Value| {
            let mut base = serde_json::json!({
                "project_path": path, "question_id": id, "conversation_id": "chat-1",
            });
            base.as_object_mut()
                .unwrap()
                .extend(body.as_object().unwrap().clone());
            base
        };
        let (st, _) = call("POST", PROJECTS_ANSWER_PATH, answer(serde_json::json!({}))).await;
        assert_eq!(st, StatusCode::BAD_REQUEST);
        let (st, _) = call(
            "POST",
            PROJECTS_ANSWER_PATH,
            answer(serde_json::json!({"option_key": "a", "text": "b"})),
        )
        .await;
        assert_eq!(st, StatusCode::BAD_REQUEST);
        let (_, body) = call(
            "POST",
            PROJECTS_ANSWER_PATH,
            answer(serde_json::json!({"option_key": "new_page", "dry_run": true})),
        )
        .await;
        assert_eq!(body["outcome"]["kind"], "preview");
        assert!(!read("src/router.ts").contains("/contact-us"));

        let (_, body) = call(
            "POST",
            PROJECTS_ANSWER_PATH,
            answer(serde_json::json!({"option_key": "new_page", "request_id": "a1"})),
        )
        .await;
        assert_eq!(body["outcome"]["kind"], "applied", "{body}");
        assert!(read("src/router.ts").contains("/contact-us"));
        assert!(read("src/components/TopNav.tsx").contains("href={\"/contact-us\"}"));
        let after = read("src/router.ts");
        let (_, retry) = call(
            "POST",
            PROJECTS_ANSWER_PATH,
            answer(serde_json::json!({"option_key": "new_page", "request_id": "a1"})),
        )
        .await;
        assert!(retry["reply"].as_str().unwrap().contains("already applied"));
        let (_, stale) = call(
            "POST",
            PROJECTS_ANSWER_PATH,
            answer(serde_json::json!({"option_key": "new_page", "request_id": "a2"})),
        )
        .await;
        assert_eq!(stale["outcome"]["kind"], "conflict");
        assert_eq!(read("src/router.ts"), after);

        // Chat reply path: a number answers the destination of the next item.
        call("POST", "/projects/modify", modify("Add Docs", "r3")).await;
        let (_, ext) = call("POST", "/projects/modify", modify("3", "r4")).await;
        assert_eq!(ext["outcome"]["kind"], "needs_clarification");
        assert_eq!(ext["outcome"]["questions"][0]["takes_text"], true);
        let (_, linked) = call(
            "POST",
            "/projects/modify",
            modify("https://example.com/docs", "r5"),
        )
        .await;
        assert_eq!(linked["outcome"]["kind"], "applied", "{linked}");
        assert!(read("src/components/TopNav.tsx").contains("https://example.com/docs"));
    }

    #[tokio::test]
    async fn chat_echoes() {
        let (st, body) = call("POST", "/chat", serde_json::json!({"prompt": "hi"})).await;
        assert_eq!(st, StatusCode::OK);
        assert_eq!(body["reply"], "hi");
    }

    #[tokio::test]
    async fn abort_endpoint_preserves_external_edit_and_reports_result() {
        use agent::apply::{apply_plan, ApplyOptions};
        use agent::contracts::PlanOutcome;

        let tmp = tempfile::tempdir().unwrap();
        let dir = agent::init_named(tmp.path(), "recovery").unwrap();
        let project = match agent::project::load_project(&dir).unwrap() {
            agent::project::ProjectState::Loaded(project) => project,
            _ => panic!("new project has no model"),
        };
        let session = agent::project::load_session(&dir, "default").unwrap();
        let plan = match agent::pipeline::plan_prompt("Add top nav", &project, &session) {
            PlanOutcome::Ready { plan } => plan,
            other => panic!("{other:?}"),
        };
        let options = ApplyOptions {
            fail_after_writes: Some(1),
            ..Default::default()
        };
        assert!(apply_plan(&dir, "default", &plan, &options).is_err());
        let app_path = dir.join("src/App.tsx");
        let external = format!(
            "// external edit\n{}",
            std::fs::read_to_string(&app_path).unwrap()
        );
        std::fs::write(&app_path, &external).unwrap();

        let body = serde_json::json!({ "project_path": dir });
        let (status, response) = call("POST", PROJECTS_ABORT_PATH, body.clone()).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(response["kind"], "aborted");
        assert_eq!(
            response["preserved_paths"],
            serde_json::json!(["src/App.tsx"])
        );
        assert_eq!(std::fs::read_to_string(&app_path).unwrap(), external);
        assert!(!dir.join("src/components/TopNav.tsx").exists());
        let (status, response) = call("POST", PROJECTS_ABORT_PATH, body).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(response["kind"], "nothing_to_do");
    }

    #[tokio::test]
    async fn abort_endpoint_reports_owned_region_conflict() {
        use agent::apply::{apply_plan, ApplyOptions};
        use agent::contracts::PlanOutcome;

        let tmp = tempfile::tempdir().unwrap();
        let dir = agent::init_named(tmp.path(), "conflict").unwrap();
        let project = match agent::project::load_project(&dir).unwrap() {
            agent::project::ProjectState::Loaded(project) => project,
            _ => panic!("new project has no model"),
        };
        let session = agent::project::load_session(&dir, "default").unwrap();
        let plan = match agent::pipeline::plan_prompt("Add top nav", &project, &session) {
            PlanOutcome::Ready { plan } => plan,
            other => panic!("{other:?}"),
        };
        let options = ApplyOptions {
            fail_after_writes: Some(3),
            ..Default::default()
        };
        assert!(apply_plan(&dir, "default", &plan, &options).is_err());
        let app_path = dir.join("src/App.tsx");
        let edited = std::fs::read_to_string(&app_path)
            .unwrap()
            .replace("<TopNav />", "<UserNav />");
        std::fs::write(&app_path, &edited).unwrap();

        let (status, response) = call(
            "POST",
            PROJECTS_ABORT_PATH,
            serde_json::json!({ "project_path": dir }),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(response["kind"], "conflict");
        assert!(response["reason"]
            .as_str()
            .unwrap()
            .contains("owned layout-top region"));
        assert_eq!(std::fs::read_to_string(&app_path).unwrap(), edited);
        assert!(dir.join(".protopie/journal.json").exists());
    }
}
