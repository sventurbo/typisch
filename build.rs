//! Passes the release date from Cargo.toml ([package.metadata.typisch] release-date) to the app.
fn main() {
    println!("cargo:rerun-if-changed=Cargo.toml");
    let manifest = std::fs::read_to_string("Cargo.toml").unwrap_or_default();
    let date = manifest
        .lines()
        .find_map(|l| l.trim().strip_prefix("release-date").map(|rest| rest.trim_start_matches([' ', '=']).trim().trim_matches('"').to_string()))
        .unwrap_or_default();
    println!("cargo:rustc-env=TYPISCH_RELEASE_DATE={date}");
}
