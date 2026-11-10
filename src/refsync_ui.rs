//! The interface of the Zotero reference sync (see `refsync.rs`): the account in the settings,
//! the "Aus Zotero synchronisieren" dialog and the bar above a synced file in the editor.

use std::time::Duration;

use eframe::egui::{self, Align, Align2, Color32, FontId, Layout, Rect, RichText, Sense, Stroke, StrokeKind, Ui, UiBuilder, pos2, vec2};

use crate::app::{App, SettingsTab, bold};
use crate::icons::{Icon, paint};
use crate::refsync::{self, LibraryTree};
use crate::screens::{group, note, section, sep, toggle};
use crate::theme::{mix, pal};
use crate::widgets::{
    BAR_PAD, Button, CONTROL_H, RADIUS, RADIUS_L, RADIUS_XL, TEXT, TEXT_SMALL, TOOL_GAP, ToolButton, field, row_bg, secret_field,
};
use crate::zotero::{self, LibraryRef};

/// A small square with a "Z", standing for Zotero.
fn zotero_tile(ui: &mut Ui, size: f32) {
    let (r, _) = ui.allocate_exact_size(vec2(size, size), Sense::hover());
    ui.painter().rect_filled(r, RADIUS, pal().selected);
    ui.painter()
        .text(r.center(), Align2::CENTER_CENTER, "Z", bold(size * 0.62), pal().text);
}

fn error_text(ui: &mut Ui, text: &str) {
    ui.label(RichText::new(text).size(TEXT_SMALL).color(pal().error));
}

fn ok_color() -> Color32 {
    mix(pal().text, Color32::from_rgb(0x40, 0xb0, 0x40), 0.7)
}

// ------------------------------------------------------------------ settings

/// The "Konten" page of the settings.
pub fn accounts_tab(app: &mut App, ui: &mut Ui) {
    section(ui, "Verbindungen zu anderen Diensten");
    group(ui, |ui| {
        ui.horizontal(|ui| {
            ui.set_min_height(44.0);
            zotero_tile(ui, 28.0);
            ui.add_space(4.0);
            ui.label(RichText::new("Zotero").font(bold(15.5)).color(pal().text));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| match app.refsync.account() {
                Some(a) => {
                    ui.label(RichText::new(format!("Verbunden als {}", a.label())).size(14.0).color(ok_color()));
                }
                None => {
                    ui.label(RichText::new("Nicht verbunden").size(14.0).color(pal().dim));
                }
            });
        });
        sep(ui);
        if app.refsync.account().is_some() {
            connected_info(app, ui);
        } else {
            connect_form(app, ui, "settings", false);
        }
    });
    ui.add_space(12.0);
}

fn connected_info(app: &mut App, ui: &mut Ui) {
    let Some(account) = app.refsync.account().cloned() else { return };
    let files = app.refsync.store.files.len();
    note(
        ui,
        &format!(
            "Typisch liest die Bibliothek und die Gruppen von {} ({}) und hält damit Literaturdateien in deinen Projekten aktuell. Neue Dateien legst du in der Dateiliste über „Aus Zotero synchronisieren …“ an.",
            account.label(),
            account.username
        ),
    );
    match files {
        0 => {}
        1 => note(ui, "1 Datei wird aus Zotero synchronisiert."),
        n => note(ui, &format!("{n} Dateien werden aus Zotero synchronisiert.")),
    }
    ui.add_space(4.0);
    let mut disconnect = false;
    ui.horizontal(|ui| {
        // Not while a new key is being checked (that would undo the disconnect a moment later).
        if ui.add_enabled(!app.refsync.connecting, Button::new("Verbindung trennen")).clicked() {
            disconnect = true;
        }
        let label = if app.refsync.replacing_key {
            "Nicht ersetzen"
        } else {
            "Schlüssel ersetzen …"
        };
        if ui.add(Button::new(label)).clicked() {
            app.refsync.replacing_key = !app.refsync.replacing_key;
            if !app.refsync.replacing_key {
                app.refsync.cancel_connect();
            }
            app.refsync.connect_error = None;
        }
        if ui.add(Button::new("Schlüssel bei Zotero verwalten …")).clicked() {
            ui.ctx().open_url(egui::OpenUrl::new_tab(zotero::KEYS_URL));
        }
    });
    ui.add_space(4.0);
    if app.refsync.replacing_key {
        sep(ui);
        note(
            ui,
            "Ein neuer Schlüssel, etwa wenn der alte widerrufen wurde. Alle synchronisierten Dateien bleiben verbunden.",
        );
        connect_form(app, ui, "replace", false);
        sep(ui);
    }
    note(
        ui,
        "Beim Trennen wird der Schlüssel von diesem Rechner gelöscht, und alle synchronisierten Literaturdateien werden zu normalen Textdateien, die du selbst bearbeiten kannst.",
    );
    if disconnect {
        app.refsync.disconnect();
        app.toast("Die Verbindung zu Zotero wurde getrennt.", true);
    }
}

