//! Reference sync: BibLaTeX files in a project that follow a Zotero library or collection,
//! like "Sync from Zotero" in the Typst web app.
//!
//! - The Zotero account (user id and name) and the list of synced files are kept in
//!   `<config dir>/typisch/reference-sync.json`. The API key itself goes into the macOS
//!   Keychain (elsewhere: a file in the config dir only the user can read).
//! - A background thread talks to Zotero. The UI thread writes the files, so a file that was
//!   just disconnected is never overwritten by a sync that was still under way.
//! - Synced files are read-only in the editor. They are refreshed when their project opens,
//!   every five minutes while it is open, and whenever the person asks.
//! - Disconnecting a file, or the whole Zotero account, turns the files into ordinary text files.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use eframe::egui;
use serde::{Deserialize, Serialize};

use crate::zotero::{self, Client, Fetched, LibraryRef, Target};

/// How often the files of the open project are refreshed.
pub const AUTO_SYNC: Duration = Duration::from_secs(5 * 60);

/// The name Zotero gives a person's own library.
pub const MY_LIBRARY: &str = "Meine Bibliothek";

const NOT_CONNECTED: &str = "Zotero ist nicht verbunden.";

/// A project folder and a file in it (relative, with `/`).
pub type FileKey = (PathBuf, String);

