//! Git Sync in the window: the Pull and Push buttons in the top bar, their dialogs, the Git page
//! of the settings and the timers that keep the status fresh. All git work runs on the worker
//! thread (git.rs); this module only sends jobs and applies the replies.

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use eframe::egui::{self, Align, Align2, Color32, FontId, Galley, Layout, Rect, RichText, Sense, Stroke, Ui, pos2, vec2};

use crate::app::{App, OpenDoc, Screen, SettingsTab, bold, tilde};
use crate::git::{
    self, ChangedFile, Change, Choice, Credentials, GitError, GitWorker, Identity, Job, PullOutcome, PullPlan, Question,
    Reply, RepoStatus,
};
use crate::icons::{Icon, paint};
use crate::menu::{PopupItem, popup_button};
use crate::screens::{group, note, section, sep, setting_row, toggle};
use crate::theme::{Settings, pal};
use crate::widgets::{
    Button, CONTROL_H, RADIUS, RADIUS_XL, TOOL, TOOL_GAP, TOOL_ICON, ToolButton, bar_separator, fg, field, flat_fill,
    secret_field, segmented,
};

/// How often the status is read while the window is in front.
const STATUS_EVERY: Duration = Duration::from_secs(15);
/// How often a linked project looks for new commits (setting `git_auto_fetch`).
const FETCH_EVERY: Duration = Duration::from_secs(5 * 60);
/// Coming back to the window looks again if the last look is older than this.
const FETCH_ON_FOCUS: Duration = Duration::from_secs(60);
/// Saves come in bursts (autosave, tab switches); one status read after them is enough.
const AFTER_SAVE: Duration = Duration::from_secs(1);

const LINK_TEXT: &str = "Verbinde dieses Projekt mit einem Repository auf GitHub, GitLab oder einem anderen Git-Server. Das Repository muss bereits existieren.";

/// What git is doing for the user right now.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Busy {
    /// A background look for new commits; it blocks nothing.
    Fetch,
    Plan,
    Pull,
    Push,
    Link,
    Unlink,
    Clone,
}

/// A job the user started, kept so it can be sent again after logging in.
#[derive(Clone, Debug)]
enum Action {
    Plan,
    Pull { answers: Vec<(String, Choice)>, identity: Identity },
    Push { message: String, identity: Identity },
    Link { url: String, branch: String },
    Clone { url: String, branch: Option<String>, dest: PathBuf },
}

impl Action {
    fn busy(&self) -> Busy {
        match self {
            Action::Plan => Busy::Plan,
            Action::Pull { .. } => Busy::Pull,
            Action::Push { .. } => Busy::Push,
            Action::Link { .. } => Busy::Link,
            Action::Clone { .. } => Busy::Clone,
        }
    }
}

/// A job the user started for a project whose answer has not come yet.
struct Awaited {
    kind: Busy,
    /// The repository it works in (None for a folder that is not one yet).
    repo: Option<PathBuf>,
}

/// The questions of a pull and the user's answers (same order).
pub struct PullDialog {
    plan: PullPlan,
    choices: Vec<Choice>,
}

impl PullDialog {
    fn new(plan: PullPlan) -> Self {
        // The non-destructive answer is preselected: keep what is on this computer.
        let choices = vec![Choice::Local; plan.questions.len()];
        Self { plan, choices }
    }
}

/// The commit message lives in [`GitUi::push_message`], so it survives a "pull first".
pub struct PushDialog {
    name: String,
    email: String,
    needs_pull: bool,
    error: Option<String>,
    focus: bool,
}

pub struct LinkDialog {
    url: String,
    branch: String,
    error: Option<String>,
    focus: bool,
}

pub struct CloneDialog {
    url: String,
    branch: String,
    /// Folder name; follows the address while `auto` (until the user edits it).
    name: String,
    auto: bool,
    parent: PathBuf,
    error: Option<String>,
    focus: bool,
}

pub enum Dialog {
    Pull(PullDialog),
    Push(PushDialog),
    Link(LinkDialog),
    Clone(CloneDialog),
}

/// Asked on top of whatever dialog is open when the server wants a login.
pub enum Auth {
    Login { host: String, username: String, password: String, failed: bool, focus: bool },
    Ssh,
}

/// Git Sync state of the open project, plus the dialogs.
pub struct GitUi {
    /// Check pictures show made-up state and never run git.
    screenshot: bool,
    ctx: Option<egui::Context>,
    /// Started with the first job, so git never runs before the user needs it.
    worker: Option<GitWorker>,
    /// The project folder the state below belongs to.
    project: Option<PathBuf>,
    /// The project lies in a git work tree (found without running git).
    in_repo: bool,
    /// The root of that work tree.
    repo: Option<PathBuf>,
    status: Option<RepoStatus>,
    /// A status reply arrived for this project (until then "not linked" is not known yet).
    status_known: bool,
    status_error: Option<String>,
    git_missing: bool,
    busy: Option<Busy>,
    last_fetch: Option<Instant>,
    last_status: Option<Instant>,
    status_due: Option<Instant>,
    /// Look for new commits as soon as the next status shows the project linked (after linking).
    fetch_after_status: bool,
    fetch_error: Option<String>,
    dialog: Option<Dialog>,
    auth: Option<Auth>,
    confirm_unlink: bool,
    /// Logins typed this session, by host. Never written to disk by Typisch.
    creds: HashMap<String, Credentials>,
    /// The last job the user started, the host it talks to and whether it carried a login.
    last: Option<(Action, String, bool)>,
    focused: bool,
    /// What was typed in the push dialog, kept for the next attempt.
    push_message: String,
    /// Jobs the user started for a project (all but Clone) that the worker has not answered yet,
    /// oldest first. It answers them in the order they were sent, one reply each.
    awaited: VecDeque<Awaited>,
    /// How many of the oldest of those were started before the last project switch. The window
    /// forgot them then, so their answers are dropped – even when the user has come back to that
    /// project meanwhile. (A forgotten pull still changes files, though: see `forgotten_pull_here`.)
    forgotten: usize,
}

impl GitUi {
    pub fn new(screenshot: bool) -> Self {
        Self {
            screenshot,
            ctx: None,
            worker: None,
            project: None,
            in_repo: false,
            repo: None,
            status: None,
            status_known: false,
            status_error: None,
            git_missing: false,
            busy: None,
            last_fetch: None,
            last_status: None,
            status_due: None,
            fetch_after_status: false,
            fetch_error: None,
            dialog: None,
            auth: None,
            confirm_unlink: false,
            creds: HashMap::new(),
            last: None,
            focused: true,
            push_message: String::new(),
            awaited: VecDeque::new(),
            forgotten: 0,
        }
    }

    /// A file of the project was written: read the status again shortly (the folder may also
    /// have become a repository meanwhile; finding that out runs no git).
    pub fn file_saved(&mut self) {
        if self.project.is_some() {
            self.status_due = Some(Instant::now() + AFTER_SAVE);
        }
    }

    /// While a pull is being prepared or runs, files change underneath the editor: no typing then.
    pub fn editor_locked(&self) -> bool {
        matches!(self.busy, Some(Busy::Plan | Busy::Pull))
            || matches!(self.dialog, Some(Dialog::Pull(_)))
            || self.forgotten_pull_here()
    }

    /// A pull started before the last project switch still runs in the open project's
    /// repository (the same project again, or another folder of that repository).
    fn forgotten_pull_here(&self) -> bool {
        self.repo.is_some() && self.awaited.iter().take(self.forgotten).any(|a| a.kind == Busy::Pull && a.repo == self.repo)
    }

    /// Looks (without git) whether the project lies in a git work tree.
    fn find_repo(&mut self) {
        self.repo = self.project.as_deref().and_then(git::find_repo_root);
        self.in_repo = self.repo.is_some();
    }

    fn linked(&self) -> bool {
        self.status.as_ref().is_some_and(RepoStatus::linked)
    }

    /// A job the user started is running (background fetches don't count).
    fn blocking(&self) -> bool {
        self.busy.is_some_and(|b| b != Busy::Fetch)
    }

    /// False when the job could not be sent (check pictures, or before the first frame).
    fn send(&mut self, job: Job) -> bool {
        if self.screenshot {
            return false;
        }
        let Some(ctx) = &self.ctx else { return false };
        self.worker.get_or_insert_with(|| GitWorker::spawn(ctx.clone())).send(job);
        true
    }

    /// Sends a job the user started for the open project; its answer is awaited.
    fn send_awaited(&mut self, kind: Busy, job: Job) {
        if self.send(job) {
            self.awaited.push_back(Awaited { kind, repo: self.repo.clone() });
        }
    }

    /// The answer to the oldest awaited job came: false when the window had forgotten that job.
    fn answered(&mut self) -> bool {
        self.awaited.pop_front();
        if self.forgotten > 0 {
            self.forgotten -= 1;
            return false;
        }
        true
    }

    /// Another project was opened (or none): forget everything about the old one.
    fn switch(&mut self, project: Option<PathBuf>) {
        let cloning = self.busy == Some(Busy::Clone);
        self.project = project;
        self.find_repo();
        self.forgotten = self.awaited.len();
        self.status = None;
        self.status_known = false;
        self.status_error = None;
        self.busy = if cloning {
            Some(Busy::Clone)
        } else if self.forgotten_pull_here() {
            // Its files are still changing: show the pull running until it is done.
            Some(Busy::Pull)
        } else {
            None
        };
        self.last_fetch = None;
        self.last_status = None;
        self.status_due = self.in_repo.then(Instant::now);
        self.fetch_after_status = false;
        self.fetch_error = None;
        if !matches!(self.dialog, Some(Dialog::Clone(_))) {
            self.dialog = None;
            self.auth = None;
            self.last = None;
        }
        self.confirm_unlink = false;
        self.push_message.clear();
    }

