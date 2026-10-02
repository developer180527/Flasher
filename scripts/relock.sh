#!/bin/sh
# Re-resolve Cargo.lock against the libflasher commit pinned in Cargo.toml,
# ignoring a local .cargo/config.toml override. Run before committing when
# you have been building against a local libflasher checkout.
set -e
cd "$(dirname "$0")/.."
if [ -f .cargo/config.toml ]; then
  mv .cargo/config.toml .cargo/config.toml.relock
  trap 'mv .cargo/config.toml.relock .cargo/config.toml' EXIT
fi
cargo metadata --format-version 1 >/dev/null
scripts/check-lock.sh