fn file_key(f: &SyncedFile) -> FileKey {
    (f.project.clone(), f.rel.clone())
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Account {
    pub user_id: u64,
    pub username: String,
    #[serde(default)]
    pub display_name: String,
}

impl Account {
    /// How the account is named in the interface.
    pub fn label(&self) -> &str {
        if self.display_name.trim().is_empty() {
            &self.username
        } else {
            &self.display_name
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SyncedFile {
    /// The project folder, as opened (canonical path).
    pub project: PathBuf,
    /// The file inside it.
    pub rel: String,
    pub target: Target,
    /// Where the entries come from, e.g. "Meine Bibliothek › Doktorarbeit".
    pub source: String,
    /// When the last sync succeeded (seconds since 1970).
    #[serde(default)]
    pub last_sync: Option<u64>,
    /// The Zotero library version the file content belongs to.
    #[serde(default)]
    pub version: Option<u64>,
    #[serde(default)]
    pub entries: usize,
    /// Item key → citation key of every entry, so keys stay stable (see `zotero::build_bib`).
    #[serde(default)]
    pub keys: BTreeMap<String, String>,
    /// Fingerprint of the text last written, to notice changes made outside Typisch.
    #[serde(default)]
    pub written: Option<u64>,
    /// Why the last sync failed.
    #[serde(default)]
    pub error: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Store {
    pub account: Option<Account>,
    pub files: Vec<SyncedFile>,
}

impl Store {
    /// `<config dir>/typisch/reference-sync.json`
    pub fn default_path() -> Option<PathBuf> {
        dirs::config_dir().map(|d| d.join("typisch").join("reference-sync.json"))
    }

    pub fn load(path: &Path) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        write_atomic(path, &serde_json::to_string_pretty(self).map_err(std::io::Error::other)?)
    }
}

// ---------------------------------------------------------------------------- key storage

mod keychain {
    #[cfg(target_os = "macos")]
    const SERVICE: &str = "app.typisch.Typisch.zotero";
    #[cfg(target_os = "macos")]
    const ACCOUNT: &str = "zotero-api-key";

    /// errSecItemNotFound
    #[cfg(target_os = "macos")]
    const NOT_FOUND: i32 = -25300;

    #[cfg(target_os = "macos")]
    pub fn load() -> Result<Option<String>, String> {
        match security_framework::passwords::get_generic_password(SERVICE, ACCOUNT) {
            Ok(bytes) => Ok(String::from_utf8(bytes).ok().filter(|k| !k.is_empty())),
            Err(e) if e.code() == NOT_FOUND => Ok(None),
            Err(e) => Err(e.to_string()),
        }
    }

    #[cfg(target_os = "macos")]
    pub fn save(key: &str) -> Result<(), String> {
        security_framework::passwords::set_generic_password(SERVICE, ACCOUNT, key.as_bytes()).map_err(|e| e.to_string())
    }

    #[cfg(target_os = "macos")]
    pub fn delete() -> Result<(), String> {
        match security_framework::passwords::delete_generic_password(SERVICE, ACCOUNT) {
            Ok(()) => Ok(()),
            Err(e) if e.code() == NOT_FOUND => Ok(()),
            Err(e) => Err(e.to_string()),
        }
    }

    #[cfg(not(target_os = "macos"))]
    fn path() -> Result<std::path::PathBuf, String> {
        dirs::config_dir()
            .map(|d| d.join("typisch").join("zotero-key"))
            .ok_or_else(|| "Kein Konfigurationsordner gefunden".to_string())
    }

    #[cfg(not(target_os = "macos"))]
    pub fn load() -> Result<Option<String>, String> {
        match std::fs::read_to_string(path()?) {
            Ok(k) => Ok(Some(k.trim().to_string()).filter(|k| !k.is_empty())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.to_string()),
        }
    }

    #[cfg(not(target_os = "macos"))]
    pub fn save(key: &str) -> Result<(), String> {
        use std::io::Write;
        let path = path()?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        let mut f = opts.open(&path).map_err(|e| e.to_string())?;
        // `mode` only applies to new files; an older file gets the same tight rights.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            f.set_permissions(std::fs::Permissions::from_mode(0o600))
                .map_err(|e| e.to_string())?;
        }
        f.write_all(key.as_bytes()).map_err(|e| e.to_string())
    }

    #[cfg(not(target_os = "macos"))]
    pub fn delete() -> Result<(), String> {
        match std::fs::remove_file(path()?) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e.to_string()),
        }
    }
}

// ---------------------------------------------------------------------------- background thread

/// A library with its collections, for choosing what to sync.
#[derive(Clone, Debug)]
pub struct LibraryTree {
    pub library: LibraryRef,
    pub name: String,
    /// In display order: parents before children, siblings by name.
    pub collections: Vec<CollectionRow>,
    /// Why the collections could not be listed (then the library can't be chosen).
    pub error: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CollectionRow {
    pub key: String,
    pub name: String,
    /// "Oberste › Darunter › Diese"
    pub path: String,
    pub depth: usize,
    pub items: u64,
    pub has_children: bool,
}

/// Account work: quick, and someone is waiting for it.
enum AccountJob {
    /// Check a key with Zotero. The UI thread stores it, if the answer is still wanted.
    Connect {
        key: String,
        generation: u64,
    },
    Disconnect,
    Libraries {
        user_id: u64,
    },
}

/// Exports can take a minute for a big library; they run on their own thread so the
/// account and the dialog's collection list never wait for them.
struct SyncJob {
    id: FileKey,
    /// Started by the person (then a Keychain read that failed before is tried again).
    manual: bool,
    target: Target,
    header: String,
    since: Option<u64>,
    keys: BTreeMap<String, String>,
}

enum Synced {
    Unchanged,
    Content { version: Option<u64>, built: zotero::Built },
}

enum Reply {
    Connected {
        generation: u64,
        result: Result<(Account, String), String>,
    },
    Libraries(Result<Vec<LibraryTree>, String>),
    Synced {
        id: FileKey,
        result: Result<Synced, String>,
    },
}

/// The API key as last read from the Keychain. (No `Debug`: the key must never end up in a log.)
#[derive(Clone, PartialEq, Eq)]
enum KeyState {
    Present(String),
    Missing,
    /// Reading failed (e.g. the person denied access). Automatic syncs do not ask again,
    /// so a refused Keychain prompt does not come back every five minutes.
    Failed(String),
}

/// The API key, shared by the UI thread and both background threads.
struct Keys {
    /// `None`: not read yet. The Keychain is only read once Zotero is needed, because
    /// reading may ask the person for permission. Only ever locked briefly.
    state: Mutex<Option<KeyState>>,
    /// Held by a background thread while it reads the Keychain (which may show a prompt), so
    /// only one prompt appears. The UI thread never takes it, so it never waits for a prompt.
    reading: Mutex<()>,
    /// Counts connects and disconnects: a Keychain read that started before one is stale.
    epoch: AtomicU64,
    /// Whether the system's key storage is used at all (never in tests or check pictures).
    system: bool,
}

type KeyCache = Arc<Keys>;

impl Keys {
    fn new(system: bool) -> Self {
        Self {
            state: Mutex::new(None),
            reading: Mutex::new(()),
            epoch: AtomicU64::new(0),
            system,
        }
    }

    fn get(&self) -> Option<KeyState> {
        self.state.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// Read the Keychain again when the key is next needed (another running Typisch changed it).
    fn forget(&self) {
        self.epoch.fetch_add(1, Ordering::SeqCst);
        *self.state.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }

    /// After a connect or disconnect.
    fn set(&self, state: KeyState) {
        self.epoch.fetch_add(1, Ordering::SeqCst);
        *self.state.lock().unwrap_or_else(|e| e.into_inner()) = Some(state);
    }
}

const NO_KEY: &str = "Kein Zotero-Schlüssel gespeichert. Bitte in den Einstellungen unter „Konten“ einen Schlüssel eintragen.";

fn stored_key(keys: &Keys, retry: bool) -> Result<String, String> {
    // Something the person asked for reads the Keychain again (a failed read may work now,
    // and another running Typisch may have replaced the key); automatic syncs use the cache.
    let needs_read = |s: &Option<KeyState>| s.is_none() || retry;
    let seen = keys.get();
    if keys.system && needs_read(&seen) {
        let _reading = keys.reading.lock().unwrap_or_else(|e| e.into_inner());
        // Unless the other thread has just read it (or the person connected meanwhile).
        if keys.get() == seen {
            let epoch = keys.epoch.load(Ordering::SeqCst);
            let loaded = match keychain::load() {
                Ok(Some(k)) => KeyState::Present(k),
                Ok(None) => KeyState::Missing,
                Err(e) => KeyState::Failed(format!(
                    "Der Zotero-Schlüssel konnte nicht aus dem Schlüsselbund gelesen werden: {e}"
                )),
            };
            let mut slot = keys.state.lock().unwrap_or_else(|e| e.into_inner());
            // A connect or disconnect while the Keychain was being read has the last word.
            if keys.epoch.load(Ordering::SeqCst) == epoch {
                *slot = Some(loaded);
            }
        }
    }
    match keys.get() {
        Some(KeyState::Present(k)) => Ok(k),
        Some(KeyState::Failed(e)) => Err(e),
        _ => Err(NO_KEY.into()),
    }
}

/// A client for the current key (made anew when the key changed).
fn client<'a>(keys: &Keys, api: &'a mut Option<(String, Client)>, retry: bool) -> Result<&'a mut Client, String> {
    let key = stored_key(keys, retry)?;
    if api.as_ref().map(|(k, _)| *k != key).unwrap_or(true) {
        // An empty key only exists in tests: requests without a key (public group libraries).
        let client = Client::new(Some(key.clone()).filter(|k| !k.is_empty()));
        *api = Some((key, client));
    }
    Ok(&mut api.as_mut().expect("just set").1)
}

/// Error text for a failed export, with a hint where a refused key can be replaced.
fn export_error(e: zotero::Error) -> String {
    match e {
        zotero::Error::Forbidden => format!("{e} Unter Einstellungen › Konten lässt sich der Schlüssel ersetzen."),
        e => e.to_string(),
    }
}

fn account_worker(rx: Receiver<AccountJob>, tx: Sender<Reply>, keys: KeyCache, ctx: egui::Context) {
    let mut api: Option<(String, Client)> = None;
    while let Ok(job) = rx.recv() {
        let reply = match job {
            AccountJob::Connect { key, generation } => {
                let mut c = Client::new(Some(key.clone()));
                let result = match c.key_info() {
                    Ok(info) if !info.library && !info.groups => Err(
                        "Dieser Schlüssel darf weder deine Bibliothek noch deine Gruppen lesen. Bitte einen Schlüssel mit Lesezugriff erstellen."
                            .to_string(),
                    ),
                    Ok(info) => Ok((Account { user_id: info.user_id, username: info.username, display_name: info.display_name }, key)),
                    Err(zotero::Error::Forbidden) => {
                        Err("Zotero kennt diesen API-Schlüssel nicht. Bitte prüfen, ob er vollständig eingefügt wurde.".to_string())
                    }
                    Err(e) => Err(e.to_string()),
                };
                Reply::Connected { generation, result }
            }
            // The key itself was already deleted on the UI thread; only the client goes.
            AccountJob::Disconnect => {
                api = None;
                continue;
            }
            AccountJob::Libraries { user_id } => Reply::Libraries(client(&keys, &mut api, true).and_then(|c| libraries(c, user_id))),
        };
        if tx.send(reply).is_err() {
            break;
        }
        ctx.request_repaint();
    }
}

fn sync_worker(rx: Receiver<SyncJob>, tx: Sender<Reply>, api_key: KeyCache, ctx: egui::Context) {
    let mut api: Option<(String, Client)> = None;
    while let Ok(SyncJob {
        id,
        manual,
        target,
        header,
        since,
        keys,
    }) = rx.recv()
    {
        let result = client(&api_key, &mut api, manual).and_then(|c| match c.export(&target, since) {
            Ok(Fetched::NotModified) => Ok(Synced::Unchanged),
            Ok(Fetched::Changed { version, items }) => Ok(Synced::Content {
                version,
                built: zotero::build_bib(&header, &items, &keys),
            }),
            Err(e) => Err(export_error(e)),
        });
        if tx.send(Reply::Synced { id, result }).is_err() {
            break;
        }
        ctx.request_repaint();
    }
}

/// The person's own library and their groups, each with its collections.
fn libraries(c: &mut Client, user_id: u64) -> Result<Vec<LibraryTree>, String> {
    let mine = c.collections(LibraryRef::User(user_id));
    let groups = c.groups(user_id);
    // Refused everywhere: the key itself is no longer valid (revoked or replaced).
    if matches!(mine, Err(zotero::Error::Forbidden)) && matches!(groups, Err(zotero::Error::Forbidden)) {
        return Err(export_error(zotero::Error::Forbidden));
    }
    let mut out = vec![library_tree(LibraryRef::User(user_id), MY_LIBRARY.to_string(), mine)];
    match groups {
        Ok(groups) => {
            for (id, name) in groups {
                let collections = c.collections(LibraryRef::Group(id));
                out.push(library_tree(LibraryRef::Group(id), name, collections));
            }
        }
        // A key without group access may not list them; the own library still works.
        Err(zotero::Error::Forbidden) => {}
        Err(e) => return Err(e.to_string()),
    }
    Ok(out)
}

fn library_tree(library: LibraryRef, name: String, collections: zotero::Result<Vec<zotero::Collection>>) -> LibraryTree {
    match collections {
        Ok(all) => LibraryTree {
            library,
            name,
            collections: collection_rows(&all),
            error: None,
        },
        Err(zotero::Error::Forbidden) => LibraryTree {
            library,
            name,
            collections: Vec::new(),
            error: Some("Kein Lesezugriff mit diesem Schlüssel".into()),
        },
        Err(e) => LibraryTree {
            library,
            name,
            collections: Vec::new(),
            error: Some(e.to_string()),
        },
    }
}

/// Collections as an indented list: parents before children, siblings by name.
pub fn collection_rows(all: &[zotero::Collection]) -> Vec<CollectionRow> {
    let keys: HashSet<&str> = all.iter().map(|c| c.key.as_str()).collect();
    let mut children: HashMap<Option<&str>, Vec<&zotero::Collection>> = HashMap::new();
    for c in all {
        // A collection whose parent is missing is shown at the top.
        let parent = c.parent.as_deref().filter(|p| keys.contains(p) && *p != c.key);
        children.entry(parent).or_default().push(c);
    }
    for list in children.values_mut() {
        list.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()).then_with(|| a.key.cmp(&b.key)));
    }
    fn walk(
        parent: Option<&str>,
        path: &str,
        depth: usize,
        children: &HashMap<Option<&str>, Vec<&zotero::Collection>>,
        seen: &mut HashSet<String>,
        out: &mut Vec<CollectionRow>,
    ) {
        let Some(list) = children.get(&parent) else { return };
        for c in list {
            if !seen.insert(c.key.clone()) {
                continue;
            }
            let full = if path.is_empty() {
                c.name.clone()
            } else {
                format!("{path} › {}", c.name)
            };
            out.push(CollectionRow {
                key: c.key.clone(),
                name: c.name.clone(),
                path: full.clone(),
                depth,
                items: c.items,
                has_children: children.get(&Some(c.key.as_str())).map(|v| !v.is_empty()).unwrap_or(false),
            });
            walk(Some(c.key.as_str()), &full, depth + 1, children, seen, out);
        }
    }
    let mut out = Vec::new();
    walk(None, "", 0, &children, &mut HashSet::new(), &mut out);
    out
}

// ---------------------------------------------------------------------------- helpers

pub fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// "vor 4 Minuten"
pub fn ago(then: u64, now: u64) -> String {
    let s = now.saturating_sub(then);
    match s {
        0..60 => "gerade eben".into(),
        60..120 => "vor 1 Minute".into(),
        120..3600 => format!("vor {} Minuten", s / 60),
        3600..7200 => "vor 1 Stunde".into(),
        7200..86400 => format!("vor {} Stunden", s / 3600),
        86400..172800 => "vor 1 Tag".into(),
        _ => format!("vor {} Tagen", s / 86400),
    }
}

