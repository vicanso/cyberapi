#!/usr/bin/env bash
# The desktop app, packaged for the host it is built on. Called by
# `make desktop-dist`; needs cargo-bundle 0.11 (`cargo install --locked
# cargo-bundle@0.11.0`; 0.12 reads [package.metadata.bundle] differently).
#
#   macOS    CyberAPI.app in cyberapi-<version>-<triple>.dmg, and cyberapi-cli-<version>-<triple>.tar.gz
#   Linux    the .deb cargo-bundle makes, a tarball of the binary, and cyberapi-cli-<version>-<triple>.tar.gz
#   Windows  a zip of cyberapi.exe, and cyberapi-cli-<version>-<triple>.zip
#
# All under <cargo target dir>/desktop-dist, each with a .sha256. The
# version in the names is Cargo.toml's, or a nightly's own when
# CYBERAPI_VERSION gives it (#54; the binary reports the same): cargo-bundle
# names the .deb for Cargo.toml's, so it is renamed, and the package inside
# keeps that version.
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

CARGO_VERSION=$(sed -n 's/^version = "\([^"]*\)"/\1/p' Cargo.toml | head -1)
VERSION=${CYBERAPI_VERSION:-$CARGO_VERSION}
TARGET=$(rustc -vV | sed -n 's/^host: //p')
NAME="cyberapi-${VERSION}-${TARGET}"
CLI_NAME="cyberapi-cli-${VERSION}-${TARGET}"
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
    # The .app (scripts/bundle.sh), then the disk image around it.
    scripts/bundle.sh
    APP="$TARGET_DIR/release/bundle/osx/CyberAPI.app"
    STAGE=$(mktemp -d)
    trap 'rm -rf "$STAGE"' EXIT
    cp -R "$APP" "$STAGE/"
    ln -s /Applications "$STAGE/Applications"
    hdiutil create -quiet -volname "CyberAPI" -srcfolder "$STAGE" -ov -format UDZO "$OUT_DIR/$NAME.dmg"
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
    cargo build --release -p cyberapi-cli
    if [ -n "${CYBERAPI_SIGN_IDENTITY:-}" ]; then
      codesign --force --options runtime --timestamp --sign "$CYBERAPI_SIGN_IDENTITY" "$TARGET_DIR/release/cyberapi-cli"
    else
      codesign --force --sign - "$TARGET_DIR/release/cyberapi-cli"
    fi
    CLI_STAGE="$OUT_DIR/$CLI_NAME"
    rm -rf "$CLI_STAGE"
    mkdir -p "$CLI_STAGE"
    cp "$TARGET_DIR/release/cyberapi-cli" LICENSE "$CLI_STAGE/"
    tar -czf "$OUT_DIR/$CLI_NAME.tar.gz" -C "$OUT_DIR" "$CLI_NAME"
    rm -rf "$CLI_STAGE"
    checksum "$CLI_NAME.tar.gz"
    ;;
  Linux)
    need_bundle
    cargo bundle --release --format deb
    for deb in "$TARGET_DIR"/release/bundle/deb/*.deb; do
      named=$(basename "$deb")
      named=${named/"_${CARGO_VERSION}_"/"_${VERSION}_"}
      cp "$deb" "$OUT_DIR/$named"
      checksum "$named"
    done
    STAGE="$OUT_DIR/$NAME"
    rm -rf "$STAGE"
    mkdir -p "$STAGE"
    cp "$TARGET_DIR/release/cyberapi" LICENSE "$STAGE/"
    tar -czf "$OUT_DIR/$NAME.tar.gz" -C "$OUT_DIR" "$NAME"
    checksum "$NAME.tar.gz"
    cargo build --release -p cyberapi-cli
    CLI_STAGE="$OUT_DIR/$CLI_NAME"
    rm -rf "$CLI_STAGE"
    mkdir -p "$CLI_STAGE"
    cp "$TARGET_DIR/release/cyberapi-cli" LICENSE "$CLI_STAGE/"
    tar -czf "$OUT_DIR/$CLI_NAME.tar.gz" -C "$OUT_DIR" "$CLI_NAME"
    rm -rf "$CLI_STAGE"
    checksum "$CLI_NAME.tar.gz"
    ;;
  MINGW* | MSYS* | CYGWIN*)
    cargo build --release
    STAGE="$OUT_DIR/$NAME"
    rm -rf "$STAGE"
    mkdir -p "$STAGE"
    cp "$TARGET_DIR/release/cyberapi.exe" LICENSE "$STAGE/"
    (cd "$OUT_DIR" && powershell -NoProfile -Command "Compress-Archive -Force -Path '$NAME' -DestinationPath '$NAME.zip'")
    checksum "$NAME.zip"
    cargo build --release -p cyberapi-cli
    CLI_STAGE="$OUT_DIR/$CLI_NAME"
    rm -rf "$CLI_STAGE"
    mkdir -p "$CLI_STAGE"
    cp "$TARGET_DIR/release/cyberapi-cli.exe" LICENSE "$CLI_STAGE/"
    (cd "$OUT_DIR" && powershell -NoProfile -Command "Compress-Archive -Force -Path '$CLI_NAME' -DestinationPath '$CLI_NAME.zip'")
    rm -rf "$CLI_STAGE"
    checksum "$CLI_NAME.zip"
    ;;
  *)
    echo "no package for $(uname -s)" >&2
    exit 1
    ;;
esac
