//! Editor colour themes and persisted settings.

use eframe::egui::Color32;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Copy)]
pub struct Theme {
    pub name: &'static str,
    pub bg: Color32,
    pub gutter_bg: Color32,
    pub current_line: Color32,
    pub text: Color32,
    pub line_number: Color32,
    pub line_number_current: Color32,
    pub comment: Color32,
    pub keyword: Color32,
    pub string: Color32,
    pub function: Color32,
    pub number: Color32,
    pub heading: Color32,
    pub markup: Color32,
    pub math: Color32,
    pub label: Color32,
    pub selection: Color32,
    pub cursor: Color32,
    pub dark: bool,
    /// Highlight colour for buttons, selected tabs and toggles across the app.
    pub accent: Color32,
    /// The five dots shown in the theme picker.
    pub swatch: [Color32; 5],
}

const fn c(hex: u32) -> Color32 {
    Color32::from_rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

pub const THEMES: [Theme; 7] = [
    Theme {
        name: "Xcode Default",
        bg: c(0x1f1f24),
        gutter_bg: c(0x1f1f24),
        current_line: c(0x2a2b31),
        text: c(0xdfdfe0),
        line_number: c(0x6c6c74),
        line_number_current: c(0xdfdfe0),
        comment: c(0x7f8c98),
        keyword: c(0xfc5fa3),
        string: c(0xfc6a5d),
        function: c(0x41a1c0),
        number: c(0xd0bf69),
        heading: c(0xffffff),
        markup: c(0xa167e6),
        math: c(0x67b7a4),
        label: c(0xd0bf69),
        selection: c(0x515b70),
        cursor: c(0x2f8cf5),
        dark: true,
        accent: c(0x2f8cf5),
        swatch: [c(0xfc5fa3), c(0xfc6a5d), c(0x41a1c0), c(0xd0bf69), c(0xdfdfe0)],
    },
    Theme {
        name: "Catppuccin Mocha",
        bg: c(0x1e1e2e),
        gutter_bg: c(0x1e1e2e),
        current_line: c(0x2a2b3c),
        text: c(0xcdd6f4),
        line_number: c(0x6c7086),
        line_number_current: c(0xcdd6f4),
        comment: c(0x7f849c),
        keyword: c(0xf38ba8),
        string: c(0xa6e3a1),
        function: c(0x89b4fa),
        number: c(0xfab387),
        heading: c(0xf5c2e7),
        markup: c(0xcba6f7),
        math: c(0x94e2d5),
        label: c(0xfab387),
        selection: c(0x45475a),
        cursor: c(0xf5e0dc),
        dark: true,
        accent: c(0x89b4fa),
        swatch: [c(0xf38ba8), c(0xa6e3a1), c(0xcba6f7), c(0xfab387), c(0x89b4fa)],
    },
    Theme {
        name: "Typst Default",
        bg: c(0x171717),
        gutter_bg: c(0x171717),
        current_line: c(0x23252b),
        text: c(0xe6e6e6),
        line_number: c(0x6b6b6b),
        line_number_current: c(0xe6e6e6),
        comment: c(0x8a8a8f),
        keyword: c(0xff4f6a),
        string: c(0x4cd04a),
        function: c(0x7c8cff),
        number: c(0xe6e6e6),
        heading: c(0xffffff),
        markup: c(0xe6e6e6),
        math: c(0x43c4c4),
        label: c(0x4cd04a),
        selection: c(0x3a4a66),
        cursor: c(0x2f8cf5),
        dark: true,
        accent: c(0x2f8cf5),
        swatch: [c(0xff4f6a), c(0x3fc23c), c(0x6a7cff), c(0xe0e0e0), c(0xe0e0e0)],
    },
    Theme {
        name: "Atom One",
        bg: c(0x21252b),
        gutter_bg: c(0x21252b),
        current_line: c(0x2c313a),
        text: c(0xabb2bf),
        line_number: c(0x5c6370),
        line_number_current: c(0xabb2bf),
        comment: c(0x5c6370),
        keyword: c(0xc678dd),
        string: c(0x98c379),
        function: c(0x61afef),
        number: c(0xd19a66),
        heading: c(0xe06c75),
        markup: c(0xe5c07b),
        math: c(0x56b6c2),
        label: c(0xd19a66),
        selection: c(0x3e4451),
        cursor: c(0x528bff),
        dark: true,
        accent: c(0x61afef),
        swatch: [c(0xc678dd), c(0x98c379), c(0x61afef), c(0xd19a66), c(0xe06c75)],
    },
    Theme {
        name: "Catppuccin Latte",
        bg: c(0xeff1f5),
        gutter_bg: c(0xeff1f5),
        current_line: c(0xe3e6ee),
        text: c(0x4c4f69),
        line_number: c(0x9ca0b0),
        line_number_current: c(0x4c4f69),
        comment: c(0x8c8fa1),
        keyword: c(0xd20f39),
        string: c(0x40a02b),
        function: c(0x1e66f5),
        number: c(0xfe640b),
        heading: c(0x8839ef),
        markup: c(0x8839ef),
        math: c(0x179299),
        label: c(0xfe640b),
        selection: c(0xccd0da),
        cursor: c(0xdc8a78),
        dark: false,
        accent: c(0x1e66f5),
        swatch: [c(0xd20f39), c(0x40a02b), c(0x8839ef), c(0xfe640b), c(0x1e66f5)],
    },
    Theme {
        name: "Gruvbox",
        bg: c(0x1d2021),
        gutter_bg: c(0x1d2021),
        current_line: c(0x282828),
        text: c(0xebdbb2),
        line_number: c(0x665c54),
        line_number_current: c(0xebdbb2),
        comment: c(0x928374),
        keyword: c(0xfb4934),
        string: c(0xb8bb26),
        function: c(0x8ec07c),
        number: c(0xd3869b),
        heading: c(0xfabd2f),
        markup: c(0xfe8019),
        math: c(0x83a598),
        label: c(0xd3869b),
        selection: c(0x504945),
        cursor: c(0xebdbb2),
        dark: true,
        accent: c(0xd79921),
        swatch: [c(0xfb4934), c(0xb8bb26), c(0x689d6a), c(0xd3869b), c(0xd3869b)],
    },
    Theme {
        name: "Nord",
        bg: c(0x232731),
        gutter_bg: c(0x232731),
        current_line: c(0x2e3440),
        text: c(0xd8dee9),
        line_number: c(0x4c566a),
        line_number_current: c(0xd8dee9),
        comment: c(0x616e88),
        keyword: c(0x81a1c1),
        string: c(0xa3be8c),
        function: c(0x88c0d0),
        number: c(0xb48ead),
        heading: c(0x8fbcbb),
        markup: c(0xebcb8b),
        math: c(0x88c0d0),
        label: c(0xb48ead),
        selection: c(0x434c5e),
        cursor: c(0xd8dee9),
        dark: true,
        accent: c(0x88c0d0),
        swatch: [c(0x81a1c1), c(0xa3be8c), c(0x88c0d0), c(0xb48ead), c(0x81a1c1)],
    },
];

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Debug)]
pub enum AutoClose {
    Immediately,
    BeforeWhitespace,
    Never,
}

