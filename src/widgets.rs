//! Shared controls and measures, so every bar, button and field in the app looks and behaves the same.
//!
//! Buttons come in two kinds:
//! - **flat** (toolbar buttons, close buttons, row actions): no background at rest, a light fill on
//!   hover, the `selected` fill while pressed or switched on;
//! - **filled** (dialog buttons, the welcome screen's big buttons, chips): a `surface` fill that
//!   lightens on hover; a dialog's main action is filled with the accent colour.
//!
//! Anything that is switched on or chosen gets the `selected` fill and full-strength text; at rest,
//! icons and labels on controls use `text_soft`.

use std::hash::Hash;

use eframe::egui::{self, Align2, Color32, FontId, Rect, Response, Sense, Stroke, StrokeKind, Ui, Widget, pos2, vec2};

use crate::icons::{Icon, paint};
use crate::theme::{mix, pal};

/// Height of the bars below the top bar: editor tabs, preview toolbar and status bar.
pub const BAR_H: f32 = 32.0;
/// Room between a bar's edge and its outermost button.
pub const BAR_PAD: f32 = 8.0;
/// Square toolbar buttons, the icon inside them and the gap between neighbours.
pub const TOOL: f32 = 28.0;
pub const TOOL_ICON: f32 = 16.0;
pub const TOOL_GAP: f32 = 2.0;
/// Height of text buttons, pop-up buttons, text fields, chips and segmented controls.
pub const CONTROL_H: f32 = 28.0;
/// Corner radius of controls and list rows …
pub const RADIUS: f32 = 6.0;
/// … of large buttons and panels inside a group …
pub const RADIUS_L: f32 = 8.0;
/// … and of groups, cards and dialogs.
pub const RADIUS_XL: f32 = 12.0;
/// Text on controls and in bars, and the smaller status text.
pub const TEXT: f32 = 13.5;
pub const TEXT_SMALL: f32 = 12.5;

/// Background of a flat control: none at rest, a light fill on hover, `selected` when on or pressed.
pub fn flat_fill(resp: &Response, on: bool) -> Option<Color32> {
    if on || resp.is_pointer_button_down_on() {
        Some(pal().selected)
    } else if resp.hovered() {
        Some(pal().surface_hover)
    } else {
        None
    }
}

/// Background of a filled control.
pub fn filled_fill(resp: &Response, on: bool) -> Color32 {
    if on || resp.is_pointer_button_down_on() {
        pal().selected
    } else if resp.hovered() {
        pal().surface_hover
    } else {
        pal().surface
    }
}

/// Colour of a control's icon or label: full strength when on.
pub fn fg(on: bool) -> Color32 {
    if on { pal().text } else { pal().text_soft }
}

/// Highlight of a list row: strong when it is the chosen one, light on hover.
pub fn row_bg(ui: &Ui, rect: Rect, resp: &Response, selected: bool) {
    if selected {
        ui.painter().rect_filled(rect, RADIUS, pal().row_selected);
    } else if resp.hovered() {
        ui.painter().rect_filled(rect, RADIUS, pal().hover);
    }
}

/// Background of a bar, with a hairline on the edge that borders the content.
pub fn bar(ui: &Ui, rect: Rect, line_at_top: bool) {
    ui.painter().rect_filled(rect, 0.0, pal().topbar);
    let y = if line_at_top { rect.top() + 0.5 } else { rect.bottom() - 0.5 };
    ui.painter().hline(rect.x_range(), y, Stroke::new(1.0_f32, pal().separator));
}

/// A short upright line between groups of toolbar buttons.
pub fn bar_separator(ui: &mut Ui) {
    let (r, _) = ui.allocate_exact_size(vec2(9.0, 16.0), Sense::hover());
    ui.painter().vline(r.center().x, r.y_range(), Stroke::new(1.0_f32, pal().separator));
}

enum Face<'a> {
    Icon(Icon),
    Glyph {
        text: &'a str,
        font: FontId,
        italics: bool,
        underline: bool,
    },
    Text {
        text: String,
        width: f32,
    },
}

/// A flat toolbar button showing an icon, a letter or a short text.
pub struct ToolButton<'a> {
    face: Face<'a>,
    tip: &'a str,
    on: bool,
    color: Option<Color32>,
}

impl<'a> ToolButton<'a> {
    pub fn new(icon: Icon, tip: &'a str) -> Self {
        Self {
            face: Face::Icon(icon),
            tip,
            on: false,
            color: None,
        }
    }

