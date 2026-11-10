//! Git Sync end to end: the real window code (`App::frame`, the same as every eframe frame), the
//! real git worker and real repositories in a temp folder, driven headless. Buttons are pressed
//! by their label (see `clicked` in gitui.rs) and typing is done by filling the dialog's fields,
//! as a click or keystroke would. Git runs isolated from the user's own configuration.

use std::cell::RefCell;
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};

use super::*;
use crate::app::MenuAction;

/// The longest any step may take (tests run in parallel, git is quick).
const TIMEOUT: Duration = Duration::from_secs(60);
/// The top bar buttons, by the first line of their tooltip.
const PULL: &str = "Änderungen holen (Pull)";
const PUSH: &str = "Änderungen hochladen (Push)";

thread_local! {
    /// The label of the button the test presses in the next frame (frames run on the test's thread).
    static PRESS: RefCell<Option<String>> = const { RefCell::new(None) };
}

/// Asked by every Git Sync button as it is drawn: true (once) when the test pressed this one.
pub(super) fn take_press(label: &str) -> bool {
    PRESS.with(|p| {
        let mut p = p.borrow_mut();
        let hit = p.as_deref() == Some(label);
        if hit {
            *p = None;
        }
        hit
    })
}

// ---------------------------------------------------------------------------
// Repositories.

/// One test's folder: a bare remote, clones, project folders. Removed afterwards.
struct Sandbox {
    root: PathBuf,
}

impl Sandbox {
    fn new(name: &str) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("typisch-e2e-{name}-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // git and the app report real paths (/private/var/… on macOS); compare with those.
        let root = dir.canonicalize().unwrap();
        std::fs::write(root.join("gitconfig"), "").unwrap();
        Self { root }
    }

    fn path(&self, rel: &str) -> PathBuf {
        self.root.join(rel)
    }

    /// git as "someone else" (Bert), with the same isolation as the app's worker.
    fn output(&self, dir: &Path, args: &[&str]) -> Output {
        let mut c = Command::new("git");
        c.arg("-C").arg(dir).args(args);
        for key in [
            "GIT_DIR",
            "GIT_WORK_TREE",
            "GIT_INDEX_FILE",
            "GIT_OBJECT_DIRECTORY",
            "GIT_COMMON_DIR",
            "GIT_PREFIX",
        ] {
            c.env_remove(key);
        }
        c.env("HOME", &self.root)
            .env("XDG_CONFIG_HOME", self.root.join(".config"))
            .env("GIT_CONFIG_GLOBAL", self.root.join("gitconfig"))
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("LC_ALL", "C")
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GIT_AUTHOR_NAME", "Bert Beispiel")
            .env("GIT_AUTHOR_EMAIL", "bert@example.com")
            .env("GIT_COMMITTER_NAME", "Bert Beispiel")
            .env("GIT_COMMITTER_EMAIL", "bert@example.com")
            .stdin(Stdio::null());
        c.output().unwrap()
    }

    /// Runs git; it must succeed. Returns stdout without the final line break.
    fn git(&self, dir: &Path, args: &[&str]) -> String {
        let out = self.output(dir, args);
        assert!(
            out.status.success(),
            "git {args:?} in {}: {}",
            dir.display(),
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).trim_end().to_string()
    }

    /// An empty bare repository "remote.git" whose unborn branch is main; returns its file:// URL.
    fn bare(&self) -> String {
        self.git(&self.root, &["init", "-q", "--bare", "-b", "main", "remote.git"]);
        format!("file://{}", self.path("remote.git").display())
    }

    /// A remote whose main branch holds `files` in one commit; returns its URL.
    fn seeded(&self, files: &[(&str, &[u8])]) -> String {
        let url = self.bare();
        self.git(&self.root, &["init", "-q", "-b", "main", "seed"]);
        let seed = self.path("seed");
        self.git(&seed, &["remote", "add", "origin", &url]);
        for (path, content) in files {
            write(&seed.join(path), content);
        }
        self.commit_push(&seed, "Anfang");
        url
    }

    fn clone(&self, url: &str, name: &str) -> PathBuf {
        self.git(&self.root, &["clone", "-q", "--", url, name]);
        self.path(name)
    }

    /// Commits everything in `dir` and pushes it to the remote's main branch; returns the commit.
    fn commit_push(&self, dir: &Path, message: &str) -> String {
        self.git(dir, &["add", "-A"]);
        self.git(dir, &["commit", "-q", "-m", message]);
        self.git(dir, &["push", "-q", "origin", "HEAD:refs/heads/main"]);
        self.head(dir)
    }

    fn head(&self, dir: &Path) -> String {
        self.git(dir, &["rev-parse", "HEAD"])
    }

    /// The remote's main branch: "<author name>|<author email>|<subject>" of each commit, newest first.
    fn remote_log(&self) -> Vec<String> {
        let log = self.git(&self.path("remote.git"), &["log", "--format=%an|%ae|%s", "main"]);
        log.lines().map(str::to_string).collect()
    }

    fn remote_head(&self) -> String {
        self.git(&self.path("remote.git"), &["rev-parse", "main"])
    }

