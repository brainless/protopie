//! Deterministic style policy (Epic 001, T7). Pure; version [`crate::contracts::POLICY_VERSION`].
//!
//! # Tokens
//!
//! The generated project defines spacing and radius tokens in the `tokens`
//! region of `src/index.css`. This module is the Rust side of that table:
//!
//! | spacing token | value | radius token | value |
//! | --- | --- | --- | --- |
//! | `--space-0` | `0` | | |
//! | `--space-1` | `0.25rem` | `--radius-sm` | `0.25rem` |
//! | `--space-2` | `0.5rem` | `--radius-md` (default) | `0.5rem` |
//! | `--space-3` | `0.75rem` | `--radius-lg` | `1rem` |
//! | `--space-4` | `1rem` | | |
//! | `--space-5` | `1.5rem` | | |
//! | `--space-6` | `2rem` | | |
//! | `--space-7` | `3rem` | | |
//! | `--space-8` | `4rem` | | |
//! | `--space-9` | `6rem` | | |
//!
//! # Policy
//!
//! * Rounded corners select [`DEFAULT_RADIUS`] unless the element already has
//!   a nonzero radius (then there is nothing to do).
//! * More / less padding moves every component of the `padding` shorthand
//!   independently to the nearest strictly higher / lower spacing token. A
//!   component at the end of the scale stays; when no component can move the
//!   request is a no-change.
//! * Supported declared values are `0`, spacing tokens and `rem` literals with
//!   at most two decimals, one to four per `padding`. Anything else (other
//!   units, `calc()`, `auto`, unknown variables) is reported, never guessed.
//! * Full width is [`FULL_WIDTH`] and needs a known vertical parent layout.

use crate::contracts::StyleProperty;
use crate::parser::PaddingChange;

/// Spacing scale in hundredths of a rem; index N is token `--space-N`.
pub const SPACE_SCALE: [u32; 10] = [0, 25, 50, 75, 100, 150, 200, 300, 400, 600];
pub const DEFAULT_RADIUS: &str = "var(--radius-md)";
pub const RADIUS_TOKENS: [&str; 3] = ["var(--radius-sm)", "var(--radius-md)", "var(--radius-lg)"];
pub const FULL_WIDTH: &str = "100%";

/// Why a declared value cannot be stepped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StyleError {
    /// The value uses something outside the supported set.
    Unsupported(String),
}

impl std::fmt::Display for StyleError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let StyleError::Unsupported(value) = self;
        write!(
            f,
            "the current value {value:?} uses units or functions the spacing scale does not cover; \
             set an explicit value in the stylesheet first"
        )
    }
}

fn token(index: usize) -> String {
    format!("var(--space-{index})")
}

/// Parses `0`, `var(--space-N)` or `<n>rem` into hundredths of a rem.
pub fn parse_length(text: &str) -> Option<u32> {
    if text == "0" {
        return Some(0);
    }
    if let Some(n) = text
        .strip_prefix("var(--space-")
        .and_then(|r| r.strip_suffix(')'))
    {
        let index: usize = n.parse().ok()?;
        if n != index.to_string() {
            return None;
        }
        return SPACE_SCALE.get(index).copied();
    }
    let number = text.strip_suffix("rem")?;
    let (whole, fraction) = number.split_once('.').unwrap_or((number, ""));
    if whole.is_empty()
        || fraction.len() > 2
        || !whole.bytes().all(|b| b.is_ascii_digit())
        || !fraction.bytes().all(|b| b.is_ascii_digit())
        || whole.len() > 4
    {
        return None;
    }
    let whole: u32 = whole.parse().ok()?;
    let fraction: u32 = format!("{fraction:0<2}").parse().ok()?;
    Some(whole * 100 + fraction)
}

fn scale_index(hundredths: u32) -> Option<usize> {
    SPACE_SCALE.iter().position(|&v| v == hundredths)
}

/// The padding shorthand's components, or `None` if any is unsupported or the
/// count is not 1 to 4.
pub fn parse_padding(value: &str) -> Option<Vec<u32>> {
    let parts: Vec<&str> = value.split_whitespace().collect();
    if parts.is_empty() || parts.len() > 4 {
        return None;
    }
    parts.iter().map(|p| parse_length(p)).collect()
}

/// Scale index of the nearest token strictly above / below `hundredths`.
fn step(hundredths: u32, change: PaddingChange) -> Option<usize> {
    match change {
        PaddingChange::Increase => SPACE_SCALE.iter().position(|&v| v > hundredths),
        PaddingChange::Decrease => SPACE_SCALE.iter().rposition(|&v| v < hundredths),
    }
}

