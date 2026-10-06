//! Pure syntax parsing for bounded UI requests.

use serde::{Deserialize, Serialize};

/// Half-open UTF-8 byte range into the original prompt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ParseOutcome {
    Parsed(Request),
    NeedsClarification(Clarification),
    Unsupported(Unsupported),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Request {
    CreateNavigation(NavigationCreation),
    AddLabelledItem(LabelledAddition),
    CreatePage(PageCreation),
    Style(StyleRequest),
    /// `add a <role> [called <label>] [<relation> <anchor>]`.
    AddElement(ElementAddition),
    /// `move the <role> <relation> <anchor>`.
    MoveElement(ElementMove),
    /// `share the <label> [across pages]`.
    ShareState(StateSharing),
}

/// Request to share one piece of state through a Solid context. The parser
/// carries only what the prompt says: the state's label and, optionally, its
/// scope. Shape, initial value and consumers are never invented here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StateSharing {
    pub label: Label,
    /// `None` preserves an omitted scope for the resolver to ask about.
    pub scope: Option<ShareScope>,
    /// Complete meaningful command, excluding surrounding whitespace.
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ShareScope {
    /// `across pages`: `span` covers those two words in the original prompt.
    AllPages { span: Span },
}

/// Request to add one element, e.g. `Add a form below hero`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ElementAddition {
    pub role: ElementRole,
    /// The role phrase (`form`, `hero section`) in the original prompt.
    pub role_span: Span,
    /// Display label from `called` / `named` / a quoted label. Only a button
    /// accepts one; for other roles the parser rejects it.
    pub label: Option<Label>,
    /// Where the element goes, relative to an anchor. A positioning request,
    /// not a selector relation. `None` leaves the position to the planner.
    pub position: Option<PositionClause>,
    /// Complete meaningful command, excluding surrounding whitespace.
    pub span: Span,
}

/// Request to move an existing element relative to an anchor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ElementMove {
    pub selector: RoleSelector,
    pub position: PositionClause,
    /// Complete meaningful command, excluding surrounding whitespace.
    pub span: Span,
}

