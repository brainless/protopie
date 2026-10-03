//! Serializable contracts shared by the parse / resolve / plan stages (Epic 001, T1).
//!
//! Each stage has its own distinct types. The parse-stage type is
//! [`crate::parser::Request`], re-exported here as [`ParsedRequest`]. Nothing in
//! this module performs IO.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub use crate::parser::{
    ElementRole, PositionRelation, Request as ParsedRequest, Span, StyleChange, UnsupportedReason,
};

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
/// Page scope of the reference application's initial page. It is implicit in
/// every project: it has no [`PageRecord`] and always lives at [`HOME_PATH`].
pub const DEFAULT_PAGE: &str = "home";
pub const HOME_PATH: &str = "/";
pub const HOME_LABEL: &str = "Home";

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
    Footer,
    Button,
}

impl ElementKind {
    /// Elements that stack in a page's top-level flow and can be positioned
    /// relative to one another.
    pub fn is_flow(self) -> bool {
        matches!(
            self,
            ElementKind::Hero
                | ElementKind::Image
                | ElementKind::Form
                | ElementKind::Footer
                | ElementKind::Button
        )
    }

    /// Elements that lead somewhere (their `destination` is meaningful).
    pub fn has_destination(self) -> bool {
        matches!(self, ElementKind::NavigationItem | ElementKind::Button)
    }

    /// Plain word used in messages and as the ID prefix stem.
    pub fn word(self) -> &'static str {
        match self {
            ElementKind::Navigation => "navigation",
            ElementKind::NavigationItem => "navigation item",
            ElementKind::Hero => "hero",
            ElementKind::Image => "image",
            ElementKind::Form => "form",
            ElementKind::Footer => "footer",
            ElementKind::Button => "button",
        }
    }

    /// Prefix of generated element IDs (`form_1`, `img_2`).
    pub fn id_prefix(self) -> &'static str {
        match self {
            ElementKind::Navigation => "nav",
            ElementKind::NavigationItem => "item",
            ElementKind::Hero => "hero",
            ElementKind::Image => "img",
            ElementKind::Form => "form",
            ElementKind::Footer => "footer",
            ElementKind::Button => "button",
        }
    }
}

impl From<ElementRole> for ElementKind {
    fn from(role: ElementRole) -> Self {
        match role {
            ElementRole::Image => ElementKind::Image,
            ElementRole::Form => ElementKind::Form,
            ElementRole::Hero => ElementKind::Hero,
            ElementRole::Footer => ElementKind::Footer,
            ElementRole::Button => ElementKind::Button,
        }
    }
}

/// Where an element goes or moves to: a relation to a concrete anchor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Position {
    pub relation: PositionRelation,
    pub anchor: ElementId,
}

/// One piece of content a generated element needs before it can exist. The
/// agent asks for each missing one instead of inventing text, images or
/// fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContentField {
    /// Headline of a hero.
    Headline,
    /// Text of a footer.
    FooterText,
    /// Label of a button.
    ButtonLabel,
    /// Address of an image (an http(s) URL).
    ImageSource,
    /// Alternative text of an image.
    ImageAlt,
    /// Field labels of a form (comma separated when asked).
    FormFields,
    /// Label of a form's submit button.
    SubmitLabel,
}

/// Literal content of a generated element; every field is user supplied.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ElementContent {
    /// Hero headline, footer text, button label or form submit label.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// Image address.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub src: Option<String>,
    /// Image alternative text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alt: Option<String>,
    /// Form field labels, in order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fields: Vec<String>,
}

impl ElementContent {
    /// The first content field `kind` still needs, if any.
    pub fn missing(&self, kind: ElementKind) -> Option<ContentField> {
        let text = self.text.is_none();
        match kind {
            ElementKind::Hero if text => Some(ContentField::Headline),
            ElementKind::Footer if text => Some(ContentField::FooterText),
            ElementKind::Button if text => Some(ContentField::ButtonLabel),
            ElementKind::Image if self.src.is_none() => Some(ContentField::ImageSource),
            ElementKind::Image if self.alt.is_none() => Some(ContentField::ImageAlt),
            ElementKind::Form if self.fields.is_empty() => Some(ContentField::FormFields),
            ElementKind::Form if text => Some(ContentField::SubmitLabel),
            _ => None,
        }
    }
}

/// An element to create: what it is, where it goes and the content gathered
/// so far. A spec with [`ElementContent::missing`] content is not plannable yet.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ElementSpec {
    pub kind: ElementKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position: Option<Position>,
    #[serde(default)]
    pub content: ElementContent,
}