    fn is_current(&self, project: &Path) -> bool {
        self.project.as_deref() == Some(project)
    }

    fn queue_status(&mut self) {
        let Some(project) = self.project.clone() else { return };
        self.last_status = Some(Instant::now());
        self.find_repo();
        if self.in_repo {
            self.send(Job::Status { project });
        } else {
            self.status = None;
            self.status_known = true;
        }
    }

    fn queue_fetch(&mut self) {
        let Some(project) = self.project.clone() else { return };
        self.last_fetch = Some(Instant::now());
        if self.busy.is_none() {
            self.busy = Some(Busy::Fetch);
        }
        let creds = self.creds.get(&self.remote_host()).cloned();
        self.send(Job::Fetch { project, creds });
    }

    /// The host of the linked repository ("" for local paths).
    fn remote_host(&self) -> String {
        self.status.as_ref().and_then(|s| s.remote_url.as_deref()).and_then(git::host_of).unwrap_or_default()
    }

    /// Sends a job the user asked for, with the session's login for its host.
    fn start(&mut self, action: Action) {
        let host = match &action {
            Action::Link { url, .. } | Action::Clone { url, .. } => git::host_of(url).unwrap_or_default(),
            _ => self.remote_host(),
        };
        let creds = self.creds.get(&host).cloned();
        let with_login = creds.is_some();
        let job = match (action.clone(), self.project.clone()) {
            (Action::Clone { url, branch, dest }, _) => Job::Clone { creds, url, branch, dest },
            (_, None) => return,
            (Action::Plan, Some(project)) => Job::PlanPull { project, creds },
            (Action::Pull { answers, identity }, Some(project)) => Job::Pull { project, creds, answers, identity },
            (Action::Push { message, identity }, Some(project)) => Job::Push { project, creds, message, identity },
            (Action::Link { url, branch }, Some(project)) => Job::Link { project, creds, url, branch },
        };
        let kind = action.busy();
        self.busy = Some(kind);
        self.last = Some((action, host, with_login));
        if kind == Busy::Clone {
            // Not about the open project: its answer is wanted after a switch too.
            self.send(job);
        } else {
            self.send_awaited(kind, job);
        }
    }

    /// The reply to a user's job arrived: it is no longer busy.
    fn done(&mut self, kind: Busy) {
        if self.busy == Some(kind) {
            self.busy = None;
        }
    }

    /// Shows a failed job's message in its own dialog; false when that dialog isn't open.
    fn dialog_error(&mut self, job: Busy, message: &str) -> bool {
        let slot = match (&mut self.dialog, job) {
            (Some(Dialog::Push(d)), Busy::Push) => &mut d.error,
            (Some(Dialog::Link(d)), Busy::Link) => &mut d.error,
            (Some(Dialog::Clone(d)), Busy::Clone) => &mut d.error,
            _ => return false,
        };
        *slot = Some(message.to_string());
        true
    }

    /// The project folder relative to the repository root, with a trailing '/' ("" at the root).
    fn project_prefix(&self) -> String {
        let (Some(project), Some(st)) = (&self.project, &self.status) else { return String::new() };
        match project.strip_prefix(&st.repo_root) {
            Ok(rel) if !rel.as_os_str().is_empty() => format!("{}/", rel.to_string_lossy().replace('\\', "/")),
            _ => String::new(),
        }
    }
}

fn unix_now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

// ---------------------------------------------------------------------------
// Every frame: replies and timers.

/// Applies what the worker answered and starts what is due. Runs before anything is drawn.
pub fn poll(app: &mut App, ctx: &egui::Context) {
    if app.git.screenshot {
        return;
    }
    app.git.ctx.get_or_insert_with(|| ctx.clone());
    let root = app.project.as_ref().map(|p| p.root.clone());
    if root != app.git.project {
        app.git.switch(root);
    }
    let replies: Vec<Reply> = app.git.worker.as_ref().map(|w| w.rx.try_iter().collect()).unwrap_or_default();
    for reply in replies {
        apply(app, reply, ctx);
    }
    timers(app, ctx);
}

fn timers(app: &mut App, ctx: &egui::Context) {
    let auto_fetch = app.settings.git_auto_fetch;
    let git = &mut app.git;
    if git.project.is_none() {
        return;
    }
    let now = Instant::now();
    let focused = ctx.input(|i| i.focused);
    let regained = focused && !git.focused;
    git.focused = focused;

    let due = git.status_due.is_some_and(|t| now >= t);
    let stale = focused && git.in_repo && git.last_status.is_none_or(|t| now - t >= STATUS_EVERY);
    if due || stale {
        git.status_due = None;
        git.queue_status();
    }
    let may_fetch = auto_fetch && git.linked() && git.busy.is_none() && git.status_known;
    let periodic = git.last_fetch.is_none_or(|t| now - t >= FETCH_EVERY);
    let back = regained && git.last_fetch.is_none_or(|t| now - t > FETCH_ON_FOCUS);
    if may_fetch && (periodic || back) {
        git.queue_fetch();
    }

    // Wake up for the next timer even when nothing else happens. Only timers that can fire:
    // a fetch that may not run now is queued as soon as a later frame can.
    let mut next: Vec<Instant> = git.status_due.into_iter().collect();
    if focused && git.in_repo {
        next.extend(git.last_status.map(|t| t + STATUS_EVERY));
    }
    if may_fetch {
        next.extend(git.last_fetch.map(|t| t + FETCH_EVERY));
    }
    if git.status.as_ref().is_some_and(|s| s.linked() && s.last_push.is_some()) {
        // The age next to the Push button.
        next.push(now + Duration::from_secs(60));
    }
    if let Some(at) = next.into_iter().min() {
        ctx.request_repaint_after(at.saturating_duration_since(now).max(Duration::from_millis(50)));
    }
}

fn apply(app: &mut App, reply: Reply, ctx: &egui::Context) {
    let user_job = matches!(
        reply,
        Reply::Planned { .. } | Reply::Pulled { .. } | Reply::Pushed { .. } | Reply::Linked { .. } | Reply::Unlinked { .. }
    );
    if user_job && !app.git.answered() {
        // A forgotten pull still changed the files of its repository, which may be open (again).
        if let Reply::Pulled { project, .. } = &reply {
            let git = &mut app.git;
            if git.repo.is_some() && git::find_repo_root(project) == git.repo {
                if git.busy == Some(Busy::Pull) && !git.forgotten_pull_here() {
                    git.busy = None;
                }
                git.status_due = Some(Instant::now());
                reload_after_pull(app);
            }
        }
        return;
    }
    match reply {
        Reply::Status { project, result } if app.git.is_current(&project) => applied_status(&mut app.git, result),
        Reply::Fetched { project, result } if app.git.is_current(&project) => {
            let git = &mut app.git;
            git.done(Busy::Fetch);
            git.fetch_error = match result {
                Ok(()) => None,
                Err(GitError::NotInstalled) => {
                    git.git_missing = true;
                    Some(GitError::NotInstalled.to_string())
                }
                // Background checks stay quiet; the Pull button's tooltip tells.
                Err(GitError::AuthRequired { host, detail, .. }) => {
                    let host = if host.is_empty() { git.remote_host() } else { host };
                    Some(format!("Anmeldung bei {host} erforderlich.\n{detail}").trim_end().to_string())
                }
                Err(e) => Some(e.to_string()),
            };
        }
        Reply::Planned { project, result } if app.git.is_current(&project) => {
            app.git.done(Busy::Plan);
            match result {
                Err(e) => report(app, Busy::Plan, e),
                Ok(plan) => {
                    // The remote answered, so an earlier fetch error is out of date.
                    app.git.fetch_error = None;
                    if plan.behind == 0 {
                        app.toast("Bereits auf dem neuesten Stand.", true);
                    } else if plan.questions.is_empty() {
                        app.save_now();
                        let identity = own_identity(&app.settings);
                        app.git.start(Action::Pull { answers: Vec::new(), identity });
                    } else {
                        app.git.dialog = Some(Dialog::Pull(PullDialog::new(plan)));
                    }
                }
            }
        }
        Reply::Pulled { project, result } if app.git.is_current(&project) => {
            app.git.done(Busy::Pull);
            // Even a failed pull may have changed files (its recovery restores them): show what is on disk.
            reload_after_pull(app);
            match result {
                Ok(outcome) => {
                    app.git.fetch_error = None;
                    app.toast(pull_summary(&outcome), true);
                }
                Err(e) => report(app, Busy::Pull, e),
            }
        }
        Reply::Pushed { project, result } if app.git.is_current(&project) => {
            app.git.done(Busy::Push);
            match result {
                Ok(out) => {
                    app.git.fetch_error = None;
                    if matches!(app.git.dialog, Some(Dialog::Push(_))) {
                        app.git.dialog = None;
                    }
                    app.git.push_message.clear();
                    let text = match out.commits {
                        0 => "Nichts zu pushen.".to_string(),
                        1 => "Gepusht: 1 Commit.".to_string(),
                        n => format!("Gepusht: {n} Commits."),
                    };
                    app.toast(text, true);
                }
                Err(GitError::NeedsPull { behind }) => {
                    let git = &mut app.git;
                    if let Some(st) = git.status.as_mut() {
                        st.behind = st.behind.max(behind);
                    }
                    if !matches!(git.dialog, Some(Dialog::Push(_))) {
                        git.dialog = Some(Dialog::Push(push_dialog_for(&app.settings, git.status.as_ref())));
                    }
                    if let Some(Dialog::Push(d)) = &mut git.dialog {
                        d.needs_pull = true;
                        d.error = None;
                    }
                }
                Err(e) => report(app, Busy::Push, e),
            }
        }
        Reply::Linked { project, result } if app.git.is_current(&project) => {
            app.git.done(Busy::Link);
            match result {
                Ok(()) => {
                    let git = &mut app.git;
                    if matches!(git.dialog, Some(Dialog::Link(_))) {
                        git.dialog = None;
                    }
                    git.find_repo();
                    git.fetch_after_status = true;
                    let text = match &git.last {
                        Some((Action::Link { url, branch }, ..)) => {
                            format!("Verbunden mit {} ({branch}).", git::display_url(url))
                        }
                        _ => "Verbunden.".to_string(),
                    };
                    app.toast(text, true);
                    // Linking to a repository with files brings them into the folder.
                    app.refresh_files();
                    app.schedule_compile(Duration::ZERO);
                }
                Err(e) => report(app, Busy::Link, e),
            }
        }
        Reply::Unlinked { project, result } if app.git.is_current(&project) => {
            app.git.done(Busy::Unlink);
            match result {
                Ok(()) => app.toast("Die Verbindung zum Repository ist getrennt.", true),
                Err(e) => report(app, Busy::Unlink, e),
            }
        }
        Reply::Cloned { dest, result } => {
            app.git.done(Busy::Clone);
            match result {
                Ok(path) if matches!(app.git.dialog, Some(Dialog::Clone(_))) => {
                    app.git.dialog = None;
                    app.open_path(&path, ctx);
                }
                // The dialog was closed meanwhile: don't switch projects under the user's hands.
                Ok(_) => app.toast(format!("Geklont nach {}.", tilde(&dest)), true),
                Err(e) => report(app, Busy::Clone, e),
            }
        }
        // A reply about a project that is no longer open.
        _ => {}
    }
}

