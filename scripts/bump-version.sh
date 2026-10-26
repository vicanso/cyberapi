#!/usr/bin/env bash
# The next version, in [workspace.package] of Cargo.toml: the one place every
# crate and package reads it from (each crate says `version.workspace =
# true`). Called by `make version-{patch,minor,major}`.
#
#   scripts/bump-version.sh patch|minor|major
#
# Cargo.lock's entries for the workspace's own crates move with it (offline,
# and only those: `cargo update --workspace` leaves the pinned dependencies
# alone), so the lockfile committed with the bump matches. So do the
# deployment guides' examples: the image tag, the tarball, the version
# named as one in backticks (#54); and the Helm chart's version and
# appVersion (G6).
set -euo pipefail
cd "$(dirname "$0")/.."

KIND=${1:?usage: bump-version.sh <patch|minor|major>}

CURRENT=$(sed -n 's/^version = "\([^"]*\)"/\1/p' Cargo.toml | head -1)
[ -n "$CURRENT" ] || { echo "failed to read the version from Cargo.toml" >&2; exit 1; }
IFS=. read -r MAJ MIN PAT <<<"$CURRENT"

case "$KIND" in
  patch) PAT=$((PAT + 1)) ;;
  minor) MIN=$((MIN + 1)); PAT=0 ;;
  major) MAJ=$((MAJ + 1)); MIN=0; PAT=0 ;;
  *) echo "unknown bump kind: $KIND (want patch|minor|major)" >&2; exit 1 ;;
esac
NEXT="$MAJ.$MIN.$PAT"

# A bump that fails part-way leaves the tree as it found it: the version is
# read back from Cargo.toml, so a half-applied bump would skip one on retry.
DOCS=(docs/DEPLOY.md docs/DEPLOY_zh.md)
CHART=deploy/helm/cyberapi/Chart.yaml
TOUCHED=(Cargo.toml Cargo.lock "${DOCS[@]}" "$CHART")
SNAPSHOT=$(mktemp -d)
# Each under its own path: two of them share a name in another directory.
for f in "${TOUCHED[@]}"; do
  mkdir -p "$SNAPSHOT/$(dirname "$f")"
  cp "$f" "$SNAPSHOT/$f"
done
finish() {
  status=$?
  if [ "$status" -ne 0 ]; then
    for f in "${TOUCHED[@]}"; do cp "$SNAPSHOT/$f" "$f"; done
    echo "bump failed; ${TOUCHED[*]} restored" >&2
  fi
  rm -rf "$SNAPSHOT"
}
trap finish EXIT

# Only the first `version = "…"` line, [workspace.package]'s: the root
# [package] says `version.workspace = true`, which does not match.
CURRENT="$CURRENT" NEXT="$NEXT" perl -pi -e \
  's|^version = "\Q$ENV{CURRENT}\E"|version = "$ENV{NEXT}"| && ++$done unless $done' Cargo.toml
grep -q "^version = \"$NEXT\"" Cargo.toml || { echo "failed to bump Cargo.toml" >&2; exit 1; }

cargo update --workspace --offline --quiet

CURRENT="$CURRENT" NEXT="$NEXT" perl -pi -e \
  's/(cyberapi-web[:-])\Q$ENV{CURRENT}\E(?![\d.])/$1$ENV{NEXT}/g; s/`\Q$ENV{CURRENT}\E`/`$ENV{NEXT}`/g' "${DOCS[@]}"

CURRENT="$CURRENT" NEXT="$NEXT" perl -pi -e \
  's/^(version: )\Q$ENV{CURRENT}\E$/$1$ENV{NEXT}/; s/^(appVersion: ")\Q$ENV{CURRENT}\E"$/$1$ENV{NEXT}"/' "$CHART"
grep -q "^appVersion: \"$NEXT\"" "$CHART" || { echo "failed to bump $CHART" >&2; exit 1; }

echo "version: $CURRENT -> $NEXT"