/// FNV-1a: a fingerprint that stays the same across program versions.
fn fingerprint(bytes: &[u8]) -> u64 {
    bytes
        .iter()
        .fold(0xcbf29ce484222325u64, |h, b| (h ^ *b as u64).wrapping_mul(0x100000001b3))
}

/// Writes through a hidden temporary file, so readers never see half a file (see files.rs).
fn write_atomic(path: &Path, text: &str) -> std::io::Result<()> {
    crate::files::atomic_write(path, text.as_bytes())
}

/// The name the disk uses for `part` among `names`: an exact match wins, then one that
/// differs only in Unicode composition (folders made in the Finder are stored decomposed,
/// typed names are composed) and, with `any_case`, one that differs only in case (what a
/// case-insensitive disk just wrote through).
fn matching_name(names: &[String], part: &str, any_case: bool) -> Option<String> {
    use unicode_normalization::UnicodeNormalization;
    let folded = |s: &str| -> String { s.nfc().flat_map(char::to_lowercase).collect() };
    names
        .iter()
        .find(|n| *n == part)
        .or_else(|| names.iter().find(|n| n.nfc().eq(part.nfc())))
        .or_else(|| {
            if any_case {
                names.iter().find(|n| folded(n) == folded(part))
            } else {
                None
            }
        })
        .cloned()
}

/// `rel` with every existing folder part spelled as the disk spells it; parts below the
/// deepest real folder keep the typed spelling. `create` writes into a folder made in the
/// Finder instead of making a sibling that differs only in Unicode composition, which a
/// normalisation-sensitive disk keeps apart.
fn folder_spelling(root: &Path, rel: &str, any_case: bool) -> String {
    let mut dir = root.to_path_buf();
    let mut parts = Vec::new();
    let all: Vec<&str> = rel.split('/').collect();
    let (folders, last) = all.split_at(all.len().saturating_sub(1));
    for part in folders {
        let names: Vec<String> = std::fs::read_dir(&dir)
            .map(|rd| rd.flatten().filter_map(|e| e.file_name().into_string().ok()).collect())
            .unwrap_or_default();
        let name = matching_name(&names, part, any_case).unwrap_or_else(|| (*part).to_string());
        dir.push(&name);
        parts.push(name);
    }
    parts.extend(last.iter().map(|s| (*s).to_string()));
    parts.join("/")
}

/// `rel` as it is spelled on disk (the way the file list names it), if it is a file.
fn disk_spelling(root: &Path, rel: &str, any_case: bool) -> Option<String> {
    let mut dir = root.to_path_buf();
    let mut parts = Vec::new();
    for part in rel.split('/') {
        let names: Vec<String> = std::fs::read_dir(&dir)
            .ok()?
            .flatten()
            .filter_map(|e| e.file_name().into_string().ok())
            .collect();
        let name = matching_name(&names, part, any_case)?;
        dir.push(&name);
        parts.push(name);
    }
    dir.is_file().then(|| parts.join("/"))
}

/// Whether `rel` exists with exactly this spelling. On a case-insensitive disk `literatur.bib`
/// also "exists" after a rename to `Literatur.bib` in the Finder, but then it is a different
/// file for the editor and must no longer be synced.
fn exists_exact(root: &Path, rel: &str) -> bool {
    disk_spelling(root, rel, false).as_deref() == Some(rel)
}

/// Whether `rel` stays inside the project, also after following symlinked folders.
fn inside(root: &Path, rel: &str) -> bool {
    if clean_rel(rel).as_deref() != Ok(rel) {
        return false;
    }
    let Ok(root) = root.canonicalize() else { return false };
    let mut probe = root.join(rel);
    probe.pop();
    // The deepest folder on the way that exists already.
    loop {
        if let Ok(real) = probe.canonicalize() {
            return real.starts_with(&root);
        }
        if !probe.pop() {
            return false;
        }
    }
}

/// The comment at the top of every synced file.
pub fn header(source: &str) -> String {
    let source: String = source.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
    format!("% Automatisch aus Zotero synchronisiert von Typisch. Änderungen in dieser Datei werden überschrieben.\n% Quelle: {source}\n")
}

/// Checks a file name typed by the person: a path inside the project, ending in `.bib`.
pub fn clean_rel(input: &str) -> Result<String, String> {
    let mut s = input.trim().replace('\\', "/");
    if s.is_empty() {
        return Err("Bitte einen Dateinamen eingeben.".into());
    }
    if s.starts_with('/') || s.contains(':') || s.chars().any(|c| c.is_control()) {
        return Err("Bitte einen Dateinamen innerhalb des Projekts angeben.".into());
    }
    for part in s.split('/') {
        if part.is_empty() || part == "." || part == ".." {
            return Err("Bitte einen Dateinamen innerhalb des Projekts angeben.".into());
        }
        if part.starts_with('.') {
            return Err("Versteckte Dateien und Ordner (mit Punkt am Anfang) sind nicht möglich.".into());
        }
    }
    if !s.to_lowercase().ends_with(".bib") {
        s.push_str(".bib");
    }
    Ok(s)
}

// ---------------------------------------------------------------------------- state on the UI thread

/// The "Aus Zotero synchronisieren" dialog.
pub struct CreateDialog {
    pub name: String,
    /// The chosen library and, optionally, collection.
    pub choice: Option<(LibraryRef, Option<String>)>,
    pub subcollections: bool,
    pub error: Option<String>,
    /// Put the cursor in the name field (once, when the dialog opens).
    pub focus_name: bool,
}

/// What `poll` reports to the app.
pub enum Event {
    /// A synced file got new content on disk.
    Written(FileKey),
    /// A sync the person started failed.
    Failed(FileKey, String),
    /// Connected as this account; `true` when it is another account than before.
    Connected(String, bool),
}

pub struct RefSync {
    pub store: Store,
    account_tx: Option<Sender<AccountJob>>,
    sync_tx: Option<Sender<SyncJob>>,
    rx: Option<Receiver<Reply>>,
    key: KeyCache,
    /// Where the state is saved; `None` keeps it in memory only (check pictures, tests).
    store_path: Option<PathBuf>,
    /// Whether the system's key storage may be touched (never in tests or check pictures).
    keychain: bool,
    /// For merging with another running Typisch: the files and the account on disk when last
    /// seen, the files dropped and changed here since, and whether the account was
    /// disconnected here since.
    known: HashSet<FileKey>,
    /// The account was connected or its key replaced here since the last save.
    account_changed: bool,
    dropped: HashSet<FileKey>,
    touched: HashSet<FileKey>,
    cleared: bool,
    /// Counts connect attempts and disconnects; an answer to an older attempt is dropped.
    connect_generation: u64,
    /// Files with a sync under way; `true` when the person started it.
    busy: HashMap<FileKey, bool>,
    /// The open project and when its files are refreshed next.
    next: Option<(PathBuf, Instant)>,
    pub key_input: String,
    /// Changes whenever `key_input` is cleared, so the field forgets its undo history too.
    pub key_generation: u32,
    /// "Schlüssel ersetzen" is open in the settings.
    pub replacing_key: bool,
    pub connecting: bool,
    pub connect_error: Option<String>,
    pub libraries: Option<Result<Vec<LibraryTree>, String>>,
    pub loading_libraries: bool,
    pub dialog: Option<CreateDialog>,
}

impl RefSync {
    pub fn new(ctx: &egui::Context) -> Self {
        let mut s = Self::offline();
        s.use_store(Store::default_path());
        s.keychain = true;
        s.key = Arc::new(Keys::new(true));
        s.start_threads(ctx);
        s
    }

    fn use_store(&mut self, path: Option<PathBuf>) {
        self.store = path.as_deref().map(Store::load).unwrap_or_default();
        self.known = self.store.files.iter().map(file_key).collect();
        self.store_path = path;
    }

    fn start_threads(&mut self, ctx: &egui::Context) {
        let (account_tx, account_rx) = channel::<AccountJob>();
        let (sync_tx, sync_rx) = channel::<SyncJob>();
        let (reply_tx, rx) = channel::<Reply>();
        let (cache, tx, c) = (self.key.clone(), reply_tx.clone(), ctx.clone());
        let a = std::thread::Builder::new()
            .name("zotero".into())
            .spawn(move || account_worker(account_rx, tx, cache, c));
        let (cache, c) = (self.key.clone(), ctx.clone());
        let b = std::thread::Builder::new()
            .name("zotero-sync".into())
            .spawn(move || sync_worker(sync_rx, reply_tx, cache, c));
        if a.is_ok() && b.is_ok() {
            self.account_tx = Some(account_tx);
            self.sync_tx = Some(sync_tx);
            self.rx = Some(rx);
        }
    }