/// Absolute padding after one step of `change`, or `Ok(None)` when no
/// component can move (clamped at a scale limit). `current` is the declared
/// value; `None` means undeclared, which is padding `0`.
pub fn step_padding(
    current: Option<&str>,
    change: PaddingChange,
) -> Result<Option<String>, StyleError> {
    let text = current.unwrap_or("0");
    let components =
        parse_padding(text).ok_or_else(|| StyleError::Unsupported(text.to_string()))?;
    let originals: Vec<&str> = text.split_whitespace().collect();
    let mut moved = false;
    let rendered: Vec<String> = components
        .iter()
        .zip(&originals)
        .map(|(&value, original)| match step(value, change) {
            Some(index) => {
                moved = true;
                token(index)
            }
            // Clamped: keep the component, normalizing on-scale values to tokens.
            None => scale_index(value).map_or_else(|| (*original).to_string(), token),
        })
        .collect();
    Ok(moved.then(|| rendered.join(" ")))
}

/// True for no radius at all: undeclared or an explicit zero.
pub fn radius_is_zero(current: Option<&str>) -> bool {
    matches!(current, None | Some("0" | "0px" | "0rem"))
}

/// True if `value` is something the emitter may write for `property`. Keeps
/// plan-supplied values out of CSS unless they match the policy's grammar.
pub fn is_valid_value(property: StyleProperty, value: &str) -> bool {
    match property {
        StyleProperty::Padding => parse_padding(value).is_some(),
        StyleProperty::BorderRadius => RADIUS_TOKENS.contains(&value),
        StyleProperty::Width => value == FULL_WIDTH,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use PaddingChange::{Decrease, Increase};

    fn up(v: &str) -> Option<String> {
        step_padding(Some(v), Increase).unwrap()
    }
    fn down(v: &str) -> Option<String> {
        step_padding(Some(v), Decrease).unwrap()
    }

    #[test]
    fn axes_advance_independently_one_step() {
        assert_eq!(
            up("2rem 1rem").as_deref(),
            Some("var(--space-7) var(--space-5)")
        );
        assert_eq!(
            down("2rem 1rem").as_deref(),
            Some("var(--space-5) var(--space-3)")
        );
        assert_eq!(
            up("var(--space-6) var(--space-4)").as_deref(),
            Some("var(--space-7) var(--space-5)")
        );
        assert_eq!(up("1rem").as_deref(), Some("var(--space-5)"));
        assert_eq!(
            up("0 1rem 2rem 3rem").as_deref(),
            Some("var(--space-1) var(--space-5) var(--space-7) var(--space-8)")
        );
    }

    #[test]
    fn off_scale_values_move_to_the_nearest_strictly_higher_or_lower_token() {
        assert_eq!(up("1.25rem").as_deref(), Some("var(--space-5)"));
        assert_eq!(down("1.25rem").as_deref(), Some("var(--space-4)"));
        assert_eq!(up("7rem").as_deref(), None);
        assert_eq!(down("7rem").as_deref(), Some("var(--space-9)"));
        assert_eq!(up("0.1rem").as_deref(), Some("var(--space-1)"));
        assert_eq!(down("0.1rem").as_deref(), Some("var(--space-0)"));
    }

    #[test]
    fn scale_limits_clamp_per_axis_and_report_no_change_when_nothing_moves() {
        assert_eq!(up("6rem"), None);
        assert_eq!(down("0"), None);
        assert_eq!(step_padding(None, Decrease).unwrap(), None);
        assert_eq!(
            step_padding(None, Increase).unwrap().as_deref(),
            Some("var(--space-1)")
        );
        // One axis at the top stays while the other moves.
        assert_eq!(
            up("6rem 1rem").as_deref(),
            Some("var(--space-9) var(--space-5)")
        );
        assert_eq!(
            down("0 1rem").as_deref(),
            Some("var(--space-0) var(--space-3)")
        );
    }

    #[test]
    fn unsupported_values_are_rejected_not_guessed() {
        for bad in [
            "10px",
            "1em 2rem",
            "calc(1rem + 2px)",
            "auto",
            "var(--gap)",
            "var(--space-10)",
            "var(--space-04)",
            "1rem 2rem 3rem 4rem 5rem",
            "1.255rem",
            "-1rem",
            ".5rem",
            "",
            "1rem;color:red",
        ] {
            let result = step_padding(Some(bad), Increase);
            assert!(result.is_err(), "{bad:?} -> {result:?}");
        }
        assert!(parse_length("0.50rem") == Some(50));
    }

    #[test]
    fn emitted_value_grammar() {
        assert!(is_valid_value(
            StyleProperty::Padding,
            "var(--space-6) 1rem"
        ));
        assert!(!is_valid_value(StyleProperty::Padding, "1rem; x: y"));
        assert!(is_valid_value(StyleProperty::BorderRadius, DEFAULT_RADIUS));
        assert!(!is_valid_value(StyleProperty::BorderRadius, "5px"));
        assert!(is_valid_value(StyleProperty::Width, FULL_WIDTH));
        assert!(!is_valid_value(StyleProperty::Width, "100%; x"));
        assert!(radius_is_zero(None) && radius_is_zero(Some("0")));
        assert!(!radius_is_zero(Some("var(--radius-lg)")));
    }
}
