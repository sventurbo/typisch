//! Writing files so that nothing is lost: a file is replaced in one step, never left half
//! written, and an open document only counts as saved once its text is on disk. No window
//! code here, so the tests call it directly.

use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::app::OpenDoc;

// ---------------------------------------------------------------------------
// Replacing a file in one step.

/// Replaces `path` with `bytes` through a hidden temporary file in the same folder: a crash, a
/// full disk or a power cut leaves either the old file or the new one, never part of one. The
/// temporary file is created fresh (never through something already lying there, such as a
/// planted symlink), and the rename replaces the target's directory entry, not what it may
/// point to. An existing file keeps its permissions (and, on macOS, its Finder tags and other
/// extended attributes).
pub fn atomic_write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let tmp = temp_path(path);
    let file = create_fresh(&tmp)?;
    finish(&tmp, file, path, bytes)
}

/// Saves a file the user works on: like [`atomic_write`], but it behaves as writing into the
/// file would. A symlink stays a symlink (the file it points to is replaced), a file that is
/// protected from writing is not replaced behind that protection (a rename would not care), and
/// in a folder that allows no new files the existing file is written in place instead, as there
/// is no other way to save it there.
pub fn replace_file(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let target = if path.symlink_metadata().is_ok_and(|m| m.file_type().is_symlink()) {
        path.canonicalize()?
    } else {
        path.to_path_buf()
    };
    match std::fs::OpenOptions::new().write(true).open(&target) {
        Ok(_) => {}
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }
    let tmp = temp_path(&target);
    let file = match create_fresh(&tmp) {
        Ok(file) => file,
        Err(e) if e.kind() == io::ErrorKind::PermissionDenied && target.is_file() => return std::fs::write(&target, bytes),
        Err(e) => return Err(e),
    };
    finish(&tmp, file, &target, bytes)
}

/// The temporary file's place next to `path`. Its name starts with a dot (the file list, and
/// most other programs, leave such files alone) and is short, whatever the target is called.
fn temp_path(path: &Path) -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    path.with_file_name(format!(".typisch-{}-{n}.tmp", std::process::id()))
}

/// A new, empty file at `tmp`, whatever lies there already (a leftover of an earlier run that
/// crashed with the same process number, or something planted) is removed, never written through.
fn create_fresh(tmp: &Path) -> io::Result<std::fs::File> {
    match std::fs::remove_file(tmp) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e),
        _ => {}
    }
    std::fs::OpenOptions::new().write(true).create_new(true).open(tmp)
}

fn finish(tmp: &Path, file: std::fs::File, path: &Path, bytes: &[u8]) -> io::Result<()> {
    let result = write_and_rename(tmp, file, path, bytes);
    if result.is_err() {
        let _ = std::fs::remove_file(tmp);
    }
    result
}

fn write_and_rename(tmp: &Path, mut file: std::fs::File, path: &Path, bytes: &[u8]) -> io::Result<()> {
    file.write_all(bytes)?;
    // What the new file takes over from the one it replaces (a link is replaced as a link).
    if let Ok(old) = path.symlink_metadata()
        && old.is_file()
    {
        copy_attributes(path, tmp);
        file.set_permissions(old.permissions())?;
    }
    // On disk before it takes the old file's place.
    file.sync_all()?;
    drop(file);
    std::fs::rename(tmp, path)?;
    // The rename itself is only safe once the folder is on disk too; not every system allows
    // opening a folder for that, and the file is complete either way.
    #[cfg(unix)]
    if let Some(dir) = path.parent()
        && let Ok(dir) = std::fs::File::open(if dir.as_os_str().is_empty() { Path::new(".") } else { dir })
    {
        let _ = dir.sync_all();
    }
    Ok(())
}

