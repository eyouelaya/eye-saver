#!/usr/bin/env bash
# Packs dist/EyeSaver.app into a disk image, building it first if necessary.
#
# Notarisation is attempted only when all three credentials are present; otherwise the
# image is produced unsigned, and the README tells users to right-click -> Open once.
#
#   APPLE_SIGNING_IDENTITY  Developer ID Application: ...
#   AC_APPLE_ID             Apple ID for notarytool
#   AC_PASSWORD             app-specific password
#   AC_TEAM_ID              team identifier
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)"
APP="$ROOT/dist/EyeSaver.app"
DMG="$ROOT/dist/EyeSaver-$VERSION-universal.dmg"
STAGE="$ROOT/dist/dmg"

# Always rebuild: reusing whatever happens to be in dist/ would happily ship the debug,
# single-architecture bundle left behind by `make dev-bundle`.
"$ROOT/packaging/macos/build-app.sh"

echo "==> staging"
rm -rf "$STAGE" "$DMG"
mkdir -p "$STAGE"
cp -R "$APP" "$STAGE/"
ln -s /Applications "$STAGE/Applications"

echo "==> creating $DMG"
hdiutil create \
  -volname "EyeSaver $VERSION" \
  -srcfolder "$STAGE" \
  -ov -format UDZO \
  "$DMG" >/dev/null
rm -rf "$STAGE"

if [[ -n "${APPLE_SIGNING_IDENTITY:-}" && -n "${AC_APPLE_ID:-}" && -n "${AC_PASSWORD:-}" && -n "${AC_TEAM_ID:-}" ]]; then
  echo "==> signing and notarising"
  codesign --force --sign "$APPLE_SIGNING_IDENTITY" --timestamp "$DMG"
  xcrun notarytool submit "$DMG" \
    --apple-id "$AC_APPLE_ID" --password "$AC_PASSWORD" --team-id "$AC_TEAM_ID" --wait
  xcrun stapler staple "$DMG"
else
  echo "==> not notarised: users will need to right-click -> Open on first launch"
fi

echo "==> built $DMG"
hdiutil verify "$DMG" >/dev/null && echo "image verified"
