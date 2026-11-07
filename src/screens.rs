//! Welcome screen, template picker and the settings window.

use eframe::egui::{
    self, Align, Align2, Color32, FontId, Layout, Mesh, Pos2, Rect, RichText, Sense, Stroke, StrokeKind, Ui, UiBuilder,
    pos2, vec2,
};

use crate::app::{App, Screen, SettingsTab, bold, tilde};
use crate::compiler::Job;
use crate::icons::{Icon, icon_button, paint};
use crate::templates::{CATEGORIES, TEMPLATES};
use crate::theme::{AutoClose, THEMES, pal};


/// Our own logo: a teal-to-blue rounded tile with a page and a serif "T".
pub fn paint_logo(ui: &Ui, rect: Rect) {
    let p = ui.painter();
    let r = rect.width() * 0.23;
    // Vertical gradient via a mesh, clipped to a rounded shape by drawing corners on top.
    let top = Color32::from_rgb(0x2f, 0xc4, 0xa8);
    let bot = Color32::from_rgb(0x1f, 0x6f, 0xc8);
    let shape_pts = rounded_rect_points(rect, r, 10);
    let mut mesh = Mesh::default();
    let center_col = lerp(top, bot, 0.5);
    mesh.colored_vertex(rect.center(), center_col);
    for pt in &shape_pts {
        let t = (pt.y - rect.top()) / rect.height();
        mesh.colored_vertex(*pt, lerp(top, bot, t));
    }
    let n = shape_pts.len() as u32;
    for i in 0..n {
        mesh.add_triangle(0, 1 + i, 1 + (i + 1) % n);
    }
    p.add(egui::Shape::mesh(mesh));
    // Grid lines
    let grid = Color32::from_white_alpha(38);
    for k in 1..4 {
        let x = rect.left() + rect.width() * k as f32 / 4.0;
        let y = rect.top() + rect.height() * k as f32 / 4.0;
        p.line_segment([pos2(x, rect.top() + 4.0), pos2(x, rect.bottom() - 4.0)], Stroke::new(1.0_f32, grid));
        p.line_segment([pos2(rect.left() + 4.0, y), pos2(rect.right() - 4.0, y)], Stroke::new(1.0_f32, grid));
    }
    // Page
    let page = Rect::from_center_size(rect.center() + vec2(0.0, rect.height() * 0.02), vec2(rect.width() * 0.5, rect.height() * 0.62));
    p.rect_filled(page.translate(vec2(2.0, 3.0)), 4.0, Color32::from_black_alpha(50));
    p.rect_filled(page, 4.0, Color32::from_rgb(0xf7, 0xf9, 0xfa));
    p.text(
        page.center() - vec2(0.0, page.height() * 0.08),
        Align2::CENTER_CENTER,
        "T",
        FontId::new(rect.height() * 0.38, egui::FontFamily::Name("dejavu".into())),
        Color32::from_rgb(0x1d, 0x4f, 0x7a),
    );
    for i in 0..2 {
        let y = page.bottom() - page.height() * (0.24 - i as f32 * 0.1);
        p.line_segment(
            [pos2(page.left() + page.width() * 0.2, y), pos2(page.right() - page.width() * (0.2 + i as f32 * 0.15), y)],
            Stroke::new(rect.width() * 0.025, Color32::from_rgb(0x9a, 0xb4, 0xc8)),
        );
    }
}

fn lerp(a: Color32, b: Color32, t: f32) -> Color32 {
    let f = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgb(f(a.r(), b.r()), f(a.g(), b.g()), f(a.b(), b.b()))
}

fn rounded_rect_points(rect: Rect, r: f32, steps: usize) -> Vec<Pos2> {
    use std::f32::consts::PI;
    let corners = [
        (pos2(rect.right() - r, rect.top() + r), -PI / 2.0),
        (pos2(rect.right() - r, rect.bottom() - r), 0.0),
        (pos2(rect.left() + r, rect.bottom() - r), PI / 2.0),
        (pos2(rect.left() + r, rect.top() + r), PI),
    ];
    let mut pts = Vec::new();
    for (c, a0) in corners {
        for i in 0..=steps {
            let a = a0 + (PI / 2.0) * i as f32 / steps as f32;
            pts.push(c + vec2(a.cos(), a.sin()) * r);
        }
    }
    pts
}

fn big_button(ui: &mut Ui, icon: Icon, label: &str, width: f32) -> bool {
    let (rect, resp) = ui.allocate_exact_size(vec2(width, 50.0), Sense::click());
    let bg = if resp.hovered() { pal().surface_hover } else { pal().surface };
    ui.painter().rect_filled(rect, 10.0, bg);
    paint(ui.painter(), Rect::from_center_size(pos2(rect.left() + 30.0, rect.center().y), vec2(22.0, 22.0)), icon, pal().dim);
    ui.painter().text(pos2(rect.left() + 56.0, rect.center().y), Align2::LEFT_CENTER, label, FontId::proportional(16.0), pal().text);
    resp.clicked()
}

