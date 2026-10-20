#!/usr/bin/env bash
# The browser bundle: the kit's icons copied beside the page, then the wasm
# built by wasm-pack into cyberapi-web/www/wasm — the directory
# `make web-serve` serves.
#
#   scripts/web-bundle.sh            the iteration build: `--profile web`, the
#                                    name section kept for readable panics, no
#                                    wasm-opt
#   scripts/web-bundle.sh --release  the shipped form: `--profile web-release`
#                                    (fat LTO, one codegen unit, stripped),
#                                    then `wasm-opt -Oz`
#
# wasm-pack runs from cyberapi-web/ so rustup reads the `rust-toolchain.toml`
# there and selects the nightly the browser backend needs.
set -euo pipefail
cd "$(dirname "$0")/.."

mode=dev
case "${1:-}" in
  "") ;;
  --release) mode=release ;;
  *)
    echo "usage: $0 [--release]" >&2
    exit 2
    ;;
esac

if ! command -v wasm-pack >/dev/null 2>&1; then
  echo "wasm-pack is required: cargo install wasm-pack" >&2
  exit 1
fi
# Checked before the build: finding out after a fat-LTO build is the
# expensive way.
if [ "$mode" = release ] && ! command -v wasm-opt >/dev/null 2>&1; then
  echo "wasm-opt is required for --release: brew install binaryen, or cargo install wasm-opt" >&2
  exit 1
fi

# The kit's icons are fetched by the page (`/assets/icons/<name>.svg`), not
# embedded, so they are copied from wherever cargo has the crate.
kit_manifest=$(cargo metadata --format-version 1 2>/dev/null \
  | python3 -c 'import json,sys; m=json.load(sys.stdin); print(next(p["manifest_path"] for p in m["packages"] if p["name"]=="gpui-kit-assets"))')
kit_dir=$(dirname "$kit_manifest")
mkdir -p cyberapi-web/www/assets/icons
cp "$kit_dir"/assets/icons/*.svg cyberapi-web/www/assets/icons/
echo "icons: $(ls cyberapi-web/www/assets/icons | wc -l | tr -d ' ') from $kit_dir"

profile=web
if [ "$mode" = release ]; then
  profile=web-release
fi
(
  cd cyberapi-web
  # `--no-opt` in both modes: the release form runs wasm-opt itself below,
  # with its flags in view, rather than through wasm-pack's defaults.
  wasm-pack build . --target web --profile "$profile" --no-opt --out-dir www/wasm --out-name cyberapi_web
)

wasm=cyberapi-web/www/wasm/cyberapi_web_bg.wasm
size() { wc -c < "$1" | tr -d ' '; }
mib() { awk -v b="$1" 'BEGIN { printf "%.1f MiB", b / 1048576 }'; }

if [ "$mode" = release ]; then
  before=$(size "$wasm")
  # The features wasm-opt may assume, spelled out: cargo's `strip = true`
  # removes the module's own `target_features` section, so wasm-opt would see
  # an MVP module and refuse what it then meets. `threads` is there because
  # gpui-pre-web compiles atomic instructions into the module even though the
  # app runs single-threaded.
  features=(
    --enable-bulk-memory
    --enable-mutable-globals
    --enable-sign-ext
    --enable-nontrapping-float-to-int
    --enable-multivalue
    --enable-reference-types
    --enable-threads
  )
  wasm-opt -Oz --strip-debug --strip-producers "${features[@]}" -o "$wasm.opt" "$wasm"
  mv "$wasm.opt" "$wasm"
  echo "wasm-opt -Oz: $(mib "$before") -> $(mib "$(size "$wasm")")"
fi

echo "wasm ($mode): $(mib "$(size "$wasm")")"