fn applied_status(git: &mut GitUi, result: Result<Option<RepoStatus>, GitError>) {
    git.status_known = true;
    match result {
        Ok(status) => {
            git.git_missing = false;
            git.status_error = None;
            git.status = status;
            if git.linked() && std::mem::take(&mut git.fetch_after_status) {
                git.queue_fetch();
            }
        }
        Err(GitError::NotInstalled) => {
            git.git_missing = true;
            git.status = None;
        }
        Err(e) => git.status_error = Some(e.to_string()),
    }
}

/// A job the user started failed: ask for a login when that helps, else show the message in the
/// job's dialog (or as a toast).
fn report(app: &mut App, job: Busy, e: GitError) {
    let git = &mut app.git;
    match e {
        GitError::AuthRequired { ssh: true, .. } => git.auth = Some(Auth::Ssh),
        GitError::AuthRequired { host, ssh: false, .. } if git.last.is_some() => {
            let (_, job_host, with_login) = git.last.clone().unwrap_or((Action::Plan, String::new(), false));
            let host = if host.is_empty() { job_host } else { host };
            // A login that just failed is no use for later jobs either.
            let tried = if with_login { git.creds.remove(&host) } else { git.creds.get(&host).cloned() };
            let username = tried.map(|c| c.username).unwrap_or_default();
            git.auth = Some(Auth::Login { host, username, password: String::new(), failed: with_login, focus: true });
        }
        e => {
            if matches!(e, GitError::NotInstalled) {
                git.git_missing = true;
            }
            let message = e.to_string();
            if !git.dialog_error(job, &message) {
                app.toast(message, false);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Reloading open documents after a pull.

/// What reading an open document's file again found.
#[derive(Debug, PartialEq, Eq)]
enum Fresh {
    Unchanged,
    Changed,
    Gone,
}

/// The result of [`reload_open_docs`].
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Reloaded {
    /// The active tab got new text (its editor state must be restored from `OpenDoc::state`).
    pub active_changed: bool,
    /// Tabs closed because their file is gone.
    pub closed: Vec<String>,
}

/// Re-reads every open document after a pull. New content replaces `text` and `saved` and drops
/// the undo history (undo would bring back the old file); a deleted file closes its tab. The tabs
/// were saved before the pull and the editor was locked, so nothing typed is lost — should a tab
/// still differ from its last save, its text wins and is saved later.
pub fn reload_open_docs(root: &Path, doc: &mut Option<OpenDoc>, tabs: &mut Vec<OpenDoc>, order: &mut Vec<String>) -> Reloaded {
    let mut out = Reloaded::default();
    if let Some(d) = doc.as_mut() {
        match reread(root, d) {
            Fresh::Changed => out.active_changed = true,
            Fresh::Gone => out.closed.push(d.rel.clone()),
            Fresh::Unchanged => {}
        }
    }
    for d in tabs.iter_mut() {
        if reread(root, d) == Fresh::Gone {
            out.closed.push(d.rel.clone());
        }
    }
    for rel in &out.closed {
        close_without_saving(doc, tabs, order, rel);
    }
    out
}

fn reread(root: &Path, d: &mut OpenDoc) -> Fresh {
    if d.text != d.saved {
        return Fresh::Unchanged;
    }
    match std::fs::read(root.join(&d.rel)) {
        Ok(bytes) => match String::from_utf8(bytes) {
            Ok(text) if text == d.saved => Fresh::Unchanged,
            Ok(text) => {
                d.saved = text.clone();
                d.text = text;
                d.edited_at = None;
                if let Some(state) = d.state.as_mut() {
                    state.clear_undoer();
                }
                Fresh::Changed
            }
            // Now a binary file: the editor can't show it.
            Err(_) => Fresh::Gone,
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Fresh::Gone,
        Err(_) => Fresh::Unchanged,
    }
}

/// Closes a tab like `App::close_tab`, but never writes it back (its file was deleted on purpose).
fn close_without_saving(doc: &mut Option<OpenDoc>, tabs: &mut Vec<OpenDoc>, order: &mut Vec<String>, rel: &str) {
    let Some(pos) = order.iter().position(|r| r == rel) else { return };
    order.remove(pos);
    if doc.as_ref().is_some_and(|d| d.rel == rel) {
        *doc = None;
        let next = order.get(pos).or_else(|| order.get(pos.wrapping_sub(1))).cloned();
        if let Some(i) = next.and_then(|next| tabs.iter().position(|d| d.rel == next)) {
            *doc = Some(tabs.remove(i));
        }
    } else {
        tabs.retain(|d| d.rel != rel);
    }
}

fn reload_after_pull(app: &mut App) {
    let Some(root) = app.project.as_ref().map(|p| p.root.clone()) else { return };
    let r = reload_open_docs(&root, &mut app.doc, &mut app.tabs, &mut app.tab_order);
    if r.active_changed || !r.closed.is_empty() {
        // The editor takes the cursor (and the emptied undo history) from the tab again.
        app.editor_shown_rel = None;
    }
    app.refresh_files();
    // A pull can delete or rename the main document: point at something that exists.
    if let Some(project) = app.project.as_mut()
        && !root.join(&project.main_rel).exists() {
            let fallback = app
                .doc
                .iter()
                .chain(app.tabs.iter())
                .map(|d| d.rel.clone())
                .find(|rel| rel.ends_with(".typ") && root.join(rel).exists())
                .or_else(|| project.files.iter().find(|f| f.rel.ends_with(".typ")).map(|f| f.rel.clone()));
            if let Some(fallback) = fallback {
                project.main_rel = fallback;
            }
        }
    app.schedule_compile(Duration::ZERO);
}

/// The toast after a pull: how much came in and which files to look at.
fn pull_summary(out: &PullOutcome) -> String {
    let mut text = match out.commits {
        0 => "Bereits auf dem neuesten Stand.".to_string(),
        1 => "1 Commit geholt.".to_string(),
        n => format!("{n} Commits geholt."),
    };
    if !out.merged_text.is_empty() {
        text += &format!(" Zusammengeführt: {} – bitte prüfen.", file_names(&out.merged_text));
    }
    if !out.took_remote.is_empty() {
        text += &format!(" Aus dem Repository übernommen: {}.", file_names(&out.took_remote));
    }
    if !out.kept_local.is_empty() {
        text += &format!(" Deine Version behalten: {}.", file_names(&out.kept_local));
    }
    text
}

/// "a.typ, b.typ, c.typ, …": file names only, at most three.
fn file_names(paths: &[String]) -> String {
    let mut names: Vec<&str> = paths.iter().take(3).map(|p| p.rsplit('/').next().unwrap_or(p)).collect();
    if paths.len() > 3 {
        names.push("…");
    }
    names.join(", ")
}

/// "vor 36 Minuten" – the age in words, for tooltips and the settings.
fn ago_words(secs: u64) -> String {
    let (n, one, many) = match secs {
        s if s < 60 => return "gerade eben".into(),
        s if s < 3600 => (s / 60, "Minute", "Minuten"),
        s if s < 86_400 => (s / 3600, "Stunde", "Stunden"),
        s if s < 14 * 86_400 => (s / 86_400, "Tag", "Tagen"),
        s => (s / (7 * 86_400), "Woche", "Wochen"),
    };
    format!("vor {n} {}", if n == 1 { one } else { many })
}

/// The number on the Pull badge.
fn badge_text(behind: usize) -> String {
    if behind > 99 { "99+".into() } else { behind.to_string() }
}

// ---------------------------------------------------------------------------
// Starting the flows (top bar, menus, settings, welcome screen).

/// Pull: save, then fetch and work out the questions first.
pub fn start_pull(app: &mut App) {
    if !ready_for_sync(app) {
        return;
    }
    app.save_now();
    app.git.start(Action::Plan);
}

/// Push: save and open the push dialog (with a fresh look at the changes).
pub fn open_push(app: &mut App) {
    if !ready_for_sync(app) {
        return;
    }
    app.save_now();
    app.git.queue_status();
    app.git.dialog = Some(Dialog::Push(push_dialog_for(&app.settings, app.git.status.as_ref())));
}

/// False when Pull/Push can't start now; offers the link dialog when the project isn't linked.
fn ready_for_sync(app: &mut App) -> bool {
    let git = &mut app.git;
    if git.blocking() || app.project.is_none() {
        return false;
    }
    if git.linked() {
        return true;
    }
    if git.in_repo && !git.status_known && !git.git_missing {
        app.toast("Einen Moment – der Git-Status wird noch gelesen.", false);
        return false;
    }
    open_link(app);
    false
}

/// The name and address from the settings; empty fields mean git's own configuration.
fn own_identity(settings: &Settings) -> Identity {
    Identity { name: settings.git_name.trim().to_string(), email: settings.git_email.trim().to_string() }
}

fn push_dialog_for(settings: &Settings, st: Option<&RepoStatus>) -> PushDialog {
    let config = st.map(|s| s.config_identity.clone()).unwrap_or_default();
    let pick = |own: &str, theirs: String| if own.trim().is_empty() { theirs } else { own.to_string() };
    PushDialog {
        name: pick(&settings.git_name, config.name),
        email: pick(&settings.git_email, config.email),
        needs_pull: false,
        error: None,
        focus: true,
    }
}

pub fn open_link(app: &mut App) {
    if app.project.is_none() {
        return;
    }
    let st = app.git.status.as_ref();
    app.git.dialog = Some(Dialog::Link(LinkDialog {
        url: st.and_then(|s| s.known_url.clone()).unwrap_or_default(),
        branch: st.and_then(|s| s.branch.clone()).unwrap_or_else(|| "main".into()),
        error: None,
        focus: true,
    }));
}

pub fn open_clone(app: &mut App) {
    if app.git.busy == Some(Busy::Clone) && matches!(app.git.dialog, Some(Dialog::Clone(_))) {
        return;
    }
    let parent = app
        .settings
        .git_clone_dir
        .clone()
        .filter(|d| d.is_dir())
        .or_else(dirs::document_dir)
        .or_else(dirs::home_dir)
        .unwrap_or_else(|| PathBuf::from("."));
    app.git.dialog = Some(Dialog::Clone(CloneDialog {
        url: String::new(),
        branch: String::new(),
        name: String::new(),
        auto: true,
        parent,
        error: None,
        focus: true,
    }));
}

// ---------------------------------------------------------------------------
// Top bar.

/// The age shown in the Push button's pill.
fn push_pill(ui: &Ui, st: &RepoStatus) -> Option<Arc<Galley>> {
    let at = st.last_push?;
    let text = git::relative_time(unix_now().saturating_sub(at));
    Some(ui.fonts_mut(|f| f.layout_no_wrap(text, FontId::proportional(11.0), pal().dim)))
}

fn push_width(pill: Option<&Arc<Galley>>) -> f32 {
    TOOL + pill.map(|g| g.size().x + 12.0 + 4.0).unwrap_or(0.0)
}

/// Room the Pull/Push group takes in the top bar (none when the project is not linked).
pub fn topbar_width(app: &App, ui: &Ui) -> f32 {
    match app.git.status.as_ref().filter(|s| s.linked()) {
        Some(st) => 9.0 + TOOL + push_width(push_pill(ui, st).as_ref()) + 3.0 * TOOL_GAP,
        None => 0.0,
    }
}

/// Pull and Push for a right-to-left row: a separator, Push, then Pull to its left.
pub fn topbar_buttons(app: &mut App, ui: &mut Ui) {
    let Some(st) = app.git.status.clone().filter(RepoStatus::linked) else { return };
    let busy = app.git.busy;
    let enabled = !app.git.blocking();
    bar_separator(ui);

    let pill = push_pill(ui, &st);
    let mut tip = String::from("Änderungen hochladen (Push)");
    tip += &match st.changes.len() {
        0 => "\nKeine lokalen Änderungen".to_string(),
        1 => "\n1 geänderte Datei".to_string(),
        n => format!("\n{n} geänderte Dateien"),
    };
    if let Some(at) = st.last_push {
        tip += &format!("\nZuletzt gepusht {}", ago_words(unix_now().saturating_sub(at)));
    }
    let (rect, push) = bar_button(ui, push_width(pill.as_ref()), enabled, &tip);
    let icon = Rect::from_min_size(rect.min, vec2(TOOL, TOOL));
    icon_or_spinner(ui, icon, Icon::Push, busy == Some(Busy::Push), enabled);
    if let Some(g) = pill {
        let r = Rect::from_min_size(pos2(icon.right(), rect.center().y - 9.0), vec2(g.size().x + 12.0, 18.0));
        ui.painter().rect_filled(r, 9.0, pal().badge);
        ui.painter().galley(r.center() - g.size() / 2.0, g, pal().dim);
    }
    if push {
        open_push(app);
    }

    let mut tip = String::from("Änderungen holen (Pull)");
    tip += &match (&app.git.fetch_error, st.behind) {
        (Some(e), _) => format!("\nLetzte Prüfung fehlgeschlagen: {e}"),
        (None, 0) => "\nAuf dem neuesten Stand".to_string(),
        (None, 1) => "\n1 neuer Commit im Repository".to_string(),
        (None, n) => format!("\n{n} neue Commits im Repository"),
    };
    let (rect, pull) = bar_button(ui, TOOL, enabled, &tip);
    let pulling = matches!(busy, Some(Busy::Plan | Busy::Pull));
    icon_or_spinner(ui, rect, Icon::Pull, pulling, enabled);
    if st.behind > 0 && !pulling {
        // A blue count at the top-right corner, like a notification badge.
        let text = badge_text(st.behind);
        let g = ui.fonts_mut(|f| f.layout_no_wrap(text, bold(10.5), pal().on_accent));
        let w = (g.size().x + 8.0).max(15.0);
        let badge = Rect::from_center_size(pos2(rect.right() - 6.0 - (w - 15.0) / 2.0, rect.top() + 6.0), vec2(w, 15.0));
        ui.painter().rect_filled(badge, 7.5, pal().accent);
        ui.painter().galley(badge.center() - g.size() / 2.0, g, pal().on_accent);
    }
    if pull {
        start_pull(app);
    }
}

/// A flat top-bar button of any width that can be switched off. Returns its place and whether
/// it was clicked; the tooltip's first line names it.
fn bar_button(ui: &mut Ui, width: f32, enabled: bool, tip: &str) -> (Rect, bool) {
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let (rect, resp) = ui.allocate_exact_size(vec2(width, TOOL), sense);
    let resp = resp.on_hover_text(tip);
    if let Some(fill) = flat_fill(&resp, false).filter(|_| enabled) {
        ui.painter().rect_filled(rect, RADIUS, fill);
    }
    let name = tip.lines().next().unwrap_or(tip);
    (rect, resp.clicked() || (enabled && pressed_by_test(name)))
}

/// A click on an enabled button. In the end-to-end tests also a press of it by its label.
fn clicked(resp: &egui::Response, label: &str) -> bool {
    resp.clicked() || (resp.enabled() && pressed_by_test(label))
}

#[cfg(not(test))]
fn pressed_by_test(_label: &str) -> bool {
    false
}

#[cfg(test)]
fn pressed_by_test(label: &str) -> bool {
    e2e::take_press(label)
}

fn icon_or_spinner(ui: &Ui, rect: Rect, icon: Icon, spinning: bool, enabled: bool) {
    let inner = Rect::from_center_size(rect.center(), vec2(TOOL_ICON, TOOL_ICON));
    if spinning {
        egui::Spinner::new().size(14.0).color(pal().dim).paint_at(ui, inner);
    } else {
        paint(ui.painter(), inner, icon, if enabled { fg(false) } else { pal().faint });
    }
}

// ---------------------------------------------------------------------------
// Dialogs.

/// Draws the open dialog and, on top of it, a login request.
pub fn dialogs(app: &mut App, ctx: &egui::Context) {
    if !app.settings_open {
        app.git.confirm_unlink = false;
    }
    if let Some(mut d) = app.git.dialog.take() {
        let open = match &mut d {
            Dialog::Pull(p) => pull_dialog(app, ctx, p),
            Dialog::Push(p) => push_dialog(app, ctx, p),
            Dialog::Link(l) => link_dialog(app, ctx, l),
            Dialog::Clone(c) => clone_dialog(app, ctx, c),
        };
        if open && app.git.dialog.is_none() {
            app.git.dialog = Some(d);
        }
    }
    if let Some(mut a) = app.git.auth.take()
        && auth_dialog(app, ctx, &mut a) && app.git.auth.is_none() {
            app.git.auth = Some(a);
        }
}

/// A dialog in the app's look. Returns what `add` returned and whether Esc asked to close it.
fn modal<R>(ctx: &egui::Context, id: &str, width: f32, add: impl FnOnce(&mut Ui) -> R) -> (R, bool) {
    let frame = egui::Frame::new()
        .fill(pal().window)
        .stroke(Stroke::new(1.0_f32, pal().separator))
        .corner_radius(RADIUS_XL)
        .inner_margin(egui::Margin::same(20));
    let r = egui::Modal::new(egui::Id::unique(id)).frame(frame).show(ctx, |ui| {
        ui.set_width(width);
        ui.spacing_mut().item_spacing.y = 6.0;
        add(ui)
    });
    // Only Esc (and the buttons) close a dialog: a stray click beside a form must not throw away
    // what was typed.
    let esc = r.should_close() && !r.backdrop_response.clicked();
    (r.inner, esc)
}

fn title(ui: &mut Ui, text: &str) {
    ui.label(RichText::new(text).font(bold(17.0)).color(pal().text));
    ui.add_space(2.0);
}

fn body(ui: &mut Ui, text: &str) {
    ui.label(RichText::new(text).size(13.5).color(pal().text_soft));
}

fn caption(ui: &mut Ui, text: &str) {
    ui.add_space(4.0);
    ui.label(RichText::new(text).size(12.5).color(pal().dim));
}

fn small(ui: &mut Ui, text: &str, color: Color32) {
    ui.label(RichText::new(text).size(12.5).color(color));
}

/// Cancel and the main action, right-aligned; a spinner beside them while the job runs.
/// Returns (cancel, ok).
fn buttons(ui: &mut Ui, ok: &str, enabled: bool, running: bool) -> (bool, bool) {
    ui.add_space(10.0);
    let mut out = (false, false);
    ui.allocate_ui_with_layout(vec2(ui.available_width(), CONTROL_H), Layout::right_to_left(Align::Center), |ui| {
        out.1 = clicked(&ui.add_enabled(enabled && !running, Button::primary(ok)), ok);
        // A running job cannot be cancelled: its result decides what the dialog shows next.
        out.0 = clicked(&ui.add_enabled(!running, Button::new("Abbrechen")), "Abbrechen");
        if running {
            ui.add_space(4.0);
            ui.add(egui::Spinner::new().size(16.0).color(pal().dim));
        }
    });
    out
}

/// Enter in this single-line field.
fn submitted(ui: &Ui, resp: &egui::Response) -> bool {
    resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter))
}

fn focus_once(resp: &egui::Response, focus: &mut bool) {
    if std::mem::take(focus) {
        resp.request_focus();
    }
}

/// A caption above a field of the given width (for two fields side by side).
fn column<R>(ui: &mut Ui, label: &str, width: f32, add: impl FnOnce(&mut Ui) -> R) -> R {
    ui.vertical(|ui| {
        ui.set_width(width);
        caption(ui, label);
        add(ui)
    })
    .inner
}

/// A framed multi-line text field.
fn text_area(ui: &mut Ui, text: &mut String, hint: &str, rows: usize, id_salt: &str) {
    let id = ui.make_persistent_id(id_salt);
    let focused = ui.memory(|m| m.has_focus(id));
    let stroke = if focused { Stroke::new(1.5_f32, pal().accent) } else { Stroke::NONE };
    egui::Frame::new().fill(pal().surface).stroke(stroke).corner_radius(RADIUS).inner_margin(egui::Margin::symmetric(10, 6)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        egui::ScrollArea::vertical().id_salt(id_salt).max_height(rows as f32 * 18.0).show(ui, |ui| {
            ui.add(
                egui::TextEdit::multiline(text)
                    .id(id)
                    .hint_text(hint)
                    .frame(egui::Frame::NONE)
                    .margin(egui::Margin::ZERO)
                    .desired_width(f32::INFINITY)
                    .desired_rows(rows),
            );
        });
    });
}

fn pull_dialog(app: &mut App, ctx: &egui::Context, d: &mut PullDialog) -> bool {
    let prefix = app.git.project_prefix();
    let ((cancel, ok), esc) = modal(ctx, "git-pull", 500.0, |ui| {
        title(ui, "Änderungen holen");
        body(ui, "Einige Dateien wurden hier und im Repository geändert. Was soll übernommen werden?");
        ui.add_space(6.0);
        egui::ScrollArea::vertical().id_salt("git-questions").max_height(330.0).auto_shrink([false, true]).show(ui, |ui| {
            for (i, (q, choice)) in d.plan.questions.iter().zip(d.choices.iter_mut()).enumerate() {
                if i > 0 {
                    sep(ui);
                }
                question_row(ui, i, q, &prefix, choice);
            }
        });
        buttons(ui, "Übernehmen", true, false)
    });
    if ok {
        let answers = d.plan.questions.iter().zip(&d.choices).map(|(q, c)| (q.path().to_string(), *c)).collect();
        app.save_now();
        let identity = own_identity(&app.settings);
        app.git.start(Action::Pull { answers, identity });
        return false;
    }
    !(cancel || esc)
}

/// One file of the pull dialog: name, what happened, and the two-way choice.
fn question_row(ui: &mut Ui, i: usize, q: &Question, prefix: &str, choice: &mut Choice) {
    let (what, labels) = match q {
        Question::BinaryChanged(_) => ("Diese Datei wurde hier und im Repository geändert.", ["Meine Version", "Aus dem Repository"]),
        Question::DeletedInRepo(_) => ("Im Repository gelöscht, hier aber bearbeitet.", ["Behalten", "Löschen"]),
        Question::AddedBoth(_) => ("Hier und im Repository neu angelegt.", ["Meine Version", "Aus dem Repository"]),
    };
    let path = q.path().strip_prefix(prefix).unwrap_or(q.path());
    let (row, _) = ui.allocate_exact_size(vec2(ui.available_width(), 52.0), Sense::hover());
    let seg = Rect::from_min_size(pos2(row.right() - 250.0, row.center().y - CONTROL_H / 2.0), vec2(250.0, CONTROL_H));
    let text_w = seg.left() - row.left() - 14.0;
    let name = one_line(ui, path, bold(14.0), pal().text, text_w);
    let mut job = egui::text::LayoutJob::simple(what.to_string(), FontId::proportional(12.5), pal().dim, text_w);
    job.wrap.max_rows = 2;
    job.wrap.overflow_character = Some('…');
    let detail = ui.fonts_mut(|f| f.layout_job(job));
    let top = row.center().y - (name.size().y + 2.0 + detail.size().y) / 2.0;
    let name_rect = Rect::from_min_size(pos2(row.left(), top), name.size());
    ui.interact(name_rect, ui.scope_id().with(("git-q-name", i)), Sense::hover()).on_hover_text(q.path());
    ui.painter().galley(name_rect.min, name, pal().text);
    ui.painter().galley(pos2(row.left(), name_rect.bottom() + 2.0), detail, pal().dim);
    let selected = (*choice == Choice::Remote) as usize;
    for (k, (r, resp)) in segmented(ui, seg, selected, ("git-q", i), &["", ""]).into_iter().enumerate() {
        ui.painter().text(r.center(), Align2::CENTER_CENTER, labels[k], FontId::proportional(12.5), fg(k == selected));
        if resp.clicked() {
            *choice = if k == 0 { Choice::Local } else { Choice::Remote };
        }
    }
}

/// A single line of text, shortened with "…" to `width`.
fn one_line(ui: &Ui, text: &str, font: FontId, color: Color32, width: f32) -> Arc<Galley> {
    let mut job = egui::text::LayoutJob::simple_singleline(text.to_string(), font, color);
    job.wrap.max_width = width;
    job.wrap.max_rows = 1;
    job.wrap.break_anywhere = true;
    job.wrap.overflow_character = Some('…');
    ui.fonts_mut(|f| f.layout_job(job))
}

/// Letter, colour and meaning of a change, as in the changed-files list.
fn change_mark(change: Change) -> (&'static str, Color32, &'static str) {
    let green = if pal().dark { Color32::from_rgb(0x4c, 0xc0, 0x5a) } else { Color32::from_rgb(0x2f, 0x8a, 0x3a) };
    match change {
        Change::Modified => ("M", pal().warning, "Geändert"),
        Change::Added => ("A", green, "Neu"),
        Change::Deleted => ("D", pal().error, "Gelöscht"),
        Change::Renamed => ("R", pal().accent, "Umbenannt"),
        Change::Conflicted => ("U", pal().error, "Nicht zusammengeführt"),
    }
}

fn changes_list(ui: &mut Ui, changes: &[ChangedFile]) {
    egui::Frame::new().fill(pal().group).corner_radius(RADIUS).inner_margin(egui::Margin::symmetric(4, 4)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        egui::ScrollArea::vertical().id_salt("git-changes").max_height(6.0 * 24.0).auto_shrink([false, true]).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            for c in changes {
                let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 24.0), Sense::hover());
                let (letter, color, meaning) = change_mark(c.change);
                ui.painter().text(pos2(r.left() + 12.0, r.center().y), Align2::CENTER_CENTER, letter, bold(12.5), color);
                let g = one_line(ui, &c.path, FontId::proportional(13.0), pal().text, r.width() - 36.0);
                ui.painter().galley(pos2(r.left() + 26.0, r.center().y - g.size().y / 2.0), g, pal().text);
                resp.on_hover_text(format!("{meaning}: {}", c.path));
            }
        });
    });
}

