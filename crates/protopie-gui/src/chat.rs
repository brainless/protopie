//! Project entry and single-column agent chat, with a modal for naming a new project.

use akar_components::{
    akar_button, akar_text_input, modal_begin, modal_end, scroll_area_begin, scroll_area_end,
    AkarTheme, ButtonVariant, TextEditState, AKAR_THEME_DARK,
};
use akar_core::{AkarCore, Key, QuadCall, TextCall, Z_FLOAT, Z_OVERLAY};
use akar_layout::{length, Dimension, Display, FlexDirection, Layout, NodeId, Size, Style};
use protopie_api::{ModifyOutcome, ModifyProjectResponse, ProjectInfo, CHAT_EXAMPLES};

use crate::client::{ApiClient, Event};
use crate::preview::{Action, PreviewUi};
use crate::review::{Choice, Review, Sent};

/// Folder new projects are created in (the server lists its own, equal, constant).
const PROJECTS_BASE_PATH: &str = ".projects";

const THEME: AkarTheme = AKAR_THEME_DARK;
const HEADER_H: f32 = 44.0;
const INPUT_BAR_H: f32 = 56.0;
const BUBBLE_PAD_X: f32 = 12.0;
const BUBBLE_PAD_Y: f32 = 8.0;
const MSG_GAP: f32 = 8.0;
const MSG_MARGIN: f32 = 12.0;
const BTN_H: f32 = 30.0;
const ROW_H: f32 = 28.0;
const MAX_ROWS: usize = 6;
const MODAL_NS: u64 = 0xD1A1_06;
// Fixed text-buffer ids for the dropdown (the pipeline keys buffers by id).
const BUF_LABEL: u64 = 0xC0DE_0001;
const BUF_CHEVRON: u64 = 0xC0DE_0002;
const BUF_ROW: u64 = 0xC0DE_1000;
const BUF_CHOICE: u64 = 0xC0DE_2000;
const CHOICE_H: f32 = 30.0;
const CHOICE_GAP: f32 = 8.0;
const CHOICE_PAD_X: f32 = 14.0;
const ENTRY_ROW_H: f32 = 86.0;
const BUF_ENTRY: u64 = 0xC0DE_3000;
const BUF_PREVIEW: u64 = 0xC0DE_4000;
const PREVIEW_BAR_H: f32 = 104.0;
const PREVIEW_LOG_H: f32 = 106.0;
const EXAMPLE_ROW_H: f32 = 47.0;
const BUF_EXAMPLE: u64 = 0xC0DE_5000;

/// Dropdown rows before the project list: "New Project" and "No project".
const NEW_ROW: usize = 0;
const NONE_ROW: usize = 1;
const FIXED_ROWS: usize = 2;

#[derive(Clone, Copy, PartialEq)]
enum Role {
    User,
    Agent,
    Error,
}

struct Message {
    role: Role,
    text: String,
    buffer: Option<u64>,
}

struct NewProjectForm {
    name: String,
    edit: TextEditState,
}

pub struct ChatView {
    client: ApiClient,
    layout: Layout,
    root: NodeId,
    header: NodeId,
    project_btn: NodeId,
    projects: Vec<ProjectInfo>,
    selected: Option<ProjectInfo>,
    /// Slug of a just-created project, selected once the refreshed list arrives.
    select_after_refresh: Option<String>,
    popup_open: bool,
    popup_scroll: f32,
    entry_scroll: f32,
    entry_error: Option<String>,
    /// Messages are not drawn above this y while the popup covers them.
    occlude_top: f32,
    new_project: Option<NewProjectForm>,
    messages_area: NodeId,
    input: NodeId,
    send: NodeId,
    messages: Vec<Message>,
    input_text: String,
    edit: TextEditState,
    scroll_y: f32,
    pending: usize,
    stick_to_bottom: bool,
    /// Project the review buttons act on.
    review_project: Option<ProjectInfo>,
    /// Plan/diff preview or question options awaiting a click.
    review: Review,
    /// Rows the review buttons wrapped into at the last layout.
    review_rows: usize,
    /// The request whose preview is shown, repeated to apply it.
    last_sent: Option<Sent>,
    preview: PreviewUi,
}

fn rgba(c: u32) -> [f32; 4] {
    [
        ((c >> 24) & 0xFF) as f32 / 255.0,
        ((c >> 16) & 0xFF) as f32 / 255.0,
        ((c >> 8) & 0xFF) as f32 / 255.0,
        (c & 0xFF) as f32 / 255.0,
    ]
}

fn quad(rect: [f32; 4], fill: u32, radius: f32) -> QuadCall {
    QuadCall {
        rect,
        fill: rgba(fill),
        border_color: [0.0; 4],
        corner_radii: [radius; 4],
        border_width: 0.0,
        z: 0.0,
        shadow_blur: 0.0,
        shadow_spread: 0.0,
        shadow_color: [0.0; 4],
        shadow_offset: [0.0; 2],
        _pad: [0.0; 2],
    }
}

/// `r` without the part above `y` (used to hide text behind the dropdown popup).
fn clip_below(r: [f32; 4], y: f32) -> [f32; 4] {
    if y <= r[1] {
        return r;
    }
    [r[0], y, r[2], (r[1] + r[3] - y).max(0.0)]
}

/// Intersection of two rects, clamped to non-negative size.
fn clip_to(r: [f32; 4], bounds: [f32; 4]) -> [f32; 4] {
    let x0 = r[0].max(bounds[0]);
    let y0 = r[1].max(bounds[1]);
    let x1 = (r[0] + r[2]).min(bounds[0] + bounds[2]);
    let y1 = (r[1] + r[3]).min(bounds[1] + bounds[3]);
    [x0, y0, (x1 - x0).max(0.0), (y1 - y0).max(0.0)]
}

fn is_available(project: &ProjectInfo) -> bool {
    project.unavailable_reason.is_none()
}

fn now_unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|elapsed| u64::try_from(elapsed.as_millis()).ok())
        .unwrap_or(0)
}

fn modified_label(now_ms: u64, modified_ms: Option<u64>) -> String {
    let Some(modified_ms) = modified_ms else {
        return "Modified time unavailable".into();
    };
    let minutes = now_ms.saturating_sub(modified_ms) / 60_000;
    let time = match minutes {
        0 => "just now".to_string(),
        1..=59 => format!("{minutes} min ago"),
        60..=1439 => format!("{} hr ago", minutes / 60),
        _ => format!("{} days ago", minutes / 1440),
    };
    format!("Modified {time}")
}

fn refreshed_selection(
    projects: &[ProjectInfo],
    selected: Option<&ProjectInfo>,
    created_slug: Option<&str>,
) -> Option<ProjectInfo> {
    if let Some(slug) = created_slug {
        projects
            .iter()
            .find(|p| p.name == slug && is_available(p))
            .cloned()
    } else {
        selected.and_then(|selected| {
            projects
                .iter()
                .find(|p| p.path == selected.path && is_available(p))
                .cloned()
        })
    }
}

