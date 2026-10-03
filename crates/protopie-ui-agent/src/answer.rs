//! Question continuations (Epic 001, T6): turning an answer to a pending
//! question into typed data, a further question, or a plan.
//!
//! Everything here is pure. Answers are revalidated against the current
//! project snapshot: an answer to a question that is no longer pending, or
//! whose target was deleted or already linked, is a conflict that changes
//! nothing. Bare chat replies (`1`, `new page`, `yes`) are only turned into
//! answers when exactly one pending question can take them.

use crate::contracts::*;
use crate::emit;
use crate::parser::{self, ParseOutcome, UnsupportedReason};
use crate::pipeline::{build_plan, plan_style, question_id};
use crate::place;

// ------------------------------------------------------------------ pages

/// A page that occupies a slug (the implicit home page included).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageRef {
    pub id: String,
    pub path: String,
    pub label: String,
}

/// The existing page that stops `slug` from being used for a new page: one
/// with the same ID/path, or whose derived component name would be identical.
pub fn conflicting_page(project: &ProjectSnapshot, slug: &str) -> Option<PageRef> {
    if slug == DEFAULT_PAGE {
        return Some(PageRef {
            id: DEFAULT_PAGE.into(),
            path: HOME_PATH.into(),
            label: HOME_LABEL.into(),
        });
    }
    let component = emit::page_component_name(slug);
    project
        .pages
        .iter()
        .find(|p| p.id == slug || p.component == component)
        .map(|p| PageRef {
            id: p.id.clone(),
            path: p.path.clone(),
            label: p.label.clone(),
        })
}

/// First free `<slug>-<n>` (n from 2) that is also a valid slug.
pub fn alternative_slug(project: &ProjectSnapshot, slug: &str) -> Option<String> {
    (2u32..1000)
        .map(|n| format!("{slug}-{n}"))
        .find(|candidate| {
            emit::is_valid_slug(candidate) && conflicting_page(project, candidate).is_none()
        })
}

/// Operations that create a page and register its route.
pub fn create_page_ops(slug: &str, label: &str) -> Vec<Operation> {
    vec![
        Operation::CreatePage {
            page: slug.into(),
            label: label.into(),
        },
        Operation::RegisterRoute {
            page: slug.into(),
            path: emit::page_path(slug),
        },
    ]
}

// ------------------------------------------------------------------ link targets

/// An existing page or section a navigation item can lead to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkTarget {
    pub key: String,
    pub label: String,
    pub destination: Destination,
}

/// Existing destinations in display order: pages (home first, then created
/// pages with a registered route), then anchorable sections (heroes).
pub fn link_targets(project: &ProjectSnapshot) -> Vec<LinkTarget> {
    let mut targets = vec![LinkTarget {
        key: format!("page:{DEFAULT_PAGE}"),
        label: format!("{HOME_LABEL} page ({HOME_PATH})"),
        destination: Destination::Page {
            page: DEFAULT_PAGE.into(),
        },
    }];
    for page in project.pages.iter().filter(|p| p.registered) {
        targets.push(LinkTarget {
            key: format!("page:{}", page.id),
            label: format!("{} page ({})", page.label, page.path),
            destination: Destination::Page {
                page: page.id.clone(),
            },
        });
    }
    for element in project.of_kind(ElementKind::Hero) {
        let Some(page_label) = project
            .page_path(&element.page)
            .and_then(|_| project.page_label(&element.page))
        else {
            continue;
        };
        targets.push(LinkTarget {
            key: format!("section:{}:{}", element.page, element.id.0),
            label: format!(
                "{} section on {page_label}",
                element.label.as_deref().unwrap_or(&element.id.0)
            ),
            destination: Destination::Section {
                page: element.page.clone(),
                section: element.id.0.clone(),
            },
        });
    }
    targets
}

// ------------------------------------------------------------------ questions

fn option(key: &str, label: impl Into<String>) -> QuestionOption {
    QuestionOption {
        key: key.into(),
        label: label.into(),
    }
}

fn blocking(
    session: &Session,
    slug: &str,
    item: &ElementId,
    prompt: String,
    options: Vec<QuestionOption>,
    continuation: Continuation,
) -> Question {
    Question {
        id: question_id(session, slug),
        kind: QuestionKind::BlockingClarification,
        target: Some(item.clone()),
        prompt,
        options,
        continuation,
    }
}

fn quote(text: &str) -> String {
    format!("\u{201c}{text}\u{201d}")
}

