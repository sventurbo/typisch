//! Git Sync: pull and push a project with any git server through the system `git` command.
//!
//! The UI (gitui.rs) sends [`Job`]s to one background thread ([`GitWorker`]) and applies its
//! [`Reply`]s. Every operation is also a plain function over a [`Git`] context, so the tests can
//! call it directly. The rule above all: local work is never lost. A pull keeps a snapshot of the
//! work tree until everything is back in place, a failed push takes its commit back, and nothing
//! here ever forces a push. Only [`reset`] drops local work, because the user asked for exactly
//! that, and even it keeps a backup.

use std::cell::OnceCell;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};
use std::ffi::{OsStr, OsString};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use eframe::egui;

use crate::tr;

// ---------------------------------------------------------------------------
// What the UI works with.

/// Name and e-mail address that commits are recorded with.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Identity {
    pub name: String,
    pub email: String,
}

impl Identity {
    /// This identity with each empty field taken from `fallback`.
    fn or(&self, fallback: &Identity) -> Identity {
        let pick = |a: &str, b: &str| {
            if a.trim().is_empty() {
                b.trim().to_string()
            } else {
                a.trim().to_string()
            }
        };
        Identity {
            name: pick(&self.name, &fallback.name),
            email: pick(&self.email, &fallback.email),
        }
    }

    fn is_complete(&self) -> bool {
        !self.name.trim().is_empty() && !self.email.trim().is_empty()
    }
}

/// For commits that only need *some* author (the backup snapshot before a pull).
fn placeholder_identity() -> Identity {
    Identity {
        name: "Typisch".into(),
        email: "typisch@localhost".into(),
    }
}

/// HTTPS login for one host. Typisch keeps it in memory only; git's credential helper may store it.
#[derive(Clone)]
pub struct Credentials {
    pub username: String,
    pub password: String,
}

impl std::fmt::Debug for Credentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Credentials")
            .field("username", &self.username)
            .field("password", &"***")
            .finish()
    }
}

/// How a file differs from the last commit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Change {
    Modified,
    Added,
    Deleted,
    Renamed,
    Conflicted,
}

/// An uncommitted change inside the project folder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChangedFile {
    /// Relative to the project folder, with '/' separators.
    pub path: String,
    pub change: Change,
}

/// Where the project stands in its repository.
#[derive(Clone, Debug, Default)]
pub struct RepoStatus {
    pub repo_root: PathBuf,
    /// None = detached HEAD.
    pub branch: Option<String>,
    /// The branch has no commits yet.
    pub unborn: bool,
    /// Upstream remote name (only a real remote, never ".").
    pub remote: Option<String>,
    pub remote_url: Option<String>,
    /// Branch name on the remote, e.g. "main".
    pub remote_branch: Option<String>,
    /// The remote-tracking ref exists locally (the branch existed at the last fetch).
    pub remote_branch_exists: bool,
    /// Local commits not on the remote branch (all of them while it does not exist yet).
    pub ahead: usize,
    /// Remote commits not pulled yet, as of the last fetch.
    pub behind: usize,
    /// Uncommitted changes inside the project folder.
    pub changes: Vec<ChangedFile>,
    /// Unix seconds of the last push from Typisch (git config `typisch.lastPush`).
    pub last_push: Option<u64>,
    /// "Merge", "Rebase", "Cherry-Pick" or "Revert" when one is unfinished (unmerged paths count as "Merge").
    pub in_progress: Option<String>,
    /// Effective `user.name` / `user.email` from git's config (may be empty).
    pub config_identity: Identity,
    /// The address of a remote the repository already has (the upstream's, else origin's,
    /// else the first one), to suggest when linking.
    pub known_url: Option<String>,
}

impl RepoStatus {
    /// Pull and Push are available: a branch that has an upstream branch on a real remote.
    pub fn linked(&self) -> bool {
        self.remote.is_some() && self.remote_branch.is_some() && self.branch.is_some()
    }
}

/// A file a pull cannot merge on its own. Paths are relative to the repository root.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Question {
    /// A non-text file changed on both sides.
    BinaryChanged(String),
    /// Deleted in the repository, modified (or re-added) here.
    DeletedInRepo(String),
    /// Created on both sides with different content.
    AddedBoth(String),
}

impl Question {
    pub fn path(&self) -> &str {
        match self {
            Question::BinaryChanged(p) | Question::DeletedInRepo(p) | Question::AddedBoth(p) => p,
        }
    }
}

/// The answer to a [`Question`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Choice {
    /// Keep this computer's version (or its deletion).
    Local,
    /// Take the repository's version (or its deletion).
    Remote,
}

/// What a pull would do, worked out without changing anything but the remote-tracking ref.
#[derive(Clone, Debug)]
pub struct PullPlan {
    pub behind: usize,
    /// The commit that would be pulled ("" when the remote branch does not exist).
    pub upstream: String,
    pub questions: Vec<Question>,
    /// Text files changed on both sides; both sides' lines are kept, so the user should check them.
    pub merged_text: Vec<String>,
}

#[derive(Clone, Debug, Default)]
pub struct PullOutcome {
    pub commits: usize,
    pub merged_text: Vec<String>,
    pub kept_local: Vec<String>,
    pub took_remote: Vec<String>,
}

/// What a reset did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ResetOutcome {
    /// The folder differed from the remote branch (false = nothing was changed).
    pub changed: bool,
    /// The ref that keeps what was dropped (refs/typisch/backup/…), when anything was.
    pub backup: Option<String>,
}

#[derive(Clone, Debug)]
pub struct PushOutcome {
    /// Commits that reached the remote (0 = nothing to push).
    pub commits: usize,
}

#[derive(Clone, Debug)]
pub enum GitError {
    /// git is missing, not runnable or older than 2.28.
    NotInstalled,
    NotARepo,
    NotLinked,
    Detached,
    /// An unfinished merge, rebase, … ("Merge").
    InProgress(String),
    AuthRequired {
        host: String,
        ssh: bool,
        detail: String,
    },
    /// The repository's address.
    RepoNotFound(String),
    /// Push refused: the remote has commits that are not here yet.
    NeedsPull {
        behind: usize,
    },
    /// Link refused: the remote branch has a different history.
    Unrelated {
        branch: String,
    },
    /// Link refused: the same paths exist on both sides with different content (at most 8 listed).
    Overlap {
        paths: Vec<String>,
    },
    /// Clone into a folder that is not empty.
    DestinationExists(PathBuf),
    Timeout,
    /// A message for the user (usually the end of git's own).
    Other(String),
}

impl std::fmt::Display for GitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GitError::NotInstalled if cfg!(target_os = "macos") => {
                f.write_str(tr!("Git ist nicht verfügbar. Installiere die Xcode Command Line Tools (xcode-select --install)."))
            }
            GitError::NotInstalled => f.write_str(tr!("Git ist nicht installiert.")),
            GitError::NotARepo => f.write_str(tr!("Dieser Ordner ist kein Git-Repository.")),
            GitError::NotLinked => f.write_str(tr!("Das Projekt ist mit keinem Repository verbunden.")),
            GitError::Detached => f.write_str(tr!("Es ist kein Branch ausgecheckt (losgelöster HEAD).")),
            GitError::InProgress(op) => {
                f.write_str(&tr!("Im Repository ist ein {op} nicht abgeschlossen. Schließe ihn zuerst im Terminal ab.", op: op))
            }
            GitError::AuthRequired { host, .. } if host.is_empty() => f.write_str(tr!("Anmeldung erforderlich.")),
            GitError::AuthRequired { host, .. } => f.write_str(&tr!("Anmeldung bei {host} erforderlich.", host: host)),
            GitError::RepoNotFound(url) => f.write_str(&tr!("Repository nicht gefunden: {url}", url: redact_url(url))),
            GitError::NeedsPull { .. } => {
                f.write_str(tr!("Im Repository gibt es neue Änderungen. Hole sie zuerst mit Pull und pushe dann erneut."))
            }
            GitError::Unrelated { branch } => f.write_str(&tr!(
                "Der Branch „{branch}“ enthält eine andere Versionsgeschichte. Wähle einen neuen oder leeren Branch – oder klone das Repository als neues Projekt.",
                branch: branch
            )),
            GitError::Overlap { paths } => f.write_str(&tr!(
                "Diese Dateien gibt es hier und im Repository mit unterschiedlichem Inhalt: {files}. Wähle einen neuen Branch oder klone das Repository.",
                files: paths.join(", ")
            )),
            GitError::DestinationExists(path) => {
                f.write_str(&tr!("Der Ordner {path} existiert bereits und ist nicht leer.", path: path.display()))
            }
            GitError::Timeout => f.write_str(tr!("Git hat zu lange nicht geantwortet.")),
            GitError::Other(message) => write!(f, "{message}"),
        }
    }
}

fn no_url() -> &'static str {
    tr!("Bitte eine Repository-Adresse angeben.")
}

#[derive(Debug)]
pub enum Job {
    Status {
        project: PathBuf,
    },
    /// Background check for new commits; quiet.
    Fetch {
        project: PathBuf,
        creds: Option<Credentials>,
    },
    /// Fetch and work out the questions; changes nothing locally.
    PlanPull {
        project: PathBuf,
        creds: Option<Credentials>,
    },
    Pull {
        project: PathBuf,
        creds: Option<Credentials>,
        answers: Vec<(String, Choice)>,
        identity: Identity,
    },
    Push {
        project: PathBuf,
        creds: Option<Credentials>,
        message: String,
        identity: Identity,
    },
    /// Make the folder match the remote branch, dropping local commits and changes.
    Reset {
        project: PathBuf,
        creds: Option<Credentials>,
        identity: Identity,
    },
    Link {
        project: PathBuf,
        creds: Option<Credentials>,
        url: String,
        branch: String,
    },
    Unlink {
        project: PathBuf,
    },
    Clone {
        creds: Option<Credentials>,
        url: String,
        branch: Option<String>,
        dest: PathBuf,
    },
}

#[derive(Debug)]
pub enum Reply {
    /// Ok(None) = the project is not in a git work tree.
    Status {
        project: PathBuf,
        result: Result<Option<RepoStatus>, GitError>,
    },
    Fetched {
        project: PathBuf,
        result: Result<(), GitError>,
    },
    Planned {
        project: PathBuf,
        result: Result<PullPlan, GitError>,
    },
    Pulled {
        project: PathBuf,
        result: Result<PullOutcome, GitError>,
    },
    Pushed {
        project: PathBuf,
        result: Result<PushOutcome, GitError>,
    },
    Reset {
        project: PathBuf,
        result: Result<ResetOutcome, GitError>,
    },
    Linked {
        project: PathBuf,
        result: Result<(), GitError>,
    },
    Unlinked {
        project: PathBuf,
        result: Result<(), GitError>,
    },
    Cloned {
        dest: PathBuf,
        result: Result<PathBuf, GitError>,
    },
}

// ---------------------------------------------------------------------------
// The worker thread.

/// One background thread that runs git jobs one at a time, in order.
pub struct GitWorker {
    tx: Sender<Job>,
    pub rx: Receiver<Reply>,
}

impl GitWorker {
    pub fn spawn(ctx: egui::Context) -> Self {
        Self::spawn_with(ctx, Vec::new())
    }

    /// A worker whose git never sees the user's own configuration (see [`Git::isolated`]).
    #[cfg(test)]
    pub fn spawn_isolated(ctx: egui::Context, home: &Path) -> Self {
        Self::spawn_with(ctx, Git::new(home).isolated(home).env)
    }

    /// `env` is added to every git call of this worker.
    fn spawn_with(ctx: egui::Context, env: Vec<(OsString, OsString)>) -> Self {
        let (tx, jobs) = channel::<Job>();
        let (replies, rx) = channel::<Reply>();
        std::thread::Builder::new()
            .name("git".into())
            .spawn(move || worker(jobs, replies, ctx, env))
            .expect("spawn git thread");
        Self { tx, rx }
    }

    pub fn send(&self, job: Job) {
        let _ = self.tx.send(job);
    }
}

fn worker(jobs: Receiver<Job>, replies: Sender<Reply>, ctx: egui::Context, env: Vec<(OsString, OsString)>) {
    let mut queue: VecDeque<Job> = VecDeque::new();
    // Checked before the first job. A failure is retried only when the user starts something
    // again: on macOS a missing git is a shim that opens an installer, and background jobs must
    // not pop that dialog again and again.
    let mut installed: Option<Result<(), GitError>> = None;
    loop {
        if queue.is_empty() {
            match jobs.recv() {
                Ok(job) => queue.push_back(job),
                Err(_) => break,
            }
        }
        while let Ok(job) = jobs.try_recv() {
            queue.push_back(job);
        }
        coalesce(&mut queue);
        let Some(job) = queue.pop_front() else { continue };
        let user_started = !matches!(job, Job::Status { .. } | Job::Fetch { .. });
        if installed.is_none() || (user_started && !matches!(installed, Some(Ok(())))) {
            installed = Some(check_version(&env));
        }
        let answers = match installed.as_ref().expect("checked") {
            Ok(()) => run_guarded(job, &env),
            Err(e) => vec![failed(job, e.clone())],
        };
        for reply in answers {
            if replies.send(reply).is_err() {
                return;
            }
        }
        ctx.request_repaint();
    }
}

/// Drops queued jobs that a later one makes pointless: older Status jobs for the same project,
/// and a Fetch next to a PlanPull, Pull, Push or Reset of the same project (those fetch anyway).
fn coalesce(queue: &mut VecDeque<Job>) {
    let mut keep = vec![true; queue.len()];
    let mut seen = HashSet::new();
    for (i, job) in queue.iter().enumerate().rev() {
        if let Job::Status { project } = job {
            keep[i] = seen.insert(project.clone());
        }
    }
    let fetching: HashSet<&PathBuf> = queue
        .iter()
        .filter_map(|job| match job {
            Job::PlanPull { project, .. } | Job::Pull { project, .. } | Job::Push { project, .. } | Job::Reset { project, .. } => {
                Some(project)
            }
            _ => None,
        })
        .collect();
    for (i, job) in queue.iter().enumerate() {
        if let Job::Fetch { project, .. } = job {
            keep[i] &= !fetching.contains(project);
        }
    }
    let mut flags = keep.into_iter();
    queue.retain(|_| flags.next().unwrap_or(true));
}

/// Runs a job; a panic becomes an error reply instead of a dead worker.
fn run_guarded(job: Job, env: &[(OsString, OsString)]) -> Vec<Reply> {
    let fallback = blank(&job);
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run_job(job, env)))
        .unwrap_or_else(|_| vec![failed(fallback, GitError::Other(tr!("Interner Fehler bei Git Sync.").into()))])
}

fn run_job(job: Job, env: &[(OsString, OsString)]) -> Vec<Reply> {
    let git = |dir: &Path| Git::new(dir).with_env(env);
    let fresh = |project: &PathBuf| Reply::Status {
        project: project.clone(),
        result: status(&git(project)),
    };
    match job {
        Job::Status { project } => vec![fresh(&project)],
        Job::Fetch { project, creds } => {
            let result = fetch(&git(&project).with_creds(creds));
            vec![
                Reply::Fetched {
                    project: project.clone(),
                    result,
                },
                fresh(&project),
            ]
        }
        Job::PlanPull { project, creds } => {
            // Its fetch moves the tracking ref, so the status (the Pull badge) changes too.
            let result = plan_pull(&git(&project).with_creds(creds));
            vec![
                Reply::Planned {
                    project: project.clone(),
                    result,
                },
                fresh(&project),
            ]
        }
        Job::Pull {
            project,
            creds,
            answers,
            identity,
        } => {
            let result = pull(&git(&project).with_creds(creds), &answers, &identity);
            vec![
                Reply::Pulled {
                    project: project.clone(),
                    result,
                },
                fresh(&project),
            ]
        }
        Job::Push {
            project,
            creds,
            message,
            identity,
        } => {
            let result = push(&git(&project).with_creds(creds), &message, &identity);
            vec![
                Reply::Pushed {
                    project: project.clone(),
                    result,
                },
                fresh(&project),
            ]
        }
        Job::Reset { project, creds, identity } => {
            let result = reset(&git(&project).with_creds(creds), &identity);
            vec![
                Reply::Reset {
                    project: project.clone(),
                    result,
                },
                fresh(&project),
            ]
        }
        Job::Link {
            project,
            creds,
            url,
            branch,
        } => {
            let result = link(&git(&project).with_creds(creds), &url, &branch);
            vec![
                Reply::Linked {
                    project: project.clone(),
                    result,
                },
                fresh(&project),
            ]
        }
        Job::Unlink { project } => {
            let result = unlink(&git(&project));
            vec![
                Reply::Unlinked {
                    project: project.clone(),
                    result,
                },
                fresh(&project),
            ]
        }
        Job::Clone { creds, url, branch, dest } => {
            let parent = dest.parent().unwrap_or(&dest).to_path_buf();
            let result = clone_repo(&git(&parent).with_creds(creds), &url, branch.as_deref(), &dest);
            vec![Reply::Cloned { dest, result }]
        }
    }
}

/// A copy of the job without its payload, for answering it after a panic.
fn blank(job: &Job) -> Job {
    match job {
        Job::Status { project } => Job::Status { project: project.clone() },
        Job::Fetch { project, .. } => Job::Fetch {
            project: project.clone(),
            creds: None,
        },
        Job::PlanPull { project, .. } => Job::PlanPull {
            project: project.clone(),
            creds: None,
        },
        Job::Pull { project, .. } => Job::Pull {
            project: project.clone(),
            creds: None,
            answers: Vec::new(),
            identity: Identity::default(),
        },
        Job::Push { project, .. } => Job::Push {
            project: project.clone(),
            creds: None,
            message: String::new(),
            identity: Identity::default(),
        },
        Job::Reset { project, .. } => Job::Reset {
            project: project.clone(),
            creds: None,
            identity: Identity::default(),
        },
        Job::Link { project, .. } => Job::Link {
            project: project.clone(),
            creds: None,
            url: String::new(),
            branch: String::new(),
        },
        Job::Unlink { project } => Job::Unlink { project: project.clone() },
        Job::Clone { dest, .. } => Job::Clone {
            creds: None,
            url: String::new(),
            branch: None,
            dest: dest.clone(),
        },
    }
}

/// The reply for a job that could not run at all.
fn failed(job: Job, e: GitError) -> Reply {
    match job {
        Job::Status { project } => Reply::Status { project, result: Err(e) },
        Job::Fetch { project, .. } => Reply::Fetched { project, result: Err(e) },
        Job::PlanPull { project, .. } => Reply::Planned { project, result: Err(e) },
        Job::Pull { project, .. } => Reply::Pulled { project, result: Err(e) },
        Job::Push { project, .. } => Reply::Pushed { project, result: Err(e) },
        Job::Reset { project, .. } => Reply::Reset { project, result: Err(e) },
        Job::Link { project, .. } => Reply::Linked { project, result: Err(e) },
        Job::Unlink { project } => Reply::Unlinked { project, result: Err(e) },
        Job::Clone { dest, .. } => Reply::Cloned { dest, result: Err(e) },
    }
}

/// git ≥ 2.28 is needed (`init -b`); an older or missing git counts as not installed.
fn check_version(env: &[(OsString, OsString)]) -> Result<(), GitError> {
    let out = Git::new(&std::env::temp_dir()).with_env(env).cmd(&["--version"]).output()?;
    match parse_version(&out.text()) {
        Some(version) if out.ok() && version >= (2, 28) => Ok(()),
        _ => Err(GitError::NotInstalled),
    }
}

/// "git version 2.54.0 (Apple Git-157)" → (2, 54).
fn parse_version(text: &str) -> Option<(u32, u32)> {
    let version = text.trim().strip_prefix("git version ")?.split_whitespace().next()?;
    let mut parts = version.split('.');
    Some((parts.next()?.parse().ok()?, parts.next()?.parse().ok()?))
}

// ---------------------------------------------------------------------------
// Askpass: git starts Typisch itself to ask for a login.

/// When git runs Typisch as its GIT_ASKPASS helper: pass git's prompt to the Typisch that started
/// git, print its answer and return the exit code (main exits with it before opening any
/// window). None in a normal start.
pub fn askpass_main() -> Option<i32> {
    std::env::var_os("TYPISCH_GIT_ASKPASS")?;
    let prompt = std::env::args().nth(1).unwrap_or_default();
    let Some(answer) = askpass_ask(&prompt) else { return Some(1) };
    let mut out = std::io::stdout();
    Some(if writeln!(out, "{answer}").and_then(|_| out.flush()).is_ok() {
        0
    } else {
        1
    })
}

/// Where the helper asks: the socket of the [`AskpassServer`] that runs for this git call.
const ASKPASS_SOCKET: &str = "TYPISCH_GIT_ASKPASS_SOCKET";
/// How long either side waits for the other.
#[cfg(unix)]
const ASKPASS_PATIENCE: Duration = Duration::from_secs(5);

/// The helper's side: sends the prompt, gets the answer ("+" and the text) or nothing.
#[cfg(unix)]
fn askpass_ask(prompt: &str) -> Option<String> {
    ask_server(Path::new(&std::env::var_os(ASKPASS_SOCKET)?), prompt)
}

#[cfg(unix)]
fn ask_server(socket: &Path, prompt: &str) -> Option<String> {
    let mut stream = std::os::unix::net::UnixStream::connect(socket).ok()?;
    stream.set_read_timeout(Some(ASKPASS_PATIENCE)).ok()?;
    stream.set_write_timeout(Some(ASKPASS_PATIENCE)).ok()?;
    // One line: the prompt has none of its own, and nothing else may be smuggled into it.
    let line: String = prompt.chars().map(|c| if c == '\n' || c == '\r' { ' ' } else { c }).collect();
    stream.write_all(format!("{line}\n").as_bytes()).ok()?;
    let mut reply = String::new();
    stream.take(64 * 1024).read_to_string(&mut reply).ok()?;
    Some(reply.strip_prefix('+')?.trim_end_matches('\n').to_string())
}

/// Hands a login to the askpass helper of one network call, and to nothing else. It used to
/// travel in the environment of git, where every program git starts inherits it: hooks, filters,
/// git-lfs, ssh, and helpers that stay around long after the call (a credential cache, an ssh
/// connection master, a background `gc`). Now the environment only names a socket in a folder
/// that no other user can enter, which answers while the call runs: the right prompt for the
/// right host gets the login, everything else nothing, and when git has finished the socket is
/// gone, so whatever still carries its name has nothing to ask.
///
/// A hook of the repository runs as the user and inside this call, so it could ask the socket
/// like the helper does. Hooks are left on all the same: they are the user's own (they are not
/// cloned), git-lfs uploads through one (`pre-push`), and a program that runs as the user can
/// read the user's stored logins anyway.
#[cfg(unix)]
struct AskpassServer {
    dir: PathBuf,
    socket: PathBuf,
    stop: Arc<std::sync::atomic::AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

#[cfg(unix)]
impl AskpassServer {
    fn start(host: Option<String>, creds: Credentials) -> std::io::Result<Self> {
        use std::os::unix::net::UnixListener;
        use std::sync::atomic::{AtomicBool, Ordering};
        let (dir, listener) = Self::listen()?;
        let socket = dir.join("s");
        let cleanup = |e| {
            let _ = std::fs::remove_file(&socket);
            let _ = std::fs::remove_dir(&dir);
            e
        };
        // Polled, so that stopping never depends on someone connecting.
        listener.set_nonblocking(true).map_err(cleanup)?;
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = Arc::clone(&stop);
        let serve = move |listener: UnixListener| {
            while !stopped.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((stream, _)) => Self::answer(stream, host.as_deref(), &creds),
                    Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted) => {
                        std::thread::sleep(POLL)
                    }
                    Err(_) => break,
                }
            }
        };
        let thread = std::thread::Builder::new()
            .name("git-askpass".into())
            .spawn(move || serve(listener))
            .map_err(cleanup)?;
        Ok(Self {
            dir,
            socket,
            stop,
            thread: Some(thread),
        })
    }

    /// A socket named "s" in a new folder only this user can enter. Socket paths are short
    /// (about 100 bytes), so a long temporary folder falls back to /tmp.
    fn listen() -> std::io::Result<(PathBuf, std::os::unix::net::UnixListener)> {
        use std::os::unix::fs::DirBuilderExt;
        use std::sync::atomic::{AtomicUsize, Ordering};
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let mut last = std::io::Error::other("no folder for the login socket");
        for base in [std::env::temp_dir(), PathBuf::from("/tmp")] {
            // Creating fails if the name is taken, so a folder someone else prepared is never used.
            for _ in 0..16 {
                let n = NEXT.fetch_add(1, Ordering::Relaxed);
                let dir = base.join(format!("typisch-login-{}-{n}", std::process::id()));
                match std::fs::DirBuilder::new().mode(0o700).create(&dir) {
                    Ok(()) => match std::os::unix::net::UnixListener::bind(dir.join("s")) {
                        Ok(listener) => return Ok((dir, listener)),
                        Err(e) => {
                            let _ = std::fs::remove_dir(&dir);
                            last = e;
                            break;
                        }
                    },
                    Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => last = e,
                    Err(e) => {
                        last = e;
                        break;
                    }
                }
            }
        }
        Err(last)
    }

    /// One question: a line with git's prompt in, "+answer" out, or nothing.
    fn answer(stream: std::os::unix::net::UnixStream, host: Option<&str>, creds: &Credentials) {
        // On some systems the connection inherits the listener's non-blocking mode.
        let ready = stream.set_nonblocking(false).is_ok()
            && stream.set_read_timeout(Some(ASKPASS_PATIENCE)).is_ok()
            && stream.set_write_timeout(Some(ASKPASS_PATIENCE)).is_ok();
        if !ready {
            return;
        }
        let mut prompt = String::new();
        if std::io::BufRead::read_line(&mut std::io::BufReader::new((&stream).take(4096)), &mut prompt).is_err() {
            return;
        }
        let prompt = prompt.trim_end_matches(['\n', '\r']);
        if let Some(answer) = askpass_answer(prompt, host, Some(creds.username.clone()), Some(creds.password.clone())) {
            let _ = (&stream).write_all(format!("+{answer}\n").as_bytes());
        }
    }
}

