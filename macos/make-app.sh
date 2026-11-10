#!/bin/sh
# Builds Typisch and packs it as a Mac app: dist/Typisch.app (icon, name, version, Retina support).
# Run in the app folder:  sh macos/make-app.sh
# Then drag dist/Typisch.app into the Programme (Applications) folder.
#
# As built here the app is signed for this Mac only. On other Macs Gatekeeper refuses it; for
# those, set these before running the script:
#
#   UNIVERSAL=1        One app for Apple silicon and Intel Macs. Needs both Rust targets:
#                      rustup target add aarch64-apple-darwin x86_64-apple-darwin
#   DEVELOPER_ID=…     Sign with a certificate from the Apple Developer Program, by its name
#                      in the keychain ("Developer ID Application: Erika Mustermann (TEAMID)"),
#                      with the Hardened Runtime that notarisation asks for.
#   NOTARY_PROFILE=…   Also have Apple notarise the app and staple the ticket to it, so it opens
#                      on any Mac without a warning. Needs DEVELOPER_ID. The value is the name of
#                      a keychain profile, made once with:
#                      xcrun notarytool store-credentials <name> --apple-id … --team-id …
set -e
cd "$(dirname "$0")/.."
version="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n 1)"
date="$(sed -n 's/^release-date = "\(.*\)"/\1/p' Cargo.toml | head -n 1)"

if [ -n "$NOTARY_PROFILE" ] && [ -z "$DEVELOPER_ID" ]; then
  echo "NOTARY_PROFILE braucht auch DEVELOPER_ID: Apple notarisiert nur Apps mit Developer-ID-Signatur." >&2
  exit 1
fi

app="dist/Typisch.app"
rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"

# --locked: exactly the crate versions in Cargo.lock, as tested by CI.
if [ -n "$UNIVERSAL" ]; then
  for target in aarch64-apple-darwin x86_64-apple-darwin; do
    cargo build --release --locked --target "$target"
  done
  lipo -create -output "$app/Contents/MacOS/Typisch" \
    target/aarch64-apple-darwin/release/typisch target/x86_64-apple-darwin/release/typisch
else
  cargo build --release --locked
  cp target/release/typisch "$app/Contents/MacOS/Typisch"
fi

sed -e "s/__VERSION__/$version/g" -e "s/__DATE__/$date/g" macos/Info.plist > "$app/Contents/Info.plist"
printf 'APPL????' > "$app/Contents/PkgInfo"

# App icon: drawn by the program itself, in every size macOS asks for.
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
iconset="$work/Typisch.iconset"
mkdir -p "$iconset"
for size in 16 32 128 256 512; do
  "$app/Contents/MacOS/Typisch" --export-icon "$iconset/icon_${size}x${size}.png" "$size"
  "$app/Contents/MacOS/Typisch" --export-icon "$iconset/icon_${size}x${size}@2x.png" "$((size * 2))"
done
iconutil -c icns "$iconset" -o "$app/Contents/Resources/Typisch.icns"

# The signature covers the whole bundle (the program, Info.plist, the icon); an error stops the
# script, as an unsigned app does not start on Apple silicon.
if [ -n "$DEVELOPER_ID" ]; then
  codesign --force --options runtime --timestamp --sign "$DEVELOPER_ID" "$app"
else
  # A local ("ad hoc") signature, so macOS starts it without complaining on this Mac.
  codesign --force --sign - "$app"
fi
codesign --verify --strict "$app"

if [ -n "$NOTARY_PROFILE" ]; then
  # Apple takes an archive, checks it and files a ticket; stapling puts the ticket into the app,
  # so that it also opens without an internet connection.
  ditto -c -k --keepParent "$app" "$work/Typisch.zip"
  xcrun notarytool submit "$work/Typisch.zip" --keychain-profile "$NOTARY_PROFILE" --wait
  xcrun stapler staple "$app"
fi

echo "Fertig: $app"