pub fn welcome(app: &mut App, ctx: &egui::Context) {
    egui::SidePanel::right("recent")
        .exact_width((ctx.content_rect().width() * 0.37).max(240.0))
        .resizable(false)
        .frame(egui::Frame::new().fill(pal().sidebar).inner_margin(egui::Margin { left: 22, right: 16, top: 26, bottom: 16 }))
        .show_separator_line(false)
        .show(ctx, |ui| {
            ui.label(RichText::new("Letzte Projekte").font(bold(16.5)).color(pal().text));
            ui.add_space(12.0);
            let recent = app.settings.recent.clone();
            let mut open = None;
            let mut remove = None;
            egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
                if recent.is_empty() {
                    ui.label(RichText::new("Noch keine Projekte").color(pal().dim));
                }
                for (i, path) in recent.iter().enumerate() {
                    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 48.0), Sense::click());
                    if resp.hovered() {
                        ui.painter().rect_filled(rect, 8.0, pal().hover);
                    }
                    paint(ui.painter(), Rect::from_center_size(pos2(rect.left() + 14.0, rect.center().y), vec2(20.0, 20.0)), Icon::Document, pal().dim);
                    let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                    ui.painter().text(pos2(rect.left() + 36.0, rect.top() + 15.0), Align2::LEFT_CENTER, name, FontId::proportional(16.5), pal().text);
                    ui.painter().text(pos2(rect.left() + 36.0, rect.top() + 35.0), Align2::LEFT_CENTER, tilde(path), FontId::proportional(12.5), pal().dim);
                    if resp.clicked() {
                        open = Some(path.clone());
                    }
                    resp.context_menu(|ui| {
                        if ui.button("Aus der Liste entfernen").clicked() {
                            remove = Some(i);
                            ui.close();
                        }
                    });
                }
            });
            if let Some(i) = remove {
                app.settings.recent.remove(i);
                app.settings.save();
            }
            if let Some(p) = open {
                if p.exists() {
                    app.open_path(&p, ctx);
                } else {
                    app.toast("Dieser Ordner existiert nicht mehr.", false);
                }
            }
        });
    egui::CentralPanel::default().frame(egui::Frame::new().fill(pal().editor_bg)).show(ctx, |ui| {
        let full = ui.max_rect();
        crate::ide::window_drag_area(ui, Rect::from_min_size(full.min, vec2(full.width(), 52.0)), "welcome-drag");
        let h = full.height();
        let scale = (h / 500.0).clamp(0.75, 1.3);
        ui.vertical_centered(|ui| {
            ui.add_space(30.0 * scale);
            let (logo, _) = ui.allocate_exact_size(vec2(96.0, 96.0) * scale, Sense::hover());
            paint_logo(ui, logo);
            ui.add_space(8.0);
            ui.label(RichText::new("Typisch").font(bold(38.0 * scale)).color(pal().text));
            ui.label(RichText::new(crate::app::version_line()).size(14.0).color(pal().text_soft));
            ui.label(RichText::new("Typst-Editor mit Live-Vorschau").size(12.5).color(pal().dim));
            ui.add_space(22.0 * scale);
            let w = (full.width() * 0.62).min(330.0);
            if big_button(ui, Icon::Folder, "Ordner öffnen …", w) {
                if let Some(dir) = rfd::FileDialog::new().set_title("Ordner öffnen").pick_folder() {
                    app.open_path(&dir, ctx);
                }
            }
            ui.add_space(4.0);
            if big_button(ui, Icon::DocumentPlus, "Neues Projekt aus Template …", w) {
                app.screen = Screen::Templates;
                crate::app::resize_window(ctx, vec2(1280.0, 840.0));
            }
            ui.add_space(4.0);
            if big_button(ui, Icon::Document, "Datei öffnen …", w) {
                if let Some(file) = rfd::FileDialog::new().set_title("Datei öffnen").add_filter("Typst", &["typ"]).pick_file() {
                    app.open_path(&file, ctx);
                }
            }
            ui.add_space(10.0);
            ui.label(RichText::new("Beim Öffnen einer Datei wird ihr Ordner als Projekt geöffnet.").size(12.5).color(pal().faint));
        });
    });
}