fn name_page_question(session: &Session, item: &ElementId, label: &str, why: &str) -> Question {
    blocking(
        session,
        "name-page",
        item,
        format!(
            "{why} What should the new page for {} be called? Use letters or digits.",
            quote(label)
        ),
        Vec::new(),
        Continuation::NamePage { item: item.clone() },
    )
}

// ------------------------------------------------------------------ answers

fn normalize(text: &str) -> String {
    let lowered = text.to_lowercase().replace('_', " ");
    let joined = lowered.split_whitespace().collect::<Vec<_>>().join(" ");
    let trimmed = joined.trim_end_matches(['.', '!', '?']).trim();
    for article in ["a ", "an ", "the "] {
        if let Some(rest) = trimmed.strip_prefix(article) {
            return rest.to_string();
        }
    }
    trimmed.to_string()
}

/// The option an answer picks: an exact key, a 1-based number, or a label
/// (case-insensitive, leading article ignored).
fn match_option<'q>(question: &'q Question, choice: &AnswerChoice) -> Option<&'q QuestionOption> {
    match choice {
        AnswerChoice::Option { key } => question.options.iter().find(|o| &o.key == key),
        AnswerChoice::Text { text } => option_for_text(question, text),
    }
}

fn option_for_text<'q>(question: &'q Question, text: &str) -> Option<&'q QuestionOption> {
    let t = normalize(text);
    if t.is_empty() {
        return None;
    }
    if let Ok(n) = t.parse::<usize>() {
        return n.checked_sub(1).and_then(|i| question.options.get(i));
    }
    question
        .options
        .iter()
        .find(|o| normalize(&o.key) == t || normalize(&o.label) == t)
}

fn is_yes_no(question: &Question) -> bool {
    let mut keys: Vec<&str> = question.options.iter().map(|o| o.key.as_str()).collect();
    keys.sort_unstable();
    keys == ["no", "yes"]
}

fn conflict(reason: impl Into<String>) -> PlanOutcome {
    PlanOutcome::Conflict {
        reason: reason.into(),
    }
}

fn reask(question: &Question, prefix: &str) -> PlanOutcome {
    let mut q = question.clone();
    if !prefix.is_empty() {
        q.prompt = format!("{prefix} {}", q.prompt);
    }
    PlanOutcome::NeedsClarification { questions: vec![q] }
}

fn preconditions(project: &ProjectSnapshot, item: &ElementId) -> Vec<Precondition> {
    vec![
        Precondition::RevisionIs {
            revision: project.revision,
        },
        Precondition::ElementExists { id: item.clone() },
    ]
}

/// A plan that only changes the conversation: it answers `question` and asks
/// `next`. Applying it writes session and ledger, never source.
fn transition(
    project: &ProjectSnapshot,
    session: &Session,
    item: &ElementId,
    question: &Question,
    next: Question,
) -> PlanOutcome {
    PlanOutcome::Ready {
        plan: build_plan(
            project,
            session,
            Vec::new(),
            vec![next],
            preconditions(project, item),
            vec![question.id.clone()],
        ),
    }
}

fn link_plan(
    project: &ProjectSnapshot,
    session: &Session,
    item: &ElementId,
    question: &Question,
    destination: Destination,
) -> PlanOutcome {
    PlanOutcome::Ready {
        plan: build_plan(
            project,
            session,
            vec![Operation::SetNavigationDestination {
                item: item.clone(),
                destination,
            }],
            Vec::new(),
            preconditions(project, item),
            vec![question.id.clone()],
        ),
    }
}

/// New-page route for `item`: the page, its route and the link in one plan,
/// or a question when the path cannot be determined uniquely.
fn plan_new_page(
    project: &ProjectSnapshot,
    session: &Session,
    item: &ElementId,
    question: &Question,
    page_label: &str,
    item_label: &str,
) -> PlanOutcome {
    let reasked = matches!(question.continuation, Continuation::NamePage { .. });
    let Some(slug) = emit::derive_slug(page_label) else {
        let why = format!("A page path cannot be derived from {}.", quote(page_label));
        return if reasked {
            reask(question, &why)
        } else {
            transition(
                project,
                session,
                item,
                question,
                name_page_question(session, item, item_label, &why),
            )
        };
    };
    if let Some(existing) = conflicting_page(project, &slug) {
        let Some(alternative) = alternative_slug(project, &slug) else {
            return transition(
                project,
                session,
                item,
                question,
                name_page_question(
                    session,
                    item,
                    item_label,
                    &format!("A page at /{slug} already exists."),
                ),
            );
        };
        let next = blocking(
            session,
            "page-collision",
            item,
            format!(
                "A page at {} already exists ({}). What should {} lead to?",
                existing.path,
                existing.label,
                quote(item_label)
            ),
            vec![
                option(
                    "use_existing",
                    format!("Link to the existing page ({})", existing.path),
                ),
                option(
                    "new_page",
                    format!("Create a new page at {}", emit::page_path(&alternative)),
                ),
            ],
            Continuation::ResolveCollision {
                item: item.clone(),
                existing_page: existing.id,
                alternative,
            },
        );
        return transition(project, session, item, question, next);
    }
    new_page_plan(project, session, item, question, &slug, page_label)
}