fn response_diagnostic(response: &ModifyProjectResponse) -> serde_json::Value {
    let mut safe = response.clone();
    let mut diff_omitted = false;
    if let Some(ModifyOutcome::Preview { diffs, .. }) = &mut safe.outcome {
        diff_omitted = !diffs.is_empty();
        diffs.clear();
    }
    let (kind, plan_id, revision, question_ids): (&str, Option<&str>, Option<u64>, Vec<&str>) =
        match &response.outcome {
            Some(ModifyOutcome::Preview {
                plan_id,
                base_revision,
                questions,
                ..
            }) => (
                "preview",
                Some(plan_id),
                Some(*base_revision),
                questions.iter().map(|q| q.id.as_str()).collect(),
            ),
            Some(ModifyOutcome::Applied {
                plan_id, questions, ..
            }) => (
                "applied",
                Some(plan_id),
                None,
                questions.iter().map(|q| q.id.as_str()).collect(),
            ),
            Some(ModifyOutcome::NeedsClarification { questions, .. }) => (
                "needs_clarification",
                None,
                None,
                questions.iter().map(|q| q.id.as_str()).collect(),
            ),
            Some(ModifyOutcome::NoChange { questions, .. }) => (
                "no_change",
                None,
                None,
                questions.iter().map(|q| q.id.as_str()).collect(),
            ),
            Some(ModifyOutcome::Unsupported { .. }) => ("unsupported", None, None, vec![]),
            Some(ModifyOutcome::Conflict { .. }) => ("conflict", None, None, vec![]),
            None => ("unstructured", None, None, vec![]),
        };
    serde_json::json!({
        "outcome_kind": kind, "plan_id": plan_id, "base_revision": revision,
        "question_ids": question_ids,
        "question_persisted": match &response.outcome {
            Some(ModifyOutcome::NeedsClarification { persisted, .. }) => Some(*persisted),
            _ => None,
        },
        "displayed_text_without_diffs": protopie_api::diagnostics::bounded_text(&safe.display_text()),
        "diff_omitted": diff_omitted,
    })
}

fn open_browser(url: &str) -> Result<(), String> {
    if !url.starts_with("http://127.0.0.1:") && !url.starts_with("http://localhost:") {
        return Err("Preview URL is not on the local machine".into());
    }
    #[cfg(target_os = "macos")]
    let mut command = std::process::Command::new("open");
    #[cfg(target_os = "linux")]
    let mut command = std::process::Command::new("xdg-open");
    #[cfg(target_os = "windows")]
    let mut command = {
        let mut command = std::process::Command::new("cmd");
        command.args(["/C", "start", ""]);
        command
    };
    command
        .arg(url)
        .spawn()
        .map(|_| ())
        .map_err(|e| e.to_string())
}

impl ChatView {
    pub fn new(client: ApiClient) -> Self {
        client.list_projects();
        let mut layout = Layout::new();
        let project_btn = layout.new_leaf(Style {
            flex_grow: 1.0,
            size: Size {
                width: Dimension::auto(),
                height: length(BTN_H),
            },
            ..Default::default()
        });
        let header = layout.new_with_children(
            Style {
                display: Display::Flex,
                flex_direction: FlexDirection::Row,
                align_items: Some(akar_layout::AlignItems::CENTER),
                size: Size {
                    width: Dimension::percent(1.0),
                    height: length(HEADER_H),
                },
                ..Default::default()
            },
            &[project_btn],
        );
        layout.set_padding(header, 0.0, 10.0, 0.0, 10.0);
        let messages_area = layout.new_leaf(Style {
            flex_grow: 1.0,
            size: Size {
                width: Dimension::percent(1.0),
                height: Dimension::auto(),
            },
            ..Default::default()
        });
        let input = layout.new_leaf(Style {
            flex_grow: 1.0,
            size: Size {
                width: Dimension::auto(),
                height: length(36.0_f32),
            },
            ..Default::default()
        });
        layout.set_margin(input, 0.0, 8.0, 0.0, 0.0);
        let send = layout.new_leaf(Style {
            size: Size {
                width: length(64.0_f32),
                height: length(36.0_f32),
            },
            ..Default::default()
        });
        let input_bar = layout.new_with_children(
            Style {
                display: Display::Flex,
                flex_direction: FlexDirection::Row,
                align_items: Some(akar_layout::AlignItems::CENTER),
                size: Size {
                    width: Dimension::percent(1.0),
                    height: length(INPUT_BAR_H),
                },
                ..Default::default()
            },
            &[input, send],
        );
        layout.set_padding(input_bar, 0.0, 10.0, 0.0, 10.0);
        let root = layout.new_with_children(
            Style {
                display: Display::Flex,
                flex_direction: FlexDirection::Column,
                size: Size {
                    width: Dimension::percent(1.0),
                    height: Dimension::percent(1.0),
                },
                ..Default::default()
            },
            &[header, messages_area, input_bar],
        );
        Self {
            client,
            layout,
            root,
            header,
            project_btn,
            projects: Vec::new(),
            selected: None,
            select_after_refresh: None,
            popup_open: false,
            popup_scroll: 0.0,
            entry_scroll: 0.0,
            entry_error: None,
            occlude_top: 0.0,
            new_project: None,
            messages_area,
            input,
            send,
            messages: vec![Message {
                role: Role::Agent,
                text: "Hi! Pick a project and describe a UI change, e.g. \"Need a top navigation\" then \"Add Contact Us\". \
                       I show a diff first: choose Apply changes or Discard. \
                       When I ask a question, click an option or reply with its number or text."
                    .into(),
                buffer: None,
            }],
            input_text: String::new(),
            edit: TextEditState::default(),
            scroll_y: 0.0,
            pending: 0,
            stick_to_bottom: true,
            review_project: None,
            review: Review::default(),
            review_rows: 0,
            last_sent: None,
            preview: PreviewUi::new(),
        }
    }

    fn push(&mut self, role: Role, text: String) {
        self.messages.push(Message {
            role,
            text,
            buffer: None,
        });
        self.stick_to_bottom = true;
    }

    fn submit(&mut self) {
        let prompt = self.input_text.trim().to_string();
        if prompt.is_empty() {
            return;
        }
        self.input_text.clear();
        self.edit = TextEditState::default();
        self.push(Role::User, prompt.clone());
        if prompt == protopie_api::ABORT_COMMAND {
            match &self.selected {
                Some(p) => {
                    self.pending += 1;
                    self.client.abort_interrupted_apply(p.path.clone());
                }
                None => self.push(
                    Role::Error,
                    "Select a project before aborting an interrupted application.".into(),
                ),
            }
            return;
        }
        // A new message replaces whatever was waiting for a decision.
        self.review = Review::default();
        self.pending += 1;
        match &self.selected {
            // Commands are previewed first; the diff is applied on request.
            Some(p) => {
                self.review_project = Some(p.clone());
                let sent = Sent::Command(prompt);
                self.last_sent = Some(sent.clone());
                self.client.send(p.path.clone(), sent, None)
            }
            None => self.client.chat(prompt),
        }
    }

    fn show_examples(&self) -> bool {
        self.selected.is_some() && self.messages.iter().all(|m| m.role != Role::User)
    }

    fn fill_example(&mut self, index: usize) {
        if let Some(example) = CHAT_EXAMPLES.get(index) {
            self.input_text = example.prompt.to_string();
            self.edit = TextEditState::default();
        }
    }

