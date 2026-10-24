#!/usr/bin/env bash
# The browser build's deployment: the bridge, built in release form with the
# release page compiled in, in one tarball. Called by `make web-dist`.
#
#   <cargo target dir>/web-dist/cyberapi-web-<version>-<host triple>.tar.gz
#   (and its .sha256; the script prints the paths)
#
# The page is rebuilt in its release form first: the bridge embeds whatever
# cyberapi-web/www holds when it compiles, which after a `make web-bundle`
# is the large iteration build. The binary is the host's own: build on the
# platform it will run on.
set -euo pipefail
cd "$(dirname "$0")/.."

VERSION=$(sed -n 's/^version = "\([^"]*\)"/\1/p' Cargo.toml | head -1)
TARGET=$(rustc -vV | sed -n 's/^host: //p')
NAME="cyberapi-web-${VERSION}-${TARGET}"
# CARGO_TARGET_DIR when set (CI sets it), else asked of cargo, never
# assumed: `build.target-dir` may move it.
TARGET_DIR="${CARGO_TARGET_DIR:-$(cargo metadata --format-version 1 --no-deps \
  | python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"])')}"
OUT_DIR="$TARGET_DIR/web-dist"
STAGE="$OUT_DIR/$NAME"

echo "==> page (release)"
scripts/web-bundle.sh --release

echo "==> cyberapi-bridge (release)"
cargo build --release -p cyberapi-bridge
BIN="$TARGET_DIR/release/cyberapi-bridge"
if [ -f "$BIN.exe" ]; then
  BIN="$BIN.exe"
fi

rm -rf "$STAGE"
mkdir -p "$STAGE"
cp "$BIN" LICENSE "$STAGE/"
cat > "$STAGE/README.txt" <<README
Cyber API, browser build $VERSION ($TARGET)

cyberapi-bridge is the whole deployment: it keeps the accounts and the
projects in PostgreSQL or MySQL, sends the requests the page prepares, and
serves the page (compiled in) and its API from one origin.

Run:

  CYBERAPI_BRIDGE_STORE=postgres://user:password@host/db ./cyberapi-bridge

then open http://127.0.0.1:7380/. The schema is created on start. With no
account yet, the bridge logs a setup token (CYBERAPI_BRIDGE_SETUP_TOKEN
chooses it); the page asks for it to make the first administrator, who then
adds the other accounts from the settings menu.

--listen 0.0.0.0:7380 listens beyond this machine: put TLS in front. The
login cookie is Secure; --insecure-cookie lifts that for plain HTTP on this
machine only.

Requests are sent from the bridge, to public and private networks, never to
this host (loopback) or link-local addresses (the cloud metadata address
among them). --allow-target 127.0.0.1/32 (or CYBERAPI_BRIDGE_ALLOW_TARGETS)
allows ranges back; --public-only (or CYBERAPI_BRIDGE_PUBLIC_ONLY=1) refuses
private networks too, for a bridge open to the internet.

Failed sign-ins are slowed per name and per address; behind a reverse proxy
every request comes from the proxy's address, so rate-limit there as well.

--base-path /cyberapi (or CYBERAPI_BRIDGE_BASE_PATH) serves the page and the
API under that path, for a reverse proxy that keeps the prefix; the login
cookie is kept to it.

--www <dir> serves a directory as the page instead of the one built in.
RUST_LOG=debug logs more.

The whole guide (Docker Compose, systemd, a reverse proxy, upgrades):
https://github.com/vicanso/cyberapi/blob/main/docs/DEPLOY.md
README

TARBALL="$OUT_DIR/$NAME.tar.gz"
tar -czf "$TARBALL" -C "$OUT_DIR" "$NAME"
if command -v sha256sum >/dev/null 2>&1; then
  (cd "$OUT_DIR" && sha256sum "$NAME.tar.gz" > "$NAME.tar.gz.sha256")
else
  (cd "$OUT_DIR" && shasum -a 256 "$NAME.tar.gz" > "$NAME.tar.gz.sha256")
fi

mib() { awk -v b="$1" 'BEGIN { printf "%.1f MiB", b / 1048576 }'; }
echo "bridge:  $(mib "$(wc -c < "$BIN" | tr -d ' ')") (page included)"
echo "tarball: $(mib "$(wc -c < "$TARBALL" | tr -d ' ')")  $TARBALL"
cat "$TARBALL.sha256"