/// Addresses offered next to the e-mail field: the ones used before, then git's own.
fn known_emails(settings: &Settings, st: &RepoStatus) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for e in settings.git_emails.iter().chain([&st.config_identity.email]) {
        let e = e.trim();
        if !e.is_empty() && !out.iter().any(|o| o == e) {
            out.push(e.to_string());
        }
    }
    out
}

fn push_dialog(app: &mut App, ctx: &egui::Context, d: &mut PushDialog) -> bool {
    let st = app.git.status.clone().unwrap_or_default();
    let running = app.git.busy == Some(Busy::Push);
    let emails = known_emails(&app.settings, &st);
    let mut message = std::mem::take(&mut app.git.push_message);
    let mut pull_now = false;
    let something = !st.changes.is_empty() || st.ahead > 0;
    let ((cancel, ok), esc) = modal(ctx, "git-push", 480.0, |ui| {
        title(ui, "Änderungen hochladen");
        if !st.changes.is_empty() {
            changes_list(ui, &st.changes);
        } else if st.ahead == 1 {
            body(ui, "1 lokaler Commit wird hochgeladen.");
        } else if st.ahead > 1 {
            body(ui, &format!("{} lokale Commits werden hochgeladen.", st.ahead));
        } else {
            body(ui, "Keine Änderungen zum Hochladen.");
        }
        caption(ui, "Commit-Nachricht");
        text_area(ui, &mut message, "Was hat sich geändert?", 3, "git-message");
        let w = ui.available_width();
        let half = (w - 12.0) / 2.0;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 12.0;
            column(ui, "Name", half, |ui| {
                let r = field(ui, &mut d.name, "Vorname Nachname", half, None, "git-push-name");
                focus_once(&r, &mut d.focus);
            });
            column(ui, "E-Mail-Adresse", half, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    let field_w = if emails.is_empty() { half } else { half - 48.0 };
                    field(ui, &mut d.email, "name@example.com", field_w, None, "git-push-email");
                    if !emails.is_empty() {
                        let items = emails.iter().map(|e| PopupItem::new(e.as_str()).checked(*e == d.email.trim())).collect();
                        if let Some(i) = popup_button(ui, "git-push-emails", "", items) {
                            d.email = emails.get(i).cloned().unwrap_or_default();
                        }
                    }
                });
            });
        });
        if d.needs_pull {
            ui.add_space(4.0);
            small(ui, &GitError::NeedsPull { behind: 0 }.to_string(), pal().error);
            if clicked(&ui.add(Button::new("Jetzt holen (Pull)")), "Jetzt holen (Pull)") {
                pull_now = true;
            }
        }
        if let Some(e) = &d.error {
            ui.add_space(4.0);
            small(ui, e, pal().error);
        }
        let complete = !d.name.trim().is_empty() && !d.email.trim().is_empty();
        buttons(ui, "Pushen", something && complete && !d.needs_pull, running)
    });
    app.git.push_message = message;
    if pull_now {
        start_pull(app);
        return false;
    }
    if ok {
        remember_identity(&mut app.settings, &d.name, &d.email);
        if !app.git.screenshot {
            app.settings.save();
        }
        let identity = Identity { name: d.name.trim().to_string(), email: d.email.trim().to_string() };
        let message = app.git.push_message.trim().to_string();
        let message = if message.is_empty() { "Änderungen aus Typisch".to_string() } else { message };
        d.error = None;
        app.save_now();
        app.git.start(Action::Push { message, identity });
        return true;
    }
    // While the job runs, neither Abbrechen nor Esc may close the dialog: its result decides.
    !(cancel || (esc && !running))
}

