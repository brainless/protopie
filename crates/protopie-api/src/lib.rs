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
pub const PROJECTS_ANSWER_PATH: &str = "/projects/answer";
pub const PROJECTS_ABORT_PATH: &str = "/projects/recovery/abort";
pub const ABORT_COMMAND: &str = "/abort-interrupted-apply";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AbortProjectRequest {
    pub project_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AbortProjectResponse {
    Aborted {
        plan_id: String,
        preserved_paths: Vec<String>,
    },
    NothingToDo,
    Conflict {
        reason: String,
    },
}

impl AbortProjectResponse {
    pub fn display_text(&self) -> String {
        match self {
            Self::Aborted {
                plan_id,
                preserved_paths,
            } => {
                if preserved_paths.is_empty() {
                    format!("Aborted interrupted application {plan_id}.")
                } else {
                    format!("Aborted interrupted application {plan_id}; preserved external edits in {}.", preserved_paths.join(", "))
                }
            }
            Self::NothingToDo => "No interrupted application is pending.".into(),
            Self::Conflict { reason } => format!("Cannot abort interrupted application: {reason}"),
        }
    }
}

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
    /// Conversation scope (`[a-z0-9-]+`); the project's default when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversation_id: Option<String>,
    /// Unique per user action. Re-sending the same ID is a transport retry and
    /// returns the recorded result instead of applying again.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    /// Prepare only: report what would change and write nothing.
    #[serde(default)]
    pub dry_run: bool,
}

/// Body of `POST /projects/answer`: a structured answer to a pending question
/// of the conversation. Send `option_key` (a key from the question's
/// `option_keys`) or free `text` (an option number or label, a URL or a page
/// name, depending on the question). Replies as `/projects/modify` does.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnswerProjectRequest {
    pub project_path: String,
    pub question_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub option_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversation_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    #[serde(default)]
    pub dry_run: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModifyProjectResponse {
    /// Human-readable summary for chat.
    pub reply: String,
    /// Structured result; absent from older servers.
    #[serde(default)]
    pub outcome: Option<ModifyOutcome>,
}

/// Question summary shown to the user.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuestionSummary {
    pub id: String,
    pub prompt: String,
    /// Option labels in display order.
    pub options: Vec<String>,
    /// Stable option keys, parallel to `options`, for structured answers.
    #[serde(default)]
    pub option_keys: Vec<String>,
    /// True if the answer is free text (a URL, a page name) rather than an option.
    #[serde(default)]
    pub takes_text: bool,
    /// True if the edit cannot proceed until answered.
    pub blocking: bool,
}

/// Wire form of the agent's modify outcome.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ModifyOutcome {
    /// Dry run result; nothing was written.
    Preview {
        plan_id: String,
        changed_files: Vec<String>,
        questions: Vec<QuestionSummary>,
    },
    Applied {
        plan_id: String,
        changed_files: Vec<String>,
        questions: Vec<QuestionSummary>,
    },
    NeedsClarification {
        questions: Vec<QuestionSummary>,
    },
    NoChange {
        reason: String,
        questions: Vec<QuestionSummary>,
    },
    Unsupported {
        explanation: String,
    },
    Conflict {
        reason: String,
    },
}

impl ModifyProjectResponse {
    /// Text for the chat view: the summary plus the outcome's details.
    pub fn display_text(&self) -> String {
        let mut text = self.reply.clone();
        let questions: &[QuestionSummary] = match &self.outcome {
            Some(ModifyOutcome::Unsupported { explanation }) => {
                text.push_str(&format!("\nUnsupported: {explanation}"));
                &[]
            }
            Some(ModifyOutcome::Conflict { reason }) => {
                text.push_str(&format!("\nConflict: {reason}"));
                &[]
            }
            Some(ModifyOutcome::Applied { questions, .. })
            | Some(ModifyOutcome::Preview { questions, .. })
            | Some(ModifyOutcome::NoChange { questions, .. }) => questions,
            // Blocking questions are already the summary.
            Some(ModifyOutcome::NeedsClarification { questions }) => {
                for q in questions {
                    for (i, option) in q.options.iter().enumerate() {
                        text.push_str(&format!("\n  {}. {option}", i + 1));
                    }
                }
                &[]
            }
            None => &[],
        };
        for q in questions {
            text.push_str(&format!("\n? {}", q.prompt));
            for (i, option) in q.options.iter().enumerate() {
                text.push_str(&format!("\n  {}. {option}", i + 1));
            }
        }
        text
    }
}

/// JSON body of every non-2xx response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorResponse {
    pub error: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modify_request_ids_are_optional_on_the_wire() {
        let old: ModifyProjectRequest =
            serde_json::from_str(r#"{"project_path":"/p","command":"x"}"#).unwrap();
        assert!(old.conversation_id.is_none() && old.request_id.is_none() && !old.dry_run);
        let json = serde_json::to_value(ModifyProjectRequest {
            project_path: "/p".into(),
            command: "x".into(),
            conversation_id: Some("gui".into()),
            request_id: Some("r1".into()),
            dry_run: false,
        })
        .unwrap();
        assert_eq!(json["conversation_id"], "gui");
        assert_eq!(json["request_id"], "r1");
    }

    #[test]
    fn answer_requests_and_question_keys_are_optional_on_the_wire() {
        let a: AnswerProjectRequest = serde_json::from_str(
            r#"{"project_path":"/p","question_id":"q1-destination","option_key":"new_page"}"#,
        )
        .unwrap();
        assert_eq!(a.option_key.as_deref(), Some("new_page"));
        assert!(a.text.is_none() && a.request_id.is_none() && !a.dry_run);
        let q: QuestionSummary =
            serde_json::from_str(r#"{"id":"q","prompt":"?","options":["A"],"blocking":false}"#)
                .unwrap();
        assert!(q.option_keys.is_empty() && !q.takes_text);
    }

    #[test]
    fn display_text_lists_follow_up_after_an_applied_change() {
        let applied = ModifyProjectResponse {
            reply: "Added it.".into(),
            outcome: Some(ModifyOutcome::Applied {
                plan_id: "p".into(),
                changed_files: vec![],
                questions: vec![QuestionSummary {
                    id: "q".into(),
                    prompt: "Where?".into(),
                    options: vec!["A".into()],
                    option_keys: vec![],
                    takes_text: false,
                    blocking: false,
                }],
            }),
        };
        assert_eq!(applied.display_text(), "Added it.\n? Where?\n  1. A");
    }

    #[test]
    fn old_servers_without_outcome_still_parse() {
        let r: ModifyProjectResponse = serde_json::from_str(r#"{"reply":"hi"}"#).unwrap();
        assert_eq!(r.display_text(), "hi");
    }

    #[test]
    fn display_text_shows_unsupported_and_questions() {
        let unsupported = ModifyProjectResponse {
            reply: "received: x".into(),
            outcome: Some(ModifyOutcome::Unsupported {
                explanation: "nope".into(),
            }),
        };
        assert_eq!(unsupported.display_text(), "received: x\nUnsupported: nope");
        let question = ModifyProjectResponse {
            reply: "Which?".into(),
            outcome: Some(ModifyOutcome::NeedsClarification {
                questions: vec![QuestionSummary {
                    id: "q".into(),
                    prompt: "Which?".into(),
                    options: vec!["A".into(), "B".into()],
                    option_keys: vec![],
                    takes_text: false,
                    blocking: true,
                }],
            }),
        };
        assert_eq!(question.display_text(), "Which?\n  1. A\n  2. B");
    }
}
