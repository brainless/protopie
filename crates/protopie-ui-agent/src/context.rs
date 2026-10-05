//! Solid context extension (Epic 001, T10): resolving and planning
//! `share the <label> [across pages]`. Pure.
//!
//! # Semantics
//!
//! A shared state needs four things the agent never invents:
//!
//! * **Scope**: where the provider is mounted. `across pages` says it; when
//!   the prompt omits it the agent asks. The only supported scope is
//!   [`ContextScope::AllPages`]: the provider wraps the router's root layout
//!   (the `providers` region of `src/App.tsx`), so every page reads the same
//!   value and the value survives navigation.
//! * **State shape**: text, a number, yes or no, or text that may be missing.
//! * **Initial value**: the user's answer (an optional text starts unset by
//!   that choice).
//! * **Consumer bindings**: the pages that display the value. A consumer is a
//!   page created by the agent; it renders `<label>: <value>` read with
//!   `useContext`. Nothing else reads or writes the state yet, and the home
//!   page cannot be a consumer yet.
//!
//! Each missing requirement is a persisted blocking question; the draft
//! accumulates in the question's continuation, so nothing is written until the
//! context is complete, and then one plan creates the context, its provider
//! and every consumer.

use crate::contracts::*;
use crate::emit;
use crate::parser::StateSharing;
use crate::pipeline::{build_plan, question_id};
use crate::place::question_plan;

fn quote(text: &str) -> String {
    format!("\u{201c}{text}\u{201d}")
}

fn rejection(explanation: String) -> Rejection {
    Rejection {
        reason: RejectionReason::CapabilityNotImplemented,
        explanation,
        span: None,
    }
}

fn unsupported(explanation: String) -> PlanOutcome {
    PlanOutcome::Unsupported {
        rejection: rejection(explanation),
    }
}

/// Why `label` cannot name a context, if it cannot.
fn check_label(label: &str) -> Result<(String, String), String> {
    if label.trim().is_empty() || label.chars().count() > emit::MAX_LABEL_CHARS {
        return Err(format!(
            "A state's name must have 1 to {} characters.",
            emit::MAX_LABEL_CHARS
        ));
    }
    emit::context_names(label).ok_or_else(|| {
        format!(
            "Cannot derive a context name from {}: start with a letter and use at most {} letters, digits or separators.",
            quote(label),
            emit::MAX_CONTEXT_ID_CHARS
        )
    })
}

/// Resolves `share the <label> [across pages]`.
pub(crate) fn resolve_share(
    share: &StateSharing,
    project: &ProjectSnapshot,
) -> ResolveOutcome {
    let label = share.label.text.clone();
    let (id, _) = match check_label(&label) {
        Ok(names) => names,
        Err(why) => {
            return ResolveOutcome::Unsupported {
                rejection: rejection(why),
            }
        }
    };
    if project.context(&id).is_none() && project.pages.is_empty() {
        return ResolveOutcome::Unsupported {
            rejection: rejection(
                "No page can show a shared value yet. Create a page first, for example \u{201c}Add a Doctors page\u{201d}; the home page cannot read a context yet."
                    .into(),
            ),
        };
    }
    let mut draft = ContextDraft::new(label);
    draft.scope = share.scope.as_ref().map(|_| ContextScope::AllPages);
    ResolveOutcome::Resolved {
        request: ResolvedRequest::ShareContext { draft },
    }
}

fn option(key: &str, label: impl Into<String>) -> QuestionOption {
    QuestionOption {
        key: key.into(),
        label: label.into(),
    }
}

/// The pages `draft` has not yet bound, as options.
fn page_options(draft: &ContextDraft, project: &ProjectSnapshot) -> Vec<QuestionOption> {
    project
        .pages
        .iter()
        .filter(|p| !draft.consumers.contains(&p.id))
        .map(|p| option(&p.id, format!("{} ({})", p.label, p.path)))
        .collect()
}

/// Blocking question for the next missing requirement of `draft`.
fn context_question(
    session: &Session,
    project: &ProjectSnapshot,
    draft: &ContextDraft,
    field: ContextField,
) -> Question {
    let label = quote(&draft.label);
    let (slug, prompt, options) = match field {
        ContextField::Scope => (
            "context-scope",
            format!("Where should {label} be shared?"),
            vec![option("all_pages", "Across all pages")],
        ),
        ContextField::Shape => (
            "context-shape",
            format!("What kind of value is {label}?"),
            vec![
                option("text", "Text"),
                option("number", "A number"),
                option("flag", "Yes or no"),
                option(
                    "optional_text",
                    "Text that may be missing (starts with no value)",
                ),
            ],
        ),
        ContextField::Initial => {
            let hint = match draft.shape {
                Some(StateShape::Number) => "a number such as 3 or 0.5",
                Some(StateShape::Flag) => "yes or no",
                _ => "some text on one line",
            };
            (
                "context-initial",
                format!("What value should {label} start with? Give {hint}."),
                Vec::new(),
            )
        }
        ContextField::Consumers => {
            let mut options = page_options(draft, project);
            let prompt = if draft.consumers.is_empty() {
                format!("Which page should show {label}?")
            } else {
                options.push(option("done", "No, that is all"));
                format!("Should another page show {label}?")
            };
            ("context-consumers", prompt, options)
        }
    };
    Question {
        id: question_id(session, slug),
        kind: QuestionKind::BlockingClarification,
        target: None,
        prompt,
        options,
        continuation: Continuation::ProvideContext {
            draft: draft.clone(),
            field,
        },
    }
}

