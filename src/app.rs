//! Application state, project handling and the frame loop.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use eframe::egui::{self, Color32, FontData, FontDefinitions, FontFamily, FontId, TextStyle};

use crate::compiler::{Compiler, Diag, Job, OutlineItem, PageLink, Reply};
use crate::theme::Settings;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    Welcome,
    Templates,
    Ide,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SideTab {
    Files,
    Search,
    Outline,
    Symbols,
    Problems,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SettingsTab {
    General,
    Editor,
    Packages,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SearchMode {
    Containing,
    WholeWord,
    StartsWith,
}

#[derive(Clone)]
pub struct FileEntry {
    pub rel: String,
    pub name: String,
    pub depth: usize,
    pub is_dir: bool,
}

pub struct Project {
    pub root: PathBuf,
    pub files: Vec<FileEntry>,
    pub collapsed: HashSet<String>,
    pub main_rel: String,
}

pub struct OpenDoc {
    pub rel: String,
    pub text: String,
    pub saved: String,
    pub edited_at: Option<Instant>,
    /// Cursor, selection and undo history of this tab while another tab is showing.
    pub state: Option<egui::text_edit::TextEditState>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MenuAction {
    NewFromTemplate,
    OpenFolder,
    OpenFile,
    Save,
    ExportPdf,
    CloseProject,
    Settings,
    ToggleSidebar,
    TogglePreview,
    SearchProject,
    PreviewZoomIn,
    PreviewZoomOut,
    PreviewFit,
    UiBigger,
    UiSmaller,
    UiReset,
    Fullscreen,
    About,
    Quit,
    Undo,
    Redo,
    Cut,
    Copy,
    Paste,
    SelectAll,
    Bold,
    Italic,
    Underline,
    Formula,
    ShowProblems,
    Documentation,
    OpenRecent(usize),
    CloseTab,
    NextTab,
    PrevTab,
}

/// The app's version and release date (both set in Cargo.toml).
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const RELEASE_DATE: &str = env!("TYPISCH_RELEASE_DATE");

/// "Version 0.1.0 · November 2026"
pub fn version_line() -> String {
    format!("Version {VERSION} · {RELEASE_DATE}")
}

/// Resize the window, never beyond what fits on the screen it is on.
pub fn resize_window(ctx: &egui::Context, want: egui::Vec2) {
    let monitor = ctx.input(|i| i.viewport().monitor_size);
    let size = match monitor {
        Some(m) if m.x > 100.0 && m.y > 100.0 => want.min(m * 0.92),
        _ => want,
    };
    ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(size));
}

/// Shortcut text in the platform's own words ("Ctrl+S" on Linux, "⌘S" on macOS).
pub fn shortcut_label(s: &str) -> String {
    if s.is_empty() {
        return String::new();
    }
    if cfg!(target_os = "macos") {
        s.replace("Mod+Shift+", "⇧⌘").replace("Mod+", "⌘")
    } else {
        s.replace("Mod+", "Strg+").replace("Shift+", "Umschalt+")
    }
}

/// One page of the preview. Pixels are only kept for pages near the visible area.
pub struct PreviewPage {
    pub size_pt: egui::Vec2,
    pub hash: u128,
    /// Clickable links on this page, in points.
    pub links: Vec<PageLink>,
    /// The texture currently shown, with the content fingerprint and scale it was rendered at.
    pub tex: Option<(egui::TextureHandle, u128, f32)>,
    /// A render already asked for: (hash, scale).
    pub pending: Option<(u128, f32)>,
}

/// A place in the preview the reader left by following a link, so they can jump back to it.
#[derive(Clone, Copy)]
pub struct PreviewSpot {
    /// Scroll target: page (1-based) and offset from its top, in points.
    pub page: usize,
    pub y_pt: f32,
    /// The page shown as current at that moment, for the button label.
    pub shown: usize,
}

pub struct SearchHit {
    pub rel: String,
    pub line: usize,
    pub text: String,
    pub start: usize,
    pub len: usize,
}

pub struct App {
    pub settings: Settings,
    pub screen: Screen,
    pub compiler: Compiler,
    pub project: Option<Project>,
    /// The file in the active editor tab.
    pub doc: Option<OpenDoc>,
    /// The other open tabs (kept in memory, already saved).
    pub tabs: Vec<OpenDoc>,
    /// All open tabs, left to right (relative paths).
    pub tab_order: Vec<String>,
    /// Which tab the editor widget currently shows (to swap cursor and undo state).
    pub editor_shown_rel: Option<String>,

    // Preview state
    pub pages: Vec<PreviewPage>,
    pub diags: Vec<Diag>,
    pub outline: Vec<OutlineItem>,
    pub words: usize,
    pub compile_tag: u64,
    pub compiled_tag: u64,
    pub compile_due: Option<Instant>,
    pub compiling: bool,
    pub last_compile_ms: u128,
    pub zoom: f32,
    pub fit_width: bool,
    pub preview_visible: bool,
    pub preview_scroll_to: Option<(usize, f32)>,
    /// Where followed links were clicked, newest last; the back button returns there.
    pub preview_back: Vec<PreviewSpot>,
    pub preview_page_now: usize,
    pub ripple: Option<(egui::Pos2, Instant)>,

    // Sidebar
    pub side_tab: SideTab,
    pub search_query: String,
    pub search_mode: SearchMode,
    pub search_hits: Vec<SearchHit>,
    pub search_for: String,
    pub symbol_query: String,
    pub symbol_draw: bool,
    pub strokes: Vec<Vec<egui::Pos2>>,

    // Editor
    pub jump_to: Option<(usize, usize)>,
    pub row_y: Vec<f32>,
    pub cursor_line: usize,
    pub pending_op: Option<crate::ide::EditOp>,
    pub two_pages: bool,

    // Screens
    pub settings_open: bool,
    /// Where the settings window opens (centred over the main window), fixed while it is open.
    pub settings_pos: Option<egui::Pos2>,
    pub settings_tab: SettingsTab,
    pub template_category: Option<&'static str>,
    pub template_query: String,
    pub template_selected: usize,
    pub thumbs: HashMap<String, egui::TextureHandle>,
    pub thumbs_requested: bool,
    pub pkg_spec: String,
    pub pkg_message: Option<(bool, String)>,
    pub namespace_input: String,
    pub toast: Option<(String, Instant, bool)>,
    pub window_title: String,
    pub size_checked: bool,
    /// The system menu bar (macOS); elsewhere the menu bar is drawn in the window.
    pub native_menu: Option<crate::menu::NativeMenu>,
    /// Text-editing commands from a menu, applied at the start of the next frame.
    pub pending_edit: Option<MenuAction>,

    // Screenshot mode
    pub screenshot: Option<PathBuf>,
    pub frames: u32,
    pub shot_requested: bool,
    pub started: Instant,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, screenshot: Option<PathBuf>, open: Option<PathBuf>) -> Self {
        install_fonts(&cc.egui_ctx);
        egui_extras_style(&cc.egui_ctx);
        let compiler = Compiler::spawn(cc.egui_ctx.clone());
        let settings = if screenshot.is_some() {
            // Screenshots always use the same, fixed look.
            Settings { follow_system: false, ..Settings::default() }
        } else {
            Settings::load()
        };
        // Desktop-app behaviour: Ctrl/Cmd +/- zoom the preview, not the whole interface.
        cc.egui_ctx.options_mut(|o| o.zoom_with_keyboard = false);
        let mut app = Self {
            settings,
            screen: Screen::Welcome,
            compiler,
            project: None,
            doc: None,
            tabs: Vec::new(),
            tab_order: Vec::new(),
            editor_shown_rel: None,
            pages: Vec::new(),
            diags: Vec::new(),
            outline: Vec::new(),
            words: 0,
            compile_tag: 0,
            compiled_tag: 0,
            compile_due: None,
            compiling: false,
            last_compile_ms: 0,
            zoom: 1.16,
            fit_width: true,
            preview_visible: true,
            preview_scroll_to: None,
            preview_back: Vec::new(),
            preview_page_now: 1,
            ripple: None,
            side_tab: SideTab::Files,
            search_query: String::new(),
            search_mode: SearchMode::Containing,
            search_hits: Vec::new(),
            search_for: String::new(),
            symbol_query: String::new(),
            symbol_draw: false,
            strokes: Vec::new(),
            jump_to: None,
            row_y: Vec::new(),
            cursor_line: 0,
            pending_op: None,
            two_pages: false,
            settings_open: false,
            settings_pos: None,
            settings_tab: SettingsTab::General,
            template_category: None,
            template_query: String::new(),
            template_selected: 0,
            thumbs: HashMap::new(),
            thumbs_requested: false,
            pkg_spec: String::new(),
            pkg_message: None,
            namespace_input: String::new(),
            toast: None,
            window_title: String::new(),
            size_checked: false,
            // TYPISCH_WINDOW_MENU shows the in-window menu bar (as on Linux) for testing.
            native_menu: if std::env::var("TYPISCH_WINDOW_MENU").is_ok() { None } else { crate::menu::NativeMenu::install(&cc.egui_ctx) },
            pending_edit: None,
            screenshot: screenshot.clone(),
            frames: 0,
            shot_requested: false,
            started: Instant::now(),
        };

        if let (Some(_), Some(p)) = (&screenshot, &open) {
            app.open_path(p, &cc.egui_ctx);
        } else if screenshot.is_some() {
            let dir = std::env::temp_dir().join("typisch-demo").join("Rechnung");
            let _ = std::fs::remove_dir_all(&dir);
            let _ = std::fs::create_dir_all(&dir);
            let _ = std::fs::write(dir.join("main.typ"), crate::templates::INVOICE);
            let _ = std::fs::write(dir.join("beispiel-unterschrift.svg"), crate::templates::SIGNATURE_SVG);
            app.open_path(&dir.join("main.typ"), &cc.egui_ctx);
            // Optional view for testing other parts of the window in screenshot mode.
            match std::env::var("TYPISCH_SHOT_VIEW").unwrap_or_default().as_str() {
                "welcome" => app.screen = Screen::Welcome,
                "templates" => app.screen = Screen::Templates,
                "settings" => app.settings_open = true,
                "settings-editor" => {
                    app.settings_open = true;
                    app.settings_tab = SettingsTab::Editor;
                }
                "settings-packages" => {
                    app.settings_open = true;
                    app.settings_tab = SettingsTab::Packages;
                }
                "search" => {
                    app.side_tab = SideTab::Search;
                    app.search_query = "rechnung".into();
                }
                "outline" => app.side_tab = SideTab::Outline,
                v if v.starts_with("theme-") => {
                    // theme-<n>[-settings|-welcome|-templates|-search]
                    let mut parts = v.split('-').skip(1);
                    if let Some(n) = parts.next().and_then(|n| n.parse::<usize>().ok()) {
                        app.settings.choose_theme(n);
                    }
                    match parts.next() {
                        Some("settings") => {
                            app.settings_open = true;
                            app.settings_tab = SettingsTab::Editor;
                        }
                        Some("welcome") => app.screen = Screen::Welcome,
                        Some("templates") => app.screen = Screen::Templates,
                        Some("search") => {
                            app.side_tab = SideTab::Search;
                            app.search_query = "rechnung".into();
                        }
                        _ => {}
                    }
                }
                "symbols" => app.side_tab = SideTab::Symbols,
                "problems" => {
                    app.side_tab = SideTab::Problems;
                    if let Some(d) = app.doc.as_mut() {
                        d.text = d.text.replacen("nummer: \"2026-001\",", "nummer: \"2026-001\"", 1);
                    }
                }
                _ => {}
            }
        } else if let Some(p) = open {
            app.open_path(&p, &cc.egui_ctx);
        } else {
            crate::app::resize_window(&cc.egui_ctx, egui::vec2(820.0, 500.0));
        }
        app
    }

    pub fn toast(&mut self, msg: impl Into<String>, ok: bool) {
        self.toast = Some((msg.into(), Instant::now(), ok));
    }

    // ---------------------------------------------------------------- projects

    pub fn open_path(&mut self, path: &Path, ctx: &egui::Context) {
        let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        if path.is_dir() {
            self.open_project(path, None, ctx);
        } else if let Some(dir) = path.parent() {
            let rel = path.file_name().map(|n| n.to_string_lossy().to_string());
            self.open_project(dir.to_path_buf(), rel, ctx);
        }
    }

    pub fn open_project(&mut self, root: PathBuf, file: Option<String>, ctx: &egui::Context) {
        self.save_now();
        let files = scan_files(&root);
        let main_rel = if let Some(f) = file.as_ref().filter(|f| f.ends_with(".typ")) {
            f.clone()
        } else if files.iter().any(|f| f.rel == "main.typ") {
            "main.typ".to_string()
        } else if let Some(f) = files.iter().find(|f| !f.is_dir && f.rel.ends_with(".typ")) {
            f.rel.clone()
        } else {
            let _ = std::fs::write(root.join("main.typ"), "");
            "main.typ".to_string()
        };
        let files = scan_files(&root);
        self.project = Some(Project { root: root.clone(), files, collapsed: HashSet::new(), main_rel: main_rel.clone() });
        self.pages.clear();
        self.preview_back.clear();
        self.diags.clear();
        self.outline.clear();
        self.search_hits.clear();
        self.doc = None;
        self.tabs.clear();
        self.tab_order.clear();
        self.editor_shown_rel = None;
        self.open_file(&file.unwrap_or(main_rel));
        self.settings.add_recent(root);
        if self.screenshot.is_none() {
            self.settings.save();
        }
        if self.screen != Screen::Ide && self.screenshot.is_none() {
            crate::app::resize_window(ctx, self.ide_size());
        }
        self.screen = Screen::Ide;
        self.schedule_compile(Duration::ZERO);
    }

    pub fn close_project(&mut self, ctx: &egui::Context) {
        self.save_now();
        self.project = None;
        self.doc = None;
        self.pages.clear();
        self.preview_back.clear();
        self.screen = Screen::Welcome;
        crate::app::resize_window(ctx, egui::vec2(820.0, 500.0));
    }

    pub fn refresh_files(&mut self) {
        if let Some(p) = self.project.as_mut() {
            p.files = scan_files(&p.root);
        }
    }

    /// Shows a file in the editor: switches to its tab, or opens a new tab next to the current one.
    pub fn open_file(&mut self, rel: &str) {
        if self.doc.as_ref().map(|d| d.rel == rel).unwrap_or(false) {
            return;
        }
        if let Some(i) = self.tabs.iter().position(|d| d.rel == rel) {
            self.stash_active();
            self.doc = Some(self.tabs.remove(i));
            return;
        }
        let Some(project) = self.project.as_ref() else { return };
        let path = project.root.join(rel);
        match std::fs::read(&path) {
            Ok(bytes) => match String::from_utf8(bytes) {
                Ok(text) => {
                    let after = self.doc.as_ref().and_then(|d| self.tab_order.iter().position(|r| *r == d.rel));
                    self.stash_active();
                    let at = after.map(|i| i + 1).unwrap_or(self.tab_order.len());
                    self.tab_order.insert(at.min(self.tab_order.len()), rel.to_string());
                    self.doc = Some(OpenDoc { rel: rel.to_string(), saved: text.clone(), text, edited_at: None, state: None });
                    self.jump_to = Some((0, 0));
                }
                Err(_) => self.toast("Diese Datei ist keine Textdatei.", false),
            },
            Err(e) => self.toast(format!("Konnte {rel} nicht öffnen: {e}"), false),
        }
    }

    /// Saves the active tab and moves it to the background.
    fn stash_active(&mut self) {
        self.save_now();
        if let Some(d) = self.doc.take() {
            self.tabs.push(d);
        }
    }

    /// Closes a tab (its changes are saved first) and shows its neighbour.
    pub fn close_tab(&mut self, rel: &str) {
        let Some(pos) = self.tab_order.iter().position(|r| r == rel) else { return };
        if self.doc.as_ref().map(|d| d.rel == rel).unwrap_or(false) {
            self.save_now();
            self.doc = None;
            self.tab_order.remove(pos);
            let next = self.tab_order.get(pos).or_else(|| self.tab_order.get(pos.wrapping_sub(1))).cloned();
            if let Some(next) = next {
                if let Some(i) = self.tabs.iter().position(|d| d.rel == next) {
                    self.doc = Some(self.tabs.remove(i));
                }
            }
        } else {
            self.tab_order.remove(pos);
            if let Some(i) = self.tabs.iter().position(|d| d.rel == rel) {
                let d = self.tabs.remove(i);
                if d.text != d.saved {
                    if let Some(p) = self.project.as_ref() {
                        let _ = std::fs::write(p.root.join(&d.rel), &d.text);
                    }
                }
            }
        }
        self.schedule_compile(Duration::ZERO);
    }

    /// Switches to the tab `step` places to the right (negative: left), wrapping around.
    pub fn cycle_tab(&mut self, step: i32) {
        let n = self.tab_order.len() as i32;
        if n < 2 {
            return;
        }
        let cur = self.doc.as_ref().and_then(|d| self.tab_order.iter().position(|r| *r == d.rel)).unwrap_or(0) as i32;
        let next = self.tab_order[((cur + step).rem_euclid(n)) as usize].clone();
        self.open_file(&next);
    }

    pub fn save_now(&mut self) {
        let (Some(project), Some(doc)) = (self.project.as_ref(), self.doc.as_mut()) else { return };
        if doc.text != doc.saved {
            let path = project.root.join(&doc.rel);
            match std::fs::write(&path, &doc.text) {
                Ok(()) => {
                    doc.saved = doc.text.clone();
                    doc.edited_at = None;
                }
                Err(e) => {
                    let msg = format!("Speichern fehlgeschlagen: {e}");
                    self.toast(msg, false);
                }
            }
        }
    }

    pub fn create_from_template(&mut self, index: usize, ctx: &egui::Context) {
        let t = &crate::templates::TEMPLATES[index];
        let Some(parent) = rfd::FileDialog::new().set_title("Speicherort für das neue Projekt wählen").pick_folder() else {
            return;
        };
        let base = if t.source.is_empty() { "Untitled".to_string() } else { t.name.to_string() };
        let mut dir = parent.join(&base);
        let mut n = 2;
        while dir.exists() {
            dir = parent.join(format!("{base} {n}"));
            n += 1;
        }
        if let Err(e) = std::fs::create_dir_all(&dir) {
            self.toast(format!("Ordner konnte nicht angelegt werden: {e}"), false);
            return;
        }
        let _ = std::fs::write(dir.join("main.typ"), t.source);
        if t.source.contains("beispiel-unterschrift.svg") {
            let _ = std::fs::write(dir.join("beispiel-unterschrift.svg"), crate::templates::SIGNATURE_SVG);
        }
        self.open_project(dir, Some("main.typ".into()), ctx);
    }

    // ---------------------------------------------------------------- compile

    pub fn schedule_compile(&mut self, delay: Duration) {
        self.compile_due = Some(Instant::now() + delay);
    }

    fn send_compile(&mut self) {
        let Some(project) = self.project.as_ref() else { return };
        let mut overlays = Vec::new();
        if let Some(doc) = self.doc.as_ref() {
            overlays.push((doc.rel.clone(), doc.text.clone()));
        }
        self.compile_tag += 1;
        self.compiling = true;
        self.compiler.send(Job::Compile {
            tag: self.compile_tag,
            root: project.root.clone(),
            main_rel: project.main_rel.clone(),
            overlays,
        });
    }

    fn poll_compiler(&mut self, ctx: &egui::Context) {
        while let Ok(reply) = self.compiler.rx.try_recv() {
            match reply {
                Reply::Compiled { tag, pages, diags, outline, words, millis } => {
                    self.compiled_tag = tag;
                    if tag == self.compile_tag {
                        self.compiling = false;
                    }
                    self.last_compile_ms = millis;
                    self.diags = diags;
                    if let Some(pages) = pages {
                        // Keep the old pictures on screen until the changed pages are re-rendered.
                        let mut old = std::mem::take(&mut self.pages).into_iter();
                        self.pages = pages
                            .into_iter()
                            .map(|p| {
                                let tex = old.next().and_then(|o| o.tex);
                                PreviewPage { size_pt: p.size_pt, hash: p.hash, links: p.links, tex, pending: None }
                            })
                            .collect();
                        self.outline = outline;
                        self.words = words;
                    }
                }
                Reply::Rendered { index, hash, scale, image } => {
                    if let Some(page) = self.pages.get_mut(index) {
                        if page.hash == hash {
                            match page.tex.as_mut() {
                                Some((tex, h, s)) => {
                                    tex.set(image, egui::TextureOptions::LINEAR);
                                    *h = hash;
                                    *s = scale;
                                }
                                None => {
                                    let tex = ctx.load_texture(format!("page-{index}"), image, egui::TextureOptions::LINEAR);
                                    page.tex = Some((tex, hash, scale));
                                }
                            }
                            if page.pending.map(|(h, _)| h == hash).unwrap_or(false) {
                                page.pending = None;
                            }
                        }
                    }
                }
                Reply::Exported(res) => match res {
                    Ok(p) => self.toast(format!("PDF gespeichert: {}", p.display()), true),
                    Err(e) => self.toast(format!("Export fehlgeschlagen: {e}"), false),
                },
                Reply::Thumbnail { key, image } => {
                    if let Some(img) = image {
                        let tex = ctx.load_texture(format!("thumb-{key}"), img, egui::TextureOptions::LINEAR);
                        self.thumbs.insert(key, tex);
                    }
                }
                Reply::Package(res) => {
                    self.pkg_message = Some(match res {
                        Ok(s) => (true, format!("{s} wurde geladen.")),
                        Err(e) => (false, e),
                    });
                }
            }
        }
    }

    pub fn export_pdf(&mut self) {
        let Some(project) = self.project.as_ref() else { return };
        let stem = Path::new(&project.main_rel)
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "main".into());
        if let Some(path) = rfd::FileDialog::new()
            .set_title("Das aktuelle Dokument als PDF exportieren")
            .set_directory(&project.root)
            .set_file_name(format!("{stem}.pdf"))
            .add_filter("PDF", &["pdf"])
            .save_file()
        {
            self.compiler.send(Job::ExportPdf { path });
        }
    }

    /// One place for everything the app menu, shortcuts and buttons can do.
    pub fn run(&mut self, action: MenuAction, ctx: &egui::Context) {
        match action {
            MenuAction::NewFromTemplate => {
                self.screen = Screen::Templates;
            }
            MenuAction::OpenFolder => {
                if let Some(dir) = rfd::FileDialog::new().set_title("Ordner öffnen").pick_folder() {
                    self.open_path(&dir, ctx);
                }
            }
            MenuAction::OpenFile => {
                if let Some(file) = rfd::FileDialog::new().set_title("Datei öffnen").add_filter("Typst", &["typ"]).pick_file() {
                    self.open_path(&file, ctx);
                }
            }
            MenuAction::Save => self.save_now(),
            MenuAction::ExportPdf => self.export_pdf(),
            MenuAction::CloseProject => self.close_project(ctx),
            MenuAction::CloseTab => {
                // ⌘W closes what is in front: the settings window, else the current tab,
                // and with no tabs left the project.
                if self.settings_open && self.screenshot.is_none() {
                    self.settings_open = false;
                } else if let Some(rel) = self.doc.as_ref().map(|d| d.rel.clone()) {
                    self.close_tab(&rel);
                } else if self.project.is_some() {
                    self.close_project(ctx);
                }
            }
            MenuAction::NextTab => self.cycle_tab(1),
            MenuAction::PrevTab => self.cycle_tab(-1),
            MenuAction::Settings => {
                if self.settings_open && self.screenshot.is_none() {
                    ctx.send_viewport_cmd_to(egui::ViewportId::from_hash_of("typisch-settings"), egui::ViewportCommand::Focus);
                } else {
                    self.settings_open = !self.settings_open;
                    self.settings_pos = None;
                }
            }
            MenuAction::ToggleSidebar => self.settings.sidebar_visible = !self.settings.sidebar_visible,
            MenuAction::TogglePreview => self.preview_visible = !self.preview_visible,
            MenuAction::SearchProject => {
                self.side_tab = SideTab::Search;
                self.settings.sidebar_visible = true;
            }
            MenuAction::PreviewZoomIn => {
                self.fit_width = false;
                self.zoom = (self.zoom * 1.15).min(5.0);
            }
            MenuAction::PreviewZoomOut => {
                self.fit_width = false;
                self.zoom = (self.zoom / 1.15).max(0.25);
            }
            MenuAction::PreviewFit => self.fit_width = true,
            MenuAction::UiBigger => self.settings.ui_scale = (self.settings.ui_scale + 0.1).min(2.0),
            MenuAction::UiSmaller => self.settings.ui_scale = (self.settings.ui_scale - 0.1).max(0.6),
            MenuAction::UiReset => self.settings.ui_scale = 1.0,
            MenuAction::Fullscreen => {
                let full = ctx.input(|i| i.viewport().fullscreen.unwrap_or(false));
                ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(!full));
            }
            MenuAction::About => {
                self.toast(format!("Typisch {VERSION} ({RELEASE_DATE}) – Typst-Editor mit Live-Vorschau (Typst 0.15)"), true);
            }
            MenuAction::Quit => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
            MenuAction::Undo | MenuAction::Redo | MenuAction::Cut | MenuAction::Copy | MenuAction::Paste | MenuAction::SelectAll => {
                self.pending_edit = Some(action);
                ctx.request_repaint();
            }
            MenuAction::Bold => self.pending_op = Some(crate::ide::EditOp::Wrap("*".into(), "*".into())),
            MenuAction::Italic => self.pending_op = Some(crate::ide::EditOp::Wrap("_".into(), "_".into())),
            MenuAction::Underline => self.pending_op = Some(crate::ide::EditOp::Wrap("#underline[".into(), "]".into())),
            MenuAction::Formula => self.pending_op = Some(crate::ide::EditOp::Wrap("$ ".into(), " $".into())),
            MenuAction::ShowProblems => {
                self.side_tab = SideTab::Problems;
                self.settings.sidebar_visible = true;
            }
            MenuAction::Documentation => ctx.open_url(egui::OpenUrl::new_tab("https://typst.app/docs/")),
            MenuAction::OpenRecent(i) => {
                if let Some(p) = self.settings.recent.get(i).cloned() {
                    if p.exists() {
                        self.open_path(&p, ctx);
                    } else {
                        self.toast("Dieser Ordner existiert nicht mehr.", false);
                    }
                }
            }
        }
        if matches!(action, MenuAction::UiBigger | MenuAction::UiSmaller | MenuAction::UiReset | MenuAction::ToggleSidebar)
            && self.screenshot.is_none()
        {
            self.settings.save();
        }
    }

    /// Undo, cut, copy, paste … from a menu act on the focused text field (the editor by default),
    /// exactly as if the keys had been pressed there.
    fn apply_pending_edit(&mut self, ctx: &egui::Context) {
        let Some(action) = self.pending_edit.take() else { return };
        if ctx.memory(|m| m.focused().is_none()) && self.doc.is_some() {
            ctx.memory_mut(|m| m.request_focus(crate::ide::editor_id()));
        }
        let key = |key: egui::Key, modifiers: egui::Modifiers| egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers };
        let event = match action {
            MenuAction::Undo => Some(key(egui::Key::Z, egui::Modifiers::COMMAND)),
            MenuAction::Redo => Some(key(egui::Key::Z, egui::Modifiers::COMMAND | egui::Modifiers::SHIFT)),
            MenuAction::SelectAll => Some(key(egui::Key::A, egui::Modifiers::COMMAND)),
            MenuAction::Cut => Some(egui::Event::Cut),
            MenuAction::Copy => Some(egui::Event::Copy),
            MenuAction::Paste => arboard::Clipboard::new().ok().and_then(|mut c| c.get_text().ok()).map(egui::Event::Paste),
            _ => None,
        };
        if let Some(e) = event {
            ctx.input_mut(|i| i.events.push(e));
        }
    }

    /// Keyboard shortcuts when there is no system menu bar to deliver them (Linux, Windows).
    fn global_shortcuts(&mut self, ctx: &egui::Context) {
        let shift_cmd = egui::Modifiers::COMMAND | egui::Modifiers::SHIFT;
        let pressed = |m: egui::Modifiers, k: egui::Key| ctx.input_mut(|i| i.consume_key(m, k));
        let cmd = |k: egui::Key| pressed(egui::Modifiers::COMMAND, k);
        if pressed(shift_cmd, egui::Key::Plus) || pressed(shift_cmd, egui::Key::Equals) {
            self.run(MenuAction::UiBigger, ctx);
        }
        if pressed(shift_cmd, egui::Key::Minus) {
            self.run(MenuAction::UiSmaller, ctx);
        }
        if pressed(shift_cmd, egui::Key::Num0) {
            self.run(MenuAction::UiReset, ctx);
        }
        if pressed(shift_cmd, egui::Key::F) {
            self.run(MenuAction::SearchProject, ctx);
        }
        if cmd(egui::Key::Comma) {
            self.run(MenuAction::Settings, ctx);
        }
        if cmd(egui::Key::O) {
            self.run(MenuAction::OpenFolder, ctx);
        }
        if cmd(egui::Key::N) {
            self.run(MenuAction::NewFromTemplate, ctx);
        }
        if cmd(egui::Key::Q) {
            self.run(MenuAction::Quit, ctx);
        }
        if pressed(egui::Modifiers::NONE, egui::Key::F11) {
            self.run(MenuAction::Fullscreen, ctx);
        }
        if self.screen == Screen::Ide {
            if cmd(egui::Key::S) {
                self.run(MenuAction::Save, ctx);
            }
            if cmd(egui::Key::E) {
                self.run(MenuAction::ExportPdf, ctx);
            }
            if pressed(shift_cmd, egui::Key::W) {
                self.run(MenuAction::CloseProject, ctx);
            }
            if cmd(egui::Key::W) {
                self.run(MenuAction::CloseTab, ctx);
            }
            if pressed(egui::Modifiers::CTRL | egui::Modifiers::SHIFT, egui::Key::Tab) {
                self.run(MenuAction::PrevTab, ctx);
            }
            if pressed(egui::Modifiers::CTRL, egui::Key::Tab) {
                self.run(MenuAction::NextTab, ctx);
            }
            if pressed(egui::Modifiers::COMMAND | egui::Modifiers::ALT, egui::Key::S) {
                self.run(MenuAction::ToggleSidebar, ctx);
            }
            if cmd(egui::Key::Plus) || cmd(egui::Key::Equals) {
                self.run(MenuAction::PreviewZoomIn, ctx);
            }
            if cmd(egui::Key::Minus) {
                self.run(MenuAction::PreviewZoomOut, ctx);
            }
            if cmd(egui::Key::Num0) {
                self.run(MenuAction::PreviewFit, ctx);
            }
            if pressed(egui::Modifiers::COMMAND | egui::Modifiers::ALT, egui::Key::P) {
                self.run(MenuAction::TogglePreview, ctx);
            }
            if pressed(shift_cmd, egui::Key::M) {
                self.run(MenuAction::ShowProblems, ctx);
            }
        }
        if pressed(shift_cmd, egui::Key::O) {
            self.run(MenuAction::OpenFile, ctx);
        }
    }

    /// Size of the editor window: as the person last left it, else the designed size.
    pub fn ide_size(&self) -> egui::Vec2 {
        self.settings.window_size.map(|[w, h]| egui::vec2(w, h)).unwrap_or(egui::vec2(1600.0, 968.0))
    }

    /// Keep the first window inside the screen and remember the editor window's size.
    fn track_window(&mut self, ctx: &egui::Context) {
        if self.screenshot.is_some() {
            return;
        }
        let (inner, monitor) = ctx.input(|i| (i.viewport().inner_rect, i.viewport().monitor_size));
        let zoom = ctx.zoom_factor();
        if !self.size_checked {
            if let (Some(r), Some(m)) = (inner, monitor) {
                self.size_checked = true;
                let max = m * 0.92 / zoom;
                if r.width() > max.x || r.height() > max.y {
                    ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(r.size().min(max)));
                }
            }
        }
        let maximized = ctx.input(|i| i.viewport().maximized.unwrap_or(false) || i.viewport().fullscreen.unwrap_or(false));
        if self.screen == Screen::Ide && !maximized {
            if let Some(r) = inner {
                self.settings.window_size = Some([(r.width() * zoom).round(), (r.height() * zoom).round()]);
            }
        }
    }

    fn update_window_title(&mut self, ctx: &egui::Context) {
        let title = match (self.screen, self.project.as_ref(), self.doc.as_ref()) {
            (Screen::Ide, Some(p), Some(d)) => {
                let project = p.root.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                let dirty = if d.text != d.saved { "• " } else { "" };
                format!("{dirty}{} – {project} – Typisch", d.rel)
            }
            (Screen::Templates, _, _) => "Template auswählen – Typisch".to_string(),
            _ => "Typisch".to_string(),
        };
        if title != self.window_title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.window_title = title;
        }
    }

    pub fn error_count(&self) -> usize {
        self.visible_diags().filter(|d| d.error).count()
    }

    pub fn visible_diags(&self) -> impl Iterator<Item = &Diag> {
        let ignore = self.settings.ignore_library_warnings;
        self.diags.iter().filter(move |d| d.error || !(ignore && d.in_package))
    }

    pub fn run_search(&mut self) {
        self.search_hits.clear();
        self.search_for = self.search_query.clone();
        let q = self.search_query.to_lowercase();
        let Some(project) = self.project.as_ref() else { return };
        if q.is_empty() {
            return;
        }
        for f in project.files.iter().filter(|f| !f.is_dir) {
            let text = if self.doc.as_ref().map(|d| d.rel == f.rel).unwrap_or(false) {
                self.doc.as_ref().unwrap().text.clone()
            } else {
                match std::fs::read_to_string(project.root.join(&f.rel)) {
                    Ok(t) => t,
                    Err(_) => continue,
                }
            };
            for (i, line) in text.lines().enumerate() {
                let lower = line.to_lowercase();
                let mut from = 0;
                while let Some(pos) = lower[from..].find(&q) {
                    let start = from + pos;
                    let end = start + q.len();
                    from = end;
                    let before = lower[..start].chars().next_back();
                    let after = lower[end..].chars().next();
                    let is_word = |c: Option<char>| c.map(|c| c.is_alphanumeric() || c == '_').unwrap_or(false);
                    let ok = match self.search_mode {
                        SearchMode::Containing => true,
                        SearchMode::WholeWord => !is_word(before) && !is_word(after),
                        SearchMode::StartsWith => !is_word(before),
                    };
                    if ok && line.is_char_boundary(start) && line.is_char_boundary(end.min(line.len())) {
                        self.search_hits.push(SearchHit {
                            rel: f.rel.clone(),
                            line: i,
                            text: line.to_string(),
                            start,
                            len: q.len(),
                        });
                    }
                    if from >= lower.len() {
                        break;
                    }
                }
            }
        }
    }

    // ---------------------------------------------------------------- screenshot

    fn handle_screenshot(&mut self, ctx: &egui::Context) {
        let Some(path) = self.screenshot.clone() else { return };
        // Wait for the first preview (or give up after a while), then draw ~120 frames.
        let first_page_drawn = self.pages.first().map(|p| p.tex.is_some()).unwrap_or(false);
        let failed = self.compiled_tag > 0 && self.pages.is_empty();
        let ready = first_page_drawn || failed || self.screen != Screen::Ide || self.started.elapsed() > Duration::from_secs(40);
        if ready {
            self.frames += 1;
        }
        if self.frames >= 120 && !self.shot_requested {
            self.shot_requested = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
        }
        let shot = ctx.input(|i| {
            i.events.iter().find_map(|e| match e {
                egui::Event::Screenshot { image, .. } => Some(image.clone()),
                _ => None,
            })
        });
        if let Some(image) = shot {
            let [w, h] = image.size;
            let mut buf = image::RgbaImage::new(w as u32, h as u32);
            for (i, px) in image.pixels.iter().enumerate() {
                let [r, g, b, a] = px.to_srgba_unmultiplied();
                buf.put_pixel((i % w) as u32, (i / w) as u32, image::Rgba([r, g, b, a]));
            }
            if let Some(dir) = path.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            if let Err(e) = buf.save(&path) {
                eprintln!("could not save screenshot: {e}");
            }
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        ctx.request_repaint();
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let system_dark = ctx.system_theme().map(|t| t == egui::Theme::Dark);
        let theme = self.settings.effective_theme(system_dark);
        if crate::theme::use_theme(ctx, theme) {
            ctx.request_repaint();
        }
        let zoom = if self.screenshot.is_some() {
            // Fixed for screenshots; TYPISCH_SHOT_SCALE tries other interface sizes.
            std::env::var("TYPISCH_SHOT_SCALE").ok().and_then(|v| v.parse().ok()).unwrap_or(1.0)
        } else {
            (self.settings.ui_scale * system_text_scale()).clamp(0.6, 3.0)
        };
        if (ctx.zoom_factor() - zoom).abs() > 0.001 {
            ctx.set_zoom_factor(zoom);
        }
        self.update_window_title(ctx);
        self.track_window(ctx);
        // Menu choices: the macOS menu bar reports them here; elsewhere the bar is drawn below.
        let has_project = self.screen == Screen::Ide && self.project.is_some();
        let recent = self.settings.recent.clone();
        let chosen = self.native_menu.as_mut().map(|m| m.poll(&recent, has_project)).unwrap_or_default();
        for action in chosen {
            self.run(action, ctx);
        }
        self.apply_pending_edit(ctx);
        let dropped: Vec<PathBuf> = ctx.input(|i| i.raw.dropped_files.iter().filter_map(|f| f.path.clone()).collect());
        if let Some(path) = dropped.into_iter().next() {
            if path.is_dir() || path.extension().map(|e| e == "typ").unwrap_or(false) {
                self.open_path(&path, ctx);
            } else {
                self.toast("Bitte einen Ordner oder eine .typ-Datei hierher ziehen.", false);
            }
        }
        self.poll_compiler(ctx);

        // Debounced compile & autosave.
        if let Some(due) = self.compile_due {
            if Instant::now() >= due {
                self.compile_due = None;
                self.send_compile();
            } else {
                ctx.request_repaint_after(due - Instant::now());
            }
        }
        if let Some(t) = self.doc.as_ref().and_then(|d| d.edited_at) {
            let wait = Duration::from_millis(1200);
            if t.elapsed() >= wait {
                if self.screenshot.is_none() {
                    self.save_now();
                }
                if let Some(d) = self.doc.as_mut() {
                    d.edited_at = None;
                }
            } else {
                ctx.request_repaint_after(wait - t.elapsed());
            }
        }

        // Without a system menu bar the window gets its own, and handles the shortcuts itself
        // (with the macOS menu bar, the system delivers them as menu choices).
        if self.native_menu.is_none() {
            crate::menu::window_menu_bar(self, ctx);
            self.global_shortcuts(ctx);
        }

        match self.screen {
            Screen::Welcome => crate::screens::welcome(self, ctx),
            Screen::Templates => crate::screens::templates(self, ctx),
            Screen::Ide => crate::ide::show(self, ctx),
        }
        if self.settings_open {
            crate::screens::settings_window(self, ctx);
        }

        show_toast(self, ctx);
        self.handle_screenshot(ctx);
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        if self.screenshot.is_none() {
            self.save_now();
            self.settings.save();
        }
    }
}

