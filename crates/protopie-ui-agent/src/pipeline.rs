//! Pure stage entry points: parse -> resolve -> plan. No IO, no clocks, no randomness.
//!
//! T1 scope: only top-navigation creation and adding an item to a navigation
//! are resolved and planned; every other parsed request is reported as
//! `CapabilityNotImplemented`. Preparing and applying plans belong to T4+.

use crate::contracts::*;
use crate::parser::{self, ParseOutcome, Request};
use crate::project::fingerprint;

fn not_implemented(what: &str) -> Rejection {
    Rejection {
        reason: RejectionReason::CapabilityNotImplemented,
        explanation: format!("{what} is understood but not supported yet"),
        span: None,
    }
}

fn question_id(session: &Session, slug: &str) -> String {
    format!("q{}-{slug}", session.turn)
}

fn destination_question(session: &Session, item: &ElementId, label: &str) -> Question {
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
        Request::CreatePage(_) => ResolveOutcome::Unsupported {
            rejection: not_implemented("Creating a page"),
        },
        Request::Style(_) => ResolveOutcome::Unsupported {
            rejection: not_implemented("Changing styles"),
        },
    }
}

/// Plans a resolved request. Pure; produces absolute results only.
pub fn plan(
    resolved: &ResolvedRequest,
    project: &ProjectSnapshot,
    session: &Session,
) -> PlanOutcome {
    let base = |operations, follow_up, preconditions| {
        let mut plan = Plan {
            id: String::new(),
            schema_version: PLAN_SCHEMA_VERSION,
            policy_version: POLICY_VERSION,
            base_revision: project.revision,
            preconditions,
            operations,
            follow_up,
        };
        // Keep IDs reproducible, while including every semantic part of the
        // plan and the conversation scope. The ledger compares the full plan
        // as well, so even a fingerprint collision cannot authorize a replay.
        let identity = serde_json::to_string(&(&session.conversation_id, &plan))
            .expect("plan identity is serializable");
        plan.id = format!(
            "plan-r{}-t{}-{}",
            project.revision,
            session.turn,
            fingerprint(&identity)
        );
        plan
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
                        },
                        Operation::InsertElement {
                            element: id.clone(),
                            page: DEFAULT_PAGE.into(),
                            parent: None,
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
    fn fixture_parsed_but_unimplemented_requests_are_rejected() {
        for (prompt, what) in [
            ("Add a Contact Us page", "Creating a page"),
            ("Image needs rounded corners", "Changing styles"),
        ] {
            run(json!({
                "name": prompt,
                "prompt": prompt,
                "expect": {"kind":"unsupported","rejection":{
                    "reason":{"kind":"capability_not_implemented"},
                    "explanation": format!("{what} is understood but not supported yet"),
                    "span":null}}
            }));
        }
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
}
