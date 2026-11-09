//! Autocompletion as in the Typst web app. Typst's own IDE engine (`typst-ide`) works out what
//! fits at the cursor, in a thread of its own so that typing never waits for a compile. This file
//! also ranks the suggestions against what has been typed since, and expands their snippets
//! (`heading(${title})`) into text with fields that Tab moves through. The popup is in
//! complete_ui.rs.

use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use eframe::egui;
use typst::World;
use typst::ecow::EcoString;
use typst::syntax::package::PackageSpec;
use typst::syntax::{FileId, RootedPath, VirtualPath, VirtualRoot};
use typst_layout::PagedDocument;

use crate::compiler::{EditorWorld, PackageList, Reply};

/// The last document that compiled, shared by the compiler: labels and citations come from it.
pub type DocSlot = Arc<Mutex<Option<Arc<PagedDocument>>>>;

/// The editor asks for suggestions at `cursor` in the text of the file `rel`.
pub struct Request {
    pub id: u64,
    pub root: PathBuf,
    pub main_rel: String,
    pub rel: String,
    pub text: String,
    /// Character index of the cursor in `text`.
    pub cursor: usize,
    /// Asked for with Ctrl+Space: also suggests where typing alone would not.
    pub explicit: bool,
    /// The project's files (relative paths), for suggestions like `image("…")`.
    pub files: Vec<String>,
}

pub struct Response {
    pub id: u64,
    /// Character index where the suggestions apply: the text from here to the cursor is replaced.
    pub from: usize,
    pub items: Vec<Item>,
}

/// What a suggestion is; decides its icon.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Kind {
    Syntax,
    Func,
    Type,
    Param,
    Constant,
    Path,
    Package,
    Label,
    Font,
    /// A symbol, with the character it stands for.
    Symbol(String),
}

#[derive(Clone, Debug)]
pub struct Item {
    pub kind: Kind,
    pub label: String,
    /// What is inserted (a snippet, see [`expand`]); the label itself when `None`.
    pub apply: Option<String>,
    /// One sentence about it.
    pub detail: Option<String>,
}

/// The completion thread: answers the newest request, drops the ones typing has overtaken.
pub(crate) fn worker(rx: Receiver<Request>, tx: Sender<Reply>, doc: DocSlot, ctx: egui::Context) {
    let mut world: Option<EditorWorld> = None;
    let mut read_at = Instant::now();
    let mut packages = Packages::default();
    while let Ok(mut req) = rx.recv() {
        while let Ok(newer) = rx.try_recv() {
            req = newer;
        }
        let Some(main) = file_id(&req.main_rel) else { continue };
        let Some(id) = file_id(&req.rel) else { continue };
        if world.as_ref().map(|w| w.root() != req.root).unwrap_or(true) {
            world = Some(EditorWorld::new(req.root.clone(), main));
            read_at = Instant::now();
        }
        let w = world.as_mut().unwrap();
        w.set_main(main);
        // Other files may have changed on disk (a saved tab, a pull): read them again now and then.
        if read_at.elapsed() > Duration::from_secs(2) {
            w.reset();
            read_at = Instant::now();
        }
        w.set_only_overlay(id, &req.text);
        w.set_files(req.files.iter().filter_map(|rel| file_id(rel)).collect());
        let cursor = byte_index(&req.text, req.cursor);
        if in_package_import(&req.text, cursor) {
            w.set_packages(packages.get());
        }
        let output = doc.lock().ok().and_then(|d| d.clone());
        // The IDE code is best effort; one odd spot in a document must not end suggestions for good.
        let found = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let source = w.source(id).ok()?;
            typst_ide::autocomplete(&*w, output.as_deref(), &source, cursor, req.explicit)
        }))
        .ok()
        .flatten();
        let (from, items) = match found {
            Some((from, list)) if req.text.is_char_boundary(from) && from <= cursor => {
                (req.text[..from].chars().count(), list.into_iter().map(item).collect())
            }
            _ => (req.cursor, Vec::new()),
        };
        let _ = tx.send(Reply::Completions(Response { id: req.id, from, items }));
        ctx.request_repaint();
    }
}

