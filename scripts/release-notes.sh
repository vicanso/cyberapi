#!/usr/bin/env bash
# The text of a version's release: its section of CHANGELOG.md (`make
# version` writes it), heading and all, as the release page shows it and the
# desktop's update dialog reads it (GitHub's API, `body`). Called by
# release.yml.
#
#   scripts/release-notes.sh vX.Y.Z [CHANGELOG.md]   the section, on stdout
#
# A version without a section is told on stderr and gets an empty text: the
# release still goes out, with its packages.
set -euo pipefail

version=${1:?usage: release-notes.sh <version> [changelog]}
version=${version#v}
changelog=${2:-CHANGELOG.md}

notes=$(awk -v heading="## [$version]" '
  /^## \[/ {
    if (found) exit
    found = index($0, heading) == 1
  }
  found { print }
' "$changelog")

if [ -z "$notes" ]; then
  echo "release-notes.sh: $changelog has no section for $version" >&2
  exit 0
fi
printf '%s\n' "$notes"