/// Explains how to get an API key and takes it. Shared by the settings and the dialog.
fn connect_form(app: &mut App, ui: &mut Ui, id: &str, focus: bool) {
    if app.refsync.account().is_none() {
        note(
            ui,
            "Verbinde dein Zotero-Konto, um Literaturverzeichnisse aus deiner Bibliothek, einer Sammlung oder einer Gruppe als BibLaTeX-Datei in deine Projekte zu übernehmen. Typisch hält sie danach automatisch aktuell.",
        );
    }
    ui.add_space(4.0);
    step(ui, "1", "Einen Schlüssel mit Lesezugriff bei Zotero erstellen:");
    ui.horizontal(|ui| {
        ui.add_space(26.0);
        if ui.add(Button::new("Schlüssel bei Zotero erstellen …")).clicked() {
            ui.ctx().open_url(egui::OpenUrl::new_tab(zotero::NEW_KEY_URL));
        }
    });
    ui.horizontal(|ui| {
        ui.add_space(26.0);
        ui.vertical(|ui| note(ui, "Die Seite öffnet sich im Browser, die Rechte (nur lesen) sind schon gesetzt. Unten auf „Save Key“ klicken und den angezeigten Schlüssel kopieren."));
    });
    ui.add_space(4.0);
    step(ui, "2", "Den Schlüssel hier einfügen:");
    let mut go = false;
    let row = vec2(ui.available_width(), CONTROL_H);
    ui.allocate_ui_with_layout(row, Layout::left_to_right(Align::Center), |ui| {
        ui.add_space(26.0);
        let can = !app.refsync.key_input.trim().is_empty() && !app.refsync.connecting;
        let w = (ui.available_width() - 112.0).max(120.0);
        // A new id after each use, so the field's undo history cannot bring a used key back.
        let generation = app.refsync.key_generation;
        let resp = secret_field(ui, &mut app.refsync.key_input, "API-Schlüssel", w, ("zotero-key", id, generation));
        if focus {
            resp.request_focus();
        }
        if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) && can {
            go = true;
        }
        ui.add_space(6.0);
        if ui.add_enabled(can, Button::primary("Verbinden")).clicked() {
            go = true;
        }
    });
    if go {
        app.refsync.connect();
    }
    ui.horizontal(|ui| {
        ui.add_space(26.0);
        if app.refsync.connecting {
            ui.add(egui::Spinner::new().size(13.0).color(pal().dim));
            ui.label(RichText::new("Wird bei Zotero geprüft …").size(TEXT_SMALL).color(pal().dim));
        } else if let Some(e) = app.refsync.connect_error.clone() {
            ui.vertical(|ui| error_text(ui, &e));
        }
    });
    ui.horizontal(|ui| {
        ui.add_space(26.0);
        let where_ = if cfg!(target_os = "macos") {
            "im Schlüsselbund von macOS"
        } else {
            "nur für dich lesbar in deinem Konfigurationsordner"
        };
        ui.vertical(|ui| {
            note(
                ui,
                &format!("Der Schlüssel wird {where_} gespeichert und nur an api.zotero.org gesendet."),
            )
        });
    });
}

