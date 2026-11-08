<p align="center">
  <img src="assets/typisch.svg" width="128" alt="Typisch logo">
</p>

<h1 align="center">Typisch</h1>

<p align="center">
  A fast, native <a href="https://typst.app">Typst</a> editor for macOS.<br>
  Write your document and watch it take shape as you type.
</p>

---

- **Live preview** with clickable links
- **PDF export**, ready to print or share
- **Built-in templates** for letters, CVs, reports and more
- **Symbol palette** – search the full Typst symbol table or just draw the symbol
- **Fully native** – written in Rust, no browser, no account

## Get started

Requires the newest stable Rust (`rustup update stable`).

```sh
cargo run --release -- [folder-or-file]
```

To build the Mac app, run `sh macos/make-app.sh` and drag `dist/Typisch.app` into your Applications folder.

<details>
<summary>Command-line options</summary>

| Option | Purpose |
| --- | --- |
| `[folder-or-file]` | Open a project folder or `.typ` file |
| `--export-icon <icon.png> [size]` | Write the app icon as a PNG and exit |
| `--screenshot <image.png>` | Render the window to an image (for testing) |

</details>

## Continuous integration

[CI](.github/workflows/ci.yml) lints (clippy) and tests every pull request and every push to `master` on macOS and Linux with the newest stable Rust, and checks the dependencies for security advisories ([deny.toml](deny.toml)). Every Monday it repeats this with all crates moved to their newest compatible versions. [Dependabot](.github/dependabot.yml) opens weekly pull requests for new crate and GitHub Actions releases.
