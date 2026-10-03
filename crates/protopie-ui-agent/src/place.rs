//! Page elements and positioning (Epic 001, T8): resolving and planning
//! `add a <role>` and `move the <role> <relation> <anchor>`. Pure.
//!
//! # Semantics
//!
//! * A *selector relation* (`the form below hero`) picks an existing element.
//!   A *position clause* (`add a form below hero`, `move the form above the
//!   image`) says where an element goes. They are separate parser types and
//!   never share code paths.
//! * Only the home page's top-level flow is supported: hero, image, form,
//!   footer and button elements stack in the order of the model. `above` and
//!   `below` need a known vertical container around the anchor; `left of` and
//!   `right of` need a known horizontal one. Anything else is reported as an
//!   unsupported layout and nothing is guessed. The generated reference
//!   project only has vertical layouts, so horizontal placement is planned but
//!   the emitter reports it unsupported if a model claims one.
//! * Without a position an element is appended to the end of the page.
//! * A hero and a footer are ensure-style per page. Images, forms and buttons
//!   are additive.
//! * Content the agent must not invent (a headline, an image address and
//!   alternative text, form fields and a submit label, footer text, a button
//!   label) is asked for with persisted blocking questions before anything is
//!   generated. A button's destination is a follow-up question, exactly like
//!   a navigation item's: until answered it is a disabled button.

use crate::contracts::*;
use crate::emit;
use crate::parser::{ElementAddition, ElementMove, RoleSelector};
use crate::pipeline::{build_plan, destination_question, question_id, select_by_role};

fn rejection(reason: RejectionReason, explanation: String) -> Rejection {
    Rejection {
        reason,
        explanation,
        span: None,
    }
}

fn stop(reason: RejectionReason, explanation: String) -> ResolveOutcome {
    ResolveOutcome::Unsupported {
        rejection: rejection(reason, explanation),
    }
}

pub(crate) fn relation_text(relation: PositionRelation) -> &'static str {
    match relation {
        PositionRelation::Above => "above",
        PositionRelation::Below => "below",
        PositionRelation::LeftOf => "left of",
        PositionRelation::RightOf => "right of",
    }
}

/// Elements stacked in the top-level flow of a page, in sibling order.
pub(crate) fn flow_siblings<'a>(project: &'a ProjectSnapshot, page: &str) -> Vec<&'a ElementRecord> {
    project
        .elements
        .iter()
        .filter(|e| e.page == page && e.parent.is_none() && e.kind.is_flow())
        .collect()
}

/// Checks that `relation` to `anchor` has known layout semantics.
pub(crate) fn check_position(
    project: &ProjectSnapshot,
    relation: PositionRelation,
    anchor: &ElementRecord,
) -> Result<(), Rejection> {
    if !anchor.kind.is_flow() || anchor.page != DEFAULT_PAGE || anchor.parent.is_some() {
        return Err(rejection(
            RejectionReason::CapabilityNotImplemented,
            format!(
                "Placing elements relative to {} is not supported yet; only the top level of the home page is.",
                anchor.id.0
            ),
        ));
    }
    let (needed, direction) = match relation {
        PositionRelation::Above | PositionRelation::Below => (ContainerLayout::Vertical, "vertical"),
        PositionRelation::LeftOf | PositionRelation::RightOf => {
            (ContainerLayout::Horizontal, "horizontal")
        }
    };
    let known = project.container_layout(anchor.parent.as_ref(), &anchor.page);
    if known == Some(needed) {
        return Ok(());
    }
    let state = match known {
        Some(ContainerLayout::Vertical) => "vertical",
        Some(ContainerLayout::Horizontal) => "horizontal",
        None => "unknown",
    };
    Err(rejection(
        RejectionReason::UnsupportedLayout,
        format!(
            "\u{201c}{}\u{201d} needs a known {direction} layout around {}, but its container's layout is {state}, so the position is not guessed.",
            relation_text(relation),
            anchor.id.0
        ),
    ))
}

/// Selection or focus, when it names one of `candidates`.
fn pick(candidates: &[&ElementRecord], session: &Session) -> Option<ElementId> {
    let among = |id: &Option<ElementId>| {
        id.as_ref()
            .filter(|id| candidates.iter().any(|c| &c.id == *id))
            .cloned()
    };
    among(&session.selection).or_else(|| among(&session.focus))
}

fn choose_question(
    session: &Session,
    word: &str,
    candidates: &[&ElementRecord],
    continuation: Continuation,
    purpose: &str,
) -> ResolveOutcome {
    ResolveOutcome::NeedsClarification {
        questions: vec![Question {
            id: question_id(session, &format!("which-{purpose}-{word}")),
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
            continuation,
        }],
    }
}