fn step(ui: &mut Ui, n: &str, text: &str) {
    ui.horizontal(|ui| {
        let (r, _) = ui.allocate_exact_size(vec2(20.0, 20.0), Sense::hover());
        ui.painter().circle_filled(r.center(), 10.0, pal().selected);
        ui.painter().text(r.center(), Align2::CENTER_CENTER, n, bold(12.0), pal().text);
        ui.add_space(2.0);
        ui.label(RichText::new(text).size(14.0).color(pal().text));
    });
}

/// Opens the settings on the "Konten" page.
pub fn open_accounts(app: &mut App, ctx: &egui::Context) {
    app.settings_tab = SettingsTab::Accounts;
    if app.settings_open && app.screenshot.is_none() {
        ctx.send_viewport_cmd_to(egui::ViewportId::from_hash_of("typisch-settings"), egui::ViewportCommand::Focus);
    } else {
        app.settings_open = true;
        app.settings_pos = None;
    }
}

// ------------------------------------------------------------------ dialog

/// "Aus Zotero synchronisieren": file name and what to sync, like the Typst web app.
pub fn create_dialog(app: &mut App, ctx: &egui::Context) {
    let Some(root) = app.project.as_ref().map(|p| p.root.clone()) else {
        app.refsync.dialog = None;
        return;
    };
    if app.refsync.dialog.is_none() {
        return;
    }
    let screen = ctx.content_rect();
    let size = vec2(580.0, 620.0).min(screen.size() - vec2(32.0, 32.0)).max(vec2(320.0, 320.0));
    let outer = Rect::from_center_size(screen.center(), size);
    let mut close = ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
    let mut create = false;
    egui::Area::new(egui::Id::unique("zotero-dialog"))
        .order(egui::Order::Foreground)
        .fixed_pos(screen.min)
        .constrain(false)
        .show(ctx, |ui| {
            // Modal: keyboard focus (also with Tab) stays in the dialog, never in the editor behind.
            ui.ctx().memory_mut(|m| {
                m.set_modal_layer(ui.layer_id());
                m.surrender_focus(crate::ide::editor_id());
            });
            // The window behind is dimmed and does not take clicks while the dialog is open.
            ui.painter()
                .rect_filled(screen, 0.0, Color32::from_black_alpha(if pal().dark { 110 } else { 60 }));
            ui.interact(screen, ui.scope_id().with("zotero-dialog-shade"), Sense::click_and_drag());
            ui.painter().rect(
                outer,
                RADIUS_XL,
                pal().window,
                Stroke::new(1.0_f32, pal().separator),
                StrokeKind::Inside,
            );
            ui.interact(outer, ui.scope_id().with("zotero-dialog-bg"), Sense::click_and_drag());
            let inner = outer.shrink2(vec2(20.0, 12.0));
            ui.scope_builder(UiBuilder::new().max_rect(inner), |ui| {
                ui.set_clip_rect(outer);
                let (row, _) = ui.allocate_exact_size(vec2(inner.width(), 34.0), Sense::hover());
                ui.painter().text(
                    row.center(),
                    Align2::CENTER_CENTER,
                    "Aus Zotero synchronisieren",
                    FontId::proportional(17.0),
                    pal().text_soft,
                );
                let x = Rect::from_center_size(pos2(row.right() - 10.0, row.center().y), vec2(28.0, 28.0));
                if ui.put(x, ToolButton::new(Icon::Close, "Schließen (Esc)")).clicked() {
                    close = true;
                }
                ui.add_space(6.0);
                if app.refsync.account().is_none() {
                    // The key field takes the keyboard when the dialog opens.
                    let focus = app
                        .refsync
                        .dialog
                        .as_mut()
                        .map(|d| std::mem::take(&mut d.focus_name))
                        .unwrap_or(false);
                    egui::ScrollArea::vertical()
                        .auto_shrink(false)
                        .show(ui, |ui| connect_form(app, ui, "dialog", focus));
                } else {
                    create = dialog_body(app, ui, inner);
                }
            });
        });
    if create {
        match app.refsync.create(&root) {
            Ok(rel) => {
                let path = bibliography_path(app, &rel);
                app.refresh_files();
                app.open_file(&rel);
                app.schedule_compile(Duration::ZERO);
                app.toast(
                    format!("„{rel}“ wird aus Zotero synchronisiert. Einbinden mit #bibliography(\"{path}\")."),
                    true,
                );
            }
            Err(e) => {
                if let Some(d) = app.refsync.dialog.as_mut() {
                    d.error = Some(e);
                }
            }
        }
    }
    if close {
        app.refsync.dialog = None;
    }
}

