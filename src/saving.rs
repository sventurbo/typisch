//! Saving the open tabs, and the question that comes up when another program changed a file
//! that has unsaved changes here. The writing itself is in files.rs.

use std::time::Duration;

use eframe::egui::{self, Align, Layout, vec2};

use crate::app::App;
use crate::files::{Conflict, SaveError, save_doc};
use crate::gitui::{body, clicked, modal, title};
use crate::widgets::{Button, CONTROL_H};

/// A tab that cannot be saved without overwriting what another program did to its file.
pub struct SaveConflict {
    pub rel: String,
    pub kind: Conflict,
}

const CANCEL: &str = "Abbrechen";
const RELOAD: &str = "Neu laden";
const OVERWRITE: &str = "Überschreiben";
const CLOSE_TAB: &str = "Tab schließen";
const RECREATE: &str = "Wieder anlegen";

impl App {
    /// Saves the active tab. False when it has changes that could not be written; the user was
    /// told why (a message, or the question what to do about a file changed elsewhere).
    pub fn save_now(&mut self) -> bool {
        match self.doc.as_ref().map(|d| d.rel.clone()) {
            Some(rel) => self.save_tab(&rel, false),
            None => true,
        }
    }

    /// Saves every tab with unsaved changes, also background tabs whose earlier save failed.
    /// False when one of them could not be written.
    pub fn save_all(&mut self) -> bool {
        let mut all = true;
        for rel in self.tab_order.clone() {
            all &= self.save_tab(&rel, false);
        }
        all
    }

    /// The save a little after typing stops. It does not ask again about a file whose question
    /// the user put off; saving by hand, closing the tab and leaving the project do.
    pub fn autosave(&mut self) {
        let postponed = self.conflict_postponed.as_deref();
        if self.doc.as_ref().is_some_and(|d| postponed != Some(d.rel.as_str())) {
            self.save_now();
        }
    }

    /// Saves the active tab and moves it to the background.
    pub fn stash_active(&mut self) {
        self.autosave();
        if let Some(d) = self.doc.take() {
            self.tabs.push(d);
        }
    }