    /// Without network, Keychain or saved state (for the check pictures).
    pub fn offline() -> Self {
        Self {
            store: Store::default(),
            account_tx: None,
            sync_tx: None,
            rx: None,
            key: Arc::new(Keys::new(false)),
            store_path: None,
            keychain: false,
            known: HashSet::new(),
            account_changed: false,
            dropped: HashSet::new(),
            touched: HashSet::new(),
            cleared: false,
            connect_generation: 0,
            busy: HashMap::new(),
            next: None,
            key_input: String::new(),
            key_generation: 0,
            replacing_key: false,
            connecting: false,
            connect_error: None,
            libraries: None,
            loading_libraries: false,
            dialog: None,
        }
    }

    fn send(&self, job: AccountJob) -> bool {
        self.account_tx.as_ref().map(|tx| tx.send(job).is_ok()).unwrap_or(false)
    }

    /// Takes in what another running Typisch saved since: files it added (unless dropped
    /// here), files it dropped, newer sync state of files not changed here, and a connect,
    /// key change or disconnect of the account (unless the account was changed here).
    fn merge_from_disk(&mut self) {
        let Some(path) = self.store_path.clone() else { return };
        // A disconnect here wins: it is written as it is.
        if self.cleared {
            return;
        }
        let disk = Store::load(&path);
        if !self.account_changed {
            let mine = self.store.account.as_ref().map(|a| a.user_id);
            let theirs = disk.account.as_ref().map(|a| a.user_id);
            if mine.is_some() && theirs.is_none() {
                // Disconnected over there (the key is already gone from the Keychain).
                self.key.set(KeyState::Missing);
                self.busy.clear();
            } else if theirs.is_some() && theirs != mine {
                // Connected (to another account) over there: read the key it stored.
                self.key.forget();
            }
            self.store.account = disk.account.clone();
        }
        let on_disk: HashMap<FileKey, SyncedFile> = disk.files.into_iter().map(|f| (file_key(&f), f)).collect();
        let (known, touched) = (&self.known, &self.touched);
        self.store.files.retain(|f| {
            let k = file_key(f);
            on_disk.contains_key(&k) || !known.contains(&k)
        });
        for f in self.store.files.iter_mut() {
            let k = file_key(f);
            if !touched.contains(&k)
                && let Some(newer) = on_disk.get(&k)
            {
                *f = newer.clone();
            }
        }
        for (k, f) in &on_disk {
            if !self.dropped.contains(k) && self.get(&k.0, &k.1).is_none() {
                self.store.files.push(f.clone());
            }
        }
        self.known = on_disk.into_keys().collect();
    }

    fn save(&mut self) {
        let Some(path) = self.store_path.clone() else { return };
        self.merge_from_disk();
        match self.store.save(&path) {
            Ok(()) => {
                self.known = self.store.files.iter().map(file_key).collect();
                self.account_changed = false;
                self.dropped.clear();
                self.touched.clear();
                self.cleared = false;
            }
            // Kept as "changed here", so the next save tries again.
            Err(e) => eprintln!("Typisch: {} konnte nicht gespeichert werden: {e}", path.display()),
        }
    }

    fn forget(&mut self, root: &Path, rel: &str) {
        self.store.files.retain(|f| !(f.project == root && f.rel == rel));
        let id = (root.to_path_buf(), rel.to_string());
        self.busy.remove(&id);
        self.dropped.insert(id);
    }

    fn clear_key_input(&mut self) {
        self.key_input.clear();
        self.key_generation = self.key_generation.wrapping_add(1);
    }

    pub fn account(&self) -> Option<&Account> {
        self.store.account.as_ref()
    }

    pub fn get(&self, root: &Path, rel: &str) -> Option<&SyncedFile> {
        self.store.files.iter().find(|f| f.project == root && f.rel == rel)
    }

    fn get_mut(&mut self, root: &Path, rel: &str) -> Option<&mut SyncedFile> {
        self.store.files.iter_mut().find(|f| f.project == root && f.rel == rel)
    }

    pub fn is_synced(&self, root: &Path, rel: &str) -> bool {
        self.get(root, rel).is_some()
    }

    pub fn is_busy(&self, root: &Path, rel: &str) -> bool {
        self.busy.contains_key(&(root.to_path_buf(), rel.to_string()))
    }

    fn has_files(&self, root: &Path) -> bool {
        self.store.files.iter().any(|f| f.project == root)
    }

    // ------------------------------------------------------------ account

    /// Checks the key in `key_input` with Zotero and keeps it when it works.
    pub fn connect(&mut self) {
        let key: String = self.key_input.chars().filter(|c| !c.is_whitespace()).collect();
        if key.is_empty() || self.connecting {
            return;
        }
        // Zotero keys are letters and digits. Anything else (an invisible character copied
        // along) would end up quoted in an error message, i.e. the key shown in clear text.
        if !key.chars().all(|c| c.is_ascii_alphanumeric()) {
            self.connect_error = Some("Der Schlüssel enthält ungültige Zeichen. Bitte nur den Schlüssel von zotero.org einfügen.".into());
            return;
        }
        self.connect_error = None;
        self.connect_generation += 1;
        if self.send(AccountJob::Connect {
            key,
            generation: self.connect_generation,
        }) {
            self.connecting = true;
        } else {
            self.connect_error = Some("Zotero ist in dieser Sitzung nicht verfügbar.".into());
        }
    }

    /// Gives up a key check still under way ("Nicht ersetzen"): its answer is ignored.
    pub fn cancel_connect(&mut self) {
        self.connect_generation += 1;
        self.connecting = false;
        self.connect_error = None;
    }

    /// Forgets the account and its key. All synced files become ordinary text files.
    pub fn disconnect(&mut self) {
        // Right here, not on the thread: the key must be gone even if the app quits now.
        let deleted = if self.keychain { keychain::delete() } else { Ok(()) };
        self.key.set(KeyState::Missing);
        // A key check still under way must not connect again afterwards.
        self.connect_generation += 1;
        self.connecting = false;
        self.send(AccountJob::Disconnect);
        self.store.account = None;
        self.store.files.clear();
        self.busy.clear();
        self.next = None;
        self.libraries = None;
        self.replacing_key = false;
        self.clear_key_input();
        self.connect_error = deleted.err().map(|e| format!("Der Schlüssel konnte nicht gelöscht werden: {e}"));
        self.cleared = true;
        self.save();
    }

    /// Asks for the libraries and collections (for the dialog).
    pub fn request_libraries(&mut self) {
        let Some(user_id) = self.store.account.as_ref().map(|a| a.user_id) else {
            return;
        };
        if !self.loading_libraries && self.send(AccountJob::Libraries { user_id }) {
            self.loading_libraries = true;
        }
    }

    // ------------------------------------------------------------ files

    /// Opens the dialog with a file name that is still free.
    pub fn open_dialog(&mut self, root: &Path) {
        self.prune(root);
        let mut name = "literatur.bib".to_string();
        let mut n = 2;
        // `symlink_metadata`: a dangling symlink counts as taken too.
        while root.join(&name).symlink_metadata().is_ok() || self.is_synced(root, &name) {
            name = format!("literatur-{n}.bib");
            n += 1;
        }
        self.dialog = Some(CreateDialog {
            name,
            choice: None,
            subcollections: true,
            error: None,
            focus_name: true,
        });
        // Collections may have changed since the dialog was last open.
        self.libraries = None;
        self.request_libraries();
    }

    /// "Meine Bibliothek › Doktorarbeit › Kapitel 1"
    pub fn source_label(&self, library: LibraryRef, collection: Option<&str>) -> String {
        let lib = self
            .libraries
            .as_ref()
            .and_then(|l| l.as_ref().ok())
            .and_then(|l| l.iter().find(|t| t.library == library));
        let name = lib.map(|t| t.name.clone()).unwrap_or_else(|| match library {
            LibraryRef::User(_) => MY_LIBRARY.to_string(),
            LibraryRef::Group(id) => format!("Gruppe {id}"),
        });
        match collection {
            None => name,
            Some(key) => {
                let path = lib
                    .and_then(|t| t.collections.iter().find(|c| c.key == key))
                    .map(|c| c.path.clone())
                    .unwrap_or_else(|| key.to_string());
                format!("{name} › {path}")
            }
        }
    }

