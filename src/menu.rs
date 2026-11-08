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
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
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
        Item("Aus Zotero synchronisieren …", "", A::SyncFromZotero),
        Separator,
        Item("Tab schließen", "CmdOrCtrl+W", A::CloseTab),
        Item("Projekt schließen", "CmdOrCtrl+Shift+W", A::CloseProject),
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
        Item("Nächster Tab", "Control+Tab", A::NextTab),
        Item("Vorheriger Tab", "Control+Shift+Tab", A::PrevTab),
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
            | SyncFromZotero
            | CloseProject
            | NextTab
            | PrevTab
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
            "Control" => {
                if MAC {
                    "⌃"
                } else {
                    "Strg"
                }
            }
            "Tab" => {
                if MAC {
                    "⇥"
                } else {
                    "Tab"
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
pub fn window_menu_bar(app: &mut App, ui: &mut egui::Ui) {
    let pal = crate::theme::pal();
    egui::Panel::top("menubar")
        .frame(egui::Frame::new().fill(pal.topbar).inner_margin(egui::Margin::symmetric(6, 2)))
        .show_separator_line(true)
        .show(ui, |ui| {
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

        // Clicks (menu bar and pop-up menus) arrive on the menu's own callback; queue them and
        // wake the app up.
        let ctx = ctx.clone();
        muda::MenuEvent::set_event_handler(Some(move |event: muda::MenuEvent| {
            if let Ok(mut q) = MENU_EVENTS.lock() {
                q.push(event.id().clone());
                if q.len() > 64 {
                    q.remove(0);
                }
            }
            ctx.request_repaint();
        }));
        Some(Self {
            _menu: menu,
            items,
            recent: recent?,
            recent_items: Vec::new(),
            recent_shown: vec![std::path::PathBuf::from("\u{0}")],
            enabled_for_project: None,
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
        if let Ok(mut q) = MENU_EVENTS.lock() {
            q.retain(|id| {
                if let Some((_, action)) = self.items.iter().find(|(item, _)| item.id() == id) {
                    out.push(*action);
                    false
                } else if let Some(i) = self.recent_items.iter().position(|item| item.id() == id) {
                    if i < recent.len() {
                        out.push(MenuAction::OpenRecent(i));
                    }
                    false
                } else {
                    true
                }
            });
        }
        out
    }
}

#[cfg(target_os = "macos")]
static MENU_EVENTS: std::sync::Mutex<Vec<muda::MenuId>> = std::sync::Mutex::new(Vec::new());

// ---------------------------------------------------------------- pop-up menus
//
// Right-click menus and drop-down choices. On macOS they are real system menus (NSMenu),
// elsewhere a menu drawn in the window. Call sites use the same two calls on every system:
// `open_popup(...)` when clicked, and `popup_choice(key)` every frame to get the answer.

/// One line of a pop-up menu.
#[derive(Clone)]
pub struct PopupItem {
    pub label: String,
    pub enabled: bool,
    pub checked: bool,
    pub separator_before: bool,
    pub shortcut: &'static str,
}

impl PopupItem {
    pub fn new(label: impl Into<String>) -> Self {
        Self { label: label.into(), enabled: true, checked: false, separator_before: false, shortcut: "" }
    }
    pub fn enabled(mut self, on: bool) -> Self {
        self.enabled = on;
        self
    }
    pub fn checked(mut self, on: bool) -> Self {
        self.checked = on;
        self
    }
    /// A separator line above this item.
    pub fn sep(mut self) -> Self {
        self.separator_before = true;
        self
    }
    pub fn shortcut(mut self, accel: &'static str) -> Self {
        self.shortcut = accel;
        self
    }
}

struct PendingPopup {
    key: String,
    payload: String,
    #[cfg(target_os = "macos")]
    ids: Vec<Option<muda::MenuId>>,
    #[cfg(not(target_os = "macos"))]
    items: Vec<PopupItem>,
    #[cfg(not(target_os = "macos"))]
    at: egui::Pos2,
    #[cfg(not(target_os = "macos"))]
    viewport: egui::ViewportId,
    #[cfg(not(target_os = "macos"))]
    opened_frame: u64,
    chosen: Option<usize>,
}

thread_local! {
    static POPUP: std::cell::RefCell<Option<PendingPopup>> = const { std::cell::RefCell::new(None) };
}

/// Shows a pop-up menu at `at` (in the coordinates of the window being drawn). `payload`
/// travels with the answer, e.g. which file was right-clicked.
pub fn open_popup(ctx: &egui::Context, key: &str, payload: impl Into<String>, items: Vec<PopupItem>, at: egui::Pos2) {
    #[cfg(target_os = "macos")]
    {
        use muda::ContextMenu;
        let menu = muda::Menu::new();
        let mut ids = Vec::new();
        for item in &items {
            if item.separator_before {
                let _ = menu.append(&muda::PredefinedMenuItem::separator());
            }
            let accel = mac_key_accelerator(item.shortcut);
            if item.checked {
                let m = muda::CheckMenuItem::new(&item.label, item.enabled, true, None);
                if let Some(k) = accel {
                    let _ = m.set_key_accelerator(Some(k));
                }
                ids.push(Some(m.id().clone()));
                let _ = menu.append(&m);
            } else {
                let m = muda::MenuItem::new(&item.label, item.enabled, None);
                if let Some(k) = accel {
                    let _ = m.set_key_accelerator(Some(k));
                }
                ids.push(Some(m.id().clone()));
                let _ = menu.append(&m);
            }
        }
        POPUP.with(|p| *p.borrow_mut() = Some(PendingPopup { key: key.into(), payload: payload.into(), ids, chosen: None }));
        let Some(view) = key_window_view() else { return };
        // muda's `show_context_menu_for_nsview` assumes an unflipped NSView, but the winit
        // content view is flipped (origin at the top left, like egui's coordinates) – passing
        // the position through it mirrors the anchor vertically across the window's middle.
        // So position the menu ourselves: AppKit interprets the location in the view's own
        // coordinate system, which for a flipped view is exactly the point `at`, scaled to
        // window points. AppKit also fits the menu to the screen like any native context menu.
        let zoom = ctx.zoom_factor();
        let location = objc2_foundation::NSPoint::new((at.x * zoom) as f64, (at.y * zoom) as f64);
        // SAFETY: `ns_menu()` returns the NSMenu belonging to this muda Menu, which outlives the call.
        let ns_menu = unsafe { &*(ContextMenu::ns_menu(&menu) as *const objc2_app_kit::NSMenu) };
        // Blocks until the menu closes, like any system menu.
        ns_menu.popUpMenuPositioningItem_atLocation_inView(None, location, Some(&view));
        ctx.request_repaint();
    }
    #[cfg(not(target_os = "macos"))]
    {
        let frame = ctx.cumulative_frame_nr();
        let viewport = ctx.viewport_id();
        POPUP.with(|p| {
            *p.borrow_mut() = Some(PendingPopup { key: key.into(), payload: payload.into(), items, at, viewport, opened_frame: frame, chosen: None })
        });
    }
}

/// The item chosen in the pop-up menu opened under `key`, with its payload.
pub fn popup_choice(key: &str) -> Option<(usize, String)> {
    POPUP.with(|p| {
        let mut slot = p.borrow_mut();
        let pending = slot.as_mut()?;
        if pending.key != key {
            return None;
        }
        #[cfg(target_os = "macos")]
        if pending.chosen.is_none()
            && let Ok(mut q) = MENU_EVENTS.lock()
            && let Some(pos) = q.iter().position(|id| pending.ids.iter().any(|i| i.as_ref() == Some(id)))
        {
            let id = q.remove(pos);
            pending.chosen = pending.ids.iter().position(|i| i.as_ref() == Some(&id));
        }
        let chosen = pending.chosen?;
        let payload = pending.payload.clone();
        *slot = None;
        Some((chosen, payload))
    })
}

/// The key window's content view: system pop-up menus open in the window that was clicked.
#[cfg(target_os = "macos")]
fn key_window_view() -> Option<objc2::rc::Retained<objc2_app_kit::NSView>> {
    use objc2::MainThreadMarker;
    use objc2_app_kit::NSApplication;
    let mtm = MainThreadMarker::new()?;
    let app = NSApplication::sharedApplication(mtm);
    let window = app.keyWindow().or_else(|| app.mainWindow())?;
    window.contentView()
}

/// Draws the pop-up menu inside the window (systems without native pop-up menus).
#[cfg(not(target_os = "macos"))]
pub fn draw_fallback_popup(ctx: &egui::Context) {
    let open = POPUP.with(|p| {
        p.borrow()
            .as_ref()
            .filter(|x| x.chosen.is_none() && x.viewport == ctx.viewport_id())
            .map(|x| (x.items.clone(), x.at, x.opened_frame))
    });
    let Some((items, at, opened)) = open else { return };
    let mut chosen = None;
    let area = egui::Area::new(egui::Id::unique("typisch-popup")).order(egui::Order::Foreground).fixed_pos(at).show(ctx, |ui| {
        egui::Frame::menu(ui.style()).show(ui, |ui| {
            ui.set_min_width(200.0);
            for (i, item) in items.iter().enumerate() {
                if item.separator_before {
                    ui.separator();
                }
                let label = if item.checked { format!("✓ {}", item.label) } else { item.label.clone() };
                let b = egui::Button::new(label).shortcut_text(shortcut_text(item.shortcut)).frame(false);
                if ui.add_enabled(item.enabled, b).clicked() {
                    chosen = Some(i);
                }
            }
        });
    });
    let clicked_outside = ctx.input(|i| i.pointer.any_click()) && !area.response.hovered() && ctx.cumulative_frame_nr() > opened + 1;
    let escape = ctx.input(|i| i.key_pressed(egui::Key::Escape));
    POPUP.with(|p| {
        let mut slot = p.borrow_mut();
        if chosen.is_some() {
            if let Some(x) = slot.as_mut() {
                x.chosen = chosen;
            }
        } else if clicked_outside || escape {
            *slot = None;
        }
    });
}

/// A drop-down button in the Mac style (value and ⌃⌄ arrows) that opens a system menu.
/// Returns the chosen index.
pub fn popup_button(ui: &mut egui::Ui, key: &str, current: &str, items: Vec<PopupItem>) -> Option<usize> {
    use crate::widgets::{CONTROL_H, RADIUS, TEXT, flat_fill};
    let pal = crate::theme::pal();
    let font = egui::FontId::proportional(TEXT);
    let galley = ui.fonts_mut(|f| f.layout_no_wrap(current.to_string(), font.clone(), pal.text));
    let size = egui::vec2(galley.size().x + 44.0, CONTROL_H);
    let (rect, resp) = ui.allocate_exact_size(size, egui::Sense::click());
    let resp = resp.on_hover_cursor(egui::CursorIcon::Default);
    if let Some(fill) = flat_fill(&resp, false) {
        ui.painter().rect_filled(rect, RADIUS, fill);
    }
    ui.painter().galley(egui::pos2(rect.left() + 8.0, rect.center().y - galley.size().y / 2.0), galley, pal.text);
    // The round ⌃⌄ badge macOS shows on pop-up buttons.
    let c = egui::pos2(rect.right() - 15.0, rect.center().y);
    ui.painter().circle_filled(c, 10.0, pal.selected);
    let s = egui::Stroke::new(1.5_f32, pal.text_soft);
    ui.painter().line_segment([c + egui::vec2(-3.5, -1.5), c + egui::vec2(0.0, -5.0)], s);
    ui.painter().line_segment([c + egui::vec2(0.0, -5.0), c + egui::vec2(3.5, -1.5)], s);
    ui.painter().line_segment([c + egui::vec2(-3.5, 1.5), c + egui::vec2(0.0, 5.0)], s);
    ui.painter().line_segment([c + egui::vec2(0.0, 5.0), c + egui::vec2(3.5, 1.5)], s);
    if resp.clicked() {
        open_popup(ui.ctx(), key, "", items, rect.left_bottom() + egui::vec2(0.0, 4.0));
    }
    popup_choice(key).map(|(i, _)| i)
}
