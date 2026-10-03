//! Pure stage entry points: parse -> resolve -> plan. No IO, no clocks, no randomness.
//!
//! T1 scope: only top-navigation creation and adding an item to a navigation
//! are resolved and planned; every other parsed request is reported as
//! `CapabilityNotImplemented`. Preparing and applying plans belong to T4+.

use crate::contracts::*;
use crate::parser::{
    self, ElementRole, PaddingChange, ParseOutcome, Request, RoleSelector,
    SelectorRelation as ParsedRelation, StyleRequest,
};
use crate::project::fingerprint;
use crate::style;

pub(crate) fn question_id(session: &Session, slug: &str) -> String {
    format!("q{}-{slug}", session.turn)
}

pub(crate) fn destination_question(session: &Session, item: &ElementId, label: &str) -> Question {
    let option = |key: &str, text: &str| QuestionOption {
        key: key.into(),
        label: text.into(),
    };
    Question {
        id: question_id(session, "destination"),
        kind: QuestionKind::FollowUp,
        target: Some(item.clone()),
        prompt: format!(
            "Where should \u{201c}{label}\u{201d} lead: a new page, an existing page or section, or an external URL?"
        ),
        options: vec![
            option("new_page", "A new page"),
            option("existing", "An existing page or section"),
            option("external", "An external URL"),
            option("unlinked", "Leave unlinked"),
        ],
        continuation: Continuation::ChooseDestination { item: item.clone() },
    }
}

enum NavChoice<'a> {
    Chosen(ElementId),
    None,
    Ambiguous(Vec<&'a ElementRecord>),
}

/// Navigation that `id` denotes: the navigation itself, or an item's parent.
fn navigation_of<'a>(project: &'a ProjectSnapshot, id: &ElementId) -> Option<&'a ElementRecord> {
    let element = project.element(id)?;
    match element.kind {
        ElementKind::Navigation => Some(element),
        ElementKind::NavigationItem => element
            .parent
            .as_ref()
            .and_then(|p| project.element(p))
            .filter(|p| p.kind == ElementKind::Navigation),
        _ => None,
    }
}

/// Picks the navigation that receives an added item.
///
/// Precedence: (1) an explicit `top nav` reference restricts candidates to top
/// navigations (unknown placement counts as top); it is never overridden by
/// focus, which may only break a tie among those candidates; (2) compatible GUI
/// selection; (3) compatible focus; (4) the unique navigation in scope; else
/// ambiguity. Remembered IDs that no longer resolve to a navigation are ignored.
fn choose_navigation<'a>(
    explicit_top: bool,
    project: &'a ProjectSnapshot,
    session: &Session,
) -> NavChoice<'a> {
    let candidates: Vec<&ElementRecord> = project
        .of_kind(ElementKind::Navigation)
        .filter(|n| session.page.as_ref().map_or(true, |p| &n.page == p))
        .filter(|n| !explicit_top || n.placement != Some(Placement::Side))
        .collect();
    let compatible = |id: &Option<ElementId>| {
        id.as_ref()
            .and_then(|id| navigation_of(project, id))
            .filter(|n| candidates.iter().any(|c| c.id == n.id))
            .map(|n| n.id.clone())
    };
    let remembered = if explicit_top {
        None
    } else {
        compatible(&session.selection)
    }
    .or_else(|| compatible(&session.focus));
    match (remembered, candidates.as_slice()) {
        (Some(id), _) => NavChoice::Chosen(id),
        (None, []) => NavChoice::None,
        (None, [only]) => NavChoice::Chosen(only.id.clone()),
        (None, _) => NavChoice::Ambiguous(candidates),
    }
}

fn role_kind(role: ElementRole) -> ElementKind {
    ElementKind::from(role)
}

fn role_word(role: ElementRole) -> &'static str {
    role_kind(role).word()
}

fn reject(reason: RejectionReason, explanation: String) -> ResolveOutcome {
    ResolveOutcome::Unsupported {
        rejection: Rejection {
            reason,
            explanation,
            span: None,
        },
    }
}

/// Elements of `selector`'s role in scope that satisfy its relation.
///
/// `below` is structural: the element is the first of its role after an
/// anchor among siblings (same page and parent) of a container whose layout is
/// known to be vertical. Source order in an unknown or horizontal container
/// says nothing about geometry and never matches.
pub(crate) fn select_by_role<'a>(
    selector: &RoleSelector,
    project: &'a ProjectSnapshot,
    session: &Session,
) -> Result<Vec<&'a ElementRecord>, ResolveOutcome> {
    let kind = role_kind(selector.role);
    let in_scope = |e: &&ElementRecord| session.page.as_ref().map_or(true, |p| &e.page == p);
    let Some(ParsedRelation::Below { anchor, .. }) = &selector.relation else {
        return Ok(project.of_kind(kind).filter(in_scope).collect());
    };
    let anchors: Vec<&ElementRecord> = select_by_role(anchor, project, session)?;
    let (word, anchor_word) = (role_word(selector.role), role_word(anchor.role));
    if anchors.is_empty() {
        return Err(reject(
            RejectionReason::TargetNotFound,
            format!("There is no {anchor_word} to look below."),
        ));
    }
    let mut matches: Vec<&ElementRecord> = Vec::new();
    let mut unknown_layout = false;
    for a in &anchors {
        if project.container_layout(a.parent.as_ref(), &a.page) != Some(ContainerLayout::Vertical) {
            unknown_layout = true;
            continue;
        }
        let first_after = project
            .elements
            .iter()
            .filter(|e| e.page == a.page && e.parent == a.parent)
            .skip_while(|e| e.id != a.id)
            .skip(1)
            .find(|e| e.kind == kind);
        if let Some(found) = first_after {
            if !matches.iter().any(|m| m.id == found.id) {
                matches.push(found);
            }
        }
    }
    if matches.is_empty() {
        return Err(if unknown_layout {
            reject(
                RejectionReason::UnsupportedLayout,
                format!(
                    "\u{201c}below\u{201d} needs a known vertical layout around the {anchor_word}; \
                     its container's layout is unknown or horizontal, so the position is not guessed."
                ),
            )
        } else {
            reject(
                RejectionReason::TargetNotFound,
                format!("There is no {word} below the {anchor_word}."),
            )
        });
    }
    Ok(matches)
}