    fn save_tab(&mut self, rel: &str, overwrite: bool) -> bool {
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
        // Its question is on screen: the answer decides, not another attempt.
        if !overwrite && self.save_conflict.as_ref().is_some_and(|c| c.rel == rel) {
            return false;
        }
        match save_doc(&project.root, doc, overwrite) {
            Ok(_) => {
                self.git.file_saved();
                true
            }
            Err(SaveError::Conflict(kind)) => {
                // One question at a time; the next save attempt finds the others.
                if self.save_conflict.is_none() {
                    self.save_conflict = Some(SaveConflict {
                        rel: rel.to_string(),
                        kind,
                    });
                }
                false
            }
            Err(SaveError::Io(e)) => {
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
        if !self.save_tab(rel, false) {
            return;
        }
        self.drop_tab(rel);
        self.schedule_compile(Duration::ZERO);
    }

    /// Takes a tab away without writing it.
    fn drop_tab(&mut self, rel: &str) {
        crate::gitui::close_without_saving(&mut self.doc, &mut self.tabs, &mut self.tab_order, rel);
        if self.conflict_postponed.as_deref() == Some(rel) {
            self.conflict_postponed = None;
        }
    }

    /// The window was asked to close: save everything first. It stays open while a question
    /// about a changed file is unanswered, and once when a file cannot be written at all (the
    /// message says so; closing again then quits without that file's changes).
    pub fn before_close(&mut self, ctx: &egui::Context) {
        if self.save_all() {
            return;
        }
        if self.save_conflict.is_some() {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
        } else if !std::mem::replace(&mut self.close_warned, true) {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            if let Some((message, ..)) = self.toast.as_mut() {
                message.push_str(" Noch einmal schließen beendet Typisch ohne diese Änderungen.");
            }
        }
    }

    fn answer_conflict(&mut self, rel: &str, answer: &str, ctx: &egui::Context) {
        self.save_conflict = None;
        self.conflict_postponed = None;
        match answer {
            RELOAD => {
                if self.reload_from_disk(rel, ctx) {
                    self.schedule_compile(Duration::ZERO);
                } else {
                    self.toast(format!("„{rel}“ ließ sich nicht neu laden."), false);
                }
            }
            OVERWRITE | RECREATE => {
                if self.save_tab(rel, true) {
                    self.refresh_files();
                    self.schedule_compile(Duration::ZERO);
                }
            }
            CLOSE_TAB => {
                self.drop_tab(rel);
                self.refresh_files();
                self.schedule_compile(Duration::ZERO);
            }
            // Put off: the tab keeps its changes, unsaved.
            _ => self.conflict_postponed = Some(rel.to_string()),
        }
    }
}

/// "Die Datei wurde außerhalb von Typisch geändert": load it again or overwrite it.
pub fn conflict_dialog(app: &mut App, ctx: &egui::Context) {
    let Some(SaveConflict { rel, kind }) = app.save_conflict.as_ref() else {
        return;
    };
    let (rel, kind) = (rel.clone(), *kind);
    // The tab went away meanwhile (a pull deleted its file, say): nothing left to ask.
    if !app.tab_order.contains(&rel) {
        app.save_conflict = None;
        return;
    }
    let (heading, text, discard, keep) = match kind {
        Conflict::Changed => (
            format!("„{rel}“ wurde außerhalb von Typisch geändert"),
            "Ein anderes Programm hat die Datei geändert, während du sie hier bearbeitet hast. „Neu laden“ zeigt die Datei, \
             wie sie jetzt auf der Festplatte liegt, und verwirft deine ungespeicherten Änderungen. „Überschreiben“ \
             speichert deinen Stand und ersetzt damit die Änderung des anderen Programms.",
            RELOAD,
            OVERWRITE,
        ),
        Conflict::Deleted => (
            format!("„{rel}“ gibt es nicht mehr"),
            "Die Datei wurde gelöscht, umbenannt oder verschoben, während du sie hier bearbeitet hast. „Wieder anlegen“ \
             speichert deinen Stand unter dem alten Namen. „Tab schließen“ verwirft deine ungespeicherten Änderungen.",
            CLOSE_TAB,
            RECREATE,
        ),
    };
    let (answer, esc) = modal(ctx, "save-conflict", 460.0, |ui| {
        title(ui, &heading);
        body(ui, text);
        ui.add_space(10.0);
        let mut answer = None;
        // No button stands out: either choice gives something up.
        ui.allocate_ui_with_layout(vec2(ui.available_width(), CONTROL_H), Layout::right_to_left(Align::Center), |ui| {
            for label in [keep, discard, CANCEL] {
                if clicked(&ui.add(Button::new(label)), label) {
                    answer = Some(label);
                }
            }
        });
        answer
    });
    if let Some(answer) = answer.or(esc.then_some(CANCEL)) {
        app.answer_conflict(&rel, answer, ctx);
    }
}

/// The whole window driven headless (as in gitui_e2e.rs), with real files in a temp folder.
#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};
    use std::time::Instant;

    use eframe::egui::{Rect, pos2};

    use super::*;
    use crate::app::MenuAction;

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

        fn press(&mut self, label: &str) -> bool {
            crate::gitui::press_during(label, || self.frame())
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

        fn asked(&self) -> Option<(&str, Conflict)> {
            self.app.save_conflict.as_ref().map(|c| (c.rel.as_str(), c.kind))
        }
    }

    impl Drop for Harness {
        fn drop(&mut self) {
            unlock(&self.dir);
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    /// Another program writes the file (with a time stamp that differs even on coarse clocks).
    fn write_outside(path: &Path, text: &str) {
        std::fs::write(path, text).unwrap();
        let file = std::fs::File::options().write(true).open(path).unwrap();
        file.set_modified(std::time::SystemTime::now() + Duration::from_secs(5)).unwrap();
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

    #[test]
    fn a_file_changed_elsewhere_can_be_loaded_again() {
        let mut h = Harness::new("reload", &[("main.typ", "alt")]);
        h.edit("main.typ", "meins");
        write_outside(&h.path("main.typ"), "fremd");
        h.frame();
        assert_eq!(h.asked(), Some(("main.typ", Conflict::Changed)));
        assert_eq!(h.on_disk("main.typ"), "fremd", "nothing was overwritten");
        assert!(h.doc("main.typ").unsaved());
        // Further saves wait for the answer; the window stays open for it.
        assert!(!h.app.save_now());
        assert!(!h.close_window());
        assert_eq!(h.on_disk("main.typ"), "fremd");

        assert!(h.press(RELOAD));
        assert_eq!(h.asked(), None);
        assert_eq!((h.doc("main.typ").text.as_str(), h.doc("main.typ").unsaved()), ("fremd", false));
        // From here on it is an ordinary file again.
        h.edit("main.typ", "fremd und meins");
        h.frame();
        assert_eq!((h.on_disk("main.typ").as_str(), h.asked()), ("fremd und meins", None));
    }

    #[test]
    fn a_file_changed_elsewhere_can_be_overwritten() {
        let mut h = Harness::new("overwrite", &[("main.typ", "alt")]);
        h.edit("main.typ", "meins");
        write_outside(&h.path("main.typ"), "fremd");
        h.frame();
        assert!(!h.press(RECREATE), "that button belongs to a deleted file");
        assert!(h.press(OVERWRITE));
        assert_eq!((h.on_disk("main.typ").as_str(), h.asked()), ("meins", None));
        assert!(!h.doc("main.typ").unsaved());
    }

    #[test]
    fn a_question_put_off_comes_back_when_saving_by_hand() {
        let mut h = Harness::new("postpone", &[("main.typ", "alt"), ("kapitel.typ", "k")]);
        h.edit("main.typ", "meins");
        write_outside(&h.path("main.typ"), "fremd");
        h.frame();
        assert!(h.press(CANCEL));
        assert_eq!(h.asked(), None);
        // Typing on does not ask again, and neither does looking at another tab.
        h.edit("main.typ", "meins, weiter");
        h.frame();
        h.app.open_file("kapitel.typ");
        h.app.open_file("main.typ");
        h.frame();
        assert_eq!(h.asked(), None);
        assert_eq!(h.on_disk("main.typ"), "fremd");
        assert_eq!(h.doc("main.typ").text, "meins, weiter");
        // Other tabs are saved as ever meanwhile.
        h.edit("kapitel.typ", "k2");
        assert!(!h.app.save_all());
        assert_eq!(h.on_disk("kapitel.typ"), "k2");
        h.app.save_conflict = None;

        let ctx = h.ctx.clone();
        h.app.run(MenuAction::Save, &ctx);
        assert_eq!(h.asked(), Some(("main.typ", Conflict::Changed)));
        h.frame();
        assert!(h.press(OVERWRITE));
        assert_eq!(h.on_disk("main.typ"), "meins, weiter");
    }

    #[test]
    fn a_deleted_file_is_created_again_or_its_tab_closed() {
        let mut h = Harness::new("deleted", &[("main.typ", "haupt"), ("a.typ", "a"), ("b.typ", "b")]);
        h.edit("a.typ", "a, getippt");
        h.edit("b.typ", "b, getippt");
        std::fs::remove_file(h.path("a.typ")).unwrap();
        std::fs::remove_file(h.path("b.typ")).unwrap();
        // One question at a time.
        assert!(!h.app.save_all());
        assert_eq!(h.asked(), Some(("a.typ", Conflict::Deleted)));
        h.frame();
        assert!(!h.path("a.typ").exists());
        assert!(h.press(RECREATE));
        assert_eq!(h.on_disk("a.typ"), "a, getippt");

        assert!(!h.app.save_all());
        assert_eq!(h.asked(), Some(("b.typ", Conflict::Deleted)));
        h.frame();
        assert!(h.press(CLOSE_TAB));
        assert_eq!(h.app.tab_order, ["main.typ", "a.typ"]);
        assert!(!h.path("b.typ").exists());
        assert!(h.app.save_all());
    }

    #[test]
    fn a_file_rewritten_with_the_same_content_is_no_question() {
        let mut h = Harness::new("touched", &[("main.typ", "alt")]);
        write_outside(&h.path("main.typ"), "alt");
        h.edit("main.typ", "neu");
        h.frame();
        assert_eq!((h.on_disk("main.typ").as_str(), h.asked()), ("neu", None));
    }
}