    /// Acts on a clicked review button.
    fn choose(&mut self, button: usize) {
        let Some(project) = self.review_project.clone() else {
            return;
        };
        let Some(b) = self.review.buttons.get(button).cloned() else {
            return;
        };
        let expected = self.review.expected.clone();
        // The buttons are rebuilt from the next reply.
        self.review = Review::default();
        match b.choice {
            Choice::Discard => {
                let _ = protopie_api::diagnostics::log_project_event(
                    std::path::Path::new(&project.path),
                    "gui",
                    serde_json::json!({
                        "kind": "discard", "plan_id": expected.as_ref().map(|e| &e.plan_id),
                        "base_revision": expected.as_ref().map(|e| e.revision),
                    }),
                );
                self.push(
                    Role::Agent,
                    "Discarded the preview. Nothing was changed.".into(),
                );
            }
            Choice::Apply => {
                let Some(expected) = expected else { return };
                self.pending += 1;
                self.push(Role::User, b.label);
                match self.last_sent.clone() {
                    Some(sent) => self.client.send(project.path, sent, Some(expected)),
                    None => self.pending -= 1,
                }
            }
            Choice::Answer {
                question_id,
                option_key,
            } => {
                self.push(Role::User, b.label);
                self.pending += 1;
                let sent = Sent::Answer {
                    question_id,
                    option_key,
                };
                self.last_sent = Some(sent.clone());
                self.client.send(project.path, sent, None);
            }
        }
    }

    fn handle_event(&mut self, event: Event) {
        match event {
            Event::Reply(result) => {
                self.pending = self.pending.saturating_sub(1);
                match result {
                    Ok(reply) => self.push(Role::Agent, reply),
                    Err(e) => self.push(Role::Error, format!("Request failed: {e}")),
                }
            }
            Event::Modified {
                applied,
                project_path,
                request_id,
                result,
            } => {
                self.pending = self.pending.saturating_sub(1);
                let diagnostic = match &result {
                    Ok(response) => response_diagnostic(response),
                    Err(error) => {
                        serde_json::json!({"error": protopie_api::diagnostics::bounded_text(error)})
                    }
                };
                let _ = protopie_api::diagnostics::log_project_event(
                    std::path::Path::new(&project_path),
                    "gui",
                    serde_json::json!({
                        "kind": "response", "request_id": request_id,
                        "applied_request": applied, "response": diagnostic,
                    }),
                );
                match result {
                    Ok(response) => {
                        self.push(Role::Agent, response.display_text());
                        self.review = crate::review::review_for(&response);
                        if applied {
                            self.last_sent = None;
                        }
                    }
                    Err(e) => {
                        self.review = Review::default();
                        self.push(Role::Error, format!("Request failed: {e}"));
                    }
                }
            }
            Event::Projects(Ok(list)) => {
                self.projects = list;
                if let Some(slug) = self.select_after_refresh.as_deref() {
                    if let Some(found) = refreshed_selection(&self.projects, None, Some(slug)) {
                        self.selected = Some(found);
                        self.select_after_refresh = None;
                    }
                    // An older in-flight list may arrive first; retain the new
                    // selection until the list requested after creation arrives.
                } else {
                    self.selected =
                        refreshed_selection(&self.projects, self.selected.as_ref(), None);
                }
                if let Some(epoch) = self
                    .preview
                    .select(self.selected.as_ref().map(|p| p.path.as_str()))
                {
                    self.client.preview_runtime(epoch);
                    if let Some(epoch) = self.preview.mark_status_requested() {
                        self.client.preview_status(epoch);
                    }
                }
            }
            Event::Projects(Err(e)) => {
                self.entry_error = Some(format!("Could not load projects: {e}"));
                self.push(Role::Error, format!("Could not load projects: {e}"));
            }
            Event::ProjectCreated(Ok(created)) => {
                self.entry_error = None;
                self.select_project(Some(ProjectInfo {
                    name: created.slug.clone(),
                    path: created.path.clone(),
                    last_modified_unix_ms: Some(now_unix_ms()),
                    unavailable_reason: None,
                }));
                self.push(
                    Role::Agent,
                    format!("Created project \"{}\".", created.slug),
                );
                self.select_after_refresh = Some(created.slug);
                self.client.list_projects();
            }
            Event::ProjectCreated(Err(e)) => {
                self.entry_error = Some(format!("Could not create project: {e}"));
                self.push(Role::Error, format!("Could not create project: {e}"));
            }
            Event::PreviewRuntime { epoch, result } => self.preview.receive_runtime(epoch, result),
            Event::PreviewStatus {
                epoch,
                operation,
                result,
            } => {
                if let Some(url) = self.preview.receive_status(epoch, operation, result) {
                    if let Err(error) = open_browser(&url) {
                        self.push(Role::Error, format!("Could not open browser: {error}"));
                    }
                }
                self.fetch_preview_logs();
            }
            Event::PreviewLogs { epoch, result } => self.preview.receive_logs(epoch, result),
        }
    }

    fn fetch_preview_logs(&mut self) {
        if let Some((epoch, path, cursor)) = self.preview.mark_logs_requested() {
            self.client.preview_logs(epoch, path, cursor);
        }
    }

    fn poll_preview(&mut self) {
        if self.preview.poll_due(std::time::Instant::now()) {
            if let Some(epoch) = self.preview.mark_status_requested() {
                self.client.preview_status(epoch);
            }
            self.fetch_preview_logs();
        }
    }

    fn preview_action(&mut self, action: Action) {
        if let Some((epoch, path)) = self.preview.begin_action(action) {
            self.client.preview_action(epoch, action, path);
        }
    }

    /// Draws one frame. `size` is the logical (scale-independent) window size.
    pub fn render(&mut self, core: &mut AkarCore, size: [f32; 2], cursor_visible: bool) {
        while let Some(event) = self.client.try_recv() {
            self.handle_event(event);
        }
        self.poll_preview();

        self.layout.compute(
            self.root,
            (Some(size[0]), Some(size[1])),
            |_, _, _, _, _| Size::ZERO,
        );

        core.draw_list
            .push_quad(quad([0.0, 0.0, size[0], size[1]], THEME.base_100, 0.0));

        // The modal replaces the chat contents: akar draws all text above all quads, so
        // underlying text would show through its scrim.
        if self.new_project.is_some() {
            self.draw_new_project_modal(core, size, cursor_visible);
            return;
        }

        self.draw_header(core);
        if self.selected.is_none() {
            self.draw_entry(core, size);
            return;
        }
        let preview_height = self.draw_preview(core, size);
        let reserved = self.layout_review(size[0]);
        let example_height = self.draw_examples(core, size, preview_height, reserved);
        self.draw_messages(core, reserved, preview_height + example_height);
        self.draw_review(core, size, reserved);

        // Input bar.
        let bar_y = size[1] - INPUT_BAR_H;
        core.draw_list
            .push_quad(quad([0.0, bar_y, size[0], 1.0], THEME.base_300, 0.0));
        let input = akar_text_input(
            core,
            &self.layout,
            self.input,
            &mut self.input_text,
            &mut self.edit,
            "Message the agent…",
            cursor_visible,
            &THEME,
        );
        let send = akar_button(
            core,
            &self.layout,
            self.send,
            "Send",
            ButtonVariant::Solid,
            &THEME,
        );
        if input.submitted || send.clicked {
            self.submit();
        }
    }

    fn project_label(&self) -> &str {
        self.selected
            .as_ref()
            .map_or("Select project", |p| p.name.as_str())
    }

    fn start_new_project(&mut self, core: &mut AkarCore) {
        self.entry_error = None;
        self.new_project = Some(NewProjectForm {
            name: String::new(),
            edit: TextEditState::default(),
        });
        core.input.focused_id = None;
        self.popup_open = false;
    }