fn show_toast(app: &mut App, ctx: &egui::Context) {
    let Some((msg, at, ok)) = app.toast.clone() else { return };
    if at.elapsed() > Duration::from_secs(5) {
        app.toast = None;
        return;
    }
    ctx.request_repaint_after(Duration::from_millis(250));
    egui::Area::new(egui::Id::new("toast"))
        .anchor(egui::Align2::CENTER_BOTTOM, egui::vec2(0.0, -48.0))
        .order(egui::Order::Foreground)
        .show(ctx, |ui| {
            egui::Frame::new()
                .fill(if ok { Color32::from_rgb(38, 64, 46) } else { Color32::from_rgb(90, 32, 34) })
                .corner_radius(crate::widgets::RADIUS_L)
                .inner_margin(egui::Margin::symmetric(14, 9))
                .show(ui, |ui| {
                    ui.label(egui::RichText::new(msg).color(Color32::WHITE));
                });
        });
}

pub fn scan_files(root: &Path) -> Vec<FileEntry> {
    fn walk(root: &Path, dir: &Path, depth: usize, out: &mut Vec<FileEntry>) {
        if depth > 6 || out.len() > 2000 {
            return;
        }
        let Ok(rd) = std::fs::read_dir(dir) else { return };
        let mut entries: Vec<_> = rd.flatten().collect();
        entries.sort_by_key(|e| (!e.path().is_dir(), e.file_name().to_string_lossy().to_lowercase()));
        for e in entries {
            let name = e.file_name().to_string_lossy().to_string();
            if name.starts_with('.') || name == "target" || name == "node_modules" {
                continue;
            }
            let path = e.path();
            let rel = path.strip_prefix(root).unwrap_or(&path).to_string_lossy().replace('\\', "/");
            let is_dir = path.is_dir();
            out.push(FileEntry { rel, name, depth, is_dir });
            if is_dir {
                walk(root, &path, depth + 1, out);
            }
        }
    }
    let mut out = Vec::new();
    walk(root, root, 0, &mut out);
    out
}

