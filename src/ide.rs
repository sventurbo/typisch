//! The main window: sidebar, code editor and live preview.

use std::time::{Duration, Instant};

use eframe::egui::{
    self, Align, Align2, Color32, CornerRadius, FontId, Layout, Pos2, Rect, RichText, Sense, Shape, Stroke, StrokeKind,
    Ui, UiBuilder, pos2, vec2,
};

use crate::app::{App, SearchMode, SideTab, bold};
use crate::icons::{Icon, icon_button, paint};
use crate::menu::{PopupItem, open_popup, popup_choice};
use crate::theme::pal;

pub enum EditOp {
    Wrap(String, String),
    LinePrefix(String),
    Insert(String),
}

const TOP_H: f32 = 51.0;

/// Room for the macOS window buttons, which sit inside the window like in the original.
pub const MAC_BUTTONS_W: f32 = if cfg!(target_os = "macos") { 76.0 } else { 0.0 };
const EDITOR_MIN_W: f32 = 280.0;
const PREVIEW_MIN_W: f32 = 240.0;

/// Empty bar areas move the window (and double-click maximizes), like a native title bar.
pub fn window_drag_area(ui: &Ui, rect: Rect, id: &str) {
    let resp = ui.interact(rect, ui.id().with(id), Sense::click_and_drag());
    if resp.drag_started_by(egui::PointerButton::Primary) {
        ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
    }
    if resp.double_clicked() {
        let maximized = ui.ctx().input(|i| i.viewport().maximized.unwrap_or(false));
        ui.ctx().send_viewport_cmd(egui::ViewportCommand::Maximized(!maximized));
    }
}

pub fn show(app: &mut App, ctx: &egui::Context) {
    // On small windows the sidebar gives way first, so the editor keeps a usable width.
    let window_w = ctx.content_rect().width();
    let needed = if app.preview_visible { 258.0 + EDITOR_MIN_W + PREVIEW_MIN_W + 120.0 } else { 258.0 + EDITOR_MIN_W + 120.0 };
    let sidebar_fits = window_w >= needed;
    if app.settings.sidebar_visible && sidebar_fits {
        // The sidebar keeps exactly the width the person gave it, whichever tab is showing;
        // only dragging its right edge changes it.
        let max_w = (window_w * 0.4).clamp(200.0, 460.0);
        let width = app.settings.sidebar_width.clamp(200.0, max_w);
        let panel = egui::SidePanel::left("sidebar")
            .resizable(false)
            .exact_width(width)
            .show_separator_line(false)
            .frame(egui::Frame::new().fill(pal().sidebar))
            .show(ctx, |ui| {
                let rect = ui.max_rect();
                ui.set_clip_rect(rect);
                sidebar(app, ui);
            });
        let edge = panel.response.rect.right();
        let handle = Rect::from_min_max(pos2(edge - 3.0, panel.response.rect.top()), pos2(edge + 3.0, panel.response.rect.bottom()));
        let resp = egui::Area::new(egui::Id::new("sidebar-resize"))
            .order(egui::Order::Middle)
            .fixed_pos(handle.min)
            .show(ctx, |ui| {
                let (r, resp) = ui.allocate_exact_size(handle.size(), Sense::click_and_drag());
                if resp.hovered() || resp.dragged() {
                    ui.painter().line_segment(
                        [pos2(r.center().x, r.top()), pos2(r.center().x, r.bottom())],
                        Stroke::new(2.0_f32, pal().accent.gamma_multiply(0.6)),
                    );
                }
                resp.on_hover_cursor(egui::CursorIcon::ResizeHorizontal)
            })
            .inner;
        if resp.dragged() {
            if let Some(p) = resp.interact_pointer_pos() {
                app.settings.sidebar_width = (p.x - panel.response.rect.left()).clamp(200.0, max_w).round();
            }
        }
        if resp.double_clicked() {
            app.settings.sidebar_width = crate::theme::Settings::default().sidebar_width;
        }
        if resp.drag_stopped() && app.screenshot.is_none() {
            app.settings.save();
        }
    }
    egui::TopBottomPanel::top("topbar")
        .exact_height(TOP_H)
        .frame(egui::Frame::new().fill(pal().topbar).inner_margin(egui::Margin { left: 12, right: 14, top: 0, bottom: 0 }))
        .show_separator_line(false)
        .show(ctx, |ui| topbar(app, ui, sidebar_fits));
    egui::CentralPanel::default().frame(egui::Frame::new().fill(pal().preview_bg)).show(ctx, |ui| {
        let full = ui.max_rect();
        let room_for_both = full.width() >= EDITOR_MIN_W + PREVIEW_MIN_W;
        if app.preview_visible && room_for_both {
            // The split between editor and preview can be dragged; double-click resets it.
            let ratio = app.settings.editor_ratio.clamp(0.15, 0.85);
            let share = if app.settings.editor_left { ratio } else { 1.0 - ratio };
            let split = (full.width() * share).round().clamp(
                if app.settings.editor_left { EDITOR_MIN_W } else { PREVIEW_MIN_W },
                full.width() - if app.settings.editor_left { PREVIEW_MIN_W } else { EDITOR_MIN_W },
            );
            let (first, second) = (
                Rect::from_min_size(full.min, vec2(split, full.height())),
                Rect::from_min_max(pos2(full.min.x + split, full.min.y), full.max),
            );
            let (left, right) = if app.settings.editor_left { (first, second) } else { (second, first) };
            ui.scope_builder(UiBuilder::new().max_rect(left), |ui| editor_pane(app, ui));
            ui.scope_builder(UiBuilder::new().max_rect(right), |ui| preview_pane(app, ui));

            let handle = Rect::from_center_size(pos2(full.min.x + split, full.center().y), vec2(8.0, full.height()));
            let resp = ui
                .interact(handle, ui.id().with("split"), Sense::click_and_drag())
                .on_hover_cursor(egui::CursorIcon::ResizeHorizontal);
            if resp.hovered() || resp.dragged() {
                ui.painter().line_segment(
                    [pos2(handle.center().x, full.top()), pos2(handle.center().x, full.bottom())],
                    Stroke::new(2.0_f32, pal().accent.gamma_multiply(0.6)),
                );
            }
            if resp.dragged() {
                let x = (handle.center().x + resp.drag_delta().x - full.min.x) / full.width();
                let editor_share = if app.settings.editor_left { x } else { 1.0 - x };
                app.settings.editor_ratio = editor_share.clamp(0.15, 0.85);
            }
            if resp.drag_stopped() && app.screenshot.is_none() {
                app.settings.save();
            }
            if resp.double_clicked() {
                app.settings.editor_ratio = crate::theme::Settings::default().editor_ratio;
            }
        } else {
            ui.scope_builder(UiBuilder::new().max_rect(full), |ui| editor_pane(app, ui));
        }
    });
}

// ------------------------------------------------------------------ top bar

fn topbar(app: &mut App, ui: &mut Ui, sidebar_fits: bool) {
    window_drag_area(ui, ui.max_rect().expand2(vec2(14.0, 0.0)), "topbar-drag");
    let sidebar_shown = app.settings.sidebar_visible && sidebar_fits;
    ui.horizontal_centered(|ui| {
        if !sidebar_shown {
            ui.add_space(MAC_BUTTONS_W);
            if icon_button(ui, Icon::Sidebar, 34.0, pal().dim, false, &format!("Seitenleiste einblenden ({})", crate::menu::shortcut_text("CmdOrCtrl+Alt+S")))
                .clicked()
            {
                app.settings.sidebar_visible = true;
            }
            ui.add_space(6.0);
        }
        let name = app.doc.as_ref().map(|d| d.rel.clone()).unwrap_or_default();
        let unsaved = app.doc.as_ref().map(|d| d.text != d.saved).unwrap_or(false);
        let project = app.project.as_ref().and_then(|p| p.root.file_name()).map(|n| n.to_string_lossy().to_string());
        if let Some(project) = project {
            // Plain text, no box behind it.
            ui.add_space(4.0);
            ui.label(RichText::new(project).color(pal().dim).size(13.0)).on_hover_text("Projektordner");
            ui.label(RichText::new("›").color(pal().faint).size(15.0));
        }
        // Right-hand buttons first, so a long file name gets shortened instead of pushing them out.
        let right_w = 2.0 * 32.0 + 24.0;
        let name_w = (ui.available_width() - right_w - 20.0).max(40.0);
        ui.allocate_ui(vec2(name_w, ui.available_height()), |ui| {
            ui.horizontal_centered(|ui| {
                ui.add(egui::Label::new(RichText::new(name).font(bold(15.5)).color(pal().text)).truncate());
                if unsaved {
                    ui.label(RichText::new("•").color(pal().dim)).on_hover_text("Nicht gespeicherte Änderungen");
                }
            });
        });
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            // Flat icon buttons: a highlight shows only on hover.
            ui.scope(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                let prev_col = if app.preview_visible { pal().accent } else { pal().dim };
                if icon_button(ui, Icon::Preview, 30.0, prev_col, false, "Vorschau ein-/ausblenden").clicked() {
                    app.preview_visible = !app.preview_visible;
                }
                let errors = app.error_count();
                let warn_col = if errors > 0 { pal().error } else { pal().text_soft };
                if icon_button(ui, Icon::Warning, 30.0, warn_col, false, "Probleme").clicked() {
                    app.settings.sidebar_visible = true;
                    app.side_tab = SideTab::Problems;
                }
            });
            if app.compiling && app.started.elapsed() > Duration::from_millis(400) {
                ui.add(egui::Spinner::new().size(14.0).color(pal().dim));
            }
        });
    });
}

