#!/usr/bin/env bash
# The macOS application. Called by `make bundle`, and by `make desktop-dist`
# before it packs the disk image. Needs cargo-bundle 0.11
# (`cargo install --locked cargo-bundle@0.11.0`; 0.12 reads
# [package.metadata.bundle] differently).
#
# Writes CyberAPI.app to <cargo target dir>/release/bundle/osx/ and prints
# that path. Ad-hoc signed unless CYBERAPI_SIGN_IDENTITY names a Developer
# ID, in which case the hardened runtime and Entitlements.plist are used.
set -euo pipefail
cd "$(dirname "$0")/.."

if [ "$(uname -s)" != "Darwin" ]; then
  echo "make bundle builds the macOS app; this host is $(uname -s)" >&2
  exit 1
fi

if ! command -v cargo-bundle >/dev/null 2>&1; then
  echo "cargo-bundle 0.11.0 is required: cargo install --locked cargo-bundle@0.11.0" >&2
  exit 1
fi

# CARGO_TARGET_DIR when set (CI sets it), else asked of cargo, never
# assumed: `build.target-dir` may move it.
TARGET_DIR="${CARGO_TARGET_DIR:-$(cargo metadata --format-version 1 --no-deps \
  | python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"])')}"
APP="$TARGET_DIR/release/bundle/osx/CyberAPI.app"

# No `--bin`: with it cargo-bundle ignores [package.metadata.bundle].
cargo bundle --release

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
  # open the app on the machine that built it.
  codesign --force --deep --sign - "$APP"
fi

echo "$APP"
