//! The suggestion popup under the editor's cursor, as in the Typst web app: it opens while typing
//! (after `#`, in a function's arguments, on `@`, in formulas, …) or with Ctrl+Space; the arrow
//! keys choose, Return or Tab insert, Escape closes. An inserted snippet's fields (the `title` of
//! `heading[title]`) are then visited with Tab and Shift+Tab. Finding the suggestions is
//! complete.rs's job.

use std::sync::Arc;

use eframe::egui::{
    self, Align2, Color32, FontId, Galley, Key, Modifiers, Pos2, Rect, Sense, Stroke, StrokeKind, Ui, UiBuilder, pos2, text::CCursor,
    text::CCursorRange, text::LayoutJob, vec2,
};

use crate::app::App;
use crate::complete::{Expansion, Item, Kind, Request, Response, byte_index, changed_span, expand, filter, map_field};
use crate::icons::{Icon, paint};
use crate::theme::{Theme, mix, pal};
use crate::widgets::{RADIUS, RADIUS_L, TEXT_SMALL};

/// Rows the popup shows at once (more scroll).
const ROWS: usize = 9;

#[derive(Default)]
pub struct Completion {
    popup: Option<Popup>,
    /// The fields of the last inserted snippet that Tab still visits.
    snippet: Option<Snippet>,
    /// The newest request; only its answer is shown, and only if the text is still the same.
    sent: Option<Sent>,
    next_id: u64,
    /// Counts the open document's changes.
    version: u64,
    /// Ctrl+Space or the menu asked for suggestions.
    pub explicit_pending: bool,
    /// A suggestion clicked in the popup; it is inserted in the next frame.
    clicked: Option<usize>,
    /// Ask again once a function's parentheses were inserted (for its parameters).
    follow_up: bool,
    /// The cursor after the last frame (as character indices), to notice it moving.
    last_cursor: Option<(usize, usize)>,
}

struct Sent {
    id: u64,
    rel: String,
    version: u64,
    explicit: bool,
}

struct Popup {
    rel: String,
    /// Character index where the suggestions apply; what was typed from here on filters them.
    from: usize,
    items: Vec<Item>,
    explicit: bool,
    /// The suggestions that fit what was typed (index into `items`, highlighted characters).
    shown: Vec<(usize, Vec<usize>)>,
    /// What `shown` was filtered with.
    typed: Option<String>,
    /// Index into `shown`.
    selected: usize,
    /// Scroll the selected row into view.
    reveal: bool,
    scroll: f32,
    /// Whether it was on screen (and so takes the keys), and where.
    visible: bool,
    rect: Option<Rect>,
}

struct Snippet {
    rel: String,
    /// Character ranges of each field, in the order Tab visits them.
    fields: Vec<Vec<(usize, usize)>>,
    active: usize,
    /// The text the ranges belong to.
    text: String,
}

impl Completion {
    /// Another project: nothing shown or asked for belongs to it.
    pub fn forget(&mut self) {
        self.popup = None;
        self.snippet = None;
        self.sent = None;
        self.clicked = None;
        self.follow_up = false;
    }
}

/// An answer from the completion thread.
pub fn receive(c: &mut Completion, resp: Response, open_rel: Option<&str>) {
    let Some(sent) = &c.sent else { return };
    // Typing has moved on since this was asked; a newer answer is on its way.
    if resp.id != sent.id || sent.version != c.version || Some(sent.rel.as_str()) != open_rel {
        return;
    }
    if resp.items.is_empty() {
        c.popup = None;
        return;
    }
    // A popup on screen stays there while its suggestions are renewed.
    let visible = c.popup.as_ref().is_some_and(|p| p.visible);
    c.popup = Some(Popup {
        rel: sent.rel.clone(),
        from: resp.from,
        items: resp.items,
        explicit: sent.explicit,
        shown: Vec::new(),
        typed: None,
        selected: 0,
        reveal: true,
        scroll: 0.0,
        visible,
        rect: None,
    });
}

/// Takes a key press with exactly these modifiers (Shift+↓ still extends the selection).
fn take_key(ui: &Ui, key: Key, mods: impl Fn(Modifiers) -> bool) -> bool {
    ui.input_mut(|i| {
        let before = i.events.len();
        i.events
            .retain(|e| !matches!(e, egui::Event::Key { key: k, pressed: true, modifiers, .. } if *k == key && mods(*modifiers)));
        i.events.len() != before
    })
}

