//! Small vector icons painted with egui primitives (no icon font needed).

use eframe::egui::{self, Color32, Painter, Pos2, Rect, Shape, Stroke, pos2, vec2};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Icon {
    Sidebar,
    Folder,
    Search,
    Outline,
    Sqrt,
    Warning,
    ZoomIn,
    ZoomOut,
    Fit,
    Share,
    Document,
    DocumentPlus,
    Close,
    Columns,
    Bullets,
    Numbers,
    Check,
    Gear,
    Package,
    Pages,
    Image,
    Trash,
    ChevronDown,
    ChevronRight,
    Preview,
    Back,
    /// Two arrows chasing each other round a circle (sync now).
    Sync,
    /// Two chain links (connected accounts).
    Link,
    /// A broken chain (disconnect).
    Unlink,
}

fn line(p: &Painter, a: Pos2, b: Pos2, s: Stroke) {
    p.line_segment([a, b], s);
}

/// Points on a circle from angle `a0` to `a1` (degrees, clockwise on screen).
fn arc(c: Pos2, radius: f32, a0: f32, a1: f32) -> Vec<Pos2> {
    (0..=12)
        .map(|i| {
            let a = (a0 + (a1 - a0) * i as f32 / 12.0).to_radians();
            pos2(c.x + radius * a.cos(), c.y + radius * a.sin())
        })
        .collect()
}

/// Outline of a rounded bar (one chain link) around `c`, tilted by `angle` degrees.
fn link_shape(c: Pos2, half: f32, radius: f32, angle: f32) -> Vec<Pos2> {
    let d = vec2(angle.to_radians().cos(), angle.to_radians().sin());
    let mut pts = arc(c + d * half, radius, angle - 90.0, angle + 90.0);
    pts.extend(arc(c - d * half, radius, angle + 90.0, angle + 270.0));
    pts
}