fn check_scope(session: &Session) -> Result<(), ResolveOutcome> {
    match session.page.as_deref() {
        None | Some(DEFAULT_PAGE) => Ok(()),
        Some(page) => Err(stop(
            RejectionReason::CapabilityNotImplemented,
            format!("Placing elements on page {page} is not supported yet; only the home page is."),
        )),
    }
}

/// Resolves the anchor of a placement: the one element of `role` in scope
/// (selection or focus breaks a tie), or a question.
fn resolve_anchor(
    role: ElementRole,
    relation: PositionRelation,
    action: &PlacementAction,
    exclude: Option<&ElementId>,
    project: &ProjectSnapshot,
    session: &Session,
) -> Result<Position, ResolveOutcome> {
    let selector = RoleSelector {
        role,
        relation: None,
        span: crate::parser::Span { start: 0, end: 0 },
    };
    let mut candidates = select_by_role(&selector, project, session)?;
    candidates.retain(|e| Some(&e.id) != exclude);
    let word = ElementKind::from(role).word();
    let anchor = match candidates.as_slice() {
        [] => {
            return Err(stop(
                RejectionReason::TargetNotFound,
                format!("There is no {word} to place it {}.", relation_text(relation)),
            ))
        }
        [only] => only.id.clone(),
        many => match pick(many, session) {
            Some(id) => id,
            None => {
                return Err(choose_question(
                    session,
                    word,
                    many,
                    Continuation::ChoosePlacementAnchor {
                        action: action.clone(),
                        relation,
                    },
                    "anchor",
                ))
            }
        },
    };
    let rec = project.element(&anchor).expect("candidate is in the model");
    check_position(project, relation, rec).map_err(|r| ResolveOutcome::Unsupported { rejection: r })?;
    Ok(Position { relation, anchor })
}

/// Resolves `add a <role> ...`.
pub(crate) fn resolve_add(
    add: &ElementAddition,
    project: &ProjectSnapshot,
    session: &Session,
) -> ResolveOutcome {
    if let Err(outcome) = check_scope(session) {
        return outcome;
    }
    let kind = ElementKind::from(add.role);
    let content = ElementContent {
        text: add.label.as_ref().map(|l| l.text.clone()),
        ..Default::default()
    };
    let mut spec = ElementSpec {
        kind,
        position: None,
        content,
    };
    // An ensure-style element that exists needs no anchor to answer "no change".
    let exists = matches!(kind, ElementKind::Hero | ElementKind::Footer)
        && project
            .of_kind(kind)
            .any(|e| e.page == DEFAULT_PAGE);
    if let (Some(clause), false) = (&add.position, exists) {
        let action = PlacementAction::Add { spec: spec.clone() };
        match resolve_anchor(
            clause.anchor.role,
            clause.relation,
            &action,
            None,
            project,
            session,
        ) {
            Ok(position) => spec.position = Some(position),
            Err(outcome) => return outcome,
        }
    }
    ResolveOutcome::Resolved {
        request: ResolvedRequest::AddElement { spec },
    }
}

/// Resolves `move the <role> <relation> <anchor>`.
pub(crate) fn resolve_move(
    mv: &ElementMove,
    project: &ProjectSnapshot,
    session: &Session,
) -> ResolveOutcome {
    if let Err(outcome) = check_scope(session) {
        return outcome;
    }
    let candidates = match select_by_role(&mv.selector, project, session) {
        Ok(c) => c,
        Err(outcome) => return outcome,
    };
    let word = ElementKind::from(mv.selector.role).word();
    let target = match candidates.as_slice() {
        [] => {
            return stop(
                RejectionReason::TargetNotFound,
                format!("There is no {word} to move."),
            )
        }
        [only] => only.id.clone(),
        many => match pick(many, session) {
            Some(id) => id,
            None => {
                return choose_question(
                    session,
                    word,
                    many,
                    Continuation::ChooseMoveTarget {
                        relation: mv.position.relation,
                        anchor: mv.position.anchor.role,
                    },
                    "move",
                )
            }
        },
    };
    resolve_move_target(
        target,
        mv.position.relation,
        mv.position.anchor.role,
        project,
        session,
    )
}

/// Second half of a move: the target is known, find the anchor.
pub(crate) fn resolve_move_target(
    target: ElementId,
    relation: PositionRelation,
    anchor: ElementRole,
    project: &ProjectSnapshot,
    session: &Session,
) -> ResolveOutcome {
    let action = PlacementAction::Move {
        target: target.clone(),
    };
    match resolve_anchor(anchor, relation, &action, Some(&target), project, session) {
        Ok(position) => ResolveOutcome::Resolved {
            request: ResolvedRequest::MoveElement { target, position },
        },
        Err(outcome) => outcome,
    }
}

