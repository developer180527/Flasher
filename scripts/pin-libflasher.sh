#!/bin/sh
# Move the libflasher pin to a commit (default: libflasher's main on GitHub)
# and re-lock. Touches only the libflasher line of Cargo.toml.
set -e
cd "$(dirname "$0")/.."
sha=${1:-$(git ls-remote https://github.com/developer180527/libflasher HEAD | cut -f1)}
case "$sha" in
  *[!0-9a-f]* | "") echo "not a commit id: $sha" >&2; exit 1 ;;
esac
[ ${#sha} -eq 40 ] || { echo "need the full 40-character commit id" >&2; exit 1; }
sed -i.bak "/^libflasher = /s/rev = \"[0-9a-f]*\"/rev = \"$sha\"/" Cargo.toml && rm Cargo.toml.bak
grep '^libflasher = ' Cargo.toml
scripts/relock.sh
