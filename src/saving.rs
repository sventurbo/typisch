//! Saving the open tabs. The writing itself is in files.rs.

use std::time::Duration;

use eframe::egui;

use crate::app::App;
use crate::files::save_doc;

impl App {
    /// Saves the active tab. False when it has changes that could not be written; the user was
    /// told why.
    pub fn save_now(&mut self) -> bool {
        match self.doc.as_ref().map(|d| d.rel.clone()) {
            Some(rel) => self.save_tab(&rel),
            None => true,
        }
    }

    /// Saves every tab with unsaved changes, also background tabs whose earlier save failed.
    /// False when one of them could not be written.
    pub fn save_all(&mut self) -> bool {
        let mut all = true;
        for rel in self.tab_order.clone() {
            all &= self.save_tab(&rel);
        }
        all
    }

    /// Saves the active tab and moves it to the background.
    pub fn stash_active(&mut self) {
        self.save_now();
        if let Some(d) = self.doc.take() {
            self.tabs.push(d);
        }
    }

    fn save_tab(&mut self, rel: &str) -> bool {
        let Some(project) = self.project.as_ref() else { return true };
        // A file synced from Zotero is never written from the editor (it is read-only there).
        if self.refsync.is_synced(&project.root, rel) {
            return true;
        }
        let Some(doc) = self.doc.iter_mut().chain(self.tabs.iter_mut()).find(|d| d.rel == rel) else {
            return true;
        };
        if !doc.unsaved() {
            return true;
        }
        match save_doc(&project.root, doc) {
            Ok(_) => {
                self.git.file_saved();
                true
            }
            Err(e) => {
                self.toast(format!("„{rel}“ konnte nicht gespeichert werden: {e}"), false);
                false
            }
        }
    }

    /// Closes a tab and shows its neighbour. Its changes are saved first; a tab that cannot be
    /// saved stays open, so what was typed is not lost.
    pub fn close_tab(&mut self, rel: &str) {
        if !self.tab_order.iter().any(|r| r == rel) {
            return;
        }
        if !self.save_tab(rel) {
            return;
        }
        crate::gitui::close_without_saving(&mut self.doc, &mut self.tabs, &mut self.tab_order, rel);
        self.schedule_compile(Duration::ZERO);
    }

    /// The window was asked to close: save everything first. It stays open once when a file
    /// cannot be written (the message says so; closing again then quits without that file's
    /// changes).
    pub fn before_close(&mut self, ctx: &egui::Context) {
        if self.save_all() {
            return;
        }
        if !std::mem::replace(&mut self.close_warned, true) {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            if let Some((message, ..)) = self.toast.as_mut() {
                message.push_str(" Noch einmal schließen beendet Typisch ohne diese Änderungen.");
            }
        }
    }
}

