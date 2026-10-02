#!/bin/sh
# Fails if Cargo.lock records libflasher from a local path instead of the
# commit pinned in Cargo.toml (what building with the local override does).
cd "$(dirname "$0")/.."
if awk '/^name = "libflasher"$/{getline; getline; print}' Cargo.lock | grep -q '^source = "git+'; then
  echo "Cargo.lock: libflasher pinned to its git commit"
else
  echo "Cargo.lock points at a local libflasher checkout; run scripts/relock.sh before committing" >&2
  exit 1
fi