/// A positioning request: a relation and the role it is relative to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PositionClause {
    pub relation: PositionRelation,
    pub anchor: RoleSelector,
    /// Relation phrase through the anchor, in the original prompt.
    pub span: Span,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PositionRelation {
    Above,
    Below,
    LeftOf,
    RightOf,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StyleRequest {
    pub selector: RoleSelector,
    pub change: StyleChange,
    /// Complete meaningful command, excluding surrounding whitespace.
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoleSelector {
    pub role: ElementRole,
    /// Optional syntactic constraint, not a resolved project element.
    pub relation: Option<SelectorRelation>,
    /// Role and any relation phrase in the original prompt.
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SelectorRelation {
    Below {
        anchor: Box<RoleSelector>,
        span: Span,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ElementRole {
    Image,
    Form,
    Hero,
    Footer,
    Button,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum StyleChange {
    RoundedCorners,
    FullWidth,
    Padding(PaddingChange),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PaddingChange {
    Increase,
    Decrease,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PageCreation {
    pub label: Label,
    /// Complete meaningful command, excluding surrounding whitespace.
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LabelledAddition {
    pub label: Label,
    /// None preserves the omitted container for a future resolver.
    pub target: Option<ContainerTarget>,
    /// Complete meaningful command, excluding surrounding whitespace.
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Label {
    pub text: String,
    /// Interior label text in the original prompt; no surrounding whitespace.
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ContainerTarget {
    TopNavigation { span: Span },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NavigationCreation {
    pub position: NavigationPosition,
    /// Span of the complete recognized command, excluding surrounding whitespace.
    pub span: Span,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NavigationPosition {
    Top,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Clarification {
    pub reason: ClarificationReason,
    pub span: Span,
    pub quoted_form_suggestion: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ClarificationReason {
    AmbiguousLabelBoundary,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Unsupported {
    pub reason: UnsupportedReason,
    pub span: Span,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum UnsupportedReason {
    EmptyInput,
    UnrecognizedInput,
    IncompleteInput,
    NegatedRequest,
    UnsupportedTail,
}

struct Token<'a> {
    text: &'a str,
    span: Span,
}

fn tokens(input: &str) -> Vec<Token<'_>> {
    let mut result = Vec::new();
    let mut start = None;
    for (index, character) in input.char_indices() {
        if character.is_whitespace() {
            if let Some(begin) = start.take() {
                result.push(Token {
                    text: &input[begin..index],
                    span: Span {
                        start: begin,
                        end: index,
                    },
                });
            }
        } else if start.is_none() {
            start = Some(index);
        }
    }
    if let Some(begin) = start {
        result.push(Token {
            text: &input[begin..],
            span: Span {
                start: begin,
                end: input.len(),
            },
        });
    }
    result
}

fn token_matches(token: &Token<'_>, expected: &str, terminal: bool) -> bool {
    let text = if terminal {
        token.text.strip_suffix('.').unwrap_or(token.text)
    } else {
        token.text
    };
    text.eq_ignore_ascii_case(expected)
}

fn labelled_target(
    tokens: &[Token<'_>],
    index: usize,
    span: Span,
) -> Result<ContainerTarget, ParseOutcome> {
    let unsupported = |reason, span| ParseOutcome::Unsupported(Unsupported { reason, span });
    if index + 1 == tokens.len()
        || (index + 2 == tokens.len() && token_matches(&tokens[index + 1], "top", false))
    {
        return Err(unsupported(UnsupportedReason::IncompleteInput, span));
    }
    if index + 2 >= tokens.len()
        || !token_matches(&tokens[index + 1], "top", false)
        || !token_matches(&tokens[index + 2], "nav", false)
    {
        return Err(unsupported(UnsupportedReason::UnrecognizedInput, span));
    }
    if index + 3 < tokens.len() {
        return Err(unsupported(
            UnsupportedReason::UnsupportedTail,
            Span {
                start: tokens[index + 3].span.start,
                end: span.end,
            },
        ));
    }
    Ok(ContainerTarget::TopNavigation {
        span: Span {
            start: tokens[index + 1].span.start,
            end: tokens[index + 2].span.end,
        },
    })
}

fn quoted_label(
    input: &str,
    open: usize,
    command_span: Span,
) -> Result<(Label, usize), ParseOutcome> {
    let mut text = String::new();
    let mut chars = input[open + 1..].char_indices();
    while let Some((relative, character)) = chars.next() {
        let position = open + 1 + relative;
        match character {
            '"' => {
                if position == open + 1 {
                    return Err(ParseOutcome::Unsupported(Unsupported {
                        reason: UnsupportedReason::IncompleteInput,
                        span: command_span,
                    }));
                }
                return Ok((
                    Label {
                        text,
                        span: Span {
                            start: open + 1,
                            end: position,
                        },
                    },
                    position + 1,
                ));
            }
            '\\' => match chars.next() {
                Some((_, escaped @ ('"' | '\\'))) => text.push(escaped),
                Some((offset, escaped)) => {
                    return Err(ParseOutcome::Unsupported(Unsupported {
                        reason: UnsupportedReason::UnrecognizedInput,
                        span: Span {
                            start: position,
                            end: open + 1 + offset + escaped.len_utf8(),
                        },
                    }))
                }
                None => {
                    return Err(ParseOutcome::Unsupported(Unsupported {
                        reason: UnsupportedReason::IncompleteInput,
                        span: command_span,
                    }))
                }
            },
            _ => text.push(character),
        }
    }
    Err(ParseOutcome::Unsupported(Unsupported {
        reason: UnsupportedReason::IncompleteInput,
        span: command_span,
    }))
}

fn parse_page_creation(input: &str, tokens: &[Token<'_>], span: Span) -> Option<ParseOutcome> {
    if tokens.len() < 2
        || !token_matches(&tokens[0], "add", false)
        || !token_matches(&tokens[1], "a", false)
    {
        return None;
    }
    // The existing `Add a navigation` near match belongs to navigation grammar.
    if tokens.len() == 3 && token_matches(&tokens[2], "navigation", false) {
        return None;
    }
    let unsupported = |reason, span| ParseOutcome::Unsupported(Unsupported { reason, span });
    if tokens.len() == 2 {
        return Some(unsupported(UnsupportedReason::IncompleteInput, span));
    }

    if input[tokens[2].span.start..].starts_with('"') {
        let (label, close) = match quoted_label(input, tokens[2].span.start, span) {
            Ok(value) => value,
            Err(outcome) => return Some(outcome),
        };
        if close < span.end && !input[close..].starts_with(char::is_whitespace) {
            return Some(unsupported(UnsupportedReason::UnrecognizedInput, span));
        }
        let remaining: Vec<_> = tokens
            .iter()
            .filter(|token| token.span.start >= close)
            .collect();
        if remaining.is_empty() {
            return Some(unsupported(UnsupportedReason::IncompleteInput, span));
        }
        if !token_matches(remaining[0], "page", false) {
            return Some(unsupported(UnsupportedReason::UnrecognizedInput, span));
        }
        if remaining.len() > 1 {
            return Some(unsupported(
                UnsupportedReason::UnsupportedTail,
                Span {
                    start: remaining[1].span.start,
                    end: span.end,
                },
            ));
        }
        return Some(ParseOutcome::Parsed(Request::CreatePage(PageCreation {
            label,
            span,
        })));
    }

    // Punctuation (ASCII or not) on the final page noun, or a plural `pages`, is
    // malformed page syntax, not a label. Applies even when no label precedes
    // the noun.
    let last = tokens.last().unwrap();
    let stem = last.text.trim_end_matches(|c: char| !c.is_alphanumeric());
    if (stem.eq_ignore_ascii_case("page") && last.text.len() != stem.len())
        || stem.eq_ignore_ascii_case("pages")
    {
        return Some(unsupported(UnsupportedReason::UnrecognizedInput, span));
    }
    // A final page noun distinguishes this form from a general labelled addition.
    let page_index = if token_matches(tokens.last().unwrap(), "page", false) {
        Some(tokens.len() - 1)
    } else {
        tokens
            .iter()
            .enumerate()
            .skip(2)
            .find(|(_, token)| token_matches(token, "page", false))
            .map(|(index, _)| index)
    };
    let Some(page_index) = page_index else {
        return None;
    };
    if page_index == 2 {
        return Some(unsupported(UnsupportedReason::IncompleteInput, span));
    }
    if tokens[2..page_index]
        .iter()
        .any(|token| token.text.contains(['"', '\\', '\'', '“', '‘']))
    {
        return Some(unsupported(UnsupportedReason::UnrecognizedInput, span));
    }
    if let Some(index) = tokens[2..page_index]
        .iter()
        .position(|token| token_matches(token, "and", false) || token_matches(token, "then", false))
    {
        return Some(unsupported(
            UnsupportedReason::UnsupportedTail,
            Span {
                start: tokens[index + 2].span.start,
                end: span.end,
            },
        ));
    }
    if page_index + 1 < tokens.len() {
        return Some(unsupported(
            UnsupportedReason::UnsupportedTail,
            Span {
                start: tokens[page_index + 1].span.start,
                end: span.end,
            },
        ));
    }
    let label_span = Span {
        start: tokens[2].span.start,
        end: tokens[page_index - 1].span.end,
    };
    Some(ParseOutcome::Parsed(Request::CreatePage(PageCreation {
        label: Label {
            text: input[label_span.start..label_span.end].to_string(),
            span: label_span,
        },
        span,
    })))
}

/// `Share the <label> [across pages]`. The label is a double-quoted literal or
/// the unquoted words before the first `across`; only the exact clause
/// `across pages` states a scope, and an omitted scope is preserved.
fn parse_state_sharing(input: &str, tokens: &[Token<'_>], span: Span) -> Option<ParseOutcome> {
    if !token_matches(&tokens[0], "share", false) {
        return None;
    }
    let unsupported = |reason, span| Some(ParseOutcome::Unsupported(Unsupported { reason, span }));
    if tokens.len() == 1 {
        return unsupported(UnsupportedReason::IncompleteInput, span);
    }
    if !token_matches(&tokens[1], "the", false) {
        return unsupported(UnsupportedReason::UnrecognizedInput, span);
    }
    if tokens.len() == 2 {
        return unsupported(UnsupportedReason::IncompleteInput, span);
    }
    let label_start = tokens[2].span.start;
    let (label, clause_from) = if input[label_start..].starts_with('"') {
        let (label, close) = match quoted_label(input, label_start, span) {
            Ok(value) => value,
            Err(outcome) => return Some(outcome),
        };
        if close < span.end && !input[close..].starts_with(char::is_whitespace) {
            return unsupported(UnsupportedReason::UnrecognizedInput, span);
        }
        let from = tokens
            .iter()
            .position(|token| token.span.start >= close)
            .unwrap_or(tokens.len());
        if from < tokens.len() && !token_matches(&tokens[from], "across", false) {
            return unsupported(
                UnsupportedReason::UnsupportedTail,
                Span {
                    start: tokens[from].span.start,
                    end: span.end,
                },
            );
        }
        (label, from)
    } else {
        let across = tokens
            .iter()
            .skip(2)
            .position(|token| token_matches(token, "across", false))
            .map(|index| index + 2)
            .unwrap_or(tokens.len());
        if across == 2 {
            return unsupported(UnsupportedReason::IncompleteInput, span);
        }
        if tokens[2..across]
            .iter()
            .any(|token| token.text.contains(['"', '\\', '\'', '“', '‘']))
        {
            return unsupported(UnsupportedReason::UnrecognizedInput, span);
        }
        if let Some(index) = tokens[2..across]
            .iter()
            .position(|t| token_matches(t, "and", false) || token_matches(t, "then", false))
        {
            return unsupported(
                UnsupportedReason::UnsupportedTail,
                Span {
                    start: tokens[index + 2].span.start,
                    end: span.end,
                },
            );
        }
        let label_span = Span {
            start: label_start,
            end: tokens[across - 1].span.end,
        };
        (
            Label {
                text: input[label_span.start..label_span.end].to_string(),
                span: label_span,
            },
            across,
        )
    };
    if clause_from == tokens.len() {
        return Some(ParseOutcome::Parsed(Request::ShareState(StateSharing {
            label,
            scope: None,
            span,
        })));
    }
    // `tokens[clause_from]` is `across`.
    if clause_from + 1 == tokens.len() {
        return unsupported(UnsupportedReason::IncompleteInput, span);
    }
    let noun = &tokens[clause_from + 1];
    if !token_matches(noun, "pages", false) {
        let stem = noun
            .text
            .trim_end_matches(|c: char| c.is_ascii_punctuation());
        return if stem.eq_ignore_ascii_case("pages") {
            unsupported(UnsupportedReason::UnrecognizedInput, span)
        } else {
            unsupported(
                UnsupportedReason::UnrecognizedInput,
                Span {
                    start: tokens[clause_from].span.start,
                    end: span.end,
                },
            )
        };
    }
    if clause_from + 2 < tokens.len() {
        return unsupported(
            UnsupportedReason::UnsupportedTail,
            Span {
                start: tokens[clause_from + 2].span.start,
                end: span.end,
            },
        );
    }
    Some(ParseOutcome::Parsed(Request::ShareState(StateSharing {
        label,
        scope: Some(ShareScope::AllPages {
            span: Span {
                start: tokens[clause_from].span.start,
                end: noun.span.end,
            },
        }),
        span,
    })))
}

fn parse_labelled_addition(input: &str, tokens: &[Token<'_>], span: Span) -> Option<ParseOutcome> {
    if !token_matches(&tokens[0], "add", false) || tokens.len() < 2 {
        return None;
    }
    // These starts belong to the navigation grammar, including its near matches.
    if ["top", "nav", "navigation", "navigational"]
        .iter()
        .any(|word| token_matches(&tokens[1], word, false))
        || (tokens.len() >= 3
            && token_matches(&tokens[1], "a", false)
            && token_matches(&tokens[2], "navigation", false))
    {
        return None;
    }

    let unsupported = |reason, span| ParseOutcome::Unsupported(Unsupported { reason, span });
    let label_start = if token_matches(&tokens[1], "an", false)
        && tokens
            .get(2)
            .is_some_and(|token| token_matches(token, "item", false))
    {
        if tokens.len() < 4 {
            return Some(unsupported(UnsupportedReason::IncompleteInput, span));
        }
        if token_matches(&tokens[3], "called", false) || token_matches(&tokens[3], "named", false) {
            4
        } else {
            1
        }
    } else {
        1
    };
    if label_start == tokens.len() {
        return Some(unsupported(UnsupportedReason::IncompleteInput, span));
    }

    if tokens[label_start].text.starts_with(['\'', '“', '‘']) {
        return Some(unsupported(UnsupportedReason::UnrecognizedInput, span));
    }

    if input[tokens[label_start].span.start..].starts_with('"') {
        let (label, close) = match quoted_label(input, tokens[label_start].span.start, span) {
            Ok(value) => value,
            Err(outcome) => return Some(outcome),
        };
        if close < span.end && !input[close..].starts_with(char::is_whitespace) {
            return Some(unsupported(UnsupportedReason::UnrecognizedInput, span));
        }
        let remaining: Vec<_> = tokens
            .iter()
            .filter(|token| token.span.start >= close)
            .collect();
        let target = if remaining.is_empty() {
            None
        } else {
            if !token_matches(remaining[0], "to", false) {
                return Some(unsupported(
                    UnsupportedReason::UnsupportedTail,
                    Span {
                        start: remaining[0].span.start,
                        end: span.end,
                    },
                ));
            }
            let rest: Vec<_> = remaining
                .into_iter()
                .map(|token| Token {
                    text: token.text,
                    span: token.span,
                })
                .collect();
            match labelled_target(&rest, 0, span) {
                Ok(target) => Some(target),
                Err(outcome) => return Some(outcome),
            }
        };
        return Some(ParseOutcome::Parsed(Request::AddLabelledItem(
            LabelledAddition {
                label,
                target,
                span,
            },
        )));
    }

    let delimiter = tokens
        .iter()
        .enumerate()
        .skip(label_start)
        .rev()
        .find(|(index, token)| {
            token_matches(token, "to", false)
                && tokens
                    .get(index + 1)
                    .is_some_and(|next| token_matches(next, "top", false))
                && tokens
                    .get(index + 2)
                    .is_some_and(|next| token_matches(next, "nav", false))
        })
        .map(|(index, _)| index);
    let delimiter = delimiter.or_else(|| {
        tokens
            .iter()
            .enumerate()
            .skip(label_start)
            .find(|(_, token)| token_matches(token, "to", false))
            .map(|(index, _)| index)
    });
    let label_end = delimiter.unwrap_or(tokens.len());
    if label_end == label_start {
        return Some(unsupported(UnsupportedReason::IncompleteInput, span));
    }
    if tokens[label_start..label_end]
        .iter()
        .any(|token| token.text.contains(['"', '\\']))
    {
        return Some(unsupported(UnsupportedReason::UnrecognizedInput, span));
    }
    if let Some(index) = tokens[label_start..label_end]
        .iter()
        .position(|token| token_matches(token, "and", false) || token_matches(token, "then", false))
    {
        return Some(unsupported(
            UnsupportedReason::UnsupportedTail,
            Span {
                start: tokens[index + label_start].span.start,
                end: span.end,
            },
        ));
    }

    let target = if let Some(index) = delimiter {
        match labelled_target(tokens, index, span) {
            Ok(target) => Some(target),
            Err(outcome) => return Some(outcome),
        }
    } else {
        None
    };
    let label_span = Span {
        start: tokens[label_start].span.start,
        end: tokens[label_end - 1].span.end,
    };
    Some(ParseOutcome::Parsed(Request::AddLabelledItem(
        LabelledAddition {
            label: Label {
                text: input[label_span.start..label_span.end].to_string(),
                span: label_span,
            },
            target,
            span,
        },
    )))
}

/// Narrow GUI-observed alias for a labelled item in the existing top navigation.
/// Only a double-quoted label and an explicit `to top nav/navigation` target
/// are accepted, so a later clause cannot be mistaken for label text.
fn parse_link_alias(input: &str, tokens: &[Token<'_>], span: Span) -> Option<ParseOutcome> {
    if tokens.len() < 3
        || !token_matches(&tokens[0], "add", false)
        || !token_matches(&tokens[1], "link", false)
        || !input[tokens[2].span.start..].starts_with('"')
    {
        return None;
    }
    let unsupported = |reason, span| ParseOutcome::Unsupported(Unsupported { reason, span });
    let (label, close) = match quoted_label(input, tokens[2].span.start, span) {
        Ok(value) => value,
        Err(outcome) => return Some(outcome),
    };
    if close < span.end && !input[close..].starts_with(char::is_whitespace) {
        return Some(unsupported(UnsupportedReason::UnrecognizedInput, span));
    }
    let remaining: Vec<_> = tokens
        .iter()
        .filter(|token| token.span.start >= close)
        .collect();
    if remaining.len() < 3 {
        return Some(unsupported(UnsupportedReason::IncompleteInput, span));
    }
    if !token_matches(remaining[0], "to", false)
        || !token_matches(remaining[1], "top", false)
        || !(token_matches(remaining[2], "nav", false)
            || token_matches(remaining[2], "navigation", false))
    {
        return Some(unsupported(UnsupportedReason::UnrecognizedInput, span));
    }
    if remaining.len() > 3 {
        return Some(unsupported(
            UnsupportedReason::UnsupportedTail,
            Span {
                start: remaining[3].span.start,
                end: span.end,
            },
        ));
    }
    Some(ParseOutcome::Parsed(Request::AddLabelledItem(
        LabelledAddition {
            label,
            target: Some(ContainerTarget::TopNavigation {
                span: Span {
                    start: remaining[1].span.start,
                    end: remaining[2].span.end,
                },
            }),
            span,
        },
    )))
}

/// Role named by a single word (style grammar and anchors).
fn role_word(text: &str) -> Option<ElementRole> {
    Some(match text.to_ascii_lowercase().as_str() {
        "image" => ElementRole::Image,
        "form" => ElementRole::Form,
        "hero" => ElementRole::Hero,
        "footer" => ElementRole::Footer,
        "button" => ElementRole::Button,
        _ => return None,
    })
}

/// Role phrase starting at `index` and its token count: a role word, or
/// `hero section`. `terminal` allows a final period on the last token.
fn role_at(tokens: &[Token<'_>], index: usize) -> Option<(ElementRole, usize)> {
    let token = tokens.get(index)?;
    let is_last = |count: usize| index + count == tokens.len();
    if token_matches(token, "hero", false)
        && tokens
            .get(index + 1)
            .is_some_and(|next| token_matches(next, "section", is_last(2)))
    {
        return Some((ElementRole::Hero, 2));
    }
    let stem = if is_last(1) {
        token.text.strip_suffix('.').unwrap_or(token.text)
    } else {
        token.text
    };
    role_word(stem).map(|role| (role, 1))
}

/// Relation phrase starting at `index` and its token count. `to the left of`
/// and `to the right of` are the long forms of `left of` and `right of`.
fn relation_at(tokens: &[Token<'_>], index: usize) -> Option<(PositionRelation, usize)> {
    let words = |expected: &[&str]| {
        expected.iter().enumerate().all(|(offset, word)| {
            tokens
                .get(index + offset)
                .is_some_and(|token| token_matches(token, word, false))
        })
    };
    const PHRASES: &[(&[&str], PositionRelation)] = &[
        (&["to", "the", "left", "of"], PositionRelation::LeftOf),
        (&["to", "the", "right", "of"], PositionRelation::RightOf),
        (&["left", "of"], PositionRelation::LeftOf),
        (&["right", "of"], PositionRelation::RightOf),
        (&["below"], PositionRelation::Below),
        (&["above"], PositionRelation::Above),
    ];
    PHRASES
        .iter()
        .find(|(phrase, _)| words(phrase))
        .map(|(phrase, relation)| (*relation, phrase.len()))
}

/// Span of `tokens[from..to]`; a final period of the whole command is
/// sentence punctuation, not part of the phrase.
fn phrase_span(tokens: &[Token<'_>], from: usize, to: usize) -> Span {
    let last = &tokens[to - 1];
    let trim = usize::from(to == tokens.len() && last.text.ends_with('.'));
    Span {
        start: tokens[from].span.start,
        end: last.span.end - trim,
    }
}

fn unsupported_outcome(reason: UnsupportedReason, span: Span) -> ParseOutcome {
    ParseOutcome::Unsupported(Unsupported { reason, span })
}

fn tail_from(tokens: &[Token<'_>], index: usize, span: Span) -> ParseOutcome {
    unsupported_outcome(
        UnsupportedReason::UnsupportedTail,
        Span {
            start: tokens[index].span.start,
            end: span.end,
        },
    )
}

/// `<relation> [the] <role>` at `index`, consuming the rest of the command.
/// `Err` is the outcome to return (incomplete, unknown anchor, or tail).
fn position_clause(
    tokens: &[Token<'_>],
    index: usize,
    span: Span,
) -> Result<PositionClause, ParseOutcome> {
    let (relation, relation_len) =
        relation_at(tokens, index).expect("caller checked for a relation phrase");
    let mut at = index + relation_len;
    if tokens
        .get(at)
        .is_some_and(|token| token_matches(token, "the", false))
    {
        at += 1;
    }
    if at >= tokens.len() {
        return Err(unsupported_outcome(UnsupportedReason::IncompleteInput, span));
    }
    let Some((role, role_len)) = role_at(tokens, at) else {
        return Err(unsupported_outcome(
            UnsupportedReason::UnrecognizedInput,
            span,
        ));
    };
    let end = at + role_len;
    if end < tokens.len() {
        return Err(tail_from(tokens, end, span));
    }
    let anchor_span = phrase_span(tokens, at, end);
    Ok(PositionClause {
        relation,
        anchor: RoleSelector {
            role,
            relation: None,
            span: anchor_span,
        },
        span: Span {
            start: tokens[index].span.start,
            end: anchor_span.end,
        },
    })
}

/// `add|need a|an <role> [called|named <label> | "<label>"] [<relation> [the] <role>]`.
///
/// Returns `None` for anything that is not unmistakably this pattern (so
/// `Add a Contact Us page` and `Add a form to top nav` keep their meaning).
/// Only a button takes a label; elsewhere the clause is an unsupported tail.
fn parse_element_add(input: &str, tokens: &[Token<'_>], span: Span) -> Option<ParseOutcome> {
    if tokens.len() < 3
        || !(token_matches(&tokens[0], "add", false) || token_matches(&tokens[0], "need", false))
        || !(token_matches(&tokens[1], "a", false) || token_matches(&tokens[1], "an", false))
    {
        return None;
    }
    // A final `page` noun belongs to the page grammar (`add a form below hero page`).
    if token_matches(tokens.last().expect("non-empty"), "page", true) {
        return None;
    }
    let (role, role_len) = role_at(tokens, 2)?;
    let role_span = phrase_span(tokens, 2, 2 + role_len);
    let mut at = 2 + role_len;
    let mut label = None;
    let mut label_start = None;
    if let Some(token) = tokens.get(at) {
        let named = token_matches(token, "called", false) || token_matches(token, "named", false);
        let quoted = input[token.span.start..].starts_with('"');
        if named || quoted {
            label_start = Some(token.span.start);
            let first = if named { at + 1 } else { at };
            if first >= tokens.len() {
                return Some(unsupported_outcome(UnsupportedReason::IncompleteInput, span));
            }
            if input[tokens[first].span.start..].starts_with('"') {
                let (parsed, close) = match quoted_label(input, tokens[first].span.start, span) {
                    Ok(value) => value,
                    Err(outcome) => return Some(outcome),
                };
                if close < span.end && !input[close..].starts_with(char::is_whitespace) {
                    return Some(unsupported_outcome(
                        UnsupportedReason::UnrecognizedInput,
                        span,
                    ));
                }
                at = tokens
                    .iter()
                    .position(|t| t.span.start >= close)
                    .unwrap_or(tokens.len());
                label = Some(parsed);
            } else {
                // Unquoted: words up to a relation phrase or the end. A
                // relation word inside a label needs quotes.
                let mut end = first + 1;
                while end < tokens.len() && relation_at(tokens, end).is_none() {
                    end += 1;
                }
                if tokens[first..end]
                    .iter()
                    .any(|t| t.text.contains(['"', '\\']))
                {
                    return Some(unsupported_outcome(UnsupportedReason::UnrecognizedInput, span));
                }
                if let Some(index) = tokens[first..end].iter().position(|t| {
                    token_matches(t, "and", false) || token_matches(t, "then", false)
                }) {
                    return Some(tail_from(tokens, first + index, span));
                }
                let label_span = Span {
                    start: tokens[first].span.start,
                    end: tokens[end - 1].span.end,
                };
                let text = input[label_span.start..label_span.end].to_string();
                // A final period belongs to the sentence, not the label.
                let (text, label_span) = if end == tokens.len() && text.ends_with('.') {
                    let trimmed = text[..text.len() - 1].to_string();
                    let span = Span {
                        start: label_span.start,
                        end: label_span.end - 1,
                    };
                    (trimmed, span)
                } else {
                    (text, label_span)
                };
                if text.is_empty() {
                    return Some(unsupported_outcome(UnsupportedReason::IncompleteInput, span));
                }
                label = Some(Label {
                    text,
                    span: label_span,
                });
                at = end;
            }
        }
    }
    if label.is_some() && role != ElementRole::Button {
        return Some(tail_from(
            tokens,
            tokens
                .iter()
                .position(|t| Some(t.span.start) == label_start)
                .unwrap_or(2 + role_len),
            span,
        ));
    }
    let mut position = None;
    if at < tokens.len() {
        if relation_at(tokens, at).is_some() {
            match position_clause(tokens, at, span) {
                Ok(clause) => position = Some(clause),
                Err(outcome) => return Some(outcome),
            }
        } else if label.is_some() {
            return Some(tail_from(tokens, at, span));
        } else {
            // Not this pattern (`to top nav`, `page`, ...): let the other
            // grammars decide, so nothing is silently dropped here.
            return None;
        }
    }
    Some(ParseOutcome::Parsed(Request::AddElement(ElementAddition {
        role,
        role_span,
        label,
        position,
        span,
    })))
}

/// `move [the] <role> <relation> [the] <role>`.
fn parse_element_move(tokens: &[Token<'_>], span: Span) -> Option<ParseOutcome> {
    if !token_matches(&tokens[0], "move", false) {
        return None;
    }
    let mut at = 1;
    if tokens
        .get(at)
        .is_some_and(|token| token_matches(token, "the", false))
    {
        at += 1;
    }
    if at >= tokens.len() {
        return Some(unsupported_outcome(UnsupportedReason::IncompleteInput, span));
    }
    let Some((role, role_len)) = role_at(tokens, at) else {
        return Some(unsupported_outcome(
            UnsupportedReason::UnrecognizedInput,
            span,
        ));
    };
    let selector = RoleSelector {
        role,
        relation: None,
        span: phrase_span(tokens, at, at + role_len),
    };
    at += role_len;
    if at >= tokens.len() {
        return Some(unsupported_outcome(UnsupportedReason::IncompleteInput, span));
    }
    if relation_at(tokens, at).is_none() {
        return Some(unsupported_outcome(
            UnsupportedReason::UnrecognizedInput,
            span,
        ));
    }
    Some(match position_clause(tokens, at, span) {
        Ok(position) => ParseOutcome::Parsed(Request::MoveElement(ElementMove {
            selector,
            position,
            span,
        })),
        Err(outcome) => outcome,
    })
}

const ROLE_SLOT: &str = "{role}";
const ANCHOR_SLOT: &str = "{anchor}";

fn parse_style(tokens: &[Token<'_>], span: Span) -> Option<ParseOutcome> {
    #[derive(Clone, Copy)]
    struct Pattern {
        words: &'static [&'static str],
        role_index: usize,
        related: bool,
        change: StyleChange,
    }
    // `{role}` matches any element role word; `{anchor}` the role after `below`.
    const PATTERNS: &[Pattern] = &[
        Pattern {
            words: &[
                "give", "the", ROLE_SLOT, "below", ANCHOR_SLOT, "more", "padding",
            ],
            role_index: 2,
            related: true,
            change: StyleChange::Padding(PaddingChange::Increase),
        },
        Pattern {
            words: &[ROLE_SLOT, "needs", "rounded", "corners"],
            role_index: 0,
            related: false,
            change: StyleChange::RoundedCorners,
        },
        Pattern {
            words: &[ROLE_SLOT, "needs", "full", "width"],
            role_index: 0,
            related: false,
            change: StyleChange::FullWidth,
        },
        Pattern {
            words: &["give", "the", ROLE_SLOT, "more", "padding"],
            role_index: 2,
            related: false,
            change: StyleChange::Padding(PaddingChange::Increase),
        },
        Pattern {
            words: &["give", "the", ROLE_SLOT, "less", "padding"],
            role_index: 2,
            related: false,
            change: StyleChange::Padding(PaddingChange::Decrease),
        },
        Pattern {
            words: &["increase", ROLE_SLOT, "padding"],
            role_index: 1,
            related: false,
            change: StyleChange::Padding(PaddingChange::Increase),
        },
        Pattern {
            words: &["decrease", ROLE_SLOT, "padding"],
            role_index: 1,
            related: false,
            change: StyleChange::Padding(PaddingChange::Decrease),
        },
    ];

    let style_start = ["give", "increase", "decrease"]
        .iter()
        .any(|word| token_matches(&tokens[0], word, false))
        || role_word(tokens[0].text).is_some();
    if !style_start {
        return None;
    }
    let slot_matches = |token: &Token<'_>, expected: &str| {
        if expected == ROLE_SLOT || expected == ANCHOR_SLOT {
            role_word(token.text).is_some()
        } else {
            token_matches(token, expected, false)
        }
    };
    for pattern in PATTERNS {
        let compared = tokens.len().min(pattern.words.len());
        if !tokens[..compared]
            .iter()
            .zip(pattern.words.iter())
            .all(|(token, expected)| slot_matches(token, expected))
        {
            continue;
        }
        if tokens.len() < pattern.words.len() {
            return Some(ParseOutcome::Unsupported(Unsupported {
                reason: UnsupportedReason::IncompleteInput,
                span,
            }));
        }
        if tokens.len() > pattern.words.len() {
            return Some(ParseOutcome::Unsupported(Unsupported {
                reason: UnsupportedReason::UnsupportedTail,
                span: Span {
                    start: tokens[pattern.words.len()].span.start,
                    end: span.end,
                },
            }));
        }
        let role_token = &tokens[pattern.role_index];
        let role = role_word(role_token.text).expect("slot matched a role word");
        let relation = pattern.related.then(|| {
            let anchor_token = &tokens[pattern.role_index + 2];
            SelectorRelation::Below {
                anchor: Box::new(RoleSelector {
                    role: role_word(anchor_token.text).expect("slot matched a role word"),
                    relation: None,
                    span: anchor_token.span,
                }),
                span: Span {
                    start: tokens[pattern.role_index + 1].span.start,
                    end: anchor_token.span.end,
                },
            }
        });
        let selector_end = if pattern.related {
            tokens[pattern.role_index + 2].span.end
        } else {
            role_token.span.end
        };
        return Some(ParseOutcome::Parsed(Request::Style(StyleRequest {
            selector: RoleSelector {
                role,
                relation,
                span: Span {
                    start: role_token.span.start,
                    end: selector_end,
                },
            },
            change: pattern.change,
            span,
        })));
    }
    Some(ParseOutcome::Unsupported(Unsupported {
        reason: UnsupportedReason::UnrecognizedInput,
        span,
    }))
}

/// Parses one complete prompt without reading project state or performing IO.
pub fn parse(input: &str) -> ParseOutcome {
    let tokens = tokens(input);
    if tokens.is_empty() {
        return ParseOutcome::Unsupported(Unsupported {
            reason: UnsupportedReason::EmptyInput,
            span: Span {
                start: 0,
                end: input.len(),
            },
        });
    }

    let span = Span {
        start: tokens[0].span.start,
        end: tokens.last().unwrap().span.end,
    };
    if tokens.len() >= 3
        && token_matches(&tokens[0], "do", false)
        && token_matches(&tokens[1], "not", false)
        && token_matches(&tokens[2], "add", false)
    {
        return ParseOutcome::Unsupported(Unsupported {
            reason: UnsupportedReason::NegatedRequest,
            span,
        });
    }

    if tokens.len() == 5
        && token_matches(&tokens[0], "add", false)
        && token_matches(&tokens[1], "navigation", false)
        && token_matches(&tokens[2], "to", false)
        && token_matches(&tokens[3], "top", false)
        && token_matches(&tokens[4], "nav", false)
    {
        return ParseOutcome::NeedsClarification(Clarification {
            reason: ClarificationReason::AmbiguousLabelBoundary,
            span: tokens[1].span,
            quoted_form_suggestion: "Add \"navigation\" to top nav".into(),
        });
    }

    // Longest first so a more specific navigation phrase wins whenever aliases overlap.
    const ALIASES: &[&[&str]] = &[
        &["need", "a", "top", "navigation"],
        &["add", "top", "navigation"],
        &["add", "top", "nav"],
        &["add", "navigation"],
    ];
    for alias in ALIASES {
        let compared = tokens.len().min(alias.len());
        if !tokens[..compared].iter().zip(alias.iter()).enumerate().all(
            |(index, (token, expected))| {
                token_matches(
                    token,
                    expected,
                    index + 1 == alias.len() && tokens.len() == alias.len(),
                )
            },
        ) {
            continue;
        }
        if tokens.len() < alias.len() {
            return ParseOutcome::Unsupported(Unsupported {
                reason: UnsupportedReason::IncompleteInput,
                span,
            });
        }
        if tokens.len() > alias.len() {
            return ParseOutcome::Unsupported(Unsupported {
                reason: UnsupportedReason::UnsupportedTail,
                span: Span {
                    start: tokens[alias.len()].span.start,
                    end: span.end,
                },
            });
        }
        return ParseOutcome::Parsed(Request::CreateNavigation(NavigationCreation {
            position: NavigationPosition::Top,
            span,
        }));
    }
    if let Some(outcome) = parse_style(&tokens, span) {
        return outcome;
    }
    if let Some(outcome) = parse_element_add(input, &tokens, span) {
        return outcome;
    }
    if let Some(outcome) = parse_element_move(&tokens, span) {
        return outcome;
    }
    if let Some(outcome) = parse_state_sharing(input, &tokens, span) {
        return outcome;
    }
    if let Some(outcome) = parse_page_creation(input, &tokens, span) {
        return outcome;
    }
    if let Some(outcome) = parse_link_alias(input, &tokens, span) {
        return outcome;
    }
    if let Some(outcome) = parse_labelled_addition(input, &tokens, span) {
        return outcome;
    }
    ParseOutcome::Unsupported(Unsupported {
        reason: UnsupportedReason::UnrecognizedInput,
        span,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_valid_span(input: &str, span: Span) {
        assert!(
            span.start <= span.end,
            "reversed span {span:?} for {input:?}"
        );
        assert!(
            span.end <= input.len(),
            "out-of-bounds span {span:?} for {input:?}"
        );
        assert!(
            input.is_char_boundary(span.start),
            "invalid start {span:?} for {input:?}"
        );
        assert!(
            input.is_char_boundary(span.end),
            "invalid end {span:?} for {input:?}"
        );
    }

    fn assert_all_spans_valid(input: &str, outcome: &ParseOutcome) {
        match outcome {
            ParseOutcome::Parsed(request) => match request {
                Request::CreateNavigation(navigation) => assert_valid_span(input, navigation.span),
                Request::AddLabelledItem(item) => {
                    assert_valid_span(input, item.span);
                    assert_valid_span(input, item.label.span);
                    if let Some(ContainerTarget::TopNavigation { span }) = item.target {
                        assert_valid_span(input, span);
                    }
                }
                Request::CreatePage(page) => {
                    assert_valid_span(input, page.span);
                    assert_valid_span(input, page.label.span);
                }
                Request::AddElement(add) => {
                    assert_valid_span(input, add.span);
                    assert_valid_span(input, add.role_span);
                    if let Some(label) = &add.label {
                        assert_valid_span(input, label.span);
                    }
                    if let Some(position) = &add.position {
                        assert_valid_span(input, position.span);
                        assert_valid_span(input, position.anchor.span);
                    }
                }
                Request::MoveElement(mv) => {
                    assert_valid_span(input, mv.span);
                    assert_valid_span(input, mv.selector.span);
                    assert_valid_span(input, mv.position.span);
                    assert_valid_span(input, mv.position.anchor.span);
                }
                Request::ShareState(share) => {
                    assert_valid_span(input, share.span);
                    assert_valid_span(input, share.label.span);
                    if let Some(ShareScope::AllPages { span }) = &share.scope {
                        assert_valid_span(input, *span);
                    }
                }
                Request::Style(style) => {
                    assert_valid_span(input, style.span);
                    assert_valid_span(input, style.selector.span);
                    if let Some(SelectorRelation::Below { anchor, span }) = &style.selector.relation
                    {
                        assert_valid_span(input, *span);
                        assert_valid_span(input, anchor.span);
                    }
                }
            },
            ParseOutcome::NeedsClarification(clarification) => {
                assert_valid_span(input, clarification.span);
            }
            ParseOutcome::Unsupported(unsupported) => assert_valid_span(input, unsupported.span),
        }
    }

    #[test]
    fn adversarial_inputs_have_specified_outcomes_and_valid_byte_spans() {
        let cases = [
            (
                "",
                ParseOutcome::Unsupported(Unsupported {
                    reason: UnsupportedReason::EmptyInput,
                    span: Span { start: 0, end: 0 },
                }),
            ),
            (
                "\t \n",
                ParseOutcome::Unsupported(Unsupported {
                    reason: UnsupportedReason::EmptyInput,
                    span: Span { start: 0, end: 3 },
                }),
            ),
            (
                "!!!",
                ParseOutcome::Unsupported(Unsupported {
                    reason: UnsupportedReason::UnrecognizedInput,
                    span: Span { start: 0, end: 3 },
                }),
            ),
            (
                "Add top nav!!",
                ParseOutcome::Unsupported(Unsupported {
                    reason: UnsupportedReason::UnrecognizedInput,
                    span: Span { start: 0, end: 13 },
                }),
            ),
            (
                "🚀",
                ParseOutcome::Unsupported(Unsupported {
                    reason: UnsupportedReason::UnrecognizedInput,
                    span: Span { start: 0, end: 4 },
                }),
            ),
            (
                "Add Café ☕",
                ParseOutcome::Parsed(Request::AddLabelledItem(LabelledAddition {
                    label: Label {
                        text: "Café ☕".into(),
                        span: Span { start: 4, end: 13 },
                    },
                    target: None,
                    span: Span { start: 0, end: 13 },
                })),
            ),
            (
                "Add \"Unclosed ☕",
                ParseOutcome::Unsupported(Unsupported {
                    reason: UnsupportedReason::IncompleteInput,
                    span: Span { start: 0, end: 17 },
                }),
            ),
            (
                "Add \"x\"\" to top nav",
                ParseOutcome::Unsupported(Unsupported {
                    reason: UnsupportedReason::UnrecognizedInput,
                    span: Span { start: 0, end: 19 },
                }),
            ),
            (
                "Add Contact Us to top nav to top nav",
                ParseOutcome::Parsed(Request::AddLabelledItem(LabelledAddition {
                    label: Label {
                        text: "Contact Us to top nav".into(),
                        span: Span { start: 4, end: 25 },
                    },
                    target: Some(ContainerTarget::TopNavigation {
                        span: Span { start: 29, end: 36 },
                    }),
                    span: Span { start: 0, end: 36 },
                })),
            ),
        ];
        for (input, expected) in cases {
            let actual = parse(input);
            assert_eq!(actual, expected, "{input:?}");
            assert_all_spans_valid(input, &actual);
        }
    }

    fn share(label: &str, label_span: (usize, usize), scope: Option<(usize, usize)>, span: (usize, usize)) -> ParseOutcome {
        ParseOutcome::Parsed(Request::ShareState(StateSharing {
            label: Label {
                text: label.into(),
                span: Span { start: label_span.0, end: label_span.1 },
            },
            scope: scope.map(|(start, end)| ShareScope::AllPages { span: Span { start, end } }),
            span: Span { start: span.0, end: span.1 },
        }))
    }

    fn unsupported_at(reason: UnsupportedReason, start: usize, end: usize) -> ParseOutcome {
        ParseOutcome::Unsupported(Unsupported {
            reason,
            span: Span { start, end },
        })
    }

    #[test]
    fn share_the_state_across_pages_preserves_label_scope_and_spans() {
        // `Share the selected doctor across pages`: label 10..25, scope 26..38.
        assert_eq!(
            parse("Share the selected doctor across pages"),
            share("selected doctor", (10, 25), Some((26, 38)), (0, 38))
        );
        assert_eq!(
            parse("  SHARE THE Selected   Doctor ACROSS Pages "),
            share("Selected   Doctor", (12, 29), Some((30, 42)), (2, 42))
        );
    }

    #[test]
    fn share_without_a_scope_preserves_the_omission() {
        assert_eq!(
            parse("Share the selected doctor"),
            share("selected doctor", (10, 25), None, (0, 25))
        );
    }

    #[test]
    fn share_quoted_labels_keep_syntax_words_literal() {
        assert_eq!(
            parse("Share the \"across pages\" across pages"),
            share("across pages", (11, 23), Some((25, 37)), (0, 37))
        );
        assert_eq!(
            parse("Share the \"Café and tea\""),
            share("Café and tea", (11, 24), None, (0, 25))
        );
    }

    #[test]
    fn share_rejects_incomplete_unsupported_and_malformed_forms() {
        for (input, expected) in [
            ("Share", unsupported_at(UnsupportedReason::IncompleteInput, 0, 5)),
            ("Share the", unsupported_at(UnsupportedReason::IncompleteInput, 0, 9)),
            ("Share the across pages", unsupported_at(UnsupportedReason::IncompleteInput, 0, 22)),
            ("Share the doctor across", unsupported_at(UnsupportedReason::IncompleteInput, 0, 23)),
            // Only `the` introduces the label; other scopes are not supported.
            ("Share doctor across pages", unsupported_at(UnsupportedReason::UnrecognizedInput, 0, 25)),
            ("Share the doctor across the app", unsupported_at(UnsupportedReason::UnrecognizedInput, 17, 31)),
            // Punctuation on syntax words is unsupported, as for page requests.
            ("Share the doctor across pages.", unsupported_at(UnsupportedReason::UnrecognizedInput, 0, 30)),
            ("Share the doctor across pages and add a page", unsupported_at(UnsupportedReason::UnsupportedTail, 30, 44)),
            ("Share the doctor and the nurse across pages", unsupported_at(UnsupportedReason::UnsupportedTail, 17, 43)),
            ("Share the \"doctor across pages", unsupported_at(UnsupportedReason::IncompleteInput, 0, 30)),
        ] {
            assert_eq!(parse(input), expected, "{input:?}");
        }
    }

    #[test]
    fn repeated_calls_are_equal_across_outcome_kinds() {
        for input in [
            "Add top nav",
            "Add Contact Us",
            "Add a Contact Us page",
            "Give the form below hero more padding",
            "Add navigation to top nav",
            "Add top nav then move it",
            "Add \"Unclosed ☕",
        ] {
            let first = parse(input);
            let _interleaved = parse("Image needs rounded corners");
            let second = parse(input);
            assert_eq!(first, second, "{input:?}");
            assert_all_spans_valid(input, &first);
        }
    }

    #[test]
    fn review_probes_page_style_precedence_relations_and_tails() {
        assert_eq!(
            parse("  add a Form below hero page  "),
            ParseOutcome::Parsed(Request::CreatePage(PageCreation {
                label: Label {
                    text: "Form below hero".into(),
                    span: Span { start: 8, end: 23 },
                },
                span: Span { start: 2, end: 28 },
            }))
        );
        assert_eq!(
            parse("  GIVE the form below hero more padding  "),
            ParseOutcome::Parsed(Request::Style(StyleRequest {
                selector: RoleSelector {
                    role: ElementRole::Form,
                    relation: Some(SelectorRelation::Below {
                        anchor: Box::new(RoleSelector {
                            role: ElementRole::Hero,
                            relation: None,
                            span: Span { start: 22, end: 26 },
                        }),
                        span: Span { start: 16, end: 26 },
                    }),
                    span: Span { start: 11, end: 26 },
                },
                change: StyleChange::Padding(PaddingChange::Increase),
                span: Span { start: 2, end: 39 },
            }))
        );
        for (input, tail_start, tail_end) in [
            ("  Add a Contact Us page then move it  ", 24, 36),
            ("  IMAGE needs full width and delete it  ", 25, 38),
        ] {
            assert_eq!(
                parse(input),
                ParseOutcome::Unsupported(Unsupported {
                    reason: UnsupportedReason::UnsupportedTail,
                    span: Span {
                        start: tail_start,
                        end: tail_end,
                    },
                }),
                "{input:?}"
            );
        }
    }

    #[test]
    fn image_style_requests_preserve_unresolved_role_and_concept() {
        for (input, change) in [
            ("Image needs rounded corners", StyleChange::RoundedCorners),
            ("Image needs full width", StyleChange::FullWidth),
        ] {
            assert_eq!(
                parse(input),
                ParseOutcome::Parsed(Request::Style(StyleRequest {
                    selector: RoleSelector {
                        role: ElementRole::Image,
                        relation: None,
                        span: Span { start: 0, end: 5 },
                    },
                    change,
                    span: Span {
                        start: 0,
                        end: input.len()
                    },
                })),
                "{input:?}"
            );
        }
    }

    #[test]
    fn form_below_hero_keeps_relation_separate_from_relative_padding() {
        let input = "Give the form below hero more padding";
        assert_eq!(
            parse(input),
            ParseOutcome::Parsed(Request::Style(StyleRequest {
                selector: RoleSelector {
                    role: ElementRole::Form,
                    relation: Some(SelectorRelation::Below {
                        anchor: Box::new(RoleSelector {
                            role: ElementRole::Hero,
                            relation: None,
                            span: Span { start: 20, end: 24 },
                        }),
                        span: Span { start: 14, end: 24 },
                    }),
                    span: Span { start: 9, end: 24 },
                },
                change: StyleChange::Padding(PaddingChange::Increase),
                span: Span {
                    start: 0,
                    end: input.len()
                },
            }))
        );
    }

    #[test]
    fn relative_form_padding_variants_keep_direction_and_source_spans() {
        for (input, selector_start, direction) in [
            ("Give the form less padding", 9, PaddingChange::Decrease),
            ("Increase form padding", 9, PaddingChange::Increase),
            ("Decrease form padding", 9, PaddingChange::Decrease),
        ] {
            assert_eq!(
                parse(input),
                ParseOutcome::Parsed(Request::Style(StyleRequest {
                    selector: RoleSelector {
                        role: ElementRole::Form,
                        relation: None,
                        span: Span {
                            start: selector_start,
                            end: selector_start + 4
                        },
                    },
                    change: StyleChange::Padding(direction),
                    span: Span {
                        start: 0,
                        end: input.len()
                    },
                })),
                "{input:?}"
            );
        }
    }

    #[test]
    fn style_requests_reject_unsupported_properties_and_selectors() {
        for input in [
            "Image needs square corners",
            "Image needs fixed width",
            "Give the form below hero more margin",
            "Give the form less margin",
            "Increase form margin",
            "Decrease form margin",
            "Give the forms less padding",
            "Image needs 12px corners",
            "Give the form below header more padding",
            "Increase form horizontal padding",
        ] {
            assert_eq!(
                parse(input),
                ParseOutcome::Unsupported(Unsupported {
                    reason: UnsupportedReason::UnrecognizedInput,
                    span: Span {
                        start: 0,
                        end: input.len()
                    },
                }),
                "{input:?}"
            );
        }
    }

    #[test]
    fn style_requests_reject_incomplete_phrases() {
        for input in [
            "Image",
            "Image needs",
            "Image needs rounded",
            "Image needs full",
            "Give the",
            "Give the form below",
            "Give the form below hero",
            "Give the form less",
            "Increase",
            "Increase form",
            "Decrease",
            "Decrease form",
        ] {
            assert_eq!(
                parse(input),
                ParseOutcome::Unsupported(Unsupported {
                    reason: UnsupportedReason::IncompleteInput,
                    span: Span {
                        start: 0,
                        end: input.len()
                    },
                }),
                "{input:?}"
            );
        }
    }

    #[test]
    fn style_requests_reject_every_trailing_instruction() {
        for input in [
            "Image needs rounded corners and delete hero",
            "Image needs full width then move it",
            "Give the form below hero more padding and move it",
            "Give the form less padding then delete hero",
            "Increase form padding and delete hero",
            "Decrease form padding then move it",
        ] {
            let tail_start = input.find("and ").or_else(|| input.find("then ")).unwrap();
            assert_eq!(
                parse(input),
                ParseOutcome::Unsupported(Unsupported {
                    reason: UnsupportedReason::UnsupportedTail,
                    span: Span {
                        start: tail_start,
                        end: input.len()
                    },
                }),
                "{input:?}"
            );
        }
    }

    #[test]
    fn review_probes_page_label_boundaries_and_tails() {
        assert_eq!(
            parse("  ADD a Terms of Use page  "),
            ParseOutcome::Parsed(Request::CreatePage(PageCreation {
                label: Label {
                    text: "Terms of Use".into(),
                    span: Span { start: 8, end: 20 },
                },
                span: Span { start: 2, end: 25 },
            }))
        );
        assert_eq!(
            parse("Add a \"Terms and page\" page"),
            ParseOutcome::Parsed(Request::CreatePage(PageCreation {
                label: Label {
                    text: "Terms and page".into(),
                    span: Span { start: 7, end: 21 },
                },
                span: Span { start: 0, end: 27 },
            }))
        );
        assert_eq!(
            parse("Add a \"Contact Us\" page then delete hero"),
            ParseOutcome::Unsupported(Unsupported {
                reason: UnsupportedReason::UnsupportedTail,
                span: Span { start: 24, end: 40 },
            })
        );
    }

    #[test]
    fn page_creation_preserves_display_label_and_original_spans() {
        let input = "  Add a Contact Us page  ";
        assert_eq!(
            parse(input),
            ParseOutcome::Parsed(Request::CreatePage(PageCreation {
                label: Label {
                    text: "Contact Us".into(),
                    span: Span { start: 8, end: 18 },
                },
                span: Span { start: 2, end: 23 },
            }))
        );
    }

    #[test]
    fn lowercase_and_quoted_page_labels_preserve_display_text() {
        for (input, label, label_start, label_end) in [
            ("add a contact us page", "contact us", 6, 16),
            ("Add a \"Contact Us\" page", "Contact Us", 7, 17),
            ("Add a \"Café page\" page", "Café page", 7, 17),
            ("Add a Help page page", "Help page", 6, 15),
        ] {
            assert_eq!(
                parse(input),
                ParseOutcome::Parsed(Request::CreatePage(PageCreation {
                    label: Label {
                        text: label.into(),
                        span: Span {
                            start: label_start,
                            end: label_end
                        },
                    },
                    span: Span {
                        start: 0,
                        end: input.len()
                    },
                })),
                "{input:?}"
            );
        }
    }

    #[test]
    fn page_requests_are_distinct_from_navigation_additions_and_literal_page_words() {
        assert_eq!(
            parse("Add Contact Us to top nav"),
            ParseOutcome::Parsed(Request::AddLabelledItem(LabelledAddition {
                label: Label {
                    text: "Contact Us".into(),
                    span: Span { start: 4, end: 14 }
                },
                target: Some(ContainerTarget::TopNavigation {
                    span: Span { start: 18, end: 25 }
                }),
                span: Span { start: 0, end: 25 },
            }))
        );
        assert_eq!(
            parse("Add \"Contact Us page\" to top nav"),
            ParseOutcome::Parsed(Request::AddLabelledItem(LabelledAddition {
                label: Label {
                    text: "Contact Us page".into(),
                    span: Span { start: 5, end: 20 }
                },
                target: Some(ContainerTarget::TopNavigation {
                    span: Span { start: 25, end: 32 }
                }),
                span: Span { start: 0, end: 32 },
            }))
        );
    }

    #[test]
    fn page_requests_reject_missing_parts_and_unsupported_tails() {
        for input in ["Add a", "Add a page", "Add a \"Contact Us\""] {
            assert_eq!(
                parse(input),
                ParseOutcome::Unsupported(Unsupported {
                    reason: UnsupportedReason::IncompleteInput,
                    span: Span {
                        start: 0,
                        end: input.len()
                    },
                }),
                "{input:?}"
            );
        }
        for (input, start) in [
            ("Add a Contact Us page and delete the hero", 22),
            ("Add a \"Contact Us\" page to top nav", 24),
            ("Add a Contact Us page please", 22),
        ] {
            assert_eq!(
                parse(input),
                ParseOutcome::Unsupported(Unsupported {
                    reason: UnsupportedReason::UnsupportedTail,
                    span: Span {
                        start,
                        end: input.len()
                    },
                }),
                "{input:?}"
            );
        }
    }

    #[test]
    fn page_creation_rejects_non_ascii_punctuation_on_page_noun() {
        for input in [
            "Add a Contact Us page\u{2026}",
            "Add a Contact Us page\u{3002}",
            "Add a Contact Us page\u{2019}",
            "Add a Contact Us page\u{2014}",
        ] {
            assert_eq!(
                parse(input),
                ParseOutcome::Unsupported(Unsupported {
                    reason: UnsupportedReason::UnrecognizedInput,
                    span: Span {
                        start: 0,
                        end: input.len()
                    },
                }),
                "{input:?}"
            );
        }
        assert!(matches!(
            parse("Add a Caf\u{e9} page"),
            ParseOutcome::Parsed(Request::CreatePage(_))
        ));
        assert!(matches!(
            parse("Add a Contact Us page"),
            ParseOutcome::Parsed(Request::CreatePage(_))
        ));
    }

    #[test]
    fn page_creation_rejects_punctuation_on_page_noun() {
        for input in [
            "Add a Contact Us page.",
            "Add a Contact Us page,",
            "Add a Contact Us page!",
        ] {
            assert_eq!(
                parse(input),
                ParseOutcome::Unsupported(Unsupported {
                    reason: UnsupportedReason::UnrecognizedInput,
                    span: Span { start: 0, end: 22 },
                }),
                "{input:?}"
            );
        }
        assert!(matches!(
            parse("Add a Contact Us page"),
            ParseOutcome::Parsed(Request::CreatePage(_))
        ));
        assert!(matches!(
            parse("Add Contact Us"),
            ParseOutcome::Parsed(Request::AddLabelledItem(_))
        ));
    }

    #[test]
    fn page_creation_rejects_repeated_punctuation_missing_labels_and_plural_noun() {
        for input in [
            "Add a Contact Us page..",
            "Add a Contact Us page!?",
            "Add a page.",
            "Add a Page.",
            "Add a Contact Us pages.",
            "Add a Contact Us pages",
        ] {
            assert_eq!(
                parse(input),
                ParseOutcome::Unsupported(Unsupported {
                    reason: UnsupportedReason::UnrecognizedInput,
                    span: Span {
                        start: 0,
                        end: input.len()
                    },
                }),
                "{input:?}"
            );
        }
    }

    #[test]
    fn add_top_nav_creates_top_navigation() {
        assert_eq!(
            parse("Add top nav"),
            ParseOutcome::Parsed(Request::CreateNavigation(NavigationCreation {
                position: NavigationPosition::Top,
                span: Span { start: 0, end: 11 },
            }))
        );
    }

    #[test]
    fn empty_and_whitespace_inputs_are_unsupported() {
        for input in ["", " \t\n"] {
            assert_eq!(
                parse(input),
                ParseOutcome::Unsupported(Unsupported {
                    reason: UnsupportedReason::EmptyInput,
                    span: Span {
                        start: 0,
                        end: input.len()
                    },
                }),
                "{input:?}"
            );
        }
    }

    #[test]
    fn unrelated_text_is_unsupported() {
        let input = "Tell me a joke";
        assert_eq!(
            parse(input),
            ParseOutcome::Unsupported(Unsupported {
                reason: UnsupportedReason::UnrecognizedInput,
                span: Span {
                    start: 0,
                    end: input.len()
                },
            })
        );
    }

    #[test]
    fn navigation_aliases_and_case_share_the_top_navigation_request() {
        for input in [
            "Need a top navigation",
            "add navigation",
            "ADD TOP NAV",
            "Add top navigation",
        ] {
            assert_eq!(
                parse(input),
                ParseOutcome::Parsed(Request::CreateNavigation(NavigationCreation {
                    position: NavigationPosition::Top,
                    span: Span {
                        start: 0,
                        end: input.len()
                    },
                })),
                "{input:?}"
            );
        }
    }

    #[test]
    fn navigation_whitespace_and_terminal_period_preserve_original_span() {
        let input = " \tneed   A\n top\tnavigation.  ";
        assert_eq!(
            parse(input),
            ParseOutcome::Parsed(Request::CreateNavigation(NavigationCreation {
                position: NavigationPosition::Top,
                span: Span {
                    start: 2,
                    end: input.len() - 2
                },
            }))
        );
    }

    #[test]
    fn navigation_near_matches_and_incomplete_commands_are_rejected() {
        for input in [
            "Add top navbar",
            "Add navigational",
            "Need a top navigationbar",
            "Add a navigation",
            "Need top navigation",
            "Add top nav!",
            "Add top nav,",
        ] {
            assert_eq!(
                parse(input),
                ParseOutcome::Unsupported(Unsupported {
                    reason: UnsupportedReason::UnrecognizedInput,
                    span: Span {
                        start: 0,
                        end: input.len()
                    },
                }),
                "{input:?}"
            );
        }
        for input in ["Add", "Add top", "Need", "Need a", "Need a top"] {
            assert_eq!(
                parse(input),
                ParseOutcome::Unsupported(Unsupported {
                    reason: UnsupportedReason::IncompleteInput,
                    span: Span {
                        start: 0,
                        end: input.len()
                    },
                }),
                "{input:?}"
            );
        }
    }

    #[test]
    fn negated_navigation_command_is_rejected() {
        let input = "do not add a nav";
        assert_eq!(
            parse(input),
            ParseOutcome::Unsupported(Unsupported {
                reason: UnsupportedReason::NegatedRequest,
                span: Span {
                    start: 0,
                    end: input.len()
                },
            })
        );
    }

    #[test]
    fn navigation_commands_reject_unsupported_tails() {
        for (input, tail_start) in [
            ("Add top nav and delete the hero", 12),
            ("Add top nav then move it", 12),
            ("add navigation please", 15),
            ("Add navigation top", 15),
        ] {
            assert_eq!(
                parse(input),
                ParseOutcome::Unsupported(Unsupported {
                    reason: UnsupportedReason::UnsupportedTail,
                    span: Span {
                        start: tail_start,
                        end: input.len()
                    },
                }),
                "{input:?}"
            );
        }
    }

    #[test]
    fn review_probes_preserve_rejection_spans() {
        assert_eq!(
            parse("  do not add top nav  "),
            ParseOutcome::Unsupported(Unsupported {
                reason: UnsupportedReason::NegatedRequest,
                span: Span { start: 2, end: 20 },
            })
        );
        assert_eq!(
            parse("Add top nav \tand delete the hero  "),
            ParseOutcome::Unsupported(Unsupported {
                reason: UnsupportedReason::UnsupportedTail,
                span: Span { start: 13, end: 32 },
            })
        );
        assert_eq!(
            parse("Add top navé"),
            ParseOutcome::Unsupported(Unsupported {
                reason: UnsupportedReason::UnrecognizedInput,
                span: Span { start: 0, end: 13 },
            })
        );
    }

    #[test]
    fn labelled_addition_preserves_explicit_container_and_label() {
        let input = "  Add Contact Us to top nav  ";
        assert_eq!(
            parse(input),
            ParseOutcome::Parsed(Request::AddLabelledItem(LabelledAddition {
                label: Label {
                    text: "Contact Us".into(),
                    span: Span { start: 6, end: 16 },
                },
                target: Some(ContainerTarget::TopNavigation {
                    span: Span { start: 20, end: 27 },
                }),
                span: Span { start: 2, end: 27 },
            }))
        );
    }

    #[test]
    fn quoted_link_aliases_keep_label_and_target_utf8_spans() {
        for target in ["top nav", "top navigation"] {
            let input = format!("  Add link \"Café Features\" to {target}  ");
            let ParseOutcome::Parsed(Request::AddLabelledItem(item)) = parse(&input) else {
                panic!("{input:?}: {:?}", parse(&input));
            };
            assert_eq!(item.label.text, "Café Features");
            assert_eq!(
                &input[item.label.span.start..item.label.span.end],
                "Café Features"
            );
            let Some(ContainerTarget::TopNavigation { span }) = &item.target else {
                panic!("{item:?}")
            };
            assert_eq!(&input[span.start..span.end], target);
            assert_eq!(
                &input[item.span.start..item.span.end],
                format!("Add link \"Café Features\" to {target}")
            );
            assert_all_spans_valid(
                &input,
                &ParseOutcome::Parsed(Request::AddLabelledItem(item)),
            );
        }
    }

    #[test]
    fn quoted_link_aliases_reject_tails_and_incomplete_targets() {
        for target in ["top nav", "top navigation"] {
            let input = format!("Add link \"Features\" to {target} and delete the hero");
            let ParseOutcome::Unsupported(unsupported) = parse(&input) else {
                panic!("{input:?}: {:?}", parse(&input))
            };
            assert_eq!(unsupported.reason, UnsupportedReason::UnsupportedTail);
            assert_eq!(
                &input[unsupported.span.start..unsupported.span.end],
                "and delete the hero"
            );
        }
        for input in [
            "Add link \"Features\"",
            "Add link \"Features\" to",
            "Add link \"Features\" to top",
        ] {
            assert!(
                matches!(
                    parse(input),
                    ParseOutcome::Unsupported(Unsupported {
                        reason: UnsupportedReason::IncompleteInput,
                        ..
                    })
                ),
                "{input}"
            );
        }
        assert!(matches!(
            parse("Add link \"Features\" to top navbar"),
            ParseOutcome::Unsupported(Unsupported {
                reason: UnsupportedReason::UnrecognizedInput,
                ..
            })
        ));
    }

    #[test]
    fn labelled_addition_preserves_omitted_container() {
        let input = "Add Contact Us";
        let expected = ParseOutcome::Parsed(Request::AddLabelledItem(LabelledAddition {
            label: Label {
                text: "Contact Us".into(),
                span: Span { start: 4, end: 14 },
            },
            target: None,
            span: Span { start: 0, end: 14 },
        }));
        assert_eq!(parse(input), expected);
        parse("Add top nav");
        assert_eq!(parse(input), expected);
    }

    #[test]
    fn lowercase_label_spelling_is_preserved() {
        let input = "add contact us to top nav";
        assert_eq!(
            parse(input),
            ParseOutcome::Parsed(Request::AddLabelledItem(LabelledAddition {
                label: Label {
                    text: "contact us".into(),
                    span: Span { start: 4, end: 14 },
                },
                target: Some(ContainerTarget::TopNavigation {
                    span: Span { start: 18, end: 25 },
                }),
                span: Span { start: 0, end: 25 },
            }))
        );
    }

    #[test]
    fn labelled_addition_rejects_missing_label_and_incomplete_delimiters() {
        for input in [
            "Add to top nav",
            "Add to",
            "Add Contact Us to",
            "Add Contact Us to top",
        ] {
            assert_eq!(
                parse(input),
                ParseOutcome::Unsupported(Unsupported {
                    reason: UnsupportedReason::IncompleteInput,
                    span: Span {
                        start: 0,
                        end: input.len()
                    },
                }),
                "{input:?}"
            );
        }
    }

    #[test]
    fn labelled_addition_rejects_unsupported_target_tails() {
        for (input, tail_start) in [
            ("Add Contact Us to top nav and delete the hero", 26),
            ("Add Contact Us and delete the hero", 15),
        ] {
            assert_eq!(
                parse(input),
                ParseOutcome::Unsupported(Unsupported {
                    reason: UnsupportedReason::UnsupportedTail,
                    span: Span {
                        start: tail_start,
                        end: input.len()
                    },
                }),
                "{input:?}"
            );
        }
    }

    #[test]
    fn review_probes_label_spans_and_complete_targets() {
        let input = " \tADD Contact\tUs TO TOP NAV  ";
        assert_eq!(
            parse(input),
            ParseOutcome::Parsed(Request::AddLabelledItem(LabelledAddition {
                label: Label {
                    text: "Contact\tUs".into(),
                    span: Span { start: 6, end: 16 },
                },
                target: Some(ContainerTarget::TopNavigation {
                    span: Span { start: 20, end: 27 },
                }),
                span: Span { start: 2, end: 27 },
            }))
        );
        assert_eq!(
            parse("Add Contact Us to footer"),
            ParseOutcome::Unsupported(Unsupported {
                reason: UnsupportedReason::UnrecognizedInput,
                span: Span { start: 0, end: 24 },
            })
        );
        assert_eq!(
            parse("Add Contact Us then remove hero"),
            ParseOutcome::Unsupported(Unsupported {
                reason: UnsupportedReason::UnsupportedTail,
                span: Span { start: 15, end: 31 },
            })
        );
    }

    #[test]
    fn relation_words_inside_unquoted_labels_keep_the_final_container_boundary() {
        for (input, label, label_end, target_start) in [
            ("Add Get in Touch to top nav", "Get in Touch", 16, 20),
            ("Add Terms of Use to top nav", "Terms of Use", 16, 20),
            ("Add Back to School to top nav", "Back to School", 18, 22),
        ] {
            assert_eq!(
                parse(input),
                ParseOutcome::Parsed(Request::AddLabelledItem(LabelledAddition {
                    label: Label {
                        text: label.into(),
                        span: Span {
                            start: 4,
                            end: label_end
                        },
                    },
                    target: Some(ContainerTarget::TopNavigation {
                        span: Span {
                            start: target_start,
                            end: input.len()
                        },
                    }),
                    span: Span {
                        start: 0,
                        end: input.len()
                    },
                })),
                "{input:?}"
            );
        }
    }

    #[test]
    fn quoted_labels_keep_syntax_words_and_conjunctions_literal() {
        for (input, label, end, target) in [
            (
                "Add \"Back to School and navigation\" to top nav",
                "Back to School and navigation",
                34,
                Some(46),
            ),
            ("Add \"top nav\"", "top nav", 12, None),
        ] {
            assert_eq!(
                parse(input),
                ParseOutcome::Parsed(Request::AddLabelledItem(LabelledAddition {
                    label: Label {
                        text: label.into(),
                        span: Span { start: 5, end }
                    },
                    target: target.map(|target_end| ContainerTarget::TopNavigation {
                        span: Span {
                            start: target_end - 7,
                            end: target_end
                        },
                    }),
                    span: Span {
                        start: 0,
                        end: input.len()
                    },
                })),
                "{input:?}"
            );
        }
    }

    #[test]
    fn double_quote_escapes_are_decoded_and_other_escapes_rejected() {
        let input = "Add \"Say \\\"Hello\\\" \\\\ Home\" to top nav";
        assert_eq!(
            parse(input),
            ParseOutcome::Parsed(Request::AddLabelledItem(LabelledAddition {
                label: Label {
                    text: "Say \"Hello\" \\ Home".into(),
                    span: Span { start: 5, end: 26 },
                },
                target: Some(ContainerTarget::TopNavigation {
                    span: Span { start: 31, end: 38 },
                }),
                span: Span { start: 0, end: 38 },
            }))
        );
        assert_eq!(
            parse("Add \"Bad \\n escape\""),
            ParseOutcome::Unsupported(Unsupported {
                reason: UnsupportedReason::UnrecognizedInput,
                span: Span { start: 9, end: 11 },
            })
        );
        for input in ["Add 'Contact Us'", "Add “Contact Us”"] {
            assert_eq!(
                parse(input),
                ParseOutcome::Unsupported(Unsupported {
                    reason: UnsupportedReason::UnrecognizedInput,
                    span: Span {
                        start: 0,
                        end: input.len()
                    },
                })
            );
        }
        for input in [
            "Add Contact \"Us\"",
            "Add \"Contact Us\"suffix",
            "Add Contact \\ Us",
        ] {
            assert_eq!(
                parse(input),
                ParseOutcome::Unsupported(Unsupported {
                    reason: UnsupportedReason::UnrecognizedInput,
                    span: Span {
                        start: 0,
                        end: input.len()
                    },
                })
            );
        }
        let input = "Add \"Unclosed label";
        assert_eq!(
            parse(input),
            ParseOutcome::Unsupported(Unsupported {
                reason: UnsupportedReason::IncompleteInput,
                span: Span {
                    start: 0,
                    end: input.len()
                },
            })
        );
    }

    #[test]
    fn called_and_named_item_forms_parse_the_same_labelled_addition() {
        for (introducer, start, target_start) in [("called", 19, 33), ("named", 18, 32)] {
            let input = format!("Add an item {introducer} Contact Us to top nav");
            assert_eq!(
                parse(&input),
                ParseOutcome::Parsed(Request::AddLabelledItem(LabelledAddition {
                    label: Label {
                        text: "Contact Us".into(),
                        span: Span {
                            start,
                            end: start + 10
                        },
                    },
                    target: Some(ContainerTarget::TopNavigation {
                        span: Span {
                            start: target_start,
                            end: input.len()
                        },
                    }),
                    span: Span {
                        start: 0,
                        end: input.len()
                    },
                }))
            );
        }
    }

    #[test]
    fn navigation_word_is_ambiguous_unquoted_and_literal_when_quoted() {
        let input = "Add navigation to top nav";
        assert_eq!(
            parse(input),
            ParseOutcome::NeedsClarification(Clarification {
                reason: ClarificationReason::AmbiguousLabelBoundary,
                span: Span { start: 4, end: 14 },
                quoted_form_suggestion: "Add \"navigation\" to top nav".into(),
            })
        );
        assert_eq!(
            parse("Add \"navigation\" to top nav"),
            ParseOutcome::Parsed(Request::AddLabelledItem(LabelledAddition {
                label: Label {
                    text: "navigation".into(),
                    span: Span { start: 5, end: 15 }
                },
                target: Some(ContainerTarget::TopNavigation {
                    span: Span { start: 20, end: 27 }
                }),
                span: Span { start: 0, end: 27 },
            }))
        );
    }

    #[test]
    fn unicode_label_and_surrounding_syntax_use_original_byte_spans() {
        let input = "  ADD Café ☕ TO TOP NAV  ";
        let expected = ParseOutcome::Parsed(Request::AddLabelledItem(LabelledAddition {
            label: Label {
                text: "Café ☕".into(),
                span: Span { start: 6, end: 15 },
            },
            target: Some(ContainerTarget::TopNavigation {
                span: Span { start: 19, end: 26 },
            }),
            span: Span { start: 2, end: 26 },
        }));
        assert_eq!(parse(input), expected);
        if let ParseOutcome::Parsed(Request::AddLabelledItem(item)) = expected {
            assert_eq!(
                &input[item.label.span.start..item.label.span.end],
                item.label.text
            );
        }
    }

    #[test]
    fn review_probes_quoted_boundaries_and_repeated_target_phrase() {
        let input = "  ADD \"Café ☕\" TO TOP NAV  ";
        assert_eq!(
            parse(input),
            ParseOutcome::Parsed(Request::AddLabelledItem(LabelledAddition {
                label: Label {
                    text: "Café ☕".into(),
                    span: Span { start: 7, end: 16 },
                },
                target: Some(ContainerTarget::TopNavigation {
                    span: Span { start: 21, end: 28 },
                }),
                span: Span { start: 2, end: 28 },
            }))
        );
        assert_eq!(&input[7..16], "Café ☕");

        let input = "Add Back to top nav to top nav";
        assert_eq!(
            parse(input),
            ParseOutcome::Parsed(Request::AddLabelledItem(LabelledAddition {
                label: Label {
                    text: "Back to top nav".into(),
                    span: Span { start: 4, end: 19 },
                },
                target: Some(ContainerTarget::TopNavigation {
                    span: Span { start: 23, end: 30 },
                }),
                span: Span { start: 0, end: 30 },
            }))
        );

        assert_eq!(
            parse("Add \"Contact Us\"to top nav"),
            ParseOutcome::Unsupported(Unsupported {
                reason: UnsupportedReason::UnrecognizedInput,
                span: Span { start: 0, end: 26 },
            })
        );
        assert_eq!(
            parse("Add \"Contact Us\" then remove hero"),
            ParseOutcome::Unsupported(Unsupported {
                reason: UnsupportedReason::UnsupportedTail,
                span: Span { start: 17, end: 33 },
            })
        );
    }

    // ---------------------------------------------------------------- T8

    fn whole(input: &str) -> Span {
        Span {
            start: 0,
            end: input.len(),
        }
    }

    fn rejected(input: &str, reason: UnsupportedReason, span: Span) {
        let outcome = parse(input);
        assert_eq!(
            outcome,
            ParseOutcome::Unsupported(Unsupported { reason, span }),
            "{input:?}"
        );
        assert_all_spans_valid(input, &outcome);
    }

    fn anchor(role: ElementRole, start: usize, end: usize) -> RoleSelector {
        RoleSelector {
            role,
            relation: None,
            span: Span { start, end },
        }
    }

    #[test]
    fn add_a_role_parses_every_element_with_the_role_span() {
        for (input, role, start, end) in [
            ("Add a form", ElementRole::Form, 6, 10),
            ("add an image", ElementRole::Image, 7, 12),
            ("Need a footer", ElementRole::Footer, 7, 13),
            ("Need a hero section", ElementRole::Hero, 7, 19),
            ("add a hero", ElementRole::Hero, 6, 10),
            ("Add a button", ElementRole::Button, 6, 12),
            ("add a form.", ElementRole::Form, 6, 10),
        ] {
            let outcome = parse(input);
            assert_eq!(
                outcome,
                ParseOutcome::Parsed(Request::AddElement(ElementAddition {
                    role,
                    role_span: Span { start, end },
                    label: None,
                    position: None,
                    span: Span {
                        start: 0,
                        end: input.trim_end().len()
                    },
                })),
                "{input:?}"
            );
            assert_all_spans_valid(input, &outcome);
        }
    }

    #[test]
    fn position_clauses_are_positioning_not_selector_relations() {
        let input = "Add a form below hero";
        assert_eq!(
            parse(input),
            ParseOutcome::Parsed(Request::AddElement(ElementAddition {
                role: ElementRole::Form,
                role_span: Span { start: 6, end: 10 },
                label: None,
                position: Some(PositionClause {
                    relation: PositionRelation::Below,
                    anchor: anchor(ElementRole::Hero, 17, 21),
                    span: Span { start: 11, end: 21 },
                }),
                span: whole(input),
            }))
        );
        for (input, relation) in [
            ("add an image above the footer", PositionRelation::Above),
            ("add a button left of the image", PositionRelation::LeftOf),
            ("add a button right of the image", PositionRelation::RightOf),
            ("add a button to the left of the image", PositionRelation::LeftOf),
            ("add a button to the right of image", PositionRelation::RightOf),
        ] {
            let ParseOutcome::Parsed(Request::AddElement(add)) = parse(input) else {
                panic!("{input:?}")
            };
            assert_eq!(add.position.unwrap().relation, relation, "{input:?}");
            assert_all_spans_valid(input, &parse(input));
        }
        // Anything unknown after `below` is rejected, never guessed.
        let input = "Add a form below the page footer";
        rejected(
            input,
            UnsupportedReason::UnrecognizedInput,
            whole(input),
        );
        let input = "Add a form below the hero section.";
        let ParseOutcome::Parsed(Request::AddElement(add)) = parse(input) else {
            panic!()
        };
        let position = add.position.unwrap();
        assert_eq!(position.anchor.role, ElementRole::Hero);
        assert_eq!(&input[position.anchor.span.start..position.anchor.span.end], "hero section");
        assert_eq!(&input[position.span.start..position.span.end], "below the hero section");
    }

    #[test]
    fn a_button_takes_a_label_in_called_named_and_quoted_forms() {
        for (input, text, start) in [
            ("Add a button called Get a callback", "Get a callback", 20),
            ("add a button named contact us", "contact us", 19),
            ("Add a button called \"Back below\"", "Back below", 21),
            ("Add a button \"Terms of Use\"", "Terms of Use", 14),
        ] {
            let outcome = parse(input);
            let ParseOutcome::Parsed(Request::AddElement(add)) = &outcome else {
                panic!("{input:?}: {outcome:?}")
            };
            let label = add.label.as_ref().unwrap();
            assert_eq!(label.text, text, "{input:?}");
            assert_eq!(label.span.start, start, "{input:?}");
            assert_eq!(&input[label.span.start..label.span.end], text);
            assert!(add.position.is_none());
            assert_all_spans_valid(input, &outcome);
        }
        // Label and position together; a relation word ends an unquoted label.
        let input = "Add a button called Get a callback below the form";
        let ParseOutcome::Parsed(Request::AddElement(add)) = parse(input) else {
            panic!()
        };
        assert_eq!(add.label.unwrap().text, "Get a callback");
        assert_eq!(add.position.unwrap().anchor.role, ElementRole::Form);
        // A quoted label may contain relation words and an escaped quote.
        let input = "Add a button \"Above \\\"all\\\"\" above the form";
        let ParseOutcome::Parsed(Request::AddElement(add)) = parse(input) else {
            panic!("{:?}", parse(input))
        };
        assert_eq!(add.label.unwrap().text, "Above \"all\"");
        assert_eq!(add.position.unwrap().relation, PositionRelation::Above);
    }

    #[test]
    fn only_buttons_take_labels_and_nothing_is_dropped() {
        let input = "Add a form called Contact";
        rejected(
            input,
            UnsupportedReason::UnsupportedTail,
            Span { start: 11, end: input.len() },
        );
        let input = "Add an image \"Cat\"";
        rejected(
            input,
            UnsupportedReason::UnsupportedTail,
            Span { start: 13, end: input.len() },
        );
        let input = "Add a form below hero and delete the footer";
        rejected(
            input,
            UnsupportedReason::UnsupportedTail,
            Span { start: 22, end: input.len() },
        );
        let input = "Add a button called Go and stop";
        rejected(
            input,
            UnsupportedReason::UnsupportedTail,
            Span { start: 23, end: input.len() },
        );
        let input = "Add a form below hero footer";
        rejected(
            input,
            UnsupportedReason::UnsupportedTail,
            Span { start: 22, end: input.len() },
        );
    }

    #[test]
    fn incomplete_and_unknown_positions_are_rejected_not_guessed() {
        for input in ["Add a form below", "Add a form below the", "Add a button called", "Add a button left of"] {
            rejected(input, UnsupportedReason::IncompleteInput, whole(input));
        }
        for input in [
            "Add a form below header",
            "Add a button called \"Go\"x",
        ] {
            let outcome = parse(input);
            assert!(
                matches!(&outcome, ParseOutcome::Unsupported(u) if u.reason == UnsupportedReason::UnrecognizedInput),
                "{input:?}: {outcome:?}"
            );
            assert!(!matches!(outcome, ParseOutcome::Parsed(_)), "{input:?}: {outcome:?}");
            assert_all_spans_valid(input, &outcome);
        }
        rejected("do not add a form", UnsupportedReason::NegatedRequest, whole("do not add a form"));
        // Unquoted unmatched quote inside a label.
        rejected(
            "Add a button called \"Go",
            UnsupportedReason::IncompleteInput,
            whole("Add a button called \"Go"),
        );
    }

    #[test]
    fn other_add_grammars_keep_their_meaning() {
        // Labelled additions: the element words are only labels here.
        for (input, label) in [
            ("Add a form to top nav", "a form"),
            ("Add form to top nav", "form"),
            ("Add Image", "Image"),
            ("add a button to top nav", "a button"),
        ] {
            let ParseOutcome::Parsed(Request::AddLabelledItem(item)) = parse(input) else {
                panic!("{input:?}: {:?}", parse(input))
            };
            assert_eq!(item.label.text, label, "{input:?}");
        }
        // A page named like a role.
        let ParseOutcome::Parsed(Request::CreatePage(page)) = parse("Add a form page") else {
            panic!()
        };
        assert_eq!(page.label.text, "form");
        // An unquoted label that is not a role stays a label.
        assert!(matches!(
            parse("Add a Contact Us page"),
            ParseOutcome::Parsed(Request::CreatePage(_))
        ));
    }

    #[test]
    fn move_requests_parse_target_relation_and_anchor() {
        let input = "Move the form below hero";
        assert_eq!(
            parse(input),
            ParseOutcome::Parsed(Request::MoveElement(ElementMove {
                selector: anchor(ElementRole::Form, 9, 13),
                position: PositionClause {
                    relation: PositionRelation::Below,
                    anchor: anchor(ElementRole::Hero, 20, 24),
                    span: Span { start: 14, end: 24 },
                },
                span: whole(input),
            }))
        );
        for (input, role, relation, anchor_role) in [
            ("move image above the footer", ElementRole::Image, PositionRelation::Above, ElementRole::Footer),
            ("Move the button right of the image", ElementRole::Button, PositionRelation::RightOf, ElementRole::Image),
            ("move the footer to the left of the form.", ElementRole::Footer, PositionRelation::LeftOf, ElementRole::Form),
        ] {
            let outcome = parse(input);
            let ParseOutcome::Parsed(Request::MoveElement(mv)) = &outcome else {
                panic!("{input:?}: {outcome:?}")
            };
            assert_eq!(
                (mv.selector.role, mv.position.relation, mv.position.anchor.role),
                (role, relation, anchor_role),
                "{input:?}"
            );
            assert_all_spans_valid(input, &outcome);
        }
        for input in ["Move", "Move the", "Move the form", "Move the form below", "Move the form below the"] {
            rejected(input, UnsupportedReason::IncompleteInput, whole(input));
        }
        for input in ["Move the header below hero", "Move the form beside hero", "Move the form below header", "Move it below hero"] {
            rejected(input, UnsupportedReason::UnrecognizedInput, whole(input));
        }
        let input = "Move the form below hero then add a footer";
        rejected(input, UnsupportedReason::UnsupportedTail, Span { start: 25, end: input.len() });
    }

    #[test]
    fn style_patterns_cover_every_role_and_keep_rejections() {
        for (input, role) in [
            ("Hero needs rounded corners", ElementRole::Hero),
            ("Footer needs full width", ElementRole::Footer),
            ("Button needs rounded corners", ElementRole::Button),
            ("Give the hero less padding", ElementRole::Hero),
            ("Increase footer padding", ElementRole::Footer),
            ("Decrease button padding", ElementRole::Button),
        ] {
            let outcome = parse(input);
            let ParseOutcome::Parsed(Request::Style(style)) = &outcome else {
                panic!("{input:?}: {outcome:?}")
            };
            assert_eq!(style.selector.role, role, "{input:?}");
            assert_all_spans_valid(input, &outcome);
        }
        let input = "Give the form below image more padding";
        let ParseOutcome::Parsed(Request::Style(style)) = parse(input) else {
            panic!()
        };
        let Some(SelectorRelation::Below { anchor, .. }) = style.selector.relation else {
            panic!()
        };
        assert_eq!(anchor.role, ElementRole::Image);
        rejected("Hero needs square corners", UnsupportedReason::UnrecognizedInput, whole("Hero needs square corners"));
        rejected("Footer needs", UnsupportedReason::IncompleteInput, whole("Footer needs"));
        rejected("Hero banner", UnsupportedReason::UnrecognizedInput, whole("Hero banner"));
    }

    #[test]
    fn element_grammar_adversarial_inputs_never_panic_and_keep_valid_spans() {
        for input in [
            "Add a",
            "Add a form below",
            "add a form below the the hero",
            "Add an",
            "Add a button called",
            "Add a button called \"",
            "Add a button \"\"",
            "Add a button called ☕ below the form",
            "Add a button called \"☕\" below the form",
            "move move move",
            "Move the hero section below the hero section",
            "Add a hero section.",
            "Add a hero section called x",
            "Add a button called below the form",
            "add a button to the left of",
            "add a button to the left of the",
            "Add a form to the left of hero and below the footer",
            "Add a form below hero. ",
            "ADD A FORM BELOW HERO",
            "add\ta\nform\u{a0}below hero",
        ] {
            let first = parse(input);
            assert_eq!(first, parse(input), "{input:?}");
            assert_all_spans_valid(input, &first);
        }
        // Case and whitespace do not change the request.
        let canonical = parse("add a form below hero");
        for variant in ["ADD A FORM BELOW HERO", "  Add   a  Form below   Hero  "] {
            let (ParseOutcome::Parsed(Request::AddElement(a)), ParseOutcome::Parsed(Request::AddElement(b))) =
                (&parse(variant), &canonical)
            else {
                panic!("{variant:?}")
            };
            assert_eq!(
                (a.role, a.position.as_ref().map(|p| (p.relation, p.anchor.role))),
                (b.role, b.position.as_ref().map(|p| (p.relation, p.anchor.role)))
            );
        }
    }
}