/// How the main file refers to `rel`: as it is from the project folder, else from the
/// project root ("/literatur.bib"), which works from any subfolder.
fn bibliography_path(app: &App, rel: &str) -> String {
    let main_in_root = app.project.as_ref().map(|p| !p.main_rel.contains('/')).unwrap_or(true);
    if main_in_root { rel.to_string() } else { format!("/{rel}") }
}

/// The dialog's content once Zotero is connected. Returns true when "Erstellen" was chosen.
fn dialog_body(app: &mut App, ui: &mut Ui, inner: Rect) -> bool {
    let mut create = false;
    ui.label(RichText::new("Dateiname").size(TEXT).color(pal().text_soft));
    ui.add_space(2.0);
    let w = ui.available_width();
    let mut enter = false;
    if let Some(d) = app.refsync.dialog.as_mut() {
        let before = d.name.clone();
        let resp = field(ui, &mut d.name, "literatur.bib", w, Some(Icon::Document), "zotero-file-name");
        // Typing goes into the dialog straight away, not into the editor behind it.
        if d.focus_name {
            d.focus_name = false;
            resp.request_focus();
        }
        enter = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        // Enter is the default button also after the source was picked with the mouse.
        enter |= ui.ctx().memory(|m| m.focused().is_none()) && ui.input(|i| i.key_pressed(egui::Key::Enter));
        if d.name != before {
            d.error = None;
        }
    }
    let name = app.refsync.dialog.as_ref().map(|d| d.name.clone()).unwrap_or_default();
    let shown = bibliography_path(app, &refsync::clean_rel(&name).unwrap_or_else(|_| "literatur.bib".into()));
    ui.add_space(2.0);
    note(
        ui,
        &format!("Eine BibLaTeX-Datei im Projekt, schreibgeschützt und automatisch aktuell. Einbinden mit #bibliography(\"{shown}\")."),
    );
    ui.add_space(6.0);
    ui.label(RichText::new("Was soll synchronisiert werden?").size(TEXT).color(pal().text_soft));
    ui.add_space(4.0);

    // The list takes the room the footer leaves.
    let footer_h = 92.0;
    let list_h = (inner.bottom() - ui.cursor().top() - footer_h).max(120.0);
    let list = Rect::from_min_size(ui.cursor().min, vec2(ui.available_width(), list_h));
    ui.allocate_rect(list, Sense::hover());
    ui.painter().rect_filled(list, RADIUS_L, pal().group);
    let mut double_clicked = false;
    ui.scope_builder(UiBuilder::new().max_rect(list.shrink(6.0)), |ui| {
        double_clicked = source_list(app, ui);
    });

    ui.add_space(8.0);
    let selected_collection = app
        .refsync
        .dialog
        .as_ref()
        .and_then(|d| d.choice.clone())
        .and_then(|(lib, c)| c.map(|c| (lib, c)));
    let has_children = selected_collection
        .as_ref()
        .and_then(|(lib, key)| {
            let libs = app.refsync.libraries.as_ref()?.as_ref().ok()?;
            let tree = libs.iter().find(|t| t.library == *lib)?;
            tree.collections.iter().find(|c| c.key == *key).map(|c| c.has_children)
        })
        .unwrap_or(false);
    ui.horizontal(|ui| {
        ui.set_min_height(28.0);
        ui.add_enabled_ui(has_children, |ui| {
            if let Some(d) = app.refsync.dialog.as_mut() {
                let mut on = d.subcollections && has_children;
                if toggle(ui, &mut on) && has_children {
                    d.subcollections = on;
                }
            }
            let col = if has_children { pal().text } else { pal().faint };
            ui.label(RichText::new("Einträge aus Unter-Sammlungen einbeziehen").size(14.0).color(col));
        });
    });
    ui.add_space(6.0);
    let can_create = app
        .refsync
        .dialog
        .as_ref()
        .map(|d| d.choice.is_some() && refsync::clean_rel(&d.name).is_ok())
        .unwrap_or(false);
    ui.horizontal(|ui| {
        ui.set_min_height(CONTROL_H);
        if let Some(e) = app
            .refsync
            .dialog
            .as_ref()
            .and_then(|d| d.error.clone())
            .or_else(|| refsync::clean_rel(&name).err())
        {
            let w = (ui.available_width() - 300.0).max(80.0);
            ui.allocate_ui(vec2(w, CONTROL_H), |ui| {
                ui.add(egui::Label::new(RichText::new(e).size(TEXT_SMALL).color(pal().error)).wrap());
            });
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if ui.add_enabled(can_create, Button::primary("Erstellen & synchronisieren")).clicked() {
                create = true;
            }
            if ui.add(Button::new("Abbrechen")).clicked() {
                app.refsync.dialog = None;
            }
        });
    });
    create || ((double_clicked || enter) && can_create)
}

