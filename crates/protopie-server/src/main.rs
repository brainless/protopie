use std::path::{Path, PathBuf};

use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use protopie_api::{
    AbortProjectRequest, AbortProjectResponse, AnswerProjectRequest, ChatRequest, ChatResponse,
    CreateProjectRequest, CreateProjectResponse, DiagnosticSpan, ErrorResponse, FileDiff,
    ListProjectsResponse, ModifyOutcome, ModifyProjectRequest, ModifyProjectResponse,
    ParserRejectionReason, PreviewLogRequest, PreviewLogResponse, PreviewProjectRequest,
    PreviewStatusResponse, ProjectInfo, QuestionSummary, RejectionReason, RuntimeCheckResponse,
    CHAT_PATH, HEALTH_PATH, PREVIEW_LAUNCH_PATH, PREVIEW_LOGS_PATH, PREVIEW_RESTART_PATH,
    PREVIEW_RUNTIME_PATH, PREVIEW_STATUS_PATH, PREVIEW_STOP_PATH, PROJECTS_ABORT_PATH,
    PROJECTS_ANSWER_PATH, PROJECTS_MODIFY_PATH, PROJECTS_PATH,
};
use protopie_ui_agent as agent;

mod preview;
mod runtime;

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

fn next_trace_id() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    format!(
        "{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )
}

fn pending_state(project: &Path, conversation: &str) -> serde_json::Value {
    match agent::project::load_session(project, conversation) {
        Ok(session) => serde_json::json!({
            "status": "available",
            "questions": session.pending_questions.iter().map(|q| serde_json::json!({
                "id": q.id, "prompt": protopie_api::diagnostics::bounded_text(&q.prompt),
                "option_keys": q.options.iter().map(|o| &o.key).collect::<Vec<_>>(),
            })).collect::<Vec<_>>(),
        }),
        Err(_) => serde_json::json!({"status": "unavailable"}),
    }
}

fn outcome_meta(outcome: &agent::contracts::ModifyOutcome) -> serde_json::Value {
    use agent::contracts::ModifyOutcome as O;
    match outcome {
        O::Preview {
            plan_id,
            base_revision,
            follow_up,
            ..
        } => serde_json::json!({
            "kind": "preview", "plan_id": plan_id, "base_revision": base_revision,
            "question_ids": follow_up.iter().map(|q| &q.id).collect::<Vec<_>>(),
        }),
        O::Applied {
            plan_id, follow_up, ..
        } => serde_json::json!({
            "kind": "applied", "plan_id": plan_id,
            "question_ids": follow_up.iter().map(|q| &q.id).collect::<Vec<_>>(),
        }),
        O::NeedsClarification {
            questions,
            persisted,
        } => serde_json::json!({
            "kind": "needs_clarification", "persisted": persisted,
            "question_ids": questions.iter().map(|q| &q.id).collect::<Vec<_>>(),
            "questions": questions.iter().map(|q| serde_json::json!({
                "id": q.id, "prompt": protopie_api::diagnostics::bounded_text(&q.prompt),
                "option_keys": q.options.iter().map(|o| &o.key).collect::<Vec<_>>(),
            })).collect::<Vec<_>>(),
        }),
        O::NoChange { follow_up, .. } => serde_json::json!({
            "kind": "no_change", "question_ids": follow_up.iter().map(|q| &q.id).collect::<Vec<_>>(),
        }),
        O::Unsupported { rejection } => serde_json::json!({
            "kind": "unsupported", "reason": rejection.reason, "span": rejection.span,
        }),
        O::Conflict { .. } => serde_json::json!({"kind": "conflict"}),
    }
}

fn log_result(
    project: &Path,
    conversation: &str,
    request_id: Option<&str>,
    trace_id: &str,
    kind: &str,
    result: &agent::Result<agent::contracts::ModifyResult>,
) {
    let outcome = match result {
        Ok(result) => serde_json::json!({
            "outcome": outcome_meta(&result.outcome),
            "summary": protopie_api::diagnostics::bounded_text(&result.summary),
        }),
        Err(error) => {
            serde_json::json!({"error": protopie_api::diagnostics::bounded_text(&error.to_string())})
        }
    };
    let _ = protopie_api::diagnostics::log_project_event(
        project,
        "server",
        serde_json::json!({
            "kind": kind, "trace_id": trace_id, "conversation_id": conversation,
            "request_id": request_id, "result": outcome, "pending_after": pending_state(project, conversation),
        }),
    );
}

fn project_info(root: &Path, name: String) -> ProjectInfo {
    let path = root.join(&name);
    let model_path = path.join(".protopie/model.json");
    let last_modified_unix_ms = std::fs::metadata(model_path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|modified| modified.duration_since(std::time::UNIX_EPOCH).ok())
        .and_then(|elapsed| u64::try_from(elapsed.as_millis()).ok());
    let unavailable_reason = preview::validate_project(&path)
        .err()
        .map(|error| error.to_string());
    ProjectInfo {
        path: path_string(&path),
        name,
        last_modified_unix_ms,
        unavailable_reason,
    }
}

async fn chat(Json(req): Json<ChatRequest>) -> Json<ChatResponse> {
    Json(ChatResponse { reply: req.prompt })
}

async fn preview_runtime() -> Result<Json<RuntimeCheckResponse>, ApiError> {
    tokio::task::spawn_blocking(runtime::check_runtime)
        .await
        .map(Json)
        .map_err(|e| {
            ApiError(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("runtime check failed: {e}"),
            )
        })
}