#[cfg(unix)]
impl Drop for AskpassServer {
    fn drop(&mut self) {
        self.stop.store(true, std::sync::atomic::Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        let _ = std::fs::remove_file(&self.socket);
        let _ = std::fs::remove_dir(&self.dir);
    }
}

/// Without Unix sockets the login travels in git's environment, as it always did there.
#[cfg(not(unix))]
fn askpass_ask(prompt: &str) -> Option<String> {
    let host = std::env::var("TYPISCH_GIT_HOST").ok();
    askpass_answer(
        prompt,
        host.as_deref(),
        std::env::var("TYPISCH_GIT_USERNAME").ok(),
        std::env::var("TYPISCH_GIT_PASSWORD").ok(),
    )
}

#[cfg(not(unix))]
struct AskpassServer;

/// Only username and password prompts for the host this job is talking to are answered; anything
/// else (an ssh host-key question, or a prompt for another host) fails. Git puts the address it
/// asks for in the prompt ("Password for 'https://user@host': "), which is what is checked.
fn askpass_answer(prompt: &str, host: Option<&str>, username: Option<String>, password: Option<String>) -> Option<String> {
    let kind = if prompt.starts_with("Username") {
        username
    } else if prompt.starts_with("Password") {
        password
    } else {
        return None;
    };
    let (expected, asked) = (host?, prompt_host(prompt)?);
    if !expected.eq_ignore_ascii_case(&asked) {
        return None;
    }
    kind
}

/// The host in git's "… for 'https://user@host': " prompt.
fn prompt_host(prompt: &str) -> Option<String> {
    let rest = &prompt[prompt.find("for '")? + 5..];
    let url = &rest[..rest.find('\'')?];
    let a = address(url);
    (!a.host.is_empty() && a.scheme != "file").then(|| a.host.to_lowercase())
}

// ---------------------------------------------------------------------------
// Running git.

const NETWORK_TIMEOUT: Duration = Duration::from_secs(180);
const LOCAL_TIMEOUT: Duration = Duration::from_secs(300);
const POLL: Duration = Duration::from_millis(20);
const STDERR_LIMIT: usize = 256 * 1024;

#[cfg(unix)]
const SIGTERM: i32 = 15;
#[cfg(unix)]
const SIGKILL: i32 = 9;

#[cfg(unix)]
unsafe extern "C" {
    fn kill(pid: i32, sig: i32) -> i32;
}

/// Signals the whole process group of a git call (it is spawned with its own group), so hooks,
/// filters and ssh children stop with it.
#[cfg(unix)]
fn signal_group(child: &std::process::Child, signal: i32) {
    // A negative pid means the process group whose id is the child's pid.
    unsafe {
        let _ = kill(-(child.id() as i32), signal);
    }
}

#[cfg(not(unix))]
fn signal_group(child: &std::process::Child, _signal: i32) {
    let _ = child.kill();
}
/// ssh must never wait for a password or host-key answer that nobody can type. The host key
/// policy is left to the user's own ssh configuration (an unknown host fails instead of being
/// trusted silently).
const SSH_COMMAND: &str = "ssh -o BatchMode=yes -o ConnectTimeout=20";
/// Inherited variables that would point git at another repository, index or pathspec syntax.
const FOREIGN_VARS: [&str; 12] = [
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_INDEX_FILE",
    "GIT_OBJECT_DIRECTORY",
    "GIT_ALTERNATE_OBJECT_DIRECTORIES",
    "GIT_COMMON_DIR",
    "GIT_NAMESPACE",
    "GIT_PREFIX",
    "GIT_GLOB_PATHSPECS",
    "GIT_NOGLOB_PATHSPECS",
    "GIT_ICASE_PATHSPECS",
    "GIT_LITERAL_PATHSPECS",
];

/// A pathspec for exactly this repo-root relative path: a file may well be called "*.typ" or
/// "[1].typ", and must then not match others. (Per argument rather than GIT_LITERAL_PATHSPECS,
/// which hooks would inherit and trip over.)
fn exact(path: &str) -> String {
    format!(":(top,literal){path}")
}

/// The context every git call runs in: the project folder, an optional login and (in tests) extra environment.
#[derive(Clone)]
pub struct Git {
    dir: PathBuf,
    creds: Option<Credentials>,
    env: Vec<(OsString, OsString)>,
    ssh_configured: OnceCell<bool>,
}

impl Git {
    pub fn new(project: &Path) -> Self {
        // Absolute, because git answers --git-path relative to it and the paths are then passed
        // to calls that run elsewhere (the repository root).
        let dir = std::path::absolute(project).unwrap_or_else(|_| project.to_path_buf());
        Self {
            dir,
            creds: None,
            env: Vec::new(),
            ssh_configured: OnceCell::new(),
        }
    }

    pub fn with_creds(mut self, creds: Option<Credentials>) -> Self {
        self.creds = creds;
        self
    }

    /// Extra environment for every call (the worker's; empty outside tests).
    fn with_env(mut self, env: &[(OsString, OsString)]) -> Self {
        self.env.extend_from_slice(env);
        self
    }

    /// Keeps git away from the user's own configuration (tests run in parallel, so this is per command).
    #[cfg(test)]
    fn isolated(mut self, home: &Path) -> Self {
        for (key, value) in [
            ("HOME", home.to_path_buf()),
            ("XDG_CONFIG_HOME", home.join(".config")),
            ("GIT_CONFIG_GLOBAL", home.join("gitconfig")),
            ("GIT_CONFIG_NOSYSTEM", PathBuf::from("1")),
        ] {
            self.env.push((key.into(), value.into()));
        }
        self
    }

    /// The same context for another folder (a fresh clone).
    fn at(&self, dir: &Path) -> Self {
        Self {
            dir: dir.to_path_buf(),
            ssh_configured: OnceCell::new(),
            ..self.clone()
        }
    }

    fn cmd<S: AsRef<OsStr>>(&self, args: &[S]) -> Cmd<'_> {
        Cmd {
            git: self,
            dir: self.dir.clone(),
            pre: Vec::new(),
            args: args.iter().map(|a| a.as_ref().to_os_string()).collect(),
            env: Vec::new(),
            url: None,
            host: None,
            limit: usize::MAX,
        }
    }

    /// The user's own ssh command (environment or `core.sshCommand`) wins over ours.
    fn ssh_configured(&self, dir: &Path) -> bool {
        *self.ssh_configured.get_or_init(|| {
            let set = |key: &str| {
                std::env::var_os(key).is_some_and(|v| !v.is_empty()) || self.env.iter().any(|(k, _)| k.as_os_str() == OsStr::new(key))
            };
            set("GIT_SSH_COMMAND")
                || set("GIT_SSH")
                || self
                    .cmd(&["config", "--get", "core.sshCommand"])
                    .in_dir(dir)
                    .output()
                    .is_ok_and(|o| o.ok())
        })
    }
}

/// One git invocation, built up like a `Command`.
struct Cmd<'a> {
    git: &'a Git,
    dir: PathBuf,
    /// Options before the subcommand (`-c key=value`).
    pre: Vec<OsString>,
    args: Vec<OsString>,
    env: Vec<(OsString, OsString)>,
    /// The remote's address for network calls: longer timeout, ssh options, login errors.
    url: Option<String>,
    /// The host of `url`, which the askpass helper only answers for.
    host: Option<String>,
    /// Bytes of stdout to keep; the rest is read and dropped.
    limit: usize,
}

/// What a finished git call printed.
struct Output {
    code: Option<i32>,
    stdout: Vec<u8>,
    stderr: String,
}

impl Output {
    fn ok(&self) -> bool {
        self.code == Some(0)
    }

    /// Stdout as text, without the final line break.
    fn text(&self) -> String {
        String::from_utf8_lossy(&self.stdout).trim_end_matches(['\n', '\r']).to_string()
    }
}

impl Cmd<'_> {
    fn arg(mut self, arg: impl AsRef<OsStr>) -> Self {
        self.args.push(arg.as_ref().to_os_string());
        self
    }

    fn args<S: AsRef<OsStr>>(mut self, args: impl IntoIterator<Item = S>) -> Self {
        self.args.extend(args.into_iter().map(|a| a.as_ref().to_os_string()));
        self
    }

    fn env(mut self, key: &str, value: impl AsRef<OsStr>) -> Self {
        self.env.push((key.into(), value.as_ref().to_os_string()));
        self
    }

    fn in_dir(mut self, dir: &Path) -> Self {
        self.dir = dir.to_path_buf();
        self
    }

    fn network(mut self, url: &str) -> Self {
        self.url = Some(url.to_string());
        self.host = host_of(url);
        self
    }

    fn limit(mut self, bytes: usize) -> Self {
        self.limit = bytes;
        self
    }

    /// Author and committer of any commit this call makes.
    fn identity(self, id: &Identity) -> Self {
        self.env("GIT_AUTHOR_NAME", &id.name)
            .env("GIT_AUTHOR_EMAIL", &id.email)
            .env("GIT_COMMITTER_NAME", &id.name)
            .env("GIT_COMMITTER_EMAIL", &id.email)
    }

    /// Typisch's union merge rule for this call only (see [`write_attributes`]).
    fn union(mut self, repo: &Repo) -> Self {
        let mut setting = OsString::from("core.attributesFile=");
        setting.push(&repo.attributes);
        self.pre.push("-c".into());
        self.pre.push(setting);
        self
    }

    fn command(&self) -> Command {
        let mut c = Command::new("git");
        c.arg("-C").arg(&self.dir);
        // A repository's local config must not run commands behind the user's back (fsmonitor,
        // clean filters run by status) and must not recurse into submodules during resets.
        c.arg("-c").arg("core.fsmonitor=false").arg("-c").arg("submodule.recurse=false");
        c.args(&self.pre).args(&self.args);
        for key in FOREIGN_VARS {
            c.env_remove(key);
        }
        c.env_remove("LANGUAGE")
            .env("LC_ALL", "C")
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GIT_EDITOR", "true")
            .env("GIT_MERGE_AUTOEDIT", "no")
            .env("GIT_PAGER", "cat")
            .env("GIT_HTTP_LOW_SPEED_LIMIT", "1000")
            .env("GIT_HTTP_LOW_SPEED_TIME", "30");
        if let Some(path) = search_path() {
            c.env("PATH", path);
        }
        // Git asks Typisch itself for a login (askpass_main), which only answers with one the user
        // typed, and only for the host this network call talks to. The login is never put into
        // the environment (see `offer_login`); local calls get no way to ask at all.
        c.env_remove(ASKPASS_SOCKET);
        if self.url.is_some() {
            // The test binary must never be started as that helper.
            let helper = if cfg!(test) {
                OsString::from("false")
            } else {
                std::env::current_exe().map(OsString::from).unwrap_or_else(|_| "false".into())
            };
            c.env("GIT_ASKPASS", helper).env("TYPISCH_GIT_ASKPASS", "1");
        } else {
            c.env("GIT_ASKPASS", "false").env_remove("TYPISCH_GIT_ASKPASS");
        }
        if self.url.is_some() && !self.git.ssh_configured(&self.dir) {
            c.env("GIT_SSH_COMMAND", SSH_COMMAND);
        }
        for (key, value) in self.git.env.iter().chain(&self.env) {
            c.env(key, value);
        }
        // Its own process group, so a timeout can stop git and everything it started.
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            c.process_group(0);
        }
        c.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
        c
    }

    /// Makes the user's login available to the askpass helper of this call, for as long as the
    /// returned server lives. Only network calls that have a login get one.
    #[cfg(unix)]
    fn offer_login(&self, c: &mut Command) -> Result<Option<AskpassServer>, GitError> {
        let (Some(_), Some(creds)) = (&self.url, &self.git.creds) else {
            return Ok(None);
        };
        let server = AskpassServer::start(self.host.clone(), creds.clone())
            .map_err(|e| GitError::Other(tr!("Die Anmeldung ließ sich nicht an Git übergeben: {e}", e: e)))?;
        c.env(ASKPASS_SOCKET, &server.socket);
        Ok(Some(server))
    }

    #[cfg(not(unix))]
    fn offer_login(&self, c: &mut Command) -> Result<Option<AskpassServer>, GitError> {
        if let (Some(_), Some(creds)) = (&self.url, &self.git.creds) {
            if let Some(host) = &self.host {
                c.env("TYPISCH_GIT_HOST", host);
            }
            c.env("TYPISCH_GIT_USERNAME", &creds.username)
                .env("TYPISCH_GIT_PASSWORD", &creds.password);
        }
        Ok(None)
    }

    /// Runs git whatever its exit code; Err only when git cannot run or takes too long.
    fn output(&self) -> Result<Output, GitError> {
        let timeout = if self.url.is_some() { NETWORK_TIMEOUT } else { LOCAL_TIMEOUT };
        let mut command = self.command();
        // Dropped when this call returns, on every path: after that nothing answers any more.
        let _login = self.offer_login(&mut command)?;
        let mut child = command.spawn().map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound | std::io::ErrorKind::PermissionDenied => GitError::NotInstalled,
            _ => GitError::Other(tr!("Git ließ sich nicht starten: {e}", e: e)),
        })?;
        let stdout = Drain::start(child.stdout.take(), self.limit);
        let stderr = Drain::start(child.stderr.take(), STDERR_LIMIT);
        let started = Instant::now();
        let mut pause = Duration::from_millis(1);
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) if started.elapsed() < timeout => {
                    std::thread::sleep(pause);
                    pause = (pause * 2).min(POLL);
                }
                Ok(None) => {
                    // Ask the whole process group to stop, so hooks, filters and ssh do not run on.
                    signal_group(&child, SIGTERM);
                    let grace = Instant::now();
                    while grace.elapsed() < Duration::from_secs(2) {
                        if child.try_wait().is_ok_and(|s| s.is_some()) {
                            break;
                        }
                        std::thread::sleep(POLL);
                    }
                    signal_group(&child, SIGKILL);
                    let _ = child.wait();
                    return Err(GitError::Timeout);
                }
                Err(e) => {
                    signal_group(&child, SIGKILL);
                    let _ = child.wait();
                    return Err(GitError::Other(tr!("Git ließ sich nicht ausführen: {e}", e: e)));
                }
            }
        };
        Ok(Output {
            code: status.code(),
            stdout: stdout.finish(),
            stderr: String::from_utf8_lossy(&stderr.finish()).into_owned(),
        })
    }

    /// Runs git; a non-zero exit becomes a [`GitError`].
    fn run(&self) -> Result<Output, GitError> {
        let out = self.output()?;
        if out.ok() { Ok(out) } else { Err(self.error(&out)) }
    }

    /// Runs git and returns its stdout as text.
    fn read(&self) -> Result<String, GitError> {
        self.run().map(|out| out.text())
    }

    fn error(&self, out: &Output) -> GitError {
        classify(&out.stderr, out.code, self.url.as_deref(), self.git.creds.is_some(), &self.dir)
    }
}

/// Reads a child's pipe on its own thread, so a full pipe can never stall git.
struct Drain {
    buf: Arc<Mutex<Vec<u8>>>,
    done: Receiver<()>,
}

impl Drain {
    fn start(pipe: Option<impl Read + Send + 'static>, limit: usize) -> Self {
        let buf = Arc::new(Mutex::new(Vec::new()));
        let (tx, done) = channel();
        let shared = Arc::clone(&buf);
        std::thread::spawn(move || {
            if let Some(mut pipe) = pipe {
                let mut chunk = [0u8; 16 * 1024];
                loop {
                    match pipe.read(&mut chunk) {
                        Ok(0) => break,
                        Ok(n) => {
                            let mut buf = shared.lock().unwrap_or_else(|e| e.into_inner());
                            let room = limit.saturating_sub(buf.len());
                            buf.extend_from_slice(&chunk[..n.min(room)]);
                        }
                        Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                        Err(_) => break,
                    }
                }
            }
            let _ = tx.send(());
        });
        Self { buf, done }
    }

    /// What was read. Waits only briefly for the end: a process git started (an ssh connection
    /// master, say) may keep the pipe open long after git itself has exited.
    fn finish(self) -> Vec<u8> {
        let _ = self.done.recv_timeout(Duration::from_millis(500));
        std::mem::take(&mut *self.buf.lock().unwrap_or_else(|e| e.into_inner()))
    }
}

/// GUI apps on macOS start with a bare PATH; git-lfs and hook tools usually live in Homebrew's folders.
fn search_path() -> Option<OsString> {
    if !cfg!(target_os = "macos") {
        return None;
    }
    let mut dirs: Vec<PathBuf> = match std::env::var_os("PATH") {
        Some(path) => std::env::split_paths(&path).collect(),
        None => ["/usr/bin", "/bin", "/usr/sbin", "/sbin"].map(PathBuf::from).to_vec(),
    };
    for extra in ["/opt/homebrew/bin", "/usr/local/bin"] {
        if !dirs.iter().any(|d| d == Path::new(extra)) {
            dirs.push(extra.into());
        }
    }
    std::env::join_paths(dirs).ok()
}

const AUTH_HINTS: [&str; 9] = [
    "authentication failed",
    "could not read username",
    "could not read password",
    "terminal prompts disabled",
    "invalid username or password",
    "permission denied (publickey",
    "access denied",
    "http basic: access denied",
    "the requested url returned error: 401",
];
const NETWORK_HINTS: [&str; 6] = [
    "could not resolve host",
    "connection timed out",
    "network is unreachable",
    "failed to connect",
    "connection refused",
    "operation timed out",
];

/// Turns git's complaint into an error the UI can act on (login dialog, "pull first", …).
/// Login and network problems are only looked for in calls that talked to a remote (`url`).
fn classify(stderr: &str, code: Option<i32>, url: Option<&str>, has_creds: bool, dir: &Path) -> GitError {
    let lower = stderr.to_lowercase();
    if lower.contains("dubious ownership") {
        let path = stderr
            .lines()
            .find_map(|line| {
                line.split_once("safe.directory ")
                    .map(|(_, p)| p.trim().trim_matches('\'').to_string())
            })
            .unwrap_or_else(|| dir.display().to_string());
        return GitError::Other(tr!(
            "Git vertraut diesem Ordner nicht, weil er einem anderen Benutzer gehört. Erlaube ihn im Terminal mit: git config --global --add safe.directory {path}",
            path: path
        ));
    }
    let detail = redact_url(&tail(stderr, code));
    let Some(url) = url else { return GitError::Other(detail) };
    let host = host_of(url).unwrap_or_default();
    let auth = || GitError::AuthRequired {
        host: host.clone(),
        ssh: is_ssh_url(url),
        detail: detail.clone(),
    };
    if AUTH_HINTS.iter().any(|hint| lower.contains(hint)) || has_word(&lower, "401") || has_word(&lower, "403") {
        return auth();
    }
    if lower.contains("not found in upstream") {
        return GitError::Other(tr!("Diesen Branch gibt es im Repository nicht.").into());
    }
    if lower.contains("not found") || lower.contains("does not appear to be a git repository") || has_word(&lower, "404") {
        // GitHub answers "not found" for private repositories when nobody is logged in.
        let https = url.starts_with("https://") || url.starts_with("http://");
        return if https && !has_creds {
            auth()
        } else {
            GitError::RepoNotFound(redact_url(url))
        };
    }
    if NETWORK_HINTS.iter().any(|hint| lower.contains(hint)) {
        return GitError::Other(if host.is_empty() {
            tr!("Keine Verbindung zum Git-Server. Prüfe die Internetverbindung.").to_string()
        } else {
            tr!("Keine Verbindung zu {host}. Prüfe die Internetverbindung.", host: host)
        });
    }
    GitError::Other(detail)
}

/// `word` stands on its own in `text` (so "403" does not match inside "project-2403").
fn has_word(text: &str, word: &str) -> bool {
    text.match_indices(word).any(|(i, _)| {
        let before = text[..i].chars().next_back();
        let after = text[i + word.len()..].chars().next();
        !before.is_some_and(char::is_alphanumeric) && !after.is_some_and(char::is_alphanumeric)
    })
}

/// The last lines of git's message, without hints.
fn tail(stderr: &str, code: Option<i32>) -> String {
    let lines: Vec<&str> = stderr
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with("hint:"))
        .collect();
    if lines.is_empty() {
        return match code {
            Some(code) => tr!("Git ist mit Fehler {code} abgebrochen.", code: code),
            None => tr!("Git wurde abgebrochen.").into(),
        };
    }
    lines[lines.len().saturating_sub(6)..].join("\n")
}

/// Replaces every "user:password@" login in the text with "user@": messages and details must not
/// show a password that was typed into an address field.
pub fn redact_url(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut pos = 0;
    while let Some(rel) = text[pos..].find("://") {
        let i = pos + rel;
        out.push_str(&text[pos..i + 3]);
        let after = i + 3;
        // The address ends at the next '/', space, quote or bracket; "user:password@" may sit in front.
        let end = text[after..]
            .find(['/', ' ', '\'', '"', ')', '<'])
            .map_or(text.len(), |e| after + e);
        match text[after..end].rsplit_once('@') {
            Some((userinfo, host)) => {
                out.push_str(userinfo.split(':').next().unwrap_or(userinfo));
                out.push('@');
                out.push_str(host);
            }
            None => out.push_str(&text[after..end]),
        }
        pos = end;
    }
    out.push_str(&text[pos..]);
    out
}

fn io_error(e: std::io::Error) -> GitError {
    GitError::Other(tr!("Dateifehler: {e}", e: e))
}

fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

// ---------------------------------------------------------------------------
// Reading the repository.

/// The unfinished operations git leaves marker files for, with their names for messages.
const OPERATIONS: [(&str, &str); 5] = [
    ("MERGE_HEAD", "Merge"),
    ("rebase-merge", "Rebase"),
    ("rebase-apply", "Rebase"),
    ("CHERRY_PICK_HEAD", "Cherry-Pick"),
    ("REVERT_HEAD", "Revert"),
];

/// The repository around the project folder.
struct Repo {
    root: PathBuf,
    /// The project folder relative to `root`, ending in '/' ("" when it is the root).
    prefix: String,
    index: PathBuf,
    /// Scratch index for looking at the work tree without touching the real one.
    temp_index: PathBuf,
    /// The merge attributes of Typisch's own merges.
    attributes: PathBuf,
    merge_head: PathBuf,
    /// Written before a pull resets the work tree, removed when the local work is back.
    journal: PathBuf,
    operation: Option<&'static str>,
    config: Config,
}

/// None when the project folder is not inside a git work tree.
fn open_repo(g: &Git) -> Result<Option<Repo>, GitError> {
    let mut cmd = g.cmd(&["rev-parse", "--show-toplevel", "--show-prefix"]);
    for name in ["index", "typisch-index", "typisch-attributes"]
        .into_iter()
        .chain(OPERATIONS.map(|(m, _)| m))
    {
        cmd = cmd.arg("--git-path").arg(name);
    }
    cmd = cmd.arg("--git-path").arg("typisch-pull");
    let out = cmd.output()?;
    if !out.ok() {
        let err = out.stderr.to_lowercase();
        let outside = err.contains("not a git repository") || err.contains("must be run in a work tree");
        if outside && !err.contains("dubious ownership") {
            return Ok(None);
        }
        return Err(cmd.error(&out));
    }
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    let lines: Vec<&str> = text.split('\n').collect();
    if lines.len() < 5 + OPERATIONS.len() {
        return Err(GitError::Other(tr!("Git hat unerwartet geantwortet (rev-parse).").into()));
    }
    // --git-path answers relative to the folder git ran in.
    let at = |i: usize| g.dir.join(lines[i]);
    let operation = OPERATIONS
        .iter()
        .enumerate()
        .find(|(i, _)| at(5 + i).exists())
        .map(|(_, (_, name))| *name);
    let root = PathBuf::from(lines[0]);
    // A repository-local core.worktree can point git (and our reset/add) at a completely
    // different folder. Refuse when git's work tree is not the one we found on disk.
    if let Some(found) = find_repo_root(&g.dir)
        && !same_path(&found, &root)
    {
        return Err(GitError::Other(tr!(
            "Dieses Repository verweist über core.worktree auf einen anderen Ordner ({path}). Entferne die Einstellung im Terminal.",
            path: root.display()
        )));
    }
    Ok(Some(Repo {
        root,
        prefix: lines[1].to_string(),
        index: at(2),
        temp_index: at(3),
        attributes: at(4),
        merge_head: at(5),
        journal: at(5 + OPERATIONS.len()),
        operation,
        config: Config::read(g)?,
    }))
}

