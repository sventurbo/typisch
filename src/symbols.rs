//! Symbol palette: the full Typst `sym` table, plus a tiny drawn-symbol recogniser.

use std::sync::OnceLock;

use eframe::egui::{Pos2, Vec2, pos2};

pub struct Sym {
    pub name: String,
    pub glyph: &'static str,
}

pub fn all() -> &'static [Sym] {
    static LIST: OnceLock<Vec<Sym>> = OnceLock::new();
    LIST.get_or_init(|| {
        let mut out = Vec::new();
        collect(codex::SYM, "", &mut out);
        out
    })
}

fn collect(module: codex::Module, prefix: &str, out: &mut Vec<Sym>) {
    for (name, binding) in module.iter() {
        if binding.deprecation.is_some() {
            continue;
        }
        let full = if prefix.is_empty() { name.to_string() } else { format!("{prefix}.{name}") };
        match binding.def {
            codex::Def::Symbol(sym) => {
                for (mods, glyph, deprecated) in sym.variants() {
                    if deprecated.is_some() {
                        continue;
                    }
                    let n = if mods.is_empty() { full.clone() } else { format!("{full}.{}", mods.as_str()) };
                    out.push(Sym { name: n, glyph });
                }
            }
            codex::Def::Module(m) => collect(m, &full, out),
        }
    }
}

// ---------------------------------------------------------------------------
// Drawing recogniser: a small "$1"-style template matcher.

const N: usize = 48;

struct Template {
    name: &'static str,
    glyph: &'static str,
    points: Vec<Pos2>,
}

fn poly(pts: &[(f32, f32)]) -> Vec<Pos2> {
    pts.iter().map(|&(x, y)| pos2(x, y)).collect()
}

fn arc(cx: f32, cy: f32, r: f32, a0: f32, a1: f32, steps: usize) -> Vec<(f32, f32)> {
    (0..=steps)
        .map(|i| {
            let a = a0 + (a1 - a0) * i as f32 / steps as f32;
            (cx + r * a.cos(), cy + r * a.sin())
        })
        .collect()
}

fn templates() -> &'static [Template] {
    static T: OnceLock<Vec<Template>> = OnceLock::new();
    T.get_or_init(|| {
        use std::f32::consts::PI;
        let mut t = Vec::new();
        let mut add = |name: &'static str, glyph: &'static str, pts: Vec<(f32, f32)>| {
            t.push(Template { name, glyph, points: normalize(&poly(&pts)) });
        };
        // alpha: loop opening to the right
        add("alpha", "α", vec![(1.0, 0.0), (0.7, 0.6), (0.4, 0.9), (0.1, 0.8), (0.0, 0.5), (0.1, 0.2), (0.4, 0.1), (0.7, 0.4), (1.0, 1.0)]);
        // infinity
        let mut inf = Vec::new();
        for i in 0..=40 {
            let a = i as f32 / 40.0 * 2.0 * PI;
            inf.push((a.sin(), a.sin() * a.cos()));
        }
        add("infinity", "∞", inf);
        // proportional
        add("prop", "∝", vec![(1.0, 0.0), (0.6, 0.6), (0.3, 0.9), (0.05, 0.6), (0.2, 0.2), (0.5, 0.3), (0.8, 0.8), (1.0, 1.0)]);
        // at
        let mut at = arc(0.5, 0.5, 0.2, 0.0, 2.0 * PI, 12);
        at.extend(arc(0.5, 0.5, 0.45, 0.2, 2.0 * PI - 0.4, 24));
        add("at", "@", at);
        add("minus", "−", vec![(0.0, 0.5), (1.0, 0.5)]);
        add("triangle.r", "▷", vec![(0.0, 0.0), (1.0, 0.5), (0.0, 1.0), (0.0, 0.0)]);
        add("integral", "∫", vec![(0.9, 0.05), (0.7, 0.0), (0.55, 0.15), (0.5, 0.5), (0.45, 0.85), (0.3, 1.0), (0.1, 0.95)]);
        add("sum", "∑", vec![(1.0, 0.0), (0.0, 0.0), (0.5, 0.5), (0.0, 1.0), (1.0, 1.0)]);
        add("arrow.r", "→", vec![(0.0, 0.5), (1.0, 0.5), (0.75, 0.3), (1.0, 0.5), (0.75, 0.7)]);
        add("plus", "+", vec![(0.0, 0.5), (1.0, 0.5), (0.5, 0.0), (0.5, 1.0)]);
        add("times", "×", vec![(0.0, 0.0), (1.0, 1.0), (1.0, 0.0), (0.0, 1.0)]);
        add("circle.stroked", "○", arc(0.5, 0.5, 0.5, -PI / 2.0, 1.5 * PI, 32));
        add("sqrt", "√", vec![(0.0, 0.6), (0.2, 0.5), (0.4, 1.0), (0.7, 0.0), (1.0, 0.0)]);
        add("beta", "β", vec![(0.1, 1.0), (0.1, 0.1), (0.4, 0.0), (0.7, 0.15), (0.4, 0.45), (0.8, 0.65), (0.5, 0.95), (0.1, 0.8)]);
        add("pi", "π", vec![(0.0, 0.1), (1.0, 0.1), (0.3, 0.1), (0.25, 1.0), (0.7, 0.1), (0.75, 1.0)]);
        add("lambda", "λ", vec![(0.1, 0.0), (0.3, 0.05), (0.9, 1.0), (0.5, 0.4), (0.1, 1.0)]);
        add("partial", "∂", vec![(0.2, 0.1), (0.5, 0.0), (0.8, 0.3), (0.75, 0.8), (0.45, 1.0), (0.15, 0.8), (0.3, 0.5), (0.75, 0.55)]);
        add("nabla", "∇", vec![(0.0, 0.0), (1.0, 0.0), (0.5, 1.0), (0.0, 0.0)]);
        add("lt.eq", "≤", vec![(1.0, 0.0), (0.0, 0.35), (1.0, 0.7), (0.0, 1.0), (1.0, 1.0)]);
        add("in", "∈", vec![(1.0, 0.0), (0.5, 0.0), (0.05, 0.5), (0.5, 1.0), (1.0, 1.0), (0.0, 0.5), (1.0, 0.5)]);
        add("checkmark", "✓", vec![(0.0, 0.6), (0.35, 1.0), (1.0, 0.0)]);
        add("star.stroked", "☆", vec![(0.5, 0.0), (0.65, 0.4), (1.0, 0.4), (0.7, 0.65), (0.8, 1.0), (0.5, 0.78), (0.2, 1.0), (0.3, 0.65), (0.0, 0.4), (0.35, 0.4), (0.5, 0.0)]);
        t
    })
}

