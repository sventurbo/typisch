//! The app's menus, described once and shown natively:
//! on macOS in the system menu bar at the top of the screen, elsewhere as a classic
//! menu bar at the top of the window (the usual place on Linux desktops).

use eframe::egui;

use crate::app::{App, MenuAction, Screen};

/// One entry of a menu. Shortcuts use the accelerator syntax, e.g. "CmdOrCtrl+Shift+F".
pub enum Entry {
    Item(&'static str, &'static str, MenuAction),
    Separator,
    /// The "recently opened" projects submenu.
    Recent,
    /// Standard macOS items handled by the system itself.
    #[allow(dead_code)]
    System(SystemItem),
}

#[derive(Clone, Copy)]
#[allow(dead_code)]
pub enum SystemItem {
    About,
    Services,
    Hide,
    HideOthers,
    ShowAll,
    Minimize,
    Zoom,
    Fullscreen,
    BringAllToFront,
}

pub struct MenuDef {
    pub title: &'static str,
    pub entries: Vec<Entry>,
    /// The macOS "Window" / "Help" menus get extra system behaviour.
    pub role: Role,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Normal,
    Window,
    Help,
}

const MAC: bool = cfg!(target_os = "macos");

pub fn menus() -> Vec<MenuDef> {
    use Entry::*;
    use MenuAction as A;
    let mut out = Vec::new();
    if MAC {
        out.push(MenuDef {
            title: "Typisch",
            role: Role::Normal,
            entries: vec![
                System(SystemItem::About),
                Separator,
                Item("Einstellungen …", "CmdOrCtrl+Comma", A::Settings),
                Separator,
                System(SystemItem::Services),
                Separator,
                System(SystemItem::Hide),
                System(SystemItem::HideOthers),
                System(SystemItem::ShowAll),
                Separator,
                Item("Typisch beenden", "CmdOrCtrl+Q", A::Quit),
            ],
        });
    }
    let mut file = vec![
        Item("Neues Projekt aus Template …", "CmdOrCtrl+N", A::NewFromTemplate),
        Item("Ordner öffnen …", "CmdOrCtrl+O", A::OpenFolder),
        Item("Datei öffnen …", "CmdOrCtrl+Shift+O", A::OpenFile),
        Recent,
        Separator,
        Item(if MAC { "Sichern" } else { "Speichern" }, "CmdOrCtrl+S", A::Save),
        Item("Als PDF exportieren …", "CmdOrCtrl+E", A::ExportPdf),
        Separator,
        Item("Projekt schließen", "CmdOrCtrl+W", A::CloseProject),
    ];
    if !MAC {
        file.extend([Separator, Item("Einstellungen …", "CmdOrCtrl+Comma", A::Settings), Separator, Item("Beenden", "CmdOrCtrl+Q", A::Quit)]);
    }
    out.push(MenuDef { title: if MAC { "Ablage" } else { "Datei" }, role: Role::Normal, entries: file });
    out.push(MenuDef {
        title: "Bearbeiten",
        role: Role::Normal,
        entries: vec![
            Item("Widerrufen", "CmdOrCtrl+Z", A::Undo),
            Item("Wiederholen", "CmdOrCtrl+Shift+Z", A::Redo),
            Separator,
            Item("Ausschneiden", "CmdOrCtrl+X", A::Cut),
            Item("Kopieren", "CmdOrCtrl+C", A::Copy),
            Item("Einfügen", "CmdOrCtrl+V", A::Paste),
            Item("Alles auswählen", "CmdOrCtrl+A", A::SelectAll),
            Separator,
            Item("Fett", "CmdOrCtrl+B", A::Bold),
            Item("Kursiv", "CmdOrCtrl+I", A::Italic),
            Item("Unterstrichen", "CmdOrCtrl+U", A::Underline),
            Item("Formel", "", A::Formula),
            Separator,
            Item("Im Projekt suchen", "CmdOrCtrl+Shift+F", A::SearchProject),
        ],
    });
    let mut view = vec![
        Item("Seitenleiste ein/aus", "CmdOrCtrl+Alt+S", A::ToggleSidebar),
        Item("Vorschau ein/aus", "CmdOrCtrl+Alt+P", A::TogglePreview),
        Item("Probleme anzeigen", "CmdOrCtrl+Shift+M", A::ShowProblems),
        Separator,
        Item("Vorschau vergrößern", "CmdOrCtrl+Equal", A::PreviewZoomIn),
        Item("Vorschau verkleinern", "CmdOrCtrl+Minus", A::PreviewZoomOut),
        Item("Vorschau an Breite anpassen", "CmdOrCtrl+Digit0", A::PreviewFit),
        Separator,
        Item("Oberfläche größer", "CmdOrCtrl+Shift+Equal", A::UiBigger),
        Item("Oberfläche kleiner", "CmdOrCtrl+Shift+Minus", A::UiSmaller),
        Item("Oberfläche normal", "CmdOrCtrl+Shift+Digit0", A::UiReset),
    ];
    // macOS adds its own "Vollbildmodus" entry to this menu.
    if !MAC {
        view.extend([Separator, Item("Vollbild", "F11", A::Fullscreen)]);
    }
    out.push(MenuDef { title: if MAC { "Darstellung" } else { "Ansicht" }, role: Role::Normal, entries: view });
    if MAC {
        out.push(MenuDef {
            title: "Fenster",
            role: Role::Window,
            entries: vec![System(SystemItem::Minimize), System(SystemItem::Zoom), Separator, System(SystemItem::BringAllToFront)],
        });
    }
    let mut help = vec![Item("Typst-Dokumentation", "", A::Documentation)];
    if !MAC {
        help.extend([Separator, Item("Über Typisch", "", A::About)]);
    }
    out.push(MenuDef { title: "Hilfe", role: Role::Help, entries: help });
    out
}