// ------------------------------------------------------------------ sidebar

fn sidebar(app: &mut App, ui: &mut Ui) {
    let rect = ui.max_rect();
    window_drag_area(ui, Rect::from_min_size(rect.min, vec2(rect.width(), TOP_H)), "sidebar-drag");
    // Toggle sits where the top bar is.
    let toggle = Rect::from_center_size(pos2(rect.right() - 38.0, rect.top() + TOP_H / 2.0), vec2(34.0, 34.0));
    let resp = ui.interact(toggle, ui.id().with("sb-toggle"), Sense::click()).on_hover_text("Seitenleiste ausblenden");
    if resp.hovered() {
        ui.painter().circle_filled(toggle.center(), 15.0, pal().hover);
    }
    paint(ui.painter(), toggle.shrink(8.0), Icon::Sidebar, pal().text_soft);
    if resp.clicked() {
        app.settings.sidebar_visible = false;
    }

    // Tab pill
    let tabs = [
        (SideTab::Files, Icon::Folder, "Dateien"),
        (SideTab::Search, Icon::Search, "Suchen"),
        (SideTab::Outline, Icon::Outline, "Gliederung"),
        (SideTab::Symbols, Icon::Sqrt, "Symbole"),
        (SideTab::Problems, Icon::Warning, "Probleme"),
    ];
    let bw = 28.5;
    let pill_w = bw * tabs.len() as f32 + 12.0;
    let pill = Rect::from_center_size(pos2(rect.center().x - 4.0, rect.top() + TOP_H + 13.0), vec2(pill_w, 28.0));
    ui.painter().rect_filled(pill, 14.0, pal().pill);
    for (i, (tab, icon, tip)) in tabs.iter().enumerate() {
        let r = Rect::from_min_size(pos2(pill.left() + 6.0 + i as f32 * bw, pill.top() + 1.0), vec2(bw, 26.0));
        let resp = ui.interact(r, ui.id().with(("tab", i)), Sense::click()).on_hover_text(*tip);
        if app.side_tab == *tab {
            ui.painter().rect_filled(r.expand2(vec2(1.0, -0.5)), 13.0, pal().pill_selected);
        } else if resp.hovered() {
            ui.painter().rect_filled(r.expand2(vec2(1.0, -0.5)), 13.0, pal().pill_hover);
        }
        let col = if *tab == SideTab::Problems && app.error_count() > 0 { pal().error } else { pal().text_soft };
        paint(ui.painter(), Rect::from_center_size(r.center(), vec2(16.0, 16.0)), *icon, col);
        if i > 0 && i < tabs.len() && app.side_tab != *tab && app.side_tab != tabs[i - 1].0 {
            ui.painter().line_segment(
                [pos2(r.left(), r.top() + 9.0), pos2(r.left(), r.bottom() - 9.0)],
                Stroke::new(1.0_f32, pal().separator),
            );
        }
        if resp.clicked() {
            app.side_tab = *tab;
        }
    }

    let content = Rect::from_min_max(pos2(rect.left(), pill.bottom() + 5.0), rect.max);
    ui.scope_builder(UiBuilder::new().max_rect(content), |ui| {
        ui.spacing_mut().item_spacing.y = 2.0;
        match app.side_tab {
            SideTab::Files => files_tab(app, ui),
            SideTab::Search => search_tab(app, ui),
            SideTab::Outline => outline_tab(app, ui),
            SideTab::Symbols => symbols_tab(app, ui),
            SideTab::Problems => problems_tab(app, ui),
        }
    });
}

fn file_icon(ui: &Ui, rect: Rect, name: &str, is_dir: bool) {
    let lower = name.to_lowercase();
    if is_dir {
        paint(ui.painter(), rect, Icon::Folder, pal().accent);
    } else if lower.ends_with(".typ") {
        ui.painter().text(rect.center(), Align2::CENTER_CENTER, "t", bold(17.0), pal().file_typ);
    } else if [".png", ".jpg", ".jpeg", ".gif", ".svg", ".webp"].iter().any(|e| lower.ends_with(e)) {
        paint(ui.painter(), rect, Icon::Image, pal().file_image);
    } else {
        paint(ui.painter(), rect, Icon::Document, pal().dim);
    }
}

fn row(ui: &mut Ui, height: f32, selected: bool) -> (Rect, egui::Response) {
    let w = ui.available_width();
    let (rect, resp) = ui.allocate_exact_size(vec2(w, height), Sense::click());
    let r = rect.shrink2(vec2(6.0, 0.0));
    if selected {
        ui.painter().rect_filled(r, 7.0, pal().row_selected);
    } else if resp.hovered() {
        ui.painter().rect_filled(r, 7.0, pal().hover);
    }
    (r, resp)
}

fn files_tab(app: &mut App, ui: &mut Ui) {
    let Some(project) = app.project.as_ref() else { return };
    let entries = project.files.clone();
    let collapsed = project.collapsed.clone();
    let main_rel = project.main_rel.clone();
    let open_rel = app.doc.as_ref().map(|d| d.rel.clone());
    let mut open_file: Option<String> = None;
    let mut toggle: Option<String> = None;
    let mut set_main: Option<String> = None;
    let mut refresh = false;
    egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
        let mut hide_below: Option<usize> = None;
        for e in &entries {
            if let Some(d) = hide_below {
                if e.depth > d {
                    continue;
                }
                hide_below = None;
            }
            let selected = open_rel.as_deref() == Some(e.rel.as_str());
            let (r, resp) = row(ui, 25.0, selected);
            let x = r.left() + 12.0 + e.depth as f32 * 16.0;
            if e.is_dir {
                let is_collapsed = collapsed.contains(&e.rel);
                paint(
                    ui.painter(),
                    Rect::from_center_size(pos2(x + 2.0, r.center().y), vec2(9.0, 9.0)),
                    if is_collapsed { Icon::ChevronRight } else { Icon::ChevronDown },
                    pal().dim,
                );
                if is_collapsed {
                    hide_below = Some(e.depth);
                }
            }
            file_icon(ui, Rect::from_center_size(pos2(x + 22.0, r.center().y), vec2(18.0, 18.0)), &e.name, e.is_dir);
            ui.painter().text(pos2(x + 38.0, r.center().y), Align2::LEFT_CENTER, &e.name, FontId::proportional(14.0), pal().text);
            if e.rel == main_rel {
                ui.painter().text(pos2(r.right() - 10.0, r.center().y), Align2::RIGHT_CENTER, "Haupt", FontId::proportional(10.5), if selected { pal().text_soft } else { pal().dim });
            }
            if resp.clicked() {
                if e.is_dir {
                    toggle = Some(e.rel.clone());
                } else {
                    open_file = Some(e.rel.clone());
                }
            }
            if resp.secondary_clicked() {
                let is_typ = !e.is_dir && e.rel.ends_with(".typ");
                let items = vec![
                    PopupItem::new("Im Tab öffnen").enabled(!e.is_dir),
                    PopupItem::new("Als Hauptdatei festlegen").enabled(is_typ).checked(e.rel == main_rel).sep(),
                    PopupItem::new(if cfg!(target_os = "macos") { "Im Finder zeigen" } else { "Im Dateimanager zeigen" }).sep(),
                    PopupItem::new("Dateiliste neu laden"),
                ];
                let at = resp.interact_pointer_pos().unwrap_or(r.left_bottom());
                open_popup(ui.ctx(), "file-menu", e.rel.clone(), items, at);
            }
        }
        if entries.is_empty() {
            ui.add_space(20.0);
            ui.vertical_centered(|ui| ui.label(RichText::new("Keine Dateien").color(pal().dim)));
        }
    });
    if let Some((choice, rel)) = popup_choice("file-menu") {
        match choice {
            0 => open_file = Some(rel),
            1 => set_main = Some(rel),
            2 => {
                if let Some(p) = app.project.as_ref() {
                    reveal_in_file_manager(&p.root.join(&rel));
                }
            }
            _ => refresh = true,
        }
    }
    if let Some(rel) = toggle {
        let p = app.project.as_mut().unwrap();
        if !p.collapsed.remove(&rel) {
            p.collapsed.insert(rel);
        }
    }
    if let Some(rel) = open_file {
        let lower = rel.to_lowercase();
        let texty = [".typ", ".bib", ".txt", ".md", ".yml", ".yaml", ".toml", ".csv", ".json", ".xml", ".svg"].iter().any(|e| lower.ends_with(e));
        if texty {
            app.open_file(&rel);
            if lower.ends_with(".typ") && app.project.as_ref().map(|p| !p.root.join(&p.main_rel).exists()).unwrap_or(false) {
                app.project.as_mut().unwrap().main_rel = rel;
            }
            app.schedule_compile(Duration::ZERO);
        } else if let Some(p) = app.project.as_ref() {
            let _ = open_external(&p.root.join(&rel));
        }
    }
    if let Some(rel) = set_main {
        app.project.as_mut().unwrap().main_rel = rel;
        app.schedule_compile(Duration::ZERO);
    }
    if refresh {
        app.refresh_files();
    }
}