fn file_id(rel: &str) -> Option<FileId> {
    Some(RootedPath::new(VirtualRoot::Project, VirtualPath::new(rel).ok()?).intern())
}

pub fn byte_index(text: &str, chars: usize) -> usize {
    text.char_indices().nth(chars).map(|(b, _)| b).unwrap_or(text.len())
}

fn item(c: typst_ide::Completion) -> Item {
    use typst_ide::CompletionKind as K;
    let kind = match c.kind {
        K::Syntax => Kind::Syntax,
        K::Func => Kind::Func,
        K::Type => Kind::Type,
        K::Param => Kind::Param,
        K::Constant => Kind::Constant,
        K::Path => Kind::Path,
        K::Package => Kind::Package,
        K::Label => Kind::Label,
        K::Font => Kind::Font,
        K::Symbol(s) => Kind::Symbol(s.to_string()),
    };
    Item {
        kind,
        label: c.label.to_string(),
        apply: c.apply.map(|a| a.to_string()),
        detail: c.detail.map(|d| d.to_string()).filter(|d| !d.trim().is_empty()),
    }
}

// ---------------------------------------------------------------------------
// Packages for `#import "@…"`.

/// Is the cursor in the string of an import that starts with `@` (a package)?
fn in_package_import(text: &str, cursor: usize) -> bool {
    let before = &text[..cursor];
    let line = &before[before.rfind('\n').map(|i| i + 1).unwrap_or(0)..];
    line.contains("import") && line.contains("\"@")
}

/// Packages on this computer, and the official list from packages.typst.org (fetched the first
/// time a package name is completed, as Typst itself does when a package is imported).
#[derive(Default)]
struct Packages {
    list: Option<PackageList>,
    tried_at: Option<Instant>,
}

impl Packages {
    fn get(&mut self) -> PackageList {
        let retry = self.tried_at.map(|t| t.elapsed() > Duration::from_secs(600)).unwrap_or(true);
        if let Some(list) = &self.list
            && !retry
        {
            return list.clone();
        }
        let mut list = local_packages();
        self.tried_at = Some(Instant::now());
        match preview_index() {
            Ok(index) => {
                let local: Vec<PackageSpec> = list.iter().map(|(spec, _)| spec.clone()).collect();
                list.extend(index.into_iter().filter(|(spec, _)| !local.contains(spec)));
                // The full list does not need fetching again during this session.
                self.tried_at = Some(Instant::now() + Duration::from_secs(365 * 24 * 3600));
            }
            Err(e) => eprintln!("typisch: package list not available: {e}"),
        }
        let list = Arc::new(list);
        self.list = Some(list.clone());
        list
    }
}

/// Packages already downloaded (cache) or installed locally (data folder), as `@namespace/name:version`.
fn local_packages() -> Vec<(PackageSpec, Option<EcoString>)> {
    let mut out = Vec::new();
    for base in [dirs::data_dir(), dirs::cache_dir()].into_iter().flatten() {
        let base = base.join("typst").join("packages");
        let Ok(namespaces) = std::fs::read_dir(&base) else { continue };
        for ns in namespaces.flatten() {
            let Ok(names) = std::fs::read_dir(ns.path()) else { continue };
            for name in names.flatten() {
                let Ok(versions) = std::fs::read_dir(name.path()) else { continue };
                for version in versions.flatten() {
                    let spec = format!(
                        "@{}/{}:{}",
                        ns.file_name().to_string_lossy(),
                        name.file_name().to_string_lossy(),
                        version.file_name().to_string_lossy()
                    );
                    if let Ok(spec) = spec.parse::<PackageSpec>()
                        && !out.iter().any(|(s, _)| *s == spec)
                    {
                        out.push((spec, None));
                    }
                }
            }
        }
    }
    out
}