fn new_page_plan(
    project: &ProjectSnapshot,
    session: &Session,
    item: &ElementId,
    question: &Question,
    slug: &str,
    page_label: &str,
) -> PlanOutcome {
    let mut operations = create_page_ops(slug, page_label);
    operations.push(Operation::SetNavigationDestination {
        item: item.clone(),
        destination: Destination::Page { page: slug.into() },
    });
    let mut pre = preconditions(project, item);
    pre.push(Precondition::PageAbsent { page: slug.into() });
    PlanOutcome::Ready {
        plan: build_plan(
            project,
            session,
            operations,
            Vec::new(),
            pre,
            vec![question.id.clone()],
        ),
    }
}

/// Plans the effect of `answer` on the pending question it references.
pub fn plan_answer(answer: &Answer, project: &ProjectSnapshot, session: &Session) -> PlanOutcome {
    let Some(question) = session
        .pending_questions
        .iter()
        .find(|q| q.id == answer.question_id)
    else {
        return conflict(format!(
            "Question {} is not pending (it was already answered, replaced, or never asked).",
            answer.question_id
        ));
    };
    if let Continuation::ChooseStyleTarget { change } = &question.continuation {
        // Option keys are the element IDs the question offered. Only those are
        // valid, and each must still exist.
        let Some(chosen) = match_option(question, &answer.choice) else {
            return reask(question, "That is not one of the options.");
        };
        let target = ElementId::new(chosen.key.as_str());
        return plan_style(
            &target,
            *change,
            project,
            session,
            vec![question.id.clone()],
        );
    }
    match &question.continuation {
        Continuation::ProvideContent { spec } => {
            let AnswerChoice::Text { text } = &answer.choice else {
                return reask(question, "Please answer in words.");
            };
            let Some(field) = spec.content.missing(spec.kind) else {
                return conflict("This question no longer needs an answer.");
            };
            return match place::with_content(spec, field, text) {
                Ok(next) => {
                    place::plan_add_element(&next, project, session, vec![question.id.clone()])
                }
                Err(why) => reask(question, &why),
            };
        }
        Continuation::ChoosePlacementAnchor { action, relation } => {
            let Some(chosen) = match_option(question, &answer.choice) else {
                return reask(question, "That is not one of the options.");
            };
            let position = Position {
                relation: *relation,
                anchor: ElementId::new(chosen.key.as_str()),
            };
            let resolves = vec![question.id.clone()];
            return match action {
                PlacementAction::Add { spec } => {
                    let mut spec = spec.clone();
                    spec.position = Some(position);
                    place::plan_add_element(&spec, project, session, resolves)
                }
                PlacementAction::Move { target } => {
                    place::plan_move(target, &position, project, session, resolves)
                }
            };
        }
        Continuation::ChooseMoveTarget { relation, anchor } => {
            let Some(chosen) = match_option(question, &answer.choice) else {
                return reask(question, "That is not one of the options.");
            };
            let target = ElementId::new(chosen.key.as_str());
            if project.element(&target).is_none() {
                return conflict(format!("{} no longer exists.", target.0));
            }
            let resolves = vec![question.id.clone()];
            return match place::resolve_move_target(target, *relation, *anchor, project, session) {
                ResolveOutcome::Resolved {
                    request: ResolvedRequest::MoveElement { target, position },
                } => place::plan_move(&target, &position, project, session, resolves),
                ResolveOutcome::NeedsClarification { mut questions }
                    if questions.len() == 1 && place::is_placement_question(&questions[0]) =>
                {
                    place::question_plan(project, session, questions.remove(0), Vec::new(), resolves)
                }
                ResolveOutcome::NeedsClarification { questions } => {
                    PlanOutcome::NeedsClarification { questions }
                }
                ResolveOutcome::Unsupported { rejection } => PlanOutcome::Unsupported { rejection },
                ResolveOutcome::Resolved { .. } => conflict("This move no longer applies."),
            };
        }
        _ => {}
    }
    let Some(item_id) = question.continuation.item() else {
        return PlanOutcome::Unsupported {
            rejection: Rejection {
                reason: RejectionReason::CapabilityNotImplemented,
                explanation: "This question cannot be answered by ID; send the request again."
                    .into(),
                span: None,
            },
        };
    };
    let Some(item) = project
        .element(item_id)
        .filter(|e| e.kind.has_destination())
    else {
        return conflict(format!(
            "The item {} no longer exists, so this answer is out of date.",
            item_id.0
        ));
    };
    if item.destination != Some(Destination::Unresolved) {
        return conflict(format!(
            "{} already leads somewhere; this answer would override a newer change.",
            quote(item.label.as_deref().unwrap_or(&item_id.0))
        ));
    }
    let item_label = item.label.clone().unwrap_or_default();
    let text = match &answer.choice {
        AnswerChoice::Text { text } => Some(text.as_str()),
        AnswerChoice::Option { .. } => None,
    };

    match &question.continuation {
        Continuation::ChooseDestination { .. } => {
            let Some(chosen) = match_option(question, &answer.choice) else {
                return reask(question, "That is not one of the options.");
            };
            match chosen.key.as_str() {
                "new_page" => plan_new_page(
                    project,
                    session,
                    item_id,
                    question,
                    &item_label,
                    &item_label,
                ),
                "existing" => {
                    let options = link_targets(project)
                        .into_iter()
                        .map(|t| option(&t.key, t.label))
                        .collect();
                    let next = blocking(
                        session,
                        "which-page",
                        item_id,
                        format!(
                            "Which page or section should {} lead to?",
                            quote(&item_label)
                        ),
                        options,
                        Continuation::ChooseExisting {
                            item: item_id.clone(),
                        },
                    );
                    transition(project, session, item_id, question, next)
                }
                "external" => {
                    let next = blocking(
                        session,
                        "url",
                        item_id,
                        format!(
                            "What URL should {} lead to? (http:// or https://)",
                            quote(&item_label)
                        ),
                        Vec::new(),
                        Continuation::EnterUrl {
                            item: item_id.clone(),
                        },
                    );
                    transition(project, session, item_id, question, next)
                }
                // Dismisses the question; the item stays unresolved text.
                "unlinked" => PlanOutcome::Ready {
                    plan: build_plan(
                        project,
                        session,
                        Vec::new(),
                        Vec::new(),
                        preconditions(project, item_id),
                        vec![question.id.clone()],
                    ),
                },
                _ => reask(question, "That is not one of the options."),
            }
        }
        Continuation::ChooseExisting { .. } => {
            let Some(chosen) = match_option(question, &answer.choice) else {
                return reask(question, "That is not one of the options.");
            };
            // Only destinations that exist right now are valid, whatever the
            // question offered when it was asked.
            match link_targets(project)
                .into_iter()
                .find(|t| t.key == chosen.key)
            {
                Some(target) => link_plan(project, session, item_id, question, target.destination),
                None => conflict(format!(
                    "{} no longer exists, so this answer is out of date.",
                    chosen.label
                )),
            }
        }
        Continuation::EnterUrl { .. } => match text.and_then(emit::validate_external_url) {
            Some(url) => link_plan(
                project,
                session,
                item_id,
                question,
                Destination::External { url },
            ),
            None => reask(
                question,
                "That is not a valid http(s) URL (a host name such as example.com is required).",
            ),
        },
        Continuation::NamePage { .. } => {
            let name = text
                .map(str::trim)
                .filter(|t| !t.is_empty() && t.chars().count() <= emit::MAX_LABEL_CHARS);
            match name {
                Some(name) => plan_new_page(project, session, item_id, question, name, &item_label),
                None => reask(question, "A page name needs 1 to 80 characters."),
            }
        }
        Continuation::ResolveCollision {
            existing_page,
            alternative,
            ..
        } => {
            let Some(chosen) = match_option(question, &answer.choice) else {
                return reask(question, "That is not one of the options.");
            };
            match chosen.key.as_str() {
                "use_existing" => {
                    if project.page_path(existing_page).is_none() {
                        return conflict("The existing page is no longer available.");
                    }
                    link_plan(
                        project,
                        session,
                        item_id,
                        question,
                        Destination::Page {
                            page: existing_page.clone(),
                        },
                    )
                }
                "new_page" => {
                    if conflicting_page(project, alternative).is_some() {
                        return conflict(format!(
                            "/{alternative} is no longer free; ask again to pick another path."
                        ));
                    }
                    new_page_plan(
                        project,
                        session,
                        item_id,
                        question,
                        alternative,
                        &item_label,
                    )
                }
                _ => reask(question, "That is not one of the options."),
            }
        }
        Continuation::Rephrase { .. }
        | Continuation::ChooseNavigation { .. }
        | Continuation::ChooseStyleTarget { .. }
        | Continuation::ChoosePlacementAnchor { .. }
        | Continuation::ChooseMoveTarget { .. }
        | Continuation::ProvideContent { .. } => {
            unreachable!("filtered out: these continuations have no item")
        }
    }
}