pub fn paint(p: &Painter, rect: Rect, icon: Icon, color: Color32) {
    let c = rect.center();
    let r = rect.width().min(rect.height()) / 2.0;
    let w = (r * 0.12).max(1.2);
    let s = Stroke::new(w, color);
    let at = |x: f32, y: f32| pos2(c.x + x * r, c.y + y * r);
    match icon {
        Icon::Sidebar => {
            let b = Rect::from_center_size(c, vec2(r * 1.7, r * 1.4));
            p.rect_stroke(b, r * 0.25, s, egui::StrokeKind::Middle);
            line(p, pos2(b.left() + r * 0.6, b.top()), pos2(b.left() + r * 0.6, b.bottom()), s);
            for i in 0..3 {
                let y = b.top() + r * (0.35 + 0.22 * i as f32);
                line(p, pos2(b.left() + r * 0.18, y), pos2(b.left() + r * 0.42, y), Stroke::new(w * 0.8, color));
            }
        }
        Icon::Folder => {
            let pts = vec![
                at(-0.8, -0.55),
                at(-0.3, -0.55),
                at(-0.15, -0.38),
                at(0.8, -0.38),
                at(0.8, 0.6),
                at(-0.8, 0.6),
            ];
            p.add(Shape::closed_line(pts, s));
            line(p, at(-0.8, -0.2), at(0.8, -0.2), s);
        }
        Icon::Search => {
            p.circle_stroke(at(-0.15, -0.15), r * 0.5, s);
            line(p, at(0.22, 0.22), at(0.7, 0.7), Stroke::new(w * 1.3, color));
        }
        Icon::Outline => {
            for (i, (x0, x1)) in [(-0.5, 0.8), (-0.2, 0.8), (-0.2, 0.8), (-0.5, 0.8)].iter().enumerate() {
                let y = -0.55 + i as f32 * 0.37;
                p.circle_filled(at(x0 - 0.22, y), w * 0.7, color);
                line(p, at(*x0, y), at(*x1, y), s);
            }
        }
        Icon::Sqrt => {
            let pts = vec![at(-0.85, 0.05), at(-0.65, -0.05), at(-0.4, 0.6), at(-0.1, -0.7), at(0.85, -0.7)];
            p.add(Shape::line(pts, s));
            line(p, at(0.05, -0.3), at(0.65, 0.45), s);
            line(p, at(0.65, -0.3), at(0.05, 0.45), s);
        }
        Icon::Warning => {
            let pts = vec![at(0.0, -0.8), at(0.85, 0.7), at(-0.85, 0.7)];
            p.add(Shape::closed_line(pts, s));
            line(p, at(0.0, -0.3), at(0.0, 0.25), Stroke::new(w * 1.2, color));
            p.circle_filled(at(0.0, 0.47), w * 0.8, color);
        }
        Icon::ZoomIn | Icon::ZoomOut => {
            p.circle_stroke(at(-0.15, -0.15), r * 0.55, s);
            line(p, at(0.25, 0.25), at(0.75, 0.75), Stroke::new(w * 1.2, color));
            line(p, at(-0.42, -0.15), at(0.12, -0.15), s);
            if icon == Icon::ZoomIn {
                line(p, at(-0.15, -0.42), at(-0.15, 0.12), s);
            }
        }
        Icon::Fit => {
            line(p, at(-0.6, 0.6), at(0.6, -0.6), s);
            line(p, at(0.6, -0.6), at(0.15, -0.6), s);
            line(p, at(0.6, -0.6), at(0.6, -0.15), s);
            line(p, at(-0.6, 0.6), at(-0.15, 0.6), s);
            line(p, at(-0.6, 0.6), at(-0.6, 0.15), s);
        }
        Icon::Share => {
            let pts = vec![at(-0.3, -0.2), at(-0.65, -0.2), at(-0.65, 0.8), at(0.65, 0.8), at(0.65, -0.2), at(0.3, -0.2)];
            p.add(Shape::line(pts, s));
            line(p, at(0.0, -0.85), at(0.0, 0.3), s);
            line(p, at(0.0, -0.85), at(-0.3, -0.55), s);
            line(p, at(0.0, -0.85), at(0.3, -0.55), s);
        }
        Icon::Document | Icon::DocumentPlus => {
            let pts = vec![at(-0.55, -0.8), at(0.2, -0.8), at(0.6, -0.4), at(0.6, 0.8), at(-0.55, 0.8)];
            p.add(Shape::closed_line(pts, s));
            line(p, at(0.2, -0.8), at(0.2, -0.4), s);
            line(p, at(0.2, -0.4), at(0.6, -0.4), s);
            if icon == Icon::Document {
                for y in [0.0, 0.25, 0.5] {
                    line(p, at(-0.3, y), at(0.35, y), Stroke::new(w * 0.8, color));
                }
            } else {
                p.circle_filled(at(-0.55, 0.55), r * 0.35, color);
                let bg = crate::theme::pal().surface;
                line(p, at(-0.75, 0.55), at(-0.35, 0.55), Stroke::new(w, bg));
                line(p, at(-0.55, 0.35), at(-0.55, 0.75), Stroke::new(w, bg));
            }
        }
        Icon::Close => {
            line(p, at(-0.5, -0.5), at(0.5, 0.5), s);
            line(p, at(0.5, -0.5), at(-0.5, 0.5), s);
        }
        Icon::Columns => {
            for x in [-0.6, -0.25, 0.1] {
                line(p, at(x, -0.6), at(x, 0.6), s);
            }
            p.add(Shape::convex_polygon(vec![at(0.35, -0.3), at(0.8, 0.0), at(0.35, 0.3)], color, Stroke::NONE));
        }
        Icon::Bullets | Icon::Numbers => {
            for i in 0..3 {
                let y = -0.55 + i as f32 * 0.55;
                if icon == Icon::Bullets {
                    p.circle_filled(at(-0.6, y), w * 1.1, color);
                } else {
                    line(p, at(-0.65, y - 0.15), at(-0.65, y + 0.15), Stroke::new(w * 0.9, color));
                }
                line(p, at(-0.3, y), at(0.8, y), s);
            }
        }
        Icon::Check => {
            p.circle_stroke(c, r * 0.9, s);
            p.add(Shape::line(vec![at(-0.4, 0.0), at(-0.1, 0.3), at(0.45, -0.35)], s));
        }
        Icon::Gear => {
            p.circle_stroke(c, r * 0.3, s);
            p.circle_stroke(c, r * 0.62, s);
            for i in 0..8 {
                let a = i as f32 * std::f32::consts::TAU / 8.0;
                let d = vec2(a.cos(), a.sin());
                line(p, c + d * r * 0.62, c + d * r * 0.88, Stroke::new(w * 2.0, color));
            }
        }
        Icon::Package => {
            let top = vec![at(0.0, -0.85), at(0.75, -0.45), at(0.0, -0.05), at(-0.75, -0.45)];
            p.add(Shape::closed_line(top, s));
            line(p, at(-0.75, -0.45), at(-0.75, 0.45), s);
            line(p, at(0.75, -0.45), at(0.75, 0.45), s);
            line(p, at(0.0, -0.05), at(0.0, 0.85), s);
            line(p, at(-0.75, 0.45), at(0.0, 0.85), s);
            line(p, at(0.75, 0.45), at(0.0, 0.85), s);
        }
        Icon::Pages => {
            p.rect_stroke(Rect::from_min_max(at(-0.75, -0.45), at(0.45, 0.65)), r * 0.15, s, egui::StrokeKind::Middle);
            p.add(Shape::line(vec![at(-0.45, -0.45), at(-0.45, -0.75), at(0.75, -0.75), at(0.75, 0.35), at(0.45, 0.35)], s));
        }
        Icon::Image => {
            let b = Rect::from_center_size(c, vec2(r * 1.7, r * 1.3));
            p.rect_filled(b, r * 0.2, color);
            let bg = Color32::from_rgba_unmultiplied(255, 255, 255, 200);
            p.add(Shape::convex_polygon(vec![at(-0.65, 0.5), at(-0.2, -0.05), at(0.15, 0.5)], bg, Stroke::NONE));
            p.add(Shape::convex_polygon(vec![at(0.0, 0.5), at(0.35, 0.1), at(0.7, 0.5)], bg, Stroke::NONE));
            p.circle_filled(at(0.35, -0.3), r * 0.13, bg);
        }
        Icon::Trash => {
            line(p, at(-0.7, -0.55), at(0.7, -0.55), s);
            line(p, at(-0.2, -0.75), at(0.2, -0.75), s);
            p.add(Shape::closed_line(vec![at(-0.55, -0.55), at(0.55, -0.55), at(0.45, 0.8), at(-0.45, 0.8)], s));
            for x in [-0.2, 0.0, 0.2] {
                line(p, at(x, -0.3), at(x, 0.55), Stroke::new(w * 0.8, color));
            }
        }
        Icon::ChevronDown => {
            p.add(Shape::line(vec![at(-0.5, -0.25), at(0.0, 0.25), at(0.5, -0.25)], s));
        }
        Icon::ChevronRight => {
            p.add(Shape::line(vec![at(-0.25, -0.5), at(0.25, 0.0), at(-0.25, 0.5)], s));
        }
        Icon::Back => {
            line(p, at(-0.65, 0.0), at(0.65, 0.0), s);
            p.add(Shape::line(vec![at(-0.15, -0.5), at(-0.65, 0.0), at(-0.15, 0.5)], s));
        }
        Icon::Sync => {
            let radius = r * 0.62;
            for (a0, a1) in [(195.0_f32, 335.0_f32), (15.0, 155.0)] {
                p.add(Shape::line(arc(c, radius, a0, a1), s));
                // Arrow head at the end, pointing along the circle.
                let a = a1.to_radians();
                let tip = pos2(c.x + radius * a.cos(), c.y + radius * a.sin());
                let dir = vec2(-a.sin(), a.cos());
                let back = -dir * r * 0.34;
                let side = vec2(-dir.y, dir.x) * r * 0.24;
                line(p, tip, tip + back + side, s);
                line(p, tip, tip + back - side, s);
            }
        }
        Icon::Link | Icon::Unlink => {
            let d = vec2(1.0, -1.0).normalized();
            if icon == Icon::Link {
                for k in [-1.0, 1.0] {
                    p.add(Shape::closed_line(link_shape(c + d * (k * r * 0.3), r * 0.24, r * 0.27, -45.0), s));
                }
            } else {
                for k in [-1.0, 1.0] {
                    p.add(Shape::closed_line(link_shape(c + d * (k * r * 0.46), r * 0.12, r * 0.25, -45.0), s));
                }
                // The break: two short strokes across the gap.
                let n = vec2(d.y, -d.x);
                for k in [-1.0, 1.0] {
                    let m = c + n * (k * r * 0.42);
                    line(p, m - d * (r * 0.1) - n * (k * r * 0.06), m + d * (r * 0.1) + n * (k * r * 0.12), s);
                }
            }
        }
        Icon::Preview => {
            // The counterpart of the sidebar icon: a window with the page shown on its right half.
            let b = Rect::from_center_size(c, vec2(r * 1.7, r * 1.4));
            p.rect_stroke(b, r * 0.25, s, egui::StrokeKind::Middle);
            line(p, pos2(b.center().x, b.top()), pos2(b.center().x, b.bottom()), s);
            for i in 0..3 {
                let y = b.top() + r * (0.35 + 0.22 * i as f32);
                line(p, pos2(b.center().x + r * 0.2, y), pos2(b.right() - r * 0.2, y), Stroke::new(w * 0.8, color));
            }
        }
    }
}
