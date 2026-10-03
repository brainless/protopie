//! Code emission helpers for the reference template (Epic 001, T5).
//!
//! Labels are display text: they are never spliced into generated source
//! verbatim. They always pass through one of the escaping helpers here, and
//! identifiers, paths and component names are validated separately.

use crate::contracts::{ElementContent, ElementId, ElementKind, Placement};
use std::collections::BTreeMap;

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
    /// Where the item leads; `None` renders noninteractive text.
    pub link: Option<LinkView<'a>>,
}

/// A resolved link target as emitted.
pub enum LinkView<'a> {
    /// Same-application route, optionally with a section anchor.
    Internal {
        path: &'a str,
        anchor: Option<&'a str>,
    },
    External {
        url: &'a str,
    },
}

impl LinkView<'_> {
    fn href(&self) -> String {
        match self {
            LinkView::Internal {
                path,
                anchor: Some(anchor),
            } => format!("{path}#{anchor}"),
            LinkView::Internal { path, anchor: None } => (*path).to_string(),
            LinkView::External { url } => (*url).to_string(),
        }
    }
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
            let content = match &item.link {
                None => format!(
                    "<span class={{styles.label}}>{}</span>",
                    tsx_text_child(item.label)
                ),
                Some(link) => {
                    // The href goes through an expression container: JSX
                    // attribute strings do not process backslash escapes.
                    let rel = match link {
                        LinkView::External { .. } => " rel=\"noopener noreferrer\"",
                        LinkView::Internal { .. } => "",
                    };
                    format!(
                        "<a class={{styles.link}} href={{{}}}{rel}>{}</a>",
                        tsx_string_literal(&link.href()),
                        tsx_text_child(item.label)
                    )
                }
            };
            out.push_str(&format!(
                "\t\t\t\t<li class={{styles.item}} data-protopie-id=\"{}\">\n\t\t\t\t\t{content}\n\t\t\t\t</li>\n",
                item.id.0,
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

.link {
  font-family: var(--font-body);
  color: #3b3fd8;
  text-decoration: none;
}

.link:hover {
  text-decoration: underline;
}
"
    .into()
}

// ------------------------------------------------------------------ page elements

/// Longest hero headline or footer text accepted.
pub const MAX_TEXT_CHARS: usize = 200;
/// Most form fields, and longest field label, accepted.
pub const MAX_FORM_FIELDS: usize = 8;
pub const MAX_FIELD_CHARS: usize = 40;

/// Component (and file stem) of a generated page element: the kind's name
/// plus the number of its ID (`form_2` is `Form2`), so it is a valid,
/// collision-free identifier that never shadows a browser global such as
/// `Image`. `None` for kinds that are not generated components of this kind
/// or IDs without a number.
pub fn element_component_name(kind: ElementKind, id: &ElementId) -> Option<String> {
    let stem = match kind {
        ElementKind::Hero => "Hero",
        ElementKind::Image => "Image",
        ElementKind::Form => "Form",
        ElementKind::Footer => "Footer",
        ElementKind::Button => "Button",
        ElementKind::Navigation | ElementKind::NavigationItem => return None,
    };
    let (prefix, number) = id.0.rsplit_once('_')?;
    (prefix == kind.id_prefix()
        && !number.is_empty()
        && number.len() <= 6
        && number.bytes().all(|b| b.is_ascii_digit()))
    .then(|| format!("{stem}{number}"))
}

/// Trimmed display text of 1 to `max` characters without control characters.
pub fn clean_text(text: &str, max: usize) -> Option<String> {
    let text = text.trim();
    (!text.is_empty() && text.chars().count() <= max && !text.chars().any(char::is_control))
        .then(|| text.to_string())
}

/// Field labels from a comma separated list: 1 to [`MAX_FORM_FIELDS`] distinct
/// (case-insensitive) labels of at most [`MAX_FIELD_CHARS`] characters.
pub fn parse_form_fields(text: &str) -> Result<Vec<String>, String> {
    let mut fields: Vec<String> = Vec::new();
    for part in text.split(',') {
        let Some(field) = clean_text(part, MAX_FIELD_CHARS) else {
            return Err(format!(
                "each field name needs 1 to {MAX_FIELD_CHARS} characters; separate fields with commas"
            ));
        };
        if fields.iter().any(|f| f.to_lowercase() == field.to_lowercase()) {
            return Err(format!("the field {field:?} is listed twice"));
        }
        fields.push(field);
    }
    if fields.len() > MAX_FORM_FIELDS {
        return Err(format!("a form can have at most {MAX_FORM_FIELDS} fields"));
    }
    Ok(fields)
}

/// Checks that `content` is complete and valid for `kind`. This is the last
/// line of defense: planning and answering validate too, but the emitter never
/// writes content that fails here.
pub fn validate_content(kind: ElementKind, content: &ElementContent) -> Result<(), String> {
    let text_ok = |max: usize| {
        content
            .text
            .as_deref()
            .is_some_and(|t| clean_text(t, max).as_deref() == Some(t))
    };
    let ok = match kind {
        ElementKind::Hero | ElementKind::Footer => text_ok(MAX_TEXT_CHARS),
        ElementKind::Button => text_ok(MAX_LABEL_CHARS),
        ElementKind::Image => {
            content
                .src
                .as_deref()
                .is_some_and(|u| validate_external_url(u).as_deref() == Some(u))
                && content
                    .alt
                    .as_deref()
                    .is_some_and(|a| clean_text(a, MAX_TEXT_CHARS).as_deref() == Some(a))
        }
        ElementKind::Form => {
            text_ok(MAX_LABEL_CHARS)
                && !content.fields.is_empty()
                && parse_form_fields(&content.fields.join(","))
                    .is_ok_and(|fields| fields == content.fields)
        }
        ElementKind::Navigation | ElementKind::NavigationItem => false,
    };
    if ok {
        Ok(())
    } else {
        Err(format!("the content of the {} is incomplete or invalid", kind.word()))
    }
}

fn text_of(content: &ElementContent) -> &str {
    content.text.as_deref().unwrap_or_default()
}

/// Source of a hero component (wholly owned). `id` doubles as the section
/// anchor so a navigation item can link to it.
pub fn hero_tsx(name: &str, id: &ElementId, content: &ElementContent) -> String {
    format!(
        "import styles from \"./{name}.module.css\";\n\nexport default function {name}() {{\n\treturn (\n\t\t<section id=\"{id}\" data-protopie-id=\"{id}\" class={{styles.hero}}>\n\t\t\t<h1 class={{styles.title}}>{}</h1>\n\t\t</section>\n\t);\n}}\n",
        tsx_text_child(text_of(content)),
        id = id.0
    )
}

pub fn hero_css() -> String {
    "\
.hero {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 1rem;
  padding: 4rem 2rem;
  text-align: center;
  background: #f8f9ff;
}

.title {
  margin: 0;
  font-family: var(--font-heading);
  font-weight: 800;
  font-size: clamp(2rem, 8vw, 4rem);
  line-height: 1.1;
  color: #1b1b2f;
}
"
    .into()
}