fn preview_index() -> Result<Vec<(PackageSpec, Option<EcoString>)>, String> {
    let mut builder = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(10))
        .timeout_read(Duration::from_secs(30))
        .user_agent(&format!("Typisch/{}", env!("CARGO_PKG_VERSION")))
        .https_only(true);
    if let Ok(tls) = native_tls::TlsConnector::new() {
        builder = builder.tls_connector(Arc::new(tls));
    }
    let entries: Vec<serde_json::Value> = builder
        .build()
        .get("https://packages.typst.org/preview/index.json")
        .call()
        .map_err(|e| e.to_string())?
        .into_json()
        .map_err(|e| e.to_string())?;
    Ok(entries
        .iter()
        .filter_map(|e| {
            let spec = format!("@preview/{}:{}", e.get("name")?.as_str()?, e.get("version")?.as_str()?);
            let detail = e.get("description").and_then(|d| d.as_str()).map(EcoString::from);
            Some((spec.parse().ok()?, detail))
        })
        .collect())
}

// ---------------------------------------------------------------------------
// Ranking.

/// How well a suggestion fits what was typed since the suggestions started (lower is better),
/// with the label's matching characters for highlighting; `None` when it does not fit at all.
pub fn rank(typed: &str, label: &str) -> Option<(u8, Vec<usize>)> {
    if typed.is_empty() {
        return Some((0, Vec::new()));
    }
    let q: Vec<char> = typed.chars().collect();
    let l: Vec<char> = label.chars().collect();
    let run = |start: usize| (start..start + q.len()).collect::<Vec<_>>();
    if l == q {
        return Some((0, run(0)));
    }
    if l.starts_with(&q) {
        return Some((1, run(0)));
    }
    let lower = |c: &char| c.to_lowercase().next().unwrap_or(*c);
    let ql: Vec<char> = q.iter().map(lower).collect();
    let ll: Vec<char> = l.iter().map(lower).collect();
    if ll.starts_with(&ql) {
        return Some((2, run(0)));
    }
    // A word inside the label: "break" in "page-break", "r" in "arrow.r".
    let word_start = |i: usize| i == 0 || !l[i - 1].is_alphanumeric() || (l[i - 1].is_lowercase() && l[i].is_uppercase());
    if let Some(i) = (1..ll.len()).find(|&i| word_start(i) && ll[i..].starts_with(&ql)) {
        return Some((3, run(i)));
    }
    if ql.len() > 1
        && let Some(i) = (1..ll.len()).find(|&i| ll[i..].starts_with(&ql))
    {
        return Some((4, run(i)));
    }
    // The typed letters in order, starting at the start of a word: "lb" for "linebreak".
    if ql.len() > 1 && ll[0] == ql[0] {
        let mut at = Vec::with_capacity(ql.len());
        let mut i = 0;
        for c in &ql {
            while i < ll.len() && ll[i] != *c {
                i += 1;
            }
            if i == ll.len() {
                return None;
            }
            at.push(i);
            i += 1;
        }
        return Some((5, at));
    }
    None
}

/// The suggestions that fit `typed`, best first, each with its highlighted characters.
pub fn filter(items: &[Item], typed: &str) -> Vec<(usize, Vec<usize>)> {
    let mut found: Vec<(u8, usize, usize, Vec<usize>)> = items
        .iter()
        .enumerate()
        .filter_map(|(i, item)| rank(typed, &item.label).map(|(tier, at)| (tier, item.label.chars().count(), i, at)))
        .collect();
    // Nothing typed yet: Typst's own order (the document's own definitions first).
    if !typed.is_empty() {
        found.sort_by_key(|(tier, len, i, _)| (*tier, *len, *i));
    }
    found.into_iter().map(|(_, _, i, at)| (i, at)).collect()
}

// ---------------------------------------------------------------------------
// Snippets.

/// A suggestion's text, ready to insert.
#[derive(Debug, PartialEq)]
pub struct Expansion {
    pub text: String,
    /// The fields in the order Tab visits them; each lists the character ranges (in `text`) it
    /// covers (one, unless the snippet repeats a field).
    pub fields: Vec<Vec<(usize, usize)>>,
}