    /// Creates the file chosen in the dialog and starts its first sync. Returns its path.
    pub fn create(&mut self, root: &Path) -> Result<String, String> {
        self.prune(root);
        let Some(d) = self.dialog.as_ref() else { return Err(String::new()) };
        let rel = clean_rel(&d.name)?;
        // A folder made in the Finder is stored decomposed, the typed name is composed; use
        // the folder as the disk spells it (on a normalisation-sensitive disk the two are
        // different folders).
        let rel = folder_spelling(root, &rel, true);
        let Some((library, collection)) = d.choice.clone() else {
            return Err("Bitte auswählen, was synchronisiert werden soll.".into());
        };
        if self.store.account.is_none() {
            return Err(NOT_CONNECTED.into());
        }
        let path = root.join(&rel);
        if path.symlink_metadata().is_ok() || self.is_synced(root, &rel) {
            return Err(format!("„{rel}“ gibt es schon. Bitte einen anderen Namen wählen."));
        }
        let outside = || format!("„{rel}“ liegt über einen verknüpften Ordner außerhalb des Projekts.");
        if !inside(root, &rel) {
            return Err(outside());
        }
        let source = self.source_label(library, collection.as_deref());
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("Der Ordner konnte nicht angelegt werden: {e}"))?;
        }
        let placeholder = format!("{}\n% Wird gerade synchronisiert …\n", header(&source));
        {
            use std::io::Write;
            // `create_new` never writes through something that appeared in the meantime.
            std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
                .and_then(|mut f| f.write_all(placeholder.as_bytes()))
                .map_err(|e| format!("Die Datei konnte nicht angelegt werden: {e}"))?;
        }
        // Stored as the file list spells it, so the editor recognises it as synced.
        let rel = disk_spelling(root, &rel, true).unwrap_or(rel);
        // Only what the dialog showed: its switch is off for a collection without children.
        let has_children = self
            .libraries
            .as_ref()
            .and_then(|l| l.as_ref().ok())
            .and_then(|l| l.iter().find(|t| t.library == library))
            .and_then(|t| t.collections.iter().find(|c| Some(&c.key) == collection.as_ref()))
            .map(|c| c.has_children)
            .unwrap_or(false);
        let subcollections = d.subcollections && has_children;
        self.store.files.push(SyncedFile {
            project: root.to_path_buf(),
            rel: rel.clone(),
            target: Target {
                library,
                collection,
                subcollections,
            },
            source,
            last_sync: None,
            version: None,
            entries: 0,
            keys: BTreeMap::new(),
            written: Some(fingerprint(placeholder.as_bytes())),
            error: None,
        });
        self.save();
        self.dialog = None;
        self.sync_file(root, &rel, true);
        Ok(rel)
    }

    /// Refreshes one file. `manual`: the person asked for it, so everything is fetched again
    /// (even when Zotero reports no change) and a failure is announced.
    pub fn sync_file(&mut self, root: &Path, rel: &str, manual: bool) {
        let id: FileKey = (root.to_path_buf(), rel.to_string());
        if self.busy.contains_key(&id) || self.sync_tx.is_none() {
            return;
        }
        let connected = self.store.account.is_some();
        let Some(f) = self.get_mut(root, rel) else { return };
        if !connected {
            f.error = Some(NOT_CONNECTED.into());
            return;
        }
        // A file changed outside Typisch is restored with a full fetch.
        let intact = std::fs::read(root.join(rel))
            .map(|b| Some(fingerprint(&b)) == f.written)
            .unwrap_or(false);
        let since = if manual || !intact { None } else { f.version };
        let job = SyncJob {
            id: id.clone(),
            manual,
            target: f.target.clone(),
            header: header(&f.source),
            since,
            keys: f.keys.clone(),
        };
        if self.sync_tx.as_ref().map(|tx| tx.send(job).is_ok()).unwrap_or(false) {
            self.busy.insert(id, manual);
        }
    }

    pub fn sync_project(&mut self, root: &Path, manual: bool) {
        let rels: Vec<String> = self
            .store
            .files
            .iter()
            .filter(|f| f.project == root)
            .map(|f| f.rel.clone())
            .collect();
        for rel in rels {
            self.sync_file(root, &rel, manual);
        }
    }

    /// Stops syncing the file; it stays as it is, as an ordinary text file.
    pub fn detach(&mut self, root: &Path, rel: &str) {
        self.forget(root, rel);
        self.save();
    }

    /// Forgets files of this project that were deleted or renamed outside Typisch.
    pub fn prune(&mut self, root: &Path) {
        let gone: Vec<String> = self
            .store
            .files
            .iter()
            .filter(|f| f.project == root && !exists_exact(root, &f.rel))
            .map(|f| f.rel.clone())
            .collect();
        for rel in &gone {
            self.forget(root, rel);
        }
        if !gone.is_empty() {
            self.save();
        }
    }

    /// Keeps the open project's files fresh: right after it opens, then every five minutes.
    /// Returns how long until the next refresh is due.
    pub fn tick(&mut self, root: Option<&Path>) -> Option<Duration> {
        let Some(root) = root else {
            self.next = None;
            return None;
        };
        let now = Instant::now();
        let same_project = self.next.as_ref().map(|(p, _)| p == root).unwrap_or(false);
        if !same_project {
            // Another running Typisch may have set up files in this project.
            self.merge_from_disk();
            self.prune(root);
        }
        let due = !same_project || self.next.as_ref().map(|(_, at)| *at <= now).unwrap_or(true);
        if due {
            self.next = Some((root.to_path_buf(), now + AUTO_SYNC));
            if self.store.account.is_some() {
                self.sync_project(root, false);
            }
        }
        if !self.has_files(root) {
            return None;
        }
        self.next.as_ref().map(|(_, at)| at.saturating_duration_since(now))
    }

    /// Takes in what the background thread finished.
    pub fn poll(&mut self) -> Vec<Event> {
        let mut events = Vec::new();
        let replies: Vec<Reply> = match self.rx.as_ref() {
            Some(rx) => rx.try_iter().collect(),
            None => return events,
        };
        let mut changed = false;
        // Another running Typisch may have disconnected a file meanwhile (and the person may be
        // editing it there): see that before writing anything.
        if replies.iter().any(|r| matches!(r, Reply::Synced { .. })) {
            self.merge_from_disk();
        }
        for reply in replies {
            match reply {
                // An answer to an attempt given up since (disconnected meanwhile).
                Reply::Connected { generation, .. } if generation != self.connect_generation => {}
                Reply::Connected {
                    result: Ok((account, key)),
                    ..
                } => {
                    self.connecting = false;
                    // Stored only now, on this thread, so a disconnect always has the last word.
                    if let Err(e) = if self.keychain { keychain::save(&key) } else { Ok(()) } {
                        self.connect_error = Some(format!("Der Schlüssel konnte nicht gespeichert werden: {e}"));
                        continue;
                    }
                    self.key.set(KeyState::Present(key));
                    self.replacing_key = false;
                    self.clear_key_input();
                    self.connect_error = None;
                    // A replaced key keeps all synced files; only another account is worth a word.
                    let other = self.store.account.as_ref().map(|a| a.user_id != account.user_id).unwrap_or(false);
                    events.push(Event::Connected(account.label().to_string(), other));
                    self.store.account = Some(account);
                    self.account_changed = true;
                    for f in &mut self.store.files {
                        f.error = None;
                        self.touched.insert((f.project.clone(), f.rel.clone()));
                    }
                    // Sync the open project's files straight away.
                    self.next = None;
                    self.libraries = None;
                    if self.dialog.is_some() {
                        self.request_libraries();
                    }
                    changed = true;
                }
                Reply::Connected { result: Err(e), .. } => {
                    self.connecting = false;
                    self.connect_error = Some(e);
                }
                Reply::Libraries(result) => {
                    self.loading_libraries = false;
                    // A reply that arrives after the account was disconnected is of no use.
                    if self.store.account.is_some() {
                        self.libraries = Some(result);
                    }
                }
                Reply::Synced { id, result } => {
                    let manual = self.busy.remove(&id).unwrap_or(false);
                    let (root, rel) = (&id.0, id.1.as_str());
                    let path = root.join(rel);
                    // Disconnected (or deleted outside Typisch) while the sync was under way.
                    if !self.is_synced(root, rel) {
                        continue;
                    }
                    if !exists_exact(root, rel) {
                        self.forget(root, rel);
                        changed = true;
                        continue;
                    }
                    // Never write outside the project (a symlinked folder, an edited state file).
                    let blocked = !inside(root, rel);
                    self.touched.insert(id.clone());
                    let Some(f) = self.get_mut(root, rel) else { continue };
                    match result {
                        Ok(Synced::Unchanged) => {
                            f.last_sync = Some(now_secs());
                            f.error = None;
                        }
                        Ok(Synced::Content { version, built }) => {
                            let same = std::fs::read(&path).map(|b| b == built.text.as_bytes()).unwrap_or(false);
                            let written = if same {
                                Ok(())
                            } else if blocked {
                                Err(std::io::Error::other("Der Pfad führt aus dem Projekt hinaus."))
                            } else {
                                write_atomic(&path, &built.text)
                            };
                            match written {
                                Ok(()) => {
                                    f.version = version;
                                    f.keys = built.keys;
                                    f.entries = built.entries;
                                    f.written = Some(fingerprint(built.text.as_bytes()));
                                    f.last_sync = Some(now_secs());
                                    f.error = None;
                                    if !same {
                                        events.push(Event::Written(id.clone()));
                                    }
                                }
                                Err(e) => {
                                    let msg = format!("Die Datei konnte nicht geschrieben werden: {e}");
                                    f.error = Some(msg.clone());
                                    if manual {
                                        events.push(Event::Failed(id.clone(), msg));
                                    }
                                }
                            }
                        }
                        Err(msg) => {
                            f.error = Some(msg.clone());
                            if manual {
                                events.push(Event::Failed(id.clone(), msg));
                            }
                        }
                    }
                    changed = true;
                }
            }
        }
        if changed {
            self.save();
        }
        events
    }

    /// Example state for the check pictures (`TYPISCH_SHOT_VIEW=zotero-…`).
    pub fn demo(&mut self, root: &Path, rel: &str) {
        self.store.account = Some(Account {
            user_id: 1,
            username: "sven".into(),
            display_name: "Sven".into(),
        });
        self.store.files.push(SyncedFile {
            project: root.to_path_buf(),
            rel: rel.to_string(),
            target: Target {
                library: LibraryRef::User(1),
                collection: Some("THESIS".into()),
                subcollections: true,
            },
            source: format!("{MY_LIBRARY} › Doktorarbeit"),
            last_sync: Some(now_secs().saturating_sub(4 * 60)),
            version: Some(1),
            entries: 2,
            keys: BTreeMap::new(),
            written: None,
            error: None,
        });
        self.demo_libraries();
    }

    /// The example libraries and collections of the check pictures.
    pub fn demo_libraries(&mut self) {
        let rows = vec![
            ("THESIS", "Doktorarbeit", None, 42),
            ("CH1", "Kapitel 1 – Grundlagen", Some("THESIS"), 17),
            ("CH2", "Kapitel 2 – Methoden", Some("THESIS"), 9),
            ("READ", "Leseliste", None, 120),
        ];
        let all: Vec<zotero::Collection> = rows
            .into_iter()
            .map(|(k, n, p, i)| zotero::Collection {
                key: k.into(),
                name: n.into(),
                parent: p.map(str::to_string),
                items: i,
            })
            .collect();
        self.libraries = Some(Ok(vec![
            LibraryTree {
                library: LibraryRef::User(1),
                name: MY_LIBRARY.into(),
                collections: collection_rows(&all),
                error: None,
            },
            LibraryTree {
                library: LibraryRef::Group(7),
                name: "Lehrstuhl Typografie".into(),
                collections: Vec::new(),
                error: None,
            },
        ]));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_names_stay_inside_the_project() {
        assert_eq!(clean_rel(" literatur "), Ok("literatur.bib".into()));
        assert_eq!(clean_rel("refs/zotero.BIB"), Ok("refs/zotero.BIB".into()));
        assert_eq!(clean_rel("refs\\a.bib"), Ok("refs/a.bib".into()));
        for bad in [
            "",
            "  ",
            "/etc/x.bib",
            "../x.bib",
            "a/../../x.bib",
            "a//b.bib",
            ".hidden.bib",
            "C:/x.bib",
            "a/./b.bib",
            "x\n.bib",
        ] {
            assert!(clean_rel(bad).is_err(), "{bad:?} should be refused");
        }
    }

    #[test]
    fn relative_times_read_naturally() {
        assert_eq!(ago(1000, 1030), "gerade eben");
        assert_eq!(ago(1000, 1000 + 61), "vor 1 Minute");
        assert_eq!(ago(1000, 1000 + 4 * 60 + 5), "vor 4 Minuten");
        assert_eq!(ago(0, 3600), "vor 1 Stunde");
        assert_eq!(ago(0, 5 * 3600), "vor 5 Stunden");
        assert_eq!(ago(0, 86400), "vor 1 Tag");
        assert_eq!(ago(0, 3 * 86400), "vor 3 Tagen");
        // A clock that went backwards.
        assert_eq!(ago(2000, 1000), "gerade eben");
    }

    #[test]
    fn collections_are_listed_as_a_tree() {
        let c = |k: &str, n: &str, p: Option<&str>| zotero::Collection {
            key: k.into(),
            name: n.into(),
            parent: p.map(str::to_string),
            items: 1,
        };
        let all = vec![
            c("B", "beta", None),
            c("A", "Alpha", None),
            c("A2", "zwei", Some("A")),
            c("A1", "eins", Some("A")),
            c("O", "Waise", Some("GONE")),
        ];
        let rows = collection_rows(&all);
        let names: Vec<(&str, usize)> = rows.iter().map(|r| (r.path.as_str(), r.depth)).collect();
        assert_eq!(
            names,
            vec![("Alpha", 0), ("Alpha › eins", 1), ("Alpha › zwei", 1), ("beta", 0), ("Waise", 0)]
        );
        assert!(rows[0].has_children && !rows[1].has_children);
    }

    #[test]
    fn header_is_a_biblatex_comment() {
        let h = header("Meine Bibliothek › A\nB");
        assert!(h.lines().all(|l| l.starts_with('%')));
        assert!(h.contains("A B"));
    }

    #[test]
    fn fingerprint_is_stable() {
        assert_eq!(fingerprint(b""), 0xcbf29ce484222325);
        assert_eq!(fingerprint(b"a"), 0xaf63dc4c8601ec8c);
    }

    fn temp_project(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("typisch-refsync-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn creating_a_file_needs_an_account_and_a_free_name() {
        let root = temp_project("create");
        let mut s = RefSync::offline();
        s.open_dialog(&root);
        assert_eq!(s.dialog.as_ref().unwrap().name, "literatur.bib");
        s.dialog.as_mut().unwrap().choice = Some((LibraryRef::User(1), None));
        assert_eq!(s.create(&root), Err(NOT_CONNECTED.to_string()));
        s.store.account = Some(Account {
            user_id: 1,
            username: "u".into(),
            display_name: String::new(),
        });
        assert_eq!(s.create(&root), Ok("literatur.bib".to_string()));
        assert!(s.dialog.is_none());
        let text = std::fs::read_to_string(root.join("literatur.bib")).unwrap();
        assert!(text.starts_with("% Automatisch aus Zotero synchronisiert"));
        assert!(text.contains("% Quelle: Meine Bibliothek"));
        assert!(s.is_synced(&root, "literatur.bib"));
        // The next dialog proposes another name, and an existing one is refused.
        s.open_dialog(&root);
        assert_eq!(s.dialog.as_ref().unwrap().name, "literatur-2.bib");
        s.dialog.as_mut().unwrap().name = "literatur".into();
        s.dialog.as_mut().unwrap().choice = Some((LibraryRef::User(1), None));
        assert!(s.create(&root).unwrap_err().contains("gibt es schon"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn detaching_and_pruning_forget_files() {
        let root = temp_project("prune");
        let mut s = RefSync::offline();
        s.demo(&root, "a.bib");
        s.demo(&root, "b.bib");
        std::fs::write(root.join("a.bib"), "x").unwrap();
        s.prune(&root);
        assert!(s.is_synced(&root, "a.bib"));
        assert!(!s.is_synced(&root, "b.bib"), "b.bib does not exist");
        s.detach(&root, "a.bib");
        assert!(!s.is_synced(&root, "a.bib"));
        assert!(root.join("a.bib").exists(), "the file itself stays");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    #[cfg(unix)]
    fn creating_never_writes_through_a_symlink() {
        let root = temp_project("dangling");
        let outside = temp_project("dangling-outside");
        // A dangling link with the proposed name, and a linked folder leading elsewhere.
        std::os::unix::fs::symlink(outside.join("x"), root.join("literatur.bib")).unwrap();
        std::os::unix::fs::symlink(&outside, root.join("refs")).unwrap();
        let mut s = RefSync::offline();
        s.store.account = Some(Account {
            user_id: 1,
            username: "u".into(),
            display_name: String::new(),
        });
        s.open_dialog(&root);
        assert_eq!(s.dialog.as_ref().unwrap().name, "literatur-2.bib");
        for name in ["literatur.bib", "refs/a.bib"] {
            let d = s.dialog.as_mut().unwrap();
            d.name = name.into();
            d.choice = Some((LibraryRef::User(1), None));
            assert!(s.create(&root).is_err(), "{name} must be refused");
        }
        assert!(!outside.join("x").exists() && !outside.join("a.bib").exists());
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&outside);
    }

    #[test]
    fn exact_spelling_is_required() {
        let root = temp_project("case");
        std::fs::create_dir_all(root.join("Refs")).unwrap();
        std::fs::write(root.join("Refs/Literatur.bib"), "x").unwrap();
        assert!(exists_exact(&root, "Refs/Literatur.bib"));
        assert!(!exists_exact(&root, "refs/Literatur.bib"));
        assert!(!exists_exact(&root, "Refs/literatur.bib"));
        assert!(!exists_exact(&root, "Refs"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn two_running_apps_keep_each_others_files() {
        let dir = temp_project("store");
        let path = dir.join("reference-sync.json");
        let (p1, p2) = (dir.join("p1"), dir.join("p2"));
        for p in [&p1, &p2] {
            std::fs::create_dir_all(p).unwrap();
        }
        let mut a = RefSync::offline();
        a.use_store(Some(path.clone()));
        a.demo(&p1, "a.bib");
        a.save();
        let mut b = RefSync::offline();
        b.use_store(Some(path.clone()));
        b.demo(&p2, "b.bib");
        b.save();
        // A saves again (e.g. after a sync) without knowing about b.bib: b.bib stays.
        a.save();
        assert!(a.is_synced(&p2, "b.bib"));
        assert_eq!(Store::load(&path).files.len(), 2);
        // B detaches b.bib, then A saves: b.bib stays gone.
        b.detach(&p2, "b.bib");
        a.save();
        assert!(!a.is_synced(&p2, "b.bib"));
        let names: Vec<String> = Store::load(&path).files.iter().map(|f| f.rel.clone()).collect();
        assert_eq!(names, vec!["a.bib"]);
        // Disconnecting the account wins over the other app's older view.
        b.disconnect();
        assert!(Store::load(&path).files.is_empty() && Store::load(&path).account.is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn two_apps(name: &str) -> (PathBuf, PathBuf, RefSync, RefSync) {
        let dir = temp_project(name);
        let path = dir.join("reference-sync.json");
        let mut a = RefSync::offline();
        a.use_store(Some(path.clone()));
        a.demo(&dir.join("p1"), "a.bib");
        a.save();
        let mut b = RefSync::offline();
        b.use_store(Some(path.clone()));
        (dir, path, a, b)
    }

    #[test]
    fn a_disconnect_in_one_app_is_not_undone_by_the_other() {
        let (dir, path, mut a, mut b) = two_apps("store-disconnect");
        b.disconnect();
        // A saves after a sync: the account stays disconnected, and A sees it too.
        a.save();
        let disk = Store::load(&path);
        assert!(disk.account.is_none() && disk.files.is_empty());
        assert!(a.account().is_none() && a.key.get() == Some(KeyState::Missing));
        // B saves again later: still disconnected.
        b.save();
        assert!(Store::load(&path).account.is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn files_picked_up_on_a_project_switch_can_still_be_detached_elsewhere() {
        let (dir, path, mut a, mut b) = two_apps("store-known");
        let p2 = dir.join("p2");
        b.demo(&p2, "b.bib");
        b.save();
        // A opens another project: it merges b.bib in without saving.
        a.merge_from_disk();
        assert!(a.is_synced(&p2, "b.bib"));
        b.detach(&p2, "b.bib");
        a.save();
        assert!(!Store::load(&path).files.iter().any(|f| f.rel == "b.bib"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_newer_sync_state_of_the_other_app_is_kept() {
        let (dir, path, mut a, mut b) = two_apps("store-state");
        let p1 = dir.join("p1");
        // B syncs a.bib and records citation keys; A has not touched a.bib since.
        if let Some(f) = b.get_mut(&p1, "a.bib") {
            f.keys.insert("ITEM1".into(), "smith2020".into());
        }
        b.touched.insert((p1.clone(), "a.bib".into()));
        b.save();
        a.save();
        assert_eq!(Store::load(&path).files[0].keys.get("ITEM1").map(String::as_str), Some("smith2020"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_answer_to_an_abandoned_connect_is_dropped() {
        let mut s = RefSync::offline();
        let (tx, rx) = channel();
        s.rx = Some(rx);
        s.connect_generation = 1;
        s.connecting = true;
        // The person disconnected while Zotero was checking the key.
        s.disconnect();
        let account = Account {
            user_id: 9,
            username: "x".into(),
            display_name: String::new(),
        };
        tx.send(Reply::Connected {
            generation: 1,
            result: Ok((account.clone(), "KEY".into())),
        })
        .unwrap();
        assert!(s.poll().is_empty());
        assert!(s.account().is_none() && !s.connecting);
        assert!(s.key.get() == Some(KeyState::Missing));
        // The current attempt is taken.
        tx.send(Reply::Connected {
            generation: s.connect_generation,
            result: Ok((account, "KEY".into())),
        })
        .unwrap();
        assert!(matches!(s.poll().as_slice(), [Event::Connected(_, false)]));
        assert!(s.key.get() == Some(KeyState::Present("KEY".into())));
    }

    #[test]
    fn finder_folder_names_in_decomposed_unicode_are_found() {
        let root = temp_project("nfd");
        let decomposed = "Bu\u{308}cher";
        std::fs::create_dir_all(root.join(decomposed)).unwrap();
        let mut s = RefSync::offline();
        s.store.account = Some(Account {
            user_id: 1,
            username: "u".into(),
            display_name: String::new(),
        });
        s.open_dialog(&root);
        let d = s.dialog.as_mut().unwrap();
        d.name = "Bücher/literatur.bib".into();
        d.choice = Some((LibraryRef::User(1), None));
        let rel = s.create(&root).unwrap();
        // A disk that keeps composed and decomposed names apart must still use the folder
        // that is there, not make a sibling with the typed spelling.
        assert_eq!(rel, format!("{decomposed}/literatur.bib"), "stored as the file list spells it");
        s.prune(&root);
        assert!(s.is_synced(&root, &rel));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn matching_names_folds_unicode_composition_and_case() {
        let names = vec!["Bu\u{308}cher".to_string(), "Literatur.bib".to_string()];
        assert_eq!(matching_name(&names, "Bücher", false).as_deref(), Some("Bu\u{308}cher"));
        assert_eq!(matching_name(&names, "literatur.BIB", true).as_deref(), Some("Literatur.bib"));
        assert_eq!(matching_name(&names, "anderer", true), None);
    }

    #[test]
    fn a_folder_typed_in_another_case_is_found() {
        let root = temp_project("folder-case");
        std::fs::create_dir_all(root.join("Refs")).unwrap();
        let mut s = RefSync::offline();
        s.store.account = Some(Account {
            user_id: 1,
            username: "u".into(),
            display_name: String::new(),
        });
        s.open_dialog(&root);
        let d = s.dialog.as_mut().unwrap();
        d.name = "refs/literatur.bib".into();
        d.choice = Some((LibraryRef::User(1), None));
        let rel = s.create(&root).unwrap();
        // On a case-sensitive disk a new folder "refs" was made; else the file is in "Refs".
        assert!(rel == "Refs/literatur.bib" || rel == "refs/literatur.bib", "{rel}");
        s.prune(&root);
        assert!(s.is_synced(&root, &rel));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn an_account_connected_elsewhere_reads_the_key_again() {
        let (dir, _path, mut a, mut b) = two_apps("store-reconnect");
        b.disconnect();
        a.save();
        assert!(a.key.get() == Some(KeyState::Missing));
        // B connects again.
        b.store.account = Some(Account {
            user_id: 1,
            username: "sven".into(),
            display_name: String::new(),
        });
        b.account_changed = true;
        b.save();
        a.merge_from_disk();
        assert!(a.account().is_some());
        assert!(a.key.get().is_none(), "the key is read from the Keychain again");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_key_replaced_here_survives_an_older_disconnect_elsewhere() {
        let (dir, path, mut a, mut b) = two_apps("store-replace");
        b.disconnect();
        let (tx, rx) = channel();
        a.rx = Some(rx);
        let account = Account {
            user_id: 1,
            username: "sven".into(),
            display_name: String::new(),
        };
        tx.send(Reply::Connected {
            generation: a.connect_generation,
            result: Ok((account, "NEW".into())),
        })
        .unwrap();
        assert!(matches!(a.poll().as_slice(), [Event::Connected(..)]));
        assert!(a.account().is_some() && a.key.get() == Some(KeyState::Present("NEW".into())));
        assert!(Store::load(&path).account.is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn errors_cleared_by_connecting_stay_cleared() {
        let (dir, path, mut a, _b) = two_apps("store-errors");
        if let Some(f) = a.store.files.first_mut() {
            f.error = Some("alt".into());
        }
        a.touched.insert(file_key(&a.store.files[0]));
        a.save();
        let (tx, rx) = channel();
        a.rx = Some(rx);
        let account = Account {
            user_id: 1,
            username: "sven".into(),
            display_name: String::new(),
        };
        tx.send(Reply::Connected {
            generation: a.connect_generation,
            result: Ok((account, "K".into())),
        })
        .unwrap();
        a.poll();
        assert_eq!(Store::load(&path).files[0].error, None);
        assert_eq!(a.store.files[0].error, None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_cancelled_key_check_changes_nothing() {
        let mut s = RefSync::offline();
        let (tx, rx) = channel();
        s.rx = Some(rx);
        s.connecting = true;
        let generation = s.connect_generation;
        s.cancel_connect();
        assert!(!s.connecting);
        let account = Account {
            user_id: 1,
            username: "u".into(),
            display_name: String::new(),
        };
        tx.send(Reply::Connected {
            generation,
            result: Ok((account, "K".into())),
        })
        .unwrap();
        assert!(s.poll().is_empty() && s.account().is_none());
    }

    #[test]
    fn a_file_disconnected_in_another_app_is_not_overwritten() {
        let (dir, _path, mut a, mut b) = two_apps("store-overwrite");
        let p1 = dir.join("p1");
        std::fs::create_dir_all(&p1).unwrap();
        std::fs::write(p1.join("a.bib"), "eigene Notizen").unwrap();
        b.detach(&p1, "a.bib");
        let (tx, rx) = channel();
        a.rx = Some(rx);
        let built = zotero::Built {
            text: "@book{x,\n}\n".into(),
            entries: 1,
            keys: BTreeMap::new(),
        };
        tx.send(Reply::Synced {
            id: (p1.clone(), "a.bib".into()),
            result: Ok(Synced::Content { version: Some(2), built }),
        })
        .unwrap();
        a.poll();
        assert_eq!(std::fs::read_to_string(p1.join("a.bib")).unwrap(), "eigene Notizen");
        assert!(!a.is_synced(&p1, "a.bib"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn subcollections_are_only_stored_when_there_are_some() {
        let root = temp_project("subs");
        let mut s = RefSync::offline();
        s.demo(&root, "x.bib");
        s.store.files.clear();
        for (key, expected) in [("READ", false), ("THESIS", true)] {
            s.open_dialog(&root);
            s.demo_libraries();
            let d = s.dialog.as_mut().unwrap();
            d.name = format!("{key}.bib");
            d.choice = Some((LibraryRef::User(1), Some(key.into())));
            assert!(d.subcollections, "the switch defaults to on");
            let rel = s.create(&root).unwrap();
            assert_eq!(s.get(&root, &rel).unwrap().target.subcollections, expected, "{key}");
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn disconnecting_turns_all_files_into_plain_files() {
        let mut s = RefSync::offline();
        s.demo(Path::new("/p1"), "a.bib");
        s.demo(Path::new("/p2"), "b.bib");
        s.disconnect();
        assert!(s.account().is_none());
        assert!(s.store.files.is_empty());
    }

    #[test]
    fn store_survives_a_round_trip() {
        let mut s = RefSync::offline();
        s.demo(Path::new("/p"), "x.bib");
        let json = serde_json::to_string(&s.store).unwrap();
        assert!(json.contains(r#""library":{"type":"user","id":1}"#));
        let back: Store = serde_json::from_str(&json).unwrap();
        assert_eq!(back.files.len(), 1);
        assert_eq!(back.files[0].target, s.store.files[0].target);
        // Older or partial files still load.
        let partial: Store =
            serde_json::from_str(r#"{"files":[{"project":"/p","rel":"a.bib","target":{"library":{"type":"group","id":3}},"source":"G"}]}"#)
                .unwrap();
        assert_eq!(partial.files[0].target.collection, None);
        assert!(partial.account.is_none());
    }

    /// Compiles `main.typ` in `root` and returns the error messages.
    fn typst_errors(root: &Path) -> Vec<String> {
        let main = crate::compiler::virtual_id("main.typ");
        let world = crate::compiler::EditorWorld::new(root.to_path_buf(), main);
        match typst::compile::<typst_layout::PagedDocument>(&world).output {
            Ok(_) => Vec::new(),
            Err(errors) => errors.iter().map(|e| e.message.to_string()).collect(),
        }
    }

    /// Waits for the background threads and returns the events.
    fn wait_for(s: &mut RefSync, done: impl Fn(&RefSync, &[Event]) -> bool) -> Vec<Event> {
        let mut events = Vec::new();
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(180) {
            events.extend(s.poll());
            if done(s, &events) {
                return events;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        panic!("Zotero did not answer in time");
    }

    /// The whole way, against the real Zotero API (a public group, no key needed):
    /// create a synced file, let it fill, compile a document citing it, sync again.
    /// `cargo test -- --ignored live_`
    #[test]
    #[ignore]
    fn live_create_sync_and_compile() {
        let ctx = egui::Context::default();
        let root = temp_project("live");
        std::fs::write(
            root.join("main.typ"),
            "#set page(width: 12cm, height: auto)\n#bibliography(\"literatur.bib\", full: true)\n",
        )
        .unwrap();
        let mut s = RefSync::offline();
        s.start_threads(&ctx);
        s.key.set(KeyState::Present(String::new()));
        s.store.account = Some(Account {
            user_id: 0,
            username: "test".into(),
            display_name: String::new(),
        });
        s.open_dialog(&root);
        let d = s.dialog.as_mut().unwrap();
        // "Zotero repository proposed" in Zotero's public test library.
        d.choice = Some((LibraryRef::Group(2205533), Some("ECHCPXZN".into())));
        let rel = s.create(&root).expect("create");
        assert!(s.is_busy(&root, &rel));

        let events = wait_for(&mut s, |s, _| !s.is_busy(&root, "literatur.bib"));
        let f = s.get(&root, &rel).unwrap().clone();
        assert_eq!(f.error, None);
        assert!(events.iter().any(|e| matches!(e, Event::Written((_, r)) if *r == rel)));
        assert!(f.entries > 0 && f.version.is_some() && f.last_sync.is_some());
        let text = std::fs::read_to_string(root.join(&rel)).unwrap();
        assert!(text.starts_with("% Automatisch aus Zotero synchronisiert"));
        assert_eq!(biblatex::Bibliography::parse(&text).expect("valid BibLaTeX").len(), f.entries);
        assert_eq!(typst_errors(&root), Vec::<String>::new());

        // Nothing changed in Zotero: no new content, but the sync time moves on.
        s.sync_file(&root, &rel, false);
        let events = wait_for(&mut s, |s, _| !s.is_busy(&root, "literatur.bib"));
        assert!(!events.iter().any(|e| matches!(e, Event::Written(_))));
        assert_eq!(s.get(&root, &rel).unwrap().error, None);

        // An edit made outside Typisch is undone by the next sync.
        std::fs::write(root.join(&rel), "kaputt").unwrap();
        s.sync_file(&root, &rel, false);
        let events = wait_for(&mut s, |s, _| !s.is_busy(&root, "literatur.bib"));
        assert!(events.iter().any(|e| matches!(e, Event::Written(_))));
        assert_eq!(std::fs::read_to_string(root.join(&rel)).unwrap(), text);

        // Disconnected while a sync runs: the file is left alone.
        s.sync_file(&root, &rel, true);
        s.detach(&root, &rel);
        std::fs::write(root.join(&rel), "eigene Notizen").unwrap();
        std::thread::sleep(Duration::from_secs(8));
        let _ = s.poll();
        assert_eq!(std::fs::read_to_string(root.join(&rel)).unwrap(), "eigene Notizen");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Every item of Zotero's public test library (1500+ odd test entries) as one file,
    /// read by Typst. Shows whether Zotero's BibLaTeX and Typst get along.
    #[test]
    #[ignore]
    fn live_whole_test_library_compiles_in_typst() {
        let mut c = Client::new(None);
        let target = Target {
            library: LibraryRef::Group(2205533),
            collection: None,
            subcollections: false,
        };
        let Ok(Fetched::Changed { items, .. }) = c.export(&target, None) else {
            panic!("export failed")
        };
        let built = zotero::build_bib(&header("Test Items Library"), &items, &BTreeMap::new());
        let root = temp_project("whole");
        std::fs::write(root.join("all.bib"), &built.text).unwrap();
        std::fs::write(root.join("main.typ"), "#bibliography(\"all.bib\", full: true)\n").unwrap();
        let errors = typst_errors(&root);
        let _ = std::fs::remove_dir_all(&root);
        assert_eq!(errors, Vec::<String>::new());
    }
}
