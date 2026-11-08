//! Typst compilation in a background thread: a `World` that serves the files
//! being edited from memory and everything else from disk / package cache.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::OnceLock;

use typst::diag::{FileError, FileResult, Severity};
use typst::foundations::{Bytes, Datetime, Duration, Selector};
use typst::layout::{Frame, FrameItem};
use typst::model::Destination;
use typst::syntax::{FileId, RootedPath, Source, VirtualPath, VirtualRoot};
use typst::text::{Font, FontBook};
use typst::utils::LazyHash;
use typst::{Library, LibraryExt, World, WorldExt};
use typst_kit::datetime::Time;
use typst_kit::downloader::SystemDownloader;
use typst_kit::files::{FileStore, FsRoot, SystemFiles};
use typst_kit::fonts::FontStore;
use typst_kit::packages::SystemPackages;
use typst_layout::{Page, PagedDocument, PagedIntrospector};
use eframe::egui;

/// Fonts are expensive to discover, so do it once for the whole program.
pub fn fonts() -> &'static FontStore {
    static FONTS: OnceLock<FontStore> = OnceLock::new();
    FONTS.get_or_init(|| {
        let mut store = FontStore::new();
        store.extend(typst_kit::fonts::embedded());
        store.extend(typst_kit::fonts::system());
        store
    })
}

fn library() -> &'static LazyHash<Library> {
    static LIB: OnceLock<LazyHash<Library>> = OnceLock::new();
    LIB.get_or_init(|| LazyHash::new(Library::default()))
}

pub struct EditorWorld {
    root: PathBuf,
    main: FileId,
    store: FileStore<SystemFiles>,
    overlays: HashMap<FileId, Source>,
    time: Time,
}

pub(crate) fn virtual_id(rel: &str) -> FileId {
    let vpath = VirtualPath::new(rel).unwrap_or_else(|_| VirtualPath::new("main.typ").unwrap());
    RootedPath::new(VirtualRoot::Project, vpath).intern()
}

impl EditorWorld {
    pub fn new(root: PathBuf, main: FileId) -> Self {
        let packages = SystemPackages::new(SystemDownloader::new("typisch/1.0"));
        let files = SystemFiles::new(FsRoot::new(root.clone()), packages);
        Self { root, main, store: FileStore::new(files), overlays: HashMap::new(), time: Time::system() }
    }

    pub fn set_main(&mut self, main: FileId) {
        self.main = main;
    }

    pub fn set_overlay(&mut self, id: FileId, text: &str) {
        match self.overlays.get_mut(&id) {
            Some(src) => {
                if src.text() != text {
                    src.replace(text);
                }
            }
            None => {
                self.overlays.insert(id, Source::new(id, text.to_string()));
            }
        }
    }

    pub fn reset(&mut self) {
        self.store.reset();
        self.time.reset();
    }

    fn rel_path(&self, id: FileId) -> String {
        let rp = id.get();
        match rp.root() {
            VirtualRoot::Project => rp.vpath().get_without_slash().to_string(),
            VirtualRoot::Package(spec) => format!("{spec}/{}", rp.vpath().get_without_slash()),
        }
    }
}

impl World for EditorWorld {
    fn library(&self) -> &LazyHash<Library> {
        library()
    }
    fn book(&self) -> &LazyHash<FontBook> {
        fonts().book()
    }
    fn main(&self) -> FileId {
        self.main
    }
    fn source(&self, id: FileId) -> FileResult<Source> {
        if let Some(src) = self.overlays.get(&id) {
            return Ok(src.clone());
        }
        self.store.source(id)
    }
    fn file(&self, id: FileId) -> FileResult<Bytes> {
        if let Some(src) = self.overlays.get(&id) {
            return Ok(Bytes::from_string(src.text().to_string()));
        }
        self.store.file(id)
    }
    fn font(&self, index: usize) -> Option<Font> {
        fonts().font(index)
    }
    fn today(&self, offset: Option<Duration>) -> Option<Datetime> {
        self.time.today(offset)
    }
}