/// Shows the file in Finder (or the file manager) with it selected.
pub fn reveal_in_file_manager(path: &std::path::Path) {
    #[cfg(target_os = "macos")]
    let _ = std::process::Command::new("open").arg("-R").arg(path).spawn();
    #[cfg(not(target_os = "macos"))]
    let _ = std::process::Command::new("xdg-open").arg(path.parent().unwrap_or(path)).spawn();
}

fn open_external(path: &std::path::Path) -> std::io::Result<()> {
    #[cfg(target_os = "macos")]
    let cmd = "open";
    #[cfg(not(target_os = "macos"))]
    let cmd = "xdg-open";
    std::process::Command::new(cmd).arg(path).spawn().map(|_| ())
}

fn search_field(ui: &mut Ui, text: &mut String, hint: &str, id: &str) -> egui::Response {
    let w = ui.available_width() - 24.0;
    let mut out = None;
    ui.horizontal(|ui| {
        ui.add_space(12.0);
        let frame = egui::Frame::new().fill(pal().surface).corner_radius(8.0).inner_margin(egui::Margin { left: 30, right: 28, top: 6, bottom: 6 });
        let r = frame.show(ui, |ui| {
            ui.add(egui::TextEdit::singleline(text).id_salt(id).hint_text(hint).frame(false).desired_width(w - 58.0))
        });
        let fr = r.response.rect;
        paint(ui.painter(), Rect::from_center_size(pos2(fr.left() + 16.0, fr.center().y), vec2(14.0, 14.0)), Icon::Search, pal().dim);
        if !text.is_empty() {
            let c = Rect::from_center_size(pos2(fr.right() - 15.0, fr.center().y), vec2(16.0, 16.0));
            let resp = ui.interact(c, ui.id().with((id, "clear")), Sense::click());
            ui.painter().circle_filled(c.center(), 8.0, if resp.hovered() { pal().dim } else { pal().faint });
            paint(ui.painter(), c.shrink(4.5), Icon::Close, pal().surface);
            if resp.clicked() {
                text.clear();
            }
        }
        out = Some(r.inner);
    });
    out.unwrap()
}

fn search_tab(app: &mut App, ui: &mut Ui) {
    ui.horizontal(|ui| {
        ui.add_space(12.0);
        ui.spacing_mut().item_spacing.x = 4.0;
        ui.label(RichText::new("Suchen").color(pal().text_soft).size(12.5));
        paint(ui.painter(), Rect::from_center_size(ui.cursor().min + vec2(6.0, 9.0), vec2(8.0, 8.0)), Icon::ChevronRight, pal().dim);
        ui.add_space(12.0);
        let label = match app.search_mode {
            SearchMode::Containing => "Enthält",
            SearchMode::WholeWord => "Ganzes Wort",
            SearchMode::StartsWith => "Beginnt mit",
        };
        let before = app.search_mode;
        let resp = ui
            .add(egui::Button::new(RichText::new(format!("{label} ⌄")).color(pal().text_soft).size(12.5)).frame(false))
            .on_hover_text("Suchart");
        if resp.clicked() {
            let modes = [SearchMode::Containing, SearchMode::WholeWord, SearchMode::StartsWith];
            let items = ["Enthält", "Ganzes Wort", "Beginnt mit"]
                .iter()
                .zip(modes)
                .map(|(l, m)| PopupItem::new(*l).checked(app.search_mode == m))
                .collect();
            open_popup(ui.ctx(), "search-mode", "", items, resp.rect.left_bottom() + vec2(0.0, 4.0));
        }
        if let Some((i, _)) = popup_choice("search-mode") {
            app.search_mode = [SearchMode::Containing, SearchMode::WholeWord, SearchMode::StartsWith][i.min(2)];
        }
        if before != app.search_mode {
            app.run_search();
        }
    });
    ui.add_space(4.0);
    let mut q = std::mem::take(&mut app.search_query);
    search_field(ui, &mut q, "Im Projekt suchen", "search");
    app.search_query = q;
    if app.search_query != app.search_for {
        app.run_search();
    }
    ui.add_space(8.0);
    if !app.search_for.is_empty() {
        let files: std::collections::BTreeSet<&str> = app.search_hits.iter().map(|h| h.rel.as_str()).collect();
        let n = app.search_hits.len();
        let text = format!(
            "{} {} in {} {}",
            n,
            if n == 1 { "Ergebnis" } else { "Ergebnisse" },
            files.len(),
            if files.len() == 1 { "Datei" } else { "Dateien" }
        );
        ui.vertical_centered(|ui| ui.label(RichText::new(text).color(pal().dim).size(12.0)));
        ui.add_space(4.0);
        let r = ui.cursor();
        ui.painter().line_segment([pos2(r.left(), r.top()), pos2(r.right(), r.top())], Stroke::new(1.0_f32, pal().separator));
        ui.add_space(8.0);
    }
    let mut jump: Option<(String, usize, usize)> = None;
    egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
        let mut last_file = "";
        for (i, hit) in app.search_hits.iter().enumerate() {
            if hit.rel != last_file {
                last_file = &hit.rel;
                let count = app.search_hits.iter().filter(|h| h.rel == hit.rel).count();
                let (r, _) = row(ui, 30.0, false);
                paint(ui.painter(), Rect::from_center_size(pos2(r.left() + 8.0, r.center().y), vec2(9.0, 9.0)), Icon::ChevronDown, pal().dim);
                file_icon(ui, Rect::from_center_size(pos2(r.left() + 28.0, r.center().y), vec2(17.0, 17.0)), &hit.rel, false);
                ui.painter().text(pos2(r.left() + 44.0, r.center().y), Align2::LEFT_CENTER, &hit.rel, bold(14.5), pal().text);
                let badge = Rect::from_center_size(pos2(r.right() - 16.0, r.center().y), vec2(22.0, 18.0));
                ui.painter().rect_filled(badge, 9.0, pal().badge);
                ui.painter().text(badge.center(), Align2::CENTER_CENTER, count.to_string(), FontId::proportional(11.0), pal().dim);
            }
            let (r, resp) = row(ui, 26.0, false);
            let resp = resp.on_hover_text(hit.text.trim());
            ui.painter().text(pos2(r.left() + 34.0, r.center().y), Align2::RIGHT_CENTER, (hit.line + 1).to_string(), FontId::proportional(12.5), pal().dim);
            let mut job = egui::text::LayoutJob::default();
            let line = &hit.text;
            let trim = line.len() - line.trim_start().len();
            let (a, b) = (hit.start.max(trim), (hit.start + hit.len).min(line.len()));
            let f = FontId::proportional(13.5);
            let plain = egui::TextFormat { font_id: f.clone(), color: pal().text_soft, ..Default::default() };
            let strong = egui::TextFormat {
                font_id: bold(13.5),
                color: pal().text,
                background: pal().selected,
                ..Default::default()
            };
            if line.is_char_boundary(a) && line.is_char_boundary(b) && a <= b {
                job.append(&line[trim.min(a)..a], 0.0, plain.clone());
                job.append(&line[a..b], 0.0, strong);
                job.append(&line[b..], 0.0, plain);
            } else {
                job.append(line.trim(), 0.0, plain);
            }
            job.wrap.max_width = r.width() - 52.0;
            job.wrap.max_rows = 1;
            job.wrap.break_anywhere = true;
            job.wrap.overflow_character = Some('…');
            let galley = ui.fonts_mut(|fo| fo.layout_job(job));
            ui.painter().galley(pos2(r.left() + 44.0, r.center().y - galley.size().y / 2.0), galley, pal().text);
            if resp.clicked() {
                let col = hit.text[..hit.start.min(hit.text.len())].chars().count();
                jump = Some((hit.rel.clone(), hit.line, col));
            }
            let _ = i;
        }
    });
    if let Some((rel, line, col)) = jump {
        go_to(app, &rel, line, col);
    }
}

pub fn go_to(app: &mut App, rel: &str, line: usize, col: usize) {
    if app.doc.as_ref().map(|d| d.rel != rel).unwrap_or(true) {
        if rel.contains('@') || rel.starts_with('@') {
            return;
        }
        app.open_file(rel);
    }
    app.jump_to = Some((line, col));
}