fn same_path(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

/// Effective git configuration; the last value of a key wins, as in git. Section and variable
/// names are lower case ("typisch.lastpush"), subsections keep their case ("branch.Main.remote").
#[derive(Default)]
struct Config(HashMap<String, String>);

impl Config {
    fn read(g: &Git) -> Result<Self, GitError> {
        Ok(Self::parse(&g.cmd(&["config", "-z", "--list"]).run()?.stdout))
    }

    fn parse(raw: &[u8]) -> Self {
        let text = String::from_utf8_lossy(raw);
        let entries = text.split('\0').filter(|e| !e.is_empty()).map(|entry| {
            // A key without "= value" is a boolean true.
            let (key, value) = entry.split_once('\n').unwrap_or((entry, "true"));
            (key.to_string(), value.to_string())
        });
        Self(entries.collect())
    }

    fn get(&self, key: &str) -> Option<&str> {
        self.0.get(key).map(String::as_str)
    }

    /// Remote names with their URLs.
    fn remotes(&self) -> Vec<(&str, &str)> {
        let mut remotes: Vec<(&str, &str)> = self
            .0
            .iter()
            .filter_map(|(key, url)| Some((key.strip_prefix("remote.")?.strip_suffix(".url")?, url.as_str())))
            .collect();
        remotes.sort();
        remotes
    }

    fn has_remote_section(&self, name: &str) -> bool {
        let prefix = format!("remote.{name}.");
        self.0.keys().any(|k| k.starts_with(&prefix))
    }
}

/// `git status --porcelain=v2 --branch -z`, taken apart (paths stay repo-root relative).
#[derive(Debug, Default, PartialEq)]
struct Porcelain {
    head: Option<String>,
    initial: bool,
    upstream: Option<String>,
    ab: Option<(usize, usize)>,
    entries: Vec<(String, Change)>,
}

fn parse_porcelain(raw: &[u8]) -> Porcelain {
    let text = String::from_utf8_lossy(raw);
    let mut records = text.split('\0');
    let mut p = Porcelain::default();
    while let Some(record) = records.next() {
        if let Some(header) = record.strip_prefix("# ") {
            let (key, value) = header.split_once(' ').unwrap_or((header, ""));
            match key {
                "branch.oid" => p.initial = value == "(initial)",
                "branch.head" => p.head = (value != "(detached)").then(|| value.to_string()),
                "branch.upstream" => p.upstream = Some(value.to_string()),
                "branch.ab" => p.ab = parse_ab(value),
                _ => {}
            }
            continue;
        }
        let (kind, rest) = record.split_once(' ').unwrap_or((record, ""));
        let entry = match kind {
            "1" => fields_then_path(rest, 7).map(|(xy, path)| (path, change_of(xy))),
            "2" => {
                let entry = fields_then_path(rest, 8).map(|(_, path)| (path, Change::Renamed));
                records.next(); // the original path
                entry
            }
            "u" => fields_then_path(rest, 9).map(|(_, path)| (path, Change::Conflicted)),
            "?" if !rest.is_empty() => Some((rest, Change::Added)),
            _ => None,
        };
        if let Some((path, change)) = entry {
            p.entries.push((path.to_string(), change));
        }
    }
    p
}

/// The XY code and the path of an entry whose path follows `n` space-separated fields.
fn fields_then_path(rest: &str, n: usize) -> Option<(&str, &str)> {
    let fields: Vec<&str> = rest.splitn(n + 1, ' ').collect();
    (fields.len() == n + 1).then(|| (fields[0], fields[n]))
}

fn change_of(xy: &str) -> Change {
    if xy.contains('D') {
        Change::Deleted
    } else if xy.contains('A') {
        Change::Added
    } else {
        Change::Modified
    }
}

/// "+3 -2" → (3, 2).
fn parse_ab(value: &str) -> Option<(usize, usize)> {
    let mut parts = value.split_whitespace();
    let ahead = parts.next()?.strip_prefix('+')?.parse().ok()?;
    let behind = parts.next()?.strip_prefix('-')?.parse().ok()?;
    Some((ahead, behind))
}

/// Repository facts plus the status the UI shows; None outside a git work tree.
fn load(g: &Git) -> Result<Option<(Repo, RepoStatus)>, GitError> {
    let Some(repo) = open_repo(g)? else { return Ok(None) };
    // An interrupted pull (quit, crash, killed git) must not look like a clean folder.
    finish_interrupted_pull(g, &repo)?;
    let raw = g
        .cmd(&["status", "--porcelain=v2", "--branch", "-z", "--untracked-files=all", "--", "."])
        .env("GIT_OPTIONAL_LOCKS", "0")
        .run()?
        .stdout;
    let porcelain = parse_porcelain(&raw);
    // Conflicts anywhere in the repository block a pull, not only those inside the project folder.
    let conflicted = if repo.prefix.is_empty() {
        porcelain.entries.iter().any(|(_, change)| *change == Change::Conflicted)
    } else {
        !unmerged_paths(g, &repo)?.is_empty()
    };
    let config = &repo.config;
    let text = |key: &str| config.get(key).unwrap_or("").to_string();
    let mut st = RepoStatus {
        repo_root: repo.root.clone(),
        branch: porcelain.head.clone(),
        unborn: porcelain.initial,
        changes: porcelain
            .entries
            .iter()
            .filter_map(|(path, change)| {
                Some(ChangedFile {
                    path: path.strip_prefix(repo.prefix.as_str())?.to_string(),
                    change: *change,
                })
            })
            .collect(),
        last_push: config.get("typisch.lastpush").and_then(|v| v.trim().parse().ok()),
        in_progress: repo
            .operation
            .map(str::to_string)
            .or_else(|| conflicted.then(|| "Merge".to_string())),
        config_identity: Identity {
            name: text("user.name"),
            email: text("user.email"),
        },
        ..Default::default()
    };
    if let Some((remote, branch)) = st.branch.as_deref().and_then(|b| upstream_config(config, b)) {
        fill_upstream(g, &porcelain, &mut st, remote, branch)?;
    }
    st.known_url = st.remote_url.clone().or_else(|| known_remote_url(config));
    Ok(Some((repo, st)))
}

/// The upstream of `branch` from its config: a real remote (not ".") and a branch under refs/heads/.
/// (Read from the config rather than split from "origin/main": remote names may contain '/'.)
fn upstream_config(config: &Config, branch: &str) -> Option<(String, String)> {
    let remote = config
        .get(&format!("branch.{branch}.remote"))
        .filter(|r| !r.is_empty() && *r != ".")?;
    let merge = config.get(&format!("branch.{branch}.merge"))?;
    let remote_branch = merge.strip_prefix("refs/heads/").filter(|b| !b.is_empty())?;
    Some((remote.to_string(), remote_branch.to_string()))
}

/// origin's address, else the first remote's (a repository whose remote was added by hand but
/// never pushed to has no upstream yet).
fn known_remote_url(config: &Config) -> Option<String> {
    let remotes = config.remotes();
    remotes
        .iter()
        .find(|(name, _)| *name == "origin")
        .or(remotes.first())
        .map(|(_, url)| url.to_string())
}

/// Where our fetch keeps the remote branch.
fn tracking_ref(remote: &str, branch: &str) -> String {
    format!("refs/remotes/{remote}/{branch}")
}

fn fill_upstream(g: &Git, porcelain: &Porcelain, st: &mut RepoStatus, remote: String, branch: String) -> Result<(), GitError> {
    let url = g.cmd(&["remote", "get-url", "--"]).arg(&remote).output()?;
    st.remote_url = url.ok().then(|| url.text()).filter(|u| !u.is_empty());
    let tracking = resolve(g, &tracking_ref(&remote, &branch))?;
    st.remote_branch_exists = tracking.is_some();
    let upstream_name = format!("{remote}/{branch}");
    (st.ahead, st.behind) = match tracking {
        None if st.unborn => (0, 0),
        // Nothing there yet: the first push takes every local commit along.
        None => (count(g, "HEAD")?, 0),
        Some(t) if st.unborn => (0, count(g, &t)?),
        Some(t) => match porcelain.ab {
            Some(ab) if porcelain.upstream.as_deref() == Some(upstream_name.as_str()) => ab,
            _ => ahead_behind(g, &t)?,
        },
    };
    st.remote = Some(remote);
    st.remote_branch = Some(branch);
    Ok(())
}

/// The commit a revision names, or None when there is none.
fn resolve(g: &Git, rev: &str) -> Result<Option<String>, GitError> {
    let cmd = g.cmd(&["rev-parse", "--verify", "-q"]).arg(format!("{rev}^{{commit}}"));
    let out = cmd.output()?;
    match out.code {
        Some(0) => Ok(Some(out.text())),
        Some(1) => Ok(None),
        _ => Err(cmd.error(&out)),
    }
}

fn must_resolve(g: &Git, rev: &str) -> Result<String, GitError> {
    resolve(g, rev)?.ok_or_else(|| GitError::Other(tr!("Git kennt „{rev}“ nicht.", rev: rev)))
}

/// HEAD's commit; None while the branch is unborn.
fn head_commit(g: &Git, st: &RepoStatus) -> Result<Option<String>, GitError> {
    if st.unborn { Ok(None) } else { must_resolve(g, "HEAD").map(Some) }
}

fn count(g: &Git, range: &str) -> Result<usize, GitError> {
    let text = g.cmd(&["rev-list", "--count", range, "--"]).read()?;
    text.trim()
        .parse()
        .map_err(|_| GitError::Other(tr!("Git hat unerwartet geantwortet: {text}", text: text)))
}

/// Commits of `upstream` that `head` lacks (all of them when there is no head yet).
fn commits_between(g: &Git, head: Option<&str>, upstream: &str) -> Result<usize, GitError> {
    match head {
        Some(head) => count(g, &format!("{head}..{upstream}")),
        None => count(g, upstream),
    }
}

fn ahead_behind(g: &Git, upstream: &str) -> Result<(usize, usize), GitError> {
    let text = g
        .cmd(&["rev-list", "--left-right", "--count"])
        .arg(format!("HEAD...{upstream}"))
        .arg("--")
        .read()?;
    let mut numbers = text.split_whitespace().map(|n| n.parse::<usize>().unwrap_or(0));
    Ok((numbers.next().unwrap_or(0), numbers.next().unwrap_or(0)))
}

/// Paths with unmerged index entries anywhere in the repository.
fn unmerged_paths(g: &Git, repo: &Repo) -> Result<BTreeSet<String>, GitError> {
    let out = g.cmd(&["ls-files", "-u", "-z"]).in_dir(&repo.root).run()?;
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    Ok(text
        .split('\0')
        .filter_map(|entry| entry.split_once('\t'))
        .map(|(_, path)| path.to_string())
        .collect())
}

/// The branch on the remote that the project is linked to.
struct Upstream {
    remote: String,
    branch: String,
    url: String,
}

impl Upstream {
    fn tracking(&self) -> String {
        tracking_ref(&self.remote, &self.branch)
    }
}

fn linked_upstream(st: &RepoStatus) -> Result<Upstream, GitError> {
    if st.branch.is_none() {
        return Err(GitError::Detached);
    }
    match (&st.remote, &st.remote_branch) {
        (Some(remote), Some(branch)) => Ok(Upstream {
            remote: remote.clone(),
            branch: branch.clone(),
            url: st.remote_url.clone().unwrap_or_else(|| remote.clone()),
        }),
        _ => Err(GitError::NotLinked),
    }
}

/// The checks every pull and push starts with.
fn ready(g: &Git) -> Result<(Repo, RepoStatus, Upstream), GitError> {
    let (repo, st) = load(g)?.ok_or(GitError::NotARepo)?;
    let up = linked_upstream(&st)?;
    if let Some(op) = &st.in_progress {
        return Err(GitError::InProgress(op.clone()));
    }
    Ok((repo, st, up))
}

/// Fetches the upstream branch into its tracking ref; false when it does not exist on the remote (yet).
fn fetch_upstream(g: &Git, up: &Upstream) -> Result<bool, GitError> {
    let existed = g.cmd(&["show-ref", "--verify", "--quiet"]).arg(up.tracking()).output()?.ok();
    let refspec = format!("+refs/heads/{}:{}", up.branch, up.tracking());
    let cmd = g
        .cmd(&["fetch", "--no-tags", "--quiet", "--recurse-submodules=no", "--"])
        .arg(&up.remote)
        .arg(refspec)
        .network(&up.url);
    let out = cmd.output()?;
    if out.ok() {
        return Ok(true);
    }
    if out.stderr.to_lowercase().contains("couldn't find remote ref") {
        if existed {
            // The branch we were tracking is gone (renamed or deleted). Do not silently treat
            // this as "up to date" and later recreate it with the next push.
            return Err(GitError::Other(tr!(
                "Der Branch „{branch}“ wurde im Repository gelöscht oder umbenannt. Verbinde das Projekt im Terminal mit dem neuen Branch.",
                branch: up.branch
            )));
        }
        // Never there (a new, empty repository): nothing to count against.
        g.cmd(&["update-ref", "-d"]).arg(up.tracking()).run()?;
        return Ok(false);
    }
    Err(cmd.error(&out))
}

// ---------------------------------------------------------------------------
// Operations.

pub fn status(g: &Git) -> Result<Option<RepoStatus>, GitError> {
    Ok(load(g)?.map(|(_, st)| st))
}

pub fn fetch(g: &Git) -> Result<(), GitError> {
    let (_, st) = load(g)?.ok_or(GitError::NotARepo)?;
    fetch_upstream(g, &linked_upstream(&st)?).map(|_| ())
}

pub fn plan_pull(g: &Git) -> Result<PullPlan, GitError> {
    let (repo, st, up) = ready(g)?;
    let mut plan = PullPlan {
        behind: 0,
        upstream: String::new(),
        questions: Vec::new(),
        merged_text: Vec::new(),
    };
    if !fetch_upstream(g, &up)? {
        return Ok(plan);
    }
    plan.upstream = must_resolve(g, &up.tracking())?;
    let head = head_commit(g, &st)?;
    plan.behind = commits_between(g, head.as_deref(), &plan.upstream)?;
    if plan.behind > 0 {
        let base = pull_base(g, &up, head.as_deref(), &plan.upstream)?;
        (plan.questions, plan.merged_text) = analyse(g, &repo, base.as_deref(), &plan.upstream)?;
    }
    Ok(plan)
}

/// Brings the remote commits in. Local edits stay uncommitted; only diverged local commits lead
/// to a merge commit. Unanswered questions keep the local version.
pub fn pull(g: &Git, answers: &[(String, Choice)], id: &Identity) -> Result<PullOutcome, GitError> {
    let (repo, st, up) = ready(g)?;
    if !fetch_upstream(g, &up)? {
        return Ok(PullOutcome::default());
    }
    let upstream = must_resolve(g, &up.tracking())?;
    let head = head_commit(g, &st)?;
    let behind = commits_between(g, head.as_deref(), &upstream)?;
    if behind == 0 {
        return Ok(PullOutcome::default());
    }
    let base = pull_base(g, &up, head.as_deref(), &upstream)?;
    let (questions, merged_text) = analyse(g, &repo, base.as_deref(), &upstream)?;
    // Only the questions of this moment count (the repository may have moved since the plan).
    let given: HashMap<&str, Choice> = answers.iter().map(|(path, choice)| (path.as_str(), *choice)).collect();
    let choices: Vec<(String, Choice)> = questions
        .iter()
        .map(|q| (q.path().to_string(), given.get(q.path()).copied().unwrap_or(Choice::Local)))
        .collect();
    let paths = |wanted: Choice| -> Vec<String> { choices.iter().filter(|(_, c)| *c == wanted).map(|(p, _)| p.clone()).collect() };
    match (&head, &base) {
        (Some(head), Some(base)) => {
            let id = id.or(&st.config_identity).or(&placeholder_identity());
            let commits = Commits {
                head,
                base,
                upstream: &upstream,
            };
            merge_upstream(g, &repo, &up, &commits, &choices, &id)?;
        }
        // An unborn branch (a born one without a merge base was refused by pull_base).
        _ => adopt(g, &repo, &upstream, &paths(Choice::Remote))?,
    }
    Ok(PullOutcome {
        commits: behind,
        merged_text,
        kept_local: paths(Choice::Local),
        took_remote: paths(Choice::Remote),
    })
}

/// Commits the changes inside the project folder (only there) and pushes. Never forces: when the
/// remote moved on, nothing is committed (or our commit is taken back) and `NeedsPull` returned.
pub fn push(g: &Git, message: &str, id: &Identity) -> Result<PushOutcome, GitError> {
    let (_, st, up) = ready(g)?;
    let id = id.or(&st.config_identity);
    if !id.is_complete() {
        return Err(GitError::Other(tr!("Name und E-Mail-Adresse fehlen.").into()));
    }
    let upstream = if fetch_upstream(g, &up)? {
        Some(must_resolve(g, &up.tracking())?)
    } else {
        None
    };
    // `git switch -c experiment origin/main` leaves the new branch tracking origin/main. Pushing
    // that would publish the experiment on main, which plain `git push` refuses to do. A remote
    // branch that does not exist yet is different: linking to a new branch name wants its first
    // push to create it.
    if let Some(local) = &st.branch
        && local != &up.branch
        && upstream.is_some()
    {
        return Err(GitError::Other(tr!(
            "Der Branch „{local}“ verfolgt „{remote}/{branch}“. Ein Push würde nach „{target}“ schreiben. Benenne den Branch um oder verbinde ihn im Terminal neu.",
            local: local, remote: up.remote, branch: up.branch, target: up.branch
        )));
    }
    let before = head_commit(g, &st)?;
    if let Some(upstream) = &upstream {
        let behind = commits_between(g, before.as_deref(), upstream)?;
        if behind > 0 {
            return Err(GitError::NeedsPull { behind });
        }
    }
    let created = commit_project(g, &st, message, &id)?;
    let Some(head) = created.clone().or_else(|| before.clone()) else {
        return Ok(PushOutcome { commits: 0 });
    };
    match push_head(g, &up, &head, upstream.as_deref()) {
        Ok(commits) => Ok(PushOutcome { commits }),
        Err(e) => {
            // Only a definite refusal means the commit never reached the server. After a timeout
            // or a broken connection it may well be there, so keep the commit.
            if let (Some(created), GitError::NeedsPull { .. }) = (&created, &e) {
                undo_commit(g, created, before.as_deref());
            }
            Err(match e {
                GitError::NeedsPull { .. } => GitError::NeedsPull {
                    behind: behind_after_rejection(g, &up),
                },
                e => e,
            })
        }
    }
}

/// Makes the folder match the remote branch: local commits leave the branch, uncommitted changes
/// and new files leave the folder (ignored files stay). What is dropped stays reachable under
/// refs/typisch/backup/. If anything fails on the way, the folder is put back as it was.
pub fn reset(g: &Git, id: &Identity) -> Result<ResetOutcome, GitError> {
    let (repo, st, up) = ready(g)?;
    if !fetch_upstream(g, &up)? {
        return Err(GitError::Other(tr!(
            "Den Branch „{branch}“ gibt es im Repository noch nicht.",
            branch: up.branch
        )));
    }
    let upstream = must_resolve(g, &up.tracking())?;
    let Some(head) = head_commit(g, &st)? else {
        return Err(GitError::Other(
            tr!("Der Branch hat hier noch keine Commits. Hole die Änderungen zuerst mit Pull.").into(),
        ));
    };
    // The user was shown the project folder's changes only: nothing else may be dropped.
    let mut outside = changes_outside(g, &repo)?;
    if !outside.is_empty() {
        outside.truncate(8);
        return Err(GitError::Other(tr!(
            "Im Repository gibt es Änderungen außerhalb des Projektordners: {files}. Sichere oder verwirf sie zuerst im Terminal.",
            files: outside.join(", ")
        )));
    }
    stage_ignored_in_the_way(g, &repo, &upstream)?;
    let id = id.or(&st.config_identity).or(&placeholder_identity());
    let work = snapshot(g, &repo, &id, "Zurücksetzen")?;
    if work.is_none() && head == upstream {
        return Ok(ResetOutcome::default());
    }
    // Local commits are part of the snapshot's history; without one they get a ref of their own.
    let backup = match &work {
        Some(work) => Some(work.name.clone()),
        None if count(g, &format!("{upstream}..{head}"))? > 0 => Some(keep_ref(g, &repo, &head)?.name),
        None => None,
    };
    // If anything stops the reset after this point, the next start puts the folder back.
    write_journal(&repo, &head, work.as_ref())?;
    match g.cmd(&["reset", "-q", "--hard", upstream.as_str()]).in_dir(&repo.root).run() {
        Ok(_) => {
            remove_journal(&repo);
            Ok(ResetOutcome { changed: true, backup })
        }
        Err(e) => Err(recover(g, &repo, &head, work.as_ref(), e)),
    }
}

/// Connects the project folder with a branch of a remote repository: `git init` if needed, adds
/// (or reuses) the remote and sets the upstream. Anything this call created is removed on failure.
pub fn link(g: &Git, url: &str, branch: &str) -> Result<(), GitError> {
    let (url, branch) = (url.trim(), branch.trim());
    if url.is_empty() {
        return Err(GitError::Other(no_url().into()));
    }
    if branch.is_empty() {
        return Err(GitError::Other(tr!("Bitte einen Branch angeben.").into()));
    }
    check_branch(g, &g.dir, branch)?;
    let mut created = Created::default();
    let result = link_inner(g, url, branch, &mut created);
    if result.is_err() {
        created.roll_back(g);
    }
    result
}

/// Removes the upstream of the current branch. The remote, the `.git` folder and all history stay.
pub fn unlink(g: &Git) -> Result<(), GitError> {
    let (_, st) = load(g)?.ok_or(GitError::NotARepo)?;
    let branch = st.branch.ok_or(GitError::Detached)?;
    for key in ["remote", "merge"] {
        let cmd = g.cmd(&["config", "--unset-all"]).arg(format!("branch.{branch}.{key}"));
        let out = cmd.output()?;
        // 5 = the key was not set.
        if !out.ok() && out.code != Some(5) {
            return Err(cmd.error(&out));
        }
    }
    Ok(())
}

/// Clones `url` into `dest` (a new or empty folder). An empty repository gets its upstream set,
/// so the first push creates the branch.
pub fn clone_repo(g: &Git, url: &str, branch: Option<&str>, dest: &Path) -> Result<PathBuf, GitError> {
    let url = url.trim();
    if url.is_empty() {
        return Err(GitError::Other(no_url().into()));
    }
    let branch = branch.map(str::trim).filter(|b| !b.is_empty());
    if dest.is_file() || std::fs::read_dir(dest).is_ok_and(|mut entries| entries.next().is_some()) {
        return Err(GitError::DestinationExists(dest.to_path_buf()));
    }
    let (Some(parent), Some(name)) = (dest.parent(), dest.file_name()) else {
        return Err(GitError::Other(tr!("Ungültiger Zielordner: {path}", path: dest.display())));
    };
    let created = !dest.exists();
    std::fs::create_dir_all(parent).map_err(io_error)?;
    if let Some(branch) = branch {
        check_branch(g, parent, branch)?;
    }
    let result = clone_into(g, url, branch, parent, name, dest);
    if result.is_err() {
        remove_clone(dest, created);
    }
    result.map(|()| dest.to_path_buf())
}

// ---------------------------------------------------------------------------
// Pull internals.

/// The commit a pull merges from: the merge base of HEAD and `upstream`, None for an unborn
/// branch. Histories without a common commit are refused rather than merged.
fn pull_base(g: &Git, up: &Upstream, head: Option<&str>, upstream: &str) -> Result<Option<String>, GitError> {
    let Some(head) = head else { return Ok(None) };
    match merge_base(g, head, upstream)? {
        Some(base) => Ok(Some(base)),
        None => Err(GitError::Unrelated { branch: up.branch.clone() }),
    }
}

/// Questions and union-merged text files for bringing `upstream` into the current state
/// (commits and work tree), compared from `base` (the empty tree when None). Paths are
/// repo-root relative.
fn analyse(g: &Git, repo: &Repo, base: Option<&str>, upstream: &str) -> Result<(Vec<Question>, Vec<String>), GitError> {
    let work = work_tree(g, repo)?;
    let base = match base {
        Some(base) => base.to_string(),
        None => empty_tree(g)?,
    };
    let local = diff_trees(g, repo, &base, &work)?;
    let remote = diff_trees(g, repo, &base, upstream)?;
    // A file on one side and a folder of that name on the other cannot both exist; git's merge
    // would move one aside ("notes~HEAD") and a later deletion step could throw it away. Refuse
    // before anything changes. Compare the full trees: the colliding path need not have changed.
    let work_paths = tree_paths(g, repo, &work)?;
    let upstream_paths = tree_paths(g, repo, upstream)?;
    let mut collisions = path_collisions(&work_paths, &upstream_paths, repo);
    if let Some(head) = resolve(g, "HEAD")? {
        collisions.extend(path_collisions(&tree_paths(g, repo, &head)?, &upstream_paths, repo));
    }
    if !collisions.is_empty() {
        let mut paths: Vec<String> = collisions.into_iter().collect();
        paths.truncate(8);
        return Err(GitError::Other(tr!(
            "Datei- und Ordnernamen kollidieren zwischen Ordner und Repository: {files}. Benenne sie um und versuche es erneut.",
            files: paths.join(", ")
        )));
    }
    let mut questions = Vec::new();
    let mut text = Vec::new();
    for (path, l) in &local {
        let Some(r) = remote.get(path) else { continue };
        // Same result on both sides, or deleted here: the local deletion wins (like Typst).
        if l.new == r.new || l.status == 'D' {
            continue;
        }
        if r.status == 'D' {
            questions.push(Question::DeletedInRepo(path.clone()));
        } else if l.status == 'A' && r.status == 'A' {
            questions.push(Question::AddedBoth(path.clone()));
        } else if is_binary(g, &l.old_mode, &l.old)? || is_binary(g, &l.new_mode, &l.new)? || is_binary(g, &r.new_mode, &r.new)? {
            questions.push(Question::BinaryChanged(path.clone()));
        } else {
            text.push(path.clone());
        }
    }
    // The repository's own .gitattributes may pick another merge driver, which can conflict: ask about those.
    let unions = union_merged(g, repo, &text)?;
    let (merged, asked): (Vec<String>, Vec<String>) = text.into_iter().partition(|path| unions.contains(path));
    questions.extend(asked.into_iter().map(Question::BinaryChanged));
    questions.sort_by(|a, b| a.path().cmp(b.path()));
    Ok((questions, merged))
}

/// The tree of the work tree as `git add -A` would stage it, built in a scratch index so the
/// user's staging area stays as it is.
fn work_tree(g: &Git, repo: &Repo) -> Result<String, GitError> {
    let scratch = &repo.temp_index;
    let _ = std::fs::remove_file(scratch);
    // A copy of the real index lets git skip unchanged files; without one it reads them all.
    if repo.index.exists() && copy_index(&repo.index, scratch).is_err() {
        let _ = std::fs::remove_file(scratch);
    }
    let tree = g
        .cmd(&["add", "-A"])
        .in_dir(&repo.root)
        .env("GIT_INDEX_FILE", scratch)
        .run()
        .and_then(|_| g.cmd(&["write-tree"]).in_dir(&repo.root).env("GIT_INDEX_FILE", scratch).read());
    let _ = std::fs::remove_file(scratch);
    tree
}

/// Copies the index together with the time it was written. Git goes by that time: a file whose
/// size and times are as recorded counts as unchanged, unless it may have been written in the
/// second the index was, and then git reads the file. A copy with a time of its own (as on
/// Linux) is past that second, and an edit that keeps the file's size would go unseen.
fn copy_index(index: &Path, scratch: &Path) -> std::io::Result<()> {
    // Read first: should the index be rewritten meanwhile, the older time only makes git look
    // at more files.
    let written = std::fs::metadata(index)?.modified()?;
    std::fs::copy(index, scratch)?;
    std::fs::File::options().write(true).open(scratch)?.set_modified(written)
}

fn merge_base(g: &Git, a: &str, b: &str) -> Result<Option<String>, GitError> {
    let cmd = g.cmd(&["merge-base", a, b]);
    let out = cmd.output()?;
    match out.code {
        Some(0) => Ok(Some(out.text())),
        Some(1) => Ok(None),
        _ => Err(cmd.error(&out)),
    }
}

/// The empty tree in this repository's hash format.
fn empty_tree(g: &Git) -> Result<String, GitError> {
    g.cmd(&["hash-object", "-t", "tree", "--stdin"]).read()
}

/// A file's change between two trees (one record of `git diff-tree` raw output).
#[derive(Clone, Debug, PartialEq, Eq)]
struct Delta {
    status: char,
    old_mode: String,
    new_mode: String,
    old: String,
    new: String,
}

fn diff_trees(g: &Git, repo: &Repo, from: &str, to: &str) -> Result<BTreeMap<String, Delta>, GitError> {
    let out = g
        .cmd(&["diff-tree", "-r", "--no-renames", "-z", from, to])
        .in_dir(&repo.root)
        .run()?;
    Ok(parse_raw_diff(&out.stdout))
}

fn parse_raw_diff(raw: &[u8]) -> BTreeMap<String, Delta> {
    let text = String::from_utf8_lossy(raw);
    let mut records = text.split('\0');
    let mut deltas = BTreeMap::new();
    while let Some(record) = records.next() {
        let Some(meta) = record.strip_prefix(':') else { continue };
        let Some(path) = records.next() else { break };
        let fields: Vec<&str> = meta.split(' ').collect();
        if fields.len() < 5 {
            continue;
        }
        deltas.insert(
            path.to_string(),
            Delta {
                status: fields[4].chars().next().unwrap_or('M'),
                old_mode: fields[0].to_string(),
                new_mode: fields[1].to_string(),
                old: fields[2].to_string(),
                new: fields[3].to_string(),
            },
        );
    }
    deltas
}

/// Every file path in a tree (or commit), repo-root relative.
fn tree_paths(g: &Git, repo: &Repo, rev: &str) -> Result<BTreeSet<String>, GitError> {
    let out = g.cmd(&["ls-tree", "-r", "-z", "--name-only", rev]).in_dir(&repo.root).run()?;
    Ok(out.text().split('\0').filter(|p| !p.is_empty()).map(String::from).collect())
}

/// Names that cannot coexist on this file system: a file where the other side has a folder of
/// the same name, or two paths that differ only in case on a case-insensitive volume.
fn path_collisions(a: &BTreeSet<String>, b: &BTreeSet<String>, repo: &Repo) -> BTreeSet<String> {
    fn files(set: &BTreeSet<String>) -> HashSet<&str> {
        set.iter().map(String::as_str).collect()
    }
    fn dirs(set: &BTreeSet<String>) -> HashSet<&str> {
        set.iter().flat_map(|p| ancestors(p)).collect()
    }
    let (a_files, a_dirs, b_files, b_dirs) = (files(a), dirs(a), files(b), dirs(b));
    let mut out = BTreeSet::new();
    for p in a {
        if b_files.contains(p.as_str()) {
            continue;
        }
        if b_dirs.contains(p.as_str()) || ancestors(p).any(|x| b_files.contains(x)) {
            out.insert(p.clone());
        }
    }
    for p in b {
        if a_files.contains(p.as_str()) {
            continue;
        }
        if a_dirs.contains(p.as_str()) || ancestors(p).any(|x| a_files.contains(x)) {
            out.insert(p.clone());
        }
    }
    let case_insensitive = cfg!(target_os = "macos") || repo.config.get("core.ignorecase") == Some("true");
    if case_insensitive {
        let folded: HashMap<String, &String> = b.iter().map(|p| (p.to_lowercase(), p)).collect();
        for p in a {
            if let Some(other) = folded.get(&p.to_lowercase())
                && *other != p
            {
                out.insert(p.clone());
            }
        }
    }
    out
}

/// "a/b/c" → "a", "a/b".
fn ancestors(path: &str) -> impl Iterator<Item = &str> {
    path.match_indices('/').map(move |(i, _)| &path[..i])
}

fn is_zero(oid: &str) -> bool {
    oid.bytes().all(|b| b == b'0')
}

/// git's own test: a NUL byte in the first 8000 bytes. Links and submodules count as binary too.
fn is_binary(g: &Git, mode: &str, oid: &str) -> Result<bool, GitError> {
    if is_zero(oid) {
        return Ok(false);
    }
    if mode != "100644" && mode != "100755" {
        return Ok(true);
    }
    let out = g.cmd(&["cat-file", "blob", oid]).limit(8000).run()?;
    Ok(out.stdout.contains(&0))
}

/// Lets Typisch's own merges keep both sides of a text conflict (like Typst). The file lives in
/// the git directory and is passed per call, so the user's own merges are not affected. Their
/// global attributes are appended after the default, so narrower rules (`*.csv -merge`, binary,
/// filter, eol) keep precedence.
fn write_attributes(g: &Git, repo: &Repo) -> Result<(), GitError> {
    let mut text = String::from("* merge=union\n");
    if let Some(global) = global_attributes(g) {
        text.push_str(&global);
        if !text.ends_with('\n') {
            text.push('\n');
        }
    }
    std::fs::write(&repo.attributes, text).map_err(io_error)
}

/// The user's global attributes file, when there is one.
fn global_attributes(g: &Git) -> Option<String> {
    let configured = g
        .cmd(&["config", "--get", "core.attributesFile"])
        .output()
        .ok()
        .filter(Output::ok)
        .map(|o| PathBuf::from(o.text()));
    if let Some(path) = configured.filter(|p| p.is_file()) {
        return std::fs::read_to_string(path).ok();
    }
    let env = |key: &str| {
        g.env
            .iter()
            .find(|(k, _)| k.as_os_str() == OsStr::new(key))
            .map(|(_, v)| PathBuf::from(v))
            .or_else(|| std::env::var_os(key).map(PathBuf::from))
    };
    let home = env("HOME");
    let xdg = env("XDG_CONFIG_HOME").or_else(|| home.as_ref().map(|h| h.join(".config")));
    let candidates = xdg
        .into_iter()
        .map(|d| d.join("git/attributes"))
        .chain(home.map(|h| h.join(".gitattributes")));
    candidates.filter(|p| p.is_file()).find_map(|p| std::fs::read_to_string(p).ok())
}

/// The paths among `paths` that Typisch's merges would union-merge.
fn union_merged(g: &Git, repo: &Repo, paths: &[String]) -> Result<HashSet<String>, GitError> {
    if paths.is_empty() {
        return Ok(HashSet::new());
    }
    write_attributes(g, repo)?;
    let out = g
        .cmd(&["check-attr", "-z", "merge", "--"])
        .args(paths)
        .union(repo)
        .in_dir(&repo.root)
        .run();
    let _ = std::fs::remove_file(&repo.attributes);
    let text = String::from_utf8_lossy(&out?.stdout).into_owned();
    let fields: Vec<&str> = text.split('\0').collect();
    Ok(fields
        .chunks(3)
        .filter(|f| f.len() == 3 && f[2] == "union")
        .map(|f| f[0].to_string())
        .collect())
}

/// The snapshot of local work a pull keeps until everything is back in place.
struct Backup {
    sha: String,
    /// refs/typisch/backup/<unix seconds>
    name: String,
}

/// Commits the whole work tree (tracked and untracked files) as a stash-like commit without
/// touching the stash list, and keeps it reachable under refs/typisch/backup/. None when clean.
/// `before` names what the snapshot is taken for ("Pull").
fn snapshot(g: &Git, repo: &Repo, id: &Identity, before: &str) -> Result<Option<Backup>, GitError> {
    let dirty = g
        .cmd(&["status", "--porcelain", "-z", "--untracked-files=all"])
        .in_dir(&repo.root)
        .run()?;
    if dirty.stdout.is_empty() {
        return Ok(None);
    }
    g.cmd(&["add", "-A"]).in_dir(&repo.root).run()?;
    let backup = keep_snapshot(g, repo, id, before);
    if backup.is_err() {
        // Only the staging area changed so far: un-stage again, the files stay as they are.
        let _ = g.cmd(&["reset", "-q"]).in_dir(&repo.root).output();
    }
    backup
}

/// `git stash create` of the staged work tree, under a backup ref of its own.
fn keep_snapshot(g: &Git, repo: &Repo, id: &Identity, before: &str) -> Result<Option<Backup>, GitError> {
    let sha = g
        .cmd(&["stash", "create"])
        .arg(format!("Typisch: Sicherung vor {before}"))
        .in_dir(&repo.root)
        .identity(id)
        .read()?;
    if sha.is_empty() {
        return Ok(None);
    }
    keep_ref(g, repo, &sha).map(Some)
}

/// Keeps a commit reachable under a new backup ref.
fn keep_ref(g: &Git, repo: &Repo, sha: &str) -> Result<Backup, GitError> {
    let secs = now_secs();
    for n in 0..100 {
        let name = if n == 0 {
            format!("refs/typisch/backup/{secs}")
        } else {
            format!("refs/typisch/backup/{secs}-{n}")
        };
        // The empty old value: only create, never overwrite another backup.
        let cmd = g.cmd(&["update-ref", name.as_str(), sha, ""]).in_dir(&repo.root);
        let out = cmd.output()?;
        if out.ok() {
            return Ok(Backup {
                sha: sha.to_string(),
                name,
            });
        }
        if !out.stderr.contains("already exists") {
            return Err(cmd.error(&out));
        }
    }
    Err(GitError::Other(
        tr!("Die Sicherung der lokalen Änderungen ließ sich nicht anlegen.").into(),
    ))
}

/// The commits a pull into a branch with history works with.
struct Commits<'a> {
    /// HEAD before the pull.
    head: &'a str,
    /// The merge base of `head` and `upstream`.
    base: &'a str,
    upstream: &'a str,
}