/// Persists a blocking question: a plan without operations whose follow-up is
/// the question. Applying it writes the conversation only, never source.
pub(crate) fn question_plan(
    project: &ProjectSnapshot,
    session: &Session,
    question: Question,
    mut preconditions: Vec<Precondition>,
    resolves: Vec<String>,
) -> PlanOutcome {
    preconditions.insert(
        0,
        Precondition::RevisionIs {
            revision: project.revision,
        },
    );
    PlanOutcome::Ready {
        plan: build_plan(project, session, Vec::new(), vec![question], preconditions, resolves),
    }
}

/// True if `question` is a placement question the conversation should keep.
pub(crate) fn is_placement_question(question: &Question) -> bool {
    matches!(
        question.continuation,
        Continuation::ChoosePlacementAnchor { .. } | Continuation::ChooseMoveTarget { .. }
    )
}

// ------------------------------------------------------------------ content

fn field_prompt(kind: ElementKind, field: ContentField) -> String {
    match field {
        ContentField::Headline => "What headline should the hero show?".into(),
        ContentField::FooterText => "What text should the footer show?".into(),
        ContentField::ButtonLabel => "What should the button say?".into(),
        ContentField::ImageSource => {
            "What is the image's address? Give a full http:// or https:// URL.".into()
        }
        ContentField::ImageAlt => {
            "How would you describe the image for people who cannot see it (its alt text)?".into()
        }
        ContentField::FormFields => format!(
            "Which fields should the {} have? List their names separated by commas, for example Name, Email.",
            kind.word()
        ),
        ContentField::SubmitLabel => "What should the form's submit button say?".into(),
    }
}

fn field_slug(field: ContentField) -> &'static str {
    match field {
        ContentField::Headline => "content-headline",
        ContentField::FooterText => "content-footer-text",
        ContentField::ButtonLabel => "content-button-label",
        ContentField::ImageSource => "content-image-source",
        ContentField::ImageAlt => "content-image-alt",
        ContentField::FormFields => "content-form-fields",
        ContentField::SubmitLabel => "content-submit-label",
    }
}

/// Blocking question for one missing content field of `spec`.
pub(crate) fn content_question(session: &Session, spec: &ElementSpec, field: ContentField) -> Question {
    Question {
        id: question_id(session, field_slug(field)),
        kind: QuestionKind::BlockingClarification,
        target: None,
        prompt: field_prompt(spec.kind, field),
        options: Vec::new(),
        continuation: Continuation::ProvideContent { spec: spec.clone() },
    }
}

/// `spec` with `field` set from the user's free text, or why it is invalid.
pub(crate) fn with_content(
    spec: &ElementSpec,
    field: ContentField,
    text: &str,
) -> Result<ElementSpec, String> {
    let mut spec = spec.clone();
    let text_of = |max: usize| {
        emit::clean_text(text, max)
            .ok_or_else(|| format!("That needs 1 to {max} characters on one line."))
    };
    match field {
        ContentField::Headline | ContentField::FooterText => {
            spec.content.text = Some(text_of(emit::MAX_TEXT_CHARS)?)
        }
        ContentField::ButtonLabel | ContentField::SubmitLabel => {
            spec.content.text = Some(text_of(emit::MAX_LABEL_CHARS)?)
        }
        ContentField::ImageSource => {
            spec.content.src = Some(emit::validate_external_url(text).ok_or_else(|| {
                "That is not a valid http(s) URL (a host name such as example.com is required)."
                    .to_string()
            })?)
        }
        ContentField::ImageAlt => spec.content.alt = Some(text_of(emit::MAX_TEXT_CHARS)?),
        ContentField::FormFields => {
            spec.content.fields =
                emit::parse_form_fields(text).map_err(|e| format!("That will not work: {e}."))?
        }
    }
    Ok(spec)
}

// ------------------------------------------------------------------ planning

fn conflict(reason: impl Into<String>) -> PlanOutcome {
    PlanOutcome::Conflict {
        reason: reason.into(),
    }
}

fn rejected(r: Rejection) -> PlanOutcome {
    PlanOutcome::Unsupported { rejection: r }
}