fn outline_tab(app: &mut App, ui: &mut Ui) {
    if app.outline.is_empty() {
        ui.add_space(30.0);
        ui.vertical_centered(|ui| ui.label(RichText::new("Keine Überschriften").color(pal().dim)));
        return;
    }
    let items = app.outline.clone();
    egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
        for item in items {
            let (r, resp) = row(ui, 28.0, false);
            let x = r.left() + 24.0 + (item.level.saturating_sub(1)) as f32 * 14.0;
            let font = if item.level == 1 { bold(14.5) } else { FontId::proportional(14.0) };
            let galley = ui.fonts_mut(|f| {
                let mut job = egui::text::LayoutJob::simple_singleline(item.title.clone(), font, pal().text);
                job.wrap.max_width = r.right() - x - 8.0;
                job.wrap.max_rows = 1;
                job.wrap.overflow_character = Some('…');
                f.layout_job(job)
            });
            ui.painter().galley(pos2(x, r.center().y - galley.size().y / 2.0), galley, pal().text);
            if resp.clicked() {
                app.preview_scroll_to = Some((item.page, item.y_pt));
                // Jump in the editor to a line that mentions the heading.
                if let Some(doc) = app.doc.as_ref() {
                    if let Some(line) = doc.text.lines().position(|l| l.contains(item.title.trim()) || (l.trim_start().starts_with('=') && l.contains(item.title.split_whitespace().next().unwrap_or("")))) {
                        app.jump_to = Some((line, 0));
                    }
                }
            }
        }
    });
}

fn segmented(ui: &mut Ui, options: &[&str], selected: &mut usize) {
    ui.horizontal(|ui| {
        let w = 108.0 * options.len() as f32;
        ui.add_space(((ui.available_width() - w) / 2.0).max(0.0));
        let (rect, _) = ui.allocate_exact_size(vec2(w, 30.0), Sense::hover());
        ui.painter().rect_filled(rect, 8.0, pal().surface);
        for (i, o) in options.iter().enumerate() {
            let r = Rect::from_min_size(pos2(rect.left() + i as f32 * 108.0, rect.top()), vec2(108.0, 30.0)).shrink(2.0);
            let resp = ui.interact(r, ui.id().with(("seg", i, *o)), Sense::click());
            if *selected == i {
                ui.painter().rect_filled(r, 7.0, pal().selected);
            }
            ui.painter().text(r.center(), Align2::CENTER_CENTER, *o, FontId::proportional(14.5), pal().text);
            if resp.clicked() {
                *selected = i;
            }
        }
    });
}

fn symbol_cell(ui: &mut Ui, glyph: &str, name: &str) -> bool {
    let (rect, resp) = ui.allocate_exact_size(vec2(52.0, 52.0), Sense::click());
    let bg = if resp.hovered() { pal().surface_hover } else { pal().surface };
    ui.painter().rect_filled(rect.shrink(2.0), 8.0, bg);
    ui.painter().text(rect.center(), Align2::CENTER_CENTER, glyph, FontId::proportional(22.0), pal().text);
    let resp = resp.on_hover_text(format!("{glyph}  {name}"));
    resp.clicked()
}

fn symbols_tab(app: &mut App, ui: &mut Ui) {
    let mut mode = app.symbol_draw as usize;
    segmented(ui, &["Text", "Zeichnen"], &mut mode);
    app.symbol_draw = mode == 1;
    ui.add_space(8.0);
    let mut chosen: Option<String> = None;
    if !app.symbol_draw {
        let mut q = std::mem::take(&mut app.symbol_query);
        search_field(ui, &mut q, "Symbole suchen", "symq");
        app.symbol_query = q;
        ui.add_space(6.0);
        let ql = app.symbol_query.to_lowercase();
        let list: Vec<&crate::symbols::Sym> = crate::symbols::all()
            .iter()
            .filter(|s| ql.is_empty() || s.name.to_lowercase().contains(&ql) || s.glyph == app.symbol_query)
            .collect();
        let cols = 4;
        let rows = list.len().div_ceil(cols);
        egui::ScrollArea::vertical().auto_shrink(false).show_rows(ui, 58.0, rows, |ui, range| {
            for r in range {
                ui.horizontal(|ui| {
                    ui.add_space(12.0);
                    ui.spacing_mut().item_spacing.x = 6.0;
                    for s in list.iter().skip(r * cols).take(cols) {
                        if symbol_cell(ui, s.glyph, &s.name) {
                            chosen = Some(s.name.clone());
                        }
                    }
                });
            }
        });
    } else {
        ui.horizontal(|ui| {
            ui.add_space(12.0);
            let size = vec2(ui.available_width() - 12.0, 220.0);
            let (rect, resp) = ui.allocate_exact_size(size, Sense::drag());
            ui.painter().rect_filled(rect, 10.0, pal().surface);
            ui.painter().rect_stroke(rect, 10.0, Stroke::new(1.0_f32, pal().separator), StrokeKind::Inside);
            if resp.drag_started() {
                app.strokes.push(Vec::new());
            }
            if resp.dragged() {
                if let (Some(p), Some(s)) = (resp.interact_pointer_pos(), app.strokes.last_mut()) {
                    let p = (p - rect.min).to_pos2();
                    if s.last().map(|l| l.distance(p) > 1.5).unwrap_or(true) {
                        s.push(p);
                    }
                }
            }
            for s in &app.strokes {
                let pts: Vec<Pos2> = s.iter().map(|p| rect.min + p.to_vec2()).collect();
                ui.painter().add(Shape::line(pts, Stroke::new(2.5_f32, pal().text)));
            }
            if app.strokes.is_empty() {
                ui.painter().text(rect.center(), Align2::CENTER_CENTER, "Symbol hier zeichnen", FontId::proportional(13.0), pal().dim);
            }
        });
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.add_space(12.0);
            if ui.button("Löschen").clicked() {
                app.strokes.clear();
            }
        });
        ui.add_space(6.0);
        let found = crate::symbols::recognize(&app.strokes);
        for chunk in found.chunks(4) {
            ui.horizontal(|ui| {
                ui.add_space(12.0);
                ui.spacing_mut().item_spacing.x = 6.0;
                for (name, glyph) in chunk {
                    if symbol_cell(ui, glyph, name) {
                        chosen = Some(name.to_string());
                    }
                }
            });
        }
    }
    if let Some(name) = chosen {
        // In math mode insert the bare name, otherwise the #sym form.
        let in_math = app
            .doc
            .as_ref()
            .and_then(|d| {
                let ctx = ui.ctx();
                egui::TextEdit::load_state(ctx, editor_id()).and_then(|s| s.cursor.char_range()).map(|r| {
                    let ci = r.primary.index;
                    d.text.chars().take(ci).filter(|c| *c == '$').count() % 2 == 1
                })
            })
            .unwrap_or(false);
        let text = if in_math { format!("{name} ") } else { format!("#sym.{name}") };
        app.pending_op = Some(EditOp::Insert(text));
    }
}

fn problems_tab(app: &mut App, ui: &mut Ui) {
    let diags: Vec<_> = app.visible_diags().cloned().collect();
    if diags.is_empty() {
        let r = ui.max_rect();
        let c = pos2(r.center().x, r.top() + r.height() * 0.38);
        paint(ui.painter(), Rect::from_center_size(c, vec2(30.0, 30.0)), Icon::Check, pal().faint);
        ui.painter().text(c + vec2(0.0, 34.0), Align2::CENTER_CENTER, "Keine Probleme.", FontId::proportional(14.0), pal().dim);
        return;
    }
    let mut jump = None;
    egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
        let mut files: Vec<String> = Vec::new();
        for d in &diags {
            if !files.contains(&d.file) {
                files.push(d.file.clone());
            }
        }
        for f in files {
            let items: Vec<_> = diags.iter().filter(|d| d.file == f).collect();
            let (r, _) = row(ui, 30.0, false);
            paint(ui.painter(), Rect::from_center_size(pos2(r.left() + 8.0, r.center().y), vec2(9.0, 9.0)), Icon::ChevronDown, pal().dim);
            file_icon(ui, Rect::from_center_size(pos2(r.left() + 28.0, r.center().y), vec2(17.0, 17.0)), &f, false);
            let name = if f.is_empty() { "Allgemein".to_string() } else { f.clone() };
            ui.painter().text(pos2(r.left() + 44.0, r.center().y), Align2::LEFT_CENTER, name, bold(14.5), pal().text);
            let badge = Rect::from_center_size(pos2(r.right() - 16.0, r.center().y), vec2(22.0, 18.0));
            ui.painter().rect_filled(badge, 9.0, pal().badge);
            let errs = items.iter().any(|d| d.error);
            ui.painter().text(badge.center(), Align2::CENTER_CENTER, items.len().to_string(), FontId::proportional(11.0), if errs { pal().error } else { pal().dim });
            for d in items {
                let h = 28.0 + d.hints.len() as f32 * 16.0;
                let (r, resp) = row(ui, h, false);
                let c = pos2(r.left() + 36.0, r.top() + 14.0);
                if d.error {
                    ui.painter().circle_filled(c, 7.0, pal().error);
                    paint(ui.painter(), Rect::from_center_size(c, vec2(8.0, 8.0)), Icon::Close, Color32::WHITE);
                } else {
                    paint(ui.painter(), Rect::from_center_size(c, vec2(15.0, 15.0)), Icon::Warning, pal().warning);
                }
                let galley = ui.fonts_mut(|fo| {
                    let mut job = egui::text::LayoutJob::simple_singleline(d.message.clone(), FontId::proportional(14.0), pal().text);
                    job.wrap.max_width = r.right() - c.x - 20.0;
                    job.wrap.max_rows = 1;
                    job.wrap.overflow_character = Some('…');
                    fo.layout_job(job)
                });
                ui.painter().galley(pos2(c.x + 14.0, c.y - galley.size().y / 2.0), galley, pal().text);
                for (i, hint) in d.hints.iter().enumerate() {
                    ui.painter().text(
                        pos2(c.x + 14.0, c.y + 16.0 + i as f32 * 16.0),
                        Align2::LEFT_CENTER,
                        format!("Hinweis: {hint}"),
                        FontId::proportional(11.5),
                        pal().dim,
                    );
                }
                let resp = resp.on_hover_text(match d.line {
                    Some(l) => format!("{} – Zeile {}", d.message, l + 1),
                    None => d.message.clone(),
                });
                if resp.clicked() {
                    if let Some(l) = d.line {
                        jump = Some((d.file.clone(), l, d.col.unwrap_or(0)));
                    }
                }
            }
        }
    });
    if let Some((f, l, c)) = jump {
        go_to(app, &f, l, c);
    }
}