impl AutoClose {
    pub fn label(self) -> &'static str {
        match self {
            AutoClose::Immediately => "Sofort schließen",
            AutoClose::BeforeWhitespace => "Nur vor Leerraum",
            AutoClose::Never => "Nie",
        }
    }
}

#[derive(Clone, Serialize, Deserialize, Debug)]
#[serde(default)]
pub struct Settings {
    pub theme: usize,
    pub font_size: f32,
    pub monospace_font: String,
    pub animated_cursor: bool,
    pub rounded_selection: bool,
    pub line_numbers: bool,
    pub word_count: bool,
    pub spell_check: bool,
    pub auto_close: AutoClose,
    pub vim: bool,
    pub relative_line_numbers: bool,
    pub editor_left: bool,
    pub ripple: bool,
    pub ignore_library_warnings: bool,
    pub recent: Vec<PathBuf>,
    pub local_namespaces: Vec<(String, PathBuf)>,
    pub sidebar_visible: bool,
    /// Switch between `dark_theme` and `light_theme` with the system's appearance.
    pub follow_system: bool,
    pub dark_theme: usize,
    pub light_theme: usize,
    /// Interface size on top of the display's own scale (1.0 = normal).
    pub ui_scale: f32,
    pub sidebar_width: f32,
    /// Share of the space the editor takes next to the preview.
    pub editor_ratio: f32,
    pub window_size: Option<[f32; 2]>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: 2,
            font_size: 13.0,
            monospace_font: "System Monospaced".into(),
            animated_cursor: true,
            rounded_selection: true,
            line_numbers: true,
            word_count: true,
            spell_check: false,
            auto_close: AutoClose::Immediately,
            vim: false,
            relative_line_numbers: false,
            editor_left: true,
            ripple: true,
            ignore_library_warnings: true,
            recent: Vec::new(),
            local_namespaces: Vec::new(),
            sidebar_visible: true,
            follow_system: true,
            dark_theme: 2,
            light_theme: 4,
            ui_scale: 1.0,
            sidebar_width: 258.0,
            editor_ratio: 0.4955,
            window_size: None,
        }
    }
}