    fn select_project(&mut self, project: Option<ProjectInfo>) {
        if self.selected.as_ref().map(|p| &p.path) != project.as_ref().map(|p| &p.path) {
            self.review = Review::default();
            self.review_project = None;
            self.last_sent = None;
        }
        self.selected = project.filter(is_available);
        if let Some(epoch) = self
            .preview
            .select(self.selected.as_ref().map(|p| p.path.as_str()))
        {
            self.client.preview_runtime(epoch);
            if let Some(epoch) = self.preview.mark_status_requested() {
                self.client.preview_status(epoch);
            }
        }
        self.popup_open = false;
    }

    pub fn close(&mut self) {
        self.preview.select(None);
    }

    fn draw_preview(&mut self, core: &mut AkarCore, size: [f32; 2]) -> f32 {
        let top = HEADER_H;
        let height = PREVIEW_BAR_H
            + if self.preview.logs_open {
                PREVIEW_LOG_H
            } else {
                0.0
            };
        let width = size[0].max(0.0);
        core.draw_list
            .push_quad(quad([0.0, top, width, height], THEME.base_200, 0.0));
        let status = self.preview.status_text();
        Self::draw_text(
            core,
            BUF_PREVIEW,
            &status,
            THEME.font_size_sm,
            [10.0, top + 7.0],
            clip_below(
                [10.0, top + 5.0, (width - 20.0).max(0.0), 34.0],
                self.occlude_top,
            ),
            THEME.base_content,
            0.0,
        );
        let controls = self.preview.controls();
        let button_y = top + 43.0;
        let gap = 5.0;
        let button_w = ((width - 20.0 - gap * 3.0) / 4.0).max(0.0);
        let mut chosen = None;
        for (i, (label, enabled)) in [
            ("Launch", controls.launch),
            ("Stop", controls.stop),
            ("Restart", controls.restart),
            ("Open", controls.open),
        ]
        .into_iter()
        .enumerate()
        {
            let rect = [10.0 + i as f32 * (button_w + gap), button_y, button_w, 28.0];
            core.draw_list.push_quad(quad(
                rect,
                if enabled {
                    THEME.primary
                } else {
                    THEME.base_300
                },
                THEME.radius_field,
            ));
            Self::draw_text(
                core,
                BUF_PREVIEW + 1 + i as u64,
                label,
                THEME.font_size_sm,
                [rect[0] + 5.0, rect[1] + 6.0],
                clip_below(rect, self.occlude_top),
                if enabled {
                    THEME.primary_content
                } else {
                    THEME.base_content
                },
                0.0,
            );
            if enabled && !self.popup_open && core.input.is_clicked(rect) {
                chosen = Some(i);
            }
        }
        let log_toggle = [10.0, top + 78.0, (width - 20.0).max(0.0), 22.0];
        Self::draw_text(
            core,
            BUF_PREVIEW + 5,
            if self.preview.logs_open {
                "▼ Preview logs"
            } else {
                "▶ Preview logs"
            },
            THEME.font_size_sm,
            [10.0, top + 80.0],
            clip_below(log_toggle, self.occlude_top),
            THEME.base_content,
            0.0,
        );
        if !self.popup_open && core.input.is_clicked(log_toggle) {
            self.preview.logs_open = !self.preview.logs_open;
            if self.preview.logs_open {
                self.fetch_preview_logs();
            }
        }
        if self.preview.logs_open {
            let logs = self.preview.visible_logs();
            let log_top = top + PREVIEW_BAR_H;
            let start = logs.len().saturating_sub(4);
            let text = if logs.is_empty() {
                "No preview logs yet".into()
            } else {
                logs[start..].join("\n")
            };
            Self::draw_text(
                core,
                BUF_PREVIEW + 6,
                &text,
                THEME.font_size_sm,
                [10.0, log_top + 4.0],
                clip_below(
                    [10.0, log_top, (width - 20.0).max(0.0), PREVIEW_LOG_H - 4.0],
                    self.occlude_top,
                ),
                THEME.base_content,
                0.0,
            );
        }
        match chosen {
            Some(0) => self.preview_action(Action::Launch),
            Some(1) => self.preview_action(Action::Stop),
            Some(2) => self.preview_action(Action::Restart),
            Some(3) => {
                if let Some(url) = self.preview.running_url() {
                    if let Err(error) = open_browser(url) {
                        self.push(Role::Error, format!("Could not open browser: {error}"));
                    }
                }
            }
            _ => {}
        }
        height
    }

    /// First-run and project-switch view. The list remains visible for unsupported
    /// folders, but only eligible projects can be continued.
    fn draw_entry(&mut self, core: &mut AkarCore, size: [f32; 2]) {
        let width = (size[0] - 32.0).max(0.0);
        Self::draw_text(
            core,
            BUF_ENTRY,
            "Your projects",
            22.0,
            [16.0, 64.0],
            [16.0, 60.0, width, 30.0],
            THEME.base_content,
            0.0,
        );
        Self::draw_text(
            core,
            BUF_ENTRY + 1,
            "Create a project or continue where you left off.",
            THEME.font_size_sm,
            [16.0, 98.0],
            [16.0, 96.0, width, 40.0],
            THEME.base_content,
            0.0,
        );
        let create = [16.0, 140.0, width, 42.0];
        core.draw_list
            .push_quad(quad(create, THEME.primary, THEME.radius_field));
        Self::draw_text(
            core,
            BUF_ENTRY + 2,
            "+ New Project",
            THEME.font_size_base,
            [create[0] + 14.0, create[1] + 10.0],
            create,
            THEME.primary_content,
            0.0,
        );
        if !self.popup_open && core.input.is_clicked(create) {
            self.start_new_project(core);
            return;
        }
        let section_y = if let Some(error) = &self.entry_error {
            Self::draw_text(
                core,
                BUF_ENTRY + 5,
                error,
                THEME.font_size_sm,
                [16.0, 194.0],
                [16.0, 190.0, width, 48.0],
                THEME.error,
                0.0,
            );
            250.0
        } else {
            205.0
        };
        Self::draw_text(
            core,
            BUF_ENTRY + 3,
            "Continue",
            18.0,
            [16.0, section_y],
            [16.0, section_y - 4.0, width, 28.0],
            THEME.base_content,
            0.0,
        );
        let list_y = section_y + 33.0;
        let list = [16.0, list_y, width, (size[1] - list_y - 12.0).max(0.0)];
        if self.projects.is_empty() {
            Self::draw_text(
                core,
                BUF_ENTRY + 4,
                "No projects yet. Create one to start editing.",
                THEME.font_size_sm,
                [16.0, list_y + 12.0],
                list,
                THEME.base_content,
                0.0,
            );
            return;
        }
        let max_scroll = (self.projects.len() as f32 * ENTRY_ROW_H - list[3]).max(0.0);
        if core.input.is_hovering(list) && !self.popup_open {
            self.entry_scroll -= core.input.scroll_delta.y;
        }
        self.entry_scroll = self.entry_scroll.clamp(0.0, max_scroll);
        let mut picked = None;
        for (i, project) in self.projects.iter().enumerate() {
            let row = [
                list[0],
                list[1] + i as f32 * ENTRY_ROW_H - self.entry_scroll,
                list[2],
                ENTRY_ROW_H - 6.0,
            ];
            let visible = clip_to(row, list);
            if visible[3] <= 0.0 {
                continue;
            }
            let available = is_available(project);
            let hovered = core.input.is_hovering(visible) && !self.popup_open;
            core.draw_list.push_quad(quad(
                visible,
                if hovered && available {
                    THEME.base_300
                } else {
                    THEME.base_200
                },
                THEME.radius_field,
            ));
            let name_clip = clip_to([row[0] + 12.0, row[1] + 8.0, row[2] - 24.0, 25.0], list);
            Self::draw_text(
                core,
                BUF_ENTRY + 100 + i as u64 * 2,
                &project.name,
                THEME.font_size_base,
                [row[0] + 12.0, row[1] + 9.0],
                name_clip,
                THEME.base_content,
                0.0,
            );
            let detail = project.unavailable_reason.as_ref().map_or_else(
                || modified_label(now_unix_ms(), project.last_modified_unix_ms),
                |reason| format!("Unavailable: {reason}"),
            );
            let detail_clip = clip_to([row[0] + 12.0, row[1] + 37.0, row[2] - 24.0, 39.0], list);
            Self::draw_text(
                core,
                BUF_ENTRY + 101 + i as u64 * 2,
                &detail,
                THEME.font_size_sm,
                [row[0] + 12.0, row[1] + 38.0],
                detail_clip,
                THEME.base_content,
                0.0,
            );
            if hovered && available && core.input.is_clicked(visible) {
                picked = Some(i);
            }
        }
        if let Some(i) = picked {
            self.select_project(Some(self.projects[i].clone()));
        }
    }

