use std::path::{Path, PathBuf};

use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use protopie_api::{
    ChatRequest, ChatResponse, CreateProjectRequest, CreateProjectResponse, ErrorResponse,
    ListProjectsResponse, ModifyProjectRequest, ModifyProjectResponse, ProjectInfo, CHAT_PATH,
    HEALTH_PATH, PROJECTS_MODIFY_PATH, PROJECTS_PATH,
};
use protopie_ui_agent as agent;

/// Folder `GET /projects` lists. Resolved against the server's cwd when relative.
const PROJECTS_BASE_PATH: &str = ".projects";

/// An HTTP error with a JSON `{"error": ...}` body.
struct ApiError(StatusCode, String);

impl From<agent::Error> for ApiError {
    fn from(e: agent::Error) -> Self {
        let status = match &e {
            agent::Error::InvalidSlug(_) | agent::Error::Escape(_) => StatusCode::BAD_REQUEST,
            agent::Error::AlreadyExists(_) => StatusCode::CONFLICT,
            agent::Error::NotADirectory(_) => StatusCode::NOT_FOUND,
            agent::Error::Io { .. } => StatusCode::INTERNAL_SERVER_ERROR,
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
        Err(e) => Err(ApiError(StatusCode::INTERNAL_SERVER_ERROR, format!("task failed: {e}"))),
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
            .map(|name| ProjectInfo { path: path_string(&abs.join(&name)), name })
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
    Ok((StatusCode::CREATED, Json(CreateProjectResponse { slug, path: path_string(&path) })))
}

async fn modify_project(
    Json(req): Json<ModifyProjectRequest>,
) -> Result<Json<ModifyProjectResponse>, ApiError> {
    let reply =
        blocking(move || agent::modify(Path::new(&req.project_path), &req.command)).await?;
    Ok(Json(ModifyProjectResponse { reply }))
}

fn app() -> Router {
    Router::new()
        .route(CHAT_PATH, post(chat))
        .route(HEALTH_PATH, get(|| async { "ok" }))
        .route(PROJECTS_PATH, get(list_projects).post(create_project))
        .route(PROJECTS_MODIFY_PATH, post(modify_project))
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

    async fn call(method: &str, uri: &str, body: serde_json::Value) -> (StatusCode, serde_json::Value) {
        let req = Request::builder()
            .method(method)
            .uri(uri)
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap();
        let resp = app().oneshot(req).await.unwrap();
        let status = resp.status();
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        (status, serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null))
    }

    #[tokio::test]
    async fn create_then_modify_and_errors() {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path().join("projects").to_string_lossy().into_owned();

        let (st, body) = call("POST", "/projects", serde_json::json!({"base_path": base, "name": "My App"})).await;
        assert_eq!(st, StatusCode::CREATED);
        assert_eq!(body["slug"], "my-app");
        let path = body["path"].as_str().unwrap().to_string();

        let (st, _) = call("POST", "/projects", serde_json::json!({"base_path": base, "name": "my app"})).await;
        assert_eq!(st, StatusCode::CONFLICT);
        let (st, body) = call("POST", "/projects", serde_json::json!({"base_path": base, "name": "!!!"})).await;
        assert_eq!(st, StatusCode::BAD_REQUEST);
        assert!(body["error"].is_string());

        let (st, body) = call("POST", "/projects/modify", serde_json::json!({"project_path": path, "command": "Add top header"})).await;
        assert_eq!(st, StatusCode::OK);
        assert_eq!(body["reply"], "received: Add top header");
        let (st, _) = call("POST", "/projects/modify", serde_json::json!({"project_path": "/nonexistent/x", "command": "x"})).await;
        assert_eq!(st, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn chat_echoes() {
        let (st, body) = call("POST", "/chat", serde_json::json!({"prompt": "hi"})).await;
        assert_eq!(st, StatusCode::OK);
        assert_eq!(body["reply"], "hi");
    }
}
