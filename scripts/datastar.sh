#!/usr/bin/env bash
# Vendors Datastar's script: bundles/datastar.js at a release's tag, byte for
# byte, into assets/datastar.js, and the release into src/script.rs's VERSION.
# With --check, downloads the release VERSION names and fails unless
# assets/datastar.js is exactly it, so the copy shipped is upstream's.
#
#   scripts/datastar.sh X.Y.Z
#   scripts/datastar.sh --check
set -euo pipefail

cd "$(git rev-parse --show-toplevel)"
current=$(sed -n 's/^pub const VERSION: &str = "\(.*\)";$/\1/p' src/script.rs)
[ -n "$current" ] || { echo "src/script.rs names no VERSION" >&2; exit 1; }

tmp=$(mktemp)
trap 'rm -f "$tmp"' EXIT

# The tag's file, from GitHub itself rather than a CDN's copy of it.
fetch() {
  curl -fsSL --retry 3 "https://raw.githubusercontent.com/starfederation/datastar/v$1/bundles/datastar.js" > "$tmp"
}

case "${1:-}" in
  --check)
    fetch "$current"
    if ! cmp -s "$tmp" assets/datastar.js; then
      echo "assets/datastar.js is not Datastar v$current's bundles/datastar.js: run scripts/datastar.sh $current" >&2
      exit 1
    fi
    echo "assets/datastar.js is Datastar v$current's"
    ;;
  [0-9]*.[0-9]*.[0-9]*)
    version=$1
    fetch "$version"
    if [ "$(head -n 1 "$tmp")" != "// Datastar v$version" ]; then
      echo "the download does not open with // Datastar v$version" >&2
      exit 1
    fi
    cat "$tmp" > assets/datastar.js
    sed -i.bak "s/^pub const VERSION: &str = \".*\";$/pub const VERSION: \&str = \"$version\";/" src/script.rs
    rm -f src/script.rs.bak
    echo "vendored Datastar v$version, replacing v$current; add a changelog fragment saying what the release changes"
    ;;
  *)
    echo "usage: scripts/datastar.sh X.Y.Z | --check" >&2
    exit 2
    ;;
esac