fn plain(m: Modifiers) -> bool {
    m.is_none()
}

/// What [`keys`] changed.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Edit {
    Nothing,
    /// The cursor or selection (Tab to the next snippet field).
    Cursor,
    /// The text (a suggestion was inserted).
    Text,
}

/// Keys for the popup and snippet fields, before the editor sees them.
pub fn keys(
    c: &mut Completion,
    ui: &Ui,
    text: &mut String,
    rel: &str,
    state: &mut egui::text_edit::TextEditState,
    focused: bool,
    editable: bool,
) -> Edit {
    if focused && take_key(ui, Key::Space, |m| m.ctrl && !m.shift && !m.alt && !m.mac_cmd) {
        c.explicit_pending = true;
    }
    let cursor = state.cursor.char_range().map(|r| usize::from(r.primary.index)).unwrap_or(0);
    if let Some(i) = c.clicked.take()
        && editable
    {
        return accept(c, i, text, rel, cursor, state);
    }
    if !focused || !editable {
        return Edit::Nothing;
    }
    if let Some(p) = c.popup.as_mut().filter(|p| p.visible && p.rel == rel) {
        let n = p.shown.len();
        let mut moved = true;
        if take_key(ui, Key::ArrowDown, plain) {
            p.selected = (p.selected + 1) % n;
        } else if take_key(ui, Key::ArrowUp, plain) {
            p.selected = (p.selected + n - 1) % n;
        } else if take_key(ui, Key::PageDown, plain) {
            p.selected = (p.selected + ROWS - 1).min(n - 1);
        } else if take_key(ui, Key::PageUp, plain) {
            p.selected = p.selected.saturating_sub(ROWS - 1);
        } else {
            moved = false;
        }
        if moved {
            p.reveal = true;
            return Edit::Nothing;
        }
        if take_key(ui, Key::Escape, plain) {
            c.popup = None;
            return Edit::Nothing;
        }
        // Return on a word that is already complete is a line break, not a choice.
        let complete = p.shown.get(p.selected).is_some_and(|(i, _)| {
            let item = &p.items[*i];
            let typed = &text[byte_index(text, p.from)..byte_index(text, cursor.max(p.from))];
            item.label == typed && item.apply.as_ref().is_none_or(|a| *a == item.label)
        });
        if complete && ui.input(|i| i.key_pressed(Key::Enter)) {
            c.popup = None;
            return Edit::Nothing;
        }
        if take_key(ui, Key::Enter, plain) || take_key(ui, Key::Tab, plain) {
            let i = p.selected;
            return accept(c, i, text, rel, cursor, state);
        }
        return Edit::Nothing;
    }
    let Some(s) = c.snippet.as_mut().filter(|s| s.rel == rel) else {
        return Edit::Nothing;
    };
    let to = if take_key(ui, Key::Tab, |m| m.shift_only()) {
        s.active.checked_sub(1)
    } else if take_key(ui, Key::Tab, plain) {
        Some(s.active + 1)
    } else {
        if take_key(ui, Key::Escape, plain) {
            c.snippet = None;
        }
        return Edit::Nothing;
    };
    let Some(to) = to.filter(|to| *to < s.fields.len()) else {
        return Edit::Nothing;
    };
    s.active = to;
    let (a, b) = s.fields[to][0];
    state
        .cursor
        .set_char_range(Some(CCursorRange::two(CCursor::new(a), CCursor::new(b))));
    // Arriving at the last field ends the snippet; Tab is a Tab again.
    if to + 1 == s.fields.len() {
        c.snippet = None;
    }
    Edit::Cursor
}