/// What a placement question is about.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PlacementAction {
    Add { spec: ElementSpec },
    Move { target: ElementId },
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

/// How a container lays out its children. Only known layouts support relational
/// selectors (`below`) and `full width`; source order alone is never geometry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContainerLayout {
    Vertical,
    Horizontal,
}

/// A style property the style policy can edit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StyleProperty {
    Padding,
    BorderRadius,
    Width,
}

impl StyleProperty {
    /// The CSS property name; also the key in [`StyleBinding::values`].
    pub fn css_name(self) -> &'static str {
        match self {
            StyleProperty::Padding => "padding",
            StyleProperty::BorderRadius => "border-radius",
            StyleProperty::Width => "width",
        }
    }
}

/// One absolute style result: the value is never read back and incremented by
/// the emitter.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StyleEdit {
    pub property: StyleProperty,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StyleBinding {
    pub file: String,
    pub class: String,
    #[serde(default)]
    pub region: Option<String>,
    pub scope: StyleScope,
    /// Declared values of the properties the style policy edits, keyed by CSS
    /// name (`padding`, `border-radius`, `width`). This is the planner's view
    /// of the cascade for this element; absent means undeclared.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub values: BTreeMap<String, String>,
    /// True once the element has its own rule
    /// `.class[data-protopie-id="<id>"]`, so edits no longer touch the shared
    /// class rule.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub instance_override: bool,
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
    /// How this element lays out its children, when it is a container with a
    /// known layout. Sibling order is the order of `ProjectSnapshot::elements`
    /// among elements with the same page and parent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layout: Option<ContainerLayout>,
    /// Literal content of a generated element (hero, image, form, footer,
    /// button); the component is regenerated from it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<ElementContent>,
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
            layout: None,
            content: None,
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

/// A page created by the agent (the implicit home page has no record).
///
/// `id` doubles as the page scope of elements (`ElementRecord::page`) and as
/// the route slug: `path` is always `/<id>`. The emitter derives the component
/// and file names from it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PageRecord {
    pub id: String,
    pub label: String,
    pub path: String,
    /// Component (and file stem) of the page in `src/pages/`.
    pub component: String,
    /// True once a route for the page is registered in the router.
    #[serde(default)]
    pub registered: bool,
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
    /// Pages created by the agent, in creation order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pages: Vec<PageRecord>,
    /// Layout of the top level of a page (elements without a parent), when known.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub page_layouts: BTreeMap<String, ContainerLayout>,
}

impl ProjectSnapshot {
    pub fn empty() -> Self {
        Self {
            schema_version: MODEL_SCHEMA_VERSION,
            revision: 0,
            elements: Vec::new(),
            owned: Vec::new(),
            id_counters: BTreeMap::new(),
            pages: Vec::new(),
            page_layouts: BTreeMap::new(),
        }
    }

    /// Known layout of the container holding children of `parent` (the page
    /// itself when `parent` is `None`).
    pub fn container_layout(
        &self,
        parent: Option<&ElementId>,
        page: &str,
    ) -> Option<ContainerLayout> {
        match parent {
            Some(id) => self.element(id).and_then(|e| e.layout),
            None => self.page_layouts.get(page).copied(),
        }
    }

    pub fn page(&self, id: &str) -> Option<&PageRecord> {
        self.pages.iter().find(|p| p.id == id)
    }

    /// True for the implicit home page and every created page.
    pub fn page_exists(&self, id: &str) -> bool {
        id == DEFAULT_PAGE || self.page(id).is_some()
    }

    /// Display label of a page scope.
    pub fn page_label(&self, id: &str) -> Option<&str> {
        if id == DEFAULT_PAGE {
            Some(HOME_LABEL)
        } else {
            self.page(id).map(|p| p.label.as_str())
        }
    }