/// Finder tags, comments, the quarantine mark and access control lists belong to the file as the
/// user sees it; a rename would silently drop them with the old file.
#[cfg(target_os = "macos")]
fn copy_attributes(from: &Path, to: &Path) {
    use std::ffi::{CString, c_char, c_int, c_void};
    use std::os::unix::ffi::OsStrExt;
    unsafe extern "C" {
        fn copyfile(from: *const c_char, to: *const c_char, state: *mut c_void, flags: u32) -> c_int;
    }
    const COPYFILE_ACL: u32 = 1 << 0;
    const COPYFILE_XATTR: u32 = 1 << 2;
    let (Ok(from), Ok(to)) = (CString::new(from.as_os_str().as_bytes()), CString::new(to.as_os_str().as_bytes())) else {
        return;
    };
    // Both are valid, NUL-terminated paths that outlive the call; no state object is used.
    // Best effort: not every volume keeps these attributes.
    unsafe {
        let _ = copyfile(from.as_ptr(), to.as_ptr(), std::ptr::null_mut(), COPYFILE_ACL | COPYFILE_XATTR);
    }
}

#[cfg(not(target_os = "macos"))]
fn copy_attributes(_from: &Path, _to: &Path) {}

// ---------------------------------------------------------------------------
// Saving open documents.