/// Inserts the `n`-th shown suggestion in place of what was typed since the popup opened.
fn accept(c: &mut Completion, n: usize, text: &mut String, rel: &str, cursor: usize, state: &mut egui::text_edit::TextEditState) -> Edit {
    let Some(p) = c.popup.take().filter(|p| p.rel == rel) else {
        return Edit::Nothing;
    };
    let Some(item) = p.shown.get(n).map(|(i, _)| &p.items[*i]) else {
        return Edit::Nothing;
    };
    let (start, end) = (byte_index(text, p.from), byte_index(text, cursor.max(p.from)));
    let line = text[..start].rfind('\n').map(|i| i + 1).unwrap_or(0);
    let indent: String = text[line..].chars().take_while(|c| *c == ' ' || *c == '\t').collect();
    let exp = match &item.apply {
        Some(apply) => expand(apply, &indent),
        None => Expansion {
            text: item.label.clone(),
            fields: Vec::new(),
        },
    };
    text.replace_range(start..end, &exp.text);
    let at = |(a, b): (usize, usize)| (p.from + a, p.from + b);
    let fields: Vec<Vec<(usize, usize)>> = exp.fields.iter().map(|f| f.iter().copied().map(at).collect()).collect();
    let after = p.from + exp.text.chars().count();
    let (a, b) = fields.first().map(|f| f[0]).unwrap_or((after, after));
    state
        .cursor
        .set_char_range(Some(CCursorRange::two(CCursor::new(a), CCursor::new(b))));
    c.snippet = (fields.len() > 1).then(|| Snippet {
        rel: rel.to_string(),
        fields,
        active: 0,
        text: text.clone(),
    });
    // `text(|)` and `fill: |`: the parameters or values are what comes next.
    let before = text[..byte_index(text, a)].trim_end_matches(' ');
    c.follow_up = a == b && (before.ends_with('(') || before.ends_with(':'));
    Edit::Text
}

/// What the editor shows this frame.
pub struct View<'a> {
    pub text: &'a str,
    pub rel: &'a str,
    pub galley: &'a Arc<Galley>,
    pub galley_pos: Pos2,
    pub cursor: Option<CCursorRange>,
    /// The text changed this frame …
    pub changed: bool,
    /// … and this was typed.
    pub typed: &'a str,
    pub focused: bool,
    /// Suggestions make sense here (a Typst file one may edit).
    pub editable: bool,
    /// Suggest while typing (else only on Ctrl+Space).
    pub while_typing: bool,
    pub font_size: f32,
    pub theme: &'a Theme,
    /// The visible part of the editor.
    pub area: Rect,
}

/// After the editor ran: follows the snippet fields, decides whether to ask for suggestions
/// (returns `Some(explicit)` with the cursor) and draws the popup.
pub fn after_edit(c: &mut Completion, ui: &Ui, v: &View) -> Option<(bool, usize)> {
    let sel = v.cursor.map(|r| (usize::from(r.secondary.index), usize::from(r.primary.index)));
    let cursor = sel.map(|s| s.1).unwrap_or(0);
    if v.changed {
        c.version += 1;
    }
    let moved = sel != c.last_cursor;
    let before = c.last_cursor.map(|(a, b)| a.min(b)).unwrap_or(cursor);
    c.last_cursor = sel;

    // Snippet fields grow and move with the text, and end when the cursor leaves them.
    if let Some(s) = c.snippet.as_mut() {
        if s.rel == v.rel && v.changed && s.text != v.text {
            let low = sel.map(|(a, b)| a.min(b)).unwrap_or(cursor).min(before);
            let span = changed_span(&s.text, v.text, low);
            for field in &mut s.fields {
                for range in field.iter_mut() {
                    *range = map_field(*range, span);
                }
            }
            s.text = v.text.to_string();
        }
        let (a, b) = sel.map(|(a, b)| (a.min(b), a.max(b))).unwrap_or((cursor, cursor));
        let inside = s.fields.iter().flatten().any(|&(from, to)| from <= a && b <= to);
        if s.rel != v.rel || !inside {
            c.snippet = None;
        }
    }
    if let Some(s) = &c.snippet {
        paint_fields(ui, s, v);
    }

    let mut want = None;
    if std::mem::take(&mut c.explicit_pending) && v.editable {
        want = Some(true);
    }
    if let Some(p) = &c.popup {
        let over = p.rect.zip(ui.ctx().pointer_hover_pos()).is_some_and(|(r, at)| r.contains(at));
        let left = cursor < p.from || v.text[byte_index(v.text, p.from)..byte_index(v.text, cursor)].contains('\n');
        let gone = p.rel != v.rel
            || !v.editable
            || (!v.focused && !over && c.clicked.is_none())
            // Arrow keys or a click moved the cursor elsewhere.
            || (moved && !v.changed)
            || sel.is_none_or(|(a, b)| a != b)
            || left;
        if gone {
            c.popup = None;
        }
    }
    if v.changed && v.editable && want.is_none() {
        // While the popup is open, more letters only narrow it down (here, below); anything else
        // (`(`, `.`, `:`, a space) may change what fits, so Typst is asked again.
        let word = |ch: char| ch.is_alphanumeric() || ch == '_' || ch == '-';
        if c.popup.is_some() {
            if !v.typed.is_empty() && !v.typed.chars().all(word) && v.while_typing {
                want = Some(false);
            }
        } else if (!v.typed.is_empty() || c.follow_up) && v.while_typing {
            want = Some(false);
        }
    }
    c.follow_up = false;

    if let Some(p) = c.popup.as_mut() {
        let typed = &v.text[byte_index(v.text, p.from)..byte_index(v.text, cursor)];
        if p.typed.as_deref() != Some(typed) {
            p.shown = filter(&p.items, typed);
            p.typed = Some(typed.to_string());
            p.selected = 0;
            p.reveal = true;
        }
        // A new popup for a single letter after a space (often a variable in a formula) waits for
        // a second one.
        let before_from = v.text[..byte_index(v.text, p.from)].chars().next_back();
        let early = !p.explicit && !p.visible && typed.chars().count() == 1 && before_from.is_none_or(|c| c.is_whitespace() || c == '$');
        p.visible = false;
        if p.shown.is_empty() {
            c.popup = None;
        } else if !early && !draw(p, ui, v, &mut c.clicked) {
            // Scrolled out of sight.
            c.popup = None;
        }
    }
    want.map(|explicit| (explicit, cursor))
}