/// Source of an image component. The address and alternative text are literal
/// expressions, never spliced into markup.
pub fn image_tsx(name: &str, id: &ElementId, content: &ElementContent) -> String {
    format!(
        "import styles from \"./{name}.module.css\";\n\nexport default function {name}() {{\n\treturn (\n\t\t<img\n\t\t\tdata-protopie-id=\"{}\"\n\t\t\tclass={{styles.image}}\n\t\t\tsrc={{{}}}\n\t\t\talt={{{}}}\n\t\t/>\n\t);\n}}\n",
        id.0,
        tsx_string_literal(content.src.as_deref().unwrap_or_default()),
        tsx_string_literal(content.alt.as_deref().unwrap_or_default()),
    )
}

pub fn image_css() -> String {
    "\
.image {
  display: block;
  max-width: 100%;
  height: auto;
}
"
    .into()
}

/// Source of a form component: one text input per field and a submit button.
/// Submission is deliberately not wired anywhere: the handler only prevents
/// the browser's default navigation, because no destination was given.
pub fn form_tsx(name: &str, id: &ElementId, content: &ElementContent) -> String {
    let mut out = String::new();
    out.push_str(&format!("import styles from \"./{name}.module.css\";\n\n"));
    out.push_str(&format!("export default function {name}() {{\n\treturn (\n"));
    out.push_str(&format!(
        "\t\t<form\n\t\t\tdata-protopie-id=\"{}\"\n\t\t\tclass={{styles.form}}\n\t\t\tonSubmit={{(event) => event.preventDefault()}}\n\t\t>\n",
        id.0
    ));
    for (index, field) in content.fields.iter().enumerate() {
        out.push_str(&format!(
            "\t\t\t<label class={{styles.field}}>\n\t\t\t\t<span class={{styles.label}}>{}</span>\n\t\t\t\t<input class={{styles.input}} type=\"text\" name=\"field-{}\" />\n\t\t\t</label>\n",
            tsx_text_child(field),
            index + 1
        ));
    }
    out.push_str(&format!(
        "\t\t\t<button class={{styles.submit}} type=\"submit\">{}</button>\n\t\t</form>\n\t);\n}}\n",
        tsx_text_child(text_of(content))
    ));
    out
}