impl Settings {
    fn path() -> Option<PathBuf> {
        dirs::config_dir().map(|d| d.join("typisch").join("settings.json"))
    }

    pub fn load() -> Self {
        Self::path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) {
        if let Some(p) = Self::path() {
            if let Some(dir) = p.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            if let Ok(s) = serde_json::to_string_pretty(self) {
                let _ = std::fs::write(p, s);
            }
        }
    }

    /// The theme in use right now (with "follow system", light or dark as the system is).
    pub fn effective_theme(&self, system_dark: Option<bool>) -> usize {
        let pick = self.theme.min(THEMES.len() - 1);
        if !self.follow_system {
            return pick;
        }
        match system_dark {
            Some(true) => self.dark_theme.min(THEMES.len() - 1),
            Some(false) => self.light_theme.min(THEMES.len() - 1),
            None => pick,
        }
    }

    /// Choosing a theme also makes it the one used for its brightness when following the system.
    pub fn choose_theme(&mut self, index: usize) {
        self.theme = index;
        if THEMES[index].dark {
            self.dark_theme = index;
        } else {
            self.light_theme = index;
        }
    }

    pub fn theme(&self) -> &'static Theme {
        &THEMES[current_theme_index().unwrap_or(self.theme).min(THEMES.len() - 1)]
    }

    pub fn add_recent(&mut self, path: PathBuf) {
        self.recent.retain(|p| p != &path);
        self.recent.insert(0, path);
        self.recent.truncate(10);
    }
}

// ---------------------------------------------------------------------------
// Whole-app palette, derived from the chosen editor theme.

fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let f = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round().clamp(0.0, 255.0) as u8;
    Color32::from_rgb(f(a.r(), b.r()), f(a.g(), b.g()), f(a.b(), b.b()))
}

fn luminance(c: Color32) -> f32 {
    (0.2126 * c.r() as f32 + 0.7152 * c.g() as f32 + 0.0722 * c.b() as f32) / 255.0
}

/// Every colour the window chrome uses (sidebar, bars, preview, dialogs, buttons).
#[derive(Clone, Copy, Debug)]
pub struct Palette {
    pub dark: bool,
    pub editor_bg: Color32,
    pub sidebar: Color32,
    pub topbar: Color32,
    pub bottombar: Color32,
    pub preview_bg: Color32,
    pub preview_bar: Color32,
    pub pill: Color32,
    pub pill_selected: Color32,
    pub pill_hover: Color32,
    pub row_selected: Color32,
    pub preview_fg: Color32,
    pub window: Color32,
    pub group: Color32,
    pub surface: Color32,
    pub surface_hover: Color32,
    pub hover: Color32,
    pub selected: Color32,
    pub badge: Color32,
    pub separator: Color32,
    pub text: Color32,
    pub text_soft: Color32,
    pub dim: Color32,
    pub faint: Color32,
    pub accent: Color32,
    pub on_accent: Color32,
    pub selection: Color32,
    pub error: Color32,
    pub error_line: Color32,
    pub warning: Color32,
    pub file_typ: Color32,
    pub file_image: Color32,
}