    /// `git status --porcelain` of the repository at `dir`, one entry per line.
    fn dirty(&self, dir: &Path) -> Vec<String> {
        let text = self.git(dir, &["status", "--porcelain", "--untracked-files=all"]);
        text.lines().map(str::to_string).collect()
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn write(path: &Path, content: impl AsRef<[u8]>) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(path, content).unwrap();
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// Installs a hook script in the repository at `dir`.
#[cfg(unix)]
fn hook(dir: &Path, name: &str, script: &str) {
    use std::os::unix::fs::PermissionsExt;
    let path = dir.join(".git/hooks").join(name);
    write(&path, format!("#!/bin/sh\n{script}\n"));
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

// ---------------------------------------------------------------------------
// The app.

struct Harness {
    ctx: egui::Context,
    app: App,
    sb: Sandbox,
}

impl Harness {
    fn new(name: &str) -> Self {
        let sb = Sandbox::new(name);
        let ctx = egui::Context::default();
        let mut app = App::for_tests(&ctx);
        // The worker the first job would start, with git kept away from the user's configuration.
        app.git.worker = Some(GitWorker::spawn_isolated(ctx.clone(), &sb.root));
        let mut h = Self { ctx, app, sb };
        h.frame();
        h
    }

    /// One frame of the whole window, exactly as eframe runs it.
    fn frame(&mut self) {
        self.frame_with(Vec::new());
    }

    fn frame_with(&mut self, events: Vec<egui::Event>) {
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(1600.0, 968.0))),
            events,
            ..Default::default()
        };
        let app = &mut self.app;
        self.ctx.run_ui(input, |ui| app.frame(ui)).drop_without_applying_deltas();
    }

    /// Keyboard input to the editor (it has the focus, as after a click into the text).
    fn keys(&mut self, events: Vec<egui::Event>) {
        self.ctx.memory_mut(|m| m.request_focus(crate::ide::editor_id()));
        self.frame_with(events);
        self.frame();
    }

    fn type_text(&mut self, text: &str) {
        self.keys(vec![egui::Event::Text(text.into())]);
    }

    /// ⌘Z / Ctrl+Z in the editor.
    fn undo(&mut self) {
        let key = egui::Event::Key {
            key: egui::Key::Z,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::COMMAND,
        };
        self.keys(vec![key]);
    }

    /// Puts the editor's cursor at a line and column (0-based), like a click there.
    fn cursor_at(&mut self, line: usize, col: usize) {
        self.app.jump_to = Some((line, col));
        self.frame();
    }