pub fn form_css() -> String {
    "\
.form {
  display: flex;
  flex-direction: column;
  gap: 1rem;
  max-width: 32rem;
  margin: 0 auto;
  padding: 2rem 1rem;
}

.field {
  display: flex;
  flex-direction: column;
  gap: 0.25rem;
}

.label {
  font-family: var(--font-body);
  font-weight: 600;
  color: #1b1b2f;
}

.input {
  padding: 0.5rem 0.75rem;
  border: 1px solid #c9ccdf;
  border-radius: 0.25rem;
  font: inherit;
}

.submit {
  align-self: flex-start;
  padding: 0.75rem 1.5rem;
  border: 0;
  border-radius: 0.25rem;
  background: #3b3fd8;
  color: #fff;
  font: inherit;
  cursor: pointer;
}
"
    .into()
}

pub fn footer_tsx(name: &str, id: &ElementId, content: &ElementContent) -> String {
    format!(
        "import styles from \"./{name}.module.css\";\n\nexport default function {name}() {{\n\treturn (\n\t\t<footer data-protopie-id=\"{}\" class={{styles.footer}}>\n\t\t\t<p class={{styles.text}}>{}</p>\n\t\t</footer>\n\t);\n}}\n",
        id.0,
        tsx_text_child(text_of(content))
    )
}

pub fn footer_css() -> String {
    "\
.footer {
  padding: 2rem 1rem;
  border-top: 1px solid #e3e5f0;
  text-align: center;
}

.text {
  margin: 0;
  font-family: var(--font-body);
  color: #55557a;
}
"
    .into()
}

/// Source of a button component. Without a destination it is a disabled
/// button: nothing is linked and no behavior is invented.
pub fn button_tsx(
    name: &str,
    id: &ElementId,
    content: &ElementContent,
    link: Option<LinkView>,
) -> String {
    let label = tsx_text_child(text_of(content));
    let element = match link {
        None => format!(
            "<button type=\"button\" disabled data-protopie-id=\"{}\" class={{styles.button}}>{label}</button>",
            id.0
        ),
        Some(link) => {
            let rel = match link {
                LinkView::External { .. } => " rel=\"noopener noreferrer\"",
                LinkView::Internal { .. } => "",
            };
            format!(
                "<a data-protopie-id=\"{}\" class={{styles.button}} href={{{}}}{rel}>{label}</a>",
                id.0,
                tsx_string_literal(&link.href())
            )
        }
    };
    format!(
        "import styles from \"./{name}.module.css\";\n\nexport default function {name}() {{\n\treturn (\n\t\t{element}\n\t);\n}}\n"
    )
}

/// `display: block` with `width: fit-content` keeps buttons stacking
/// vertically in the page flow instead of sitting side by side inline.
pub fn button_css() -> String {
    "\
.button {
  display: block;
  width: fit-content;
  padding: 0.75rem 1.5rem;
  border: 0;
  background: #3b3fd8;
  color: #fff;
  font: inherit;
  text-decoration: none;
  cursor: pointer;
}

.button:disabled {
  background: #c9ccdf;
  color: #55557a;
  cursor: not-allowed;
}
"
    .into()
}

/// Class of the root element of a generated component, the one that carries
/// `data-protopie-id` and is styled by the style policy.
pub fn root_class(kind: ElementKind) -> &'static str {
    match kind {
        ElementKind::Hero => "hero",
        ElementKind::Image => "image",
        ElementKind::Form => "form",
        ElementKind::Footer => "footer",
        ElementKind::Button => "button",
        ElementKind::Navigation | ElementKind::NavigationItem => "nav",
    }
}

/// Stylesheet of a generated page element.
pub fn element_css(kind: ElementKind) -> String {
    match kind {
        ElementKind::Hero => hero_css(),
        ElementKind::Image => image_css(),
        ElementKind::Form => form_css(),
        ElementKind::Footer => footer_css(),
        ElementKind::Button => button_css(),
        ElementKind::Navigation | ElementKind::NavigationItem => navigation_css(),
    }
}

