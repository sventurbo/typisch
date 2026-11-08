//! The Typisch logo, described once: a tile with a page, a serif "T" and two text lines.
//! The welcome screen paints these shapes with egui; the app icon (window, taskbar,
//! macOS .icns, `--export-icon`) rasterizes the same shapes into pixels.

use eframe::egui::{self, Color32, Rect, vec2};

// The only three colors in the logo: flat fills, no gradients, no shadows.
const TILE: [u8; 3] = [0x26, 0x93, 0xbe];
const PAPER: [u8; 3] = [0xf7, 0xf9, 0xfa];
const INK: [u8; 3] = [0x1d, 0x4f, 0x7a];

/// A filled rounded rectangle in tile units (0..1 across the tile). Later parts sit on top.
struct Part {
    l: f32,
    t: f32,
    r: f32,
    b: f32,
    radius: f32,
    color: [u8; 3],
}

const fn part(l: f32, t: f32, r: f32, b: f32, radius: f32, color: [u8; 3]) -> Part {
    Part { l, t, r, b, radius, color }
}

const PARTS: [Part; 9] = [
    part(0.0, 0.0, 1.0, 1.0, 0.225, TILE),
    // Page
    part(0.25, 0.19, 0.75, 0.81, 0.045, PAPER),
    // "T": bar, serifs at both bar ends, stem, foot serif
    part(0.34, 0.28, 0.66, 0.335, 0.0, INK),
    part(0.34, 0.28, 0.37, 0.375, 0.0, INK),
    part(0.63, 0.28, 0.66, 0.375, 0.0, INK),
    part(0.468, 0.28, 0.532, 0.585, 0.0, INK),
    part(0.42, 0.555, 0.58, 0.585, 0.0, INK),
    // Text lines
    part(0.34, 0.66, 0.66, 0.69, 0.015, TILE),
    part(0.34, 0.72, 0.56, 0.75, 0.015, TILE),
];

impl Part {
    fn contains(&self, u: f32, v: f32) -> bool {
        if u < self.l || u > self.r || v < self.t || v > self.b {
            return false;
        }
        // Distance to the nearest point of the inner (corner-free) rectangle.
        let cx = u.max(self.l + self.radius).min(self.r - self.radius);
        let cy = v.max(self.t + self.radius).min(self.b - self.radius);
        (u - cx).powi(2) + (v - cy).powi(2) <= self.radius * self.radius
    }
}

/// Paints the logo so its tile fills `tile` (meant to be square).
pub fn paint(painter: &egui::Painter, tile: Rect) {
    let s = tile.width().min(tile.height());
    for p in &PARTS {
        let rect = Rect::from_min_max(tile.lerp_inside(vec2(p.l, p.t)), tile.lerp_inside(vec2(p.r, p.b)));
        painter.rect_filled(rect, p.radius * s, Color32::from_rgb(p.color[0], p.color[1], p.color[2]));
    }
}

/// The logo as an SVG document: the tile fills a 1000 × 1000 canvas, without the icon-grid margin.
pub fn svg() -> String {
    // Tile units → canvas units, with at most one decimal.
    let num = |v: f32| {
        let s = format!("{:.1}", v * 1000.0);
        s.strip_suffix(".0").map(str::to_owned).unwrap_or(s)
    };
    let mut out = String::from(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 1000 1000\" width=\"1000\" height=\"1000\">\n  <title>Typisch</title>\n",
    );
    for p in &PARTS {
        let [r, g, b] = p.color;
        let corner = if p.radius > 0.0 { format!(" rx=\"{}\"", num(p.radius)) } else { String::new() };
        out += &format!(
            "  <rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\"{corner} fill=\"#{r:02x}{g:02x}{b:02x}\"/>\n",
            num(p.l),
            num(p.t),
            num(p.r - p.l),
            num(p.b - p.t),
        );
    }
    out + "</svg>\n"
}

/// The app icon as pixels: the logo on the macOS icon grid (an 824 px tile centered on 1024 px).
pub fn icon_rgba(size: u32) -> egui::IconData {
    let n = size as f32;
    let inset = n * 100.0 / 1024.0;
    let tile = n - 2.0 * inset;
    // Supersampling per pixel gives smooth edges, also for the thin parts at 16 px.
    let ss = if size <= 256 { 8 } else { 4 };
    let samples = (ss * ss) as f32;
    let mut rgba = vec![0u8; (size * size * 4) as usize];
    for y in 0..size {
        for x in 0..size {
            let mut sum = [0.0f32; 3];
            let mut hits = 0.0f32;
            for sy in 0..ss {
                for sx in 0..ss {
                    let u = (x as f32 + (sx as f32 + 0.5) / ss as f32 - inset) / tile;
                    let v = (y as f32 + (sy as f32 + 0.5) / ss as f32 - inset) / tile;
                    if let Some(p) = PARTS.iter().rev().find(|p| p.contains(u, v)) {
                        for (s, c) in sum.iter_mut().zip(p.color) {
                            *s += c as f32;
                        }
                        hits += 1.0;
                    }
                }
            }
            if hits > 0.0 {
                let i = ((y * size + x) * 4) as usize;
                for c in 0..3 {
                    rgba[i + c] = (sum[c] / hits).round() as u8;
                }
                rgba[i + 3] = (hits / samples * 255.0).round() as u8;
            }
        }
    }
    egui::IconData { rgba, width: size, height: size }
}
