//! Serializable contracts shared by the parse / resolve / plan stages (Epic 001, T1).
//!
//! Each stage has its own distinct types. The parse-stage type is
//! [`crate::parser::Request`], re-exported here as [`ParsedRequest`]. Nothing in
//! this module performs IO.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub use crate::parser::{Request as ParsedRequest, Span, StyleChange, UnsupportedReason};

/// Schema version of the persisted project model (`.protopie/model.json`).
pub const MODEL_SCHEMA_VERSION: u32 = 1;
/// Schema version of persisted conversation state.
pub const SESSION_SCHEMA_VERSION: u32 = 1;
/// Schema version of [`Plan`].
pub const PLAN_SCHEMA_VERSION: u32 = 1;
/// Version of the deterministic planning / style policy.
pub const POLICY_VERSION: u32 = 1;
/// Conversation used by the simple `modify(path, command)` entry point.
pub const DEFAULT_CONVERSATION_ID: &str = "default";
/// Page scope of the reference application's single page.
pub const DEFAULT_PAGE: &str = "home";

/// Stable identifier of a modelled element, e.g. `nav_1`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ElementId(pub String);

impl ElementId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ElementKind {
    Navigation,
    NavigationItem,
    Hero,
    Image,
    Form,
}

// ---------------------------------------------------------------- project / session

/// Where a navigation sits. `None` on a record means unknown, which is treated
/// as compatible with a `top nav` reference.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Placement {
    Top,
    Side,
}

/// Source location of an element: a file and, for shared files, the owned region.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceBinding {
    pub file: String,
    #[serde(default)]
    pub region: Option<String>,
}

/// Whether a style edit through this binding affects one instance or every
/// instance of a component definition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StyleScope {
    Instance,
    Definition,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StyleBinding {
    pub file: String,
    pub class: String,
    #[serde(default)]
    pub region: Option<String>,
    pub scope: StyleScope,
}

/// One modelled element in a project snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ElementRecord {
    pub id: ElementId,
    pub kind: ElementKind,
    #[serde(default)]
    pub label: Option<String>,
    pub page: String,
    #[serde(default)]
    pub parent: Option<ElementId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub placement: Option<Placement>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<SourceBinding>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub style: Option<StyleBinding>,
    /// Where a navigation item leads. `None` for non-items.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub destination: Option<Destination>,
}

impl ElementRecord {
    pub fn new(id: ElementId, kind: ElementKind, page: impl Into<String>) -> Self {
        Self {
            id,
            kind,
            label: None,
            page: page.into(),
            parent: None,
            placement: None,
            source: None,
            style: None,
            destination: None,
        }
    }
}

/// Recorded state of source the agent owns: a whole file (`region: None`) or one
/// marker-delimited region of a shared file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OwnedSource {
    pub path: String,
    #[serde(default)]
    pub region: Option<String>,
    pub fingerprint: String,
}

/// Explicit, read-only view of a project for the pure stages.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectSnapshot {
    pub schema_version: u32,
    /// Incremented by every applied change.
    pub revision: u64,
    #[serde(default)]
    pub elements: Vec<ElementRecord>,
    #[serde(default)]
    pub owned: Vec<OwnedSource>,
    /// Highest number ever issued per ID prefix, so IDs are never reused.
    #[serde(default)]
    pub id_counters: BTreeMap<String, u64>,
}

impl ProjectSnapshot {
    pub fn empty() -> Self {
        Self {
            schema_version: MODEL_SCHEMA_VERSION,
            revision: 0,
            elements: Vec::new(),
            owned: Vec::new(),
            id_counters: BTreeMap::new(),
        }
    }

    pub fn element(&self, id: &ElementId) -> Option<&ElementRecord> {
        self.elements.iter().find(|e| &e.id == id)
    }

    /// Next never-used ID `<prefix>_<n>`: above every existing suffix and every
    /// number already issued. Does not record it; see [`Self::reserve_id`].
    pub fn next_id(&self, prefix: &str) -> ElementId {
        let existing = self
            .elements
            .iter()
            .filter_map(|e| {
                e.id.0
                    .strip_prefix(prefix)?
                    .strip_prefix('_')?
                    .parse::<u64>()
                    .ok()
            })
            .max()
            .unwrap_or(0);
        let issued = self.id_counters.get(prefix).copied().unwrap_or(0);
        ElementId::new(format!("{prefix}_{}", existing.max(issued) + 1))
    }