/// Check pictures (`TYPISCH_SHOT_VIEW`): the popup over the invoice template, after the `d` of
/// `datum: datetime(…)` ("complete") or on an empty line ("complete-markup").
pub fn demo(app: &mut App, view: &str) {
    app.jump_to = Some(if view == "complete-markup" { (37, 0) } else { (3, 11) });
    app.complete.explicit_pending = true;
}

/// Gives the editor the keyboard back. Only when it lacks it: asking while it has it would let
/// ↓ and Tab move the focus away in the next frame.
pub fn focus_editor(ctx: &egui::Context) {
    let id = crate::ide::editor_id();
    if !ctx.memory(|m| m.has_focus(id)) {
        ctx.memory_mut(|m| m.request_focus(id));
    }
}

/// Sends the editor's text to the completion thread.
pub fn request(app: &mut App, explicit: bool, cursor: usize) {
    let (Some(project), Some(doc)) = (app.project.as_ref(), app.doc.as_ref()) else {
        return;
    };
    let c = &mut app.complete;
    c.next_id += 1;
    c.sent = Some(Sent {
        id: c.next_id,
        rel: doc.rel.clone(),
        version: c.version,
        explicit,
    });
    let files = project.files.iter().filter(|f| !f.is_dir).map(|f| f.rel.clone()).collect();
    app.compiler.complete(Request {
        id: c.next_id,
        root: project.root.clone(),
        main_rel: project.main_rel.clone(),
        rel: doc.rel.clone(),
        text: doc.text.clone(),
        cursor,
        explicit,
        files,
    });
}

/// Where a character of the text is on screen.
fn char_rect(v: &View, index: usize) -> Rect {
    v.galley.pos_from_cursor(CCursor::new(index)).translate(v.galley_pos.to_vec2())
}

/// A thin frame around each field of the snippet, the one being filled in stronger.
fn paint_fields(ui: &Ui, s: &Snippet, v: &View) {
    let accent = pal().accent;
    for (i, field) in s.fields.iter().enumerate() {
        let strength = if i == s.active { 0.75 } else { 0.4 };
        let stroke = Stroke::new(1.0_f32, mix(v.theme.bg, accent, strength));
        for &(from, to) in field {
            let (a, b) = (char_rect(v, from), char_rect(v, to));
            // Fields over several lines only get their start marked.
            let right = if (a.center().y - b.center().y).abs() < 1.0 {
                b.left()
            } else {
                a.left()
            };
            let r = Rect::from_min_max(pos2(a.left() - 1.5, a.top()), pos2(right.max(a.left() + 1.0) + 1.5, a.bottom()));
            ui.painter().rect_stroke(r, 3.0, stroke, StrokeKind::Outside);
        }
    }
}