/// Writes a document with unsaved changes to its file under `root`; Ok(true) when it wrote.
/// `saved` only ever becomes the text that is on disk.
pub fn save_doc(root: &Path, doc: &mut OpenDoc) -> io::Result<bool> {
    if doc.text == doc.saved {
        return Ok(false);
    }
    let path = root.join(&doc.rel);
    replace_file(&path, doc.text.as_bytes())?;
    doc.saved.clone_from(&doc.text);
    doc.edited_at = None;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One test's own folder, removed afterwards.
    struct Dir(PathBuf);

    impl Dir {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("typisch-files-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir.canonicalize().unwrap())
        }

        fn names(&self) -> Vec<String> {
            let mut names: Vec<String> = std::fs::read_dir(&self.0)
                .unwrap()
                .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
                .collect();
            names.sort();
            names
        }
    }

    impl Drop for Dir {
        fn drop(&mut self) {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = std::fs::set_permissions(&self.0, std::fs::Permissions::from_mode(0o755));
            }
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// A document as the editor holds it right after opening `rel`.
    fn opened(dir: &Dir, rel: &str) -> OpenDoc {
        let text = std::fs::read_to_string(dir.0.join(rel)).unwrap();
        OpenDoc {
            rel: rel.to_string(),
            saved: text.clone(),
            text,
            edited_at: None,
            state: None,
        }
    }

    #[test]
    fn atomic_write_replaces_the_file_and_leaves_no_temporary_file() {
        let dir = Dir::new("atomic");
        let path = dir.0.join("main.typ");
        atomic_write(&path, b"eins").unwrap();
        atomic_write(&path, b"zwei").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"zwei");
        assert_eq!(dir.names(), ["main.typ"]);
    }

    #[test]
    fn a_failed_write_keeps_the_old_file_and_cleans_up() {
        let dir = Dir::new("failed");
        // The target is a folder: the rename fails after the temporary file was written.
        let path = dir.0.join("ordner");
        std::fs::create_dir(&path).unwrap();
        std::fs::write(path.join("inhalt.typ"), "bleibt").unwrap();
        assert!(atomic_write(&path, b"neu").is_err());
        assert_eq!(dir.names(), ["ordner"]);
        assert_eq!(std::fs::read(path.join("inhalt.typ")).unwrap(), b"bleibt");
    }

    #[cfg(unix)]
    #[test]
    fn a_replaced_file_keeps_its_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let dir = Dir::new("mode");
        let path = dir.0.join("skript.typ");
        std::fs::write(&path, "alt").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o640)).unwrap();
        atomic_write(&path, b"neu").unwrap();
        assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o640);
    }

    #[cfg(unix)]
    #[test]
    fn a_write_protected_file_is_not_replaced() {
        use std::os::unix::fs::PermissionsExt;
        let dir = Dir::new("protected");
        let path = dir.0.join("fertig.typ");
        std::fs::write(&path, "alt").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o444)).unwrap();
        // Root may write anyway; the test says nothing then.
        if std::fs::OpenOptions::new().write(true).open(&path).is_ok() {
            return;
        }
        // The folder would allow the rename; the file's own protection still counts.
        let err = replace_file(&path, b"neu").unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::PermissionDenied);
        assert_eq!(std::fs::read(&path).unwrap(), b"alt");
        assert_eq!(dir.names(), ["fertig.typ"]);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn a_replaced_file_keeps_its_finder_tags() {
        let dir = Dir::new("xattr");
        let path = dir.0.join("main.typ");
        std::fs::write(&path, "alt").unwrap();
        let xattr = |args: &[&str]| {
            let out = std::process::Command::new("/usr/bin/xattr").args(args).arg(&path).output().unwrap();
            assert!(out.status.success(), "xattr {args:?}: {}", String::from_utf8_lossy(&out.stderr));
            String::from_utf8_lossy(&out.stdout).trim().to_string()
        };
        // Finder tags and comments are stored in attributes like this one.
        xattr(&["-w", "com.apple.metadata:_kMDItemUserTags", "Wichtig"]);
        replace_file(&path, b"neu").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"neu");
        assert_eq!(xattr(&["-p", "com.apple.metadata:_kMDItemUserTags"]), "Wichtig");
    }

    #[cfg(unix)]
    #[test]
    fn saving_through_a_symlink_keeps_the_link() {
        let dir = Dir::new("link");
        let real = dir.0.join("echt.typ");
        let link = dir.0.join("main.typ");
        std::fs::write(&real, "alt").unwrap();
        std::os::unix::fs::symlink(&real, &link).unwrap();
        replace_file(&link, b"neu").unwrap();
        assert!(link.symlink_metadata().unwrap().file_type().is_symlink());
        assert_eq!(std::fs::read(&real).unwrap(), b"neu");
        // atomic_write, used for the app's own files, replaces the link itself instead.
        atomic_write(&link, b"eigen").unwrap();
        assert!(link.symlink_metadata().unwrap().is_file());
        assert_eq!(std::fs::read(&real).unwrap(), b"neu");
    }

    #[cfg(unix)]
    #[test]
    fn a_planted_temp_symlink_is_not_followed() {
        let dir = Dir::new("planted");
        let victim = dir.0.join("victim.txt");
        std::fs::write(&victim, "keep me").unwrap();
        let target = dir.0.join("literatur.bib");
        std::fs::write(&target, "old").unwrap();
        let tmp = dir.0.join(".typisch-planted.tmp");
        std::os::unix::fs::symlink(&victim, &tmp).unwrap();
        let file = create_fresh(&tmp).unwrap();
        finish(&tmp, file, &target, b"new").unwrap();
        assert_eq!(std::fs::read_to_string(&victim).unwrap(), "keep me");
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "new");
        assert!(tmp.symlink_metadata().is_err());
    }

    #[test]
    fn save_doc_writes_changes_and_remembers_them() {
        let dir = Dir::new("save");
        std::fs::write(dir.0.join("main.typ"), "alt").unwrap();
        let mut doc = opened(&dir, "main.typ");
        assert!(matches!(save_doc(&dir.0, &mut doc), Ok(false)), "nothing to save");
        doc.text = "neu".into();
        doc.edited_at = Some(std::time::Instant::now());
        assert!(matches!(save_doc(&dir.0, &mut doc), Ok(true)));
        assert_eq!(std::fs::read_to_string(dir.0.join("main.typ")).unwrap(), "neu");
        assert_eq!((doc.saved.as_str(), doc.edited_at), ("neu", None));
        assert_eq!(dir.names(), ["main.typ"]);
    }

    #[cfg(unix)]
    #[test]
    fn a_failed_save_leaves_the_document_unsaved() {
        use std::os::unix::fs::PermissionsExt;
        let dir = Dir::new("readonly");
        let path = dir.0.join("main.typ");
        std::fs::write(&path, "alt").unwrap();
        let mut doc = opened(&dir, "main.typ");
        doc.text = "neu".into();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o444)).unwrap();
        std::fs::set_permissions(&dir.0, std::fs::Permissions::from_mode(0o555)).unwrap();
        // Root may write anywhere; the test says nothing then.
        if std::fs::write(dir.0.join("probe"), "").is_ok() {
            return;
        }
        assert!(save_doc(&dir.0, &mut doc).is_err());
        assert_eq!((doc.text.as_str(), doc.saved.as_str()), ("neu", "alt"), "still unsaved");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "alt");

        // A writable file in a folder that takes no new files is written in place.
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(matches!(save_doc(&dir.0, &mut doc), Ok(true)));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "neu");
        assert_eq!(doc.saved, "neu");
    }
}