    /// Like [`Self::next_id`] but records the number as issued.
    pub fn reserve_id(&mut self, prefix: &str) -> ElementId {
        let id = self.next_id(prefix);
        let n =
            id.0.rsplit('_')
                .next()
                .and_then(|n| n.parse().ok())
                .unwrap_or(0);
        self.id_counters.insert(prefix.to_string(), n);
        id
    }

    pub fn of_kind(&self, kind: ElementKind) -> impl Iterator<Item = &ElementRecord> {
        self.elements.iter().filter(move |e| e.kind == kind)
    }
}

impl Default for ProjectSnapshot {
    fn default() -> Self {
        Self::empty()
    }
}

/// Bounded conversation state, scoped by project and conversation ID.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Session {
    pub schema_version: u32,
    pub conversation_id: String,
    /// Turn counter; deterministic replacement for wall-clock recency.
    pub turn: u64,
    /// Element focused by the last successful application.
    #[serde(default)]
    pub focus: Option<ElementId>,
    #[serde(default)]
    pub pending_questions: Vec<Question>,
    /// Current page scope, when known.
    #[serde(default)]
    pub page: Option<String>,
    /// Explicit GUI selection, when available.
    #[serde(default)]
    pub selection: Option<ElementId>,
    /// Bounded log of recent successful edits, oldest first.
    #[serde(default)]
    pub recent: Vec<RecentEdit>,
}

/// A successful edit remembered for diagnostics and focus.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecentEdit {
    pub turn: u64,
    pub element: ElementId,
    pub kind: ElementKind,
}

/// Maximum remembered recent edits and pending questions per conversation.
pub const MAX_RECENT_EDITS: usize = 16;
pub const MAX_PENDING_QUESTIONS: usize = 8;

impl Session {
    pub fn new(conversation_id: impl Into<String>) -> Self {
        Self {
            schema_version: SESSION_SCHEMA_VERSION,
            conversation_id: conversation_id.into(),
            turn: 0,
            focus: None,
            pending_questions: Vec::new(),
            page: None,
            selection: None,
            recent: Vec::new(),
        }
    }

    /// Starts a new turn (call once per accepted prompt that is not a dry run).
    pub fn advance_turn(&mut self) {
        self.turn += 1;
    }

    /// Records a successfully applied edit and moves focus to `element`. This is
    /// the only way focus is established; rejected, previewed or failed
    /// requests never call it. History is bounded and turn-based.
    pub fn record_success(&mut self, element: ElementId, kind: ElementKind) {
        self.focus = Some(element.clone());
        self.recent.push(RecentEdit {
            turn: self.turn,
            element,
            kind,
        });
        if self.recent.len() > MAX_RECENT_EDITS {
            let excess = self.recent.len() - MAX_RECENT_EDITS;
            self.recent.drain(..excess);
        }
    }

    /// Adds a pending question (replacing one with the same ID), dropping the
    /// oldest beyond [`MAX_PENDING_QUESTIONS`].
    pub fn add_pending(&mut self, question: Question) {
        self.pending_questions.retain(|q| q.id != question.id);
        self.pending_questions.push(question);
        if self.pending_questions.len() > MAX_PENDING_QUESTIONS {
            let excess = self.pending_questions.len() - MAX_PENDING_QUESTIONS;
            self.pending_questions.drain(..excess);
        }
    }

    /// Removes and returns the pending question with this ID.
    pub fn take_pending(&mut self, question_id: &str) -> Option<Question> {
        let at = self
            .pending_questions
            .iter()
            .position(|q| q.id == question_id)?;
        Some(self.pending_questions.remove(at))
    }
}

impl Default for Session {
    fn default() -> Self {
        Self::new(DEFAULT_CONVERSATION_ID)
    }
}

// ---------------------------------------------------------------- selectors