pub fn templates(app: &mut App, ctx: &egui::Context) {
    if !app.thumbs_requested {
        app.thumbs_requested = true;
        for t in TEMPLATES {
            let src = t.source.replace("#image(\"beispiel-unterschrift.svg\", width: 4.5cm)", "");
            app.compiler.send(Job::Thumbnail { key: t.name.to_string(), source: src });
        }
    }
    egui::TopBottomPanel::top("tpl-head")
        .frame(egui::Frame::new().fill(pal().topbar).inner_margin(egui::Margin { left: 18, right: 18, top: 12, bottom: 10 }))
        .show(ctx, |ui| {
            crate::ide::window_drag_area(ui, ui.max_rect().expand2(vec2(18.0, 12.0)), "tpl-drag");
            ui.horizontal(|ui| {
                ui.add_space(crate::ide::MAC_BUTTONS_W);
                let (r, resp) = ui.allocate_exact_size(vec2(30.0, 30.0), Sense::click());
                ui.painter().circle_filled(r.center(), 15.0, if resp.hovered() { pal().surface_hover } else { pal().surface });
                paint(ui.painter(), r.shrink(10.0), Icon::Close, pal().text_soft);
                if resp.clicked() {
                    back_from_templates(app, ctx);
                }
                ui.add_space(8.0);
                ui.label(RichText::new("Template auswählen").font(bold(22.0)).color(pal().text));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let frame = egui::Frame::new().fill(pal().surface).corner_radius(8.0).inner_margin(egui::Margin { left: 32, right: 10, top: 7, bottom: 7 });
                    let r = frame.show(ui, |ui| {
                        ui.add(egui::TextEdit::singleline(&mut app.template_query).hint_text("Templates suchen").frame(false).desired_width(220.0));
                    });
                    let fr = r.response.rect;
                    paint(ui.painter(), Rect::from_center_size(pos2(fr.left() + 17.0, fr.center().y), vec2(15.0, 15.0)), Icon::Search, pal().dim);
                });
            });
            ui.add_space(10.0);
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
                let chip = |ui: &mut Ui, label: &str, on: bool| -> bool {
                    let g = ui.fonts_mut(|f| f.layout_no_wrap(label.to_string(), FontId::proportional(14.5), pal().text));
                    let (r, resp) = ui.allocate_exact_size(vec2(g.size().x + 26.0, 30.0), Sense::click());
                    let bg = if on { pal().selected } else if resp.hovered() { pal().surface_hover } else { pal().surface };
                    ui.painter().rect_filled(r, 15.0, bg);
                    ui.painter().galley(r.center() - g.size() / 2.0, g, if on { pal().dim } else { pal().text });
                    resp.clicked()
                };
                if chip(ui, "Alle", app.template_category.is_none()) {
                    app.template_category = None;
                }
                for c in CATEGORIES {
                    if chip(ui, c, app.template_category == Some(c)) {
                        app.template_category = Some(c);
                    }
                }
            });
        });
    let mut create = false;
    egui::TopBottomPanel::bottom("tpl-foot")
        .frame(egui::Frame::new().fill(pal().topbar).inner_margin(egui::Margin { left: 18, right: 18, top: 10, bottom: 12 }))
        .show(ctx, |ui| {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let create_btn = egui::Button::new(RichText::new("Erstellen …").size(16.0).color(pal().on_accent))
                    .fill(pal().accent)
                    .min_size(vec2(130.0, 36.0))
                    .corner_radius(8.0);
                if ui.add(create_btn).clicked() {
                    create = true;
                }
                let cancel = egui::Button::new(RichText::new("Abbrechen").size(16.0)).min_size(vec2(120.0, 36.0)).corner_radius(8.0);
                if ui.add(cancel).clicked() {
                    back_from_templates(app, ctx);
                }
            });
        });
    let q = app.template_query.to_lowercase();
    let visible: Vec<usize> = TEMPLATES
        .iter()
        .enumerate()
        .filter(|(_, t)| app.template_category.map(|c| t.category == c).unwrap_or(true))
        .filter(|(_, t)| q.is_empty() || t.name.to_lowercase().contains(&q) || t.description.to_lowercase().contains(&q))
        .map(|(i, _)| i)
        .collect();
    egui::CentralPanel::default().frame(egui::Frame::new().fill(pal().topbar).inner_margin(egui::Margin::symmetric(18, 14))).show(ctx, |ui| {
        let r = ui.max_rect();
        ui.painter().line_segment([pos2(r.left() - 18.0, r.top() - 14.0), pos2(r.right() + 18.0, r.top() - 14.0)], Stroke::new(1.0_f32, pal().separator));
        egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            let cols = ((ui.available_width() + 16.0) / 280.0).floor().max(1.0) as usize;
            let card_w = (ui.available_width() - 16.0 * (cols as f32 - 1.0)) / cols as f32;
            let thumb_h = card_w * 0.95;
            if visible.is_empty() {
                ui.add_space(40.0);
                ui.vertical_centered(|ui| ui.label(RichText::new("Keine Templates gefunden").color(pal().dim)));
            }
            for chunk in visible.chunks(cols) {
                ui.horizontal_top(|ui| {
                    ui.spacing_mut().item_spacing.x = 16.0;
                    for &i in chunk {
                        let t = &TEMPLATES[i];
                        let (rect, resp) = ui.allocate_exact_size(vec2(card_w, thumb_h + 82.0), Sense::click());
                        let tr = Rect::from_min_size(rect.min, vec2(card_w, thumb_h));
                        let sel = app.template_selected == i;
                        ui.painter().rect_filled(tr, 12.0, pal().group);
                        ui.painter().rect_stroke(
                            tr,
                            12.0,
                            Stroke::new(if sel { 3.0_f32 } else { 1.0 }, if sel { pal().accent } else { pal().separator }),
                            StrokeKind::Inside,
                        );
                        let inner = tr.shrink(24.0);
                        if let Some(tex) = app.thumbs.get(t.name) {
                            let size = tex.size_vec2();
                            let s = (inner.width() / size.x).min(inner.height() / size.y);
                            let ir = Rect::from_center_size(inner.center(), size * s);
                            ui.painter().image(tex.id(), ir, Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0)), Color32::WHITE);
                        } else {
                            let ir = Rect::from_center_size(inner.center(), vec2(inner.height() * 0.707, inner.height()).min(inner.size()));
                            ui.painter().rect_filled(ir, 2.0, Color32::WHITE);
                        }
                        ui.painter().text(pos2(rect.left() + 4.0, tr.bottom() + 22.0), Align2::LEFT_CENTER, t.name, bold(19.0), pal().text);
                        let g = ui.fonts_mut(|f| {
                            let mut job = egui::text::LayoutJob::simple(t.description.to_string(), FontId::proportional(14.5), pal().dim, card_w - 8.0);
                            job.wrap.max_rows = 2;
                            job.wrap.overflow_character = Some('…');
                            f.layout_job(job)
                        });
                        ui.painter().galley(pos2(rect.left() + 4.0, tr.bottom() + 36.0), g, pal().dim);
                        if resp.clicked() {
                            app.template_selected = i;
                        }
                        if resp.double_clicked() {
                            app.template_selected = i;
                            create = true;
                        }
                    }
                });
                ui.add_space(18.0);
            }
        });
    });
    if create {
        app.create_from_template(app.template_selected, ctx);
    }
}

