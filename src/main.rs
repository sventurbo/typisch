mod app;
mod compiler;
mod highlight;
mod icons;
mod ide;
mod logo;
mod menu;
mod refsync;
mod refsync_ui;
mod screens;
mod symbols;
mod templates;
mod theme;
mod widgets;
mod zotero;

use eframe::egui;
use std::path::PathBuf;

fn main() -> eframe::Result<()> {
    let mut screenshot: Option<PathBuf> = None;
    let mut open: Option<PathBuf> = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--screenshot" => screenshot = args.next().map(PathBuf::from),
            "--export-icon" => {
                // Writes the app icon as a PNG (macos/make-app.sh builds the .icns from these), then exits.
                let path = args.next().unwrap_or_else(|| "typisch.png".into());
                let size = args.next().and_then(|s| s.parse().ok()).unwrap_or(1024);
                let icon = logo::icon_rgba(size);
                let img = image::RgbaImage::from_raw(icon.width, icon.height, icon.rgba).expect("icon pixels");
                img.save(&path).expect("write icon");
                return Ok(());
            }
            "--export-logo" => {
                // Writes the logo as an SVG (the tile alone, without the icon's margin), then exits.
                let path = args.next().unwrap_or_else(|| "typisch.svg".into());
                std::fs::write(&path, logo::svg()).expect("write logo");
                return Ok(());
            }
            "-h" | "--help" => {
                println!(
                    "typisch [--screenshot <image.png>] [--export-icon <icon.png> [size]] [--export-logo <logo.svg>] [folder-or-file]"
                );
                return Ok(());
            }
            other => open = Some(PathBuf::from(other)),
        }
    }

    // For a screenshot keep the window at its designed size (tiling window managers may resize it otherwise).
    let fixed = screenshot.is_some();
    let saved = if fixed { None } else { theme::Settings::load().window_size };
    let shot_size = std::env::var("TYPISCH_SHOT_SIZE").ok().and_then(|v| {
        let (w, h) = v.split_once('x')?;
        Some([w.parse().ok()?, h.parse().ok()?])
    });
    let size = shot_size.filter(|_| fixed).or(saved).unwrap_or([1600.0, 968.0]);
    let mut viewport = egui::ViewportBuilder::default()
        .with_resizable(!fixed)
        .with_title("Typisch")
        // Matches the typisch.desktop launcher, so Linux docks group the window with its icon.
        .with_app_id("typisch")
        .with_icon(std::sync::Arc::new(logo::icon_rgba(256)))
        .with_inner_size(size)
        .with_min_inner_size([640.0, 400.0]);
    if cfg!(target_os = "macos") {
        // The window buttons sit inside the window, over the sidebar.
        viewport = viewport.with_fullsize_content_view(true).with_titlebar_shown(false).with_title_shown(false);
    }
    let options = eframe::NativeOptions { viewport, centered: true, ..Default::default() };
    eframe::run_native(
        "Typisch",
        options,
        Box::new(move |cc| Ok(Box::new(app::App::new(cc, screenshot, open)))),
    )
}