    fn draw_text(
        core: &mut AkarCore,
        id: u64,
        text: &str,
        size: f32,
        pos: [f32; 2],
        clip: [f32; 4],
        color: u32,
        z: f32,
    ) {
        let buf = core.text_pipeline.set_text(
            Some(id),
            text,
            glyphon::Metrics::new(size, size * 1.2),
            Some(clip[2]),
            None,
            None,
        );
        core.draw_list.push_text(TextCall {
            buffer_id: buf,
            x: pos[0],
            y: pos[1],
            clip,
            color: rgba(color),
            z,
        });
    }

    /// Header: the project dropdown button and, when open, its popup list.
    fn draw_header(&mut self, core: &mut AkarCore) {
        let header = self.layout.rect(self.header);
        let btn = self.layout.rect(self.project_btn);
        core.draw_list.push_quad(quad(
            [header[0], header[1] + header[3] - 1.0, header[2], 1.0],
            THEME.base_300,
            0.0,
        ));

        let hovered = core.input.is_hovering(btn);
        core.draw_list.push_quad(QuadCall {
            border_color: rgba(THEME.base_300),
            border_width: THEME.border_width,
            ..quad(
                btn,
                if hovered {
                    THEME.base_300
                } else {
                    THEME.base_200
                },
                THEME.radius_field,
            )
        });
        let label = self.project_label().to_string();
        let text_clip = [btn[0], btn[1], btn[2] - 28.0, btn[3]];
        let ty = btn[1] + (btn[3] - THEME.font_size_base * 1.2) / 2.0;
        Self::draw_text(
            core,
            BUF_LABEL,
            &label,
            THEME.font_size_base,
            [btn[0] + THEME.padding_x, ty],
            text_clip,
            THEME.base_content,
            0.0,
        );
        Self::draw_text(
            core,
            BUF_CHEVRON,
            "\u{25BC}",
            THEME.font_size_sm,
            [
                btn[0] + btn[2] - 20.0,
                btn[1] + (btn[3] - THEME.font_size_sm * 1.2) / 2.0,
            ],
            btn,
            THEME.base_content,
            0.0,
        );

        // Popup rows: "New Project", "No project", then the projects.
        let total_rows = FIXED_ROWS + self.projects.len();
        let visible = total_rows.min(MAX_ROWS);
        let popup = [
            btn[0],
            btn[1] + btn[3] + 4.0,
            btn[2],
            visible as f32 * ROW_H,
        ];
        let toggled = core.input.is_clicked(btn);
        if self.popup_open
            && core.input.mouse_buttons_pressed[0]
            && !core.input.is_hovering(btn)
            && !core.input.is_hovering(popup)
        {
            self.popup_open = false;
        }
        if toggled {
            self.popup_open = !self.popup_open;
            self.popup_scroll = 0.0;
        }
        self.occlude_top = 0.0;
        if !self.popup_open {
            return;
        }
        self.occlude_top = popup[1] + popup[3] + 4.0;

        // The wheel scrolls long lists.
        let max_scroll = (total_rows as f32 * ROW_H - popup[3]).max(0.0);
        if core.input.is_hovering(popup) {
            self.popup_scroll -= core.input.scroll_delta.y;
            core.input.scroll_delta.y = 0.0; // don't also scroll the messages underneath
        }
        self.popup_scroll = self.popup_scroll.clamp(0.0, max_scroll);

        core.draw_list.push_quad(QuadCall {
            border_color: rgba(THEME.base_300),
            border_width: 1.0,
            shadow_blur: 12.0,
            shadow_color: [0.0, 0.0, 0.0, 0.25],
            z: Z_OVERLAY,
            ..quad(popup, THEME.base_100, THEME.radius_field)
        });
        let inner = [
            popup[0] + 1.0,
            popup[1] + 1.0,
            popup[2] - 2.0,
            popup[3] - 2.0,
        ];
        let mut chosen = None;
        for i in 0..total_rows {
            let row = [
                inner[0],
                inner[1] + i as f32 * ROW_H - self.popup_scroll,
                inner[2],
                ROW_H,
            ];
            if row[1] + row[3] < inner[1] || row[1] > inner[1] + inner[3] {
                continue;
            }
            let visible_row = clip_to(row, inner);
            let (text, active) = match i {
                NEW_ROW => ("+ New Project".to_string(), false),
                NONE_ROW => ("No project".to_string(), self.selected.is_none()),
                _ => {
                    let p = &self.projects[i - FIXED_ROWS];
                    (
                        if is_available(p) {
                            p.name.clone()
                        } else {
                            format!("{} (Unavailable)", p.name)
                        },
                        self.selected.as_ref().is_some_and(|s| s.path == p.path),
                    )
                }
            };
            if core.input.is_hovering(visible_row) {
                core.draw_list.push_quad(QuadCall {
                    z: Z_OVERLAY,
                    ..quad(visible_row, THEME.base_300, 0.0)
                });
                if core.input.is_clicked(visible_row) {
                    chosen = Some(i);
                }
            }
            let color = if active {
                THEME.primary
            } else {
                THEME.base_content
            };
            Self::draw_text(
                core,
                BUF_ROW + i as u64,
                &text,
                THEME.font_size_base,
                [
                    row[0] + THEME.padding_x,
                    row[1] + (ROW_H - THEME.font_size_base * 1.2) / 2.0,
                ],
                visible_row,
                color,
                Z_OVERLAY,
            );
        }
        if let Some(i) = chosen {
            self.popup_open = false;
            match i {
                NEW_ROW => {
                    self.start_new_project(core);
                }
                NONE_ROW => self.select_project(None),
                _ => self.select_project(Some(self.projects[i - FIXED_ROWS].clone())),
            }
        }
    }