fn back_from_templates(app: &mut App, ctx: &egui::Context) {
    if app.project.is_some() {
        app.screen = Screen::Ide;
        crate::app::resize_window(ctx, app.ide_size());
    } else {
        app.screen = Screen::Welcome;
        crate::app::resize_window(ctx, vec2(820.0, 500.0));
    }
}

// ------------------------------------------------------------------ settings

fn section(ui: &mut Ui, title: &str) {
    ui.add_space(14.0);
    ui.label(RichText::new(title).font(bold(16.0)).color(pal().text));
    ui.add_space(6.0);
}

fn group<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    egui::Frame::new()
        .fill(pal().group)
        .corner_radius(12.0)
        .inner_margin(egui::Margin::symmetric(16, 8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui)
        })
        .inner
}

fn toggle(ui: &mut Ui, on: &mut bool) -> bool {
    let (rect, resp) = ui.allocate_exact_size(vec2(44.0, 24.0), Sense::click());
    if resp.clicked() {
        *on = !*on;
    }
    let t = ui.ctx().animate_bool(resp.id, *on);
    let bg = lerp(pal().selected, pal().accent, t);
    ui.painter().rect_filled(rect, 12.0, bg);
    let x = egui::lerp(rect.left() + 12.0..=rect.right() - 12.0, t);
    ui.painter().circle_filled(pos2(x, rect.center().y), 10.0, if pal().dark { Color32::from_gray(232) } else { Color32::WHITE });
    resp.clicked()
}

fn setting_row(ui: &mut Ui, label: &str, right: impl FnOnce(&mut Ui)) {
    ui.horizontal(|ui| {
        ui.set_min_height(36.0);
        ui.label(RichText::new(label).size(15.0).color(pal().text));
        ui.with_layout(Layout::right_to_left(Align::Center), right);
    });
}

fn note(ui: &mut Ui, text: &str) {
    ui.label(RichText::new(text).size(12.5).color(pal().dim));
    ui.add_space(4.0);
}

fn sep(ui: &mut Ui) {
    let r = ui.cursor();
    ui.painter().line_segment([pos2(r.left(), r.top()), pos2(r.right(), r.top())], Stroke::new(1.0_f32, pal().separator));
    ui.add_space(2.0);
}

/// Settings live in their own window, like a Mac app's settings window: titled after the
/// selected tab, always the same size, closed with its close button, Esc or ⌘W.
pub fn settings_window(app: &mut App, ctx: &egui::Context) {
    if app.screenshot.is_some() && std::env::var("TYPISCH_NATIVE_SETTINGS").is_err() {
        // The check picture only shows the main window, so draw the settings inside it there
        // (TYPISCH_NATIVE_SETTINGS tests the separate window instead).
        settings_dialog_in_window(app, ctx);
        return;
    }
    let before = app.settings.clone();
    let title = match app.settings_tab {
        SettingsTab::General => "Allgemein",
        SettingsTab::Editor => "Editor",
        SettingsTab::Packages => "Pakete",
    };
    let size = [640.0, 660.0];
    if app.settings_pos.is_none() {
        app.settings_pos = ctx
            .input(|i| i.viewport().outer_rect)
            .map(|r| r.center() - vec2(size[0], size[1] + 28.0) / 2.0);
    }
    let mut builder = egui::ViewportBuilder::default()
        .with_title(title)
        .with_app_id("typisch-settings")
        .with_inner_size(size)
        .with_min_inner_size(size)
        .with_max_inner_size(size)
        .with_resizable(false)
        .with_minimize_button(false)
        .with_maximize_button(false);
    if let Some(pos) = app.settings_pos {
        builder = builder.with_position(pos.max(egui::Pos2::ZERO));
    }
    let mut open = true;
    ctx.show_viewport_immediate(egui::ViewportId::from_hash_of("typisch-settings"), builder, |ctx, _class| {
        let close_keys = ctx.input_mut(|i| {
            i.consume_key(egui::Modifiers::NONE, egui::Key::Escape) || i.consume_key(egui::Modifiers::COMMAND, egui::Key::W)
        });
        if close_keys || ctx.input(|i| i.viewport().close_requested()) {
            open = false;
        }
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(pal().window).inner_margin(egui::Margin { left: 18, right: 18, top: 10, bottom: 8 }))
            .show(ctx, |ui| settings_body(app, ui));
    });
    if !open {
        app.settings_open = false;
    }
    save_if_changed(app, &before);
}