    /// Runs frames until `done` holds (the worker answers in the background meanwhile).
    fn until(&mut self, what: &str, done: impl Fn(&App) -> bool) {
        let start = Instant::now();
        loop {
            self.frame();
            if done(&self.app) {
                return;
            }
            assert!(start.elapsed() < TIMEOUT, "timed out waiting for {what}: {}", self.describe());
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    /// Runs frames for a while (to see that something does *not* happen).
    fn idle_for(&mut self, time: Duration) {
        let start = Instant::now();
        while start.elapsed() < time {
            self.frame();
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    /// Presses the button labelled `label` in the next frame; false when it is not shown or disabled.
    fn press(&mut self, label: &str) -> bool {
        PRESS.with(|p| *p.borrow_mut() = Some(label.to_string()));
        self.frame();
        PRESS.with(|p| p.borrow_mut().take()).is_none()
    }

    fn open(&mut self, path: &Path) {
        let ctx = self.ctx.clone();
        self.app.open_path(path, &ctx);
        self.frame();
    }

    fn menu(&mut self, action: MenuAction) {
        let ctx = self.ctx.clone();
        self.app.run(action, &ctx);
        self.frame();
    }

    /// The open project's status has been read and no job runs.
    fn settled(&mut self) {
        self.until("the status to be read", |a| {
            a.git.status_known && a.git.busy.is_none() && a.git.status.is_some()
        });
    }

    fn status(&self) -> &RepoStatus {
        self.app
            .git
            .status
            .as_ref()
            .unwrap_or_else(|| panic!("no status: {}", self.describe()))
    }

    fn toast(&self) -> String {
        self.app.toast.as_ref().map(|t| t.0.clone()).unwrap_or_default()
    }

    fn doc(&self) -> &OpenDoc {
        self.app
            .doc
            .as_ref()
            .unwrap_or_else(|| panic!("no open document: {}", self.describe()))
    }

    fn dialog_name(&self) -> &'static str {
        match &self.app.git.dialog {
            None => "none",
            Some(Dialog::Pull(_)) => "pull",
            Some(Dialog::Push(_)) => "push",
            Some(Dialog::Link(_)) => "link",
            Some(Dialog::Clone(_)) => "clone",
        }
    }

    fn push_dialog(&mut self) -> &mut PushDialog {
        assert_eq!(self.dialog_name(), "push", "{}", self.describe());
        match &mut self.app.git.dialog {
            Some(Dialog::Push(d)) => d,
            _ => unreachable!(),
        }
    }

    /// What the app is up to, for failure messages.
    fn describe(&self) -> String {
        let g = &self.app.git;
        let st = g.status.as_ref().map(|s| {
            format!(
                "linked={} branch={:?} unborn={} ahead={} behind={} changes={:?} last_push={:?} in_progress={:?}",
                s.linked(),
                s.branch,
                s.unborn,
                s.ahead,
                s.behind,
                s.changes,
                s.last_push,
                s.in_progress
            )
        });
        let error = match &g.dialog {
            Some(Dialog::Push(d)) => format!("{:?} needs_pull={}", d.error, d.needs_pull),
            Some(Dialog::Link(d)) => format!("{:?}", d.error),
            Some(Dialog::Clone(d)) => format!("{:?}", d.error),
            _ => String::new(),
        };
        format!(
            "project={:?} busy={:?} dialog={} {error} auth={} status_known={} status={st:?} status_error={:?} fetch_error={:?} toast={:?}",
            g.project,
            g.busy,
            self.dialog_name(),
            g.auth.is_some(),
            g.status_known,
            g.status_error,
            g.fetch_error,
            self.app.toast.as_ref().map(|t| &t.0),
        )
    }
}

const ME: (&str, &str) = ("Erika Mustermann", "erika@example.com");

/// Seven lines, so that edits on line 1 and line 5 merge without touching each other.
const TEXT: &str = "= Titel\n\nZeile 2\nZeile 3\nZeile 4\nZeile 5\nZeile 6\n";

const PNG: &[u8] = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR base";
const PNG_HERE: &[u8] = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR hier";
const PNG_THERE: &[u8] = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR dort";

// ---------------------------------------------------------------------------
// Scenarios.

/// 1. A plain folder linked to an empty repository through the link dialog, then the first push.
#[test]
fn link_a_folder_and_push_it() {
    let mut h = Harness::new("link-push");
    let url = h.sb.bare();
    let dir = h.sb.path("Projekt");
    write(&dir.join("main.typ"), "= Hallo\n");
    h.open(&dir);
    assert_eq!(h.app.project.as_ref().map(|p| p.root.clone()), Some(dir.clone()));
    // Not a repository: git is not even asked, and there are no Pull/Push buttons.
    h.idle_for(Duration::from_millis(100));
    assert!(h.app.git.status.is_none() && !h.app.git.in_repo, "{}", h.describe());
    assert!(!h.press(PULL) && !h.press(PUSH));

    // Pull from the menu offers to link the project first.
    h.menu(MenuAction::GitPull);
    assert_eq!(h.dialog_name(), "link");
    if let Some(Dialog::Link(d)) = &mut h.app.git.dialog {
        assert_eq!(d.branch, "main");
        d.url = url.clone();
    }
    assert!(h.press("Verbinden"), "{}", h.describe());
    assert_eq!(h.app.git.busy, Some(Busy::Link));
    h.until("the project to be linked", |a| {
        a.git.linked() && a.git.busy.is_none() && a.git.dialog.is_none()
    });
    assert!(h.toast().starts_with("Verbunden mit"), "{}", h.describe());
    assert!(dir.join(".git").is_dir());
    assert_eq!(h.sb.git(&dir, &["config", "branch.main.merge"]), "refs/heads/main");
    assert!(!h.status().remote_branch_exists);

    // Push: the dialog lists the new files; message and identity are typed in.
    assert!(h.press(PUSH), "{}", h.describe());
    let d = h.push_dialog();
    assert!(d.name.is_empty() && d.email.is_empty(), "nothing configured in the isolated git");
    (d.name, d.email) = (ME.0.into(), ME.1.into());
    h.app.git.push_message = "Erster Stand".into();
    let mut listed: Vec<String> = h.status().changes.iter().map(|c| c.path.clone()).collect();
    listed.sort();
    assert_eq!(listed, [".gitignore", "main.typ"]);
    assert!(h.press("Pushen"), "{}", h.describe());
    assert_eq!(h.app.git.busy, Some(Busy::Push));
    h.until("the push", |a| a.git.busy.is_none() && a.git.dialog.is_none());
    assert_eq!(h.toast(), "Gepusht: 1 Commit.", "{}", h.describe());
    assert_eq!(h.sb.remote_log(), ["Erika Mustermann|erika@example.com|Erster Stand"]);
    assert_eq!(h.sb.remote_head(), h.sb.head(&dir));
    assert!(h.app.git.push_message.is_empty());
    // Name and address are remembered for the next push.
    assert_eq!((h.app.settings.git_name.as_str(), h.app.settings.git_email.as_str()), ME);
    assert_eq!(h.app.settings.git_emails, [ME.1]);
    // The Push button's pill shows the age of this push.
    h.until("the status after the push", |a| {
        a.git
            .status
            .as_ref()
            .is_some_and(|s| s.last_push.is_some() && s.changes.is_empty() && s.remote_branch_exists)
    });
    let pushed = h.status().last_push.unwrap();
    assert_eq!(git::relative_time(unix_now().saturating_sub(pushed)), "jetzt");
    assert!(h.status().ahead == 0 && h.status().behind == 0);
    // Nothing left to push: the dialog says so and can't push.
    assert!(h.press(PUSH), "{}", h.describe());
    h.idle_for(Duration::from_millis(100));
    assert!(!h.press("Pushen"), "{}", h.describe());
    assert!(h.press("Abbrechen"));
    assert_eq!(h.dialog_name(), "none");
}

/// 1b. Linking to a repository that already has files brings them into the folder (and the
/// sidebar); files with other content on both sides are refused, with the reason in the dialog.
#[test]
fn link_to_a_repository_with_files() {
    let mut h = Harness::new("link-adopt");
    let url = h.sb.seeded(&[
        ("README.md", b"# Projekt\n"),
        ("bilder/logo.png", PNG),
        ("main.typ", b"= Aus dem Repository\n"),
    ]);
    let dir = h.sb.path("Projekt");
    write(&dir.join("main.typ"), "= Hier\n");
    h.open(&dir);

    // main.typ differs on both sides: refused, nothing left behind, the dialog says why.
    h.menu(MenuAction::GitPull);
    if let Some(Dialog::Link(d)) = &mut h.app.git.dialog {
        d.url = url.clone();
    }
    assert!(h.press("Verbinden"), "{}", h.describe());
    h.until("the refusal", |a| {
        a.git.busy.is_none() && matches!(&a.git.dialog, Some(Dialog::Link(d)) if d.error.is_some())
    });
    let Some(Dialog::Link(d)) = &h.app.git.dialog else { unreachable!() };
    assert!(d.error.as_deref().unwrap().contains("main.typ"), "{:?}", d.error);
    assert!(!dir.join(".git").exists() && !dir.join(".gitignore").exists());
    assert!(!h.app.git.linked());

    // Without the clash the repository's files come in; the local file stays as it is.
    std::fs::remove_file(dir.join("main.typ")).unwrap();
    write(&dir.join("notizen.typ"), "Notizen\n");
    h.app.close_tab("main.typ");
    let ctx = h.ctx.clone();
    h.app.open_project(dir.clone(), Some("notizen.typ".into()), &ctx);
    h.frame();
    h.menu(MenuAction::GitPull);
    if let Some(Dialog::Link(d)) = &mut h.app.git.dialog {
        d.url = url.clone();
    }
    assert!(h.press("Verbinden"), "{}", h.describe());
    h.until("the project to be linked", |a| {
        a.git.linked() && a.git.busy.is_none() && a.git.dialog.is_none()
    });
    assert_eq!(read(&dir.join("README.md")), "# Projekt\n");
    assert_eq!(read(&dir.join("main.typ")), "= Aus dem Repository\n");
    assert_eq!(read(&dir.join("notizen.typ")), "Notizen\n");
    assert_eq!(h.doc().text, "Notizen\n");
    let files: Vec<String> = h.app.project.as_ref().unwrap().files.iter().map(|f| f.rel.clone()).collect();
    for rel in ["README.md", "bilder", "bilder/logo.png", "main.typ", "notizen.typ"] {
        assert!(files.iter().any(|f| f == rel), "{rel} is missing in the sidebar: {files:?}");
    }
    // (The new repository's .gitignore is Typisch's own, uncommitted like the local file.)
    assert_eq!(h.sb.dirty(&dir), ["?? .gitignore", "?? notizen.typ"]);
}

/// 2. Someone else pushed; the background fetch shows it on the Pull badge. Pull with an unsaved
///    edit in the open document on another line: both edits end up in the editor and on disk.
#[test]
fn pull_into_the_open_document() {
    let mut h = Harness::new("pull-open");
    let url = h.sb.seeded(&[("main.typ", TEXT.as_bytes())]);
    let a = h.sb.clone(&url, "a");
    let b = h.sb.clone(&url, "b");
    write(&b.join("main.typ"), TEXT.replace("= Titel", "= Neuer Titel"));
    let theirs = h.sb.commit_push(&b, "Titel");

    h.open(&a.join("main.typ"));
    assert_eq!(h.doc().rel, "main.typ");
    // The status first, then the automatic look for new commits: one is waiting.
    h.until("the Pull badge to count the new commit", |a| {
        a.git.status.as_ref().is_some_and(|s| s.linked() && s.behind == 1) && a.git.busy.is_none()
    });

    // Typed at the end of line 5, not saved yet.
    h.cursor_at(5, 7);
    h.type_text(" – hier ergänzt");
    assert_eq!(h.doc().text, TEXT.replace("Zeile 5", "Zeile 5 – hier ergänzt"), "{}", h.describe());
    assert_ne!(h.doc().text, h.doc().saved);
    assert!(h.press(PULL), "{}", h.describe());
    assert!(h.app.git.editor_locked(), "the editor is read-only while the pull runs");
    assert_eq!(
        read(&a.join("main.typ")),
        TEXT.replace("Zeile 5", "Zeile 5 – hier ergänzt"),
        "saved before the pull"
    );
    // Typing while files change underneath goes nowhere.
    h.type_text("X");
    assert!(!h.doc().text.contains('X'), "{}", h.describe());
    h.until("the pull", |a| {
        a.git.busy.is_none() && a.toast.as_ref().is_some_and(|t| t.0.contains("geholt"))
    });

    let want = TEXT
        .replace("= Titel", "= Neuer Titel")
        .replace("Zeile 5", "Zeile 5 – hier ergänzt");
    assert_eq!(h.doc().text, want, "{}", h.describe());
    assert_eq!(h.doc().saved, want);
    assert_eq!(read(&a.join("main.typ")), want);
    assert!(!h.app.git.editor_locked());
    assert!(h.toast().starts_with("1 Commit geholt."), "{}", h.toast());
    // HEAD is the remote's commit and the local edit is still uncommitted.
    assert_eq!(h.sb.head(&a), theirs);
    assert_eq!(h.sb.dirty(&a), [" M main.typ"]);
    h.until("the badge to clear", |a| a.git.status.as_ref().is_some_and(|s| s.behind == 0));
    // Undo can't bring back the text from before the pull (it would undo the other side's edit).
    h.undo();
    h.undo();
    assert_eq!(h.doc().text, want, "{}", h.describe());
    // And the editor takes input again.
    h.type_text("!");
    assert_eq!(h.doc().text.chars().filter(|c| *c == '!').count(), 1, "{}", h.describe());
}

/// 2b. The same line changed here (typed, unsaved) and in the repository: both lines survive.
#[test]
fn pull_keeps_both_versions_of_a_line() {
    let mut h = Harness::new("pull-same-line");
    let url = h.sb.seeded(&[("main.typ", TEXT.as_bytes())]);
    let a = h.sb.clone(&url, "a");
    let b = h.sb.clone(&url, "b");
    write(&b.join("main.typ"), TEXT.replace("Zeile 3", "Zeile 3 von Bert"));
    h.sb.commit_push(&b, "Bert");
    h.open(&a);
    h.settled();
    h.cursor_at(3, 7);
    h.type_text(" von Erika");
    assert!(h.press(PULL), "{}", h.describe());
    h.until("the pull", |a| {
        a.git.busy.is_none() && a.toast.as_ref().is_some_and(|t| t.0.contains("geholt"))
    });
    let text = h.doc().text.clone();
    assert!(
        text.contains("Zeile 3 von Erika\n") && text.contains("Zeile 3 von Bert\n"),
        "{text}"
    );
    assert!(!text.contains("<<<<<<<") && !text.contains(">>>>>>>"), "{text}");
    assert_eq!(read(&a.join("main.typ")), text);
    assert!(h.toast().contains("Zusammengeführt: main.typ – bitte prüfen."), "{}", h.toast());
}

/// 2c. A pull that deletes the main document points the project at another .typ file.
#[test]
fn pull_of_a_deleted_main_document_falls_back() {
    let mut h = Harness::new("pull-main-gone");
    let url = h.sb.seeded(&[("main.typ", b"= Hauptdatei\n"), ("zweit.typ", b"= Zweit\n")]);
    let a = h.sb.clone(&url, "a");
    let b = h.sb.clone(&url, "b");
    std::fs::remove_file(b.join("main.typ")).unwrap();
    h.sb.commit_push(&b, "Hauptdatei entfernt");

    h.open(&a);
    h.settled();
    assert_eq!(h.app.project.as_ref().unwrap().main_rel, "main.typ");
    assert!(h.press(PULL), "{}", h.describe());
    h.until("the pull", |a| {
        a.git.busy.is_none() && a.toast.as_ref().is_some_and(|t| t.0.contains("geholt"))
    });
    assert!(!a.join("main.typ").exists(), "{}", h.describe());
    // The project still has a main document: the remaining .typ file takes over.
    assert_eq!(h.app.project.as_ref().unwrap().main_rel, "zweit.typ", "{}", h.describe());
}

/// 3. A file deleted in the repository but edited here, and a picture changed on both sides:
///    the pull asks; one answer takes the remote side, the other keeps the local one.
#[test]
fn pull_asks_about_files_it_cannot_merge() {
    let mut h = Harness::new("pull-questions");
    let url =
        h.sb.seeded(&[("main.typ", TEXT.as_bytes()), ("kapitel.typ", b"= Kapitel\n"), ("logo.png", PNG)]);
    let a = h.sb.clone(&url, "a");
    let b = h.sb.clone(&url, "b");
    std::fs::remove_file(b.join("kapitel.typ")).unwrap();
    write(&b.join("logo.png"), PNG_THERE);
    let theirs = h.sb.commit_push(&b, "Aufgeräumt");

    h.open(&a.join("main.typ"));
    h.settled();
    // kapitel.typ is edited in its own tab, which then goes to the background (and is saved).
    h.app.open_file("kapitel.typ");
    h.app.doc.as_mut().unwrap().text.push_str("Hier weitergeschrieben.\n");
    h.app.open_file("main.typ");
    h.frame();
    assert_eq!(read(&a.join("kapitel.typ")), "= Kapitel\nHier weitergeschrieben.\n");
    assert_eq!(h.app.tab_order, ["main.typ", "kapitel.typ"]);
    write(&a.join("logo.png"), PNG_HERE);

    // Asked, then cancelled: nothing changes.
    assert!(h.press(PULL), "{}", h.describe());
    h.until("the pull dialog", |a| matches!(a.git.dialog, Some(Dialog::Pull(_))));
    assert!(h.app.git.editor_locked());
    let mine = h.sb.head(&a);
    assert!(h.press("Abbrechen"));
    assert!(h.app.git.dialog.is_none() && !h.app.git.editor_locked(), "{}", h.describe());
    h.idle_for(Duration::from_millis(100));
    assert_eq!(h.sb.head(&a), mine);
    assert_eq!(read(&a.join("kapitel.typ")), "= Kapitel\nHier weitergeschrieben.\n");
    assert_eq!(h.app.tab_order, ["main.typ", "kapitel.typ"]);

    assert!(h.press(PULL), "{}", h.describe());
    h.until("the pull dialog", |a| matches!(a.git.dialog, Some(Dialog::Pull(_))));
    let Some(Dialog::Pull(d)) = &mut h.app.git.dialog else {
        unreachable!()
    };
    assert_eq!(
        d.plan.questions,
        [
            Question::DeletedInRepo("kapitel.typ".into()),
            Question::BinaryChanged("logo.png".into())
        ]
    );
    assert_eq!(d.choices, [Choice::Local, Choice::Local], "the safe answers are preselected");
    // Delete the chapter as in the repository, keep this computer's picture.
    d.choices = vec![Choice::Remote, Choice::Local];
    assert!(h.press("Übernehmen"), "{}", h.describe());
    h.until("the pull", |a| {
        a.git.busy.is_none() && a.toast.as_ref().is_some_and(|t| t.0.contains("geholt"))
    });

    assert!(!a.join("kapitel.typ").exists());
    assert_eq!(std::fs::read(a.join("logo.png")).unwrap(), PNG_HERE);
    assert_eq!(h.sb.head(&a), theirs);
    assert_eq!(h.sb.dirty(&a), [" M logo.png"]);
    // The deleted chapter's tab is closed, without writing it back.
    assert_eq!(h.app.tab_order, ["main.typ"]);
    assert!(h.app.tabs.is_empty());
    assert_eq!(h.doc().rel, "main.typ");
    h.idle_for(Duration::from_millis(200));
    assert!(!a.join("kapitel.typ").exists());
    assert_eq!(
        h.toast(),
        "1 Commit geholt. Aus dem Repository übernommen: kapitel.typ. Deine Version behalten: logo.png.",
    );
    assert!(h.app.git.dialog.is_none() && !h.app.git.editor_locked());
}

/// 4. Push while the repository has news: nothing is committed, the dialog says to pull first
///    and keeps the message; after "pull now" the push goes through.
#[test]
fn push_when_behind_asks_to_pull_first() {
    let mut h = Harness::new("push-behind");
    let url = h.sb.seeded(&[("main.typ", TEXT.as_bytes())]);
    let a = h.sb.clone(&url, "a");
    let b = h.sb.clone(&url, "b");
    write(&b.join("notizen.typ"), "Notizen\n");
    h.sb.commit_push(&b, "Notizen");
    // No look at the repository yet: the push itself finds out.
    h.app.settings.git_auto_fetch = false;
    let mine = h.sb.head(&a);

    h.open(&a);
    h.settled();
    assert_eq!(h.status().behind, 0, "not fetched yet");
    let doc = h.app.doc.as_mut().unwrap();
    doc.text = doc.text.replace("Zeile 3", "Zeile 3 geändert");
    assert!(h.press(PUSH), "{}", h.describe());
    let d = h.push_dialog();
    (d.name, d.email) = (ME.0.into(), ME.1.into());
    h.app.git.push_message = "Zeile 3".into();
    assert_eq!(
        read(&a.join("main.typ")),
        TEXT.replace("Zeile 3", "Zeile 3 geändert"),
        "saved when the dialog opened"
    );
    // The dialog was opened with the status from before that save; the fresh one follows.
    h.until("the dialog to list the change", |a| {
        a.git.status.as_ref().is_some_and(|s| !s.changes.is_empty())
    });
    assert!(h.press("Pushen"), "{}", h.describe());
    h.until("the refused push", |a| {
        a.git.busy.is_none() && matches!(&a.git.dialog, Some(Dialog::Push(d)) if d.needs_pull)
    });

    assert_eq!(h.sb.head(&a), mine, "no commit was made");
    assert_eq!(h.sb.dirty(&a), [" M main.typ"]);
    assert_eq!(h.app.git.push_message, "Zeile 3");
    assert!(!h.press("Pushen"), "pushing again is pointless before the pull");
    assert!(h.status().behind >= 1, "{}", h.describe());

    // "Pull now" closes the dialog and pulls; the message waits for the next push.
    assert!(h.press("Jetzt holen (Pull)"), "{}", h.describe());
    assert_eq!(h.dialog_name(), "none");
    h.until("the pull", |a| {
        a.git.busy.is_none() && a.toast.as_ref().is_some_and(|t| t.0.contains("geholt"))
    });
    assert!(a.join("notizen.typ").exists());
    assert_eq!(h.app.git.push_message, "Zeile 3");

    h.until("the status after the pull", |a| {
        a.git.status.as_ref().is_some_and(|s| s.behind == 0 && !s.changes.is_empty())
    });
    assert!(h.press(PUSH), "{}", h.describe());
    let d = h.push_dialog();
    assert_eq!((d.name.as_str(), d.email.as_str()), ME, "remembered");
    assert!(h.press("Pushen"), "{}", h.describe());
    h.until("the push", |a| a.git.busy.is_none() && a.git.dialog.is_none());
    assert_eq!(h.toast(), "Gepusht: 1 Commit.", "{}", h.describe());
    assert_eq!(h.sb.remote_log()[0], "Erika Mustermann|erika@example.com|Zeile 3");
    assert!(h.sb.dirty(&a).is_empty());
}

/// 5. Cloning from the menu opens the clone as the project.
#[test]
fn clone_opens_the_project() {
    let mut h = Harness::new("clone");
    let url = h.sb.seeded(&[("main.typ", TEXT.as_bytes()), ("kapitel/eins.typ", b"Eins\n")]);
    let parent = h.sb.path("Klone");
    std::fs::create_dir_all(&parent).unwrap();
    assert!(h.app.project.is_none());

    // A folder of that name with something in it: refused, said in the dialog, left alone.
    write(&parent.join("remote/notiz.txt"), "meins");
    h.menu(MenuAction::GitClone);
    assert_eq!(h.dialog_name(), "clone");
    if let Some(Dialog::Clone(d)) = &mut h.app.git.dialog {
        d.url = url.clone();
        d.parent = parent.clone();
    }
    assert!(h.press("Klonen"), "{}", h.describe());
    h.until("the refusal", |a| {
        a.git.busy.is_none() && matches!(&a.git.dialog, Some(Dialog::Clone(d)) if d.error.is_some())
    });
    assert!(h.app.project.is_none());
    assert_eq!(read(&parent.join("remote/notiz.txt")), "meins");
    std::fs::remove_dir_all(parent.join("remote")).unwrap();

    assert!(h.press("Klonen"), "{}", h.describe());
    assert_eq!(h.app.git.busy, Some(Busy::Clone));
    h.until("the clone to open", |a| a.project.is_some() && a.git.busy.is_none());
    let dest = parent.join("remote");
    assert_eq!(h.app.project.as_ref().map(|p| p.root.clone()), Some(dest.clone()));
    assert_eq!(h.dialog_name(), "none");
    assert_eq!(h.doc().rel, "main.typ");
    assert_eq!(h.app.settings.git_clone_dir.as_deref(), Some(parent.as_path()));
    h.settled();
    assert!(h.status().linked(), "{}", h.describe());
    assert_eq!(h.status().repo_root, dest);
    assert_eq!(read(&dest.join("kapitel/eins.typ")), "Eins\n");
}

/// 6a. Opening another project while a pull is being prepared: its reply is dropped and the
/// editor is not left locked.
#[test]
fn switching_projects_drops_the_old_reply() {
    let mut h = Harness::new("switch");
    let url = h.sb.seeded(&[("main.typ", TEXT.as_bytes())]);
    let a = h.sb.clone(&url, "a");
    let other = h.sb.clone(&url, "anderes");
    let b = h.sb.clone(&url, "b");
    write(&b.join("notizen.typ"), "Notizen\n");
    h.sb.commit_push(&b, "Notizen");
    let mine = h.sb.head(&a);

    h.open(&a);
    h.settled();
    assert!(h.press(PULL), "{}", h.describe());
    assert!(h.app.git.editor_locked());
    h.open(&other);
    // (The other project's own background fetch may already run.)
    assert!(!h.app.git.editor_locked() && !h.app.git.blocking(), "{}", h.describe());
    // The other project's status is asked after the pull's plan, so once it is here the plan's
    // reply has been seen too.
    h.until("the other project's status", |app| {
        app.git.status.as_ref().is_some_and(|s| s.repo_root == other)
    });
    h.idle_for(Duration::from_millis(200));
    assert!(
        !h.app.git.blocking() && h.app.git.dialog.is_none() && !h.app.git.editor_locked(),
        "{}",
        h.describe()
    );
    assert_eq!(h.sb.head(&a), mine, "the forgotten pull was not carried out");
    assert!(!h.toast().contains("geholt"));
}

/// 6b. Away and back again while the pull is still being prepared: the reply belongs to a job the
/// window has forgotten (no spinner, editor unlocked), so it must not start a pull on its own.
#[cfg(unix)]
#[test]
fn switching_back_does_not_resume_a_forgotten_pull() {
    let mut h = Harness::new("switch-back");
    let url = h.sb.seeded(&[("main.typ", TEXT.as_bytes())]);
    let a = h.sb.clone(&url, "a");
    let other = h.sb.clone(&url, "anderes");
    let b = h.sb.clone(&url, "b");
    write(&b.join("notizen.typ"), "Notizen\n");
    h.sb.commit_push(&b, "Notizen");
    let mine = h.sb.head(&a);
    // Only the pull's own fetch moves a ref, and that takes a while.
    h.app.settings.git_auto_fetch = false;
    hook(&a, "reference-transaction", "[ \"$1\" = committed ] && sleep 2\nexit 0");

    h.open(&a);
    h.settled();
    assert!(h.press(PULL), "{}", h.describe());
    h.open(&other);
    h.open(&a);
    assert!(h.app.git.busy.is_none() && !h.app.git.editor_locked(), "{}", h.describe());
    // A's new status is asked after the plan, so once it is here the plan's reply was seen.
    h.until("the status read after the switch", |app| app.git.status_known);
    h.idle_for(Duration::from_millis(300));
    assert!(
        h.app.git.busy.is_none() && h.app.git.dialog.is_none() && !h.app.git.editor_locked(),
        "{}",
        h.describe()
    );
    assert_eq!(h.sb.head(&a), mine, "a pull nobody waits for anymore was started");
}

/// Makes a pull slow: its backup of the local work takes two seconds to be written and removed.
#[cfg(unix)]
fn slow_pull(repo: &Path) {
    hook(
        repo,
        "reference-transaction",
        "[ \"$1\" = committed ] || exit 0\nif grep refs/typisch/backup >/dev/null; then sleep 2; fi\nexit 0",
    );
}

/// 6c. Away and back again while the pull itself runs: the files are still changing, so the
/// editor stays read-only until it is done, and then shows the pulled text.
#[cfg(unix)]
#[test]
fn switching_back_during_a_pull_waits_for_it() {
    let mut h = Harness::new("switch-back-pulling");
    let url = h.sb.seeded(&[("main.typ", TEXT.as_bytes())]);
    let a = h.sb.clone(&url, "a");
    let other = h.sb.clone(&url, "anderes");
    let b = h.sb.clone(&url, "b");
    write(&b.join("main.typ"), TEXT.replace("= Titel", "= Neuer Titel"));
    let theirs = h.sb.commit_push(&b, "Titel");
    slow_pull(&a);

    h.open(&a);
    h.settled();
    let doc = h.app.doc.as_mut().unwrap();
    doc.text = doc.text.replace("Zeile 5", "Zeile 5 – hier ergänzt");
    assert!(h.press(PULL), "{}", h.describe());
    h.until("the pull itself", |a| a.git.busy == Some(Busy::Pull));
    h.open(&other);
    assert!(!h.app.git.editor_locked(), "{}", h.describe());
    h.open(&a);
    assert!(h.app.git.editor_locked(), "the pull still changes these files: {}", h.describe());
    h.type_text("X");
    assert!(!h.doc().text.contains('X'));
    h.until("the pull to end", |app| !app.git.editor_locked());
    let want = TEXT
        .replace("= Titel", "= Neuer Titel")
        .replace("Zeile 5", "Zeile 5 – hier ergänzt");
    assert_eq!(read(&a.join("main.typ")), want);
    assert_eq!(h.doc().text, want, "{}", h.describe());
    assert_eq!(h.doc().saved, want);
    assert_eq!(h.sb.head(&a), theirs);
    assert!(h.app.git.busy.is_none() || h.app.git.busy == Some(Busy::Fetch), "{}", h.describe());
}

/// 6d. Two projects in one repository: while the first one pulls (which merges the whole
/// repository), the second is opened. Its files change too, so it waits, then shows them.
#[cfg(unix)]
#[test]
fn opening_a_sibling_project_during_a_pull_waits_for_it() {
    let mut h = Harness::new("sibling");
    let url =
        h.sb.seeded(&[("eins/main.typ", TEXT.as_bytes()), ("zwei/main.typ", TEXT.as_bytes())]);
    let a = h.sb.clone(&url, "a");
    let b = h.sb.clone(&url, "b");
    write(&b.join("zwei/main.typ"), TEXT.replace("= Titel", "= Zwei von Bert"));
    h.sb.commit_push(&b, "Zwei");
    slow_pull(&a);

    h.open(&a.join("eins"));
    h.settled();
    let doc = h.app.doc.as_mut().unwrap();
    doc.text = doc.text.replace("Zeile 5", "Zeile 5 – hier ergänzt");
    assert!(h.press(PULL), "{}", h.describe());
    h.until("the pull itself", |a| a.git.busy == Some(Busy::Pull));
    // The second project is read while the pull is still on its way.
    h.open(&a.join("zwei"));
    assert_eq!(h.doc().text, TEXT);
    assert!(h.app.git.editor_locked(), "the pull still changes these files: {}", h.describe());
    h.until("the pull to end", |app| !app.git.editor_locked());
    let want = TEXT.replace("= Titel", "= Zwei von Bert");
    assert_eq!(read(&a.join("zwei/main.typ")), want);
    assert_eq!(h.doc().text, want, "the editor must show what the pull wrote: {}", h.describe());
    assert_eq!(h.doc().saved, want);
    // Typing and saving now cannot undo the pulled change.
    h.cursor_at(6, 7);
    h.type_text("!");
    let ctx = h.ctx.clone();
    h.app.run(MenuAction::Save, &ctx);
    assert!(read(&a.join("zwei/main.typ")).starts_with("= Zwei von Bert\n"));
}

/// 1c. An ssh server that refuses the key: the explanation, then the reason in the link dialog;
/// nothing is left behind.
#[cfg(unix)]
#[test]
fn link_over_ssh_without_a_key() {
    use std::os::unix::fs::PermissionsExt;
    let mut h = Harness::new("ssh-refused");
    // Instead of a server: an ssh that refuses, configured where the isolated git looks.
    let ssh = h.sb.path("ssh.sh");
    write(
        &ssh,
        "#!/bin/sh\necho 'git@git.example.com: Permission denied (publickey).' >&2\nexit 255\n",
    );
    std::fs::set_permissions(&ssh, std::fs::Permissions::from_mode(0o755)).unwrap();
    std::fs::write(h.sb.path("gitconfig"), format!("[core]\n\tsshCommand = {}\n", ssh.display())).unwrap();
    let dir = h.sb.path("Projekt");
    write(&dir.join("main.typ"), "= Hallo\n");
    h.open(&dir);

    h.menu(MenuAction::GitPush);
    assert_eq!(h.dialog_name(), "link");
    if let Some(Dialog::Link(d)) = &mut h.app.git.dialog {
        d.url = "git@git.example.com:erika/projekt.git".into();
    }
    assert!(h.press("Verbinden"), "{}", h.describe());
    h.until("the ssh refusal", |a| a.git.busy.is_none() && matches!(a.git.auth, Some(Auth::Ssh)));
    assert!(!dir.join(".git").exists(), "the failed link left a repository behind");
    assert!(h.press("OK"), "{}", h.describe());
    assert!(h.app.git.auth.is_none());
    let Some(Dialog::Link(d)) = &h.app.git.dialog else {
        panic!("{}", h.describe())
    };
    assert_eq!(d.error.as_deref(), Some("Der Server hat den SSH-Schlüssel abgelehnt."));
}

/// 2c. The background look for new commits: every five minutes and on coming back to the window
/// after a minute, never with the setting off.
#[test]
fn background_fetch_follows_the_timers() {
    let mut h = Harness::new("fetch-timers");
    let url = h.sb.seeded(&[("main.typ", TEXT.as_bytes())]);
    let a = h.sb.clone(&url, "a");
    let b = h.sb.clone(&url, "b");
    h.open(&a);
    h.until("the first look", |a| {
        a.git.last_fetch.is_some() && a.git.busy.is_none() && a.git.status_known
    });
    let behind = |a: &App| a.git.status.as_ref().map_or(0, |s| s.behind);

    write(&b.join("eins.typ"), "1");
    h.sb.commit_push(&b, "Eins");
    h.idle_for(Duration::from_millis(300));
    assert_eq!(behind(&h.app), 0, "the next look is minutes away");
    h.app.git.last_fetch = Instant::now().checked_sub(FETCH_EVERY);
    h.until("the periodic look", |a| behind(a) == 1);

    // Away from the window and back after more than a minute: look again.
    write(&b.join("zwei.typ"), "2");
    h.sb.commit_push(&b, "Zwei");
    h.until("the look to end", |a| a.git.busy.is_none());
    let away = egui::RawInput {
        focused: false,
        ..Default::default()
    };
    let app = &mut h.app;
    h.ctx.run_ui(away, |ui| app.frame(ui)).drop_without_applying_deltas();
    h.app.git.last_fetch = Instant::now().checked_sub(FETCH_ON_FOCUS + Duration::from_secs(1));
    h.until("the look on coming back", |a| behind(a) == 2);

    // Switched off: nothing, however long ago.
    h.app.settings.git_auto_fetch = false;
    write(&b.join("drei.typ"), "3");
    h.sb.commit_push(&b, "Drei");
    h.until("the look to end", |a| a.git.busy.is_none());
    h.app.git.last_fetch = Instant::now().checked_sub(FETCH_EVERY * 2);
    h.idle_for(Duration::from_millis(400));
    assert_eq!(behind(&h.app), 2, "{}", h.describe());
}

/// 7. Disconnecting in the settings: Pull and Push disappear, the repository stays.
#[test]
fn unlink_hides_pull_and_push() {
    let mut h = Harness::new("unlink");
    let url = h.sb.seeded(&[("main.typ", TEXT.as_bytes())]);
    let a = h.sb.clone(&url, "a");
    h.open(&a);
    h.settled();
    assert!(h.status().linked());

    h.menu(MenuAction::GitSettings);
    assert!(h.app.settings_open);
    assert!(h.press("Verbindung trennen …"), "{}", h.describe());
    assert!(h.app.git.confirm_unlink);
    assert!(h.press("Trennen"), "{}", h.describe());
    h.until("the unlink", |a| {
        a.git.busy.is_none() && a.git.status_known && a.git.status.as_ref().is_some_and(|s| !s.linked())
    });
    assert_eq!(h.toast(), "Die Verbindung zum Repository ist getrennt.");
    assert!(!h.app.git.confirm_unlink);
    assert!(a.join(".git").is_dir());
    assert!(!h.sb.output(&a, &["config", "--get", "branch.main.remote"]).status.success());
    assert_eq!(h.sb.git(&a, &["remote"]), "origin");
    h.app.settings_open = false;
    assert!(!h.press(PULL) && !h.press(PUSH), "the buttons are gone");
    // Pull from the menu now offers to link again, suggesting the old address.
    h.menu(MenuAction::GitPull);
    assert_eq!(h.dialog_name(), "link");
    if let Some(Dialog::Link(d)) = &h.app.git.dialog {
        assert_eq!(d.url, url);
    }
}