    /// Route path of a page scope; created pages need a registered route.
    pub fn page_path(&self, id: &str) -> Option<&str> {
        if id == DEFAULT_PAGE {
            Some(HOME_PATH)
        } else {
            self.page(id)
                .filter(|p| p.registered)
                .map(|p| p.path.as_str())
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
    /// Direct page creation: the page and its route, nothing else.
    CreatePage { label: String },
    /// A style change of one resolved element.
    Style {
        target: ElementId,
        change: StyleChange,
    },
    /// Create a page element. The spec's content may still be incomplete; the
    /// planner then asks for it. Hero and footer are ensure-style: the planner
    /// reports no change when the page already has one.
    AddElement { spec: ElementSpec },
    /// Move an existing element relative to an anchor.
    MoveElement {
        target: ElementId,
        position: Position,
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
        /// Literal content of a hero, image, form, footer or button.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        content: Option<ElementContent>,
    },
    InsertElement {
        element: ElementId,
        page: String,
        parent: Option<ElementId>,
        /// Sibling position; `None` appends at the end of the container.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        at: Option<Position>,
    },
    /// Moves an element among its siblings.
    MoveElement {
        element: ElementId,
        position: Position,
    },
    AddNavigationItem {
        navigation: ElementId,
        item: ElementId,
        label: String,
        destination: Destination,
    },
    /// Absolute style results for one element. `scope` is what the edit
    /// affects: `Instance` changes only `target`, never other users of a
    /// shared class; `Definition` is reserved for a future request form.
    SetStyle {
        target: ElementId,
        change: StyleChange,
        scope: StyleScope,
        edits: Vec<StyleEdit>,
    },
    /// Creates page `page` (its ID and route slug) shown as `label`.
    CreatePage { page: String, label: String },
    /// Registers the route of an existing page in the router.
    RegisterRoute { page: String, path: String },
    SetNavigationDestination {
        item: ElementId,
        destination: Destination,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Precondition {
    RevisionIs {
        revision: u64,
    },
    ElementExists {
        id: ElementId,
    },
    ElementAbsent {
        id: ElementId,
    },
    PageExists {
        page: String,
    },
    PageAbsent {
        page: String,
    },
    /// The element's recorded declared value of `property` is `value`
    /// (`None`: undeclared). Guards absolute style results.
    StyleValueIs {
        id: ElementId,
        property: StyleProperty,
        value: Option<String>,
    },
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
    /// Pending questions this plan answers. Applying the plan removes them
    /// from the session atomically with its changes.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub resolves: Vec<String>,
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
    /// Pick which element a style change applies to (option keys are element IDs).
    ChooseStyleTarget { change: StyleChange },
    /// Pick the anchor of a placement (option keys are element IDs).
    ChoosePlacementAnchor {
        action: PlacementAction,
        relation: PositionRelation,
    },
    /// Pick which element to move (option keys are element IDs); the anchor
    /// is resolved afterwards.
    ChooseMoveTarget {
        relation: PositionRelation,
        anchor: ElementRole,
    },
    /// Free text for the next missing content field of `spec`.
    ProvideContent { spec: ElementSpec },
    /// Choose where an unresolved navigation item leads.
    ChooseDestination { item: ElementId },
    /// Pick the existing page or section an item leads to.
    ChooseExisting { item: ElementId },
    /// Provide the external URL an item leads to (free text).
    EnterUrl { item: ElementId },
    /// Provide a name for the new page an item leads to (free text).
    NamePage { item: ElementId },
    /// The new page's path is taken: link the existing page or create the
    /// page at `alternative` instead.
    ResolveCollision {
        item: ElementId,
        existing_page: String,
        alternative: String,
    },
}

impl Continuation {
    /// The navigation item a destination-related continuation is about.
    pub fn item(&self) -> Option<&ElementId> {
        match self {
            Continuation::ChooseDestination { item }
            | Continuation::ChooseExisting { item }
            | Continuation::EnterUrl { item }
            | Continuation::NamePage { item }
            | Continuation::ResolveCollision { item, .. } => Some(item),
            Continuation::Rephrase { .. }
            | Continuation::ChooseNavigation { .. }
            | Continuation::ChooseStyleTarget { .. }
            | Continuation::ChoosePlacementAnchor { .. }
            | Continuation::ChooseMoveTarget { .. }
            | Continuation::ProvideContent { .. } => None,
        }
    }

    /// True if the answer is free text rather than one of the options.
    pub fn takes_text(&self) -> bool {
        matches!(
            self,
            Continuation::EnterUrl { .. }
                | Continuation::NamePage { .. }
                | Continuation::ProvideContent { .. }
        )
    }
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
    /// No element matches the selector (nothing to change).
    TargetNotFound,
    /// The container layout is unknown or does not support the request;
    /// nothing is guessed.
    UnsupportedLayout,
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
    /// The request (typically an answer) no longer fits the project or the
    /// conversation; nothing is changed.
    Conflict {
        reason: String,
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
            resolves: Vec::new(),
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
                    scope: StyleScope::Instance,
                    edits: vec![StyleEdit {
                        property: StyleProperty::BorderRadius,
                        value: "var(--radius-md)".into(),
                    }],
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