fn settings_dialog_in_window(app: &mut App, ctx: &egui::Context) {
    let mut open = true;
    let before = app.settings.clone();
    // One fixed size for every tab (it only shrinks when the app window itself is too small),
    // a plain 1-pixel border and no drop shadow.
    let screen = ctx.content_rect();
    let size = vec2(640.0, 660.0).min(screen.size() - vec2(32.0, 32.0)).max(vec2(320.0, 240.0));
    let outer = Rect::from_center_size(screen.center(), size);
    if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        open = false;
    }
    egui::Area::new(egui::Id::new("settings"))
        .order(egui::Order::Foreground)
        .fixed_pos(outer.min)
        .constrain(false)
        .show(ctx, |ui| {
            ui.set_clip_rect(outer);
            ui.painter().rect(outer, 12.0, pal().window, Stroke::new(1.0_f32, pal().separator), StrokeKind::Inside);
            // Clicks on the dialog's empty parts must not reach the window below.
            ui.interact(outer, ui.id().with("settings-bg"), Sense::click_and_drag());
            let inner = outer.shrink2(vec2(18.0, 10.0));
            ui.scope_builder(UiBuilder::new().max_rect(inner), |ui| {
                ui.set_min_size(inner.size());
                ui.set_max_size(inner.size());
                // Title row: name of the tab in the middle, close button on the right.
                let title = match app.settings_tab {
                    SettingsTab::General => "Allgemein",
                    SettingsTab::Editor => "Editor",
                    SettingsTab::Packages => "Pakete",
                };
                let (row, _) = ui.allocate_exact_size(vec2(inner.width(), 34.0), Sense::hover());
                ui.painter().text(row.center(), Align2::CENTER_CENTER, title, FontId::proportional(17.0), pal().text_soft);
                let close = Rect::from_center_size(pos2(row.right() - 12.0, row.center().y), vec2(26.0, 26.0));
                let close_resp = ui.interact(close, ui.id().with("settings-close"), Sense::click()).on_hover_text("Schließen (Esc)");
                if close_resp.hovered() {
                    ui.painter().circle_filled(close.center(), 13.0, pal().hover);
                }
                paint(ui.painter(), close.shrink(8.0), Icon::Close, pal().text_soft);
                if close_resp.clicked() {
                    open = false;
                }
                settings_body(app, ui);
            });
        });
    if !open {
        app.settings_open = false;
    }
    save_if_changed(app, &before);
}

fn settings_body(app: &mut App, ui: &mut Ui) {
    {
        ui.horizontal(|ui| {
            let w = 3.0 * 96.0;
            ui.add_space((ui.available_width() - w) / 2.0);
            for (tab, icon, label) in [
                (SettingsTab::General, Icon::Gear, "Allgemein"),
                (SettingsTab::Editor, Icon::Document, "Editor"),
                (SettingsTab::Packages, Icon::Package, "Pakete"),
            ] {
                let (r, resp) = ui.allocate_exact_size(vec2(92.0, 64.0), Sense::click());
                let sel = app.settings_tab == tab;
                if sel {
                    ui.painter().rect_filled(r, 10.0, pal().selected);
                }
                let col = if sel { pal().accent } else { pal().text_soft };
                if tab == SettingsTab::Editor {
                    ui.painter().text(pos2(r.center().x, r.top() + 22.0), Align2::CENTER_CENTER, "Aa", FontId::proportional(24.0), col);
                } else {
                    paint(ui.painter(), Rect::from_center_size(pos2(r.center().x, r.top() + 22.0), vec2(28.0, 28.0)), icon, col);
                }
                ui.painter().text(pos2(r.center().x, r.bottom() - 13.0), Align2::CENTER_CENTER, label, FontId::proportional(13.5), col);
                if resp.clicked() {
                    app.settings_tab = tab;
                }
            }
        });
        ui.add_space(6.0);
        sep(ui);
        // The page scrolls inside the dialog; its width never changes the dialog.
        let width = ui.available_width();
        egui::ScrollArea::vertical().id_salt(("settings-page", app.settings_tab as u8)).auto_shrink(false).show(ui, |ui| {
            ui.set_width(width - 10.0);
            ui.set_max_width(width - 10.0);
            match app.settings_tab {
                SettingsTab::General => general_tab(app, ui),
                SettingsTab::Editor => editor_tab(app, ui),
                SettingsTab::Packages => packages_tab(app, ui),
            }
        });
    }
}

fn save_if_changed(app: &mut App, before: &crate::theme::Settings) {
    if before.line_numbers != app.settings.line_numbers
        || before.theme != app.settings.theme
        || before.follow_system != app.settings.follow_system
        || before.ui_scale != app.settings.ui_scale
        || before.font_size != app.settings.font_size
        || before.editor_left != app.settings.editor_left
        || before.ripple != app.settings.ripple
        || before.word_count != app.settings.word_count
        || before.relative_line_numbers != app.settings.relative_line_numbers
        || before.ignore_library_warnings != app.settings.ignore_library_warnings
        || before.auto_close != app.settings.auto_close
        || before.animated_cursor != app.settings.animated_cursor
        || before.monospace_font != app.settings.monospace_font
    {
        if app.screenshot.is_none() {
            app.settings.save();
        }
    }
}

