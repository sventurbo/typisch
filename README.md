# Typisch

A native Typst editor with live preview and PDF export, written in Rust (egui + Typst 0.15). Built for macOS.

## Requirements

- Rust 1.85+ (edition 2024)

## Run

```sh
cargo run --release -- [folder-or-file]
```

## Build the Mac app

```sh
sh macos/make-app.sh
```

This creates `dist/Typisch.app`. Drag it into the Applications folder.

## Command-line options

| Option | Purpose |
| --- | --- |
| `[folder-or-file]` | Open a project folder or `.typ` file |
| `--export-icon <icon.png> [size]` | Write the app icon as a PNG and exit |
| `--screenshot <image.png>` | Render the window to an image (for testing) |