/// Expands a snippet with the same rules as the Typst web app's editor (CodeMirror): `${}`,
/// `${name}`, `${2}` and `${2:name}` are fields (`#{…}` too), numbered ones are visited first,
/// in order of their number. Later lines get `indent`, leading tabs become indentation.
pub fn expand(snippet: &str, indent: &str) -> Expansion {
    struct Field {
        seq: Option<u32>,
        name: String,
    }
    let mut fields: Vec<Field> = Vec::new();
    // (field, start, end) in characters of the output.
    let mut spots: Vec<(usize, usize, usize)> = Vec::new();
    let mut text = String::new();
    let mut len = 0;
    for (n, line) in snippet.split('\n').enumerate() {
        let mut line = line;
        if n > 0 {
            text.push('\n');
            text.push_str(indent);
            len += 1 + indent.chars().count();
            let tabs = line.len() - line.trim_start_matches('\t').len();
            text.push_str(&"  ".repeat(tabs));
            len += 2 * tabs;
            line = &line[tabs..];
        }
        let chars: Vec<char> = line.chars().collect();
        let mut i = 0;
        while i < chars.len() {
            if matches!(chars[i], '$' | '#')
                && chars.get(i + 1) == Some(&'{')
                && let Some((raw, end)) = field_body(&chars, i + 2)
            {
                let (seq, name) = match raw.split_once(':') {
                    Some((num, rest)) if !num.is_empty() && num.chars().all(|c| c.is_ascii_digit()) => {
                        (num.parse().ok(), rest.to_string())
                    }
                    _ if !raw.is_empty() && raw.chars().all(|c| c.is_ascii_digit()) => (raw.parse().ok(), String::new()),
                    _ => (None, raw.replace("\\{", "{").replace("\\}", "}")),
                };
                let same = fields.iter().position(|f| match seq {
                    Some(_) => f.seq == seq,
                    None => !name.is_empty() && f.name == name,
                });
                let index = match same {
                    Some(index) => index,
                    None => {
                        let mut at = 0;
                        while at < fields.len() && seq.is_some() && fields[at].seq.is_some() && fields[at].seq < seq {
                            at += 1;
                        }
                        if seq.is_none() {
                            at = fields.len();
                        }
                        for spot in &mut spots {
                            if spot.0 >= at {
                                spot.0 += 1;
                            }
                        }
                        fields.insert(at, Field { seq, name: name.clone() });
                        at
                    }
                };
                let count = name.chars().count();
                spots.push((index, len, len + count));
                text.push_str(&name);
                len += count;
                i = end;
            } else {
                text.push(chars[i]);
                len += 1;
                i += 1;
            }
        }
    }
    let mut ranges = vec![Vec::new(); fields.len()];
    for (field, start, end) in spots {
        ranges[field].push((start, end));
    }
    Expansion { text, fields: ranges }
}

/// The inside of a field starting at `from` (just after `{`) and the index after its `}`.
fn field_body(chars: &[char], from: usize) -> Option<(String, usize)> {
    let mut i = from;
    let mut raw = String::new();
    while i < chars.len() {
        match chars[i] {
            '}' => return Some((raw, i + 1)),
            '\\' if matches!(chars.get(i + 1), Some('{' | '}')) => {
                raw.push('\\');
                raw.push(chars[i + 1]);
                i += 2;
            }
            c => {
                raw.push(c);
                i += 1;
            }
        }
    }
    None
}

/// Where an edit turned `old` into `new`, in characters: the start, the end of what was removed
/// and the end of what was inserted. `at_most` is where the edit cannot have started after (the
/// cursor), which keeps typing a letter next to the same letter from being misplaced.
pub fn changed_span(old: &str, new: &str, at_most: usize) -> (usize, usize, usize) {
    let o: Vec<char> = old.chars().collect();
    let n: Vec<char> = new.chars().collect();
    let mut start = o.iter().zip(&n).take_while(|(a, b)| a == b).count().min(at_most);
    start = start.min(o.len()).min(n.len());
    let room = o.len().min(n.len()) - start;
    let tail = o.iter().rev().zip(n.iter().rev()).take_while(|(a, b)| a == b).count().min(room);
    (start, o.len() - tail, n.len() - tail)
}