async fn preview_launch(
    State(manager): State<preview::PreviewManager>,
    Json(req): Json<PreviewProjectRequest>,
) -> Json<PreviewStatusResponse> {
    Json(manager.launch(&req.project_path, false).await)
}

async fn preview_restart(
    State(manager): State<preview::PreviewManager>,
    Json(req): Json<PreviewProjectRequest>,
) -> Json<PreviewStatusResponse> {
    Json(manager.launch(&req.project_path, true).await)
}

async fn preview_stop(
    State(manager): State<preview::PreviewManager>,
) -> Json<PreviewStatusResponse> {
    Json(manager.stop().await)
}

async fn preview_status(
    State(manager): State<preview::PreviewManager>,
) -> Json<PreviewStatusResponse> {
    Json(manager.status().await)
}

async fn preview_logs(
    State(manager): State<preview::PreviewManager>,
    Json(req): Json<PreviewLogRequest>,
) -> Json<PreviewLogResponse> {
    Json(manager.logs(req).await)
}

async fn list_projects() -> Result<Json<ListProjectsResponse>, ApiError> {
    let projects = blocking(|| {
        let root = Path::new(PROJECTS_BASE_PATH);
        // Absolute so clients can send the path back regardless of cwd.
        let abs = std::path::absolute(root).unwrap_or_else(|_| root.to_path_buf());
        Ok(agent::list_projects(root)?
            .into_iter()
            .map(|name| project_info(&abs, name))
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
        let project = Path::new(&req.project_path);
        let trace_id = next_trace_id();
        let conversation = req.conversation_id.as_deref().unwrap_or(agent::contracts::DEFAULT_CONVERSATION_ID);
        let before = pending_state(project, conversation);
        let _ = protopie_api::diagnostics::log_project_event(project, "server", serde_json::json!({
            "kind": "modify_request", "trace_id": trace_id, "conversation_id": conversation, "request_id": req.request_id,
            "command": protopie_api::diagnostics::bounded_text(&req.command), "dry_run": req.dry_run,
            "expected_revision": req.expected_revision, "expected_plan_id": req.expected_plan_id,
            "pending_before": before,
        }));
        let result = agent::modify_with(
            project,
            &req.command,
            &agent::ModifyOptions {
                conversation_id: req.conversation_id.as_deref(),
                request_id: req.request_id.as_deref(),
                dry_run: req.dry_run,
                expected_revision: req.expected_revision,
                expected_plan_id: req.expected_plan_id.as_deref(),
            },
        );
        log_result(project, conversation, req.request_id.as_deref(), &trace_id, "modify_result", &result);
        result
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
    let choice = match (req.option_key.clone(), req.text.clone()) {
        (Some(key), None) => agent::contracts::AnswerChoice::Option { key },
        (None, Some(text)) => agent::contracts::AnswerChoice::Text { text },
        _ => {
            let project = Path::new(&req.project_path);
            let conversation = req
                .conversation_id
                .as_deref()
                .unwrap_or(agent::contracts::DEFAULT_CONVERSATION_ID);
            let _ = protopie_api::diagnostics::log_project_event(
                project,
                "server",
                serde_json::json!({
                    "kind": "answer_validation_error", "conversation_id": conversation,
                    "request_id": req.request_id, "question_id": req.question_id,
                    "option_key": req.option_key.as_deref().map(protopie_api::diagnostics::bounded_text),
                    "text": req.text.as_deref().map(protopie_api::diagnostics::bounded_text),
                    "dry_run": req.dry_run, "expected_revision": req.expected_revision,
                    "expected_plan_id": req.expected_plan_id,
                    "pending_before": pending_state(project, conversation),
                    "error": "send exactly one of option_key or text",
                }),
            );
            return Err(ApiError(
                StatusCode::BAD_REQUEST,
                "send exactly one of option_key or text".into(),
            ));
        }
    };
    let answer = agent::contracts::Answer {
        question_id: req.question_id,
        choice,
    };
    let result = blocking(move || {
        let project = Path::new(&req.project_path);
        let trace_id = next_trace_id();
        let conversation = req.conversation_id.as_deref().unwrap_or(agent::contracts::DEFAULT_CONVERSATION_ID);
        let before = pending_state(project, conversation);
        let (option_key, text) = match &answer.choice {
            agent::contracts::AnswerChoice::Option { key } => (Some(key.as_str()), None),
            agent::contracts::AnswerChoice::Text { text } => (None, Some(text.as_str())),
        };
        let _ = protopie_api::diagnostics::log_project_event(project, "server", serde_json::json!({
            "kind": "answer_request", "trace_id": trace_id, "conversation_id": conversation, "request_id": req.request_id,
            "question_id": answer.question_id, "option_key": option_key.map(protopie_api::diagnostics::bounded_text),
            "text": text.map(protopie_api::diagnostics::bounded_text), "dry_run": req.dry_run,
            "expected_revision": req.expected_revision, "expected_plan_id": req.expected_plan_id,
            "pending_before": before,
        }));
        let result = agent::answer_with(
            project,
            &answer,
            &agent::ModifyOptions {
                conversation_id: req.conversation_id.as_deref(),
                request_id: req.request_id.as_deref(),
                dry_run: req.dry_run,
                expected_revision: req.expected_revision,
                expected_plan_id: req.expected_plan_id.as_deref(),
            },
        );
        log_result(project, conversation, req.request_id.as_deref(), &trace_id, "answer_result", &result);
        result
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
            base_revision,
            changed_files,
            diffs,
            follow_up,
        } => ModifyOutcome::Preview {
            plan_id,
            base_revision,
            changed_files,
            diffs: diffs
                .into_iter()
                .map(|d| FileDiff {
                    path: d.path,
                    diff: d.diff,
                })
                .collect(),
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
        O::NeedsClarification {
            questions,
            persisted,
        } => ModifyOutcome::NeedsClarification {
            questions: question_summaries(questions),
            persisted,
        },
        O::NoChange { reason, follow_up } => ModifyOutcome::NoChange {
            reason,
            questions: question_summaries(follow_up),
        },
        O::Unsupported { rejection } => {
            use agent::contracts::RejectionReason as R;
            use agent::parser::UnsupportedReason as P;
            let reason = match rejection.reason {
                R::Parse { reason } => RejectionReason::Parse {
                    reason: match reason {
                        P::EmptyInput => ParserRejectionReason::EmptyInput,
                        P::UnrecognizedInput => ParserRejectionReason::UnrecognizedInput,
                        P::IncompleteInput => ParserRejectionReason::IncompleteInput,
                        P::NegatedRequest => ParserRejectionReason::NegatedRequest,
                        P::UnsupportedTail => ParserRejectionReason::UnsupportedTail,
                    },
                },
                R::CapabilityNotImplemented => RejectionReason::CapabilityNotImplemented,
                R::TargetNotFound => RejectionReason::TargetNotFound,
                R::UnsupportedLayout => RejectionReason::UnsupportedLayout,
                R::ProjectNotInitialized => RejectionReason::ProjectNotInitialized,
            };
            ModifyOutcome::Unsupported {
                explanation: rejection.explanation,
                reason: Some(reason),
                span: rejection.span.map(|s| DiagnosticSpan {
                    start: s.start,
                    end: s.end,
                }),
            }
        }
        O::Conflict { reason } => ModifyOutcome::Conflict { reason },
    }
}

#[cfg(test)]
fn app() -> Router {
    app_with_preview(preview::PreviewManager::new())
}

fn app_with_preview(manager: preview::PreviewManager) -> Router {
    Router::new()
        .route(CHAT_PATH, post(chat))
        .route(HEALTH_PATH, get(|| async { "ok" }))
        .route(PREVIEW_RUNTIME_PATH, get(preview_runtime))
        .route(PREVIEW_LAUNCH_PATH, post(preview_launch))
        .route(PREVIEW_RESTART_PATH, post(preview_restart))
        .route(PREVIEW_STOP_PATH, post(preview_stop))
        .route(PREVIEW_STATUS_PATH, get(preview_status))
        .route(PREVIEW_LOGS_PATH, post(preview_logs))
        .route(PROJECTS_PATH, get(list_projects).post(create_project))
        .route(PROJECTS_MODIFY_PATH, post(modify_project))
        .route(PROJECTS_ANSWER_PATH, post(answer_project))
        .route(PROJECTS_ABORT_PATH, post(abort_project))
        .with_state(manager)
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
    let (stdin_closed_tx, stdin_closed_rx) = tokio::sync::oneshot::channel::<()>();
    if exit_on_stdin_close {
        std::thread::spawn(move || {
            let _ = std::io::copy(&mut std::io::stdin(), &mut std::io::sink());
            let _ = stdin_closed_tx.send(());
        });
    }

    let manager = preview::PreviewManager::new();
    let app = app_with_preview(manager.clone());
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    // The GUI parses this line to learn the bound port.
    println!("listening on {}", listener.local_addr()?);
    let shutdown = async move {
        if exit_on_stdin_close {
            tokio::select! { _ = tokio::signal::ctrl_c() => {}, _ = stdin_closed_rx => {} }
        } else {
            let _ = tokio::signal::ctrl_c().await;
        }
    };
    let result = axum::serve(listener, app)
        .with_graceful_shutdown(shutdown)
        .await;
    manager.stop().await;
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{body::Body, http::Request};
    use http_body_util::BodyExt;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tower::ServiceExt;

    async fn gui_preview_apply(
        endpoint: &str,
        mut request: serde_json::Value,
    ) -> (serde_json::Value, serde_json::Value) {
        let base_id = request["request_id"].as_str().unwrap().to_string();
        request["request_id"] = serde_json::json!(format!("{base_id}-preview"));
        request["dry_run"] = serde_json::json!(true);
        let (status, preview) = call("POST", endpoint, request.clone()).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(preview["outcome"]["kind"], "preview", "{preview}");
        request["request_id"] = serde_json::json!(format!("{base_id}-apply"));
        request["dry_run"] = serde_json::json!(false);
        request["expected_revision"] = preview["outcome"]["base_revision"].clone();
        request["expected_plan_id"] = preview["outcome"]["plan_id"].clone();
        let (status, applied) = call("POST", endpoint, request).await;
        assert_eq!(status, StatusCode::OK);
        if preview["outcome"]["changed_files"]
            .as_array()
            .unwrap()
            .is_empty()
        {
            assert_eq!(
                applied["outcome"]["kind"], "needs_clarification",
                "{applied}"
            );
            assert_eq!(applied["outcome"]["persisted"], true);
        } else {
            assert_eq!(applied["outcome"]["kind"], "applied", "{applied}");
        }
        (preview, applied)
    }

    #[tokio::test]
    async fn gui_context_questions_preview_then_persist_every_step() {
        let temp = tempfile::tempdir().unwrap();
        let project = agent::init_named(temp.path(), "context-gui").unwrap();
        agent::modify(&project, "Add a Doctors page").unwrap();
        agent::modify(&project, "Add a Visits page").unwrap();
        let path = project.to_string_lossy().into_owned();
        let pending = || {
            agent::project::load_session(&project, "gui")
                .unwrap()
                .pending_questions
        };
        let modify = |command: &str, id: &str| {
            serde_json::json!({
                "project_path": path, "command": command, "conversation_id": "gui", "request_id": id,
            })
        };
        let answer = |question_id: &str, option_key: Option<&str>, text: Option<&str>, id: &str| {
            serde_json::json!({
                "project_path": path, "question_id": question_id, "option_key": option_key,
                "text": text, "conversation_id": "gui", "request_id": id,
            })
        };

        let mut opening = modify("Share the selected doctor across pages", "share");
        opening["dry_run"] = serde_json::json!(true);
        let (_, first_preview) = call("POST", PROJECTS_MODIFY_PATH, opening.clone()).await;
        assert_eq!(
            first_preview["outcome"]["kind"], "preview",
            "{first_preview}"
        );
        assert!(first_preview["outcome"]["changed_files"]
            .as_array()
            .unwrap()
            .is_empty());
        assert!(
            pending().is_empty(),
            "dry run must leave questions unpersisted"
        );
        let revision = first_preview["outcome"]["base_revision"].clone();
        opening["dry_run"] = serde_json::json!(false);
        opening["request_id"] = serde_json::json!("share-wrong-revision");
        opening["expected_revision"] = serde_json::json!(revision.as_u64().unwrap() + 1);
        opening["expected_plan_id"] = first_preview["outcome"]["plan_id"].clone();
        let (_, wrong_revision) = call("POST", PROJECTS_MODIFY_PATH, opening.clone()).await;
        assert_eq!(wrong_revision["outcome"]["kind"], "conflict");
        assert!(pending().is_empty());
        opening["request_id"] = serde_json::json!("share-wrong-plan");
        opening["expected_revision"] = revision.clone();
        opening["expected_plan_id"] = serde_json::json!("wrong-plan");
        let (_, wrong) = call("POST", PROJECTS_MODIFY_PATH, opening.clone()).await;
        assert_eq!(wrong["outcome"]["kind"], "conflict");
        assert!(pending().is_empty());
        opening["request_id"] = serde_json::json!("share-apply");
        opening["expected_plan_id"] = first_preview["outcome"]["plan_id"].clone();
        let (_, first_applied) = call("POST", PROJECTS_MODIFY_PATH, opening).await;
        assert_eq!(
            first_applied["outcome"]["kind"], "needs_clarification",
            "{first_applied}"
        );
        assert_eq!(first_applied["outcome"]["persisted"], true);
        let shape = first_applied["outcome"]["questions"][0]["id"]
            .as_str()
            .unwrap()
            .to_string();
        assert_eq!(pending()[0].id, shape);

        let (shape_preview, shape_applied) = gui_preview_apply(
            PROJECTS_ANSWER_PATH,
            answer(&shape, Some("text"), None, "shape"),
        )
        .await;
        assert!(shape_preview["outcome"]["changed_files"]
            .as_array()
            .unwrap()
            .is_empty());
        let initial = shape_applied["outcome"]["questions"][0]["id"]
            .as_str()
            .unwrap()
            .to_string();
        assert_eq!(pending()[0].id, initial);

        // Replaying the opening request after answering shape must not offer
        // the old shape button, even though the request itself was applied.
        let (_, replay) = call("POST", PROJECTS_MODIFY_PATH, serde_json::json!({
            "project_path": path, "command": "Share the selected doctor across pages",
            "conversation_id": "gui", "request_id": "share-apply", "dry_run": false,
        })).await;
        assert_eq!(replay["outcome"]["kind"], "needs_clarification");
        assert_eq!(replay["outcome"]["persisted"], false);
        assert_eq!(pending()[0].id, initial);

        // The GUI sends free text through /projects/modify, so cover that path.
        let (value_preview, value_applied) =
            gui_preview_apply(PROJECTS_MODIFY_PATH, modify("Dr. Rao", "initial-value")).await;
        assert!(value_preview["outcome"]["changed_files"]
            .as_array()
            .unwrap()
            .is_empty());
        let consumer = value_applied["outcome"]["questions"][0]["id"]
            .as_str()
            .unwrap()
            .to_string();
        assert_eq!(pending()[0].id, consumer);

        let (consumer_preview, consumer_applied) = gui_preview_apply(
            PROJECTS_ANSWER_PATH,
            answer(&consumer, Some("doctors"), None, "consumer"),
        )
        .await;
        assert!(consumer_preview["outcome"]["changed_files"]
            .as_array()
            .unwrap()
            .is_empty());
        let done = consumer_applied["outcome"]["questions"][0]["id"]
            .as_str()
            .unwrap()
            .to_string();
        assert_eq!(pending()[0].id, done);
        assert!(!project.join("src/context/SelectedDoctor.tsx").exists());

        let (done_preview, done_applied) = gui_preview_apply(
            PROJECTS_ANSWER_PATH,
            answer(&done, Some("done"), None, "done"),
        )
        .await;
        assert!(!done_preview["outcome"]["changed_files"]
            .as_array()
            .unwrap()
            .is_empty());
        assert_eq!(done_applied["outcome"]["kind"], "applied");
        assert!(pending().is_empty());
        assert!(project.join("src/context/SelectedDoctor.tsx").exists());
    }

    #[tokio::test]
    async fn project_diagnostics_record_requests_outcomes_and_pending_state() {
        let temp = tempfile::tempdir().unwrap();
        let project = agent::init_named(temp.path(), "diagnostic-example").unwrap();
        agent::modify(&project, "Add a Doctors page").unwrap();
        let path = project.to_string_lossy().into_owned();
        let share = modify_project(Json(ModifyProjectRequest {
            project_path: path.clone(),
            command: "Share the selected doctor across pages".into(),
            conversation_id: Some("gui".into()),
            request_id: Some("diag-share".into()),
            dry_run: false,
            expected_revision: None,
            expected_plan_id: None,
        }))
        .await
        .ok()
        .unwrap()
        .0;
        let Some(ModifyOutcome::NeedsClarification { questions, .. }) = share.outcome else {
            panic!("{share:?}")
        };
        let question_id = questions[0].id.clone();
        let _answer = answer_project(Json(AnswerProjectRequest {
            project_path: path,
            question_id: question_id.clone(),
            option_key: Some("text".into()),
            text: None,
            conversation_id: Some("gui".into()),
            request_id: Some("diag-text".into()),
            dry_run: true,
            expected_revision: None,
            expected_plan_id: None,
        }))
        .await
        .ok()
        .unwrap()
        .0;
        let dir = protopie_api::diagnostics::project_log_dir(&project).unwrap();
        let lines = std::fs::read_to_string(dir.join("events.jsonl")).unwrap();
        let entries: Vec<serde_json::Value> = lines
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        let event = |kind: &str, request_id: &str| {
            entries
                .iter()
                .find(|entry| {
                    entry["event"]["kind"] == kind && entry["event"]["request_id"] == request_id
                })
                .unwrap()
        };
        assert_eq!(
            event("modify_request", "diag-share")["event"]["command"],
            "Share the selected doctor across pages"
        );
        assert_eq!(
            event("modify_result", "diag-share")["event"]["result"]["outcome"]["kind"],
            "needs_clarification"
        );
        assert_eq!(
            event("answer_request", "diag-text")["event"]["question_id"],
            question_id
        );
        assert_eq!(
            event("answer_request", "diag-text")["event"]["option_key"],
            "text"
        );
        assert_eq!(
            event("answer_request", "diag-text")["event"]["pending_before"]["questions"][0]["id"],
            question_id
        );
        assert!(
            event("answer_result", "diag-text")["event"]["pending_after"]["questions"].is_array()
        );
        assert!(!lines.contains("diff"));
    }

    #[tokio::test]
    async fn gui_style_link_aliases_preview_apply_and_keep_destination_question() {
        for command in [
            "Add link \"Features\" to top nav",
            "Add link \"Features\" to top navigation",
        ] {
            let temp = tempfile::tempdir().unwrap();
            let project = agent::init_named(temp.path(), "alias").unwrap();
            let path = project.to_string_lossy().into_owned();
            let modify = |command: &str,
                          request_id: &str,
                          dry_run: bool,
                          preview: Option<&serde_json::Value>| {
                let mut request = serde_json::json!({
                    "project_path": path, "command": command, "conversation_id": "gui",
                    "request_id": request_id, "dry_run": dry_run,
                });
                if let Some(preview) = preview {
                    request["expected_revision"] = preview["outcome"]["base_revision"].clone();
                    request["expected_plan_id"] = preview["outcome"]["plan_id"].clone();
                }
                request
            };
            let (status, nav_preview) = call(
                "POST",
                PROJECTS_MODIFY_PATH,
                modify("Add top navigation", "nav-preview", true, None),
            )
            .await;
            assert_eq!(status, StatusCode::OK);
            assert_eq!(nav_preview["outcome"]["kind"], "preview");
            let (_, nav_applied) = call(
                "POST",
                PROJECTS_MODIFY_PATH,
                modify("Add top navigation", "nav-apply", false, Some(&nav_preview)),
            )
            .await;
            assert_eq!(nav_applied["outcome"]["kind"], "applied", "{nav_applied}");
            let (_, preview) = call(
                "POST",
                PROJECTS_MODIFY_PATH,
                modify(command, "link-preview", true, None),
            )
            .await;
            assert_eq!(
                preview["outcome"]["kind"], "preview",
                "{command}: {preview}"
            );
            let (_, applied) = call(
                "POST",
                PROJECTS_MODIFY_PATH,
                modify(command, "link-apply", false, Some(&preview)),
            )
            .await;
            assert_eq!(
                applied["outcome"]["kind"], "applied",
                "{command}: {applied}"
            );
            let question = &applied["outcome"]["questions"][0];
            assert!(question["prompt"].as_str().unwrap().contains("Features"));
            assert!(question["option_keys"]
                .as_array()
                .unwrap()
                .iter()
                .any(|key| key == "new_page"));
            let answer = |dry_run: bool, preview: Option<&serde_json::Value>| {
                let mut request = serde_json::json!({
                    "project_path": path, "question_id": question["id"], "option_key": "new_page",
                    "conversation_id": "gui", "request_id": if dry_run { "answer-preview" } else { "answer-apply" },
                    "dry_run": dry_run,
                });
                if let Some(preview) = preview {
                    request["expected_revision"] = preview["outcome"]["base_revision"].clone();
                    request["expected_plan_id"] = preview["outcome"]["plan_id"].clone();
                }
                request
            };
            let (_, answer_preview) = call("POST", PROJECTS_ANSWER_PATH, answer(true, None)).await;
            assert_eq!(
                answer_preview["outcome"]["kind"], "preview",
                "{answer_preview}"
            );
            let (_, answer_applied) = call(
                "POST",
                PROJECTS_ANSWER_PATH,
                answer(false, Some(&answer_preview)),
            )
            .await;
            assert_eq!(
                answer_applied["outcome"]["kind"], "applied",
                "{answer_applied}"
            );
            assert!(std::fs::read_to_string(project.join("src/router.ts"))
                .unwrap()
                .contains("/features"));
        }
    }

    #[test]
    fn maintained_examples_parse_and_match_their_stated_project_states() {
        use agent::contracts::ModifyOutcome as Outcome;
        use agent::parser::ParseOutcome;
        for example in protopie_api::CHAT_EXAMPLES {
            assert!(
                matches!(
                    agent::parser::parse(example.prompt),
                    ParseOutcome::Parsed(_)
                ),
                "{}",
                example.prompt
            );
            let temp = tempfile::tempdir().unwrap();
            let project = agent::init_named(temp.path(), "example").unwrap();
            if example.prompt.starts_with("Give the form") {
                let setup = agent::modify(&project, "Add a form below hero").unwrap();
                let mut outcome = setup.outcome;
                for text in ["Name, Email", "Send"] {
                    let Outcome::NeedsClarification { questions, .. } = outcome else {
                        panic!("{outcome:?}")
                    };
                    let answer = agent::contracts::Answer {
                        question_id: questions[0].id.clone(),
                        choice: agent::contracts::AnswerChoice::Text { text: text.into() },
                    };
                    outcome =
                        agent::answer_with(&project, &answer, &agent::ModifyOptions::default())
                            .unwrap()
                            .outcome;
                }
                assert!(matches!(outcome, Outcome::Applied { .. }), "{outcome:?}");
            }
            if example.prompt.starts_with("Share the") {
                let setup = agent::modify(&project, "Add a Doctors page").unwrap();
                assert!(
                    matches!(setup.outcome, Outcome::Applied { .. }),
                    "{setup:?}"
                );
            }
            let result = agent::modify_with(
                &project,
                example.prompt,
                &agent::ModifyOptions {
                    dry_run: true,
                    ..Default::default()
                },
            )
            .unwrap();
            assert!(
                matches!(
                    result.outcome,
                    Outcome::Preview { .. } | Outcome::NeedsClarification { .. }
                ),
                "{}: {result:?}",
                example.prompt
            );
        }
    }

    #[test]
    fn unsupported_mapping_keeps_non_parser_reason_without_inventing_a_span() {
        let wire = to_wire(agent::contracts::ModifyOutcome::Unsupported {
            rejection: agent::contracts::Rejection {
                reason: agent::contracts::RejectionReason::TargetNotFound,
                explanation: "There is no form to move.".into(),
                span: None,
            },
        });
        assert!(matches!(
            wire,
            ModifyOutcome::Unsupported {
                reason: Some(RejectionReason::TargetNotFound),
                span: None,
                ..
            }
        ));
    }

    #[test]
    fn project_listing_reports_timestamp_and_unsupported_reason() {
        let temp = tempfile::tempdir().unwrap();
        let good = agent::init_named(temp.path(), "good").unwrap();
        let info = project_info(temp.path(), "good".into());
        assert_eq!(std::fs::canonicalize(&info.path).unwrap(), good);
        assert!(info.last_modified_unix_ms.is_some());
        assert!(info.unavailable_reason.is_none());

        let old = temp.path().join("old");
        std::fs::create_dir(&old).unwrap();
        let info = project_info(temp.path(), "old".into());
        assert!(info.last_modified_unix_ms.is_none());
        assert!(info
            .unavailable_reason
            .as_deref()
            .unwrap()
            .contains("Create a project from the reference template"));

        std::fs::write(good.join("src/router.ts"), "no template regions").unwrap();
        let info = project_info(temp.path(), "good".into());
        assert!(info.unavailable_reason.is_some());
    }

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

    async fn call_over_http(
        port: u16,
        method: &str,
        path: &str,
        body: serde_json::Value,
    ) -> (StatusCode, serde_json::Value) {
        let body = body.to_string();
        let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .unwrap();
        stream
            .write_all(
                format!(
                    "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
                .as_bytes(),
            )
            .await
            .unwrap();
        let mut response = Vec::new();
        tokio::time::timeout(
            std::time::Duration::from_secs(15),
            stream.read_to_end(&mut response),
        )
        .await
        .expect("server HTTP response timed out")
        .unwrap();
        let response = String::from_utf8(response).unwrap();
        let (headers, body) = response.split_once("\r\n\r\n").unwrap();
        let status =
            StatusCode::from_u16(headers.split_whitespace().nth(1).unwrap().parse().unwrap())
                .unwrap();
        (status, serde_json::from_str(body).unwrap())
    }

    /// Fetch a Vite response over a real loopback connection. The project
    /// routes use SPA fallback, so only transformed module URLs prove that
    /// Vite can compile newly generated source.
    async fn vite_get(port: u16, path: &str) -> String {
        let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .unwrap();
        stream
            .write_all(
                format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n")
                    .as_bytes(),
            )
            .await
            .unwrap();
        let mut response = Vec::new();
        tokio::time::timeout(
            std::time::Duration::from_secs(15),
            stream.read_to_end(&mut response),
        )
        .await
        .expect("Vite response timed out")
        .unwrap();
        String::from_utf8(response).unwrap()
    }

    #[cfg(unix)]
    #[tokio::test]
    #[ignore = "requires Node/npm, first npm ci, and loopback sockets"]
    async fn real_http_conversation_serves_generated_modules_and_reaps() {
        use protopie_api::PreviewState;
        use std::time::{Duration, Instant};

        let temp = tempfile::tempdir().unwrap();
        let base = temp.path().to_string_lossy().into_owned();
        let runtime = runtime::check_runtime();
        assert!(
            matches!(runtime.status, protopie_api::RuntimeStatus::Available),
            "Node/npm prerequisites: {runtime:?}"
        );
        let manager = preview::PreviewManager::new();
        let app = app_with_preview(manager.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let server_port = listener.local_addr().unwrap().port();
        let server = tokio::spawn(async move { axum::serve(listener, app).await });
        // A task panic is reported only after the manager has stopped and
        // reaped its child. Keep the temp project alive until that cleanup.
        let task_manager = manager.clone();
        let task = tokio::spawn(async move {
            let request = |method, path, body| call_over_http(server_port, method, path, body);
            let (status, created) = request(
                "POST",
                PROJECTS_PATH,
                serde_json::json!({"base_path": base, "name": "live integration"}),
            )
            .await;
            assert_eq!(status, StatusCode::CREATED);
            let project_path = created["path"].as_str().unwrap();
            let (status, launch) = request(
                "POST",
                PREVIEW_LAUNCH_PATH,
                serde_json::json!({"project_path": project_path}),
            )
            .await;
            assert_eq!(status, StatusCode::OK);
            assert_eq!(launch["state"]["kind"], "preparing", "{launch}");

            let deadline = Instant::now() + Duration::from_secs(330);
            let url = loop {
                let (status, body) =
                    request("GET", PREVIEW_STATUS_PATH, serde_json::Value::Null).await;
                assert_eq!(status, StatusCode::OK);
                let state: PreviewStatusResponse = serde_json::from_value(body).unwrap();
                match state.state {
                    PreviewState::Running { url } => break url,
                    PreviewState::Failed { reason } => panic!("preview failed: {reason:?}"),
                    _ => assert!(Instant::now() < deadline, "preview did not become ready"),
                }
                tokio::time::sleep(Duration::from_millis(200)).await;
            };
            let port: u16 = url
                .trim_end_matches('/')
                .rsplit(':')
                .next()
                .unwrap()
                .parse()
                .unwrap();

            let conversation_id = "live-integration";
            let preview_command = |command: &str| {
                serde_json::json!({
                    "project_path": project_path, "conversation_id": conversation_id,
                    "command": command, "dry_run": true,
                })
            };
            let (status, nav_preview) = request(
                "POST",
                PROJECTS_MODIFY_PATH,
                preview_command("Need a top navigation"),
            )
            .await;
            assert_eq!(status, StatusCode::OK);
            assert_eq!(nav_preview["outcome"]["kind"], "preview", "{nav_preview}");
            let (status, nav_applied) = request(
                "POST",
                PROJECTS_MODIFY_PATH,
                serde_json::json!({
                    "project_path": project_path, "conversation_id": conversation_id,
                    "command": "Need a top navigation",
                    "expected_revision": nav_preview["outcome"]["base_revision"],
                    "expected_plan_id": nav_preview["outcome"]["plan_id"],
                }),
            )
            .await;
            assert_eq!(status, StatusCode::OK);
            assert_eq!(nav_applied["outcome"]["kind"], "applied", "{nav_applied}");

            let (status, item_preview) = request(
                "POST",
                PROJECTS_MODIFY_PATH,
                preview_command("Add Contact Us"),
            )
            .await;
            assert_eq!(status, StatusCode::OK);
            assert_eq!(item_preview["outcome"]["kind"], "preview", "{item_preview}");
            let (status, question) = request(
                "POST",
                PROJECTS_MODIFY_PATH,
                serde_json::json!({
                    "project_path": project_path, "conversation_id": conversation_id,
                    "command": "Add Contact Us",
                    "expected_revision": item_preview["outcome"]["base_revision"],
                    "expected_plan_id": item_preview["outcome"]["plan_id"],
                }),
            )
            .await;
            assert_eq!(status, StatusCode::OK);
            assert_eq!(question["outcome"]["kind"], "applied", "{question}");
            let question_id = question["outcome"]["questions"][0]["id"].as_str().unwrap();
            assert!(question["outcome"]["questions"][0]["option_keys"]
                .as_array()
                .unwrap()
                .iter()
                .any(|key| key == "new_page"));
            let answer = |dry_run: bool| {
                serde_json::json!({
                    "project_path": project_path, "conversation_id": conversation_id,
                    "question_id": question_id, "option_key": "new_page", "dry_run": dry_run,
                })
            };
            let (status, page_preview) = request("POST", PROJECTS_ANSWER_PATH, answer(true)).await;
            assert_eq!(status, StatusCode::OK);
            assert_eq!(page_preview["outcome"]["kind"], "preview", "{page_preview}");
            let mut apply_answer = answer(false);
            apply_answer["expected_revision"] = page_preview["outcome"]["base_revision"].clone();
            apply_answer["expected_plan_id"] = page_preview["outcome"]["plan_id"].clone();
            let (status, page_applied) = request("POST", PROJECTS_ANSWER_PATH, apply_answer).await;
            assert_eq!(status, StatusCode::OK);
            assert_eq!(page_applied["outcome"]["kind"], "applied", "{page_applied}");

            for path in ["/", "/contact-us"] {
                let response = vite_get(port, path).await;
                assert!(response.starts_with("HTTP/1.1 200"), "{path}: {response}");
                assert!(response.contains("/src/index.tsx"), "{path}: {response}");
            }
            for (path, expected) in [
                ("/src/index.tsx", "render"),
                ("/src/router.ts", "/contact-us"),
                ("/src/components/TopNav.tsx", "Contact Us"),
                ("/src/pages/ContactUsPage.tsx", "ContactUsPage"),
            ] {
                let response = vite_get(port, path).await;
                assert!(response.starts_with("HTTP/1.1 200"), "{path}: {response}");
                let headers = response.split("\r\n\r\n").next().unwrap();
                assert!(
                    headers.to_ascii_lowercase().contains("content-type:")
                        && headers.to_ascii_lowercase().contains("javascript"),
                    "{path}: {response}"
                );
                assert!(response.contains(expected), "{path}: {response}");
                assert!(
                    !response.contains("vite-error-overlay"),
                    "{path}: {response}"
                );
            }
            let pid = task_manager.child_pid().await.expect("Vite child PID");
            let (status, stopped) =
                request("POST", PREVIEW_STOP_PATH, serde_json::Value::Null).await;
            assert_eq!(status, StatusCode::OK);
            assert_eq!(stopped["state"]["kind"], "stopped", "{stopped}");
            pid
        });
        let result = task.await;
        let active_pid = manager.child_pid().await;
        let stopped = manager.stop().await;
        server.abort();
        let _ = server.await;
        assert!(matches!(stopped.state, PreviewState::Stopped));
        let pid = match result {
            Ok(pid) => Some(pid),
            Err(error) => {
                if let Some(pid) = active_pid {
                    assert_eq!(
                        unsafe { libc::kill(pid as i32, 0) },
                        -1,
                        "Vite child survived test panic"
                    );
                }
                std::panic::resume_unwind(error.into_panic());
            }
        };
        if let Some(pid) = pid {
            assert_eq!(
                unsafe { libc::kill(pid as i32, 0) },
                -1,
                "Vite child was not reaped"
            );
        }
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
        assert_eq!(body["outcome"]["reason"]["kind"], "parse");
        assert!(body["outcome"]["reason"]["reason"].is_string());
        assert!(body["outcome"]["span"]["start"].is_number());
        assert!(body["outcome"]["span"]["end"].is_number());
        let (st, body) = call(
            "POST",
            "/projects/modify",
            serde_json::json!({"project_path": path, "command": "Add navigation to top nav"}),
        )
        .await;
        assert_eq!(st, StatusCode::OK);
        assert_eq!(body["outcome"]["kind"], "needs_clarification");
        assert_eq!(body["outcome"]["persisted"], false);
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
    async fn preview_carries_diffs_and_apply_is_bound_to_the_previewed_revision() {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path().join("projects").to_string_lossy().into_owned();
        let (_, body) = call(
            "POST",
            "/projects",
            serde_json::json!({"base_path": base, "name": "review"}),
        )
        .await;
        let path = body["path"].as_str().unwrap().to_string();
        let command = "Need a top navigation";
        let (_, preview) = call(
            "POST",
            "/projects/modify",
            serde_json::json!({"project_path": path, "command": command, "dry_run": true}),
        )
        .await;
        let outcome = &preview["outcome"];
        assert_eq!(outcome["kind"], "preview");
        assert_eq!(outcome["base_revision"], 0);
        assert!(outcome["diffs"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["path"] == "src/components/TopNav.tsx"
                && d["diff"].as_str().unwrap().contains("+")));
        assert!(!std::path::Path::new(&path).join("src/components").exists());

        // Another change lands: the preview's revision is stale.
        call(
            "POST",
            "/projects/modify",
            serde_json::json!({"project_path": path, "command": "Add a footer"}),
        )
        .await;
        let apply = |expected: u64| {
            serde_json::json!({
                "project_path": path, "command": command,
                "expected_revision": expected, "expected_plan_id": outcome["plan_id"],
            })
        };
        let (st, stale) = call("POST", "/projects/modify", apply(0)).await;
        assert_eq!(st, StatusCode::OK);
        assert_eq!(stale["outcome"]["kind"], "conflict");
        assert!(!std::path::Path::new(&path)
            .join("src/components/TopNav.tsx")
            .exists());

        let (_, fresh) = call(
            "POST",
            "/projects/modify",
            serde_json::json!({"project_path": path, "command": command, "dry_run": true}),
        )
        .await;
        let rev = fresh["outcome"]["base_revision"].as_u64().unwrap();
        let (_, applied) = call(
            "POST",
            "/projects/modify",
            serde_json::json!({
                "project_path": path, "command": command,
                "expected_revision": rev, "expected_plan_id": fresh["outcome"]["plan_id"],
            }),
        )
        .await;
        assert_eq!(applied["outcome"]["kind"], "applied", "{applied}");
        assert!(std::path::Path::new(&path)
            .join("src/components/TopNav.tsx")
            .exists());
    }

    #[tokio::test]
    async fn chat_echoes() {
        let (st, body) = call("POST", "/chat", serde_json::json!({"prompt": "hi"})).await;
        assert_eq!(st, StatusCode::OK);
        assert_eq!(body["reply"], "hi");
    }

    #[tokio::test]
    async fn runtime_endpoint_returns_the_shared_contract() {
        let (status, body) = call("GET", PREVIEW_RUNTIME_PATH, serde_json::Value::Null).await;
        assert_eq!(status, StatusCode::OK);
        let response: RuntimeCheckResponse = serde_json::from_value(body).unwrap();
        assert_eq!(
            response.required_node_range,
            protopie_api::REQUIRED_NODE_RANGE
        );
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
