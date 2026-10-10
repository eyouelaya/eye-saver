#!/usr/bin/env bash
# Builds EyeSaver.app.
#
#   ./packaging/macos/build-app.sh          universal (Apple silicon + Intel)
#   ./packaging/macos/build-app.sh --dev    this machine's architecture only, debug
#
# A bundle is needed even during development: macOS attributes a notification to the
# surrounding .app, so an unbundled `cargo run` cannot show one.
#
# Signing: ad-hoc by default, which is enough to run locally. Set APPLE_SIGNING_IDENTITY
# to a Developer ID to sign properly, and see make-dmg.sh for notarisation.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

DEV=0
[[ "${1:-}" == "--dev" ]] && DEV=1

VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)"
APP="$ROOT/dist/EyeSaver.app"
MACOS_DIR="$APP/Contents/MacOS"
RESOURCES="$APP/Contents/Resources"

echo "==> generating icons"
cargo run -p xtask --quiet -- icons >/dev/null

if (( DEV )); then
  echo "==> building for this machine (debug)"
  cargo build -p eye-saver
  BINARIES=("$ROOT/target/debug/eye-saver")
else
  echo "==> building for Apple silicon and Intel (release)"
  for target in aarch64-apple-darwin x86_64-apple-darwin; do
    rustup target add "$target" >/dev/null 2>&1 || true
    cargo build --release -p eye-saver --target "$target"
  done
  BINARIES=(
    "$ROOT/target/aarch64-apple-darwin/release/eye-saver"
    "$ROOT/target/x86_64-apple-darwin/release/eye-saver"
  )
fi

echo "==> assembling $APP"
rm -rf "$APP"
mkdir -p "$MACOS_DIR" "$RESOURCES"

if (( ${#BINARIES[@]} > 1 )); then
  lipo -create -output "$MACOS_DIR/eye-saver" "${BINARIES[@]}"
else
  cp "${BINARIES[0]}" "$MACOS_DIR/eye-saver"
fi
chmod +x "$MACOS_DIR/eye-saver"
cp "$ROOT/packaging/generated/EyeSaver.icns" "$RESOURCES/EyeSaver.icns"

# LSUIElement is what makes this a menu bar extra: no Dock icon, no app switcher entry.
cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>CFBundleDevelopmentRegion</key>
	<string>en</string>
	<key>CFBundleDisplayName</key>
	<string>EyeSaver</string>
	<key>CFBundleExecutable</key>
	<string>eye-saver</string>
	<key>CFBundleIconFile</key>
	<string>EyeSaver</string>
	<key>CFBundleIdentifier</key>
	<string>com.eyesaver.app</string>
	<key>CFBundleInfoDictionaryVersion</key>
	<string>6.0</string>
	<key>CFBundleName</key>
	<string>EyeSaver</string>
	<key>CFBundlePackageType</key>
	<string>APPL</string>
	<key>CFBundleShortVersionString</key>
	<string>$VERSION</string>
	<key>CFBundleVersion</key>
	<string>$VERSION</string>
	<key>LSApplicationCategoryType</key>
	<string>public.app-category.healthcare-fitness</string>
	<key>LSMinimumSystemVersion</key>
	<string>11.0</string>
	<key>LSUIElement</key>
	<true/>
	<key>NSHighResolutionCapable</key>
	<true/>
	<key>NSHumanReadableCopyright</key>
	<string>MIT licensed</string>
</dict>
</plist>
PLIST

printf 'APPL????' > "$APP/Contents/PkgInfo"

IDENTITY="${APPLE_SIGNING_IDENTITY:--}"
if [[ "$IDENTITY" == "-" ]]; then
  echo "==> signing ad-hoc (set APPLE_SIGNING_IDENTITY for a Developer ID signature)"
  codesign --force --sign - --timestamp=none "$APP"
else
  echo "==> signing with $IDENTITY"
  codesign --force --options runtime --timestamp --sign "$IDENTITY" "$APP"
fi

codesign --verify --verbose=1 "$APP"
echo "==> built $APP"
file "$MACOS_DIR/eye-saver"