/// Actions that only make sense with a project open.
pub fn needs_project(a: MenuAction) -> bool {
    use MenuAction::*;
    matches!(
        a,
        Save | ExportPdf
            | CloseProject
            | Bold
            | Italic
            | Underline
            | Formula
            | SearchProject
            | ToggleSidebar
            | TogglePreview
            | ShowProblems
            | PreviewZoomIn
            | PreviewZoomOut
            | PreviewFit
    )
}

/// "CmdOrCtrl+Shift+Equal" → "Strg+Umschalt++" (shown next to entries in the window's menu bar).
pub fn shortcut_text(accel: &str) -> String {
    if accel.is_empty() {
        return String::new();
    }
    accel
        .split('+')
        .map(|part| match part {
            "CmdOrCtrl" => {
                if MAC {
                    "⌘"
                } else {
                    "Strg"
                }
            }
            "Shift" => {
                if MAC {
                    "⇧"
                } else {
                    "Umschalt"
                }
            }
            "Alt" => {
                if MAC {
                    "⌥"
                } else {
                    "Alt"
                }
            }
            "Equal" => "+",
            "Minus" => "-",
            "Backslash" => "\\",
            "Comma" => ",",
            "Digit0" => "0",
            other => other,
        })
        .collect::<Vec<_>>()
        .join(if MAC { "" } else { "+" })
}

// ---------------------------------------------------------------- in-window menu bar

