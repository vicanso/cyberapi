#!/usr/bin/env bash
# A nightly's version (#54): a pre-release of the patch after Cargo.toml's,
# with the day (UTC) and the commit built, e.g. 0.2.2-nightly.20261024.137a858.
# Newer than the release it was built after, older than the one it leads
# to, so the update check orders them rightly. nightly.yml hands it to the
# builds as CYBERAPI_VERSION.
#
#   scripts/nightly-version.sh [commit]   the commit defaults to HEAD
set -euo pipefail
cd "$(dirname "$0")/.."

version=$(sed -n 's/^version = "\([^"]*\)"/\1/p' Cargo.toml | head -1)
IFS=. read -r major minor patch <<<"${version%%-*}"
commit=$(git rev-parse --short=7 "${1:-HEAD}")
echo "$major.$minor.$((patch + 1))-nightly.$(date -u +%Y%m%d).$commit"