/// Libraries and their collections to choose from. Returns true on a double click.
fn source_list(app: &mut App, ui: &mut Ui) -> bool {
    let libs: Vec<LibraryTree> = match &app.refsync.libraries {
        Some(Ok(l)) => l.clone(),
        Some(Err(e)) => {
            let e = e.clone();
            ui.add_space(16.0);
            ui.vertical_centered(|ui| {
                ui.label(RichText::new(e).size(TEXT_SMALL).color(pal().error));
                ui.add_space(8.0);
                if ui.add(Button::new("Erneut versuchen")).clicked() {
                    app.refsync.libraries = None;
                    app.refsync.request_libraries();
                }
            });
            return false;
        }
        None => {
            if !app.refsync.loading_libraries {
                app.refsync.request_libraries();
            }
            ui.add_space(24.0);
            ui.vertical_centered(|ui| {
                ui.add(egui::Spinner::new().size(18.0).color(pal().dim));
                ui.add_space(6.0);
                ui.label(
                    RichText::new("Bibliotheken werden von Zotero geladen …")
                        .size(TEXT_SMALL)
                        .color(pal().dim),
                );
            });
            return false;
        }
    };
    let choice = app.refsync.dialog.as_ref().and_then(|d| d.choice.clone());
    let mut picked: Option<(LibraryRef, Option<String>)> = None;
    let mut double = false;
    egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 1.0;
        for (i, lib) in libs.iter().enumerate() {
            if i > 0 {
                ui.add_space(6.0);
            }
            let me = (lib.library, None);
            let usable = lib.error.is_none();
            let w = ui.available_width();
            let (r, resp) = ui.allocate_exact_size(vec2(w, 30.0), if usable { Sense::click() } else { Sense::hover() });
            row_bg(ui, r, &resp, choice.as_ref() == Some(&me));
            let icon_col = if usable { pal().accent } else { pal().faint };
            paint(
                ui.painter(),
                Rect::from_center_size(pos2(r.left() + 16.0, r.center().y), vec2(17.0, 17.0)),
                Icon::Folder,
                icon_col,
            );
            let text_col = if usable { pal().text } else { pal().dim };
            ui.painter().text(
                pos2(r.left() + 32.0, r.center().y),
                Align2::LEFT_CENTER,
                &lib.name,
                bold(14.0),
                text_col,
            );
            let right = match &lib.error {
                Some(e) => e.clone(),
                None => "ganze Bibliothek".to_string(),
            };
            ui.painter().text(
                pos2(r.right() - 10.0, r.center().y),
                Align2::RIGHT_CENTER,
                right,
                FontId::proportional(12.0),
                pal().dim,
            );
            if usable {
                if resp.clicked() {
                    picked = Some(me.clone());
                }
                if resp.double_clicked() {
                    picked = Some(me);
                    double = true;
                }
            }
            for c in &lib.collections {
                let me = (lib.library, Some(c.key.clone()));
                let (r, resp) = ui.allocate_exact_size(vec2(w, 26.0), Sense::click());
                row_bg(ui, r, &resp, choice.as_ref() == Some(&me));
                let x = r.left() + 32.0 + c.depth as f32 * 16.0;
                paint(
                    ui.painter(),
                    Rect::from_center_size(pos2(x, r.center().y), vec2(15.0, 15.0)),
                    Icon::Folder,
                    pal().dim,
                );
                let galley = ui.fonts_mut(|f| {
                    let mut job = egui::text::LayoutJob::simple_singleline(c.name.clone(), FontId::proportional(14.0), pal().text);
                    job.wrap.max_width = (r.right() - x - 70.0).max(40.0);
                    job.wrap.max_rows = 1;
                    job.wrap.overflow_character = Some('…');
                    f.layout_job(job)
                });
                ui.painter()
                    .galley(pos2(x + 14.0, r.center().y - galley.size().y / 2.0), galley, pal().text);
                let count_col = if choice.as_ref() == Some(&me) { pal().text_soft } else { pal().dim };
                ui.painter().text(
                    pos2(r.right() - 10.0, r.center().y),
                    Align2::RIGHT_CENTER,
                    c.items.to_string(),
                    FontId::proportional(12.0),
                    count_col,
                );
                let resp = resp.on_hover_text(format!(
                    "{} › {} – {} {}",
                    lib.name,
                    c.path,
                    c.items,
                    if c.items == 1 { "Eintrag" } else { "Einträge" }
                ));
                if resp.clicked() {
                    picked = Some(me.clone());
                }
                if resp.double_clicked() {
                    picked = Some(me);
                    double = true;
                }
            }
        }
    });
    if let Some(p) = picked
        && let Some(d) = app.refsync.dialog.as_mut()
    {
        d.choice = Some(p);
        d.error = None;
    }
    double
}