#[derive(Clone, Debug)]
pub struct Diag {
    pub error: bool,
    pub message: String,
    /// Path relative to the project root (or package path).
    pub file: String,
    pub in_package: bool,
    /// 0-based line and column, when known.
    pub line: Option<usize>,
    pub col: Option<usize>,
    pub hints: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct OutlineItem {
    pub level: usize,
    pub title: String,
    pub page: usize,
    pub y_pt: f32,
}

/// Where a link in the preview leads to.
#[derive(Clone, Debug)]
pub enum LinkTarget {
    /// A place in this document: 1-based page and y position in points.
    Page { page: usize, y_pt: f32 },
    /// An external URL, opened in the browser.
    Url(String),
}

/// A clickable link area on a page, in points from the page's top-left corner.
#[derive(Clone, Debug)]
pub struct PageLink {
    pub rect_pt: egui::Rect,
    pub target: LinkTarget,
}

/// Size, content fingerprint and links of one laid-out page (pixels are rendered on demand).
#[derive(Clone, Debug)]
pub struct PageInfo {
    pub size_pt: egui::Vec2,
    pub hash: u128,
    pub links: Vec<PageLink>,
}

pub enum Job {
    Compile {
        tag: u64,
        root: PathBuf,
        main_rel: String,
        overlays: Vec<(String, String)>,
    },
    /// Render these pages of the last compiled document: (index, hash, pixels per pt).
    Render {
        requests: Vec<(usize, u128, f32)>,
    },
    ExportPdf {
        path: PathBuf,
    },
    /// Render the first page of a standalone source (template thumbnails).
    Thumbnail {
        key: String,
        source: String,
    },
    FetchPackage {
        spec: String,
    },
}

pub enum Reply {
    Compiled {
        tag: u64,
        pages: Option<Vec<PageInfo>>,
        diags: Vec<Diag>,
        outline: Vec<OutlineItem>,
        words: usize,
        millis: u128,
    },
    Rendered {
        index: usize,
        hash: u128,
        scale: f32,
        image: egui::ColorImage,
    },
    Exported(Result<PathBuf, String>),
    Thumbnail {
        key: String,
        image: Option<egui::ColorImage>,
    },
    Package(Result<String, String>),
}

pub struct Compiler {
    tx: Sender<Job>,
    pub rx: Receiver<Reply>,
}

impl Compiler {
    pub fn spawn(ctx: egui::Context) -> Self {
        let (tx, job_rx) = channel::<Job>();
        let (reply_tx, rx) = channel::<Reply>();
        std::thread::Builder::new()
            .name("typst".into())
            .stack_size(64 * 1024 * 1024)
            .spawn(move || worker(job_rx, reply_tx, ctx))
            .expect("spawn compiler thread");
        Self { tx, rx }
    }