fn resolve_style(
    style: &StyleRequest,
    project: &ProjectSnapshot,
    session: &Session,
) -> ResolveOutcome {
    let candidates = match select_by_role(&style.selector, project, session) {
        Ok(c) => c,
        Err(outcome) => return outcome,
    };
    let word = role_word(style.selector.role);
    let chosen = match candidates.as_slice() {
        [] => {
            return reject(
                RejectionReason::TargetNotFound,
                format!("There is no {word} to change."),
            )
        }
        [only] => Some(only.id.clone()),
        many => {
            // An explicit relation already narrowed the candidates; selection
            // and focus may only break a tie among them.
            let among = |id: &Option<ElementId>| {
                id.as_ref()
                    .filter(|id| many.iter().any(|c| &c.id == *id))
                    .cloned()
            };
            among(&session.selection).or_else(|| among(&session.focus))
        }
    };
    match chosen {
        Some(target) => ResolveOutcome::Resolved {
            request: ResolvedRequest::Style {
                target,
                change: style.change,
            },
        },
        None => ResolveOutcome::NeedsClarification {
            questions: vec![Question {
                id: question_id(session, &format!("which-{word}")),
                kind: QuestionKind::BlockingClarification,
                target: None,
                prompt: format!("Which {word} do you mean?"),
                options: candidates
                    .iter()
                    .map(|e| QuestionOption {
                        key: e.id.0.clone(),
                        label: match &e.label {
                            Some(label) => format!("{label} ({})", e.id.0),
                            None => format!("{} on {}", e.id.0, e.page),
                        },
                    })
                    .collect(),
                continuation: Continuation::ChooseStyleTarget {
                    change: style.change,
                },
            }],
        },
    }
}

/// Resolves a parsed request against an explicit project snapshot and session.
pub fn resolve(
    request: &ParsedRequest,
    project: &ProjectSnapshot,
    session: &Session,
) -> ResolveOutcome {
    match request {
        Request::CreateNavigation(_) => {
            let page = session.page.as_deref().unwrap_or(DEFAULT_PAGE);
            let existing = project
                .of_kind(ElementKind::Navigation)
                .filter(|e| e.page == page && e.placement != Some(Placement::Side))
                .min_by(|a, b| {
                    let number = |e: &ElementRecord| {
                        e.id.0
                            .strip_prefix("nav_")
                            .and_then(|s| s.parse::<u64>().ok())
                    };
                    number(a).cmp(&number(b)).then_with(|| a.id.0.cmp(&b.id.0))
                })
                .map(|e| e.id.clone());
            if existing.is_none() && page != DEFAULT_PAGE {
                ResolveOutcome::Unsupported {
                    rejection: Rejection {
                        reason: RejectionReason::CapabilityNotImplemented,
                        explanation: format!(
                            "Creating a top navigation on page {page} is not supported yet"
                        ),
                        span: None,
                    },
                }
            } else {
                ResolveOutcome::Resolved {
                    request: ResolvedRequest::EnsureNavigation { existing },
                }
            }
        }
        Request::AddLabelledItem(item) => {
            let label = item.label.text.clone();
            let ask = |slug: &str, prompt: String, options: Vec<QuestionOption>| {
                ResolveOutcome::NeedsClarification {
                    questions: vec![Question {
                        id: question_id(session, slug),
                        kind: QuestionKind::BlockingClarification,
                        target: None,
                        prompt,
                        options,
                        continuation: Continuation::ChooseNavigation {
                            label: label.clone(),
                        },
                    }],
                }
            };
            match choose_navigation(item.target.is_some(), project, session) {
                NavChoice::Chosen(navigation) => ResolveOutcome::Resolved {
                    request: ResolvedRequest::AddNavigationItem { navigation, label },
                },
                NavChoice::None => ask(
                    "no-navigation",
                    "There is no navigation yet. Create a top navigation first?".into(),
                    Vec::new(),
                ),
                NavChoice::Ambiguous(candidates) => {
                    let options = candidates
                        .iter()
                        .map(|n| QuestionOption {
                            key: n.id.0.clone(),
                            label: n.label.clone().unwrap_or_else(|| n.id.0.clone()),
                        })
                        .collect();
                    ask(
                        "which-navigation",
                        "Which navigation should it be added to?".into(),
                        options,
                    )
                }
            }
        }
        Request::CreatePage(page) => ResolveOutcome::Resolved {
            request: ResolvedRequest::CreatePage {
                label: page.label.text.clone(),
            },
        },
        Request::Style(style) => resolve_style(style, project, session),
        Request::AddElement(add) => crate::place::resolve_add(add, project, session),
        Request::MoveElement(mv) => crate::place::resolve_move(mv, project, session),
    }
}

