use serde::{Deserialize, Serialize};

pub const CHAT_PATH: &str = "/chat";
pub const HEALTH_PATH: &str = "/health";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatRequest {
    pub prompt: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatResponse {
    pub reply: String,
}

pub const PROJECTS_PATH: &str = "/projects";
pub const PROJECTS_MODIFY_PATH: &str = "/projects/modify";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectInfo {
    pub name: String,
    /// Full path of the project folder.
    pub path: String,
}

/// Response of `GET /projects`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListProjectsResponse {
    pub projects: Vec<ProjectInfo>,
}

/// Body of `POST /projects`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateProjectRequest {
    /// Folder the project is created in; relative paths resolve against the server's cwd.
    pub base_path: String,
    /// Human-entered name; the server slugifies it into the folder name.
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateProjectResponse {
    pub slug: String,
    /// Full path of the new project folder.
    pub path: String,
}

/// Body of `POST /projects/modify`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModifyProjectRequest {
    pub project_path: String,
    pub command: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModifyProjectResponse {
    pub reply: String,
}

/// JSON body of every non-2xx response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorResponse {
    pub error: String,
}