/// Body of the `home-flow` region of `Home.tsx`: one import per generated
/// component and the `HomeAbove` / `HomeBelow` components the page renders
/// around its hero. Empty lists reproduce the template's body exactly.
pub fn home_flow_region(above: &[String], below: &[String]) -> String {
    let mut out = String::new();
    let mut imported: Vec<&String> = Vec::new();
    for name in above.iter().chain(below) {
        if !imported.contains(&name) {
            imported.push(name);
            out.push_str(&format!("import {name} from \"../components/{name}\";\n"));
        }
    }
    if !imported.is_empty() {
        out.push('\n');
    }
    for (function, names) in [("HomeAbove", above), ("HomeBelow", below)] {
        out.push_str(&format!("function {function}() {{\n"));
        if names.is_empty() {
            out.push_str("\treturn <></>;\n");
        } else {
            out.push_str("\treturn (\n\t\t<>\n");
            for name in names {
                out.push_str(&format!("\t\t\t<{name} />\n"));
            }
            out.push_str("\t\t</>\n\t);\n");
        }
        out.push_str("}\n");
        if function == "HomeAbove" {
            out.push('\n');
        }
    }
    out
}

/// Declared values of the properties the style policy edits (`padding`,
/// `border-radius`, `width`) in the rule `selector { ... }` of `css`. Empty if
/// the rule does not exist. This is how the model learns what the cascade
/// holds for an element it generated or seeded.
pub fn css_declared_values(css: &str, selector: &str) -> BTreeMap<String, String> {
    let open = format!("{selector} {{");
    let mut values = BTreeMap::new();
    let mut inside = false;
    for line in css.lines() {
        if !inside {
            inside = line.trim_end() == open;
        } else if line.trim_end() == "}" {
            break;
        } else if let Some((property, value)) = line.trim().split_once(':') {
            let property = property.trim();
            if matches!(property, "padding" | "border-radius" | "width") {
                if let Some(value) = value.trim().strip_suffix(';') {
                    values.insert(property.to_string(), value.trim().to_string());
                }
            }
        }
    }
    values
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

// ------------------------------------------------------------------ pages

/// Longest page slug (and so route segment) the emitter accepts.
pub const MAX_SLUG_CHARS: usize = 48;

/// Route slug derived from a page label: lowercase ASCII letters and digits
/// joined by single dashes. `None` when nothing usable remains or the result
/// is too long to be a sensible path; the caller then asks for a name.
pub fn derive_slug(label: &str) -> Option<String> {
    let slug = crate::slugify(label);
    (!slug.is_empty() && slug.chars().count() <= MAX_SLUG_CHARS).then_some(slug)
}

/// True for a slug the emitter will build names and paths from.
pub fn is_valid_slug(slug: &str) -> bool {
    !slug.is_empty()
        && slug.chars().count() <= MAX_SLUG_CHARS
        && !slug.starts_with('-')
        && !slug.ends_with('-')
        && !slug.contains("--")
        && slug
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// Component (and file stem) of a page: PascalCase of the slug plus a `Page`
/// suffix (`Page` prefix for digit-leading slugs), so it is always a valid
/// identifier that cannot collide with `Home` or shadow a global.
pub fn page_component_name(slug: &str) -> String {
    let mut out = String::new();
    for segment in slug.split('-').filter(|s| !s.is_empty()) {
        let mut chars = segment.chars();
        if let Some(first) = chars.next() {
            out.push(first.to_ascii_uppercase());
            out.push_str(chars.as_str());
        }
    }
    // A digit-leading slug (`404`) cannot start an identifier.
    if out.starts_with(|c: char| c.is_ascii_digit()) {
        format!("Page{out}")
    } else {
        out.push_str("Page");
        out
    }
}

/// Route path of a page slug.
pub fn page_path(slug: &str) -> String {
    format!("/{slug}")
}

/// Source of a generated page. Wholly owned and regenerated from the model.
pub fn page_tsx(name: &str, id: &str, label: &str) -> String {
    format!(
        "import styles from \"./{name}.module.css\";\n\nexport default function {name}() {{\n\treturn (\n\t\t<main data-protopie-id=\"{id}\" class={{styles.page}}>\n\t\t\t<h1 class={{styles.title}}>{}</h1>\n\t\t</main>\n\t);\n}}\n",
        tsx_text_child(label)
    )
}

/// Stylesheet of a generated page (CSS module, wholly owned).
pub fn page_css() -> String {
    "\
.page {
  min-height: 100vh;
  padding: 4rem 2rem;
  font-family: var(--font-body);
  color: #1b1b2f;
}

.title {
  margin: 0;
}
"
    .into()
}

/// One route as emitted in the `routes` region of the router.
pub struct RouteView<'a> {
    pub path: &'a str,
    pub component: &'a str,
}

