/// Fixed v1 scaffold description. Every protopie project uses this same stack until the
/// scaffold itself changes, so this is a constant, not something read per-project.
pub struct TechStack {
    pub framework: &'static str,
    pub router: &'static str,
    pub styling: &'static str,
    pub mock_data_proxy: &'static str,
}

impl TechStack {
    pub const V1: TechStack = TechStack {
        framework: "SolidJS + TypeScript + Vite",
        router: "Solid Router",
        styling: "TailwindCSS + DaisyUI",
        mock_data_proxy: "all data comes from fetch() calls to a local mock-data HTTP proxy \
            route per entity; never import a JSON fixture directly into a component",
    };
}

/// One prior brain-dump's text. The generator never fetches this itself — callers assemble
/// the list from wherever the history actually lives (SQLite today is out of scope; see
/// epics/002-system-prompt-generator.md).
pub struct ChangeSummary {
    pub text: String,
}

/// Pure, deterministic: same inputs always produce the same string, no LLM call.
pub fn generate_system_prompt(stack: &TechStack, recent_changes: &[ChangeSummary]) -> String {
    let mut prompt = format!(
        "You are generating code for a protopie project. The project's fixed stack is:\n\
         - Framework: {}\n\
         - Routing: {}\n\
         - Styling: {}\n\
         - Data: {}\n",
        stack.framework, stack.router, stack.styling, stack.mock_data_proxy
    );

    if recent_changes.is_empty() {
        prompt.push_str("\nThis is a fresh project with no prior changes yet.\n");
    } else {
        prompt.push_str("\nRecent changes already made to this project:\n");
        for change in recent_changes {
            prompt.push_str(&format!("- {}\n", change.text));
        }
    }

    prompt.push_str(
        "\nGenerate only code consistent with this stack (SolidJS components, Solid Router \
         routes, Tailwind/DaisyUI classes). Never produce plain HTML/CSS/vanilla-JS output.\n",
    );

    prompt
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn describes_the_fixed_stack() {
        let prompt = generate_system_prompt(&TechStack::V1, &[]);
        assert!(prompt.contains("SolidJS"));
        assert!(prompt.contains("Solid Router"));
        assert!(prompt.contains("TailwindCSS"));
        assert!(prompt.contains("mock-data HTTP proxy"));
    }

    #[test]
    fn notes_a_fresh_project_when_history_is_empty() {
        let prompt = generate_system_prompt(&TechStack::V1, &[]);
        assert!(prompt.contains("fresh project"));
    }

    #[test]
    fn lists_recent_changes_when_present() {
        let history = vec![
            ChangeSummary {
                text: "added a top nav".into(),
            },
            ChangeSummary {
                text: "added a customer list view".into(),
            },
        ];
        let prompt = generate_system_prompt(&TechStack::V1, &history);
        assert!(!prompt.contains("fresh project"));
        assert!(prompt.contains("- added a top nav"));
        assert!(prompt.contains("- added a customer list view"));
    }
}
