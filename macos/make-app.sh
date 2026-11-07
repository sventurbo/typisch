#!/bin/sh
# Builds Typisch and packs it as a Mac app: dist/Typisch.app (icon, name, version, Retina support).
# Run in the app folder:  sh macos/make-app.sh
# Then drag dist/Typisch.app into the Programme (Applications) folder.
set -e
cd "$(dirname "$0")/.."
version="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n 1)"
date="$(sed -n 's/^release-date = "\(.*\)"/\1/p' Cargo.toml | head -n 1)"

cargo build --release

app="dist/Typisch.app"
rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp target/release/typisch "$app/Contents/MacOS/Typisch"
sed -e "s/__VERSION__/$version/g" -e "s/__DATE__/$date/g" macos/Info.plist > "$app/Contents/Info.plist"
printf 'APPL????' > "$app/Contents/PkgInfo"

# App icon: drawn by the program itself, in every size macOS asks for.
iconset="$(mktemp -d)/Typisch.iconset"
mkdir -p "$iconset"
for size in 16 32 128 256 512; do
  target/release/typisch --export-icon "$iconset/icon_${size}x${size}.png" "$size"
  target/release/typisch --export-icon "$iconset/icon_${size}x${size}@2x.png" "$((size * 2))"
done
iconutil -c icns "$iconset" -o "$app/Contents/Resources/Typisch.icns"

# Local signature so macOS starts it without complaining on this Mac.
codesign --force --deep --sign - "$app" >/dev/null 2>&1 || true

echo "Fertig: $app"