/// The push's name and address become the defaults; the address goes to the front of the list.
fn remember_identity(settings: &mut Settings, name: &str, email: &str) {
    let email = email.trim().to_string();
    settings.git_name = name.trim().to_string();
    settings.git_emails.retain(|e| *e != email);
    settings.git_emails.insert(0, email.clone());
    settings.git_emails.truncate(5);
    settings.git_email = email;
}

fn link_dialog(app: &mut App, ctx: &egui::Context, d: &mut LinkDialog) -> bool {
    let running = app.git.busy == Some(Busy::Link);
    let ((cancel, ok), esc) = modal(ctx, "git-link", 460.0, |ui| {
        title(ui, "Mit Git-Repository verbinden");
        body(ui, LINK_TEXT);
        caption(ui, "Repository-Adresse");
        let w = ui.available_width();
        let url = field(ui, &mut d.url, "https://github.com/name/projekt.git", w, None, "git-link-url");
        focus_once(&url, &mut d.focus);
        caption(ui, "Branch");
        let branch = field(ui, &mut d.branch, "main", 200.0, None, "git-link-branch");
        ui.add_space(2.0);
        small(
            ui,
            "Liegen im Branch schon Dateien mit anderem Inhalt, wähle einen neuen Branch – oder klone das Repository als neues Projekt.",
            pal().dim,
        );
        if let Some(e) = &d.error {
            small(ui, e, pal().error);
        }
        let can = !d.url.trim().is_empty() && !d.branch.trim().is_empty();
        let enter = (submitted(ui, &url) || submitted(ui, &branch)) && can && !running;
        let (cancel, ok) = buttons(ui, "Verbinden", can, running);
        (cancel, ok || enter)
    });
    if ok {
        d.error = None;
        app.git.start(Action::Link { url: d.url.trim().to_string(), branch: d.branch.trim().to_string() });
        return true;
    }
    // While the job runs, neither Abbrechen nor Esc may close the dialog: its result decides.
    !(cancel || (esc && !running))
}