    pub fn send(&self, job: Job) {
        let _ = self.tx.send(job);
    }
}

fn worker(rx: Receiver<Job>, tx: Sender<Reply>, ctx: egui::Context) {
    let mut world: Option<EditorWorld> = None;
    let mut last_doc: Option<PagedDocument> = None;
    let mut hashes: Vec<u128> = Vec::new();
    let mut queue: std::collections::VecDeque<Job> = std::collections::VecDeque::new();
    // Warm up the font list straight away.
    let _ = fonts();

    loop {
        if queue.is_empty() {
            match rx.recv() {
                Ok(j) => queue.push_back(j),
                Err(_) => break,
            }
        }
        while let Ok(j) = rx.try_recv() {
            queue.push_back(j);
        }
        // Only the newest compile matters; render requests made before it are stale.
        let job = match queue.iter().rposition(|j| matches!(j, Job::Compile { .. })) {
            Some(pos) => {
                let mut kept = std::collections::VecDeque::new();
                let mut newest = None;
                for (i, j) in std::mem::take(&mut queue).into_iter().enumerate() {
                    match j {
                        Job::Compile { .. } if i == pos => newest = Some(j),
                        Job::Compile { .. } => {}
                        Job::Render { .. } if i < pos => {}
                        other => kept.push_back(other),
                    }
                }
                queue = kept;
                newest.unwrap()
            }
            None => queue.pop_front().unwrap(),
        };
        match job {
            Job::Render { requests } => {
                for (index, hash, scale) in requests {
                    // Stop early when a newer compile arrived; the UI asks again afterwards.
                    while let Ok(j) = rx.try_recv() {
                        queue.push_back(j);
                    }
                    if queue.iter().any(|j| matches!(j, Job::Compile { .. })) {
                        break;
                    }
                    let (Some(doc), Some(h)) = (last_doc.as_ref(), hashes.get(index)) else { continue };
                    if *h != hash {
                        continue;
                    }
                    let opts = typst_render::RenderOptions {
                        pixel_per_pt: typst::utils::Scalar::new(scale as f64),
                        render_bleed: false,
                    };
                    let image = pixmap_to_image(&typst_render::render(&doc.pages()[index], &opts));
                    let _ = tx.send(Reply::Rendered { index, hash, scale, image });
                    ctx.request_repaint();
                }
            }
            other => handle(&mut world, &mut last_doc, &mut hashes, other, &tx),
        }
        ctx.request_repaint();
    }
}

fn handle(
    world: &mut Option<EditorWorld>,
    last_doc: &mut Option<PagedDocument>,
    hashes: &mut Vec<u128>,
    job: Job,
    tx: &Sender<Reply>,
) {
    match job {
        Job::Render { .. } => {}
        Job::Compile { tag, root, main_rel, overlays } => {
            let start = std::time::Instant::now();
            let main = virtual_id(&main_rel);
            if world.as_ref().map(|w| w.root != root).unwrap_or(true) {
                *world = Some(EditorWorld::new(root.clone(), main));
                *last_doc = None;
                hashes.clear();
            }
            let w = world.as_mut().unwrap();
            w.set_main(main);
            w.reset();
            // Only this job's texts come from memory. A tab that is no longer active was saved
            // (or, for a synced bibliography, rewritten by the sync), so it is read from disk.
            let current: std::collections::HashSet<FileId> = overlays.iter().map(|(rel, _)| virtual_id(rel)).collect();
            w.overlays.retain(|id, _| current.contains(id));
            for (rel, text) in &overlays {
                w.set_overlay(virtual_id(rel), text);
            }
            let result = typst::compile::<PagedDocument>(&*w);
            let mut diags: Vec<Diag> = Vec::new();
            let to_diag = |d: &typst::diag::SourceDiagnostic, w: &EditorWorld| {
                let id = d.span.id();
                let (mut line, mut col, mut file, mut in_package) = (None, None, String::new(), false);
                if let Some(id) = id {
                    file = w.rel_path(id);
                    in_package = matches!(id.get().root(), VirtualRoot::Package(_));
                    if let (Some(range), Ok(src)) = (w.range(d.span), w.source(id))
                        && let Some((l, c)) = src.lines().byte_to_line_column(range.start) {
                        line = Some(l);
                        col = Some(c);
                    }
                }
                Diag {
                    error: d.severity == Severity::Error,
                    message: d.message.to_string(),
                    file,
                    in_package,
                    line,
                    col,
                    hints: d.hints.iter().map(|h| h.v.to_string()).collect(),
                }
            };
            for warn in result.warnings.iter() {
                diags.push(to_diag(warn, w));
            }
            let (pages, outline, words) = match result.output {
                Ok(doc) => {
                    // Only sizes, fingerprints and link areas here; pixels are rendered for
                    // visible pages on request.
                    let intro = doc.introspector();
                    let pages: Vec<PageInfo> = doc
                        .pages()
                        .iter()
                        .map(|page| {
                            let size = page.frame.size();
                            PageInfo {
                                size_pt: egui::vec2(size.x.to_pt() as f32, size.y.to_pt() as f32),
                                hash: typst::utils::hash128(page),
                                links: links_of(page, intro),
                            }
                        })
                        .collect();
                    *hashes = pages.iter().map(|p| p.hash).collect();
                    let outline = outline_of(&doc);
                    let mut words = 0;
                    for page in doc.pages() {
                        words += count_words(&page.frame);
                    }
                    *last_doc = Some(doc);
                    (Some(pages), outline, words)
                }
                Err(errors) => {
                    for e in errors.iter() {
                        diags.insert(0, to_diag(e, w));
                    }
                    (None, Vec::new(), 0)
                }
            };
            let _ = tx.send(Reply::Compiled {
                tag,
                pages,
                diags,
                outline,
                words,
                millis: start.elapsed().as_millis(),
            });
        }
        Job::ExportPdf { path } => {
            let res = match last_doc.as_ref() {
                None => Err("Das Dokument enthält Fehler und kann nicht exportiert werden.".to_string()),
                Some(doc) => match typst_pdf::pdf(doc, &typst_pdf::PdfOptions::default()) {
                    Ok(bytes) => std::fs::write(&path, bytes).map(|_| path.clone()).map_err(|e| e.to_string()),
                    Err(errs) => Err(errs.iter().map(|e| e.message.to_string()).collect::<Vec<_>>().join("\n")),
                },
            };
            let _ = tx.send(Reply::Exported(res));
        }
        Job::Thumbnail { key, source } => {
            let dir = std::env::temp_dir().join("typisch-thumbs");
            let _ = std::fs::create_dir_all(&dir);
            let main = virtual_id("main.typ");
            let mut tw = EditorWorld::new(dir, main);
            tw.set_overlay(main, &source);
            let image = typst::compile::<PagedDocument>(&tw).output.ok().and_then(|doc| {
                doc.pages().first().map(|p| {
                    let opts = typst_render::RenderOptions {
                        pixel_per_pt: typst::utils::Scalar::new(0.9),
                        render_bleed: false,
                    };
                    pixmap_to_image(&typst_render::render(p, &opts))
                })
            });
            let _ = tx.send(Reply::Thumbnail { key, image });
        }
        Job::FetchPackage { spec } => {
            let res = spec
                .trim()
                .parse::<typst::syntax::package::PackageSpec>()
                .map_err(|e| e.to_string())
                .and_then(|s| {
                    let packages = SystemPackages::new(SystemDownloader::new("typisch/1.0"));
                    packages.obtain(&s).map(|_| s.to_string()).map_err(|e| e.to_string())
                });
            let _ = tx.send(Reply::Package(res));
        }
    }
}

fn pixmap_to_image(pix: &tiny_skia::Pixmap) -> egui::ColorImage {
    let size = [pix.width() as usize, pix.height() as usize];
    // tiny-skia stores premultiplied RGBA.
    egui::ColorImage::from_rgba_premultiplied(size, pix.data())
}

fn count_words(frame: &Frame) -> usize {
    let mut n = 0;
    let mut in_word = false;
    let mut text = String::new();
    collect_text(frame, &mut text);
    for c in text.chars() {
        if c.is_alphanumeric() {
            if !in_word {
                n += 1;
                in_word = true;
            }
        } else if c != '\'' && c != '-' {
            in_word = false;
        }
    }
    n
}

fn collect_text(frame: &Frame, out: &mut String) {
    for (_, item) in frame.items() {
        match item {
            FrameItem::Group(g) => collect_text(&g.frame, out),
            FrameItem::Text(t) => {
                out.push_str(&t.text);
                out.push(' ');
            }
            _ => {}
        }
    }
}

/// Maps coordinates inside a frame to points on the page.
#[derive(Clone, Copy)]
struct Affine {
    sx: f32,
    kx: f32,
    tx: f32,
    ky: f32,
    sy: f32,
    ty: f32,
}

impl Affine {
    const IDENTITY: Self = Self { sx: 1.0, kx: 0.0, tx: 0.0, ky: 0.0, sy: 1.0, ty: 0.0 };