    /// A letter or sign instead of an icon (the formatting buttons).
    pub fn glyph(text: &'a str, font: FontId, tip: &'a str) -> Self {
        Self {
            face: Face::Glyph {
                text,
                font,
                italics: false,
                underline: false,
            },
            tip,
            on: false,
            color: None,
        }
    }

    /// A short text such as the zoom level, in a button of fixed width so the bar doesn't shift.
    pub fn text(text: impl Into<String>, width: f32, tip: &'a str) -> Self {
        Self {
            face: Face::Text { text: text.into(), width },
            tip,
            on: false,
            color: None,
        }
    }

    pub fn italic(mut self) -> Self {
        if let Face::Glyph { italics, .. } = &mut self.face {
            *italics = true;
        }
        self
    }

    pub fn underline(mut self) -> Self {
        if let Face::Glyph { underline, .. } = &mut self.face {
            *underline = true;
        }
        self
    }

    /// Stays highlighted while `on` (for buttons that switch something on and off).
    pub fn on(mut self, on: bool) -> Self {
        self.on = on;
        self
    }

    /// A colour that carries meaning, e.g. red while there are errors.
    pub fn color(mut self, color: Color32) -> Self {
        self.color = Some(color);
        self
    }
}

impl Widget for ToolButton<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        let width = match &self.face {
            Face::Text { width, .. } => *width,
            _ => TOOL,
        };
        let (rect, resp) = ui.allocate_exact_size(vec2(width, TOOL), Sense::click());
        let resp = if self.tip.is_empty() { resp } else { resp.on_hover_text(self.tip) };
        if !ui.is_rect_visible(rect) {
            return resp;
        }
        if let Some(fill) = flat_fill(&resp, self.on) {
            ui.painter().rect_filled(rect, RADIUS, fill);
        }
        let color = self.color.unwrap_or(fg(self.on));
        match self.face {
            Face::Icon(icon) => paint(
                ui.painter(),
                Rect::from_center_size(rect.center(), vec2(TOOL_ICON, TOOL_ICON)),
                icon,
                color,
            ),
            Face::Glyph {
                text,
                font,
                italics,
                underline,
            } => {
                let mut job = egui::text::LayoutJob::default();
                let format = egui::TextFormat {
                    font_id: font,
                    color,
                    italics,
                    underline: if underline { Stroke::new(1.2_f32, color) } else { Stroke::NONE },
                    ..Default::default()
                };
                job.append(text, 0.0, format);
                let g = ui.fonts_mut(|f| f.layout_job(job));
                ui.painter().galley(rect.center() - g.size() / 2.0, g, color);
            }
            Face::Text { text, .. } => {
                ui.painter()
                    .text(rect.center(), Align2::CENTER_CENTER, text, FontId::proportional(TEXT), color);
            }
        }
        resp
    }
}

/// A text button; `primary` for a dialog's main action.
pub struct Button<'a> {
    label: &'a str,
    primary: bool,
}

impl<'a> Button<'a> {
    pub fn new(label: &'a str) -> Self {
        Self { label, primary: false }
    }

    pub fn primary(label: &'a str) -> Self {
        Self { label, primary: true }
    }
}

impl Widget for Button<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        let color = if self.primary { pal().on_accent } else { pal().text };
        let g = ui.fonts_mut(|f| f.layout_no_wrap(self.label.to_string(), FontId::proportional(TEXT), color));
        let (rect, resp) = ui.allocate_exact_size(vec2((g.size().x + 28.0).max(80.0), CONTROL_H), Sense::click());
        if ui.is_rect_visible(rect) {
            let fill = if self.primary {
                let a = pal().accent;
                if resp.is_pointer_button_down_on() {
                    mix(a, Color32::BLACK, 0.15)
                } else if resp.hovered() {
                    mix(a, Color32::WHITE, 0.12)
                } else {
                    a
                }
            } else {
                filled_fill(&resp, false)
            };
            ui.painter().rect_filled(rect, RADIUS, fill);
            ui.painter().galley(rect.center() - g.size() / 2.0, g, color);
        }
        resp
    }
}

/// A capsule that is either on or off (the template categories).
pub fn chip(ui: &mut Ui, label: &str, on: bool) -> Response {
    let color = fg(on);
    let g = ui.fonts_mut(|f| f.layout_no_wrap(label.to_string(), FontId::proportional(TEXT), color));
    let (rect, resp) = ui.allocate_exact_size(vec2(g.size().x + 24.0, CONTROL_H), Sense::click());
    ui.painter().rect_filled(rect, CONTROL_H / 2.0, filled_fill(&resp, on));
    ui.painter().galley(rect.center() - g.size() / 2.0, g, color);
    resp
}