fn clone_dialog(app: &mut App, ctx: &egui::Context, d: &mut CloneDialog) -> bool {
    let running = app.git.busy == Some(Busy::Clone);
    // Opened from the menu while a pull or push of the open project still runs: wait for it.
    let other = app.git.blocking() && !running;
    if d.auto {
        d.name = if d.url.trim().is_empty() { String::new() } else { git::repo_name(&d.url) };
    }
    let mut pick = false;
    let ((cancel, ok), esc) = modal(ctx, "git-clone", 440.0, |ui| {
        title(ui, "Repository klonen");
        caption(ui, "Repository-Adresse");
        let w = ui.available_width();
        let url = field(ui, &mut d.url, "https://github.com/name/projekt.git", w, None, "git-clone-url");
        focus_once(&url, &mut d.focus);
        let half = (w - 12.0) / 2.0;
        let mut enter = submitted(ui, &url);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 12.0;
            column(ui, "Branch (optional)", half, |ui| {
                let r = field(ui, &mut d.branch, "Standard-Branch", half, None, "git-clone-branch");
                enter |= submitted(ui, &r);
            });
            column(ui, "Ordnername", half, |ui| {
                let r = field(ui, &mut d.name, "Projekt", half, None, "git-clone-name");
                if r.changed() {
                    d.auto = d.name.trim().is_empty();
                }
                enter |= submitted(ui, &r);
            });
        });
        caption(ui, "Speicherort");
        ui.allocate_ui_with_layout(vec2(w, CONTROL_H), Layout::right_to_left(Align::Center), |ui| {
            pick = ui.add_enabled(!running, Button::new("Ändern …")).clicked();
            let shown = tilde(&d.parent.join(d.name.trim()));
            let g = one_line(ui, &shown, FontId::proportional(13.5), pal().text_soft, ui.available_width() - 8.0);
            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                ui.label(g).on_hover_text(shown.as_str());
            });
        });
        if let Some(e) = &d.error {
            ui.add_space(2.0);
            small(ui, e, pal().error);
        }
        let name = d.name.trim();
        let can = !other && !d.url.trim().is_empty() && !name.is_empty() && !name.contains(['/', '\\']) && name != "." && name != "..";
        let (cancel, ok) = buttons(ui, "Klonen", can, running);
        (cancel, ok || (enter && can && !running))
    });
    if pick
        && let Some(dir) = rfd::FileDialog::new().set_title("Speicherort für das Repository wählen").set_directory(&d.parent).pick_folder() {
            d.parent = dir;
        }
    if ok {
        d.error = None;
        app.settings.git_clone_dir = Some(d.parent.clone());
        if !app.git.screenshot {
            app.settings.save();
        }
        let branch = Some(d.branch.trim().to_string()).filter(|b| !b.is_empty());
        let dest = d.parent.join(d.name.trim());
        app.git.start(Action::Clone { url: d.url.trim().to_string(), branch, dest });
        return true;
    }
    // While the job runs, neither Abbrechen nor Esc may close the dialog: its result decides.
    !(cancel || (esc && !running))
}