/// Selector in the resolve stage's vocabulary. Separate from the parser's
/// `RoleSelector`: it can additionally carry an explicit ID, label and scope.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Selector {
    #[serde(default)]
    pub role: Option<ElementKind>,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub id: Option<ElementId>,
    #[serde(default)]
    pub relation: Option<Box<SelectorRelation>>,
    /// Page scope; `None` means the current scope.
    #[serde(default)]
    pub scope: Option<String>,
}

/// A target's relational constraint (not a move destination).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SelectorRelation {
    Below(Selector),
    Above(Selector),
}

// ---------------------------------------------------------------- resolved

/// Resolve-stage output: concrete element IDs, no unresolved omissions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ResolvedRequest {
    /// Ensure-style: `existing` is the navigation already in scope, if any.
    EnsureNavigation { existing: Option<ElementId> },
    AddNavigationItem {
        navigation: ElementId,
        label: String,
    },
}

// ---------------------------------------------------------------- plan

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Destination {
    /// Rendered as noninteractive text; never an invented URL.
    Unresolved,
    Page {
        page: String,
    },
    Section {
        page: String,
        section: String,
    },
    External {
        url: String,
    },
}

/// Semantic operations; the emitter owns filenames and syntax.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Operation {
    CreateComponent {
        id: ElementId,
        kind: ElementKind,
        name: String,
    },
    InsertElement {
        element: ElementId,
        page: String,
        parent: Option<ElementId>,
    },
    AddNavigationItem {
        navigation: ElementId,
        item: ElementId,
        label: String,
        destination: Destination,
    },
    SetStyle {
        target: ElementId,
        change: StyleChange,
    },
    CreatePage {
        label: String,
    },
    RegisterRoute {
        page: String,
    },
    SetNavigationDestination {
        item: ElementId,
        destination: Destination,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Precondition {
    RevisionIs { revision: u64 },
    ElementExists { id: ElementId },
    ElementAbsent { id: ElementId },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Plan {
    pub id: String,
    pub schema_version: u32,
    pub policy_version: u32,
    pub base_revision: u64,
    pub preconditions: Vec<Precondition>,
    pub operations: Vec<Operation>,
    pub follow_up: Vec<Question>,
}

// ---------------------------------------------------------------- questions / answers

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QuestionKind {
    /// Blocks the edit: no code changes until answered.
    BlockingClarification,
    /// Asked after a successful change; optional to answer.
    FollowUp,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuestionOption {
    /// Stable key an answer refers to, e.g. `"new_page"`.
    pub key: String,
    pub label: String,
}

/// What an answer does once given.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Continuation {
    /// Retry the prompt in the suggested (quoted) form.
    Rephrase { suggestion: String },
    /// Add `label` to the navigation the answer selects.
    ChooseNavigation { label: String },
    /// Choose where an unresolved navigation item leads.
    ChooseDestination { item: ElementId },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Question {
    pub id: String,
    pub kind: QuestionKind,
    #[serde(default)]
    pub target: Option<ElementId>,
    pub prompt: String,
    pub options: Vec<QuestionOption>,
    pub continuation: Continuation,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AnswerChoice {
    Option { key: String },
    Text { text: String },
}

/// Structured answer referencing a question ID.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Answer {
    pub question_id: String,
    pub choice: AnswerChoice,
}

// ---------------------------------------------------------------- outcomes

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RejectionReason {
    /// The parser rejected the prompt.
    Parse { reason: UnsupportedReason },
    /// Understood, but no planner/emitter capability exists yet.
    CapabilityNotImplemented,
    /// The project has no (or an unreadable) `.protopie/` model; see
    /// [`crate::project::initialize_project`].
    ProjectNotInitialized,
}

/// Unsupported portion of a request with an explanation; no code changes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rejection {
    pub reason: RejectionReason,
    pub explanation: String,
    #[serde(default)]
    pub span: Option<Span>,
}

/// Result of the pure parse -> resolve -> plan pipeline.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PlanOutcome {
    /// A plan, possibly with follow-up questions.
    Ready {
        plan: Plan,
    },
    /// Blocking questions; no code changes.
    NeedsClarification {
        questions: Vec<Question>,
    },
    /// Already satisfied.
    NoChange {
        reason: String,
        follow_up: Vec<Question>,
    },
    Unsupported {
        rejection: Rejection,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ResolveOutcome {
    Resolved { request: ResolvedRequest },
    NeedsClarification { questions: Vec<Question> },
    Unsupported { rejection: Rejection },
}

/// What `modify` reports. Extends [`PlanOutcome`] with application results.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ModifyOutcome {
    /// Dry run: the plan was prepared but nothing was written.
    Preview {
        plan_id: String,
        changed_files: Vec<String>,
        follow_up: Vec<Question>,
    },
    Applied {
        plan_id: String,
        changed_files: Vec<String>,
        follow_up: Vec<Question>,
    },
    NeedsClarification {
        questions: Vec<Question>,
    },
    NoChange {
        reason: String,
        follow_up: Vec<Question>,
    },
    Unsupported {
        rejection: Rejection,
    },
    /// A plan could not be applied against the current project state.
    Conflict {
        reason: String,
    },
}

/// Structured outcome plus the human-readable summary for chat.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModifyResult {
    pub outcome: ModifyOutcome,
    pub summary: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roundtrip<T>(value: &T)
    where
        T: Serialize + for<'de> Deserialize<'de> + PartialEq + std::fmt::Debug,
    {
        let json = serde_json::to_string(value).unwrap();
        let back: T = serde_json::from_str(&json).unwrap();
        assert_eq!(&back, value, "{json}");
    }

    #[test]
    fn schemas_carry_versions_and_roundtrip() {
        let project = ProjectSnapshot::empty();
        let session = Session::default();
        assert_eq!(project.schema_version, MODEL_SCHEMA_VERSION);
        assert_eq!(session.schema_version, SESSION_SCHEMA_VERSION);
        assert_eq!(session.conversation_id, DEFAULT_CONVERSATION_ID);
        roundtrip(&project);
        roundtrip(&session);
        // Optional collections may be omitted in persisted JSON.
        let minimal: Session =
            serde_json::from_str(r#"{"schema_version":1,"conversation_id":"c","turn":3}"#).unwrap();
        assert_eq!(minimal.turn, 3);
        assert!(minimal.pending_questions.is_empty());
    }

    #[test]
    fn plans_questions_and_outcomes_roundtrip_with_stable_shape() {
        let question = Question {
            id: "q1".into(),
            kind: QuestionKind::FollowUp,
            target: Some(ElementId::new("item_1")),
            prompt: "Where should it lead?".into(),
            options: vec![QuestionOption {
                key: "unlinked".into(),
                label: "Leave unlinked".into(),
            }],
            continuation: Continuation::ChooseDestination {
                item: ElementId::new("item_1"),
            },
        };
        let plan = Plan {
            id: "p".into(),
            schema_version: PLAN_SCHEMA_VERSION,
            policy_version: POLICY_VERSION,
            base_revision: 2,
            preconditions: vec![Precondition::RevisionIs { revision: 2 }],
            operations: vec![
                Operation::AddNavigationItem {
                    navigation: ElementId::new("nav_1"),
                    item: ElementId::new("item_1"),
                    label: "Contact Us".into(),
                    destination: Destination::Unresolved,
                },
                Operation::SetStyle {
                    target: ElementId::new("img_1"),
                    change: StyleChange::RoundedCorners,
                },
            ],
            follow_up: vec![question.clone()],
        };
        roundtrip(&plan);
        roundtrip(&PlanOutcome::Ready { plan });
        roundtrip(&Answer {
            question_id: "q1".into(),
            choice: AnswerChoice::Option {
                key: "unlinked".into(),
            },
        });
        roundtrip(&ModifyOutcome::Conflict {
            reason: "revision changed".into(),
        });
        assert_eq!(
            serde_json::to_value(&ModifyOutcome::NeedsClarification {
                questions: vec![question]
            })
            .unwrap()["kind"],
            "needs_clarification"
        );
        assert_eq!(
            serde_json::to_value(Destination::Unresolved).unwrap(),
            serde_json::json!({"kind": "unresolved"})
        );
    }

    #[test]
    fn parsed_requests_serialize() {
        let outcome = crate::parser::parse("Add Contact Us to top nav");
        let json = serde_json::to_string(&outcome).unwrap();
        let back: crate::parser::ParseOutcome = serde_json::from_str(&json).unwrap();
        assert_eq!(back, outcome);
    }
}