// ------------------------------------------------------------------ editor

/// Text on the system clipboard (for the right-click "Einfügen").
fn clipboard_text() -> Option<String> {
    arboard::Clipboard::new().ok()?.get_text().ok()
}

pub const TAB_BAR_H: f32 = 32.0;

/// The open files as tabs above the editor: click to switch, × or middle-click to close,
/// drag to reorder, right-click for more.
fn tab_bar(app: &mut App, ui: &mut Ui, editor_bg: Color32) {
    let rect = ui.max_rect();
    ui.painter().rect_filled(rect, 0.0, pal().topbar);
    ui.painter().line_segment(
        [pos2(rect.left(), rect.bottom() - 0.5), pos2(rect.right(), rect.bottom() - 0.5)],
        Stroke::new(1.0_f32, pal().separator),
    );
    let active = app.doc.as_ref().map(|d| d.rel.clone());
    let dirty = app.doc.as_ref().map(|d| d.text != d.saved).unwrap_or(false);
    let main_rel = app.project.as_ref().map(|p| p.main_rel.clone()).unwrap_or_default();
    let order = app.tab_order.clone();
    let mut activate: Option<String> = None;
    let mut close: Option<String> = None;
    let mut swap: Option<(usize, usize)> = None;
    egui::ScrollArea::horizontal()
        .id_salt("editor-tabs")
        .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                let mut rects = Vec::new();
                for (i, rel) in order.iter().enumerate() {
                    let name = rel.rsplit('/').next().unwrap_or(rel).to_string();
                    let is_active = active.as_deref() == Some(rel.as_str());
                    let font = if is_active { bold(13.0) } else { FontId::proportional(13.0) };
                    let galley = ui.fonts_mut(|f| f.layout_no_wrap(name.clone(), font, pal().text));
                    let w = (galley.size().x + 62.0).clamp(110.0, 240.0);
                    let (r, resp) = ui.allocate_exact_size(vec2(w, rect.height()), Sense::click_and_drag());
                    rects.push(r);
                    let resp = resp.on_hover_text(rel.as_str());
                    if is_active {
                        ui.painter().rect_filled(r.with_max_y(rect.bottom()), 0.0, editor_bg);
                        ui.painter().line_segment([pos2(r.left(), r.top()), pos2(r.right(), r.top())], Stroke::new(2.0_f32, pal().accent));
                    } else if resp.hovered() {
                        ui.painter().rect_filled(r.shrink2(vec2(0.0, 0.0)).with_max_y(rect.bottom() - 1.0), 0.0, pal().hover);
                    }
                    ui.painter().line_segment(
                        [pos2(r.right() - 0.5, r.top() + 7.0), pos2(r.right() - 0.5, r.bottom() - 7.0)],
                        Stroke::new(1.0_f32, pal().separator),
                    );
                    file_icon(ui, Rect::from_center_size(pos2(r.left() + 16.0, r.center().y), vec2(14.0, 14.0)), &name, false);
                    let text_col = if is_active { pal().text } else { pal().dim };
                    let max_text = r.width() - 56.0;
                    let galley = ui.fonts_mut(|f| {
                        let mut job = egui::text::LayoutJob::simple_singleline(
                            name.clone(),
                            if is_active { bold(13.0) } else { FontId::proportional(13.0) },
                            text_col,
                        );
                        job.wrap.max_width = max_text;
                        job.wrap.max_rows = 1;
                        job.wrap.overflow_character = Some('…');
                        f.layout_job(job)
                    });
                    ui.painter().galley(pos2(r.left() + 30.0, r.center().y - galley.size().y / 2.0), galley, text_col);
                    if *rel == main_rel {
                        ui.painter().circle_filled(pos2(r.left() + 23.0, r.center().y - 6.0), 2.0, pal().accent);
                    }
                    // Close button; an unsaved tab shows a dot there until hovered.
                    let c = Rect::from_center_size(pos2(r.right() - 16.0, r.center().y), vec2(18.0, 18.0));
                    let close_resp = ui.interact(c, ui.id().with(("tab-close", rel)), Sense::click()).on_hover_text("Tab schließen");
                    let show_dot = is_active && dirty && !resp.hovered() && !close_resp.hovered();
                    if show_dot {
                        ui.painter().circle_filled(c.center(), 4.0, pal().text_soft);
                    } else if resp.hovered() || close_resp.hovered() || is_active {
                        if close_resp.hovered() {
                            ui.painter().rect_filled(c, 4.0, pal().surface_hover);
                        }
                        paint(ui.painter(), c.shrink(5.0), Icon::Close, pal().text_soft);
                    }
                    if close_resp.clicked() || resp.middle_clicked() {
                        close = Some(rel.clone());
                    } else if resp.clicked() || resp.drag_started() {
                        activate = Some(rel.clone());
                    }
                    if resp.secondary_clicked() {
                        let is_typ = rel.ends_with(".typ");
                        let items = vec![
                            PopupItem::new("Tab schließen").shortcut("CmdOrCtrl+W"),
                            PopupItem::new("Andere Tabs schließen").enabled(order.len() > 1),
                            PopupItem::new("Tabs rechts schließen").enabled(i + 1 < order.len()),
                            PopupItem::new("Als Hauptdatei festlegen").enabled(is_typ).checked(*rel == main_rel).sep(),
                            PopupItem::new(if cfg!(target_os = "macos") { "Im Finder zeigen" } else { "Im Dateimanager zeigen" }),
                        ];
                        let at = resp.interact_pointer_pos().unwrap_or(r.left_bottom());
                        open_popup(ui.ctx(), "tab-menu", rel.clone(), items, at);
                    }
                    // Drag a tab sideways to move it.
                    if resp.dragged() {
                        if let Some(p) = resp.interact_pointer_pos() {
                            if i > 0 && p.x < rects[i - 1].center().x {
                                swap = Some((i, i - 1));
                            }
                        }
                    }
                    if resp.dragged() {
                        if let Some(p) = resp.interact_pointer_pos() {
                            if p.x > r.right() + 30.0 && i + 1 < order.len() {
                                swap = Some((i, i + 1));
                            }
                        }
                    }
                }
            });
        });
    if let Some((choice, rel)) = popup_choice("tab-menu") {
        match choice {
            0 => close = Some(rel),
            1 => {
                for other in app.tab_order.clone().into_iter().filter(|r| *r != rel) {
                    app.close_tab(&other);
                }
                app.open_file(&rel);
            }
            2 => {
                if let Some(pos) = app.tab_order.iter().position(|r| *r == rel) {
                    for other in app.tab_order.clone().into_iter().skip(pos + 1) {
                        app.close_tab(&other);
                    }
                }
            }
            3 => {
                if let Some(p) = app.project.as_mut() {
                    p.main_rel = rel;
                }
                app.schedule_compile(Duration::ZERO);
            }
            _ => {
                if let Some(p) = app.project.as_ref() {
                    reveal_in_file_manager(&p.root.join(&rel));
                }
            }
        }
    }
    if let Some((a, b)) = swap {
        app.tab_order.swap(a, b);
    }
    if let Some(rel) = close {
        app.close_tab(&rel);
    } else if let Some(rel) = activate {
        app.open_file(&rel);
    }
}

pub fn editor_id() -> egui::Id {
    egui::Id::new("typisch-editor")
}

fn char_to_byte(s: &str, ci: usize) -> usize {
    s.char_indices().nth(ci).map(|(b, _)| b).unwrap_or(s.len())
}

fn line_col_to_char(s: &str, line: usize, col: usize) -> usize {
    let mut ci = 0;
    for (i, l) in s.split('\n').enumerate() {
        if i == line {
            return ci + col.min(l.chars().count());
        }
        ci += l.chars().count() + 1;
    }
    s.chars().count()
}