fn general_tab(app: &mut App, ui: &mut Ui) {
    section(ui, "Darstellung");
    group(ui, |ui| {
        setting_row(ui, "Hell/Dunkel wie das System", |ui| {
            toggle(ui, &mut app.settings.follow_system);
        });
        sep(ui);
        note(ui, "Wechselt automatisch zwischen deinem hellen und deinem dunklen Theme, wenn das System umschaltet.");
        sep(ui);
        setting_row(ui, "Oberflächengröße", |ui| {
            ui.label(RichText::new(format!("{:.0} %", app.settings.ui_scale * 100.0)).size(15.0).color(pal().text));
            ui.spacing_mut().slider_width = 200.0;
            // Applied when the slider is let go, so the window doesn't jump under the pointer.
            let mut v = app.settings.ui_scale;
            let r = ui.add(egui::Slider::new(&mut v, 0.6..=2.0).step_by(0.05).show_value(false));
            if r.drag_stopped() || (r.changed() && !r.dragged()) {
                app.settings.ui_scale = v;
            }
        });
        sep(ui);
        note(ui, &format!(
            "Vergrößert oder verkleinert die ganze Oberfläche ({} / {}). Die Bildschirmskalierung des Systems wird schon berücksichtigt.",
            crate::app::shortcut_label("Mod+Shift++"),
            crate::app::shortcut_label("Mod+Shift+-")
        ));
    });
    section(ui, "Live-Vorschau");
    group(ui, |ui| {
        setting_row(ui, "Editor-Position", |ui| {
            egui::ComboBox::from_id_salt("pos")
                .selected_text(if app.settings.editor_left { "Links" } else { "Rechts" })
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut app.settings.editor_left, true, "Links");
                    ui.selectable_value(&mut app.settings.editor_left, false, "Rechts");
                });
        });
        sep(ui);
        note(ui, "Auf welcher Seite des Fensters der Editor liegt – die Live-Vorschau nimmt die andere Seite ein.");
        sep(ui);
        setting_row(ui, "Welle beim Klicken", |ui| {
            toggle(ui, &mut app.settings.ripple);
        });
        sep(ui);
        note(ui, "Lässt eine sanfte Welle vom angeklickten Punkt in der Vorschau ausgehen.");
    });
    section(ui, "Diagnose");
    group(ui, |ui| {
        setting_row(ui, "Bibliothekswarnungen ignorieren", |ui| {
            toggle(ui, &mut app.settings.ignore_library_warnings);
        });
        sep(ui);
        note(ui, "Blendet Warnungen aus importierten Paketen aus (z. B. @preview/…). Warnungen in eigenen Dateien und alle Fehler werden weiterhin angezeigt.");
    });
    section(ui, "Typst-Compiler");
    group(ui, |ui| {
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new("Latest (0.15)").font(bold(16.0)).color(pal().text));
            for (t, c) in [("Integriert", pal().selected), ("Ausgewählt", pal().badge)] {
                egui::Frame::new().fill(c).corner_radius(8.0).inner_margin(egui::Margin::symmetric(8, 3)).show(ui, |ui| {
                    ui.label(RichText::new(t).size(12.5).color(pal().text_soft));
                });
            }
        });
        note(ui, "Der eingebaute Typst-Compiler (Version 0.15). Er läuft vollständig auf diesem Rechner.");
        if app.last_compile_ms > 0 {
            note(ui, &format!("Letzter Durchlauf: {} ms", app.last_compile_ms));
        }
    });
    section(ui, "Über Typisch");
    group(ui, |ui| {
        setting_row(ui, "Typisch", |ui| {
            ui.label(RichText::new(crate::app::version_line()).size(14.0).color(pal().text_soft));
        });
    });
    ui.add_space(12.0);
}