/// How a kind of suggestion is shown: its sign, colour and name.
fn kind_look(kind: &Kind, t: &Theme) -> (String, Color32, &'static str) {
    match kind {
        Kind::Syntax => ("{}".into(), t.markup, "Snippet"),
        Kind::Func => ("ƒ".into(), t.function, "Funktion"),
        Kind::Type => ("T".into(), t.keyword, "Typ"),
        Kind::Param => ("P".into(), t.label, "Parameter"),
        Kind::Constant => ("c".into(), t.number, "Wert"),
        Kind::Path => (String::new(), t.string, "Datei"),
        Kind::Package => (String::new(), t.function, "Paket"),
        Kind::Label => ("@".into(), t.label, "Label"),
        Kind::Font => ("Aa".into(), t.string, "Schrift"),
        Kind::Symbol(s) => (s.clone(), t.math, "Symbol"),
    }
}

/// Draws the popup; false when the place it belongs to is out of sight.
fn draw(p: &mut Popup, ui: &Ui, v: &View, clicked: &mut Option<usize>) -> bool {
    let anchor = char_rect(v, p.from);
    if !v.area.contains(anchor.left_center()) {
        return false;
    }
    let ctx = ui.ctx().clone();
    let t = v.theme;
    let font = FontId::monospace(v.font_size);
    let char_w = ui.fonts_mut(|f| f.glyph_width(&font, 'm'));
    let row_h = (v.font_size * 1.8).round().max(22.0);
    let pad = 4.0;
    let icon_w = row_h;
    let kind_w = 78.0;
    let longest = p
        .shown
        .iter()
        .take(300)
        .map(|(i, _)| p.items[*i].label.chars().count())
        .max()
        .unwrap_or(0) as f32;
    let width = (2.0 * pad + icon_w + 8.0 + longest * char_w + 16.0 + kind_w)
        .clamp(280.0, 560.0)
        .min(v.area.width() - 8.0);
    let rows = p.shown.len().min(ROWS);
    let list_h = rows as f32 * row_h + 2.0 * pad;

    // The selected suggestion's description underneath, with `code` set in the editor's font.
    let selected = &p.items[p.shown[p.selected.min(p.shown.len() - 1)].0];
    let has_details = p.shown.iter().take(300).any(|(i, _)| p.items[*i].detail.is_some());
    let detail_galley = selected.detail.as_ref().map(|d| {
        let mut job = LayoutJob::default();
        for (n, part) in d.split('`').enumerate() {
            let (font, color) = if n % 2 == 1 {
                (FontId::monospace(TEXT_SMALL - 0.5), t.function)
            } else {
                (FontId::proportional(TEXT_SMALL), pal().text_soft)
            };
            job.append(
                part,
                0.0,
                egui::TextFormat {
                    font_id: font,
                    color,
                    ..Default::default()
                },
            );
        }
        job.wrap.max_width = width - 24.0;
        job.wrap.max_rows = 3;
        job.wrap.overflow_character = Some('…');
        ui.fonts_mut(|f| f.layout_job(job))
    });
    let line_h = ui.fonts_mut(|f| f.row_height(&FontId::proportional(TEXT_SMALL)));
    let footer_h = if has_details {
        detail_galley.as_ref().map(|g| g.size().y).unwrap_or(0.0).max(line_h) + 16.0
    } else {
        0.0
    };
    let height = list_h + footer_h;

    // The labels line up with the text typed so far; below the line, or above when there is no room.
    let x = (anchor.left() - pad - icon_w - 8.0)
        .min(v.area.right() - width - 4.0)
        .max(v.area.left() + 4.0);
    let below = anchor.bottom() + 3.0;
    let y = if below + height <= v.area.bottom() - 4.0 || anchor.top() - v.area.top() < height + 7.0 {
        below
    } else {
        anchor.top() - 3.0 - height
    };

    let bg = mix(v.theme.bg, pal().text, if pal().dark { 0.07 } else { 0.035 });
    let area = egui::Area::new(egui::Id::unique("typisch-completions"))
        .order(egui::Order::Foreground)
        .fixed_pos(pos2(x, y))
        .constrain(false)
        .show(&ctx, |ui| {
            let (rect, _) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
            ui.painter()
                .rect(rect, RADIUS_L, bg, Stroke::new(1.0_f32, pal().separator), StrokeKind::Inside);
            let list = Rect::from_min_size(rect.min, vec2(width, list_h)).shrink(pad);
            ui.scope_builder(UiBuilder::new().max_rect(list), |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                let mut scroll = egui::ScrollArea::vertical()
                    .id_salt("typisch-completions-list")
                    .auto_shrink(false)
                    .max_height(list.height());
                if std::mem::take(&mut p.reveal) {
                    let top = p.selected as f32 * row_h;
                    let view_h = list.height();
                    if top < p.scroll {
                        p.scroll = top;
                    } else if top + row_h > p.scroll + view_h {
                        p.scroll = top + row_h - view_h;
                    }
                    scroll = scroll.vertical_scroll_offset(p.scroll);
                }
                let out = scroll.show_rows(ui, row_h, p.shown.len(), |ui, range| {
                    for n in range {
                        let (i, at) = &p.shown[n];
                        let item = &p.items[*i];
                        let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), row_h), Sense::click());
                        if n == p.selected {
                            ui.painter().rect_filled(r, RADIUS, mix(bg, pal().accent, 0.24));
                        } else if resp.hovered() {
                            ui.painter().rect_filled(r, RADIUS, mix(bg, pal().text, 0.07));
                        }
                        let (sign, color, name) = kind_look(&item.kind, t);
                        let icon = Rect::from_center_size(pos2(r.left() + icon_w / 2.0, r.center().y), vec2(row_h - 6.0, row_h - 6.0));
                        ui.painter().rect_filled(icon, 4.0, mix(bg, color, 0.16));
                        match item.kind {
                            Kind::Path => paint(ui.painter(), icon.shrink(3.0), Icon::Document, color),
                            Kind::Package => paint(ui.painter(), icon.shrink(3.0), Icon::Package, color),
                            Kind::Symbol(_) => {
                                ui.painter().text(
                                    icon.center(),
                                    Align2::CENTER_CENTER,
                                    sign,
                                    FontId::proportional(row_h * 0.62),
                                    color,
                                );
                            }
                            _ => {
                                ui.painter()
                                    .text(icon.center(), Align2::CENTER_CENTER, sign, FontId::proportional(row_h * 0.5), color);
                            }
                        }
                        // The label, with the typed characters picked out.
                        let mut job = LayoutJob::default();
                        for (k, ch) in item.label.chars().enumerate() {
                            let color = if at.contains(&k) { pal().accent } else { pal().text };
                            let mut buf = [0u8; 4];
                            job.append(
                                ch.encode_utf8(&mut buf),
                                0.0,
                                egui::TextFormat {
                                    font_id: font.clone(),
                                    color,
                                    ..Default::default()
                                },
                            );
                        }
                        job.wrap.max_width = r.width() - icon_w - 8.0 - kind_w - 8.0;
                        job.wrap.max_rows = 1;
                        job.wrap.overflow_character = Some('…');
                        let g = ui.fonts_mut(|f| f.layout_job(job));
                        ui.painter()
                            .galley(pos2(r.left() + icon_w + 8.0, r.center().y - g.size().y / 2.0), g, pal().text);
                        ui.painter().text(
                            pos2(r.right() - 8.0, r.center().y),
                            Align2::RIGHT_CENTER,
                            name,
                            FontId::proportional(TEXT_SMALL - 1.0),
                            pal().dim,
                        );
                        if resp.clicked() {
                            *clicked = Some(n);
                            focus_editor(ui.ctx());
                            ui.ctx().request_repaint();
                        }
                    }
                });
                p.scroll = out.state.clamped_offset().y;
            });
            if footer_h > 0.0 {
                let top = rect.top() + list_h;
                ui.painter()
                    .hline(rect.x_range().shrink(1.0), top, Stroke::new(1.0_f32, pal().separator));
                if let Some(g) = detail_galley {
                    ui.painter().galley(pos2(rect.left() + 12.0, top + 8.0), g, pal().text_soft);
                }
            }
        });
    p.rect = Some(area.response.rect);
    p.visible = true;
    true
}