// ------------------------------------------------------------------ editor bar

/// The bar above a synced file: read-only, where it comes from, when it was last synced, and
/// buttons to sync now or to disconnect it (it then becomes an ordinary text file).
pub fn sync_bar(app: &mut App, ui: &mut Ui, rect: Rect, rel: &str) {
    let Some(root) = app.project.as_ref().map(|p| p.root.clone()) else {
        return;
    };
    let Some(file) = app.refsync.get(&root, rel).cloned() else { return };
    let connected = app.refsync.account().is_some();
    let busy = app.refsync.is_busy(&root, rel);
    ui.painter().rect_filled(rect, 0.0, mix(pal().topbar, pal().accent, 0.07));
    ui.painter()
        .hline(rect.x_range(), rect.bottom() - 0.5, Stroke::new(1.0_f32, pal().separator));
    let error = if connected {
        file.error.clone()
    } else {
        Some("Zotero ist nicht verbunden – die Datei wird nicht aktualisiert.".to_string())
    };
    let mut sync_now = false;
    let mut detach = false;
    let mut connect = false;
    ui.scope_builder(UiBuilder::new().max_rect(rect.shrink2(vec2(BAR_PAD, 0.0))), |ui| {
        ui.horizontal_centered(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            let (icon_rect, _) = ui.allocate_exact_size(vec2(18.0, 18.0), Sense::hover());
            if busy {
                ui.put(icon_rect, egui::Spinner::new().size(13.0).color(pal().dim));
            } else if error.is_some() {
                paint(ui.painter(), icon_rect.shrink(1.0), Icon::Warning, pal().error);
            } else {
                paint(ui.painter(), icon_rect.shrink(1.0), Icon::Sync, pal().accent);
            }
            // Right-hand side first, so a long source name gets shortened instead.
            let status = if busy {
                "Wird synchronisiert …".to_string()
            } else {
                match file.last_sync {
                    Some(t) => format!("Zuletzt synchronisiert {}", refsync::ago(t, refsync::now_secs())),
                    None => "Noch nicht synchronisiert".to_string(),
                }
            };
            let status_w = ui.fonts_mut(|f| f.layout_no_wrap(status.clone(), FontId::proportional(TEXT_SMALL), pal().dim).size().x);
            let right_w = status_w + 2.0 * (28.0 + TOOL_GAP) + if connected { 0.0 } else { 110.0 } + 24.0;
            let left_w = (ui.available_width() - right_w).max(60.0);
            let (text, color, tip) = match &error {
                Some(e) => (e.clone(), pal().error, e.clone()),
                None => (
                    format!("Schreibgeschützt · aus Zotero: {}", file.source),
                    pal().text_soft,
                    format!(
                        "Diese Datei wird aus Zotero synchronisiert ({}) und kann hier nicht bearbeitet werden.\n{} {}. Zum Bearbeiten die Synchronisierung trennen.",
                        file.source,
                        file.entries,
                        if file.entries == 1 { "Eintrag" } else { "Einträge" }
                    ),
                ),
            };
            ui.allocate_ui(vec2(left_w, ui.available_height()), |ui| {
                ui.horizontal_centered(|ui| {
                    ui.add(egui::Label::new(RichText::new(text).size(TEXT_SMALL).color(color)).truncate()).on_hover_text(tip);
                });
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = TOOL_GAP;
                if ui.add(ToolButton::new(Icon::Unlink, "Synchronisierung trennen – die Datei wird zu einer normalen, bearbeitbaren Datei")).clicked() {
                    detach = true;
                }
                if connected {
                    let tip = if busy { "Wird gerade synchronisiert" } else { "Jetzt synchronisieren" };
                    if ui.add_enabled(!busy, ToolButton::new(Icon::Sync, tip)).clicked() {
                        sync_now = true;
                    }
                } else if ui.add(Button::new("Verbinden …")).clicked() {
                    connect = true;
                }
                ui.add_space(6.0);
                ui.label(RichText::new(status).size(TEXT_SMALL).color(pal().dim));
            });
        });
    });
    // "vor 4 Minuten" moves on by itself.
    ui.ctx().request_repaint_after(Duration::from_secs(20));
    if sync_now {
        app.refsync.sync_file(&root, rel, true);
    }
    if detach {
        app.refsync.detach(&root, rel);
        app.toast(
            format!("„{rel}“ ist jetzt eine normale Datei und wird nicht mehr synchronisiert."),
            true,
        );
    }
    if connect {
        open_accounts(app, ui.ctx());
    }
}

