//! Passes the release date from Cargo.toml to the app as TYPISCH_RELEASE_DATE:
//!
//! ```toml
//! [package.metadata.typisch]
//! release-date = "November 2026"
//! ```
//!
//! Cargo hands a build script the package's name and version, but nothing from
//! `package.metadata`, so the manifest is read here. That is done by hand rather than with a
//! TOML parser as a build dependency: it is one `key = "value"` line in one table, and
//! macos/make-app.sh reads the same line with sed. The line must stay that simple (in that
//! table, on one line, in double quotes); a build that cannot find it says so.
fn main() {
    println!("cargo:rerun-if-changed=Cargo.toml");
    let manifest = std::fs::read_to_string("Cargo.toml").unwrap_or_default();
    let date = release_date(&manifest).unwrap_or_else(|| {
        println!("cargo:warning=no release-date = \"…\" line under [package.metadata.typisch] in Cargo.toml; the app shows no date");
        String::new()
    });
    println!("cargo:rustc-env=TYPISCH_RELEASE_DATE={date}");
}

/// The value of `release-date` in the `[package.metadata.typisch]` table.
fn release_date(manifest: &str) -> Option<String> {
    let mut in_table = false;
    for line in manifest.lines().map(str::trim) {
        if line.starts_with('[') {
            in_table = line == "[package.metadata.typisch]";
        } else if in_table && let Some(rest) = line.strip_prefix("release-date") {
            let value = rest.trim_start().strip_prefix('=')?.trim();
            // Only a plain quoted string: no escapes, nothing after it but a comment.
            let value = value.strip_prefix('"')?;
            let (date, after) = value.split_once('"')?;
            let after = after.trim();
            return (after.is_empty() || after.starts_with('#')).then(|| date.to_string());
        }
    }
    None
}