/// Assembles a plan against `project`/`session` with a reproducible ID that
/// covers every semantic part of the plan and the conversation scope. The
/// ledger compares the full plan as well, so even a fingerprint collision
/// cannot authorize a replay.
pub(crate) fn build_plan(
    project: &ProjectSnapshot,
    session: &Session,
    operations: Vec<Operation>,
    follow_up: Vec<Question>,
    preconditions: Vec<Precondition>,
    resolves: Vec<String>,
) -> Plan {
    let mut plan = Plan {
        id: String::new(),
        schema_version: PLAN_SCHEMA_VERSION,
        policy_version: POLICY_VERSION,
        base_revision: project.revision,
        preconditions,
        operations,
        follow_up,
        resolves,
    };
    let identity = serde_json::to_string(&(&session.conversation_id, &plan))
        .expect("plan identity is serializable");
    plan.id = format!(
        "plan-r{}-t{}-{}",
        project.revision,
        session.turn,
        fingerprint(&identity)
    );
    plan
}

fn style_unsupported(reason: RejectionReason, explanation: String) -> PlanOutcome {
    PlanOutcome::Unsupported {
        rejection: Rejection {
            reason,
            explanation,
            span: None,
        },
    }
}

/// Plans one style change as absolute results. Pure: the current value comes
/// from the model's style binding, never from reading source.
pub(crate) fn plan_style(
    target: &ElementId,
    change: StyleChange,
    project: &ProjectSnapshot,
    session: &Session,
    resolves: Vec<String>,
) -> PlanOutcome {
    let Some(rec) = project.element(target) else {
        return PlanOutcome::Conflict {
            reason: format!("{} no longer exists.", target.0),
        };
    };
    let Some(binding) = rec.style.as_ref() else {
        return style_unsupported(
            RejectionReason::CapabilityNotImplemented,
            format!("{} has no style binding the agent can edit.", target.0),
        );
    };
    let current =
        |property: StyleProperty| binding.values.get(property.css_name()).map(String::as_str);
    let no_change = |reason: String| PlanOutcome::NoChange {
        reason,
        follow_up: Vec::new(),
    };
    let (property, value) = match change {
        StyleChange::RoundedCorners => {
            let now = current(StyleProperty::BorderRadius);
            if !style::radius_is_zero(now) {
                return no_change(format!(
                    "{} already has rounded corners ({}).",
                    target.0,
                    now.unwrap_or_default()
                ));
            }
            (
                StyleProperty::BorderRadius,
                style::DEFAULT_RADIUS.to_string(),
            )
        }
        StyleChange::FullWidth => {
            if current(StyleProperty::Width) == Some(style::FULL_WIDTH) {
                return no_change(format!("{} is already full width.", target.0));
            }
            if project.container_layout(rec.parent.as_ref(), &rec.page)
                != Some(ContainerLayout::Vertical)
            {
                return style_unsupported(
                    RejectionReason::UnsupportedLayout,
                    format!(
                        "Full width needs a known vertical container; the layout around {} is unknown or horizontal.",
                        target.0
                    ),
                );
            }
            (StyleProperty::Width, style::FULL_WIDTH.to_string())
        }
        StyleChange::Padding(direction) => {
            match style::step_padding(current(StyleProperty::Padding), direction) {
                Err(e) => {
                    return style_unsupported(
                        RejectionReason::CapabilityNotImplemented,
                        format!("Cannot change the padding of {}: {e}.", target.0),
                    )
                }
                Ok(None) => {
                    return no_change(format!(
                        "The padding of {} is already at the {} of the spacing scale.",
                        target.0,
                        match direction {
                            PaddingChange::Increase => "largest step",
                            PaddingChange::Decrease => "smallest step",
                        }
                    ))
                }
                Ok(Some(value)) => (StyleProperty::Padding, value),
            }
        }
    };
    let preconditions = vec![
        Precondition::RevisionIs {
            revision: project.revision,
        },
        Precondition::ElementExists { id: target.clone() },
        Precondition::StyleValueIs {
            id: target.clone(),
            property,
            value: current(property).map(str::to_string),
        },
    ];
    PlanOutcome::Ready {
        plan: build_plan(
            project,
            session,
            vec![Operation::SetStyle {
                target: target.clone(),
                change,
                scope: StyleScope::Instance,
                edits: vec![StyleEdit { property, value }],
            }],
            Vec::new(),
            preconditions,
            resolves,
        ),
    }
}