fn auth_dialog(app: &mut App, ctx: &egui::Context, a: &mut Auth) -> bool {
    match a {
        Auth::Ssh => {
            let (ok, esc) = modal(ctx, "git-ssh", 400.0, |ui| {
                title(ui, "SSH-Schlüssel nicht akzeptiert");
                body(
                    ui,
                    "Der Server hat deinen SSH-Schlüssel abgelehnt oder es ist keiner geladen. Füge ihn mit „ssh-add“ hinzu oder verwende eine HTTPS-Adresse.",
                );
                ui.add_space(10.0);
                ui.allocate_ui_with_layout(vec2(ui.available_width(), CONTROL_H), Layout::right_to_left(Align::Center), |ui| {
                    clicked(&ui.add(Button::primary("OK")), "OK")
                })
                .inner
            });
            if ok || esc {
                if let Some(job) = app.git.last.as_ref().map(|(action, ..)| action.busy()) {
                    let _ = app.git.dialog_error(job, "Der Server hat den SSH-Schlüssel abgelehnt.");
                }
                return false;
            }
            true
        }
        Auth::Login { host, username, password, failed, focus } => {
            let ((cancel, ok), esc) = modal(ctx, "git-login", 420.0, |ui| {
                title(ui, &format!("Anmeldung bei {host}"));
                body(
                    ui,
                    "Typisch braucht Zugangsdaten für dieses Repository. Git speichert sie nach erfolgreicher Anmeldung im Schlüsselbund (falls eingerichtet).",
                );
                let w = ui.available_width();
                caption(ui, "Benutzername");
                let user = field(ui, username, "", w, None, "git-login-user");
                focus_once(&user, focus);
                caption(ui, "Token oder Passwort");
                let pass = secret_field(ui, password, "", w, "git-login-password");
                if let Some(url) = git::token_url(host) {
                    ui.add_space(2.0);
                    ui.hyperlink_to(RichText::new("Zugangstoken erstellen …").size(12.5), url);
                }
                small(ui, "Bei GitHub und GitLab ein Zugangstoken statt des Passworts verwenden.", pal().dim);
                if *failed {
                    small(ui, "Anmeldung fehlgeschlagen.", pal().error);
                }
                let can = !username.trim().is_empty() && !password.is_empty();
                let enter = (submitted(ui, &user) || submitted(ui, &pass)) && can;
                let (cancel, ok) = buttons(ui, "Anmelden", can, false);
                (cancel, ok || enter)
            });
            let git = &mut app.git;
            if ok {
                let creds = Credentials { username: username.trim().to_string(), password: std::mem::take(password) };
                git.creds.insert(host.clone(), creds);
                if let Some((action, ..)) = git.last.clone() {
                    git.start(action);
                }
                return false;
            }
            if cancel || esc {
                let message = GitError::AuthRequired { host: host.clone(), ssh: false, detail: String::new() }.to_string();
                if let Some((action, ..)) = git.last.take() {
                    let _ = git.dialog_error(action.busy(), &message);
                }
                return false;
            }
            true
        }
    }
}

// ---------------------------------------------------------------------------
// Settings page.

/// The "Git" page of the settings window.
pub fn settings_tab(app: &mut App, ui: &mut Ui) {
    section(ui, "Dieses Projekt");
    group(ui, |ui| project_group(app, ui));

    let id = app.git.status.as_ref().map(|s| s.config_identity.clone()).unwrap_or_default();
    let hint = |own: String, fallback: &str| if own.is_empty() { fallback.to_string() } else { own };
    let (name_hint, email_hint) = (hint(id.name, "Vorname Nachname"), hint(id.email, "name@example.com"));
    section(ui, "Commits");
    group(ui, |ui| {
        setting_row(ui, "Name", |ui| {
            field(ui, &mut app.settings.git_name, &name_hint, 260.0, None, "git-settings-name");
        });
        sep(ui);
        setting_row(ui, "E-Mail-Adresse", |ui| {
            field(ui, &mut app.settings.git_email, &email_hint, 260.0, None, "git-settings-email");
        });
        sep(ui);
        note(ui, "Wird für deine Commits verwendet. Leer lassen, um die Angaben aus der Git-Konfiguration zu nutzen.");
    });

    section(ui, "Abgleich");
    group(ui, |ui| {
        setting_row(ui, "Automatisch nach Änderungen suchen", |ui| {
            toggle(ui, &mut app.settings.git_auto_fetch);
        });
        sep(ui);
        note(ui, "Prüft alle 5 Minuten, ob es im Repository neue Änderungen gibt, und zeigt ihre Anzahl am Pull-Knopf.");
    });

    section(ui, "Anmeldung");
    group(ui, |ui| {
        note(
            ui,
            "Typisch verwendet die Anmeldedaten deines Git (Schlüsselbund oder SSH-Schlüssel). Für HTTPS-Adressen fragt Typisch bei Bedarf nach einem Zugangstoken.",
        );
        let mut hosts: Vec<String> = app.git.creds.keys().cloned().collect();
        hosts.sort();
        let mut forget = None;
        for host in &hosts {
            sep(ui);
            setting_row(ui, host, |ui| {
                if ui.add(ToolButton::new(Icon::Trash, "Zugangsdaten vergessen")).clicked() {
                    forget = Some(host.clone());
                }
            });
        }
        if let Some(host) = forget {
            app.git.creds.remove(&host);
        }
    });
    ui.add_space(12.0);
    unlink_confirm(app, ui.ctx());
}

fn warning(ui: &mut Ui, text: &str) {
    ui.add_space(2.0);
    ui.label(RichText::new(text).size(12.5).color(pal().warning));
    ui.add_space(2.0);
}

fn project_group(app: &mut App, ui: &mut Ui) {
    if app.project.is_none() {
        note(ui, "Öffne ein Projekt, um es mit einem Git-Repository zu verbinden.");
        return;
    }
    if app.git.git_missing {
        note(
            ui,
            if cfg!(target_os = "macos") {
                "Git ist nicht installiert. Installiere die Xcode Command Line Tools mit „xcode-select --install“ im Terminal."
            } else {
                "Installiere Git, um diese Funktion zu nutzen."
            },
        );
        return;
    }
    let st = app.git.status.clone();
    let busy = app.git.blocking();
    let Some(st) = st.filter(RepoStatus::linked) else {
        ui.add_space(4.0);
        note(ui, LINK_TEXT);
        if let Some(st) = &app.git.status {
            if st.branch.is_none() {
                warning(ui, &GitError::Detached.to_string());
            }
            if let Some(op) = &st.in_progress {
                warning(ui, &GitError::InProgress(op.clone()).to_string());
            }
        }
        if let Some(e) = &app.git.status_error {
            warning(ui, e);
        }
        if clicked(&ui.add_enabled(!busy, Button::new("Mit Repository verbinden …")), "Mit Repository verbinden …") {
            app.settings_open = false;
            open_link(app);
        }
        ui.add_space(6.0);
        return;
    };
    let now = unix_now();
    let url = st.remote_url.clone().unwrap_or_default();
    setting_row(ui, "Repository", |ui| {
        let shown = RichText::new(git::display_url(&url)).size(14.0);
        match git::web_url(&url) {
            Some(web) => {
                ui.hyperlink_to(shown, web).on_hover_text(url.as_str());
            }
            None => {
                ui.label(shown.color(pal().text_soft)).on_hover_text(url.as_str());
            }
        }
    });
    sep(ui);
    setting_row(ui, "Branch", |ui| {
        let branch = st.remote_branch.clone().unwrap_or_default();
        let text = if st.unborn { format!("{branch} (noch ohne Commits)") } else { branch };
        ui.label(RichText::new(text).size(14.0).color(pal().text_soft));
    });
    sep(ui);
    setting_row(ui, "Letzter Push", |ui| {
        let text = st.last_push.map(|t| ago_words(now.saturating_sub(t))).unwrap_or_else(|| "noch nie".into());
        ui.label(RichText::new(text).size(14.0).color(pal().text_soft));
    });
    sep(ui);
    setting_row(ui, "Lokale Änderungen", |ui| {
        ui.label(RichText::new(st.changes.len().to_string()).size(14.0).color(pal().text_soft));
    });
    if !st.remote_branch_exists {
        sep(ui);
        note(ui, "Den Branch gibt es im Repository noch nicht – der erste Push legt ihn an.");
    }
    if let Some(op) = &st.in_progress {
        warning(ui, &GitError::InProgress(op.clone()).to_string());
    }
    if let Some(e) = &app.git.status_error {
        warning(ui, e);
    }
    sep(ui);
    ui.add_space(6.0);
    if clicked(&ui.add_enabled(!busy, Button::new("Verbindung trennen …")), "Verbindung trennen …") {
        app.git.confirm_unlink = true;
    }
    ui.add_space(6.0);
}

/// "Disconnect?" – shown in the settings window, which is where it was asked.
fn unlink_confirm(app: &mut App, ctx: &egui::Context) {
    if !app.git.confirm_unlink {
        return;
    }
    let ((cancel, ok), esc) = modal(ctx, "git-unlink", 380.0, |ui| {
        title(ui, "Verbindung trennen?");
        body(ui, "Pull und Push sind danach nicht mehr verfügbar. Der Ordner und seine Versionsgeschichte bleiben erhalten.");
        buttons(ui, "Trennen", true, false)
    });
    if ok
        && let Some(project) = app.git.project.clone() {
            app.git.busy = Some(Busy::Unlink);
            app.git.send_awaited(Busy::Unlink, Job::Unlink { project });
        }
    if ok || cancel || esc {
        app.git.confirm_unlink = false;
    }
}

// ---------------------------------------------------------------------------
// Check pictures.

/// The check-picture views this module draws.
const DEMO_VIEWS: [&str; 11] = [
    "git",
    "git-pulling",
    "git-pull",
    "git-push",
    "git-push-rejected",
    "git-link",
    "git-login",
    "git-ssh",
    "git-clone",
    "settings-git",
    "settings-git-unlink",
];