// ------------------------------------------------------------------ bare answers

/// Pending questions that can still be answered against `project`.
pub fn applicable_questions<'a>(
    project: &ProjectSnapshot,
    session: &'a Session,
) -> Vec<&'a Question> {
    session
        .pending_questions
        .iter()
        .filter(|q| match &q.continuation {
            Continuation::ChoosePlacementAnchor { .. }
            | Continuation::ChooseMoveTarget { .. }
            | Continuation::ProvideContent { .. } => true,
            other => other.item().is_some_and(|id| {
                project.element(id).is_some_and(|e| {
                    e.kind.has_destination() && e.destination == Some(Destination::Unresolved)
                })
            }),
        })
        .collect()
}

/// `q<turn>-<slug>`, the shape of every ID the planner issues.
fn looks_like_question_id(text: &str) -> bool {
    let Some(rest) = text.strip_prefix('q') else {
        return false;
    };
    let digits = rest.chars().take_while(char::is_ascii_digit).count();
    let slug = &rest[digits..];
    digits > 0
        && slug.len() > 1
        && slug.starts_with('-')
        && slug[1..]
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b == b'-')
}

/// How a chat message relates to the pending questions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BareAnswer {
    /// Not an answer: treat the message as a new command.
    NotAnAnswer,
    Answer(Answer),
    /// Several pending questions could take it; none is chosen.
    Ambiguous(Vec<Question>),
}