/// A capsule with one chosen segment (the sidebar tabs, the symbol input mode). Paints the track
/// and highlights and returns each segment's rect and response; the caller draws what is in them,
/// coloured with [`fg`].
pub fn segmented(
    ui: &mut Ui,
    rect: Rect,
    selected: usize,
    id_salt: impl Hash + Copy + std::fmt::Debug,
    tips: &[&str],
) -> Vec<(Rect, Response)> {
    ui.painter().rect_filled(rect, rect.height() / 2.0, pal().pill);
    let inner = rect.shrink(2.0);
    let w = inner.width() / tips.len().max(1) as f32;
    let segments: Vec<(Rect, Response)> = tips
        .iter()
        .enumerate()
        .map(|(i, tip)| {
            let r = Rect::from_min_size(pos2(inner.left() + i as f32 * w, inner.top()), vec2(w, inner.height()));
            let resp = ui.interact(r, ui.scope_id().with((id_salt, i)), Sense::click());
            (r, if tip.is_empty() { resp } else { resp.on_hover_text(*tip) })
        })
        .collect();
    let quiet = |i: usize| i != selected && !segments[i].1.hovered();
    for (i, (r, resp)) in segments.iter().enumerate() {
        if i == selected {
            ui.painter().rect_filled(*r, r.height() / 2.0, pal().pill_selected);
        } else if resp.hovered() {
            ui.painter().rect_filled(*r, r.height() / 2.0, pal().pill_hover);
        }
        if i > 0 && quiet(i) && quiet(i - 1) {
            ui.painter().vline(
                r.left(),
                (r.top() + 7.0)..=(r.bottom() - 7.0),
                Stroke::new(1.0_f32, pal().separator),
            );
        }
    }
    segments
}

/// A one-line text field, optionally with an icon in front. With the search icon it also gets
/// a button that clears it.
pub fn field(ui: &mut Ui, text: &mut String, hint: &str, width: f32, icon: Option<Icon>, id_salt: impl Hash + std::fmt::Debug) -> Response {
    text_field(ui, text, hint, width, icon, id_salt, false)
}

/// A one-line field whose text is hidden (API keys, passwords).
pub fn secret_field(ui: &mut Ui, text: &mut String, hint: &str, width: f32, id_salt: impl Hash + std::fmt::Debug) -> Response {
    text_field(ui, text, hint, width, None, id_salt, true)
}

fn text_field(
    ui: &mut Ui,
    text: &mut String,
    hint: &str,
    width: f32,
    icon: Option<Icon>,
    id_salt: impl Hash + std::fmt::Debug,
    password: bool,
) -> Response {
    let id = ui.make_persistent_id(id_salt);
    let (rect, bg) = ui.allocate_exact_size(vec2(width, CONTROL_H), Sense::click());
    ui.painter().rect_filled(rect, RADIUS, pal().surface);
    let search = icon == Some(Icon::Search);
    let left = if icon.is_some() { 28.0 } else { 10.0 };
    let right = if search { 26.0 } else { 10.0 };
    let inner = Rect::from_min_max(pos2(rect.left() + left, rect.top()), pos2(rect.right() - right, rect.bottom()));
    let resp = ui.put(
        inner,
        egui::TextEdit::singleline(text)
            .id(id)
            .hint_text(hint)
            .frame(egui::Frame::NONE)
            .margin(egui::Margin::ZERO)
            .min_size(inner.size())
            .align(Align2::LEFT_CENTER)
            .password(password)
            .desired_width(inner.width()),
    );
    // A click on the icon or the padding also puts the cursor in the field.
    if bg.clicked() {
        ui.memory_mut(|m| m.request_focus(id));
    }
    if let Some(icon) = icon {
        paint(
            ui.painter(),
            Rect::from_center_size(pos2(rect.left() + 15.0, rect.center().y), vec2(14.0, 14.0)),
            icon,
            pal().dim,
        );
    }
    if search && !text.is_empty() {
        let c = Rect::from_center_size(pos2(rect.right() - 14.0, rect.center().y), vec2(16.0, 16.0));
        let clear = ui.interact(c, id.with("clear"), Sense::click()).on_hover_text("Leeren");
        ui.painter()
            .circle_filled(c.center(), 8.0, if clear.hovered() { pal().dim } else { pal().faint });
        paint(ui.painter(), c.shrink(4.5), Icon::Close, pal().surface);
        if clear.clicked() {
            text.clear();
        }
    }
    if resp.has_focus() {
        ui.painter()
            .rect_stroke(rect, RADIUS, Stroke::new(1.5_f32, pal().accent), StrokeKind::Inside);
    }
    resp
}