/// Plans a resolved request. Pure; produces absolute results only.
pub fn plan(
    resolved: &ResolvedRequest,
    project: &ProjectSnapshot,
    session: &Session,
) -> PlanOutcome {
    let base = |operations, follow_up, preconditions| {
        build_plan(
            project,
            session,
            operations,
            follow_up,
            preconditions,
            Vec::new(),
        )
    };
    match resolved {
        ResolvedRequest::EnsureNavigation { existing: Some(id) } => PlanOutcome::NoChange {
            reason: format!("A top navigation ({}) already exists.", id.0),
            follow_up: Vec::new(),
        },
        ResolvedRequest::EnsureNavigation { existing: None } => {
            let id = project.next_id("nav");
            PlanOutcome::Ready {
                plan: base(
                    vec![
                        Operation::CreateComponent {
                            id: id.clone(),
                            kind: ElementKind::Navigation,
                            name: "TopNav".into(),
                            content: None,
                        },
                        Operation::InsertElement {
                            element: id.clone(),
                            page: DEFAULT_PAGE.into(),
                            parent: None,
                            at: None,
                        },
                    ],
                    Vec::new(),
                    vec![
                        Precondition::RevisionIs {
                            revision: project.revision,
                        },
                        Precondition::ElementAbsent { id },
                    ],
                ),
            }
        }
        ResolvedRequest::CreatePage { label } => {
            if label.trim().is_empty() || label.chars().count() > crate::emit::MAX_LABEL_CHARS {
                return PlanOutcome::Unsupported {
                    rejection: Rejection {
                        reason: RejectionReason::CapabilityNotImplemented,
                        explanation: format!(
                            "A page name must have 1 to {} characters.",
                            crate::emit::MAX_LABEL_CHARS
                        ),
                        span: None,
                    },
                };
            }
            let Some(slug) = crate::emit::derive_slug(label) else {
                return PlanOutcome::Unsupported {
                    rejection: Rejection {
                        reason: RejectionReason::CapabilityNotImplemented,
                        explanation: format!(
                            "Cannot derive a page path from \u{201c}{label}\u{201d}: use a name with letters or digits (at most {} characters in the path).",
                            crate::emit::MAX_SLUG_CHARS
                        ),
                        span: None,
                    },
                };
            };
            if let Some(existing) = crate::answer::conflicting_page(project, &slug) {
                return PlanOutcome::NoChange {
                    reason: format!(
                        "A page at {} already exists ({}).",
                        existing.path, existing.label
                    ),
                    follow_up: Vec::new(),
                };
            }
            PlanOutcome::Ready {
                plan: base(
                    crate::answer::create_page_ops(&slug, label),
                    Vec::new(),
                    vec![
                        Precondition::RevisionIs {
                            revision: project.revision,
                        },
                        Precondition::PageAbsent { page: slug },
                    ],
                ),
            }
        }
        ResolvedRequest::Style { target, change } => {
            plan_style(target, *change, project, session, Vec::new())
        }
        ResolvedRequest::AddElement { spec } => {
            crate::place::plan_add_element(spec, project, session, Vec::new())
        }
        ResolvedRequest::MoveElement { target, position } => {
            crate::place::plan_move(target, position, project, session, Vec::new())
        }
        ResolvedRequest::AddNavigationItem { navigation, label } => {
            let item = project.next_id("item");
            PlanOutcome::Ready {
                plan: base(
                    vec![Operation::AddNavigationItem {
                        navigation: navigation.clone(),
                        item: item.clone(),
                        label: label.clone(),
                        destination: Destination::Unresolved,
                    }],
                    vec![destination_question(session, &item, label)],
                    vec![
                        Precondition::RevisionIs {
                            revision: project.revision,
                        },
                        Precondition::ElementExists {
                            id: navigation.clone(),
                        },
                    ],
                ),
            }
        }
    }
}

