#!/usr/bin/env bash
# The checksums a release carries, made from the packages package.yml built,
# all in one directory. Called by release.yml and nightly.yml before they
# attach the directory.
#
#   scripts/checksums.sh <dir>            SHA256SUMS
#   scripts/checksums.sh <dir> vX.Y.Z     SHA256SUMS and latest.json
#
# SHA256SUMS is sha256sum's format, sorted by name (`sha256sum -c
# SHA256SUMS` checks a download). latest.json is what the desktop's update
# reads, as zedis's: the version, and for each package its os, arch, kind,
# download URL, sha256 and size. Its version is the tag's, which must be
# Cargo.toml's. The packages are named for that version, or for a nightly's
# own when CYBERAPI_VERSION gives it (#54), as they were built.
#
# Each package's own <name>.sha256 (desktop-dist.sh and web-dist.sh make it
# on the machine that built the package) is checked against the file here,
# then dropped: the release carries the one SHA256SUMS. A file whose name
# says no platform fails the run, rather than going out without its row.
set -euo pipefail
shopt -s nullglob
cd "$(dirname "$0")/.."

DIR=${1:?usage: checksums.sh <dir> [vX.Y.Z]}
TAG=${2:-}
REPO=vicanso/cyberapi
VERSION=${CYBERAPI_VERSION:-$(sed -n 's/^version = "\([^"]*\)"/\1/p' Cargo.toml | head -1)}
if [ -n "$TAG" ] && [ "$TAG" != "v$VERSION" ]; then
  echo "the tag is $TAG, Cargo.toml says $VERSION" >&2
  exit 1
fi
cd "$DIR"

sha256() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{print $1}'
  else
    shasum -a 256 "$1" | awk '{print $1}'
  fi
}

# "<os> <arch> <kind>" of a package, from the name desktop-dist.sh or
# web-dist.sh gave it.
describe() {
  local name=$1 rest kind
  case "$name" in
    "cyberapi_${VERSION}_amd64.deb") echo "linux x86_64 deb"; return ;;
    "cyberapi_${VERSION}_arm64.deb") echo "linux aarch64 deb"; return ;;
    "cyberapi-web-$VERSION-"*.tar.gz) kind=web; rest=${name#"cyberapi-web-$VERSION-"}; rest=${rest%.tar.gz} ;;
    "cyberapi-$VERSION-"*.tar.gz) kind=tarball; rest=${name#"cyberapi-$VERSION-"}; rest=${rest%.tar.gz} ;;
    "cyberapi-$VERSION-"*.dmg) kind=dmg; rest=${name#"cyberapi-$VERSION-"}; rest=${rest%.dmg} ;;
    "cyberapi-$VERSION-"*.zip) kind=zip; rest=${name#"cyberapi-$VERSION-"}; rest=${rest%.zip} ;;
    *) return 1 ;;
  esac
  # What is left is the target triple: aarch64-apple-darwin and so on.
  case "$rest" in
    *-apple-darwin) echo "macos ${rest%%-*} $kind" ;;
    *-linux-*) echo "linux ${rest%%-*} $kind" ;;
    *-windows-*) echo "windows ${rest%%-*} $kind" ;;
    *) return 1 ;;
  esac
}

ROWS=$(mktemp)
trap 'rm -f "$ROWS"' EXIT
for f in *; do
  case "$f" in
    *.sha256 | SHA256SUMS | latest.json) continue ;;
  esac
  what=$(describe "$f") || { echo "not a package this script knows: $f" >&2; exit 1; }
  sha=$(sha256 "$f")
  if [ -f "$f.sha256" ]; then
    built=$(awk '{print $1}' "$f.sha256")
    [ "$built" = "$sha" ] || { echo "$f is not what was built: $sha, built as $built" >&2; exit 1; }
  fi
  echo "$what $f $sha $(wc -c <"$f" | tr -d ' ')" >>"$ROWS"
done
[ -s "$ROWS" ] || { echo "no packages in $DIR" >&2; exit 1; }
rm -f -- *.sha256

# os arch kind name sha256 size
awk '{print $5 "  " $4}' "$ROWS" | LC_ALL=C sort -k2 >SHA256SUMS
echo "--- SHA256SUMS"
cat SHA256SUMS

if [ -n "$TAG" ]; then
  jq -R -s \
    --arg version "$VERSION" --arg tag "$TAG" \
    --arg base "https://github.com/$REPO/releases/download/$TAG" \
    --arg notes "https://github.com/$REPO/releases/tag/$TAG" '
    split("\n") | map(select(length > 0) | split(" ")) | sort_by(.[3])
    | map({os: .[0], arch: .[1], kind: .[2], name: .[3], url: ($base + "/" + .[3]),
           sha256: .[4], size: (.[5] | tonumber)})
    | {version: $version, tag: $tag, notes: $notes, assets: .}' "$ROWS" >latest.json
  echo "--- latest.json"
  cat latest.json
fi