/// Plans `draft`: the next question while a requirement is missing, otherwise
/// the context, its provider and its consumers. Revalidates everything
/// against `project`, so it is also the last step of every answer path.
pub(crate) fn plan_context(
    draft: &ContextDraft,
    project: &ProjectSnapshot,
    session: &Session,
    resolves: Vec<String>,
) -> PlanOutcome {
    let (id, name) = match check_label(&draft.label) {
        Ok(names) => names,
        Err(why) => return unsupported(why),
    };
    if let Some(existing) = project.context(&id) {
        return PlanOutcome::NoChange {
            reason: format!(
                "{} is already shared as {} (the context file src/context/{}.tsx).",
                quote(&existing.label),
                existing.id,
                existing.name
            ),
            follow_up: Vec::new(),
        };
    }
    if let Some(other) = project
        .contexts
        .iter()
        .find(|c| c.name.eq_ignore_ascii_case(&name))
    {
        return unsupported(format!(
            "{} would use the same file name as {}; use a different name.",
            quote(&draft.label),
            quote(&other.label)
        ));
    }
    if let Some(missing) = draft
        .consumers
        .iter()
        .find(|p| project.page(p).is_none())
    {
        return PlanOutcome::Conflict {
            reason: format!("Page {missing} no longer exists, so this answer is out of date."),
        };
    }
    // Every page is already bound: nothing is left to ask.
    let mut draft = draft.clone();
    if !draft.consumers.is_empty() && draft.consumers.len() == project.pages.len() {
        draft.consumers_done = true;
    }
    if draft.consumers.is_empty() && project.pages.is_empty() {
        return unsupported(
            "No page can show a shared value yet. Create a page first; the home page cannot read a context yet."
                .into(),
        );
    }
    let mut preconditions = vec![
        Precondition::RevisionIs {
            revision: project.revision,
        },
        Precondition::ContextAbsent { id: id.clone() },
    ];
    preconditions.extend(draft.consumers.iter().map(|p| Precondition::PageExists {
        page: p.clone(),
    }));
    if let Some(field) = draft.missing() {
        return question_plan(
            project,
            session,
            context_question(session, project, &draft, field),
            preconditions,
            resolves,
        );
    }
    let (Some(scope), Some(shape), Some(initial)) =
        (draft.scope, draft.shape, draft.initial.clone())
    else {
        unreachable!("a complete draft has scope, shape and initial value")
    };
    if let Err(why) = emit::validate_initial(shape, &initial) {
        return unsupported(why);
    }
    let mut operations = vec![Operation::CreateContext {
        id: id.clone(),
        label: draft.label.clone(),
        name,
        shape,
        initial,
        scope,
    }];
    operations.extend(draft.consumers.iter().map(|page| Operation::AddContextConsumer {
        context: id.clone(),
        page: page.clone(),
    }));
    PlanOutcome::Ready {
        plan: build_plan(project, session, operations, Vec::new(), preconditions, resolves),
    }
}

/// What an answer supplies.
pub(crate) enum Given<'a> {
    /// A key of one of the question's options.
    Option(&'a str),
    Text(&'a str),
}

/// Applies an answer to the requirement `field` of `draft`: the updated
/// draft, or why the answer does not fit (the question is then asked again).
pub(crate) fn with_answer(
    draft: &ContextDraft,
    field: ContextField,
    given: Given<'_>,
    project: &ProjectSnapshot,
) -> Result<ContextDraft, String> {
    let mut next = draft.clone();
    match (field, given) {
        (ContextField::Scope, Given::Option("all_pages")) => {
            next.scope = Some(ContextScope::AllPages)
        }
        (ContextField::Shape, Given::Option(key)) => {
            let shape = [
                StateShape::Text,
                StateShape::Number,
                StateShape::Flag,
                StateShape::OptionalText,
            ]
            .into_iter()
            .find(|s| s.key() == key)
            .ok_or("That is not one of the options.")?;
            next.shape = Some(shape);
            // An optional text starts unset by its own definition.
            next.initial = (shape == StateShape::OptionalText).then_some(InitialValue::Unset);
        }
        (ContextField::Initial, Given::Text(text)) => {
            let shape = draft.shape.ok_or("The kind of value is not known yet.")?;
            next.initial = Some(emit::parse_initial(shape, text)?);
        }
        (ContextField::Consumers, Given::Option("done")) => {
            if draft.consumers.is_empty() {
                return Err("At least one page has to show it.".into());
            }
            next.consumers_done = true;
        }
        (ContextField::Consumers, Given::Option(page)) => {
            if project.page(page).is_none() || draft.consumers.iter().any(|p| p == page) {
                return Err("That is not one of the options.".into());
            }
            next.consumers.push(page.to_string());
        }
        (ContextField::Initial, Given::Option(_)) => return Err("Please answer in words.".into()),
        _ => return Err("That is not one of the options.".into()),
    }
    Ok(next)
}