/// The whole window driven headless (as in gitui_e2e.rs): typing into the editor, the real
/// completion thread answering.
#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::time::{Duration, Instant};

    use super::*;

    struct Harness {
        ctx: egui::Context,
        app: App,
        dir: PathBuf,
    }

    impl Harness {
        fn new(name: &str, source: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("typisch-complete-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("main.typ"), source).unwrap();
            let ctx = egui::Context::default();
            let mut app = App::for_tests(&ctx);
            app.open_path(&dir.join("main.typ"), &ctx);
            let mut h = Self { ctx, app, dir };
            h.frame();
            h
        }

        fn frame_with(&mut self, events: Vec<egui::Event>) {
            let input = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(1600.0, 968.0))),
                events,
                ..Default::default()
            };
            let app = &mut self.app;
            self.ctx.run_ui(input, |ui| app.frame(ui)).drop_without_applying_deltas();
        }

        fn frame(&mut self) {
            self.frame_with(Vec::new());
        }

        /// Keyboard input to the editor (it has the focus, as after a click into the text).
        fn keys(&mut self, events: Vec<egui::Event>) {
            // Asking again while it has the focus would let ↓ and Tab move the focus away.
            if !self.ctx.memory(|m| m.has_focus(crate::ide::editor_id())) {
                self.ctx.memory_mut(|m| m.request_focus(crate::ide::editor_id()));
            }
            self.frame_with(events);
            self.frame();
        }

        fn type_text(&mut self, text: &str) {
            for ch in text.chars() {
                self.keys(vec![egui::Event::Text(ch.to_string())]);
            }
        }

        fn press(&mut self, key: Key, modifiers: Modifiers) {
            self.keys(vec![egui::Event::Key {
                key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers,
            }]);
        }

        fn cursor_at(&mut self, line: usize, col: usize) {
            self.app.jump_to = Some((line, col));
            self.ctx.memory_mut(|m| m.request_focus(crate::ide::editor_id()));
            self.frame();
            self.frame();
        }

        fn until(&mut self, what: &str, done: impl Fn(&Self) -> bool) {
            let start = Instant::now();
            while !done(self) {
                assert!(
                    start.elapsed() < Duration::from_secs(30),
                    "timed out waiting for {what}; text {:?}, shown {:?}",
                    self.text(),
                    self.shown()
                );
                std::thread::sleep(Duration::from_millis(5));
                self.frame();
            }
        }

        /// Runs frames for a while (to see that something does *not* happen).
        fn idle(&mut self) {
            let start = Instant::now();
            while start.elapsed() < Duration::from_millis(400) {
                self.frame();
                std::thread::sleep(Duration::from_millis(5));
            }
        }

        fn text(&self) -> &str {
            &self.app.doc.as_ref().unwrap().text
        }

        /// The labels in the popup, best first; empty when it is not on screen.
        fn shown(&self) -> Vec<String> {
            match &self.app.complete.popup {
                Some(p) if p.visible => p.shown.iter().map(|(i, _)| p.items[*i].label.clone()).collect(),
                _ => Vec::new(),
            }
        }

        fn selection(&self) -> (usize, usize) {
            let r = egui::TextEdit::load_state(&self.ctx, crate::ide::editor_id())
                .unwrap()
                .cursor
                .char_range()
                .unwrap();
            let (a, b) = (usize::from(r.secondary.index), usize::from(r.primary.index));
            (a.min(b), a.max(b))
        }
    }

    impl Drop for Harness {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    #[test]
    fn hash_suggests_functions_then_their_parameters_and_values() {
        let mut h = Harness::new("hash", "Hallo\n");
        h.cursor_at(1, 0);
        h.type_text("#hea");
        h.until("heading to be suggested", |h| {
            h.shown().first().map(String::as_str) == Some("heading")
        });
        h.press(Key::Enter, Modifiers::NONE);
        assert_eq!(h.text(), "Hallo\n#heading()");
        assert_eq!(h.selection(), (15, 15));
        // Inside the parentheses its parameters follow by themselves.
        h.until("the parameters", |h| h.shown().iter().any(|l| l == "level"));
        h.press(Key::Escape, Modifiers::NONE);
        assert!(h.shown().is_empty());
        assert_eq!(h.text(), "Hallo\n#heading()");

        // `#text(` (the bracket closes itself) lists parameters, a parameter its values.
        h.cursor_at(1, 10);
        h.press(Key::Enter, Modifiers::NONE);
        h.type_text("#text(");
        assert_eq!(h.text(), "Hallo\n#heading()\n#text()");
        h.until("the parameters of text", |h| h.shown().iter().any(|l| l == "fill"));
        h.type_text("fil");
        assert_eq!(h.shown().first().map(String::as_str), Some("fill"));
        h.press(Key::Tab, Modifiers::NONE);
        assert_eq!(h.text(), "Hallo\n#heading()\n#text(fill: )");
        h.until("colours for fill", |h| h.shown().iter().any(|l| l == "red"));
    }

    #[test]
    fn at_suggests_the_documents_labels() {
        let mut h = Harness::new("labels", "= Einleitung <intro>\n\nSiehe \n");
        h.until("the first compile", |h| !h.app.pages.is_empty());
        h.cursor_at(2, 6);
        h.type_text("@");
        h.until("the label", |h| h.shown().iter().any(|l| l == "intro"));
        h.type_text("in");
        assert_eq!(h.shown().first().map(String::as_str), Some("intro"));
        h.press(Key::Enter, Modifiers::NONE);
        assert_eq!(h.text(), "= Einleitung <intro>\n\nSiehe @intro\n");
    }

    #[test]
    fn formulas_wait_for_a_second_letter() {
        let mut h = Harness::new("math", "$ $\n");
        h.cursor_at(0, 2);
        h.type_text("a");
        h.idle();
        assert!(h.shown().is_empty(), "shown after one letter: {:?}", h.shown());
        h.type_text("l");
        h.until("alpha", |h| h.shown().iter().any(|l| l == "alpha"));
        let at = h.shown().iter().position(|l| l == "alpha").unwrap();
        for _ in 0..at {
            h.press(Key::ArrowDown, Modifiers::NONE);
        }
        h.press(Key::Enter, Modifiers::NONE);
        assert_eq!(h.text(), "$ alpha$\n");
        // Moving the cursor away closes the popup; typing prose opens none.
        h.cursor_at(1, 0);
        h.type_text("Text ohne Code");
        h.idle();
        assert!(h.shown().is_empty());
    }

    #[test]
    fn ctrl_space_offers_snippets_whose_fields_tab_visits() {
        let mut h = Harness::new("snippet", "\n");
        h.cursor_at(0, 0);
        h.press(
            Key::Space,
            Modifiers {
                ctrl: true,
                ..Default::default()
            },
        );
        h.until("markup snippets", |h| h.shown().iter().any(|l| l == "heading"));
        h.type_text("term");
        assert_eq!(h.shown().first().map(String::as_str), Some("term list item"));
        h.press(Key::Enter, Modifiers::NONE);
        assert_eq!(h.text(), "/ term: description\n");
        assert_eq!(h.selection(), (2, 6));
        h.type_text("Wort");
        assert_eq!(h.text(), "/ Wort: description\n");
        h.press(Key::Tab, Modifiers::NONE);
        assert_eq!(h.selection(), (8, 19));
        h.type_text("Erklärung");
        assert_eq!(h.text(), "/ Wort: Erklärung\n");
        // The last field ends the snippet: Tab indents again.
        h.press(Key::Tab, Modifiers::NONE);
        assert_eq!(h.text(), "/ Wort: Erklärung  \n");
    }

    #[test]
    fn suggestions_can_be_switched_off() {
        let mut h = Harness::new("off", "\n");
        h.app.settings.autocomplete = false;
        h.cursor_at(0, 0);
        h.type_text("#hea");
        h.idle();
        assert!(h.shown().is_empty());
        // Ctrl+Space still asks.
        h.press(
            Key::Space,
            Modifiers {
                ctrl: true,
                ..Default::default()
            },
        );
        h.until("heading", |h| h.shown().first().map(String::as_str) == Some("heading"));
    }
}