/// Body of the `routes` region: one route entry per registered page, in
/// registration order, using the template's `lazy(() => import(...))` syntax.
pub fn routes_region(routes: &[RouteView]) -> String {
    let mut out = String::new();
    for r in routes {
        out.push_str(&format!(
            "\t\t{{ path: {}, component: lazy(() => import(\"./pages/{}\")) }},\n",
            tsx_string_literal(r.path),
            r.component
        ));
    }
    out
}

/// Longest external URL accepted.
pub const MAX_URL_CHARS: usize = 2048;

/// Validates an external link target. Only absolute `http`/`https` URLs with a
/// plain host (a dotted name or `localhost`, optional port) are accepted: no
/// user info, whitespace, control or markup characters. Returns the trimmed URL.
pub fn validate_external_url(input: &str) -> Option<String> {
    let url = input.trim();
    if url.is_empty() || url.chars().count() > MAX_URL_CHARS {
        return None;
    }
    if url.chars().any(|c| {
        c.is_whitespace()
            || c.is_control()
            || matches!(
                c,
                '"' | '\'' | '<' | '>' | '\\' | '`' | '{' | '}' | '|' | '^'
            )
    }) {
        return None;
    }
    let rest = ["https://", "http://"].iter().find_map(|scheme| {
        url.get(..scheme.len())
            .filter(|p| p.eq_ignore_ascii_case(scheme))
            .map(|_| &url[scheme.len()..])
    })?;
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let authority = &rest[..end];
    let (host, port) = match authority.rsplit_once(':') {
        Some((h, p)) => (h, Some(p)),
        None => (authority, None),
    };
    if let Some(p) = port {
        if p.is_empty() || p.len() > 5 || !p.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
    }
    let labels: Vec<&str> = host.split('.').collect();
    let label_ok = |l: &&str| {
        !l.is_empty()
            && !l.starts_with('-')
            && !l.ends_with('-')
            && l.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
    };
    if !labels.iter().all(label_ok) || (labels.len() < 2 && host != "localhost") {
        return None;
    }
    Some(url.to_string())
}

// ------------------------------------------------------------------ styles