    /// Modal asking for the new project's name. Enter or Create submits; Esc, Cancel, the
    /// close button or a click outside the panel cancels.
    fn draw_new_project_modal(
        &mut self,
        core: &mut AkarCore,
        size: [f32; 2],
        cursor_visible: bool,
    ) {
        let viewport = [0.0, 0.0, size[0], size[1]];
        // `modal_begin` lays out into a layout it is given; its nodes are rebuilt every frame.
        let mut modal_layout = Layout::new();
        modal_layout.set_namespace_id(MODAL_NS);
        let m = modal_begin(
            core,
            &mut modal_layout,
            viewport,
            "New project",
            300.0,
            150.0,
            &THEME,
        );
        let mut cancel = m.close_requested && {
            // akar reports any click in the viewport; only honour clicks outside the panel
            // or on the close button.
            let close_btn = [
                m.panel_rect[0] + m.panel_rect[2] - 40.0,
                m.panel_rect[1],
                40.0,
                40.0,
            ];
            !core.input.is_hovering(m.panel_rect) || core.input.is_hovering(close_btn)
        };
        let escape = core.input.keys_pressed.contains(&Key::Escape);

        // Field and buttons, laid out inside the panel's content area.
        let fw = m.content_rect[2] - 32.0;
        let mut fl = Layout::new();
        fl.set_namespace_id(MODAL_NS + 1);
        let leaf = |fl: &mut Layout, w: Dimension, h: f32, grow: f32| {
            fl.new_leaf(Style {
                flex_grow: grow,
                size: Size {
                    width: w,
                    height: length(h),
                },
                ..Default::default()
            })
        };
        let field = leaf(&mut fl, length(fw), 32.0, 0.0);
        fl.set_margin(field, 0.0, 0.0, 12.0, 0.0);
        let spacer = leaf(&mut fl, Dimension::auto(), 32.0, 1.0);
        let cancel_node = leaf(&mut fl, length(80.0_f32), 32.0, 0.0);
        fl.set_margin(cancel_node, 0.0, 8.0, 0.0, 0.0);
        let create_node = leaf(&mut fl, length(80.0_f32), 32.0, 0.0);
        let row = fl.new_with_children(
            Style {
                display: Display::Flex,
                flex_direction: FlexDirection::Row,
                size: Size {
                    width: length(fw),
                    height: length(32.0_f32),
                },
                ..Default::default()
            },
            &[spacer, cancel_node, create_node],
        );
        let root = fl.new_with_children(
            Style {
                display: Display::Flex,
                flex_direction: FlexDirection::Column,
                size: Size {
                    width: Dimension::percent(1.0),
                    height: Dimension::percent(1.0),
                },
                ..Default::default()
            },
            &[field, row],
        );
        fl.set_padding(
            root,
            m.content_rect[1] + 8.0,
            0.0,
            0.0,
            m.content_rect[0] + 16.0,
        );
        fl.compute(root, (Some(size[0]), Some(size[1])), |_, _, _, _, _| {
            Size::ZERO
        });

        // akar's text input and button quads sit at z=0, behind the modal panel, so draw the
        // field background and buttons here, above the panel.
        let field_rect = fl.rect(field);
        core.draw_list.push_quad(QuadCall {
            border_color: rgba(THEME.primary),
            border_width: THEME.border_width,
            z: Z_FLOAT + 0.01,
            ..quad(field_rect, THEME.base_100, THEME.radius_field)
        });
        let mut create = false;
        for (node, label, is_create) in [
            (cancel_node, "Cancel", false),
            (create_node, "Create", true),
        ] {
            let r = fl.rect(node);
            let hovered = core.input.is_hovering(r);
            let fill = match (is_create, hovered) {
                (true, _) => THEME.primary,
                (false, false) => THEME.base_300,
                (false, true) => THEME.base_100,
            };
            core.draw_list.push_quad(QuadCall {
                z: Z_FLOAT + 0.01,
                ..quad(r, fill, THEME.radius_field)
            });
            let color = if is_create {
                THEME.primary_content
            } else {
                THEME.base_content
            };
            let metrics = glyphon::Metrics::new(THEME.font_size_base, THEME.font_size_base * 1.2);
            let buf = core.text_pipeline.set_text(
                Some(fl.widget_id(node)),
                label,
                metrics,
                None,
                None,
                None,
            );
            let tw = core.text_pipeline.measure(buf, None).x;
            core.draw_list.push_text(TextCall {
                buffer_id: buf,
                x: r[0] + (r[2] - tw) / 2.0,
                y: r[1] + (r[3] - metrics.line_height) / 2.0,
                clip: r,
                color: rgba(color),
                z: Z_FLOAT,
            });
            if core.input.is_clicked(r) {
                if is_create {
                    create = true;
                } else {
                    cancel = true;
                }
            }
        }

        let form = self.new_project.as_mut().expect("modal is open");
        let field_id = fl.widget_id(field);
        if core.input.focused_id.is_none() {
            core.input.focused_id = Some(field_id);
        }
        let input = akar_text_input(
            core,
            &fl,
            field,
            &mut form.name,
            &mut form.edit,
            "Project name",
            cursor_visible,
            &THEME,
        );
        modal_end(core);

        let submit = input.submitted || create;
        if escape || cancel {
            self.new_project = None;
            core.input.focused_id = None;
        } else if submit && !form.name.trim().is_empty() {
            let name = form.name.trim().to_string();
            self.new_project = None;
            core.input.focused_id = None;
            self.client
                .create_project(PROJECTS_BASE_PATH.to_string(), name);
        }
    }

    fn button_width(label: &str) -> f32 {
        label.chars().count() as f32 * THEME.font_size_base * 0.6 + 2.0 * CHOICE_PAD_X
    }

    /// Wraps the review buttons into rows for `width`; returns the height the
    /// strip takes above the input bar.
    fn layout_review(&mut self, width: f32) -> f32 {
        if self.review.buttons.is_empty() {
            self.review_rows = 0;
            return 0.0;
        }
        let (mut rows, mut x) = (1, CHOICE_GAP);
        for b in &self.review.buttons {
            let w = Self::button_width(&b.label);
            if x + w + CHOICE_GAP > width && x > CHOICE_GAP {
                rows += 1;
                x = CHOICE_GAP;
            }
            x += w + CHOICE_GAP;
        }
        self.review_rows = rows;
        rows as f32 * (CHOICE_H + CHOICE_GAP) + CHOICE_GAP
    }

    /// Buttons for the pending preview or question, above the input bar.
    fn draw_review(&mut self, core: &mut AkarCore, size: [f32; 2], reserved: f32) {
        if reserved == 0.0 {
            return;
        }
        let top = size[1] - INPUT_BAR_H - reserved;
        core.draw_list
            .push_quad(quad([0.0, top, size[0], reserved], THEME.base_200, 0.0));
        let (mut x, mut y) = (CHOICE_GAP, top + CHOICE_GAP);
        let mut clicked = None;
        for (i, b) in self.review.buttons.iter().enumerate() {
            let w = Self::button_width(&b.label);
            if x + w + CHOICE_GAP > size[0] && x > CHOICE_GAP {
                x = CHOICE_GAP;
                y += CHOICE_H + CHOICE_GAP;
            }
            let rect = [x, y, w, CHOICE_H];
            let hovered = core.input.is_hovering(rect);
            let primary = b.choice == Choice::Apply;
            core.draw_list.push_quad(QuadCall {
                border_color: rgba(THEME.base_300),
                border_width: THEME.border_width,
                ..quad(
                    rect,
                    match (primary, hovered) {
                        (true, _) => THEME.primary,
                        (false, true) => THEME.base_300,
                        (false, false) => THEME.base_100,
                    },
                    THEME.radius_field,
                )
            });
            Self::draw_text(
                core,
                BUF_CHOICE + i as u64,
                &b.label,
                THEME.font_size_base,
                [
                    x + CHOICE_PAD_X,
                    y + (CHOICE_H - THEME.font_size_base * 1.2) / 2.0,
                ],
                rect,
                if primary {
                    THEME.primary_content
                } else {
                    THEME.base_content
                },
                0.0,
            );
            if core.input.is_clicked(rect) {
                clicked = Some(i);
            }
            x += w + CHOICE_GAP;
        }
        if let Some(i) = clicked {
            self.choose(i);
        }
    }