/// The small mark next to a synced file in the file list, with a tooltip.
pub fn file_badge(app: &App, ui: &Ui, resp: &egui::Response, at: egui::Pos2, rel: &str) -> egui::Rect {
    let rect = Rect::from_center_size(at, vec2(14.0, 14.0));
    let Some(root) = app.project.as_ref().map(|p| &p.root) else {
        return rect;
    };
    let Some(file) = app.refsync.get(root, rel) else { return rect };
    let failing = file.error.is_some() || app.refsync.account().is_none();
    if app.refsync.is_busy(root, rel) {
        egui::Spinner::new().size(11.0).color(pal().dim).paint_at(ui, rect);
    } else {
        paint(
            ui.painter(),
            rect,
            if failing { Icon::Warning } else { Icon::Sync },
            if failing { pal().error } else { pal().dim },
        );
    }
    if resp.hovered() {
        let when = file
            .last_sync
            .map(|t| format!("zuletzt {}", refsync::ago(t, refsync::now_secs())))
            .unwrap_or_else(|| "noch nicht synchronisiert".into());
        let mut tip = format!("Aus Zotero synchronisiert ({when})\n{}", file.source);
        if let Some(e) = &file.error {
            tip.push_str(&format!("\n{e}"));
        }
        resp.clone().on_hover_text(tip);
    }
    rect
}