pub fn tilde(path: &Path) -> String {
    if let Some(home) = dirs::home_dir() {
        if let Ok(rest) = path.strip_prefix(&home) {
            return format!("~/{}", rest.display());
        }
    }
    path.display().to_string()
}

// ---------------------------------------------------------------- look & feel

/// A font file (and face index) the desktop uses for `pattern`, asked from fontconfig.
#[cfg(all(unix, not(target_os = "macos")))]
fn fontconfig_file(pattern: &str) -> Option<(String, u32)> {
    let out = std::process::Command::new("fc-match").args(["-f", "%{file}\n%{index}", pattern]).output().ok()?;
    let text = String::from_utf8(out.stdout).ok()?;
    let mut lines = text.lines();
    let file = lines.next()?.trim().to_string();
    let index = lines.next().and_then(|i| i.trim().parse().ok()).unwrap_or(0);
    let lower = file.to_lowercase();
    (lower.ends_with(".ttf") || lower.ends_with(".otf") || lower.ends_with(".ttc")).then_some((file, index))
}

/// The GNOME/GTK interface font family names, when the desktop has them configured.
#[cfg(all(unix, not(target_os = "macos")))]
fn gsettings_string(key: &str) -> Option<String> {
    let out = std::process::Command::new("gsettings").args(["get", "org.gnome.desktop.interface", key]).output().ok()?;
    let v = String::from_utf8(out.stdout).ok()?;
    let v = v.trim().trim_matches('\'').to_string();
    (!v.is_empty()).then_some(v)
}

