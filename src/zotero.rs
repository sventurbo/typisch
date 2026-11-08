//! The Zotero Web API (read-only): checking an API key, listing libraries and collections, and
//! exporting references as BibLaTeX.
//!
//! Everything here blocks; it runs on the reference-sync thread (see `refsync.rs`). The turning
//! of exported entries into one `.bib` file ([`build_bib`]) is plain string work and has no I/O.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt;
use std::io::Read;
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

pub const API_BASE: &str = "https://api.zotero.org";

/// Where a person creates an API key for Typisch, with the needed rights already ticked:
/// read access to their own library and to all their groups, nothing else.
pub const NEW_KEY_URL: &str =
    "https://www.zotero.org/settings/keys/new?name=Typisch&library_access=1&notes_access=0&write_access=0&all_groups=read";

/// The page listing a person's API keys (to revoke the one Typisch uses).
pub const KEYS_URL: &str = "https://www.zotero.org/settings/keys";

/// Objects per request; the Web API sends at most 100.
const PAGE: usize = 100;

/// A Zotero library: someone's own ("Meine Bibliothek") or a group's.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "type", content = "id", rename_all = "lowercase")]
pub enum LibraryRef {
    User(u64),
    Group(u64),
}

impl LibraryRef {
    /// The start of every API path for this library.
    pub fn prefix(self) -> String {
        match self {
            LibraryRef::User(id) => format!("/users/{id}"),
            LibraryRef::Group(id) => format!("/groups/{id}"),
        }
    }
}

/// What a synced file contains: a whole library, or one collection (optionally with the
/// collections inside it, as Zotero's own "Sammlung exportieren …" does).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Target {
    pub library: LibraryRef,
    /// Key of the collection, `None` for the whole library.
    #[serde(default)]
    pub collection: Option<String>,
    #[serde(default)]
    pub subcollections: bool,
}

// ---------------------------------------------------------------------------- errors

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// 403: the key is wrong, was revoked, or may not read this library.
    Forbidden,
    /// 404: the library or collection does not exist (any more).
    NotFound,
    /// 429 / 503 even after waiting as asked.
    Busy,
    /// Any other unexpected HTTP status.
    Status(u16),
    /// No connection, timeout, TLS problem …
    Network(String),
    /// An answer that could not be understood.
    Invalid(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Forbidden => write!(f, "Zotero verweigert den Zugriff. Der API-Schlüssel ist ungültig, wurde widerrufen oder darf diese Bibliothek nicht lesen."),
            Error::NotFound => write!(f, "Die Bibliothek oder Sammlung wurde in Zotero nicht gefunden (gelöscht oder kein Zugriff mehr)."),
            Error::Busy => write!(f, "Zotero ist gerade überlastet oder wird gewartet. Später erneut versuchen."),
            Error::Status(s) => write!(f, "Zotero antwortet mit Fehler {s}."),
            Error::Network(e) => write!(f, "Keine Verbindung zu Zotero: {e}"),
            Error::Invalid(e) => write!(f, "Unerwartete Antwort von Zotero: {e}"),
        }
    }
}

pub type Result<T> = std::result::Result<T, Error>;

// ---------------------------------------------------------------------------- transport

/// One HTTP answer, with header names in lower case.
pub struct Response {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: String,
}

impl Response {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.trim())
    }

    fn header_num<T: std::str::FromStr>(&self, name: &str) -> Option<T> {
        self.header(name).and_then(|v| v.parse().ok())
    }
}

