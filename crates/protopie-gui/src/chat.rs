//! Single-column agent chat view: project dropdown header, scrolling message list, input bar,
//! and a modal for naming a new project.

use akar_components::{
    akar_button, akar_text_input, modal_begin, modal_end, scroll_area_begin, scroll_area_end,
    AkarTheme, ButtonVariant, TextEditState, AKAR_THEME_DARK,
};
use akar_core::{AkarCore, Key, QuadCall, TextCall, Z_FLOAT, Z_OVERLAY};
use akar_layout::{length, Dimension, Display, FlexDirection, Layout, NodeId, Size, Style};
use protopie_api::ProjectInfo;

use crate::client::{ApiClient, Event};

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

impl ChatView {
    pub fn new(client: ApiClient) -> Self {
        client.list_projects();
        let mut layout = Layout::new();
        let project_btn = layout.new_leaf(Style {
            flex_grow: 1.0,
            size: Size { width: Dimension::auto(), height: length(BTN_H) },
            ..Default::default()
        });
        let header = layout.new_with_children(
            Style {
                display: Display::Flex,
                flex_direction: FlexDirection::Row,
                align_items: Some(akar_layout::AlignItems::CENTER),
                size: Size { width: Dimension::percent(1.0), height: length(HEADER_H) },
                ..Default::default()
            },
            &[project_btn],
        );
        layout.set_padding(header, 0.0, 10.0, 0.0, 10.0);
        let messages_area = layout.new_leaf(Style {
            flex_grow: 1.0,
            size: Size { width: Dimension::percent(1.0), height: Dimension::auto() },
            ..Default::default()
        });
        let input = layout.new_leaf(Style {
            flex_grow: 1.0,
            size: Size { width: Dimension::auto(), height: length(36.0_f32) },
            ..Default::default()
        });
        layout.set_margin(input, 0.0, 8.0, 0.0, 0.0);
        let send = layout.new_leaf(Style {
            size: Size { width: length(64.0_f32), height: length(36.0_f32) },
            ..Default::default()
        });
        let input_bar = layout.new_with_children(
            Style {
                display: Display::Flex,
                flex_direction: FlexDirection::Row,
                align_items: Some(akar_layout::AlignItems::CENTER),
                size: Size { width: Dimension::percent(1.0), height: length(INPUT_BAR_H) },
                ..Default::default()
            },
            &[input, send],
        );
        layout.set_padding(input_bar, 0.0, 10.0, 0.0, 10.0);
        let root = layout.new_with_children(
            Style {
                display: Display::Flex,
                flex_direction: FlexDirection::Column,
                size: Size { width: Dimension::percent(1.0), height: Dimension::percent(1.0) },
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
            occlude_top: 0.0,
            new_project: None,
            messages_area,
            input,
            send,
            messages: vec![Message {
                role: Role::Agent,
                text: "Hi! Send me a prompt and I'll echo it back.".into(),
                buffer: None,
            }],
            input_text: String::new(),
            edit: TextEditState::default(),
            scroll_y: 0.0,
            pending: 0,
            stick_to_bottom: true,
        }
    }

    fn push(&mut self, role: Role, text: String) {
        self.messages.push(Message { role, text, buffer: None });
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
        self.pending += 1;
        match &self.selected {
            Some(p) => self.client.modify(p.path.clone(), prompt),
            None => self.client.chat(prompt),
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
            Event::Projects(Ok(list)) => {
                self.projects = list;
                if let Some(slug) = self.select_after_refresh.take() {
                    self.selected = self.projects.iter().find(|p| p.name == slug).cloned();
                } else if let Some(sel) = &self.selected {
                    // Drop a selection that no longer exists.
                    self.selected = self.projects.iter().find(|p| p.path == sel.path).cloned();
                }
            }
            Event::Projects(Err(e)) => {
                self.select_after_refresh = None;
                self.push(Role::Error, format!("Could not load projects: {e}"));
            }
            Event::ProjectCreated(Ok(created)) => {
                self.push(Role::Agent, format!("Created project \"{}\".", created.slug));
                self.select_after_refresh = Some(created.slug);
                self.client.list_projects();
            }
            Event::ProjectCreated(Err(e)) => {
                self.push(Role::Error, format!("Could not create project: {e}"));
            }
        }
    }

    /// Draws one frame. `size` is the logical (scale-independent) window size.
    pub fn render(&mut self, core: &mut AkarCore, size: [f32; 2], cursor_visible: bool) {
        while let Some(event) = self.client.try_recv() {
            self.handle_event(event);
        }

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
        self.draw_messages(core);

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
        let send = akar_button(core, &self.layout, self.send, "Send", ButtonVariant::Solid, &THEME);
        if input.submitted || send.clicked {
            self.submit();
        }
    }

    fn project_label(&self) -> &str {
        self.selected.as_ref().map_or("Select project", |p| p.name.as_str())
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
            ..quad(btn, if hovered { THEME.base_300 } else { THEME.base_200 }, THEME.radius_field)
        });
        let label = self.project_label().to_string();
        let text_clip = [btn[0], btn[1], btn[2] - 28.0, btn[3]];
        let ty = btn[1] + (btn[3] - THEME.font_size_base * 1.2) / 2.0;
        Self::draw_text(
            core, BUF_LABEL, &label, THEME.font_size_base,
            [btn[0] + THEME.padding_x, ty], text_clip, THEME.base_content, 0.0,
        );
        Self::draw_text(
            core, BUF_CHEVRON, "\u{25BC}", THEME.font_size_sm,
            [btn[0] + btn[2] - 20.0, btn[1] + (btn[3] - THEME.font_size_sm * 1.2) / 2.0],
            btn, THEME.base_content, 0.0,
        );