impl Palette {
    pub fn from_theme(t: &Theme) -> Self {
        let bg = t.bg;
        // For light themes the chrome gets slightly darker than the editor, for dark ones lighter.
        let ink = if t.dark { t.text } else { mix(t.text, Color32::BLACK, 0.2) };
        let m = |k: f32| mix(bg, ink, k);
        let error = if t.dark { Color32::from_rgb(0xe8, 0x2f, 0x3a) } else { Color32::from_rgb(0xd2, 0x0f, 0x39) };
        let on_accent = if luminance(t.accent) > 0.62 { mix(bg, Color32::BLACK, 0.6) } else { Color32::WHITE };
        Self {
            dark: t.dark,
            editor_bg: bg,
            sidebar: m(0.058),
            topbar: m(0.034),
            bottombar: m(0.07),
            preview_bg: m(0.025),
            preview_bar: m(0.15),
            pill: m(0.2),
            pill_selected: m(0.3),
            pill_hover: m(0.25),
            row_selected: if t.dark { m(0.47) } else { m(0.17) },
            preview_fg: mix(bg, t.text, 0.95),
            window: m(0.034),
            group: m(0.065),
            surface: m(0.085),
            surface_hover: m(0.14),
            hover: m(0.09),
            selected: m(0.19),
            badge: m(0.14),
            separator: m(0.15),
            text: mix(bg, t.text, 1.0),
            text_soft: mix(bg, t.text, 0.86),
            dim: mix(bg, t.text, 0.58),
            faint: mix(bg, t.text, 0.4),
            accent: t.accent,
            on_accent,
            selection: t.selection,
            error,
            error_line: mix(bg, error, if t.dark { 0.3 } else { 0.18 }),
            warning: if t.dark { Color32::from_rgb(0xf0, 0xb0, 0x30) } else { Color32::from_rgb(0xc0, 0x7a, 0x00) },
            file_typ: if t.dark { Color32::from_rgb(0x3d, 0xb8, 0xc9) } else { Color32::from_rgb(0x17, 0x92, 0x99) },
            file_image: if t.dark { Color32::from_rgb(0xcf, 0x2f, 0xe6) } else { Color32::from_rgb(0xa0, 0x1f, 0xc0) },
        }
    }

    /// egui's own widget colours (text fields, buttons, menus, sliders, scroll bars) for this palette.
    pub fn apply(&self, ctx: &eframe::egui::Context) {
        use eframe::egui::{Stroke, Visuals};
        let p = *self;
        ctx.all_styles_mut(|style| {
            let mut v = if p.dark { Visuals::dark() } else { Visuals::light() };
            v.override_text_color = None;
            v.panel_fill = p.topbar;
            v.window_fill = p.window;
            v.extreme_bg_color = mix(p.editor_bg, p.surface, 0.5);
            v.faint_bg_color = p.group;
            v.code_bg_color = p.surface;
            v.window_corner_radius = 12.into();
            v.window_stroke = Stroke::new(1.0_f32, p.separator);
            // No drop shadows: dialogs and menus get a thin border instead.
            v.window_shadow = eframe::egui::Shadow::NONE;
            v.popup_shadow = eframe::egui::Shadow::NONE;
            v.selection.bg_fill = p.selection;
            v.selection.stroke = Stroke::new(1.0_f32, p.text);
            v.hyperlink_color = p.accent;
            v.text_cursor.stroke.color = p.accent;
            for (w, fill, fg) in [
                (&mut v.widgets.noninteractive, p.group, p.text_soft),
                (&mut v.widgets.inactive, p.surface, p.text),
                (&mut v.widgets.hovered, p.surface_hover, p.text),
                (&mut v.widgets.active, p.selected, p.text),
                (&mut v.widgets.open, p.surface_hover, p.text),
            ] {
                w.bg_fill = fill;
                w.weak_bg_fill = fill;
                w.fg_stroke.color = fg;
                w.corner_radius = 6.into();
            }
            v.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, p.separator);
            v.widgets.inactive.bg_stroke = Stroke::NONE;
            v.widgets.hovered.bg_stroke = Stroke::new(1.0_f32, p.separator);
            v.widgets.active.bg_stroke = Stroke::new(1.0_f32, p.accent);
            // Slider rails and filled parts use the accent.
            v.widgets.inactive.bg_fill = p.selected;
            v.slider_trailing_fill = true;
            style.visuals = v;
        });
    }
}

thread_local! {
    static PALETTE: std::cell::Cell<Option<(usize, Palette)>> = const { std::cell::Cell::new(None) };
}

/// Which theme the whole window is using right now.
pub fn current_theme_index() -> Option<usize> {
    PALETTE.with(|p| p.get().map(|(i, _)| i))
}

/// The palette of the current theme (set once per frame by the app).
pub fn pal() -> Palette {
    PALETTE.with(|p| p.get().map(|(_, pal)| pal)).unwrap_or_else(|| Palette::from_theme(&THEMES[2]))
}

/// Switch the whole app to a theme; returns true when it changed.
pub fn use_theme(ctx: &eframe::egui::Context, index: usize) -> bool {
    let index = index.min(THEMES.len() - 1);
    let current = PALETTE.with(|p| p.get().map(|(i, _)| i));
    if current == Some(index) {
        return false;
    }
    let palette = Palette::from_theme(&THEMES[index]);
    PALETTE.with(|p| p.set(Some((index, palette))));
    palette.apply(ctx);
    true
}