/// The desktop's "large text" factor (GNOME accessibility setting); 1.0 elsewhere.
pub fn system_text_scale() -> f32 {
    static SCALE: std::sync::OnceLock<f32> = std::sync::OnceLock::new();
    *SCALE.get_or_init(|| {
        #[cfg(all(unix, not(target_os = "macos")))]
        {
            if let Some(v) = gsettings_string("text-scaling-factor").and_then(|v| v.parse::<f32>().ok()) {
                return v.clamp(0.5, 3.0);
            }
        }
        1.0
    })
}

/// (regular, bold, monospace) system fonts as (bytes, face index).
fn system_fonts() -> (Option<(Vec<u8>, u32)>, Option<(Vec<u8>, u32)>, Option<(Vec<u8>, u32)>) {
    let load = |p: &str, i: u32| std::fs::read(p).ok().map(|b| (b, i));
    #[cfg(target_os = "macos")]
    {
        return (
            load("/System/Library/Fonts/SFNS.ttf", 0),
            load("/System/Library/Fonts/HelveticaNeue.ttc", 1),
            load("/System/Library/Fonts/SFNSMono.ttf", 0),
        );
    }
    #[cfg(target_os = "windows")]
    {
        let dir = std::env::var("WINDIR").unwrap_or_else(|_| "C:\\Windows".into());
        return (
            load(&format!("{dir}\\Fonts\\segoeui.ttf"), 0),
            load(&format!("{dir}\\Fonts\\segoeuisb.ttf"), 0).or_else(|| load(&format!("{dir}\\Fonts\\segoeuib.ttf"), 0)),
            load(&format!("{dir}\\Fonts\\consola.ttf"), 0),
        );
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        // Strip the size from GNOME's "Cantarell 11" style names.
        let family = |key: &str, fallback: &str| {
            gsettings_string(key)
                .map(|v| v.rsplit_once(' ').map(|(f, size)| if size.parse::<f32>().is_ok() { f.to_string() } else { v.clone() }).unwrap_or(v))
                .unwrap_or_else(|| fallback.to_string())
        };
        let ui = family("font-name", "sans-serif");
        let mono = family("monospace-font-name", "monospace");
        let get = |pat: String| fontconfig_file(&pat).and_then(|(f, i)| load(&f, i));
        let regular = get(ui.clone());
        let bold = get(format!("{ui}:weight=200")).filter(|b| regular.as_ref().map(|r| r.0.len() != b.0.len()).unwrap_or(true));
        return (regular, bold, get(mono));
    }
    #[allow(unreachable_code)]
    (None, None, None)
}