/// Sends GET requests. The real one talks HTTPS; tests answer from a script.
pub trait Transport: Send {
    fn get(&self, url: &str, headers: &[(&'static str, String)]) -> Result<Response>;
}

/// HTTPS with the system's TLS (and proxy settings from the environment).
pub struct Http {
    agent: ureq::Agent,
}

impl Http {
    pub fn new() -> Self {
        let mut builder = ureq::AgentBuilder::new()
            .timeout_connect(Duration::from_secs(15))
            .timeout_read(Duration::from_secs(60))
            .user_agent(&format!("Typisch/{}", env!("CARGO_PKG_VERSION")))
            // The key travels in a header; it must never follow a redirect or leave HTTPS.
            .redirects(0)
            .https_only(true);
        if let Ok(tls) = native_tls::TlsConnector::new() {
            builder = builder.tls_connector(Arc::new(tls));
        }
        Self { agent: builder.build() }
    }
}

impl Transport for Http {
    fn get(&self, url: &str, headers: &[(&'static str, String)]) -> Result<Response> {
        let mut req = self.agent.get(url);
        for (k, v) in headers {
            req = req.set(k, v);
        }
        let resp = match req.call() {
            Ok(r) => r,
            Err(ureq::Error::Status(_, r)) => r,
            // A header ureq refuses would be quoted in its message: never show it.
            Err(ureq::Error::Transport(t)) if t.kind() == ureq::ErrorKind::BadHeader => {
                return Err(Error::Invalid("Ungültige Zeichen in der Anfrage".into()));
            }
            Err(ureq::Error::Transport(t)) => return Err(Error::Network(t.to_string())),
        };
        let status = resp.status();
        let headers = resp
            .headers_names()
            .into_iter()
            .filter_map(|n| resp.header(&n).map(|v| (n.to_ascii_lowercase(), v.to_string())))
            .collect();
        let mut body = String::new();
        // One page is at most a few hundred kilobytes; the cap only guards against nonsense.
        resp.into_reader()
            .take(64 * 1024 * 1024)
            .read_to_string(&mut body)
            .map_err(|e| Error::Network(e.to_string()))?;
        Ok(Response { status, headers, body })
    }
}

// ---------------------------------------------------------------------------- client

/// What an API key may do, from `GET /keys/current`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyInfo {
    pub user_id: u64,
    pub username: String,
    pub display_name: String,
    /// May read the person's own library.
    pub library: bool,
    /// May read (some of) their groups.
    pub groups: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Collection {
    pub key: String,
    pub name: String,
    pub parent: Option<String>,
    pub items: u64,
}

/// The answer to an export: nothing changed since the given version, or all entries.
#[derive(Debug, PartialEq, Eq)]
pub enum Fetched {
    NotModified,
    Changed {
        /// The library version the entries belong to (`Last-Modified-Version`).
        version: Option<u64>,
        /// (Zotero item key, BibLaTeX entry), oldest first, each item once.
        items: Vec<(String, String)>,
    },
}

enum Paged<V> {
    NotModified,
    Done { version: Option<u64>, items: Vec<V> },
}

pub struct Client<T: Transport = Http> {
    transport: T,
    key: Option<String>,
    base: String,
    /// Zotero asked us (via `Backoff`) not to send anything before this moment.
    quiet_until: Option<Instant>,
    sleep: Box<dyn Fn(Duration) + Send>,
}

impl Client<Http> {
    pub fn new(key: Option<String>) -> Self {
        Self::with_transport(Http::new(), key)
    }
}

impl<T: Transport> Client<T> {
    pub fn with_transport(transport: T, key: Option<String>) -> Self {
        Self { transport, key, base: API_BASE.to_string(), quiet_until: None, sleep: Box::new(std::thread::sleep) }
    }

    #[cfg(test)]
    fn with_sleep(mut self, sleep: impl Fn(Duration) + Send + 'static) -> Self {
        self.sleep = Box::new(sleep);
        self
    }

    /// One request, waiting and retrying as Zotero asks (Backoff, 429 / 503 with Retry-After).
    /// Returns 2xx and 304 answers; every other status becomes an [`Error`].
    fn request(&mut self, path: &str, since: Option<u64>) -> Result<Response> {
        let url = format!("{}{}", self.base, path);
        let mut retries = 0u32;
        loop {
            if let Some(until) = self.quiet_until.take() {
                let now = Instant::now();
                if until > now {
                    (self.sleep)(until - now);
                }
            }
            let mut headers = vec![("Zotero-API-Version", "3".to_string())];
            if let Some(k) = &self.key {
                headers.push(("Zotero-API-Key", k.clone()));
            }
            if let Some(v) = since {
                headers.push(("If-Modified-Since-Version", v.to_string()));
            }
            let resp = self.transport.get(&url, &headers)?;
            if let Some(secs) = resp.header_num::<u64>("backoff") {
                self.quiet_until = Some(Instant::now() + Duration::from_secs(secs.min(3600)));
            }
            match resp.status {
                200..=299 | 304 => return Ok(resp),
                429 | 503 if retries < 3 => {
                    retries += 1;
                    let wait = resp.header_num::<u64>("retry-after").unwrap_or(2u64.pow(retries + 1));
                    (self.sleep)(Duration::from_secs(wait.clamp(1, 300)));
                }
                429 | 503 => return Err(Error::Busy),
                401 | 403 => return Err(Error::Forbidden),
                404 => return Err(Error::NotFound),
                s => return Err(Error::Status(s)),
            }
        }
    }

    fn json<V: DeserializeOwned>(resp: &Response) -> Result<V> {
        serde_json::from_str(&resp.body).map_err(|e| Error::Invalid(e.to_string()))
    }

    /// Every object of a list endpoint, page by page. With `since`, a `304` on the first page
    /// means nothing in the library changed. If the library changes while paging, starts over
    /// (twice at most; after that the older version is reported, so the next sync fetches again).
    fn get_all<V: DeserializeOwned>(&mut self, path: &str, since: Option<u64>) -> Result<Paged<V>> {
        let sep = if path.contains('?') { '&' } else { '?' };
        let mut attempt = 0;
        loop {
            attempt += 1;
            let mut items: Vec<V> = Vec::new();
            let mut first_version: Option<u64> = None;
            let mut start = 0usize;
            let mut moved = false;
            loop {
                let page_since = if start == 0 { since } else { None };
                let resp = self.request(&format!("{path}{sep}limit={PAGE}&start={start}"), page_since)?;
                if resp.status == 304 {
                    return Ok(Paged::NotModified);
                }
                let version = resp.header_num::<u64>("last-modified-version");
                if start == 0 {
                    first_version = version;
                } else if version.is_some() && version != first_version {
                    moved = true;
                }
                let page: Vec<V> = Self::json(&resp)?;
                let n = page.len();
                items.extend(page);
                start += n;
                let total = resp.header_num::<usize>("total-results");
                let done = n == 0 || total.map(|t| start >= t).unwrap_or(n < PAGE);
                if done {
                    break;
                }
            }
            if !moved || attempt >= 3 {
                return Ok(Paged::Done { version: first_version, items });
            }
        }
    }

    /// Who the key belongs to and what it may read.
    pub fn key_info(&mut self) -> Result<KeyInfo> {
        #[derive(Deserialize)]
        struct KeyJson {
            #[serde(rename = "userID")]
            user_id: u64,
            #[serde(default)]
            username: String,
            #[serde(default, rename = "displayName")]
            display_name: String,
            #[serde(default)]
            access: serde_json::Value,
        }
        let resp = self.request("/keys/current", None)?;
        let k: KeyJson = Self::json(&resp)?;
        let flag = |v: &serde_json::Value| v.get("library").and_then(|b| b.as_bool()).unwrap_or(false);
        let library = k.access.get("user").map(flag).unwrap_or(false);
        let groups = k
            .access
            .get("groups")
            .and_then(|g| g.as_object())
            .map(|g| g.values().any(flag))
            .unwrap_or(false);
        Ok(KeyInfo { user_id: k.user_id, username: k.username, display_name: k.display_name, library, groups })
    }

    /// The groups the person belongs to (id, name), sorted by name.
    pub fn groups(&mut self, user_id: u64) -> Result<Vec<(u64, String)>> {
        #[derive(Deserialize)]
        struct GroupData {
            #[serde(default)]
            name: String,
        }
        #[derive(Deserialize)]
        struct GroupJson {
            id: u64,
            data: GroupData,
        }
        let Paged::Done { items, .. } = self.get_all::<GroupJson>(&format!("/users/{user_id}/groups"), None)? else {
            return Ok(Vec::new());
        };
        let mut out: Vec<(u64, String)> = items.into_iter().map(|g| (g.id, g.data.name)).collect();
        out.sort_by_key(|(_, n)| n.to_lowercase());
        Ok(out)
    }

    /// All collections of a library (in no particular order).
    pub fn collections(&mut self, library: LibraryRef) -> Result<Vec<Collection>> {
        #[derive(Deserialize)]
        struct Data {
            #[serde(default)]
            name: String,
            #[serde(default, rename = "parentCollection")]
            parent: serde_json::Value,
            /// In Zotero's trash.
            #[serde(default)]
            deleted: serde_json::Value,
        }
        #[derive(Deserialize, Default)]
        struct Meta {
            #[serde(default, rename = "numItems")]
            items: u64,
        }
        #[derive(Deserialize)]
        struct CollJson {
            key: String,
            data: Data,
            #[serde(default)]
            meta: Meta,
        }
        let Paged::Done { items, .. } = self.get_all::<CollJson>(&format!("{}/collections", library.prefix()), None)? else {
            return Ok(Vec::new());
        };
        let trashed: HashSet<String> = items
            .iter()
            .filter(|c| c.data.deleted.as_bool().unwrap_or(false) || c.data.deleted.as_u64().map(|n| n != 0).unwrap_or(false))
            .map(|c| c.key.clone())
            .collect();
        let all: Vec<Collection> = items
            .into_iter()
            .map(|c| Collection {
                key: c.key,
                name: c.data.name,
                // `false` for top-level collections, else the parent's key.
                parent: c.data.parent.as_str().map(str::to_string),
                items: c.meta.items,
            })
            .collect();
        Ok(without_trashed(all, &trashed))
    }

    /// All top-level items of the target as BibLaTeX, unless nothing changed since `since`.
    pub fn export(&mut self, target: &Target, since: Option<u64>) -> Result<Fetched> {
        let prefix = target.library.prefix();
        let query = "format=json&include=biblatex&sort=dateAdded&direction=asc";
        let Some(root) = target.collection.as_deref() else {
            return self.export_path(&format!("{prefix}/items/top?{query}"), since);
        };
        if !target.subcollections {
            return self.export_path(&format!("{prefix}/collections/{root}/items/top?{query}"), since);
        }
        // Several collections: one cheap request tells whether the library changed at all
        // (versions belong to the whole library), then every collection is fetched.
        let probe = self.request(&format!("{prefix}/items/top?limit=1&format=keys"), since)?;
        if probe.status == 304 {
            return Ok(Fetched::NotModified);
        }
        let version = probe.header_num::<u64>("last-modified-version");
        let all = self.collections(target.library)?;
        if !all.iter().any(|c| c.key == root) {
            return Err(Error::NotFound);
        }
        let mut items = Vec::new();
        let mut seen = HashSet::new();
        for key in subtree(&all, root) {
            let path = format!("{prefix}/collections/{key}/items/top?{query}");
            match self.export_path(&path, None) {
                Ok(Fetched::Changed { items: found, .. }) => {
                    for (k, entry) in found {
                        if seen.insert(k.clone()) {
                            items.push((k, entry));
                        }
                    }
                }
                Ok(Fetched::NotModified) => {}
                // A subcollection deleted in the meantime: its items are simply gone.
                Err(Error::NotFound) if key != root => {}
                Err(e) => return Err(e),
            }
        }
        Ok(Fetched::Changed { version, items })
    }

    fn export_path(&mut self, path: &str, since: Option<u64>) -> Result<Fetched> {
        #[derive(Deserialize)]
        struct ItemJson {
            key: String,
            #[serde(default)]
            biblatex: Option<String>,
        }
        match self.get_all::<ItemJson>(path, since)? {
            Paged::NotModified => Ok(Fetched::NotModified),
            Paged::Done { version, items } => {
                let mut seen = HashSet::new();
                let items = items
                    .into_iter()
                    // Notes and attachments have no BibLaTeX form.
                    .filter_map(|i| i.biblatex.filter(|b| !b.trim().is_empty()).map(|b| (i.key, b)))
                    // An item can show up twice when the library changed while paging.
                    .filter(|(k, _)| seen.insert(k.clone()))
                    .collect();
                Ok(Fetched::Changed { version, items })
            }
        }
    }
}

/// Leaves out the trashed collections and everything inside them (Zotero's own export does too).
fn without_trashed(all: Vec<Collection>, trashed: &HashSet<String>) -> Vec<Collection> {
    if trashed.is_empty() {
        return all;
    }
    let parent: HashMap<&str, &str> = all.iter().filter_map(|c| c.parent.as_deref().map(|p| (c.key.as_str(), p))).collect();
    let gone = |key: &str| {
        let mut seen = HashSet::new();
        let mut k = key;
        loop {
            if trashed.contains(k) {
                return true;
            }
            match parent.get(k) {
                Some(p) if seen.insert(k) => k = p,
                _ => return false,
            }
        }
    };
    let keep: Vec<bool> = all.iter().map(|c| !gone(&c.key)).collect();
    all.into_iter().zip(keep).filter(|(_, k)| *k).map(|(c, _)| c).collect()
}

/// `root` and every collection below it, parents before children, siblings by name.
pub fn subtree(all: &[Collection], root: &str) -> Vec<String> {
    let mut children: HashMap<&str, Vec<&Collection>> = HashMap::new();
    for c in all {
        if let Some(p) = c.parent.as_deref() {
            children.entry(p).or_default().push(c);
        }
    }
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    let mut stack = vec![root.to_string()];
    while let Some(key) = stack.pop() {
        // Guards against a (corrupt) parent cycle.
        if !seen.insert(key.clone()) {
            continue;
        }
        if let Some(kids) = children.get(key.as_str()) {
            let mut kids = kids.clone();
            kids.sort_by_key(|c| std::cmp::Reverse(c.name.to_lowercase()));
            stack.extend(kids.iter().map(|c| c.key.clone()));
        }
        out.push(key);
    }
    out
}

// ---------------------------------------------------------------------------- .bib file

/// The finished `.bib` file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Built {
    pub text: String,
    pub entries: usize,
    /// Item key → citation key of every entry, so each item keeps its key next time: a
    /// newcomer with the same key gets the `-1`, not the item that is already cited.
    pub keys: BTreeMap<String, String>,
}

/// A citation key the BibLaTeX parser accepts: no spaces, braces, commas … (as typed into
/// Zotero's "Citation Key" field, anything is possible).
fn clean_key(key: &str) -> String {
    let bad = |c: char| c.is_whitespace() || c.is_control() || "@{}\"#'(),=%\\~".contains(c);
    let mut out: String = key.chars().map(|c| if bad(c) { '_' } else { c }).collect();
    if out.starts_with([':', '<', '-', '>']) {
        out.insert(0, '_');
    }
    out
}

/// Where the citation key sits in an entry: `@type{key,` → byte range of `key`.
fn key_range(entry: &str) -> Option<std::ops::Range<usize>> {
    let rest = entry.strip_prefix('@')?;
    let open = rest.find(['{', '('])?;
    let kind = &rest[..open];
    if kind.is_empty() || !kind.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
        return None;
    }
    let after = 1 + open + 1;
    // Zotero writes `@type{key,` on a line of its own, so the key ends at the last comma of that
    // line, even when it contains commas or braces itself (anything can be typed into Zotero's
    // "Citation Key" field; `clean_key` makes it parseable). Without a line break: the first comma.
    let comma = match entry[after..].find('\n') {
        Some(eol) => entry[after..after + eol].rfind(',')? + after,
        None => entry[after..].find(',')? + after,
    };
    let raw = &entry[after..comma];
    let start = after + (raw.len() - raw.trim_start().len());
    let end = comma - (raw.len() - raw.trim_end().len());
    (start < end).then_some(start..end)
}

/// The key an item keeps from last time, if `prev` is `base` or `base-<number>` (case aside),
/// spelled like `base`: when a key is changed to `Smith2020` in Zotero, the file must say
/// `Smith2020` (or `Smith2020-1`), not keep the old `smith2020`.
fn carried(prev: &str, base: &str) -> Option<String> {
    if prev.to_lowercase() == base.to_lowercase() {
        return Some(base.to_string());
    }
    let (stem, n) = prev.rsplit_once('-')?;
    let numbered = !n.is_empty() && n.chars().all(|c| c.is_ascii_digit());
    (numbered && stem.to_lowercase() == base.to_lowercase()).then(|| format!("{base}-{n}"))
}

/// Joins exported entries into one `.bib` file. Citation keys must be unique (Typst refuses a
/// file with a key twice); clashes get `-1`, `-2` … like Zotero's own export, and an item keeps
/// the key it got last time (`previous`) as long as it is still free.
pub fn build_bib(header: &str, items: &[(String, String)], previous: &BTreeMap<String, String>) -> Built {
    struct Entry<'a> {
        item: &'a str,
        text: &'a str,
        /// Where the key is in `text`.
        range: Option<std::ops::Range<usize>>,
        /// The key as Zotero made it, made parseable.
        base: String,
        key: Option<String>,
    }
    let mut entries: Vec<Entry> = items
        .iter()
        .filter_map(|(item, text)| {
            let text = text.trim();
            if !text.starts_with('@') {
                return None;
            }
            let range = key_range(text);
            let base = range.clone().map(|r| clean_key(&text[r])).unwrap_or_default();
            Some(Entry { item, text, range, base, key: None })
        })
        .collect();

    // Keys in use, without case (BibTeX tools treat keys case-insensitively).
    let mut used: HashSet<String> = HashSet::new();
    // 1. Items keep the key they had, when it still fits their (possibly changed) base key. A
    //    change of case only keeps the place too, in the new spelling: a newcomer must not take
    //    over the key that is cited.
    for e in entries.iter_mut().filter(|e| e.range.is_some()) {
        if let Some(prev) = previous.get(e.item)
            && let Some(kept) = carried(prev, &e.base)
            && used.insert(kept.to_lowercase())
        {
            e.key = Some(kept);
        }
    }
    // 2. Everyone else takes their own key if it is free …
    for e in entries.iter_mut().filter(|e| e.range.is_some() && e.key.is_none()) {
        if used.insert(e.base.to_lowercase()) {
            e.key = Some(e.base.clone());
        }
    }
    // 3. … or the next free numbered one.
    for e in entries.iter_mut().filter(|e| e.range.is_some() && e.key.is_none()) {
        let mut n = 1;
        loop {
            let candidate = format!("{}-{n}", e.base);
            if used.insert(candidate.to_lowercase()) {
                e.key = Some(candidate);
                break;
            }
            n += 1;
        }
    }

    let keys: BTreeMap<String, String> = entries.iter().filter_map(|e| e.key.clone().map(|k| (e.item.to_string(), k))).collect();
    // Sorted by key, so the file reads like a list and changes little between syncs.
    entries.sort_by(|a, b| {
        let ka = a.key.as_deref().unwrap_or("").to_lowercase();
        let kb = b.key.as_deref().unwrap_or("").to_lowercase();
        ka.cmp(&kb).then_with(|| a.item.cmp(b.item))
    });
    let mut text = String::from(header);
    if !text.is_empty() && !text.ends_with('\n') {
        text.push('\n');
    }
    for e in &entries {
        text.push('\n');
        match (&e.range, &e.key) {
            (Some(r), Some(k)) if *k != e.text[r.clone()] => {
                text.push_str(&e.text[..r.start]);
                text.push_str(k);
                text.push_str(&e.text[r.end..]);
            }
            _ => text.push_str(e.text),
        }
        text.push('\n');
    }
    Built { text, entries: entries.len(), keys }
}

// ---------------------------------------------------------------------------- tests

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// (URL part, status, headers, body)
    type Answer = (String, u16, Vec<(String, String)>, String);
    type Log = Arc<Mutex<Vec<(String, Vec<(&'static str, String)>)>>>;

    /// Answers requests from a list of answers, the first whose URL part matches.
    struct Script {
        answers: Mutex<Vec<Answer>>,
        log: Log,
    }

    impl Script {
        #[allow(clippy::type_complexity)]
        fn new(answers: Vec<(&str, u16, Vec<(&str, &str)>, &str)>) -> (Self, Log) {
            let log = Arc::new(Mutex::new(Vec::new()));
            let answers = answers
                .into_iter()
                .map(|(u, s, h, b)| (u.to_string(), s, h.into_iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(), b.to_string()))
                .collect();
            (Self { answers: Mutex::new(answers), log: log.clone() }, log)
        }
    }

    impl Transport for Script {
        fn get(&self, url: &str, headers: &[(&'static str, String)]) -> Result<Response> {
            self.log.lock().unwrap().push((url.to_string(), headers.to_vec()));
            let mut answers = self.answers.lock().unwrap();
            let pos = answers.iter().position(|(part, ..)| url.contains(part.as_str())).unwrap_or_else(|| panic!("unexpected request {url}"));
            let (_, status, headers, body) = answers.remove(pos);
            Ok(Response { status, headers, body })
        }
    }

    fn item(key: &str, bib: &str) -> String {
        format!(r#"{{"key":"{key}","version":1,"biblatex":{}}}"#, serde_json::to_string(bib).unwrap())
    }

    #[test]
    fn sends_version_and_key_headers() {
        let (t, log) = Script::new(vec![("/keys/current", 200, vec![], r#"{"key":"x","userID":7,"username":"sven","access":{"user":{"library":true},"groups":{"all":{"library":true,"write":false}}}}"#)]);
        let info = Client::with_transport(t, Some("secret".into())).key_info().unwrap();
        assert_eq!(info, KeyInfo { user_id: 7, username: "sven".into(), display_name: String::new(), library: true, groups: true });
        let log = log.lock().unwrap();
        assert!(log[0].1.contains(&("Zotero-API-Version", "3".into())));
        assert!(log[0].1.contains(&("Zotero-API-Key", "secret".into())));
    }

    #[test]
    fn maps_error_statuses() {
        let (t, _) = Script::new(vec![("/keys/current", 403, vec![], "Invalid key")]);
        assert_eq!(Client::with_transport(t, Some("bad".into())).key_info(), Err(Error::Forbidden));
        let (t, _) = Script::new(vec![("/collections/GONE", 404, vec![], "Not found")]);
        let target = Target { library: LibraryRef::User(1), collection: Some("GONE".into()), subcollections: false };
        assert_eq!(Client::with_transport(t, None).export(&target, None), Err(Error::NotFound));
    }

    #[test]
    fn not_modified_when_library_unchanged() {
        let (t, log) = Script::new(vec![("/users/1/items/top", 304, vec![("Last-Modified-Version", "50")], "")]);
        let target = Target { library: LibraryRef::User(1), collection: None, subcollections: false };
        assert_eq!(Client::with_transport(t, None).export(&target, Some(50)).unwrap(), Fetched::NotModified);
        assert!(log.lock().unwrap()[0].1.contains(&("If-Modified-Since-Version", "50".into())));
    }

    #[test]
    fn pages_through_all_items_and_skips_notes() {
        let page1: Vec<String> = (0..100).map(|i| item(&format!("K{i:03}"), &format!("@book{{b{i},\n\ttitle = {{T}},\n}}"))).collect();
        let body1 = format!("[{}]", page1.join(","));
        let body2 = format!("[{},{}]", item("K100", "@misc{last,\n}"), r#"{"key":"NOTE","version":1,"biblatex":""}"#);
        let (t, log) = Script::new(vec![
            ("start=0", 200, vec![("Total-Results", "102"), ("Last-Modified-Version", "9")], &body1),
            ("start=100", 200, vec![("Total-Results", "102"), ("Last-Modified-Version", "9")], &body2),
        ]);
        let target = Target { library: LibraryRef::Group(5), collection: None, subcollections: false };
        let Fetched::Changed { version, items } = Client::with_transport(t, None).export(&target, Some(3)).unwrap() else { panic!() };
        assert_eq!(version, Some(9));
        assert_eq!(items.len(), 101);
        let log = log.lock().unwrap();
        assert!(log[0].0.starts_with("https://api.zotero.org/groups/5/items/top?format=json&include=biblatex"));
        // Only the first page is conditional.
        assert!(log[1].1.iter().all(|(k, _)| *k != "If-Modified-Since-Version"));
    }

    #[test]
    fn starts_over_when_library_changes_while_paging() {
        let full: Vec<String> = (0..100).map(|i| item(&format!("A{i}"), "@misc{a,\n}")).collect();
        let full = format!("[{}]", full.join(","));
        let (t, log) = Script::new(vec![
            ("start=0", 200, vec![("Total-Results", "101"), ("Last-Modified-Version", "1")], &full),
            ("start=100", 200, vec![("Total-Results", "101"), ("Last-Modified-Version", "2")], "[]"),
            ("start=0", 200, vec![("Total-Results", "1"), ("Last-Modified-Version", "2")], r#"[{"key":"Z","biblatex":"@misc{z,\n}"}]"#),
        ]);
        let target = Target { library: LibraryRef::User(1), collection: None, subcollections: false };
        let got = Client::with_transport(t, None).export(&target, None).unwrap();
        assert_eq!(got, Fetched::Changed { version: Some(2), items: vec![("Z".into(), "@misc{z,\n}".into())] });
        assert_eq!(log.lock().unwrap().len(), 3);
    }

    #[test]
    fn waits_when_rate_limited() {
        let (t, _) = Script::new(vec![
            ("/keys/current", 429, vec![("Retry-After", "7")], ""),
            ("/keys/current", 200, vec![("Backoff", "30")], r#"{"userID":1}"#),
        ]);
        let slept = Arc::new(Mutex::new(Vec::new()));
        let s = slept.clone();
        let mut c = Client::with_transport(t, None).with_sleep(move |d| s.lock().unwrap().push(d.as_secs()));
        assert_eq!(c.key_info().unwrap().user_id, 1);
        assert_eq!(*slept.lock().unwrap(), vec![7]);
        assert!(c.quiet_until.is_some());
    }

    #[test]
    fn gives_up_after_repeated_overload() {
        let (t, _) = Script::new(vec![("/keys/current", 503, vec![], ""); 4].into_iter().collect());
        let mut c = Client::with_transport(t, None).with_sleep(|_| {});
        assert_eq!(c.key_info(), Err(Error::Busy));
    }

    #[test]
    fn collection_with_subcollections_merges_items_once() {
        let colls = r#"[
            {"key":"ROOT","data":{"name":"Thesis","parentCollection":false},"meta":{"numItems":1}},
            {"key":"KID","data":{"name":"Kapitel 1","parentCollection":"ROOT"},"meta":{"numItems":2}},
            {"key":"OTHER","data":{"name":"Anderes","parentCollection":false},"meta":{"numItems":9}}
        ]"#;
        let (t, log) = Script::new(vec![
            ("items/top?limit=1&format=keys", 200, vec![("Last-Modified-Version", "77")], "A"),
            ("/collections?limit", 200, vec![("Total-Results", "3")], colls),
            ("/collections/ROOT/items/top", 200, vec![("Total-Results", "1")], &format!("[{}]", item("A", "@book{a,\n}"))),
            ("/collections/KID/items/top", 200, vec![("Total-Results", "2")], &format!("[{},{}]", item("A", "@book{a,\n}"), item("B", "@book{b,\n}"))),
        ]);
        let target = Target { library: LibraryRef::User(1), collection: Some("ROOT".into()), subcollections: true };
        let got = Client::with_transport(t, None).export(&target, Some(70)).unwrap();
        assert_eq!(got, Fetched::Changed { version: Some(77), items: vec![("A".into(), "@book{a,\n}".into()), ("B".into(), "@book{b,\n}".into())] });
        assert!(!log.lock().unwrap().iter().any(|(u, _)| u.contains("OTHER")));
    }

    #[test]
    fn subtree_orders_parents_first_and_survives_cycles() {
        let c = |k: &str, n: &str, p: Option<&str>| Collection { key: k.into(), name: n.into(), parent: p.map(str::to_string), items: 0 };
        let all = vec![c("R", "Root", None), c("B", "b", Some("R")), c("A", "a", Some("R")), c("A1", "x", Some("A")), c("X", "loop", Some("Y")), c("Y", "loop", Some("X"))];
        assert_eq!(subtree(&all, "R"), vec!["R", "A", "A1", "B"]);
        assert_eq!(subtree(&all, "X"), vec!["X", "Y"]);
    }

    #[test]
    fn finds_the_citation_key() {
        let e = "@article{smith2020,\n\ttitle = {x},\n}";
        assert_eq!(&e[key_range(e).unwrap()], "smith2020");
        let e = "@book{ spaced ,\n}";
        assert_eq!(&e[key_range(e).unwrap()], "spaced");
        assert_eq!(key_range("@comment{just text}"), None);
        assert_eq!(key_range("not an entry"), None);
        // Keys typed into Zotero with commas or braces still end at the end of the first line.
        let e = "@book{Smith, 2020,\n\ttitle = {x},\n}";
        assert_eq!(&e[key_range(e).unwrap()], "Smith, 2020");
        let e = "@book{{Smith2020},\n\ttitle = {x},\n}";
        assert_eq!(&e[key_range(e).unwrap()], "{Smith2020}");
    }

    #[test]
    fn clashing_keys_get_numbers_and_keep_them() {
        let items = vec![
            ("I1".to_string(), "\n@book{smith2020,\n\ttitle = {One},\n}".to_string()),
            ("I2".to_string(), "@article{Smith2020,\n\ttitle = {Two},\n}".to_string()),
            ("I3".to_string(), "@book{doe2019,\n\ttitle = {Three},\n}".to_string()),
            ("I4".to_string(), "@book{smith2020,\n\ttitle = {Four},\n}".to_string()),
        ];
        let first = build_bib("% head", &items, &BTreeMap::new());
        assert_eq!(first.entries, 4);
        assert!(first.text.starts_with("% head\n\n@book{doe2019,"));
        assert!(first.text.contains("@book{smith2020,\n\ttitle = {One}"));
        assert!(first.text.contains("@article{Smith2020-1,\n\ttitle = {Two}"));
        assert!(first.text.contains("@book{smith2020-2,\n\ttitle = {Four}"));
        // Every entry's key is remembered.
        assert_eq!(first.keys.len(), 4);
        assert_eq!(first.keys["I3"], "doe2019");

        // I1 was deleted and a new, older-sorting item appears: the others keep their keys.
        let items2 = vec![
            ("I0".to_string(), "@book{smith2020,\n\ttitle = {New},\n}".to_string()),
            items[1].clone(),
            items[3].clone(),
        ];
        let second = build_bib("", &items2, &first.keys);
        assert!(second.text.contains("@article{Smith2020-1,\n\ttitle = {Two}"));
        assert!(second.text.contains("@book{smith2020-2,\n\ttitle = {Four}"));
        assert!(second.text.contains("@book{smith2020,\n\ttitle = {New}"));
    }

    #[test]
    fn a_changed_base_key_drops_the_old_number() {
        let mut prev = BTreeMap::new();
        prev.insert("I2".to_string(), "smith2020-1".to_string());
        let items = vec![("I2".to_string(), "@book{jones2021,\n}".to_string())];
        let built = build_bib("", &items, &prev);
        assert!(built.text.contains("@book{jones2021,"));
        assert_eq!(built.keys["I2"], "jones2021");
    }

    #[test]
    fn an_older_newcomer_does_not_take_a_cited_key() {
        // A is in the file as smith2020; B (added to Zotero earlier, so listed first) joins.
        let a = ("A".to_string(), "@book{smith2020,\n\ttitle = {A},\n}".to_string());
        let b = ("B".to_string(), "@book{smith2020,\n\ttitle = {B},\n}".to_string());
        let first = build_bib("", std::slice::from_ref(&a), &BTreeMap::new());
        let second = build_bib("", &[b, a], &first.keys);
        assert!(second.text.contains("@book{smith2020,\n\ttitle = {A}"));
        assert!(second.text.contains("@book{smith2020-1,\n\ttitle = {B}"));
    }

    #[test]
    fn a_kept_number_survives_when_the_clash_is_gone() {
        let items = vec![
            ("I1".to_string(), "@book{k,\n}".to_string()),
            ("I2".to_string(), "@book{k,\n\tnote = {2},\n}".to_string()),
        ];
        let first = build_bib("", &items, &BTreeMap::new());
        assert_eq!(first.keys["I2"], "k-1");
        // I1 is deleted, then something unrelated changes: I2 keeps k-1, which is cited.
        let second = build_bib("", &items[1..], &first.keys);
        let third = build_bib("", &items[1..], &second.keys);
        assert!(third.text.contains("@book{k-1,"));
    }

    #[test]
    fn odd_keys_are_made_parseable() {
        let items = vec![
            ("I1".to_string(), "@book{Smith 2020,\n\ttitle = {A},\n}".to_string()),
            ("I2".to_string(), "@book{-odd,\n\ttitle = {B},\n}".to_string()),
            ("I3".to_string(), "@book{Doe, 2021,\n\ttitle = {C},\n}".to_string()),
            ("I4".to_string(), "@book{{Roe}2022,\n\ttitle = {D},\n}".to_string()),
        ];
        let built = build_bib("", &items, &BTreeMap::new());
        let bib = biblatex::Bibliography::parse(&built.text).expect("valid BibLaTeX");
        for key in ["Smith_2020", "_-odd", "Doe__2021", "_Roe_2022"] {
            assert!(bib.get(key).is_some(), "{key} missing in\n{}", built.text);
        }
    }

    #[test]
    fn a_case_change_does_not_hand_the_cited_key_to_a_newcomer() {
        let i1 = ("I1".to_string(), "@book{smith2020,\n\ttitle = {Cited},\n}".to_string());
        let first = build_bib("", std::slice::from_ref(&i1), &BTreeMap::new());
        // I1's key becomes Smith2020; I3 (listed first) has the old spelling.
        let i1 = ("I1".to_string(), "@book{Smith2020,\n\ttitle = {Cited},\n}".to_string());
        let i3 = ("I3".to_string(), "@book{smith2020,\n\ttitle = {New},\n}".to_string());
        let second = build_bib("", &[i3, i1], &first.keys);
        assert!(second.text.contains("@book{Smith2020,\n\ttitle = {Cited}"), "{}", second.text);
        assert!(second.text.contains("@book{smith2020-1,\n\ttitle = {New}"), "{}", second.text);
        // A numbered key follows the new spelling too.
        assert_eq!(carried("smith2020-2", "Smith2020").as_deref(), Some("Smith2020-2"));
        assert_eq!(carried("doe2019", "Smith2020"), None);
    }

    #[test]
    fn a_key_changed_in_case_reaches_the_file() {
        let before = vec![("I1".to_string(), "@book{smith2020,\n}".to_string())];
        let first = build_bib("", &before, &BTreeMap::new());
        let after = vec![("I1".to_string(), "@book{Smith2020,\n}".to_string())];
        let second = build_bib("", &after, &first.keys);
        assert!(second.text.contains("@book{Smith2020,"), "{}", second.text);
    }

    #[test]
    fn trashed_collections_and_their_children_are_left_out() {
        let colls = r#"[
            {"key":"KEEP","data":{"name":"Thesis","parentCollection":false},"meta":{"numItems":1}},
            {"key":"BIN","data":{"name":"Verworfen","parentCollection":"KEEP","deleted":true},"meta":{"numItems":2}},
            {"key":"BINKID","data":{"name":"Darin","parentCollection":"BIN"},"meta":{"numItems":3}}
        ]"#;
        let (t, _) = Script::new(vec![("/collections?limit", 200, vec![("Total-Results", "3")], colls)]);
        let got = Client::with_transport(t, None).collections(LibraryRef::User(1)).unwrap();
        assert_eq!(got.iter().map(|c| c.key.as_str()).collect::<Vec<_>>(), vec!["KEEP"]);
    }

    #[test]
    fn result_parses_as_biblatex() {
        let items = vec![
            ("I1".to_string(), "@book{k,\n\ttitle = {A},\n\tdate = {2020},\n}".to_string()),
            ("I2".to_string(), "@book{k,\n\ttitle = {B},\n\tdate = {2021},\n}".to_string()),
        ];
        let built = build_bib("% Automatisch synchronisiert\n% Quelle: Meine Bibliothek › Thesis", &items, &BTreeMap::new());
        let bib = biblatex::Bibliography::parse(&built.text).expect("valid BibLaTeX");
        assert_eq!(bib.len(), 2);
        assert!(bib.get("k").is_some() && bib.get("k-1").is_some());
    }

    /// Talks to the real Zotero API (a public group library, no key needed):
    /// `cargo test -- --ignored live_`
    #[test]
    #[ignore]
    fn live_public_group_exports_valid_biblatex() {
        let mut c = Client::new(None);
        let target = Target { library: LibraryRef::Group(2205533), collection: None, subcollections: false };
        let Fetched::Changed { version, items } = c.export(&target, None).expect("export") else { panic!("expected items") };
        assert!(version.is_some());
        assert!(items.len() > 100, "{} items", items.len());
        let built = build_bib("% test", &items, &BTreeMap::new());
        biblatex::Bibliography::parse(&built.text).expect("valid BibLaTeX");
        assert_eq!(c.export(&target, version).expect("second export"), Fetched::NotModified);
        let colls = c.collections(LibraryRef::Group(2205533)).expect("collections");
        assert!(!colls.is_empty());
    }
}