fn resample(pts: &[Pos2], n: usize) -> Vec<Pos2> {
    if pts.len() < 2 {
        return vec![pts.first().copied().unwrap_or(Pos2::ZERO); n];
    }
    let total: f32 = pts.windows(2).map(|w| w[0].distance(w[1])).sum();
    let step = (total / (n - 1) as f32).max(1e-6);
    let mut out = vec![pts[0]];
    let mut acc = 0.0;
    let mut prev = pts[0];
    let mut i = 1;
    while i < pts.len() && out.len() < n {
        let cur = pts[i];
        let d = prev.distance(cur);
        if acc + d >= step && d > 0.0 {
            let t = (step - acc) / d;
            let q = prev + (cur - prev) * t;
            out.push(q);
            prev = q;
            acc = 0.0;
        } else {
            acc += d;
            prev = cur;
            i += 1;
        }
    }
    while out.len() < n {
        out.push(*pts.last().unwrap());
    }
    out
}

fn normalize(pts: &[Pos2]) -> Vec<Pos2> {
    let pts = resample(pts, N);
    let (mut min, mut max) = (pts[0], pts[0]);
    for p in &pts {
        min = min.min(*p);
        max = max.max(*p);
    }
    let size = (max - min).max(Vec2::splat(1e-3));
    let scale = size.x.max(size.y);
    let center = min + size / 2.0;
    pts.iter().map(|p| pos2((p.x - center.x) / scale, (p.y - center.y) / scale)).collect()
}

fn distance(a: &[Pos2], b: &[Pos2]) -> f32 {
    let fwd: f32 = a.iter().zip(b).map(|(p, q)| p.distance(*q)).sum();
    let rev: f32 = a.iter().zip(b.iter().rev()).map(|(p, q)| p.distance(*q)).sum();
    fwd.min(rev) / a.len() as f32
}

/// Best matches for the drawn strokes, best first: (name, glyph).
pub fn recognize(strokes: &[Vec<Pos2>]) -> Vec<(&'static str, &'static str)> {
    let joined: Vec<Pos2> = strokes.iter().flatten().copied().collect();
    if joined.len() < 3 {
        return Vec::new();
    }
    let probe = normalize(&joined);
    let mut scored: Vec<(f32, &Template)> = templates().iter().map(|t| (distance(&probe, &t.points), t)).collect();
    scored.sort_by(|a, b| a.0.total_cmp(&b.0));
    scored.into_iter().take(6).map(|(_, t)| (t.name, t.glyph)).collect()
}