/// The whole window driven headless (as in gitui_e2e.rs), with real files in a temp folder.
#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};
    use std::time::Instant;

    use eframe::egui::{Rect, pos2, vec2};

    use super::*;

    struct Harness {
        ctx: egui::Context,
        app: App,
        dir: PathBuf,
    }

    impl Harness {
        /// A project with these files; the first one is open, the others are background tabs.
        fn new(name: &str, files: &[(&str, &str)]) -> Self {
            let dir = std::env::temp_dir().join(format!("typisch-saving-{name}-{}", std::process::id()));
            unlock(&dir);
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(dir.join("projekt")).unwrap();
            let dir = dir.canonicalize().unwrap();
            for (rel, text) in files {
                std::fs::write(dir.join("projekt").join(rel), text).unwrap();
            }
            let ctx = egui::Context::default();
            let mut app = App::for_tests(&ctx);
            // Git is kept away from the user's own configuration.
            app.git.isolate(&ctx, &dir);
            app.open_path(&dir.join("projekt").join(files[0].0), &ctx);
            for (rel, _) in files.iter().skip(1) {
                app.open_file(rel);
            }
            app.open_file(files[0].0);
            let mut h = Self { ctx, app, dir };
            h.frame();
            h
        }

        fn path(&self, rel: &str) -> PathBuf {
            self.dir.join("projekt").join(rel)
        }

        fn on_disk(&self, rel: &str) -> String {
            std::fs::read_to_string(self.path(rel)).unwrap()
        }

        /// One frame; returns the commands it sent to the window.
        fn frame_with(&mut self, close_requested: bool) -> Vec<egui::ViewportCommand> {
            let mut input = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(1600.0, 968.0))),
                ..Default::default()
            };
            if close_requested {
                let window = input.viewports.entry(egui::ViewportId::ROOT).or_default();
                window.events.push(egui::ViewportEvent::Close);
            }
            let app = &mut self.app;
            let mut out = self.ctx.run_ui(input, |ui| app.frame(ui));
            let commands = out
                .viewport_output
                .remove(&egui::ViewportId::ROOT)
                .map(|v| v.commands)
                .unwrap_or_default();
            out.drop_without_applying_deltas();
            commands
        }

        fn frame(&mut self) {
            self.frame_with(false);
        }

        /// The user closes the window; true when the app lets it close.
        fn close_window(&mut self) -> bool {
            let commands = self.frame_with(true);
            !commands.contains(&egui::ViewportCommand::CancelClose)
        }

        /// Typing into a tab (the active one or one in the background, as left there by a
        /// failed save); the autosave is due at once.
        fn edit(&mut self, rel: &str, text: &str) {
            let app = &mut self.app;
            let doc = app.doc.iter_mut().chain(app.tabs.iter_mut()).find(|d| d.rel == rel).unwrap();
            doc.text = text.into();
            doc.edited_at = Some(Instant::now() - Duration::from_secs(5));
        }

        fn doc(&self, rel: &str) -> &crate::app::OpenDoc {
            let app = &self.app;
            app.doc.iter().chain(app.tabs.iter()).find(|d| d.rel == rel).unwrap()
        }

        fn toast(&self) -> String {
            self.app.toast.as_ref().map(|t| t.0.clone()).unwrap_or_default()
        }
    }

    impl Drop for Harness {
        fn drop(&mut self) {
            unlock(&self.dir);
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    /// Makes a file and its folder read-only, so nothing can be saved there.
    #[cfg(unix)]
    fn lock(path: &Path) -> bool {
        use std::os::unix::fs::PermissionsExt;
        let dir = path.parent().unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o444)).unwrap();
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o555)).unwrap();
        // Root writes anyway; such a test has nothing to show then.
        std::fs::write(dir.join("probe"), "").is_err()
    }

    fn unlock(dir: &Path) {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let project = dir.join("projekt");
            let _ = std::fs::set_permissions(&project, std::fs::Permissions::from_mode(0o755));
            for entry in std::fs::read_dir(&project).into_iter().flatten().flatten() {
                let _ = std::fs::set_permissions(entry.path(), std::fs::Permissions::from_mode(0o644));
            }
        }
        let _ = dir;
    }

    #[test]
    fn typing_is_saved_shortly_after() {
        let mut h = Harness::new("autosave", &[("main.typ", "alt")]);
        h.edit("main.typ", "neu");
        h.frame();
        assert_eq!(h.on_disk("main.typ"), "neu");
        assert!(!h.doc("main.typ").unsaved());
        assert_eq!(h.toast(), "");
    }

    #[cfg(unix)]
    #[test]
    fn a_tab_that_cannot_be_saved_stays_open() {
        let mut h = Harness::new("close-tab", &[("main.typ", "haupt"), ("kapitel.typ", "alt")]);
        h.app.open_file("kapitel.typ");
        h.edit("kapitel.typ", "getippt");
        if !lock(&h.path("kapitel.typ")) {
            return;
        }
        // Closing the active tab …
        h.app.close_tab("kapitel.typ");
        h.frame();
        assert_eq!(h.app.tab_order, ["main.typ", "kapitel.typ"]);
        assert!(
            h.toast().contains("kapitel.typ") && h.toast().contains("nicht gespeichert"),
            "{}",
            h.toast()
        );
        // … switching away from it (it goes to the background unsaved), and closing it there.
        h.app.open_file("main.typ");
        h.app.toast = None;
        h.app.close_tab("kapitel.typ");
        h.frame();
        assert_eq!(h.app.tab_order, ["main.typ", "kapitel.typ"]);
        assert_eq!(h.doc("kapitel.typ").text, "getippt");
        assert!(h.toast().contains("kapitel.typ"), "{}", h.toast());
        assert_eq!(h.on_disk("kapitel.typ"), "alt");
        // Neither may the project be left with it.
        let ctx = h.ctx.clone();
        h.app.close_project(&ctx);
        assert!(h.app.project.is_some());
        h.app.open_project(h.dir.clone(), None, &ctx);
        assert_eq!(h.app.project.as_ref().map(|p| p.root.clone()), Some(h.dir.join("projekt")));
        assert_eq!(h.doc("kapitel.typ").text, "getippt");

        // Once the file can be written, closing saves it.
        unlock(&h.dir);
        h.app.close_tab("kapitel.typ");
        assert_eq!(h.app.tab_order, ["main.typ"]);
        assert_eq!(h.on_disk("kapitel.typ"), "getippt");
    }

    #[cfg(unix)]
    #[test]
    fn quitting_saves_the_background_tabs_too() {
        let mut h = Harness::new("quit", &[("main.typ", "haupt"), ("kapitel.typ", "alt")]);
        h.app.open_file("kapitel.typ");
        h.edit("kapitel.typ", "getippt");
        if !lock(&h.path("kapitel.typ")) {
            return;
        }
        h.app.open_file("main.typ");
        assert!(h.doc("kapitel.typ").unsaved(), "it went to the background unsaved");
        h.edit("main.typ", "auch getippt");
        // The first try to close the window is held back, so the message is seen.
        assert!(!h.close_window());
        assert!(h.toast().contains("Noch einmal schließen"), "{}", h.toast());
        assert!(h.close_window(), "the second one closes");
        // The disk is writable again in time: everything is written on the way out.
        unlock(&h.dir);
        eframe::App::on_exit(&mut h.app, None);
        assert_eq!(h.on_disk("kapitel.typ"), "getippt");
        assert_eq!(h.on_disk("main.typ"), "auch getippt");
    }

    #[test]
    fn closing_the_window_saves_and_closes() {
        let mut h = Harness::new("close-window", &[("main.typ", "alt")]);
        h.edit("main.typ", "neu");
        // Not yet due for the autosave.
        h.app.doc.as_mut().unwrap().edited_at = Some(Instant::now());
        assert!(h.close_window());
        assert_eq!(h.on_disk("main.typ"), "neu");
    }
}