fn apply_op(text: &mut String, op: EditOp, range: (usize, usize)) -> (usize, usize) {
    let (a, b) = (range.0.min(range.1), range.0.max(range.1));
    let ba = char_to_byte(text, a);
    let bb = char_to_byte(text, b);
    match op {
        EditOp::Wrap(pre, post) => {
            text.insert_str(bb, &post);
            text.insert_str(ba, &pre);
            let pl = pre.chars().count();
            (a + pl, b + pl)
        }
        EditOp::Insert(s) => {
            text.replace_range(ba..bb, &s);
            let e = a + s.chars().count();
            (e, e)
        }
        EditOp::LinePrefix(p) => {
            // Prefix every selected line.
            let start_line_b = text[..ba].rfind('\n').map(|i| i + 1).unwrap_or(0);
            let seg = text[start_line_b..bb.max(start_line_b)].to_string();
            let new: String = seg.split('\n').map(|l| format!("{p}{l}")).collect::<Vec<_>>().join("\n");
            let added = new.chars().count() - seg.chars().count();
            text.replace_range(start_line_b..bb.max(start_line_b), &new);
            (a + p.chars().count(), b + added)
        }
    }
}

thread_local! {
    static JOB_CACHE: std::cell::RefCell<Option<(u64, egui::text::LayoutJob)>> = const { std::cell::RefCell::new(None) };
}

fn cached_job(text: &str, theme_idx: usize, size: f32, wrap: f32, line_h: f32) -> egui::text::LayoutJob {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    text.hash(&mut h);
    theme_idx.hash(&mut h);
    size.to_bits().hash(&mut h);
    wrap.to_bits().hash(&mut h);
    let key = h.finish();
    JOB_CACHE.with(|c| {
        if let Some((k, job)) = c.borrow().as_ref() {
            if *k == key {
                return job.clone();
            }
        }
        let theme = &crate::theme::THEMES[theme_idx.min(crate::theme::THEMES.len() - 1)];
        let job = crate::highlight::layout_job(text, theme, FontId::monospace(size), wrap, line_h);
        *c.borrow_mut() = Some((key, job.clone()));
        job
    })
}

fn editor_pane(app: &mut App, ui: &mut Ui) {
    let theme = *app.settings.theme();
    let full = ui.max_rect();
    let bottom_h = 30.0;
    let tabs_h = TAB_BAR_H;
    let tabs_rect = Rect::from_min_size(full.min, vec2(full.width(), tabs_h));
    let edit_rect = Rect::from_min_max(pos2(full.min.x, full.min.y + tabs_h), pos2(full.max.x, full.max.y - bottom_h));
    let bar_rect = Rect::from_min_max(pos2(full.min.x, full.max.y - bottom_h), full.max);
    ui.painter().rect_filled(edit_rect, 0.0, theme.bg);
    ui.painter().rect_filled(bar_rect, 0.0, pal().bottombar);

    ui.scope_builder(UiBuilder::new().max_rect(tabs_rect), |ui| tab_bar(app, ui, theme.bg));
    ui.scope_builder(UiBuilder::new().max_rect(bar_rect), |ui| bottom_bar(app, ui, theme.dark));

    let Some(doc) = app.doc.as_mut() else {
        app.editor_shown_rel = None;
        ui.painter().text(edit_rect.center(), Align2::CENTER_CENTER, "Keine Datei geöffnet", FontId::proportional(15.0), pal().dim);
        return;
    };

    // Switching tabs: bring back this tab's own cursor, selection and undo history.
    if app.editor_shown_rel.as_deref() != Some(doc.rel.as_str()) {
        doc.state.clone().unwrap_or_default().store(ui.ctx(), editor_id());
        app.editor_shown_rel = Some(doc.rel.clone());
    }

    let size = app.settings.font_size;
    let line_h = (size * 1.52).round();
    let font = FontId::monospace(size);
    let id = editor_id();
    let line_count = doc.text.split('\n').count();
    let digits = line_count.to_string().len().max(2) as f32;
    let char_w = ui.fonts_mut(|f| f.glyph_width(&font, '0'));
    let gutter_w = if app.settings.line_numbers { digits * char_w + 22.0 } else { 12.0 };

    // Apply pending edits (format buttons, symbols) and jumps before the widget runs.
    let mut state = egui::TextEdit::load_state(ui.ctx(), id).unwrap_or_default();
    let cur = state.cursor.char_range().map(|r| (r.secondary.index, r.primary.index)).unwrap_or((0, 0));
    let mut state_changed = false;
    let mut text_changed = false;
    if let Some(op) = app.pending_op.take() {
        let (a, b) = apply_op(&mut doc.text, op, cur);
        state.cursor.set_char_range(Some(egui::text::CCursorRange::two(egui::text::CCursor::new(a), egui::text::CCursor::new(b))));
        state_changed = true;
        text_changed = true;
        ui.ctx().memory_mut(|m| m.request_focus(id));
    }
    let mut scroll_to_line = None;
    if let Some((line, col)) = app.jump_to.take() {
        let ci = line_col_to_char(&doc.text, line, col);
        state.cursor.set_char_range(Some(egui::text::CCursorRange::one(egui::text::CCursor::new(ci))));
        state_changed = true;
        scroll_to_line = Some(line);
        ui.ctx().memory_mut(|m| m.request_focus(id));
    }
    if state_changed {
        state.store(ui.ctx(), id);
    }

    // Keyboard helpers while the editor has focus.
    let focused = ui.ctx().memory(|m| m.has_focus(id));
    let mut typed: Option<String> = None;
    if focused {
        let (b, i, u, tab) = ui.input_mut(|inp| {
            (
                inp.consume_key(egui::Modifiers::COMMAND, egui::Key::B),
                inp.consume_key(egui::Modifiers::COMMAND, egui::Key::I),
                inp.consume_key(egui::Modifiers::COMMAND, egui::Key::U),
                inp.consume_key(egui::Modifiers::NONE, egui::Key::Tab),
            )
        });
        if b {
            app.pending_op = Some(EditOp::Wrap("*".into(), "*".into()));
        } else if i {
            app.pending_op = Some(EditOp::Wrap("_".into(), "_".into()));
        } else if u {
            app.pending_op = Some(EditOp::Wrap("#underline[".into(), "]".into()));
        } else if tab {
            app.pending_op = Some(EditOp::Insert("  ".into()));
        }
        typed = ui.input(|inp| {
            inp.events.iter().find_map(|e| match e {
                egui::Event::Text(t) if t.chars().count() == 1 => Some(t.clone()),
                _ => None,
            })
        });
    }

    let editor_bg = theme.bg;
    let mut out_galley = None;
    ui.scope_builder(UiBuilder::new().max_rect(edit_rect), |ui| {
        // Each tab keeps its own scroll position.
        let mut scroll = egui::ScrollArea::vertical().id_salt(("editor-scroll", doc.rel.as_str())).auto_shrink(false);
        if let Some(line) = scroll_to_line {
            if let Some(y) = app.row_y.get(line) {
                scroll = scroll.vertical_scroll_offset((y - 120.0).max(0.0));
            }
        }
        scroll.show(ui, |ui| {
            let content_top = ui.cursor().top();
            let bg_idx = ui.painter().add(Shape::Noop);
            let gutter_idx = ui.painter().add(Shape::Noop);
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                ui.add_space(gutter_w);
                let theme_idx = crate::theme::current_theme_index().unwrap_or(app.settings.theme);
                let mut layouter = |ui: &Ui, buf: &dyn egui::TextBuffer, wrap: f32| {
                    let job = cached_job(buf.as_str(), theme_idx, size, wrap, line_h);
                    ui.fonts_mut(|f| f.layout_job(job))
                };
                let mut te = egui::TextEdit::multiline(&mut doc.text)
                    .id(id)
                    .font(font.clone())
                    .frame(false)
                    .lock_focus(true)
                    .desired_width(f32::INFINITY)
                    .desired_rows(1)
                    .margin(egui::Margin { left: 6, right: 14, top: 8, bottom: 8 })
                    .background_color(editor_bg)
                    .layouter(&mut layouter);
                if !app.settings.animated_cursor {
                    te = te.cursor_at_end(false);
                }
                let out = te.show(ui);
                let changed = out.response.changed();
                // Right-click menu, as in any desktop text editor (a system menu on macOS).
                let has_selection = out.cursor_range.map(|r| r.primary.index != r.secondary.index).unwrap_or(false);
                if out.response.secondary_clicked() {
                    let items = vec![
                        PopupItem::new("Ausschneiden").enabled(has_selection).shortcut("CmdOrCtrl+X"),
                        PopupItem::new("Kopieren").enabled(has_selection).shortcut("CmdOrCtrl+C"),
                        PopupItem::new("Einfügen").shortcut("CmdOrCtrl+V"),
                        PopupItem::new("Alles auswählen").shortcut("CmdOrCtrl+A"),
                        PopupItem::new("Fett").shortcut("CmdOrCtrl+B").sep(),
                        PopupItem::new("Kursiv").shortcut("CmdOrCtrl+I"),
                        PopupItem::new("Unterstrichen").shortcut("CmdOrCtrl+U"),
                        PopupItem::new("Formel"),
                    ];
                    let at = out.response.interact_pointer_pos().unwrap_or(out.response.rect.left_top());
                    open_popup(ui.ctx(), "editor-menu", "", items, at);
                }
                if let Some((choice, _)) = popup_choice("editor-menu") {
                    use crate::app::MenuAction as A;
                    match choice {
                        0 => app.pending_edit = Some(A::Cut),
                        1 => app.pending_edit = Some(A::Copy),
                        2 => app.pending_edit = Some(A::Paste),
                        3 => app.pending_edit = Some(A::SelectAll),
                        4 => app.pending_op = Some(EditOp::Wrap("*".into(), "*".into())),
                        5 => app.pending_op = Some(EditOp::Wrap("_".into(), "_".into())),
                        6 => app.pending_op = Some(EditOp::Wrap("#underline[".into(), "]".into())),
                        _ => app.pending_op = Some(EditOp::Wrap("$ ".into(), " $".into())),
                    }
                    ui.ctx().memory_mut(|m| m.request_focus(id));
                    ui.ctx().request_repaint();
                }
                // Auto-close brackets.
                if changed && app.settings.auto_close != crate::theme::AutoClose::Never {
                    if let (Some(t), Some(r)) = (typed.as_deref(), out.cursor_range) {
                        let ci = r.primary.index;
                        let close = match t {
                            "(" => Some(")"),
                            "[" => Some("]"),
                            "{" => Some("}"),
                            "\"" => Some("\""),
                            "$" => Some("$"),
                            _ => None,
                        };
                        let next = doc.text.chars().nth(ci);
                        let ok_ctx = app.settings.auto_close == crate::theme::AutoClose::Immediately
                            || next.map(|c| c.is_whitespace()).unwrap_or(true);
                        if let Some(c) = close {
                            if r.primary.index == r.secondary.index && ok_ctx && !(t == "\"" && next == Some('"')) {
                                let b = char_to_byte(&doc.text, ci);
                                doc.text.insert_str(b, c);
                            } else if t == "\"" && next == Some('"') {
                                let b = char_to_byte(&doc.text, ci);
                                doc.text.remove(b);
                            }
                        } else if [")", "]", "}"].contains(&t) && next.map(|n| n.to_string() == t).unwrap_or(false) {
                            let b = char_to_byte(&doc.text, ci);
                            doc.text.remove(b);
                        }
                    }
                }
                if changed || text_changed {
                    doc.edited_at = Some(Instant::now());
                }
                out_galley = Some((out.galley.clone(), out.galley_pos, out.cursor_range, out.response.rect));
                doc.state = Some(out.state.clone());
            });

            // Paint gutter, current line and error lines.
            if let Some((galley, gpos, cursor, resp_rect)) = out_galley.clone() {
                let text = &doc.text;
                let cursor_ci = cursor.map(|c| c.primary.index).unwrap_or(0);
                let cursor_line = text.chars().take(cursor_ci).filter(|c| *c == '\n').count();
                app.cursor_line = cursor_line;
                let errors: Vec<(usize, String)> = app
                    .diags
                    .iter()
                    .filter(|d| d.error && d.file == doc.rel)
                    .filter_map(|d| d.line.map(|l| (l, d.message.clone())))
                    .collect();
                let mut bg = Vec::new();
                let mut gutter = Vec::new();
                let left = ui.max_rect().left();
                let right = resp_rect.right().max(ui.max_rect().right());
                let mut line = 0usize;
                let mut new_line = true;
                app.row_y.clear();
                let has_focus = ui.ctx().memory(|m| m.has_focus(id));
                for prow in galley.rows.iter() {
                    let rr = prow.rect().translate(gpos.to_vec2());
                    let row_rect = Rect::from_min_max(pos2(left, rr.top()), pos2(right, rr.bottom()));
                    let err = errors.iter().find(|(l, _)| *l == line);
                    if err.is_some() {
                        bg.push(Shape::rect_filled(row_rect, 0.0, pal().error_line));
                    } else if line == cursor_line && has_focus {
                        bg.push(Shape::rect_filled(row_rect.shrink2(vec2(2.0, 0.0)), 3.0, theme.current_line));
                    }
                    if new_line {
                        app.row_y.push(rr.top() - content_top);
                        if app.settings.line_numbers {
                            let num = if app.settings.relative_line_numbers && line != cursor_line {
                                line.abs_diff(cursor_line)
                            } else {
                                line + 1
                            };
                            let col = if line == cursor_line { theme.line_number_current } else { theme.line_number };
                            let fs = (size * 0.86).round();
                            let galley_n = ui.fonts_mut(|f| f.layout_no_wrap(num.to_string(), FontId::monospace(fs), col));
                            let p = pos2(left + gutter_w - 8.0 - galley_n.size().x, rr.center().y.min(rr.top() + line_h / 2.0) - galley_n.size().y / 2.0);
                            gutter.push(Shape::galley(p, galley_n, col));
                        }
                        if let Some((_, msg)) = err {
                            let g = ui.fonts_mut(|f| f.layout_no_wrap(msg.clone(), bold(12.5), Color32::WHITE));
                            let w = g.size().x + 22.0;
                            let badge = Rect::from_min_size(pos2(right - w - 6.0, rr.top() + 2.0), vec2(w, (line_h - 4.0).max(18.0)));
                            bg.push(Shape::rect_filled(badge, 5.0, pal().error));
                            gutter.push(Shape::galley(pos2(badge.left() + 11.0, badge.center().y - g.size().y / 2.0), g, Color32::WHITE));
                        }
                    }
                    new_line = prow.row.ends_with_newline;
                    if new_line {
                        line += 1;
                    }
                }
                ui.painter().set(bg_idx, Shape::Vec(bg));
                ui.painter().set(gutter_idx, Shape::Vec(gutter));
            }
        });
    });

    if app.doc.as_ref().map(|d| d.edited_at.is_some()).unwrap_or(false) && out_galley.is_some() {
        // Typing: recompile soon.
        if app.compile_due.is_none() {
            app.schedule_compile(Duration::from_millis(220));
        }
    }
}