/// Interprets a chat message as an answer when that is unambiguous.
///
/// * `<question id>: <answer>` always targets that question.
/// * An option number, key, label or `yes`/`no` answers the one applicable
///   question that accepts it; several candidates are ambiguous.
/// * Otherwise free text answers the only applicable text question, but only
///   when it is not a recognizable command.
pub fn interpret_bare(prompt: &str, project: &ProjectSnapshot, session: &Session) -> BareAnswer {
    let applicable = applicable_questions(project, session);
    let trimmed = prompt.trim();
    // `<question id>: <answer>`. An ID-shaped prefix that is not pending is
    // still an answer, so a stale reply is reported instead of misread as a
    // command.
    if let Some((left, right)) = trimmed.split_once(':') {
        let id = left.trim();
        if session.pending_questions.iter().any(|q| q.id == id) || looks_like_question_id(id) {
            return BareAnswer::Answer(Answer {
                question_id: id.to_string(),
                choice: AnswerChoice::Text {
                    text: right.trim().to_string(),
                },
            });
        }
    }
    if applicable.is_empty() {
        return BareAnswer::NotAnAnswer;
    }
    let norm = normalize(trimmed);
    let candidates: Vec<&Question> = applicable
        .iter()
        .copied()
        .filter(|q| {
            option_for_text(q, trimmed).is_some()
                || (is_yes_no(q) && (norm == "yes" || norm == "no"))
        })
        .collect();
    match candidates.as_slice() {
        [only] => {
            let key = if is_yes_no(only) && (norm == "yes" || norm == "no") {
                norm
            } else {
                option_for_text(only, trimmed)
                    .expect("candidate accepts the text")
                    .key
                    .clone()
            };
            return BareAnswer::Answer(Answer {
                question_id: only.id.clone(),
                choice: AnswerChoice::Option { key },
            });
        }
        [] => {}
        many => return BareAnswer::Ambiguous(many.iter().map(|q| (*q).clone()).collect()),
    }
    let text_questions: Vec<&Question> = applicable
        .iter()
        .copied()
        .filter(|q| q.continuation.takes_text())
        .collect();
    if let [only] = text_questions.as_slice() {
        let unrecognized = matches!(
            parser::parse(trimmed),
            ParseOutcome::Unsupported(u) if u.reason == UnsupportedReason::UnrecognizedInput
        );
        if unrecognized {
            return BareAnswer::Answer(Answer {
                question_id: only.id.clone(),
                choice: AnswerChoice::Text {
                    text: trimmed.to_string(),
                },
            });
        }
    }
    BareAnswer::NotAnAnswer
}

#[cfg(test)]
#[path = "answer_tests.rs"]
mod tests;