    fn translate(x: f32, y: f32) -> Self {
        Self { tx: x, ty: y, ..Self::IDENTITY }
    }

    /// The transform that applies `self` after `prev`.
    fn pre_concat(self, prev: Self) -> Self {
        Self {
            sx: self.sx * prev.sx + self.kx * prev.ky,
            ky: self.ky * prev.sx + self.sy * prev.ky,
            kx: self.sx * prev.kx + self.kx * prev.sy,
            sy: self.ky * prev.kx + self.sy * prev.sy,
            tx: self.sx * prev.tx + self.kx * prev.ty + self.tx,
            ty: self.ky * prev.tx + self.sy * prev.ty + self.ty,
        }
    }

    fn apply(self, x: f32, y: f32) -> egui::Pos2 {
        egui::pos2(self.sx * x + self.kx * y + self.tx, self.ky * x + self.sy * y + self.ty)
    }

    fn from_transform(t: &typst::layout::Transform) -> Self {
        Self {
            sx: t.sx.get() as f32,
            kx: t.kx.get() as f32,
            tx: t.tx.to_pt() as f32,
            ky: t.ky.get() as f32,
            sy: t.sy.get() as f32,
            ty: t.ty.to_pt() as f32,
        }
    }
}

fn links_of(page: &Page, intro: &PagedIntrospector) -> Vec<PageLink> {
    let mut links = Vec::new();
    collect_links(&page.frame, Affine::IDENTITY, intro, &mut links);
    links
}

fn collect_links(frame: &Frame, ts: Affine, intro: &PagedIntrospector, out: &mut Vec<PageLink>) {
    for (pos, item) in frame.items() {
        match item {
            FrameItem::Link(dest, size) => {
                let target = match dest {
                    Destination::Url(url) => LinkTarget::Url(url.as_str().to_string()),
                    Destination::Position(p) => LinkTarget::Page {
                        page: p.page.get(),
                        y_pt: p.point.y.to_pt() as f32,
                    },
                    Destination::Location(loc) => match intro.position(*loc) {
                        Some(p) => LinkTarget::Page {
                            page: p.page.get(),
                            y_pt: p.point.y.to_pt() as f32,
                        },
                        None => LinkTarget::Page { page: 1, y_pt: 0.0 },
                    },
                };
                let ts = ts.pre_concat(Affine::translate(pos.x.to_pt() as f32, pos.y.to_pt() as f32));
                let w = size.x.to_pt() as f32;
                let h = size.y.to_pt() as f32;
                let corners = [ts.apply(0.0, 0.0), ts.apply(w, 0.0), ts.apply(0.0, h), ts.apply(w, h)];
                out.push(PageLink { rect_pt: egui::Rect::from_points(&corners), target });
            }
            FrameItem::Group(group) => {
                let ts = ts
                    .pre_concat(Affine::translate(pos.x.to_pt() as f32, pos.y.to_pt() as f32))
                    .pre_concat(Affine::from_transform(&group.transform));
                collect_links(&group.frame, ts, intro, out);
            }
            _ => {}
        }
    }
}

fn outline_of(doc: &PagedDocument) -> Vec<OutlineItem> {
    use typst::introspection::Introspector;
    use typst::model::HeadingElem;
    let intro = doc.introspector();
    let sel = Selector::Elem(typst::foundations::Element::of::<HeadingElem>(), None);
    intro
        .query(&sel)
        .iter()
        .map(|c| {
            let level = match c.get_by_name("level") {
                Ok(typst::foundations::Value::Int(i)) => i.max(1) as usize,
                _ => 1,
            };
            let title = c
                .get_by_name("body")
                .ok()
                .and_then(|v| match v {
                    typst::foundations::Value::Content(b) => Some(b.plain_text().to_string()),
                    _ => None,
                })
                .unwrap_or_else(|| c.plain_text().to_string());
            let (page, y_pt) = c
                .location()
                .and_then(|loc| intro.position(loc))
                .map(|pos| (pos.page.get(), pos.point.y.to_pt() as f32))
                .unwrap_or((1, 0.0));
            OutlineItem { level, title, page, y_pt }
        })
        .collect()
}

#[allow(dead_code)]
pub fn file_error_text(e: &FileError) -> String {
    e.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn links_are_collected() {
        let dir = std::env::temp_dir().join(format!("typisch-link-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let source = "\
#outline(title: none)

= Kapitel Eins

Text

#pagebreak()

= Kapitel Zwei

Text und #link(\"https://typst.app\")[ein Link].
";
        std::fs::write(dir.join("main.typ"), source).unwrap();

        let main = virtual_id("main.typ");
        let mut world = EditorWorld::new(dir.clone(), main);
        world.set_overlay(main, source);
        let doc = typst::compile::<PagedDocument>(&world).output.unwrap();
        let intro = doc.introspector();
        let links: Vec<_> = doc.pages().iter().flat_map(|p| links_of(p, intro)).collect();

        assert!(links.iter().any(|l| matches!(l.target, LinkTarget::Page { page, .. } if page > 1)));
        assert!(links.iter().any(|l| matches!(&l.target, LinkTarget::Url(u) if u == "https://typst.app")));
        assert!(links.iter().all(|l| l.rect_pt.is_positive()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Guards dependency updates: a new Typst version must still compile every built-in template.
    #[test]
    fn every_template_compiles() {
        let mut failures = Vec::new();
        for (i, template) in crate::templates::TEMPLATES.iter().enumerate() {
            let dir = std::env::temp_dir().join(format!("typisch-template-test-{}-{i}", std::process::id()));
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("main.typ"), template.source).unwrap();
            if template.source.contains("beispiel-unterschrift.svg") {
                std::fs::write(dir.join("beispiel-unterschrift.svg"), crate::templates::SIGNATURE_SVG).unwrap();
            }

            let main = virtual_id("main.typ");
            let mut world = EditorWorld::new(dir.clone(), main);
            world.set_overlay(main, template.source);
            match typst::compile::<PagedDocument>(&world).output {
                Ok(doc) if doc.pages().is_empty() => failures.push(format!("{}: no pages", template.name)),
                Ok(_) => {}
                Err(errors) => {
                    let messages: Vec<_> = errors.iter().map(|e| e.message.to_string()).collect();
                    failures.push(format!("{}: {}", template.name, messages.join("; ")));
                }
            }
            let _ = std::fs::remove_dir_all(&dir);
        }
        assert!(failures.is_empty(), "templates that do not compile:\n{}", failures.join("\n"));
    }
}