/// Moves a field through an edit (from `changed_span`). Text typed at either edge of a field
/// becomes part of it.
pub fn map_field((from, to): (usize, usize), (start, old_end, new_end): (usize, usize, usize)) -> (usize, usize) {
    let shift = |x: usize| x - old_end + new_end;
    let from = if from <= start { from } else if from >= old_end { shift(from) } else { start };
    let to = if to < start { to } else if to >= old_end { shift(to) } else { new_end };
    (from, to.max(from))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn exp(s: &str) -> (String, Vec<Vec<(usize, usize)>>) {
        let e = expand(s, "  ");
        (e.text, e.fields)
    }

    #[test]
    fn snippets_expand_like_the_web_app() {
        assert_eq!(exp("heading[${}]"), ("heading[]".into(), vec![vec![(8, 8)]]));
        assert_eq!(exp("let ${name} = ${value}"), ("let name = value".into(), vec![vec![(4, 8)], vec![(11, 16)]]));
        // Numbered fields come first.
        assert_eq!(exp("${x}_${2:2}"), ("x_2".into(), vec![vec![(2, 3)], vec![(0, 1)]]));
        // "$" before a field is text; ranges count characters, not bytes.
        assert_eq!(exp("$${x}$"), ("$x$".into(), vec![vec![(1, 2)]]));
        assert_eq!(exp("ä(${})"), ("ä()".into(), vec![vec![(2, 2)]]));
        // Fields are not nested; everything up to the first `}` is the default text.
        assert_eq!(exp("(${a: 1, b: 2})"), ("(a: 1, b: 2)".into(), vec![vec![(1, 11)]]));
        assert_eq!(exp("plain"), ("plain".into(), vec![]));
        // Same name: one field in two places.
        assert_eq!(exp("${a} ${a}"), ("a a".into(), vec![vec![(0, 1), (2, 3)]]));
    }

    #[test]
    fn later_lines_are_indented() {
        let e = expand("if ${1 < 2} {\n\t${}\n}", "    ");
        assert_eq!(e.text, "if 1 < 2 {\n      \n    }");
        assert_eq!(e.fields, vec![vec![(3, 8)], vec![(17, 17)]]);
    }

    #[test]
    fn ranking_prefers_what_was_typed() {
        let items: Vec<Item> = ["terms", "text", "table", "context", "text-size", "Text", "pagebreak", "linebreak"]
            .iter()
            .map(|l| Item { kind: Kind::Func, label: l.to_string(), apply: None, detail: None })
            .collect();
        let labels = |typed: &str| filter(&items, typed).into_iter().map(|(i, _)| items[i].label.as_str()).collect::<Vec<_>>();
        assert_eq!(labels("te"), ["text", "terms", "text-size", "Text", "context", "table"]);
        assert_eq!(labels("text"), ["text", "text-size", "Text", "context"]);
        assert_eq!(labels("break"), ["pagebreak", "linebreak"]);
        assert_eq!(labels("lb"), ["linebreak"]);
        assert_eq!(labels("").len(), items.len());
        assert_eq!(rank("size", "text-size"), Some((3, vec![5, 6, 7, 8])));
    }

    #[test]
    fn fields_follow_edits() {
        // "let name = value", typing "x" at the end of the first field (after "name").
        let span = changed_span("let name = value", "let namex = value", 8);
        assert_eq!(span, (8, 8, 9));
        assert_eq!(map_field((4, 8), span), (4, 9));
        assert_eq!(map_field((11, 16), span), (12, 17));
        // Replacing the selected default text.
        let span = changed_span("let name = value", "let a = value", 4);
        assert_eq!(map_field((4, 8), span), (4, 5));
        assert_eq!(map_field((11, 16), span), (8, 13));
        // Typing the same letter that follows: the cursor says where it went.
        assert_eq!(changed_span("aa", "aaa", 1), (1, 1, 2));
    }
}