    fn draw_examples(
        &mut self,
        core: &mut AkarCore,
        size: [f32; 2],
        preview_height: f32,
        reserved: f32,
    ) -> f32 {
        if !self.show_examples() {
            return 0.0;
        }
        let top = HEADER_H + preview_height + 8.0;
        let available = (size[1] - INPUT_BAR_H - reserved - top - 12.0).max(0.0);
        let rows = CHAT_EXAMPLES
            .len()
            .min((available / EXAMPLE_ROW_H).floor() as usize);
        if rows == 0 {
            return 0.0;
        }
        let mut clicked = None;
        for (i, example) in CHAT_EXAMPLES.iter().take(rows).enumerate() {
            let rect = [
                12.0,
                top + i as f32 * EXAMPLE_ROW_H,
                (size[0] - 24.0).max(0.0),
                EXAMPLE_ROW_H - 5.0,
            ];
            core.draw_list
                .push_quad(quad(rect, THEME.base_200, THEME.radius_field));
            Self::draw_text(
                core,
                BUF_EXAMPLE + i as u64 * 2,
                example.prompt,
                THEME.font_size_base,
                [rect[0] + 10.0, rect[1] + 4.0],
                rect,
                THEME.base_content,
                0.0,
            );
            let detail = format!("{} · {}", example.prerequisite, example.next_step);
            Self::draw_text(
                core,
                BUF_EXAMPLE + i as u64 * 2 + 1,
                &detail,
                THEME.font_size_sm,
                [rect[0] + 10.0, rect[1] + 23.0],
                rect,
                THEME.base_content,
                0.0,
            );
            if !self.popup_open && core.input.is_clicked(rect) {
                clicked = Some(i);
            }
        }
        if let Some(i) = clicked {
            self.fill_example(i);
        }
        rows as f32 * EXAMPLE_ROW_H + 8.0
    }

    fn draw_messages(&mut self, core: &mut AkarCore, reserved_bottom: f32, reserved_top: f32) {
        let mut area = self.layout.rect(self.messages_area);
        area[1] += reserved_top;
        area[3] = (area[3] - reserved_bottom - reserved_top).max(0.0);
        let max_bubble_w = area[2] * 0.82;
        let max_text_w = max_bubble_w - 2.0 * BUBBLE_PAD_X;
        let metrics = glyphon::Metrics::new(THEME.font_size_base, THEME.font_size_base * 1.4);

        // Measure pass: size every bubble so the scroll extent is known.
        let mut sizes = Vec::with_capacity(self.messages.len() + 1);
        let typing = self.pending > 0;
        let mut typing_buf = None;
        for m in &mut self.messages {
            let id = core.text_pipeline.set_text(
                m.buffer,
                &m.text,
                metrics,
                Some(max_text_w),
                None,
                None,
            );
            m.buffer = Some(id);
            let s = core.text_pipeline.measure(id, Some(max_text_w));
            sizes.push([
                s.x.ceil() + 2.0 * BUBBLE_PAD_X,
                s.y.ceil() + 2.0 * BUBBLE_PAD_Y,
            ]);
        }
        if typing {
            let id =
                core.text_pipeline
                    .set_text(Some(2), "…", metrics, Some(max_text_w), None, None);
            typing_buf = Some(id);
            let s = core.text_pipeline.measure(id, Some(max_text_w));
            sizes.push([
                s.x.ceil() + 2.0 * BUBBLE_PAD_X,
                s.y.ceil() + 2.0 * BUBBLE_PAD_Y,
            ]);
        }

        let content_h: f32 = sizes.iter().map(|s| s[1] + MSG_GAP).sum::<f32>() + 2.0 * MSG_MARGIN;
        if self.stick_to_bottom {
            self.scroll_y = f32::MAX; // clamped to the bottom by scroll_area_begin
            self.stick_to_bottom = false;
        }
        let resp = scroll_area_begin(core, area, &mut self.scroll_y, content_h);

        let mut y = resp.content_y + MSG_MARGIN;
        for (i, size) in sizes.iter().enumerate() {
            let (role, buf) = match self.messages.get(i) {
                Some(m) => (m.role, m.buffer.unwrap()),
                None => (Role::Agent, typing_buf.unwrap()),
            };
            let x = match role {
                Role::User => area[0] + area[2] - MSG_MARGIN - size[0],
                _ => area[0] + MSG_MARGIN,
            };
            let (fill, text_color) = match role {
                Role::User => (THEME.primary_content, THEME.primary),
                Role::Agent => (THEME.base_300, THEME.base_content),
                Role::Error => (THEME.error, THEME.error_content),
            };
            let rect = [x, y, size[0], size[1]];
            core.draw_list.push_quad(quad(rect, fill, THEME.radius_box));
            core.draw_list.push_text(TextCall {
                buffer_id: buf,
                x: x + BUBBLE_PAD_X,
                y: y + BUBBLE_PAD_Y,
                clip: clip_below(rect, self.occlude_top),
                color: rgba(text_color),
                z: 0.0,
            });
            y += size[1] + MSG_GAP;
        }
        scroll_area_end(core);
    }
}

#[cfg(test)]
mod entry_tests {
    use super::*;
    use protopie_api::{
        ModifyOutcome, ModifyProjectResponse, PreviewState, PreviewStatusResponse, QuestionSummary,
    };

    fn project(name: &str, reason: Option<&str>) -> ProjectInfo {
        ProjectInfo {
            name: name.into(),
            path: format!("/projects/{name}"),
            last_modified_unix_ms: Some(120_000),
            unavailable_reason: reason.map(str::to_owned),
        }
    }

    #[test]
    fn continue_and_new_project_select_only_eligible_folders() {
        let old = project("old", Some("Create a project from the reference template."));
        let existing = project("existing", None);
        let created = project("created", None);
        let projects = vec![old.clone(), existing.clone(), created.clone()];
        assert_eq!(
            refreshed_selection(&projects, Some(&existing), None),
            Some(existing)
        );
        assert_eq!(refreshed_selection(&projects, Some(&old), None), None);
        assert_eq!(
            refreshed_selection(&projects, None, Some("created")),
            Some(created)
        );
        assert_eq!(refreshed_selection(&projects, None, Some("old")), None);
    }

    #[test]
    fn model_modification_time_is_presented_in_entry() {
        assert_eq!(modified_label(120_000, Some(120_000)), "Modified just now");
        assert_eq!(
            modified_label(3_720_000, Some(120_000)),
            "Modified 1 hr ago"
        );
        assert_eq!(modified_label(120_000, None), "Modified time unavailable");
    }