/// Made-up state for `TYPISCH_SHOT_VIEW=git…` (git never runs in screenshot mode). Returns false,
/// changing nothing, for other views.
pub fn demo(app: &mut App, view: &str) -> bool {
    if !DEMO_VIEWS.contains(&view) {
        return false;
    }
    let Some(root) = app.project.as_ref().map(|p| p.root.clone()) else { return false };
    let me = Identity { name: "Erika Mustermann".into(), email: "erika@example.com".into() };
    let url = "https://github.com/erika/rechnung.git".to_string();
    let linked = RepoStatus {
        repo_root: root.clone(),
        branch: Some("main".into()),
        remote: Some("origin".into()),
        remote_url: Some(url.clone()),
        remote_branch: Some("main".into()),
        remote_branch_exists: true,
        behind: 1,
        changes: vec![
            ChangedFile { path: "main.typ".into(), change: Change::Modified },
            ChangedFile { path: "beispiel-unterschrift.svg".into(), change: Change::Added },
        ],
        last_push: Some(unix_now().saturating_sub(36 * 60)),
        config_identity: me.clone(),
        known_url: Some(url),
        ..Default::default()
    };
    let git = &mut app.git;
    git.project = Some(root);
    git.in_repo = true;
    git.status_known = true;
    git.status = Some(linked);
    match view {
        "git" => {}
        "git-pulling" => git.busy = Some(Busy::Pull),
        "git-pull" => {
            git.dialog = Some(Dialog::Pull(PullDialog::new(PullPlan {
                behind: 2,
                upstream: String::new(),
                questions: vec![
                    Question::BinaryChanged("logo.png".into()),
                    Question::DeletedInRepo("kapitel/einleitung.typ".into()),
                    Question::AddedBoth("notizen.typ".into()),
                ],
                merged_text: vec!["main.typ".into()],
            })));
        }
        "git-push" | "git-push-rejected" => {
            app.settings.git_emails = vec!["erika@example.com".into(), "e.mustermann@firma.de".into()];
            app.git.push_message = "Rechnungsnummer und Adresse aktualisiert".into();
            let mut d = push_dialog_for(&app.settings, app.git.status.as_ref());
            d.needs_pull = view == "git-push-rejected";
            app.git.dialog = Some(Dialog::Push(d));
        }
        "git-link" => {
            git.status = Some(RepoStatus { repo_root: PathBuf::new(), branch: Some("main".into()), unborn: true, ..Default::default() });
            open_link(app);
        }
        "git-login" => {
            git.auth = Some(Auth::Login {
                host: "github.com".into(),
                username: "erika".into(),
                password: "geheim".into(),
                failed: true,
                focus: false,
            });
        }
        "git-ssh" => git.auth = Some(Auth::Ssh),
        "git-clone" => {
            app.screen = Screen::Welcome;
            open_clone(app);
            if let Some(Dialog::Clone(d)) = &mut app.git.dialog {
                d.url = "https://github.com/erika/bachelorarbeit.git".into();
                d.focus = false;
            }
        }
        "settings-git" | "settings-git-unlink" => {
            git.creds.insert("github.com".into(), Credentials { username: "erika".into(), password: "geheim".into() });
            git.confirm_unlink = view == "settings-git-unlink";
            app.settings_open = true;
            app.settings_tab = SettingsTab::Git;
        }
        _ => {}
    }
    true
}

/// The whole window driven headless against real repositories.
#[cfg(test)]
#[path = "gitui_e2e.rs"]
mod e2e;

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// A fresh, empty folder for one test.
    fn folder() -> PathBuf {
        static N: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!("typisch-gitui-{}-{}", std::process::id(), N.fetch_add(1, Ordering::SeqCst)));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn open(rel: &str, text: &str) -> OpenDoc {
        let mut state = egui::text_edit::TextEditState::default();
        state.cursor.set_char_range(Some(egui::text::CCursorRange::one(egui::text::CCursor::new(3))));
        OpenDoc { rel: rel.into(), text: text.into(), saved: text.into(), edited_at: None, state: Some(state) }
    }

    fn tabs_of(rels: &[&str]) -> Vec<String> {
        rels.iter().map(|r| r.to_string()).collect()
    }

    #[test]
    fn reload_replaces_changed_files_and_keeps_the_rest() {
        let dir = folder();
        std::fs::write(dir.join("main.typ"), "neu von oben").unwrap();
        std::fs::write(dir.join("kapitel.typ"), "unverändert").unwrap();
        std::fs::write(dir.join("anhang.typ"), "auch neu").unwrap();
        let mut doc = Some(open("main.typ", "alt"));
        let mut tabs = vec![open("kapitel.typ", "unverändert"), open("anhang.typ", "alt")];
        let mut order = tabs_of(&["main.typ", "kapitel.typ", "anhang.typ"]);
        let r = reload_open_docs(&dir, &mut doc, &mut tabs, &mut order);
        assert_eq!(r, Reloaded { active_changed: true, closed: Vec::new() });
        let d = doc.as_ref().unwrap();
        assert_eq!((d.text.as_str(), d.saved.as_str()), ("neu von oben", "neu von oben"));
        // The cursor stays where it was.
        assert_eq!(d.state.as_ref().and_then(|s| s.cursor.char_range()).map(|c| usize::from(c.primary.index)), Some(3));
        assert_eq!((tabs[0].text.as_str(), tabs[0].saved.as_str()), ("unverändert", "unverändert"));
        assert_eq!((tabs[1].text.as_str(), tabs[1].saved.as_str()), ("auch neu", "auch neu"));
        assert_eq!(order, tabs_of(&["main.typ", "kapitel.typ", "anhang.typ"]));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn reload_leaves_unchanged_files_alone() {
        let dir = folder();
        std::fs::write(dir.join("main.typ"), "gleich").unwrap();
        let mut doc = Some(open("main.typ", "gleich"));
        let mut tabs = Vec::new();
        let mut order = tabs_of(&["main.typ"]);
        let r = reload_open_docs(&dir, &mut doc, &mut tabs, &mut order);
        assert_eq!(r, Reloaded::default());
        assert_eq!(doc.as_ref().map(|d| d.text.as_str()), Some("gleich"));
        assert!(doc.as_ref().unwrap().state.is_some());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn reload_closes_tabs_of_deleted_files() {
        let dir = folder();
        std::fs::write(dir.join("b.typ"), "b").unwrap();
        // The active tab's file is gone: its right neighbour takes over. A gone background tab just closes.
        let mut doc = Some(open("a.typ", "a"));
        let mut tabs = vec![open("b.typ", "b"), open("c.typ", "c")];
        let mut order = tabs_of(&["a.typ", "b.typ", "c.typ"]);
        let r = reload_open_docs(&dir, &mut doc, &mut tabs, &mut order);
        assert_eq!(r.closed, tabs_of(&["a.typ", "c.typ"]));
        assert_eq!(doc.as_ref().map(|d| d.rel.as_str()), Some("b.typ"));
        assert!(tabs.is_empty());
        assert_eq!(order, tabs_of(&["b.typ"]));
        // Closing never writes a deleted file back.
        assert!(!dir.join("a.typ").exists() && !dir.join("c.typ").exists());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn reload_keeps_text_typed_after_the_last_save() {
        let dir = folder();
        std::fs::write(dir.join("main.typ"), "von oben").unwrap();
        let mut doc = Some(OpenDoc { text: "getippt".into(), ..open("main.typ", "alt") });
        let r = reload_open_docs(&dir, &mut doc, &mut Vec::new(), &mut tabs_of(&["main.typ"]));
        assert_eq!(r, Reloaded::default());
        assert_eq!(doc.as_ref().map(|d| d.text.as_str()), Some("getippt"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn ages_in_words() {
        assert_eq!(ago_words(10), "gerade eben");
        assert_eq!(ago_words(60), "vor 1 Minute");
        assert_eq!(ago_words(36 * 60), "vor 36 Minuten");
        assert_eq!(ago_words(3600), "vor 1 Stunde");
        assert_eq!(ago_words(5 * 3600), "vor 5 Stunden");
        assert_eq!(ago_words(86_400), "vor 1 Tag");
        assert_eq!(ago_words(13 * 86_400), "vor 13 Tagen");
        assert_eq!(ago_words(14 * 86_400), "vor 2 Wochen");
    }

    #[test]
    fn pull_toast() {
        let out = PullOutcome {
            commits: 3,
            merged_text: tabs_of(&["kapitel/a.typ", "b.typ", "c.typ", "d.typ"]),
            kept_local: Vec::new(),
            took_remote: tabs_of(&["bilder/logo.png"]),
        };
        assert_eq!(
            pull_summary(&out),
            "3 Commits geholt. Zusammengeführt: a.typ, b.typ, c.typ, … – bitte prüfen. Aus dem Repository übernommen: logo.png."
        );
        let one = PullOutcome { commits: 1, ..Default::default() };
        assert_eq!(pull_summary(&one), "1 Commit geholt.");
    }

    #[test]
    fn badge_is_capped() {
        assert_eq!(badge_text(7), "7");
        assert_eq!(badge_text(99), "99");
        assert_eq!(badge_text(100), "99+");
    }

    #[test]
    fn pushed_address_moves_to_the_front() {
        let mut s = Settings { git_emails: tabs_of(&["a@x.de", "b@x.de", "c@x.de", "d@x.de", "e@x.de"]), ..Settings::default() };
        remember_identity(&mut s, " Erika ", "c@x.de ");
        assert_eq!((s.git_name.as_str(), s.git_email.as_str()), ("Erika", "c@x.de"));
        assert_eq!(s.git_emails, tabs_of(&["c@x.de", "a@x.de", "b@x.de", "d@x.de", "e@x.de"]));
        remember_identity(&mut s, "Erika", "f@x.de");
        assert_eq!(s.git_emails, tabs_of(&["f@x.de", "c@x.de", "a@x.de", "b@x.de", "d@x.de"]));
    }
}
