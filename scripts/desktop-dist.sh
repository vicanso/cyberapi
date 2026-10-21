#!/usr/bin/env bash
# The desktop app, packaged for the host it is built on. Called by
# `make desktop-dist`; needs cargo-bundle 0.11 (`cargo install --locked
# cargo-bundle@0.11.0`; 0.12 reads [package.metadata.bundle] differently).
#
#   macOS    Cyber API.app in cyberapi-<version>-<triple>.dmg
#   Linux    the .deb cargo-bundle makes, and a tarball of the binary
#   Windows  a zip of cyberapi.exe
#
# All under <cargo target dir>/desktop-dist, each with a .sha256.
#
# On macOS the app is ad-hoc signed unless these name a Developer ID:
#
#   CYBERAPI_SIGN_IDENTITY   a codesign identity in the keychain, e.g.
#                            "Developer ID Application: Name (TEAMID)": signed
#                            with the hardened runtime and Entitlements.plist
#   CYBERAPI_NOTARY_APPLE_ID, CYBERAPI_NOTARY_PASSWORD (an app-specific
#   password), CYBERAPI_NOTARY_TEAM_ID
#                            with all three as well, the .dmg is notarized
#                            and the ticket stapled to it
set -euo pipefail
cd "$(dirname "$0")/.."

VERSION=$(sed -n 's/^version = "\([^"]*\)"/\1/p' Cargo.toml | head -1)
TARGET=$(rustc -vV | sed -n 's/^host: //p')
NAME="cyberapi-${VERSION}-${TARGET}"
# CARGO_TARGET_DIR when set (CI sets it), else asked of cargo, never
# assumed: `build.target-dir` may move it.
TARGET_DIR="${CARGO_TARGET_DIR:-$(cargo metadata --format-version 1 --no-deps \
  | python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"])')}"
OUT_DIR="$TARGET_DIR/desktop-dist"
mkdir -p "$OUT_DIR"

checksum() {
  if command -v sha256sum >/dev/null 2>&1; then
    (cd "$OUT_DIR" && sha256sum "$1" > "$1.sha256")
  else
    (cd "$OUT_DIR" && shasum -a 256 "$1" > "$1.sha256")
  fi
  echo "$OUT_DIR/$1"
}

need_bundle() {
  if ! command -v cargo-bundle >/dev/null 2>&1; then
    echo "cargo-bundle is required: cargo install cargo-bundle" >&2
    exit 1
  fi
}

case "$(uname -s)" in
  Darwin)
    need_bundle
    # No `--bin`: with it cargo-bundle ignores [package.metadata.bundle].
    cargo bundle --release
    APP="$TARGET_DIR/release/bundle/osx/Cyber API.app"
    if [ -n "${CYBERAPI_SIGN_IDENTITY:-}" ]; then
      # The hardened runtime and a secure timestamp are what notarization
      # asks for. Apple's timestamp server fails now and then, so retry.
      for attempt in 1 2 3 4 5; do
        if codesign --force --deep --options runtime --timestamp \
          --entitlements Entitlements.plist --sign "$CYBERAPI_SIGN_IDENTITY" "$APP"; then
          break
        fi
        [ "$attempt" -eq 5 ] && { echo "codesign kept failing" >&2; exit 1; }
        echo "codesign attempt $attempt failed; retrying in 15s" >&2
        sleep 15
      done
      codesign -dvv "$APP" 2>&1 | grep -q "^Timestamp=" || { echo "no secure timestamp in the signature" >&2; exit 1; }
    else
      # Apple silicon runs no unsigned code; an ad-hoc signature is enough to
      # start on the machine that built it, and for Gatekeeper's "Open" on
      # another. Distribution beyond that needs a Developer ID.
      codesign --force --deep --sign - "$APP"
    fi
    STAGE=$(mktemp -d)
    trap 'rm -rf "$STAGE"' EXIT
    cp -R "$APP" "$STAGE/"
    ln -s /Applications "$STAGE/Applications"
    hdiutil create -quiet -volname "Cyber API" -srcfolder "$STAGE" -ov -format UDZO "$OUT_DIR/$NAME.dmg"
    if [ -n "${CYBERAPI_SIGN_IDENTITY:-}" ] && [ -n "${CYBERAPI_NOTARY_APPLE_ID:-}" ] \
      && [ -n "${CYBERAPI_NOTARY_PASSWORD:-}" ] && [ -n "${CYBERAPI_NOTARY_TEAM_ID:-}" ]; then
      xcrun notarytool submit "$OUT_DIR/$NAME.dmg" --wait \
        --apple-id "$CYBERAPI_NOTARY_APPLE_ID" \
        --password "$CYBERAPI_NOTARY_PASSWORD" \
        --team-id "$CYBERAPI_NOTARY_TEAM_ID"
      # The ticket is fetched from Apple, which drops the connection now and
      # then right after accepting the submission.
      for attempt in 1 2 3 4 5; do
        xcrun stapler staple "$OUT_DIR/$NAME.dmg" && break
        [ "$attempt" -eq 5 ] && { echo "stapling kept failing" >&2; exit 1; }
        echo "staple attempt $attempt failed; retrying in 15s" >&2
        sleep 15
      done
    fi
    checksum "$NAME.dmg"
    ;;
  Linux)
    need_bundle
    cargo bundle --release --format deb
    for deb in "$TARGET_DIR"/release/bundle/deb/*.deb; do
      cp "$deb" "$OUT_DIR/"
      checksum "$(basename "$deb")"
    done
    STAGE="$OUT_DIR/$NAME"
    rm -rf "$STAGE"
    mkdir -p "$STAGE"
    cp "$TARGET_DIR/release/cyberapi" LICENSE "$STAGE/"
    tar -czf "$OUT_DIR/$NAME.tar.gz" -C "$OUT_DIR" "$NAME"
    checksum "$NAME.tar.gz"
    ;;
  MINGW* | MSYS* | CYGWIN*)
    cargo build --release
    STAGE="$OUT_DIR/$NAME"
    rm -rf "$STAGE"
    mkdir -p "$STAGE"
    cp "$TARGET_DIR/release/cyberapi.exe" LICENSE "$STAGE/"
    (cd "$OUT_DIR" && powershell -NoProfile -Command "Compress-Archive -Force -Path '$NAME' -DestinationPath '$NAME.zip'")
    checksum "$NAME.zip"
    ;;
  *)
    echo "no package for $(uname -s)" >&2
    exit 1
    ;;
esac