/// Pull into a branch with history: snapshot local work, merge, put local work back on top.
/// If anything fails on the way, the folder is put back as it was.
fn merge_upstream(g: &Git, repo: &Repo, up: &Upstream, c: &Commits, choices: &[(String, Choice)], id: &Identity) -> Result<(), GitError> {
    write_attributes(g, repo)?;
    let result = snapshot(g, repo, id, "Pull").and_then(|backup| {
        if let Some(backup) = &backup {
            // If anything stops the pull after this point, the next start puts the work back.
            write_journal(repo, c.head, Some(backup))?;
        }
        match merge_steps(g, repo, up, c, choices, id, backup.as_ref()) {
            Ok(()) => {
                remove_journal(repo);
                if let Some(backup) = &backup {
                    let _ = g.cmd(&["update-ref", "-d", backup.name.as_str(), backup.sha.as_str()]).output();
                }
                Ok(())
            }
            // The backup ref stays: cheap insurance should the restored state ever be incomplete.
            Err(e) => Err(recover(g, repo, c.head, backup.as_ref(), e)),
        }
    });
    let _ = std::fs::remove_file(&repo.attributes);
    result
}

fn merge_steps(
    g: &Git,
    repo: &Repo,
    up: &Upstream,
    c: &Commits,
    choices: &[(String, Choice)],
    id: &Identity,
    backup: Option<&Backup>,
) -> Result<(), GitError> {
    if backup.is_some() {
        g.cmd(&["reset", "-q", "--hard", c.head]).in_dir(&repo.root).run()?;
    }
    merge_commits(g, repo, up, c, choices, id)?;
    if let Some(backup) = backup {
        reapply(g, repo, backup, c.upstream, choices)?;
    }
    Ok(())
}

/// Merges the upstream commits into HEAD: a fast-forward without local commits. A real merge
/// stops before committing, so that conflicts get the chosen side (Local = HEAD before the
/// pull), then it is committed.
fn merge_commits(g: &Git, repo: &Repo, up: &Upstream, c: &Commits, choices: &[(String, Choice)], id: &Identity) -> Result<(), GitError> {
    let message = format!("Merge branch '{}' of {}", up.branch, display_url(&up.url));
    // Ignored files are not in the snapshot, so git must not overwrite them either (it would by default).
    let args = [
        "merge",
        "--ff",
        "--no-commit",
        "--no-edit",
        "--no-stat",
        "--no-overwrite-ignore",
        "-m",
    ];
    let cmd = g
        .cmd(&args)
        .arg(&message)
        .arg(c.upstream)
        .union(repo)
        .in_dir(&repo.root)
        .identity(id);
    let out = cmd.output()?;
    let failed = || overwritten(&out.stderr).unwrap_or_else(|| cmd.error(&out));
    if !repo.merge_head.exists() {
        return if out.ok() { Ok(()) } else { Err(failed()) };
    }
    let unmerged = unmerged_paths(g, repo)?;
    if !out.ok() && unmerged.is_empty() {
        return Err(failed());
    }
    // An answer decides the merge commit only where the local commits changed the file (an
    // add/add text file would otherwise be committed union-merged). Answers about uncommitted
    // edits are settled when the snapshot is put back; here they would revert the remote change.
    let committed = diff_trees(g, repo, c.base, c.head)?;
    let answered = choices
        .iter()
        .map(|(path, _)| path)
        .filter(|path| committed.contains_key(*path))
        .cloned();
    let paths: BTreeSet<String> = unmerged.into_iter().chain(answered).collect();
    settle(g, repo, &paths, choices, c.head, c.upstream)?;
    g.cmd(&["commit", "-q", "--no-edit"]).in_dir(&repo.root).identity(id).run()?;
    Ok(())
}

/// git refusing to overwrite files that are not part of the snapshot (ignored ones), in German.
fn overwritten(stderr: &str) -> Option<GitError> {
    if !stderr.contains("would be overwritten by merge") {
        return None;
    }
    let files: Vec<&str> = stderr.lines().filter(|line| line.starts_with('\t')).map(str::trim).collect();
    Some(GitError::Other(tr!(
        "Der Pull würde diese Dateien überschreiben: {files}. Verschiebe oder lösche sie zuerst.",
        files: files.join(", ")
    )))
}

/// Puts the snapshot of local work back on top of the merged state. Conflicts and answered
/// paths get the chosen side (Local = the snapshot), then everything is unstaged again.
fn reapply(g: &Git, repo: &Repo, backup: &Backup, upstream: &str, choices: &[(String, Choice)]) -> Result<(), GitError> {
    let cmd = g.cmd(&["stash", "apply", backup.sha.as_str()]).union(repo).in_dir(&repo.root);
    let out = cmd.output()?;
    let unmerged = unmerged_paths(g, repo)?;
    // Exit code 1 with conflicts is expected; without any, something else went wrong.
    if !out.ok() && unmerged.is_empty() {
        return Err(cmd.error(&out));
    }
    // ("ours" and "theirs" are swapped in a stash apply, so the sides are named by commit.)
    let paths: BTreeSet<String> = unmerged.into_iter().chain(choices.iter().map(|(path, _)| path.clone())).collect();
    settle(g, repo, &paths, choices, &backup.sha, upstream)?;
    g.cmd(&["reset", "-q"]).in_dir(&repo.root).run()?;
    Ok(())
}

/// Gives each of `paths` the chosen side's version (commit `local` or `remote`); a side without
/// the file deletes it. Unanswered paths keep the local side.
fn settle(g: &Git, repo: &Repo, paths: &BTreeSet<String>, choices: &[(String, Choice)], local: &str, remote: &str) -> Result<(), GitError> {
    let chosen: HashMap<&str, Choice> = choices.iter().map(|(path, choice)| (path.as_str(), *choice)).collect();
    for path in paths {
        // git's merge moves a colliding path aside as "notes~HEAD" or "notes~Stashed changes".
        // Such a name exists on neither side; deleting it would be deleting the only copy.
        if let Some((_, label)) = path.rsplit_once('~')
            && (label == "HEAD" || label.starts_with("Stashed changes"))
        {
            return Err(GitError::Other(tr!(
                "Datei- und Ordnernamen kollidieren ({path}). Benenne einen davon um und versuche es erneut.",
                path: path
            )));
        }
        let side = if chosen.get(path.as_str()) == Some(&Choice::Remote) {
            remote
        } else {
            local
        };
        put_version(g, repo, side, path)?;
    }
    Ok(())
}

/// Writes `path` as it is in commit `rev` to index and work tree (keeping its mode), or deletes
/// it when `rev` has no such file.
fn put_version(g: &Git, repo: &Repo, rev: &str, path: &str) -> Result<(), GitError> {
    let present = g
        .cmd(&["cat-file", "-e"])
        .arg(format!("{rev}:{path}"))
        .in_dir(&repo.root)
        .output()?
        .ok();
    if present {
        g.cmd(&["checkout", rev, "--"]).arg(exact(path)).in_dir(&repo.root).run()?;
        return Ok(());
    }
    g.cmd(&["rm", "-q", "-f", "--cached", "--ignore-unmatch", "--"])
        .arg(exact(path))
        .in_dir(&repo.root)
        .run()?;
    let file = repo.root.join(path);
    if std::fs::symlink_metadata(&file).is_ok_and(|m| !m.is_dir()) {
        // On a case-insensitive volume, remove_file("Kapitel.typ") also unlinks "kapitel.typ",
        // the file the repository still has. Only delete the exact name.
        let (Some(parent), Some(name)) = (file.parent(), file.file_name().map(|n| n.to_os_string())) else {
            return Ok(());
        };
        let mut exact_on_disk = false;
        let mut other_case = false;
        for entry in std::fs::read_dir(parent).map_err(io_error)?.flatten() {
            let found = entry.file_name();
            if found == name {
                exact_on_disk = true;
                break;
            }
            if found.to_string_lossy().eq_ignore_ascii_case(&name.to_string_lossy()) {
                other_case = true;
            }
        }
        if exact_on_disk {
            std::fs::remove_file(&file).map_err(io_error)?;
        } else if other_case {
            return Err(GitError::Other(tr!(
                "Die Datei {path} gibt es hier nur in anderer Groß-/Kleinschreibung. Benenne sie um und versuche es erneut.",
                path: path
            )));
        }
    }
    Ok(())
}

/// Puts the folder back as it was before the pull: HEAD at `head`, local work from the snapshot.
/// Returns the error to report; it names the backup when even that failed.
fn recover(g: &Git, repo: &Repo, head: &str, backup: Option<&Backup>, error: GitError) -> GitError {
    match restore_pre_pull(g, repo, head, backup) {
        Ok(()) => {
            remove_journal(repo);
            error
        }
        Err(restore) => match backup {
            Some(backup) => GitError::Other(tr!(
                "{error}\nDeine lokalen Änderungen sind gesichert: git stash apply {name} (Wiederherstellen: {restore})",
                error: error, name: backup.name, restore: restore
            )),
            None => GitError::Other(format!("{error}\n{restore}")),
        },
    }
}

/// The work-tree part of a pull rollback, also used when the next start finds an unfinished pull.
fn restore_pre_pull(g: &Git, repo: &Repo, head: &str, backup: Option<&Backup>) -> Result<(), GitError> {
    if repo.merge_head.exists() {
        let _ = g.cmd(&["merge", "--abort"]).in_dir(&repo.root).output();
    }
    g.cmd(&["reset", "-q", "--hard", head]).in_dir(&repo.root).run()?;
    if let Some(backup) = backup {
        let applied = g.cmd(&["stash", "apply", backup.sha.as_str()]).in_dir(&repo.root).output()?;
        if !applied.ok() {
            // Something half-written is in the way (an untracked file, say): take the
            // snapshot's files exactly as they were, overwriting such leftovers.
            g.cmd(&["reset", "-q", "--hard", head]).in_dir(&repo.root).run()?;
            g.cmd(&["read-tree", "--reset", "-u", backup.sha.as_str()])
                .in_dir(&repo.root)
                .run()?;
        }
        g.cmd(&["reset", "-q"]).in_dir(&repo.root).run()?;
    }
    Ok(())
}

/// What a pull or reset recorded before it reset the work tree.
struct Journal {
    head: String,
    backup: Option<Backup>,
}

fn write_journal(repo: &Repo, head: &str, backup: Option<&Backup>) -> Result<(), GitError> {
    let (name, sha) = backup.map_or(("-", "-"), |b| (b.name.as_str(), b.sha.as_str()));
    std::fs::write(&repo.journal, format!("{head}\n{name}\n{sha}\n")).map_err(io_error)
}

fn read_journal(repo: &Repo) -> Option<Journal> {
    let text = std::fs::read_to_string(&repo.journal).ok()?;
    let mut lines = text.lines();
    let head = lines.next()?.trim().to_string();
    if head.is_empty() {
        return None;
    }
    let (name, sha) = (lines.next().unwrap_or("-").trim(), lines.next().unwrap_or("-").trim());
    let backup = (name != "-" && sha != "-").then(|| Backup {
        name: name.to_string(),
        sha: sha.to_string(),
    });
    Some(Journal { head, backup })
}

fn remove_journal(repo: &Repo) {
    let _ = std::fs::remove_file(&repo.journal);
}

/// A pull that was interrupted (quit, crash, killed git) leaves its journal behind. Put the local
/// work back before showing a status, so the next start does not look like a clean folder.
fn finish_interrupted_pull(g: &Git, repo: &Repo) -> Result<(), GitError> {
    let Some(journal) = read_journal(repo) else {
        // An empty or unreadable journal would otherwise sit there forever.
        if repo.journal.exists() {
            remove_journal(repo);
        }
        return Ok(());
    };
    restore_pre_pull(g, repo, &journal.head, journal.backup.as_ref()).map_err(|e| {
        let hint = journal
            .backup
            .as_ref()
            .map(|b| tr!("\nDeine lokalen Änderungen sind gesichert: git stash apply {name}", name: b.name))
            .unwrap_or_default();
        GitError::Other(tr!("Ein unterbrochener Pull ließ sich nicht wiederherstellen: {e}{hint}", e: e, hint: hint))
    })?;
    remove_journal(repo);
    Ok(())
}

/// Makes `upstream` the first commit of an unborn branch: HEAD = upstream, and the local files
/// stay as uncommitted changes. Only the `overwrite` paths get the remote version; files the
/// remote has and the folder lacks are created; nothing else in the folder is replaced.
fn adopt(g: &Git, repo: &Repo, upstream: &str, overwrite: &[String]) -> Result<(), GitError> {
    // A remote file that cannot be created would later count as deleted, and a push would
    // delete it in the repository: refuse before changing anything.
    let mut blocked = blocked_paths(g, repo, upstream)?;
    if !blocked.is_empty() {
        blocked.truncate(8);
        return Err(GitError::Overlap { paths: blocked });
    }
    // The empty old value: only while the branch really has no commit.
    g.cmd(&["update-ref", "HEAD", upstream, ""]).in_dir(&repo.root).run()?;
    let result = (|| -> Result<(), GitError> {
        g.cmd(&["read-tree", upstream]).in_dir(&repo.root).run()?;
        for path in overwrite {
            g.cmd(&["checkout", upstream, "--"]).arg(exact(path)).in_dir(&repo.root).run()?;
        }
        // Creates the files the folder lacks. With -q, files that already exist are skipped
        // silently (exit code 1), which is the point: those stay as they are. Anything it says
        // is a real failure.
        let out = g.cmd(&["checkout-index", "-a", "-q"]).in_dir(&repo.root).output()?;
        let said = out
            .stderr
            .lines()
            .any(|line| !line.trim().is_empty() && !line.starts_with("warning:"));
        if said || !matches!(out.code, Some(0 | 1)) {
            return Err(GitError::Other(tail(&out.stderr, out.code)));
        }
        g.cmd(&["reset", "-q"]).in_dir(&repo.root).run()?;
        Ok(())
    })();
    if result.is_err() {
        // Back to an unborn branch; the files in the folder were not touched (apart from `overwrite`).
        let _ = g.cmd(&["update-ref", "-d", "HEAD", upstream]).in_dir(&repo.root).output();
        let _ = g.cmd(&["read-tree", "--empty"]).in_dir(&repo.root).output();
    }
    result
}

/// Files of commit `upstream` that cannot be created in the folder because something else is in
/// the way: a folder where the file belongs, or a file (or link) where it needs a folder.
fn blocked_paths(g: &Git, repo: &Repo, upstream: &str) -> Result<Vec<String>, GitError> {
    let out = g.cmd(&["ls-tree", "-r", "-z", upstream]).in_dir(&repo.root).run()?;
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    let not_a_folder = |path: &Path| std::fs::symlink_metadata(path).is_ok_and(|m| !m.is_dir());
    let blocked = text
        .split('\0')
        .filter_map(|entry| entry.split_once('\t'))
        // Submodules are folders by nature.
        .filter(|(meta, _)| !meta.starts_with("160000"))
        .map(|(_, path)| path)
        .filter(|path| {
            let file = repo.root.join(path);
            let folder_in_the_way = std::fs::symlink_metadata(&file).is_ok_and(|m| m.is_dir());
            let mut parents = Path::new(path).ancestors().skip(1).filter(|p| !p.as_os_str().is_empty());
            folder_in_the_way || parents.any(|p| not_a_folder(&repo.root.join(p)))
        })
        .map(String::from)
        .collect();
    Ok(blocked)
}

// ---------------------------------------------------------------------------
// Reset internals.

/// Uncommitted changes outside the project folder, repo-root relative (none when the project is
/// the whole repository).
fn changes_outside(g: &Git, repo: &Repo) -> Result<Vec<String>, GitError> {
    if repo.prefix.is_empty() {
        return Ok(Vec::new());
    }
    let raw = g
        .cmd(&["status", "--porcelain=v2", "-z", "--untracked-files=all"])
        .in_dir(&repo.root)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .run()?
        .stdout;
    let entries = parse_porcelain(&raw).entries.into_iter();
    Ok(entries
        .map(|(path, _)| path)
        .filter(|path| !path.starts_with(&repo.prefix))
        .collect())
}

/// Ignored files are in no snapshot, yet `reset --hard` overwrites those that `upstream` has as
/// well. Staging them puts them into the snapshot that follows.
fn stage_ignored_in_the_way(g: &Git, repo: &Repo, upstream: &str) -> Result<(), GitError> {
    let ignored = g
        .cmd(&["ls-files", "-z", "--others", "--ignored", "--exclude-standard"])
        .in_dir(&repo.root)
        .run()?
        .text();
    let theirs = tree_paths(g, repo, upstream)?;
    let hit: Vec<String> = ignored.split('\0').filter(|path| theirs.contains(*path)).map(exact).collect();
    if !hit.is_empty() {
        g.cmd(&["add", "-f", "--"]).args(hit).in_dir(&repo.root).run()?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Push internals.

/// Commits the changes inside the project folder (only there; changes staged elsewhere stay
/// staged). Returns the new commit, or None when there was nothing to commit.
fn commit_project(g: &Git, st: &RepoStatus, message: &str, id: &Identity) -> Result<Option<String>, GitError> {
    if st.changes.is_empty() {
        return Ok(None);
    }
    g.cmd(&["add", "-A", "--", "."]).run()?;
    // Edited and changed back: nothing left to commit.
    if g.cmd(&["diff", "--cached", "--quiet", "--", "."]).output()?.ok() {
        return Ok(None);
    }
    let message = message.trim();
    let message = if message.is_empty() {
        tr!("Änderungen aus Typisch")
    } else {
        message
    };
    g.cmd(&["commit", "-q", "-m", message, "--", "."]).identity(id).run()?;
    must_resolve(g, "HEAD").map(Some)
}

/// Pushes `head` to the remote branch (fast-forward only). Returns the number of commits that went up.
fn push_head(g: &Git, up: &Upstream, head: &str, upstream: Option<&str>) -> Result<usize, GitError> {
    let ahead = match upstream {
        Some(upstream) => count(g, &format!("{upstream}..{head}"))?,
        None => count(g, head)?,
    };
    if ahead == 0 {
        return Ok(0);
    }
    let refspec = format!("{head}:refs/heads/{}", up.branch);
    let cmd = g.cmd(&["push", "--porcelain", "--"]).arg(&up.remote).arg(refspec).network(&up.url);
    let out = cmd.output()?;
    let stdout = String::from_utf8_lossy(&out.stdout);
    // `--porcelain` marks every refused ref with "!"; do not rely on the exit code alone.
    if !out.ok() || stdout.lines().any(|line| line.starts_with("!\t")) {
        let said = format!("{stdout}\n{}", out.stderr).to_lowercase();
        return Err(if push_rejected(&said) {
            GitError::NeedsPull { behind: 0 }
        } else {
            cmd.error(&out)
        });
    }
    // The push went through; whatever follows must not undo it.
    let _ = g.cmd(&["update-ref"]).arg(up.tracking()).arg(head).output();
    let _ = g.cmd(&["config", "typisch.lastPush"]).arg(now_secs().to_string()).output();
    Ok(ahead)
}

/// The remote refused because it has commits we lack: a plain non-fast-forward, or a push that
/// lost the race against another one (the branch moved, or was created, between our check and
/// the update).
fn push_rejected(said: &str) -> bool {
    const HINTS: [&str; 6] = [
        "[rejected]",
        "non-fast-forward",
        "fetch first",
        "incorrect old value",
        "but expected",
        "reference already exists",
    ];
    HINTS.iter().any(|hint| said.contains(hint))
}

/// Takes back the commit a failed push made, so its changes are uncommitted again. Only when
/// HEAD is still that commit (the old values make the ref updates conditional).
fn undo_commit(g: &Git, created: &str, parent: Option<&str>) {
    match parent {
        Some(parent) => {
            let _ = g
                .cmd(&[
                    "update-ref",
                    "-m",
                    "Typisch: Commit nach fehlgeschlagenem Push zurückgenommen",
                    "HEAD",
                    parent,
                    created,
                ])
                .output();
        }
        None => {
            // The first commit: back to an unborn branch, and the files untracked as before.
            if g.cmd(&["update-ref", "-d", "HEAD", created]).output().is_ok_and(|o| o.ok()) {
                let _ = g.cmd(&["rm", "-r", "-q", "-f", "--cached", "--ignore-unmatch", "--", "."]).output();
            }
        }
    }
}

/// After a rejected push: how many commits the remote is ahead (at least one, or it would not have refused).
fn behind_after_rejection(g: &Git, up: &Upstream) -> usize {
    let counted = || -> Result<usize, GitError> {
        if !fetch_upstream(g, up)? {
            return Ok(0);
        }
        let upstream = must_resolve(g, &up.tracking())?;
        let head = resolve(g, "HEAD")?;
        commits_between(g, head.as_deref(), &upstream)
    };
    counted().unwrap_or(0).max(1)
}

// ---------------------------------------------------------------------------
// Link and clone internals.

/// What a failed `link` takes back: only what that call created.
#[derive(Default)]
struct Created {
    git_dir: Option<PathBuf>,
    gitignore: Option<PathBuf>,
    remote: Option<String>,
}

impl Created {
    fn roll_back(&self, g: &Git) {
        if let Some(dir) = &self.git_dir {
            let _ = std::fs::remove_dir_all(dir);
        } else if let Some(remote) = &self.remote {
            let _ = g.cmd(&["remote", "remove", "--"]).arg(remote).output();
        }
        if let Some(file) = &self.gitignore {
            let _ = std::fs::remove_file(file);
        }
    }
}

fn link_inner(g: &Git, url: &str, branch: &str, created: &mut Created) -> Result<(), GitError> {
    let (repo, st) = match load(g)? {
        Some(found) => found,
        None => {
            let dot_git = g.dir.join(".git");
            if std::fs::symlink_metadata(&dot_git).is_ok() {
                return Err(GitError::Other(tr!("Der Ordner enthält einen beschädigten .git-Ordner.").into()));
            }
            g.cmd(&["init", "-q"]).arg(format!("--initial-branch={branch}")).run()?;
            created.git_dir = Some(dot_git);
            let ignore = g.dir.join(".gitignore");
            if !ignore.exists() {
                std::fs::write(&ignore, ".DS_Store\n").map_err(io_error)?;
                created.gitignore = Some(ignore);
            }
            load(g)?.ok_or(GitError::NotARepo)?
        }
    };
    let local = st.branch.clone().ok_or(GitError::Detached)?;
    // A project folder inside another repository (a dotfiles repo in the home folder, say):
    // linking it would rewire the outer repository and push its whole history.
    if !repo.prefix.is_empty() {
        return Err(GitError::Other(tr!(
            "Der Projektordner liegt innerhalb des Git-Repositories {path}. Öffne diesen Ordner als Projekt oder klone das Repository als neues Projekt.",
            path: repo.root.display()
        )));
    }
    if let Some(op) = &st.in_progress {
        return Err(GitError::InProgress(op.clone()));
    }
    let (remote, new) = pick_remote(&repo.config, url);
    if new {
        g.cmd(&["remote", "add", "--"]).arg(&remote).arg(url).run()?;
        created.remote = Some(remote.clone());
    }
    let up = Upstream {
        remote,
        branch: branch.to_string(),
        url: url.to_string(),
    };
    if fetch_upstream(g, &up)? {
        let upstream = must_resolve(g, &up.tracking())?;
        if !st.unborn {
            if merge_base(g, "HEAD", &upstream)?.is_none() {
                return Err(GitError::Unrelated {
                    branch: branch.to_string(),
                });
            }
        } else {
            // Our fresh .gitignore must not collide with the repository's own: take theirs.
            if let Some(ignore) = &created.gitignore {
                let theirs = format!("{upstream}:{}.gitignore", repo.prefix);
                if g.cmd(&["cat-file", "-e"]).arg(theirs).output()?.ok() {
                    std::fs::remove_file(ignore).map_err(io_error)?;
                    created.gitignore = None;
                }
            }
            let work = work_tree(g, &repo)?;
            let mut overlap: Vec<String> = diff_trees(g, &repo, &work, &upstream)?
                .into_iter()
                .filter(|(_, d)| matches!(d.status, 'M' | 'T'))
                .map(|(path, _)| path)
                .collect();
            if !overlap.is_empty() {
                overlap.truncate(8);
                return Err(GitError::Overlap { paths: overlap });
            }
            adopt(g, &repo, &upstream, &[])?;
        }
    }
    set_upstream(g, &local, &up.remote, branch)
}

/// A name git accepts for a new branch (asked in `dir`, an existing folder).
fn check_branch(g: &Git, dir: &Path, branch: &str) -> Result<(), GitError> {
    let checked = g.cmd(&["check-ref-format", "--branch", branch]).in_dir(dir).output()?;
    if branch.starts_with('-') || !checked.ok() || checked.text() != branch {
        return Err(GitError::Other(tr!("„{branch}“ ist kein gültiger Branch-Name.", branch: branch)));
    }
    Ok(())
}

/// Reuses a remote that already points at `url`; otherwise the first free name of origin,
/// typisch, typisch2, … The URL of an existing remote is never changed. True = to be added.
fn pick_remote(config: &Config, url: &str) -> (String, bool) {
    if let Some((name, _)) = config.remotes().into_iter().find(|(_, u)| *u == url) {
        return (name.to_string(), false);
    }
    let mut names = ["origin".to_string(), "typisch".to_string()]
        .into_iter()
        .chain((2..).map(|n| format!("typisch{n}")));
    let name = names
        .find(|name| !config.has_remote_section(name))
        .unwrap_or_else(|| "typisch".into());
    (name, true)
}

/// Written directly, so it also works while the remote branch does not exist yet.
fn set_upstream(g: &Git, local: &str, remote: &str, branch: &str) -> Result<(), GitError> {
    g.cmd(&["config", "--"]).arg(format!("branch.{local}.remote")).arg(remote).run()?;
    g.cmd(&["config", "--"])
        .arg(format!("branch.{local}.merge"))
        .arg(format!("refs/heads/{branch}"))
        .run()?;
    Ok(())
}

fn clone_into(g: &Git, url: &str, branch: Option<&str>, parent: &Path, name: &OsStr, dest: &Path) -> Result<(), GitError> {
    let clone = |branch: Option<&str>| {
        let mut cmd = g.cmd(&["clone", "--origin", "origin"]).in_dir(parent).network(url);
        if let Some(branch) = branch {
            cmd = cmd.arg(format!("--branch={branch}"));
        }
        cmd.arg("--").arg(url).arg(name)
    };
    let cmd = clone(branch);
    let out = cmd.output()?;
    if !out.ok() {
        let Some(wanted) = branch.filter(|_| out.stderr.contains("not found in upstream")) else {
            return Err(cmd.error(&out));
        };
        // An empty repository has no branch to check out yet: clone it without one and start that branch.
        let heads = g.cmd(&["ls-remote", "--heads", "--", url]).in_dir(parent).network(url).read()?;
        if !heads.trim().is_empty() {
            return Err(GitError::Other(
                tr!("Den Branch „{wanted}“ gibt es im Repository nicht.", wanted: wanted),
            ));
        }
        let retry = clone(None);
        let out = retry.output()?;
        if !out.ok() {
            return Err(retry.error(&out));
        }
    }
    let target = g.at(dest);
    if resolve(&target, "HEAD")?.is_none() {
        // An empty repository: link the unborn branch to the remote branch its first push creates.
        let local = match branch {
            Some(branch) => {
                target.cmd(&["symbolic-ref", "HEAD"]).arg(format!("refs/heads/{branch}")).run()?;
                branch.to_string()
            }
            None => target.cmd(&["symbolic-ref", "--short", "HEAD"]).read()?,
        };
        set_upstream(&target, &local, "origin", &local)?;
    }
    Ok(())
}

/// Removes what a failed clone left: the folder if this call created it, else its contents
/// (it was empty before).
fn remove_clone(dest: &Path, created: bool) {
    if created {
        let _ = std::fs::remove_dir_all(dest);
        return;
    }
    let Ok(entries) = std::fs::read_dir(dest) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        let _ = if path.is_dir() && !path.is_symlink() {
            std::fs::remove_dir_all(&path)
        } else {
            std::fs::remove_file(&path)
        };
    }
}

// ---------------------------------------------------------------------------
// Addresses and times for display.

/// A remote address taken apart: `https://user@host:8443/a/b.git`, `git@host:a/b.git`,
/// `ssh://git@host:22/a/b`, or a local path.
struct Address<'a> {
    /// "https", "ssh", "file", …; "ssh" for scp-like addresses, "" for local paths.
    scheme: &'a str,
    host: &'a str,
    port: &'a str,
    path: &'a str,
}