fn bottom_bar(app: &mut App, ui: &mut Ui, dark: bool) {
    let fg = pal().text_soft;
    let _ = dark;
    ui.horizontal_centered(|ui| {
        ui.add_space(12.0);
        ui.spacing_mut().item_spacing.x = 4.0;
        let fmt = |ui: &mut Ui, s: &str, font: FontId, italic: bool, underline: bool, tip: &str| -> bool {
            let (rect, resp) = ui.allocate_exact_size(vec2(28.0, 24.0), Sense::click());
            if resp.hovered() {
                ui.painter().rect_filled(rect, 5.0, pal().hover);
            }
            let mut job = egui::text::LayoutJob::default();
            job.append(
                s,
                0.0,
                egui::TextFormat {
                    font_id: font,
                    color: fg,
                    italics: italic,
                    underline: if underline { Stroke::new(1.2_f32, fg) } else { Stroke::NONE },
                    ..Default::default()
                },
            );
            let g = ui.fonts_mut(|f| f.layout_job(job));
            ui.painter().galley(rect.center() - g.size() / 2.0, g, fg);
            resp.on_hover_text(tip).clicked()
        };
        if fmt(ui, "B", bold(15.0), false, false, "Fett (Strg+B)") {
            app.pending_op = Some(EditOp::Wrap("*".into(), "*".into()));
        }
        if fmt(ui, "I", FontId::proportional(15.0), true, false, "Kursiv (Strg+I)") {
            app.pending_op = Some(EditOp::Wrap("_".into(), "_".into()));
        }
        if fmt(ui, "U", FontId::proportional(15.0), false, true, "Unterstrichen (Strg+U)") {
            app.pending_op = Some(EditOp::Wrap("#underline[".into(), "]".into()));
        }
        let r = ui.cursor();
        ui.add_space(6.0);
        ui.painter().line_segment([pos2(r.left() + 2.0, r.top() + 7.0), pos2(r.left() + 2.0, r.bottom() - 7.0)], Stroke::new(1.0_f32, pal().separator));
        for (icon, tip, op) in [
            (Icon::Columns, "Spalten", EditOp::Wrap("#columns(2)[\n".into(), "\n]".into())),
            (Icon::Bullets, "Aufzählung", EditOp::LinePrefix("- ".into())),
            (Icon::Numbers, "Nummerierte Liste", EditOp::LinePrefix("+ ".into())),
        ] {
            if icon_button(ui, icon, 28.0, fg, false, tip).clicked() {
                app.pending_op = Some(op);
            }
        }
        if fmt(ui, "∑", FontId::proportional(16.0), false, false, "Formel") {
            app.pending_op = Some(EditOp::Wrap("$ ".into(), " $".into()));
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.add_space(14.0);
            if app.settings.word_count {
                let n = app.words;
                ui.label(RichText::new(format!("{n} {}", if n == 1 { "Wort" } else { "Wörter" })).color(pal().dim).size(12.5));
            }
            ui.label(RichText::new(format!("Zeile {}", app.cursor_line + 1)).color(pal().faint).size(12.0));
        });
    });
}