fn editor_tab(app: &mut App, ui: &mut Ui) {
    section(ui, "Editor-Typografie");
    group(ui, |ui| {
        setting_row(ui, "Schrift", |ui| {
            egui::ComboBox::from_id_salt("font").selected_text(app.settings.monospace_font.clone()).show_ui(ui, |ui| {
                ui.selectable_value(&mut app.settings.monospace_font, "System Monospaced".to_string(), "System Monospaced");
            });
        });
        sep(ui);
        setting_row(ui, "Größe", |ui| {
            ui.label(RichText::new(format!("{:.0} pt", app.settings.font_size)).size(15.0).color(pal().text));
            ui.spacing_mut().slider_width = 260.0;
            ui.add(egui::Slider::new(&mut app.settings.font_size, 9.0..=28.0).step_by(1.0).show_value(false));
        });
    });
    section(ui, "Theme");
    group(ui, |ui| {
        // As many columns as fit, so the grid works in narrow windows and at large interface sizes.
        let cols = ((ui.available_width() + 12.0) / 172.0).floor().clamp(1.0, 3.0) as usize;
        let w = (ui.available_width() - 12.0 * (cols as f32 - 1.0)) / cols as f32;
        for chunk in THEMES.iter().enumerate().collect::<Vec<_>>().chunks(cols) {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 12.0;
                for (i, t) in chunk {
                    let (r, resp) = ui.allocate_exact_size(vec2(w, 96.0), Sense::click());
                    let sel = crate::theme::current_theme_index() == Some(*i);
                    if sel {
                        ui.painter().rect_filled(r, 12.0, pal().selected);
                    }
                    let card = Rect::from_min_size(r.min + vec2(10.0, 8.0), vec2(w - 20.0, 52.0));
                    ui.painter().rect_filled(card, 8.0, t.bg);
                    ui.painter().rect_stroke(card, 8.0, Stroke::new(if sel { 2.5_f32 } else { 1.0 }, if sel { pal().accent } else { pal().separator }), StrokeKind::Inside);
                    for (k, c) in t.swatch.iter().enumerate() {
                        ui.painter().circle_filled(pos2(card.left() + 22.0 + k as f32 * 22.0, card.center().y), 7.5, *c);
                    }
                    ui.painter().text(
                        pos2(r.center().x, r.bottom() - 16.0),
                        Align2::CENTER_CENTER,
                        t.name,
                        if sel { bold(14.0) } else { FontId::proportional(14.0) },
                        if sel { pal().text } else { pal().text_soft },
                    );
                    if resp.clicked() {
                        // Picking a theme of the other brightness than the system's stops following it.
                        let system_dark = ui.ctx().system_theme().map(|t| t == egui::Theme::Dark);
                        if app.settings.follow_system && system_dark.map(|d| d != t.dark).unwrap_or(false) {
                            app.settings.follow_system = false;
                        }
                        app.settings.choose_theme(*i);
                    }
                }
            });
            ui.add_space(6.0);
        }
        ui.add_space(6.0);
        ui.label(RichText::new("Preview").size(12.5).color(pal().dim));
        let t = app.settings.theme();
        egui::Frame::new().fill(t.bg).corner_radius(8.0).inner_margin(egui::Margin::symmetric(16, 12)).stroke(Stroke::new(1.0_f32, pal().separator)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            let job = crate::highlight::layout_job(
                "#let answer = 42\n_Typst_ ist *großartig*!",
                t,
                FontId::monospace(app.settings.font_size + 2.0),
                f32::INFINITY,
                (app.settings.font_size + 2.0) * 1.4,
            );
            ui.label(job);
        });
    });
    section(ui, "Allgemein");
    group(ui, |ui| {
        let rows: [(&str, &mut bool); 4] = [
            ("Blinkende Einfügemarke", &mut app.settings.animated_cursor),
            ("Zeilennummern", &mut app.settings.line_numbers),
            ("Wortanzahl anzeigen", &mut app.settings.word_count),
            ("Relative Zeilennummern", &mut app.settings.relative_line_numbers),
        ];
        for (i, (label, val)) in rows.into_iter().enumerate() {
            if i > 0 {
                sep(ui);
            }
            setting_row(ui, label, |ui| {
                toggle(ui, val);
            });
        }
        sep(ui);
        setting_row(ui, "Automatisch schließen", |ui| {
            egui::ComboBox::from_id_salt("autoclose").selected_text(app.settings.auto_close.label()).show_ui(ui, |ui| {
                for v in [AutoClose::Immediately, AutoClose::BeforeWhitespace, AutoClose::Never] {
                    ui.selectable_value(&mut app.settings.auto_close, v, v.label());
                }
            });
        });
        sep(ui);
        note(ui, "Klammern, Anführungszeichen und $ werden beim Tippen automatisch geschlossen.");
    });
    ui.add_space(12.0);
}

fn package_dirs() -> Vec<std::path::PathBuf> {
    let mut v = Vec::new();
    if let Some(d) = dirs::cache_dir() {
        v.push(d.join("typst").join("packages"));
    }
    v
}