/// The classic menu bar at the top of the window (Linux and Windows).
pub fn window_menu_bar(app: &mut App, ctx: &egui::Context) {
    let pal = crate::theme::pal();
    egui::TopBottomPanel::top("menubar")
        .frame(egui::Frame::new().fill(pal.topbar).inner_margin(egui::Margin::symmetric(6, 2)))
        .show_separator_line(true)
        .show(ctx, |ui| {
            egui::MenuBar::new().ui(ui, |ui| {
                let has_project = app.screen == Screen::Ide && app.project.is_some();
                let mut chosen = None;
                for menu in menus() {
                    ui.menu_button(menu.title, |ui| {
                        ui.set_min_width(260.0);
                        for entry in &menu.entries {
                            match entry {
                                Entry::Item(label, accel, action) => {
                                    let enabled = has_project || !needs_project(*action);
                                    let b = egui::Button::new(*label).shortcut_text(shortcut_text(accel));
                                    if ui.add_enabled(enabled, b).clicked() {
                                        chosen = Some(*action);
                                        ui.close();
                                    }
                                }
                                Entry::Separator => {
                                    ui.separator();
                                }
                                Entry::Recent => {
                                    ui.menu_button("Zuletzt geöffnet", |ui| {
                                        if app.settings.recent.is_empty() {
                                            ui.add_enabled(false, egui::Button::new("Keine Projekte"));
                                        }
                                        for (i, p) in app.settings.recent.iter().enumerate() {
                                            let name = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                                            if ui.button(name).on_hover_text(crate::app::tilde(p)).clicked() {
                                                chosen = Some(MenuAction::OpenRecent(i));
                                                ui.close();
                                            }
                                        }
                                    });
                                }
                                Entry::System(_) => {}
                            }
                        }
                    });
                }
                if let Some(action) = chosen {
                    app.run(action, ui.ctx());
                }
            });
        });
}

// ---------------------------------------------------------------- macOS menu bar

/// The shortcut as the character printed on the key ("⌘+" is the "+" key on any layout).
/// Interface-size shortcuts stay out of the Mac menu: "+" needs Shift on some layouts and
/// would clash with the preview zoom.
#[cfg(target_os = "macos")]
fn mac_key_accelerator(accel: &str) -> Option<muda::accelerator::KeyAccelerator> {
    if accel.is_empty() {
        return None;
    }
    let (mods, key) = accel.rsplit_once('+').unwrap_or(("", accel));
    let key = match key {
        "Equal" => "+",
        "Minus" => "-",
        "Comma" => ",",
        "Digit0" => "0",
        other => other,
    };
    if mods.contains("Shift") && ["+", "-", "0"].contains(&key) {
        return None;
    }
    let mods = mods.replace("CmdOrCtrl", "Super");
    let text = if mods.is_empty() { key.to_string() } else { format!("{mods}+{key}") };
    text.parse().ok()
}

#[cfg(target_os = "macos")]
pub struct NativeMenu {
    _menu: muda::Menu,
    items: Vec<(muda::MenuItem, MenuAction)>,
    recent: muda::Submenu,
    recent_items: Vec<muda::MenuItem>,
    recent_shown: Vec<std::path::PathBuf>,
    enabled_for_project: Option<bool>,
    rx: std::sync::mpsc::Receiver<muda::MenuEvent>,
}

#[cfg(not(target_os = "macos"))]
pub struct NativeMenu;

#[cfg(not(target_os = "macos"))]
impl NativeMenu {
    pub fn install(_ctx: &egui::Context) -> Option<Self> {
        None
    }
    pub fn poll(&mut self, _app_recent: &[std::path::PathBuf], _has_project: bool) -> Vec<MenuAction> {
        Vec::new()
    }
}