fn address(url: &str) -> Address<'_> {
    let url = url.trim();
    if let Some((scheme, rest)) = url.split_once("://") {
        let (authority, path) = rest.find('/').map_or((rest, ""), |i| rest.split_at(i));
        let host_port = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
        let (host, port) = split_port(host_port);
        return Address { scheme, host, port, path };
    }
    if let Some((user_host, path)) = scp_parts(url) {
        let host = user_host.rsplit_once('@').map_or(user_host, |(_, h)| h);
        return Address {
            scheme: "ssh",
            host: host.trim_start_matches('[').trim_end_matches(']'),
            port: "",
            path,
        };
    }
    Address {
        scheme: "",
        host: "",
        port: "",
        path: url,
    }
}

/// git's scp-like syntax `[user@]host:path`: a colon before any slash (and not a drive letter).
fn scp_parts(url: &str) -> Option<(&str, &str)> {
    let colon = url.find(':')?;
    let before = &url[..colon];
    let drive = before.len() == 1 && before.as_bytes()[0].is_ascii_alphabetic();
    if before.is_empty() || before.contains('/') || before.contains('\\') || drive {
        return None;
    }
    Some((before, &url[colon + 1..]))
}

fn split_port(host_port: &str) -> (&str, &str) {
    if let Some((host, after)) = host_port.strip_prefix('[').and_then(|rest| rest.split_once(']')) {
        return (host, after.strip_prefix(':').unwrap_or(""));
    }
    match host_port.rsplit_once(':') {
        Some((host, port)) if port.bytes().all(|b| b.is_ascii_digit()) => (host, port),
        _ => (host_port, ""),
    }
}

/// "/a/b.git/" → "a/b".
fn clean_repo_path(path: &str) -> &str {
    let path = path.trim_matches('/');
    path.strip_suffix(".git").unwrap_or(path).trim_end_matches('/')
}

/// Hosts whose web pages live at https://host/<path> even when git talks to them over ssh.
const WEB_HOSTS: [&str; 4] = ["github.com", "gitlab.com", "bitbucket.org", "codeberg.org"];

/// "github.com" for https://github.com/a/b.git, git@github.com:a/b.git and ssh://git@github.com:22/a/b;
/// None for local paths.
pub fn host_of(url: &str) -> Option<String> {
    let a = address(url);
    (!a.host.is_empty() && a.scheme != "file").then(|| a.host.to_lowercase())
}

pub fn is_ssh_url(url: &str) -> bool {
    matches!(address(url).scheme, "ssh" | "git+ssh" | "ssh+git")
}

/// A page to open in the browser: https(s) addresses as they are (without .git and login), ssh
/// addresses of the big hosts as their https page; None for local paths and other ssh hosts.
pub fn web_url(url: &str) -> Option<String> {
    let a = address(url);
    let path = clean_repo_path(a.path);
    if a.host.is_empty() || path.is_empty() {
        return None;
    }
    let host = a.host.to_lowercase();
    match a.scheme {
        "https" | "http" => {
            let port = if a.port.is_empty() { String::new() } else { format!(":{}", a.port) };
            Some(format!("{}://{host}{port}/{path}", a.scheme))
        }
        "ssh" | "git+ssh" | "ssh+git" if WEB_HOSTS.contains(&host.as_str()) => Some(format!("https://{host}/{path}")),
        _ => None,
    }
}

/// "github.com/a/b": no scheme, no login, no ".git". Local paths are shown as they are.
pub fn display_url(url: &str) -> String {
    let a = address(url);
    if a.host.is_empty() {
        let path = a.path.trim_end_matches('/');
        return if path.is_empty() {
            url.trim().to_string()
        } else {
            path.to_string()
        };
    }
    let host = a.host.to_lowercase();
    let port = if matches!(a.scheme, "http" | "https") && !a.port.is_empty() {
        format!(":{}", a.port)
    } else {
        String::new()
    };
    let path = clean_repo_path(a.path);
    if path.is_empty() {
        format!("{host}{port}")
    } else {
        format!("{host}{port}/{path}")
    }
}

/// The folder name for a clone: the last path segment without ".git"; "Projekt" if there is none.
pub fn repo_name(url: &str) -> String {
    let a = address(url);
    let path = a.path.trim_end_matches(['/', '\\']);
    let last = path.rsplit(['/', '\\', ':']).next().unwrap_or("");
    let name = last.strip_suffix(".git").unwrap_or(last).trim();
    if name.is_empty() || name == "." || name == ".." {
        tr!("Projekt").into()
    } else {
        name.into()
    }
}

/// The page where the user creates an access token, for the hosts that have one we know.
pub fn token_url(host: &str) -> Option<&'static str> {
    match host {
        "github.com" => Some("https://github.com/settings/tokens/new?scopes=repo&description=Typisch"),
        "gitlab.com" => Some("https://gitlab.com/-/user_settings/personal_access_tokens?name=Typisch&scopes=write_repository"),
        _ => None,
    }
}

/// Short age for badges: "jetzt", "5 min", "3 h", "2 T", "6 W" (weeks from 14 days on).
pub fn relative_time(secs_ago: u64) -> String {
    const MIN: u64 = 60;
    const HOUR: u64 = 60 * MIN;
    const DAY: u64 = 24 * HOUR;
    match secs_ago {
        s if s < MIN => tr!("jetzt").into(),
        s if s < HOUR => format!("{} min", s / MIN),
        s if s < DAY => format!("{} h", s / HOUR),
        s if s < 14 * DAY => tr!("{n} T", n: s / DAY),
        s => tr!("{n} W", n: s / (7 * DAY)),
    }
}

/// Pure-Rust walk up from `dir` to the folder holding `.git` (a directory, or a file in worktrees
/// and submodules). Runs no git, so it is safe to call before the user asked for anything.
pub fn find_repo_root(dir: &Path) -> Option<PathBuf> {
    dir.ancestors().find(|d| d.join(".git").exists()).map(Path::to_path_buf)
}

// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use Change::{Added, Conflicted, Deleted, Modified, Renamed};
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Who every test commit is made by.
    fn me() -> Identity {
        Identity {
            name: "Erika Mustermann".into(),
            email: "erika@example.com".into(),
        }
    }

    /// One test's own folder, removed afterwards. Git runs in it isolated from the user's
    /// configuration, per command (tests run in parallel, so the process environment stays as it is).
    struct Sandbox {
        root: PathBuf,
    }

    impl Sandbox {
        fn new(name: &str) -> Self {
            static NEXT: AtomicUsize = AtomicUsize::new(0);
            let n = NEXT.fetch_add(1, Ordering::Relaxed);
            let dir = std::env::temp_dir().join(format!("typisch-git-{name}-{}-{n}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            // git reports real paths (/private/var/… on macOS); compare with those.
            let root = dir.canonicalize().unwrap();
            std::fs::write(root.join("gitconfig"), "").unwrap();
            Self { root }
        }

        fn path(&self, rel: &str) -> PathBuf {
            self.root.join(rel)
        }

        fn git(&self, dir: &Path) -> Git {
            Git::new(dir).isolated(&self.root)
        }

        /// Runs git to set up or inspect a scene, whatever its exit code.
        fn output(&self, dir: &Path, args: &[&str]) -> Output {
            self.git(dir).cmd(args).identity(&me()).output().unwrap()
        }

        /// Runs git to set up or inspect a scene; it must succeed.
        fn run(&self, dir: &Path, args: &[&str]) -> String {
            let out = self.output(dir, args);
            assert!(out.ok(), "git {args:?} in {}: {}", dir.display(), out.stderr);
            out.text()
        }

        /// A new empty bare repository whose unborn branch is main; returns its file:// URL.
        fn bare(&self, name: &str) -> String {
            let path = self.path(name);
            self.run(&self.root, &["init", "-q", "--bare", "-b", "main", path.to_str().unwrap()]);
            format!("file://{}", path.display())
        }

        fn clone(&self, url: &str, name: &str) -> PathBuf {
            self.run(&self.root, &["clone", "-q", "--", url, name]);
            self.path(name)
        }

        /// Commits everything in the repository at `dir`; returns the new HEAD.
        fn commit(&self, dir: &Path, message: &str) -> String {
            self.run(dir, &["add", "-A"]);
            self.run(dir, &["commit", "-q", "-m", message]);
            self.head(dir)
        }

        /// Commits everything and pushes it to the remote's main branch.
        fn commit_push(&self, dir: &Path, message: &str) -> String {
            let head = self.commit(dir, message);
            self.run(dir, &["push", "-q", "origin", "HEAD:refs/heads/main"]);
            head
        }

        fn head(&self, dir: &Path) -> String {
            self.run(dir, &["rev-parse", "HEAD"])
        }

        /// A file's bytes in a commit.
        fn blob(&self, dir: &Path, rev_path: &str) -> Vec<u8> {
            let out = self.output(dir, &["cat-file", "blob", rev_path]);
            assert!(out.ok(), "{rev_path}: {}", out.stderr);
            out.stdout
        }

        fn status(&self, dir: &Path) -> RepoStatus {
            status(&self.git(dir)).unwrap().expect("a git work tree")
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

    fn read(path: &Path) -> Option<String> {
        std::fs::read_to_string(path).ok()
    }

    fn remove(path: &Path) {
        std::fs::remove_file(path).unwrap();
    }

    /// The changes of a status as sorted (path, change) pairs.
    fn changes(st: &RepoStatus) -> Vec<(String, Change)> {
        let mut list: Vec<(String, Change)> = st.changes.iter().map(|c| (c.path.clone(), c.change)).collect();
        list.sort_by(|a, b| a.0.cmp(&b.0));
        list
    }

    fn expect(list: &[(&str, Change)]) -> Vec<(String, Change)> {
        list.iter().map(|(path, change)| (path.to_string(), *change)).collect()
    }

    fn strings(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    /// A remote whose main branch holds `files` in one commit; returns its URL and that commit.
    fn seeded(name: &str, files: &[(&str, &[u8])]) -> (Sandbox, String, String) {
        let sb = Sandbox::new(name);
        let url = sb.bare("remote.git");
        sb.run(&sb.root, &["init", "-q", "-b", "main", "seed"]);
        let seed = sb.path("seed");
        sb.run(&seed, &["remote", "add", "origin", &url]);
        for (path, content) in files {
            write(&seed.join(path), content);
        }
        let head = sb.commit_push(&seed, "Anfang");
        (sb, url, head)
    }

    /// A seeded remote and two clones of it: "a" (this computer) and "b" (someone else).
    fn two_clones(name: &str, files: &[(&str, &[u8])]) -> (Sandbox, String, PathBuf, PathBuf) {
        let (sb, url, _) = seeded(name, files);
        let a = sb.clone(&url, "a");
        let b = sb.clone(&url, "b");
        (sb, url, a, b)
    }

    /// Installs a hook script in the repository at `dir`.
    #[cfg(unix)]
    fn hook(dir: &Path, name: &str, script: &str) {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join(".git/hooks").join(name);
        write(
            &path,
            format!("#!/bin/sh\nunset GIT_DIR GIT_INDEX_FILE GIT_WORK_TREE GIT_PREFIX\n{script}\n"),
        );
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    const PNG: &[u8] = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR base";
    const PNG_HERE: &[u8] = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR hier";
    const PNG_THERE: &[u8] = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR dort";

    // -- 1. status ----------------------------------------------------------------------------

    #[test]
    fn status_outside_a_repository_is_none() {
        let sb = Sandbox::new("status-none");
        let dir = sb.path("Ordner ohne Git");
        std::fs::create_dir_all(&dir).unwrap();
        assert!(status(&sb.git(&dir)).unwrap().is_none());
        assert!(matches!(plan_pull(&sb.git(&dir)), Err(GitError::NotARepo)));
        assert!(matches!(push(&sb.git(&dir), "", &me()), Err(GitError::NotARepo)));
    }

    #[test]
    fn status_suggests_an_existing_remote_for_linking() {
        let sb = Sandbox::new("status-known-url");
        sb.run(&sb.root, &["init", "-q", "-b", "main", "projekt"]);
        let dir = sb.path("projekt");
        assert_eq!(sb.status(&dir).known_url, None);
        sb.run(&dir, &["remote", "add", "zweit", "https://example.com/b.git"]);
        assert_eq!(sb.status(&dir).known_url.as_deref(), Some("https://example.com/b.git"));
        sb.run(&dir, &["remote", "add", "origin", "https://example.com/a.git"]);
        let st = sb.status(&dir);
        assert!(!st.linked());
        assert_eq!(st.known_url.as_deref(), Some("https://example.com/a.git"));
        // Linked: the upstream's own remote wins.
        sb.run(&dir, &["config", "branch.main.remote", "zweit"]);
        sb.run(&dir, &["config", "branch.main.merge", "refs/heads/main"]);
        let st = sb.status(&dir);
        assert!(st.linked());
        assert_eq!(st.known_url.as_deref(), Some("https://example.com/b.git"));
    }

    #[test]
    fn status_of_a_linked_clone() {
        let (sb, url, a, _) = two_clones("status-linked", &[("main.typ", b"= Hallo\n")]);
        let st = sb.status(&a);
        assert!(st.linked());
        assert_eq!(st.repo_root, a);
        assert_eq!(st.branch.as_deref(), Some("main"));
        assert!(!st.unborn);
        assert_eq!(st.remote.as_deref(), Some("origin"));
        assert_eq!(st.remote_url.as_deref(), Some(url.as_str()));
        assert_eq!(st.remote_branch.as_deref(), Some("main"));
        assert!(st.remote_branch_exists);
        assert_eq!((st.ahead, st.behind), (0, 0));
        assert!(st.changes.is_empty());
        assert_eq!(st.last_push, None);
        assert_eq!(st.in_progress, None);
        assert_eq!(st.config_identity, Identity::default());

        sb.run(&a, &["config", "user.name", "Konfig"]);
        sb.run(&a, &["config", "user.email", "konfig@example.com"]);
        let st = sb.status(&a);
        assert_eq!(
            st.config_identity,
            Identity {
                name: "Konfig".into(),
                email: "konfig@example.com".into()
            }
        );
    }

    #[test]
    fn status_lists_only_changes_inside_the_project_folder() {
        let (sb, _, a, _) = two_clones(
            "status-folder",
            &[
                ("Mein Projekt/haupt.typ", b"= Haupt\n"),
                ("Mein Projekt/alt.typ", b"alt\n"),
                ("Mein Projekt/weg.typ", b"weg\n"),
                ("außen.txt", b"aussen\n"),
            ],
        );
        let project = a.join("Mein Projekt");
        write(&project.join("haupt.typ"), "= Haupt\ngeändert\n");
        sb.run(&project, &["mv", "alt.typ", "neu.typ"]);
        remove(&project.join("weg.typ"));
        write(&project.join("Kapitel 1/Über uns.typ"), "neu\n");
        write(&a.join("außen.txt"), "geändert\n");
        write(&a.join("Notizen/idee.txt"), "neu\n");

        let st = sb.status(&project);
        assert_eq!(
            changes(&st),
            expect(&[
                ("Kapitel 1/Über uns.typ", Added),
                ("haupt.typ", Modified),
                ("neu.typ", Renamed),
                ("weg.typ", Deleted),
            ])
        );
        assert_eq!(st.repo_root, a);
        let whole = changes(&sb.status(&a));
        assert!(whole.contains(&("außen.txt".into(), Modified)));
        assert!(whole.contains(&("Notizen/idee.txt".into(), Added)));
        assert!(whole.contains(&("Mein Projekt/Kapitel 1/Über uns.typ".into(), Added)));
    }

    #[test]
    fn status_counts_ahead_and_behind() {
        let (sb, _, a, b) = two_clones("status-ab", &[("main.typ", b"1\n")]);
        write(&b.join("x.typ"), "x\n");
        sb.commit_push(&b, "x");
        write(&b.join("y.typ"), "y\n");
        sb.commit_push(&b, "y");
        write(&a.join("z.typ"), "z\n");
        sb.commit(&a, "z");

        let st = sb.status(&a);
        assert_eq!((st.ahead, st.behind), (1, 0), "behind counts as of the last fetch");
        fetch(&sb.git(&a)).unwrap();
        let st = sb.status(&a);
        assert_eq!((st.ahead, st.behind), (1, 2));
    }

    #[test]
    fn status_of_an_unborn_branch() {
        let sb = Sandbox::new("status-unborn");
        let url = sb.bare("remote.git");
        let project = sb.path("proj");
        write(&project.join("main.typ"), "= Hallo\n");
        link(&sb.git(&project), &url, "main").unwrap();

        let st = sb.status(&project);
        assert!(st.unborn && st.linked());
        assert_eq!(st.branch.as_deref(), Some("main"));
        assert!(!st.remote_branch_exists);
        assert_eq!((st.ahead, st.behind), (0, 0));
        assert_eq!(changes(&st), expect(&[(".gitignore", Added), ("main.typ", Added)]));

        // Someone else pushes first: the commits count as "behind".
        let other = sb.clone(&url, "other");
        write(&other.join("README.md"), "eins\n");
        sb.commit_push(&other, "eins");
        write(&other.join("README.md"), "zwei\n");
        sb.commit_push(&other, "zwei");
        fetch(&sb.git(&project)).unwrap();
        let st = sb.status(&project);
        assert!(st.unborn && st.remote_branch_exists);
        assert_eq!((st.ahead, st.behind), (0, 2));
    }

    #[test]
    fn status_of_a_detached_head() {
        let (sb, _, a, _) = two_clones("status-detached", &[("main.typ", b"1\n")]);
        sb.run(&a, &["checkout", "-q", "--detach"]);
        let st = sb.status(&a);
        assert_eq!(st.branch, None);
        assert!(!st.linked());
        let g = sb.git(&a);
        assert!(matches!(fetch(&g), Err(GitError::Detached)));
        assert!(matches!(plan_pull(&g), Err(GitError::Detached)));
        assert!(matches!(pull(&g, &[], &me()), Err(GitError::Detached)));
        assert!(matches!(push(&g, "", &me()), Err(GitError::Detached)));
    }

    #[test]
    fn status_reports_an_unfinished_merge() {
        let (sb, _, a, b) = two_clones("status-merge", &[("text.typ", b"eins\nzwei\n"), ("sub/x.typ", b"x\n")]);
        write(&b.join("text.typ"), "eins\nB\n");
        sb.commit_push(&b, "B");
        write(&a.join("text.typ"), "eins\nA\n");
        sb.commit(&a, "A");
        sb.run(&a, &["fetch", "-q"]);
        assert!(!sb.output(&a, &["merge", "-q", "origin/main"]).ok());

        let st = sb.status(&a);
        assert_eq!(st.in_progress.as_deref(), Some("Merge"));
        assert!(changes(&st).contains(&("text.typ".into(), Conflicted)));
        assert_eq!(sb.status(&a.join("sub")).in_progress.as_deref(), Some("Merge"));
        let g = sb.git(&a);
        assert!(matches!(plan_pull(&g), Err(GitError::InProgress(op)) if op == "Merge"));
        assert!(matches!(push(&g, "", &me()), Err(GitError::InProgress(_))));

        // Unmerged paths alone (outside the project folder, too) still block.
        remove(&a.join(".git/MERGE_HEAD"));
        assert_eq!(sb.status(&a).in_progress.as_deref(), Some("Merge"));
        assert_eq!(sb.status(&a.join("sub")).in_progress.as_deref(), Some("Merge"));
    }

    #[test]
    fn status_reads_a_remote_name_with_a_slash() {
        let (sb, _, a, b) = two_clones("status-slash", &[("main.typ", b"1\n")]);
        sb.run(&a, &["remote", "rename", "origin", "team/haupt"]);
        let st = sb.status(&a);
        assert!(st.linked() && st.remote_branch_exists);
        assert_eq!(st.remote.as_deref(), Some("team/haupt"));
        assert_eq!(st.remote_branch.as_deref(), Some("main"));

        write(&b.join("b.typ"), "b\n");
        let upstream = sb.commit_push(&b, "b");
        fetch(&sb.git(&a)).unwrap();
        assert_eq!(sb.status(&a).behind, 1);
        pull(&sb.git(&a), &[], &me()).unwrap();
        assert_eq!(sb.head(&a), upstream);
        write(&a.join("a.typ"), "a\n");
        assert_eq!(push(&sb.git(&a), "a", &me()).unwrap().commits, 1);
    }

    // -- 2. link ------------------------------------------------------------------------------

    #[test]
    fn link_a_plain_folder_to_an_empty_repository() {
        let sb = Sandbox::new("link-empty");
        let url = sb.bare("remote.git");
        let project = sb.path("Mein Projekt");
        write(&project.join("main.typ"), "= Hallo\n");
        let g = sb.git(&project);
        link(&g, &format!("  {url}  "), " main ").unwrap();

        assert!(project.join(".git").is_dir());
        assert_eq!(read(&project.join(".gitignore")).as_deref(), Some(".DS_Store\n"));
        assert_eq!(sb.run(&project, &["config", "branch.main.remote"]), "origin");
        assert_eq!(sb.run(&project, &["config", "branch.main.merge"]), "refs/heads/main");
        assert_eq!(sb.run(&project, &["remote", "get-url", "origin"]), url);
        let st = sb.status(&project);
        assert!(st.linked() && st.unborn && !st.remote_branch_exists);

        // The first push creates the branch.
        assert_eq!(push(&g, "Erster Stand", &me()).unwrap().commits, 1);
        let remote = sb.path("remote.git");
        assert_eq!(sb.run(&remote, &["ls-tree", "-r", "--name-only", "main"]), ".gitignore\nmain.typ");
        let st = sb.status(&project);
        assert!(!st.unborn && st.remote_branch_exists);
        assert_eq!((st.ahead, st.behind), (0, 0));
        assert!(st.changes.is_empty());
        assert!(st.last_push.is_some());
    }

    #[test]
    fn link_adopts_a_repository_with_a_readme() {
        let (sb, url, upstream) = seeded("link-readme", &[("README.md", b"# Buch\n")]);
        let project = sb.path("proj");
        write(&project.join("main.typ"), "= Hallo\n");
        link(&sb.git(&project), &url, "main").unwrap();

        assert_eq!(read(&project.join("README.md")).as_deref(), Some("# Buch\n"));
        assert_eq!(sb.head(&project), upstream);
        let st = sb.status(&project);
        assert!(st.linked() && !st.unborn && st.remote_branch_exists);
        assert_eq!((st.ahead, st.behind), (0, 0));
        assert_eq!(changes(&st), expect(&[(".gitignore", Added), ("main.typ", Added)]));
    }

    #[test]
    fn link_refuses_overlapping_files_and_leaves_nothing_behind() {
        let (sb, url, _) = seeded("link-overlap", &[("main.typ", b"aus dem Repository\n"), ("README.md", b"r\n")]);
        let project = sb.path("proj");
        write(&project.join("main.typ"), "von hier\n");
        write(&project.join("eigen.typ"), "eigen\n");
        let err = link(&sb.git(&project), &url, "main").unwrap_err();

        assert!(
            matches!(&err, GitError::Overlap { paths } if *paths == strings(&["main.typ"])),
            "{err:?}"
        );
        assert!(err.to_string().contains("main.typ"));
        assert!(!project.join(".git").exists());
        assert!(!project.join(".gitignore").exists());
        assert!(!project.join("README.md").exists());
        assert_eq!(read(&project.join("main.typ")).as_deref(), Some("von hier\n"));
        assert!(status(&sb.git(&project)).unwrap().is_none());
    }

    #[test]
    fn link_refuses_files_in_the_way_of_remote_folders() {
        let (sb, url, _) = seeded(
            "link-blocked",
            &[("bilder/logo.png", PNG), ("main.typ", b"= Hallo\n"), ("vorlage", b"v\n")],
        );
        let project = sb.path("proj");
        write(&project.join("bilder"), "eine Datei, kein Ordner\n");
        write(&project.join("vorlage/eigen.typ"), "ein Ordner, keine Datei\n");
        let err = link(&sb.git(&project), &url, "main").unwrap_err();
        assert!(
            matches!(&err, GitError::Overlap { paths } if *paths == strings(&["bilder/logo.png", "vorlage"])),
            "{err:?}"
        );
        assert!(!project.join(".git").exists());
        assert!(!project.join("main.typ").exists(), "no remote file was created");
        assert_eq!(read(&project.join("bilder")).as_deref(), Some("eine Datei, kein Ordner\n"));
    }

    #[test]
    fn link_takes_the_repository_gitignore() {
        let (sb, url, _) = seeded("link-gitignore", &[(".gitignore", b"*.pdf\n"), ("README.md", b"r\n")]);
        let project = sb.path("proj");
        write(&project.join("main.typ"), "= Hallo\n");
        link(&sb.git(&project), &url, "main").unwrap();
        assert_eq!(read(&project.join(".gitignore")).as_deref(), Some("*.pdf\n"));
        assert_eq!(changes(&sb.status(&project)), expect(&[("main.typ", Added)]));
    }

    #[test]
    fn link_refuses_an_unrelated_history() {
        let (sb, url, _) = seeded("link-unrelated", &[("README.md", b"r\n")]);
        let project = sb.path("proj");
        sb.run(&sb.root, &["init", "-q", "-b", "main", "proj"]);
        write(&project.join("main.typ"), "= Hallo\n");
        let head = sb.commit(&project, "Eigene Geschichte");

        let err = link(&sb.git(&project), &url, "main").unwrap_err();
        assert!(matches!(&err, GitError::Unrelated { branch } if branch == "main"), "{err:?}");
        // The remote this call added is gone again; the repository itself stays.
        assert_eq!(sb.run(&project, &["remote"]), "");
        assert!(!sb.output(&project, &["config", "branch.main.remote"]).ok());
        assert_eq!(sb.head(&project), head);
        assert!(!sb.status(&project).linked());
    }

    #[test]
    fn link_refuses_a_folder_inside_another_repository() {
        let (sb, url, _) = seeded("link-nested", &[("README.md", b"r\n")]);
        let outer = sb.path("outer");
        sb.run(&sb.root, &["init", "-q", "-b", "main", "outer"]);
        let project = outer.join("projekte/buch");
        std::fs::create_dir_all(&project).unwrap();
        write(&project.join("main.typ"), "= Hallo\n");

        let err = link(&sb.git(&project), &url, "main").unwrap_err();
        assert!(
            matches!(&err, GitError::Other(m) if m.contains("innerhalb des Git-Repositories")),
            "{err:?}"
        );
        assert!(!project.join(".git").exists(), "nothing was initialised here");
        assert_eq!(sb.run(&outer, &["remote"]), "", "the outer repository stayed as it was");
    }

    #[test]
    fn link_refuses_a_core_worktree_pointing_elsewhere() {
        let (sb, url, _) = seeded("link-worktree", &[("README.md", b"r\n")]);
        let project = sb.path("proj");
        sb.run(&sb.root, &["init", "-q", "-b", "main", "proj"]);
        let elsewhere = sb.path("anderes");
        std::fs::create_dir_all(&elsewhere).unwrap();
        sb.run(&project, &["config", "core.worktree", elsewhere.to_str().unwrap()]);

        let err = link(&sb.git(&project), &url, "main").unwrap_err();
        assert!(matches!(&err, GitError::Other(m) if m.contains("core.worktree")), "{err:?}");
        assert_eq!(sb.run(&project, &["remote"]), "", "the remote this call would add is not there");
        assert!(!project.join(".gitignore").exists());
    }

    #[test]
    fn path_collisions_find_file_folder_and_case_conflicts() {
        let ignorecase = Repo {
            root: PathBuf::new(),
            prefix: String::new(),
            index: PathBuf::new(),
            temp_index: PathBuf::new(),
            attributes: PathBuf::new(),
            merge_head: PathBuf::new(),
            journal: PathBuf::new(),
            operation: None,
            config: Config::parse(b"core.ignorecase\ntrue\0"),
        };
        let a: BTreeSet<String> = strings(&["Bilder/Logo.png", "notizen.typ", "a/b.typ"]).into_iter().collect();
        let b: BTreeSet<String> = strings(&["Bilder", "notizen.TYP", "a/b.typ"]).into_iter().collect();
        assert_eq!(
            path_collisions(&a, &b, &ignorecase).into_iter().collect::<Vec<_>>(),
            strings(&["Bilder", "Bilder/Logo.png", "notizen.typ"])
        );
        // Without case folding only the file/folder conflicts remain.
        let exact = Repo {
            config: Config::parse(b""),
            ..ignorecase
        };
        let out = path_collisions(&a, &b, &exact);
        assert!(out.contains("Bilder") && out.contains("Bilder/Logo.png"), "{out:?}");
        if !cfg!(target_os = "macos") {
            assert!(!out.contains("notizen.typ"), "{out:?}");
        }
    }

    #[test]
    fn pull_refuses_when_file_and_folder_collide() {
        let sb = Sandbox::new("pull-collision");
        let url = sb.bare("remote.git");
        let seed = sb.clone(&url, "seed");
        // The repository knows "Bilder" as a file, the folder wants it as a folder.
        write(&seed.join("Bilder"), "eine Datei\n");
        sb.commit_push(&seed, "Bilder als Datei");
        let a = sb.clone(&url, "a");
        remove(&a.join("Bilder"));
        write(&a.join("Bilder/logo.png"), PNG);
        // Something new to pull.
        write(&seed.join("README.md"), "r\n");
        sb.commit_push(&seed, "Weiter");

        let err = pull(&sb.git(&a), &[], &me()).unwrap_err();
        assert!(
            matches!(&err, GitError::Other(m) if m.contains("kollidieren zwischen Ordner und Repository")),
            "{err:?}"
        );
        assert!(a.join("Bilder/logo.png").exists(), "the folder was not touched");
        assert!(!a.join(".git/MERGE_HEAD").exists(), "no merge was started");
    }

    #[test]
    fn link_an_existing_clone_to_a_new_branch() {
        let (sb, url, a, _) = two_clones("link-branch", &[("main.typ", b"1\n")]);
        link(&sb.git(&a), &url, "entwurf").unwrap();
        let st = sb.status(&a);
        assert_eq!(st.remote.as_deref(), Some("origin"), "the remote with this URL is reused");
        assert_eq!(st.remote_branch.as_deref(), Some("entwurf"));
        assert!(st.linked() && !st.remote_branch_exists);
        assert_eq!(st.ahead, 1, "the first push takes every local commit along");

        assert_eq!(push(&sb.git(&a), "", &me()).unwrap().commits, 1);
        assert_eq!(sb.run(&sb.path("remote.git"), &["rev-parse", "entwurf"]), sb.head(&a));
        assert!(sb.status(&a).remote_branch_exists);
    }

    #[test]
    fn link_adds_its_own_remote_next_to_a_foreign_origin() {
        let (sb, url, _) = seeded("link-remote", &[("README.md", b"r\n")]);
        let project = sb.path("proj");
        sb.run(&sb.root, &["init", "-q", "-b", "main", "proj"]);
        sb.run(&project, &["remote", "add", "origin", "https://example.com/anderes.git"]);
        link(&sb.git(&project), &url, "main").unwrap();
        assert_eq!(
            sb.run(&project, &["remote", "get-url", "origin"]),
            "https://example.com/anderes.git"
        );
        assert_eq!(sb.run(&project, &["remote", "get-url", "typisch"]), url);
        assert_eq!(sb.status(&project).remote.as_deref(), Some("typisch"));
    }

    #[test]
    fn link_checks_its_input_and_rolls_back() {
        let sb = Sandbox::new("link-input");
        let url = sb.bare("remote.git");
        let project = sb.path("proj");
        write(&project.join("main.typ"), "= Hallo\n");
        let g = sb.git(&project);

        assert!(matches!(link(&g, "  ", "main"), Err(GitError::Other(m)) if m == no_url()));
        assert!(matches!(link(&g, &url, ""), Err(GitError::Other(_))));
        assert!(matches!(link(&g, &url, "a..b"), Err(GitError::Other(m)) if m.contains("a..b")));
        assert!(matches!(link(&g, &url, "-x"), Err(GitError::Other(_))));
        let missing = format!("file://{}", sb.path("fehlt.git").display());
        assert!(matches!(link(&g, &missing, "main"), Err(GitError::RepoNotFound(u)) if u == missing));
        assert!(!project.join(".git").exists());
        assert!(!project.join(".gitignore").exists());
    }

    #[test]
    fn pick_remote_never_takes_over_a_name() {
        let config = Config::parse(b"remote.origin.url\nhttps://a\0remote.origin.fetch\nx\0remote.typisch.url\nhttps://b\0");
        assert_eq!(pick_remote(&config, "https://a"), ("origin".into(), false));
        assert_eq!(pick_remote(&config, "https://b"), ("typisch".into(), false));
        assert_eq!(pick_remote(&config, "https://c"), ("typisch2".into(), true));
        assert_eq!(pick_remote(&Config::default(), "https://c"), ("origin".into(), true));
    }

    #[test]
    fn unlink_keeps_the_repository() {
        let (sb, _, a, _) = two_clones("unlink", &[("main.typ", b"1\n")]);
        unlink(&sb.git(&a)).unwrap();
        let st = sb.status(&a);
        assert!(!st.linked());
        assert_eq!(sb.run(&a, &["remote"]), "origin");
        assert!(matches!(push(&sb.git(&a), "", &me()), Err(GitError::NotLinked)));
        unlink(&sb.git(&a)).unwrap();
    }

    // -- 3. push ------------------------------------------------------------------------------

    #[test]
    fn push_commits_only_the_project_folder() {
        let (sb, _, a, _) = two_clones("push-folder", &[("buch/main.typ", b"= Buch\n"), ("außen.txt", b"aussen\n")]);
        let project = a.join("buch");
        write(&project.join("main.typ"), "= Buch\nNeu\n");
        write(&project.join("Bilder/Logo 1.png"), PNG);
        write(&a.join("außen.txt"), "geändert\n");
        sb.run(&a, &["add", "außen.txt"]);
        write(&a.join("lose.txt"), "lose\n");
        let started = now_secs();

        assert_eq!(push(&sb.git(&project), "Kapitel überarbeitet", &me()).unwrap().commits, 1);
        let remote = sb.path("remote.git");
        assert_eq!(
            sb.run(&remote, &["log", "-1", "--format=%s%n%an%n%ae%n%cn%n%ce", "main"]),
            "Kapitel überarbeitet\nErika Mustermann\nerika@example.com\nErika Mustermann\nerika@example.com"
        );
        let out = sb.output(&remote, &["diff-tree", "--no-commit-id", "--name-only", "-r", "-z", "main"]);
        let files: Vec<String> = String::from_utf8_lossy(&out.stdout)
            .split('\0')
            .filter(|p| !p.is_empty())
            .map(String::from)
            .collect();
        assert_eq!(files, strings(&["buch/Bilder/Logo 1.png", "buch/main.typ"]));

        // Elsewhere nothing changed: the staged file is still staged, the loose one still loose.
        let staged = sb.output(&a, &["diff", "--cached", "--name-only", "-z"]);
        assert_eq!(String::from_utf8_lossy(&staged.stdout), "außen.txt\0");
        assert_eq!(changes(&sb.status(&a)), expect(&[("außen.txt", Modified), ("lose.txt", Added)]));
        let st = sb.status(&project);
        assert!(st.changes.is_empty());
        assert_eq!((st.ahead, st.behind), (0, 0));
        let pushed = st.last_push.expect("typisch.lastPush");
        assert!(pushed >= started && pushed <= now_secs());
    }

    #[test]
    fn push_refuses_when_the_repository_moved_on() {
        let (sb, _, a, b) = two_clones("push-behind", &[("main.typ", b"1\n")]);
        write(&b.join("other.typ"), "b\n");
        let theirs = sb.commit_push(&b, "Fremd");
        write(&a.join("main.typ"), "2\n");
        let before = sb.head(&a);

        let err = push(&sb.git(&a), "Mein Stand", &me()).unwrap_err();
        assert!(matches!(err, GitError::NeedsPull { behind: 1 }), "{err:?}");
        assert_eq!(sb.head(&a), before, "nothing was committed");
        assert_eq!(changes(&sb.status(&a)), expect(&[("main.typ", Modified)]));
        assert_eq!(sb.run(&sb.path("remote.git"), &["rev-parse", "main"]), theirs);
        assert_eq!(sb.status(&a).behind, 1);
    }

    #[test]
    fn push_refuses_a_local_branch_tracking_a_different_existing_branch() {
        let (sb, _, a, _) = two_clones("push-experiment", &[("main.typ", b"1\n")]);
        // `git switch -c entwurf origin/main` keeps tracking origin/main: a push must not publish there.
        sb.run(&a, &["switch", "-q", "-c", "entwurf", "origin/main"]);
        write(&a.join("main.typ"), "2\n");
        let before = sb.head(&a);

        let err = push(&sb.git(&a), "Entwurf", &me()).unwrap_err();
        assert!(
            matches!(&err, GitError::Other(m) if m.contains("Ein Push würde nach „main“ schreiben")),
            "{err:?}"
        );
        assert_eq!(sb.head(&a), before, "nothing was committed");
        assert_eq!(sb.run(&sb.path("remote.git"), &["rev-parse", "main"]), sb.head(&a));
        assert!(sb.status(&a).remote_branch_exists);
    }

    #[test]
    fn fetch_notices_a_vanished_remote_branch() {
        let (sb, _, a, _) = two_clones("fetch-vanished", &[("main.typ", b"1\n")]);
        sb.run(&sb.path("remote.git"), &["update-ref", "-d", "refs/heads/main"]);

        let err = fetch(&sb.git(&a)).unwrap_err();
        assert!(
            matches!(&err, GitError::Other(m) if m.contains("gelöscht oder umbenannt")),
            "{err:?}"
        );
        // The stale tracking ref stays, so the status does not pretend everything is fine.
        assert!(sb.status(&a).remote_branch_exists);
    }

    #[test]
    fn status_restores_after_an_interrupted_pull() {
        let (sb, _, a, b) = two_clones("pull-journal", &[("main.typ", b"1\n")]);
        write(&b.join("other.typ"), "b\n");
        sb.commit_push(&b, "Fremd");
        // Local work that only exists uncommitted.
        write(&a.join("main.typ"), "2\n");
        write(&a.join("notizen.typ"), "Notizen\n");
        let before = sb.head(&a);
        // A pull got as far as its snapshot and its work-tree reset, then died: journal, backup
        // commit and a work tree at the upstream commit, without the local changes.
        let sha = sb.run(&a, &["stash", "create", "Typisch: Sicherung vor Pull"]);
        assert!(!sha.is_empty());
        let journal = sb.output(&a, &["rev-parse", "--git-path", "typisch-pull"]).text();
        std::fs::write(a.join(journal.trim()), format!("{before}\nrefs/typisch/backup/1\n{sha}\n")).unwrap();
        sb.run(&a, &["reset", "-q", "--hard", "origin/main"]);

        let st = sb.status(&a);
        assert_eq!(sb.head(&a), before, "the local commits are back");
        assert_eq!(read(&a.join("main.typ")).as_deref(), Some("2\n"), "the uncommitted work is back");
        assert_eq!(read(&a.join("notizen.typ")).as_deref(), Some("Notizen\n"));
        assert_eq!(
            changes(&st),
            expect(&[("main.typ", Modified), ("notizen.typ", Added)]),
            "nothing stayed committed"
        );
        assert!(!a.join(journal.trim()).exists(), "the journal is spent");
    }

    #[test]
    fn push_with_nothing_to_push() {
        let (sb, _, a, _) = two_clones("push-nothing", &[("main.typ", b"1\n")]);
        let before = sb.head(&a);
        assert_eq!(push(&sb.git(&a), "", &me()).unwrap().commits, 0);
        assert_eq!(sb.head(&a), before);

        // An earlier commit made outside Typisch goes up, too.
        write(&a.join("main.typ"), "2\n");
        let earlier = sb.commit(&a, "Im Terminal");
        assert_eq!(push(&sb.git(&a), "", &me()).unwrap().commits, 1);
        assert_eq!(sb.run(&sb.path("remote.git"), &["rev-parse", "main"]), earlier);
    }

    #[test]
    fn push_falls_back_to_the_default_message_and_config_identity() {
        let (sb, _, a, _) = two_clones("push-defaults", &[("main.typ", b"1\n")]);
        sb.run(&a, &["config", "user.name", "Konfig Name"]);
        sb.run(&a, &["config", "user.email", "konfig@example.com"]);
        write(&a.join("main.typ"), "2\n");
        assert_eq!(push(&sb.git(&a), "  \n", &Identity::default()).unwrap().commits, 1);
        assert_eq!(
            sb.run(&a, &["log", "-1", "--format=%s|%an|%ae"]),
            "Änderungen aus Typisch|Konfig Name|konfig@example.com"
        );
    }

    #[test]
    fn push_without_identity_is_refused() {
        let (sb, _, a, _) = two_clones("push-anonymous", &[("main.typ", b"1\n")]);
        write(&a.join("main.typ"), "2\n");
        let before = sb.head(&a);
        let err = push(&sb.git(&a), "x", &Identity::default()).unwrap_err();
        assert!(
            matches!(&err, GitError::Other(m) if m == "Name und E-Mail-Adresse fehlen."),
            "{err:?}"
        );
        assert_eq!(sb.head(&a), before);
    }

    #[cfg(unix)]
    #[test]
    fn push_that_loses_a_race_takes_its_commit_back() {
        let (sb, _, a, b) = two_clones("push-race", &[("main.typ", b"1\n")]);
        write(&b.join("other.typ"), "b\n");
        let theirs = sb.commit(&b, "Fremd");
        write(&a.join("main.typ"), "2\n");
        let before = sb.head(&a);
        // Someone else's push lands between our check and our push.
        hook(
            &a,
            "pre-push",
            &format!("exec git -C '{}' push -q origin HEAD:refs/heads/main", b.display()),
        );

        let err = push(&sb.git(&a), "Mein Stand", &me()).unwrap_err();
        assert!(matches!(err, GitError::NeedsPull { behind: 1 }), "{err:?}");
        assert_eq!(sb.head(&a), before, "our commit was taken back");
        assert_eq!(read(&a.join("main.typ")).as_deref(), Some("2\n"));
        assert_eq!(changes(&sb.status(&a)), expect(&[("main.typ", Modified)]));
        // Never forced: the remote still has the other side's commit, and not ours.
        let remote = sb.path("remote.git");
        assert_eq!(sb.run(&remote, &["rev-parse", "main"]), theirs);
        assert!(!sb.run(&remote, &["log", "--format=%s", "main"]).contains("Mein Stand"));
    }

    #[cfg(unix)]
    #[test]
    fn first_push_that_loses_a_race_leaves_the_files_uncommitted() {
        let sb = Sandbox::new("push-race-first");
        let url = sb.bare("remote.git");
        let project = sb.path("proj");
        write(&project.join("main.typ"), "= Hallo\n");
        link(&sb.git(&project), &url, "main").unwrap();
        let other = sb.clone(&url, "other");
        write(&other.join("README.md"), "r\n");
        let theirs = sb.commit(&other, "Fremd");
        hook(
            &project,
            "pre-push",
            &format!("exec git -C '{}' push -q origin HEAD:refs/heads/main", other.display()),
        );

        let err = push(&sb.git(&project), "Erster Stand", &me()).unwrap_err();
        assert!(matches!(err, GitError::NeedsPull { behind: 1 }), "{err:?}");
        let st = sb.status(&project);
        assert!(st.unborn, "the root commit was taken back");
        assert_eq!(changes(&st), expect(&[(".gitignore", Added), ("main.typ", Added)]));
        assert_eq!(sb.run(&project, &["ls-files"]), "");
        assert_eq!(sb.run(&sb.path("remote.git"), &["rev-parse", "main"]), theirs);
    }

    #[test]
    fn push_rejections_are_recognised() {
        assert!(push_rejected("!\trefs/heads/main:refs/heads/main\t[rejected] (fetch first)"));
        assert!(push_rejected(
            "!\thead:refs/heads/main\t[remote rejected] (incorrect old value provided)"
        ));
        assert!(push_rejected(
            "remote: error: cannot lock ref 'refs/heads/main': reference already exists"
        ));
        assert!(!push_rejected(
            "!\thead:refs/heads/main\t[remote rejected] (pre-receive hook declined)"
        ));
    }

    // -- 4. plan_pull -------------------------------------------------------------------------

    /// Changes on both sides that need each kind of question, a text conflict, and a local
    /// deletion of a remotely modified file. Returns the upstream commit.
    fn conflict_scene(name: &str) -> (Sandbox, PathBuf, String) {
        let (sb, _, a, b) = two_clones(
            name,
            &[
                ("text.typ", b"eins\nzwei\ndrei\n"),
                ("bild.png", PNG),
                ("entfernt.typ", b"bleibt?\n"),
                ("lokal-weg.typ", b"alt\n"),
                ("ruhig.typ", b"ruhig\n"),
            ],
        );
        write(&b.join("text.typ"), "eins\nREMOTE\ndrei\n");
        write(&b.join("bild.png"), PNG_THERE);
        remove(&b.join("entfernt.typ"));
        write(&b.join("lokal-weg.typ"), "alt\nim Repository ergänzt\n");
        write(&b.join("neu.typ"), "aus dem Repository\n");
        let upstream = sb.commit_push(&b, "Fremd");

        write(&a.join("text.typ"), "eins\nLOKAL\ndrei\n");
        write(&a.join("bild.png"), PNG_HERE);
        write(&a.join("entfernt.typ"), "bleibt!\n");
        remove(&a.join("lokal-weg.typ"));
        write(&a.join("neu.typ"), "hier angelegt\n");
        write(&a.join("notiz.typ"), "nur hier\n");
        (sb, a, upstream)
    }

    #[test]
    fn plan_pull_asks_exactly_the_needed_questions() {
        let (sb, a, upstream) = conflict_scene("plan");
        let head = sb.head(&a);
        let before = changes(&sb.status(&a));

        let plan = plan_pull(&sb.git(&a)).unwrap();
        assert_eq!(plan.behind, 1);
        assert_eq!(plan.upstream, upstream);
        assert_eq!(
            plan.questions,
            vec![
                Question::BinaryChanged("bild.png".into()),
                Question::DeletedInRepo("entfernt.typ".into()),
                Question::AddedBoth("neu.typ".into()),
            ]
        );
        assert_eq!(plan.merged_text, strings(&["text.typ"]));

        // Only the tracking ref moved.
        assert_eq!(sb.head(&a), head);
        assert_eq!(changes(&sb.status(&a)), before);
        assert_eq!(read(&a.join("text.typ")).as_deref(), Some("eins\nLOKAL\ndrei\n"));
        assert!(
            sb.output(&a, &["diff", "--cached", "--quiet"]).ok(),
            "the staging area is untouched"
        );
        assert!(!a.join(".git/typisch-index").exists());
        assert!(!a.join(".git/typisch-attributes").exists());
    }

    /// Git takes a file for unchanged when its size and times are as the index recorded them,
    /// unless the file may have been written in the very second the index was: then it reads
    /// the file. A scratch index newer than that second would hide an edit that keeps the size.
    #[test]
    fn plan_pull_sees_a_same_size_edit_made_in_the_second_the_index_was_written() {
        let (sb, _, a, b) = two_clones("plan-racy", &[("entfernt.typ", b"bleibt?\n"), ("ruhig.typ", b"ruhig\n")]);
        remove(&b.join("entfernt.typ"));
        sb.commit_push(&b, "Fremd");

        // One moment for the index, its entry and the edit, without waiting for the clock: an
        // hour ago, set by hand. The change time cannot be set, so git is told to leave it out.
        sb.run(&a, &["config", "core.trustctime", "false"]);
        let then = std::time::SystemTime::now() - std::time::Duration::from_secs(3600);
        let dated = |path: &Path| {
            let file = std::fs::File::options().write(true).open(path).unwrap();
            file.set_modified(then).unwrap();
        };
        let file = a.join("entfernt.typ");
        dated(&file);
        sb.run(&a, &["add", "entfernt.typ"]);
        write(&file, "bleibt!\n");
        dated(&file);
        dated(&a.join(".git/index"));

        let plan = plan_pull(&sb.git(&a)).unwrap();
        assert_eq!(plan.questions, vec![Question::DeletedInRepo("entfernt.typ".into())]);
    }

    #[test]
    fn plan_pull_respects_the_repository_merge_attributes() {
        let (sb, _, a, b) = two_clones(
            "plan-attributes",
            &[(".gitattributes", b"*.csv -merge\n"), ("daten.csv", b"a,b\n1,2\n")],
        );
        write(&b.join("daten.csv"), "a,b\n1,3\n");
        sb.commit_push(&b, "Fremd");
        write(&a.join("daten.csv"), "a,b\n1,4\n");

        let plan = plan_pull(&sb.git(&a)).unwrap();
        assert_eq!(plan.questions, vec![Question::BinaryChanged("daten.csv".into())]);
        assert!(plan.merged_text.is_empty());
        pull(&sb.git(&a), &[("daten.csv".into(), Choice::Remote)], &me()).unwrap();
        assert_eq!(read(&a.join("daten.csv")).as_deref(), Some("a,b\n1,3\n"));
    }

    #[test]
    fn plan_pull_with_nothing_new() {
        let (sb, _, a, _) = two_clones("plan-nothing", &[("main.typ", b"1\n")]);
        write(&a.join("main.typ"), "2\n");
        let plan = plan_pull(&sb.git(&a)).unwrap();
        assert_eq!(plan.behind, 0);
        assert_eq!(plan.upstream, sb.head(&a));
        assert!(plan.questions.is_empty() && plan.merged_text.is_empty());
        let out = pull(&sb.git(&a), &[], &me()).unwrap();
        assert_eq!(out.commits, 0);
        assert_eq!(read(&a.join("main.typ")).as_deref(), Some("2\n"));

        // A branch that does not exist on the remote yet.
        let sb = Sandbox::new("plan-no-branch");
        let url = sb.bare("remote.git");
        let project = sb.path("proj");
        write(&project.join("main.typ"), "1\n");
        link(&sb.git(&project), &url, "main").unwrap();
        let plan = plan_pull(&sb.git(&project)).unwrap();
        assert_eq!((plan.behind, plan.upstream.as_str()), (0, ""));
        assert_eq!(pull(&sb.git(&project), &[], &me()).unwrap().commits, 0);
    }

    // -- 5. pull ------------------------------------------------------------------------------

    /// No pull leaves anything of its own behind.
    fn assert_tidy(sb: &Sandbox, repo: &Path) {
        assert_eq!(sb.run(repo, &["for-each-ref", "refs/typisch"]), "", "backup refs left");
        assert!(!repo.join(".git/MERGE_HEAD").exists());
        assert!(!repo.join(".git/typisch-attributes").exists());
        assert!(!repo.join(".git/typisch-index").exists());
        assert!(sb.output(repo, &["diff", "--cached", "--quiet"]).ok(), "local work stays unstaged");
    }

    #[test]
    fn pull_fast_forwards_a_clean_folder() {
        let (sb, _, a, b) = two_clones("pull-ff", &[("main.typ", b"1\n")]);
        write(&b.join("main.typ"), "2\n");
        sb.commit_push(&b, "zwei");
        write(&b.join("kapitel.typ"), "k\n");
        let upstream = sb.commit_push(&b, "Kapitel");

        let out = pull(&sb.git(&a), &[], &me()).unwrap();
        assert_eq!(out.commits, 2);
        assert!(out.merged_text.is_empty() && out.kept_local.is_empty() && out.took_remote.is_empty());
        assert_eq!(sb.head(&a), upstream);
        assert_eq!(read(&a.join("main.typ")).as_deref(), Some("2\n"));
        assert_eq!(read(&a.join("kapitel.typ")).as_deref(), Some("k\n"));
        let st = sb.status(&a);
        assert!(st.changes.is_empty());
        assert_eq!((st.ahead, st.behind), (0, 0));
        assert_tidy(&sb, &a);
    }

    #[test]
    fn pull_keeps_both_sides_of_a_text_conflict() {
        let (sb, _, a, b) = two_clones("pull-text", &[("text.typ", b"eins\nzwei\ndrei\n")]);
        // The user's own stash entry must survive untouched.
        write(&a.join("eigenes.typ"), "x\n");
        sb.run(&a, &["stash", "push", "-q", "-u", "-m", "eigener Stash"]);
        write(&b.join("text.typ"), "eins\nREMOTE\ndrei\n");
        let upstream = sb.commit_push(&b, "Fremd");
        write(&a.join("text.typ"), "eins\nLOKAL\ndrei\n");
        write(&a.join("notiz.typ"), "nur hier\n");

        let out = pull(&sb.git(&a), &[], &me()).unwrap();
        assert_eq!(out.commits, 1);
        assert_eq!(out.merged_text, strings(&["text.typ"]));
        let text = read(&a.join("text.typ")).unwrap();
        assert!(text.starts_with("eins\n") && text.ends_with("drei\n"), "{text}");
        assert!(text.contains("REMOTE\n") && text.contains("LOKAL\n"), "{text}");
        assert_eq!(sb.head(&a), upstream);
        assert_eq!(changes(&sb.status(&a)), expect(&[("notiz.typ", Added), ("text.typ", Modified)]));
        let stashes = sb.run(&a, &["stash", "list"]);
        assert_eq!(stashes.lines().count(), 1, "{stashes}");
        assert!(stashes.contains("eigener Stash"));
        assert_tidy(&sb, &a);
    }

    #[test]
    fn pull_keeps_the_local_side_unless_asked_otherwise() {
        let (sb, a, upstream) = conflict_scene("pull-local");
        let out = pull(&sb.git(&a), &[("bild.png".into(), Choice::Local)], &me()).unwrap();
        assert_eq!(out.commits, 1);
        assert_eq!(out.kept_local, strings(&["bild.png", "entfernt.typ", "neu.typ"]));
        assert!(out.took_remote.is_empty());
        assert_eq!(out.merged_text, strings(&["text.typ"]));

        assert_eq!(sb.head(&a), upstream);
        assert_eq!(std::fs::read(a.join("bild.png")).unwrap(), PNG_HERE);
        assert_eq!(read(&a.join("entfernt.typ")).as_deref(), Some("bleibt!\n"));
        assert_eq!(read(&a.join("neu.typ")).as_deref(), Some("hier angelegt\n"));
        assert!(!a.join("lokal-weg.typ").exists(), "the local deletion wins");
        assert_eq!(read(&a.join("notiz.typ")).as_deref(), Some("nur hier\n"));
        let text = read(&a.join("text.typ")).unwrap();
        assert!(text.contains("REMOTE\n") && text.contains("LOKAL\n"), "{text}");
        assert_eq!(
            changes(&sb.status(&a)),
            expect(&[
                ("bild.png", Modified),
                ("entfernt.typ", Added),
                ("lokal-weg.typ", Deleted),
                ("neu.typ", Modified),
                ("notiz.typ", Added),
                ("text.typ", Modified),
            ])
        );
        assert_tidy(&sb, &a);
    }

    #[test]
    fn pull_takes_the_remote_side_when_asked() {
        let (sb, a, upstream) = conflict_scene("pull-remote");
        let answers: Vec<(String, Choice)> = ["bild.png", "entfernt.typ", "neu.typ"]
            .iter()
            .map(|p| (p.to_string(), Choice::Remote))
            .collect();
        let out = pull(&sb.git(&a), &answers, &me()).unwrap();
        assert_eq!(out.took_remote, strings(&["bild.png", "entfernt.typ", "neu.typ"]));
        assert!(out.kept_local.is_empty());

        assert_eq!(sb.head(&a), upstream);
        assert_eq!(std::fs::read(a.join("bild.png")).unwrap(), PNG_THERE);
        assert!(!a.join("entfernt.typ").exists());
        assert_eq!(read(&a.join("neu.typ")).as_deref(), Some("aus dem Repository\n"));
        assert!(!a.join("lokal-weg.typ").exists(), "the local deletion wins");
        assert_eq!(read(&a.join("notiz.typ")).as_deref(), Some("nur hier\n"));
        assert_eq!(
            changes(&sb.status(&a)),
            expect(&[("lokal-weg.typ", Deleted), ("notiz.typ", Added), ("text.typ", Modified)])
        );
        assert_tidy(&sb, &a);
    }

    #[test]
    fn pull_answers_touch_only_their_own_file() {
        // "[ab].png" read as a pattern would match "a.png" too.
        let (sb, _, a, b) = two_clones("pull-glob", &[("[ab].png", PNG), ("a.png", PNG)]);
        write(&b.join("[ab].png"), PNG_THERE);
        sb.commit_push(&b, "Fremd");
        write(&a.join("[ab].png"), PNG_HERE);
        write(&a.join("a.png"), PNG_HERE);

        let plan = plan_pull(&sb.git(&a)).unwrap();
        assert_eq!(plan.questions, vec![Question::BinaryChanged("[ab].png".into())]);
        pull(&sb.git(&a), &[("[ab].png".into(), Choice::Remote)], &me()).unwrap();
        assert_eq!(std::fs::read(a.join("[ab].png")).unwrap(), PNG_THERE);
        assert_eq!(std::fs::read(a.join("a.png")).unwrap(), PNG_HERE);
        assert_tidy(&sb, &a);
    }

    #[test]
    fn pull_never_overwrites_ignored_files() {
        let (sb, _, a, b) = two_clones("pull-ignored", &[(".gitignore", b"*.pdf\n"), ("main.typ", b"1\n")]);
        write(&a.join("main.pdf"), "meine PDF\n");
        write(&a.join("main.typ"), "lokal\n");
        write(&b.join("main.pdf"), "fremde PDF\n");
        sb.run(&b, &["add", "-f", "main.pdf"]);
        sb.commit_push(&b, "PDF eingecheckt");
        let head = sb.head(&a);

        let err = pull(&sb.git(&a), &[], &me()).unwrap_err();
        assert!(
            matches!(&err, GitError::Other(m) if m.contains("überschreiben: main.pdf")),
            "{err:?}"
        );
        assert_eq!(read(&a.join("main.pdf")).as_deref(), Some("meine PDF\n"));
        assert_eq!(read(&a.join("main.typ")).as_deref(), Some("lokal\n"));
        assert_eq!(sb.head(&a), head);
        assert_eq!(changes(&sb.status(&a)), expect(&[("main.typ", Modified)]));
    }

    #[test]
    fn pull_after_diverging_commits_makes_a_merge_commit() {
        let (sb, _, a, b) = two_clones("pull-merge", &[("main.typ", b"a\n"), ("other.typ", b"o\n"), ("z.typ", b"z\n")]);
        // The user's own setting must not stop Typisch's merge.
        sb.run(&a, &["config", "merge.ff", "only"]);
        write(&a.join("main.typ"), "a2\n");
        let local = sb.commit(&a, "Lokal");
        write(&a.join("z.typ"), "z lokal\n");
        write(&a.join("notiz.typ"), "n\n");
        write(&b.join("other.typ"), "o2\n");
        let upstream = sb.commit_push(&b, "Fremd");

        assert_eq!(pull(&sb.git(&a), &[], &me()).unwrap().commits, 1);
        let parents = sb.run(&a, &["rev-list", "--parents", "-n", "1", "HEAD"]);
        let parents: Vec<&str> = parents.split_whitespace().skip(1).collect();
        assert_eq!(parents, vec![local.as_str(), upstream.as_str()]);
        assert_eq!(sb.blob(&a, "HEAD:main.typ"), b"a2\n");
        assert_eq!(sb.blob(&a, "HEAD:other.typ"), b"o2\n");
        assert_eq!(
            sb.run(&a, &["log", "-1", "--format=%an <%ae>"]),
            "Erika Mustermann <erika@example.com>"
        );
        assert_eq!(read(&a.join("z.typ")).as_deref(), Some("z lokal\n"));
        let st = sb.status(&a);
        assert_eq!(changes(&st), expect(&[("notiz.typ", Added), ("z.typ", Modified)]));
        assert_eq!((st.ahead, st.behind), (2, 0));
        assert_tidy(&sb, &a);
    }

    /// A binary file and a new file committed differently on both sides.
    fn diverged_conflict(name: &str) -> (Sandbox, PathBuf) {
        let (sb, _, a, b) = two_clones(name, &[("bild.png", PNG)]);
        write(&a.join("bild.png"), PNG_HERE);
        write(&a.join("neu.typ"), "hier\n");
        sb.commit(&a, "Lokal");
        write(&b.join("bild.png"), PNG_THERE);
        write(&b.join("neu.typ"), "dort\n");
        sb.commit_push(&b, "Fremd");
        (sb, a)
    }

    #[test]
    fn pull_after_diverging_commits_settles_their_conflicts() {
        let (sb, a) = diverged_conflict("pull-merge-remote");
        let plan = plan_pull(&sb.git(&a)).unwrap();
        assert_eq!(
            plan.questions,
            vec![Question::BinaryChanged("bild.png".into()), Question::AddedBoth("neu.typ".into())]
        );
        let answers = vec![("bild.png".to_string(), Choice::Remote), ("neu.typ".to_string(), Choice::Remote)];
        pull(&sb.git(&a), &answers, &me()).unwrap();
        // The merge commit holds the chosen versions (not a union of both new files).
        assert_eq!(sb.blob(&a, "HEAD:neu.typ"), b"dort\n");
        assert_eq!(sb.blob(&a, "HEAD:bild.png"), PNG_THERE);
        assert!(sb.status(&a).changes.is_empty());
        assert_tidy(&sb, &a);

        let (sb, a) = diverged_conflict("pull-merge-local");
        pull(&sb.git(&a), &[], &me()).unwrap();
        assert_eq!(sb.blob(&a, "HEAD:neu.typ"), b"hier\n");
        assert_eq!(sb.blob(&a, "HEAD:bild.png"), PNG_HERE);
        assert!(sb.status(&a).changes.is_empty());
        assert_tidy(&sb, &a);
    }

    #[test]
    fn pull_after_diverging_commits_keeps_uncommitted_answers_out_of_the_merge() {
        let (sb, _, a, b) = two_clones("pull-merge-uncommitted", &[("bild.png", PNG), ("x.typ", b"x\n")]);
        write(&a.join("x.typ"), "x2\n");
        sb.commit(&a, "Lokal");
        write(&a.join("bild.png"), PNG_HERE);
        write(&b.join("bild.png"), PNG_THERE);
        sb.commit_push(&b, "Fremd");

        pull(&sb.git(&a), &[("bild.png".into(), Choice::Local)], &me()).unwrap();
        // The merge commit does not quietly revert the remote change; the local version stays
        // an uncommitted change on top.
        assert_eq!(sb.blob(&a, "HEAD:bild.png"), PNG_THERE);
        assert_eq!(std::fs::read(a.join("bild.png")).unwrap(), PNG_HERE);
        assert_eq!(changes(&sb.status(&a)), expect(&[("bild.png", Modified)]));
        assert_tidy(&sb, &a);
    }

    /// A folder linked to an empty remote, then someone else pushes first.
    fn unborn_scene(name: &str) -> (Sandbox, PathBuf, String) {
        let sb = Sandbox::new(name);
        let url = sb.bare("remote.git");
        let project = sb.path("proj");
        write(&project.join("a.typ"), "lokal\n");
        write(&project.join("eigen.typ"), "nur hier\n");
        link(&sb.git(&project), &url, "main").unwrap();
        let other = sb.clone(&url, "other");
        write(&other.join("a.typ"), "fremd\n");
        write(&other.join("README.md"), "# Buch\n");
        let upstream = sb.commit_push(&other, "Fremd");
        (sb, project, upstream)
    }

    #[test]
    fn pull_into_an_unborn_branch_adopts_the_remote_commits() {
        let (sb, project, upstream) = unborn_scene("pull-unborn-remote");
        let plan = plan_pull(&sb.git(&project)).unwrap();
        assert_eq!(plan.behind, 1);
        assert_eq!(plan.questions, vec![Question::AddedBoth("a.typ".into())]);

        let out = pull(&sb.git(&project), &[("a.typ".into(), Choice::Remote)], &me()).unwrap();
        assert_eq!((out.commits, out.took_remote.clone()), (1, strings(&["a.typ"])));
        assert_eq!(sb.head(&project), upstream);
        assert_eq!(read(&project.join("a.typ")).as_deref(), Some("fremd\n"));
        assert_eq!(read(&project.join("README.md")).as_deref(), Some("# Buch\n"));
        let st = sb.status(&project);
        assert!(!st.unborn);
        assert_eq!(changes(&st), expect(&[(".gitignore", Added), ("eigen.typ", Added)]));
        assert_tidy(&sb, &project);

        let (sb, project, upstream) = unborn_scene("pull-unborn-local");
        let out = pull(&sb.git(&project), &[], &me()).unwrap();
        assert_eq!(out.kept_local, strings(&["a.typ"]));
        assert_eq!(sb.head(&project), upstream);
        assert_eq!(read(&project.join("a.typ")).as_deref(), Some("lokal\n"));
        assert_eq!(read(&project.join("README.md")).as_deref(), Some("# Buch\n"));
        assert_eq!(
            changes(&sb.status(&project)),
            expect(&[(".gitignore", Added), ("a.typ", Modified), ("eigen.typ", Added)])
        );
        assert_tidy(&sb, &project);
    }

    #[test]
    fn pull_in_a_project_subfolder_merges_the_whole_repository() {
        let (sb, _, a, b) = two_clones(
            "pull-sub",
            &[("buch/kapitel.typ", b"eins\nzwei\ndrei\n"), ("README.md", b"r1\nr2\nr3\n")],
        );
        write(&b.join("buch/kapitel.typ"), "eins\nREMOTE\ndrei\n");
        write(&b.join("README.md"), "r1\nREMOTE\nr3\n");
        write(&b.join("buch/neu.typ"), "neu\n");
        let upstream = sb.commit_push(&b, "Fremd");
        write(&a.join("buch/kapitel.typ"), "eins\nLOKAL\ndrei\n");
        write(&a.join("README.md"), "r1\nLOKAL\nr3\n");
        let project = a.join("buch");

        let plan = plan_pull(&sb.git(&project)).unwrap();
        assert!(plan.questions.is_empty());
        assert_eq!(plan.merged_text, strings(&["README.md", "buch/kapitel.typ"]), "repo-root relative");
        let out = pull(&sb.git(&project), &[], &me()).unwrap();
        assert_eq!(out.merged_text, plan.merged_text);
        assert_eq!(sb.head(&a), upstream);
        for file in [a.join("buch/kapitel.typ"), a.join("README.md")] {
            let text = read(&file).unwrap();
            assert!(text.contains("REMOTE\n") && text.contains("LOKAL\n"), "{text}");
        }
        assert_eq!(read(&project.join("neu.typ")).as_deref(), Some("neu\n"));
        assert_eq!(changes(&sb.status(&project)), expect(&[("kapitel.typ", Modified)]));
        assert_tidy(&sb, &a);
    }

    #[cfg(unix)]
    #[test]
    fn pull_restores_everything_when_the_merge_fails() {
        let (sb, _, a, b) = two_clones("pull-fail", &[("main.typ", b"a\n"), ("text.typ", b"t\n"), ("weg.typ", b"w\n")]);
        write(&a.join("main.typ"), "a2\n");
        let local = sb.commit(&a, "Lokal");
        write(&a.join("text.typ"), "t lokal\n");
        write(&a.join("notiz.typ"), "n\n");
        remove(&a.join("weg.typ"));
        write(&b.join("other.typ"), "o\n");
        sb.commit_push(&b, "Fremd");
        // The merge commit is refused, after the merge itself has changed the folder.
        hook(&a, "pre-commit", "echo 'Hook sagt nein' >&2; exit 1");
        let before = changes(&sb.status(&a));

        let err = pull(&sb.git(&a), &[], &me()).unwrap_err();
        let message = err.to_string();
        assert!(message.contains("Hook sagt nein"), "{message}");
        assert!(!message.contains("gesichert"), "recovery succeeded: {message}");
        assert_eq!(sb.head(&a), local);
        assert_eq!(read(&a.join("text.typ")).as_deref(), Some("t lokal\n"));
        assert_eq!(read(&a.join("notiz.typ")).as_deref(), Some("n\n"));
        assert!(!a.join("weg.typ").exists());
        assert!(!a.join("other.typ").exists());
        assert_eq!(changes(&sb.status(&a)), before);
        assert_eq!(sb.status(&a).in_progress, None);
        assert_eq!(sb.run(&a, &["stash", "list"]), "");
        assert!(!a.join(".git/MERGE_HEAD").exists());
        assert!(!a.join(".git/typisch-attributes").exists());
    }

    #[test]
    fn recovery_puts_the_snapshot_back_over_leftovers() {
        let (sb, _, a, b) = two_clones("recover", &[("main.typ", b"1\n"), ("weg.typ", b"w\n")]);
        write(&b.join("main.typ"), "fremd\n");
        let upstream = sb.commit_push(&b, "Fremd");
        let head = sb.head(&a);
        write(&a.join("main.typ"), "lokal\n");
        write(&a.join("notiz.typ"), "n\n");
        remove(&a.join("weg.typ"));
        let before = changes(&sb.status(&a));
        let g = sb.git(&a);
        fetch(&g).unwrap();
        let (repo, _) = load(&g).unwrap().unwrap();
        let backup = snapshot(&g, &repo, &me(), "Pull").unwrap().expect("local work to keep");

        // A pull that got half-way: HEAD moved on, and a half-written file stands where the
        // snapshot has one (so a plain `stash apply` refuses).
        sb.run(&a, &["reset", "-q", "--hard", &upstream]);
        write(&a.join("notiz.typ"), "halb geschrieben\n");
        let err = recover(&g, &repo, &head, Some(&backup), GitError::Other("kaputt".into()));
        assert!(matches!(&err, GitError::Other(m) if m == "kaputt"), "{err:?}");
        assert_eq!(sb.head(&a), head);
        assert_eq!(read(&a.join("main.typ")).as_deref(), Some("lokal\n"));
        assert_eq!(read(&a.join("notiz.typ")).as_deref(), Some("n\n"));
        assert!(!a.join("weg.typ").exists());
        assert_eq!(changes(&sb.status(&a)), before);

        // When even that fails, the message says where the work is kept.
        let err = recover(&g, &repo, "0123456789abcdef", Some(&backup), GitError::Other("kaputt".into()));
        let message = err.to_string();
        assert!(message.starts_with("kaputt\n"), "{message}");
        assert!(message.contains(&format!("git stash apply {}", backup.name)), "{message}");
        assert_eq!(sb.run(&a, &["rev-parse", &backup.name]), backup.sha);
    }

    #[test]
    fn pull_refuses_an_unrelated_history() {
        let (sb, _, a, b) = two_clones("pull-unrelated", &[("main.typ", b"1\n")]);
        sb.run(&b, &["checkout", "-q", "--orphan", "neu"]);
        write(&b.join("main.typ"), "anders\n");
        sb.commit(&b, "Neuer Anfang");
        // Setting up the scene only: Typisch itself never forces.
        sb.run(&b, &["push", "-q", "--force", "origin", "HEAD:refs/heads/main"]);
        let head = sb.head(&a);
        assert!(matches!(plan_pull(&sb.git(&a)), Err(GitError::Unrelated { branch }) if branch == "main"));
        assert!(matches!(pull(&sb.git(&a), &[], &me()), Err(GitError::Unrelated { .. })));
        assert_eq!(sb.head(&a), head);
        assert_eq!(read(&a.join("main.typ")).as_deref(), Some("1\n"));
    }

    // -- 6. reset -----------------------------------------------------------------------------

    /// The one backup ref a reset left in `repo`.
    fn backup_ref(sb: &Sandbox, repo: &Path) -> String {
        let refs = sb.run(repo, &["for-each-ref", "--format=%(refname)", "refs/typisch"]);
        assert_eq!(refs.lines().count(), 1, "{refs}");
        refs
    }

    #[test]
    fn reset_drops_changes_that_a_pull_calls_up_to_date() {
        let (sb, _, a, _) = two_clones(
            "reset-dirty",
            &[(".gitignore", b"*.pdf\n"), ("main.typ", b"1\n"), ("bild.png", PNG)],
        );
        write(&a.join("main.typ"), "kaputt\n");
        remove(&a.join("bild.png"));
        write(&a.join("notizen.typ"), "Notizen\n");
        write(&a.join("main.pdf"), "PDF\n");
        let head = sb.head(&a);
        // Nothing new in the repository: a pull leaves all of this as it is.
        assert_eq!(pull(&sb.git(&a), &[], &me()).unwrap().commits, 0);
        assert_eq!(read(&a.join("main.typ")).as_deref(), Some("kaputt\n"));

        let out = reset(&sb.git(&a), &me()).unwrap();
        assert!(out.changed);
        assert_eq!(sb.head(&a), head);
        assert_eq!(read(&a.join("main.typ")).as_deref(), Some("1\n"));
        assert_eq!(std::fs::read(a.join("bild.png")).unwrap(), PNG);
        assert!(!a.join("notizen.typ").exists(), "new files go");
        assert_eq!(read(&a.join("main.pdf")).as_deref(), Some("PDF\n"), "ignored files stay");
        assert!(sb.status(&a).changes.is_empty());
        assert!(sb.output(&a, &["diff", "--cached", "--quiet"]).ok());
        assert!(!a.join(".git/typisch-pull").exists());
        // Everything dropped is in the backup.
        let backup = backup_ref(&sb, &a);
        assert_eq!(out.backup.as_deref(), Some(backup.as_str()));
        assert_eq!(sb.blob(&a, &format!("{backup}:main.typ")), b"kaputt\n");
        assert_eq!(sb.blob(&a, &format!("{backup}:notizen.typ")), b"Notizen\n");
        assert!(!sb.output(&a, &["cat-file", "-e", &format!("{backup}:bild.png")]).ok());
    }

    #[test]
    fn reset_drops_local_commits_and_takes_the_remote_ones() {
        let (sb, _, a, b) = two_clones("reset-diverged", &[("main.typ", b"1\n")]);
        write(&a.join("main.typ"), "hier\n");
        let mine = sb.commit(&a, "Hier");
        write(&a.join("main.typ"), "hier, noch offen\n");
        write(&b.join("main.typ"), "dort\n");
        write(&b.join("kapitel.typ"), "k\n");
        let upstream = sb.commit_push(&b, "Dort");

        let out = reset(&sb.git(&a), &me()).unwrap();
        assert!(out.changed);
        assert_eq!(sb.head(&a), upstream);
        assert_eq!(read(&a.join("main.typ")).as_deref(), Some("dort\n"));
        assert_eq!(read(&a.join("kapitel.typ")).as_deref(), Some("k\n"));
        let st = sb.status(&a);
        assert!(st.changes.is_empty());
        assert_eq!((st.ahead, st.behind), (0, 0));
        // The backup holds the open edit, on top of the dropped commit.
        let backup = out.backup.unwrap();
        assert_eq!(sb.blob(&a, &format!("{backup}:main.typ")), b"hier, noch offen\n");
        assert_eq!(sb.run(&a, &["rev-parse", &format!("{backup}^1")]), mine);
    }

    #[test]
    fn reset_of_committed_work_alone_keeps_the_commits() {
        let (sb, _, a, _) = two_clones("reset-ahead", &[("main.typ", b"1\n")]);
        let upstream = sb.head(&a);
        write(&a.join("main.typ"), "hier\n");
        let mine = sb.commit(&a, "Hier");

        let out = reset(&sb.git(&a), &me()).unwrap();
        assert_eq!(sb.head(&a), upstream);
        assert_eq!(read(&a.join("main.typ")).as_deref(), Some("1\n"));
        let backup = backup_ref(&sb, &a);
        assert_eq!(out.backup.as_deref(), Some(backup.as_str()));
        assert_eq!(sb.run(&a, &["rev-parse", &backup]), mine);
    }

    #[test]
    fn reset_without_differences_changes_nothing() {
        let (sb, _, a, b) = two_clones("reset-same", &[("main.typ", b"1\n")]);
        assert_eq!(reset(&sb.git(&a), &me()).unwrap(), ResetOutcome::default());
        assert_eq!(sb.run(&a, &["for-each-ref", "refs/typisch"]), "");

        // Only behind: nothing is dropped, so there is nothing to back up.
        write(&b.join("main.typ"), "2\n");
        let upstream = sb.commit_push(&b, "zwei");
        let out = reset(&sb.git(&a), &me()).unwrap();
        assert_eq!((out.changed, out.backup), (true, None));
        assert_eq!(sb.head(&a), upstream);
        assert_eq!(read(&a.join("main.typ")).as_deref(), Some("2\n"));
    }

    #[test]
    fn reset_follows_a_rewritten_history() {
        let (sb, _, a, b) = two_clones("reset-unrelated", &[("main.typ", b"1\n")]);
        sb.run(&b, &["checkout", "-q", "--orphan", "neu"]);
        write(&b.join("main.typ"), "anders\n");
        let upstream = sb.commit(&b, "Neuer Anfang");
        // Setting up the scene only: Typisch itself never forces.
        sb.run(&b, &["push", "-q", "--force", "origin", "HEAD:refs/heads/main"]);
        let before = sb.head(&a);
        assert!(matches!(pull(&sb.git(&a), &[], &me()), Err(GitError::Unrelated { .. })));

        let out = reset(&sb.git(&a), &me()).unwrap();
        assert_eq!(sb.head(&a), upstream);
        assert_eq!(read(&a.join("main.typ")).as_deref(), Some("anders\n"));
        assert_eq!(sb.run(&a, &["rev-parse", &out.backup.unwrap()]), before);
    }

    #[test]
    fn reset_backs_up_an_ignored_file_it_overwrites() {
        let (sb, _, a, b) = two_clones("reset-ignored", &[(".gitignore", b"*.pdf\n"), ("main.typ", b"1\n")]);
        write(&a.join("main.pdf"), "meine PDF\n");
        write(&a.join("anhang.pdf"), "Anhang\n");
        write(&b.join("main.pdf"), "fremde PDF\n");
        sb.run(&b, &["add", "-f", "main.pdf"]);
        sb.commit_push(&b, "PDF eingecheckt");

        let out = reset(&sb.git(&a), &me()).unwrap();
        assert_eq!(read(&a.join("main.pdf")).as_deref(), Some("fremde PDF\n"));
        assert_eq!(
            read(&a.join("anhang.pdf")).as_deref(),
            Some("Anhang\n"),
            "not in the way: untouched"
        );
        let backup = out.backup.unwrap();
        assert_eq!(sb.blob(&a, &format!("{backup}:main.pdf")), b"meine PDF\n");
        assert!(sb.status(&a).changes.is_empty());
    }

    #[test]
    fn reset_refuses_to_drop_changes_outside_the_project_folder() {
        let (sb, _, a, _) = two_clones("reset-outside", &[("arbeit/main.typ", b"1\n"), ("README.md", b"lies\n")]);
        let project = a.join("arbeit");
        write(&project.join("main.typ"), "kaputt\n");
        write(&a.join("README.md"), "geändert\n");

        let err = reset(&sb.git(&project), &me()).unwrap_err();
        assert!(
            matches!(&err, GitError::Other(m) if m.contains("außerhalb des Projektordners: README.md")),
            "{err:?}"
        );
        assert_eq!(read(&project.join("main.typ")).as_deref(), Some("kaputt\n"));
        assert_eq!(read(&a.join("README.md")).as_deref(), Some("geändert\n"));
        assert_eq!(sb.run(&a, &["for-each-ref", "refs/typisch"]), "");

        // With the rest of the repository clean, the project folder is reset.
        sb.run(&a, &["checkout", "-q", "--", "README.md"]);
        assert!(reset(&sb.git(&project), &me()).unwrap().changed);
        assert_eq!(read(&project.join("main.typ")).as_deref(), Some("1\n"));
    }

    #[test]
    fn reset_needs_a_branch_in_the_repository() {
        let sb = Sandbox::new("reset-empty");
        let url = sb.bare("remote.git");
        let dir = sb.path("Projekt");
        write(&dir.join("main.typ"), "1\n");
        link(&sb.git(&dir), &url, "main").unwrap();

        let err = reset(&sb.git(&dir), &me()).unwrap_err();
        assert!(matches!(&err, GitError::Other(m) if m.contains("noch nicht")), "{err:?}");
        assert_eq!(read(&dir.join("main.typ")).as_deref(), Some("1\n"));
    }

    #[test]
    fn status_restores_after_an_interrupted_reset() {
        let (sb, _, a, b) = two_clones("reset-journal", &[("main.typ", b"1\n")]);
        write(&b.join("main.typ"), "dort\n");
        sb.commit_push(&b, "Dort");
        write(&a.join("main.typ"), "hier\n");
        let before = sb.commit(&a, "Hier");
        sb.run(&a, &["fetch", "-q"]);
        // A reset of a clean folder wrote its journal and moved the branch, then died.
        let journal = sb.output(&a, &["rev-parse", "--git-path", "typisch-pull"]).text();
        std::fs::write(a.join(journal.trim()), format!("{before}\n-\n-\n")).unwrap();
        sb.run(&a, &["reset", "-q", "--hard", "origin/main"]);

        assert!(sb.status(&a).changes.is_empty());
        assert_eq!(sb.head(&a), before);
        assert_eq!(read(&a.join("main.typ")).as_deref(), Some("hier\n"));
    }

    // -- 7. clone -----------------------------------------------------------------------------

    #[test]
    fn clone_into_a_new_folder() {
        let (sb, url, upstream) = seeded("clone-new", &[("main.typ", b"= Buch\n")]);
        let dest = sb.path("Projekte/Mein Buch");
        assert_eq!(clone_repo(&sb.git(&sb.root), &url, None, &dest).unwrap(), dest);
        assert_eq!(read(&dest.join("main.typ")).as_deref(), Some("= Buch\n"));
        let st = sb.status(&dest);
        assert!(st.linked() && st.remote_branch_exists && !st.unborn);
        assert_eq!(st.branch.as_deref(), Some("main"));
        assert_eq!(st.remote.as_deref(), Some("origin"));
        assert_eq!(sb.head(&dest), upstream);
    }

    #[test]
    fn clone_a_branch() {
        let (sb, url, _) = seeded("clone-branch", &[("main.typ", b"= Buch\n")]);
        let seed = sb.path("seed");
        sb.run(&seed, &["checkout", "-q", "-b", "entwurf"]);
        write(&seed.join("entwurf.typ"), "e\n");
        sb.commit(&seed, "Entwurf");
        sb.run(&seed, &["push", "-q", "origin", "entwurf"]);

        let dest = sb.path("entwurf");
        clone_repo(&sb.git(&sb.root), &url, Some(" entwurf "), &dest).unwrap();
        let st = sb.status(&dest);
        assert_eq!(st.branch.as_deref(), Some("entwurf"));
        assert_eq!(st.remote_branch.as_deref(), Some("entwurf"));
        assert!(dest.join("entwurf.typ").exists());
    }

    #[test]
    fn clone_into_a_folder_that_is_not_empty() {
        let (sb, url, _) = seeded("clone-occupied", &[("main.typ", b"= Buch\n")]);
        let dest = sb.path("belegt");
        write(&dest.join("notiz.txt"), "meins\n");
        let err = clone_repo(&sb.git(&sb.root), &url, None, &dest).unwrap_err();
        assert!(matches!(&err, GitError::DestinationExists(p) if *p == dest), "{err:?}");
        assert_eq!(read(&dest.join("notiz.txt")).as_deref(), Some("meins\n"));
        assert!(!dest.join(".git").exists());
    }

    #[test]
    fn clone_an_empty_repository_sets_the_upstream() {
        let sb = Sandbox::new("clone-empty");
        let url = sb.bare("remote.git");

        let draft = sb.path("entwurf");
        clone_repo(&sb.git(&sb.root), &url, Some("entwurf"), &draft).unwrap();
        let st = sb.status(&draft);
        assert!(st.linked() && st.unborn);
        assert_eq!(st.branch.as_deref(), Some("entwurf"));
        assert_eq!(st.remote_branch.as_deref(), Some("entwurf"));

        let dest = sb.path("leer");
        clone_repo(&sb.git(&sb.root), &url, None, &dest).unwrap();
        let st = sb.status(&dest);
        assert!(st.linked() && st.unborn && !st.remote_branch_exists);
        assert_eq!(st.remote_branch.as_deref(), Some("main"));
        write(&dest.join("main.typ"), "= Neu\n");
        assert_eq!(push(&sb.git(&dest), "Anfang", &me()).unwrap().commits, 1);
        assert_eq!(sb.run(&sb.path("remote.git"), &["rev-parse", "main"]), sb.head(&dest));
    }

    #[test]
    fn failed_clones_leave_nothing_behind() {
        let (sb, url, _) = seeded("clone-fail", &[("main.typ", b"= Buch\n")]);
        let g = sb.git(&sb.root);

        let dest = sb.path("neu");
        let err = clone_repo(&g, &url, Some("gibt-es-nicht"), &dest).unwrap_err();
        assert!(matches!(&err, GitError::Other(m) if m.contains("gibt-es-nicht")), "{err:?}");
        assert!(!dest.exists());

        let missing = format!("file://{}", sb.path("fehlt.git").display());
        assert!(matches!(clone_repo(&g, &missing, None, &dest), Err(GitError::RepoNotFound(_))));
        assert!(!dest.exists());

        // A folder that was there (and empty) before stays, empty.
        let empty = sb.path("leer");
        std::fs::create_dir_all(&empty).unwrap();
        assert!(clone_repo(&g, &missing, None, &empty).is_err());
        assert!(empty.is_dir() && std::fs::read_dir(&empty).unwrap().next().is_none());
        assert!(matches!(clone_repo(&g, " ", None, &empty), Err(GitError::Other(m)) if m == no_url()));
        let empty_remote = sb.bare("leer.git");
        assert!(matches!(clone_repo(&g, &empty_remote, Some("a..b"), &dest), Err(GitError::Other(m)) if m.contains("a..b")));
        assert!(!dest.exists());
    }

    // -- 8. pure helpers ----------------------------------------------------------------------

    #[test]
    fn addresses_are_taken_apart() {
        let https = "https://github.com/typst/typst.git";
        assert_eq!(host_of(https).as_deref(), Some("github.com"));
        assert!(!is_ssh_url(https));
        assert_eq!(web_url(https).as_deref(), Some("https://github.com/typst/typst"));
        assert_eq!(display_url(https), "github.com/typst/typst");
        assert_eq!(repo_name(https), "typst");

        let login = "https://erika:geheim@GitLab.com:8443/gruppe/unter/buch.git/";
        assert_eq!(host_of(login).as_deref(), Some("gitlab.com"));
        assert_eq!(web_url(login).as_deref(), Some("https://gitlab.com:8443/gruppe/unter/buch"));
        assert_eq!(display_url(login), "gitlab.com:8443/gruppe/unter/buch");
        assert!(!display_url(login).contains("geheim"));
        assert_eq!(repo_name(login), "buch");

        let scp = "git@github.com:erika/mein-buch.git";
        assert_eq!(host_of(scp).as_deref(), Some("github.com"));
        assert!(is_ssh_url(scp));
        assert_eq!(web_url(scp).as_deref(), Some("https://github.com/erika/mein-buch"));
        assert_eq!(display_url(scp), "github.com/erika/mein-buch");
        assert_eq!(repo_name(scp), "mein-buch");

        let ssh = "ssh://git@git.example.org:2222/team/buch";
        assert_eq!(host_of(ssh).as_deref(), Some("git.example.org"));
        assert!(is_ssh_url(ssh));
        assert_eq!(web_url(ssh), None, "unknown ssh hosts have no known web page");
        assert_eq!(display_url(ssh), "git.example.org/team/buch");
        assert_eq!(repo_name(ssh), "buch");

        let path = "/Users/erika/Repos/Mein Buch.git";
        assert_eq!(host_of(path), None);
        assert!(!is_ssh_url(path));
        assert_eq!(web_url(path), None);
        assert_eq!(display_url(path), path);
        assert_eq!(repo_name(path), "Mein Buch");

        let file = "file:///Users/erika/Repos/buch.git";
        assert_eq!(host_of(file), None);
        assert_eq!(web_url(file), None);
        assert_eq!(display_url(file), "/Users/erika/Repos/buch.git");
        assert_eq!(repo_name(file), "buch");

        assert_eq!(repo_name(r"C:\Repos\buch"), "buch");
        assert_eq!(repo_name(""), "Projekt");
        assert_eq!(repo_name("https://github.com/"), "Projekt");
        assert_eq!(
            token_url("github.com"),
            Some("https://github.com/settings/tokens/new?scopes=repo&description=Typisch")
        );
        assert!(token_url("gitlab.com").unwrap().contains("write_repository"));
        assert_eq!(token_url("git.example.org"), None);
    }

    #[test]
    fn relative_times_are_short() {
        let day = 24 * 3600;
        let cases = [
            (0, "jetzt"),
            (59, "jetzt"),
            (60, "1 min"),
            (36 * 60, "36 min"),
            (3599, "59 min"),
            (3600, "1 h"),
            (day - 1, "23 h"),
            (day, "1 T"),
            (13 * day, "13 T"),
            (14 * day, "2 W"),
            (42 * day, "6 W"),
        ];
        for (secs, text) in cases {
            assert_eq!(relative_time(secs), text, "{secs} s");
        }
    }

    #[test]
    fn git_messages_are_classified() {
        let dir = Path::new("/tmp/x");
        let url = "https://github.com/a/b.git";
        let https = Some(url);
        let ssh = Some("git@github.com:a/b.git");
        let auth =
            |e: GitError, want_ssh: bool| matches!(e, GitError::AuthRequired { host, ssh, .. } if host == "github.com" && ssh == want_ssh);
        assert!(auth(
            classify(
                "fatal: Authentication failed for 'https://github.com/a/b.git/'",
                Some(128),
                https,
                true,
                dir
            ),
            false
        ));
        assert!(auth(
            classify(
                "fatal: could not read Username for 'https://github.com': terminal prompts disabled",
                Some(128),
                https,
                false,
                dir
            ),
            false
        ));
        assert!(auth(
            classify(
                "fatal: unable to access '…': The requested URL returned error: 403",
                Some(128),
                https,
                true,
                dir
            ),
            false
        ));
        assert!(auth(
            classify("git@github.com: Permission denied (publickey).", Some(128), ssh, false, dir),
            true
        ));
        // GitHub says "not found" to anonymous requests for private repositories.
        let missing = "remote: Repository not found.\nfatal: repository 'https://github.com/a/b.git/' not found";
        assert!(auth(classify(missing, Some(128), https, false, dir), false));
        assert!(matches!(classify(missing, Some(128), https, true, dir), GitError::RepoNotFound(u) if u == url));
        assert!(matches!(
            classify("ERROR: Repository not found.", Some(128), ssh, false, dir),
            GitError::RepoNotFound(_)
        ));
        let offline = "fatal: unable to access 'https://github.com/a/b.git/': Could not resolve host: github.com";
        assert!(
            matches!(classify(offline, Some(128), https, false, dir), GitError::Other(m) if m.contains("Keine Verbindung zu github.com"))
        );
        // "403" only as a word.
        let branch = "fatal: couldn't find remote ref refs/heads/projekt-2403";
        assert!(matches!(classify(branch, Some(128), https, false, dir), GitError::Other(_)));
        let dubious = "fatal: detected dubious ownership in repository at '/x'\nTo add an exception for this directory, call:\n\n\tgit config --global --add safe.directory /x";
        assert!(matches!(classify(dubious, Some(128), None, false, dir), GitError::Other(m) if m.contains("safe.directory /x")));
        // Local calls never turn into login questions.
        assert!(matches!(
            classify("error: 403 not found", Some(1), None, false, dir),
            GitError::Other(_)
        ));
        assert_eq!(tail("hint: a\n\nline 1\nhint: b\nline 2\n", Some(1)), "line 1\nline 2");
        assert_eq!(
            tail((1..=9).map(|n| format!("l{n}\n")).collect::<String>().as_str(), Some(1)),
            "l4\nl5\nl6\nl7\nl8\nl9"
        );
        assert_eq!(tail("", Some(128)), "Git ist mit Fehler 128 abgebrochen.");
    }

    #[test]
    fn porcelain_v2_is_parsed() {
        let raw = b"# branch.oid 1234\0# branch.head main\0# branch.upstream team/haupt/main\0# branch.ab +2 -3\0\
1 .M N... 100644 100644 100644 aaaa aaaa Mein Projekt/\xC3\x9Cber uns.typ\0\
2 R. N... 100644 100644 100644 aaaa aaaa R100 neuer Name.typ\0alter Name.typ\0\
u UU N... 100644 100644 100644 100644 aaaa bbbb cccc Konflikt Datei.typ\0\
1 D. N... 100644 000000 000000 aaaa 0000 weg.typ\0\
1 A. N... 000000 100644 100644 0000 aaaa neu.typ\0\
? Bilder/Logo 1.png\0";
        let p = parse_porcelain(raw);
        assert_eq!(p.head.as_deref(), Some("main"));
        assert!(!p.initial);
        assert_eq!(p.upstream.as_deref(), Some("team/haupt/main"));
        assert_eq!(p.ab, Some((2, 3)));
        assert_eq!(
            p.entries,
            vec![
                ("Mein Projekt/Über uns.typ".to_string(), Modified),
                ("neuer Name.typ".to_string(), Renamed),
                ("Konflikt Datei.typ".to_string(), Conflicted),
                ("weg.typ".to_string(), Deleted),
                ("neu.typ".to_string(), Added),
                ("Bilder/Logo 1.png".to_string(), Added),
            ]
        );
        let p = parse_porcelain(b"# branch.oid (initial)\0# branch.head (detached)\0");
        assert!(p.initial);
        assert_eq!(p.head, None);
        assert_eq!(p.ab, None);
    }

    #[test]
    fn raw_diffs_and_config_are_parsed() {
        let raw = b":100644 100644 aaaa bbbb M\0Pfad mit Leerzeichen.typ\0:000000 100755 0000 cccc A\0neu.sh\0";
        let deltas = parse_raw_diff(raw);
        assert_eq!(deltas.len(), 2);
        let d = &deltas["Pfad mit Leerzeichen.typ"];
        assert_eq!((d.status, d.old.as_str(), d.new.as_str()), ('M', "aaaa", "bbbb"));
        assert_eq!((deltas["neu.sh"].status, deltas["neu.sh"].new_mode.as_str()), ('A', "100755"));

        let config = Config::parse(b"user.name\nErika\0branch.Main.remote\nteam/haupt\0core.bare\0user.name\nZweite\0");
        assert_eq!(config.get("user.name"), Some("Zweite"), "the last value wins");
        assert_eq!(config.get("core.bare"), Some("true"));
        assert_eq!(config.get("branch.Main.remote"), Some("team/haupt"));
        assert_eq!(upstream_config(&config, "Main"), None, "no merge key");
        let linked = Config::parse(b"branch.Main.remote\nteam/haupt\0branch.Main.merge\nrefs/heads/entwurf\0");
        assert_eq!(upstream_config(&linked, "Main"), Some(("team/haupt".into(), "entwurf".into())));
        let local = Config::parse(b"branch.main.remote\n.\0branch.main.merge\nrefs/heads/x\0");
        assert_eq!(upstream_config(&local, "main"), None, "a local upstream is not a link");
    }

    #[test]
    fn askpass_answers_only_login_prompts() {
        let user = Some("erika".to_string());
        let pass = Some("token".to_string());
        let host = Some("github.com".to_string());
        assert_eq!(
            askpass_answer("Username for 'https://github.com': ", host.as_deref(), user.clone(), pass.clone()).as_deref(),
            Some("erika")
        );
        assert_eq!(
            askpass_answer(
                "Password for 'https://erika@github.com': ",
                host.as_deref(),
                user.clone(),
                pass.clone()
            )
            .as_deref(),
            Some("token")
        );
        // Host binding: answers only go to the host this job talks to.
        assert_eq!(
            askpass_answer(
                "Password for 'https://evil.example.com': ",
                host.as_deref(),
                user.clone(),
                pass.clone()
            ),
            None
        );
        assert_eq!(
            askpass_answer("Password for 'https://github.com': ", None, user.clone(), pass.clone()),
            None
        );
        assert_eq!(
            askpass_answer(
                "Are you sure you want to continue connecting (yes/no)? ",
                host.as_deref(),
                user.clone(),
                pass.clone()
            ),
            None
        );
        assert_eq!(
            askpass_answer("Password for 'https://github.com': ", host.as_deref(), user, None),
            None
        );
    }

    #[cfg(unix)]
    fn login() -> Credentials {
        Credentials {
            username: "nutzerin-4711".into(),
            password: "geheim-0815".into(),
        }
    }

    #[cfg(unix)]
    #[test]
    fn the_login_socket_answers_the_helper_and_then_is_gone() {
        use std::os::unix::fs::PermissionsExt;
        let server = AskpassServer::start(Some("example.com".into()), login()).unwrap();
        let socket = server.socket.clone();
        let dir = socket.parent().unwrap().to_path_buf();
        assert_eq!(std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777, 0o700);
        assert_eq!(
            ask_server(&socket, "Username for 'https://example.com': ").as_deref(),
            Some("nutzerin-4711")
        );
        // Asked again as often as git likes.
        for _ in 0..2 {
            assert_eq!(
                ask_server(&socket, "Password for 'https://nutzerin-4711@example.com': ").as_deref(),
                Some("geheim-0815")
            );
        }
        // Another host, another kind of question, a second prompt hidden behind a line break.
        assert_eq!(ask_server(&socket, "Password for 'https://evil.example': "), None);
        assert_eq!(ask_server(&socket, "Enter passphrase for key '/home/x/.ssh/id_ed25519': "), None);
        assert_eq!(
            ask_server(
                &socket,
                "Password for 'https://evil.example': \nPassword for 'https://example.com': "
            ),
            None
        );
        assert_eq!(ask_server(&socket, ""), None);
        drop(server);
        assert!(!socket.exists() && !dir.exists(), "nothing is left behind");
        assert_eq!(ask_server(&socket, "Password for 'https://example.com': "), None);
    }

    #[cfg(unix)]
    #[test]
    fn the_login_is_not_in_the_environment_of_hooks() {
        let (sb, _, a, _) = two_clones("login-env", &[("main.typ", b"1\n")]);
        write(&a.join("main.typ"), "2\n");
        let seen = sb.path("hook-env");
        hook(&a, "pre-push", &format!("env > '{}'", seen.display()));
        push(&sb.git(&a).with_creds(Some(login())), "Mein Stand", &me()).unwrap();
        let env = read(&seen).expect("the pre-push hook ran");
        assert!(!env.contains("geheim-0815") && !env.contains("nutzerin-4711"), "{env}");
        assert!(
            !env.contains("TYPISCH_GIT_PASSWORD") && !env.contains("TYPISCH_GIT_USERNAME"),
            "{env}"
        );
        // What the hook does inherit is the place to ask, which is gone once the push has returned.
        let socket = env
            .lines()
            .find_map(|line| line.strip_prefix("TYPISCH_GIT_ASKPASS_SOCKET="))
            .expect("a network call with a login offers it at a socket");
        assert!(!Path::new(socket).exists());
        // A call without a login, and a local call, name no socket at all.
        write(&a.join("main.typ"), "3\n");
        push(&sb.git(&a), "Ohne Anmeldung", &me()).unwrap();
        assert!(!read(&seen).unwrap().contains("TYPISCH_GIT_ASKPASS_SOCKET"));
    }

    #[test]
    fn messages_hide_embedded_passwords() {
        assert_eq!(
            redact_url("https://erika:geheim@github.com/a/b.git"),
            "https://erika@github.com/a/b.git"
        );
        assert_eq!(redact_url("https://erika@github.com/a/b.git"), "https://erika@github.com/a/b.git");
        assert_eq!(redact_url("https://github.com/a/b.git"), "https://github.com/a/b.git");
        assert_eq!(redact_url("git@github.com:a/b.git"), "git@github.com:a/b.git");
        assert_eq!(
            redact_url("fatal: unable to access 'https://erika:geheim@github.com/a/b/': 404"),
            "fatal: unable to access 'https://erika@github.com/a/b/': 404"
        );
        // Two addresses in one text, and one that ends without a path.
        assert_eq!(redact_url("https://a:pw@x/y https://b:pw@z"), "https://a@x/y https://b@z");
        let e = GitError::RepoNotFound("https://erika:geheim@github.com/a/b.git".into());
        assert_eq!(e.to_string(), "Repository nicht gefunden: https://erika@github.com/a/b.git");
    }

    #[test]
    fn small_parts() {
        assert_eq!(parse_version("git version 2.54.0 (Apple Git-157)\n"), Some((2, 54)));
        assert_eq!(parse_version("git version 2.27.1"), Some((2, 27)));
        assert_eq!(parse_version("bash: git: command not found"), None);

        let creds = Credentials {
            username: "erika".into(),
            password: "sehr-geheim".into(),
        };
        let shown = format!("{creds:?}");
        assert!(shown.contains("erika") && !shown.contains("sehr-geheim"));
        let job = Job::Fetch {
            project: PathBuf::from("/p"),
            creds: Some(creds),
        };
        assert!(!format!("{job:?}").contains("sehr-geheim"));

        assert_eq!(
            GitError::NeedsPull { behind: 2 }.to_string(),
            "Im Repository gibt es neue Änderungen. Hole sie zuerst mit Pull und pushe dann erneut."
        );
        assert_eq!(
            GitError::InProgress("Merge".into()).to_string(),
            "Im Repository ist ein Merge nicht abgeschlossen. Schließe ihn zuerst im Terminal ab."
        );
        assert_eq!(
            GitError::AuthRequired {
                host: "github.com".into(),
                ssh: false,
                detail: "x".into()
            }
            .to_string(),
            "Anmeldung bei github.com erforderlich."
        );
        assert_eq!(
            GitError::Overlap {
                paths: strings(&["a.typ", "b.typ"])
            }
            .to_string(),
            "Diese Dateien gibt es hier und im Repository mit unterschiedlichem Inhalt: a.typ, b.typ. Wähle einen neuen Branch oder klone das Repository."
        );

        let a = Identity {
            name: " ".into(),
            email: "a@x".into(),
        };
        let b = Identity {
            name: "B".into(),
            email: "b@x".into(),
        };
        assert_eq!(
            a.or(&b),
            Identity {
                name: "B".into(),
                email: "a@x".into()
            }
        );
        assert!(!Identity::default().is_complete());
    }

    #[test]
    fn queued_jobs_are_coalesced() {
        let p = |name: &str| PathBuf::from(format!("/{name}"));
        let mut queue: VecDeque<Job> = VecDeque::from(vec![
            Job::Status { project: p("a") },
            Job::Fetch {
                project: p("a"),
                creds: None,
            },
            Job::Status { project: p("b") },
            Job::Status { project: p("a") },
            Job::PlanPull {
                project: p("a"),
                creds: None,
            },
            Job::Fetch {
                project: p("b"),
                creds: None,
            },
        ]);
        coalesce(&mut queue);
        let kinds: Vec<String> = queue
            .iter()
            .map(|job| match job {
                Job::Status { project } => format!("Status {}", project.display()),
                Job::Fetch { project, .. } => format!("Fetch {}", project.display()),
                Job::PlanPull { project, .. } => format!("PlanPull {}", project.display()),
                other => format!("{other:?}"),
            })
            .collect();
        assert_eq!(kinds, strings(&["Status /b", "Status /a", "PlanPull /a", "Fetch /b"]));
    }

    #[test]
    fn repository_roots_are_found_without_git() {
        let sb = Sandbox::new("find-root");
        let repo = sb.path("repo");
        std::fs::create_dir_all(repo.join(".git")).unwrap();
        std::fs::create_dir_all(repo.join("a/b")).unwrap();
        assert_eq!(find_repo_root(&repo.join("a/b")), Some(repo.clone()));
        assert_eq!(find_repo_root(&repo), Some(repo.clone()));
        // A worktree or submodule has a .git file.
        let worktree = sb.path("worktree");
        write(&worktree.join(".git"), "gitdir: /elsewhere\n");
        assert_eq!(find_repo_root(&worktree), Some(worktree.clone()));
        let plain = sb.path("plain");
        std::fs::create_dir_all(&plain).unwrap();
        assert_eq!(find_repo_root(&plain), None);
    }
}