    #[test]
    fn new_project_is_ready_even_if_an_old_list_reply_arrives_first() {
        let client = ApiClient::new("http://127.0.0.1:1".into());
        let mut view = ChatView::new(client);
        view.handle_event(Event::ProjectCreated(Ok(
            protopie_api::CreateProjectResponse {
                slug: "created".into(),
                path: "/projects/created".into(),
            },
        )));
        assert_eq!(view.selected.as_ref().unwrap().name, "created");
        view.handle_event(Event::Projects(Ok(vec![project("existing", None)])));
        assert_eq!(view.selected.as_ref().unwrap().name, "created");
        view.handle_event(Event::Projects(Ok(vec![project("created", None)])));
        assert_eq!(view.selected.as_ref().unwrap().name, "created");
        assert!(view.select_after_refresh.is_none());
        view.select_project(Some(project("old", Some("Unsupported project"))));
        assert!(view.selected.is_none());
    }

    #[test]
    fn example_chips_fill_input_without_sending_or_changing_review() {
        let client = ApiClient::new("http://127.0.0.1:1".into());
        let mut view = ChatView::new(client);
        view.selected = Some(project("example", None));
        assert!(view.show_examples());
        let initial_messages = view.messages.len();
        view.fill_example(0);
        assert_eq!(view.input_text, CHAT_EXAMPLES[0].prompt);
        assert_eq!(view.messages.len(), initial_messages);
        assert_eq!(view.pending, 0);
        assert_eq!(view.review, Review::default());
        view.fill_example(4);
        assert_eq!(view.input_text, CHAT_EXAMPLES[4].prompt);
        assert!(CHAT_EXAMPLES[4].prerequisite.contains("Doctors page"));
        view.messages.push(Message {
            role: Role::User,
            text: "sent".into(),
            buffer: None,
        });
        assert!(!view.show_examples());
    }

    #[test]
    fn gui_response_diagnostic_records_display_without_diff_content() {
        let response = ModifyProjectResponse {
            reply: "Preview: add page".into(),
            outcome: Some(ModifyOutcome::Preview {
                plan_id: "plan-1".into(),
                base_revision: 9,
                changed_files: vec!["src/pages/DoctorsPage.tsx".into()],
                diffs: vec![protopie_api::FileDiff {
                    path: "src/pages/DoctorsPage.tsx".into(),
                    diff: "@@\n+SECRET_SOURCE_TOKEN".into(),
                }],
                questions: vec![],
            }),
        };
        let diagnostic = response_diagnostic(&response);
        let encoded = diagnostic.to_string();
        assert_eq!(diagnostic["outcome_kind"], "preview");
        assert_eq!(diagnostic["plan_id"], "plan-1");
        assert_eq!(diagnostic["base_revision"], 9);
        assert_eq!(diagnostic["diff_omitted"], true);
        assert!(!encoded.contains("SECRET_SOURCE_TOKEN"));
    }

    #[test]
    fn review_and_structured_questions_survive_preview_running_and_stopped() {
        let client = ApiClient::new("http://127.0.0.1:1".into());
        let mut view = ChatView::new(client);
        view.selected = Some(project("example", None));
        let path = view.selected.as_ref().unwrap().path.clone();
        let epoch = view.preview.select(Some(&path)).unwrap();
        let response = ModifyProjectResponse {
            reply: "received: Need a top navigation".into(),
            outcome: Some(ModifyOutcome::Preview {
                plan_id: "plan".into(),
                base_revision: 3,
                changed_files: vec!["src/components/TopNav.tsx".into()],
                diffs: vec![],
                questions: vec![],
            }),
        };
        view.handle_event(Event::Modified {
            applied: false,
            project_path: path.clone(),
            request_id: "test-1".into(),
            result: Ok(response),
        });
        assert_eq!(view.review.buttons.len(), 2);
        assert_eq!(view.review.expected.as_ref().unwrap().revision, 3);
        for state in [
            PreviewState::Running {
                url: "http://127.0.0.1:5173".into(),
            },
            PreviewState::Stopped,
        ] {
            view.preview.receive_status(
                epoch,
                false,
                Ok(PreviewStatusResponse {
                    project_path: Some(path.clone()),
                    generation: 1,
                    state,
                }),
            );
            assert_eq!(view.review.buttons.len(), 2);
            assert_eq!(view.review.expected.as_ref().unwrap().plan_id, "plan");
        }
        view.handle_event(Event::Modified {
            applied: false,
            project_path: path.clone(),
            request_id: "test-2".into(),
            result: Ok(ModifyProjectResponse {
                reply: "Which destination?".into(),
                outcome: Some(ModifyOutcome::NeedsClarification {
                    questions: vec![QuestionSummary {
                        id: "q1".into(),
                        prompt: "Which destination?".into(),
                        options: vec!["New page".into()],
                        option_keys: vec!["new_page".into()],
                        takes_text: false,
                        blocking: true,
                    }],
                    persisted: false,
                }),
            }),
        });
        assert!(view.review.buttons.is_empty());
        assert!(view.messages.last().unwrap().text.contains("New page"));
    }

    #[test]
    fn question_only_preview_uses_continue_then_persisted_answer_buttons() {
        let client = ApiClient::new("http://127.0.0.1:1".into());
        let mut view = ChatView::new(client);
        view.selected = Some(project("example", None));
        let path = view.selected.as_ref().unwrap().path.clone();
        let epoch = view.preview.select(Some(&path)).unwrap();
        view.last_sent = Some(Sent::Command(
            "Share the selected doctor across pages".into(),
        ));
        let shape = QuestionSummary {
            id: "q3-context-shape".into(),
            prompt: "What kind of value is selected doctor?".into(),
            options: vec!["Text".into()],
            option_keys: vec!["text".into()],
            takes_text: false,
            blocking: true,
        };
        view.handle_event(Event::Modified {
            applied: false,
            project_path: path.clone(),
            request_id: "preview-shape".into(),
            result: Ok(ModifyProjectResponse {
                reply: "Preview: ask for shape".into(),
                outcome: Some(ModifyOutcome::Preview {
                    plan_id: "shape-plan".into(),
                    base_revision: 4,
                    changed_files: vec![],
                    diffs: vec![],
                    questions: vec![shape.clone()],
                }),
            }),
        });
        assert_eq!(view.review.buttons[0].label, "Continue");
        assert_eq!(view.review.expected.as_ref().unwrap().revision, 4);
        for state in [
            PreviewState::Running {
                url: "http://127.0.0.1:5173".into(),
            },
            PreviewState::Stopped,
        ] {
            view.preview.receive_status(
                epoch,
                false,
                Ok(PreviewStatusResponse {
                    project_path: Some(path.clone()),
                    generation: 1,
                    state,
                }),
            );
            assert_eq!(view.review.buttons[0].label, "Continue");
        }
        view.handle_event(Event::Modified {
            applied: true,
            project_path: path,
            request_id: "apply-shape".into(),
            result: Ok(ModifyProjectResponse {
                reply: shape.prompt.clone(),
                outcome: Some(ModifyOutcome::NeedsClarification {
                    questions: vec![shape],
                    persisted: true,
                }),
            }),
        });
        assert!(view.review.expected.is_none());
        assert_eq!(view.review.buttons[0].label, "Text");
        assert!(
            matches!(&view.review.buttons[0].choice, Choice::Answer { question_id, option_key }
            if question_id == "q3-context-shape" && option_key == "text")
        );
    }
}