#[cfg(target_os = "macos")]
impl NativeMenu {
    /// Builds the system menu bar and hands its clicks to the app.
    pub fn install(ctx: &egui::Context) -> Option<Self> {
        use muda::{AboutMetadata, Menu, MenuItem, PredefinedMenuItem, Submenu};

        let menu = Menu::new();
        let mut items = Vec::new();
        let mut recent = None;
        for def in menus() {
            let sub = Submenu::new(def.title, true);
            for entry in &def.entries {
                match entry {
                    Entry::Item(label, accel, action) => {
                        let item = MenuItem::new(*label, true, None);
                        if let Some(key) = mac_key_accelerator(accel) {
                            let _ = item.set_key_accelerator(Some(key));
                        }
                        let _ = sub.append(&item);
                        items.push((item, *action));
                    }
                    Entry::Separator => {
                        let _ = sub.append(&PredefinedMenuItem::separator());
                    }
                    Entry::Recent => {
                        let r = Submenu::new("Zuletzt geöffnet", true);
                        let _ = sub.append(&r);
                        recent = Some(r);
                    }
                    Entry::System(kind) => {
                        let item = match kind {
                            SystemItem::About => PredefinedMenuItem::about(
                                Some("Über Typisch"),
                                Some(AboutMetadata {
                                    name: Some("Typisch".into()),
                                    version: Some(crate::app::VERSION.into()),
                                    short_version: Some(crate::app::VERSION.into()),
                                    comments: Some("Typst-Editor mit Live-Vorschau (Typst 0.15)".into()),
                                    copyright: Some(format!("Veröffentlicht im {}", crate::app::RELEASE_DATE)),
                                    ..Default::default()
                                }),
                            ),
                            SystemItem::Services => PredefinedMenuItem::services(Some("Dienste")),
                            SystemItem::Hide => PredefinedMenuItem::hide(Some("Typisch ausblenden")),
                            SystemItem::HideOthers => PredefinedMenuItem::hide_others(Some("Andere ausblenden")),
                            SystemItem::ShowAll => PredefinedMenuItem::show_all(Some("Alle einblenden")),
                            SystemItem::Minimize => PredefinedMenuItem::minimize(Some("Im Dock ablegen")),
                            SystemItem::Zoom => PredefinedMenuItem::maximize(Some("Zoomen")),
                            SystemItem::Fullscreen => PredefinedMenuItem::fullscreen(Some("Vollbildmodus")),
                            SystemItem::BringAllToFront => PredefinedMenuItem::bring_all_to_front(Some("Alle nach vorne bringen")),
                        };
                        let _ = sub.append(&item);
                    }
                }
            }
            match def.role {
                Role::Window => sub.set_as_windows_menu_for_nsapp(),
                Role::Help => sub.set_as_help_menu_for_nsapp(),
                Role::Normal => {}
            }
            let _ = menu.append(&sub);
        }
        menu.init_for_nsapp();

        // Clicks arrive on the menu's own callback; pass them on and wake the app up.
        let (tx, rx) = std::sync::mpsc::channel();
        let ctx = ctx.clone();
        muda::MenuEvent::set_event_handler(Some(move |event: muda::MenuEvent| {
            let _ = tx.send(event);
            ctx.request_repaint();
        }));
        Some(Self {
            _menu: menu,
            items,
            recent: recent?,
            recent_items: Vec::new(),
            recent_shown: vec![std::path::PathBuf::from("\u{0}")],
            enabled_for_project: None,
            rx,
        })
    }

    /// Keeps the menu up to date and returns what was chosen since the last frame.
    pub fn poll(&mut self, recent: &[std::path::PathBuf], has_project: bool) -> Vec<MenuAction> {
        if self.enabled_for_project != Some(has_project) {
            for (item, action) in &self.items {
                item.set_enabled(has_project || !needs_project(*action));
            }
            self.enabled_for_project = Some(has_project);
        }
        if self.recent_shown != recent {
            for item in self.recent_items.drain(..) {
                let _ = self.recent.remove(&item);
            }
            for p in recent {
                let name = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                let item = muda::MenuItem::new(name, true, None);
                let _ = self.recent.append(&item);
                self.recent_items.push(item);
            }
            if recent.is_empty() {
                let item = muda::MenuItem::new("Keine Projekte", false, None);
                let _ = self.recent.append(&item);
                self.recent_items.push(item);
            }
            self.recent_shown = recent.to_vec();
        }
        let mut out = Vec::new();
        while let Ok(event) = self.rx.try_recv() {
            if let Some((_, action)) = self.items.iter().find(|(item, _)| item.id() == event.id()) {
                out.push(*action);
            } else if let Some(i) = self.recent_items.iter().position(|item| item.id() == event.id()) {
                if i < recent.len() {
                    out.push(MenuAction::OpenRecent(i));
                }
            }
        }
        out
    }
}
