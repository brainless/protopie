use serde::{Deserialize, Serialize};

pub mod diagnostics;

pub const CHAT_PATH: &str = "/chat";
pub const HEALTH_PATH: &str = "/health";
pub const PREVIEW_RUNTIME_PATH: &str = "/preview/runtime";
pub const PREVIEW_LAUNCH_PATH: &str = "/preview/launch";
pub const PREVIEW_STOP_PATH: &str = "/preview/stop";
pub const PREVIEW_RESTART_PATH: &str = "/preview/restart";
pub const PREVIEW_STATUS_PATH: &str = "/preview/status";
pub const PREVIEW_LOGS_PATH: &str = "/preview/logs";

/// Kept in sync with the Vite and Solid Vite plugin versions in the template lockfile.
pub const REQUIRED_NODE_RANGE: &str = "^20.19.0 || >=22.12.0";

/// Chat suggestions. Prerequisites describe when the command can produce a plan.
pub struct ChatExample {
    pub prompt: &'static str,
    pub prerequisite: &'static str,
    pub next_step: &'static str,
}

pub const CHAT_EXAMPLES: &[ChatExample] = &[
    ChatExample {
        prompt: "Need a top navigation",
        prerequisite: "Fresh project",
        next_step: "Preview, then Apply",
    },
    ChatExample {
        prompt: "Add a Doctors page",
        prerequisite: "Fresh project",
        next_step: "Preview, then Apply",
    },
    ChatExample {
        prompt: "Add a form below hero",
        prerequisite: "Fresh project",
        next_step: "Answer fields and submit label, then preview and Apply",
    },
    ChatExample {
        prompt: "Give the form below hero more padding",
        prerequisite: "Apply 'Add a form below hero' first",
        next_step: "Preview, then Apply",
    },
    ChatExample {
        prompt: "Share the selected doctor across pages",
        prerequisite: "Apply 'Add a Doctors page' first",
        next_step: "Answer type, value, destination and done; preview and Apply",
    },
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeExecutable {
    /// Absolute path to the executable used by the preview manager.
    pub path: String,
    pub version: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RuntimeFailure {
    MissingExecutable {
        tool: String,
        message: String,
    },
    UnsupportedVersion {
        tool: String,
        found: String,
        required: String,
        message: String,
    },
    CommandFailed {
        tool: String,
        path: String,
        details: String,
        message: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RuntimeStatus {
    Available,
    Unavailable { reason: RuntimeFailure },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeCheckResponse {
    pub node: Option<RuntimeExecutable>,
    pub npm: Option<RuntimeExecutable>,
    pub required_node_range: String,
    pub status: RuntimeStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PreviewProjectRequest {
    pub project_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PreviewLogRequest {
    pub project_path: String,
    /// Return lines after this cursor. Zero starts at the oldest retained line.
    pub cursor: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PreviewFailure {
    Environment { reason: RuntimeFailure },
    UnsupportedProject { message: String },
    Install { message: String },
    Start { message: String },
    Readiness { message: String },
    Exited { message: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PreviewState {
    Stopped,
    Preparing { step: String },
    Starting,
    Running { url: String },
    Failed { reason: PreviewFailure },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PreviewStatusResponse {
    /// Canonical path of the active project, if one is selected for preview.
    pub project_path: Option<String>,
    /// Increments for each launch, stop, restart, or project switch.
    pub generation: u64,
    pub state: PreviewState,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PreviewLogStream {
    Stdout,
    Stderr,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PreviewLogLine {
    pub cursor: u64,
    pub stream: PreviewLogStream,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PreviewLogResponse {
    pub project_path: Option<String>,
    pub generation: u64,
    /// Send this value in the next request, even when `lines` is empty.
    pub next_cursor: u64,
    /// Earlier lines were discarded before the supplied cursor was served.
    pub truncated: bool,
    pub lines: Vec<PreviewLogLine>,
}

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
    /// Last modification of `.protopie/model.json`, milliseconds since Unix epoch.
    #[serde(default)]
    pub last_modified_unix_ms: Option<u64>,
    /// Why this folder cannot be opened as a template-derived project.
    #[serde(default)]
    pub unavailable_reason: Option<String>,
}

#[cfg(test)]
mod project_info_tests {
    use super::ProjectInfo;

    #[test]
    fn eligibility_and_model_timestamp_round_trip_and_old_lists_decode() {
        let old: ProjectInfo = serde_json::from_str(r#"{"name":"old","path":"/p/old"}"#).unwrap();
        assert!(old.last_modified_unix_ms.is_none());
        assert!(old.unavailable_reason.is_none());
        let listed = ProjectInfo {
            name: "old".into(),
            path: "/p/old".into(),
            last_modified_unix_ms: Some(42),
            unavailable_reason: Some("Create a project from the reference template.".into()),
        };
        assert_eq!(
            serde_json::from_str::<ProjectInfo>(&serde_json::to_string(&listed).unwrap()).unwrap(),
            listed
        );
    }
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
    /// Apply only against this project revision (a preview's `base_revision`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_revision: Option<u64>,
    /// Apply only the plan a preview showed (its `plan_id`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_plan_id: Option<String>,
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
    /// Apply only against this project revision (a preview's `base_revision`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_revision: Option<u64>,
    /// Apply only the plan a preview showed (its `plan_id`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_plan_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModifyProjectResponse {
    /// Human-readable summary for chat.
    pub reply: String,
    /// Structured result; absent from older servers.
    #[serde(default)]
    pub outcome: Option<ModifyOutcome>,
}

/// A proposed change to one file as a line diff (`-` removed, `+` added).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileDiff {
    pub path: String,
    pub diff: String,
}

/// Longest diff shown per file in chat text.
const DIFF_PREVIEW_LINES: usize = 40;

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

/// Half-open UTF-8 byte range in the submitted command.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiagnosticSpan {
    pub start: usize,
    pub end: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ParserRejectionReason {
    EmptyInput,
    UnrecognizedInput,
    IncompleteInput,
    NegatedRequest,
    UnsupportedTail,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RejectionReason {
    Parse { reason: ParserRejectionReason },
    CapabilityNotImplemented,
    TargetNotFound,
    UnsupportedLayout,
    ProjectNotInitialized,
}

/// Wire form of the agent's modify outcome.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ModifyOutcome {
    /// Dry run result; nothing was written.
    Preview {
        plan_id: String,
        /// Project revision the preview was prepared against; send it back as
        /// `expected_revision` together with `plan_id` to apply this preview.
        #[serde(default)]
        base_revision: u64,
        changed_files: Vec<String>,
        /// Proposed code changes per file; absent from older servers.
        #[serde(default)]
        diffs: Vec<FileDiff>,
        questions: Vec<QuestionSummary>,
    },
    Applied {
        plan_id: String,
        changed_files: Vec<String>,
        questions: Vec<QuestionSummary>,
    },
    NeedsClarification {
        questions: Vec<QuestionSummary>,
        /// True only when a committed question-only plan stored these IDs.
        #[serde(default)]
        persisted: bool,
    },
    NoChange {
        reason: String,
        questions: Vec<QuestionSummary>,
    },
    Unsupported {
        explanation: String,
        #[serde(default)]
        reason: Option<RejectionReason>,
        #[serde(default)]
        span: Option<DiagnosticSpan>,
    },
    Conflict {
        reason: String,
    },
}

impl ModifyProjectResponse {
    /// Text for the chat view: the summary plus the outcome's details.
    pub fn display_text(&self) -> String {
        let mut text = self.reply.clone();
        if let Some(ModifyOutcome::Preview { diffs, .. }) = &self.outcome {
            for d in diffs {
                text.push_str(&format!("\n\n{}", d.path));
                let lines: Vec<&str> = d.diff.lines().skip(2).collect();
                for l in lines.iter().take(DIFF_PREVIEW_LINES) {
                    text.push_str(&format!("\n{l}"));
                }
                if lines.len() > DIFF_PREVIEW_LINES {
                    text.push_str(&format!(
                        "\n... {} more lines",
                        lines.len() - DIFF_PREVIEW_LINES
                    ));
                }
            }
        }
        let questions: &[QuestionSummary] = match &self.outcome {
            Some(ModifyOutcome::Unsupported {
                explanation, span, ..
            }) => {
                text.push_str(&format!("\nUnsupported: {explanation}"));
                if let Some(fragment) = span.and_then(|s| {
                    (s.start < s.end)
                        .then(|| self.reply.strip_prefix("received: "))
                        .flatten()
                        .and_then(|prompt| prompt.get(s.start..s.end))
                }) {
                    text.push_str(&format!("\nUnrecognized: “{fragment}”"));
                    text.push_str(&format!(
                        "\nTry: {}; {}; {}.",
                        CHAT_EXAMPLES[0].prompt, CHAT_EXAMPLES[1].prompt, CHAT_EXAMPLES[2].prompt
                    ));
                }
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
            Some(ModifyOutcome::NeedsClarification {
                questions,
                persisted,
            }) => {
                for q in questions {
                    for (i, option) in q.options.iter().enumerate() {
                        text.push_str(&format!("\n  {}. {option}", i + 1));
                    }
                }
                if !persisted {
                    text.push_str("\nType a clarified request to continue.");
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
    fn preview_contract_round_trips_and_uses_stable_tags() {
        let runtime = RuntimeCheckResponse {
            node: Some(RuntimeExecutable {
                path: "/usr/bin/node".into(),
                version: "v22.12.0".into(),
            }),
            npm: Some(RuntimeExecutable {
                path: "/usr/bin/npm".into(),
                version: "10.0.0".into(),
            }),
            required_node_range: REQUIRED_NODE_RANGE.into(),
            status: RuntimeStatus::Available,
        };
        let value = serde_json::to_value(&runtime).unwrap();
        assert_eq!(value["status"]["kind"], "available");
        assert_eq!(
            serde_json::from_value::<RuntimeCheckResponse>(value).unwrap(),
            runtime
        );

        let failure = RuntimeFailure::UnsupportedVersion {
            tool: "node".into(),
            found: "v18.0.0".into(),
            required: REQUIRED_NODE_RANGE.into(),
            message: "Install Node".into(),
        };
        let status = PreviewStatusResponse {
            project_path: Some("/projects/a".into()),
            generation: 4,
            state: PreviewState::Failed {
                reason: PreviewFailure::Environment { reason: failure },
            },
        };
        let value = serde_json::to_value(&status).unwrap();
        assert_eq!(value["state"]["kind"], "failed");
        assert_eq!(value["state"]["reason"]["kind"], "environment");
        assert_eq!(
            value["state"]["reason"]["reason"]["kind"],
            "unsupported_version"
        );
        assert_eq!(
            serde_json::from_value::<PreviewStatusResponse>(value).unwrap(),
            status
        );

        for state in [
            PreviewState::Stopped,
            PreviewState::Preparing {
                step: "npm ci".into(),
            },
            PreviewState::Starting,
            PreviewState::Running {
                url: "http://127.0.0.1:5173/".into(),
            },
        ] {
            let value = serde_json::to_value(&state).unwrap();
            assert_eq!(
                serde_json::from_value::<PreviewState>(value).unwrap(),
                state
            );
        }
        let request = PreviewLogRequest {
            project_path: "/projects/a".into(),
            cursor: 3,
        };
        assert_eq!(
            serde_json::from_value::<PreviewLogRequest>(serde_json::to_value(&request).unwrap())
                .unwrap(),
            request
        );
        let logs = PreviewLogResponse {
            project_path: Some(request.project_path),
            generation: 4,
            next_cursor: 5,
            truncated: false,
            lines: vec![PreviewLogLine {
                cursor: 5,
                stream: PreviewLogStream::Stderr,
                text: "error".into(),
            }],
        };
        assert_eq!(
            serde_json::from_value::<PreviewLogResponse>(serde_json::to_value(&logs).unwrap())
                .unwrap(),
            logs
        );
        let request = PreviewProjectRequest {
            project_path: "/projects/a".into(),
        };
        assert_eq!(
            serde_json::from_value::<PreviewProjectRequest>(
                serde_json::to_value(&request).unwrap()
            )
            .unwrap(),
            request
        );
    }

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
            expected_revision: Some(3),
            expected_plan_id: None,
        })
        .unwrap();
        assert_eq!(json["conversation_id"], "gui");
        assert_eq!(json["request_id"], "r1");
        assert_eq!(json["expected_revision"], 3);
        assert!(json.get("expected_plan_id").is_none());
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
    fn preview_text_shows_diffs_and_old_previews_still_parse() {
        let old: ModifyOutcome = serde_json::from_str(
            r#"{"kind":"preview","plan_id":"p","changed_files":[],"questions":[]}"#,
        )
        .unwrap();
        assert!(matches!(
            old,
            ModifyOutcome::Preview {
                base_revision: 0,
                ..
            }
        ));
        let long: String = (0..50).map(|i| format!("+line {i}\n")).collect();
        let preview = ModifyProjectResponse {
            reply: "Preview: x".into(),
            outcome: Some(ModifyOutcome::Preview {
                plan_id: "p".into(),
                base_revision: 2,
                changed_files: vec!["a.tsx".into()],
                diffs: vec![FileDiff {
                    path: "a.tsx".into(),
                    diff: format!("--- /dev/null\n+++ a.tsx\n{long}"),
                }],
                questions: vec![],
            }),
        };
        let text = preview.display_text();
        assert!(text.starts_with("Preview: x\n\na.tsx\n+line 0\n"));
        assert!(!text.contains("--- /dev/null"));
        assert!(text.ends_with("... 10 more lines"));
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
                reason: None,
                span: None,
            }),
        };
        assert_eq!(unsupported.display_text(), "received: x\nUnsupported: nope");
        let diagnostic = ModifyProjectResponse {
            reply: "received: Add café widgets".into(),
            outcome: Some(ModifyOutcome::Unsupported {
                explanation: "unknown request".into(),
                reason: Some(RejectionReason::Parse {
                    reason: ParserRejectionReason::UnsupportedTail,
                }),
                span: Some(DiagnosticSpan { start: 10, end: 17 }),
            }),
        };
        let encoded = serde_json::to_string(&diagnostic).unwrap();
        let restored: ModifyProjectResponse = serde_json::from_str(&encoded).unwrap();
        assert_eq!(restored.outcome, diagnostic.outcome);
        assert!(diagnostic
            .display_text()
            .contains("Unrecognized: “widgets”"));
        assert!(diagnostic
            .display_text()
            .contains("Try: Need a top navigation"));
        let mut invalid = diagnostic;
        if let Some(ModifyOutcome::Unsupported { span, .. }) = &mut invalid.outcome {
            *span = Some(DiagnosticSpan { start: 8, end: 10 }); // cuts through é
        }
        assert!(!invalid.display_text().contains("Unrecognized:"));
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
                persisted: false,
            }),
        };
        assert_eq!(
            question.display_text(),
            "Which?\n  1. A\n  2. B\nType a clarified request to continue."
        );
    }

    #[test]
    fn clarification_wire_distinguishes_stored_questions_from_rephrase_suggestions() {
        let old: ModifyOutcome =
            serde_json::from_str(r#"{"kind":"needs_clarification","questions":[]}"#).unwrap();
        assert!(matches!(
            old,
            ModifyOutcome::NeedsClarification {
                persisted: false,
                ..
            }
        ));
        let stored = ModifyOutcome::NeedsClarification {
            questions: vec![],
            persisted: true,
        };
        let encoded = serde_json::to_value(&stored).unwrap();
        assert_eq!(encoded["persisted"], true);
        assert_eq!(
            serde_json::from_value::<ModifyOutcome>(encoded).unwrap(),
            stored
        );
    }
}
