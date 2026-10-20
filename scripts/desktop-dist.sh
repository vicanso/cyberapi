#!/usr/bin/env bash
# The desktop app, packaged for the host it is built on. Called by
# `make desktop-dist`; needs cargo-bundle (`cargo install cargo-bundle`).
#
#   macOS    Cyber API.app, ad-hoc signed, in cyberapi-<version>-<triple>.dmg
#   Linux    the .deb cargo-bundle makes, and a tarball of the binary
#   Windows  a zip of cyberapi.exe
#
# All under <cargo target dir>/desktop-dist, each with a .sha256.
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
    # Apple silicon runs no unsigned code; an ad-hoc signature is enough to
    # start on the machine that built it, and for Gatekeeper's "Open" on
    # another. Distribution beyond that needs a Developer ID and notarization.
    codesign --force --deep --sign - "$APP"
    STAGE=$(mktemp -d)
    trap 'rm -rf "$STAGE"' EXIT
    cp -R "$APP" "$STAGE/"
    ln -s /Applications "$STAGE/Applications"
    hdiutil create -quiet -volname "Cyber API" -srcfolder "$STAGE" -ov -format UDZO "$OUT_DIR/$NAME.dmg"
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