fn install_fonts(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    let try_load = |paths: &[&str]| -> Option<Vec<u8>> { paths.iter().find_map(|p| std::fs::read(p).ok()) };
    let (sys_ui, sys_bold, sys_mono) = system_fonts();

    let ui_font = sys_ui.or_else(|| {
        try_load(&[
            "/usr/share/fonts/truetype/noto/NotoSans-Regular.ttf",
            "/usr/share/fonts/noto/NotoSans-Regular.ttf",
            "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
            "/usr/share/fonts/TTF/DejaVuSans.ttf",
        ])
        .map(|b| (b, 0))
    });
    let ui_bold = sys_bold.or_else(|| {
        try_load(&[
            "/usr/share/fonts/truetype/noto/NotoSans-SemiBold.ttf",
            "/usr/share/fonts/noto/NotoSans-SemiBold.ttf",
            "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf",
            "/usr/share/fonts/TTF/DejaVuSans-Bold.ttf",
        ])
        .map(|b| (b, 0))
    });
    let mono = sys_mono;

    if let Some((bytes, index)) = ui_font {
        let mut data = FontData::from_owned(bytes);
        data.index = index;
        fonts.font_data.insert("ui".into(), data.into());
        fonts.families.get_mut(&FontFamily::Proportional).unwrap().insert(0, "ui".into());
    }
    let bold_family = FontFamily::Name("bold".into());
    let mut bold_list = Vec::new();
    if let Some((bytes, index)) = ui_bold {
        let mut data = FontData::from_owned(bytes);
        data.index = index;
        fonts.font_data.insert("ui-bold".into(), data.into());
        bold_list.push("ui-bold".to_string());
    }
    // Bundled Typst fonts: DejaVu Sans Mono (code), New Computer Modern Math (symbols).
    let bundled: Vec<&'static [u8]> = typst_assets::fonts().collect();
    for (name, file_index) in [("newcm-math", 8usize), ("dejavu-mono", 16usize)] {
        if let Some(bytes) = bundled.get(file_index) {
            fonts.font_data.insert(name.into(), FontData::from_static(bytes).into());
        }
    }
    let mono_list = fonts.families.get_mut(&FontFamily::Monospace).unwrap();
    if let Some((bytes, index)) = mono {
        let mut data = FontData::from_owned(bytes);
        data.index = index;
        fonts.font_data.insert("sf-mono".into(), data.into());
        mono_list.insert(0, "sf-mono".into());
    } else if fonts.font_data.contains_key("dejavu-mono") {
        mono_list.insert(0, "dejavu-mono".into());
    }
    let mut dejavu_family = vec!["dejavu-mono".to_string()];
    for fam in [FontFamily::Proportional, FontFamily::Monospace] {
        let list = fonts.families.get_mut(&fam).unwrap();
        for f in ["dejavu-mono", "newcm-math"] {
            if fonts.font_data.contains_key(f) && !list.iter().any(|x| x == f) {
                list.push(f.into());
            }
        }
    }
    if fonts.font_data.contains_key("newcm-math") {
        dejavu_family.push("newcm-math".into());
    }
    fonts.families.insert(FontFamily::Name("dejavu".into()), dejavu_family);
    let prop = fonts.families[&FontFamily::Proportional].clone();
    bold_list.extend(prop);
    fonts.families.insert(bold_family, bold_list);
    ctx.set_fonts(fonts);
}

pub fn bold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("bold".into()))
}

fn egui_extras_style(ctx: &egui::Context) {
    ctx.set_theme(egui::Theme::Dark);
    ctx.all_styles_mut(|style| {
        style.text_styles = [
            (TextStyle::Small, FontId::proportional(11.0)),
            (TextStyle::Body, FontId::proportional(13.5)),
            (TextStyle::Button, FontId::proportional(13.5)),
            (TextStyle::Heading, FontId::proportional(20.0)),
            (TextStyle::Monospace, FontId::monospace(13.0)),
        ]
        .into();
        // Colours and corners come from theme::Palette::apply, whenever the theme changes.
        style.spacing.item_spacing = egui::vec2(8.0, 6.0);
        style.spacing.button_padding = egui::vec2(10.0, 4.0);
        style.spacing.interact_size.y = 24.0;
        style.spacing.scroll = egui::style::ScrollStyle::floating();
        style.spacing.scroll.floating_width = 3.0;
        style.spacing.scroll.bar_width = 8.0;
        style.spacing.menu_margin = egui::Margin::symmetric(6, 6);
        style.interaction.selectable_labels = false;
        style.interaction.tooltip_delay = 0.6;
        style.animation_time = 0.12;
    });
}