/// Parses, resolves and plans one prompt. Never reads or writes anything, so it
/// doubles as the dry-run path.
pub fn plan_prompt(prompt: &str, project: &ProjectSnapshot, session: &Session) -> PlanOutcome {
    match parser::parse(prompt) {
        ParseOutcome::Unsupported(u) => PlanOutcome::Unsupported {
            rejection: Rejection {
                reason: RejectionReason::Parse { reason: u.reason },
                explanation: format!("Could not understand the request ({:?}).", u.reason),
                span: Some(u.span),
            },
        },
        ParseOutcome::NeedsClarification(c) => PlanOutcome::NeedsClarification {
            questions: vec![Question {
                id: question_id(session, "boundary"),
                kind: QuestionKind::BlockingClarification,
                target: None,
                prompt: format!("Did you mean {}?", c.quoted_form_suggestion),
                options: vec![QuestionOption {
                    key: "rephrase".into(),
                    label: c.quoted_form_suggestion.clone(),
                }],
                continuation: Continuation::Rephrase {
                    suggestion: c.quoted_form_suggestion,
                },
            }],
        },
        ParseOutcome::Parsed(request) => match resolve(&request, project, session) {
            ResolveOutcome::Resolved { request } => plan(&request, project, session),
            // Placement questions are kept in the conversation so a chat
            // reply can answer them; other clarifications are not persisted.
            ResolveOutcome::NeedsClarification { mut questions }
                if questions.len() == 1 && crate::place::is_placement_question(&questions[0]) =>
            {
                crate::place::question_plan(project, session, questions.remove(0), Vec::new(), Vec::new())
            }
            ResolveOutcome::NeedsClarification { questions } => {
                PlanOutcome::NeedsClarification { questions }
            }
            ResolveOutcome::Unsupported { rejection } => PlanOutcome::Unsupported { rejection },
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};

    /// JSON fixture: prompt + project + session -> exact serialized outcome.
    fn run(fixture: Value) {
        let name = fixture["name"].as_str().unwrap().to_string();
        let project: ProjectSnapshot = fixture
            .get("project")
            .map(|v| serde_json::from_value(v.clone()).unwrap())
            .unwrap_or_default();
        let session: Session = fixture
            .get("session")
            .map(|v| serde_json::from_value(v.clone()).unwrap())
            .unwrap_or_default();
        let prompt = fixture["prompt"].as_str().unwrap();
        let actual = serde_json::to_value(plan_prompt(prompt, &project, &session)).unwrap();
        assert_eq!(actual, fixture["expect"], "fixture {name}");
        // Purity: same inputs, same outcome.
        assert_eq!(
            actual,
            serde_json::to_value(plan_prompt(prompt, &project, &session)).unwrap(),
            "fixture {name} is not deterministic"
        );
    }

    fn nav_project() -> Value {
        json!({"schema_version":1,"revision":4,"elements":[
            {"id":"nav_1","kind":"navigation","label":"Top","page":"home"}]})
    }

    #[test]
    fn fixture_unsupported_prompt_has_structured_rejection() {
        run(json!({
            "name": "unrelated text",
            "prompt": "Tell me a joke",
            "expect": {"kind":"unsupported","rejection":{
                "reason":{"kind":"parse","reason":"UnrecognizedInput"},
                "explanation":"Could not understand the request (UnrecognizedInput).",
                "span":{"start":0,"end":14}}}
        }));
    }

    #[test]
    fn fixture_add_top_nav_plans_creation_without_unauthorized_operations() {
        run(json!({
            "name": "add top nav",
            "prompt": "Add top nav",
            "expect": {"kind":"ready","plan":{
                "id":"plan-r0-t0-387b60538f684fbd","schema_version":1,"policy_version":1,"base_revision":0,
                "preconditions":[{"kind":"revision_is","revision":0},
                                 {"kind":"element_absent","id":"nav_1"}],
                "operations":[
                    {"op":"create_component","id":"nav_1","kind":"navigation","name":"TopNav"},
                    {"op":"insert_element","element":"nav_1","page":"home","parent":null}],
                "follow_up":[]}}
        }));
    }

    #[test]
    fn fixture_add_top_nav_is_ensure_style() {
        run(json!({
            "name": "ensure nav",
            "prompt": "Add top nav",
            "project": nav_project(),
            "expect": {"kind":"no_change",
                "reason":"A top navigation (nav_1) already exists.","follow_up":[]}
        }));
    }

    #[test]
    fn fixture_add_label_uses_focus_and_leaves_destination_unresolved() {
        run(json!({
            "name": "add label to focused nav",
            "prompt": "Add Contact Us",
            "project": nav_project(),
            "session": {"schema_version":1,"conversation_id":"default","turn":2,"focus":"nav_1"},
            "expect": {"kind":"ready","plan":{
                "id":"plan-r4-t2-c60727d46ecf9a33","schema_version":1,"policy_version":1,"base_revision":4,
                "preconditions":[{"kind":"revision_is","revision":4},
                                 {"kind":"element_exists","id":"nav_1"}],
                "operations":[{"op":"add_navigation_item","navigation":"nav_1",
                    "item":"item_1","label":"Contact Us","destination":{"kind":"unresolved"}}],
                "follow_up":[{"id":"q2-destination","kind":"follow_up","target":"item_1",
                    "prompt":"Where should \u{201c}Contact Us\u{201d} lead: a new page, an existing page or section, or an external URL?",
                    "options":[
                        {"key":"new_page","label":"A new page"},
                        {"key":"existing","label":"An existing page or section"},
                        {"key":"external","label":"An external URL"},
                        {"key":"unlinked","label":"Leave unlinked"}],
                    "continuation":{"kind":"choose_destination","item":"item_1"}}]}}
        }));
    }

    #[test]
    fn fixture_add_label_without_any_navigation_blocks() {
        run(json!({
            "name": "no navigation",
            "prompt": "Add Contact Us",
            "expect": {"kind":"needs_clarification","questions":[{
                "id":"q0-no-navigation","kind":"blocking_clarification","target":null,
                "prompt":"There is no navigation yet. Create a top navigation first?",
                "options":[],
                "continuation":{"kind":"choose_navigation","label":"Contact Us"}}]}
        }));
    }

    #[test]
    fn fixture_stale_focus_is_not_trusted() {
        // Focus points at an element that is not a navigation in the project.
        run(json!({
            "name": "stale focus",
            "prompt": "Add Contact Us",
            "session": {"schema_version":1,"conversation_id":"default","turn":0,"focus":"nav_9"},
            "expect": {"kind":"needs_clarification","questions":[{
                "id":"q0-no-navigation","kind":"blocking_clarification","target":null,
                "prompt":"There is no navigation yet. Create a top navigation first?",
                "options":[],
                "continuation":{"kind":"choose_navigation","label":"Contact Us"}}]}
        }));
    }

    #[test]
    fn fixture_ambiguous_navigation_asks_which() {
        run(json!({
            "name": "two navs",
            "prompt": "Add Contact Us to top nav",
            "project": {"schema_version":1,"revision":1,"elements":[
                {"id":"nav_1","kind":"navigation","label":"Top","page":"home"},
                {"id":"nav_2","kind":"navigation","page":"about"}]},
            "expect": {"kind":"needs_clarification","questions":[{
                "id":"q0-which-navigation","kind":"blocking_clarification","target":null,
                "prompt":"Which navigation should it be added to?",
                "options":[{"key":"nav_1","label":"Top"},{"key":"nav_2","label":"nav_2"}],
                "continuation":{"kind":"choose_navigation","label":"Contact Us"}}]}
        }));
    }

    #[test]
    fn fixture_parser_clarification_becomes_blocking_question() {
        run(json!({
            "name": "ambiguous boundary",
            "prompt": "Add navigation to top nav",
            "expect": {"kind":"needs_clarification","questions":[{
                "id":"q0-boundary","kind":"blocking_clarification","target":null,
                "prompt":"Did you mean Add \"navigation\" to top nav?",
                "options":[{"key":"rephrase","label":"Add \"navigation\" to top nav"}],
                "continuation":{"kind":"rephrase","suggestion":"Add \"navigation\" to top nav"}}]}
        }));
    }

    #[test]
    fn style_request_without_a_matching_element_is_rejected_not_invented() {
        run(json!({
            "name": "no image",
            "prompt": "Image needs rounded corners",
            "expect": {"kind":"unsupported","rejection":{
                "reason":{"kind":"target_not_found"},
                "explanation": "There is no image to change.",
                "span":null}}
        }));
    }

    fn style_project() -> Value {
        json!({"schema_version":1,"revision":3,
            "page_layouts":{"home":"vertical"},
            "elements":[
            {"id":"hero_1","kind":"hero","page":"home"},
            {"id":"form_1","kind":"form","page":"home",
             "style":{"file":"s.css","class":"a","scope":"instance","values":{"padding":"2rem 1rem"}}},
            {"id":"img_1","kind":"image","page":"home","label":"Logo",
             "style":{"file":"s.css","class":"b","scope":"definition"}},
            {"id":"img_2","kind":"image","page":"home",
             "style":{"file":"s.css","class":"b","scope":"definition"}},
            {"id":"hero_2","kind":"hero","page":"home"},
            {"id":"form_2","kind":"form","page":"home",
             "style":{"file":"s.css","class":"c","scope":"instance"}}]})
    }

    #[test]
    fn fixture_ambiguous_images_ask_which_image() {
        run(json!({
            "name": "two images",
            "prompt": "Image needs rounded corners",
            "project": style_project(),
            "expect": {"kind":"needs_clarification","questions":[{
                "id":"q0-which-image","kind":"blocking_clarification","target":null,
                "prompt":"Which image do you mean?",
                "options":[{"key":"img_1","label":"Logo (img_1)"},
                           {"key":"img_2","label":"img_2 on home"}],
                "continuation":{"kind":"choose_style_target","change":"RoundedCorners"}}]}
        }));
    }

    #[test]
    fn style_plans_carry_absolute_values_and_guards() {
        let project: ProjectSnapshot = serde_json::from_value(style_project()).unwrap();
        // Two heroes each have a form below them: ambiguity, not a guess.
        let out = serde_json::to_value(plan_prompt(
            "Give the form below hero more padding",
            &project,
            &Session::default(),
        ))
        .unwrap();
        assert_eq!(out["kind"], "needs_clarification");
        assert_eq!(out["questions"][0]["id"], "q0-which-form");
        let keys: Vec<_> = out["questions"][0]["options"]
            .as_array()
            .unwrap()
            .iter()
            .map(|o| o["key"].as_str().unwrap())
            .collect();
        assert_eq!(keys, ["form_1", "form_2"]);
        // Compatible focus among the relation's matches breaks the tie.
        let session = Session {
            focus: Some(ElementId::new("form_1")),
            ..Session::default()
        };
        let PlanOutcome::Ready { plan } =
            plan_prompt("Give the form below hero more padding", &project, &session)
        else {
            panic!()
        };
        assert_eq!(
            serde_json::to_value(&plan.operations).unwrap(),
            json!([{"op":"set_style","target":"form_1","change":{"Padding":"Increase"},
                "scope":"instance",
                "edits":[{"property":"padding","value":"var(--space-7) var(--space-5)"}]}])
        );
        assert_eq!(
            serde_json::to_value(&plan.preconditions).unwrap(),
            json!([{"kind":"revision_is","revision":3},
                   {"kind":"element_exists","id":"form_1"},
                   {"kind":"style_value_is","id":"form_1","property":"padding","value":"2rem 1rem"}])
        );
        assert!(plan.follow_up.is_empty());
    }

    #[test]
    fn selection_picks_among_images_and_a_second_hero_anchors_the_next_form() {
        let project: ProjectSnapshot = serde_json::from_value(style_project()).unwrap();
        let mut session = Session {
            selection: Some(ElementId::new("img_2")),
            ..Session::default()
        };
        let PlanOutcome::Ready { plan } =
            plan_prompt("Image needs rounded corners", &project, &session)
        else {
            panic!()
        };
        assert!(matches!(&plan.operations[0],
            Operation::SetStyle { target, .. } if target.0 == "img_2"));
        // Only forms in the current page scope are candidates.
        session.page = Some("about".into());
        session.selection = None;
        assert!(matches!(
            plan_prompt("Image needs rounded corners", &project, &session),
            PlanOutcome::Unsupported { .. }
        ));
    }

    fn add_target(prompt: &str, project: Value, session: Value) -> Value {
        let project: ProjectSnapshot = serde_json::from_value(project).unwrap();
        let session: Session = serde_json::from_value(session).unwrap();
        serde_json::to_value(plan_prompt(prompt, &project, &session)).unwrap()
    }

    fn two_navs(first: &str, second: &str) -> Value {
        json!({"schema_version":1,"revision":1,"elements":[
            {"id":"nav_1","kind":"navigation","page":"home","placement":first},
            {"id":"nav_2","kind":"navigation","page":"home","placement":second},
            {"id":"item_1","kind":"navigation_item","page":"home","parent":"nav_2"}]})
    }

    fn session(extra: Value) -> Value {
        let mut base = json!({"schema_version":1,"conversation_id":"default","turn":0});
        base.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        base
    }

    fn target_of(outcome: &Value) -> Option<&str> {
        outcome["plan"]["operations"][0]["navigation"].as_str()
    }

    #[test]
    fn explicit_top_nav_binds_to_a_top_navigation_even_when_focus_differs() {
        let out = add_target(
            "Add Contact Us to top nav",
            two_navs("side", "top"),
            session(json!({"focus":"nav_1"})),
        );
        assert_eq!(target_of(&out), Some("nav_2"));
        // With a single top navigation, no focus is needed.
        let out = add_target(
            "Add Contact Us to top nav",
            two_navs("side", "top"),
            session(json!({})),
        );
        assert_eq!(target_of(&out), Some("nav_2"));
        // Two top navigations: explicit reference stays ambiguous without focus...
        let out = add_target(
            "Add Contact Us to top nav",
            two_navs("top", "top"),
            session(json!({})),
        );
        assert_eq!(out["kind"], "needs_clarification");
        // ...and focus may break the tie, selection may not override explicit.
        let out = add_target(
            "Add Contact Us to top nav",
            two_navs("top", "top"),
            session(json!({"focus":"nav_1","selection":"nav_2"})),
        );
        assert_eq!(target_of(&out), Some("nav_1"));
        // Only side navigations: an explicit top nav does not exist.
        let out = add_target(
            "Add Contact Us to top nav",
            two_navs("side", "side"),
            session(json!({})),
        );
        assert_eq!(out["kind"], "needs_clarification");
        assert_eq!(out["questions"][0]["id"], "q0-no-navigation");
    }

    #[test]
    fn implicit_resolution_prefers_selection_then_focus_then_uniqueness() {
        let both = || two_navs("top", "top");
        let out = add_target(
            "Add Contact Us",
            both(),
            session(json!({"focus":"nav_1","selection":"nav_2"})),
        );
        assert_eq!(target_of(&out), Some("nav_2"));
        // A selected navigation item resolves to its parent navigation.
        let out = add_target(
            "Add Contact Us",
            both(),
            session(json!({"selection":"item_1"})),
        );
        assert_eq!(target_of(&out), Some("nav_2"));
        // An incompatible selection (hero) is ignored; focus applies.
        let mut project = both();
        project["elements"]
            .as_array_mut()
            .unwrap()
            .push(json!({"id":"hero_1","kind":"hero","page":"home"}));
        let out = add_target(
            "Add Contact Us",
            project.clone(),
            session(json!({"selection":"hero_1","focus":"nav_1"})),
        );
        assert_eq!(target_of(&out), Some("nav_1"));
        // Incompatible focus (a hero) does not pick a navigation: ambiguity.
        let out = add_target(
            "Add Contact Us",
            project,
            session(json!({"focus":"hero_1"})),
        );
        assert_eq!(out["questions"][0]["id"], "q0-which-navigation");
        // Removed focus target is stale, not trusted.
        let out = add_target("Add Contact Us", both(), session(json!({"focus":"nav_9"})));
        assert_eq!(out["questions"][0]["id"], "q0-which-navigation");
    }

    #[test]
    fn page_scope_limits_candidates() {
        let project = json!({"schema_version":1,"revision":1,"elements":[
            {"id":"nav_1","kind":"navigation","page":"home"},
            {"id":"nav_2","kind":"navigation","page":"about"}]});
        let out = add_target(
            "Add Contact Us",
            project.clone(),
            session(json!({"page":"about"})),
        );
        assert_eq!(target_of(&out), Some("nav_2"));
        // Focus on another page's navigation is not compatible with this scope.
        let out = add_target(
            "Add Contact Us",
            project,
            session(json!({"page":"about","focus":"nav_1"})),
        );
        assert_eq!(target_of(&out), Some("nav_2"));
    }

    #[test]
    fn stages_compose_individually() {
        let project = ProjectSnapshot::empty();
        let session = Session::default();
        let ParseOutcome::Parsed(request) = parser::parse("Add top nav") else {
            panic!("expected parsed request");
        };
        let resolved = resolve(&request, &project, &session);
        assert_eq!(
            resolved,
            ResolveOutcome::Resolved {
                request: ResolvedRequest::EnsureNavigation { existing: None }
            }
        );
        let ResolveOutcome::Resolved { request } = resolved else {
            unreachable!()
        };
        assert!(matches!(
            plan(&request, &project, &session),
            PlanOutcome::Ready { .. }
        ));
    }

    #[test]
    fn ensure_top_navigation_ignores_side_and_other_pages_and_uses_numeric_order() {
        let project = json!({"schema_version":1,"revision":1,"elements":[
            {"id":"nav_1","kind":"navigation","page":"home","placement":"side"},
            {"id":"nav_10","kind":"navigation","page":"home","placement":"top"},
            {"id":"nav_2","kind":"navigation","page":"home","placement":"top"},
            {"id":"nav_3","kind":"navigation","page":"about","placement":"top"}
        ]});
        let project: ProjectSnapshot = serde_json::from_value(project).unwrap();
        let ParseOutcome::Parsed(request) = parser::parse("Add top nav") else {
            panic!("expected parsed request");
        };
        assert_eq!(
            resolve(&request, &project, &Session::default()),
            ResolveOutcome::Resolved {
                request: ResolvedRequest::EnsureNavigation {
                    existing: Some(ElementId::new("nav_2"))
                }
            }
        );
        let about = session(json!({"page":"about"}));
        let about: Session = serde_json::from_value(about).unwrap();
        assert_eq!(
            resolve(&request, &project, &about),
            ResolveOutcome::Resolved {
                request: ResolvedRequest::EnsureNavigation {
                    existing: Some(ElementId::new("nav_3"))
                }
            }
        );
        let mut side_only = project;
        side_only
            .elements
            .retain(|e| e.placement == Some(Placement::Side));
        assert_eq!(
            resolve(&request, &side_only, &Session::default()),
            ResolveOutcome::Resolved {
                request: ResolvedRequest::EnsureNavigation { existing: None }
            }
        );
        assert!(matches!(
            resolve(&request, &side_only, &about),
            ResolveOutcome::Unsupported { .. }
        ));
    }

    #[test]
    fn ids_advance_past_existing_elements() {
        let mut project = ProjectSnapshot::empty();
        project.elements.push(ElementRecord::new(
            ElementId::new("item_7"),
            ElementKind::NavigationItem,
            DEFAULT_PAGE,
        ));
        assert_eq!(project.next_id("item"), ElementId::new("item_8"));
        assert_eq!(project.next_id("nav"), ElementId::new("nav_1"));
    }

    // ------------------------------------------------------------ T8 fixtures

    fn flow_project() -> Value {
        json!({"schema_version":1,"revision":5,
            "page_layouts":{"home":"vertical"},
            "elements":[
            {"id":"hero_1","kind":"hero","page":"home"},
            {"id":"img_1","kind":"image","page":"home"},
            {"id":"img_2","kind":"image","page":"home"},
            {"id":"footer_1","kind":"footer","page":"home"}]})
    }

    #[test]
    fn fixture_add_form_below_hero_asks_for_fields_and_persists_the_question() {
        run(json!({
            "name": "form below hero",
            "prompt": "add a form below hero",
            "project": flow_project(),
            "expect": {"kind":"ready","plan":{
                "base_revision":5,"id":"plan-r5-t0-4cb2fdf7ae6e449d","policy_version":1,"schema_version":1,
                "preconditions":[{"kind":"revision_is","revision":5},{"kind":"element_exists","id":"hero_1"}],
                "operations":[],
                "follow_up":[{
                    "id":"q0-content-form-fields","kind":"blocking_clarification","target":null,
                    "prompt":"Which fields should the form have? List their names separated by commas, for example Name, Email.",
                    "options":[],
                    "continuation":{"kind":"provide_content","spec":{
                        "kind":"form","position":{"relation":"Below","anchor":"hero_1"},"content":{}}}}]}}
        }));
    }

    #[test]
    fn fixture_button_with_label_plans_component_insertion_and_destination_question() {
        run(json!({
            "name": "button above footer",
            "prompt": "Add a button called Go above the footer",
            "project": flow_project(),
            "expect": {"kind":"ready","plan":{
                "base_revision":5,"id":"plan-r5-t0-7e630f14d6d0c04f","policy_version":1,"schema_version":1,
                "preconditions":[{"kind":"revision_is","revision":5},
                                 {"kind":"element_absent","id":"button_1"},
                                 {"kind":"element_exists","id":"footer_1"}],
                "operations":[
                    {"op":"create_component","id":"button_1","kind":"button","name":"Button1","content":{"text":"Go"}},
                    {"op":"insert_element","element":"button_1","page":"home","parent":null,
                     "at":{"relation":"Above","anchor":"footer_1"}}],
                "follow_up":[{
                    "id":"q0-destination","kind":"follow_up","target":"button_1",
                    "prompt":"Where should \u{201c}Go\u{201d} lead: a new page, an existing page or section, or an external URL?",
                    "options":[{"key":"new_page","label":"A new page"},
                               {"key":"existing","label":"An existing page or section"},
                               {"key":"external","label":"An external URL"},
                               {"key":"unlinked","label":"Leave unlinked"}],
                    "continuation":{"kind":"choose_destination","item":"button_1"}}]}}
        }));
    }

    #[test]
    fn fixture_move_is_a_positioning_plan_with_no_style_or_creation_operations() {
        run(json!({
            "name": "move footer above hero",
            "prompt": "Move the footer above the hero",
            "project": flow_project(),
            "expect": {"kind":"ready","plan":{
                "base_revision":5,"id":"plan-r5-t0-41a6dddc150a7c68","policy_version":1,"schema_version":1,
                "preconditions":[{"kind":"revision_is","revision":5},
                                 {"kind":"element_exists","id":"footer_1"},
                                 {"kind":"element_exists","id":"hero_1"}],
                "operations":[{"op":"move_element","element":"footer_1",
                               "position":{"relation":"Above","anchor":"hero_1"}}],
                "follow_up":[]}}
        }));
    }

    #[test]
    fn fixture_ambiguous_anchor_is_a_persisted_question_with_no_operations() {
        run(json!({
            "name": "which image",
            "prompt": "Add a button called Go below the image",
            "project": flow_project(),
            "expect": {"kind":"ready","plan":{
                "base_revision":5,"id":"plan-r5-t0-d9a331b770187a39","policy_version":1,"schema_version":1,
                "preconditions":[{"kind":"revision_is","revision":5}],
                "operations":[],
                "follow_up":[{
                    "id":"q0-which-anchor-image","kind":"blocking_clarification","target":null,
                    "prompt":"Which image do you mean?",
                    "options":[{"key":"img_1","label":"img_1 on home"},{"key":"img_2","label":"img_2 on home"}],
                    "continuation":{"kind":"choose_placement_anchor","relation":"Below",
                        "action":{"kind":"add","spec":{"kind":"button","content":{"text":"Go"}}}}}]}}
        }));
    }

    #[test]
    fn fixture_selection_breaks_an_anchor_tie_and_footer_is_ensure_style() {
        let project: ProjectSnapshot = serde_json::from_value(flow_project()).unwrap();
        let mut session = Session::default();
        session.selection = Some(ElementId::new("img_2"));
        let PlanOutcome::Ready { plan } =
            plan_prompt("Add a button called Go below the image", &project, &session)
        else {
            panic!()
        };
        assert!(matches!(
            &plan.operations[1],
            Operation::InsertElement { at: Some(Position { anchor, .. }), .. } if anchor.0 == "img_2"
        ));
        run(json!({
            "name": "footer exists",
            "prompt": "Add a footer",
            "project": flow_project(),
            "expect": {"kind":"no_change","follow_up":[],
                "reason":"The home page already has a footer (footer_1); use a move or style request to change it."}
        }));
        run(json!({
            "name": "target missing",
            "prompt": "Move the form below hero",
            "project": flow_project(),
            "expect": {"kind":"unsupported","rejection":{
                "reason":{"kind":"target_not_found"},
                "explanation":"There is no form to move.","span":null}}
        }));
    }
}