// ------------------------------------------------------------------ preview

fn preview_pane(app: &mut App, ui: &mut Ui) {
    let full = ui.max_rect();
    ui.painter().rect_filled(full, 0.0, pal().preview_bg);
    let bar = Rect::from_min_size(full.min, vec2(full.width(), 28.5));
    let body = Rect::from_min_max(pos2(full.min.x, bar.bottom()), full.max);

    // Fit-to-width zoom.
    let max_w = app.pages.iter().map(|p| p.size_pt.x).fold(0.0, f32::max);
    if app.fit_width && max_w > 0.0 {
        let per_row = if app.two_pages { 2.0 } else { 1.0 };
        app.zoom = ((body.width() - 14.0 - 8.0 * (per_row - 1.0)) / (max_w * per_row)).clamp(0.2, 5.0);
    }

    // Pages
    let mut clicked_at = None;
    ui.scope_builder(UiBuilder::new().max_rect(body), |ui| {
        let mut scroll = egui::ScrollArea::both().id_salt("preview-scroll").auto_shrink(false);
        if let Some((page, y)) = app.preview_scroll_to.take() {
            let mut off = 0.0;
            for p in app.pages.iter().take(page.saturating_sub(1)) {
                off += p.size_pt.y * app.zoom + 8.0;
            }
            scroll = scroll.vertical_scroll_offset((off + y * app.zoom - 40.0).max(0.0));
        }
        // Pixels are rendered at the screen's real resolution, and only for pages on or near screen,
        // so long documents stay fast and light on memory.
        let scale = (app.zoom * ui.ctx().pixels_per_point()).clamp(0.5, 6.0);
        let many_pages = app.pages.len() > 8;
        let mut requests: Vec<(usize, u128, f32)> = Vec::new();
        let out = scroll.show(ui, |ui| {
            let zoom = app.zoom;
            let fit = app.fit_width;
            let width = ui.available_width();
            let view = ui.clip_rect();
            let near = view.expand2(vec2(0.0, view.height()));
            let keep = view.expand2(vec2(0.0, view.height() * 3.0));
            ui.spacing_mut().item_spacing.y = 0.0;
            let mut tops = Vec::new();
            let per_row = if app.two_pages { 2 } else { 1 };
            let mut index = 0;
            for chunk in app.pages.chunks_mut(per_row) {
                let row_w: f32 = chunk.iter().map(|p| p.size_pt.x * zoom).sum::<f32>() + 8.0 * (chunk.len() as f32 - 1.0);
                let row_h = chunk.iter().map(|p| p.size_pt.y * zoom).fold(0.0, f32::max);
                let (rect, resp) = ui.allocate_exact_size(vec2(if fit { width } else { width.max(row_w + 16.0) }, row_h), Sense::click());
                let centered = ((rect.width() - row_w) / 2.0).max(8.0);
                let mut x = rect.left() + if fit { 8.0 } else { centered };
                for p in chunk.iter_mut() {
                    let r = Rect::from_min_size(pos2(x, rect.top()), p.size_pt * zoom);
                    tops.push(r.top());
                    if r.intersects(view) {
                        match &p.tex {
                            Some((tex, _, _)) => {
                                ui.painter().image(tex.id(), r, Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0)), Color32::WHITE);
                            }
                            None => {
                                ui.painter().rect_filled(r, 0.0, Color32::WHITE);
                            }
                        }
                    }
                    if r.intersects(near) {
                        let fresh = p.tex.as_ref().map(|(_, h, s)| *h == p.hash && ((s / scale) - 1.0).abs() < 0.08).unwrap_or(false);
                        let asked = p.pending.map(|(h, s)| h == p.hash && ((s / scale) - 1.0).abs() < 0.08).unwrap_or(false);
                        if !fresh && !asked {
                            p.pending = Some((p.hash, scale));
                            requests.push((index, p.hash, scale));
                        }
                    } else if many_pages && !r.intersects(keep) {
                        // Far away: let go of the pixels; they come back when scrolled to.
                        p.tex = None;
                        p.pending = None;
                    }
                    index += 1;
                    x += r.width() + 8.0;
                }
                if resp.clicked() {
                    clicked_at = resp.interact_pointer_pos();
                }
                ui.add_space(8.0);
            }
            tops
        });
        if !requests.is_empty() {
            // Pages in view first, top to bottom.
            let mid = out.inner_rect.center().y;
            requests.sort_by_key(|(i, _, _)| (out.inner.get(*i).copied().unwrap_or(0.0) - mid).abs() as i64);
            app.compiler.send(crate::compiler::Job::Render { requests });
        }
        // Which page is in view?
        let mid = out.inner_rect.top() + out.inner_rect.height() * 0.3;
        let mut now = 1;
        for (i, t) in out.inner.iter().enumerate() {
            if *t <= mid {
                now = if app.two_pages { i + 1 } else { i + 1 };
            }
        }
        app.preview_page_now = now.min(app.pages.len().max(1));
    });

    if app.pages.is_empty() {
        let msg = if app.compiling || app.compile_due.is_some() || app.compiled_tag == 0 {
            "Vorschau wird erstellt …"
        } else {
            "Keine Vorschau – das Dokument enthält Fehler."
        };
        ui.painter().text(body.center(), Align2::CENTER_CENTER, msg, FontId::proportional(14.0), pal().dim);
    }

    // Ripple on click.
    if let Some(p) = clicked_at {
        if app.settings.ripple {
            app.ripple = Some((p, Instant::now()));
        }
    }
    if let Some((p, t)) = app.ripple {
        let k = t.elapsed().as_secs_f32() / 0.6;
        if k >= 1.0 {
            app.ripple = None;
        } else {
            let alpha = ((1.0 - k) * 160.0) as u8;
            let painter = ui.painter().with_clip_rect(body);
            painter.circle_stroke(p, 6.0 + k * 46.0, Stroke::new(2.5_f32, Color32::from_rgba_unmultiplied(pal().accent.r(), pal().accent.g(), pal().accent.b(), alpha)));
            painter.circle_filled(p, 6.0 + k * 30.0, Color32::from_rgba_unmultiplied(pal().accent.r(), pal().accent.g(), pal().accent.b(), alpha / 4));
            ui.ctx().request_repaint();
        }
    }

    // Toolbar (drawn last so it sits on top).
    {
        // Toolbar with a soft vertical gradient, like a translucent bar over the page.
        let mut mesh = egui::Mesh::default();
        let (top, bottom) = (pal().preview_bar_top, pal().preview_bar_bottom);
        mesh.colored_vertex(bar.left_top(), top);
        mesh.colored_vertex(bar.right_top(), top);
        mesh.colored_vertex(bar.right_bottom(), bottom);
        mesh.colored_vertex(bar.left_bottom(), bottom);
        mesh.add_triangle(0, 1, 2);
        mesh.add_triangle(0, 2, 3);
        ui.painter().add(Shape::mesh(mesh));
    }
    ui.scope_builder(UiBuilder::new().max_rect(bar.shrink2(vec2(10.0, 0.0))), |ui| {
        ui.horizontal_centered(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            let fg = pal().preview_fg;
            if icon_button(ui, Icon::ZoomOut, 28.0, fg, false, "Verkleinern").clicked() {
                app.fit_width = false;
                app.zoom = (app.zoom / 1.15).max(0.2);
            }
            // 100 % = real size on screen, whatever the interface size is.
            let pct = format!("{:.0}%", app.zoom * ui.ctx().zoom_factor() * 100.0);
            let resp = ui.add(egui::Label::new(RichText::new(pct).color(fg).size(13.5)).sense(Sense::click()));
            if resp.on_hover_text("Auf 100 % setzen").clicked() {
                app.fit_width = false;
                app.zoom = 1.0 / ui.ctx().zoom_factor();
            }
            if icon_button(ui, Icon::ZoomIn, 28.0, fg, false, "Vergrößern").clicked() {
                app.fit_width = false;
                app.zoom = (app.zoom * 1.15).min(5.0);
            }
            let fit_col = if app.fit_width { pal().faint } else { fg };
            if icon_button(ui, Icon::Fit, 28.0, fit_col, false, "An Breite anpassen").clicked() {
                app.fit_width = true;
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if icon_button(ui, Icon::Share, 28.0, fg, false, "Als PDF exportieren (Strg+E)").clicked() {
                    app.export_pdf();
                }
                ui.add_space(8.0);
                if icon_button(ui, Icon::Pages, 28.0, if app.two_pages { pal().accent } else { fg }, false, "Zwei Seiten nebeneinander").clicked() {
                    app.two_pages = !app.two_pages;
                }
            });
        });
    });
    let total = app.pages.len().max(1);
    ui.painter().text(
        bar.center(),
        Align2::CENTER_CENTER,
        format!("Seite {} von {}", app.preview_page_now, total),
        bold(13.5),
        pal().preview_fg,
    );
    let _ = CornerRadius::ZERO;
}
