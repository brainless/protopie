//! Pure syntax parsing for bounded UI requests.

/// Half-open UTF-8 byte range into the original prompt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseOutcome {
    Parsed(Request),
    NeedsClarification(Clarification),
    Unsupported(Unsupported),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
    CreateNavigation(NavigationCreation),
    AddLabelledItem(LabelledAddition),
    CreatePage(PageCreation),
    Style(StyleRequest),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StyleRequest {
    pub selector: RoleSelector,
    pub change: StyleChange,
    /// Complete meaningful command, excluding surrounding whitespace.
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoleSelector {
    pub role: ElementRole,
    /// Optional syntactic constraint, not a resolved project element.
    pub relation: Option<SelectorRelation>,
    /// Role and any relation phrase in the original prompt.
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SelectorRelation {
    Below {
        anchor: Box<RoleSelector>,
        span: Span,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ElementRole {
    Image,
    Form,
    Hero,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StyleChange {
    RoundedCorners,
    FullWidth,
    Padding(PaddingChange),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaddingChange {
    Increase,
    Decrease,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageCreation {
    pub label: Label,
    /// Complete meaningful command, excluding surrounding whitespace.
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LabelledAddition {
    pub label: Label,
    /// None preserves the omitted container for a future resolver.
    pub target: Option<ContainerTarget>,
    /// Complete meaningful command, excluding surrounding whitespace.
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Label {
    pub text: String,
    /// Interior label text in the original prompt; no surrounding whitespace.
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContainerTarget {
    TopNavigation { span: Span },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NavigationCreation {
    pub position: NavigationPosition,
    /// Span of the complete recognized command, excluding surrounding whitespace.
    pub span: Span,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NavigationPosition {
    Top,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Clarification {
    pub reason: ClarificationReason,
    pub span: Span,
    pub quoted_form_suggestion: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClarificationReason {
    AmbiguousLabelBoundary,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unsupported {
    pub reason: UnsupportedReason,
    pub span: Span,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

fn parse_style(tokens: &[Token<'_>], span: Span) -> Option<ParseOutcome> {
    #[derive(Clone, Copy)]
    struct Pattern {
        words: &'static [&'static str],
        role_index: usize,
        related: bool,
        change: StyleChange,
    }
    const PATTERNS: &[Pattern] = &[
        Pattern {
            words: &["give", "the", "form", "below", "hero", "more", "padding"],
            role_index: 2,
            related: true,
            change: StyleChange::Padding(PaddingChange::Increase),
        },
        Pattern {
            words: &["image", "needs", "rounded", "corners"],
            role_index: 0,
            related: false,
            change: StyleChange::RoundedCorners,
        },
        Pattern {
            words: &["image", "needs", "full", "width"],
            role_index: 0,
            related: false,
            change: StyleChange::FullWidth,
        },
        Pattern {
            words: &["give", "the", "form", "less", "padding"],
            role_index: 2,
            related: false,
            change: StyleChange::Padding(PaddingChange::Decrease),
        },
        Pattern {
            words: &["increase", "form", "padding"],
            role_index: 1,
            related: false,
            change: StyleChange::Padding(PaddingChange::Increase),
        },
        Pattern {
            words: &["decrease", "form", "padding"],
            role_index: 1,
            related: false,
            change: StyleChange::Padding(PaddingChange::Decrease),
        },
    ];

    let style_start = ["image", "give", "increase", "decrease"]
        .iter()
        .any(|word| token_matches(&tokens[0], word, false));
    if !style_start {
        return None;
    }
    for pattern in PATTERNS {
        let compared = tokens.len().min(pattern.words.len());
        if !tokens[..compared]
            .iter()
            .zip(pattern.words.iter())
            .all(|(token, expected)| token_matches(token, expected, false))
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
        let relation = pattern.related.then(|| {
            let anchor_token = &tokens[pattern.role_index + 2];
            SelectorRelation::Below {
                anchor: Box::new(RoleSelector {
                    role: ElementRole::Hero,
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
                role: if pattern.role_index == 0 {
                    ElementRole::Image
                } else {
                    ElementRole::Form
                },
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
    if let Some(outcome) = parse_page_creation(input, &tokens, span) {
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
}
