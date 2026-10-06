//! Review state of the chat: what the user can click after a reply. Pure logic,
//! so it is unit tested without a window.
//!
//! A command first runs as a dry run. A preview becomes an [`Choice::Apply`] /
//! [`Choice::Discard`] pair bound to the previewed plan and revision; a reply
//! with questions becomes one button per structured option.

use protopie_api::{ModifyOutcome, ModifyProjectResponse};

/// What was sent, so a preview can be applied by repeating it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Sent {
    Command(String),
    Answer {
        question_id: String,
        option_key: String,
    },
}

/// The preview a later apply is bound to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Expected {
    pub revision: u64,
    pub plan_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Choice {
    /// Apply the previewed change against its recorded revision.
    Apply,
    /// Drop the preview; nothing was written.
    Discard,
    /// Answer a question by option; previewed like any other command.
    Answer {
        question_id: String,
        option_key: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Button {
    pub label: String,
    pub choice: Choice,
}

/// Review state derived from one reply.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Review {
    /// Set when the reply is a preview awaiting a decision.
    pub expected: Option<Expected>,
    pub buttons: Vec<Button>,
}

pub fn review_for(response: &ModifyProjectResponse) -> Review {
    let questions = match &response.outcome {
        Some(ModifyOutcome::Preview {
            plan_id,
            base_revision,
            changed_files,
            ..
        }) => {
            return Review {
                expected: Some(Expected {
                    revision: *base_revision,
                    plan_id: plan_id.clone(),
                }),
                buttons: vec![
                    Button {
                        label: if changed_files.is_empty() {
                            "Continue"
                        } else {
                            "Apply changes"
                        }
                        .into(),
                        choice: Choice::Apply,
                    },
                    Button {
                        label: "Discard".into(),
                        choice: Choice::Discard,
                    },
                ],
            }
        }
        Some(ModifyOutcome::Applied { questions, .. })
        | Some(ModifyOutcome::NoChange { questions, .. })
        | Some(ModifyOutcome::NeedsClarification {
            questions,
            persisted: true,
        }) => questions,
        // Parser/resolver suggestions are not stored in the conversation.
        Some(ModifyOutcome::NeedsClarification {
            persisted: false, ..
        }) => return Review::default(),
        _ => return Review::default(),
    };
    let several = questions.len() > 1;
    let mut buttons = Vec::new();
    for (n, q) in questions.iter().enumerate() {
        for (label, key) in q.options.iter().zip(&q.option_keys) {
            buttons.push(Button {
                label: if several {
                    format!("{}: {label}", n + 1)
                } else {
                    label.clone()
                },
                choice: Choice::Answer {
                    question_id: q.id.clone(),
                    option_key: key.clone(),
                },
            });
        }
    }
    Review {
        expected: None,
        buttons,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use protopie_api::{FileDiff, QuestionSummary};

    fn response(outcome: ModifyOutcome) -> ModifyProjectResponse {
        ModifyProjectResponse {
            reply: String::new(),
            outcome: Some(outcome),
        }
    }

    fn question(id: &str) -> QuestionSummary {
        QuestionSummary {
            id: id.into(),
            prompt: "Where?".into(),
            options: vec!["A new page".into(), "Leave unlinked".into()],
            option_keys: vec!["new_page".into(), "unlinked".into()],
            takes_text: false,
            blocking: false,
        }
    }

    #[test]
    fn a_preview_offers_apply_bound_to_its_revision_and_plan() {
        let review = review_for(&response(ModifyOutcome::Preview {
            plan_id: "p1".into(),
            base_revision: 4,
            changed_files: vec!["a".into()],
            diffs: vec![FileDiff {
                path: "a".into(),
                diff: String::new(),
            }],
            questions: vec![question("q1")],
        }));
        assert_eq!(
            review.expected,
            Some(Expected {
                revision: 4,
                plan_id: "p1".into()
            })
        );
        // Follow-up questions wait until the change is applied.
        assert_eq!(
            review.buttons.iter().map(|b| &b.choice).collect::<Vec<_>>(),
            [&Choice::Apply, &Choice::Discard]
        );
    }

    #[test]
    fn question_only_preview_requires_continue_before_options_can_be_answered() {
        let review = review_for(&response(ModifyOutcome::Preview {
            plan_id: "question-plan".into(),
            base_revision: 7,
            changed_files: vec![],
            diffs: vec![],
            questions: vec![question("shape")],
        }));
        assert_eq!(review.buttons[0].label, "Continue");
        assert_eq!(review.buttons[0].choice, Choice::Apply);
        assert_eq!(review.buttons[1].choice, Choice::Discard);
        assert_eq!(review.expected.as_ref().unwrap().revision, 7);
        assert_eq!(review.expected.as_ref().unwrap().plan_id, "question-plan");
    }

    #[test]
    fn questions_become_structured_option_buttons() {
        let review = review_for(&response(ModifyOutcome::Applied {
            plan_id: "p".into(),
            changed_files: vec![],
            questions: vec![question("q1")],
        }));
        assert!(review.expected.is_none());
        assert_eq!(review.buttons[0].label, "A new page");
        assert_eq!(
            review.buttons[1].choice,
            Choice::Answer {
                question_id: "q1".into(),
                option_key: "unlinked".into()
            }
        );
        let two_applied = review_for(&response(ModifyOutcome::Applied {
            plan_id: "p2".into(),
            changed_files: vec![],
            questions: vec![question("q1"), question("q2")],
        }));
        assert_eq!(two_applied.buttons.len(), 4);
        assert_eq!(two_applied.buttons[2].label, "2: A new page");
        let two = review_for(&response(ModifyOutcome::NeedsClarification {
            questions: vec![question("q1"), question("q2")],
            persisted: false,
        }));
        assert!(two.buttons.is_empty());
        let stored = review_for(&response(ModifyOutcome::NeedsClarification {
            questions: vec![question("shape")],
            persisted: true,
        }));
        assert_eq!(stored.buttons.len(), 2);
        assert_eq!(stored.buttons[0].label, "A new page");
    }

    #[test]
    fn conflicts_unsupported_and_plain_replies_offer_nothing() {
        for outcome in [
            ModifyOutcome::Conflict { reason: "x".into() },
            ModifyOutcome::Unsupported {
                explanation: "x".into(),
                reason: None,
                span: None,
            },
        ] {
            assert_eq!(review_for(&response(outcome)), Review::default());
        }
        let old = ModifyProjectResponse {
            reply: "hi".into(),
            outcome: None,
        };
        assert_eq!(review_for(&old), Review::default());
        // A nonpersisted question that takes text has no option buttons.
        let mut q = question("q");
        q.options.clear();
        q.option_keys.clear();
        q.takes_text = true;
        let r = review_for(&response(ModifyOutcome::NeedsClarification {
            questions: vec![q],
            persisted: false,
        }));
        assert!(r.buttons.is_empty());
    }
}