/// True for a CSS class name the emitter will put in a selector.
pub fn is_css_class(class: &str) -> bool {
    !class.is_empty()
        && class.len() <= 64
        && class.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_')
        && class
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

/// Selector of the rule that styles one instance of a shared class. Generated
/// elements carry `data-protopie-id`; the extra attribute raises specificity
/// above the shared `.class` rule wherever either is declared.
pub fn instance_selector(class: &str, id: &ElementId) -> String {
    format!(".{class}[data-protopie-id=\"{}\"]", id.0)
}

fn declaration(property: &str, value: &str) -> String {
    format!("  {property}: {value};\n")
}

/// Sets absolute declarations in the existing rule `selector { ... }` of a CSS
/// text, replacing a declaration of the same property or adding one before the
/// closing brace. Generated rules start at column 0 (`<selector> {`) and end
/// with a line `}`. `None` if no such rule exists.
pub fn css_set_declarations(css: &str, selector: &str, edits: &[(&str, &str)]) -> Option<String> {
    let open = format!("{selector} {{");
    let lines: Vec<&str> = css.split_inclusive('\n').collect();
    let start = lines.iter().position(|l| l.trim_end() == open)?;
    let end = start
        + 1
        + lines[start + 1..]
            .iter()
            .position(|l| l.trim_end() == "}")?;
    let mut body: Vec<String> = lines[start + 1..end]
        .iter()
        .map(|l| (*l).to_string())
        .collect();
    for (property, value) in edits {
        let prefix = format!("  {property}:");
        match body.iter().position(|l| l.starts_with(&prefix)) {
            Some(at) => body[at] = declaration(property, value),
            None => body.push(declaration(property, value)),
        }
    }
    let mut out: String = lines[..=start].concat();
    out.push_str(&body.concat());
    out.push_str(&lines[end..].concat());
    Some(out)
}

/// Appends a new rule with the given declarations (blank-line separated).
pub fn css_append_rule(css: &str, selector: &str, edits: &[(&str, &str)]) -> String {
    let mut out = css.to_string();
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    if !out.is_empty() && !out.ends_with("\n\n") && !out.trim().is_empty() {
        out.push('\n');
    }
    out.push_str(&format!("{selector} {{\n"));
    for (property, value) in edits {
        out.push_str(&declaration(property, value));
    }
    out.push_str("}\n");
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
                link: None,
            }],
        );
        assert!(src.contains(r#"{"Say \"hi\" \u003cb\u003e\u007bx\u007d\u003c/b\u003e"}"#));
        assert!(!src.contains("<b>") && !src.contains("<a ") && !src.contains("href"));
        assert!(src.contains("<span class={styles.label}>"));
        assert!(!navigation_tsx("TopNav", &nav, Placement::Top, &[]).contains("<li"));
    }

    #[test]
    fn linked_items_render_anchors_with_expression_hrefs() {
        let nav = ElementId::new("nav_1");
        let (a, b, c) = (
            ElementId::new("item_1"),
            ElementId::new("item_2"),
            ElementId::new("item_3"),
        );
        let src = navigation_tsx(
            "TopNav",
            &nav,
            Placement::Top,
            &[
                NavItemView {
                    id: &a,
                    label: "Contact <Us>",
                    link: Some(LinkView::Internal {
                        path: "/contact-us",
                        anchor: None,
                    }),
                },
                NavItemView {
                    id: &b,
                    label: "Top",
                    link: Some(LinkView::Internal {
                        path: "/",
                        anchor: Some("hero_1"),
                    }),
                },
                NavItemView {
                    id: &c,
                    label: "Docs",
                    link: Some(LinkView::External {
                        url: "https://example.com/a?b=1",
                    }),
                },
            ],
        );
        assert!(
            src.contains(r#"href={"/contact-us"}>{"Contact \u003cUs\u003e"}</a>"#),
            "{src}"
        );
        assert!(src.contains(r##"href={"/#hero_1"}"##));
        assert!(src.contains(r#"href={"https://example.com/a?b=1"} rel="noopener noreferrer""#));
        assert!(!src.contains("<span"));
    }

    #[test]
    fn slugs_components_and_routes_are_derived_and_validated() {
        assert_eq!(derive_slug("Contact Us").as_deref(), Some("contact-us"));
        assert_eq!(
            derive_slug("  Terms of Use!! ").as_deref(),
            Some("terms-of-use")
        );
        assert_eq!(derive_slug("???"), None);
        assert_eq!(derive_slug(&"a".repeat(MAX_SLUG_CHARS + 1)), None);
        assert!(is_valid_slug("contact-us") && is_valid_slug("404"));
        for bad in ["", "-a", "a-", "a--b", "A", "a/b", "../x", "a b", "a_b"] {
            assert!(!is_valid_slug(bad), "{bad:?}");
        }
        assert_eq!(page_component_name("contact-us"), "ContactUsPage");
        assert_eq!(page_component_name("404"), "Page404");
        assert_eq!(page_component_name("a1-b"), "A1BPage");
        assert_eq!(page_path("contact-us"), "/contact-us");
        let body = routes_region(&[RouteView {
            path: "/contact-us",
            component: "ContactUsPage",
        }]);
        assert_eq!(
            body,
            "\t\t{ path: \"/contact-us\", component: lazy(() => import(\"./pages/ContactUsPage\")) },\n"
        );
        let page = page_tsx("ContactUsPage", "contact-us", "Contact \"Us\"");
        assert!(page.contains(r#"{"Contact \"Us\""}"#));
    }

    #[test]
    fn external_urls_are_validated() {
        for ok in [
            "https://example.com",
            "http://example.com/a/b?c=d#e",
            "HTTPS://Example.COM:8080/x",
            "http://localhost:3000",
            "  https://sub.example.co.uk/p  ",
        ] {
            assert!(validate_external_url(ok).is_some(), "{ok}");
        }
        assert_eq!(
            validate_external_url("  https://example.com  ").as_deref(),
            Some("https://example.com")
        );
        for bad in [
            "",
            "example.com",
            "javascript:alert(1)",
            "ftp://example.com",
            "//example.com",
            "https://",
            "https://localhost2",
            "https://exa mple.com",
            "https://example.com/\"onclick",
            "https://user@example.com",
            "https://example.com:port",
            "https://-bad.example.com",
            "https://exa..mple.com",
            "https://example.com/<script>",
            "data:text/html,hi",
        ] {
            assert!(validate_external_url(bad).is_none(), "{bad}");
        }
        assert!(validate_external_url(&format!(
            "https://example.com/{}",
            "a".repeat(MAX_URL_CHARS)
        ))
        .is_none());
    }

    #[test]
    fn css_rules_are_edited_absolutely_and_appended_scoped() {
        let css =
            ".card {\n  padding: 2rem 1rem;\n  color: red;\n}\n\n.other {\n  padding: 1rem;\n}\n";
        let out = css_set_declarations(
            css,
            ".card",
            &[
                ("padding", "var(--space-7)"),
                ("border-radius", "var(--radius-md)"),
            ],
        )
        .unwrap();
        assert_eq!(
            out,
            ".card {\n  padding: var(--space-7);\n  color: red;\n  border-radius: var(--radius-md);\n}\n\n.other {\n  padding: 1rem;\n}\n"
        );
        assert!(css_set_declarations(css, ".missing", &[("padding", "0")]).is_none());
        // `.card` does not match `.card-2`.
        assert!(css_set_declarations(".card-2 {\n}\n", ".card", &[("a", "b")]).is_none());
        let appended = css_append_rule(
            css,
            &instance_selector("card", &ElementId::new("img_2")),
            &[("width", "100%")],
        );
        assert!(appended.ends_with("\n\n.card[data-protopie-id=\"img_2\"] {\n  width: 100%;\n}\n"));
        assert_eq!(
            css_append_rule("", ".a", &[("x", "y")]),
            ".a {\n  x: y;\n}\n"
        );
        assert!(is_css_class("hero") && is_css_class("_a-1"));
        for bad in ["", "1a", "a b", "a.b", "a\"b", "a]"] {
            assert!(!is_css_class(bad), "{bad:?}");
        }
    }

    #[test]
    fn layout_region_round_trips_component_names() {
        let body = layout_top_region(&["TopNav".into(), "Footer".into()]);
        assert_eq!(layout_top_components(&body), ["TopNav", "Footer"]);
        assert!(body.contains("<TopNav />") && body.contains("function LayoutTop"));
        assert!(layout_top_components(&layout_top_region(&[])).is_empty());
    }

    fn content(text: Option<&str>) -> ElementContent {
        ElementContent {
            text: text.map(Into::into),
            ..Default::default()
        }
    }

    #[test]
    fn element_components_are_named_from_kind_and_id_number() {
        let name = |kind, id: &str| element_component_name(kind, &ElementId::new(id));
        assert_eq!(name(ElementKind::Form, "form_1").as_deref(), Some("Form1"));
        assert_eq!(name(ElementKind::Image, "img_12").as_deref(), Some("Image12"));
        assert_eq!(name(ElementKind::Hero, "hero_2").as_deref(), Some("Hero2"));
        // Wrong prefix, no number, hostile ids and non-components are refused.
        for (kind, id) in [
            (ElementKind::Form, "img_1"),
            (ElementKind::Form, "form_"),
            (ElementKind::Form, "form_1x"),
            (ElementKind::Form, "form_1/../x"),
            (ElementKind::Navigation, "nav_1"),
        ] {
            assert_eq!(name(kind, id), None, "{id}");
        }
    }

    #[test]
    fn content_validation_requires_everything_and_rejects_bad_values() {
        let button = content(Some("Go"));
        assert!(validate_content(ElementKind::Button, &button).is_ok());
        assert!(validate_content(ElementKind::Button, &content(None)).is_err());
        assert!(validate_content(ElementKind::Button, &content(Some(" Go"))).is_err());
        assert!(validate_content(ElementKind::Button, &content(Some("a\nb"))).is_err());
        assert!(validate_content(
            ElementKind::Button,
            &content(Some(&"x".repeat(MAX_LABEL_CHARS + 1)))
        )
        .is_err());
        let image = ElementContent {
            src: Some("https://example.com/a.png".into()),
            alt: Some("A cat".into()),
            ..Default::default()
        };
        assert!(validate_content(ElementKind::Image, &image).is_ok());
        for bad in [
            ElementContent {
                src: Some("javascript:alert(1)".into()),
                ..image.clone()
            },
            ElementContent {
                alt: None,
                ..image.clone()
            },
            ElementContent {
                src: Some("https://example.com/\"onerror".into()),
                ..image.clone()
            },
        ] {
            assert!(validate_content(ElementKind::Image, &bad).is_err());
        }
        let form = ElementContent {
            text: Some("Send".into()),
            fields: vec!["Name".into(), "Email".into()],
            ..Default::default()
        };
        assert!(validate_content(ElementKind::Form, &form).is_ok());
        assert!(validate_content(
            ElementKind::Form,
            &ElementContent {
                fields: vec![],
                ..form.clone()
            }
        )
        .is_err());
        assert!(validate_content(ElementKind::Hero, &content(Some("Welcome"))).is_ok());
        assert!(validate_content(ElementKind::Navigation, &content(Some("x"))).is_err());
    }

    #[test]
    fn form_fields_are_split_trimmed_and_bounded() {
        assert_eq!(
            parse_form_fields(" Name ,Email,  Phone number ").unwrap(),
            ["Name", "Email", "Phone number"]
        );
        assert!(parse_form_fields("Name, name").is_err());
        assert!(parse_form_fields("Name,, Email").is_err());
        assert!(parse_form_fields("").is_err());
        assert!(parse_form_fields(&"a".repeat(MAX_FIELD_CHARS + 1)).is_err());
        assert!(parse_form_fields("a,b,c,d,e,f,g,h,i").is_err());
        assert_eq!(parse_form_fields("a,b,c,d,e,f,g,h").unwrap().len(), MAX_FORM_FIELDS);
    }

    #[test]
    fn generated_elements_carry_ids_and_escape_content() {
        let id = ElementId::new("form_1");
        let form = form_tsx(
            "Form1",
            &id,
            &ElementContent {
                text: Some("Say \"hi\"".into()),
                fields: vec!["<Name>".into()],
                ..Default::default()
            },
        );
        assert!(form.contains("data-protopie-id=\"form_1\""));
        assert!(form.contains("{\"\\u003cName\\u003e\"}") && !form.contains("<Name>"));
        assert!(form.contains("{\"Say \\\"hi\\\"\"}"));
        assert!(form.contains("event.preventDefault()"));
        let hero = hero_tsx("Hero2", &ElementId::new("hero_2"), &content(Some("A {b}")));
        assert!(hero.contains("id=\"hero_2\" data-protopie-id=\"hero_2\""));
        assert!(hero.contains("{\"A \\u007bb\\u007d\"}"));
        let footer = footer_tsx("Footer1", &ElementId::new("footer_1"), &content(Some("x")));
        assert!(footer.contains("data-protopie-id=\"footer_1\""));
        let image = image_tsx(
            "Image1",
            &ElementId::new("img_1"),
            &ElementContent {
                src: Some("https://example.com/a?b=1&c=2".into()),
                alt: Some("a\"b".into()),
                ..Default::default()
            },
        );
        assert!(image.contains("src={\"https://example.com/a?b=1\\u0026c=2\"}"));
        assert!(image.contains("alt={\"a\\\"b\"}"));
    }

    #[test]
    fn buttons_without_a_destination_are_disabled_and_never_linked() {
        let id = ElementId::new("button_1");
        let unlinked = button_tsx("Button1", &id, &content(Some("Go")), None);
        assert!(unlinked.contains("<button type=\"button\" disabled data-protopie-id=\"button_1\""));
        assert!(!unlinked.contains("href"));
        let linked = button_tsx(
            "Button1",
            &id,
            &content(Some("Go")),
            Some(LinkView::External {
                url: "https://example.com",
            }),
        );
        assert!(linked.contains("<a data-protopie-id=\"button_1\""));
        assert!(linked.contains("href={\"https://example.com\"} rel=\"noopener noreferrer\""));
        assert!(!linked.contains("disabled"));
    }

    #[test]
    fn home_flow_region_reproduces_the_template_when_empty() {
        let template = "\
function HomeAbove() {
\treturn <></>;
}

function HomeBelow() {
\treturn <></>;
}
";
        assert_eq!(home_flow_region(&[], &[]), template);
        let body = home_flow_region(&["Form2".into()], &["Form1".into(), "Footer1".into()]);
        assert!(body.starts_with(
            "import Form2 from \"../components/Form2\";\nimport Form1 from \"../components/Form1\";\nimport Footer1 from \"../components/Footer1\";\n\n"
        ));
        assert!(body.contains("function HomeAbove() {\n\treturn (\n\t\t<>\n\t\t\t<Form2 />\n\t\t</>\n\t);\n}"));
        assert!(body.contains("\t\t\t<Form1 />\n\t\t\t<Footer1 />\n"));
    }

    #[test]
    fn declared_values_come_from_the_named_rule_only() {
        for kind in [
            ElementKind::Hero,
            ElementKind::Image,
            ElementKind::Form,
            ElementKind::Footer,
            ElementKind::Button,
        ] {
            let css = element_css(kind);
            assert!(css.contains(&format!(".{} {{", root_class(kind))), "{kind:?}");
        }
        let form = css_declared_values(&form_css(), ".form");
        assert_eq!(form["padding"], "2rem 1rem");
        // `.input` also declares a padding; it is a different rule.
        assert_eq!(css_declared_values(&form_css(), ".input")["border-radius"], "0.25rem");
        assert_eq!(css_declared_values(&image_css(), ".image").len(), 0);
        assert_eq!(css_declared_values(&button_css(), ".button")["width"], "fit-content");
        assert!(css_declared_values(&form_css(), ".missing").is_empty());
        assert_eq!(
            css_declared_values("/* x */\n.hero {\n  padding: 2rem 1rem;\n  min-height: 100vh;\n}\n", ".hero")["padding"],
            "2rem 1rem"
        );
    }
}