/// Plans creating `spec`: a question while content is missing, otherwise the
/// element and its insertion. Revalidates the anchor against `project`, so it
/// is also the final step of every answer path.
pub(crate) fn plan_add_element(
    spec: &ElementSpec,
    project: &ProjectSnapshot,
    session: &Session,
    resolves: Vec<String>,
) -> PlanOutcome {
    let kind = spec.kind;
    if !kind.is_flow() {
        return rejected(rejection(
            RejectionReason::CapabilityNotImplemented,
            format!("A {} cannot be added this way.", kind.word()),
        ));
    }
    if matches!(kind, ElementKind::Hero | ElementKind::Footer) {
        if let Some(existing) = project.of_kind(kind).find(|e| e.page == DEFAULT_PAGE) {
            return PlanOutcome::NoChange {
                reason: format!(
                    "The home page already has a {} ({}); use a move or style request to change it.",
                    kind.word(),
                    existing.id.0
                ),
                follow_up: Vec::new(),
            };
        }
    }
    let mut preconditions = Vec::new();
    if let Some(position) = &spec.position {
        let Some(anchor) = project.element(&position.anchor) else {
            return conflict(format!(
                "{} no longer exists, so this placement is out of date.",
                position.anchor.0
            ));
        };
        if let Err(r) = check_position(project, position.relation, anchor) {
            return rejected(r);
        }
        preconditions.push(Precondition::ElementExists {
            id: position.anchor.clone(),
        });
    }
    if let Some(field) = spec.content.missing(kind) {
        return question_plan(
            project,
            session,
            content_question(session, spec, field),
            preconditions,
            resolves,
        );
    }
    if let Err(message) = emit::validate_content(kind, &spec.content) {
        return rejected(rejection(
            RejectionReason::CapabilityNotImplemented,
            message,
        ));
    }
    let id = project.next_id(kind.id_prefix());
    let Some(name) = emit::element_component_name(kind, &id) else {
        return rejected(rejection(
            RejectionReason::CapabilityNotImplemented,
            format!("Cannot derive a component name for {}.", id.0),
        ));
    };
    let mut all = vec![
        Precondition::RevisionIs {
            revision: project.revision,
        },
        Precondition::ElementAbsent { id: id.clone() },
    ];
    all.extend(preconditions);
    let follow_up = if kind == ElementKind::Button {
        let label = spec.content.text.clone().unwrap_or_default();
        vec![destination_question(session, &id, &label)]
    } else {
        Vec::new()
    };
    PlanOutcome::Ready {
        plan: build_plan(
            project,
            session,
            vec![
                Operation::CreateComponent {
                    id: id.clone(),
                    kind,
                    name,
                    content: Some(spec.content.clone()),
                },
                Operation::InsertElement {
                    element: id,
                    page: DEFAULT_PAGE.into(),
                    parent: None,
                    at: spec.position.clone(),
                },
            ],
            follow_up,
            all,
            resolves,
        ),
    }
}

/// Plans moving `target` to `position`.
pub(crate) fn plan_move(
    target: &ElementId,
    position: &Position,
    project: &ProjectSnapshot,
    session: &Session,
    resolves: Vec<String>,
) -> PlanOutcome {
    let Some(rec) = project.element(target) else {
        return conflict(format!("{} no longer exists.", target.0));
    };
    if !rec.kind.is_flow() {
        return rejected(rejection(
            RejectionReason::CapabilityNotImplemented,
            format!("{} cannot be moved.", target.0),
        ));
    }
    let Some(anchor) = project.element(&position.anchor) else {
        return conflict(format!(
            "{} no longer exists, so this move is out of date.",
            position.anchor.0
        ));
    };
    if anchor.id == rec.id {
        return rejected(rejection(
            RejectionReason::CapabilityNotImplemented,
            format!("{} cannot be placed relative to itself.", target.0),
        ));
    }
    if let Err(r) = check_position(project, position.relation, anchor) {
        return rejected(r);
    }
    if rec.page != anchor.page || rec.parent != anchor.parent {
        return rejected(rejection(
            RejectionReason::CapabilityNotImplemented,
            format!(
                "{} and {} are not in the same container.",
                target.0, anchor.id.0
            ),
        ));
    }
    let siblings = flow_siblings(project, &rec.page);
    let index_of = |id: &ElementId| siblings.iter().position(|e| &e.id == id);
    if let (Some(at), Some(anchor_at)) = (index_of(target), index_of(&position.anchor)) {
        let after = matches!(
            position.relation,
            PositionRelation::Below | PositionRelation::RightOf
        );
        if (after && at == anchor_at + 1) || (!after && at + 1 == anchor_at) {
            return PlanOutcome::NoChange {
                reason: format!(
                    "{} is already {} {}.",
                    target.0,
                    relation_text(position.relation),
                    position.anchor.0
                ),
                follow_up: Vec::new(),
            };
        }
    }
    PlanOutcome::Ready {
        plan: build_plan(
            project,
            session,
            vec![Operation::MoveElement {
                element: target.clone(),
                position: position.clone(),
            }],
            Vec::new(),
            vec![
                Precondition::RevisionIs {
                    revision: project.revision,
                },
                Precondition::ElementExists { id: target.clone() },
                Precondition::ElementExists {
                    id: position.anchor.clone(),
                },
            ],
            resolves,
        ),
    }
}