fn packages_tab(app: &mut App, ui: &mut Ui) {
    // Results from background downloads land in pkg_message.
    ui.add_space(10.0);
    ui.horizontal(|ui| {
        paint(ui.painter(), Rect::from_center_size(ui.cursor().min + vec2(10.0, 11.0), vec2(18.0, 18.0)), Icon::Package, pal().text_soft);
        ui.add_space(26.0);
        ui.label(RichText::new("Online-Pakete").size(15.5).color(pal().text));
    });
    group(ui, |ui| {
        note(ui, "Werden automatisch geladen, wenn Sie #import \"@preview/…\" verwenden. Laden Sie eines vorab per Angabe:");
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut app.pkg_spec).hint_text("@preview/cetz:0.3.1").desired_width(ui.available_width() - 90.0).margin(vec2(8.0, 6.0)));
            if ui.add_enabled(!app.pkg_spec.trim().is_empty(), egui::Button::new("Laden").min_size(vec2(80.0, 28.0))).clicked() {
                app.compiler.send(Job::FetchPackage { spec: app.pkg_spec.clone() });
                app.pkg_message = Some((true, "Wird geladen …".into()));
            }
        });
        if let Some((ok, msg)) = &app.pkg_message {
            ui.label(RichText::new(msg).size(12.5).color(if *ok { lerp(pal().text, Color32::from_rgb(0x40, 0xb0, 0x40), 0.7) } else { pal().error }));
        }
        ui.add_space(6.0);
        let mut installed = Vec::new();
        for base in package_dirs() {
            let Ok(ns) = std::fs::read_dir(&base) else { continue };
            for ns in ns.flatten() {
                let Ok(names) = std::fs::read_dir(ns.path()) else { continue };
                for name in names.flatten() {
                    let Ok(versions) = std::fs::read_dir(name.path()) else { continue };
                    for v in versions.flatten() {
                        installed.push((
                            name.file_name().to_string_lossy().to_string(),
                            ns.file_name().to_string_lossy().to_string(),
                            v.file_name().to_string_lossy().to_string(),
                            v.path(),
                        ));
                    }
                }
            }
        }
        installed.sort();
        let mut delete = None;
        if installed.is_empty() {
            ui.add_space(8.0);
            ui.vertical_centered(|ui| ui.label(RichText::new("Noch keine Pakete geladen").color(pal().dim)));
            ui.add_space(8.0);
        }
        for (name, ns, ver, path) in &installed {
            let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 52.0), Sense::hover());
            if resp.hovered() {
                ui.painter().rect_filled(r, 8.0, pal().hover);
            }
            paint(ui.painter(), Rect::from_center_size(pos2(r.left() + 22.0, r.center().y), vec2(20.0, 20.0)), Icon::Package, pal().dim);
            ui.painter().text(pos2(r.left() + 46.0, r.top() + 17.0), Align2::LEFT_CENTER, name, bold(16.0), pal().text);
            ui.painter().text(pos2(r.left() + 46.0, r.top() + 37.0), Align2::LEFT_CENTER, format!("{ns} · {ver}"), FontId::proportional(13.0), pal().dim);
            let tr = Rect::from_center_size(pos2(r.right() - 24.0, r.center().y), vec2(26.0, 26.0));
            let tresp = ui.interact(tr, ui.id().with(("del", path)), Sense::click()).on_hover_text("Aus dem Zwischenspeicher löschen");
            paint(ui.painter(), tr.shrink(4.0), Icon::Trash, if tresp.hovered() { pal().text } else if resp.hovered() { pal().dim } else { pal().faint });
            if tresp.clicked() {
                delete = Some(path.clone());
            }
        }
        if let Some(p) = delete {
            let _ = std::fs::remove_dir_all(p);
        }
    });

    ui.add_space(18.0);
    ui.horizontal(|ui| {
        paint(ui.painter(), Rect::from_center_size(ui.cursor().min + vec2(10.0, 11.0), vec2(18.0, 18.0)), Icon::Folder, pal().text_soft);
        ui.add_space(26.0);
        ui.label(RichText::new("Lokale Pakete").size(15.5).color(pal().text));
    });
    group(ui, |ui| {
        note(ui, "Registrieren Sie einen Ordner als Namespace. Ein Ordner mit meine_lib/<version>/, registriert als „privat“, wird als @privat/meine_lib:<version> importiert. Beim Entfernen wird nur die Zuordnung vergessen – Ihre Dateien bleiben unangetastet.");
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut app.namespace_input).hint_text("Namespace (optional)").desired_width(ui.available_width() - 170.0).margin(vec2(8.0, 6.0)));
            if ui.add(egui::Button::new("Ordner verknüpfen …").min_size(vec2(160.0, 28.0))).clicked() {
                if let Some(dir) = rfd::FileDialog::new().pick_folder() {
                    let ns = if app.namespace_input.trim().is_empty() { "local".to_string() } else { app.namespace_input.trim().to_string() };
                    match link_namespace(&ns, &dir) {
                        Ok(()) => {
                            app.settings.local_namespaces.retain(|(n, _)| n != &ns);
                            app.settings.local_namespaces.push((ns, dir));
                            app.namespace_input.clear();
                            app.settings.save();
                        }
                        Err(e) => app.pkg_message = Some((false, e)),
                    }
                }
            }
        });
        ui.add_space(8.0);
        if app.settings.local_namespaces.is_empty() {
            ui.vertical_centered(|ui| ui.label(RichText::new("Noch keine Namespaces registriert").color(pal().faint)));
            ui.add_space(8.0);
        }
        let mut remove = None;
        for (i, (ns, dir)) in app.settings.local_namespaces.iter().enumerate() {
            ui.horizontal(|ui| {
                ui.label(RichText::new(format!("@{ns}")).font(bold(14.5)).color(pal().text));
                ui.label(RichText::new(tilde(dir)).size(12.5).color(pal().dim));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if icon_button(ui, Icon::Trash, 26.0, pal().dim, false, "Zuordnung entfernen").clicked() {
                        remove = Some(i);
                    }
                });
            });
        }
        if let Some(i) = remove {
            let (ns, _) = app.settings.local_namespaces.remove(i);
            if let Some(d) = dirs::data_dir() {
                let link = d.join("typst").join("packages").join(&ns);
                if link.is_symlink() {
                    let _ = std::fs::remove_file(link);
                }
            }
            app.settings.save();
        }
    });
    ui.add_space(12.0);
}

/// Local packages live in `<data dir>/typst/packages/<namespace>`; we link the chosen folder there.
fn link_namespace(ns: &str, dir: &std::path::Path) -> Result<(), String> {
    let base = dirs::data_dir().ok_or("Kein Datenordner gefunden")?.join("typst").join("packages");
    std::fs::create_dir_all(&base).map_err(|e| e.to_string())?;
    let link = base.join(ns);
    if link.exists() || link.is_symlink() {
        if link.is_symlink() {
            std::fs::remove_file(&link).map_err(|e| e.to_string())?;
        } else {
            return Err(format!("Der Namespace „{ns}“ ist bereits ein echter Ordner."));
        }
    }
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(dir, &link).map_err(|e| e.to_string())
    }
    #[cfg(not(unix))]
    {
        let _ = dir;
        Err("Lokale Pakete werden auf diesem System nicht unterstützt.".into())
    }
}