        // Popup rows: "New Project", "No project", then the projects.
        let total_rows = FIXED_ROWS + self.projects.len();
        let visible = total_rows.min(MAX_ROWS);
        let popup = [btn[0], btn[1] + btn[3] + 4.0, btn[2], visible as f32 * ROW_H];
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
        let inner = [popup[0] + 1.0, popup[1] + 1.0, popup[2] - 2.0, popup[3] - 2.0];
        let mut chosen = None;
        for i in 0..total_rows {
            let row = [inner[0], inner[1] + i as f32 * ROW_H - self.popup_scroll, inner[2], ROW_H];
            if row[1] + row[3] < inner[1] || row[1] > inner[1] + inner[3] {
                continue;
            }
            let visible_row = clip_to(row, inner);
            let (text, active) = match i {
                NEW_ROW => ("+ New Project".to_string(), false),
                NONE_ROW => ("No project".to_string(), self.selected.is_none()),
                _ => {
                    let p = &self.projects[i - FIXED_ROWS];
                    (p.name.clone(), self.selected.as_ref().is_some_and(|s| s.path == p.path))
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
            let color = if active { THEME.primary } else { THEME.base_content };
            Self::draw_text(
                core,
                BUF_ROW + i as u64,
                &text,
                THEME.font_size_base,
                [row[0] + THEME.padding_x, row[1] + (ROW_H - THEME.font_size_base * 1.2) / 2.0],
                visible_row,
                color,
                Z_OVERLAY,
            );
        }
        if let Some(i) = chosen {
            self.popup_open = false;
            match i {
                NEW_ROW => {
                    self.new_project =
                        Some(NewProjectForm { name: String::new(), edit: TextEditState::default() });
                    core.input.focused_id = None;
                }
                NONE_ROW => self.selected = None,
                _ => self.selected = Some(self.projects[i - FIXED_ROWS].clone()),
            }
        }
    }

    /// Modal asking for the new project's name. Enter or Create submits; Esc, Cancel, the
    /// close button or a click outside the panel cancels.
    fn draw_new_project_modal(&mut self, core: &mut AkarCore, size: [f32; 2], cursor_visible: bool) {
        let viewport = [0.0, 0.0, size[0], size[1]];
        // `modal_begin` lays out into a layout it is given; its nodes are rebuilt every frame.
        let mut modal_layout = Layout::new();
        modal_layout.set_namespace_id(MODAL_NS);
        let m = modal_begin(core, &mut modal_layout, viewport, "New project", 300.0, 150.0, &THEME);
        let mut cancel = m.close_requested && {
            // akar reports any click in the viewport; only honour clicks outside the panel
            // or on the close button.
            let close_btn = [m.panel_rect[0] + m.panel_rect[2] - 40.0, m.panel_rect[1], 40.0, 40.0];
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
                size: Size { width: w, height: length(h) },
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
                size: Size { width: length(fw), height: length(32.0_f32) },
                ..Default::default()
            },
            &[spacer, cancel_node, create_node],
        );
        let root = fl.new_with_children(
            Style {
                display: Display::Flex,
                flex_direction: FlexDirection::Column,
                size: Size { width: Dimension::percent(1.0), height: Dimension::percent(1.0) },
                ..Default::default()
            },
            &[field, row],
        );
        fl.set_padding(root, m.content_rect[1] + 8.0, 0.0, 0.0, m.content_rect[0] + 16.0);
        fl.compute(root, (Some(size[0]), Some(size[1])), |_, _, _, _, _| Size::ZERO);

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
        for (node, label, is_create) in [(cancel_node, "Cancel", false), (create_node, "Create", true)] {
            let r = fl.rect(node);
            let hovered = core.input.is_hovering(r);
            let fill = match (is_create, hovered) {
                (true, _) => THEME.primary,
                (false, false) => THEME.base_300,
                (false, true) => THEME.base_100,
            };
            core.draw_list.push_quad(QuadCall { z: Z_FLOAT + 0.01, ..quad(r, fill, THEME.radius_field) });
            let color = if is_create { THEME.primary_content } else { THEME.base_content };
            let metrics = glyphon::Metrics::new(THEME.font_size_base, THEME.font_size_base * 1.2);
            let buf = core.text_pipeline.set_text(Some(fl.widget_id(node)), label, metrics, None, None, None);
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
            self.client.create_project(PROJECTS_BASE_PATH.to_string(), name);
        }
    }

    fn draw_messages(&mut self, core: &mut AkarCore) {
        let area = self.layout.rect(self.messages_area);
        let max_bubble_w = area[2] * 0.82;
        let max_text_w = max_bubble_w - 2.0 * BUBBLE_PAD_X;
        let metrics = glyphon::Metrics::new(THEME.font_size_base, THEME.font_size_base * 1.4);

        // Measure pass: size every bubble so the scroll extent is known.
        let mut sizes = Vec::with_capacity(self.messages.len() + 1);
        let typing = self.pending > 0;
        let mut typing_buf = None;
        for m in &mut self.messages {
            let id = core
                .text_pipeline
                .set_text(m.buffer, &m.text, metrics, Some(max_text_w), None, None);
            m.buffer = Some(id);
            let s = core.text_pipeline.measure(id, Some(max_text_w));
            sizes.push([s.x.ceil() + 2.0 * BUBBLE_PAD_X, s.y.ceil() + 2.0 * BUBBLE_PAD_Y]);
        }
        if typing {
            let id = core
                .text_pipeline
                .set_text(Some(2), "…", metrics, Some(max_text_w), None, None);
            typing_buf = Some(id);
            let s = core.text_pipeline.measure(id, Some(max_text_w));
            sizes.push([s.x.ceil() + 2.0 * BUBBLE_PAD_X, s.y.ceil() + 2.0 * BUBBLE_PAD_Y]);
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
