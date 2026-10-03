//! Code emission helpers for the reference template (Epic 001, T5).
//!
//! Labels are display text: they are never spliced into generated source
//! verbatim. They always pass through one of the escaping helpers here, and
//! identifiers, paths and component names are validated separately.

use crate::contracts::{ElementId, Placement};

/// Longest label the emitter accepts.
pub const MAX_LABEL_CHARS: usize = 80;

fn push_unicode_escape(out: &mut String, c: char) {
    let mut units = [0u16; 2];
    for unit in c.encode_utf16(&mut units) {
        out.push_str(&format!("\\u{:04x}", unit));
    }
}

/// A double-quoted JavaScript/TypeScript string literal denoting `text`.
///
/// Escapes backslashes, quotes, control characters and line/paragraph
/// separators so the result is a single-line literal whatever the label holds.
/// `<`, `>`, `{`, `}` and `&` are escaped too, so the literal is also safe to
/// read as plain text when a tool embeds it in markup.
pub fn tsx_string_literal(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for c in text.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '<' | '>' | '{' | '}' | '&' | '\u{2028}' | '\u{2029}' => {
                push_unicode_escape(&mut out, c)
            }
            c if c.is_control() => push_unicode_escape(&mut out, c),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// A JSX expression child rendering `text` literally: `{"..."}`. Using an
/// expression container avoids JSX text rules (entities, braces, `<`).
pub fn tsx_text_child(text: &str) -> String {
    format!("{{{}}}", tsx_string_literal(text))
}

/// One navigation item as emitted.
pub struct NavItemView<'a> {
    pub id: &'a ElementId,
    pub label: &'a str,
}

/// Source of a navigation component. Wholly owned and regenerated from the
/// model, so it never depends on reading the previous text back.
///
/// Items with an unresolved destination are noninteractive text: no anchor,
/// no `href`, no route.
pub fn navigation_tsx(
    name: &str,
    id: &ElementId,
    placement: Placement,
    items: &[NavItemView],
) -> String {
    let label = match placement {
        Placement::Top => "Top navigation",
        Placement::Side => "Side navigation",
    };
    let mut out = String::new();
    out.push_str(&format!("import styles from \"./{name}.module.css\";\n\n"));
    out.push_str(&format!("export default function {name}() {{\n"));
    out.push_str("\treturn (\n");
    out.push_str(&format!(
        "\t\t<nav data-protopie-id=\"{}\" class={{styles.nav}} aria-label={}>\n",
        id.0,
        tsx_text_child(label)
    ));
    if items.is_empty() {
        out.push_str("\t\t\t<ul class={styles.list} />\n");
    } else {
        out.push_str("\t\t\t<ul class={styles.list}>\n");
        for item in items {
            out.push_str(&format!(
                "\t\t\t\t<li class={{styles.item}} data-protopie-id=\"{}\">\n\t\t\t\t\t<span class={{styles.label}}>{}</span>\n\t\t\t\t</li>\n",
                item.id.0,
                tsx_text_child(item.label)
            ));
        }
        out.push_str("\t\t\t</ul>\n");
    }
    out.push_str("\t\t</nav>\n\t);\n}\n");
    out
}

/// Stylesheet of a navigation component (CSS module, wholly owned).
pub fn navigation_css() -> String {
    "\
.nav {
  display: flex;
  align-items: center;
  padding: 1rem 2rem;
  background: #fff;
  border-bottom: 1px solid #e3e5f0;
}

.list {
  display: flex;
  gap: 1.5rem;
  margin: 0;
  padding: 0;
  list-style: none;
}

.item {
  margin: 0;
}

.label {
  font-family: var(--font-body);
  color: #1b1b2f;
}
"
    .into()
}

/// Body of the `layout-top` region: one import per inserted component and the
/// `LayoutTop` component the template renders above the page content. The
/// region sits at module level, where import declarations are legal.
pub fn layout_top_region(components: &[String]) -> String {
    let mut out = String::new();
    for name in components {
        out.push_str(&format!("import {name} from \"./components/{name}\";\n"));
    }
    out.push_str("\nfunction LayoutTop() {\n\treturn (\n\t\t<>\n");
    for name in components {
        out.push_str(&format!("\t\t\t<{name} />\n"));
    }
    out.push_str("\t\t</>\n\t);\n}\n");
    out
}

/// Component names already imported by a `layout-top` region body.
pub fn layout_top_components(body: &str) -> Vec<String> {
    body.lines()
        .filter_map(|l| {
            let rest = l.strip_prefix("import ")?;
            let (name, rest) = rest.split_once(" from \"./components/")?;
            (rest == format!("{name}\";")).then(|| name.to_string())
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tsx_literals_neutralize_hostile_labels() {
        assert_eq!(tsx_string_literal("Contact Us"), "\"Contact Us\"");
        assert_eq!(tsx_string_literal("a\"b\\c"), r#""a\"b\\c""#);
        assert_eq!(tsx_string_literal("a\nb\r\tc"), r#""a\nb\r\tc""#);
        assert_eq!(
            tsx_string_literal("</nav>{x}&amp;"),
            r#""\u003c/nav\u003e\u007bx\u007d\u0026amp;""#
        );
        assert_eq!(tsx_string_literal("a\u{2028}b"), r#""a\u2028b""#);
        assert_eq!(tsx_string_literal("\u{0}\u{7f}"), r#""\u0000\u007f""#);
        // Astral characters stay as-is; no raw newline can ever survive.
        assert_eq!(
            tsx_string_literal("caf\u{e9} \u{1f600}"),
            "\"caf\u{e9} \u{1f600}\""
        );
        let hostile = "\"; alert(1); //\n</script>";
        assert!(!tsx_string_literal(hostile).contains('\n'));
        assert_eq!(tsx_text_child("x"), "{\"x\"}");
    }

    #[test]
    fn navigation_source_escapes_labels_and_never_links_unresolved_items() {
        let nav = ElementId::new("nav_1");
        let item = ElementId::new("item_1");
        let src = navigation_tsx(
            "TopNav",
            &nav,
            Placement::Top,
            &[NavItemView {
                id: &item,
                label: "Say \"hi\" <b>{x}</b>",
            }],
        );
        assert!(src.contains(r#"{"Say \"hi\" \u003cb\u003e\u007bx\u007d\u003c/b\u003e"}"#));
        assert!(!src.contains("<b>") && !src.contains("<a ") && !src.contains("href"));
        assert!(src.contains("<span class={styles.label}>"));
        assert!(!navigation_tsx("TopNav", &nav, Placement::Top, &[]).contains("<li"));
    }

    #[test]
    fn layout_region_round_trips_component_names() {
        let body = layout_top_region(&["TopNav".into(), "Footer".into()]);
        assert_eq!(layout_top_components(&body), ["TopNav", "Footer"]);
        assert!(body.contains("<TopNav />") && body.contains("function LayoutTop"));
        assert!(layout_top_components(&layout_top_region(&[])).is_empty());
    }
}
